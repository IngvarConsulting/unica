use super::*;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    (listener, base)
}

fn accept(listener: &TcpListener) -> TcpStream {
    let (stream, _) = listener.accept().unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    stream
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut request = String::new();
    let mut content_length = 0;
    loop {
        let mut line = String::new();
        assert!(reader.read_line(&mut line).unwrap() > 0);
        if line == "\r\n" {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            if key.eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap();
            }
        }
        request.push_str(&line);
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body).unwrap();
    request.push_str(std::str::from_utf8(&body).unwrap());
    request
}

fn client_with_timeout(read_timeout: Duration) -> NetworkClient {
    let (tls, trust) = tls_config().unwrap();
    NetworkClient::build(
        ProxySettings::from_lookup(&|_| None),
        tls,
        trust,
        read_timeout,
    )
    .unwrap()
}

#[test]
fn medium_completed_post_and_detached_body_survive_connection_close() {
    let (listener, base) = listener();
    let (finish, ack) = std::sync::mpsc::sync_channel(1);
    let (dropped, reset) = std::sync::mpsc::sync_channel(1);
    let server = thread::spawn(move || {
        let mut first = accept(&listener);
        let post = read_request(&mut first);
        assert!(post.starts_with("POST /rpc HTTP/1.1\r\n"));
        assert!(post.ends_with("one mutation"));
        first
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\nfinal bytes")
            .unwrap();
        // The complete response cannot be recycled, even without a
        // Connection: close header. Reset after the client has proved it
        // received every byte to avoid racing the kernel's send queue.
        ack.recv_timeout(Duration::from_secs(4)).unwrap();
        socket2::SockRef::from(&first)
            .set_linger(Some(Duration::ZERO))
            .unwrap();
        drop(first);
        dropped.send(()).unwrap();
        let mut next = accept(&listener);
        assert!(read_request(&mut next).starts_with("GET /next HTTP/1.1\r\n"));
        next.write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
            .unwrap();
    });

    let client = client_with_timeout(READ_TIMEOUT);
    let mut body = client
        .post(&format!("{base}/rpc"), &[], "one mutation", None)
        .unwrap()
        .into_reader();
    let mut recovered = Vec::new();
    let mut buffer = [0; 3];
    loop {
        let count = body.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        recovered.extend_from_slice(&buffer[..count]);
    }
    assert_eq!(recovered, b"final bytes");
    finish.send(()).unwrap();
    reset.recv_timeout(Duration::from_secs(4)).unwrap();
    // The backend can observe a stale idle socket racing its peer's reset.
    // Use a fresh client here; ordinary pooling is verified separately.
    let fresh = client_with_timeout(READ_TIMEOUT);
    assert_eq!(
        fresh
            .get(&format!("{base}/next"), &[], None)
            .unwrap()
            .into_string()
            .unwrap(),
        ""
    );
    server.join().unwrap();
}

#[test]
fn medium_an_incomplete_body_is_an_error_and_the_next_request_succeeds() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut first = accept(&listener);
        read_request(&mut first);
        first
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\npart")
            .unwrap();
        drop(first);
        let mut second = accept(&listener);
        assert!(read_request(&mut second).starts_with("GET /next "));
        second
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone")
            .unwrap();
    });
    let client = client_with_timeout(READ_TIMEOUT);
    let mut output = Vec::new();
    let error = client
        .get(&base, &[], None)
        .unwrap()
        .into_reader()
        .read_to_end(&mut output)
        .unwrap_err();
    assert_eq!(output, b"part");
    assert!(error.to_string().contains("could not be read completely"));
    assert_eq!(
        client
            .get(&format!("{base}/next"), &[], None)
            .unwrap()
            .into_string()
            .unwrap(),
        "done"
    );
    server.join().unwrap();
}

#[test]
fn medium_a_healthy_connection_is_reused_after_reading_the_body() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        assert!(read_request(&mut stream).starts_with("GET /first "));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nbody")
            .unwrap();
        assert!(read_request(&mut stream).starts_with("GET /next "));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone")
            .unwrap();
    });
    let client = client_with_timeout(READ_TIMEOUT);
    assert_eq!(
        client
            .get(&format!("{base}/first"), &[], None)
            .unwrap()
            .into_string()
            .unwrap(),
        "body"
    );
    assert_eq!(
        client
            .get(&format!("{base}/next"), &[], None)
            .unwrap()
            .into_string()
            .unwrap(),
        "done"
    );
    server.join().unwrap();
}

