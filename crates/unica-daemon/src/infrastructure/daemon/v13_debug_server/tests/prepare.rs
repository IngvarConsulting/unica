use super::*;

#[test]
fn prepare_accepts_only_the_session_owner_for_the_project_infobase() {
    let root = tempfile::tempdir().unwrap();
    let parse = |args| {
        let invocation = InvocationRequest::new(
            ToolIdentity::Run,
            json!({"op":PREPARE, "args":args}),
            root.path().to_string_lossy(),
            7000,
        )
        .unwrap();
        PreparedDebugServer::parse(&invocation, PREPARE)
    };
    let prepared = parse(json!({"ownerPid":42})).unwrap();
    assert!(matches!(
        prepared.request,
        Request::Prepare { owner_pid: 42 }
    ));
    for args in [
        json!({}),
        json!({"ownerPid":0}),
        json!({"ownerPid":"42"}),
        json!({"ownerPid":4294967296u64}),
        json!({"ownerPid":42,"mode":"client"}),
        json!({"ownerPid":42,"port":1560}),
        json!({"ownerPid":42,"clientOnly":true}),
        json!({"ownerPid":42,"debugServerUrl":"http://127.0.0.1:1560"}),
        json!({"ownerPid":42,"infoBaseAlias":"another"}),
    ] {
        let rejection = parse(args).unwrap_err();
        assert_eq!(rejection.diagnostics[0]["code"], "bad_value");
    }
}

#[test]
fn prepare_returns_the_attested_scope_and_alias_of_the_borrowed_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    let endpoint = "http://dbgs.example.test:1560";
    for owner in ["cluster", "standalone", "external"] {
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![envelope(
                "prepare",
                json!({"ok":true,
                "action":"prepared", "url":endpoint, "infobase_alias":"Приёмка", "scope":"client_and_server", "owner":owner, "release":null}),
            )]),
            cancel_during_reply: None,
        };
        let result = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
        assert_eq!(
            result.data.unwrap(),
            json!({"op":PREPARE,"contractVersion":1,
            "action":"prepared","url":endpoint,"infoBaseAlias":"Приёмка","scope":"clientAndServer","release":null})
        );
        assert!(result.changed.is_empty());
        let calls = runner.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0].0,
            vec![
                "--config".to_owned(),
                prepared
                    .context
                    .workspace_root
                    .join(CONFIG_NAME)
                    .display()
                    .to_string(),
                "--infobase".into(),
                "origin".into(),
                "--json-message".into(),
                "debug-server".into(),
                "prepare".into(),
                "--owner-pid".into(),
                "42".into()
            ]
        );
        assert!(calls[0].1);
    }
}

#[test]
fn prepare_never_tries_another_server_after_refusal_or_invalid_evidence() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    let borrowed = json!({"ok":true, "action":"prepared", "url":"http://dbgs.example.test:80",
        "infobase_alias":"Ref-A", "scope":"client_and_server", "owner":"external", "release":null});
    let mut invalid_replies = vec![];
    for (key, value) in [
        ("scope", json!("unknown")),
        ("scope", json!("client")),
        ("infobase_alias", Value::Null),
        ("url", json!("http://host:1550/extra")),
        ("action", json!("planned")),
        (
            "release",
            json!({"op":"something-else","args":{"session":SESSION}}),
        ),
    ] {
        let mut data = borrowed.clone();
        data[key] = value;
        invalid_replies.push(envelope("prepare", data));
    }
    let mut omitted_release = borrowed;
    omitted_release.as_object_mut().unwrap().remove("release");
    invalid_replies.push(envelope("prepare", omitted_release));
    for reply in invalid_replies {
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![reply]),
            cancel_during_reply: None,
        };
        let result = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        assert_eq!(runner.calls.borrow().len(), 1);
    }
    for code in [
        "debug_server_external_unavailable",
        "debug_server_external_target_unverified",
    ] {
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![json!({
                "command":"debug-server prepare","ok":false,"error":{"code":code, "message":"private connection information"}})]),
            cancel_during_reply: None,
        };
        let refused = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
        assert_eq!(refused.data.unwrap(), json!({"runnerCode":code}));
        assert!(!refused.summary.contains("private connection"));
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}

