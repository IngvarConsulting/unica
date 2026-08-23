//! Sole-writer durable Task and Invocation store used by the versioned daemon.

use crate::application::invocation_store::{
    CommitOperation, EpochMillisClock, InvocationStore, InvocationStoreError, NewInvocationRecord,
    SafeStatusMessage, StoredInvocationRecord, TaskTransition, INVOCATION_RECORD_SCHEMA_VERSION,
};
use crate::domain::invocation::{InvocationStatus, TaskId};
use crate::infrastructure::platform::filesystem::{
    create_new_regular_child, file_identity, metadata_is_link_or_reparse_point,
    open_directory_nofollow, open_directory_ownership_lock, open_regular_child_nofollow,
    read_directory_names_bounded, remove_identity_bound_regular_child,
    replace_identity_bound_regular_child, restrict_stage_to_owner, sync_directory, FileIdentity,
};
use fs2::FileExt;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
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

const STORE_LOCK_FILE: &str = ".invocation-store.lock";

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
    writer: Mutex<()>,
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
        let root_identity = file_identity(&root)
            .map_err(|error| storage_error("capture task store root identity", error))?;
        let root_lock = acquire_root_lock(&root)?;

        let store = Self {
            root,
            root_identity,
            _root_lock: root_lock,
            clock,
            writer: Mutex::new(()),
            #[cfg(test)]
            next_publication_failure: Mutex::new(None),
        };
        let report = store.recover()?;
        Ok((store, report))
    }

    fn recover(&self) -> Result<RecoveryReport, InvocationStoreError> {
        let _writer = self.lock_writer()?;
        self.verify_root_identity()?;
        let entries = read_directory_names_bounded(&self.root, usize::MAX, || Ok(()))
            .map_err(|error| storage_error("enumerate task store", error))?;
        let mut report = RecoveryReport::default();

        for entry_name in entries {
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
                        status_message: SafeStatusMessage::Interrupted,
                    },
                )?;
                self.publish_record(&recovered, CommitOperation::Recovery, || {})?;
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

    fn read_record(&self, task_id: TaskId) -> Result<StoredInvocationRecord, InvocationStoreError> {
        self.verify_root_identity()?;
        Self::read_committed_from(&self.root, task_id)
    }

    fn read_committed_from(
        root: &File,
        task_id: TaskId,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let file_name = format!("{task_id}.json");
        let mut file = match open_regular_child_nofollow(root, OsStr::new(&file_name)) {
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

    #[cfg(test)]
    fn read_committed(
        root: &Path,
        task_id: TaskId,
    ) -> Result<StoredInvocationRecord, InvocationStoreError> {
        let directory = open_directory_nofollow(root)
            .map_err(|error| storage_error("open task store for test reading", error))?;
        Self::read_committed_from(&directory, task_id)
    }

    fn publish_record<F>(
        &self,
        record: &StoredInvocationRecord,
        operation: CommitOperation,
        before_replace: F,
    ) -> Result<(), InvocationStoreError>
    where
        F: FnOnce(),
    {
        validate_record(record)?;
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
        let bytes = match serde_json::to_vec(record) {
            Ok(bytes) => bytes,
            Err(_) => {
                let _ = remove_identity_bound_regular_child(
                    &self.root,
                    temporary_name,
                    temporary_identity,
                    &file,
                );
                return Err(corrupt_error("task record could not be serialized"));
            }
        };
        if let Err(error) = file.write_all(&bytes).and_then(|_| file.sync_all()) {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(storage_error("flush task staging file", error));
        }

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
        if let Err(error) = replace_identity_bound_regular_child(
            &self.root,
            temporary_name,
            temporary_identity,
            &file,
            OsStr::new(&target_name),
        ) {
            let _ = remove_identity_bound_regular_child(
                &self.root,
                temporary_name,
                temporary_identity,
                &file,
            );
            return Err(storage_error("atomically publish task record", error));
        }
        #[cfg(test)]
        if injected_failure == Some(PublicationFailure::AfterRenameBeforeSync) {
            return Err(InvocationStoreError::CommitUncertain {
                task_id: record.task_id,
                operation,
            });
        }
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
        let _writer = self.lock_writer()?;
        let task_id = loop {
            let candidate = TaskId::new();
            let candidate_name = format!("{candidate}.json");
            let names = read_directory_names_bounded(&self.root, usize::MAX, || Ok(())).map_err(
                |error| storage_error("inspect task identity before durable creation", error),
            )?;
            if !names.iter().any(|name| name == OsStr::new(&candidate_name)) {
                break candidate;
            }
        };
        let record = new_record.into_stored(task_id, self.clock.now_epoch_millis());
        self.publish_record(&record, CommitOperation::Create, || {})?;
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
        self.publish_record(&updated, CommitOperation::Update, before_publish)?;
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
        self.publish_record(&record, CommitOperation::Cancel, || {})?;
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
        InvocationStatus::Queued | InvocationStatus::Working => record.result.is_none(),
        InvocationStatus::Completed => record.result.is_some() && record.resume.is_none(),
        InvocationStatus::Failed | InvocationStatus::Cancelled => {
            record.result.is_none() && record.resume.is_none()
        }
    };
    if !shape_is_valid {
        return Err(corrupt_error("task record status payload is inconsistent"));
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

fn corrupt_error(message: &'static str) -> InvocationStoreError {
    InvocationStoreError::Corrupt(message.to_string())
}

fn storage_error(operation: &'static str, error: std::io::Error) -> InvocationStoreError {
    InvocationStoreError::Storage(format!("{operation}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        validate_record, FileInvocationStore, PublicationFailure, RecoveryClassification,
        STORE_LOCK_FILE,
    };
    use crate::application::invocation_store::{
        CommitOperation, EpochMillisClock, InvocationStore, InvocationStoreError,
        NewInvocationRecord, SafeStatusMessage, TaskTransition, ToolIdentity,
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
    fn failed_rename_preserves_the_previous_committed_record_after_reopen() {
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
        drop(store);

        let (reopened, _) = open_store(root.path(), clock);
        assert_eq!(reopened.get(created.task_id).unwrap(), created);
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
        assert_eq!(visible.status, InvocationStatus::Queued);
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
        assert_eq!(visible.status, InvocationStatus::Working);
        assert_eq!(visible.status_message, SafeStatusMessage::Working);
        assert_eq!(visible.updated_at_epoch_ms, 1_750);
        assert_eq!(visible.resume, Some(resume));
        assert!(report
            .classifications
            .contains(&RecoveryClassification::ResumableWorking {
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
                    status_message: SafeStatusMessage::Working,
                },
            )
            .unwrap();
        drop(store);
        clock.set(9_100);

        let (reopened, report) = open_store(root.path(), clock);
        let recovered = reopened.get(created.task_id).unwrap();

        assert_eq!(recovered.status, InvocationStatus::Failed);
        assert_eq!(recovered.status_message, SafeStatusMessage::Interrupted);
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
                    status_message: SafeStatusMessage::Delivering,
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
            ToolIdentity::Run,
            NormalizedArgumentsHash::from_sha256(digest),
            SafeIdentityHash::from_sha256([0x55; 32]),
            SafeStatusMessage::Queued,
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
        let record =
            new_record(10_000, None).into_stored(crate::domain::invocation::TaskId::new(), 12_000);
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
            create_directory_link_fixture_for_test, FileLinkFixtureOutcome,
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
        fs::rename(&root_path, &displaced_path).unwrap();
        let link_outcome =
            create_directory_link_fixture_for_test(&attacker_path, &root_path).unwrap();
        if link_outcome != FileLinkFixtureOutcome::Created {
            return;
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
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                &fs::read(displaced_path.join(format!("{}.json", created.task_id))).unwrap()
            )
            .unwrap()["status"],
            "working"
        );
        assert!(fs::read_dir(&attacker_path).unwrap().next().is_none());
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
