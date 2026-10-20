//! Test-only observation and fixture access; production state stays private.
use super::*;
use crate::infrastructure::platform::filesystem::RetainedDirectoryCapability;

pub(crate) fn load(
    store: &FileAggregateStore,
    key: &Key,
) -> Result<StoredRecord, AuthorityFailureCause> {
    store.load(key).map(|(stored, _)| stored)
}

pub(crate) fn records(store: &FileAggregateStore) -> &RetainedDirectoryCapability {
    store.authority.records()
}

pub(crate) fn nonce(store: &FileAggregateStore) -> [u8; 16] {
    store.authority.nonce()
}

pub(crate) fn record_count(store: &FileAggregateStore) -> usize {
    store.entries.len()
}

pub(crate) fn scanned_records(opening: &OpeningStore) -> impl ExactSizeIterator<Item = &Record> {
    opening.entries.values().map(|entry| &entry.record)
}
