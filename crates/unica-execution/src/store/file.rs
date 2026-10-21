//! One physical aggregate per execution group, with derived dual-ID indexes.
use super::codec::{self, CodecError, EncodedFrame};
use super::namespace::{hex, FreshNamespaceTicket, NamespaceAuthority, NamespaceRegistration};
use super::{OpenError, Rejection};
use crate::coordinator::hydration::{ReadCompletion, ReadRequest};
use crate::coordinator::{
    AuthorityEpoch, AuthorityFailure, AuthorityFailureCause, CommitCompletion, CommitRequest,
    WriteFailureCause,
};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use unica_execution_contract::invocation::{InvocationId, TaskId};
use unica_execution_contract::storage::{RecoveryBuilder, RecoveryCatalog, StoredRecord};
use unica_execution_contract::{Error, Key, Record};
use unica_workspace::state_fs::{DirectoryCursor, PublicationCertainty, PublishMode, Stage};
use unica_workspace::state_fs::{FileIdentity, StateFile};

#[derive(Debug)]
pub(crate) enum CommitStep {
    Completed(CommitCompletion),
    Rejected {
        request: CommitRequest,
        reason: Rejection,
    },
    AuthorityFailed {
        request: CommitRequest,
        cause: AuthorityFailureCause,
    },
}
#[derive(Debug)]
pub(crate) enum ReadStep {
    Loaded(ReadCompletion),
    AuthorityFailed(AuthorityFailure),
    Rejected {
        request: ReadRequest,
        reason: Rejection,
    },
    Obsolete {
        key: Key,
        observed_revision: u64,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScanProgress {
    Record,
    Stage,
    Complete,
}
struct Entry {
    record: Record,
    identity: FileIdentity,
}

pub(crate) struct OpeningStore {
    authority: NamespaceAuthority,
    cursor: DirectoryCursor,
    recovery: RecoveryBuilder,
    entries: HashMap<InvocationId, Entry>,
    tasks: HashMap<TaskId, InvocationId>,
    complete: bool,
    failed: bool,
}
impl OpeningStore {
    pub(crate) fn create_new(
        ticket: FreshNamespaceTicket,
        epoch: AuthorityEpoch,
    ) -> Result<Self, OpenError> {
        Self::start(NamespaceAuthority::create(ticket, epoch)?)
    }
    pub(crate) fn open_existing(
        registration: NamespaceRegistration,
        epoch: AuthorityEpoch,
    ) -> Result<Self, OpenError> {
        Self::start(NamespaceAuthority::open(registration, epoch)?)
    }
    fn start(authority: NamespaceAuthority) -> Result<Self, OpenError> {
        let cursor = DirectoryCursor::open(authority.records())?;
        Ok(Self {
            authority,
            cursor,
            recovery: RecoveryBuilder::new(),
            entries: HashMap::new(),
            tasks: HashMap::new(),
            complete: false,
            failed: false,
        })
    }
    pub(crate) fn scan_next(&mut self) -> Result<ScanProgress, OpenError> {
        if self.failed {
            return Err(OpenError::InvalidInventory);
        }
        let result = self.scan_one();
        if result.is_err() {
            self.failed = true;
            self.authority.freeze();
        }
        result
    }
    fn scan_one(&mut self) -> Result<ScanProgress, OpenError> {
        self.authority.validate()?;
        if self.complete {
            return Ok(ScanProgress::Complete);
        }
        let Some(name) = self.cursor.next_name()? else {
            self.authority.validate()?;
            self.complete = true;
            return Ok(ScanProgress::Complete);
        };
        let text = name.to_str().ok_or(OpenError::InvalidInventory)?;
        let retained = self.authority.records().retain_regular_child(&name)?;
        let file = retained.open_named_identity_for_read()?;
        file.verify_owner_only()?;
        if retained.hard_link_count()? != 1 {
            return Err(OpenError::InvalidInventory);
        }
        let stage_prefix = format!(".stage.{}.", hex(&self.authority.nonce()));
        if let Some(suffix) = text.strip_prefix(&stage_prefix) {
            let (id, nonce) = suffix.split_once('.').ok_or(OpenError::InvalidInventory)?;
            id.parse::<u64>().map_err(|_| OpenError::InvalidInventory)?;
            uuid::Uuid::parse_str(nonce).map_err(|_| OpenError::InvalidInventory)?;
            // A partial stage is not a commit. Preserve it; cleanup must never
            // precede the complete authoritative scan or promote its contents.
            return Ok(ScanProgress::Stage);
        }
        let digest_name = text
            .strip_suffix(".aggregate")
            .ok_or(OpenError::InvalidInventory)?;
        if digest_name.len() != 64
            || !digest_name
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(OpenError::InvalidInventory);
        }
        let length = file.metadata()?.len();
        let stored = codec::read_frame(&mut &file, length, self.authority.nonce())
            .map_err(|_| OpenError::InvalidInventory)?;
        retained.validate_named_identity()?;
        if hex(&stored.record().key().digest()) != digest_name {
            return Err(OpenError::InvalidInventory);
        }
        let record = stored.record().clone();
        let invocation = record.key().invocation_id();
        let task = record.key().task_id();
        if self.entries.contains_key(&invocation) || self.tasks.contains_key(&task) {
            return Err(OpenError::IdentityCollision);
        }
        self.recovery
            .push(stored)
            .map_err(|_| OpenError::InvalidInventory)?;
        self.tasks.insert(task, invocation);
        self.entries.insert(
            invocation,
            Entry {
                record,
                identity: retained.identity(),
            },
        );
        Ok(ScanProgress::Record)
    }
    pub(crate) fn finish(self) -> Result<(FileAggregateStore, RecoveryCatalog), OpenError> {
        if !self.complete || self.failed {
            return Err(OpenError::InvalidInventory);
        }
        self.authority.validate()?;
        let catalog = self
            .recovery
            .finish()
            .map_err(|_| OpenError::InvalidInventory)?;
        Ok((
            FileAggregateStore {
                authority: self.authority,
                entries: self.entries,
                tasks: self.tasks,
            },
            catalog,
        ))
    }
}

pub(crate) struct FileAggregateStore {
    authority: NamespaceAuthority,
    entries: HashMap<InvocationId, Entry>,
    tasks: HashMap<TaskId, InvocationId>,
}
enum PhysicalCommit {
    Published {
        record: Record,
        identity: FileIdentity,
    },
    Readback(StoredRecord),
    Known(WriteFailureCause),
    Unknown,
    Fault(AuthorityFailureCause),
}
fn write_cause(error: &io::Error) -> WriteFailureCause {
    match error.kind() {
        io::ErrorKind::StorageFull => WriteFailureCause::NoSpace,
        io::ErrorKind::PermissionDenied => WriteFailureCause::AccessDenied,
        _ => WriteFailureCause::IoFailure,
    }
}

impl FileAggregateStore {
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.authority.epoch()
    }
    pub(crate) fn is_frozen(&self) -> bool {
        self.authority.is_frozen()
    }
    pub(crate) fn freeze(&self) {
        self.authority.freeze();
    }
    /// The controller must have joined I/O and drained held completions first.
    pub(crate) fn close_quiescent(self) -> Result<(), OpenError> {
        self.authority.close_quiescent()
    }

