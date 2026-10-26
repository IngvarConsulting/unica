use super::protocol_v5::{
    parse_v5_endpoint_record, V5EndpointRecord, MAX_V5_ENDPOINT_RECORD_BYTES,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use unica_execution::receipt_ledger::CoreIdentityDigest;
#[cfg(test)]
use unica_execution::receipt_runtime::authority::ReceiptAuthorityLock;
use unica_execution::receipt_runtime::authority::{state_io_error, ReceiptStateRoot};
use unica_workspace::state_fs::StateHandle as File;
use unica_workspace::state_fs::{
    create_state_directory_child, create_state_file_child, open_or_create_state_directory_nofollow,
    open_state_directory_child, open_state_directory_nofollow, open_state_ownership_lock,
    open_state_regular_child, remove_state_regular_child, replace_state_regular_child,
    restrict_state_stage_to_owner, state_file_identity, sync_state_directory,
    verify_state_owner_only, FileIdentity, StateDirectory,
};
use uuid::Uuid;

const CORE_ABI_IDENTITY: &str = "unica-v0.13-core-abi-1";
const DAEMON_PROTOCOL_IDENTITY_PREFIX: &str = "unica-daemon-jsonl-";
const ENDPOINT_FILE_NAME: &str = "endpoint.json";
const SPAWN_LOCK_NAME: &str = ".daemon-spawn.lock";
/// Why the last daemon process of this identity refused to start (#1360).
/// The spawned daemon has no stderr channel, so this file is the only way
/// its frontend learns the cause.
const STARTUP_FAILURE_FILE_NAME: &str = "startup-failure.txt";
/// Upper bound of the recorded reason; a longer one is cut at a character.
const MAX_STARTUP_FAILURE_REASON_BYTES: usize = 2_048;

/// The one daemon wire protocol this core speaks; it names the state
/// directory (`daemon-p5-…`) and the identity digest.
pub const DAEMON_PROTOCOL_VERSION: u32 = 5;

/// Environment through which the bootstrap hands the verified runtime
/// manifest to the core it launched.
const RUNTIME_MANIFEST_ENV: &str = "UNICA_RUNTIME_MANIFEST";

/// The build identity of this process, installed once by its entry point.
static BUILD_IDENTITY: OnceLock<CoreIdentity> = OnceLock::new();

/// Refusal when the executable a frontend would start its daemon from is no
/// longer the build the frontend runs.
pub const EXECUTABLE_CHANGED: &str = "the unica executable changed after this process started; restart the host to use the new build";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoreIdentity(CoreIdentityDigest);

impl CoreIdentity {
    /// The identity this process's daemon is keyed by.
    ///
    /// The frontend-to-daemon protocol is internal and keeps no backward
    /// compatibility, so a daemon is keyed by the build, not by a release
    /// version: a frontend never meets a daemon of another build. The entry
    /// point installs the build identity before any daemon exchange; a process
    /// that installed none (an in-process test) uses the ABI identity alone.
    pub fn production() -> Self {
        BUILD_IDENTITY.get().cloned().unwrap_or_else(Self::abi_v5)
    }

    pub fn production_v5() -> Self {
        Self::production()
    }

    /// The frozen ABI part: core ABI and daemon protocol, without the build.
    pub fn abi_v5() -> Self {
        let mut digest = Sha256::new();
        digest.update(CORE_ABI_IDENTITY.as_bytes());
        digest.update(b"\0");
        digest.update(DAEMON_PROTOCOL_IDENTITY_PREFIX.as_bytes());
        digest.update(DAEMON_PROTOCOL_VERSION.to_string().as_bytes());
        Self(CoreIdentityDigest::from_sha256(digest.finalize().into()))
    }

    /// The ABI identity bound to one build fingerprint.
    fn for_build(fingerprint: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(Self::abi_v5().as_str().as_bytes());
        digest.update(b"\0build\0");
        digest.update(fingerprint.as_bytes());
        Self(CoreIdentityDigest::from_sha256(digest.finalize().into()))
    }

    /// Compute this executable's build identity and install it for the
    /// process. Called once, at start, before the binary can be replaced.
    pub fn install_build() -> Result<Self, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("failed to locate the unica executable: {error}"))?;
        Self::install(build_identity_of(&executable)?)
    }

    /// Install an identity computed elsewhere: a test fixture that serves or
    /// connects on behalf of a built binary.
    pub fn install(identity: Self) -> Result<Self, String> {
        let installed = BUILD_IDENTITY.get_or_init(|| identity.clone());
        if installed != &identity {
            return Err("a different build identity is already installed".to_string());
        }
        Ok(identity)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn digest(&self) -> &CoreIdentityDigest {
        &self.0
    }
}

