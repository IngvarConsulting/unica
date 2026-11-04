use super::*;
use serde::Serialize;
use serde_json::{json, Map, Value};

pub(super) fn read(settings: roxmltree::Node<'_, '_>) -> Vec<serde_json::Value> {
    let mut rows = Vec::new();
    let indices: HashMap<_, _> = dcs_structure_nodes(settings)
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id(), index))
        .collect();
    for scope in settings.descendants().filter(|node| {
        *node == settings
            || indices.contains_key(&node.id())
            || ["row", "column", "point", "series"].iter().any(|tag| {
                unica_xml_format::names::element_has_name(*node, tag, Some(DCS_SETTINGS_NS))
            })
    }) {
        for (tag, kind, ns) in [
            ("selection", "Selection", DCS_SETTINGS_NS),
            ("filter", "Filter", DCS_SETTINGS_NS),
            ("order", "Order", DCS_SETTINGS_NS),
            ("dataParameters", "DataParameter", DCS_CORE_NS),
            ("outputParameters", "OutputParameter", DCS_CORE_NS),
            (
                "conditionalAppearance",
                "ConditionalAppearance",
                DCS_SETTINGS_NS,
            ),
        ] {
            if kind == "Filter" {
                append_filter(scope, settings, &indices, scope, None, &mut rows);
                continue;
            }
            if kind == "ConditionalAppearance"
                && matches!(
                    container_state(scope, tag, DCS_SETTINGS_NS),
                    ContainerState::Unreadable
                )
            {
                let mut row = scope_facts(scope, settings, &indices);
                row.insert("kind".into(), json!(kind));
                damage(
                    &mut row,
                    "conditional appearance container is malformed or duplicate",
                );
                rows.push(json!(row));
                continue;
            }
            if let Some(container) =
                unica_xml_format::read::child_element_in_namespace(scope, tag, DCS_SETTINGS_NS)
            {
                let mut items = Vec::new();
                if kind == "Selection" {
                    dcs_info_selection_rows(container, &[], &mut items);
                } else if kind == "ConditionalAppearance" {
                    items.extend(
                        container
                            .children()
                            .filter(|child| {
                                child.is_element()
                                    && !(child.tag_name().namespace() == Some(DCS_SETTINGS_NS)
                                        && matches!(
                                            child.tag_name().name(),
                                            "viewMode"
                                                | "userSettingID"
                                                | "userSettingPresentation"
                                        ))
                            })
                            .enumerate()
                            .map(|(index, item)| (index, Vec::new(), item)),
                    );
                } else {
                    items.extend(
                        unica_xml_format::read::child_elements_in_namespace(container, "item", ns)
                            .into_iter()
                            .enumerate()
                            .map(|(index, item)| (index, Vec::new(), item)),
                    );
                }
                for (index, parents, item) in items {
                    let mut row = scope_facts(scope, settings, &indices);
                    if kind == "ConditionalAppearance"
                        && (!item.has_tag_name((DCS_SETTINGS_NS, "item"))
                            || item.attributes().len() != 0
                            || !unica_xml_format::read::direct_text(item).trim().is_empty())
                    {
                        row.insert("kind".into(), json!(kind));
                        row.insert("index".into(), json!(index));
                        damage(
                            &mut row,
                            "conditional appearance item is malformed or unsupported",
                        );
                        rows.push(json!(row));
                        continue;
                    }

                    if !parents.is_empty() {
                        row.insert("parentIndexes".into(), serde_json::json!(parents));
                    }
                    if let Some(title) = unica_xml_format::read::child_element_in_namespace(
                        item,
                        "lwsTitle",
                        DCS_SETTINGS_NS,
                    ) {
                        row.insert(
                            "title".into(),
                            serde_json::json!(dcs_info_multilang_or_inner_text(title)),
                        );
                    }
                    if let Some(kind) = item.attribute((XML_SCHEMA_INSTANCE_NS, "type")) {
                        row.insert(
                            "itemType".into(),
                            serde_json::json!(kind.rsplit(':').next().unwrap_or(kind)),
                        );
                    }
                    row.insert("kind".into(), serde_json::json!(kind));
                    row.insert("index".into(), serde_json::json!(index));
                    if scope != settings {
                        if let Some(name) = dcs_info_child_text(scope, "name", DCS_SETTINGS_NS) {
                            row.insert("group".into(), serde_json::json!(name));
                        }
                    }
                    for (input, output) in [
                        ("field", "field"),
                        ("left", "field"),
                        ("parameter", "name"),
                        ("orderType", "direction"),
                        ("comparisonType", "comparison"),
                        ("right", "value"),
                        ("value", "value"),
                        ("use", "use"),
                        ("viewMode", "viewMode"),
                        ("userSettingID", "userSettingID"),
                        ("userSettingPresentation", "userSettingPresentation"),
                    ] {
                        let child =
                            unica_xml_format::read::child_element_in_namespace(item, input, ns)
                                .or_else(|| {
                                    unica_xml_format::read::child_element_in_namespace(
                                        item,
                                        input,
                                        DCS_SETTINGS_NS,
                                    )
                                });
                        if let Some(child) = child {
                            row.insert(output.into(), serde_json::json!(dcs_text_of(child)));
                        }
                    }
                    if kind == "ConditionalAppearance" {
                        match appearance_fields(item) {
                            Ok(Some(fields)) => {
                                row.insert("fields".into(), json!(fields));
                            }
                            Ok(None) => {}
                            Err(error) => damage(&mut row, error),
                        }
                    }
                    if kind == "ConditionalAppearance" {
                        row.insert(
                            "filterState".into(),
                            json!(container_state(item, "filter", DCS_SETTINGS_NS)),
                        );
                    }
                    rows.push(serde_json::Value::Object(row));
                    if kind == "ConditionalAppearance" {
                        append_filter(scope, settings, &indices, item, Some(index), &mut rows);
                        append_appearance(scope, settings, &indices, item, index, &mut rows);
                    }
                }
            }
        }
    }
    rows
}

