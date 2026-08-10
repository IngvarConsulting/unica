use super::super::common::{escape_xml, multilang_text};
use super::super::role::role_info_element;
use crate::domain::metadata::{
    metadata_identifier_is_valid, DateFractions, EventSourceClass, MetaEventSource,
    MetaEventSourceDateFractions, MetaFillValue, MetadataType, MetadataTypeVariant, NumberSign,
    StringLengthMode,
};
use crate::domain::source_target::{
    xml_ncname_is_valid, MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20,
};

pub(super) const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";
const DATA_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
const CURRENT_CONFIG_NS: &str = "http://v8.1c.ru/8.1/data/enterprise/current-config";
const XML_SCHEMA_NS: &str = "http://www.w3.org/2001/XMLSchema";

pub(crate) fn parse_metadata_image(
    bytes: &[u8],
) -> Result<(&str, roxmltree::Document<'_>), String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|error| format!("metadata image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(source)
        .map_err(|error| format!("metadata XML parse failed: {error}"))?;
    Ok((source, document))
}

pub(crate) fn meta_info_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| role_info_element(*child, local_name, None))
}

pub(crate) fn meta_info_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Vec<roxmltree::Node<'a, 'input>> {
    node.children()
        .filter(|child| role_info_element(*child, local_name, None))
        .collect()
}

pub(crate) fn meta_info_child_text(
    node: roxmltree::Node<'_, '_>,
    local_name: &str,
) -> Option<String> {
    meta_info_child(node, local_name).map(meta_info_inner_text)
}

pub(crate) fn meta_info_inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.text().unwrap_or("").to_string()
}

fn single_direct_md_child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    label: &str,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let matches = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    let [child] = matches.as_slice() else {
        return Err(format!(
            "{label} must contain exactly one direct {local_name}, found {}",
            matches.len()
        ));
    };
    if child.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err(format!(
            "{label}/{local_name} must use namespace {{{MD_CLASSES_NS}}}"
        ));
    }
    Ok(*child)
}

pub(super) fn meta_event_subscription_source_node<'a, 'input>(
    document: &'a roxmltree::Document<'input>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("EventSubscription target must be an MDClasses MetaDataObject".to_string());
    }
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [object] = artifacts.as_slice() else {
        return Err(format!(
            "EventSubscription target must contain exactly one direct metadata artifact, found {}",
            artifacts.len()
        ));
    };
    if object.tag_name().namespace() != Some(MD_CLASSES_NS)
        || object.tag_name().name() != "EventSubscription"
    {
        return Err(
            "EventSubscription target must contain a direct MDClasses EventSubscription"
                .to_string(),
        );
    }
    let properties = single_direct_md_child(*object, "Properties", "EventSubscription")?;
    let source = single_direct_md_child(properties, "Source", "EventSubscription Properties")?;
    Ok(source)
}

