use serde_json::{json, Value};

#[path = "support/cfe_structure.rs"]
mod cfe_structure;
#[path = "support/code_module_state.rs"]
mod code_module_state;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const RESPONSE_DEADLINE: Duration = Duration::from_secs(15);

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Receiver<String>,
    stdout_reader: Option<JoinHandle<()>>,
}

impl McpProcess {
    fn start(workspace: &std::path::Path, state: &std::path::Path) -> Self {
        let workspace = std::fs::canonicalize(workspace).expect("canonical bootstrap workspace");
        Self::start_at(&workspace, state)
    }

    /// Starts the MCP with `workspace` as the child's working directory,
    /// without canonicalizing it first: a caller standing inside a symlinked
    /// workspace reaches the server the way a shell would hand it over. What
    /// the runtime then resolves the workspace to is its own business — the
    /// point here is only that the call succeeds through the link.
    fn start_at(workspace: &std::path::Path, state: &std::path::Path) -> Self {
        let state = std::fs::canonicalize(state).expect("canonical bootstrap daemon state");
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .arg("mcp")
            .current_dir(workspace)
            .env("UNICA_PROVIDER_STATE_DIR", state)
            // Демон переживает MCP: без назначенной паузы он остаётся на
            // четверть часа, и к концу прогона их набирается столько же,
            // сколько было тестов.
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start canonical Unica MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = child.stdout.take().expect("MCP stdout");
        let (line_sender, line_receiver) = mpsc::channel();
        let stdout_reader = std::thread::spawn(move || read_stdout_lines(stdout, line_sender));
        Self {
            child,
            stdin: Some(stdin),
            stdout: line_receiver,
            stdout_reader: Some(stdout_reader),
        }
    }

    fn exchange(&mut self, request: Value) -> Value {
        let id = request["id"].clone();
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &request).expect("encode MCP request");
        stdin.write_all(b"\n").expect("terminate MCP request");
        stdin.flush().expect("flush MCP request");
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            assert!(Instant::now() < deadline, "MCP response deadline elapsed");
            let remaining = deadline.saturating_duration_since(Instant::now());
            let line = match self.stdout.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => panic!("MCP response deadline elapsed"),
                Err(RecvTimeoutError::Disconnected) => panic!("MCP exited before response"),
            };
            let response: Value = serde_json::from_str(&line).expect("decode MCP response");
            if response.get("id") == Some(&id) {
                return response;
            }
        }
    }

    fn notify(&mut self, notification: Value) {
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &notification).expect("encode MCP notification");
        stdin.write_all(b"\n").expect("terminate MCP notification");
        stdin.flush().expect("flush MCP notification");
    }

    fn finish(&mut self) {
        drop(self.stdin.take());
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        while Instant::now() < deadline {
            if self.child.try_wait().expect("poll MCP exit").is_some() {
                self.join_stdout_reader();
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.child.kill().expect("kill stalled MCP");
        self.child.wait().expect("reap stalled MCP");
        self.join_stdout_reader();
        panic!("MCP did not stop after stdin EOF");
    }

    fn join_stdout_reader(&mut self) {
        if let Some(reader) = self.stdout_reader.take() {
            reader.join().expect("join MCP stdout reader");
        }
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        self.join_stdout_reader();
    }
}

fn read_stdout_lines(stdout: ChildStdout, sender: mpsc::Sender<String>) {
    let mut stdout = BufReader::new(stdout);
    loop {
        let mut line = String::new();
        match stdout.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if sender.send(line).is_err() => return,
            Ok(_) => {}
        }
    }
}

