//! Local XML changes for DCS operations. Operands stay structured from the
//! operation contract to the writer; no whole-schema intermediate model is used.
use super::common::escape_xml;
use super::dcs::{
    DCS_CORE_NS as C, DCS_SCHEMA_NS as S, DCS_SETTINGS_NS as T, V8_DATA_NS as V,
    XML_SCHEMA_INSTANCE_NS as X,
};
use roxmltree::{Document, Node};
use serde_json::{Map, Value};
use std::ops::Range;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Primitive {
    DataSourceAdd,
    DataSourceSet,
    DataSourceRemove,
    DataSetAdd,
    DataSetSet,
    DataSetRemove,
    FieldAdd,
    FieldSet,
    FieldRemove,
    FieldRoleSet,
    ParameterAdd,
    ParameterSet,
    ParameterRemove,
    ParameterRename,
    ParameterReorder,
    CalculatedFieldAdd,
    CalculatedFieldRemove,
    TotalAdd,
    TotalRemove,
    VariantAdd,
    VariantSet,
    VariantRemove,
    QuerySet,
    QueryPatch,
    FilterAdd,
    FilterSet,
    FilterRemove,
    FilterClear,
    SelectionAdd,
    SelectionClear,
    OrderAdd,
    OrderClear,
    DataParameterAdd,
    DataParameterSet,
    OutputParameterSet,
    ConditionalAppearanceAdd,
    ConditionalAppearanceClear,
    StructureAdd,
    StructureSet,
    StructurePatch,
    StructureRemove,
    DataSetLinkAdd,
    CalculatedFieldSet,
    TotalSet,
    SelectionSet,
    SelectionRemove,
    OrderSet,
    OrderRemove,
    DataParameterRemove,
    OutputParameterRemove,
    ConditionalAppearanceRemove,
    DataSetLinkSet,
    DataSetLinkRemove,
}
impl Primitive {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "dataSource.add" => Self::DataSourceAdd,
            "dataSource.set" => Self::DataSourceSet,
            "dataSource.remove" => Self::DataSourceRemove,
            "dataSet.add" => Self::DataSetAdd,
            "dataSet.set" => Self::DataSetSet,
            "dataSet.remove" => Self::DataSetRemove,
            "field.add" => Self::FieldAdd,
            "field.set" => Self::FieldSet,
            "field.remove" => Self::FieldRemove,
            "fieldRole.set" => Self::FieldRoleSet,
            "parameter.add" => Self::ParameterAdd,
            "parameter.set" => Self::ParameterSet,
            "parameter.remove" => Self::ParameterRemove,
            "parameter.rename" => Self::ParameterRename,
            "parameter.reorder" => Self::ParameterReorder,
            "calculatedField.add" => Self::CalculatedFieldAdd,
            "calculatedField.remove" => Self::CalculatedFieldRemove,
            "total.add" => Self::TotalAdd,
            "total.remove" => Self::TotalRemove,
            "variant.add" => Self::VariantAdd,
            "variant.set" => Self::VariantSet,
            "variant.remove" => Self::VariantRemove,
            "query.set" => Self::QuerySet,
            "query.patch" => Self::QueryPatch,
            "filter.add" => Self::FilterAdd,
            "filter.set" => Self::FilterSet,
            "filter.remove" => Self::FilterRemove,
            "filter.clear" => Self::FilterClear,
            "selection.add" => Self::SelectionAdd,
            "selection.clear" => Self::SelectionClear,
            "order.add" => Self::OrderAdd,
            "order.clear" => Self::OrderClear,
            "dataParameter.add" => Self::DataParameterAdd,
            "dataParameter.set" => Self::DataParameterSet,
            "outputParameter.set" => Self::OutputParameterSet,
            "conditionalAppearance.add" => Self::ConditionalAppearanceAdd,
            "conditionalAppearance.clear" => Self::ConditionalAppearanceClear,
            "structure.add" => Self::StructureAdd,
            "structure.set" => Self::StructureSet,
            "structure.patch" => Self::StructurePatch,
            "structure.remove" => Self::StructureRemove,
            "dataSetLink.add" => Self::DataSetLinkAdd,
            "calculatedField.set" => Self::CalculatedFieldSet,
            "total.set" => Self::TotalSet,
            "selection.set" => Self::SelectionSet,
            "selection.remove" => Self::SelectionRemove,
            "order.set" => Self::OrderSet,
            "order.remove" => Self::OrderRemove,
            "dataParameter.remove" => Self::DataParameterRemove,
            "outputParameter.remove" => Self::OutputParameterRemove,
            "conditionalAppearance.remove" => Self::ConditionalAppearanceRemove,
            "dataSetLink.set" => Self::DataSetLinkSet,
            "dataSetLink.remove" => Self::DataSetLinkRemove,

            _ => return None,
        })
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::DataSourceAdd => "dataSource.add",
            Self::DataSourceSet => "dataSource.set",
            Self::DataSourceRemove => "dataSource.remove",
            Self::DataSetAdd => "dataSet.add",
            Self::DataSetSet => "dataSet.set",
            Self::DataSetRemove => "dataSet.remove",
            Self::FieldAdd => "field.add",
            Self::FieldSet => "field.set",
            Self::FieldRemove => "field.remove",
            Self::FieldRoleSet => "fieldRole.set",
            Self::ParameterAdd => "parameter.add",
            Self::ParameterSet => "parameter.set",
            Self::ParameterRemove => "parameter.remove",
            Self::ParameterRename => "parameter.rename",
            Self::ParameterReorder => "parameter.reorder",
            Self::CalculatedFieldAdd => "calculatedField.add",
            Self::CalculatedFieldRemove => "calculatedField.remove",
            Self::TotalAdd => "total.add",
            Self::TotalRemove => "total.remove",
            Self::VariantAdd => "variant.add",
            Self::VariantSet => "variant.set",
            Self::VariantRemove => "variant.remove",
            Self::QuerySet => "query.set",
            Self::QueryPatch => "query.patch",
            Self::FilterAdd => "filter.add",
            Self::FilterSet => "filter.set",
            Self::FilterRemove => "filter.remove",
            Self::FilterClear => "filter.clear",
            Self::SelectionAdd => "selection.add",
            Self::SelectionClear => "selection.clear",
            Self::OrderAdd => "order.add",
            Self::OrderClear => "order.clear",
            Self::DataParameterAdd => "dataParameter.add",
            Self::DataParameterSet => "dataParameter.set",
            Self::OutputParameterSet => "outputParameter.set",
            Self::ConditionalAppearanceAdd => "conditionalAppearance.add",
            Self::ConditionalAppearanceClear => "conditionalAppearance.clear",
            Self::StructureAdd => "structure.add",
            Self::StructureSet => "structure.set",
            Self::StructurePatch => "structure.patch",
            Self::StructureRemove => "structure.remove",
            Self::DataSetLinkAdd => "dataSetLink.add",
            Self::CalculatedFieldSet => "calculatedField.set",
            Self::TotalSet => "total.set",
            Self::SelectionSet => "selection.set",
            Self::SelectionRemove => "selection.remove",
            Self::OrderSet => "order.set",
            Self::OrderRemove => "order.remove",
            Self::DataParameterRemove => "dataParameter.remove",
            Self::OutputParameterRemove => "outputParameter.remove",
            Self::ConditionalAppearanceRemove => "conditionalAppearance.remove",
            Self::DataSetLinkSet => "dataSetLink.set",
            Self::DataSetLinkRemove => "dataSetLink.remove",
        }
    }
}

const ROOT_ORDER: &[&str] = &[
    "dataSource",
    "dataSet",
    "dataSetLink",
    "calculatedField",
    "totalField",
    "parameter",
    "template",
    "fieldTemplate",
    "groupTemplate",
    "nestedSchema",
    "settingsVariant",
];
const DATASET_ORDER: &[&str] = &[
    "name",
    "field",
    "item",
    "dataSource",
    "query",
    "objectName",
    "autoFillFields",
];
const FIELD_ORDER: &[&str] = &[
    "dataPath",
    "field",
    "title",
    "useRestriction",
    "attributeUseRestriction",
    "role",
    "presentationExpression",
    "orderExpression",
    "inHierarchyDataSet",
    "inHierarchyDataSetParameter",
    "valueType",
    "appearance",
    "availableValue",
    "inputParameters",
];
const PARAMETER_ORDER: &[&str] = &[
    "name",
    "title",
    "valueType",
    "value",
    "useRestriction",
    "expression",
    "availableValue",
    "valueListAllowed",
    "availableAsField",
    "denyIncompleteValues",
    "use",
];
const SETTINGS_ORDER: &[&str] = &[
    "selection",
    "filter",
    "dataParameters",
    "order",
    "conditionalAppearance",
    "outputParameters",
    "userFields",
    "item",
];
const GROUP_ORDER: &[&str] = &[
    "use",
    "name",
    "groupItems",
    "filter",
    "order",
    "selection",
    "conditionalAppearance",
    "outputParameters",
    "column",
    "row",
    "point",
    "series",
    "item",
    "viewMode",
    "userSettingID",
    "itemsViewMode",
    "userSettingPresentation",
];