/// Scope and indices are derived from the same traversal that publishes Item rows.
fn scope_facts(
    scope: roxmltree::Node<'_, '_>,
    settings: roxmltree::Node<'_, '_>,
    indices: &HashMap<roxmltree::NodeId, usize>,
) -> Map<String, Value> {
    let mut row = Map::new();
    row.insert(
        "scope".into(),
        json!(if scope == settings {
            "root"
        } else {
            "structure"
        }),
    );
    if scope != settings {
        if let Some(index) = scope
            .ancestors()
            .take_while(|node| *node != settings)
            .find_map(|node| indices.get(&node.id()))
        {
            row.insert("structureIndex".into(), json!(index));
        } else {
            damage(
                &mut row,
                "nested settings owner has no published structure Item",
            );
        }
        if let Some(axis) = scope
            .ancestors()
            .take_while(|node| *node != settings)
            .find(|node| {
                node.tag_name().namespace() == Some(DCS_SETTINGS_NS)
                    && matches!(
                        node.tag_name().name(),
                        "row" | "column" | "point" | "series"
                    )
            })
        {
            row.insert("axis".into(), json!(axis.tag_name().name()));
        }
        if let Some(name) = dcs_info_child_text(scope, "name", DCS_SETTINGS_NS) {
            row.insert("group".into(), json!(name));
        }
    }
    row
}

fn damage(row: &mut Map<String, Value>, message: &str) {
    row.entry("unreadable")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .unwrap()
        .push(json!(message));
}

/// These states describe observed XML, rather than inferred default settings.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum ContainerState {
    Absent,
    Empty,
    Present,
    Unreadable,
}

fn children<'a, 'i>(
    node: roxmltree::Node<'a, 'i>,
    name: &str,
    ns: &str,
) -> Vec<roxmltree::Node<'a, 'i>> {
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == name
                && child.tag_name().namespace() == Some(ns)
        })
        .collect()
}
fn container_state(node: roxmltree::Node<'_, '_>, name: &str, ns: &str) -> ContainerState {
    let all = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name)
        .collect::<Vec<_>>();
    match all.as_slice() {
        [] => ContainerState::Absent,
        [one]
            if one.tag_name().namespace() == Some(ns)
                && one.attributes().len() == 0
                && unica_xml_format::read::direct_text(*one).trim().is_empty() =>
        {
            if one.children().any(|child| child.is_element()) {
                ContainerState::Present
            } else {
                ContainerState::Empty
            }
        }
        _ => ContainerState::Unreadable,
    }
}