#[test]
fn canonical_check_null_options_preserve_root_node_and_cursor_contracts() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(workspace.join("src")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    // Readable configuration with two independent validation findings.
    let source = workspace.join("src/Configuration.xml");
    let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
<Configuration uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>Incomplete</Name><CompatibilityMode>Version8_3_27</CompatibilityMode></Properties>
<ChildObjects/></Configuration></MetaDataObject>"#;
    std::fs::write(&source, xml).unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);
    mcp.exchange(json!({"jsonrpc":"2.0", "id":1, "method":"initialize",
        "params":{"protocolVersion":"2025-11-25", "capabilities":{},
        "clientInfo":{"name":"check-options-test", "version":"1"}}}));
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized"}));
    let mut call = |arguments: Value| {
        let mut response = mcp.exchange(json!({"jsonrpc":"2.0", "id":2,
            "method":"tools/call", "params":{"name":"unica.check", "arguments":arguments}}));
        for id in 3..=12 {
            let result = &response["result"]["structuredContent"];
            let Some(task_id) = result["data"]["task"]["taskId"].as_str() else {
                assert!(result.is_object(), "{response:#}");
                return result.clone();
            };
            response = mcp.exchange(json!({"jsonrpc":"2.0", "id":id,
                "method":"tools/call", "params":{"name":"unica.task.result",
                "arguments":{"taskId":task_id,"waitMs":7000}}}));
        }
        panic!("check did not finish: {response:#}");
    };
    let root_omitted = call(json!({}));
    let root_null = call(json!({"at":null,"limit":null,"cursor":null}));
    assert_eq!(root_null["ok"], true, "{root_null:#}");
    assert_eq!(root_null["data"], root_omitted["data"]);
    let omitted = call(json!({"at":"main:Configuration"}));
    let nulls = call(json!({"at":"main:Configuration","limit":null,"cursor":null}));
    assert_eq!(nulls["ok"], true, "{nulls:#}");
    assert_eq!(nulls["data"], omitted["data"]);
    assert_eq!(nulls["data"]["status"], "failed");
    let diagnostics = nulls["data"]["diagnostics"].as_array().unwrap();
    assert!(diagnostics.len() > 1, "fixture must exercise pagination");
    let first = call(json!({"at":"main:Configuration","limit":1,"cursor":null}));
    assert_eq!(first["ok"], true, "{first:#}");
    let cursor = first["cursor"].as_str().expect("next page cursor");
    let next_args = json!({"at":"main:Configuration","limit":1,"cursor":cursor});
    let second = call(next_args.clone());
    assert_eq!(second["ok"], true, "{second:#}");
    assert_eq!(call(next_args), second, "cursor replay must be stable");
    let mut findings = first["data"]["diagnostics"].as_array().unwrap().clone();
    findings.extend(second["data"]["diagnostics"].as_array().unwrap().clone());
    assert_eq!(&findings, diagnostics);
    for page in [&first, &second] {
        assert_eq!(page["data"]["status"], "failed");
    }
    for arguments in [
        json!({"at":"main:Configuration","cursor":""}),
        json!({"at":"main:Configuration","cursor":"damaged"}),
        json!({"at":"main:Configuration","limit":2,"cursor":cursor}),
    ] {
        let rejected = call(arguments);
        assert_eq!(rejected["ok"], false, "{rejected:#}");
        assert_eq!(
            rejected["diagnostics"][0]["code"], "invalid_cursor",
            "{rejected:#}"
        );
    }
    for arguments in [
        json!({"at":false}),
        json!({"at":""}),
        json!({"at":"main:Configuration","limit":"20"}),
        json!({"at":"main:Configuration","limit":0}),
        json!({"at":"main:Configuration","limit":51}),
        json!({"at":"main:Configuration","cursor":false}),
        json!({"at":null,"unknown":null}),
    ] {
        let rejected = call(arguments);
        assert_eq!(rejected["ok"], false, "{rejected:#}");
        assert_eq!(
            rejected["diagnostics"][0]["code"], "bad_value",
            "{rejected:#}"
        );
    }
    assert_eq!(std::fs::read_to_string(source).unwrap(), xml);
    mcp.finish();
}

