use super::metadata::qualified_target;
use super::validate_platform_xml_binding;
use crate::domain::address::{NodeKind, QualifiedAddress};
use crate::domain::events::{DomainEvent, DomainEventKind};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::infrastructure::logical_event_source::attached_resource_relative;
use crate::infrastructure::native_operations::apply::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError,
    ApplyPlanErrorKind, ApplyStagedState,
};
use crate::infrastructure::native_operations::apply_families::request::{
    IndexedPlanOperation, ProvisionalApplyEffect,
};
use crate::infrastructure::workspace_actor::{DcsMxlApplyAuthority, ProviderRootBinding};
use serde_json::{Map, Value};

/// One planned DCS edit: the legacy `dcs-edit` operation name with the value
/// strings it consumes, addressed by the schema template and its dataset or
/// settings variant.
#[derive(Debug, Clone)]
struct DcsEdit {
    template: MetadataAddress,
    legacy: &'static str,
    values: Vec<String>,
    data_set: String,
    variant: String,
}

/// One cell write of `mxl.set`: 1-based row and column inside the area.
#[derive(Debug, Clone)]
struct MxlCellWrite {
    row: i64,
    col: i64,
    text: String,
}

#[derive(Debug, Clone)]
struct MxlEdit {
    template: MetadataAddress,
    area: String,
    columns: Option<i64>,
    cells: Vec<MxlCellWrite>,
}

#[derive(Debug, Clone)]
enum DcsMxlPlanKind {
    Dcs(DcsEdit),
    Mxl(MxlEdit),
    Unsupported,
}

#[derive(Debug, Clone)]
pub(crate) struct DcsMxlPlanOperation {
    operation: String,
    kind: DcsMxlPlanKind,
}

impl DcsMxlPlanOperation {
    pub(crate) fn operation(&self) -> &str {
        &self.operation
    }
}

/// The DCS parts of a logical address: the template owner, plus the dataset,
/// query, settings variant, field or parameter the address descends into.
struct DcsTarget {
    template: MetadataAddress,
    data_set: String,
    variant: String,
    terminal: Option<(NodeKind, String)>,
}

fn dcs_target(target: &QualifiedAddress, op_index: usize) -> Result<DcsTarget, ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let bad = |message: &str| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message.to_string())
            .at_path(at_path.clone())
    };
    let segments = target.segments();
    let [owner, template, rest @ ..] = segments else {
        return Err(bad(
            "DCS operations address a schema template: `Owner.Name.Template.Name[.DataSet|Setting|Query.Name]`",
        ));
    };
    if template.kind() != NodeKind::Template {
        return Err(bad(
            "DCS operations address a `Template` node of a report or object",
        ));
    }
    let owner_name = owner
        .name()
        .ok_or_else(|| bad("the template owner must be named"))?;
    let template_name = template
        .name()
        .ok_or_else(|| bad("the template must be named"))?;
    let template = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!(
            "{}.{owner_name}.Template.{template_name}",
            owner.kind().as_str()
        ),
    )
    .map_err(|error| bad(&error.to_string()))?;
    let mut data_set = String::new();
    let mut variant = String::new();
    let mut terminal = None;
    for segment in rest {
        let name = segment
            .name()
            .ok_or_else(|| bad("DCS address segments below the template must be named"))?
            .to_string();
        match segment.kind() {
            NodeKind::DataSet | NodeKind::Query => data_set = name,
            NodeKind::Setting => variant = name,
            NodeKind::Field | NodeKind::Parameter => terminal = Some((segment.kind(), name)),
            other => {
                return Err(bad(&format!(
                    "DCS operations do not address `{}` nodes",
                    other.as_str()
                )))
            }
        }
    }
    Ok(DcsTarget {
        template,
        data_set,
        variant,
        terminal,
    })
}

fn required_items(args: &Map<String, Value>, op_index: usize) -> Result<&[Value], ApplyPlanError> {
    args.get("items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "`items` must be an array")
                .at_path(format!("ops[{op_index}].args.items"))
        })
}

fn required_values(
    args: &Map<String, Value>,
    op_index: usize,
) -> Result<&Map<String, Value>, ApplyPlanError> {
    args.get("values")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "`values` must be an object")
                .at_path(format!("ops[{op_index}].args.values"))
        })
}

fn text_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
}

fn required_text<'a>(
    object: &'a Map<String, Value>,
    keys: &[&str],
    location: &str,
) -> Result<&'a str, ApplyPlanError> {
    text_field(object, keys).ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("`{}` is required and must be a non-empty string", keys[0]),
        )
        .at_path(format!("{location}.{}", keys[0]))
    })
}

