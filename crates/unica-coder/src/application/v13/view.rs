use crate::domain::node_view::SourceText;
use crate::domain::node_view::{NodeView, NodeViewData, TextView};
use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;
use unica_execution_contract::view_error::{error_result, ViewError};
use unica_navigation::result_store::{
    ViewCursorBinding, ViewCursorError, ViewCursorQuestion, ViewCursorStore,
};
use {
    crate::application::v13::source_text::ModuleText,
    unica_execution_contract::source_text_budget::body_text_result,
    unica_execution_contract::source_text_budget::oversized_body,
};

use serde_json::{json, Map, Value};
use std::io::{self, Write};
use std::sync::Arc;
use unica_address::address::QualifiedAddress;
use unica_execution_contract::invocation::DomainResult;
use unica_refusal::RefusalCode;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
pub(crate) const PREFERRED_PAGE_BYTES: usize = 64 * 1024;
type ViewProjector<'a> = dyn Fn(&Value) -> Result<Value, ViewError> + 'a;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewRequest {
    at: QualifiedAddress,
    limit: usize,
    cursor: Option<String>,
}

impl ViewRequest {
    pub(crate) fn new(at: &str) -> Result<Self, ViewError> {
        let at = QualifiedAddress::parse(at)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        Ok(Self {
            at,
            limit: DEFAULT_LIMIT,
            cursor: None,
        })
    }

    pub(crate) fn at(&self) -> String {
        self.at.to_string()
    }

    pub(crate) const fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn cursor(&self) -> Option<&str> {
        self.cursor.as_deref()
    }

    pub(crate) fn with_limit(mut self, limit: usize) -> Result<Self, ViewError> {
        if limit == 0 || limit > MAX_LIMIT {
            return Err(ViewError::new(
                RefusalCode::BadValue,
                format!("view limit must be between 1 and {MAX_LIMIT}"),
            ));
        }
        self.limit = limit;
        Ok(self)
    }

    pub(crate) fn with_cursor(mut self, cursor: String) -> Self {
        self.cursor = Some(cursor);
        self
    }

    fn continuation_question(&self, source_set_identity: String) -> ViewCursorQuestion {
        ViewCursorQuestion {
            canonical_at: self.at.to_string(),
            projection: self
                .at
                .segments()
                .last()
                .map(|segment| segment.kind().as_str())
                .unwrap_or("Configuration")
                .to_string(),
            source_set_identity,
            page_limit: self.limit,
        }
    }

    fn binding(
        &self,
        canonical_at: &QualifiedAddress,
        snapshot: &ViewSourceSnapshot,
    ) -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: self.at.to_string(),
            projection: canonical_at
                .segments()
                .last()
                .map(|segment| segment.kind().as_str())
                .unwrap_or("Configuration")
                .to_string(),
            source_set_identity: snapshot.source_set_identity.clone(),
            snapshot_id: snapshot.revision.clone(),
            page_limit: self.limit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewSourceSnapshot {
    pub(crate) source_set_identity: String,
    pub(crate) revision: String,
}

pub(crate) trait ViewReadAuthority: Send + Sync {
    fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError>;

    fn read_checkpoint(&self) -> Result<(), ViewError> {
        Ok(())
    }

    fn canonical_address(
        &self,
        at: &QualifiedAddress,
        _admitted: &ViewSourceSnapshot,
    ) -> Result<QualifiedAddress, ViewError> {
        Ok(at.clone())
    }

    /// Internal identity fact for `find`; it is never serialized by `view`.
    fn identity_export_path(&self, _at: &QualifiedAddress) -> Result<Option<String>, ViewError> {
        Ok(None)
    }

    /// Exact typed profile gaps may retain their parent-projected identity in
    /// `find` even though source-backed `view` fails. This is false by default;
    /// malformed or wrong-owner provider evidence must never opt in.
    fn permits_identity_fallback(&self, _at: &QualifiedAddress) -> bool {
        false
    }

    fn read_exact(
        &self,
        at: &QualifiedAddress,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError>;

    /// Текст модуля для его `Body`, прочитанный без разбора и не дальше
    /// предела `MAX_SOURCE_TEXT_BYTES`. `None` — адрес не `Body` модуля BSL.
    fn read_module_text(
        &self,
        _at: &QualifiedAddress,
        _admitted: &ViewSourceSnapshot,
    ) -> Result<Option<ModuleText>, ViewError> {
        Ok(None)
    }

    /// Ревизия текста модуля, чей `Body` или `Body` метода назван адресом:
    /// её принимает `ifRev` у `code.replace` (#2225). `None` — адрес не
    /// текст модуля BSL.
    fn module_text_revision(&self, _at: &QualifiedAddress) -> Result<Option<String>, ViewError> {
        Ok(None)
    }
}

pub(crate) struct ViewService<A> {
    authority: A,
    cursors: Arc<ViewCursorStore>,
}

impl<A: ViewReadAuthority> ViewService<A> {
    #[cfg(test)]
    pub(crate) fn authority_for_test(&self) -> &A {
        &self.authority
    }

    pub(crate) fn new(authority: A, cursors: ViewCursorStore) -> Self {
        Self {
            authority,
            cursors: Arc::new(cursors),
        }
    }

    pub(crate) fn with_shared_cursors(authority: A, cursors: Arc<ViewCursorStore>) -> Self {
        Self { authority, cursors }
    }

