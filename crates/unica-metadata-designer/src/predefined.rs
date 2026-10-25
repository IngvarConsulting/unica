use roxmltree::{Document, Node};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use unica_address::source_target::MetadataAddress;
use unica_metadata::*;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_source_designer::roots::{platform_xml_publication_policy, PlatformXmlPublicationPolicy};
use unica_xml_format::text::{
    resolve_line_ending, EolPolicy, LineEndingProfile, SourceTextSnapshot,
};
const PREDEFINED_NS: &str = "http://v8.1c.ru/8.3/xcf/predef";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const CFG_NS: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XS_NS: &str = "http://www.w3.org/2001/XMLSchema";
#[derive(Clone)]
pub struct FragmentXmlContext {
    pub wrapper_start: String,
    pub in_scope_prefixes: BTreeMap<String, String>,
    pub reserved_generated_prefixes: HashSet<String>,
}
impl FragmentXmlContext {
    #[cfg(any(test, feature = "test-support"))]
    pub fn platform_default() -> Self {
        Self::from_namespaces(std::iter::empty::<(&str, &str)>())
    }

    pub fn for_node(node: Node<'_, '_>) -> Self {
        Self::from_namespaces(
            node.namespaces()
                .filter_map(|namespace| namespace.name().map(|name| (name, namespace.uri()))),
        )
    }

    pub fn from_namespaces<'a>(namespaces: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
        let namespaces = namespaces
            .map(|(name, uri)| (name.to_string(), uri.to_string()))
            .collect::<Vec<_>>();
        let in_scope_prefixes = namespaces
            .iter()
            .filter(|(name, _)| name != "xml" && !name.is_empty())
            .cloned()
            .collect::<BTreeMap<_, _>>();
        let mut prefixes = BTreeMap::from([
            ("cfg".to_string(), CFG_NS.to_string()),
            ("v8".to_string(), V8_NS.to_string()),
            ("xr".to_string(), XR_NS.to_string()),
            ("xs".to_string(), XS_NS.to_string()),
            ("xsi".to_string(), XSI_NS.to_string()),
        ]);
        for (name, uri) in namespaces {
            if name != "xml" && !name.is_empty() {
                prefixes.insert(name, uri);
            }
        }
        let mut wrapper_start = format!("<Root xmlns=\"{PREDEFINED_NS}\"");
        for (name, uri) in prefixes {
            wrapper_start.push_str(&format!(
                " xmlns:{}=\"{}\"",
                unica_xml_format::text::escape_xml(&name),
                unica_xml_format::text::escape_xml(&uri)
            ));
        }
        wrapper_start.push('>');
        Self {
            wrapper_start,
            in_scope_prefixes,
            reserved_generated_prefixes: HashSet::new(),
        }
    }

    pub fn generated_prefix(&self, namespace: &str, canonical: &str) -> (String, bool) {
        if let Some((prefix, _)) = self.in_scope_prefixes.iter().find(|(prefix, uri)| {
            uri.as_str() == namespace && !self.reserved_generated_prefixes.contains(prefix.as_str())
        }) {
            return (prefix.clone(), false);
        }
        if !self.in_scope_prefixes.contains_key(canonical)
            && !self.reserved_generated_prefixes.contains(canonical)
        {
            return (canonical.to_string(), true);
        }
        for suffix in 1.. {
            let candidate = format!("{canonical}_{suffix}");
            if !self.in_scope_prefixes.contains_key(&candidate)
                && !self.reserved_generated_prefixes.contains(&candidate)
            {
                return (candidate, true);
            }
        }
        unreachable!("a finite namespace context cannot occupy every generated prefix")
    }

    pub fn with_local_declarations(
        &self,
        source: &str,
        node: Node<'_, '_>,
    ) -> Result<Self, String> {
        let mut prefixes = self.in_scope_prefixes.clone();
        for (name, _) in opening_tag_attributes(&source[node.range()])? {
            let Some(prefix) = name.strip_prefix("xmlns:") else {
                continue;
            };
            let namespace = node.lookup_namespace_uri(Some(prefix)).ok_or_else(|| {
                format!("predefined namespace declaration `{prefix}` cannot be resolved")
            })?;
            prefixes.insert(prefix.to_string(), namespace.to_string());
        }
        let mut context = Self::from_namespaces(
            prefixes
                .iter()
                .map(|(prefix, namespace)| (prefix.as_str(), namespace.as_str())),
        );
        context.reserved_generated_prefixes = self.reserved_generated_prefixes.clone();
        Ok(context)
    }

    pub fn reserve_descendant_prefixes(mut self, fragment: &str) -> Result<Self, String> {
        let wrapped = format!("{}{fragment}</Root>", self.wrapper_start);
        let document = Document::parse(&wrapped)
            .map_err(|error| format!("predefined XML fragment: {error}"))?;
        let root = document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
        for descendant in root.descendants().skip(1).filter(|node| node.is_element()) {
            for (name, _) in opening_tag_attributes(&wrapped[descendant.range()])? {
                if let Some(prefix) = name.strip_prefix("xmlns:") {
                    self.reserved_generated_prefixes.insert(prefix.to_string());
                }
            }
        }
        Ok(self)
    }

    pub fn decimal_type_attributes(&self) -> String {
        let (xsi_prefix, declare_xsi) = self.generated_prefix(XSI_NS, "xsi");
        let (xs_prefix, declare_xs) = self.generated_prefix(XS_NS, "xs");
        let mut attributes = String::new();
        if declare_xsi {
            attributes.push_str(&format!(" xmlns:{xsi_prefix}=\"{XSI_NS}\""));
        }
        if declare_xs {
            attributes.push_str(&format!(" xmlns:{xs_prefix}=\"{XS_NS}\""));
        }
        attributes.push_str(&format!(" {xsi_prefix}:type=\"{xs_prefix}:decimal\""));
        attributes
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredefinedCodeType {
    String,
    Number,
}
#[derive(Clone, Copy)]
pub struct PredefinedOperationContext<'a> {
    pub owner: &'a MetadataAddress,
    pub operation_index: usize,
}
/// Id of the predefined item named `name`, nested items included; `None`
/// when no item carries that name. Names are compared like metadata names.
pub fn predefined_item_id(
    bytes: &[u8],
    kind: MetadataKind,
    name: &str,
) -> Result<Option<String>, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Predefined.xml is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    validate_predefined_text_allowing_self_closing_items(kind, text)?;
    let mut found = item_identities(text, kind)?
        .into_iter()
        .filter(|(_, candidate)| metadata_name_eq(candidate, name));
    let first = found.next().map(|(id, _)| id);
    // Имя — селектор адреса: при двух одноимённых элементах адрес ничего
    // не выбирает, и первый попавшийся не подставляется.
    if found.next().is_some() {
        return Err(format!(
            "Predefined.xml has more than one predefined item named `{name}`"
        ));
    }
    Ok(first)
}
/// `(id, name)` каждого элемента документа на любой глубине.
pub fn item_identities(text: &str, kind: MetadataKind) -> Result<Vec<(String, String)>, String> {
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    let mut items = Vec::new();
    for item in document
        .root_element()
        .children()
        .filter(|node| is_predefined_element(*node, "Item"))
    {
        collect_items(text, item, kind, None, &mut items, &mut || Ok(()))?;
    }
    Ok(items.into_iter().map(|item| (item.id, item.name)).collect())
}
/// Отказ, если имя `name` уже носит другой элемент (на любой глубине):
/// имя предопределённого элемента уникально в объекте и служит адресом.
pub fn ensure_name_free(
    text: &str,
    kind: MetadataKind,
    name: &str,
    own_id: &str,
    context: PredefinedOperationContext<'_>,
    field: String,
) -> Result<(), unica_metadata::request::MetaFailure> {
    let PredefinedOperationContext {
        owner,
        operation_index,
    } = context;
    let taken = item_identities(text, kind)
        .map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                message,
                Some(operation_index),
                Some(field.clone()),
            )
        })?
        .into_iter()
        .any(|(id, candidate)| {
            metadata_name_eq(&candidate, name) && !id.eq_ignore_ascii_case(own_id)
        });
    if taken {
        return Err(failure(
            owner,
            MetaDiagnosticCode::AlreadyExists,
            format!("predefined item name `{name}` is already used"),
            Some(operation_index),
            Some(field),
        ));
    }
    Ok(())
}
/// Post-image of `Ext/Predefined.xml` after one predefined-item operation:
/// the planner of `apply` stages it itself, reading the current image from
/// its staged state. `None` means the file is absent afterwards: the last
/// item is gone, and the platform export keeps no empty document.
///
/// Diagnostic fields name the operation's own request (`elements[0].id`,
/// `ids[0]`) without the wrapper's `operations[0]` prefix.
pub fn predefined_post_image(
    kind: MetadataKind,
    owner: &MetadataAddress,
    descriptor_text: &str,
    original: Option<&[u8]>,
    operation: &MetaEditOperation,
) -> Result<Option<Vec<u8>>, unica_metadata::request::MetaFailure> {
    predefined_post_image_inner(kind, owner, descriptor_text, original, operation).map_err(
        |mut failure| {
            for diagnostic in &mut failure.diagnostics {
                if let Some(field) = diagnostic.field.as_mut() {
                    if let Some(rest) = field.strip_prefix("operations[0].") {
                        *field = rest.to_string();
                    }
                }
            }
            failure
        },
    )
}
pub fn predefined_post_image_inner(
    kind: MetadataKind,
    owner: &MetadataAddress,
    descriptor_text: &str,
    original: Option<&[u8]>,
    operation: &MetaEditOperation,
) -> Result<Option<Vec<u8>>, unica_metadata::request::MetaFailure> {
    let invalid = |message: String| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            Some(0),
            None,
        )
    };
    let code_type = predefined_code_type(kind, descriptor_text)
        .and_then(|code_type| code_type.ok_or_else(|| MISSING_PREDEFINED_CODE_TYPE.to_string()))
        .map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                message,
                None,
                Some("properties.CodeType".to_string()),
            )
        })?;
    let (mut text, has_bom) = match original {
        Some(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                failure(
                    owner,
                    MetaDiagnosticCode::ProviderUnavailable,
                    "Predefined.xml is not UTF-8",
                    None,
                    None,
                )
            })?;
            (
                text.trim_start_matches('\u{feff}').to_string(),
                bytes.starts_with(b"\xef\xbb\xbf"),
            )
        }
        None => (
            empty_document(kind, source_eol(descriptor_text).unwrap_or("\n")),
            true,
        ),
    };
    validate_predefined_text_allowing_self_closing_items(kind, &text).map_err(invalid)?;
    let observed_eol = source_eol(&text);
    let rendering_eol = observed_eol.as_deref().unwrap_or("\n");
    let preimage = text.clone();
    apply_operation(
        &mut text,
        rendering_eol,
        kind,
        code_type,
        operation,
        owner,
        0,
    )?;
    if text != preimage {
        if let Err(message) = &observed_eol {
            return Err(invalid(message.clone()));
        }
    }
    validate_predefined_text_allowing_self_closing_items(kind, &text).map_err(invalid)?;
    validate_predefined_text_for_code_type(kind, code_type, &text).map_err(invalid)?;
    if !document_has_items(&text).map_err(invalid)? {
        return Ok(None);
    }
    let mut post_image = Vec::with_capacity(text.len() + usize::from(has_bom) * 3);
    if has_bom {
        post_image.extend_from_slice(b"\xef\xbb\xbf");
    }
    post_image.extend_from_slice(text.as_bytes());
    Ok(Some(post_image))
}
pub fn document_has_items(text: &str) -> Result<bool, String> {
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    Ok(document
        .descendants()
        .any(|node| is_predefined_element(node, "Item")))
}
/// Отказ проверки предопределённых при неизвестном `CodeType` владельца —
/// один текст у чтения и записи.
pub const MISSING_PREDEFINED_CODE_TYPE: &str =
    "predefined code validation is unavailable because the owner CodeType is absent or invalid";
