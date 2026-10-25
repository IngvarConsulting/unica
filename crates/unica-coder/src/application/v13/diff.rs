use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::Mutex;
use unica_address::address::QualifiedAddress;
use unica_refusal::RefusalCode;
use uuid::Uuid;

use super::view::PREFERRED_PAGE_BYTES;
use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;

pub(crate) const DEFAULT_DIFF_LIMIT: usize = 20;
pub(crate) const MAX_DIFF_LIMIT: usize = 50;
const MAX_CURSOR_ENTRIES: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffRequest {
    left: QualifiedAddress,
    right: QualifiedAddress,
    limit: usize,
    cursor: Option<String>,
    workspace: String,
}

impl DiffRequest {
    pub(crate) fn new(left: &str, right: &str) -> Result<Self, DiffError> {
        let left = QualifiedAddress::parse(left).map_err(|error| DiffError::BadValue {
            field: "left".to_string(),
            message: error.to_string(),
        })?;
        let right = QualifiedAddress::parse(right).map_err(|error| DiffError::BadValue {
            field: "right".to_string(),
            message: error.to_string(),
        })?;
        Ok(Self {
            left,
            right,
            limit: DEFAULT_DIFF_LIMIT,
            cursor: None,
            workspace: String::new(),
        })
    }

    pub(crate) fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.clamp(1, MAX_DIFF_LIMIT);
        self
    }

    pub(crate) fn with_cursor(mut self, cursor: String) -> Self {
        self.cursor = Some(cursor);
        self
    }

    /// A continuation answers only the workspace that asked the first page.
    pub(crate) fn in_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = workspace.into();
        self
    }

    pub(crate) fn left(&self) -> &QualifiedAddress {
        &self.left
    }

    pub(crate) fn right(&self) -> &QualifiedAddress {
        &self.right
    }

    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    fn fingerprint(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.workspace, self.left, self.right, self.limit
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DiffSource {
    at: QualifiedAddress,
    kind: String,
    data: Value,
}

impl DiffSource {
    pub(crate) fn new(at: QualifiedAddress, kind: impl Into<String>, data: Value) -> Self {
        Self {
            at,
            kind: kind.into(),
            data,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DiffChange {
    path: String,
    left: Option<Value>,
    right: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct DiffPage {
    equal: bool,
    changes: Vec<DiffChange>,
    truncated: bool,
    /// The continuation travels in the envelope, not in the page data.
    #[serde(skip)]
    cursor: Option<String>,
    #[serde(skip)]
    stopped_by: &'static str,
}

impl DiffPage {
    pub(crate) fn changes(&self) -> &[DiffChange] {
        &self.changes
    }

    pub(crate) fn truncated(&self) -> bool {
        self.truncated
    }

    pub(crate) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    /// Why the page ended: `limit`, `bytes` or `complete`.
    pub(crate) fn stopped_by(&self) -> &'static str {
        self.stopped_by
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DiffError {
    BadValue { field: String, message: String },
    IncomparableNodes,
    InvalidCursor,
    ChangeTooLarge,
}

impl DiffError {
    pub(crate) const fn code(&self) -> RefusalCode {
        match self {
            Self::BadValue { .. } => RefusalCode::BadValue,
            Self::IncomparableNodes => RefusalCode::IncomparableNodes,
            Self::InvalidCursor => RefusalCode::InvalidCursor,
            Self::ChangeTooLarge => RefusalCode::ResultTooLarge,
        }
    }
}

impl fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadValue { field, message } => write!(formatter, "{field}: {message}"),
            Self::IncomparableNodes => {
                formatter.write_str("diff requires nodes of the same logical kind")
            }
            Self::InvalidCursor => {
                formatter.write_str("diff cursor is invalid or belongs to another question")
            }
            Self::ChangeTooLarge => formatter.write_str(
                "one difference exceeds the transport result limit; compare smaller nodes",
            ),
        }
    }
}

/// One saved pair serves every page of its walk. A token names the pair and
/// an offset this store issued for it, so a walk holds one entry however many
/// pages it reads.
#[derive(Debug, Clone)]
struct CursorEntry {
    fingerprint: String,
    left_data: std::sync::Arc<Value>,
    right_data: std::sync::Arc<Value>,
    offsets: BTreeSet<usize>,
}

#[derive(Debug, Default)]
struct SavedPairs {
    entries: HashMap<String, CursorEntry>,
    order: VecDeque<String>,
}

#[derive(Debug, Default)]
struct DiffCursorStore {
    pairs: Mutex<SavedPairs>,
}

impl DiffCursorStore {
    fn issue(&self, pair: Option<&str>, entry: CursorEntry, offset: usize) -> String {
        let mut pairs = self.pairs.lock().expect("diff cursor store poisoned");
        if let Some(saved) = pair.and_then(|pair| pairs.entries.get_mut(pair)) {
            saved.offsets.insert(offset);
            return format!("dc1.{}.{offset}", pair.expect("pair named"));
        }
        while pairs.entries.len() >= MAX_CURSOR_ENTRIES {
            let Some(oldest) = pairs.order.pop_front() else {
                break;
            };
            pairs.entries.remove(&oldest);
        }
        let id = Uuid::new_v4().simple().to_string();
        let mut entry = entry;
        entry.offsets.insert(offset);
        pairs.entries.insert(id.clone(), entry);
        pairs.order.push_back(id.clone());
        format!("dc1.{id}.{offset}")
    }

    fn read(&self, token: &str) -> Result<(String, CursorEntry, usize), DiffError> {
        let (id, offset) = token
            .strip_prefix("dc1.")
            .and_then(|rest| rest.split_once('.'))
            .ok_or(DiffError::InvalidCursor)?;
        if id.len() != 32
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            || offset.is_empty()
            || !offset.bytes().all(|byte| byte.is_ascii_digit())
            || (offset.len() > 1 && offset.starts_with('0'))
        {
            return Err(DiffError::InvalidCursor);
        }
        let offset = offset
            .parse::<usize>()
            .map_err(|_| DiffError::InvalidCursor)?;
        let pairs = self.pairs.lock().expect("diff cursor store poisoned");
        let entry = pairs
            .entries
            .get(id)
            .filter(|entry| entry.offsets.contains(&offset))
            .cloned()
            .ok_or(DiffError::InvalidCursor)?;
        Ok((id.to_owned(), entry, offset))
    }
}

#[derive(Debug, Default)]
pub(crate) struct DiffHandler {
    cursors: DiffCursorStore,
}

impl DiffHandler {
    pub(crate) fn compare(
        &self,
        request: &DiffRequest,
        left: &DiffSource,
        right: &DiffSource,
    ) -> Result<DiffPage, DiffError> {
        if request.left != left.at || request.right != right.at {
            return Err(DiffError::BadValue {
                field: "source".to_string(),
                message: "sources do not match the logical diff request".to_string(),
            });
        }
        if request.cursor.is_some() {
            return self.continue_saved(request);
        }
        if left.kind != right.kind {
            return Err(DiffError::IncomparableNodes);
        }
        let left_data = source_backed(&left.data);
        let right_data = source_backed(&right.data);
        if left_data.get("kind") != right_data.get("kind") {
            return Err(DiffError::IncomparableNodes);
        }
        self.page(
            request,
            None,
            std::sync::Arc::new(left_data),
            std::sync::Arc::new(right_data),
            0,
        )
    }

    /// Continues the pair saved by the first page. The sources are not read
    /// again: a later edit of either side does not replace the saved pair.
    pub(crate) fn continue_saved(&self, request: &DiffRequest) -> Result<DiffPage, DiffError> {
        let token = request.cursor.as_deref().ok_or(DiffError::InvalidCursor)?;
        let fingerprint = request.fingerprint();
        let (pair, entry, offset) = self.cursors.read(token)?;
        if entry.fingerprint != fingerprint {
            return Err(DiffError::InvalidCursor);
        }
        self.page(
            request,
            Some(&pair),
            entry.left_data,
            entry.right_data,
            offset,
        )
    }

    fn page(
        &self,
        request: &DiffRequest,
        pair: Option<&str>,
        left_data: std::sync::Arc<Value>,
        right_data: std::sync::Arc<Value>,
        offset: usize,
    ) -> Result<DiffPage, DiffError> {
        let (envelope, fingerprint, limit) = (
            envelope_bytes(request),
            request.fingerprint(),
            request.limit,
        );
        let mut skip = offset;
        let mut changes = Vec::with_capacity(limit.saturating_add(1));
        collect_changes(
            "",
            &left_data,
            &right_data,
            &mut skip,
            limit.saturating_add(1),
            &mut changes,
        );
        // The answer around the changes counts too: a difference fits only
        // with the envelope that carries it.
        let largest = MAX_CANONICAL_RESULT_BYTES.saturating_sub(envelope);
        // A page ends by count or by encoded size; a difference is never cut,
        // so one larger than the page target travels alone.
        let mut kept = 0;
        let mut bytes = envelope;
        let mut stopped_by = "complete";
        for change in &changes {
            if kept == limit {
                stopped_by = "limit";
                break;
            }
            // One separator between array items.
            let size = encoded_len(change).saturating_add(1);
            if size > largest {
                return Err(DiffError::ChangeTooLarge);
            }
            if kept > 0 && bytes.saturating_add(size) > PREFERRED_PAGE_BYTES {
                stopped_by = "bytes";
                break;
            }
            bytes = bytes.saturating_add(size);
            kept += 1;
        }
        let truncated = kept < changes.len();
        // A difference nobody could read refuses before the first cursor, not
        // on the page that would carry it.
        if pair.is_none() && truncated && any_change_over("", &left_data, &right_data, largest) {
            return Err(DiffError::ChangeTooLarge);
        }
        changes.truncate(kept);
        let next_cursor = truncated.then(|| {
            self.cursors.issue(
                pair,
                CursorEntry {
                    fingerprint,
                    left_data: left_data.clone(),
                    right_data: right_data.clone(),
                    offsets: BTreeSet::new(),
                },
                offset.saturating_add(changes.len()),
            )
        });
        Ok(DiffPage {
            equal: changes.is_empty() && !truncated,
            changes,
            truncated,
            cursor: next_cursor,
            stopped_by,
        })
    }
}

/// The encoded size of the answer around a page of changes: the closed
/// fields, a continuation of the longest token shape and its `next` call.
fn envelope_bytes(request: &DiffRequest) -> usize {
    let cursor = format!("dc1.{}.{}", "f".repeat(32), usize::MAX);
    let envelope = serde_json::json!({
        "ok": true,
        "summary": "logical nodes compared; more differences remain; continue with cursor",
        "data": {"equal": false, "changes": [], "truncated": true},
        "next": [{"tool": "unica.diff", "arguments": {
            "left": request.left.to_string(),
            "right": request.right.to_string(),
            "limit": request.limit,
            "cursor": cursor,
        }}],
        "cursor": cursor,
        "page": {"stoppedBy": "complete"},
    });
    encoded_len(&envelope)
}

#[derive(Serialize)]
struct ChangeRef<'a> {
    path: &'a str,
    left: Option<&'a Value>,
    right: Option<&'a Value>,
}

struct CountingWriter(usize);

impl std::io::Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encoded_len(value: &impl Serialize) -> usize {
    let mut writer = CountingWriter(0);
    serde_json::to_writer(&mut writer, value).map_or(usize::MAX, |()| writer.0)
}

/// Whether any difference of the pair encodes larger than `largest` bytes,
/// walked in the page order without copying a value.
fn any_change_over(path: &str, left: &Value, right: &Value, largest: usize) -> bool {
    if left == right {
        return false;
    }
    let over = |path: &str, left: Option<&Value>, right: Option<&Value>| {
        encoded_len(&ChangeRef { path, left, right }).saturating_add(1) > largest
    };
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => left
            .keys()
            .chain(right.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .any(|key| {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child_path = format!("{path}/{escaped}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => any_change_over(&child_path, left, right, largest),
                    (left, right) => over(&child_path, left, right),
                }
            }),
        (Value::Array(left), Value::Array(right)) => {
            (0..left.len().max(right.len())).any(|index| {
                let child_path = format!("{path}/{index}");
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => any_change_over(&child_path, left, right, largest),
                    (left, right) => over(&child_path, left, right),
                }
            })
        }
        _ => over(path_or_root(path), Some(left), Some(right)),
    }
}

fn collect_changes(
    path: &str,
    left: &Value,
    right: &Value,
    skip: &mut usize,
    limit: usize,
    changes: &mut Vec<DiffChange>,
) {
    if left == right || changes.len() >= limit {
        return;
    }
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for key in keys {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                let child_path = format!("{path}/{escaped}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => {
                        collect_changes(&child_path, left, right, skip, limit, changes)
                    }
                    (left, right) => emit_change(&child_path, left, right, skip, limit, changes),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                let child_path = format!("{path}/{index}");
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => {
                        collect_changes(&child_path, left, right, skip, limit, changes)
                    }
                    (left, right) => emit_change(&child_path, left, right, skip, limit, changes),
                }
                if changes.len() >= limit {
                    break;
                }
            }
        }
        _ => emit_change(
            path_or_root(path),
            Some(left),
            Some(right),
            skip,
            limit,
            changes,
        ),
    }
}

