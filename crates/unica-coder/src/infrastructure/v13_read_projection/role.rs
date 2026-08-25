use super::selected_scalar_props;
use crate::application::v13::view::ViewError;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use serde_json::{json, Map, Value};

pub(super) fn project_role(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let objects = role_objects(payload);
    let restricted = payload
        .get("restrictedObjects")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if suffix.is_empty() {
        let mut props = selected_scalar_props(payload, &["synonym"]);
        if let Some(totals) = payload.get("totals") {
            props.extend(
                selected_scalar_props(totals, &["allowed", "denied"])
                    .into_iter()
                    .map(|(key, value)| (format!("total{key}"), value)),
            );
        }
        let mut branches = Vec::new();
        if !objects.is_empty() {
            branches.push(BranchRef::new(format!("{}.Right", address), objects.len()));
        }
        if !restricted.is_empty() {
            branches.push(BranchRef::new(format!("{}.RLS", address), restricted.len()));
        }
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                NodeKind::Role.as_str(),
                payload
                    .get("synonym")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("name").and_then(Value::as_str))
                    .unwrap_or("Role"),
                props,
            )
            .with_branches(branches),
        ));
    }
    let segment = &suffix[0];
    match (segment.kind(), segment.name()) {
        (NodeKind::Right, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Right", "Rights", Map::new()),
            objects
                .iter()
                .filter_map(|object| role_object_value(address, object))
                .collect(),
        ))),
        (NodeKind::Right, Some(name)) => {
            let object = objects
                .iter()
                .find(|object| role_object_matches(object, name))
                .ok_or_else(|| {
                    ViewError::new("not_found", format!("role object `{name}` was not found"))
                })?;
            if suffix.len() == 1 {
                return Ok(NodeViewData::Node(role_object_node(address, object)));
            }
            project_role_object_suffix(address, object, &suffix[1..])
        }
        (NodeKind::Rls, None) => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "RLS", "RLS", Map::new()),
            restricted
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|name| {
                    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
                    serde_json::to_value(NodeView::new(at.to_string(), "RLS", name, Map::new()))
                        .ok()
                })
                .collect(),
        ))),
        (NodeKind::Rls, Some(name))
            if restricted.iter().any(|value| value.as_str() == Some(name)) =>
        {
            Ok(NodeViewData::Node(NodeView::new(
                address.to_string(),
                "RLS",
                name,
                Map::new(),
            )))
        }
        _ => Err(ViewError::new("not_found", "role projection was not found")),
    }
}

fn project_role_object_suffix(
    address: &QualifiedAddress,
    object: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let [rls] = suffix else {
        return Err(ViewError::new(
            "not_found",
            "role right projection did not consume the complete address suffix",
        ));
    };
    if rls.kind() != NodeKind::Rls {
        return Err(ViewError::new(
            "not_found",
            "role right has no requested child projection",
        ));
    }
    let rights = object
        .get("rights")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|right| right.get("restricted").and_then(Value::as_bool) == Some(true))
        .collect::<Vec<_>>();
    match rls.name() {
        None => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "RLS", "RLS", Map::new()),
            rights
                .into_iter()
                .filter_map(|right| {
                    let name = right.get("name")?.as_str()?;
                    serde_json::to_value(NodeView::new(
                        format!("{address}.{name}"),
                        "RLS",
                        name,
                        Map::new(),
                    ))
                    .ok()
                })
                .collect(),
        ))),
        Some(name) => rights
            .into_iter()
            .find(|right| right.get("name").and_then(Value::as_str) == Some(name))
            .map(|_| {
                NodeViewData::Node(NodeView::new(address.to_string(), "RLS", name, Map::new()))
            })
            .ok_or_else(|| {
                ViewError::new(
                    "not_found",
                    format!("restricted role right `{name}` was not found"),
                )
            }),
    }
}

fn role_objects(payload: &Value) -> Vec<Value> {
    let mut result = Vec::new();
    for (access, key) in [("allowed", "allowed"), ("denied", "denied")] {
        for group in payload
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let group_kind = group.get("kind").cloned().unwrap_or(Value::Null);
            for object in group
                .get("objects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let mut object = object.as_object().cloned().unwrap_or_default();
                object.insert("access".to_string(), json!(access));
                object.insert("objectKind".to_string(), group_kind.clone());
                result.push(Value::Object(object));
            }
        }
    }
    result
}

fn role_object_value(address: &QualifiedAddress, object: &Value) -> Option<Value> {
    let name = role_object_logical_name(object)?;
    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
    serde_json::to_value(role_object_node(&at, object)).ok()
}

fn role_object_logical_name(object: &Value) -> Option<String> {
    let kind = object.get("objectKind")?.as_str()?;
    let name = object.get("name")?.as_str()?;
    Some(format!("{kind}_{name}"))
}

fn role_object_matches(object: &Value, requested: &str) -> bool {
    object.get("name").and_then(Value::as_str) == Some(requested)
        || role_object_logical_name(object).as_deref() == Some(requested)
}

fn role_object_node(address: &QualifiedAddress, object: &Value) -> NodeView {
    let restricted = object
        .get("rights")
        .and_then(Value::as_array)
        .map(|rights| {
            rights
                .iter()
                .filter(|right| right.get("restricted").and_then(Value::as_bool) == Some(true))
                .count()
        })
        .unwrap_or_default();
    NodeView::new(
        address.to_string(),
        "Right",
        object
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Right"),
        Map::from_iter([
            (
                "access".to_string(),
                object.get("access").cloned().unwrap_or(Value::Null),
            ),
            (
                "objectKind".to_string(),
                object.get("objectKind").cloned().unwrap_or(Value::Null),
            ),
        ]),
    )
    .with_branches(
        (restricted > 0)
            .then(|| BranchRef::new(format!("{}.RLS", address), restricted))
            .into_iter()
            .collect(),
    )
}
