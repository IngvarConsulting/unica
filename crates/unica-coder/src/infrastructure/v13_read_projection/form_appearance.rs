use super::*;

fn limits(value: &Value) -> Vec<String> {
    value
        .get("limits")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn owner(address: &QualifiedAddress, value: &Value) -> NodeView {
    let count = value["rules"].as_array().map_or(0, Vec::len);
    NodeView::new(
        address.to_string(),
        "Setting",
        "ConditionalAppearance",
        Map::from_iter([
            (
                "sourcePath".into(),
                json!("Attributes/ConditionalAppearance"),
            ),
            ("ruleCount".into(), json!(count)),
        ]),
    )
    .with_branches(vec![BranchRef::new(format!("{address}.Item"), count)])
    .with_limits(limits(value))
}

fn rule(address: &QualifiedAddress, value: &Value) -> NodeView {
    let mut props = Map::new();
    let mut limits = limits(value);
    if let Some(enabled) = value.get("use").and_then(Value::as_bool) {
        props.insert("use".into(), json!(enabled));
    }
    for key in ["selection", "filter", "appearance"] {
        if let Some(value) = value.get(key) {
            let compact = value.to_string();
            if compact.len() <= MAX_COMPACT_PROP_BYTES {
                props.insert(key.into(), json!(compact));
            } else {
                limits.push(format!("{key}: значение превышает 2048 байт UTF-8"));
            }
        }
    }
    NodeView::new(
        address.to_string(),
        "Item",
        value["name"].as_str().unwrap_or("Rule"),
        props,
    )
    .with_limits(limits)
}

pub(super) fn project(
    address: &QualifiedAddress,
    payload: &Value,
    suffix: &[AddressSegment],
) -> Result<NodeViewData, ViewError> {
    let missing = || {
        ViewError::new(
            RefusalCode::NotFound,
            "form conditional appearance setting was not found",
        )
    };
    let value = payload
        .get("conditionalAppearance")
        .filter(|v| v.is_object())
        .ok_or_else(missing)?;
    if suffix.len() == 1 && suffix[0].name().is_none() {
        let child = address
            .member("ConditionalAppearance")
            .map_err(|_| missing())?;
        return Ok(NodeViewData::Collection(CollectionView::new(
            NodeView::new(address.to_string(), "Setting", "Setting", Map::new()),
            vec![serde_json::to_value(owner(&child, value)).expect("node serializes")],
        )));
    }
    if suffix[0].name() != Some("ConditionalAppearance") {
        return Err(missing());
    }
    if suffix.len() == 1 {
        return Ok(NodeViewData::Node(owner(address, value)));
    }
    if suffix.len() != 2 || suffix[1].kind() != NodeKind::Item {
        return Err(missing());
    }
    let rules = value["rules"].as_array().ok_or_else(missing)?;
    if let Some(name) = suffix[1].name() {
        let item = rules
            .iter()
            .find(|item| item["name"].as_str() == Some(name))
            .ok_or_else(missing)?;
        return Ok(NodeViewData::Node(rule(address, item)));
    }
    let items = rules
        .iter()
        .map(|item| {
            let child = address
                .member(item["name"].as_str().expect("typed rule name"))
                .expect("ordinal rule address");
            serde_json::to_value(rule(&child, item)).expect("node serializes")
        })
        .collect();
    Ok(NodeViewData::Collection(CollectionView::new(
        NodeView::new(
            address.to_string(),
            "Item",
            "ConditionalAppearance",
            Map::new(),
        )
        .with_limits(limits(value)),
        items,
    )))
}
