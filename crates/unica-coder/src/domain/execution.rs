//! Pure execution decisions. Persistence and authorization of effects belong to
//! application::execution; these values alone never authorize a worker.
use crate::domain::invocation::{DomainResult, InvocationId, TaskId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(crate) enum Tool {
    View,
    Apply,
    Resolve,
    Search,
    Check,
    Diff,
    Run,
    Docs,
}
impl Tool {
    pub(crate) fn is_read(self) -> bool {
        match self {
            Self::View | Self::Resolve | Self::Search | Self::Check | Self::Diff | Self::Docs => {
                true
            }
            Self::Apply | Self::Run => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Key {
    invocation: InvocationId,
    task: TaskId,
    core: [u8; 32],
    tool: Tool,
    arguments: [u8; 32],
    scope: [u8; 32],
}
impl Key {
    pub(crate) fn new(
        invocation: InvocationId,
        task: TaskId,
        core: [u8; 32],
        tool: Tool,
        arguments: [u8; 32],
        scope: [u8; 32],
    ) -> Self {
        Self {
            invocation,
            task,
            core,
            tool,
            arguments,
            scope,
        }
    }
    pub(crate) fn invocation_id(&self) -> InvocationId {
        self.invocation
    }
    pub(crate) fn task_id(&self) -> TaskId {
        self.task
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        digest(&serde_json::to_vec(self).expect("closed key serializes"))
    }
    fn same_request(&self, other: &Self) -> bool {
        self.core == other.core
            && self.tool == other.tool
            && self.arguments == other.arguments
            && self.scope == other.scope
    }
}
pub(crate) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AttemptId([u8; 32]);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunIdentity {
    attempt: AttemptId,
    generation: u64,
}
impl RunIdentity {
    pub(crate) fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FailureReason {
    InvocationFailed,
    ResultTooLarge,
    Interrupted,
    ResumeUnsupported,
    PersistenceFailed,
    OutcomeUncertain,
    WorkspaceChanged,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum OutcomeValue {
    Completed { result: Box<DomainResult> },
    Failed { reason: FailureReason },
    Cancelled,
}

/// Canonical bytes cannot be supplied independently of their typed outcome.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CanonicalOutcome {
    bytes: Vec<u8>,
    digest: [u8; 32],
}
impl<'de> Deserialize<'de> for CanonicalOutcome {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            bytes: Vec<u8>,
            digest: [u8; 32],
        }
        let encoded = Encoded::deserialize(decoder)?;
        let outcome = Self {
            bytes: encoded.bytes,
            digest: encoded.digest,
        };
        if !outcome.valid() {
            return Err(serde::de::Error::custom("invalid canonical outcome"));
        }
        Ok(outcome)
    }
}
impl std::fmt::Debug for CanonicalOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CanonicalOutcome")
            .field("bytes", &self.bytes.len())
            .field("digest", &self.digest)
            .finish()
    }
}
impl CanonicalOutcome {
    fn of(value: OutcomeValue) -> Self {
        let bytes = serde_json::to_vec(&value).expect("typed outcome serializes");
        Self {
            digest: digest(&bytes),
            bytes,
        }
    }
    pub(crate) fn completed(result: DomainResult) -> Self {
        Self::of(OutcomeValue::Completed {
            result: Box::new(result),
        })
    }
    pub(crate) fn failed(reason: FailureReason) -> Self {
        Self::of(OutcomeValue::Failed { reason })
    }
    pub(crate) fn cancelled() -> Self {
        Self::of(OutcomeValue::Cancelled)
    }
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
    fn valid(&self) -> bool {
        serde_json::from_slice::<OutcomeValue>(&self.bytes)
            .ok()
            .is_some_and(|v| Self::of(v) == *self)
    }
    fn is_admission_rejection(&self) -> bool {
        matches!(serde_json::from_slice::<OutcomeValue>(&self.bytes),
            Ok(OutcomeValue::Completed { result }) if !result.ok)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObservationWindow {
    received_epoch_ms: u64,
    budget_ms: u64,
}
impl<'de> Deserialize<'de> for ObservationWindow {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            received_epoch_ms: u64,
            budget_ms: u64,
        }
        let e = Encoded::deserialize(decoder)?;
        Self::new(e.received_epoch_ms, e.budget_ms)
            .map_err(|_| serde::de::Error::custom("invalid observation window"))
    }
}
impl ObservationWindow {
    pub(crate) fn new(received_epoch_ms: u64, budget_ms: u64) -> Result<Self, Error> {
        if budget_ms > 7_000 || received_epoch_ms.checked_add(budget_ms).is_none() {
            return Err(Error::InvalidInput);
        }
        Ok(Self {
            received_epoch_ms,
            budget_ms,
        })
    }
    pub(crate) fn end_epoch_ms(self) -> u64 {
        self.received_epoch_ms + self.budget_ms
    }
}

/// Evidence supplied by the source/actor adapter, not inferred from a Task id.
/// P1 compares this evidence; it does not perform a filesystem freshness walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadAuthority {
    source: [u8; 32],
    actor_generation: u64,
    mutation_generation: u64,
}
impl ReadAuthority {
    pub(crate) fn new(source: [u8; 32], actor_generation: u64, mutation_generation: u64) -> Self {
        Self {
            source,
            actor_generation,
            mutation_generation,
        }
    }
}
#[derive(Clone, Debug)]
pub(crate) struct JoinEvidence {
    request: Key,
    authority: ReadAuthority,
    observed_epoch_ms: u64,
}
impl JoinEvidence {
    pub(crate) fn new(request: Key, authority: ReadAuthority, observed_epoch_ms: u64) -> Self {
        Self {
            request,
            authority,
            observed_epoch_ms,
        }
    }
    pub(crate) fn alias_id(&self) -> TaskId {
        self.request.task
    }
    pub(crate) fn request(&self) -> &Key {
        &self.request
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Membership {
    alias: TaskId,
    generation: u64,
    producer: AttemptId,
}
impl Membership {
    pub(crate) fn alias_id(self) -> TaskId {
        self.alias
    }
    pub(crate) fn generation(self) -> u64 {
        self.generation
    }
}
#[derive(Clone, Debug)]
struct Holder {
    generation: u64,
    cancelled: bool,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HolderObservation<'a> {
    Working,
    Cancelled,
    Terminal(&'a CanonicalOutcome),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Accepted {
    owner_hash: [u8; 32],
    window: ObservationWindow,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Begun {
    run: RunIdentity,
    started_epoch_ms: u64,
    read_authority: Option<ReadAuthority>,
}

/// Immutable evidence of the first confirmed acknowledgement. Its key is the
/// containing record's key; this is evidence, not a writable capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AckRecord {
    digest: [u8; 32],
    first_acknowledged_epoch_ms: u64,
}
impl AckRecord {
    pub(crate) fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub(crate) fn first_acknowledged_epoch_ms(&self) -> u64 {
        self.first_acknowledged_epoch_ms
    }
}

/// The complete durable projection. It deliberately has no alias or holder
/// fields: serializing an entire live State is not an available operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Record {
    key: Key,
    revision: u64,
    accepted: Option<Accepted>,
    begun: Option<Begun>,
    cancel_requested: bool,
    producer: Option<CanonicalOutcome>,
    producer_digest: Option<[u8; 32]>,
    primary_override: Option<CanonicalOutcome>,
    primary_digest: Option<[u8; 32]>,
    acknowledgement: Option<AckRecord>,
    public_task: bool,
    compacted: bool,
}
impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            key: Key,
            revision: u64,
            accepted: Option<Accepted>,
            begun: Option<Begun>,
            cancel_requested: bool,
            producer: Option<CanonicalOutcome>,
            producer_digest: Option<[u8; 32]>,
            primary_override: Option<CanonicalOutcome>,
            primary_digest: Option<[u8; 32]>,
            acknowledgement: Option<AckRecord>,
            public_task: bool,
            compacted: bool,
        }
        let e = Encoded::deserialize(decoder)?;
        let record = Self {
            key: e.key,
            revision: e.revision,
            accepted: e.accepted,
            begun: e.begun,
            cancel_requested: e.cancel_requested,
            producer: e.producer,
            producer_digest: e.producer_digest,
            primary_override: e.primary_override,
            primary_digest: e.primary_digest,
            acknowledgement: e.acknowledgement,
            public_task: e.public_task,
            compacted: e.compacted,
        };
        record
            .validate()
            .map_err(|_| serde::de::Error::custom("invalid execution record"))?;
        Ok(record)
    }
}
impl Record {
    pub(crate) fn key(&self) -> &Key {
        &self.key
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
    pub(crate) fn producer(&self) -> Option<&CanonicalOutcome> {
        self.producer.as_ref()
    }
    pub(crate) fn primary(&self) -> Option<&CanonicalOutcome> {
        self.primary_override.as_ref().or(self.producer.as_ref())
    }
    pub(crate) fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    pub(crate) fn acknowledgement(&self) -> Option<&AckRecord> {
        self.acknowledgement.as_ref()
    }
    pub(crate) fn owned_by(&self, owner_hash: [u8; 32]) -> bool {
        self.accepted
            .as_ref()
            .is_some_and(|a| a.owner_hash == owner_hash)
    }
    pub(crate) fn public_task(&self) -> bool {
        self.public_task
    }
    pub(crate) fn compacted(&self) -> bool {
        self.compacted
    }
    pub(crate) fn accepted(&self) -> bool {
        self.accepted.is_some()
    }
    pub(crate) fn window(&self) -> Option<ObservationWindow> {
        self.accepted.as_ref().map(|a| a.window)
    }
    pub(crate) fn run(&self) -> Option<RunIdentity> {
        self.begun.as_ref().map(|b| b.run)
    }
    pub(crate) fn fingerprint(&self) -> [u8; 32] {
        digest(&self.encode())
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("closed record serializes")
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let record: Self = serde_json::from_slice(bytes).map_err(|_| Error::InvalidRecord)?;
        record.validate()?;
        Ok(record)
    }
    pub(crate) fn validate(&self) -> Result<(), Error> {
        let cancelled_digest = CanonicalOutcome::cancelled().digest();
        let interrupted_digest = CanonicalOutcome::failed(FailureReason::Interrupted).digest();
        let invalid = self.revision == 0
            || self.accepted.as_ref().is_some_and(|a| {
                ObservationWindow::new(a.window.received_epoch_ms, a.window.budget_ms).is_err()
            })
            || (self.accepted.is_none()
                && (!self.cancel_requested
                    || self.begun.is_some()
                    || self.producer.is_some()
                    || self.producer_digest.is_some()
                    || self.primary_override.is_some()
                    || self.primary_digest.is_some()
                    || self.public_task
                    || self.acknowledgement.is_some()
                    || self.compacted))
            || self.begun.as_ref().is_some_and(|b| {
                b.run.attempt != AttemptId(self.key.digest())
                    || b.run.generation != 1
                    || (!self.key.tool.is_read() && b.read_authority.is_some())
            })
            || self
                .producer
                .as_ref()
                .is_some_and(|o| !o.valid() || Some(o.digest()) != self.producer_digest)
            || self.primary_override.as_ref().is_some_and(|o| {
                !self.key.tool.is_read()
                    || *o != CanonicalOutcome::cancelled()
                    || Some(o.digest()) != self.primary_digest
            })
            || (self.primary_override.is_some() && self.begun.is_none())
            || self
                .primary()
                .is_some_and(|o| Some(o.digest()) != self.primary_digest)
            || (!self.compacted && self.primary_digest.is_some() && self.primary().is_none())
            || (self.producer_digest.is_some() && self.primary_digest.is_none())
            // The current model can terminalize an unbegun attempt only by
            // early cancellation or recovery. It cannot invent execution.
            || (self.begun.is_none()
                && self.producer_digest.is_some_and(|d| {
                    d != interrupted_digest && !(self.cancel_requested && d == cancelled_digest)
                        && !self.producer.as_ref().is_some_and(CanonicalOutcome::is_admission_rejection)
                        && !self.compacted
                }))
            || (self.compacted
                && self.primary_digest != self.producer_digest
                && !(self.key.tool.is_read() && self.primary_digest == Some(cancelled_digest)))
            || self.acknowledgement.is_some_and(|a| Some(a.digest) != self.primary_digest)
            || (self.compacted
                && (self.acknowledgement.is_none()
                    || self.public_task
                    || self.producer.is_some()
                    || self.primary_override.is_some()
                    || self.producer_digest.is_none()))
            || (!self.compacted && self.producer_digest.is_some() && self.producer.is_none());
        if invalid {
            Err(Error::InvalidRecord)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DurableCatalog {
    records: Vec<Record>,
}
impl DurableCatalog {
    pub(crate) fn new(records: Vec<Record>) -> Result<Self, Error> {
        let mut invocations = HashSet::new();
        let mut tasks = HashSet::new();
        for r in &records {
            r.validate()?;
            if !invocations.insert(r.key.invocation) || !tasks.insert(r.key.task) {
                return Err(Error::IdentityCollision);
            }
        }
        Ok(Self { records })
    }
    pub(crate) fn records(&self) -> &[Record] {
        &self.records
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(&self.records).expect("closed catalog serializes")
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Self::new(serde_json::from_slice(bytes).map_err(|_| Error::InvalidRecord)?)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct State {
    record: Record,
    holders: HashMap<TaskId, Holder>,
    membership_generation: u64,
    joins_closed: bool,
}
impl State {
    pub(crate) fn empty(key: Key) -> Self {
        Self {
            record: Record {
                key,
                revision: 0,
                accepted: None,
                begun: None,
                cancel_requested: false,
                producer: None,
                producer_digest: None,
                primary_override: None,
                primary_digest: None,
                acknowledgement: None,
                public_task: false,
                compacted: false,
            },
            holders: HashMap::new(),
            membership_generation: 0,
            joins_closed: false,
        }
    }
    pub(crate) fn from_record(record: Record) -> Result<Self, Error> {
        record.validate()?;
        Ok(Self {
            record,
            holders: HashMap::new(),
            membership_generation: 0,
            joins_closed: true,
        })
    }
    pub(crate) fn record(&self) -> &Record {
        &self.record
    }
    pub(crate) fn holder_count(&self) -> usize {
        self.holders.len()
    }
    pub(crate) fn observe_holder(
        &self,
        member: Membership,
    ) -> Result<HolderObservation<'_>, Error> {
        let h = self
            .holders
            .get(&member.alias)
            .ok_or(Error::StaleMembership)?;
        if member.producer != AttemptId(self.record.key.digest())
            || h.generation != member.generation
        {
            return Err(Error::StaleMembership);
        }
        if h.cancelled {
            Ok(HolderObservation::Cancelled)
        } else {
            Ok(self
                .record
                .producer
                .as_ref()
                .map_or(HolderObservation::Working, HolderObservation::Terminal))
        }
    }
    pub(crate) fn with_readback(&self, record: Record) -> Result<Self, Error> {
        record.validate()?;
        let old = &self.record;
        if record.key != old.key
            || record.revision < old.revision
            || (record.revision == old.revision && record != *old)
            || (old.accepted.is_some() && record.accepted != old.accepted)
            || (old.begun.is_some() && record.begun != old.begun)
            || (old.cancel_requested && !record.cancel_requested)
            || (old.producer_digest.is_some() && record.producer_digest != old.producer_digest)
            || (old.primary_digest.is_some() && record.primary_digest != old.primary_digest)
            || (old.acknowledgement.is_some() && record.acknowledgement != old.acknowledgement)
            || (old.public_task && !record.public_task)
            || (old.compacted && !record.compacted)
            || (record.compacted && !self.holders.is_empty())
        {
            return Err(Error::InvalidRecord);
        }
        let mut state = self.clone();
        state.record = record;
        Ok(state)
    }
    pub(crate) fn decide(&self, event: Event) -> Result<Option<Transition>, Error> {
        let mut next = self.clone();
        let r = &mut next.record;
        let mut intents = Vec::new();
        match event {
            Event::Submit { owner_hash, window } => {
                if let Some(a) = &r.accepted {
                    return if a.owner_hash == owner_hash {
                        Ok(None)
                    } else {
                        Err(Error::OwnerMismatch)
                    };
                }
                r.accepted = Some(Accepted { owner_hash, window });
                if r.cancel_requested {
                    let o = CanonicalOutcome::cancelled();
                    r.primary_digest = Some(o.digest());
                    r.producer_digest = Some(o.digest());
                    r.producer = Some(o);
                } else {
                    intents.push(Intent::Accepted);
                }
            }
            Event::Cancel => {
                if r.compacted || r.producer_digest.is_some() {
                    return Ok(None);
                }
                if r.accepted.is_none() {
                    if r.cancel_requested {
                        return Ok(None);
                    }
                    r.cancel_requested = true;
                } else if r.begun.is_none() {
                    r.cancel_requested = true;
                    let o = CanonicalOutcome::cancelled();
                    r.primary_digest = Some(o.digest());
                    r.producer_digest = Some(o.digest());
                    r.producer = Some(o);
                } else if r.key.tool.is_read() && !next.holders.is_empty() {
                    if r.primary_override.is_some() {
                        return Ok(None);
                    }
                    let o = CanonicalOutcome::cancelled();
                    r.primary_digest = Some(o.digest());
                    r.primary_override = Some(o);
                    if next.holders.values().all(|h| h.cancelled) {
                        r.cancel_requested = true;
                        next.joins_closed = true;
                        intents.push(Intent::Cancel);
                    }
                } else {
                    if r.cancel_requested {
                        return Ok(None);
                    }
                    r.cancel_requested = true;
                    next.joins_closed = true;
                    intents.push(Intent::Cancel);
                }
            }
            Event::Begin {
                started_epoch_ms,
                read_authority,
            } => {
                if r.accepted.is_none() {
                    return Err(Error::NotAccepted);
                }
                if r.cancel_requested || r.producer_digest.is_some() {
                    return Err(Error::NotRunnable);
                }
                if r.begun.is_some() {
                    return Err(Error::AlreadyBegun);
                }
                if !r.key.tool.is_read() && read_authority.is_some() {
                    return Err(Error::NotJoinable);
                }
                let run = RunIdentity {
                    attempt: AttemptId(r.key.digest()),
                    generation: 1,
                };
                r.begun = Some(Begun {
                    run,
                    started_epoch_ms,
                    read_authority,
                });
                intents.push(Intent::Run(run));
            }
            Event::Outcome { run, outcome } => {
                if r.run() != Some(run) {
                    return Err(Error::ForeignOutcome);
                }
                if let Some(d) = r.producer_digest {
                    return if d == outcome.digest() {
                        Ok(None)
                    } else {
                        Err(Error::OutcomeConflict)
                    };
                }
                r.producer_digest = Some(outcome.digest());
                if r.primary_override.is_none() {
                    r.primary_digest = Some(outcome.digest());
                }
                r.producer = Some(outcome);
                next.joins_closed = true;
            }
            Event::AdmissionRejected(outcome) => {
                if r.accepted.is_none() {
                    return Err(Error::NotAccepted);
                }
                if r.begun.is_some() {
                    return Err(Error::AlreadyBegun);
                }
                if r.producer_digest.is_some() {
                    return Ok(None);
                }
                if !outcome.is_admission_rejection() {
                    return Err(Error::InvalidInput);
                }
                r.producer_digest = Some(outcome.digest());
                r.primary_digest = Some(outcome.digest());
                r.producer = Some(outcome);
            }
            Event::ExposeTask => {
                if r.accepted.is_none() {
                    return Err(Error::NotAccepted);
                }
                if r.compacted {
                    return Err(Error::Compacted);
                }
                if r.public_task {
                    return Ok(None);
                }
                r.public_task = true;
                intents.push(Intent::Task);
            }
            Event::Acknowledge {
                owner_hash,
                primary_digest,
                acknowledged_epoch_ms,
            } => {
                if r.accepted.as_ref().map(|a| a.owner_hash) != Some(owner_hash) {
                    return Err(Error::OwnerMismatch);
                }
                if r.primary_digest != Some(primary_digest) {
                    return Err(Error::DigestMismatch);
                }
                if r.acknowledgement.is_some() {
                    return Ok(None);
                }
                r.acknowledgement = Some(AckRecord {
                    digest: primary_digest,
                    first_acknowledged_epoch_ms: acknowledged_epoch_ms,
                });
            }
            Event::Attach(evidence) => {
                let b = r.begun.as_ref().ok_or(Error::NotJoinable)?;
                let a = r.accepted.as_ref().ok_or(Error::NotAccepted)?;
                if next.joins_closed
                    || r.cancel_requested
                    || r.producer_digest.is_some()
                    || !r.key.tool.is_read()
                    || !r.key.same_request(&evidence.request)
                    || b.read_authority != Some(evidence.authority)
                    || evidence.observed_epoch_ms < a.window.end_epoch_ms()
                    || evidence.observed_epoch_ms < b.started_epoch_ms
                    || evidence.observed_epoch_ms - b.started_epoch_ms > 30_000
                    || evidence.request.task == r.key.task
                    || next.holders.contains_key(&evidence.request.task)
                {
                    return Err(Error::NotJoinable);
                }
                next.membership_generation = next
                    .membership_generation
                    .checked_add(1)
                    .ok_or(Error::Overflow)?;
                let member = Membership {
                    alias: evidence.request.task,
                    generation: next.membership_generation,
                    producer: AttemptId(r.key.digest()),
                };
                next.holders.insert(
                    member.alias,
                    Holder {
                        generation: member.generation,
                        cancelled: false,
                    },
                );
                intents.push(Intent::Attached(member));
            }
            Event::CancelHolder(member) | Event::RetireHolder(member) => {
                let holder = next
                    .holders
                    .get_mut(&member.alias)
                    .ok_or(Error::StaleMembership)?;
                if member.producer != AttemptId(r.key.digest())
                    || holder.generation != member.generation
                {
                    return Err(Error::StaleMembership);
                }
                if matches!(event, Event::RetireHolder(_)) {
                    next.holders.remove(&member.alias);
                } else {
                    if holder.cancelled || r.producer_digest.is_some() {
                        return Ok(None);
                    }
                    holder.cancelled = true;
                }
                if r.primary_override.is_some()
                    && next.holders.values().all(|h| h.cancelled)
                    && r.producer_digest.is_none()
                {
                    r.cancel_requested = true;
                    next.joins_closed = true;
                    intents.push(Intent::Cancel);
                }
            }
            Event::Recover => {
                if r.accepted.is_none() || r.producer_digest.is_some() {
                    return Ok(None);
                }
                let o = if r.begun.is_some() {
                    CanonicalOutcome::failed(FailureReason::OutcomeUncertain)
                } else if r.cancel_requested {
                    CanonicalOutcome::cancelled()
                } else {
                    CanonicalOutcome::failed(FailureReason::Interrupted)
                };
                r.producer_digest = Some(o.digest());
                if r.primary_override.is_none() {
                    r.primary_digest = Some(o.digest());
                }
                r.producer = Some(o);
                next.joins_closed = true;
            }
        }
        if r.acknowledgement.is_some()
            && !r.public_task
            && r.producer_digest.is_some()
            && next.holders.is_empty()
        {
            r.producer = None;
            r.primary_override = None;
            r.compacted = true;
        }
        r.revision = r.revision.checked_add(1).ok_or(Error::Overflow)?;
        r.validate()?;
        Ok(Some(Transition {
            state: next,
            intents,
        }))
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Event {
    Submit {
        owner_hash: [u8; 32],
        window: ObservationWindow,
    },
    Cancel,
    Begin {
        started_epoch_ms: u64,
        read_authority: Option<ReadAuthority>,
    },
    Outcome {
        run: RunIdentity,
        outcome: CanonicalOutcome,
    },
    AdmissionRejected(CanonicalOutcome),
    ExposeTask,
    Acknowledge {
        owner_hash: [u8; 32],
        primary_digest: [u8; 32],
        acknowledged_epoch_ms: u64,
    },
    Attach(JoinEvidence),
    CancelHolder(Membership),
    RetireHolder(Membership),
    Recover,
}
#[derive(Debug)]
pub(crate) enum Intent {
    Accepted,
    Run(RunIdentity),
    Task,
    Attached(Membership),
    Cancel,
}
#[derive(Debug)]
pub(crate) struct Transition {
    state: State,
    intents: Vec<Intent>,
}
impl Transition {
    pub(crate) fn into_parts(self) -> (State, Vec<Intent>) {
        (self.state, self.intents)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidInput,
    InvalidRecord,
    IdentityCollision,
    OwnerMismatch,
    DigestMismatch,
    NotAccepted,
    NotRunnable,
    AlreadyBegun,
    ForeignOutcome,
    OutcomeConflict,
    NotJoinable,
    StaleMembership,
    Compacted,
    AlreadyAcknowledged,
    AliasCapacity,
    Overflow,
    StoreUnavailable,
    StaleCompletion,
    RecoveryRequired,
}
