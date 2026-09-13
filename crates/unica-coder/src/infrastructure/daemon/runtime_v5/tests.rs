//! Тесты рантайма v5. Вынесены из `runtime_v5.rs`: производственный файл
//! перерос восемь тысяч строк, а модуль тестов — половина его объёма.
//! Путь модуля прежний (`infrastructure::daemon::runtime_v5::tests`),
//! поэтому выражения отбора nextest не меняются.

use super::*;
use crate::application::invocation::normalized_arguments_hash;
use crate::application::operation_descriptors::{ExecutionClass, KnownLongReason};
use crate::application::receipt_ledger::{
    receipt_key_digest, request_scope_hash, CancelExpiryOutcome, CancelResolution,
    CommittedDirectPublication, OriginalCutoffDescriptor, ReceiptKey, ReceiptLedgerPort,
    ReceiptRecordHeader, ReceiptState, ReceiptTaskProjection, ReceiptTerminalOutcome,
    ReceiptVersion, RequestIdentity, ReserveOutcome, ReservedPhase, ReservedReceipt,
    V5CanonicalTerminal, V5ToolIdentity, CANCEL_RESERVATION_TTL_MS, MAX_RECEIPT_ENTITLEMENT_BYTES,
};
use crate::domain::invocation::{DomainResult, InvocationFailure};
use crate::domain::invocation::{InvocationId, SafeIdentityHash, TaskId};
use crate::infrastructure::daemon::client_v5::V5DaemonProcessOwner;
use crate::infrastructure::daemon::identity::{CoreIdentity, DaemonStateDirectory};
use crate::infrastructure::daemon::protocol_v5::V5InvocationRequest;
use crate::infrastructure::daemon::protocol_v5::{
    decode_v5_server_response, read_bounded_v5_probe_response_frame, V5EndpointRecord,
    V5ProbeResponseKind, V5ProbeServerResponse,
};
use crate::infrastructure::platform::testing::{
    attempt_retained_directory_replacement_for_test, RetainedDirectoryReplacementOutcome,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

struct CooperativeKnownLongService {
    entered: mpsc::Sender<()>,
}

impl CanonicalInvocationService for CooperativeKnownLongService {
    fn prepare(
        &self,
        _invocation: &super::super::server::ActorBoundInvocation,
    ) -> Result<ExecutionClass, Box<DomainResult>> {
        Ok(ExecutionClass::KnownLong(KnownLongReason::ExternalProcess))
    }

    fn execute(
        &self,
        _invocation: &super::super::server::ActorBoundExecution,
        cancellation: CancellationToken,
    ) -> Result<DomainResult, InvocationFailure> {
        self.entered.send(()).expect("report task execution");
        while !cancellation.is_cancelled() {
            thread::yield_now();
        }
        Err(InvocationFailure::new(
            "cancelled",
            "cooperative task observed cancellation",
        ))
    }
}

#[test]
fn cancel_task_signals_the_running_v5_canonical_execution() {
    let root = tempfile::tempdir().expect("temporary cancellation state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let workspace = tempfile::tempdir().expect("temporary cancellation workspace");
    let source = workspace.path().join("src");
    std::fs::create_dir_all(&source).expect("create source root");
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .expect("write workspace descriptor");
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("write configuration root");
    let workspace = std::fs::canonicalize(workspace.path()).expect("physical workspace");
    let identity = CoreIdentity::production_v5();
    let (entered_tx, entered_rx) = mpsc::channel();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    )
    .with_invocation_service(Arc::new(CooperativeKnownLongService {
        entered: entered_tx,
    }));
    let server = thread::spawn(move || run_daemon(config));
    let _record = wait_for_v5_record(&state_root, &identity);
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        V5ToolIdentity::View,
        serde_json::Map::from_iter([(
            "at".to_owned(),
            serde_json::Value::String("main:Catalog.Items".to_owned()),
        )]),
        workspace.to_string_lossy().into_owned(),
        7_000,
    )
    .expect("valid known-long invocation");
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect v5 owner");
    let submitted = owner
        .submit_invocation(invocation)
        .expect("submit known-long invocation");
    let task_id = match submitted {
        V5ServerResponse::Invocation {
            outcome:
                V5InvocationResponse::Task {
                    snapshot:
                        super::super::protocol_v5::V5DaemonTaskSnapshot::Working { task_id, .. },
                },
        } => task_id,
        other => panic!("known-long submission did not return Working: {other:?}"),
    };
    if entered_rx.recv_timeout(Duration::from_secs(10)).is_err() {
        let snapshot = owner.get_task(task_id).expect("inspect stalled Task");
        panic!("canonical execution did not enter: {snapshot:?}");
    }

    owner.cancel_task(task_id).expect("cancel running Task");
    let terminal = owner
        .wait_task(task_id, 7_000)
        .expect("wait for cancellation");
    assert!(matches!(
        terminal,
        V5ServerResponse::Task {
            snapshot: super::super::protocol_v5::V5DaemonTaskSnapshot::Cancelled { .. }
        }
    ));

    drop(owner);
    server
        .join()
        .expect("v5 cancellation daemon did not panic")
        .expect("v5 cancellation daemon exited cleanly");
}

#[test]
fn promised_receipt_projects_the_exact_stable_queued_task() {
    let task = ReceiptTaskProjection::new(
        "11111111-1111-4111-8111-111111111111"
            .parse()
            .expect("valid TaskId"),
        "22222222-2222-4222-8222-222222222222"
            .parse()
            .expect("valid InvocationId"),
        1_000,
        1_000,
        3_600_000,
        250,
        1,
    )
    .expect("valid Task projection");
    let digest: crate::application::receipt_ledger::ReceiptKeyDigest =
        "33".repeat(32).parse().expect("valid receipt digest");

    let snapshot = queued_receipt_task_snapshot(&task, digest.clone(), true);

    assert_eq!(
        snapshot,
        super::super::protocol_v5::V5DaemonTaskSnapshot::Queued {
            task_id: task.task_id(),
            invocation_id: task.invocation_id(),
            receipt_key_digest: digest,
            created_at_epoch_ms: 1_000,
            updated_at_epoch_ms: 1_000,
            ttl_ms: 3_600_000,
            poll_interval_ms: 250,
            version: 1,
            cancel_requested: true,
        }
    );
}

fn write_json_line(stream: &mut TcpStream, value: &serde_json::Value) {
    let mut bytes = serde_json::to_vec(value).expect("serialize v5 frame");
    bytes.push(b'\n');
    stream.write_all(&bytes).expect("write v5 frame");
}

enum CancelPortFailure {
    ImmediateCommitUncertain,
    ImmediateStoreUnavailable,
    WaitPastOperationDeadline {
        observed_deadline: mpsc::Sender<Instant>,
    },
}

struct FailingCancelPort {
    failure: CancelPortFailure,
}

