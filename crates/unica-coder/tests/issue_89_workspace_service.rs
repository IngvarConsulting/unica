use rusqlite::Connection;
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

    mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
    assert_eq!(
        mcp.receive_ids(&[1], RESPONSE_DEADLINE)[&1]["result"]["serverInfo"]["name"],
        "unica"
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
        "unica.meta.profile",
        json!({
            "cwd": fixture.workspace,
            "name": "Catalog.Test",
            "sections": []
        }),
    ));
    let active_rlm = fixture.wait_for_two_active_rlm_starts(RESPONSE_DEADLINE);
    assert_ne!(active_rlm[0].pid, active_rlm[1].pid);
    let ping_started = Instant::now();
    mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"ping"}));
    mcp.send(json!({
        "jsonrpc":"2.0",
        "method":"notifications/cancelled",
        "params":{"requestId":2,"reason":"issue-89 regression"}
    }));
    let blocked_search = active_rlm
        .iter()
        .find(|record| record.sequence == 1)
        .expect("first RLM operation must be observed");
    assert!(wait_until_dead(blocked_search.pid, Duration::from_secs(2)));
    assert!(wait_until_dead(
        blocked_search.descendant_pid,
        Duration::from_secs(2)
    ));
    fixture.release_rlm(2);

    let (responses, response_times) =
        mcp.receive_ids_timed(&[2, 3, 4], RESPONSE_DEADLINE, ping_started);
    assert!(response_times[&4] < Duration::from_secs(2));
    assert_eq!(responses[&2]["error"]["code"], -32800);
    assert_eq!(responses[&2]["error"]["message"], "request cancelled");
    assert_tool_ok(
        &responses[&3],
        "completed through internal RLM metadata index",
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
        "unica.meta.profile",
        json!({
            "cwd": fixture.workspace,
            "name": "Catalog.Test",
            "sections": []
        }),
    ));
    let final_responses = mcp.receive_ids(&[5, 6], RESPONSE_DEADLINE);
    assert_tool_ok(&final_responses[&5], "typed bsl-analyzer MCP adapter");
    assert_tool_ok(
        &final_responses[&6],
        "completed through internal RLM metadata index",
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

    mcp.finish().unwrap();
    fixture.finish(&records).unwrap();
}

