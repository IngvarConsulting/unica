//! Bounded in-memory store for deferred typed reader results (ADR-0070).
//!
//! The store keeps an immutable snapshot of a full typed `OperationResult.data`
//! so a continuation call can serve byte-stable slices without re-reading the
//! source. Entries never outlive the server process; TTL, LRU eviction and a
//! total-bytes quota keep it bounded.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
pub const DEFAULT_MAX_ENTRIES: usize = 32;
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultStoreError {
    /// The reference is unknown: never issued, evicted, or from another process.
    Unavailable,
    /// The entry expired by TTL.
    Expired,
    /// The reference exists but belongs to another tool or argument set.
    RefMismatch,
}

impl ResultStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            ResultStoreError::Unavailable => "result_unavailable",
            ResultStoreError::Expired => "result_expired",
            ResultStoreError::RefMismatch => "result_ref_mismatch",
        }
    }
}

/// Identity of the source snapshot the stored result was computed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotIdentity {
    pub workspace_epoch: u64,
    pub cache_root: String,
    pub as_of_unix_ms: u64,
}

struct Entry {
    tool: String,
    args_identity: String,
    snapshot: SnapshotIdentity,
    data: Value,
    bytes: usize,
    stored_at: Instant,
    last_read: Instant,
    expires_at_unix_ms: u64,
}

#[derive(Debug)]
pub struct StoredView {
    pub data: Value,
    pub snapshot: SnapshotIdentity,
    pub bytes: usize,
    pub expires_at_unix_ms: u64,
}

pub struct ResultStore {
    ttl: Duration,
    max_entries: usize,
    max_total_bytes: usize,
    next_id: AtomicU64,
    entries: Mutex<HashMap<String, Entry>>,
}

impl Default for ResultStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_TOTAL_BYTES)
    }
}

impl ResultStore {
    pub fn new(ttl: Duration, max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            ttl,
            max_entries,
            max_total_bytes,
            next_id: AtomicU64::new(1),
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    /// Stores a full typed result and returns its continuation reference.
    ///
    /// Oversized single results (larger than the total quota) are refused with
    /// `None`: the caller then serves the full result inline instead of
    /// promising a continuation the store cannot honor.
    pub fn insert(
        &self,
        tool: &str,
        args_identity: &str,
        snapshot: SnapshotIdentity,
        data: Value,
        serialized_bytes: usize,
    ) -> Option<String> {
        if serialized_bytes > self.max_total_bytes {
            return None;
        }
        let now = Instant::now();
        let expires_at_unix_ms = unix_ms_in(self.ttl);
        let id = format!(
            "res-{}-{:x}",
            self.next_id.fetch_add(1, Ordering::Relaxed),
            std::process::id()
        );
        let mut entries = self.entries.lock().expect("result store poisoned");
        entries.retain(|_, entry| now.duration_since(entry.stored_at) < self.ttl);
        let mut total: usize = entries.values().map(|entry| entry.bytes).sum();
        while entries.len() >= self.max_entries
            || (total + serialized_bytes > self.max_total_bytes && !entries.is_empty())
        {
            let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_read)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(evicted) = entries.remove(&oldest) {
                total -= evicted.bytes;
            }
        }
        entries.insert(
            id.clone(),
            Entry {
                tool: tool.to_string(),
                args_identity: args_identity.to_string(),
                snapshot,
                data,
                bytes: serialized_bytes,
                stored_at: now,
                last_read: now,
                expires_at_unix_ms,
            },
        );
        Some(id)
    }