impl ReceiptLedgerPort for FailingCancelPort {
    fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        Ok(0)
    }

    fn reserve(
        &mut self,
        _key: ReceiptKey,
        _original_cutoff: OriginalCutoffDescriptor,
        _deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn request_cancel_or_reserve(
        &mut self,
        key: ReceiptKey,
        _cancel_reserved_at_epoch_ms: u64,
        deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        match self.failure {
            CancelPortFailure::ImmediateCommitUncertain => {
                Err(ReceiptLedgerError::CommitUncertain {
                    receipt_key_digest: receipt_key_digest(&key),
                })
            }
            CancelPortFailure::ImmediateStoreUnavailable => {
                Err(ReceiptLedgerError::StoreUnavailable)
            }
            CancelPortFailure::WaitPastOperationDeadline {
                ref observed_deadline,
            } => {
                observed_deadline
                    .send(deadline)
                    .expect("publish live cancel operation deadline");
                thread::sleep(
                    deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(10),
                );
                Err(ReceiptLedgerError::StoreUnavailable)
            }
        }
    }

    fn expire_cancel_reserved(
        &mut self,
        _key: ReceiptKey,
        _expected_version: ReceiptVersion,
        _expected_mutation_sequence: u64,
        _observed_at_epoch_ms: u64,
        _deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn publish_direct_terminal(
        &mut self,
        _key: &ReceiptKey,
        _expected_version: ReceiptVersion,
        _terminal_epoch_ms: u64,
        _terminal: V5CanonicalTerminal,
        _deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn recover(
        &mut self,
        _key: &ReceiptKey,
        _deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }
}

struct SlowReservePort {
    delay: Duration,
}

impl ReceiptLedgerPort for SlowReservePort {
    fn generation(&mut self, _deadline: Instant) -> Result<u64, ReceiptLedgerError> {
        Ok(0)
    }

    fn reserve(
        &mut self,
        key: ReceiptKey,
        original_cutoff: OriginalCutoffDescriptor,
        _deadline: Instant,
    ) -> Result<ReserveOutcome, ReceiptLedgerError> {
        thread::sleep(self.delay);
        Ok(ReserveOutcome::Created(ReservedReceipt::new(
            ReceiptRecordHeader::new(
                key.clone(),
                receipt_key_digest(&key),
                ReceiptVersion::initial(),
                1,
                512,
            ),
            original_cutoff.accepted_epoch_ms(),
            original_cutoff,
            ReservedPhase::Unbound,
            false,
            MAX_RECEIPT_ENTITLEMENT_BYTES - 512,
        )))
    }

    fn request_cancel_or_reserve(
        &mut self,
        _key: ReceiptKey,
        _cancel_reserved_at_epoch_ms: u64,
        _deadline: Instant,
    ) -> Result<CancelResolution, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn expire_cancel_reserved(
        &mut self,
        _key: ReceiptKey,
        _expected_version: ReceiptVersion,
        _expected_mutation_sequence: u64,
        _observed_at_epoch_ms: u64,
        _deadline: Instant,
    ) -> Result<CancelExpiryOutcome, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn publish_direct_terminal(
        &mut self,
        _key: &ReceiptKey,
        _expected_version: ReceiptVersion,
        _terminal_epoch_ms: u64,
        _terminal: V5CanonicalTerminal,
        _deadline: Instant,
    ) -> Result<CommittedDirectPublication, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }

    fn recover(
        &mut self,
        _key: &ReceiptKey,
        _deadline: Instant,
    ) -> Result<ReceiptState, ReceiptLedgerError> {
        Err(ReceiptLedgerError::StoreUnavailable)
    }
}

#[test]
fn seven_second_submit_budget_is_not_truncated_by_transport_timeouts() {
    let root = tempfile::tempdir().expect("temporary long-submit state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical long-submit state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || {
        run_daemon_configured(config, |mut runtime| {
            runtime.receipt_ledger = ReceiptLedgerActor::spawn(SlowReservePort {
                delay: Duration::from_millis(5_100),
            });
            runtime
        })
    });
    let _record = wait_for_v5_record(&state_root, &identity);
    let invocation = V5InvocationRequest::new(
        InvocationId::new(),
        TaskId::new(),
        V5ToolIdentity::View,
        serde_json::Map::new(),
        "workspace-a".to_owned(),
        7_000,
    )
    .expect("valid long-submit request");
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect long-submit owner");
    let response = owner.submit_invocation(invocation);
    drop(owner);
    let server_result = server.join().expect("join long-submit runtime");

    assert!(
        matches!(
            response,
            Ok(V5ServerResponse::Invocation { .. })
                | Ok(V5ServerResponse::Error {
                    code: V5DaemonErrorCode::StoreFailed
                })
        ),
        "seven-second reserve must reach the next runtime transition: {response:?}"
    );
    assert_eq!(server_result, Ok(()));
}

#[test]
fn commit_uncertain_is_returned_before_process_owned_fail_stop_retains_endpoint() {
    let root = tempfile::tempdir().expect("temporary fail-stop state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical fail-stop state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(30),
    );
    let server = thread::spawn(move || {
        run_daemon_configured(config, |mut runtime| {
            runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                failure: CancelPortFailure::ImmediateCommitUncertain,
            });
            runtime
        })
    });
    let record = wait_for_v5_record(&state_root, &identity);
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect fail-stop owner");
    let response = owner.cancel_invocation(key);
    drop(owner);
    let server_result = server.join().expect("join fail-stop runtime");
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("reopen fail-stop daemon state");
    let retained = state
        .read_v5_endpoint_record()
        .expect("read retained fail-stop endpoint");
    let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

    assert_eq!(
        response,
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::StoreCommitUncertain,
        })
    );
    assert_eq!(server_result, Ok(()));
    assert_eq!(retained, Some(record));
    assert!(
        competing_authority.is_err(),
        "fail-stop released receipt authority before process death"
    );
}

#[test]
fn running_mutation_timeout_preserves_response_margin_or_closes_after_it() {
    let root = tempfile::tempdir().expect("temporary timeout fail-stop state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical timeout fail-stop state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(30),
    );
    let (operation_deadline_tx, operation_deadline_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        run_daemon_configured(config, |mut runtime| {
            runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                failure: CancelPortFailure::WaitPastOperationDeadline {
                    observed_deadline: operation_deadline_tx,
                },
            });
            runtime
        })
    });
    let record = wait_for_v5_record(&state_root, &identity);
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let decoded = decode_v5_request_frame(
        serde_json::to_vec(&V5ClientRequest::CancelInvocation {
            receipt_key: key.clone(),
        })
        .expect("serialize timeout deadline fixture"),
    )
    .expect("decode timeout deadline fixture");
    let request_received_at = Instant::now();
    let deadlines = v5_request_deadlines(&decoded, request_received_at)
        .expect("derive timeout response deadlines");
    assert_eq!(
        deadlines.operation.duration_since(request_received_at),
        SESSION_READ_TIMEOUT,
        "cancel operation keeps its original bounded session budget"
    );
    assert_eq!(
        deadlines.response.duration_since(deadlines.operation),
        RESPONSE_SERIALIZATION_MARGIN,
        "response serialization gets exactly one non-renewable margin"
    );
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect timeout fail-stop owner");
    let response = owner.cancel_invocation(key);
    let response_completed_at = Instant::now();
    let operation_deadline = operation_deadline_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("observe the live cancel operation deadline");
    drop(owner);
    let server_result = server.join().expect("join timeout fail-stop runtime");
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("reopen timeout fail-stop daemon state");
    let retained = state
        .read_v5_endpoint_record()
        .expect("read retained timeout fail-stop endpoint");
    let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

    match response {
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::StoreCommitUncertain,
        }) => {}
        Err(error) => {
            assert_eq!(
                error, "read protocol-v5 cancel invocation: v5 JSON line ended before data",
                "only expiry of the bounded response margin may replace the closed error"
            );
            let final_response_deadline = operation_deadline
                .checked_add(RESPONSE_SERIALIZATION_MARGIN)
                .expect("bounded response deadline");
            assert!(
                response_completed_at >= final_response_deadline,
                "transport closed before the live operation deadline and response margin expired"
            );
        }
        unexpected => panic!("unexpected timed-out mutation response: {unexpected:?}"),
    }
    assert_eq!(server_result, Ok(()));
    assert_eq!(retained, Some(record));
    assert!(
        competing_authority.is_err(),
        "timed-out mutation released receipt authority before process death"
    );
}

#[test]
fn fail_stop_transport_margin_starts_after_response_serialization() {
    struct DelayedResponse(V5ServerResponse);

    impl Serialize for DelayedResponse {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            thread::sleep(RESPONSE_SERIALIZATION_MARGIN + Duration::from_millis(10));
            self.0.serialize(serializer)
        }
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind response listener");
    let address = listener.local_addr().expect("response listener address");
    let reader = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept response stream");
        let mut line = String::new();
        BufReader::new(stream)
            .read_line(&mut line)
            .expect("read fail-stop response");
        line
    });
    let mut stream = TcpStream::connect(address).expect("connect response stream");
    let observed_at = Instant::now();
    let expired_response_deadline = observed_at
        .checked_sub(Duration::from_millis(1))
        .expect("response deadline can precede the observation");
    let response = DelayedResponse(V5ServerResponse::Error {
        code: V5DaemonErrorCode::DurabilityUncertain,
    });

    assert_eq!(
        fail_stop_response_write_timeout(expired_response_deadline, observed_at),
        RESPONSE_SERIALIZATION_MARGIN
    );
    write_fail_stop_json_line(&mut stream, &response, expired_response_deadline)
        .expect("serialized fail-stop response retains a fresh transport margin");
    assert_eq!(
        reader.join().expect("join response reader"),
        "{\"kind\":\"error\",\"code\":\"durability_uncertain\"}\n"
    );
}

#[test]
fn every_fail_stop_store_error_is_written_without_reentering_the_actor() {
    let root = tempfile::tempdir().expect("temporary store-failure state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical store-failure state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_secs(30),
    );
    let server = thread::spawn(move || {
        run_daemon_configured(config, |mut runtime| {
            runtime.receipt_ledger = ReceiptLedgerActor::spawn(FailingCancelPort {
                failure: CancelPortFailure::ImmediateStoreUnavailable,
            });
            runtime
        })
    });
    let record = wait_for_v5_record(&state_root, &identity);
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect store-failure owner");
    let response = owner.cancel_invocation(key);
    drop(owner);
    let server_result = server.join().expect("join store-failure runtime");
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("reopen store-failure daemon state");
    let retained = state
        .read_v5_endpoint_record()
        .expect("read retained store-failure endpoint");
    let competing_authority = state.acquire_receipt_authority(Duration::from_millis(30));

    assert_eq!(
        response,
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::StoreFailed,
        })
    );
    assert_eq!(server_result, Ok(()));
    assert_eq!(retained, Some(record));
    assert!(
        competing_authority.is_err(),
        "store failure released receipt authority before process death"
    );
}