fn item_object<'a>(
    item: &'a Value,
    location: &str,
) -> Result<&'a Map<String, Value>, ApplyPlanError> {
    item.as_object().ok_or_else(|| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, "each item must be an object")
            .at_path(location.to_string())
    })
}

/// `name[:type] [title]` — the legacy field, parameter and calculated-field
/// head syntax.
fn typed_head(name: &str, type_name: Option<&str>, title: Option<&str>) -> String {
    let mut head = name.to_string();
    if let Some(type_name) = type_name {
        head.push(':');
        head.push_str(type_name);
    }
    if let Some(title) = title {
        head.push_str(&format!(" [{title}]"));
    }
    head
}

fn comparison_token(comparison: &str) -> Option<&'static str> {
    Some(match comparison {
        "Equal" => "=",
        "NotEqual" => "<>",
        "Greater" => ">",
        "GreaterOrEqual" => ">=",
        "Less" => "<",
        "LessOrEqual" => "<=",
        "Contains" => "contains",
        "NotContains" => "notContains",
        "BeginsWith" => "beginsWith",
        "NotBeginsWith" => "notBeginsWith",
        "InList" => "in",
        "NotInList" => "notIn",
        "InHierarchy" => "inHierarchy",
        "InListByHierarchy" => "inListByHierarchy",
        "Filled" => "filled",
        "NotFilled" => "notFilled",
        _ => return None,
    })
}

fn terminal_name(
    target: &DcsTarget,
    kind: NodeKind,
    op_index: usize,
    what: &str,
) -> Result<String, ApplyPlanError> {
    match &target.terminal {
        Some((found, name)) if *found == kind => Ok(name.clone()),
        _ => Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "the target address must end with the {what} to remove: `...{}.<name>`",
                kind.as_str()
            ),
        )
        .at_path(format!("ops[{op_index}].args.at"))),
    }
}

pub(crate) fn parse_dcs_mxl_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match crate::application::v13::apply::dispatch_family(operation) {
        Some(crate::domain::apply::OperationFamily::Dcs) => {
            parse_dcs_operation(operation, object, op_index, binding)?
        }
        Some(crate::domain::apply::OperationFamily::Mxl) if operation == "mxl.set" => {
            parse_mxl_set(object, op_index, binding)?
        }
        _ => DcsMxlPlanKind::Unsupported,
    };
    Ok(DcsMxlPlanOperation {
        operation: operation.to_string(),
        kind,
    })
}

