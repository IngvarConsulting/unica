//! Bounded in-memory store for deferred typed reader results.
//!
//! The store keeps an immutable snapshot of a full typed `OperationResult.data`
//! so a continuation call can serve byte-stable slices without re-reading the
//! source. Entries never outlive the server process; TTL, LRU eviction and a
//! total-bytes quota keep it bounded.

use crate::domain::refusal::RefusalCode;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use uuid::Uuid;

pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
pub const DEFAULT_MAX_ENTRIES: usize = 32;
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const VIEW_MAX_ENTRIES: usize = 128;
// The token string and entry bookkeeping are bounded separately from the
// serialized snapshot. Charge each issued token so they cannot bypass the
// aggregate byte quota while sharing one Arc.
const VIEW_ENTRY_CHARGE: usize = 128;
const SEARCH_MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;

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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ViewCursorBinding {
    pub(crate) canonical_at: String,
    pub(crate) projection: String,
    pub(crate) normalized_filter: String,
    pub(crate) source_set_identity: String,
    pub(crate) source_revision: String,
    pub(crate) page_limit: usize,
}

struct ViewCursorEntry {
    snapshot: Arc<ViewCollectionSnapshot>,
    offset: usize,
    stored_at: Instant,
    last_read: Instant,
}

#[derive(Debug)]
pub(crate) struct ViewCollectionSnapshot {
    pub(crate) binding: ViewCursorBinding,
    pub(crate) node: Value,
    pub(crate) items: Vec<Value>,
    bytes: usize,
    secret: [u8; 32],
}

#[derive(Debug, Clone)]
pub(crate) struct StoredViewCursor {
    pub(crate) snapshot: Arc<ViewCollectionSnapshot>,
    pub(crate) offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewCursorError {
    Invalid,
    Stale,
}

impl ViewCursorError {
    pub(crate) const fn code(self) -> RefusalCode {
        match self {
            Self::Invalid => RefusalCode::InvalidCursor,
            Self::Stale => RefusalCode::StaleCursor,
        }
    }
}

/// Bounded process-local storage for opaque v0.13 page continuations. Tokens
/// are opaque replayable capabilities; no numeric parser offset crosses the
/// public boundary. Entries share one bounded immutable collection snapshot.
/// Only issued continuations consume entry slots, so a long collection can
/// progress without reserving its entire chain before the first page.
pub(crate) struct ViewCursorStore {
    ttl: Duration,
    max_entries: usize,
    max_total_bytes: usize,
    entries: Mutex<HashMap<String, ViewCursorEntry>>,
}

/// A search continuation stores only the question and the next offset. The
/// source is read again under a fresh read fence, so a long result never has
/// to fit in the cursor store before its first page can be returned.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct SearchCursorBinding {
    pub(crate) workspace_identity: String,
    pub(crate) query: String,
    pub(crate) scope: Option<String>,
    pub(crate) mode: String,
    pub(crate) kind: Option<String>,
    pub(crate) source_sets: Vec<String>,
    pub(crate) revisions: Vec<String>,
    /// Names have no source revision lease. Their complete ranked answer and
    /// public coverage instead identify the replayed stream.
    pub(crate) result_fingerprint: Option<String>,
    pub(crate) page_limit: usize,
}

#[derive(Debug)]
struct SearchCursorSnapshot {
    binding: SearchCursorBinding,
    bytes: usize,
    secret: [u8; 32],
}

struct SearchCursorEntry {
    snapshot: Arc<SearchCursorSnapshot>,
    offset: usize,
    stored_at: Instant,
    last_read: Instant,
}

#[derive(Clone)]
pub(crate) struct StoredSearchCursor {
    snapshot: Arc<SearchCursorSnapshot>,
    pub(crate) offset: usize,
}

pub(crate) struct SearchCursorStore {
    ttl: Duration,
    max_entries: usize,
    max_total_bytes: usize,
    entries: Mutex<HashMap<String, SearchCursorEntry>>,
}

impl Default for SearchCursorStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL, VIEW_MAX_ENTRIES, SEARCH_MAX_TOTAL_BYTES)
    }
}

