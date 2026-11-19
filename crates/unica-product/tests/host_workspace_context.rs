//! Real stdio boundaries: host context must reach the shared daemon per call.
#[path = "support/frontend_process.rs"]
mod frontend_process;

use frontend_process::{production_identity, read_endpoint, spawn_owned_daemon, OwnedProcess};
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

/// What the test client answers to `roots/list`.
enum RootsAnswer {
    Roots(Vec<String>),
    Error,
    Silent,
}

struct Frontend {
    _process: OwnedProcess,
    input: ChildStdin,
    responses: Receiver<Value>,
    initialized: Value,
    roots: Option<RootsAnswer>,
    roots_requests: usize,
    /// Speaks 2026-07-28: no initialize; roots come as an input request.
    modern: bool,
}

impl Frontend {
    fn start(cwd: &Path, state: &Path, environment: &[(&str, &str)]) -> Self {
        Self::start_with_roots(cwd, state, environment, None)
    }

    /// A client that declares `roots` answers `roots/list` with its current
    /// project on every request.
    fn start_with_roots(
        cwd: &Path,
        state: &Path,
        environment: &[(&str, &str)],
        roots: Option<RootsAnswer>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica"));
        command
            .current_dir(cwd)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            // Демон переживает MCP: без назначенной паузы он держит четверть
            // часа `unica` из `target`, и Windows не даёт следующей сборке
            // заменить этот файл. Тест может назначить свою паузу через
            // `environment`.
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
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
        let capabilities = if roots.is_some() {
            json!({"roots": {"listChanged": true}})
        } else {
            json!({})
        };
        let mut frontend = Self {
            _process: process,
            input,
            responses,
            initialized: Value::Null,
            roots,
            roots_requests: 0,
            modern: false,
        };
        frontend.send(
            1,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25", "capabilities": capabilities,
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

    fn receive(&mut self, id: u64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let message = self
                .responses
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .expect("bounded stdio response");
            // A server request shares the id space with responses: tell them
            // apart by `method`, never by the id alone.
            if message["method"] == "roots/list" {
                self.answer_roots(&message["id"]);
                continue;
            }
            if message.get("method").is_some() {
                continue;
            }
            if message["id"] == id {
                return message;
            }
        }
    }

    fn answer_roots(&mut self, id: &Value) {
        self.roots_requests += 1;
        let answer = match self.roots.as_ref().expect("roots were not declared") {
            RootsAnswer::Roots(uris) => json!({"jsonrpc":"2.0", "id": id, "result": {
                "roots": uris.iter().map(|uri| json!({"uri": uri})).collect::<Vec<_>>()
            }}),
            RootsAnswer::Error => json!({"jsonrpc":"2.0", "id": id, "error": {
                "code": -32603, "message": "roots are unavailable"
            }}),
            RootsAnswer::Silent => return,
        };
        writeln!(self.input, "{answer}").unwrap();
        self.input.flush().unwrap();
    }

    /// A 2026-07-28 client without initialize: every request names its
    /// protocol, and roots, when declared, answer an input request.
    fn start_modern(
        cwd: &Path,
        state: &Path,
        environment: &[(&str, &str)],
        roots: Option<RootsAnswer>,
    ) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica"));
        command
            .current_dir(cwd)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
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
        Self {
            _process: process,
            input,
            responses,
            initialized: Value::Null,
            roots,
            roots_requests: 0,
            modern: true,
        }
    }