fn is(node: Node<'_, '_>, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().namespace() == Some(ns) && node.tag_name().name() == name
}
fn qtype(node: Node<'_, '_>, ns: &str, name: &str) -> bool {
    super::dcs_xml::xsi_type_matches(node, ns, name)
}
fn child<'a, 'i>(parent: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    parent.children().find(|node| is(*node, ns, name))
}
fn text(parent: Node<'_, '_>, ns: &str, name: &str) -> String {
    child(parent, ns, name)
        .map(|node| {
            node.children()
                .filter(Node::is_text)
                .filter_map(|node| node.text())
                .collect()
        })
        .unwrap_or_default()
}
fn unique<'a, 'i>(
    nodes: impl Iterator<Item = Node<'a, 'i>>,
    what: &str,
) -> Result<Node<'a, 'i>, String> {
    let mut nodes = nodes;
    let node = nodes
        .next()
        .ok_or_else(|| format!("{what} was not found"))?;
    if nodes.next().is_some() {
        return Err(format!("{what} is ambiguous"));
    }
    Ok(node)
}
fn named<'a, 'i>(
    parent: Node<'a, 'i>,
    ns: &str,
    tag: &str,
    key: &str,
    name: &str,
) -> Result<Node<'a, 'i>, String> {
    unique(
        parent
            .children()
            .filter(|node| is(*node, ns, tag) && text(*node, ns, key) == name),
        name,
    )
}
fn string<'a>(object: &'a Map<String, Value>, key: &str) -> &'a str {
    object.get(key).and_then(Value::as_str).unwrap_or("")
}
fn value_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        v => v.to_string(),
    }
}
fn xml(tag: &str, body: &str, ns: &str) -> String {
    // Local declarations also work when the owner uses different prefixes or
    // a prefixed schema root. Existing declarations and nodes stay untouched.
    format!("<{tag} xmlns=\"{ns}\" xmlns:dcsset=\"{T}\" xmlns:dcscor=\"{C}\" xmlns:v8=\"{V}\" xmlns:xsi=\"{X}\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" xmlns:v8ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" xmlns:win=\"http://v8.1c.ru/8.1/data/ui/colors/windows\" xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:dcscom=\"http://v8.1c.ru/8.1/data-composition-system/common\">{body}</{tag}>")
}
fn simple(tag: &str, value: &str, ns: &str) -> String {
    xml(tag, &escape_xml(value), ns)
}
fn typed(tag: &str, kind: &str, body: &str, ns: &str) -> String {
    xml(tag, body, ns).replacen(
        &format!("<{tag} "),
        &format!("<{tag} xsi:type=\"{kind}\" "),
        1,
    )
}
fn mltext(tag: &str, value: &Value, ns: &str) -> Result<String, String> {
    let languages = match value {
        Value::String(s) => vec![("ru", s.as_str())],
        Value::Object(values) => values
            .iter()
            .map(|(lang, text)| {
                text.as_str()
                    .map(|text| (lang.as_str(), text))
                    .ok_or_else(|| "translation must be text".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err("title must be text or a language-to-text object".to_string()),
    };
    let body = languages
        .into_iter()
        .map(|(lang, content)| {
            format!(
                "<v8:item><v8:lang>{}</v8:lang><v8:content>{}</v8:content></v8:item>",
                escape_xml(lang),
                escape_xml(content)
            )
        })
        .collect::<String>();
    Ok(typed(tag, "v8:LocalStringType", &body, ns))
}
fn content(xml_text: &str, node: Node<'_, '_>) -> Result<Range<usize>, String> {
    let range = node.range();
    let mut quote = None;
    let mut open = None;
    for (offset, ch) in xml_text[range.clone()].char_indices() {
        match (quote, ch) {
            (Some(current), ch) if current == ch => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, '>') => {
                open = Some(range.start + offset + 1);
                break;
            }
            _ => {}
        }
    }
    let open = open.ok_or("malformed XML element")?;
    if xml_text[..open].ends_with("/>") {
        return Ok(open - 2..open);
    }
    let close = xml_text[range.clone()]
        .rfind("</")
        .ok_or("missing closing element")?
        + range.start;
    Ok(open..close)
}
fn insert(
    xml_text: &mut String,
    range: Range<usize>,
    fragment: &str,
    ns: &str,
    tag: &str,
    order: &[&str],
) -> Result<(), String> {
    let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
    let parent = document
        .descendants()
        .find(|node| node.is_element() && node.range() == range)
        .ok_or("parent disappeared")?;
    let inside = content(xml_text, parent)?;
    if xml_text[inside.clone()].starts_with("/>") {
        let qualified = xml_text[range.clone()]
            .trim_start_matches('<')
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '/' | '>'))
            .next()
            .ok_or("element name missing")?;
        let replacement = format!(">{fragment}</{qualified}>");
        xml_text.replace_range(inside, &replacement);
        return Ok(());
    }
    let rank = order
        .iter()
        .position(|name| *name == tag)
        .unwrap_or(order.len());
    let pos = parent
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(ns)
                && order
                    .iter()
                    .position(|name| *name == node.tag_name().name())
                    .is_some_and(|other| other > rank)
        })
        .map_or(inside.end, |node| node.range().start);
    xml_text.insert_str(pos, fragment);
    Ok(())
}
fn equal_element(left: Node<'_, '_>, right: Node<'_, '_>) -> bool {
    if left.tag_name() != right.tag_name() {
        return false;
    }
    fn attribute_value(node: Node<'_, '_>, attribute: roxmltree::Attribute<'_, '_>) -> String {
        if attribute.namespace() == Some(X) && attribute.name() == "type" {
            let (prefix, local) = attribute
                .value()
                .split_once(':')
                .map_or((None, attribute.value()), |(prefix, local)| {
                    (Some(prefix), local)
                });
            format!(
                "{}:{local}",
                node.lookup_namespace_uri(prefix).unwrap_or("")
            )
        } else {
            attribute.value().to_string()
        }
    }
    if left.attributes().len() != right.attributes().len() {
        return false;
    }
    for attribute in left.attributes() {
        let Some(other) = right.attributes().find(|other| {
            other.namespace() == attribute.namespace() && other.name() == attribute.name()
        }) else {
            return false;
        };
        if attribute_value(left, attribute) != attribute_value(right, other) {
            return false;
        }
    }
    let has_elements = left.children().any(|node| node.is_element())
        || right.children().any(|node| node.is_element());
    if !has_elements {
        let text = |node: Node<'_, '_>| {
            node.children()
                .filter(Node::is_text)
                .filter_map(|node| node.text())
                .collect::<String>()
        };
        return text(left) == text(right);
    }
    let meaningful = |node: &Node<'_, '_>| {
        node.is_element()
            || (node.is_text()
                && (!has_elements || !node.text().unwrap_or_default().trim().is_empty()))
    };
    let left = left.children().filter(meaningful).collect::<Vec<_>>();
    let right = right.children().filter(meaningful).collect::<Vec<_>>();
    left.len() == right.len()
        && left.iter().zip(right.iter()).all(|(left, right)| {
            if left.is_element() && right.is_element() {
                equal_element(*left, *right)
            } else {
                left.is_text() && right.is_text() && left.text() == right.text()
            }
        })
}
fn same_fragment(existing: Node<'_, '_>, fragment: &str) -> bool {
    Document::parse(fragment).is_ok_and(|document| equal_element(existing, document.root_element()))
}

