use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const RESPONSE_DEADLINE: Duration = Duration::from_secs(15);

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn start(workspace: &std::path::Path) -> Self {
        let state = workspace.join(".unica-test-state");
        std::fs::create_dir_all(&state).expect("create isolated search state");
        let state = std::fs::canonicalize(state).expect("canonical isolated search state");
        let workspace = std::fs::canonicalize(workspace).expect("canonical search workspace");
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .current_dir(&workspace)
            .env("UNICA_PROVIDER_STATE_DIR", &state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start canonical Unica MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
        Self {
            child,
            stdin: Some(stdin),
            stdout,
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
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("read MCP response");
            assert!(!line.is_empty(), "MCP exited before response");
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
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        self.child.kill().expect("kill stalled MCP");
        self.child.wait().expect("reap stalled MCP");
        panic!("MCP did not stop after stdin EOF");
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn call_tool(id: u64, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
}

fn domain_result(response: &Value) -> Value {
    if response["result"]["structuredContent"].is_object() {
        return response["result"]["structuredContent"].clone();
    }
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing canonical tool result: {response:#}"));
    serde_json::from_str(text).expect("decode canonical DomainResult")
}

#[test]
#[ignore = "canonical search integration; routed by search_integration_changed or ci:full"]
fn canonical_search_is_source_scoped_and_rejects_legacy_call_shape() {
    let root = tempfile::tempdir().expect("search integration root");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext"))
        .expect("main module directory");
    std::fs::create_dir_all(workspace.join("src/extension/CommonModules/Extension/Ext"))
        .expect("extension module directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n  - name: extension\n    type: EXTENSION\n    path: src/extension\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("main configuration");
    std::fs::write(
        workspace.join("src/extension/Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Extension</Name></Properties><ChildObjects><CommonModule>Extension</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("extension configuration");
    std::fs::write(
        workspace.join("CommonModules/Main/Ext/Module.bsl"),
        "Procedure MainNeedle() Export\nEndProcedure\n",
    )
    .expect("main module");
    std::fs::write(
        workspace.join("CommonModules/Main.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Main</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("main module descriptor");
    std::fs::write(
        workspace.join("src/extension/CommonModules/Extension/Ext/Module.bsl"),
        "Procedure ExtensionNeedle() Export\nEndProcedure\n",
    )
    .expect("extension module");
    std::fs::write(
        workspace.join("src/extension/CommonModules/Extension.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="dddddddd-dddd-4ddd-8ddd-dddddddddddd"><Properties><Name>Extension</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("extension module descriptor");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "v13-search-ci", "version": "1"}
        }
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
        "params": {}
    }));

    let rejected = mcp.exchange(call_tool(
        2,
        "unica.search",
        json!({"query": "Needle", "dryRun": true}),
    ));
    assert_eq!(rejected["result"]["isError"], true, "{rejected:#}");
    assert_eq!(
        rejected["result"]["structuredContent"]["diagnostics"][0]["code"], "bad_value",
        "{rejected:#}"
    );

    let main = domain_result(&mcp.exchange(call_tool(
        3,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration"}),
    )));
    assert_eq!(main["ok"], true, "{main:#}");
    assert_eq!(main["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(main["data"]["matches"][0]["scope"], "main:Configuration");
    assert!(main["data"]["matches"][0].get("file").is_none());

    let extension = domain_result(&mcp.exchange(call_tool(
        4,
        "unica.search",
        json!({"query": "ExtensionNeedle", "scope": "extension:Configuration"}),
    )));
    assert_eq!(extension["ok"], true, "{extension:#}");
    assert_eq!(
        extension["data"]["matches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        extension["data"]["matches"][0]["scope"],
        "extension:Configuration"
    );

    let ping = mcp.exchange(json!({"jsonrpc": "2.0", "id": 5, "method": "ping"}));
    assert!(ping.get("result").is_some(), "{ping:#}");
    mcp.finish();
}