#[test]
fn healthy_runtime_drops_the_actor_store_before_releasing_named_authority() {
    // Порядок полей проверяется в производственном файле — тесты живут рядом с ним.
    let source = include_str!("../runtime_v5.rs");
    let start = source
        .find("struct V5ReceiptRuntime {")
        .expect("runtime owner declaration");
    let body = source[start..]
        .split_once("\n}")
        .expect("runtime owner declaration end")
        .0;

    assert!(
        body.find("receipt_ledger: ReceiptLedgerActor")
            < body.find("_stable_authority: ReceiptAuthorityLock"),
        "healthy Rust drop order must join actor/store before authority release"
    );
}

#[test]
fn authenticated_release_closes_only_its_owner_session() {
    let root = tempfile::tempdir().expect("temporary release state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(500),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let mut stream = TcpStream::connect(record.loopback_addr().expect("loopback address"))
        .expect("connect release owner");
    let mut reader = BufReader::new(stream.try_clone().expect("clone release stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "77777777-7777-4777-8777-777777777777"
        }),
    );
    read_bounded_v5_probe_response_frame(&mut reader).expect("read release ready");

    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read release response");
    let released = decode_v5_server_response(&released).expect("decode release response");
    drop(stream);

    let mut successor = TcpStream::connect(record.loopback_addr().expect("loopback address"))
        .expect("release must leave the daemon listener available");
    successor
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound successor session read");
    let mut successor_reader =
        BufReader::new(successor.try_clone().expect("clone successor stream"));
    write_json_line(
        &mut successor,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "88888888-8888-4888-8888-888888888888"
        }),
    );
    let successor_ready = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("released owner must not close successor admission");
    let successor_ready: V5HandshakeServerResponse =
        serde_json::from_slice(&successor_ready).expect("decode successor ready response");
    assert!(successor_ready.matches_record(&record));
    write_json_line(&mut successor, &json!({"kind": "ping"}));
    let successor_pong = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("successor session ping");
    let successor_pong: V5ProbeServerResponse =
        serde_json::from_slice(&successor_pong).expect("decode successor pong");
    assert_eq!(successor_pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor session");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
    drop(successor);

    server
        .join()
        .expect("join released v5 runtime")
        .expect("released v5 runtime");

    assert_eq!(released, V5ServerResponse::Released);
}

struct ManualEpochClock {
    epoch_ms: AtomicU64,
}

impl ManualEpochClock {
    fn new(epoch_ms: u64) -> Self {
        Self {
            epoch_ms: AtomicU64::new(epoch_ms),
        }
    }

    fn set(&self, epoch_ms: u64) {
        self.epoch_ms.store(epoch_ms, Ordering::SeqCst);
    }
}

impl EpochMillisClock for ManualEpochClock {
    fn now_epoch_millis(&self) -> u64 {
        self.epoch_ms.load(Ordering::SeqCst)
    }
}

fn exchange_once_with_epoch(
    state_root: &std::path::Path,
    identity: &CoreIdentity,
    clock: Arc<ManualEpochClock>,
    exchange: impl FnOnce(&mut V5DaemonProcessOwner) -> Result<V5ServerResponse, String>,
) -> V5ServerResponse {
    let config = DaemonServerConfig::new(
        state_root.to_path_buf(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || {
        run_daemon_configured(config, move |mut runtime| {
            runtime.epoch_clock = clock;
            runtime
        })
    });
    let _record = wait_for_v5_record(state_root, identity);
    let mut owner = V5DaemonProcessOwner::connect_or_spawn(
        state_root,
        identity.clone(),
        std::path::PathBuf::from("unused-existing-v5-endpoint"),
        Duration::from_millis(300),
    )
    .expect("connect authenticated one-shot owner");
    let response = exchange(&mut owner).expect("exchange one protocol-v5 request");
    drop(owner);
    server
        .join()
        .expect("join one-shot v5 runtime")
        .expect("one-shot v5 runtime");
    response
}

#[test]
fn receipt_digest_collision_is_a_fail_stop_store_error_not_caller_identity_mismatch() {
    let error = ReceiptLedgerError::ReceiptDigestCollision;

    assert!(error.requires_reopen());
    assert_eq!(daemon_error_code(&error), V5DaemonErrorCode::StoreFailed);
}

#[test]
fn cancel_existing_reserved_receipt_returns_the_typed_pending_winner() {
    let root = tempfile::tempdir().expect("temporary existing-winner state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open existing-winner daemon state");
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity.clone(), Duration::from_millis(50)),
    )
    .expect("open protocol-v5 runtime");
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let cutoff = OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff");
    runtime
        .receipt_ledger
        .reserve(key.clone(), cutoff, Instant::now() + Duration::from_secs(2))
        .expect("reserve exact receipt");

    let reply = runtime
        .cancel_invocation(key.clone(), 2_000, Instant::now() + Duration::from_secs(2))
        .expect("return the existing reserved winner");

    assert!(matches!(
        reply,
        V5RuntimeReply::Json(V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending {
                receipt_key,
                phase: V5InvocationPhase::ReservedUnbound,
                accepted_epoch_ms: 1_000,
                original_budget_ms: 7_000,
                cancel_requested: true,
            },
        }) if receipt_key == key
    ));
}

#[test]
fn startup_terminalizes_pre_task_receipts_without_replaying_domain_work() {
    for (phase, cancel_requested, expected) in [
        (
            ReservedPhase::Unbound,
            false,
            ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::Interrupted,
            },
        ),
        (
            ReservedPhase::ActorBound {
                bound_workspace_identity: SafeIdentityHash::from_sha256(
                    Sha256::digest(b"startup-actor").into(),
                ),
            },
            true,
            ReceiptTerminalOutcome::Cancelled,
        ),
        (
            ReservedPhase::Begun {
                bound_workspace_identity: SafeIdentityHash::from_sha256(
                    Sha256::digest(b"startup-begun").into(),
                ),
            },
            true,
            ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::OutcomeUncertain,
            },
        ),
    ] {
        let root = tempfile::tempdir().expect("temporary startup recovery root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open startup recovery daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve startup receipt")
            .into_reservation()
            .expect("new startup receipt");
        let mut current_version = reserved.record_version();
        match phase {
            ReservedPhase::Unbound => {}
            ReservedPhase::ActorBound {
                bound_workspace_identity,
            } => {
                current_version = runtime
                    .receipt_ledger
                    .bind_reserved_actor(
                        key.clone(),
                        current_version,
                        bound_workspace_identity,
                        deadline,
                    )
                    .expect("bind startup actor")
                    .record_version();
            }
            ReservedPhase::Begun {
                bound_workspace_identity,
            } => {
                let bound = runtime
                    .receipt_ledger
                    .bind_reserved_actor(
                        key.clone(),
                        current_version,
                        bound_workspace_identity,
                        deadline,
                    )
                    .expect("bind begun startup actor");
                runtime
                    .receipt_ledger
                    .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                    .expect("mark startup receipt begun");
            }
        }
        if cancel_requested {
            runtime
                .receipt_ledger
                .request_cancel_or_reserve(key.clone(), 2_000, deadline)
                .expect("persist startup cancellation");
        }
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
        let recovered = reopened
            .receipt_ledger
            .recover(key, Instant::now() + Duration::from_secs(2))
            .expect("read reconciled startup receipt");
        let ReceiptState::DirectTerminalUnacked(receipt) = recovered else {
            panic!("startup must publish one direct terminal")
        };
        assert_eq!(receipt.terminal().outcome(), &expected);
    }
}