fn emit_change(
    path: &str,
    left: Option<&Value>,
    right: Option<&Value>,
    skip: &mut usize,
    limit: usize,
    changes: &mut Vec<DiffChange>,
) {
    if *skip > 0 {
        *skip -= 1;
        return;
    }
    if changes.len() < limit {
        changes.push(DiffChange {
            path: path.to_string(),
            left: left.cloned(),
            right: right.cloned(),
        });
    }
}

fn path_or_root(path: &str) -> &str {
    if path.is_empty() {
        "/"
    } else {
        path
    }
}

/// `can` и `limits` вычисляются из реестра и места узла, а не читаются из
/// исходников: сравнение их никогда не увидело бы изменения источника.
fn source_backed(data: &Value) -> Value {
    let mut data = data.clone();
    if let Some(node) = data.as_object_mut() {
        node.remove("can");
        node.remove("limits");
    }
    data
}

#[cfg(test)]
mod tests {
    use super::{DiffError, DiffHandler, DiffPage, DiffRequest, DiffSource};
    use serde_json::json;
    use unica_address::address::QualifiedAddress;

    fn source(at: &str, kind: &str, _snapshot_label: &str, value: i64) -> DiffSource {
        DiffSource::new(
            QualifiedAddress::parse(at).unwrap(),
            kind,
            json!({"kind": kind, "props": {"value": value}, "items": (0..50).map(|n| json!({"n": n + value})).collect::<Vec<_>>() }),
        )
    }

