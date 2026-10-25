use serde_json::{json, Value};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "platform/daemon_teardown.rs"]
mod daemon_teardown;
#[path = "platform/v13_search_unreadable.rs"]
mod unreadable;

const RESPONSE_DEADLINE: Duration = Duration::from_secs(15);

struct McpProcess {
    child: Child,
    state: std::path::PathBuf,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl McpProcess {
    fn start(workspace: &std::path::Path) -> Self {
        Self::start_with(workspace, &[])
    }

    fn start_with(workspace: &std::path::Path, environment: &[(&str, &str)]) -> Self {
        let state = workspace.join(".unica-test-state");
        std::fs::create_dir_all(&state).expect("create isolated search state");
        let state = std::fs::canonicalize(state).expect("canonical isolated search state");
        let workspace = std::fs::canonicalize(workspace).expect("canonical search workspace");
        let mut command = Command::new(env!("CARGO_BIN_EXE_unica"));
        // Бюджет ответа задаёт только сама проверка, не окружение прогона.
        for key in unica_bootstrap::response_budget_environment_keys() {
            command.env_remove(key);
        }
        let mut child = command
            .current_dir(&workspace)
            .env("UNICA_PROVIDER_STATE_DIR", &state)
            // Демон переживает MCP: без назначенной паузы он остаётся на
            // четверть часа, и к концу прогона их набирается столько же,
            // сколько было тестов.
            .env("UNICA_DAEMON_IDLE_GRACE_MS", "5000")
            .envs(environment.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start canonical Unica MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = BufReader::new(child.stdout.take().expect("MCP stdout"));
        Self {
            child,
            state,
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

    fn completed_tool_call(&mut self, request: Value) -> Value {
        completed_tool_result(request, |request| self.exchange(request))
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
        daemon_teardown::stop_daemons_under(&self.state);
    }
}

fn call_tool(id: impl Into<Value>, name: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.into(),
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

fn completed_tool_result(request: Value, mut exchange: impl FnMut(Value) -> Value) -> Value {
    let first_id = request["id"].as_u64().expect("numeric request id");
    let mut result = domain_result(&exchange(request));
    let mut task_id: Option<String> = None;
    // После окна прямого ответа читаем то же задание. Повтор subject tool
    // создал бы новый вызов; отсутствие конечного ответа ограничено 10 ожиданиями.
    for poll in 0..=10 {
        let task = &result["data"]["task"];
        if result["ok"] != true || task.is_null() {
            return result;
        }
        assert!(
            matches!(
                task["status"].as_str(),
                Some("queued" | "working" | "completed")
            ),
            "unexpected task state: {result:#}"
        );
        let current_id = task["taskId"].as_str().expect("task receipt has an id");
        if let Some(expected) = &task_id {
            assert_eq!(current_id, expected, "task identity changed: {result:#}");
        } else {
            task_id = Some(current_id.to_string());
        }
        assert!(poll < 10, "task did not finish: {result:#}");
        result = domain_result(&exchange(call_tool(
            format!("task-result:{first_id}:{poll}"),
            "unica.task.result",
            json!({"taskId":current_id,"waitMs":7000}),
        )));
    }
    unreachable!("unfinished task fails at the polling bound")
}

#[test]
fn completed_tool_result_observes_the_same_task_without_resubmitting_view() {
    let task_id = "00000000-0000-4000-8000-000000000001";
    let request = call_tool(
        2,
        "unica.view",
        json!({"at":"main:CommonModule.Main.Body","limit":50}),
    );
    let terminal = json!({"ok":true,"data":{"items":[{"line":1,"text":"// body"}]}});
    let mut replies = [
        json!({"ok":true,"data":{"task":{"taskId":task_id,"status":"queued"}}}),
        json!({"ok":true,"data":{"task":{"taskId":task_id,"status":"working"}}}),
        terminal.clone(),
        terminal.clone(),
    ]
    .into_iter();
    let mut calls = Vec::new();
    let mut exchange = |request| {
        calls.push(request);
        json!({"result":{"structuredContent":replies.next().expect("no extra calls")}})
    };
    let result = completed_tool_result(request.clone(), &mut exchange);
    assert_eq!(result, terminal);
    let next_page = call_tool(
        3,
        "unica.view",
        json!({"at":"main:CommonModule.Main.Body","cursor":"page-2"}),
    );
    assert_eq!(
        completed_tool_result(next_page.clone(), &mut exchange),
        terminal
    );
    let ids: std::collections::BTreeSet<_> =
        calls.iter().map(|call| call["id"].to_string()).collect();
    assert_eq!(
        ids.len(),
        calls.len(),
        "request IDs must remain unique between pages"
    );
    assert_eq!(
        calls,
        vec![
            request,
            call_tool(
                "task-result:2:0",
                "unica.task.result",
                json!({"taskId":task_id,"waitMs":7000})
            ),
            call_tool(
                "task-result:2:1",
                "unica.task.result",
                json!({"taskId":task_id,"waitMs":7000})
            ),
            next_page,
        ]
    );
    assert!(replies.next().is_none());
}

#[test]
fn completed_tool_result_returns_direct_and_failed_results_without_retry() {
    for terminal in [
        json!({"ok":true,"data":{"items":[]}}),
        json!({"ok":false,"data":{"task":{"status":"failed"}},"diagnostics":[{"code":"task_failed"}]}),
        json!({"ok":false,"data":{"task":{"status":"cancelled"}},"diagnostics":[{"code":"task_cancelled"}]}),
    ] {
        let mut calls = 0;
        let result = completed_tool_result(call_tool(2, "unica.view", json!({})), |_| {
            calls += 1;
            json!({"result":{"structuredContent":terminal}})
        });
        assert_eq!(result, terminal);
        assert_eq!(calls, 1, "terminal result must not be retried");
    }
}

#[test]
#[should_panic(expected = "task identity changed")]
fn completed_tool_result_rejects_a_different_task() {
    let mut id = 0;
    completed_tool_result(call_tool(2, "unica.view", json!({})), |_| {
        id += 1;
        json!({"result":{"structuredContent":{"ok":true,"data":{"task":{
            "taskId":format!("00000000-0000-4000-8000-{id:012}"),"status":"working"
        }}}}})
    });
}

#[test]
#[should_panic(expected = "task did not finish")]
fn completed_tool_result_bounds_observation_of_an_unfinished_task() {
    let mut calls = 0;
    completed_tool_result(call_tool(2, "unica.view", json!({})), |_| {
        calls += 1;
        assert!(calls <= 11, "polling exceeded its bound");
        json!({"result":{"structuredContent":{"ok":true,"data":{"task":{
            "taskId":"00000000-0000-4000-8000-000000000001","status":"working"
        }}}}})
    });
}

#[test]
fn canonical_view_reads_configuration_past_eight_mebibytes_with_many_registrations() {
    let root = tempfile::tempdir().expect("large configuration workspace");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::create_dir_all(workspace.join("CommonModules")).expect("metadata directory");
    for index in 0..32 {
        let name = format!("Module{index:02}");
        std::fs::write(
            workspace.join(format!("CommonModules/{name}.xml")),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><CommonModule><Properties><Name>{name}</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"
            ),
        )
        .expect("metadata descriptor");
    }
    for name in ["Module00", "Module31", "ModuleNew"] {
        std::fs::create_dir_all(workspace.join(format!("CommonModules/{name}/Ext")))
            .expect("module source directory");
        std::fs::write(
            workspace.join(format!("CommonModules/{name}/Ext/Module.bsl")),
            "Процедура Проверка() Экспорт\nКонецПроцедуры\n",
        )
        .expect("module source");
    }
    std::fs::write(
        workspace.join("CommonModules/ModuleNew.xml"),
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><CommonModule><Properties><Name>ModuleNew</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>",
    )
    .expect("future metadata descriptor");
    let path = workspace.join("Configuration.xml");
    let mut xml = BufWriter::new(std::fs::File::create(&path).expect("configuration document"));
    xml.write_all(b"<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration><Properties><Name>First")
        .unwrap();
    xml.write_all(b"</Name></Properties><ChildObjects>")
        .unwrap();
    for index in 0..300_000 {
        write!(xml, "<CommonModule>Module{:02}</CommonModule>", index % 32).unwrap();
    }
    xml.write_all(b"</ChildObjects></Configuration></MetaDataObject>")
        .unwrap();
    xml.flush().unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > 8 * 1024 * 1024);

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-large-configuration", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}}));
    let view = mcp.completed_tool_call(call_tool(
        2,
        "unica.view",
        json!({"at": "main:Configuration"}),
    ));
    assert_eq!(view["ok"], true, "{view:#}");
    assert_eq!(view["data"]["props"]["name"], "First");
    assert_eq!(view["data"]["props"]["totalObjects"], 300_000);
    let branch = view["data"]["branches"]
        .as_array()
        .and_then(|branches| {
            branches
                .iter()
                .find(|branch| branch["at"] == "main:CommonModule")
        })
        .expect("registered CommonModule branch");
    assert_eq!(branch["count"], 32);
    let collection = mcp.completed_tool_call(call_tool(
        6,
        "unica.view",
        json!({"at": "main:CommonModule", "limit": 50}),
    ));
    assert_eq!(collection["ok"], true, "{collection:#}");
    let items = collection["data"]["items"]
        .as_array()
        .expect("branch items");
    assert_eq!(items.len(), 32);
    assert_eq!(items.first().unwrap()["at"], "main:CommonModule.Module00");
    assert_eq!(items.last().unwrap()["at"], "main:CommonModule.Module31");
    let named = mcp.completed_tool_call(call_tool(
        3,
        "unica.view",
        json!({"at": "main:CommonModule.Module00"}),
    ));
    assert_eq!(named["ok"], true, "{named:#}");
    assert_eq!(named["data"]["at"], "main:CommonModule.Module00");
    let other = mcp.completed_tool_call(call_tool(
        4,
        "unica.view",
        json!({"at": "main:CommonModule.Module31"}),
    ));
    assert_eq!(other["ok"], true, "{other:#}");
    assert!(
        other.get("rev").is_none(),
        "view must not expose a source revision"
    );
    let source = std::fs::read_to_string(&path).expect("read large root for revision change");
    let source = source.replacen(
        "<CommonModule>Module31</CommonModule>",
        "<CommonModule>ModuleNew</CommonModule>",
        1,
    );
    std::fs::write(&path, source).expect("publish changed root registration");
    let changed = mcp.completed_tool_call(call_tool(
        5,
        "unica.view",
        json!({"at": "main:CommonModule.ModuleNew"}),
    ));
    assert_eq!(changed["ok"], true, "{changed:#}");
    assert!(changed.get("rev").is_none());
    let changed_collection = mcp.completed_tool_call(call_tool(
        7,
        "unica.view",
        json!({"at": "main:CommonModule", "limit": 50}),
    ));
    assert_eq!(changed_collection["ok"], true, "{changed_collection:#}");
    assert!(
        changed_collection.get("rev").is_none(),
        "view collection must not expose a source revision: {changed_collection:#}"
    );
    assert_eq!(
        changed_collection["data"]["items"].as_array().map(Vec::len),
        Some(33)
    );
    assert!(changed_collection["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["at"] == "main:CommonModule.ModuleNew"));
    mcp.finish();
}

fn body_workspace(workspace: &std::path::Path, module_bytes: &[u8]) {
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext")).unwrap();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .unwrap();
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    ).unwrap();
    std::fs::write(
        workspace.join("CommonModules/Main.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Main</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    ).unwrap();
    std::fs::write(
        workspace.join("CommonModules/Main/Ext/Module.bsl"),
        module_bytes,
    )
    .unwrap();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(workspace)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(workspace)
        .status()
        .unwrap()
        .success());
}

/// A text-search file identifier: stable, and neither a path nor an address.
fn is_file_id(value: &Value) -> bool {
    value.as_str().is_some_and(|id| {
        id.len() == 20
            && id.starts_with("bsl-")
            && id[4..]
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
}

fn initialize_body_mcp(mcp: &mut McpProcess) {
    initialize_body_mcp_as(mcp, "body-large-test");
}

fn initialize_body_mcp_as(mcp: &mut McpProcess, client: &str) {
    let response = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": client, "version": "1"}}
    }));
    assert_eq!(response["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));
}

#[test]
fn public_body_line_ranges_open_search_hits_and_refuse_line_edits() {
    let root = tempfile::tempdir().unwrap();
    body_workspace(root.path(), "\u{feff}// outside\r\n\r\nProcedure Ping() Export\r\n    Message(\"Needle\");\r\nEndProcedure".as_bytes());
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let found = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query":"Needle","scope":"main:CommonModule.Main"}),
    ));
    assert_eq!(found["ok"], true, "{found:#}");
    let at = found["data"]["matches"][0]["at"].as_str().unwrap();
    assert_eq!(at, "main:CommonModule.Main.Method.Ping.Body#L4");
    let response = mcp.exchange(call_tool(3, "unica.view", json!({"at":at})));
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{response:#}"
    );
    assert_eq!(
        response["result"]["content"][1]["text"],
        "    Message(\"Needle\");\r\n"
    );
    let envelope: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        envelope["data"]["props"]["lines"],
        json!({"state":"range","from":4,"to":4})
    );
    assert_eq!(envelope["data"]["can"], json!([]));
    assert!(!response.to_string().contains("sourceTextReadHint"));
    let resolved = mcp.completed_tool_call(call_tool(4, "unica.resolve", json!({"at":at})));
    assert_eq!(resolved["ok"], true, "{resolved:#}");
    assert_eq!(
        resolved["data"]["lines"],
        json!({"state":"range","from":4,"to":4})
    );
    let refused = mcp.completed_tool_call(call_tool(
        5,
        "unica.apply",
        json!({"at":at,"ops":[{"op":"code.replace","args":{"text":"    Return;"}}]}),
    ));
    assert_eq!(refused["ok"], false, "{refused:#}");
    assert_eq!(refused["diagnostics"][0]["code"], "bad_value");
    assert_eq!(
        refused["next"][0]["args"]["at"],
        "main:CommonModule.Main.Method.Ping"
    );
    let plan = mcp.completed_tool_call(call_tool(6,"unica.apply",json!({"at":"main:CommonModule.Main.Method.Ping","ops":[{"op":"code.replace","args":{"text":"Procedure Ping() Export\r\n    Message(\"Changed\");\r\nEndProcedure"}}]})));
    assert_eq!(plan["ok"], true, "{plan:#}");
    let applied = mcp.completed_tool_call(call_tool(
        7,
        "unica.apply",
        json!({"executionToken":plan["data"]["executionToken"]}),
    ));
    assert_eq!(applied["ok"], true, "{applied:#}");
    let after = mcp.exchange(call_tool(8, "unica.view", json!({"at":at})));
    assert_eq!(
        after["result"]["content"][1]["text"],
        "    Message(\"Changed\");\r\n"
    );
    mcp.finish();
}

#[test]
fn oversized_error_recovery_is_a_closed_size_refusal_without_poisoning_the_daemon() {
    let root = tempfile::tempdir().unwrap();
    body_workspace(root.path(), b"Procedure Small() Export\nEndProcedure\n");
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    // The retired filter route echoes at in next before address validation.
    // Canonical JSON fits; duplicating complete recovery does not.
    let mut response = mcp.exchange(call_tool(
        2,
        "unica.view",
        json!({
            "at": "x".repeat(4_300_000), "filter": {}
        }),
    ));
    for poll in 0..=10 {
        if response["error"].is_object() {
            assert_eq!(
                response["error"]["data"]["code"], "result_too_large",
                "{response:#}"
            );
            break;
        }
        let result = domain_result(&response);
        if result["ok"] == false {
            assert_eq!(
                result["diagnostics"][0]["code"], "result_too_large",
                "{result:#}"
            );
            break;
        }
        assert!(poll < 10, "oversized refusal did not terminate: {result:#}");
        let task_id = result["data"]["task"]["taskId"]
            .as_str()
            .expect("pending Task");
        response = mcp.exchange(call_tool(
            format!("large-recovery:{poll}"),
            "unica.task.result",
            json!({"taskId":task_id,"waitMs":7000}),
        ));
    }
    let next = mcp.completed_tool_call(call_tool(
        3,
        "unica.view",
        json!({"at":"main:CommonModule.Main"}),
    ));
    assert_eq!(next["ok"], true, "daemon remains usable: {next:#}");
    mcp.finish();
}

#[test]
fn public_view_refuses_bsl_text_over_the_default_budget_whole_with_structural_next() {
    let root = tempfile::tempdir().unwrap();
    let mut source = String::from(
        "// Документирует малый метод.\nProcedure Small() Export\n    Value = 1;\nEndProcedure\n\nProcedure Large()\n",
    );
    // Метод сверх бюджета клиента по умолчанию (16 КиБ), а с ним и модуль.
    while source.len() <= 17 * 1024 {
        source.push_str("    // заполнение строки кириллицей\n");
    }
    source.push_str("EndProcedure\n");
    body_workspace(root.path(), source.as_bytes());
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let module = "main:CommonModule.Main";
    let next_targets = |result: &Value| {
        result["next"]
            .as_array()
            .unwrap()
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    let mut recovery_was_visible = false;
    let body = completed_tool_result(
        call_tool(2, "unica.view", json!({"at": format!("{module}.Body")})),
        |request| {
            let response = mcp.exchange(request);
            let result = &response["result"];
            if result["isError"] == true {
                let recovery: Value = serde_json::from_str(
                    result["content"][1]["text"]
                        .as_str()
                        .expect("refusal exposes recovery to text-only hosts"),
                )
                .expect("recovery is complete JSON");
                assert_eq!(recovery["next"], result["structuredContent"]["next"]);
                recovery_was_visible = true;
            }
            response
        },
    );
    assert!(recovery_was_visible);
    assert_eq!(body["ok"], false, "{body:#}");
    assert_eq!(
        body["diagnostics"][0]["code"], "result_too_large",
        "{body:#}"
    );
    assert!(body.get("data").is_none(), "no partial text: {body:#}");
    assert!(body.get("cursor").is_none(), "{body:#}");
    assert_eq!(
        next_targets(&body)[1..],
        [format!("{module}.Region"), format!("{module}.Method")]
    );

    let first = body["next"][0]["args"]["at"].as_str().unwrap();
    assert!(
        first.starts_with(&format!("{module}.Body#L1-L")),
        "{body:#}"
    );
    let chunk = mcp.exchange(call_tool(90, "unica.view", json!({"at":first})));
    assert_eq!(chunk["result"]["isError"], false, "{chunk:#}");
    let raw = chunk["result"]["content"][1]["text"].as_str().unwrap();
    assert!(source.starts_with(raw));
    assert!(raw.ends_with('\n'));
    assert!(!chunk.to_string().contains("sourceTextReadHint"));
    let end = first.rsplit_once("-L").unwrap().1.parse::<usize>().unwrap();
    let one_more = mcp.completed_tool_call(call_tool(
        91,
        "unica.view",
        json!({"at":format!("{module}.Body#L1-L{}",end+1)}),
    ));
    assert_eq!(one_more["ok"], false, "{one_more:#}");
    assert_eq!(one_more["diagnostics"][0]["code"], "result_too_large");

    // Прямой ответ на проводе: конверт JSON первым блоком, сырой текст вторым,
    // без `structuredContent`.
    let small = mcp.exchange(call_tool(
        3,
        "unica.view",
        json!({"at": format!("{module}.Method.Small.Body")}),
    ));
    assert!(
        small["result"].get("structuredContent").is_none(),
        "{small:#}"
    );
    assert_eq!(small["result"]["isError"], false, "{small:#}");
    let envelope: Value =
        serde_json::from_str(small["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["ok"], true, "{envelope:#}");
    assert_eq!(
        envelope["data"]["at"],
        format!("{module}.Method.Small.Body")
    );
    assert_eq!(envelope["data"]["kind"], "Body");
    assert!(envelope["data"].get("items").is_none());
    assert_eq!(
        small["result"]["content"][1]["text"],
        "// Документирует малый метод.\nProcedure Small() Export\n    Value = 1;\nEndProcedure\n"
    );

    let large = mcp.completed_tool_call(call_tool(
        4,
        "unica.view",
        json!({"at": format!("{module}.Method.Large.Body")}),
    ));
    assert_eq!(large["ok"], false, "{large:#}");
    assert_eq!(large["diagnostics"][0]["code"], "result_too_large");
    assert!(large.get("data").is_none(), "{large:#}");
    assert_eq!(
        next_targets(&large)[1..],
        [
            format!("{module}.Method.Large"),
            format!("{module}.Region"),
            format!("{module}.Method")
        ]
    );

    // Контракт операции того же узла текста не несёт и предела не касается.
    for (id, at, op) in [
        (5, format!("{module}.Body"), "code.insert"),
        (6, format!("{module}.Method.Large.Body"), "code.replace"),
    ] {
        let contract = mcp.completed_tool_call(call_tool(
            id,
            "unica.view",
            json!({"at": format!("{at}.Can.{op}")}),
        ));
        // Подробного контракта у операций кода пока нет: ответ — тот же
        // отказ `unsupported_section`, что и у малого модуля, а не предел.
        assert_eq!(contract["at"], format!("{at}.Can.{op}"), "{contract:#}");
        assert_ne!(
            contract["diagnostics"][0]["code"], "result_too_large",
            "{contract:#}"
        );
        assert!(contract.get("sourceText").is_none(), "{contract:#}");
    }
    mcp.finish();
}

/// Бюджет ответа — свойство клиента: тот же текст один клиент получает
/// целиком, другой — отказом; сводка модуля называет размер `Body` и
/// помечает ветвь, не помещающуюся в бюджет. Наибольший бюджет известных
/// хостов ограничивает текст для всех.
#[test]
fn public_view_budget_belongs_to_the_client_and_the_summary_marks_the_body() {
    let root = tempfile::tempdir().unwrap();
    let mut method = String::from("Procedure Large()\n");
    while method.len() <= 30 * 1024 {
        method.push_str("    // заполнение строки кириллицей\n");
    }
    method.push_str("EndProcedure\n");
    let mut source = method.clone();
    while source.len() <= 80 * 1024 {
        source.push_str("// хвост модуля сверх наибольшего бюджета\n");
    }
    body_workspace(root.path(), source.as_bytes());
    let module = "main:CommonModule.Main";
    let read = |mcp: &mut McpProcess, id: u64, at: String| {
        mcp.exchange(call_tool(id, "unica.view", json!({"at": at})))
    };

    let mut narrow = McpProcess::start(root.path());
    initialize_body_mcp(&mut narrow);
    let summary = narrow.completed_tool_call(call_tool(2, "unica.view", json!({"at": module})));
    let body = summary["data"]["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|branch| branch["at"] == format!("{module}.Body"))
        .unwrap_or_else(|| panic!("{summary:#}"))
        .clone();
    assert_eq!(body["bytes"], source.len(), "{summary:#}");
    assert_eq!(body["overBudget"], true, "{summary:#}");
    let refused = narrow.completed_tool_call(call_tool(
        3,
        "unica.view",
        json!({"at": format!("{module}.Method.Large.Body")}),
    ));
    assert_eq!(
        refused["diagnostics"][0]["code"], "result_too_large",
        "{refused:#}"
    );
    assert_eq!(
        refused["next"][1]["args"]["at"],
        format!("{module}.Method.Large")
    );
    narrow.finish();

    // Известный клиент получает весь текст по своему имени из `initialize`,
    // без настройки; явная настройка даёт то же любому клиенту.
    let mut wide = None;
    for (client, budget) in unica_bootstrap::known_client_budgets() {
        assert!(budget > method.len() + 2048, "{client}");
        let mut known = McpProcess::start(root.path());
        initialize_body_mcp_as(&mut known, client);
        let answer = read(&mut known, 2, format!("{module}.Method.Large.Body"));
        assert!(
            answer["result"].get("structuredContent").is_none(),
            "{client}: {answer:#}"
        );
        assert_eq!(answer["result"]["content"][1]["text"], method, "{client}");
        // Настоящий конверт меньше запаса на него, которым решаются отказ
        // и пометка `overBudget` (2 КиБ).
        let envelope = answer["result"]["content"][0]["text"].as_str().unwrap();
        assert!(envelope.len() < 2048, "{client}: {envelope}");
        match wide {
            None => wide = Some(known),
            Some(_) => known.finish(),
        }
    }
    let mut wide = wide.expect("known clients");
    let mut explicit = McpProcess::start_with(root.path(), &[("UNICA_RESPONSE_BUDGET", "47104")]);
    initialize_body_mcp(&mut explicit);
    let answer = read(&mut explicit, 2, format!("{module}.Method.Large.Body"));
    assert_eq!(answer["result"]["content"][1]["text"], method);
    explicit.finish();
    // Модуль больше наибольшего бюджета известных хостов не отдаётся никому.
    let whole = wide.completed_tool_call(call_tool(
        3,
        "unica.view",
        json!({"at": format!("{module}.Body")}),
    ));
    assert_eq!(
        whole["diagnostics"][0]["code"], "result_too_large",
        "{whole:#}"
    );
    assert!(!whole.to_string().contains("sourceTextReadHint"));
    let first = whole["next"][0]["args"]["at"].as_str().unwrap();
    assert!(
        first.starts_with(&format!("{module}.Body#L1-L")),
        "{whole:#}"
    );
    let reopened = read(&mut wide, 4, first.to_owned());
    assert_eq!(reopened["result"]["isError"], false, "{reopened:#}");
    let prefix = reopened["result"]["content"][1]["text"].as_str().unwrap();
    assert!(source.starts_with(prefix));
    assert!(prefix.ends_with('\n'));
    assert!(prefix.len() < source.len());
    assert!(!reopened.to_string().contains("sourceTextReadHint"));
    wide.finish();
}

#[path = "platform/v13_command_lifecycle.rs"]
mod command_lifecycle;
#[path = "platform/v13_metadata_members.rs"]
mod metadata_members;
#[path = "platform/v13_resolve_resources.rs"]
mod resolve_resources;
#[path = "platform/v13_support_policy_warning.rs"]
mod support_policy_warning;
#[path = "platform/v13_resolve_target_isolation.rs"]
mod target_isolation;

// Интеграционная цель — `medium` по `kind(test)`: идёт в очереди и на main,
// на pull request не идёт. Отдельной джобы и выключателя больше нет.
#[test]
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
        format!(
            "Procedure MainNeedle() Export\nEndProcedure\n{}",
            (0..21)
                .map(|index| format!("// MainNeedle {index}\n"))
                .collect::<String>()
        ),
    )
    .expect("main module");
    let long_synonym = "Я".repeat(1_025);
    std::fs::write(
        workspace.join("CommonModules/Main.xml"),
        format!(r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><CommonModule uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Main</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>{long_synonym}</v8:content></v8:item></Synonym><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#),
    )
    .expect("main module descriptor");
    std::fs::create_dir_all(workspace.join("CommonModules/Orphan/Ext"))
        .expect("unregistered module directory");
    std::fs::write(
        workspace.join("CommonModules/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"><Properties><Name>Orphan</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("unregistered module descriptor");
    std::fs::write(
        workspace.join("CommonModules/Orphan/Ext/Module.bsl"),
        "// OrphanNeedle\n",
    )
    .expect("unregistered module source");
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

    // Source admission, not Git tracking, owns the BSL corpus. Both source
    // sets are ignored; unrelated ignored text and generated BSL remain out.
    std::fs::write(workspace.join(".gitignore"), "*.bsl\n*.txt\n.build/\n")
        .expect("ignore source exports and fixture private files");

    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(workspace)
        .status()
        .expect("initialize search fixture repository")
        .success());
    assert!(Command::new("git")
        .args(["add", "."])
        .current_dir(workspace)
        .status()
        .expect("index search fixture sources")
        .success());
    for source in [
        "CommonModules/Main/Ext/Module.bsl",
        "src/extension/CommonModules/Extension/Ext/Module.bsl",
    ] {
        assert!(Command::new("git")
            .args(["check-ignore", "--quiet", source])
            .current_dir(workspace)
            .status()
            .expect("verify ignored source fixture")
            .success());
    }

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

    let main = mcp.completed_tool_call(call_tool(
        3,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration"}),
    ));
    assert_eq!(main["ok"], true, "{main:#}");
    assert!(
        main.get("rev").is_none(),
        "search must not expose a source revision"
    );
    assert_eq!(main["data"]["dataFreshness"], "unknown");
    assert_eq!(main["data"]["pageConsistency"], "live");
    assert_eq!(main["data"]["matches"].as_array().map(Vec::len), Some(20));
    assert_eq!(main["data"]["matches"][0]["scope"], "main:Configuration");
    assert!(main["data"]["matches"][0].get("file").is_none());
    assert_eq!(main["page"]["stoppedBy"], "limit");
    let cursor = main["cursor"].as_str().expect("search continuation");
    let remaining = mcp.completed_tool_call(call_tool(
        5,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(remaining["ok"], true, "{remaining:#}");
    assert_eq!(
        remaining["data"]["matches"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(remaining["page"]["stoppedBy"], "complete");
    assert!(remaining.get("cursor").is_none());
    let replay = mcp.completed_tool_call(call_tool(
        6,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(
        replay, remaining,
        "retry of one cursor must replay its page"
    );
    let wrong_question = mcp.completed_tool_call(call_tool(
        7,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(wrong_question["diagnostics"][0]["code"], "invalid_cursor");

    let extension = mcp.completed_tool_call(call_tool(
        4,
        "unica.search",
        json!({"query": "ExtensionNeedle", "scope": "extension:Configuration"}),
    ));
    assert_eq!(extension["ok"], true, "{extension:#}");
    assert_eq!(
        extension["data"]["matches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        extension["data"]["matches"][0]["scope"],
        "extension:Configuration"
    );

    std::fs::write(workspace.join("credentials.txt"), "MainNeedle\n")
        .expect("ignored non-BSL text");
    std::fs::create_dir_all(workspace.join(".build")).expect("generated cache directory");
    std::fs::write(workspace.join(".build/Module.bsl"), "// MainNeedle\n")
        .expect("generated cache text");
    std::fs::write(
        workspace.join("src/extension/CommonModules/Extension/Ext/Module.bsl"),
        "Procedure ExtensionNeedle() Export\nEndProcedure\n// MainNeedle\n",
    )
    .expect("nested source-set scope trap for the lexical provider");

    // The workspace is a Git work tree: the literal search reads it through
    // Git, with the same scope boundaries as the walk.
    let module_page = |id: u64, cursor: Option<&str>| {
        let mut arguments =
            json!({"query": "MainNeedle", "scope": "main:CommonModule.Main", "limit": 7});
        if let Some(cursor) = cursor {
            arguments["cursor"] = json!(cursor);
        }
        call_tool(id, "unica.search", arguments)
    };
    let mut module_lines = Vec::new();
    let mut module_cursor = None::<String>;
    let mut module_pages = 0;
    loop {
        let page =
            mcp.completed_tool_call(module_page(30 + module_pages, module_cursor.as_deref()));
        assert_eq!(page["ok"], true, "{page:#}");
        let hits = page["data"]["matches"].as_array().unwrap();
        assert!(hits.len() <= 7);
        assert!(hits.iter().all(|hit| hit["at"]
            .as_str()
            .unwrap()
            .starts_with("main:CommonModule.Main")));
        module_lines.extend(hits.iter().map(|hit| hit["line"].as_u64().unwrap()));
        module_pages += 1;
        match page["cursor"].as_str() {
            Some(next) => module_cursor = Some(next.to_string()),
            None => {
                assert_eq!(page["page"]["stoppedBy"], "complete");
                assert_eq!(page["data"]["fileCoverage"]["scanComplete"], true);
                break;
            }
        }
    }
    assert_eq!(module_pages, 4);
    assert_eq!(
        module_lines,
        std::iter::once(1).chain(3..=23).collect::<Vec<_>>()
    );
    let extension_module = mcp.completed_tool_call(call_tool(
        37,
        "unica.search",
        json!({"query":"ExtensionNeedle","corpus":"text","scope":"extension:CommonModule.Extension"}),
    ));
    assert_eq!(extension_module["ok"], true, "{extension_module:#}");
    assert_eq!(
        extension_module["data"]["matches"].as_array().map(Vec::len),
        Some(1),
        "the ignored extension is searched: {extension_module:#}"
    );
    let main_root = mcp.completed_tool_call(call_tool(
        38,
        "unica.search",
        json!({"query":"MainNeedle","corpus":"text","scope":"main:Configuration","limit":50}),
    ));
    assert_eq!(main_root["ok"], true, "{main_root:#}");
    assert_eq!(
        main_root["data"]["matches"].as_array().map(Vec::len),
        Some(22),
        "main excludes the nested extension, generated BSL and private text: {main_root:#}"
    );
    assert_eq!(main_root["data"]["fileCoverage"]["complete"], true);
    let all_sources = mcp.completed_tool_call(call_tool(
        39,
        "unica.search",
        json!({"query":"ExtensionNeedle","corpus":"text","limit":50}),
    ));
    assert_eq!(all_sources["ok"], true, "{all_sources:#}");
    let all_hits = all_sources["data"]["matches"]
        .as_array()
        .expect("unscoped text results");
    assert_eq!(
        all_hits.len(),
        1,
        "nested sources are searched once: {all_sources:#}"
    );
    assert_eq!(all_hits[0]["scope"], "extension:Configuration");
    let cross_mode = mcp.completed_tool_call(call_tool(
        35,
        "unica.search",
        json!({"query": "MainNeedle", "ignoreCase": true, "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(cross_mode["diagnostics"][0]["code"], "invalid_cursor");

    let main_names = mcp.completed_tool_call(call_tool(
        19,
        "unica.search",
        json!({"query": "CommonModule", "corpus": "names", "scope": "main:Configuration"}),
    ));
    assert_eq!(main_names["ok"], true, "{main_names:#}");
    let main_matches = main_names["data"]["matches"]
        .as_array()
        .expect("name matches");
    assert!(!main_matches.is_empty());
    assert!(main_matches.iter().all(|item| item["at"]
        .as_str()
        .is_some_and(|at| at.starts_with("main:"))));

    let long_name_result = mcp.completed_tool_call(call_tool(
        36,
        "unica.search",
        json!({"query": long_synonym, "corpus": "names", "scope": "main:Configuration"}),
    ));
    assert_eq!(long_name_result["ok"], true, "{long_name_result:#}");
    assert_eq!(
        long_name_result["data"]["matches"][0]["at"],
        "main:CommonModule.Main"
    );

    let named_scope = mcp.completed_tool_call(call_tool(
        20,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Main"}),
    ));
    assert_eq!(named_scope["ok"], true, "{named_scope:#}");
    assert_eq!(
        named_scope["data"]["matches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        named_scope["data"]["matches"][0]["at"],
        "main:CommonModule.Main"
    );

    let missing_scope = mcp.completed_tool_call(call_tool(
        21,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Missing"}),
    ));
    assert_eq!(missing_scope["diagnostics"][0]["code"], "not_found");
    let unsupported_scope = mcp.completed_tool_call(call_tool(
        22,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Main.Attribute.Missing"}),
    ));
    assert_eq!(
        unsupported_scope["diagnostics"][0]["code"],
        "unsupported_scope"
    );
    let unsupported_root = mcp.completed_tool_call(call_tool(
        23,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:Form"}),
    ));
    assert_eq!(
        unsupported_root["diagnostics"][0]["code"],
        "unsupported_scope"
    );

    let absent_scope = mcp.completed_tool_call(call_tool(
        16,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:CommonModule.Missing"}),
    ));
    assert_eq!(absent_scope["diagnostics"][0]["code"], "not_found");
    let orphan_scope = mcp.completed_tool_call(call_tool(
        17,
        "unica.search",
        json!({"query": "OrphanNeedle", "scope": "main:CommonModule.Orphan"}),
    ));
    assert_eq!(orphan_scope["diagnostics"][0]["code"], "not_found");
    let branch = mcp.completed_tool_call(call_tool(
        18,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:CommonModule"}),
    ));
    assert_eq!(branch["ok"], true, "{branch:#}");
    assert_eq!(branch["data"]["matches"].as_array().map(Vec::len), Some(20));
    assert!(branch["cursor"].as_str().is_some());

    let broad = mcp.completed_tool_call(call_tool(8, "unica.search", json!({"query": "Needle"})));
    assert_eq!(broad["page"]["stoppedBy"], "limit");
    let broad_cursor = broad["cursor"].as_str().expect("multi-source cursor");
    // The extension is admitted first: the first page delivers its matches
    // and the main module's head; the orphan module comes after the cursor.
    std::fs::write(
        workspace.join("CommonModules/Orphan/Ext/Module.bsl"),
        "// changed source\n",
    )
    .expect("change a source not reached by the first page");
    let stale = mcp.completed_tool_call(call_tool(
        9,
        "unica.search",
        json!({"query": "Needle", "cursor": broad_cursor}),
    ));
    assert_eq!(stale["ok"], true, "{stale:#}");
    assert_eq!(stale["data"]["dataFreshness"], "unknown");
    assert_eq!(stale["data"]["pageConsistency"], "live");

    let cross_corpus = mcp.completed_tool_call(call_tool(
        10,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "corpus": "names", "cursor": cursor}),
    ));
    assert_eq!(cross_corpus["diagnostics"][0]["code"], "invalid_cursor");

    for (id, arguments) in [
        (12, json!({"query": "Needle", "limit": 51})),
        (
            25,
            json!({"query": "Main", "corpus": "names", "role": "lexical"}),
        ),
        (
            26,
            json!({"query": "Needle", "role": "lexical", "scope": 7}),
        ),
        (
            27,
            json!({"query": "Needle", "role": "lexical", "limit": 0}),
        ),
    ] {
        let refused = mcp.completed_tool_call(call_tool(id, "unica.search", arguments));
        assert_eq!(
            refused["diagnostics"][0]["code"], "bad_value",
            "{refused:#}"
        );
    }

    let ping = mcp.exchange(json!({"jsonrpc": "2.0", "id": 15, "method": "ping"}));
    assert!(ping.get("result").is_some(), "{ping:#}");
    mcp.finish();
}

#[test]
fn a_long_single_line_can_be_read_across_more_than_one_hundred_search_pages() {
    let root = tempfile::tempdir().expect("long search root");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext")).expect("module directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    std::fs::write(
        workspace.join("CommonModules/Main/Ext/Module.bsl"),
        "Needle ".repeat(2_041),
    )
    .expect("long source line");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-long", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let started = Instant::now();
    let mut cursor = None::<String>;
    let mut total = 0;
    let mut pages = 0;
    loop {
        let mut arguments = json!({"query": "Needle"});
        if let Some(token) = cursor.as_ref() {
            arguments["cursor"] = Value::String(token.clone());
        }
        let result = mcp.completed_tool_call(call_tool(2 + pages, "unica.search", arguments));
        assert_eq!(result["ok"], true, "{result:#}");
        let matches = result["data"]["matches"].as_array().expect("matches");
        assert!(!matches.is_empty());
        assert!(matches.iter().all(|item| item["line"] == 1));
        total += matches.len();
        pages += 1;
        cursor = result["cursor"].as_str().map(ToOwned::to_owned);
        if cursor.is_none() {
            assert_eq!(result["page"]["stoppedBy"], "complete");
            break;
        }
        assert_eq!(result["page"]["stoppedBy"], "limit");
    }
    assert_eq!(total, 2_041);
    assert_eq!(pages, 103);
    eprintln!("103 search pages over one line: {:?}", started.elapsed());
    mcp.finish();
}

#[test]
fn text_search_pages_matches_after_two_mebibytes_of_source() {
    let root = tempfile::tempdir().expect("large search root");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext")).expect("module directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Main</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    let mut source = format!("// {}\r\n", "x".repeat(96)).repeat(24_000);
    source.push_str(&"// яNeedle\r\n".repeat(51));
    assert!(source.len() > 2 * 1024 * 1024);
    std::fs::write(workspace.join("CommonModules/Main/Ext/Module.bsl"), source)
        .expect("large module");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-large", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let first = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "limit": 50}),
    ));
    assert_eq!(first["ok"], true, "{first:#}");
    let matches = first["data"]["matches"].as_array().expect("first page");
    assert_eq!(matches.len(), 50);
    assert_eq!(matches[0]["line"], 24_001);
    assert_eq!(matches[0]["column"], 5);
    assert_eq!(matches[0]["snippet"], "// яNeedle");
    assert!(matches.iter().all(|hit| hit.get("file").is_none()));
    let cursor = first["cursor"].as_str().expect("remaining match");
    let second = mcp.completed_tool_call(call_tool(
        3,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "limit": 50, "cursor": cursor}),
    ));
    assert_eq!(second["ok"], true, "{second:#}");
    assert_eq!(second["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["data"]["matches"][0]["line"], 24_051);
    assert_eq!(second["page"]["stoppedBy"], "complete");

    let anchored = mcp.completed_tool_call(call_tool(
        4,
        "unica.search",
        json!({"query": "^// яNeedle$", "regex": true, "scope": "main:Configuration"}),
    ));
    assert_eq!(anchored["ok"], true, "{anchored:#}");
    assert_eq!(
        anchored["data"]["matches"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(anchored["data"]["matches"][0]["column"], 1);
    mcp.finish();
}

#[test]
fn text_search_reads_more_than_thirty_two_mebibytes_across_sources() {
    let root = tempfile::tempdir().expect("large corpus root");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    let source_line = format!("// {}\n", "x".repeat(65_532));
    assert_eq!(source_line.len(), 64 * 1024);
    let source_body = source_line.repeat(16);
    for index in 0..33 {
        let mut source = source_body.clone();
        if index == 32 {
            source.push_str("// LastNeedle\n");
        }
        std::fs::write(workspace.join(format!("Module{index:02}.bsl")), source)
            .expect("corpus module");
    }

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-large-corpus", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "LastNeedle", "scope": "main:Configuration"}),
    ));
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(result["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["data"]["matches"][0]["line"], 17);
    assert_eq!(result["page"]["stoppedBy"], "complete");
    mcp.finish();
}

#[test]
fn text_search_preserves_whole_line_regex_beyond_two_mebibytes() {
    let root = tempfile::tempdir().expect("long regex line root");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    std::fs::write(
        workspace.join("SessionModule.bsl"),
        format!("// {}Needle\r\n", "x".repeat(2 * 1024 * 1024 + 1)),
    )
    .expect("single long source line");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-long-regex", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "^// x+Needle$", "regex": true, "scope": "main:Configuration"}),
    ));
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(result["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["data"]["matches"][0]["line"], 1);
    assert_eq!(result["data"]["matches"][0]["column"], 1);
    assert_eq!(result["page"]["stoppedBy"], "complete");
    mcp.finish();
}

#[test]
fn unreadable_source_lines_keep_other_hits_and_truthful_coverage_across_pages() {
    let root = tempfile::tempdir().expect("overlong search root");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    std::fs::write(workspace.join("A.bsl"), "// Needle\n".repeat(21))
        .expect("first source with a continuation");
    std::fs::write(
        workspace.join("B.bsl"),
        format!("// {}Needle\n", "x".repeat(8 * 1024 * 1024)),
    )
    .expect("overlong source line");
    std::fs::write(workspace.join("C.bsl"), b"// Needle\n\xff Needle")
        .expect("source with an invalid UTF-8 line that may hold a match");
    std::fs::write(workspace.join("D.bsl"), "// Needle\n".repeat(21))
        .expect("source after the uncovered files");
    std::fs::write(workspace.join("E.bsl"), "// Needle\n")
        .expect("last source after the intermediate page");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-overlong", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let first = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration"}),
    ));
    assert_eq!(first["ok"], true, "{first:#}");
    assert_eq!(first["page"]["stoppedBy"], "limit");
    assert_eq!(first["data"]["fileCoverage"]["complete"], false);
    assert_eq!(first["data"]["fileCoverage"]["scanComplete"], false);
    let cursor = first["cursor"].as_str().expect("unread part of corpus");
    let partial = mcp.completed_tool_call(call_tool(
        3,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(partial["ok"], true, "{partial:#}");
    assert!(partial["summary"].as_str().unwrap().contains("partial"));
    assert_eq!(
        partial["data"]["matches"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(partial["data"]["fileCoverage"]["complete"], false);
    assert_eq!(partial["data"]["fileCoverage"]["scanComplete"], false);
    assert_eq!(partial["data"]["fileCoverage"]["uncovered"], 2);
    assert_eq!(
        partial["data"]["fileCoverage"]["uncoveredIsLowerBound"],
        true
    );
    assert_eq!(partial["data"]["fileCoverage"]["detailsTruncated"], false);
    let details = partial["data"]["fileCoverage"]["details"]
        .as_array()
        .expect("uncovered files");
    assert_eq!(details.len(), 2);
    assert_eq!(details[0]["sourceSet"], "main");
    assert_eq!(details[0]["reason"], "line_too_long");
    assert_eq!(details[1]["reason"], "invalid_utf8");
    assert_ne!(details[0]["fileId"], details[1]["fileId"]);
    assert!(details.iter().all(|detail| is_file_id(&detail["fileId"])));
    assert_eq!(partial["page"]["stoppedBy"], "limit");
    let partial_cursor = partial["cursor"].as_str().expect("remaining hits");
    assert!(!partial.to_string().contains("B.bsl"));
    assert!(!partial.to_string().contains("C.bsl"));
    assert!(!partial
        .to_string()
        .contains(&workspace.display().to_string()));
    let replay = mcp.completed_tool_call(call_tool(
        4,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "cursor": cursor}),
    ));
    assert_eq!(
        replay, partial,
        "cursor replay preserves omissions and hits"
    );
    let terminal = mcp.completed_tool_call(call_tool(
        5,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "cursor": partial_cursor}),
    ));
    assert_eq!(terminal["ok"], true, "{terminal:#}");
    assert!(terminal["summary"].as_str().unwrap().contains("partial"));
    assert_eq!(
        terminal["data"]["matches"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(terminal["data"]["fileCoverage"]["scanComplete"], true);
    assert_eq!(terminal["data"]["fileCoverage"]["complete"], false);
    assert_eq!(terminal["data"]["fileCoverage"]["uncovered"], 2);
    assert_eq!(
        terminal["data"]["fileCoverage"]["uncoveredIsLowerBound"],
        false
    );
    // The count carries the files earlier pages passed; the details name
    // only what this page read.
    assert_eq!(terminal["data"]["fileCoverage"]["details"], json!([]));
    assert_eq!(terminal["data"]["fileCoverage"]["detailsTruncated"], true);
    assert_eq!(terminal["page"]["stoppedBy"], "complete");
    assert!(terminal.get("cursor").is_none());
    mcp.finish();
}

#[test]
fn text_file_coverage_counts_all_uncovered_files_when_details_are_capped() {
    let root = tempfile::tempdir().expect("uncovered search root");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    for index in 0..33 {
        std::fs::write(workspace.join(format!("Bad{index:02}.bsl")), b"\xffNeedle")
            .expect("undecodable BSL source line that may hold a match");
    }
    std::fs::write(workspace.join("Good.bsl"), "// Needle\n").expect("readable BSL source");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-uncovered", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration"}),
    ));
    assert_eq!(result["ok"], true, "{result:#}");
    assert_eq!(result["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["data"]["fileCoverage"]["scanComplete"], true);
    assert_eq!(result["data"]["fileCoverage"]["complete"], false);
    assert_eq!(result["data"]["fileCoverage"]["uncovered"], 33);
    assert_eq!(
        result["data"]["fileCoverage"]["uncoveredIsLowerBound"],
        false
    );
    assert_eq!(result["data"]["fileCoverage"]["detailsTruncated"], true);
    let details = result["data"]["fileCoverage"]["details"]
        .as_array()
        .expect("bounded uncovered details");
    assert_eq!(details.len(), 32);
    assert!(details.iter().all(|detail| is_file_id(&detail["fileId"])));
    assert_eq!(
        details
            .iter()
            .map(|detail| detail["fileId"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        32
    );
    assert!(details
        .iter()
        .all(|detail| detail["reason"] == "invalid_utf8"));
    assert!(!result.to_string().contains("Bad00.bsl"));
    assert_eq!(result["page"]["stoppedBy"], "complete");
    mcp.finish();
}

#[test]
fn text_search_refuses_unvisited_depth_instead_of_claiming_complete_coverage() {
    let root = tempfile::tempdir().expect("deep search root");
    let workspace = root.path();
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    std::fs::write(
        workspace.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Main</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .expect("configuration");
    let mut directory = workspace.to_path_buf();
    for index in 0..33 {
        directory.push(format!("Nested{index:02}"));
        std::fs::create_dir(&directory).expect("nested source directory");
    }
    std::fs::write(directory.join("Module.bsl"), "// Needle\n")
        .expect("source beyond the retained depth budget");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-depth", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let result = mcp.completed_tool_call(call_tool(
        2,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration"}),
    ));
    assert_eq!(result["ok"], false, "{result:#}");
    assert_eq!(result["diagnostics"][0]["code"], "provider_unavailable");
    assert!(result["summary"].as_str().unwrap().contains("depth limit"));
    assert!(result.get("data").is_none());
    assert!(result.get("cursor").is_none());
    assert!(!result.to_string().contains("Nested32"));
    mcp.finish();
}

#[test]
fn scoped_text_search_does_not_need_a_projected_view_of_the_root() {
    let root = tempfile::tempdir().expect("independent text search root");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("CommonModules/Main/Ext")).expect("module directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    // The text corpus is still inspectable when a typed projection cannot
    // parse the configuration descriptor. A `view` preflight would hide it.
    std::fs::write(workspace.join("Configuration.xml"), "<incomplete")
        .expect("unprojectable descriptor");
    std::fs::write(
        workspace.join("CommonModules/Main/Ext/Module.bsl"),
        "// Needle\n",
    )
    .expect("module source");

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-search-independent", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({"jsonrpc":"2.0", "method":"notifications/initialized", "params":{}}));

    let view = mcp.completed_tool_call(call_tool(
        2,
        "unica.view",
        json!({"at": "main:Configuration"}),
    ));
    assert_eq!(view["ok"], false, "typed root must be unreadable: {view:#}");
    let search = mcp.completed_tool_call(call_tool(
        3,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration"}),
    ));
    assert_eq!(search["ok"], true, "{search:#}");
    assert_eq!(search["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(search["page"]["stoppedBy"], "complete");
    std::fs::remove_file(workspace.join("Configuration.xml"))
        .expect("remove the branch owner descriptor");
    let missing_root = mcp.completed_tool_call(call_tool(
        4,
        "unica.search",
        json!({"query": "Needle", "scope": "main:CommonModule"}),
    ));
    assert_eq!(missing_root["diagnostics"][0]["code"], "invalid_state");
    mcp.finish();
}

#[test]
fn names_search_pages_all_ranked_matches_and_rejects_changed_answers() {
    let root = tempfile::tempdir().expect("names pages workspace");
    let workspace = root.path();
    std::fs::create_dir_all(workspace.join("Catalogs")).expect("catalog collection");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: .\n",
    )
    .expect("workspace manifest");
    let children = (0..121)
        .map(|index| format!("<Catalog>Item{index:03}</Catalog>"))
        .collect::<String>();
    std::fs::write(
        workspace.join("Configuration.xml"),
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Root</Name></Properties><ChildObjects>{children}</ChildObjects></Configuration></MetaDataObject>"#
        ),
    )
    .expect("configuration descriptor");
    for index in 0..121 {
        std::fs::write(
            workspace.join(format!("Catalogs/Item{index:03}.xml")),
            format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Item{index:03}</Name></Properties></Catalog></MetaDataObject>"#),
        )
        .expect("catalog descriptor");
    }

    let mut mcp = McpProcess::start(workspace);
    let initialized = mcp.exchange(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "v13-names-pages", "version": "1"}}
    }));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "unica");
    mcp.notify(json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));

    let mut cursor: Option<String> = None;
    let mut first_cursor = String::new();
    let mut seen = Vec::new();
    for page_index in 0..16 {
        let mut arguments = json!({
            "query": "Item", "corpus": "names", "kind": "Catalog", "limit": 17
        });
        if let Some(token) = &cursor {
            arguments["cursor"] = json!(token);
        }
        let page = mcp.completed_tool_call(call_tool(10 + page_index, "unica.search", arguments));
        assert_eq!(page["ok"], true, "{page:#}");
        assert_eq!(page["data"]["approximate"], false);
        assert_eq!(page["data"]["sourceCoverage"]["complete"], true);
        assert!(page["rev"].is_null(), "names do not claim a revision");
        let matches = page["data"]["matches"].as_array().expect("name matches");
        assert!(!matches.is_empty(), "{page:#}");
        assert!(matches.len() <= 17);
        assert!(matches.iter().all(|item| item.get("file").is_none()));
        seen.extend(
            matches
                .iter()
                .map(|item| item["at"].as_str().unwrap().to_owned()),
        );
        cursor = page["cursor"].as_str().map(str::to_owned);
        if page_index == 0 {
            first_cursor = cursor
                .clone()
                .unwrap_or_else(|| panic!("first page continues: {page:#}"));
            assert_eq!(page["page"]["stoppedBy"], "limit");
        }
        if cursor.is_none() {
            assert_eq!(page["page"]["stoppedBy"], "complete");
            break;
        }
        assert_eq!(page["page"]["stoppedBy"], "limit");
    }
    assert_eq!(seen.len(), 121);
    assert_eq!(seen.first().unwrap(), "main:Catalog.Item000");
    assert_eq!(seen.last().unwrap(), "main:Catalog.Item120");
    assert_eq!(
        seen.iter().collect::<std::collections::BTreeSet<_>>().len(),
        121
    );

    let mut nearest_cursor: Option<String> = None;
    let mut nearest_seen = Vec::new();
    for page_index in 0..16 {
        let mut arguments =
            json!({"query": "Iten", "corpus": "names", "kind": "Catalog", "limit": 17});
        if let Some(token) = &nearest_cursor {
            arguments["cursor"] = json!(token);
        }
        let page = mcp.completed_tool_call(call_tool(100 + page_index, "unica.search", arguments));
        assert_eq!(page["ok"], true, "{page:#}");
        assert_eq!(page["data"]["approximate"], true);
        nearest_seen.extend(
            page["data"]["matches"]
                .as_array()
                .expect("nearest matches")
                .iter()
                .map(|item| item["at"].as_str().unwrap().to_owned()),
        );
        nearest_cursor = page["cursor"].as_str().map(str::to_owned);
        if nearest_cursor.is_none() {
            assert_eq!(page["page"]["stoppedBy"], "complete");
            break;
        }
        assert_eq!(page["page"]["stoppedBy"], "limit");
    }
    assert_eq!(nearest_seen, seen);

    let default_page = mcp.completed_tool_call(call_tool(
        200,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog"}),
    ));
    assert_eq!(
        default_page["data"]["matches"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(default_page["page"]["stoppedBy"], "limit");
    let max_page = mcp.completed_tool_call(call_tool(
        201,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 50}),
    ));
    assert_eq!(
        max_page["data"]["matches"].as_array().map(Vec::len),
        Some(50)
    );
    assert_eq!(max_page["page"]["stoppedBy"], "limit");
    let over_limit = mcp.completed_tool_call(call_tool(
        202,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 51}),
    ));
    assert_eq!(over_limit["diagnostics"][0]["code"], "bad_value");

    let replay = mcp.completed_tool_call(call_tool(
        30,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 17, "cursor": first_cursor}),
    ));
    assert_eq!(replay["ok"], true, "{replay:#}");
    assert_eq!(replay["data"]["matches"][0]["at"], "main:Catalog.Item017");
    let wrong_kind = mcp.completed_tool_call(call_tool(
        31,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Document", "limit": 17, "cursor": first_cursor}),
    ));
    assert_eq!(wrong_kind["diagnostics"][0]["code"], "invalid_cursor");
    let wrong_limit = mcp.completed_tool_call(call_tool(
        32,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 18, "cursor": first_cursor}),
    ));
    assert_eq!(wrong_limit["diagnostics"][0]["code"], "invalid_cursor");

    std::fs::write(
        workspace.join("Catalogs/Item060.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Item060Changed</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("change one ranked name");
    let stale = mcp.completed_tool_call(call_tool(
        33,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 17, "cursor": first_cursor}),
    ));
    assert_eq!(stale["diagnostics"][0]["code"], "stale_cursor", "{stale:#}");
    mcp.finish();
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).expect("create fixture copy directory");
    for entry in std::fs::read_dir(from).expect("list fixture directory") {
        let entry = entry.expect("fixture directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("fixture entry type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// Opens either a module Body or an exact search range; the raw block uses
/// the absolute starting line declared by the ranged envelope.
fn view_body_lines(
    mcp: &mut McpProcess,
    id: &mut u64,
    at: &str,
) -> std::collections::BTreeMap<u64, String> {
    let response = mcp.exchange(call_tool(
        *id,
        "unica.view",
        json!({"at": if at.contains("#L") { at.to_owned() } else { format!("{at}.Body") }}),
    ));
    *id += 1;
    assert!(
        response["result"].get("structuredContent").is_none(),
        "{at}: {response:#}"
    );
    let text = response["result"]["content"][1]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{at}: Body text block: {response:#}"));
    let envelope: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let from = envelope["data"]["props"]["lines"]["from"]
        .as_u64()
        .unwrap_or(1);
    text.lines()
        .enumerate()
        .map(|(index, line)| (index as u64 + from, line.to_owned()))
        .collect()
}

fn hit_module(at: &str) -> &str {
    let body = at.split_once('#').map_or(at, |(body, _)| body);
    let owner = body.strip_suffix(".Body").unwrap_or(body);
    owner
        .rsplit_once(".Method.")
        .map_or(owner, |(module, _)| module)
}

/// #1298: a text hit names the logical module it was found in, and `view` of
/// that module's `Body` shows the same line, also on a first line after a BOM.
/// A file without a proven module owner keeps its hit, named by the scan
/// ordinal and the reason, never by its path.
#[test]
fn text_hits_name_their_module_and_view_reads_the_same_line() {
    let root = tempfile::tempdir().expect("search integration root");
    let workspace = root.path();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace"),
        workspace,
    );
    std::fs::write(
        workspace.join("src/Ext/Сценарий.bsl"),
        "// НичейнаяСтрока вне раскладки модулей\n",
    )
    .expect("stray BSL file");
    // A module whose owner the configuration does not register.
    std::fs::create_dir_all(workspace.join("src/CommonModules/Сирота/Ext"))
        .expect("unregistered module directory");
    std::fs::write(
        workspace.join("src/CommonModules/Сирота/Ext/Module.bsl"),
        "// НичейнаяСтрока без владельца\n",
    )
    .expect("unregistered module");
    // An extension registers its own module; a module it neither owns nor
    // borrows stays unproven.
    std::fs::create_dir_all(workspace.join("ext/CommonModules/Свой/Ext"))
        .expect("extension module directory");
    std::fs::create_dir_all(workspace.join("ext/CommonModules/Чужой/Ext"))
        .expect("unregistered extension module directory");
    std::fs::write(
        workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n  - name: ext\n    type: EXTENSION\n    path: ext\n",
    )
    .expect("workspace manifest with extension");
    std::fs::write(
        workspace.join("ext/Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Расширение</Name></Properties><ChildObjects><CommonModule>Свой</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .expect("extension configuration");
    std::fs::write(
        workspace.join("ext/CommonModules/Свой.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="dddddddd-dddd-4ddd-8ddd-dddddddddddd"><Properties><Name>Свой</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
    )
    .expect("extension module descriptor");
    std::fs::write(
        workspace.join("ext/CommonModules/Свой/Ext/Module.bsl"),
        "\u{feff}// НичейнаяСтрока в своём модуле\n",
    )
    .expect("extension module");
    std::fs::write(
        workspace.join("ext/CommonModules/Чужой/Ext/Module.bsl"),
        "// НичейнаяСтрока в чужом модуле\n",
    )
    .expect("unregistered extension module");
    for args in [["init", "-q"].as_slice(), ["add", "."].as_slice()] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(workspace)
            .status()
            .expect("prepare fixture repository")
            .success());
    }
    let mut mcp = McpProcess::start(workspace);
    initialize_body_mcp(&mut mcp);

    let mut id = 2;
    let mut first_line_hits = 0;
    for (query, expected) in [
        (
            "Возврат",
            [
                "main:Catalog.Валюты.Module.Manager",
                "main:Catalog.Валюты.Module.Object",
                "main:CommonModule.GoogleПереводчик",
                "main:HTTPService.ПроверкаСвязи.Module.HTTPService",
                "main:WebService.ОбменДанными.Module.WebService",
            ]
            .as_slice(),
        ),
        // Every fixture module starts with a byte-order mark.
        (
            "#Область ОбработчикиСобытий",
            [
                "main:Catalog.Валюты.Module.Object",
                "main:Catalog.Валюты.Form.ФормаЭлемента.Module.Form",
            ]
            .as_slice(),
        ),
    ] {
        let found = mcp.completed_tool_call(call_tool(
            id,
            "unica.search",
            json!({"query": query, "scope": "main:Configuration", "limit": 50}),
        ));
        id += 1;
        assert_eq!(found["ok"], true, "{found:#}");
        assert_eq!(found["page"]["stoppedBy"], "complete", "{found:#}");
        let matches = found["data"]["matches"].as_array().expect("text matches");
        let mut modules = std::collections::BTreeSet::new();
        for hit in matches {
            assert_eq!(hit["scope"], "main:Configuration", "{hit:#}");
            assert!(hit.get("path").is_none() && hit.get("file").is_none());
            let at = hit["at"]
                .as_str()
                .unwrap_or_else(|| panic!("hit has no module: {hit:#}"));
            modules.insert(hit_module(at).to_owned());
        }
        let mut expected = expected.to_vec();
        expected.sort_unstable();
        assert_eq!(
            modules.iter().map(String::as_str).collect::<Vec<_>>(),
            expected
        );
        for hit in matches {
            let at = hit["at"].as_str().unwrap();
            assert!(
                at.contains(".Body#L"),
                "a textual hit is an executable Body line address: {hit:#}"
            );
            let lines = view_body_lines(&mut mcp, &mut id, at);
            let line = hit["line"].as_u64().expect("hit line");
            let text = lines.get(&line).map(String::as_str);
            assert_eq!(text, hit["snippet"].as_str(), "{at} line {line}");
            let column =
                usize::try_from(hit["column"].as_u64().expect("hit column")).expect("column fits");
            let tail = text
                .expect("hit line")
                .chars()
                .skip(column - 1)
                .collect::<String>();
            assert!(tail.starts_with(query), "{at} line {line}: {hit:#}");
            if line == 1 {
                first_line_hits += 1;
            }
        }
    }
    assert!(
        first_line_hits >= 2,
        "a first-line hit after a BOM is checked"
    );

    let unowned = mcp.completed_tool_call(call_tool(
        id,
        "unica.search",
        json!({"query": "НичейнаяСтрока"}),
    ));
    id += 1;
    assert_eq!(unowned["ok"], true, "{unowned:#}");
    let unowned_matches = unowned["data"]["matches"].as_array().expect("matches");
    assert_eq!(unowned_matches.len(), 4, "{unowned:#}");
    let mut reasons = std::collections::BTreeMap::new();
    for hit in unowned_matches {
        assert!(!hit.to_string().contains(".bsl"), "{hit:#}");
        assert!(hit.get("path").is_none() && hit.get("file").is_none());
        if let Some(at) = hit["at"].as_str() {
            assert!(hit.get("unaddressed").is_none() && hit.get("fileId").is_none());
            reasons.insert(at.to_owned(), hit["scope"].as_str().unwrap().to_owned());
            assert_eq!(hit["column"], 4, "the BOM is not a column: {hit:#}");
            continue;
        }
        assert!(hit["fileId"]
            .as_str()
            .is_some_and(|file| file.starts_with("bsl-")));
        assert_eq!(hit["line"], 1);
        let snippet = hit["snippet"].as_str().unwrap();
        let reason = hit["unaddressed"].as_str().unwrap();
        reasons.insert(
            format!("{reason}: {snippet}"),
            hit["scope"].as_str().unwrap().to_owned(),
        );
    }
    assert_eq!(
        reasons,
        std::collections::BTreeMap::from([
            (
                "ext:CommonModule.Свой.Body#L1".to_owned(),
                "ext:Configuration".to_owned()
            ),
            (
                "outside_module_layout: // НичейнаяСтрока вне раскладки модулей".to_owned(),
                "main:Configuration".to_owned()
            ),
            (
                "owner_unproven: // НичейнаяСтрока без владельца".to_owned(),
                "main:Configuration".to_owned()
            ),
            (
                "owner_unproven: // НичейнаяСтрока в чужом модуле".to_owned(),
                "ext:Configuration".to_owned()
            ),
        ])
    );
    let own = view_body_lines(&mut mcp, &mut id, "ext:CommonModule.Свой");
    assert_eq!(
        own.get(&1).map(String::as_str),
        Some("// НичейнаяСтрока в своём модуле")
    );
    mcp.finish();
}

/// #1416: however broad the request scope — a metadata kind, an object with
/// several modules, the whole source set or no scope at all — every text hit
/// names the module it was found in. The broad answer gives the same `at`,
/// line, column and snippet as the search scoped to that very module, keeps
/// full coverage, and `scope` still echoes the request.
#[test]
fn broad_text_search_scopes_name_the_module_of_every_hit() {
    let root = tempfile::tempdir().expect("search integration root");
    let workspace = root.path();
    copy_tree(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace"),
        workspace,
    );
    for args in [["init", "-q"].as_slice(), ["add", "."].as_slice()] {
        assert!(Command::new("git")
            .args(args)
            .current_dir(workspace)
            .status()
            .expect("prepare fixture repository")
            .success());
    }
    let mut mcp = McpProcess::start(workspace);
    initialize_body_mcp(&mut mcp);
    let mut id = 2;
    let mut search = |mcp: &mut McpProcess, args: Value| {
        let found = mcp.completed_tool_call(call_tool(id, "unica.search", args));
        id += 1;
        assert_eq!(found["ok"], true, "{found:#}");
        assert_eq!(found["page"]["stoppedBy"], "complete", "{found:#}");
        assert_eq!(found["data"]["fileCoverage"]["complete"], true, "{found:#}");
        found["data"]["matches"]
            .as_array()
            .expect("text matches")
            .clone()
    };

    let declaration = "Функция НастройкиАвторизации(";
    let module = "main:CommonModule.GoogleПереводчик";
    let narrow = search(
        &mut mcp,
        json!({"query": declaration, "corpus": "text", "scope": module, "limit": 5}),
    );
    assert_eq!(narrow.len(), 1, "{narrow:#?}");
    assert!(
        narrow[0]["at"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{module}.Method.НастройкиАвторизации.Body#L")),
        "{narrow:#?}"
    );
    for scope in [Some("main:CommonModule"), Some("main:Configuration"), None] {
        let mut args = json!({"query": declaration, "corpus": "text", "limit": 5});
        if let Some(scope) = scope {
            args["scope"] = json!(scope);
        }
        let broad = search(&mut mcp, args);
        assert_eq!(broad.len(), 1, "{scope:?}: {broad:#?}");
        let hit = &broad[0];
        assert_eq!(
            hit["scope"],
            scope.unwrap_or("main:Configuration"),
            "{hit:#}"
        );
        for field in ["at", "line", "column", "snippet"] {
            assert_eq!(hit[field], narrow[0][field], "{scope:?} {field}: {hit:#}");
        }
        assert!(hit.get("unaddressed").is_none() && hit.get("fileId").is_none());
    }

    // An object scope spans several modules; each hit names its own.
    let object = search(
        &mut mcp,
        json!({"query": "Возврат", "scope": "main:Catalog.Валюты", "limit": 50}),
    );
    let modules = object
        .iter()
        .map(|hit| {
            assert_eq!(hit["scope"], "main:Catalog.Валюты", "{hit:#}");
            hit["at"]
                .as_str()
                .unwrap_or_else(|| panic!("hit has no module: {hit:#}"))
                .to_owned()
        })
        .map(|at| hit_module(&at).to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        modules.iter().map(String::as_str).collect::<Vec<_>>(),
        [
            "main:Catalog.Валюты.Module.Manager",
            "main:Catalog.Валюты.Module.Object",
        ]
    );
    mcp.finish();
}

#[test]
fn text_search_enumerates_wide_and_aggregate_corpora_without_an_entry_quota() {
    for distributed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        body_workspace(
            root.path(),
            b"// WideNeedle\n// WideNeedle\n// WideNeedle\n",
        );
        for index in 0..16_385 {
            let directory = if distributed {
                root.path().join(format!("ZNoise/part{}", index % 8))
            } else {
                root.path().join("ZNoise")
            };
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(directory.join(format!("noise{index:05}.txt")), b"").unwrap();
        }
        let mut mcp = McpProcess::start(root.path());
        initialize_body_mcp(&mut mcp);
        let first = mcp.completed_tool_call(call_tool(
            2,
            "unica.search",
            json!({
                "query": "WideNeedle", "scope": "main:Configuration", "limit": 2
            }),
        ));
        assert_eq!(first["ok"], true, "{first:#}");
        assert_eq!(first["data"]["matches"].as_array().unwrap().len(), 2);
        assert_eq!(first["page"]["stoppedBy"], "limit");
        let args = json!({"query":"WideNeedle", "scope":"main:Configuration", "limit":2, "cursor":first["cursor"]});
        let tail = mcp.completed_tool_call(call_tool(3, "unica.search", args.clone()));
        assert_eq!(tail["ok"], true, "{tail:#}");
        assert_eq!(tail["data"]["matches"].as_array().unwrap().len(), 1);
        assert_eq!(tail["page"]["stoppedBy"], "complete");
        let replay = mcp.completed_tool_call(call_tool(4, "unica.search", args));
        assert_eq!(replay, tail);
        let empty = mcp.completed_tool_call(call_tool(
            5,
            "unica.search",
            json!({"query":"AbsentNeedle", "scope":"main:Configuration"}),
        ));
        assert_eq!(empty["ok"], true, "{empty:#}");
        assert_eq!(empty["data"]["matches"].as_array().unwrap().len(), 0);
        assert_eq!(empty["page"]["stoppedBy"], "complete");
        mcp.finish();
    }
}

#[test]
fn text_search_keeps_nested_module_order_across_pages() {
    let root = tempfile::tempdir().unwrap();
    body_workspace(root.path(), b"// OrderedNeedle\n");
    let configuration = root.path().join("Configuration.xml");
    let source = std::fs::read_to_string(&configuration).unwrap().replace(
        "<CommonModule>Main</CommonModule>",
        "<CommonModule>A</CommonModule><CommonModule>Main</CommonModule><CommonModule>Z</CommonModule>",
    );
    std::fs::write(configuration, source).unwrap();
    for name in ["A", "Z"] {
        let descriptor = std::fs::read_to_string(root.path().join("CommonModules/Main.xml"))
            .unwrap()
            .replace("<Name>Main</Name>", &format!("<Name>{name}</Name>"));
        std::fs::write(
            root.path().join(format!("CommonModules/{name}.xml")),
            descriptor,
        )
        .unwrap();
        let ext = root.path().join(format!("CommonModules/{name}/Ext"));
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(ext.join("Module.bsl"), b"// OrderedNeedle\n").unwrap();
    }
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let mut args = json!({"query":"OrderedNeedle", "scope":"main:Configuration", "limit":1});
    // Git's path order: `A/` < `Main/` < `Z/`.
    for (index, owner) in ["A", "Main", "Z"].into_iter().enumerate() {
        let page =
            mcp.completed_tool_call(call_tool(2 + index as u64, "unica.search", args.clone()));
        assert_eq!(page["ok"], true, "{page:#}");
        assert_eq!(page["data"]["matches"].as_array().unwrap().len(), 1);
        assert_eq!(
            page["data"]["matches"][0]["at"],
            format!("main:CommonModule.{owner}.Body#L1")
        );
        if index == 2 {
            assert_eq!(page["page"]["stoppedBy"], "complete");
        } else {
            args["cursor"] = page["cursor"].clone();
        }
    }
    mcp.finish();
}

/// #1119: the literal search is not a window of its first 200 matches,
/// with Git and without it. Every match stays reachable through cursors in
/// traversal order, a cursor replays its page while the delivered part is
/// unchanged, and an edit before the cursor makes it stale instead of
/// shifting matches.
#[test]
fn literal_search_pages_past_two_hundred_matches_without_loss() {
    for git in [false, true] {
        literal_search_pages_past_two_hundred_matches(git);
    }
}

fn literal_search_pages_past_two_hundred_matches(git: bool) {
    let root = tempfile::tempdir().unwrap();
    body_workspace(root.path(), "// WideNeedle\n".repeat(450).as_bytes());
    if !git {
        // Without a work tree the walk answers.
        std::fs::remove_dir_all(root.path().join(".git")).unwrap();
    }
    let mut mcp = McpProcess::start(root.path());
    initialize_body_mcp(&mut mcp);
    let question = json!({"query": "WideNeedle", "scope": "main:CommonModule.Main", "limit": 50});
    let mut arguments = question.clone();
    let mut lines = Vec::new();
    let mut cursors = Vec::new();
    let last = loop {
        let page = mcp.completed_tool_call(call_tool(
            2 + cursors.len() as u64,
            "unica.search",
            arguments.clone(),
        ));
        assert_eq!(page["ok"], true, "git {git}: {page:#}");
        lines.extend(
            page["data"]["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hit| hit["line"].as_u64().unwrap()),
        );
        let Some(cursor) = page["cursor"].as_str() else {
            break page;
        };
        cursors.push(cursor.to_string());
        arguments["cursor"] = json!(cursor);
    };
    assert_eq!(lines, (1..=450).collect::<Vec<_>>(), "git {git}");
    assert_eq!(cursors.len(), 8, "git {git}");
    assert_eq!(last["page"]["stoppedBy"], "complete");
    assert_eq!(last["data"]["fileCoverage"]["complete"], true, "{last:#}");
    assert!(
        last["warnings"].as_array().is_none_or(Vec::is_empty),
        "{last:#}"
    );

    // The fifth cursor continues at match 251, past the former window.
    let mut past_window = question.clone();
    past_window["cursor"] = json!(cursors[4]);
    let page = mcp.completed_tool_call(call_tool(20, "unica.search", past_window.clone()));
    let replay = mcp.completed_tool_call(call_tool(21, "unica.search", past_window.clone()));
    assert_eq!(page, replay, "a cursor replays its page and continuation");
    assert_eq!(page["data"]["matches"][0]["line"], 251);

    // An edit after the delivered part is read by the next page.
    let module = root.path().join("CommonModules/Main/Ext/Module.bsl");
    let mut tail = "// WideNeedle\n".repeat(449);
    tail.push_str("// WideNeedle tail\n");
    std::fs::write(&module, tail).unwrap();
    let after_tail = mcp.completed_tool_call(call_tool(22, "unica.search", past_window.clone()));
    assert_eq!(after_tail["ok"], true, "{after_tail:#}");
    assert_eq!(after_tail["data"], page["data"]);

    // An edit inside the delivered part makes the cursor stale.
    let mut prefix = String::from("// WideNeedle head\n");
    prefix.push_str(&"// WideNeedle\n".repeat(450));
    std::fs::write(&module, prefix).unwrap();
    let stale = mcp.completed_tool_call(call_tool(23, "unica.search", past_window));
    assert_eq!(stale["ok"], false, "{stale:#}");
    assert_eq!(stale["diagnostics"][0]["code"], "stale_cursor");
    mcp.finish();
}