#[test]
fn prepare_preview_returns_no_endpoint_scope_or_release() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    prepared.dry_run = true;
    let runner = Provider {
        calls: RefCell::default(),
        results: RefCell::new(vec![envelope(
            "prepare",
            json!({"ok":true,"action":"planned"}),
        )]),
        cancel_during_reply: None,
    };
    let result = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
    assert_eq!(
        result.data.unwrap(),
        json!({"op":PREPARE,"contractVersion":1,"action":"planned"})
    );
    assert!(result.changed.is_empty());
    assert!(runner.calls.borrow()[0]
        .0
        .iter()
        .any(|arg| arg == "--dry-run"));
    assert!(!runner.calls.borrow()[0].1);
}

fn owned_prepare(session: &str) -> Value {
    envelope(
        "prepare",
        json!({"action":"prepared","url":"http://127.0.0.1:1550",
        "infobase_alias":"DefAlias","scope":"client","owner":"runner","owner_pid":42,
        "release":{"action":"stop","session":session}}),
    )
}

#[test]
fn owned_servers_report_the_attested_file_emulation_or_thin_client_scope() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    for (url, wire_scope, public_scope) in [
        (
            "http://localhost:1550",
            "client_and_server",
            "clientAndServer",
        ),
        ("http://[::1]:1550", "client", "client"),
    ] {
        let mut data = owned_prepare(SESSION);
        data["data"]["url"] = json!(url);
        data["data"]["scope"] = json!(wire_scope);
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![data]),
            cancel_during_reply: None,
        };
        let result = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
        assert_eq!(
            result.data.unwrap(),
            json!({"op":PREPARE,"contractVersion":1,"action":"prepared",
            "url":url,"scope":public_scope,"infoBaseAlias":"DefAlias","release":{"op":STOP,"args":{"session":SESSION}}})
        );
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}

#[test]
fn prepared_borrowed_servers_stay_running_when_handoff_is_cancelled() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    for owner in ["external", "cluster", "standalone"] {
        let cancellation = CancellationToken::new();
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![envelope(
                "prepare",
                json!({"action":"prepared","url":"http://cluster.example.test:1561",
                "infobase_alias":"db","scope":"client_and_server","owner":owner,"release":null}),
            )]),
            cancel_during_reply: Some(cancellation.clone()),
        };
        let result = prepared.prepare(&tool, &runner, &cancellation, 42);
        assert_eq!(result.diagnostics[0]["code"], "cancelled");
        assert_eq!(runner.calls.borrow().len(), 1, "{owner}");
    }
}

#[test]
fn invalid_scope_releases_only_an_attested_new_lease_and_exposes_unconfirmed_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    let mut reply = owned_prepare(SESSION);
    reply["data"]["scope"] = json!("unverified");
    let mut stale_stop = stop_data(SESSION);
    stale_stop["data"]["stale_record"] = json!(true);
    let mut no_effect_with_pid = stop_data(SESSION);
    no_effect_with_pid["data"]["action"] = json!("nothing_to_stop");
    for (response, expected_code) in [
        (stop_data(SESSION), "invalid_result"),
        (stop_data("another-token"), "provider_failed"),
        (stale_stop, "provider_failed"),
        (no_effect_with_pid, "provider_failed"),
    ] {
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![reply.clone(), response]),
            cancel_during_reply: None,
        };
        let result = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
        assert_eq!(result.diagnostics[0]["code"], expected_code);
        assert!(runner.calls.borrow()[1]
            .0
            .ends_with(&["stop".into(), SESSION.into()]));
        if expected_code == "provider_failed" {
            assert_eq!(
                result.data.unwrap()["release"],
                json!({"op":STOP,"args":{"session":SESSION}})
            );
        }
    }
}