/// The QName and lexical scalar are observed facts; no runtime value is evaluated.
#[derive(Serialize)]
struct TypedValue {
    r#type: ExpandedName,
    value: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpandedName {
    namespace: String,
    local_name: String,
}
fn typed_value(node: roxmltree::Node<'_, '_>) -> Result<TypedValue, &'static str> {
    if node.children().any(|child| child.is_element()) || node.attributes().len() != 1 {
        return Err("value is not a scalar with one declared type");
    }
    let raw = node
        .attribute((XML_SCHEMA_INSTANCE_NS, "type"))
        .ok_or("value type is absent")?;
    let (prefix, local) = raw.split_once(':').ok_or("value type is not qualified")?;
    let ns = node
        .lookup_namespace_uri(Some(prefix))
        .ok_or("value type prefix is not declared")?;
    let value = unica_xml_format::read::direct_text(node);
    if ns != "http://www.w3.org/2001/XMLSchema" {
        return Err("value type is outside the scalar settings profile");
    }
    let trimmed = value.trim();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let unsigned = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    let valid = match local {
        "string" => true,
        "boolean" => matches!(trimmed, "true" | "false" | "0" | "1"),
        "int" | "integer" => digits(unsigned),
        "decimal" => match unsigned.split_once('.') {
            Some((whole, part)) => {
                (whole.is_empty() || digits(whole))
                    && (part.is_empty() || digits(part))
                    && (!whole.is_empty() || !part.is_empty())
            }
            None => digits(unsigned),
        },
        _ => return Err("value type is outside the scalar settings profile"),
    };
    if !valid {
        return Err("value has a malformed scalar representation");
    }
    Ok(TypedValue {
        r#type: ExpandedName {
            namespace: ns.into(),
            local_name: local.into(),
        },
        value,
    })
}

