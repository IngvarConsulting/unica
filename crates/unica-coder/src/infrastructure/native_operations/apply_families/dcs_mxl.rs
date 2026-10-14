use super::metadata::qualified_target;
use super::validate_platform_xml_binding;
use crate::domain::address::{member_address, NodeKind, QualifiedAddress};
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

/// A normalized basic operation and its captured logical target.
#[derive(Debug, Clone)]
struct DcsPrimitivePlan {
    template: MetadataAddress,
    operation: crate::infrastructure::native_operations::dcs_primitives::Primitive,
    args: Map<String, Value>,
    target: crate::infrastructure::native_operations::dcs_primitives::Target,
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
    /// Квалифицированный адрес макета для текста отказа.
    template_at: String,
    area: String,
    /// `area.add` создаёт область, `mxl.set` пишет только в существующую.
    creates: bool,
    /// Путь аргументов области: `values` или `items[i]`.
    location: String,
    columns: Option<i64>,
    cells: Vec<MxlCellWrite>,
}

#[derive(Debug, Clone)]
enum DcsMxlPlanKind {
    Dcs(DcsPrimitivePlan),
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
}

/// The DCS parts of a logical address: the template owner, plus the dataset,
/// settings variant and its structure items, or the field, parameter, data
/// source, calculated field or total the address descends into, and whether
/// it ends at the dataset's unnamed `Query`.
struct DcsTarget {
    template: MetadataAddress,
    datasets: Vec<String>,
    variant: String,
    structure: Vec<String>,
    terminal: Option<(NodeKind, String)>,
    query: bool,
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
            "DCS operations address a schema template: `Owner.Name.Template.Name[.DataSet.Name[.Query]|.Setting.Name[.Item.Name…]|.DataSource.Name|.CalculatedField.Name|.Total.Name]`",
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
    let mut datasets = Vec::new();
    let mut variant = String::new();
    let mut structure = Vec::new();
    let mut terminal = None;
    let mut query = false;
    for segment in rest {
        if terminal.is_some() || query {
            return Err(bad("a DCS terminal has no addressed children"));
        }
        if segment.kind() == NodeKind::Query {
            // Запрос — узел набора без имени: `DataSet.D.Query`. Прежний
            // псевдоним набора `Template.T.Query.D` не принимается (#1353).
            match segment.name() {
                None if !datasets.is_empty() && variant.is_empty() => {
                    query = true;
                    continue;
                }
                Some(name) => {
                    return Err(bad(&format!(
                    "a query is addressed through its dataset: `Template.T.DataSet.{name}.Query`"
                )))
                }
                None => {
                    return Err(bad(
                        "a query is addressed through its dataset: `DataSet.D.Query`",
                    ))
                }
            }
        }
        let name = segment
            .name()
            .ok_or_else(|| bad("DCS address segments below the template must be named"))?
            .to_string();
        let at_schema = datasets.is_empty() && variant.is_empty();
        match segment.kind() {
            NodeKind::DataSet if variant.is_empty() => datasets.push(name),
            NodeKind::Setting if at_schema => variant = name,
            // Элемент структуры варианта: цепочка `Item.<имя>` (#1357).
            NodeKind::Item if !variant.is_empty() => structure.push(name),
            NodeKind::Field if !datasets.is_empty() && variant.is_empty() => {
                terminal = Some((NodeKind::Field, name))
            }
            NodeKind::Parameter if variant.is_empty() => {
                terminal = Some((NodeKind::Parameter, name))
            }
            kind @ (NodeKind::DataSource | NodeKind::CalculatedField | NodeKind::Total)
                if at_schema =>
            {
                terminal = Some((kind, name))
            }
            _ => return Err(bad(
                "DCS address must select a Template, nested DataSet, Setting with its Item chain, or terminal Field/Parameter/DataSource/CalculatedField/Total",
            )),
        }
    }
    Ok(DcsTarget {
        template,
        datasets,
        variant,
        structure,
        terminal,
        query,
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

pub(crate) fn parse_dcs_mxl_plan_operation(
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
    let kind = match crate::application::v13::apply::dispatch_family(operation) {
        Some(crate::domain::apply::OperationFamily::Dcs) => {
            parse_dcs_operation(operation, object, op_index, binding)?
        }
        Some(crate::domain::apply::OperationFamily::Mxl)
            if matches!(operation, "mxl.set" | "area.add") =>
        {
            parse_mxl_operation(operation, object, op_index, binding)?
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
    let Some(contract) = crate::domain::operation_contract::OperationContract::dcs(operation)
    else {
        return Ok(DcsMxlPlanKind::Unsupported);
    };
    let normalized = contract
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("{}: {}", error.path, error.message),
            )
            .at_path(error.path)
        })?;
    let args = normalized
        .as_object()
        .expect("the DCS contract requires object arguments");
    let address = qualified_target(args, op_index, binding)?;
    let target = dcs_target(&address, op_index)?;
    if !target.query && matches!(operation, "query.set" | "query.patch") {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{operation} addresses the dataset's query: `DataSet.D.Query`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    if target.query && !matches!(operation, "query.set" | "query.patch") {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{operation} addresses the dataset, not its Query"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    // Операции подузла исполняются по его адресу (#1354): поле, параметр
    // или вариант называет адрес, а не аргумент.
    if matches!(operation, "variant.set" | "variant.remove")
        && (target.variant.is_empty() || target.terminal.is_some() || !target.structure.is_empty())
    {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{operation} addresses a settings variant: `Template.T.Setting.S`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    // Элемент структуры правится и удаляется по своему адресу; новый
    // элемент добавляется в контейнер, который называет адрес (#1357).
    if matches!(operation, "structure.patch" | "structure.remove") && target.structure.is_empty() {
        return Err(ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!("{operation} addresses a named structure item: `Template.T.Setting.S.Item.I`"),
        )
        .at_path(format!("ops[{op_index}].args.at")));
    }
    let terminal_kind = match operation {
        "field.remove" | "field.set" | "fieldRole.set" => Some(NodeKind::Field),
        "parameter.remove" | "parameter.set" | "parameter.rename" => Some(NodeKind::Parameter),
        "dataSource.set" | "dataSource.remove" => Some(NodeKind::DataSource),
        "calculatedField.set" | "calculatedField.remove" => Some(NodeKind::CalculatedField),
        "total.set" | "total.remove" => Some(NodeKind::Total),
        _ => None,
    };
    if let Some(expected) = terminal_kind {
        if !target
            .terminal
            .as_ref()
            .is_some_and(|(kind, _)| *kind == expected)
        {
            return Err(ApplyPlanError::new(
                ApplyPlanErrorKind::BadValue,
                format!("address a named {}", expected.as_str()),
            )
            .at_path(format!("ops[{op_index}].args.at")));
        }
    }
    let operation =
        crate::infrastructure::native_operations::dcs_primitives::Primitive::parse(operation)
            .expect("every DCS contract has a primitive");
    Ok(DcsMxlPlanKind::Dcs(DcsPrimitivePlan {
        template: target.template,
        operation,
        args: args.clone(),
        target: crate::infrastructure::native_operations::dcs_primitives::Target {
            datasets: target.datasets,
            variant: target.variant,
            structure: target.structure,
            terminal: target.terminal.map(|(_, name)| name),
        },
    }))
}

use crate::domain::operation_contract::parse_mxl_cell_address as parse_cell_address;

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
    let root = document.root_element();
    let refusal = |reason: &str| {
        ApplyPlanError::new(
            ApplyPlanErrorKind::InvalidSource,
            format!("the cell editor cannot preserve {reason}; edit this template in the Designer"),
        )
        .at_path(at_path.to_string())
    };
    fn child<'a, 'input>(
        node: roxmltree::Node<'a, 'input>,
        name: &str,
    ) -> Option<roxmltree::Node<'a, 'input>> {
        node.children()
            .find(|item| item.is_element() && item.tag_name().name() == name)
    }
    let columns: Vec<_> = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "columns")
        .collect();
    if columns.len() > 1
        || root
            .descendants()
            .any(|node| node.is_element() && node.tag_name().name() == "columnsID")
        || columns.iter().any(|node| child(*node, "id").is_some())
    {
        return Err(refusal("multiple or identified column sets"));
    }
    if let Some(settings) = child(root, "languageSettings") {
        if ["currentLanguage", "defaultLanguage"].iter().any(|name| {
            child(settings, name)
                .and_then(|node| node.text())
                .is_some_and(|text| text.trim() != "ru")
        }) || settings
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "languageInfo")
            .any(|info| child(info, "id").and_then(|node| node.text()) != Some("ru"))
        {
            return Err(refusal("non-Russian language settings"));
        }
    }
    for node in root.descendants().filter(|node| node.is_element()) {
        let translations: Vec<_> = node
            .children()
            .filter(|item| {
                item.is_element()
                    && item.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
                    && item.tag_name().name() == "item"
            })
            .collect();
        if translations.len() > 1
            || translations.iter().any(|item| {
                item.children()
                    .find(|part| part.is_element() && part.tag_name().name() == "lang")
                    .and_then(|part| part.text())
                    != Some("ru")
            })
        {
            return Err(refusal("multilingual cell text or format translations"));
        }
    }
    let rows: Vec<_> = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "rowsItem")
        .collect();
    if crate::infrastructure::native_operations::mxl::is_platform_empty_mxl_sentinel(root, &rows) {
        return Ok(());
    }
    let integer = |node, name| {
        child(node, name)
            .and_then(|item| item.text())
            .and_then(|text| text.trim().parse::<i64>().ok())
    };
    let mut end = 0i64;
    for named in root
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "namedItem")
    {
        let area = child(named, "area").ok_or_else(|| refusal("an incomplete named area"))?;
        let begin = integer(area, "beginRow").ok_or_else(|| refusal("an invalid area boundary"))?;
        let last = integer(area, "endRow").ok_or_else(|| refusal("an invalid area boundary"))?;
        if begin != end || last < begin {
            return Err(refusal(
                "outside rows, overlapping areas or a different physical area order",
            ));
        }
        end = last
            .checked_add(1)
            .ok_or_else(|| refusal("an overflowing area boundary"))?;
    }
    if rows.iter().any(|row| {
        let Some(index) = integer(*row, "index") else {
            return true;
        };
        let last = if child(*row, "indexTo").is_some() {
            integer(*row, "indexTo")
        } else {
            Some(index)
        };
        !last.is_some_and(|last| index >= 0 && last >= index && last < end)
    }) {
        return Err(refusal("rows outside the named areas"));
    }
    Ok(())
}

