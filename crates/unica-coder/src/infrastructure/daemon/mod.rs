pub(crate) mod client;
pub(crate) mod identity;
pub(crate) mod protocol;
pub(crate) mod server;

#[cfg(test)]
mod tests {
    use super::client::{
        DaemonClient, DaemonClientConfig, ExistingDaemon, ManualDaemonClientClock,
    };
    use super::identity::{CoreIdentity, DaemonStateDirectory};
    use super::protocol::{
        parse_response, read_bounded_json_line, ClientRequest, DaemonErrorCode, EndpointRecord,
        ServerResponse, DAEMON_PROTOCOL_VERSION, MAX_JSON_LINE_BYTES,
    };
    use super::server::{
        install_handshake_pause, run_daemon, DaemonServerConfig, MAX_HANDSHAKES, MAX_OWNER_SESSIONS,
    };
    use crate::application::invocation_store::InvocationStoreError;
    use crate::infrastructure::platform::testing::{
        create_directory_link_fixture_for_test, set_unix_mode_for_test, unix_mode_for_test,
        FileLinkFixtureOutcome,
    };
    use crate::infrastructure::task_store::{FileInvocationStore, SystemEpochMillisClock};
    use std::io::{BufReader, Cursor, Write};
    use std::net::{Ipv4Addr, TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, Instant};