// The aggregate size is deliberately above the retired 256 MiB policy limit,
// while every staged and working resource stays below the per-file bound.
#[test]
fn canonical_check_has_no_aggregate_size_refusal_above_256_mib() {
    let root = tempfile::tempdir().expect("large repository integration root");
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(workspace.join("src/Corpus")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("src/Configuration.xml"),
        "<MetaDataObject/>\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join(".gitignore"),
        "**/.build/\nConfigDumpInfo.xml\nDumpFilesIndex.txt\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join(".gitattributes"),
        "*.xml text eol=lf\n*.bsl text eol=lf\n",
    )
    .unwrap();

    let first = workspace.join("src/Corpus/Module0.bsl");
    let mut file = std::fs::File::create(&first).unwrap();
    let chunk = [b'x'; 64 * 1024];
    for _ in 0..(31 * 1024 * 1024 / chunk.len()) {
        file.write_all(&chunk).unwrap();
    }
    drop(file);
    for index in 1..9 {
        let path = workspace.join(format!("src/Corpus/Module{index}.bsl"));
        if std::fs::hard_link(&first, &path).is_err() {
            std::fs::copy(&first, &path).unwrap();
        }
    }
    assert!(9 * std::fs::metadata(&first).unwrap().len() > 256 * 1024 * 1024);

    for arguments in [vec!["init"], vec!["add", "."]] {
        let output = Command::new("git")
            .args(&arguments)
            .current_dir(&workspace)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let mut mcp = McpProcess::start(&workspace, &state);
    mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "large-repository-check-ci", "version": "1"}
        }
    }));
    mcp.notify(json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));
    let check_started = Instant::now();
    let mut response = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "unica.check", "arguments": {}}
    }));
    for id in 3..=7 {
        let Some(task_id) =
            response["result"]["structuredContent"]["data"]["task"]["taskId"].as_str()
        else {
            break;
        };
        response = mcp.exchange(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {
                "name": "unica.task.result",
                "arguments": {"taskId": task_id, "waitMs": 7000}
            }
        }));
    }
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{response:#}");
    let data = &result["data"];
    if data["readinessState"] == "incomplete" {
        // A slow host may spend the request's seven-second inspection budget.
        // That is an honest incomplete result, unlike a hard aggregate cap.
        assert!(
            check_started.elapsed() >= Duration::from_secs(7),
            "inspection stopped before the request deadline: {response:#}"
        );
        assert_eq!(data["repositoryReady"], false, "{response:#}");
        assert!(
            data["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| {
                    diagnostic["code"] == "git.inspection_timeout"
                        || (diagnostic["code"] == "git.inspection_incomplete"
                            && diagnostic["evidence"].as_array().is_some_and(|items| {
                                items.iter().any(|item| {
                                    item.as_str().is_some_and(|text| {
                                        text.contains("deadline") || text.contains("timed out")
                                    })
                                })
                            }))
                }),
            "incomplete inspection must identify its deadline: {response:#}"
        );
    } else {
        assert_eq!(data["readinessState"], "complete", "{response:#}");
        assert_eq!(data["repositoryReady"], true, "{response:#}");
    }
    assert!(
        !serde_json::to_string(data)
            .unwrap()
            .contains("bytes in total"),
        "aggregate resource size must not be an admission criterion: {response:#}"
    );
    for check in [
        "repository.index_eol",
        "repository.working_eol",
        "repository.lfs",
    ] {
        assert!(
            data["checks"].as_array().unwrap().iter().any(|row| {
                row["id"] == check
                    && row["sourceSet"] == "main"
                    && (row["status"] == "passed"
                        || (data["readinessState"] == "incomplete" && row["status"] == "notRun"))
            }),
            "{check}: {response:#}"
        );
    }
    mcp.finish();
}

