use super::validate_platform_xml_binding;
use crate::domain::apply::OperationDescriptorExt;
use serde_json::{Map, Value};
use unica_address::address::{member_address, NodeKind, QualifiedAddress};

use unica_operations::OperationDescriptor;

use unica_source_contract::proposal::{
    empty_apply_family_batch, hidden_apply_family_unimplemented, ApplyPlanError, ApplyPlanErrorKind,
};
use unica_source_contract::proposal::{ApplyProposal, ApplySourceAuthority};
use unica_source_contract::proposal::{IndexedPlanOperation, ProvisionalApplyEffect};

use unica_templates::operations::MxlOperation;
use unica_templates_designer::planner::{stage_mxl_edit, MxlEdit};
use unica_workspace::actor::ProviderRootBinding;

use unica_templates_designer::mxl_cells::CellText;

#[derive(Debug, Clone)]
enum DcsMxlPlanKind {
    Dcs(Box<unica_dcs::edit::DcsEdit>),
    Mxl(Vec<MxlEdit>),
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

    pub(super) fn dcs_plan(&self) -> Option<&unica_dcs::edit::DcsEdit> {
        match &self.kind {
            DcsMxlPlanKind::Dcs(edit) => Some(edit),
            _ => None,
        }
    }
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

#[cfg(test)]
pub(crate) fn parse_dcs_mxl_plan_operation(
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanOperation, ApplyPlanError> {
    let at = args
        .get("at")
        .and_then(Value::as_str)
        .and_then(|raw| QualifiedAddress::resolve_input(raw, &[binding.source_set_name()]).ok());
    let descriptor = super::resolved_descriptor(operation, at.as_ref(), binding);
    parse_dcs_mxl_plan_operation_in(descriptor, operation, args, op_index, binding)
}

pub(crate) fn parse_dcs_mxl_plan_operation_in(
    descriptor: Option<OperationDescriptor>,
    operation: &str,
    args: &Value,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanOperation, ApplyPlanError> {
    validate_platform_xml_binding(binding, operation, op_index)?;
    let object = args.as_object().ok_or_else(|| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            "operation args must be an object",
        )
        .at_path(format!("ops[{op_index}].args"))
    })?;
    let kind = match descriptor
        .and_then(OperationDescriptor::operation::<unica_templates::operations::MxlOperation>)
    {
        Some(operation) => parse_mxl_operation(operation, object, op_index, binding)?,
        None => match descriptor
            .and_then(OperationDescriptor::operation::<unica_dcs::operations::Primitive>)
        {
            Some(operation) => DcsMxlPlanKind::Dcs(Box::new(unica_dcs::edit::plan_edit(
                operation,
                object,
                op_index,
                binding.source_set_name(),
            )?)),
            None => DcsMxlPlanKind::Unsupported,
        },
    };
    Ok(DcsMxlPlanOperation {
        operation: operation.to_string(),
        kind,
    })
}

use unica_templates::contracts::parse_mxl_cell_address as parse_cell_address;

/// Ячейки области из контракта: строка и колонка с 1, по порядку.
fn mxl_cells(area: &Map<String, Value>) -> Vec<CellText> {
    let cells_object = area["cells"]
        .as_object()
        .expect("shared contract requires cells object");
    let mut cells = Vec::with_capacity(cells_object.len());
    for (key, value) in cells_object {
        let (row, col) = parse_cell_address(key).expect("shared contract validates cell address");
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Null => String::new(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => flag.to_string(),
            _ => unreachable!("shared contract requires scalar cells"),
        };
        cells.push(CellText {
            key: key.clone(),
            row,
            col,
            text,
        });
    }
    cells.sort_by_key(|cell| (cell.row, cell.col));
    cells
}

/// `mxl.set` пишет ячейки существующей области `Template.T.Area.A`;
/// `area.add` на самом макете создаёт новые области с начальными ячейками.
/// Узел создаёт операция родителя: опечатка в имени области не заводит
/// новую (#1354).
fn parse_mxl_operation(
    operation: MxlOperation,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanKind, ApplyPlanError> {
    let normalized = operation
        .contract()
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })?;
    let args = normalized.as_object().expect("object contract");
    let address = unica_source_contract::proposal::input::qualified_target(
        args,
        op_index,
        binding.source_set_name(),
    )?;
    let at_path = format!("ops[{op_index}].args.at");
    let segments = address.segments();
    let shape = match operation {
        MxlOperation::Set => "mxl.set addresses an existing area of a spreadsheet template: `Owner.Name.Template.T.Area.A`",
        MxlOperation::AreaAdd => "area.add addresses the spreadsheet template itself: `Owner.Name.Template.T`",
    };
    let (template_segments, area) = match (operation, segments) {
        (MxlOperation::Set, [owner, template, area])
            if area.kind() == NodeKind::Area && area.name().is_some() =>
        {
            (vec![owner.clone(), template.clone()], area.name())
        }
        (MxlOperation::AreaAdd, [owner, template]) => (vec![owner.clone(), template.clone()], None),
        _ => return Err(ApplyPlanError::new(ApplyPlanErrorKind::BadValue, shape).at_path(at_path)),
    };
    let template_address = QualifiedAddress {
        line_range: None,
        source_set: address.source_set().to_string(),
        segments: template_segments,
    };
    let (template, _) = unica_templates::address::template_owner(
        &template_address,
        op_index,
        shape,
        "MXL operations address a `Template` node of a report or object",
    )?;
    let edits = match operation {
        MxlOperation::Set => {
            let area = area.expect("mxl.set retained the addressed area");
            let values = required_values(args, op_index)?;
            vec![MxlEdit {
                template,
                template_at: template_address.to_string(),
                area: area.to_string(),
                operation,
                location: format!("ops[{op_index}].args.values"),
                columns: values.get("columns").and_then(Value::as_i64),
                cells: mxl_cells(values),
            }]
        }
        MxlOperation::AreaAdd => {
            let items = required_items(args, op_index)?;
            let mut edits: Vec<MxlEdit> = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let location = format!("ops[{op_index}].args.items[{index}]");
                let item = item_object(item, &location)?;
                let name = required_text(item, &["name"], &location)?.to_string();
                // Созданную область `mxl.set` правит по адресу; точка в имени
                // экранируется, а имя, которое грамматика адреса не разбирает,
                // оставило бы область без адреса.
                if QualifiedAddress::parse(&member_address(
                    &format!("{template_address}.Area"),
                    &name,
                ))
                .ok()
                .and_then(|address| {
                    address
                        .segments()
                        .last()
                        .and_then(|s| s.name().map(str::to_string))
                })
                .as_deref()
                    != Some(name.as_str())
                {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        "the area name is not addressable as `Area.<name>`",
                    )
                    .at_path(format!("{location}.name")));
                }
                if edits.iter().any(|edit| edit.area == name) {
                    return Err(ApplyPlanError::new(
                        ApplyPlanErrorKind::BadValue,
                        format!("area `{name}` is listed twice"),
                    )
                    .at_path(format!("{location}.name")));
                }
                edits.push(MxlEdit {
                    template: template.clone(),
                    template_at: template_address.to_string(),
                    area: name,
                    operation,
                    columns: item.get("columns").and_then(Value::as_i64),
                    cells: mxl_cells(item),
                    location,
                });
            }
            edits
        }
    };
    Ok(DcsMxlPlanKind::Mxl(edits))
}