fn parse_dcs_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanKind, ApplyPlanError> {
    let address = qualified_target(args, op_index, binding)?;
    let target = dcs_target(&address, op_index)?;
    let items_path = format!("ops[{op_index}].args.items");
    let values_path = format!("ops[{op_index}].args.values");
    let mut data_set = target.data_set.clone();
    let mut variant = target.variant.clone();
    let (legacy, values): (&'static str, Vec<String>) = match operation {
        "field.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let name = text_field(item, &["dataPath", "name"]).ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        "a field needs `dataPath` (or `name`)",
                    )
                    .at_path(format!("{location}.dataPath"))
                })?;
                values.push(checked_head(
                    name,
                    text_field(item, &["type"]),
                    text_field(item, &["title"]),
                    &location,
                )?);
            }
            ("add-field", values)
        }
        "field.set" => {
            let values = required_values(args, op_index)?;
            let field = required_text(values, &["field", "dataPath", "name"], &values_path)?;
            (
                "modify-field",
                vec![checked_head(
                    field,
                    text_field(values, &["type"]),
                    text_field(values, &["title"]),
                    &values_path,
                )?],
            )
        }
        "field.remove" => (
            "remove-field",
            vec![terminal_name(&target, NodeKind::Field, op_index, "field")?],
        ),
        "fieldRole.set" => {
            let values = required_values(args, op_index)?;
            let field = required_text(values, &["field", "dataPath", "name"], &values_path)?;
            let role = required_text(values, &["role"], &values_path)?;
            reject_tokens(field, HEAD_TOKENS, &format!("{values_path}.field"))?;
            reject_tokens(role, HEAD_TOKENS, &format!("{values_path}.role"))?;
            ("set-field-role", vec![format!("{field} {role}")])
        }
        "parameter.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let name = required_text(item, &["name"], &location)?;
                let mut value = checked_head(
                    name,
                    text_field(item, &["type"]),
                    text_field(item, &["title"]),
                    &location,
                )?;
                if let Some(default) = text_field(item, &["value"]) {
                    reject_tokens(default, PARAMETER_TOKENS, &format!("{location}.value"))?;
                    value.push('=');
                    value.push_str(default);
                }
                values.push(value);
            }
            ("add-parameter", values)
        }
        "parameter.set" => {
            let values = required_values(args, op_index)?;
            let name = required_text(values, &["name"], &values_path)?;
            reject_tokens(name, PARAMETER_TOKENS, &format!("{values_path}.name"))?;
            let mut value = name.to_string();
            if let Some(default) = text_field(values, &["value"]) {
                reject_tokens(default, PARAMETER_TOKENS, &format!("{values_path}.value"))?;
                value.push_str(&format!(" value={default}"));
            }
            if let Some(title) = text_field(values, &["title"]) {
                reject_tokens(title, PARAMETER_TOKENS, &format!("{values_path}.title"))?;
                value.push_str(&format!(" [{title}]"));
            }
            if let Some(type_name) = text_field(values, &["type"]) {
                reject_tokens(type_name, PARAMETER_TOKENS, &format!("{values_path}.type"))?;
                value.push_str(&format!(" type={type_name}"));
            }
            ("modify-parameter", vec![value])
        }
        "parameter.remove" => (
            "remove-parameter",
            vec![terminal_name(
                &target,
                NodeKind::Parameter,
                op_index,
                "parameter",
            )?],
        ),
        "filter.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let field = required_text(item, &["field", "dataPath"], &location)?;
                let comparison = text_field(item, &["comparison"]).unwrap_or("Equal");
                let token = comparison_token(comparison).ok_or_else(|| {
                    ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        format!("unknown filter comparison `{comparison}`"),
                    )
                    .at_path(format!("{location}.comparison"))
                })?;
                let value = item
                    .get("value")
                    .map(|value| match value {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_default();
                reject_tokens(field, FILTER_TOKENS, &format!("{location}.field"))?;
                reject_tokens(&value, FILTER_TOKENS, &format!("{location}.value"))?;
                values.push(format!("{field} {token} {value}").trim_end().to_string());
            }
            ("add-filter", values)
        }
        "filter.clear" => ("clear-filter", vec!["all".to_string()]),
        "selection.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let field = required_text(item, &["field", "dataPath"], &location)?;
                reject_tokens(field, HEAD_TOKENS, &format!("{location}.field"))?;
                values.push(field.to_string());
            }
            ("add-selection", values)
        }
        "selection.clear" => ("clear-selection", vec!["all".to_string()]),
        "order.clear" => ("clear-order", vec!["all".to_string()]),
        "conditionalAppearance.clear" => ("clear-conditionalAppearance", vec!["all".to_string()]),
        "query.set" => {
            let values = required_values(args, op_index)?;
            if let Some(named) = text_field(values, &["dataSet"]) {
                data_set = named.to_string();
            }
            let query = required_text(values, &["query", "text"], &values_path)?;
            if query.trim_start().starts_with('@') {
                // The legacy editor reads `@path` as a file to load; the
                // typed surface takes the query text itself.
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "`query` is the query text itself; file references (`@path`) are not accepted",
                )
                .at_path(format!("{values_path}.query")));
            }
            ("set-query", vec![query.to_string()])
        }
        "query.patch" => {
            let values = required_values(args, op_index)?;
            if let Some(named) = text_field(values, &["dataSet"]) {
                data_set = named.to_string();
            }
            let find = required_text(values, &["find"], &values_path)?;
            let replace = values
                .get("replace")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let once = values.get("once").and_then(Value::as_bool).unwrap_or(false);
            for (text, field) in [(find, "find"), (replace, "replace")] {
                reject_tokens(text, &[" => ", "@once"], &format!("{values_path}.{field}"))?;
            }
            let mut value = format!("{find} => {replace}");
            if once {
                value.push_str(" @once");
            }
            ("patch-query", vec![value])
        }
        "calculatedField.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let name = required_text(item, &["name", "dataPath"], &location)?;
                let expression = required_text(item, &["expression"], &location)?;
                reject_tokens(expression, &["@"], &format!("{location}.expression"))?;
                values.push(format!(
                    "{}={expression}",
                    checked_head(
                        name,
                        text_field(item, &["type"]),
                        text_field(item, &["title"]),
                        &location,
                    )?
                ));
            }
            ("add-calculated-field", values)
        }
        "total.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let field = required_text(item, &["field", "dataPath"], &location)?;
                reject_tokens(field, &["@", ":"], &format!("{location}.field"))?;
                if let Some(expression) = text_field(item, &["expression"]) {
                    reject_tokens(expression, &["@"], &format!("{location}.expression"))?;
                }
                values.push(match text_field(item, &["expression"]) {
                    Some(expression) => format!("{field}:{expression}"),
                    None => field.to_string(),
                });
            }
            ("add-total", values)
        }
        "variant.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let name = required_text(item, &["name"], &location)?;
                reject_tokens(name, HEAD_TOKENS, &format!("{location}.name"))?;
                if let Some(title) = text_field(item, &["title", "presentation"]) {
                    reject_tokens(title, HEAD_TOKENS, &format!("{location}.title"))?;
                }
                values.push(match text_field(item, &["title", "presentation"]) {
                    Some(title) => format!("{name} [{title}]"),
                    None => name.to_string(),
                });
            }
            ("add-variant", values)
        }
        "structure.set" | "structure.patch" => {
            let values = required_values(args, op_index)?;
            if let Some(named) = text_field(values, &["variant"]) {
                variant = named.to_string();
            }
            let group_by = values
                .get("groupBy")
                .and_then(Value::as_array)
                .map(|fields| {
                    fields
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|field| !field.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let details = values
                .get("details")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            let mut segments = Vec::new();
            if !group_by.is_empty() {
                segments.push(group_by.join(","));
            }
            if details || segments.is_empty() {
                segments.push("details".to_string());
            }
            (
                if operation == "structure.set" {
                    "set-structure"
                } else {
                    "modify-structure"
                },
                vec![segments.join(" > ")],
            )
        }
        _ => return Ok(DcsMxlPlanKind::Unsupported),
    };
    if variant.is_empty() {
        // Settings-level operations need a variant; the schema's first
        // variant is the platform default.
        variant = "Основной".to_string();
    }
    Ok(DcsMxlPlanKind::Dcs(DcsEdit {
        template: target.template,
        legacy,
        values,
        data_set,
        variant,
    }))
}

