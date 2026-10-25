//! Bounded in-memory store for deferred typed reader results.
//!
//! The store keeps an immutable snapshot of a full typed `OperationResult.data`
//! so a continuation call can serve byte-stable slices without re-reading the
//! source. Legacy result references use TTL and LRU. View continuations retain
//! their promised snapshot for the process lifetime and refuse new admission
//! before exceeding their cumulative retained-state budget.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use unica_execution_contract::invocation::DomainResult;
use unica_refusal::{RefusalCode, RefusalDetail};

use serde_json::Value;

pub const DEFAULT_TTL: Duration = Duration::from_secs(15 * 60);
pub const DEFAULT_MAX_ENTRIES: usize = 32;
pub const DEFAULT_MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
// Temporary and unproven retained-state budget; see #1119. This does not
// bound process RSS or transient source reads, JSON clones and index growth.
const VIEW_CURSOR_PAYLOAD_BYTES: usize = 17;
const VIEW_CURSOR_CIPHERTEXT_BYTES: usize = VIEW_CURSOR_PAYLOAD_BYTES + 16;
const VIEW_CURSOR_TOKEN_BYTES: usize = 4 + 2 * VIEW_CURSOR_CIPHERTEXT_BYTES;
const VIEW_CURSOR_AAD: &[u8] = b"unica.view.cursor.vc2";

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

/// A continuation binds the original question and an immutable issued result.
/// Its snapshot ID is internal and does not track current source contents.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ViewCursorBinding {
    pub canonical_at: String,
    pub projection: String,
    pub source_set_identity: String,
    pub snapshot_id: String,
    pub page_limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewCursorQuestion {
    pub canonical_at: String,
    pub projection: String,
    pub source_set_identity: String,
    pub page_limit: usize,
}

impl ViewCursorBinding {
    fn has_same_question(&self, other: &Self) -> bool {
        self.canonical_at == other.canonical_at
            && self.projection == other.projection
            && self.source_set_identity == other.source_set_identity
            && self.page_limit == other.page_limit
    }

    pub fn question(&self) -> ViewCursorQuestion {
        ViewCursorQuestion {
            canonical_at: self.canonical_at.clone(),
            projection: self.projection.clone(),
            source_set_identity: self.source_set_identity.clone(),
            page_limit: self.page_limit,
        }
    }
}

#[derive(Default)]
struct ViewCursorState {
    key: Option<zeroize::Zeroizing<[u8; 64]>>,
    snapshots: Vec<Arc<ViewCollectionSnapshot>>,
    snapshot_bytes: usize,
    payload_bytes: usize,
    payload_count: usize,
}

struct ViewCollectionPayload {
    node: Value,
    items: Box<dyn SpilledItems>,
    digest: [u8; 32],
}

impl std::fmt::Debug for ViewCollectionPayload {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ViewCollectionPayload")
            .field("node", &self.node)
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

/// Где лежат элементы снимка. Компонент сам файлов не открывает: хранение
/// на диске подставляет платформенный слой продукта, а по умолчанию элементы
/// остаются в памяти.
pub trait SnapshotSpill: Send + Sync {
    /// Удерживаемые байты снимка с этим узлом и элементами в этом хранилище.
    #[allow(clippy::ptr_arg)] // the retained charge includes the vector capacity
    fn payload_charge(&self, node: &Value, items: &Vec<Value>) -> Result<usize, ViewCursorError>;
    fn store(&self, items: Vec<Value>) -> Result<Box<dyn SpilledItems>, ViewCursorError>;
}

/// Элементы одного снимка в порядке коллекции.
pub trait SpilledItems: Send + Sync {
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Не больше `count` элементов, начиная с `start`.
    fn read(&self, start: usize, count: usize) -> Result<Vec<Value>, ViewCursorError>;
}

/// Элементы снимка в памяти процесса.
#[derive(Debug, Default)]
pub struct MemorySpill;

impl SnapshotSpill for MemorySpill {
    fn payload_charge(&self, node: &Value, items: &Vec<Value>) -> Result<usize, ViewCursorError> {
        snapshot_payload_charge(node, items)
    }

    fn store(&self, items: Vec<Value>) -> Result<Box<dyn SpilledItems>, ViewCursorError> {
        Ok(Box::new(MemoryItems(items)))
    }
}

struct MemoryItems(Vec<Value>);

impl SpilledItems for MemoryItems {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn read(&self, start: usize, count: usize) -> Result<Vec<Value>, ViewCursorError> {
        let end = start.saturating_add(count).min(self.0.len());
        Ok(self.0.get(start..end).unwrap_or_default().to_vec())
    }
}

/// Удерживаемое снимком, чьи элементы лежат вне памяти: узел и границы
/// элементов в памяти плюс байты самих элементов в их хранилище.
pub fn spilled_payload_charge(node: &Value, items: &[Value]) -> Result<usize, ViewCursorError> {
    let node_json = bounded_json_size_attempt(node, usize::MAX)
        .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
    let node_heap = view_add(
        std::mem::size_of::<ViewCollectionPayload>(),
        value_extra_heap_bytes(node)?,
    )?;
    let bounds = view_mul(view_add(items.len(), 1)?, std::mem::size_of::<u64>())?;
    let mut item_bytes = 0usize;
    for item in items {
        let bytes = bounded_json_size_attempt(item, usize::MAX)
            .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
        item_bytes = view_add(item_bytes, bytes)?;
    }
    view_add(
        view_add(node_json.max(node_heap), bounds)?,
        view_add(item_bytes, 2 * std::mem::size_of::<usize>())?,
    )
}

/// Цифровой отпечаток элементов: равные снимки делят одно хранилище.
fn items_digest(items: &[Value]) -> Result<[u8; 32], ViewCursorError> {
    let mut hasher = Sha256::new();
    for item in items {
        // The length prefix keeps item boundaries part of the identity.
        let bytes =
            serde_json::to_vec(item).map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(&bytes);
    }
    hasher.update((items.len() as u64).to_be_bytes());
    Ok(hasher.finalize().into())
}

#[derive(Debug)]
pub struct ViewCollectionSnapshot {
    pub binding: ViewCursorBinding,
    payload: Arc<ViewCollectionPayload>,
    id: u64,
}

impl ViewCollectionSnapshot {
    pub fn node(&self) -> &Value {
        &self.payload.node
    }

    /// Число элементов снимка.
    pub fn len(&self) -> usize {
        self.payload.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Не больше `count` элементов, начиная с `start`, в порядке снимка.
    pub fn read_items(&self, start: usize, count: usize) -> Result<Vec<Value>, ViewCursorError> {
        self.payload.items.read(start, count)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn items(&self) -> Vec<Value> {
        self.read_items(0, self.len())
            .expect("snapshot items are readable")
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, PartialEq, Eq)]
pub struct ViewRetainedUsage {
    pub records: usize,
    pub unique_payloads: usize,
    pub record_bytes: usize,
    pub payload_bytes: usize,
    pub index_capacity: usize,
    pub retained_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct StoredViewCursor {
    pub snapshot: Arc<ViewCollectionSnapshot>,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewCursorError {
    Invalid,
    CapacityExceeded,
    OwnerUnavailable,
    EntropyUnavailable,
    TokenUnavailable,
    OffsetNotRepresentable,
    AllocationUnavailable,
    StorageUnavailable,
}

impl ViewCursorError {
    pub const fn code(self) -> RefusalCode {
        match self {
            Self::Invalid => RefusalCode::InvalidCursor,
            Self::CapacityExceeded => RefusalCode::ResultTooLarge,
            Self::OwnerUnavailable => RefusalDetail::CachePoisoned.code(),
            Self::EntropyUnavailable => RefusalDetail::ProviderAbsent.code(),
            Self::TokenUnavailable
            | Self::OffsetNotRepresentable
            | Self::AllocationUnavailable
            | Self::StorageUnavailable => RefusalCode::ProviderFailed,
        }
    }

    pub const fn detail(self) -> Option<RefusalDetail> {
        match self {
            Self::OwnerUnavailable => Some(RefusalDetail::CachePoisoned),
            Self::EntropyUnavailable => Some(RefusalDetail::ProviderAbsent),
            _ => None,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Invalid => "view cursor is invalid, expired, or belongs to another question",
            Self::CapacityExceeded => "new view snapshot exceeds the temporary retained-state budget; earlier continuations remain available",
            Self::OwnerUnavailable => "view cursor owner is unavailable",
            Self::EntropyUnavailable => "view cursor entropy is unavailable",
            Self::TokenUnavailable => "view continuation could not be authenticated or issued",
            Self::OffsetNotRepresentable => "view continuation size or offset is not representable",
            Self::AllocationUnavailable => "view snapshot index allocation is unavailable",
            Self::StorageUnavailable => "view snapshot storage is unavailable",
        }
    }

    pub fn into_result(self, at: Option<String>) -> DomainResult {
        match self.detail() {
            Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, self.message()),
            None => DomainResult::canonical_rejection(at, self.code(), self.message()),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
type ViewEntropy = fn(&mut [u8]) -> Result<(), ViewCursorError>;

/// Optional observation port implemented outside the application layer.
pub trait ViewCapacityObserver: Send + Sync {
    fn record_view(&self, bytes: u64, exact: bool, admitted: bool);
    fn record_view_admission_refusal(&self);
}

/// One append-only process owner for immutable view snapshots. An encrypted
/// snapshot index and offset need no retained per-page entries. New admission
/// cannot reclaim any snapshot already promised to a caller.
pub struct ViewCursorStore {
    max_total_bytes: usize,
    spill: Arc<dyn SnapshotSpill>,
    state: Mutex<ViewCursorState>,
    capacity_observer: Option<Arc<dyn ViewCapacityObserver>>,
    #[cfg(any(test, feature = "test-support"))]
    entropy: Option<ViewEntropy>,
}

/// A search continuation binds the exact question through its digest. The
/// source is read again; neither the question nor the result is retained here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SearchCursorBinding {
    pub workspace_identity: String,
    pub query: String,
    pub scope: Option<String>,
    pub mode: String,
    pub kind: Option<String>,
    pub source_sets: Vec<String>,
    /// Names have no source revision lease. Their complete ranked answer and
    /// public coverage instead identify the replayed stream.
    pub result_fingerprint: Option<String>,
    pub page_limit: usize,
}

const SEARCH_CURSOR_PAYLOAD_BYTES: usize = 106;
const SEARCH_CURSOR_CIPHERTEXT_BYTES: usize = SEARCH_CURSOR_PAYLOAD_BYTES + 16;
const SEARCH_CURSOR_TOKEN_BYTES: usize = 4 + 2 * SEARCH_CURSOR_CIPHERTEXT_BYTES;
const SEARCH_CURSOR_AAD: &[u8] = b"unica.search.cursor.sc2";
/// A keyed continuation (`sc3.`) carries the rank of the last item shown
/// after the fixed payload, so the next page starts after it instead of
/// re-ranking everything before it.
const SEARCH_CURSOR_KEYED_AAD: &[u8] = b"unica.search.cursor.sc3";
/// A resumable continuation (`sc4.`) carries the place a paged text search
/// resumes at after the fixed payload: the matches already delivered in the
/// file it re-enters, the files left uncovered before it and that file.
const SEARCH_CURSOR_RESUME_AAD: &[u8] = b"unica.search.cursor.sc4";
/// The longest resume path a cursor carries: a source-set name and one
/// relative path, far below what a file system names.
const SEARCH_RESUME_PATH_MAX_BYTES: usize = 64 * 1024;

/// Where a paged text search resumes: the file it re-enters, how many of
/// that file's matches were already delivered, and how many files before it
/// stayed uncovered. A cursor of any other search keeps
/// [`SearchResume::START`], an empty `file`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResume {
    pub file: Vec<u8>,
    pub hits: u64,
    pub uncovered: u64,
}

impl SearchResume {
    pub const START: Self = Self {
        file: Vec::new(),
        hits: 0,
        uncovered: 0,
    };
}

/// What a token carries after the fixed payload.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SearchCursorKind {
    Fixed,
    Keyed,
    Resume,
}

impl SearchCursorKind {
    const fn aad(self) -> &'static [u8] {
        match self {
            Self::Fixed => SEARCH_CURSOR_AAD,
            Self::Keyed => SEARCH_CURSOR_KEYED_AAD,
            Self::Resume => SEARCH_CURSOR_RESUME_AAD,
        }
    }

    const fn prefix(self) -> &'static str {
        match self {
            Self::Fixed => "sc2.",
            Self::Keyed => "sc3.",
            Self::Resume => "sc4.",
        }
    }
}

/// An authenticated token: its owner, decrypted payload and tail kind.
type OpenedSearchCursor = (
    Arc<SearchCursorOwner>,
    zeroize::Zeroizing<Vec<u8>>,
    SearchCursorKind,
);

/// The tail a token is issued with.
enum SearchCursorTail<'a> {
    None,
    After(&'a SearchAfter),
    Resume(&'a SearchResume),
}

impl<'a> From<Option<&'a SearchAfter>> for SearchCursorTail<'a> {
    fn from(after: Option<&'a SearchAfter>) -> Self {
        after.map_or(Self::None, Self::After)
    }
}

/// The rank key of the last item a page showed: the next page holds only
/// what ranks after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchAfter {
    pub score: u64,
    pub at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchCursorError {
    Invalid,
    Stale,
    OwnerUnavailable,
    EntropyUnavailable,
    TokenUnavailable,
    OffsetNotRepresentable,
}

impl SearchCursorError {
    pub const fn code(self) -> RefusalCode {
        match self {
            Self::Invalid => RefusalCode::InvalidCursor,
            Self::Stale => RefusalCode::StaleCursor,
            Self::OwnerUnavailable => RefusalDetail::CachePoisoned.code(),
            Self::EntropyUnavailable => RefusalDetail::ProviderAbsent.code(),
            Self::TokenUnavailable | Self::OffsetNotRepresentable => RefusalCode::ProviderFailed,
        }
    }

