use crate::application::metadata::MetaFailure;
use crate::application::ports::{
    MetadataAuxiliaryXmlKind, MetadataResourceImage, MetadataResourceRole,
};
use crate::domain::format_profile::ACTIVE_FORMAT_PROFILE;
use crate::domain::metadata::{
    metadata_decimal_shape, MetaDiagnostic, MetaDiagnosticCode, MetaEditOperation,
    MetaMutationEffect, MetaPredefinedAccountType, MetaPredefinedExtDimensionType,
    MetaPredefinedExtDimensionTypeData, MetaPredefinedFields, MetaPredefinedItemAdd,
    MetaPredefinedItemData, MetaPredefinedItemUpdate, MetaPredefinedItemsData,
    MetaPublicationAction, MetaPublicationPlanEntry, MetaPublicationResource, MetadataKind,
    MetadataType,
};
use crate::domain::source_target::MetadataAddress;
use crate::infrastructure::platform::secure_read::read_root_relative_regular_file;
use crate::infrastructure::platform_xml_roots::{
    platform_xml_root_versioning, PlatformXmlRootVersioning,
};
use roxmltree::{Document, Node};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::io::ErrorKind;
use std::ops::Range;
use std::path::Path;

use super::super::common::escape_xml;
use super::edit::{TypedChildFileMutation, TypedChildResourcePlan};
use super::xml_model::emit_meta_typed_value_type;

const PREDEFINED_NS: &str = "http://v8.1c.ru/8.3/xcf/predef";
const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const V8_NS: &str = "http://v8.1c.ru/8.1/data/core";
const CFG_NS: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XR_NS: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XS_NS: &str = "http://www.w3.org/2001/XMLSchema";
const MAX_PREDEFINED_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone)]
struct FragmentXmlContext {
    wrapper_start: String,
    in_scope_prefixes: BTreeMap<String, String>,
}

impl FragmentXmlContext {
    #[cfg(test)]
    fn platform_default() -> Self {
        Self::from_namespaces(std::iter::empty::<(&str, &str)>())
    }

    fn for_node(node: Node<'_, '_>) -> Self {
        Self::from_namespaces(
            node.namespaces()
                .filter_map(|namespace| namespace.name().map(|name| (name, namespace.uri()))),
        )
    }

    fn from_namespaces<'a>(namespaces: impl Iterator<Item = (&'a str, &'a str)>) -> Self {
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
                escape_xml(&name),
                escape_xml(&uri)
            ));
        }
        wrapper_start.push('>');
        Self {
            wrapper_start,
            in_scope_prefixes,
        }
    }

    fn generated_prefix(&self, namespace: &str, canonical: &str) -> (String, bool) {
        if let Some((prefix, _)) = self
            .in_scope_prefixes
            .iter()
            .find(|(_, uri)| uri.as_str() == namespace)
        {
            return (prefix.clone(), false);
        }
        if !self.in_scope_prefixes.contains_key(canonical) {
            return (canonical.to_string(), true);
        }
        for suffix in 1.. {
            let candidate = format!("{canonical}_{suffix}");
            if !self.in_scope_prefixes.contains_key(&candidate) {
                return (candidate, true);
            }
        }
        unreachable!("a finite namespace context cannot occupy every generated prefix")
    }

    fn with_local_declarations(&self, source: &str, node: Node<'_, '_>) -> Result<Self, String> {
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
        Ok(Self::from_namespaces(prefixes.iter().map(
            |(prefix, namespace)| (prefix.as_str(), namespace.as_str()),
        )))
    }

    fn decimal_type_attributes(&self) -> String {
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

pub(super) struct PlannedPredefinedResource {
    pub(super) resources: TypedChildResourcePlan,
    pub(super) effects: Vec<MetaMutationEffect>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PredefinedCodeType {
    String,
    Number,
}

#[derive(Clone, Copy)]
struct PredefinedOperationContext<'a> {
    owner: &'a MetadataAddress,
    operation_index: usize,
}

pub(super) fn plan_predefined_resource(
    source_root: &Path,
    descriptor_path: &Path,
    owner: &MetadataAddress,
    kind: MetadataKind,
    descriptor_post_image: &str,
    operations: &[MetaEditOperation],
) -> Result<PlannedPredefinedResource, MetaFailure> {
    if !operations.iter().any(is_predefined_operation) {
        return Ok(PlannedPredefinedResource {
            resources: TypedChildResourcePlan::default(),
            effects: Vec::new(),
        });
    }
    let code_type = predefined_code_type(kind, descriptor_post_image).map_err(|message| {
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            None,
            Some("properties.CodeType".to_string()),
        )
    })?;

    let path = descriptor_path
        .with_extension("")
        .join("Ext/Predefined.xml");
    let original =
        match read_root_relative_regular_file(source_root, &path, MAX_PREDEFINED_BYTES, |_| {}) {
            Ok(read) => Some(read.bytes),
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
        None => (empty_document(kind), true),
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
    let eol = source_eol(&text);
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
        apply_operation(
            &mut text,
            eol,
            kind,
            code_type,
            operation,
            owner,
            operation_index,
        )?;
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
        failure(
            owner,
            MetaDiagnosticCode::ValidationFailed,
            message,
            operations.iter().rposition(is_predefined_operation),
            None,
        )
    })?;

    let mut post_image = Vec::with_capacity(text.len() + usize::from(has_bom) * 3);
    if has_bom {
        post_image.extend_from_slice(b"\xef\xbb\xbf");
    }
    post_image.extend_from_slice(text.as_bytes());
    let changed = original.as_deref() != Some(post_image.as_slice());
    let mut resources = TypedChildResourcePlan::default();
    resources.validation_resources.push(MetadataResourceImage {
        role: MetadataResourceRole::AuxiliaryXml {
            owner: owner.clone(),
            kind: MetadataAuxiliaryXmlKind::PredefinedData(kind),
        },
        bytes: post_image.clone(),
    });
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

