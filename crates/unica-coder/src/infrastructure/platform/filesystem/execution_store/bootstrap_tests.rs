//! Public bootstrap/owner entry points against interrupted initialization and
//! replaced directory authority. The child case is SIGKILL, not power loss.
use crate::application::execution::{
    AuthorityEpoch, AuthorityFailureCause, CommitRequest, Effect, OwnerCapability,
};
use crate::domain::execution::{digest, CanonicalOutcome, Error, Key, ObservationWindow, Tool};
use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
use crate::infrastructure::execution_store::file::OpeningStore;
use crate::infrastructure::execution_store::namespace::{
    AuthorityRoot, NamespaceSpec, ResolvedNamespace,
};
use crate::infrastructure::execution_store::owner::{OwnerError, PrototypeStoreOwner};
use crate::infrastructure::execution_store::OpenError;
use crate::infrastructure::platform::filesystem::execution_store::{
    install_publication_hook, PublicationPoint,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CHILD_CONFIG: &str = "UNICA_EXECUTION_STORE_BOOTSTRAP_TEST";
const BARRIER_PREFIX: &str = "EXECUTION_STORE_REGISTRATION_DURABLE ";
const CHILD_TEST: &str = "infrastructure::platform::filesystem::execution_store::bootstrap_tests::interrupted_registration_refuses_fresh_after_actual_process_death";
const WATCHDOG: Duration = Duration::from_secs(30);

fn identity() -> [u8; 32] {
    digest(uuid::Uuid::new_v4().as_bytes())
}

#[derive(Serialize, Deserialize)]
struct Configuration {
    root: PathBuf,
    identity: [u8; 32],
    spec: NamespaceSpec,
}

struct Fixture {
    _temp: tempfile::TempDir,
    parent: PathBuf,
    config: Configuration,
    root: AuthorityRoot,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let parent = fs::canonicalize(temp.path()).unwrap();
        fs::create_dir(parent.join("route")).unwrap();
        let config = Configuration {
            root: parent.join("route/authority"),
            identity: identity(),
            spec: NamespaceSpec::new(identity(), identity()),
        };
        let root = AuthorityRoot::provision(&config.root, config.identity).unwrap();
        Self {
            _temp: temp,
            parent,
            config,
            root,
        }
    }
    fn create_owner(&self) -> PrototypeStoreOwner {
        let ResolvedNamespace::Fresh(ticket) = self.root.resolve(self.config.spec.clone()).unwrap()
        else {
            panic!("explicit new fixture must resolve Fresh")
        };
        let opening = OpeningStore::create_new(ticket, AuthorityEpoch::new(identity())).unwrap();
        PrototypeStoreOwner::finish_opening(opening).unwrap()
    }
    fn reopen(&self) -> Result<PrototypeStoreOwner, OwnerError> {
        let ResolvedNamespace::Existing(registration) = self
            .root
            .resolve(self.config.spec.clone())
            .map_err(OwnerError::Open)?
        else {
            panic!("previously initialized namespace must never resolve Fresh")
        };
        let opening = OpeningStore::open_existing(registration, AuthorityEpoch::new(identity()))
            .map_err(OwnerError::Open)?;
        PrototypeStoreOwner::finish_opening(opening)
    }
    fn namespace(&self) -> PathBuf {
        only_entry(&self.config.root.join("namespaces"))
    }
}

fn only_entry(path: &Path) -> PathBuf {
    let entries: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(entries.len(), 1, "{path:?}: {entries:?}");
    entries.into_iter().next().unwrap()
}

