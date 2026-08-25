use super::selected_scalar_props;
use crate::application::v13::view::ViewError;
use crate::domain::address::{AddressSegment, NodeKind, QualifiedAddress};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct RoleObject {
    kind: String,
    name: String,
    allowed: Vec<Value>,
    denied: Vec<Value>,
}

pub(super) fn project_role(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let objects = role_objects(payload);
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
            let canonical = canonical_role_object_address(address, object)?;
            if suffix.len() == 1 {
                return Ok(NodeViewData::Node(role_object_node(&canonical, object)));
            }
            project_role_object_suffix(&canonical, object, &suffix[1..])
        }
        _ => Err(ViewError::new("not_found", "role projection was not found")),
    }
}

fn project_role_object_suffix(
    address: &QualifiedAddress,
    object: &RoleObject,
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
    let rights = restricted_rights(object);
    match rls.name() {
        None => Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(format!("{address}.RLS"), "RLS", "RLS", Map::new()),
            rights
                .keys()
                .filter_map(|name| {
                    serde_json::to_value(NodeView::new(
                        format!("{address}.RLS.{name}"),
                        "RLS",
                        name,
                        Map::new(),
                    ))
                    .ok()
                })
                .collect(),
        ))),
        Some(name) => rights
            .contains_key(name)
            .then(|| {
                NodeViewData::Node(NodeView::new(
                    format!("{address}.RLS.{name}"),
                    "RLS",
                    name,
                    Map::new(),
                ))
            })
            .ok_or_else(|| {
                ViewError::new(
                    "not_found",
                    format!("restricted role right `{name}` was not found"),
                )
            }),
    }
}

fn role_objects(payload: &Value) -> Vec<RoleObject> {
    let mut result = BTreeMap::<(String, String), RoleObject>::new();
    for key in ["allowed", "denied"] {
        for group in payload
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(group_kind) = group.get("kind").and_then(Value::as_str) else {
                continue;
            };
            for object in group
                .get("objects")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(name) = object.get("name").and_then(Value::as_str) else {
                    continue;
                };
                let entry = result
                    .entry((group_kind.to_string(), name.to_string()))
                    .or_insert_with(|| RoleObject {
                        kind: group_kind.to_string(),
                        name: name.to_string(),
                        allowed: Vec::new(),
                        denied: Vec::new(),
                    });
                let rights = object
                    .get("rights")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                if key == "allowed" {
                    entry.allowed.extend(rights);
                } else {
                    entry.denied.extend(rights);
                }
            }
        }
    }
    result.into_values().collect()
}

fn role_object_value(address: &QualifiedAddress, object: &RoleObject) -> Option<Value> {
    let name = role_object_logical_name(object)?;
    let at = QualifiedAddress::parse(&format!("{address}.{name}")).ok()?;
    serde_json::to_value(role_object_node(&at, object)).ok()
}

fn role_object_logical_name(object: &RoleObject) -> Option<String> {
    (!object.kind.is_empty() && !object.name.is_empty())
        .then(|| format!("{}_{}", object.kind, object.name))
}

fn role_object_matches(object: &RoleObject, requested: &str) -> bool {
    object.name == requested || role_object_logical_name(object).as_deref() == Some(requested)
}

fn canonical_role_object_address(
    requested: &QualifiedAddress,
    object: &RoleObject,
) -> Result<QualifiedAddress, ViewError> {
    let role_name = requested
        .segments()
        .first()
        .and_then(AddressSegment::name)
        .ok_or_else(|| ViewError::new("provider_unavailable", "role name is unavailable"))?;
    let logical = role_object_logical_name(object)
        .ok_or_else(|| ViewError::new("provider_unavailable", "role object identity is invalid"))?;
    QualifiedAddress::parse(&format!(
        "{}:Role.{role_name}.Right.{logical}",
        requested.source_set()
    ))
    .map_err(|error| ViewError::new("provider_unavailable", error.to_string()))
}

fn role_object_node(address: &QualifiedAddress, object: &RoleObject) -> NodeView {
    let restricted = restricted_rights(object).len();
    NodeView::new(
        address.to_string(),
        "Right",
        &object.name,
        Map::from_iter([
            ("allowedCount".to_string(), json!(object.allowed.len())),
            ("deniedCount".to_string(), json!(object.denied.len())),
            ("objectKind".to_string(), json!(object.kind)),
        ]),
    )
    .with_branches(
        (restricted > 0)
            .then(|| BranchRef::new(format!("{}.RLS", address), restricted))
            .into_iter()
            .collect(),
    )
}

fn restricted_rights(object: &RoleObject) -> BTreeMap<String, Value> {
    let mut restricted = BTreeMap::new();
    for right in object.allowed.iter().chain(object.denied.iter()) {
        if right.get("restricted").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        let Some(name) = right.get("name").and_then(Value::as_str) else {
            continue;
        };
        restricted
            .entry(name.to_string())
            .or_insert_with(|| right.clone());
    }
    restricted
}