fn predefined_code_type(
    kind: MetadataKind,
    descriptor_post_image: &str,
) -> Result<PredefinedCodeType, String> {
    if !matches!(
        kind,
        MetadataKind::Catalog | MetadataKind::ChartOfCalculationTypes
    ) {
        return Ok(PredefinedCodeType::String);
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
    let code_type = code_types
        .next()
        .and_then(|node| node.text())
        .map(str::trim)
        .unwrap_or("String");
    if code_types.next().is_some() {
        return Err(format!(
            "{} descriptor has duplicate direct CodeType",
            kind.as_str()
        ));
    }
    match code_type {
        "String" => Ok(PredefinedCodeType::String),
        "Number" => Ok(PredefinedCodeType::Number),
        other => Err(format!(
            "{} descriptor CodeType `{other}` is unsupported",
            kind.as_str()
        )),
    }
}

pub(crate) fn read_predefined_items(
    bytes: &[u8],
    kind: MetadataKind,
    limit: usize,
) -> Result<MetaPredefinedItemsData, String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Predefined.xml is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    validate_predefined_text(kind, text)?;
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    let mut items = Vec::new();
    for item in document
        .root_element()
        .children()
        .filter(|node| is_predefined_element(*node, "Item"))
    {
        collect_items(text, item, kind, None, &mut items)?;
    }
    let total = items.len();
    items.truncate(limit);
    Ok(MetaPredefinedItemsData {
        total,
        returned: items.len(),
        truncated: total > items.len(),
        items,
    })
}

pub(super) fn validate_predefined_image(kind: MetadataKind, bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Predefined.xml is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    validate_predefined_text(kind, text)
}

fn validate_predefined_text(kind: MetadataKind, text: &str) -> Result<(), String> {
    validate_predefined_text_with_options(kind, None, text, false)
}

fn validate_predefined_text_for_code_type(
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    text: &str,
) -> Result<(), String> {
    validate_predefined_text_with_options(kind, Some(code_type), text, false)
}

fn validate_predefined_text_allowing_self_closing_items(
    kind: MetadataKind,
    text: &str,
) -> Result<(), String> {
    validate_predefined_text_with_options(kind, None, text, true)
}