    /// Reads the immutable snapshot back for a continuation call.
    pub fn read(
        &self,
        result_ref: &str,
        tool: &str,
        args_identity: &str,
    ) -> Result<StoredView, ResultStoreError> {
        let mut entries = self.entries.lock().expect("result store poisoned");
        let Some(entry) = entries.get_mut(result_ref) else {
            return Err(ResultStoreError::Unavailable);
        };
        if entry.stored_at.elapsed() >= self.ttl {
            entries.remove(result_ref);
            return Err(ResultStoreError::Expired);
        }
        if entry.tool != tool || entry.args_identity != args_identity {
            return Err(ResultStoreError::RefMismatch);
        }
        entry.last_read = Instant::now();
        Ok(StoredView {
            data: entry.data.clone(),
            snapshot: entry.snapshot.clone(),
            bytes: entry.bytes,
            expires_at_unix_ms: entry.expires_at_unix_ms,
        })
    }
}

pub fn unix_ms_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

fn unix_ms_in(ttl: Duration) -> u64 {
    unix_ms_now().saturating_add(ttl.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot() -> SnapshotIdentity {
        SnapshotIdentity {
            workspace_epoch: 7,
            cache_root: "/tmp/ws".to_string(),
            as_of_unix_ms: unix_ms_now(),
        }
    }

    #[test]
    fn continuation_reads_the_same_immutable_data() {
        let store = ResultStore::default();
        let data = json!({"rights": [1, 2, 3]});
        let reference = store
            .insert("unica.role.info", "args-a", snapshot(), data.clone(), 64)
            .unwrap();
        let first = store.read(&reference, "unica.role.info", "args-a").unwrap();
        let second = store.read(&reference, "unica.role.info", "args-a").unwrap();
        assert_eq!(first.data, data);
        assert_eq!(second.data, data);
        assert_eq!(first.snapshot, second.snapshot);
    }

    #[test]
    fn unknown_reference_is_unavailable() {
        let store = ResultStore::default();
        assert_eq!(
            store
                .read("res-404", "unica.role.info", "args-a")
                .unwrap_err(),
            ResultStoreError::Unavailable
        );
    }

    #[test]
    fn another_tool_or_args_is_a_ref_mismatch() {
        let store = ResultStore::default();
        let reference = store
            .insert("unica.role.info", "args-a", snapshot(), json!({}), 8)
            .unwrap();
        assert_eq!(
            store
                .read(&reference, "unica.subsystem.info", "args-a")
                .unwrap_err(),
            ResultStoreError::RefMismatch
        );
        assert_eq!(
            store
                .read(&reference, "unica.role.info", "args-b")
                .unwrap_err(),
            ResultStoreError::RefMismatch
        );
    }

    #[test]
    fn expired_entry_reports_result_expired() {
        let store = ResultStore::new(Duration::ZERO, 8, 1024);
        let reference = store
            .insert("unica.role.info", "args-a", snapshot(), json!({}), 8)
            .unwrap();
        assert_eq!(
            store
                .read(&reference, "unica.role.info", "args-a")
                .unwrap_err(),
            ResultStoreError::Expired
        );
    }

    #[test]
    fn lru_eviction_keeps_the_store_bounded() {
        let store = ResultStore::new(DEFAULT_TTL, 2, 1024);
        let first = store.insert("t", "a", snapshot(), json!(1), 8).unwrap();
        let second = store.insert("t", "b", snapshot(), json!(2), 8).unwrap();
        // Touch the first entry so the second becomes the eviction candidate.
        store.read(&first, "t", "a").unwrap();
        let _third = store.insert("t", "c", snapshot(), json!(3), 8).unwrap();
        assert!(store.read(&first, "t", "a").is_ok());
        assert_eq!(
            store.read(&second, "t", "b").unwrap_err(),
            ResultStoreError::Unavailable
        );
    }

    #[test]
    fn byte_quota_evicts_and_oversized_results_are_refused() {
        let store = ResultStore::new(DEFAULT_TTL, 8, 100);
        let first = store.insert("t", "a", snapshot(), json!(1), 60).unwrap();
        let _second = store.insert("t", "b", snapshot(), json!(2), 60).unwrap();
        assert_eq!(
            store.read(&first, "t", "a").unwrap_err(),
            ResultStoreError::Unavailable,
        );
        assert!(store
            .insert("t", "big", snapshot(), json!(3), 200)
            .is_none());
    }
}
