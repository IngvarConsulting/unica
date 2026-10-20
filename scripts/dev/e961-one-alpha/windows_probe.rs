//! Test-only read-only empirical Native API observer. NOT a supported Windows ABI.
// Prefix layouts adapted from MSxDOS/ntapi0.4.3, commit
// 3f8ac79fdfea53d8fbe1282237173cee8c4f198c (ntrtl.rs/ntpebteb.rs/STRUCT macro).
// MIT OR Apache-2.0 upstream; exact MIT notice is NTAPI-LICENSE-MIT.txt in control inputs.
use super::*;
use core::ffi::c_void;
use core::mem::{align_of, offset_of, size_of};
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, BorrowedHandle};
use crate::infrastructure::platform::filesystem::{
    capture_windows_immutable_entry_evidence, file_identity, FileIdentity,
    path_starts_with_host_root,
};

pub(super) type Identity=FileIdentity;
const MAX_CWD_BYTES:usize=65534; // Largest even USHORT byte length.
const MAX_REPORT_BYTES:u64=(MAX_CWD_BYTES*3+1024) as u64; // u16 decimal JSON + closed schema.
const CHILD_TEST:&str="infrastructure::workspace_services::e961_one_alpha::cwd_calibration_child";
type Handle=*mut c_void;
#[repr(C)] struct FileTime {low:u32,high:u32}
#[repr(C)] struct BasicInfo {exit_status:i32,peb:Handle,affinity:usize,base_priority:i32,pid:usize,parent_pid:usize}
#[repr(C)] struct UnicodeString {length:u16,maximum_length:u16,buffer:*mut u16}
#[repr(C)] struct CurDir {dos_path:UnicodeString,handle:Handle}
#[repr(C)] struct ParametersPrefix {maximum_length:u32,length:u32,flags:u32,debug_flags:u32,console_handle:Handle,console_flags:u32,standard_input:Handle,standard_output:Handle,standard_error:Handle,current_directory:CurDir}
#[repr(C)] struct PebPrefix {inherited:u8,read_image_options:u8,being_debugged:u8,bit_field:u8,mutant:Handle,image_base:Handle,ldr:Handle,process_parameters:Handle}
const _:()={
    assert!(size_of::<Handle>()==8&&align_of::<Handle>()==8);
    assert!(size_of::<BasicInfo>()==48&&offset_of!(BasicInfo,peb)==8&&offset_of!(BasicInfo,pid)==32);
    assert!(size_of::<UnicodeString>()==16&&offset_of!(UnicodeString,buffer)==8);
    assert!(size_of::<CurDir>()==24&&offset_of!(CurDir,dos_path)==0);
    assert!(size_of::<ParametersPrefix>()==80&&offset_of!(ParametersPrefix,flags)==8&&offset_of!(ParametersPrefix,current_directory)==56);
    assert!(size_of::<PebPrefix>()==40&&offset_of!(PebPrefix,process_parameters)==32);
};
#[link(name="kernel32")]
extern "system" {
    fn GetCurrentProcess()->Handle;
    fn GetProcessId(process:Handle)->u32;
    fn GetProcessTimes(process:Handle,created:*mut FileTime,exited:*mut FileTime,kernel:*mut FileTime,user:*mut FileTime)->i32;
    fn IsWow64Process2(process:Handle,machine:*mut u16,native_machine:*mut u16)->i32;
    fn WaitForSingleObject(handle:Handle,milliseconds:u32)->u32;
    fn ReadProcessMemory(process:Handle,address:*const c_void,output:*mut c_void,length:usize,read:*mut usize)->i32;
    fn GetModuleHandleW(name:*const u16)->Handle;
    fn GetProcAddress(module:Handle,name:*const u8)->*mut c_void;
    fn GetLogicalDrives()->u32;
    fn GetDriveTypeW(root:*const u16)->u32;
}
type NtQuery=unsafe extern "system" fn(Handle,u32,*mut c_void,u32,*mut u32)->i32;
fn last(stage:&str)->String {format!("{stage}: {}",std::io::Error::last_os_error())}
fn hex(bytes:&[u8])->String {bytes.iter().map(|byte|format!("{byte:02x}")).collect()}
fn wide(path:&Path)->Vec<u16> {use std::os::windows::ffi::OsStrExt;path.as_os_str().encode_wide().chain(Some(0)).collect()}

