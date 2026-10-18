use super::{endpoint_security, frontend_process};
use frontend_process::{endpoint_path, production_identity, read_endpoint, OwnedProcess};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use unica_coder::interfaces::daemon::{connect_owner_for_protocol_test, DaemonOwnerLease};

const IDLE_GRACE_MS: u64 = 2_000;

struct IdleDaemon {
    // Child cleanup precedes removal of its private state, including on panic.
    process: OwnedProcess,
    owner: Option<DaemonOwnerLease>,
    state: tempfile::TempDir,
    state_root: PathBuf,
}

impl IdleDaemon {
    fn start() -> Self {
        let state = tempfile::tempdir().unwrap();
        let state_root = state.path().canonicalize().unwrap();
        let process =
            frontend_process::spawn_owned_daemon_with_idle_grace(&state_root, IDLE_GRACE_MS);
        let mut owner = connect_owner_for_protocol_test(
            &state_root,
            production_identity(),
            Path::new(env!("CARGO_BIN_EXE_unica")),
            IDLE_GRACE_MS,
        )
        .expect("connect to the actual daemon child");
        owner
            .ping()
            .expect("authenticate and ping the daemon child");
        assert_eq!(owner.daemon_pid(), process.0.id());
        Self {
            process,
            owner: Some(owner),
            state,
            state_root,
        }
    }

    fn endpoint(&self) -> PathBuf {
        endpoint_path(&self.state_root, production_identity())
    }

    fn finish_idle(&mut self) {
        drop(self.owner.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self
                .process
                .0
                .try_wait()
                .expect("observe the actual daemon child")
            {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "daemon child did not exit after losing its last owner"
            );
            thread::sleep(Duration::from_millis(10));
        };
        assert!(
            status.success(),
            "daemon did not finish idle cleanup: {status}"
        );
        assert!(
            self.state.path().exists(),
            "test state must survive the process observation"
        );
    }
}

#[test]
fn idle_daemon_removes_its_own_endpoint_after_actual_process_exit() {
    let mut daemon = IdleDaemon::start();
    let endpoint = daemon.endpoint();
    endpoint_security::assert_owner_only(endpoint.parent().unwrap());
    endpoint_security::assert_owner_only(&endpoint);
    daemon.finish_idle();
    assert!(
        !endpoint.exists(),
        "the exited daemon left its own endpoint"
    );
}

#[test]
fn idle_daemon_preserves_same_pid_successor_on_the_original_file() {
    let mut daemon = IdleDaemon::start();
    let endpoint = daemon.endpoint();
    let identity = endpoint_security::file_identity(&endpoint);
    let mut successor = read_endpoint(&daemon.state_root, production_identity());
    for key in ["instanceId", "token"] {
        let first = "00000000-0000-4000-8000-000000000001";
        let second = "00000000-0000-4000-8000-000000000002";
        successor[key] = if successor[key] == first {
            second
        } else {
            first
        }
        .into();
    }
    assert_eq!(successor["pid"], daemon.process.0.id());
    let bytes = serde_json::to_vec(&successor).unwrap();
    let mut file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&endpoint)
        .unwrap();
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .unwrap();
    drop(file);
    assert_eq!(endpoint_security::file_identity(&endpoint), identity);

    daemon.finish_idle();

    assert_eq!(endpoint_security::file_identity(&endpoint), identity);
    assert!(
        std::fs::read(&endpoint).unwrap() == bytes,
        "idle cleanup changed the same-PID successor"
    );
}

#[test]
fn idle_daemon_preserves_a_byte_identical_replacement_file() {
    let mut daemon = IdleDaemon::start();
    let endpoint = daemon.endpoint();
    let identity = endpoint_security::file_identity(&endpoint);
    let permissions = endpoint_security::SecuritySnapshot::capture(&endpoint);
    let bytes = std::fs::read(&endpoint).unwrap();
    let staging = endpoint.with_file_name("replacement-endpoint.json");
    std::fs::write(&staging, &bytes).unwrap();
    permissions.apply_to(&staging);
    endpoint_security::assert_owner_only(&staging);
    std::fs::rename(&staging, &endpoint)
        .expect("replace the retained endpoint with another physical file");
    let replacement = endpoint_security::file_identity(&endpoint);
    assert_ne!(
        replacement, identity,
        "replacement fixture kept the original physical file"
    );

    daemon.finish_idle();

    assert_eq!(endpoint_security::file_identity(&endpoint), replacement);
    assert!(
        std::fs::read(&endpoint).unwrap() == bytes,
        "idle cleanup changed the byte-identical replacement"
    );
}

pub(super) fn insecure_endpoint_is_refused(
    directory: bool,
    install: impl FnOnce(&Path) -> endpoint_security::InsecureAccess,
) {
    let mut daemon = IdleDaemon::start();
    let endpoint = daemon.endpoint();
    let target = if directory {
        endpoint.parent().unwrap()
    } else {
        &endpoint
    };
    endpoint_security::assert_owner_only(target);
    let bytes = std::fs::read(&endpoint).unwrap();
    let identity = endpoint_security::file_identity(&endpoint);
    let permissions = install(target);
    let result = connect_owner_for_protocol_test(
        &daemon.state_root,
        production_identity(),
        Path::new(env!("CARGO_BIN_EXE_unica")),
        IDLE_GRACE_MS,
    );
    let error = match result {
        Ok(_) => panic!("a frontend accepted an insecure daemon endpoint"),
        Err(error) => error,
    };
    assert!(
        error.contains(if directory {
            "daemon identity directory is not owner-only"
        } else {
            "v5 daemon endpoint record is not owner-only"
        }),
        "unexpected refusal: {error}"
    );
    assert_eq!(endpoint_security::file_identity(&endpoint), identity);
    assert!(
        std::fs::read(&endpoint).unwrap() == bytes,
        "refusal changed the existing endpoint"
    );
    assert!(
        daemon.process.0.try_wait().unwrap().is_none(),
        "refusal killed the existing daemon"
    );
    drop(permissions);
    endpoint_security::assert_owner_only(target);
    daemon
        .owner
        .as_mut()
        .unwrap()
        .ping()
        .expect("the original authenticated owner still works");
    daemon.finish_idle();
    assert!(!endpoint.exists());
}

#[test]
fn frontend_refuses_an_insecure_daemon_endpoint_file() {
    insecure_endpoint_is_refused(false, endpoint_security::InsecureAccess::install);
}

#[test]
fn frontend_refuses_an_insecure_daemon_endpoint_directory() {
    insecure_endpoint_is_refused(true, endpoint_security::InsecureAccess::install);
}