/// Запись предопределённых данных требует известного вида кодов владельца.
/// Описание без `CodeType` отказывает тем же текстом, что и чтение.
pub fn require_predefined_code_type(kind: MetadataKind, descriptor: &str) -> Result<(), String> {
    predefined_code_type(kind, descriptor)?
        .map(|_| ())
        .ok_or_else(|| MISSING_PREDEFINED_CODE_TYPE.to_string())
}
/// `None` — описание не несёт `CodeType`: вид кодов неизвестен.
pub fn predefined_code_type(
    kind: MetadataKind,
    descriptor_post_image: &str,
) -> Result<Option<PredefinedCodeType>, String> {
    if !matches!(
        kind,
        MetadataKind::Catalog | MetadataKind::ChartOfCalculationTypes
    ) {
        return Ok(Some(PredefinedCodeType::String));
    }
    let document = Document::parse(descriptor_post_image)
        .map_err(|error| format!("metadata descriptor: {error}"))?;
    let root = document.root_element();
    let mut owners = root.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == kind.as_str()
    });
    let object = owners
        .next()
        .ok_or_else(|| format!("{} descriptor has no direct owner", kind.as_str()))?;
    if owners.next().is_some() {
        return Err(format!(
            "{} descriptor has duplicate direct owners",
            kind.as_str()
        ));
    }
    let mut property_containers = object.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "Properties"
    });
    let properties = property_containers
        .next()
        .ok_or_else(|| format!("{} descriptor has no direct Properties", kind.as_str()))?;
    if property_containers.next().is_some() {
        return Err(format!(
            "{} descriptor has duplicate direct Properties",
            kind.as_str()
        ));
    }
    let mut code_types = properties.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(MD_CLASSES_NS)
            && node.tag_name().name() == "CodeType"
    });
    let Some(code_type_text) = code_types.next().map(direct_text_content) else {
        return Ok(None);
    };
    let code_type = code_type_text.trim();
    if code_types.next().is_some() {
        return Err(format!(
            "{} descriptor has duplicate direct CodeType",
            kind.as_str()
        ));
    }
    match code_type {
        "String" => Ok(Some(PredefinedCodeType::String)),
        "Number" => Ok(Some(PredefinedCodeType::Number)),
        other => Err(format!(
            "{} descriptor CodeType `{other}` is unsupported",
            kind.as_str()
        )),
    }
}
pub fn validate_predefined_text_for_code_type(
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    text: &str,
) -> Result<(), String> {
    validate_predefined_text_with_options(kind, Some(code_type), text, false)
}
pub fn validate_predefined_text_allowing_self_closing_items(
    kind: MetadataKind,
    text: &str,
) -> Result<(), String> {
    validate_predefined_text_with_options(kind, None, text, true)
}
pub fn validate_predefined_text_with_options(
    kind: MetadataKind,
    code_type: Option<PredefinedCodeType>,
    text: &str,
    allow_self_closing_items: bool,
) -> Result<(), String> {
    validate_predefined_text_with_checkpoint(
        kind,
        code_type,
        text,
        allow_self_closing_items,
        &mut || Ok(()),
    )
}
pub fn validate_predefined_text_with_checkpoint(
    kind: MetadataKind,
    code_type: Option<PredefinedCodeType>,
    text: &str,
    allow_self_closing_items: bool,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    checkpoint()?;
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    checkpoint()?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or_default();
    let local_name = root.tag_name().name();
    if namespace != PREDEFINED_NS
        || local_name != "PredefinedData"
        || platform_xml_publication_policy(namespace, local_name)
            != Some(PlatformXmlPublicationPolicy::ExactRootVersion)
        || root.attribute((XSI_NS, "type")) != Some(root_type(kind)?)
        || unica_source_designer::roots::root_version_literal(text, root).as_deref()
            != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(format!(
            "Predefined.xml does not match the exact {}/{} profile for {}",
            ACTIVE_FORMAT_PROFILE.platform_line,
            ACTIVE_FORMAT_PROFILE.export_format,
            kind.as_str()
        ));
    }
    for node in root.descendants().filter(|node| node.is_element()) {
        checkpoint()?;
        let Some(parent) = node
            .parent_element()
            .filter(|parent| parent.tag_name().namespace() == Some(PREDEFINED_NS))
        else {
            continue;
        };
        if node.tag_name().namespace() != Some(PREDEFINED_NS)
            && expected_predefined_child_name(
                kind,
                parent.tag_name().name(),
                node.tag_name().name(),
            )
        {
            return Err(format!(
                "predefined {} must use the predefined namespace under {}",
                node.tag_name().name(),
                parent.tag_name().name()
            ));
        }
    }
    let mut ids = HashSet::new();
    for child_items in root
        .descendants()
        .filter(|node| is_predefined_element(*node, "ChildItems"))
    {
        checkpoint()?;
        if child_items
            .parent_element()
            .is_none_or(|parent| !is_predefined_element(parent, "Item"))
        {
            return Err("ChildItems is not a direct child of Item".to_string());
        }
    }
    for item in root
        .descendants()
        .filter(|node| is_predefined_element(*node, "Item"))
    {
        checkpoint()?;
        let Some(raw_id) = item.attribute("id") else {
            return Err("predefined Item requires id".to_string());
        };
        let id = canonical_predefined_uuid(raw_id)
            .ok_or_else(|| "predefined Item id is not a canonical UUID".to_string())?;
        if !ids.insert(id) {
            return Err("predefined Item UUID is duplicated".to_string());
        }
        let parent = item.parent_element().ok_or_else(|| {
            "predefined Item has no direct PredefinedData/ChildItems owner".to_string()
        })?;
        if !is_predefined_element(parent, "PredefinedData")
            && !is_predefined_element(parent, "ChildItems")
        {
            return Err(
                "predefined Item is not a direct child of PredefinedData/ChildItems".into(),
            );
        }
        let mut supported = vec!["Name", "Code", "Description", "ChildItems"];
        supported.extend(match kind {
            MetadataKind::Catalog => &["IsFolder"][..],
            MetadataKind::ChartOfCharacteristicTypes => &["Type", "IsFolder"][..],
            MetadataKind::ChartOfAccounts => &[
                "AccountType",
                "OffBalance",
                "Order",
                "AccountingFlags",
                "ExtDimensionTypes",
            ][..],
            MetadataKind::ChartOfCalculationTypes => &["ActionPeriodIsBase"][..],
            _ => return Err(format!("{} does not own predefined data", kind.as_str())),
        });
        for tag in supported {
            let count = item
                .children()
                .filter(|node| is_predefined_element(*node, tag))
                .count();
            let missing_name_is_repairable = tag == "Name"
                && count == 0
                && allow_self_closing_items
                && text[item.range()].trim_end().ends_with("/>");
            if count > 1 || (tag == "Name" && count != 1 && !missing_name_is_repairable) {
                return Err(format!(
                    "predefined Item must contain {} direct {tag} element",
                    if tag == "Name" {
                        "exactly one"
                    } else {
                        "at most one"
                    }
                ));
            }
        }
        if let (Some(code_type), Some(code)) = (code_type, child(item, "Code")) {
            validate_code_value(code, code_type)?;
        }
        parse_item(text, item, kind, None)?;
    }
    Ok(())
}
pub fn validate_code_value(
    code: Node<'_, '_>,
    code_type: PredefinedCodeType,
) -> Result<(), String> {
    let value = direct_text_content(code);
    if code_type == PredefinedCodeType::String {
        return if code.attribute((XSI_NS, "type")).is_none() {
            Ok(())
        } else {
            Err("predefined string Code must not declare xsi:type".to_string())
        };
    }
    if value.is_empty() {
        return if code.attribute((XSI_NS, "type")).is_none() {
            Ok(())
        } else {
            Err("empty predefined numeric Code must not declare xsi:type".to_string())
        };
    }
    if metadata_decimal_shape(&value).is_none() {
        return Err("predefined numeric Code is not an XML Schema decimal".to_string());
    }
    let raw_type = code
        .attribute((XSI_NS, "type"))
        .ok_or_else(|| "predefined numeric Code requires xsi:type".to_string())?;
    let (prefix, local) = raw_type
        .split_once(':')
        .filter(|(prefix, local)| !prefix.is_empty() && !local.is_empty() && !local.contains(':'))
        .ok_or_else(|| "predefined numeric Code xsi:type is not a QName".to_string())?;
    if local != "decimal" || code.lookup_namespace_uri(Some(prefix)) != Some(XS_NS) {
        return Err("predefined numeric Code xsi:type must resolve to xs:decimal".to_string());
    }
    Ok(())
}
pub fn expected_predefined_child_name(kind: MetadataKind, parent: &str, child: &str) -> bool {
    match parent {
        "PredefinedData" | "ChildItems" => child == "Item",
        "Item" => {
            matches!(child, "Name" | "Code" | "Description" | "ChildItems")
                || match kind {
                    MetadataKind::Catalog => child == "IsFolder",
                    MetadataKind::ChartOfCharacteristicTypes => {
                        matches!(child, "Type" | "IsFolder")
                    }
                    MetadataKind::ChartOfAccounts => matches!(
                        child,
                        "AccountType"
                            | "OffBalance"
                            | "Order"
                            | "AccountingFlags"
                            | "ExtDimensionTypes"
                    ),
                    MetadataKind::ChartOfCalculationTypes => child == "ActionPeriodIsBase",
                    _ => false,
                }
        }
        "AccountingFlags" => child == "Flag",
        "ExtDimensionTypes" => child == "ExtDimensionType",
        "ExtDimensionType" => matches!(child, "Turnover" | "AccountingFlags"),
        _ => false,
    }
}
pub fn collect_items(
    source: &str,
    item: Node<'_, '_>,
    kind: MetadataKind,
    parent_id: Option<String>,
    output: &mut Vec<MetaPredefinedItemData>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    checkpoint()?;
    let data = parse_item(source, item, kind, parent_id)?;
    let id = data.id.clone();
    output.push(data);
    if let Some(children) = child(item, "ChildItems") {
        for nested in children
            .children()
            .filter(|node| is_predefined_element(*node, "Item"))
        {
            collect_items(source, nested, kind, Some(id.clone()), output, checkpoint)?;
        }
    }
    Ok(())
}
pub fn parse_item(
    source: &str,
    item: Node<'_, '_>,
    kind: MetadataKind,
    parent_id: Option<String>,
) -> Result<MetaPredefinedItemData, String> {
    let id = canonical_predefined_uuid(
        item.attribute("id")
            .ok_or_else(|| "predefined Item requires id".to_string())?,
    )
    .ok_or_else(|| "predefined Item id is not a canonical UUID".to_string())?;
    let text = |tag: &str| child_text(item, tag).unwrap_or_default();
    let bool_value = |tag: &str| -> Result<bool, String> {
        match child_text(item, tag).as_deref().unwrap_or("false").trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("predefined {tag} is not boolean")),
        }
    };
    let common = MetaPredefinedItemData {
        id,
        parent_id,
        name: text("Name"),
        code: text("Code"),
        description: text("Description"),
        is_folder: None,
        r#type: None,
        account_type: None,
        off_balance: None,
        order: None,
        accounting_flags: None,
        ext_dimension_types: None,
        action_period_is_base: None,
    };
    match kind {
        MetadataKind::Catalog => Ok(MetaPredefinedItemData {
            is_folder: Some(bool_value("IsFolder")?),
            ..common
        }),
        MetadataKind::ChartOfCharacteristicTypes => {
            let r#type = child(item, "Type")
                .map(|node| parse_item_type(source, node))
                .transpose()?;
            Ok(MetaPredefinedItemData {
                is_folder: Some(bool_value("IsFolder")?),
                r#type,
                ..common
            })
        }
        MetadataKind::ChartOfAccounts => Ok(MetaPredefinedItemData {
            account_type: Some(
                MetaPredefinedAccountType::parse(
                    child_text(item, "AccountType")
                        .as_deref()
                        .unwrap_or("ActivePassive")
                        .trim(),
                    "accountType",
                )
                .map_err(|diagnostic| diagnostic.message)?,
            ),
            off_balance: Some(bool_value("OffBalance")?),
            order: Some(text("Order")),
            accounting_flags: Some(parse_flags(child(item, "AccountingFlags"))?),
            ext_dimension_types: Some(parse_ext_dimensions(child(item, "ExtDimensionTypes"))?),
            ..common
        }),
        MetadataKind::ChartOfCalculationTypes => Ok(MetaPredefinedItemData {
            action_period_is_base: Some(bool_value("ActionPeriodIsBase")?),
            ..common
        }),
        _ => Err(format!("{} does not own predefined data", kind.as_str())),
    }
}
pub fn parse_flags(container: Option<Node<'_, '_>>) -> Result<BTreeMap<String, bool>, String> {
    let mut flags = BTreeMap::new();
    for flag in container
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| is_predefined_element(*node, "Flag"))
    {
        let name = flag
            .attribute("ref")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "predefined Flag requires ref".to_string())?;
        let flag_text = direct_text_content(flag);
        let value = match flag_text.trim() {
            "true" => true,
            "false" => false,
            _ => return Err("predefined Flag value is not boolean".to_string()),
        };
        if flags
            .keys()
            .any(|existing: &String| metadata_name_eq(existing, name))
        {
            return Err("predefined Flag ref is duplicated".to_string());
        }
        flags.insert(name.to_string(), value);
    }
    Ok(flags)
}
pub fn parse_ext_dimensions(
    container: Option<Node<'_, '_>>,
) -> Result<Vec<MetaPredefinedExtDimensionTypeData>, String> {
    let mut values = Vec::new();
    let mut names = HashSet::new();
    for item in container
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| is_predefined_element(*node, "ExtDimensionType"))
    {
        let name = item
            .attribute("name")
            .filter(|name| !name.is_empty())
            .ok_or_else(|| "ExtDimensionType requires name".to_string())?;
        if !names.insert(name.to_lowercase()) {
            return Err("ExtDimensionType name is duplicated".to_string());
        }
        for tag in ["Turnover", "AccountingFlags"] {
            if item
                .children()
                .filter(|node| is_predefined_element(*node, tag))
                .count()
                > 1
            {
                return Err(format!(
                    "ExtDimensionType contains duplicate direct {tag} elements"
                ));
            }
        }
        let turnover = match child_text(item, "Turnover")
            .as_deref()
            .unwrap_or("false")
            .trim()
        {
            "true" => true,
            "false" => false,
            _ => return Err("ExtDimensionType Turnover is not boolean".to_string()),
        };
        values.push(MetaPredefinedExtDimensionTypeData {
            name: name.to_string(),
            turnover,
            accounting_flags: parse_flags(child(item, "AccountingFlags"))?,
        });
    }
    Ok(values)
}
pub fn apply_operation(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    operation: &MetaEditOperation,
    owner: &MetadataAddress,
    operation_index: usize,
) -> Result<(), unica_metadata::request::MetaFailure> {
    let operation_context = PredefinedOperationContext {
        owner,
        operation_index,
    };
    match operation {
        MetaEditOperation::AddPredefinedItems { elements } => {
            for (index, element) in elements.iter().enumerate() {
                validate_requested_code(
                    element.fields.code.as_deref(),
                    code_type,
                    owner,
                    operation_index,
                    index,
                )?;
                add_item(
                    text,
                    eol,
                    kind,
                    code_type,
                    element,
                    operation_context,
                    index,
                )?;
            }
        }
        MetaEditOperation::UpdatePredefinedItems { elements } => {
            for (index, element) in elements.iter().enumerate() {
                validate_requested_code(
                    element.fields.code.as_deref(),
                    code_type,
                    owner,
                    operation_index,
                    index,
                )?;
                update_item(
                    text,
                    eol,
                    kind,
                    code_type,
                    element,
                    operation_context,
                    index,
                )?;
            }
        }
        MetaEditOperation::RemovePredefinedItems { ids } => {
            // Все цели разрешаются на одном прообразе до первой мутации:
            // `[родитель, потомок]` и `[потомок, родитель]` равнозначны, а id
            // внутри уже запрошенного поддерева покрывается его удалением.
            let mut locations = Vec::with_capacity(ids.len());
            for (index, id) in ids.iter().enumerate() {
                let Some(location) = find_item(text, id).map_err(|message| {
                    failure(
                        owner,
                        MetaDiagnosticCode::ValidationFailed,
                        message,
                        Some(operation_index),
                        Some(format!("ids[{index}]")),
                    )
                })?
                else {
                    return Err(failure(
                        owner,
                        MetaDiagnosticCode::TargetNotFound,
                        format!("predefined item `{id}` was not found"),
                        Some(operation_index),
                        Some(format!("ids[{index}]")),
                    ));
                };
                locations.push(location.range);
            }
            let requested = locations.clone();
            locations.retain(|range| {
                !requested.iter().any(|other| {
                    *other != *range && other.start <= range.start && range.end <= other.end
                })
            });
            locations.sort_by_key(|range| std::cmp::Reverse(range.start));
            for location in locations {
                let range = removal_range(text, location);
                text.replace_range(range, "");
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn validate_requested_code(
    code: Option<&str>,
    code_type: PredefinedCodeType,
    owner: &MetadataAddress,
    operation_index: usize,
    element_index: usize,
) -> Result<(), unica_metadata::request::MetaFailure> {
    if code_type == PredefinedCodeType::Number
        && code.is_some_and(|value| !value.is_empty() && metadata_decimal_shape(value).is_none())
    {
        return Err(failure(
            owner,
            MetaDiagnosticCode::InvalidArguments,
            "predefined numeric Code is not an XML Schema decimal",
            Some(operation_index),
            Some(format!("elements[{element_index}].code")),
        ));
    }
    Ok(())
}
pub fn add_item(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    element: &MetaPredefinedItemAdd,
    context: PredefinedOperationContext<'_>,
    element_index: usize,
) -> Result<(), unica_metadata::request::MetaFailure> {
    let PredefinedOperationContext {
        owner,
        operation_index,
    } = context;
    if let Some(location) = find_item(text, &element.id).map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            Some(operation_index),
            Some(format!("elements[{element_index}].id")),
        )
    })? {
        if location.parent_id.is_none() {
            let document = Document::parse(text).expect("validated predefined image");
            let item = document
                .descendants()
                .find(|node| node.range() == location.range)
                .expect("located item remains present");
            let existing = parse_item(text, item, kind, None).map_err(|message| {
                failure(
                    owner,
                    MetaDiagnosticCode::ValidationFailed,
                    message,
                    Some(operation_index),
                    Some(format!("elements[{element_index}]")),
                )
            })?;
            let has_nested_items = child(item, "ChildItems").is_some_and(|children| {
                children
                    .children()
                    .any(|node| is_predefined_element(node, "Item"))
            });
            if !has_nested_items && existing == add_projection(element, kind) {
                return Ok(());
            }
        }
        return Err(failure(
            owner,
            MetaDiagnosticCode::AlreadyExists,
            format!("predefined item `{}` already exists", element.id),
            Some(operation_index),
            Some(format!("elements[{element_index}].id")),
        ));
    }
    ensure_name_free(
        text,
        kind,
        &element.name,
        &element.id,
        context,
        format!("elements[{element_index}].name"),
    )?;
    expand_self_closing_root(text, eol);
    let (root_qname, root_context, root_range) = {
        let document = Document::parse(text).map_err(|error| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                format!("Predefined.xml: {error}"),
                Some(operation_index),
                None,
            )
        })?;
        let root = document.root_element();
        lexical_element_name(&text[root.range()])
            .map(str::to_string)
            .map_err(|message| {
                failure(
                    owner,
                    MetaDiagnosticCode::ValidationFailed,
                    message,
                    Some(operation_index),
                    None,
                )
            })
            .map(|qname| (qname, FragmentXmlContext::for_node(root), root.range()))?
    };
    let closing_tag = format!("</{root_qname}>");
    // Якорь — распарсенный диапазон корня: текст после него (например,
    // хвостовой комментарий с литералом закрывающего тега) не может
    // перехватить точку вставки.
    let close = text[root_range.clone()]
        .rfind(&closing_tag)
        .map(|offset| root_range.start + offset)
        .ok_or_else(|| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                "PredefinedData closing tag is unavailable",
                Some(operation_index),
                None,
            )
        })?;
    let rendered_item =
        render_item(element, kind, code_type, eol, &root_context).map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::InvalidArguments,
                message,
                Some(operation_index),
                Some(format!("elements[{element_index}].code")),
            )
        })?;
    let fragment =
        qualify_generated_predefined_elements(&rendered_item, lexical_element_prefix(&root_qname));
    let fragment = format!("\t{}", fragment.replace(eol, &format!("{eol}\t")));
    text.insert_str(close, &format!("{fragment}{eol}"));
    Ok(())
}
pub fn update_item(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    element: &MetaPredefinedItemUpdate,
    context: PredefinedOperationContext<'_>,
    element_index: usize,
) -> Result<(), unica_metadata::request::MetaFailure> {
    let PredefinedOperationContext {
        owner,
        operation_index,
    } = context;
    let Some(location) = find_item(text, &element.id).map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            Some(operation_index),
            Some(format!("elements[{element_index}].id")),
        )
    })?
    else {
        return Err(failure(
            owner,
            MetaDiagnosticCode::TargetNotFound,
            format!("predefined item `{}` was not found", element.id),
            Some(operation_index),
            Some(format!("elements[{element_index}].id")),
        ));
    };
    if let Some(name) = &element.name {
        ensure_name_free(
            text,
            kind,
            name,
            &element.id,
            context,
            format!("elements[{element_index}].name"),
        )?;
    }
    let mut fragment = text[location.range.clone()].to_string();
    let fragment_context = &location.fragment_context;
    if let Some(name) = &element.name {
        set_simple_child(&mut fragment, "Name", name, eol, fragment_context).map_err(
            |message| {
                failure(
                    owner,
                    MetaDiagnosticCode::ValidationFailed,
                    message,
                    Some(operation_index),
                    Some(format!("elements[{element_index}].name")),
                )
            },
        )?;
    }
    patch_fields(
        &mut fragment,
        kind,
        code_type,
        &element.fields,
        eol,
        fragment_context,
    )
    .map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            Some(operation_index),
            Some(format!("elements[{element_index}]")),
        )
    })?;
    text.replace_range(location.range, &fragment);
    Ok(())
}
pub fn patch_fields(
    item: &mut String,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    fields: &MetaPredefinedFields,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    if let Some(value) = &fields.code {
        set_code_child(item, value, code_type, eol, fragment_context)?;
    }
    if let Some(value) = &fields.description {
        set_simple_child(item, "Description", value, eol, fragment_context)?;
    }
    if let Some(value) = fields.is_folder {
        set_simple_child(item, "IsFolder", &value.to_string(), eol, fragment_context)?;
    }
    if let Some(value) = fields.account_type {
        set_simple_child(item, "AccountType", value.as_str(), eol, fragment_context)?;
    }
    if let Some(value) = fields.off_balance {
        set_simple_child(
            item,
            "OffBalance",
            &value.to_string(),
            eol,
            fragment_context,
        )?;
    }
    if let Some(value) = &fields.order {
        set_simple_child(item, "Order", value, eol, fragment_context)?;
    }
    if let Some(value) = fields.action_period_is_base {
        set_simple_child(
            item,
            "ActionPeriodIsBase",
            &value.to_string(),
            eol,
            fragment_context,
        )?;
    }
    if let Some(value) = &fields.r#type {
        let render_context = if let Some(location) =
            fragment_direct_child_location(item, "Type", None, fragment_context)?
        {
            let FragmentChildLocation { range, context, .. } = location;
            context.reserve_descendant_prefixes(&item[range])?
        } else {
            fragment_context.clone()
        };
        let indent = child_indent(item);
        let rendered = render_type(value, &indent, eol, &render_context);
        let rendered = rendered.strip_prefix(&indent).unwrap_or(&rendered);
        set_complex_child(item, "Type", rendered, eol, fragment_context)?;
    }
    if let Some(flags) = &fields.accounting_flags {
        merge_flags_container(item, "AccountingFlags", flags, eol, fragment_context)?;
    }
    if let Some(types) = &fields.ext_dimension_types {
        merge_ext_dimensions(item, types, eol, fragment_context)?;
    }
    debug_assert!(matches!(
        kind,
        MetadataKind::Catalog
            | MetadataKind::ChartOfAccounts
            | MetadataKind::ChartOfCharacteristicTypes
            | MetadataKind::ChartOfCalculationTypes
    ));
    Ok(())
}
pub fn render_item(
    element: &MetaPredefinedItemAdd,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<String, String> {
    let code = render_code(
        element.fields.code.as_deref().unwrap_or(""),
        code_type,
        fragment_context,
    )?;
    let mut lines = vec![
        format!(
            "<Item id=\"{}\">",
            unica_xml_format::text::escape_xml(&element.id)
        ),
        format!(
            "\t<Name>{}</Name>",
            unica_xml_format::text::escape_xml(&element.name)
        ),
        format!("\t{code}"),
        simple_line(
            "Description",
            element.fields.description.as_deref().unwrap_or(""),
            "\t",
        ),
    ];
    match kind {
        MetadataKind::Catalog => lines.push(format!(
            "\t<IsFolder>{}</IsFolder>",
            element.fields.is_folder.unwrap_or(false)
        )),
        MetadataKind::ChartOfCharacteristicTypes => {
            if let Some(metadata_type) = &element.fields.r#type {
                let rendered = render_type(metadata_type, "\t", eol, fragment_context);
                lines.extend(rendered.split(eol).map(str::to_string));
            }
            lines.push(format!(
                "\t<IsFolder>{}</IsFolder>",
                element.fields.is_folder.unwrap_or(false)
            ));
        }
        MetadataKind::ChartOfAccounts => {
            lines.push(format!(
                "\t<AccountType>{}</AccountType>",
                element
                    .fields
                    .account_type
                    .unwrap_or(MetaPredefinedAccountType::ActivePassive)
                    .as_str()
            ));
            lines.push(format!(
                "\t<OffBalance>{}</OffBalance>",
                element.fields.off_balance.unwrap_or(false)
            ));
            lines.push(simple_line(
                "Order",
                element.fields.order.as_deref().unwrap_or(""),
                "\t",
            ));
            lines.extend(render_flags(
                "AccountingFlags",
                element.fields.accounting_flags.as_ref(),
                "\t",
            ));
            lines.extend(render_ext_dimensions(
                element.fields.ext_dimension_types.as_deref(),
                "\t",
            ));
        }
        MetadataKind::ChartOfCalculationTypes => lines.push(format!(
            "\t<ActionPeriodIsBase>{}</ActionPeriodIsBase>",
            element.fields.action_period_is_base.unwrap_or(false)
        )),
        _ => unreachable!("capability validation rejects this predefined owner"),
    }
    lines.push("</Item>".to_string());
    Ok(lines.join(eol))
}
pub fn add_projection(
    element: &MetaPredefinedItemAdd,
    kind: MetadataKind,
) -> MetaPredefinedItemData {
    let common = MetaPredefinedItemData {
        id: element.id.clone(),
        parent_id: None,
        name: element.name.clone(),
        code: element.fields.code.clone().unwrap_or_default(),
        description: element.fields.description.clone().unwrap_or_default(),
        is_folder: None,
        r#type: None,
        account_type: None,
        off_balance: None,
        order: None,
        accounting_flags: None,
        ext_dimension_types: None,
        action_period_is_base: None,
    };
    match kind {
        MetadataKind::Catalog => MetaPredefinedItemData {
            is_folder: Some(element.fields.is_folder.unwrap_or(false)),
            ..common
        },
        MetadataKind::ChartOfCharacteristicTypes => MetaPredefinedItemData {
            is_folder: Some(element.fields.is_folder.unwrap_or(false)),
            r#type: element.fields.r#type.clone(),
            ..common
        },
        MetadataKind::ChartOfAccounts => MetaPredefinedItemData {
            account_type: Some(
                element
                    .fields
                    .account_type
                    .unwrap_or(MetaPredefinedAccountType::ActivePassive),
            ),
            off_balance: Some(element.fields.off_balance.unwrap_or(false)),
            order: Some(element.fields.order.clone().unwrap_or_default()),
            accounting_flags: Some(element.fields.accounting_flags.clone().unwrap_or_default()),
            ext_dimension_types: Some(
                element
                    .fields
                    .ext_dimension_types
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|item| MetaPredefinedExtDimensionTypeData {
                        name: item.name.clone(),
                        turnover: item.turnover.unwrap_or(false),
                        accounting_flags: item.accounting_flags.clone().unwrap_or_default(),
                    })
                    .collect(),
            ),
            ..common
        },
        MetadataKind::ChartOfCalculationTypes => MetaPredefinedItemData {
            action_period_is_base: Some(element.fields.action_period_is_base.unwrap_or(false)),
            ..common
        },
        _ => common,
    }
}
#[derive(Clone)]
pub struct ItemLocation {
    pub range: Range<usize>,
    pub parent_id: Option<String>,
    pub fragment_context: FragmentXmlContext,
}
pub fn find_item(text: &str, requested_id: &str) -> Result<Option<ItemLocation>, String> {
    let requested = uuid::Uuid::parse_str(requested_id)
        .map_err(|_| "predefined item id is not a UUID".to_string())?;
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    for node in document
        .descendants()
        .filter(|node| is_predefined_element(*node, "Item"))
    {
        let Some(id) = node.attribute("id") else {
            continue;
        };
        if uuid::Uuid::parse_str(id).ok() != Some(requested) {
            continue;
        }
        let parent_id = node
            .parent_element()
            .filter(|parent| is_predefined_element(*parent, "ChildItems"))
            .and_then(|parent| parent.parent_element())
            .and_then(|parent| parent.attribute("id"))
            .map(str::to_string);
        return Ok(Some(ItemLocation {
            range: node.range(),
            parent_id,
            fragment_context: FragmentXmlContext::for_node(node),
        }));
    }
    Ok(None)
}
pub fn set_simple_child(
    item: &mut String,
    tag: &str,
    value: &str,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    if direct_child_typed_text_matches(item, tag, value, fragment_context)? {
        return Ok(());
    }
    let rendered = if value.is_empty() {
        format!("<{tag}/>")
    } else {
        format!(
            "<{tag}>{}</{tag}>",
            unica_xml_format::text::escape_xml(value)
        )
    };
    set_complex_child(item, tag, &rendered, eol, fragment_context)
}
pub fn set_code_child(
    item: &mut String,
    value: &str,
    code_type: PredefinedCodeType,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    let existing = fragment_direct_child_location(item, "Code", None, fragment_context)?;
    if let Some(location) = &existing {
        let wrapped = format!(
            "{}{}</Root>",
            location.context.wrapper_start,
            &item[location.range.clone()]
        );
        let document = Document::parse(&wrapped)
            .map_err(|error| format!("predefined XML fragment: {error}"))?;
        let code = document
            .root_element()
            .children()
            .find(|node| node.is_element())
            .ok_or_else(|| "predefined Code fragment has no element".to_string())?;
        if direct_text_content(code) == value && validate_code_value(code, code_type).is_ok() {
            return Ok(());
        }
    }
    let render_context =
        existing.map_or_else(|| fragment_context.clone(), |location| location.context);
    let rendered = render_code(value, code_type, &render_context)?;
    set_complex_child(item, "Code", &rendered, eol, fragment_context)
}
pub fn direct_child_typed_text_matches(
    parent: &str,
    tag: &str,
    value: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<bool, String> {
    let Some(location) = fragment_direct_child_location(parent, tag, None, fragment_context)?
    else {
        return Ok(false);
    };
    let wrapped = format!(
        "{}{}</Root>",
        location.context.wrapper_start, &parent[location.range]
    );
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    Ok(document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .is_some_and(|node| direct_text_content(node) == value))
}
pub fn render_code(
    value: &str,
    code_type: PredefinedCodeType,
    fragment_context: &FragmentXmlContext,
) -> Result<String, String> {
    if value.is_empty() {
        return Ok("<Code/>".to_string());
    }
    if code_type == PredefinedCodeType::Number {
        if metadata_decimal_shape(value).is_none() {
            return Err("predefined numeric Code is not an XML Schema decimal".to_string());
        }
        return Ok(format!(
            "<Code{}>{}</Code>",
            fragment_context.decimal_type_attributes(),
            unica_xml_format::text::escape_xml(value)
        ));
    }
    Ok(format!(
        "<Code>{}</Code>",
        unica_xml_format::text::escape_xml(value)
    ))
}
pub fn opening_tag_end(fragment: &str) -> Result<usize, String> {
    let mut quote = None;
    for (index, character) in fragment.char_indices() {
        match (quote, character) {
            (Some(expected), current) if current == expected => quote = None,
            (None, '\'' | '"') => quote = Some(character),
            (None, '>') => return Ok(index),
            _ => {}
        }
    }
    Err("predefined element opening tag is unavailable".to_string())
}
pub fn opening_tag_attributes(fragment: &str) -> Result<Vec<(String, String)>, String> {
    let end = opening_tag_end(fragment)?;
    let opening = &fragment[..end];
    let bytes = opening.as_bytes();
    let mut index = 1;
    while bytes
        .get(index)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
    {
        index += 1;
    }
    let mut attributes = Vec::new();
    while index < bytes.len() {
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] == b'/' {
            break;
        }
        let name_start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'=')
        {
            index += 1;
        }
        let name_end = index;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) != Some(&b'=') {
            return Err("predefined element attribute is malformed".to_string());
        }
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        let quote = *bytes
            .get(index)
            .filter(|quote| matches!(quote, b'\'' | b'"'))
            .ok_or_else(|| "predefined element attribute value is unquoted".to_string())?;
        index += 1;
        while bytes.get(index).is_some_and(|byte| *byte != quote) {
            index += 1;
        }
        if bytes.get(index) != Some(&quote) {
            return Err("predefined element attribute value is unterminated".to_string());
        }
        index += 1;
        attributes.push((
            opening[name_start..name_end].to_string(),
            opening[name_start..index].to_string(),
        ));
    }
    Ok(attributes)
}
pub fn insert_opening_attributes(
    fragment: &mut String,
    attributes: &[String],
) -> Result<(), String> {
    if attributes.is_empty() {
        return Ok(());
    }
    let end = opening_tag_end(fragment)?;
    let insert_at = fragment[..end]
        .rfind('/')
        .filter(|slash| fragment[*slash + 1..end].trim().is_empty())
        .unwrap_or(end);
    fragment.insert_str(insert_at, &format!(" {}", attributes.join(" ")));
    Ok(())
}
pub fn preserve_element_attributes(
    existing: &str,
    rendered: &str,
    existing_context: &FragmentXmlContext,
    rendered_context: &FragmentXmlContext,
) -> Result<String, String> {
    let existing_opening = opening_tag_attributes(existing)?;
    let rendered_opening = opening_tag_attributes(rendered)?;
    let rendered_names = rendered_opening
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    let wrapped_existing = format!("{}{existing}</Root>", existing_context.wrapper_start);
    let existing_document = Document::parse(&wrapped_existing)
        .map_err(|error| format!("predefined XML fragment: {error}"))?;
    let existing_node = existing_document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let wrapped_rendered = format!("{}{rendered}</Root>", rendered_context.wrapper_start);
    let rendered_document = Document::parse(&wrapped_rendered)
        .map_err(|error| format!("predefined XML fragment: {error}"))?;
    let rendered_node = rendered_document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let preserved_namespaces = existing_opening
        .iter()
        .filter(|(name, _)| name == "xmlns" || name.starts_with("xmlns:"))
        .filter(|(name, _)| !rendered_names.contains(name.as_str()))
        .filter(|(name, _)| {
            !writer_controls_namespace_declaration(existing_node, rendered_node, name)
        })
        .map(|(_, lexeme)| lexeme.clone())
        .collect::<Vec<_>>();

    let mut merged = rendered.to_string();
    insert_opening_attributes(&mut merged, &preserved_namespaces)?;

    let wrapped_merged = format!("{}{merged}</Root>", rendered_context.wrapper_start);
    let merged_document = Document::parse(&wrapped_merged)
        .map_err(|error| format!("predefined XML fragment: {error}"))?;
    let merged_node = merged_document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let rendered_keys = merged_node
        .attributes()
        .map(|attribute| {
            (
                attribute.namespace().map(str::to_string),
                attribute.name().to_string(),
            )
        })
        .collect::<HashSet<_>>();
    let preserved_attributes = existing_node
        .attributes()
        .filter(|attribute| {
            !writer_controls_attribute(existing_node, *attribute)
                && !rendered_keys.contains(&(
                    attribute.namespace().map(str::to_string),
                    attribute.name().to_string(),
                ))
        })
        .map(|attribute| wrapped_existing[attribute.range()].to_string())
        .collect::<Vec<_>>();
    insert_opening_attributes(&mut merged, &preserved_attributes)?;
    Ok(merged)
}
pub fn writer_controls_attribute(
    element: Node<'_, '_>,
    attribute: roxmltree::Attribute<'_, '_>,
) -> bool {
    element.tag_name().namespace() == Some(PREDEFINED_NS)
        && element.tag_name().name() == "Code"
        && attribute.namespace() == Some(XSI_NS)
        && attribute.name() == "type"
}
pub fn writer_controls_namespace_declaration(
    existing: Node<'_, '_>,
    rendered: Node<'_, '_>,
    declaration_name: &str,
) -> bool {
    if existing.tag_name().namespace() != Some(PREDEFINED_NS)
        || existing.tag_name().name() != "Code"
        || rendered.attribute((XSI_NS, "type")).is_some()
    {
        return false;
    }
    let Some(prefix) = declaration_name.strip_prefix("xmlns:") else {
        return false;
    };
    let Some(namespace) = existing.lookup_namespace_uri(Some(prefix)) else {
        return false;
    };
    if !matches!(namespace, XSI_NS | XS_NS) {
        return false;
    }
    let qname_prefix = format!("{prefix}:");
    !existing.descendants().any(|node| {
        (node.is_element()
            && ((node != existing && node.tag_name().namespace() == Some(namespace))
                || node.attributes().any(|attribute| {
                    !writer_controls_attribute(node, attribute)
                        && (attribute.namespace() == Some(namespace)
                            || attribute.value().contains(&qname_prefix))
                })))
            || node
                .children()
                .filter_map(|child| child.text())
                .any(|text| text.contains(&qname_prefix))
    })
}
type ExpandedElementName = (Option<String>, String);
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChildAnchor {
    Element(ExpandedElementName, usize),
    Text(usize),
}
#[derive(Clone)]
pub struct ControlledChildFragment {
    pub key: ExpandedElementName,
    pub occurrence: usize,
    pub range: Range<usize>,
    pub context: FragmentXmlContext,
    pub has_preservation_obligation: bool,
}
pub struct PreservedChildFragment {
    pub source: String,
    pub following_anchors: Vec<ChildAnchor>,
}
pub struct ElementChildren {
    pub controlled: Vec<ControlledChildFragment>,
    pub preserved: Vec<PreservedChildFragment>,
    pub anchors: Vec<(ChildAnchor, usize)>,
}
pub fn expanded_element_name(node: Node<'_, '_>) -> ExpandedElementName {
    (
        node.tag_name().namespace().map(str::to_string),
        node.tag_name().name().to_string(),
    )
}
pub fn writer_controls_child(parent: Node<'_, '_>, child: Node<'_, '_>) -> bool {
    if !child.is_element() || child.tag_name().namespace() != Some(V8_NS) {
        return false;
    }
    match (parent.tag_name().namespace(), parent.tag_name().name()) {
        (Some(PREDEFINED_NS), "Type") => matches!(
            child.tag_name().name(),
            "Type"
                | "TypeSet"
                | "NumberQualifiers"
                | "StringQualifiers"
                | "DateQualifiers"
                | "BinaryDataQualifiers"
        ),
        (Some(V8_NS), "NumberQualifiers") => matches!(
            child.tag_name().name(),
            "Digits" | "FractionDigits" | "AllowedSign"
        ),
        (Some(V8_NS), "StringQualifiers" | "BinaryDataQualifiers") => {
            matches!(child.tag_name().name(), "Length" | "AllowedLength")
        }
        (Some(V8_NS), "DateQualifiers") => child.tag_name().name() == "DateFractions",
        _ => false,
    }
}
pub fn controlled_subtree_has_preservation_obligation(node: Node<'_, '_>) -> bool {
    node.attributes().next().is_some()
        || node.children().any(|child| {
            if child.is_text() {
                false
            } else if writer_controls_child(node, child) {
                controlled_subtree_has_preservation_obligation(child)
            } else {
                true
            }
        })
}
pub fn element_children(
    fragment: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<ElementChildren, String> {
    let wrapped = format!("{}{fragment}</Root>", fragment_context.wrapper_start);
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    let root = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let root_context = fragment_context.with_local_declarations(&wrapped, root)?;
    let children = root.children().collect::<Vec<_>>();
    let has_controlled_elements = children
        .iter()
        .any(|child| writer_controls_child(root, *child));
    let mut occurrences = BTreeMap::<ExpandedElementName, usize>::new();
    let mut text_occurrence = 0;
    let mut labels = Vec::with_capacity(children.len());
    let mut controlled = Vec::new();
    let mut anchors = Vec::new();
    for child in &children {
        if child.is_text() && !has_controlled_elements {
            let anchor = ChildAnchor::Text(text_occurrence);
            text_occurrence += 1;
            anchors.push((
                anchor.clone(),
                child.range().start - fragment_context.wrapper_start.len(),
            ));
            labels.push(Some(anchor));
            continue;
        }
        if !writer_controls_child(root, *child) {
            labels.push(None);
            continue;
        }
        let key = expanded_element_name(*child);
        let occurrence = *occurrences.entry(key.clone()).or_default();
        occurrences.insert(key.clone(), occurrence + 1);
        let range = child.range();
        let anchor = ChildAnchor::Element(key.clone(), occurrence);
        anchors.push((
            anchor.clone(),
            range.start - fragment_context.wrapper_start.len(),
        ));
        labels.push(Some(anchor));
        controlled.push(ControlledChildFragment {
            key,
            occurrence,
            range: range.start - fragment_context.wrapper_start.len()
                ..range.end - fragment_context.wrapper_start.len(),
            context: root_context.with_local_declarations(&wrapped, *child)?,
            has_preservation_obligation: controlled_subtree_has_preservation_obligation(*child),
        });
    }

    let mut preserved = Vec::new();
    for (index, child) in children.iter().enumerate() {
        if child.is_text() || labels[index].is_some() {
            continue;
        }
        preserved.push(PreservedChildFragment {
            source: wrapped[child.range()].to_string(),
            following_anchors: labels[index + 1..]
                .iter()
                .filter_map(Clone::clone)
                .collect(),
        });
    }
    Ok(ElementChildren {
        controlled,
        preserved,
        anchors,
    })
}
pub fn expand_self_closing_for_preserved_children(fragment: &mut String) -> Result<(), String> {
    if !fragment.trim_end().ends_with("/>") {
        return Ok(());
    }
    let qname = lexical_element_name(fragment)?.to_string();
    let slash = fragment
        .rfind("/>")
        .ok_or_else(|| "predefined self-closing element terminator is unavailable".to_string())?;
    fragment.replace_range(slash..slash + 2, &format!("></{qname}>"));
    Ok(())
}
pub fn closing_element_start(fragment: &str) -> Result<usize, String> {
    let qname = lexical_element_name(fragment)?;
    fragment
        .rfind(&format!("</{qname}>"))
        .ok_or_else(|| format!("predefined {qname} closing tag is unavailable"))
}
pub fn preserve_element_extensions_with_contexts(
    existing: &str,
    rendered: &str,
    existing_context: &FragmentXmlContext,
    rendered_context: &FragmentXmlContext,
) -> Result<String, String> {
    let existing_children = element_children(existing, existing_context)?;
    let mut merged =
        preserve_element_attributes(existing, rendered, existing_context, rendered_context)?;
    let rendered_children = element_children(&merged, rendered_context)?;
    let rendered_by_key = rendered_children
        .controlled
        .iter()
        .map(|child| ((child.key.clone(), child.occurrence), child))
        .collect::<BTreeMap<_, _>>();
    let mut replacements = Vec::new();
    for existing_child in &existing_children.controlled {
        let Some(rendered_child) =
            rendered_by_key.get(&(existing_child.key.clone(), existing_child.occurrence))
        else {
            if existing_child.has_preservation_obligation {
                return Err(
                    "predefined type update would discard unknown XML from a removed typed branch"
                        .to_string(),
                );
            }
            continue;
        };
        replacements.push((
            rendered_child.range.clone(),
            preserve_element_extensions_with_contexts(
                &existing[existing_child.range.clone()],
                &merged[rendered_child.range.clone()],
                &existing_child.context,
                &rendered_child.context,
            )?,
        ));
    }
    replacements.sort_by_key(|(range, _)| range.start);
    for (range, replacement) in replacements.into_iter().rev() {
        merged.replace_range(range, &replacement);
    }

    if existing_children.preserved.is_empty() {
        return Ok(merged);
    }
    expand_self_closing_for_preserved_children(&mut merged)?;
    let current_children = element_children(&merged, rendered_context)?;
    let anchors = current_children
        .anchors
        .into_iter()
        .collect::<BTreeMap<_, _>>();
    let closing = closing_element_start(&merged)?;
    let mut insertions = existing_children
        .preserved
        .into_iter()
        .enumerate()
        .map(|(sequence, preserved)| {
            let position = preserved
                .following_anchors
                .iter()
                .find_map(|anchor| anchors.get(anchor).copied())
                .unwrap_or(closing);
            (position, sequence, preserved.source)
        })
        .collect::<Vec<_>>();
    insertions.sort_by_key(|(position, sequence, _)| (*position, *sequence));
    for (position, _, source) in insertions.into_iter().rev() {
        merged.insert_str(position, &source);
    }
    Ok(merged)
}
pub fn preserve_element_extensions(
    existing: &str,
    rendered: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<String, String> {
    preserve_element_extensions_with_contexts(
        existing,
        rendered,
        fragment_context,
        fragment_context,
    )
}
pub fn set_complex_child(
    parent: &mut String,
    tag: &str,
    rendered: &str,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    if let Some(location) = fragment_direct_child_location(parent, tag, None, fragment_context)? {
        let range = location.range;
        let existing_qname = lexical_element_name(&parent[range.clone()])?;
        let prefix = lexical_element_prefix(existing_qname);
        let rendered = qualify_generated_predefined_elements(rendered, prefix);
        let rendered =
            preserve_element_extensions(&parent[range.clone()], &rendered, &location.context)?;
        if parent[range.clone()] != rendered {
            parent.replace_range(range, &rendered);
        }
        return Ok(());
    }
    let parent_tag = fragment_root_name(parent, fragment_context)?;
    let prefix = lexical_element_prefix(&parent_tag);
    let rendered = qualify_generated_predefined_elements(rendered, prefix);
    expand_self_closing(parent, &parent_tag, eol, "");
    let indent = child_indent(parent);
    if let Some(before) = ordered_insertion_anchor(parent, tag, fragment_context)? {
        parent.insert_str(before, &format!("{rendered}{eol}{indent}"));
    } else {
        let closing_tag = format!("</{parent_tag}>");
        let close = parent
            .rfind(&closing_tag)
            .ok_or_else(|| format!("predefined {parent_tag} closing tag is unavailable"))?;
        parent.insert_str(close, &format!("{eol}{indent}{rendered}"));
    }
    Ok(())
}
pub fn merge_flags_container(
    parent: &mut String,
    tag: &str,
    requested: &BTreeMap<String, bool>,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    if requested.is_empty() {
        if let Some(location) = fragment_direct_child_location(parent, tag, None, fragment_context)?
        {
            let range = location.range;
            let mut container = parent[range.clone()].to_string();
            remove_direct_children(&mut container, "Flag", &location.context)?;
            if parent[range.clone()] != container {
                parent.replace_range(range, &container);
            }
        }
        return Ok(());
    }
    if fragment_direct_child_range(parent, tag, None, fragment_context)?.is_none() {
        set_complex_child(parent, tag, &format!("<{tag}/>"), eol, fragment_context)?;
    }
    let container_location = fragment_direct_child_location(parent, tag, None, fragment_context)?
        .ok_or_else(|| format!("predefined {tag} is unavailable"))?;
    let range = container_location.range;
    let container_context = container_location.context;
    let mut container = parent[range.clone()].to_string();
    let container_qname = lexical_element_name(&container)?.to_string();
    let container_prefix = lexical_element_prefix(&container_qname);
    let indent = format!("{}\t", child_indent(parent));
    expand_self_closing(&mut container, &container_qname, eol, &child_indent(parent));
    for (name, value) in requested {
        if let Some(flag_location) = fragment_direct_child_location(
            &container,
            "Flag",
            Some(("ref", name.as_str())),
            &container_context,
        )? {
            // Имя метаданных сравнивается без регистра, но в файле остаётся
            // написание источника: правка значения не переименовывает ссылку.
            let stored_name = flag_location.matched_attribute.as_deref().unwrap_or(name);
            let rendered = format!(
                "<Flag ref=\"{}\">{value}</Flag>",
                unica_xml_format::text::escape_xml(stored_name)
            );
            let flag_range = flag_location.range;
            let existing_qname = lexical_element_name(&container[flag_range.clone()])?;
            let rendered = qualify_generated_predefined_elements(
                &rendered,
                lexical_element_prefix(existing_qname),
            );
            let rendered = preserve_element_extensions(
                &container[flag_range.clone()],
                &rendered,
                &flag_location.context,
            )?;
            if container[flag_range.clone()] != rendered {
                container.replace_range(flag_range, &rendered);
            }
        } else {
            let rendered = format!(
                "<Flag ref=\"{}\">{value}</Flag>",
                unica_xml_format::text::escape_xml(name)
            );
            let rendered = qualify_generated_predefined_elements(&rendered, container_prefix);
            let close = container
                .rfind(&format!("</{container_qname}>"))
                .ok_or_else(|| format!("predefined {tag} closing tag is unavailable"))?;
            container.insert_str(close, &format!("{eol}{indent}{rendered}"));
        }
    }
    parent.replace_range(range, &container);
    Ok(())
}
pub fn merge_ext_dimensions(
    item: &mut String,
    requested: &[MetaPredefinedExtDimensionType],
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    if requested.is_empty() {
        if let Some(location) =
            fragment_direct_child_location(item, "ExtDimensionTypes", None, fragment_context)?
        {
            let range = location.range;
            let mut container = item[range.clone()].to_string();
            remove_direct_children(&mut container, "ExtDimensionType", &location.context)?;
            if item[range.clone()] != container {
                item.replace_range(range, &container);
            }
        }
        return Ok(());
    }
    if fragment_direct_child_range(item, "ExtDimensionTypes", None, fragment_context)?.is_none() {
        set_complex_child(
            item,
            "ExtDimensionTypes",
            "<ExtDimensionTypes/>",
            eol,
            fragment_context,
        )?;
    }
    let container_location =
        fragment_direct_child_location(item, "ExtDimensionTypes", None, fragment_context)?
            .ok_or_else(|| "ExtDimensionTypes is unavailable".to_string())?;
    let range = container_location.range;
    let container_context = container_location.context;
    let mut container = item[range.clone()].to_string();
    let container_qname = lexical_element_name(&container)?.to_string();
    let container_prefix = lexical_element_prefix(&container_qname);
    let container_indent = child_indent(item);
    expand_self_closing(&mut container, &container_qname, eol, &container_indent);
    for requested in requested {
        if let Some(item_location) = fragment_direct_child_location(
            &container,
            "ExtDimensionType",
            Some(("name", requested.name.as_str())),
            &container_context,
        )? {
            let item_range = item_location.range;
            let mut existing = container[item_range.clone()].to_string();
            if let Some(turnover) = requested.turnover {
                set_simple_child(
                    &mut existing,
                    "Turnover",
                    &turnover.to_string(),
                    eol,
                    &item_location.context,
                )?;
            }
            if let Some(flags) = &requested.accounting_flags {
                merge_flags_container(
                    &mut existing,
                    "AccountingFlags",
                    flags,
                    eol,
                    &item_location.context,
                )?;
            }
            container.replace_range(item_range, &existing);
        } else {
            let indent = format!("{container_indent}\t");
            let rendered = qualify_generated_predefined_elements(
                &render_ext_dimension(requested, &indent, eol),
                container_prefix,
            );
            let close = container
                .rfind(&format!("</{container_qname}>"))
                .ok_or_else(|| "ExtDimensionTypes closing tag is unavailable".to_string())?;
            container.insert_str(close, &format!("{eol}{rendered}"));
        }
    }
    item.replace_range(range, &container);
    Ok(())
}
pub fn render_flags(
    tag: &str,
    flags: Option<&BTreeMap<String, bool>>,
    indent: &str,
) -> Vec<String> {
    let Some(flags) = flags.filter(|flags| !flags.is_empty()) else {
        return vec![format!("{indent}<{tag}/>")];
    };
    let mut lines = vec![format!("{indent}<{tag}>")];
    for (name, value) in flags {
        lines.push(format!(
            "{indent}\t<Flag ref=\"{}\">{value}</Flag>",
            unica_xml_format::text::escape_xml(name)
        ));
    }
    lines.push(format!("{indent}</{tag}>"));
    lines
}
pub fn render_ext_dimensions(
    values: Option<&[MetaPredefinedExtDimensionType]>,
    indent: &str,
) -> Vec<String> {
    let Some(values) = values.filter(|values| !values.is_empty()) else {
        return vec![format!("{indent}<ExtDimensionTypes/>")];
    };
    let mut lines = vec![format!("{indent}<ExtDimensionTypes>")];
    for value in values {
        lines.extend(render_ext_dimension_lines(value, &format!("{indent}\t")));
    }
    lines.push(format!("{indent}</ExtDimensionTypes>"));
    lines
}
pub fn render_ext_dimension(
    value: &MetaPredefinedExtDimensionType,
    indent: &str,
    eol: &str,
) -> String {
    render_ext_dimension_lines(value, indent).join(eol)
}
pub fn render_ext_dimension_lines(
    value: &MetaPredefinedExtDimensionType,
    indent: &str,
) -> Vec<String> {
    let mut lines = vec![format!(
        "{indent}<ExtDimensionType name=\"{}\">",
        unica_xml_format::text::escape_xml(&value.name)
    )];
    lines.push(format!(
        "{indent}\t<Turnover>{}</Turnover>",
        value.turnover.unwrap_or(false)
    ));
    lines.extend(render_flags(
        "AccountingFlags",
        value.accounting_flags.as_ref(),
        &format!("{indent}\t"),
    ));
    lines.push(format!("{indent}</ExtDimensionType>"));
    lines
}
pub fn render_type(
    metadata_type: &MetadataType,
    indent: &str,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> String {
    let mut lines = Vec::new();
    crate::xml_model::emit_meta_typed_value_type(&mut lines, indent, metadata_type);
    let (v8_prefix, declare_v8) = fragment_context.generated_prefix(V8_NS, "v8");
    let (xs_prefix, declare_xs) = fragment_context.generated_prefix(XS_NS, "xs");
    let (cfg_prefix, declare_cfg) = fragment_context.generated_prefix(CFG_NS, "cfg");
    for line in &mut lines {
        *line = line
            .replace("<v8:", &format!("<{v8_prefix}:"))
            .replace("</v8:", &format!("</{v8_prefix}:"))
            .replace(">xs:", &format!(">{xs_prefix}:"))
            .replace(">cfg:", &format!(">{cfg_prefix}:"))
            .replace(">v8:", &format!(">{v8_prefix}:"));
    }
    let rendered_uses_xs = lines
        .iter()
        .any(|line| line.contains(&format!(">{xs_prefix}:")));
    let rendered_uses_cfg = lines
        .iter()
        .any(|line| line.contains(&format!(">{cfg_prefix}:")));
    if let Some(opening) = lines.first_mut() {
        let mut declarations = Vec::new();
        if declare_v8 {
            declarations.push(format!("xmlns:{v8_prefix}=\"{V8_NS}\""));
        }
        if rendered_uses_xs && declare_xs {
            declarations.push(format!("xmlns:{xs_prefix}=\"{XS_NS}\""));
        }
        if rendered_uses_cfg && declare_cfg {
            declarations.push(format!("xmlns:{cfg_prefix}=\"{CFG_NS}\""));
        }
        if !declarations.is_empty() {
            opening.insert_str(opening.len() - 1, &format!(" {}", declarations.join(" ")));
        }
    }
    lines.join(eol)
}
pub fn parse_item_type(_source: &str, node: Node<'_, '_>) -> Result<MetadataType, String> {
    let observed = crate::info_projection::parse_observed_predefined_type_node(node)
        .map_err(|diagnostic| diagnostic.message)?;
    MetadataType::try_from(observed).map_err(|diagnostic| diagnostic.message)
}
pub fn fragment_root_name(
    fragment: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<String, String> {
    let wrapped = format!("{}{fragment}</Root>", fragment_context.wrapper_start);
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    let node = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no root element".to_string())?;
    let range = node.range();
    let relative = range.start - fragment_context.wrapper_start.len()
        ..range.end - fragment_context.wrapper_start.len();
    lexical_element_name(&fragment[relative]).map(str::to_string)
}
pub fn lexical_element_name(source: &str) -> Result<&str, String> {
    source
        .strip_prefix('<')
        .and_then(|source| {
            let end = source
                .find(|character: char| {
                    character.is_ascii_whitespace() || matches!(character, '/' | '>')
                })
                .unwrap_or(source.len());
            source.get(..end)
        })
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "predefined element QName cannot be preserved".to_string())
}
pub fn lexical_element_prefix(qname: &str) -> &str {
    qname
        .rsplit_once(':')
        .map_or("", |(prefix, _)| &qname[..prefix.len() + 1])
}
pub fn qualify_generated_predefined_elements(fragment: &str, prefix: &str) -> String {
    if prefix.is_empty() {
        return fragment.to_string();
    }
    const GENERATED_NAMES: &[&str] = &[
        "Item",
        "Name",
        "Code",
        "Description",
        "IsFolder",
        "Type",
        "AccountType",
        "OffBalance",
        "Order",
        "AccountingFlags",
        "Flag",
        "ExtDimensionTypes",
        "ExtDimensionType",
        "Turnover",
        "ActionPeriodIsBase",
        "ChildItems",
    ];

    let bytes = fragment.as_bytes();
    let mut insertions = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'<' {
            index += 1;
            continue;
        }
        let mut name_start = index + 1;
        if bytes.get(name_start) == Some(&b'/') {
            name_start += 1;
        }
        if matches!(bytes.get(name_start), Some(b'!' | b'?')) {
            index = name_start + 1;
            continue;
        }
        let mut name_end = name_start;
        while bytes
            .get(name_end)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'/' | b'>'))
        {
            name_end += 1;
        }
        if let Some(name) = fragment.get(name_start..name_end) {
            if !name.contains(':') && GENERATED_NAMES.contains(&name) {
                insertions.push(name_start);
            }
        }
        index = name_end.max(index + 1);
    }
    let mut qualified = fragment.to_string();
    for insertion in insertions.into_iter().rev() {
        qualified.insert_str(insertion, prefix);
    }
    qualified
}
pub fn ordered_insertion_anchor(
    parent: &str,
    inserted_tag: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<Option<usize>, String> {
    let wrapped = format!("{}{parent}</Root>", fragment_context.wrapper_start);
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    let root = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no root element".to_string())?;
    let inserted_rank = predefined_field_rank(root.tag_name().name(), inserted_tag);
    Ok(root
        .children()
        .filter(|node| node.is_element())
        .find(|node| {
            predefined_field_rank(root.tag_name().name(), node.tag_name().name()) > inserted_rank
        })
        .map(|node| node.range().start - fragment_context.wrapper_start.len()))
}
pub fn predefined_field_rank(parent: &str, tag: &str) -> usize {
    if parent == "ExtDimensionType" {
        return match tag {
            "Turnover" => 0,
            "AccountingFlags" => 1,
            _ => 50,
        };
    }
    match tag {
        "Name" => 0,
        "Code" => 1,
        "Description" => 2,
        "Type" | "AccountType" | "ActionPeriodIsBase" => 3,
        "IsFolder" | "OffBalance" => 4,
        "Order" => 5,
        "AccountingFlags" => 6,
        "ExtDimensionTypes" => 7,
        "ChildItems" => 100,
        _ => 50,
    }
}
#[derive(Clone)]
pub struct FragmentChildLocation {
    pub range: Range<usize>,
    pub context: FragmentXmlContext,
    /// Написание значения атрибута, уже лежащее в источнике. Имена метаданных
    /// 1С сравниваются без учёта регистра, но переписывать чужое написание
    /// мутация не должна.
    pub matched_attribute: Option<String>,
}
pub fn fragment_direct_child_location(
    fragment: &str,
    tag: &str,
    attribute: Option<(&str, &str)>,
    fragment_context: &FragmentXmlContext,
) -> Result<Option<FragmentChildLocation>, String> {
    let wrapped = format!("{}{fragment}</Root>", fragment_context.wrapper_start);
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    let parent = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let Some(node) = parent.children().find(|node| {
        is_predefined_element(*node, tag)
            && attribute.is_none_or(|(name, value)| {
                node.attribute(name)
                    .is_some_and(|existing| metadata_name_eq(existing, value))
            })
    }) else {
        return Ok(None);
    };
    let range = node.range();
    let parent_context = fragment_context.with_local_declarations(&wrapped, parent)?;
    Ok(Some(FragmentChildLocation {
        range: range.start - fragment_context.wrapper_start.len()
            ..range.end - fragment_context.wrapper_start.len(),
        context: parent_context.with_local_declarations(&wrapped, node)?,
        matched_attribute: attribute
            .and_then(|(name, _)| node.attribute(name))
            .map(str::to_string),
    }))
}
pub fn fragment_direct_child_range(
    fragment: &str,
    tag: &str,
    attribute: Option<(&str, &str)>,
    fragment_context: &FragmentXmlContext,
) -> Result<Option<Range<usize>>, String> {
    Ok(
        fragment_direct_child_location(fragment, tag, attribute, fragment_context)?
            .map(|location| location.range),
    )
}
pub fn fragment_direct_child_ranges(
    fragment: &str,
    tag: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<Vec<Range<usize>>, String> {
    let wrapped = format!("{}{fragment}</Root>", fragment_context.wrapper_start);
    let document =
        Document::parse(&wrapped).map_err(|error| format!("predefined XML fragment: {error}"))?;
    let parent = document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    Ok(parent
        .children()
        .filter(|node| is_predefined_element(*node, tag))
        .map(|node| {
            let range = node.range();
            range.start - fragment_context.wrapper_start.len()
                ..range.end - fragment_context.wrapper_start.len()
        })
        .collect())
}
pub fn remove_direct_children(
    fragment: &mut String,
    tag: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    let ranges = fragment_direct_child_ranges(fragment, tag, fragment_context)?;
    for range in ranges.into_iter().rev() {
        let range = removal_range(fragment, range);
        fragment.replace_range(range, "");
    }
    Ok(())
}
pub fn expand_self_closing_root(text: &mut String, eol: &str) {
    let Ok(document) = Document::parse(text) else {
        return;
    };
    let root = document.root_element();
    let range = root.range();
    if text[range.clone()].trim_end().ends_with("/>") {
        let fragment = &text[range.clone()];
        let Ok(qname) = lexical_element_name(fragment) else {
            return;
        };
        let slash = fragment
            .rfind("/>")
            .expect("self-closing root has terminator");
        let replacement = format!("{}>{eol}</{qname}>", &fragment[..slash]);
        text.replace_range(range, &replacement);
    }
}
pub fn expand_self_closing(text: &mut String, tag: &str, eol: &str, indent: &str) {
    if !text.trim_end().ends_with("/>") {
        return;
    }
    if let Some(slash) = text.rfind("/>") {
        text.replace_range(slash..slash + 2, &format!(">{eol}{indent}</{tag}>"));
    }
}
pub fn child_indent(fragment: &str) -> String {
    for line in fragment.lines().skip(1) {
        let indent = line
            .chars()
            .take_while(|ch| ch.is_whitespace())
            .collect::<String>();
        if line.trim_start().starts_with('<') {
            return indent;
        }
    }
    "\t".to_string()
}
pub fn removal_range(text: &str, range: Range<usize>) -> Range<usize> {
    let line_start = text[..range.start]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    if !text[line_start..range.start].trim().is_empty() {
        return range;
    }
    let mut line_end = range.end;
    while line_end < text.len() && matches!(text.as_bytes()[line_end], b' ' | b'\t') {
        line_end += 1;
    }
    let eol_end = if text[line_end..].starts_with("\r\n") {
        Some(line_end + 2)
    } else if line_end < text.len() && matches!(text.as_bytes()[line_end], b'\r' | b'\n') {
        Some(line_end + 1)
    } else {
        None
    };
    if let Some(eol_end) = eol_end {
        line_start..eol_end
    } else {
        range
    }
}
pub fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|child| is_predefined_element(*child, name))
}
pub fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name).map(direct_text_content)
}
pub fn direct_text_content(node: Node<'_, '_>) -> String {
    node.children()
        .filter(|child| child.is_text())
        .filter_map(|child| child.text())
        .collect()
}
pub fn is_predefined_element(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(PREDEFINED_NS)
        && node.tag_name().name() == name
}
pub fn root_type(kind: MetadataKind) -> Result<&'static str, String> {
    match kind {
        MetadataKind::Catalog => Ok("CatalogPredefinedItems"),
        MetadataKind::ChartOfAccounts => Ok("ChartOfAccountsPredefinedItems"),
        MetadataKind::ChartOfCharacteristicTypes => Ok("PlanOfCharacteristicKindPredefinedItems"),
        MetadataKind::ChartOfCalculationTypes => Ok("CalculationTypePredefinedItems"),
        _ => Err(format!("{} does not own predefined data", kind.as_str())),
    }
}
pub fn empty_document(kind: MetadataKind, eol: &str) -> String {
    let root_type = root_type(kind).expect("domain capability validated predefined owner");
    // Новый companion наследует EOL описателя, рядом с которым живёт, и, как
    // платформенный экспорт, не несёт хвостового перевода строки.
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>{eol}<PredefinedData xmlns=\"{PREDEFINED_NS}\" xmlns:v8=\"{V8_NS}\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"{XSI_NS}\" xsi:type=\"{root_type}\" version=\"{}\">{eol}</PredefinedData>",
        ACTIVE_FORMAT_PROFILE.export_format
    )
}
pub fn source_eol(text: &str) -> Result<&'static str, String> {
    let snapshot = SourceTextSnapshot::from_bytes(text.as_bytes())
        .map_err(|error| format!("Predefined.xml {error}"))?;
    let policy = if snapshot.line_endings() == LineEndingProfile::None {
        EolPolicy::Lf
    } else {
        EolPolicy::Preserve
    };
    resolve_line_ending(policy, &snapshot, None)
        .map(|ending| ending.as_str())
        .map_err(|error| format!("Predefined.xml {error}"))
}
pub fn simple_line(tag: &str, value: &str, indent: &str) -> String {
    if value.is_empty() {
        format!("{indent}<{tag}/>")
    } else {
        format!(
            "{indent}<{tag}>{}</{tag}>",
            unica_xml_format::text::escape_xml(value)
        )
    }
}
pub fn failure(
    owner: &MetadataAddress,
    code: MetaDiagnosticCode,
    message: impl Into<String>,
    operation_index: Option<usize>,
    field: Option<String>,
) -> unica_metadata::request::MetaFailure {
    let mut diagnostic = MetaDiagnostic::error(code, message).with_metadata_path(owner.clone());
    if let Some(operation_index) = operation_index {
        diagnostic = diagnostic.with_operation_index(operation_index);
    }
    if let Some(field) = field {
        diagnostic.field = Some(format!(
            "operations[{}].{field}",
            operation_index.unwrap_or_default()
        ));
    }
    diagnostic.into()
}

