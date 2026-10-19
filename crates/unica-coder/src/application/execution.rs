//! P1 application owner. Default-compiled but not connected to the runtime until
//! P3. The storage driver consumes commit requests outside this decision loop.
use crate::domain::execution::storage::{PayloadDelta, RecoveryCatalog, StoredRecord};
use crate::domain::execution::{
    self as model, CanonicalOutcome, Error, Event, Intent, JoinEvidence, Key, Membership,
    ObservationWindow, ReadAuthority, Record, RunIdentity, State,
};
use crate::domain::invocation::{InvocationId, TaskId};
use std::collections::{HashMap, VecDeque};
pub(crate) mod hydration;

/// A frontend secret. Debug and durable projections never expose its bytes.
pub(crate) struct OwnerCapability([u8; 32]);
impl OwnerCapability {
    pub(crate) fn new(secret: [u8; 32]) -> Self {
        Self(secret)
    }
    fn hash(&self) -> [u8; 32] {
        model::digest(&self.0)
    }
}
impl std::fmt::Debug for OwnerCapability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OwnerCapability([redacted])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthorityEpoch([u8; 32]);
impl AuthorityEpoch {
    pub(crate) fn new(value: [u8; 32]) -> Self {
        Self(value)
    }
}

/// Non-Clone, non-deserializable rights, produced only by confirmed transitions.
#[derive(Debug)]
pub(crate) struct AcceptedPermit {
    key: Key,
    epoch: AuthorityEpoch,
}
impl AcceptedPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn reject(
        self,
        result: crate::domain::invocation::DomainResult,
    ) -> Result<AdmissionRejection, Error> {
        if result.ok {
            return Err(Error::InvalidInput);
        }
        Ok(AdmissionRejection {
            key: self.key,
            epoch: self.epoch,
            outcome: CanonicalOutcome::completed(result),
        })
    }
}
#[derive(Debug)]
pub(crate) struct AdmissionRejection {
    key: Key,
    epoch: AuthorityEpoch,
    outcome: CanonicalOutcome,
}
#[derive(Debug)]
pub(crate) struct RunPermit {
    key: Key,
    run: RunIdentity,
    epoch: AuthorityEpoch,
}
impl RunPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn run_identity(&self) -> RunIdentity {
        self.run
    }
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
    pub(crate) fn finish(self, outcome: CanonicalOutcome) -> OutcomeEvidence {
        OutcomeEvidence {
            key: self.key,
            run: self.run,
            epoch: self.epoch,
            outcome,
        }
    }
}
#[derive(Debug)]
pub(crate) struct OutcomeEvidence {
    key: Key,
    run: RunIdentity,
    epoch: AuthorityEpoch,
    outcome: CanonicalOutcome,
}
#[derive(Debug)]
pub(crate) struct TaskPublicationPermit {
    key: Key,
}
#[derive(Debug)]
pub(crate) struct DirectPublicationPermit {
    key: Key,
    outcome: CanonicalOutcome,
}
#[derive(Debug)]
pub(crate) struct CancellationPermit {
    key: Key,
    epoch: AuthorityEpoch,
}
impl CancellationPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.epoch
    }
}
impl TaskPublicationPermit {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
}

#[derive(Debug)]
pub(crate) enum Effect {
    Commit(CommitRequest),
    Accepted(AcceptedPermit),
    Run(RunPermit),
    Task(TaskPublicationPermit),
    Attached(Membership),
    CancelRequested(CancellationPermit),
    FailStop,
    Rejected { key: Key, error: Error },
}

