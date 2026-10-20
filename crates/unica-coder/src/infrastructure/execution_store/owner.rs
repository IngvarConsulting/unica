//! Prototype composition: one decision owner and one physical worker. There
//! is at most one in-flight storage command, outside the model's decision call.
//! This is deliberately not a production transport or a deadline/SLA policy.
use super::file::{OpeningStore, ScanProgress};
use super::{CommitStep, FileAggregateStore, OpenError, ReadStep, Rejection};
use crate::application::execution::hydration::{ReadCompletion, ReadPublication, ReadRequest};
use crate::application::execution::{
    AcceptedPermit, AcceptedStoreAuthority, AuthorityFailureCause, CommitRequest, Effect,
    ExecutionCoordinator, OutcomeEvidence, OwnerCapability,
};
use crate::domain::execution::{Error, Key, ObservationWindow, ReadAuthority};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::{self, JoinHandle};

enum Command {
    Commit(Box<CommitRequest>),
    #[cfg(test)]
    CommitWithHook(Box<CommitRequest>, TestHook),
    Read(Box<ReadRequest>),
    Close,
}
enum Completion {
    Commit(Box<CommitStep>),
    Read(Box<ReadStep>),
    Closed(Result<(), OpenError>),
}

#[cfg(test)]
type TestHook = Box<
    dyn FnMut(
            crate::infrastructure::platform::filesystem::execution_store::PublicationPoint,
        ) -> std::io::Result<()>
        + Send,
>;

#[derive(Debug)]
pub(crate) enum OwnerError {
    Model(Error),
    Open(OpenError),
    WorkerUnavailable,
    EpochMismatch,
    NotQuiescent,
    RejectedCommit {
        request: Box<CommitRequest>,
        reason: Rejection,
    },
    RejectedRead {
        request: Box<ReadRequest>,
        reason: Rejection,
    },
    ObsoleteRead {
        key: Box<Key>,
        observed_revision: u64,
    },
}
impl From<Error> for OwnerError {
    fn from(error: Error) -> Self {
        Self::Model(error)
    }
}

pub(crate) struct PrototypeStoreOwner {
    model: ExecutionCoordinator,
    authority: AcceptedStoreAuthority,
    commands: Option<SyncSender<Command>>,
    completions: Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    dispatch_open: bool,
    loaded_reads: usize,
}

impl PrototypeStoreOwner {
    /// The only public binding route owns validation and recovery. A caller
    /// cannot discard an existing catalog and substitute an empty model.
    pub(crate) fn finish_opening(mut opening: OpeningStore) -> Result<Self, OwnerError> {
        while opening.scan_next().map_err(OwnerError::Open)? != ScanProgress::Complete {}
        let (store, catalog) = opening.finish().map_err(OwnerError::Open)?;
        let epoch = store.authority_epoch();
        let (model, effects) = ExecutionCoordinator::restore(catalog, epoch)?;
        let mut owner = Self::bind_recovered(model, store)?;
        for effect in effects {
            let Effect::Commit(request) = effect else {
                owner.fail(AuthorityFailureCause::InvalidAggregate);
                return Err(OwnerError::Model(Error::InvalidRecord));
            };
            let completed = owner.commit(request)?;
            if !completed.is_empty() || owner.model.unavailable() {
                owner.fail(AuthorityFailureCause::InvalidAggregate);
                return Err(OwnerError::Model(Error::StoreUnavailable));
            }
        }
        if owner.model.recovering() {
            owner.fail(AuthorityFailureCause::InvalidAggregate);
            return Err(OwnerError::Model(Error::RecoveryRequired));
        }
        Ok(owner)
    }
    fn bind_recovered(
        model: ExecutionCoordinator,
        mut store: FileAggregateStore,
    ) -> Result<Self, OwnerError> {
        let authority = model.store_authority();
        if authority.authority_epoch() != store.authority_epoch() {
            return Err(OwnerError::EpochMismatch);
        }
        if model.unavailable() || store.is_frozen() {
            return Err(OwnerError::Model(Error::StoreUnavailable));
        }
        let (commands, incoming) = mpsc::sync_channel(1);
        let (outgoing, completions) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("execution-store-prototype".into())
            .spawn(move || {
                while let Ok(command) = incoming.recv() {
                    let completion = match command {
                        Command::Commit(request) => Completion::Commit(Box::new(store.commit(*request))),
                    #[cfg(test)]
                    Command::CommitWithHook(request, hook) => {
                        let _guard = crate::infrastructure::platform::filesystem::execution_store::install_publication_hook(hook);
                        Completion::Commit(Box::new(store.commit(*request)))
                    }
                        Command::Read(request) => Completion::Read(Box::new(store.read(*request))),
                        Command::Close => {
                            let result = store.close_quiescent();
                            let _ = outgoing.send(Completion::Closed(result));
                            return;
                        }
                    };
                    if outgoing.send(completion).is_err() {
                        // Dropping the store freezes its already-installed keeper.
                        // No worker shutdown implicitly authorizes OS lease release.
                        return;
                    }
                }
            })
            .map_err(|error| OwnerError::Open(OpenError::Io(error)))?;
        Ok(Self {
            model,
            authority,
            commands: Some(commands),
            completions,
            worker: Some(worker),
            dispatch_open: true,
            loaded_reads: 0,
        })
    }