    /// One tool call. A modern client answers the roots input request and
    /// repeats the call, as a 2026-07-28 host does; `roots_requests` counts them.
    fn call(&mut self, id: u64, name: &str, arguments: Value) -> Value {
        let mut params = json!({"name": name, "arguments": arguments});
        if self.modern {
            let capabilities = if self.roots.is_some() {
                json!({"roots": {"listChanged": true}})
            } else {
                json!({})
            };
            params["_meta"] = json!({
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {"name": "host-context-integration", "version": "1"},
                "io.modelcontextprotocol/clientCapabilities": capabilities
            });
        }
        self.send(id, "tools/call", params.clone());
        let response = self.receive(id);
        if response["result"]["resultType"] != "input_required" {
            return response;
        }
        let requests = &response["result"]["inputRequests"];
        assert_eq!(
            requests["unica/roots"]["method"], "roots/list",
            "{response:#}"
        );
        self.roots_requests += 1;
        let roots = match self.roots.as_ref().expect("roots were not declared") {
            RootsAnswer::Roots(uris) => uris
                .iter()
                .map(|uri| json!({"uri": uri}))
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        params["inputResponses"] = json!({"unica/roots": {"roots": roots}});
        let retry = id + 1_000_000;
        self.send(retry, "tools/call", params);
        self.receive(retry)
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
    let actual = Path::new(
        result["data"]["workspaceRoot"]
            .as_str()
            .expect("workspace root path"),
    );
    assert!(actual.is_absolute(), "{response:#}");
    assert_eq!(
        actual.canonicalize().expect("returned workspace directory"),
        expected
            .canonicalize()
            .expect("expected workspace directory"),
        "{response:#}"
    );
}

/// rule: INV.WORKSPACE.HOST-CONTEXT
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
    // The model names no directory of its own (#672): no free `cwd`; a
    // subject tool only selects inside the host's set with `workspace`.
    for tool in listed["result"]["tools"].as_array().unwrap() {
        let properties = &tool["inputSchema"]["properties"];
        assert!(properties.get("cwd").is_none(), "{tool:#}");
        let subject = !tool["name"].as_str().unwrap().starts_with("unica.task.");
        assert_eq!(
            properties["workspace"]["description"].is_string(),
            subject,
            "{tool:#}"
        );
    }
    let location = json!(url::Url::from_directory_path(&project).unwrap().as_str());
    let response = frontend.view(2, Some(location.clone()));
    assert_workspace(&response, &project);
    // The plugin's own directory is outside the set the host named: the
    // selection is refused, never served.
    let mut params = view_params(Some(location));
    params["arguments"] = json!({"workspace": plugin.to_str().unwrap()});
    frontend.send(3, "tools/call", params);
    assert_refusal(&frontend.receive(3));
    let fixture = contract();
    assert_eq!(
        frontend.initialized["result"]["capabilities"]["experimental"]
            [fixture["metadataKey"].as_str().unwrap()],
        json!({})
    );
}

/// rule: INV.WORKSPACE.HOST-CONTEXT
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
        read_endpoint(&state, production_identity())["pid"],
        daemon.0.id(),
        "both frontends must use the owned shared daemon"
    );
}

/// rule: INV.WORKSPACE.HOST-CONTEXT
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

/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn required_or_malformed_context_never_falls_back_to_process_cwd() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project = workspace(root.path(), "project");
    let state = workspace(root.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start(&plugin, &state, &[("UNICA_HOST_CONTEXT_REQUIRED", "1")]);
    let missing = frontend.view(2, None);
    assert_refusal(&missing);
    let result = &missing["result"]["structuredContent"];
    assert!(
        result.get("data").is_none(),
        "a frontend context refusal must not create a Task: {result}"
    );
    let message = result["summary"].as_str().unwrap();
    let fixture = contract();
    for variable in fixture["projectEnvironment"].as_array().unwrap() {
        assert!(message.contains(variable.as_str().unwrap()), "{message}");
    }
    for key in ["metadataKey", "metadataPath"] {
        assert!(
            message.contains(fixture[key].as_str().unwrap()),
            "{message}"
        );
    }
    assert!(message.contains("reconnect"), "{message}");
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

/// rule: INV.WORKSPACE.HOST-CONTEXT
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
    let captured = frontend.view(2, None);
    assert_refusal(&captured);
    let message = captured["result"]["structuredContent"]["summary"]
        .as_str()
        .unwrap();
    assert!(
        message.contains("startup workspace context")
            && message.contains("conflicting directories"),
        "{message}"
    );
    for variable in names {
        assert!(message.contains(variable.as_str().unwrap()), "{message}");
    }
    assert_workspace(
        &frontend.view(3, Some(json!(project_a.to_str().unwrap()))),
        &project_a,
    );
    assert_refusal(&frontend.view(4, None));
    let mut roots_frontend = Frontend::start_with_roots(
        &plugin,
        &state,
        &[
            (names[0].as_str().unwrap(), project_a.to_str().unwrap()),
            (names[1].as_str().unwrap(), project_b.to_str().unwrap()),
        ],
        Some(RootsAnswer::Roots(vec![url::Url::from_directory_path(
            &project_a,
        )
        .unwrap()
        .to_string()])),
    );
    assert_workspace(&roots_frontend.view(2, None), &project_a);
    roots_frontend.roots = Some(RootsAnswer::Roots(Vec::new()));
    assert_refusal(&roots_frontend.view(3, None));
}