#[test]
#[ignore = "daemon tier: raises a daemon process; disabled on purpose until the tier is routed"]
fn canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery() {
    let root = tempfile::tempdir().expect("bootstrap integration root");
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&state).unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);

    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-bootstrap-ci", "version": "1"}
        }
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    assert!(initialized["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("unica.view using an empty object"));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let listed = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    }));
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 11);
    assert!(tools.iter().all(|tool| tool["description"]
        .as_str()
        .is_some_and(|description| !description.is_empty())));
    let view = tools
        .iter()
        .find(|tool| tool["name"] == "unica.view")
        .unwrap();
    assert!(!view["inputSchema"]["required"]
        .as_array()
        .is_some_and(|required| required.iter().any(|name| name == "at")));

    let response = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {}}
    }));
    let result = &response["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{response:#}");
    assert_eq!(
        result["summary"],
        "workspace is uninitialized; no v8project.yaml or 1C source roots were found"
    );
    assert_eq!(result["data"]["config"]["state"], "missing");
    assert_eq!(result["data"]["setup"]["path"], "v8project.yaml");
    assert_eq!(result["data"]["setup"]["content"], Value::Null);
    assert_eq!(result["data"]["checks"], json!([]));
    assert_eq!(result["data"]["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(
        result["data"]["diagnostics"][0]["code"],
        "source_roots_missing"
    );
    assert_eq!(
        result["next"],
        json!([{
            "tool": "unica.run",
            "args": {},
            "reason": "inspect the implemented and planned workspace initialization routes"
        }])
    );
    assert!(!serde_json::to_string(result)
        .unwrap()
        .contains("unica.project."));

    let dictionary = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {"name": "unica.run", "arguments": {}}
    }));
    let operations = dictionary["result"]["structuredContent"]["data"]["operations"]
        .as_array()
        .unwrap();
    let source_attach = operations
        .iter()
        .find(|operation| operation["op"] == "workspace.initialize")
        .unwrap();
    assert_eq!(source_attach["implemented"], true);
    assert_eq!(source_attach["execution"], "previewApply");
    assert_eq!(source_attach["effects"], json!(["workspaceFiles"]));
    assert!(source_attach["description"]
        .as_str()
        .is_some_and(|description| description.contains("v8project.yaml")));
    assert_eq!(source_attach["previewRequired"], false);
    assert_eq!(source_attach["dryRunRequired"], true);
    assert!(source_attach.get("ifRevRequiredOnApply").is_none());
    assert_eq!(
        source_attach["argsSchema"],
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {},
            "required": []
        })
    );
    assert!(operations
        .iter()
        .filter(|operation| operation["implemented"] == false)
        .all(|operation| operation["argsSchema"].is_null()));
    assert_eq!(
        operations
            .iter()
            .filter(|operation| operation["implemented"] == true)
            .map(|operation| operation["op"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["workspace.initialize", "cf.export", "infobase.export",])
    );
    for operation in ["cf.export", "infobase.export"] {
        assert!(operations
            .iter()
            .find(|candidate| candidate["op"] == operation)
            .unwrap()["argsSchema"]
            .is_object());
    }

    for (id, op, expected_summary) in [
        (
            41,
            "source.create",
            "canonical run operation `source.create` is not implemented yet",
        ),
        (42, "test.run", "unknown canonical run operation `test.run`"),
    ] {
        let unsupported = mcp.exchange(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": "unica.run", "arguments": {"op": op, "args": {}}}
        }));
        let unsupported = &unsupported["result"]["structuredContent"];
        assert_eq!(unsupported["ok"], false, "{unsupported:#}");
        assert_eq!(unsupported["summary"], expected_summary);
        assert_eq!(
            unsupported["diagnostics"],
            json!([{"code": "unsupported_operation", "message": expected_summary}])
        );
    }

    let invalid_dictionary = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {"name": "unica.run", "arguments": {"dryRun": true}}
    }));
    assert_eq!(
        invalid_dictionary["result"]["structuredContent"]["diagnostics"][0]["code"],
        "bad_value"
    );

    mcp.finish();
}