#[test]
fn startup_terminalizes_unbound_promised_task_without_task_store_create() {
    for (cancel_requested, expected) in [
        (
            false,
            ReceiptTerminalOutcome::Failed {
                reason: V5SafeFailureReason::Interrupted,
            },
        ),
        (true, ReceiptTerminalOutcome::Cancelled),
    ] {
        let root = tempfile::tempdir().expect("temporary promised recovery root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open promised recovery daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve promised startup receipt")
            .into_reservation()
            .expect("new promised startup receipt");
        let promised = runtime
            .receipt_ledger
            .promise_task_unbound(
                key.clone(),
                reserved.record_version(),
                1_007,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("promise startup Task");
        if cancel_requested {
            runtime
                .receipt_ledger
                .request_task_cancel(
                    key.clone(),
                    TaskCancellationReceipt::PromisedUnbound(promised),
                    deadline,
                )
                .expect("persist promised Task cancellation");
        }
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
        let recovered = reopened
            .receipt_ledger
            .recover(key, Instant::now() + Duration::from_secs(2))
            .expect("read reconciled promised Task receipt");
        let ReceiptState::TaskTerminalReceiptBacked(receipt) = recovered else {
            panic!("startup must publish one receipt-backed Task terminal")
        };
        assert_eq!(receipt.terminal().outcome(), &expected);
        assert_eq!(reopened.task_projection.recovery.entries().len(), 0);
    }
}

#[test]
fn startup_materializes_and_terminalizes_actor_bound_promised_task() {
    for cancel_requested in [false, true] {
        let root = tempfile::tempdir().expect("temporary actor-bound recovery root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open actor-bound recovery daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve actor-bound startup receipt")
            .into_reservation()
            .expect("new actor-bound startup receipt");
        let promised = runtime
            .receipt_ledger
            .promise_task_unbound(
                key.clone(),
                reserved.record_version(),
                1_007,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("promise startup Task");
        let actor_bound = runtime
            .receipt_ledger
            .bind_promised_task_actor(
                key.clone(),
                promised.record_version(),
                SafeIdentityHash::from_sha256(Sha256::digest(b"startup-actor").into()),
                deadline,
            )
            .expect("bind promised startup Task actor");
        if cancel_requested {
            runtime
                .receipt_ledger
                .request_task_cancel(
                    key.clone(),
                    TaskCancellationReceipt::PromisedActorBound(actor_bound),
                    deadline,
                )
                .expect("persist actor-bound startup cancellation");
        }
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
        assert_eq!(
            reopened
                .receipt_ledger
                .recover(key.clone(), Instant::now() + Duration::from_secs(2)),
            Err(ReceiptLedgerError::ReceiptNotFound)
        );
        let snapshot = reopened
            .resolve_task(
                key.reserved_task_id(),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("resolve recovered actor-bound Task");
        match (cancel_requested, snapshot) {
            (
                false,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason: V5SafeFailureReason::Interrupted,
                    cancel_requested: false,
                    ..
                },
            )
            | (
                true,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                    cancel_requested: true,
                    ..
                },
            ) => {}
            (_, other) => panic!("unexpected recovered actor-bound Task: {other:?}"),
        }
    }
}

#[test]
fn startup_materializes_handoff_without_replaying_begun_work() {
    for (phase, cancel_requested) in [
        (AttemptPhase::NotBegun, false),
        (AttemptPhase::NotBegun, true),
        (AttemptPhase::Begun, false),
        (AttemptPhase::Begun, true),
    ] {
        let root = tempfile::tempdir().expect("temporary handoff recovery root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open handoff recovery daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let key = ReceiptKey::new(
            InvocationId::new(),
            TaskId::new(),
            RequestIdentity::new(
                identity.digest().clone(),
                V5ToolIdentity::View,
                normalized_arguments_hash(&serde_json::Map::new()),
                request_scope_hash("workspace-a").expect("request scope"),
            ),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let reserved = runtime
            .receipt_ledger
            .reserve(
                key.clone(),
                OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
                deadline,
            )
            .expect("reserve handoff startup receipt")
            .into_reservation()
            .expect("new handoff startup receipt");
        let bound = runtime
            .receipt_ledger
            .bind_reserved_actor(
                key.clone(),
                reserved.record_version(),
                SafeIdentityHash::from_sha256(Sha256::digest(b"startup-handoff").into()),
                deadline,
            )
            .expect("bind handoff startup actor");
        let version = match phase {
            AttemptPhase::NotBegun => bound.record_version(),
            AttemptPhase::Begun => runtime
                .receipt_ledger
                .mark_reserved_begun(key.clone(), bound.record_version(), deadline)
                .expect("mark startup handoff begun")
                .record_version(),
        };
        let handoff = runtime
            .receipt_ledger
            .begin_bound_task_handoff(
                key.clone(),
                version,
                1_009,
                3_600_000,
                V5_TASK_POLL_INTERVAL_MS,
                deadline,
            )
            .expect("persist startup Task handoff");
        if cancel_requested {
            runtime
                .receipt_ledger
                .request_task_cancel(
                    key.clone(),
                    TaskCancellationReceipt::HandoffActorBound(handoff),
                    deadline,
                )
                .expect("persist startup handoff cancellation");
        }
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config).expect("reconcile startup");
        assert_eq!(
            reopened
                .receipt_ledger
                .recover(key.clone(), Instant::now() + Duration::from_secs(2)),
            Err(ReceiptLedgerError::ReceiptNotFound)
        );
        let snapshot = reopened
            .resolve_task(
                key.reserved_task_id(),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("resolve recovered handoff Task");
        match (phase, cancel_requested, snapshot) {
            (
                AttemptPhase::NotBegun,
                false,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason: V5SafeFailureReason::Interrupted,
                    cancel_requested: false,
                    ..
                },
            )
            | (
                AttemptPhase::NotBegun,
                true,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                    cancel_requested: true,
                    ..
                },
            )
            | (
                AttemptPhase::Begun,
                false,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason: V5SafeFailureReason::OutcomeUncertain,
                    cancel_requested: false,
                    ..
                },
            )
            | (
                AttemptPhase::Begun,
                true,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason: V5SafeFailureReason::OutcomeUncertain,
                    cancel_requested: true,
                    ..
                },
            ) => {}
            (_, _, other) => panic!("unexpected recovered handoff Task: {other:?}"),
        }
    }
}

fn materialize_startup_task_bound(
    runtime: &V5ReceiptRuntime,
    identity: &CoreIdentity,
    phase: AttemptPhase,
) -> (ReceiptKey, V5StoredInvocationRecord, TaskBoundReceipt) {
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            deadline,
        )
        .expect("reserve materialized startup receipt")
        .into_reservation()
        .expect("new materialized startup receipt");
    let actor_bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            SafeIdentityHash::from_sha256(Sha256::digest(b"materialized-startup").into()),
            deadline,
        )
        .expect("bind materialized startup actor");
    let receipt_version = match phase {
        AttemptPhase::NotBegun => actor_bound.record_version(),
        AttemptPhase::Begun => runtime
            .receipt_ledger
            .mark_reserved_begun(key.clone(), actor_bound.record_version(), deadline)
            .expect("mark materialized startup receipt begun")
            .record_version(),
    };
    let handoff = runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            receipt_version,
            1_009,
            3_600_000,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .expect("begin materialized startup handoff");
    let (record, task_bound) = runtime
        .task_projection
        .materialize_bound_handoff(&handoff, 1_009, deadline, runtime.hooks.as_ref())
        .unwrap_or_else(|failure| panic!("materialize startup TaskBound: {}", failure.error));
    let task_bound = runtime
        .receipt_ledger
        .complete_bound_task_handoff(key.clone(), handoff.record_version(), task_bound, deadline)
        .expect("complete startup TaskBound ownership");
    (key, record, task_bound)
}

/// Two retirement passes meet at the snapshot: the hook holds each one
/// until the other arrives.
struct RetirementSnapshotBarrier {
    barrier: Arc<std::sync::Barrier>,
}

impl V5RuntimeHooks for RetirementSnapshotBarrier {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn pause(&self, point: V5PausePoint, _deadline: Instant) -> Result<(), ReceiptLedgerError> {
        if point == V5PausePoint::BeforeRetirementSnapshot {
            self.barrier.wait();
        }
        Ok(())
    }
}

