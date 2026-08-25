use crate::application::result_store::{ViewCursorBinding, ViewCursorError, ViewCursorStore};
use crate::domain::address::QualifiedAddress;
use crate::domain::invocation::DomainResult;
use crate::domain::node_view::NodeViewData;
use serde_json::{Map, Value};
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 1_000;

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
            .map_err(|error| ViewError::new("bad_value", error.to_string()))?;
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
                "bad_value",
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

    fn binding(&self, snapshot: &ViewSourceSnapshot) -> ViewCursorBinding {
        ViewCursorBinding {
            canonical_at: self.at.to_string(),
            projection: self
                .at
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

    /// Internal identity fact for `find`; it is never serialized by `view`.
    fn identity_export_path(&self, _at: &QualifiedAddress) -> Result<Option<String>, ViewError> {
        Ok(None)
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
    code: &'static str,
    message: String,
}

impl ViewError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub(crate) const fn code(&self) -> &'static str {
        self.code
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
        match self.try_view(&request) {
            Ok(result) => result,
            Err(error) => error_result(Some(request.at.to_string()), error),
        }
    }

    fn try_view(&self, request: &ViewRequest) -> Result<DomainResult, ViewError> {
        let snapshot = self.authority.snapshot(&request.at)?;
        let binding = request.binding(&snapshot);
        if let Some(cursor) = request.cursor.as_deref() {
            let stored = self
                .cursors
                .read(cursor, &binding, &snapshot.revision)
                .map_err(cursor_error)?;
            return self.stored_page_result(
                stored.node,
                stored.items,
                stored.next_cursor,
                request,
                stored.binding,
            );
        }

        let view = self
            .authority
            .read_exact(&request.at, &request.filter, &snapshot)?;
        if view.at() != request.at.to_string() {
            return Err(ViewError::new(
                "provider_unavailable",
                "typed reader returned a projection for another logical address",
            ));
        }
        let serialized = serde_json::to_value(view)
            .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))?;
        let mut object = serialized.as_object().cloned().ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "typed node projection is not an object",
            )
        })?;
        let Some(items) = object.remove("items") else {
            let mut result = DomainResult::success("logical node resolved");
            result.at = Some(request.at.to_string());
            result.data = Some(Value::Object(object));
            result.rev = Some(snapshot.revision);
            return Ok(result);
        };
        let items = items.as_array().cloned().ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
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
                "invalid_cursor",
                "view cursor offset is invalid",
            ));
        }
        let end = offset.saturating_add(request.limit).min(items.len());
        let mut page = node.as_object().cloned().ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "stored node projection is not an object",
            )
        })?;
        page.insert(
            "items".to_string(),
            Value::Array(items[offset..end].to_vec()),
        );
        let cursor = if end < items.len() {
            Some(
                self.cursors
                    .insert_pages(binding.clone(), node, items, end, request.limit)
                    .ok_or_else(|| {
                        ViewError::new(
                            "result_too_large",
                            "logical collection exceeds the bounded cursor store",
                        )
                    })?,
            )
        } else {
            None
        };
        let mut result = DomainResult::success("logical collection page resolved");
        result.at = Some(request.at.to_string());
        result.data = Some(Value::Object(page));
        result.rev = Some(binding.source_revision);
        result.cursor = cursor;
        Ok(result)
    }

    fn stored_page_result(
        &self,
        node: Value,
        items: Vec<Value>,
        cursor: Option<String>,
        request: &ViewRequest,
        binding: ViewCursorBinding,
    ) -> Result<DomainResult, ViewError> {
        let mut page = node.as_object().cloned().ok_or_else(|| {
            ViewError::new(
                "provider_unavailable",
                "stored node projection is not an object",
            )
        })?;
        page.insert("items".to_string(), Value::Array(items));
        let mut result = DomainResult::success("logical collection page resolved");
        result.at = Some(request.at.to_string());
        result.data = Some(Value::Object(page));
        result.rev = Some(binding.source_revision);
        result.cursor = cursor;
        Ok(result)
    }
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
    let mut result = DomainResult::success(error.message.clone());
    result.ok = false;
    result.at = at;
    result.diagnostics = vec![serde_json::json!({
        "code": error.code,
        "message": error.message,
    })];
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
                .ok_or_else(|| ViewError::new("not_found", "fixture address was not found"))
        }

        fn read_exact(
            &self,
            at: &QualifiedAddress,
            _filter: &ViewFilter,
            _admitted: &ViewSourceSnapshot,
        ) -> Result<NodeViewData, ViewError> {
            self.views
                .get(&at.to_string())
                .cloned()
                .ok_or_else(|| ViewError::new("not_found", "fixture address was not found"))
        }
    }

    #[test]
    fn view_request_is_selected_only_by_address_and_normalized_filter() {
        let request = ViewRequest::new("main:Document.Заказ.Module.Object.Method").unwrap();
        assert_eq!(request.at(), "main:Document.Заказ.Module.Object.Method");
        assert_eq!(request.filter(), &ViewFilter::default());
        assert_eq!(request.limit(), 50);
        assert_eq!(
            ViewRequest::new("main:Document.Заказ.Module.Object.Method")
                .unwrap()
                .with_limit(1_001)
                .unwrap_err()
                .code(),
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
}