    pub fn into_result(self, at: Option<String>) -> DomainResult {
        let detail = match self {
            Self::OwnerUnavailable => Some(RefusalDetail::CachePoisoned),
            Self::EntropyUnavailable => Some(RefusalDetail::ProviderAbsent),
            _ => None,
        };
        match detail {
            Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, self.message()),
            None => DomainResult::canonical_rejection(at, self.code(), self.message()),
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::Invalid => "search cursor does not belong to this question and owner",
            Self::Stale => "search cursor answer or coverage has changed",
            Self::OwnerUnavailable => "search cursor owner is unavailable",
            Self::EntropyUnavailable => "search cursor entropy is unavailable",
            Self::TokenUnavailable => "search continuation could not be authenticated or issued",
            Self::OffsetNotRepresentable => "search continuation offset is not representable",
        }
    }
}

struct SearchCursorOwner {
    key: zeroize::Zeroizing<[u8; 64]>,
}

impl std::fmt::Debug for SearchCursorOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SearchCursorOwner")
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct StoredSearchCursor {
    owner: Arc<SearchCursorOwner>,
    payload: zeroize::Zeroizing<[u8; SEARCH_CURSOR_PAYLOAD_BYTES]>,
    /// Items shown before this page. A keyed (name) continuation resumes
    /// by `after` and keeps the offset only as a page count.
    pub offset: usize,
    pub after: Option<SearchAfter>,
    /// Where a paged text search resumes; [`SearchResume::START`] otherwise.
    pub resume: SearchResume,
}

#[cfg(any(test, feature = "test-support"))]
type SearchEntropy = fn(&mut [u8]) -> Result<(), SearchCursorError>;

/// Search retains one owner key, not the questions, results or issued tokens.
/// An authenticated token carries its fixed record; the answer is read again.
#[derive(Default)]
pub struct SearchCursorStore {
    owner: Mutex<Option<Arc<SearchCursorOwner>>>,
    #[cfg(any(test, feature = "test-support"))]
    entropy: Option<SearchEntropy>,
}

impl SearchCursorStore {
    pub fn insert_first(
        &self,
        binding: SearchCursorBinding,
        offset: usize,
    ) -> Result<String, SearchCursorError> {
        self.insert_first_after(binding, offset, None)
    }

    /// The first continuation of a question, continuing after `after` when
    /// the search pages by rank.
    pub fn insert_first_after(
        &self,
        binding: SearchCursorBinding,
        offset: usize,
        after: Option<&SearchAfter>,
    ) -> Result<String, SearchCursorError> {
        self.insert_first_with(binding, offset, after.into())
    }

    /// The first continuation of a paged text search that resumes at
    /// `resume`.
    pub fn insert_first_resumable(
        &self,
        binding: SearchCursorBinding,
        offset: usize,
        resume: &SearchResume,
    ) -> Result<String, SearchCursorError> {
        self.insert_first_with(binding, offset, SearchCursorTail::Resume(resume))
    }

    fn insert_first_with(
        &self,
        binding: SearchCursorBinding,
        offset: usize,
        tail: SearchCursorTail<'_>,
    ) -> Result<String, SearchCursorError> {
        let mut payload = zeroize::Zeroizing::new([0u8; SEARCH_CURSOR_PAYLOAD_BYTES]);
        payload[0] = 1;
        payload[1..33].copy_from_slice(&search_question_digest(&binding)?);
        let (present, digest) = search_answer_digest(&binding);
        payload[33] = present;
        payload[34..66].copy_from_slice(&digest);
        payload[98..106].copy_from_slice(&search_offset_bytes(offset)?);
        let owner = {
            let mut current = self
                .owner
                .lock()
                .map_err(|_| SearchCursorError::OwnerUnavailable)?;
            if current.is_none() {
                let mut key = zeroize::Zeroizing::new([0u8; 64]);
                self.fill_entropy(&mut key[..])?;
                *current = Some(Arc::new(SearchCursorOwner { key }));
            }
            Arc::clone(
                current
                    .as_ref()
                    .ok_or(SearchCursorError::OwnerUnavailable)?,
            )
        };
        // A failure here leaves the installed owner and all earlier tokens intact.
        self.fill_entropy(&mut payload[66..98])?;
        issue_search_cursor(&owner, &payload, tail)
    }

    pub fn read(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<StoredSearchCursor, SearchCursorError> {
        let (owner, plaintext, kind) = self.open_question(token, expected)?;
        let (present, digest) = search_answer_digest(expected);
        if plaintext[33] != present || plaintext[34..66] != digest {
            return Err(SearchCursorError::Stale);
        }
        let (offset, after, resume) = search_cursor_position(&plaintext, kind)?;
        let mut payload = zeroize::Zeroizing::new([0u8; SEARCH_CURSOR_PAYLOAD_BYTES]);
        payload.copy_from_slice(&plaintext[..SEARCH_CURSOR_PAYLOAD_BYTES]);
        Ok(StoredSearchCursor {
            owner,
            payload,
            offset,
            after,
            resume,
        })
    }

    /// Authenticates a token and its question before any answer exists: the
    /// format, this owner and the question binding (query, scope, mode,
    /// kind, source sets, page limit). The answer half — whether the result
    /// or coverage changed — needs the answer and stays with [`Self::read`].
    pub fn verify_question(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<(), SearchCursorError> {
        self.open_question(token, expected).map(drop)
    }

    /// The position of an authenticated token whose question matches — its
    /// offset and, for a keyed token, the rank it continues after — read
    /// before the answer exists so a search keeps only the ranks it will
    /// show; [`Self::read`] still checks the answer afterwards.
    pub fn question_position(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<(usize, Option<SearchAfter>), SearchCursorError> {
        let (_, plaintext, kind) = self.open_question(token, expected)?;
        search_cursor_position(&plaintext, kind).map(|(offset, after, _)| (offset, after))
    }

    /// The offset and resume place of an authenticated token whose question
    /// matches, read before the answer exists so a paged text search starts
    /// where the cursor points; [`Self::read`] checks the answer afterwards.
    pub fn resume_position(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<(usize, SearchResume), SearchCursorError> {
        let (_, plaintext, kind) = self.open_question(token, expected)?;
        search_cursor_position(&plaintext, kind).map(|(offset, _, resume)| (offset, resume))
    }

    fn open_question(
        &self,
        token: &str,
        expected: &SearchCursorBinding,
    ) -> Result<OpenedSearchCursor, SearchCursorError> {
        use aes_siv::KeyInit;
        let (ciphertext, kind) = decode_search_cursor(token)?;
        let owner = self
            .owner
            .lock()
            .map_err(|_| SearchCursorError::OwnerUnavailable)?
            .as_ref()
            .cloned()
            .ok_or(SearchCursorError::Invalid)?;
        let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(&owner.key[..])
            .map_err(|_| SearchCursorError::TokenUnavailable)?;
        let plaintext = zeroize::Zeroizing::new(
            cipher
                .decrypt([kind.aad()], &ciphertext)
                .map_err(|_| SearchCursorError::Invalid)?,
        );
        let length_fits = match kind {
            SearchCursorKind::Fixed => plaintext.len() == SEARCH_CURSOR_PAYLOAD_BYTES,
            SearchCursorKind::Keyed => plaintext.len() > SEARCH_CURSOR_PAYLOAD_BYTES + 8,
            SearchCursorKind::Resume => plaintext.len() > SEARCH_CURSOR_PAYLOAD_BYTES + 16,
        };
        if !length_fits
            || plaintext[0] != 1
            || plaintext[33] > 1
            || (plaintext[33] == 0 && plaintext[34..66].iter().any(|byte| *byte != 0))
        {
            return Err(SearchCursorError::Invalid);
        }
        if plaintext[1..33] != search_question_digest(expected)? {
            return Err(SearchCursorError::Invalid);
        }
        Ok((owner, plaintext, kind))
    }

    pub fn insert_next(
        &self,
        current: &StoredSearchCursor,
        offset: usize,
    ) -> Result<String, SearchCursorError> {
        self.insert_next_after(current, offset, None)
    }

    /// The successor of `current`, continuing after `after` when the search
    /// pages by rank.
    pub fn insert_next_after(
        &self,
        current: &StoredSearchCursor,
        offset: usize,
        after: Option<&SearchAfter>,
    ) -> Result<String, SearchCursorError> {
        let owner = self
            .owner
            .lock()
            .map_err(|_| SearchCursorError::OwnerUnavailable)?
            .as_ref()
            .cloned()
            .ok_or(SearchCursorError::Invalid)?;
        if !Arc::ptr_eq(&owner, &current.owner) {
            return Err(SearchCursorError::Invalid);
        }
        let mut payload = current.payload.clone();
        payload[98..106].copy_from_slice(&search_offset_bytes(offset)?);
        issue_search_cursor(&owner, &payload, after.into())
    }

    /// The successor of `current` whose answer half names a different
    /// answer and which resumes at `resume`: a continuation bound to what
    /// the caller has received so far rather than to one fixed answer. A
    /// replay of the same page issues the same token.
    pub fn insert_next_answer(
        &self,
        current: &StoredSearchCursor,
        answer: &SearchCursorBinding,
        offset: usize,
        resume: &SearchResume,
    ) -> Result<String, SearchCursorError> {
        let owner = self
            .owner
            .lock()
            .map_err(|_| SearchCursorError::OwnerUnavailable)?
            .as_ref()
            .cloned()
            .ok_or(SearchCursorError::Invalid)?;
        if !Arc::ptr_eq(&owner, &current.owner)
            || current.payload[1..33] != search_question_digest(answer)?
        {
            return Err(SearchCursorError::Invalid);
        }
        let mut payload = current.payload.clone();
        let (present, digest) = search_answer_digest(answer);
        payload[33] = present;
        payload[34..66].copy_from_slice(&digest);
        payload[98..106].copy_from_slice(&search_offset_bytes(offset)?);
        issue_search_cursor(&owner, &payload, SearchCursorTail::Resume(resume))
    }

    pub fn token_probe() -> String {
        encode_search_cursor(
            &[0u8; SEARCH_CURSOR_CIPHERTEXT_BYTES],
            SearchCursorKind::Fixed,
        )
    }

    /// A keyed token of the same length as one continuing after an address
    /// of `at_bytes` bytes.
    pub fn keyed_token_probe(at_bytes: usize) -> String {
        encode_search_cursor(
            &vec![0u8; SEARCH_CURSOR_CIPHERTEXT_BYTES + 8 + at_bytes],
            SearchCursorKind::Keyed,
        )
    }

    /// A resumable token of the same length as one re-entering a file named
    /// by `file_bytes` bytes.
    pub fn resume_token_probe(file_bytes: usize) -> String {
        encode_search_cursor(
            &vec![0u8; SEARCH_CURSOR_CIPHERTEXT_BYTES + 16 + file_bytes],
            SearchCursorKind::Resume,
        )
    }

    pub fn next_offset(offset: usize, consumed: usize) -> Result<usize, SearchCursorError> {
        offset
            .checked_add(consumed)
            .ok_or(SearchCursorError::OffsetNotRepresentable)
    }

    fn fill_entropy(&self, bytes: &mut [u8]) -> Result<(), SearchCursorError> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fill) = self.entropy {
            return fill(bytes);
        }
        getrandom::fill(bytes).map_err(|_| SearchCursorError::EntropyUnavailable)
    }
}

fn search_offset_bytes(offset: usize) -> Result<[u8; 8], SearchCursorError> {
    u64::try_from(offset)
        .map(u64::to_be_bytes)
        .map_err(|_| SearchCursorError::OffsetNotRepresentable)
}

fn issue_search_cursor(
    owner: &SearchCursorOwner,
    payload: &[u8; SEARCH_CURSOR_PAYLOAD_BYTES],
    tail: SearchCursorTail<'_>,
) -> Result<String, SearchCursorError> {
    use aes_siv::KeyInit;
    let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(&owner.key[..])
        .map_err(|_| SearchCursorError::TokenUnavailable)?;
    let mut plaintext = zeroize::Zeroizing::new(payload.to_vec());
    let kind = match tail {
        SearchCursorTail::None => SearchCursorKind::Fixed,
        SearchCursorTail::After(after) => {
            if after.at.is_empty() {
                return Err(SearchCursorError::TokenUnavailable);
            }
            plaintext.extend_from_slice(&after.score.to_be_bytes());
            plaintext.extend_from_slice(after.at.as_bytes());
            SearchCursorKind::Keyed
        }
        SearchCursorTail::Resume(resume) => {
            // A path past the bound is no continuation the token can carry.
            if resume.file.is_empty() || resume.file.len() > SEARCH_RESUME_PATH_MAX_BYTES {
                return Err(SearchCursorError::OffsetNotRepresentable);
            }
            plaintext.extend_from_slice(&resume.hits.to_be_bytes());
            plaintext.extend_from_slice(&resume.uncovered.to_be_bytes());
            plaintext.extend_from_slice(&resume.file);
            SearchCursorKind::Resume
        }
    };
    let ciphertext = cipher
        .encrypt([kind.aad()], &plaintext[..])
        .map_err(|_| SearchCursorError::TokenUnavailable)?;
    if ciphertext.len() != plaintext.len() + 16 {
        return Err(SearchCursorError::TokenUnavailable);
    }
    Ok(encode_search_cursor(&ciphertext, kind))
}

/// The offset of a decrypted payload and, past the fixed part, the rank of
/// the item it continues after or the place it resumes at.
fn search_cursor_position(
    plaintext: &[u8],
    kind: SearchCursorKind,
) -> Result<(usize, Option<SearchAfter>, SearchResume), SearchCursorError> {
    let raw_offset: [u8; 8] = plaintext[98..106]
        .try_into()
        .map_err(|_| SearchCursorError::Invalid)?;
    let offset =
        usize::try_from(u64::from_be_bytes(raw_offset)).map_err(|_| SearchCursorError::Invalid)?;
    let tail = &plaintext[SEARCH_CURSOR_PAYLOAD_BYTES..];
    let number = |bytes: &[u8]| {
        bytes
            .try_into()
            .map(u64::from_be_bytes)
            .map_err(|_| SearchCursorError::Invalid)
    };
    match kind {
        SearchCursorKind::Fixed => Ok((offset, None, SearchResume::START)),
        SearchCursorKind::Keyed => {
            let (score, at) = tail.split_at_checked(8).ok_or(SearchCursorError::Invalid)?;
            let at = std::str::from_utf8(at).map_err(|_| SearchCursorError::Invalid)?;
            if at.is_empty() {
                return Err(SearchCursorError::Invalid);
            }
            let after = SearchAfter {
                score: number(score)?,
                at: at.to_owned(),
            };
            Ok((offset, Some(after), SearchResume::START))
        }
        SearchCursorKind::Resume => {
            let (counts, file) = tail
                .split_at_checked(16)
                .ok_or(SearchCursorError::Invalid)?;
            if file.is_empty() {
                return Err(SearchCursorError::Invalid);
            }
            let resume = SearchResume {
                file: file.to_vec(),
                hits: number(&counts[..8])?,
                uncovered: number(&counts[8..])?,
            };
            Ok((offset, None, resume))
        }
    }
}

fn encode_search_cursor(ciphertext: &[u8], kind: SearchCursorKind) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(4 + 2 * ciphertext.len());
    token.push_str(kind.prefix());
    for byte in ciphertext {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 15)]));
    }
    token
}