/// The largest area the cell editor addresses in one call; a spreadsheet
/// template is a print form, not a data grid, and an unbounded row number
/// would otherwise make the planner allocate that many rows.
const MXL_MAX_ROW: i64 = 10_000;
const MXL_MAX_COLUMN: i64 = 1_000;

fn parse_cell_address(key: &str) -> Option<(i64, i64)> {
    let rest = key.strip_prefix('R')?;
    let (row, col) = rest.split_once('C')?;
    let row = row.parse::<i64>().ok()?;
    let col = col.parse::<i64>().ok()?;
    ((1..=MXL_MAX_ROW).contains(&row) && (1..=MXL_MAX_COLUMN).contains(&col)).then_some((row, col))
}

/// The legacy schema editor reads its values as a small text language with
/// reserved tokens (`@on`, `@user`, `[title]`, `name:type`, ` => `). Typed
/// arguments must not carry them, or the intent changes silently.
fn reject_tokens(value: &str, tokens: &[&str], location: &str) -> Result<(), ApplyPlanError> {
    if let Some(token) = tokens.iter().find(|token| value.contains(**token)) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "the value contains `{}`, a token reserved by the schema editor",
                token.trim()
            ),
        )
        .at_path(location.to_string()));
    }
    Ok(())
}

/// Tokens the field shorthand `name:type [title]` reads specially.
const HEAD_TOKENS: &[&str] = &["@", "#", "[", "]"];
/// Operator markers of the filter expression grammar.
const FILTER_TOKENS: &[&str] = &[
    "@",
    " notBeginsWith",
    " beginsWith",
    " inListByHierarchy",
    " inHierarchy",
    " notContains",
    " contains",
    " notFilled",
    " filled",
    " notIn",
    " in",
    " <>",
    " >=",
    " <=",
    " =",
    " >",
    " <",
];
/// Tokens of the parameter edit grammar.
const PARAMETER_TOKENS: &[&str] = &["@", "[", "]", " value=", " type=", " title="];

fn checked_head(
    name: &str,
    type_name: Option<&str>,
    title: Option<&str>,
    location: &str,
) -> Result<String, ApplyPlanError> {
    for (value, field) in [(Some(name), "name"), (type_name, "type"), (title, "title")] {
        let Some(value) = value else {
            continue;
        };
        reject_tokens(value, HEAD_TOKENS, &format!("{location}.{field}"))?;
        if field != "title" && (value.contains(':') || value.contains(char::is_whitespace)) {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("`{field}` must not contain `:` or whitespace"),
            )
            .at_path(format!("{location}.{field}")));
        }
    }
    Ok(typed_head(name, type_name, title))
}

