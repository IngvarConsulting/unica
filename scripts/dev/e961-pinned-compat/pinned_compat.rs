use super::*;
use crate::infrastructure::workspace_index::UsableIndexBuild;
use std::collections::BTreeMap;
use std::ffi::OsStr;

type Check<T = ()> = Result<T, String>;
const MODULE: &str = "CommonModules/SearchProbe/Ext/Module.bsl";
fn plugin() -> Check<PathBuf> {
    normalize_path_identity(&PathBuf::from(env::var_os("UNICA_PLUGIN_ROOT").ok_or("missing isolated plugin")?))
}

fn selected_bsl() -> bool { cfg!(windows) }

fn live_cwd_witness(root: &Path, context: &WorkspaceContext, source: &Path, pid: u32, provider: &str) -> Check {
    #[cfg(target_os = "linux")]
    {
        let actual = normalize_path_identity(&fs::read_link(format!("/proc/{pid}/cwd")).map_err(|e| e.to_string())?)?;
        for excluded in [context.workspace_root.as_path(), source] {
            ensure(!crate::infrastructure::platform::filesystem::path_starts_with_host_root(
                &actual, &normalize_path_identity(excluded)?), "live engine cwd entered workspace/source")?;
        }
        event(root, "live-cwd", json!({"provider":provider,"pid":pid,"observedVia":"procfs","cwd":actual}))?;
    }
    #[cfg(windows)]
    {
        // A positive round trip while the real engine remains warm observes
        // retention of the owned workspace; it does not inspect remote PEBs.
        let moved = root.join(format!("workspace-moved-{provider}-{pid}"));
        ensure(!moved.exists(), "rename destination was previously used")?;
        fs::rename(&context.workspace_root, &moved).map_err(|e| format!("live workspace rename: {e}"))?;
        let restored = fs::rename(&moved, &context.workspace_root);
        event(root, "live-workspace-rename", json!({"provider":provider,"pid":pid,"moved":moved,"restored":restored.is_ok()}))?;
        restored.map_err(|e| format!("restore owned workspace after rename: {e}"))?;
        ensure(source.exists(), "source did not return after workspace rename")?;
    }
    Ok(())
}