pub fn validate_predefined_image(kind: MetadataKind, bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Predefined.xml is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    validate_predefined_text(kind, text)
}

pub fn validate_predefined_text(kind: MetadataKind, text: &str) -> Result<(), String> {
    validate_predefined_text_with_options(kind, None, text, false)
}

use crate::resource_plan::{TypedChildFileMutation, TypedChildResourcePlan};
use std::io::ErrorKind;
use std::path::Path;
/// Source-contained ordinary reads at the existing resource phase.
/// The binding reuses the actual secure reader and cannot publish bytes.
pub trait PredefinedResourceRead {
    fn read_bounded_resource(
        &self,
        source_root: &Path,
        path: &Path,
        maximum: usize,
    ) -> std::io::Result<Vec<u8>>;
    /// The resource up to the apply file limit: the plan reads no more than apply stages.
    fn read_resource(&self, source_root: &Path, path: &Path) -> std::io::Result<Vec<u8>>;
    /// The whole resource, whatever its size (#1119).
    fn read_whole_resource(&self, source_root: &Path, path: &Path) -> std::io::Result<Vec<u8>>;
}

pub struct PlannedPredefinedResource<G> {
    pub resources: TypedChildResourcePlan<G>,
    pub effects: Vec<MetaMutationEffect>,
}

