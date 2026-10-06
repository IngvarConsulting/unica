use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use crate::error::{BootstrapError, Failure, Result};
use crate::network::{NetworkClient, NetworkError, NetworkFailure};

pub trait Downloader: Send + Sync {
    fn download(
        &self,
        url: &str,
        destination: &Path,
        observer: &dyn DownloadObserver,
    ) -> Result<()>;
}

/// Кому сообщать о ходе загрузки.
///
/// Вызывающий, дожидающийся движка, видит не проценты, а байты: сколько лежит
/// на диске и сколько обещал сервер. С этого же места продолжит следующая
/// попытка, поэтому число одно и то же для отчёта и для докачки.
pub trait DownloadObserver: Send + Sync {
    fn transferred(&self, received: u64, total: Option<u64>);
}

/// Загрузка, о ходе которой некому рассказывать.
pub struct SilentDownload;

impl DownloadObserver for SilentDownload {
    fn transferred(&self, _received: u64, _total: Option<u64>) {}
}

/// Шаг чтения.
const CHUNK: usize = 64 * 1024;

/// HTTP-загрузчик поверх общего клиента процесса ([`NetworkClient`]):
/// TLS с доверием ОС, прокси из окружения, подключение до 30 с и ожидание
/// данных до 60 с.
///
/// Сейчас общего срока передачи нет. Контроль длительного почти нулевого
/// прогресса и ограниченные автоматические повторы ещё не реализованы.
#[derive(Default)]
pub struct HttpDownloader {
    /// `None` — общий клиент процесса, собираемый при первой загрузке.
    client: Option<Arc<NetworkClient>>,
}

impl HttpDownloader {
    /// Загрузчик с заданным клиентом, например собранным из своей таблицы
    /// переменных прокси.
    pub fn with_client(client: Arc<NetworkClient>) -> Self {
        Self {
            client: Some(client),
        }
    }

    fn client(&self) -> std::result::Result<Arc<NetworkClient>, NetworkError> {
        match &self.client {
            Some(client) => Ok(Arc::clone(client)),
            None => NetworkClient::shared(),
        }
    }

    /// Перенести байты, продолжив с того места, где оборвалась прошлая попытка.
    ///
    /// Схему проверяет вызывающий: здесь только перенос.
    fn transfer(
        &self,
        url: &str,
        destination: &Path,
        observer: &dyn DownloadObserver,
    ) -> Result<()> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let received = fs::metadata(destination)
            .map(|meta| meta.len())
            .unwrap_or(0);

        let range = format!("bytes={received}-");
        let headers: &[(&str, &str)] = if received > 0 {
            &[("Range", range.as_str())]
        } else {
            &[]
        };
        let response = match self
            .client()
            .and_then(|client| client.get(url, headers, None))
        {
            Ok(response) => response,
            // Диапазон за концом файла: качать больше нечего. Тот ли это файл,
            // рассудит контрольная сумма у вызывающего.
            Err(error) if error.status() == Some(416) => return Ok(()),
            Err(error) => return Err(download_failure(url, error)),
        };

        // Сервер вправе забыть про диапазон и ответить `200` целым файлом. Так
        // делают зеркала и прокси. Дописать такой ответ к недокачанному файлу
        // значит склеить мусор, поэтому продолжаем только после `206`.
        let resumed = response.status() == 206;
        let mut output = if resumed {
            OpenOptions::new().append(true).open(destination)?
        } else {
            File::create(destination)?
        };
        let start = if resumed { received } else { 0 };
        // Сервер обещает длину остатка; вызывающему нужен весь файл.
        let total = response
            .header("Content-Length")
            .and_then(|value| value.parse::<u64>().ok())
            .map(|remaining| start + remaining);
        observer.transferred(start, total);

