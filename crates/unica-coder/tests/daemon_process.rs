use serde_json::Value;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROCESS_FIXTURE_ENV: &str = "UNICA_DAEMON_PROCESS_FIXTURE";
const IDENTITY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const IDENTITY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PRODUCTION_V5_IDENTITY: &str =
    "884b76181583ce34907a2a9758e2b493e5b40883e7cbb0d7f88dcec0e468cfa0";
const STALE_ENDPOINT_INITIAL_IDLE_GRACE_MS: u64 = 500;

#[test]
fn daemon_frontend_process_fixture() {
    if std::env::var_os(PROCESS_FIXTURE_ENV).is_none() {
        return;
    }
    let state_root = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_STATE_ROOT").unwrap());
    let identity = std::env::var("UNICA_DAEMON_TEST_CORE_IDENTITY").unwrap();
    let executable = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_EXECUTABLE").unwrap());
    let ready = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_READY").unwrap());
    let go = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_GO").unwrap());
    let result = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_RESULT").unwrap());
    let release = PathBuf::from(std::env::var("UNICA_DAEMON_TEST_RELEASE").unwrap());
    std::fs::write(&ready, b"ready").unwrap();
    wait_until(
        Duration::from_secs(10),
        || go.exists(),
        "race release marker",
    );

    let mut owner = unica_coder::interfaces::daemon::connect_owner_for_protocol_test(
        &state_root,
        &identity,
        &executable,
        350,
    )
    .unwrap();
    owner.ping().unwrap();
    std::fs::write(&result, owner.daemon_pid().to_string()).unwrap();
    wait_until(
        Duration::from_secs(10),
        || release.exists(),
        "owner release marker",
    );
    owner.ping().unwrap();
}

#[test]
fn two_frontend_processes_race_to_one_daemon_pid_record_and_endpoint() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let go = state_root.join("go");
    let release = state_root.join("release");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_unica"));
    let mut first = spawn_frontend(&state_root, IDENTITY_A, &executable, "first", &go, &release);
    let mut second = spawn_frontend(
        &state_root,
        IDENTITY_A,
        &executable,
        "second",
        &go,
        &release,
    );
    wait_for_frontend_ready(&state_root, &["first", "second"]);
    std::fs::write(&go, b"go").unwrap();
    wait_for_frontend_results(&state_root, &["first", "second"]);

    let first_pid = read_pid(state_root.join("first.result"));
    let second_pid = read_pid(state_root.join("second.result"));
    assert_eq!(first_pid, second_pid);
    let endpoint = read_endpoint(&state_root, IDENTITY_A);
    assert_eq!(endpoint["pid"], first_pid);
    assert_eq!(endpoint["host"], "127.0.0.1");
    assert!(endpoint["port"].as_u64().is_some_and(|port| port > 0));
    let competing = Command::new(&executable)
        .args([
            "--daemon",
            "--state-root",
            state_root.to_str().unwrap(),
            "--core-identity",
            IDENTITY_A,
            "--idle-grace-ms",
            "350",
        ])
        .output()
        .unwrap();
    assert!(!competing.status.success());
    assert!(String::from_utf8_lossy(&competing.stderr)
        .contains("task store already has an active owner"));
    assert_eq!(read_endpoint(&state_root, IDENTITY_A), endpoint);

    std::fs::write(&release, b"release").unwrap();
    assert_child_success(&mut first);
    assert_child_success(&mut second);
    wait_until(
        Duration::from_secs(5),
        || !endpoint_path(&state_root, IDENTITY_A).exists(),
        "owned endpoint removal",
    );
}