/// The ciphertext of a token and whether it is keyed. A keyed token is
/// longer than the fixed one by its rank and address. Its length is bounded
/// only by the request line that carries it, as every argument is: decoding
/// and authenticating cost no more than reading that line, and decryption
/// refuses anything the owner did not issue.
fn decode_search_cursor(token: &str) -> Result<(Vec<u8>, SearchCursorKind), SearchCursorError> {
    let tailed = token.len() > SEARCH_CURSOR_TOKEN_BYTES + 16 && token.len().is_multiple_of(2);
    let kind = if token.len() == SEARCH_CURSOR_TOKEN_BYTES && token.starts_with("sc2.") {
        SearchCursorKind::Fixed
    } else if tailed && token.starts_with("sc3.") {
        SearchCursorKind::Keyed
    } else if tailed
        && token.len() > SEARCH_CURSOR_TOKEN_BYTES + 32
        && token.len() <= SEARCH_CURSOR_TOKEN_BYTES + 32 + 2 * SEARCH_RESUME_PATH_MAX_BYTES
        && token.starts_with("sc4.")
    {
        SearchCursorKind::Resume
    } else {
        return Err(SearchCursorError::Invalid);
    };
    fn nibble(byte: u8) -> Result<u8, SearchCursorError> {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(SearchCursorError::Invalid),
        }
    }
    let mut ciphertext = vec![0u8; (token.len() - 4) / 2];
    for (pair, out) in token.as_bytes()[4..]
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&mut ciphertext)
    {
        *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok((ciphertext, kind))
}

fn search_question_digest(binding: &SearchCursorBinding) -> Result<[u8; 32], SearchCursorError> {
    #[derive(serde::Serialize)]
    struct Question<'a> {
        workspace_identity: &'a str,
        query: &'a str,
        scope: &'a Option<String>,
        mode: &'a str,
        kind: &'a Option<String>,
        source_sets: &'a [String],
        page_limit: usize,
    }
    struct HashWriter(Sha256);
    impl Write for HashWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut writer = HashWriter(Sha256::new());
    writer.0.update(b"unica.search.question.sc2\0");
    serde_json::to_writer(
        &mut writer,
        &Question {
            workspace_identity: &binding.workspace_identity,
            query: &binding.query,
            scope: &binding.scope,
            mode: &binding.mode,
            kind: &binding.kind,
            source_sets: &binding.source_sets,
            page_limit: binding.page_limit,
        },
    )
    .map_err(|_| SearchCursorError::TokenUnavailable)?;
    Ok(writer.0.finalize().into())
}

fn search_answer_digest(binding: &SearchCursorBinding) -> (u8, [u8; 32]) {
    match &binding.result_fingerprint {
        None => (0, [0u8; 32]),
        Some(answer) => {
            let mut digest = Sha256::new();
            digest.update(b"unica.search.answer.sc2\0");
            digest.update(answer.as_bytes());
            (1, digest.finalize().into())
        }
    }
}

impl Default for ViewCursorStore {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_TOTAL_BYTES)
    }
}

impl ViewCursorStore {
    pub fn new(max_total_bytes: usize) -> Self {
        Self {
            max_total_bytes,
            spill: Arc::new(MemorySpill),
            state: Mutex::new(ViewCursorState::default()),
            capacity_observer: None,
            #[cfg(any(test, feature = "test-support"))]
            entropy: None,
        }
    }

    pub fn with_capacity_observer(mut self, observer: Arc<dyn ViewCapacityObserver>) -> Self {
        self.capacity_observer = Some(observer);
        self
    }

    /// Хранилище элементов снимков; по умолчанию — память процесса.
    pub fn with_spill(mut self, spill: Arc<dyn SnapshotSpill>) -> Self {
        self.spill = spill;
        self
    }

    pub fn insert_snapshot(
        &self,
        binding: ViewCursorBinding,
        node: Value,
        items: Vec<Value>,
        offset: usize,
    ) -> Result<String, ViewCursorError> {
        if offset >= items.len() {
            return Err(ViewCursorError::Invalid);
        }
        if let Err(lower_bound) =
            bounded_json_size_attempt(&(&binding, &node, &items), self.max_total_bytes)
        {
            if let Some(observer) = &self.capacity_observer {
                observer.record_view(lower_bound as u64, false, false);
            }
            return Err(ViewCursorError::CapacityExceeded);
        }
        // serde_json::Map does not expose its IndexMap capacity. Rebuild only
        // objects, preserving order and moving their owned keys/values, so an
        // arbitrarily preallocated sparse map is not retained uncharged.
        let node = compact_view_objects(node);
        let items = items
            .into_iter()
            .map(compact_view_objects)
            .collect::<Vec<_>>();
        let charges = snapshot_record_charge(&binding).and_then(|record| {
            let payload = self.spill.payload_charge(&node, &items)?;
            Ok((record, payload, view_add(record, payload)?))
        });
        let (record_bytes, payload_bytes, bytes) = match charges {
            Ok(charges) => charges,
            Err(error) => {
                if let Some(observer) = &self.capacity_observer {
                    observer.record_view_admission_refusal();
                }
                return Err(error);
            }
        };
        // The complete fresh input is observed independently of whether its
        // immutable payload is already retained by this owner.
        let result = self.admit_snapshot(binding, node, items, offset, record_bytes, payload_bytes);
        if let Some(observer) = &self.capacity_observer {
            observer.record_view(bytes as u64, true, result.is_ok());
        }
        result
    }

