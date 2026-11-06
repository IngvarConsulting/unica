use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, Once};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct DiagnosticLogger(Mutex<Vec<String>>);
static LOGGER: DiagnosticLogger = DiagnosticLogger(Mutex::new(Vec::new()));
static LOGGING: Once = Once::new();
impl log::Log for DiagnosticLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Debug && metadata.target().starts_with("ureq")
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.0
                .lock()
                .unwrap()
                .push(format!("{}: {}", record.target(), record.args()));
        }
    }
    fn flush(&self) {}
}
fn logging() {
    LOGGING.call_once(|| {
        log::set_logger(&LOGGER).unwrap();
        log::set_max_level(log::LevelFilter::Debug);
    });
}
fn diagnostics() -> String {
    LOGGER.0.lock().unwrap().join("\n")
}
fn hidden(text: &str, markers: &[&str]) {
    for marker in markers {
        assert!(
            !text.contains(marker),
            "diagnostic disclosed synthetic marker {}: {}",
            marker,
            text
        );
    }
}
fn builder() -> ureq::AgentBuilder {
    logging();
    ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .timeout(Duration::from_secs(5))
}
fn headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16_384);
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}
fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(n, v)| n.eq_ignore_ascii_case(name).then_some(v.trim()))
}
struct Server {
    address: SocketAddr,
    cancelled: Arc<AtomicBool>,
    received: mpsc::Receiver<Vec<String>>,
    worker: Option<JoinHandle<()>>,
}
impl Server {
    fn new(replies: Vec<String>) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&cancelled);
        let (send, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let (mut stream, peer) = listener.accept().unwrap();
                if stop.load(Ordering::Acquire) {
                    return;
                }
                assert!(peer.ip().is_loopback());
                requests.push(headers(&mut stream));
                stream.write_all(reply.as_bytes()).unwrap();
            }
            let _ = send.send(requests);
        });
        Self {
            address,
            cancelled,
            received,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }
    fn requests(&self) -> Vec<String> {
        self.received.recv_timeout(Duration::from_secs(10)).unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn response(status: &str, extra: &str) -> String {
    format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n")
}

#[test]
fn request_and_request_url_diagnostics_preserve_wire_values_without_credentials() {
    let server = Server::new(vec![response("200 OK", "")]);
    let url = format!(
        "http://RequestUser1792:RequestPass1792@{}/request?visible=retained",
        server.address
    );
    let request = builder()
        .build()
        .get(&url)
        .set("aUtHoRiZaTiOn", "Bearer RequestAuth1792")
        .set("pRoXy-AuThOrIzAtIoN", "Basic RequestProxyHeader1792")
        .set("cOoKiE", "session=RequestCookie1792")
        .set("X-Visible", "retained");
    let parsed = request.request_url().unwrap();
    assert_eq!(request.url(), url);
    assert_eq!(parsed.as_url().username(), "RequestUser1792");
    assert_eq!(parsed.as_url().password(), Some("RequestPass1792"));
    assert_eq!(parsed.query_pairs(), vec![("visible", "retained")]);
    let shown = format!("{request:?} {parsed:?}");
    let response = request.call().unwrap();
    assert_eq!(response.status(), 200);
    let requests = server.requests();
    assert_eq!(
        header(&requests[0], "Authorization"),
        Some("Bearer RequestAuth1792")
    );
    assert_eq!(
        header(&requests[0], "Proxy-Authorization"),
        Some("Basic RequestProxyHeader1792")
    );
    assert_eq!(
        header(&requests[0], "Cookie"),
        Some("session=RequestCookie1792")
    );
    assert_eq!(
        requests[0].lines().next(),
        Some("GET /request?visible=retained HTTP/1.1")
    );
    hidden(
        &shown,
        &[
            "RequestUser1792",
            "RequestPass1792",
            "RequestAuth1792",
            "RequestProxyHeader1792",
            "RequestCookie1792",
        ],
    );
    assert!(shown.contains("X-Visible: retained"));
    hidden(
        &diagnostics(),
        &[
            "RequestUser1792",
            "RequestPass1792",
            "RequestAuth1792",
            "RequestProxyHeader1792",
            "RequestCookie1792",
        ],
    );
    for url in [
        "http://Encoded%55ser1792:Encoded%50ass1792@127.0.0.1/a",
        "http://:OnlyPass1792@127.0.0.1/a",
        "http://MalformedUser1792:MalformedPass1792@127.0.0.1:bad/a",
    ] {
        let request = ureq::get(url);
        let shown = format!("{request:?}");
        hidden(
            &shown,
            &[
                "Encoded%55ser1792",
                "Encoded%50ass1792",
                "OnlyPass1792",
                "MalformedUser1792",
                "MalformedPass1792",
            ],
        );
        if let Ok(parsed) = request.request_url() {
            hidden(
                &format!("{parsed:?}"),
                &["Encoded%55ser1792", "Encoded%50ass1792", "OnlyPass1792"],
            );
        }
    }
    let hostless = "TypoUser1792:TypoPass1792@127.0.0.1/path";
    let request = ureq::get(hostless);
    assert_eq!(request.url(), hostless);
    assert_eq!(
        request.request_url().unwrap_err().kind(),
        ureq::ErrorKind::InvalidUrl
    );
    hidden(&format!("{request:?}"), &["TypoUser1792", "TypoPass1792"]);
}

#[test]
fn response_and_status_error_diagnostics_hide_url_and_redirect_history() {
    // A separate successful response checks the raw response URL accessor.
    let success = Server::new(vec![response("200 OK", "")]);
    let raw_url = format!(
        "http://ResponseUser1792:ResponsePass1792@{}/response",
        success.address
    );
    let result = builder().build().get(&raw_url).call().unwrap();
    assert_eq!(result.get_url(), raw_url);
    let response_debug = format!("{result:?}");
    success.requests();
    hidden(&response_debug, &["ResponseUser1792", "ResponsePass1792"]);
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        headers(&mut stream);
        stream.write_all(response("302 Found",&format!("Location: http://StatusNewUser1792:StatusNewPass1792@{address}/status-final\r\n")).as_bytes()).unwrap();
        let (mut stream, _) = listener.accept().unwrap();
        headers(&mut stream);
        stream
            .write_all(response("404 Missing", "").as_bytes())
            .unwrap();
    });
    let error = builder()
        .build()
        .get(&format!(
            "http://StatusOldUser1792:StatusOldPass1792@{address}/status-start"
        ))
        .call()
        .unwrap_err();
    worker.join().unwrap();
    hidden(
        &format!("{error} {error:?}"),
        &[
            "StatusOldUser1792",
            "StatusOldPass1792",
            "StatusNewUser1792",
            "StatusNewPass1792",
        ],
    );
    assert!(error.to_string().contains("status code 404"));
    assert!(error.to_string().contains("redirected from"));
}

