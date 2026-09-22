use crate::application::result_store::{
    ViewCursorBinding, ViewCursorError, ViewCursorStore, ViewStoredPage,
};
use crate::domain::address::QualifiedAddress;
use crate::domain::invocation::DomainResult;
use crate::domain::node_view::NodeViewData;
use crate::domain::refusal::{RefusalCode, RefusalDetail};
use serde_json::{Map, Value};
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 50;
pub(crate) const PAGE_BYTES: usize = 64 * 1024;
const CURSOR_SIZE_PLACEHOLDER: &str = "vc1.00000000000000000000000000000000";
type ViewProjector<'a> = dyn Fn(&Value) -> Result<Value, ViewError> + 'a;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewFilter(Map<String, Value>);

impl Default for ViewFilter {
    fn default() -> Self {
        Self(Map::new())
    }
}

impl ViewFilter {
    pub(crate) fn new(values: Map<String, Value>) -> Self {
        Self(values)
    }

    pub(crate) fn get(&self, name: &str) -> Option<&Value> {
        self.0.get(name)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.0.iter()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn normalized(&self) -> String {
        serde_json::to_string(&canonical_value(Value::Object(self.0.clone())))
            .expect("a JSON filter always serializes")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ViewRequest {
    at: QualifiedAddress,
    filter: ViewFilter,
    limit: usize,
    cursor: Option<String>,
}

impl ViewRequest {
    pub(crate) fn new(at: &str) -> Result<Self, ViewError> {
        let at = QualifiedAddress::parse(at)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?;
        Ok(Self {
            at,
            filter: ViewFilter::default(),
            limit: DEFAULT_LIMIT,
            cursor: None,
        })
    }

    pub(crate) fn at(&self) -> String {
        self.at.to_string()
    }

    pub(crate) const fn filter(&self) -> &ViewFilter {
        &self.filter
    }

    pub(crate) const fn limit(&self) -> usize {
        self.limit
    }

    pub(crate) fn with_filter(mut self, filter: Map<String, Value>) -> Self {
        self.filter = ViewFilter::new(filter);
        self
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

    fn binding(
        &self,
        canonical_at: &QualifiedAddress,
        snapshot: &ViewSourceSnapshot,
    ) -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: canonical_at.to_string(),
            projection: canonical_at
                .segments()
                .last()
                .map(|segment| segment.kind().as_str())
                .unwrap_or("Configuration")
                .to_string(),
            normalized_filter: self.filter.normalized(),
            source_set_identity: snapshot.source_set_identity.clone(),
            source_revision: snapshot.revision.clone(),
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
        filter: &ViewFilter,
        admitted: &ViewSourceSnapshot,
    ) -> Result<NodeViewData, ViewError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ViewError {
    code: RefusalCode,
    detail: Option<RefusalDetail>,
    message: String,
    next: Option<Value>,
}

impl ViewError {
    pub(crate) fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            detail: None,
            message: message.into(),
            next: None,
        }
    }

    /// Отказ с уточнением: код берётся из уточнения, поэтому пару «код и не
    /// его уточнение» составить нельзя.
    pub(crate) fn detailed(detail: RefusalDetail, message: impl Into<String>) -> Self {
        Self {
            code: detail.code(),
            detail: Some(detail),
            message: message.into(),
            next: None,
        }
    }

    pub(crate) const fn code(&self) -> RefusalCode {
        self.code
    }

    pub(crate) const fn detail(&self) -> Option<RefusalDetail> {
        self.detail
    }

    /// Маршрут к следующему вопросу. Отказ, который знает альтернативу, обязан
    /// её назвать — иначе агент останавливается там, где путь есть.
    pub(crate) fn set_next(&mut self, next: Value) {
        self.next = Some(next);
    }

    pub(crate) fn next(&self) -> Option<&Value> {
        self.next.as_ref()
    }
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) struct ViewService<A> {
    authority: A,
    cursors: Arc<ViewCursorStore>,
}

impl<A: ViewReadAuthority> ViewService<A> {
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
        match self.try_view(&request, None) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    pub(crate) fn view_projected(
        &self,
        request: ViewRequest,
        project: &ViewProjector<'_>,
    ) -> DomainResult {
        match self.try_view(&request, Some(project)) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    fn try_view(
        &self,
        request: &ViewRequest,
        project: Option<&ViewProjector<'_>>,
    ) -> Result<DomainResult, ViewError> {
        let snapshot = self.authority.snapshot(&request.at)?;
        let canonical_at = self.authority.canonical_address(&request.at, &snapshot)?;
        let binding = request.binding(&canonical_at, &snapshot);
        if let Some(cursor) = request.cursor.as_deref() {
            let stored = self
                .cursors
                .read(cursor, &binding, &snapshot.revision)
                .map_err(cursor_error)?;
            return self.stored_page_result(
                stored.node,
                stored.items,
                stored.next_cursor,
                stored.stopped_by,
                request,
                stored.binding,
            );
        }

        let view = self
            .authority
            .read_exact(&canonical_at, &request.filter, &snapshot)?;
        if view.at() != canonical_at.to_string() {
            return Err(ViewError::new(
                RefusalCode::ProviderUnavailable,
                "typed reader returned a projection for another logical address",
            ));
        }
        let serialized = serde_json::to_value(view)
            .map_err(|error| ViewError::new(RefusalCode::ProviderUnavailable, error.to_string()))?;
        let serialized = match project {
            Some(project) => project(&serialized)?,
            None => serialized,
        };
        let mut object = serialized.as_object().cloned().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "typed node projection is not an object",
            )
        })?;
        let Some(items) = object.remove("items") else {
            let mut result = DomainResult::success("logical node resolved");
            result.at = Some(canonical_at.to_string());
            result.data = Some(Value::Object(object));
            result.rev = Some(snapshot.revision);
            return Ok(result);
        };
        let items = items.as_array().cloned().ok_or_else(|| {
            ViewError::new(
                RefusalCode::ProviderUnavailable,
                "typed collection items are not an array",
            )
        })?;
        self.page_result(Value::Object(object), items, 0, request, binding)
    }

