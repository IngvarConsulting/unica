//! Real stdio boundaries: host context must reach the shared daemon per call.
#[path = "support/frontend_process.rs"]
mod frontend_process;

use frontend_process::{read_endpoint, spawn_owned_daemon, OwnedProcess, PRODUCTION_V5_IDENTITY};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn contract() -> Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/mcp/host-workspace-context.json"
    ))
    .unwrap()
}

struct Frontend {
    _process: OwnedProcess,
    input: ChildStdin,
    responses: Receiver<Value>,
    initialized: Value,
}

impl Frontend {
    fn start(cwd: &Path, state: &Path, environment: &[(&str, &str)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica"));
        command
            .current_dir(cwd)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            .env_remove("UNICA_RUNTIME_MANIFEST")
            .env_remove("UNICA_HOST_CONTEXT_REQUIRED")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for name in contract()["projectEnvironment"].as_array().unwrap() {
            command.env_remove(name.as_str().unwrap());
        }
        command.envs(environment.iter().copied());
        let mut process = OwnedProcess(command.spawn().unwrap());
        let input = process.0.stdin.take().unwrap();
        let output = process.0.stdout.take().unwrap();
        let (sender, responses) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                let Ok(response) = serde_json::from_str(&line) else {
                    break;
                };
                if sender.send(response).is_err() {
                    break;
                }
            }
        });
        let mut frontend = Self {
            _process: process,
            input,
            responses,
            initialized: Value::Null,
        };
        frontend.send(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25", "capabilities": {},
                "clientInfo": {"name": "host-context-integration", "version": "1"}
            }),
        );
        frontend.initialized = frontend.receive(1);
        assert!(
            frontend.initialized["result"].is_object(),
            "{}",
            frontend.initialized
        );
        writeln!(
            frontend.input,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        frontend.input.flush().unwrap();
        frontend
    }

    fn send(&mut self, id: u64, method: &str, params: Value) {
        writeln!(
            self.input,
            "{}",
            json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
        )
        .unwrap();
        self.input.flush().unwrap();
    }

    fn receive(&self, id: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let response = self
                .responses
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("bounded stdio response");
            if response["id"] == id {
                return response;
            }
        }
    }

    fn view(&mut self, id: u64, location: Option<Value>) -> Value {
        self.send(id, "tools/call", view_params(location));
        self.receive(id)
    }
}

fn view_params(location: Option<Value>) -> Value {
    let mut params = json!({"name":"unica.view", "arguments":{}});
    if let Some(location) = location {
        let fixture = contract();
        params["_meta"] = json!({fixture["metadataKey"].as_str().unwrap(): {
            fixture["metadataPath"].as_str().unwrap(): location
        }});
    }
    params
}

fn workspace(root: &Path, name: &str) -> std::path::PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::canonicalize(path).unwrap()
}

