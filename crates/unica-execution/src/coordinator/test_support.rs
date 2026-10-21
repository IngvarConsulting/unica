//! Test adapter only: stores validated aggregates and acknowledges model requests.
//! It never decides an execution transition, and proves no physical durability.
use super::hydration::*;
use super::*;
use std::ops::{Deref, DerefMut};
use unica_execution_contract::storage::RecoveryBuilder;

#[derive(Clone, Debug)]
pub(super) struct Archive(Vec<StoredRecord>);
impl Archive {
    pub(super) fn new(records: Vec<StoredRecord>) -> Result<Self, Error> {
        let mut builder = RecoveryBuilder::default();
        for record in &records {
            builder.push(record.clone())?;
        }
        builder.finish()?;
        Ok(Self(records))
    }
    pub(super) fn records(&self) -> &[StoredRecord] {
        &self.0
    }
    pub(super) fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).unwrap()
    }
    pub(super) fn decode(bytes: &[u8]) -> Result<Self, Error> {
        Self::new(serde_json::from_slice(bytes).map_err(|_| Error::InvalidRecord)?)
    }
}

pub(super) struct Fixture {
    coordinator: ExecutionCoordinator,
    store: HashMap<Key, StoredRecord>,
}
impl Deref for Fixture {
    type Target = ExecutionCoordinator;
    fn deref(&self) -> &Self::Target {
        &self.coordinator
    }
}
impl DerefMut for Fixture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.coordinator
    }
}
impl Fixture {
    pub(super) fn new(epoch: AuthorityEpoch) -> Self {
        Self {
            coordinator: ExecutionCoordinator::new(epoch),
            store: HashMap::new(),
        }
    }
    pub(super) fn store(&self, key: &Key) -> &StoredRecord {
        &self.store[key]
    }
    pub(super) fn assemble(&self, request: &CommitRequest) -> StoredRecord {
        request
            .assemble(self.store.get(request.candidate().key()))
            .unwrap()
    }
    pub(super) fn commit(&mut self, request: CommitRequest) -> Result<Vec<Effect>, Error> {
        let stored = self.assemble(&request);
        self.store.insert(stored.record().key().clone(), stored);
        self.coordinator.complete(request.committed())
    }
    pub(super) fn readback(
        &mut self,
        request: CommitRequest,
        actual: StoredRecord,
    ) -> Result<Vec<Effect>, Error> {
        let result = self
            .coordinator
            .complete(request.readback(actual.clone()))?;
        if !self.coordinator.unavailable() {
            self.store.insert(actual.record().key().clone(), actual);
        }
        Ok(result)
    }
    pub(super) fn catalog(&self) -> Result<Archive, Error> {
        let mut records: Vec<_> = self.store.values().cloned().collect();
        records.sort_by_key(|r| r.record().key().digest());
        assert_eq!(
            records
                .iter()
                .map(|r| r.record().clone())
                .collect::<Vec<_>>(),
            self.coordinator.snapshot(),
            "test store and confirmed model diverged"
        );
        Archive::new(records)
    }
    pub(super) fn restore(
        archive: Archive,
        epoch: AuthorityEpoch,
    ) -> Result<(Self, Vec<Effect>), Error> {
        let mut builder = RecoveryBuilder::default();
        let mut store = HashMap::new();
        for stored in archive.0 {
            builder.push(stored.clone())?;
            store.insert(stored.record().key().clone(), stored);
        }
        let (coordinator, effects) = ExecutionCoordinator::restore(builder.finish()?, epoch)?;
        Ok((Self { coordinator, store }, effects))
    }
    pub(super) fn direct(
        &self,
        key: &Key,
        owner: &OwnerCapability,
    ) -> Result<DirectPublicationPermit, Error> {
        let request = self.coordinator.prepare_direct(key, owner)?;
        match self
            .coordinator
            .complete_read(request.loaded(self.store(key))?)?
        {
            ReadPublication::Direct(permit) => Ok(permit),
            _ => unreachable!("request binds Direct"),
        }
    }
}
