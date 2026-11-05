use super::*;

struct ServiceGuard {
    runtime: Arc<WorkspaceServiceRuntime>,
    server: Option<thread::JoinHandle<Result<(), String>>>,
}
impl Drop for ServiceGuard {
    fn drop(&mut self) {
        self.runtime.test_begin_shutdown();
        if let Some(server) = self.server.take() {
            let result = server.join();
            if !thread::panicking() {
                result.unwrap().unwrap();
            }
        }
    }
}

// Only this managed provider understands either opaque format. It reads the
// actual selected immutable build on every execute, rather than returning a
// predetermined answer from a Rust provider double.
fn compile_provider(directory: &Path) -> PathBuf {
    let source = directory.join("provider.rs");
    let executable =
        unica_process::test_support::fixture_executable_path(directory, "opaque-provider");
    fs::create_dir_all(directory).unwrap();
    fs::write(&source, r###"use std::{env,fs,io::{self,BufRead,Write}};
fn quote(s:&str)->String { format!("\"{}\"",s.replace('\\',"\\\\").replace('"',"\\\"").replace('\n',"\\n")) }
fn main(){
 let db=env::args().nth(1).unwrap(); let trace=env::args().nth(2).unwrap();
 for line in io::stdin().lock().lines(){
  let line=line.unwrap();
  let Some(rest)=line.split("\"id\":").nth(1) else {continue};
  let id=rest.chars().take_while(|c|c.is_ascii_digit()).collect::<String>();
  assert!(!id.is_empty(),"{line}");
  let result=if line.contains("\"method\":\"initialize\"") { r#"{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"opaque-index-provider","version":"1"}}"#.to_owned() } else {
   let text=if line.contains("\"name\":\"rlm_start\"") {
    "{\"session_id\":\"opaque-session\",\"index\":{\"index_status\":\"ready\"}}".to_owned()
   } else if line.contains("\"name\":\"rlm_execute\"") {
    let bytes=fs::read(&db).unwrap();
    let (name,row)=match bytes.as_slice(){b"PRIVATE-A\0Alpha"=>("Alpha",1),b"\xffOpaque-B:Beta:private-layout-v2"=>("Beta",4),_=>panic!("unrecognized provider-owned format")};
    let mut log=fs::OpenOptions::new().create(true).append(true).open(&trace).unwrap();
    writeln!(log,"read-private-index:{name}:rlm_execute").unwrap();
    let rows=format!("[{{\"path\":\"CommonModules/Probe/Ext/Module.bsl\",\"text\":\"{name}\",\"source_type\":\"method\",\"detail\":{{\"line\":{row},\"name\":\"{name}\",\"type\":\"procedure\"}}}}]");
    format!("{{\"stdout\":{}}}",quote(&rows))
   } else if line.contains("\"name\":\"rlm_end\"") {"{}".to_owned()} else {panic!("unexpected supported-interface request: {line}")};
   format!("{{\"content\":[{{\"type\":\"text\",\"text\":{}}}]}}",quote(&text))
  };
  println!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{result}}}"); io::stdout().flush().unwrap();
 }
}"###).unwrap();
    assert!(Command::new("rustc")
        .arg("--edition=2021")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap()
        .success());
    executable
}

fn normalize_service_access(snapshot: &mut [(PathBuf, Option<Vec<u8>>)], record: &Path) {
    let bytes = snapshot
        .iter_mut()
        .find(|(path, _)| path == record)
        .unwrap()
        .1
        .as_mut()
        .unwrap();
    let mut value: Value = serde_json::from_slice(bytes).unwrap();
    assert!(value["last_access_at"].as_u64().is_some());
    value["last_access_at"] = json!(0);
    *bytes = serde_json::to_vec(&value).unwrap();
}