fn upsert(
    xml_text: &mut String,
    parent: Range<usize>,
    ns: &str,
    tag: &str,
    fragment: &str,
    order: &[&str],
) -> Result<(), String> {
    let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
    let node = document
        .descendants()
        .find(|node| node.is_element() && node.range() == parent)
        .ok_or("parent disappeared")?;
    let children = node
        .children()
        .filter(|node| is(*node, ns, tag))
        .collect::<Vec<_>>();
    if children.len() > 1 {
        return Err(format!("{tag} is ambiguous"));
    }
    if let Some(existing) = children.first() {
        if same_fragment(*existing, fragment) {
            return Ok(());
        }
        let range = existing.range();
        xml_text.replace_range(range, fragment);
        Ok(())
    } else {
        insert(xml_text, parent, fragment, ns, tag, order)
    }
}
fn remove_ranges(xml_text: &mut String, mut ranges: Vec<Range<usize>>) {
    ranges.sort_by_key(|range| std::cmp::Reverse(range.start));
    for range in ranges {
        xml_text.replace_range(range, "");
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Target {
    pub(crate) datasets: Vec<String>,
    pub(crate) variant: String,
    pub(crate) terminal: Option<String>,
}
fn dataset<'a, 'i>(
    root: Node<'a, 'i>,
    target: &Target,
    values: &Map<String, Value>,
) -> Result<Node<'a, 'i>, String> {
    let path = if let Some(Value::String(name)) = values.get("dataSet") {
        vec![name.clone()]
    } else {
        target.datasets.clone()
    };
    let mut parent = root;
    if path.is_empty() {
        return Err("address an existing DataSet".to_string());
    }
    for (index, name) in path.iter().enumerate() {
        parent = named(
            parent,
            S,
            if index == 0 { "dataSet" } else { "item" },
            "name",
            name,
        )?;
    }
    Ok(parent)
}
fn settings<'a, 'i>(
    root: Node<'a, 'i>,
    target: &Target,
    values: &Map<String, Value>,
) -> Result<Node<'a, 'i>, String> {
    let name = values
        .get("variant")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&target.variant);
    if name.is_empty() {
        return Err("address an existing Setting".to_string());
    }
    let variant = unique(
        root.children()
            .filter(|node| is(*node, S, "settingsVariant") && text(*node, T, "name") == name),
        name,
    )?;
    let settings = child(variant, T, "settings").ok_or("variant has no settings")?;
    let group = string(values, "group");
    if group.is_empty() {
        return Ok(settings);
    }
    unique(
        settings.descendants().filter(|node| {
            (is(*node, T, "item")
                || ["row", "column", "point", "series"]
                    .iter()
                    .any(|tag| is(*node, T, tag)))
                && text(*node, T, "name") == group
        }),
        group,
    )
}
const UI_NS: &str = "http://v8.1c.ru/8.1/data/ui";
const CFG_NS: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XS_NS: &str = "http://www.w3.org/2001/XMLSchema";

fn output_enum_values(kind: &str) -> Option<&'static [&'static str]> {
    match kind {
        "dcsset:DataCompositionTextOutputType" => Some(&["Auto", "DontOutput", "Output"]),
        "dcsset:DataCompositionGroupFieldsPlacement" => {
            Some(&["Together", "Separately", "SeparatelyAndInTotalsOnly"])
        }
        "dcsset:DataCompositionAttributesPlacement" => Some(&[
            "Together",
            "Separately",
            "WithOwnerField",
            "SpecialPosition",
        ]),
        "dcscor:DataCompositionTotalPlacement" => {
            Some(&["None", "Begin", "End", "BeginAndEnd", "Auto"])
        }
        "dcsset:DataCompositionGroupPlacement" => Some(&["None", "Begin", "End", "BeginAndEnd"]),
        "dcsset:DataCompositionResourcesPlacement" => Some(&["Vertically", "Horizontally"]),
        "dcsset:DataCompositionGroupTemplateType" => Some(&["Vertical", "Horizontal", "Auto"]),
        _ => None,
    }
}
fn output_declared_type(name: &str) -> Option<&'static str> {
    match name {
        "Заголовок" => Some("v8:LocalStringType"),
        "МакетОформления" => Some("xs:string"),
        "ВыводитьЗаголовок" | "ВыводитьПараметрыДанных" | "ВыводитьОтбор" => {
            Some("dcsset:DataCompositionTextOutputType")
        }
        "РасположениеПолейГруппировки" => {
            Some("dcsset:DataCompositionGroupFieldsPlacement")
        }
        "РасположениеРеквизитов" => {
            Some("dcsset:DataCompositionAttributesPlacement")
        }
        "ГоризонтальноеРасположениеОбщихИтогов"
        | "ВертикальноеРасположениеОбщихИтогов"
        | "РасположениеОбщихИтогов"
        | "РасположениеИтогов" => Some("dcscor:DataCompositionTotalPlacement"),
        "РасположениеГруппировки" => {
            Some("dcsset:DataCompositionGroupPlacement")
        }
        "РасположениеРесурсов" => {
            Some("dcsset:DataCompositionResourcesPlacement")
        }
        "ТипМакета" => Some("dcsset:DataCompositionGroupTemplateType"),
        _ => None,
    }
}
fn canonical_value_type(kind: &str) -> Result<String, String> {
    let canonical = match kind {
        "string" | "String" | "xsd:string" => "xs:string",
        "decimal" | "number" | "Number" | "xsd:decimal" => "xs:decimal",
        "boolean" | "bool" | "Boolean" | "xsd:boolean" => "xs:boolean",
        "date" | "dateTime" | "DateTime" | "xsd:dateTime" => "xs:dateTime",
        "StandardPeriod" => "v8:StandardPeriod",
        "LocalStringType" | "mltext" => "v8:LocalStringType",
        "Color" => "v8ui:Color",
        kind => kind,
    };
    if matches!(
        canonical,
        "xs:string"
            | "xs:decimal"
            | "xs:boolean"
            | "xs:dateTime"
            | "v8:StandardPeriod"
            | "v8:LocalStringType"
            | "v8ui:Color"
            | "dcscor:DesignTimeValue"
    ) || output_enum_values(canonical).is_some()
    {
        return Ok(canonical.to_string());
    }
    let reference = canonical.strip_prefix("cfg:").unwrap_or(canonical);
    if reference.split_once('.').is_some_and(|(kind, _)| {
        matches!(
            kind,
            "CatalogRef"
                | "DocumentRef"
                | "EnumRef"
                | "ChartOfAccountsRef"
                | "ChartOfCharacteristicTypesRef"
                | "ChartOfCalculationTypesRef"
                | "BusinessProcessRef"
                | "TaskRef"
                | "ExchangePlanRef"
        )
    }) {
        super::dcs_xml::parse_value_type(reference)?;
        return Ok(format!("cfg:{reference}"));
    }
    Err(format!("unsupported scalar valueType {kind}"))
}
fn canonical_node_value_type(node: Node<'_, '_>, qname: &str) -> Result<String, String> {
    let (prefix, local) = qname
        .split_once(':')
        .map_or((None, qname), |(prefix, local)| (Some(prefix), local));
    let ns = node
        .lookup_namespace_uri(prefix)
        .ok_or_else(|| format!("undeclared valueType QName {qname}"))?;
    let canonical_prefix = match ns {
        XS_NS => "xs",
        V => "v8",
        C => "dcscor",
        T => "dcsset",
        UI_NS => "v8ui",
        CFG_NS => "cfg",
        _ => return Err(format!("foreign valueType QName {qname} in {ns}")),
    };
    canonical_value_type(&format!("{canonical_prefix}:{local}"))
}
fn reference_value(value: &str) -> bool {
    let mut parts = value.split('.');
    let Some(kind) = parts.next() else {
        return false;
    };
    let allowed = matches!(
        kind,
        "Перечисление"
            | "Справочник"
            | "ПланСчетов"
            | "Документ"
            | "ПланВидовХарактеристик"
            | "ПланВидовРасчета"
            | "БизнесПроцесс"
            | "Задача"
            | "РегистрСведений"
            | "ПланОбмена"
            | "Catalog"
            | "Document"
            | "Enum"
            | "ChartOfAccounts"
            | "ChartOfCharacteristicTypes"
            | "ChartOfCalculationTypes"
            | "BusinessProcess"
            | "Task"
            | "InformationRegister"
            | "ExchangePlan"
    );
    let parts = parts.collect::<Vec<_>>();
    allowed
        && parts.len() >= 2
        && parts.iter().all(|part| {
            !part.is_empty() && part.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        })
}
fn color_value(value: &str) -> bool {
    value.split_once(':').is_some_and(|(prefix, name)| {
        matches!(prefix, "web" | "win" | "style")
            && !name.is_empty()
            && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
    })
}
fn scalar_value(
    tag: &str,
    value: &Value,
    declared: Option<&str>,
    ns: &str,
) -> Result<String, String> {
    let kind = match declared {
        Some(kind) => Some(canonical_value_type(kind)?),
        None if value.is_null() => None,
        None => Some(
            match value {
                Value::Bool(_) => "xs:boolean",
                Value::Number(_) => "xs:decimal",
                Value::String(value) if reference_value(value) => "dcscor:DesignTimeValue",
                Value::Object(_) => "v8:LocalStringType",
                _ => "xs:string",
            }
            .to_string(),
        ),
    };
    if value.is_null() {
        return Ok(xml(tag, "", ns).replacen(
            &format!("<{tag} "),
            &format!("<{tag} xsi:nil=\"true\" "),
            1,
        ));
    }
    let kind = kind.as_deref().expect("non-null values have a type");
    let kind = if kind.starts_with("cfg:") {
        "dcscor:DesignTimeValue"
    } else {
        kind
    };
    if kind == "v8:LocalStringType" {
        return mltext(tag, value, ns);
    }
    if kind == "v8:StandardPeriod" {
        let variant = if let Some(object) = value.as_object() {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "variant" | "startDate" | "endDate"))
            {
                return Err("unexpected StandardPeriod property".to_string());
            }
            string(object, "variant")
        } else {
            value
                .as_str()
                .ok_or("period value must be a variant or object")?
        };
        if !super::dcs_xml::is_standard_period_variant(variant) {
            return Err("invalid StandardPeriod variant".to_string());
        }
        let mut body = simple("variant", variant, V);
        if variant == "Custom" {
            let empty = Map::new();
            let dates = value.as_object().unwrap_or(&empty);
            for key in ["startDate", "endDate"] {
                let date = match dates.get(key) {
                    Some(value) => value.as_str().ok_or("period dates must be text")?,
                    None => "0001-01-01T00:00:00",
                };
                if !super::dcs_xml::is_date_time_literal(date) {
                    return Err(format!("invalid StandardPeriod {key}"));
                }
                body += &simple(key, date, V);
            }
        } else if value.as_object().is_some_and(|object| {
            object.contains_key("startDate") || object.contains_key("endDate")
        }) {
            return Err("dates are only allowed for Custom StandardPeriod".to_string());
        }
        return Ok(typed(tag, "v8:StandardPeriod", &body, ns));
    }
    if value.is_object() || value.is_array() {
        return Err("this valueType requires a scalar".to_string());
    }
    let value = value_text(value);
    match kind {
        "xs:decimal" if !super::dcs_xml::is_valid_xs_decimal(&value) => {
            return Err("invalid decimal value".to_string())
        }
        "xs:boolean" if !matches!(value.as_str(), "true" | "false" | "0" | "1") => {
            return Err("invalid boolean value".to_string())
        }
        "xs:dateTime" if !super::dcs_xml::is_date_time_literal(&value) => {
            return Err("invalid dateTime value".to_string())
        }
        "dcscor:DesignTimeValue" if !reference_value(&value) => {
            return Err("DesignTimeValue requires a metadata reference path".to_string())
        }
        "v8ui:Color" if !color_value(&value) => {
            return Err("Color requires a web:, win: or style: QName".to_string())
        }
        _ => {}
    }
    if let Some(allowed) = output_enum_values(kind) {
        if !allowed.contains(&value.as_str()) {
            return Err(format!("invalid {kind} value {value}"));
        }
    }
    Ok(typed(tag, kind, &escape_xml(&value), ns))
}
fn appearance_value(name: &str, value: &Value) -> Result<String, String> {
    if let Some(atom) = value
        .as_object()
        .filter(|atom| atom.contains_key("valueType"))
    {
        if atom
            .keys()
            .any(|key| !matches!(key.as_str(), "valueType" | "value"))
        {
            return Err("unexpected appearance atom property".to_string());
        }
        let kind = atom
            .get("valueType")
            .and_then(Value::as_str)
            .ok_or("appearance valueType must be text")?;
        return scalar_value(
            "value",
            atom.get("value").ok_or("appearance atom requires value")?,
            Some(kind),
            C,
        );
    }
    let kind = if matches!(
        name,
        "Формат" | "Текст" | "Заголовок" | "Format" | "Text" | "Title"
    ) || value.is_object()
    {
        Some("v8:LocalStringType")
    } else if value.as_str().is_some_and(color_value) {
        Some("v8ui:Color")
    } else {
        None
    };
    scalar_value("value", value, kind, C)
}
fn output_value(name: &str, value: &Value, declared: Option<&str>) -> Result<String, String> {
    let expected = output_declared_type(name);
    let declared = declared.map(canonical_value_type).transpose()?;
    if let Some(expected) = expected.filter(|kind| output_enum_values(kind).is_some()) {
        if declared.as_deref().is_some_and(|kind| kind != expected) {
            return Err(format!("output parameter {name} requires {expected}"));
        }
    }
    scalar_value("value", value, declared.as_deref().or(expected), C)
}
fn value_type(value: &str) -> Result<String, String> {
    let mut lines = Vec::new();
    super::dcs_xml::emit_value_type(&mut lines, value, "")?;
    Ok(xml(
        "valueType",
        lines
            .join("")
            .trim_start_matches("<valueType>")
            .trim_end_matches("</valueType>"),
        S,
    ))
}
fn parameter_value_type(
    node: Option<Node<'_, '_>>,
    values: &Map<String, Value>,
) -> Result<Option<String>, String> {
    if let Some(kind) = values.get("valueType").and_then(Value::as_str) {
        return canonical_value_type(kind).map(Some);
    }
    let nil = values.get("value").is_some_and(Value::is_null);
    let fragment;
    let document;
    let declaration = if !string(values, "type").is_empty() {
        fragment = value_type(string(values, "type"))?;
        document = Document::parse(&fragment).map_err(|error| error.to_string())?;
        Some(document.root_element())
    } else {
        node.and_then(|node| child(node, S, "valueType"))
    };
    let Some(declaration) = declaration else {
        return Ok(None);
    };
    let types = declaration
        .children()
        .filter(|node| is(*node, V, "Type"))
        .map(|node| canonical_node_value_type(node, node.text().ok_or("empty parameter Type")?))
        .collect::<Result<Vec<_>, String>>()?;
    if types.len() != 1 {
        if nil {
            return Ok(None);
        }
        return Err("a composite parameter needs explicit valueType".to_string());
    }
    Ok(types.into_iter().next())
}
fn restriction(tag: &str, value: &Value) -> Result<String, String> {
    let object = value.as_object().ok_or("restriction must be an object")?;
    let mut body = String::new();
    for key in ["field", "condition", "group", "order"] {
        if let Some(value) = object.get(key) {
            if !value.is_boolean() {
                return Err("restriction values must be boolean".to_string());
            }
            body += &simple(key, &value_text(value), S);
        }
    }
    Ok(xml(tag, &body, S))
}
fn available_values(values: &Value, kind: Option<&str>) -> Result<String, String> {
    let mut body = String::new();
    for item in values
        .as_array()
        .ok_or("availableValues must be an array")?
    {
        let value = item.get("value").ok_or("available value requires value")?;
        let mut entry = scalar_value("value", value, kind, S)?;
        if let Some(title) = item.get("presentation") {
            entry += &mltext("presentation", title, S)?;
        }
        body += &xml("availableValue", &entry, S);
    }
    Ok(body)
}
fn field_fragment(item: &Map<String, Value>, query: bool) -> Result<String, String> {
    let name = string(item, "dataPath");
    let field = item.get("field").and_then(Value::as_str).unwrap_or(name);
    if !string(item, "type").is_empty() {
        super::dcs_xml::parse_value_type(string(item, "type"))?;
    }
    let mut body = simple("dataPath", name, S) + &simple("field", field, S);
    if let Some(title) = item.get("title") {
        body += &mltext("title", title, S)?;
    }
    for key in ["useRestriction", "attributeUseRestriction"] {
        if let Some(value) = item.get(key) {
            body += &restriction(key, value)?;
        }
    }
    if let Some(expression) = item.get("presentationExpression") {
        body += &simple("presentationExpression", &value_text(expression), S);
    }
    if !query && !string(item, "type").is_empty() {
        body += &value_type(string(item, "type"))?;
    }
    Ok(typed("field", "DataSetFieldField", &body, S))
}
fn filter_fragment(item: &Map<String, Value>) -> Result<String, String> {
    let mut body = String::new();
    if let Some(use_flag) = item.get("use") {
        body += &simple("use", &value_text(use_flag), T);
    }
    body += &typed(
        "left",
        "dcscor:Field",
        &escape_xml(string(item, "field")),
        T,
    );
    body += &simple(
        "comparisonType",
        item.get("comparison")
            .and_then(Value::as_str)
            .unwrap_or("Equal"),
        T,
    );
    if let Some(value) = item.get("value") {
        body += &scalar_value(
            "right",
            value,
            item.get("valueType").and_then(Value::as_str),
            T,
        )?;
    }
    for key in ["viewMode", "userSettingID", "userSettingPresentation"] {
        if let Some(value) = item.get(key) {
            body += &simple(key, &value_text(value), T);
        }
    }
    Ok(typed("item", "dcsset:FilterItemComparison", &body, T))
}
fn total_groups(value: &Value) -> Result<String, String> {
    if let Some(group) = value.as_str() {
        return Ok(simple("group", group, S));
    }
    value
        .as_array()
        .ok_or("group must be a string or array of strings")?
        .iter()
        .map(|group| {
            group
                .as_str()
                .map(|group| simple("group", group, S))
                .ok_or_else(|| "total group must be text".to_string())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|groups| groups.join(""))
}
fn selection_container<'a, 'i>(
    setting: Node<'a, 'i>,
    values: &Map<String, Value>,
) -> Result<Node<'a, 'i>, String> {
    let mut container = child(setting, T, "selection").ok_or("selection collection missing")?;
    for index in values
        .get("parentIndexes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let index = index
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or("parent index must be nonnegative integer")?;
        container = container
            .children()
            .filter(|node| is(*node, T, "item"))
            .nth(index)
            .ok_or("selection parent index was not found")?;
        if !qtype(container, T, "SelectedItemFolder") {
            return Err("selection parent must be a folder".to_string());
        }
    }
    Ok(container)
}
fn group_items(fields: &[Value]) -> Result<String, String> {
    let mut body = String::new();
    for field in fields {
        let field = field.as_str().ok_or("groupBy items must be text")?;
        body += &typed(
            "item",
            "dcsset:GroupItemField",
            &(simple("field", field, T)
                + &simple("groupType", "Items", T)
                + &simple("periodAdditionType", "None", T)),
            T,
        );
    }
    Ok(xml("groupItems", &body, T))
}
const TABLE_ORDER: &[&str] = &[
    "use",
    "name",
    "column",
    "row",
    "selection",
    "conditionalAppearance",
    "outputParameters",
    "columnsViewMode",
    "rowsViewMode",
    "viewMode",
    "userSettingID",
    "itemsViewMode",
    "userSettingPresentation",
];
const CHART_ORDER: &[&str] = &[
    "use",
    "name",
    "point",
    "series",
    "selection",
    "conditionalAppearance",
    "outputParameters",
    "pointsViewMode",
    "seriesViewMode",
    "viewMode",
    "userSettingID",
    "itemsViewMode",
    "userSettingPresentation",
];
const STRUCTURE_PROPERTIES: &[&str] = &[
    "use",
    "columnsViewMode",
    "rowsViewMode",
    "pointsViewMode",
    "seriesViewMode",
    "viewMode",
    "userSettingID",
    "itemsViewMode",
    "userSettingPresentation",
];
fn structure_order(node: Node<'_, '_>) -> &'static [&'static str] {
    if is(node, T, "settings") {
        SETTINGS_ORDER
    } else if qtype(node, T, "StructureItemTable") {
        TABLE_ORDER
    } else if qtype(node, T, "StructureItemChart") {
        CHART_ORDER
    } else {
        GROUP_ORDER
    }
}
fn named_structure<'a, 'i>(setting: Node<'a, 'i>, name: &str) -> Result<Node<'a, 'i>, String> {
    unique(
        setting.descendants().filter(|node| {
            (is(*node, T, "item")
                || ["row", "column", "point", "series"]
                    .iter()
                    .any(|tag| is(*node, T, tag)))
                && text(*node, T, "name") == name
        }),
        name,
    )
}
fn structure_property(tag: &str, value: &Value, kind: &str) -> Result<String, String> {
    if (matches!(tag, "columnsViewMode" | "rowsViewMode") && kind != "table")
        || (matches!(tag, "pointsViewMode" | "seriesViewMode") && kind != "chart")
    {
        return Err(format!("{tag} is incompatible with {kind}"));
    }
    if tag == "userSettingPresentation" {
        mltext(tag, value, T)
    } else {
        Ok(simple(tag, &value_text(value), T))
    }
}
fn structure_fragment(item: &Map<String, Value>) -> Result<String, String> {
    let kind = item.get("kind").and_then(Value::as_str).unwrap_or("group");
    let mut body = item
        .get("use")
        .map(|flag| simple("use", &value_text(flag), T))
        .unwrap_or_default();
    if let Some(name) = item.get("name") {
        body += &simple("name", &value_text(name), T);
    }
    if kind == "group" {
        if let Some(fields) = item
            .get("groupBy")
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty())
        {
            body += &group_items(fields)?;
        }
    }
    for key in STRUCTURE_PROPERTIES.iter().filter(|key| **key != "use") {
        if let Some(value) = item.get(*key) {
            body += &structure_property(key, value, kind)?;
        }
    }
    let wire = match kind {
        "group" => "dcsset:StructureItemGroup",
        "table" => "dcsset:StructureItemTable",
        "chart" => "dcsset:StructureItemChart",
        _ => return Err("unknown structure kind".to_string()),
    };
    Ok(typed("item", wire, &body, T))
}