fn strict_leaf_text(node: roxmltree::Node<'_, '_>, label: &str) -> Result<String, String> {
    if node.attributes().len() != 0 {
        return Err(format!("{label} must not have attributes"));
    }
    let children = node.children().collect::<Vec<_>>();
    let [text] = children.as_slice() else {
        return Err(format!("{label} must contain exactly one text node"));
    };
    if !text.is_text() {
        return Err(format!("{label} must contain text only"));
    }
    let value = text.text().unwrap_or_default();
    if value.trim().is_empty() {
        return Err(format!("{label} must contain non-empty text"));
    }
    if value != value.trim() {
        return Err(format!("{label} must not contain surrounding whitespace"));
    }
    Ok(value.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct EventSourceQName {
    namespace: String,
    local_name: String,
}

fn resolve_event_source_qname(
    node: roxmltree::Node<'_, '_>,
    wire_type: &str,
) -> Result<EventSourceQName, String> {
    let Some((prefix, local_name)) = wire_type.split_once(':') else {
        return Err(format!(
            "EventSubscription Source wire type {wire_type} must be a QName"
        ));
    };
    if local_name.contains(':') || !xml_ncname_is_valid(prefix) || !xml_ncname_is_valid(local_name)
    {
        return Err(format!(
            "EventSubscription Source wire type {wire_type} is not a lexical QName"
        ));
    }
    let Some(namespace) = node.lookup_namespace_uri(Some(prefix)) else {
        return Err(format!(
            "EventSubscription Source wire type {wire_type} must bind {prefix} at the declaring node"
        ));
    };
    if !matches!(namespace, CURRENT_CONFIG_NS | DATA_CORE_NS | XML_SCHEMA_NS) {
        return Err(format!(
            "EventSubscription Source wire type {wire_type} must bind {prefix} to a supported source namespace at the declaring node"
        ));
    }
    Ok(EventSourceQName {
        namespace: namespace.to_string(),
        local_name: local_name.to_string(),
    })
}

fn strict_qualifier_values(
    qualifier: roxmltree::Node<'_, '_>,
    expected_children: &[&str],
) -> Result<Vec<String>, String> {
    if qualifier.attributes().len() != 0 {
        return Err(format!(
            "EventSubscription Source v8:{} must not have attributes",
            qualifier.tag_name().name()
        ));
    }
    let mut actual_names = Vec::new();
    let mut values = Vec::new();
    for child in qualifier.children() {
        if child.is_text() && child.text().is_some_and(|text| text.trim().is_empty()) {
            continue;
        }
        if !child.is_element() || child.tag_name().namespace() != Some(DATA_CORE_NS) {
            return Err(format!(
                "EventSubscription Source v8:{} contains a non-v8 child",
                qualifier.tag_name().name()
            ));
        }
        actual_names.push(child.tag_name().name());
        values.push(strict_leaf_text(
            child,
            &format!(
                "EventSubscription Source v8:{}/v8:{}",
                qualifier.tag_name().name(),
                child.tag_name().name()
            ),
        )?);
    }
    if actual_names != expected_children {
        return Err(format!(
            "EventSubscription Source v8:{} children must be {:?}, got {:?}",
            qualifier.tag_name().name(),
            expected_children,
            actual_names
        ));
    }
    Ok(values)
}

#[derive(Clone, Copy)]
enum EventSourceGeneratedKind {
    Object,
    Manager,
    Reference,
    RecordSet,
    DefinedType,
}

fn event_source_generated_contract(
    prefix: &str,
) -> Option<(&'static str, EventSourceGeneratedKind)> {
    let contract = match prefix {
        "CatalogObject" => ("Catalog", EventSourceGeneratedKind::Object),
        "DocumentObject" => ("Document", EventSourceGeneratedKind::Object),
        "ChartOfAccountsObject" => ("ChartOfAccounts", EventSourceGeneratedKind::Object),
        "ChartOfCharacteristicTypesObject" => (
            "ChartOfCharacteristicTypes",
            EventSourceGeneratedKind::Object,
        ),
        "ChartOfCalculationTypesObject" => {
            ("ChartOfCalculationTypes", EventSourceGeneratedKind::Object)
        }
        "ExchangePlanObject" => ("ExchangePlan", EventSourceGeneratedKind::Object),
        "BusinessProcessObject" => ("BusinessProcess", EventSourceGeneratedKind::Object),
        "TaskObject" => ("Task", EventSourceGeneratedKind::Object),
        "ReportObject" => ("Report", EventSourceGeneratedKind::Object),
        "DataProcessorObject" => ("DataProcessor", EventSourceGeneratedKind::Object),
        "CatalogManager" => ("Catalog", EventSourceGeneratedKind::Manager),
        "DocumentManager" => ("Document", EventSourceGeneratedKind::Manager),
        "EnumManager" => ("Enum", EventSourceGeneratedKind::Manager),
        "ConstantValueManager" => ("Constant", EventSourceGeneratedKind::Manager),
        "InformationRegisterManager" => ("InformationRegister", EventSourceGeneratedKind::Manager),
        "AccumulationRegisterManager" => {
            ("AccumulationRegister", EventSourceGeneratedKind::Manager)
        }
        "AccountingRegisterManager" => ("AccountingRegister", EventSourceGeneratedKind::Manager),
        "CalculationRegisterManager" => ("CalculationRegister", EventSourceGeneratedKind::Manager),
        "ChartOfAccountsManager" => ("ChartOfAccounts", EventSourceGeneratedKind::Manager),
        "ChartOfCharacteristicTypesManager" => (
            "ChartOfCharacteristicTypes",
            EventSourceGeneratedKind::Manager,
        ),
        "ChartOfCalculationTypesManager" => {
            ("ChartOfCalculationTypes", EventSourceGeneratedKind::Manager)
        }
        "BusinessProcessManager" => ("BusinessProcess", EventSourceGeneratedKind::Manager),
        "TaskManager" => ("Task", EventSourceGeneratedKind::Manager),
        "ExchangePlanManager" => ("ExchangePlan", EventSourceGeneratedKind::Manager),
        "DocumentJournalManager" => ("DocumentJournal", EventSourceGeneratedKind::Manager),
        "ReportManager" => ("Report", EventSourceGeneratedKind::Manager),
        "DataProcessorManager" => ("DataProcessor", EventSourceGeneratedKind::Manager),
        "CatalogRef" => ("Catalog", EventSourceGeneratedKind::Reference),
        "DocumentRef" => ("Document", EventSourceGeneratedKind::Reference),
        "EnumRef" => ("Enum", EventSourceGeneratedKind::Reference),
        "ChartOfAccountsRef" => ("ChartOfAccounts", EventSourceGeneratedKind::Reference),
        "ChartOfCharacteristicTypesRef" => (
            "ChartOfCharacteristicTypes",
            EventSourceGeneratedKind::Reference,
        ),
        "ChartOfCalculationTypesRef" => (
            "ChartOfCalculationTypes",
            EventSourceGeneratedKind::Reference,
        ),
        "ExchangePlanRef" => ("ExchangePlan", EventSourceGeneratedKind::Reference),
        "BusinessProcessRef" => ("BusinessProcess", EventSourceGeneratedKind::Reference),
        "TaskRef" => ("Task", EventSourceGeneratedKind::Reference),
        "InformationRegisterRecordSet" => {
            ("InformationRegister", EventSourceGeneratedKind::RecordSet)
        }
        "AccumulationRegisterRecordSet" => {
            ("AccumulationRegister", EventSourceGeneratedKind::RecordSet)
        }
        "AccountingRegisterRecordSet" => {
            ("AccountingRegister", EventSourceGeneratedKind::RecordSet)
        }
        "CalculationRegisterRecordSet" => {
            ("CalculationRegister", EventSourceGeneratedKind::RecordSet)
        }
        "DefinedType" => ("DefinedType", EventSourceGeneratedKind::DefinedType),
        _ => return None,
    };
    Some(contract)
}

fn parse_event_source_generated(
    tag: &str,
    wire_type: &str,
    generated: &str,
) -> Result<MetaEventSource, String> {
    if tag == "TypeSet" && !generated.contains('.') {
        let source_class = event_source_class_from_wire_name(generated).ok_or_else(|| {
            format!("EventSubscription Source class family {wire_type} is unsupported")
        })?;
        return Ok(MetaEventSource::Family { source_class });
    }
    let (prefix, name) = generated.split_once('.').ok_or_else(|| {
        format!("EventSubscription Source configuration type {wire_type} is malformed")
    })?;
    if name.contains('.') || !metadata_identifier_is_valid(name) {
        return Err(format!(
            "EventSubscription Source configuration type {wire_type} is malformed"
        ));
    }
    let (owner_kind, generated_kind) =
        event_source_generated_contract(prefix).ok_or_else(|| {
            format!("EventSubscription Source configuration type {wire_type} is unsupported")
        })?;
    let expected_tag = if matches!(generated_kind, EventSourceGeneratedKind::DefinedType) {
        "TypeSet"
    } else {
        "Type"
    };
    if tag != expected_tag {
        return Err(format!(
            "EventSubscription Source {wire_type} must use v8:{expected_tag}"
        ));
    }
    let metadata_path = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner_kind}.{name}"),
    )
    .map_err(|_| format!("EventSubscription Source configuration type {wire_type} is malformed"))?;
    Ok(match generated_kind {
        EventSourceGeneratedKind::Object => MetaEventSource::Object { metadata_path },
        EventSourceGeneratedKind::Manager => MetaEventSource::Manager { metadata_path },
        EventSourceGeneratedKind::Reference => MetaEventSource::Reference { metadata_path },
        EventSourceGeneratedKind::RecordSet => MetaEventSource::RecordSet { metadata_path },
        EventSourceGeneratedKind::DefinedType => MetaEventSource::DefinedType { metadata_path },
    })
}

