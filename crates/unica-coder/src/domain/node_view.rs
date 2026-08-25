use serde::Serialize;
use serde_json::{Map, Value};

/// The seven slots shared by every addressable v0.13 node. Pagination and
/// revision facts intentionally live in `DomainResult`, while `items` belongs
/// to [`CollectionView`] only.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct NodeView {
    at: String,
    kind: String,
    title: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    props: Map<String, Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    branches: Vec<BranchRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    can: Vec<OperationRef>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    limits: Vec<String>,
}

impl NodeView {
    pub(crate) fn new(
        at: impl Into<String>,
        kind: impl Into<String>,
        title: impl Into<String>,
        props: Map<String, Value>,
    ) -> Self {
        Self {
            at: at.into(),
            kind: kind.into(),
            title: title.into(),
            props,
            branches: Vec::new(),
            can: Vec::new(),
            limits: Vec::new(),
        }
    }

    #[must_use]
    pub(crate) fn with_branches(mut self, branches: Vec<BranchRef>) -> Self {
        self.branches = branches;
        self
    }

    #[must_use]
    pub(crate) fn with_can(mut self, can: Vec<OperationRef>) -> Self {
        self.can = can;
        self
    }

    pub(crate) fn at(&self) -> &str {
        &self.at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BranchRef {
    at: String,
    count: usize,
}

impl BranchRef {
    pub(crate) fn new(at: impl Into<String>, count: usize) -> Self {
        Self {
            at: at.into(),
            count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct OperationRef {
    op: String,
    #[serde(skip_serializing_if = "Map::is_empty")]
    args: Map<String, Value>,
}

impl OperationRef {
    pub(crate) fn new(op: impl Into<String>, args: Map<String, Value>) -> Self {
        Self {
            op: op.into(),
            args,
        }
    }
}

/// A branch is itself addressable, but its data rows are not. In particular a
/// source line has only `line` and `text`; it must never acquire a fabricated
/// logical address.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct CollectionView {
    #[serde(flatten)]
    node: NodeView,
    items: Vec<Value>,
}

impl CollectionView {
    pub(crate) fn new(node: NodeView, items: Vec<Value>) -> Self {
        Self { node, items }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub(crate) enum NodeViewData {
    Node(NodeView),
    Collection(CollectionView),
}

impl NodeViewData {
    pub(crate) fn at(&self) -> &str {
        match self {
            Self::Node(node) => node.at(),
            Self::Collection(collection) => collection.node.at(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BranchRef, CollectionView, NodeView, NodeViewData};
    use serde_json::{json, Map};

    #[test]
    fn node_view_has_exactly_seven_common_slots_and_omits_empty_optional_slots() {
        let view = NodeView::new(
            "main:Catalog.Валюты",
            "Catalog",
            "Валюты",
            Map::from_iter([("hierarchical".to_string(), json!(false))]),
        )
        .with_branches(vec![BranchRef::new("main:Catalog.Валюты.Attribute", 12)]);
        let value = serde_json::to_value(view).unwrap();
        assert_eq!(
            value,
            json!({
                "at": "main:Catalog.Валюты",
                "kind": "Catalog",
                "title": "Валюты",
                "props": {"hierarchical": false},
                "branches": [{"at": "main:Catalog.Валюты.Attribute", "count": 12}]
            })
        );
        let allowed = ["at", "kind", "title", "props", "branches", "can", "limits"];
        assert!(value
            .as_object()
            .unwrap()
            .keys()
            .all(|key| allowed.contains(&key.as_str())));
        for forbidden in ["set", "sourceState", "fileExists", "layout", "provider"] {
            assert!(value.get(forbidden).is_none());
        }
    }

    #[test]
    fn only_a_collection_adds_items_and_data_rows_do_not_gain_addresses() {
        let data = NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                "main:Document.Заказ.Module.Object.Body",
                "Body",
                "Тело модуля объекта Заказ",
                Map::new(),
            ),
            vec![json!({"line": 1, "text": "Процедура ПередЗаписью()"})],
        ));
        let value = serde_json::to_value(data).unwrap();
        assert_eq!(value["items"][0].get("at"), None);
        assert_eq!(value["at"], "main:Document.Заказ.Module.Object.Body");
    }

    #[test]
    fn node_view_contract_is_complete() {
        node_view_has_exactly_seven_common_slots_and_omits_empty_optional_slots();
        only_a_collection_adds_items_and_data_rows_do_not_gain_addresses();
    }
}