/// Apply one normalized operation to captured XML. The caller owns atomic
/// publication and discards this tentative image if any operation fails.
pub(crate) fn apply(
    xml_text: &mut String,
    operation: &str,
    args: &Map<String, Value>,
    target: &Target,
) -> Result<(), String> {
    let mut candidate = xml_text.clone();
    apply_candidate(&mut candidate, operation, args, target)?;
    Document::parse(&candidate).map_err(|error| error.to_string())?;
    *xml_text = candidate;
    Ok(())
}

fn apply_candidate(
    xml_text: &mut String,
    operation: &str,
    args: &Map<String, Value>,
    target: &Target,
) -> Result<(), String> {
    let empty = Map::new();
    let values = args
        .get("values")
        .and_then(Value::as_object)
        .unwrap_or(&empty);
    if operation == "parameter.reorder" {
        let items = args
            .get("items")
            .and_then(Value::as_array)
            .ok_or("parameter list required")?;
        let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
        let parameters = document
            .root_element()
            .children()
            .filter(|node| is(*node, S, "parameter"))
            .collect::<Vec<_>>();
        if items.len() != parameters.len() {
            return Err("reorder needs every parameter exactly once".to_string());
        }
        let mut used = std::collections::HashSet::new();
        let mut replacements = Vec::new();
        for (slot, item) in parameters.iter().zip(items) {
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .ok_or("parameter name required")?;
            if !used.insert(name) {
                return Err("duplicate parameter in reorder".to_string());
            }
            let parameter = unique(
                parameters
                    .iter()
                    .copied()
                    .filter(|node| text(*node, S, "name") == name),
                name,
            )?;
            replacements.push((slot.range(), xml_text[parameter.range()].to_string()));
        }
        for (range, replacement) in replacements.into_iter().rev() {
            xml_text.replace_range(range, &replacement);
        }
        return Ok(());
    }
    if let Some(items) = args.get("items").and_then(Value::as_array) {
        for item in items {
            apply_item(
                xml_text,
                operation,
                item.as_object().ok_or("item must be an object")?,
                target,
            )?;
        }
        return Ok(());
    }
    apply_item(xml_text, operation, values, target)
}