#[test]
fn concurrent_terminal_retirement_is_idempotent() {
    let root = tempfile::tempdir().expect("temporary retirement-race state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical retirement-race state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    )
    .with_v5_epoch_clock_for_test(clock.clone())
    .with_runtime_hooks_for_test(Arc::new(RetirementSnapshotBarrier {
        barrier: Arc::clone(&barrier),
    }));
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open retirement-race daemon state");
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open retirement-race runtime");
    let (_key, queued, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
    clock.set(2_000);
    let provider_deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(2),
    );
    let terminal = runtime
        .task_projection
        .task_store
        .terminalize_recovered_exact(
            &queued.identity(),
            queued.version,
            RecoveryTerminalReason::InterruptedBeforeExecution,
            provider_deadline,
        )
        .expect("terminalize retirement-race Task");
    let V5StoredTask::Failed {
        terminal_epoch_ms,
        terminal_digest,
        ..
    } = &terminal.task
    else {
        panic!("retirement-race Task must be terminal")
    };
    runtime
        .task_projection
        .lifecycle_links
        .publish_task_terminal_bound(
            &task_bound,
            receipt_task_projection_from_store(&terminal)
                .unwrap_or_else(|failure| panic!("project terminal Task: {}", failure.error)),
            terminal.version,
            ClosedTerminalStatus::Failed,
            terminal_digest.clone(),
            *terminal_epoch_ms,
            provider_deadline,
        )
        .expect("publish retirement-race terminal link");
    clock.set(4_000_000);

    let runtime = Arc::new(runtime);
    let workers = (0..2)
        .map(|_| {
            let runtime = Arc::clone(&runtime);
            thread::spawn(move || {
                runtime.task_projection.retire_expired_terminal_tasks(
                    Instant::now() + Duration::from_secs(2),
                    runtime.hooks.as_ref(),
                )
            })
        })
        .collect::<Vec<_>>();
    let outcomes = workers
        .into_iter()
        .map(|worker| worker.join().expect("join retirement worker"))
        .collect::<Vec<_>>();

    assert!(
        outcomes.iter().all(Result::is_ok),
        "ordinary concurrent retirement produced a fail-stop failure"
    );
}

#[test]
fn startup_terminalizes_already_materialized_not_begun_task_bound() {
    for cancel_requested in [false, true] {
        let root = tempfile::tempdir().expect("temporary materialized TaskBound root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let state = DaemonStateDirectory::open(&state_root, &identity)
            .expect("open materialized TaskBound daemon state");
        let config = DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(50),
        );
        let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
        let (key, _record, _task_bound) =
            materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
        if cancel_requested {
            runtime
                .task_projection
                .cancel_bound_task(
                    key.reserved_task_id(),
                    Instant::now() + Duration::from_secs(2),
                )
                .unwrap_or_else(|failure| {
                    panic!("request materialized Task cancellation: {}", failure.error)
                })
                .expect("materialized Task exists");
        }
        drop(runtime);

        let reopened = V5ReceiptRuntime::open(&state, &config)
            .expect("reconcile already materialized TaskBound");
        let snapshot = reopened
            .resolve_task(
                key.reserved_task_id(),
                Instant::now() + Duration::from_secs(2),
            )
            .expect("resolve reconciled materialized Task");
        match (cancel_requested, snapshot) {
            (
                false,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
                    reason: V5SafeFailureReason::Interrupted,
                    ..
                },
            )
            | (
                true,
                crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Cancelled {
                    cancel_requested: true,
                    ..
                },
            ) => {}
            (_, other) => panic!("unexpected materialized TaskBound recovery: {other:?}"),
        }
    }
}

#[test]
fn startup_terminalizes_exact_working_begun_task_bound_as_outcome_uncertain() {
    let root = tempfile::tempdir().expect("temporary begun TaskBound root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open begun TaskBound daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let (key, record, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::Begun);
    let (_working, _working_bound) = runtime
        .task_projection
        .start_bound_task(&task_bound, record, Instant::now() + Duration::from_secs(2))
        .unwrap_or_else(|failure| panic!("start exact begun Task: {}", failure.error));
    drop(runtime);

    let reopened =
        V5ReceiptRuntime::open(&state, &config).expect("reconcile exact begun TaskBound");
    assert!(matches!(
        reopened
            .resolve_task(
                key.reserved_task_id(),
                Instant::now() + Duration::from_secs(2)
            )
            .expect("resolve reconciled begun Task"),
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
            reason: V5SafeFailureReason::OutcomeUncertain,
            ..
        }
    ));
}

