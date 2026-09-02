//! Isolated sole-writer TaskStore for the private protocol-v5 daemon.

use crate::application::invocation_store::{
    canonical_result_size, CanonicalResultSizeError, EpochMillisClock, MAX_TASK_RECORD_BYTES,
};
use crate::application::invocation_store_v5::{
    InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, TaskStoreRecoveryCatalog,
    V5CommitOperation, V5DeleteTerminalOutcome, V5StartWorkingOutcome, V5StoredInvocationRecord,
    V5StoredTask, V5TaskIdentity, V5TaskMismatch, V5TaskRecoveryEntry, V5TaskRetirement,
    V5TaskStatus, V5TaskStoreError, V5TerminalPublication, MAX_V5_TASK_RECORDS,
};
use crate::application::receipt_ledger::{canonical_v5_terminal, ReceiptTerminalOutcome};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::TaskId;
use crate::infrastructure::platform::filesystem::{
    create_new_regular_child, file_identity, open_directory_ownership_lock,
    open_regular_child_nofollow, read_directory_names_bounded, remove_identity_bound_regular_child,
    rename_identity_bound_regular_child_no_replace, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, verify_owner_only_acl, RetainedDirectoryCapability,
};
use fs2::FileExt;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::Duration;
use uuid::Uuid;

const STORE_LOCK_FILE: &str = ".task-store-v5.lock";
const STORE_WRITER_WAIT_SLICE: Duration = Duration::from_millis(10);
const MAX_INSPECTION_EXTRA_ENTRIES: usize = 64;

#[derive(Debug, Clone, Copy)]
struct StoreLimits {
    max_records: usize,
    max_record_bytes: usize,
}

impl StoreLimits {
    const fn production() -> Self {
        Self {
            max_records: MAX_V5_TASK_RECORDS,
            max_record_bytes: MAX_TASK_RECORD_BYTES,
        }
    }
}

#[derive(Default)]
struct StoreCatalog {
    records: HashMap<TaskId, V5StoredInvocationRecord>,
}

#[cfg(any(test, feature = "receipt-ledger-test-support"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PublicationFailure {
    AfterRenameBeforeSync,
    AfterDeleteBeforeSync,
}

/// The retained root is dedicated to protocol v5. It never reads or rewrites
/// the legacy TaskStore namespace and holds the sole-writer lock for its life.
pub(crate) struct FileInvocationStoreV5 {
    root: RetainedDirectoryCapability,
    root_file: File,
    _ownership_lock: File,
    clock: Arc<dyn EpochMillisClock>,
    writer: Mutex<StoreCatalog>,
    limits: StoreLimits,
    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    next_publication_failure: Mutex<Option<PublicationFailure>>,
}