    fn admit_snapshot(
        &self,
        binding: ViewCursorBinding,
        node: Value,
        items: Vec<Value>,
        offset: usize,
        record_bytes: usize,
        payload_bytes: usize,
    ) -> Result<String, ViewCursorError> {
        // The digest is computed before the owner lock: a large snapshot must
        // not stall reads and continuations of the others.
        let digest = items_digest(&items)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ViewCursorError::OwnerUnavailable)?;
        // Every caller has already completed a fresh full projection. Preserve
        // each issued binding; only the immutable equal data may share storage.
        let same_payload = |snapshot: &ViewCollectionSnapshot| {
            snapshot.node() == &node
                && snapshot.payload.digest == digest
                && snapshot.len() == items.len()
        };
        let mut shared_payload = None;
        for snapshot in &state.snapshots {
            if snapshot.binding == binding && same_payload(snapshot) {
                let key = state
                    .key
                    .as_ref()
                    .ok_or(ViewCursorError::OwnerUnavailable)?;
                return issue_view_cursor(key, snapshot.id, offset);
            }
            if shared_payload.is_none()
                && snapshot.binding.has_same_question(&binding)
                && same_payload(snapshot)
            {
                shared_payload = Some(Arc::clone(&snapshot.payload));
            }
        }
        let len = state
            .snapshots
            .len()
            .checked_add(1)
            .ok_or(ViewCursorError::OffsetNotRepresentable)?;
        let id = u64::try_from(len).map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
        let new_payload_bytes = if shared_payload.is_none() {
            payload_bytes
        } else {
            0
        };
        let snapshot_bytes = view_add(
            view_add(state.snapshot_bytes, record_bytes)?,
            new_payload_bytes,
        )?;
        let total_payload_bytes = view_add(state.payload_bytes, new_payload_bytes)?;
        let payload_count = view_add(state.payload_count, usize::from(shared_payload.is_none()))?;
        let mut grown = None;
        if len > state.snapshots.capacity() {
            let amortized = state
                .snapshots
                .capacity()
                .checked_mul(2)
                .unwrap_or(len)
                .max(len)
                .max(4);
            // A spare-capacity policy must not refuse a snapshot that fits with
            // an exact index. Neither failed candidate is installed or retained.
            for capacity in [amortized, len] {
                let mut candidate = Vec::new();
                if candidate.try_reserve_exact(capacity).is_err() {
                    if capacity != len {
                        continue;
                    }
                    return Err(ViewCursorError::AllocationUnavailable);
                }
                if view_retained_charge(snapshot_bytes, candidate.capacity())?
                    <= self.max_total_bytes
                {
                    grown = Some(candidate);
                    break;
                }
                if capacity == len {
                    return Err(ViewCursorError::CapacityExceeded);
                }
            }
        } else if view_retained_charge(snapshot_bytes, state.snapshots.capacity())?
            > self.max_total_bytes
        {
            return Err(ViewCursorError::CapacityExceeded);
        }
        // A failed entropy or cipher operation leaves both the old key and the
        // append-only index unchanged. Only a successful admission installs a key.
        let mut new_key = None;
        if state.key.is_none() {
            let mut key = zeroize::Zeroizing::new([0u8; 64]);
            self.fill_entropy(&mut key[..])?;
            new_key = Some(key);
        }
        let key = state
            .key
            .as_ref()
            .or(new_key.as_ref())
            .ok_or(ViewCursorError::OwnerUnavailable)?;
        let token = issue_view_cursor(key, id, offset)?;
        let payload = match shared_payload {
            Some(payload) => payload,
            None => Arc::new(ViewCollectionPayload {
                node,
                items: self.spill.store(items)?,
                digest,
            }),
        };
        let snapshot = Arc::new(ViewCollectionSnapshot {
            binding,
            payload,
            id,
        });
        if let Some(mut candidate) = grown {
            candidate.extend(state.snapshots.iter().cloned());
            candidate.push(snapshot);
            state.snapshots = candidate;
        } else {
            state.snapshots.push(snapshot);
        }
        state.snapshot_bytes = snapshot_bytes;
        state.payload_bytes = total_payload_bytes;
        state.payload_count = payload_count;
        if let Some(key) = new_key {
            state.key = Some(key);
        }
        // On reuse, the unused fresh node/items remain function parameters and
        // are destroyed after this guard, including every early refusal path.
        drop(state);
        Ok(token)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_usage_for_test(&self) -> ViewRetainedUsage {
        let state = self.state.lock().unwrap();
        ViewRetainedUsage {
            records: state.snapshots.len(),
            unique_payloads: state.payload_count,
            record_bytes: state.snapshot_bytes - state.payload_bytes,
            payload_bytes: state.payload_bytes,
            index_capacity: state.snapshots.capacity(),
            retained_bytes: view_retained_charge(state.snapshot_bytes, state.snapshots.capacity())
                .unwrap(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_binding_ids_for_test(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .snapshots
            .iter()
            .map(|snapshot| snapshot.binding.snapshot_id.clone())
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn retained_payload_identities_for_test(&self) -> Vec<usize> {
        self.state
            .lock()
            .unwrap()
            .snapshots
            .iter()
            .map(|snapshot| Arc::as_ptr(&snapshot.payload) as usize)
            .collect()
    }

    pub fn insert_next(
        &self,
        current: &StoredViewCursor,
        offset: usize,
        current_token: &str,
    ) -> Result<String, ViewCursorError> {
        let state = self
            .state
            .lock()
            .map_err(|_| ViewCursorError::OwnerUnavailable)?;
        let snapshot = view_snapshot_by_id(&state, current.snapshot.id)?;
        if !Arc::ptr_eq(snapshot, &current.snapshot) || offset >= snapshot.len() {
            return Err(ViewCursorError::Invalid);
        }
        let key = state.key.as_ref().ok_or(ViewCursorError::Invalid)?;
        if issue_view_cursor(key, snapshot.id, current.offset)? != current_token {
            return Err(ViewCursorError::Invalid);
        }
        issue_view_cursor(key, snapshot.id, offset)
    }

    pub fn read(
        &self,
        token: &str,
        expected: &ViewCursorQuestion,
    ) -> Result<StoredViewCursor, ViewCursorError> {
        use aes_siv::KeyInit;
        let ciphertext = decode_view_cursor(token)?;
        let state = self
            .state
            .lock()
            .map_err(|_| ViewCursorError::OwnerUnavailable)?;
        let key = state.key.as_ref().ok_or(ViewCursorError::Invalid)?;
        let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(&key[..])
            .map_err(|_| ViewCursorError::TokenUnavailable)?;
        let plaintext = zeroize::Zeroizing::new(
            cipher
                .decrypt([VIEW_CURSOR_AAD], &ciphertext)
                .map_err(|_| ViewCursorError::Invalid)?,
        );
        if plaintext.len() != VIEW_CURSOR_PAYLOAD_BYTES || plaintext[0] != 1 {
            return Err(ViewCursorError::Invalid);
        }
        let id = u64::from_be_bytes(
            plaintext[1..9]
                .try_into()
                .map_err(|_| ViewCursorError::Invalid)?,
        );
        let offset = usize::try_from(u64::from_be_bytes(
            plaintext[9..17]
                .try_into()
                .map_err(|_| ViewCursorError::Invalid)?,
        ))
        .map_err(|_| ViewCursorError::Invalid)?;
        // Authenticate and parse before consulting the private index.
        let snapshot = view_snapshot_by_id(&state, id)?;
        let binding = &snapshot.binding;
        if offset >= snapshot.len()
            || binding.canonical_at != expected.canonical_at
            || binding.projection != expected.projection
            || binding.source_set_identity != expected.source_set_identity
            || binding.page_limit != expected.page_limit
        {
            return Err(ViewCursorError::Invalid);
        }
        Ok(StoredViewCursor {
            snapshot: Arc::clone(snapshot),
            offset,
        })
    }

    pub fn token_probe() -> String {
        encode_view_cursor(&[0u8; VIEW_CURSOR_CIPHERTEXT_BYTES])
    }

    fn fill_entropy(&self, bytes: &mut [u8]) -> Result<(), ViewCursorError> {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(fill) = self.entropy {
            return fill(bytes);
        }
        getrandom::fill(bytes).map_err(|_| ViewCursorError::EntropyUnavailable)
    }
}

fn view_snapshot_by_id(
    state: &ViewCursorState,
    id: u64,
) -> Result<&Arc<ViewCollectionSnapshot>, ViewCursorError> {
    let index = usize::try_from(id)
        .ok()
        .and_then(|id| id.checked_sub(1))
        .ok_or(ViewCursorError::Invalid)?;
    state.snapshots.get(index).ok_or(ViewCursorError::Invalid)
}

fn view_add(left: usize, right: usize) -> Result<usize, ViewCursorError> {
    left.checked_add(right)
        .ok_or(ViewCursorError::OffsetNotRepresentable)
}

fn view_mul(left: usize, right: usize) -> Result<usize, ViewCursorError> {
    left.checked_mul(right)
        .ok_or(ViewCursorError::OffsetNotRepresentable)
}

fn view_retained_charge(
    snapshot_bytes: usize,
    index_capacity: usize,
) -> Result<usize, ViewCursorError> {
    let owner = view_add(
        std::mem::size_of::<ViewCursorStore>(),
        2 * std::mem::size_of::<usize>(),
    )?;
    view_add(
        view_add(owner, snapshot_bytes)?,
        view_mul(
            index_capacity,
            std::mem::size_of::<Arc<ViewCollectionSnapshot>>(),
        )?,
    )
}

fn issue_view_cursor(key: &[u8; 64], id: u64, offset: usize) -> Result<String, ViewCursorError> {
    use aes_siv::KeyInit;
    let mut payload = zeroize::Zeroizing::new([0u8; VIEW_CURSOR_PAYLOAD_BYTES]);
    payload[0] = 1;
    payload[1..9].copy_from_slice(&id.to_be_bytes());
    payload[9..17].copy_from_slice(
        &u64::try_from(offset)
            .map_err(|_| ViewCursorError::OffsetNotRepresentable)?
            .to_be_bytes(),
    );
    let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(key)
        .map_err(|_| ViewCursorError::TokenUnavailable)?;
    let ciphertext = cipher
        .encrypt([VIEW_CURSOR_AAD], &payload[..])
        .map_err(|_| ViewCursorError::TokenUnavailable)?;
    if ciphertext.len() != VIEW_CURSOR_CIPHERTEXT_BYTES {
        return Err(ViewCursorError::TokenUnavailable);
    }
    Ok(encode_view_cursor(&ciphertext))
}

fn encode_view_cursor(ciphertext: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(VIEW_CURSOR_TOKEN_BYTES);
    token.push_str("vc2.");
    for byte in ciphertext {
        token.push(char::from(HEX[usize::from(byte >> 4)]));
        token.push(char::from(HEX[usize::from(byte & 15)]));
    }
    token
}

fn decode_view_cursor(token: &str) -> Result<[u8; VIEW_CURSOR_CIPHERTEXT_BYTES], ViewCursorError> {
    if token.len() != VIEW_CURSOR_TOKEN_BYTES || !token.starts_with("vc2.") {
        return Err(ViewCursorError::Invalid);
    }
    fn nibble(byte: u8) -> Result<u8, ViewCursorError> {
        match byte {
            b'0'..=b'9' => Ok(byte - b'0'),
            b'a'..=b'f' => Ok(byte - b'a' + 10),
            _ => Err(ViewCursorError::Invalid),
        }
    }
    let mut ciphertext = [0u8; VIEW_CURSOR_CIPHERTEXT_BYTES];
    for (pair, out) in token.as_bytes()[4..]
        .as_chunks::<2>()
        .0
        .iter()
        .zip(&mut ciphertext)
    {
        *out = (nibble(pair[0])? << 4) | nibble(pair[1])?;
    }
    Ok(ciphertext)
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

#[cfg(test)]
fn bounded_json_size(value: &impl serde::Serialize, limit: usize) -> Option<usize> {
    bounded_json_size_attempt(value, limit).ok()
}

fn bounded_json_size_attempt(value: &impl serde::Serialize, limit: usize) -> Result<usize, usize> {
    let mut writer = BoundedSizeWriter { bytes: 0, limit };
    serde_json::to_writer(&mut writer, value).map_err(|_| writer.bytes)?;
    Ok(writer.bytes)
}

fn compact_view_objects(value: Value) -> Value {
    match value {
        Value::Object(values) => {
            let mut compact = serde_json::Map::with_capacity(values.len());
            for (key, value) in values {
                compact.insert(key, compact_view_objects(value));
            }
            Value::Object(compact)
        }
        Value::Array(values) => {
            Value::Array(values.into_iter().map(compact_view_objects).collect())
        }
        value => value,
    }
}

fn compact_view_object_charge(len: usize) -> Result<usize, ViewCursorError> {
    if len == 0 {
        return Ok(0);
    }
    // Cargo.lock pins serde_json 1.0.150 (preserve_order), indexmap 2.14.0
    // Core::with_capacity (one Vec<Bucket> plus HashTable<usize>), and
    // hashbrown 0.17.1 capacity_to_buckets/TableLayout::calculate_layout_for.
    // Newly compacted maps request exactly len entries. Charge their bucket
    // vector and the hash index, padding/control group at the maximum width
    // 16 used by these locked implementations. This is a constructor-based
    // retained-allocation bound, not allocator overhead or process RSS.
    let word = std::mem::size_of::<usize>();
    let buckets = if len < 15 {
        let min = match word {
            0..=1 => 14,
            2..=3 => 7,
            _ => 3,
        };
        match len.max(min) {
            0..=3 => 4,
            4..=7 => 8,
            _ => 16,
        }
    } else {
        (view_mul(len, 8)? / 7)
            .checked_next_power_of_two()
            .ok_or(ViewCursorError::OffsetNotRepresentable)?
    };
    let entry = view_add(
        view_add(word, std::mem::size_of::<String>())?,
        std::mem::size_of::<Value>(),
    )?;
    let entries = view_mul(len, entry)?;
    let ctrl_offset = view_add(view_mul(buckets, word)?, 15)? & !15;
    view_add(entries, view_add(ctrl_offset, view_add(buckets, 16)?)?)
}

fn snapshot_record_charge(binding: &ViewCursorBinding) -> Result<usize, ViewCursorError> {
    let json_bytes = bounded_json_size_attempt(binding, usize::MAX)
        .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
    let mut heap = std::mem::size_of::<ViewCollectionSnapshot>();
    for text in [
        &binding.canonical_at,
        &binding.projection,
        &binding.source_set_identity,
        &binding.snapshot_id,
    ] {
        heap = view_add(heap, text.capacity())?;
    }
    view_add(json_bytes.max(heap), 2 * std::mem::size_of::<usize>())
}

fn snapshot_payload_charge(node: &Value, items: &Vec<Value>) -> Result<usize, ViewCursorError> {
    let json_bytes = bounded_json_size_attempt(&(node, items), usize::MAX)
        .map_err(|_| ViewCursorError::OffsetNotRepresentable)?;
    let mut heap = std::mem::size_of::<ViewCollectionPayload>();
    heap = view_add(heap, value_extra_heap_bytes(node)?)?;
    heap = view_add(
        heap,
        view_mul(items.capacity(), std::mem::size_of::<Value>())?,
    )?;
    for value in items {
        heap = view_add(heap, value_extra_heap_bytes(value)?)?;
    }
    view_add(json_bytes.max(heap), 2 * std::mem::size_of::<usize>())
}

#[cfg(test)]
fn snapshot_charge(
    binding: &ViewCursorBinding,
    node: &Value,
    items: &Vec<Value>,
) -> Result<usize, ViewCursorError> {
    view_add(
        snapshot_record_charge(binding)?,
        snapshot_payload_charge(node, items)?,
    )
}

fn value_extra_heap_bytes(value: &Value) -> Result<usize, ViewCursorError> {
    match value {
        Value::String(text) => Ok(text.capacity()),
        Value::Array(values) => {
            let mut bytes = view_mul(values.capacity(), std::mem::size_of::<Value>())?;
            for value in values {
                bytes = view_add(bytes, value_extra_heap_bytes(value)?)?;
            }
            Ok(bytes)
        }
        Value::Object(values) => {
            let mut bytes = compact_view_object_charge(values.len())?;
            for (key, value) in values {
                bytes = view_add(bytes, key.capacity())?;
                bytes = view_add(bytes, value_extra_heap_bytes(value)?)?;
            }
            Ok(bytes)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => Ok(0),
    }
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

    #[derive(Default)]
    struct ViewObservationProbe {
        samples: Mutex<Vec<(u64, bool, bool)>>,
        admission_only_refusals: AtomicU64,
    }

    impl ViewCapacityObserver for ViewObservationProbe {
        fn record_view(&self, bytes: u64, exact: bool, admitted: bool) {
            self.samples.lock().unwrap().push((bytes, exact, admitted));
        }

        fn record_view_admission_refusal(&self) {
            self.admission_only_refusals.fetch_add(1, Ordering::Relaxed);
        }
    }

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
            source_set_identity: "main:sha256-source-id".to_string(),
            snapshot_id: revision.to_string(),
            page_limit: 1,
        }
    }

    fn search_binding(query: &str, _snapshot_label: &str) -> SearchCursorBinding {
        SearchCursorBinding {
            workspace_identity: "workspace-a".to_owned(),
            query: query.to_owned(),
            scope: None,
            mode: "literal".to_owned(),
            kind: None,
            source_sets: vec!["main".to_owned()],
            result_fingerprint: None,
            page_limit: 20,
        }
    }

    #[test]
    fn live_search_cursor_binds_the_question_and_reissues_a_successor() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "rev-1");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        assert!(first.starts_with("sc2."));
        assert_eq!(
            SearchCursorStore::default().read(&first, &binding).err(),
            Some(SearchCursorError::Invalid)
        );
        let mut different = binding.clone();
        different.source_sets.push("extension".to_owned());
        assert_eq!(
            store.read(&first, &different).err(),
            Some(SearchCursorError::Invalid)
        );
        let mut other_workspace = binding.clone();
        other_workspace.workspace_identity = "workspace-b".to_owned();
        assert_eq!(
            store.read(&first, &other_workspace).err(),
            Some(SearchCursorError::Invalid)
        );
        let stale = binding.clone();
        assert_eq!(store.read(&first, &stale).unwrap().offset, 20);
        let page = store.read(&first, &binding).unwrap();
        assert_eq!(page.offset, 20);
        let second = store.insert_next(&page, 40).unwrap();
        assert_eq!(store.insert_next(&page, 40).unwrap(), second);
        assert_eq!(store.read(&second, &binding).unwrap().offset, 40);
        assert_eq!(
            store
                .read("sc1.00000000000000000000000000000000", &binding)
                .err(),
            Some(SearchCursorError::Invalid)
        );
    }

    #[test]
    fn names_cursor_binds_kind_and_complete_ranked_answer() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding("Node", "unused");
        binding.mode = "names".to_owned();
        binding.kind = Some("Catalog".to_owned());
        binding.result_fingerprint = Some("names-sha256-v1:one".to_owned());
        let token = store.insert_first(binding.clone(), 20).unwrap();

        let mut other_kind = binding.clone();
        other_kind.kind = Some("Document".to_owned());
        assert_eq!(
            store.read(&token, &other_kind).err(),
            Some(SearchCursorError::Invalid)
        );
        let mut changed_answer = binding.clone();
        changed_answer.result_fingerprint = Some("names-sha256-v1:two".to_owned());
        assert_eq!(
            store.read(&token, &changed_answer).err(),
            Some(SearchCursorError::Stale)
        );
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);

        // Before the answer exists only the question can be verified: a
        // foreign question, owner or token is refused, a changed answer is not
        // yet known and stays for `read` to report as stale.
        let mut unanswered = changed_answer.clone();
        unanswered.result_fingerprint = None;
        assert_eq!(store.verify_question(&token, &unanswered), Ok(()));
        assert_eq!(store.verify_question(&token, &changed_answer), Ok(()));
        // The offset of such a token is known before the answer: a search
        // keeps only the ranks up to it, and `read` still finds it stale.
        assert_eq!(store.question_position(&token, &unanswered), Ok((20, None)));
        assert_eq!(
            store.question_position(&token, &changed_answer),
            Ok((20, None))
        );
        assert_eq!(
            store.question_position(&token, &other_kind),
            Err(SearchCursorError::Invalid)
        );

        // A keyed continuation carries the rank it resumes after through
        // the first token and its successor; the answer still binds it.
        let after = SearchAfter {
            score: 17,
            at: "main:Catalog.Валюты".to_owned(),
        };
        let keyed = store
            .insert_first_after(binding.clone(), 20, Some(&after))
            .unwrap();
        assert!(keyed.starts_with("sc3."));
        assert_eq!(
            keyed.len(),
            SearchCursorStore::keyed_token_probe(after.at.len()).len()
        );
        assert_eq!(
            store.question_position(&keyed, &unanswered),
            Ok((20, Some(after.clone())))
        );
        assert_eq!(
            store.read(&keyed, &changed_answer).err(),
            Some(SearchCursorError::Stale)
        );
        let page = store.read(&keyed, &binding).unwrap();
        assert_eq!(page.after.as_ref(), Some(&after));
        let next_after = SearchAfter {
            score: 30,
            at: "main:Document.Заказ".to_owned(),
        };
        let next = store
            .insert_next_after(&page, 40, Some(&next_after))
            .unwrap();
        let next = store.read(&next, &binding).unwrap();
        assert_eq!((next.offset, next.after), (40, Some(next_after)));
        // A keyed payload under the fixed prefix is not a token.
        assert_eq!(
            store
                .read(&keyed.replacen("sc3.", "sc2.", 1), &binding)
                .err(),
            Some(SearchCursorError::Invalid)
        );
        // Neither an odd, a too short nor a fixed-length keyed token decodes,
        // and a fixed token under the keyed prefix does not authenticate.
        let fixed = store.insert_first(binding.clone(), 20).unwrap();
        for forged in [
            format!("{keyed}0"),
            format!("sc3.{}", &keyed[4..keyed.len() - 18]),
            fixed.replacen("sc2.", "sc3.", 1),
            format!("{}00", fixed.replacen("sc2.", "sc3.", 1)),
        ] {
            assert_eq!(
                store.read(&forged, &binding).err(),
                Some(SearchCursorError::Invalid),
                "{forged}"
            );
        }
        assert_eq!(
            store.verify_question(&token, &other_kind),
            Err(SearchCursorError::Invalid)
        );
        assert_eq!(
            SearchCursorStore::default().verify_question(&token, &binding),
            Err(SearchCursorError::Invalid)
        );
        assert_eq!(
            store.verify_question("не-наш-курсор", &binding),
            Err(SearchCursorError::Invalid)
        );
    }

