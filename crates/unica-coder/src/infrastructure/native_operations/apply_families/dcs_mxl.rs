use super::validate_platform_xml_binding;
use crate::domain::apply::OperationDescriptorExt;
use serde_json::{Map, Value};
use unica_address::address::{member_address, NodeKind, QualifiedAddress};

#[cfg(test)]
use unica_dcs::operations::DcsIntent;
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
    Dcs(Box<unica_dcs_designer::planner::DcsPrimitivePlan>),
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

    pub(super) fn dcs_plan(&self) -> Option<&unica_dcs_designer::planner::DcsPrimitivePlan> {
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
            Some(operation) => {
                DcsMxlPlanKind::Dcs(Box::new(unica_dcs_designer::planner::parse_dcs_operation(
                    operation,
                    object,
                    op_index,
                    binding.source_set_name(),
                )?))
            }
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
    let target = unica_dcs_designer::planner::dcs_target(&template_address, op_index)?;
    let edits = match operation {
        MxlOperation::Set => {
            let area = area.expect("mxl.set retained the addressed area");
            let values = required_values(args, op_index)?;
            vec![MxlEdit {
                template: target.template,
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
                    template: target.template.clone(),
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
        unica_dcs_designer::planner::stage_dcs_edit(
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
pub(crate) fn plan_dcs_mxl_batch_owned_for_test(
    mut staged: unica_workspace::apply::ApplyStagedState,
    authority: unica_workspace::actor::DcsMxlApplyAuthority<'_>,
    operations: &[IndexedPlanOperation<DcsMxlPlanOperation>],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        Vec<ProvisionalApplyEffect>,
    ),
    ApplyPlanError,
> {
    let effects = plan_dcs_mxl_batch(&mut staged.proposal(), &authority, operations)?;
    Ok((staged, effects))
}

#[cfg(test)]
mod tests {
    use super::DcsMxlPlanKind;
    use super::{
        parse_cell_address, parse_dcs_mxl_plan_operation, plan_dcs_mxl_batch_owned_for_test,
    };
    use crate::domain::apply::ApplyRequest;
    use crate::infrastructure::native_operations::apply_families::tests::ApplySeamFixture;
    use serde_json::json;
    use std::path::Path;
    use unica_source_contract::proposal::IndexedPlanOperation;
    use unica_source_contract::proposal::{
        ApplyPlanError, ApplyPlanErrorKind, PlannedApplyEffects, StagedFileState,
    };
    use unica_workspace::apply::ApplyStagedState;

    const SCHEMA: &str = include_str!(
        "../../../../../../tests/fixtures/acceptance/workspace/src/Reports/АнализВерсийОбъектов/Templates/ОсновнаяСхемаКомпоновкиДанных/Ext/Template.xml"
    );
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
        let namespace = unica_dcs_designer::schema::DCS_SCHEMA_NS;
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
        let namespace = unica_dcs_designer::schema::DCS_SCHEMA_NS;
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
    fn dcs_contract_refuses_ignored_arguments_instead_of_planning_another_edit() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
        for (op, args, path) in [
            (
                "query.patch",
                json!({"at":at,"values":{"find":"1","replace":true}}),
                "values.replace",
            ),
            (
                "query.patch",
                json!({"at":at,"values":{"find":"1","once":"true"}}),
                "values.once",
            ),
            (
                "structure.set",
                json!({"at":at,"values":{"details":"false"}}),
                "values.details",
            ),
            (
                "structure.set",
                json!({"at":at,"values":{"groupBy":["Amount",42]}}),
                "values.groupBy[1]",
            ),
            (
                "query.set",
                json!({"at":at,"values":{"query":"ВЫБРАТЬ 1","typo":1}}),
                "values.typo",
            ),
            (
                "field.add",
                json!({"at":at,"items":[{"dataPath":"New","typo":1}]}),
                "items[0].typo",
            ),
            ("filter.clear", json!({"at":at,"typo":1}), "typo"),
        ] {
            let error = parse_dcs_mxl_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("a malformed request must not silently become a different edit");
            assert_eq!(
                error.kind(),
                ApplyPlanErrorKind::BadValue,
                "{op}: {error:?}"
            );
            assert_eq!(error.path(), Some(format!("ops[0].args.{path}").as_str()));
        }
    }

    /// Набор находится по имени на любой глубине объединения; повторённое
    /// имя даёт все пути, отсутствующее — ни одного (#1357).
    #[test]
    fn a_data_set_is_found_by_name_inside_unions() {
        let schema = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema">
<dataSet><name>Union</name><item><name>Member</name></item><item><name>Twin</name></item></dataSet>
<dataSet><name>Top</name></dataSet><dataSet><name>Twin</name></dataSet></DataCompositionSchema>"#;
        let path =
            |name| unica_dcs_designer::recovery::data_set_path_in_schema(schema.as_bytes(), name);
        use unica_operations::recovery::NamedLookup;
        assert_eq!(
            path("Member"),
            NamedLookup::Found(vec!["Union".to_string(), "Member".to_string()])
        );
        assert_eq!(path("Top"), NamedLookup::Found(vec!["Top".to_string()]));
        assert_eq!(
            path("Twin"),
            NamedLookup::Ambiguous(vec![
                vec!["Union".to_string(), "Twin".to_string()],
                vec!["Twin".to_string()],
            ])
        );
        assert_eq!(path("Missing"), NamedLookup::Missing);
    }

    /// Элемент структуры находится так, как его адресует чтение: путь —
    /// именованные предки, безымянная группа прозрачна, повторённое имя
    /// даёт пути всех адресуемых совпадений (#1357).
    #[test]
    fn a_structure_item_is_found_by_its_named_ancestors() {
        let schema = r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" xmlns:dcsset="http://v8.1c.ru/8.1/data-composition-system/settings" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
<settingsVariant><dcsset:name>Main</dcsset:name><dcsset:settings>
<dcsset:item xsi:type="dcsset:StructureItemGroup">
 <dcsset:item xsi:type="dcsset:StructureItemGroup"><dcsset:name>Top</dcsset:name>
  <dcsset:item xsi:type="dcsset:StructureItemGroup"><dcsset:name>Details</dcsset:name></dcsset:item>
  <dcsset:item xsi:type="dcsset:StructureItemGroup"><dcsset:name>Twin</dcsset:name></dcsset:item>
 </dcsset:item>
</dcsset:item>
<dcsset:item xsi:type="dcsset:StructureItemTable"><dcsset:name>Table</dcsset:name>
 <dcsset:row><dcsset:name>Twin</dcsset:name></dcsset:row>
</dcsset:item>
<dcsset:item xsi:type="dcsset:StructureItemGroup"><dcsset:name>Same</dcsset:name></dcsset:item>
<dcsset:item xsi:type="dcsset:StructureItemGroup"><dcsset:name>Same</dcsset:name></dcsset:item>
</dcsset:settings></settingsVariant></DataCompositionSchema>"#;
        use unica_operations::recovery::NamedLookup;
        let path = |name| {
            unica_dcs_designer::recovery::structure_item_path_in_schema(
                schema.as_bytes(),
                "Main",
                name,
            )
        };
        let names = |items: &[&str]| {
            items
                .iter()
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(path("Top"), NamedLookup::Found(names(&["Top"])));
        assert_eq!(
            path("Details"),
            NamedLookup::Found(names(&["Top", "Details"]))
        );
        assert_eq!(
            path("Twin"),
            NamedLookup::Ambiguous(vec![names(&["Top", "Twin"]), names(&["Table", "Twin"])])
        );
        // Два одноимённых элемента на одном уровне не адресуемы: ни один
        // путь не выбирается.
        assert_eq!(path("Same"), NamedLookup::Ambiguous(Vec::new()));
        assert_eq!(path("Missing"), NamedLookup::Missing);
        assert_eq!(
            unica_dcs_designer::recovery::structure_item_path_in_schema(
                schema.as_bytes(),
                "Other",
                "Top"
            ),
            NamedLookup::Missing
        );
    }

    /// Запрос — безымянный узел набора `DataSet.D.Query`: его правят только
    /// операции запроса, а прежний псевдоним набора `Template.T.Query.D`
    /// и аргументы, дублировавшие адрес, отказывают (#1353).
    #[test]
    fn a_query_is_addressed_through_its_dataset_and_retired_spellings_refuse() {
        let fixture = ApplySeamFixture::new();
        let schema = "main:Report.Versions.Template.Schema";
        for (op, at, values) in [
            (
                "query.set",
                "DataSet.НаборДанных1.Query",
                json!({"query": "ВЫБРАТЬ 1"}),
            ),
            (
                "query.patch",
                "DataSet.НаборДанных1.Query",
                json!({"find": "1"}),
            ),
        ] {
            let args = json!({"at": format!("{schema}.{at}"), "values": values});
            let Ok(DcsMxlPlanKind::Dcs(plan)) =
                parse_dcs_mxl_plan_operation(op, &args, 0, &fixture.binding)
                    .map(|operation| operation.kind)
            else {
                panic!("{op} at {at} is a DCS plan");
            };
            assert_eq!(plan.target.datasets, vec!["НаборДанных1".to_string()]);
        }
        for (op, at, values, path, says) in [
            (
                "query.set",
                "Query.НаборДанных1",
                json!({"query": "ВЫБРАТЬ 1"}),
                "at",
                "DataSet.НаборДанных1.Query",
            ),
            (
                "dataSet.set",
                "DataSet.НаборДанных1.Query",
                json!({"autoFillFields": false}),
                "at",
                "not its Query",
            ),
            (
                "query.set",
                "DataSet.НаборДанных1",
                json!({"query": "ВЫБРАТЬ 1"}),
                "at",
                "DataSet.D.Query",
            ),
            (
                "query.set",
                "DataSet.НаборДанных1.Query",
                json!({"dataSet": "НаборДанных1", "query": "ВЫБРАТЬ 1"}),
                "values.dataSet",
                "unknown argument",
            ),
            (
                "structure.set",
                "Setting.Основной",
                json!({"variant": "Основной"}),
                "values.variant",
                "unknown argument",
            ),
        ] {
            let args = json!({"at": format!("{schema}.{at}"), "values": values});
            let error = parse_dcs_mxl_plan_operation(op, &args, 0, &fixture.binding)
                .expect_err("a retired spelling is refused");
            assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{op} at {at}");
            assert_eq!(
                error.path(),
                Some(format!("ops[0].args.{path}").as_str()),
                "{op} at {at}"
            );
            let message = error.to_string();
            assert!(message.contains(says), "{op} at {at}: {message}");
        }
    }

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
        let (staged, effects) = plan_dcs_mxl_batch_owned_for_test(
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
        let unica_source_contract::proposal::StagedFileState::Bytes(current) = &changes[0].current
        else {
            panic!("the schema keeps bytes");
        };
        assert!(current.starts_with(b"\xef\xbb\xbf"));
        let text = String::from_utf8(current[3..].to_vec()).unwrap();
        assert!(
            roxmltree::Document::parse(&text)
                .unwrap()
                .descendants()
                .any(|node| node.tag_name().name() == "dataPath" && node.text() == Some("Автор")),
            "{text}"
        );
        assert!(text.contains("Автор версии"), "{text}");
        assert!(text.contains("ТипОбъекта"), "{text}");
    }

    /// Lays the acceptance schema out as `Report.Versions.Template.Schema`
    /// and plans `operations` on it.
    fn plan_on_versions_schema(
        fixture: &ApplySeamFixture,
        operations: &[(&str, serde_json::Value)],
    ) -> Result<String, unica_source_contract::proposal::ApplyPlanError> {
        let template_dir = fixture
            .source_dir()
            .join("Reports/Versions/Templates/Schema/Ext");
        std::fs::create_dir_all(&template_dir).unwrap();
        std::fs::write(template_dir.join("Template.xml"), SCHEMA.as_bytes()).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let operations = operations
            .iter()
            .enumerate()
            .map(|(index, (op, args))| {
                IndexedPlanOperation::new(
                    index,
                    parse_dcs_mxl_plan_operation(op, args, index, &fixture.binding)
                        .unwrap_or_else(|error| panic!("{op}: {error:?}")),
                )
            })
            .collect::<Vec<_>>();
        let (staged, _) = plan_dcs_mxl_batch_owned_for_test(staged, authority, &operations)?;
        let changes = staged.planned_changes();
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current
        else {
            panic!("expected XML bytes");
        };
        Ok(std::str::from_utf8(bytes)
            .unwrap()
            .trim_start_matches('\u{feff}')
            .to_string())
    }

    /// The translations of the element of `tag` named by its `key` child.
    fn translations(
        xml: &str,
        tag: &str,
        key: &str,
        name: &str,
        text: &str,
    ) -> Vec<(String, String)> {
        let document = roxmltree::Document::parse(xml).unwrap();
        let owner = document
            .descendants()
            .find(|node| {
                node.tag_name().name() == tag
                    && node
                        .children()
                        .any(|child| child.tag_name().name() == key && child.text() == Some(name))
            })
            .unwrap_or_else(|| panic!("{tag} {name} is missing\n{xml}"));
        let value = owner
            .children()
            .find(|child| child.tag_name().name() == text)
            .unwrap_or_else(|| panic!("{tag} {name} has no {text}\n{xml}"));
        value
            .children()
            .filter(|item| item.tag_name().name() == "item")
            .map(|item| {
                let part = |name: &str| {
                    item.children()
                        .find(|child| child.tag_name().name() == name)
                        .and_then(|child| child.text())
                        .unwrap_or_default()
                        .to_string()
                };
                (part("lang"), part("content"))
            })
            .collect()
    }

    /// #1257: a plain-string title is written in the default language of the
    /// source set; a set keeps the translations of the other languages.
    #[test]
    fn dcs_titles_write_the_configuration_default_language() {
        let fixture = ApplySeamFixture::new();
        fixture.use_english_only_language();
        let schema = "main:Report.Versions.Template.Schema";
        let data_set = format!("{schema}.DataSet.НаборДанных1");
        let xml = plan_on_versions_schema(
            &fixture,
            &[
                (
                    "field.add",
                    json!({"at": data_set, "items": [{"dataPath": "Автор", "title": "Version author"}]}),
                ),
                (
                    "field.set",
                    json!({"at": format!("{data_set}.Field.Количество"), "values": {"title": "Count"}}),
                ),
                (
                    "parameter.add",
                    json!({"at": schema, "items": [{"name": "Period", "title": "Period"}]}),
                ),
                (
                    "calculatedField.add",
                    json!({"at": schema, "items": [{"name": "Share", "expression": "1", "title": "Share"}]}),
                ),
                (
                    "variant.add",
                    json!({"at": schema, "items": [{"name": "Brief", "title": "Brief"}]}),
                ),
                (
                    "variant.set",
                    json!({"at": format!("{schema}.Setting.Основной"), "values": {"title": "Stored versions"}}),
                ),
            ],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        let en = |text: &str| ("en".to_string(), text.to_string());
        let ru = |text: &str| ("ru".to_string(), text.to_string());
        assert_eq!(
            translations(&xml, "field", "dataPath", "Автор", "title"),
            [en("Version author")]
        );
        assert_eq!(
            translations(&xml, "field", "dataPath", "Количество", "title"),
            [ru("Количество"), en("Count")],
            "the Russian translation is kept"
        );
        assert_eq!(
            translations(&xml, "parameter", "name", "Period", "title"),
            [en("Period")]
        );
        assert_eq!(
            translations(&xml, "calculatedField", "dataPath", "Share", "title"),
            [en("Share")]
        );
        assert_eq!(
            translations(&xml, "settingsVariant", "name", "Brief", "presentation"),
            [en("Brief")]
        );
        assert_eq!(
            translations(&xml, "settingsVariant", "name", "Основной", "presentation"),
            [
                ru("Количество и объем хранимых версий объектов"),
                en("Stored versions")
            ]
        );
        let content = "<v8:content>Количество и объем хранимых версий объектов</v8:content>";
        let start = SCHEMA[..SCHEMA.find(content).unwrap()]
            .rfind("<v8:item>")
            .unwrap();
        let end = SCHEMA[start..].find("</v8:item>").unwrap() + start + "</v8:item>".len();
        assert!(
            xml.contains(&SCHEMA[start..end]),
            "the kept translation keeps its bytes\n{xml}"
        );
    }

    /// The default language is read only for a plain-string text: other DCS
    /// edits do not depend on it, a title does and is refused without it.
    #[test]
    fn dcs_edits_without_text_do_not_read_the_default_language() {
        let fixture = ApplySeamFixture::new();
        std::fs::remove_file(fixture.source_dir().join("Languages/Русский.xml")).unwrap();
        let data_set = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
        let xml = plan_on_versions_schema(
            &fixture,
            &[(
                "field.add",
                json!({"at": data_set, "items": [{"dataPath": "Автор", "title": {"en": "Author"}}]}),
            )],
        )
        .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert_eq!(
            translations(&xml, "field", "dataPath", "Автор", "title"),
            [("en".to_string(), "Author".to_string())],
            "an explicit translation needs no default language"
        );
        let error = plan_on_versions_schema(
            &fixture,
            &[(
                "field.set",
                json!({"at": format!("{data_set}.Field.Количество"), "values": {"title": "Count"}}),
            )],
        )
        .expect_err("a plain-string title needs the default language");
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{error:?}");
        assert_eq!(error.path(), Some("ops[0].args.at"));
    }

    #[test]
    fn typed_dcs_role_changes_the_existing_field_instead_of_a_skipped_renamed_target() {
        let fixture = ApplySeamFixture::new();
        let template_dir = fixture
            .source_dir()
            .join("Reports/Versions/Templates/Schema/Ext");
        std::fs::create_dir_all(&template_dir).unwrap();
        std::fs::write(template_dir.join("Template.xml"), SCHEMA.as_bytes()).unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let operation = parse_dcs_mxl_plan_operation(
            "fieldRole.set",
            &json!({
                "at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.РазмерДанных",
                "values":{"role":{"dimension":true}}
            }),
            0,
            &fixture.binding,
        )
        .unwrap();
        let (staged, effects) = plan_dcs_mxl_batch_owned_for_test(
            staged,
            authority,
            &[IndexedPlanOperation::new(0, operation)],
        )
        .unwrap();
        assert_eq!(
            effects.len(),
            1,
            "a success must change the named existing field"
        );
        let changes = staged.planned_changes();
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current
        else {
            panic!("expected XML bytes");
        };
        let xml = std::str::from_utf8(bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(xml).unwrap();
        let namespace = unica_dcs_designer::schema::DCS_COMMON_NS;
        let field = document
            .descendants()
            .find(|node| {
                node.tag_name().name() == "field"
                    && node.children().any(|child| {
                        child.tag_name().name() == "dataPath"
                            && child.text() == Some("РазмерДанных")
                    })
            })
            .unwrap();
        assert!(field
            .descendants()
            .any(|node| node.tag_name().namespace() == Some(namespace)
                && node.tag_name().name() == "dimension"
                && node.text() == Some("true")));
    }

    /// Поле с составным `dataPath` адресуется `Field.A\.B` (#1356):
    /// `field.set` и `fieldRole.set` по этому адресу меняют именно его,
    /// а не поле `A` и не несуществующую ветвь `B`.
    #[test]
    fn typed_dcs_field_set_changes_the_composite_field_by_its_escaped_address() {
        let fixture = ApplySeamFixture::new();
        let template_dir = fixture
            .source_dir()
            .join("Reports/Versions/Templates/Schema/Ext");
        std::fs::create_dir_all(&template_dir).unwrap();
        let composite = "<field xsi:type=\"DataSetFieldField\">\n\t\t\t<dataPath>ТипОбъекта.Представление</dataPath>\n\t\t\t<field>ТипОбъекта.Представление</field>\n\t\t</field>\n\t\t<field xsi:type=\"DataSetFieldField\">";
        std::fs::write(
            template_dir.join("Template.xml"),
            SCHEMA
                .replacen("<field xsi:type=\"DataSetFieldField\">", composite, 1)
                .as_bytes(),
        )
        .unwrap();
        let admission = fixture.admission();
        let staged = admission.staged_state().unwrap();
        let authority = admission
            .dcs_mxl_planning_authority(&fixture.binding)
            .unwrap();
        let at = r"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.ТипОбъекта\.Представление";
        let operations = [
            IndexedPlanOperation::new(
                0,
                parse_dcs_mxl_plan_operation(
                    "field.set",
                    &json!({"at": at, "values": {"title": "Тип (представление)"}}),
                    0,
                    &fixture.binding,
                )
                .unwrap(),
            ),
            IndexedPlanOperation::new(
                1,
                parse_dcs_mxl_plan_operation(
                    "fieldRole.set",
                    &json!({"at": at, "values": {"role": {"dimension": true}}}),
                    1,
                    &fixture.binding,
                )
                .unwrap(),
            ),
        ];
        let (staged, effects) = plan_dcs_mxl_batch_owned_for_test(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert!(!effects.is_empty());
        let changes = staged.planned_changes();
        let unica_source_contract::proposal::StagedFileState::Bytes(bytes) = &changes[0].current
        else {
            panic!("expected XML bytes");
        };
        let xml = std::str::from_utf8(bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(xml).unwrap();
        let field_by_path = |path: &str| {
            document
                .descendants()
                .find(|node| {
                    node.tag_name().name() == "field"
                        && node.children().any(|child| {
                            child.tag_name().name() == "dataPath" && child.text() == Some(path)
                        })
                })
                .unwrap_or_else(|| panic!("field {path}"))
        };
        let changed = field_by_path("ТипОбъекта.Представление");
        assert!(changed
            .descendants()
            .any(|node| node.text() == Some("Тип (представление)")));
        let namespace = unica_dcs_designer::schema::DCS_COMMON_NS;
        assert!(changed
            .descendants()
            .any(|node| node.tag_name().namespace() == Some(namespace)
                && node.tag_name().name() == "dimension"
                && node.text() == Some("true")));
        // Соседнее поле `ТипОбъекта` не тронуто.
        let neighbour = field_by_path("ТипОбъекта");
        assert!(!neighbour
            .descendants()
            .any(|node| node.text() == Some("Тип (представление)")
                || node.tag_name().name() == "dimension"));
    }

    #[test]
    fn typed_dcs_aliases_have_one_owner_and_preserve_primary_priority() {
        let fixture = ApplySeamFixture::new();
        let at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1";
        let query_at = "main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Query";
        let field = |item| {
            parse_dcs_mxl_plan_operation(
                "field.add",
                &json!({"at":at,"items":[item]}),
                0,
                &fixture.binding,
            )
        };
        for item in [
            json!({"name":"Amount"}),
            json!({"dataPath":"Amount","name":""}),
            json!({"dataPath":"Amount","name":"Wrong"}),
        ] {
            let plan = field(item).unwrap();
            let DcsMxlPlanKind::Dcs(edit) = plan.kind else {
                panic!("DCS operation")
            };
            let super::DcsIntent::FieldAdd(items) = edit.intent else {
                panic!("field.add keeps its typed intent");
            };
            assert_eq!(items[0].data_path, "Amount");
        }
        assert!(field(json!({"dataPath":"","name":"Amount"})).is_err());
        let query = |values| {
            parse_dcs_mxl_plan_operation(
                "query.set",
                &json!({"at":query_at,"values":values}),
                0,
                &fixture.binding,
            )
        };
        for values in [
            json!({"text":"ВЫБРАТЬ 1"}),
            json!({"query":"ВЫБРАТЬ 1","text":""}),
            json!({"query":"ВЫБРАТЬ 1","text":"ВЫБРАТЬ 2"}),
        ] {
            let plan = query(values).unwrap();
            let DcsMxlPlanKind::Dcs(edit) = plan.kind else {
                panic!("DCS operation")
            };
            let super::DcsIntent::QuerySet(query) = edit.intent else {
                panic!("query.set keeps its typed intent");
            };
            assert_eq!(query, "ВЫБРАТЬ 1");
        }
        assert!(query(json!({"query":"","text":"ВЫБРАТЬ 1"})).is_err());
    }

    #[test]
    fn typed_dcs_role_preserves_every_supported_key_and_replaces_previous_entries() {
        for (role, expected) in [
            ("periodNumber=2", vec![("periodNumber", "2")]),
            ("periodType=Main", vec![("periodType", "Main")]),
            ("dimension=false", vec![("dimension", "false")]),
            (
                "parentDimension=Parent",
                vec![("parentDimension", "Parent")],
            ),
            ("account=true", vec![("account", "true")]),
            (
                "accountTypeExpression=Type",
                vec![("accountTypeExpression", "Type")],
            ),
            ("balance=true", vec![("balance", "true")]),
            (
                "balanceGroupName=Amount",
                vec![("balanceGroupName", "Amount")],
            ),
            (
                "balanceType=OpeningBalance",
                vec![("balanceType", "OpeningBalance")],
            ),
            (
                "accountingBalanceType=Debit",
                vec![("accountingBalanceType", "Debit")],
            ),
            ("accountField=Account", vec![("accountField", "Account")]),
            ("ignoreNullValues=true", vec![("ignoreNullValues", "true")]),
            ("required=true", vec![("required", "true")]),
            (
                "dimensionAttribute=true",
                vec![("dimensionAttribute", "true")],
            ),
            (
                "dimension=true parentDimension=Parent",
                vec![("dimension", "true"), ("parentDimension", "Parent")],
            ),
        ] {
            let fixture = ApplySeamFixture::new();
            let template_dir = fixture
                .source_dir()
                .join("Reports/Versions/Templates/Schema/Ext");
            std::fs::create_dir_all(&template_dir).unwrap();
            let mut original = SCHEMA.to_string();
            let document =
                roxmltree::Document::parse(original.trim_start_matches('\u{feff}')).unwrap();
            let field = document
                .descendants()
                .find(|node| {
                    node.children().any(|child| {
                        child.tag_name().name() == "dataPath"
                            && child.text() == Some("РазмерДанных")
                    })
                })
                .unwrap();
            let insert = field
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == "appearance")
                .unwrap()
                .range()
                .start
                + usize::from(original.starts_with('\u{feff}')) * 3;
            original.insert_str(
                insert,
                "<role><dcscom:required>false</dcscom:required></role>",
            );
            let role_args = expected
                .iter()
                .map(|(key, value)| {
                    (
                        (*key).to_string(),
                        if matches!(*value, "true" | "false") {
                            json!(*value == "true")
                        } else {
                            json!(value)
                        },
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            std::fs::write(template_dir.join("Template.xml"), original.as_bytes()).unwrap();
            let admission = fixture.admission();
            let staged = admission.staged_state().unwrap();
            let authority = admission
                .dcs_mxl_planning_authority(&fixture.binding)
                .unwrap();
            let operation=parse_dcs_mxl_plan_operation("fieldRole.set",&json!({"at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1.Field.РазмерДанных","values":{"role":role_args}}),0,&fixture.binding).unwrap();
            let (staged, effects) = plan_dcs_mxl_batch_owned_for_test(
                staged,
                authority,
                &[IndexedPlanOperation::new(0, operation)],
            )
            .unwrap();
            assert_eq!(effects.len(), 1, "{role}");
            let changes = staged.planned_changes();
            let unica_source_contract::proposal::StagedFileState::Bytes(bytes) =
                &changes[0].current
            else {
                panic!("XML bytes")
            };
            let document = roxmltree::Document::parse(
                std::str::from_utf8(bytes)
                    .unwrap()
                    .trim_start_matches('\u{feff}'),
            )
            .unwrap();
            let field = document
                .descendants()
                .find(|node| {
                    node.tag_name().name() == "field"
                        && node.children().any(|child| {
                            child.tag_name().name() == "dataPath"
                                && child.text() == Some("РазмерДанных")
                        })
                })
                .unwrap();
            let entries: Vec<_> = field
                .descendants()
                .filter(|node| {
                    node.is_element()
                        && node.tag_name().namespace()
                            == Some(unica_dcs_designer::schema::DCS_COMMON_NS)
                })
                .map(|node| (node.tag_name().name(), node.text().unwrap_or_default()))
                .collect();
            assert_eq!(entries, expected, "{role}");
        }
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
    fn full_schema_input_is_absent_from_the_closed_registry() {
        assert!(crate::domain::apply::operation_registry()
            .lookup("dcs.set")
            .is_none());
        assert!(unica_dcs::operations::Primitive::named("dcs.set").is_none());
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
    fn typed_dcs_operands_do_not_pass_through_editor_tokens() {
        let fixture = ApplySeamFixture::new();
        let operation=parse_dcs_mxl_plan_operation("field.add",&json!({"at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1","items":[{"dataPath":"Sum","title":"Итого [шт] @user"}]}),0,&fixture.binding).unwrap();
        let DcsMxlPlanKind::Dcs(edit) = operation.kind else {
            panic!("DCS")
        };
        let super::DcsIntent::FieldAdd(items) = edit.intent else {
            panic!("field.add keeps its typed intent");
        };
        assert_eq!(
            items[0].title.as_ref().unwrap().value(),
            &json!("Итого [шт] @user")
        );
    }
}

#[cfg(test)]
#[path = "dcs_mxl_guard_tests.rs"]
mod guard_tests;
