use super::protocol_v5::{
    parse_v5_endpoint_record, V5EndpointRecord, MAX_V5_ENDPOINT_RECORD_BYTES,
};
use crate::application::receipt_ledger::CoreIdentityDigest;
use crate::infrastructure::platform::filesystem::{
    create_owner_only_directory_child, create_owner_only_file_child, file_identity,
    open_directory_child_nofollow, open_directory_ownership_lock,
    open_or_create_absolute_directory_path_nofollow, open_regular_child_nofollow,
    remove_identity_bound_regular_child, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, verify_owner_only_acl, FileIdentity,
    RetainedDirectoryCapability,
};
use fs2::FileExt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CORE_ABI_IDENTITY: &str = "unica-v0.13-core-abi-1";
const DAEMON_PROTOCOL_IDENTITY_PREFIX: &str = "unica-daemon-jsonl-";
const ENDPOINT_FILE_NAME: &str = "endpoint.json";
const SPAWN_LOCK_NAME: &str = ".daemon-spawn.lock";
const RECEIPT_AUTHORITY_DIRECTORY_NAME: &str = ".receipt-authority";
const RECEIPT_AUTHORITY_LOCK_NAME: &str = ".receipt-authority.lock";

/// The one daemon wire protocol this core speaks; it names the state
/// directory (`daemon-p5-…`) and the identity digest.
pub(crate) const DAEMON_PROTOCOL_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoreIdentity(CoreIdentityDigest);

impl CoreIdentity {
    /// The identity the production frontend connects to: the exact protocol-v5
    /// daemon of this core ABI.
    pub(crate) fn production() -> Self {
        Self::production_v5()
    }

    pub(crate) fn production_v5() -> Self {
        let mut digest = Sha256::new();
        digest.update(CORE_ABI_IDENTITY.as_bytes());
        digest.update(b"\0");
        digest.update(DAEMON_PROTOCOL_IDENTITY_PREFIX.as_bytes());
        digest.update(DAEMON_PROTOCOL_VERSION.to_string().as_bytes());
        Self(CoreIdentityDigest::from_sha256(digest.finalize().into()))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub(crate) fn digest(&self) -> &CoreIdentityDigest {
        &self.0
    }
}

impl fmt::Display for CoreIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
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
pub(crate) struct DaemonStateDirectory {
    #[cfg(test)]
    path: PathBuf,
    directory: File,
    retained_directory: RetainedDirectoryCapability,
    identity: FileIdentity,
}

impl DaemonStateDirectory {
    pub(crate) fn path_for(state_root: &Path, core_identity: &CoreIdentity) -> PathBuf {
        state_root.join(format!(
            "daemon-p{DAEMON_PROTOCOL_VERSION}-{}",
            core_identity.as_str()
        ))
    }