#[test]
fn issue_89_fixture_cleanup_is_bounded_during_assertion_unwind() {
    let tracked = Arc::new(Mutex::new(Vec::<ToolRecord>::new()));
    let fixture_root = Arc::new(Mutex::new(None::<PathBuf>));
    let tracked_inside = Arc::clone(&tracked);
    let root_inside = Arc::clone(&fixture_root);
    let started = Instant::now();
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let fixture = Fixture::new();
        *root_inside.lock().unwrap() = Some(fixture.root.clone());
        let mut mcp = McpProcess::start(&fixture);
        mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}));
        let _ = mcp.receive_ids(&[1], RESPONSE_DEADLINE);
        mcp.send(tool_call(
            2,
            "unica.code.search",
            json!({"cwd":fixture.workspace,"query":"Procedure"}),
        ));
        fixture.wait_for_log("rlm|", RESPONSE_DEADLINE);
        *tracked_inside.lock().unwrap() = fixture.log_records();
        panic!("intentional assertion unwind exercises RAII cleanup");
    }));

    assert!(unwind.is_err());
    assert!(started.elapsed() < Duration::from_secs(8));
    verify_records_dead(&tracked.lock().unwrap(), Duration::from_secs(3)).unwrap();
    let root = fixture_root.lock().unwrap().clone().unwrap();
    assert!(
        !root.exists(),
        "fixture root survived unwind: {}",
        root.display()
    );
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
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("missing tool result: {response:#}"));
    let operation: Value = serde_json::from_str(text).unwrap();
    assert_eq!(operation["ok"], true, "{operation:#}");
    assert!(
        operation["summary"]
            .as_str()
            .is_some_and(|value| value.contains(summary)),
        "{operation:#}"
    );
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
            .env("ISSUE89_RLM_DB", &fixture.rlm_db)
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
    rlm_db: PathBuf,
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
        let rlm_db = root.join("rlm-index.sqlite");
        fs::create_dir_all(workspace.join("src/cf/Configuration")).unwrap();
        fs::create_dir_all(workspace.join("src/cf/CommonModules/Test/Ext")).unwrap();
        fs::create_dir_all(workspace.join("exts/TESTS/Configuration")).unwrap();
        fs::create_dir_all(plugin_root.join("skills")).unwrap();
        fs::create_dir_all(plugin_root.join("third-party")).unwrap();
        fs::create_dir_all(&cache).unwrap();
        fs::create_dir_all(&rlm_state).unwrap();
        fs::write(workspace.join("v8project.yaml"), "format: DESIGNER\nsource-set:\n  main:\n    type: CONFIGURATION\n    path: src/cf\n  TESTS:\n    type: CONFIGURATION\n    path: exts/TESTS\n").unwrap();
        fs::write(workspace.join("src/cf/Configuration.xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><MetaDataObject><Configuration/></MetaDataObject>").unwrap();
        fs::write(
            workspace.join("src/cf/CommonModules/Test/Ext/Module.bsl"),
            "Procedure Test() Export\nEndProcedure\n",
        )
        .unwrap();
        fs::write(workspace.join("exts/TESTS/Configuration.xml"), "<?xml version=\"1.0\" encoding=\"UTF-8\"?><MetaDataObject><Configuration/></MetaDataObject>").unwrap();
        create_rlm_database(&rlm_db);
        compile_fake_tools(&root, &plugin_root);
        Self {
            root,
            workspace,
            plugin_root,
            cache,
            log,
            rlm_state,
            rlm_db,
            cleaned: false,
        }
    }

    fn release_rlm(&self, sequence: u32) {
        fs::write(self.rlm_state.join(format!("release-{sequence}")), "go").unwrap();
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

    fn wait_for_two_active_rlm_starts(&self, timeout: Duration) -> Vec<ToolRecord> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let records = self
                .log_records()
                .into_iter()
                .filter(|record| {
                    record.kind == "rlm"
                        && process_alive(record.pid)
                        && process_alive(record.descendant_pid)
                })
                .collect::<Vec<_>>();
            if records
                .iter()
                .map(|record| record.sequence)
                .collect::<HashSet<_>>()
                .len()
                >= 2
            {
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
        panic!("two distinct RLM processes were never concurrently active: {states:#?}");
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
        self.shutdown_services(Duration::from_secs(2))?;
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

fn create_rlm_database(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE modules (
                id INTEGER PRIMARY KEY,
                category TEXT,
                object_name TEXT,
                rel_path TEXT,
                module_type TEXT
            );
            CREATE TABLE object_attributes (
                category TEXT,
                object_name TEXT,
                attr_kind TEXT,
                attr_name TEXT,
                attr_type TEXT,
                ts_name TEXT
            );
            INSERT INTO modules(category, object_name, rel_path, module_type)
            VALUES ('Catalog', 'Test', 'Catalogs/Test/Ext/ObjectModule.bsl', 'object');",
        )
        .unwrap();
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
    for name in ["bsl-analyzer", "rlm-bsl-index"] {
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
    if name.contains("bsl-analyzer") { analyzer(&args); } else { rlm(&args); }
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

fn rlm(args: &[String]) {
    let root = args.last().unwrap();
    let sequence = claim_sequence();
    let mut descendant = spawn_descendant("rlm", root);
    record("rlm", sequence, descendant.id(), root);
    if sequence <= 2 {
        let release = std::path::Path::new(&env::var("ISSUE89_RLM_STATE").unwrap())
            .join(format!("release-{sequence}"));
        while !release.is_file() { thread::sleep(Duration::from_millis(2)); }
    }
    if sequence >= 2 {
        let _ = descendant.kill();
        for _ in 0..100 {
            if descendant.try_wait().ok().flatten().is_some() { break; }
            thread::sleep(Duration::from_millis(2));
        }
    }
    println!("Status: fresh");
    println!("Index: {}", env::var("ISSUE89_RLM_DB").unwrap());
    io::stdout().flush().unwrap();
}
"#;
