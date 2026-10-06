//! Стенды для тестов сетевых потребителей: HTTPS-сервер на собственном корне
//! и HTTP-прокси с методом CONNECT.
//!
//! Продуктовая сборка этого модуля не содержит: он есть только в тестах
//! крейта и под фичей `network-test-support`, которую включают
//! dev-dependencies. Клиенту стенд ничего не подкладывает: доверие корню
//! стенда тест обязан получить тем же путём, что и пользователь, — через
//! хранилище ОС.

use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};

use super::NetworkClient;

/// Клиент с переменными прокси из таблицы вместо окружения процесса и
/// доверием ОС. Пустая таблица — прямой путь, что бы ни стояло на машине.
pub fn client_with(variables: &[(&str, &str)]) -> Arc<NetworkClient> {
    let table: Vec<(String, String)> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let client = NetworkClient::from_environment(&move |name| {
        table
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
    })
    .expect("network client over the OS trust store");
    Arc::new(client)
}

/// Собственный корневой сертификат стенда.
pub struct TestRoot {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

impl TestRoot {
    pub fn generate(name: &str) -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).expect("root params");
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.distinguished_name.push(DnType::CommonName, name);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let key = KeyPair::generate().expect("root key");
        Self {
            issuer: CertifiedIssuer::self_signed(params, key).expect("self-signed root"),
        }
    }

    /// Корень в PEM: то, что кладут в `SSL_CERT_FILE` или в хранилище ОС.
    pub fn pem(&self) -> String {
        self.issuer.pem()
    }

    pub fn write_pem(&self, path: &Path) {
        std::fs::write(path, self.pem()).expect("write root PEM");
    }
}

/// Что стенд увидел.
#[derive(Default)]
struct Seen {
    connections: AtomicUsize,
    requests: Mutex<Vec<String>>,
}

/// HTTPS-сервер на `127.0.0.1` с сертификатом от [`TestRoot`]. На любой запрос
/// отвечает одним и тем же телом.
pub struct TlsStand {
    port: u16,
    seen: Arc<Seen>,
}

impl TlsStand {
    pub fn start(root: &TestRoot, body: impl Into<Vec<u8>>, content_type: &str) -> Self {
        let mut params =
            CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()])
                .expect("leaf params");
        params
            .distinguished_name
            .push(DnType::CommonName, "unica test stand");
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let key = KeyPair::generate().expect("leaf key");
        let leaf = params.signed_by(&key, &root.issuer).expect("leaf");
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone(), root.issuer.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
        )
        .expect("server config");
        let config = Arc::new(config);

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind TLS stand");
        let port = listener.local_addr().expect("stand address").port();
        let seen = Arc::new(Seen::default());
        let body: Arc<Vec<u8>> = Arc::new(body.into());
        let content_type = content_type.to_owned();
        let shared = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                shared.connections.fetch_add(1, Ordering::SeqCst);
                let config = Arc::clone(&config);
                let seen = Arc::clone(&shared);
                let body = Arc::clone(&body);
                let content_type = content_type.clone();
                thread::spawn(move || {
                    let _ = serve_tls(stream, config, &seen, &body, &content_type);
                });
            }
        });
        Self { port, seen }
    }

    pub fn url(&self, path: &str) -> String {
        format!(
            "https://127.0.0.1:{}/{}",
            self.port,
            path.trim_start_matches('/')
        )
    }

    pub fn authority(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    /// Сколько TCP-соединений принято, включая оборванные на рукопожатии.
    pub fn connections(&self) -> usize {
        self.seen.connections.load(Ordering::SeqCst)
    }

    /// Строки запросов, дошедших до HTTP после рукопожатия.
    pub fn requests(&self) -> Vec<String> {
        self.seen.requests.lock().expect("stand log").clone()
    }
}

fn serve_tls(
    stream: TcpStream,
    config: Arc<rustls::ServerConfig>,
    seen: &Seen,
    body: &[u8],
    content_type: &str,
) -> std::io::Result<()> {
    let connection = rustls::ServerConnection::new(config).map_err(std::io::Error::other)?;
    let mut tls = rustls::StreamOwned::new(connection, stream);
    // Тело POST дочитывается в `read_head`: отвечаем только после него.
    let request_line = read_head(&mut BufReader::new(&mut tls))?;
    seen.requests.lock().expect("stand log").push(request_line);
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    tls.write_all(head.as_bytes())?;
    tls.write_all(body)?;
    tls.flush()?;
    tls.conn.send_close_notify();
    let _ = tls.flush();
    Ok(())
}

/// Прочитать строку запроса и заголовки, а затем тело по `Content-Length`.
fn read_head(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0_u8; content_length];
    reader.read_exact(&mut body)?;
    Ok(request_line.trim().to_owned())
}

/// HTTP-прокси, который туннелирует CONNECT и запоминает цели. Запрос
/// к http-адресу он только записывает и отвечает `405`.
pub struct ConnectProxy {
    port: u16,
    targets: Arc<Mutex<Vec<String>>>,
}

impl ConnectProxy {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind proxy");
        let port = listener.local_addr().expect("proxy address").port();
        let targets = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&targets);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                let seen = Arc::clone(&seen);
                thread::spawn(move || {
                    let _ = tunnel(stream, &seen);
                });
            }
        });
        Self { port, targets }
    }

    /// Значение для `HTTPS_PROXY`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Цели в порядке прихода: `127.0.0.1:443` для CONNECT, полный адрес
    /// для запроса к http-адресу.
    pub fn targets(&self) -> Vec<String> {
        self.targets.lock().expect("proxy log").clone()
    }
}

fn tunnel(client: TcpStream, seen: &Mutex<Vec<String>>) -> std::io::Result<()> {
    let mut reader = BufReader::new(client.try_clone()?);
    let request_line = read_head(&mut reader)?;
    let mut parts = request_line.split_whitespace();
    let (Some("CONNECT"), Some(target)) = (parts.next(), parts.next()) else {
        // Запрос к http-адресу через прокси идёт абсолютной формой, без
        // туннеля: запоминаем адрес и отказываем, пересылать стенд не умеет.
        if let Some(target) = request_line.split_whitespace().nth(1) {
            seen.lock().expect("proxy log").push(target.to_owned());
        }
        let mut client = client;
        client.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n")?;
        return Ok(());
    };
    seen.lock().expect("proxy log").push(target.to_owned());
    let upstream = TcpStream::connect(target)?;
    let mut client_writer = client.try_clone()?;
    client_writer.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?;

    // Байты, которые клиент успел прислать вслед за заголовком, уже в буфере.
    let pending = reader.buffer().to_vec();
    let mut upstream_writer = upstream.try_clone()?;
    upstream_writer.write_all(&pending)?;
    let mut upstream_reader = upstream;
    let downstream = thread::spawn(move || {
        let _ = std::io::copy(&mut upstream_reader, &mut client_writer);
        let _ = client_writer.shutdown(Shutdown::Write);
    });
    let mut client_reader = reader.into_inner();
    let _ = std::io::copy(&mut client_reader, &mut upstream_writer);
    let _ = upstream_writer.shutdown(Shutdown::Write);
    let _ = downstream.join();
    Ok(())
}