/// A packaged core's build: the runtime manifest names its core archive.
fn write_core_manifest(path: &Path, archive_sha256: &str) {
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "darwin-arm64",
        ("linux", "x86_64") => "linux-x64",
        ("windows", "x86_64") => "win-x64",
        other => panic!("unsupported test host {other:?}"),
    };
    let manifest = json!({
        "schemaVersion": 2,
        "pluginVersion": "0.0.0",
        "source": {"repository": "test", "commit": "workspace"},
        "release": {"repository": "test", "tag": "workspace"},
        "artifacts": {"unica": {"version": "0.0.0", "role": "core", "targets": {target: {
            "asset": {"name": "unica.tar.gz", "url": "https://example.invalid/unica.tar.gz",
                      "mediaType": "application/gzip", "sha256": archive_sha256},
            "files": []
        }}}}
    });
    std::fs::write(path, manifest.to_string()).unwrap();
}

fn daemon_directories(state: &Path) -> Vec<String> {
    let mut names = std::fs::read_dir(state)
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("daemon-p5-"))
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// The frontend-to-daemon protocol keeps no backward compatibility, so a
/// daemon is keyed by the build: two builds on one state root never share a
/// daemon, and a frontend whose executable was replaced does not start the
/// new build under its own identity.
/// rule: INV.APP.HIDDEN-SERVICES
#[test]
fn each_build_gets_its_own_daemon_and_a_replaced_build_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project = workspace(root.path(), "project");
    let state = workspace(root.path(), "state");
    let fixture = contract();
    let project_environment = fixture["projectEnvironment"][0].as_str().unwrap();
    let manifest_a = root.path().join("build-a.json");
    let manifest_b = root.path().join("build-b.json");
    write_core_manifest(&manifest_a, &"a".repeat(64));
    write_core_manifest(&manifest_b, &"b".repeat(64));
    let start = |manifest: &Path| {
        Frontend::start(
            &plugin,
            &state,
            &[
                (project_environment, project.to_str().unwrap()),
                ("UNICA_RUNTIME_MANIFEST", manifest.to_str().unwrap()),
                ("UNICA_DAEMON_IDLE_GRACE_MS", "300"),
            ],
        )
    };

    // Build A's daemon is owned here so the test can stop it the way an
    // idle exit or a crash would.
    let (identity_a, mut daemon_a) = owned_build_daemon(&state, &manifest_a);
    assert_ne!(
        identity_a,
        production_identity(),
        "the manifest names the build"
    );

    let mut build_a = start(&manifest_a);
    assert_workspace(&build_a.view(2, None), &project);
    let mut build_b = start(&manifest_b);
    assert_workspace(&build_b.view(3, None), &project);
    let directories = daemon_directories(&state);
    assert_eq!(directories.len(), 2, "{directories:?}");
    assert!(directories.contains(&format!("daemon-p5-{identity_a}")));
    drop(build_b);

    // The daemon of build A is gone and the file it would restart from is
    // now another build.
    daemon_a.stop();
    write_core_manifest(&manifest_a, &"c".repeat(64));
    let refused = build_a.view(4, None);
    assert!(
        refused.to_string().contains("restart the host"),
        "{refused:#}"
    );
    assert_eq!(daemon_directories(&state).len(), 2);
}