    pub(crate) fn model(&self) -> &ExecutionCoordinator {
        &self.model
    }
    pub(crate) fn submit(
        &mut self,
        key: Key,
        owner: &OwnerCapability,
        window: ObservationWindow,
    ) -> Result<Vec<Effect>, Error> {
        self.model.submit(key, owner, window)
    }
    pub(crate) fn begin(
        &mut self,
        permit: AcceptedPermit,
        at: u64,
        authority: Option<ReadAuthority>,
    ) -> Result<Vec<Effect>, Error> {
        self.model.begin(permit, at, authority)
    }
    pub(crate) fn report(&mut self, evidence: OutcomeEvidence) -> Result<Vec<Effect>, Error> {
        // Facts from a worker that was already authorized must survive freeze.
        self.model.report(evidence)
    }
    pub(crate) fn cancel(&mut self, key: &Key) -> Result<Vec<Effect>, Error> {
        self.model.cancel(key)
    }
    pub(crate) fn expose_task(&mut self, key: &Key) -> Result<Vec<Effect>, Error> {
        self.model.expose_task(key)
    }
    pub(crate) fn acknowledge(
        &mut self,
        key: &Key,
        owner: &OwnerCapability,
        digest: [u8; 32],
        at: u64,
    ) -> Result<Vec<Effect>, Error> {
        self.model.acknowledge(key, owner, digest, at)
    }

    fn fail(&mut self, cause: AuthorityFailureCause) {
        self.dispatch_open = false;
        // The binding was validated against the physical store at construction.
        self.model
            .apply_authority_failure(self.authority.failed(cause))
            .expect("same owner epoch");
    }
    fn exchange(&mut self, command: Command) -> Result<Completion, OwnerError> {
        if !self.dispatch_open || self.model.unavailable() {
            return Err(OwnerError::Model(Error::StoreUnavailable));
        }
        let sent = self
            .commands
            .as_ref()
            .is_some_and(|sender| sender.send(command).is_ok());
        if !sent {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        }
        match self.completions.recv() {
            Ok(completion) => Ok(completion),
            Err(_) => {
                self.fail(AuthorityFailureCause::WorkerFailed);
                Err(OwnerError::WorkerUnavailable)
            }
        }
    }
    fn check_commit_route(&self, request: &CommitRequest) -> Option<Rejection> {
        if request.authority_epoch() != self.authority.authority_epoch() {
            Some(Rejection::ForeignEpoch)
        } else if !self.model.owns_commit_request(request) {
            Some(Rejection::UnownedRequest)
        } else {
            None
        }
    }
    pub(crate) fn commit(&mut self, request: CommitRequest) -> Result<Vec<Effect>, OwnerError> {
        if let Some(reason) = self.check_commit_route(&request) {
            return Err(OwnerError::RejectedCommit {
                request: Box::new(request),
                reason,
            });
        }
        let completion = self.exchange(Command::Commit(Box::new(request)))?;
        self.apply_commit(completion)
    }
    #[cfg(test)]
    pub(crate) fn commit_with_publication_hook(
        &mut self,
        request: CommitRequest,
        hook: impl FnMut(
                crate::infrastructure::platform::filesystem::execution_store::PublicationPoint,
            ) -> std::io::Result<()>
            + Send
            + 'static,
    ) -> Result<Vec<Effect>, OwnerError> {
        if let Some(reason) = self.check_commit_route(&request) {
            return Err(OwnerError::RejectedCommit {
                request: Box::new(request),
                reason,
            });
        }
        let completion =
            self.exchange(Command::CommitWithHook(Box::new(request), Box::new(hook)))?;
        self.apply_commit(completion)
    }
    fn apply_commit(&mut self, completion: Completion) -> Result<Vec<Effect>, OwnerError> {
        let Completion::Commit(step) = completion else {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        };
        match *step {
            CommitStep::Completed(completion) => {
                let result = self.model.complete(completion);
                if result.is_err() {
                    // The physical owner accepted this write, so a rejected
                    // completion is a broken binding, not a harmless peer retry.
                    self.fail(AuthorityFailureCause::InvalidAggregate);
                }
                if self.model.unavailable() {
                    self.dispatch_open = false;
                }
                result.map_err(OwnerError::Model)
            }
            CommitStep::AuthorityFailed { request, cause } => {
                self.fail(cause);
                // Model still owns the unconfirmed candidate and queued facts.
                drop(request);
                Ok(vec![Effect::FailStop])
            }
            CommitStep::Rejected { request, reason } => Err(OwnerError::RejectedCommit {
                request: Box::new(request),
                reason,
            }),
        }
    }

