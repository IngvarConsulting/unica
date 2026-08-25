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
use uuid::Uuid;

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

/// Identity a v0.13 continuation is allowed to resume. The source revision is
/// deliberately separate: replay against the same question after a change is
/// `stale_cursor`, while replay against another question is `invalid_cursor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewCursorBinding {
    pub(crate) canonical_at: String,
    pub(crate) projection: String,
    pub(crate) normalized_filter: String,
    pub(crate) source_set_identity: String,
    pub(crate) source_revision: String,
    pub(crate) page_limit: usize,
}

struct ViewCursorEntry {
    binding: ViewCursorBinding,
    node: Value,
    items: Vec<Value>,
    next_cursor: Option<String>,
    bytes: usize,
    stored_at: Instant,
    last_read: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredViewCursor {
    pub(crate) binding: ViewCursorBinding,
    pub(crate) node: Value,
    pub(crate) items: Vec<Value>,
    pub(crate) next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewCursorError {
    Invalid,
    Stale,
}

impl ViewCursorError {
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::Invalid => "invalid_cursor",
            Self::Stale => "stale_cursor",
        }
    }
}

/// Bounded process-local storage for opaque v0.13 page continuations. Tokens
/// are random replayable capabilities; no numeric parser offset crosses the
/// public boundary. Each stored entry owns one already-cut page and its stable
/// successor, so a client retry after a transport timeout is byte-equivalent.
pub(crate) struct ViewCursorStore {
    ttl: Duration,
    max_entries: usize,
    max_total_bytes: usize,
    entries: Mutex<HashMap<String, ViewCursorEntry>>,
}

impl Default for ViewCursorStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL, DEFAULT_MAX_ENTRIES, DEFAULT_MAX_TOTAL_BYTES)
    }
}

