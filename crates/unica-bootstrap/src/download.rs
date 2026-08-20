use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::{BootstrapError, Failure, Result};

pub trait Downloader: Send + Sync {
    fn download(&self, url: &str, destination: &Path) -> Result<()>;
}

/// Сколько времени отведено одной загрузке целиком.
///
/// `timeout_read` ловит замерший канал, но не ловит сочащийся: сервер, отдающий
/// байт раз в полминуты, укладывается в каждый отдельный таймаут и не кончается
/// никогда. Бюджет на всю загрузку закрывает эту дыру.
///
/// Полчаса выведены из худшего замеренного у пользователя канала: самый крупный
/// архив, 69 МБ, при 0,5 МБ/с едет 138 с, и до получаса остаётся запас на канал
/// в тринадцать раз медленнее. Истёкший бюджет ничего не теряет: полученное
/// лежит на диске, и следующая попытка продолжает с того же места.
const DOWNLOAD_BUDGET: Duration = Duration::from_secs(30 * 60);

/// Шаг чтения. Бюджет проверяется между шагами, поэтому шаг задаёт и точность
/// проверки.
const CHUNK: usize = 64 * 1024;

pub struct HttpDownloader {
    agent: ureq::Agent,
    budget: Duration,
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::with_budget(DOWNLOAD_BUDGET)
    }
}

impl HttpDownloader {
    pub fn with_budget(budget: Duration) -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(30))
                .timeout_read(Duration::from_secs(60))
                .redirects(5)
                .build(),
            budget,
        }
    }

    /// Перенести байты, продолжив с того места, где оборвалась прошлая попытка.
    ///
    /// Схему проверяет вызывающий: здесь только перенос.
    fn transfer(&self, url: &str, destination: &Path) -> Result<()> {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        let received = fs::metadata(destination)
            .map(|meta| meta.len())
            .unwrap_or(0);

        let mut request = self.agent.get(url);
        if received > 0 {
            request = request.set("Range", &format!("bytes={received}-"));
        }
        let response = match request.call() {
            Ok(response) => response,
            // Диапазон за концом файла: качать больше нечего. Тот ли это файл,
            // рассудит контрольная сумма у вызывающего.
            Err(ureq::Error::Status(416, _)) => return Ok(()),
            Err(error) => {
                return Err(BootstrapError::of(
                    Failure::Network,
                    format!("failed to download runtime asset {url}: {error}"),
                ))
            }
        };
        // Редирект не вправе понизить схему.
        if url.starts_with("https://") && !response.get_url().starts_with("https://") {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!(
                    "runtime download redirected to a non-HTTPS URL: {}",
                    response.get_url()
                ),
            ));
        }

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

        let deadline = Instant::now() + self.budget;
        let mut reader = response.into_reader();
        let mut buffer = vec![0_u8; CHUNK];
        let mut moved = 0_u64;
        loop {
            if Instant::now() >= deadline {
                output.sync_all()?;
                return Err(BootstrapError::of(
                    Failure::Timeout,
                    format!(
                        "runtime asset {url} exhausted the {}s download budget after {} bytes",
                        self.budget.as_secs(),
                        start + moved
                    ),
                ));
            }
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
        }
        output.sync_all()?;
        Ok(())
    }
}

impl Downloader for HttpDownloader {
    fn download(&self, url: &str, destination: &Path) -> Result<()> {
        if !url.starts_with("https://") {
            return Err(BootstrapError::of(
                Failure::Configuration,
                format!("runtime download URL must use HTTPS: {url}"),
            ));
        }
        self.transfer(url, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
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
                            thread::sleep(Duration::from_millis(50));
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

    fn payload(size: usize) -> Vec<u8> {
        (0..size).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn a_resumed_download_asks_only_for_the_missing_tail() {
        let bytes = payload(4096);
        let stand = serve(bytes.clone(), Ranges::Honoured);
        let destination = scratch("tail");
        fs::write(&destination, &bytes[..1000]).expect("partial file");

        HttpDownloader::default()
            .transfer(&stand.url, &destination)
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

        HttpDownloader::default()
            .transfer(&stand.url, &destination)
            .expect("download from a forgetful server");

        assert_eq!(fs::read(&destination).expect("restarted file"), bytes);
    }

    #[test]
    fn the_budget_ends_a_channel_that_never_finishes() {
        let stand = serve(payload(4096), Ranges::Trickle);
        let destination = scratch("budget");

        let error = HttpDownloader::with_budget(Duration::from_millis(400))
            .transfer(&stand.url, &destination)
            .expect_err("бесконечный канал обязан кончиться отказом");

        assert!(
            error.to_string().contains("budget"),
            "отказ называет причину: {error}"
        );
        let received = fs::metadata(&destination).expect("partial file").len();
        assert!(
            received > 0 && received < 4096,
            "полученное остаётся следующей попытке: {received} байт"
        );
    }

    #[test]
    fn a_plaintext_url_is_refused_before_any_byte_moves() {
        let destination = scratch("plaintext");

        let error = HttpDownloader::default()
            .download("http://example.invalid/artifact.tar.gz", &destination)
            .expect_err("HTTPS обязателен");

        assert!(error.to_string().contains("HTTPS"), "{error}");
        assert!(!destination.exists());
    }
}
