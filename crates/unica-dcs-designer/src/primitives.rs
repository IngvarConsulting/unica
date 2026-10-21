//! Local XML changes for DCS operations. Operands stay structured from the
//! operation contract to the writer; no whole-schema intermediate model is used.

use crate::schema::{
    DCS_CORE_NS as C, DCS_SCHEMA_NS as S, DCS_SETTINGS_NS as T, V8_DATA_NS as V,
    XML_SCHEMA_INSTANCE_NS as X,
};
use roxmltree::{Document, Node};
use serde_json::{Map, Value};
use std::ops::Range;

use unica_dcs::arguments::*;
use unica_dcs::operations::DcsIntent;
#[cfg(test)]
pub use unica_dcs::operations::Primitive;

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
const LINK_ORDER: &[&str] = &[
    "sourceDataSet",
    "destinationDataSet",
    "sourceExpression",
    "destinationExpression",
    "parameter",
    "parameterListAllowed",
    "linkConditionExpression",
    "startExpression",
    "required",
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
    crate::xml::xsi_type_matches(node, ns, name)
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

pub(super) fn declares_data_source(root: Node<'_, '_>, name: &str) -> bool {
    root.children()
        .any(|node| is(node, S, "dataSource") && text(node, S, "name") == name)
}

pub(super) fn references_data_source(root: Node<'_, '_>, name: &str) -> bool {
    root.descendants().any(|node| {
        is(node, S, "dataSource")
            && !node.children().any(|child| child.is_element())
            && node
                .children()
                .filter(Node::is_text)
                .filter_map(|child| child.text())
                .collect::<String>()
                == name
    })
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
fn atom_string<'a>(object: &'a Map<String, Value>, key: &str) -> &'a str {
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
    xml(tag, &unica_xml_format::text::escape_xml(value), ns)
}
fn typed(tag: &str, kind: &str, body: &str, ns: &str) -> String {
    xml(tag, body, ns).replacen(
        &format!("<{tag} "),
        &format!("<{tag} xsi:type=\"{kind}\" "),
        1,
    )
}
/// Refusal of a plain-string multilingual value written without a language;
/// [`apply`] reports it as [`ApplyError::TextLanguageRequired`].
const TEXT_LANGUAGE_REQUIRED: &str = "a plain-string multilingual value needs the default language";
/// A `v8:LocalStringType` element from `value`. A plain string is the text
/// of the `lang` language: the `LanguageCode` of the source set's default
/// language (#1257). A language-to-text object is written as given.
fn mltext(tag: &str, value: &Value, ns: &str, lang: Option<&str>) -> Result<String, String> {
    let languages = match value {
        Value::String(s) => vec![(
            lang.ok_or_else(|| TEXT_LANGUAGE_REQUIRED.to_string())?,
            s.as_str(),
        )],
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
                unica_xml_format::text::escape_xml(lang),
                unica_xml_format::text::escape_xml(content)
            )
        })
        .collect::<String>();
    Ok(typed(tag, "v8:LocalStringType", &body, ns))
}
/// The new image of an existing multilingual `existing` element set to
/// `value`, or the fresh [`mltext`] where there is nothing to keep.
///
/// A plain string changes only the item of the default language `lang` and
/// keeps the other translations byte for byte, as `props.set` does for
/// metadata synonyms (#909). A language-to-text object replaces the value as
/// given. An empty string removes the `lang` item while other translations
/// remain; when it is the only item, the value is rewritten as before.
fn mltext_update(
    xml_text: &str,
    existing: Option<Node<'_, '_>>,
    tag: &str,
    value: &Value,
    ns: &str,
    lang: Option<&str>,
) -> Result<String, String> {
    if let (Some(existing), Value::String(new_text)) = (existing, value) {
        let lang = lang.ok_or_else(|| TEXT_LANGUAGE_REQUIRED.to_string())?;
        let items = existing
            .children()
            .filter(|node| is(*node, V, "item"))
            .collect::<Vec<_>>();
        let only_default = items
            .iter()
            .all(|item| text(*item, V, "lang").trim() == lang);
        if !(items.is_empty() || new_text.is_empty() && only_default) {
            return Ok(unica_xml_format::localized::replace_language_value(
                xml_text, existing, "", lang, new_text,
            ));
        }
    }
    mltext(tag, value, ns, lang)
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
pub struct Target {
    pub datasets: Vec<String>,
    pub variant: String,
    /// Цепочка имён элементов структуры варианта: `Setting.V.Item.A.Item.B`
    /// даёт `["A", "B"]` (#1357).
    pub structure: Vec<String>,
    pub terminal: Option<String>,
}
/// Набор данных, который называет адрес: `DataSet.D[.DataSet.M]`.
fn dataset<'a, 'i>(root: Node<'a, 'i>, target: &Target) -> Result<Node<'a, 'i>, String> {
    let path = &target.datasets;
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
/// Контейнер настроек, который называет адрес: сами настройки варианта
/// `Setting.V` или элемент его структуры `Setting.V.Item.A[.Item.B…]`.
/// Элемент находится так же, как его адресует чтение: безымянные элементы
/// прозрачны, имя единственно в своей области.
fn settings<'a, 'i>(root: Node<'a, 'i>, target: &Target) -> Result<Node<'a, 'i>, String> {
    let name = target.variant.as_str();
    if name.is_empty() {
        return Err("address an existing Setting".to_string());
    }
    let variant = unique(
        root.children()
            .filter(|node| is(*node, S, "settingsVariant") && text(*node, T, "name") == name),
        name,
    )?;
    let settings = child(variant, T, "settings").ok_or("variant has no settings")?;
    if target.structure.is_empty() {
        return Ok(settings);
    }
    let rows = crate::schema::dcs_structure_rows(settings);
    let nodes = crate::schema::dcs_structure_nodes(settings);
    let mut scope = None;
    for item in &target.structure {
        scope = Some(
            unica_dcs::structure::dcs_structure_child(&rows, scope, item)
                .ok_or_else(|| {
                    format!(
                        "structure item `{item}` was not found uniquely; read the Item collection of its parent"
                    )
                })?,
        );
    }
    scope
        .and_then(|index| nodes.get(index).copied())
        .ok_or_else(|| "the addressed structure item was not found".to_string())
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
        crate::xml::parse_value_type(reference)?;
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
    lang: Option<&str>,
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
        return mltext(tag, value, ns, lang);
    }
    if kind == "v8:StandardPeriod" {
        let variant = if let Some(object) = value.as_object() {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "variant" | "startDate" | "endDate"))
            {
                return Err("unexpected StandardPeriod property".to_string());
            }
            atom_string(object, "variant")
        } else {
            value
                .as_str()
                .ok_or("period value must be a variant or object")?
        };
        if !crate::xml::is_standard_period_variant(variant) {
            return Err("invalid StandardPeriod variant".to_string());
        }
        let mut body = typed(
            "variant",
            "v8:StandardPeriodVariant",
            &unica_xml_format::text::escape_xml(variant),
            V,
        );
        if variant == "Custom" {
            let empty = Map::new();
            let dates = value.as_object().unwrap_or(&empty);
            for key in ["startDate", "endDate"] {
                let date = match dates.get(key) {
                    Some(value) => value.as_str().ok_or("period dates must be text")?,
                    None => "0001-01-01T00:00:00",
                };
                if !crate::xml::is_date_time_literal(date) {
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
        "xs:decimal" if !crate::xml::is_valid_xs_decimal(&value) => {
            return Err("invalid decimal value".to_string())
        }
        "xs:boolean" if !matches!(value.as_str(), "true" | "false" | "0" | "1") => {
            return Err("invalid boolean value".to_string())
        }
        "xs:dateTime" if !crate::xml::is_date_time_literal(&value) => {
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
    Ok(typed(
        tag,
        kind,
        &unica_xml_format::text::escape_xml(&value),
        ns,
    ))
}
fn appearance_value(name: &str, value: &Value, lang: Option<&str>) -> Result<String, String> {
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
            lang,
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
    scalar_value("value", value, kind, C, lang)
}
fn output_value(
    name: &str,
    value: &Value,
    declared: Option<&str>,
    lang: Option<&str>,
) -> Result<String, String> {
    let expected = output_declared_type(name);
    let declared = declared.map(canonical_value_type).transpose()?;
    if let Some(expected) = expected.filter(|kind| output_enum_values(kind).is_some()) {
        if declared.as_deref().is_some_and(|kind| kind != expected) {
            return Err(format!("output parameter {name} requires {expected}"));
        }
    }
    scalar_value("value", value, declared.as_deref().or(expected), C, lang)
}
fn value_type(value: &str) -> Result<String, String> {
    let mut lines = Vec::new();
    crate::xml::emit_value_type(&mut lines, value, "")?;
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
    values: &ParameterPatch,
) -> Result<Option<String>, String> {
    if let Some(kind) = &values.value_type {
        return canonical_value_type(kind).map(Some);
    }
    let nil = values
        .value
        .as_ref()
        .is_some_and(|value| value.value().is_null());
    let fragment;
    let document;
    let declaration =
        if let Some(kind) = values.type_name.as_deref().filter(|kind| !kind.is_empty()) {
            fragment = value_type(kind)?;
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
fn restriction(tag: &str, value: &Restriction) -> String {
    let mut body = String::new();
    for (key, flag) in [
        ("field", value.field),
        ("condition", value.condition),
        ("group", value.group),
        ("order", value.order),
    ] {
        if let Some(flag) = flag {
            body += &simple(key, &flag.to_string(), S);
        }
    }
    xml(tag, &body, S)
}
fn available_values(
    values: &[AvailableValue],
    kind: Option<&str>,
    lang: Option<&str>,
) -> Result<String, String> {
    let mut body = String::new();
    for item in values {
        let mut entry = scalar_value("value", item.value.value(), kind, S, lang)?;
        if let Some(title) = &item.presentation {
            entry += &mltext("presentation", title.value(), S, lang)?;
        }
        body += &xml("availableValue", &entry, S);
    }
    Ok(body)
}
fn field_fragment(item: &FieldSpec, lang: Option<&str>) -> Result<String, String> {
    let name = &item.data_path;
    if let Some(kind) = item.type_name.as_deref().filter(|kind| !kind.is_empty()) {
        crate::xml::parse_value_type(kind)?;
    }
    let mut body =
        simple("dataPath", name, S) + &simple("field", item.field.as_deref().unwrap_or(name), S);
    if let Some(title) = &item.title {
        body += &mltext("title", title.value(), S, lang)?;
    }
    if let Some(value) = &item.use_restriction {
        body += &restriction("useRestriction", value);
    }
    if let Some(value) = &item.attribute_use_restriction {
        body += &restriction("attributeUseRestriction", value);
    }
    if let Some(expression) = &item.presentation_expression {
        body += &simple("presentationExpression", expression, S);
    }
    if let Some(kind) = item.type_name.as_deref().filter(|kind| !kind.is_empty()) {
        body += &value_type(kind)?;
    }
    Ok(typed("field", "DataSetFieldField", &body, S))
}
fn filter_fragment(item: &FilterSpec, lang: Option<&str>) -> Result<String, String> {
    let mut body = item
        .meta
        .use_flag
        .map(|flag| simple("use", &flag.to_string(), T))
        .unwrap_or_default();
    body += &typed(
        "left",
        "dcscor:Field",
        &unica_xml_format::text::escape_xml(&item.field),
        T,
    );
    body += &simple("comparisonType", item.comparison.as_str(), T);
    if let Some(value) = &item.value {
        body += &scalar_value("right", value.value(), item.value_type.as_deref(), T, lang)?;
    }
    for (key, value) in settings_text_fields(&item.meta).into_iter().skip(1) {
        if let Some(value) = value {
            body += &simple(key, &value, T);
        }
    }
    Ok(typed("item", "dcsset:FilterItemComparison", &body, T))
}
fn total_groups(groups: &[String]) -> String {
    groups
        .iter()
        .map(|group| simple("group", group, S))
        .collect::<String>()
}
fn selection_container<'a, 'i>(
    setting: Node<'a, 'i>,
    parents: &[DcsIndex],
) -> Result<Node<'a, 'i>, String> {
    let mut container = child(setting, T, "selection").ok_or("selection collection missing")?;
    for index in parents {
        container = container
            .children()
            .filter(|node| is(*node, T, "item"))
            .nth(index.get())
            .ok_or("selection parent index was not found")?;
        if !qtype(container, T, "SelectedItemFolder") {
            return Err("selection parent must be a folder".to_string());
        }
    }
    Ok(container)
}
fn group_items(fields: &[String]) -> String {
    let body = fields
        .iter()
        .map(|field| {
            typed(
                "item",
                "dcsset:GroupItemField",
                &(simple("field", field, T)
                    + &simple("groupType", "Items", T)
                    + &simple("periodAdditionType", "None", T)),
                T,
            )
        })
        .collect::<String>();
    xml("groupItems", &body, T)
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
fn settings_text_fields(meta: &SettingsMeta<String>) -> [(&'static str, Option<String>); 4] {
    [
        ("use", meta.use_flag.map(|flag| flag.to_string())),
        (
            "viewMode",
            meta.view_mode.map(|value| value.as_str().to_string()),
        ),
        ("userSettingID", meta.user_setting_id.clone()),
        (
            "userSettingPresentation",
            meta.user_setting_presentation.clone(),
        ),
    ]
}
fn structure_text_fields(meta: &StructureMeta) -> [(&'static str, Option<String>); 8] {
    [
        ("use", meta.settings.use_flag.map(|flag| flag.to_string())),
        (
            "columnsViewMode",
            meta.columns_view_mode
                .map(|value| value.as_str().to_string()),
        ),
        (
            "rowsViewMode",
            meta.rows_view_mode.map(|value| value.as_str().to_string()),
        ),
        (
            "pointsViewMode",
            meta.points_view_mode
                .map(|value| value.as_str().to_string()),
        ),
        (
            "seriesViewMode",
            meta.series_view_mode
                .map(|value| value.as_str().to_string()),
        ),
        (
            "viewMode",
            meta.settings
                .view_mode
                .map(|value| value.as_str().to_string()),
        ),
        ("userSettingID", meta.settings.user_setting_id.clone()),
        (
            "itemsViewMode",
            meta.items_view_mode.map(|value| value.as_str().to_string()),
        ),
    ]
}
fn structure_property(tag: &str, value: &str, kind: StructureKind) -> Result<String, String> {
    if (matches!(tag, "columnsViewMode" | "rowsViewMode") && kind != StructureKind::Table)
        || (matches!(tag, "pointsViewMode" | "seriesViewMode") && kind != StructureKind::Chart)
    {
        return Err(format!("{tag} is incompatible with {}", kind.as_str()));
    }
    Ok(simple(tag, value, T))
}
fn structure_fragment(item: &StructureSpec, lang: Option<&str>) -> Result<String, String> {
    let mut body = item
        .meta
        .settings
        .use_flag
        .map(|flag| simple("use", &flag.to_string(), T))
        .unwrap_or_default();
    if let Some(name) = &item.name {
        body += &simple("name", name, T);
    }
    if item.kind == StructureKind::Group {
        if let Some(fields) = item.group_by.as_ref().filter(|fields| !fields.is_empty()) {
            body += &group_items(fields);
        }
    }
    for (key, value) in structure_text_fields(&item.meta).into_iter().skip(1) {
        if let Some(value) = value {
            body += &structure_property(key, &value, item.kind)?;
        }
    }
    if let Some(value) = &item.meta.settings.user_setting_presentation {
        body += &mltext("userSettingPresentation", value.value(), T, lang)?;
    }
    let wire = match item.kind {
        StructureKind::Group => "dcsset:StructureItemGroup",
        StructureKind::Table => "dcsset:StructureItemTable",
        StructureKind::Chart => "dcsset:StructureItemChart",
    };
    Ok(typed("item", wire, &body, T))
}

/// Apply one decoded operation to a tentative XML image. Publication belongs
/// to the request planner; failure leaves the caller's image unchanged.
pub fn apply(
    xml_text: &mut String,
    intent: &DcsIntent,
    target: &Target,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let mut candidate = xml_text.clone();
    apply_item(&mut candidate, intent, target, lang)?;
    Document::parse(&candidate).map_err(|error| ApplyError::Invalid(error.to_string()))?;
    *xml_text = candidate;
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyError {
    TextLanguageRequired,
    Invalid(String),
    InvalidArgument { path: String, message: String },
}
impl ApplyError {
    fn argument(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::InvalidArgument {
            path: path.into(),
            message: message.into(),
        }
    }
    pub fn argument_path(&self) -> Option<&str> {
        match self {
            Self::InvalidArgument { path, .. } => Some(path),
            Self::TextLanguageRequired | Self::Invalid(_) => None,
        }
    }
}
impl From<String> for ApplyError {
    fn from(message: String) -> Self {
        if message == TEXT_LANGUAGE_REQUIRED {
            Self::TextLanguageRequired
        } else {
            Self::Invalid(message)
        }
    }
}
impl From<&str> for ApplyError {
    fn from(message: &str) -> Self {
        message.to_string().into()
    }
}
impl std::fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TextLanguageRequired => formatter.write_str(TEXT_LANGUAGE_REQUIRED),
            Self::Invalid(message) | Self::InvalidArgument { message, .. } => {
                formatter.write_str(message)
            }
        }
    }
}
fn apply_many<T>(
    xml_text: &mut String,
    items: &[T],
    mut apply: impl FnMut(&mut String, &T, &str) -> Result<(), ApplyError>,
) -> Result<(), ApplyError> {
    for (index, item) in items.iter().enumerate() {
        apply(xml_text, item, &format!("items[{index}]"))?;
    }
    Ok(())
}
fn parse_document(xml_text: &str) -> Result<Document<'_>, ApplyError> {
    Document::parse(xml_text).map_err(|error| ApplyError::Invalid(error.to_string()))
}
fn apply_item(
    xml_text: &mut String,
    intent: &DcsIntent,
    target: &Target,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    if !matches!(intent, DcsIntent::ParameterReorder(_)) {
        crate::schema::require_dcs_root(document.root_element())?;
    }
    match intent {
        DcsIntent::DataSourceAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| add_data_source(xml, item))
        }
        DcsIntent::DataSourceSet(kind) => set_data_source(xml_text, target, *kind),
        DcsIntent::DataSourceRemove(()) => remove_named_schema(
            xml_text,
            target,
            "dataSource",
            "name",
            "address a named DataSource",
        ),
        DcsIntent::DataSetAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_data_set(xml, item, target)
        }),
        DcsIntent::DataSetSet(patch) => set_data_set(xml_text, target, patch),
        DcsIntent::DataSetRemove(()) => {
            let document = parse_document(xml_text)?;
            let range = dataset(document.root_element(), target)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        DcsIntent::FieldAdd(items) => apply_many(xml_text, items, |xml, item, location| {
            add_field(xml, target, item, location, lang)
        }),
        DcsIntent::FieldSet(patch) => set_field(xml_text, target, patch, lang),
        DcsIntent::FieldRemove(()) => {
            let document = parse_document(xml_text)?;
            let ds = dataset(document.root_element(), target)?;
            let name = target.terminal.as_deref().ok_or("address a named Field")?;
            let range = named(ds, S, "field", "dataPath", name)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        DcsIntent::FieldRoleSet(role) => set_field_role(xml_text, target, role),
        DcsIntent::ParameterAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_parameter(xml, item, lang)
        }),
        DcsIntent::ParameterSet(patch) => set_parameter(xml_text, target, patch, lang),
        DcsIntent::ParameterRemove(()) => remove_named_schema(
            xml_text,
            target,
            "parameter",
            "name",
            "address a named Parameter, CalculatedField or Total",
        ),
        DcsIntent::ParameterRename(name) => rename_parameter(xml_text, target, name),
        DcsIntent::ParameterReorder(names) => reorder_parameters(xml_text, names),
        DcsIntent::CalculatedFieldAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_calculated_field(xml, item, lang)
        }),
        DcsIntent::CalculatedFieldRemove(()) => remove_named_schema(
            xml_text,
            target,
            "calculatedField",
            "dataPath",
            "address a named Parameter, CalculatedField or Total",
        ),
        DcsIntent::TotalAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| add_total(xml, item))
        }
        DcsIntent::TotalRemove(()) => remove_named_schema(
            xml_text,
            target,
            "totalField",
            "dataPath",
            "address a named Parameter, CalculatedField or Total",
        ),
        DcsIntent::VariantAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| add_variant(xml, item, lang))
        }
        DcsIntent::VariantSet(title) => set_variant(xml_text, target, title, lang),
        DcsIntent::VariantRemove(()) => remove_variant(xml_text, target),
        DcsIntent::QuerySet(query) => set_query(xml_text, target, query),
        DcsIntent::QueryPatch(patch) => patch_query(xml_text, target, patch),
        DcsIntent::FilterAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_filter(xml, target, item, lang)?;
            Ok(())
        }),
        DcsIntent::FilterSet(patch) => set_filter(xml_text, target, patch, lang),
        DcsIntent::FilterRemove(field) => remove_filter(xml_text, target, field),
        DcsIntent::FilterClear(()) => clear_collection(xml_text, target, "filter"),
        DcsIntent::SelectionAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_selection(xml, target, item, lang)
        }),
        DcsIntent::SelectionClear(()) => clear_collection(xml_text, target, "selection"),
        DcsIntent::OrderAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| add_order(xml, target, item))
        }
        DcsIntent::OrderClear(()) => clear_collection(xml_text, target, "order"),
        DcsIntent::DataParameterAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            write_settings_parameter(xml, target, item, false, false, lang)
        }),
        DcsIntent::DataParameterSet(item) => {
            write_settings_parameter(xml_text, target, item, true, false, lang)
        }
        DcsIntent::OutputParameterSet(item) => {
            write_settings_parameter(xml_text, target, item, true, true, lang)
        }
        DcsIntent::ConditionalAppearanceAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| {
                add_conditional_appearance(xml, target, item, lang)
            })
        }
        DcsIntent::ConditionalAppearanceClear(()) => {
            clear_collection(xml_text, target, "conditionalAppearance")
        }
        DcsIntent::StructureAdd(items) => apply_many(xml_text, items, |xml, item, _| {
            add_structure(xml, target, item, lang)
        }),
        DcsIntent::StructureSet(item) => set_structure(xml_text, target, item, lang),
        DcsIntent::StructurePatch(patch) => patch_structure(xml_text, target, patch, lang),
        DcsIntent::StructureRemove(()) => {
            let document = parse_document(xml_text)?;
            let setting = settings(document.root_element(), target)?;
            if target.structure.is_empty() {
                return Err("address a named structure item: Setting.V.Item.I".into());
            }
            let range = setting.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        DcsIntent::DataSetLinkAdd(items) => {
            apply_many(xml_text, items, |xml, item, _| add_link(xml, item))
        }
        DcsIntent::CalculatedFieldSet(patch) => set_calculated_field(xml_text, target, patch, lang),
        DcsIntent::TotalSet(patch) => set_total(xml_text, target, patch),
        DcsIntent::SelectionSet(patch) => patch_selected_item(
            xml_text,
            target,
            "selection",
            &patch.selector,
            &patch.parent_indexes,
            None,
            &patch.meta,
        ),
        DcsIntent::SelectionRemove(item) => remove_selected_item(
            xml_text,
            target,
            "selection",
            &item.selector,
            &item.parent_indexes,
        ),
        DcsIntent::OrderSet(patch) => patch_selected_item(
            xml_text,
            target,
            "order",
            &patch.selector,
            &[],
            patch.direction,
            &patch.meta,
        ),
        DcsIntent::OrderRemove(selector) => {
            remove_selected_item(xml_text, target, "order", selector, &[])
        }
        DcsIntent::DataParameterRemove(name) => {
            remove_settings_parameter(xml_text, target, "dataParameters", name)
        }
        DcsIntent::OutputParameterRemove(name) => {
            remove_settings_parameter(xml_text, target, "outputParameters", name)
        }
        DcsIntent::ConditionalAppearanceRemove(index) => {
            let document = parse_document(xml_text)?;
            let setting = settings(document.root_element(), target)?;
            let container =
                child(setting, T, "conditionalAppearance").ok_or("settings collection missing")?;
            let item = container
                .children()
                .filter(|node| is(*node, T, "item"))
                .nth(index.get())
                .ok_or("appearance index was not found")?;
            let range = item.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
        DcsIntent::DataSetLinkSet(item) => set_link(xml_text, item),
        DcsIntent::DataSetLinkRemove(selector) => {
            let document = parse_document(xml_text)?;
            let range = find_link(document.root_element(), selector)?.range();
            xml_text.replace_range(range, "");
            Ok(())
        }
    }
}

