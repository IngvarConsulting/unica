use super::{dropped_by_size, item_identity, select_named, selected_scalar_props};
use crate::domain::node_view::{BranchRef, CollectionView, NodeView, NodeViewData};
use unica_bsl::projection::EventProjection;
use unica_execution_contract::view_error::ViewError;

use serde_json::{Map, Value};
use unica_address::address::{AddressSegment, NodeKind, QualifiedAddress};
use unica_refusal::RefusalCode;

/// Скаляры корня формы в его `props`.
const FORM_ROOT_SCALAR_KEYS: &[&str] = &[
    "title",
    "objectContext",
    "isExtension",
    "baseFormVersion",
    "commandBarLocation",
];

pub(super) const PAYLOAD_KEYS: &[&str] = &[
    "name",
    "title",
    "objectContext",
    "isExtension",
    "baseFormVersion",
    "support",
    "supportDetails",
    "properties",
    "events",
    "autoCommandBar",
    "commandBarLocation",
    "conditionalAppearance",
    "elements",
    "attributes",
    "parameters",
    "commands",
];

pub(super) const BRANCH_KINDS: &[NodeKind] = &[
    NodeKind::Item,
    NodeKind::Attribute,
    NodeKind::Parameter,
    NodeKind::Command,
    NodeKind::Event,
];

/// Свойства корня формы под теми же именами и с теми же типами, что
/// принимает `props.set` на форме; заданы только отличные от умолчания.
fn root_properties(payload: &Value) -> Option<Value> {
    use unica_forms::root_properties::{root_property, RootValue};
    let properties = payload
        .get("properties")?
        .as_array()?
        .iter()
        .filter_map(|property| {
            let name = property.get("name")?.as_str()?;
            let text = property.get("value")?.as_str()?;
            let value = match root_property(name).map(|(_, property)| property.value) {
                Some(RootValue::Flag) => Value::Bool(text == "true"),
                Some(RootValue::Size) => text
                    .parse::<u64>()
                    .map_or_else(|_| Value::String(text.to_string()), Value::from),
                _ => Value::String(text.to_string()),
            };
            Some((name.to_string(), value))
        })
        .collect::<Map<String, Value>>();
    (!properties.is_empty()).then_some(Value::Object(properties))
}