impl FileInvocationStoreV5 {
    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn seed_exact_records_bulk_for_test(
        &self,
        records: Vec<V5StoredInvocationRecord>,
        deadline: ProviderDeadline,
    ) -> Result<(), V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        if !writer.records.is_empty() || records.len() > self.limits.max_records {
            return Err(V5TaskStoreError::Capacity {
                max_records: self.limits.max_records,
            });
        }
        self.verify_root_authority()?;
        let mut prepared = Vec::with_capacity(records.len());
        for record in records {
            validate_record(&record)?;
            let encoded = serde_json::to_vec(&record)
                .map_err(|_| V5TaskStoreError::Corrupt("Task fixture serialization failed"))?;
            if encoded.len() > self.limits.max_record_bytes {
                return Err(V5TaskStoreError::RecordTooLarge {
                    max_bytes: self.limits.max_record_bytes,
                });
            }
            if writer.records.contains_key(&record.task_id)
                || prepared.iter().any(
                    |(prepared_record, _): &(V5StoredInvocationRecord, Vec<u8>)| {
                        prepared_record.task_id == record.task_id
                    },
                )
            {
                return Err(V5TaskStoreError::Mismatch {
                    task_id: record.task_id,
                    reason: V5TaskMismatch::ExistingRecord,
                });
            }
            prepared.push((record, encoded));
        }
        for (index, (record, encoded)) in prepared.iter().enumerate() {
            if index % 128 == 0 {
                check_deadline(deadline)?;
            }
            let target_name = format!("{}.json", record.task_id);
            let mut file = create_new_regular_child(&self.root_file, OsStr::new(&target_name))
                .map_err(|error| storage_error("create bulk Task fixture record", error))?;
            restrict_stage_to_owner(&file)
                .map_err(|error| storage_error("restrict bulk Task fixture record", error))?;
            file.write_all(encoded)
                .map_err(|error| storage_error("write bulk Task fixture record", error))?;
        }
        sync_directory(&self.root_file)
            .map_err(|error| storage_error("sync bulk Task fixture directory", error))?;
        for (record, _) in prepared {
            writer.records.insert(record.task_id, record);
        }
        Ok(())
    }

    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn seed_exact_record_for_test(
        &self,
        record: V5StoredInvocationRecord,
        deadline: ProviderDeadline,
    ) -> Result<(), V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        if writer.records.contains_key(&record.task_id) {
            return Err(V5TaskStoreError::Mismatch {
                task_id: record.task_id,
                reason: V5TaskMismatch::ExistingRecord,
            });
        }
        if writer.records.len() >= self.limits.max_records {
            return Err(V5TaskStoreError::Capacity {
                max_records: self.limits.max_records,
            });
        }
        if record.version == 0
            || record.ttl_ms == 0
            || record.poll_interval_ms == 0
            || record.updated_at_epoch_ms < record.created_at_epoch_ms
        {
            return Err(V5TaskStoreError::Corrupt(
                "test Task seed violates the persisted record contract",
            ));
        }
        self.publish_record(&mut writer, &record, V5CommitOperation::Create, deadline)
    }

    pub(crate) fn open_inspect_only(
        root: impl AsRef<Path>,
        clock: Arc<dyn EpochMillisClock>,
        deadline: ProviderDeadline,
    ) -> Result<(Self, TaskStoreRecoveryCatalog), V5TaskStoreError> {
        check_deadline(deadline)?;
        let root = RetainedDirectoryCapability::open(root.as_ref())
            .map_err(|error| storage_error("retain protocol-v5 task root", error))?;
        Self::open_retained_directory_inspect_only_with_limits(
            root,
            clock,
            deadline,
            StoreLimits::production(),
        )
    }

    pub(crate) fn open_retained_directory_inspect_only(
        root: RetainedDirectoryCapability,
        clock: Arc<dyn EpochMillisClock>,
        deadline: ProviderDeadline,
    ) -> Result<(Self, TaskStoreRecoveryCatalog), V5TaskStoreError> {
        Self::open_retained_directory_inspect_only_with_limits(
            root,
            clock,
            deadline,
            StoreLimits::production(),
        )
    }

    fn open_retained_directory_inspect_only_with_limits(
        root: RetainedDirectoryCapability,
        clock: Arc<dyn EpochMillisClock>,
        deadline: ProviderDeadline,
        limits: StoreLimits,
    ) -> Result<(Self, TaskStoreRecoveryCatalog), V5TaskStoreError> {
        check_deadline(deadline)?;
        root.validate_named_identity()
            .map_err(|error| storage_error("validate protocol-v5 task root", error))?;
        let root_file = root
            .try_clone_directory()
            .map_err(|error| storage_error("clone protocol-v5 task root", error))?;
        verify_owner_only_acl(&root_file)
            .map_err(|error| storage_error("verify protocol-v5 task root ownership", error))?;
        let ownership_lock = open_directory_ownership_lock(&root_file, OsStr::new(STORE_LOCK_FILE))
            .map_err(|error| storage_error("open protocol-v5 task ownership object", error))?;
        verify_owner_only_acl(&ownership_lock)
            .map_err(|error| storage_error("verify protocol-v5 task ownership object", error))?;
        match FileExt::try_lock_exclusive(&ownership_lock) {
            Ok(()) => {}
            Err(error) if lock_is_contended(&error) => return Err(V5TaskStoreError::AlreadyOwned),
            Err(error) => {
                return Err(storage_error(
                    "acquire protocol-v5 task ownership lock",
                    error,
                ))
            }
        }

        let store = Self {
            root,
            root_file,
            _ownership_lock: ownership_lock,
            clock,
            writer: Mutex::new(StoreCatalog::default()),
            limits,
            #[cfg(any(test, feature = "receipt-ledger-test-support"))]
            next_publication_failure: Mutex::new(None),
        };
        let catalog = store.inspect_only(deadline)?;
        Ok((store, catalog))
    }

    #[cfg(test)]
    fn open_with_limits_for_test(
        root: impl AsRef<Path>,
        clock: Arc<dyn EpochMillisClock>,
        max_records: usize,
        deadline: ProviderDeadline,
    ) -> Result<(Self, TaskStoreRecoveryCatalog), V5TaskStoreError> {
        let root = RetainedDirectoryCapability::open(root.as_ref())
            .map_err(|error| storage_error("retain protocol-v5 task root", error))?;
        Self::open_retained_directory_inspect_only_with_limits(
            root,
            clock,
            deadline,
            StoreLimits {
                max_records,
                max_record_bytes: MAX_TASK_RECORD_BYTES,
            },
        )
    }

    fn inspect_only(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<TaskStoreRecoveryCatalog, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        self.verify_root_authority()?;
        let maximum_entries = self
            .limits
            .max_records
            .saturating_add(MAX_INSPECTION_EXTRA_ENTRIES)
            .saturating_add(1);
        let entries = read_directory_names_bounded(&self.root_file, maximum_entries, || {
            checkpoint_io(deadline)
        })
        .map_err(|error| {
            if error.kind() == io::ErrorKind::TimedOut {
                V5TaskStoreError::DeadlineExceeded
            } else if error.kind() == io::ErrorKind::FileTooLarge {
                V5TaskStoreError::Capacity {
                    max_records: self.limits.max_records,
                }
            } else {
                storage_error("enumerate protocol-v5 task root", error)
            }
        })?;

        let mut recovery_entries = Vec::new();
        let mut abandoned_staging = Vec::new();
        for entry_name in entries {
            check_deadline(deadline)?;
            let file_name = entry_name.to_str().ok_or(V5TaskStoreError::Corrupt(
                "task root entry name is not UTF-8",
            ))?;
            if file_name == STORE_LOCK_FILE {
                continue;
            }
            if file_name.starts_with('.') && file_name.ends_with(".tmp") {
                let staged =
                    open_regular_child_nofollow(&self.root_file, &entry_name).map_err(|_| {
                        V5TaskStoreError::Corrupt("task staging entry is not a regular file")
                    })?;
                verify_owner_only_acl(&staged).map_err(|error| {
                    storage_error("verify protocol-v5 task staging ownership", error)
                })?;
                let identity = file_identity(&staged).map_err(|error| {
                    storage_error("identify protocol-v5 task staging entry", error)
                })?;
                abandoned_staging.push((entry_name, identity, staged));
                continue;
            }
            let encoded_task_id =
                file_name
                    .strip_suffix(".json")
                    .ok_or(V5TaskStoreError::Corrupt(
                        "task root entry has an unsupported name",
                    ))?;
            let task_id = encoded_task_id
                .parse::<TaskId>()
                .map_err(|_| V5TaskStoreError::Corrupt("task file name is not a TaskId"))?;
            if writer.records.len() >= self.limits.max_records {
                return Err(V5TaskStoreError::Capacity {
                    max_records: self.limits.max_records,
                });
            }
            let record =
                Self::read_committed_from(&self.root_file, task_id, self.limits.max_record_bytes)?;
            recovery_entries.push(V5TaskRecoveryEntry::from_record(&record));
            writer.records.insert(task_id, record);
        }
        for (name, identity, staged) in abandoned_staging {
            check_deadline(deadline)?;
            remove_identity_bound_regular_child(&self.root_file, &name, identity, &staged)
                .map_err(|error| {
                    storage_error("remove abandoned protocol-v5 task staging entry", error)
                })?;
        }
        sync_directory(&self.root_file)
            .map_err(|error| storage_error("sync protocol-v5 task staging cleanup", error))?;
        self.verify_root_authority()?;
        Ok(TaskStoreRecoveryCatalog::new(recovery_entries))
    }

    fn lock_writer(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<MutexGuard<'_, StoreCatalog>, V5TaskStoreError> {
        loop {
            check_deadline(deadline)?;
            match self.writer.try_lock() {
                Ok(writer) => {
                    check_deadline(deadline)?;
                    return Ok(writer);
                }
                Err(TryLockError::Poisoned(poisoned)) => {
                    check_deadline(deadline)?;
                    return Ok(poisoned.into_inner());
                }
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(deadline.remaining().min(STORE_WRITER_WAIT_SLICE));
                }
            }
        }
    }

    fn verify_root_authority(&self) -> Result<(), V5TaskStoreError> {
        self.root
            .validate_named_identity()
            .map_err(|error| storage_error("validate protocol-v5 task root", error))?;
        verify_owner_only_acl(&self.root_file)
            .map_err(|error| storage_error("verify protocol-v5 task root ownership", error))
    }

    fn read_record(
        &self,
        task_id: TaskId,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let record =
            Self::read_committed_from(&self.root_file, task_id, self.limits.max_record_bytes)?;
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        Ok(record)
    }

    fn read_committed_from(
        root: &File,
        task_id: TaskId,
        max_record_bytes: usize,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let file_name = format!("{task_id}.json");
        let file = match open_regular_child_nofollow(root, OsStr::new(&file_name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(V5TaskStoreError::NotFound { task_id })
            }
            Err(error) => return Err(storage_error("open protocol-v5 task record", error)),
        };
        verify_owner_only_acl(&file)
            .map_err(|error| storage_error("verify protocol-v5 task record ownership", error))?;
        let limit = u64::try_from(max_record_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let mut bytes = Vec::new();
        file.take(limit)
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read protocol-v5 task record", error))?;
        if bytes.len() > max_record_bytes {
            return Err(V5TaskStoreError::RecordTooLarge {
                max_bytes: max_record_bytes,
            });
        }
        let record: V5StoredInvocationRecord = serde_json::from_slice(&bytes)
            .map_err(|_| V5TaskStoreError::Corrupt("task record is not strict protocol-v5 JSON"))?;
        validate_record(&record)?;
        if record.task_id != task_id {
            return Err(V5TaskStoreError::Corrupt(
                "task record identity does not match its file name",
            ));
        }
        Ok(record)
    }

    fn publish_record(
        &self,
        catalog: &mut StoreCatalog,
        record: &V5StoredInvocationRecord,
        operation: V5CommitOperation,
        deadline: ProviderDeadline,
    ) -> Result<(), V5TaskStoreError> {
        validate_record(record)?;
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let target_name = format!("{}.json", record.task_id);
        let temporary_name = format!(".{}.{}.tmp", record.task_id, Uuid::new_v4());
        let temporary_name = OsStr::new(&temporary_name);
        let mut staged = create_new_regular_child(&self.root_file, temporary_name)
            .map_err(|error| storage_error("create protocol-v5 task staging file", error))?;
        let staged_identity = file_identity(&staged)
            .map_err(|error| storage_error("identify protocol-v5 task staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&staged) {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(storage_error(
                "restrict protocol-v5 task staging file",
                error,
            ));
        }
        if let Err(error) =
            serialize_record_bounded(&mut staged, record, self.limits.max_record_bytes, deadline)
        {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(error);
        }
        if let Err(error) = staged.sync_all() {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(storage_error("flush protocol-v5 task staging file", error));
        }
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let target_name = OsStr::new(&target_name);
        let publication = if operation == V5CommitOperation::Create {
            rename_identity_bound_regular_child_no_replace(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
                &self.root_file,
                target_name,
            )
        } else {
            replace_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
                target_name,
            )
        };
        if let Err(error) = publication {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            if operation == V5CommitOperation::Create
                && error.kind() == io::ErrorKind::AlreadyExists
            {
                return Err(V5TaskStoreError::Mismatch {
                    task_id: record.task_id,
                    reason: V5TaskMismatch::ExistingRecord,
                });
            }
            return Err(storage_error("publish protocol-v5 task record", error));
        }

        catalog.records.insert(record.task_id, record.clone());
        #[cfg(any(test, feature = "receipt-ledger-test-support"))]
        if self.take_publication_failure()? == Some(PublicationFailure::AfterRenameBeforeSync) {
            return Err(V5TaskStoreError::CommitUncertain {
                task_id: record.task_id,
                operation,
            });
        }
        check_deadline(deadline).map_err(|_| V5TaskStoreError::CommitUncertain {
            task_id: record.task_id,
            operation,
        })?;
        sync_directory(&self.root_file).map_err(|_| V5TaskStoreError::CommitUncertain {
            task_id: record.task_id,
            operation,
        })?;
        self.verify_root_authority()
            .map_err(|_| V5TaskStoreError::CommitUncertain {
                task_id: record.task_id,
                operation,
            })
    }

    fn delete_terminal_record(
        &self,
        catalog: &mut StoreCatalog,
        retirement: &V5TaskRetirement,
        deadline: ProviderDeadline,
    ) -> Result<(), V5TaskStoreError> {
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let task_id = retirement.identity().task_id();
        let file_name = format!("{task_id}.json");
        let file_name = OsStr::new(&file_name);
        let retained = open_regular_child_nofollow(&self.root_file, file_name)
            .map_err(|error| storage_error("retain protocol-v5 terminal task record", error))?;
        verify_owner_only_acl(&retained).map_err(|error| {
            storage_error("verify protocol-v5 terminal task record ownership", error)
        })?;
        let retained_identity = file_identity(&retained)
            .map_err(|error| storage_error("identify protocol-v5 terminal task record", error))?;
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        remove_identity_bound_regular_child(
            &self.root_file,
            file_name,
            retained_identity,
            &retained,
        )
        .map_err(|error| storage_error("delete protocol-v5 terminal task record", error))?;
        catalog.records.remove(&task_id);

        #[cfg(any(test, feature = "receipt-ledger-test-support"))]
        if self.take_publication_failure()? == Some(PublicationFailure::AfterDeleteBeforeSync) {
            return Err(V5TaskStoreError::CommitUncertain {
                task_id,
                operation: V5CommitOperation::DeleteTerminal,
            });
        }
        check_deadline(deadline).map_err(|_| V5TaskStoreError::CommitUncertain {
            task_id,
            operation: V5CommitOperation::DeleteTerminal,
        })?;
        sync_directory(&self.root_file).map_err(|_| V5TaskStoreError::CommitUncertain {
            task_id,
            operation: V5CommitOperation::DeleteTerminal,
        })?;
        self.verify_root_authority()
            .map_err(|_| V5TaskStoreError::CommitUncertain {
                task_id,
                operation: V5CommitOperation::DeleteTerminal,
            })
    }

    fn validate_exact(
        record: &V5StoredInvocationRecord,
        identity: &V5TaskIdentity,
        expected_version: u64,
    ) -> Result<(), V5TaskStoreError> {
        if !identity.matches_record(record) {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::Identity,
            });
        }
        if record.version != expected_version {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::Version {
                    expected: expected_version,
                    actual: record.version,
                },
            });
        }
        Ok(())
    }

    fn next_version(record: &V5StoredInvocationRecord) -> Result<u64, V5TaskStoreError> {
        record
            .version
            .checked_add(1)
            .ok_or(V5TaskStoreError::Corrupt("task record version overflow"))
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    pub(crate) fn inject_next_publication_failure(&self, failure: PublicationFailure) {
        *self
            .next_publication_failure
            .lock()
            .expect("publication failure lock") = Some(failure);
    }

    #[cfg(any(test, feature = "receipt-ledger-test-support"))]
    fn take_publication_failure(&self) -> Result<Option<PublicationFailure>, V5TaskStoreError> {
        self.next_publication_failure
            .lock()
            .map(|mut failure| failure.take())
            .map_err(|_| V5TaskStoreError::Storage {
                operation: "lock protocol-v5 publication failure injection",
                message: "mutex poisoned".to_string(),
            })
    }
}