/// The daemon of the build `manifest` names, owned by the test so it can be
/// stopped the way an idle exit or a crash would; returns its identity.
fn owned_build_daemon(state: &Path, manifest: &Path) -> (String, OwnedProcess) {
    let identity = {
        let output = Command::new(env!("CARGO_BIN_EXE_unica"))
            .arg("--print-core-identity")
            .env("UNICA_RUNTIME_MANIFEST", manifest)
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let daemon = OwnedProcess(
        Command::new(env!("CARGO_BIN_EXE_unica"))
            .args(["--daemon", "--state-root"])
            .arg(state)
            .args(["--core-identity", &identity, "--idle-grace-ms", "20000"])
            .env("UNICA_RUNTIME_MANIFEST", manifest)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let endpoint = state
        .join(format!("daemon-p5-{identity}"))
        .join("endpoint.json");
    let ready_until = Instant::now() + Duration::from_secs(10);
    while !endpoint.exists() {
        assert!(
            Instant::now() < ready_until,
            "owned build daemon did not start"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    (identity, daemon)
}

/// A build starts beside a live daemon of another build on the same state
/// root and over the spawn lock its own killed daemon left behind. The lock
/// is the operating system's: it ends with its holder, and the file left on
/// disk holds nothing (#3083).
#[test]
fn a_build_starts_beside_another_builds_daemon_and_a_left_spawn_lock() {
    let root = tempfile::tempdir().unwrap();
    let plugin = workspace(root.path(), "plugin");
    let project = workspace(root.path(), "project");
    let state = workspace(root.path(), "state");
    let fixture = contract();
    let project_environment = fixture["projectEnvironment"][0].as_str().unwrap();
    let manifest_a = root.path().join("build-a.json");
    let manifest_b = root.path().join("build-b.json");
    write_core_manifest(&manifest_a, &"a".repeat(64));
    write_core_manifest(&manifest_b, &"b".repeat(64));
    let start = || {
        Frontend::start(
            &plugin,
            &state,
            &[
                (project_environment, project.to_str().unwrap()),
                ("UNICA_RUNTIME_MANIFEST", manifest_b.to_str().unwrap()),
                ("UNICA_DAEMON_IDLE_GRACE_MS", "300"),
            ],
        )
    };
    let (_identity_a, _daemon_a) = owned_build_daemon(&state, &manifest_a);
    let (identity_b, mut daemon_b) = owned_build_daemon(&state, &manifest_b);
    // A frontend of build B takes the spawn lock of its state; then B's
    // daemon dies as a crash would, leaving the lock file and no endpoint.
    let mut first = start();
    assert_workspace(&first.view(2, None), &project);
    drop(first);
    daemon_b.stop();
    let state_b = state.join(format!("daemon-p5-{identity_b}"));
    // Windows locks this file and leaves it behind; elsewhere the lock is on
    // the directory, so a left file is placed here to the same effect.
    let left = state_b.join(".daemon-spawn.lock");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&left)
        .unwrap();
    assert!(left.is_file());
    let _ = std::fs::remove_file(state_b.join("endpoint.json"));

    let mut next = start();
    assert_workspace(&next.view(3, None), &project);
    assert_eq!(daemon_directories(&state).len(), 2);
}

fn root(path: &Path) -> String {
    url::Url::from_directory_path(path).unwrap().to_string()
}

/// Окружение проекта фиксируется при запуске frontend, а сессия
/// хоста может сменить каталог позже (а в worktree-сессии окружение
/// и вовсе называет основной checkout). Первый root клиента, запрошенный на
/// этот вызов, перекрывает устаревшее окружение; метаданные вызова — выше.
/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn client_roots_override_stale_startup_environment_on_every_call() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let stale = workspace(root_dir.path(), "старый проект");
    let current = workspace(root_dir.path(), "текущий проект");
    let moved = workspace(root_dir.path(), "новый каталог");
    let extra = workspace(root_dir.path(), "дополнительный");
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    // The variable the host captures at launch; the contract names it.
    let fixture = contract();
    let project_environment = fixture["projectEnvironment"][0].as_str().unwrap();
    let mut frontend = Frontend::start_with_roots(
        &plugin,
        &state,
        &[(project_environment, stale.to_str().unwrap())],
        Some(RootsAnswer::Roots(vec![root(&current), root(&extra)])),
    );

    assert_workspace(&frontend.view(2, None), &current);
    // The client changed its project: the next call follows without restart.
    frontend.roots = Some(RootsAnswer::Roots(vec![root(&moved)]));
    assert_workspace(&frontend.view(3, None), &moved);
    // Request metadata still outranks roots, and is not asked for them.
    let asked = frontend.roots_requests;
    assert_workspace(&frontend.view(4, Some(json!(root(&current)))), &current);
    assert_eq!(frontend.roots_requests, asked);
    // A supplied but malformed root is refused, never replaced by the env.
    frontend.roots = Some(RootsAnswer::Roots(vec![moved.to_str().unwrap().to_owned()]));
    assert_refusal(&frontend.view(5, None));
    // A failed exchange supplies nothing: the launch context still applies.
    frontend.roots = Some(RootsAnswer::Error);
    assert_workspace(&frontend.view(6, None), &stale);
    // No roots listed: likewise the launch context.
    frontend.roots = Some(RootsAnswer::Roots(Vec::new()));
    assert_workspace(&frontend.view(7, None), &stale);
    // A client that never answers costs a bounded wait, not the call.
    frontend.roots = Some(RootsAnswer::Silent);
    let started = Instant::now();
    assert_workspace(&frontend.view(8, None), &stale);
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );

    // Task controls own no workspace and do not ask for roots.
    frontend.roots = Some(RootsAnswer::Roots(vec![root(&current)]));
    let asked = frontend.roots_requests;
    frontend.send(
        9,
        "tools/call",
        json!({"name": "unica.task.get", "arguments": {"taskId": "unknown"}}),
    );
    frontend.receive(9);
    assert_eq!(frontend.roots_requests, asked);

    // Roots belong to one connection: a second frontend on the same daemon
    // keeps its own project between the first one's calls.
    let mut second = Frontend::start_with_roots(
        &plugin,
        &state,
        &[(project_environment, stale.to_str().unwrap())],
        Some(RootsAnswer::Roots(vec![root(&extra)])),
    );
    assert_workspace(&frontend.view(10, None), &current);
    assert_workspace(&second.view(11, None), &extra);
    assert_workspace(&frontend.view(12, None), &current);
}

fn origin(response: &Value) -> Value {
    let data = &response["result"]["structuredContent"]["data"];
    assert!(data["workspaceRoot"].is_string(), "{response:#}");
    data["workspaceRootOrigin"].clone()
}

/// Ответ называет канал, выбравший каталог, и сам запрошенный
/// каталог. Подсказка о переходе есть только у каналов, захваченных при
/// запуске: метаданные вызова и roots уже следуют за проектом сессии.
/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn view_names_the_channel_that_chose_the_workspace_and_hints_only_for_stale_ones() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let started = workspace(root_dir.path(), "проект запуска");
    let current = workspace(root_dir.path(), "проект сессии");
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let fixture = contract();
    let variable = fixture["projectEnvironment"][0].as_str().unwrap();
    let environment = [(variable, started.to_str().unwrap())];

    let mut plain = Frontend::start(&plugin, &state, &environment);
    let from_environment = origin(&plain.view(2, None));
    assert_eq!(from_environment["channel"], "startupEnvironment");
    assert_eq!(from_environment["variable"], variable);
    assert!(
        from_environment.get("roots").is_none(),
        "{from_environment:#}"
    );
    assert_eq!(
        Path::new(from_environment["requestedDirectory"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        started
    );
    assert!(
        from_environment["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains(variable)),
        "{from_environment:#}"
    );

    let from_metadata = origin(&plain.view(3, Some(json!(root(&current)))));
    assert_eq!(from_metadata["channel"], "requestMetadata");
    assert_eq!(from_metadata["capability"], fixture["metadataKey"]);
    assert!(from_metadata.get("hint").is_none(), "{from_metadata:#}");

    // Refusals that still name a root name its origin too: an invalid
    // project file, and a tool refused before any source set is admitted.
    let invalid = workspace(root_dir.path(), "битая настройка");
    std::fs::write(invalid.join("v8project.yaml"), "source-set: [").unwrap();
    let refused = plain.view(8, Some(json!(root(&invalid))));
    assert_eq!(refused["result"]["isError"], true, "{refused:#}");
    assert_eq!(
        refused["result"]["structuredContent"]["data"]["workspaceRootOrigin"]["channel"],
        "requestMetadata",
        "{refused:#}"
    );
    plain.send(
        9,
        "tools/call",
        json!({"name": "unica.search", "arguments": {"query": "Run"}}),
    );
    let unadmitted = plain.receive(9);
    assert_eq!(unadmitted["result"]["isError"], true, "{unadmitted:#}");
    let unadmitted_origin =
        &unadmitted["result"]["structuredContent"]["data"]["workspaceRootOrigin"];
    assert_eq!(
        unadmitted_origin["channel"], "startupEnvironment",
        "{unadmitted:#}"
    );
    assert!(unadmitted_origin["hint"].is_string(), "{unadmitted:#}");
    drop(plain);

    let mut with_roots = Frontend::start_with_roots(
        &plugin,
        &state,
        &environment,
        Some(RootsAnswer::Roots(vec![root(&current)])),
    );
    let from_roots = origin(&with_roots.view(4, None));
    assert_eq!(from_roots["channel"], "clientRoots");
    assert!(from_roots.get("hint").is_none(), "{from_roots:#}");
    assert_eq!(
        Path::new(from_roots["requestedDirectory"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        current
    );

    for (answer, fallback, id) in [
        (RootsAnswer::Roots(Vec::new()), "empty", 5),
        (RootsAnswer::Error, "error", 6),
        (RootsAnswer::Silent, "timeout", 7),
    ] {
        with_roots.roots = Some(answer);
        let fell_back = origin(&with_roots.view(id, None));
        assert_eq!(fell_back["channel"], "startupEnvironment", "{fell_back:#}");
        assert_eq!(fell_back["roots"], fallback, "{fell_back:#}");
        assert!(
            fell_back["hint"]
                .as_str()
                .is_some_and(|hint| hint.contains("roots")),
            "{fell_back:#}"
        );
    }
}

/// A copy of the acceptance workspace: one `main` configuration source set.
fn acceptance_copy(root: &Path, name: &str) -> std::path::PathBuf {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), &target).unwrap();
            }
        }
    }
    let target = root.join(name);
    copy(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/acceptance/workspace"),
        &target,
    );
    std::fs::canonicalize(target).unwrap()
}