/// Descriptor edits that never reach `Ext/Predefined.xml` must not require a
/// resolvable source set: the caller resolves it only when this returns true.
pub fn descriptor_edit_requires_predefined_planning(operations: &[MetaEditOperation]) -> bool {
    operations.iter().any(is_predefined_operation)
        || operations.iter().any(operation_touches_code_type)
}

pub fn plan_predefined_resource_after_descriptor_edit<G>(
    reader: &dyn PredefinedResourceRead,
    source_root: &Path,
    descriptor: crate::resource_plan::MetadataDescriptorTarget<'_>,
    kind: MetadataKind,
    descriptor_pre_image: &str,
    descriptor_post_image: &str,
    operations: &[MetaEditOperation],
) -> Result<PlannedPredefinedResource<G>, unica_metadata::request::MetaFailure> {
    let crate::resource_plan::MetadataDescriptorTarget {
        path: descriptor_path,
        address: owner,
    } = descriptor;

    let has_predefined_operations = operations.iter().any(is_predefined_operation);
    let code_type_is_touched = operations.iter().any(operation_touches_code_type);
    if !has_predefined_operations && !code_type_is_touched {
        return Ok(PlannedPredefinedResource {
            resources: TypedChildResourcePlan::default(),
            effects: Vec::new(),
        });
    }
    let code_type_failure = |message: String| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            None,
            Some("properties.CodeType".to_string()),
        )
    };
    // Читатель не выдумывает `CodeType`, которого описание не несёт
    // (заимствованный справочник платформа выгружает без него): писатель
    // тоже не проверяет коды по придуманному виду и отказывает тем же текстом.
    let code_type = predefined_code_type(kind, descriptor_post_image)
        .map_err(code_type_failure)?
        .ok_or_else(|| code_type_failure(MISSING_PREDEFINED_CODE_TYPE.to_string()))?;
    let code_type_changed = if code_type_is_touched {
        predefined_code_type(kind, descriptor_pre_image).map_err(code_type_failure)?
            != Some(code_type)
    } else {
        false
    };
    if !has_predefined_operations && !code_type_changed {
        return Ok(PlannedPredefinedResource {
            resources: TypedChildResourcePlan::default(),
            effects: Vec::new(),
        });
    }

    let path = descriptor_path
        .with_extension("")
        .join("Ext/Predefined.xml");
    let original = match reader.read_resource(source_root, &path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(_) => {
            return Err(failure(
                owner,
                MetaDiagnosticCode::ProviderUnavailable,
                "predefined data cannot be read through the source-root boundary",
                None,
                None,
            ))
        }
    };
    if original.is_none() && !has_predefined_operations {
        let mut resources = TypedChildResourcePlan::default();
        resources.absent_path_guards.push(path);
        return Ok(PlannedPredefinedResource {
            resources,
            effects: Vec::new(),
        });
    }
    let (mut text, has_bom) = match &original {
        Some(bytes) => {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                failure(
                    owner,
                    MetaDiagnosticCode::ProviderUnavailable,
                    "Predefined.xml is not UTF-8",
                    None,
                    None,
                )
            })?;
            (
                text.trim_start_matches('\u{feff}').to_string(),
                bytes.starts_with(b"\xef\xbb\xbf"),
            )
        }
        None => (
            empty_document(kind, source_eol(descriptor_post_image).unwrap_or("\n")),
            true,
        ),
    };
    validate_predefined_text_allowing_self_closing_items(kind, &text).map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            None,
            None,
        )
    })?;
    let observed_eol = source_eol(&text);
    let rendering_eol = observed_eol.as_deref().unwrap_or("\n");
    let mut effects = Vec::new();
    for (operation_index, operation) in operations.iter().enumerate() {
        if !is_predefined_operation(operation) {
            continue;
        }
        let before = selected_effect(&text, kind, operation, false).map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                message,
                Some(operation_index),
                None,
            )
        })?;
        let operation_preimage = text.clone();
        apply_operation(
            &mut text,
            rendering_eol,
            kind,
            code_type,
            operation,
            owner,
            operation_index,
        )?;
        if text != operation_preimage {
            if let Err(message) = &observed_eol {
                return Err(failure(
                    owner,
                    MetaDiagnosticCode::ValidationFailed,
                    message.clone(),
                    Some(operation_index),
                    None,
                ));
            }
        }
        validate_predefined_text_allowing_self_closing_items(kind, &text).map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                message,
                Some(operation_index),
                None,
            )
        })?;
        let after = selected_effect(&text, kind, operation, true).map_err(|message| {
            failure(
                owner,
                MetaDiagnosticCode::ValidationFailed,
                message,
                Some(operation_index),
                None,
            )
        })?;
        effects.push(MetaMutationEffect {
            operation_index: Some(operation_index as u64),
            operation: operation_name(operation).to_string(),
            target: format!("{}.collections.predefinedItems", owner.as_str()),
            before,
            after,
        });
    }
    validate_predefined_text_for_code_type(kind, code_type, &text).map_err(|message| {
        let operation_index = if code_type_changed {
            operations.iter().rposition(operation_touches_code_type)
        } else {
            operations.iter().rposition(is_predefined_operation)
        };
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            operation_index,
            code_type_changed.then(|| "values.CodeType".to_string()),
        )
    })?;

    let mut post_image = Vec::with_capacity(text.len() + usize::from(has_bom) * 3);
    if has_bom {
        post_image.extend_from_slice(b"\xef\xbb\xbf");
    }
    post_image.extend_from_slice(text.as_bytes());

    // Пустой `PredefinedData` платформе не принадлежит: импортировав его,
    // 8.3.27.2074 при выгрузке файл не пишет вовсе. Поэтому уход последнего
    // элемента — удаление файла, а не запись пустого документа; и создавать
    // такой файл ради одного `CodeType` тоже нечего.
    if !document_has_items(&text).map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            None,
            None,
        )
    })? {
        let mut resources = TypedChildResourcePlan::default();
        match original {
            Some(original) => {
                resources.file_mutations.push(TypedChildFileMutation {
                    path: path.clone(),
                    pre_image: Some(original),
                    post_image: None,
                });
                resources.publication_plan.push(MetaPublicationPlanEntry {
                    action: MetaPublicationAction::Remove,
                    resource: MetaPublicationResource::PredefinedData,
                    metadata_path: Some(owner.clone()),
                });
                resources.absent_path_guards.push(path);
            }
            None => resources.absent_path_guards.push(path),
        }
        return Ok(PlannedPredefinedResource { resources, effects });
    }

    let changed = original.as_deref() != Some(post_image.as_slice());
    let mut resources = TypedChildResourcePlan::default();
    resources.validation_resources.push(
        unica_metadata::validation_subject::MetadataResourceImage {
            role: unica_metadata::validation_subject::MetadataResourceRole::AuxiliaryXml {
                owner: owner.clone(),
                kind: unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::PredefinedData(
                    kind,
                ),
            },
            bytes: post_image.clone(),
        },
    );
    if changed {
        resources.file_mutations.push(TypedChildFileMutation {
            path: path.clone(),
            pre_image: original.clone(),
            post_image: Some(post_image.clone()),
        });
        resources.publication_plan.push(MetaPublicationPlanEntry {
            action: if original.is_some() {
                MetaPublicationAction::Update
            } else {
                MetaPublicationAction::Create
            },
            resource: MetaPublicationResource::PredefinedData,
            metadata_path: Some(owner.clone()),
        });
        resources
            .expected_post_images
            .push((path, post_image.clone()));
    } else if let Some(original) = original {
        resources.exact_file_guards.push((path, original));
    }
    Ok(PlannedPredefinedResource { resources, effects })
}