/// The template format filter of the canonical path, read from the staged
/// template descriptor: a DCS operation applies to a data composition schema
/// and `mxl.set` to a spreadsheet only. The filter is the registry's
/// (`OperationDescriptor::admits_template_format`), the one `can` uses; reading
/// the staged state keeps it right for a template added earlier in the same
/// request.
fn require_template_format(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operation: &DcsMxlPlanOperation,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    let template = match &operation.kind {
        DcsMxlPlanKind::Dcs(edit) => &edit.template,
        DcsMxlPlanKind::Mxl(edits) => match edits.first() {
            Some(edit) => &edit.template,
            None => return Ok(()),
        },
        DcsMxlPlanKind::Unsupported => return Ok(()),
    };
    let Some(descriptor) = crate::domain::apply::operation_registry().lookup(&operation.operation)
    else {
        return Ok(());
    };
    let at_path = format!("ops[{op_index}].args.at");
    let relative = unica_source_designer::layout::metadata_descriptor_relative(
        template,
        authority.source_kind(),
    )
    .map_err(|message| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(at_path.clone())
    })?;
    let Some(bytes) = staged
        .read(&relative)
        .map_err(|error| ApplyPlanError::staging(error, at_path.clone()))?
    else {
        // Без описания формат не установлен; тело макета проверяет свой
        // корень само и отвечает «не найден» или «не тот документ».
        return Ok(());
    };
    let format = unica_templates_designer::descriptor::template_type_of(&bytes)
        .map(|raw| unica_templates::TemplateFormat::parse(&raw));
    if descriptor.admits_template_format(format) {
        return Ok(());
    }
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        format!(
            "operation `{}` does not apply to the template `{}` of format `{}`",
            operation.operation,
            template.as_str(),
            format.map_or("unknown", unica_templates::TemplateFormat::as_str)
        ),
    )
    .at_path(at_path))
}

