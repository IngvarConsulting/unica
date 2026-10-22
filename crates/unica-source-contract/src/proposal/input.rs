//! Existing operation target decoding against immutable admitted source facts.
use super::{ApplyPlanError, ApplyPlanErrorKind};
use serde_json::{Map, Value};
use unica_address::address::{NodeKind, QualifiedAddress};

pub fn qualified_target(
    args: &Map<String, Value>,
    op_index: usize,
    source_set_name: &str,
) -> Result<QualifiedAddress, ApplyPlanError> {
    let raw = args.get("at").and_then(Value::as_str).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "`at` must be a logical address",
        )
        .at_path(format!("ops[{op_index}].args.at"))
    })?;
    let target = QualifiedAddress::resolve_input(raw, &[source_set_name]).map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
            .at_path(format!("ops[{op_index}].args.at"))
    })?;
    if target.source_set() != source_set_name {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation target belongs to another admitted source set",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    Ok(target)
}

pub fn bad(op_index: usize, field: &str, message: impl Into<String>) -> ApplyPlanError {
    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message.into())
        .at_path(format!("ops[{op_index}].args.{field}"))
}
pub fn required_values<'a>(
    args: &'a Map<String, Value>,
    op_index: usize,
    allowed: &[&str],
) -> Result<&'a Map<String, Value>, ApplyPlanError> {
    let values = args
        .get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| bad(op_index, "values", "`values` must be an object"))?;
    if let Some(field) = values
        .keys()
        .find(|field| !allowed.contains(&field.as_str()))
    {
        return Err(bad(
            op_index,
            &format!("values.{field}"),
            format!(
                "values accept only {}, not `{field}`",
                allowed
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    Ok(values)
}
pub fn required_string<'a>(
    values: &'a Map<String, Value>,
    field: &str,
    op_index: usize,
    location: &str,
) -> Result<&'a str, ApplyPlanError> {
    values
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            bad(
                op_index,
                &format!("{location}.{field}"),
                format!("`{field}` is required and must be a non-empty string"),
            )
        })
}
pub fn creation_name(
    target: &QualifiedAddress,
    kind: NodeKind,
    values: &Map<String, Value>,
    op_index: usize,
    canonical_identifier: for<'a> fn(&'a str) -> Option<&'a str>,
) -> Result<String, ApplyPlanError> {
    let raw = required_string(values, "name", op_index, "values")?;
    let name = canonical_identifier(raw).ok_or_else(|| {
        bad(
            op_index,
            "values.name",
            format!("`{raw}` is not a valid 1C identifier"),
        )
    })?;
    // Узел создаёт операция родителя: `at` всегда называет существующий
    // узел, а имя нового — аргумент.
    if !matches!(target.segments(), [root] if root.kind() == NodeKind::Configuration) {
        return Err(bad(
            op_index,
            "at",
            format!(
                "`{}` creation targets the configuration root; name the new object in `values.name`",
                kind.as_str()
            ),
        ));
    }
    Ok(name.to_string())
}