#[test]
fn startup_rejects_queued_begun_task_bound_without_mutation() {
    let root = tempfile::tempdir().expect("temporary invalid begun TaskBound root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open invalid begun TaskBound daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let (key, queued, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
    assert_eq!(queued.task, V5StoredTask::Queued);
    runtime
        .task_projection
        .lifecycle_links
        .mark_task_bound_begun(
            &task_bound,
            queued.version,
            queued.updated_at_epoch_ms,
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("mark lifecycle link begun without advancing the queued Task fixture");
    drop(runtime);

    let error = match V5ReceiptRuntime::open(&state, &config) {
        Ok(_) => panic!("queued begun TaskBound must fail-stop startup"),
        Err(error) => error,
    };
    assert!(error.contains("TaskBound Begun requires exact Working Task"));

    let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
        .expect("retain TaskStore after failed startup");
    let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        Arc::new(SystemEpochMillisClock),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect TaskStore after failed startup");
    assert_eq!(
        store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read unchanged queued Task"),
        queued
    );
    assert_eq!(recovery.entries().len(), 1);
}

#[test]
fn startup_rejects_active_task_without_lifecycle_link_without_mutation() {
    let root = tempfile::tempdir().expect("temporary orphan Task root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open orphan Task daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let workspace_identity =
        SafeIdentityHash::from_sha256(Sha256::digest(b"orphan-startup").into());
    let orphan = runtime
        .task_projection
        .task_store
        .create_exact(
            NewV5InvocationRecord::new(
                V5TaskIdentity::new(
                    key.reserved_task_id(),
                    key.invocation_id(),
                    receipt_key_digest(&key),
                ),
                key.tool(),
                key.normalized_arguments_hash().clone(),
                workspace_identity,
                V5_TASK_POLL_INTERVAL_MS,
                3_600_000,
            )
            .with_initial_epoch_ms(1_009),
            crate::domain::code_intelligence::ProviderDeadline::new(
                Instant::now() + Duration::from_secs(2),
            ),
        )
        .expect("create orphan startup Task");
    drop(runtime);

    let error = match V5ReceiptRuntime::open(&state, &config) {
        Ok(_) => panic!("active Task without lifecycle link must fail-stop startup"),
        Err(error) => error,
    };
    assert!(
        error.contains("TaskStore Task has no exact lifecycle link"),
        "unexpected startup failure: {error}"
    );

    let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
        .expect("retain orphan TaskStore after failed startup");
    let (store, recovery) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        Arc::new(SystemEpochMillisClock),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect orphan TaskStore after failed startup");
    assert_eq!(
        store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read unchanged orphan Task"),
        orphan
    );
    assert_eq!(recovery.entries().len(), 1);
}

#[test]
fn startup_receipt_loop_completes_exact_reserved_link_with_preexisting_queued_task() {
    let root = tempfile::tempdir().expect("temporary preexisting handoff root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open preexisting handoff daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            deadline,
        )
        .expect("reserve preexisting handoff receipt")
        .into_reservation()
        .expect("new preexisting handoff receipt");
    let workspace_identity =
        SafeIdentityHash::from_sha256(Sha256::digest(b"preexisting-handoff").into());
    let actor_bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            workspace_identity.clone(),
            deadline,
        )
        .expect("bind preexisting handoff actor");
    let handoff = runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            actor_bound.record_version(),
            1_009,
            3_600_000,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .expect("begin preexisting handoff");
    runtime
        .task_projection
        .lifecycle_links
        .reserve_task_link(
            key.clone(),
            handoff.link().clone(),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .expect("reserve exact preexisting Task link");
    runtime
        .task_projection
        .task_store
        .create_exact(
            NewV5InvocationRecord::new(
                V5TaskIdentity::new(
                    key.reserved_task_id(),
                    key.invocation_id(),
                    receipt_key_digest(&key),
                ),
                key.tool(),
                key.normalized_arguments_hash().clone(),
                workspace_identity,
                V5_TASK_POLL_INTERVAL_MS,
                3_600_000,
            )
            .with_initial_epoch_ms(1_009),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .expect("create exact preexisting queued Task");
    drop(runtime);

    let reopened = V5ReceiptRuntime::open(&state, &config)
        .expect("receipt loop completes preexisting handoff");
    assert!(matches!(
        reopened
            .resolve_task(
                key.reserved_task_id(),
                Instant::now() + Duration::from_secs(2)
            )
            .expect("resolve preexisting handoff Task"),
        crate::infrastructure::daemon::protocol_v5::V5DaemonTaskSnapshot::Failed {
            reason: V5SafeFailureReason::Interrupted,
            ..
        }
    ));
}

#[test]
fn startup_rejects_preexisting_handoff_task_without_prior_link_reservation() {
    let root = tempfile::tempdir().expect("temporary missing handoff reservation root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open missing handoff reservation daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let key = ReceiptKey::new(
        InvocationId::new(),
        TaskId::new(),
        RequestIdentity::new(
            identity.digest().clone(),
            V5ToolIdentity::View,
            normalized_arguments_hash(&serde_json::Map::new()),
            request_scope_hash("workspace-a").expect("request scope"),
        ),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let reserved = runtime
        .receipt_ledger
        .reserve(
            key.clone(),
            OriginalCutoffDescriptor::new(1_000, 7_000).expect("valid cutoff"),
            deadline,
        )
        .expect("reserve missing-reservation handoff receipt")
        .into_reservation()
        .expect("new missing-reservation handoff receipt");
    let workspace_identity =
        SafeIdentityHash::from_sha256(Sha256::digest(b"missing-handoff-reservation").into());
    let actor_bound = runtime
        .receipt_ledger
        .bind_reserved_actor(
            key.clone(),
            reserved.record_version(),
            workspace_identity.clone(),
            deadline,
        )
        .expect("bind missing-reservation handoff actor");
    runtime
        .receipt_ledger
        .begin_bound_task_handoff(
            key.clone(),
            actor_bound.record_version(),
            1_009,
            3_600_000,
            V5_TASK_POLL_INTERVAL_MS,
            deadline,
        )
        .expect("begin missing-reservation handoff");
    let queued = runtime
        .task_projection
        .task_store
        .create_exact(
            NewV5InvocationRecord::new(
                V5TaskIdentity::new(
                    key.reserved_task_id(),
                    key.invocation_id(),
                    receipt_key_digest(&key),
                ),
                key.tool(),
                key.normalized_arguments_hash().clone(),
                workspace_identity,
                V5_TASK_POLL_INTERVAL_MS,
                3_600_000,
            )
            .with_initial_epoch_ms(1_009),
            crate::domain::code_intelligence::ProviderDeadline::new(deadline),
        )
        .expect("create preexisting handoff Task without reservation");
    drop(runtime);

    let error = match V5ReceiptRuntime::open(&state, &config) {
        Ok(_) => panic!("handoff Task without prior reservation must fail-stop startup"),
        Err(error) => error,
    };
    assert!(error.contains("preexisting handoff Task has no exact prior link reservation"));

    let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
        .expect("retain handoff TaskStore after failed startup");
    let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        Arc::new(SystemEpochMillisClock),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect handoff TaskStore after failed startup");
    assert_eq!(
        store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read unchanged handoff Task"),
        queued
    );
    let links = TaskLifecycleLinkStoreV5::open(
        state.path().join("task-lifecycle-links"),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect lifecycle store after failed startup")
    .catalog_snapshot(crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(2),
    ))
    .expect("snapshot unchanged lifecycle store");
    assert!(links.entries().is_empty());
}

#[test]
fn startup_rejects_task_terminal_bound_that_does_not_confirm_exact_terminal_task() {
    let root = tempfile::tempdir().expect("temporary terminal mismatch root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity)
        .expect("open terminal mismatch daemon state");
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(50),
    );
    let runtime = V5ReceiptRuntime::open(&state, &config).expect("open initial runtime");
    let (key, queued, task_bound) =
        materialize_startup_task_bound(&runtime, &identity, AttemptPhase::NotBegun);
    let deadline = crate::domain::code_intelligence::ProviderDeadline::new(
        Instant::now() + Duration::from_secs(2),
    );
    let terminal = runtime
        .task_projection
        .task_store
        .terminalize_recovered_exact(
            &queued.identity(),
            queued.version,
            RecoveryTerminalReason::InterruptedBeforeExecution,
            deadline,
        )
        .expect("terminalize exact TaskStore record");
    let V5StoredTask::Failed {
        terminal_epoch_ms, ..
    } = &terminal.task
    else {
        panic!("recovery terminal must be Failed")
    };
    let wrong_digest: TerminalDigest = "ff".repeat(32).parse().expect("wrong terminal digest");
    runtime
        .task_projection
        .lifecycle_links
        .publish_task_terminal_bound(
            &task_bound,
            receipt_task_projection_from_store(&terminal)
                .unwrap_or_else(|failure| panic!("project exact terminal Task: {}", failure.error)),
            terminal.version,
            ClosedTerminalStatus::Failed,
            wrong_digest,
            *terminal_epoch_ms,
            deadline,
        )
        .expect("publish deliberately mismatched TaskTerminalBound");
    drop(runtime);

    let error = match V5ReceiptRuntime::open(&state, &config) {
        Ok(_) => panic!("mismatched TaskTerminalBound must fail-stop startup"),
        Err(error) => error,
    };
    assert!(error.contains("TaskTerminalBound does not confirm the exact terminal Task"));

    let task_root = RetainedDirectoryCapability::open(&state.path().join("tasks"))
        .expect("retain terminal TaskStore after failed startup");
    let (store, _) = FileInvocationStoreV5::open_retained_directory_inspect_only(
        task_root,
        Arc::new(SystemEpochMillisClock),
        crate::domain::code_intelligence::ProviderDeadline::new(
            Instant::now() + Duration::from_secs(2),
        ),
    )
    .expect("inspect terminal TaskStore after failed startup");
    assert_eq!(
        store
            .get(
                key.reserved_task_id(),
                crate::domain::code_intelligence::ProviderDeadline::new(
                    Instant::now() + Duration::from_secs(2),
                ),
            )
            .expect("read unchanged terminal Task"),
        terminal
    );
}

#[test]
fn cancel_reserved_reopens_with_the_original_absolute_7125ms_expiry() {
    let root = tempfile::tempdir().expect("temporary restart-stable receipt root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let clock = Arc::new(ManualEpochClock::new(1_000));
    let arguments = serde_json::Map::new();
    let request_identity = RequestIdentity::new(
        identity.digest().clone(),
        V5ToolIdentity::View,
        normalized_arguments_hash(&arguments),
        request_scope_hash("workspace-a").expect("request scope"),
    );
    let key = ReceiptKey::new(InvocationId::new(), TaskId::new(), request_identity);

    let initial = exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
        owner.cancel_invocation(key.clone())
    });
    assert!(matches!(
        initial,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending {
                accepted_epoch_ms: 1_000,
                phase: V5InvocationPhase::CancelReserved,
                ..
            }
        }
    ));

    clock.set(4_000);
    let duplicate = exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
        owner.cancel_invocation(key.clone())
    });
    assert_eq!(duplicate, initial, "reopen extended the cancellation TTL");

    clock.set(1_000 + CANCEL_RESERVATION_TTL_MS - 1);
    let before_expiry =
        exchange_once_with_epoch(&state_root, &identity, Arc::clone(&clock), |owner| {
            owner.recover_invocation_receipt(key.clone())
        });
    assert_eq!(before_expiry, initial);

    clock.set(1_000 + CANCEL_RESERVATION_TTL_MS);
    let expired = exchange_once_with_epoch(&state_root, &identity, clock, |owner| {
        owner.recover_invocation_receipt(key)
    });
    assert_eq!(
        expired,
        V5ServerResponse::Error {
            code: V5DaemonErrorCode::ReceiptNotFound,
        }
    );
}

#[test]
fn authenticated_pre_cancel_submit_and_recover_cross_the_actor_owned_runtime() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(300),
    );
    let server = thread::spawn(move || run_daemon(config));
    let _record = wait_for_v5_record(&state_root, &identity);

    let arguments = serde_json::Map::new();
    let invocation_id = InvocationId::new();
    let reserved_task_id = TaskId::new();
    let request_identity = RequestIdentity::new(
        identity.digest().clone(),
        V5ToolIdentity::View,
        normalized_arguments_hash(&arguments),
        request_scope_hash("workspace-a").expect("request scope"),
    );
    let key = ReceiptKey::new(invocation_id, reserved_task_id, request_identity);
    let invocation = V5InvocationRequest::new(
        invocation_id,
        reserved_task_id,
        V5ToolIdentity::View,
        arguments,
        "workspace-a".to_string(),
        7_000,
    )
    .expect("strict invocation");
    let unused_executable = std::path::PathBuf::from("unused-existing-v5-endpoint");

    let mut cancel_owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        unused_executable.clone(),
        Duration::from_millis(300),
    )
    .expect("connect authenticated cancel owner");
    let cancel = cancel_owner
        .cancel_invocation(key.clone())
        .expect("durably reserve pre-submit cancellation");
    assert!(matches!(
        cancel,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::ReceiptPending {
                phase: V5InvocationPhase::CancelReserved,
                cancel_requested: true,
                ..
            }
        }
    ));

    let mut submit_owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity.clone(),
        unused_executable.clone(),
        Duration::from_millis(300),
    )
    .expect("connect authenticated submit owner");
    let submit = submit_owner
        .submit_invocation(invocation)
        .expect("terminalize exact pre-cancelled submit");
    assert!(matches!(
        &submit,
        V5ServerResponse::Invocation {
            outcome: V5InvocationResponse::Direct { receipt }
        } if matches!(receipt.terminal(), ReceiptTerminalOutcome::Cancelled)
            && receipt.receipt_key() == &key
    ));

    let mut recover_owner = V5DaemonProcessOwner::connect_or_spawn(
        &state_root,
        identity,
        unused_executable,
        Duration::from_millis(300),
    )
    .expect("connect authenticated recovery owner");
    let recovered = recover_owner
        .recover_invocation_receipt(key)
        .expect("recover committed direct terminal");
    assert_eq!(recovered, submit, "recovery changed the prepared response");
    drop(cancel_owner);
    drop(submit_owner);
    drop(recover_owner);

    server.join().expect("join v5 runtime").expect("v5 runtime");
}

