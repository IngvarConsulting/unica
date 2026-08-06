use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[cfg(windows)]
use windows_sys::Win32::Foundation::CloseHandle;
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const RESPONSE_DEADLINE: Duration = Duration::from_secs(10);
static FIXTURE_NONCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue_89_multi_source_workspace_uses_main_root_and_remains_cancellable() {
    let mut fixture = Fixture::new();
    let mut mcp = McpProcess::start(&fixture);

    mcp.send(initialize_request());
    assert_eq!(
        mcp.receive_ids(&[1], RESPONSE_DEADLINE)[&1]["result"]["serverInfo"]["name"],
        "unica"
    );
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}));
    mcp.send(tool_call(
        10,
        "unica.code.search",
        json!({
            "cwd": fixture.workspace,
            "query": "Procedure",
            "dryRun": true
        }),
    ));
    let dry_run = mcp.receive_ids(&[10], RESPONSE_DEADLINE);
    assert_tool_ok(&dry_run[&10], "provider-neutral search coordinator");
    assert!(
        fixture.service_records().is_empty(),
        "code.search dryRun must not start workspace services"
    );

    mcp.send(tool_call(
        2,
        "unica.code.search",
        json!({
            "cwd": fixture.workspace,
            "query": "Procedure"
        }),
    ));
    fixture.wait_for_log("rlm|", RESPONSE_DEADLINE);
    let initial_owner = fixture.single_service_owner();

    mcp.send(tool_call(
        3,
        "unica.meta.info",
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Test",
            "sections": ["roles"]
        }),
    ));
    let first_rlm = fixture.wait_for_rlm_starts(1, RESPONSE_DEADLINE)[0].clone();
    assert_eq!(first_rlm.sequence, 1);
    let ping_started = Instant::now();
    mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"ping"}));
    mcp.send(json!({
        "jsonrpc":"2.0",
        "method":"notifications/cancelled",
        "params":{"requestId":2,"reason":"issue-89 regression"}
    }));
    assert!(wait_until_dead(first_rlm.pid, Duration::from_secs(2)));
    assert!(wait_until_dead(
        first_rlm.descendant_pid,
        Duration::from_secs(2)
    ));

    // The SDK drops the response of a cancelled request (MCP spec, ADR-0013);
    // cancellation itself is proven by the RLM process-tree death above.
    let (responses, response_times) =
        mcp.receive_ids_timed(&[3, 4], RESPONSE_DEADLINE, ping_started);
    assert!(response_times[&4] < Duration::from_secs(2));
    assert_meta_info_data(&responses[&3]);

    // meta.info is best-effort: it may return local metadata while the related
    // RLM section is unavailable, so it cannot prove that a persistent RLM
    // process restarted after cancellation. A direct code.search call invokes
    // that provider and therefore drives the recovery assertion below.
    mcp.send(tool_call(
        11,
        "unica.code.search",
        json!({
            "cwd": fixture.workspace,
            "query": "Procedure"
        }),
    ));
    let restarted_rlm = fixture.wait_for_rlm_starts(2, RESPONSE_DEADLINE);
    assert_ne!(restarted_rlm[0].pid, restarted_rlm[1].pid);
    assert_eq!(
        restarted_rlm
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    let recovery_search = mcp.receive_ids(&[11], RESPONSE_DEADLINE);
    assert_tool_ok(
        &recovery_search[&11],
        "provider-neutral code intelligence",
    );
    let session_records = fixture.log_records();
    assert_eq!(
        session_records
            .iter()
            .filter(|record| record.kind == "rlm-session-start" && record.sequence == 2)
            .count(),
        3,
        "rlm_start and rlm_execute session invalidation must each retry in the same RLM process"
    );
    assert!(
        session_records
            .iter()
            .any(|record| record.kind == "rlm-end" && record.sequence == 2),
        "the expired logical session must be closed before retry"
    );
    assert!(responses[&4].get("result").is_some(), "{:#}", responses[&4]);

    mcp.send(tool_call(
        5,
        "unica.code.graph",
        json!({
            "cwd": fixture.workspace,
            "mode": "callers",
            "query": "Test"
        }),
    ));
    mcp.send(tool_call(
        6,
        "unica.meta.info",
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Test",
            "sections": ["roles"]
        }),
    ));
    let final_responses = mcp.receive_ids(&[5, 6], RESPONSE_DEADLINE);
    assert_tool_ok(&final_responses[&5], "typed bsl-analyzer MCP adapter");
    assert_meta_info_data(&final_responses[&6]);
    mcp.send(tool_call(
        8,
        "unica.meta.info",
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.LogicalError",
            "sections": ["roles"]
        }),
    ));
    let logical_error = mcp.receive_ids(&[8], RESPONSE_DEADLINE);
    let logical_error = tool_operation(&logical_error[&8]);
    assert_eq!(logical_error["data"]["metadataPath"], "Catalog.LogicalError");
    assert!(logical_error.get("stdout").is_none(), "{logical_error:#}");
    // Nothing here can be unavailable any more: `meta.info` reads the source
    // tree and never asks a provider that could be down.
    assert!(logical_error["data"].get("related").is_none());
    // meta.info is a best-effort observer and does not promise to recover an
    // unavailable RLM provider. Drive recovery through the direct provider
    // contract before asking meta.info to observe the recovered session.
    mcp.send(tool_call(
        7,
        "unica.code.search",
        json!({
            "cwd": fixture.workspace,
            "query": "Procedure"
        }),
    ));
    let search_response = mcp.receive_ids(&[7], RESPONSE_DEADLINE);
    let search = tool_operation(&search_response[&7]);
    assert_eq!(search["ok"], true, "{search:#}");
    let search_sections = search["data"]["sections"].as_array().unwrap();
    assert_eq!(
        search_sections
            .iter()
            .map(|section| section["provider"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["rlm", "bsl-analyzer", "git-grep"]
    );
    let rlm_status = search_sections
        .iter()
        .find(|section| section["provider"] == "rlm")
        .and_then(|section| section["status"].as_str());
    assert!(
        matches!(rlm_status, Some("ok" | "empty")),
        "the direct recovery probe did not recover RLM: response={search:#}, records={:#?}",
        fixture.log_records()
    );
    mcp.send(tool_call(
        9,
        "unica.meta.info",
        json!({
            "sourceSet": "main",
            "metadataPath": "Catalog.Test",
            "sections": ["roles"]
        }),
    ));
    let after_logical_error = mcp.receive_ids(&[9], RESPONSE_DEADLINE);
    assert_meta_info_data(&after_logical_error[&9]);
    // `meta.info` no longer observes the index session at all, so recovery is
    // proven through the direct provider probe below rather than through it.
    assert!(
        tool_operation(&after_logical_error[&9])["data"]["usage"]["roles"].is_array(),
        "usage was not read after the logical error: response={:#}, records={:#?}",
        tool_operation(&after_logical_error[&9]),
        fixture.log_records()
    );

    let expected_root = canonical_display(&fixture.workspace.join("src/cf"));
    let records = fixture.log_records();
    assert!(records.iter().any(|record| record.kind == "analyzer"));
    assert!(records.iter().any(|record| record.kind == "rlm"));
    assert!(
        records
            .iter()
            .all(|record| record.source_root == expected_root),
        "{records:#?}"
    );
    let service_records = fixture.service_records();
    assert_eq!(
        service_records.len(),
        1,
        "parallel calls for the same effective source root must reuse one service identity"
    );
    assert_eq!(service_records[0]["source_root"], expected_root);
    assert_eq!(
        service_records[0]["workspace_root"],
        canonical_display(&fixture.workspace)
    );
    assert_eq!(fixture.single_service_owner(), initial_owner);
    assert!(fixture.service_is_alive());
    assert_eq!(
        fixture
            .log_records()
            .into_iter()
            .filter(|record| record.kind == "rlm")
            .count(),
        2,
        "the second persistent RLM process must be reused after cancellation recovery"
    );

    mcp.finish().unwrap();
    fixture.finish(&records).unwrap();
}

#[test]
fn issue_89_fixture_cleanup_is_bounded_during_assertion_unwind() {
    let tracked = Arc::new(Mutex::new(Vec::<ToolRecord>::new()));
    let fixture_root = Arc::new(Mutex::new(None::<PathBuf>));
    let cleanup_started = Arc::new(Mutex::new(None::<Instant>));
    let tracked_inside = Arc::clone(&tracked);
    let root_inside = Arc::clone(&fixture_root);
    let cleanup_started_inside = Arc::clone(&cleanup_started);
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let fixture = Fixture::new();
        *root_inside.lock().unwrap() = Some(fixture.root.clone());
        let mut mcp = McpProcess::start(&fixture);
        mcp.send(initialize_request());
        let _ = mcp.receive_ids(&[1], RESPONSE_DEADLINE);
        mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}));
        mcp.send(tool_call(
            2,
            "unica.code.search",
            json!({"cwd":fixture.workspace,"query":"Procedure"}),
        ));
        fixture.wait_for_log("rlm|", RESPONSE_DEADLINE);
        *tracked_inside.lock().unwrap() = fixture.log_records();
        *cleanup_started_inside.lock().unwrap() = Some(Instant::now());
        panic!("intentional assertion unwind exercises RAII cleanup");
    }));

    assert!(unwind.is_err());
    let cleanup_elapsed = cleanup_started
        .lock()
        .unwrap()
        .expect("cleanup timer must start before intentional unwind")
        .elapsed();
    assert!(
        cleanup_elapsed < Duration::from_secs(8),
        "RAII cleanup exceeded its deadline: {cleanup_elapsed:?}"
    );
    verify_records_dead(&tracked.lock().unwrap(), Duration::from_secs(3)).unwrap();
    let root = fixture_root.lock().unwrap().clone().unwrap();
    assert!(
        !root.exists(),
        "fixture root survived unwind: {}",
        root.display()
    );
}

fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "issue-89-regression", "version": "1"}
        }
    })
}

fn tool_call(id: u64, name: &str, arguments: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

fn send_service_request(record: &Value, kind: Value) -> Result<Value, String> {
    send_service_request_with_timeout(record, kind, RESPONSE_DEADLINE)
}

fn send_service_request_with_timeout(
    record: &Value,
    kind: Value,
    timeout: Duration,
) -> Result<Value, String> {
    let port = record["port"]
        .as_u64()
        .ok_or_else(|| "service record has no port".to_string())?;
    let token = record["token"]
        .as_str()
        .ok_or_else(|| "service record has no token".to_string())?;
    let address = SocketAddr::from(([127, 0, 0, 1], port as u16));
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| error.to_string())?;
    serde_json::to_writer(&mut stream, &json!({"token":token,"kind":kind}))
        .map_err(|error| error.to_string())?;
    stream.write_all(b"\n").map_err(|error| error.to_string())?;
    stream.flush().map_err(|error| error.to_string())?;
    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|error| error.to_string())?;
    serde_json::from_str(&response).map_err(|error| error.to_string())
}

fn assert_tool_ok(response: &Value, summary: &str) {
    let operation = tool_operation(response);
    assert_eq!(operation["ok"], true, "{operation:#}");
    assert!(
        operation["summary"]
            .as_str()
            .is_some_and(|value| value.contains(summary)),
        "{operation:#}"
    );
}

