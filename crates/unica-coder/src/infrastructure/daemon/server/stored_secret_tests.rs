//! #980: a connection secret that reaches the runner through the operational
//! configuration, and comes back in the runner's answer, stays out of the
//! daemon's durable state and out of every Task read.
use super::actor_capacity_tests::{canonical_v13_service, install_runner_probe, LiveV5Daemon};
use super::*;
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;
use unica_execution::receipt_runtime::response::V5DaemonTaskSnapshot;

/// The runner reads the secret from the configuration it was given, records
/// that it saw it, and echoes it in its answer: bare and after a key.
const SECRET_ECHO_RUNNER: &str = r##"
use std::{env, fs, path::Path};
fn secret_in(dir: &Path) -> Option<String> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let Ok(text) = fs::read_to_string(entry.path()) else { continue };
        if let Some(start) = text.find("UnicaIbSecret") {
            let token: String = text[start..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
            return Some(token);
        }
    }
    None
}
fn main() {
    let args: Vec<String> = env::args().collect();
    let cwd = env::current_dir().unwrap();
    let config = args.iter().position(|arg| arg == "--config").map(|index| Path::new(&args[index + 1]).to_path_buf()).unwrap();
    let secret = secret_in(config.parent().unwrap()).or_else(|| secret_in(&config)).expect("the configuration carries the secret");
    fs::write(cwd.join("probe-saw-secret"), &secret).unwrap();
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let fail = !dry_run && fs::read_to_string(cwd.join("probe-mode")).unwrap_or_default().trim() == "fail";
    let extensions = args.iter().any(|arg| arg == "extensions");
    let command = if extensions { "extensions" } else { "infobase create" };
    if fail {
        eprintln!("login rejected for operator with {secret}");
        println!(r#"{{"ok":false,"command":"{command}","duration_ms":0,"error":{{"code":"platform_failure","message":"login rejected for operator: {secret}; password={secret}"}},"warnings":["{secret}"],"steps":[]}}"#);
        std::process::exit(1);
    }
    if extensions {
        let records = if dry_run { String::new() } else {
            format!(r#"{{"name":"Тест","version":"connection=Srvr=x;Pwd={secret}","name_prefix":null,"purpose":"{secret}","active":false,"safe_mode":true,"unsafe_action_protection":false,"used_in_distributed_infobase":false,"scope":"Pwd={secret}","hash_sum":"{secret}","security_profile_name":null}}"#)
        };
        println!(r#"{{"ok":true,"command":"extensions","duration_ms":0,"data":{{"ok":true,"provider_dispatched":{},"provider":{{"selected":"ibcmd","origin":{{"kind":"default"}}}},"requested":{{"kind":"all"}},"extensions":[{records}],"plan":"{secret}"}},"warnings":["{secret}"],"steps":[]}}"#, !dry_run);
    } else {
        let created = cwd.join("created.marker");
        if !dry_run {
            fs::write(&created, "probe created infobase").unwrap();
        }
        let status = if !dry_run { "ok" } else if created.exists() { "skipped" } else { "planned" };
        println!(r#"{{"ok":true,"command":"infobase create","duration_ms":0,"data":{{"ok":true,"provider_dispatched":{},"steps":[{{"target":"infobase","action":"create","status":"{status}","message":"opened as operator with {secret}","duration_ms":0}},{{"target":"edt_workspace","action":"import","status":"skipped","message":"password={secret}","duration_ms":0}}],"duration_ms":0}},"warnings":["{secret}"],"steps":[]}}"#, !dry_run);
    }
}
"##;

struct Case {
    op: &'static str,
    fail: bool,
    secret: String,
    workspace: PathBuf,
}

fn case(root: &Path, op: &'static str, fail: bool) -> Case {
    let secret = format!("UnicaIbSecret{}", uuid::Uuid::new_v4().simple());
    let workspace = root.join(format!(
        "{}-{}",
        op.replace('.', "-"),
        if fail { "fail" } else { "ok" }
    ));
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\ninfobase:\n  connection: 'File=build/ib'\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("v8project.local.yaml"),
        format!("infobases:\n  origin:\n    user: operator\n    password: {secret}\n"),
    )
    .unwrap();
    if fail {
        std::fs::write(workspace.join("probe-mode"), "fail").unwrap();
    }
    Case {
        op,
        fail,
        secret,
        workspace: std::fs::canonicalize(workspace).unwrap(),
    }
}

/// The text around the first occurrence of `secret`, with the secret masked,
/// so a failure names the field that carried it.
fn around(text: &str, secret: &str) -> Option<String> {
    let start = text.find(secret)?;
    let from = text[..start]
        .char_indices()
        .rev()
        .nth(80)
        .map_or(0, |(index, _)| index);
    let to = (start + secret.len() + 20).min(text.len());
    let to = (to..=text.len())
        .find(|index| text.is_char_boundary(*index))
        .unwrap();
    // The report names the field, never the value itself.
    Some(text[from..to].replace(secret, "<secret>"))
}

/// Every file under `root` whose bytes carry `secret`, except `kept`.
fn files_with(root: &Path, secret: &str, kept: &[PathBuf]) -> Vec<PathBuf> {
    unica_workspace::test_support::tree_snapshot(root)
        .into_iter()
        .map(|(relative, bytes)| (root.join(relative), bytes))
        .filter(|(path, bytes)| {
            !kept.iter().any(|kept| path.starts_with(kept))
                && bytes
                    .windows(secret.len())
                    .any(|window| window == secret.as_bytes())
        })
        .map(|(path, _)| path)
        .collect()
}

#[test]
fn a_connection_secret_from_configuration_and_runner_answer_is_not_stored_or_read_back() {
    let root = tempfile::tempdir().unwrap();
    install_runner_probe(root.path(), SECRET_ECHO_RUNNER);
    let cases = [
        case(root.path(), "infobase.create", false),
        case(root.path(), "infobase.create", true),
        case(root.path(), "extensions.list", false),
        case(root.path(), "extensions.list", true),
    ];
    let mut daemon = LiveV5Daemon::start(canonical_v13_service());
    let owner = daemon.owner();
    let mut leaks = Vec::new();
    let mut tasks = Vec::new();
    for case in &cases {
        let request = InvocationRequest::new(
            ToolIdentity::Run,
            serde_json::json!({"op": case.op, "args": {}, "dryRun": false}),
            case.workspace.to_string_lossy(),
            7_000,
        )
        .unwrap();
        let task = daemon.task_id(&owner, &request);
        let terminal = daemon.wait_terminal(&owner, task, Duration::from_secs(30));
        let read = daemon.get(&owner, task);
        let failed = match &terminal {
            V5DaemonTaskSnapshot::Completed { result, .. } => !result.ok,
            V5DaemonTaskSnapshot::Failed { .. } => true,
            other => panic!("{} did not settle: {other:?}", case.op),
        };
        assert_eq!(failed, case.fail, "{}: {terminal:?}", case.op);
        // The secret reached the runner through the configuration: the
        // test is not vacuous.
        assert_eq!(
            std::fs::read_to_string(case.workspace.join("probe-saw-secret")).unwrap(),
            case.secret,
            "{}",
            case.op
        );
        for (name, snapshot) in [("terminal", &terminal), ("get", &read)] {
            if let Some(context) = around(&format!("{snapshot:?}"), &case.secret) {
                leaks.push(format!(
                    "{} fail={}: {name} read: …{context}…",
                    case.op, case.fail
                ));
            }
        }
        tasks.push(task);
    }

    // Reads after a restart come from what the first life left on disk.
    daemon.stop(owner);
    daemon.restart(canonical_v13_service());
    let owner = daemon.owner();
    for (case, task) in cases.iter().zip(&tasks) {
        if format!("{:?}", daemon.get(&owner, *task)).contains(&case.secret) {
            leaks.push(format!("{} fail={}: get after restart", case.op, case.fail));
        }
    }

    // The daemon is stopped before its files are read: on Windows a live
    // daemon holds mandatory locks on its ledger, and what is stored is
    // what the stopped daemon left on disk.
    daemon.stop(owner);
    let state = daemon.state_directory();
    let stored = unica_workspace::test_support::tree_snapshot(&state);
    assert!(
        stored.keys().any(|path| path.starts_with("tasks")),
        "the Task store is on disk: {:?}",
        stored.keys().collect::<Vec<_>>()
    );
    assert!(
        stored.keys().any(|path| path.starts_with("receipts")),
        "the receipt store is on disk"
    );
    for case in &cases {
        for path in files_with(&state, &case.secret, &[]) {
            leaks.push(format!(
                "{} fail={}: stored in {}",
                case.op,
                case.fail,
                path.strip_prefix(&state).unwrap().display()
            ));
        }
        // The runner's own configuration projection and the probe's marker
        // hold the secret by design; nothing else in the workspace may.
        let kept = [
            case.workspace.join("v8project.local.yaml"),
            case.workspace.join(".build/unica/runner-project"),
            case.workspace.join("probe-saw-secret"),
        ];
        for path in files_with(&case.workspace, &case.secret, &kept) {
            leaks.push(format!(
                "{} fail={}: written to {}",
                case.op,
                case.fail,
                path.strip_prefix(&case.workspace).unwrap().display()
            ));
        }
    }
    assert!(leaks.is_empty(), "the connection secret leaked: {leaks:#?}");
}