/// The adapter is the trust boundary for known commit/rollback/readback. It can
/// inspect only the durable candidate, never the group's volatile membership.
#[derive(Debug)]
pub(crate) struct CommitRequest {
    stamp: Stamp,
    candidate: Box<Record>,
    payloads: PayloadDelta,
}
impl CommitRequest {
    pub(crate) fn key(&self) -> &Key {
        &self.stamp.key
    }
    pub(crate) fn id(&self) -> u64 {
        self.stamp.id
    }
    pub(crate) fn authority_epoch(&self) -> AuthorityEpoch {
        self.stamp.epoch
    }
    pub(crate) fn expected_revision(&self) -> u64 {
        self.stamp.expected_revision
    }
    pub(crate) fn candidate(&self) -> &Record {
        &self.candidate
    }
    pub(crate) fn candidate_fingerprint(&self) -> [u8; 32] {
        self.stamp.fingerprint
    }
    pub(crate) fn assemble(&self, prior: Option<&StoredRecord>) -> Result<StoredRecord, Error> {
        if prior.map(|p| p.record().fingerprint()) != self.stamp.previous_fingerprint {
            return Err(Error::InvalidRecord);
        }
        self.payloads.assemble(*self.candidate.clone(), prior)
    }
    pub(crate) fn payloads(&self) -> &PayloadDelta {
        &self.payloads
    }
    pub(crate) fn committed(self) -> CommitCompletion {
        CommitCompletion {
            stamp: self.stamp,
            resolution: Resolution::Committed,
        }
    }
    pub(crate) fn rolled_back(self) -> CommitCompletion {
        CommitCompletion {
            stamp: self.stamp,
            resolution: Resolution::RolledBack,
        }
    }
    pub(crate) fn unknown(self) -> CommitCompletion {
        CommitCompletion {
            stamp: self.stamp,
            resolution: Resolution::Unknown,
        }
    }
    pub(crate) fn readback(self, record: StoredRecord) -> CommitCompletion {
        CommitCompletion {
            stamp: self.stamp,
            resolution: Resolution::Readback(Box::new(record)),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    key: Key,
    epoch: AuthorityEpoch,
    id: u64,
    expected_revision: u64,
    previous_fingerprint: Option<[u8; 32]>,
    fingerprint: [u8; 32],
}
#[derive(Debug)]
enum Resolution {
    Committed,
    RolledBack,
    Readback(Box<StoredRecord>),
    Unknown,
}
#[derive(Debug)]
pub(crate) struct CommitCompletion {
    stamp: Stamp,
    resolution: Resolution,
}
#[derive(Debug)]
struct Pending {
    stamp: Stamp,
    event: Event,
    candidate: State,
    intents: Vec<Intent>,
}
#[derive(Debug)]
struct Slot {
    confirmed: State,
    pending: Option<Pending>,
    queue: VecDeque<Event>,
}

#[derive(Debug)]
pub(crate) struct ExecutionCoordinator {
    epoch: AuthorityEpoch,
    next_commit: u64,
    unavailable: bool,
    recovering: bool,
    groups: HashMap<InvocationId, Slot>,
    // Both indexes claim the complete key, including pending and cancel-only
    // records. Compaction never releases either claim.
    invocations: HashMap<InvocationId, Key>,
    tasks: HashMap<TaskId, Key>,
    aliases: HashMap<TaskId, Key>,
    alias_invocations: HashMap<InvocationId, TaskId>,
    alias_capacity: usize,
}
impl ExecutionCoordinator {
    pub(crate) fn new(epoch: AuthorityEpoch) -> Self {
        Self {
            epoch,
            next_commit: 0,
            unavailable: false,
            recovering: false,
            groups: HashMap::new(),
            invocations: HashMap::new(),
            tasks: HashMap::new(),
            aliases: HashMap::new(),
            alias_invocations: HashMap::new(),
            alias_capacity: 1_024,
        }
    }
    pub(crate) fn restore(
        catalog: RecoveryCatalog,
        epoch: AuthorityEpoch,
    ) -> Result<(Self, Vec<Effect>), Error> {
        // Only a completely validated stream can construct this catalog.
        let records = catalog.into_records();
        let mut owner = Self::new(epoch);
        let mut effects = Vec::new();
        for record in records {
            let key = record.key().clone();
            owner.claim(&key)?;
            owner.groups.insert(
                key.invocation_id(),
                Slot {
                    confirmed: State::from_record(record)?,
                    pending: None,
                    queue: VecDeque::new(),
                },
            );
        }
        owner.recovering = true;
        let keys: Vec<_> = owner.invocations.values().cloned().collect();
        for key in keys {
            effects.extend(owner.enqueue(&key, Event::Recover, true)?);
        }
        owner.finish_recovery();
        Ok((owner, effects))
    }
    /// Compact comparison data, never sufficient to recover a durable store.
    pub(crate) fn snapshot(&self) -> Vec<Record> {
        let mut records: Vec<_> = self
            .groups
            .values()
            .map(|s| s.confirmed.record().clone())
            .filter(|r| r.revision() > 0)
            .collect();
        records.sort_by_key(|r| r.key().digest());
        records
    }
    pub(crate) fn inspect(&self, key: &Key) -> Result<&Record, Error> {
        let slot = self
            .groups
            .get(&key.invocation_id())
            .ok_or(Error::NotAccepted)?;
        if slot.confirmed.record().key() != key {
            return Err(Error::IdentityCollision);
        }
        if slot.confirmed.record().revision() == 0 {
            return Err(Error::NotAccepted);
        }
        Ok(slot.confirmed.record())
    }
    /// Accepted lookup is durable; mere early cancellation is not a Task promise.
    pub(crate) fn lookup_task(&self, task: TaskId) -> Result<&Record, Error> {
        self.check_available()?;
        let key = self.tasks.get(&task).ok_or(Error::NotAccepted)?;
        let record = self.inspect(key)?;
        if !record.accepted() {
            return Err(Error::NotAccepted);
        }
        Ok(record)
    }
    pub(crate) fn holder_count(&self, key: &Key) -> usize {
        self.groups
            .get(&key.invocation_id())
            .map_or(0, |s| s.confirmed.holder_count())
    }
    pub(crate) fn observe_holder(
        &self,
        key: &Key,
        member: Membership,
    ) -> Result<model::HolderObservation<'_>, Error> {
        self.check_available()?;
        let slot = self
            .groups
            .get(&key.invocation_id())
            .ok_or(Error::NotAccepted)?;
        if slot.confirmed.record().key() != key {
            return Err(Error::IdentityCollision);
        }
        slot.confirmed.observe_holder(member)
    }
    fn check_available(&self) -> Result<(), Error> {
        if self.unavailable {
            Err(Error::StoreUnavailable)
        } else if self.recovering {
            Err(Error::RecoveryRequired)
        } else {
            Ok(())
        }
    }
    pub(crate) fn unavailable(&self) -> bool {
        self.unavailable
    }
    pub(crate) fn recovering(&self) -> bool {
        self.recovering
    }

    pub(crate) fn submit(
        &mut self,
        key: Key,
        owner: &OwnerCapability,
        window: ObservationWindow,
    ) -> Result<Vec<Effect>, Error> {
        self.enqueue(
            &key,
            Event::Submit {
                owner_hash: owner.hash(),
                window,
            },
            false,
        )
    }
    pub(crate) fn cancel(&mut self, key: &Key) -> Result<Vec<Effect>, Error> {
        self.enqueue(key, Event::Cancel, false)
    }
    pub(crate) fn begin(
        &mut self,
        permit: AcceptedPermit,
        started_epoch_ms: u64,
        read_authority: Option<ReadAuthority>,
    ) -> Result<Vec<Effect>, Error> {
        if permit.epoch != self.epoch {
            return Err(Error::ForeignOutcome);
        }
        self.enqueue(
            &permit.key,
            Event::Begin {
                started_epoch_ms,
                read_authority,
            },
            false,
        )
    }
    pub(crate) fn report(&mut self, evidence: OutcomeEvidence) -> Result<Vec<Effect>, Error> {
        if evidence.epoch != self.epoch {
            return Err(Error::ForeignOutcome);
        }
        if self.unavailable {
            let slot = self
                .groups
                .get_mut(&evidence.key.invocation_id())
                .ok_or(Error::ForeignOutcome)?;
            if slot.confirmed.record().key() != &evidence.key
                || slot.confirmed.record().run() != Some(evidence.run)
            {
                return Err(Error::ForeignOutcome);
            }
            // A permitted worker may finish after metadata persistence failed.
            // Retain its volatile evidence without driving commits or reopening
            // admission. This is not a durable crash/recovery guarantee.
            slot.queue.push_back(Event::Outcome {
                run: evidence.run,
                outcome: evidence.outcome,
            });
            return Ok(vec![Effect::FailStop]);
        }
        self.enqueue(
            &evidence.key,
            Event::Outcome {
                run: evidence.run,
                outcome: evidence.outcome,
            },
            false,
        )
    }
    pub(crate) fn reject_admission(
        &mut self,
        evidence: AdmissionRejection,
    ) -> Result<Vec<Effect>, Error> {
        if evidence.epoch != self.epoch {
            return Err(Error::ForeignOutcome);
        }
        self.enqueue(
            &evidence.key,
            Event::AdmissionRejected(evidence.outcome),
            false,
        )
    }
    pub(crate) fn expose_task(&mut self, key: &Key) -> Result<Vec<Effect>, Error> {
        self.enqueue(key, Event::ExposeTask, false)
    }
    pub(crate) fn acknowledge(
        &mut self,
        key: &Key,
        owner: &OwnerCapability,
        primary_digest: [u8; 32],
        acknowledged_epoch_ms: u64,
    ) -> Result<Vec<Effect>, Error> {
        self.enqueue(
            key,
            Event::Acknowledge {
                owner_hash: owner.hash(),
                primary_digest,
                acknowledged_epoch_ms,
            },
            false,
        )
    }
    pub(crate) fn attach(
        &mut self,
        key: &Key,
        evidence: JoinEvidence,
    ) -> Result<Vec<Effect>, Error> {
        if self.tasks.contains_key(&evidence.alias_id())
            || self.aliases.contains_key(&evidence.alias_id())
            || self
                .invocations
                .contains_key(&evidence.request().invocation_id())
            || self
                .alias_invocations
                .contains_key(&evidence.request().invocation_id())
        {
            return Err(Error::IdentityCollision);
        }
        self.check_available()?;
        if self.aliases.len() >= self.alias_capacity {
            return Err(Error::AliasCapacity);
        }
        // Tentative alias claims also prevent concurrent attaches to different
        // groups. They are never part of the durable catalog.
        self.aliases.insert(evidence.alias_id(), key.clone());
        self.alias_invocations
            .insert(evidence.request().invocation_id(), evidence.alias_id());
        let alias = evidence.alias_id();
        let result = self.enqueue(key, Event::Attach(evidence), false);
        if result.is_err() {
            self.release_alias(alias);
        }
        result
    }
    pub(crate) fn cancel_holder(
        &mut self,
        key: &Key,
        member: Membership,
    ) -> Result<Vec<Effect>, Error> {
        self.enqueue(key, Event::CancelHolder(member), false)
    }
    pub(crate) fn retire_holder(
        &mut self,
        key: &Key,
        member: Membership,
    ) -> Result<Vec<Effect>, Error> {
        self.enqueue(key, Event::RetireHolder(member), false)
    }

    fn claim(&mut self, key: &Key) -> Result<(), Error> {
        if self
            .invocations
            .get(&key.invocation_id())
            .is_some_and(|k| k != key)
            || self.tasks.get(&key.task_id()).is_some_and(|k| k != key)
            || self.aliases.contains_key(&key.task_id())
            || self.alias_invocations.contains_key(&key.invocation_id())
        {
            return Err(Error::IdentityCollision);
        }
        self.invocations.insert(key.invocation_id(), key.clone());
        self.tasks.insert(key.task_id(), key.clone());
        Ok(())
    }
    fn enqueue(&mut self, key: &Key, event: Event, recovery: bool) -> Result<Vec<Effect>, Error> {
        if self.unavailable {
            return Err(Error::StoreUnavailable);
        }
        if self.recovering && !recovery {
            return Err(Error::RecoveryRequired);
        }
        if !self.groups.contains_key(&key.invocation_id())
            && !matches!(event, Event::Submit { .. } | Event::Cancel)
        {
            return Err(Error::NotAccepted);
        }
        self.claim(key)?;
        let slot = self
            .groups
            .entry(key.invocation_id())
            .or_insert_with(|| Slot {
                confirmed: State::empty(key.clone()),
                pending: None,
                queue: VecDeque::new(),
            });
        slot.queue.push_back(event);
        self.drive(key)
    }
    fn drive(&mut self, key: &Key) -> Result<Vec<Effect>, Error> {
        let mut effects = Vec::new();
        let slot = self
            .groups
            .get_mut(&key.invocation_id())
            .ok_or(Error::NotAccepted)?;
        if slot.pending.is_some() {
            return Ok(effects);
        }
        while let Some(event) = slot.queue.pop_front() {
            match slot.confirmed.decide(event.clone()) {
                Ok(Some(transition)) => {
                    let (candidate, intents, payloads) = transition.into_parts();
                    self.next_commit = self.next_commit.checked_add(1).ok_or(Error::Overflow)?;
                    let stamp = Stamp {
                        key: key.clone(),
                        epoch: self.epoch,
                        id: self.next_commit,
                        expected_revision: slot.confirmed.record().revision(),
                        previous_fingerprint: (slot.confirmed.record().revision() > 0)
                            .then(|| slot.confirmed.record().fingerprint()),
                        fingerprint: candidate.record().fingerprint(),
                    };
                    effects.push(Effect::Commit(CommitRequest {
                        stamp: stamp.clone(),
                        candidate: Box::new(candidate.record().clone()),
                        payloads,
                    }));
                    slot.pending = Some(Pending {
                        stamp,
                        event,
                        candidate,
                        intents,
                    });
                    break;
                }
                Ok(None) => {
                    // A repeated exposure observes an already confirmed promise.
                    if matches!(event, Event::ExposeTask) {
                        effects.push(Effect::Task(TaskPublicationPermit { key: key.clone() }));
                    }
                }
                Err(error) => {
                    if matches!(event, Event::Outcome { .. }) {
                        slot.queue.push_front(event);
                        self.unavailable = true;
                        effects.push(Effect::FailStop);
                        break;
                    }
                    if let Event::Attach(e) = event {
                        self.aliases.remove(&e.alias_id());
                        self.alias_invocations.retain(|_, a| *a != e.alias_id());
                    }
                    effects.push(Effect::Rejected {
                        key: key.clone(),
                        error,
                    });
                }
            }
        }
        Ok(effects)
    }
    pub(crate) fn complete(&mut self, completion: CommitCompletion) -> Result<Vec<Effect>, Error> {
        if self.unavailable {
            return Err(Error::StoreUnavailable);
        }
        let key = completion.stamp.key.clone();
        let slot = self
            .groups
            .get_mut(&key.invocation_id())
            .ok_or(Error::StaleCompletion)?;
        if slot.pending.as_ref().map(|p| &p.stamp) != Some(&completion.stamp)
            || completion.stamp.epoch != self.epoch
        {
            return Err(Error::StaleCompletion);
        }
        // Unknown preserves the candidate and queued factual outcome for this
        // authority's fail-stop. No callback can reopen admission afterward.
        if matches!(completion.resolution, Resolution::Unknown) {
            self.unavailable = true;
            return Ok(vec![Effect::FailStop]);
        }
        let mut committed = matches!(completion.resolution, Resolution::Committed);
        if let Resolution::Readback(ref actual) = completion.resolution {
            if actual.record().key() != &key {
                self.unavailable = true;
                return Ok(vec![Effect::FailStop]);
            }
            let pending = slot.pending.as_ref().expect("checked pending");
            committed = actual.record() == pending.candidate.record();
            if !committed
                && slot
                    .confirmed
                    .with_readback(actual.record().clone())
                    .is_err()
            {
                self.unavailable = true;
                return Ok(vec![Effect::FailStop]);
            }
        }
        let pending = slot.pending.take().expect("checked pending");
        let mut effects = Vec::new();
        if committed {
            if let Event::RetireHolder(member) = pending.event {
                self.aliases.remove(&member.alias_id());
                self.alias_invocations
                    .retain(|_, a| *a != member.alias_id());
            }
            slot.confirmed = pending.candidate;
            for intent in pending.intents {
                effects.push(match intent {
                    Intent::Accepted => Effect::Accepted(AcceptedPermit {
                        key: key.clone(),
                        epoch: self.epoch,
                    }),
                    Intent::Run(run) => Effect::Run(RunPermit {
                        key: key.clone(),
                        run,
                        epoch: self.epoch,
                    }),
                    Intent::Task => Effect::Task(TaskPublicationPermit { key: key.clone() }),
                    Intent::Attached(member) => Effect::Attached(member),
                    Intent::Cancel => Effect::CancelRequested(CancellationPermit {
                        key: key.clone(),
                        epoch: self.epoch,
                    }),
                });
            }
        } else {
            if let Resolution::Readback(actual) = completion.resolution {
                slot.confirmed = slot.confirmed.with_readback(actual.into_metadata())?;
            }
            // Retry the semantic fact, not a stale candidate revision. This is
            // especially important for Outcome queued behind Cancel or Expose.
            slot.queue.push_front(pending.event);
        }
        effects.extend(self.drive(&key)?);
        self.finish_recovery();
        Ok(effects)
    }
    fn finish_recovery(&mut self) {
        if self.recovering
            && self
                .groups
                .values()
                .all(|s| s.pending.is_none() && s.queue.is_empty())
        {
            self.recovering = false;
        }
    }
    fn release_alias(&mut self, alias: TaskId) {
        self.aliases.remove(&alias);
        self.alias_invocations.retain(|_, a| *a != alias);
    }
}

/// Frontend observation choice is independent of execution ownership. Building
/// a direct result is irreversible even when ACK transport later fails.
#[derive(Debug)]
pub(crate) struct DeliveryChoice {
    key: Key,
    state: DeliveryState,
}
#[derive(Debug)]
enum DeliveryState {
    Undecided,
    DirectBuilt(CanonicalOutcome),
    TaskReturned,
}
impl DeliveryChoice {
    pub(crate) fn new(key: Key) -> Self {
        Self {
            key,
            state: DeliveryState::Undecided,
        }
    }
    pub(crate) fn build_direct(&mut self, permit: DirectPublicationPermit) -> Result<(), Error> {
        if permit.key != self.key {
            return Err(Error::IdentityCollision);
        }
        if !matches!(self.state, DeliveryState::Undecided) {
            return Err(Error::InvalidInput);
        }
        self.state = DeliveryState::DirectBuilt(permit.outcome);
        Ok(())
    }
    pub(crate) fn return_task(&mut self, permit: TaskPublicationPermit) -> Result<(), Error> {
        if permit.key != self.key {
            return Err(Error::IdentityCollision);
        }
        if !matches!(self.state, DeliveryState::Undecided) {
            return Err(Error::InvalidInput);
        }
        self.state = DeliveryState::TaskReturned;
        Ok(())
    }
    pub(crate) fn direct(&self) -> Option<&CanonicalOutcome> {
        match &self.state {
            DeliveryState::DirectBuilt(o) => Some(o),
            _ => None,
        }
    }
    pub(crate) fn ack_digest(&self) -> Option<[u8; 32]> {
        self.direct().map(CanonicalOutcome::digest)
    }
}

#[cfg(test)]
mod acceptance_tests;
#[cfg(test)]
mod recovery_acceptance_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod test_support;

#[cfg(test)]
mod compact_tests;