impl fmt::Display for CoreIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The build identity of `executable`: the archive digest the bootstrap
/// verified for this target when it launched a packaged core, otherwise the
/// SHA-256 of the executable bytes.
pub fn build_identity_of(executable: &Path) -> Result<CoreIdentity, String> {
    if let Some(digest) = packaged_core_digest() {
        return Ok(CoreIdentity::for_build(&format!("release:{digest}")));
    }
    build_identity_of_executable(executable)
}

fn build_identity_of_executable(executable: &Path) -> Result<CoreIdentity, String> {
    let mut file = std::fs::File::open(executable)
        .map_err(|error| format!("failed to read the unica executable: {error}"))?;
    let mut digest = Sha256::new();
    io::copy(&mut file, &mut digest)
        .map_err(|error| format!("failed to read the unica executable: {error}"))?;
    let digest = CoreIdentityDigest::from_sha256(digest.finalize().into());
    Ok(CoreIdentity::for_build(&format!(
        "executable:{}",
        digest.as_str()
    )))
}

/// A frontend about to start its daemon from `executable` proves that the file
/// is still the build it runs: otherwise the daemon would serve another build
/// under this build's identity.
pub fn verify_executable_build(executable: &Path, identity: &CoreIdentity) -> Result<(), String> {
    if BUILD_IDENTITY.get() != Some(identity) {
        return Ok(());
    }
    if &build_identity_of(executable)? != identity {
        return Err(EXECUTABLE_CHANGED.to_string());
    }
    Ok(())
}

fn packaged_core_digest() -> Option<String> {
    let path = std::env::var_os(RUNTIME_MANIFEST_ENV).filter(|value| !value.is_empty())?;
    let manifest = unica_artifact_delivery::RuntimeManifest::load(Path::new(&path)).ok()?;
    let target = unica_artifact_delivery::HostTarget::current().ok()?;
    Some(manifest.target(target).ok()?.asset.sha256.clone())
}

impl FromStr for CoreIdentity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse::<CoreIdentityDigest>()
            .map(Self)
            .map_err(|_| "core identity must be exactly 64 lowercase hexadecimal bytes".to_string())
    }
}

impl Serialize for CoreIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CoreIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug)]
pub struct DaemonStateDirectory {
    path: PathBuf,
    directory: File,
    retained_directory: StateDirectory,
    identity: FileIdentity,
}

impl DaemonStateDirectory {
    pub fn path_for(state_root: &Path, core_identity: &CoreIdentity) -> PathBuf {
        state_root.join(format!(
            "daemon-p{DAEMON_PROTOCOL_VERSION}-{}",
            core_identity.as_str()
        ))
    }

    pub fn open(state_root: &Path, core_identity: &CoreIdentity) -> Result<Self, String> {
        Self::open_with(state_root, core_identity, true)?
            .ok_or_else(|| "daemon identity directory vanished during admission".to_string())
    }

    /// Open the state directory of an identity only if it already exists:
    /// inspecting another build's directory must not recreate it.
    pub fn open_existing(
        state_root: &Path,
        core_identity: &CoreIdentity,
    ) -> Result<Option<Self>, String> {
        Self::open_with(state_root, core_identity, false)
    }