/// Parse the exact direct contents of EventSubscription/Properties/Source.
/// The same closed parser is used by mutation planning and meta.info readback.
pub(super) fn parse_meta_event_subscription_source(
    source: roxmltree::Node<'_, '_>,
) -> Result<Vec<MetaEventSource>, String> {
    if source.tag_name().namespace() != Some(MD_CLASSES_NS) || source.tag_name().name() != "Source"
    {
        return Err("EventSubscription source must be a direct MDClasses Source".to_string());
    }
    if source.attributes().len() != 0 {
        return Err("EventSubscription Properties/Source must not have attributes".to_string());
    }

    enum WireSource {
        Ready(MetaEventSource),
        String,
        Number,
        Date,
    }

    let mut wire_sources = Vec::new();
    let mut seen_wire_types = std::collections::HashSet::new();
    let mut type_stage = 0u8;
    let mut qualifier_rank = 0u8;
    let mut seen_qualifiers = std::collections::HashSet::new();
    let mut string_qualifier = None;
    let mut number_qualifier = None;
    let mut date_qualifier = None;

    for child in source.children() {
        if child.is_text() && child.text().is_some_and(|text| text.trim().is_empty()) {
            continue;
        }
        if !child.is_element() {
            return Err(
                "EventSubscription Source may contain only whitespace and v8 elements".to_string(),
            );
        }
        if child.tag_name().namespace() != Some(DATA_CORE_NS) {
            return Err(format!(
                "EventSubscription Source child {} must use namespace {{{DATA_CORE_NS}}}",
                child.tag_name().name()
            ));
        }
        match child.tag_name().name() {
            tag @ ("Type" | "TypeSet") => {
                if qualifier_rank != 0 || (tag == "Type" && type_stage > 0) {
                    return Err(
                        "EventSubscription Source requires v8:Type before v8:TypeSet before qualifiers"
                            .to_string(),
                    );
                }
                if tag == "TypeSet" {
                    type_stage = 1;
                }
                let wire_type =
                    strict_leaf_text(child, &format!("EventSubscription Source v8:{tag}"))?;
                let qname = resolve_event_source_qname(child, &wire_type)?;
                if !seen_wire_types.insert((tag.to_string(), qname.clone())) {
                    return Err(format!(
                        "EventSubscription Source contains duplicate v8:{tag} {wire_type}"
                    ));
                }
                let parsed = match (tag, qname.namespace.as_str(), qname.local_name.as_str()) {
                    ("Type", XML_SCHEMA_NS, "string") => WireSource::String,
                    ("Type", XML_SCHEMA_NS, "decimal") => WireSource::Number,
                    ("Type", XML_SCHEMA_NS, "boolean") => {
                        WireSource::Ready(MetaEventSource::Boolean)
                    }
                    ("Type", XML_SCHEMA_NS, "dateTime") => WireSource::Date,
                    ("Type", DATA_CORE_NS, "ValueStorage") => {
                        WireSource::Ready(MetaEventSource::ValueStorage)
                    }
                    (_, CURRENT_CONFIG_NS, generated) => {
                        WireSource::Ready(parse_event_source_generated(tag, &wire_type, generated)?)
                    }
                    _ => {
                        return Err(format!(
                            "EventSubscription Source contains unsupported wire type {wire_type}"
                        ));
                    }
                };
                wire_sources.push(parsed);
            }
            qualifier @ ("NumberQualifiers" | "StringQualifiers" | "DateQualifiers") => {
                let rank = match qualifier {
                    "NumberQualifiers" => 1,
                    "StringQualifiers" => 2,
                    "DateQualifiers" => 3,
                    _ => return Err("unreachable EventSubscription qualifier".to_string()),
                };
                if rank < qualifier_rank || !seen_qualifiers.insert(qualifier) {
                    return Err(format!(
                        "EventSubscription Source contains duplicate or out-of-order v8:{qualifier}"
                    ));
                }
                qualifier_rank = rank;
                match qualifier {
                    "NumberQualifiers" => {
                        let values = strict_qualifier_values(
                            child,
                            &["Digits", "FractionDigits", "AllowedSign"],
                        )?;
                        let digits = values[0].parse::<u32>().map_err(|_| {
                            "EventSubscription Source v8:Digits must be an unsigned integer"
                                .to_string()
                        })?;
                        let fraction = values[1].parse::<u32>().map_err(|_| {
                            "EventSubscription Source v8:FractionDigits must be an unsigned integer"
                                .to_string()
                        })?;
                        let sign = match values[2].as_str() {
                            "Any" => Ok(NumberSign::Any),
                            "Nonnegative" => Ok(NumberSign::NonNegative),
                            other => Err(format!(
                                "EventSubscription Source v8:AllowedSign is unsupported: {other}"
                            )),
                        }?;
                        number_qualifier = Some((digits, fraction, sign));
                    }
                    "StringQualifiers" => {
                        let values = strict_qualifier_values(child, &["Length", "AllowedLength"])?;
                        let length = values[0].parse::<u32>().map_err(|_| {
                            "EventSubscription Source v8:Length must be an unsigned integer"
                                .to_string()
                        })?;
                        let allowed_length = match values[1].as_str() {
                            "Variable" => Ok(StringLengthMode::Variable),
                            "Fixed" => Ok(StringLengthMode::Fixed),
                            other => Err(format!(
                                "EventSubscription Source v8:AllowedLength is unsupported: {other}"
                            )),
                        }?;
                        string_qualifier = Some((length, allowed_length));
                    }
                    "DateQualifiers" => {
                        let values = strict_qualifier_values(child, &["DateFractions"])?;
                        let fractions = match values[0].as_str() {
                            "Date" => Ok(MetaEventSourceDateFractions::Date),
                            "DateTime" => Ok(MetaEventSourceDateFractions::DateTime),
                            other => Err(format!(
                                "EventSubscription Source v8:DateFractions is unsupported: {other}"
                            )),
                        }?;
                        date_qualifier = Some(fractions);
                    }
                    _ => return Err("unreachable EventSubscription qualifier".to_string()),
                }
            }
            other => {
                return Err(format!(
                    "EventSubscription Source contains unsupported direct v8:{other} element"
                ))
            }
        }
    }

    let has_string = wire_sources
        .iter()
        .any(|source| matches!(source, WireSource::String));
    let has_number = wire_sources
        .iter()
        .any(|source| matches!(source, WireSource::Number));
    let has_date = wire_sources
        .iter()
        .any(|source| matches!(source, WireSource::Date));
    if has_string != string_qualifier.is_some()
        || has_number != number_qualifier.is_some()
        || has_date != date_qualifier.is_some()
    {
        return Err(
            "EventSubscription Source primitive types and qualifiers must match exactly"
                .to_string(),
        );
    }

    let mut sources = Vec::with_capacity(wire_sources.len());
    for source in wire_sources {
        sources.push(match source {
            WireSource::Ready(source) => source,
            WireSource::String => {
                let (length, allowed_length) = string_qualifier.ok_or_else(|| {
                    "EventSubscription Source string qualifiers are unavailable".to_string()
                })?;
                MetaEventSource::String {
                    length,
                    allowed_length,
                }
            }
            WireSource::Number => {
                let (digits, fraction, sign) = number_qualifier.ok_or_else(|| {
                    "EventSubscription Source number qualifiers are unavailable".to_string()
                })?;
                MetaEventSource::Number {
                    digits,
                    fraction,
                    sign,
                }
            }
            WireSource::Date => MetaEventSource::Date {
                fractions: date_qualifier.ok_or_else(|| {
                    "EventSubscription Source date qualifiers are unavailable".to_string()
                })?,
            },
        });
    }
    if sources.len() > 1
        && sources
            .iter()
            .any(|source| matches!(source, MetaEventSource::ValueStorage))
    {
        return Err(
            "ValueStorage must be the only platform type in an 8.3.27 EventSubscription Source"
                .to_string(),
        );
    }
    let mut identities = std::collections::HashSet::new();
    for source in &sources {
        match source {
            MetaEventSource::String {
                length,
                allowed_length,
            } if *length != 0 || *allowed_length != StringLengthMode::Variable => {
                return Err(
                    "EventSubscription Source string requires Length=0 and AllowedLength=Variable"
                        .to_string(),
                )
            }
            MetaEventSource::Number {
                digits, fraction, ..
            } if *digits > 38 || *fraction > *digits => {
                return Err(
                    "EventSubscription Source number requires Digits 0..=38 and FractionDigits not greater than Digits"
                        .to_string(),
                )
            }
            _ => {}
        }
        if !identities.insert(source.identity_key()) {
            return Err(format!(
                "EventSubscription Source contains duplicate semantic identity {}",
                source.identity_key()
            ));
        }
    }
    Ok(sources)
}