fn wait_for_v5_record(
    state_root: &std::path::Path,
    core_identity: &CoreIdentity,
) -> V5EndpointRecord {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let state = DaemonStateDirectory::open(state_root, core_identity)
            .expect("open v5 daemon state while waiting");
        if let Some(record) = state
            .read_v5_endpoint_record()
            .expect("read v5 endpoint record")
        {
            return record;
        }
        assert!(Instant::now() < deadline, "v5 endpoint was not published");
        thread::sleep(Duration::from_millis(5));
    }
}

fn connect_v5_owner(
    record: &V5EndpointRecord,
    identity: &CoreIdentity,
    owner_lease: &str,
) -> (TcpStream, BufReader<TcpStream>) {
    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon owner");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound v5 owner response read");
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 owner stream"));
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": owner_lease
        }),
    );
    let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 owner ready");
    assert!(matches!(
        decode_v5_server_response(&ready),
        Ok(V5ServerResponse::Ready { .. })
    ));
    (stream, reader)
}

#[test]
fn v5_owner_session_accepts_ping_then_release() {
    let root = tempfile::tempdir().expect("temporary session state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical session state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut stream, mut reader) =
        connect_v5_owner(&record, &identity, "44444444-4444-4444-8444-444444444444");

    write_json_line(&mut stream, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    assert_eq!(decode_v5_server_response(&pong), Ok(V5ServerResponse::Pong));

    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(stream);

    server
        .join()
        .expect("join v5 session runtime")
        .expect("v5 session runtime");
}

#[test]
fn seven_second_wait_task_keeps_its_requested_operation_window() {
    let task_id = TaskId::new();
    let frame = serde_json::to_vec(&json!({
        "kind": "wait_task",
        "taskId": task_id.to_string(),
        "waitMs": 7_000
    }))
    .expect("serialize v5 wait request");
    let decoded = decode_v5_request_frame(frame).expect("decode v5 wait request");
    let received_at = Instant::now();

    let deadlines = v5_request_deadlines(&decoded, received_at).expect("derive v5 wait deadlines");

    assert_eq!(
        deadlines.operation.duration_since(received_at),
        Duration::from_secs(9)
    );
}

#[test]
fn v5_duplicate_live_owner_lease_is_rejected() {
    let root = tempfile::tempdir().expect("temporary duplicate-lease state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical duplicate-lease state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let lease = "55555555-5555-4555-8555-555555555555";
    let (mut first, mut first_reader) = connect_v5_owner(&record, &identity, lease);

    let mut duplicate = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect duplicate v5 owner");
    duplicate
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound duplicate response read");
    let mut duplicate_reader =
        BufReader::new(duplicate.try_clone().expect("clone duplicate stream"));
    write_json_line(
        &mut duplicate,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": lease
        }),
    );
    let response = read_bounded_v5_probe_response_frame(&mut duplicate_reader)
        .expect("read duplicate owner rejection");
    assert_eq!(
        decode_v5_server_response(&response),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::DuplicateLease,
        })
    );

    write_json_line(&mut first, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut first_reader).expect("release original v5 owner");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);
    drop(duplicate);

    server
        .join()
        .expect("join duplicate-lease runtime")
        .expect("duplicate-lease runtime");
}

#[test]
fn v5_rejects_connections_above_handshake_limit() {
    let root = tempfile::tempdir().expect("temporary handshake-limit state root");
    let state_root =
        std::fs::canonicalize(root.path()).expect("physical handshake-limit state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(100),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let address = record.loopback_addr().expect("v5 loopback address");
    let blockers = (0..MAX_HANDSHAKES)
        .map(|_| TcpStream::connect(address).expect("occupy v5 handshake slot"))
        .collect::<Vec<_>>();
    thread::sleep(Duration::from_millis(100));

    let overflow = TcpStream::connect(address).expect("connect overflow v5 handshake");
    overflow
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound overflow response read");
    let mut overflow_reader = BufReader::new(overflow);
    let response = read_bounded_v5_probe_response_frame(&mut overflow_reader)
        .expect("read overloaded v5 handshake response");
    assert_eq!(
        decode_v5_server_response(&response),
        Ok(V5ServerResponse::Error {
            code: V5DaemonErrorCode::Overloaded,
        })
    );

    drop(blockers);
    server
        .join()
        .expect("join handshake-limit runtime")
        .expect("handshake-limit runtime");
}

#[test]
fn live_v5_owner_prevents_idle_listener_shutdown() {
    let root = tempfile::tempdir().expect("temporary owner-idle state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical owner-idle state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let (mut first, mut first_reader) =
        connect_v5_owner(&record, &identity, "66666666-6666-4666-8666-666666666666");

    thread::sleep(Duration::from_millis(200));
    let (mut successor, mut successor_reader) =
        connect_v5_owner(&record, &identity, "77777777-7777-4777-8777-777777777777");
    write_json_line(&mut successor, &json!({"kind": "release"}));
    let successor_released = read_bounded_v5_probe_response_frame(&mut successor_reader)
        .expect("release successor v5 owner");
    assert_eq!(
        decode_v5_server_response(&successor_released),
        Ok(V5ServerResponse::Released)
    );
    drop(successor);

    write_json_line(&mut first, &json!({"kind": "release"}));
    let first_released = read_bounded_v5_probe_response_frame(&mut first_reader)
        .expect("release original idle-fencing owner");
    assert_eq!(
        decode_v5_server_response(&first_released),
        Ok(V5ServerResponse::Released)
    );
    drop(first);

    server
        .join()
        .expect("join owner-idle runtime")
        .expect("owner-idle runtime");
}

#[test]
fn exact_v5_runtime_opens_receipt_ledger_and_serves_real_handshake_and_ping() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);

    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("bound v5 response read");
    write_json_line(
        &mut stream,
        &json!({
            "kind": "hello",
            "protocolVersion": 5,
            "token": record.token(),
            "coreIdentity": identity.as_str(),
            "ownerLease": "33333333-3333-4333-8333-333333333333"
        }),
    );
    let mut reader = BufReader::new(stream.try_clone().expect("clone v5 stream"));
    let ready = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 ready");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&ready).expect("decode v5 ready"),
        json!({
            "kind": "ready",
            "protocolVersion": 5,
            "coreIdentity": identity.as_str(),
            "daemonPid": std::process::id(),
            "instanceId": record.instance_id()
        })
    );

    write_json_line(&mut stream, &json!({"kind": "ping"}));
    let pong = read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 pong");
    let pong: V5ProbeServerResponse = serde_json::from_slice(&pong).expect("decode strict v5 pong");
    assert_eq!(pong.kind(), V5ProbeResponseKind::Pong);
    write_json_line(&mut stream, &json!({"kind": "release"}));
    let released =
        read_bounded_v5_probe_response_frame(&mut reader).expect("read v5 release response");
    assert_eq!(
        decode_v5_server_response(&released),
        Ok(V5ServerResponse::Released)
    );
    drop(stream);

    server.join().expect("join v5 runtime").expect("v5 runtime");
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("reopen v5 state");
    assert!(state.read_v5_endpoint_record().unwrap().is_none());
    let receipts = state
        .create_private_retained_subdirectory("receipts")
        .expect("retain production receipts directory");
    assert_eq!(
        std::fs::read(receipts.path().join("generation")).expect("read v5 generation"),
        b"0\n"
    );
}

#[test]
fn direct_runtime_entry_rejects_every_non_v5_identity_before_state_creation() {
    use std::str::FromStr;

    for identity in [
        CoreIdentity::from_str("2f4dd5713d11e5211a92c5fa01b1ec5722dc3a3160b9b1e0b667f8d8da3d9c28")
            .expect("the retired protocol-v3 digest still parses as a canonical identity"),
        CoreIdentity::from_str("eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee")
            .expect("arbitrary accepted identity"),
    ] {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let result = run_daemon(DaemonServerConfig::new(
            state_root,
            identity,
            Duration::from_millis(10),
        ));

        assert_eq!(
            result,
            Err("protocol-v5 runtime requires the exact production-v5 core identity".to_string())
        );
        assert_eq!(
            std::fs::read_dir(root.path())
                .expect("read untouched root")
                .count(),
            0
        );
    }
}

