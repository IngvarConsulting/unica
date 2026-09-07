//! Sole-writer durable Task and Invocation store used by the versioned daemon.

use crate::application::invocation_store::{
    canonical_result_size, store_operation_checkpoint, CanonicalResultSizeError, CommitOperation,
    EpochMillisClock, InvocationStore, InvocationStoreError, NewInvocationRecord,
    SafeFailureReason, SafeStatusMessage, StoredInvocationRecord, TaskTransition, ToolIdentity,
    INVOCATION_RECORD_SCHEMA_VERSION, LEGACY_INVOCATION_RECORD_SCHEMA_VERSION,
    MAX_CANONICAL_RESULT_BYTES, MAX_TASK_RECORD_BYTES,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{
    DomainResult, InvocationId, InvocationStatus, NormalizedArgumentsHash, ResumeDescriptor,
    SafeIdentityHash, TaskId,
};
use crate::infrastructure::platform::filesystem::{
    create_new_regular_child, file_identity, metadata_is_link_or_reparse_point,
    open_directory_nofollow, open_directory_ownership_lock, open_regular_child_nofollow,
    read_directory_names_bounded, remove_identity_bound_regular_child,
    rename_identity_bound_regular_child_no_replace, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, FileIdentity,
};
use fs2::FileExt;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SystemEpochMillisClock;

impl EpochMillisClock for SystemEpochMillisClock {
    fn now_epoch_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecoveryClassification {
    DiscardedTemporary { file_name: String },
    InterruptedNonResumable { task_id: TaskId },
    UnsupportedResume { task_id: TaskId },
    MigratedV1Record { task_id: TaskId },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RecoveryReport {
    pub(crate) classifications: Vec<RecoveryClassification>,
}

const STORE_LOCK_FILE: &str = ".invocation-store.lock";
const STORE_OPERATION_BUDGET: Duration = Duration::from_secs(7);
const STORE_WRITER_WAIT_SLICE: Duration = Duration::from_millis(10);
const MAX_TASK_RECORDS: usize = 4_096;
const MAX_RECOVERY_EXTRA_ENTRIES: usize = 64;

#[derive(Debug, Clone, Copy)]
struct StoreLimits {
    max_records: usize,
    max_record_bytes: usize,
}

impl StoreLimits {
    const fn production() -> Self {
        Self {
            max_records: MAX_TASK_RECORDS,
            max_record_bytes: MAX_TASK_RECORD_BYTES,
        }
    }
}

#[derive(Default)]
struct StoreCatalog {
    records: HashMap<TaskId, RetentionRecord>,
}

#[derive(Clone, Copy)]
struct RetentionRecord {
    terminal: bool,
    updated_at_epoch_ms: u64,
    ttl_ms: u64,
}

impl From<&StoredInvocationRecord> for RetentionRecord {
    fn from(record: &StoredInvocationRecord) -> Self {
        Self {
            terminal: record.is_terminal(),
            updated_at_epoch_ms: record.updated_at_epoch_ms,
            ttl_ms: record.ttl_ms,
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LegacyStoredInvocationRecordV1 {
    schema_version: u32,
    task_id: TaskId,
    invocation_id: InvocationId,
    tool: ToolIdentity,
    normalized_arguments_hash: NormalizedArgumentsHash,
    workspace_identity_hash: SafeIdentityHash,
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    status: InvocationStatus,
    status_message: SafeStatusMessage,
    poll_interval_ms: u64,
    ttl_ms: u64,
    #[serde(default)]
    result: Option<DomainResult>,
    #[serde(default)]
    resume: Option<ResumeDescriptor>,
}

impl LegacyStoredInvocationRecordV1 {
    fn migrate(self) -> Result<StoredInvocationRecord, InvocationStoreError> {
        if self.schema_version != LEGACY_INVOCATION_RECORD_SCHEMA_VERSION {
            return Err(corrupt_error(
                "legacy task record schema version is invalid",
            ));
        }
        let failure_reason = (self.status == InvocationStatus::Failed).then_some(
            if self.status_message == SafeStatusMessage::Interrupted {
                SafeFailureReason::Interrupted
            } else {
                SafeFailureReason::InvocationFailed
            },
        );
        let record = StoredInvocationRecord {
            schema_version: INVOCATION_RECORD_SCHEMA_VERSION,
            task_id: self.task_id,
            invocation_id: self.invocation_id,
            tool: self.tool,
            normalized_arguments_hash: self.normalized_arguments_hash,
            workspace_identity_hash: self.workspace_identity_hash,
            created_at_epoch_ms: self.created_at_epoch_ms,
            updated_at_epoch_ms: self.updated_at_epoch_ms,
            status: self.status,
            status_message: self.status_message,
            poll_interval_ms: self.poll_interval_ms,
            ttl_ms: self.ttl_ms,
            result: self.result,
            failure_reason,
            resume: self.resume,
        };
        validate_record(&record)?;
        Ok(record)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodedSchema {
    V1,
    V2,
}

struct DecodedRecord {
    record: StoredInvocationRecord,
    schema: DecodedSchema,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationFailure {
    BeforeRename,
    AfterRenameBeforeSync,
}

/// One daemon owns one instance. The mutex serializes that daemon's writes;
/// readers observe either the previous committed file or its complete replacement.
pub(crate) struct FileInvocationStore {
    root: File,
    root_identity: FileIdentity,
    _root_lock: File,
    clock: Arc<dyn EpochMillisClock>,
    writer: Mutex<StoreCatalog>,
    limits: StoreLimits,
    #[cfg(test)]
    next_publication_failure: Mutex<Option<PublicationFailure>>,
}

impl FileInvocationStore {
    pub(crate) fn open(
        root: impl AsRef<Path>,
        clock: Arc<dyn EpochMillisClock>,
    ) -> Result<(Self, RecoveryReport), InvocationStoreError> {
        let root = root.as_ref().to_path_buf();
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| storage_error("inspect task store root", error))?;
        if !metadata.is_dir() || metadata_is_link_or_reparse_point(&metadata) {
            return Err(InvocationStoreError::Storage(
                "task store root must be a private physical directory".to_string(),
            ));
        }
        let root = open_directory_nofollow(&root)
            .map_err(|error| storage_error("open task store root", error))?;
        Self::open_retained_directory_with_limits(root, clock, StoreLimits::production())
    }

    pub(crate) fn open_retained_directory(
        root: File,
        clock: Arc<dyn EpochMillisClock>,
    ) -> Result<(Self, RecoveryReport), InvocationStoreError> {
        Self::open_retained_directory_with_limits(root, clock, StoreLimits::production())
    }

    fn open_retained_directory_with_limits(
        root: File,
        clock: Arc<dyn EpochMillisClock>,
        limits: StoreLimits,
    ) -> Result<(Self, RecoveryReport), InvocationStoreError> {
        let root_identity = file_identity(&root)
            .map_err(|error| storage_error("capture task store root identity", error))?;
        let root_lock = acquire_root_lock(&root)?;

        let store = Self {
            root,
            root_identity,
            _root_lock: root_lock,
            clock,
            writer: Mutex::new(StoreCatalog::default()),
            limits,
            #[cfg(test)]
            next_publication_failure: Mutex::new(None),
        };
        let report = store.recover()?;
        Ok((store, report))
    }

    #[cfg(test)]
    fn open_with_limits_for_test(
        root: impl AsRef<Path>,
        clock: Arc<dyn EpochMillisClock>,
        max_records: usize,
        max_record_bytes: usize,
    ) -> Result<(Self, RecoveryReport), InvocationStoreError> {
        let root = root.as_ref();
        let retained = open_directory_nofollow(root)
            .map_err(|error| storage_error("open task store root", error))?;
        Self::open_retained_directory_with_limits(
            retained,
            clock,
            StoreLimits {
                max_records,
                max_record_bytes,
            },
        )
    }

    fn recover(&self) -> Result<RecoveryReport, InvocationStoreError> {
        let deadline = ProviderDeadline::from_budget(STORE_OPERATION_BUDGET);
        let cancellation = CancellationToken::new();
        let mut writer = self.lock_writer_before(deadline, &cancellation)?;
        self.verify_root_identity()?;
        let maximum_entries = self
            .limits
            .max_records
            .saturating_add(MAX_RECOVERY_EXTRA_ENTRIES)
            .saturating_add(1);
        let entries = read_directory_names_bounded(&self.root, maximum_entries, || {
            store_operation_checkpoint(deadline, &cancellation).map_err(store_checkpoint_io_error)
        })
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::TimedOut {
                InvocationStoreError::DeadlineExceeded
            } else if error.kind() == std::io::ErrorKind::Interrupted {
                InvocationStoreError::Cancelled
            } else if error.kind() == std::io::ErrorKind::FileTooLarge {
                InvocationStoreError::Capacity {
                    max_records: self.limits.max_records,
                }
            } else {
                storage_error("enumerate task store", error)
            }
        })?;
        let mut report = RecoveryReport::default();

        for entry_name in entries {
            store_operation_checkpoint(deadline, &cancellation)?;
            let file_name = entry_name
                .clone()
                .into_string()
                .map_err(|_| corrupt_error("task store entry name is not UTF-8"))?;
            if file_name == STORE_LOCK_FILE {
                continue;
            }
            if file_name.starts_with('.') && file_name.ends_with(".tmp") {
                let retained = open_regular_child_nofollow(&self.root, &entry_name)
                    .map_err(|_| corrupt_error("temporary task entry is not a regular file"))?;
                let identity = file_identity(&retained)
                    .map_err(|error| storage_error("identify temporary task record", error))?;
                remove_identity_bound_regular_child(&self.root, &entry_name, identity, &retained)
                    .map_err(|error| storage_error("discard temporary task record", error))?;
                sync_directory(&self.root)
                    .map_err(|error| storage_error("sync task store cleanup", error))?;
                report
                    .classifications
                    .push(RecoveryClassification::DiscardedTemporary { file_name });
                continue;
            }
            if !file_name.ends_with(".json") {
                return Err(corrupt_error("unexpected task store entry"));
            }

            let encoded_task_id = file_name
                .strip_suffix(".json")
                .expect("suffix checked above");
            let task_id = encoded_task_id
                .parse::<TaskId>()
                .map_err(|_| corrupt_error("task record file name is not a canonical TaskId"))?;
            let decoded =
                Self::read_decoded_from(&self.root, task_id, self.limits.max_record_bytes)?;
            store_operation_checkpoint(deadline, &cancellation)?;
            let mut record = decoded.record;
            if self.is_expired(&record) {
                self.remove_record_file(task_id)?;
                continue;
            }
            if writer.records.len() >= self.limits.max_records {
                return Err(InvocationStoreError::Capacity {
                    max_records: self.limits.max_records,
                });
            }
            if matches!(
                record.status,
                InvocationStatus::Queued | InvocationStatus::Working
            ) {
                let (reason, status_message, classification) = if record.resume.is_some() {
                    (
                        SafeFailureReason::ResumeUnsupported,
                        SafeStatusMessage::Failed,
                        RecoveryClassification::UnsupportedResume { task_id },
                    )
                } else {
                    (
                        SafeFailureReason::Interrupted,
                        SafeStatusMessage::Interrupted,
                        RecoveryClassification::InterruptedNonResumable { task_id },
                    )
                };
                record.status = InvocationStatus::Working;
                let recovered = self.transition_record(
                    record,
                    TaskTransition::Fail {
                        status_message,
                        reason,
                    },
                )?;
                self.publish_record(
                    &mut writer,
                    &recovered,
                    CommitOperation::Recovery,
                    deadline,
                    &cancellation,
                    || {},
                )?;
                store_operation_checkpoint(deadline, &cancellation)?;
                record = recovered;
                report.classifications.push(classification);
            } else if decoded.schema == DecodedSchema::V1 {
                self.publish_record(
                    &mut writer,
                    &record,
                    CommitOperation::Recovery,
                    deadline,
                    &cancellation,
                    || {},
                )?;
                store_operation_checkpoint(deadline, &cancellation)?;
                report
                    .classifications
                    .push(RecoveryClassification::MigratedV1Record { task_id });
            }
            writer
                .records
                .insert(task_id, RetentionRecord::from(&record));
        }
        Ok(report)
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, StoreCatalog>, InvocationStoreError> {
        self.lock_writer_before(
            ProviderDeadline::from_budget(STORE_OPERATION_BUDGET),
            &CancellationToken::new(),
        )
    }

    fn lock_writer_before(
        &self,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<MutexGuard<'_, StoreCatalog>, InvocationStoreError> {
        loop {
            store_operation_checkpoint(deadline, cancellation)?;
            match self.writer.try_lock() {
                Ok(guard) => {
                    store_operation_checkpoint(deadline, cancellation)?;
                    return Ok(guard);
                }
                Err(TryLockError::Poisoned(poisoned)) => {
                    store_operation_checkpoint(deadline, cancellation)?;
                    return Ok(poisoned.into_inner());
                }
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(deadline.remaining().min(STORE_WRITER_WAIT_SLICE));
                }
            }
        }
    }

    #[cfg(test)]
    fn hold_writer_for_test(&self) -> MutexGuard<'_, StoreCatalog> {
        self.writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn verify_root_identity(&self) -> Result<(), InvocationStoreError> {
        let current = file_identity(&self.root)
            .map_err(|error| storage_error("verify task store root identity", error))?;
        if current != self.root_identity {
            return Err(InvocationStoreError::Storage(
                "retained task store root identity changed".to_string(),
            ));
        }
        Ok(())
    }

    fn remove_record_file(&self, task_id: TaskId) -> Result<(), InvocationStoreError> {
        let file_name = format!("{task_id}.json");
        let name = OsStr::new(&file_name);
        let retained = open_regular_child_nofollow(&self.root, name)
            .map_err(|error| storage_error("retain expired task record", error))?;
        let identity = file_identity(&retained)
            .map_err(|error| storage_error("identify expired task record", error))?;
        remove_identity_bound_regular_child(&self.root, name, identity, &retained)
            .map_err(|error| storage_error("remove expired task record", error))?;
        sync_directory(&self.root)
            .map_err(|error| storage_error("sync expired task record removal", error))
    }

    fn read_record(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.verify_root_identity()?;
        Self::read_committed_from(&self.root, task_id, self.limits.max_record_bytes)
    }

    fn read_committed_from(
        root: &File,
        task_id: TaskId,
        max_record_bytes: usize,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let file_name = format!("{task_id}.json");
        let file = match open_regular_child_nofollow(root, OsStr::new(&file_name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(InvocationStoreError::NotFound)
            }
            Err(error) => return Err(storage_error("open committed task record", error)),
        };
        let limit = u64::try_from(max_record_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read committed task record", error))?;
        if bytes.len() > max_record_bytes {
            return Err(InvocationStoreError::RecordTooLarge {
                max_bytes: max_record_bytes,
            });
        }
        let record = Self::decode_record(&bytes)?.record;
        if record.task_id != task_id {
            return Err(corrupt_error(
                "task record identity does not match its file name",
            ));
        }
        Ok(record)
    }

    fn read_decoded_from(
        root: &File,
        task_id: TaskId,
        max_record_bytes: usize,
    ) -> Result<DecodedRecord, InvocationStoreError> {
        let file_name = format!("{task_id}.json");
        let file = match open_regular_child_nofollow(root, OsStr::new(&file_name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(InvocationStoreError::NotFound)
            }
            Err(error) => return Err(storage_error("open committed task record", error)),
        };
        let limit = u64::try_from(max_record_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read committed task record", error))?;
        if bytes.len() > max_record_bytes {
            return Err(InvocationStoreError::RecordTooLarge {
                max_bytes: max_record_bytes,
            });
        }
        let decoded = Self::decode_record(&bytes)?;
        if decoded.record.task_id != task_id {
            return Err(corrupt_error(
                "task record identity does not match its file name",
            ));
        }
        Ok(decoded)
    }

    fn decode_record(bytes: &[u8]) -> Result<DecodedRecord, InvocationStoreError> {
        let envelope: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|_| corrupt_error("committed task record is not valid versioned JSON"))?;
        let schema_version = envelope
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| corrupt_error("task record schema version is missing"))?;
        match schema_version {
            value if value == u64::from(LEGACY_INVOCATION_RECORD_SCHEMA_VERSION) => {
                let legacy: LegacyStoredInvocationRecordV1 = serde_json::from_slice(bytes)
                    .map_err(|_| corrupt_error("schema-v1 task record is not strict JSON"))?;
                Ok(DecodedRecord {
                    record: legacy.migrate()?,
                    schema: DecodedSchema::V1,
                })
            }
            value if value == u64::from(INVOCATION_RECORD_SCHEMA_VERSION) => {
                let record: StoredInvocationRecord = serde_json::from_slice(bytes)
                    .map_err(|_| corrupt_error("schema-v2 task record is not strict JSON"))?;
                validate_record(&record)?;
                Ok(DecodedRecord {
                    record,
                    schema: DecodedSchema::V2,
                })
            }
            _ => Err(corrupt_error("unsupported task record schema version")),
        }
    }

    #[cfg(test)]
    fn read_committed(
        root: &Path,
        task_id: TaskId,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let directory = open_directory_nofollow(root)
            .map_err(|error| storage_error("open task store for test reading", error))?;
        Self::read_committed_from(&directory, task_id, MAX_TASK_RECORD_BYTES)
    }

    fn publish_record<F>(
        &self,
        catalog: &mut StoreCatalog,
        record: &StoredInvocationRecord,
        operation: CommitOperation,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        before_replace: F,
    ) -> Result<(), InvocationStoreError>
    where
        F: FnOnce(),
    {
        validate_record(record)?;
        store_operation_checkpoint(deadline, cancellation)?;
        self.verify_root_identity()?;
        let target_name = format!("{}.json", record.task_id);
        let temporary_name = format!(".{}.{}.tmp", record.task_id, Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut file = create_new_regular_child(&self.root, temporary_name)
            .map_err(|error| storage_error("create private task staging file", error))?;
        let temporary_identity = file_identity(&file)
            .map_err(|error| storage_error("identify private task staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&file) {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(storage_error("restrict task staging file", error));
        }
        let serialization = serialize_record_bounded(
            &mut file,
            record,
            self.limits.max_record_bytes,
            deadline,
            cancellation,
        );
        if let Err(error) = serialization {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(error);
        }
        store_operation_checkpoint(deadline, cancellation)?;
        if let Err(error) = file.sync_all() {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(storage_error("flush task staging file", error));
        }

        store_operation_checkpoint(deadline, cancellation)?;
        before_replace();
        #[cfg(test)]
        let injected_failure = self.take_publication_failure()?;
        #[cfg(test)]
        if injected_failure == Some(PublicationFailure::BeforeRename) {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(InvocationStoreError::Storage(
                "atomically publish task record: injected pre-commit rename failure".to_string(),
            ));
        }
        let target_name = OsStr::new(&target_name);
        let publication = if operation == CommitOperation::Create {
            rename_identity_bound_regular_child_no_replace(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
                &self.root,
                target_name,
            )
        } else {
            replace_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
                target_name,
            )
        };
        if let Err(error) = publication {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            if operation == CommitOperation::Create
                && error.kind() == std::io::ErrorKind::AlreadyExists
            {
                return Err(InvocationStoreError::TaskIdCollision {
                    task_id: record.task_id,
                });
            }
            return Err(storage_error("atomically publish task record", error));
        }
        // The record is visible after the atomic rename even when directory
        // durability cannot be confirmed. Retention therefore changes at this
        // exact visibility point, before the fallible directory sync.
        catalog
            .records
            .insert(record.task_id, RetentionRecord::from(record));
        #[cfg(test)]
        if injected_failure == Some(PublicationFailure::AfterRenameBeforeSync) {
            return Err(InvocationStoreError::CommitUncertain {
                task_id: record.task_id,
                operation,
            });
        }
        store_operation_checkpoint(deadline, cancellation).map_err(|_| {
            InvocationStoreError::CommitUncertain {
                task_id: record.task_id,
                operation,
            }
        })?;
        sync_directory(&self.root).map_err(|_| InvocationStoreError::CommitUncertain {
            task_id: record.task_id,
            operation,
        })
    }

    #[cfg(test)]
    fn inject_next_publication_failure(&self, failure: PublicationFailure) {
        *self
            .next_publication_failure
            .lock()
            .expect("publication failure lock") = Some(failure);
    }

    #[cfg(test)]
    fn take_publication_failure(&self) -> Result<Option<PublicationFailure>, InvocationStoreError> {
        self.next_publication_failure
            .lock()
            .map(|mut failure| failure.take())
            .map_err(|_| InvocationStoreError::Storage("publication failure lock poisoned".into()))
    }

    fn create_with_after_publish_hook<F>(
        &self,
        new_record: NewInvocationRecord,
        after_publish: F,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(TaskId),
    {
        self.create_with_after_publish_hook_before(
            new_record,
            after_publish,
            ProviderDeadline::from_budget(STORE_OPERATION_BUDGET),
            &CancellationToken::new(),
            false,
        )
    }

    fn create_with_after_publish_hook_before<F>(
        &self,
        new_record: NewInvocationRecord,
        after_publish: F,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
        working: bool,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(TaskId),
    {
        let mut writer = self.lock_writer_before(deadline, cancellation)?;
        self.ensure_create_capacity(&mut writer, deadline, cancellation)?;
        let task_id = new_record.task_id();
        if writer.records.contains_key(&task_id) {
            return Err(InvocationStoreError::TaskIdCollision { task_id });
        }
        let record = if working {
            new_record.into_working_stored(self.clock.now_epoch_millis())
        } else {
            new_record.into_stored(self.clock.now_epoch_millis())
        };
        store_operation_checkpoint(deadline, cancellation)?;
        self.publish_record(
            &mut writer,
            &record,
            CommitOperation::Create,
            deadline,
            cancellation,
            || {},
        )?;
        after_publish(task_id);
        Ok(record)
    }

    fn create_working_with_after_publish_hook<F>(
        &self,
        new_record: NewInvocationRecord,
        after_publish: F,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(TaskId),
    {
        self.create_with_after_publish_hook_before(
            new_record,
            after_publish,
            ProviderDeadline::from_budget(STORE_OPERATION_BUDGET),
            &CancellationToken::new(),
            true,
        )
    }

    fn update_with_before_publish_hook<F>(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
        before_publish: F,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(),
    {
        self.update_with_before_publish_hook_before(
            task_id,
            transition,
            before_publish,
            ProviderDeadline::from_budget(STORE_OPERATION_BUDGET),
            &CancellationToken::new(),
        )
    }

    fn update_with_before_publish_hook_before<F>(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
        before_publish: F,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(),
    {
        let mut writer = self.lock_writer_before(deadline, cancellation)?;
        let record = self.read_record(task_id)?;
        if self.is_expired(&record) {
            return Err(InvocationStoreError::Expired);
        }
        let updated = self.transition_record(record, transition)?;
        store_operation_checkpoint(deadline, cancellation)?;
        self.publish_record(
            &mut writer,
            &updated,
            CommitOperation::Update,
            deadline,
            cancellation,
            before_publish,
        )?;
        Ok(updated)
    }

    fn ensure_create_capacity(
        &self,
        catalog: &mut StoreCatalog,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<(), InvocationStoreError> {
        if catalog.records.len() < self.limits.max_records {
            return Ok(());
        }
        let now = self.clock.now_epoch_millis();
        let expired = catalog
            .records
            .iter()
            .filter_map(|(task_id, retained)| {
                (retained.terminal
                    && now
                        .checked_sub(retained.updated_at_epoch_ms)
                        .is_some_and(|elapsed| elapsed >= retained.ttl_ms))
                .then_some(*task_id)
            })
            .collect::<Vec<_>>();
        for task_id in expired {
            store_operation_checkpoint(deadline, cancellation)?;
            self.remove_record_file(task_id)?;
            catalog.records.remove(&task_id);
        }
        if catalog.records.len() >= self.limits.max_records {
            return Err(InvocationStoreError::Capacity {
                max_records: self.limits.max_records,
            });
        }
        Ok(())
    }

    fn transition_record(
        &self,
        mut record: StoredInvocationRecord,
        transition: TaskTransition,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let attempted = match &transition {
            TaskTransition::StartWorking { .. } => "start_working",
            TaskTransition::Complete { .. } => "complete",
            TaskTransition::Fail { .. } => "fail",
        };
        match transition {
            TaskTransition::StartWorking { status_message }
                if record.status == InvocationStatus::Queued =>
            {
                record.status = InvocationStatus::Working;
                record.status_message = status_message;
                record.failure_reason = None;
            }
            TaskTransition::Complete {
                status_message,
                result,
            } if record.status == InvocationStatus::Working => {
                record.status = InvocationStatus::Completed;
                record.status_message = status_message;
                record.result = Some(*result);
                record.failure_reason = None;
                record.resume = None;
            }
            TaskTransition::Fail {
                status_message,
                reason,
            } if record.status == InvocationStatus::Working => {
                record.status = InvocationStatus::Failed;
                record.status_message = status_message;
                record.result = None;
                record.failure_reason = Some(reason);
                record.resume = None;
            }
            _ => {
                return Err(InvocationStoreError::InvalidTransition {
                    from: record.status,
                    attempted,
                })
            }
        }
        record.updated_at_epoch_ms = record
            .updated_at_epoch_ms
            .max(self.clock.now_epoch_millis());
        Ok(record)
    }

    fn is_expired(&self, record: &StoredInvocationRecord) -> bool {
        if !record.is_terminal() {
            return false;
        }
        self.clock
            .now_epoch_millis()
            .checked_sub(record.updated_at_epoch_ms)
            .is_some_and(|elapsed| elapsed >= record.ttl_ms)
    }
}

impl InvocationStore for FileInvocationStore {
    fn create(
        &self,
        new_record: NewInvocationRecord,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.create_with_after_publish_hook(new_record, |_| {})
    }

    fn get(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let record = self.read_record(task_id)?;
        if self.is_expired(&record) {
            Err(InvocationStoreError::Expired)
        } else {
            Ok(record)
        }
    }

    fn create_working(
        &self,
        new_record: NewInvocationRecord,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.create_working_with_after_publish_hook(new_record, |_| {})
    }

    fn update(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.update_with_before_publish_hook(task_id, transition, || {})
    }

    fn cancel(
        &self,
        task_id: TaskId,
        status_message: SafeStatusMessage,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.cancel_before(
            task_id,
            status_message,
            ProviderDeadline::from_budget(STORE_OPERATION_BUDGET),
            &CancellationToken::new(),
        )
    }

    fn create_working_before(
        &self,
        new_record: NewInvocationRecord,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.create_with_after_publish_hook_before(new_record, |_| {}, deadline, cancellation, true)
    }

    fn get_before(
        &self,
        task_id: TaskId,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        store_operation_checkpoint(deadline, cancellation)?;
        let record = self.read_record(task_id)?;
        store_operation_checkpoint(deadline, cancellation)?;
        if self.is_expired(&record) {
            Err(InvocationStoreError::Expired)
        } else {
            Ok(record)
        }
    }

    fn update_before(
        &self,
        task_id: TaskId,
        transition: TaskTransition,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.update_with_before_publish_hook_before(
            task_id,
            transition,
            || {},
            deadline,
            cancellation,
        )
    }

    fn cancel_before(
        &self,
        task_id: TaskId,
        status_message: SafeStatusMessage,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let mut writer = self.lock_writer_before(deadline, cancellation)?;
        let mut record = self.read_record(task_id)?;
        if self.is_expired(&record) {
            return Err(InvocationStoreError::Expired);
        }
        if record.is_terminal() {
            return Ok(record);
        }
        record.status = InvocationStatus::Cancelled;
        record.status_message = status_message;
        record.updated_at_epoch_ms = record
            .updated_at_epoch_ms
            .max(self.clock.now_epoch_millis());
        record.result = None;
        record.failure_reason = None;
        record.resume = None;
        store_operation_checkpoint(deadline, cancellation)?;
        self.publish_record(
            &mut writer,
            &record,
            CommitOperation::Cancel,
            deadline,
            cancellation,
            || {},
        )?;
        Ok(record)
    }
}

fn acquire_root_lock(root: &File) -> Result<File, InvocationStoreError> {
    let lock_name = OsStr::new(STORE_LOCK_FILE);
    let lock = open_directory_ownership_lock(root, lock_name)
        .map_err(|error| storage_error("open stable task store ownership lock", error))?;
    match FileExt::try_lock_exclusive(&lock) {
        Ok(()) => Ok(lock),
        Err(error) if lock_is_contended(&error) => Err(InvocationStoreError::AlreadyOwned),
        Err(error) => Err(storage_error("acquire task store lock", error)),
    }
}

fn lock_is_contended(error: &std::io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == std::io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn validate_record(record: &StoredInvocationRecord) -> Result<(), InvocationStoreError> {
    if record.schema_version != INVOCATION_RECORD_SCHEMA_VERSION {
        return Err(corrupt_error("unsupported task record schema version"));
    }
    if record.updated_at_epoch_ms < record.created_at_epoch_ms {
        return Err(corrupt_error("task record timestamp moved backwards"));
    }
    let shape_is_valid = match record.status {
        InvocationStatus::Queued | InvocationStatus::Working => {
            record.result.is_none() && record.failure_reason.is_none()
        }
        InvocationStatus::Completed => {
            record.result.is_some() && record.failure_reason.is_none() && record.resume.is_none()
        }
        InvocationStatus::Failed => {
            record.result.is_none() && record.failure_reason.is_some() && record.resume.is_none()
        }
        InvocationStatus::Cancelled => {
            record.result.is_none() && record.failure_reason.is_none() && record.resume.is_none()
        }
    };
    if !shape_is_valid {
        return Err(corrupt_error("task record status payload is inconsistent"));
    }
    if let Some(result) = record.result.as_ref() {
        match canonical_result_size(result) {
            Ok(_) => {}
            Err(CanonicalResultSizeError::TooLarge) => {
                return Err(InvocationStoreError::ResultTooLarge {
                    max_bytes: MAX_CANONICAL_RESULT_BYTES,
                })
            }
            Err(CanonicalResultSizeError::Checkpoint(never)) => match never {},
            Err(CanonicalResultSizeError::Serialization) => {
                return Err(corrupt_error(
                    "canonical task result could not be serialized",
                ))
            }
        }
    }
    let status_message_is_valid = match record.status {
        InvocationStatus::Queued => record.status_message == SafeStatusMessage::Queued,
        InvocationStatus::Working => matches!(
            record.status_message,
            SafeStatusMessage::Working | SafeStatusMessage::Delivering
        ),
        InvocationStatus::Completed => record.status_message == SafeStatusMessage::Completed,
        InvocationStatus::Failed => matches!(
            record.status_message,
            SafeStatusMessage::Failed | SafeStatusMessage::Interrupted
        ),
        InvocationStatus::Cancelled => record.status_message == SafeStatusMessage::Cancelled,
    };
    if !status_message_is_valid {
        return Err(corrupt_error(
            "task record status message is inconsistent with lifecycle state",
        ));
    }
    Ok(())
}

struct BoundedRecordWriter<'a> {
    file: &'a mut File,
    bytes: usize,
    max_bytes: usize,
    deadline: ProviderDeadline,
    cancellation: &'a CancellationToken,
    failure: Option<InvocationStoreError>,
}

impl Write for BoundedRecordWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if let Err(error) = store_operation_checkpoint(self.deadline, self.cancellation) {
            self.failure = Some(error);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "task record serialization deadline elapsed",
            ));
        }
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.failure = Some(InvocationStoreError::RecordTooLarge {
                max_bytes: self.max_bytes,
            });
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "task record exceeds byte limit",
            ));
        };
        if next > self.max_bytes {
            self.failure = Some(InvocationStoreError::RecordTooLarge {
                max_bytes: self.max_bytes,
            });
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "task record exceeds byte limit",
            ));
        }
        match self.file.write(buffer) {
            Ok(written) => {
                self.bytes += written;
                Ok(written)
            }
            Err(error) => Err(error),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn serialize_record_bounded(
    file: &mut File,
    record: &StoredInvocationRecord,
    max_bytes: usize,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(), InvocationStoreError> {
    let mut writer = BoundedRecordWriter {
        file,
        bytes: 0,
        max_bytes,
        deadline,
        cancellation,
        failure: None,
    };
    let serialized = serde_json::to_writer(&mut writer, record);
    if let Some(error) = writer.failure.take() {
        return Err(error);
    }
    if serialized.is_err() {
        return Err(InvocationStoreError::Storage(
            "serialize task staging record".to_string(),
        ));
    }
    store_operation_checkpoint(deadline, cancellation)
}

fn corrupt_error(message: &'static str) -> InvocationStoreError {
    InvocationStoreError::Corrupt(message.to_string())
}

fn store_checkpoint_io_error(error: InvocationStoreError) -> std::io::Error {
    let kind = match error {
        InvocationStoreError::DeadlineExceeded => std::io::ErrorKind::TimedOut,
        InvocationStoreError::Cancelled => std::io::ErrorKind::Interrupted,
        _ => std::io::ErrorKind::Other,
    };
    std::io::Error::new(kind, "task store recovery checkpoint rejected")
}

fn storage_error(operation: &'static str, error: std::io::Error) -> InvocationStoreError {
    InvocationStoreError::Storage(format!("{operation}: {error}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        validate_record, FileInvocationStore, PublicationFailure, RecoveryClassification,
        STORE_LOCK_FILE,
    };
    use crate::application::invocation_store::{
        CommitOperation, EpochMillisClock, InvocationStore, InvocationStoreError,
        NewInvocationRecord, SafeFailureReason, SafeStatusMessage, TaskTransition, ToolIdentity,
        MAX_CANONICAL_RESULT_BYTES,
    };
    use crate::domain::invocation::{
        DeliveryResume, DomainResult, InvocationId, InvocationStatus, NormalizedArgumentsHash,
        ResumeDescriptor, SafeIdentityHash,
    };
    use crate::domain::{cancellation::CancellationToken, code_intelligence::ProviderDeadline};
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{mpsc, Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct ManualEpochClock(AtomicU64);

    impl ManualEpochClock {
        fn at(now: u64) -> Self {
            Self(AtomicU64::new(now))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl EpochMillisClock for ManualEpochClock {
        fn now_epoch_millis(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    static SERIALIZATION_DEADLINE_NOW: OnceLock<Mutex<Instant>> = OnceLock::new();

    fn advancing_serialization_now() -> Instant {
        let clock = SERIALIZATION_DEADLINE_NOW.get_or_init(|| Mutex::new(Instant::now()));
        let mut now = clock.lock().unwrap();
        *now += Duration::from_millis(1);
        *now
    }

    fn new_record(ttl_ms: u64, resume: Option<ResumeDescriptor>) -> NewInvocationRecord {
        NewInvocationRecord::new(
            InvocationId::new(),
            ToolIdentity::View,
            NormalizedArgumentsHash::from_sha256([0x22; 32]),
            SafeIdentityHash::from_sha256([0x33; 32]),
            SafeStatusMessage::Queued,
            250,
            ttl_ms,
            resume,
        )
    }

    fn open_store(
        root: &Path,
        clock: Arc<ManualEpochClock>,
    ) -> (FileInvocationStore, super::RecoveryReport) {
        FileInvocationStore::open(root, clock).expect("open task store")
    }

    #[test]
    fn create_returns_only_after_the_record_is_durable_and_readable() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_000));
        let (store, _) = open_store(root.path(), clock);
        let new = new_record(10_000, None);

        let created = store
            .create_with_after_publish_hook(new, |task_id| {
                let committed = FileInvocationStore::read_committed(root.path(), task_id).unwrap();
                assert_eq!(committed.task_id, task_id);
                assert_eq!(committed.status, InvocationStatus::Queued);
            })
            .unwrap();

        assert_eq!(store.get(created.task_id).unwrap(), created);
    }

    #[test]
    fn failed_rename_preserves_previous_commit_until_recovery_terminalizes_it() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_500));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        store.inject_next_publication_failure(PublicationFailure::BeforeRename);

        let error = store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap_err();
        assert!(matches!(error, InvocationStoreError::Storage(_)));
        assert_eq!(
            FileInvocationStore::read_committed(root.path(), created.task_id).unwrap(),
            created
        );
        drop(store);

        let (reopened, _) = open_store(root.path(), clock);
        let recovered = reopened.get(created.task_id).unwrap();
        assert_eq!(recovered.task_id, created.task_id);
        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::Interrupted)
        );
    }

    #[test]
    fn create_sync_failure_reports_commit_uncertain_with_visible_task_identity() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_600));
        let (store, _) = open_store(root.path(), clock.clone());
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);

        let error = store.create(new_record(10_000, None)).unwrap_err();
        let task_id = match error {
            InvocationStoreError::CommitUncertain {
                task_id,
                operation: CommitOperation::Create,
            } => task_id,
            other => panic!("unexpected create outcome: {other:?}"),
        };
        drop(store);

        let (reopened, _) = open_store(root.path(), clock);
        let visible = reopened.get(task_id).unwrap();
        assert_eq!(visible.task_id, task_id);
        assert_eq!(visible.status, InvocationStatus::Failed);
        assert_eq!(visible.failure_reason, Some(SafeFailureReason::Interrupted));
    }

    #[test]
    fn atomic_working_create_has_distinct_before_and_after_commit_faults() {
        let before_root = tempfile::tempdir().unwrap();
        let (before, _) = open_store(before_root.path(), Arc::new(ManualEpochClock::at(1_625)));
        before.inject_next_publication_failure(PublicationFailure::BeforeRename);
        assert!(matches!(
            before.create_working(new_record(10_000, None)),
            Err(InvocationStoreError::Storage(_))
        ));
        assert_eq!(
            fs::read_dir(before_root.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .count(),
            0
        );

        let after_root = tempfile::tempdir().unwrap();
        let (after, _) = open_store(after_root.path(), Arc::new(ManualEpochClock::at(1_650)));
        after.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
        let task_id = match after.create_working(new_record(10_000, None)).unwrap_err() {
            InvocationStoreError::CommitUncertain {
                task_id,
                operation: CommitOperation::Create,
            } => task_id,
            other => panic!("unexpected create outcome: {other:?}"),
        };
        assert_eq!(
            after.get(task_id).unwrap().status,
            InvocationStatus::Working
        );
    }

    #[test]
    fn uncertain_visible_create_counts_toward_capacity_before_and_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_660));
        let (store, _) = FileInvocationStore::open_with_limits_for_test(
            root.path(),
            clock.clone(),
            1,
            1_048_576,
        )
        .unwrap();
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
        let task_id = match store.create_working(new_record(10_000, None)).unwrap_err() {
            InvocationStoreError::CommitUncertain {
                task_id,
                operation: CommitOperation::Create,
            } => task_id,
            other => panic!("unexpected create outcome: {other:?}"),
        };
        assert_eq!(
            store.get(task_id).unwrap().status,
            InvocationStatus::Working
        );
        assert_eq!(
            store.create_working(new_record(10_000, None)).unwrap_err(),
            InvocationStoreError::Capacity { max_records: 1 },
            "a visible uncertain create must consume the in-process retention slot"
        );
        drop(store);

        let (reopened, _) =
            FileInvocationStore::open_with_limits_for_test(root.path(), clock, 1, 1_048_576)
                .unwrap();
        assert_eq!(
            reopened.get(task_id).unwrap().status,
            InvocationStatus::Failed
        );
        assert_eq!(
            reopened
                .create_working(new_record(10_000, None))
                .unwrap_err(),
            InvocationStoreError::Capacity { max_records: 1 }
        );
    }

    #[test]
    fn pre_rename_create_failure_does_not_consume_retention_capacity() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_665));
        let (store, _) =
            FileInvocationStore::open_with_limits_for_test(root.path(), clock, 1, 1_048_576)
                .unwrap();
        store.inject_next_publication_failure(PublicationFailure::BeforeRename);
        assert!(matches!(
            store.create_working(new_record(10_000, None)),
            Err(InvocationStoreError::Storage(_))
        ));

        let committed = store.create_working(new_record(10_000, None)).unwrap();
        assert_eq!(store.get(committed.task_id).unwrap(), committed);
    }

    #[test]
    fn uncertain_visible_terminal_update_and_cancel_refresh_retention_state() {
        for cancel in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let clock = Arc::new(ManualEpochClock::at(1_670));
            let (store, _) = FileInvocationStore::open_with_limits_for_test(
                root.path(),
                clock.clone(),
                1,
                1_048_576,
            )
            .unwrap();
            let working = store.create_working(new_record(1, None)).unwrap();
            store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
            let error = if cancel {
                store
                    .cancel(working.task_id, SafeStatusMessage::Cancelled)
                    .unwrap_err()
            } else {
                store
                    .update(
                        working.task_id,
                        TaskTransition::Complete {
                            status_message: SafeStatusMessage::Completed,
                            result: Box::new(DomainResult::success("terminal")),
                        },
                    )
                    .unwrap_err()
            };
            assert!(matches!(
                error,
                InvocationStoreError::CommitUncertain { .. }
            ));
            assert!(store.get(working.task_id).unwrap().is_terminal());

            clock.set(1_672);
            let replacement = store.create_working(new_record(10_000, None)).unwrap();
            assert_eq!(store.get(replacement.task_id).unwrap(), replacement);
            assert!(matches!(
                store.get(working.task_id),
                Err(InvocationStoreError::NotFound)
            ));
        }
    }

    #[test]
    fn complete_fail_and_cancel_have_exact_before_and_after_commit_fault_states() {
        for (transition, expected_status, expected_reason) in [
            (
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(DomainResult::success("committed completion")),
                },
                InvocationStatus::Completed,
                None,
            ),
            (
                TaskTransition::Fail {
                    status_message: SafeStatusMessage::Failed,
                    reason: SafeFailureReason::InvocationFailed,
                },
                InvocationStatus::Failed,
                Some(SafeFailureReason::InvocationFailed),
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let (store, _) = open_store(root.path(), Arc::new(ManualEpochClock::at(1_675)));
            let working = store.create_working(new_record(10_000, None)).unwrap();
            store.inject_next_publication_failure(PublicationFailure::BeforeRename);
            assert!(matches!(
                store.update(working.task_id, transition.clone()),
                Err(InvocationStoreError::Storage(_))
            ));
            assert_eq!(
                store.get(working.task_id).unwrap().status,
                InvocationStatus::Working
            );

            store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
            assert_eq!(
                store.update(working.task_id, transition).unwrap_err(),
                InvocationStoreError::CommitUncertain {
                    task_id: working.task_id,
                    operation: CommitOperation::Update,
                }
            );
            let committed = store.get(working.task_id).unwrap();
            assert_eq!(committed.status, expected_status);
            assert_eq!(committed.failure_reason, expected_reason);
        }

        let root = tempfile::tempdir().unwrap();
        let (store, _) = open_store(root.path(), Arc::new(ManualEpochClock::at(1_690)));
        let working = store.create_working(new_record(10_000, None)).unwrap();
        store.inject_next_publication_failure(PublicationFailure::BeforeRename);
        assert!(matches!(
            store.cancel(working.task_id, SafeStatusMessage::Cancelled),
            Err(InvocationStoreError::Storage(_))
        ));
        assert_eq!(
            store.get(working.task_id).unwrap().status,
            InvocationStatus::Working
        );
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);
        assert_eq!(
            store
                .cancel(working.task_id, SafeStatusMessage::Cancelled)
                .unwrap_err(),
            InvocationStoreError::CommitUncertain {
                task_id: working.task_id,
                operation: CommitOperation::Cancel,
            }
        );
        assert_eq!(
            store.get(working.task_id).unwrap().status,
            InvocationStatus::Cancelled
        );
    }

    #[test]
    fn update_sync_failure_reports_commit_uncertain_and_reopens_new_state() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_700));
        let (store, _) = open_store(root.path(), clock.clone());
        let resume = ResumeDescriptor::Delivery(DeliveryResume::new(
            SafeIdentityHash::from_sha256([0x77; 32]),
        ));
        let created = store
            .create(new_record(10_000, Some(resume.clone())))
            .unwrap();
        clock.set(1_750);
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);

        assert_eq!(
            store
                .update(
                    created.task_id,
                    TaskTransition::StartWorking {
                        status_message: SafeStatusMessage::Working,
                    },
                )
                .unwrap_err(),
            InvocationStoreError::CommitUncertain {
                task_id: created.task_id,
                operation: CommitOperation::Update,
            }
        );
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        let visible = reopened.get(created.task_id).unwrap();
        assert_eq!(visible.task_id, created.task_id);
        assert_eq!(visible.status, InvocationStatus::Failed);
        assert_eq!(
            visible.failure_reason,
            Some(SafeFailureReason::ResumeUnsupported)
        );
        assert!(visible.updated_at_epoch_ms >= 1_750);
        assert!(visible.resume.is_none());
        assert!(report
            .classifications
            .contains(&RecoveryClassification::UnsupportedResume {
                task_id: created.task_id,
            }));
    }

    #[test]
    fn exclusive_root_lock_prevents_recovery_until_the_owner_drops() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_800));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();

        let second_open = FileInvocationStore::open(root.path(), clock.clone());
        assert!(matches!(
            second_open,
            Err(InvocationStoreError::AlreadyOwned)
        ));
        assert_eq!(
            store.get(created.task_id).unwrap().status,
            InvocationStatus::Working,
            "a rejected second owner must not run recovery"
        );
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        assert_eq!(
            reopened.get(created.task_id).unwrap().status,
            InvocationStatus::Failed
        );
        assert!(report.classifications.contains(
            &RecoveryClassification::InterruptedNonResumable {
                task_id: created.task_id,
            }
        ));
    }

    #[test]
    fn replacing_the_conventional_lock_name_cannot_create_a_second_unix_owner() {
        use crate::infrastructure::platform::testing::can_rename_parent_with_retained_cleanup_child_for_test;

        if !can_rename_parent_with_retained_cleanup_child_for_test() {
            return;
        }

        let root = tempfile::tempdir().unwrap();
        let lock_path = root.path().join(STORE_LOCK_FILE);
        fs::write(&lock_path, b"initial lock identity").unwrap();
        let clock = Arc::new(ManualEpochClock::at(1_900));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();

        fs::rename(&lock_path, root.path().join("displaced-lock")).unwrap();
        fs::write(&lock_path, b"replacement lock identity").unwrap();
        let second_open = FileInvocationStore::open(root.path(), clock);

        assert_eq!(
            store.get(created.task_id).unwrap().status,
            InvocationStatus::Working,
            "a rejected second owner must not run recovery through a replacement lock name"
        );
        assert!(matches!(
            second_open,
            Err(InvocationStoreError::AlreadyOwned)
        ));
    }

    #[test]
    fn status_update_never_exposes_a_partially_written_record() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(2_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        clock.set(2_100);

        let working = store
            .update_with_before_publish_hook(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
                || {
                    let visible =
                        FileInvocationStore::read_committed(root.path(), created.task_id).unwrap();
                    assert_eq!(visible.status, InvocationStatus::Queued);
                    assert_eq!(visible.updated_at_epoch_ms, 2_000);
                },
            )
            .unwrap();

        assert_eq!(working.status, InvocationStatus::Working);
        assert_eq!(working.updated_at_epoch_ms, 2_100);
        assert_eq!(store.get(created.task_id).unwrap(), working);
    }

    #[test]
    fn terminal_result_is_retained_without_changing_its_domain_shape() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(3_000));
        let (store, _) = open_store(root.path(), clock);
        let created = store.create(new_record(10_000, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();
        let result = DomainResult::success("complete");
        store
            .update(
                created.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(result.clone()),
                },
            )
            .unwrap();

        let retained = store.get(created.task_id).unwrap();
        assert_eq!(retained.status, InvocationStatus::Completed);
        assert_eq!(retained.result, Some(result));
        assert!(retained.resume.is_none());
    }

    #[test]
    fn terminal_record_expires_only_after_its_ttl_despite_wall_clock_rollback() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(4_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(100, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();
        let terminal = store
            .update(
                created.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(DomainResult::success("complete")),
                },
            )
            .unwrap();

        clock.set(3_900);
        assert_eq!(store.get(created.task_id).unwrap(), terminal);
        clock.set(4_099);
        assert_eq!(store.get(created.task_id).unwrap(), terminal);
        clock.set(4_100);
        assert_eq!(
            store.get(created.task_id).unwrap_err(),
            InvocationStoreError::Expired
        );
    }

    #[test]
    fn nonterminal_record_never_expires() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(5_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(1, None)).unwrap();
        let working = store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();

        clock.set(u64::MAX);
        assert_eq!(store.get(created.task_id).unwrap(), working);
    }

    #[test]
    fn cancellation_is_idempotent() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(6_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();

        let first = store
            .cancel(created.task_id, SafeStatusMessage::Cancelled)
            .unwrap();
        clock.set(6_500);
        let second = store
            .cancel(created.task_id, SafeStatusMessage::Cancelled)
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(second.status, InvocationStatus::Cancelled);
        assert_eq!(second.updated_at_epoch_ms, 6_000);
    }

    #[test]
    fn truncated_temporary_record_is_ignored_and_classified_on_recovery() {
        let root = tempfile::tempdir().unwrap();
        let temporary_name = ".task-record.crash.tmp";
        fs::write(root.path().join(temporary_name), b"{\"schemaVersion\":").unwrap();

        let (_, report) = open_store(root.path(), Arc::new(ManualEpochClock::at(7_000)));

        assert_eq!(
            report.classifications,
            vec![RecoveryClassification::DiscardedTemporary {
                file_name: temporary_name.to_string(),
            }]
        );
        assert!(!root.path().join(temporary_name).exists());
    }

    #[test]
    fn queued_record_without_a_live_owner_recovers_as_interrupted() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(8_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let queued = store.create(new_record(10_000, None)).unwrap();
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(queued.task_id).unwrap();

        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(recovered.status_message, SafeStatusMessage::Interrupted);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::Interrupted)
        );
        assert!(recovered.result.is_none());
        assert!(recovered.resume.is_none());
        assert!(report.classifications.contains(
            &RecoveryClassification::InterruptedNonResumable {
                task_id: queued.task_id,
            }
        ));
    }

    #[test]
    fn v1_nonresumable_working_record_recovers_as_interrupted_failure() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(9_000));
        let mut legacy = new_record(10_000, None).into_stored(9_000);
        legacy.status = InvocationStatus::Working;
        legacy.status_message = SafeStatusMessage::Working;
        write_legacy_v1_record(root.path(), &legacy);
        clock.set(9_100);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(legacy.task_id).unwrap();

        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(recovered.status_message, SafeStatusMessage::Interrupted);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::Interrupted)
        );
        assert!(report.classifications.contains(
            &RecoveryClassification::InterruptedNonResumable {
                task_id: legacy.task_id,
            }
        ));
    }

    #[test]
    fn v2_working_without_a_live_owner_recovers_as_interrupted() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(9_500));
        let (store, _) = open_store(root.path(), clock.clone());
        let working = store.create_working(new_record(10_000, None)).unwrap();
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(working.task_id).unwrap();
        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::Interrupted)
        );
        assert!(report.classifications.contains(
            &RecoveryClassification::InterruptedNonResumable {
                task_id: working.task_id,
            }
        ));
    }

    #[test]
    fn v2_working_resume_descriptor_without_registered_owner_is_unsupported() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(9_750));
        let resume = ResumeDescriptor::Delivery(DeliveryResume::new(
            SafeIdentityHash::from_sha256([0x48; 32]),
        ));
        let (store, _) = open_store(root.path(), clock.clone());
        let working = store
            .create_working(new_record(10_000, Some(resume)))
            .unwrap();
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(working.task_id).unwrap();
        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::ResumeUnsupported)
        );
        assert!(recovered.resume.is_none());
        assert!(report
            .classifications
            .contains(&RecoveryClassification::UnsupportedResume {
                task_id: working.task_id,
            }));
    }

    #[test]
    fn v1_resumable_working_without_a_registered_owner_recovers_as_unsupported() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(10_000));
        let resume = ResumeDescriptor::Delivery(DeliveryResume::new(
            SafeIdentityHash::from_sha256([0x44; 32]),
        ));
        let mut legacy = new_record(10_000, Some(resume)).into_stored(10_000);
        legacy.status = InvocationStatus::Working;
        legacy.status_message = SafeStatusMessage::Delivering;
        write_legacy_v1_record(root.path(), &legacy);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(legacy.task_id).unwrap();

        assert_eq!(recovered.task_id, legacy.task_id);
        assert_eq!(recovered.schema_version, 2);
        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(
            recovered.failure_reason,
            Some(SafeFailureReason::ResumeUnsupported)
        );
        assert!(recovered.resume.is_none());
        assert!(report
            .classifications
            .contains(&RecoveryClassification::UnsupportedResume {
                task_id: legacy.task_id,
            }));
    }

    #[test]
    fn v1_terminal_record_migrates_to_v2_without_changing_its_domain_result() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(10_500));
        let expected = DomainResult::success("legacy terminal");
        let mut legacy = new_record(10_000, None).into_stored(10_000);
        legacy.status = InvocationStatus::Completed;
        legacy.status_message = SafeStatusMessage::Completed;
        legacy.result = Some(expected.clone());
        write_legacy_v1_record(root.path(), &legacy);

        let (store, report) = open_store(root.path(), clock);
        let migrated = store.get(legacy.task_id).unwrap();

        assert_eq!(migrated.schema_version, 2);
        assert_eq!(migrated.status, InvocationStatus::Completed);
        assert_eq!(migrated.result, Some(expected));
        assert!(report
            .classifications
            .contains(&RecoveryClassification::MigratedV1Record {
                task_id: legacy.task_id,
            }));
        let persisted: Value = serde_json::from_slice(
            &fs::read(root.path().join(format!("{}.json", legacy.task_id))).unwrap(),
        )
        .unwrap();
        assert_eq!(persisted["schemaVersion"], 2);
    }

    #[test]
    fn v1_failed_and_cancelled_terminal_records_migrate_deterministically() {
        for (status, message, expected_reason) in [
            (
                InvocationStatus::Failed,
                SafeStatusMessage::Failed,
                Some(SafeFailureReason::InvocationFailed),
            ),
            (
                InvocationStatus::Cancelled,
                SafeStatusMessage::Cancelled,
                None,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let clock = Arc::new(ManualEpochClock::at(10_625));
            let mut legacy = new_record(10_000, None).into_stored(10_500);
            legacy.status = status;
            legacy.status_message = message;
            write_legacy_v1_record(root.path(), &legacy);

            let (store, report) = open_store(root.path(), clock);
            let migrated = store.get(legacy.task_id).unwrap();
            assert_eq!(migrated.schema_version, 2);
            assert_eq!(migrated.status, status);
            assert_eq!(migrated.failure_reason, expected_reason);
            assert!(report
                .classifications
                .contains(&RecoveryClassification::MigratedV1Record {
                    task_id: legacy.task_id,
                }));
        }
    }

    #[test]
    fn v2_failed_record_persists_only_a_closed_failure_reason() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(10_750));
        let (store, _) = open_store(root.path(), clock);
        let working = store.create_working(new_record(10_000, None)).unwrap();
        let failed = store
            .update(
                working.task_id,
                TaskTransition::Fail {
                    status_message: SafeStatusMessage::Failed,
                    reason: SafeFailureReason::PersistenceFailed,
                },
            )
            .unwrap();
        assert_eq!(failed.schema_version, 2);
        assert_eq!(
            failed.failure_reason,
            Some(SafeFailureReason::PersistenceFailed)
        );

        let bytes = fs::read(root.path().join(format!("{}.json", failed.task_id))).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("persistenceFailed"));
        for forbidden in ["permission denied", "/private/store", "runtime stderr"] {
            assert!(!text.contains(forbidden));
        }
    }

    #[test]
    fn unknown_record_schema_fails_closed_instead_of_reinterpreting_bytes() {
        let root = tempfile::tempdir().unwrap();
        let record = new_record(10_000, None).into_working_stored(10_000);
        let mut value = serde_json::to_value(&record).unwrap();
        value["schemaVersion"] = json!(99);
        fs::write(
            root.path().join(format!("{}.json", record.task_id)),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let error = FileInvocationStore::open(root.path(), Arc::new(ManualEpochClock::at(11_000)))
            .err()
            .expect("unknown schema must reject store admission");
        assert!(matches!(error, InvocationStoreError::Corrupt(_)));
    }

    fn write_legacy_v1_record(
        root: &Path,
        record: &crate::application::invocation_store::StoredInvocationRecord,
    ) {
        let mut value = serde_json::to_value(record).unwrap();
        value["schemaVersion"] = json!(1);
        value.as_object_mut().unwrap().remove("failureReason");
        fs::write(
            root.join(format!("{}.json", record.task_id)),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn raw_arguments_and_secret_sentinels_never_enter_any_store_bytes() {
        const SECRET: &str = "TASK_STORE_SECRET_SENTINEL_7b824f";
        const CREDENTIAL_URL: &str = "https://user:password@example.invalid/private";
        const USER_PATH: &str = "/Users/sentinel/Customer Name/source.cf";
        const RAW_COMMAND: &str = "runner --password TASK_STORE_SECRET_SENTINEL_7b824f";
        let raw_arguments = json!({
            "password": SECRET,
            "credentialUrl": CREDENTIAL_URL,
            "cwd": USER_PATH,
            "command": RAW_COMMAND,
        });
        let digest: [u8; 32] = Sha256::digest(serde_json::to_vec(&raw_arguments).unwrap()).into();
        let expected_hash = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(11_000));
        let (store, _) = open_store(root.path(), clock);
        let record = NewInvocationRecord::new(
            InvocationId::new(),
            ToolIdentity::Run,
            NormalizedArgumentsHash::from_sha256(digest),
            SafeIdentityHash::from_sha256([0x55; 32]),
            SafeStatusMessage::Queued,
            250,
            10_000,
            None,
        );
        store.create(record).unwrap();
        drop(store);

        let mut all_bytes = Vec::new();
        collect_recursive_bytes(root.path(), &mut all_bytes);
        let text = String::from_utf8_lossy(&all_bytes);
        for forbidden in [SECRET, CREDENTIAL_URL, USER_PATH, RAW_COMMAND] {
            assert!(!text.contains(forbidden), "persisted sentinel {forbidden}");
        }
        assert!(
            text.contains(&expected_hash),
            "normalized hash was not persisted"
        );

        let json_file = fs::read_dir(root.path())
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .unwrap();
        let value: Value = serde_json::from_slice(&fs::read(json_file.path()).unwrap()).unwrap();
        assert_eq!(value["tool"], "unica.run");
        assert_eq!(value["statusMessage"], "queued");
        assert_eq!(
            value
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "schemaVersion",
                "taskId",
                "invocationId",
                "tool",
                "normalizedArgumentsHash",
                "workspaceIdentityHash",
                "createdAtEpochMs",
                "updatedAtEpochMs",
                "status",
                "statusMessage",
                "pollIntervalMs",
                "ttlMs",
            ]
        );
    }

    #[test]
    fn record_validation_rejects_status_codes_inconsistent_with_lifecycle_state() {
        let record = new_record(10_000, None).into_stored(12_000);
        let mut inconsistent = record;
        inconsistent.status_message = SafeStatusMessage::Completed;

        assert!(matches!(
            validate_record(&inconsistent),
            Err(InvocationStoreError::Corrupt(_))
        ));
    }

    #[test]
    fn retained_root_handle_prevents_symlink_swap_from_redirecting_updates() {
        use crate::infrastructure::platform::testing::{
            attempt_retained_directory_replacement_for_test,
            create_directory_link_fixture_for_test, path_identity_for_test, FileLinkFixtureOutcome,
            RetainedDirectoryReplacementOutcome,
        };

        let parent = tempfile::tempdir().unwrap();
        let root_path = parent.path().join("store");
        let displaced_path = parent.path().join("retained-store");
        let attacker_path = parent.path().join("attacker");
        fs::create_dir(&root_path).unwrap();
        fs::create_dir(&attacker_path).unwrap();
        let clock = Arc::new(ManualEpochClock::at(13_000));
        let (store, _) = open_store(&root_path, clock);
        let created = store.create(new_record(10_000, None)).unwrap();
        let retained_identity = path_identity_for_test(&root_path)
            .unwrap()
            .expect("store root identity must be available on supported CI platforms");
        let replacement =
            attempt_retained_directory_replacement_for_test(&root_path, &displaced_path).unwrap();
        match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => {
                let link_outcome =
                    create_directory_link_fixture_for_test(&attacker_path, &root_path).unwrap();
                if link_outcome != FileLinkFixtureOutcome::Created {
                    return;
                }
            }
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
                assert_eq!(
                    path_identity_for_test(&root_path).unwrap().as_deref(),
                    Some(retained_identity.as_str())
                );
                assert!(!displaced_path.exists());
            }
        }

        let updated = store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();

        assert_eq!(store.get(created.task_id).unwrap(), updated);
        let authoritative_root = match replacement {
            RetainedDirectoryReplacementOutcome::Replaced => &displaced_path,
            RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => &root_path,
        };
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(authoritative_root.join(format!("{}.json", created.task_id))).unwrap()
            )
            .unwrap()["status"],
            "working"
        );
        assert!(fs::read_dir(&attacker_path).unwrap().next().is_none());
    }

    #[test]
    fn preallocated_create_collision_is_typed_and_never_replaces_the_first_record() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_000));
        let (store, _) = open_store(root.path(), clock);
        let intended = new_record(10_000, None);
        let task_id = intended.task_id();
        let first = store.create_working(intended.clone()).unwrap();

        assert_eq!(
            store.create_working(intended).unwrap_err(),
            InvocationStoreError::TaskIdCollision { task_id }
        );
        assert_eq!(store.get(task_id).unwrap(), first);
    }

    #[test]
    fn held_file_writer_is_bounded_by_the_same_deadline_without_releasing_guard() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_100));
        let (store, _) = open_store(root.path(), clock);
        let store = Arc::new(store);
        let held = store.hold_writer_for_test();
        let caller = Arc::clone(&store);
        let (finished, finished_wait) = mpsc::channel();
        std::thread::spawn(move || {
            finished
                .send(caller.create_working_before(
                    new_record(10_000, None),
                    ProviderDeadline::new(Instant::now() + Duration::from_millis(40)),
                    &CancellationToken::new(),
                ))
                .unwrap();
        });

        let result = finished_wait
            .recv_timeout(Duration::from_secs(1))
            .expect("writer acquisition must be bounded while the guard remains held");
        assert_eq!(result.unwrap_err(), InvocationStoreError::DeadlineExceeded);
        drop(held);
    }

    #[test]
    fn expired_terminal_records_are_reclaimed_only_when_bounded_capacity_is_needed() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_200));
        let (store, _) = FileInvocationStore::open_with_limits_for_test(
            root.path(),
            clock.clone(),
            2,
            1_048_576,
        )
        .unwrap();
        let first = store.create_working(new_record(1, None)).unwrap();
        let second = store.create_working(new_record(1, None)).unwrap();
        store
            .cancel(first.task_id, SafeStatusMessage::Cancelled)
            .unwrap();
        store
            .cancel(second.task_id, SafeStatusMessage::Cancelled)
            .unwrap();
        clock.set(14_202);

        let third = store.create_working(new_record(1, None)).unwrap();
        assert!(matches!(
            store.get(first.task_id),
            Err(InvocationStoreError::NotFound)
        ));
        assert!(matches!(
            store.get(second.task_id),
            Err(InvocationStoreError::NotFound)
        ));
        assert_eq!(store.get(third.task_id).unwrap(), third);
    }

    #[test]
    fn recovered_queued_record_releases_capacity_after_its_terminal_ttl() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_225));
        let (store, _) = FileInvocationStore::open_with_limits_for_test(
            root.path(),
            clock.clone(),
            1,
            1_048_576,
        )
        .unwrap();
        let queued = store.create(new_record(1, None)).unwrap();
        drop(store);

        let (reopened, _) = FileInvocationStore::open_with_limits_for_test(
            root.path(),
            clock.clone(),
            1,
            1_048_576,
        )
        .unwrap();
        assert_eq!(
            reopened.get(queued.task_id).unwrap().status,
            InvocationStatus::Failed
        );
        clock.set(14_227);

        let replacement = reopened.create_working(new_record(10_000, None)).unwrap();
        assert_eq!(replacement.status, InvocationStatus::Working);
        assert_eq!(
            reopened.get(queued.task_id).unwrap_err(),
            InvocationStoreError::NotFound
        );
    }

    #[test]
    fn active_and_nonexpired_terminal_records_are_never_evicted_at_capacity() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_250));
        let (store, _) =
            FileInvocationStore::open_with_limits_for_test(root.path(), clock, 2, 1_048_576)
                .unwrap();
        let active = store.create_working(new_record(10_000, None)).unwrap();
        let terminal = store.create_working(new_record(10_000, None)).unwrap();
        let terminal = store
            .cancel(terminal.task_id, SafeStatusMessage::Cancelled)
            .unwrap();

        assert_eq!(
            store.create_working(new_record(10_000, None)).unwrap_err(),
            InvocationStoreError::Capacity { max_records: 2 }
        );
        assert_eq!(store.get(active.task_id).unwrap(), active);
        assert_eq!(store.get(terminal.task_id).unwrap(), terminal);
    }

    #[test]
    fn create_does_not_rescan_a_directory_that_grew_beyond_the_recovery_bound() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_275));
        let (store, _) =
            FileInvocationStore::open_with_limits_for_test(root.path(), clock, 2, 1_048_576)
                .unwrap();
        for index in 0..70 {
            fs::write(
                root.path().join(format!("post-open-noise-{index}")),
                b"noise",
            )
            .unwrap();
        }

        let created = store.create_working(new_record(10_000, None)).unwrap();
        assert_eq!(store.get(created.task_id).unwrap(), created);
    }

    #[test]
    fn recovery_excess_is_typed_capacity_not_unbounded_enumeration_or_corruption() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_300));
        let (store, _) = open_store(root.path(), clock.clone());
        for _ in 0..3 {
            store.create_working(new_record(10_000, None)).unwrap();
        }
        drop(store);

        let error = match FileInvocationStore::open_with_limits_for_test(
            root.path(),
            clock,
            2,
            1_048_576,
        ) {
            Ok(_) => panic!("recovery must reject excess retained records"),
            Err(error) => error,
        };
        assert_eq!(error, InvocationStoreError::Capacity { max_records: 2 });
    }

    #[test]
    fn oversized_valid_record_is_rejected_before_unbounded_recovery_read() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_400));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create_working(new_record(10_000, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(DomainResult::success("X".repeat(4_096))),
                },
            )
            .unwrap();
        drop(store);

        let error = match FileInvocationStore::open_with_limits_for_test(root.path(), clock, 8, 512)
        {
            Ok(_) => panic!("recovery must reject an oversized record"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            InvocationStoreError::RecordTooLarge { max_bytes: 512 }
        );
    }

    #[test]
    fn file_store_enforces_the_canonical_result_limit_before_publication() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_450));
        let (store, _) = open_store(root.path(), clock);
        let working = store.create_working(new_record(10_000, None)).unwrap();

        let error = store
            .update(
                working.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(DomainResult::success(
                        "X".repeat(MAX_CANONICAL_RESULT_BYTES + 1),
                    )),
                },
            )
            .unwrap_err();

        assert_eq!(
            error,
            InvocationStoreError::ResultTooLarge {
                max_bytes: MAX_CANONICAL_RESULT_BYTES,
            }
        );
        assert_eq!(
            store.get(working.task_id).unwrap().status,
            InvocationStatus::Working
        );
    }

    #[test]
    fn record_serialization_uses_the_original_store_deadline_without_reset() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(14_475));
        let (store, _) = open_store(root.path(), clock);
        let working = store.create_working(new_record(10_000, None)).unwrap();
        let started = Instant::now();
        *SERIALIZATION_DEADLINE_NOW
            .get_or_init(|| Mutex::new(started))
            .lock()
            .unwrap() = started;
        let deadline = ProviderDeadline::with_clock(
            started + Duration::from_millis(7),
            advancing_serialization_now,
        );

        let error = store
            .update_before(
                working.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::Completed,
                    result: Box::new(DomainResult::success("Y".repeat(64 * 1024))),
                },
                deadline,
                &CancellationToken::new(),
            )
            .unwrap_err();

        assert_eq!(error, InvocationStoreError::DeadlineExceeded);
        assert_eq!(
            store.get(working.task_id).unwrap().status,
            InvocationStatus::Working
        );
    }

    fn collect_recursive_bytes(path: &Path, output: &mut Vec<u8>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            output.extend_from_slice(entry.file_name().to_string_lossy().as_bytes());
            if file_type.is_dir() {
                collect_recursive_bytes(&entry.path(), output);
            } else if file_type.is_file() {
                output.extend_from_slice(&fs::read(entry.path()).unwrap());
            }
        }
    }
}