/// A procedure only project B has, so a hit proves which tree was read.
fn mark_project_b(project: &Path) {
    let module = project.join("src/CommonModules/GoogleПереводчик/Ext/Module.bsl");
    let mut text = std::fs::read_to_string(&module).unwrap();
    text.push_str("\r\nПроцедура МаркерПроектаБ() Экспорт\r\nКонецПроцедуры\r\n");
    std::fs::write(module, text).unwrap();
}

fn structured(response: &Value) -> &Value {
    &response["result"]["structuredContent"]
}

fn same(path: &Value, expected: &Path) -> bool {
    path.as_str()
        .and_then(|path| Path::new(path).canonicalize().ok())
        .is_some_and(|path| path == expected.canonicalize().unwrap())
}

/// Хост открыл сессии две папки (проект и `--add-dir`): без выбора работает
/// проект, как раньше; `unica.view {}` перечисляет обе; выбор второй папки
/// читает её дерево, а курсор и токен плана одной папки в другой не действуют.
/// rule: INV.WORKSPACE.HOST-CONTEXT
/// rule: INV.APP.DEFERRED-READ
/// rule: INV.WIRE.APPLY-REQUIRES-THE-FENCE
#[test]
fn a_selection_chooses_among_client_roots_and_keeps_their_state_apart() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let project_a = acceptance_copy(root_dir.path(), "проект A");
    let project_b = acceptance_copy(root_dir.path(), "проект B");
    mark_project_b(&project_b);
    let foreign = workspace(root_dir.path(), "чужой C");
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start_with_roots(
        &plugin,
        &state,
        &[],
        Some(RootsAnswer::Roots(vec![root(&project_a), root(&project_b)])),
    );
    let b = project_b.to_str().unwrap();

    // 1, 13: no selection keeps the first root and lists the whole set.
    let listed = frontend.call(2, "unica.view", json!({}));
    assert_workspace(&listed, &project_a);
    let origin = &structured(&listed)["data"]["workspaceRootOrigin"];
    assert_eq!(origin["channel"], "clientRoots", "{origin:#}");
    assert!(origin.get("selected").is_none(), "{origin:#}");
    let available = origin["available"].as_array().expect("allowed set");
    assert_eq!(available.len(), 2, "{origin:#}");
    assert!(same(&available[0]["directory"], &project_a));
    assert!(same(&available[1]["directory"], &project_b));
    assert_eq!(available[1]["projectFile"], true);

    // 2: the selected root, named as an explicit choice.
    let chosen = frontend.call(3, "unica.view", json!({"workspace": b}));
    assert_workspace(&chosen, &project_b);
    let origin = &structured(&chosen)["data"]["workspaceRootOrigin"];
    assert_eq!(origin["selected"], true, "{origin:#}");
    assert!(same(&origin["root"], &project_b), "{origin:#}");
    assert!(origin.get("available").is_none(), "{origin:#}");

    // 3: the search reads the selected tree, and only with the selection.
    let found = frontend.call(
        4,
        "unica.search",
        json!({"query": "МаркерПроектаБ", "workspace": b}),
    );
    assert_eq!(structured(&found)["ok"], true, "{found:#}");
    assert!(found.to_string().contains("GoogleПереводчик"), "{found:#}");
    let unselected = frontend.call(5, "unica.search", json!({"query": "МаркерПроектаБ"}));
    assert!(
        !unselected.to_string().contains("GoogleПереводчик"),
        "{unselected:#}"
    );

    // 4: a view cursor of B continues in B, never in A, and A stays A.
    let at = "main:CommonModule.GoogleПереводчик.Method";
    let first = frontend.call(
        6,
        "unica.view",
        json!({"at": at, "limit": 1, "workspace": b}),
    );
    let cursor = structured(&first)["cursor"]
        .as_str()
        .unwrap_or_else(|| panic!("{first:#}"))
        .to_owned();
    let elsewhere = frontend.call(
        7,
        "unica.view",
        json!({"at": at, "limit": 1, "cursor": cursor}),
    );
    assert_eq!(
        structured(&elsewhere)["diagnostics"][0]["code"],
        "invalid_cursor",
        "{elsewhere:#}"
    );
    let continued = frontend.call(
        8,
        "unica.view",
        json!({"at": at, "limit": 1, "cursor": cursor, "workspace": b}),
    );
    assert_eq!(structured(&continued)["ok"], true, "{continued:#}");
    assert_workspace(&frontend.call(9, "unica.view", json!({})), &project_a);

    // 9: a plan saved in B does not execute in A.
    let planned = frontend.call(
        10,
        "unica.apply",
        json!({"workspace": b, "at": "main:Catalog.Валюты",
               "ops": [{"op": "props.set", "args": {"values": {"Comment": "из B"}}}]}),
    );
    let token = structured(&planned)["data"]["executionToken"]
        .as_str()
        .unwrap_or_else(|| panic!("{planned:#}"))
        .to_owned();
    let crossed = frontend.call(11, "unica.apply", json!({"executionToken": token}));
    assert!(
        structured(&crossed)["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("unavailable in this workspace actor")),
        "{crossed:#}"
    );
    let catalog_a = std::fs::read_to_string(project_a.join("src/Catalogs/Валюты.xml")).unwrap();
    assert!(!catalog_a.contains("из B"));

    // 6: outside the set: refused, nothing substituted.
    assert_refusal(&frontend.call(
        12,
        "unica.view",
        json!({"workspace": foreign.to_str().unwrap()}),
    ));

    // 11: the host opened another directory: the next call may select it.
    frontend.roots = Some(RootsAnswer::Roots(vec![
        root(&project_a),
        root(&project_b),
        root(&foreign),
    ]));
    assert_workspace(
        &frontend.call(
            13,
            "unica.view",
            json!({"workspace": foreign.to_str().unwrap()}),
        ),
        &foreign,
    );
}