    fn page_result(
        &self,
        node: Value,
        items: Vec<Value>,
        offset: usize,
        request: &ViewRequest,
        binding: ViewCursorBinding,
    ) -> Result<DomainResult, ViewError> {
        if offset > items.len() {
            return Err(ViewError::new(
                RefusalCode::InvalidCursor,
                "view cursor offset is invalid",
            ));
        }
        let mut pages = prepare_pages(
            &node,
            &items[offset..],
            request.limit,
            &binding,
            self.cursors.max_entries().saturating_add(1),
        )?;
        let first = pages.remove(0);
        let cursor = if pages.is_empty() {
            None
        } else {
            Some(
                self.cursors
                    .insert_prepared_pages(binding.clone(), node.clone(), pages)
                    .ok_or_else(|| {
                        ViewError::new(
                            RefusalCode::ResultTooLarge,
                            "logical collection exceeds the bounded cursor store",
                        )
                    })?,
            )
        };
        collection_result(&node, first.items, cursor, first.stopped_by, &binding)
    }

    fn stored_page_result(
        &self,
        node: Value,
        items: Vec<Value>,
        cursor: Option<String>,
        stopped_by: &'static str,
        _request: &ViewRequest,
        binding: ViewCursorBinding,
    ) -> Result<DomainResult, ViewError> {
        collection_result(&node, items, cursor, stopped_by, &binding)
    }
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
    result.rev = Some(binding.source_revision.clone());
    result.cursor = cursor;
    result.page = Some(serde_json::json!({"stoppedBy": stopped_by}));
    Ok(result)
}