#[test]
fn cancelling_a_second_prepare_releases_only_its_own_lease() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    let second = "da77c727b94944e4b696667ba84caded";
    let released = envelope(
        "stop",
        json!({"ok":true,"action":"released",
        "session":second,"known_session":true}),
    );
    let replay = envelope(
        "stop",
        json!({"ok":true,"action":"nothing_to_stop",
        "session":SESSION,"known_session":false}),
    );
    let mut runner = Provider {
        calls: RefCell::default(),
        results: RefCell::new(vec![
            owned_prepare(SESSION),
            owned_prepare(second),
            released,
            stop_data(SESSION),
            replay,
        ]),
        cancel_during_reply: None,
    };
    let first = prepared.prepare(&tool, &runner, &CancellationToken::new(), 42);
    let release = &first.data.unwrap()["release"];
    assert_eq!(*release, json!({"op":STOP, "args":{"session":SESSION}}));
    let cancellation = CancellationToken::new();
    runner.cancel_during_reply = Some(cancellation.clone());
    let cancelled = prepared.prepare(&tool, &runner, &cancellation, 42);
    assert_eq!(cancelled.diagnostics[0]["code"], "cancelled");
    assert_eq!(runner.calls.borrow().len(), 3);
    assert!(runner.calls.borrow()[2]
        .0
        .ends_with(&["stop".into(), second.into()]));
    assert_eq!(
        prepared
            .stop(
                &tool,
                &runner,
                SESSION,
                false,
                true,
                &CancellationToken::new()
            )
            .data
            .unwrap()["action"],
        "stopped"
    );
    let again = prepared.stop(
        &tool,
        &runner,
        SESSION,
        false,
        true,
        &CancellationToken::new(),
    );
    assert_eq!(again.data.unwrap()["action"], "nothing_to_stop");
    assert!(again.changed.is_empty());
}

#[test]
fn prepare_rejects_foreign_and_malformed_release_authority_without_dispatching_stop() {
    let root = tempfile::tempdir().unwrap();
    let (mut prepared, tool) = fixture(root.path());
    prepared.request = Request::Prepare { owner_pid: 42 };
    for (field, value) in [
        ("owner", json!("external")),
        ("owner", json!("cluster")),
        ("owner", json!("standalone")),
        ("owner", json!("unknown")),
        ("owner_pid", json!(43)),
        ("url", json!("http://other.example.test:1550")),
        ("url", json!("http://127.deceptive.example.test:1550")),
        ("release", Value::Null),
        ("release", json!({"action":"shutdown","session":SESSION})),
        ("release", json!({"action":"stop","session":"invalid"})),
        ("release", json!({"untrusted":"stop","session":SESSION})),
    ] {
        let mut wire = owned_prepare(SESSION);
        wire["data"][field] = value;
        let cancellation = CancellationToken::new();
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![wire]),
            cancel_during_reply: Some(cancellation.clone()),
        };
        let result = prepared.prepare(&tool, &runner, &cancellation, 42);
        assert_eq!(result.diagnostics[0]["code"], "invalid_result");
        assert!(result.data.is_none());
        assert_eq!(runner.calls.borrow().len(), 1);
    }
}

#[test]
fn stop_confirms_release_without_claiming_a_process_stop_only_on_a_lease_capable_runner() {
    let root = tempfile::tempdir().unwrap();
    let (prepared, tool) = fixture(root.path());
    for (supported, stopped, known, accepted) in [
        (true, Value::Null, true, true),
        (false, Value::Null, true, false),
        (true, json!(123), true, false),
        (true, Value::Null, false, false),
    ] {
        let runner = Provider {
            calls: RefCell::default(),
            results: RefCell::new(vec![envelope(
                "stop",
                json!({"ok":true,"session":SESSION,"action":"released","known_session":known,"stopped_pid":stopped}),
            )]),
            cancel_during_reply: None,
        };
        let result = prepared.stop(
            &tool,
            &runner,
            SESSION,
            false,
            supported,
            &CancellationToken::new(),
        );
        assert_eq!(result.ok, accepted);
        assert!(result.changed.is_empty());
        if accepted {
            assert_eq!(result.data.unwrap()["action"], "released");
        }
    }
}