#[test]
fn incompatible_core_identities_spawn_separate_process_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let go = state_root.join("go");
    let release = state_root.join("release");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_unica"));
    let mut first = spawn_frontend(
        &state_root,
        IDENTITY_A,
        &executable,
        "identity-a",
        &go,
        &release,
    );
    let mut second = spawn_frontend(
        &state_root,
        IDENTITY_B,
        &executable,
        "identity-b",
        &go,
        &release,
    );
    wait_for_frontend_ready(&state_root, &["identity-a", "identity-b"]);
    std::fs::write(&go, b"go").unwrap();
    wait_for_frontend_results(&state_root, &["identity-a", "identity-b"]);

    let first_pid = read_pid(state_root.join("identity-a.result"));
    let second_pid = read_pid(state_root.join("identity-b.result"));
    assert_ne!(first_pid, second_pid);
    assert_eq!(read_endpoint(&state_root, IDENTITY_A)["pid"], first_pid);
    assert_eq!(read_endpoint(&state_root, IDENTITY_B)["pid"], second_pid);
    assert_ne!(
        endpoint_path(&state_root, IDENTITY_A),
        endpoint_path(&state_root, IDENTITY_B)
    );

    std::fs::write(&release, b"release").unwrap();
    assert_child_success(&mut first);
    assert_child_success(&mut second);
    wait_until(
        Duration::from_secs(5),
        || {
            !endpoint_path(&state_root, IDENTITY_A).exists()
                && !endpoint_path(&state_root, IDENTITY_B).exists()
        },
        "incompatible endpoint removal",
    );
}

#[test]
fn v5_frontend_process_spawns_the_same_binary_and_pings_the_v5_runtime() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_unica"));

    let mut owner = unica_coder::interfaces::daemon::connect_owner_for_protocol_test(
        &state_root,
        PRODUCTION_V5_IDENTITY,
        &executable,
        150,
    )
    .expect("spawn and connect exact protocol-v5 daemon");
    owner.ping().expect("ping exact protocol-v5 daemon");
    let endpoint = read_endpoint(&state_root, PRODUCTION_V5_IDENTITY);
    assert_eq!(endpoint["protocolVersion"], 5);
    assert_eq!(endpoint["coreIdentity"], PRODUCTION_V5_IDENTITY);
    assert_eq!(endpoint["pid"], owner.daemon_pid());
    drop(owner);

    wait_until(
        Duration::from_secs(5),
        || !endpoint_path(&state_root, PRODUCTION_V5_IDENTITY).exists(),
        "v5 owned endpoint removal",
    );
}

#[test]
fn stale_v5_endpoint_probe_preserves_budget_to_spawn_a_replacement() {
    let root = tempfile::tempdir().unwrap();
    let state_root = std::fs::canonicalize(root.path()).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_unica"));

    let owner = unica_coder::interfaces::daemon::connect_owner_for_protocol_test(
        &state_root,
        PRODUCTION_V5_IDENTITY,
        &executable,
        STALE_ENDPOINT_INITIAL_IDLE_GRACE_MS,
    )
    .expect("spawn the initial exact protocol-v5 daemon");
    let initial_pid = owner.daemon_pid();
    let blackhole = TcpListener::bind(("127.0.0.1", 0)).expect("bind stale endpoint blackhole");
    let mut stale_endpoint = read_endpoint(&state_root, PRODUCTION_V5_IDENTITY);
    let stale_instance = stale_endpoint["instanceId"].clone();
    stale_endpoint["port"] = Value::from(blackhole.local_addr().expect("blackhole address").port());
    let endpoint_record_path = endpoint_path(&state_root, PRODUCTION_V5_IDENTITY);
    let mut stale_bytes = serde_json::to_vec(&stale_endpoint).expect("serialize stale endpoint");
    stale_bytes.push(b'\n');
    let mut endpoint_file = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&endpoint_record_path)
        .expect("open the production-created owner-only endpoint");
    endpoint_file
        .write_all(&stale_bytes)
        .and_then(|()| endpoint_file.sync_all())
        .expect("persist stale endpoint on the same owner-only file identity");
    drop(endpoint_file);
    drop(owner);
    thread::sleep(Duration::from_millis(
        STALE_ENDPOINT_INITIAL_IDLE_GRACE_MS + 250,
    ));
    assert_eq!(
        read_endpoint(&state_root, PRODUCTION_V5_IDENTITY),
        stale_endpoint,
        "the original daemon removed a stale endpoint record it no longer owned"
    );

    let started = Instant::now();
    let replacement = unica_coder::interfaces::daemon::connect_owner_for_protocol_test(
        &state_root,
        PRODUCTION_V5_IDENTITY,
        &executable,
        100,
    );
    let elapsed = started.elapsed();
    drop(blackhole);
    let mut replacement = replacement.expect(
        "a stale endpoint handshake must leave enough of the single startup deadline to spawn",
    );

    replacement
        .ping()
        .expect("ping the replacement exact protocol-v5 daemon");
    assert_ne!(replacement.daemon_pid(), initial_pid);
    let replacement_endpoint = read_endpoint(&state_root, PRODUCTION_V5_IDENTITY);
    assert_ne!(replacement_endpoint["instanceId"], stale_instance);
    assert_ne!(replacement_endpoint["port"], stale_endpoint["port"]);
    assert!(
        elapsed < Duration::from_secs(4),
        "stale endpoint probes consumed the spawn budget for {elapsed:?}"
    );
    drop(replacement);

    wait_until(
        Duration::from_secs(5),
        || !endpoint_path(&state_root, PRODUCTION_V5_IDENTITY).exists(),
        "replacement v5 endpoint removal",
    );
}