fn add_data_source(xml_text: &mut String, item: &DataSourceSpec) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    if declares_data_source(root, &item.name) {
        return Err(format!("dataSource {} already exists", item.name).into());
    }
    let fragment = xml(
        "dataSource",
        &(simple("name", &item.name, S) + &simple("dataSourceType", item.kind.as_str(), S)),
        S,
    );
    Ok(insert(
        xml_text,
        root.range(),
        &fragment,
        S,
        "dataSource",
        ROOT_ORDER,
    )?)
}
fn set_data_source(
    xml_text: &mut String,
    target: &Target,
    kind: DataSourceKind,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let name = target
        .terminal
        .as_deref()
        .ok_or("address a named DataSource")?;
    let source = named(document.root_element(), S, "dataSource", "name", name)?;
    Ok(upsert(
        xml_text,
        source.range(),
        S,
        "dataSourceType",
        &simple("dataSourceType", kind.as_str(), S),
        &["name", "dataSourceType"],
    )?)
}
fn remove_named_schema(
    xml_text: &mut String,
    target: &Target,
    tag: &str,
    key: &str,
    missing: &str,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let name = target.terminal.as_deref().ok_or(missing)?;
    let range = named(document.root_element(), S, tag, key, name)?.range();
    xml_text.replace_range(range, "");
    Ok(())
}
fn add_data_set(
    xml_text: &mut String,
    item: &DataSetSpec,
    target: &Target,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    let parent = if target.datasets.is_empty() {
        root
    } else {
        dataset(root, target)?
    };
    let tag = if target.datasets.is_empty() {
        "dataSet"
    } else {
        "item"
    };
    if tag == "item" && !qtype(parent, S, "DataSetUnion") {
        return Err("only a Union dataset accepts child datasets".into());
    }
    if parent
        .children()
        .any(|node| is(node, S, tag) && text(node, S, "name") == item.name)
    {
        return Err(format!("dataset {} already exists", item.name).into());
    }
    match item.kind {
        DataSetKind::Query
            if item.query.as_deref().unwrap_or("").is_empty() || item.object_name.is_some() =>
        {
            return Err("Query requires query and accepts no objectName".into())
        }
        DataSetKind::Object
            if item.object_name.as_deref().unwrap_or("").is_empty() || item.query.is_some() =>
        {
            return Err("Object requires objectName and accepts no query".into())
        }
        DataSetKind::Union
            if item.data_source.is_some() || item.query.is_some() || item.object_name.is_some() =>
        {
            return Err("Union members are composed separately".into())
        }
        DataSetKind::Query | DataSetKind::Object | DataSetKind::Union => {}
    }
    if item
        .query
        .as_deref()
        .unwrap_or("")
        .trim_start()
        .starts_with('@')
    {
        return Err("query takes text, not @file".into());
    }
    let mut body = simple("name", &item.name, S);
    if item.kind != DataSetKind::Union {
        let source = item.data_source.as_deref().unwrap_or("");
        named(root, S, "dataSource", "name", source)?;
        body += &simple("dataSource", source, S);
    }
    match item.kind {
        DataSetKind::Query => body += &simple("query", item.query.as_deref().unwrap_or(""), S),
        DataSetKind::Object => {
            body += &simple("objectName", item.object_name.as_deref().unwrap_or(""), S)
        }
        DataSetKind::Union => {}
    }
    let fragment = typed(tag, &format!("DataSet{}", item.kind.as_str()), &body, S);
    Ok(insert(
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
    )?)
}
fn set_data_set(
    xml_text: &mut String,
    target: &Target,
    patch: &DataSetPatch,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    dataset(document.root_element(), target)?;
    for (key, value) in [
        ("dataSource", patch.data_source.clone()),
        ("objectName", patch.object_name.clone()),
        (
            "autoFillFields",
            patch.auto_fill_fields.map(|flag| flag.to_string()),
        ),
    ] {
        let Some(value) = value else {
            continue;
        };
        let document = parse_document(xml_text)?;
        let root = document.root_element();
        let ds = dataset(root, target)?;
        let query = qtype(ds, S, "DataSetQuery");
        let object = qtype(ds, S, "DataSetObject");
        if key == "dataSource" {
            named(root, S, "dataSource", "name", &value)?;
        }
        if (key == "objectName" && !object)
            || (key == "autoFillFields" && !query)
            || (!object && !query)
        {
            return Err(format!("{key} does not apply to this dataset kind").into());
        }
        upsert(
            xml_text,
            ds.range(),
            S,
            key,
            &simple(key, &value, S),
            DATASET_ORDER,
        )?;
    }
    Ok(())
}
fn add_field(
    xml_text: &mut String,
    target: &Target,
    item: &FieldSpec,
    location: &str,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    if qtype(ds, S, "DataSetQuery") && item.type_name.is_some() {
        return Err(ApplyError::argument(
            format!("{location}.type"),
            "a Query field derives its type from the query; omit type",
        ));
    }
    if ds
        .children()
        .any(|node| is(node, S, "field") && text(node, S, "dataPath") == item.data_path)
    {
        return Ok(());
    }
    let fragment = field_fragment(item, lang)?;
    Ok(insert(
        xml_text,
        ds.range(),
        &fragment,
        S,
        "field",
        DATASET_ORDER,
    )?)
}
fn update_field(
    xml_text: &mut String,
    target: &Target,
    name: &str,
    tag: &str,
    encode: impl FnOnce(&str, Node<'_, '_>) -> Result<String, String>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    let field = named(ds, S, "field", "dataPath", name)?;
    let fragment = encode(xml_text, field)?;
    Ok(upsert(
        xml_text,
        field.range(),
        S,
        tag,
        &fragment,
        FIELD_ORDER,
    )?)
}
fn set_field(
    xml_text: &mut String,
    target: &Target,
    patch: &FieldPatch,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    let name = target.terminal.as_deref().ok_or("address a named Field")?;
    named(ds, S, "field", "dataPath", name)?;
    if qtype(ds, S, "DataSetQuery") && patch.type_name.is_some() {
        return Err(ApplyError::argument(
            "values.type",
            "a Query field derives its type from the query; omit type",
        ));
    }
    if let Some(kind) = patch.type_name.as_deref().filter(|kind| !kind.is_empty()) {
        crate::xml::parse_value_type(kind)?;
    }
    if let Some(value) = &patch.source_field {
        update_field(xml_text, target, name, "field", |_, _| {
            Ok(simple("field", value, S))
        })?;
    }
    if let Some(value) = &patch.title {
        update_field(xml_text, target, name, "title", |xml, field| {
            mltext_update(
                xml,
                child(field, S, "title"),
                "title",
                value.value(),
                S,
                lang,
            )
        })?;
    }
    if let Some(value) = &patch.use_restriction {
        update_field(xml_text, target, name, "useRestriction", |_, _| {
            Ok(restriction("useRestriction", value))
        })?;
    }
    if let Some(value) = &patch.attribute_use_restriction {
        update_field(xml_text, target, name, "attributeUseRestriction", |_, _| {
            Ok(restriction("attributeUseRestriction", value))
        })?;
    }
    if let Some(value) = &patch.presentation_expression {
        update_field(xml_text, target, name, "presentationExpression", |_, _| {
            Ok(simple("presentationExpression", value, S))
        })?;
    }
    if let Some(value) = &patch.type_name {
        update_field(xml_text, target, name, "valueType", |_, _| {
            value_type(value)
        })?;
    }
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
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
fn set_field_role(
    xml_text: &mut String,
    target: &Target,
    role: &FieldRole,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    let name = target.terminal.as_deref().ok_or("address a named Field")?;
    let field = named(ds, S, "field", "dataPath", name)?;
    let properties = [
        ("periodNumber", role.period_number.clone()),
        ("periodType", role.period_type.clone()),
        ("dimension", role.dimension.map(|value| value.to_string())),
        ("parentDimension", role.parent_dimension.clone()),
        ("account", role.account.map(|value| value.to_string())),
        (
            "accountTypeExpression",
            role.account_type_expression.clone(),
        ),
        ("balance", role.balance.map(|value| value.to_string())),
        ("balanceGroupName", role.balance_group_name.clone()),
        ("balanceType", role.balance_type.clone()),
        (
            "accountingBalanceType",
            role.accounting_balance_type.clone(),
        ),
        ("accountField", role.account_field.clone()),
        (
            "ignoreNullValues",
            role.ignore_null_values.map(|value| value.to_string()),
        ),
        ("required", role.required.map(|value| value.to_string())),
        (
            "dimensionAttribute",
            role.dimension_attribute.map(|value| value.to_string()),
        ),
    ];
    let mut body = String::new();
    for (key, value) in properties {
        if let Some(value) = value {
            crate::xml::validate_field_role_value(key, &value)?;
            body += &simple(key, &value, crate::schema::DCS_COMMON_NS);
        }
    }
    Ok(upsert(
        xml_text,
        field.range(),
        S,
        "role",
        &xml("role", &body, S),
        FIELD_ORDER,
    )?)
}
fn set_query(xml_text: &mut String, target: &Target, value: &str) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    if !qtype(ds, S, "DataSetQuery") {
        return Err("only a Query dataset has query text".into());
    }
    let query = child(ds, S, "query").ok_or("dataset has no query")?;
    let range = content(xml_text, query)?;
    let current = query
        .children()
        .filter(Node::is_text)
        .filter_map(|node| node.text())
        .collect::<String>();
    if current == value {
        return Ok(());
    }
    if value.trim_start().starts_with('@') {
        return Err("query takes text, not @file".into());
    }
    xml_text.replace_range(range, &unica_xml_format::text::escape_xml(value));
    Ok(())
}
fn patch_query(
    xml_text: &mut String,
    target: &Target,
    patch: &QueryPatch,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let ds = dataset(document.root_element(), target)?;
    if !qtype(ds, S, "DataSetQuery") {
        return Err("only a Query dataset has query text".into());
    }
    let query = child(ds, S, "query").ok_or("dataset has no query")?;
    let (range, replacement) = change_query(xml_text, query, patch)?;
    xml_text.replace_range(range, &replacement);
    Ok(())
}
fn change_query(
    xml_text: &str,
    query: Node<'_, '_>,
    patch: &QueryPatch,
) -> Result<(Range<usize>, String), String> {
    let range = content(xml_text, query)?;
    let old = patch.find.as_str();
    let new = patch.replace.as_str();
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
    if patch.once && count != 1 {
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
        inner.push_str(&unica_xml_format::text::escape_xml(
            &patched[copied..transformed],
        ));
        inner.push_str(&raw);
        copied = transformed;
    }
    inner.push_str(&unica_xml_format::text::escape_xml(&patched[copied..]));
    Ok((range, inner))
}

