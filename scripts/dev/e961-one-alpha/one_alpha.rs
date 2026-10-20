//! One fresh Alpha causal observation. Temporary control payload, not product code.
use super::*;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::panic::{catch_unwind, AssertUnwindSafe};

#[path = "windows_probe.rs"]
mod windows_probe;
#[path = "owned_handles.rs"]
mod owned_handles;

type Check<T = ()> = Result<T, String>;
const MODULE: &str = "CommonModules/SearchProbe/Ext/Module.bsl";
const SYMBOL: &str = "E1PinnedAlpha";
const INNER: &str = "infrastructure::workspace_services::e961_one_alpha::one_alpha_causal_observation";
const EXPECTED_BSL: &str = "3736453d18dd7ea12634b6a78839eed1fd3a8d821819b752b39f51afd91869d9";

fn ensure(ok: bool, reason: impl Into<String>) -> Check {
    if ok { Ok(()) } else { Err(reason.into()) }
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn event(root: &Path, kind: &str, data: Value) -> Check {
    let mut file = OpenOptions::new().create(true).append(true).open(root.join("events.jsonl")).map_err(|e| e.to_string())?;
    writeln!(file, "{}", json!({"kind":kind,"data":data})).map_err(|e| e.to_string())
}
fn io_error(error: &std::io::Error) -> Value {
    json!({"kind":format!("{:?}",error.kind()),"osError":error.raw_os_error(),"message":error.to_string()})
}
#[derive(Default)]
struct Report { first: Option<Value>, secondary: Vec<Value>, capture: Vec<Value> }
impl Report {
    fn fail(&mut self, stage: &str, error: Value) {
        let item=json!({"stage":stage,"error":error});
        if self.first.is_none() { self.first=Some(item); } else { self.secondary.push(item); }
    }
    fn capture_error(&mut self, stage: &str, error: impl ToString) {
        self.capture.push(json!({"stage":stage,"error":error.to_string()}));
    }
    fn emit(&mut self, root: &Path, kind: &str, data: Value) {
        if let Err(error)=event(root,kind,data) { self.capture_error(&format!("write-{kind}"),error); }
    }
    fn status(&self) -> &'static str {
        if !self.capture.is_empty() { "CAPTURE_INCOMPLETE" } else if self.first.is_some() { "OBSERVED_FAIL" } else { "PASS" }
    }
}
fn snapshot(root: &Path) -> Check<BTreeMap<String,String>> {
    fn walk(base:&Path,at:&Path,out:&mut BTreeMap<String,String>) -> Check {
        for entry in fs::read_dir(at).map_err(|e|e.to_string())? {
            let entry=entry.map_err(|e|e.to_string())?; let path=entry.path();
            let meta=fs::symlink_metadata(&path).map_err(|e|e.to_string())?;
            let name=path.strip_prefix(base).map_err(|e|e.to_string())?.display().to_string();
            ensure(!meta.file_type().is_symlink(),"source gained a symlink")?;
            if meta.is_dir() { out.insert(name,"directory".into()); walk(base,&path,out)?; }
            else { ensure(meta.is_file(),"source gained special file")?;out.insert(name,digest(&fs::read(path).map_err(|e|e.to_string())?)); }
        }
        Ok(())
    }
    let mut result=BTreeMap::new();walk(root,root,&mut result)?;Ok(result)
}
fn budget(deadline:Instant,cap:Duration)->Check<Duration> {
    let left=deadline.saturating_duration_since(Instant::now());ensure(!left.is_zero(),"causal deadline expired")?;Ok(left.min(cap))
}
fn command_evidence(command:&Command)->Value {
    json!({"program":command.get_program().to_string_lossy(),"args":command.get_args().map(|v|v.to_string_lossy().into_owned()).collect::<Vec<_>>(),"cwd":command.get_current_dir(),"explicitEnv":command.get_envs().map(|(k,v)|(k.to_string_lossy().into_owned(),v.map(|x|x.to_string_lossy().into_owned()))).collect::<BTreeMap<_,_>>()})
}
fn command_argument(command:&Command,flag:&str)->Check<PathBuf> {
    let args=command.get_args().collect::<Vec<_>>();
    let positions=args.iter().enumerate().filter_map(|(i,v)|(*v==OsStr::new(flag)).then_some(i)).collect::<Vec<_>>();
    ensure(positions.len()==1,format!("one explicit {flag} required"))?;
    args.get(positions[0]+1).map(|v|PathBuf::from(*v)).ok_or_else(||format!("{flag} missing"))
}
fn external(command:&Command,context:&WorkspaceContext,source:&Path)->Check {
    let cwd=normalize_path_identity(command.get_current_dir().ok_or("missing configured cwd")?)?;
    for root in [context.workspace_root.as_path(),source] {
        ensure(!crate::infrastructure::platform::filesystem::path_starts_with_host_root(&cwd,&normalize_path_identity(root)?),"configured cwd entered workspace/source")?;
    }
    Ok(())
}