fn spawn_frontend(
    state_root: &Path,
    identity: &str,
    executable: &Path,
    name: &str,
    go: &Path,
    release: &Path,
) -> Child {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "daemon_frontend_process_fixture", "--nocapture"])
        .env(PROCESS_FIXTURE_ENV, "1")
        .env("UNICA_DAEMON_TEST_STATE_ROOT", state_root)
        .env("UNICA_DAEMON_TEST_CORE_IDENTITY", identity)
        .env("UNICA_DAEMON_TEST_EXECUTABLE", executable)
        .env(
            "UNICA_DAEMON_TEST_READY",
            state_root.join(format!("{name}.ready")),
        )
        .env("UNICA_DAEMON_TEST_GO", go)
        .env(
            "UNICA_DAEMON_TEST_RESULT",
            state_root.join(format!("{name}.result")),
        )
        .env("UNICA_DAEMON_TEST_RELEASE", release)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_frontend_ready(state_root: &Path, names: &[&str]) {
    wait_until(
        Duration::from_secs(10),
        || {
            names
                .iter()
                .all(|name| state_root.join(format!("{name}.ready")).exists())
        },
        "frontend readiness",
    );
}

fn wait_for_frontend_results(state_root: &Path, names: &[&str]) {
    wait_until(
        Duration::from_secs(10),
        || {
            names
                .iter()
                .all(|name| state_root.join(format!("{name}.result")).exists())
        },
        "frontend daemon results",
    );
}

fn read_pid(path: PathBuf) -> u64 {
    std::fs::read_to_string(path).unwrap().parse().unwrap()
}

fn endpoint_path(state_root: &Path, identity: &str) -> PathBuf {
    unica_coder::interfaces::daemon::endpoint_path_for_protocol_test(state_root, identity)
}

fn read_endpoint(state_root: &Path, identity: &str) -> Value {
    serde_json::from_slice(&std::fs::read(endpoint_path(state_root, identity)).unwrap()).unwrap()
}

fn assert_child_success(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            if !status.success() {
                let stderr = child
                    .stderr
                    .take()
                    .map(|stderr| std::io::read_to_string(stderr).unwrap_or_default())
                    .unwrap_or_default();
                panic!("frontend fixture failed with {status}: {stderr}");
            }
            return;
        }
        assert!(Instant::now() < deadline, "frontend fixture did not exit");
        thread::sleep(Duration::from_millis(20));
    }
}

fn wait_until(timeout: Duration, predicate: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + timeout;
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}