    fn load(&self, key: &Key) -> Result<(StoredRecord, StateFile), AuthorityFailureCause> {
        self.authority
            .validate()
            .map_err(|_| AuthorityFailureCause::IdentityLost)?;
        let entry = self
            .entries
            .get(&key.invocation_id())
            .ok_or(AuthorityFailureCause::InvalidAggregate)?;
        if entry.record.key() != key {
            return Err(AuthorityFailureCause::InvalidAggregate);
        }
        let name = format!("{}.aggregate", hex(&key.digest()));
        let retained = self
            .authority
            .records()
            .retain_regular_child(OsStr::new(&name))
            .map_err(|_| AuthorityFailureCause::ReadFailed)?;
        if retained.identity() != entry.identity {
            return Err(AuthorityFailureCause::IdentityLost);
        }
        let file = retained
            .open_named_identity_for_read()
            .map_err(|_| AuthorityFailureCause::IdentityLost)?;
        file.verify_owner_only()
            .map_err(|_| AuthorityFailureCause::IdentityLost)?;
        if retained
            .hard_link_count()
            .map_err(|_| AuthorityFailureCause::ReadFailed)?
            != 1
        {
            return Err(AuthorityFailureCause::IdentityLost);
        }
        let length = file
            .metadata()
            .map_err(|_| AuthorityFailureCause::ReadFailed)?
            .len();
        let stored =
            codec::read_frame(&mut &file, length, self.authority.nonce()).map_err(|e| match e {
                CodecError::Io(_) => AuthorityFailureCause::ReadFailed,
                _ => AuthorityFailureCause::InvalidAggregate,
            })?;
        retained
            .validate_named_identity()
            .map_err(|_| AuthorityFailureCause::IdentityLost)?;
        self.authority
            .validate()
            .map_err(|_| AuthorityFailureCause::IdentityLost)?;
        if stored.record() != &entry.record {
            return Err(AuthorityFailureCause::InvalidAggregate);
        }
        Ok((stored, retained))
    }

