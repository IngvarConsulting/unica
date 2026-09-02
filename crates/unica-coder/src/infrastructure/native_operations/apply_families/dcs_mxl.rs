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

#[derive(Debug, Clone)]
enum DcsMxlPlanKind {
    Dcs(DcsEdit),
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
                values.push(typed_head(
                    name,
                    text_field(item, &["type"]),
                    text_field(item, &["title"]),
                ));
            }
            ("add-field", values)
        }
        "field.set" => {
            let values = required_values(args, op_index)?;
            let field = required_text(values, &["field", "dataPath", "name"], &values_path)?;
            (
                "modify-field",
                vec![typed_head(
                    field,
                    text_field(values, &["type"]),
                    text_field(values, &["title"]),
                )],
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
            ("set-field-role", vec![format!("{field} {role}")])
        }
        "parameter.add" => {
            let mut values = Vec::new();
            for (index, item) in required_items(args, op_index)?.iter().enumerate() {
                let location = format!("{items_path}[{index}]");
                let item = item_object(item, &location)?;
                let name = required_text(item, &["name"], &location)?;
                let mut value = typed_head(
                    name,
                    text_field(item, &["type"]),
                    text_field(item, &["title"]),
                );
                if let Some(default) = text_field(item, &["value"]) {
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
            let mut value = name.to_string();
            if let Some(default) = text_field(values, &["value"]) {
                value.push_str(&format!(" value={default}"));
            }
            if let Some(title) = text_field(values, &["title"]) {
                value.push_str(&format!(" [{title}]"));
            }
            if let Some(type_name) = text_field(values, &["type"]) {
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
                values.push(required_text(item, &["field", "dataPath"], &location)?.to_string());
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
                values.push(format!(
                    "{}={expression}",
                    typed_head(
                        name,
                        text_field(item, &["type"]),
                        text_field(item, &["title"])
                    )
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
        let DcsMxlPlanKind::Dcs(edit) = &operation.operation().kind else {
            return Err(hidden_apply_family_unimplemented(op_index));
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

#[cfg(test)]
mod tests {
    use super::{parse_dcs_mxl_plan_operation, plan_dcs_mxl_batch};
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
            "mxl.set",
            &json!({"at": "main:Configuration"}),
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
}