fn assert_meta_info_data(response: &Value) {
    let operation = tool_operation(response);
    assert_eq!(operation["data"]["metadataPath"], "Catalog.Test", "{operation:#}");
    // `meta.info` is local now: the object's own structure is the whole answer
    // unless usage sections are asked for, and no section consults the index.
    assert!(operation["data"].get("related").is_none(), "{operation:#}");
    assert!(operation["data"]["usage"].is_object(), "{operation:#}");
    assert!(operation.get("stdout").is_none(), "{operation:#}");
}

fn tool_operation(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing tool result: {response:#}"));
    serde_json::from_str(text).unwrap()
}

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    responses: mpsc::Receiver<String>,
}

impl McpProcess {
    fn start(fixture: &Fixture) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_unica"))
            .current_dir(&fixture.workspace)
            .env("UNICA_PLUGIN_ROOT", &fixture.plugin_root)
            .env("UNICA_CACHE_DIR", &fixture.cache)
            .env("ISSUE89_LOG", &fixture.log)
            .env("ISSUE89_RLM_STATE", &fixture.rlm_state)
            .env("UNICA_WORKSPACE_SERVICE_IDLE_SECS", "30")
            .env("UNICA_WORKSPACE_SERVICE_MAX_AGE_SECS", "60")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start unica MCP");
        let stdin = child.stdin.take().expect("MCP stdin");
        let stdout = child.stdout.take().expect("MCP stdout");
        let (tx, responses) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if tx.send(line).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        });
        Self {
            child,
            stdin: Some(stdin),
            responses,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("open MCP stdin");
        serde_json::to_writer(&mut *stdin, &message).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }

    fn receive_ids(&self, ids: &[u64], timeout: Duration) -> HashMap<u64, Value> {
        self.receive_ids_timed(ids, timeout, Instant::now()).0
    }

    fn receive_ids_timed(
        &self,
        ids: &[u64],
        timeout: Duration,
        started: Instant,
    ) -> (HashMap<u64, Value>, HashMap<u64, Duration>) {
        let deadline = Instant::now() + timeout;
        let expected = ids.iter().copied().collect::<HashSet<_>>();
        let mut found = HashMap::new();
        let mut response_times = HashMap::new();
        while found.len() < expected.len() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting for MCP ids {expected:?}; got {found:?}"
            );
            let line = self
                .responses
                .recv_timeout(remaining)
                .expect("MCP response before deadline");
            let response: Value = serde_json::from_str(&line).expect("JSON MCP response");
            if let Some(id) = response.get("id").and_then(Value::as_u64) {
                if expected.contains(&id) {
                    response_times.insert(id, started.elapsed());
                    found.insert(id, response);
                }
            }
        }
        (found, response_times)
    }

    fn finish(&mut self) -> Result<(), String> {
        drop(self.stdin.take());
        if let Some(status) = wait_child_bounded(&mut self.child, RESPONSE_DEADLINE)? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("unica exited with {status}"))
            };
        }
        self.child.kill().map_err(|error| error.to_string())?;
        wait_child_bounded(&mut self.child, Duration::from_secs(2))?
            .ok_or_else(|| "unica did not exit after kill fallback".to_string())?;
        Err("unica required kill fallback after stdin EOF".to_string())
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if wait_child_bounded(&mut self.child, Duration::from_millis(500))
            .ok()
            .flatten()
            .is_none()
        {
            let _ = self.child.kill();
            let _ = wait_child_bounded(&mut self.child, Duration::from_secs(2));
        }
    }
}