fn append_filter(
    scope: roxmltree::Node<'_, '_>,
    settings: roxmltree::Node<'_, '_>,
    indices: &HashMap<roxmltree::NodeId, usize>,
    owner: roxmltree::Node<'_, '_>,
    appearance: Option<usize>,
    rows: &mut Vec<Value>,
) {
    let mut context = scope_facts(scope, settings, indices);
    if let Some(index) = appearance {
        context.insert("appearanceIndex".into(), json!(index));
    }
    let mut state = context.clone();
    state.insert("kind".into(), json!("FilterContainer"));
    let status = container_state(owner, "filter", DCS_SETTINGS_NS);
    state.insert("state".into(), json!(status));
    if matches!(status, ContainerState::Unreadable) {
        damage(&mut state, "filter container is malformed or duplicate");
    }
    rows.push(json!(state));
    if matches!(status, ContainerState::Present) {
        filter_items(
            children(owner, "filter", DCS_SETTINGS_NS)[0],
            &context,
            &[],
            rows,
        );
    }
}
fn filter_items(
    container: roxmltree::Node<'_, '_>,
    context: &Map<String, Value>,
    parents: &[usize],
    rows: &mut Vec<Value>,
) {
    for (index, item) in container
        .children()
        .filter(|child| {
            child.is_element()
                && !(child.tag_name().namespace() == Some(DCS_SETTINGS_NS)
                    && (matches!(
                        child.tag_name().name(),
                        "use" | "userSettingID" | "userSettingPresentation" | "viewMode"
                    ) || (child.tag_name().name() == "groupType"
                        && xsi_type_matches(container, DCS_SETTINGS_NS, "FilterItemGroup"))))
        })
        .enumerate()
    {
        let mut row = context.clone();
        row.insert("kind".into(), json!("Filter"));
        row.insert("index".into(), json!(index));
        if !parents.is_empty() {
            row.insert("parentIndexes".into(), json!(parents));
        }
        let group = xsi_type_matches(item, DCS_SETTINGS_NS, "FilterItemGroup");
        let comparison = xsi_type_matches(item, DCS_SETTINGS_NS, "FilterItemComparison");
        if !item.has_tag_name((DCS_SETTINGS_NS, "item")) || (!group && !comparison) {
            damage(&mut row, "filter item kind or namespace is unsupported");
            rows.push(json!(row));
            continue;
        }
        row.insert(
            "itemType".into(),
            json!(if group {
                "FilterItemGroup"
            } else {
                "FilterItemComparison"
            }),
        );
        if item
            .attributes()
            .any(|attr| attr.namespace() != Some(XML_SCHEMA_INSTANCE_NS) || attr.name() != "type")
            || !unica_xml_format::read::direct_text(item).trim().is_empty()
        {
            damage(
                &mut row,
                "filter item has unexpected attributes or mixed text",
            );
        }
        for child in item.children().filter(|child| child.is_element()) {
            let known = if group {
                matches!(
                    child.tag_name().name(),
                    "groupType"
                        | "item"
                        | "use"
                        | "userSettingID"
                        | "userSettingPresentation"
                        | "viewMode"
                )
            } else {
                matches!(
                    child.tag_name().name(),
                    "left"
                        | "comparisonType"
                        | "right"
                        | "use"
                        | "userSettingID"
                        | "userSettingPresentation"
                        | "viewMode"
                )
            };
            if child.tag_name().namespace() != Some(DCS_SETTINGS_NS) || !known {
                damage(&mut row, "filter item contains unsupported content");
            }
        }
        for (input, output) in [
            ("left", "field"),
            ("comparisonType", "comparison"),
            ("right", "value"),
            ("use", "use"),
            ("viewMode", "viewMode"),
            ("userSettingID", "userSettingID"),
            ("userSettingPresentation", "userSettingPresentation"),
            ("groupType", "groupType"),
        ] {
            let found = children(item, input, DCS_SETTINGS_NS);
            if found.len() > 1 {
                damage(&mut row, "filter property is duplicated");
                continue;
            }
            if let Some(child) = found.first() {
                if !matches!(input, "right" | "userSettingPresentation")
                    && (child.children().any(|node| node.is_element())
                        || (child.attributes().len() != 0
                            && !(input == "left"
                                && child.attributes().len() == 1
                                && xsi_type_matches(*child, DCS_CORE_NS, "Field"))))
                {
                    damage(&mut row, "filter property is not a supported scalar");
                    continue;
                }
                if input == "groupType"
                    && !matches!(
                        dcs_text_of(*child).as_str(),
                        "AndGroup" | "OrGroup" | "NotGroup"
                    )
                {
                    damage(&mut row, "filter groupType is unsupported");
                    continue;
                }
                row.insert(output.into(), json!(dcs_text_of(*child)));
                if input == "right" {
                    match typed_value(*child) {
                        Ok(value) => {
                            row.insert("typedValue".into(), json!(value));
                        }
                        Err(error) => damage(&mut row, error),
                    }
                }
            }
        }
        if group && children(item, "groupType", DCS_SETTINGS_NS).is_empty() {
            damage(&mut row, "filter groupType is absent");
        }
        if comparison
            && (children(item, "left", DCS_SETTINGS_NS).is_empty()
                || children(item, "comparisonType", DCS_SETTINGS_NS).is_empty())
        {
            damage(
                &mut row,
                "filter comparison has no field or comparison type",
            );
        }
        rows.push(json!(row));
        if group {
            let mut nested = parents.to_vec();
            nested.push(index);
            filter_items(item, context, &nested, rows);
        }
    }
}