impl InvocationStoreV5 for FileInvocationStoreV5 {
    fn create_exact(
        &self,
        new_record: NewV5InvocationRecord,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let task_id = new_record.task_id();
        if writer.records.contains_key(&task_id) {
            let existing = self.read_record(task_id, deadline)?;
            if new_record.matches_record(&existing) {
                return Ok(existing);
            }
            return Err(V5TaskStoreError::Mismatch {
                task_id,
                reason: V5TaskMismatch::ExistingRecord,
            });
        }
        if writer.records.len() >= self.limits.max_records {
            return Err(V5TaskStoreError::Capacity {
                max_records: self.limits.max_records,
            });
        }
        let record = new_record.into_stored(self.clock.now_epoch_millis());
        self.publish_record(&mut writer, &record, V5CommitOperation::Create, deadline)?;
        Ok(record)
    }

    fn get(
        &self,
        task_id: TaskId,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        self.read_record(task_id, deadline)
    }

    fn request_cancel_exact(
        &self,
        identity: &V5TaskIdentity,
        expected_version: u64,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let mut record = self.read_record(identity.task_id(), deadline)?;
        Self::validate_exact(&record, identity, expected_version)?;
        if record.cancel_requested {
            return Ok(record);
        }
        record.cancel_requested = true;
        record.version = Self::next_version(&record)?;
        record.updated_at_epoch_ms = record
            .updated_at_epoch_ms
            .max(self.clock.now_epoch_millis());
        self.publish_record(
            &mut writer,
            &record,
            V5CommitOperation::RequestCancel,
            deadline,
        )?;
        Ok(record)
    }