pub(crate) fn plan_dcs_mxl_batch(
    staged: &mut ApplyProposal<'_>,
    authority: &dyn ApplySourceAuthority,
    operations: &[IndexedPlanOperation<DcsMxlPlanOperation>],
) -> Result<Vec<ProvisionalApplyEffect>, ApplyPlanError> {
    if operations.is_empty() {
        return Err(empty_apply_family_batch());
    }
    if !authority.owns_staged_state(staged) {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidState,
            "DCS/MXL planner authority does not own the staged state",
        )
        .at_path("ops"));
    }
    let mut provisional = Vec::new();
    for operation in operations {
        let op_index = operation.index();
        require_template_format(staged, authority, operation.operation(), op_index)?;
        let edit = match &operation.operation().kind {
            DcsMxlPlanKind::Dcs(edit) => edit,
            DcsMxlPlanKind::Mxl(edits) => {
                for edit in edits {
                    stage_mxl_edit(staged, authority, edit, op_index, &mut provisional)?;
                }
                continue;
            }
            DcsMxlPlanKind::Unsupported => {
                return Err(hidden_apply_family_unimplemented(op_index));
            }
        };
        crate::infrastructure::family_codecs::dcs_codec().stage(
            staged,
            authority,
            edit,
            op_index,
            &mut provisional,
        )?;
    }
    Ok(provisional)
}

#[cfg(test)]
mod tests {
    use super::{parse_cell_address, parse_dcs_mxl_plan_operation};
    use crate::domain::apply::ApplyRequest;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;
    use std::path::Path;
    use unica_source_contract::proposal::{
        ApplyPlanError, ApplyPlanErrorKind, PlannedApplyEffects, StagedFileState,
    };
    use unica_workspace::apply::ApplyStagedState;

    /// Пространство имён корня схемы: постобраз сверяется по XML, а не через кодек.
    const DCS_SCHEMA_NS: &str = "http://v8.1c.ru/8.1/data-composition-system/schema";
    const COMPOSED_TEMPLATE: &str = "main:Report.Versions.Template.Schema";
    const COMPOSED_RELATIVE: &str = "Reports/Versions/Templates/Schema/Ext/Template.xml";
    const REMOVED_SOURCE: &str =
        "<dataSource><name>Local</name><dataSourceType>Local</dataSourceType></dataSource>";
    const REMOVED_DATASET: &str = "<dataSet xsi:type=\"DataSetQuery\"><name>Data</name><dataSource>Lo<!-- reference -->c&#97;l</dataSource><query>ВЫБРАТЬ 1 КАК Amount</query></dataSet>";
    const REMOVED_SELECTION: &str =
        "<t:item xsi:type=\"t:SelectedItemField\"><t:field>Amount</t:field></t:item>";