/// A spreadsheet the cell editor can rewrite without losing content: only the
/// constructs the decompile/compile cores model may be present.
fn require_editable_spreadsheet(xml_text: &str, at_path: &str) -> Result<(), ApplyPlanError> {
    const MODELED: &[&str] = &[
        "languageSettings",
        "columns",
        "rowsItem",
        "templateMode",
        "defaultFormatIndex",
        "height",
        "vgRows",
        "merge",
        "namedItem",
        "line",
        "font",
        "format",
        "columnsID",
    ];
    let document = roxmltree::Document::parse(xml_text).map_err(|error| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("the spreadsheet template is not well-formed XML: {error}"),
        )
        .at_path(at_path.to_string())
    })?;
    for child in document
        .root_element()
        .children()
        .filter(|node| node.is_element())
    {
        let name = child.tag_name().name();
        let area_kind_supported = name != "namedItem"
            || child
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "type")
                .and_then(|node| node.text())
                .map(str::trim)
                == Some("Rows");
        if !MODELED.contains(&name) || !area_kind_supported {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                format!(
                    "the spreadsheet holds `{name}` content the cell editor cannot preserve; edit this template in the Designer"
                ),
            )
            .at_path(at_path.to_string()));
        }
    }
    Ok(())
}

fn parse_mxl_set(
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanKind, ApplyPlanError> {
    let address = qualified_target(args, op_index, binding)?;
    let target = dcs_target(&address, op_index)?;
    if target.terminal.is_some() || !target.data_set.is_empty() || !target.variant.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "mxl.set addresses the spreadsheet template itself: `Owner.Name.Template.T`",
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let values = required_values(args, op_index)?;
    let values_path = format!("ops[{op_index}].args.values");
    if let Some(field) = values
        .keys()
        .find(|field| !["area", "cells", "columns"].contains(&field.as_str()))
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("mxl.set values accept `area`, `cells` and `columns`, not `{field}`"),
        )
        .at_path(format!("{values_path}.{field}")));
    }
    let area = required_text(values, &["area"], &values_path)?.to_string();
    let columns = match values.get("columns") {
        None => None,
        Some(Value::Number(number)) if number.as_i64().is_some_and(|value| value >= 1) => {
            number.as_i64()
        }
        Some(_) => {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "`columns` must be a positive integer",
            )
            .at_path(format!("{values_path}.columns")))
        }
    };
    let cells_object = values
        .get("cells")
        .and_then(Value::as_object)
        .filter(|cells| !cells.is_empty())
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                "`cells` must be a non-empty object keyed by cell address, e.g. {\"R1C1\": \"Валюта\"}",
            )
            .at_path(format!("{values_path}.cells"))
        })?;
    let mut cells = Vec::with_capacity(cells_object.len());
    for (key, value) in cells_object {
        let (row, col) = parse_cell_address(key).ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!(
                    "`{key}` is not a cell address; use `R<row>C<column>` with 1-based numbers up to R{MXL_MAX_ROW}C{MXL_MAX_COLUMN}"
                ),
            )
            .at_path(format!("{values_path}.cells.{key}"))
        })?;
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => {
                return Err(ApplyPlanError::new(
                    ApplyPlanErrorKind::BadValue,
                    "a cell value must be a string, number, boolean or null",
                )
                .at_path(format!("{values_path}.cells.{key}")))
            }
        };
        cells.push(MxlCellWrite { row, col, text });
    }
    cells.sort_by_key(|cell| (cell.row, cell.col));
    Ok(DcsMxlPlanKind::Mxl(MxlEdit {
        template: target.template,
        area,
        columns,
        cells,
    }))
}