    #[test]
    fn diff_rejects_incomparable_logical_kinds() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Role.Manager").unwrap();
        let error = handler
            .compare(
                &request,
                &source("main:Catalog.Items", "Catalog", "left-1", 1),
                &source("main:Role.Manager", "Role", "right-1", 1),
            )
            .unwrap_err();
        assert_eq!(error.code().as_str(), "incomparable_nodes");
    }

    #[test]
    fn diff_limit_bounds_materialized_changes_and_issues_a_cursor() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(2);
        let page = handler
            .compare(
                &request,
                &source("main:Catalog.Items", "Catalog", "left-1", 1),
                &source("main:Catalog.Items", "Catalog", "right-1", 2),
            )
            .unwrap();
        assert_eq!(page.changes().len(), 2);
        assert!(page.truncated());
        assert!(page.cursor().is_some());
    }

    #[test]
    fn diff_cursor_replays_the_saved_comparison_after_sources_change() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(1);
        let left = source("main:Catalog.Items", "Catalog", "left", 1);
        let right = source("main:Catalog.Items", "Catalog", "right", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let continued = request.with_cursor(first.cursor().unwrap().to_string());
        let expected = handler.compare(&continued, &left, &right).unwrap();
        let changed_left = source("main:Catalog.Items", "Document", "changed", 100);
        let changed_right = source("main:Catalog.Items", "Catalog", "changed", 200);
        let replayed = handler
            .compare(&continued, &changed_left, &changed_right)
            .unwrap();
        assert_eq!(replayed.changes(), expected.changes());
        assert_eq!(replayed.truncated(), expected.truncated());
        assert!(replayed.cursor().is_some());
    }

    #[test]
    fn diff_cursor_cannot_be_replayed_for_another_question() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(1);
        let left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let cursor = first.cursor().unwrap();
        let replay = request.with_cursor(cursor.to_string()).with_limit(2);
        assert!(matches!(
            handler.compare(&replay, &left, &right),
            Err(DiffError::InvalidCursor)
        ));
    }

    #[test]
    fn diff_cursor_continues_from_the_bounded_change_offset() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items")
            .unwrap()
            .with_limit(2);
        let left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 2);
        let first = handler.compare(&request, &left, &right).unwrap();
        let second = handler
            .compare(
                &request.with_cursor(first.cursor().unwrap().to_string()),
                &left,
                &right,
            )
            .unwrap();
        assert_eq!(first.changes()[0].path, "/items/0/n");
        assert_eq!(second.changes()[0].path, "/items/2/n");
        assert_ne!(first.changes()[0].path, second.changes()[0].path);
    }

    fn wide(at: &str, sizes: &[usize], fill: char) -> DiffSource {
        let props = sizes
            .iter()
            .enumerate()
            .map(|(n, bytes)| (format!("f{n:02}"), json!(fill.to_string().repeat(*bytes))))
            .collect::<serde_json::Map<_, _>>();
        DiffSource::new(
            QualifiedAddress::parse(at).unwrap(),
            "Catalog",
            json!({"kind": "Catalog", "props": props}),
        )
    }

    fn walk(handler: &DiffHandler, request: &DiffRequest, sizes: &[usize]) -> Vec<DiffPage> {
        let at = "main:Catalog.Items";
        let (left, right) = (wide(at, sizes, 'a'), wide(at, sizes, 'b'));
        let mut pages = vec![handler.compare(request, &left, &right).unwrap()];
        while let Some(cursor) = pages.last().unwrap().cursor() {
            let next = request.clone().with_cursor(cursor.to_string());
            pages.push(handler.compare(&next, &left, &right).unwrap());
        }
        pages
    }

    /// A page ends by count, by encoded size or at the end, and says which;
    /// pages walk every difference once, and a difference larger than the
    /// page target still travels whole.
    #[test]
    fn diff_page_names_why_it_stopped_and_never_cuts_a_difference() {
        let handler = DiffHandler::default();
        let at = "main:Catalog.Items";
        let request = DiffRequest::new(at, at).unwrap();

        let counted = walk(&handler, &request.clone().with_limit(2), &[8, 8, 8]);
        let stops = counted.iter().map(DiffPage::stopped_by).collect::<Vec<_>>();
        assert_eq!(stops, ["limit", "complete"]);
        assert_eq!(counted[1].changes().len(), 1);

        // Twenty changes of about 6 KiB: several fit under 64 KiB, the next
        // one does not, and the walk still reads all twenty exactly once.
        let sizes = [3 * 1024; 20];
        let sized = walk(&handler, &request, &sizes);
        assert!(
            sized.len() > 1,
            "a 120 KiB difference needs more than one page"
        );
        for page in &sized[..sized.len() - 1] {
            assert_eq!(page.stopped_by(), "bytes");
            assert!(
                page.changes().len() > 1,
                "several small differences share a page"
            );
            let encoded = serde_json::to_vec(page.changes()).unwrap().len();
            assert!(encoded <= super::PREFERRED_PAGE_BYTES, "{encoded}");
        }
        assert_eq!(sized.last().unwrap().stopped_by(), "complete");
        let paths = sized
            .iter()
            .flat_map(|page| page.changes().iter().map(|change| change.path.clone()))
            .collect::<Vec<_>>();
        let expected = (0..20)
            .map(|n| format!("/props/f{n:02}"))
            .collect::<Vec<_>>();
        assert_eq!(paths, expected);

        let over_target = walk(&handler, &request, &[100 * 1024]);
        assert_eq!(over_target.len(), 1);
        assert_eq!(over_target[0].stopped_by(), "complete");
        assert_eq!(
            over_target[0].changes().len(),
            1,
            "an indivisible difference travels whole"
        );
    }

    /// A difference over the transport limit cannot travel at all: the
    /// comparison refuses before it promises a continuation, wherever that
    /// difference stands.
    #[test]
    fn a_difference_over_the_transport_limit_refuses_before_a_cursor() {
        let handler = DiffHandler::default();
        let at = "main:Catalog.Items";
        let request = DiffRequest::new(at, at).unwrap();
        let huge = 5 * 1024 * 1024;
        for sizes in [
            vec![huge],
            vec![8, huge],
            vec![8; 3].into_iter().chain([huge]).collect(),
        ] {
            let error = handler
                .compare(
                    &request.clone().with_limit(1),
                    &wide(at, &sizes, 'a'),
                    &wide(at, &sizes, 'b'),
                )
                .unwrap_err();
            assert_eq!(error.code().as_str(), "result_too_large", "{sizes:?}");
        }
    }

    /// `can` и `limits` вычисляются для `view`, а не читаются из исходников:
    /// у узлов с одинаковыми данными словари не дают ложного различия.
    #[test]
    fn diff_ignores_computed_view_sections() {
        let handler = DiffHandler::default();
        let request = DiffRequest::new("main:Catalog.Items", "main:Catalog.Items").unwrap();
        let mut left = source("main:Catalog.Items", "Catalog", "left-1", 1);
        let right = source("main:Catalog.Items", "Catalog", "right-1", 1);
        left.data["can"] = json!([{"op": "props.set"}]);
        left.data["limits"] = json!({"items": 1});
        let page = handler.compare(&request, &left, &right).unwrap();
        assert!(page.changes().is_empty(), "computed sections leaked");
    }
}