    fn start_working_if_not_cancel_requested(
        &self,
        identity: &V5TaskIdentity,
        expected_version: u64,
        deadline: ProviderDeadline,
    ) -> Result<V5StartWorkingOutcome, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let mut record = self.read_record(identity.task_id(), deadline)?;
        if !identity.matches_record(&record) {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::Identity,
            });
        }
        if record.cancel_requested || record.task.is_terminal() {
            return Ok(V5StartWorkingOutcome::CancelOrTerminalWinner(record));
        }
        Self::validate_exact(&record, identity, expected_version)?;
        if record.task != V5StoredTask::Queued {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::State,
            });
        }
        record.task = V5StoredTask::Working;
        record.version = Self::next_version(&record)?;
        record.updated_at_epoch_ms = record
            .updated_at_epoch_ms
            .max(self.clock.now_epoch_millis());
        self.publish_record(
            &mut writer,
            &record,
            V5CommitOperation::StartWorking,
            deadline,
        )?;
        Ok(V5StartWorkingOutcome::Started(record))
    }

    fn publish_terminal_exact(
        &self,
        identity: &V5TaskIdentity,
        expected_version: u64,
        publication: V5TerminalPublication,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let mut record = self.read_record(identity.task_id(), deadline)?;
        if !identity.matches_record(&record) {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::Identity,
            });
        }
        if record.task.is_terminal() {
            if expected_version
                .checked_add(1)
                .is_some_and(|committed_version| committed_version == record.version)
                && publication.matches_task(&record.task)
            {
                return Ok(record);
            }
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::State,
            });
        }
        Self::validate_exact(&record, identity, expected_version)?;
        let state_accepts_publication = match publication.status() {
            V5TaskStatus::Completed | V5TaskStatus::Failed => record.task == V5StoredTask::Working,
            V5TaskStatus::Cancelled => {
                matches!(record.task, V5StoredTask::Queued | V5StoredTask::Working)
                    && record.cancel_requested
            }
            V5TaskStatus::Queued | V5TaskStatus::Working => false,
        };
        if !state_accepts_publication {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::State,
            });
        }
        let terminal_epoch_ms = publication.terminal_epoch_ms();
        record.task = publication.into_stored_task();
        record.version = Self::next_version(&record)?;
        record.updated_at_epoch_ms = record
            .updated_at_epoch_ms
            .max(self.clock.now_epoch_millis())
            .max(terminal_epoch_ms);
        self.publish_record(
            &mut writer,
            &record,
            V5CommitOperation::PublishTerminal,
            deadline,
        )?;
        Ok(record)
    }

    fn terminalize_recovered_exact(
        &self,
        identity: &V5TaskIdentity,
        expected_version: u64,
        reason: RecoveryTerminalReason,
        deadline: ProviderDeadline,
    ) -> Result<V5StoredInvocationRecord, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let mut record = self.read_record(identity.task_id(), deadline)?;
        if !identity.matches_record(&record) {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::Identity,
            });
        }
        if record.task.is_terminal() {
            let exact_successor = expected_version
                .checked_add(1)
                .is_some_and(|committed_version| committed_version == record.version);
            let exact_reason = matches!(
                (&reason, &record.task),
                (RecoveryTerminalReason::Cancelled, V5StoredTask::Cancelled { .. })
                    | (
                        RecoveryTerminalReason::InterruptedBeforeExecution,
                        V5StoredTask::Failed {
                            reason: crate::application::invocation_store_v5::V5SafeFailureReason::Interrupted,
                            ..
                        }
                    )
                    | (
                        RecoveryTerminalReason::OutcomeUncertain,
                        V5StoredTask::Failed {
                            reason: crate::application::invocation_store_v5::V5SafeFailureReason::OutcomeUncertain,
                            ..
                        }
                    )
            );
            if exact_successor && exact_reason {
                return Ok(record);
            }
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::State,
            });
        }
        Self::validate_exact(&record, identity, expected_version)?;
        let valid_state = match reason {
            RecoveryTerminalReason::Cancelled => {
                record.cancel_requested
                    && matches!(record.task, V5StoredTask::Queued | V5StoredTask::Working)
            }
            RecoveryTerminalReason::InterruptedBeforeExecution => {
                matches!(record.task, V5StoredTask::Queued | V5StoredTask::Working)
            }
            RecoveryTerminalReason::OutcomeUncertain => record.task == V5StoredTask::Working,
        };
        if !valid_state {
            return Err(V5TaskStoreError::Mismatch {
                task_id: identity.task_id(),
                reason: V5TaskMismatch::State,
            });
        }
        let terminal_epoch_ms = self.clock.now_epoch_millis();
        let outcome = match reason {
            RecoveryTerminalReason::Cancelled => ReceiptTerminalOutcome::Cancelled,
            RecoveryTerminalReason::InterruptedBeforeExecution => ReceiptTerminalOutcome::Failed {
                reason: crate::application::invocation_store_v5::V5SafeFailureReason::Interrupted,
            },
            RecoveryTerminalReason::OutcomeUncertain => ReceiptTerminalOutcome::Failed {
                reason:
                    crate::application::invocation_store_v5::V5SafeFailureReason::OutcomeUncertain,
            },
        };
        let terminal = canonical_v5_terminal(&outcome).map_err(|_| {
            V5TaskStoreError::Corrupt("canonical recovery terminal could not be encoded")
        })?;
        record.task = match outcome {
            ReceiptTerminalOutcome::Cancelled => V5StoredTask::Cancelled {
                terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
            },
            ReceiptTerminalOutcome::Failed { reason } => V5StoredTask::Failed {
                terminal_epoch_ms,
                terminal_digest: terminal.digest().clone(),
                reason,
            },
            ReceiptTerminalOutcome::Completed { .. } => unreachable!("recovery is never success"),
        };
        record.version = Self::next_version(&record)?;
        record.updated_at_epoch_ms = record.updated_at_epoch_ms.max(terminal_epoch_ms);
        self.publish_record(
            &mut writer,
            &record,
            V5CommitOperation::PublishTerminal,
            deadline,
        )?;
        Ok(record)
    }

    fn delete_terminal_if_expired(
        &self,
        retirement: &V5TaskRetirement,
        observed_at_epoch_ms: u64,
        deadline: ProviderDeadline,
    ) -> Result<V5DeleteTerminalOutcome, V5TaskStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let task_id = retirement.identity().task_id();
        let record = match self.read_record(task_id, deadline) {
            Ok(record) => record,
            Err(V5TaskStoreError::NotFound { .. }) => {
                return Ok(V5DeleteTerminalOutcome::AlreadyAbsent(retirement.clone()))
            }
            Err(error) => return Err(error),
        };
        if !retirement.identity().matches_record(&record) {
            return Err(V5TaskStoreError::Mismatch {
                task_id,
                reason: V5TaskMismatch::Identity,
            });
        }
        if record.version != retirement.expected_terminal_version() {
            return Err(V5TaskStoreError::Mismatch {
                task_id,
                reason: V5TaskMismatch::Version {
                    expected: retirement.expected_terminal_version(),
                    actual: record.version,
                },
            });
        }
        let expired = observed_at_epoch_ms
            .checked_sub(record.updated_at_epoch_ms)
            .is_some_and(|elapsed| elapsed >= record.ttl_ms);
        if !record.task.is_terminal() || !retirement.matches_terminal_record(&record) || !expired {
            return Err(V5TaskStoreError::Mismatch {
                task_id,
                reason: V5TaskMismatch::State,
            });
        }
        self.delete_terminal_record(&mut writer, retirement, deadline)?;
        Ok(V5DeleteTerminalOutcome::Deleted(retirement.clone()))
    }
}

