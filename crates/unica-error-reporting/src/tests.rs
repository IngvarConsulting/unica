use super::*;
use sentry::protocol::{Attachment, EnvelopeHeaders, Transaction};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

fn event(kind: &str) -> Event<'static> {
    Event {
        message: Some(kind.to_owned()),
        ..Event::default()
    }
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn collection_is_default_on_and_opt_out_fails_closed() {
    assert!(enabled(None));
    for value in ["1", "true", "ON", " true "] {
        assert!(enabled(Some(value)));
    }
    for value in ["0", "false", "off", "", "yes", "typo"] {
        assert!(!enabled(Some(value)));
    }
    let guard = init_if_enabled(false, Component::Core, || {
        panic!("disabled reporting must not create a transport")
    });
    assert!(guard.0.is_none());
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn unknown_errors_and_expected_user_failures_are_not_reported() {
    for value in [
        "bad_input",
        "file_not_found",
        "cancelled",
        "connection_refused",
        "SECRET",
    ] {
        assert!(sanitise(event(value), Component::Core).is_none());
    }
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn serialized_envelope_drops_private_scope_headers_attachments_and_context() {
    let secret = "SECRET_USER_INPUT";
    let payload = json!({
        "message":secret,"platform":secret,"release":secret,"environment":secret,
        "server_name":secret,"transaction":secret,"culprit":secret,"logger":secret,
        "user":{"username":secret,"email":secret,"ip_address":"127.0.0.1"},
        "request":{"url":format!("https://{secret}"),"data":secret},
        "tags":{"password":secret},"extra":{"token":secret},
        "contexts":{"custom":{"secret":secret}},
        "breadcrumbs":{"values":[{"message":secret}]},
        "sdk":{"name":secret,"version":secret},
        "exception":{"values":[{"type":"panic","value":secret,"module":secret,
            "stacktrace":{"frames":[{
                "function":"unica_product::known_function","filename":secret,"abs_path":secret,
                "package":secret,"module":secret,"context_line":secret,
                "pre_context":[secret],"post_context":[secret],"vars":{"secret":secret},
                "lineno":42,"colno":3
            },{"function":"unica::main","filename":secret,"abs_path":secret},
            {"function":secret,"filename":secret}]},
            "raw_stacktrace":{"frames":[{"function":secret}]}}]}
    });
    let raw: Event<'static> = serde_json::from_value(payload).unwrap();
    let mut envelope = Envelope::new().with_headers(
        EnvelopeHeaders::new().with_dsn("https://SECRET_USER_INPUT@example.org/1".parse().unwrap()),
    );
    envelope.add_item(raw);
    envelope.add_item(Attachment {
        buffer: secret.as_bytes().to_vec(),
        filename: secret.to_owned(),
        ..Attachment::default()
    });
    envelope.add_item(Transaction::default());
    let (send, receive) = mpsc::channel();
    let transport = QueuedTransport::new(Component::Core, move |body| {
        send.send(body).unwrap();
    });
    transport.send_envelope(envelope);
    assert!(transport.flush(Duration::from_secs(1)));
    let body = receive.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!body.contains(secret), "payload leaked user data");
    for forbidden in [
        "abs_path",
        "context_line",
        "pre_context",
        "post_context",
        "request",
        "server_name",
        "username",
        "attachment",
        "transaction",
        "raw_stacktrace",
    ] {
        assert!(!body.contains(forbidden), "unexpected field {forbidden}");
    }
    let safe = Envelope::from_slice(body.as_bytes()).unwrap();
    assert_eq!(safe.items().count(), 1);
    let event = safe.event().unwrap();
    assert_eq!(event.message.as_deref(), Some("rust_panic"));
    let frame = &event.exception.values[0]
        .stacktrace
        .as_ref()
        .unwrap()
        .frames[0];
    assert_eq!(
        frame.function.as_deref(),
        Some("unica_product::known_function")
    );
    assert_eq!(frame.lineno, Some(42));
    let frames = &event.exception.values[0]
        .stacktrace
        .as_ref()
        .unwrap()
        .frames;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1].function.as_deref(), Some("unica::main"));
    assert!(receive.try_recv().is_err());
    assert!(transport.shutdown(Duration::from_secs(1)));
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn full_queue_and_shutdown_deadline_do_not_wait_for_network() {
    let (started_send, started_receive) = mpsc::channel();
    let (release_send, release_receive) = mpsc::channel();
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = Arc::clone(&calls);
    let transport = Arc::new(QueuedTransport::new(Component::Core, move |_| {
        if worker_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            started_send.send(()).unwrap();
            release_receive.recv().unwrap();
        }
    }));
    transport.send_envelope(event("bootstrap_internal").into());
    started_receive
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    let started = Instant::now();
    for index in 0..128 {
        let mut event = event("rust_panic");
        event.exception = vec![Exception {
            ty: "panic".to_owned(),
            stacktrace: Some(Stacktrace {
                frames: vec![Frame {
                    function: Some(format!("unica_product::function_{index}")),
                    ..Frame::default()
                }],
                ..Stacktrace::default()
            }),
            ..Exception::default()
        }]
        .into();
        transport.send_envelope(event.into());
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "capture blocked on collector"
    );
    let started = Instant::now();
    assert!(!transport.shutdown(Duration::from_millis(20)));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "shutdown exceeded deadline"
    );
    release_send.send(()).unwrap();
    assert!(transport.flush(Duration::from_secs(1)));
    assert!(calls.load(Ordering::SeqCst) <= QUEUE_CAPACITY + 1);
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn duplicate_events_and_events_after_shutdown_are_dropped() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker_calls = Arc::clone(&calls);
    let transport = QueuedTransport::new(Component::Bootstrap, move |_| {
        worker_calls.fetch_add(1, Ordering::SeqCst);
    });
    for _ in 0..100 {
        transport.send_envelope(event("bootstrap_internal").into());
    }
    assert!(transport.flush(Duration::from_secs(1)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(transport.shutdown(Duration::from_secs(1)));
    transport.send_envelope(event("mcp_runtime_start").into());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
fn medium_process_environment_optout_installs_no_client_or_transport() {
    let values = ["0", "false", "off", "unrecognised"];
    for value in values {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::medium_optout_child", "--ignored"])
            .env("UNICA_ERROR_REPORTING", value)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child opt-out failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
    }
}

// rule: INV.PKG.INTERNAL-ERROR-REPORTING
#[test]
#[ignore = "helper process for the actual environment opt-out path"]
fn medium_optout_child() {
    let guard = init(Component::Core);
    assert!(
        guard.0.is_none(),
        "environment opt-out must not initialise collection"
    );
    assert!(
        sentry::Hub::main().client().is_none(),
        "disabled reporting installed an SDK client"
    );
    capture_internal(InternalError::BootstrapInternal);
    flush_before_exit();
    guard.shutdown(Duration::from_millis(1));
}