    fn open_with(
        state_root: &Path,
        core_identity: &CoreIdentity,
        create: bool,
    ) -> Result<Option<Self>, String> {
        if !state_root.is_absolute() {
            return Err("daemon state root must be absolute".to_string());
        }
        let parent = if create {
            open_or_create_state_directory_nofollow(state_root)
        } else {
            open_state_directory_nofollow(state_root)
        }
        .map_err(|error| state_io_error("open daemon provider state root", error))?;
        let child_name = format!(
            "daemon-p{DAEMON_PROTOCOL_VERSION}-{}",
            core_identity.as_str()
        );
        let directory = match open_state_directory_child(&parent, OsStr::new(&child_name)) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_state_directory_child(&parent, OsStr::new(&child_name)) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        open_state_directory_child(&parent, OsStr::new(&child_name)).map_err(
                            |error| state_io_error("open raced daemon identity directory", error),
                        )?
                    }
                    Err(error) => {
                        return Err(state_io_error(
                            "create private daemon identity directory",
                            error,
                        ))
                    }
                }
            }
            Err(error) => {
                return Err(state_io_error(
                    "open private daemon identity directory",
                    error,
                ))
            }
        };
        verify_state_owner_only(&directory).map_err(|error| {
            state_io_error("daemon identity directory is not owner-only", error)
        })?;
        let identity = state_file_identity(&directory)
            .map_err(|error| state_io_error("identify daemon identity directory", error))?;
        let path = state_root.join(&child_name);
        let retained_directory = StateDirectory::open(&path)
            .map_err(|error| state_io_error("retain named daemon identity directory", error))?;
        if retained_directory.identity() != identity {
            return Err("named daemon identity directory changed during admission".to_string());
        }
        retained_directory
            .validate_named_identity()
            .map_err(|error| state_io_error("validate named daemon identity directory", error))?;
        Ok(Some(Self {
            path,
            directory,
            retained_directory,
            identity,
        }))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn create_private_subdirectory(&self, name: &str) -> Result<File, String> {
        self.verify_identity()?;
        let child = match open_state_directory_child(&self.directory, OsStr::new(name)) {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_state_directory_child(&self.directory, OsStr::new(name)) {
                    Ok(child) => child,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        open_state_directory_child(&self.directory, OsStr::new(name)).map_err(
                            |error| state_io_error("open raced private daemon child", error),
                        )?
                    }
                    Err(error) => return Err(state_io_error("create private daemon child", error)),
                }
            }
            Err(error) => return Err(state_io_error("open private daemon child", error)),
        };
        verify_state_owner_only(&child)
            .map_err(|error| state_io_error("daemon child is not owner-only", error))?;
        Ok(child)
    }

    pub fn create_private_retained_subdirectory(
        &self,
        name: &str,
    ) -> Result<StateDirectory, String> {
        let child = self.create_private_subdirectory(name)?;
        let expected_identity = state_file_identity(&child)
            .map_err(|error| state_io_error("identify private daemon child", error))?;
        let retained = self
            .retained_directory
            .retain_directory_child(OsStr::new(name))
            .map_err(|error| state_io_error("retain named private daemon child", error))?;
        if retained.identity() != expected_identity {
            return Err("named private daemon child changed during admission".to_string());
        }
        retained
            .validate_named_identity()
            .map_err(|error| state_io_error("validate named private daemon child", error))?;
        Ok(retained)
    }

    #[cfg(test)]
    pub fn acquire_spawn_lock(&self, timeout: Duration) -> Result<SpawnLock, String> {
        self.acquire_spawn_lock_before(Instant::now() + timeout)
    }

    pub fn acquire_spawn_lock_before(&self, deadline: Instant) -> Result<SpawnLock, String> {
        if Instant::now() >= deadline {
            return Err("timed out waiting for daemon spawn ownership".to_string());
        }
        self.verify_identity()?;
        let file = open_state_ownership_lock(&self.directory, OsStr::new(SPAWN_LOCK_NAME))
            .map_err(|error| state_io_error("open daemon spawn ownership object", error))?;
        loop {
            if Instant::now() >= deadline {
                return Err("timed out waiting for daemon spawn ownership".to_string());
            }
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(SpawnLock { file }),
                Err(error)
                    if unica_workspace::state_fs::is_file_lock_contended(&error)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(
                        Duration::from_millis(10)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                Err(error) if unica_workspace::state_fs::is_file_lock_contended(&error) => {
                    return Err("timed out waiting for daemon spawn ownership".to_string())
                }
                Err(error) => {
                    return Err(state_io_error("lock daemon spawn ownership object", error))
                }
            }
        }
    }

    /// One attempt at the spawn lock: `None` while a frontend is starting
    /// this identity's daemon.
    pub fn try_spawn_lock(&self) -> Result<Option<SpawnLock>, String> {
        self.verify_identity()?;
        let file = open_state_ownership_lock(&self.directory, OsStr::new(SPAWN_LOCK_NAME))
            .map_err(|error| state_io_error("open daemon spawn ownership object", error))?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(SpawnLock { file })),
            Err(error) if unica_workspace::state_fs::is_file_lock_contended(&error) => Ok(None),
            Err(error) => Err(state_io_error("lock daemon spawn ownership object", error)),
        }
    }

    pub fn read_v5_endpoint_record(&self) -> Result<Option<V5EndpointRecord>, String> {
        Ok(self
            .read_v5_endpoint_record_retained()?
            .map(|retained| retained.record))
    }

    fn read_v5_endpoint_record_retained(&self) -> Result<Option<RetainedV5EndpointRecord>, String> {
        self.verify_identity()?;
        let mut file =
            match open_state_regular_child(&self.directory, OsStr::new(ENDPOINT_FILE_NAME)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(state_io_error("open v5 daemon endpoint record", error)),
            };
        verify_state_owner_only(&file).map_err(|error| {
            state_io_error("v5 daemon endpoint record is not owner-only", error)
        })?;
        let state_file_identity = state_file_identity(&file)
            .map_err(|error| state_io_error("identify v5 daemon endpoint record", error))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((MAX_V5_ENDPOINT_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| state_io_error("read v5 daemon endpoint record", error))?;
        if bytes.len() > MAX_V5_ENDPOINT_RECORD_BYTES {
            return Err("v5 daemon endpoint record exceeds the byte limit".to_string());
        }
        let record = parse_v5_endpoint_record(&bytes)?;
        Ok(Some(RetainedV5EndpointRecord {
            record,
            file,
            identity: state_file_identity,
        }))
    }

    pub fn publish_v5_endpoint_record(
        &self,
        record: &V5EndpointRecord,
    ) -> Result<RetainedV5EndpointRecord, String> {
        record.validate()?;
        self.verify_identity()?;
        let temporary_name = format!(".endpoint.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_state_file_child(&self.directory, temporary_name)
            .map_err(|error| state_io_error("create private v5 endpoint staging file", error))?;
        let staged_identity = state_file_identity(&file)
            .map_err(|error| state_io_error("identify v5 endpoint staging file", error))?;
        if let Err(error) = restrict_state_stage_to_owner(&file) {
            let _ =
                remove_state_regular_child(&self.directory, temporary_name, staged_identity, &file);
            return Err(state_io_error("restrict v5 endpoint staging file", error));
        }
        let mut bytes = serde_json::to_vec(record)
            .map_err(|_| "v5 daemon endpoint record could not be serialized".to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_V5_ENDPOINT_RECORD_BYTES {
            return Err("v5 daemon endpoint record exceeds the byte limit".to_string());
        }
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ =
                remove_state_regular_child(&self.directory, temporary_name, staged_identity, &file);
            return Err(state_io_error("flush v5 endpoint staging file", error));
        }
        replace_state_regular_child(
            &self.directory,
            temporary_name,
            staged_identity,
            &file,
            OsStr::new(ENDPOINT_FILE_NAME),
        )
        .map_err(|error| state_io_error("publish v5 endpoint record", error))?;
        sync_state_directory(&self.directory)
            .map_err(|error| state_io_error("sync v5 endpoint publication", error))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| state_io_error("rewind v5 endpoint record", error))?;
        Ok(RetainedV5EndpointRecord {
            record: record.clone(),
            file,
            identity: staged_identity,
        })
    }

    pub fn remove_v5_endpoint_if_owned(
        &self,
        retained: &RetainedV5EndpointRecord,
    ) -> Result<bool, String> {
        self.verify_identity()?;
        let Some(current) = self.read_v5_endpoint_record_retained()? else {
            return Ok(false);
        };
        if current.record != retained.record || current.identity != retained.identity {
            return Ok(false);
        }
        remove_state_regular_child(
            &self.directory,
            OsStr::new(ENDPOINT_FILE_NAME),
            retained.identity,
            &retained.file,
        )
        .map_err(|error| state_io_error("remove owned v5 endpoint record", error))?;
        sync_state_directory(&self.directory)
            .map_err(|error| state_io_error("sync v5 endpoint removal", error))?;
        Ok(true)
    }

    pub fn remove_matching_v5_endpoint_record(
        &self,
        expected: &V5EndpointRecord,
    ) -> Result<bool, String> {
        let Some(current) = self.read_v5_endpoint_record_retained()? else {
            return Ok(false);
        };
        if &current.record != expected {
            return Ok(false);
        }
        self.remove_v5_endpoint_if_owned(&current)
    }

    /// Records why the daemon process `pid` refused to start. The record is
    /// replaced atomically: a reader sees either the previous record or this
    /// one, and only the one naming its own child's pid is believed.
    pub fn publish_startup_failure(&self, pid: u32, reason: &str) -> Result<(), String> {
        self.verify_identity()?;
        self.sweep_startup_failure_stages();
        let bytes = startup_failure_bytes(pid, reason);
        let temporary_name = format!("{STARTUP_FAILURE_STAGE_PREFIX}{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_state_file_child(&self.directory, temporary_name)
            .map_err(|error| state_io_error("create startup failure staging file", error))?;
        let staged_identity = state_file_identity(&file)
            .map_err(|error| state_io_error("identify startup failure staging file", error))?;
        if let Err(error) = restrict_state_stage_to_owner(&file)
            .and_then(|_| file.write_all(&bytes))
            .and_then(|_| file.sync_all())
        {
            let _ =
                remove_state_regular_child(&self.directory, temporary_name, staged_identity, &file);
            return Err(state_io_error("write startup failure staging file", error));
        }
        replace_state_regular_child(
            &self.directory,
            temporary_name,
            staged_identity,
            &file,
            OsStr::new(STARTUP_FAILURE_FILE_NAME),
        )
        .map_err(|error| state_io_error("publish startup failure record", error))?;
        sync_state_directory(&self.directory)
            .map_err(|error| state_io_error("sync startup failure record", error))
    }

    /// The reason the daemon process `pid` recorded, if this very process
    /// wrote the current record. A record of another process is stale.
    pub fn read_startup_failure(&self, pid: u32) -> Option<String> {
        self.verify_identity().ok()?;
        let mut file =
            open_state_regular_child(&self.directory, OsStr::new(STARTUP_FAILURE_FILE_NAME))
                .ok()?;
        verify_state_owner_only(&file).ok()?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((MAX_STARTUP_FAILURE_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .ok()?;
        parse_startup_failure(&bytes, pid)
    }

    /// Removes the startup failure record, whoever wrote it.
    pub fn remove_startup_failure(&self) -> Result<(), String> {
        self.verify_identity()?;
        self.sweep_startup_failure_stages();
        let file = match open_state_regular_child(
            &self.directory,
            OsStr::new(STARTUP_FAILURE_FILE_NAME),
        ) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(state_io_error("open startup failure record", error)),
        };
        let identity = state_file_identity(&file)
            .map_err(|error| state_io_error("identify startup failure record", error))?;
        remove_state_regular_child(
            &self.directory,
            OsStr::new(STARTUP_FAILURE_FILE_NAME),
            identity,
            &file,
        )
        .map_err(|error| state_io_error("remove startup failure record", error))?;
        sync_state_directory(&self.directory)
            .map_err(|error| state_io_error("sync startup failure removal", error))
    }

    /// Best effort: removes staging files a process left when it died
    /// between creating and publishing its startup failure record.
    fn sweep_startup_failure_stages(&self) {
        let Ok(entries) = std::fs::read_dir(&self.path) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let is_stage = name.to_str().is_some_and(|name| {
                name.starts_with(STARTUP_FAILURE_STAGE_PREFIX) && name.ends_with(".tmp")
            });
            if !is_stage {
                continue;
            }
            let Ok(file) = open_state_regular_child(&self.directory, &name) else {
                continue;
            };
            let Ok(identity) = state_file_identity(&file) else {
                continue;
            };
            let _ = remove_state_regular_child(&self.directory, &name, identity, &file);
        }
    }

    fn verify_identity(&self) -> Result<(), String> {
        self.retained_directory
            .validate_named_identity()
            .map_err(|error| state_io_error("validate named daemon identity directory", error))?;
        let actual = state_file_identity(&self.directory)
            .map_err(|error| state_io_error("verify daemon directory identity", error))?;
        if actual != self.identity {
            return Err("retained daemon directory identity changed".to_string());
        }
        verify_state_owner_only(&self.directory)
            .map_err(|error| state_io_error("daemon identity directory is not owner-only", error))
    }
}

#[derive(Debug)]
pub struct RetainedV5EndpointRecord {
    pub record: V5EndpointRecord,
    file: File,
    identity: FileIdentity,
}

pub struct SpawnLock {
    file: File,
}

impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

const STARTUP_FAILURE_PID_PREFIX: &str = "pid=";
const STARTUP_FAILURE_STAGE_PREFIX: &str = ".startup-failure.";
const MAX_STARTUP_FAILURE_RECORD_BYTES: usize =
    STARTUP_FAILURE_PID_PREFIX.len() + 10 + 1 + MAX_STARTUP_FAILURE_REASON_BYTES;

/// `pid=<pid>` on the first line, then the reason on one line: control
/// characters become spaces and the text is cut at the byte bound.
fn startup_failure_bytes(pid: u32, reason: &str) -> Vec<u8> {
    let mut text = String::new();
    for character in reason.chars() {
        let character = if is_hidden_or_line_breaking(character) {
            ' '
        } else {
            character
        };
        if text.len() + character.len_utf8() > MAX_STARTUP_FAILURE_REASON_BYTES {
            break;
        }
        text.push(character);
    }
    format!("{STARTUP_FAILURE_PID_PREFIX}{pid}\n{text}").into_bytes()
}

/// Characters that must not reach the one-line message a host shows: control
/// characters (Cc), invisible format and bidi controls (Cf) and the Unicode
/// line and paragraph separators (Zl, Zp).
fn is_hidden_or_line_breaking(character: char) -> bool {
    character.is_control()
        || matches!(
            u32::from(character),
            0x00AD
                | 0x0600..=0x0605
                | 0x061C
                | 0x06DD
                | 0x070F
                | 0x0890..=0x0891
                | 0x08E2
                | 0x180E
                | 0x200B..=0x200F
                | 0x2028..=0x202E
                | 0x2060..=0x2064
                | 0x2066..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x13430..=0x1343F
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0001
                | 0xE0020..=0xE007F
        )
}

fn parse_startup_failure(bytes: &[u8], pid: u32) -> Option<String> {
    if bytes.len() > MAX_STARTUP_FAILURE_RECORD_BYTES {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;
    let (header, reason) = text.split_once('\n')?;
    let recorded = header
        .strip_prefix(STARTUP_FAILURE_PID_PREFIX)?
        .parse::<u32>()
        .ok()?;
    (recorded == pid && !reason.trim().is_empty()).then(|| reason.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #2252: a link among the ancestors of the state root (macOS `/var` →
    /// `/private/var`) does not stop the daemon state from opening once the
    /// root is resolved, while a state root that is itself a link is refused.
    #[cfg(unix)]
    #[test]
    fn a_linked_ancestor_opens_and_a_linked_state_root_is_refused() {
        use unica_workspace::state_fs::state_root_with_resolved_ancestors;
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();

        let given = alias.join("provider-state");
        assert!(DaemonStateDirectory::open(&given, &CoreIdentity::production_v5()).is_err());
        let resolved = state_root_with_resolved_ancestors(&given).unwrap();
        assert_eq!(
            resolved,
            std::fs::canonicalize(&real).unwrap().join("provider-state")
        );
        DaemonStateDirectory::open(&resolved, &CoreIdentity::production_v5())
            .expect("the state opens below a linked ancestor");

        let linked_root = root.path().join("linked-state");
        std::os::unix::fs::symlink(&resolved, &linked_root).unwrap();
        let resolved = state_root_with_resolved_ancestors(&linked_root).unwrap();
        assert!(
            DaemonStateDirectory::open(&resolved, &CoreIdentity::production_v5()).is_err(),
            "a state root replaced by a link is still refused"
        );
    }

    #[test]
    fn startup_failure_record_is_bounded_single_line_and_bound_to_its_pid() {
        let root = tempfile::tempdir().unwrap();
        let physical = std::fs::canonicalize(root.path()).unwrap();
        let state = DaemonStateDirectory::open(&physical, &CoreIdentity::production_v5()).unwrap();
        assert_eq!(state.read_startup_failure(7), None);

        state.publish_startup_failure(7, "first\nrefusal").unwrap();
        assert_eq!(
            state.read_startup_failure(7).as_deref(),
            Some("first refusal")
        );
        assert_eq!(
            state.read_startup_failure(8),
            None,
            "another pid's record is stale"
        );

        let long = "ж".repeat(MAX_STARTUP_FAILURE_REASON_BYTES);
        state.publish_startup_failure(8, &long).unwrap();
        assert_eq!(
            state.read_startup_failure(7),
            None,
            "the newer record replaced it"
        );
        let bounded = state.read_startup_failure(8).unwrap();
        assert!(bounded.len() <= MAX_STARTUP_FAILURE_REASON_BYTES);
        assert!(bounded.chars().all(|character| character == 'ж'));

        state.remove_startup_failure().unwrap();
        assert_eq!(state.read_startup_failure(8), None);
        state.remove_startup_failure().unwrap();
    }

    #[test]
    fn startup_failure_record_flattens_hidden_and_line_breaking_characters() {
        let root = tempfile::tempdir().unwrap();
        let physical = std::fs::canonicalize(root.path()).unwrap();
        let state = DaemonStateDirectory::open(&physical, &CoreIdentity::production_v5()).unwrap();
        state
            .publish_startup_failure(
                9,
                "a\u{202E}b\u{2028}c\u{2029}d\u{200B}e\u{FEFF}f\u{2066}g\u{00AD}h\re",
            )
            .unwrap();
        assert_eq!(
            state.read_startup_failure(9).as_deref(),
            Some("a b c d e f g h e")
        );
    }

    #[test]
    fn startup_failure_record_sweeps_orphaned_staging_files() {
        let root = tempfile::tempdir().unwrap();
        let physical = std::fs::canonicalize(root.path()).unwrap();
        let state = DaemonStateDirectory::open(&physical, &CoreIdentity::production_v5()).unwrap();
        let stages = |state: &DaemonStateDirectory| {
            std::fs::read_dir(state.path())
                .unwrap()
                .flatten()
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| name.starts_with(STARTUP_FAILURE_STAGE_PREFIX))
                })
                .count()
        };
        let orphan = state
            .path()
            .join(format!("{STARTUP_FAILURE_STAGE_PREFIX}orphan.tmp"));
        std::fs::write(&orphan, b"pid=1\nhalf").unwrap();
        state.publish_startup_failure(10, "refusal").unwrap();
        assert_eq!(stages(&state), 0, "publication swept the orphaned stage");

        std::fs::write(&orphan, b"pid=1\nhalf").unwrap();
        state.remove_startup_failure().unwrap();
        assert_eq!(stages(&state), 0, "removal swept the orphaned stage");
        assert_eq!(state.read_startup_failure(10), None);
    }

    #[test]
    fn production_identity_is_the_frozen_v5_digest_and_the_only_daemon_protocol() {
        let production = CoreIdentity::production();

        assert_eq!(production, CoreIdentity::production_v5());
        assert_eq!(
            production.as_str(),
            "884b76181583ce34907a2a9758e2b493e5b40883e7cbb0d7f88dcec0e468cfa0"
        );
        assert_eq!(production.digest().as_str(), production.as_str());
        assert_ne!(
            production.as_str(),
            "2f4dd5713d11e5211a92c5fa01b1ec5722dc3a3160b9b1e0b667f8d8da3d9c28",
            "the retired protocol-v3 digest is not an identity of this core"
        );
        assert_eq!(DAEMON_PROTOCOL_VERSION, 5);
    }

    /// Сборка определяется байтами исполняемого файла: та же сборка под
    /// другим путём делит демон, любая пересборка получает свой.
    #[test]
    fn executable_bytes_name_the_build_and_its_daemon_directory() {
        let root = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let path = root.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        };
        let original = write("a", b"build one");
        let copy = write("b", b"build one");
        let rebuilt = write("c", b"build two");
        let identity = |path: &Path| build_identity_of_executable(path).unwrap();

        assert_eq!(identity(&original), identity(&copy));
        assert_ne!(identity(&original), identity(&rebuilt));
        assert_ne!(identity(&original), CoreIdentity::abi_v5());
        assert_ne!(
            DaemonStateDirectory::path_for(Path::new("/state"), &identity(&original)),
            DaemonStateDirectory::path_for(Path::new("/state"), &identity(&rebuilt))
        );
    }

    #[test]
    fn every_canonical_core_identity_lives_under_the_protocol_v5_state_path() {
        let state_root = Path::new("/provider-state");
        for encoded in [
            CoreIdentity::production().as_str().to_owned(),
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".to_owned(),
            "b1966ce0792d157e8716a0f29a386a2d8efe801b0abb752c342014bc6eec2d77".to_owned(),
        ] {
            let identity = CoreIdentity::from_str(&encoded).unwrap();

            assert_eq!(
                DaemonStateDirectory::path_for(state_root, &identity),
                state_root.join(format!("daemon-p5-{}", identity.as_str()))
            );
        }
    }

    #[test]
    fn expired_spawn_deadline_cannot_acquire_even_a_free_lock() {
        let root = tempfile::tempdir().unwrap();
        let state_root = std::fs::canonicalize(root.path()).unwrap();
        let state =
            DaemonStateDirectory::open(&state_root, &CoreIdentity::production_v5()).unwrap();
        assert!(state.acquire_spawn_lock_before(Instant::now()).is_err());
        assert!(state.acquire_spawn_lock(Duration::from_millis(50)).is_ok());
    }

    #[test]
    fn spawn_lock_does_not_block_child_receipt_authority_before_readiness() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let parent = DaemonStateDirectory::open(&state_root, &identity).expect("parent state");
        let child = DaemonStateDirectory::open(&state_root, &identity).expect("child state");
        let _spawn = parent
            .acquire_spawn_lock(Duration::from_millis(30))
            .expect("parent spawn authority");

        let _receipt = ReceiptAuthorityLock::acquire(&child, Duration::from_millis(30))
            .expect("child receipt authority must use a distinct stable lock identity");
    }

    #[test]
    fn each_daemon_lock_class_still_serializes_independent_state_handles() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let first = DaemonStateDirectory::open(&state_root, &identity).expect("first state");
        let second = DaemonStateDirectory::open(&state_root, &identity).expect("second state");

        let spawn = first
            .acquire_spawn_lock(Duration::from_millis(30))
            .expect("first spawn authority");
        assert!(
            second
                .acquire_spawn_lock(Duration::from_millis(20))
                .is_err(),
            "same-class spawn authorities ran concurrently"
        );
        drop(spawn);

        let receipt = ReceiptAuthorityLock::acquire(&first, Duration::from_millis(30))
            .expect("first receipt authority");
        assert!(
            ReceiptAuthorityLock::acquire(&second, Duration::from_millis(20)).is_err(),
            "same-class receipt authorities ran concurrently"
        );
        drop(receipt);
    }
}

#[cfg(test)]
#[path = "../tests/platform/identity_admission.rs"]
mod identity_admission_tests;

impl ReceiptStateRoot for DaemonStateDirectory {
    fn verify_identity(&self) -> Result<(), String> {
        DaemonStateDirectory::verify_identity(self)
    }

    fn create_private_retained_subdirectory(&self, name: &str) -> Result<StateDirectory, String> {
        DaemonStateDirectory::create_private_retained_subdirectory(self, name)
    }
}
