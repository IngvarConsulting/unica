//! Process-level lease acceptance. These tests prove Unix PID lifetime and
//! actual persisted-byte recovery, not power-loss durability. Windows remains
//! behind the prototype's explicit unsupported directory-durability gate.

use crate::application::execution::hydration::ReadPublication;
use crate::application::execution::{
    AcceptedPermit, AuthorityEpoch, CommitRequest, Effect, OwnerCapability, RunPermit,
};
use crate::domain::execution::{
    digest, CanonicalOutcome, FailureReason, Key, ObservationWindow, Tool,
};
use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
use crate::infrastructure::execution_store::file::OpeningStore;
use crate::infrastructure::execution_store::namespace::{
    AuthorityRoot, NamespaceSpec, ResolvedNamespace,
};
use crate::infrastructure::execution_store::owner::{OwnerError, PrototypeStoreOwner};
use crate::infrastructure::execution_store::OpenError;
use crate::infrastructure::platform::filesystem::execution_store::PublicationPoint;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_CONFIG: &str = "UNICA_EXECUTION_STORE_PROCESS_TEST";
const MESSAGE_PREFIX: &str = "EXECUTION_STORE_PROCESS_MESSAGE ";
const WATCHDOG: Duration = Duration::from_secs(30);
const FROZEN_TEST: &str = "infrastructure::platform::filesystem::execution_store::process_tests::frozen_owner_process_holds_lease_after_worker_drop_until_pid_exit";
const HEALTHY_TEST: &str = "infrastructure::platform::filesystem::execution_store::process_tests::quiescent_owner_process_releases_lease_while_pid_remains_alive";
const BEGUN_TEST: &str = "infrastructure::platform::filesystem::execution_store::process_tests::dead_owner_begun_without_outcome_recovers_uncertain_through_factory";
const KILLED_TEST: &str = "infrastructure::platform::filesystem::execution_store::process_tests::forced_process_death_recovers_accepted_begun_and_durable_outcome";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Mode {
    Frozen,
    Healthy,
    Begun,
    KillBeforeBeginRename,
    KillAfterBegun,
    KillAfterOutcomeDurable,
}

#[derive(Serialize, Deserialize)]
struct Configuration {
    root: PathBuf,
    root_identity: [u8; 32],
    spec: NamespaceSpec,
    key: Key,
    payload: String,
    mode: Mode,
}

struct Fixture {
    _temp: tempfile::TempDir,
    config: Configuration,
}

impl Fixture {
    fn create(mode: Mode) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let parent = std::fs::canonicalize(temp.path()).unwrap();
        let root_identity = fresh_identity();
        let root = parent.join("authority");
        drop(AuthorityRoot::provision(&root, root_identity).unwrap());
        Self {
            _temp: temp,
            config: Configuration {
                root,
                root_identity,
                spec: NamespaceSpec::new(fresh_identity(), fresh_identity()),
                key: Key::new(
                    InvocationId::new(),
                    TaskId::new(),
                    fresh_identity(),
                    Tool::Apply,
                    fresh_identity(),
                    fresh_identity(),
                ),
                payload: format!("synthetic completed mutation {}", uuid::Uuid::new_v4()),
                mode,
            },
        }
    }

    fn root(&self) -> AuthorityRoot {
        AuthorityRoot::open_existing(&self.config.root, self.config.root_identity).unwrap()
    }
}

fn fresh_identity() -> [u8; 32] {
    digest(uuid::Uuid::new_v4().as_bytes())
}

fn capability() -> OwnerCapability {
    OwnerCapability::new([0x49; 32])
}

fn only_commit(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(
        effects.len(),
        1,
        "expected one physical commit: {effects:?}"
    );
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("expected a commit, received {other:?}"),
    }
}

fn open_owner(
    root: &AuthorityRoot,
    spec: NamespaceSpec,
    allow_fresh: bool,
) -> Result<PrototypeStoreOwner, OwnerError> {
    let epoch = AuthorityEpoch::new(fresh_identity());
    let opening = match root.resolve(spec).map_err(OwnerError::Open)? {
        ResolvedNamespace::Fresh(ticket) => {
            assert!(allow_fresh, "a previously admitted namespace became fresh");
            OpeningStore::create_new(*ticket, epoch).map_err(OwnerError::Open)?
        }
        ResolvedNamespace::Existing(registration) => {
            OpeningStore::open_existing(*registration, epoch).map_err(OwnerError::Open)?
        }
    };
    // The public factory owns scan, restore, and recovery commits. A caller
    // cannot discard the catalog or expose an owner before recovery confirms.
    PrototypeStoreOwner::finish_opening(opening)
}