    pub(crate) fn open(state_root: &Path, core_identity: &CoreIdentity) -> Result<Self, String> {
        if !state_root.is_absolute() {
            return Err("daemon state root must be absolute".to_string());
        }
        let parent = open_or_create_absolute_directory_path_nofollow(state_root)
            .map_err(|error| daemon_io_error("open or create daemon provider state root", error))?;
        let child_name = format!(
            "daemon-p{DAEMON_PROTOCOL_VERSION}-{}",
            core_identity.as_str()
        );
        let directory = match open_directory_child_nofollow(&parent, OsStr::new(&child_name)) {
            Ok(directory) => directory,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_owner_only_directory_child(&parent, OsStr::new(&child_name)) {
                    Ok(directory) => directory,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        open_directory_child_nofollow(&parent, OsStr::new(&child_name)).map_err(
                            |error| daemon_io_error("open raced daemon identity directory", error),
                        )?
                    }
                    Err(error) => {
                        return Err(daemon_io_error(
                            "create private daemon identity directory",
                            error,
                        ))
                    }
                }
            }
            Err(error) => {
                return Err(daemon_io_error(
                    "open private daemon identity directory",
                    error,
                ))
            }
        };
        verify_owner_only_acl(&directory).map_err(|error| {
            daemon_io_error("daemon identity directory is not owner-only", error)
        })?;
        let identity = file_identity(&directory)
            .map_err(|error| daemon_io_error("identify daemon identity directory", error))?;
        let path = state_root.join(&child_name);
        let retained_directory = RetainedDirectoryCapability::open(&path)
            .map_err(|error| daemon_io_error("retain named daemon identity directory", error))?;
        if retained_directory.identity() != identity {
            return Err("named daemon identity directory changed during admission".to_string());
        }
        retained_directory
            .validate_named_identity()
            .map_err(|error| daemon_io_error("validate named daemon identity directory", error))?;
        Ok(Self {
            #[cfg(test)]
            path,
            directory,
            retained_directory,
            identity,
        })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn create_private_subdirectory(&self, name: &str) -> Result<File, String> {
        self.verify_identity()?;
        let child = match open_directory_child_nofollow(&self.directory, OsStr::new(name)) {
            Ok(child) => child,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_owner_only_directory_child(&self.directory, OsStr::new(name)) {
                    Ok(child) => child,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        open_directory_child_nofollow(&self.directory, OsStr::new(name)).map_err(
                            |error| daemon_io_error("open raced private daemon child", error),
                        )?
                    }
                    Err(error) => {
                        return Err(daemon_io_error("create private daemon child", error))
                    }
                }
            }
            Err(error) => return Err(daemon_io_error("open private daemon child", error)),
        };
        verify_owner_only_acl(&child)
            .map_err(|error| daemon_io_error("daemon child is not owner-only", error))?;
        Ok(child)
    }

    pub(crate) fn create_private_retained_subdirectory(
        &self,
        name: &str,
    ) -> Result<RetainedDirectoryCapability, String> {
        let child = self.create_private_subdirectory(name)?;
        let expected_identity = file_identity(&child)
            .map_err(|error| daemon_io_error("identify private daemon child", error))?;
        let retained = self
            .retained_directory
            .retain_directory_child(OsStr::new(name))
            .map_err(|error| daemon_io_error("retain named private daemon child", error))?;
        if retained.identity() != expected_identity {
            return Err("named private daemon child changed during admission".to_string());
        }
        retained
            .validate_named_identity()
            .map_err(|error| daemon_io_error("validate named private daemon child", error))?;
        Ok(retained)
    }

    pub(crate) fn acquire_spawn_lock(&self, timeout: Duration) -> Result<SpawnLock, String> {
        self.verify_identity()?;
        let file = open_directory_ownership_lock(&self.directory, OsStr::new(SPAWN_LOCK_NAME))
            .map_err(|error| daemon_io_error("open daemon spawn ownership object", error))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(SpawnLock { file }),
                Err(error) if lock_is_contended(&error) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if lock_is_contended(&error) => {
                    return Err("timed out waiting for daemon spawn ownership".to_string())
                }
                Err(error) => {
                    return Err(daemon_io_error("lock daemon spawn ownership object", error))
                }
            }
        }
    }

    pub(crate) fn acquire_receipt_authority(
        &self,
        timeout: Duration,
    ) -> Result<ReceiptAuthorityLock, String> {
        self.verify_identity()?;
        // Unix ownership locks are directory-inode scoped. A dedicated retained
        // child keeps receipt authority distinct from the state-inode spawn gate.
        let authority_directory =
            self.create_private_retained_subdirectory(RECEIPT_AUTHORITY_DIRECTORY_NAME)?;
        let authority_file = authority_directory
            .try_clone_directory()
            .map_err(|error| daemon_io_error("clone receipt authority directory", error))?;
        let file =
            open_directory_ownership_lock(&authority_file, OsStr::new(RECEIPT_AUTHORITY_LOCK_NAME))
                .map_err(|error| {
                    daemon_io_error("open receipt authority ownership object", error)
                })?;
        verify_owner_only_acl(&file)
            .map_err(|error| daemon_io_error("verify receipt authority ownership object", error))?;
        let deadline = Instant::now() + timeout;
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => {
                    authority_directory
                        .validate_named_identity()
                        .map_err(|error| {
                            daemon_io_error("validate named receipt authority directory", error)
                        })?;
                    return Ok(ReceiptAuthorityLock {
                        file,
                        _authority_directory: authority_directory,
                    });
                }
                Err(error) if lock_is_contended(&error) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) if lock_is_contended(&error) => {
                    return Err("timed out waiting for stable receipt authority".to_string())
                }
                Err(error) => {
                    return Err(daemon_io_error(
                        "lock receipt authority ownership object",
                        error,
                    ))
                }
            }
        }
    }

    pub(crate) fn read_v5_endpoint_record(&self) -> Result<Option<V5EndpointRecord>, String> {
        Ok(self
            .read_v5_endpoint_record_retained()?
            .map(|retained| retained.record))
    }

    fn read_v5_endpoint_record_retained(&self) -> Result<Option<RetainedV5EndpointRecord>, String> {
        self.verify_identity()?;
        let mut file =
            match open_regular_child_nofollow(&self.directory, OsStr::new(ENDPOINT_FILE_NAME)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(daemon_io_error("open v5 daemon endpoint record", error)),
            };
        verify_owner_only_acl(&file).map_err(|error| {
            daemon_io_error("v5 daemon endpoint record is not owner-only", error)
        })?;
        let file_identity = file_identity(&file)
            .map_err(|error| daemon_io_error("identify v5 daemon endpoint record", error))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((MAX_V5_ENDPOINT_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| daemon_io_error("read v5 daemon endpoint record", error))?;
        if bytes.len() > MAX_V5_ENDPOINT_RECORD_BYTES {
            return Err("v5 daemon endpoint record exceeds the byte limit".to_string());
        }
        let record = parse_v5_endpoint_record(&bytes)?;
        Ok(Some(RetainedV5EndpointRecord {
            record,
            file,
            identity: file_identity,
        }))
    }

    pub(crate) fn publish_v5_endpoint_record(
        &self,
        record: &V5EndpointRecord,
    ) -> Result<RetainedV5EndpointRecord, String> {
        record.validate()?;
        self.verify_identity()?;
        let temporary_name = format!(".endpoint.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_owner_only_file_child(&self.directory, temporary_name)
            .map_err(|error| daemon_io_error("create private v5 endpoint staging file", error))?;
        let staged_identity = file_identity(&file)
            .map_err(|error| daemon_io_error("identify v5 endpoint staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&file) {
            let _ = remove_identity_bound_regular_child(
                &self.directory,
                temporary_name,
                staged_identity,
                &file,
            );
            return Err(daemon_io_error("restrict v5 endpoint staging file", error));
        }
        let mut bytes = serde_json::to_vec(record)
            .map_err(|_| "v5 daemon endpoint record could not be serialized".to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_V5_ENDPOINT_RECORD_BYTES {
            return Err("v5 daemon endpoint record exceeds the byte limit".to_string());
        }
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ = remove_identity_bound_regular_child(
                &self.directory,
                temporary_name,
                staged_identity,
                &file,
            );
            return Err(daemon_io_error("flush v5 endpoint staging file", error));
        }
        replace_identity_bound_regular_child(
            &self.directory,
            temporary_name,
            staged_identity,
            &file,
            OsStr::new(ENDPOINT_FILE_NAME),
        )
        .map_err(|error| daemon_io_error("publish v5 endpoint record", error))?;
        sync_directory(&self.directory)
            .map_err(|error| daemon_io_error("sync v5 endpoint publication", error))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| daemon_io_error("rewind v5 endpoint record", error))?;
        Ok(RetainedV5EndpointRecord {
            record: record.clone(),
            file,
            identity: staged_identity,
        })
    }

    pub(crate) fn remove_v5_endpoint_if_owned(
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
        remove_identity_bound_regular_child(
            &self.directory,
            OsStr::new(ENDPOINT_FILE_NAME),
            retained.identity,
            &retained.file,
        )
        .map_err(|error| daemon_io_error("remove owned v5 endpoint record", error))?;
        sync_directory(&self.directory)
            .map_err(|error| daemon_io_error("sync v5 endpoint removal", error))?;
        Ok(true)
    }

    pub(crate) fn remove_matching_v5_endpoint_record(
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

    fn verify_identity(&self) -> Result<(), String> {
        self.retained_directory
            .validate_named_identity()
            .map_err(|error| daemon_io_error("validate named daemon identity directory", error))?;
        let actual = file_identity(&self.directory)
            .map_err(|error| daemon_io_error("verify daemon directory identity", error))?;
        if actual != self.identity {
            return Err("retained daemon directory identity changed".to_string());
        }
        verify_owner_only_acl(&self.directory)
            .map_err(|error| daemon_io_error("daemon identity directory is not owner-only", error))
    }
}

#[derive(Debug)]
pub(crate) struct RetainedV5EndpointRecord {
    pub(crate) record: V5EndpointRecord,
    file: File,
    identity: FileIdentity,
}

pub(crate) struct SpawnLock {
    file: File,
}

pub(crate) struct ReceiptAuthorityLock {
    file: File,
    _authority_directory: RetainedDirectoryCapability,
}

impl Drop for SpawnLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

impl Drop for ReceiptAuthorityLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

fn lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn daemon_io_error(operation: &str, error: io::Error) -> String {
    format!("{operation}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn spawn_lock_does_not_block_child_receipt_authority_before_readiness() {
        let root = tempfile::tempdir().expect("temporary state root");
        let state_root = std::fs::canonicalize(root.path()).expect("physical state root");
        let identity = CoreIdentity::production_v5();
        let parent = DaemonStateDirectory::open(&state_root, &identity).expect("parent state");
        let child = DaemonStateDirectory::open(&state_root, &identity).expect("child state");
        let _spawn = parent
            .acquire_spawn_lock(Duration::from_millis(30))
            .expect("parent spawn authority");

        let _receipt = child
            .acquire_receipt_authority(Duration::from_millis(30))
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

        let receipt = first
            .acquire_receipt_authority(Duration::from_millis(30))
            .expect("first receipt authority");
        assert!(
            second
                .acquire_receipt_authority(Duration::from_millis(20))
                .is_err(),
            "same-class receipt authorities ran concurrently"
        );
        drop(receipt);
    }
}