#[derive(Clone, Copy)]
enum EventSourceWireNamespace {
    Data,
    CurrentConfig,
    XmlSchema,
}

fn event_source_wire_contract(
    source: &MetaEventSource,
) -> (&'static str, EventSourceWireNamespace, String) {
    match source {
        MetaEventSource::String { .. } => (
            "Type",
            EventSourceWireNamespace::XmlSchema,
            "string".to_string(),
        ),
        MetaEventSource::Number { .. } => (
            "Type",
            EventSourceWireNamespace::XmlSchema,
            "decimal".to_string(),
        ),
        MetaEventSource::Boolean => (
            "Type",
            EventSourceWireNamespace::XmlSchema,
            "boolean".to_string(),
        ),
        MetaEventSource::Date { .. } => (
            "Type",
            EventSourceWireNamespace::XmlSchema,
            "dateTime".to_string(),
        ),
        MetaEventSource::ValueStorage => (
            "Type",
            EventSourceWireNamespace::Data,
            "ValueStorage".to_string(),
        ),
        MetaEventSource::Object { metadata_path }
        | MetaEventSource::Manager { metadata_path }
        | MetaEventSource::Reference { metadata_path }
        | MetaEventSource::RecordSet { metadata_path } => {
            let prefix = event_source_generated_prefix(source);
            let name = metadata_path.segments().nth(1).unwrap_or_default();
            (
                "Type",
                EventSourceWireNamespace::CurrentConfig,
                format!("{prefix}.{name}"),
            )
        }
        MetaEventSource::DefinedType { metadata_path } => {
            let name = metadata_path.segments().nth(1).unwrap_or_default();
            (
                "TypeSet",
                EventSourceWireNamespace::CurrentConfig,
                format!("DefinedType.{name}"),
            )
        }
        MetaEventSource::Family { source_class } => (
            "TypeSet",
            EventSourceWireNamespace::CurrentConfig,
            event_source_class_wire_name(*source_class).to_string(),
        ),
    }
}

struct EventSourceEmissionNamespaces {
    data: String,
    current_config: Option<String>,
    xml_schema: Option<String>,
    declarations: Vec<(String, &'static str)>,
}

impl EventSourceEmissionNamespaces {
    fn for_source(context: roxmltree::Node<'_, '_>, sources: &[MetaEventSource]) -> Self {
        let mut declarations = Vec::new();
        let mut allocated_prefixes = std::collections::BTreeSet::new();
        let data = event_source_emission_prefix(
            context,
            "v8",
            DATA_CORE_NS,
            &mut declarations,
            &mut allocated_prefixes,
        );
        let current_config = sources
            .iter()
            .any(|source| {
                source.metadata_path().is_some() || matches!(source, MetaEventSource::Family { .. })
            })
            .then(|| {
                event_source_emission_prefix(
                    context,
                    "cfg",
                    CURRENT_CONFIG_NS,
                    &mut declarations,
                    &mut allocated_prefixes,
                )
            });
        let xml_schema = sources
            .iter()
            .any(|source| {
                matches!(
                    source,
                    MetaEventSource::String { .. }
                        | MetaEventSource::Number { .. }
                        | MetaEventSource::Boolean
                        | MetaEventSource::Date { .. }
                )
            })
            .then(|| {
                event_source_emission_prefix(
                    context,
                    "xs",
                    XML_SCHEMA_NS,
                    &mut declarations,
                    &mut allocated_prefixes,
                )
            });
        Self {
            data,
            current_config,
            xml_schema,
            declarations,
        }
    }