/// Репозиторий с продуктами в подпапках: файл проекта у каждого свой, в корне
/// его нет. Выбор подпапки по относительному пути даёт её рабочую область;
/// файл проекта выше открытой папки выбор не подхватывает.
/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn a_repository_of_workspaces_selects_each_product_by_relative_path() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let repository = workspace(root_dir.path(), "репозиторий R");
    let smb = acceptance_copy(&repository, "smb");
    let acc = acceptance_copy(&repository, "acc");
    workspace(&repository, "docs");
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start_with_roots(
        &plugin,
        &state,
        &[],
        Some(RootsAnswer::Roots(vec![root(&repository)])),
    );

    let listed = frontend.call(2, "unica.view", json!({}));
    let available = &structured(&listed)["data"]["workspaceRootOrigin"]["available"];
    assert_eq!(available[0]["projectFile"], false, "{listed:#}");
    assert_eq!(
        available[0]["workspaces"],
        json!(["acc", "smb"]),
        "{listed:#}"
    );

    // 5: two products, two workspaces.
    let first = frontend.call(3, "unica.view", json!({"workspace": "smb"}));
    assert_workspace(&first, &smb);
    assert_ne!(
        structured(&first)["data"]["config"]["state"],
        "missing",
        "{first:#}"
    );
    assert_workspace(
        &frontend.call(4, "unica.view", json!({"workspace": "acc"})),
        &acc,
    );

    // 7: a project file above the open directory is not borrowed.
    std::fs::write(root_dir.path().join("v8project.yaml"), "format: DESIGNER\n").unwrap();
    let above = frontend.call(5, "unica.view", json!({"workspace": "docs"}));
    assert_refusal(&above);
    assert!(above.to_string().contains("lies above"), "{above:#}");
}