pub(super) fn project_form(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
    semantic_events: Option<&[EventProjection]>,
) -> Result<NodeViewData, ViewError> {
    if suffix
        .first()
        .is_some_and(|part| part.kind() == NodeKind::Setting)
    {
        return super::form_appearance::project(address, payload, suffix);
    }
    if let Some(events) = semantic_events.filter(|_| {
        suffix
            .last()
            .is_some_and(|part| part.kind() == NodeKind::Event)
    }) {
        return project_semantic_form_event(address, suffix, events);
    }
    if suffix.is_empty() {
        let mut props = selected_scalar_props(payload, FORM_ROOT_SCALAR_KEYS);
        // Заголовок длиннее предела не пропадает молча: предел называет
        // адрес, по которому он читается целиком.
        let limits = dropped_by_size(payload, &props, address, |probe| {
            selected_scalar_props(probe, FORM_ROOT_SCALAR_KEYS)
        });
        if let Some(properties) = root_properties(payload) {
            props.insert("properties".to_string(), properties);
        }
        if let Some(support) = payload
            .get("support")
            .and_then(|support| support.get("state"))
        {
            props.insert("support".to_string(), support.clone());
        }
        if let Some(details) = payload.get("supportDetails") {
            props.extend(selected_scalar_props(
                details,
                &["supportRule", "supportRuleReason", "directEditSafe"],
            ));
        }
        let mut branches = BRANCH_KINDS
            .iter()
            .copied()
            .filter_map(|kind| {
                if kind == NodeKind::Event {
                    if let Some(events) = semantic_events {
                        let count = form_owner_events(&address.to_string(), events).count();
                        return (count > 0).then(|| {
                            BranchRef::new(format!("{}.{}", address, kind.as_str()), count)
                        });
                    }
                }
                let values = form_child_values(payload, kind)?;
                let count = values.as_array().map_or(0, Vec::len);
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            })
            .collect::<Vec<_>>();
        if payload
            .get("conditionalAppearance")
            .is_some_and(Value::is_object)
        {
            branches.push(BranchRef::new(format!("{address}.Setting"), 1));
        }
        return Ok(NodeViewData::Node(
            NodeView::new(
                address.to_string(),
                "Form",
                payload
                    .get("title")
                    .and_then(Value::as_str)
                    .or_else(|| payload.get("name").and_then(Value::as_str))
                    .unwrap_or("Form"),
                props,
            )
            .with_branches(branches)
            .with_additional_limits(limits),
        ));
    }
    let first = &suffix[0];
    let mut values = form_child_values(payload, first.kind()).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!("form has no {} collection", first.kind().as_str()),
        )
    })?;
    if first.name().is_none() {
        if suffix.len() != 1 {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "form collection did not consume the complete suffix",
            ));
        }
        return form_collection(address, first.kind(), values, semantic_events);
    }
    let mut kind = first.kind();
    let mut item = select_named(values, first.name().unwrap()).ok_or_else(|| {
        ViewError::new(
            RefusalCode::NotFound,
            format!(
                "form {} `{}` was not found",
                first.kind().as_str(),
                first.name().unwrap()
            ),
        )
    })?;
    for (index, segment) in suffix[1..].iter().enumerate() {
        kind = segment.kind();
        values = form_nested_values(item, kind).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("form node has no {} collection", kind.as_str()),
            )
        })?;
        let terminal = index + 2 == suffix.len();
        let Some(name) = segment.name() else {
            if !terminal {
                return Err(ViewError::new(
                    RefusalCode::NotFound,
                    "form collection did not consume the complete suffix",
                ));
            }
            return form_collection(address, kind, values, semantic_events);
        };
        item = select_named(values, name).ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("form {} `{name}` was not found", kind.as_str()),
            )
        })?;
    }
    Ok(NodeViewData::Node(form_node(
        address,
        kind,
        item,
        semantic_events,
    )))
}

fn project_semantic_form_event(
    address: &QualifiedAddress,
    suffix: &[AddressSegment],
    events: &[EventProjection],
) -> Result<NodeViewData, ViewError> {
    let event = suffix
        .last()
        .expect("the semantic event route has an Event suffix");
    if event.name().is_none() {
        let address_text = address.to_string();
        let owner_at = address_text.strip_suffix(".Event").ok_or_else(|| {
            ViewError::new(RefusalCode::NotFound, "event collection owner is invalid")
        })?;
        let items = form_owner_events(owner_at, events)
            .map(crate::infrastructure::v13_read::event_node_value)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return Err(ViewError::new(
                RefusalCode::NotFound,
                "form owner has no applicable events",
            ));
        }
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(
                address.to_string(),
                NodeKind::Event.as_str(),
                "Event",
                Map::new(),
            ),
            items,
        )));
    }
    events
        .iter()
        .find(|candidate| candidate.at == address.to_string())
        .map(crate::infrastructure::v13_read::event_node)
        .map(NodeViewData::Node)
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!(
                    "form Event `{}` was not found",
                    event.name().unwrap_or_default()
                ),
            )
        })
}

fn form_owner_events<'a>(
    owner_at: &'a str,
    events: &'a [EventProjection],
) -> impl Iterator<Item = &'a EventProjection> + 'a {
    events.iter().filter(move |event| {
        event
            .at
            .rsplit_once(".Event.")
            .is_some_and(|(candidate, _)| candidate == owner_at)
    })
}

fn form_child_values(payload: &Value, kind: NodeKind) -> Option<&Value> {
    match kind {
        NodeKind::Item => payload.get("elements"),
        NodeKind::Attribute => payload.get("attributes"),
        NodeKind::Parameter => payload.get("parameters"),
        NodeKind::Command => payload.get("commands"),
        NodeKind::Event => payload.get("events"),
        _ => None,
    }
}