    fn read_loaded(&mut self, request: ReadRequest) -> Result<ReadCompletion, OwnerError> {
        let rejection = if request.authority_epoch() != self.authority.authority_epoch() {
            Some(Rejection::ForeignEpoch)
        } else if !self.model.owns_read_request(&request) {
            Some(Rejection::UnownedRequest)
        } else {
            None
        };
        if let Some(reason) = rejection {
            return Err(OwnerError::RejectedRead {
                request: Box::new(request),
                reason,
            });
        }

        let Completion::Read(step) = self.exchange(Command::Read(Box::new(request)))? else {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        };
        match *step {
            ReadStep::Loaded(completion) => {
                self.loaded_reads += 1;
                Ok(completion)
            }
            ReadStep::AuthorityFailed(failure) => {
                if failure.authority_epoch() != self.authority.authority_epoch() {
                    return Err(OwnerError::Model(Error::StaleCompletion));
                }
                // Close physical dispatch before applying the fault, then no
                // delayed Loaded/Committed completion can grant fresh rights.
                self.dispatch_open = false;
                self.model.apply_authority_failure(failure)?;
                Err(OwnerError::Model(Error::StoreUnavailable))
            }
            ReadStep::Rejected { request, reason } => Err(OwnerError::RejectedRead {
                request: Box::new(request),
                reason,
            }),
            ReadStep::Obsolete {
                key,
                observed_revision,
            } => Err(OwnerError::ObsoleteRead {
                key: Box::new(key),
                observed_revision,
            }),
        }
    }
    fn publish_loaded(
        &mut self,
        completion: ReadCompletion,
    ) -> Result<ReadPublication, OwnerError> {
        if self.loaded_reads == 0 {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        }
        self.loaded_reads -= 1;
        self.model
            .complete_read(completion)
            .map_err(OwnerError::Model)
    }
    pub(crate) fn read(&mut self, request: ReadRequest) -> Result<ReadPublication, OwnerError> {
        let loaded = self.read_loaded(request)?;
        self.publish_loaded(loaded)
    }

    /// Successful calls have consumed all worker completions; the public API
    /// cannot retain a raw ReadCompletion. Refuse while the model still owns
    /// a pending commit or an execution lacking a producer outcome.
    pub(crate) fn close_quiescent(mut self) -> Result<(), OwnerError> {
        if !self.model.storage_is_quiescent() || self.loaded_reads != 0 {
            return Err(OwnerError::NotQuiescent);
        }
        let Completion::Closed(result) = self.exchange(Command::Close)? else {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        };
        self.dispatch_open = false;
        self.commands.take();
        if self
            .worker
            .take()
            .is_none_or(|worker| worker.join().is_err())
        {
            self.fail(AuthorityFailureCause::WorkerFailed);
            return Err(OwnerError::WorkerUnavailable);
        }
        result.map_err(OwnerError::Open)
    }
}

// No custom Drop releases the store. An implicit drop closes the channel and
// detaches the worker; its store freezes the process keeper even after unwind.

#[cfg(test)]
pub(crate) mod test_support;