/// Snapshot includes names and directory entries, so accidental empty-store
/// creation is visible even when every preexisting sentinel byte survives.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn visit(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                result.insert(relative, None);
                visit(root, &path, result);
            } else {
                assert!(entry.file_type().unwrap().is_file());
                result.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn one_commit(mut effects: Vec<Effect>) -> CommitRequest {
    assert_eq!(effects.len(), 1, "{effects:?}");
    match effects.remove(0) {
        Effect::Commit(request) => request,
        other => panic!("expected physical Commit: {other:?}"),
    }
}

fn terminal(owner: &mut PrototypeStoreOwner) -> Key {
    let key = Key::new(
        InvocationId::new(),
        TaskId::new(),
        identity(),
        Tool::Apply,
        identity(),
        identity(),
    );
    let request = one_commit(
        owner
            .submit(
                key.clone(),
                &OwnerCapability::new(identity()),
                ObservationWindow::new(10, 7_000).unwrap(),
            )
            .unwrap(),
    );
    let Effect::Accepted(accepted) = owner.commit(request).unwrap().remove(0) else {
        panic!("Accepted")
    };
    let request = one_commit(owner.begin(accepted, 20, None).unwrap());
    let Effect::Run(run) = owner.commit(request).unwrap().remove(0) else {
        panic!("Run")
    };
    let request = one_commit(
        owner
            .report(
                run.finish(CanonicalOutcome::completed(DomainResult::success(format!(
                    "retained sentinel {}",
                    uuid::Uuid::new_v4()
                )))),
            )
            .unwrap(),
    );
    assert!(owner.commit(request).unwrap().is_empty());
    key
}

#[test]
fn incomplete_bootstrap_never_creates_empty_history() {
    for failure in ["orphan-child", "missing-manifest", "lost-ownership"] {
        let fixture = Fixture::new();
        let mut owner = fixture.create_owner();
        let key = terminal(&mut owner);
        owner.close_quiescent().unwrap();
        // Same public path succeeds before the one deliberate intervention.
        let mut healthy = fixture.reopen().unwrap();
        let request = healthy.model().prepare_task(&key).unwrap();
        healthy.read(request).unwrap();
        healthy.close_quiescent().unwrap();
        let source = match failure {
            "orphan-child" => only_entry(&fixture.config.root.join("registrations")),
            "missing-manifest" => fixture.namespace().join("manifest.v1"),
            "lost-ownership" => only_entry(&fixture.config.root.join("ownership")),
            _ => unreachable!(),
        };
        let witness = fixture.parent.join("removed-witness");
        fs::rename(&source, &witness).unwrap();
        let witness_bytes = fs::read(&witness).unwrap();
        let before = snapshot(&fixture.config.root);
        match (failure, fixture.reopen()) {
            ("orphan-child", Err(OwnerError::Open(OpenError::IncompleteBootstrap))) => {}
            (
                "missing-manifest" | "lost-ownership",
                Err(OwnerError::Open(OpenError::Io(error))),
            ) => {
                assert_eq!(error.kind(), io::ErrorKind::NotFound, "{failure}: {error}");
            }
            (_, Err(error)) => panic!("wrong refusal for {failure}: {error:?}"),
            (_, Ok(_)) => panic!("{failure} adopted an incomplete namespace"),
        }
        assert_eq!(snapshot(&fixture.config.root), before, "{failure}");
        assert!(
            !source.exists(),
            "{failure} silently recreated missing authority"
        );
        assert_eq!(fs::read(witness).unwrap(), witness_bytes);
    }
}

#[test]
fn root_and_ancestor_replacement_stop_owner_without_touching_foreign_namespace() {
    enum Prepared {
        Read(Box<crate::application::execution::hydration::ReadRequest>),
        Commit(Box<CommitRequest>),
    }
    for (replace_ancestor, write) in [(false, false), (true, false), (false, true), (true, true)] {
        let fixture = Fixture::new();
        let mut owner = fixture.create_owner();
        let key = terminal(&mut owner);
        let prepared = if write {
            Prepared::Commit(Box::new(one_commit(owner.expose_task(&key).unwrap())))
        } else {
            Prepared::Read(Box::new(owner.model().prepare_task(&key).unwrap()))
        };
        assert!(!owner.model().inspect(&key).unwrap().public_task());
        let named_namespace = fixture.namespace();
        let namespace_name = named_namespace.file_name().unwrap();
        let row_name = only_entry(&named_namespace.join("records"))
            .file_name()
            .unwrap()
            .to_os_string();
        let old_row = fs::read(named_namespace.join("records").join(&row_name)).unwrap();
        let moved = fixture.parent.join("retained-original");
        let target = if replace_ancestor {
            fixture.config.root.parent().unwrap()
        } else {
            fixture.config.root.as_path()
        };
        fs::rename(target, &moved).unwrap();
        if replace_ancestor {
            fs::create_dir(target).unwrap();
        }
        // A separately provisioned foreign root is a valid directory tree.
        // Add the old lexical row name to catch any fallback to that route.
        drop(AuthorityRoot::provision(&fixture.config.root, identity()).unwrap());
        let foreign_namespace = fixture.config.root.join("namespaces").join(namespace_name);
        fs::create_dir(&foreign_namespace).unwrap();
        fs::create_dir(foreign_namespace.join("records")).unwrap();
        fs::write(
            foreign_namespace.join("records").join(&row_name),
            b"foreign row sentinel",
        )
        .unwrap();
        fs::write(
            foreign_namespace.join("manifest.v1"),
            b"foreign manifest sentinel",
        )
        .unwrap();
        let before = snapshot(&fixture.config.root);
        assert!(!owner.model().unavailable());
        match prepared {
            Prepared::Read(request) => assert!(matches!(
                owner.read(*request),
                Err(OwnerError::Model(Error::StoreUnavailable))
            )),
            Prepared::Commit(request) => {
                let effects = owner.commit(*request).unwrap();
                assert!(
                    matches!(effects.as_slice(), [Effect::FailStop]),
                    "{effects:?}"
                );
                assert!(!owner.model().inspect(&key).unwrap().public_task());
                assert!(owner.model().first_known_write_failure().is_none());
            }
        }
        assert_eq!(
            owner.model().first_authority_failure().unwrap().cause(),
            AuthorityFailureCause::IdentityLost
        );
        assert!(owner.model().unavailable());
        assert!(
            AuthorityRoot::open_existing(&fixture.config.root, fixture.config.identity).is_err()
        );
        assert_eq!(snapshot(&fixture.config.root), before);
        let retained_root = if replace_ancestor {
            moved.join("authority")
        } else {
            moved
        };
        assert_eq!(
            fs::read(
                retained_root
                    .join("namespaces")
                    .join(namespace_name)
                    .join("records")
                    .join(row_name)
            )
            .unwrap(),
            old_row
        );
    }
}

fn registration_child(config: Configuration) {
    let root = AuthorityRoot::open_existing(&config.root, config.identity).unwrap();
    let ResolvedNamespace::Fresh(ticket) = root.resolve(config.spec).unwrap() else {
        panic!("child bootstrap must start fresh")
    };
    let registrations = config.root.join("registrations");
    let namespaces = config.root.join("namespaces");
    let _hook = install_publication_hook(move |point| {
        if point == PublicationPoint::AfterDirectorySync
            && fs::read_dir(&registrations)?.next().is_some()
        {
            assert!(fs::read_dir(&namespaces)?.next().is_none());
            let mut stdout = io::stdout().lock();
            writeln!(stdout, "\n{BARRIER_PREFIX}{}", std::process::id())?;
            stdout.flush()?;
            drop(stdout);
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            panic!("registration barrier must end only by parent SIGKILL");
        }
        Ok(())
    });
    let _opening = OpeningStore::create_new(ticket, AuthorityEpoch::new(identity())).unwrap();
    panic!("registration barrier was not reached");
}

struct RegistrationChild {
    child: Option<Child>,
    input: Option<ChildStdin>,
    output: Receiver<io::Result<String>>,
    reader: Option<JoinHandle<()>>,
}
impl RegistrationChild {
    fn spawn(config: &Configuration) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD_TEST, "--nocapture", "--test-threads=1"])
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
                if sender.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            child: Some(child),
            input: Some(input),
            output,
            reader: Some(reader),
        }
    }
    fn await_registration(&mut self) {
        let deadline = Instant::now() + WATCHDOG;
        let mut transcript = String::new();
        loop {
            let line = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("registration barrier: {error}; {transcript}"))
                .unwrap();
            transcript.push_str(&line);
            transcript.push('\n');
            if let Some(pid) = line.strip_prefix(BARRIER_PREFIX) {
                assert_eq!(
                    pid.parse::<u32>().unwrap(),
                    self.child.as_ref().unwrap().id()
                );
                assert!(self.child.as_mut().unwrap().try_wait().unwrap().is_none());
                return;
            }
        }
    }
    fn kill_and_wait(mut self) {
        use std::os::unix::process::ExitStatusExt;
        self.child.as_mut().unwrap().kill().unwrap();
        let status = self.child.as_mut().unwrap().wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
        self.child.take();
        self.input.take();
        self.reader.take().unwrap().join().unwrap();
    }
}
impl Drop for RegistrationChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.input.take();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