    pub(crate) fn commit(&mut self, request: CommitRequest) -> CommitStep {
        if request.authority_epoch() != self.authority_epoch() {
            return CommitStep::Rejected {
                request,
                reason: Rejection::ForeignEpoch,
            };
        }
        if self.is_frozen() {
            return CommitStep::Rejected {
                request,
                reason: Rejection::Frozen,
            };
        }
        let key = request.key();
        if self
            .entries
            .get(&key.invocation_id())
            .is_some_and(|e| e.record.key() != key)
            || self
                .tasks
                .get(&key.task_id())
                .is_some_and(|id| *id != key.invocation_id())
        {
            return CommitStep::Rejected {
                request,
                reason: Rejection::IdentityCollision,
            };
        }
        // Request stays outside the fallible closure. A panic can never lose
        // its exact completion stamp or create another future publisher.
        let result = catch_unwind(AssertUnwindSafe(|| self.commit_physical(&request)));
        match result {
            Ok(PhysicalCommit::Published { record, identity }) => {
                let invocation = record.key().invocation_id();
                self.tasks.insert(record.key().task_id(), invocation);
                self.entries.insert(invocation, Entry { record, identity });
                CommitStep::Completed(request.committed())
            }
            Ok(PhysicalCommit::Readback(actual)) => CommitStep::Completed(request.readback(actual)),
            Ok(PhysicalCommit::Known(cause)) => {
                self.freeze();
                CommitStep::Completed(request.failed_before_publication(cause))
            }
            Ok(PhysicalCommit::Fault(cause)) => {
                self.freeze();
                CommitStep::AuthorityFailed { request, cause }
            }
            Ok(PhysicalCommit::Unknown) | Err(_) => {
                self.freeze();
                CommitStep::Completed(request.unknown())
            }
        }
    }