fn open_evidence(path:&Path)->std::io::Result<fs::File> {
    use windows_sys::Win32::Storage::FileSystem::{FILE_READ_ATTRIBUTES,READ_CONTROL,FILE_SHARE_READ,FILE_SHARE_WRITE,FILE_SHARE_DELETE,FILE_FLAG_BACKUP_SEMANTICS,FILE_FLAG_OPEN_REPARSE_POINT,FILE_ATTRIBUTE_DIRECTORY,FILE_ATTRIBUTE_REPARSE_POINT};
    let file=OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES|READ_CONTROL)
        .share_mode(FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT).open(path)?;
    let attributes=file.metadata()?.file_attributes();
    if attributes&FILE_ATTRIBUTE_DIRECTORY==0||attributes&FILE_ATTRIBUTE_REPARSE_POINT!=0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput,"evidence target is not a no-follow directory"));
    }
    Ok(file)
}
pub(super) fn directory_identity(path:&Path)->std::io::Result<Identity> {
    let file=open_evidence(path)?;file_identity(&file) // file closes before caller can rename.
}
pub(super) fn directory_evidence(path:&Path)->Check<Value> {
    let file=open_evidence(path).map_err(|e|e.to_string())?;
    let evidence=capture_windows_immutable_entry_evidence(&file).map_err(|e|e.to_string())?;
    let attributes=file.metadata().map_err(|e|e.to_string())?.file_attributes();
    Ok(json!({"identity":hex(&evidence.identity.stable_bytes()),"ownerDaclSha256":hex(&evidence.security_descriptor_sha256),"attributes":attributes,"handlePolicy":"temporary metadata-only, share RWD, closed before rename"}))
}
fn architecture(handle:Handle)->Check {
    let mut machine=0u16;let mut native=0u16;
    // SAFETY: valid held process handle and writable scalar outputs.
    ensure(unsafe{IsWow64Process2(handle,&mut machine,&mut native)}!=0,last("IsWow64Process2"))?;
    ensure(machine==0&&native==0x8664&&size_of::<usize>()==8,"empirical observer requires native AMD64, not WOW64/ARM64")
}
pub(super) fn identity(handle:Handle)->Check<Value> {
    // SAFETY: GetCurrentProcess returns the current-process pseudo handle; it is not closed.
    architecture(unsafe{GetCurrentProcess()})?;architecture(handle)?;
    let mut created=FileTime{low:0,high:0};let mut exited=FileTime{low:0,high:0};let mut kernel=FileTime{low:0,high:0};let mut user=FileTime{low:0,high:0};
    // SAFETY: handle is held by the owned child and all outputs have proper ABI layout.
    ensure(unsafe{GetProcessTimes(handle,&mut created,&mut exited,&mut kernel,&mut user)}!=0,last("GetProcessTimes"))?;
    // SAFETY: zero timeout only observes the held process object.
    ensure(unsafe{WaitForSingleObject(handle,0)}==0x102,"owned process is exited or liveness query failed")?;
    // SAFETY: valid process handle.
    let pid=unsafe{GetProcessId(handle)};ensure(pid!=0,last("GetProcessId"))?;
    Ok(json!({"pid":pid,"creationFileTime":(u64::from(created.high)<<32)|u64::from(created.low),"nativeMachine":0x8664,"processMachine":0,"live":true}))
}
pub(super) fn process_evidence(child:&ManagedChild)->Check<Value> {
    child.e961_with_owned_process_handle(|held|identity(held.as_raw_handle()))
}
fn add(base:usize,offset:usize)->Check<usize> {base.checked_add(offset).ok_or("remote address arithmetic overflow".into())}
fn read(handle:Handle,address:usize,length:usize)->Check<Vec<u8>> {
    ensure(address!=0&&length>0&&length<=MAX_CWD_BYTES,"remote read exceeds the closed cwd bound")?;
    let _=add(address,length)?;
    let mut bytes=vec![0u8;length];let mut actual=0usize;
    // SAFETY: foreign address is passed only to RPM, never dereferenced locally; output owns length bytes.
    ensure(unsafe{ReadProcessMemory(handle,address as *const c_void,bytes.as_mut_ptr().cast(),length,&mut actual)}!=0,last("ReadProcessMemory"))?;
    ensure(actual==length,"ReadProcessMemory returned partial bytes")?;Ok(bytes)
}
fn u64_at(bytes:&[u8],start:usize)->Check<u64> {Ok(u64::from_le_bytes(bytes.get(start..start+8).ok_or("truncated pointer")?.try_into().map_err(|_|"pointer width")?))}
fn u16_at(bytes:&[u8],start:usize)->Check<u16> {Ok(u16::from_le_bytes(bytes.get(start..start+2).ok_or("truncated USHORT")?.try_into().map_err(|_|"USHORT width")?))}
#[derive(PartialEq,Eq)]
struct Sample {parameters:usize,flags:u32,descriptor:Vec<u8>,bytes:Vec<u8>}
fn sample(handle:Handle,peb:usize)->Check<Sample> {
    let parameters=usize::try_from(u64_at(&read(handle,add(peb,offset_of!(PebPrefix,process_parameters))?,8)?,0)?).map_err(|_|"pointer does not fit AMD64")?;
    ensure(parameters!=0&&parameters%8==0,"invalid process parameters pointer")?;
    let flags=u32::from_le_bytes(read(handle,add(parameters,offset_of!(ParametersPrefix,flags))?,4)?.try_into().map_err(|_|"Flags width")?);
    ensure(flags&1!=0,"process parameters are not normalized; no relative-pointer fallback")?;
    let descriptor=read(handle,add(parameters,offset_of!(ParametersPrefix,current_directory)+offset_of!(CurDir,dos_path))?,size_of::<UnicodeString>())?;
    let length=usize::from(u16_at(&descriptor,0)?);let maximum=usize::from(u16_at(&descriptor,2)?);
    let buffer=usize::try_from(u64_at(&descriptor,8)?).map_err(|_|"cwd pointer does not fit")?;
    ensure(length>0&&length%2==0&&maximum%2==0&&length<=maximum&&length<=MAX_CWD_BYTES&&buffer!=0&&buffer%2==0,"invalid cwd UNICODE_STRING descriptor")?;
    let bytes=read(handle,buffer,length)?;Ok(Sample{parameters,flags,descriptor,bytes})
}
struct RawCwd {units:Vec<u16>,canonical:PathBuf,identity:Value}
fn raw_cwd(held:BorrowedHandle<'_>,expected_pid:u32)->Check<RawCwd> {
    let handle=held.as_raw_handle();let before=identity(handle)?;
    ensure(before.get("pid").and_then(Value::as_u64)==Some(u64::from(expected_pid)),"held process PID differs from owner")?;
    let name="ntdll.dll".encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    // SAFETY: query an already loaded system module; no LoadLibrary/search/install.
    let module=unsafe{GetModuleHandleW(name.as_ptr())};ensure(!module.is_null(),last("GetModuleHandleW ntdll"))?;
    // SAFETY: fixed NUL-terminated export name; ABI is checked by the private function type.
    let function=unsafe{GetProcAddress(module,b"NtQueryInformationProcess\0".as_ptr())};ensure(!function.is_null(),"NtQueryInformationProcess unavailable")?;
    // SAFETY: resolved known NT API signature; internal-layout uncertainty is separately calibrated.
    let query:NtQuery=unsafe{core::mem::transmute(function)};
    let mut basic=BasicInfo{exit_status:0,peb:std::ptr::null_mut(),affinity:0,base_priority:0,pid:0,parent_pid:0};let mut returned=0u32;
    // SAFETY: buffer is exactly the declared ABI structure; query only ProcessBasicInformation0.
    let status=unsafe{query(handle,0,(&mut basic as *mut BasicInfo).cast(),size_of::<BasicInfo>() as u32,&mut returned)};
    ensure(status==0&&returned==size_of::<BasicInfo>() as u32&&basic.pid==expected_pid as usize&&!basic.peb.is_null(),format!("ProcessBasicInformation rejected status={status} bytes={returned}"))?;
    let first=sample(handle,basic.peb as usize)?;let second=sample(handle,basic.peb as usize)?;
    ensure(first==second,"cwd pointer/descriptor/bytes changed across bounded samples")?;
    let after=identity(handle)?;ensure(before==after,"owned process identity/liveness changed during cwd sample")?;
    let units=first.bytes.chunks_exact(2).map(|part|u16::from_le_bytes([part[0],part[1]])).collect::<Vec<_>>();
    ensure(!units.contains(&0),"cwd Length unexpectedly includes NUL")?;
    let canonical=normalize_path_identity(&PathBuf::from(OsString::from_wide(&units)))?;
    Ok(RawCwd{units,canonical,identity:before})
}
pub(super) struct ValidatedLayout { _private:() }
pub(super) fn observe(child:&ManagedChild,_validated:&ValidatedLayout,workspace:&Path,source:&Path)->Value {
    let result=(||->Check<Value>{
        let raw=child.e961_with_owned_process_handle(|held|raw_cwd(held,child.id()))?;
        let workspace=normalize_path_identity(workspace)?;let source=normalize_path_identity(source)?;
        let inside=path_starts_with_host_root(&raw.canonical,&workspace)||path_starts_with_host_root(&raw.canonical,&source);
        Ok(json!({"status":if inside{"ObservedInside"}else{"ObservedExternal"},"rawUtf16":raw.units,"canonical":raw.canonical,"process":raw.identity,"layout":"ntapi0.4.3 empirical AMD64 calibrated C/D/A","continuousInvariantClaim":false}))
    })();
    match result {Ok(value)=>value,Err(error)=>json!({"status":"Unknown","stage":"read-or-canonicalization","reason":error})}
}
fn drive(path:&Path)->Option<u8> {
    use std::path::{Component,Prefix};
    match path.components().next()? {Component::Prefix(prefix)=>match prefix.kind(){Prefix::Disk(letter)|Prefix::VerbatimDisk(letter)=>Some(letter.to_ascii_uppercase()),_=>None},_=>None}
}
fn second_root(root:&Path,token:&str)->Check<PathBuf> {
    let first=drive(root).ok_or("calibration root has no drive-letter prefix")?;
    let temp=env::temp_dir();
    if let Some(letter)=drive(&temp) {
        let base=PathBuf::from(format!("{}:\\",letter as char));
        // SAFETY: fixed root string, read-only drive classification.
        if letter!=first&&unsafe{GetDriveTypeW(wide(&base).as_ptr())}==3 {
            let path=temp.join(format!("unica-e961-cwd-{token}"));fs::create_dir(&path).map_err(|e|e.to_string())?;return Ok(path);
        }
    }
    // SAFETY: read-only finite26-bit drive-letter inventory, no mount or SUBST changes.
    let available=unsafe{GetLogicalDrives()};ensure(available!=0,last("GetLogicalDrives"))?;
    for index in 0u8..26 {
        let letter=b'A'+index;if letter==first||available&(1u32<<index)==0 {continue;}
        let base=PathBuf::from(format!("{}:\\",letter as char));
        // SAFETY: NUL-terminated root for a single existing drive.
        if unsafe{GetDriveTypeW(wide(&base).as_ptr())}!=3 {continue;}
        let path=base.join(format!("unica-e961-cwd-{token}"));
        // One selected existing fixed drive: failure is Unknown, never try more until green.
        fs::create_dir(&path).map_err(|e|e.to_string())?;return Ok(path);
    }
    Err("no existing second fixed drive; no mappings created".into())
}
pub(super) fn write_new(path:&Path,bytes:&[u8])->Check {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
        Ok(_)=>return Err("owned control destination already exists".into()),
        Err(error)=>return Err(error.to_string()),
    }
    let mut pending=path.as_os_str().to_os_string();pending.push(".pending");let pending=PathBuf::from(pending);
    {
        let mut file=OpenOptions::new().write(true).create_new(true).open(&pending).map_err(|e|e.to_string())?;
        file.write_all(bytes).map_err(|e|e.to_string())?;file.flush().map_err(|e|e.to_string())?;
    }
    fs::rename(pending,path).map_err(|e|e.to_string())
}
pub(super) fn wait_file(path:&Path,deadline:Instant)->Check<Vec<u8>> {
    loop {
        match fs::metadata(path) {
            Ok(meta)=>{ensure(meta.is_file()&&meta.len()<=MAX_REPORT_BYTES,"control file exceeds closed bound")?;return fs::read(path).map_err(|e|e.to_string());},
            Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},
            Err(error)=>return Err(error.to_string()),
        }
        ensure(Instant::now()<deadline,"calibration protocol deadline expired")?;thread::sleep(Duration::from_millis(10));
    }
}
pub(super) fn calibrate(root:&Path,overall:Instant)->Check<(Option<ValidatedLayout>,Value)> {
    let token=env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e|e.to_string())?;
    ensure(token.chars().all(|c|c.is_ascii_hexdigit()||c=='-'),"invalid owned calibration token")?;
    let state=root.join("cwd-calibration");fs::create_dir(&state).map_err(|e|e.to_string())?;
    let a=state.join("a");fs::create_dir(&a).map_err(|e|e.to_string())?;
    let b_root=match second_root(root,&token) {Ok(path)=>path,Err(error)=>return Ok((None,json!({"status":"Unknown","stage":"second-drive","reason":error,"childStarted":false}))) };
    write_new(&b_root.join("owner-token.txt"),token.as_bytes())?;
    let b=b_root.join("b-Ж-different-length");fs::create_dir(&b).map_err(|e|e.to_string())?;
    ensure(drive(&a)!=drive(&b),"calibration did not select different drives")?;
    let mut command=Command::new(env::current_exe().map_err(|e|e.to_string())?);
    command.args(["--ignored","--exact",CHILD_TEST,"--nocapture","--test-threads=1"]).current_dir(&a)
        .env("UNICA_E961_CALIB_STATE",&state).env("UNICA_E961_CALIB_A",&a).env("UNICA_E961_CALIB_B",&b);
    let timeout=budget(overall,Duration::from_secs(20))?;let deadline=Instant::now()+timeout;
    let mut child=ManagedChild::spawn_process(command,Some(timeout),CancellationToken::new())?;let pid=child.id();
    let outcome=catch_unwind(AssertUnwindSafe(||->Check<Vec<Value>>{
        let mut observations=Vec::new();
        for (phase,expected) in [&a,&b,&a].into_iter().enumerate() {
            let ready=wait_file(&state.join(format!("{phase}.ready")),deadline)?;ensure(ready==token.as_bytes(),"calibration readiness token differs")?;
            let report:Value=serde_json::from_slice(&wait_file(&state.join(format!("{phase}.json")),deadline)?).map_err(|e|e.to_string())?;
            ensure(report.get("pid").and_then(Value::as_u64)==Some(u64::from(pid))&&report.get("phase").and_then(Value::as_u64)==Some(phase as u64),"calibration child identity/phase differs")?;
            let units:Vec<u16>=serde_json::from_value(report.get("rawUtf16").cloned().ok_or("self-report raw units missing")?).map_err(|e|e.to_string())?;
            let self_path=normalize_path_identity(&PathBuf::from(OsString::from_wide(&units)))?;
            let raw=child.e961_with_owned_process_handle(|held|raw_cwd(held,pid))?;
            let expected=normalize_path_identity(expected)?;
            ensure(self_path==expected&&raw.canonical==expected,"observer/self-report/known physical cwd differ")?;
            let inside=path_starts_with_host_root(&raw.canonical,&normalize_path_identity(&b_root)?);
            ensure(inside==(phase==1),"calibration Inside/External classifier differs")?;
            observations.push(json!({"phase":phase,"selfReport":report,"observedRawUtf16":raw.units,"canonical":raw.canonical,"process":raw.identity,"insideControlRootB":inside}));
            write_new(&state.join(format!("{phase}.ack")),token.as_bytes())?;
        }
        Ok(observations)
    }));
    let outcome=match outcome {Ok(value)=>value,Err(_)=>Err("calibration protocol panicked".into())};
    let termination=if outcome.is_err(){Some(child.terminate())}else{None};
    let output=child.wait_for_output();let reaped=child.poll_owned_leader_exit();
    let (drained,status_success,stdout,stderr)=match output {Ok(value)=>(!value.timed_out&&!value.cancelled&&!value.stdout_truncated&&!value.stderr_truncated,value.status_success,value.stdout,value.stderr),Err(error)=>(false,false,String::new(),error)};
    fs::write(state.join("child.stdout"),&stdout).map_err(|e|e.to_string())?;fs::write(state.join("child.stderr"),&stderr).map_err(|e|e.to_string())?;
    let confirmed=drained&&matches!(&reaped,Ok(true))&&termination.as_ref().is_none_or(Result::is_ok);
    let evidence=json!({"status":if outcome.is_ok()&&status_success{"Validated"}else{"Unknown"},"pid":pid,"phases":outcome,"termination":termination,"ownedLeaderReaped":reaped,"streamsDrained":drained,"cleanupConfirmed":confirmed,"source":"ntapi0.4.3 empirical AMD64","externalOwnedRootPreserved":b_root});
    event(root,"cwd-calibration-cleanup",evidence.clone())?;
    ensure(confirmed,"calibration child cleanup unconfirmed; do not start native Alpha")?;
    if evidence.get("status").and_then(Value::as_str)==Some("Validated") {Ok((Some(ValidatedLayout{_private:()}),evidence))}else{Ok((None,evidence))}
}
pub(super) fn calibration_child()->Check {
    let state=PathBuf::from(env::var_os("UNICA_E961_CALIB_STATE").ok_or("missing private calibration state")?);
    let a=PathBuf::from(env::var_os("UNICA_E961_CALIB_A").ok_or("missing calibration A")?);
    let b=PathBuf::from(env::var_os("UNICA_E961_CALIB_B").ok_or("missing calibration B")?);
    let token=env::var("UNICA_E1_COMPAT_TOKEN").map_err(|e|e.to_string())?;
    let deadline=Instant::now()+Duration::from_secs(20);
    for (phase,target) in [&a,&b,&a].into_iter().enumerate() {
        env::set_current_dir(target).map_err(|e|e.to_string())?; // This isolated child only.
        let current=env::current_dir().map_err(|e|e.to_string())?;
        use std::os::windows::ffi::OsStrExt;
        let report=json!({"phase":phase,"pid":std::process::id(),"rawUtf16":current.as_os_str().encode_wide().collect::<Vec<_>>()});
        write_new(&state.join(format!("{phase}.json")),serde_json::to_vec(&report).map_err(|e|e.to_string())?.as_slice())?;
        write_new(&state.join(format!("{phase}.ready")),token.as_bytes())?;
        ensure(wait_file(&state.join(format!("{phase}.ack")),deadline)?==token.as_bytes(),"calibration ack token differs")?;
    }
    Ok(())
}