#[test]
fn partial_handshake_bytes_cannot_replenish_the_absolute_frame_deadline() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("bind slowloris fixture");
    let address = listener.local_addr().expect("slowloris address");
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept slowloris fixture");
        stream
            .set_nonblocking(false)
            .expect("blocking fixture stream");
        let mut reader = BufReader::new(stream);
        let started = Instant::now();
        let result = read_v5_request_before(&mut reader, started + Duration::from_millis(60));
        done_tx
            .send((result.is_err(), started.elapsed()))
            .expect("report bounded read");
    });
    let mut client = TcpStream::connect(address).expect("connect slowloris fixture");
    for byte in b"{\"kind\":\"ping\"}\n" {
        if client.write_all(&[*byte]).is_err() {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let (rejected, elapsed) = done_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("absolute frame deadline must release the reader");
    assert!(rejected);
    assert!(elapsed < Duration::from_millis(180), "elapsed={elapsed:?}");
    server.join().expect("join slowloris fixture");
}

#[test]
fn expired_partial_handshake_closes_transport_without_a_late_protocol_response() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);

    let mut stream = TcpStream::connect(record.loopback_addr().expect("v5 loopback address"))
        .expect("connect v5 daemon");
    stream
        .set_read_timeout(Some(HANDSHAKE_READ_TIMEOUT + Duration::from_secs(1)))
        .expect("bound expired-handshake read");
    stream.write_all(b"{").expect("write partial handshake");
    let started = Instant::now();
    let mut response = Vec::new();
    if let Err(error) = stream.read_to_end(&mut response) {
        assert_eq!(
            error.kind(),
            io::ErrorKind::ConnectionReset,
            "expired handshake must close the transport: {error}"
        );
    }

    assert!(
        response.is_empty(),
        "transport timeout was misclassified as protocol response: {}",
        String::from_utf8_lossy(&response)
    );
    assert!(
        started.elapsed() < HANDSHAKE_READ_TIMEOUT + Duration::from_millis(500),
        "expired handshake received a replenished response budget: {:?}",
        started.elapsed()
    );
    server.join().expect("join v5 runtime").expect("v5 runtime");
}

#[test]
fn complete_v5_frame_near_cutoff_cannot_receive_a_fresh_response_budget() {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("bind response-deadline fixture");
    let address = listener.local_addr().expect("response-deadline address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept response-deadline fixture");
        let original_deadline = Instant::now() + Duration::from_millis(30);
        thread::sleep(Duration::from_millis(45));
        let result = write_json_line_before(
            &mut stream,
            &V5ProbeServerResponse::Pong {},
            original_deadline,
        );
        drop(stream);
        result
    });
    let mut client = TcpStream::connect(address).expect("connect response-deadline fixture");
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound response-deadline read");
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("expired response deadline closes transport");

    assert!(
        server
            .join()
            .expect("join response-deadline fixture")
            .is_err(),
        "expired original deadline granted a new response-write budget"
    );
    assert!(response.is_empty(), "late response escaped: {response:?}");
}

#[test]
fn displaced_receipt_authority_fail_stops_until_process_death() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let config = DaemonServerConfig::new(
        state_root.clone(),
        identity.clone(),
        Duration::from_millis(80),
    );
    let server = thread::spawn(move || run_daemon(config));
    let record = wait_for_v5_record(&state_root, &identity);
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
    let receipts = state.path().join("receipts");
    let displaced = state.path().join("receipts-displaced");

    match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
        .expect("attempt receipt authority replacement")
    {
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            server.join().expect("join v5 runtime").expect("v5 runtime");
        }
        RetainedDirectoryReplacementOutcome::Replaced => {
            let displaced_still_ready =
                match TcpStream::connect(record.loopback_addr().expect("old v5 address")) {
                    Ok(mut stream) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .expect("bound displaced-daemon read");
                        let hello = json!({
                            "kind": "hello",
                            "protocolVersion": 5,
                            "token": record.token(),
                            "coreIdentity": identity.as_str(),
                            "ownerLease": "33333333-3333-4333-8333-333333333333"
                        });
                        if serde_json::to_writer(&mut stream, &hello).is_ok()
                            && stream.write_all(b"\n").is_ok()
                        {
                            let mut reader = BufReader::new(
                                stream.try_clone().expect("clone displaced v5 stream"),
                            );
                            read_bounded_v5_probe_response_frame(&mut reader).is_ok()
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                };
            let server_result = server.join().expect("join displaced v5 runtime");
            assert!(
                !displaced_still_ready,
                "displaced receipt owner still accepted a handshake"
            );
            assert!(
                server_result.is_ok(),
                "process-owned fail-stop is a controlled daemon shutdown: {server_result:?}"
            );
            let retained_record = state
                .read_v5_endpoint_record()
                .expect("read fail-stop endpoint")
                .expect("fail-stop keeps the PID-bound endpoint until process death");
            assert_eq!(retained_record, record);
            assert!(
                V5ReceiptRuntime::open(
                    &state,
                    &DaemonServerConfig::new(state_root, identity, Duration::from_millis(120),),
                )
                .is_err(),
                "same-process successor bypassed the retained fail-stop authority"
            );
        }
    }
}

#[test]
fn displaced_runtime_retains_stable_authority_until_the_old_owner_drops() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
    let first = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        ),
    )
    .expect("open first runtime owner");
    let receipts = state.path().join("receipts");
    let displaced = state.path().join("receipts-displaced");

    match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
        .expect("attempt receipt authority replacement")
    {
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
        RetainedDirectoryReplacementOutcome::Replaced => {}
    }

    let successor_state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
    let successor_while_old_is_live = V5ReceiptRuntime::open(
        &successor_state,
        &DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        ),
    );
    assert!(
        successor_while_old_is_live.is_err(),
        "replacement receipts directory created a second live runtime authority"
    );

    drop(first);
    V5ReceiptRuntime::open(
        &successor_state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
    )
    .expect("successor acquires the stable authority after old owner drops");
}

#[test]
fn replacement_receipt_authority_directory_alone_cannot_create_a_successor_runtime() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
    let first = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        ),
    )
    .expect("open first runtime owner");
    let authority = state.path().join(".receipt-authority");
    let displaced = state.path().join(".receipt-authority-displaced");

    match attempt_retained_directory_replacement_for_test(&authority, &displaced)
        .expect("attempt stable receipt-authority replacement")
    {
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
        RetainedDirectoryReplacementOutcome::Replaced => {}
    }

    let successor_state =
        DaemonStateDirectory::open(&state_root, &identity).expect("open successor state");
    let successor_while_old_is_live = V5ReceiptRuntime::open(
        &successor_state,
        &DaemonServerConfig::new(
            state_root.clone(),
            identity.clone(),
            Duration::from_millis(80),
        ),
    );

    let error = match successor_while_old_is_live {
        Ok(_) => {
            panic!("replacement receipt-authority directory created a second live runtime")
        }
        Err(error) => error,
    };
    assert_eq!(
        error,
        "open protocol-v5 receipt ledger: receipt ledger is already owned"
    );
    first
        .ensure_named_authority()
        .expect("unchanged receipt ledger keeps the original runtime authoritative");

    drop(first);
    V5ReceiptRuntime::open(
        &successor_state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
    )
    .expect("successor acquires both authority layers after old owner drops");
}

#[test]
fn displaced_runtime_cannot_write_a_response_after_the_final_authority_check() {
    let root = tempfile::tempdir().expect("temporary state root");
    let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
    let identity = CoreIdentity::production_v5();
    let state = DaemonStateDirectory::open(&state_root, &identity).expect("open daemon state");
    let runtime = V5ReceiptRuntime::open(
        &state,
        &DaemonServerConfig::new(state_root, identity, Duration::from_millis(80)),
    )
    .expect("open runtime owner");
    let receipts = state.path().join("receipts");
    let displaced = state.path().join("receipts-displaced");
    match attempt_retained_directory_replacement_for_test(&receipts, &displaced)
        .expect("attempt receipt authority replacement")
    {
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => return,
        RetainedDirectoryReplacementOutcome::Replaced => {}
    }

    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .expect("bind displaced response fixture");
    let address = listener.local_addr().expect("displaced response address");
    let client = TcpStream::connect(address).expect("connect displaced response fixture");
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .expect("bound displaced response read");
    let (mut server, _) = listener
        .accept()
        .expect("accept displaced response fixture");
    let result = write_runtime_json_line_before(
        &mut server,
        &runtime,
        &V5ProbeServerResponse::Pong {},
        Instant::now() + Duration::from_secs(1),
    );
    drop(server);
    let mut reader = BufReader::new(client);
    let mut response = Vec::new();
    reader
        .read_to_end(&mut response)
        .expect("read displaced response transport");

    assert!(result.is_err(), "displaced runtime wrote a response");
    assert!(
        response.is_empty(),
        "displaced response escaped: {response:?}"
    );
}