    #[test]
    fn live_search_keeps_all_300_independent_chains_and_their_replay() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "unused");
        let tokens: Vec<_> = (0..300)
            .map(|_| store.insert_first(binding.clone(), 20).unwrap())
            .collect();
        assert_eq!(
            tokens
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            300
        );
        for token in &tokens {
            let page = store
                .read(token, &binding)
                .expect("issued chain is retained");
            assert_eq!(page.offset, 20);
            let successor = store.insert_next(&page, 40).unwrap();
            assert_eq!(store.read(&successor, &binding).unwrap().offset, 40);
            assert_eq!(store.insert_next(&page, 40).unwrap(), successor);
            assert_eq!(store.read(token, &binding).unwrap().offset, 20);
        }
    }

    #[test]
    fn live_search_keeps_the_exact_question_past_eight_mib() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding(&"needle".repeat(1_600_000), "unused");
        binding.scope = Some("main:Configuration".to_owned());
        binding.source_sets.push("extension".to_owned());
        let token = store
            .insert_first(binding.clone(), 20)
            .expect("question metadata has no aggregate byte quota");
        let page = store.read(&token, &binding).unwrap();
        assert_eq!(page.offset, 20);
        let next = store.insert_next(&page, 40).unwrap();
        assert_eq!(store.read(&next, &binding).unwrap().offset, 40);
        let mut changed = binding.clone();
        changed.query.push('x');
        assert_eq!(
            store.read(&token, &changed).err(),
            Some(SearchCursorError::Invalid)
        );
        let mut reordered = binding.clone();
        reordered.source_sets.reverse();
        assert_eq!(
            store.read(&token, &reordered).err(),
            Some(SearchCursorError::Invalid)
        );
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_question_mismatch_precedes_changed_result_fingerprint() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding("needle", "unused");
        binding.result_fingerprint = Some(String::new());
        let token = store.insert_first(binding.clone(), 20).unwrap();
        let mut no_fingerprint = binding.clone();
        no_fingerprint.result_fingerprint = None;
        assert_eq!(
            store.read(&token, &no_fingerprint).err(),
            Some(SearchCursorError::Stale)
        );
        no_fingerprint.query.push('x');
        assert_eq!(
            store.read(&token, &no_fingerprint).err(),
            Some(SearchCursorError::Invalid)
        );
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_cursor_lifetime_is_the_actual_owner_without_retained_entries() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "unused");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        let page = store.read(&first, &binding).unwrap();
        let successor = store.insert_next(&page, 40).unwrap();
        assert_eq!(store.insert_next(&page, 40).unwrap(), successor);
        assert_eq!(store.read(&first, &binding).unwrap().offset, 20);
        assert_eq!(store.read(&successor, &binding).unwrap().offset, 40);
        let foreign = SearchCursorStore::default();
        assert_eq!(
            foreign.insert_next(&page, 60).err(),
            Some(SearchCursorError::Invalid)
        );
        assert!(foreign.owner.lock().unwrap().is_none());
        let foreign_token = foreign.insert_first(binding.clone(), 20).unwrap();
        assert_eq!(
            foreign.insert_next(&page, 60).err(),
            Some(SearchCursorError::Invalid)
        );
        assert_eq!(
            store.read(&foreign_token, &binding).err(),
            Some(SearchCursorError::Invalid)
        );
        drop(page);
        assert_eq!(
            Arc::strong_count(store.owner.lock().unwrap().as_ref().unwrap()),
            1
        );
        drop(store);
        assert_eq!(
            SearchCursorStore::default().read(&first, &binding).err(),
            Some(SearchCursorError::Invalid)
        );
        assert_eq!(
            foreign.read(&first, &binding).err(),
            Some(SearchCursorError::Invalid)
        );
    }

    #[test]
    fn search_authentication_rejects_every_changed_token_byte_and_noncanonical_encoding() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "unused");
        let token = store.insert_first(binding.clone(), 20).unwrap();
        assert_eq!(token.len(), SearchCursorStore::token_probe().len());
        assert_eq!(token.len(), SEARCH_CURSOR_TOKEN_BYTES);
        for index in 0..token.len() {
            let mut altered = token.clone().into_bytes();
            altered[index] = if altered[index] == b'0' { b'1' } else { b'0' };
            assert_eq!(
                store
                    .read(std::str::from_utf8(&altered).unwrap(), &binding)
                    .err(),
                Some(SearchCursorError::Invalid),
                "changed token byte {index}"
            );
        }
        for invalid in [
            token[..token.len() - 1].to_owned(),
            format!("{token}0"),
            token.to_uppercase(),
            SearchCursorStore::token_probe(),
            "sc1.00000000000000000000000000000000".into(),
        ] {
            assert_eq!(
                store.read(&invalid, &binding).err(),
                Some(SearchCursorError::Invalid)
            );
        }
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_authenticated_schema_is_closed_and_some_zero_is_not_a_none_sentinel() {
        let store = SearchCursorStore::default();
        let binding = search_binding("needle", "unused");
        let token = store.insert_first(binding.clone(), 20).unwrap();
        let page = store.read(&token, &binding).unwrap();
        for (position, value) in [(0, 2), (33, 2), (34, 1)] {
            let mut payload = page.payload.clone();
            payload[position] = value;
            let invalid =
                issue_search_cursor(&page.owner, &payload, SearchCursorTail::None).unwrap();
            assert_eq!(
                store.read(&invalid, &binding).err(),
                Some(SearchCursorError::Invalid)
            );
        }
        let mut some_zero = page.payload.clone();
        some_zero[33] = 1;
        let signed = issue_search_cursor(&page.owner, &some_zero, SearchCursorTail::None).unwrap();
        assert_eq!(
            store.read(&signed, &binding).err(),
            Some(SearchCursorError::Stale)
        );
        assert_eq!(
            SearchCursorStore::next_offset(usize::MAX, 1),
            Err(SearchCursorError::OffsetNotRepresentable)
        );
        assert_eq!(
            SearchCursorStore::next_offset(usize::MAX - 1, 1),
            Ok(usize::MAX)
        );
    }

    #[test]
    fn search_next_answer_rebinds_only_the_answer_and_replays_its_token() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding("needle", "unused");
        binding.result_fingerprint = Some("prefix-50".into());
        let resume = SearchResume {
            file: b"main\0Catalogs/Items/Ext/ObjectModule.bsl".to_vec(),
            hits: 3,
            uncovered: 2,
        };
        let token = store
            .insert_first_resumable(binding.clone(), 50, &resume)
            .unwrap();
        assert!(token.starts_with("sc4."));
        assert_eq!(
            store.resume_position(&token, &binding),
            Ok((50, resume.clone())),
            "the question half carries the resume place"
        );
        let stored = store.read(&token, &binding).unwrap();
        assert_eq!((stored.offset, &stored.resume), (50, &resume));
        let mut next = binding.clone();
        next.result_fingerprint = Some("prefix-100".into());
        let further = SearchResume {
            hits: 5,
            ..resume.clone()
        };
        let successor = store
            .insert_next_answer(&stored, &next, 100, &further)
            .unwrap();
        assert_eq!(
            store
                .insert_next_answer(&stored, &next, 100, &further)
                .unwrap(),
            successor,
            "a replayed page issues the same continuation"
        );
        let read = store.read(&successor, &next).unwrap();
        assert_eq!((read.offset, &read.resume), (100, &further));
        assert_eq!(
            store.read(&successor, &binding).err(),
            Some(SearchCursorError::Stale)
        );
        let mut other_question = next.clone();
        other_question.query.push('x');
        assert_eq!(
            store
                .insert_next_answer(&stored, &other_question, 100, &further)
                .err(),
            Some(SearchCursorError::Invalid)
        );
        let foreign = SearchCursorStore::default();
        foreign.insert_first(binding.clone(), 0).unwrap();
        assert_eq!(
            foreign
                .insert_next_answer(&stored, &next, 100, &further)
                .err(),
            Some(SearchCursorError::Invalid)
        );
        // Another tail under the same ciphertext is not a token.
        let forged = format!("sc3.{}", &successor[4..]);
        assert_eq!(
            store.read(&forged, &next).err(),
            Some(SearchCursorError::Invalid)
        );
    }

    #[test]
    fn search_every_question_field_precedes_answer_fingerprint_validation() {
        let store = SearchCursorStore::default();
        let mut binding = search_binding("needle", "unused");
        binding.source_sets = vec!["main".into(), "extension".into()];
        binding.result_fingerprint = Some("answer".into());
        let token = store.insert_first(binding.clone(), 20).unwrap();
        let changes: [fn(&mut SearchCursorBinding); 7] = [
            |v| v.workspace_identity.push('x'),
            |v| v.query.push('x'),
            |v| v.scope = Some(String::new()),
            |v| v.mode.push('x'),
            |v| v.kind = Some(String::new()),
            |v| v.source_sets.reverse(),
            |v| v.page_limit += 1,
        ];
        for change in changes {
            let mut other = binding.clone();
            change(&mut other);
            other.result_fingerprint = None;
            assert_eq!(
                store.read(&token, &other).err(),
                Some(SearchCursorError::Invalid)
            );
        }
        for fingerprint in [
            None,
            Some(String::new()),
            Some("changed coverage/index/warnings".into()),
        ] {
            let mut other = binding.clone();
            other.result_fingerprint = fingerprint;
            assert_eq!(
                store.read(&token, &other).err(),
                Some(SearchCursorError::Stale)
            );
        }
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_failed_key_entropy_publishes_nothing_and_retry_uses_a_real_key() {
        fn partial_failure(bytes: &mut [u8]) -> Result<(), SearchCursorError> {
            bytes[0] = 42;
            Err(SearchCursorError::EntropyUnavailable)
        }
        let mut store = SearchCursorStore {
            entropy: Some(partial_failure),
            ..Default::default()
        };
        let binding = search_binding("needle", "unused");
        let error = store.insert_first(binding.clone(), 20).unwrap_err();
        assert_eq!(error, SearchCursorError::EntropyUnavailable);
        assert_eq!(
            error.into_result(None).diagnostics[0]["detailCode"],
            "provider_absent"
        );
        assert!(store.owner.lock().unwrap().is_none());
        assert_eq!(
            store
                .read(&SearchCursorStore::token_probe(), &binding)
                .err(),
            Some(SearchCursorError::Invalid)
        );
        store.entropy = None;
        let token = store.insert_first(binding.clone(), 20).unwrap();
        assert_eq!(store.read(&token, &binding).unwrap().offset, 20);
    }

    #[test]
    fn search_failed_chain_entropy_preserves_the_existing_owner_and_tokens() {
        fn failure(_: &mut [u8]) -> Result<(), SearchCursorError> {
            Err(SearchCursorError::EntropyUnavailable)
        }
        let mut store = SearchCursorStore::default();
        let binding = search_binding("needle", "unused");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        let original = store.owner.lock().unwrap().as_ref().unwrap().clone();
        store.entropy = Some(failure);
        let error = store.insert_first(binding.clone(), 20).unwrap_err();
        assert_eq!(error, SearchCursorError::EntropyUnavailable);
        assert_eq!(
            error.into_result(None).diagnostics[0]["detailCode"],
            "provider_absent"
        );
        assert!(Arc::ptr_eq(
            &original,
            store.owner.lock().unwrap().as_ref().unwrap()
        ));
        let page = store.read(&first, &binding).unwrap();
        let next = store.insert_next(&page, 40).unwrap();
        assert_eq!(store.read(&next, &binding).unwrap().offset, 40);
        assert_eq!(format!("{original:?}"), "SearchCursorOwner { .. }");
    }

    #[test]
    fn search_poison_is_a_typed_owner_failure_without_discarding_its_identity() {
        let store = Arc::new(SearchCursorStore::default());
        let binding = search_binding("needle", "unused");
        let first = store.insert_first(binding.clone(), 20).unwrap();
        let page = store.read(&first, &binding).unwrap();
        let poisoned = Arc::clone(&store);
        assert!(std::thread::spawn(move || {
            let _owner = poisoned.owner.lock().unwrap();
            panic!("injected cursor owner poison");
        })
        .join()
        .is_err());
        let error = store
            .read(&first, &binding)
            .err()
            .expect("actual poisoned owner refuses");
        assert_eq!(error, SearchCursorError::OwnerUnavailable);
        assert_eq!(
            error.into_result(None).diagnostics[0]["detailCode"],
            "cache_poisoned"
        );
        assert_eq!(
            store.insert_first(binding, 40).err(),
            Some(SearchCursorError::OwnerUnavailable)
        );
        assert_eq!(
            store.insert_next(&page, 40).err(),
            Some(SearchCursorError::OwnerUnavailable)
        );
        assert_eq!(
            SearchCursorError::OwnerUnavailable.code(),
            RefusalCode::ProviderUnavailable
        );
    }

    #[test]
    fn search_concurrent_chains_and_successor_replay_share_one_immutable_owner() {
        let store = Arc::new(SearchCursorStore::default());
        let binding = search_binding("needle", "unused");
        let workers: Vec<_> = (0..16)
            .map(|_| {
                let store = Arc::clone(&store);
                let binding = binding.clone();
                std::thread::spawn(move || {
                    let first = store.insert_first(binding.clone(), 20).unwrap();
                    let page = store.read(&first, &binding).unwrap();
                    let next = store.insert_next(&page, 40).unwrap();
                    assert_eq!(store.insert_next(&page, 40).unwrap(), next);
                    assert_eq!(store.read(&first, &binding).unwrap().offset, 20);
                    (first, next)
                })
            })
            .collect();
        let chains: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(
            chains
                .iter()
                .map(|chain| &chain.0)
                .collect::<HashSet<_>>()
                .len(),
            chains.len()
        );
        for (first, next) in chains {
            assert_eq!(store.read(&first, &binding).unwrap().offset, 20);
            assert_eq!(store.read(&next, &binding).unwrap().offset, 40);
        }
        assert_eq!(
            Arc::strong_count(store.owner.lock().unwrap().as_ref().unwrap()),
            1
        );
    }

    #[test]
    fn search_cursor_failures_preserve_exact_public_reason_and_next_actor() {
        let cases = [
            (
                SearchCursorError::Invalid,
                "invalid_cursor",
                None,
                "fixCall",
            ),
            (SearchCursorError::Stale, "stale_cursor", None, "fixCall"),
            (
                SearchCursorError::OwnerUnavailable,
                "provider_unavailable",
                Some("cache_poisoned"),
                "needsHuman",
            ),
            (
                SearchCursorError::EntropyUnavailable,
                "provider_unavailable",
                Some("provider_absent"),
                "needsHuman",
            ),
            (
                SearchCursorError::TokenUnavailable,
                "provider_failed",
                None,
                "deadEnd",
            ),
            (
                SearchCursorError::OffsetNotRepresentable,
                "provider_failed",
                None,
                "deadEnd",
            ),
        ];
        for (error, code, detail, outcome) in cases {
            let at = "main:Configuration";
            let result = error.into_result(Some(at.into()));
            assert!(!result.ok);
            assert_eq!(result.at.as_deref(), Some(at));
            assert!(result.data.is_none());
            let diagnostic = &result.diagnostics[0];
            assert_eq!(diagnostic["code"], code);
            assert_eq!(diagnostic.get("detailCode").and_then(Value::as_str), detail);
            assert_eq!(diagnostic["outcome"], outcome);
            assert_eq!(diagnostic["message"], error.message());
        }
    }

    #[test]
    fn search_cipher_passes_rfc5297_deterministic_known_answer() {
        use aes_siv::KeyInit;
        fn hex(text: &str) -> Vec<u8> {
            text.as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        }
        let key = hex("fffefdfcfbfaf9f8f7f6f5f4f3f2f1f0f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
        let aad = hex("101112131415161718191a1b1c1d1e1f2021222324252627");
        let plain = hex("112233445566778899aabbccddee");
        let expected = hex("85632d07c6e8f37f950acd320a2ecc9340c02b9690c4dc04daef7f6afe5c");
        let mut cipher = aes_siv::siv::Aes128Siv::new_from_slice(&key).unwrap();
        assert_eq!(cipher.encrypt([&aad], &plain).unwrap(), expected);
        assert_eq!(cipher.decrypt([&aad], &expected).unwrap(), plain);
    }

    #[test]
    fn search_cipher_passes_upstream_aes256_known_answer() {
        use aes_siv::KeyInit;
        fn hex(text: &str) -> Vec<u8> {
            text.as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        }
        // RustCrypto aes-siv v0.8.0, tests/siv.rs::aes256cmacsiv, first vector:
        // https://github.com/RustCrypto/AEADs/blob/aes-siv-v0.8.0/aes-siv/tests/siv.rs
        let key = hex("fffefdfcfbfaf9f8f7f6f5f4f3f2f1f06f6e6d6c6b6a69686766656463626160f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff000102030405060708090a0b0c0d0e0f");
        let aad = hex("101112131415161718191a1b1c1d1e1f2021222324252627");
        let plain = hex("112233445566778899aabbccddee");
        let expected = hex("f125274c598065cfc26b0e71575029088b035217e380cac8919ee800c126");
        let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(&key).unwrap();
        assert_eq!(cipher.encrypt([&aad], &plain).unwrap(), expected);
        assert_eq!(cipher.decrypt([&aad], &expected).unwrap(), plain);
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
        assert!(token.starts_with("vc2."));
        assert!(token[4..].parse::<usize>().is_err());
        assert_eq!(
            ViewCursorStore::default()
                .read(&token, &binding.question())
                .unwrap_err(),
            ViewCursorError::Invalid
        );

        let mut other = binding.clone();
        other.canonical_at = "main:Document.Счет.Module.Object.Body".to_string();
        assert_eq!(
            store.read(&token, &other.question()).unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.projection = "Method".to_string();
        assert_eq!(
            store.read(&token, &other.question()).unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.source_set_identity = "other:sha256-source-id".to_string();
        assert_eq!(
            store.read(&token, &other.question()).unwrap_err(),
            ViewCursorError::Invalid
        );
        let mut other = binding.clone();
        other.page_limit = 2;
        assert_eq!(
            store.read(&token, &other.question()).unwrap_err(),
            ViewCursorError::Invalid
        );
        let page = store.read(&token, &binding.question()).unwrap();
        assert_eq!(page.snapshot.items()[page.offset], json!({"line": 2}));
        let successor = store.insert_next(&page, 1, &token).unwrap();
        let replay = store.read(&token, &binding.question()).unwrap();
        assert_eq!(replay.offset, page.offset);
        assert_eq!(store.insert_next(&replay, 1, &token).unwrap(), successor);
    }

    #[test]
    fn new_snapshot_pressure_preserves_every_promised_chain() {
        let store = ViewCursorStore::new(4_096);
        let mut issued = Vec::new();
        for index in 0..100 {
            let binding = view_binding(
                &format!("main:Document.Owner{index}.Module.Object.Body"),
                "rev-1",
            );
            match store.insert_snapshot(
                binding.clone(),
                json!({}),
                vec![json!("X".repeat(800)), json!(2)],
                0,
            ) {
                Ok(token) => issued.push((binding, token)),
                Err(error) => {
                    assert_eq!(error, ViewCursorError::CapacityExceeded);
                    break;
                }
            }
        }
        assert!(!issued.is_empty() && issued.len() < 100);
        let state = store.state.lock().unwrap();
        let before = (
            state.snapshots.len(),
            state.snapshots.capacity(),
            state.snapshot_bytes,
        );
        assert!(
            view_retained_charge(state.snapshot_bytes, state.snapshots.capacity()).unwrap()
                <= 4_096
        );
        drop(state);
        for (binding, token) in &issued {
            let page = store.read(token, &binding.question()).unwrap();
            let successor = store.insert_next(&page, 1, token).unwrap();
            assert_eq!(
                store.read(&successor, &binding.question()).unwrap().offset,
                1
            );
            assert_eq!(store.insert_next(&page, 1, token).unwrap(), successor);
        }
        let state = store.state.lock().unwrap();
        assert_eq!(
            (
                state.snapshots.len(),
                state.snapshots.capacity(),
                state.snapshot_bytes
            ),
            before
        );
    }

    #[test]
    fn snapshot_admission_reserves_room_for_the_next_cursor() {
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let node = json!({"at": binding.canonical_at});
        let items = vec![json!(1), json!(2), json!(3)];
        let bytes = snapshot_charge(&binding, &node, &items).unwrap();
        let exact = view_retained_charge(bytes, 1).unwrap();
        let too_small = ViewCursorStore::new(exact - 1);
        assert_eq!(
            too_small.insert_snapshot(binding.clone(), node.clone(), items.clone(), 1),
            Err(ViewCursorError::CapacityExceeded)
        );
        let state = too_small.state.lock().unwrap();
        assert!(state.key.is_none() && state.snapshots.is_empty());
        assert_eq!(
            state.snapshots.capacity(),
            0,
            "refused growth must leave no retained index allocation"
        );
        drop(state);
        let enough = ViewCursorStore::new(exact);
        let first = enough
            .insert_snapshot(binding.clone(), node, items, 1)
            .unwrap();
        let page = enough.read(&first, &binding.question()).unwrap();
        let second = enough.insert_next(&page, 2, &first).unwrap();
        assert_eq!(enough.read(&second, &binding.question()).unwrap().offset, 2);
        let state = enough.state.lock().unwrap();
        assert_eq!(state.snapshots.len(), 1);
        assert!(
            view_retained_charge(state.snapshot_bytes, state.snapshots.capacity()).unwrap()
                <= exact
        );
    }

    #[test]
    fn view_observation_distinguishes_complete_size_from_quota_lower_bound() {
        let observer = Arc::new(ViewObservationProbe::default());
        let port: Arc<dyn ViewCapacityObserver> = observer.clone();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let capped = ViewCursorStore::new(512).with_capacity_observer(Arc::clone(&port));
        assert_eq!(
            capped.insert_snapshot(
                binding.clone(),
                json!({}),
                vec![json!("x".repeat(2_048))],
                0
            ),
            Err(ViewCursorError::CapacityExceeded)
        );
        let successful = ViewCursorStore::new(4_096).with_capacity_observer(Arc::clone(&port));
        assert!(successful
            .insert_snapshot(binding, json!({}), vec![json!(1)], 0)
            .is_ok());
        let samples = observer.samples.lock().unwrap();
        assert_eq!(samples.len(), 2);
        assert!(samples[0].0 > 512);
        assert_eq!((samples[0].1, samples[0].2), (false, false));
        assert_eq!((samples[1].1, samples[1].2), (true, true));
    }

    #[test]
    fn many_small_items_are_charged_for_retained_heap_not_only_json() {
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let items = vec![Value::Null; 10_000];
        let json_bytes = bounded_json_size(&(&binding, json!({}), &items), usize::MAX).unwrap();
        assert!(json_bytes < 100_000);
        let store = ViewCursorStore::new(100_000);
        assert_eq!(
            store.insert_snapshot(binding, json!({}), items, 1),
            Err(ViewCursorError::CapacityExceeded)
        );
    }

    #[test]
    fn source_change_preserves_the_snapshot_but_tampering_is_invalid() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let token = store
            .insert_snapshot(binding.clone(), json!({}), vec![json!({"line": 2})], 0)
            .unwrap();
        let mut changed = binding.clone();
        changed.snapshot_id = "rev-after-source-change".into();
        assert_eq!(
            store
                .read(&token, &changed.question())
                .unwrap()
                .snapshot
                .items(),
            vec![json!({"line": 2})]
        );
        assert_eq!(
            store
                .read(&ViewCursorStore::token_probe(), &binding.question())
                .unwrap_err(),
            ViewCursorError::Invalid
        );
    }

    #[test]
    fn hundreds_of_independent_chains_have_no_record_count_eviction() {
        let store = ViewCursorStore::default();
        let issued = (0..300)
            .map(|index| {
                let binding =
                    view_binding(&format!("main:CommonModule.Owner{index}.Method"), "rev-1");
                let token = store
                    .insert_snapshot(
                        binding.clone(),
                        json!({}),
                        vec![json!(index), json!(index + 1)],
                        0,
                    )
                    .unwrap();
                (binding, token, index)
            })
            .collect::<Vec<_>>();
        for (binding, token, index) in issued {
            let page = store.read(&token, &binding.question()).unwrap();
            assert_eq!(page.snapshot.items()[page.offset], json!(index));
            let next = store.insert_next(&page, 1, &token).unwrap();
            assert_eq!(
                store
                    .read(&next, &binding.question())
                    .unwrap()
                    .snapshot
                    .items()[1],
                json!(index + 1)
            );
        }
        assert_eq!(store.state.lock().unwrap().snapshots.len(), 300);
    }

    #[test]
    fn only_complete_equal_fresh_results_share_retained_state() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let first = store
            .insert_snapshot(
                binding.clone(),
                json!({"title": "one"}),
                vec![json!(1), json!(2)],
                0,
            )
            .unwrap();
        for _ in 0..130 {
            assert_eq!(
                store
                    .insert_snapshot(
                        binding.clone(),
                        json!({"title": "one"}),
                        vec![json!(1), json!(2)],
                        0
                    )
                    .unwrap(),
                first
            );
        }
        let first_usage = store.retained_usage_for_test();
        assert_eq!((first_usage.records, first_usage.unique_payloads), (1, 1));
        let original = store.read(&first, &binding.question()).unwrap();
        let mut revised = binding.clone();
        revised.snapshot_id = "rev-2".into();
        let revision_token = store
            .insert_snapshot(
                revised.clone(),
                json!({"title": "one"}),
                vec![json!(1), json!(2)],
                0,
            )
            .unwrap();
        assert_ne!(revision_token, first);
        let revision = store.read(&revision_token, &binding.question()).unwrap();
        assert_ne!(original.snapshot.id, revision.snapshot.id);
        assert_eq!(original.snapshot.binding.snapshot_id, "rev-1");
        assert_eq!(revision.snapshot.binding.snapshot_id, "rev-2");
        assert!(Arc::ptr_eq(
            &original.snapshot.payload,
            &revision.snapshot.payload
        ));
        let revised_usage = store.retained_usage_for_test();
        assert_eq!(
            (revised_usage.records, revised_usage.unique_payloads),
            (2, 1)
        );
        assert_eq!(revised_usage.payload_bytes, first_usage.payload_bytes);
        assert_eq!(revised_usage.record_bytes, 2 * first_usage.record_bytes);
        assert_eq!(
            store
                .insert_snapshot(
                    revised,
                    json!({"title": "one"}),
                    vec![json!(1), json!(2)],
                    0,
                )
                .unwrap(),
            revision_token,
            "a later full-binding retry must find its own record past an earlier shared payload"
        );
        assert_eq!(store.retained_usage_for_test(), revised_usage);
        for (node, items) in [
            (json!({"title": "two"}), vec![json!(1), json!(2)]),
            (json!({"title": "one"}), vec![json!(1), json!(3)]),
            (json!({"title": "one"}), vec![json!(2), json!(1)]),
        ] {
            let token = store
                .insert_snapshot(binding.clone(), node, items, 0)
                .unwrap();
            assert_ne!(token, first);
            let changed = store.read(&token, &binding.question()).unwrap();
            assert!(!Arc::ptr_eq(
                &original.snapshot.payload,
                &changed.snapshot.payload
            ));
        }
        let usage = store.retained_usage_for_test();
        assert_eq!((usage.records, usage.unique_payloads), (5, 4));
        assert_eq!(original.snapshot.items(), vec![json!(1), json!(2)]);
    }

    #[test]
    fn equal_payloads_from_different_complete_questions_are_retained_separately() {
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let store = ViewCursorStore::default();
        let first = store
            .insert_snapshot(binding.clone(), json!({}), vec![json!(1)], 0)
            .unwrap();
        let original = store.read(&first, &binding.question()).unwrap();
        for field in 0..4 {
            let mut changed = binding.clone();
            match field {
                0 => changed.canonical_at = "main:CommonModule.Other.Method".into(),
                1 => changed.projection = "other".into(),
                2 => changed.source_set_identity = "other-workspace-and-source-set".into(),
                3 => changed.page_limit += 1,
                _ => unreachable!(),
            }
            let token = store
                .insert_snapshot(changed.clone(), json!({}), vec![json!(1)], 0)
                .unwrap();
            let retained = store.read(&token, &changed.question()).unwrap();
            assert!(!Arc::ptr_eq(
                &original.snapshot.payload,
                &retained.snapshot.payload
            ));
            assert_eq!(
                store.read(&token, &binding.question()).unwrap_err(),
                ViewCursorError::Invalid
            );
        }
        let usage = store.retained_usage_for_test();
        assert_eq!((usage.records, usage.unique_payloads), (5, 5));
    }

    #[test]
    fn shared_payload_boundary_charges_each_binding_and_preserves_every_refusal() {
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let node = json!({});
        let items = vec![json!("x".repeat(16_384)), json!(2)];
        let record_bytes = snapshot_record_charge(&binding).unwrap();
        let payload_bytes = snapshot_payload_charge(&node, &items).unwrap();
        let exact = view_retained_charge(payload_bytes + 5 * record_bytes, 5).unwrap();
        assert!(exact < view_retained_charge(5 * (payload_bytes + record_bytes), 5).unwrap());
        let store = ViewCursorStore::new(exact);
        let first = store
            .insert_snapshot(binding.clone(), node.clone(), items.clone(), 0)
            .unwrap();
        let mut changed = binding.clone();
        changed.snapshot_id = "rev-2".into();
        let second = store
            .insert_snapshot(changed.clone(), node.clone(), items.clone(), 0)
            .unwrap();
        assert_ne!(first, second);
        let mut issued = vec![first, second];
        for revision in ["rev-3", "rev-4", "rev-5"] {
            changed.snapshot_id = revision.into();
            issued.push(
                store
                    .insert_snapshot(changed.clone(), node.clone(), items.clone(), 0)
                    .unwrap(),
            );
        }
        let before = store.retained_usage_for_test();
        assert_eq!((before.records, before.unique_payloads), (5, 1));
        assert_eq!(before.payload_bytes, payload_bytes);
        assert_eq!(before.record_bytes, 5 * record_bytes);
        assert_eq!(
            before.index_capacity, 5,
            "near-bound growth uses the exact fallback"
        );
        assert_eq!(before.retained_bytes, exact);
        let key = store.state.lock().unwrap().key.as_ref().unwrap().clone();
        changed.snapshot_id = "rev-6".into();
        assert_eq!(
            store.insert_snapshot(changed.clone(), node.clone(), items.clone(), 0),
            Err(ViewCursorError::CapacityExceeded),
            "sharing still reserves each fresh binding"
        );
        assert_eq!(store.retained_usage_for_test(), before);
        assert_eq!(
            store.admit_snapshot(changed, node, items, 0, usize::MAX, 0),
            Err(ViewCursorError::OffsetNotRepresentable)
        );
        assert_eq!(store.retained_usage_for_test(), before);
        assert_eq!(
            &store.state.lock().unwrap().key.as_ref().unwrap()[..],
            &key[..]
        );
        for token in &issued {
            let page = store.read(token, &binding.question()).unwrap();
            let next = store.insert_next(&page, 1, token).unwrap();
            assert_eq!(store.insert_next(&page, 1, token).unwrap(), next);
            assert_eq!(
                store
                    .read(&next, &binding.question())
                    .unwrap()
                    .snapshot
                    .items()[1],
                json!(2)
            );
        }
        assert_eq!(store.retained_usage_for_test(), before);
    }

    #[test]
    fn concurrent_fresh_bindings_charge_one_shared_payload_and_all_successful_records() {
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-0000");
        let node = json!({});
        let items = vec![json!("x".repeat(16_384)), json!(2)];
        let record_bytes = snapshot_record_charge(&binding).unwrap();
        let payload_bytes = snapshot_payload_charge(&node, &items).unwrap();
        let budget = view_retained_charge(payload_bytes + 4 * record_bytes, 4).unwrap();
        let store = Arc::new(ViewCursorStore::new(budget));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let mut binding = binding.clone();
                binding.snapshot_id = format!("rev-{index:04}").into_boxed_str().into_string();
                let node = node.clone();
                let items = items.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.insert_snapshot(binding, node, items, 0)
                })
            })
            .collect::<Vec<_>>();
        let tokens = handles
            .into_iter()
            .filter_map(|handle| match handle.join().unwrap() {
                Ok(token) => Some(token),
                Err(error) => {
                    assert_eq!(error, ViewCursorError::CapacityExceeded);
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens.iter().collect::<HashSet<_>>().len(), 4);
        let usage = store.retained_usage_for_test();
        assert_eq!((usage.records, usage.unique_payloads), (4, 1));
        assert_eq!(usage.record_bytes, 4 * record_bytes);
        assert_eq!(usage.payload_bytes, payload_bytes);
        assert_eq!(usage.retained_bytes, budget);
        let pages = tokens
            .iter()
            .map(|token| store.read(token, &binding.question()).unwrap())
            .collect::<Vec<_>>();
        for page in &pages[1..] {
            assert!(Arc::ptr_eq(
                &pages[0].snapshot.payload,
                &page.snapshot.payload
            ));
        }
        for (page, token) in pages.iter().zip(&tokens) {
            let next = store.insert_next(page, 1, token).unwrap();
            assert_eq!(store.insert_next(page, 1, token).unwrap(), next);
        }
        assert_eq!(store.retained_usage_for_test(), usage);
    }

    #[test]
    fn concurrent_new_admissions_reserve_one_atomic_cumulative_budget() {
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let node = json!({});
        let items = vec![json!(1), json!(2)];
        let bytes = snapshot_charge(&binding, &node, &items).unwrap();
        let budget = view_retained_charge(bytes * 2, 4).unwrap();
        let store = Arc::new(ViewCursorStore::new(budget));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles = (0..8)
            .map(|index| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                let binding = binding.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.insert_snapshot(
                        binding,
                        json!({}),
                        vec![json!(index * 2), json!(index * 2 + 1)],
                        0,
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 2);
        assert!(results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| *error == ViewCursorError::CapacityExceeded));
        let state = store.state.lock().unwrap();
        assert_eq!(state.snapshots.len(), 2);
        assert!(
            view_retained_charge(state.snapshot_bytes, state.snapshots.capacity()).unwrap()
                <= budget
        );
        drop(state);
        for token in results.into_iter().flatten() {
            let page = store.read(&token, &binding.question()).unwrap();
            assert!(store.insert_next(&page, 1, &token).is_ok());
        }
    }

    #[test]
    fn view_tokens_authenticate_every_byte_owner_offset_and_domain() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let token = store
            .insert_snapshot(binding.clone(), json!({}), vec![json!(1), json!(2)], 0)
            .unwrap();
        assert_eq!(token.len(), ViewCursorStore::token_probe().len());
        for index in 0..token.len() {
            let mut changed = token.as_bytes().to_vec();
            changed[index] = if changed[index] == b'0' { b'1' } else { b'0' };
            assert_eq!(
                store
                    .read(std::str::from_utf8(&changed).unwrap(), &binding.question())
                    .unwrap_err(),
                ViewCursorError::Invalid,
                "tampered byte {index}"
            );
        }
        let current = store.read(&token, &binding.question()).unwrap();
        let foreign = ViewCursorStore::default();
        let foreign_token = foreign
            .insert_snapshot(binding.clone(), json!({}), vec![json!(1), json!(2)], 0)
            .unwrap();
        assert_eq!(
            foreign.read(&token, &binding.question()).unwrap_err(),
            ViewCursorError::Invalid
        );
        assert_eq!(
            foreign.insert_next(&current, 1, &foreign_token),
            Err(ViewCursorError::Invalid)
        );
        let key = store.state.lock().unwrap().key.as_ref().unwrap().clone();
        for (id, offset) in [(0, 0), (u64::MAX, 0), (1, usize::MAX)] {
            let crafted = issue_view_cursor(&key, id, offset).unwrap();
            assert_eq!(
                store.read(&crafted, &binding.question()).unwrap_err(),
                ViewCursorError::Invalid
            );
        }
        use aes_siv::KeyInit;
        let mut cipher = aes_siv::siv::Aes256Siv::new_from_slice(&key[..]).unwrap();
        let wrong_domain = cipher
            .encrypt([SEARCH_CURSOR_AAD], &[0u8; VIEW_CURSOR_PAYLOAD_BYTES])
            .unwrap();
        assert_eq!(
            store
                .read(&encode_view_cursor(&wrong_domain), &binding.question())
                .unwrap_err(),
            ViewCursorError::Invalid
        );
        assert_eq!(store.state.lock().unwrap().snapshots.len(), 1);
    }

    #[test]
    fn view_entropy_failure_never_replaces_an_installed_owner_or_promise() {
        fn partial_failure(bytes: &mut [u8]) -> Result<(), ViewCursorError> {
            bytes[0] = 7;
            Err(ViewCursorError::EntropyUnavailable)
        }
        let mut store = ViewCursorStore {
            entropy: Some(partial_failure),
            ..ViewCursorStore::default()
        };
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        assert_eq!(
            store.insert_snapshot(binding.clone(), json!({}), vec![json!(1), json!(2)], 0),
            Err(ViewCursorError::EntropyUnavailable)
        );
        {
            let state = store.state.lock().unwrap();
            assert!(state.key.is_none() && state.snapshots.is_empty());
            assert_eq!(state.snapshots.capacity(), 0);
            assert_eq!(state.snapshot_bytes, 0);
            assert_eq!(state.payload_bytes, 0);
            assert_eq!(state.payload_count, 0);
        }
        store.entropy = None;
        let token = store
            .insert_snapshot(binding.clone(), json!({}), vec![json!(1), json!(2)], 0)
            .unwrap();
        let key = store.state.lock().unwrap().key.as_ref().unwrap().clone();
        store.entropy = Some(partial_failure);
        let current = store.read(&token, &binding.question()).unwrap();
        let next = store.insert_next(&current, 1, &token).unwrap();
        assert_eq!(store.insert_next(&current, 1, &token).unwrap(), next);
        assert!(
            store
                .insert_snapshot(binding.clone(), json!({}), vec![json!(3), json!(4)], 0)
                .is_ok(),
            "new chains use the existing key, not fresh entropy"
        );
        assert_eq!(
            &store.state.lock().unwrap().key.as_ref().unwrap()[..],
            &key[..]
        );
        assert_eq!(store.read(&token, &binding.question()).unwrap().offset, 0);
    }

    #[test]
    fn poisoned_view_owner_is_distinct_from_cursor_or_budget_refusal() {
        let store = Arc::new(ViewCursorStore::default());
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let token = store
            .insert_snapshot(binding.clone(), json!({}), vec![json!(1), json!(2)], 0)
            .unwrap();
        let current = store.read(&token, &binding.question()).unwrap();
        let poisoned = Arc::clone(&store);
        assert!(std::thread::spawn(move || {
            let _lock = poisoned.state.lock().unwrap();
            panic!("poison view owner");
        })
        .join()
        .is_err());
        assert_eq!(
            store.read(&token, &binding.question()).unwrap_err(),
            ViewCursorError::OwnerUnavailable
        );
        assert_eq!(
            store.insert_next(&current, 1, &token),
            Err(ViewCursorError::OwnerUnavailable)
        );
        assert_eq!(
            store.insert_snapshot(binding, json!({}), vec![json!(3)], 0),
            Err(ViewCursorError::OwnerUnavailable)
        );
    }

    #[test]
    fn view_cursor_failures_keep_typed_causes_on_the_wire() {
        for (error, code, detail) in [
            (ViewCursorError::Invalid, "invalid_cursor", None),
            (ViewCursorError::CapacityExceeded, "result_too_large", None),
            (
                ViewCursorError::OwnerUnavailable,
                "provider_unavailable",
                Some("cache_poisoned"),
            ),
            (
                ViewCursorError::EntropyUnavailable,
                "provider_unavailable",
                Some("provider_absent"),
            ),
            (ViewCursorError::TokenUnavailable, "provider_failed", None),
            (
                ViewCursorError::OffsetNotRepresentable,
                "provider_failed",
                None,
            ),
            (
                ViewCursorError::AllocationUnavailable,
                "provider_failed",
                None,
            ),
        ] {
            let result = error.into_result(Some("main:Configuration".into()));
            assert_eq!(result.diagnostics[0]["code"], code);
            assert_eq!(
                result.diagnostics[0]
                    .get("detailCode")
                    .and_then(Value::as_str),
                detail
            );
        }
        assert_eq!(
            view_add(usize::MAX, 1),
            Err(ViewCursorError::OffsetNotRepresentable)
        );
        assert_eq!(
            view_mul(usize::MAX, 2),
            Err(ViewCursorError::OffsetNotRepresentable)
        );
    }

    #[test]
    fn every_promised_page_survives_a_chain_longer_than_128_pages() {
        let store = ViewCursorStore::default();
        let binding = view_binding("main:Document.Заказ.Module.Object.Body", "rev-1");
        let items = (0..260)
            .map(|line| json!({"line": line}))
            .collect::<Vec<_>>();
        let mut token = store
            .insert_snapshot(
                binding.clone(),
                json!({"at": binding.canonical_at}),
                items.clone(),
                0,
            )
            .unwrap();
        let mut promised = Vec::new();
        for offset in 0..items.len() {
            let page = store.read(&token, &binding.question()).unwrap();
            assert_eq!(page.offset, offset);
            assert_eq!(page.snapshot.items()[offset], items[offset]);
            let next = (offset + 1 < items.len())
                .then(|| store.insert_next(&page, offset + 1, &token).unwrap());
            promised.push((token.clone(), offset, next.clone()));
            if let Some(next) = next {
                token = next;
            }
        }
        for (token, offset, expected_next) in promised {
            let page = store
                .read(&token, &binding.question())
                .expect("every issued page must survive subsequent page requests");
            assert_eq!(page.offset, offset);
            assert_eq!(page.snapshot.items(), items);
            let actual_next = (offset + 1 < items.len())
                .then(|| store.insert_next(&page, offset + 1, &token).unwrap());
            assert_eq!(
                actual_next, expected_next,
                "retry must return the same successor"
            );
        }
    }

    #[test]
    fn sparse_and_empty_object_allocations_are_compacted_before_retention() {
        let mut object = serde_json::Map::with_capacity(16_384);
        object.insert("small".into(), json!(1));
        let original_entry = object.get_key_value("small").unwrap().0 as *const String as usize;
        let mut empty = serde_json::Map::with_capacity(16_384);
        empty.insert("removed".into(), json!(1));
        empty.clear();
        let binding = view_binding("main:CommonModule.Owner.Method", "rev-1");
        let store = ViewCursorStore::new(4_096);
        let token = store
            .insert_snapshot(
                binding.clone(),
                Value::Object(object),
                vec![Value::Object(empty), json!(2)],
                0,
            )
            .unwrap();
        let retained = store.read(&token, &binding.question()).unwrap();
        let compact = retained.snapshot.node().as_object().unwrap();
        let retained_entry = compact.get_key_value("small").unwrap().0 as *const String as usize;
        assert_ne!(
            original_entry, retained_entry,
            "the original sparse object allocation must not be retained"
        );
        assert_eq!(retained.snapshot.node(), &json!({"small": 1}));
        assert_eq!(retained.snapshot.items()[0], json!({}));
        let state = store.state.lock().unwrap();
        assert!(
            view_retained_charge(state.snapshot_bytes, state.snapshots.capacity()).unwrap()
                <= 4_096
        );
        assert_eq!(compact_view_object_charge(0).unwrap(), 0);
    }
}