fn accept(owner: &mut PrototypeStoreOwner, key: &Key) -> AcceptedPermit {
    let request = only_commit(
        owner
            .submit(
                key.clone(),
                &capability(),
                ObservationWindow::new(10, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let mut accepted = owner.commit(request).unwrap();
    assert_eq!(accepted.len(), 1, "{accepted:?}");
    let Effect::Accepted(accepted) = accepted.remove(0) else {
        panic!("durable Submit must grant Accepted")
    };
    accepted
}

fn start(owner: &mut PrototypeStoreOwner, key: &Key) -> RunPermit {
    let accepted = accept(owner, key);
    let request = only_commit(owner.begin(accepted, 20, None).unwrap());
    let mut begun = owner.commit(request).unwrap();
    assert_eq!(begun.len(), 1, "{begun:?}");
    match begun.remove(0) {
        Effect::Run(permit) => permit,
        other => panic!("durable Begin must grant one Run: {other:?}"),
    }
}

fn assert_exact_terminal(owner: &mut PrototypeStoreOwner, config: &Configuration) {
    assert_eq!(
        owner
            .model()
            .lookup_task(config.key.task_id())
            .unwrap()
            .key(),
        &config.key,
    );
    let request = owner.model().prepare_task(&config.key).unwrap();
    let ReadPublication::Task(permit) = owner.read(request).unwrap() else {
        panic!("retained terminal must hydrate through Task")
    };
    let expected = CanonicalOutcome::completed(DomainResult::success(config.payload.clone()));
    assert_eq!(permit.key(), &config.key);
    assert_eq!(permit.outcome().bytes(), expected.bytes());
    assert_eq!(permit.outcome().digest(), expected.digest());
}

/// A TLS destructor runs after the storage thread's closure and captured
/// FileAggregateStore have been dropped. This is only a barrier: it cannot
/// keep a lease alive or make a failing store transition succeed.
struct WorkerExit(Sender<()>);
impl Drop for WorkerExit {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
thread_local! {
    static WORKER_EXIT: RefCell<Option<WorkerExit>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    Ready,
    Dropped,
    Closed,
    CrashBoundary,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    phase: Phase,
    pid: u32,
    executions: usize,
}

fn send_message(phase: Phase, executions: usize) {
    let message = Message {
        phase,
        pid: std::process::id(),
        executions,
    };
    let mut stdout = io::stdout().lock();
    // The leading newline separates our frame from libtest's progress prefix.
    writeln!(
        stdout,
        "\n{MESSAGE_PREFIX}{}",
        serde_json::to_string(&message).unwrap()
    )
    .unwrap();
    stdout.flush().unwrap();
}

fn wait_command(expected: &str) {
    let mut line = String::new();
    let read = io::stdin().lock().read_line(&mut line).unwrap();
    assert_ne!(read, 0, "parent closed the command pipe before {expected}");
    assert_eq!(line.trim_end(), expected);
}

fn run_child(config: Configuration) {
    let root = AuthorityRoot::open_existing(&config.root, config.root_identity).unwrap();
    let mut owner = open_owner(&root, config.spec.clone(), true).unwrap();
    let run = start(&mut owner, &config.key);
    let mut executions = 0;
    // The executor is synthetic, but its RunPermit came from real durable
    // Submit/Begin. The observed result is committed through the actual owner.
    executions += 1;
    let factual = CanonicalOutcome::completed(DomainResult::success(config.payload.clone()));
    if config.mode == Mode::Begun {
        // Work was authorized and its synthetic result exists only in this
        // process; neither report nor Outcome commit is performed.
        drop(factual);
        let _unused_run_permit = run;
        send_message(Phase::Ready, executions);
        wait_command("EXIT");
        drop(owner);
        drop(root);
        return;
    }
    let request = only_commit(owner.report(run.finish(factual)).unwrap());
    match config.mode {
        Mode::Frozen => {
            let (exit_sender, worker_exited) = mpsc::channel();
            let mut exit_sender = Some(exit_sender);
            let effects = owner
                .commit_with_publication_hook(request, move |point| {
                    if point == PublicationPoint::AfterRename {
                        WORKER_EXIT.with(|slot| {
                            assert!(slot.borrow().is_none());
                            *slot.borrow_mut() = Some(WorkerExit(exit_sender.take().unwrap()));
                        });
                        panic!("injected process acceptance panic after actual terminal rename");
                    }
                    Ok(())
                })
                .unwrap();
            assert!(
                matches!(effects.as_slice(), [Effect::FailStop]),
                "{effects:?}"
            );
            assert!(owner.model().unavailable());
            assert!(owner.model().first_known_write_failure().is_none());
            send_message(Phase::Ready, executions);
            wait_command("DROP");
            drop(owner);
            drop(root);
            worker_exited
                .recv_timeout(WATCHDOG)
                .expect("storage worker must finish after owner Drop");
            send_message(Phase::Dropped, executions);
        }
        Mode::Healthy => {
            assert!(owner.commit(request).unwrap().is_empty());
            send_message(Phase::Ready, executions);
            wait_command("CLOSE");
            owner.close_quiescent().unwrap();
            drop(root);
            send_message(Phase::Closed, executions);
        }
        Mode::Begun => unreachable!("handled before Outcome report"),
        Mode::KillBeforeBeginRename | Mode::KillAfterBegun | Mode::KillAfterOutcomeDurable => {
            unreachable!("separate forced-death child driver")
        }
    }
    wait_command("EXIT");
}

enum Output {
    Line(String),
    Error(String),
    End,
}

struct ChildOwner {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Receiver<Output>,
    reader: Option<JoinHandle<()>>,
    transcript: String,
}

impl ChildOwner {
    fn spawn(test_name: &str, config: &Configuration) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture", "--test-threads=1"])
            .env(CHILD_CONFIG, serde_json::to_string(config).unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let result = match line {
                    Ok(line) => sender.send(Output::Line(line)),
                    Err(error) => {
                        let _ = sender.send(Output::Error(error.to_string()));
                        return;
                    }
                };
                if result.is_err() {
                    return;
                }
            }
            let _ = sender.send(Output::End);
        });
        Self {
            child: Some(child),
            input: Some(input),
            output,
            reader: Some(reader),
            transcript: String::new(),
        }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.input.as_mut().unwrap(), "{command}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }

    fn receive_until(&mut self, deadline: Instant) -> Output {
        self.output
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| {
                panic!(
                    "child barrier failed: {error}; stdout:\n{}",
                    self.transcript
                )
            })
    }

    fn expect(&mut self, expected: Phase) {
        self.expect_executions(expected, 1);
    }

    fn expect_executions(&mut self, expected: Phase, executions: usize) {
        let deadline = Instant::now() + WATCHDOG;
        loop {
            match self.receive_until(deadline) {
                Output::Line(line) => {
                    self.transcript.push_str(&line);
                    self.transcript.push('\n');
                    if let Some(body) = line.strip_prefix(MESSAGE_PREFIX) {
                        let message: Message = serde_json::from_str(body).unwrap();
                        assert_eq!(message.phase, expected, "{}", self.transcript);
                        assert_eq!(message.pid, self.child.as_ref().unwrap().id());
                        assert_eq!(message.executions, executions);
                        self.assert_alive();
                        return;
                    }
                }
                Output::Error(error) => panic!("child stdout error: {error}"),
                Output::End => panic!("child ended before {expected:?}: {}", self.transcript),
            }
        }
    }

    fn assert_alive(&mut self) {
        assert!(
            self.child.as_mut().unwrap().try_wait().unwrap().is_none(),
            "child must still be alive at this barrier: {}",
            self.transcript
        );
    }

    fn finish(mut self) {
        self.send("EXIT");
        self.input.take();
        let deadline = Instant::now() + WATCHDOG;
        loop {
            match self.receive_until(deadline) {
                Output::Line(line) => {
                    self.transcript.push_str(&line);
                    self.transcript.push('\n');
                }
                Output::Error(error) => panic!("child stdout error at exit: {error}"),
                Output::End => break,
            }
        }
        let status = self.child.as_mut().unwrap().wait().unwrap();
        assert!(
            status.success(),
            "child failed: {status}; stdout:\n{}",
            self.transcript
        );
        self.child.take();
        self.reader.take().unwrap().join().unwrap();
    }

    fn kill_and_wait(mut self) {
        use std::os::unix::process::ExitStatusExt;
        self.assert_alive();
        self.child.as_mut().unwrap().kill().unwrap();
        let status = self.child.as_mut().unwrap().wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL), "{status}");
        self.child.take();
        self.input.take();
        self.reader.take().unwrap().join().unwrap();
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        self.input.take();
        if let Some(mut child) = self.child.take() {
            // Includes every assertion/timeout/panic path in the parent.
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn child_configuration(mode: Mode) -> Option<Configuration> {
    std::env::var(CHILD_CONFIG).ok().map(|value| {
        let config: Configuration = serde_json::from_str(&value).unwrap();
        assert_eq!(config.mode, mode);
        config
    })
}

#[test]
fn frozen_owner_process_holds_lease_after_worker_drop_until_pid_exit() {
    if let Some(config) = child_configuration(Mode::Frozen) {
        run_child(config);
        return;
    }
    let fixture = Fixture::create(Mode::Frozen);
    let mut child = ChildOwner::spawn(FROZEN_TEST, &fixture.config);
    child.expect(Phase::Ready);
    child.send("DROP");
    child.expect(Phase::Dropped);
    let root = fixture.root();
    let contender = open_owner(&root, fixture.config.spec.clone(), false);
    match contender {
        Err(OwnerError::Open(OpenError::Busy)) => {}
        Err(other) => panic!("expected OS ownership Busy, received {other:?}"),
        Ok(_) => panic!("live frozen owner allowed a new physical owner"),
    }
    child.assert_alive();
    child.finish(); // wait confirms actual process death before the reopen.
    let mut reopened = open_owner(&root, fixture.config.spec.clone(), false).unwrap();
    assert_exact_terminal(&mut reopened, &fixture.config);
    reopened.close_quiescent().unwrap();
}

#[test]
fn quiescent_owner_process_releases_lease_while_pid_remains_alive() {
    if let Some(config) = child_configuration(Mode::Healthy) {
        run_child(config);
        return;
    }
    let fixture = Fixture::create(Mode::Healthy);
    let mut child = ChildOwner::spawn(HEALTHY_TEST, &fixture.config);
    child.expect(Phase::Ready);
    child.send("CLOSE");
    child.expect(Phase::Closed);
    let root = fixture.root();
    let mut concurrent = open_owner(&root, fixture.config.spec.clone(), false).unwrap();
    child.assert_alive();
    assert_exact_terminal(&mut concurrent, &fixture.config);
    concurrent.close_quiescent().unwrap();
    child.assert_alive();
    child.finish();
}

#[test]
fn dead_owner_begun_without_outcome_recovers_uncertain_through_factory() {
    if let Some(config) = child_configuration(Mode::Begun) {
        run_child(config);
        return;
    }
    let fixture = Fixture::create(Mode::Begun);
    let mut child = ChildOwner::spawn(BEGUN_TEST, &fixture.config);
    child.expect(Phase::Ready);
    child.finish();
    let root = fixture.root();
    let mut recovered = open_owner(&root, fixture.config.spec.clone(), false).unwrap();
    let request = recovered.model().prepare_task(&fixture.config.key).unwrap();
    let ReadPublication::Task(result) = recovered.read(request).unwrap() else {
        panic!("completed recovery must expose the uncertain Task result")
    };
    let expected = CanonicalOutcome::failed(FailureReason::OutcomeUncertain);
    assert_eq!(result.key(), &fixture.config.key);
    assert_eq!(result.outcome().bytes(), expected.bytes());
    assert_eq!(result.outcome().digest(), expected.digest());
    // A second physical reopen proves the factory committed recovery rather
    // than synthesizing the uncertain outcome only in its in-memory model.
    recovered.close_quiescent().unwrap();
    let mut reopened = open_owner(&root, fixture.config.spec.clone(), false).unwrap();
    let request = reopened.model().prepare_task(&fixture.config.key).unwrap();
    let ReadPublication::Task(result) = reopened.read(request).unwrap() else {
        panic!("persisted recovery outcome")
    };
    assert_eq!(result.outcome().bytes(), expected.bytes());
    reopened.close_quiescent().unwrap();
}

fn crash_barrier(executions: usize) -> ! {
    send_message(Phase::CrashBoundary, executions);
    // The parent kills this PID while it is blocked on the command pipe.
    // Returning from a publication hook would permit the very callback whose
    // loss this case must prove, so even an unexpected command fails the child.
    wait_command("PARENT_MUST_KILL_THIS_PROCESS");
    panic!("a forced-death barrier must never return");
}

fn run_killed_child(config: Configuration) {
    let root = AuthorityRoot::open_existing(&config.root, config.root_identity).unwrap();
    let mut owner = open_owner(&root, config.spec.clone(), true).unwrap();
    let accepted = accept(&mut owner, &config.key);
    let begin = only_commit(owner.begin(accepted, 20, None).unwrap());
    if config.mode == Mode::KillBeforeBeginRename {
        owner
            .commit_with_publication_hook(begin, |point| {
                if point == PublicationPoint::BeforeRename {
                    crash_barrier(0);
                }
                Ok(())
            })
            .unwrap();
        panic!("Begin unexpectedly completed before SIGKILL");
    }
    let mut effects = owner.commit(begin).unwrap();
    assert_eq!(effects.len(), 1, "{effects:?}");
    let Effect::Run(run) = effects.remove(0) else {
        panic!("durable Begin must grant Run")
    };
    if config.mode == Mode::KillAfterBegun {
        // No Outcome publication has been requested. The real RunPermit is
        // still owned by this PID when it dies.
        let _authorized_run = run;
        crash_barrier(1);
    }
    assert_eq!(config.mode, Mode::KillAfterOutcomeDurable);
    let factual = CanonicalOutcome::completed(DomainResult::success(config.payload));
    let outcome = only_commit(owner.report(run.finish(factual)).unwrap());
    owner
        .commit_with_publication_hook(outcome, |point| {
            if point == PublicationPoint::AfterDirectorySync {
                crash_barrier(1);
            }
            Ok(())
        })
        .unwrap();
    panic!("durable Outcome callback unexpectedly completed before SIGKILL");
}

fn persisted_record(config: &Configuration) -> crate::domain::execution::storage::StoredRecord {
    let children: Vec<_> = std::fs::read_dir(config.root.join("namespaces"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(children.len(), 1);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(children[0].join("manifest.v1")).unwrap()).unwrap();
    let nonce: [u8; 16] =
        serde_json::from_value(manifest["registration"]["nonce"].clone()).unwrap();
    let rows: Vec<_> = std::fs::read_dir(children[0].join("records"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "aggregate")
        })
        .collect();
    assert_eq!(rows.len(), 1, "one whole aggregate must claim both IDs");
    let bytes = std::fs::read(&rows[0]).unwrap();
    crate::infrastructure::execution_store::codec::read_frame(
        &mut io::Cursor::new(&bytes),
        bytes.len() as u64,
        nonce,
    )
    .unwrap()
}

#[test]
fn forced_process_death_recovers_accepted_begun_and_durable_outcome() {
    if let Ok(value) = std::env::var(CHILD_CONFIG) {
        let config: Configuration = serde_json::from_str(&value).unwrap();
        run_killed_child(config);
        return;
    }
    for mode in [
        Mode::KillBeforeBeginRename,
        Mode::KillAfterBegun,
        Mode::KillAfterOutcomeDurable,
    ] {
        let fixture = Fixture::create(mode);
        let mut child = ChildOwner::spawn(KILLED_TEST, &fixture.config);
        child.expect_executions(
            Phase::CrashBoundary,
            usize::from(mode != Mode::KillBeforeBeginRename),
        );
        // This is a separate PID observer. Decode the actual row at the
        // blocked physical boundary, not a model snapshot or a staged file.
        let before = persisted_record(&fixture.config);
        assert_eq!(before.record().key(), &fixture.config.key);
        assert!(before.record().accepted());
        assert_eq!(
            before.record().run().is_some(),
            mode != Mode::KillBeforeBeginRename
        );
        assert_eq!(
            before.producer().is_some(),
            mode == Mode::KillAfterOutcomeDurable
        );
        child.kill_and_wait();
        assert_eq!(persisted_record(&fixture.config), before);
        let expected = match mode {
            Mode::KillBeforeBeginRename => CanonicalOutcome::failed(FailureReason::Interrupted),
            Mode::KillAfterBegun => CanonicalOutcome::failed(FailureReason::OutcomeUncertain),
            Mode::KillAfterOutcomeDurable => {
                CanonicalOutcome::completed(DomainResult::success(fixture.config.payload.clone()))
            }
            _ => unreachable!(),
        };
        let root = fixture.root();
        for _ in 0..2 {
            let mut recovered = open_owner(&root, fixture.config.spec.clone(), false).unwrap();
            assert!(!recovered.model().recovering());
            assert_eq!(
                recovered
                    .model()
                    .lookup_task(fixture.config.key.task_id())
                    .unwrap()
                    .key(),
                &fixture.config.key
            );
            assert_eq!(
                recovered
                    .model()
                    .inspect(&fixture.config.key)
                    .unwrap()
                    .run(),
                before.record().run(),
                "recovery must not grant a new execution"
            );
            let request = recovered.model().prepare_task(&fixture.config.key).unwrap();
            let ReadPublication::Task(result) = recovered.read(request).unwrap() else {
                panic!("Task recovery result")
            };
            assert_eq!(result.outcome().bytes(), expected.bytes());
            assert_eq!(result.outcome().digest(), expected.digest());
            assert!(
                recovered
                    .submit(
                        fixture.config.key.clone(),
                        &capability(),
                        ObservationWindow::new(10, 7_000).unwrap()
                    )
                    .unwrap()
                    .is_empty(),
                "duplicate Submit must not emit a new Commit or Run"
            );
            recovered.close_quiescent().unwrap();
        }
    }
}