    fn commit_physical(&mut self, request: &CommitRequest) -> PhysicalCommit {
        if self.authority.validate().is_err() {
            return PhysicalCommit::Fault(AuthorityFailureCause::IdentityLost);
        }
        let prior = if self.entries.contains_key(&request.key().invocation_id()) {
            match self.load(request.key()) {
                Ok(p) => Some(p),
                Err(cause) => return PhysicalCommit::Fault(cause),
            }
        } else {
            None
        };
        if let Some((stored, _)) = &prior {
            if stored.record().revision() != request.expected_revision() {
                // This exact physical identity and metadata came from this
                // owner's published cache (or the complete initial scan).
                return PhysicalCommit::Readback(stored.clone());
            }
        } else if request.expected_revision() != 0 {
            return PhysicalCommit::Fault(AuthorityFailureCause::InvalidAggregate);
        }
        let candidate = match request.assemble(prior.as_ref().map(|p| &p.0)) {
            Ok(value) => value,
            Err(_) => return PhysicalCommit::Fault(AuthorityFailureCause::InvalidAggregate),
        };
        let frame = match EncodedFrame::prepare(&candidate, self.authority.nonce()) {
            Ok(frame) => frame,
            Err(CodecError::TooLarge) => {
                return PhysicalCommit::Known(WriteFailureCause::CandidateTooLarge);
            }
            Err(_) => return PhysicalCommit::Fault(AuthorityFailureCause::InvalidAggregate),
        };
        let stage_name = format!(
            ".stage.{}.{}.{}",
            hex(&self.authority.nonce()),
            request.id(),
            uuid::Uuid::new_v4()
        );
        let mut stage = match Stage::create(self.authority.records(), OsStr::new(&stage_name)) {
            Ok(stage) => stage,
            Err(error) => return self.before_publication_error(&error),
        };
        if let Err(error) = frame.write_to(&mut stage) {
            return self.before_publication_error(&error);
        }
        let sealed = match stage.seal() {
            Ok(stage) => stage,
            Err(error) => return self.before_publication_error(&error),
        };
        if self.authority.validate().is_err() {
            return PhysicalCommit::Fault(AuthorityFailureCause::IdentityLost);
        }
        let mode =
            prior
                .as_ref()
                .map_or(PublishMode::CreateNew, |(_, prior)| PublishMode::Replace {
                    expected: prior.identity(),
                });
        let destination = format!("{}.aggregate", hex(&request.key().digest()));
        match sealed.publish(OsStr::new(&destination), mode) {
            Ok(published) => {
                if self.authority.validate().is_err() {
                    return PhysicalCommit::Unknown;
                }
                PhysicalCommit::Published {
                    record: candidate.record().clone(),
                    identity: published.identity(),
                }
            }
            Err(error) => {
                let certainty = error.certainty();
                let before = self.before_publication_error(error.io_error());
                self.authority.retain_failure(error);
                match certainty {
                    PublicationCertainty::NeverPublished => before,
                    PublicationCertainty::MayHavePublished => PhysicalCommit::Unknown,
                }
            }
        }
    }

    fn before_publication_error(&self, error: &io::Error) -> PhysicalCommit {
        if self.authority.validate().is_err()
            || matches!(
                error.kind(),
                io::ErrorKind::InvalidData | io::ErrorKind::AlreadyExists
            )
        {
            PhysicalCommit::Fault(AuthorityFailureCause::IdentityLost)
        } else {
            PhysicalCommit::Known(write_cause(error))
        }
    }

    pub(crate) fn read(&mut self, request: ReadRequest) -> ReadStep {
        if request.authority_epoch() != self.authority_epoch() {
            return ReadStep::Rejected {
                request,
                reason: Rejection::ForeignEpoch,
            };
        }
        if self.is_frozen() {
            return ReadStep::Rejected {
                request,
                reason: Rejection::Frozen,
            };
        }
        let loaded = catch_unwind(AssertUnwindSafe(|| self.load(request.key())));
        let stored = match loaded {
            Ok(Ok((stored, _))) => stored,
            Ok(Err(cause)) => {
                self.freeze();
                return ReadStep::AuthorityFailed(request.authority_failed(cause));
            }
            Err(_) => {
                self.freeze();
                return ReadStep::AuthorityFailed(
                    request.authority_failed(AuthorityFailureCause::WorkerFailed),
                );
            }
        };
        let key = request.key().clone();
        let revision = stored.record().revision();
        if stored.record().fingerprint() != request.expected_record_fingerprint() {
            return ReadStep::Obsolete {
                key,
                observed_revision: revision,
            };
        }
        match request.loaded_or_return(&stored) {
            Ok(value) => ReadStep::Loaded(value),
            Err((_, Error::StaleCompletion)) => ReadStep::Obsolete {
                key,
                observed_revision: revision,
            },
            Err((request, _)) => {
                self.freeze();
                ReadStep::AuthorityFailed(
                    (*request).authority_failed(AuthorityFailureCause::InvalidAggregate),
                )
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;