    pub(crate) fn view(&self, request: ViewRequest) -> DomainResult {
        match self.try_view(&request, None, None, None) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    pub(crate) fn view_projected(
        &self,
        request: ViewRequest,
        project: &ViewProjector<'_>,
    ) -> DomainResult {
        match self.try_view(&request, Some(project), None, None) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    /// Page a collection supplied by a non-source provider while binding its
    /// cursor to the owning source node. Continuations reuse the captured
    /// collection; `data` is required only for the first page.
    pub(crate) fn view_preloaded_collection(
        &self,
        request: ViewRequest,
        source_at: &QualifiedAddress,
        expected_revision: &str,
        data: Option<Value>,
    ) -> DomainResult {
        match self.try_view(
            &request,
            None,
            Some((source_at, expected_revision, data)),
            None,
        ) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    pub(crate) fn view_saved_collection(
        &self,
        request: ViewRequest,
        source_at: &QualifiedAddress,
    ) -> DomainResult {
        if request.cursor().is_none() {
            return error_result(
                Some(request.at()),
                ViewError::new(
                    RefusalCode::InvalidState,
                    "a saved collection requires its issued cursor",
                ),
            );
        }
        match self.try_view(&request, None, None, Some(source_at)) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at()), error),
        }
    }

    fn try_view(
        &self,
        request: &ViewRequest,
        project: Option<&ViewProjector<'_>>,
        supplied: Option<(&QualifiedAddress, &str, Option<Value>)>,
        continuation_source: Option<&QualifiedAddress>,
    ) -> Result<DomainResult, ViewError> {
        let source_at = supplied
            .as_ref()
            .map_or(continuation_source.unwrap_or(&request.at), |(at, _, _)| *at);
        let snapshot = self.authority.snapshot(source_at)?;
        if let Some(cursor) = request.cursor.as_deref() {
            let question = request.continuation_question(snapshot.source_set_identity.clone());
            let stored = self.cursors.read(cursor, &question).map_err(cursor_error)?;
            // The window holds one item past the page: its presence alone
            // tells the page whether a continuation follows.
            let window = stored
                .snapshot
                .read_items(stored.offset, request.limit.saturating_add(1))
                .map_err(cursor_error)?;
            let page = prepare_page(
                stored.snapshot.node(),
                &window,
                0,
                request.limit,
                &stored.snapshot.binding,
            )?;
            let next_offset = stored.offset + page.next_offset;
            let next_cursor = if next_offset < stored.snapshot.len() {
                self.cursors
                    .insert_next(&stored, next_offset, cursor)
                    .map_err(cursor_error)?
                    .into()
            } else {
                None
            };
            return collection_result(
                stored.snapshot.node(),
                page.items,
                next_cursor,
                page.stopped_by,
                &stored.snapshot.binding,
            );
        }

        if supplied.as_ref().is_some_and(|(_, expected, _)| {
            request.cursor.is_none() && snapshot.revision != *expected
        }) {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "source changed while the supplied collection was read; retry the question",
            ));
        }
        let canonical_at = if supplied.is_some() || request.cursor.is_some() {
            request.at.clone()
        } else {
            self.authority.canonical_address(&request.at, &snapshot)?
        };
        let binding = request.binding(&canonical_at, &snapshot);

        let mut text = None;
        // The revision of a Body's module text (#2225), read on both sides
        // of the text: equal, it names the very bytes this answer gives.
        let mut text_revision = None;
        let serialized = if let Some((_, _, data)) = supplied {
            data.ok_or_else(|| {
                ViewError::new(
                    RefusalCode::InvalidState,
                    "a new supplied collection has no provider result",
                )
            })?
        } else {
            let revision_before = self.authority.module_text_revision(&canonical_at)?;
            let mut view = match self.authority.read_module_text(&canonical_at, &snapshot)? {
                Some(text) => module_body_view(&canonical_at, text)?,
                None => self.authority.read_exact(&canonical_at, &snapshot)?,
            };
            text = view.take_text();
            if let (Some(before), Some(_)) = (revision_before, &text) {
                if self.authority.module_text_revision(&canonical_at)?.as_ref() != Some(&before) {
                    return Err(ViewError::new(
                        RefusalCode::ConcurrentChange,
                        "the module changed while its text was read; retry the question",
                    ));
                }
                text_revision = Some(before);
            }
            let serialized = serde_json::to_value(view).map_err(|error| {
                ViewError::new(RefusalCode::ProviderUnavailable, error.to_string())
            })?;
            match project {
                Some(project) => project(&serialized)?,
                None => serialized,
            }
        };
        if serialized.get("at").and_then(Value::as_str) != Some(canonical_at.to_string().as_str()) {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "reader returned a projection for another logical address",
            ));
        }
        let mut object = serialized.as_object().cloned().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "typed node projection is not an object",
            )
        })?;
        if let Some(text) = text {
            // Проекция может ответить на другой вопрос о том же узле
            // (контракт операции `.Can.<op>`): текст несёт только сам `Body`.
            if object.get("kind").and_then(Value::as_str) == Some("Body") {
                if self.authority.snapshot(source_at)? != snapshot {
                    return Err(ViewError::new(
                        RefusalCode::ConcurrentChange,
                        "source changed while the source text was read; retry the question",
                    ));
                }
                let (text, from, to) = match text {
                    SourceText::Window { text, from, to } => (*text, from, to),
                    SourceText::Whole(text) => {
                        let to = text.lines().count();
                        (SourceText::Whole(text), 1, to)
                    }
                    SourceText::Oversized { bytes, prefix } => {
                        (SourceText::Oversized { bytes, prefix }, 1, 0)
                    }
                };
                if canonical_at.line_range().is_some() {
                    object.insert(
                        "props".into(),
                        json!({"lines": {"state": "range", "from": from, "to": to}}),
                    );
                }
                let mut result = body_text_result(&canonical_at, object, text_revision.take());
                match text {
                    SourceText::Whole(text) => {
                        result.source_text = Some(text);
                    }
                    SourceText::Oversized { bytes, prefix } => {
                        let error = oversized_body(
                            &canonical_at.to_string(),
                            bytes,
                            unica_bootstrap::MAX_RESPONSE_BUDGET,
                        );
                        let data = result.data.take();
                        let rev = result.rev.take();
                        result = DomainResult::canonical_rejection(
                            Some(canonical_at.to_string()),
                            RefusalCode::ResultTooLarge,
                            error.to_string(),
                        );
                        result.data = data;
                        result.rev = rev;
                        result.next.extend(error.next().iter().cloned());
                        result.source_text_read_hint = Some(Box::new(
                            unica_execution_contract::invocation::SourceTextReadHint {
                                from_line: from,
                                to_line: to,
                                prefix: Some(prefix),
                            },
                        ));
                    }
                    SourceText::Window { .. } => unreachable!("a Body has one coordinate window"),
                }
                if result.source_text_read_hint.is_none() && to >= from {
                    result.source_text_read_hint = Some(Box::new(
                        unica_execution_contract::invocation::SourceTextReadHint {
                            from_line: from,
                            to_line: to,
                            prefix: None,
                        },
                    ));
                }
                if serialized_result_size(&result) > MAX_CANONICAL_RESULT_BYTES {
                    return Err(ViewError::new(
                        RefusalCode::ResultTooLarge,
                        "logical node exceeds the transport result limit",
                    ));
                }
                return Ok(result);
            }
        }
        let Some(items) = object.remove("items") else {
            return node_result(&canonical_at, object, snapshot.revision);
        };
        let items = items.as_array().cloned().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "typed collection items are not an array",
            )
        })?;
        let node = Value::Object(object);
        preflight_collection(&node, &items, &binding)?;
        let current = self.authority.snapshot(source_at)?;
        if current != snapshot {
            return Err(ViewError::new(
                RefusalCode::ConcurrentChange,
                "source changed while the view collection was read; retry the question",
            ));
        }
        self.page_result(node, items, request, binding)
    }

    fn page_result(
        &self,
        node: Value,
        items: Vec<Value>,
        request: &ViewRequest,
        binding: ViewCursorBinding,
    ) -> Result<DomainResult, ViewError> {
        let first = prepare_page(&node, &items, 0, request.limit, &binding)?;
        let cursor = if first.next_offset == items.len() {
            None
        } else {
            Some(
                self.cursors
                    .insert_snapshot(binding.clone(), node.clone(), items, first.next_offset)
                    .map_err(cursor_error)?,
            )
        };
        collection_result(&node, first.items, cursor, first.stopped_by, &binding)
    }
}

