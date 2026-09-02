//! Durable sole-record lifecycle-link pool for protocol-v5 Tasks.

use crate::application::receipt_ledger::{
    receipt_key_digest, AttemptPhase, ClosedTerminalStatus, LifecycleLinkRecordHeader, ReceiptKey,
    ReceiptKeyDigest, ReceiptLedgerError, ReceiptTaskProjection, RetainedDualIdAccounting,
    TaskBoundReceipt, TaskLinkDigest, TaskLinkReference, TaskRetirementPendingReceipt,
    TaskTerminalBoundReceipt, TerminalDigest,
    MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES as APPLICATION_MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
};
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::invocation::{InvocationId, SafeIdentityHash, TaskId};
use crate::infrastructure::platform::filesystem::{
    create_new_regular_child, file_identity, open_directory_ownership_lock,
    open_regular_child_nofollow, read_directory_names_bounded, remove_identity_bound_regular_child,
    rename_identity_bound_regular_child_no_replace, replace_identity_bound_regular_child,
    restrict_stage_to_owner, sync_directory, verify_owner_only_acl, RetainedDirectoryCapability,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;
use uuid::Uuid;

pub(crate) const MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES: usize =
    APPLICATION_MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES as usize;
pub(crate) const MAX_TASK_LIFECYCLE_LINK_RECORDS: usize = 4_096;
pub(crate) const MAX_TASK_LIFECYCLE_LINK_POOL_BYTES: usize = 4 * 1_024 * 1_024;

const STORE_SCHEMA_VERSION: u32 = 1;
const STORE_LOCK_FILE: &str = ".task-lifecycle-link-v5.lock";
const STORE_SNAPSHOT_FILE: &str = "task-lifecycle-links-v1.json";
const STORE_STAGING_PREFIX: &str = ".task-lifecycle-links-v1.";
const STORE_STAGING_SUFFIX: &str = ".tmp";
const STORE_WRITER_WAIT_SLICE: Duration = Duration::from_millis(10);
const MAX_DIRECTORY_ENTRIES: usize = 66;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskLifecycleLinkStoreError {
    AlreadyOwned,
    DeadlineExceeded,
    NotFound {
        task_id: TaskId,
    },
    AlreadyMaterialized {
        task_id: TaskId,
    },
    IdentityMismatch,
    ReservationMismatch,
    StateMismatch,
    VersionMismatch {
        expected: u64,
        actual: u64,
    },
    Capacity {
        maximum_records: usize,
        maximum_bytes: usize,
    },
    RecordTooLarge {
        actual: usize,
        maximum: usize,
    },
    CommitUncertain {
        receipt_key_digest: ReceiptKeyDigest,
    },
    Corrupt(&'static str),
    Storage {
        operation: &'static str,
        message: String,
    },
}

impl fmt::Display for TaskLifecycleLinkStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyOwned => formatter.write_str("Task lifecycle-link store is already owned"),
            Self::DeadlineExceeded => {
                formatter.write_str("Task lifecycle-link store deadline expired")
            }
            Self::NotFound { task_id } => write!(formatter, "Task lifecycle link {task_id} was not found"),
            Self::AlreadyMaterialized { task_id } => {
                write!(formatter, "Task lifecycle link {task_id} is already materialized")
            }
            Self::IdentityMismatch => {
                formatter.write_str("Task lifecycle-link identity belongs to another exact key")
            }
            Self::ReservationMismatch => {
                formatter.write_str("Task lifecycle-link reservation does not match durable state")
            }
            Self::StateMismatch => formatter.write_str("Task lifecycle-link state mismatch"),
            Self::VersionMismatch { expected, actual } => write!(
                formatter,
                "Task lifecycle-link version mismatch: expected {expected}, actual {actual}"
            ),
            Self::Capacity {
                maximum_records,
                maximum_bytes,
            } => write!(
                formatter,
                "Task lifecycle-link capacity is exhausted ({maximum_records} records, {maximum_bytes} bytes)"
            ),
            Self::RecordTooLarge { actual, maximum } => write!(
                formatter,
                "Task lifecycle-link record uses {actual} bytes, maximum is {maximum}"
            ),
            Self::CommitUncertain { receipt_key_digest } => write!(
                formatter,
                "Task lifecycle-link commit is uncertain for {receipt_key_digest}"
            ),
            Self::Corrupt(message) => write!(formatter, "corrupt Task lifecycle-link store: {message}"),
            Self::Storage { operation, message } => write!(formatter, "{operation}: {message}"),
        }
    }
}

impl std::error::Error for TaskLifecycleLinkStoreError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskLinkReservation {
    key: ReceiptKey,
    key_digest: ReceiptKeyDigest,
    link: TaskLinkReference,
    reservation_version: u64,
    mutation_sequence: u64,
    encoded_bytes: u64,
}

impl TaskLinkReservation {
    pub(crate) fn key(&self) -> &ReceiptKey {
        &self.key
    }

    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        &self.key_digest
    }

    pub(crate) fn link(&self) -> &TaskLinkReference {
        &self.link
    }

    pub(crate) const fn reservation_version(&self) -> u64 {
        self.reservation_version
    }

    pub(crate) const fn mutation_sequence(&self) -> u64 {
        self.mutation_sequence
    }

    pub(crate) const fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)] // The variants preserve the normative lifecycle state names.
pub(crate) enum TaskLifecycleLinkRecord {
    TaskBound(TaskBoundReceipt),
    TaskTerminalBound(TaskTerminalBoundReceipt),
    TaskRetirementPending(TaskRetirementPendingReceipt),
}

impl TaskLifecycleLinkRecord {
    pub(crate) fn key(&self) -> &ReceiptKey {
        match self {
            Self::TaskBound(record) => record.key(),
            Self::TaskTerminalBound(record) => record.key(),
            Self::TaskRetirementPending(record) => record.key(),
        }
    }

    fn key_digest(&self) -> &ReceiptKeyDigest {
        match self {
            Self::TaskBound(record) => record.key_digest(),
            Self::TaskTerminalBound(record) => record.key_digest(),
            Self::TaskRetirementPending(record) => record.key_digest(),
        }
    }

    fn link(&self) -> &TaskLinkReference {
        match self {
            Self::TaskBound(record) => record.link(),
            Self::TaskTerminalBound(record) => record.link(),
            Self::TaskRetirementPending(record) => record.link(),
        }
    }

    fn lifecycle_link_version(&self) -> u64 {
        match self {
            Self::TaskBound(record) => record.lifecycle_link_version(),
            Self::TaskTerminalBound(record) => record.lifecycle_link_version(),
            Self::TaskRetirementPending(record) => record.lifecycle_link_version(),
        }
    }

    fn mutation_sequence(&self) -> u64 {
        match self {
            Self::TaskBound(record) => record.mutation_sequence(),
            Self::TaskTerminalBound(record) => record.mutation_sequence(),
            Self::TaskRetirementPending(record) => record.mutation_sequence(),
        }
    }