fn ensure(ok: bool, reason: impl Into<String>) -> Check {
    if ok { Ok(()) } else { Err(reason.into()) }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn event(root: &Path, kind: &str, data: Value) -> Check {
    let mut file = OpenOptions::new().create(true).append(true)
        .open(root.join("events.jsonl")).map_err(|e| e.to_string())?;
    writeln!(file, "{}", json!({"kind": kind, "data": data})).map_err(|e| e.to_string())
}

fn snapshot(root: &Path) -> Check<BTreeMap<String, String>> {
    fn walk(base: &Path, at: &Path, out: &mut BTreeMap<String, String>) -> Check {
        for entry in fs::read_dir(at).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            let name = path.strip_prefix(base).map_err(|e| e.to_string())?.display().to_string();
            ensure(!metadata.file_type().is_symlink(), "source gained a symlink")?;
            if metadata.is_dir() {
                out.insert(name, "directory".into());
                walk(base, &path, out)?;
            } else {
                ensure(metadata.is_file(), "source gained a special file")?;
                out.insert(name, digest(&fs::read(&path).map_err(|e| e.to_string())?));
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out)?;
    Ok(out)
}

fn budget(deadline: Instant, cap: Duration) -> Check<Duration> {
    let left = deadline.saturating_duration_since(Instant::now());
    ensure(!left.is_zero(), "compatibility deadline expired")?;
    Ok(left.min(cap))
}

fn command_evidence(command: &Command) -> Value {
    json!({
        "program": command.get_program().to_string_lossy(),
        "args": command.get_args().map(|v| v.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "cwd": command.get_current_dir(),
        "explicitEnv": command.get_envs().map(|(k,v)| (k.to_string_lossy().into_owned(), v.map(|x| x.to_string_lossy().into_owned()))).collect::<BTreeMap<_,_>>()
    })
}

fn command_argument(command: &Command, flag: &str) -> Check<PathBuf> {
    let arguments = command.get_args().collect::<Vec<_>>();
    let positions = arguments.iter().enumerate().filter_map(|(index, value)|
        (*value == OsStr::new(flag)).then_some(index)).collect::<Vec<_>>();
    ensure(positions.len() == 1, format!("one explicit {flag} required"))?;
    arguments.get(positions[0] + 1).map(|value| PathBuf::from(*value)).ok_or_else(|| format!("{flag} value missing"))
}

fn external(command: &Command, context: &WorkspaceContext, source: &Path) -> Check {
    let directory = normalize_path_identity(command.get_current_dir().ok_or("missing explicit cwd")?)?;
    ensure(directory.is_dir(), "cwd is not an existing directory")?;
    for root in [context.workspace_root.as_path(), source] {
        ensure(!crate::infrastructure::platform::filesystem::path_starts_with_host_root(
            &directory, &normalize_path_identity(root)?), "configured cwd is inside workspace/source")?;
    }
    Ok(())
}

enum RealSession {
    Bsl(PersistentMcpSession),
    Rlm(RlmMcpSession),
}

struct SessionOwner {
    session: RealSession,
    root: PathBuf,
    label: String,
    cleanup_attempted: bool,
}

impl SessionOwner {
    fn transport(&mut self) -> &mut PersistentMcpSession {
        match &mut self.session { RealSession::Bsl(s) => s, RealSession::Rlm(s) => &mut s.transport }
    }

    fn close(&mut self) -> Check {
        self.cleanup_attempted = true;
        if let RealSession::Rlm(session) = &mut self.session {
            session.close_logical_session(&ElapsedBudgetDeadline::new(Duration::from_secs(2)), &CancellationToken::new());
        }
        let session = self.transport();
        let pid = session.child.id();
        let retired = session.retire_for_replacement(
            &ElapsedBudgetDeadline::new(Duration::from_secs(5)), &CancellationToken::new());
        let reaped = session.child.poll_owned_leader_exit();
        let reader_deadline = Instant::now() + Duration::from_secs(2);
        while !session.reader_terminal.load(Ordering::Acquire) && Instant::now() < reader_deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let reader_terminal = session.reader_terminal.load(Ordering::Acquire);
        let good = retired.is_ok() && matches!(&reaped, Ok(true)) && reader_terminal;
        event(&self.root, "session-cleanup", json!({"label": self.label, "pid": pid,
            "retirement": retired, "ownedLeaderReaped": reaped,
            "readerTerminal": reader_terminal, "confirmed": good}))?;
        ensure(good, "provider cleanup remains unconfirmed")
    }
}

impl Drop for SessionOwner {
    fn drop(&mut self) {
        if !self.cleanup_attempted { let _ = self.close(); }
    }
}

struct IndexOwner {
    tracker: Arc<SessionTeardownLifecycle>,
    cancellation: CancellationToken,
    root: PathBuf,
    cleanup_attempted: bool,
}

impl IndexOwner {
    fn close(&mut self) -> Check {
        self.cleanup_attempted = true;
        self.cancellation.cancel();
        self.tracker.close();
        let drained = self.tracker.drain(Duration::from_secs(10));
        event(&self.root, "index-cleanup", json!({"drained": drained}))?;
        ensure(drained, "index worker cleanup remains pending")
    }
}

impl Drop for IndexOwner {
    fn drop(&mut self) { if !self.cleanup_attempted { let _ = self.close(); } }
}

fn combine(outcome: Check, cleanup: Check) -> Check {
    match (outcome, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
        (Err(e), Err(c)) => Err(format!("{e}; cleanup: {c}")),
    }
}

fn assert_bsl_hit(text: &str, source: &Path, symbol: &str) -> Check {
    let mut matched = false;
    for line in text.lines().map(str::trim).filter(|line| line.starts_with('#')) {
        let (_, after_modality) = line.split_once("] ").ok_or("BSL hit has no modality")?;
        let (location, named_kind) = after_modality.split_once(" :: ").ok_or("BSL hit has no symbol")?;
        let (path, lines) = location.rsplit_once(':').ok_or("BSL hit has no line")?;
        let first = lines.split('-').next().ok_or("BSL hit line is empty")?.parse::<usize>().map_err(|e| e.to_string())?;
        let (name, kind) = named_kind.strip_suffix(')').ok_or("BSL hit kind is missing")?
            .rsplit_once(" (").ok_or("BSL hit kind separator is missing")?;
        let path = Path::new(path.trim());
        let actual = normalize_path_identity(&if path.is_absolute() { path.to_path_buf() } else { source.join(path) })?;
        ensure(actual == normalize_path_identity(&source.join(MODULE))?, "BSL hit came from another source/path")?;
        ensure(name == symbol && first == 1 && !kind.is_empty(), "BSL hit did not identify the actual exported procedure at line 1")?;
        matched = true;
    }
    ensure(matched, "BSL returned no source hit; query echo is insufficient")
}

fn bsl_ready(session: &mut PersistentMcpSession, root: &Path, label: &str, symbol: &str, deadline: Instant) -> Check<String> {
    for attempt in 0usize.. {
        let arguments = json!({"action":"search_code", "query":symbol, "limit":5});
        event(root, "bsl-request", json!({"label":label,"symbol":symbol,"attempt":attempt,"arguments":arguments}))?;
        let output = match session.call("search", arguments,
            budget(deadline, Duration::from_secs(30))?, &CancellationToken::new()) {
            Ok(output) => output,
            Err(error) => {
                event(root, "bsl-error", json!({"label":label,"symbol":symbol,"attempt":attempt,"error":error}))?;
                return Err(error);
            }
        };
        event(root, "bsl-response", json!({"label":label,"attempt":attempt,"text":output.result_text,"stderr":output.stderr}))?;
        let pending = serde_json::from_str::<Value>(output.result_text.trim()).ok()
            .filter(|v| v.get("status").and_then(Value::as_str) == Some("not_ready"));
        let Some(pending) = pending else { return Ok(output.result_text); };
        let millis = pending.get("retry_after_ms").and_then(Value::as_u64)
            .ok_or("explicit BSL not_ready has no retry_after_ms")?;
        let delay = Duration::from_millis(millis);
        ensure(delay < budget(deadline, Duration::MAX)?, "BSL dependency did not become ready within the deadline")?;
        thread::sleep(delay);
    }
    unreachable!("unbounded range returns through readiness or deadline")
}

fn run_bsl(root: &Path, context: &WorkspaceContext, source: &Path, symbol: &str, deadline: Instant) -> Check<PathBuf> {
    let program = resolve_bundled_tool(&plugin()?, "bsl-analyzer", true)?.program;
    let command = bsl_analyzer_command(&program, context, source)?;
    external(&command, context, source)?;
    let arguments = command.get_args().map(|v| v.to_os_string()).collect::<Vec<_>>();
    let cache = command_argument(&command, "--cache-dir")?;
    ensure(cache.starts_with(root) && !cache.starts_with(source), "BSL cache escaped the owned external cache")?;
    event(root, "bsl-command", json!({"symbol":symbol,"command":command_evidence(&command)}))?;
    let session = PersistentMcpSession::start(context, source, &CancellationToken::new())?;
    let mut owned = SessionOwner { session: RealSession::Bsl(session), root: root.to_path_buf(), label:symbol.into(), cleanup_attempted:false };
    let result = (|| {
        let session = owned.transport();
        let pid = session.child.id();
        event(root, "session-start", json!({"provider":"bsl","symbol":symbol,"pid":pid}))?;
        let first = bsl_ready(session, root, "first", symbol, deadline)?;
        assert_bsl_hit(&first, source, symbol)?;
        ensure(session.is_reusable(), "BSL process was not warm after first call")?;
        live_cwd_witness(root, context, source, pid, "bsl")?;
        let second = bsl_ready(session, root, "warm", symbol, deadline)?;
        assert_bsl_hit(&second, source, symbol)?;
        ensure(session.child.id() == pid && session.is_reusable(), "BSL warm call replaced/exited its process")?;
        let again = bsl_analyzer_command(&program, context, source)?;
        ensure(again.get_args().map(|v|v.to_os_string()).collect::<Vec<_>>() == arguments, "BSL explicit source/cache arguments changed")?;
        event(root, "bsl-warm", json!({"symbol":symbol,"samePid":pid,"semanticHitVerified":true}))
    })();
    combine(result, owned.close())?;
    Ok(cache)
}

fn assert_rlm_hit(text: &str, source: &Path, symbol: &str) -> Check {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let rows = value.as_array().ok_or("RLM search did not return the supported row array")?;
    let mut matched = false;
    for row in rows {
        let name = row.pointer("/detail/name").and_then(Value::as_str);
        if name != Some(symbol) { continue; }
        let path = Path::new(row.get("path").and_then(Value::as_str).ok_or("RLM hit has no path")?);
        let actual = normalize_path_identity(&if path.is_absolute() { path.to_path_buf() } else { source.join(path) })?;
        ensure(actual == normalize_path_identity(&source.join(MODULE))?, "RLM hit came from another source")?;
        ensure(row.pointer("/detail/line").and_then(Value::as_u64) == Some(1), "RLM hit line is not 1")?;
        matched = true;
    }
    ensure(matched, "RLM returned no actual source symbol/path/line")
}

fn build_index(root: &Path, context: &WorkspaceContext, source: &Path, index: &IndexOwner, deadline: Instant) -> Check<UsableIndexBuild> {
    let runner = SystemIndexRunner::tracked(index.tracker.clone());
    let service = WorkspaceIndexService::with_runner(&runner);
    ensure(service.active_build(context, source)?.is_none(), "fresh source already has an active index")?;
    let report = service.start_isolated_build(context, source, &index.cancellation);
    event(root, "index-start", json!({"source":source,"maintenance":report.maintenance,"warnings":report.warnings}))?;
    ensure(matches!(&report.maintenance, IndexMaintenanceStatus::Queued | IndexMaintenanceStatus::Running | IndexMaintenanceStatus::Completed),
        "real index worker was not admitted")?;
    ensure(index.tracker.drain(budget(deadline, Duration::from_secs(60))?), "real index worker did not finish within deadline")?;
    let maintenance = service.maintenance_status(context, source);
    let build = service.active_build(context, source)?;
    // The worker releases/unregisters its live maintenance observation before
    // drain returns. Completion is its joined worker plus a newly usable build;
    // an absent ephemeral observation is not a failed engine status.
    event(root, "index-observed", json!({"source":source,"trackedWorkerDrained":true,"maintenanceAfterDrain":maintenance,"build":build.as_ref().map(|b| json!({"id":b.id,"directory":b.directory,"db":b.db_path}))}))?;
    let build = build.ok_or("real index build produced no usable active generation")?;
    ensure(build.directory.starts_with(root) && build.db_path.starts_with(&build.directory), "index generation escaped owned root")?;
    Ok(build)
}

fn run_rlm(root: &Path, context: &WorkspaceContext, source: &Path, symbol: &str, index: &IndexOwner, deadline: Instant) -> Check<String> {
    let build = build_index(root, context, source, index, deadline)?;
    let program = resolve_bundled_tool(&plugin()?, "rlm-bsl-mcp", true)?.program;
    let command = rlm_transport_command_for_build(program, context, source, &build.directory)?;
    external(&command, context, source)?;
    ensure(command.get_envs().find(|(k,_)| *k==OsStr::new("RLM_INDEX_DIR")).and_then(|(_,v)|v)==Some(build.directory.as_os_str()), "RLM lost native pinned generation environment")?;
    event(root, "rlm-command", json!({"symbol":symbol,"command":command_evidence(&command)}))?;
    let session = RlmMcpSession::start_pinned(context, source, &build.directory, &build.id, &CancellationToken::new())?;
    let mut owned = SessionOwner { session:RealSession::Rlm(session), root:root.to_path_buf(), label:symbol.into(), cleanup_attempted:false };
    let result = (|| {
        let RealSession::Rlm(session) = &mut owned.session else { return Err("wrong session owner".into()); };
        let pid = session.transport.child.id();
        event(root, "session-start", json!({"provider":"rlm","symbol":symbol,"pid":pid}))?;
        let operation = WorkspaceRlmOperation::Search { query:symbol.into(), limit:5 };
        event(root,"rlm-request",json!({"symbol":symbol,"phase":"first","operation":operation}))?;
        let first = session.execute(&operation, budget(deadline, Duration::from_secs(30))?, &CancellationToken::new())?;
        event(root,"rlm-response",json!({"symbol":symbol,"phase":"first","text":first.result_text,"stderr":first.stderr,"buildId":first.build_id}))?;
        assert_rlm_hit(&first.result_text,source,symbol)?;
        let session_id = session.session_id.clone().filter(|v|!v.is_empty()).ok_or("RLM did not retain its logical session")?;
        ensure(session.is_reusable(),"RLM first call did not leave a warm process")?;
        live_cwd_witness(root, context, source, pid, "rlm")?;
        event(root,"rlm-request",json!({"symbol":symbol,"phase":"warm","operation":operation}))?;
        let second = session.execute(&operation, budget(deadline,Duration::from_secs(30))?,&CancellationToken::new())?;
        event(root,"rlm-response",json!({"symbol":symbol,"phase":"warm","text":second.result_text,"stderr":second.stderr,"buildId":second.build_id}))?;
        assert_rlm_hit(&second.result_text,source,symbol)?;
        ensure(session.transport.child.id()==pid && session.is_reusable() && session.session_id.as_deref()==Some(session_id.as_str()),"RLM warm process/logical session changed")?;
        ensure(session.build_id==build.id && first.build_id.as_deref()==Some(build.id.as_str()) && second.build_id.as_deref()==Some(build.id.as_str()),"RLM warm call changed its pinned build")?;
        let runner=SystemIndexRunner::tracked(index.tracker.clone());
        let active=WorkspaceIndexService::with_runner(&runner).active_build(context,source)?;
        ensure(active.as_ref()==Some(&build),"RLM active generation changed during warm repeat")?;
        event(root,"rlm-warm",json!({"symbol":symbol,"pid":pid,"sessionId":session_id,"buildId":build.id,"generation":build.directory,"semanticHitVerified":true}))
    })();
    combine(result,owned.close())?;
    Ok(build.id)
}

fn run(root: &Path) -> Check {
    ensure(!root.join("workspace").exists(),"fixture root was previously used")?;
    let workspace=root.join("workspace");
    let control=root.join("control");
    let cwd=workspace.join("nested/deeper");
    let cache=root.join("cache");
    for directory in [&control,&cwd,&cache] { fs::create_dir_all(directory).map_err(|e|e.to_string())?; }
    let context=WorkspaceContext { cwd, workspace_root:workspace, cache_root:cache, workspace_epoch:1 };
    let fixture=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/acceptance/workspace-symbol/src");
    let mut sources=Vec::new();
    for (name,symbol) in [("alpha","E1PinnedAlpha"),("beta","E1PinnedBeta")] {
        let source=context.workspace_root.join(name);
        for relative in ["Configuration.xml","CommonModules/SearchProbe.xml",MODULE] {
            let target=source.join(relative);
            fs::create_dir_all(target.parent().ok_or("fixture parent missing")?).map_err(|e|e.to_string())?;
            fs::copy(fixture.join(relative),&target).map_err(|e|e.to_string())?;
        }
        fs::write(source.join(MODULE),format!("Procedure {symbol}() Export\nEndProcedure\n")).map_err(|e|e.to_string())?;
        sources.push((source,symbol));
    }
    let before=sources.iter().map(|(p,_)|snapshot(p)).collect::<Check<Vec<_>>>()?;
    event(root,"sources-before",json!(before))?;
    let deadline=Instant::now()+Duration::from_secs(180);
    let mut index=IndexOwner { tracker:Arc::new(SessionTeardownLifecycle::default()), cancellation:CancellationToken::new(),root:root.to_path_buf(),cleanup_attempted:false };
    let result=(|| {
        if selected_bsl() {
            let mut caches=Vec::new();
            for (source,symbol) in &sources { caches.push(run_bsl(root,&context,source,symbol,deadline)?); }
            ensure(caches[0]!=caches[1],"two BSL sources unexpectedly share one cache")?;
        }
        let mut builds=Vec::new();
        for (source,symbol) in &sources { builds.push(run_rlm(root,&context,source,symbol,&index,deadline)?); }
        ensure(builds[0]!=builds[1],"two sources unexpectedly share one build identity")?;
        let after=sources.iter().map(|(p,_)|snapshot(p)).collect::<Check<Vec<_>>>()?;
        event(root,"sources-after",json!(after))?;
        ensure(before==after,"real providers modified source files/directories")
    })();
    combine(result,index.close())
}

#[test]
#[ignore = "manual missing-platform witness; isolated verified runtime and owned driver required"]
fn real_pinned_bsl_and_rlm_keep_explicit_sources_and_warm_generation_with_external_cwd() -> Check {
    let root=normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E1_COMPAT_ROOT").ok_or("missing owned root")?))?;
    let token=env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e|e.to_string())?;
    let owner:Value=serde_json::from_slice(&fs::read(root.join("owner.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    ensure(owner.get("token").and_then(Value::as_str)==Some(token.as_str()),"owned root token differs")?;
    let plugin = plugin()?;
    ensure(normalize_path_identity(&find_plugin_root(&root).ok_or("plugin lookup failed")?)? == plugin,
        "plugin lookup did not select the isolated verified runtime")?;
    let plan:Value = serde_json::from_slice(&fs::read(root.join("engines.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let tools = plan.get("tools").and_then(Value::as_array).ok_or("engine plan has no tools")?;
    ensure(tools.len() == if selected_bsl() { 3 } else { 2 }, "unexpected engine inventory")?;
    for entry in tools {
        let name = entry.get("name").and_then(Value::as_str).ok_or("tool name missing")?;
        ensure(matches!(name,"rlm-bsl-mcp"|"rlm-bsl-index") || (selected_bsl() && name=="bsl-analyzer"), "unexpected tool")?;
        let hash = entry.get("sha256").and_then(Value::as_str).ok_or("tool SHA missing")?;
        let tool = resolve_bundled_tool(&plugin,name,true)?;
        ensure(digest(&fs::read(&tool.program).map_err(|e|e.to_string())?)==hash,
            format!("actual {name} differs from the verified staged archive member"))?;
        event(&root,"tool",json!({"name":name,"program":tool.program,"sha256":hash}))?;
    }
    let result=run(&root);
    event(&root,"final",json!({"status":if result.is_ok(){"PASS"}else{"FAIL"},"error":result.as_ref().err()}))?;
    result
}


#[test]
#[ignore = "manual owned process driver for a single missing-platform compatibility observation"]
fn owned_real_platform_compatibility_driver() -> Check {
    let root = normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E1_COMPAT_ROOT").ok_or("missing root")?))?;
    let mut command = Command::new(env::current_exe().map_err(|e|e.to_string())?);
    command.args(["--ignored", "--exact",
        "infrastructure::workspace_services::persistent_provider_cwd_pinned_compat::real_pinned_bsl_and_rlm_keep_explicit_sources_and_warm_generation_with_external_cwd",
        "--nocapture", "--test-threads=1"])
        .current_dir(root.join("control"));
    let mut child = ManagedChild::spawn_process(command, Some(Duration::from_secs(240)), CancellationToken::new())?;
    let pid = child.id();
    let output = child.wait_for_output()?;
    fs::write(root.join("inner.stdout"), &output.stdout).map_err(|e|e.to_string())?;
    fs::write(root.join("inner.stderr"), &output.stderr).map_err(|e|e.to_string())?;
    let reaped = child.poll_owned_leader_exit()?;
    event(&root,"driver-final",json!({"pid":pid,"statusSuccess":output.status_success,
        "timedOut":output.timed_out,"cancelled":output.cancelled,
        "stdoutTruncated":output.stdout_truncated,"stderrTruncated":output.stderr_truncated,
        "ownedLeaderReaped":reaped}))?;
    ensure(output.status_success && !output.timed_out && !output.cancelled &&
        !output.stdout_truncated && !output.stderr_truncated && reaped,
        "owned real-engine fixture failed or did not finish cleanly")
}