/// `Body` модуля из его текста. Текст сверх предела отказывает лишь там,
/// где отдаётся сам: контракт `.Can.<op>` того же узла его не касается.
fn module_body_view(
    body_at: &QualifiedAddress,
    text: ModuleText,
) -> Result<NodeViewData, ViewError> {
    let (text, count) = match text {
        ModuleText::TooLarge {
            bytes,
            prefix,
            lines,
        } => (SourceText::Oversized { bytes, prefix }, lines),
        ModuleText::Text(text) => {
            let lines = text.lines().count();
            (SourceText::Whole(text), lines)
        }
    };
    let (from, to) = body_at
        .line_range()
        .map_or((1, count), |range| (range.from, range.to));
    let text = SourceText::Window {
        text: Box::new(text),
        from,
        to,
    };
    Ok(NodeViewData::Text(TextView::new(
        NodeView::new(body_at.to_string(), "Body", "Body", Map::new()),
        text,
    )))
}

fn collection_result(
    node: &Value,
    items: Vec<Value>,
    cursor: Option<String>,
    stopped_by: &'static str,
    binding: &ViewCursorBinding,
) -> Result<DomainResult, ViewError> {
    let mut page = node.as_object().cloned().ok_or_else(|| {
        ViewError::new(
            RefusalCode::ProviderUnavailable,
            "stored node projection is not an object",
        )
    })?;
    page.insert("items".to_string(), Value::Array(items));
    let mut result = DomainResult::success("logical collection page resolved");
    result.at = Some(binding.canonical_at.clone());
    result.data = Some(Value::Object(page));
    result.rev = Some(binding.snapshot_id.clone());
    result.cursor = cursor;
    result.page = Some(serde_json::json!({"stoppedBy": stopped_by}));
    Ok(result)
}

fn node_result(
    at: &QualifiedAddress,
    node: Map<String, Value>,
    revision: String,
) -> Result<DomainResult, ViewError> {
    let mut result = DomainResult::success("logical node resolved");
    result.at = Some(at.to_string());
    result.data = Some(Value::Object(node));
    result.rev = Some(revision);
    if serialized_result_size(&result) > MAX_CANONICAL_RESULT_BYTES {
        return Err(ViewError::new(
            RefusalCode::ResultTooLarge,
            "logical node exceeds the transport result limit",
        ));
    }
    Ok(result)
}

struct PreparedViewPage {
    items: Vec<Value>,
    next_offset: usize,
    stopped_by: &'static str,
}

fn preflight_collection(
    node: &Value,
    items: &[Value],
    binding: &ViewCursorBinding,
) -> Result<(), ViewError> {
    let empty = collection_result(node, Vec::new(), None, "complete", binding)?;
    if serialized_result_size(&empty) > MAX_CANONICAL_RESULT_BYTES {
        return Err(ViewError::new(
            RefusalCode::ResultTooLarge,
            "logical collection metadata exceeds the transport result limit",
        ));
    }
    if items.is_empty() {
        return Ok(());
    }
    let with_cursor = collection_result(
        node,
        vec![Value::Null],
        Some(ViewCursorStore::token_probe()),
        "complete",
        binding,
    )?;
    let terminal = collection_result(node, vec![Value::Null], None, "complete", binding)?;
    let with_cursor_base = serialized_result_size(&with_cursor) - 4;
    let terminal_base = serialized_result_size(&terminal) - 4;
    // A continuation is issued only if every indivisible item can later fit
    // inside the exact public result envelope, including its cursor.
    for (index, item) in items.iter().enumerate() {
        let base = if index + 1 < items.len() {
            with_cursor_base
        } else {
            terminal_base
        };
        if base.saturating_add(serialized_value_size(item)) > MAX_CANONICAL_RESULT_BYTES {
            return Err(ViewError::new(
                RefusalCode::ResultTooLarge,
                "one collection item exceeds the transport result limit",
            ));
        }
    }
    Ok(())
}