    fn encoded_bytes(&self) -> u64 {
        match self {
            Self::TaskBound(record) => record.encoded_bytes(),
            Self::TaskTerminalBound(record) => record.encoded_bytes(),
            Self::TaskRetirementPending(record) => record.encoded_bytes(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskLifecycleLinkCatalogEntry {
    Reservation(TaskLinkReservation),
    Record(TaskLifecycleLinkRecord),
}

impl TaskLifecycleLinkCatalogEntry {
    pub(crate) fn key_digest(&self) -> &ReceiptKeyDigest {
        match self {
            Self::Reservation(reservation) => reservation.key_digest(),
            Self::Record(record) => record.key_digest(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskLifecycleLinkCatalogSnapshot {
    generation: u64,
    entries: Vec<TaskLifecycleLinkCatalogEntry>,
    count: usize,
    actual_bytes: u64,
    reserved_count: usize,
    reserved_bytes: u64,
}

impl TaskLifecycleLinkCatalogSnapshot {
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn entries(&self) -> &[TaskLifecycleLinkCatalogEntry] {
        &self.entries
    }

    pub(crate) const fn count(&self) -> usize {
        self.count
    }

    pub(crate) const fn actual_bytes(&self) -> u64 {
        self.actual_bytes
    }

    pub(crate) const fn reserved_count(&self) -> usize {
        self.reserved_count
    }

    pub(crate) const fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TaskLifecycleLinkCapacitySnapshot {
    live_reservations: usize,
    materialized_links: usize,
    accounted_bytes: usize,
}

impl TaskLifecycleLinkCapacitySnapshot {
    pub(crate) const fn live_reservations(self) -> usize {
        self.live_reservations
    }

    pub(crate) const fn materialized_links(self) -> usize {
        self.materialized_links
    }

    pub(crate) const fn task_store_slots_accounted(self) -> usize {
        self.live_reservations + self.materialized_links
    }

    pub(crate) const fn accounted_bytes(self) -> usize {
        self.accounted_bytes
    }
}

#[derive(Debug, Clone, Copy)]
struct StoreLimits {
    max_records: usize,
    max_pool_bytes: usize,
    max_record_bytes: usize,
}

impl StoreLimits {
    const fn production() -> Self {
        Self {
            max_records: MAX_TASK_LIFECYCLE_LINK_RECORDS,
            max_pool_bytes: MAX_TASK_LIFECYCLE_LINK_POOL_BYTES,
            max_record_bytes: MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CatalogEntry {
    Reservation(TaskLinkReservation),
    Link(TaskLifecycleLinkRecord),
}

impl CatalogEntry {
    fn key(&self) -> &ReceiptKey {
        match self {
            Self::Reservation(reservation) => reservation.key(),
            Self::Link(record) => record.key(),
        }
    }

    fn key_digest(&self) -> &ReceiptKeyDigest {
        match self {
            Self::Reservation(reservation) => reservation.key_digest(),
            Self::Link(record) => record.key_digest(),
        }
    }

    fn link(&self) -> &TaskLinkReference {
        match self {
            Self::Reservation(reservation) => reservation.link(),
            Self::Link(record) => record.link(),
        }
    }

    fn mutation_sequence(&self) -> u64 {
        match self {
            Self::Reservation(reservation) => reservation.mutation_sequence(),
            Self::Link(record) => record.mutation_sequence(),
        }
    }

    fn accounted_bytes(&self, maximum_record_bytes: usize) -> usize {
        match self {
            Self::Reservation(_) => maximum_record_bytes,
            Self::Link(record) => usize::try_from(record.encoded_bytes()).unwrap_or(usize::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StoreCatalog {
    mutation_sequence: u64,
    entries: HashMap<ReceiptKeyDigest, CatalogEntry>,
    task_index: HashMap<TaskId, ReceiptKeyDigest>,
    invocation_index: HashMap<InvocationId, ReceiptKeyDigest>,
}

impl StoreCatalog {
    fn capacity_snapshot(&self, maximum_record_bytes: usize) -> TaskLifecycleLinkCapacitySnapshot {
        let mut live_reservations = 0;
        let mut materialized_links = 0;
        let mut accounted_bytes = 0usize;
        for entry in self.entries.values() {
            match entry {
                CatalogEntry::Reservation(_) => live_reservations += 1,
                CatalogEntry::Link(_) => materialized_links += 1,
            }
            accounted_bytes =
                accounted_bytes.saturating_add(entry.accounted_bytes(maximum_record_bytes));
        }
        TaskLifecycleLinkCapacitySnapshot {
            live_reservations,
            materialized_links,
            accounted_bytes,
        }
    }

    fn insert_exact(&mut self, entry: CatalogEntry) -> Result<(), TaskLifecycleLinkStoreError> {
        let key_digest = entry.key_digest().clone();
        let task_id = entry.key().reserved_task_id();
        let invocation_id = entry.key().invocation_id();
        if receipt_key_digest(entry.key()) != key_digest
            || entry.link().receipt_key_digest() != &key_digest
            || entry.link().task_id() != task_id
            || entry.link().invocation_id() != invocation_id
        {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "catalog entry has contradictory key or link identity",
            ));
        }
        if self
            .task_index
            .get(&task_id)
            .is_some_and(|existing| existing != &key_digest)
            || self
                .invocation_index
                .get(&invocation_id)
                .is_some_and(|existing| existing != &key_digest)
        {
            return Err(TaskLifecycleLinkStoreError::IdentityMismatch);
        }
        self.task_index.insert(task_id, key_digest.clone());
        self.invocation_index
            .insert(invocation_id, key_digest.clone());
        self.entries.insert(key_digest, entry);
        Ok(())
    }

    fn remove_exact_pending(
        &mut self,
        expected: &TaskRetirementPendingReceipt,
    ) -> Result<(), TaskLifecycleLinkStoreError> {
        let key_digest = expected.key_digest();
        let current =
            self.entries
                .get(key_digest)
                .ok_or(TaskLifecycleLinkStoreError::NotFound {
                    task_id: expected.task().task_id(),
                })?;
        if current
            != &CatalogEntry::Link(TaskLifecycleLinkRecord::TaskRetirementPending(
                expected.clone(),
            ))
        {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        if self.task_index.get(&expected.task().task_id()) != Some(key_digest)
            || self.invocation_index.get(&expected.task().invocation_id()) != Some(key_digest)
        {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "TaskRetirementPending indexes do not bind its exact identity",
            ));
        }
        self.entries.remove(key_digest);
        self.task_index.remove(&expected.task().task_id());
        self.invocation_index
            .remove(&expected.task().invocation_id());
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredTaskProjectionV1 {
    created_at_epoch_ms: u64,
    updated_at_epoch_ms: u64,
    ttl_ms: u64,
    poll_interval_ms: u64,
    version: u64,
}

impl StoredTaskProjectionV1 {
    fn from_application(task: &ReceiptTaskProjection) -> Self {
        Self {
            created_at_epoch_ms: task.created_at_epoch_ms(),
            updated_at_epoch_ms: task.updated_at_epoch_ms(),
            ttl_ms: task.ttl_ms(),
            poll_interval_ms: task.poll_interval_ms(),
            version: task.version(),
        }
    }

    fn into_application(
        self,
        task_id: TaskId,
        invocation_id: InvocationId,
    ) -> Result<ReceiptTaskProjection, TaskLifecycleLinkStoreError> {
        ReceiptTaskProjection::new(
            task_id,
            invocation_id,
            self.created_at_epoch_ms,
            self.updated_at_epoch_ms,
            self.ttl_ms,
            self.poll_interval_ms,
            self.version,
        )
        .map_err(application_corruption)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredTaskLinkReferenceV1 {
    workspace_identity_hash: SafeIdentityHash,
    task_link_digest: TaskLinkDigest,
}

impl StoredTaskLinkReferenceV1 {
    fn from_application(link: &TaskLinkReference) -> Self {
        Self {
            workspace_identity_hash: link.workspace_identity_hash().clone(),
            task_link_digest: link.digest().clone(),
        }
    }

    fn into_application(
        self,
        receipt_key_digest: ReceiptKeyDigest,
        task_id: TaskId,
        invocation_id: InvocationId,
    ) -> Result<TaskLinkReference, TaskLifecycleLinkStoreError> {
        let link = TaskLinkReference::new(
            receipt_key_digest,
            task_id,
            invocation_id,
            self.workspace_identity_hash,
        );
        if link.digest() != &self.task_link_digest {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "persisted lifecycle-link digest does not match its exact identity",
            ));
        }
        Ok(link)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredAttemptPhaseV1 {
    NotBegun,
    Begun,
}

impl From<AttemptPhase> for StoredAttemptPhaseV1 {
    fn from(value: AttemptPhase) -> Self {
        match value {
            AttemptPhase::NotBegun => Self::NotBegun,
            AttemptPhase::Begun => Self::Begun,
        }
    }
}

impl From<StoredAttemptPhaseV1> for AttemptPhase {
    fn from(value: StoredAttemptPhaseV1) -> Self {
        match value {
            StoredAttemptPhaseV1::NotBegun => Self::NotBegun,
            StoredAttemptPhaseV1::Begun => Self::Begun,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StoredClosedTerminalStatusV1 {
    Completed,
    Failed,
    Cancelled,
}

impl From<ClosedTerminalStatus> for StoredClosedTerminalStatusV1 {
    fn from(value: ClosedTerminalStatus) -> Self {
        match value {
            ClosedTerminalStatus::Completed => Self::Completed,
            ClosedTerminalStatus::Failed => Self::Failed,
            ClosedTerminalStatus::Cancelled => Self::Cancelled,
        }
    }
}

impl From<StoredClosedTerminalStatusV1> for ClosedTerminalStatus {
    fn from(value: StoredClosedTerminalStatusV1) -> Self {
        match value {
            StoredClosedTerminalStatusV1::Completed => Self::Completed,
            StoredClosedTerminalStatusV1::Failed => Self::Failed,
            StoredClosedTerminalStatusV1::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum StoredEntryV1 {
    Reservation {
        schema_version: u32,
        key: ReceiptKey,
        link: StoredTaskLinkReferenceV1,
        reservation_version: u64,
        mutation_sequence: u64,
    },
    TaskBound {
        schema_version: u32,
        key: ReceiptKey,
        link: StoredTaskLinkReferenceV1,
        task: StoredTaskProjectionV1,
        lifecycle_link_version: u64,
        mutation_sequence: u64,
        task_record_version: u64,
        bind_epoch_ms: u64,
        phase: StoredAttemptPhaseV1,
    },
    TaskTerminalBound {
        schema_version: u32,
        key: ReceiptKey,
        link: StoredTaskLinkReferenceV1,
        task: StoredTaskProjectionV1,
        lifecycle_link_version: u64,
        mutation_sequence: u64,
        task_record_version: u64,
        terminal_status: StoredClosedTerminalStatusV1,
        terminal_digest: TerminalDigest,
        terminal_epoch_ms: u64,
        expires_at_epoch_ms: u64,
    },
    TaskRetirementPending {
        schema_version: u32,
        key: ReceiptKey,
        link: StoredTaskLinkReferenceV1,
        task: StoredTaskProjectionV1,
        lifecycle_link_version: u64,
        mutation_sequence: u64,
        expected_terminal_task_version: u64,
        terminal_status: StoredClosedTerminalStatusV1,
        terminal_digest: TerminalDigest,
        terminal_epoch_ms: u64,
        expires_at_epoch_ms: u64,
        #[serde(rename = "l")]
        retained_link_bytes: u64,
        #[serde(rename = "i")]
        invocation_index_bytes: u64,
        #[serde(rename = "r")]
        reserved_task_index_bytes: u64,
    },
}

impl StoredEntryV1 {
    fn from_catalog_entry(entry: &CatalogEntry) -> Self {
        match entry {
            CatalogEntry::Reservation(reservation) => Self::Reservation {
                schema_version: STORE_SCHEMA_VERSION,
                key: reservation.key().clone(),
                link: StoredTaskLinkReferenceV1::from_application(reservation.link()),
                reservation_version: reservation.reservation_version(),
                mutation_sequence: reservation.mutation_sequence(),
            },
            CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(record)) => Self::TaskBound {
                schema_version: STORE_SCHEMA_VERSION,
                key: record.key().clone(),
                link: StoredTaskLinkReferenceV1::from_application(record.link()),
                task: StoredTaskProjectionV1::from_application(record.task()),
                lifecycle_link_version: record.lifecycle_link_version(),
                mutation_sequence: record.mutation_sequence(),
                task_record_version: record.task_record_version(),
                bind_epoch_ms: record.bind_epoch_ms(),
                phase: record.phase().into(),
            },
            CatalogEntry::Link(TaskLifecycleLinkRecord::TaskTerminalBound(record)) => {
                Self::TaskTerminalBound {
                    schema_version: STORE_SCHEMA_VERSION,
                    key: record.key().clone(),
                    link: StoredTaskLinkReferenceV1::from_application(record.link()),
                    task: StoredTaskProjectionV1::from_application(record.task()),
                    lifecycle_link_version: record.lifecycle_link_version(),
                    mutation_sequence: record.mutation_sequence(),
                    task_record_version: record.task_record_version(),
                    terminal_status: record.terminal_status().into(),
                    terminal_digest: record.terminal_digest().clone(),
                    terminal_epoch_ms: record.terminal_epoch_ms(),
                    expires_at_epoch_ms: record.expires_at_epoch_ms(),
                }
            }
            CatalogEntry::Link(TaskLifecycleLinkRecord::TaskRetirementPending(record)) => {
                Self::TaskRetirementPending {
                    schema_version: STORE_SCHEMA_VERSION,
                    key: record.key().clone(),
                    link: StoredTaskLinkReferenceV1::from_application(record.link()),
                    task: StoredTaskProjectionV1::from_application(record.task()),
                    lifecycle_link_version: record.lifecycle_link_version(),
                    mutation_sequence: record.mutation_sequence(),
                    expected_terminal_task_version: record.expected_terminal_task_version(),
                    terminal_status: record.terminal_status().into(),
                    terminal_digest: record.terminal_digest().clone(),
                    terminal_epoch_ms: record.terminal_epoch_ms(),
                    expires_at_epoch_ms: record.expires_at_epoch_ms(),
                    retained_link_bytes: record.retained_link_bytes(),
                    invocation_index_bytes: record
                        .retained_dual_id_accounting()
                        .invocation_index_bytes(),
                    reserved_task_index_bytes: record
                        .retained_dual_id_accounting()
                        .reserved_task_index_bytes(),
                }
            }
        }
    }

    fn encoded_len(&self) -> Result<usize, TaskLifecycleLinkStoreError> {
        serde_json::to_vec(self)
            .map(|encoded| encoded.len())
            .map_err(|error| storage_message("serialize Task lifecycle-link record", error))
    }

    fn into_catalog_entry(self) -> Result<CatalogEntry, TaskLifecycleLinkStoreError> {
        let encoded_bytes = u64::try_from(self.encoded_len()?).map_err(|_| {
            TaskLifecycleLinkStoreError::Corrupt("lifecycle-link record length does not fit u64")
        })?;
        match self {
            Self::Reservation {
                schema_version,
                key,
                link,
                reservation_version,
                mutation_sequence,
            } => {
                require_schema_v1(schema_version)?;
                let key_digest = receipt_key_digest(&key);
                let link = link.into_application(
                    key_digest.clone(),
                    key.reserved_task_id(),
                    key.invocation_id(),
                )?;
                validate_exact_identity(&key, &key_digest, &link)?;
                if reservation_version == 0 || mutation_sequence == 0 {
                    return Err(TaskLifecycleLinkStoreError::Corrupt(
                        "lifecycle-link reservation has a zero version",
                    ));
                }
                Ok(CatalogEntry::Reservation(TaskLinkReservation {
                    key,
                    key_digest,
                    link,
                    reservation_version,
                    mutation_sequence,
                    encoded_bytes,
                }))
            }
            Self::TaskBound {
                schema_version,
                key,
                link,
                task,
                lifecycle_link_version,
                mutation_sequence,
                task_record_version,
                bind_epoch_ms,
                phase,
            } => {
                require_schema_v1(schema_version)?;
                let key_digest = receipt_key_digest(&key);
                let link = link.into_application(
                    key_digest.clone(),
                    key.reserved_task_id(),
                    key.invocation_id(),
                )?;
                validate_exact_identity(&key, &key_digest, &link)?;
                let task_id = key.reserved_task_id();
                let invocation_id = key.invocation_id();
                let header = LifecycleLinkRecordHeader::new(
                    key,
                    link,
                    lifecycle_link_version,
                    mutation_sequence,
                    encoded_bytes,
                )
                .map_err(application_corruption)?;
                let record = TaskBoundReceipt::new(
                    header,
                    task.into_application(task_id, invocation_id)?,
                    task_record_version,
                    bind_epoch_ms,
                    phase.into(),
                )
                .map_err(application_corruption)?;
                Ok(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(
                    record,
                )))
            }
            Self::TaskTerminalBound {
                schema_version,
                key,
                link,
                task,
                lifecycle_link_version,
                mutation_sequence,
                task_record_version,
                terminal_status,
                terminal_digest,
                terminal_epoch_ms,
                expires_at_epoch_ms,
            } => {
                require_schema_v1(schema_version)?;
                let key_digest = receipt_key_digest(&key);
                let link = link.into_application(
                    key_digest.clone(),
                    key.reserved_task_id(),
                    key.invocation_id(),
                )?;
                validate_exact_identity(&key, &key_digest, &link)?;
                let task_id = key.reserved_task_id();
                let invocation_id = key.invocation_id();
                let header = LifecycleLinkRecordHeader::new(
                    key,
                    link,
                    lifecycle_link_version,
                    mutation_sequence,
                    encoded_bytes,
                )
                .map_err(application_corruption)?;
                let record = TaskTerminalBoundReceipt::new(
                    header,
                    task.into_application(task_id, invocation_id)?,
                    task_record_version,
                    terminal_status.into(),
                    terminal_digest,
                    terminal_epoch_ms,
                    expires_at_epoch_ms,
                )
                .map_err(application_corruption)?;
                Ok(CatalogEntry::Link(
                    TaskLifecycleLinkRecord::TaskTerminalBound(record),
                ))
            }
            Self::TaskRetirementPending {
                schema_version,
                key,
                link,
                task,
                lifecycle_link_version,
                mutation_sequence,
                expected_terminal_task_version,
                terminal_status,
                terminal_digest,
                terminal_epoch_ms,
                expires_at_epoch_ms,
                retained_link_bytes,
                invocation_index_bytes,
                reserved_task_index_bytes,
            } => {
                require_schema_v1(schema_version)?;
                let key_digest = receipt_key_digest(&key);
                let link = link.into_application(
                    key_digest.clone(),
                    key.reserved_task_id(),
                    key.invocation_id(),
                )?;
                validate_exact_identity(&key, &key_digest, &link)?;
                let task_id = key.reserved_task_id();
                let invocation_id = key.invocation_id();
                let header = LifecycleLinkRecordHeader::new(
                    key,
                    link,
                    lifecycle_link_version,
                    mutation_sequence,
                    encoded_bytes,
                )
                .map_err(application_corruption)?;
                let accounting = RetainedDualIdAccounting::new(
                    invocation_index_bytes,
                    reserved_task_index_bytes,
                )
                .map_err(application_corruption)?;
                let record = TaskRetirementPendingReceipt::new(
                    header,
                    task.into_application(task_id, invocation_id)?,
                    expected_terminal_task_version,
                    terminal_status.into(),
                    terminal_digest,
                    terminal_epoch_ms,
                    expires_at_epoch_ms,
                    retained_link_bytes,
                    accounting,
                )
                .map_err(application_corruption)?;
                Ok(CatalogEntry::Link(
                    TaskLifecycleLinkRecord::TaskRetirementPending(record),
                ))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCatalogV1 {
    schema_version: u32,
    mutation_sequence: u64,
    entries: Vec<StoredEntryV1>,
}

pub(crate) struct TaskLifecycleLinkStoreV5 {
    root: RetainedDirectoryCapability,
    root_file: File,
    _ownership_lock: File,
    writer: Mutex<StoreCatalog>,
    limits: StoreLimits,
}

impl TaskLifecycleLinkStoreV5 {
    #[cfg(feature = "receipt-ledger-test-support")]
    pub(crate) fn seed_task_terminal_bounds_bulk_for_test(
        &self,
        records: Vec<(
            ReceiptKey,
            TaskLinkReference,
            ReceiptTaskProjection,
            u64,
            u64,
            TerminalDigest,
        )>,
        deadline: ProviderDeadline,
    ) -> Result<Vec<TaskTerminalBoundReceipt>, TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        if !writer.entries.is_empty() || records.len() > self.limits.max_records {
            return Err(self.capacity_error());
        }
        self.verify_root_authority()?;
        let mut next = writer.clone();
        let mut first_digest = None;
        let mut seeded = Vec::with_capacity(records.len());
        for (key, link, task, task_record_version, terminal_epoch_ms, terminal_digest) in records {
            let mutation_sequence = next_sequence(next.mutation_sequence)?;
            let key_digest = receipt_key_digest(&key);
            first_digest.get_or_insert_with(|| key_digest.clone());
            let expires_at_epoch_ms = terminal_epoch_ms.checked_add(task.ttl_ms()).ok_or(
                TaskLifecycleLinkStoreError::Corrupt("bulk terminal fixture expiry exceeds u64"),
            )?;
            let record = build_task_terminal_bound(
                key,
                link,
                task,
                3,
                mutation_sequence,
                task_record_version,
                ClosedTerminalStatus::Completed,
                terminal_digest,
                terminal_epoch_ms,
                expires_at_epoch_ms,
                self.limits,
            )?;
            seeded.push(record.clone());
            next.mutation_sequence = mutation_sequence;
            next.insert_exact(CatalogEntry::Link(
                TaskLifecycleLinkRecord::TaskTerminalBound(record),
            ))?;
        }
        validate_capacity(&next, self.limits)?;
        let Some(first_digest) = first_digest else {
            return Ok(Vec::new());
        };
        let committed = self.publish_and_readback(&next, &first_digest, deadline)?;
        *writer = committed;
        Ok(seeded)
    }

    pub(crate) fn open(
        root: impl AsRef<Path>,
        deadline: ProviderDeadline,
    ) -> Result<Self, TaskLifecycleLinkStoreError> {
        Self::open_with_limits(root, StoreLimits::production(), deadline)
    }

    #[cfg(test)]
    fn open_with_limits_for_test(
        root: impl AsRef<Path>,
        max_records: usize,
        max_pool_bytes: usize,
        deadline: ProviderDeadline,
    ) -> Result<Self, TaskLifecycleLinkStoreError> {
        Self::open_with_limits(
            root,
            StoreLimits {
                max_records,
                max_pool_bytes,
                max_record_bytes: MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES,
            },
            deadline,
        )
    }

    fn open_with_limits(
        root: impl AsRef<Path>,
        limits: StoreLimits,
        deadline: ProviderDeadline,
    ) -> Result<Self, TaskLifecycleLinkStoreError> {
        check_deadline(deadline)?;
        let root = RetainedDirectoryCapability::open(root.as_ref())
            .map_err(|error| storage_error("retain Task lifecycle-link root", error))?;
        root.validate_named_identity()
            .map_err(|error| storage_error("validate Task lifecycle-link root", error))?;
        let root_file = root
            .try_clone_directory()
            .map_err(|error| storage_error("clone Task lifecycle-link root", error))?;
        verify_owner_only_acl(&root_file)
            .map_err(|error| storage_error("verify Task lifecycle-link root ownership", error))?;
        let ownership_lock = open_directory_ownership_lock(&root_file, OsStr::new(STORE_LOCK_FILE))
            .map_err(|error| storage_error("open Task lifecycle-link ownership object", error))?;
        verify_owner_only_acl(&ownership_lock)
            .map_err(|error| storage_error("verify Task lifecycle-link ownership object", error))?;
        match FileExt::try_lock_exclusive(&ownership_lock) {
            Ok(()) => {}
            Err(error) if lock_is_contended(&error) => {
                return Err(TaskLifecycleLinkStoreError::AlreadyOwned)
            }
            Err(error) => {
                return Err(storage_error(
                    "acquire Task lifecycle-link ownership lock",
                    error,
                ))
            }
        }
        let mut store = Self {
            root,
            root_file,
            _ownership_lock: ownership_lock,
            writer: Mutex::new(StoreCatalog::default()),
            limits,
        };
        let catalog = store.inspect_and_recover(deadline)?;
        store.writer = Mutex::new(catalog);
        Ok(store)
    }

    pub(crate) fn capacity_snapshot(&self) -> TaskLifecycleLinkCapacitySnapshot {
        self.writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .capacity_snapshot(self.limits.max_record_bytes)
    }

    pub(crate) fn catalog_snapshot(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<TaskLifecycleLinkCatalogSnapshot, TaskLifecycleLinkStoreError> {
        check_deadline(deadline)?;
        let writer = self.lock_writer(deadline)?;
        self.verify_root_authority()?;
        check_deadline(deadline)?;

        let maximum_record_bytes = u64::try_from(self.limits.max_record_bytes).map_err(|_| {
            TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link record limit does not fit u64",
            )
        })?;
        let mut entries = Vec::with_capacity(writer.entries.len());
        let mut actual_bytes = 0u64;
        let mut reserved_count = 0usize;
        let mut reserved_bytes = 0u64;
        for entry in writer.entries.values() {
            match entry {
                CatalogEntry::Reservation(reservation) => {
                    reserved_count = reserved_count.checked_add(1).ok_or(
                        TaskLifecycleLinkStoreError::Corrupt(
                            "Task lifecycle-link reservation count overflowed",
                        ),
                    )?;
                    reserved_bytes = reserved_bytes.checked_add(maximum_record_bytes).ok_or(
                        TaskLifecycleLinkStoreError::Corrupt(
                            "Task lifecycle-link reserved byte count overflowed",
                        ),
                    )?;
                    entries.push(TaskLifecycleLinkCatalogEntry::Reservation(
                        reservation.clone(),
                    ));
                }
                CatalogEntry::Link(record) => {
                    actual_bytes = actual_bytes.checked_add(record.encoded_bytes()).ok_or(
                        TaskLifecycleLinkStoreError::Corrupt(
                            "Task lifecycle-link actual byte count overflowed",
                        ),
                    )?;
                    entries.push(TaskLifecycleLinkCatalogEntry::Record(record.clone()));
                }
            }
        }
        entries.sort_by(|left, right| left.key_digest().as_str().cmp(right.key_digest().as_str()));
        let count = entries.len();
        Ok(TaskLifecycleLinkCatalogSnapshot {
            generation: writer.mutation_sequence,
            entries,
            count,
            actual_bytes,
            reserved_count,
            reserved_bytes,
        })
    }

    pub(crate) fn reserve_task_link(
        &self,
        key: ReceiptKey,
        link: TaskLinkReference,
        deadline: ProviderDeadline,
    ) -> Result<TaskLinkReservation, TaskLifecycleLinkStoreError> {
        validate_exact_identity(&key, &receipt_key_digest(&key), &link)?;
        let mut writer = self.lock_writer(deadline)?;
        self.verify_root_authority()?;
        let key_digest = receipt_key_digest(&key);
        if let Some(existing) = writer.entries.get(&key_digest) {
            return match existing {
                CatalogEntry::Reservation(reservation)
                    if reservation.key() == &key && reservation.link() == &link =>
                {
                    Ok(reservation.clone())
                }
                CatalogEntry::Reservation(_) => Err(TaskLifecycleLinkStoreError::IdentityMismatch),
                CatalogEntry::Link(record) if record.key() == &key && record.link() == &link => {
                    Err(TaskLifecycleLinkStoreError::AlreadyMaterialized {
                        task_id: key.reserved_task_id(),
                    })
                }
                CatalogEntry::Link(_) => Err(TaskLifecycleLinkStoreError::IdentityMismatch),
            };
        }
        if writer
            .task_index
            .get(&key.reserved_task_id())
            .is_some_and(|digest| digest != &key_digest)
            || writer
                .invocation_index
                .get(&key.invocation_id())
                .is_some_and(|digest| digest != &key_digest)
        {
            return Err(TaskLifecycleLinkStoreError::IdentityMismatch);
        }
        let capacity = writer.capacity_snapshot(self.limits.max_record_bytes);
        if capacity.task_store_slots_accounted() >= self.limits.max_records
            || capacity
                .accounted_bytes()
                .checked_add(self.limits.max_record_bytes)
                .is_none_or(|bytes| bytes > self.limits.max_pool_bytes)
        {
            return Err(self.capacity_error());
        }
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let reservation = build_reservation(key, link, mutation_sequence, self.limits)?;
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Reservation(reservation.clone()))?;
        let committed = self.publish_and_readback(&next, &key_digest, deadline)?;
        *writer = committed;
        match writer.entries.get(&key_digest) {
            Some(CatalogEntry::Reservation(readback)) if readback == &reservation => {
                Ok(readback.clone())
            }
            _ => Err(TaskLifecycleLinkStoreError::Corrupt(
                "durable lifecycle-link reservation readback changed",
            )),
        }
    }

    pub(crate) fn materialize_task_bound(
        &self,
        reservation: &TaskLinkReservation,
        task: ReceiptTaskProjection,
        task_record_version: u64,
        bind_epoch_ms: u64,
        phase: AttemptPhase,
        deadline: ProviderDeadline,
    ) -> Result<TaskBoundReceipt, TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let current = writer
            .entries
            .get(reservation.key_digest())
            .ok_or(TaskLifecycleLinkStoreError::ReservationMismatch)?;
        if current != &CatalogEntry::Reservation(reservation.clone()) {
            return Err(TaskLifecycleLinkStoreError::ReservationMismatch);
        }
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let record = build_task_bound(
            reservation.key().clone(),
            reservation.link().clone(),
            task,
            next_sequence(reservation.reservation_version())?,
            mutation_sequence,
            task_record_version,
            bind_epoch_ms,
            phase,
            self.limits,
        )?;
        let result = record.clone();
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(
            record,
        )))?;
        let committed = self.publish_and_readback(&next, reservation.key_digest(), deadline)?;
        *writer = committed;
        match writer.entries.get(reservation.key_digest()) {
            Some(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(readback)))
                if readback == &result =>
            {
                Ok(readback.clone())
            }
            _ => Err(TaskLifecycleLinkStoreError::Corrupt(
                "durable TaskBound readback changed",
            )),
        }
    }

    pub(crate) fn mark_task_bound_begun(
        &self,
        expected: &TaskBoundReceipt,
        task_record_version: u64,
        updated_at_epoch_ms: u64,
        deadline: ProviderDeadline,
    ) -> Result<TaskBoundReceipt, TaskLifecycleLinkStoreError> {
        if expected.phase() != AttemptPhase::NotBegun {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        let mut writer = self.lock_writer(deadline)?;
        let current = exact_task_bound(
            &writer,
            expected.key_digest(),
            expected.lifecycle_link_version(),
        )?;
        if current != expected {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        let task = ReceiptTaskProjection::new(
            expected.task().task_id(),
            expected.task().invocation_id(),
            expected.task().created_at_epoch_ms(),
            updated_at_epoch_ms,
            expected.task().ttl_ms(),
            expected.task().poll_interval_ms(),
            task_record_version,
        )
        .map_err(application_corruption)?;
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let record = build_task_bound(
            expected.key().clone(),
            expected.link().clone(),
            task,
            next_sequence(expected.lifecycle_link_version())?,
            mutation_sequence,
            task_record_version,
            expected.bind_epoch_ms(),
            AttemptPhase::Begun,
            self.limits,
        )?;
        let result = record.clone();
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(
            record,
        )))?;
        let committed = self.publish_and_readback(&next, expected.key_digest(), deadline)?;
        *writer = committed;
        Ok(result)
    }

    pub(crate) fn refresh_task_bound_projection(
        &self,
        expected: &TaskBoundReceipt,
        task: ReceiptTaskProjection,
        deadline: ProviderDeadline,
    ) -> Result<TaskBoundReceipt, TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let current = exact_task_bound(
            &writer,
            expected.key_digest(),
            expected.lifecycle_link_version(),
        )?;
        if current != expected {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let record = build_task_bound(
            expected.key().clone(),
            expected.link().clone(),
            task.clone(),
            next_sequence(expected.lifecycle_link_version())?,
            mutation_sequence,
            task.version(),
            expected.bind_epoch_ms(),
            expected.phase(),
            self.limits,
        )?;
        let result = record.clone();
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(
            record,
        )))?;
        let committed = self.publish_and_readback(&next, expected.key_digest(), deadline)?;
        *writer = committed;
        match writer.entries.get(expected.key_digest()) {
            Some(CatalogEntry::Link(TaskLifecycleLinkRecord::TaskBound(readback)))
                if readback == &result =>
            {
                Ok(readback.clone())
            }
            _ => Err(TaskLifecycleLinkStoreError::Corrupt(
                "durable refreshed TaskBound readback changed",
            )),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn publish_task_terminal_bound(
        &self,
        expected: &TaskBoundReceipt,
        terminal_task: ReceiptTaskProjection,
        task_record_version: u64,
        terminal_status: ClosedTerminalStatus,
        terminal_digest: TerminalDigest,
        terminal_epoch_ms: u64,
        deadline: ProviderDeadline,
    ) -> Result<TaskTerminalBoundReceipt, TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let current = exact_task_bound(
            &writer,
            expected.key_digest(),
            expected.lifecycle_link_version(),
        )?;
        if current != expected {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let expires_at_epoch_ms = terminal_epoch_ms
            .checked_add(terminal_task.ttl_ms())
            .ok_or(TaskLifecycleLinkStoreError::Corrupt(
                "terminal retention timestamp overflowed",
            ))?;
        let record = build_task_terminal_bound(
            expected.key().clone(),
            expected.link().clone(),
            terminal_task,
            next_sequence(expected.lifecycle_link_version())?,
            mutation_sequence,
            task_record_version,
            terminal_status,
            terminal_digest,
            terminal_epoch_ms,
            expires_at_epoch_ms,
            self.limits,
        )?;
        let result = record.clone();
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Link(
            TaskLifecycleLinkRecord::TaskTerminalBound(record),
        ))?;
        let committed = self.publish_and_readback(&next, expected.key_digest(), deadline)?;
        *writer = committed;
        Ok(result)
    }

    pub(crate) fn begin_task_retirement(
        &self,
        expected: &TaskTerminalBoundReceipt,
        invocation_index_bytes: u64,
        reserved_task_index_bytes: u64,
        deadline: ProviderDeadline,
    ) -> Result<TaskRetirementPendingReceipt, TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let current = exact_task_terminal_bound(
            &writer,
            expected.key_digest(),
            expected.lifecycle_link_version(),
        )?;
        if current != expected {
            return Err(TaskLifecycleLinkStoreError::StateMismatch);
        }
        let mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let record = build_task_retirement_pending(
            expected,
            next_sequence(expected.lifecycle_link_version())?,
            mutation_sequence,
            invocation_index_bytes,
            reserved_task_index_bytes,
            self.limits,
        )?;
        let result = record.clone();
        let mut next = writer.clone();
        next.mutation_sequence = mutation_sequence;
        next.insert_exact(CatalogEntry::Link(
            TaskLifecycleLinkRecord::TaskRetirementPending(record),
        ))?;
        let committed = self.publish_and_readback(&next, expected.key_digest(), deadline)?;
        *writer = committed;
        Ok(result)
    }

    pub(crate) fn finalize_task_retirement(
        &self,
        expected: &TaskRetirementPendingReceipt,
        deadline: ProviderDeadline,
    ) -> Result<(), TaskLifecycleLinkStoreError> {
        let mut writer = self.lock_writer(deadline)?;
        let mut next = writer.clone();
        next.remove_exact_pending(expected)?;
        next.mutation_sequence = next_sequence(writer.mutation_sequence)?;
        let committed = self.publish_and_readback(&next, expected.key_digest(), deadline)?;
        *writer = committed;
        Ok(())
    }

    pub(crate) fn read_by_task_id(
        &self,
        task_id: TaskId,
        deadline: ProviderDeadline,
    ) -> Result<TaskLifecycleLinkRecord, TaskLifecycleLinkStoreError> {
        let writer = self.lock_writer(deadline)?;
        self.verify_root_authority()?;
        let key_digest = writer
            .task_index
            .get(&task_id)
            .ok_or(TaskLifecycleLinkStoreError::NotFound { task_id })?;
        match writer.entries.get(key_digest) {
            Some(CatalogEntry::Link(record)) => Ok(record.clone()),
            Some(CatalogEntry::Reservation(_)) => {
                Err(TaskLifecycleLinkStoreError::NotFound { task_id })
            }
            None => Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link task index points to no record",
            )),
        }
    }

    fn inspect_and_recover(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<StoreCatalog, TaskLifecycleLinkStoreError> {
        self.verify_root_authority()?;
        let entries = read_directory_names_bounded(&self.root_file, MAX_DIRECTORY_ENTRIES, || {
            checkpoint_io(deadline)
        })
        .map_err(|error| {
            if error.kind() == io::ErrorKind::TimedOut {
                TaskLifecycleLinkStoreError::DeadlineExceeded
            } else if error.kind() == io::ErrorKind::FileTooLarge {
                TaskLifecycleLinkStoreError::Corrupt(
                    "Task lifecycle-link directory entry bound was exceeded",
                )
            } else {
                storage_error("enumerate Task lifecycle-link root", error)
            }
        })?;
        let mut snapshot_present = false;
        let mut staging_removed = false;
        for name in entries {
            check_deadline(deadline)?;
            let encoded = name.to_str().ok_or(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link entry name is not UTF-8",
            ))?;
            if encoded == STORE_LOCK_FILE {
                continue;
            }
            if encoded == STORE_SNAPSHOT_FILE {
                if snapshot_present {
                    return Err(TaskLifecycleLinkStoreError::Corrupt(
                        "duplicate Task lifecycle-link snapshot entry",
                    ));
                }
                snapshot_present = true;
                continue;
            }
            if encoded.starts_with(STORE_STAGING_PREFIX) && encoded.ends_with(STORE_STAGING_SUFFIX)
            {
                let staged = open_regular_child_nofollow(&self.root_file, &name).map_err(|_| {
                    TaskLifecycleLinkStoreError::Corrupt(
                        "Task lifecycle-link staging entry is not a regular file",
                    )
                })?;
                verify_owner_only_acl(&staged).map_err(|error| {
                    storage_error("verify Task lifecycle-link staging ownership", error)
                })?;
                let identity = file_identity(&staged).map_err(|error| {
                    storage_error("identify Task lifecycle-link staging entry", error)
                })?;
                remove_identity_bound_regular_child(&self.root_file, &name, identity, &staged)
                    .map_err(|error| {
                        storage_error("remove abandoned Task lifecycle-link staging", error)
                    })?;
                staging_removed = true;
                continue;
            }
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link root contains an unsupported entry",
            ));
        }
        if staging_removed {
            sync_directory(&self.root_file).map_err(|error| {
                storage_error("sync Task lifecycle-link staging cleanup", error)
            })?;
        }
        if !snapshot_present {
            return Ok(StoreCatalog::default());
        }
        self.read_catalog_from_disk(deadline)
    }

    fn read_catalog_from_disk(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<StoreCatalog, TaskLifecycleLinkStoreError> {
        check_deadline(deadline)?;
        let file = open_regular_child_nofollow(&self.root_file, OsStr::new(STORE_SNAPSHOT_FILE))
            .map_err(|error| storage_error("open Task lifecycle-link snapshot", error))?;
        verify_owner_only_acl(&file).map_err(|error| {
            storage_error("verify Task lifecycle-link snapshot ownership", error)
        })?;
        let mut bytes = Vec::new();
        file.take((MAX_SNAPSHOT_BYTES as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| storage_error("read Task lifecycle-link snapshot", error))?;
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link snapshot exceeds its bounded envelope",
            ));
        }
        let stored: StoredCatalogV1 = serde_json::from_slice(&bytes).map_err(|_| {
            TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link snapshot is not strict schema-v1 JSON",
            )
        })?;
        require_schema_v1(stored.schema_version)?;
        let mut catalog = StoreCatalog {
            mutation_sequence: stored.mutation_sequence,
            ..StoreCatalog::default()
        };
        let mut maximum_sequence = 0;
        for stored_entry in stored.entries {
            let encoded_len = stored_entry.encoded_len()?;
            ensure_record_size(encoded_len, self.limits)?;
            let entry = stored_entry.into_catalog_entry()?;
            maximum_sequence = maximum_sequence.max(entry.mutation_sequence());
            if catalog.entries.contains_key(entry.key_digest()) {
                return Err(TaskLifecycleLinkStoreError::Corrupt(
                    "Task lifecycle-link snapshot contains a duplicate exact key",
                ));
            }
            catalog.insert_exact(entry)?;
        }
        if maximum_sequence > catalog.mutation_sequence {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link entry mutation exceeds the durable high-water mark",
            ));
        }
        validate_capacity(&catalog, self.limits)?;
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        Ok(catalog)
    }

    fn publish_and_readback(
        &self,
        catalog: &StoreCatalog,
        receipt_key_digest: &ReceiptKeyDigest,
        deadline: ProviderDeadline,
    ) -> Result<StoreCatalog, TaskLifecycleLinkStoreError> {
        validate_capacity(catalog, self.limits)?;
        let snapshot = stored_catalog(catalog)?;
        let encoded = serde_json::to_vec(&snapshot)
            .map_err(|error| storage_message("serialize Task lifecycle-link snapshot", error))?;
        if encoded.len() > MAX_SNAPSHOT_BYTES {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link snapshot exceeds its bounded envelope",
            ));
        }
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let temporary_name = format!(
            "{STORE_STAGING_PREFIX}{}{STORE_STAGING_SUFFIX}",
            Uuid::new_v4()
        );
        let temporary_name = OsStr::new(&temporary_name);
        let mut staged = create_new_regular_child(&self.root_file, temporary_name)
            .map_err(|error| storage_error("create Task lifecycle-link staging file", error))?;
        let staged_identity = file_identity(&staged)
            .map_err(|error| storage_error("identify Task lifecycle-link staging file", error))?;
        if let Err(error) = restrict_stage_to_owner(&staged) {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(storage_error(
                "restrict Task lifecycle-link staging file",
                error,
            ));
        }
        if let Err(error) = staged.write_all(&encoded).and_then(|()| staged.sync_all()) {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(storage_error(
                "flush Task lifecycle-link staging file",
                error,
            ));
        }
        check_deadline(deadline)?;
        self.verify_root_authority()?;
        let target = OsStr::new(STORE_SNAPSHOT_FILE);
        let publication = match open_regular_child_nofollow(&self.root_file, target) {
            Ok(existing) => {
                verify_owner_only_acl(&existing).map_err(|error| {
                    storage_error("verify existing Task lifecycle-link snapshot", error)
                })?;
                replace_identity_bound_regular_child(
                    &self.root_file,
                    temporary_name,
                    staged_identity,
                    &staged,
                    target,
                )
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                rename_identity_bound_regular_child_no_replace(
                    &self.root_file,
                    temporary_name,
                    staged_identity,
                    &staged,
                    &self.root_file,
                    target,
                )
            }
            Err(error) => return Err(storage_error("inspect Task lifecycle-link snapshot", error)),
        };
        if let Err(error) = publication {
            let _ = remove_identity_bound_regular_child(
                &self.root_file,
                temporary_name,
                staged_identity,
                &staged,
            );
            return Err(storage_error("publish Task lifecycle-link snapshot", error));
        }
        check_deadline(deadline).map_err(|_| TaskLifecycleLinkStoreError::CommitUncertain {
            receipt_key_digest: receipt_key_digest.clone(),
        })?;
        sync_directory(&self.root_file).map_err(|_| {
            TaskLifecycleLinkStoreError::CommitUncertain {
                receipt_key_digest: receipt_key_digest.clone(),
            }
        })?;
        let readback = self
            .read_catalog_from_disk(deadline)
            .map_err(|error| match error {
                TaskLifecycleLinkStoreError::DeadlineExceeded => {
                    TaskLifecycleLinkStoreError::CommitUncertain {
                        receipt_key_digest: receipt_key_digest.clone(),
                    }
                }
                other => other,
            })?;
        if &readback != catalog {
            return Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link exact durable readback changed the committed catalog",
            ));
        }
        Ok(readback)
    }

    fn lock_writer(
        &self,
        deadline: ProviderDeadline,
    ) -> Result<MutexGuard<'_, StoreCatalog>, TaskLifecycleLinkStoreError> {
        loop {
            check_deadline(deadline)?;
            match self.writer.try_lock() {
                Ok(writer) => return Ok(writer),
                Err(TryLockError::Poisoned(poisoned)) => return Ok(poisoned.into_inner()),
                Err(TryLockError::WouldBlock) => {
                    std::thread::sleep(deadline.remaining().min(STORE_WRITER_WAIT_SLICE));
                }
            }
        }
    }

    fn verify_root_authority(&self) -> Result<(), TaskLifecycleLinkStoreError> {
        self.root
            .validate_named_identity()
            .map_err(|error| storage_error("validate Task lifecycle-link root", error))?;
        verify_owner_only_acl(&self.root_file)
            .map_err(|error| storage_error("verify Task lifecycle-link root ownership", error))
    }

    fn capacity_error(&self) -> TaskLifecycleLinkStoreError {
        TaskLifecycleLinkStoreError::Capacity {
            maximum_records: self.limits.max_records,
            maximum_bytes: self.limits.max_pool_bytes,
        }
    }
}

fn build_reservation(
    key: ReceiptKey,
    link: TaskLinkReference,
    mutation_sequence: u64,
    limits: StoreLimits,
) -> Result<TaskLinkReservation, TaskLifecycleLinkStoreError> {
    let key_digest = receipt_key_digest(&key);
    validate_exact_identity(&key, &key_digest, &link)?;
    let mut reservation = TaskLinkReservation {
        key,
        key_digest,
        link,
        reservation_version: 1,
        mutation_sequence,
        encoded_bytes: 0,
    };
    let stored = StoredEntryV1::from_catalog_entry(&CatalogEntry::Reservation(reservation.clone()));
    let encoded_len = stored.encoded_len()?;
    ensure_record_size(encoded_len, limits)?;
    reservation.encoded_bytes = u64::try_from(encoded_len)
        .map_err(|_| TaskLifecycleLinkStoreError::Corrupt("reservation length does not fit u64"))?;
    Ok(reservation)
}

#[allow(clippy::too_many_arguments)]
fn build_task_bound(
    key: ReceiptKey,
    link: TaskLinkReference,
    task: ReceiptTaskProjection,
    lifecycle_link_version: u64,
    mutation_sequence: u64,
    task_record_version: u64,
    bind_epoch_ms: u64,
    phase: AttemptPhase,
    limits: StoreLimits,
) -> Result<TaskBoundReceipt, TaskLifecycleLinkStoreError> {
    let provisional_header = LifecycleLinkRecordHeader::new(
        key.clone(),
        link.clone(),
        lifecycle_link_version,
        mutation_sequence,
        1,
    )
    .map_err(application_corruption)?;
    let provisional = TaskBoundReceipt::new(
        provisional_header,
        task.clone(),
        task_record_version,
        bind_epoch_ms,
        phase,
    )
    .map_err(application_corruption)?;
    let stored = StoredEntryV1::from_catalog_entry(&CatalogEntry::Link(
        TaskLifecycleLinkRecord::TaskBound(provisional),
    ));
    let encoded_len = stored.encoded_len()?;
    ensure_record_size(encoded_len, limits)?;
    let header = LifecycleLinkRecordHeader::new(
        key,
        link,
        lifecycle_link_version,
        mutation_sequence,
        encoded_len as u64,
    )
    .map_err(application_corruption)?;
    TaskBoundReceipt::new(header, task, task_record_version, bind_epoch_ms, phase)
        .map_err(application_corruption)
}

#[allow(clippy::too_many_arguments)]
fn build_task_terminal_bound(
    key: ReceiptKey,
    link: TaskLinkReference,
    task: ReceiptTaskProjection,
    lifecycle_link_version: u64,
    mutation_sequence: u64,
    task_record_version: u64,
    terminal_status: ClosedTerminalStatus,
    terminal_digest: TerminalDigest,
    terminal_epoch_ms: u64,
    expires_at_epoch_ms: u64,
    limits: StoreLimits,
) -> Result<TaskTerminalBoundReceipt, TaskLifecycleLinkStoreError> {
    let provisional_header = LifecycleLinkRecordHeader::new(
        key.clone(),
        link.clone(),
        lifecycle_link_version,
        mutation_sequence,
        1,
    )
    .map_err(application_corruption)?;
    let provisional = TaskTerminalBoundReceipt::new(
        provisional_header,
        task.clone(),
        task_record_version,
        terminal_status,
        terminal_digest.clone(),
        terminal_epoch_ms,
        expires_at_epoch_ms,
    )
    .map_err(application_corruption)?;
    let stored = StoredEntryV1::from_catalog_entry(&CatalogEntry::Link(
        TaskLifecycleLinkRecord::TaskTerminalBound(provisional),
    ));
    let encoded_len = stored.encoded_len()?;
    ensure_record_size(encoded_len, limits)?;
    let header = LifecycleLinkRecordHeader::new(
        key,
        link,
        lifecycle_link_version,
        mutation_sequence,
        encoded_len as u64,
    )
    .map_err(application_corruption)?;
    TaskTerminalBoundReceipt::new(
        header,
        task,
        task_record_version,
        terminal_status,
        terminal_digest,
        terminal_epoch_ms,
        expires_at_epoch_ms,
    )
    .map_err(application_corruption)
}

fn build_task_retirement_pending(
    expected: &TaskTerminalBoundReceipt,
    lifecycle_link_version: u64,
    mutation_sequence: u64,
    invocation_index_bytes: u64,
    reserved_task_index_bytes: u64,
    limits: StoreLimits,
) -> Result<TaskRetirementPendingReceipt, TaskLifecycleLinkStoreError> {
    let accounting =
        RetainedDualIdAccounting::new(invocation_index_bytes, reserved_task_index_bytes)
            .map_err(application_corruption)?;
    let provisional_header = LifecycleLinkRecordHeader::new(
        expected.key().clone(),
        expected.link().clone(),
        lifecycle_link_version,
        mutation_sequence,
        1,
    )
    .map_err(application_corruption)?;
    let provisional = TaskRetirementPendingReceipt::new(
        provisional_header,
        expected.task().clone(),
        expected.task_record_version(),
        expected.terminal_status(),
        expected.terminal_digest().clone(),
        expected.terminal_epoch_ms(),
        expected.expires_at_epoch_ms(),
        expected.encoded_bytes(),
        accounting.clone(),
    )
    .map_err(application_corruption)?;
    let stored = StoredEntryV1::from_catalog_entry(&CatalogEntry::Link(
        TaskLifecycleLinkRecord::TaskRetirementPending(provisional),
    ));
    let encoded_len = stored.encoded_len()?;
    ensure_record_size(encoded_len, limits)?;
    let header = LifecycleLinkRecordHeader::new(
        expected.key().clone(),
        expected.link().clone(),
        lifecycle_link_version,
        mutation_sequence,
        encoded_len as u64,
    )
    .map_err(application_corruption)?;
    TaskRetirementPendingReceipt::new(
        header,
        expected.task().clone(),
        expected.task_record_version(),
        expected.terminal_status(),
        expected.terminal_digest().clone(),
        expected.terminal_epoch_ms(),
        expected.expires_at_epoch_ms(),
        expected.encoded_bytes(),
        accounting,
    )
    .map_err(application_corruption)
}

fn exact_task_bound<'a>(
    catalog: &'a StoreCatalog,
    key_digest: &ReceiptKeyDigest,
    expected_version: u64,
) -> Result<&'a TaskBoundReceipt, TaskLifecycleLinkStoreError> {
    let entry = catalog
        .entries
        .get(key_digest)
        .ok_or(TaskLifecycleLinkStoreError::StateMismatch)?;
    let CatalogEntry::Link(record) = entry else {
        return Err(TaskLifecycleLinkStoreError::StateMismatch);
    };
    if record.lifecycle_link_version() != expected_version {
        return Err(TaskLifecycleLinkStoreError::VersionMismatch {
            expected: expected_version,
            actual: record.lifecycle_link_version(),
        });
    }
    match record {
        TaskLifecycleLinkRecord::TaskBound(record) => Ok(record),
        TaskLifecycleLinkRecord::TaskTerminalBound(_)
        | TaskLifecycleLinkRecord::TaskRetirementPending(_) => {
            Err(TaskLifecycleLinkStoreError::StateMismatch)
        }
    }
}

fn exact_task_terminal_bound<'a>(
    catalog: &'a StoreCatalog,
    key_digest: &ReceiptKeyDigest,
    expected_version: u64,
) -> Result<&'a TaskTerminalBoundReceipt, TaskLifecycleLinkStoreError> {
    let entry = catalog
        .entries
        .get(key_digest)
        .ok_or(TaskLifecycleLinkStoreError::StateMismatch)?;
    let CatalogEntry::Link(record) = entry else {
        return Err(TaskLifecycleLinkStoreError::StateMismatch);
    };
    if record.lifecycle_link_version() != expected_version {
        return Err(TaskLifecycleLinkStoreError::VersionMismatch {
            expected: expected_version,
            actual: record.lifecycle_link_version(),
        });
    }
    match record {
        TaskLifecycleLinkRecord::TaskTerminalBound(record) => Ok(record),
        TaskLifecycleLinkRecord::TaskBound(_)
        | TaskLifecycleLinkRecord::TaskRetirementPending(_) => {
            Err(TaskLifecycleLinkStoreError::StateMismatch)
        }
    }
}