fn validate_predefined_text_with_options(
    kind: MetadataKind,
    code_type: Option<PredefinedCodeType>,
    text: &str,
    allow_self_closing_items: bool,
) -> Result<(), String> {
    let document = Document::parse(text).map_err(|error| format!("Predefined.xml: {error}"))?;
    let root = document.root_element();
    let namespace = root.tag_name().namespace().unwrap_or_default();
    let local_name = root.tag_name().name();
    if namespace != PREDEFINED_NS
        || local_name != "PredefinedData"
        || platform_xml_root_versioning(namespace, local_name)
            != Some(PlatformXmlRootVersioning::ExactRootVersion)
        || root.attribute((XSI_NS, "type")) != Some(root_type(kind)?)
        || root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format)
    {
        return Err(format!(
            "Predefined.xml does not match the exact {}/{} profile for {}",
            ACTIVE_FORMAT_PROFILE.platform_line,
            ACTIVE_FORMAT_PROFILE.export_format,
            kind.as_str()
        ));
    }
    for node in root.descendants().filter(|node| node.is_element()) {
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
        let Some(raw_id) = item.attribute("id") else {
            return Err("predefined Item requires id".to_string());
        };
        let id = uuid::Uuid::parse_str(raw_id)
            .map_err(|_| "predefined Item id is not a UUID".to_string())?
            .to_string();
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

fn validate_code_value(code: Node<'_, '_>, code_type: PredefinedCodeType) -> Result<(), String> {
    let value = code.text().unwrap_or_default();
    if code_type == PredefinedCodeType::String {
        return if code.attribute((XSI_NS, "type")).is_none() {
            Ok(())
        } else {
            Err("predefined string Code must not declare xsi:type".to_string())
        };
    }
    if value.is_empty() {
        return Ok(());
    }
    if metadata_decimal_shape(value).is_none() {
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

fn expected_predefined_child_name(kind: MetadataKind, parent: &str, child: &str) -> bool {
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

fn collect_items(
    source: &str,
    item: Node<'_, '_>,
    kind: MetadataKind,
    parent_id: Option<String>,
    output: &mut Vec<MetaPredefinedItemData>,
) -> Result<(), String> {
    let data = parse_item(source, item, kind, parent_id)?;
    let id = data.id.clone();
    output.push(data);
    if let Some(children) = child(item, "ChildItems") {
        for nested in children
            .children()
            .filter(|node| is_predefined_element(*node, "Item"))
        {
            collect_items(source, nested, kind, Some(id.clone()), output)?;
        }
    }
    Ok(())
}

fn parse_item(
    source: &str,
    item: Node<'_, '_>,
    kind: MetadataKind,
    parent_id: Option<String>,
) -> Result<MetaPredefinedItemData, String> {
    let id = uuid::Uuid::parse_str(
        item.attribute("id")
            .ok_or_else(|| "predefined Item requires id".to_string())?,
    )
    .map_err(|_| "predefined Item id is not a UUID".to_string())?
    .to_string();
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

fn parse_flags(container: Option<Node<'_, '_>>) -> Result<BTreeMap<String, bool>, String> {
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
        let value = match flag.text().map(str::trim) {
            Some("true") => true,
            Some("false") => false,
            _ => return Err("predefined Flag value is not boolean".to_string()),
        };
        if flags.insert(name.to_string(), value).is_some() {
            return Err("predefined Flag ref is duplicated".to_string());
        }
    }
    Ok(flags)
}

fn parse_ext_dimensions(
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

fn apply_operation(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    operation: &MetaEditOperation,
    owner: &MetadataAddress,
    operation_index: usize,
) -> Result<(), MetaFailure> {
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
                let range = removal_range(text, location.range);
                text.replace_range(range, "");
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_requested_code(
    code: Option<&str>,
    code_type: PredefinedCodeType,
    owner: &MetadataAddress,
    operation_index: usize,
    element_index: usize,
) -> Result<(), MetaFailure> {
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

fn add_item(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    element: &MetaPredefinedItemAdd,
    context: PredefinedOperationContext<'_>,
    element_index: usize,
) -> Result<(), MetaFailure> {
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
    expand_self_closing_root(text, eol);
    let (root_qname, root_context) = {
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
            .map(|qname| (qname, FragmentXmlContext::for_node(root)))?
    };
    let closing_tag = format!("</{root_qname}>");
    let close = text.rfind(&closing_tag).ok_or_else(|| {
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

fn update_item(
    text: &mut String,
    eol: &str,
    kind: MetadataKind,
    code_type: PredefinedCodeType,
    element: &MetaPredefinedItemUpdate,
    context: PredefinedOperationContext<'_>,
    element_index: usize,
) -> Result<(), MetaFailure> {
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

fn patch_fields(
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
        let render_context = fragment_direct_child_location(item, "Type", None, fragment_context)?
            .map_or_else(|| fragment_context.clone(), |location| location.context);
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

fn render_item(
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
        format!("<Item id=\"{}\">", escape_xml(&element.id)),
        format!("\t<Name>{}</Name>", escape_xml(&element.name)),
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

fn add_projection(element: &MetaPredefinedItemAdd, kind: MetadataKind) -> MetaPredefinedItemData {
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
struct ItemLocation {
    range: Range<usize>,
    parent_id: Option<String>,
    fragment_context: FragmentXmlContext,
}

fn find_item(text: &str, requested_id: &str) -> Result<Option<ItemLocation>, String> {
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

fn selected_effect(
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

fn set_simple_child(
    item: &mut String,
    tag: &str,
    value: &str,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    let rendered = if value.is_empty() {
        format!("<{tag}/>")
    } else {
        format!("<{tag}>{}</{tag}>", escape_xml(value))
    };
    set_complex_child(item, tag, &rendered, eol, fragment_context)
}

fn set_code_child(
    item: &mut String,
    value: &str,
    code_type: PredefinedCodeType,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<(), String> {
    let render_context = fragment_direct_child_location(item, "Code", None, fragment_context)?
        .map_or_else(|| fragment_context.clone(), |location| location.context);
    let rendered = render_code(value, code_type, &render_context)?;
    set_complex_child(item, "Code", &rendered, eol, fragment_context)
}

fn render_code(
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
            escape_xml(value)
        ));
    }
    Ok(format!("<Code>{}</Code>", escape_xml(value)))
}

fn opening_tag_end(fragment: &str) -> Result<usize, String> {
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

fn opening_tag_attributes(fragment: &str) -> Result<Vec<(String, String)>, String> {
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

fn insert_opening_attributes(fragment: &mut String, attributes: &[String]) -> Result<(), String> {
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

fn preserve_element_attributes(
    existing: &str,
    rendered: &str,
    fragment_context: &FragmentXmlContext,
) -> Result<String, String> {
    let existing_opening = opening_tag_attributes(existing)?;
    let rendered_opening = opening_tag_attributes(rendered)?;
    let rendered_names = rendered_opening
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<HashSet<_>>();
    let preserved_namespaces = existing_opening
        .iter()
        .filter(|(name, _)| name == "xmlns" || name.starts_with("xmlns:"))
        .filter(|(name, _)| !rendered_names.contains(name.as_str()))
        .map(|(_, lexeme)| lexeme.clone())
        .collect::<Vec<_>>();

    let mut merged = rendered.to_string();
    insert_opening_attributes(&mut merged, &preserved_namespaces)?;

    let wrapped_existing = format!("{}{existing}</Root>", fragment_context.wrapper_start);
    let existing_document = Document::parse(&wrapped_existing)
        .map_err(|error| format!("predefined XML fragment: {error}"))?;
    let existing_node = existing_document
        .root_element()
        .children()
        .find(|node| node.is_element())
        .ok_or_else(|| "predefined XML fragment has no element".to_string())?;
    let wrapped_merged = format!("{}{merged}</Root>", fragment_context.wrapper_start);
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
            !rendered_keys.contains(&(
                attribute.namespace().map(str::to_string),
                attribute.name().to_string(),
            ))
        })
        .map(|attribute| wrapped_existing[attribute.range()].to_string())
        .collect::<Vec<_>>();
    insert_opening_attributes(&mut merged, &preserved_attributes)?;
    Ok(merged)
}

fn set_complex_child(
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
            preserve_element_attributes(&parent[range.clone()], &rendered, &location.context)?;
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

fn merge_flags_container(
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
        let rendered = format!("<Flag ref=\"{}\">{value}</Flag>", escape_xml(name));
        if let Some(flag_location) = fragment_direct_child_location(
            &container,
            "Flag",
            Some(("ref", name.as_str())),
            &container_context,
        )? {
            let flag_range = flag_location.range;
            let existing_qname = lexical_element_name(&container[flag_range.clone()])?;
            let rendered = qualify_generated_predefined_elements(
                &rendered,
                lexical_element_prefix(existing_qname),
            );
            let rendered = preserve_element_attributes(
                &container[flag_range.clone()],
                &rendered,
                &flag_location.context,
            )?;
            if container[flag_range.clone()] != rendered {
                container.replace_range(flag_range, &rendered);
            }
        } else {
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

fn merge_ext_dimensions(
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

fn render_flags(tag: &str, flags: Option<&BTreeMap<String, bool>>, indent: &str) -> Vec<String> {
    let Some(flags) = flags.filter(|flags| !flags.is_empty()) else {
        return vec![format!("{indent}<{tag}/>")];
    };
    let mut lines = vec![format!("{indent}<{tag}>")];
    for (name, value) in flags {
        lines.push(format!(
            "{indent}\t<Flag ref=\"{}\">{value}</Flag>",
            escape_xml(name)
        ));
    }
    lines.push(format!("{indent}</{tag}>"));
    lines
}

fn render_ext_dimensions(
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

fn render_ext_dimension(value: &MetaPredefinedExtDimensionType, indent: &str, eol: &str) -> String {
    render_ext_dimension_lines(value, indent).join(eol)
}

fn render_ext_dimension_lines(value: &MetaPredefinedExtDimensionType, indent: &str) -> Vec<String> {
    let mut lines = vec![format!(
        "{indent}<ExtDimensionType name=\"{}\">",
        escape_xml(&value.name)
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

fn render_type(
    metadata_type: &MetadataType,
    indent: &str,
    eol: &str,
    fragment_context: &FragmentXmlContext,
) -> String {
    let mut lines = Vec::new();
    emit_meta_typed_value_type(&mut lines, indent, metadata_type);
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

fn parse_item_type(source: &str, node: Node<'_, '_>) -> Result<MetadataType, String> {
    let node_range = node.range();
    let mut fragment = source[node_range.clone()].to_string();
    let opening_end = fragment
        .find('>')
        .ok_or_else(|| "predefined Type opening tag is unavailable".to_string())?;
    let opening = &fragment[..opening_end];
    let declarations = node
        .namespaces()
        .filter_map(|namespace| {
            let prefix = namespace.name()?;
            (!opening.contains(&format!("xmlns:{prefix}=")))
                .then(|| format!(" xmlns:{prefix}=\"{}\"", escape_xml(namespace.uri())))
        })
        .collect::<String>();
    fragment.insert_str(opening_end, &declarations);

    let declaration_length = declarations.len();
    let mut edits = Vec::new();
    for value_node in node.descendants().filter(|candidate| {
        candidate.is_element()
            && candidate.tag_name().namespace() == Some(V8_NS)
            && matches!(candidate.tag_name().name(), "Type" | "TypeSet")
    }) {
        let Some(text_node) = value_node.children().find(|child| child.is_text()) else {
            continue;
        };
        let raw = text_node.text().unwrap_or_default().trim();
        let (prefix, local) = raw.split_once(':').ok_or_else(|| {
            "predefined Type value must be a namespace-qualified QName".to_string()
        })?;
        if local.contains(':') || prefix.is_empty() || local.is_empty() {
            return Err("predefined Type value is not a valid QName".to_string());
        }
        let namespace = value_node
            .lookup_namespace_uri(Some(prefix))
            .ok_or_else(|| format!("predefined Type prefix `{prefix}` is not declared"))?;
        let canonical_prefix = match namespace {
            CFG_NS => "cfg",
            XS_NS => "xs",
            V8_NS => "v8",
            _ => {
                return Err(format!(
                    "predefined Type QName namespace `{namespace}` is unsupported"
                ))
            }
        };
        let range = text_node.range();
        let mut start = range.start - node_range.start;
        let mut end = range.end - node_range.start;
        if start >= opening_end {
            start += declaration_length;
            end += declaration_length;
        }
        edits.push((start..end, format!("{canonical_prefix}:{local}")));
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, replacement) in edits {
        fragment.replace_range(range, &replacement);
    }
    super::edit::parse_typed_metadata_type(&fragment).map_err(|diagnostic| diagnostic.message)
}

fn fragment_root_name(
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

fn lexical_element_name(source: &str) -> Result<&str, String> {
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

fn lexical_element_prefix(qname: &str) -> &str {
    qname
        .rsplit_once(':')
        .map_or("", |(prefix, _)| &qname[..prefix.len() + 1])
}

fn qualify_generated_predefined_elements(fragment: &str, prefix: &str) -> String {
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

fn ordered_insertion_anchor(
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

fn predefined_field_rank(parent: &str, tag: &str) -> usize {
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
struct FragmentChildLocation {
    range: Range<usize>,
    context: FragmentXmlContext,
}

fn fragment_direct_child_location(
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
            && attribute.is_none_or(|(name, value)| node.attribute(name) == Some(value))
    }) else {
        return Ok(None);
    };
    let range = node.range();
    let parent_context = fragment_context.with_local_declarations(&wrapped, parent)?;
    Ok(Some(FragmentChildLocation {
        range: range.start - fragment_context.wrapper_start.len()
            ..range.end - fragment_context.wrapper_start.len(),
        context: parent_context.with_local_declarations(&wrapped, node)?,
    }))
}

fn fragment_direct_child_range(
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

fn fragment_direct_child_ranges(
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

fn remove_direct_children(
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

fn expand_self_closing_root(text: &mut String, eol: &str) {
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

fn expand_self_closing(text: &mut String, tag: &str, eol: &str, indent: &str) {
    if !text.trim_end().ends_with("/>") {
        return;
    }
    if let Some(slash) = text.rfind("/>") {
        text.replace_range(slash..slash + 2, &format!(">{eol}{indent}</{tag}>"));
    }
}

fn child_indent(fragment: &str) -> String {
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

fn removal_range(text: &str, range: Range<usize>) -> Range<usize> {
    let line_start = text[..range.start]
        .rfind(['\n', '\r'])
        .map_or(0, |index| index + 1);
    if !text[line_start..range.start].trim().is_empty() {
        return range;
    }
    let mut end = range.end;
    while end < text.len() && matches!(text.as_bytes()[end], b' ' | b'\t') {
        end += 1;
    }
    if text[end..].starts_with("\r\n") {
        end += 2;
    } else if end < text.len() && matches!(text.as_bytes()[end], b'\r' | b'\n') {
        end += 1;
    }
    line_start..end
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|child| is_predefined_element(*child, name))
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, name)
        .and_then(|child| child.text())
        .map(str::to_string)
}

fn is_predefined_element(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(PREDEFINED_NS)
        && node.tag_name().name() == name
}

fn root_type(kind: MetadataKind) -> Result<&'static str, String> {
    match kind {
        MetadataKind::Catalog => Ok("CatalogPredefinedItems"),
        MetadataKind::ChartOfAccounts => Ok("ChartOfAccountsPredefinedItems"),
        MetadataKind::ChartOfCharacteristicTypes => Ok("PlanOfCharacteristicKindPredefinedItems"),
        MetadataKind::ChartOfCalculationTypes => Ok("CalculationTypePredefinedItems"),
        _ => Err(format!("{} does not own predefined data", kind.as_str())),
    }
}

fn empty_document(kind: MetadataKind) -> String {
    let root_type = root_type(kind).expect("domain capability validated predefined owner");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<PredefinedData xmlns=\"{PREDEFINED_NS}\" xmlns:v8=\"{V8_NS}\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"{XSI_NS}\" xsi:type=\"{root_type}\" version=\"{}\">\n</PredefinedData>\n",
        ACTIVE_FORMAT_PROFILE.export_format
    )
}

fn source_eol(text: &str) -> &'static str {
    if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    }
}

fn simple_line(tag: &str, value: &str, indent: &str) -> String {
    if value.is_empty() {
        format!("{indent}<{tag}/>")
    } else {
        format!("{indent}<{tag}>{}</{tag}>", escape_xml(value))
    }
}

fn is_predefined_operation(operation: &MetaEditOperation) -> bool {
    matches!(
        operation,
        MetaEditOperation::AddPredefinedItems { .. }
            | MetaEditOperation::UpdatePredefinedItems { .. }
            | MetaEditOperation::RemovePredefinedItems { .. }
    )
}

fn operation_name(operation: &MetaEditOperation) -> &'static str {
    match operation {
        MetaEditOperation::AddPredefinedItems { .. } => "add",
        MetaEditOperation::UpdatePredefinedItems { .. } => "update",
        MetaEditOperation::RemovePredefinedItems { .. } => "remove",
        _ => unreachable!("called only for predefined operation"),
    }
}

fn failure(
    owner: &MetadataAddress,
    code: MetaDiagnosticCode,
    message: impl Into<String>,
    operation_index: Option<usize>,
    field: Option<String>,
) -> MetaFailure {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::MetadataTypeVariant;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_root(label: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("unica-predefined-{label}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn owner(kind: &str) -> MetadataAddress {
        MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{kind}.Items"),
        )
        .unwrap()
    }

    fn descriptor_image(kind: MetadataKind, code_type: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"{MD_CLASSES_NS}\" version=\"2.20\"><{kind}><Properties><Name>Items</Name><CodeType>{code_type}</CodeType></Properties></{kind}></MetaDataObject>",
            kind = kind.as_str()
        )
    }

    #[test]
    fn info_is_flat_document_order_with_explicit_root_parent() {
        let bytes = br#"<?xml version="1.0"?><PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Parent</Name><ChildItems><Item id="b7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Child</Name></Item></ChildItems></Item></PredefinedData>"#;
        let data = read_predefined_items(bytes, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].parent_id, None);
        assert_eq!(
            data.items[1].parent_id.as_deref(),
            Some("a7d2e6fc-3824-4b56-b4be-ae6be4944c0e")
        );
        let json = serde_json::to_value(&data).unwrap();
        assert!(json["items"][0].get("parentId").is_some());
        assert!(json["items"][0]["parentId"].is_null());
    }

    #[test]
    fn account_type_wire_value_round_trips_in_platform_case() {
        assert_eq!(
            serde_json::to_value(MetaPredefinedAccountType::ActivePassive).unwrap(),
            Value::String("ActivePassive".to_string())
        );
        assert_eq!(
            MetaPredefinedAccountType::parse("ActivePassive", "accountType").unwrap(),
            MetaPredefinedAccountType::ActivePassive
        );
    }

    #[test]
    fn predefined_root_uses_the_shared_platform_root_registry() {
        assert_eq!(
            platform_xml_root_versioning(PREDEFINED_NS, "PredefinedData"),
            Some(PlatformXmlRootVersioning::ExactRootVersion)
        );
    }

    #[test]
    fn other_exact_registered_roots_are_not_predefined_data() {
        for xml in [
            r#"<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"/>"#,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"/>"#,
        ] {
            assert!(validate_predefined_text(MetadataKind::Catalog, xml).is_err());
        }
    }

    #[test]
    fn type_qname_is_resolved_by_namespace_not_by_prefix_spelling() {
        let bytes = br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:ent="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="PlanOfCharacteristicKindPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type><v8:Type>ent:CatalogRef.Items</v8:Type></Type><IsFolder>false</IsFolder></Item></PredefinedData>"#;
        let data =
            read_predefined_items(bytes, MetadataKind::ChartOfCharacteristicTypes, 10).unwrap();
        assert!(matches!(
            data.items[0].r#type.as_ref().unwrap().variants.as_slice(),
            [MetadataTypeVariant::Reference { metadata_path }]
                if metadata_path.as_str() == "Catalog.Items"
        ));
    }

    #[test]
    fn type_update_preserves_the_item_child_indentation() {
        let mut item = concat!(
            "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n",
            "\t<Name>Kind</Name>\n",
            "\t<Type/>\n",
            "\t<IsFolder>false</IsFolder>\n",
            "</Item>"
        )
        .to_string();
        let fields = MetaPredefinedFields {
            r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(
            item.contains(&format!(
                "\n\t<Type xmlns:v8=\"{V8_NS}\" xmlns:xs=\"{XS_NS}\">\n\t\t<v8:Type>xs:boolean</v8:Type>\n\t</Type>\n"
            )),
            "{item}"
        );
        assert!(!item.lines().any(|line| line.starts_with("<v8:")), "{item}");
    }

    #[test]
    fn type_writer_reuses_aliases_or_declares_every_required_namespace_locally() {
        let aliases =
            FragmentXmlContext::from_namespaces([("core", V8_NS), ("schema", XS_NS)].into_iter());
        let metadata_type = MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap();
        let aliased = render_type(&metadata_type, "", "\n", &aliases);
        assert_eq!(
            aliased,
            "<Type>\n\t<core:Type>schema:boolean</core:Type>\n</Type>"
        );

        let local_type = MetadataType::new(vec![
            MetadataTypeVariant::Boolean,
            MetadataTypeVariant::Reference {
                metadata_path: owner("Catalog"),
            },
        ])
        .unwrap();
        let local = render_type(
            &local_type,
            "\t",
            "\n",
            &FragmentXmlContext::platform_default(),
        );
        for declaration in [
            format!(r#"xmlns:v8="{V8_NS}""#),
            format!(r#"xmlns:xs="{XS_NS}""#),
            format!(r#"xmlns:cfg="{CFG_NS}""#),
        ] {
            assert!(
                local.contains(&declaration),
                "missing {declaration}: {local}"
            );
        }
        let xml = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="PlanOfCharacteristicKindPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name>{local}<IsFolder>false</IsFolder></Item></PredefinedData>"#
        );
        validate_predefined_text(MetadataKind::ChartOfCharacteristicTypes, &xml).unwrap();
        let data =
            read_predefined_items(xml.as_bytes(), MetadataKind::ChartOfCharacteristicTypes, 10)
                .unwrap();
        assert_eq!(data.items[0].r#type.as_ref().unwrap(), &local_type);
    }

    #[test]
    fn type_update_avoids_a_local_prefix_collision_and_preserves_attribute_identity() {
        let mut item = r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Kind</Name><Type xmlns:v8="urn:future" v8:stamp="keep"/><IsFolder>false</IsFolder></Item>"#.to_string();
        let fields = MetaPredefinedFields {
            r#type: Some(MetadataType::new(vec![MetadataTypeVariant::Boolean]).unwrap()),
            ..MetaPredefinedFields::default()
        };

        patch_fields(
            &mut item,
            MetadataKind::ChartOfCharacteristicTypes,
            PredefinedCodeType::String,
            &fields,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains(&format!(r#"xmlns:v8_1="{V8_NS}""#)), "{item}");
        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        let document = Document::parse(&wrapped).unwrap();
        let type_node = document
            .descendants()
            .find(|node| is_predefined_element(*node, "Type"))
            .unwrap();
        assert_eq!(type_node.attribute(("urn:future", "stamp")), Some("keep"));
        assert_eq!(type_node.attribute((V8_NS, "stamp")), None);
        assert!(type_node
            .children()
            .any(|node| node.tag_name().namespace() == Some(V8_NS)));
    }

    #[test]
    fn ext_dimension_update_inherits_an_alias_declared_on_its_container() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><ExtDimensionTypes xmlns:p="{PREDEFINED_NS}"><p:ExtDimensionType name="Kinds"><p:Turnover>false</p:Turnover><p:AccountingFlags><p:Flag ref="Debit">false</p:Flag></p:AccountingFlags></p:ExtDimensionType></ExtDimensionTypes></Item>"#
        );
        let requested = [MetaPredefinedExtDimensionType {
            name: "Kinds".to_string(),
            turnover: Some(true),
            accounting_flags: Some(BTreeMap::from([("Debit".to_string(), true)])),
        }];

        merge_ext_dimensions(
            &mut item,
            &requested,
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains("<p:Turnover>true</p:Turnover>"), "{item}");
        assert!(
            item.contains("<p:Flag ref=\"Debit\">true</p:Flag>"),
            "{item}"
        );
        let wrapped = format!(r#"<Root xmlns="{PREDEFINED_NS}">{item}</Root>"#);
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn replacement_preserves_local_namespace_declarations_and_unknown_attributes() {
        let mut item = format!(
            r#"<Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Description xmlns:p="{PREDEFINED_NS}" xmlns:future="urn:future" future:stamp="keep">old</p:Description></Item>"#
        );

        set_simple_child(
            &mut item,
            "Description",
            "new",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();

        assert!(item.contains(&format!(
            r#"<p:Description xmlns:p="{PREDEFINED_NS}" xmlns:future="urn:future" future:stamp="keep">new</p:Description>"#
        )), "{item}");
        let wrapped = format!("<Root xmlns=\"{PREDEFINED_NS}\">{item}</Root>");
        Document::parse(&wrapped).unwrap();
    }

    #[test]
    fn explicit_empty_structural_fields_clear_typed_children_and_preserve_unknown_xml() {
        let context = FragmentXmlContext::platform_default();
        let mut item = concat!(
            "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n",
            "\t<AccountingFlags xmlns:future=\"urn:future\" future:stamp=\"keep\">\n",
            "\t\t<Flag ref=\"Debit\">true</Flag>\n",
            "\t\t<future:Keep/>\n",
            "\t\t<Flag ref=\"Credit\">false</Flag>\n",
            "\t</AccountingFlags>\n",
            "\t<ExtDimensionTypes future:stamp=\"keep\" xmlns:future=\"urn:future\">\n",
            "\t\t<ExtDimensionType name=\"Kinds\"><Turnover>true</Turnover></ExtDimensionType>\n",
            "\t\t<future:Keep/>\n",
            "\t</ExtDimensionTypes>\n",
            "</Item>"
        )
        .to_string();

        merge_flags_container(
            &mut item,
            "AccountingFlags",
            &BTreeMap::new(),
            "\n",
            &context,
        )
        .unwrap();
        merge_ext_dimensions(&mut item, &[], "\n", &context).unwrap();

        assert!(!item.contains("<Flag "), "{item}");
        assert!(!item.contains("<ExtDimensionType "), "{item}");
        assert_eq!(item.matches("<future:Keep/>").count(), 2, "{item}");
        assert_eq!(item.matches("future:stamp=\"keep\"").count(), 2, "{item}");
    }

    #[test]
    fn planner_clears_explicit_empty_structural_fields_and_repeats_as_a_noop() {
        let root = temp_root("clear-structural-fields");
        let descriptor = root.join("ChartsOfAccounts/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:future="urn:future" xmlns:xsi="{XSI_NS}" xsi:type="ChartOfAccountsPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><AccountingFlags future:stamp="keep"><Flag ref="Debit">true</Flag><future:Keep/></AccountingFlags><ExtDimensionTypes future:stamp="keep"><ExtDimensionType name="Kinds"><Turnover>true</Turnover></ExtDimensionType><future:Keep/></ExtDimensionTypes></Item></PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    accounting_flags: Some(BTreeMap::new()),
                    ext_dimension_types: Some(Vec::new()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            std::slice::from_ref(&operation),
        )
        .unwrap();
        assert_eq!(first.resources.file_mutations.len(), 1);
        assert_ne!(first.effects[0].before, first.effects[0].after);
        let after = first.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let after_text = std::str::from_utf8(after).unwrap();
        assert!(!after_text.contains("<Flag "), "{after_text}");
        assert!(!after_text.contains("<ExtDimensionType "), "{after_text}");
        assert_eq!(after_text.matches("<future:Keep/>").count(), 2);
        assert_eq!(after_text.matches("future:stamp=\"keep\"").count(), 2);

        fs::write(&path, after).unwrap();
        let repeated = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            &[operation],
        )
        .unwrap();
        assert!(repeated.resources.file_mutations.is_empty());
        assert_eq!(repeated.effects[0].before, repeated.effects[0].after);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remove_effect_reports_the_entire_subtree_in_document_order() {
        let parent = "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let child = "b7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let sibling = "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e";
        let xml = format!(
            r#"<PredefinedData xmlns="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="{parent}"><Name>Parent</Name><ChildItems><Item id="{child}"><Name>Child</Name></Item></ChildItems></Item><Item id="{sibling}"><Name>Sibling</Name></Item></PredefinedData>"#
        );
        let operation =
            MetaEditOperation::remove_predefined_items(vec![parent.to_string()]).unwrap();

        let before = selected_effect(&xml, MetadataKind::Catalog, &operation, false)
            .unwrap()
            .unwrap();
        let items = before.as_array().unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["id"], parent);
        assert_eq!(items[0]["parentId"], Value::Null);
        assert_eq!(items[1]["id"], child);
        assert_eq!(items[1]["parentId"], parent);
        assert!(!items.iter().any(|item| item["id"] == sibling));
    }

    #[test]
    fn duplicate_direct_supported_child_is_rejected() {
        let xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>First</Name><Name>Second</Name></Item></PredefinedData>"#;
        assert!(validate_predefined_text(MetadataKind::Catalog, xml).is_err());
    }

    #[test]
    fn missing_supported_field_is_inserted_before_child_items() {
        let mut item = "<Item id=\"a7d2e6fc-3824-4b56-b4be-ae6be4944c0e\">\n\t<Name>Parent</Name>\n\t<ChildItems/>\n</Item>".to_string();
        set_simple_child(
            &mut item,
            "Code",
            "01",
            "\n",
            &FragmentXmlContext::platform_default(),
        )
        .unwrap();
        assert!(item.find("<Code>01</Code>").unwrap() < item.find("<ChildItems/>").unwrap());
    }

    #[test]
    fn planner_repairs_a_self_closing_item_when_update_supplies_its_name() {
        let root = temp_root("self-closing-item");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"/></PredefinedData>"#,
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: Some("Main".to_string()),
                fields: MetaPredefinedFields {
                    description: Some("Repaired".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = planned.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let data = read_predefined_items(after, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(data.items[0].name, "Main");
        assert_eq!(data.items[0].description, "Repaired");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_preserves_unknown_nodes_using_a_namespace_declared_on_the_document_root() {
        let root = temp_root("inherited-unknown-namespace");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let unknown = r#"<future:Extension future:mode="keep"><future:Nested/></future:Extension>"#;
        fs::write(
            &path,
            format!(
                r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:future="urn:unica:future" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e" future:stamp="keep"><Name>Main</Name>{unknown}</Item></PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    description: Some("Changed".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();

        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(after.contains(r#"future:stamp="keep""#));
        assert!(after.contains(unknown));
        assert!(after.contains("<Description>Changed</Description>"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn add_equivalence_does_not_ignore_an_existing_child_subtree() {
        let mut xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code/><Description/><IsFolder>false</IsFolder><ChildItems><Item id="b7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Child</Name></Item></ChildItems></Item></PredefinedData>"#.to_string();
        let owner = MetadataAddress::parse(
            crate::domain::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Catalog.Items",
        )
        .unwrap();
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };
        let failure = add_item(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            PredefinedCodeType::String,
            &element,
            PredefinedOperationContext {
                owner: &owner,
                operation_index: 0,
            },
            0,
        )
        .unwrap_err();
        assert_eq!(
            failure.diagnostics[0].code,
            MetaDiagnosticCode::AlreadyExists
        );
    }

    #[test]
    fn add_equivalence_accepts_an_empty_child_items_container() {
        let mut xml = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code/><Description/><IsFolder>false</IsFolder><ChildItems/></Item></PredefinedData>"#.to_string();
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };

        add_item(
            &mut xml,
            "\n",
            MetadataKind::Catalog,
            PredefinedCodeType::String,
            &element,
            PredefinedOperationContext {
                owner: &owner("Catalog"),
                operation_index: 0,
            },
            0,
        )
        .unwrap();

        assert!(xml.contains("<ChildItems/>"));
    }

    #[test]
    fn planner_creates_bom_file_and_second_equivalent_add_is_noop() {
        let root = temp_root("create-noop");
        let descriptor = root.join("Catalogs/Items.xml");
        let element = MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        };
        let operation = MetaEditOperation::add_predefined_items(vec![element]).unwrap();
        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            std::slice::from_ref(&operation),
        )
        .unwrap();
        assert_eq!(first.resources.file_mutations.len(), 1);
        let bytes = first.resources.file_mutations[0]
            .post_image
            .as_ref()
            .unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        let parsed = read_predefined_items(bytes, MetadataKind::Catalog, 10).unwrap();
        assert_eq!(parsed.items[0].name, "Main");

        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let second = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        assert!(second.resources.file_mutations.is_empty());
        assert_eq!(second.resources.exact_file_guards.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn planner_expands_self_closing_root_and_preserves_crlf_and_bom() {
        let root = temp_root("crlf");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let before = concat!(
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
            "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" ",
            "xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" ",
            "xsi:type=\"CatalogPredefinedItems\" version=\"2.20\"/>\r\n"
        );
        fs::write(&path, before.as_bytes()).unwrap();
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = planned.resources.file_mutations[0]
            .post_image
            .as_ref()
            .unwrap();
        assert!(after.starts_with(b"\xef\xbb\xbf"));
        let text = std::str::from_utf8(&after[3..]).unwrap();
        assert!(text.contains("\r\n\t<Item"));
        assert!(!text.replace("\r\n", "").contains('\n'));
        assert!(text.contains("</PredefinedData>"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn numeric_code_uses_effective_descriptor_post_image_for_add_and_update() {
        let root = temp_root("numeric-code");
        let descriptor = root.join("Catalogs/Items.xml");
        let add = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields {
                code: Some("643".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let first = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[add],
        )
        .unwrap();
        let first_bytes = first.resources.file_mutations[0]
            .post_image
            .as_deref()
            .unwrap();
        let first_text = std::str::from_utf8(first_bytes)
            .unwrap()
            .trim_start_matches('\u{feff}');
        assert!(first_text.contains(r#"<Code xsi:type="xs:decimal">643</Code>"#));
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            PredefinedCodeType::Number,
            first_text,
        )
        .unwrap();

        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, first_bytes).unwrap();
        let update = MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: None,
            fields: MetaPredefinedFields {
                code: Some("840".to_string()),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let second = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[update],
        )
        .unwrap();
        let second_text = std::str::from_utf8(
            second.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(second_text.contains(r#"<Code xsi:type="xs:decimal">840</Code>"#));
        assert_eq!(second_text.matches("xsi:type=\"xs:decimal\"").count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn numeric_code_rejects_non_decimal_public_values() {
        for (index, code) in ["ABC", "1e3", " 1", "1 ", "1."].into_iter().enumerate() {
            let root = temp_root(&format!("invalid-numeric-code-{index}"));
            let descriptor = root.join("Catalogs/Items.xml");
            let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
                id: format!("a7d2e6fc-3824-4b56-b4be-{:012x}", index + 1),
                name: "Main".to_string(),
                fields: MetaPredefinedFields {
                    code: Some(code.to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
            let failure = plan_predefined_resource(
                &root,
                &descriptor,
                &owner("Catalog"),
                MetadataKind::Catalog,
                &descriptor_image(MetadataKind::Catalog, "Number"),
                &[operation],
            )
            .err()
            .expect("invalid numeric code must be rejected");
            assert_eq!(
                failure.diagnostics[0].code,
                MetaDiagnosticCode::InvalidArguments,
                "{code}"
            );
            assert_eq!(
                failure.diagnostics[0].field.as_deref(),
                Some("operations[0].elements[0].code")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn numeric_code_validation_resolves_the_xs_decimal_qname_by_namespace() {
        let alias = r#"<PredefinedData xmlns="http://v8.1c.ru/8.3/xcf/predef" xmlns:d="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="CatalogPredefinedItems" version="2.20"><Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><Name>Main</Name><Code xsi:type="d:decimal">643</Code></Item></PredefinedData>"#;
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            PredefinedCodeType::Number,
            alias,
        )
        .unwrap();
        let wrong = alias.replace(
            "http://www.w3.org/2001/XMLSchema\" xmlns:xsi",
            "urn:not-xml-schema\" xmlns:xsi",
        );
        assert!(validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            PredefinedCodeType::Number,
            &wrong,
        )
        .is_err());
    }

    #[test]
    fn numeric_code_writer_reuses_in_scope_namespace_aliases() {
        let root = temp_root("numeric-code-aliases");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:i="{XSI_NS}" xmlns:d="{XS_NS}" i:type="CatalogPredefinedItems" version="2.20"><p:Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Name>Main</p:Name><p:Code/></p:Item></p:PredefinedData>"#
            ),
        )
        .unwrap();
        let operation =
            MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                name: None,
                fields: MetaPredefinedFields {
                    code: Some("643".to_string()),
                    ..MetaPredefinedFields::default()
                },
            }])
            .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "Number"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert!(
            after.contains(r#"<p:Code i:type="d:decimal">643</p:Code>"#),
            "{after}"
        );
        assert!(!after.contains("xmlns:xs="), "{after}");
        assert!(!after.contains("xmlns:xsi="), "{after}");
        validate_predefined_text_for_code_type(
            MetadataKind::Catalog,
            PredefinedCodeType::Number,
            after,
        )
        .unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn prefixed_predefined_update_preserves_qnames_and_repairs_self_closing_code() {
        for (index, code) in ["<p:Code>1</p:Code>", "<p:Code/>"].into_iter().enumerate() {
            let root = temp_root(&format!("prefixed-update-{index}"));
            let descriptor = root.join("Catalogs/Items.xml");
            let path = descriptor.with_extension("").join("Ext/Predefined.xml");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(
                &path,
                format!(
                    r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"><p:Item id="a7d2e6fc-3824-4b56-b4be-ae6be4944c0e"><p:Name>Main</p:Name>{code}</p:Item></p:PredefinedData>"#
                ),
            )
            .unwrap();
            let operation =
                MetaEditOperation::update_predefined_items(vec![MetaPredefinedItemUpdate {
                    id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
                    name: None,
                    fields: MetaPredefinedFields {
                        code: Some("2".to_string()),
                        description: Some("Changed".to_string()),
                        ..MetaPredefinedFields::default()
                    },
                }])
                .unwrap();
            let planned = plan_predefined_resource(
                &root,
                &descriptor,
                &owner("Catalog"),
                MetadataKind::Catalog,
                &descriptor_image(MetadataKind::Catalog, "String"),
                &[operation],
            )
            .unwrap();
            let after = std::str::from_utf8(
                planned.resources.file_mutations[0]
                    .post_image
                    .as_deref()
                    .unwrap(),
            )
            .unwrap();
            assert!(after.contains("<p:Code>2</p:Code>"), "{after}");
            assert!(
                after.contains("<p:Description>Changed</p:Description>"),
                "{after}"
            );
            assert!(!after.contains("<Code>"), "{after}");
            let data = read_predefined_items(after.as_bytes(), MetadataKind::Catalog, 10).unwrap();
            assert_eq!(data.items[0].code, "2");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn prefixed_self_closing_root_add_uses_the_root_qname_for_the_whole_item() {
        let root = temp_root("prefixed-root-add");
        let descriptor = root.join("Catalogs/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                r#"<p:PredefinedData xmlns:p="{PREDEFINED_NS}" xmlns:xsi="{XSI_NS}" xsi:type="CatalogPredefinedItems" version="2.20"/>"#
            ),
        )
        .unwrap();
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "a7d2e6fc-3824-4b56-b4be-ae6be4944c0e".to_string(),
            name: "Main".to_string(),
            fields: MetaPredefinedFields::default(),
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("Catalog"),
            MetadataKind::Catalog,
            &descriptor_image(MetadataKind::Catalog, "String"),
            &[operation],
        )
        .unwrap();
        let after = std::str::from_utf8(
            planned.resources.file_mutations[0]
                .post_image
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        for expected in [
            "<p:Item ",
            "<p:Name>Main</p:Name>",
            "<p:Code/>",
            "<p:Description/>",
            "<p:IsFolder>false</p:IsFolder>",
            "</p:Item>",
            "</p:PredefinedData>",
        ] {
            assert!(after.contains(expected), "missing {expected}: {after}");
        }
        read_predefined_items(after.as_bytes(), MetadataKind::Catalog, 10).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn captured_platform_fixtures_cover_all_four_owner_shapes() {
        for (kind, bytes) in [
            (
                MetadataKind::Catalog,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/Catalog.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfAccounts,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfAccounts.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfCharacteristicTypes,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfCharacteristicTypes.xml"
                ))
                .as_slice(),
            ),
            (
                MetadataKind::ChartOfCalculationTypes,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfCalculationTypes.xml"
                ))
                .as_slice(),
            ),
        ] {
            let data = read_predefined_items(bytes, kind, 10).unwrap();
            assert_eq!(data.total, 1, "{} fixture", kind.as_str());
            if kind == MetadataKind::ChartOfAccounts {
                assert_eq!(data.items[0].order.as_deref(), Some(" 01.01"));
            }
        }
    }

    #[test]
    fn chart_of_accounts_fixture_is_an_exact_add_noop_including_order_whitespace() {
        let root = temp_root("chart-account-noop");
        let descriptor = root.join("ChartsOfAccounts/Items.xml");
        let path = descriptor.with_extension("").join("Ext/Predefined.xml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let fixture = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/platform_8_3_27/predefined/ChartOfAccounts.xml"
        ));
        fs::write(&path, fixture).unwrap();
        let accounting_flags = BTreeMap::from([
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.Валютный".to_string(),
                false,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.Количественный".to_string(),
                false,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.УчетПоПодразделениям".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.AccountingFlag.НалоговыйУчет".to_string(),
                true,
            ),
        ]);
        let ext_flags = BTreeMap::from([
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Суммовой".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Валютный".to_string(),
                true,
            ),
            (
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Количественный"
                    .to_string(),
                true,
            ),
        ]);
        let operation = MetaEditOperation::add_predefined_items(vec![MetaPredefinedItemAdd {
            id: "83d798ec-929f-4f72-b78b-44e770856be4".to_string(),
            name: "ОСвОрганизации".to_string(),
            fields: MetaPredefinedFields {
                code: Some("01.01".to_string()),
                description: Some("Основные средства в организации".to_string()),
                account_type: Some(MetaPredefinedAccountType::Active),
                off_balance: Some(false),
                order: Some(" 01.01".to_string()),
                accounting_flags: Some(accounting_flags),
                ext_dimension_types: Some(vec![MetaPredefinedExtDimensionType {
                    name: "ChartOfCharacteristicTypes.ВидыСубконтоХозрасчетные.ОсновныеСредства"
                        .to_string(),
                    turnover: Some(false),
                    accounting_flags: Some(ext_flags),
                }]),
                ..MetaPredefinedFields::default()
            },
        }])
        .unwrap();
        let planned = plan_predefined_resource(
            &root,
            &descriptor,
            &owner("ChartOfAccounts"),
            MetadataKind::ChartOfAccounts,
            &descriptor_image(MetadataKind::ChartOfAccounts, "String"),
            &[operation],
        )
        .unwrap();
        assert!(planned.resources.file_mutations.is_empty());
        assert_eq!(planned.resources.exact_file_guards.len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