#[test]
fn medium_idle_limit_and_total_deadline_are_independent() {
    for (deadline, expected) in [
        (None, Ok("abcd")),
        (
            Some(Duration::from_millis(350)),
            Err(std::io::ErrorKind::TimedOut),
        ),
    ] {
        let (listener, base) = listener();
        let server = thread::spawn(move || {
            let mut stream = accept(&listener);
            read_request(&mut stream);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                .unwrap();
            for byte in b"abcd" {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(200));
            }
        });
        let client = client_with_timeout(Duration::from_millis(450));
        let result = client.get(&base, &[], deadline).unwrap().into_string();
        match expected {
            Ok(text) => assert_eq!(result.unwrap(), text),
            Err(kind) => assert_eq!(result.unwrap_err().kind(), kind),
        }
        server.join().unwrap();
    }
}

#[test]
fn medium_idle_body_timeout_reports_stall() {
    let (listener, base) = listener();
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\na")
            .unwrap();
        thread::sleep(Duration::from_millis(400));
    });
    let client = client_with_timeout(Duration::from_millis(150));
    let response = client.get(&base, &[], None).unwrap();
    assert_eq!(
        response.into_string().unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    server.join().unwrap();
}

struct TransportLog(Mutex<Vec<String>>);

impl log::Log for TransportLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.target().starts_with("reqwest") || metadata.target().starts_with("hyper")
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0.lock().unwrap().push(format!("{}", record.args()));
        }
    }

    fn flush(&self) {}
}

static TRANSPORT_LOG: TransportLog = TransportLog(Mutex::new(Vec::new()));

#[test]
fn medium_http_proxy_and_server_authorization_survive_redacted_errors() {
    log::set_logger(&TRANSPORT_LOG).unwrap();
    log::set_max_level(log::LevelFilter::Trace);
    let (listener, proxy) = listener();
    let proxy = proxy.replacen("http://", "http://proxy-user:proxy-password@", 1);
    let server = thread::spawn(move || {
        let mut stream = accept(&listener);
        let request = read_request(&mut stream);
        assert!(request.starts_with("GET http://example.test/path?token=query-marker HTTP/1.1\r\n"));
        assert!(request.contains("authorization: Bearer explicit-header-marker\r\n"));
        assert!(
            request.contains("proxy-authorization: Basic cHJveHktdXNlcjpwcm94eS1wYXNzd29yZA==\r\n")
        );
        assert_eq!(request.matches("\r\nauthorization:").count(), 1);
        stream
            .write_all(b"HTTP/1.1 401 reply-secret-marker\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });
    let proxied =
        NetworkClient::from_environment(&|name| (name == "http_proxy").then(|| proxy.clone()))
            .unwrap();
    let error = proxied
        .get(
            "http://request-user:request-password@example.test/path?token=query-marker",
            &[
                ("Authorization", "old-header-marker"),
                ("authorization", "Bearer explicit-header-marker"),
            ],
            None,
        )
        .unwrap_err();
    assert_eq!(error.status(), Some(401));
    let logged = TRANSPORT_LOG.0.lock().unwrap().join("\n");
    assert!(logged.contains("intercepts"), "{logged}");
    let diagnostic = format!("{error} {error:?} {logged}");
    for secret in [
        "request-user",
        "request-password",
        "proxy-user",
        "proxy-password",
        "old-header-marker",
        "explicit-header-marker",
        "reply-secret-marker",
        "query-marker",
    ] {
        assert!(!diagnostic.contains(secret), "{diagnostic}");
    }
    server.join().unwrap();

    let stand = test_support::HttpStand::start("ok", "text/plain");
    let url = stand.url("root").replacen("http://", "http://u:p@", 1);
    assert_eq!(
        client_with_timeout(READ_TIMEOUT)
            .get(&url, &[], None)
            .unwrap()
            .into_string()
            .unwrap(),
        "ok"
    );
    assert_eq!(
        stand.header("authorization"),
        vec![vec!["Basic dTpw".to_string()]]
    );
}
