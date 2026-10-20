//! Complete stored aggregates and streaming validation. Metadata snapshots are
//! deliberately not deserializable durable evidence.
use super::*;
use std::collections::HashSet;
pub(crate) mod codec;

#[derive(Clone, Debug)]
pub(crate) enum PayloadChange {
    Keep,
    Set(CanonicalOutcome),
    Drop,
}
#[derive(Clone, Debug)]
pub(crate) struct PayloadDelta {
    pub(super) producer: PayloadChange,
    pub(super) primary_override: PayloadChange,
}
impl PayloadDelta {
    pub(super) fn keep() -> Self {
        Self {
            producer: PayloadChange::Keep,
            primary_override: PayloadChange::Keep,
        }
    }
    pub(crate) fn producer(&self) -> &PayloadChange {
        &self.producer
    }
    pub(crate) fn primary_override(&self) -> &PayloadChange {
        &self.primary_override
    }
    pub(crate) fn assemble(
        &self,
        record: Record,
        prior: Option<&StoredRecord>,
    ) -> Result<StoredRecord, Error> {
        if prior.is_some_and(|p| p.record.key() != record.key()) {
            return Err(Error::IdentityCollision);
        }
        fn apply(
            change: &PayloadChange,
            prior: Option<&CanonicalOutcome>,
        ) -> Option<CanonicalOutcome> {
            match change {
                PayloadChange::Keep => prior.cloned(),
                PayloadChange::Set(body) => Some(body.clone()),
                PayloadChange::Drop => None,
            }
        }
        StoredRecord::new(
            record,
            apply(&self.producer, prior.and_then(|p| p.producer.as_ref())),
            apply(
                &self.primary_override,
                prior.and_then(|p| p.primary_override.as_ref()),
            ),
        )
    }
}

/// All required role payloads have been validated against this exact metadata.
/// An adapter must persist this entire aggregate in one atomic boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredRecord {
    record: Record,
    producer: Option<CanonicalOutcome>,
    primary_override: Option<CanonicalOutcome>,
}
impl<'de> Deserialize<'de> for StoredRecord {
    fn deserialize<D: serde::Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Encoded {
            record: EncodedRecord,
            producer: Option<CanonicalOutcome>,
            primary_override: Option<CanonicalOutcome>,
        }
        let e = Encoded::deserialize(decoder)?;
        Self::new(
            e.record
                .validate()
                .map_err(|_| serde::de::Error::custom("invalid execution metadata"))?,
            e.producer,
            e.primary_override,
        )
        .map_err(|_| serde::de::Error::custom("invalid complete execution aggregate"))
    }
}
impl StoredRecord {
    pub(crate) fn new(
        record: Record,
        producer: Option<CanonicalOutcome>,
        primary_override: Option<CanonicalOutcome>,
    ) -> Result<Self, Error> {
        record.validate()?;
        fn matches(
            body: Option<&CanonicalOutcome>,
            descriptor: Option<&OutcomeDescriptor>,
            compacted: bool,
        ) -> bool {
            if compacted {
                return body.is_none();
            }
            match (body, descriptor) {
                (None, None) => true,
                (Some(body), Some(descriptor)) => body.valid() && body.descriptor() == *descriptor,
                _ => false,
            }
        }
        if !matches(
            producer.as_ref(),
            record.producer.as_ref(),
            record.compacted,
        ) || !matches(
            primary_override.as_ref(),
            record.primary_override.as_ref(),
            record.compacted,
        ) {
            return Err(Error::InvalidRecord);
        }
        Ok(Self {
            record,
            producer,
            primary_override,
        })
    }
    pub(crate) fn record(&self) -> &Record {
        &self.record
    }
    pub(crate) fn producer(&self) -> Option<&CanonicalOutcome> {
        self.producer.as_ref()
    }
    pub(crate) fn primary_override(&self) -> Option<&CanonicalOutcome> {
        self.primary_override.as_ref()
    }
    pub(crate) fn primary(&self) -> Option<&CanonicalOutcome> {
        self.primary_override.as_ref().or(self.producer.as_ref())
    }
    pub(crate) fn into_metadata(self) -> Record {
        self.record
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("closed aggregate serializes")
    }
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidRecord)
    }
}

/// Proof that every aggregate supplied to one stream was validated. This is
/// not proof of filesystem inventory completeness, durability, or authority.
#[derive(Debug)]
pub(crate) struct RecoveryCatalog {
    records: Vec<Record>,
}
impl RecoveryCatalog {
    pub(crate) fn into_records(self) -> Vec<Record> {
        self.records
    }
}
#[derive(Debug, Default)]
pub(crate) struct RecoveryBuilder {
    records: Vec<Record>,
    invocations: HashSet<InvocationId>,
    tasks: HashSet<TaskId>,
    failed: Option<Error>,
}
impl RecoveryBuilder {
    pub(crate) fn new() -> Self {
        Self::default()
    }
    pub(crate) fn push(&mut self, aggregate: StoredRecord) -> Result<(), Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        let key = aggregate.record.key();
        if self.invocations.contains(&key.invocation_id()) || self.tasks.contains(&key.task_id()) {
            self.failed = Some(Error::IdentityCollision);
            return Err(Error::IdentityCollision);
        }
        self.invocations.insert(key.invocation_id());
        self.tasks.insert(key.task_id());
        self.records.push(aggregate.into_metadata());
        Ok(())
    }
    pub(crate) fn push_encoded(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        match StoredRecord::decode(bytes) {
            Ok(record) => self.push(record),
            Err(error) => {
                self.failed = Some(error);
                Err(error)
            }
        }
    }
    pub(crate) fn finish(self) -> Result<RecoveryCatalog, Error> {
        if let Some(error) = self.failed {
            return Err(error);
        }
        Ok(RecoveryCatalog {
            records: self.records,
        })
    }
}