#[test]
fn invalid_sensitive_headers_and_redirect_locations_hide_values() {
    for name in [
        "Authorization",
        "pRoXy-AuThOrIzAtIoN",
        "cOoKiE",
        "Authorization ",
        "Proxy-Authorization ",
        "Cookie ",
    ] {
        let request = builder()
            .build()
            .get("http://127.0.0.1:1/no-network")
            .set(name, "InvalidHeader1792\r\nvalue");
        hidden(&format!("{request:?}"), &["InvalidHeader1792"]);
        let error = request.call().unwrap_err();
        assert_eq!(error.kind(), ureq::ErrorKind::BadHeader);
        hidden(&format!("{error} {error:?}"), &["InvalidHeader1792"]);
    }
    let injected = builder().build().get("http://127.0.0.1:1/no-network").set(
        "X-Note",
        "public\r\nAuthorization: Bearer InjectedSecret1792",
    );
    let shown = format!("{injected:?}");
    let error = injected.call().unwrap_err();
    assert_eq!(error.kind(), ureq::ErrorKind::BadHeader);
    hidden(&shown, &["InjectedSecret1792"]);
    hidden(&format!("{error} {error:?}"), &["InjectedSecret1792"]);
    let location = "http://LocationUser1792:LocationPass1792@127.0.0.1:bad/next";
    let server = Server::new(vec![response(
        "302 Found",
        &format!("Location: {location}\r\n"),
    )]);
    let error = builder()
        .build()
        .get(&server.url("/invalid-location"))
        .call()
        .unwrap_err();
    server.requests();
    hidden(
        &format!("{error} {error:?}"),
        &["LocationUser1792", "LocationPass1792"],
    );
    assert_eq!(error.kind(), ureq::ErrorKind::InvalidUrl);
}