fn add_parameter(
    xml_text: &mut String,
    item: &ParameterSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    if root
        .children()
        .any(|node| is(node, S, "parameter") && text(node, S, "name") == item.name)
    {
        return Ok(());
    }
    let values = &item.properties;
    let mut body = simple("name", &item.name, S);
    if let Some(title) = &values.title {
        body += &mltext("title", title.value(), S, lang)?;
    }
    if let Some(kind) = values.type_name.as_deref().filter(|kind| !kind.is_empty()) {
        body += &value_type(kind)?;
    }
    if let Some(value) = &values.value {
        let kind = parameter_value_type(None, values)?;
        body += &scalar_value("value", value.value(), kind.as_deref(), S, lang)?;
    }
    if let Some(list) = &values.values {
        if values.value.is_some() {
            return Err("value and values are mutually exclusive".into());
        }
        let kind = parameter_value_type(None, values)?;
        for value in list {
            body += &scalar_value("value", value.value(), kind.as_deref(), S, lang)?;
        }
    }
    if let Some(flag) = values.use_restriction {
        body += &simple("useRestriction", &flag.to_string(), S);
    }
    if let Some(expression) = values
        .expression
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        body += &simple("expression", expression, S);
    }
    if let Some(list) = &values.available_values {
        let kind = parameter_value_type(None, values)?;
        body += &available_values(list, kind.as_deref(), lang)?;
    }
    for (key, value) in parameter_flags(values) {
        if let Some(value) = value {
            body += &simple(key, &value, S);
        }
    }
    Ok(insert(
        xml_text,
        root.range(),
        &xml("parameter", &body, S),
        S,
        "parameter",
        ROOT_ORDER,
    )?)
}
fn parameter_flags(patch: &ParameterPatch) -> [(&'static str, Option<String>); 4] {
    [
        (
            "valueListAllowed",
            patch.value_list_allowed.map(|value| value.to_string()),
        ),
        (
            "availableAsField",
            patch.available_as_field.map(|value| value.to_string()),
        ),
        (
            "denyIncompleteValues",
            patch.deny_incomplete_values.map(|value| value.to_string()),
        ),
        (
            "use",
            patch.use_mode.map(|value| value.as_str().to_string()),
        ),
    ]
}
fn update_parameter(
    xml_text: &mut String,
    name: &str,
    tag: &str,
    encode: impl FnOnce(&str, Node<'_, '_>) -> Result<String, String>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let parameter = named(document.root_element(), S, "parameter", "name", name)?;
    let fragment = encode(xml_text, parameter)?;
    Ok(upsert(
        xml_text,
        parameter.range(),
        S,
        tag,
        &fragment,
        PARAMETER_ORDER,
    )?)
}
enum ParameterValues<'a> {
    One(&'a ValueAtom),
    Many(&'a [ValueAtom]),
    Available(&'a [AvailableValue]),
}
fn replace_parameter_values(
    xml_text: &mut String,
    name: &str,
    values: &ParameterPatch,
    payload: ParameterValues<'_>,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let parameter = named(document.root_element(), S, "parameter", "name", name)?;
    if values.value.is_some() && values.values.is_some() {
        return Err("value and values are mutually exclusive".into());
    }
    let kind = parameter_value_type(Some(parameter), values)?;
    let (tag, fragment) = match &payload {
        ParameterValues::One(value) => (
            "value",
            scalar_value("value", value.value(), kind.as_deref(), S, lang)?,
        ),
        ParameterValues::Many(values) => (
            "value",
            values
                .iter()
                .map(|value| scalar_value("value", value.value(), kind.as_deref(), S, lang))
                .collect::<Result<Vec<_>, _>>()?
                .join(""),
        ),
        ParameterValues::Available(values) => (
            "availableValue",
            available_values(values, kind.as_deref(), lang)?,
        ),
    };
    let old = parameter
        .children()
        .filter(|node| is(*node, S, tag))
        .collect::<Vec<_>>();
    if let ([existing], ParameterValues::One(value)) = (old.as_slice(), &payload) {
        if value.value().is_string()
            && fragment.starts_with("<value xsi:type=\"v8:LocalStringType\"")
        {
            let range = existing.range();
            let updated =
                mltext_update(xml_text, Some(*existing), "value", value.value(), S, lang)?;
            if xml_text[range.clone()] != updated {
                xml_text.replace_range(range, &updated);
            }
            return Ok(());
        }
    }
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
        return Ok(());
    }
    let ranges = old.iter().map(|node| node.range()).collect();
    remove_ranges(xml_text, ranges);
    if !fragment.is_empty() {
        let document = parse_document(xml_text)?;
        let parameter = named(document.root_element(), S, "parameter", "name", name)?;
        insert(
            xml_text,
            parameter.range(),
            &fragment,
            S,
            tag,
            PARAMETER_ORDER,
        )?;
    }
    Ok(())
}
fn set_parameter(
    xml_text: &mut String,
    target: &Target,
    patch: &ParameterPatch,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let name = target
        .terminal
        .as_deref()
        .ok_or("address a named Parameter")?;
    let document = parse_document(xml_text)?;
    named(document.root_element(), S, "parameter", "name", name)?;
    if let Some(value) = &patch.title {
        update_parameter(xml_text, name, "title", |xml, node| {
            mltext_update(
                xml,
                child(node, S, "title"),
                "title",
                value.value(),
                S,
                lang,
            )
        })?;
    }
    if let Some(value) = &patch.type_name {
        update_parameter(xml_text, name, "valueType", |_, _| value_type(value))?;
    }
    if let Some(value) = &patch.value {
        replace_parameter_values(xml_text, name, patch, ParameterValues::One(value), lang)?;
    }
    if let Some(values) = &patch.values {
        replace_parameter_values(xml_text, name, patch, ParameterValues::Many(values), lang)?;
    }
    if let Some(value) = &patch.expression {
        update_parameter(xml_text, name, "expression", |_, _| {
            Ok(simple("expression", value, S))
        })?;
    }
    if let Some(value) = patch.use_restriction {
        update_parameter(xml_text, name, "useRestriction", |_, _| {
            Ok(simple("useRestriction", &value.to_string(), S))
        })?;
    }
    if let Some(values) = &patch.available_values {
        replace_parameter_values(
            xml_text,
            name,
            patch,
            ParameterValues::Available(values),
            lang,
        )?;
    }
    for (key, value) in parameter_flags(patch) {
        if let Some(value) = value {
            update_parameter(xml_text, name, key, |_, _| Ok(simple(key, &value, S)))?;
        }
    }
    Ok(())
}
fn rename_parameter(xml_text: &mut String, target: &Target, new: &str) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    let name = target
        .terminal
        .as_deref()
        .ok_or("address a named Parameter")?;
    if root
        .children()
        .any(|node| is(node, S, "parameter") && text(node, S, "name") == new)
    {
        return Err("parameter name already exists".into());
    }
    let parameter = named(root, S, "parameter", "name", name)?;
    Ok(upsert(
        xml_text,
        parameter.range(),
        S,
        "name",
        &simple("name", new, S),
        PARAMETER_ORDER,
    )?)
}
fn reorder_parameters(xml_text: &mut String, names: &[String]) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let parameters = document
        .root_element()
        .children()
        .filter(|node| is(*node, S, "parameter"))
        .collect::<Vec<_>>();
    if names.len() != parameters.len() {
        return Err("reorder needs every parameter exactly once".into());
    }
    let mut used = std::collections::HashSet::new();
    let mut replacements = Vec::new();
    for (slot, name) in parameters.iter().zip(names) {
        if !used.insert(name) {
            return Err("duplicate parameter in reorder".into());
        }
        let parameter = unique(
            parameters
                .iter()
                .copied()
                .filter(|node| text(*node, S, "name") == *name),
            name,
        )?;
        replacements.push((slot.range(), xml_text[parameter.range()].to_string()));
    }
    for (range, replacement) in replacements.into_iter().rev() {
        xml_text.replace_range(range, &replacement);
    }
    Ok(())
}
fn add_calculated_field(
    xml_text: &mut String,
    item: &CalculatedFieldSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    if root
        .children()
        .any(|node| is(node, S, "calculatedField") && text(node, S, "dataPath") == item.name)
    {
        return Ok(());
    }
    let mut body = simple("dataPath", &item.name, S) + &simple("expression", &item.expression, S);
    if let Some(title) = &item.title {
        body += &mltext("title", title.value(), S, lang)?;
    }
    if let Some(value) = &item.use_restriction {
        body += &restriction("useRestriction", value);
    }
    if let Some(kind) = item.type_name.as_deref().filter(|kind| !kind.is_empty()) {
        body += &value_type(kind)?;
    }
    Ok(insert(
        xml_text,
        root.range(),
        &xml("calculatedField", &body, S),
        S,
        "calculatedField",
        ROOT_ORDER,
    )?)
}
fn add_total(xml_text: &mut String, item: &TotalSpec) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    if root
        .children()
        .any(|node| is(node, S, "totalField") && text(node, S, "dataPath") == item.field)
    {
        return Ok(());
    }
    let expression = item
        .expression
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Сумма({})", item.field));
    let mut body = simple("dataPath", &item.field, S) + &simple("expression", &expression, S);
    if let Some(groups) = &item.groups {
        body += &total_groups(groups);
    }
    Ok(insert(
        xml_text,
        root.range(),
        &xml("totalField", &body, S),
        S,
        "totalField",
        ROOT_ORDER,
    )?)
}
fn update_calculated(
    xml_text: &mut String,
    target: &Target,
    tag: &str,
    property: &str,
    encode: impl FnOnce(&str, Node<'_, '_>) -> Result<String, String>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let path = target
        .terminal
        .as_deref()
        .ok_or("address a named CalculatedField or Total")?;
    let node = named(document.root_element(), S, tag, "dataPath", path)?;
    let fragment = encode(xml_text, node)?;
    Ok(upsert(
        xml_text,
        node.range(),
        S,
        property,
        &fragment,
        &[
            "dataPath",
            "expression",
            "title",
            "useRestriction",
            "valueType",
        ],
    )?)
}
fn set_calculated_field(
    xml_text: &mut String,
    target: &Target,
    patch: &CalculatedFieldPatch,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let path = target
        .terminal
        .as_deref()
        .ok_or("address a named CalculatedField or Total")?;
    named(
        document.root_element(),
        S,
        "calculatedField",
        "dataPath",
        path,
    )?;
    if let Some(value) = &patch.expression {
        update_calculated(xml_text, target, "calculatedField", "expression", |_, _| {
            Ok(simple("expression", value, S))
        })?;
    }
    if let Some(value) = &patch.title {
        update_calculated(xml_text, target, "calculatedField", "title", |xml, node| {
            mltext_update(
                xml,
                child(node, S, "title"),
                "title",
                value.value(),
                S,
                lang,
            )
        })?;
    }
    if let Some(value) = &patch.type_name {
        update_calculated(xml_text, target, "calculatedField", "valueType", |_, _| {
            value_type(value)
        })?;
    }
    if let Some(value) = &patch.use_restriction {
        update_calculated(
            xml_text,
            target,
            "calculatedField",
            "useRestriction",
            |_, _| Ok(restriction("useRestriction", value)),
        )?;
    }
    Ok(())
}
fn set_total(xml_text: &mut String, target: &Target, patch: &TotalPatch) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let path = target
        .terminal
        .as_deref()
        .ok_or("address a named CalculatedField or Total")?;
    named(document.root_element(), S, "totalField", "dataPath", path)?;
    if let Some(value) = &patch.expression {
        update_calculated(xml_text, target, "totalField", "expression", |_, _| {
            Ok(simple("expression", value, S))
        })?;
    }
    if let Some(groups) = &patch.groups {
        let document = parse_document(xml_text)?;
        let node = named(document.root_element(), S, "totalField", "dataPath", path)?;
        let fragment = total_groups(groups);
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
            return Ok(());
        }
        remove_ranges(xml_text, old.iter().map(|node| node.range()).collect());
        let document = parse_document(xml_text)?;
        let node = named(document.root_element(), S, "totalField", "dataPath", path)?;
        insert(
            xml_text,
            node.range(),
            &fragment,
            S,
            "group",
            &["dataPath", "expression", "group"],
        )?;
    }
    Ok(())
}