/// Ячейки области из контракта: строка и колонка с 1, по порядку.
fn mxl_cells(area: &Map<String, Value>) -> Vec<MxlCellWrite> {
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
        cells.push(MxlCellWrite { row, col, text });
    }
    cells.sort_by_key(|cell| (cell.row, cell.col));
    cells
}

/// `mxl.set` пишет ячейки существующей области `Template.T.Area.A`;
/// `area.add` на самом макете создаёт новые области с начальными ячейками.
/// Узел создаёт операция родителя: опечатка в имени области не заводит
/// новую (#1354).
fn parse_mxl_operation(
    operation: &str,
    args: &Map<String, Value>,
    op_index: usize,
    binding: &ProviderRootBinding,
) -> Result<DcsMxlPlanKind, ApplyPlanError> {
    let normalized = crate::domain::operation_contract::OperationContract::mxl(operation)
        .expect("MXL contract")
        .normalize(
            &Value::Object(args.clone()),
            &format!("ops[{op_index}].args"),
        )
        .map_err(|error| {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.message).at_path(error.path)
        })?;
    let args = normalized.as_object().expect("object contract");
    let address = qualified_target(args, op_index, binding)?;
    let at_path = format!("ops[{op_index}].args.at");
    let segments = address.segments();
    let shape = if operation == "mxl.set" {
        "mxl.set addresses an existing area of a spreadsheet template: `Owner.Name.Template.T.Area.A`"
    } else {
        "area.add addresses the spreadsheet template itself: `Owner.Name.Template.T`"
    };
    let (template_segments, area) = match (operation, segments) {
        ("mxl.set", [owner, template, area])
            if area.kind() == NodeKind::Area && area.name().is_some() =>
        {
            (vec![owner.clone(), template.clone()], area.name())
        }
        ("area.add", [owner, template]) => (vec![owner.clone(), template.clone()], None),
        _ => return Err(ApplyPlanError::new(ApplyPlanErrorKind::BadValue, shape).at_path(at_path)),
    };
    let template_address = QualifiedAddress {
        source_set: address.source_set().to_string(),
        segments: template_segments,
    };
    let target = dcs_target(&template_address, op_index)?;
    let edits = match area {
        Some(area) => {
            let values = required_values(args, op_index)?;
            vec![MxlEdit {
                template: target.template,
                template_at: template_address.to_string(),
                area: area.to_string(),
                creates: false,
                location: format!("ops[{op_index}].args.values"),
                columns: values.get("columns").and_then(Value::as_i64),
                cells: mxl_cells(values),
            }]
        }
        None => {
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
                    creates: true,
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

/// Есть ли именованная область в стадированном табличном документе.
/// `None` — документ не читается.
pub(super) fn spreadsheet_area_exists(bytes: &[u8], name: &str) -> Option<bool> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    Some(
        document
            .root_element()
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "namedItem")
            .any(|node| {
                node.children().any(|part| {
                    part.is_element()
                        && part.tag_name().name() == "name"
                        && part.text().map(str::trim) == Some(name)
                })
            }),
    )
}

