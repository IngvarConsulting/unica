//! Sole-writer durable Task and Invocation store used by the versioned daemon.

use crate::application::invocation_store::{
    EpochMillisClock, InvocationStore, InvocationStoreError, NewInvocationRecord,
    SafeStatusMessage, StoredInvocationRecord, TaskTransition, INVOCATION_RECORD_SCHEMA_VERSION,
};
use crate::domain::invocation::{InvocationStatus, TaskId};
use crate::infrastructure::platform::filesystem::{
    create_new_regular_child, metadata_is_link_or_reparse_point, open_directory_nofollow,
    open_regular_child_nofollow, replace_file_atomically, restrict_stage_to_owner,
    sync_parent_directory,
};
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
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
    ResumableWorking { task_id: TaskId },
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct RecoveryReport {
    pub(crate) classifications: Vec<RecoveryClassification>,
}

/// One daemon owns one instance. The mutex serializes that daemon's writes;
/// readers observe either the previous committed file or its complete replacement.
pub(crate) struct FileInvocationStore {
    root: PathBuf,
    clock: Arc<dyn EpochMillisClock>,
    writer: Mutex<()>,
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
        open_directory_nofollow(&root)
            .map_err(|error| storage_error("open task store root", error))?;

        let store = Self {
            root,
            clock,
            writer: Mutex::new(()),
        };
        let report = store.recover()?;
        Ok((store, report))
    }

    fn recover(&self) -> Result<RecoveryReport, InvocationStoreError> {
        let _writer = self.lock_writer()?;
        let mut entries = fs::read_dir(&self.root)
            .map_err(|error| storage_error("enumerate task store", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| storage_error("read task store entry", error))?;
        entries.sort_by_key(|entry| entry.file_name());
        let mut report = RecoveryReport::default();

        for entry in entries {
            let file_name = entry
                .file_name()
                .into_string()
                .map_err(|_| corrupt_error("task store entry name is not UTF-8"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| storage_error("classify task store entry", error))?;
            if file_name.starts_with('.') && file_name.ends_with(".tmp") {
                if !file_type.is_file() {
                    return Err(corrupt_error("temporary task entry is not a regular file"));
                }
                fs::remove_file(entry.path())
                    .map_err(|error| storage_error("discard temporary task record", error))?;
                sync_parent_directory(&self.root)
                    .map_err(|error| storage_error("sync task store cleanup", error))?;
                report
                    .classifications
                    .push(RecoveryClassification::DiscardedTemporary { file_name });
                continue;
            }
            if !file_name.ends_with(".json") || !file_type.is_file() {
                return Err(corrupt_error("unexpected task store entry"));
            }

            let encoded_task_id = file_name
                .strip_suffix(".json")
                .expect("suffix checked above");
            let task_id = encoded_task_id
                .parse::<TaskId>()
                .map_err(|_| corrupt_error("task record file name is not a canonical TaskId"))?;
            let record = self.read_record(task_id)?;
            if record.status != InvocationStatus::Working {
                continue;
            }
            if record.resume.is_some() {
                report
                    .classifications
                    .push(RecoveryClassification::ResumableWorking { task_id });
            } else {
                let recovered = self.transition_record(
                    record,
                    TaskTransition::Fail {
                        status_message: SafeStatusMessage::from_static("interrupted"),
                    },
                )?;
                self.publish_record(&recovered, || {})?;
                report
                    .classifications
                    .push(RecoveryClassification::InterruptedNonResumable { task_id });
            }
        }
        Ok(report)
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, ()>, InvocationStoreError> {
        self.writer
            .lock()
            .map_err(|_| InvocationStoreError::Storage("task store writer lock poisoned".into()))
    }

    fn record_path(&self, task_id: TaskId) -> PathBuf {
        self.root.join(format!("{task_id}.json"))
    }

    fn read_record(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
        Self::read_committed(&self.root, task_id)
    }

    fn read_committed(
        root: &Path,
        task_id: TaskId,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let file_name = format!("{task_id}.json");
        let directory = open_directory_nofollow(root)
            .map_err(|error| storage_error("open task store for reading", error))?;
        let mut file = match open_regular_child_nofollow(&directory, OsStr::new(&file_name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(InvocationStoreError::NotFound)
            }
            Err(error) => return Err(storage_error("open committed task record", error)),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| storage_error("read committed task record", error))?;
        let record: StoredInvocationRecord = serde_json::from_slice(&bytes)
            .map_err(|_| corrupt_error("committed task record is not valid versioned JSON"))?;
        validate_record(&record)?;
        if record.task_id != task_id {
            return Err(corrupt_error(
                "task record identity does not match its file name",
            ));
        }
        Ok(record)
    }

    fn publish_record<F>(
        &self,
        record: &StoredInvocationRecord,
        before_replace: F,
    ) -> Result<(), InvocationStoreError>
    where
        F: FnOnce(),
    {
        validate_record(record)?;
        let target = self.record_path(record.task_id);
        let temporary_name = format!(".{}.{}.tmp", record.task_id, Uuid::new_v4());
        let temporary = self.root.join(&temporary_name);
        let directory = open_directory_nofollow(&self.root)
            .map_err(|error| storage_error("open task store for publication", error))?;
        let mut file = create_new_regular_child(&directory, OsStr::new(&temporary_name))
            .map_err(|error| storage_error("create private task staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&file) {
            let _ = fs::remove_file(&temporary);
            return Err(storage_error("restrict task staging file", error));
        }
        let bytes = serde_json::to_vec(record)
            .map_err(|_| corrupt_error("task record could not be serialized"))?;
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&temporary);
            return Err(storage_error("flush task staging file", error));
        }
        drop(file);

        before_replace();
        if let Err(error) = replace_file_atomically(&temporary, &target) {
            let _ = fs::remove_file(&temporary);
            return Err(storage_error("atomically publish task record", error));
        }
        sync_parent_directory(&self.root)
            .map_err(|error| storage_error("sync committed task directory", error))
    }

    fn create_with_after_publish_hook<F>(
        &self,
        new_record: NewInvocationRecord,
        after_publish: F,
    ) -> Result<StoredInvocationRecord, InvocationStoreError>
    where
        F: FnOnce(TaskId),
    {
        let _writer = self.lock_writer()?;
        let task_id = loop {
            let candidate = TaskId::new();
            if !self.record_path(candidate).try_exists().map_err(|error| {
                storage_error("inspect task identity before durable creation", error)
            })? {
                break candidate;
            }
        };
        let record = new_record.into_stored(task_id, self.clock.now_epoch_millis());
        self.publish_record(&record, || {})?;
        after_publish(task_id);
        Ok(record)
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
        let _writer = self.lock_writer()?;
        let record = self.read_record(task_id)?;
        if self.is_expired(&record) {
            return Err(InvocationStoreError::Expired);
        }
        let updated = self.transition_record(record, transition)?;
        self.publish_record(&updated, before_publish)?;
        Ok(updated)
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
            }
            TaskTransition::Complete {
                status_message,
                result,
            } if record.status == InvocationStatus::Working => {
                record.status = InvocationStatus::Completed;
                record.status_message = status_message;
                record.result = Some(*result);
                record.resume = None;
            }
            TaskTransition::Fail { status_message }
                if record.status == InvocationStatus::Working =>
            {
                record.status = InvocationStatus::Failed;
                record.status_message = status_message;
                record.result = None;
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
        let _writer = self.lock_writer()?;
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
        record.resume = None;
        self.publish_record(&record, || {})?;
        Ok(record)
    }
}

fn validate_record(record: &StoredInvocationRecord) -> Result<(), InvocationStoreError> {
    if record.schema_version != INVOCATION_RECORD_SCHEMA_VERSION {
        return Err(corrupt_error("unsupported task record schema version"));
    }
    if !record.tool.starts_with("unica.") {
        return Err(corrupt_error("task record tool identity is not canonical"));
    }
    if record.updated_at_epoch_ms < record.created_at_epoch_ms {
        return Err(corrupt_error("task record timestamp moved backwards"));
    }
    let shape_is_valid = match record.status {
        InvocationStatus::Queued | InvocationStatus::Working => record.result.is_none(),
        InvocationStatus::Completed => record.result.is_some() && record.resume.is_none(),
        InvocationStatus::Failed | InvocationStatus::Cancelled => {
            record.result.is_none() && record.resume.is_none()
        }
    };
    if !shape_is_valid {
        return Err(corrupt_error("task record status payload is inconsistent"));
    }
    Ok(())
}

fn corrupt_error(message: &'static str) -> InvocationStoreError {
    InvocationStoreError::Corrupt(message.to_string())
}

fn storage_error(operation: &'static str, error: std::io::Error) -> InvocationStoreError {
    InvocationStoreError::Storage(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{FileInvocationStore, RecoveryClassification};
    use crate::application::invocation_store::{
        EpochMillisClock, InvocationStore, InvocationStoreError, NewInvocationRecord,
        SafeStatusMessage, TaskTransition,
    };
    use crate::domain::invocation::{
        DeliveryResume, DomainResult, InvocationId, InvocationStatus, NormalizedArgumentsHash,
        ResumeDescriptor, SafeIdentityHash,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

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

    fn new_record(ttl_ms: u64, resume: Option<ResumeDescriptor>) -> NewInvocationRecord {
        NewInvocationRecord::new(
            InvocationId::new(),
            "unica.view",
            NormalizedArgumentsHash::from_sha256([0x22; 32]),
            SafeIdentityHash::from_sha256([0x33; 32]),
            SafeStatusMessage::from_static("queued"),
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
                    status_message: SafeStatusMessage::from_static("working"),
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
                    status_message: SafeStatusMessage::from_static("working"),
                },
            )
            .unwrap();
        let result = DomainResult::success("complete");
        store
            .update(
                created.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::from_static("completed"),
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
                    status_message: SafeStatusMessage::from_static("working"),
                },
            )
            .unwrap();
        let terminal = store
            .update(
                created.task_id,
                TaskTransition::Complete {
                    status_message: SafeStatusMessage::from_static("completed"),
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
                    status_message: SafeStatusMessage::from_static("working"),
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
            .cancel(created.task_id, SafeStatusMessage::from_static("cancelled"))
            .unwrap();
        clock.set(6_500);
        let second = store
            .cancel(
                created.task_id,
                SafeStatusMessage::from_static("cancelled again"),
            )
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
    fn committed_record_survives_reopening() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(8_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);

        assert!(report.classifications.is_empty());
        assert_eq!(reopened.get(created.task_id).unwrap(), created);
    }

    #[test]
    fn nonresumable_working_record_recovers_as_interrupted_failure() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(9_000));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store.create(new_record(10_000, None)).unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::from_static("working"),
                },
            )
            .unwrap();
        drop(store);
        clock.set(9_100);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(created.task_id).unwrap();

        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(recovered.status_message.as_str(), "interrupted");
        assert!(report.classifications.contains(
            &RecoveryClassification::InterruptedNonResumable {
                task_id: created.task_id,
            }
        ));
    }

    #[test]
    fn resumable_delivery_keeps_its_task_identity_after_recovery() {
        let root = tempfile::tempdir().unwrap();
        let clock = Arc::new(ManualEpochClock::at(10_000));
        let resume = ResumeDescriptor::Delivery(DeliveryResume::new(
            SafeIdentityHash::from_sha256([0x44; 32]),
        ));
        let (store, _) = open_store(root.path(), clock.clone());
        let created = store
            .create(new_record(10_000, Some(resume.clone())))
            .unwrap();
        store
            .update(
                created.task_id,
                TaskTransition::StartWorking {
                    status_message: SafeStatusMessage::from_static("delivering"),
                },
            )
            .unwrap();
        drop(store);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(created.task_id).unwrap();

        assert_eq!(recovered.task_id, created.task_id);
        assert_eq!(recovered.status, InvocationStatus::Working);
        assert_eq!(recovered.resume, Some(resume));
        assert!(report
            .classifications
            .contains(&RecoveryClassification::ResumableWorking {
                task_id: created.task_id,
            }));
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
            "unica.run",
            NormalizedArgumentsHash::from_sha256(digest),
            SafeIdentityHash::from_sha256([0x55; 32]),
            SafeStatusMessage::from_static("queued"),
            250,
            10_000,
            None,
        );
        store.create(record).unwrap();

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