/// Applies the cell writes to a decompiled definition: the area is found or
/// appended, compressed empty rows are expanded, cells are set by column.
fn apply_mxl_edit(definition: &mut Value, edit: &MxlEdit) -> Result<(), String> {
    let object = definition
        .as_object_mut()
        .ok_or_else(|| "the decompiled spreadsheet definition is not an object".to_string())?;
    let max_col = edit.cells.iter().map(|cell| cell.col).max().unwrap_or(1);
    let current_columns = object.get("columns").and_then(Value::as_i64).unwrap_or(0);
    let columns = current_columns.max(max_col).max(edit.columns.unwrap_or(0));
    object.insert("columns".to_string(), Value::from(columns));
    let areas = object
        .entry("areas".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let areas = areas
        .as_array_mut()
        .ok_or_else(|| "the decompiled definition has no areas array".to_string())?;
    let index = match areas
        .iter()
        .position(|area| area.get("name").and_then(Value::as_str) == Some(edit.area.as_str()))
    {
        Some(index) => index,
        None => {
            areas.push(serde_json::json!({"name": edit.area, "rows": []}));
            areas.len() - 1
        }
    };
    let area = areas[index]
        .as_object_mut()
        .ok_or_else(|| "an area of the decompiled definition is not an object".to_string())?;
    let rows = area
        .entry("rows".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let mut expanded = Vec::new();
    for row in rows.as_array().cloned().unwrap_or_default() {
        match row.get("empty").and_then(Value::as_i64) {
            Some(count) if row.as_object().is_some_and(|object| object.len() == 1) => {
                for _ in 0..count.max(0) {
                    expanded.push(Value::Object(Map::new()));
                }
            }
            _ if row.is_array() => expanded.push(Value::Object(Map::new())),
            _ => expanded.push(row),
        }
    }
    let needed = edit.cells.iter().map(|cell| cell.row).max().unwrap_or(0) as usize;
    while expanded.len() < needed {
        expanded.push(Value::Object(Map::new()));
    }
    for cell in &edit.cells {
        let row = expanded[cell.row as usize - 1]
            .as_object_mut()
            .ok_or_else(|| "a row of the decompiled definition is not an object".to_string())?;
        row.remove("empty");
        let cells = row
            .entry("cells".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        let cells = cells
            .as_array_mut()
            .ok_or_else(|| "a row's cells are not an array".to_string())?;
        match cells
            .iter_mut()
            .find(|existing| existing.get("col").and_then(Value::as_i64) == Some(cell.col))
        {
            Some(existing) => {
                if let Some(existing) = existing.as_object_mut() {
                    existing.insert("text".to_string(), Value::String(cell.text.clone()));
                    existing.remove("param");
                    existing.remove("template");
                }
            }
            None => cells.push(serde_json::json!({"col": cell.col, "text": cell.text})),
        }
        cells.sort_by_key(|existing| existing.get("col").and_then(Value::as_i64).unwrap_or(0));
    }
    *rows = Value::Array(expanded);
    Ok(())
}

pub(crate) fn plan_dcs_mxl_batch(
    staged: ApplyStagedState,
    authority: DcsMxlApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<DcsMxlPlanOperation>],
) -> Result<(ApplyStagedState, Vec<ProvisionalApplyEffect>), ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(&staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "DCS/MXL planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut staged = staged;
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        let edit = match &operation.operation().kind {
            DcsMxlPlanKind::Dcs(edit) => edit,
            DcsMxlPlanKind::Mxl(edit) => {
                stage_mxl_edit(&mut staged, &authority, edit, op_index, &mut provisional)?;
                continue;
            }
            DcsMxlPlanKind::Unsupported => {
                return Err(hidden_apply_family_unimplemented(op_index));
            }
        };
        let at_path = format!("ops[{op_index}].args.at");
        let relative =
            attached_resource_relative(&edit.template, "Template.xml", authority.source_kind())
                .map_err(|message| {
                    ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                        .at_path(at_path.clone())
                })?;
        let preimage = staged
            .read(&relative)
            .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
            .ok_or_else(|| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::NotFound,
                    "the data composition schema template was not found",
                )
                .at_path(at_path.clone())
            })?;
        let (bom, body) = match preimage.strip_prefix(b"\xef\xbb\xbf") {
            Some(body) => (&b"\xef\xbb\xbf"[..], body),
            None => (&b""[..], preimage.as_slice()),
        };
        let mut xml_text = String::from_utf8(body.to_vec()).map_err(|_| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidSource,
                "the data composition schema is not UTF-8",
            )
            .at_path(at_path.clone())
        })?;
        {
            let document = roxmltree::Document::parse(&xml_text).map_err(|error| {
                ApplyPlanError::new(
                    ApplyPlanErrorKind::InvalidSource,
                    format!("the data composition schema is not well-formed XML: {error}"),
                )
                .at_path(at_path.clone())
            })?;
            crate::infrastructure::native_operations::dcs::require_dcs_root(
                document.root_element(),
            )
            .map_err(|error| {
                ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, error)
                    .at_path(at_path.clone())
            })?;
        }
        let original_line_ending = if xml_text.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let original = xml_text.clone();
        let root = authority.source_root();
        let template_dir = root.join(&relative);
        let base_dir = template_dir.parent().unwrap_or(root);
        let mut query_inputs = Vec::new();
        let mut stdout = String::new();
        let mut force_save = false;
        let value_path = format!("ops[{op_index}].args");
        for value in &edit.values {
            let applied = crate::infrastructure::native_operations::dcs::dcs_edit_apply_operation(
                &mut xml_text,
                edit.legacy,
                value,
                &edit.data_set,
                &edit.variant,
                false,
                base_dir,
                root,
                &mut query_inputs,
                &mut stdout,
                &mut force_save,
            );
            match applied {
                Ok(()) => {}
                // Clearing a section the schema never had is already the
                // requested state, not a failure.
                Err(message)
                    if edit.legacy.starts_with("clear-") && message.contains("section found") =>
                {
                    xml_text = original.clone();
                }
                Err(message) => {
                    return Err(ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message)
                        .at_path(value_path));
                }
            }
        }
        if !force_save && xml_text == original {
            continue;
        }
        let mut xml_text = xml_text.replacen("encoding=\"UTF-8\"", "encoding=\"utf-8\"", 1);
        if original_line_ending == "\r\n" {
            xml_text = xml_text.replace("\r\n", "\n").replace('\n', "\r\n");
        } else {
            xml_text = xml_text.replace("\r\n", "\n");
        }
        if !xml_text.ends_with('\n') {
            xml_text.push_str(original_line_ending);
        }
        let mut postimage = Vec::with_capacity(bom.len() + xml_text.len());
        postimage.extend_from_slice(bom);
        postimage.extend_from_slice(xml_text.as_bytes());
        staged
            .replace(&relative, &preimage, postimage)
            .map_err(|error| ApplyPlanError::staging(error, at_path))?;
        provisional.push(ProvisionalApplyEffect::single(
            relative,
            DomainEvent::new(
                DomainEventKind::DcsChanged,
                edit.template.as_str().to_string(),
            ),
            op_index,
        ));
    }
    Ok((staged, provisional))
}