#[cfg(any(test, feature = "test-support"))]
pub fn read_predefined_items(
    bytes: &[u8],
    kind: MetadataKind,
    limit: usize,
) -> Result<MetaPredefinedItemsData, String> {
    read_predefined_items_with_code_type(bytes, kind, None, limit)
}

pub fn read_predefined_items_for_code_type(
    bytes: &[u8],
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    limit: usize,
) -> Result<MetaPredefinedItemsData, String> {
    read_predefined_items_with_code_type(bytes, kind, Some(code_type), limit)
}

pub fn read_predefined_items_with_code_type(
    bytes: &[u8],
    kind: MetadataKind,
    code_type: Option<PredefinedCodeType>,
    limit: usize,
) -> Result<MetaPredefinedItemsData, String> {
    read_predefined_items_with_checkpoint(bytes, kind, code_type, Some(limit), &mut || Ok(()))
}

pub fn read_complete_predefined_items_for_code_type(
    bytes: &[u8],
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<MetaPredefinedItemsData, String> {
    read_predefined_items_with_checkpoint(bytes, kind, Some(code_type), None, checkpoint)
}

pub fn read_predefined_items_with_checkpoint(
    bytes: &[u8],
    kind: MetadataKind,
    code_type: Option<PredefinedCodeType>,
    limit: Option<usize>,
    checkpoint: &mut dyn FnMut() -> Result<(), String>,
) -> Result<MetaPredefinedItemsData, String> {
    checkpoint()?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Predefined.xml is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    validate_predefined_text_with_checkpoint(kind, code_type, text, false, checkpoint)?;
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    checkpoint()?;
    let mut items = Vec::new();
    for item in document
        .root_element()
        .children()
        .filter(|node| is_predefined_element(*node, "Item"))
    {
        collect_items(text, item, kind, None, &mut items, checkpoint)?;
    }
    let total = items.len();
    if let Some(limit) = limit {
        items.truncate(limit);
    }
    Ok(MetaPredefinedItemsData {
        total,
        returned: items.len(),
        truncated: total > items.len(),
        items,
    })
}

pub fn selected_effect(
    text: &str,
    kind: MetadataKind,
    operation: &MetaEditOperation,
    after: bool,
) -> Result<Option<Value>, String> {
    let ids = match operation {
        MetaEditOperation::AddPredefinedItems { elements } => elements
            .iter()
            .map(|element| element.id.as_str())
            .collect::<Vec<_>>(),
        MetaEditOperation::UpdatePredefinedItems { elements } => elements
            .iter()
            .map(|element| element.id.as_str())
            .collect::<Vec<_>>(),
        MetaEditOperation::RemovePredefinedItems { ids } => {
            ids.iter().map(String::as_str).collect::<Vec<_>>()
        }
        _ => return Ok(None),
    };
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    if matches!(operation, MetaEditOperation::RemovePredefinedItems { .. }) && !after {
        let requested = ids
            .into_iter()
            .map(|id| {
                uuid::Uuid::parse_str(id)
                    .map_err(|_| "predefined item id is not a UUID".to_string())
            })
            .collect::<Result<HashSet<_>, _>>()?;
        let mut selected = Vec::new();
        for item in document
            .descendants()
            .filter(|node| is_predefined_element(*node, "Item"))
        {
            let belongs_to_removed_subtree = std::iter::once(item)
                .chain(item.ancestors())
                .filter(|node| is_predefined_element(*node, "Item"))
                .filter_map(|node| node.attribute("id"))
                .filter_map(|id| uuid::Uuid::parse_str(id).ok())
                .any(|id| requested.contains(&id));
            if !belongs_to_removed_subtree {
                continue;
            }
            let parent_id = item
                .parent_element()
                .filter(|parent| is_predefined_element(*parent, "ChildItems"))
                .and_then(|parent| parent.parent_element())
                .and_then(|parent| parent.attribute("id"))
                .map(str::to_string);
            selected.push(parse_item(text, item, kind, parent_id)?);
        }
        return serde_json::to_value(selected)
            .map(Some)
            .map_err(|_| "predefined semantic effect cannot be serialized".to_string());
    }
    let mut selected = Vec::new();
    for requested in ids {
        let canonical = uuid::Uuid::parse_str(requested)
            .map_err(|_| "predefined item id is not a UUID".to_string())?;
        let item = document.descendants().find(|node| {
            is_predefined_element(*node, "Item")
                && node
                    .attribute("id")
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    == Some(canonical)
        });
        if let Some(item) = item {
            let parent_id = item
                .parent_element()
                .filter(|parent| is_predefined_element(*parent, "ChildItems"))
                .and_then(|parent| parent.parent_element())
                .and_then(|parent| parent.attribute("id"))
                .map(str::to_string);
            selected.push(parse_item(text, item, kind, parent_id)?);
        }
    }
    if selected.is_empty()
        && ((matches!(operation, MetaEditOperation::AddPredefinedItems { .. }) && !after)
            || (matches!(operation, MetaEditOperation::RemovePredefinedItems { .. }) && after))
    {
        return Ok(None);
    }
    serde_json::to_value(selected)
        .map(Some)
        .map_err(|_| "predefined semantic effect cannot be serialized".to_string())
}

pub fn is_predefined_operation(operation: &MetaEditOperation) -> bool {
    matches!(
        operation,
        MetaEditOperation::AddPredefinedItems { .. }
            | MetaEditOperation::UpdatePredefinedItems { .. }
            | MetaEditOperation::RemovePredefinedItems { .. }
    )
}

pub fn operation_touches_code_type(operation: &MetaEditOperation) -> bool {
    matches!(
        operation,
        MetaEditOperation::SetProperties { values }
            if values
                .entries()
                .iter()
                .any(|(key, _)| *key == MetaPropertyKey::CodeType)
    )
}

pub fn operation_name(operation: &MetaEditOperation) -> &'static str {
    match operation {
        MetaEditOperation::AddPredefinedItems { .. } => "add",
        MetaEditOperation::UpdatePredefinedItems { .. } => "update",
        MetaEditOperation::RemovePredefinedItems { .. } => "remove",
        _ => unreachable!("called only for predefined operation"),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    pub const PREDEFINED_NS: &str = super::PREDEFINED_NS;
    pub const MD_CLASSES_NS: &str = super::MD_CLASSES_NS;
    pub const V8_NS: &str = super::V8_NS;
    pub const CFG_NS: &str = super::CFG_NS;
    pub const XSI_NS: &str = super::XSI_NS;
    pub const XS_NS: &str = super::XS_NS;
}
