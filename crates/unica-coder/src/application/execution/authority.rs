//! A physical store fault stops this exact owner before any later completion
//! can grant rights. Diagnostic read revisions do not fence authority loss.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AuthorityFailureCause {
    InvalidAggregate,
    IdentityLost,
    ReadFailed,
    WorkerFailed,
}

/// Not a deserializable failure assertion from a peer. Only an accepted store
/// binding or a consumed read request can report loss of this authority.
#[derive(Debug)]
pub(crate) struct AuthorityFailure {
    epoch: AuthorityEpoch,
    cause: AuthorityFailureCause,
    read: Option<(Key, u64)>,
}
impl AuthorityFailure {
    pub(super) fn for_read(
        epoch: AuthorityEpoch,
        cause: AuthorityFailureCause,
        key: Key,
        revision: u64,
    ) -> Self {
        Self {
            epoch,
            cause,
            read: Some((key, revision)),
        }
    }
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
    pub(crate) fn cause(&self) -> AuthorityFailureCause {
        self.cause
    }
    pub(crate) fn read_context(&self) -> Option<(&Key, u64)> {
        self.read.as_ref().map(|(key, revision)| (key, *revision))
    }
}

/// This binding belongs to one coordinator epoch. It does not hold or release
/// an OS lease and cannot assert that a commit was or was not published.
#[derive(Debug)]
pub(crate) struct AcceptedStoreAuthority {
    epoch: AuthorityEpoch,
}
impl AcceptedStoreAuthority {
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
    pub(crate) fn failed(&self, cause: AuthorityFailureCause) -> AuthorityFailure {
        AuthorityFailure {
            epoch: self.epoch,
            cause,
            read: None,
        }
    }
}

impl ExecutionCoordinator {
    pub(crate) fn store_authority(&self) -> AcceptedStoreAuthority {
        AcceptedStoreAuthority { epoch: self.epoch }
    }
    /// Routing check before physical dispatch. Epoch equality alone cannot
    /// establish that this model owns the exact pending transition.
    pub(crate) fn owns_commit_request(&self, request: &CommitRequest) -> bool {
        request.stamp.epoch == self.epoch
            && self
                .groups
                .get(&request.stamp.key.invocation_id())
                .and_then(|slot| slot.pending.as_ref())
                .is_some_and(|pending| pending.stamp == request.stamp)
    }
    pub(crate) fn owns_read_request(&self, request: &hydration::ReadRequest) -> bool {
        request.authority_epoch() == self.epoch
            && self
                .groups
                .get(&request.key().invocation_id())
                .is_some_and(|slot| slot.confirmed.record().key() == request.key())
    }
    pub(crate) fn first_authority_failure(&self) -> Option<&AuthorityFailure> {
        self.first_authority_failure.as_ref()
    }
    /// Necessary model-side condition for an explicit storage-owner close.
    /// The physical owner must also drain every I/O and completion. Drop or a
    /// timeout is not this proof; fail-stop owners never qualify for release.
    pub(crate) fn storage_is_quiescent(&self) -> bool {
        !self.unavailable
            && !self.recovering
            && self.groups.values().all(|slot| {
                slot.pending.is_none()
                    && slot.queue.is_empty()
                    && (slot.confirmed.record().run().is_none()
                        || slot.confirmed.record().producer().is_some())
            })
    }
    pub(crate) fn apply_authority_failure(
        &mut self,
        failure: AuthorityFailure,
    ) -> Result<Vec<Effect>, Error> {
        // A stale read revision still witnessed a physical failure of this
        // owner. Only a foreign epoch is ignored, before any state mutation.
        if failure.epoch != self.epoch {
            return Err(Error::StaleCompletion);
        }
        if self.unavailable {
            return Ok(Vec::new());
        }
        self.first_authority_failure = Some(failure);
        self.unavailable = true;
        // Pending and queued factual outcomes remain owned by the model.
        // Neither a fault nor a later callback acknowledges their commit.
        Ok(vec![Effect::FailStop])
    }
}
