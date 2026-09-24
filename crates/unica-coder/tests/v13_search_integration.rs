use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "platform/v13_search_unreadable.rs"]
mod unreadable;

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
    std::fs::write(
        workspace.join("CommonModules/Main.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><Properties><Name>Main</Name><Global>false</Global><ClientManagedApplication>true</ClientManagedApplication><Server>true</Server><ExternalConnection>false</ExternalConnection><ClientOrdinaryApplication>false</ClientOrdinaryApplication><ServerCall>false</ServerCall><Privileged>false</Privileged><ReturnValuesReuse>DontUse</ReturnValuesReuse></Properties></CommonModule></MetaDataObject>"#,
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
    assert_eq!(main["data"]["matches"].as_array().map(Vec::len), Some(20));
    assert_eq!(main["data"]["matches"][0]["scope"], "main:Configuration");
    assert!(main["data"]["matches"][0].get("file").is_none());
    assert_eq!(main["page"]["stoppedBy"], "limit");
    let cursor = main["cursor"].as_str().expect("search continuation");
    let remaining = domain_result(&mcp.exchange(call_tool(
        5,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "cursor": cursor}),
    )));
    assert_eq!(remaining["ok"], true, "{remaining:#}");
    assert_eq!(
        remaining["data"]["matches"].as_array().map(Vec::len),
        Some(2)
    );
    assert_eq!(remaining["page"]["stoppedBy"], "complete");
    assert!(remaining.get("cursor").is_none());
    let replay = domain_result(&mcp.exchange(call_tool(
        6,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "cursor": cursor}),
    )));
    assert_eq!(
        replay, remaining,
        "retry of one cursor must replay its page"
    );
    let wrong_question = domain_result(&mcp.exchange(call_tool(
        7,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration", "cursor": cursor}),
    )));
    assert_eq!(wrong_question["diagnostics"][0]["code"], "invalid_cursor");

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

    let lexical = domain_result(&mcp.exchange(call_tool(
        24,
        "unica.search",
        json!({"query": "MainNeedle", "role": "lexical", "scope": "main:CommonModule.Main", "limit": 7}),
    )));
    assert_eq!(lexical["ok"], true, "{lexical:#}");
    assert_eq!(lexical["data"]["mode"], "lexical");
    let sections = lexical["data"]["matches"]
        .as_array()
        .expect("provider sections");
    assert_eq!(sections.len(), 1, "only the selected role may answer");
    assert_eq!(sections[0]["role"], "lexical");
    assert_eq!(sections[0]["hits"].as_array().map(Vec::len), Some(7));
    assert_eq!(sections[0]["matches"]["returned"], 7);
    let mut lexical_lines = sections[0]["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hit| hit["line"].as_u64().unwrap())
        .collect::<Vec<_>>();
    let mut lexical_cursor = lexical["cursor"]
        .as_str()
        .expect("provider continuation")
        .to_string();
    let mut lexical_pages = 1;
    loop {
        let page = domain_result(&mcp.exchange(call_tool(
            30 + lexical_pages,
            "unica.search",
            json!({"query": "MainNeedle", "role": "lexical", "scope": "main:CommonModule.Main", "limit": 7, "cursor": lexical_cursor}),
        )));
        assert_eq!(page["ok"], true, "{page:#}");
        let hits = page["data"]["matches"][0]["hits"].as_array().unwrap();
        assert_eq!(
            page["data"]["matches"][0]["matches"]["returned"],
            hits.len()
        );
        lexical_lines.extend(hits.iter().map(|hit| hit["line"].as_u64().unwrap()));
        lexical_pages += 1;
        if let Some(next) = page["cursor"].as_str() {
            lexical_cursor = next.to_string();
        } else {
            assert_eq!(page["page"]["stoppedBy"], "complete");
            assert_eq!(page["data"]["matches"][0]["searchComplete"], true);
            break;
        }
    }
    assert_eq!(lexical_pages, 4);
    assert_eq!(
        lexical_lines,
        std::iter::once(1).chain(3..=23).collect::<Vec<_>>()
    );
    let cross_mode = domain_result(&mcp.exchange(call_tool(
        35,
        "unica.search",
        json!({"query": "MainNeedle", "role": "lexical", "scope": "main:Configuration", "cursor": cursor}),
    )));
    assert_eq!(cross_mode["diagnostics"][0]["code"], "invalid_cursor");

    let main_names = domain_result(&mcp.exchange(call_tool(
        19,
        "unica.search",
        json!({"query": "CommonModule", "corpus": "names", "scope": "main:Configuration"}),
    )));
    assert_eq!(main_names["ok"], true, "{main_names:#}");
    let main_matches = main_names["data"]["matches"]
        .as_array()
        .expect("name matches");
    assert!(!main_matches.is_empty());
    assert!(main_matches.iter().all(|item| item["at"]
        .as_str()
        .is_some_and(|at| at.starts_with("main:"))));

    let named_scope = domain_result(&mcp.exchange(call_tool(
        20,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Main"}),
    )));
    assert_eq!(named_scope["ok"], true, "{named_scope:#}");
    assert_eq!(
        named_scope["data"]["matches"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        named_scope["data"]["matches"][0]["at"],
        "main:CommonModule.Main"
    );

    let missing_scope = domain_result(&mcp.exchange(call_tool(
        21,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Missing"}),
    )));
    assert_eq!(missing_scope["diagnostics"][0]["code"], "not_found");
    let unsupported_scope = domain_result(&mcp.exchange(call_tool(
        22,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:CommonModule.Main.Attribute.Missing"}),
    )));
    assert_eq!(
        unsupported_scope["diagnostics"][0]["code"],
        "unsupported_scope"
    );
    let unsupported_root = domain_result(&mcp.exchange(call_tool(
        23,
        "unica.search",
        json!({"query": "Main", "corpus": "names", "scope": "main:Form"}),
    )));
    assert_eq!(
        unsupported_root["diagnostics"][0]["code"],
        "unsupported_scope"
    );

    let absent_scope = domain_result(&mcp.exchange(call_tool(
        16,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:CommonModule.Missing"}),
    )));
    assert_eq!(absent_scope["diagnostics"][0]["code"], "not_found");
    let orphan_scope = domain_result(&mcp.exchange(call_tool(
        17,
        "unica.search",
        json!({"query": "OrphanNeedle", "scope": "main:CommonModule.Orphan"}),
    )));
    assert_eq!(orphan_scope["diagnostics"][0]["code"], "not_found");
    let branch = domain_result(&mcp.exchange(call_tool(
        18,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:CommonModule"}),
    )));
    assert_eq!(branch["ok"], true, "{branch:#}");
    assert_eq!(branch["data"]["matches"].as_array().map(Vec::len), Some(20));
    assert!(branch["cursor"].as_str().is_some());

    let broad =
        domain_result(&mcp.exchange(call_tool(8, "unica.search", json!({"query": "Needle"}))));
    assert_eq!(broad["page"]["stoppedBy"], "limit");
    let broad_cursor = broad["cursor"].as_str().expect("multi-source cursor");
    std::fs::write(
        workspace.join("src/extension/CommonModules/Extension/Ext/Module.bsl"),
        "Procedure ExtensionNeedle() Export\nEndProcedure\n// changed source\n",
    )
    .expect("change a source not reached by the first page");
    let stale = domain_result(&mcp.exchange(call_tool(
        9,
        "unica.search",
        json!({"query": "Needle", "cursor": broad_cursor}),
    )));
    assert_eq!(stale["diagnostics"][0]["code"], "stale_cursor");

    let cross_corpus = domain_result(&mcp.exchange(call_tool(
        10,
        "unica.search",
        json!({"query": "MainNeedle", "scope": "main:Configuration", "corpus": "names", "cursor": cursor}),
    )));
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
        let refused = domain_result(&mcp.exchange(call_tool(id, "unica.search", arguments)));
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
        let result = domain_result(&mcp.exchange(call_tool(2 + pages, "unica.search", arguments)));
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

    let view = domain_result(&mcp.exchange(call_tool(
        2,
        "unica.view",
        json!({"at": "main:Configuration"}),
    )));
    assert_eq!(view["ok"], false, "typed root must be unreadable: {view:#}");
    let search = domain_result(&mcp.exchange(call_tool(
        3,
        "unica.search",
        json!({"query": "Needle", "scope": "main:Configuration"}),
    )));
    assert_eq!(search["ok"], true, "{search:#}");
    assert_eq!(search["data"]["matches"].as_array().map(Vec::len), Some(1));
    assert_eq!(search["page"]["stoppedBy"], "complete");
    std::fs::remove_file(workspace.join("Configuration.xml"))
        .expect("remove the branch owner descriptor");
    let missing_root = domain_result(&mcp.exchange(call_tool(
        4,
        "unica.search",
        json!({"query": "Needle", "scope": "main:CommonModule"}),
    )));
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
        let page =
            domain_result(&mcp.exchange(call_tool(10 + page_index, "unica.search", arguments)));
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
        let page =
            domain_result(&mcp.exchange(call_tool(100 + page_index, "unica.search", arguments)));
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

    let default_page = domain_result(&mcp.exchange(call_tool(
        200,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog"}),
    )));
    assert_eq!(
        default_page["data"]["matches"].as_array().map(Vec::len),
        Some(20)
    );
    assert_eq!(default_page["page"]["stoppedBy"], "limit");
    let max_page = domain_result(&mcp.exchange(call_tool(
        201,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 50}),
    )));
    assert_eq!(
        max_page["data"]["matches"].as_array().map(Vec::len),
        Some(50)
    );
    assert_eq!(max_page["page"]["stoppedBy"], "limit");
    let over_limit = domain_result(&mcp.exchange(call_tool(
        202,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 51}),
    )));
    assert_eq!(over_limit["diagnostics"][0]["code"], "bad_value");

    let replay = domain_result(&mcp.exchange(call_tool(
        30,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 17, "cursor": first_cursor}),
    )));
    assert_eq!(replay["ok"], true, "{replay:#}");
    assert_eq!(replay["data"]["matches"][0]["at"], "main:Catalog.Item017");
    let wrong_kind = domain_result(&mcp.exchange(call_tool(
        31,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Document", "limit": 17, "cursor": first_cursor}),
    )));
    assert_eq!(wrong_kind["diagnostics"][0]["code"], "invalid_cursor");
    let wrong_limit = domain_result(&mcp.exchange(call_tool(
        32,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 18, "cursor": first_cursor}),
    )));
    assert_eq!(wrong_limit["diagnostics"][0]["code"], "invalid_cursor");

    std::fs::write(
        workspace.join("Catalogs/Item060.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Item060Changed</Name></Properties></Catalog></MetaDataObject>"#,
    )
    .expect("change one ranked name");
    let stale = domain_result(&mcp.exchange(call_tool(
        33,
        "unica.search",
        json!({"query": "Item", "corpus": "names", "kind": "Catalog", "limit": 17, "cursor": first_cursor}),
    )));
    assert_eq!(stale["diagnostics"][0]["code"], "stale_cursor", "{stale:#}");
    mcp.finish();
}
