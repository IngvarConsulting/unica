use serde_json::{json, Value};
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

    /// Starts the MCP in `workspace` exactly as given: a symlinked workspace
    /// path stays a symlink, the way a shell would hand it over.
    fn start_at(workspace: &std::path::Path, state: &std::path::Path) -> Self {
        let state = std::fs::canonicalize(state).expect("canonical bootstrap daemon state");
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .arg("mcp")
            .current_dir(workspace)
            .env("UNICA_PROVIDER_STATE_DIR", state)
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
    assert_eq!(source_attach["previewRequired"], true);
    assert_eq!(source_attach["ifRevRequiredOnApply"], true);
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
        std::collections::BTreeSet::from([
            "workspace.initialize",
            "infobase.configuration.export",
            "infobase.dump",
        ])
    );
    for operation in ["infobase.configuration.export", "infobase.dump"] {
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

#[test]
fn canonical_stdio_previews_and_applies_workspace_initialization_before_admission() {
    let root = tempfile::tempdir().expect("source attach integration root");
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
            "clientInfo": {"name": "v13-source-attach-ci", "version": "1"}
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
    assert!(bootstrap_result["next"]
        .as_array()
        .unwrap()
        .iter()
        .any(|next| {
            next["tool"] == "unica.run"
                && next["args"]["op"] == "workspace.initialize"
                && next["args"]["dryRun"] == true
        }));

    let missing_mode = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {"op": "workspace.initialize", "args": {}}
        }
    }));
    assert_eq!(missing_mode["result"]["structuredContent"]["ok"], false);
    assert_eq!(
        missing_mode["result"]["structuredContent"]["diagnostics"][0]["code"],
        "bad_value"
    );

    let preview = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {"op": "workspace.initialize", "args": {}, "dryRun": true}
        }
    }));
    let preview_result = &preview["result"]["structuredContent"];
    assert_eq!(preview_result["ok"], true, "{preview:#}");
    assert_eq!(preview_result["data"]["op"], "workspace.initialize");
    assert_eq!(preview_result["data"]["dryRun"], true);
    assert_eq!(preview_result["data"]["target"], "v8project.yaml");
    assert!(!workspace.join("v8project.yaml").exists());
    let rev = preview_result["rev"]
        .as_str()
        .expect("source attachment preview revision")
        .to_string();

    std::fs::create_dir_all(workspace.join("src/cfe/Late")).unwrap();
    std::fs::write(
        workspace.join("src/cfe/Late/Configuration.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();

    let stale = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {
                "op": "workspace.initialize",
                "args": {},
                "dryRun": false,
                "ifRev": rev
            }
        }
    }));
    let stale_result = &stale["result"]["structuredContent"];
    assert_eq!(stale_result["ok"], false, "{stale:#}");
    assert_eq!(stale_result["diagnostics"][0]["code"], "revision_mismatch");
    assert!(!workspace.join("v8project.yaml").exists());

    let refreshed = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {"op": "workspace.initialize", "args": {}, "dryRun": true}
        }
    }));
    let rev = refreshed["result"]["structuredContent"]["rev"]
        .as_str()
        .expect("refreshed source attachment revision")
        .to_string();

    let applied = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {
                "op": "workspace.initialize",
                "args": {},
                "dryRun": false,
                "ifRev": rev
            }
        }
    }));
    let applied_result = &applied["result"]["structuredContent"];
    assert_eq!(applied_result["ok"], true, "{applied:#}");
    assert_eq!(applied_result["data"]["dryRun"], false);
    assert_eq!(applied_result["changed"][0]["path"], "v8project.yaml");
    assert_eq!(applied_result["changed"][0]["kind"], "created");
    let config = std::fs::read_to_string(workspace.join("v8project.yaml")).unwrap();
    assert!(config.contains("format: DESIGNER"), "{config}");
    assert!(config.contains("path: src/cf"), "{config}");
    assert!(config.contains("path: src/cfe/Late"), "{config}");

    let overwrite = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {"op": "workspace.initialize", "args": {}, "dryRun": true}
        }
    }));
    assert_eq!(overwrite["result"]["structuredContent"]["ok"], false);
    assert_eq!(
        overwrite["result"]["structuredContent"]["diagnostics"][0]["code"],
        "invalid_state"
    );

    mcp.finish();
}