fn assert_workspace(response: &Value, expected: &Path) {
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{response:#}");
    assert_eq!(
        Path::new(
            result["data"]["workspaceRoot"]
                .as_str()
                .expect("workspace root path")
        ),
        expected,
        "{response:#}"
    );
}

#[test]
fn stdio_call_uses_host_workspace_instead_of_plugin_cwd() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project = workspace(root.path(), "проект A");
    let state = workspace(root.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start(&plugin, &state, &[]);
    frontend.send(10, "tools/list", json!({}));
    let listed = frontend.receive(10);
    for tool in listed["result"]["tools"].as_array().unwrap() {
        assert!(
            tool["inputSchema"]["properties"].get("cwd").is_none(),
            "the model must not choose workspace context: {tool:#}"
        );
    }
    let response = frontend.view(
        2,
        Some(json!(url::Url::from_directory_path(&project)
            .unwrap()
            .as_str())),
    );
    assert_workspace(&response, &project);
    let fixture = contract();
    assert_eq!(
        frontend.initialized["result"]["capabilities"]["experimental"]
            [fixture["metadataKey"].as_str().unwrap()],
        json!({})
    );
}

#[test]
fn two_frontends_and_interleaved_calls_keep_their_workspace_on_one_daemon() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project_a = workspace(root.path(), "project A");
    let project_b = root.path().join("worktree B");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .current_dir(&project_a)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Integration test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Integration test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    git(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "--allow-empty",
        "-m",
        "Fixture",
    ]);
    git(&[
        "worktree",
        "add",
        "--detach",
        project_b.to_str().unwrap(),
        "HEAD",
    ]);
    let project_b = std::fs::canonicalize(project_b).unwrap();
    // Only B has a source root: checking config state detects crossed data as well as paths.
    std::fs::create_dir_all(project_b.join("src/cf")).unwrap();
    std::fs::write(
        project_b.join("src/cf/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    let state = workspace(root.path(), "state");
    let daemon = spawn_owned_daemon(&state);
    let mut first = Frontend::start(&plugin, &state, &[]);
    let mut second = Frontend::start(&plugin, &state, &[]);
    let locations = [
        json!(url::Url::from_directory_path(&project_a).unwrap().as_str()),
        json!(project_b.to_str().unwrap()),
    ];
    first.send(2, "tools/call", view_params(Some(locations[0].clone())));
    first.send(3, "tools/call", view_params(Some(locations[1].clone())));
    second.send(4, "tools/call", view_params(Some(locations[0].clone())));
    // Receive both first-frontend responses without assuming completion order.
    let mut seen = std::collections::BTreeMap::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while seen.len() < 2 {
        let response = first
            .responses
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        if let Some(id @ (2 | 3)) = response["id"].as_u64() {
            seen.insert(id, response);
        }
    }
    assert_workspace(&seen[&2], &project_a);
    assert_workspace(&seen[&3], &project_b);
    assert_eq!(
        seen[&2]["result"]["structuredContent"]["data"]["config"]["state"],
        "missing"
    );
    assert_eq!(
        seen[&3]["result"]["structuredContent"]["data"]["config"]["state"],
        "autodetected"
    );
    assert_workspace(&second.receive(4), &project_a);
    drop(first);
    assert_workspace(&second.view(5, Some(locations[1].clone())), &project_b);
    assert_eq!(
        read_endpoint(&state, PRODUCTION_V5_IDENTITY)["pid"],
        daemon.0.id(),
        "both frontends must use the owned shared daemon"
    );
}

#[test]
fn startup_project_environment_and_request_override_are_forwarded() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project_a = workspace(root.path(), "проект A");
    let project_b = workspace(root.path(), "проект B");
    let state = workspace(root.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let fixture = contract();
    for name in fixture["projectEnvironment"].as_array().unwrap() {
        let mut frontend = Frontend::start(
            &plugin,
            &state,
            &[(name.as_str().unwrap(), project_a.to_str().unwrap())],
        );
        assert_workspace(&frontend.view(2, None), &project_a);
        assert_workspace(
            &frontend.view(
                3,
                Some(json!(url::Url::from_directory_path(&project_b)
                    .unwrap()
                    .as_str())),
            ),
            &project_b,
        );
        assert_workspace(&frontend.view(4, None), &project_a);
        assert_refusal(&frontend.view(5, Some(json!("relative/path"))));
    }
}

fn assert_refusal(response: &Value) {
    assert_eq!(
        response["result"]["isError"], true,
        "invalid host context must refuse rather than select process cwd: {response:#}"
    );
    assert_eq!(
        response["result"]["structuredContent"]["diagnostics"][0]["code"], "invalid_state",
        "{response:#}"
    );
    assert!(
        response["result"]["structuredContent"]["data"]["workspaceRoot"].is_null(),
        "refusal must not expose another workspace: {response:#}"
    );
}

#[test]
fn required_or_malformed_context_never_falls_back_to_process_cwd() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project = workspace(root.path(), "project");
    let state = workspace(root.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start(&plugin, &state, &[("UNICA_HOST_CONTEXT_REQUIRED", "1")]);
    assert_refusal(&frontend.view(2, None));
    let ordinary_file = project.join("not-a-directory.txt");
    std::fs::write(&ordinary_file, "ordinary file").unwrap();
    for (index, location) in [
        json!("relative/path"),
        json!("https://example.invalid/project"),
        json!(17),
        Value::Null,
        json!(root.path().join("missing").to_str().unwrap()),
        json!(ordinary_file.to_str().unwrap()),
    ]
    .into_iter()
    .enumerate()
    {
        assert_refusal(&frontend.view(3 + index as u64, Some(location)));
    }
    assert_workspace(
        &frontend.view(20, Some(json!(project.to_str().unwrap()))),
        &project,
    );
    assert_refusal(&frontend.view(21, None));
    // Explicit direct use retains cwd discovery when no host context is required.
    let mut direct = Frontend::start(&project, &state, &[]);
    assert_workspace(&direct.view(2, None), &project);
}

#[test]
fn conflicting_startup_project_environments_refuse_workspace_selection() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project_a = workspace(root.path(), "project A");
    let project_b = workspace(root.path(), "project B");
    let state = workspace(root.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let fixture = contract();
    let names = fixture["projectEnvironment"].as_array().unwrap();
    let mut frontend = Frontend::start(
        &plugin,
        &state,
        &[
            (names[0].as_str().unwrap(), project_a.to_str().unwrap()),
            (names[1].as_str().unwrap(), project_b.to_str().unwrap()),
        ],
    );
    assert_refusal(&frontend.view(2, None));
}
