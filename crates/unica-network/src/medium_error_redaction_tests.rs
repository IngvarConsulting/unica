use super::*;
use crate::test_support::{HttpStand, TestRoot, TlsStand};
use std::io::{BufRead, BufReader};
use std::net::TcpListener;
fn client() -> NetworkClient {
    NetworkClient::from_environment(&|_| None).expect("direct client")
}
fn assert_hidden(error: &NetworkError, markers: &[&str]) {
    for shown in [
        error.message().to_owned(),
        error.to_string(),
        format!("{error:?}"),
    ] {
        for marker in markers {
            assert!(!shown.contains(marker), "credential {marker} in {shown}");
        }
    }
}
fn marked_url(base: &str, user: &str, password: &str) -> String {
    let mut url = url::Url::parse(base).unwrap();
    url.set_username(user).unwrap();
    url.set_password(Some(password)).unwrap();
    url.into()
}
#[test]
fn http_status_errors_hide_userinfo_and_keep_wire_basic_authentication() {
    for status in [401, 404] {
        let stand = HttpStand::respond(status, &[], Vec::new(), "text/plain");
        let endpoint = marked_url(&stand.url("status"), "NetworkUser2566", "NetworkPass2566");
        let error = client()
            .get(&endpoint, &[], Some(Duration::from_secs(5)))
            .unwrap_err();
        assert_eq!(error.failure(), NetworkFailure::Status(status));
        assert_eq!(error.status(), Some(status));
        assert_eq!(stand.requests(), vec!["GET /status HTTP/1.1"]);
        assert_eq!(
            stand.header("Authorization"),
            vec![vec![
                "Basic TmV0d29ya1VzZXIyNTY2Ok5ldHdvcmtQYXNzMjU2Ng==".to_owned()
            ]]
        );
        for safe in [
            "127.0.0.1",
            "/status",
            "direct connection",
            &format!("HTTP {status}"),
        ] {
            assert!(error.message().contains(safe), "{}", error.message());
        }
        assert_hidden(&error, &["NetworkUser2566", "NetworkPass2566"]);
    }
}
#[test]
fn invalid_url_and_location_diagnostics_hide_each_userinfo_and_keep_failure_classes() {
    let error = client()
        .get(
            "http://InvalidUser2566:InvalidPass2566@127.0.0.1:bad/start",
            &[],
            None,
        )
        .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::Configuration);
    assert_hidden(&error, &["InvalidUser2566", "InvalidPass2566"]);
    let location = "http://LocationUser2566:LocationPass2566@127.0.0.1:bad/final";
    let stand = HttpStand::respond(302, &[("Location", location)], Vec::new(), "text/plain");
    let endpoint = marked_url(
        &stand.url("start"),
        "LocationSourceUser2566",
        "LocationSourcePass2566",
    );
    let error = client()
        .get(&endpoint, &[], Some(Duration::from_secs(5)))
        .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::Transport);
    for safe in ["invalid location", "127.0.0.1", "/start"] {
        assert!(error.message().contains(safe), "{}", error.message());
    }
    assert_hidden(
        &error,
        &[
            "LocationUser2566",
            "LocationPass2566",
            "LocationSourceUser2566",
            "LocationSourcePass2566",
        ],
    );
    let error = client()
        .get("TypoUser2566:TypoPass2566@127.0.0.1/path", &[], None)
        .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::Transport);
    assert_hidden(&error, &["TypoUser2566", "TypoPass2566"]);
}
#[test]
fn redirect_limit_error_hides_userinfo_and_keeps_the_transport_class() {
    let stand = HttpStand::respond(302, &[("Location", "/again")], Vec::new(), "text/plain");
    let endpoint = marked_url(&stand.url("again"), "LimitUser2566", "LimitPass2566");
    let error = client()
        .get(&endpoint, &[], Some(Duration::from_secs(5)))
        .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::Transport);
    assert_eq!(stand.requests().len(), MAX_REDIRECTS + 1);
    for safe in ["redirects", "/again"] {
        assert!(error.message().contains(safe));
    }
    assert_hidden(&error, &["LimitUser2566", "LimitPass2566"]);
}
#[test]
fn https_downgrade_diagnostics_hide_both_urls_without_changing_the_class() {
    let current =
        url::Url::parse("https://SecureUser2566:SecurePass2566@secure.example/start").unwrap();
    let error = next_hop(
        &current,
        "http://PlainUser2566:PlainPass2566@plain.example/final",
    )
    .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::InsecureRedirect);
    for safe in ["secure.example/start", "plain.example/final", "non-HTTPS"] {
        assert!(error.message().contains(safe));
    }
    assert_hidden(
        &error,
        &[
            "SecureUser2566",
            "SecurePass2566",
            "PlainUser2566",
            "PlainPass2566",
        ],
    );
}
#[test]
fn peer_closed_transport_error_hides_userinfo_and_retains_a_typed_io_cause() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let worker = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap() > 2 {
            line.clear();
        }
    });
    let error = client()
        .get(
            &format!("http://ClosedUser2566:ClosedPass2566@{address}/closed"),
            &[],
            Some(Duration::from_secs(5)),
        )
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(error.failure(), NetworkFailure::Transport);
    assert_hidden(&error, &["ClosedUser2566", "ClosedPass2566"]);
    for safe in ["ConnectionAborted", "127.0.0.1", "direct connection"] {
        assert!(error.message().contains(safe), "{}", error.message());
    }
}
#[test]
fn untrusted_tls_error_hides_userinfo_and_keeps_interception_cure() {
    let root = TestRoot::generate("Unica 2566 untrusted root");
    let stand = TlsStand::start(&root, Vec::new(), "text/plain");
    let endpoint = marked_url(&stand.url("tls"), "TlsUser2566", "TlsPass2566");
    let error = client()
        .get(&endpoint, &[], Some(Duration::from_secs(5)))
        .unwrap_err();
    assert_eq!(error.failure(), NetworkFailure::UntrustedCertificate);
    assert!(stand.connections() > 0);
    assert!(stand.requests().is_empty());
    for safe in ["HTTPS_PROXY", "trust store"] {
        assert!(error.cure().unwrap().contains(safe));
    }
    for safe in ["certificate", "127.0.0.1", "direct connection"] {
        assert!(error.message().contains(safe));
    }
    assert_hidden(&error, &["TlsUser2566", "TlsPass2566"]);
}