pub(crate) fn with_provider(context: &WorkspaceContext, format: u8, call: impl FnOnce()) {
    let source_root = context.workspace_root.join("src");
    write_ready_rlm_status_for_current_source(context, &source_root);
    let build = usable_index_build(context, &source_root).unwrap().unwrap();
    let bytes: &[u8] = if format == 0 {
        b"PRIVATE-A\0Alpha"
    } else {
        b"\xffOpaque-B:Beta:private-layout-v2"
    };
    fs::write(&build.db_path, bytes).unwrap();
    let fixture = context.workspace_root.join("provider-fixture");
    let executable = compile_provider(&fixture);
    let trace = fixture.join("protocol.trace");
    let identity = WorkspaceServiceIdentity::new(context, &source_root).unwrap();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let record = test_record(
        &identity,
        listener.local_addr().unwrap().port(),
        CoreIdentity::production().as_str(),
    );
    write_record(&identity, record.clone());
    // The manager opens this empty coordination file while reusing the record.
    let lock_path = identity.service_dir.join("service.lock");
    let relative_lock = lock_path
        .strip_prefix(&context.workspace_root)
        .unwrap()
        .to_owned();
    fs::write(&lock_path, b"").unwrap();
    let relative_record = identity
        .service_dir
        .join("service.json")
        .strip_prefix(&context.workspace_root)
        .unwrap()
        .to_owned();
    let mut runtime = new_runtime(identity, &record);
    runtime.test_set_rlm_starter(Arc::new({
        let executable = executable.clone();
        let db = build.db_path.clone();
        let trace = trace.clone();
        move |_, source_root, build_directory, build_id, cancellation| {
            assert_eq!(build_directory, db.parent().unwrap());
            let mut command = Command::new(&executable);
            command.args([&db, &trace]);
            Ok(RlmMcpSession::test_from_transport(
                PersistentMcpSession::start_with_command(command, cancellation)?,
                source_root.to_owned(),
                build_id.to_owned(),
                None,
            ))
        }
    }));
    let runtime = Arc::new(runtime);
    let server_runtime = runtime.clone();
    let _guard = ServiceGuard {
        runtime,
        server: Some(thread::spawn(move || {
            serve_workspace_service(
                listener,
                server_runtime,
                Arc::new(SystemWorkspaceServiceOperationExecutor),
                Duration::from_secs(120),
                SERVICE_REQUEST_HEADER_TIMEOUT,
                SERVICE_SHUTDOWN_GRACE,
            )
        })),
    };
    let mut before = unica_workspace::actor::test_support::snapshot_tree(&context.workspace_root);
    before.retain(|(path, _)| path != &relative_lock);
    normalize_service_access(&mut before, &relative_record);
    call();
    // Force the legitimate heartbeat across a clock tick without a sleep.
    update_service_record_last_access(&_guard.runtime, record.last_access_at + 1);
    assert_eq!(
        fs::read(&build.db_path).unwrap(),
        bytes,
        "consumer rewrote private index"
    );
    let expected = if format == 0 { "Alpha" } else { "Beta" };
    assert_eq!(
        fs::read_to_string(&trace).unwrap(),
        format!("read-private-index:{expected}:rlm_execute\n")
    );
    let observed: Value =
        serde_json::from_slice(&fs::read(context.workspace_root.join(&relative_record)).unwrap())
            .unwrap();
    assert_eq!(
        observed["last_access_at"],
        record.last_access_at + 1,
        "the controlled heartbeat must update the real owned record"
    );
    let mut after = unica_workspace::actor::test_support::snapshot_tree(&context.workspace_root);
    normalize_service_access(&mut after, &relative_record);
    // Only the managed provider trace and the manager's exact coordination
    // lock may change. The service record normalizes only last_access_at;
    // all its other fields and the complete provider-state tree are compared.
    let lock = fs::read_to_string(&lock_path).unwrap();
    let mut lines = lock.lines();
    assert_eq!(lines.next().unwrap(), format!("pid={}", std::process::id()));
    assert!(lines
        .next()
        .unwrap()
        .strip_prefix("started_at=")
        .unwrap()
        .parse::<u64>()
        .is_ok());
    assert!(lines.next().is_none());
    after.retain(|(path, _)| {
        path != Path::new("provider-fixture/protocol.trace") && path != &relative_lock
    });
    let changed: Vec<_> = after
        .iter()
        .filter(|entry| !before.contains(entry))
        .map(|(path, _)| path)
        .collect();
    let removed: Vec<_> = before
        .iter()
        .filter(|entry| !after.contains(entry))
        .map(|(path, _)| path)
        .collect();
    assert!(
        changed.is_empty() && removed.is_empty(),
        "unexpected consumer writes/copies: changed={changed:?}, removed={removed:?}"
    );
}