/// Why a cell write did not apply: the area's existence contradicts the
/// operation, or the decompiled definition is malformed.
#[derive(Debug)]
enum MxlEditError {
    AreaExists,
    AreaMissing,
    Invalid(String),
}

impl From<String> for MxlEditError {
    fn from(message: String) -> Self {
        Self::Invalid(message)
    }
}

/// Applies the cell writes to a decompiled definition: `mxl.set` finds the
/// existing area and `area.add` appends a new one, compressed empty rows are
/// expanded, cells are set by column.
fn apply_mxl_edit(definition: &mut Value, edit: &MxlEdit) -> Result<(), MxlEditError> {
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
    let index = match (
        areas
            .iter()
            .position(|area| area.get("name").and_then(Value::as_str) == Some(edit.area.as_str())),
        edit.creates,
    ) {
        (Some(index), false) => index,
        (None, true) => {
            areas.push(serde_json::json!({"name": edit.area, "rows": []}));
            areas.len() - 1
        }
        (Some(_), true) => return Err(MxlEditError::AreaExists),
        (None, false) => return Err(MxlEditError::AreaMissing),
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
        require_template_format(&mut staged, &authority, operation.operation(), op_index)?;
        let edit = match &operation.operation().kind {
            DcsMxlPlanKind::Dcs(edit) => edit,
            DcsMxlPlanKind::Mxl(edits) => {
                for edit in edits {
                    stage_mxl_edit(&mut staged, &authority, edit, op_index, &mut provisional)?;
                }
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
        let original = xml_text.clone();
        apply_dcs_edit(&mut staged, &authority, &mut xml_text, edit, op_index)?;
        if xml_text == original {
            continue;
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

/// The template format filter of the canonical path, read from the staged
/// template descriptor: a DCS operation applies to a data composition schema
/// and `mxl.set` to a spreadsheet only. The filter is the registry's
/// (`OperationDescriptor::admits_template_format`), the one `can` uses; reading
/// the staged state keeps it right for a template added earlier in the same
/// request.
fn require_template_format(
    staged: &mut ApplyStagedState,
    authority: &DcsMxlApplyAuthority<'_>,
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
    let Some(descriptor) =
        crate::domain::apply::OperationRegistry::closed().lookup(&operation.operation)
    else {
        return Ok(());
    };
    let at_path = format!("ops[{op_index}].args.at");
    let relative = crate::infrastructure::logical_event_source::metadata_descriptor_relative(
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
    let format =
        template_type_of(&bytes).map(|raw| crate::domain::apply::TemplateFormat::parse(&raw));
    if descriptor.admits_template_format(format) {
        return Ok(());
    }
    Err(ApplyPlanError::new(
        ApplyPlanErrorKind::BadValue,
        format!(
            "operation `{}` does not apply to the template `{}` of format `{}`",
            operation.operation,
            template.as_str(),
            format.map_or("unknown", crate::domain::apply::TemplateFormat::as_str)
        ),
    )
    .at_path(at_path))
}

/// Путь набора с именем `name` в теле схемы: `["A", "B"]` для члена
/// объединения `A`; несколько наборов с этим именем — все их пути.
pub(super) fn data_set_path_in_schema(
    bytes: &[u8],
    name: &str,
) -> crate::domain::apply::NamedLookup {
    use crate::domain::apply::NamedLookup;
    use crate::infrastructure::native_operations::dcs::DCS_SCHEMA_NS as S;
    fn walk(
        node: roxmltree::Node<'_, '_>,
        tag: &str,
        name: &str,
        path: &mut Vec<String>,
        found: &mut Vec<Vec<String>>,
    ) {
        for child in node
            .children()
            .filter(|child| child.is_element() && child.has_tag_name((S, tag)))
        {
            let Some(own) = child
                .children()
                .find(|item| item.is_element() && item.has_tag_name((S, "name")))
                .and_then(|item| item.text())
                .map(str::trim)
            else {
                continue;
            };
            path.push(own.to_string());
            if own == name {
                found.push(path.clone());
            }
            walk(child, "item", name, path, found);
            path.pop();
        }
    }
    let Some(document) = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .ok()
        .and_then(|text| roxmltree::Document::parse(text).ok())
    else {
        return NamedLookup::Missing;
    };
    let mut found = Vec::new();
    walk(
        document.root_element(),
        "dataSet",
        name,
        &mut Vec::new(),
        &mut found,
    );
    match found.len() {
        0 => NamedLookup::Missing,
        1 => NamedLookup::Found(found.remove(0)),
        _ => NamedLookup::Ambiguous(found),
    }
}

/// Путь элемента структуры варианта `variant` с именем `name`: имена
/// именованных предков и его собственное, как их адресует чтение
/// (`Setting.V.Item.A.Item.B`). Несколько элементов с этим именем — пути
/// тех, что адресуемы.
pub(super) fn structure_item_path_in_schema(
    bytes: &[u8],
    variant: &str,
    name: &str,
) -> crate::domain::apply::NamedLookup {
    use crate::domain::apply::NamedLookup;
    let Some(data) = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes))
        .ok()
        .and_then(|text| {
            crate::infrastructure::native_operations::dcs::parse_dcs_info_xml(
                text,
                crate::domain::support_state::ObjectSupportData {
                    state: crate::domain::support_state::ObjectSupportState::NotSupported,
                    direct_edit_safe: None,
                },
            )
            .ok()
        })
    else {
        return NamedLookup::Missing;
    };
    let Some(rows) = data
        .variants
        .iter()
        .find(|candidate| candidate.name == variant)
        .and_then(|variant| serde_json::to_value(&variant.structure_items).ok())
    else {
        return NamedLookup::Missing;
    };
    let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
    let (mut paths, found) =
        crate::infrastructure::v13_read_projection::dcs_structure_paths(rows, name);
    match (found, paths.len()) {
        (0, _) => NamedLookup::Missing,
        (1, 1) => NamedLookup::Found(paths.remove(0)),
        _ => NamedLookup::Ambiguous(paths),
    }
}

/// `TemplateType` of a template descriptor.
pub(super) fn template_type_of(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).ok()?;
    let document = roxmltree::Document::parse(text).ok()?;
    document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "TemplateType")
        .and_then(|node| node.text())
        .map(|text| text.trim().to_string())
}

/// Applies one DCS edit to `xml_text`. The default language of the source
/// set is read only when the edit writes a plain-string multilingual value,
/// so edits without such text do not depend on `Configuration.xml` (#1257).
fn apply_dcs_edit(
    staged: &mut ApplyStagedState,
    authority: &DcsMxlApplyAuthority<'_>,
    xml_text: &mut String,
    edit: &DcsPrimitivePlan,
    op_index: usize,
) -> Result<(), ApplyPlanError> {
    use crate::infrastructure::native_operations::dcs_primitives::{apply, ApplyError};
    let apply_with = |xml_text: &mut String, lang: Option<&str>| {
        apply(
            xml_text,
            edit.operation.as_str(),
            &edit.args,
            &edit.target,
            lang,
        )
    };
    let result = match apply_with(xml_text, None) {
        Err(ApplyError::TextLanguageRequired) => {
            let lang =
                super::metadata::staged_text_language(staged, authority.source_kind(), op_index)?;
            apply_with(xml_text, Some(&lang))
        }
        result => result,
    };
    result.map_err(|error| {
        ApplyPlanError::new(ApplyPlanErrorKind::BadValue, error.to_string())
            .at_path(format!("ops[{op_index}].args"))
    })
}

fn stage_mxl_edit(
    staged: &mut ApplyStagedState,
    authority: &DcsMxlApplyAuthority<'_>,
    edit: &MxlEdit,
    op_index: usize,
    provisional: &mut Vec<ProvisionalApplyEffect>,
) -> Result<(), ApplyPlanError> {
    let at_path = format!("ops[{op_index}].args.at");
    let values_path = edit.location.clone();
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
    apply_mxl_edit(&mut definition, edit).map_err(|error| match error {
        MxlEditError::AreaMissing => ApplyPlanError::new(
            ApplyPlanErrorKind::NotFound,
            format!(
                "the template has no area `{}`; create it with area.add at `{}`",
                edit.area, edit.template_at
            ),
        )
        .at_path(at_path.clone()),
        MxlEditError::AreaExists => ApplyPlanError::new(
            ApplyPlanErrorKind::BadValue,
            format!(
                "the template already has area `{}`; change its cells with mxl.set at `{}`",
                edit.area,
                member_address(&format!("{}.Area", edit.template_at), &edit.area)
            ),
        )
        .at_path(format!("{values_path}.name")),
        MxlEditError::Invalid(message) => {
            ApplyPlanError::new(ApplyPlanErrorKind::BadValue, message).at_path(values_path.clone())
        }
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
    use super::DcsMxlPlanKind;
    use super::{
        parse_cell_address, parse_dcs_mxl_plan_operation, plan_dcs_mxl_batch,
        require_editable_spreadsheet,
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
    fn mxl_editor_refuses_proven_lossy_writer_shapes_before_decompilation() {
        for (name, xml) in [
            ("UnsafeOutsideRow", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeOutsideRow/Ext/Template.xml")),
            ("UnsafeOverlappingAreas", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeOverlappingAreas/Ext/Template.xml")),
            ("UnsafePhysicalAreaOrder", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafePhysicalAreaOrder/Ext/Template.xml")),
            ("UnsafeCellTranslations", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeCellTranslations/Ext/Template.xml")),
            ("UnsafeMultipleColumnSets", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeMultipleColumnSets/Ext/Template.xml")),
            ("UnsafeColumnsId", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeColumnsId/Ext/Template.xml")),
            ("UnsafeLanguageSettings", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeLanguageSettings/Ext/Template.xml")),
            ("UnsafeDrawing", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeDrawing/Ext/Template.xml")),
            ("UnsafeColumnArea", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeColumnArea/Ext/Template.xml")),
            ("UnsafeFormatTranslations", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeFormatTranslations/Ext/Template.xml")),
            ("UnsafeCompressedOutsideRow", include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeCompressedOutsideRow/Ext/Template.xml")),
        ] {
            assert!(require_editable_spreadsheet(xml, "ops[0].args.at").is_err(), "{name} would lose untouched content");
        }
        let inside = include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/UnsafeCompressedOutsideRow/Ext/Template.xml")
            .replace("<indexTo>4</indexTo>", "<indexTo>1</indexTo>");
        assert!(require_editable_spreadsheet(&inside, "ops[0].args.at").is_ok());
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
        let path = |name| super::data_set_path_in_schema(schema.as_bytes(), name);
        use crate::domain::apply::NamedLookup;
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
        use crate::domain::apply::NamedLookup;
        let path = |name| super::structure_item_path_in_schema(schema.as_bytes(), "Main", name);
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
            super::structure_item_path_in_schema(schema.as_bytes(), "Other", "Top"),
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
    ) -> Result<String, crate::infrastructure::native_operations::apply::ApplyPlanError> {
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
        let (staged, _) = plan_dcs_mxl_batch(staged, authority, &operations)?;
        let changes = staged.planned_changes();
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            &changes[0].current
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
        let (staged, effects) = plan_dcs_mxl_batch(
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
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            &changes[0].current
        else {
            panic!("expected XML bytes");
        };
        let xml = std::str::from_utf8(bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(xml).unwrap();
        let namespace = crate::infrastructure::native_operations::dcs::DCS_COMMON_NS;
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
        let (staged, effects) = plan_dcs_mxl_batch(staged, authority, &operations)
            .unwrap_or_else(|error| panic!("{error:?} at {:?}", error.path()));
        assert!(!effects.is_empty());
        let changes = staged.planned_changes();
        let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
            &changes[0].current
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
        let namespace = crate::infrastructure::native_operations::dcs::DCS_COMMON_NS;
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
            assert_eq!(edit.args["items"][0]["dataPath"], json!("Amount"));
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
            assert_eq!(edit.args["values"]["query"], json!("ВЫБРАТЬ 1"));
        }
        assert!(query(json!({"query":"","text":"ВЫБРАТЬ 1"})).is_err());
    }

    #[test]
    fn typed_dcs_role_preserves_every_supported_key_and_replaces_previous_entries() {
        use crate::infrastructure::native_operations::dcs::DCS_COMMON_NS;
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
            let (staged, effects) = plan_dcs_mxl_batch(
                staged,
                authority,
                &[IndexedPlanOperation::new(0, operation)],
            )
            .unwrap();
            assert_eq!(effects.len(), 1, "{role}");
            let changes = staged.planned_changes();
            let crate::infrastructure::native_operations::apply::StagedFileState::Bytes(bytes) =
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
                    node.is_element() && node.tag_name().namespace() == Some(DCS_COMMON_NS)
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
        assert!(crate::domain::apply::OperationRegistry::closed()
            .lookup("dcs.set")
            .is_none());
        assert!(
            crate::infrastructure::native_operations::dcs_primitives::Primitive::parse("dcs.set")
                .is_none()
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
        let editable = include_str!("../../../../../../tests/fixtures/acceptance/workspace-mxl/src/cf/Reports/F06Report/Templates/F06Template/Ext/Template.xml");
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
    fn typed_dcs_operands_do_not_pass_through_editor_tokens() {
        let fixture = ApplySeamFixture::new();
        let operation=parse_dcs_mxl_plan_operation("field.add",&json!({"at":"main:Report.Versions.Template.Schema.DataSet.НаборДанных1","items":[{"dataPath":"Sum","title":"Итого [шт] @user"}]}),0,&fixture.binding).unwrap();
        let DcsMxlPlanKind::Dcs(edit) = operation.kind else {
            panic!("DCS")
        };
        assert_eq!(edit.args["items"][0]["title"], json!("Итого [шт] @user"));
    }
}

#[cfg(test)]
#[path = "dcs_mxl_guard_tests.rs"]
mod guard_tests;