fn form_nested_values(item: &Value, kind: NodeKind) -> Option<&Value> {
    match kind {
        NodeKind::Item => item.get("children"),
        NodeKind::Column => item.get("columns"),
        NodeKind::Event => item.get("events").or_else(|| item.get("actions")),
        _ => None,
    }
}

fn form_collection(
    address: &QualifiedAddress,
    kind: NodeKind,
    values: &Value,
    semantic_events: Option<&[EventProjection]>,
) -> Result<NodeViewData, ViewError> {
    let items = values
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item_identity(item)?;
            let child = address.member(name).ok()?;
            serde_json::to_value(form_node(&child, kind, item, semantic_events)).ok()
        })
        .collect();
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            address.to_string(),
            kind.as_str(),
            kind.as_str(),
            Map::new(),
        ),
        items,
    )))
}

fn form_node(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    semantic_events: Option<&[EventProjection]>,
) -> NodeView {
    let mut props = node_props(kind, item);
    let mut limits = dropped_by_size(item, &props, address, |probe| node_props(kind, probe));
    if kind == NodeKind::Item {
        for (source, property) in [("dataPath", "DataPath"), ("commandName", "CommandName")] {
            if let Some(value) = item.get(source).filter(|value| value.is_string()) {
                super::place_prop(
                    &mut props,
                    &mut limits,
                    address,
                    property,
                    value,
                    false,
                    property,
                );
            }
        }
        limits.extend(
            item.get("bindingLimits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
    }
    let mut node = NodeView::new(
        address.to_string(),
        kind.as_str(),
        item.get("title")
            .and_then(Value::as_str)
            .or_else(|| item_identity(item))
            .unwrap_or(kind.as_str()),
        props,
    )
    .with_additional_limits(limits);
    let branch_kinds: &[NodeKind] = match kind {
        NodeKind::Item => &[NodeKind::Item, NodeKind::Event],
        NodeKind::Attribute => &[NodeKind::Column],
        NodeKind::Command => &[NodeKind::Event],
        _ => &[],
    };
    node = node.with_branches(
        branch_kinds
            .iter()
            .filter_map(|child_kind| {
                if *child_kind == NodeKind::Event {
                    if let Some(events) = semantic_events {
                        let count = form_owner_events(&address.to_string(), events).count();
                        return (count > 0).then(|| {
                            BranchRef::new(format!("{}.{}", address, child_kind.as_str()), count)
                        });
                    }
                }
                let values = form_nested_values(item, *child_kind)?;
                let count = values.as_array().map_or(0, Vec::len);
                (count > 0)
                    .then(|| BranchRef::new(format!("{}.{}", address, child_kind.as_str()), count))
            })
            .collect(),
    );
    node
}

pub(super) fn branch_fields(kind: NodeKind) -> &'static [&'static str] {
    match kind {
        NodeKind::Item => &["elements", "children"],
        NodeKind::Attribute => &["attributes"],
        NodeKind::Parameter => &["parameters"],
        NodeKind::Command => &["commands"],
        NodeKind::Event => &["events", "actions"],
        _ => super::branch_field_names(kind),
    }
}

pub(super) fn node_props(kind: NodeKind, value: &Value) -> Map<String, Value> {
    let keys: &[&str] = match kind {
        NodeKind::Item => &["tag", "title", "visible", "enabled", "readOnly"],
        NodeKind::Event => &["handler", "callType"],
        NodeKind::Attribute => &["type", "isMain", "mainTable"],
        NodeKind::Column => &["type", "parentType"],
        NodeKind::Parameter => &["type", "isKey"],
        NodeKind::Command => &["title", "tooltip", "shortcut", "picture", "representation"],
        _ => &[],
    };
    selected_scalar_props(value, keys)
}