    /// A complete addressed resource for common-planner composition tests.
    /// Its literal neighbours make the whole staged postimage the oracle.
    fn composed_dcs_fixture() -> (ApplySeamFixture, String) {
        let fixture = ApplySeamFixture::new();
        let source = fixture.source_dir();
        let configuration = source.join("Configuration.xml");
        let original = std::fs::read_to_string(&configuration).unwrap();
        std::fs::write(
            configuration,
            original.replace("<ChildObjects>", "<ChildObjects><Report>Versions</Report>"),
        )
        .unwrap();
        std::fs::create_dir_all(source.join("Reports/Versions/Templates/Schema/Ext")).unwrap();
        std::fs::write(source.join("Reports/Versions.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Report uuid="aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"><Properties><Name>Versions</Name><Comment/></Properties><ChildObjects><Template>Schema</Template></ChildObjects></Report></MetaDataObject>"#).unwrap();
        std::fs::write(source.join("Reports/Versions/Templates/Schema.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"><Properties><Name>Schema</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></Template></MetaDataObject>"#).unwrap();
        let xml = format!(
            "\u{feff}<DataCompositionSchema xmlns=\"http://v8.1c.ru/8.1/data-composition-system/schema\" xmlns:t=\"http://v8.1c.ru/8.1/data-composition-system/settings\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\r\n{REMOVED_SOURCE}\n<dataSource><name>Kept</name><dataSourceType>External</dataSourceType></dataSource>\r\n{REMOVED_DATASET}\n<dataSet xsi:type=\"DataSetQuery\"><name>Neighbour</name><dataSource>Kept</dataSource><query>ВЫБРАТЬ 2 КАК Kept</query></dataSet>\r\n<settingsVariant><t:name>Main</t:name><t:settings><t:selection><t:item xsi:type=\"t:SelectedItemFolder\">{REMOVED_SELECTION}<t:item xsi:type=\"t:SelectedItemField\"><t:field>Kept</t:field></t:item><t:placement>Auto</t:placement></t:item><t:item xsi:type=\"t:SelectedItemAuto\"/></t:selection></t:settings></settingsVariant>\n<!-- unchanged --><?outside exact?></DataCompositionSchema>"
        );
        std::fs::write(source.join(COMPOSED_RELATIVE), xml.as_bytes()).unwrap();
        (fixture, xml)
    }

    fn plan_composed_dcs(
        fixture: &ApplySeamFixture,
        at: &str,
        ops: serde_json::Value,
    ) -> Result<(ApplyStagedState, PlannedApplyEffects), ApplyPlanError> {
        let args = json!({"at":at,"ops":ops,"dryRun":true});
        let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
        let admission = fixture.admission();
        super::super::plan_hidden_v13_apply(&request, &fixture.binding, &admission)
    }

    fn projected_source_tree(
        staged: &ApplyStagedState,
        before: &std::collections::BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
        let mut projected = before.clone();
        for change in staged.planned_changes() {
            match change.current {
                StagedFileState::Bytes(bytes) => {
                    projected.insert(change.relative_path, bytes);
                }
                StagedFileState::Absent => {
                    projected.remove(&change.relative_path);
                }
            }
        }
        projected
    }

    #[test]
    fn dcs_productive_integer_indices_use_json_schema_number_semantics() {
        for spelling in ["0", "0.0", "0e0"] {
            for slot in ["index", "parentIndexes"] {
                let (fixture, original) = composed_dcs_fixture();
                let source_before =
                    unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
                let (index, parent) = if slot == "index" {
                    (spelling, "0")
                } else {
                    ("0", spelling)
                };
                let values: serde_json::Value = serde_json::from_str(&format!(
                    r#"{{"index":{index},"parentIndexes":[{parent}]}}"#
                ))
                .unwrap();
                let args = json!({
                    "at":format!("{COMPOSED_TEMPLATE}.Setting.Main"),
                    "ops":[{"op":"selection.remove","args":{"values":values}}],
                    "dryRun":true,
                });
                let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
                let admission = fixture.admission();
                let (staged, _) =
                    super::super::plan_hidden_v13_apply(&request, &fixture.binding, &admission)
                        .unwrap_or_else(|error| {
                            panic!("{slot}={spelling} denotes the same integer index: {error:?}")
                        });
                let changes = staged.planned_changes();
                assert_eq!(changes.len(), 1, "{slot}={spelling}");
                assert_eq!(changes[0].relative_path, Path::new(COMPOSED_RELATIVE));
                let StagedFileState::Bytes(current) = &changes[0].current else {
                    panic!("the selected Template keeps XML bytes");
                };
                assert_eq!(
                    current.as_slice(),
                    original.replace(REMOVED_SELECTION, "").as_bytes(),
                    "{slot}={spelling} must remove only the nested zero-th selection"
                );
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
                    source_before,
                    "planning must not publish {slot}={spelling}"
                );
            }
        }
    }

    #[test]
    fn dcs_final_template_accepts_source_removal_before_its_referencing_dataset() {
        let (fixture, original) = composed_dcs_fixture();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let args = json!({
            "at":COMPOSED_TEMPLATE,
            "ops":[
                {"op":"dataSource.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSource.Local")}},
                {"op":"dataSet.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSet.Data")}},
            ],
            "dryRun":true,
        });
        let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
        let admission = fixture.admission();
        let (staged, _) =
            super::super::plan_hidden_v13_apply(&request, &fixture.binding, &admission)
                .expect("the final Template has no reference to the removed source");
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, Path::new(COMPOSED_RELATIVE));
        let StagedFileState::Bytes(current) = &changes[0].current else {
            panic!("the composed Template keeps XML bytes");
        };
        assert_eq!(
            current.as_slice(),
            original
                .replace(REMOVED_SOURCE, "")
                .replace(REMOVED_DATASET, "")
                .as_bytes(),
            "the final whole Template must retain the other source, dataset and settings exactly"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before,
            "a valid final staged image must remain a preview"
        );
    }

    #[test]
    fn dcs_final_source_refusal_keeps_original_index_and_decoded_reference_without_publication() {
        let (fixture, _) = composed_dcs_fixture();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let args = json!({
            "at":COMPOSED_TEMPLATE,
            "ops":[
                {"op":"filter.clear","args":{"at":format!("{COMPOSED_TEMPLATE}.Setting.Main")}},
                {"op":"dataSource.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSource.Local")}},
            ],
            "dryRun":true,
        });
        let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
        let admission = fixture.admission();
        let error =
            match super::super::plan_hidden_v13_apply(&request, &fixture.binding, &admission) {
                Err(error) => error,
                Ok(_) => {
                    panic!("a final reference split by a comment and entity must prevent removal")
                }
            };
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[1].args"));
        assert!(error
            .to_string()
            .contains("dataSource is still referenced by a dataset"));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before,
            "late resource refusal must not publish a partial Template"
        );
    }

    #[test]
    fn dcs_final_source_refusal_sees_nested_union_and_discards_prior_family_staging() {
        let (fixture, original) = composed_dcs_fixture();
        let nested = "<dataSet xsi:type=\"DataSetUnion\"><name>Union</name><item xsi:type=\"DataSetQuery\"><name>Nested</name><dataSource>Lo<!-- nested reference -->c&#97;l</dataSource><query>ВЫБРАТЬ 3</query></item></dataSet>";
        let source = original.replacen(
            "<settingsVariant>",
            &format!("{nested}<settingsVariant>"),
            1,
        );
        std::fs::write(
            fixture.source_dir().join(COMPOSED_RELATIVE),
            source.as_bytes(),
        )
        .unwrap();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let result = plan_composed_dcs(
            &fixture,
            "main:Report.Versions",
            json!([
                {"op":"props.set","args":{"at":"main:Report.Versions","values":{"Comment":"must not publish"}}},
                {"op":"dataSource.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSource.Local")}},
                {"op":"dataSet.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSet.Data")}},
            ]),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => {
                panic!("the surviving nested Union member still references the removed source")
            }
        };
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[1].args"));
        assert!(error
            .to_string()
            .contains("dataSource is still referenced by a dataset"));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before,
            "late DCS refusal must discard both the Template and earlier metadata staging"
        );
    }

    #[test]
    fn dcs_final_source_readdition_satisfies_obligation_and_preserves_neighbours() {
        let (fixture, original) = composed_dcs_fixture();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let (staged, _) = plan_composed_dcs(&fixture, COMPOSED_TEMPLATE, json!([
            {"op":"dataSource.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSource.Local")}},
            {"op":"dataSource.add","args":{"items":[{"name":"Local"}]}},
        ]))
        .expect("a final declaration with the same name satisfies the source reference");
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].relative_path, Path::new(COMPOSED_RELATIVE));
        let StagedFileState::Bytes(current) = &changes[0].current else {
            panic!("the final Template keeps XML bytes");
        };
        assert!(current.starts_with(b"\xef\xbb\xbf"));
        let text = std::str::from_utf8(&current[3..]).unwrap();
        let document = roxmltree::Document::parse(text).unwrap();
        let namespace = DCS_SCHEMA_NS;
        let sources = document
            .root_element()
            .children()
            .filter(|node| {
                node.has_tag_name((namespace, "dataSource"))
                    && node.children().any(|child| {
                        child.has_tag_name((namespace, "name")) && child.text() == Some("Local")
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(sources.len(), 1);
        assert!(sources[0].children().any(|node| {
            node.has_tag_name((namespace, "dataSourceType")) && node.text() == Some("Local")
        }));
        let mut neighbours = text.to_string();
        neighbours.replace_range(sources[0].range(), "");
        assert_eq!(
            neighbours,
            original
                .trim_start_matches('\u{feff}')
                .replace(REMOVED_SOURCE, ""),
            "every byte outside the re-created source must remain exact"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before
        );
    }

    #[test]
    fn dcs_final_source_obligations_span_separated_family_runs() {
        let (fixture, original) = composed_dcs_fixture();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let (staged, _) = plan_composed_dcs(&fixture, "main:Report.Versions", json!([
            {"op":"dataSource.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSource.Local")}},
            {"op":"props.set","args":{"at":"main:Report.Versions","values":{"Comment":"separate run"}}},
            {"op":"dataSet.remove","args":{"at":format!("{COMPOSED_TEMPLATE}.DataSet.Data")}},
        ]))
        .expect("DCS obligations are discharged after the entire request, across metadata runs");
        let mut expected = source_before.clone();
        expected.insert(
            std::path::PathBuf::from(COMPOSED_RELATIVE),
            original
                .replace(REMOVED_SOURCE, "")
                .replace(REMOVED_DATASET, "")
                .into_bytes(),
        );
        let report_path = std::path::PathBuf::from("Reports/Versions.xml");
        let report = std::str::from_utf8(&source_before[&report_path]).unwrap();
        expected.insert(
            report_path,
            report
                .replace("<Comment/>", "<Comment>separate run</Comment>")
                .into_bytes(),
        );
        assert_eq!(projected_source_tree(&staged, &source_before), expected);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before
        );
    }

    #[test]
    fn dcs_final_source_refusals_follow_operation_order_across_template_groups() {
        let (fixture, original) = composed_dcs_fixture();
        let source = fixture.source_dir();
        let owner_path = source.join("Reports/Versions.xml");
        let owner = std::fs::read_to_string(&owner_path).unwrap();
        std::fs::write(&owner_path, owner.replace("<Template>Schema</Template>", "<Template>Schema</Template><Template>ASchema</Template><Template>ZSchema</Template>")).unwrap();
        let descriptor =
            std::fs::read_to_string(source.join("Reports/Versions/Templates/Schema.xml")).unwrap();
        for (name, uuid, body) in [
            (
                "ASchema",
                "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
                original.replace("Kept", "Other"),
            ),
            (
                "ZSchema",
                "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
                original.clone(),
            ),
        ] {
            std::fs::create_dir_all(source.join(format!("Reports/Versions/Templates/{name}/Ext")))
                .unwrap();
            std::fs::write(
                source.join(format!("Reports/Versions/Templates/{name}.xml")),
                descriptor
                    .replace("<Name>Schema</Name>", &format!("<Name>{name}</Name>"))
                    .replace("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", uuid),
            )
            .unwrap();
            std::fs::write(
                source.join(format!(
                    "Reports/Versions/Templates/{name}/Ext/Template.xml"
                )),
                body.as_bytes(),
            )
            .unwrap();
        }
        let source_before = unica_workspace::test_support::tree_snapshot(&source);
        let result = plan_composed_dcs(
            &fixture,
            "main:Report.Versions",
            json!([
                {"op":"dataSource.remove","args":{"at":"main:Report.Versions.Template.ASchema.DataSource.Local"}},
                {"op":"dataSource.remove","args":{"at":"main:Report.Versions.Template.ZSchema.DataSource.Local"}},
                {"op":"dataSource.add","args":{"at":"main:Report.Versions.Template.ASchema","items":[{"name":"Local"}]}},
                {"op":"dataSource.remove","args":{"at":"main:Report.Versions.Template.ASchema.DataSource.Other"}},
            ]),
        );
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("both final Templates retain a reference to an absent source"),
        };
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some("ops[1].args"));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&source),
            source_before
        );
    }

    #[test]
    fn dcs_query_fields_refuse_explicit_type_but_keep_other_patches_and_cleanup() {
        let (fixture, original) = composed_dcs_fixture();
        let stale_type = r#"<valueType><v8:Type xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xs="http://www.w3.org/2001/XMLSchema">xs:string</v8:Type></valueType>"#;
        let field = format!(
            "<field xsi:type=\"DataSetFieldField\"><dataPath>Amount</dataPath><field>Amount</field>{stale_type}<appearance/></field>"
        );
        let source =
            original.replacen("<name>Data</name>", &format!("<name>Data</name>{field}"), 1);
        std::fs::write(
            fixture.source_dir().join(COMPOSED_RELATIVE),
            source.as_bytes(),
        )
        .unwrap();
        let source_before = unica_workspace::test_support::tree_snapshot(&fixture.source_dir());
        let dataset_at = format!("{COMPOSED_TEMPLATE}.DataSet.Data");
        let field_at = format!("{dataset_at}.Field.Amount");
        let plan = |ops| {
            let args = json!({"at":COMPOSED_TEMPLATE,"ops":ops,"dryRun":true});
            let request = ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
            let admission = fixture.admission();
            super::super::plan_hidden_v13_apply(&request, &fixture.binding, &admission)
        };
        let (staged, _) = plan(json!([
            {"op":"field.add","args":{"at":dataset_at,"items":[{"dataPath":"Added"}]}},
            {"op":"field.set","args":{"at":field_at,"values":{"presentationExpression":"Amount + 1"}}},
        ]))
        .expect("Query fields still accept additions and patches without an explicit type");
        let changes = staged.planned_changes();
        assert_eq!(changes.len(), 1);
        let StagedFileState::Bytes(current) = &changes[0].current else {
            panic!("the patched Query fields keep XML bytes");
        };
        let text = std::str::from_utf8(current)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(text).unwrap();
        let namespace = DCS_SCHEMA_NS;
        let field_by_path = |path| {
            document
                .descendants()
                .find(|node| {
                    node.has_tag_name((namespace, "field"))
                        && node.children().any(|child| {
                            child.has_tag_name((namespace, "dataPath"))
                                && child.text() == Some(path)
                        })
                })
                .unwrap()
        };
        assert!(!field_by_path("Added")
            .children()
            .any(|node| node.has_tag_name((namespace, "valueType"))));
        let patched = field_by_path("Amount");
        assert!(!patched
            .children()
            .any(|node| node.has_tag_name((namespace, "valueType"))));
        assert!(patched.children().any(|node| {
            node.has_tag_name((namespace, "presentationExpression"))
                && node.text() == Some("Amount + 1")
        }));
        assert!(patched
            .children()
            .any(|node| node.has_tag_name((namespace, "appearance"))));

        for (operation, args, path) in [
            (
                "field.add",
                json!({"at":dataset_at,"items":[{"dataPath":"Added","type":"string"}]}),
                "ops[0].args.items[0].type",
            ),
            (
                "field.set",
                json!({"at":field_at,"values":{"type":"string"}}),
                "ops[0].args.values.type",
            ),
        ] {
            let error = match plan(json!([{"op":operation,"args":args}])) {
                Err(error) => error,
                Ok(_) => panic!(
                    "{operation}: a Query field derives its type and must refuse an explicit type"
                ),
            };
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{operation}");
            assert_eq!(error.path(), Some(path), "{operation}: {error}");
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source_dir()),
            source_before,
            "Query field refusal and preview must not publish source changes"
        );
    }

    #[test]
    fn typed_mxl_decoder_normalizes_before_target_and_keeps_exact_integer_grammar() {
        let fixture = ApplySeamFixture::new();
        for number in ["1.0", "1e0", "0", "-1", "18446744073709551615"] {
            let args = serde_json::from_str(&format!(
                r#"{{"at":"main:Report.Versions.Template.Schema","values":{{"cells":{{"R1C1":"A"}},"columns":{number}}}}}"#
            ))
            .unwrap();
            let error = parse_dcs_mxl_plan_operation("mxl.set", &args, 3, &fixture.binding)
                .expect_err("full normalization must precede the wrong target shape");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{number}");
            assert_eq!(error.path(), Some("ops[3].args.values.columns"), "{number}");
        }
        for number in [1_i64, i64::MAX] {
            let parsed = parse_dcs_mxl_plan_operation(
                "mxl.set",
                &json!({
                    "at":"main:Report.Versions.Template.Schema.Area.A",
                    "values":{"cells":{"R+1C+1":null},"columns":number},
                }),
                3,
                &fixture.binding,
            )
            .unwrap();
            let super::DcsMxlPlanKind::Mxl(edits) = parsed.kind else {
                panic!("typed MXL must select the MXL planner");
            };
            assert_eq!(edits.len(), 1);
            assert_eq!(edits[0].operation, super::MxlOperation::Set);
            assert_eq!(edits[0].columns, Some(number));
            assert_eq!(edits[0].cells[0].text, "");
            assert_eq!((edits[0].cells[0].row, edits[0].cells[0].col), (1, 1));
        }
    }

    #[test]
    fn full_schema_input_is_absent_from_the_closed_registry() {
        assert!(crate::domain::apply::operation_registry()
            .lookup("dcs.set")
            .is_none());
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
}

#[cfg(test)]
#[path = "dcs_mxl_guard_tests.rs"]
mod guard_tests;