// Исключение из отключённого яруса: живой процесс — единственное место, где
// видно, что снятая операция снята и на проводе. Внутрипроцессной замены
// этому доказательству нет.
#[test]
fn canonical_stdio_hands_the_project_file_recipe_without_an_initialize_operation() {
    let root = tempfile::tempdir().expect("workspace recipe integration root");
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(workspace.join("src/cf")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        workspace.join("src/cf/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);

    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-workspace-recipe-ci", "version": "1"}
        }
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let bootstrap = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {}}
    }));
    let bootstrap_result = &bootstrap["result"]["structuredContent"];
    assert_eq!(bootstrap_result["data"]["config"]["state"], "autodetected");

    // Всё, что делала снятая операция, читатель ответа делает сам: содержимое
    // файла уже в ответе, и создаёт файл он своими файловыми средствами.
    let setup = &bootstrap_result["data"]["setup"];
    assert_eq!(setup["path"], "v8project.yaml", "{bootstrap_result:#}");
    let content = setup["content"]
        .as_str()
        .unwrap_or_else(|| panic!("recommended project file content: {bootstrap_result:#}"));
    assert!(content.contains("format: DESIGNER"), "{content}");
    assert!(content.contains("path: src/cf"), "{content}");

    // Подсказка-действие ушла вместе с операцией: ответ больше не называет
    // вызова, которым файл появится.
    assert!(
        !bootstrap_result["next"]
            .as_array()
            .unwrap()
            .iter()
            .any(|next| next["args"]["op"] == "workspace.initialize"),
        "{bootstrap_result:#}"
    );

    for arguments in [
        json!({"op": "workspace.initialize", "args": {}}),
        json!({"op": "workspace.initialize", "args": {}, "dryRun": true}),
    ] {
        let retired = mcp.exchange(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "unica.run", "arguments": arguments}
        }));
        let retired_result = &retired["result"]["structuredContent"];
        assert_eq!(retired_result["ok"], false, "{retired:#}");
        assert_eq!(
            retired_result["diagnostics"][0]["code"], "unsupported_operation",
            "{retired:#}"
        );
    }
    assert!(!workspace.join("v8project.yaml").exists());

    mcp.finish();
}

// Исключение из отключённого яруса: живой процесс — единственное место, где
// видно, как рекомендация ведёт себя на смешанных форматах.
#[test]
fn canonical_stdio_names_mixed_source_formats_instead_of_recommending_a_project_file() {
    let root = tempfile::tempdir().expect("mixed source recipe integration root");
    let workspace = root.path().join("workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(workspace.join("src/cf")).unwrap();
    std::fs::create_dir_all(workspace.join("src/cfe/Edt/Configuration")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        workspace.join("src/cf/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    std::fs::write(
        workspace.join("src/cfe/Edt/Configuration/Configuration.mdo"),
        "",
    )
    .unwrap();
    let mut mcp = McpProcess::start(&workspace, &state);

    mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-mixed-recipe-ci", "version": "1"}
        }
    }));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let bootstrap = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {}}
    }));
    let setup = &bootstrap["result"]["structuredContent"]["data"]["setup"];
    // Один формат на смешанных наборах выбрать нельзя, и ответ говорит об
    // этом словами, а не молчанием: содержимого нет, причина названа.
    assert_eq!(setup["path"], "v8project.yaml", "{bootstrap:#}");
    assert_eq!(setup["content"], serde_json::Value::Null, "{bootstrap:#}");
    let reason = setup["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("known format"), "{bootstrap:#}");
    assert!(!workspace.join("v8project.yaml").exists());

    mcp.finish();
}

// The symlink evidence for the canonical module view lives under `platform/`:
// the OS-specific link call belongs to a platform facade path.
include!("platform/v13_canonical_symlinked_workspace.rs");