struct Fixture { context:WorkspaceContext, source:PathBuf, identity:windows_probe::Identity, names:Vec<PathBuf> }
impl Fixture {
    fn prepare(root:&Path)->Check<Self> {
        let workspace=root.join("workspace");fs::create_dir(&workspace).map_err(|e|e.to_string())?;
        let cwd=workspace.join("nested/deeper");let cache=root.join("cache");
        fs::create_dir_all(&cwd).map_err(|e|e.to_string())?;fs::create_dir_all(&cache).map_err(|e|e.to_string())?;
        let source=workspace.join("alpha");
        let original=Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/acceptance/workspace-symbol/src");
        for relative in ["Configuration.xml","CommonModules/SearchProbe.xml",MODULE] {
            let target=source.join(relative);fs::create_dir_all(target.parent().ok_or("fixture parent missing")?).map_err(|e|e.to_string())?;
            fs::copy(original.join(relative),target).map_err(|e|e.to_string())?;
        }
        fs::write(source.join(MODULE),format!("Procedure {SYMBOL}() Export\nEndProcedure\n")).map_err(|e|e.to_string())?;
        let identity=windows_probe::directory_identity(&workspace).map_err(|e|e.to_string())?;
        let names=vec![workspace.clone(),root.join("workspace-baseline-moved"),root.join("workspace-live-moved"),root.join("workspace-post-moved")];
        Ok(Self {context:WorkspaceContext {cwd,workspace_root:workspace,cache_root:cache,workspace_epoch:1},source,identity,names})
    }
    fn locate(&self)->Check<PathBuf> {
        let mut matches=Vec::new();
        for name in &self.names {
            match windows_probe::directory_identity(name) {
                Ok(identity) if identity==self.identity=>matches.push(name.clone()),
                Ok(_)=>{},
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
                Err(error)=>return Err(format!("cannot establish directory identity at {}: {error}",name.display())),
            }
        }
        ensure(matches.len()==1,"workspace physical location is missing or ambiguous; preserve")?;
        matches.pop().ok_or("workspace location missing".into())
    }
    fn metadata(&self,root:&Path,phase:&str,report:&mut Report) {
        let located=self.locate();
        match located {
            Ok(location)=>{
                let paths=[("workspace",location.clone()),("source",location.join("alpha")),("parent",root.to_path_buf())];
                for (label,path) in paths {
                    match windows_probe::directory_evidence(&path) {
                        Ok(data)=>report.emit(root,"directory-evidence",json!({"phase":phase,"label":label,"path":path,"evidence":data})),
                        Err(error)=>report.capture_error(&format!("{phase}-{label}-metadata"),error),
                    }
                }
            }
            Err(error)=>report.capture_error(&format!("{phase}-location"),error),
        }
    }
    fn rename(&self,root:&Path,phase:&str,from:&Path,to:&Path,report:&mut Report)->bool {
        let pre=(||->Check {
            ensure(self.locate()?==from,"rename source is not the physically identified owned workspace")?;
            match fs::symlink_metadata(to) {
                Err(error) if error.kind()==std::io::ErrorKind::NotFound=>Ok(()),
                Ok(_)=>Err("rename destination is occupied; preserve".into()),
                Err(error)=>Err(format!("cannot establish missing destination: {error}")),
            }
        })();
        if let Err(error)=pre { report.capture_error(&format!("{phase}-precondition"),error);return false; }
        let operation=fs::rename(from,to);let location=self.locate();
        report.emit(root,"rename",json!({"phase":phase,"from":from,"to":to,"result":operation.as_ref().map(|_|()).map_err(io_error),"location":location.as_ref().map(|p|p.display().to_string()).map_err(Clone::clone)}));
        if let Err(error)=&operation { report.fail(phase,io_error(error)); }
        if let Err(error)=&location { report.capture_error(&format!("{phase}-location-after"),error); }
        operation.is_ok() && location.as_ref().is_ok_and(|path|path==to)
    }
    fn roundtrip(&self,root:&Path,phase:&str,moved_index:usize,report:&mut Report)->bool {
        if !self.rename(root,&format!("{phase}-forward"),&self.context.workspace_root,&self.names[moved_index],report) { return false; }
        self.rename(root,&format!("{phase}-restore"),&self.names[moved_index],&self.context.workspace_root,report)
    }
}