fn validate_record(record: &V5StoredInvocationRecord) -> Result<(), V5TaskStoreError> {
    if record.version == 0 {
        return Err(V5TaskStoreError::Corrupt(
            "task record version must be nonzero",
        ));
    }
    if record.updated_at_epoch_ms < record.created_at_epoch_ms {
        return Err(V5TaskStoreError::Corrupt(
            "task record timestamp moved backwards",
        ));
    }
    let terminal_epoch_ms = match &record.task {
        V5StoredTask::Completed {
            terminal_epoch_ms, ..
        }
        | V5StoredTask::Failed {
            terminal_epoch_ms, ..
        }
        | V5StoredTask::Cancelled {
            terminal_epoch_ms, ..
        } => Some(*terminal_epoch_ms),
        V5StoredTask::Queued | V5StoredTask::Working => None,
    };
    if terminal_epoch_ms.is_some_and(|terminal_epoch_ms| {
        terminal_epoch_ms < record.created_at_epoch_ms
            || terminal_epoch_ms > record.updated_at_epoch_ms
    }) {
        return Err(V5TaskStoreError::Corrupt(
            "task terminal timestamp is outside the record lifetime",
        ));
    }
    if let V5StoredTask::Completed { result, .. } = &record.task {
        match canonical_result_size(result) {
            Ok(_) => {}
            Err(CanonicalResultSizeError::TooLarge) => {
                return Err(V5TaskStoreError::RecordTooLarge {
                    max_bytes: MAX_TASK_RECORD_BYTES,
                })
            }
            Err(CanonicalResultSizeError::Checkpoint(never)) => match never {},
            Err(CanonicalResultSizeError::Serialization) => {
                return Err(V5TaskStoreError::Corrupt(
                    "canonical task result could not be serialized",
                ))
            }
        }
    }
    Ok(())
}

struct BoundedRecordWriter<'a> {
    file: &'a mut File,
    bytes: usize,
    max_bytes: usize,
    deadline: ProviderDeadline,
    failure: Option<V5TaskStoreError>,
}

impl Write for BoundedRecordWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if check_deadline(self.deadline).is_err() {
            self.failure = Some(V5TaskStoreError::DeadlineExceeded);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "protocol-v5 task serialization deadline expired",
            ));
        }
        let Some(next) = self.bytes.checked_add(buffer.len()) else {
            self.failure = Some(V5TaskStoreError::RecordTooLarge {
                max_bytes: self.max_bytes,
            });
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "protocol-v5 task record exceeds byte limit",
            ));
        };
        if next > self.max_bytes {
            self.failure = Some(V5TaskStoreError::RecordTooLarge {
                max_bytes: self.max_bytes,
            });
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "protocol-v5 task record exceeds byte limit",
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
    record: &V5StoredInvocationRecord,
    max_bytes: usize,
    deadline: ProviderDeadline,
) -> Result<(), V5TaskStoreError> {
    let mut writer = BoundedRecordWriter {
        file,
        bytes: 0,
        max_bytes,
        deadline,
        failure: None,
    };
    let serialized = serde_json::to_writer(&mut writer, record);
    if let Some(error) = writer.failure.take() {
        return Err(error);
    }
    if serialized.is_err() {
        return Err(V5TaskStoreError::Storage {
            operation: "serialize protocol-v5 task record",
            message: "JSON serialization failed".to_string(),
        });
    }
    check_deadline(deadline)
}

fn check_deadline(deadline: ProviderDeadline) -> Result<(), V5TaskStoreError> {
    if deadline.remaining().is_zero() {
        Err(V5TaskStoreError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

fn checkpoint_io(deadline: ProviderDeadline) -> io::Result<()> {
    check_deadline(deadline).map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "protocol-v5 task inspection deadline expired",
        )
    })
}

fn lock_is_contended(error: &io::Error) -> bool {
    let expected = fs2::lock_contended_error();
    error.kind() == io::ErrorKind::WouldBlock
        || error
            .raw_os_error()
            .zip(expected.raw_os_error())
            .is_some_and(|(actual, expected)| actual == expected)
}