impl SearchCursorStore {
    pub(crate) fn new(ttl: Duration, max_entries: usize, max_total_bytes: usize) -> Self {
        Self {
            ttl,
            max_entries,
            max_total_bytes,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn insert_first(
        &self,
        binding: SearchCursorBinding,
        offset: usize,
    ) -> Option<String> {
        if self.max_entries < 2 {
            return None;
        }
        let json_bytes = bounded_json_size(&binding, self.max_total_bytes)?;
        let bytes = search_snapshot_charge(&binding, json_bytes);
        if bytes.saturating_add(2 * VIEW_ENTRY_CHARGE) > self.max_total_bytes {
            return None;
        }
        let mut secret = [0u8; 32];
        secret[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        secret[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        self.insert_entry(
            Arc::new(SearchCursorSnapshot {
                binding,
                bytes,
                secret,
            }),
            offset,
            None,
        )
    }

    pub(crate) fn read(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<StoredSearchCursor, ViewCursorError> {
        if !valid_search_cursor_token(token) {
            return Err(ViewCursorError::Invalid);
        }
        let mut entries = self.entries.lock().expect("search cursor store poisoned");
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
            .expect("checked search cursor exists");
        let binding = &entry.snapshot.binding;
        if binding.workspace_identity != expected.workspace_identity
            || binding.query != expected.query
            || binding.scope != expected.scope
            || binding.mode != expected.mode
            || binding.kind != expected.kind
            || binding.source_sets != expected.source_sets
            || binding.page_limit != expected.page_limit
        {
            return Err(ViewCursorError::Invalid);
        }
        if binding.revisions != expected.revisions
            || binding.result_fingerprint != expected.result_fingerprint
        {
            return Err(ViewCursorError::Stale);
        }
        entry.last_read = now;
        Ok(StoredSearchCursor {
            snapshot: Arc::clone(&entry.snapshot),
            offset: entry.offset,
        })
    }

    pub(crate) fn insert_next(
        &self,
        current: &StoredSearchCursor,
        offset: usize,
        current_token: &str,
    ) -> Option<String> {
        self.insert_entry(Arc::clone(&current.snapshot), offset, Some(current_token))
    }

    fn insert_entry(
        &self,
        snapshot: Arc<SearchCursorSnapshot>,
        offset: usize,
        current_token: Option<&str>,
    ) -> Option<String> {
        if self.max_entries < 2
            || snapshot.bytes.saturating_add(2 * VIEW_ENTRY_CHARGE) > self.max_total_bytes
        {
            return None;
        }
        let token = search_cursor_token(&snapshot.secret, offset);
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("search cursor store poisoned");
        entries.retain(|_, entry| now.duration_since(entry.stored_at) < self.ttl);
        if let Some(existing) = entries.get_mut(&token) {
            if Arc::ptr_eq(&existing.snapshot, &snapshot) && existing.offset == offset {
                existing.last_read = now;
                return Some(token);
            }
            return None;
        }
        while entries.len() >= self.max_entries
            || unique_search_snapshot_bytes(&entries, Some(&snapshot))
                .saturating_add((entries.len() + 1).saturating_mul(VIEW_ENTRY_CHARGE))
                > self.max_total_bytes
        {
            let oldest = entries
                .iter()
                .filter(|(candidate, _)| current_token != Some(candidate.as_str()))
                .min_by_key(|(_, entry)| entry.last_read)
                .map(|(candidate, _)| candidate.clone())?;
            entries.remove(&oldest);
        }
        entries.insert(
            token.clone(),
            SearchCursorEntry {
                snapshot,
                offset,
                stored_at: now,
                last_read: now,
            },
        );
        Some(token)
    }
}

fn search_snapshot_charge(binding: &SearchCursorBinding, json_bytes: usize) -> usize {
    let strings = binding
        .source_sets
        .iter()
        .chain(binding.revisions.iter())
        .map(String::capacity)
        .fold(
            binding
                .workspace_identity
                .capacity()
                .saturating_add(binding.query.capacity())
                .saturating_add(binding.scope.as_ref().map_or(0, String::capacity))
                .saturating_add(binding.mode.capacity())
                .saturating_add(binding.kind.as_ref().map_or(0, String::capacity))
                .saturating_add(
                    binding
                        .result_fingerprint
                        .as_ref()
                        .map_or(0, String::capacity),
                ),
            usize::saturating_add,
        );
    json_bytes.max(
        std::mem::size_of::<SearchCursorSnapshot>()
            .saturating_add(strings)
            .saturating_add(
                (binding.source_sets.capacity() + binding.revisions.capacity())
                    .saturating_mul(std::mem::size_of::<String>()),
            ),
    )
}

fn unique_search_snapshot_bytes(
    entries: &HashMap<String, SearchCursorEntry>,
    extra: Option<&Arc<SearchCursorSnapshot>>,
) -> usize {
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for entry in entries.values() {
        if seen.insert(Arc::as_ptr(&entry.snapshot) as usize) {
            total = total.saturating_add(entry.snapshot.bytes);
        }
    }
    if let Some(snapshot) = extra {
        if seen.insert(Arc::as_ptr(snapshot) as usize) {
            total = total.saturating_add(snapshot.bytes);
        }
    }
    total
}

fn search_cursor_token(secret: &[u8; 32], offset: usize) -> String {
    let mut token = view_cursor_token(secret, offset);
    token.replace_range(..3, "sc1");
    token
}

fn valid_search_cursor_token(token: &str) -> bool {
    token.len() == 36
        && token.starts_with("sc1.")
        && token[4..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

impl Default for ViewCursorStore {
    fn default() -> Self {
        Self::new(DEFAULT_TTL, VIEW_MAX_ENTRIES, DEFAULT_MAX_TOTAL_BYTES)
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

    pub(crate) fn insert_snapshot(
        &self,
        binding: ViewCursorBinding,
        node: Value,
        items: Vec<Value>,
        offset: usize,
    ) -> Option<String> {
        if offset >= items.len() || self.max_entries == 0 {
            return None;
        }
        let json_bytes = bounded_json_size(&(&binding, &node, &items), self.max_total_bytes)?;
        let bytes = snapshot_charge(&binding, &node, &items, json_bytes);
        // The current cursor must remain available while its successor is
        // published. Admit enough quota for both before returning page one,
        // otherwise a later page could fail solely on cursor bookkeeping.
        let reserved_entries = if self.max_entries > 1 && offset + 1 < items.len() {
            2
        } else {
            1
        };
        if bytes.saturating_add(reserved_entries * VIEW_ENTRY_CHARGE) > self.max_total_bytes {
            return None;
        }
        let mut secret = [0u8; 32];
        secret[..16].copy_from_slice(Uuid::new_v4().as_bytes());
        secret[16..].copy_from_slice(Uuid::new_v4().as_bytes());
        let snapshot = Arc::new(ViewCollectionSnapshot {
            binding,
            node,
            items,
            bytes,
            secret,
        });
        self.insert_entry(snapshot, offset, None)
    }

    /// Reissue the same successor on retries, including when an older LRU
    /// eviction removed that successor after its first publication.
    pub(crate) fn insert_next(
        &self,
        current: &StoredViewCursor,
        offset: usize,
        current_token: &str,
    ) -> Option<String> {
        if offset >= current.snapshot.items.len() {
            return None;
        }
        self.insert_entry(Arc::clone(&current.snapshot), offset, Some(current_token))
    }

    fn insert_entry(
        &self,
        snapshot: Arc<ViewCollectionSnapshot>,
        offset: usize,
        current_token: Option<&str>,
    ) -> Option<String> {
        if self.max_entries == 0
            || snapshot.bytes.saturating_add(VIEW_ENTRY_CHARGE) > self.max_total_bytes
        {
            return None;
        }
        let token = view_cursor_token(&snapshot.secret, offset);
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("view cursor store poisoned");
        entries.retain(|_, entry| now.duration_since(entry.stored_at) < self.ttl);
        if let Some(existing) = entries.get_mut(&token) {
            if Arc::ptr_eq(&existing.snapshot, &snapshot) && existing.offset == offset {
                existing.last_read = now;
                return Some(token);
            }
            return None;
        }
        while entries.len() >= self.max_entries
            || unique_snapshot_bytes(&entries, Some(&snapshot))
                .saturating_add((entries.len() + 1).saturating_mul(VIEW_ENTRY_CHARGE))
                > self.max_total_bytes
        {
            let oldest = entries
                .iter()
                .filter(|(candidate, _)| {
                    self.max_entries == 1 || current_token != Some(candidate.as_str())
                })
                .min_by_key(|(_, entry)| entry.last_read)
                .map(|(candidate, _)| candidate.clone())?;
            entries.remove(&oldest);
        }
        entries.insert(
            token.clone(),
            ViewCursorEntry {
                snapshot,
                offset,
                stored_at: now,
                last_read: now,
            },
        );
        Some(token)
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
        let binding = &entry.snapshot.binding;
        if binding.canonical_at != expected.canonical_at
            || binding.projection != expected.projection
            || binding.normalized_filter != expected.normalized_filter
            || binding.source_set_identity != expected.source_set_identity
            || binding.page_limit != expected.page_limit
        {
            return Err(ViewCursorError::Invalid);
        }
        if binding.source_revision != current_revision {
            return Err(ViewCursorError::Stale);
        }
        entry.last_read = now;
        Ok(StoredViewCursor {
            snapshot: Arc::clone(&entry.snapshot),
            offset: entry.offset,
        })
    }
}

fn unique_snapshot_bytes(
    entries: &HashMap<String, ViewCursorEntry>,
    extra: Option<&Arc<ViewCollectionSnapshot>>,
) -> usize {
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for entry in entries.values() {
        if seen.insert(Arc::as_ptr(&entry.snapshot) as usize) {
            total = total.saturating_add(entry.snapshot.bytes);
        }
    }
    if let Some(snapshot) = extra {
        if seen.insert(Arc::as_ptr(snapshot) as usize) {
            total = total.saturating_add(snapshot.bytes);
        }
    }
    total
}

struct BoundedSizeWriter {
    bytes: usize,
    limit: usize,
}

impl Write for BoundedSizeWriter {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        self.bytes = self.bytes.saturating_add(chunk.len());
        if self.bytes > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "view snapshot exceeds quota",
            ));
        }
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn bounded_json_size(value: &impl serde::Serialize, limit: usize) -> Option<usize> {
    let mut writer = BoundedSizeWriter { bytes: 0, limit };
    serde_json::to_writer(&mut writer, value).ok()?;
    Some(writer.bytes)
}

fn snapshot_charge(
    binding: &ViewCursorBinding,
    node: &Value,
    items: &Vec<Value>,
    json_bytes: usize,
) -> usize {
    let binding_heap = [
        &binding.canonical_at,
        &binding.projection,
        &binding.normalized_filter,
        &binding.source_set_identity,
        &binding.source_revision,
    ]
    .iter()
    .fold(0usize, |total, value| {
        total.saturating_add(value.capacity())
    });
    let estimated_heap = std::mem::size_of::<ViewCollectionSnapshot>()
        .saturating_add(binding_heap)
        .saturating_add(value_heap_bytes(node))
        .saturating_add(
            items
                .capacity()
                .saturating_mul(std::mem::size_of::<Value>()),
        )
        .saturating_add(items.iter().fold(0usize, |total, value| {
            total.saturating_add(value_extra_heap_bytes(value))
        }));
    json_bytes.max(estimated_heap)
}

fn value_heap_bytes(value: &Value) -> usize {
    std::mem::size_of::<Value>().saturating_add(value_extra_heap_bytes(value))
}

fn value_extra_heap_bytes(value: &Value) -> usize {
    match value {
        Value::String(text) => text.capacity(),
        Value::Array(values) => values
            .capacity()
            .saturating_mul(std::mem::size_of::<Value>())
            .saturating_add(values.iter().fold(0usize, |total, value| {
                total.saturating_add(value_extra_heap_bytes(value))
            })),
        Value::Object(values) => values
            .len()
            .saturating_mul(std::mem::size_of::<String>() + std::mem::size_of::<Value>() + 64)
            .saturating_add(values.iter().fold(0usize, |total, (key, value)| {
                total
                    .saturating_add(key.capacity())
                    .saturating_add(value_extra_heap_bytes(value))
            })),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

fn view_cursor_token(secret: &[u8; 32], offset: usize) -> String {
    // HMAC-SHA256 keeps the offset opaque and makes reissued successors stable.
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for (index, byte) in secret.iter().enumerate() {
        ipad[index] ^= byte;
        opad[index] ^= byte;
    }
    let inner = Sha256::new()
        .chain_update(ipad)
        .chain_update((offset as u64).to_be_bytes())
        .finalize();
    let digest = Sha256::new()
        .chain_update(opad)
        .chain_update(inner)
        .finalize();
    use std::fmt::Write as _;
    let mut token = String::with_capacity(36);
    token.push_str("vc1.");
    for byte in &digest[..16] {
        write!(&mut token, "{byte:02x}").expect("writing to a String cannot fail");
    }
    token
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

    fn search_binding(query: &str, revision: &str) -> SearchCursorBinding {
        SearchCursorBinding {
            workspace_identity: "workspace-a".to_owned(),
            query: query.to_owned(),
            scope: None,
            mode: "literal".to_owned(),
            kind: None,
            source_sets: vec!["main".to_owned()],
            revisions: vec![revision.to_owned()],
            result_fingerprint: None,
            page_limit: 20,
        }
    }

    #[test]
    fn search_cursor_binds_the_question_and_revision_and_reissues_a_successor() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "rev-1");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        assert!(first.starts_with("sc1."));
        assert_eq!(
            SearchCursorStore::default().read(&first, &binding).err(),
            Some(ViewCursorError::Invalid)
        );
        let mut different = binding.clone();
        different.source_sets.push("extension".to_owned());
        assert_eq!(
            store.read(&first, &different).err(),
            Some(ViewCursorError::Invalid)
        );
        let mut other_workspace = binding.clone();
        other_workspace.workspace_identity = "workspace-b".to_owned();
        assert_eq!(
            store.read(&first, &other_workspace).err(),
            Some(ViewCursorError::Invalid)
        );
        let mut stale = binding.clone();
        stale.revisions[0] = "rev-2".to_owned();
        assert_eq!(
            store.read(&first, &stale).err(),
            Some(ViewCursorError::Stale)
        );
        let page = store.read(&first, &binding).unwrap();
        assert_eq!(page.offset, 20);
        let second = store.insert_next(&page, 40, &first).unwrap();
        assert_eq!(store.insert_next(&page, 40, &first).unwrap(), second);
        assert_eq!(store.read(&second, &binding).unwrap().offset, 40);
        assert_eq!(
            store
                .read("sc1.00000000000000000000000000000000", &binding)
                .err(),
            Some(ViewCursorError::Invalid)
        );
    }

    #[test]
    fn names_cursor_binds_kind_and_complete_ranked_answer() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding("Node", "unused");
        binding.mode = "names".to_owned();
        binding.kind = Some("Catalog".to_owned());
        binding.revisions.clear();
        binding.result_fingerprint = Some("names-sha256-v1:one".to_owned());
        let token = store.insert_first(binding.clone(), 20).unwrap();

        let mut other_kind = binding.clone();
        other_kind.kind = Some("Document".to_owned());
        assert_eq!(
            store.read(&token, &other_kind).err(),
            Some(ViewCursorError::Invalid)
        );
        let mut changed_answer = binding.clone();
        changed_answer.result_fingerprint = Some("names-sha256-v1:two".to_owned());
        assert_eq!(
            store.read(&token, &changed_answer).err(),
            Some(ViewCursorError::Stale)
        );
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_cursor_store_bounds_binding_bytes_and_reserves_its_successor() {
        let binding = search_binding("needle", "rev-1");
        let json_bytes = bounded_json_size(&binding, usize::MAX).unwrap();
        let bytes = search_snapshot_charge(&binding, json_bytes);
        let insufficient =
            SearchCursorStore::new(DEFAULT_TTL, 2, bytes + 2 * VIEW_ENTRY_CHARGE - 1);
        assert!(insufficient.insert_first(binding.clone(), 20).is_none());
        let enough = SearchCursorStore::new(DEFAULT_TTL, 2, bytes + 2 * VIEW_ENTRY_CHARGE);
        let first = enough.insert_first(binding.clone(), 20).unwrap();
        let page = enough.read(&first, &binding).unwrap();
        assert!(enough.insert_next(&page, 40, &first).is_some());

        let limited = SearchCursorStore::new(DEFAULT_TTL, 2, 2_048);
        let first = limited.insert_first(binding.clone(), 20).unwrap();
        let oversized = search_binding(&"X".repeat(3_000), "rev-1");
        assert!(limited.insert_first(oversized, 20).is_none());
        assert!(limited.read(&first, &binding).is_ok());
    }

    #[test]
    fn expired_search_cursor_is_invalid_and_releases_its_charge() {
        let store = SearchCursorStore::new(Duration::ZERO, 2, 2_048);
        let binding = search_binding("needle", "rev-1");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        assert_eq!(
            store.read(&first, &binding).err(),
            Some(ViewCursorError::Invalid)
        );
        let second = store.insert_first(binding, 40).unwrap();
        let entries = store.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(!entries.contains_key(&first));
        assert!(entries.contains_key(&second));
    }

    #[test]
    fn opaque_view_cursor_retry_is_idempotent_and_bound_to_the_complete_question() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let token = store
            .insert_snapshot(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2}), json!({"line": 3})],
                0,
            )
            .unwrap();
        assert!(token.starts_with("vc1."));
        assert!(token[4..].parse::<usize>().is_err());
        assert_eq!(
            ViewCursorStore::default()
                .read(&token, &binding, "rev-1")
                .unwrap_err(),
            ViewCursorError::Invalid
        );

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
        assert_eq!(page.snapshot.items[page.offset], json!({"line": 2}));
        let successor = store.insert_next(&page, 1, &token).unwrap();
        let replay = store.read(&token, &binding, "rev-1").unwrap();
        assert_eq!(replay.offset, page.offset);
        assert_eq!(store.insert_next(&replay, 1, &token).unwrap(), successor);
    }

    #[test]
    fn long_chain_issues_one_cursor_at_a_time_and_reissues_an_evicted_successor() {
        let store = ViewCursorStore::new(DEFAULT_TTL, 2, 4_096);
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let first = store
            .insert_snapshot(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!(1), json!(2), json!(3), json!(4)],
                0,
            )
            .unwrap();
        let first_page = store.read(&first, &binding, "rev-1").unwrap();
        let second = store.insert_next(&first_page, 1, &first).unwrap();
        let second_page = store.read(&second, &binding, "rev-1").unwrap();
        let third = store.insert_next(&second_page, 2, &second).unwrap();
        assert_eq!(
            store.read(&first, &binding, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        let third_page = store.read(&third, &binding, "rev-1").unwrap();
        let fourth = store.insert_next(&third_page, 3, &third).unwrap();
        assert_eq!(
            store.read(&second, &binding, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        store.read(&third, &binding, "rev-1").unwrap();
        let _other = store
            .insert_snapshot(
                view_binding("main:Document.Другой.Module.Object.Body", "rev-1"),
                json!({"at":"other"}),
                vec![json!(1)],
                0,
            )
            .unwrap();
        assert_eq!(
            store.read(&fourth, &binding, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        // A still-live predecessor reissues exactly its evicted successor.
        let third_page = store.read(&third, &binding, "rev-1").unwrap();
        assert_eq!(store.insert_next(&third_page, 3, &third).unwrap(), fourth);
        assert_eq!(store.read(&fourth, &binding, "rev-1").unwrap().offset, 3);
    }

    #[test]
    fn view_snapshot_byte_quota_and_lru_evict_old_chains() {
        let binding = |name| view_binding(name, "rev-1");
        let store = ViewCursorStore::new(DEFAULT_TTL, 8, 2_048);
        let insert = |name| {
            store
                .insert_snapshot(
                    binding(name),
                    json!({"at":name}),
                    vec![json!("X".repeat(800))],
                    0,
                )
                .unwrap()
        };
        let first_at = "main:Document.Первый.Module.Object.Body";
        let second_at = "main:Document.Второй.Module.Object.Body";
        let third_at = "main:Document.Третий.Module.Object.Body";
        let first = insert(first_at);
        let second = insert(second_at);
        assert_eq!(
            store.read(&first, &binding(first_at), "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        store.read(&second, &binding(second_at), "rev-1").unwrap();
        let third = insert(third_at);
        assert_eq!(
            store
                .read(&second, &binding(second_at), "rev-1")
                .unwrap_err(),
            ViewCursorError::Invalid
        );
        assert!(store.read(&third, &binding(third_at), "rev-1").is_ok());

        let lru = ViewCursorStore::new(DEFAULT_TTL, 2, 4_096);
        let first = lru
            .insert_snapshot(binding(first_at), json!({}), vec![json!(1)], 0)
            .unwrap();
        let second = lru
            .insert_snapshot(binding(second_at), json!({}), vec![json!(2)], 0)
            .unwrap();
        lru.read(&first, &binding(first_at), "rev-1").unwrap();
        let third = lru
            .insert_snapshot(binding(third_at), json!({}), vec![json!(3)], 0)
            .unwrap();
        assert_eq!(
            lru.read(&second, &binding(second_at), "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
        assert!(lru.read(&first, &binding(first_at), "rev-1").is_ok());
        assert!(lru.read(&third, &binding(third_at), "rev-1").is_ok());
    }

    #[test]
    fn snapshot_admission_reserves_room_for_the_next_cursor() {
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let node = json!({"at": binding.canonical_at});
        let items = vec![json!(1), json!(2), json!(3)];
        let json_bytes = bounded_json_size(&(&binding, &node, &items), usize::MAX).unwrap();
        let snapshot_bytes = snapshot_charge(&binding, &node, &items, json_bytes);
        let too_small =
            ViewCursorStore::new(DEFAULT_TTL, 2, snapshot_bytes + 2 * VIEW_ENTRY_CHARGE - 1);
        assert!(too_small
            .insert_snapshot(binding.clone(), node.clone(), items.clone(), 1)
            .is_none());

        let enough = ViewCursorStore::new(DEFAULT_TTL, 2, snapshot_bytes + 2 * VIEW_ENTRY_CHARGE);
        let first = enough
            .insert_snapshot(binding.clone(), node, items, 1)
            .unwrap();
        let page = enough.read(&first, &binding, "rev-1").unwrap();
        let second = enough.insert_next(&page, 2, &first).unwrap();
        assert_eq!(enough.read(&second, &binding, "rev-1").unwrap().offset, 2);
    }

    #[test]
    fn expired_view_cursor_releases_its_snapshot_charge() {
        let store = ViewCursorStore::new(Duration::ZERO, 8, 4_096);
        let first = view_binding("main:Document.Первый.Module.Object.Body", "rev-1");
        let second = view_binding("main:Document.Второй.Module.Object.Body", "rev-1");
        let first_token = store
            .insert_snapshot(first.clone(), json!({}), vec![json!(1)], 0)
            .unwrap();
        let second_token = store
            .insert_snapshot(second.clone(), json!({}), vec![json!(2)], 0)
            .unwrap();
        let entries = store.entries.lock().unwrap();
        assert_eq!(
            entries.len(),
            1,
            "expired snapshot must be discarded on admission"
        );
        assert!(!entries.contains_key(&first_token));
        assert!(entries.contains_key(&second_token));
    }

    #[test]
    fn many_small_items_are_charged_for_retained_heap_not_only_json() {
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let items = vec![Value::Null; 10_000];
        let json_bytes = bounded_json_size(&(&binding, json!({}), &items), usize::MAX).unwrap();
        assert!(json_bytes < 100_000);
        let store = ViewCursorStore::new(DEFAULT_TTL, 128, 100_000);
        assert!(store
            .insert_snapshot(binding, json!({}), items, 1)
            .is_none());
    }

    #[test]
    fn exact_revision_change_is_stale_but_tampering_and_expiry_are_invalid() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let token = store
            .insert_snapshot(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2})],
                0,
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
            .insert_snapshot(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                vec![json!({"line": 2})],
                0,
            )
            .unwrap();
        assert_eq!(
            expiring.read(&token, &binding, "rev-1").unwrap_err(),
            ViewCursorError::Invalid
        );
    }
}