fn apply_item(
    xml_text: &mut String,
    operation: &str,
    values: &Map<String, Value>,
    target: &Target,
) -> Result<(), String> {
    let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
    let root = document.root_element();
    super::dcs::require_dcs_root(root)?;
    match operation {
        "dataSource.add" => {
            let name = string(values, "name");
            if root
                .children()
                .any(|node| is(node, S, "dataSource") && text(node, S, "name") == name)
            {
                return Err(format!("dataSource {name} already exists"));
            }
            let kind = values
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("Local");
            let fragment = xml(
                "dataSource",
                &(simple("name", name, S) + &simple("dataSourceType", kind, S)),
                S,
            );
            insert(
                xml_text,
                root.range(),
                &fragment,
                S,
                "dataSource",
                ROOT_ORDER,
            )
        }
        "dataSource.set" | "dataSource.remove" => {
            let source = named(root, S, "dataSource", "name", string(values, "name"))?;
            if operation.ends_with("remove") {
                if root.descendants().any(|node| {
                    is(node, S, "dataSource")
                        && !node.children().any(|n| n.is_element())
                        && node
                            .children()
                            .filter(Node::is_text)
                            .filter_map(|node| node.text())
                            .collect::<String>()
                            == string(values, "name")
                }) {
                    return Err("dataSource is still referenced by a dataset".to_string());
                }
                let range = source.range();
                xml_text.replace_range(range, "");
                Ok(())
            } else {
                upsert(
                    xml_text,
                    source.range(),
                    S,
                    "dataSourceType",
                    &simple("dataSourceType", string(values, "kind"), S),
                    &["name", "dataSourceType"],
                )
            }
        }
        "dataSet.add" => {
            let parent = if target.datasets.is_empty() {
                root
            } else {
                dataset(root, target, &Map::new())?
            };
            let tag = if target.datasets.is_empty() {
                "dataSet"
            } else {
                "item"
            };
            if tag == "item" && !qtype(parent, S, "DataSetUnion") {
                return Err("only a Union dataset accepts child datasets".to_string());
            }
            let name = string(values, "name");
            if parent
                .children()
                .any(|node| is(node, S, tag) && text(node, S, "name") == name)
            {
                return Err(format!("dataset {name} already exists"));
            }
            let kind = values
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or("Query");
            match kind {
                "Query"
                    if string(values, "query").is_empty() || values.contains_key("objectName") =>
                {
                    return Err("Query requires query and accepts no objectName".to_string())
                }
                "Object"
                    if string(values, "objectName").is_empty() || values.contains_key("query") =>
                {
                    return Err("Object requires objectName and accepts no query".to_string())
                }
                "Union"
                    if values.contains_key("dataSource")
                        || values.contains_key("query")
                        || values.contains_key("objectName") =>
                {
                    return Err("Union members are composed separately".to_string())
                }
                _ => {}
            }
            if string(values, "query").trim_start().starts_with('@') {
                return Err("query takes text, not @file".to_string());
            }
            let mut body = simple("name", name, S);
            if kind != "Union" {
                named(root, S, "dataSource", "name", string(values, "dataSource"))?;
                body += &simple("dataSource", string(values, "dataSource"), S);
            }
            match kind {
                "Query" => {
                    body += &simple("query", string(values, "query"), S);
                }
                "Object" => {
                    body += &simple("objectName", string(values, "objectName"), S);
                }
                "Union" => {}
                _ => return Err("unsupported dataset kind".to_string()),
            }
            let fragment = typed(tag, &format!("DataSet{kind}"), &body, S);
            insert(
                xml_text,
                parent.range(),
                &fragment,
                S,
                tag,
                if tag == "dataSet" {
                    ROOT_ORDER
                } else {
                    DATASET_ORDER
                },
            )
        }
        "dataSet.set" => {
            dataset(root, target, values)?;
            for key in ["dataSource", "objectName", "autoFillFields"] {
                if let Some(value) = values.get(key) {
                    let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
                    let root = document.root_element();
                    let ds = dataset(root, target, values)?;
                    let query = qtype(ds, S, "DataSetQuery");
                    let object = qtype(ds, S, "DataSetObject");
                    if key == "dataSource" {
                        named(root, S, "dataSource", "name", &value_text(value))?;
                    }
                    if (key == "objectName" && !object)
                        || (key == "autoFillFields" && !query)
                        || (!object && !query)
                    {
                        return Err(format!("{key} does not apply to this dataset kind"));
                    }
                    upsert(
                        xml_text,
                        ds.range(),
                        S,
                        key,
                        &simple(key, &value_text(value), S),
                        DATASET_ORDER,
                    )?;
                }
            }
            Ok(())
        }
        "dataSet.remove" => {
            let range = dataset(root, target, values)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        "field.add" => {
            let ds = dataset(root, target, values)?;
            let name = string(values, "dataPath");
            if ds
                .children()
                .any(|node| is(node, S, "field") && text(node, S, "dataPath") == name)
            {
                return Ok(());
            }
            let fragment = field_fragment(values, qtype(ds, S, "DataSetQuery"))?;
            insert(xml_text, ds.range(), &fragment, S, "field", DATASET_ORDER)
        }
        "field.set" => {
            if !string(values, "type").is_empty() {
                super::dcs_xml::parse_value_type(string(values, "type"))?;
            }
            let name = string(values, "field");
            named(dataset(root, target, values)?, S, "field", "dataPath", name)?;
            for key in [
                "sourceField",
                "title",
                "useRestriction",
                "attributeUseRestriction",
                "presentationExpression",
                "type",
            ] {
                let Some(value) = values.get(key) else {
                    continue;
                };
                let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
                let ds = dataset(document.root_element(), target, values)?;
                let field = named(ds, S, "field", "dataPath", name)?;
                let tag = match key {
                    "type" => "valueType",
                    "sourceField" => "field",
                    other => other,
                };
                if key == "type" && qtype(ds, S, "DataSetQuery") {
                    let ranges = field
                        .children()
                        .filter(|node| is(*node, S, "valueType"))
                        .map(|node| node.range())
                        .collect();
                    remove_ranges(xml_text, ranges);
                } else {
                    let fragment = match key {
                        "type" => value_type(&value_text(value))?,
                        "title" => mltext(tag, value, S)?,
                        "useRestriction" | "attributeUseRestriction" => restriction(tag, value)?,
                        _ => simple(tag, &value_text(value), S),
                    };
                    upsert(xml_text, field.range(), S, tag, &fragment, FIELD_ORDER)?;
                }
            }
            let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
            let ds = dataset(document.root_element(), target, values)?;
            if qtype(ds, S, "DataSetQuery") {
                let field = named(ds, S, "field", "dataPath", name)?;
                let ranges = field
                    .children()
                    .filter(|node| is(*node, S, "valueType"))
                    .map(|node| node.range())
                    .collect();
                remove_ranges(xml_text, ranges);
            }
            Ok(())
        }
        "field.remove" => {
            let ds = dataset(root, target, values)?;
            let name = target.terminal.as_deref().ok_or("address a named Field")?;
            let range = named(ds, S, "field", "dataPath", name)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        "fieldRole.set" => {
            let ds = dataset(root, target, values)?;
            let field = named(ds, S, "field", "dataPath", string(values, "field"))?;
            let role = values
                .get("role")
                .and_then(Value::as_object)
                .ok_or("role must be an object")?;
            let mut body = String::new();
            for key in [
                "periodNumber",
                "periodType",
                "dimension",
                "parentDimension",
                "account",
                "accountTypeExpression",
                "balance",
                "balanceGroupName",
                "balanceType",
                "accountingBalanceType",
                "accountField",
                "ignoreNullValues",
                "required",
                "dimensionAttribute",
            ] {
                if let Some(value) = role.get(key) {
                    super::dcs_xml::validate_field_role_value(key, &value_text(value))?;
                    body += &simple(key, &value_text(value), super::dcs::DCS_COMMON_NS);
                }
            }
            upsert(
                xml_text,
                field.range(),
                S,
                "role",
                &xml("role", &body, S),
                FIELD_ORDER,
            )
        }
        "query.set" | "query.patch" => {
            let ds = dataset(root, target, values)?;
            if !qtype(ds, S, "DataSetQuery") {
                return Err("only a Query dataset has query text".to_string());
            }
            let query = child(ds, S, "query").ok_or("dataset has no query")?;
            let (range, replacement) =
                change_query(xml_text, query, values, operation == "query.patch")?;
            xml_text.replace_range(range, &replacement);
            Ok(())
        }
        "parameter.add" | "calculatedField.add" | "total.add" => {
            let (tag, key, name) = match operation {
                "parameter.add" => ("parameter", "name", string(values, "name")),
                "calculatedField.add" => ("calculatedField", "dataPath", string(values, "name")),
                _ => ("totalField", "dataPath", string(values, "field")),
            };
            if root
                .children()
                .any(|node| is(node, S, tag) && text(node, S, key) == name)
            {
                return Ok(());
            }
            let mut body = simple(key, name, S);
            if tag == "calculatedField" {
                body += &simple("expression", string(values, "expression"), S);
            }
            if let Some(title) = values.get("title") {
                body += &mltext("title", title, S)?;
            }
            if tag == "calculatedField" {
                if let Some(value) = values.get("useRestriction") {
                    body += &restriction("useRestriction", value)?;
                }
            }
            if !string(values, "type").is_empty() {
                body += &value_type(string(values, "type"))?;
            }
            if let Some(value) = values.get("value") {
                let kind = parameter_value_type(None, values)?;
                body += &scalar_value("value", value, kind.as_deref(), S)?;
            }
            if let Some(list) = values.get("values") {
                if values.contains_key("value") {
                    return Err("value and values are mutually exclusive".to_string());
                }
                let kind = parameter_value_type(None, values)?;
                for value in list.as_array().ok_or("values must be an array")? {
                    body += &scalar_value("value", value, kind.as_deref(), S)?;
                }
            }
            let expression = string(values, "expression");
            if tag == "totalField" {
                let expression = if expression.is_empty() {
                    format!("Сумма({name})")
                } else {
                    expression.to_string()
                };
                body += &simple("expression", &expression, S);
            }

            if tag == "parameter" && values.contains_key("useRestriction") {
                body += &simple("useRestriction", &value_text(&values["useRestriction"]), S);
            }
            if tag == "parameter" && !expression.is_empty() {
                body += &simple("expression", expression, S);
            }
            if let Some(list) = values.get("availableValues") {
                let kind = parameter_value_type(None, values)?;
                body += &available_values(list, kind.as_deref())?;
            }
            for key in [
                "valueListAllowed",
                "availableAsField",
                "denyIncompleteValues",
                "use",
                "group",
            ] {
                if let Some(value) = values.get(key) {
                    body += &if key == "group" {
                        total_groups(value)?
                    } else {
                        simple(key, &value_text(value), S)
                    };
                }
            }
            insert(
                xml_text,
                root.range(),
                &xml(tag, &body, S),
                S,
                tag,
                ROOT_ORDER,
            )
        }
        "parameter.set" => {
            named(root, S, "parameter", "name", string(values, "name"))?;
            for key in [
                "title",
                "type",
                "value",
                "values",
                "expression",
                "useRestriction",
                "availableValues",
                "valueListAllowed",
                "availableAsField",
                "denyIncompleteValues",
                "use",
            ] {
                let Some(value) = values.get(key) else {
                    continue;
                };
                let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
                let parameter = named(
                    document.root_element(),
                    S,
                    "parameter",
                    "name",
                    string(values, "name"),
                )?;
                let tag = match key {
                    "type" => "valueType",
                    "values" => "value",
                    "availableValues" => "availableValue",
                    other => other,
                };
                if matches!(key, "value" | "values" | "availableValues") {
                    if values.contains_key("value") && values.contains_key("values") {
                        return Err("value and values are mutually exclusive".to_string());
                    }
                    let kind = parameter_value_type(Some(parameter), values)?;
                    let fragment = if key == "availableValues" {
                        available_values(value, kind.as_deref())?
                    } else if key == "values" {
                        value
                            .as_array()
                            .ok_or("values must be an array")?
                            .iter()
                            .map(|value| scalar_value("value", value, kind.as_deref(), S))
                            .collect::<Result<Vec<_>, _>>()?
                            .join("")
                    } else {
                        scalar_value("value", value, kind.as_deref(), S)?
                    };
                    let old = parameter
                        .children()
                        .filter(|node| is(*node, S, tag))
                        .collect::<Vec<_>>();
                    let wrapped = format!("<root>{fragment}</root>");
                    let parsed = Document::parse(&wrapped).map_err(|error| error.to_string())?;
                    let new = parsed
                        .root_element()
                        .children()
                        .filter(|node| node.is_element())
                        .collect::<Vec<_>>();
                    if old.len() == new.len()
                        && old
                            .iter()
                            .zip(new.iter())
                            .all(|(left, right)| equal_element(*left, *right))
                    {
                        continue;
                    }
                    let ranges = parameter
                        .children()
                        .filter(|node| is(*node, S, tag))
                        .map(|node| node.range())
                        .collect();
                    remove_ranges(xml_text, ranges);
                    if !fragment.is_empty() {
                        let document =
                            Document::parse(xml_text).map_err(|error| error.to_string())?;
                        let parameter = named(
                            document.root_element(),
                            S,
                            "parameter",
                            "name",
                            string(values, "name"),
                        )?;
                        insert(
                            xml_text,
                            parameter.range(),
                            &fragment,
                            S,
                            tag,
                            PARAMETER_ORDER,
                        )?;
                    }
                    continue;
                }
                let fragment = match key {
                    "title" => mltext(tag, value, S)?,
                    "type" => value_type(&value_text(value))?,
                    "value" => {
                        let kind = parameter_value_type(Some(parameter), values)?;
                        scalar_value(tag, value, kind.as_deref(), S)?
                    }
                    _ => simple(tag, &value_text(value), S),
                };
                upsert(
                    xml_text,
                    parameter.range(),
                    S,
                    tag,
                    &fragment,
                    PARAMETER_ORDER,
                )?;
            }
            Ok(())
        }
        "parameter.rename" => {
            let name = string(values, "name");
            let new = string(values, "newName");
            if root
                .children()
                .any(|node| is(node, S, "parameter") && text(node, S, "name") == new)
            {
                return Err("parameter name already exists".to_string());
            }
            let parameter = named(root, S, "parameter", "name", name)?;
            upsert(
                xml_text,
                parameter.range(),
                S,
                "name",
                &simple("name", new, S),
                PARAMETER_ORDER,
            )
        }
        "parameter.remove" | "calculatedField.remove" | "total.remove" => {
            let (tag, key) = match operation {
                "parameter.remove" => ("parameter", "name"),
                "total.remove" => ("totalField", "dataPath"),
                _ => ("calculatedField", "dataPath"),
            };
            let name = values
                .get("name")
                .and_then(Value::as_str)
                .or(target.terminal.as_deref())
                .ok_or("supply a name or address a named parameter/calculation")?;
            let range = named(root, S, tag, key, name)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        "variant.add" => {
            let name = string(values, "name");
            if root
                .children()
                .any(|node| is(node, S, "settingsVariant") && text(node, T, "name") == name)
            {
                return Ok(());
            }
            let title = values
                .get("title")
                .cloned()
                .unwrap_or_else(|| Value::String(name.to_string()));
            let body = simple("name", name, T)
                + &mltext("presentation", &title, T)?
                + &xml("settings", "", T);
            insert(
                xml_text,
                root.range(),
                &xml("settingsVariant", &body, S),
                S,
                "settingsVariant",
                ROOT_ORDER,
            )
        }
        "variant.set" | "variant.remove" => {
            let name = if string(values, "name").is_empty() {
                &target.variant
            } else {
                string(values, "name")
            };
            let variant = unique(
                root.children().filter(|node| {
                    is(*node, S, "settingsVariant") && text(*node, T, "name") == name
                }),
                name,
            )?;
            if operation.ends_with("remove") {
                let range = variant.range();
                xml_text.replace_range(range, "");
                Ok(())
            } else {
                upsert(
                    xml_text,
                    variant.range(),
                    T,
                    "presentation",
                    &mltext(
                        "presentation",
                        values.get("title").ok_or("title required")?,
                        T,
                    )?,
                    &["name", "presentation", "settings"],
                )
            }
        }
        "structure.add" | "structure.set" | "structure.patch" | "structure.remove" => {
            let setting = settings(root, target, values)?;
            if operation == "structure.remove" {
                let group = unique(
                    setting.descendants().filter(|node| {
                        (is(*node, T, "item")
                            || ["row", "column", "point", "series"]
                                .iter()
                                .any(|tag| is(*node, T, tag)))
                            && text(*node, T, "name") == string(values, "name")
                    }),
                    string(values, "name"),
                )?;
                let range = group.range();
                xml_text.replace_range(range, "");
                return Ok(());
            }
            if operation == "structure.patch" {
                let name = string(values, "name");
                let group = named_structure(setting, name)?;
                let kind = if qtype(group, T, "StructureItemTable") {
                    "table"
                } else if qtype(group, T, "StructureItemChart") {
                    "chart"
                } else if qtype(group, T, "StructureItemGroup")
                    || ["row", "column", "point", "series"]
                        .iter()
                        .any(|tag| is(group, T, tag))
                {
                    "group"
                } else {
                    return Err("structure type is unsupported for patch".to_string());
                };
                if let Some(fields) = values.get("groupBy").and_then(Value::as_array) {
                    if kind != "group" {
                        return Err(
                            "groupBy applies to a group or named table/chart axis".to_string()
                        );
                    }
                    if fields.is_empty() {
                        let ranges = group
                            .children()
                            .filter(|node| is(*node, T, "groupItems"))
                            .map(|node| node.range())
                            .collect();
                        remove_ranges(xml_text, ranges);
                    } else {
                        upsert(
                            xml_text,
                            group.range(),
                            T,
                            "groupItems",
                            &group_items(fields)?,
                            GROUP_ORDER,
                        )?;
                    }
                }
                for key in STRUCTURE_PROPERTIES {
                    if let Some(value) = values.get(*key) {
                        let document =
                            Document::parse(xml_text).map_err(|error| error.to_string())?;
                        let setting = settings(document.root_element(), target, values)?;
                        let group = named_structure(setting, name)?;
                        upsert(
                            xml_text,
                            group.range(),
                            T,
                            key,
                            &structure_property(key, value, kind)?,
                            structure_order(group),
                        )?;
                    }
                }
                return Ok(());
            }
            if operation == "structure.set" {
                let ranges = setting
                    .children()
                    .filter(|node| is(*node, T, "item"))
                    .map(|node| node.range())
                    .collect();
                remove_ranges(xml_text, ranges);
                let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
                let setting = settings(document.root_element(), target, values)?;
                let fields = values
                    .get("groupBy")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let mut fragment = structure_fragment(values)?;
                if !fields.is_empty()
                    && values
                        .get("details")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                {
                    let details = structure_fragment(&Map::new())?;
                    let close = fragment.rfind("</item>").ok_or("structure close missing")?;
                    fragment.insert_str(close, &details);
                }
                return insert(
                    xml_text,
                    setting.range(),
                    &fragment,
                    T,
                    "item",
                    structure_order(setting),
                );
            }
            let mut parent = setting;
            if !string(values, "parent").is_empty() {
                parent = unique(
                    setting.descendants().filter(|node| {
                        (is(*node, T, "item")
                            || ["row", "column", "point", "series"]
                                .iter()
                                .any(|tag| is(*node, T, tag)))
                            && text(*node, T, "name") == string(values, "parent")
                    }),
                    string(values, "parent"),
                )?;
            }
            if setting.descendants().any(|node| {
                (is(node, T, "item")
                    || ["row", "column", "point", "series"]
                        .iter()
                        .any(|tag| is(node, T, tag)))
                    && !string(values, "name").is_empty()
                    && text(node, T, "name") == string(values, "name")
            }) {
                return Err("structure name already exists".to_string());
            }
            let fragment = structure_fragment(values)?;
            let axis = string(values, "axis");
            if !axis.is_empty() {
                if !((qtype(parent, T, "StructureItemTable") && matches!(axis, "row" | "column"))
                    || (qtype(parent, T, "StructureItemChart")
                        && matches!(axis, "point" | "series")))
                {
                    return Err("axis is incompatible with the parent structure kind".to_string());
                }
                if values
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind != "group")
                {
                    return Err("a table/chart axis must be a group".to_string());
                }
                let parsed = Document::parse(&fragment).map_err(|error| error.to_string())?;
                let inner = content(&fragment, parsed.root_element())?;
                let fragment = xml(axis, &fragment[inner], T);
                insert(
                    xml_text,
                    parent.range(),
                    &fragment,
                    T,
                    axis,
                    structure_order(parent),
                )
            } else {
                insert(
                    xml_text,
                    parent.range(),
                    &fragment,
                    T,
                    "item",
                    structure_order(parent),
                )
            }
        }
        "selection.add"
        | "filter.add"
        | "order.add"
        | "dataParameter.add"
        | "dataParameter.set"
        | "conditionalAppearance.add"
        | "outputParameter.set" => {
            let setting = settings(root, target, values)?;
            if matches!(operation, "dataParameter.set" | "outputParameter.set") {
                let container = if operation == "dataParameter.set" {
                    "dataParameters"
                } else {
                    "outputParameters"
                };
                if child(setting, T, container).is_some_and(|collection| {
                    collection.children().any(|node| {
                        is(node, C, "item") && text(node, C, "parameter") == string(values, "name")
                    })
                }) {
                    return patch_settings_parameter(xml_text, target, values, container);
                }
            }
            let (container, mut fragment) = match operation {
                "selection.add" => {
                    let kind = values
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or("Field");
                    let body = match kind {
                        "Field" => {
                            if values.contains_key("title") {
                                return Err("title belongs to Folder selection".to_string());
                            }
                            if string(values, "field").is_empty() {
                                return Err("Field selection requires field".to_string());
                            }
                            simple("field", string(values, "field"), T)
                        }
                        "Auto" => {
                            if values.contains_key("field") || values.contains_key("title") {
                                return Err("Auto selection has no field/title".to_string());
                            }
                            String::new()
                        }
                        "Folder" => {
                            if values.contains_key("field") {
                                return Err(
                                    "Folder has no field; add its children separately".to_string()
                                );
                            }
                            mltext(
                                "lwsTitle",
                                values.get("title").ok_or("Folder requires title")?,
                                T,
                            )? + &simple("placement", "Auto", T)
                        }
                        _ => return Err("unknown selection kind".to_string()),
                    };
                    (
                        "selection",
                        typed("item", &format!("dcsset:SelectedItem{kind}"), &body, T),
                    )
                }
                "filter.add" => ("filter", filter_fragment(values)?),
                "order.add" => {
                    let auto = string(values, "kind") == "Auto";
                    if auto && values.contains_key("field") {
                        return Err("Auto order has no field".to_string());
                    }
                    if !auto && string(values, "field").is_empty() {
                        return Err("Field order requires field".to_string());
                    }
                    (
                        "order",
                        typed(
                            "item",
                            if auto {
                                "dcsset:OrderItemAuto"
                            } else {
                                "dcsset:OrderItemField"
                            },
                            &if auto {
                                String::new()
                            } else {
                                simple("field", string(values, "field"), T)
                                    + &simple(
                                        "orderType",
                                        if string(values, "direction") == "Desc" {
                                            "Desc"
                                        } else {
                                            "Asc"
                                        },
                                        T,
                                    )
                            },
                            T,
                        ),
                    )
                }
                "dataParameter.add" | "dataParameter.set" => {
                    let value = values.get("value").ok_or("data parameter value required")?;
                    (
                        "dataParameters",
                        typed(
                            "item",
                            "dcsset:SettingsParameterValue",
                            &(values
                                .get("use")
                                .map(|flag| simple("use", &value_text(flag), C))
                                .unwrap_or_default()
                                + &simple("parameter", string(values, "name"), C)
                                + &scalar_value(
                                    "value",
                                    value,
                                    values.get("valueType").and_then(Value::as_str),
                                    C,
                                )?),
                            C,
                        ),
                    )
                }
                "outputParameter.set" => {
                    let value = values
                        .get("value")
                        .ok_or("output parameter value required")?;
                    (
                        "outputParameters",
                        typed(
                            "item",
                            "dcsset:SettingsParameterValue",
                            &(values
                                .get("use")
                                .map(|flag| simple("use", &value_text(flag), C))
                                .unwrap_or_default()
                                + &simple("parameter", string(values, "name"), C)
                                + &output_value(
                                    string(values, "name"),
                                    value,
                                    values.get("valueType").and_then(Value::as_str),
                                )?),
                            C,
                        ),
                    )
                }
                _ => {
                    let fields = values
                        .get("fields")
                        .and_then(Value::as_array)
                        .ok_or("appearance fields required")?;
                    let mut body = String::new();
                    for field in fields {
                        body += &xml(
                            "item",
                            &simple(
                                "field",
                                field.as_str().ok_or("appearance field must be text")?,
                                T,
                            ),
                            T,
                        );
                    }
                    let fields = xml("selection", &body, T);
                    body.clear();
                    let appearance = values
                        .get("appearance")
                        .and_then(Value::as_object)
                        .ok_or("appearance required")?;
                    for (key, value) in appearance {
                        body += &typed(
                            "item",
                            "dcsset:SettingsParameterValue",
                            &(simple("parameter", key, C) + &appearance_value(key, value)?),
                            C,
                        );
                    }
                    let mut entry = values
                        .get("use")
                        .map(|flag| simple("use", &value_text(flag), T))
                        .unwrap_or_default()
                        + &fields;
                    if let Some(filters) = values.get("filter").and_then(Value::as_array) {
                        let mut body = String::new();
                        for filter in filters {
                            body += &filter_fragment(
                                filter
                                    .as_object()
                                    .ok_or("appearance filter must be object")?,
                            )?;
                        }
                        entry += &xml("filter", &body, T);
                    }
                    entry += &xml("appearance", &body, T);
                    ("conditionalAppearance", xml("item", &entry, T))
                }
            };
            if matches!(
                operation,
                "dataParameter.add" | "dataParameter.set" | "outputParameter.set"
            ) {
                let mut extras = String::new();
                for key in ["viewMode", "userSettingID", "userSettingPresentation"] {
                    if let Some(value) = values.get(key) {
                        extras += &simple(key, &value_text(value), T);
                    }
                }
                let close = fragment
                    .rfind("</item>")
                    .ok_or("parameter item end missing")?;
                fragment.insert_str(close, &extras);
            }
            if operation == "selection.add"
                && values
                    .get("parentIndexes")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
            {
                let folder = selection_container(setting, values)?;
                return insert(
                    xml_text,
                    folder.range(),
                    &fragment,
                    T,
                    "item",
                    &[
                        "use",
                        "lwsTitle",
                        "item",
                        "placement",
                        "viewMode",
                        "userSettingID",
                        "userSettingPresentation",
                    ],
                );
            }
            if let Some(container_node) = child(setting, T, container) {
                if operation == "outputParameter.set" || operation.starts_with("dataParameter.") {
                    let existing = container_node.children().find(|node| {
                        is(*node, C, "item")
                            && text(*node, C, "parameter") == string(values, "name")
                    });
                    if let Some(existing) = existing {
                        let _ = existing;
                        patch_settings_parameter(xml_text, target, values, container)?;
                        return Ok(());
                    }
                }
                insert(xml_text, container_node.range(), &fragment, T, "item", &[])
            } else {
                insert(
                    xml_text,
                    setting.range(),
                    &xml(container, &fragment, T),
                    T,
                    container,
                    structure_order(setting),
                )
            }
        }
        "filter.set" | "filter.remove" => {
            let field = string(values, "field");
            let setting = settings(root, target, values)?;
            let filter = child(setting, T, "filter").ok_or("filter section missing")?;
            let item = unique(
                filter
                    .children()
                    .filter(|node| is(*node, T, "item") && text(*node, T, "left") == field),
                field,
            )?;
            if operation == "filter.remove" {
                let range = item.range();
                xml_text.replace_range(range, "");
                return Ok(());
            }
            for key in [
                "use",
                "comparison",
                "value",
                "presentation",
                "application",
                "viewMode",
                "userSettingID",
                "userSettingPresentation",
            ] {
                let Some(value) = values.get(key) else {
                    continue;
                };
                let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
                let setting = settings(document.root_element(), target, values)?;
                let filter = child(setting, T, "filter").ok_or("filter section missing")?;
                let item = unique(
                    filter
                        .children()
                        .filter(|node| is(*node, T, "item") && text(*node, T, "left") == field),
                    field,
                )?;
                let tag = match key {
                    "comparison" => "comparisonType",
                    "value" => "right",
                    other => other,
                };
                let fragment = if key == "value" {
                    scalar_value(
                        tag,
                        value,
                        values.get("valueType").and_then(Value::as_str),
                        T,
                    )?
                } else {
                    simple(tag, &value_text(value), T)
                };
                upsert(
                    xml_text,
                    item.range(),
                    T,
                    tag,
                    &fragment,
                    &[
                        "use",
                        "left",
                        "comparisonType",
                        "right",
                        "presentation",
                        "application",
                        "viewMode",
                        "userSettingID",
                        "userSettingPresentation",
                    ],
                )?;
            }
            Ok(())
        }
        "selection.clear" | "filter.clear" | "order.clear" | "conditionalAppearance.clear" => {
            let setting = settings(root, target, values)?;
            let tag = operation.split('.').next().ok_or("operation missing")?;
            if let Some(container) = child(setting, T, tag) {
                let ranges = container
                    .children()
                    .filter(|node| node.is_element())
                    .map(|node| node.range())
                    .collect();
                remove_ranges(xml_text, ranges);
            }
            Ok(())
        }
        "calculatedField.set" | "total.set" => {
            let (tag, key) = if operation == "total.set" {
                ("totalField", "field")
            } else {
                ("calculatedField", "name")
            };
            named(root, S, tag, "dataPath", string(values, key))?;
            for property in ["expression", "title", "type", "useRestriction", "group"] {
                let Some(value) = values.get(property) else {
                    continue;
                };
                let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
                let node = named(
                    document.root_element(),
                    S,
                    tag,
                    "dataPath",
                    string(values, key),
                )?;
                let tag = match property {
                    "type" => "valueType",
                    "group" => "group",
                    other => other,
                };
                let fragment = match property {
                    "title" => mltext(tag, value, S)?,
                    "type" => value_type(&value_text(value))?,
                    "useRestriction" => restriction(tag, value)?,
                    "group" => total_groups(value)?,
                    _ => simple(tag, &value_text(value), S),
                };
                if property == "group" {
                    let old = node
                        .children()
                        .filter(|node| is(*node, S, "group"))
                        .collect::<Vec<_>>();
                    let wrapped = format!("<root>{fragment}</root>");
                    let parsed = Document::parse(&wrapped).map_err(|error| error.to_string())?;
                    let new = parsed
                        .root_element()
                        .children()
                        .filter(Node::is_element)
                        .collect::<Vec<_>>();
                    if old.len() == new.len()
                        && old
                            .iter()
                            .zip(new.iter())
                            .all(|(left, right)| equal_element(*left, *right))
                    {
                        continue;
                    }
                    let ranges = node
                        .children()
                        .filter(|node| is(*node, S, "group"))
                        .map(|node| node.range())
                        .collect();
                    remove_ranges(xml_text, ranges);
                    let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
                    let node = named(
                        document.root_element(),
                        S,
                        "totalField",
                        "dataPath",
                        string(values, key),
                    )?;
                    insert(
                        xml_text,
                        node.range(),
                        &fragment,
                        S,
                        tag,
                        &["dataPath", "expression", "group"],
                    )?;
                } else {
                    upsert(
                        xml_text,
                        node.range(),
                        S,
                        tag,
                        &fragment,
                        &[
                            "dataPath",
                            "expression",
                            "title",
                            "useRestriction",
                            "valueType",
                        ],
                    )?;
                }
            }
            Ok(())
        }
        "selection.set"
        | "selection.remove"
        | "order.set"
        | "order.remove"
        | "dataParameter.remove"
        | "outputParameter.remove"
        | "conditionalAppearance.remove" => {
            let setting = settings(root, target, values)?;
            let family = operation.split('.').next().ok_or("operation missing")?;
            if matches!(family, "selection" | "order")
                && values.contains_key("field") == values.contains_key("index")
            {
                return Err("supply exactly one field or index selector".to_string());
            }
            let container = match family {
                "dataParameter" => "dataParameters",
                "outputParameter" => "outputParameters",
                other => other,
            };
            let container_name = container.to_string();
            let container = if family == "selection" {
                selection_container(setting, values)?
            } else {
                child(setting, T, &container_name).ok_or("settings collection missing")?
            };
            let item = if values.contains_key("index") {
                let index = values
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or("index must be a nonnegative integer")?;
                container
                    .children()
                    .filter(|node| is(*node, T, "item"))
                    .nth(usize::try_from(index).map_err(|_| "index too large")?)
                    .ok_or("appearance index was not found")?
            } else {
                let (ns, key, selector) = if matches!(family, "dataParameter" | "outputParameter") {
                    (C, "parameter", "name")
                } else {
                    (T, "field", "field")
                };
                unique(
                    container.children().filter(|node| {
                        is(*node, ns, "item") && text(*node, ns, key) == string(values, selector)
                    }),
                    string(values, selector),
                )?
            };
            if operation.ends_with("remove") {
                let range = item.range();
                xml_text.replace_range(range, "");
                return Ok(());
            }
            for key in [
                "use",
                "direction",
                "viewMode",
                "userSettingID",
                "userSettingPresentation",
            ] {
                if let Some(value) = values.get(key) {
                    let tag = if key == "direction" { "orderType" } else { key };
                    let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
                    let setting = settings(document.root_element(), target, values)?;
                    let container = if family == "selection" {
                        selection_container(setting, values)?
                    } else {
                        child(setting, T, &container_name).ok_or("settings collection missing")?
                    };
                    let node = if let Some(index) = values.get("index").and_then(Value::as_u64) {
                        container
                            .children()
                            .filter(|node| is(*node, T, "item"))
                            .nth(usize::try_from(index).map_err(|_| "index too large")?)
                            .ok_or("settings index missing")?
                    } else {
                        unique(
                            container.children().filter(|node| {
                                is(*node, T, "item")
                                    && text(*node, T, "field") == string(values, "field")
                            }),
                            string(values, "field"),
                        )?
                    };
                    if key == "direction" && !qtype(node, T, "OrderItemField") {
                        return Err("only Field order accepts direction".to_string());
                    }
                    upsert(
                        xml_text,
                        node.range(),
                        T,
                        tag,
                        &simple(tag, &value_text(value), T),
                        if qtype(node, T, "SelectedItemFolder") {
                            &[
                                "use",
                                "lwsTitle",
                                "item",
                                "placement",
                                "viewMode",
                                "userSettingID",
                                "userSettingPresentation",
                            ]
                        } else {
                            &[
                                "use",
                                "field",
                                "orderType",
                                "presentation",
                                "viewMode",
                                "userSettingID",
                                "userSettingPresentation",
                            ]
                        },
                    )?;
                }
            }
            Ok(())
        }
        "dataSetLink.set" | "dataSetLink.remove" => {
            let select = values
                .get("selector")
                .and_then(Value::as_object)
                .ok_or("selector required")?;
            let link = unique(
                root.children().filter(|node| {
                    is(*node, S, "dataSetLink")
                        && [
                            ("source", "sourceDataSet"),
                            ("destination", "destinationDataSet"),
                            ("sourceExpression", "sourceExpression"),
                            ("destinationExpression", "destinationExpression"),
                        ]
                        .iter()
                        .all(|(key, tag)| text(*node, S, tag) == string(select, key))
                }),
                "dataset link",
            )?;
            if operation.ends_with("remove") {
                let range = link.range();
                xml_text.replace_range(range, "");
                return Ok(());
            }
            for (key, tag) in [
                ("parameter", "parameter"),
                ("condition", "linkConditionExpression"),
                ("startExpression", "startExpression"),
            ] {
                if let Some(value) = values.get(key) {
                    let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
                    let link = unique(
                        document.root_element().children().filter(|node| {
                            is(*node, S, "dataSetLink")
                                && [
                                    ("source", "sourceDataSet"),
                                    ("destination", "destinationDataSet"),
                                    ("sourceExpression", "sourceExpression"),
                                    ("destinationExpression", "destinationExpression"),
                                ]
                                .iter()
                                .all(|(key, tag)| text(*node, S, tag) == string(select, key))
                        }),
                        "dataset link",
                    )?;
                    upsert(
                        xml_text,
                        link.range(),
                        S,
                        tag,
                        &simple(tag, &value_text(value), S),
                        &[
                            "sourceDataSet",
                            "destinationDataSet",
                            "sourceExpression",
                            "destinationExpression",
                            "parameter",
                            "linkConditionExpression",
                            "startExpression",
                        ],
                    )?;
                }
            }
            Ok(())
        }
        "dataSetLink.add" => {
            named(root, S, "dataSet", "name", string(values, "source"))?;
            named(root, S, "dataSet", "name", string(values, "destination"))?;
            let mut body = String::new();
            for (key, tag) in [
                ("source", "sourceDataSet"),
                ("destination", "destinationDataSet"),
                ("sourceExpression", "sourceExpression"),
                ("destinationExpression", "destinationExpression"),
                ("parameter", "parameter"),
                ("condition", "linkConditionExpression"),
                ("startExpression", "startExpression"),
            ] {
                if let Some(value) = values.get(key) {
                    body += &simple(tag, &value_text(value), S);
                }
            }
            insert(
                xml_text,
                root.range(),
                &xml("dataSetLink", &body, S),
                S,
                "dataSetLink",
                ROOT_ORDER,
            )
        }
        _ => Err(format!("unsupported DCS primitive {operation}")),
    }
}

fn patch_settings_parameter(
    xml_text: &mut String,
    target: &Target,
    values: &Map<String, Value>,
    container: &str,
) -> Result<(), String> {
    {
        let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
        let setting = settings(document.root_element(), target, values)?;
        let collection = child(setting, T, container).ok_or("parameter collection missing")?;
        unique(
            collection.children().filter(|node| {
                is(*node, C, "item") && text(*node, C, "parameter") == string(values, "name")
            }),
            string(values, "name"),
        )?;
    }
    for key in [
        "use",
        "value",
        "viewMode",
        "userSettingID",
        "userSettingPresentation",
    ] {
        let Some(value) = values.get(key) else {
            continue;
        };
        let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
        let setting = settings(document.root_element(), target, values)?;
        let container = child(setting, T, container).ok_or("parameter collection missing")?;
        let item = unique(
            container.children().filter(|node| {
                is(*node, C, "item") && text(*node, C, "parameter") == string(values, "name")
            }),
            string(values, "name"),
        )?;
        let ns = if key == "value" || key == "use" { C } else { T };
        let fragment = if key == "value" {
            let declared = values.get("valueType").and_then(Value::as_str);
            let old_kind = if declared.is_none() {
                child(item, C, "value")
                    .and_then(|node| node.attribute((X, "type")).map(|kind| (node, kind)))
                    .map(|(node, kind)| canonical_node_value_type(node, kind))
                    .transpose()?
            } else {
                None
            };
            let kind = declared.or(old_kind.as_deref());
            if container.tag_name().name() == "outputParameters" {
                output_value(string(values, "name"), value, kind)?
            } else {
                scalar_value(key, value, kind, C)?
            }
        } else if key == "userSettingPresentation" {
            mltext(key, value, ns)?
        } else {
            simple(key, &value_text(value), ns)
        };
        // Mixed core/settings children still belong to one XSD sequence.
        if let Some(existing) = child(item, ns, key) {
            if same_fragment(existing, &fragment) {
                continue;
            }
            let range = existing.range();
            xml_text.replace_range(range, &fragment);
        } else {
            insert_mixed(
                xml_text,
                item.range(),
                &fragment,
                key,
                &[
                    "use",
                    "parameter",
                    "value",
                    "viewMode",
                    "userSettingID",
                    "userSettingPresentation",
                ],
            )?;
        }
    }
    Ok(())
}
fn insert_mixed(
    xml_text: &mut String,
    range: Range<usize>,
    fragment: &str,
    tag: &str,
    order: &[&str],
) -> Result<(), String> {
    let document = Document::parse(xml_text).map_err(|e| e.to_string())?;
    let parent = document
        .descendants()
        .find(|node| node.is_element() && node.range() == range)
        .ok_or("parent disappeared")?;
    let rank = order
        .iter()
        .position(|name| *name == tag)
        .unwrap_or(order.len());
    let pos = parent
        .children()
        .find(|node| {
            node.is_element()
                && matches!(node.tag_name().namespace(), Some(C) | Some(T))
                && order
                    .iter()
                    .position(|name| *name == node.tag_name().name())
                    .is_some_and(|other| other > rank)
        })
        .map(|node| node.range().start);
    if let Some(pos) = pos {
        xml_text.insert_str(pos, fragment);
        Ok(())
    } else {
        insert(xml_text, range, fragment, T, tag, order)
    }
}

fn change_query(
    xml_text: &str,
    query: Node<'_, '_>,
    values: &Map<String, Value>,
    patch: bool,
) -> Result<(Range<usize>, String), String> {
    let range = content(xml_text, query)?;
    if !patch {
        let value = string(values, "query");
        let current = query
            .children()
            .filter(Node::is_text)
            .filter_map(|node| node.text())
            .collect::<String>();
        if current == value {
            return Ok((range.clone(), xml_text[range].to_string()));
        }
        if value.trim_start().starts_with('@') {
            return Err("query takes text, not @file".to_string());
        }
        return Ok((range, escape_xml(value)));
    }
    let old = string(values, "find");
    let new = string(values, "replace");
    let mut current = String::new();
    let mut markup = Vec::new();
    for node in query.children() {
        if node.is_text() {
            current.push_str(node.text().unwrap_or_default());
        } else if node.is_comment() || node.is_pi() {
            markup.push((current.len(), xml_text[node.range()].to_string()));
        } else {
            return Err("query contains an XML child element".to_string());
        }
    }
    let count = current.matches(old).count();
    if count == 0 {
        return Err("query patch found no matches".to_string());
    }
    if values.get("once").and_then(Value::as_bool) == Some(true) && count != 1 {
        return Err(format!("once requires one match, found {count}"));
    }
    let patched = current.replace(old, new);
    if patched == current {
        return Ok((range.clone(), xml_text[range].to_string()));
    }
    let replacements = current
        .match_indices(old)
        .map(|(start, _)| start)
        .collect::<Vec<_>>();
    let mut inner = String::new();
    let mut copied = 0;
    for (position, raw) in markup {
        let mut transformed = position;
        for &start in &replacements {
            if position >= start + old.len() {
                transformed = transformed - old.len() + new.len();
            } else if position > start {
                transformed -= position - start;
                break;
            } else {
                break;
            }
        }
        inner.push_str(&escape_xml(&patched[copied..transformed]));
        inner.push_str(&raw);
        copied = transformed;
    }
    inner.push_str(&escape_xml(&patched[copied..]));
    Ok((range, inner))
}

#[cfg(test)]
mod tests;