        let mut reader = response.into_reader();
        let mut buffer = vec![0_u8; CHUNK];
        let mut moved = 0_u64;
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                BootstrapError::of(
                    Failure::Network,
                    format!(
                        "failed to read runtime asset {url} after {} bytes: {error}",
                        start + moved
                    ),
                )
            })?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).map_err(|error| {
                BootstrapError::of(
                    Failure::Disk,
                    format!(
                        "failed to write runtime asset {}: {error}",
                        destination.display()
                    ),
                )
            })?;
            moved += read as u64;
            observer.transferred(start + moved, total);
        }
        output.sync_all()?;
        Ok(())
    }
}

/// Перевести сетевой отказ в отказ загрузчика: класс задаёт код выхода,
/// а совет сетевого отказа заменяет общий «повторите».
fn download_failure(url: &str, error: NetworkError) -> BootstrapError {
    let failure = match error.failure() {
        NetworkFailure::Configuration | NetworkFailure::InsecureRedirect => Failure::Configuration,
        NetworkFailure::UntrustedCertificate
        | NetworkFailure::RejectedCertificate
        | NetworkFailure::Status(_)
        | NetworkFailure::Transport => Failure::Network,
    };
    let refusal = BootstrapError::of(
        failure,
        format!(
            "failed to download runtime asset {url}: {}",
            error.message()
        ),
    );
    match error.cure() {
        Some(cure) => refusal.with_cure(cure),
        None => refusal,
    }
}

