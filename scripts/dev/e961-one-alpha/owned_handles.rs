//! Owned, post-failure diagnostic only. PSS names outside the fixture never leave this module.
use super::*;
use core::mem::{align_of, size_of};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, IntoRawHandle};
use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, HANDLE, DUPLICATE_SAME_ACCESS, ERROR_NO_MORE_ITEMS, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Diagnostics::ProcessSnapshotting::*;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const CHILD: &str = "infrastructure::workspace_services::e961_one_alpha::handle_diagnostic_child";
const RWD: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const MAX_UNITS: usize = u16::MAX as usize / 2;
const KNOWN: [&str; 10] = ["", "alpha", "alpha/Configuration.xml", "alpha/CommonModules", "alpha/CommonModules/SearchProbe.xml", "alpha/CommonModules/SearchProbe", "alpha/CommonModules/SearchProbe/Ext", "alpha/CommonModules/SearchProbe/Ext/Module.bsl", "nested", "nested/deeper"];
fn win(stage: &str, code: u32) -> String { format!("{stage}: Win32 {code}") }
fn wide(path: &Path) -> Vec<u16> { path.as_os_str().encode_wide().chain(Some(0)).collect() }
struct Held(Option<HANDLE>);
impl Held {
    fn raw(&self) -> HANDLE { self.0.unwrap_or(std::ptr::null_mut()) }
    fn close(mut self) -> Check {
        let handle = self.0.take().ok_or("handle already consumed")?;
        // SAFETY: exclusively owned, real handle; one checked close, never CLOSE_SOURCE.
        let code = if unsafe { CloseHandle(handle) } != 0 { 0 } else { std::io::Error::last_os_error().raw_os_error().unwrap_or(-1) };
        ensure(code == 0, format!("CloseHandle failed: Win32 {code}; worker exit must reclaim resource"))
    }
}
impl Drop for Held { fn drop(&mut self) { if let Some(handle) = self.0.take() { unsafe { CloseHandle(handle); } } } }
fn file_access(path: &Path, share: u32, access: u32) -> Check<fs::File> {
    let file = OpenOptions::new().access_mode(access).share_mode(share)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT).open(path).map_err(|e| e.to_string())?;
    use std::os::windows::fs::MetadataExt;
    ensure(file.metadata().map_err(|e| e.to_string())?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT == 0, "owned object became a reparse point")?;
    Ok(file)
}
fn file(path: &Path, share: u32) -> Check<fs::File> { file_access(path, share, FILE_READ_ATTRIBUTES) }
fn open(path: &Path, share: u32) -> Check<Held> { Ok(Held(Some(file(path, share)?.into_raw_handle()))) }
// Real read/list access makes the four calibration holders participate in share-access accounting.
fn holder(path: &Path, share: u32) -> Check<Held> { Ok(Held(Some(file_access(path, share, FILE_GENERIC_READ)?.into_raw_handle()))) }
fn object_state(paths: &[PathBuf]) -> Check<Vec<Value>> {
    paths.iter().map(|path| {
        let file = file(path, RWD)?;
        let result = (|| -> Check<Value> {
            let meta = file.metadata().map_err(|e| e.to_string())?;
            let identity = crate::infrastructure::platform::filesystem::file_identity(&file).map_err(|e| e.to_string())?.stable_bytes();
            let hash = if meta.is_file() { Some(digest(&fs::read(path).map_err(|e| e.to_string())?)) } else { None };
            Ok(json!({"identity":identity,"directory":meta.is_dir(),"length":meta.len(),"sha256":hash}))
        })();
        Held(Some(file.into_raw_handle())).close()?; result
    }).collect()
}
fn nt_name(handle: HANDLE) -> Check<Vec<u16>> {
    let mut units = vec![0u16; MAX_UNITS + 1];
    // SAFETY: held file/directory handle and owned writable buffer; no foreign name query.
    let length = unsafe { GetFinalPathNameByHandleW(handle, units.as_mut_ptr(), units.len() as u32, VOLUME_NAME_NT | FILE_NAME_NORMALIZED) } as usize;
    ensure(length > 0 && length < units.len(), "owned NT name unavailable or exceeds PSS WORD bound")?;
    units.truncate(length); Ok(units)
}
fn same_name(left: &[u16], right: &[u16]) -> bool {
    // SAFETY: slices are owned, bounded UTF-16 units, with explicit lengths, no NUL required.
    unsafe { CompareStringOrdinal(left.as_ptr(), left.len() as i32, right.as_ptr(), right.len() as i32, 1) == CSTR_EQUAL }
}
fn descriptor_valid(pointer: *const u16, bytes: u16) -> bool {
    bytes > 0 && bytes % 2 == 0 && !pointer.is_null() && (pointer as usize) % align_of::<u16>() == 0 && (pointer as usize).checked_add(bytes as usize).is_some()
}
unsafe fn copy_name(pointer: *const u16, bytes: u16) -> Check<Vec<u16>> {
    ensure(descriptor_valid(pointer, bytes), "invalid PSS UTF-16 descriptor")?;
    // SAFETY: PSS owns the flagged string until marker free; validated WORD byte count/alignment.
    let units = unsafe { std::slice::from_raw_parts(pointer, bytes as usize / 2) }.to_vec();
    ensure(!units.contains(&0), "PSS name unexpectedly contains NUL")?; Ok(units)
}
struct Target { label: String, name: Vec<u16>, known_handle: Option<usize> }
unsafe fn match_entry(entry: &PSS_HANDLE_ENTRY, targets: &[Target]) -> Check<Option<usize>> {
    if entry.Flags & PSS_HANDLE_HAVE_NAME == 0 { return Ok(None); }
    // SAFETY: only flagged PSS buffers or our controlled input arrays, still alive here.
    let name = unsafe { copy_name(entry.ObjectName, entry.ObjectNameLength) }?;
    let Some(index) = targets.iter().position(|target| same_name(&name, &target.name)) else { return Ok(None); };
    // HAVE_TYPE validates ObjectType only; TypeName has its own documented bounded descriptor.
    let kind = unsafe { copy_name(entry.TypeName, entry.TypeNameLength) }?;
    ensure(kind == "File".encode_utf16().collect::<Vec<_>>(), "matched PSS row is not a named File")?;
    Ok(Some(index)) // ObjectType UNKNOWN remains admitted; there is no File enum member.
}
fn target(label: &str, path: &Path, known_handle: Option<usize>) -> Check<Target> {
    let handle = open(path, RWD)?; let name = nt_name(handle.raw())?; handle.close()?;
    Ok(Target { label: label.into(), name, known_handle })
}
struct Snapshot { snapshot: Option<HPSS>, marker: Option<HPSSWALK> }
impl Snapshot {
    fn finish(mut self) -> Check {
        let marker = self.marker.take().map(|value| unsafe { PssWalkMarkerFree(value) });
        let snapshot = self.snapshot.take().map(|value| unsafe { PssFreeSnapshot(GetCurrentProcess(), value) });
        ensure(marker.is_none_or(|code| code == 0) && snapshot.is_none_or(|code| code == 0), format!("PSS free failed: marker={marker:?} snapshot={snapshot:?}"))
    }
}
impl Drop for Snapshot {
    fn drop(&mut self) {
        // Best effort on error/unwind; any uncertainty remains an error, followed by worker exit/reap.
        if let Some(value) = self.marker.take() { unsafe { PssWalkMarkerFree(value); } }
        if let Some(value) = self.snapshot.take() { unsafe { PssFreeSnapshot(GetCurrentProcess(), value); } }
    }
}
fn capture(process: HANDLE, targets: &[Target], deadline: Instant, state: &Path, label: &str) -> Check<Value> {
    let mut owned = Snapshot { snapshot: None, marker: None };
    let work = (|| -> Check<Value> {
    let mut snapshot = std::ptr::null_mut();
    // SAFETY: only current worker or its delegated, identity-checked owned engine process handle.
    let code = unsafe { PssCaptureSnapshot(process, PSS_CAPTURE_HANDLES | PSS_CAPTURE_HANDLE_NAME_INFORMATION, 0, &mut snapshot) };
    ensure(code == 0 && !snapshot.is_null(), format!("{} outNonNull={}", win("PssCaptureSnapshot", code), !snapshot.is_null()))?;
    owned.snapshot = Some(snapshot);
    let mut info = PSS_HANDLE_INFORMATION { HandlesCaptured: 0 };
    // SAFETY: exactly the documented output structure, no reserved/type-specific fields consumed.
    let code = unsafe { PssQuerySnapshot(snapshot, PSS_QUERY_HANDLE_INFORMATION, (&mut info as *mut PSS_HANDLE_INFORMATION).cast(), size_of::<PSS_HANDLE_INFORMATION>() as u32) };
    ensure(code == 0, win("PssQuerySnapshot", code))?;
    let mut marker = std::ptr::null_mut();
    let code = unsafe { PssWalkMarkerCreate(std::ptr::null(), &mut marker) };
    ensure(code == 0 && !marker.is_null(), format!("{} outNonNull={}", win("PssWalkMarkerCreate", code), !marker.is_null()))?;
    owned.marker = Some(marker);
    let mut counts = vec![0u32; targets.len()]; let mut exact = vec![false; targets.len()]; let mut representatives = vec![None; targets.len()]; let mut object_types = vec![None; targets.len()];
    let mut unnamed = 0u32; let mut unmapped = 0u32; let mut invalid = 0u32; let mut visited = 0u32;
    loop {
        let _ = budget(deadline, Duration::MAX)?;
        let mut entry: PSS_HANDLE_ENTRY = unsafe { std::mem::zeroed() };
        // SAFETY: PSS buffer exact ABI; pointers copied before marker free/next walk.
        let code = unsafe { PssWalkSnapshot(snapshot, PSS_WALK_HANDLES, marker, (&mut entry as *mut PSS_HANDLE_ENTRY).cast(), size_of::<PSS_HANDLE_ENTRY>() as u32) };
        if code == ERROR_NO_MORE_ITEMS { break; }
        ensure(code == 0 && visited < info.HandlesCaptured, win("PssWalkSnapshot/count", code))?; visited += 1;
        if entry.Flags & PSS_HANDLE_HAVE_NAME == 0 { unnamed += 1; continue; }
        let index = match unsafe { match_entry(&entry, targets) } {
            Ok(Some(index)) => index, Ok(None) => { unmapped += 1; continue; }, Err(_) => { invalid += 1; continue; }
        };
        counts[index] += 1; representatives[index].get_or_insert(entry.Handle as usize); if entry.Flags & PSS_HANDLE_HAVE_TYPE != 0 { object_types[index].get_or_insert(entry.ObjectType); }
        if targets[index].known_handle == Some(entry.Handle as usize) { exact[index] = true; }
    }
    ensure(visited == info.HandlesCaptured, "PSS iteration ended before declared captured count")?;
    let rows = targets.iter().enumerate().map(|(i, target)| json!({"path":target.label,"count":counts[i],"representativeHandle":representatives[i],"representativeObjectType":object_types[i],"typeName":"File","knownHandleMatched":exact[i]})).collect::<Vec<_>>();
    Ok(json!({"rows":rows,"capturedCount":visited,"unnamed":unnamed,"unmapped":unmapped,"invalid":invalid,"snapshotAndMarkerFreed":false,"absenceIsNotExclusion":true}))
    })();
    let freed = owned.finish(); // Checked even when capture/query/walk failed; precedes all probes.
    let written = windows_probe::write_new(&state.join(format!("{label}-walk.json")), &serde_json::to_vec(&work).map_err(|e| e.to_string())?);
    windows_probe::write_new(&state.join(format!("{label}-free.json")), &serde_json::to_vec(&freed).map_err(|e| e.to_string())?)?;
    written?; let mut data = work?; freed?; data["snapshotAndMarkerFreed"] = json!(true); Ok(data)
}
fn classify(code: u32) -> &'static str { match code { 0 => "opened", 32 => "sharing_violation", _ => "other_win32" } }
fn probe_code(code: u32) -> Value { json!({"rawWin32":code,"classification":classify(code)}) }
fn delete_probe(path: &Path) -> Check<Value> {
    // SAFETY: known owned path, access-only OPEN_EXISTING; never delete/rename/delete-on-close.
    let handle = unsafe { CreateFileW(wide(path).as_ptr(), DELETE, RWD, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, std::ptr::null_mut()) };
    let code = if handle == INVALID_HANDLE_VALUE { std::io::Error::last_os_error().raw_os_error().ok_or("Win32 probe error missing")? as u32 } else { Held(Some(handle)).close()?; 0 };
    let mut record = probe_code(code); record["openedHandleClosed"] = json!(code == 0); Ok(record)
}
fn inputs() -> Check<Value> {
    let kind = [70u16, 105, 108, 101]; let owned = "owned-fixture".encode_utf16().collect::<Vec<_>>();
    let mut entry: PSS_HANDLE_ENTRY = unsafe { std::mem::zeroed() };
    entry.ObjectType = PSS_OBJECT_TYPE_UNKNOWN; entry.Flags = PSS_HANDLE_HAVE_NAME | PSS_HANDLE_HAVE_TYPE;
    entry.ObjectName = owned.as_ptr(); entry.ObjectNameLength = (owned.len() * 2) as u16; entry.TypeName = kind.as_ptr(); entry.TypeNameLength = 8;
    let target = Target { label: "owned".into(), name: owned.clone(), known_handle: None };
    ensure(unsafe { match_entry(&entry, &[target]) }? == Some(0), "UNKNOWN named File control lost")?;
    ensure(descriptor_valid(kind.as_ptr(), 8) && !descriptor_valid(kind.as_ptr(), 7) && !descriptor_valid(std::ptr::null(), 8) && !descriptor_valid(1usize as *const u16, 2) && !descriptor_valid((usize::MAX - 1) as *const u16, 2), "descriptor controls failed")?;
    entry.Flags = 0; entry.ObjectName = std::ptr::null();
    ensure(unsafe { match_entry(&entry, &[]) }?.is_none(), "unflagged pointer was consumed")?;
    let foreign = "not-the-owned-path".encode_utf16().collect::<Vec<_>>(); entry.Flags = PSS_HANDLE_HAVE_NAME; entry.ObjectName = foreign.as_ptr(); entry.ObjectNameLength = (foreign.len() * 2) as u16;
    ensure(unsafe { match_entry(&entry, &[Target { label: "owned".into(), name: owned, known_handle: None }]) }?.is_none(), "foreign name escaped closed matcher")?;
    let codes = [0u32, 32, 5].map(probe_code);
    ensure(codes[2]["rawWin32"] == 5 && codes[2]["classification"] == "other_win32" && codes[1]["classification"] == "sharing_violation", "Win32 codes were conflated")?;
    Ok(json!({"unknownNamedFileAccepted":true,"invalidDescriptorsRejected":true,"unflaggedPointerIgnored":true,"foreignNameNotEmitted":true,"win32Codes":codes}))
}
fn calibration(state: &Path, deadline: Instant) -> Check<Value> {
    let controls = inputs()?; let base = state.join("calibration"); fs::create_dir(&base).map_err(|e| e.to_string())?;
    let labels = ["rwd-file", "rwd-directory", "rw-file", "rw-directory"];
    let paths = labels.map(|name| base.join(name));
    fs::write(&paths[0], b"owned-share-calibration").map_err(|e| e.to_string())?; fs::create_dir(&paths[1]).map_err(|e| e.to_string())?;
    fs::write(&paths[2], b"owned-share-calibration").map_err(|e| e.to_string())?; fs::create_dir(&paths[3]).map_err(|e| e.to_string())?;
    let mut held = Vec::new(); let mut targets = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let handle = holder(path, if i < 2 { RWD } else { FILE_SHARE_READ | FILE_SHARE_WRITE })?;
        targets.push(target(labels[i], path, Some(handle.raw() as usize))?); held.push(handle);
    }
    let before = object_state(&paths)?;
    let snapshot = capture(unsafe { GetCurrentProcess() }, &targets, deadline, state, "calibration")?;
    ensure(snapshot["rows"].as_array().is_some_and(|rows| rows.len() == 4 && rows.iter().all(|row| row["knownHandleMatched"] == true)), "PSS lacks named File capability for all four known handles")?;
    let live = paths.iter().map(|path| delete_probe(path)).collect::<Check<Vec<_>>>()?;
    windows_probe::write_new(&state.join("calibration-live.json"), &serde_json::to_vec(&live).map_err(|e| e.to_string())?)?;
    ensure(live.iter().map(|v| v["rawWin32"].as_u64()).collect::<Vec<_>>() == [Some(0), Some(0), Some(32), Some(32)], "RWD/RW sharing calibration differs")?;
    held.pop().ok_or("RW directory holder missing")?.close()?; held.pop().ok_or("RW file holder missing")?.close()?;
    let after_close = paths[2..].iter().map(|path| delete_probe(path)).collect::<Check<Vec<_>>>()?;
    windows_probe::write_new(&state.join("calibration-after-close.json"), &serde_json::to_vec(&after_close).map_err(|e| e.to_string())?)?;
    ensure(after_close.iter().all(|value| value["rawWin32"] == 0), "closed RW holders still deny DELETE access")?;
    for handle in held { handle.close()?; }
    let after = object_state(&paths)?;
    ensure(before == after, "calibration object metadata changed")?;
    Ok(json!({"status":"Validated","snapshot":snapshot,"live":live,"afterClose":after_close,"allFourHoldersClosed":true,"before":before,"after":after,"inputControls":controls,"daclMutation":false}))
}
fn worker_work(state: &Path, request: &Value, deadline: Instant) -> Check<Value> {
    let calibration = calibration(state, deadline)?;
    let raw = usize::try_from(request["engineHandle"].as_u64().ok_or("engine process handle missing")?).map_err(|e| e.to_string())?;
    ensure(raw > 0 && raw != INVALID_HANDLE_VALUE as usize, "delegated handle is invalid")?;
    let engine = Held(Some(raw as HANDLE));
    let before = windows_probe::identity(engine.raw())?; ensure(before == request["engineIdentity"], "delegated engine identity differs")?;
    let workspace = state.parent().ok_or("state parent missing")?.join("workspace");
    ensure(windows_probe::directory_evidence(&workspace)? == request["workspaceEvidence"], "owned workspace identity/metadata differs")?;
    let targets = KNOWN.iter().map(|label| target(label, &workspace.join(label), None)).collect::<Check<Vec<_>>>()?;
    let snapshot = capture(engine.raw(), &targets, deadline, state, "engine")?;
    let probes = KNOWN.iter().map(|label| delete_probe(&workspace.join(label)).map(|value| json!({"path":label,"probe":value}))).collect::<Check<Vec<_>>>()?;
    windows_probe::write_new(&state.join("engine-probes.json"), &serde_json::to_vec(&probes).map_err(|e| e.to_string())?)?;
    let after = windows_probe::identity(engine.raw())?; ensure(before == after, "engine identity/liveness changed during observer")?;
    engine.close()?;
    Ok(json!({"calibration":calibration,"engineBefore":before,"engineAfter":after,"snapshot":snapshot,"deleteProbes":probes,"delegatedProcessHandleClosed":true}))
}
pub(super) fn worker() -> Check {
    let state = normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E961_HANDLES_STATE").ok_or("missing owned handle state")?))?;
    let root = normalize_path_identity(&PathBuf::from(env::var_os("UNICA_E1_COMPAT_ROOT").ok_or("missing owned root")?))?;
    ensure(state == root.join("handles"), "worker state outside owned scope")?;
    let token = env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e| e.to_string())?;
    let owner: Value = serde_json::from_slice(&fs::read(root.join("owner.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    ensure(owner["token"].as_str() == Some(token.as_str()), "worker root owner differs")?;
    let deadline = Instant::now() + Duration::from_secs(25);
    let request: Value = serde_json::from_slice(&windows_probe::wait_file(&state.join("request.json"), deadline)?).map_err(|e| e.to_string())?;
    let self_identity = windows_probe::identity(unsafe { GetCurrentProcess() })?;
    ensure(request["token"].as_str() == Some(token.as_str()) && request["workerIdentity"] == self_identity, "worker request token/creation identity differs")?;
    let result = worker_work(&state, &request, deadline);
    windows_probe::write_new(&state.join("response.json"), &serde_json::to_vec(&json!({"token":token,"workerIdentity":self_identity,"result":&result})).map_err(|e| e.to_string())?)?;
    result.map(|_| ())
}
pub(super) fn observe(root: &Path, fixture: &Fixture, engine: &ManagedChild, overall: Instant) -> (bool, Check<Value>) {
    let prepared = (|| -> Check<(PathBuf, Duration)> {
        ensure(fixture.locate()? == fixture.context.workspace_root, "failed live rename lost original workspace identity")?;
        let state = root.join("handles"); fs::create_dir(&state).map_err(|e| e.to_string())?;
        Ok((state, budget(overall, Duration::from_secs(30))?))
    })();
    let (state, timeout) = match prepared { Ok(value) => value, Err(error) => return (true, Err(error)) };
    let mut command = Command::new(match env::current_exe() { Ok(value) => value, Err(error) => return (true, Err(error.to_string())) });
    command.args(["--ignored", "--exact", CHILD, "--nocapture", "--test-threads=1"]).current_dir(root.join("control")).env("UNICA_E961_HANDLES_STATE", &state);
    let command_record = command_evidence(&command);
    let mut child = match ManagedChild::spawn_process(command, Some(timeout), CancellationToken::new()) { Ok(value) => value, Err(error) => return (false, Err(error)) };
    let pid = child.id(); let deadline = Instant::now() + timeout;
    let protocol = catch_unwind(AssertUnwindSafe(|| -> Check<Value> {
        let worker_identity = windows_probe::process_evidence(&child)?; let engine_identity = windows_probe::process_evidence(engine)?;
        event(root, "handle-worker-start", json!({"worker":worker_identity,"engine":engine_identity,"command":command_record,"delegation":{"object":"owned process handle only","sameAccess":true,"inherit":false,"closeSource":false}}))?;
        let mut remote = std::ptr::null_mut();
        let duplicated = engine.e961_with_owned_process_handle(|source| child.e961_with_owned_process_handle(|worker| unsafe {
            // SAFETY: only two retained owned PROCESS handles; no PID reopen/object-handle duplication.
            DuplicateHandle(GetCurrentProcess(), source.as_raw_handle(), worker.as_raw_handle(), &mut remote, 0, 0, DUPLICATE_SAME_ACCESS)
        }));
        ensure(duplicated != 0 && !remote.is_null(), "engine process-handle delegation failed")?;
        let token = env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e| e.to_string())?;
        let request = json!({"token":token,"workerIdentity":worker_identity,"engineIdentity":engine_identity,"engineHandle":remote as usize,"workspaceEvidence":windows_probe::directory_evidence(&fixture.context.workspace_root)?});
        windows_probe::write_new(&state.join("request.json"), &serde_json::to_vec(&request).map_err(|e| e.to_string())?)?;
        let response: Value = serde_json::from_slice(&windows_probe::wait_file(&state.join("response.json"), deadline)?).map_err(|e| e.to_string())?;
        ensure(response["token"] == request["token"] && response["workerIdentity"] == worker_identity, "observer response identity differs")?;
        Ok(response)
    }));
    let protocol = match protocol { Ok(value) => value, Err(_) => Err("owned observer protocol panicked".into()) };
    let termination = if protocol.is_err() { Some(child.terminate()) } else { None };
    let output = child.wait_for_output(); let reaped = child.poll_owned_leader_exit();
    let (drained, success, stdout, stderr) = match output { Ok(value) => (!value.timed_out && !value.cancelled && !value.stdout_truncated && !value.stderr_truncated, value.status_success, value.stdout, value.stderr), Err(error) => (false, false, String::new(), error) };
    let quiescent = drained && matches!(&reaped, Ok(true)) && termination.as_ref().is_none_or(Result::is_ok);
    let cleanup = json!({"pid":pid,"streamsDrained":drained,"ownedLeaderReaped":reaped,"termination":termination,"cleanupConfirmed":quiescent,"statusSuccess":success});
    let retained = (|| -> Check<Value> {
        event(root, "handle-worker-cleanup", cleanup.clone())?;
        fs::write(state.join("child.stdout"), stdout).map_err(|e| e.to_string())?; fs::write(state.join("child.stderr"), stderr).map_err(|e| e.to_string())?;
        let response = protocol?;
        event(root, "handle-worker-response", response.clone())?;
        ensure(quiescent && success && response["result"].get("Ok").is_some(), "observer unsupported/failed/incomplete; first live failure preserved")?;
        Ok(json!({"cleanup":cleanup,"observation":response["result"]["Ok"],"firstFailureUnchanged":true}))
    })();
    (quiescent, retained)
}