struct SessionOwner { session:PersistentMcpSession, root:PathBuf, attempted:bool }
impl SessionOwner {
    fn close(&mut self)->(bool,Value) {
        self.attempted=true;
        let pid=self.session.child.id();
        let retirement=self.session.retire_for_replacement(&ElapsedBudgetDeadline::new(Duration::from_secs(5)),&CancellationToken::new());
        let reaped=self.session.child.poll_owned_leader_exit();
        let until=Instant::now()+Duration::from_secs(2);
        while !self.session.reader_terminal.load(Ordering::Acquire)&&Instant::now()<until {thread::sleep(Duration::from_millis(10));}
        let terminal=self.session.reader_terminal.load(Ordering::Acquire);
        let confirmed=retirement.is_ok()&&matches!(&reaped,Ok(true))&&terminal;
        (confirmed,json!({"provider":"bsl","pid":pid,"retirement":retirement,"ownedLeaderReaped":reaped,"readerTerminal":terminal,"confirmed":confirmed}))
    }
}
impl Drop for SessionOwner {fn drop(&mut self){if !self.attempted {let (_,data)=self.close();let _=event(&self.root,"session-cleanup-fallback",data);}}}
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

fn bsl_ready(session: &mut PersistentMcpSession, root: &Path, label: &str, symbol: &str, deadline: Instant, report: &mut Report) -> Check<String> {
    for attempt in 0usize.. {
        let arguments = json!({"action":"search_code", "query":symbol, "limit":5});
        report.emit(root, "bsl-request", json!({"label":label,"symbol":symbol,"attempt":attempt,"arguments":arguments}));
        ensure(report.capture.is_empty(),"BSL request evidence write failed")?;
        let output = match session.call("search", arguments,
            budget(deadline, Duration::from_secs(30))?, &CancellationToken::new()) {
            Ok(output) => output,
            Err(error) => {
                report.fail("bsl-first-request",json!(error));
                report.emit(root, "bsl-error", json!({"label":label,"symbol":symbol,"attempt":attempt,"error":error}));
                return Err(error);
            }
        };
        report.emit(root, "bsl-response", json!({"label":label,"attempt":attempt,"text":output.result_text,"stderr":output.stderr}));
        ensure(report.capture.is_empty(),"BSL response evidence write failed")?;
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


fn active(root:&Path,fixture:&Fixture,layout:Option<&windows_probe::ValidatedLayout>,deadline:Instant,owner:&mut Option<SessionOwner>,observer_quiescent:&mut bool,report:&mut Report)->Check {
    fixture.metadata(root,"baseline-before",report);
    ensure(report.capture.is_empty(),"baseline evidence incomplete")?;
    ensure(fixture.roundtrip(root,"baseline",1,report),"pre-engine rename roundtrip failed")?;
    let plugin=normalize_path_identity(&PathBuf::from(env::var_os("UNICA_PLUGIN_ROOT").ok_or("missing isolated plugin")?))?;
    ensure(normalize_path_identity(&find_plugin_root(root).ok_or("isolated plugin lookup failed")?)?==plugin,"runtime lookup did not select isolated pinned plugin")?;
    let program=resolve_bundled_tool(&plugin,"bsl-analyzer",true)?.program;
    ensure(digest(&fs::read(&program).map_err(|e|e.to_string())?)==EXPECTED_BSL,"actual BSL bytes differ")?;
    let command=bsl_analyzer_command(&program,&fixture.context,&fixture.source)?;
    external(&command,&fixture.context,&fixture.source)?;
    let cache=command_argument(&command,"--cache-dir")?;
    ensure(cache.starts_with(root)&&!cache.starts_with(&fixture.context.workspace_root),"BSL cache is not owned external cache")?;
    report.emit(root,"bsl-command",json!({"symbol":SYMBOL,"command":command_evidence(&command),"cache":cache}));
    let session=PersistentMcpSession::start(&fixture.context,&fixture.source,&CancellationToken::new())?;
    *owner=Some(SessionOwner {session,root:root.to_path_buf(),attempted:false});
    let session=&mut owner.as_mut().ok_or("session owner missing")?.session;
    let pid=session.child.id();report.emit(root,"session-start",json!({"provider":"bsl","symbol":SYMBOL,"pid":pid}));
    let first=bsl_ready(session,root,"first",SYMBOL,deadline,report)?;assert_bsl_hit(&first,&fixture.source,SYMBOL)?;
    ensure(session.is_reusable(),"BSL first call did not leave a live process")?;
    report.emit(root,"semantic-hit",json!({"symbol":SYMBOL,"path":fixture.source.join(MODULE),"line":1,"pid":pid}));
    let identity_before=match windows_probe::process_evidence(&session.child) {Ok(value)=>value,Err(error)=>{report.capture_error("live-process-before",&error);return Err(error);}};
    report.emit(root,"live-process-before",identity_before.clone());
    let cwd=match layout {
        Some(validated)=>windows_probe::observe(&session.child,validated,&fixture.context.workspace_root,&fixture.source),
        None=>json!({"status":"Unknown","stage":"calibration","reason":"no validated empirical layout"}),
    };
    if cwd.get("status").and_then(Value::as_str)==Some("ObservedInside") {report.fail("actual-cwd",cwd.clone());}
    report.emit(root,"actual-cwd",cwd);
    fixture.metadata(root,"live-before",report);
    if report.capture.is_empty() {let _=fixture.roundtrip(root,"live",2,report);}
    let identity_after=match windows_probe::process_evidence(&session.child) {Ok(value)=>value,Err(error)=>{report.capture_error("live-process-after",&error);return Err(error);}};
    report.emit(root,"live-process-after",identity_after.clone());
    ensure(identity_before==identity_after,"owned process identity/liveness changed across live rename")?;
    let eligible=report.first.as_ref().is_some_and(|failure|failure["stage"]=="live-forward"&&failure["error"]["osError"]==5);
    if eligible&&report.capture.is_empty() {
        *observer_quiescent=false; // Unknown/unwind blocks post-rename, never the mandatory engine cleanup.
        let (quiescent,observation)=owned_handles::observe(root,fixture,&session.child,deadline);
        *observer_quiescent=quiescent;
        report.emit(root,"handle-observer-disposition",json!({"attempted":true,"workerQuiescent":quiescent,"status":if observation.is_ok(){"Captured"}else{"Unknown"}}));
        match observation {Ok(value)=>report.emit(root,"handle-observation",value),Err(error)=>report.capture_error("handle-observer",error)}
    }

    let again=bsl_analyzer_command(&program,&fixture.context,&fixture.source)?;
    ensure(command_evidence(&command)==command_evidence(&again),"factory arguments/environment changed")?;
    Ok(())
}

fn parent_state()->Value {
    json!({"pid":std::process::id(),"actualCwd":env::current_dir().map_err(|e|e.to_string()),"privateCalibrationEnvPresent":(["UNICA_E961_CALIB_STATE","UNICA_E961_CALIB_A","UNICA_E961_CALIB_B","UNICA_E961_HANDLES_STATE"].map(|name|(name,env::var_os(name).is_some())))})
}
fn run_owned(root:&Path)->Report {
    let mut report=Report::default();
    let parent_before=parent_state();report.emit(root,"parent-before",parent_before.clone());
    let fixture=match Fixture::prepare(root) {Ok(value)=>value,Err(error)=>{report.capture_error("fixture-prepare",error);return report;}};
    let before=match snapshot(&fixture.source) {Ok(value)=>{report.emit(root,"sources-before",json!(value));Some(value)},Err(error)=>{report.capture_error("sources-before",error);None}};
    let deadline=Instant::now()+Duration::from_secs(180);
    let calibration=catch_unwind(AssertUnwindSafe(||windows_probe::calibrate(root,deadline)));
    let layout=match calibration {
        Ok(Ok(value))=>{report.emit(root,"cwd-calibration",value.1);value.0},
        Ok(Err(error))=>{report.capture_error("cwd-calibration-cleanup",error);None},
        Err(_)=>{report.capture_error("cwd-calibration-panic","calibration ownership outcome unconfirmed");None},
    };
    let mut owner=None;let mut observer_quiescent=true;
    if report.capture.is_empty() {
        match catch_unwind(AssertUnwindSafe(||active(root,&fixture,layout.as_ref(),deadline,&mut owner,&mut observer_quiescent,&mut report))) {
            Ok(Err(error))=>if report.first.is_none(){report.fail("active",json!(error));},
            Err(_)=>report.fail("active-panic",json!("panic during Alpha; original stderr retained")),
            Ok(Ok(()))=>{},
        }
    }
    let engine_started=owner.is_some();let mut cleaned=false;
    if let Some(mut owned)=owner.take() {
        match catch_unwind(AssertUnwindSafe(||owned.close())) {
            Ok((confirmed,data))=>{cleaned=confirmed;report.emit(root,"session-cleanup",data);if !confirmed {report.capture_error("session-cleanup","provider retirement/reap/reader remains unconfirmed");}},
            Err(_)=>report.capture_error("session-cleanup-panic","provider ownership outcome unconfirmed"),
        }
    }
    // Parent/context remain alive; post-retire is a distinct phase, never a live retry.
    if engine_started&&cleaned&&observer_quiescent {
        fixture.metadata(root,"post-retire-before",&mut report);
        match fixture.locate() {
            Ok(path) if path==fixture.context.workspace_root=>{let _=fixture.roundtrip(root,"post-retire",3,&mut report);},
            Ok(path)=>{let _=fixture.rename(root,"post-retire-restoration",&path,&fixture.context.workspace_root,&mut report);},
            Err(error)=>report.capture_error("post-retire-location",error),
        }
    } else {report.emit(root,"post-retire-not-run",json!({"engineStarted":engine_started,"cleanupConfirmed":cleaned,"observerQuiescent":observer_quiescent}));}
    // Mandatory finally snapshot, including a failed forward/restore/semantic request.
    match fixture.locate() {
        Ok(path)=>match snapshot(&path.join("alpha")) {
            Ok(after)=>{report.emit(root,"sources-finally",json!({"location":path,"snapshot":after}));if before.as_ref().is_some_and(|value|value!=&after){report.fail("source-preservation",json!("source snapshot changed"));}},
            Err(error)=>report.capture_error("sources-finally",error),
        },
        Err(error)=>report.capture_error("sources-finally-location",error),
    }
    fixture.metadata(root,"finally",&mut report);
    report.emit(root,"preserved-fixture",json!({"policy":"keep owned data; no deletion by this diagnostic","knownLocation":fixture.locate(),"names":fixture.names}));
    let parent_after=parent_state();report.emit(root,"parent-finally",parent_after.clone());
    if parent_before!=parent_after {report.capture_error("parent-context","parent cwd or private calibration env presence changed");}
    report
}

#[test]
#[ignore = "manual one-Alpha causal diagnostic, not gap closure"]
fn one_alpha_causal_observation()->Check {
    let root=normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E1_COMPAT_ROOT").ok_or("missing owned root")?))?;
    let token=env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e|e.to_string())?;
    let owner:Value=serde_json::from_slice(&fs::read(root.join("owner.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
    ensure(owner.get("token").and_then(Value::as_str)==Some(token.as_str()),"owned root token differs")?;
    let report=run_owned(&root);let status=report.status();
    event(&root,"final",json!({"status":status,"firstFailure":report.first,"secondaryFailures":report.secondary,"captureErrors":report.capture}))?;
    ensure(status=="PASS",format!("{status}: first outcome and all finally evidence retained"))
}
#[test]
#[ignore = "manual owned driver; exactly one inner forensic observation"]
fn owned_one_alpha_causal_driver()->Check {
    let root=normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E1_COMPAT_ROOT").ok_or("missing root")?))?;
    let mut command=Command::new(env::current_exe().map_err(|e|e.to_string())?);
    command.args(["--ignored","--exact",INNER,"--nocapture","--test-threads=1"]).current_dir(root.join("control"));
    let mut child=ManagedChild::spawn_process(command,Some(Duration::from_secs(240)),CancellationToken::new())?;
    let pid=child.id();let output=child.wait_for_output()?;
    fs::write(root.join("inner.stdout"),&output.stdout).map_err(|e|e.to_string())?;
    fs::write(root.join("inner.stderr"),&output.stderr).map_err(|e|e.to_string())?;
    let reaped=child.poll_owned_leader_exit()?;
    event(&root,"driver-final",json!({"pid":pid,"statusSuccess":output.status_success,"timedOut":output.timed_out,"cancelled":output.cancelled,"stdoutTruncated":output.stdout_truncated,"stderrTruncated":output.stderr_truncated,"ownedLeaderReaped":reaped}))?;
    ensure(output.status_success&&!output.timed_out&&!output.cancelled&&!output.stdout_truncated&&!output.stderr_truncated&&reaped,"one-Alpha inner outcome was nonzero or incomplete; preserved")
}
#[test]
#[ignore = "owned child-only cwd calibration, never ordinary test selection"]
fn cwd_calibration_child()->Check {windows_probe::calibration_child()}

#[test]
#[ignore = "owned post-failure PSS/share observer worker only"]
fn handle_diagnostic_child()->Check {owned_handles::worker()}