    fn wire_prefix(&self, namespace: EventSourceWireNamespace) -> &str {
        match namespace {
            EventSourceWireNamespace::Data => &self.data,
            EventSourceWireNamespace::CurrentConfig => self
                .current_config
                .as_deref()
                .expect("configuration source requires a configuration namespace"),
            EventSourceWireNamespace::XmlSchema => self
                .xml_schema
                .as_deref()
                .expect("primitive source requires an XML Schema namespace"),
        }
    }
}

fn event_source_emission_prefix(
    context: roxmltree::Node<'_, '_>,
    canonical_prefix: &'static str,
    namespace_uri: &'static str,
    declarations: &mut Vec<(String, &'static str)>,
    allocated_prefixes: &mut std::collections::BTreeSet<String>,
) -> String {
    if !allocated_prefixes.contains(canonical_prefix)
        && context.lookup_namespace_uri(Some(canonical_prefix)) == Some(namespace_uri)
    {
        allocated_prefixes.insert(canonical_prefix.to_string());
        return canonical_prefix.to_string();
    }
    if let Some(prefix) = context.namespaces().find_map(|namespace| {
        (namespace.uri() == namespace_uri
            && namespace
                .name()
                .is_some_and(|prefix| !allocated_prefixes.contains(prefix)))
        .then(|| namespace.name())
        .flatten()
    }) {
        allocated_prefixes.insert(prefix.to_string());
        return prefix.to_string();
    }
    let mut prefix = canonical_prefix.to_string();
    let mut suffix = 1_u32;
    while allocated_prefixes.contains(&prefix) {
        prefix = format!("{canonical_prefix}{suffix}");
        suffix += 1;
    }
    allocated_prefixes.insert(prefix.clone());
    declarations.push((prefix.clone(), namespace_uri));
    prefix
}

pub(super) fn event_source_generated_prefix(source: &MetaEventSource) -> &'static str {
    let kind = source
        .metadata_path()
        .and_then(|path| path.segments().next())
        .unwrap_or_default();
    match source {
        MetaEventSource::Object { .. } => match kind {
            "Catalog" => "CatalogObject",
            "Document" => "DocumentObject",
            "ChartOfAccounts" => "ChartOfAccountsObject",
            "ChartOfCharacteristicTypes" => "ChartOfCharacteristicTypesObject",
            "ChartOfCalculationTypes" => "ChartOfCalculationTypesObject",
            "ExchangePlan" => "ExchangePlanObject",
            "BusinessProcess" => "BusinessProcessObject",
            "Task" => "TaskObject",
            "Report" => "ReportObject",
            "DataProcessor" => "DataProcessorObject",
            _ => "",
        },
        MetaEventSource::Manager { .. } => match kind {
            "Catalog" => "CatalogManager",
            "Document" => "DocumentManager",
            "Enum" => "EnumManager",
            "Constant" => "ConstantValueManager",
            "InformationRegister" => "InformationRegisterManager",
            "AccumulationRegister" => "AccumulationRegisterManager",
            "AccountingRegister" => "AccountingRegisterManager",
            "CalculationRegister" => "CalculationRegisterManager",
            "ChartOfAccounts" => "ChartOfAccountsManager",
            "ChartOfCharacteristicTypes" => "ChartOfCharacteristicTypesManager",
            "ChartOfCalculationTypes" => "ChartOfCalculationTypesManager",
            "BusinessProcess" => "BusinessProcessManager",
            "Task" => "TaskManager",
            "ExchangePlan" => "ExchangePlanManager",
            "DocumentJournal" => "DocumentJournalManager",
            "Report" => "ReportManager",
            "DataProcessor" => "DataProcessorManager",
            _ => "",
        },
        MetaEventSource::Reference { .. } => match kind {
            "Catalog" => "CatalogRef",
            "Document" => "DocumentRef",
            "Enum" => "EnumRef",
            "ChartOfAccounts" => "ChartOfAccountsRef",
            "ChartOfCharacteristicTypes" => "ChartOfCharacteristicTypesRef",
            "ChartOfCalculationTypes" => "ChartOfCalculationTypesRef",
            "ExchangePlan" => "ExchangePlanRef",
            "BusinessProcess" => "BusinessProcessRef",
            "Task" => "TaskRef",
            _ => "",
        },
        MetaEventSource::RecordSet { .. } => match kind {
            "InformationRegister" => "InformationRegisterRecordSet",
            "AccumulationRegister" => "AccumulationRegisterRecordSet",
            "AccountingRegister" => "AccountingRegisterRecordSet",
            "CalculationRegister" => "CalculationRegisterRecordSet",
            _ => "",
        },
        MetaEventSource::DefinedType { .. } => "DefinedType",
        MetaEventSource::Family { source_class } => event_source_class_wire_name(*source_class),
        MetaEventSource::String { .. }
        | MetaEventSource::Number { .. }
        | MetaEventSource::Boolean
        | MetaEventSource::Date { .. }
        | MetaEventSource::ValueStorage => "",
    }
}

fn event_source_group_rank(source: &MetaEventSource) -> u8 {
    match source {
        MetaEventSource::Object { .. }
        | MetaEventSource::Manager { .. }
        | MetaEventSource::Reference { .. }
        | MetaEventSource::RecordSet { .. } => 0,
        MetaEventSource::Boolean => 1,
        MetaEventSource::String { .. } => 2,
        MetaEventSource::Date { .. } => 3,
        MetaEventSource::Number { .. } => 4,
        MetaEventSource::ValueStorage => 5,
        MetaEventSource::DefinedType { .. } => 6,
        MetaEventSource::Family { .. } => 6,
    }
}

fn event_source_semantic_key(source: &MetaEventSource) -> String {
    match source {
        MetaEventSource::String {
            length,
            allowed_length,
        } => format!(
            "string:{length}:{}",
            match allowed_length {
                StringLengthMode::Variable => "variable",
                StringLengthMode::Fixed => "fixed",
            }
        ),
        MetaEventSource::Number {
            digits,
            fraction,
            sign,
        } => format!(
            "number:{digits}:{fraction}:{}",
            match sign {
                NumberSign::Any => "any",
                NumberSign::NonNegative => "nonnegative",
            }
        ),
        MetaEventSource::Boolean => "boolean".to_string(),
        MetaEventSource::Date { fractions } => format!(
            "date:{}",
            match fractions {
                MetaEventSourceDateFractions::Date => "date",
                MetaEventSourceDateFractions::DateTime => "dateTime",
            }
        ),
        MetaEventSource::ValueStorage => "valueStorage".to_string(),
        MetaEventSource::Object { metadata_path } => {
            format!("object:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::Manager { metadata_path } => {
            format!("manager:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::Reference { metadata_path } => {
            format!("reference:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::RecordSet { metadata_path } => {
            format!("recordSet:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::DefinedType { metadata_path } => {
            format!("definedType:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::Family { source_class } => {
            format!("family:{}", source_class.as_str())
        }
    }
}

fn event_source_class_wire_name(source_class: EventSourceClass) -> &'static str {
    match source_class {
        EventSourceClass::AccountingRegisterManager => "AccountingRegisterManager",
        EventSourceClass::AccountingRegisterRecordSet => "AccountingRegisterRecordSet",
        EventSourceClass::AccumulationRegisterManager => "AccumulationRegisterManager",
        EventSourceClass::AccumulationRegisterRecordSet => "AccumulationRegisterRecordSet",
        EventSourceClass::BusinessProcessManager => "BusinessProcessManager",
        EventSourceClass::BusinessProcessObject => "BusinessProcessObject",
        EventSourceClass::CalculationRegisterManager => "CalculationRegisterManager",
        EventSourceClass::CalculationRegisterRecordSet => "CalculationRegisterRecordSet",
        EventSourceClass::CatalogManager => "CatalogManager",
        EventSourceClass::CatalogObject => "CatalogObject",
        EventSourceClass::ChartOfAccountsManager => "ChartOfAccountsManager",
        EventSourceClass::ChartOfAccountsObject => "ChartOfAccountsObject",
        EventSourceClass::ChartOfCalculationTypesManager => "ChartOfCalculationTypesManager",
        EventSourceClass::ChartOfCalculationTypesObject => "ChartOfCalculationTypesObject",
        EventSourceClass::ChartOfCharacteristicTypesManager => "ChartOfCharacteristicTypesManager",
        EventSourceClass::ChartOfCharacteristicTypesObject => "ChartOfCharacteristicTypesObject",
        EventSourceClass::ConstantManager => "ConstantManager",
        EventSourceClass::ConstantValueManager => "ConstantValueManager",
        EventSourceClass::DataProcessorManager => "DataProcessorManager",
        EventSourceClass::DataProcessorObject => "DataProcessorObject",
        EventSourceClass::DocumentJournalManager => "DocumentJournalManager",
        EventSourceClass::DocumentManager => "DocumentManager",
        EventSourceClass::DocumentObject => "DocumentObject",
        EventSourceClass::EnumManager => "EnumManager",
        EventSourceClass::ExchangePlanManager => "ExchangePlanManager",
        EventSourceClass::ExchangePlanObject => "ExchangePlanObject",
        EventSourceClass::ExternalDataSourceCubeDimensionTableManager => {
            "ExternalDataSourceCubeDimensionTableManager"
        }
        EventSourceClass::ExternalDataSourceCubeManager => "ExternalDataSourceCubeManager",
        EventSourceClass::ExternalDataSourceTableManager => "ExternalDataSourceTableManager",
        EventSourceClass::ExternalDataSourceTableObject => "ExternalDataSourceTableObject",
        EventSourceClass::ExternalDataSourceTableRecordSet => "ExternalDataSourceTableRecordSet",
        EventSourceClass::FilterCriterionManager => "FilterCriterionManager",
        EventSourceClass::InformationRegisterManager => "InformationRegisterManager",
        EventSourceClass::InformationRegisterRecordSet => "InformationRegisterRecordSet",
        EventSourceClass::RecalculationRecordSet => "RecalculationRecordSet",
        EventSourceClass::ReportManager => "ReportManager",
        EventSourceClass::ReportObject => "ReportObject",
        EventSourceClass::SequenceRecordSet => "SequenceRecordSet",
        EventSourceClass::SettingsStorageManager => "SettingsStorageManager",
        EventSourceClass::TaskManager => "TaskManager",
        EventSourceClass::TaskObject => "TaskObject",
    }
}

fn event_source_class_from_wire_name(wire_name: &str) -> Option<EventSourceClass> {
    EventSourceClass::ALL
        .iter()
        .copied()
        .find(|source_class| event_source_class_wire_name(*source_class) == wire_name)
}

pub(super) fn canonical_meta_event_sources(sources: &[MetaEventSource]) -> Vec<MetaEventSource> {
    let mut canonical = sources.to_vec();
    canonical.sort_by(|left, right| {
        event_source_group_rank(left)
            .cmp(&event_source_group_rank(right))
            .then_with(|| event_source_semantic_key(left).cmp(&event_source_semantic_key(right)))
    });
    canonical
}

/// Emit only the Source element. Callers supply its existing indentation so
/// direct-node replacement preserves the descriptor's surrounding layout.
pub(super) fn emit_meta_event_subscription_source(
    indent: &str,
    sources: &[MetaEventSource],
    namespace_context: roxmltree::Node<'_, '_>,
) -> String {
    if sources.is_empty() {
        return format!("{indent}<Source/>");
    }
    let sources = canonical_meta_event_sources(sources);
    // The complete old Source node is replaced, so only declarations on its
    // surviving parent (or an ancestor) can be reused by the new fragment.
    let surviving_namespace_context = namespace_context
        .parent_element()
        .unwrap_or(namespace_context);
    let namespaces =
        EventSourceEmissionNamespaces::for_source(surviving_namespace_context, &sources);
    let content_indent = format!("{indent}\t");
    let declarations = namespaces
        .declarations
        .iter()
        .map(|(prefix, uri)| format!(r#" xmlns:{prefix}="{uri}""#))
        .collect::<String>();
    let mut lines = vec![format!("{indent}<Source{declarations}>")];
    let data_prefix = &namespaces.data;
    for expected_tag in ["Type", "TypeSet"] {
        for source in &sources {
            let (tag, namespace, local_name) = event_source_wire_contract(source);
            if tag == expected_tag {
                let wire_type = format!("{}:{local_name}", namespaces.wire_prefix(namespace));
                lines.push(format!(
                    "{content_indent}<{data_prefix}:{tag}>{}</{data_prefix}:{tag}>",
                    escape_xml(&wire_type)
                ));
            }
        }
    }
    for source in &sources {
        if let MetaEventSource::Number {
            digits,
            fraction,
            sign,
        } = source
        {
            lines.push(format!("{content_indent}<{data_prefix}:NumberQualifiers>"));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:Digits>{digits}</{data_prefix}:Digits>"
            ));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:FractionDigits>{fraction}</{data_prefix}:FractionDigits>"
            ));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:AllowedSign>{}</{data_prefix}:AllowedSign>",
                match sign {
                    NumberSign::Any => "Any",
                    NumberSign::NonNegative => "Nonnegative",
                }
            ));
            lines.push(format!("{content_indent}</{data_prefix}:NumberQualifiers>"));
        }
    }
    for source in &sources {
        if let MetaEventSource::String {
            length,
            allowed_length,
        } = source
        {
            lines.push(format!("{content_indent}<{data_prefix}:StringQualifiers>"));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:Length>{length}</{data_prefix}:Length>"
            ));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:AllowedLength>{}</{data_prefix}:AllowedLength>",
                match allowed_length {
                    StringLengthMode::Variable => "Variable",
                    StringLengthMode::Fixed => "Fixed",
                }
            ));
            lines.push(format!("{content_indent}</{data_prefix}:StringQualifiers>"));
        }
    }
    for source in &sources {
        if let MetaEventSource::Date { fractions } = source {
            lines.push(format!("{content_indent}<{data_prefix}:DateQualifiers>"));
            lines.push(format!(
                "{content_indent}\t<{data_prefix}:DateFractions>{}</{data_prefix}:DateFractions>",
                match fractions {
                    MetaEventSourceDateFractions::Date => "Date",
                    MetaEventSourceDateFractions::DateTime => "DateTime",
                }
            ));
            lines.push(format!("{content_indent}</{data_prefix}:DateQualifiers>"));
        }
    }
    lines.push(format!("{indent}</Source>"));
    lines.join("\n")
}