fn append_appearance(
    scope: roxmltree::Node<'_, '_>,
    settings: roxmltree::Node<'_, '_>,
    indices: &HashMap<roxmltree::NodeId, usize>,
    owner: roxmltree::Node<'_, '_>,
    appearance: usize,
    rows: &mut Vec<Value>,
) {
    let mut context = scope_facts(scope, settings, indices);
    context.insert("appearanceIndex".into(), json!(appearance));
    context.insert("kind".into(), json!("AppearanceParameter"));
    match container_state(owner, "appearance", DCS_SETTINGS_NS) {
        ContainerState::Absent | ContainerState::Empty => return,
        ContainerState::Unreadable => {
            damage(
                &mut context,
                "appearance container is malformed or duplicate",
            );
            rows.push(json!(context));
            return;
        }
        ContainerState::Present => {}
    }
    for (index, item) in children(owner, "appearance", DCS_SETTINGS_NS)[0]
        .children()
        .filter(|child| child.is_element())
        .enumerate()
    {
        let mut row = context.clone();
        row.insert("index".into(), json!(index));
        let parameters = children(item, "parameter", DCS_CORE_NS);
        let values = children(item, "value", DCS_CORE_NS);
        let allowed = item
            .children()
            .filter(|child| child.is_element())
            .all(|child| {
                (child.tag_name().namespace() == Some(DCS_CORE_NS)
                    && matches!(child.tag_name().name(), "parameter" | "value"))
                    || (child.tag_name().namespace() == Some(DCS_SETTINGS_NS)
                        && matches!(child.tag_name().name(), "userSettingID" | "viewMode"))
            });
        if !item.has_tag_name((DCS_CORE_NS, "item"))
            || !xsi_type_matches(item, DCS_SETTINGS_NS, "SettingsParameterValue")
            || item.attributes().len() != 1
            || !unica_xml_format::read::direct_text(item).trim().is_empty()
            || !allowed
            || parameters.len() != 1
            || values.len() != 1
        {
            damage(
                &mut row,
                "appearance parameter has an unsupported or malformed structure",
            );
            rows.push(json!(row));
            continue;
        }
        let parameter = parameters[0];
        if parameter.attributes().len() != 0
            || parameter.children().any(|child| child.is_element())
            || !matches!(
                dcs_text_of(parameter).as_str(),
                "МаксимальнаяШирина" | "МинимальнаяШирина"
            )
        {
            damage(
                &mut row,
                "appearance parameter is outside the width profile",
            );
            rows.push(json!(row));
            continue;
        }
        row.insert("parameter".into(), json!(dcs_text_of(parameter)));
        match typed_value(values[0]) {
            Ok(value) => {
                row.insert("typedValue".into(), json!(value));
            }
            Err(error) => damage(&mut row, error),
        }
        rows.push(json!(row));
    }
}

fn appearance_fields(owner: roxmltree::Node<'_, '_>) -> Result<Option<String>, &'static str> {
    match container_state(owner, "selection", DCS_SETTINGS_NS) {
        ContainerState::Absent => return Ok(None),
        ContainerState::Empty => return Ok(Some(String::new())),
        ContainerState::Unreadable => return Err("appearance selection is malformed or duplicate"),
        ContainerState::Present => {}
    }
    let mut fields = Vec::new();
    for item in children(owner, "selection", DCS_SETTINGS_NS)[0]
        .children()
        .filter(|node| node.is_element())
    {
        if !item.has_tag_name((DCS_SETTINGS_NS, "item"))
            || (item.attributes().len() != 0
                && !(item.attributes().len() == 1
                    && xsi_type_matches(item, DCS_SETTINGS_NS, "SelectedItemField")))
            || !unica_xml_format::read::direct_text(item).trim().is_empty()
        {
            return Err("appearance selection item is unsupported");
        }
        let values = children(item, "field", DCS_SETTINGS_NS);
        if values.len() != 1 || item.children().filter(|node| node.is_element()).count() != 1 {
            return Err("appearance selected field is malformed or duplicate");
        }
        let value = values[0];
        if value.children().any(|node| node.is_element())
            || (value.attributes().len() != 0
                && !(value.attributes().len() == 1
                    && xsi_type_matches(value, DCS_CORE_NS, "Field")))
        {
            return Err("appearance selected field is not scalar");
        }
        fields.push(dcs_text_of(value));
    }
    Ok(Some(fields.join(", ")))
}