#[test]
fn interrupted_registration_refuses_fresh_after_actual_process_death() {
    if let Ok(value) = std::env::var(CHILD_CONFIG) {
        registration_child(serde_json::from_str(&value).unwrap());
        return;
    }
    let fixture = Fixture::new();
    let mut child = RegistrationChild::spawn(&fixture.config);
    child.await_registration();
    assert!(fs::read_dir(fixture.config.root.join("namespaces"))
        .unwrap()
        .next()
        .is_none());
    let registration = only_entry(&fixture.config.root.join("registrations"));
    let ownership = only_entry(&fixture.config.root.join("ownership"));
    assert_eq!(registration.file_name(), ownership.file_name());
    let before = snapshot(&fixture.config.root);
    child.kill_and_wait();
    let reopened_root =
        AuthorityRoot::open_existing(&fixture.config.root, fixture.config.identity).unwrap();
    assert!(matches!(
        reopened_root.resolve(fixture.config.spec.clone()),
        Err(OpenError::IncompleteBootstrap)
    ));
    assert_eq!(snapshot(&fixture.config.root), before);
    // A separate untouched authority proves the public fresh/open route works.
    let healthy = Fixture::new();
    healthy.create_owner().close_quiescent().unwrap();
    healthy.reopen().unwrap().close_quiescent().unwrap();
}