    fn alternate_identity() -> CoreIdentity {
        CoreIdentity::from_str("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
            .unwrap()
    }

    fn server_config(root: PathBuf, identity: CoreIdentity) -> DaemonServerConfig {
        DaemonServerConfig::new(physical_root(&root), identity, Duration::from_millis(350))
    }

    fn physical_root(root: &std::path::Path) -> PathBuf {
        std::fs::canonicalize(root).unwrap()
    }

    fn wait_for_record(
        root: &std::path::Path,
        identity: &CoreIdentity,
    ) -> (DaemonStateDirectory, EndpointRecord) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let directory = DaemonStateDirectory::open(&physical_root(root), identity).unwrap();
            if let Some(record) = directory.read_endpoint_record().unwrap() {
                return (directory, record);
            }
            assert!(
                Instant::now() < deadline,
                "daemon endpoint was not published"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn write_json_line<T: serde::Serialize>(stream: &mut TcpStream, value: &T) {
        let mut bytes = serde_json::to_vec(value).unwrap();
        bytes.push(b'\n');
        stream.write_all(&bytes).unwrap();
        stream.flush().unwrap();
    }

    fn connect_raw_owner(
        record: &EndpointRecord,
        identity: &CoreIdentity,
    ) -> (TcpStream, ServerResponse) {
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let hello = ClientRequest::hello_with_owner_for_test(
            record.token().to_string(),
            identity.clone(),
            uuid::Uuid::new_v4().to_string(),
        );
        write_json_line(&mut stream, &hello);
        let response = serde_json::from_slice(
            &read_bounded_json_line(&mut BufReader::new(stream.try_clone().unwrap())).unwrap(),
        )
        .unwrap();
        (stream, response)
    }

    #[test]
    fn core_identity_is_closed_compile_time_abi_protocol_digest() {
        let production = CoreIdentity::production();
        assert_eq!(production.as_str().len(), 64);
        assert!(production
            .as_str()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(production.as_str(), env!("CARGO_PKG_VERSION"));
        assert_ne!(production, alternate_identity());
        assert!(CoreIdentity::from_str(&production.as_str().to_uppercase()).is_err());
        assert!(CoreIdentity::from_str("v0.13").is_err());
    }

    #[test]
    fn world_readable_identity_directory_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let path = DaemonStateDirectory::path_for(&physical, &identity);
        std::fs::create_dir_all(&path).unwrap();
        if !set_unix_mode_for_test(&path, 0o755).unwrap() {
            return;
        }

        let error = DaemonStateDirectory::open(&physical, &identity).unwrap_err();
        assert!(error.contains("owner-only"), "{error}");
        assert_eq!(unix_mode_for_test(&path).unwrap(), Some(0o755));
    }

    #[test]
    fn symlinked_provider_state_root_is_rejected_before_creating_identity_state() {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("redirected-parent");
        std::fs::create_dir(&target).unwrap();
        let routed_parent = fixture.path().join("provider-parent-link");
        match create_directory_link_fixture_for_test(&target, &routed_parent).unwrap() {
            FileLinkFixtureOutcome::Created => {}
            FileLinkFixtureOutcome::Unsupported
            | FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,
        }
        let routed = routed_parent.join("provider-state-created-through-link");
        let identity = CoreIdentity::production();

        assert!(DaemonStateDirectory::open(&routed, &identity).is_err());
        assert!(
            !target.join("provider-state-created-through-link").exists(),
            "rejected ambient symlink must not receive even the provider-state directory"
        );
    }

    #[test]
    fn missing_provider_state_root_is_created_before_private_identity_child() {
        let fixture = tempfile::tempdir().unwrap();
        let physical_fixture = std::fs::canonicalize(fixture.path()).unwrap();
        let state_root = physical_fixture.join("cold").join("provider-state");
        let identity = CoreIdentity::production();

        let state = DaemonStateDirectory::open(&state_root, &identity).unwrap();

        assert!(state_root.is_dir());
        assert_eq!(
            state.path(),
            DaemonStateDirectory::path_for(&state_root, &identity)
        );
    }

    #[test]
    fn protocol_rejects_oversized_and_noncanonical_lines() {
        let oversized = vec![b'x'; MAX_JSON_LINE_BYTES + 1];
        let error =
            read_bounded_json_line(&mut BufReader::new(Cursor::new(oversized))).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        let unknown = format!(
            "{{\"kind\":\"ping\",\"unknown\":true,\"protocolVersion\":{DAEMON_PROTOCOL_VERSION}}}\n"
        );
        let error = serde_json::from_slice::<ClientRequest>(unknown.as_bytes()).unwrap_err();
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn wrong_token_and_protocol_version_are_rejected_without_echoing_token() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);

        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let bad_token = "ffffffff-ffff-4fff-8fff-ffffffffffff";
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            bad_token.to_string(),
            identity.clone(),
        );
        write_json_line(&mut stream, &request);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&stream)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::Unauthorized));
        assert!(!serde_json::to_string(&response)
            .unwrap()
            .contains(bad_token));

        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION + 1,
            record.token().to_string(),
            identity.clone(),
        );
        write_json_line(&mut stream, &request);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&stream)).unwrap())
                .unwrap();
        assert_eq!(
            response.error_code(),
            Some(DaemonErrorCode::ProtocolMismatch)
        );

        server.join().unwrap().unwrap();
    }

    #[test]
    fn fake_peer_error_code_is_closed_and_never_reaches_client_diagnostic() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            write_json_line(
                &mut stream,
                &serde_json::json!({
                    "kind": "error",
                    "code": "credential\n\u{001b}[31msecret-looking-value"
                }),
            );
        });

        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon response is not strict versioned JSON");
        assert!(!error.contains("credential"));
        assert!(!error.contains("secret-looking-value"));
        assert!(parse_response(br#"{"kind":"error","code":"future_code"}"#).is_err());
    }

    #[test]
    fn fake_peer_ready_at_deadline_cannot_restart_handshake_budget() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn late_malformed_peer_response_cannot_override_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(
                &mut stream,
                &serde_json::json!({"kind": "error", "code": "future_code"}),
            );
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn late_peer_disconnect_cannot_override_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let fake_peer = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _hello = read_bounded_json_line(&mut BufReader::new(&stream)).unwrap();
            peer_clock.advance(Duration::from_secs(5));
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);

        let error = client.connect_existing().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during handshake response");
    }

    #[test]
    fn ping_uses_one_aggregate_deadline_for_write_and_response() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let clock = ManualDaemonClientClock::new();
        let peer_clock = clock.clone();
        let peer_record = record.clone();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let ping = read_bounded_json_line(&mut reader).unwrap();
            assert_eq!(
                serde_json::from_slice::<ClientRequest>(&ping).unwrap(),
                ClientRequest::Ping {}
            );
            peer_clock.advance(Duration::from_secs(5));
            write_json_line(&mut stream, &ServerResponse::Pong);
        });
        let config = DaemonClientConfig::existing_only(physical, identity)
            .with_clock_for_test(clock)
            .with_connect_timeout_for_test(Duration::from_secs(5));
        let client = DaemonClient::new(config);
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        let error = owner.ping().unwrap_err();
        fake_peer.join().unwrap();

        assert_eq!(error, "daemon deadline expired during ping response");
    }

    #[test]
    fn owner_drop_closes_connection_without_waiting_for_release_ack() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let record = EndpointRecord::new(identity.clone(), listener.local_addr().unwrap().port());
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        directory.write_endpoint_record_for_test(&record).unwrap();
        let peer_record = record.clone();
        let (observed_tx, observed_rx) = mpsc::channel();
        let fake_peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let _hello = read_bounded_json_line(&mut reader).unwrap();
            write_json_line(&mut stream, &ServerResponse::ready(&peer_record));
            let observed_eof = read_bounded_json_line(&mut reader).unwrap_err().kind()
                == std::io::ErrorKind::UnexpectedEof;
            observed_tx.send(observed_eof).unwrap();
        });
        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("fake endpoint must connect"),
        };

        drop(owner);
        assert!(
            observed_rx.recv().unwrap(),
            "owner drop sent a release request"
        );
        fake_peer.join().unwrap();
    }

    #[test]
    fn exited_startup_child_is_reported_before_readiness_deadline() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = DaemonClientConfig::new(
            physical,
            identity,
            std::env::current_exe().unwrap(),
            Duration::from_millis(350),
        )
        .with_connect_timeout_for_test(Duration::from_millis(500));
        let client = DaemonClient::new(config);

        let error = match client.connect_or_spawn() {
            Ok(_) => panic!("exited fixture unexpectedly became a daemon owner"),
            Err(error) => error,
        };

        assert!(error.contains("exited before readiness with"), "{error}");
    }

    #[test]
    fn owner_lease_keeps_daemon_alive_then_idle_removes_only_its_record() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical,
            identity.clone(),
        ));
        let mut owner = match client.connect_existing().unwrap() {
            ExistingDaemon::Connected(owner) => owner,
            ExistingDaemon::Absent => panic!("published daemon must connect"),
        };
        owner.ping().unwrap();
        thread::sleep(Duration::from_millis(600));
        owner.ping().unwrap();

        let replacement = EndpointRecord::test_replacement(&record);
        directory
            .write_endpoint_record_for_test(&replacement)
            .unwrap();
        drop(owner);
        server.join().unwrap().unwrap();
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(replacement));
    }

    #[test]
    fn authenticated_owners_release_handshake_capacity() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut owners = Vec::new();

        for owner_index in 0..=MAX_HANDSHAKES {
            let (stream, response) = connect_raw_owner(&record, &identity);
            assert!(
                response.matches_record(&record),
                "owner {owner_index} was rejected after authentication: {response:?}"
            );
            owners.push(stream);
        }

        drop(owners);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn owner_session_capacity_is_distinct_and_retryable() {
        const EXPECTED_OWNER_SESSION_LIMIT: usize = 64;
        assert_eq!(MAX_OWNER_SESSIONS, EXPECTED_OWNER_SESSION_LIMIT);

        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut owners = Vec::new();

        for owner_index in 0..EXPECTED_OWNER_SESSION_LIMIT {
            let (stream, response) = connect_raw_owner(&record, &identity);
            assert!(
                response.matches_record(&record),
                "owner {owner_index} was rejected below the owner-session bound: {response:?}"
            );
            owners.push(stream);
        }

        let client = DaemonClient::new(DaemonClientConfig::existing_only(physical, identity));
        let error = client.connect_existing().unwrap_err();
        assert_eq!(error, "daemon owner capacity reached; retry later");

        drop(owners.pop());
        let retry_deadline = Instant::now() + Duration::from_secs(2);
        let mut recovered = loop {
            match client.connect_existing() {
                Ok(ExistingDaemon::Connected(owner)) => break owner,
                Ok(ExistingDaemon::Absent) => panic!("published daemon disappeared during retry"),
                Err(error) if error == "daemon owner capacity reached; retry later" => {
                    assert!(
                        Instant::now() < retry_deadline,
                        "owner capacity did not recover after a live session closed"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("owner-capacity retry failed unexpectedly: {error}"),
            }
        };
        recovered.ping().unwrap();
        drop(recovered);

        drop(owners);
        server.join().unwrap().unwrap();
    }

    #[test]
    fn stale_record_is_never_signalled_and_foreign_identity_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let identity = CoreIdentity::production();
        let directory = DaemonStateDirectory::open(&physical, &identity).unwrap();
        let stale = EndpointRecord::test_stale(identity.clone(), 4_294_967_000);
        directory.write_endpoint_record_for_test(&stale).unwrap();

        let client = DaemonClient::new(DaemonClientConfig::existing_only(
            physical.clone(),
            identity.clone(),
        ));
        assert!(matches!(
            client.connect_existing().unwrap(),
            ExistingDaemon::Absent
        ));
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(stale));

        let foreign = EndpointRecord::test_stale(alternate_identity(), 42);
        directory.write_endpoint_record_for_test(&foreign).unwrap();
        let error = client.connect_existing().unwrap_err();
        assert!(error.contains("foreign core identity"), "{error}");
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(foreign));
    }

    #[test]
    fn incompatible_core_identities_use_separate_endpoint_directories() {
        let root = tempfile::tempdir().unwrap();
        let physical = physical_root(root.path());
        let production = CoreIdentity::production();
        let alternate = alternate_identity();
        assert_ne!(
            DaemonStateDirectory::path_for(&physical, &production),
            DaemonStateDirectory::path_for(&physical, &alternate)
        );
        let first = DaemonStateDirectory::open(&physical, &production).unwrap();
        let second = DaemonStateDirectory::open(&physical, &alternate).unwrap();
        assert_ne!(first.path(), second.path());
    }

    #[test]
    fn admitted_handshake_blocks_idle_exit_until_lease_is_registered() {
        let pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = DaemonServerConfig::new(
            physical_root(root.path()),
            identity.clone(),
            Duration::from_millis(80),
        )
        .with_handshake_pause(&pause);
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let mut stream = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        let request = ClientRequest::hello(
            DAEMON_PROTOCOL_VERSION,
            record.token().to_string(),
            identity,
        );
        write_json_line(&mut stream, &request);
        pause.wait_until_entered();

        thread::sleep(Duration::from_millis(160));
        assert!(
            !server.is_finished(),
            "admitted handshake lost the daemon to idle exit"
        );
        assert_eq!(
            directory.read_endpoint_record().unwrap(),
            Some(record.clone())
        );

        pause.release();
        let mut reader = BufReader::new(&stream);
        let ready: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut reader).unwrap()).unwrap();
        assert!(ready.matches_record(&record));
        drop(reader);
        write_json_line(&mut stream, &ClientRequest::Release {});
        server.join().unwrap().unwrap();
    }

    #[test]
    fn unauthenticated_connection_admission_is_bounded() {
        let pause = install_handshake_pause();
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = DaemonServerConfig::new(
            physical_root(root.path()),
            identity.clone(),
            Duration::from_millis(120),
        )
        .with_handshake_pause(&pause);
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let mut admitted = Vec::new();
        for _ in 0..MAX_HANDSHAKES {
            admitted.push(TcpStream::connect(record.loopback_addr().unwrap()).unwrap());
            thread::sleep(Duration::from_millis(30));
        }
        pause.wait_until_entered();
        let extra = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        extra
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&extra)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::Overloaded));

        drop(extra);
        drop(admitted);
        pause.release();
        server.join().unwrap().unwrap();
    }

    #[test]
    fn duplicate_owner_lease_and_oversized_wire_request_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (_directory, record) = wait_for_record(root.path(), &identity);
        let lease = "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string();
        let hello = ClientRequest::hello_with_owner_for_test(
            record.token().to_string(),
            identity.clone(),
            lease.clone(),
        );
        let mut first = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        write_json_line(&mut first, &hello);
        let ready: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&first)).unwrap())
                .unwrap();
        assert!(ready.matches_record(&record));

        let duplicate =
            ClientRequest::hello_with_owner_for_test(record.token().to_string(), identity, lease);
        let mut second = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        write_json_line(&mut second, &duplicate);
        let response: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&second)).unwrap())
                .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::DuplicateLease));

        write_json_line(&mut first, &ClientRequest::Release {});
        let released: ServerResponse =
            serde_json::from_slice(&read_bounded_json_line(&mut BufReader::new(&first)).unwrap())
                .unwrap();
        assert_eq!(released, ServerResponse::Released);
        let release_deadline = Instant::now() + Duration::from_secs(2);
        let mut reused = loop {
            let mut candidate = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
            write_json_line(&mut candidate, &hello);
            let response: ServerResponse = serde_json::from_slice(
                &read_bounded_json_line(&mut BufReader::new(&candidate)).unwrap(),
            )
            .unwrap();
            if response.matches_record(&record) {
                break candidate;
            }
            assert_eq!(response.error_code(), Some(DaemonErrorCode::DuplicateLease));
            assert!(
                Instant::now() < release_deadline,
                "released owner lease remained registered"
            );
            thread::sleep(Duration::from_millis(5));
        };

        let mut oversized = TcpStream::connect(record.loopback_addr().unwrap()).unwrap();
        oversized
            .write_all(&vec![b'x'; MAX_JSON_LINE_BYTES + 1])
            .unwrap();
        oversized.write_all(b"\n").unwrap();
        let response: ServerResponse = serde_json::from_slice(
            &read_bounded_json_line(&mut BufReader::new(&oversized)).unwrap(),
        )
        .unwrap();
        assert_eq!(response.error_code(), Some(DaemonErrorCode::InvalidRequest));

        write_json_line(&mut reused, &ClientRequest::Release {});
        server.join().unwrap().unwrap();
    }

    #[test]
    fn daemon_is_the_sole_invocation_store_writer_for_its_lifetime() {
        let root = tempfile::tempdir().unwrap();
        let identity = CoreIdentity::production();
        let config = server_config(root.path().to_path_buf(), identity.clone());
        let server = thread::spawn(move || run_daemon(config));
        let (directory, record) = wait_for_record(root.path(), &identity);
        let task_root = directory.path().join("tasks");

        assert!(matches!(
            FileInvocationStore::open(&task_root, Arc::new(SystemEpochMillisClock)),
            Err(InvocationStoreError::AlreadyOwned)
        ));
        let competing = run_daemon(server_config(root.path().to_path_buf(), identity));
        assert!(competing
            .unwrap_err()
            .contains("task store already has an active owner"));
        assert_eq!(directory.read_endpoint_record().unwrap(), Some(record));
        server.join().unwrap().unwrap();
        let reopened = FileInvocationStore::open(&task_root, Arc::new(SystemEpochMillisClock));
        assert!(reopened.is_ok());
    }
}