pub(super) fn meta_info_ml_text(node: roxmltree::Node<'_, '_>) -> String {
    let value = multilang_text(node);
    if value.is_empty() {
        node.text().unwrap_or("").trim().to_string()
    } else {
        value
    }
}

pub(super) fn meta_info_normalize_cfg_prefix(raw: &str) -> String {
    let Some((prefix, rest)) = raw.split_once(':') else {
        return raw.to_string();
    };
    if prefix.starts_with('d')
        && prefix[1..]
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == 'p')
    {
        format!("cfg:{rest}")
    } else {
        raw.to_string()
    }
}

pub(super) fn emit_meta_mltext(lines: &mut Vec<String>, indent: &str, tag: &str, text: &str) {
    if text.is_empty() {
        lines.push(format!("{indent}<{tag}/>"));
        return;
    }
    lines.push(format!("{indent}<{tag}>"));
    lines.push(format!("{indent}\t<v8:item>"));
    lines.push(format!("{indent}\t\t<v8:lang>ru</v8:lang>"));
    lines.push(format!(
        "{indent}\t\t<v8:content>{}</v8:content>",
        escape_xml(text)
    ));
    lines.push(format!("{indent}\t</v8:item>"));
    lines.push(format!("{indent}</{tag}>"));
}

pub(super) enum MetadataXmlType<'a> {
    Boolean,
    String { length: u32 },
    Number { digits: u32, fraction: u32 },
    DateTime,
    Configuration(&'a str),
}