fn prepare_pages(
    node: &Value,
    items: &[Value],
    limit: usize,
    binding: &ViewCursorBinding,
    max_pages: usize,
) -> Result<Vec<ViewStoredPage>, ViewError> {
    let mut pages = Vec::new();
    let mut offset = 0;
    loop {
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
                (offset + 1 < items.len()).then(|| CURSOR_SIZE_PLACEHOLDER.to_string()),
                "complete",
                binding,
            )?;
            let bytes = serde_json::to_vec(&probe)
                .expect("a domain result containing JSON values serializes")
                .len();
            if bytes > PAGE_BYTES {
                if page_items.is_empty() {
                    return Err(ViewError::new(
                        RefusalCode::ResultTooLarge,
                        "one collection item exceeds the 64 KiB page budget",
                    ));
                }
                break "bytes";
            }
            page_items = candidate;
            offset += 1;
        };
        pages.push(ViewStoredPage {
            items: page_items,
            stopped_by,
        });
        if pages.len() > max_pages {
            return Err(ViewError::new(
                RefusalCode::ResultTooLarge,
                "logical collection exceeds the bounded cursor store",
            ));
        }
        if offset == items.len() {
            break;
        }
    }
    let first = &pages[0];
    let first_probe = collection_result(
        node,
        first.items.clone(),
        (pages.len() > 1).then(|| CURSOR_SIZE_PLACEHOLDER.to_string()),
        first.stopped_by,
        binding,
    )?;
    if serde_json::to_vec(&first_probe)
        .expect("a domain result containing JSON values serializes")
        .len()
        > PAGE_BYTES
    {
        return Err(ViewError::new(
            RefusalCode::ResultTooLarge,
            "logical collection metadata exceeds the 64 KiB page budget",
        ));
    }
    Ok(pages)
}

fn cursor_error(error: ViewCursorError) -> ViewError {
    ViewError::new(
        error.code(),
        match error {
            ViewCursorError::Invalid => {
                "view cursor is invalid, expired, or belongs to another question"
            }
            ViewCursorError::Stale => "source revision changed after the view cursor was issued",
        },
    )
}