fn add_variant(
    xml_text: &mut String,
    item: &VariantSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    if root
        .children()
        .any(|node| is(node, S, "settingsVariant") && text(node, T, "name") == item.name)
    {
        return Ok(());
    }
    let body = simple("name", &item.name, T)
        + &mltext("presentation", item.title.value(), T, lang)?
        + &xml("settings", "", T);
    Ok(insert(
        xml_text,
        root.range(),
        &xml("settingsVariant", &body, S),
        S,
        "settingsVariant",
        ROOT_ORDER,
    )?)
}
fn variant<'a, 'i>(root: Node<'a, 'i>, target: &Target) -> Result<Node<'a, 'i>, String> {
    if target.variant.is_empty() {
        return Err("address an existing Setting".to_string());
    }
    unique(
        root.children().filter(|node| {
            is(*node, S, "settingsVariant") && text(*node, T, "name") == target.variant
        }),
        &target.variant,
    )
}
fn set_variant(
    xml_text: &mut String,
    target: &Target,
    title: &LocalizedText,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let node = variant(document.root_element(), target)?;
    let fragment = mltext_update(
        xml_text,
        child(node, T, "presentation"),
        "presentation",
        title.value(),
        T,
        lang,
    )?;
    Ok(upsert(
        xml_text,
        node.range(),
        T,
        "presentation",
        &fragment,
        &["name", "presentation", "settings"],
    )?)
}
fn remove_variant(xml_text: &mut String, target: &Target) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let range = variant(document.root_element(), target)?.range();
    xml_text.replace_range(range, "");
    Ok(())
}
fn insert_settings_item(
    xml_text: &mut String,
    target: &Target,
    container: &str,
    fragment: &str,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    if let Some(node) = child(setting, T, container) {
        Ok(insert(xml_text, node.range(), fragment, T, "item", &[])?)
    } else {
        Ok(insert(
            xml_text,
            setting.range(),
            &xml(container, fragment, T),
            T,
            container,
            structure_order(setting),
        )?)
    }
}
fn add_filter(
    xml_text: &mut String,
    target: &Target,
    item: &FilterSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    settings(document.root_element(), target)?;
    let fragment = filter_fragment(item, lang)?;
    insert_settings_item(xml_text, target, "filter", &fragment)
}
fn add_selection(
    xml_text: &mut String,
    target: &Target,
    item: &SelectionSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let body = match item.kind {
        SelectionKind::Field => {
            if item.title.is_some() {
                return Err("title belongs to Folder selection".into());
            }
            let field = item
                .field
                .as_deref()
                .filter(|field| !field.is_empty())
                .ok_or("Field selection requires field")?;
            simple("field", field, T)
        }
        SelectionKind::Auto => {
            if item.field.is_some() || item.title.is_some() {
                return Err("Auto selection has no field/title".into());
            }
            String::new()
        }
        SelectionKind::Folder => {
            if item.field.is_some() {
                return Err("Folder has no field; add its children separately".into());
            }
            mltext(
                "lwsTitle",
                item.title.as_ref().ok_or("Folder requires title")?.value(),
                T,
                lang,
            )? + &simple("placement", "Auto", T)
        }
    };
    let fragment = typed(
        "item",
        &format!("dcsset:SelectedItem{}", item.kind.as_str()),
        &body,
        T,
    );
    if !item.parent_indexes.is_empty() {
        let folder = selection_container(setting, &item.parent_indexes)?;
        return Ok(insert(
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
        )?);
    }
    insert_settings_item(xml_text, target, "selection", &fragment)
}
fn add_order(xml_text: &mut String, target: &Target, item: &OrderSpec) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    settings(document.root_element(), target)?;
    let body = match item.kind {
        OrderKind::Auto => {
            if item.field.is_some() {
                return Err("Auto order has no field".into());
            }
            String::new()
        }
        OrderKind::Field => {
            let field = item
                .field
                .as_deref()
                .filter(|field| !field.is_empty())
                .ok_or("Field order requires field")?;
            simple("field", field, T)
                + &simple(
                    "orderType",
                    item.direction
                        .expect("Field direction normalized by the DCS contract")
                        .as_str(),
                    T,
                )
        }
    };
    let fragment = typed(
        "item",
        match item.kind {
            OrderKind::Auto => "dcsset:OrderItemAuto",
            OrderKind::Field => "dcsset:OrderItemField",
        },
        &body,
        T,
    );
    insert_settings_item(xml_text, target, "order", &fragment)
}
fn add_conditional_appearance(
    xml_text: &mut String,
    target: &Target,
    item: &ConditionalAppearanceSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    settings(document.root_element(), target)?;
    let fields = item
        .fields
        .iter()
        .map(|field| xml("item", &simple("field", field, T), T))
        .collect::<String>();
    let mut appearance = String::new();
    for (key, value) in &item.appearance {
        appearance += &typed(
            "item",
            "dcsset:SettingsParameterValue",
            &(simple("parameter", key, C) + &appearance_value(key, value.value(), lang)?),
            C,
        );
    }
    let mut entry = item
        .use_flag
        .map(|flag| simple("use", &flag.to_string(), T))
        .unwrap_or_default()
        + &xml("selection", &fields, T);
    if let Some(filters) = &item.filters {
        let filters = filters
            .iter()
            .map(|filter| filter_fragment(filter, lang))
            .collect::<Result<Vec<_>, _>>()?
            .join("");
        entry += &xml("filter", &filters, T);
    }
    entry += &xml("appearance", &appearance, T);
    insert_settings_item(
        xml_text,
        target,
        "conditionalAppearance",
        &xml("item", &entry, T),
    )
}
fn find_filter<'a, 'i>(setting: Node<'a, 'i>, field: &str) -> Result<Node<'a, 'i>, String> {
    let filter = child(setting, T, "filter").ok_or("filter section missing")?;
    unique(
        filter
            .children()
            .filter(|node| is(*node, T, "item") && text(*node, T, "left") == field),
        field,
    )
}
fn remove_filter(xml_text: &mut String, target: &Target, field: &str) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let range = find_filter(setting, field)?.range();
    xml_text.replace_range(range, "");
    Ok(())
}
fn set_filter(
    xml_text: &mut String,
    target: &Target,
    patch: &FilterPatch,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    find_filter(setting, &patch.field)?;
    let mut properties = settings_text_fields(&patch.meta)
        .into_iter()
        .collect::<Vec<_>>();
    properties.insert(
        1,
        (
            "comparisonType",
            patch.comparison.map(|value| value.as_str().to_string()),
        ),
    );
    properties.insert(2, ("right", None));
    for (tag, value) in properties {
        if value.is_none() && !(tag == "right" && patch.value.is_some()) {
            continue;
        }
        let document = parse_document(xml_text)?;
        let setting = settings(document.root_element(), target)?;
        let item = find_filter(setting, &patch.field)?;
        let fragment = if tag == "right" {
            scalar_value(
                tag,
                patch.value.as_ref().expect("present value").value(),
                patch.value_type.as_deref(),
                T,
                lang,
            )?
        } else {
            simple(tag, value.as_deref().expect("present text"), T)
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
fn clear_collection(xml_text: &mut String, target: &Target, tag: &str) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    if let Some(container) = child(setting, T, tag) {
        let ranges = container
            .children()
            .filter(Node::is_element)
            .map(|node| node.range())
            .collect();
        remove_ranges(xml_text, ranges);
    }
    Ok(())
}
fn validate_selection_key(selector: &SelectionKey) -> Result<(), String> {
    if selector.field.is_some() == selector.index.is_some() {
        return Err("supply exactly one field or index selector".to_string());
    }
    Ok(())
}
fn selected_container<'a, 'i>(
    setting: Node<'a, 'i>,
    tag: &str,
    parents: &[DcsIndex],
) -> Result<Node<'a, 'i>, String> {
    if tag == "selection" {
        selection_container(setting, parents)
    } else {
        child(setting, T, tag).ok_or_else(|| "settings collection missing".to_string())
    }
}
fn selected_item<'a, 'i>(
    container: Node<'a, 'i>,
    selector: &SelectionKey,
    missing: &str,
) -> Result<Node<'a, 'i>, String> {
    if let Some(index) = selector.index {
        container
            .children()
            .filter(|node| is(*node, T, "item"))
            .nth(index.get())
            .ok_or_else(|| missing.to_string())
    } else {
        let field = selector.field.as_deref().unwrap_or("");
        unique(
            container
                .children()
                .filter(|node| is(*node, T, "item") && text(*node, T, "field") == field),
            field,
        )
    }
}
fn remove_selected_item(
    xml_text: &mut String,
    target: &Target,
    tag: &str,
    selector: &SelectionKey,
    parents: &[DcsIndex],
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    validate_selection_key(selector)?;
    let container = selected_container(setting, tag, parents)?;
    let range = selected_item(container, selector, "appearance index was not found")?.range();
    xml_text.replace_range(range, "");
    Ok(())
}
fn patch_selected_item(
    xml_text: &mut String,
    target: &Target,
    tag: &str,
    selector: &SelectionKey,
    parents: &[DcsIndex],
    direction: Option<OrderDirection>,
    meta: &SettingsMeta<String>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    validate_selection_key(selector)?;
    selected_item(
        selected_container(setting, tag, parents)?,
        selector,
        "appearance index was not found",
    )?;
    let mut properties = settings_text_fields(meta).into_iter().collect::<Vec<_>>();
    properties.insert(
        1,
        (
            "orderType",
            direction.map(|value| value.as_str().to_string()),
        ),
    );
    for (property, value) in properties {
        let Some(value) = value else {
            continue;
        };
        let document = parse_document(xml_text)?;
        let setting = settings(document.root_element(), target)?;
        let node = selected_item(
            selected_container(setting, tag, parents)?,
            selector,
            "settings index missing",
        )?;
        if property == "orderType" && !qtype(node, T, "OrderItemField") {
            return Err("only Field order accepts direction".into());
        }
        let order: &[&str] = if qtype(node, T, "SelectedItemFolder") {
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
        };
        upsert(
            xml_text,
            node.range(),
            T,
            property,
            &simple(property, &value, T),
            order,
        )?;
    }
    Ok(())
}
fn remove_settings_parameter(
    xml_text: &mut String,
    target: &Target,
    tag: &str,
    name: &str,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let container = child(setting, T, tag).ok_or("settings collection missing")?;
    let item = unique(
        container
            .children()
            .filter(|node| is(*node, C, "item") && text(*node, C, "parameter") == name),
        name,
    )?;
    let range = item.range();
    xml_text.replace_range(range, "");
    Ok(())
}
fn write_settings_parameter(
    xml_text: &mut String,
    target: &Target,
    item: &SettingsParameterSpec,
    is_set: bool,
    output: bool,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let container = if output {
        "outputParameters"
    } else {
        "dataParameters"
    };
    let exists = child(setting, T, container).is_some_and(|collection| {
        collection
            .children()
            .any(|node| is(node, C, "item") && text(node, C, "parameter") == item.name)
    });
    if is_set && exists {
        return patch_settings_parameter(xml_text, target, item, container, lang);
    }
    let value = item.value.as_ref().ok_or(if output {
        "output parameter value required"
    } else {
        "data parameter value required"
    })?;
    let mut body = item
        .meta
        .use_flag
        .map(|flag| simple("use", &flag.to_string(), C))
        .unwrap_or_default()
        + &simple("parameter", &item.name, C);
    body += &if output {
        output_value(&item.name, value.value(), item.value_type.as_deref(), lang)?
    } else {
        scalar_value("value", value.value(), item.value_type.as_deref(), C, lang)?
    };
    if let Some(value) = item.meta.view_mode {
        body += &simple("viewMode", value.as_str(), T);
    }
    if let Some(value) = &item.meta.user_setting_id {
        body += &simple("userSettingID", value, T);
    }
    if let Some(value) = &item.meta.user_setting_presentation {
        body += &simple("userSettingPresentation", &value_text(value.value()), T);
    }
    let fragment = typed("item", "dcsset:SettingsParameterValue", &body, C);
    if exists {
        return patch_settings_parameter(xml_text, target, item, container, lang);
    }
    insert_settings_item(xml_text, target, container, &fragment)
}
fn patch_settings_parameter(
    xml_text: &mut String,
    target: &Target,
    values: &SettingsParameterSpec,
    container: &str,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let collection = child(setting, T, container).ok_or("parameter collection missing")?;
    unique(
        collection
            .children()
            .filter(|node| is(*node, C, "item") && text(*node, C, "parameter") == values.name),
        &values.name,
    )?;
    let properties = [
        ("use", values.meta.use_flag.map(|flag| flag.to_string())),
        ("value", None),
        (
            "viewMode",
            values
                .meta
                .view_mode
                .map(|value| value.as_str().to_string()),
        ),
        ("userSettingID", values.meta.user_setting_id.clone()),
        ("userSettingPresentation", None),
    ];
    for (key, value) in properties {
        if value.is_none()
            && !(key == "value" && values.value.is_some())
            && !(key == "userSettingPresentation"
                && values.meta.user_setting_presentation.is_some())
        {
            continue;
        }
        let document = parse_document(xml_text)?;
        let setting = settings(document.root_element(), target)?;
        let collection = child(setting, T, container).ok_or("parameter collection missing")?;
        let item = unique(
            collection
                .children()
                .filter(|node| is(*node, C, "item") && text(*node, C, "parameter") == values.name),
            &values.name,
        )?;
        let ns = if key == "value" || key == "use" { C } else { T };
        let fragment = if key == "value" {
            let value = values.value.as_ref().expect("present value").value();
            let declared = values.value_type.as_deref();
            let old_kind = if declared.is_none() {
                child(item, C, "value")
                    .and_then(|node| node.attribute((X, "type")).map(|kind| (node, kind)))
                    .map(|(node, kind)| canonical_node_value_type(node, kind))
                    .transpose()?
            } else {
                None
            };
            let kind = declared.or(old_kind.as_deref());
            let fragment = if container == "outputParameters" {
                output_value(&values.name, value, kind, lang)?
            } else {
                scalar_value(key, value, kind, C, lang)?
            };
            if value.is_string()
                && fragment.starts_with(&format!("<{key} xsi:type=\"v8:LocalStringType\""))
            {
                mltext_update(xml_text, child(item, C, key), key, value, C, lang)?
            } else {
                fragment
            }
        } else if key == "userSettingPresentation" {
            mltext_update(
                xml_text,
                child(item, ns, key),
                key,
                values
                    .meta
                    .user_setting_presentation
                    .as_ref()
                    .expect("present presentation")
                    .value(),
                ns,
                lang,
            )?
        } else {
            simple(key, value.as_deref().expect("present text"), ns)
        };
        if let Some(existing) = child(item, ns, key) {
            if same_fragment(existing, &fragment) {
                continue;
            }
            xml_text.replace_range(existing.range(), &fragment);
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
    let document = Document::parse(xml_text).map_err(|error| error.to_string())?;
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

fn add_structure(
    xml_text: &mut String,
    target: &Target,
    item: &StructureSpec,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let parent = settings(document.root_element(), target)?;
    let mut variant_settings = parent;
    while !is(variant_settings, T, "settings") {
        variant_settings = variant_settings
            .parent_element()
            .ok_or("variant has no settings")?;
    }
    if let Some(name) = item.name.as_deref().filter(|name| !name.is_empty()) {
        if variant_settings.descendants().any(|node| {
            (is(node, T, "item")
                || ["row", "column", "point", "series"]
                    .iter()
                    .any(|tag| is(node, T, tag)))
                && text(node, T, "name") == name
        }) {
            return Err("structure name already exists".into());
        }
    }
    let fragment = structure_fragment(item, lang)?;
    if let Some(axis) = item.axis {
        let axis = axis.as_str();
        if !((qtype(parent, T, "StructureItemTable") && matches!(axis, "row" | "column"))
            || (qtype(parent, T, "StructureItemChart") && matches!(axis, "point" | "series")))
        {
            return Err("axis is incompatible with the parent structure kind".into());
        }
        if item.kind != StructureKind::Group {
            return Err("a table/chart axis must be a group".into());
        }
        let parsed = Document::parse(&fragment).map_err(|error| error.to_string())?;
        let inner = content(&fragment, parsed.root_element())?;
        let fragment = xml(axis, &fragment[inner], T);
        Ok(insert(
            xml_text,
            parent.range(),
            &fragment,
            T,
            axis,
            structure_order(parent),
        )?)
    } else {
        Ok(insert(
            xml_text,
            parent.range(),
            &fragment,
            T,
            "item",
            structure_order(parent),
        )?)
    }
}
fn set_structure(
    xml_text: &mut String,
    target: &Target,
    item: &StructureSet,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let ranges = setting
        .children()
        .filter(|node| is(*node, T, "item"))
        .map(|node| node.range())
        .collect();
    remove_ranges(xml_text, ranges);
    let document = parse_document(xml_text)?;
    let setting = settings(document.root_element(), target)?;
    let mut fragment = structure_fragment(&item.item, lang)?;
    if item
        .item
        .group_by
        .as_ref()
        .is_some_and(|fields| !fields.is_empty())
        && item.details
    {
        let details = typed("item", "dcsset:StructureItemGroup", "", T);
        let close = fragment.rfind("</item>").ok_or("structure close missing")?;
        fragment.insert_str(close, &details);
    }
    Ok(insert(
        xml_text,
        setting.range(),
        &fragment,
        T,
        "item",
        structure_order(setting),
    )?)
}
fn patch_structure(
    xml_text: &mut String,
    target: &Target,
    patch: &StructurePatch,
    lang: Option<&str>,
) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let group = settings(document.root_element(), target)?;
    if target.structure.is_empty() {
        return Err("address a named structure item: Setting.V.Item.I".into());
    }
    let kind = if qtype(group, T, "StructureItemTable") {
        StructureKind::Table
    } else if qtype(group, T, "StructureItemChart") {
        StructureKind::Chart
    } else if qtype(group, T, "StructureItemGroup")
        || ["row", "column", "point", "series"]
            .iter()
            .any(|tag| is(group, T, tag))
    {
        StructureKind::Group
    } else {
        return Err("structure type is unsupported for patch".into());
    };
    if let Some(fields) = &patch.group_by {
        if kind != StructureKind::Group {
            return Err("groupBy applies to a group or named table/chart axis".into());
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
                &group_items(fields),
                GROUP_ORDER,
            )?;
        }
    }
    for (key, value) in structure_text_fields(&patch.meta) {
        let Some(value) = value else {
            continue;
        };
        let document = parse_document(xml_text)?;
        let group = settings(document.root_element(), target)?;
        let fragment = structure_property(key, &value, kind)?;
        upsert(
            xml_text,
            group.range(),
            T,
            key,
            &fragment,
            structure_order(group),
        )?;
    }
    if let Some(value) = &patch.meta.settings.user_setting_presentation {
        let document = parse_document(xml_text)?;
        let group = settings(document.root_element(), target)?;
        let fragment = mltext_update(
            xml_text,
            child(group, T, "userSettingPresentation"),
            "userSettingPresentation",
            value.value(),
            T,
            lang,
        )?;
        upsert(
            xml_text,
            group.range(),
            T,
            "userSettingPresentation",
            &fragment,
            structure_order(group),
        )?;
    }
    Ok(())
}
fn find_link<'a, 'i>(
    root: Node<'a, 'i>,
    selector: &DataSetLinkSelector,
) -> Result<Node<'a, 'i>, String> {
    unique(
        root.children().filter(|node| {
            is(*node, S, "dataSetLink")
                && link_selector_fields(selector)
                    .iter()
                    .all(|(tag, value)| text(*node, S, tag) == *value)
        }),
        "dataset link",
    )
}
fn link_selector_fields(selector: &DataSetLinkSelector) -> [(&'static str, &str); 4] {
    [
        ("sourceDataSet", &selector.source),
        ("destinationDataSet", &selector.destination),
        ("sourceExpression", &selector.source_expression),
        ("destinationExpression", &selector.destination_expression),
    ]
}
fn link_property_fields(patch: &DataSetLinkPatch) -> [(&'static str, Option<String>); 5] {
    [
        ("parameter", patch.parameter.clone()),
        (
            "parameterListAllowed",
            patch.parameter_list_allowed.map(|flag| flag.to_string()),
        ),
        ("linkConditionExpression", patch.condition.clone()),
        ("startExpression", patch.start_expression.clone()),
        ("required", patch.required.map(|flag| flag.to_string())),
    ]
}
fn add_link(xml_text: &mut String, item: &DataSetLinkSpec) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    let root = document.root_element();
    named(root, S, "dataSet", "name", &item.selector.source)?;
    named(root, S, "dataSet", "name", &item.selector.destination)?;
    let mut body = String::new();
    for (tag, value) in link_selector_fields(&item.selector) {
        body += &simple(tag, value, S);
    }
    for (tag, value) in link_property_fields(&item.properties) {
        if let Some(value) = value {
            if (tag == "required" && item.properties.required == Some(true))
                || (tag == "parameterListAllowed"
                    && item.properties.parameter_list_allowed == Some(false))
            {
                continue;
            }
            body += &simple(tag, &value, S);
        }
    }
    Ok(insert(
        xml_text,
        root.range(),
        &xml("dataSetLink", &body, S),
        S,
        "dataSetLink",
        ROOT_ORDER,
    )?)
}
fn set_link(xml_text: &mut String, item: &DataSetLinkSpec) -> Result<(), ApplyError> {
    let document = parse_document(xml_text)?;
    find_link(document.root_element(), &item.selector)?;
    for (tag, value) in link_property_fields(&item.properties) {
        let Some(value) = value else {
            continue;
        };
        let document = parse_document(xml_text)?;
        let link = find_link(document.root_element(), &item.selector)?;
        if let Some(default) = match tag {
            "required" => Some(true),
            "parameterListAllowed" => Some(false),
            _ => None,
        } {
            let wanted = if tag == "required" {
                item.properties.required.expect("present required flag")
            } else {
                item.properties
                    .parameter_list_allowed
                    .expect("present parameter-list flag")
            };
            let existing = child(link, S, tag);
            let old = if existing.is_some() {
                match text(link, S, tag).trim() {
                    "true" | "1" => true,
                    "false" | "0" => false,
                    _ => return Err(format!("link {tag} has an invalid boolean").into()),
                }
            } else {
                default
            };
            if old == wanted {
                continue;
            }
            if wanted == default {
                if let Some(existing) = existing {
                    xml_text.replace_range(existing.range(), "");
                }
                continue;
            }
        }
        upsert(
            xml_text,
            link.range(),
            S,
            tag,
            &simple(tag, &value, S),
            LINK_ORDER,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