pub(super) fn emit_metadata_xml_value_type(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_types: &[MetadataXmlType<'_>],
) {
    lines.push(format!("{indent}<Type>"));
    emit_metadata_xml_type_contents(lines, &format!("{indent}\t"), metadata_types);
    lines.push(format!("{indent}</Type>"));
}

pub(super) fn emit_metadata_xml_type_contents(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_types: &[MetadataXmlType<'_>],
) {
    for metadata_type in metadata_types {
        let wire_name = match metadata_type {
            MetadataXmlType::Boolean => "xs:boolean".to_string(),
            MetadataXmlType::String { .. } => "xs:string".to_string(),
            MetadataXmlType::Number { .. } => "xs:decimal".to_string(),
            MetadataXmlType::DateTime => "xs:dateTime".to_string(),
            MetadataXmlType::Configuration(name) => format!("cfg:{name}"),
        };
        lines.push(format!(
            "{indent}<v8:Type>{}</v8:Type>",
            escape_xml(&wire_name)
        ));
    }
    for qualifier_rank in 0..3 {
        for metadata_type in metadata_types {
            let rank = match metadata_type {
                MetadataXmlType::Number { .. } => 0,
                MetadataXmlType::String { .. } => 1,
                MetadataXmlType::DateTime => 2,
                MetadataXmlType::Boolean | MetadataXmlType::Configuration(_) => continue,
            };
            if rank != qualifier_rank {
                continue;
            }
            match metadata_type {
                MetadataXmlType::Number { digits, fraction } => {
                    lines.push(format!("{indent}<v8:NumberQualifiers>"));
                    lines.push(format!("{indent}\t<v8:Digits>{digits}</v8:Digits>"));
                    lines.push(format!(
                        "{indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
                    ));
                    lines.push(format!("{indent}\t<v8:AllowedSign>Any</v8:AllowedSign>"));
                    lines.push(format!("{indent}</v8:NumberQualifiers>"));
                }
                MetadataXmlType::String { length } => {
                    lines.push(format!("{indent}<v8:StringQualifiers>"));
                    lines.push(format!("{indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{indent}\t<v8:AllowedLength>Variable</v8:AllowedLength>"
                    ));
                    lines.push(format!("{indent}</v8:StringQualifiers>"));
                }
                MetadataXmlType::DateTime => {
                    lines.push(format!("{indent}<v8:DateQualifiers>"));
                    lines.push(format!(
                        "{indent}\t<v8:DateFractions>DateTime</v8:DateFractions>"
                    ));
                    lines.push(format!("{indent}</v8:DateQualifiers>"));
                }
                MetadataXmlType::Boolean | MetadataXmlType::Configuration(_) => {}
            }
        }
    }
}

/// Emit the closed metadata type algebra directly to Platform XML. This is the
/// typed writer boundary: it intentionally does not round-trip through the
/// legacy string grammar.
pub(super) fn emit_meta_typed_value_type(
    lines: &mut Vec<String>,
    indent: &str,
    metadata_type: &MetadataType,
) {
    lines.push(format!("{indent}<Type>"));
    let content_indent = format!("{indent}\t");
    for expected_tag in ["Type", "TypeSet"] {
        for variant in &metadata_type.variants {
            let (tag, wire_name) = typed_wire_type(variant);
            if tag != expected_tag {
                continue;
            }
            lines.push(format!(
                "{content_indent}<v8:{tag}>{}</v8:{tag}>",
                escape_xml(&wire_name)
            ));
        }
    }
    for qualifier_rank in 0..4 {
        for variant in &metadata_type.variants {
            let variant_rank = match variant {
                MetadataTypeVariant::Number { .. } => 0,
                MetadataTypeVariant::String { .. } => 1,
                MetadataTypeVariant::Date { .. } => 2,
                MetadataTypeVariant::BinaryData { .. } => 3,
                _ => continue,
            };
            if variant_rank != qualifier_rank {
                continue;
            }
            match variant {
                MetadataTypeVariant::String {
                    length,
                    allowed_length,
                } => {
                    lines.push(format!("{content_indent}<v8:StringQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                        match allowed_length {
                            StringLengthMode::Variable => "Variable",
                            StringLengthMode::Fixed => "Fixed",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:StringQualifiers>"));
                }
                MetadataTypeVariant::Number {
                    digits,
                    fraction,
                    sign,
                } => {
                    lines.push(format!("{content_indent}<v8:NumberQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Digits>{digits}</v8:Digits>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:FractionDigits>{fraction}</v8:FractionDigits>"
                    ));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedSign>{}</v8:AllowedSign>",
                        match sign {
                            NumberSign::Any => "Any",
                            NumberSign::NonNegative => "Nonnegative",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:NumberQualifiers>"));
                }
                MetadataTypeVariant::Date { fractions } => {
                    lines.push(format!("{content_indent}<v8:DateQualifiers>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:DateFractions>{}</v8:DateFractions>",
                        match fractions {
                            DateFractions::Date => "Date",
                            DateFractions::Time => "Time",
                            DateFractions::DateTime => "DateTime",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:DateQualifiers>"));
                }
                MetadataTypeVariant::BinaryData {
                    length,
                    allowed_length,
                } => {
                    lines.push(format!("{content_indent}<v8:BinaryDataQualifiers>"));
                    lines.push(format!("{content_indent}\t<v8:Length>{length}</v8:Length>"));
                    lines.push(format!(
                        "{content_indent}\t<v8:AllowedLength>{}</v8:AllowedLength>",
                        match allowed_length {
                            StringLengthMode::Variable => "Variable",
                            StringLengthMode::Fixed => "Fixed",
                        }
                    ));
                    lines.push(format!("{content_indent}</v8:BinaryDataQualifiers>"));
                }
                MetadataTypeVariant::Boolean
                | MetadataTypeVariant::ValueStorage
                | MetadataTypeVariant::Reference { .. }
                | MetadataTypeVariant::DefinedType { .. } => {}
            }
        }
    }
    lines.push(format!("{indent}</Type>"));
}

fn typed_wire_type(variant: &MetadataTypeVariant) -> (&'static str, String) {
    match variant {
        MetadataTypeVariant::String { .. } => ("Type", "xs:string".to_string()),
        MetadataTypeVariant::Number { .. } => ("Type", "xs:decimal".to_string()),
        MetadataTypeVariant::Boolean => ("Type", "xs:boolean".to_string()),
        MetadataTypeVariant::Date { .. } => ("Type", "xs:dateTime".to_string()),
        MetadataTypeVariant::BinaryData { .. } => ("Type", "xs:binary".to_string()),
        MetadataTypeVariant::ValueStorage => ("Type", "v8:ValueStorage".to_string()),
        MetadataTypeVariant::Reference { metadata_path } => {
            let mut segments = metadata_path.segments();
            let kind = segments.next().unwrap_or_default();
            let tail = segments.collect::<Vec<_>>().join(".");
            ("Type", format!("cfg:{kind}Ref.{tail}"))
        }
        MetadataTypeVariant::DefinedType { metadata_path } => {
            ("TypeSet", format!("cfg:{}", metadata_path.as_str()))
        }
    }
}

pub(super) fn emit_meta_typed_fill_value(
    lines: &mut Vec<String>,
    indent: &str,
    fill_value: Option<&MetaFillValue>,
) {
    match fill_value {
        None => lines.push(format!("{indent}<FillValue xsi:nil=\"true\"/>")),
        Some(MetaFillValue::String(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:string\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Number(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:decimal\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Boolean(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:boolean\">{value}</FillValue>"
        )),
        Some(MetaFillValue::DateTime(value)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xs:dateTime\">{}</FillValue>",
            escape_xml(value)
        )),
        Some(MetaFillValue::Reference(reference)) => lines.push(format!(
            "{indent}<FillValue xsi:type=\"xr:DesignTimeRef\">{}</FillValue>",
            escape_xml(reference.metadata_path.as_str())
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_path(value: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value).unwrap()
    }

    fn event_subscription_xml(source: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:v8="{DATA_CORE_NS}" xmlns:cfg="{CURRENT_CONFIG_NS}" xmlns:xs="{XML_SCHEMA_NS}" version="2.20"><EventSubscription><Properties><Name>Events</Name>{source}</Properties><ChildObjects/></EventSubscription></MetaDataObject>"#
        )
    }

    #[test]
    fn event_subscription_source_typed_union_round_trips_in_platform_order() {
        let requested = vec![
            MetaEventSource::DefinedType {
                metadata_path: metadata_path("DefinedType.Filter"),
            },
            MetaEventSource::Number {
                digits: 15,
                fraction: 2,
                sign: NumberSign::NonNegative,
            },
            MetaEventSource::Object {
                metadata_path: metadata_path("Catalog.Items"),
            },
            MetaEventSource::Date {
                fractions: MetaEventSourceDateFractions::Date,
            },
            MetaEventSource::Reference {
                metadata_path: metadata_path("Document.Sale"),
            },
            MetaEventSource::String {
                length: 0,
                allowed_length: StringLengthMode::Variable,
            },
            MetaEventSource::RecordSet {
                metadata_path: metadata_path("InformationRegister.Events"),
            },
            MetaEventSource::Boolean,
        ];
        let namespace_context_xml = event_subscription_xml("<Source/>");
        let namespace_context_document =
            roxmltree::Document::parse(&namespace_context_xml).unwrap();
        let namespace_context =
            meta_event_subscription_source_node(&namespace_context_document).unwrap();
        let source = emit_meta_event_subscription_source("\t", &requested, namespace_context);
        let xml = event_subscription_xml(&source);
        let document = roxmltree::Document::parse(&xml).unwrap();
        let source_node = meta_event_subscription_source_node(&document).unwrap();
        let parsed = parse_meta_event_subscription_source(source_node).unwrap();

        assert_eq!(parsed, canonical_meta_event_sources(&requested));
        assert!(
            source.find("cfg:CatalogObject.Items").unwrap() < source.find("xs:boolean").unwrap()
        );
        assert!(source.find("xs:boolean").unwrap() < source.find("xs:string").unwrap());
        assert!(source.find("xs:string").unwrap() < source.find("xs:dateTime").unwrap());
        assert!(source.find("xs:dateTime").unwrap() < source.find("xs:decimal").unwrap());
        assert!(
            source.find("xs:decimal").unwrap() < source.find("cfg:DefinedType.Filter").unwrap()
        );
        assert!(
            source.find("NumberQualifiers").unwrap() < source.find("StringQualifiers").unwrap()
        );
        assert!(source.find("StringQualifiers").unwrap() < source.find("DateQualifiers").unwrap());
    }

    #[test]
    fn event_subscription_source_accepts_empty_self_closing_and_rejects_invalid_profile() {
        let empty = event_subscription_xml("<Source/>");
        let document = roxmltree::Document::parse(&empty).unwrap();
        assert!(parse_meta_event_subscription_source(
            meta_event_subscription_source_node(&document).unwrap()
        )
        .unwrap()
        .is_empty());

        let invalid = event_subscription_xml(
            "<Source><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>1</v8:Length><v8:AllowedLength>Fixed</v8:AllowedLength></v8:StringQualifiers></Source>",
        );
        let document = roxmltree::Document::parse(&invalid).unwrap();
        assert!(parse_meta_event_subscription_source(
            meta_event_subscription_source_node(&document).unwrap()
        )
        .unwrap_err()
        .contains("Length=0"));

        for shadowed in [
            r#"<Source xmlns:cfg="urn:not-current-config"><v8:Type>cfg:CatalogRef.Items</v8:Type></Source>"#,
            r#"<Source><v8:Type xmlns:xs="urn:not-xml-schema">xs:boolean</v8:Type></Source>"#,
        ] {
            let xml = event_subscription_xml(shadowed);
            let document = roxmltree::Document::parse(&xml).unwrap();
            let error = parse_meta_event_subscription_source(
                meta_event_subscription_source_node(&document).unwrap(),
            )
            .unwrap_err();
            assert!(error.contains("must bind"), "{error}");
        }
    }

    #[test]
    fn event_subscription_source_resolves_alias_qnames_by_namespace_uri() {
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:d="{DATA_CORE_NS}" xmlns:c="{CURRENT_CONFIG_NS}" xmlns:s="{XML_SCHEMA_NS}" version="2.20"><EventSubscription><Properties><Source><d:Type>c:CatalogRef.Items</d:Type><d:Type>s:boolean</d:Type></Source></Properties></EventSubscription></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let parsed = parse_meta_event_subscription_source(
            meta_event_subscription_source_node(&document).unwrap(),
        )
        .unwrap();

        assert_eq!(
            parsed,
            vec![
                MetaEventSource::Reference {
                    metadata_path: metadata_path("Catalog.Items"),
                },
                MetaEventSource::Boolean,
            ]
        );

        let duplicate = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" xmlns:d="{DATA_CORE_NS}" xmlns:cfg="{CURRENT_CONFIG_NS}" xmlns:c="{CURRENT_CONFIG_NS}" version="2.20"><EventSubscription><Properties><Source><d:Type>cfg:CatalogRef.Items</d:Type><d:Type>c:CatalogRef.Items</d:Type></Source></Properties></EventSubscription></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&duplicate).unwrap();
        assert!(parse_meta_event_subscription_source(
            meta_event_subscription_source_node(&document).unwrap()
        )
        .unwrap_err()
        .contains("duplicate"));
    }

    #[test]
    fn event_subscription_source_does_not_shadow_an_allocated_inherited_prefix() {
        let xml = format!(
            r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><EventSubscription><Properties xmlns:cfg="{DATA_CORE_NS}"><Source/></Properties></EventSubscription></MetaDataObject>"#
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let source_node = meta_event_subscription_source_node(&document).unwrap();
        let requested = vec![MetaEventSource::Reference {
            metadata_path: metadata_path("Catalog.Items"),
        }];

        let emitted = emit_meta_event_subscription_source("", &requested, source_node);
        let rewritten = xml.replace("<Source/>", &emitted);
        let rewritten_document = roxmltree::Document::parse(&rewritten).unwrap();
        let rewritten_source = meta_event_subscription_source_node(&rewritten_document).unwrap();

        assert!(emitted.contains(&format!(r#"xmlns:cfg1="{CURRENT_CONFIG_NS}""#)));
        assert!(emitted.contains("<cfg:Type>cfg1:CatalogRef.Items</cfg:Type>"));
        assert_eq!(
            parse_meta_event_subscription_source(rewritten_source).unwrap(),
            requested
        );
    }

    #[test]
    fn event_subscription_source_rejects_generated_names_that_are_not_1c_identifiers() {
        for generated in [
            "CatalogRef.Bad Name",
            "CatalogRef.1Bad",
            "CatalogRef.Bad:Name",
            "CatalogRef.Bad-Name",
        ] {
            let xml = event_subscription_xml(&format!(
                "<Source><v8:Type>cfg:{generated}</v8:Type></Source>"
            ));
            let document = roxmltree::Document::parse(&xml).unwrap();
            let error = parse_meta_event_subscription_source(
                meta_event_subscription_source_node(&document).unwrap(),
            )
            .unwrap_err();
            assert!(
                error.contains("malformed") || error.contains("lexical QName"),
                "{generated}: {error}"
            );
        }
    }

    #[test]
    fn event_subscription_source_requires_one_exact_direct_source() {
        let nested_only = event_subscription_xml("<Wrapper><Source/></Wrapper>");
        let document = roxmltree::Document::parse(&nested_only).unwrap();
        assert!(meta_event_subscription_source_node(&document)
            .unwrap_err()
            .contains("exactly one direct Source"));

        let duplicate = event_subscription_xml("<Source/><Source/>");
        let document = roxmltree::Document::parse(&duplicate).unwrap();
        assert!(meta_event_subscription_source_node(&document)
            .unwrap_err()
            .contains("found 2"));
    }
}