fn error_result(at: Option<String>, error: ViewError) -> DomainResult {
    let mut result = match error.detail {
        Some(detail) => DomainResult::canonical_rejection_detailed(at, detail, error.message),
        None => DomainResult::canonical_rejection(at, error.code, error.message),
    };
    if let Some(next) = error.next {
        result.next.push(next);
    }
    result
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Object(values) => {
            let mut ordered = values.into_iter().collect::<Vec<_>>();
            ordered.sort_by(|left, right| left.0.cmp(&right.0));
            Value::Object(
                ordered
                    .into_iter()
                    .map(|(key, value)| (key, canonical_value(value)))
                    .collect(),
            )
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ViewError, ViewFilter, ViewReadAuthority, ViewRequest, ViewService, ViewSourceSnapshot,
    };
    use crate::application::result_store::ViewCursorStore;
    use crate::domain::address::QualifiedAddress;
    use crate::domain::node_view::{CollectionView, NodeView, NodeViewData};
    use serde_json::{json, Map};
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct FixtureAuthority {
        revisions: Mutex<HashMap<String, ViewSourceSnapshot>>,
        views: HashMap<String, NodeViewData>,
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
            }
        }

        fn change_revision(&self, at: &str) {
            self.revisions.lock().unwrap().get_mut(at).unwrap().revision = "rev-2".to_string();
        }
    }

    impl ViewReadAuthority for FixtureAuthority {
        fn snapshot(&self, at: &QualifiedAddress) -> Result<ViewSourceSnapshot, ViewError> {
            self.revisions
                .lock()
                .unwrap()
                .get(&at.to_string())
                .cloned()
                .ok_or_else(|| {
                    ViewError::new(
                        crate::domain::refusal::RefusalCode::NotFound,
                        "fixture address was not found",
                    )
                })
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            self.views.get(&at.to_string()).cloned().ok_or_else(|| {
                ViewError::new(
                    crate::domain::refusal::RefusalCode::NotFound,
                    "fixture address was not found",
                )
            })
        }
    }

    #[test]
    fn view_request_is_selected_only_by_address_and_normalized_filter() {
        let request = ViewRequest::new("main:Document.Заказ.Module.Object.Method").unwrap();
        assert_eq!(request.at(), "main:Document.Заказ.Module.Object.Method");
        assert_eq!(request.filter(), &ViewFilter::default());
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
    fn view_keeps_content_behind_explicit_body_and_paginates_whole_lines() {
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
        assert!(cursor.starts_with("vc1."));
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
    fn cursor_replay_is_bound_and_revision_change_is_stale() {
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
        assert!(!stale.ok);
        assert_eq!(stale.diagnostics[0]["code"], "stale_cursor");

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
    fn addressed_collection_stops_at_the_serialized_64_kib_budget_and_replays() {
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
        assert!(first.ok, "{first:?}");
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(serde_json::to_vec(&first).unwrap().len() <= super::PAGE_BYTES);
        let cursor = first.cursor.clone().unwrap();
        let second = service.view(request().with_cursor(cursor.clone()));
        assert_eq!(second, service.view(request().with_cursor(cursor)));
        assert_eq!(second.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert!(serde_json::to_vec(&second).unwrap().len() <= super::PAGE_BYTES);
        let last = service.view(request().with_cursor(second.cursor.unwrap()));
        assert_eq!(last.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(last.cursor.is_none());
        assert!(serde_json::to_vec(&last).unwrap().len() <= super::PAGE_BYTES);
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
        assert!(first.ok, "{first:?}");
        assert_eq!(first.page.as_ref().unwrap()["stoppedBy"], "bytes");
        assert_eq!(
            first.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(serde_json::to_vec(&first).unwrap().len() <= super::PAGE_BYTES);
        let cursor = first.cursor.unwrap();
        let next = service.view_projected(request().with_cursor(cursor.clone()), &project);
        assert_eq!(
            next,
            service.view_projected(request().with_cursor(cursor), &project)
        );
        assert_eq!(next.page.as_ref().unwrap()["stoppedBy"], "complete");
        assert!(next.cursor.is_none());
        assert!(serde_json::to_vec(&next).unwrap().len() <= super::PAGE_BYTES);
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
    fn one_oversized_collection_item_is_refused_without_publishing_a_cursor() {
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
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    #[test]
    fn empty_collection_with_oversized_metadata_is_not_published() {
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
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
    }

    #[test]
    fn collection_exceeding_cursor_capacity_is_refused_before_a_cursor_is_issued() {
        use crate::application::result_store::DEFAULT_TTL;
        let at = "main:Document.Заказ.Module.Object.Body";
        let mut authority = FixtureAuthority::new();
        authority.views.insert(
            at.to_string(),
            NodeViewData::Collection(CollectionView::new(
                NodeView::new(at, "Body", "Тело модуля", Map::new()),
                (0..100).map(|line| json!({"line": line})).collect(),
            )),
        );
        let store = ViewCursorStore::new(DEFAULT_TTL, 2, 1024 * 1024);
        let result = ViewService::new(authority, store)
            .view(ViewRequest::new(at).unwrap().with_limit(1).unwrap());
        assert!(!result.ok);
        assert_eq!(result.diagnostics[0]["code"], "result_too_large");
        assert!(result.cursor.is_none());
    }

    /// Уточнение обязано пережить дорогу от чтения до провода. Разбирать
    /// отказ на код и текст по дороге нельзя: тогда `detailCode` исчезает
    /// молча, а код возвращается к своему умолчанию — и различие, ради
    /// которого уточнение заведено, теряется.
    #[test]
    fn a_detailed_read_refusal_keeps_its_detail_and_overridden_outcome_on_the_wire() {
        use crate::domain::refusal::{Outcome, RefusalCode, RefusalDetail};

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
                _filter: &ViewFilter,
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
}