struct SizeWriter(usize);

impl Write for SizeWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_value_size(value: &Value) -> usize {
    let mut writer = SizeWriter(0);
    serde_json::to_writer(&mut writer, value).expect("a JSON value serializes");
    writer.0
}

fn serialized_result_size(result: &DomainResult) -> usize {
    serde_json::to_vec(result)
        .expect("a domain result containing JSON values serializes")
        .len()
}

fn prepare_page(
    node: &Value,
    items: &[Value],
    mut offset: usize,
    limit: usize,
    binding: &ViewCursorBinding,
) -> Result<PreparedViewPage, ViewError> {
    if offset > items.len() {
        return Err(ViewError::new(
            RefusalCode::InvalidCursor,
            "view cursor offset is invalid",
        ));
    }
    let base = collection_result(node, Vec::new(), None, "complete", binding)?;
    let base_bytes = serialized_result_size(&base);
    if base_bytes > MAX_CANONICAL_RESULT_BYTES {
        return Err(ViewError::new(
            RefusalCode::ResultTooLarge,
            "logical collection metadata exceeds the transport result limit",
        ));
    }
    // Metadata alone may exceed the preferred page size. In that case an
    // element cap and the transport limit still apply, without forcing every
    // otherwise readable element onto its own page.
    let prefer_page_bytes = base_bytes <= PREFERRED_PAGE_BYTES;
    let mut page_items = Vec::new();
    let stopped_by = loop {
        if offset == items.len() {
            break "complete";
        }
        if page_items.len() == limit {
            break "limit";
        }
        let mut candidate = page_items.clone();
        candidate.push(items[offset].clone());
        let probe = collection_result(
            node,
            candidate.clone(),
            (offset + 1 < items.len()).then(ViewCursorStore::token_probe),
            "complete",
            binding,
        )?;
        let bytes = serialized_result_size(&probe);
        if bytes > MAX_CANONICAL_RESULT_BYTES {
            if page_items.is_empty() {
                return Err(ViewError::new(
                    RefusalCode::ResultTooLarge,
                    "one collection item exceeds the transport result limit",
                ));
            }
            break "bytes";
        }
        if prefer_page_bytes && bytes > PREFERRED_PAGE_BYTES && !page_items.is_empty() {
            break "bytes";
        }
        page_items = candidate;
        offset += 1;
    };
    let page_probe = collection_result(
        node,
        page_items.clone(),
        (offset < items.len()).then(ViewCursorStore::token_probe),
        stopped_by,
        binding,
    )?;
    if serialized_result_size(&page_probe) > MAX_CANONICAL_RESULT_BYTES {
        return Err(ViewError::new(
            RefusalCode::ResultTooLarge,
            "logical collection page exceeds the transport result limit",
        ));
    }
    Ok(PreparedViewPage {
        items: page_items,
        next_offset: offset,
        stopped_by,
    })
}