#[test]
fn redirect_log_events_hide_both_urls_and_preserve_authentication_policies() {
    for keep in [false, true] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let worker = thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(headers(&mut stream));
                let reply = if index == 0 {
                    response("302 Found",&format!("Location: http://RedirectNewUser1792:RedirectNewPass1792@{address}/redirect-final\r\n"))
                } else {
                    response("200 OK", "")
                };
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        let mut agent = builder();
        if keep {
            agent = agent.redirect_auth_headers(ureq::RedirectAuthHeaders::SameHost);
        }
        agent
            .build()
            .get(&format!(
                "http://RedirectOldUser1792:RedirectOldPass1792@{address}/redirect-start"
            ))
            .set("Authorization", "Bearer RedirectExplicit1792")
            .call()
            .unwrap();
        let requests = worker.join().unwrap();
        assert_eq!(
            header(&requests[0], "Authorization"),
            Some("Bearer RedirectExplicit1792")
        );
        let new_basic = format!(
            "Basic {}",
            STANDARD.encode("RedirectNewUser1792:RedirectNewPass1792")
        );
        assert_eq!(
            header(&requests[1], "Authorization"),
            Some(if keep {
                "Bearer RedirectExplicit1792"
            } else {
                new_basic.as_str()
            })
        );
    }
    let logs = diagnostics();
    let event = logs
        .lines()
        .filter(|line| line.contains("redirect 302") && line.contains("/redirect-start"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !event.is_empty(),
        "the redirect log sink must actually execute"
    );
    hidden(
        &event,
        &[
            "RedirectOldUser1792",
            "RedirectOldPass1792",
            "RedirectNewUser1792",
            "RedirectNewPass1792",
        ],
    );
    hidden(&logs, &["RedirectExplicit1792"]);
}

#[test]
fn proxy_and_transport_diagnostics_preserve_connect_authentication() {
    let proxy_server = Server::new(vec![response("407 Synthetic Stop", "")]);
    let proxy = ureq::Proxy::new(format!(
        "http://ProxyUser1792:ProxyPass1792@{}",
        proxy_server.address
    ))
    .unwrap();
    let agent = builder().proxy(proxy.clone());
    let proxy_debug = format!("{proxy:?} {agent:?}");
    let error = agent
        .build()
        .get("https://TransportUser1792:TransportPass1792@127.0.0.1:9/transport")
        .call()
        .unwrap_err();
    let requests = proxy_server.requests();
    assert_eq!(
        requests[0].lines().next(),
        Some("CONNECT 127.0.0.1:9 HTTP/1.1")
    );
    let expected = format!("basic {}", STANDARD.encode("ProxyUser1792:ProxyPass1792"));
    assert_eq!(
        header(&requests[0], "Proxy-Authorization"),
        Some(expected.as_str())
    );
    hidden(&diagnostics(), &[expected.strip_prefix("basic ").unwrap()]);
    assert_eq!(error.kind(), ureq::ErrorKind::ProxyUnauthorized);
    let transport = error.into_transport().unwrap();
    assert_eq!(transport.url().unwrap().username(), "TransportUser1792");
    hidden(&proxy_debug, &["ProxyUser1792", "ProxyPass1792"]);
    hidden(
        &format!("{transport} {transport:?}"),
        &["TransportUser1792", "TransportPass1792"],
    );
}

#[test]
fn reused_connection_and_retry_log_events_hide_userinfo() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let worker = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let mut requests = vec![headers(&mut first)];
        first
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
        requests.push(headers(&mut first));
        drop(first);
        let (mut fresh, _) = listener.accept().unwrap();
        requests.push(headers(&mut fresh));
        fresh.write_all(response("200 OK", "").as_bytes()).unwrap();
        requests
    });
    let agent = builder().build();
    agent
        .get(&format!("http://{address}/warmup"))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    agent
        .get(&format!(
            "http://ReusedUser1792:ReusedPass1792@{address}/reused"
        ))
        .call()
        .unwrap()
        .into_string()
        .unwrap();
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 3);
    let expected = format!("Basic {}", STANDARD.encode("ReusedUser1792:ReusedPass1792"));
    for request in &requests[1..] {
        assert_eq!(header(request, "Authorization"), Some(expected.as_str()));
    }
    let logs = diagnostics();
    assert!(logs.lines().any(
        |line| line.contains("sending request (reused connection)") && line.contains("/reused")
    ));
    assert!(logs
        .lines()
        .any(|line| line.contains("retrying request ") && line.contains("/reused")));
    hidden(&logs, &["ReusedUser1792", "ReusedPass1792"]);
    hidden(&logs, &[expected.strip_prefix("Basic ").unwrap()]);
}