#[test]
fn canonical_stdio_previews_and_applies_autodetected_source_attachment_before_admission() {
    // Historical evidence retained for the superseded source.attach contract.
    canonical_stdio_previews_and_applies_workspace_initialization_before_admission();
}

#[test]
fn canonical_workspace_initialization_refuses_mixed_designer_and_edt_discovery() {
    let root = tempfile::tempdir().expect("mixed source attach integration root");
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
            "clientInfo": {"name": "v13-mixed-attach-ci", "version": "1"}
        }
    }));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let preview = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "unica.run",
            "arguments": {"op": "workspace.initialize", "args": {}, "dryRun": true}
        }
    }));
    let result = &preview["result"]["structuredContent"];
    assert_eq!(result["ok"], false, "{preview:#}");
    assert_eq!(result["diagnostics"][0]["code"], "ambiguous_source_format");
    assert!(!workspace.join("v8project.yaml").exists());

    mcp.finish();
}

/// The module outline the retired `unica.code.outline` answered lives in the
/// module node of `unica.view`: through a symlinked workspace it resolves the
/// methods of the module from the current file, and no BSL index state is
/// created for it.
#[cfg(unix)]
#[test]
fn canonical_stdio_views_a_module_through_a_symlinked_workspace() {
    let root = tempfile::tempdir().expect("symlinked workspace root");
    let real = root.path().join("real-workspace");
    let link = root.path().join("linked-workspace");
    let state = root.path().join("state");
    std::fs::create_dir_all(real.join("src/CommonModules/Demo/Ext")).unwrap();
    std::fs::create_dir(&state).unwrap();
    std::fs::write(
        real.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        real.join("src/Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Outline</Name></Properties><ChildObjects><CommonModule>Demo</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        real.join("src/CommonModules/Demo.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><CommonModule uuid="ac847dc9-e222-45cf-af4a-6fa863c919a8"><Properties><Name>Demo</Name><Synonym/><Comment/><Global>false</Global><ClientManagedApplication>false</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>true</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        real.join("src/CommonModules/Demo/Ext/Module.bsl"),
        "#Область Служебный\nПроцедура Служебная()\nКонецПроцедуры\n#КонецОбласти\n\nФункция Экспортная() Экспорт\n\tВозврат 1;\nКонецФункции\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let mut mcp = McpProcess::start_at(&link, &state);
    mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "v13-outline-ci", "version": "1"}
        }
    }));
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));
    let node = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {"at": "main:CommonModule.Demo"}}
    }));
    let result = &node["result"]["structuredContent"];
    assert_eq!(result["ok"], true, "{node:#}");
    assert_eq!(result["data"]["kind"], "Module");
    let branches = result["data"]["branches"].as_array().unwrap();
    let method_branch = branches
        .iter()
        .find(|branch| branch["at"] == "main:CommonModule.Demo.Method")
        .expect("the module node counts its methods");
    assert_eq!(method_branch["count"], 2);
    let methods = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {"name": "unica.view", "arguments": {"at": "main:CommonModule.Demo.Method"}}
    }));
    let items = methods["result"]["structuredContent"]["data"]["items"]
        .as_array()
        .unwrap();
    let exported = items
        .iter()
        .find(|item| item["title"] == "Экспортная")
        .expect("the exported function is listed");
    assert_eq!(exported["props"]["export"], true);
    assert_eq!(exported["props"]["methodKind"], "function");
    mcp.finish();
    assert!(
        !real.join(".build/unica/bsl_index").exists(),
        "viewing a module must not create BSL index state"
    );
}