fn cursor_error(error: ViewCursorError) -> ViewError {
    match error.detail() {
        Some(detail) => ViewError::detailed(detail, error.message()),
        None => ViewError::new(error.code(), error.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::{ViewError, ViewReadAuthority, ViewRequest, ViewService, ViewSourceSnapshot};
    use crate::application::v13::source_text::{ModuleText, MAX_SOURCE_TEXT_BYTES};
    use crate::domain::node_view::{CollectionView, NodeView, NodeViewData};
    use unica_navigation::result_store::ViewCursorStore;

    use serde_json::{json, Map};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use unica_address::address::QualifiedAddress;

    struct FixtureAuthority {
        revisions: Mutex<HashMap<String, ViewSourceSnapshot>>,
        views: HashMap<String, NodeViewData>,
        module_text: Option<ModuleText>,
        revision_change_on_snapshot: Option<usize>,
        snapshot_calls: AtomicUsize,
    }

    impl FixtureAuthority {
        fn new() -> Self {
            let module = "main:Document.Заказ.Module.Object";
            let body = format!("{module}.Body");
            Self {
                revisions: Mutex::new(HashMap::from([
                    (
                        module.to_string(),
                        ViewSourceSnapshot {
                            source_set_identity: "main:source-id".to_string(),
                            revision: "rev-1".to_string(),
                        },
                    ),
                    (
                        body.clone(),
                        ViewSourceSnapshot {
                            source_set_identity: "main:source-id".to_string(),
                            revision: "rev-1".to_string(),
                        },
                    ),
                ])),
                views: HashMap::from([
                    (
                        module.to_string(),
                        NodeViewData::Node(NodeView::new(
                            module,
                            "Module",
                            "Модуль объекта Заказ",
                            Map::new(),
                        )),
                    ),
                    (
                        body.clone(),
                        NodeViewData::Collection(CollectionView::new(
                            NodeView::new(&body, "Body", "Тело модуля объекта Заказ", Map::new()),
                            vec![
                                json!({"line": 1, "text": "Первая"}),
                                json!({"line": 2, "text": "Вторая"}),
                                json!({"line": 3, "text": "Третья"}),
                            ],
                        )),
                    ),
                ]),
                module_text: None,
                revision_change_on_snapshot: None,
                snapshot_calls: AtomicUsize::new(0),
            }
        }

        fn change_revision(&self, at: &str) {
            self.revisions.lock().unwrap().get_mut(at).unwrap().revision = "rev-2".to_string();
        }
    }

    impl ViewReadAuthority for FixtureAuthority {
        fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            let call = self.snapshot_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let mut snapshot = self
                .revisions
                .lock()
                .unwrap()
                .get(&at.to_string())
                .cloned()
                .ok_or_else(|| {
                    ViewError::new(
                        unica_refusal::RefusalCode::NotFound,
                        "fixture address was not found",
                    )
                })?;
            if self
                .revision_change_on_snapshot
                .is_some_and(|at| call >= at)
            {
                snapshot.revision = "rev-2".to_string();
            }
            Ok(snapshot)
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            self.views.get(&at.to_string()).cloned().ok_or_else(|| {
                ViewError::new(
                    unica_refusal::RefusalCode::NotFound,
                    "fixture address was not found",
                )
            })
        }

        fn read_module_text(
            &self,
            _at: &QualifiedAddress,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<Option<ModuleText>, ViewError> {
            Ok(self.module_text.clone())
        }
    }

    #[test]
    fn view_request_is_selected_only_by_address_and_page_limit() {
        let request = ViewRequest::new("main:Document.Заказ.Module.Object.Method").unwrap();
        assert_eq!(request.at(), "main:Document.Заказ.Module.Object.Method");
        assert_eq!(request.limit(), 20);
        assert_eq!(
            ViewRequest::new("main:Document.Заказ.Module.Object.Method")
                .unwrap()
                .with_limit(51)
                .unwrap_err()
                .code()
                .as_str(),
            "bad_value",
        );
    }

    #[test]
    fn view_keeps_content_behind_explicit_branches_and_paginates_collections() {
        let service = ViewService::new(FixtureAuthority::new(), ViewCursorStore::default());
        let summary = service.view(ViewRequest::new("main:Document.Заказ.Module.Object").unwrap());
        assert!(summary.ok);
        let data = summary.data.unwrap();
        assert!(data.get("items").is_none());
        assert!(!data.to_string().contains("Первая"));

        let first = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(2)
                .unwrap(),
        );
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(first.data.as_ref().unwrap()["items"][1]["line"], 2);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "limit");
        let cursor = first.cursor.unwrap();
        assert!(cursor.starts_with("vc2."));
        assert!(cursor[4..].parse::<usize>().is_err());

        let second = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(2)
                .unwrap()
                .with_cursor(cursor),
        );
        assert_eq!(
            second.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(second.data.as_ref().unwrap()["items"][0]["line"], 3);
        assert!(second.cursor.is_none());
        assert_eq!(second.page.as_ref().unwrap()["stoppedBy"], "complete");

        let methods = "main:Document.Заказ.Module.Object.Method";
        let mut authority = FixtureAuthority::new();
        authority.revisions.get_mut().unwrap().insert(
            methods.to_string(),
            ViewSourceSnapshot {
                source_set_identity: "main:source-id".to_string(),
                revision: "rev-1".to_string(),
            },
        );
        authority.views.insert(
            methods.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(methods, "Method", "Методы модуля", Map::new()),
                vec![json!({"name": "Первая"}), json!({"name": "Вторая"})],
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let request = || ViewRequest::new(methods).unwrap().with_limit(1).unwrap();
        let first = service.view(request());
        assert_eq!(
            first.data.as_ref().unwrap()["items"],
            json!([{"name": "Первая"}])
        );
        let second = service.view(request().with_cursor(first.cursor.unwrap()));
        assert_eq!(
            second.data.as_ref().unwrap()["items"],
            json!([{"name": "Вторая"}])
        );
        assert!(second.cursor.is_none());
    }

    #[test]
    fn supplied_graph_collection_uses_stable_pages_bound_to_its_owner() {
        let service = ViewService::new(FixtureAuthority::new(), ViewCursorStore::default());
        let owner = QualifiedAddress::parse("main:Document.Заказ.Module.Object").unwrap();
        let branch = "main:Document.Заказ.Module.Object.Method.Проба.Caller";
        let items: Vec<_> = (0..120)
            .map(|n| json!({"at":format!("main:CommonModule.Узел{n}"),"kind":"Method"}))
            .collect();
        let first = service.view_preloaded_collection(
            ViewRequest::new(branch).unwrap(),
            &owner,
            "rev-1",
            Some(json!({"at":branch,"kind":"Caller","title":"Caller","props":{"callGraph":"ready"},"items":items})),
        );
        assert!(first.ok);
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "limit");
        let cursor = first.cursor.unwrap();
        let second = service.view_preloaded_collection(
            ViewRequest::new(branch)
                .unwrap()
                .with_cursor(cursor.clone()),
            &owner,
            "rev-1",
            None,
        );
        assert!(second.ok);
        assert_eq!(
            second.data.as_ref().unwrap()["items"][0]["at"],
            "main:CommonModule.Узел20"
        );
        assert_ne!(second.cursor, Some(cursor.clone()));

        service.authority.change_revision(&owner.to_string());
        let stale = service.view_preloaded_collection(
            ViewRequest::new(branch).unwrap().with_cursor(cursor),
            &owner,
            "rev-1",
            None,
        );
        assert!(stale.ok);
        assert_eq!(stale, second);
    }

    #[test]
    fn cursor_replay_keeps_the_issued_snapshot_after_source_change() {
        let authority = FixtureAuthority::new();
        let service = ViewService::new(authority, ViewCursorStore::default());
        let first = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(1)
                .unwrap(),
        );
        let cursor = first.cursor.unwrap();
        service
            .authority
            .change_revision("main:Document.Заказ.Module.Object.Body");
        let stale = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(1)
                .unwrap()
                .with_cursor(cursor),
        );
        assert!(stale.ok, "{stale:?}");
        assert_eq!(stale.data.as_ref().unwrap()["items"][0]["line"], 2);

        let invalid = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_cursor("vc1.00000000000000000000000000000000".to_string()),
        );
        assert_eq!(invalid.diagnostics[0]["code"], "invalid_cursor");
    }

    #[test]
    fn retrying_the_same_cursor_returns_the_same_page_and_successor() {
        let service = ViewService::new(FixtureAuthority::new(), ViewCursorStore::default());
        let first = service.view(
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(1)
                .unwrap(),
        );
        let cursor = first.cursor.unwrap();
        let request = || {
            ViewRequest::new("main:Document.Заказ.Module.Object.Body")
                .unwrap()
                .with_limit(1)
                .unwrap()
                .with_cursor(cursor.clone())
        };
        let page = service.view(request());
        let replay = service.view(request());
        assert_eq!(replay, page);
        assert!(page.cursor.is_some());
    }

    #[test]
    fn a_collection_longer_than_the_cursor_entry_limit_still_has_a_first_page() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                (0..2_601).map(|line| json!({"line": line})).collect(),
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let request = || ViewRequest::new(at).unwrap().with_limit(20).unwrap();
        let mut page = service.view(request());
        assert!(page.ok);
        let mut lines = Vec::new();
        loop {
            lines.extend(
                page.data.as_ref().unwrap()["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item["line"].as_u64().unwrap()),
            );
            let Some(cursor) = page.cursor.clone() else {
                break;
            };
            let next = service.view(request().with_cursor(cursor.clone()));
            assert_eq!(next, service.view(request().with_cursor(cursor)));
            page = next;
        }
        assert_eq!(lines, (0..2_601).collect::<Vec<_>>());
        assert_eq!(page.page.as_ref().unwrap()["stoppedBy"], "complete");
    }

    #[test]
    fn addressed_collection_prefers_64_kib_pages_and_replays() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![
                    json!({"line": 1, "text": "А".repeat(20_000)}),
                    json!({"line": 2, "text": "Б".repeat(20_000)}),
                    json!({"line": 3, "text": "В".repeat(20_000)}),
                ],
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let request = || ViewRequest::new(at).unwrap().with_limit(50).unwrap();
        let first = service.view(request());
        assert!(first.ok);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(serde_json::to_vec(&first).unwrap().len() <= super::PREFERRED_PAGE_BYTES);
        let cursor = first.cursor.clone().unwrap();
        let second = service.view(request().with_cursor(cursor.clone()));
        assert_eq!(second, service.view(request().with_cursor(cursor)));
        assert_eq!(second.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert!(serde_json::to_vec(&second).unwrap().len() <= super::PREFERRED_PAGE_BYTES);
        let last = service.view(request().with_cursor(second.cursor.unwrap()));
        assert_eq!(last.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(last.cursor.is_none());
        assert!(serde_json::to_vec(&last).unwrap().len() <= super::PREFERRED_PAGE_BYTES);
    }

    #[test]
    fn projected_section_is_counted_before_publishing_a_replayable_cursor() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![
                    json!({"line": 1, "text": "А".repeat(15_000)}),
                    json!({"line": 2, "text": "Б".repeat(15_000)}),
                ],
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let project = |data: &serde_json::Value| -> Result<serde_json::Value, ViewError> {
            let mut projected = data.clone();
            projected["can"] = json!(["operation".repeat(1_000)]);
            Ok(projected)
        };
        let request = || ViewRequest::new(at).unwrap().with_limit(50).unwrap();
        let first = service.view_projected(request(), &project);
        assert!(first.ok);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(serde_json::to_vec(&first).unwrap().len() <= super::PREFERRED_PAGE_BYTES);
        let cursor = first.cursor.unwrap();
        let next = service.view_projected(request().with_cursor(cursor.clone()), &project);
        assert_eq!(
            next,
            service.view_projected(request().with_cursor(cursor), &project)
        );
        assert_eq!(next.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(next.cursor.is_none());
        assert!(serde_json::to_vec(&next).unwrap().len() <= super::PREFERRED_PAGE_BYTES);
    }

    #[test]
    fn projection_without_items_returns_one_node_without_a_spurious_cursor() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let service = ViewService::new(FixtureAuthority::new(), ViewCursorStore::default());
        let result = service.view_projected(ViewRequest::new(at).unwrap(), &|data| {
            let mut projected = data.clone();
            projected.as_object_mut().unwrap().remove("items");
            Ok(projected)
        });
        assert!(result.ok);
        assert!(result.data.as_ref().unwrap().get("items").is_none());
        assert!(result.cursor.is_none());
        assert!(result.page.is_none());
    }

    #[test]
    fn module_text_is_returned_whole_beside_the_projected_node_without_pages() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let text = "// Комментарий\r\nProcedure A()\n\tReturn;\nEndProcedure\n";
        let mut authority = FixtureAuthority::new();
        authority.module_text = Some(ModuleText::Text(text.into()));
        let service = ViewService::new(authority, ViewCursorStore::default());
        let request = ViewRequest::new(at).unwrap().with_limit(1).unwrap();
        let result = service.view_projected(request, &|data| {
            let mut projected = data.clone();
            projected["can"] = json!([{"op": "code.replace"}]);
            Ok(projected)
        });
        assert!(result.ok, "the Body read must succeed");
        assert_eq!(result.source_text.as_deref(), Some(text));
        assert_eq!(
            result.data.unwrap(),
            json!({"at": at, "kind": "Body", "title": "Body", "can": [{"op": "code.replace"}]})
        );
        assert!(result.cursor.is_none());
        assert!(result.page.is_none());
    }

    #[test]
    fn oversized_module_text_does_not_refuse_a_projection_that_returns_no_text() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.module_text = Some(ModuleText::TooLarge {
            bytes: MAX_SOURCE_TEXT_BYTES as u64 + 1,
            prefix: String::new(),
            lines: 1,
        });
        let result = ViewService::new(authority, ViewCursorStore::default()).view_projected(
            ViewRequest::new(at).unwrap(),
            &|data| {
                let mut contract = data.clone();
                contract["kind"] = json!("Can");
                Ok(contract)
            },
        );
        assert!(result.ok, "the Body read must succeed");
        assert!(result.source_text.is_none());
        assert_eq!(result.data.unwrap()["kind"], "Can");
    }

    #[test]
    fn module_text_rechecks_revision_after_the_read() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.module_text = Some(ModuleText::Text("Procedure A()\n".into()));
        authority.revision_change_on_snapshot = Some(2);
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "concurrent_change");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn module_text_over_the_limit_is_refused_whole_with_structural_next() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.module_text = Some(ModuleText::TooLarge {
            bytes: MAX_SOURCE_TEXT_BYTES as u64 + 1,
            prefix: String::new(),
            lines: 1,
        });
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert_eq!(result.data.as_ref().unwrap()["kind"], "Body");
        assert!(result.source_text_read_hint.is_some());
        assert!(result.cursor.is_none());
        let targets = result
            .next
            .iter()
            .map(|next| next["args"]["at"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            [
                "main:Document.Заказ.Module.Object.Region",
                "main:Document.Заказ.Module.Object.Method"
            ]
        );
    }

    #[test]
    fn one_item_over_the_page_target_is_returned_whole() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![json!({"line": 1, "text": "А".repeat(70_000)})],
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok);
        assert_eq!(
            result.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(result.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn indivisible_large_items_keep_a_replayable_continuation() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![
                    json!({"line": 1, "text": "А".repeat(70_000)}),
                    json!({"line": 2, "text": "Б".repeat(70_000)}),
                ],
            )),
        );
        let service = ViewService::new(authority, ViewCursorStore::default());
        let first = service.view(ViewRequest::new(at).unwrap());
        assert!(first.ok);
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(serde_json::to_vec(&first).unwrap().len() > super::PREFERRED_PAGE_BYTES);
        let cursor = first.cursor.unwrap();
        let request = || ViewRequest::new(at).unwrap().with_cursor(cursor.clone());
        let last = service.view(request());
        assert_eq!(last, service.view(request()));
        assert!(last.ok);
        assert_eq!(last.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert_eq!(
            last.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(last.cursor.is_none());
    }

    #[test]
    fn empty_collection_with_metadata_over_the_page_target_is_returned_whole() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        let mut props = Map::new();
        props.insert("description".to_string(), json!("А".repeat(70_000)));
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", props),
                vec![],
            )),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(result.ok);
        assert_eq!(result.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn metadata_over_the_page_target_does_not_force_one_item_per_page() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        let mut props = Map::new();
        props.insert("description".to_string(), json!("А".repeat(70_000)));
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", props),
                (0..20).map(|line| json!({"line": line})).collect(),
            )),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(result.ok);
        assert_eq!(
            result.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            20
        );
        assert_eq!(result.page.as_ref().unwrap()["stoppedBy"], "complete");
    }

    #[test]
    fn item_over_the_transport_limit_is_refused_without_a_cursor() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![json!({"text": "X".repeat(unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES)})],
            )),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn node_over_the_transport_limit_is_refused_explicitly() {
        let at = "main:Document.Заказ.Module.Object";
        let mut authority = FixtureAuthority::new();
        let mut props = Map::new();
        props.insert(
            "description".to_string(),
            json!("X".repeat(unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES)),
        );
        authority.views.insert(
            at.to_string(),
            NodeViewData::Node(NodeView::new(at, "Module", "Модуль объекта", props)),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn a_final_page_uses_its_actual_size_without_a_placeholder_cursor() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let read = |description_len| {
            let mut authority = FixtureAuthority::new();
            let mut props = Map::new();
            props.insert(
                "description".to_string(),
                json!("X".repeat(description_len)),
            );
            authority.views.insert(
                at.to_string(),
                NodeViewData::Collection(CollectionView::new(
                    NodeView::new(at, "Body", "Тело модуля", props),
                    vec![json!(1)],
                )),
            );
            ViewService::new(authority, ViewCursorStore::default())
                .view(ViewRequest::new(at).unwrap())
        };
        let baseline = read(0);
        assert!(baseline.ok);
        let exact_size = unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;
        let result = read(exact_size - serde_json::to_vec(&baseline).unwrap().len());
        assert!(result.ok);
        assert_eq!(serde_json::to_vec(&result).unwrap().len(), exact_size);
        assert_eq!(result.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn collection_exceeding_cursor_entry_count_still_has_usable_pages() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                (0..100).map(|line| json!({"line": line})).collect(),
            )),
        );
        let store = ViewCursorStore::new(1024 * 1024);
        let service = ViewService::new(authority, store);
        let request = || ViewRequest::new(at).unwrap().with_limit(1).unwrap();
        let mut page = service.view(request());
        let mut seen = Vec::new();
        loop {
            assert!(page.ok);
            seen.push(
                page.data.as_ref().unwrap()["items"][0]["line"]
                    .as_u64()
                    .unwrap(),
            );
            let Some(cursor) = page.cursor.take() else {
                break;
            };
            page = service.view(request().with_cursor(cursor));
        }
        assert_eq!(seen, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn snapshot_over_byte_quota_refuses_before_promising_a_continuation() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                (0..100).map(|line| json!({"line": line})).collect(),
            )),
        );
        let store = ViewCursorStore::new(512);
        let result = ViewService::new(authority, store)
            .view(ViewRequest::new(at).unwrap().with_limit(1).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn a_late_oversized_item_refuses_before_the_first_page() {
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                vec![json!({"line":1}), json!({"line":2,"text":"X".repeat(unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES)})],
            )),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(at).unwrap().with_limit(1).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn concurrent_cursor_retries_return_the_same_page_and_successor() {
        use std::sync::Arc;
        let at = "main:Document.Заказ.Module.Object.Body";
        let service = Arc::new(ViewService::new(
            FixtureAuthority::new(),
            ViewCursorStore::default(),
        ));
        let first = service.view(ViewRequest::new(at).unwrap().with_limit(1).unwrap());
        let cursor = first.cursor.unwrap();
        let handles = (0..8)
            .map(|_| {
                let service = Arc::clone(&service);
                let cursor = cursor.clone();
                std::thread::spawn(move || {
                    service.view(
                        ViewRequest::new(at)
                            .unwrap()
                            .with_limit(1)
                            .unwrap()
                            .with_cursor(cursor),
                    )
                })
            })
            .collect::<Vec<_>>();
        let pages = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(pages[0].ok);
        assert!(pages[0].cursor.is_some());
        assert!(pages.iter().all(|page| page == &pages[0]));
    }

    /// Уточнение обязано пережить дорогу от чтения до провода. Разбирать
    /// отказ на код и текст по дороге нельзя: тогда `detailCode` исчезает
    /// молча, а код возвращается к своему умолчанию — и различие, ради
    /// которого уточнение заведено, теряется.
    #[test]
    fn a_detailed_read_refusal_keeps_its_detail_and_overridden_outcome_on_the_wire() {
        use unica_refusal::{Outcome, RefusalCode, RefusalDetail};

        struct UnreadableSource;

        impl ViewReadAuthority for UnreadableSource {
            fn snapshot(&self, _at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
                Err(ViewError::detailed(
                    RefusalDetail::SourceUnreadable,
                    "Configuration.xml is not UTF-8",
                ))
            }

            fn read_exact(
                &self,
                _at: &QualifiedAddress,
                _admitted: &ViewSourceSnapshot,
            ) -> Result<NodeViewData, ViewError> {
                unreachable!("чтение не начинается, пока снимок не взят")
            }
        }

        let service = ViewService::new(UnreadableSource, ViewCursorStore::default());
        let request = ViewRequest::new("main:Catalog.Валюты").expect("адрес разбирается");
        let result = service.view(request);

        assert!(!result.ok, "{result:?}");
        let diagnostic = &result.diagnostics[0];
        assert_eq!(diagnostic["code"], "provider_unavailable");
        assert_eq!(diagnostic["detailCode"], "source_unreadable");
        assert_eq!(
            diagnostic["outcome"], "fixSource",
            "уточнение перекрывает умолчание кода: {result:?}"
        );
        assert_ne!(
            RefusalCode::ProviderUnavailable.outcome(),
            Outcome::FixSource,
            "проверка пуста, если умолчание кода совпадает с исходом уточнения"
        );
    }

    #[test]
    fn actual_cursor_size_is_reserved_for_late_items_and_metadata() {
        use unica_execution_contract::result_size::MAX_CANONICAL_RESULT_BYTES;
        use unica_navigation::result_store::ViewCursorBinding;
        let at = "main:Document.Заказ.Module.Object.Body";
        let binding = ViewCursorBinding {
            canonical_at: at.into(),
            projection: "Body".into(),
            source_set_identity: "main:source-id".into(),
            snapshot_id: "rev-1".into(),
            page_limit: 1,
        };
        let node = json!({"at": at, "kind": "Body", "title": ""});
        let probe = super::collection_result(
            &node,
            vec![json!("")],
            Some(ViewCursorStore::token_probe()),
            "complete",
            &binding,
        )
        .unwrap();
        let padding = MAX_CANONICAL_RESULT_BYTES - super::serialized_result_size(&probe);
        let items = vec![json!("first"), json!("X".repeat(padding)), json!("last")];
        super::preflight_collection(&node, &items, &binding).unwrap();
        let page = super::prepare_page(&node, &items, 1, 1, &binding).unwrap();
        let store = ViewCursorStore::default();
        let token = store
            .insert_snapshot(binding.clone(), node.clone(), items.clone(), 2)
            .unwrap();
        let encoded =
            super::collection_result(&node, page.items, Some(token), page.stopped_by, &binding)
                .unwrap();
        assert!(super::serialized_result_size(&encoded) <= MAX_CANONICAL_RESULT_BYTES);
        let mut over = items;
        over[1] = json!("X".repeat(padding + 1));
        assert_eq!(
            super::preflight_collection(&node, &over, &binding)
                .unwrap_err()
                .code(),
            unica_refusal::RefusalCode::ResultTooLarge
        );

        let base = super::collection_result(
            &node,
            vec![serde_json::Value::Null],
            Some(ViewCursorStore::token_probe()),
            "complete",
            &binding,
        )
        .unwrap();
        let mut metadata = node;
        metadata["title"] =
            json!("M".repeat(MAX_CANONICAL_RESULT_BYTES - super::serialized_result_size(&base)));
        let items = vec![serde_json::Value::Null, serde_json::Value::Null];
        super::preflight_collection(&metadata, &items, &binding).unwrap();
        let page = super::prepare_page(&metadata, &items, 0, 1, &binding).unwrap();
        let token = store
            .insert_snapshot(binding.clone(), metadata.clone(), items.clone(), 1)
            .unwrap();
        let encoded = super::collection_result(
            &metadata,
            page.items,
            Some(token),
            page.stopped_by,
            &binding,
        )
        .unwrap();
        assert!(super::serialized_result_size(&encoded) <= MAX_CANONICAL_RESULT_BYTES);
        let title = metadata["title"].as_str().unwrap().to_owned() + "!";
        metadata["title"] = json!(title);
        assert!(super::preflight_collection(&metadata, &items, &binding).is_err());
    }
}
