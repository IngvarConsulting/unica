use super::{
    dropped_by_size, member_properties, metadata_collection, place_prop, selected_scalar_props,
    unreadable_entries,
};
use crate::domain::node_view::{BranchRef, NodeView};
use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};

/// Скаляры члена объекта метаданных в его `props`.
const ITEM_SCALAR_KEYS: &[&str] = &[
    "synonym",
    "comment",
    "required",
    "master",
    "mainFilter",
    "denyIncompleteValues",
    "fillValue",
    "addressingDimension",
    "incomplete",
    "template",
    "httpMethod",
    "handler",
    "procedure",
    "nillable",
    "transactioned",
    "direction",
    // Предопределённый элемент: собственные скаляры и поля по виду
    // владельца. Без них ветвь отвечала бы одним заголовком, то есть
    // домом без содержимого.
    "id",
    "parentId",
    "code",
    "description",
    "isFolder",
    "accountType",
    "offBalance",
    "order",
    "actionPeriodIsBase",
];

pub(super) fn metadata_items(
    address: &QualifiedAddress,
    kind: NodeKind,
    value: &Value,
    payload: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> Vec<Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let child = address.member(name).ok()?;
            serde_json::to_value(metadata_item_with_context(
                &child,
                kind,
                item,
                payload,
                member_profile,
            ))
            .ok()
        })
        .collect()
}

pub(super) fn metadata_item_with_context(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    payload: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> NodeView {
    let node = metadata_item_node(address, kind, item, member_profile);
    if kind == NodeKind::TabularSection {
        node.with_additional_limits(tabular_standard_attribute_limits(payload, item))
    } else {
        node
    }
}

pub(super) fn tabular_standard_attribute_limits(payload: &Value, item: &Value) -> Vec<String> {
    let Some(sections) =
        metadata_collection(payload, NodeKind::TabularSection).and_then(Value::as_array)
    else {
        return Vec::new();
    };
    // Both routes borrow items directly from this array, so the native
    // diagnostic index belongs to this exact section, even for equal values.
    let Some(index) = sections
        .iter()
        .position(|section| std::ptr::eq(section, item))
    else {
        return Vec::new();
    };
    let prefix = format!("collections.tabularSections[{index}].standardAttributes");
    unreadable_entries(payload)
        .filter(|(field, _)| {
            field
                .strip_prefix(&prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(['.', '[']))
        })
        .map(|(field, message)| format!("source_unreadable: {field}: {message}"))
        .collect()
}

pub(super) fn metadata_item_node(
    address: &QualifiedAddress,
    kind: NodeKind,
    item: &Value,
    member_profile: Option<unica_metadata::MetaMemberReadProfile>,
) -> NodeView {
    let mut props = selected_scalar_props(item, ITEM_SCALAR_KEYS);
    let mut limits = dropped_by_size(item, &props, address, |probe| {
        selected_scalar_props(probe, ITEM_SCALAR_KEYS)
    });
    if kind == NodeKind::StandardAttribute {
        let tabular = address
            .segments()
            .iter()
            .rev()
            .nth(1)
            .is_some_and(|segment| segment.kind() == NodeKind::TabularSection);
        let allowed: &[&str] = if tabular {
            unica_metadata::META_STANDARD_ATTRIBUTE_PROPERTY_NAMES
        } else if matches!(address.segments(), [owner, member]
            if owner.kind() == NodeKind::Catalog && owner.name().is_some()
            && member.kind() == NodeKind::StandardAttribute && member.name() == Some("Owner"))
        {
            &["Synonym", "ToolTip", "FillChecking", "FillFromFillingValue"]
        } else {
            &["Synonym", "ToolTip", "FillChecking"]
        };
        for property in item
            .get("properties")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = property
                .get("name")
                .and_then(Value::as_str)
                .filter(|key| allowed.contains(key))
            else {
                continue;
            };
            let Some(value) = property.get("value") else {
                continue;
            };
            place_prop(
                &mut props,
                &mut limits,
                address,
                key,
                value,
                true,
                &format!("metadata standard attribute property {key}"),
            );
        }
    }
    for key in [
        "accountingFlags",
        "extDimensionTypes",
        "returnType",
        "type",
        "fillValue",
    ] {
        if let Some(value) = item.get(key) {
            let subject = format!("metadata {key}");
            place_prop(
                &mut props,
                &mut limits,
                address,
                key,
                value,
                false,
                &subject,
            );
        }
    }
    let mut branches = [
        ("attributes", NodeKind::Attribute),
        ("methods", NodeKind::Method),
        ("parameters", NodeKind::Parameter),
        ("standardAttributes", NodeKind::StandardAttribute),
    ]
    .into_iter()
    .filter_map(|(field, kind)| {
        let items = item.get(field)?.as_array()?;
        (!items.is_empty())
            .then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), items.len()))
    })
    .collect::<Vec<_>>();
    branches.extend(
        item.get("logicalBranches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|branch| {
                let kind = branch.get("kind")?.as_str()?;
                let count = usize::try_from(branch.get("count")?.as_u64()?).ok()?;
                let kind = NodeKind::parse(kind).ok()?;
                (count > 0).then(|| BranchRef::new(format!("{}.{}", address, kind.as_str()), count))
            }),
    );
    // Пределы элемента приходят от читателя: профиль свойств переносит их
    // вместе со своими, а без профиля они переносятся как есть, иначе узел
    // отвечал бы `incomplete` без причины (#1933).
    let (member_props, member_limits) = member_profile
        .and_then(|profile| {
            profile
                .context_for_address(address, kind)
                .map(|context| (profile, context))
        })
        .map_or_else(
            || {
                let limits = item
                    .get("propertyLimits")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
                (Map::new(), limits)
            },
            |(profile, context)| member_properties::project(address, profile, context, item),
        );
    props.extend(member_props);
    limits.extend(member_limits);
    NodeView::new(
        address.to_string(),
        kind.as_str(),
        item.get("synonym")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or(kind.as_str()),
        props,
    )
    .with_branches(branches)
    .with_limits(limits)
}