fn stored_catalog(catalog: &StoreCatalog) -> Result<StoredCatalogV1, TaskLifecycleLinkStoreError> {
    let mut entries: Vec<_> = catalog
        .entries
        .values()
        .map(StoredEntryV1::from_catalog_entry)
        .map(|entry| {
            serialize_stored_entry_for_order(&entry)
                .map(|canonical_bytes| (canonical_bytes, entry))
                .map_err(|error| {
                    storage_message("serialize Task lifecycle-link catalog entry", error)
                })
        })
        .collect::<Result<_, _>>()?;
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    let entries = entries.into_iter().map(|(_, entry)| entry).collect();
    Ok(StoredCatalogV1 {
        schema_version: STORE_SCHEMA_VERSION,
        mutation_sequence: catalog.mutation_sequence,
        entries,
    })
}

fn serialize_stored_entry_for_order(entry: &StoredEntryV1) -> Result<Vec<u8>, serde_json::Error> {
    #[cfg(test)]
    STORED_ENTRY_SERIALIZATION_CALLS.with(|calls| calls.set(calls.get() + 1));
    serde_json::to_vec(entry)
}

#[cfg(test)]
thread_local! {
    static STORED_ENTRY_SERIALIZATION_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn validate_exact_identity(
    key: &ReceiptKey,
    key_digest: &ReceiptKeyDigest,
    link: &TaskLinkReference,
) -> Result<(), TaskLifecycleLinkStoreError> {
    if receipt_key_digest(key) != *key_digest
        || link.receipt_key_digest() != key_digest
        || link.task_id() != key.reserved_task_id()
        || link.invocation_id() != key.invocation_id()
    {
        return Err(TaskLifecycleLinkStoreError::IdentityMismatch);
    }
    Ok(())
}

fn validate_capacity(
    catalog: &StoreCatalog,
    limits: StoreLimits,
) -> Result<(), TaskLifecycleLinkStoreError> {
    let snapshot = catalog.capacity_snapshot(limits.max_record_bytes);
    if snapshot.task_store_slots_accounted() > limits.max_records
        || snapshot.accounted_bytes() > limits.max_pool_bytes
    {
        return Err(TaskLifecycleLinkStoreError::Capacity {
            maximum_records: limits.max_records,
            maximum_bytes: limits.max_pool_bytes,
        });
    }
    Ok(())
}

fn ensure_record_size(
    encoded_len: usize,
    limits: StoreLimits,
) -> Result<(), TaskLifecycleLinkStoreError> {
    if encoded_len > limits.max_record_bytes {
        Err(TaskLifecycleLinkStoreError::RecordTooLarge {
            actual: encoded_len,
            maximum: limits.max_record_bytes,
        })
    } else {
        Ok(())
    }
}

fn next_sequence(current: u64) -> Result<u64, TaskLifecycleLinkStoreError> {
    current
        .checked_add(1)
        .ok_or(TaskLifecycleLinkStoreError::Corrupt(
            "Task lifecycle-link sequence overflowed",
        ))
}

fn require_schema_v1(schema_version: u32) -> Result<(), TaskLifecycleLinkStoreError> {
    if schema_version == STORE_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(TaskLifecycleLinkStoreError::Corrupt(
            "Task lifecycle-link schema version is not 1",
        ))
    }
}

