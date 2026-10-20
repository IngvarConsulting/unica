//! Explicit body reads outside the decision loop. Neither a metadata snapshot
//! nor an unvalidated byte buffer can mint a publication capability.
use super::*;
use crate::domain::execution::{HolderObservation, OutcomeDescriptor};

#[derive(Debug)]
enum Role {
    Direct { owner_hash: [u8; 32] },
    Task,
    Holder(Membership),
}
#[derive(Debug)]
pub(crate) struct ReadRequest {
    epoch: AuthorityEpoch,
    record: Record,
    descriptor: OutcomeDescriptor,
    role: Role,
}
impl ReadRequest {
    pub(crate) fn key(&self) -> &Key {
        self.record.key()
    }
    pub(crate) fn revision(&self) -> u64 {
        self.record.revision()
    }
    pub(crate) fn expected_record_fingerprint(&self) -> [u8; 32] {
        self.record.fingerprint()
    }
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
    pub(crate) fn descriptor(&self) -> &OutcomeDescriptor {
        &self.descriptor
    }
    pub(crate) fn authority_failed(self, cause: AuthorityFailureCause) -> AuthorityFailure {
        AuthorityFailure::for_read(
            self.epoch,
            cause,
            self.record.key().clone(),
            self.revision(),
        )
    }
    pub(crate) fn loaded(self, stored: &StoredRecord) -> Result<ReadCompletion, Error> {
        self.loaded_or_return(stored).map_err(|(_, error)| error)
    }
    /// Keep the consuming request available for a typed authority failure if
    /// a physical adapter cannot validate its loaded aggregate.
    pub(crate) fn loaded_or_return(
        self,
        stored: &StoredRecord,
    ) -> Result<ReadCompletion, (Box<Self>, Error)> {
        match self.loaded_body(stored) {
            Ok(outcome) => Ok(ReadCompletion {
                request: self,
                outcome,
            }),
            Err(error) => Err((Box::new(self), error)),
        }
    }
    fn loaded_body(&self, stored: &StoredRecord) -> Result<CanonicalOutcome, Error> {
        if stored.record() != &self.record {
            return Err(Error::StaleCompletion);
        }
        let outcome = match &self.role {
            Role::Direct { .. } | Role::Task => stored.primary(),
            Role::Holder(_) => stored.producer(),
        }
        .ok_or(Error::Compacted)?;
        if outcome.descriptor() != self.descriptor {
            return Err(Error::InvalidRecord);
        }
        Ok(outcome.clone())
    }
}
#[derive(Debug)]
pub(crate) struct ReadCompletion {
    request: ReadRequest,
    outcome: CanonicalOutcome,
}
#[derive(Debug)]
pub(crate) struct ResultPublicationPermit {
    key: Key,
    outcome: CanonicalOutcome,
}
impl ResultPublicationPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn outcome(&self) -> &CanonicalOutcome {
        &self.outcome
    }
}
#[derive(Debug)]
pub(crate) struct HolderPublicationPermit {
    key: Key,
    membership: Membership,
    outcome: CanonicalOutcome,
}
impl HolderPublicationPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn membership(&self) -> Membership {
        self.membership
    }
    pub(crate) fn outcome(&self) -> &CanonicalOutcome {
        &self.outcome
    }
}
#[derive(Debug)]
pub(crate) enum ReadPublication {
    Direct(DirectPublicationPermit),
    Task(ResultPublicationPermit),
    Holder(HolderPublicationPermit),
}
impl ExecutionCoordinator {
    pub(crate) fn prepare_direct(
        &self,
        key: &Key,
        owner: &OwnerCapability,
    ) -> Result<ReadRequest, Error> {
        self.prepare_read(
            key,
            Role::Direct {
                owner_hash: owner.hash(),
            },
        )
    }
    pub(crate) fn prepare_task(&self, key: &Key) -> Result<ReadRequest, Error> {
        self.prepare_read(key, Role::Task)
    }
    pub(crate) fn prepare_holder(
        &self,
        key: &Key,
        membership: Membership,
    ) -> Result<ReadRequest, Error> {
        self.prepare_read(key, Role::Holder(membership))
    }
    fn read_descriptor(&self, key: &Key, role: &Role) -> Result<OutcomeDescriptor, Error> {
        self.check_available()?;
        let record = self.inspect(key)?;
        if record.compacted() {
            return Err(Error::Compacted);
        }
        let descriptor = match role {
            Role::Direct { owner_hash } => {
                if !record.owned_by(*owner_hash) {
                    return Err(Error::OwnerMismatch);
                }
                if record.acknowledgement().is_some() {
                    return Err(Error::AlreadyAcknowledged);
                }
                record.primary().copied()
            }
            Role::Task => {
                // Accepted lookup survives a lost Submit even before ExposeTask.
                if !record.accepted() {
                    return Err(Error::NotAccepted);
                }
                record.primary().copied()
            }
            Role::Holder(member) => match self.observe_holder(key, *member)? {
                HolderObservation::Terminal(d) => Some(*d),
                HolderObservation::Cancelled | HolderObservation::Working => None,
            },
        };
        descriptor.ok_or(Error::NotRunnable)
    }
    fn prepare_read(&self, key: &Key, role: Role) -> Result<ReadRequest, Error> {
        let descriptor = self.read_descriptor(key, &role)?;
        Ok(ReadRequest {
            epoch: self.epoch,
            record: self.inspect(key)?.clone(),
            descriptor,
            role,
        })
    }
    pub(crate) fn complete_read(
        &self,
        completion: ReadCompletion,
    ) -> Result<ReadPublication, Error> {
        let ReadCompletion { request, outcome } = completion;
        self.check_available()?;
        if request.epoch != self.epoch {
            return Err(Error::StaleCompletion);
        }
        let record = self.inspect(request.key())?;
        if record != &request.record {
            return Err(Error::StaleCompletion);
        }
        if self.read_descriptor(request.key(), &request.role)? != request.descriptor
            || outcome.descriptor() != request.descriptor
        {
            return Err(Error::StaleCompletion);
        }
        let key = request.record.key().clone();
        Ok(match request.role {
            Role::Direct { .. } => {
                ReadPublication::Direct(DirectPublicationPermit { key, outcome })
            }
            Role::Task => ReadPublication::Task(ResultPublicationPermit { key, outcome }),
            Role::Holder(membership) => ReadPublication::Holder(HolderPublicationPermit {
                key,
                membership,
                outcome,
            }),
        })
    }
}