fn wait_child_bounded(
    child: &mut Child,
    timeout: Duration,
) -> Result<Option<std::process::ExitStatus>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::yield_now();
    }
}

struct Fixture {
    root: PathBuf,
    workspace: PathBuf,
    plugin_root: PathBuf,
    cache: PathBuf,
    log: PathBuf,
    rlm_state: PathBuf,
    cleaned: bool,
}

impl Fixture {
    fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let nonce = FIXTURE_NONCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "unica-issue-89-{}-{timestamp}-{nonce}",
            std::process::id()
        ));
        let workspace = root.join("workspace");
        let plugin_root = root.join("plugin");
        let cache = root.join("cache");
        let log = root.join("tool.log");
        let rlm_state = root.join("rlm-state");
        fs::create_dir_all(workspace.join("src/cf/Configuration")).unwrap();
        fs::create_dir_all(workspace.join("src/cf/CommonModules/Test/Ext")).unwrap();
        fs::create_dir_all(workspace.join("src/cf/Catalogs")).unwrap();
        fs::create_dir_all(workspace.join("src/cf/Languages")).unwrap();
        fs::create_dir_all(workspace.join("exts/TESTS/Configuration")).unwrap();
        fs::create_dir_all(plugin_root.join("skills")).unwrap();
        fs::create_dir_all(plugin_root.join("third-party")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&rlm_state).unwrap();
        fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  main:\n    type: CONFIGURATION\n    path: src/cf\n  TESTS:\n    type: CONFIGURATION\n    path: exts/TESTS\n").unwrap();
        fs::write(
            workspace.join("src/cf/Configuration.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><InternalInfo/><Properties><Name>Issue89</Name><DefaultLanguage>Russian</DefaultLanguage></Properties><ChildObjects><Language>Russian</Language><Catalog>Test</Catalog><Catalog>LogicalError</Catalog><CommonModule>Test</CommonModule></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            workspace.join("src/cf/Languages/Russian.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="dddddddd-dddd-4ddd-8ddd-dddddddddddd"><Properties><Name>Russian</Name><Synonym/><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            workspace.join("src/cf/Catalogs/Test.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><InternalInfo/><Properties><Name>Test</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            workspace.join("src/cf/Catalogs/LogicalError.xml"),
            r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="cccccccc-cccc-4ccc-8ccc-cccccccccccc"><InternalInfo/><Properties><Name>LogicalError</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            workspace.join("src/cf/CommonModules/Test/Ext/Module.bsl"),
            "Procedure Test() Export\nEndProcedure\n",
        )
        .unwrap();
        fs::write(workspace.join("exts/TESTS/Configuration.xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><MetaDataObject><Configuration/></MetaDataObject>").unwrap();
        compile_fake_tools(&root, &plugin_root);
        Self {
            root,
            workspace,
            plugin_root,
            cache,
            log,
            rlm_state,
            cleaned: false,
        }
    }

    fn wait_for_log(&self, prefix: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .any(|line| line.starts_with(prefix))
            {
                return;
            }
            thread::yield_now();
        }
        panic!("timed out waiting for fake-tool log prefix {prefix}");
    }

    fn wait_for_rlm_starts(&self, expected: usize, timeout: Duration) -> Vec<ToolRecord> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let records = self
                .log_records()
                .into_iter()
                .filter(|record| record.kind == "rlm")
                .collect::<Vec<_>>();
            if records.len() >= expected {
                return records;
            }
            thread::yield_now();
        }
        let records = self.try_log_records().unwrap_or_default();
        let states = records
            .iter()
            .map(|record| {
                (
                    record.clone(),
                    process_alive(record.pid),
                    process_alive(record.descendant_pid),
                )
            })
            .collect::<Vec<_>>();
        panic!("expected {expected} RLM process starts were not observed: {states:#?}");
    }

    fn single_service_owner(&self) -> (u64, String, u64, u64) {
        let records = self.service_records();
        assert_eq!(records.len(), 1);
        (
            records[0]["pid"].as_u64().unwrap(),
            records[0]["token"].as_str().unwrap().to_string(),
            records[0]["port"].as_u64().unwrap(),
            records[0]["started_at"].as_u64().unwrap(),
        )
    }

    fn service_is_alive(&self) -> bool {
        let records = self.service_records();
        let Some(record) = records.first() else {
            return false;
        };
        send_service_request(record, json!({"type":"ping"}))
            .ok()
            .and_then(|response| response["status"].as_str().map(ToString::to_string))
            .as_deref()
            == Some("alive")
    }

    fn log_records(&self) -> Vec<ToolRecord> {
        self.try_log_records().unwrap()
    }

    fn try_log_records(&self) -> Result<Vec<ToolRecord>, String> {
        let text = fs::read_to_string(&self.log).map_err(|error| error.to_string())?;
        text.lines()
            .map(|line| {
                let fields = line.splitn(5, '|').collect::<Vec<_>>();
                if fields.len() != 5 {
                    return Err(format!("incomplete fake-tool log line: {line}"));
                }
                Ok(ToolRecord {
                    kind: fields[0].to_string(),
                    sequence: fields[1]
                        .parse::<u32>()
                        .map_err(|error| error.to_string())?,
                    pid: fields[2]
                        .parse::<u32>()
                        .map_err(|error| error.to_string())?,
                    descendant_pid: fields[3]
                        .parse::<u32>()
                        .map_err(|error| error.to_string())?,
                    source_root: fields[4].to_string(),
                })
            })
            .collect()
    }

    fn service_records(&self) -> Vec<Value> {
        self.try_service_records().unwrap()
    }

    fn try_service_records(&self) -> Result<Vec<Value>, String> {
        let services = self.cache.join("services");
        if !services.is_dir() {
            return Ok(Vec::new());
        }
        Ok(fs::read_dir(services)
            .map_err(|error| error.to_string())?
            .flatten()
            .filter_map(|entry| fs::read_to_string(entry.path().join("service.json")).ok())
            .filter_map(|text| serde_json::from_str(&text).ok())
            .collect())
    }

    fn finish(&mut self, records: &[ToolRecord]) -> Result<(), String> {
        self.shutdown_services(RESPONSE_DEADLINE)?;
        let deadline = Instant::now() + RESPONSE_DEADLINE;
        loop {
            let final_records = self.try_log_records()?;
            if final_records
                .iter()
                .filter(|record| record.kind == "rlm-end" && record.sequence == 2)
                .count()
                >= 2
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(
                    "persistent RLM session was not ended during service shutdown".to_string(),
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
        verify_records_dead(records, RESPONSE_DEADLINE)?;
        fs::remove_dir_all(&self.root).map_err(|error| error.to_string())?;
        self.cleaned = true;
        Ok(())
    }

    fn shutdown_services(&self, timeout: Duration) -> Result<(), String> {
        let records = self.try_service_records()?;
        for record in &records {
            let response =
                send_service_request_with_timeout(record, json!({"type":"shutdown"}), timeout)?;
            if response["ok"] != true {
                return Err(format!("workspace service rejected shutdown: {response}"));
            }
        }
        for record in records {
            if let Some(pid) = record["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
            {
                if !wait_until_dead(pid, timeout) {
                    terminate_pid_tree(pid);
                    if !wait_until_dead(pid, Duration::from_secs(2)) {
                        return Err(format!("workspace service pid {pid} survived shutdown"));
                    }
                }
            }
        }
        Ok(())
    }

    fn cleanup_best_effort(&mut self) {
        let service_records = self.try_service_records().unwrap_or_default();
        for record in &service_records {
            let _ = send_service_request_with_timeout(
                record,
                json!({"type":"shutdown"}),
                Duration::from_millis(500),
            );
        }
        for record in service_records {
            if let Some(pid) = record["pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
            {
                if !wait_until_dead(pid, Duration::from_millis(500)) {
                    terminate_pid_tree(pid);
                    let _ = wait_until_dead(pid, Duration::from_secs(1));
                }
            }
        }
        for record in self.try_log_records().unwrap_or_default() {
            for pid in [record.pid, record.descendant_pid] {
                if process_alive(pid) {
                    terminate_pid_tree(pid);
                    let _ = wait_until_dead(pid, Duration::from_secs(1));
                }
            }
        }
        let _ = fs::remove_dir_all(&self.root);
        self.cleaned = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.cleaned {
            self.cleanup_best_effort();
        }
    }
}

#[derive(Clone, Debug)]
struct ToolRecord {
    kind: String,
    sequence: u32,
    pid: u32,
    descendant_pid: u32,
    source_root: String,
}

fn compile_fake_tools(root: &Path, plugin_root: &Path) {
    let source = root.join("fake_tool.rs");
    fs::write(&source, FAKE_TOOL_SOURCE).unwrap();
    let fake = root.join(format!("fake-tool{}", std::env::consts::EXE_SUFFIX));
    let output = Command::new("rustc")
        .args(["--edition=2021", "-O"])
        .arg(&source)
        .arg("-o")
        .arg(&fake)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fake tool compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let lock_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../plugins/unica/third-party/tools.lock.json");
    let lock_text = fs::read_to_string(&lock_path).unwrap();
    let lock: Value = serde_json::from_str(&lock_text).unwrap();
    let target = host_target();
    let target_contract = &lock["targets"][target];
    let exe = target_contract["exe"].as_str().unwrap();
    let bin = plugin_root.join("bin").join(target);
    fs::create_dir_all(&bin).unwrap();
    let sha256 = sha256_file(&fake);
    let mut manifest_tools = Vec::new();
    for name in ["bsl-analyzer", "rlm-tools-bsl", "rlm-bsl-index"] {
        let contract = lock["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        let binary_name = contract["binaryName"].as_str().unwrap();
        let relative = format!("bin/{target}/{binary_name}{exe}");
        fs::copy(&fake, plugin_root.join(&relative)).unwrap();
        manifest_tools.push(json!({
            "name": name,
            "version": contract["version"],
            "binaries": {
                (target): {
                    "targetTriple": target_contract["targetTriple"],
                    "binaryPath": relative,
                    "sha256": sha256.clone(),
                }
            }
        }));
    }
    fs::write(
        plugin_root.join("third-party/manifest.json"),
        serde_json::to_vec_pretty(&json!({"schemaVersion": 2, "tools": manifest_tools})).unwrap(),
    )
    .unwrap();
    fs::write(plugin_root.join("third-party/tools.lock.json"), lock_text).unwrap();
}

fn sha256_file(path: &Path) -> String {
    let mut file = fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).unwrap();
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    format!("{:x}", digest.finalize())
}

fn host_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "win-x64",
        ("linux", "x86_64") => "linux-x64",
        ("macos", "aarch64") => "darwin-arm64",
        host => panic!("unsupported integration-test host {host:?}"),
    }
}

fn canonical_display(path: &Path) -> String {
    let path = fs::canonicalize(path).unwrap();
    #[cfg(windows)]
    return path
        .display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .to_string();
    #[cfg(not(windows))]
    path.display().to_string()
}

fn verify_records_dead(records: &[ToolRecord], timeout: Duration) -> Result<(), String> {
    let pids = records
        .iter()
        .flat_map(|record| [record.pid, record.descendant_pid])
        .collect::<HashSet<_>>();
    for pid in pids {
        if !wait_until_dead(pid, timeout) {
            return Err(format!(
                "fake tool parent/descendant pid {pid} survived cancellation/shutdown"
            ));
        }
    }
    Ok(())
}

fn wait_until_dead(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while process_alive(pid) && Instant::now() < deadline {
        thread::yield_now();
    }
    !process_alive(pid)
}

#[cfg(unix)]
fn terminate_pid_tree(pid: u32) {
    let group = format!("-{pid}");
    let direct = pid.to_string();
    let _ = Command::new("kill")
        .args(["-TERM", &group])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = Command::new("kill")
        .args(["-TERM", &direct])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !wait_until_dead(pid, Duration::from_millis(500)) {
        let _ = Command::new("kill")
            .args(["-KILL", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("kill")
            .args(["-KILL", &direct])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(windows)]
fn terminate_pid_tree(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0_u32;
        let result = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        result != 0 && exit_code == 259
    }
}

const FAKE_TOOL_SOURCE: &str = r#"
use std::env;
use std::fs::OpenOptions;
use std::io::{self, BufRead, Write};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn main() {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("--descendant") {
        loop { thread::park_timeout(Duration::from_secs(60)); }
    }
    let exe = env::current_exe().unwrap();
    let name = exe.file_stem().unwrap().to_string_lossy();
    if name.contains("bsl-analyzer") {
        analyzer(&args);
    } else if name.contains("rlm-bsl-index") {
        rlm_index();
    } else {
        rlm_mcp();
    }
}

fn spawn_descendant(kind: &str, root: &str) -> Child {
    let mut command = Command::new(env::current_exe().unwrap());
    command.args(["--descendant", kind, root]);
    if kind == "rlm" {
        command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    }
    command.spawn().unwrap()
}

fn record(kind: &str, sequence: u32, descendant: u32, root: &str) {
    let mut file = OpenOptions::new().create(true).append(true).open(env::var("ISSUE89_LOG").unwrap()).unwrap();
    let line = format!("{}|{}|{}|{}|{}\n", kind, sequence, std::process::id(), descendant, root);
    file.write_all(line.as_bytes()).unwrap();
    file.flush().unwrap();
}

fn analyzer(args: &[String]) {
    let root = args.windows(2).find(|pair| pair[0] == "--source-dir").map(|pair| pair[1].clone()).unwrap();
    let descendant = spawn_descendant("analyzer", &root);
    record("analyzer", 0, descendant.id(), &root);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if !line.contains("\"id\"") { continue; }
        let id = line.split("\"id\":").nth(1).and_then(|tail| tail.split(|c: char| !c.is_ascii_digit()).next()).unwrap();
        if line.contains("\"method\":\"initialize\"") {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{{}},\"serverInfo\":{{\"name\":\"fake\",\"version\":\"test\"}}}}}}", id);
        } else {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{{\\\"action\\\":\\\"callers\\\",\\\"nodes\\\":[]}}\"}}]}}}}", id);
        }
        io::stdout().flush().unwrap();
    }
}

fn claim_sequence() -> u32 {
    let state = env::var("ISSUE89_RLM_STATE").unwrap();
    for sequence in 1.. {
        let marker = std::path::Path::new(&state).join(format!("start-{sequence}"));
        if OpenOptions::new().write(true).create_new(true).open(marker).is_ok() {
            return sequence;
        }
    }
    unreachable!()
}

fn json_string(line: &str, key: &str) -> String {
    let marker = format!("\"{key}\":\"");
    line.split_once(&marker)
        .and_then(|(_, tail)| tail.split('"').next())
        .unwrap_or("")
        .replace("\\\\", "\\")
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn respond(id: &str, text: &str) {
    println!(
        "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}}}}",
        json_escape(text)
    );
    io::stdout().flush().unwrap();
}

fn rlm_mcp() {
    let sequence = claim_sequence();
    let mut descendant = spawn_descendant("rlm", "pending");
    let mut recorded = false;
    let mut root = String::new();
    let mut start_count = 0_u32;
    let mut execute_count = 0_u32;
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if !line.contains("\"id\"") { continue; }
        let id = line.split("\"id\":").nth(1).and_then(|tail| tail.split(|c: char| !c.is_ascii_digit()).next()).unwrap();
        if line.contains("\"method\":\"initialize\"") {
            println!("{{\"jsonrpc\":\"2.0\",\"id\":{},\"result\":{{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{{}},\"serverInfo\":{{\"name\":\"fake-rlm\",\"version\":\"test\"}}}}}}", id);
            io::stdout().flush().unwrap();
            continue;
        }
        if line.contains("\"name\":\"rlm_start\"") {
            root = json_string(&line, "path");
            if !recorded {
                record("rlm", sequence, descendant.id(), &root);
                recorded = true;
            }
            record("rlm-session-start", sequence, descendant.id(), &root);
            start_count += 1;
            if sequence == 2 && start_count == 1 {
                respond(id, "{\"error\":\"Sandbox not found\"}");
                continue;
            }
            respond(id, &format!("{{\"session_id\":\"session-{sequence}\",\"index\":{{\"index_status\":\"fresh\"}}}}"));
        } else if line.contains("\"name\":\"rlm_execute\"") {
            if sequence == 1 {
                loop { thread::sleep(Duration::from_secs(60)); }
            }
            execute_count += 1;
            let operation_kind = if line.contains("get_object_profile") {
                "rlm-execute-profile"
            } else {
                "rlm-execute-search"
            };
            record(operation_kind, sequence, descendant.id(), &root);
            if line.contains("LogicalError") {
                respond(id, "{\"error\":\"invalid logical request\"}");
                continue;
            }
            if sequence == 2 && execute_count == 1 {
                respond(id, "{\"error\":\"Session not found or expired\"}");
                continue;
            }
            let helper = if line.contains("get_object_profile") {
                "{\"object_name\":\"Test\",\"category\":\"Catalog\",\"sections\":{\"modules\":{\"status\":\"ok\",\"items\":[],\"total\":0,\"returned\":0}}}"
            } else {
                "[]"
            };
            let envelope = format!("{{\"stdout\":\"{}\",\"error\":null}}", json_escape(helper));
            respond(id, &envelope);
        } else if line.contains("\"name\":\"rlm_end\"") {
            record("rlm-end", sequence, descendant.id(), &root);
            respond(id, "{\"ended\":true}");
        } else {
            respond(id, "{\"error\":\"unexpected fake RLM tool\"}");
        }
    }
    let _ = descendant.kill();
    for _ in 0..100 {
        if descendant.try_wait().ok().flatten().is_some() { break; }
        thread::sleep(Duration::from_millis(2));
    }
}

fn rlm_index() {
    println!("Status: fresh");
    println!(
        "Index: {}",
        std::path::Path::new(&env::var("RLM_INDEX_DIR").unwrap())
            .join("fake/bsl_index.db")
            .display()
    );
    io::stdout().flush().unwrap();
}
"#;