/// Метаданные вызова называют одну папку: выбор возможен только внутри неё.
/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn request_metadata_allows_only_its_own_directory() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let project_a = workspace(root_dir.path(), "проект A");
    let nested = workspace(&project_a, "вложенная");
    let project_b = workspace(root_dir.path(), "проект B");
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let mut frontend = Frontend::start(&plugin, &state, &[]);
    let fixture = contract();
    let mut call = |id: u64, selection: &Path| {
        let mut params = view_params(Some(json!(project_a.to_str().unwrap())));
        params["arguments"] = json!({"workspace": selection.to_str().unwrap()});
        frontend.send(id, "tools/call", params);
        frontend.receive(id)
    };
    // 12
    assert_refusal(&call(2, &project_b));
    let inner = call(3, &nested);
    assert_workspace(&inner, &nested);
    assert_eq!(
        structured(&inner)["data"]["workspaceRootOrigin"]["capability"],
        fixture["metadataKey"]
    );
}

/// Клиент 2026-07-28 отдаёт roots только по
/// запросу ввода: Unica спрашивает их, когда вызову нужно всё множество, —
/// при выборе и в `unica.view {}`, — и не тратит повтор на остальные вызовы.
/// rule: INV.WORKSPACE.HOST-CONTEXT
#[test]
fn a_modern_client_is_asked_for_roots_only_when_a_call_needs_the_set() {
    let root_dir = tempfile::tempdir().unwrap();
    let plugin = workspace(root_dir.path(), "plugin");
    let project_a = acceptance_copy(root_dir.path(), "проект A");
    let project_b = acceptance_copy(root_dir.path(), "проект B");
    mark_project_b(&project_b);
    let state = workspace(root_dir.path(), "state");
    let _daemon = spawn_owned_daemon(&state);
    let variable = contract()["projectEnvironment"][0]
        .as_str()
        .unwrap()
        .to_owned();
    let mut frontend = Frontend::start_modern(
        &plugin,
        &state,
        &[(variable.as_str(), project_a.to_str().unwrap())],
        Some(RootsAnswer::Roots(vec![root(&project_a), root(&project_b)])),
    );

    let listed = frontend.call(2, "unica.view", json!({}));
    assert_workspace(&listed, &project_a);
    assert_eq!(frontend.roots_requests, 1);
    assert_eq!(
        structured(&listed)["data"]["workspaceRootOrigin"]["available"]
            .as_array()
            .map(Vec::len),
        Some(2),
        "{listed:#}"
    );

    let b = project_b.to_str().unwrap();
    let found = frontend.call(
        3,
        "unica.search",
        json!({"query": "МаркерПроектаБ", "workspace": b}),
    );
    assert!(found.to_string().contains("GoogleПереводчик"), "{found:#}");
    assert_eq!(frontend.roots_requests, 2);

    // A call without a selection costs no retry and keeps the project.
    let unselected = frontend.call(4, "unica.search", json!({"query": "МаркерПроектаБ"}));
    assert_eq!(structured(&unselected)["ok"], true, "{unselected:#}");
    assert!(
        !unselected.to_string().contains("GoogleПереводчик"),
        "{unselected:#}"
    );
    assert_eq!(frontend.roots_requests, 2);
}
