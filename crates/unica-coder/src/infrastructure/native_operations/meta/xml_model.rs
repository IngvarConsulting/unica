use super::super::common::{escape_xml, multilang_text};
use super::super::role::role_info_element;
use crate::domain::metadata::{
    DateFractions, MetaEventSource, MetaEventSourceDateFractions, MetaFillValue, MetadataType,
    MetadataTypeVariant, NumberSign, StringLengthMode,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};

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
    for (prefix, expected) in [
        (None, MD_CLASSES_NS),
        (Some("v8"), DATA_CORE_NS),
        (Some("cfg"), CURRENT_CONFIG_NS),
        (Some("xs"), XML_SCHEMA_NS),
    ] {
        if properties.lookup_namespace_uri(prefix) != Some(expected) {
            let label = prefix.unwrap_or("default");
            return Err(format!(
                "EventSubscription Properties must bind {label} namespace to {expected}"
            ));
        }
    }
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

fn parse_event_source_generated(tag: &str, wire_type: &str) -> Result<MetaEventSource, String> {
    let generated = wire_type.strip_prefix("cfg:").ok_or_else(|| {
        format!("EventSubscription Source contains unsupported wire type {wire_type}")
    })?;
    let (prefix, name) = generated.split_once('.').ok_or_else(|| {
        format!("EventSubscription Source configuration type {wire_type} is malformed")
    })?;
    if name.contains('.') || name.is_empty() {
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
                if !seen_wire_types.insert((tag.to_string(), wire_type.clone())) {
                    return Err(format!(
                        "EventSubscription Source contains duplicate v8:{tag} {wire_type}"
                    ));
                }
                let parsed = match (tag, wire_type.as_str()) {
                    ("Type", "xs:string") => WireSource::String,
                    ("Type", "xs:decimal") => WireSource::Number,
                    ("Type", "xs:boolean") => WireSource::Ready(MetaEventSource::Boolean),
                    ("Type", "xs:dateTime") => WireSource::Date,
                    ("Type", "v8:ValueStorage") => WireSource::Ready(MetaEventSource::ValueStorage),
                    _ => WireSource::Ready(parse_event_source_generated(tag, &wire_type)?),
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

fn event_source_wire_contract(source: &MetaEventSource) -> (&'static str, String) {
    match source {
        MetaEventSource::String { .. } => ("Type", "xs:string".to_string()),
        MetaEventSource::Number { .. } => ("Type", "xs:decimal".to_string()),
        MetaEventSource::Boolean => ("Type", "xs:boolean".to_string()),
        MetaEventSource::Date { .. } => ("Type", "xs:dateTime".to_string()),
        MetaEventSource::ValueStorage => ("Type", "v8:ValueStorage".to_string()),
        MetaEventSource::Object { metadata_path }
        | MetaEventSource::Reference { metadata_path }
        | MetaEventSource::RecordSet { metadata_path } => {
            let prefix = event_source_generated_prefix(source);
            let name = metadata_path.segments().nth(1).unwrap_or_default();
            ("Type", format!("cfg:{prefix}.{name}"))
        }
        MetaEventSource::DefinedType { metadata_path } => {
            let name = metadata_path.segments().nth(1).unwrap_or_default();
            ("TypeSet", format!("cfg:DefinedType.{name}"))
        }
    }
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
        | MetaEventSource::Reference { .. }
        | MetaEventSource::RecordSet { .. } => 0,
        MetaEventSource::Boolean => 1,
        MetaEventSource::String { .. } => 2,
        MetaEventSource::Date { .. } => 3,
        MetaEventSource::Number { .. } => 4,
        MetaEventSource::ValueStorage => 5,
        MetaEventSource::DefinedType { .. } => 6,
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
        MetaEventSource::Reference { metadata_path } => {
            format!("reference:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::RecordSet { metadata_path } => {
            format!("recordSet:{}", metadata_path.as_str().to_lowercase())
        }
        MetaEventSource::DefinedType { metadata_path } => {
            format!("definedType:{}", metadata_path.as_str().to_lowercase())
        }
    }
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
) -> String {
    if sources.is_empty() {
        return format!("{indent}<Source/>");
    }
    let sources = canonical_meta_event_sources(sources);
    let content_indent = format!("{indent}\t");
    let mut lines = vec![format!("{indent}<Source>")];
    for expected_tag in ["Type", "TypeSet"] {
        for source in &sources {
            let (tag, wire_type) = event_source_wire_contract(source);
            if tag == expected_tag {
                lines.push(format!(
                    "{content_indent}<v8:{tag}>{}</v8:{tag}>",
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
    }
    for source in &sources {
        if let MetaEventSource::String {
            length,
            allowed_length,
        } = source
        {
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
    }
    for source in &sources {
        if let MetaEventSource::Date { fractions } = source {
            lines.push(format!("{content_indent}<v8:DateQualifiers>"));
            lines.push(format!(
                "{content_indent}\t<v8:DateFractions>{}</v8:DateFractions>",
                match fractions {
                    MetaEventSourceDateFractions::Date => "Date",
                    MetaEventSourceDateFractions::DateTime => "DateTime",
                }
            ));
            lines.push(format!("{content_indent}</v8:DateQualifiers>"));
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
        let source = emit_meta_event_subscription_source("\t", &requested);
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