fn application_corruption(_: ReceiptLedgerError) -> TaskLifecycleLinkStoreError {
    TaskLifecycleLinkStoreError::Corrupt(
        "Task lifecycle-link application model rejected persisted exact state",
    )
}

fn check_deadline(deadline: ProviderDeadline) -> Result<(), TaskLifecycleLinkStoreError> {
    if deadline.remaining().is_zero() {
        Err(TaskLifecycleLinkStoreError::DeadlineExceeded)
    } else {
        Ok(())
    }
}

fn checkpoint_io(deadline: ProviderDeadline) -> io::Result<()> {
    check_deadline(deadline).map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "Task lifecycle-link inspection deadline expired",
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

fn storage_error(operation: &'static str, error: io::Error) -> TaskLifecycleLinkStoreError {
    TaskLifecycleLinkStoreError::Storage {
        operation,
        message: error.to_string(),
    }
}

fn storage_message(
    operation: &'static str,
    error: impl fmt::Display,
) -> TaskLifecycleLinkStoreError {
    TaskLifecycleLinkStoreError::Storage {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::receipt_ledger::{
        receipt_key_digest, request_scope_hash, AttemptPhase, ClosedTerminalStatus,
        CoreIdentityDigest, ReceiptKey, ReceiptTaskProjection, RequestIdentity, TaskLinkReference,
        TerminalDigest, V5ToolIdentity,
    };
    use crate::domain::code_intelligence::ProviderDeadline;
    use crate::domain::invocation::{
        InvocationId, NormalizedArgumentsHash, SafeIdentityHash, TaskId,
    };
    use crate::infrastructure::platform::filesystem::{
        create_owner_only_directory_child, open_directory_nofollow,
    };
    use std::fs;
    use std::str::FromStr;
    use std::time::Duration;

    const INVOCATION_A: &str = "11111111-1111-4111-8111-111111111111";
    const INVOCATION_B: &str = "22222222-2222-4222-8222-222222222222";
    const TASK_A: &str = "33333333-3333-4333-8333-333333333333";
    const TASK_B: &str = "44444444-4444-4444-8444-444444444444";

    fn generated_fixture(index: u128) -> (ReceiptKey, TaskLinkReference, ReceiptTaskProjection) {
        let invocation_id = Uuid::from_u128(0x11111111_1111_4111_8111_000000000000 + index);
        let task_id = Uuid::from_u128(0x33333333_3333_4333_8333_000000000000 + index);
        fixture(
            &invocation_id.to_string(),
            &task_id.to_string(),
            &format!("workspace-{index}"),
        )
    }

    fn deadline() -> ProviderDeadline {
        ProviderDeadline::from_budget(Duration::from_secs(7))
    }

    fn physical_root(root: &tempfile::TempDir) -> std::path::PathBuf {
        let parent = fs::canonicalize(root.path()).expect("physical temporary parent");
        let parent_file = open_directory_nofollow(&parent).expect("open temporary parent");
        create_owner_only_directory_child(&parent_file, std::ffi::OsStr::new("links"))
            .expect("create owner-only lifecycle-link root");
        parent.join("links")
    }

    fn fixture(
        invocation_id: &str,
        task_id: &str,
        workspace_hint: &str,
    ) -> (ReceiptKey, TaskLinkReference, ReceiptTaskProjection) {
        let invocation_id = InvocationId::from_str(invocation_id).expect("canonical invocation");
        let task_id = TaskId::from_str(task_id).expect("canonical task");
        let key = ReceiptKey::new(
            invocation_id,
            task_id,
            RequestIdentity::new(
                CoreIdentityDigest::from_sha256([0x55; 32]),
                V5ToolIdentity::View,
                NormalizedArgumentsHash::from_sha256([0x66; 32]),
                request_scope_hash(workspace_hint).expect("bounded request scope"),
            ),
        );
        let link = TaskLinkReference::new(
            receipt_key_digest(&key),
            task_id,
            invocation_id,
            SafeIdentityHash::from_sha256([0x77; 32]),
        );
        let task =
            ReceiptTaskProjection::new(task_id, invocation_id, 1_000, 1_000, 3_600_000, 100, 1)
                .expect("valid Task projection");
        (key, link, task)
    }

    fn terminal_digest() -> TerminalDigest {
        TerminalDigest::from_str(&"88".repeat(32)).expect("terminal digest")
    }

    #[test]
    fn reservation_consumes_task_store_slot_before_materialization_and_reopens_exactly() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (key, link, task) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");

        let reservation = store
            .reserve_task_link(key.clone(), link.clone(), deadline())
            .expect("reserve lifecycle-link and TaskStore slot");
        assert!(reservation.encoded_bytes() <= 1_024);
        assert_eq!(store.capacity_snapshot().live_reservations(), 1);
        assert_eq!(store.capacity_snapshot().task_store_slots_accounted(), 1);
        assert_eq!(store.capacity_snapshot().accounted_bytes(), 1_024);

        let bound = store
            .materialize_task_bound(
                &reservation,
                task,
                1,
                1_000,
                AttemptPhase::NotBegun,
                deadline(),
            )
            .expect("materialize exact TaskBound");
        assert_eq!(bound.lifecycle_link_version(), 2);
        assert_eq!(bound.phase(), AttemptPhase::NotBegun);
        assert_eq!(store.capacity_snapshot().live_reservations(), 0);
        assert_eq!(store.capacity_snapshot().materialized_links(), 1);

        drop(store);
        let reopened =
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("reopen store");
        assert_eq!(
            reopened
                .read_by_task_id(key.reserved_task_id(), deadline())
                .expect("exact reopened link"),
            TaskLifecycleLinkRecord::TaskBound(bound)
        );
    }

    #[test]
    fn catalog_snapshot_lists_exact_live_entries_and_separates_actual_from_reserved_bytes() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (reserved_key, reserved_link, _) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let (bound_key, bound_link, bound_task) = fixture(INVOCATION_B, TASK_B, "workspace-b");
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");
        let reservation = store
            .reserve_task_link(reserved_key, reserved_link, deadline())
            .expect("retain exact reservation");
        let bound_reservation = store
            .reserve_task_link(bound_key, bound_link, deadline())
            .expect("reserve exact bound link");
        let bound = store
            .materialize_task_bound(
                &bound_reservation,
                bound_task,
                1,
                1_000,
                AttemptPhase::NotBegun,
                deadline(),
            )
            .expect("materialize exact TaskBound");
        drop(store);
        let store =
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("reopen durable store");
        let persisted_before =
            fs::read(root_path.join(STORE_SNAPSHOT_FILE)).expect("read snapshot before inspection");

        let snapshot = store
            .catalog_snapshot(deadline())
            .expect("inspect exact live catalog");

        assert_eq!(snapshot.generation(), 3);
        assert_eq!(snapshot.count(), 2);
        assert_eq!(snapshot.reserved_count(), 1);
        assert_eq!(snapshot.actual_bytes(), bound.encoded_bytes());
        assert_eq!(
            snapshot.reserved_bytes(),
            u64::try_from(MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES).expect("record limit fits u64")
        );
        assert_eq!(
            snapshot.actual_bytes() + snapshot.reserved_bytes(),
            u64::try_from(store.capacity_snapshot().accounted_bytes())
                .expect("capacity accounting fits u64")
        );
        assert_eq!(snapshot.entries().len(), snapshot.count());
        assert!(snapshot.entries().windows(2).all(|entries| {
            entries[0].key_digest().to_string() <= entries[1].key_digest().to_string()
        }));
        assert!(snapshot
            .entries()
            .contains(&TaskLifecycleLinkCatalogEntry::Reservation(reservation)));
        assert!(snapshot
            .entries()
            .contains(&TaskLifecycleLinkCatalogEntry::Record(
                TaskLifecycleLinkRecord::TaskBound(bound)
            )));
        assert_eq!(
            fs::read(root_path.join(STORE_SNAPSHOT_FILE)).expect("read snapshot after inspection"),
            persisted_before
        );
        assert_eq!(
            store
                .catalog_snapshot(deadline())
                .expect("repeat read-only inspection"),
            snapshot
        );
    }

    #[test]
    fn task_bound_terminal_and_retirement_transitions_are_exact_cas_and_reopen_stable() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (key, link, task) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");
        let reservation = store
            .reserve_task_link(key.clone(), link, deadline())
            .expect("reserve link");
        let not_begun = store
            .materialize_task_bound(
                &reservation,
                task,
                1,
                1_000,
                AttemptPhase::NotBegun,
                deadline(),
            )
            .expect("materialize TaskBound");
        assert!(not_begun.encoded_bytes() <= 1_024);
        let begun = store
            .mark_task_bound_begun(&not_begun, 2, 1_100, deadline())
            .expect("exact not-begun to begun CAS");
        assert!(begun.encoded_bytes() <= 1_024);
        let terminal_task = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            1_000,
            2_000,
            3_600_000,
            100,
            3,
        )
        .expect("terminal projection");
        let terminal = store
            .publish_task_terminal_bound(
                &begun,
                terminal_task,
                3,
                ClosedTerminalStatus::Completed,
                terminal_digest(),
                2_000,
                deadline(),
            )
            .expect("exact TaskTerminalBound CAS");
        assert!(terminal.encoded_bytes() <= 1_024);
        let pending = store
            .begin_task_retirement(&terminal, 64, 64, deadline())
            .expect("exact TaskRetirementPending CAS");
        assert!(pending.encoded_bytes() <= 1_024);

        assert!(matches!(
            store.mark_task_bound_begun(&not_begun, 2, 1_100, deadline()),
            Err(TaskLifecycleLinkStoreError::VersionMismatch {
                expected: 2,
                actual: 5
            })
        ));
        drop(store);

        let reopened =
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("reopen store");
        assert_eq!(
            reopened
                .read_by_task_id(key.reserved_task_id(), deadline())
                .expect("reopened pending link"),
            TaskLifecycleLinkRecord::TaskRetirementPending(pending)
        );
    }

    #[test]
    fn finalized_retirement_removes_exact_link_and_both_indexes_across_reopen() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (key, link, task) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");
        let reservation = store
            .reserve_task_link(key.clone(), link, deadline())
            .expect("reserve link");
        let bound = store
            .materialize_task_bound(
                &reservation,
                task,
                1,
                1_000,
                AttemptPhase::Begun,
                deadline(),
            )
            .expect("materialize TaskBound");
        let terminal_task = ReceiptTaskProjection::new(
            key.reserved_task_id(),
            key.invocation_id(),
            1_000,
            2_000,
            3_600_000,
            100,
            2,
        )
        .expect("terminal projection");
        let terminal = store
            .publish_task_terminal_bound(
                &bound,
                terminal_task,
                2,
                ClosedTerminalStatus::Completed,
                terminal_digest(),
                2_000,
                deadline(),
            )
            .expect("publish terminal link");
        let pending = store
            .begin_task_retirement(&terminal, 64, 64, deadline())
            .expect("commit pending retirement");

        store
            .finalize_task_retirement(&pending, deadline())
            .expect("finalize exact retirement");
        assert!(matches!(
            store.read_by_task_id(key.reserved_task_id(), deadline()),
            Err(TaskLifecycleLinkStoreError::NotFound { .. })
        ));
        assert_eq!(store.catalog_snapshot(deadline()).unwrap().count(), 0);
        drop(store);

        let reopened =
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("reopen store");
        assert!(matches!(
            reopened.read_by_task_id(key.reserved_task_id(), deadline()),
            Err(TaskLifecycleLinkStoreError::NotFound { .. })
        ));
        assert_eq!(reopened.catalog_snapshot(deadline()).unwrap().count(), 0);
    }

    #[test]
    fn count_and_byte_entitlement_reject_second_reservation_before_task_store_create() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (first_key, first_link, _) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let (second_key, second_link, _) = fixture(INVOCATION_B, TASK_B, "workspace-b");
        let store =
            TaskLifecycleLinkStoreV5::open_with_limits_for_test(&root_path, 1, 1_024, deadline())
                .expect("open bounded store");

        store
            .reserve_task_link(first_key, first_link, deadline())
            .expect("first reservation");
        assert!(matches!(
            store.reserve_task_link(second_key, second_link, deadline()),
            Err(TaskLifecycleLinkStoreError::Capacity { .. })
        ));
        assert_eq!(store.capacity_snapshot().task_store_slots_accounted(), 1);
        assert_eq!(store.capacity_snapshot().accounted_bytes(), 1_024);
    }

    #[test]
    fn persisted_record_schema_is_v1_strict_and_revalidates_the_link_digest() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let (key, link, _) = fixture(INVOCATION_A, TASK_A, "workspace-a");
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");
        store
            .reserve_task_link(key, link, deadline())
            .expect("reserve link");
        drop(store);

        let snapshot_path = root_path.join(STORE_SNAPSHOT_FILE);
        let mut snapshot: serde_json::Value = serde_json::from_slice(
            &fs::read(&snapshot_path).expect("read lifecycle-link snapshot"),
        )
        .expect("strict snapshot fixture");
        {
            let entry = snapshot["entries"][0]
                .as_object_mut()
                .expect("stored lifecycle-link entry");
            assert_eq!(entry.get("schemaVersion"), Some(&serde_json::json!(1)));
            assert_eq!(entry.get("state"), Some(&serde_json::json!("reservation")));
            entry.insert("unexpected".to_owned(), serde_json::json!(true));
        }
        fs::write(
            &snapshot_path,
            serde_json::to_vec(&snapshot).expect("serialize corrupt fixture"),
        )
        .expect("write corrupt fixture");

        assert!(matches!(
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()),
            Err(TaskLifecycleLinkStoreError::Corrupt(
                "Task lifecycle-link snapshot is not strict schema-v1 JSON"
            ))
        ));

        {
            let entry = snapshot["entries"][0]
                .as_object_mut()
                .expect("stored lifecycle-link entry");
            entry.remove("unexpected");
            entry
                .get_mut("link")
                .and_then(serde_json::Value::as_object_mut)
                .expect("stored exact link")
                .insert(
                    "taskLinkDigest".to_owned(),
                    serde_json::json!("99".repeat(32)),
                );
        }
        fs::write(
            &snapshot_path,
            serde_json::to_vec(&snapshot).expect("serialize digest mismatch fixture"),
        )
        .expect("write digest mismatch fixture");
        assert!(matches!(
            TaskLifecycleLinkStoreV5::open(&root_path, deadline()),
            Err(TaskLifecycleLinkStoreError::Corrupt(
                "persisted lifecycle-link digest does not match its exact identity"
            ))
        ));
    }

    #[test]
    fn production_link_pool_limits_are_the_decision_literals() {
        assert_eq!(MAX_TASK_LIFECYCLE_LINK_RECORD_BYTES, 1_024);
        assert_eq!(MAX_TASK_LIFECYCLE_LINK_RECORDS, 4_096);
        assert_eq!(MAX_TASK_LIFECYCLE_LINK_POOL_BYTES, 4 * 1_024 * 1_024);
    }

    #[test]
    fn canonical_catalog_order_serializes_each_entry_once() {
        let root = tempfile::tempdir().expect("temporary root");
        let root_path = physical_root(&root);
        let store = TaskLifecycleLinkStoreV5::open(&root_path, deadline()).expect("open store");
        for index in 0..8 {
            let (key, link, _) = generated_fixture(index);
            store
                .reserve_task_link(key, link, deadline())
                .expect("reserve distinct link");
        }
        let catalog = store.lock_writer(deadline()).expect("lock live catalog");
        STORED_ENTRY_SERIALIZATION_CALLS.with(|calls| calls.set(0));

        let snapshot = stored_catalog(&catalog).expect("build canonical stored catalog");

        assert_eq!(snapshot.entries.len(), catalog.entries.len());
        STORED_ENTRY_SERIALIZATION_CALLS.with(|calls| {
            assert_eq!(
                calls.get(),
                catalog.entries.len(),
                "canonical ordering must serialize each entry exactly once"
            );
        });
    }
}