impl ViewCursorStore {
    pub(crate) fn new(ttl: Duration, max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            ttl,
            max_entries,
            max_total_bytes,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert_pages(
        &self,
        binding: ViewCursorBinding,
        node: Value,
        items: Vec<Value>,
        offset: usize,
        page_limit: usize,
    ) -> Option<String> {
        if offset >= items.len() || page_limit == 0 {
            return None;
        }
        let page_count = items.len().saturating_sub(offset).div_ceil(page_limit);
        if page_count > self.max_entries {
            return None;
        }
        let now = Instant::now();
        let pages = items[offset..]
            .chunks(page_limit)
            .map(<[Value]>::to_vec)
            .collect::<Vec<_>>();
        let tokens = (0..pages.len())
            .map(|_| format!("vc1.{}", Uuid::new_v4().simple()))
            .collect::<Vec<_>>();
        let entries_to_add = pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let next_cursor = tokens.get(index + 1).cloned();
                let bytes = serde_json::to_vec(&(&node, &page, &next_cursor))
                    .ok()?
                    .len();
                Some((
                    tokens[index].clone(),
                    ViewCursorEntry {
                        binding: binding.clone(),
                        node: node.clone(),
                        items: page,
                        next_cursor,
                        bytes,
                        stored_at: now,
                        last_read: now,
                    },
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        let added_bytes = entries_to_add
            .iter()
            .map(|(_, entry)| entry.bytes)
            .sum::<usize>();
        if added_bytes > self.max_total_bytes {
            return None;
        }
        let mut entries = self.entries.lock().expect("view cursor store poisoned");
        entries.retain(|_, entry| now.duration_since(entry.stored_at) < self.ttl);
        let mut total = entries.values().map(|entry| entry.bytes).sum::<usize>();
        while entries.len().saturating_add(entries_to_add.len()) > self.max_entries
            || (total.saturating_add(added_bytes) > self.max_total_bytes && !entries.is_empty())
        {
            let oldest = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_read)
                .map(|(token, _)| token.clone())?;
            if let Some(removed) = entries.remove(&oldest) {
                total = total.saturating_sub(removed.bytes);
            }
        }
        let first = tokens.first()?.clone();
        entries.extend(entries_to_add);
        Some(first)
    }

    pub(crate) fn read(
        &self,
        token: &str,
        expected: &ViewCursorBinding,
        current_revision: &str,
    ) -> Result<StoredViewCursor, ViewCursorError> {
        if !valid_view_cursor_token(token) {
            return Err(ViewCursorError::Invalid);
        }
        let mut entries = self.entries.lock().expect("view cursor store poisoned");
        let now = Instant::now();
        let Some(entry) = entries.get(token) else {
            return Err(ViewCursorError::Invalid);
        };
        if now.duration_since(entry.stored_at) >= self.ttl {
            entries.remove(token);
            return Err(ViewCursorError::Invalid);
        }
        let entry = entries
            .get_mut(token)
            .expect("the checked view cursor remains present");
        if entry.binding.canonical_at != expected.canonical_at
            || entry.binding.projection != expected.projection
            || entry.binding.normalized_filter != expected.normalized_filter
            || entry.binding.source_set_identity != expected.source_set_identity
            || entry.binding.page_limit != expected.page_limit
        {
            return Err(ViewCursorError::Invalid);
        }
        if entry.binding.source_revision != current_revision {
            return Err(ViewCursorError::Stale);
        }
        entry.last_read = now;
        Ok(StoredViewCursor {
            binding: entry.binding.clone(),
            node: entry.node.clone(),
            items: entry.items.clone(),
            next_cursor: entry.next_cursor.clone(),
        })
    }
}

fn valid_view_cursor_token(token: &str) -> bool {
    token.len() == 36
        && token.starts_with("vc1.")
        && token[4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
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

    fn view_binding(at: &str, revision: &str) -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: at.to_string(),
            projection: "Body".to_string(),
            normalized_filter: "{}".to_string(),
            source_set_identity: "main:sha256-source-id".to_string(),
            source_revision: revision.to_string(),
            page_limit: 1,
        }
    }

    #[test]
    fn opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let token = store
            .insert_pages(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2}), json!({"line": 3})],
                0,
                1,
            )
            .unwrap();
        assert!(token.starts_with("vc1."));
        assert!(token[4..].parse::<usize>().is_err());

        let mut other = binding.clone();
        other.canonical_at = "main:Document.Счет.Module.Object.Body".to_string();
        assert_eq!(
            store.read(&token, &other, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.projection = "Method".to_string();
        assert_eq!(
            store.read(&token, &other, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.normalized_filter = "{\"visibility\":\"public\"}".to_string();
        assert_eq!(
            store.read(&token, &other, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.source_set_identity = "other:sha256-source-id".to_string();
        assert_eq!(
            store.read(&token, &other, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.page_limit = 2;
        assert_eq!(
            store.read(&token, &other, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let page = store.read(&token, &binding, "rev-1").unwrap();
        assert_eq!(page.items, vec![json!({"line": 2})]);
        assert!(page.next_cursor.is_some());
        let replay = store.read(&token, &binding, "rev-1").unwrap();
        assert_eq!(replay, page);
    }

    #[test]
    fn cursor_chain_is_refused_before_it_can_exceed_the_entry_bound() {
        let store = ViewCursorStore::new(DEFAULT_TTL, 2, 1_024);
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");

        assert!(store
            .insert_pages(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!(1), json!(2), json!(3)],
                0,
                1,
            )
            .is_none());
        assert!(store
            .insert_pages(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!(1), json!(2)],
                0,
                1,
            )
            .is_some());
    }

    #[test]
    fn exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let token = store
            .insert_pages(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2})],
                0,
                1,
            )
            .unwrap();
        assert_eq!(
            store.read(&token, &binding, "rev-2").unwrap_err(),
            ViewCursorError::Stale
        );
        assert_eq!(
            store
                .read("vc1.00000000000000000000000000000000", &binding, "rev-1")
                .unwrap_err(),
            ViewCursorError::Invalid
        );

        let expiring = ViewCursorStore::new(Duration::ZERO, 4, 1024);
        let token = expiring
            .insert_pages(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2})],
                0,
                1,
            )
            .unwrap();
        assert_eq!(
            expiring.read(&token, &binding, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
    }

    #[test]
    fn revision_bound_view_cursor_contract_is_complete() {
        opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question();
        exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid();
        cursor_chain_is_refused_before_it_can_exceed_the_entry_bound();
    }
}