fn stage_mxl_edit(
    staged: &mut ApplyStagedState,
    authority: &DcsMxlApplyAuthority<'_>,
    edit: &MxlEdit,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let values_path = format!("ops[{op_index}].args.values");
    let relative =
        attached_resource_relative(&edit.template, "Template.xml", authority.source_kind())
            .map_err(|message| {
                ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
            })?;
    let preimage = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
        .ok_or_else(|| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::NotFound,
                "the spreadsheet template (Ext/Template.xml) was not found",
            )
            .at_path(at_path.clone())
        })?;
    let text = String::from_utf8(
        preimage
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&preimage)
            .to_vec(),
    )
    .map_err(|_| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            "the spreadsheet template is not UTF-8",
        )
        .at_path(at_path.clone())
    })?;
    require_editable_spreadsheet(&text, &at_path)?;
    let decompiled = crate::infrastructure::native_operations::mxl::mxl_decompile_document(
        &text,
        &relative.display().to_string(),
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::InvalidSource, message).at_path(at_path.clone())
    })?;
    let mut definition: Value = serde_json::from_str(&decompiled.json_text).map_err(|error| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::ProviderUnavailable,
            format!("the decompiled spreadsheet definition is not JSON: {error}"),
        )
        .at_path(at_path.clone())
    })?;
    apply_mxl_edit(&mut definition, edit).map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(values_path.clone())
    })?;
    let compiled = crate::infrastructure::native_operations::mxl::mxl_compile_document(&definition)
        .map_err(|message| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(values_path)
        })?;
    let mut postimage = b"\xef\xbb\xbf".to_vec();
    postimage.extend_from_slice(compiled.xml.as_bytes());
    if postimage == preimage {
        return Ok(());
    }
    staged
        .replace(&relative, &preimage, postimage)
        .map_err(|error| ApplyPlanError::staging(error, at_path))?;
    provisional.push(ProvisionalApplyEffect::single(
        relative,
        DomainEvent::new(
            DomainEventKind::MxlChanged,
            edit.template.as_str().to_string(),
        ),
        op_index,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        checked_head, parse_cell_address, parse_dcs_mxl_plan_operation, plan_dcs_mxl_batch,
        reject_tokens, require_editable_spreadsheet, typed_head, FILTER_TOKENS,
    };
    use crate::infrastructure::native_operations::apply::ApplyPlanErrorKind;
    use crate::infrastructure::native_operations::apply_families::request::IndexedPlanOperation;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;
    use std::path::Path;

    const SCHEMA: &str = include_str!(
        "../../../../../../tests/fixtures/acceptance/workspace/src/Reports/АнализВерсийОбъектов/Templates/ОсновнаяСхемаКомпоновкиДанных/Ext/Template.xml"
    );

    #[test]
    fn dcs_operations_transform_the_staged_schema_and_keep_its_byte_order_mark() {
        let fixture = ApplySeamFixture::new();
        let template_dir = fixture
            .source_dir()
            .join("Reports/Versions/Templates/Schema/Ext");
        std::fs::create_dir_all(&template_dir).unwrap();
        let mut bytes = b"\xef\xbb\xbf".to_vec();
        bytes.extend_from_slice(SCHEMA.trim_start_matches('\u{feff}').as_bytes());
        std::fs::write(template_dir.join("Template.xml"), &bytes).unwrap();

        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let add_field = parse_dcs_mxl_plan_operation(
            "field.add",
            &json!({
                "at": "main:Report.Versions.Template.Schema.DataSet.НаборДанных1",
                "items": [{"dataPath": "Автор", "title": "Автор версии"}]
            }),
            0,
            &fixture.binding,
        )
        .unwrap();
        let clear_filter = parse_dcs_mxl_plan_operation(
            "filter.clear",
            &json!({"at": "main:Report.Versions.Template.Schema.Setting.Основной"}),
            1,
            &fixture.binding,
        )
        .unwrap();
        let structure = parse_dcs_mxl_plan_operation(
            "structure.set",
            &json!({
                "at": "main:Report.Versions.Template.Schema.Setting.Основной",
                "values": {"groupBy": ["ТипОбъекта"]}
            }),
            2,
            &fixture.binding,
        )
        .unwrap();
        let (staged, effects) = plan_dcs_mxl_batch(
            staged,
            authority,
            &[
                IndexedPlanOperation::new(0, add_field),
                IndexedPlanOperation::new(1, clear_filter),
                IndexedPlanOperation::new(2, structure),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        // Clearing a filter the schema never had changes nothing.
        assert_eq!(effects.len(), 2);
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].relative_path,
            Path::new("Reports/Versions/Templates/Schema/Ext/Template.xml")
        );
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(current) =
            &changes[0].current
        else {
            panic!("the schema keeps bytes");
        };
        assert!(current.starts_with(b"\xef\xbb\xbf"));
        let text = String::from_utf8(current[3..].to_vec()).unwrap();
        assert!(text.contains("<dataPath>Автор</dataPath>"), "{text}");
        assert!(text.contains("Автор версии"), "{text}");
        assert!(text.contains("ТипОбъекта"), "{text}");
    }

    #[test]
    fn dcs_field_removal_needs_the_field_in_the_address() {
        let fixture = ApplySeamFixture::new();
        let error = parse_dcs_mxl_plan_operation(
            "field.remove",
            &json!({"at": "main:Report.Versions.Template.Schema.DataSet.НаборДанных1"}),
            0,
            &fixture.binding,
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }

    #[test]
    fn dcs_mxl_apply_seam_routes_actor_authorized_batch_to_stable_unsupported() {
        let fixture = ApplySeamFixture::new();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_dcs_mxl_plan_operation(
            "dcs.set",
            &json!({"at": "main:Report.Sales.Template.Layout"}),
            0,
            &fixture.binding,
        )
        .unwrap();

        let operation = IndexedPlanOperation::new(0, operation);
        let error = plan_dcs_mxl_batch(staged, authority, &[operation]).unwrap_err();

        assert_eq!(error.kind(), ApplyPlanErrorKind::ProviderUnavailable);
        assert_eq!(error.path(), Some("ops[0].op"));
        assert_eq!(
            error.to_string(),
            "hidden v0.13 apply family is not implemented"
        );
    }

    #[test]
    fn cell_addresses_are_bounded_and_one_based() {
        assert_eq!(parse_cell_address("R1C1"), Some((1, 1)));
        assert_eq!(parse_cell_address("R10000C1000"), Some((10_000, 1_000)));
        assert_eq!(parse_cell_address("R0C1"), None);
        assert_eq!(parse_cell_address("R10001C1"), None);
        assert_eq!(parse_cell_address("R1C1001"), None);
        assert_eq!(parse_cell_address("R99999999999C1"), None);
    }

    #[test]
    fn spreadsheets_with_unmodeled_content_are_refused_as_invalid_source() {
        let editable = "<?xml version=\"1.0\"?><document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><languageSettings/><columns/><rowsItem/><namedItem><type>Rows</type></namedItem></document>";
        assert!(require_editable_spreadsheet(editable, "ops[0].args.at").is_ok());
        let with_drawing = "<?xml version=\"1.0\"?><document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><columns/><drawing/></document>";
        let error = require_editable_spreadsheet(with_drawing, "ops[0].args.at").unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
        assert!(error.to_string().contains("`drawing`"), "{error}");
        let column_area = "<?xml version=\"1.0\"?><document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"><namedItem><type>Columns</type></namedItem></document>";
        let error = require_editable_spreadsheet(column_area, "ops[0].args.at").unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
    }

    #[test]
    fn typed_dcs_arguments_reject_editor_tokens() {
        let error =
            checked_head("Sum", None, Some("Итого [шт]"), "ops[0].args.items[0]").unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.items[0].title"));
        let error = checked_head("Sum:Number", None, None, "ops[0].args.items[0]").unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[0].args.items[0].name"));
        assert_eq!(
            checked_head("Sum", Some("Number"), Some("Итого"), "ops[0].args.items[0]").unwrap(),
            typed_head("Sum", Some("Number"), Some("Итого"))
        );
        let error = reject_tokens(
            "work in progress",
            FILTER_TOKENS,
            "ops[0].args.items[0].value",
        )
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert!(reject_tokens("in progress", FILTER_TOKENS, "x").is_ok());
        assert!(reject_tokens("@user", FILTER_TOKENS, "x").is_err());
    }
}