impl Downloader for HttpDownloader {
    fn download(
        &self,
        url: &str,
        destination: &Path,
        observer: &dyn DownloadObserver,
    ) -> Result<()> {
        if !url.starts_with("https://") {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("runtime download URL must use HTTPS: {url}"),
            ));
        }
        self.transfer(url, destination, observer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::network::test_support::{client_with, ConnectProxy, TestRoot, TlsStand};
    use std::thread;

    /// Что стенд делает с заголовком `Range`.
    #[derive(Clone, Copy)]
    enum Ranges {
        /// Отдать хвост: `206` и `Content-Range`.
        Honoured,
        /// Забыть про диапазон и отдать файл целиком. Так отвечают зеркала и
        /// прокси, и докачка обязана это пережить.
        Ignored,
        /// Отдавать по байту с паузой: канал жив, но не кончается.
        Trickle,
    }

    struct Stand {
        url: String,
        requested: Arc<Mutex<Vec<Option<String>>>>,
    }

    impl Stand {
        fn requested(&self) -> Vec<Option<String>> {
            self.requested.lock().expect("stand log").clone()
        }
    }

    fn serve(payload: Vec<u8>, mode: Ranges) -> Stand {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stand");
        let url = format!(
            "http://{}/artifact.tar.gz",
            listener.local_addr().expect("stand address")
        );
        let requested = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requested);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = BufReader::new(stream.try_clone().expect("clone stand stream"));
                let mut range = None;
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Range: ") {
                        range = Some(value.trim().to_owned());
                    }
                }
                seen.lock().expect("stand log").push(range.clone());

                let first = match (mode, range.as_deref()) {
                    (Ranges::Honoured, Some(value)) => value
                        .trim_start_matches("bytes=")
                        .trim_end_matches('-')
                        .parse::<usize>()
                        .unwrap_or(0),
                    _ => 0,
                };
                let body = &payload[first.min(payload.len())..];
                let head = if first > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {first}-{}/{}\r\nAccept-Ranges: bytes\r\n\r\n",
                        body.len(),
                        payload.len() - 1,
                        payload.len()
                    )
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                        body.len()
                    )
                };
                if stream.write_all(head.as_bytes()).is_err() {
                    continue;
                }
                match mode {
                    Ranges::Trickle => {
                        for byte in body {
                            if stream.write_all(&[*byte]).is_err() || stream.flush().is_err() {
                                break;
                            }
                            thread::sleep(Duration::from_millis(1));
                        }
                    }
                    _ => {
                        let _ = stream.write_all(body);
                    }
                }
            }
        });
        Stand { url, requested }
    }

    fn scratch(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("unica-download-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("scratch directory");
        path.join("artifact.tar.gz")
    }

    /// Загрузчик без прокси: тест не должен зависеть от переменных машины.
    fn local() -> HttpDownloader {
        downloader_with(&[])
    }

    fn downloader_with(variables: &[(&str, &str)]) -> HttpDownloader {
        HttpDownloader::with_client(client_with(variables))
    }

    fn payload(size: usize) -> Vec<u8> {
        (0..size).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn a_resumed_download_asks_only_for_the_missing_tail() {
        let bytes = payload(4096);
        let stand = serve(bytes.clone(), Ranges::Honoured);
        let destination = scratch("tail");
        fs::write(&destination, &bytes[..1000]).expect("partial file");

        local()
            .transfer(&stand.url, &destination, &SilentDownload)
            .expect("resume the download");

        assert_eq!(
            stand.requested(),
            vec![Some("bytes=1000-".to_owned())],
            "докачка просит хвост, а не файл целиком"
        );
        assert_eq!(fs::read(&destination).expect("resumed file"), bytes);
    }

    #[test]
    fn a_server_that_forgets_the_range_still_leaves_the_whole_file() {
        // Зеркала и прокси отвечают `200` на запрос с диапазоном. Дописать
        // такой ответ к недокачанному файлу значит склеить мусор.
        let bytes = payload(4096);
        let stand = serve(bytes.clone(), Ranges::Ignored);
        let destination = scratch("forgetful");
        fs::write(&destination, &bytes[..1000]).expect("partial file");

        local()
            .transfer(&stand.url, &destination, &SilentDownload)
            .expect("download from a forgetful server");

        assert_eq!(fs::read(&destination).expect("restarted file"), bytes);
    }

    /// Наблюдатель, запоминающий, что ему сообщили.
    #[derive(Default)]
    struct Watched {
        seen: Mutex<Vec<(u64, Option<u64>)>>,
    }

    impl DownloadObserver for Watched {
        fn transferred(&self, received: u64, total: Option<u64>) {
            self.seen.lock().expect("seen").push((received, total));
        }
    }

    #[test]
    fn a_download_reports_the_size_of_what_is_on_disk() {
        // Вызывающий ждёт доставки и хочет видеть, что она идёт. Считать
        // приходится то, что лежит на диске: с этого места продолжит следующая
        // попытка, и это же число он увидит, если посмотрит сам.
        let bytes = payload(4096);
        let stand = serve(bytes.clone(), Ranges::Honoured);
        let destination = scratch("watched");
        let watched = Watched::default();

        local()
            .transfer(&stand.url, &destination, &watched)
            .expect("download");

        let seen = watched.seen.lock().expect("seen").clone();
        assert_eq!(
            seen.last().copied(),
            Some((4096, Some(4096))),
            "последнее сообщение — весь файл: {seen:?}"
        );
    }

    #[test]
    fn a_resumed_download_counts_from_what_already_arrived() {
        // Сообщать про хвост значит показать откат к нулю на середине.
        let bytes = payload(4096);
        let stand = serve(bytes.clone(), Ranges::Honoured);
        let destination = scratch("watched-resume");
        fs::write(&destination, &bytes[..1000]).expect("partial file");
        let watched = Watched::default();

        local()
            .transfer(&stand.url, &destination, &watched)
            .expect("resume");

        let seen = watched.seen.lock().expect("seen").clone();
        assert!(
            seen.iter().all(|(received, _)| *received >= 1000),
            "счёт идёт от уже полученного: {seen:?}"
        );
        assert_eq!(seen.last().copied(), Some((4096, Some(4096))));
    }

    #[test]
    fn a_slow_channel_is_not_cut_off_for_being_slow() {
        // Проверяем получение файла при выдаче по одному байту с паузами.
        // Этот короткий сценарий не доказывает отсутствие общего срока
        // и не проверяет восстановление после длительного слабого прогресса.
        let bytes = payload(256);
        let stand = serve(bytes.clone(), Ranges::Trickle);
        let destination = scratch("slow");

        local()
            .transfer(&stand.url, &destination, &SilentDownload)
            .expect("медленный канал доезжает");

        assert_eq!(fs::read(&destination).expect("whole file"), bytes);
    }

    #[test]
    fn a_plaintext_url_is_refused_before_any_byte_moves() {
        let destination = scratch("plaintext");

        let error = local()
            .download(
                "http://example.invalid/artifact.tar.gz",
                &destination,
                &SilentDownload,
            )
            .expect_err("HTTPS обязателен");

        assert!(error.to_string().contains("HTTPS"), "{error}");
        assert!(!destination.exists());
    }

    #[test]
    fn a_root_the_os_does_not_trust_is_refused_with_the_interception_cure() {
        // Корень стенда не стоит в хранилище ОС: так выглядит подмена
        // сертификата средством защиты, корня которого ОС не знает.
        let root = TestRoot::generate("Unica untrusted test root");
        let stand = TlsStand::start(&root, payload(64), "application/octet-stream");
        let destination = scratch("untrusted");

        let error = local()
            .download(&stand.url("artifact.tar.gz"), &destination, &SilentDownload)
            .expect_err("недоверенный корень");

        assert_eq!(error.failure(), Failure::Network);
        let diagnosis = error.diagnosis();
        for expected in ["intercepted", "trust store", "HTTPS_PROXY", "127.0.0.1"] {
            assert!(diagnosis.contains(expected), "{expected}: {diagnosis}");
        }
        assert!(
            !diagnosis.contains("run again"),
            "повтор не лечит недоверенный корень: {diagnosis}"
        );
        assert!(stand.connections() >= 1, "соединение дошло до стенда");
        assert!(stand.requests().is_empty(), "до HTTP дело не дошло");
    }

    #[test]
    fn https_proxy_carries_the_download_to_the_proxy() {
        let root = TestRoot::generate("Unica proxy test root");
        let stand = TlsStand::start(&root, payload(64), "application/octet-stream");
        let proxy = ConnectProxy::start();
        let destination = scratch("proxied");

        // Корень стенда ОС не доверяет, поэтому загрузка дальше рукопожатия
        // не пойдёт; здесь проверяется маршрут. Полную загрузку через прокси
        // с доверенным корнем доказывает тест хранилища Linux.
        let error = downloader_with(&[("HTTPS_PROXY", &proxy.url())])
            .download(&stand.url("artifact.tar.gz"), &destination, &SilentDownload)
            .expect_err("корень не доверен");

        assert_eq!(proxy.targets(), vec![stand.authority()]);
        assert!(
            error.to_string().contains("from HTTPS_PROXY"),
            "отказ называет маршрут: {error}"
        );
    }

    #[test]
    fn no_proxy_sends_the_download_past_the_proxy() {
        let root = TestRoot::generate("Unica no-proxy test root");
        let stand = TlsStand::start(&root, payload(64), "application/octet-stream");
        let proxy = ConnectProxy::start();
        let destination = scratch("bypass");

        let error = downloader_with(&[("HTTPS_PROXY", &proxy.url()), ("NO_PROXY", "127.0.0.1")])
            .download(&stand.url("artifact.tar.gz"), &destination, &SilentDownload)
            .expect_err("корень не доверен");

        assert!(proxy.targets().is_empty(), "{:?}", proxy.targets());
        assert!(stand.connections() >= 1, "соединение шло напрямую");
        assert!(error.to_string().contains("direct connection"), "{error}");
    }

    #[test]
    fn an_unsupported_proxy_is_a_configuration_refusal_without_the_password() {
        let error = downloader_with(&[("HTTPS_PROXY", "socks5://alice:s3cret@127.0.0.1:9")])
            .download(
                "https://127.0.0.1:9/artifact.tar.gz",
                &scratch("socks"),
                &SilentDownload,
            )
            .expect_err("SOCKS не поддерживается");

        assert_eq!(error.failure(), Failure::Configuration);
        let diagnosis = error.diagnosis();
        assert!(diagnosis.contains("HTTPS_PROXY"), "{diagnosis}");
        assert!(!diagnosis.contains("s3cret"), "{diagnosis}");
        assert!(
            !diagnosis.contains("reinstall"),
            "совет про прокси, а не про переустановку: {diagnosis}"
        );
    }
}