fn storage_error(operation: &'static str, error: io::Error) -> V5TaskStoreError {
    V5TaskStoreError::Storage {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{FileInvocationStoreV5, PublicationFailure};
    use crate::application::invocation_store::EpochMillisClock;
    use crate::application::invocation_store_v5::V5SafeFailureReason;
    use crate::application::invocation_store_v5::{
        InvocationStoreV5, NewV5InvocationRecord, RecoveryTerminalReason, V5CommitOperation,
        V5DeleteTerminalOutcome, V5StartWorkingOutcome, V5StoredInvocationRecord,
        V5StoredInvocationSchemaVersion, V5StoredTask, V5TaskIdentity, V5TaskMismatch,
        V5TaskRetirement, V5TaskStatus, V5TaskStoreError, V5TerminalPublication,
        MAX_V5_TASK_RECORDS,
    };
    use crate::application::receipt_ledger::{ReceiptKeyDigest, TerminalDigest, V5ToolIdentity};
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::invocation::{
        DomainResult, InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use crate::infrastructure::platform::filesystem::{
        create_owner_only_directory_child, open_directory_nofollow, restrict_stage_to_owner,
    };
    use std::fs::{self, OpenOptions};
    use std::str::FromStr;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

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

    fn deadline() -> ProviderDeadline {
        ProviderDeadline::from_budget(Duration::from_secs(7))
    }

    fn physical_root(root: &tempfile::TempDir) -> std::path::PathBuf {
        let parent = fs::canonicalize(root.path()).expect("physical temporary parent");
        let parent_file = open_directory_nofollow(&parent).expect("open temporary parent");
        create_owner_only_directory_child(&parent_file, std::ffi::OsStr::new("v5"))
            .expect("create owner-only v5 root");
        parent.join("v5")
    }

    fn task_id(encoded: &str) -> TaskId {
        TaskId::from_str(encoded).expect("canonical task id")
    }

    fn invocation_id(encoded: &str) -> InvocationId {
        InvocationId::from_str(encoded).expect("canonical invocation id")
    }

    fn receipt_digest(byte: u8) -> ReceiptKeyDigest {
        ReceiptKeyDigest::from_str(&format!("{byte:02x}").repeat(32)).expect("receipt digest")
    }

    fn terminal_digest(byte: u8) -> TerminalDigest {
        TerminalDigest::from_str(&format!("{byte:02x}").repeat(32)).expect("terminal digest")
    }

    fn new_record(
        task_id: TaskId,
        invocation_id: InvocationId,
        digest: u8,
    ) -> NewV5InvocationRecord {
        NewV5InvocationRecord::new(
            V5TaskIdentity::new(task_id, invocation_id, receipt_digest(digest)),
            V5ToolIdentity::View,
            NormalizedArgumentsHash::from_sha256([0x11; 32]),
            SafeIdentityHash::from_sha256([0x22; 32]),
            100,
            3_600_000,
        )
    }

    fn stored_record(
        task_id: TaskId,
        invocation_id: InvocationId,
        digest: u8,
        version: u64,
        cancel_requested: bool,
        task: V5StoredTask,
    ) -> V5StoredInvocationRecord {
        V5StoredInvocationRecord {
            schema_version: V5StoredInvocationSchemaVersion,
            task_id,
            invocation_id,
            receipt_key_digest: receipt_digest(digest),
            tool: V5ToolIdentity::View,
            normalized_arguments_hash: NormalizedArgumentsHash::from_sha256([0x11; 32]),
            workspace_identity_hash: SafeIdentityHash::from_sha256([0x22; 32]),
            created_at_epoch_ms: 1_000,
            updated_at_epoch_ms: 1_100,
            ttl_ms: 100,
            poll_interval_ms: 100,
            version,
            cancel_requested,
            task,
        }
    }

    fn write_record(root: &std::path::Path, record: &V5StoredInvocationRecord) -> Vec<u8> {
        let bytes = serde_json::to_vec(record).expect("serialize fixture");
        let path = root.join(format!("{}.json", record.task_id));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("create fixture record");
        restrict_stage_to_owner(&file).expect("restrict fixture record");
        std::io::Write::write_all(&mut file, &bytes).expect("write fixture record");
        file.sync_all().expect("sync fixture record");
        bytes
    }

    #[test]
    fn inspect_only_removes_verified_abandoned_staging_entries() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let staging_path = root_path.join(".abandoned.tmp");
        let mut staging = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging_path)
            .expect("create abandoned staging entry");
        restrict_stage_to_owner(&staging).expect("restrict abandoned staging entry");
        std::io::Write::write_all(&mut staging, b"partial").expect("write staging entry");
        staging.sync_all().expect("sync staging entry");
        drop(staging);

        let (_store, catalog) = FileInvocationStoreV5::open_inspect_only(
            &root_path,
            Arc::new(ManualEpochClock::at(9_000)),
            deadline(),
        )
        .expect("inspect v5 store");

        assert!(catalog.entries().is_empty());
        assert!(
            !staging_path.exists(),
            "verified abandoned staging entry must be durably removed"
        );
    }

    #[test]
    fn open_inspect_only_preserves_nonterminal_bytes_and_catalogues_exact_state() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let queued_id = task_id("11111111-1111-4111-8111-111111111111");
        let working_id = task_id("22222222-2222-4222-8222-222222222222");
        let queued = stored_record(
            queued_id,
            invocation_id("33333333-3333-4333-8333-333333333333"),
            0x01,
            7,
            true,
            V5StoredTask::Queued,
        );
        let working = stored_record(
            working_id,
            invocation_id("44444444-4444-4444-8444-444444444444"),
            0x02,
            9,
            false,
            V5StoredTask::Working,
        );
        let queued_bytes = write_record(&root_path, &queued);
        let working_bytes = write_record(&root_path, &working);

        let (_store, catalog) = FileInvocationStoreV5::open_inspect_only(
            &root_path,
            Arc::new(ManualEpochClock::at(9_000)),
            deadline(),
        )
        .expect("inspect v5 store");

        assert_eq!(catalog.entries().len(), 2);
        assert_eq!(catalog.entry(queued_id).expect("queued entry").version(), 7);
        assert_eq!(
            catalog.entry(queued_id).unwrap().status(),
            V5TaskStatus::Queued
        );
        assert!(catalog.entry(queued_id).unwrap().cancel_requested());
        assert_eq!(
            catalog.entry(working_id).unwrap().status(),
            V5TaskStatus::Working
        );
        assert_eq!(
            fs::read(root_path.join(format!("{queued_id}.json"))).unwrap(),
            queued_bytes
        );
        assert_eq!(
            fs::read(root_path.join(format!("{working_id}.json"))).unwrap(),
            working_bytes
        );
    }

    #[test]
    fn create_exact_uses_preallocated_identity_and_is_idempotent_only_for_exact_readback() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(2_000));
        let (store, catalog) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock, deadline()).unwrap();
        assert!(catalog.entries().is_empty());
        let task_id = task_id("55555555-5555-4555-8555-555555555555");
        let invocation_id = invocation_id("66666666-6666-4666-8666-666666666666");
        let new = new_record(task_id, invocation_id, 0x03);

        let created = store.create_exact(new.clone(), deadline()).unwrap();
        assert_eq!(created.task_id, task_id);
        assert_eq!(created.invocation_id, invocation_id);
        assert_eq!(created.receipt_key_digest, receipt_digest(0x03));
        assert_eq!(created.version, 1);
        assert_eq!(created.task, V5StoredTask::Queued);
        assert!(!created.cancel_requested);
        assert_eq!(store.get(task_id, deadline()).unwrap(), created);
        assert_eq!(store.create_exact(new, deadline()).unwrap(), created);

        let mismatch = store
            .create_exact(new_record(task_id, invocation_id, 0x04), deadline())
            .unwrap_err();
        assert!(matches!(
            mismatch,
            V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::ExistingRecord,
                ..
            }
        ));
    }

    #[test]
    fn request_cancel_is_monotonic_and_cancel_wins_atomic_start() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(3_000));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock.clone(), deadline())
                .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("77777777-7777-4777-8777-777777777777"),
                    invocation_id("88888888-8888-4888-8888-888888888888"),
                    0x05,
                ),
                deadline(),
            )
            .unwrap();
        let identity = V5TaskIdentity::from_record(&created);
        clock.set(3_100);

        let cancelled = store
            .request_cancel_exact(&identity, 1, deadline())
            .unwrap();
        assert!(cancelled.cancel_requested);
        assert_eq!(cancelled.version, 2);
        let repeated = store
            .request_cancel_exact(&identity, 2, deadline())
            .unwrap();
        assert_eq!(repeated, cancelled, "repeat must not bump the version");
        assert_eq!(
            store
                .start_working_if_not_cancel_requested(&identity, 1, deadline())
                .unwrap(),
            V5StartWorkingOutcome::CancelOrTerminalWinner(cancelled.clone())
        );
        assert_eq!(
            store
                .start_working_if_not_cancel_requested(&identity, 2, deadline())
                .unwrap(),
            V5StartWorkingOutcome::CancelOrTerminalWinner(cancelled.clone())
        );
        assert_eq!(store.get(created.task_id, deadline()).unwrap(), cancelled);
    }

    #[test]
    fn start_working_rejects_foreign_or_stale_proof_before_committing_exact_readback() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(4_000));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock.clone(), deadline())
                .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("99999999-9999-4999-8999-999999999999"),
                    invocation_id("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"),
                    0x06,
                ),
                deadline(),
            )
            .unwrap();
        let identity = V5TaskIdentity::from_record(&created);
        let foreign = V5TaskIdentity::new(
            created.task_id,
            invocation_id("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"),
            created.receipt_key_digest.clone(),
        );
        assert!(matches!(
            store.start_working_if_not_cancel_requested(&foreign, 1, deadline()),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::Identity,
                ..
            })
        ));
        assert!(matches!(
            store.start_working_if_not_cancel_requested(&identity, 2, deadline()),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::Version {
                    expected: 2,
                    actual: 1
                },
                ..
            })
        ));
        assert_eq!(store.get(created.task_id, deadline()).unwrap(), created);

        clock.set(4_100);
        let started = store
            .start_working_if_not_cancel_requested(&identity, 1, deadline())
            .unwrap();
        let V5StartWorkingOutcome::Started(working) = started else {
            panic!("queued uncancelled task did not start");
        };
        assert_eq!(working.version, 2);
        assert_eq!(working.task, V5StoredTask::Working);
        assert_eq!(working.updated_at_epoch_ms, 4_100);
        assert_eq!(store.get(created.task_id, deadline()).unwrap(), working);
    }

    #[test]
    fn capacity_never_lazily_expires_terminal_records_and_not_found_is_typed() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let retained_id = task_id("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
        let retained = stored_record(
            retained_id,
            invocation_id("dddddddd-dddd-4ddd-8ddd-dddddddddddd"),
            0x07,
            4,
            false,
            V5StoredTask::Cancelled {
                terminal_epoch_ms: 1_100,
                terminal_digest: std::str::FromStr::from_str(&"ee".repeat(32)).unwrap(),
            },
        );
        write_record(&root_path, &retained);
        let (store, _) = FileInvocationStoreV5::open_with_limits_for_test(
            &root_path,
            Arc::new(ManualEpochClock::at(99_000)),
            1,
            deadline(),
        )
        .unwrap();

        assert_eq!(store.get(retained_id, deadline()).unwrap(), retained);
        assert!(matches!(
            store.create_exact(
                new_record(
                    task_id("eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"),
                    invocation_id("ffffffff-ffff-4fff-8fff-ffffffffffff"),
                    0x08,
                ),
                deadline(),
            ),
            Err(V5TaskStoreError::Capacity { max_records: 1 })
        ));
        assert!(matches!(
            store.get(task_id("12121212-1212-4212-8212-121212121212"), deadline()),
            Err(V5TaskStoreError::NotFound { .. })
        ));
        assert_eq!(MAX_V5_TASK_RECORDS, 4_096);
    }

    #[test]
    fn post_publish_sync_failure_is_commit_uncertain_with_exact_visible_readback() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let (store, _) = FileInvocationStoreV5::open_inspect_only(
            &root_path,
            Arc::new(ManualEpochClock::at(5_000)),
            deadline(),
        )
        .unwrap();
        let task_id = task_id("13131313-1313-4313-8313-131313131313");
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);

        assert!(matches!(
            store.create_exact(
                new_record(
                    task_id,
                    invocation_id("14141414-1414-4414-8414-141414141414"),
                    0x09,
                ),
                deadline(),
            ),
            Err(V5TaskStoreError::CommitUncertain {
                task_id: uncertain_id,
                operation: V5CommitOperation::Create,
            }) if uncertain_id == task_id
        ));
        assert_eq!(store.get(task_id, deadline()).unwrap().task_id, task_id);
    }

    #[test]
    fn completed_terminal_cas_reconciles_commit_uncertain_by_exact_readback() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(6_000));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock.clone(), deadline())
                .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("15151515-1515-4515-8515-151515151515"),
                    invocation_id("16161616-1616-4616-8616-161616161616"),
                    0x0a,
                ),
                deadline(),
            )
            .unwrap();
        let identity = created.identity();
        let V5StartWorkingOutcome::Started(working) = store
            .start_working_if_not_cancel_requested(&identity, 1, deadline())
            .unwrap()
        else {
            panic!("task did not enter working");
        };
        clock.set(6_100);
        let publication = V5TerminalPublication::Completed {
            terminal_epoch_ms: 6_100,
            terminal_digest: terminal_digest(0xaa),
            result: Box::new(DomainResult::success("done")),
        };
        store.inject_next_publication_failure(PublicationFailure::AfterRenameBeforeSync);

        assert!(matches!(
            store.publish_terminal_exact(&identity, 2, publication.clone(), deadline()),
            Err(V5TaskStoreError::CommitUncertain {
                task_id: uncertain_id,
                operation: V5CommitOperation::PublishTerminal,
            }) if uncertain_id == created.task_id
        ));
        let visible = store.get(created.task_id, deadline()).unwrap();
        assert_eq!(visible.version, 3);
        assert_eq!(visible.updated_at_epoch_ms, 6_100);
        assert_eq!(
            visible.task,
            V5StoredTask::Completed {
                terminal_epoch_ms: 6_100,
                terminal_digest: terminal_digest(0xaa),
                result: Box::new(DomainResult::success("done")),
            }
        );
        assert_eq!(
            store
                .publish_terminal_exact(&identity, working.version, publication, deadline())
                .unwrap(),
            visible,
            "an exact retry after uncertain commit must read back the winner"
        );
    }

    #[test]
    fn terminal_cas_rejects_foreign_stale_invalid_state_and_different_winner() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let (store, _) = FileInvocationStoreV5::open_inspect_only(
            &root_path,
            Arc::new(ManualEpochClock::at(7_000)),
            deadline(),
        )
        .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("17171717-1717-4717-8717-171717171717"),
                    invocation_id("18181818-1818-4818-8818-181818181818"),
                    0x0b,
                ),
                deadline(),
            )
            .unwrap();
        let identity = created.identity();
        let foreign = V5TaskIdentity::new(
            created.task_id,
            invocation_id("19191919-1919-4919-8919-191919191919"),
            created.receipt_key_digest.clone(),
        );
        let failed = V5TerminalPublication::Failed {
            terminal_epoch_ms: 7_100,
            terminal_digest: terminal_digest(0xbb),
            reason: V5SafeFailureReason::InvocationFailed,
        };

        assert!(matches!(
            store.publish_terminal_exact(&foreign, 1, failed.clone(), deadline()),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::Identity,
                ..
            })
        ));
        assert!(matches!(
            store.publish_terminal_exact(&identity, 2, failed.clone(), deadline()),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::Version {
                    expected: 2,
                    actual: 1,
                },
                ..
            })
        ));
        assert!(matches!(
            store.publish_terminal_exact(&identity, 1, failed, deadline()),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::State,
                ..
            })
        ));
        assert_eq!(store.get(created.task_id, deadline()).unwrap(), created);

        let cancel_requested = store
            .request_cancel_exact(&identity, 1, deadline())
            .unwrap();
        let cancelled = V5TerminalPublication::Cancelled {
            terminal_epoch_ms: 7_200,
            terminal_digest: terminal_digest(0xcc),
        };
        let terminal = store
            .publish_terminal_exact(&identity, 2, cancelled.clone(), deadline())
            .unwrap();
        assert_eq!(terminal.version, 3);
        assert!(terminal.cancel_requested);
        assert_eq!(
            terminal.task,
            V5StoredTask::Cancelled {
                terminal_epoch_ms: 7_200,
                terminal_digest: terminal_digest(0xcc),
            }
        );
        assert!(matches!(
            store.publish_terminal_exact(
                &identity,
                cancel_requested.version,
                V5TerminalPublication::Cancelled {
                    terminal_epoch_ms: 7_200,
                    terminal_digest: terminal_digest(0xdd),
                },
                deadline(),
            ),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::State,
                ..
            })
        ));
        assert_eq!(store.get(created.task_id, deadline()).unwrap(), terminal);
    }

    #[test]
    fn recovery_terminalizes_queued_without_starting_domain_work() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(7_500));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock, deadline()).unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("27272727-2727-4727-8727-272727272727"),
                    invocation_id("28282828-2828-4828-8828-282828282828"),
                    0x1b,
                ),
                deadline(),
            )
            .unwrap();

        let terminal = store
            .terminalize_recovered_exact(
                &created.identity(),
                created.version,
                RecoveryTerminalReason::InterruptedBeforeExecution,
                deadline(),
            )
            .expect("terminalize queued recovery without Working transition");

        assert_eq!(terminal.version, created.version + 1);
        assert_eq!(terminal.updated_at_epoch_ms, 7_500);
        assert!(matches!(
            terminal.task,
            V5StoredTask::Failed {
                terminal_epoch_ms: 7_500,
                reason: V5SafeFailureReason::Interrupted,
                ..
            }
        ));
        assert_eq!(
            store
                .terminalize_recovered_exact(
                    &created.identity(),
                    created.version,
                    RecoveryTerminalReason::InterruptedBeforeExecution,
                    deadline(),
                )
                .expect("repeat exact recovery is idempotent"),
            terminal
        );
    }

    #[test]
    fn recovered_begun_task_is_created_working_and_keeps_cancel_intent() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(7_600));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock, deadline()).unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("29292929-2929-4929-8929-292929292929"),
                    invocation_id("30303030-3030-4030-8030-303030303030"),
                    0x1c,
                )
                .for_recovered_begun(true),
                deadline(),
            )
            .expect("create exact recovered begun Task");

        assert_eq!(created.task, V5StoredTask::Working);
        assert!(created.cancel_requested);
        let terminal = store
            .terminalize_recovered_exact(
                &created.identity(),
                created.version,
                RecoveryTerminalReason::OutcomeUncertain,
                deadline(),
            )
            .expect("terminalize recovered begun Task as outcome uncertain");
        assert!(matches!(
            terminal.task,
            V5StoredTask::Failed {
                reason: V5SafeFailureReason::OutcomeUncertain,
                ..
            }
        ));
        assert!(terminal.cancel_requested);
    }

    #[test]
    fn failed_terminal_publication_requires_working_and_persists_exact_reason() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let (store, _) = FileInvocationStoreV5::open_inspect_only(
            &root_path,
            Arc::new(ManualEpochClock::at(8_000)),
            deadline(),
        )
        .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("20202020-2020-4020-8020-202020202020"),
                    invocation_id("21212121-2121-4121-8121-212121212121"),
                    0x0c,
                ),
                deadline(),
            )
            .unwrap();
        let identity = created.identity();
        store
            .start_working_if_not_cancel_requested(&identity, 1, deadline())
            .unwrap();

        let terminal = store
            .publish_terminal_exact(
                &identity,
                2,
                V5TerminalPublication::Failed {
                    terminal_epoch_ms: 8_100,
                    terminal_digest: terminal_digest(0xee),
                    reason: V5SafeFailureReason::OutcomeUncertain,
                },
                deadline(),
            )
            .unwrap();
        assert_eq!(terminal.version, 3);
        assert_eq!(
            terminal.task,
            V5StoredTask::Failed {
                terminal_epoch_ms: 8_100,
                terminal_digest: terminal_digest(0xee),
                reason: V5SafeFailureReason::OutcomeUncertain,
            }
        );
    }

    #[test]
    fn terminal_retirement_is_exact_explicit_and_reconciles_absence_after_uncertain_delete() {
        let root = tempfile::tempdir().expect("temporary v5 root");
        let root_path = physical_root(&root);
        let clock = Arc::new(ManualEpochClock::at(9_000));
        let (store, _) =
            FileInvocationStoreV5::open_inspect_only(&root_path, clock.clone(), deadline())
                .unwrap();
        let created = store
            .create_exact(
                new_record(
                    task_id("22222222-2222-4222-8222-222222222222"),
                    invocation_id("23232323-2323-4323-8323-232323232323"),
                    0x0d,
                ),
                deadline(),
            )
            .unwrap();
        let identity = created.identity();
        store
            .start_working_if_not_cancel_requested(&identity, 1, deadline())
            .unwrap();
        clock.set(9_100);
        let terminal = store
            .publish_terminal_exact(
                &identity,
                2,
                V5TerminalPublication::Failed {
                    terminal_epoch_ms: 9_100,
                    terminal_digest: terminal_digest(0xf0),
                    reason: V5SafeFailureReason::PersistenceFailed,
                },
                deadline(),
            )
            .unwrap();
        let retirement = V5TaskRetirement::from_terminal_record(&terminal)
            .expect("terminal record creates retirement proof");
        assert!(V5TaskRetirement::from_terminal_record(&created).is_none());

        assert!(matches!(
            store.delete_terminal_if_expired(
                &retirement,
                terminal.updated_at_epoch_ms + terminal.ttl_ms - 1,
                deadline(),
            ),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::State,
                ..
            })
        ));
        let mut stale_terminal = terminal.clone();
        stale_terminal.version = 2;
        let stale = V5TaskRetirement::from_terminal_record(&stale_terminal).unwrap();
        assert!(matches!(
            store.delete_terminal_if_expired(
                &stale,
                terminal.updated_at_epoch_ms + terminal.ttl_ms,
                deadline(),
            ),
            Err(V5TaskStoreError::Mismatch {
                reason: V5TaskMismatch::Version {
                    expected: 2,
                    actual: 3,
                },
                ..
            })
        ));

        store.inject_next_publication_failure(PublicationFailure::AfterDeleteBeforeSync);
        assert!(matches!(
            store.delete_terminal_if_expired(
                &retirement,
                terminal.updated_at_epoch_ms + terminal.ttl_ms,
                deadline(),
            ),
            Err(V5TaskStoreError::CommitUncertain {
                task_id: uncertain_id,
                operation: V5CommitOperation::DeleteTerminal,
            }) if uncertain_id == created.task_id
        ));
        assert!(matches!(
            store.get(created.task_id, deadline()),
            Err(V5TaskStoreError::NotFound { .. })
        ));
        assert_eq!(
            store
                .delete_terminal_if_expired(
                    &retirement,
                    terminal.updated_at_epoch_ms + terminal.ttl_ms,
                    deadline(),
                )
                .unwrap(),
            V5DeleteTerminalOutcome::AlreadyAbsent(retirement)
        );
    }
}
