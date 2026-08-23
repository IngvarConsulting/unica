use super::protocol::{parse_endpoint_record, EndpointRecord, MAX_JSON_LINE_BYTES};
use crate::infrastructure::platform::filesystem::{
    create_owner_only_directory_child, create_owner_only_file_child, file_identity,
    open_directory_child_nofollow, open_directory_ownership_lock,
    open_or_create_absolute_directory_path_nofollow, open_regular_child_nofollow,
    remove_identity_bound_regular_child, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, verify_owner_only_acl, FileIdentity,
};
use fs2::FileExt;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};
use uuid::Uuid;

const CORE_ABI_IDENTITY: &str = "unica-v0.13-core-abi-1";
const DAEMON_PROTOCOL_IDENTITY: &str = "unica-daemon-jsonl-2";
const ENDPOINT_FILE_NAME: &str = "endpoint.json";
const SPAWN_LOCK_NAME: &str = ".daemon-spawn.lock";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoreIdentity(String);

impl CoreIdentity {
    pub(crate) fn production() -> Self {
        let mut digest = Sha256::new();
        digest.update(CORE_ABI_IDENTITY.as_bytes());
        digest.update(b"\0");
        digest.update(DAEMON_PROTOCOL_IDENTITY.as_bytes());
        Self(hex_digest(digest.finalize().as_slice()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CoreIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for CoreIdentity {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("core identity must be exactly 64 lowercase hexadecimal bytes".to_string());
        }
        Ok(Self(value.to_string()))
    }
}

impl Serialize for CoreIdentity {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
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

fn hex_digest(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[derive(Debug)]
pub(crate) struct DaemonStateDirectory {
    #[cfg(test)]
    path: PathBuf,
    directory: File,
    identity: FileIdentity,
}

impl DaemonStateDirectory {
    #[cfg(test)]
    pub(crate) fn path_for(state_root: &Path, core_identity: &CoreIdentity) -> PathBuf {
        state_root.join(format!(
            "daemon-p{}-{}",
            super::protocol::DAEMON_PROTOCOL_VERSION,
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
            "daemon-p{}-{}",
            super::protocol::DAEMON_PROTOCOL_VERSION,
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
        Ok(Self {
            #[cfg(test)]
            path: state_root.join(child_name),
            directory,
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

    pub(crate) fn read_endpoint_record(&self) -> Result<Option<EndpointRecord>, String> {
        Ok(self
            .read_endpoint_record_retained()?
            .map(|retained| retained.record))
    }

    pub(crate) fn read_endpoint_record_retained(
        &self,
    ) -> Result<Option<RetainedEndpointRecord>, String> {
        self.verify_identity()?;
        let mut file =
            match open_regular_child_nofollow(&self.directory, OsStr::new(ENDPOINT_FILE_NAME)) {
                Ok(file) => file,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
                Err(error) => return Err(daemon_io_error("open daemon endpoint record", error)),
            };
        verify_owner_only_acl(&file)
            .map_err(|error| daemon_io_error("daemon endpoint record is not owner-only", error))?;
        let file_identity = file_identity(&file)
            .map_err(|error| daemon_io_error("identify daemon endpoint record", error))?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take((MAX_JSON_LINE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| daemon_io_error("read daemon endpoint record", error))?;
        if bytes.len() > MAX_JSON_LINE_BYTES {
            return Err("daemon endpoint record exceeds the byte limit".to_string());
        }
        let record = parse_endpoint_record(&bytes)?;
        Ok(Some(RetainedEndpointRecord {
            record,
            file,
            identity: file_identity,
        }))
    }

    pub(crate) fn publish_endpoint_record(
        &self,
        record: &EndpointRecord,
    ) -> Result<RetainedEndpointRecord, String> {
        record.validate()?;
        self.verify_identity()?;
        let temporary_name = format!(".endpoint.{}.tmp", Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_owner_only_file_child(&self.directory, temporary_name)
            .map_err(|error| daemon_io_error("create private endpoint staging file", error))?;
        let staged_identity = file_identity(&file)
            .map_err(|error| daemon_io_error("identify endpoint staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&file) {
            let _ = remove_identity_bound_regular_child(
                &self.directory,
                temporary_name,
                staged_identity,
                &file,
            );
            return Err(daemon_io_error("restrict endpoint staging file", error));
        }
        let mut bytes = serde_json::to_vec(record)
            .map_err(|_| "daemon endpoint record could not be serialized".to_string())?;
        bytes.push(b'\n');
        if bytes.len() > MAX_JSON_LINE_BYTES {
            return Err("daemon endpoint record exceeds the byte limit".to_string());
        }
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ = remove_identity_bound_regular_child(
                &self.directory,
                temporary_name,
                staged_identity,
                &file,
            );
            return Err(daemon_io_error("flush endpoint staging file", error));
        }
        replace_identity_bound_regular_child(
            &self.directory,
            temporary_name,
            staged_identity,
            &file,
            OsStr::new(ENDPOINT_FILE_NAME),
        )
        .map_err(|error| daemon_io_error("publish endpoint record", error))?;
        sync_directory(&self.directory)
            .map_err(|error| daemon_io_error("sync endpoint publication", error))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|error| daemon_io_error("rewind endpoint record", error))?;
        Ok(RetainedEndpointRecord {
            record: record.clone(),
            file,
            identity: staged_identity,
        })
    }

    pub(crate) fn remove_endpoint_if_owned(
        &self,
        retained: &RetainedEndpointRecord,
    ) -> Result<bool, String> {
        self.verify_identity()?;
        let Some(current) = self.read_endpoint_record_retained()? else {
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
        .map_err(|error| daemon_io_error("remove owned endpoint record", error))?;
        sync_directory(&self.directory)
            .map_err(|error| daemon_io_error("sync endpoint removal", error))?;
        Ok(true)
    }

    fn verify_identity(&self) -> Result<(), String> {
        let actual = file_identity(&self.directory)
            .map_err(|error| daemon_io_error("verify daemon directory identity", error))?;
        if actual != self.identity {
            return Err("retained daemon directory identity changed".to_string());
        }
        verify_owner_only_acl(&self.directory)
            .map_err(|error| daemon_io_error("daemon identity directory is not owner-only", error))
    }

    #[cfg(test)]
    pub(crate) fn write_endpoint_record_for_test(
        &self,
        record: &EndpointRecord,
    ) -> Result<(), String> {
        self.publish_endpoint_record(record).map(|_| ())
    }
}

#[derive(Debug)]
pub(crate) struct RetainedEndpointRecord {
    pub(crate) record: EndpointRecord,
    file: File,
    identity: FileIdentity,
}

pub(crate) struct SpawnLock {
    file: File,
}

impl Drop for SpawnLock {
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
