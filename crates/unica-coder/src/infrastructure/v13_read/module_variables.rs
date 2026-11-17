use super::*;

pub(super) fn find_variable<'a>(
    projections: &'a ModuleProjectionSet,
    name: &str,
) -> Result<&'a VariableProjection, ViewError> {
    let variable = projections
        .variables()
        .iter()
        .find(|variable| variable.name == name)
        .ok_or_else(|| {
            ViewError::new(
                RefusalCode::NotFound,
                format!("variable `{name}` was not found"),
            )
        })?;
    if variable.at.is_none() {
        return Err(ViewError::new(
            RefusalCode::NotFound,
            format!("variable name `{name}` is ambiguous"),
        ));
    }
    Ok(variable)
}

fn props(variable: &VariableProjection) -> Map<String, Value> {
    let mut props = Map::from_iter([
        ("name".to_string(), json!(variable.name)),
        ("export".to_string(), json!(variable.export)),
        ("line".to_string(), json!(variable.line)),
    ]);
    if let Some(directive) = &variable.directive {
        props.insert("directive".to_string(), json!(directive));
    }
    if let Some(region) = &variable.region {
        props.insert("region".to_string(), json!(region));
    }
    props
}

pub(super) fn variable_node(variable: &VariableProjection) -> NodeView {
    NodeView::new(
        variable.at.as_deref().unwrap_or_default(),
        NodeKind::Variable.as_str(),
        &variable.name,
        props(variable),
    )
    .with_limits(variable.limits.clone())
}

pub(super) fn variable_node_value(variable: &VariableProjection) -> Value {
    if variable.at.is_some() {
        serde_json::to_value(variable_node(variable)).expect("variable node is serializable")
    } else {
        json!({"kind": "Variable", "title": variable.name, "props": props(variable), "limits": variable.limits})
    }
}
