use super::xml_model::MD_CLASSES_NS;
use roxmltree::Document;
use std::collections::HashSet;
use unica_address::source_target::MetadataAddress;
use unica_metadata::validation_subject::MetadataTemplateType;
use unica_source_designer::format_profile::ACTIVE_FORMAT_PROFILE;
use unica_source_designer::owner::{PlatformXmlRootExpectation, DCS_ROOT, MXL_ROOT};
use unica_source_designer::roots::root_version_literal;
pub fn document_registers(bytes: &[u8], expected: &str) -> bool {
    let Ok((_, document)) = unica_xml_format::read::parse_metadata_image(bytes) else {
        return false;
    };
    document
        .root_element()
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(MD_CLASSES_NS)
                && node.tag_name().name() == "Document"
        })
        .and_then(|object| unica_xml_format::read::child_element(object, "Properties"))
        .and_then(|properties| unica_xml_format::read::child_element(properties, "RegisterRecords"))
        .is_some_and(|records| {
            unica_xml_format::read::child_elements(records, "Item")
                .into_iter()
                .any(|item| unica_xml_format::read::direct_text(item).trim() == expected)
        })
}
pub fn is_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| [8, 13, 18, 23].contains(&index) || ch.is_ascii_hexdigit())
}
pub fn meta_validate_valid_types() -> &'static [&'static str] {
    &[
        "Catalog",
        "Document",
        "Enum",
        "Constant",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "ExchangePlan",
        "DocumentJournal",
        "Report",
        "DataProcessor",
        "ExternalReport",
        "ExternalDataProcessor",
        "CommonModule",
        "ScheduledJob",
        "EventSubscription",
        "HTTPService",
        "WebService",
        "DefinedType",
    ]
}
pub fn meta_validate_property_values() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("CodeType", &["String", "Number"]),
        ("CodeAllowedLength", &["Variable", "Fixed"]),
        ("NumberType", &["String", "Number"]),
        ("NumberAllowedLength", &["Variable", "Fixed"]),
        ("Posting", &["Allow", "Deny"]),
        ("RealTimePosting", &["Allow", "Deny"]),
        (
            "RegisterRecordsDeletion",
            &["AutoDelete", "AutoDeleteOnUnpost", "AutoDeleteOff"],
        ),
        (
            "RegisterRecordsWritingOnPost",
            &["WriteModified", "WriteSelected", "WriteAll"],
        ),
        ("DataLockControlMode", &["Automatic", "Managed"]),
        ("FullTextSearch", &["Use", "DontUse"]),
        ("DefaultPresentation", &["AsDescription", "AsCode"]),
        (
            "HierarchyType",
            &["HierarchyFoldersAndItems", "HierarchyOfItems"],
        ),
        ("EditType", &["InDialog", "InList", "BothWays"]),
        ("WriteMode", &["Independent", "RecorderSubordinate"]),
        (
            "InformationRegisterPeriodicity",
            &[
                "Nonperiodical",
                "Second",
                "Day",
                "Month",
                "Quarter",
                "Year",
                "RecorderPosition",
            ],
        ),
        ("RegisterType", &["Balance", "Turnovers"]),
        (
            "ReturnValuesReuse",
            &["DontUse", "DuringRequest", "DuringSession"],
        ),
        ("ReuseSessions", &["DontUse", "AutoUse"]),
        ("FillChecking", &["DontCheck", "ShowError"]),
        (
            "Indexing",
            &["DontIndex", "Index", "IndexWithAdditionalOrder"],
        ),
        ("DataHistory", &["Use", "DontUse"]),
        (
            "DependenceOnCalculationTypes",
            &["DontUse", "OnActionPeriod"],
        ),
        (
            "SubordinationUse",
            &["ToFolders", "ToFoldersAndItems", "ToItems"],
        ),
        (
            "CatalogCodeSeries",
            &[
                "WholeCatalog",
                "WithinOwnerSubordination",
                "WithinSubordination",
            ],
        ),
        (
            "ChartOfAccountsCodeSeries",
            &["WholeChartOfAccounts", "WithinSubordination"],
        ),
        (
            "CharacteristicTypeCodeSeries",
            &["WholeCharacteristicKind", "WithinSubordination"],
        ),
        ("ChoiceMode", &["BothWays", "FromForm", "QuickChoice"]),
        (
            "DocumentNumberPeriodicity",
            &["Day", "Month", "Nonperiodical", "Quarter", "Year"],
        ),
        (
            "BusinessProcessNumberPeriodicity",
            &["Day", "Month", "Nonperiodical", "Quarter", "Year"],
        ),
        (
            "CalculationRegisterPeriodicity",
            &["Day", "Month", "Quarter", "Year"],
        ),
        (
            "PredefinedDataUpdate",
            &["Auto", "AutoUpdate", "DontAutoUpdate"],
        ),
        (
            "HTTPMethod",
            &[
                "Any",
                "CONNECT",
                "COPY",
                "DELETE",
                "GET",
                "HEAD",
                "LOCK",
                "MERGE",
                "MKCOL",
                "MOVE",
                "OPTIONS",
                "PATCH",
                "POST",
                "PROPFIND",
                "PROPPATCH",
                "PUT",
                "TRACE",
                "UNLOCK",
            ],
        ),
        ("TransferDirection", &["In", "InOut", "Out"]),
    ]
}
pub fn meta_validate_valid_http_methods() -> &'static [&'static str] {
    // Single authority: the HTTPMethod entry of the 8.3.27 property enum table.
    meta_validate_property_enum("HTTPMethod")
}
pub fn meta_validate_property_enum(property: &str) -> &'static [&'static str] {
    meta_validate_property_values()
        .iter()
        .find(|(name, _)| *name == property)
        .map(|(_, allowed)| *allowed)
        .unwrap_or_else(|| panic!("the 8.3.27 property enum table defines {property}"))
}

/// Reading proves the known structure; compatibility is reported by check.
/// Publication requires the active export profile as well as that structure.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MetadataAdmission {
    Read,
    Writable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedChildKind {
    Form,
    Template,
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedOwnerChild {
    pub child: MetadataAddress,
    pub kind: ClosedChildKind,
    pub has_inline_descriptor: bool,
}

pub fn validate_exact_owner_identity(
    artifact: roxmltree::Node<'_, '_>,
    owner: &MetadataAddress,
) -> Result<(), String> {
    let properties = exact_mdclasses_children(artifact, "Properties", "owner descriptor")?;
    let [properties] = properties.as_slice() else {
        return Err("owner descriptor must contain exactly one Properties".into());
    };
    let names = exact_mdclasses_children(*properties, "Name", "owner descriptor")?;
    let [name] = names.as_slice() else {
        return Err("owner descriptor must contain exactly one Name".into());
    };
    let name = unica_xml_format::read::direct_text(*name)
        .trim()
        .to_string();
    if name.is_empty() {
        return Err("owner descriptor Name is empty".into());
    }
    let actual = format!("{}.{name}", artifact.tag_name().name());
    if actual != owner.as_str() {
        return Err(format!(
            "owner descriptor identity {actual} does not match closed target {owner}"
        ));
    }
    Ok(())
}

pub fn parse_owner_child_artifact(
    artifact: roxmltree::Node<'_, '_>,
    owner: &MetadataAddress,
) -> Result<ClosedOwnerChild, String> {
    if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err("owner child entry is outside the MDClasses namespace".into());
    }
    let (kind, child_kind) = match artifact.tag_name().name() {
        "Form" => (ClosedChildKind::Form, "Form"),
        "Template" => (ClosedChildKind::Template, "Template"),
        "Command" => (ClosedChildKind::Command, "Command"),
        _ => return Err("owner child entry is not a closed physical child kind".into()),
    };
    let properties =
        exact_mdclasses_children(artifact, "Properties", &format!("{child_kind} descriptor"))?;
    let child_objects = exact_mdclasses_children(
        artifact,
        "ChildObjects",
        &format!("{child_kind} descriptor"),
    )?;
    if child_objects.len() > 1 {
        return Err(format!(
            "{child_kind} owner entry must contain at most one ChildObjects"
        ));
    }
    let name = match properties.as_slice() {
        [] => {
            if artifact.children().any(|node| node.is_element()) {
                return Err(format!(
                    "{child_kind} owner entry has elements but no exact Properties"
                ));
            }
            unica_xml_format::read::direct_text(artifact)
                .trim()
                .to_string()
        }
        [properties] => {
            let template_types = exact_mdclasses_children(
                *properties,
                "TemplateType",
                &format!("{child_kind} descriptor"),
            )?;
            if template_types.len() > 1 {
                return Err(format!(
                    "{child_kind} owner entry must contain at most one TemplateType"
                ));
            }
            exact_child_name(*properties, child_kind)?
        }
        _ => {
            return Err(format!(
                "{child_kind} owner entry must contain at most one Properties"
            ))
        }
    };
    if name.is_empty() {
        return Err(format!("{child_kind} owner entry name is empty"));
    }
    let child = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner}.{child_kind}.{name}"),
    )
    .map_err(|_| "owner child identity is not representable".to_string())?;
    Ok(ClosedOwnerChild {
        child,
        kind,
        has_inline_descriptor: !properties.is_empty(),
    })
}

/// The ranges use the exact owner_source string supplied by the planner.
/// Any BOM removed by the metadata parser is translated back into that basis.
pub struct TemplateRemovalIdentity {
    pub registration: std::ops::Range<usize>,
    pub reference_edits: Vec<(std::ops::Range<usize>, String)>,
    pub template_type: Option<String>,
}

/// Identity for whole-template removal. The content kind stays opaque: the
/// editable-template enum deliberately does not include every platform kind.
pub fn validate_template_removal_identity(
    owner_source: &str,
    descriptor_bytes: &[u8],
    owner: &MetadataAddress,
    name: &str,
    allow_inherited: bool,
) -> Result<TemplateRemovalIdentity, String> {
    let expected = format!("{owner}.Template.{name}");
    let (parsed_owner, owner_document) =
        unica_xml_format::read::parse_metadata_image(owner_source.as_bytes())?;
    let owner_offset = owner_source.len() - parsed_owner.len();
    let owner_artifact = exact_metadata_artifact(&owner_document, MetadataAdmission::Writable)?;
    validate_exact_owner_identity(owner_artifact, owner)?;
    let containers = exact_mdclasses_children(owner_artifact, "ChildObjects", "owner descriptor")?;
    let [children] = containers.as_slice() else {
        return Err("template removal requires one immediate owner ChildObjects".into());
    };
    let mut registration = None;
    for node in children.children().filter(|node| {
        node.is_element() && matches!(node.tag_name().name(), "Form" | "Template" | "Command")
    }) {
        let entry = parse_owner_child_artifact(node, owner)?;
        if entry.child.as_str() == expected {
            if registration.is_some() {
                return Err(
                    "template removal requires exactly one immediate owner registration".into(),
                );
            }
            let range = node.range();
            registration = Some(range.start + owner_offset..range.end + owner_offset);
        }
    }
    let registration =
        registration.ok_or_else(|| "template owner registration is missing".to_string())?;
    let properties = template_removal_exact_properties(owner_artifact)?;
    let mut reference_edits = Vec::new();
    for property in ["MainDataCompositionSchema", "DefaultTemplate"] {
        // Foreign namesakes are not metadata references and remain unchanged.
        let references = properties
            .children()
            .filter(|node| node.has_tag_name((MD_CLASSES_NS, property)))
            .collect::<Vec<_>>();
        if references.len() > 1 {
            return Err(format!("owner descriptor has duplicate {property}"));
        }
        for node in references {
            if node.children().any(|child| child.is_element()) {
                return Err(format!("owner {property} reference must be a scalar"));
            }
            if unica_xml_format::read::direct_text(node).trim() != expected {
                continue;
            }
            let range = node.range();
            let range = range.start + owner_offset..range.end + owner_offset;
            let canonical = format!("<{property}>{expected}</{property}>");
            if owner_source[range.clone()] == canonical {
                // Preserve the existing writer's empty spelling for its exact
                // simple representation, only inside this confirmed node.
                reference_edits.push((range, format!("<{property}/>")));
            } else {
                // Keep prefixes, attributes, comments and surrounding tokens;
                // remove only the text nodes that form this scalar reference.
                for text in node.children().filter(roxmltree::Node::is_text) {
                    let range = text.range();
                    reference_edits.push((
                        range.start + owner_offset..range.end + owner_offset,
                        String::new(),
                    ));
                }
            }
        }
    }
    let (_, document) = unica_xml_format::read::parse_metadata_image(descriptor_bytes)?;
    let artifact = exact_metadata_artifact(&document, MetadataAdmission::Writable)?;
    if artifact.tag_name().name() != "Template" {
        return Err("template removal descriptor is not a Template".into());
    }
    if !artifact.attribute("uuid").is_some_and(is_guid) {
        return Err("template removal descriptor has a missing or invalid uuid".into());
    }
    let properties = exact_mdclasses_children(artifact, "Properties", "Template descriptor")?;
    let [properties] = properties.as_slice() else {
        return Err("Template descriptor must contain exactly one Properties".into());
    };
    let names = exact_mdclasses_children(*properties, "Name", "Template descriptor")?;
    let [name_node] = names.as_slice() else {
        return Err("Template descriptor must contain exactly one Name".into());
    };
    if name_node.children().any(|child| child.is_element())
        || exact_child_name(*properties, "Template")? != name
    {
        return Err("Template descriptor Name does not match the selected target".into());
    }
    let types = exact_mdclasses_children(*properties, "TemplateType", "Template descriptor")?;
    let template_type = match types.as_slice() {
        [] if allow_inherited => None,
        [template_type] => {
            if template_type.children().any(|child| child.is_element()) {
                return Err("TemplateType must be a scalar".into());
            }
            let kind = unica_xml_format::read::direct_text(*template_type)
                .trim()
                .to_string();
            if kind.is_empty() {
                return Err("TemplateType is empty".into());
            }
            Some(kind)
        }
        _ => {
            return Err(
                "Template descriptor must contain exactly one TemplateType unless inherited".into(),
            )
        }
    };
    Ok(TemplateRemovalIdentity {
        registration,
        reference_edits,
        template_type,
    })
}

pub fn template_removal_exact_properties<'a, 'input>(
    artifact: roxmltree::Node<'a, 'input>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let properties =
        exact_mdclasses_children(artifact, "Properties", "template removal descriptor")?;
    let [properties] = properties.as_slice() else {
        return Err("template removal descriptor must contain exactly one Properties".into());
    };
    Ok(*properties)
}

pub fn template_removal_has_exact_parent(bytes: &[u8]) -> Result<bool, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document, MetadataAdmission::Writable)?;
    let properties = template_removal_exact_properties(artifact)?;
    let parents = exact_mdclasses_children(
        properties,
        "ExtendedConfigurationObject",
        "template removal descriptor",
    )?;
    let parent = match parents.as_slice() {
        [] => return Ok(false),
        [parent] => *parent,
        _ => {
            return Err(
                "template removal descriptor has duplicate ExtendedConfigurationObject".into(),
            )
        }
    };
    if parent.children().any(|child| child.is_element())
        || !is_guid(unica_xml_format::read::direct_text(parent).trim())
    {
        return Err("template removal descriptor parent must be a scalar UUID".into());
    }
    Ok(true)
}

/// Exact write admission for the no-payload inherited exception. The caller
/// separately proves Extension source kind and retained payload-root absence.
pub fn template_removal_has_inherited_owners(
    configuration_bytes: &[u8],
    owner_bytes: &[u8],
    descriptor_bytes: &[u8],
) -> Result<bool, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(configuration_bytes)?;
    let configuration = exact_metadata_artifact(&document, MetadataAdmission::Writable)?;
    if configuration.tag_name().name() != "Configuration" {
        return Err("template removal extension root is not Configuration".into());
    }
    let properties = template_removal_exact_properties(configuration)?;
    let purposes = exact_mdclasses_children(
        properties,
        "ConfigurationExtensionPurpose",
        "template removal extension root",
    )?;
    let purpose = match purposes.as_slice() {
        [] => return Ok(false),
        [purpose] => *purpose,
        _ => {
            return Err(
                "template removal extension root has duplicate ConfigurationExtensionPurpose"
                    .into(),
            )
        }
    };
    if purpose.children().any(|child| child.is_element())
        || unica_xml_format::read::direct_text(purpose)
            .trim()
            .is_empty()
    {
        return Err("ConfigurationExtensionPurpose must be a nonempty scalar".into());
    }
    let owner_parent = template_removal_has_exact_parent(owner_bytes)?;
    let child_parent = template_removal_has_exact_parent(descriptor_bytes)?;
    Ok(owner_parent && child_parent)
}

/// The primary path of a platform kind whose footprint Unica knows. A truly
/// unknown kind remains opaque; a filename alone cannot establish XML semantics.
pub fn template_removal_primary_path(template_type: &str) -> Option<&'static str> {
    if template_type == "GraphicalSchema" {
        Some("Ext/Template.xml")
    } else {
        unica_metadata::validation_subject::MetadataTemplateType::from_descriptor_value(
            template_type,
        )
        .map(crate::edit::typed_template_primary_path)
    }
}

/// Validate only the known primary resource, not unknown opaque members.
pub fn validate_template_removal_primary_image(
    template_type: &str,
    bytes: &[u8],
) -> Result<(), String> {
    if template_type == "GraphicalSchema" {
        return validate_exact_version_xml(
            bytes,
            PlatformXmlRootExpectation::new("http://v8.1c.ru/8.3/xcf/scheme", "GraphicalSchema"),
            "graphical template",
        );
    }
    if let Some(kind) =
        unica_metadata::validation_subject::MetadataTemplateType::from_descriptor_value(
            template_type,
        )
    {
        validate_template_resource(
            kind,
            unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
            bytes,
        )
    } else {
        Ok(())
    }
}

pub fn exact_metadata_artifact<'a, 'input>(
    document: &'a Document<'input>,
    admission: MetadataAdmission,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
        || (admission == MetadataAdmission::Writable
            && root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format))
    {
        return Err("metadata descriptor root does not match the active MDClasses profile".into());
    }
    let artifacts = root
        .children()
        .filter(roxmltree::Node::is_element)
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err("metadata descriptor must contain exactly one element artifact".into());
    };
    if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err("metadata artifact is outside the MDClasses namespace".into());
    }
    Ok(*artifact)
}

pub fn exact_child_name(properties: roxmltree::Node<'_, '_>, kind: &str) -> Result<String, String> {
    let names = exact_mdclasses_children(properties, "Name", &format!("{kind} descriptor"))?;
    let [name] = names.as_slice() else {
        return Err(format!("{kind} descriptor must contain exactly one Name"));
    };
    let name = unica_xml_format::read::direct_text(*name)
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(format!("{kind} descriptor Name is empty"));
    }
    Ok(name)
}

pub fn exact_mdclasses_children<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
    role: &str,
) -> Result<Vec<roxmltree::Node<'a, 'input>>, String> {
    let named = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == local_name)
        .collect::<Vec<_>>();
    if named
        .iter()
        .any(|child| child.tag_name().namespace() != Some(MD_CLASSES_NS))
    {
        return Err(format!(
            "{role} contains {local_name} outside the MDClasses namespace"
        ));
    }
    Ok(named)
}

pub fn validate_template_resource(
    template_type: unica_metadata::validation_subject::MetadataTemplateType,
    part: unica_metadata::validation_subject::MetadataTemplateResourcePart,
    bytes: &[u8],
) -> Result<(), String> {
    match (template_type, part) {
        (unica_metadata::validation_subject::MetadataTemplateType::SpreadsheetDocument, unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary) => {
            validate_versionless_xml(bytes, MXL_ROOT, "spreadsheet template")
        }
        (unica_metadata::validation_subject::MetadataTemplateType::DataCompositionSchema, unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary) => {
            validate_versionless_xml(bytes, DCS_ROOT, "data composition schema template")
        }
        (unica_metadata::validation_subject::MetadataTemplateType::TextDocument, unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary) => {
            std::str::from_utf8(bytes)
                .map(|_| ())
                .map_err(|error| format!("text template is not UTF-8: {error}"))
        }
        (
            unica_metadata::validation_subject::MetadataTemplateType::BinaryData
            | unica_metadata::validation_subject::MetadataTemplateType::AddIn
            | unica_metadata::validation_subject::MetadataTemplateType::DataCompositionAppearanceTemplate,
            unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
        ) => Ok(()),
        (unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument, unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary) => {
            html_template_page_names(bytes).map(|_| ())
        }
        (unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument, unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage) => {
            std::str::from_utf8(bytes)
                .map(|_| ())
                .map_err(|error| format!("HTML template page is not UTF-8: {error}"))
        }
        _ => Err("template resource part does not match its TemplateType".to_string()),
    }
}

pub fn html_template_page_names(bytes: &[u8]) -> Result<Vec<String>, String> {
    const NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/extrnprops";
    validate_exact_version_xml(
        bytes,
        PlatformXmlRootExpectation::new(NAMESPACE, "Help"),
        "HTML template descriptor",
    )?;
    let (_, document) = parse_typed_payload_xml(bytes, "HTML template descriptor")?;
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    for node in document.root_element().children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(NAMESPACE)
            && node.tag_name().name() == "Page"
    }) {
        let page = node.text().map(str::trim).unwrap_or_default();
        if page.is_empty()
            || page
                .chars()
                .any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        {
            return Err("HTML template descriptor contains an invalid language page".to_string());
        }
        if !seen.insert(page.to_string()) {
            return Err("HTML template descriptor contains a duplicate language page".to_string());
        }
        pages.push(page.to_string());
    }
    if pages.is_empty() {
        return Err("HTML template descriptor must declare at least one language page".to_string());
    }
    Ok(pages)
}

pub fn validate_versionless_xml(
    bytes: &[u8],
    expected: PlatformXmlRootExpectation,
    role: &str,
) -> Result<(), String> {
    let (source, document) = parse_typed_payload_xml(bytes, role)?;
    let root = document.root_element();
    require_exact_payload_root(root, expected, role)?;
    if root_version_literal(source, root).is_some() {
        return Err(format!(
            "{role} must use its versionless platform XML format"
        ));
    }
    Ok(())
}

pub fn validate_exact_version_xml(
    bytes: &[u8],
    expected: PlatformXmlRootExpectation,
    role: &str,
) -> Result<(), String> {
    let (source, document) = parse_typed_payload_xml(bytes, role)?;
    let root = document.root_element();
    require_exact_payload_root(root, expected, role)?;
    if root_version_literal(source, root).as_deref() != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err(format!(
            "{role} must use export format {}",
            ACTIVE_FORMAT_PROFILE.export_format
        ));
    }
    Ok(())
}

pub fn parse_typed_payload_xml<'a>(
    bytes: &'a [u8],
    role: &str,
) -> Result<(&'a str, Document<'a>), String> {
    let text =
        std::str::from_utf8(bytes).map_err(|error| format!("{role} is not UTF-8: {error}"))?;
    let source = text.trim_start_matches('\u{feff}');
    require_utf8_xml_declaration(source, role)?;
    let document =
        Document::parse(source).map_err(|error| format!("{role} is malformed: {error}"))?;
    Ok((source, document))
}

pub fn require_utf8_xml_declaration(source: &str, role: &str) -> Result<(), String> {
    let Some(declaration) = source
        .strip_prefix("<?xml")
        .and_then(|rest| rest.split_once("?>").map(|(declaration, _)| declaration))
    else {
        return Ok(());
    };
    let Some((_, raw_value)) = declaration.split_once("encoding") else {
        return Ok(());
    };
    let raw_value = raw_value.trim_start();
    let Some(raw_value) = raw_value.strip_prefix('=') else {
        return Err(format!("{role} has a malformed XML encoding declaration"));
    };
    let raw_value = raw_value.trim_start();
    let Some(quote) = raw_value
        .chars()
        .next()
        .filter(|value| matches!(value, '\'' | '"'))
    else {
        return Err(format!("{role} has a malformed XML encoding declaration"));
    };
    let raw_value = &raw_value[quote.len_utf8()..];
    let Some((encoding, _)) = raw_value.split_once(quote) else {
        return Err(format!("{role} has a malformed XML encoding declaration"));
    };
    if !encoding.eq_ignore_ascii_case("UTF-8") {
        return Err(format!("{role} must declare UTF-8 encoding"));
    }
    Ok(())
}

pub fn require_exact_payload_root(
    root: roxmltree::Node<'_, '_>,
    expected: PlatformXmlRootExpectation,
    role: &str,
) -> Result<(), String> {
    if root.tag_name().namespace() != Some(expected.namespace)
        || root.tag_name().name() != expected.local_name
    {
        return Err(format!(
            "{role} root does not match its declared TemplateType"
        ));
    }
    Ok(())
}

impl ClosedChildKind {
    pub fn matches(
        self,
        profile: unica_metadata::validation_subject::MetadataChildProfile,
    ) -> bool {
        matches!(
            (self, profile),
            (
                Self::Form,
                unica_metadata::validation_subject::MetadataChildProfile::Form
                    | unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm
            ) | (
                Self::Template,
                unica_metadata::validation_subject::MetadataChildProfile::Template(_)
            ) | (
                Self::Command,
                unica_metadata::validation_subject::MetadataChildProfile::Command
            )
        )
    }
}

use std::collections::BTreeMap;
use std::path::Path;
use unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20;
use unica_bsl_parser::{
    exact_bsl_method_facts, exported_bsl_procedure_matches, BslProcedureRequirement,
};
use unica_metadata::{
    metadata_kind_collections, validate_event_subscription_binding, EventBindingError,
    EventHandlerFacts, EventHandlerMethodKind, EventSourceClass, MetaCollection, MetaDiagnostic,
    MetaDiagnosticCode, MetaDiagnosticSeverity, MetaEventSource, MetaValidationData,
    MetaValidationStatus, MetadataKind,
};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_designer::format_profile::{classify_root_version, FormatCompatibility};
use unica_source_designer::read::{path_exists, path_is_dir, read_utf8_sig};
#[derive(Clone, Copy)]
pub struct MetadataValidationSource<'a> {
    pub config_dir: &'a Path,
    pub reader: &'a dyn ProjectDiscoveryRead,
}
pub struct MetaValidationReporter {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub findings: Vec<MetaValidationFinding>,
    pub stopped: bool,
    pub max_errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaValidationFinding {
    pub code: MetaDiagnosticCode,
    pub field: String,
    pub language: Option<String>,
    pub message: String,
}

impl MetaValidationFinding {
    pub fn into_diagnostic(
        self,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    ) -> MetaDiagnostic {
        let diagnostic = MetaDiagnostic::warning(self.code, self.message)
            .with_metadata_path(subject.target.clone())
            .with_field(self.field);
        match self.language {
            Some(language) => diagnostic.with_language(language),
            None => diagnostic,
        }
    }
}

pub struct MetaValidationRun {
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub findings: Vec<MetaValidationFinding>,
}

#[derive(Debug, Clone)]
pub struct MetaValidationOptions {
    pub max_errors: usize,
}

pub struct MetaValidationReferenceInputs<'a> {
    pub config_dir: Option<MetadataValidationSource<'a>>,
    pub language_codes: Vec<String>,
    pub proof_subject: Option<&'a unica_metadata::validation_subject::MetadataValidationSubject>,
}

/// Required immutable Form grammar binding; validated only at the existing FormContent phase.
#[derive(Clone, Copy)]
pub struct FormRootValidator(
    pub for<'a, 'input> fn(roxmltree::Node<'a, 'input>) -> Result<(), String>,
);
pub struct MetadataValidator {
    form_root: FormRootValidator,
}

impl MetadataValidator {
    pub const fn new(form_root: FormRootValidator) -> Self {
        Self { form_root }
    }
    pub fn validate(
        &self,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        _context: &WorkspaceContext,
    ) -> unica_metadata::validation_subject::MetadataValidationResult {
        self.evaluate(
            subject,
            &MetaValidationOptions { max_errors: 30 },
            MetadataAdmission::Writable,
        )
    }

    pub fn validate_complete_read(
        &self,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        _context: &WorkspaceContext,
    ) -> unica_metadata::validation_subject::MetadataValidationResult {
        let mut result = self.evaluate(
            subject,
            &MetaValidationOptions { max_errors: 30 },
            MetadataAdmission::Read,
        );
        let mut completeness = complete_read_proof_diagnostics(subject);
        if let unica_metadata::validation_subject::MetadataEvidenceAvailability::Unavailable(
            diagnostics,
        ) = &subject.registrar_evidence
        {
            completeness.extend(diagnostics.clone());
        }
        if let Some(unica_metadata::validation_subject::MetadataSubsystemEvidence::Unavailable(
            diagnostics,
        )) = &subject.subsystem_evidence
        {
            completeness.extend(diagnostics.clone());
        }
        if !completeness.is_empty() {
            result.status = MetaValidationStatus::Failed;
            result.diagnostics.extend(completeness);
        }
        result
    }

    pub fn evaluate(
        &self,
        subject: &unica_metadata::validation_subject::MetadataValidationSubject,
        options: &MetaValidationOptions,
        admission: MetadataAdmission,
    ) -> unica_metadata::validation_subject::MetadataValidationResult {
        let mut diagnostics = Vec::new();
        let descriptor_indices = subject
            .resources
            .iter()
            .enumerate()
            .filter_map(|(index, resource)| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Descriptor
                )
                .then_some(index)
            })
            .collect::<Vec<_>>();

        if descriptor_indices.len() > 1 {
            diagnostics.push(validation_diagnostic(
                subject,
                "resources",
                "validation subject contains more than one descriptor image",
            ));
        }

        let mut registrations = Vec::new();
        let mut registered_languages = Vec::new();
        let mut language_images = BTreeMap::new();
        for (index, resource) in subject.resources.iter().enumerate() {
            match &resource.role {
                unica_metadata::validation_subject::MetadataResourceRole::Descriptor
                | unica_metadata::validation_subject::MetadataResourceRole::Registration
                | unica_metadata::validation_subject::MetadataResourceRole::Dependency { .. }
                | unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                | unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                | unica_metadata::validation_subject::MetadataResourceRole::Command { .. } => {
                    if let Err(error) =
                        unica_xml_format::read::parse_metadata_image(&resource.bytes)
                    {
                        diagnostics.push(provider_diagnostic(subject, index, error));
                        continue;
                    }
                }
                unica_metadata::validation_subject::MetadataResourceRole::Module { .. } => {
                    if let Err(error) = std::str::from_utf8(&resource.bytes) {
                        diagnostics.push(provider_diagnostic(
                            subject,
                            index,
                            format!("module image is not UTF-8: {error}"),
                        ));
                        continue;
                    }
                }
                unica_metadata::validation_subject::MetadataResourceRole::AuxiliaryXml {
                    kind,
                    ..
                } => {
                    if let Err(error) = validate_auxiliary_xml(*kind, &resource.bytes) {
                        diagnostics.push(provider_diagnostic(subject, index, error));
                        continue;
                    }
                }
                unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                    child,
                    kind,
                    ..
                } => {
                    if let Err(error) =
                        validate_child_resource(self.form_root, *kind, &resource.bytes)
                    {
                        diagnostics.push(child_resource_diagnostic(child, index, error));
                        continue;
                    }
                }
            }

            if matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Registration
            ) {
                match crate::validation_context::inspect_metadata_registration_image(
                    &resource.bytes,
                ) {
                    Ok(registration) => {
                        registrations.extend(registration.registrations);
                        registered_languages.extend(registration.registered_languages);
                    }
                    Err(error) => diagnostics.push(validation_diagnostic(
                        subject,
                        &format!("resources[{index}].bytes"),
                        error,
                    )),
                }
            } else if let unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                target,
            } = &resource.role
            {
                match unica_source_designer::language::inspect_language_image(&resource.bytes) {
                    Ok(Some((name, code))) => {
                        if target.as_str() != format!("Language.{name}") {
                            diagnostics.push(provider_diagnostic(
                                subject,
                                index,
                                format!(
                                    "dependency image identity Language.{name} does not match declared target {target}"
                                ),
                            ));
                        }
                        language_images.insert(name, code);
                    }
                    Ok(None) => match metadata_image_reference(&resource.bytes) {
                        Ok(actual) => {
                            if target.as_str() != actual {
                                diagnostics.push(provider_diagnostic(
                                    subject,
                                    index,
                                    format!(
                                        "dependency image identity {actual} does not match declared target {target}"
                                    ),
                                ));
                            }
                        }
                        Err(error) => diagnostics.push(provider_diagnostic(subject, index, error)),
                    },
                    Err(error) => diagnostics.push(provider_diagnostic(subject, index, error)),
                }
            }
        }

        validate_html_template_page_registrations(
            subject,
            &registered_languages,
            &language_images,
            &mut diagnostics,
        );
        validate_child_footprints(subject, &mut diagnostics, admission);

        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable)
        {
            return failed_validation(diagnostics);
        }

        let (target_type, target_name) = subject_target_identity(subject);
        if let Some(descriptor_index) = descriptor_indices.first().copied() {
            let descriptor = &subject.resources[descriptor_index];
            match crate::validation_context::inspect_metadata_image_identity(&descriptor.bytes) {
                Ok(identity) => {
                    if identity.object_type != target_type || identity.object_name != target_name {
                        diagnostics.push(validation_diagnostic(
                            subject,
                            &format!("resources[{descriptor_index}].bytes"),
                            format!(
                                "descriptor identity {}.{} does not match target {}",
                                identity.object_type, identity.object_name, subject.target
                            ),
                        ));
                    }
                }
                Err(error) => {
                    let closed_owner_namespace_error = matches!(
                        &error,
                        crate::validation_context::MetaValidationImageIdentityError::ForeignProperties(_)
                            | crate::validation_context::MetaValidationImageIdentityError::ForeignName(_)
                    );
                    let message = match &error {
                        crate::validation_context::MetaValidationImageIdentityError::ForeignProperties(_) => format!(
                            "owner descriptor contains Properties outside the MDClasses namespace: {}",
                            subject.target
                        ),
                        crate::validation_context::MetaValidationImageIdentityError::ForeignName(_) => format!(
                            "owner descriptor contains Name outside the MDClasses namespace: {}",
                            subject.target
                        ),
                        _ => format!("metadata descriptor identity is invalid: {error}"),
                    };
                    if !error.is_structural() {
                        diagnostics.push(validation_diagnostic(
                            subject,
                            &format!("resources[{descriptor_index}].bytes"),
                            message,
                        ));
                    } else if closed_owner_namespace_error {
                        diagnostics.push(
                            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
                                .with_metadata_path(subject.target.clone())
                                .with_field("resources"),
                        );
                    } else {
                        diagnostics.push(provider_diagnostic(subject, descriptor_index, message));
                    }
                }
            }

            let has_registration_image = subject.resources.iter().any(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Registration
                )
            });
            let owns_itself = matches!(target_type, "ExternalReport" | "ExternalDataProcessor");
            if !owns_itself && !has_registration_image {
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ProviderUnavailable,
                        format!(
                            "owner registration image for {} is not available",
                            subject.target
                        ),
                    )
                    .with_metadata_path(subject.target.clone())
                    .with_field("resources"),
                );
            }

            let mut language_codes = Vec::new();
            let mut seen_codes = HashSet::new();
            let is_registered = registrations
                .iter()
                .any(|(kind, name)| kind == target_type && name == target_name);
            if crate::validation_context::meta_validate_types_with_list_presentation()
                .contains(&target_type)
            {
                if owns_itself {
                    // External descriptors own themselves and have no Configuration image.
                } else if is_registered && !registered_languages.is_empty() {
                    for language in &registered_languages {
                        match language_images.get(language) {
                            Some(code) if seen_codes.insert(code.clone()) => {
                                language_codes.push(code.clone())
                            }
                            Some(_) => {}
                            None => diagnostics.push(
                                MetaDiagnostic::error(
                                    MetaDiagnosticCode::ProviderUnavailable,
                                    format!(
                                        "registered language image `{language}` is not available"
                                    ),
                                )
                                .with_metadata_path(subject.target.clone())
                                .with_field("resources"),
                            ),
                        }
                    }
                }
            }

            if diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
            {
                return failed_validation(diagnostics);
            }

            let inputs = MetaValidationReferenceInputs {
                config_dir: None,
                language_codes,
                proof_subject: Some(subject),
            };
            match meta_validate_source(&descriptor.bytes, options, &inputs) {
                Ok(run) => {
                    diagnostics.extend(run.errors.iter().map(|error| {
                        let field = if error.contains("is not registered in the owner image") {
                            subject
                                .resources
                                .iter()
                                .position(|resource| {
                                    matches!(resource.role, unica_metadata::validation_subject::MetadataResourceRole::Registration)
                                })
                                .map(|index| format!("resources[{index}].bytes"))
                                .unwrap_or_else(|| "resources".to_string())
                        } else if error.contains("no registered language profile") {
                            "resources".to_string()
                        } else {
                            validation_field_for_legacy_error(error).to_string()
                        };
                        validation_diagnostic(subject, &field, error.trim_start_matches("[ERROR] "))
                    }));
                    diagnostics.extend(run.warnings.iter().map(|warning| {
                        validation_warning(
                            subject,
                            validation_field_for_legacy_error(warning),
                            warning.trim_start_matches("[WARN]  "),
                        )
                    }));
                    diagnostics.extend(
                        run.findings
                            .into_iter()
                            .map(|finding| finding.into_diagnostic(subject)),
                    );
                }
                Err(error) => {
                    diagnostics.push(provider_diagnostic(subject, descriptor_index, error))
                }
            }
            if target_type == "EventSubscription" {
                diagnostics.extend(validate_event_subscription_binding_subject(
                    subject,
                    &descriptor.bytes,
                ));
            }
        } else {
            for (index, resource) in subject.resources.iter().enumerate() {
                match resource.role {
                    unica_metadata::validation_subject::MetadataResourceRole::Registration => {
                        if crate::validation_context::inspect_metadata_registration_image(
                            &resource.bytes,
                        )
                        .is_ok_and(|registration| {
                            registration
                                .registrations
                                .iter()
                                .any(|(kind, name)| kind == target_type && name == target_name)
                        }) {
                            diagnostics.push(validation_diagnostic(
                                subject,
                                &format!("resources[{index}].bytes"),
                                format!("surviving registration still contains {}", subject.target),
                            ));
                        }
                    }
                    unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                        ..
                    } => {
                        if image_contains_reference(&resource.bytes, subject.target.as_str()) {
                            diagnostics.push(validation_diagnostic(
                                subject,
                                &format!("resources[{index}].bytes"),
                                format!("surviving reference still targets {}", subject.target),
                            ));
                        }
                    }
                    unica_metadata::validation_subject::MetadataResourceRole::Descriptor
                    | unica_metadata::validation_subject::MetadataResourceRole::Module { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Template {
                        ..
                    }
                    | unica_metadata::validation_subject::MetadataResourceRole::Command {
                        ..
                    }
                    | unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        ..
                    }
                    | unica_metadata::validation_subject::MetadataResourceRole::AuxiliaryXml {
                        ..
                    } => {}
                }
            }
        }

        let result = if diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && matches!(
                    diagnostic.code,
                    MetaDiagnosticCode::ValidationFailed | MetaDiagnosticCode::ProviderUnavailable
                )
        }) {
            failed_validation(diagnostics)
        } else {
            MetaValidationData {
                status: MetaValidationStatus::Passed,
                diagnostics,
            }
        };
        result
    }
}

pub fn complete_read_proof_diagnostics(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
) -> Vec<MetaDiagnostic> {
    let Some(descriptor) = subject.resources.iter().find(|resource| {
        matches!(
            resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Descriptor
        )
    }) else {
        return vec![complete_read_missing(
            subject,
            "metadata descriptor proof is unavailable",
        )];
    };
    let (_, document) = match unica_xml_format::read::parse_metadata_image(&descriptor.bytes) {
        Ok(parsed) => parsed,
        Err(error) => {
            return vec![complete_read_missing(
                subject,
                format!("metadata descriptor proof is invalid: {error}"),
            )]
        }
    };
    let object = match exact_metadata_artifact(&document, MetadataAdmission::Read) {
        Ok(object) => object,
        Err(error) => {
            return vec![complete_read_missing(
                subject,
                format!("metadata descriptor proof is invalid: {error}"),
            )]
        }
    };
    let object_type = object.tag_name().name();
    let properties = unica_xml_format::read::child_element(object, "Properties");
    let object_name = properties
        .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
        .unwrap_or_default();
    let mut diagnostics = Vec::new();

    for language in subject
        .resources
        .iter()
        .find(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Registration
            )
        })
        .and_then(|resource| {
            crate::validation_context::inspect_metadata_registration_image(&resource.bytes).ok()
        })
        .into_iter()
        .flat_map(|registration| registration.registered_languages)
    {
        let expected = format!("Language.{language}");
        if !has_dependency(subject, &expected) {
            diagnostics.push(complete_read_missing(
                subject,
                format!("registered language evidence `{language}` is unavailable"),
            ));
        }
    }

    if object_type == "Document" {
        for reference in properties
            .and_then(|node| unica_xml_format::read::child_element(node, "RegisterRecords"))
            .map(|node| unica_xml_format::read::child_elements(node, "Item"))
            .unwrap_or_default()
            .into_iter()
            .map(unica_xml_format::read::direct_text)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if !has_dependency(subject, &reference) {
                diagnostics.push(complete_read_missing(
                    subject,
                    format!("referenced metadata evidence `{reference}` is unavailable"),
                ));
            }
        }
    }

    if object_type == "EventSubscription" {
        if let Ok(sources) = crate::xml_model::meta_event_subscription_source_node(&document)
            .and_then(crate::xml_model::parse_meta_event_subscription_source)
        {
            let registration = subject.resources.iter().find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Registration
                )
            });
            for (index, source) in sources.sources.iter().enumerate() {
                let Some(target) = source.metadata_path() else {
                    continue;
                };
                let dependency = subject.resources.iter().find(|resource| {
                    matches!(
                        &resource.role,
                        unica_metadata::validation_subject::MetadataResourceRole::Dependency { target: dependency_target }
                            if dependency_target == target
                    )
                });
                let Some(dependency) = dependency else {
                    diagnostics.push(complete_read_source_missing(
                        subject,
                        index,
                        format!("EventSubscription Source dependency `{target}` is unavailable"),
                    ));
                    continue;
                };
                let (
                    registration_kind,
                    registration_name,
                    descriptor_kind,
                    descriptor_name,
                    generated_name,
                ) = match crate::validation_context::event_source_dependency_contract(target) {
                    Ok(contract) => contract,
                    _ => {
                        diagnostics.push(complete_read_source_invalid(
                                subject,
                                index,
                                format!(
                                    "EventSubscription Source dependency `{target}` is not an addressable metadata object"
                                ),
                            ));
                        continue;
                    }
                };
                let Some(registration) = registration else {
                    diagnostics.push(complete_read_source_missing(
                        subject,
                        index,
                        "EventSubscription Source registration evidence is unavailable",
                    ));
                    continue;
                };
                let generated_prefixes =
                    match crate::xml_model::event_source_generated_prefix(source) {
                        Ok(prefix) => vec![prefix.to_string()],
                        Err(message) => {
                            diagnostics.push(complete_read_source_invalid(subject, index, message));
                            continue;
                        }
                    };
                if let Err(message) = crate::validation_context::validate_event_source_registration(
                    &registration.bytes,
                    registration_kind,
                    registration_name,
                )
                .and_then(|()| {
                    crate::validation_context::validate_event_source_dependency_descriptor(
                        &dependency.bytes,
                        descriptor_kind,
                        descriptor_name,
                        generated_name.as_deref(),
                        &generated_prefixes,
                    )
                }) {
                    diagnostics.push(complete_read_source_invalid(subject, index, message));
                }
            }
        }
    }

    if matches!(object_type, "EventSubscription" | "ScheduledJob") {
        let property = if object_type == "EventSubscription" {
            "Handler"
        } else {
            "MethodName"
        };
        if let Some(reference) = properties
            .and_then(|node| unica_xml_format::read::child_direct_text(node, property))
            .filter(|value| !value.is_empty())
        {
            let parts = reference.split('.').collect::<Vec<_>>();
            let module_name = if parts.len() == 3 && parts[0] == "CommonModule" {
                Some(parts[1])
            } else if parts.len() == 2 {
                Some(parts[0])
            } else {
                None
            };
            if let Some(module_name) = module_name {
                let target = format!("CommonModule.{module_name}");
                if !has_dependency(subject, &target) {
                    diagnostics.push(complete_read_missing(
                        subject,
                        format!("referenced metadata evidence `{target}` is unavailable"),
                    ));
                }
                let has_module = subject.resources.iter().any(|resource| {
                    matches!(
                        &resource.role,
                        unica_metadata::validation_subject::MetadataResourceRole::Module { owner } if owner.as_str() == target
                    )
                });
                if !has_module {
                    diagnostics.push(complete_read_missing(
                        subject,
                        format!("referenced module evidence `{target}` is unavailable"),
                    ));
                }
            }
        }
    }

    let reads_registrar = matches!(
        object_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (object_type == "InformationRegister"
        && properties
            .and_then(|node| unica_xml_format::read::child_direct_text(node, "WriteMode"))
            .as_deref()
            == Some("RecorderSubordinate"));
    if reads_registrar
        && !object_name.is_empty()
        && matches!(
            subject.registrar_evidence,
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete
        )
    {
        let reference = format!("{object_type}.{object_name}");
        let has_registrar = subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Dependency { target }
                    if target.segments().next() == Some("Document")
            ) && crate::validation::document_registers(&resource.bytes, &reference)
        });
        if !has_registrar {
            diagnostics.push(complete_read_invalid(
                subject,
                format!("reverse registrar evidence for `{reference}` is inconsistent"),
            ));
        }
    }

    for child in object.descendants().filter(|node| {
        node.is_element()
            && matches!(
                node.tag_name().name(),
                "Attribute" | "TabularSection" | "Dimension" | "Resource" | "EnumValue" | "Column"
            )
    }) {
        let complete = unica_xml_format::read::child_element(child, "Properties")
            .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
            .is_some_and(|name| !name.trim().is_empty());
        if !complete {
            diagnostics.push(complete_read_invalid(
                subject,
                format!(
                    "{} child evidence has no complete Properties/Name",
                    child.tag_name().name()
                ),
            ));
        }
    }
    diagnostics
}

pub fn has_dependency(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    expected: &str,
) -> bool {
    subject.resources.iter().any(|resource| {
        matches!(
            &resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Dependency { target } if target.as_str() == expected
        )
    })
}

pub fn complete_read_missing(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(subject.target.clone())
        .with_field("resources")
}

pub fn complete_read_invalid(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field("resources")
}

pub fn complete_read_source_missing(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    index: usize,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(subject.target.clone())
        .with_field(format!("relations.source[{index}]"))
}

pub fn complete_read_source_invalid(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    index: usize,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field(format!("relations.source[{index}]"))
}

pub fn validate_auxiliary_xml(
    kind: unica_metadata::validation_subject::MetadataAuxiliaryXmlKind,
    bytes: &[u8],
) -> Result<(), String> {
    if let unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::PredefinedData(
        owner_kind,
    ) = kind
    {
        return crate::predefined::validate_predefined_image(owner_kind, bytes);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("auxiliary XML image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let document = Document::parse(text)
        .map_err(|error| format!("auxiliary XML image is malformed: {error}"))?;
    let root = document.root_element();
    let (expected_name, expected_namespace) = match kind {
        unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::ExchangePlanContent => {
            ("ExchangePlanContent", "http://v8.1c.ru/8.3/xcf/extrnprops")
        }
        unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::BusinessProcessFlowchart => {
            ("GraphicalSchema", "http://v8.1c.ru/8.3/xcf/scheme")
        }
        unica_metadata::validation_subject::MetadataAuxiliaryXmlKind::PredefinedData(_) => {
            unreachable!("handled above")
        }
    };
    if root.tag_name().name() != expected_name
        || root.tag_name().namespace() != Some(expected_namespace)
    {
        return Err(format!(
            "auxiliary XML root does not match declared {:?} resource",
            kind
        ));
    }
    if root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format) {
        return Err("auxiliary XML format version is unsupported".to_string());
    }
    Ok(())
}

pub fn validate_child_resource(
    form_root: FormRootValidator,
    kind: unica_metadata::validation_subject::MetadataChildResourceKind,
    bytes: &[u8],
) -> Result<(), String> {
    match kind {
        unica_metadata::validation_subject::MetadataChildResourceKind::FormContent => {
            let text = std::str::from_utf8(bytes)
                .map_err(|error| format!("form content is not UTF-8: {error}"))?;
            let source = text.trim_start_matches('\u{feff}');
            require_utf8_xml_declaration(source, "form content")?;
            let document = Document::parse(source)
                .map_err(|error| format!("form content is not valid XML: {error}"))?;
            let root = document.root_element();
            (form_root.0)(root)?;
            if root_version_literal(source, root).as_deref()
                != Some(ACTIVE_FORMAT_PROFILE.export_format)
            {
                return Err(format!(
                    "form content must use export format {}",
                    ACTIVE_FORMAT_PROFILE.export_format
                ));
            }
            Ok(())
        }
        unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
            template_type,
            part,
        } => validate_template_resource(template_type, part, bytes),
        unica_metadata::validation_subject::MetadataChildResourceKind::Module => {
            std::str::from_utf8(bytes)
                .map(|_| ())
                .map_err(|error| format!("child module is not UTF-8: {error}"))
        }
        // Retained payload is carried verbatim and never interpreted, so there
        // is no content contract to check — only the path shape, which
        // `validate_one_child_footprint` re-derives.
        unica_metadata::validation_subject::MetadataChildResourceKind::Retained
        | unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent => {
            Ok(())
        }
    }
}

pub fn validate_child_footprints(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
    admission: MetadataAdmission,
) {
    let expected = final_owner_child_graph(subject, diagnostics, admission);
    let descriptors = actual_child_descriptor_graph(subject, diagnostics, admission);

    for child in &expected {
        let matching_descriptors = descriptors
            .iter()
            .filter(|candidate| candidate.0.child == child.child)
            .collect::<Vec<_>>();
        // Three independent things can go wrong here, and a reader who only
        // sees "not backed by a descriptor" cannot tell which. Name the one
        // that actually failed.
        let descriptor = match matching_descriptors.as_slice() {
            [] => Err("final child has no descriptor among the read resources"),
            [_, _, ..] => Err("final child is backed by more than one descriptor"),
            [descriptor] if !child.kind.matches(descriptor.0.profile) => {
                Err("final child descriptor declares a different child kind than the owner graph")
            }
            [descriptor]
                if !child_descriptor_role_matches(
                    &descriptor.1,
                    &child.child,
                    descriptor.0.profile,
                ) =>
            {
                Err("final child descriptor is filed under a resource role naming another child")
            }
            [descriptor] => Ok(&descriptor.0),
        };
        let descriptor = match descriptor {
            Ok(descriptor) => descriptor,
            Err(message) => {
                diagnostics.push(template_resource_set_diagnostic(&child.child, message));
                continue;
            }
        };

        if child.has_inline_descriptor
            && matches!(
                child.kind,
                ClosedChildKind::Form | ClosedChildKind::Template
            )
        {
            let registration = child
                .child
                .segments()
                .skip(subject.target.segments().count())
                .collect::<Vec<_>>()
                .join(".");
            diagnostics.push(validation_warning(
                subject,
                &format!("childObjects.{registration}"),
                format!(
                    "Legacy inline registration {} exists alongside its separate descriptor; \
                     platform 8.3.27 may hang while importing this owner",
                    child.child
                ),
            ));
        }

        let footprints = subject
            .child_footprints
            .iter()
            .filter(|footprint| footprint.child == child.child)
            .collect::<Vec<_>>();
        if footprints.len() != 1 || footprints[0].profile != descriptor.profile {
            diagnostics.push(template_resource_set_diagnostic(
                &child.child,
                "final child is not backed by exactly one matching footprint evidence",
            ));
            continue;
        }

        validate_one_child_footprint(subject, descriptor, footprints[0], diagnostics);
    }

    for descriptor in &descriptors {
        if !expected.iter().any(|child| {
            child.child == descriptor.0.child && child.kind.matches(descriptor.0.profile)
        }) {
            diagnostics.push(template_resource_set_diagnostic(
                &descriptor.0.child,
                "child descriptor is orphaned from the final owner graph",
            ));
        }
    }
    for footprint in &subject.child_footprints {
        if !expected
            .iter()
            .any(|child| child.child == footprint.child && child.kind.matches(footprint.profile))
        {
            diagnostics.push(template_resource_set_diagnostic(
                &footprint.child,
                "child footprint evidence is orphaned from the final owner graph",
            ));
        }
    }
    for child in subject
        .resources
        .iter()
        .filter_map(|resource| match &resource.role {
            unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                child,
                ..
            } => Some(child),
            _ => None,
        })
    {
        if !expected.iter().any(|expected| &expected.child == child) {
            diagnostics.push(template_resource_set_diagnostic(
                child,
                "child resource is orphaned from the final owner graph",
            ));
        }
    }
}

pub fn validate_html_template_page_registrations(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    registered_languages: &[String],
    language_images: &BTreeMap<String, String>,
    diagnostics: &mut Vec<MetaDiagnostic>,
) {
    let html_descriptors = subject
        .resources
        .iter()
        .enumerate()
        .filter_map(|(index, resource)| match &resource.role {
            unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                child,
                kind:
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type: unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                    },
                ..
            } => Some((index, child, &resource.bytes)),
            _ => None,
        })
        .collect::<Vec<_>>();
    if html_descriptors.is_empty() || registered_languages.is_empty() {
        return;
    }
    let mut configured_codes = HashSet::new();
    let mut complete_evidence = true;
    for language in registered_languages {
        match language_images.get(language) {
            Some(code) => {
                configured_codes.insert(code.as_str());
            }
            None => {
                complete_evidence = false;
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ProviderUnavailable,
                        format!("registered language image `{language}` is not available"),
                    )
                    .with_metadata_path(subject.target.clone())
                    .with_field("resources"),
                );
            }
        }
    }
    if !complete_evidence {
        return;
    }
    for (index, child, bytes) in html_descriptors {
        let Ok(pages) = html_template_page_names(bytes) else {
            continue;
        };
        for page in pages {
            if !configured_codes.contains(page.as_str()) {
                diagnostics.push(child_resource_diagnostic(
                    child,
                    index,
                    format!("HTML language page `{page}` is not registered in the owner image"),
                ));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClosedChildDescriptor {
    pub child: MetadataAddress,
    pub profile: unica_metadata::validation_subject::MetadataChildProfile,
}

pub fn final_owner_child_graph(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
    admission: MetadataAdmission,
) -> Vec<ClosedOwnerChild> {
    let owners = subject
        .resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Descriptor
            )
        })
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        if !subject.child_footprints.is_empty() || subject.resources.iter().any(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Command { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::ChildResource { .. }
            )
        }) {
            diagnostics.push(template_resource_set_diagnostic(
                &subject.target,
                "child validation requires exactly one final owner descriptor",
            ));
        }
        return Vec::new();
    };
    let Ok(identity) = crate::validation_context::inspect_metadata_image_identity(&owner.bytes)
    else {
        return Vec::new();
    };
    if subject.target.as_str() != format!("{}.{}", identity.object_type, identity.object_name) {
        return Vec::new();
    }
    match parse_final_owner_children(&owner.bytes, &subject.target, admission) {
        Ok(children) => {
            let mut seen = HashSet::new();
            for child in &children {
                if !seen.insert(child.child.clone()) {
                    diagnostics.push(template_resource_set_diagnostic(
                        &child.child,
                        "final owner graph contains a duplicate child identity",
                    ));
                }
            }
            children
        }
        Err(message) => {
            diagnostics.push(template_resource_set_diagnostic(&subject.target, &message));
            Vec::new()
        }
    }
}

pub fn actual_child_descriptor_graph(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
    admission: MetadataAdmission,
) -> Vec<(
    ClosedChildDescriptor,
    unica_metadata::validation_subject::MetadataResourceRole,
)> {
    let mut descriptors = Vec::new();
    for resource in &subject.resources {
        if matches!(
            resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Descriptor
        ) {
            continue;
        }
        let declared_child = matches!(
            resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                | unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                | unica_metadata::validation_subject::MetadataResourceRole::Command { .. }
        );
        match parse_child_descriptor(&resource.bytes, &subject.target, admission) {
            Ok(Some(actual)) => descriptors.push((actual, resource.role.clone())),
            Ok(None) if declared_child => diagnostics.push(template_resource_set_diagnostic(
                &subject.target,
                "declared child descriptor bytes contain a different metadata kind",
            )),
            Err(message) if declared_child => diagnostics.push(template_resource_set_diagnostic(
                &subject.target,
                &format!("declared child descriptor is invalid: {message}"),
            )),
            Ok(None) | Err(_) => {}
        }
    }
    descriptors
}

pub fn parse_final_owner_children(
    bytes: &[u8],
    owner: &MetadataAddress,
    admission: MetadataAdmission,
) -> Result<Vec<ClosedOwnerChild>, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document, admission)?;
    validate_exact_owner_identity(artifact, owner)?;
    let child_objects = exact_mdclasses_children(artifact, "ChildObjects", "owner descriptor")?;
    let children = match child_objects.as_slice() {
        [] => return Ok(Vec::new()),
        [children] => *children,
        _ => return Err("owner descriptor must contain at most one ChildObjects".into()),
    };
    children
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|child| matches!(child.tag_name().name(), "Form" | "Template" | "Command"))
        .map(|child| parse_owner_child_artifact(child, owner))
        .collect()
}

pub fn parse_child_descriptor(
    bytes: &[u8],
    owner: &MetadataAddress,
    admission: MetadataAdmission,
) -> Result<Option<ClosedChildDescriptor>, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document, admission)?;
    if !matches!(artifact.tag_name().name(), "Form" | "Template" | "Command") {
        return Ok(None);
    }
    parse_child_artifact(artifact, owner).map(Some)
}

/// Parse one already-retained physical child descriptor through the same
/// structural validator, with an explicit read or publication admission policy.
pub fn parse_child_profile_from_bytes(
    bytes: &[u8],
    owner: &MetadataAddress,
    admission: MetadataAdmission,
) -> Result<
    Option<(
        MetadataAddress,
        unica_metadata::validation_subject::MetadataChildProfile,
    )>,
    String,
> {
    parse_child_descriptor(bytes, owner, admission)
        .map(|descriptor| descriptor.map(|descriptor| (descriptor.child, descriptor.profile)))
}

pub fn parse_child_artifact(
    artifact: roxmltree::Node<'_, '_>,
    owner: &MetadataAddress,
) -> Result<ClosedChildDescriptor, String> {
    if artifact.tag_name().namespace() != Some(MD_CLASSES_NS) {
        return Err("child metadata artifact is outside the MDClasses namespace".into());
    }
    let kind = artifact.tag_name().name();
    let child_kind = match kind {
        "Form" => "Form",
        "Template" => "Template",
        "Command" => "Command",
        _ => return Err("metadata artifact is not a closed physical child kind".into()),
    };
    let properties =
        exact_mdclasses_children(artifact, "Properties", &format!("{kind} descriptor"))?;
    let child_objects =
        exact_mdclasses_children(artifact, "ChildObjects", &format!("{kind} descriptor"))?;
    if child_objects.len() > 1 {
        return Err(format!(
            "{kind} descriptor must contain at most one ChildObjects"
        ));
    }
    let [properties] = properties.as_slice() else {
        return Err(format!(
            "{kind} descriptor must contain exactly one Properties"
        ));
    };
    let name = exact_child_name(*properties, kind)?;
    let profile = match kind {
        "Form" => form_profile(*properties)?,
        "Command" => unica_metadata::validation_subject::MetadataChildProfile::Command,
        "Template" => unica_metadata::validation_subject::MetadataChildProfile::Template(
            template_descriptor_type(*properties)?,
        ),
        _ => unreachable!(),
    };
    let child = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner}.{child_kind}.{name}"),
    )
    .map_err(|_| "child descriptor identity is not representable".to_string())?;
    Ok(ClosedChildDescriptor { child, profile })
}

fn template_descriptor_type(
    properties: roxmltree::Node<'_, '_>,
) -> Result<MetadataTemplateType, String> {
    let types = exact_mdclasses_children(properties, "TemplateType", "Template descriptor")?;
    let [value] = types.as_slice() else {
        return Err("Template descriptor must contain exactly one TemplateType".into());
    };
    if value.children().any(|child| child.is_element()) {
        return Err("TemplateType must be a scalar".into());
    }
    MetadataTemplateType::from_descriptor_value(unica_xml_format::read::direct_text(*value).trim())
        .ok_or_else(|| "Template descriptor has unsupported TemplateType".into())
}

/// Read the top-level template through the same closed type vocabulary and
/// descriptor admission as a physical child template. This is a read profile,
/// not a new metadata creation kind.
pub struct CommonTemplateDescriptorFacts {
    pub template_type: MetadataTemplateType,
    pub synonym: Option<String>,
    pub unreadable_synonym: Option<String>,
}

pub fn parse_common_template_descriptor_from_bytes(
    bytes: &[u8],
    target: &MetadataAddress,
) -> Result<CommonTemplateDescriptorFacts, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document, MetadataAdmission::Read)?;
    if artifact.tag_name().name() != "CommonTemplate" {
        return Err("common template descriptor has the wrong artifact kind".into());
    }
    let properties = exact_mdclasses_children(artifact, "Properties", "CommonTemplate descriptor")?;
    let [properties] = properties.as_slice() else {
        return Err("CommonTemplate descriptor must contain exactly one Properties".into());
    };
    let name = exact_child_name(*properties, "CommonTemplate")?;
    if target.as_str() != format!("CommonTemplate.{name}") {
        return Err("common template descriptor belongs to another logical address".into());
    }
    let template_type = template_descriptor_type(*properties)?;
    let synonyms = properties
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Synonym")
        .collect::<Vec<_>>();
    let (synonym, unreadable_synonym) = match synonyms.as_slice() {
        [] => (None, None),
        [node]
            if super::info_projection::meta_info_property_value_is_valid(
                *node,
                unica_metadata::MetaInfoPropertyValueKind::LocalizedString,
            ) && super::info_projection::direct_text_content(*node)
                .trim()
                .is_empty() =>
        {
            let value = super::xml_model::meta_info_ml_text(*node);
            ((!value.is_empty()).then_some(value), None)
        }
        _ => (
            None,
            Some(
                "CommonTemplate Synonym is malformed, duplicated or has a foreign namespace".into(),
            ),
        ),
    };
    Ok(CommonTemplateDescriptorFacts {
        template_type,
        synonym,
        unreadable_synonym,
    })
}

pub fn form_profile(
    properties: roxmltree::Node<'_, '_>,
) -> Result<unica_metadata::validation_subject::MetadataChildProfile, String> {
    let types = exact_mdclasses_children(properties, "FormType", "Form descriptor")?;
    let form_type = match types.as_slice() {
        // Preserve abbreviated descriptors accepted by the existing reader.
        [] => return Ok(unica_metadata::validation_subject::MetadataChildProfile::Form),
        [value] => *value,
        _ => return Err("Form descriptor must contain at most one FormType".into()),
    };
    if form_type.children().any(|child| child.is_element()) {
        return Err("FormType must be a scalar".into());
    }
    match unica_xml_format::read::direct_text(form_type).trim() {
        "Managed" => Ok(unica_metadata::validation_subject::MetadataChildProfile::Form),
        "Ordinary" => Ok(unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm),
        _ => Err("Form descriptor has unsupported FormType".into()),
    }
}

pub fn child_descriptor_role_matches(
    role: &unica_metadata::validation_subject::MetadataResourceRole,
    child: &MetadataAddress,
    profile: unica_metadata::validation_subject::MetadataChildProfile,
) -> bool {
    match (role, profile) {
        (
            unica_metadata::validation_subject::MetadataResourceRole::Form { owner, name },
            unica_metadata::validation_subject::MetadataChildProfile::Form
            | unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm,
        ) => child.as_str() == format!("{owner}.Form.{name}"),
        (
            unica_metadata::validation_subject::MetadataResourceRole::Command { owner, name },
            unica_metadata::validation_subject::MetadataChildProfile::Command,
        ) => child.as_str() == format!("{owner}.Command.{name}"),
        (
            unica_metadata::validation_subject::MetadataResourceRole::Template { owner, name },
            unica_metadata::validation_subject::MetadataChildProfile::Template(_),
        ) => child.as_str() == format!("{owner}.Template.{name}"),
        _ => false,
    }
}

pub fn validate_one_child_footprint(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    expected: &ClosedChildDescriptor,
    footprint: &unica_metadata::validation_subject::MetadataChildFootprintEvidence,
    diagnostics: &mut Vec<MetaDiagnostic>,
) {
    let resources = subject
        .resources
        .iter()
        .filter_map(|resource| match resource.role {
            unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                ref child,
                kind,
                ordinal,
            } if child == &expected.child => Some((kind, ordinal)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let module_count = resources
        .iter()
        .filter(|(kind, _)| {
            matches!(
                kind,
                unica_metadata::validation_subject::MetadataChildResourceKind::Module
            )
        })
        .count();
    let html_pages = if expected.profile
        == unica_metadata::validation_subject::MetadataChildProfile::Template(
            unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
        ) {
        subject
                .resources
                .iter()
                .find_map(|resource| match &resource.role {
                    unica_metadata::validation_subject::MetadataResourceRole::ChildResource {
                        child,
                        kind:
                            unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                                template_type: unica_metadata::validation_subject::MetadataTemplateType::HtmlDocument,
                                part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                            },
                        ..
                    } if child == &expected.child => html_template_page_names(&resource.bytes).ok(),
                    _ => None,
                })
                .unwrap_or_default()
    } else {
        Vec::new()
    };

    // A borrowed child of a borrowed owner that the extension does not change
    // carries no payload: platform 8.3.27 exports it without `Form.xml` or
    // `Template.xml`. Both descriptors prove the borrowing independently of
    // the plan, and nothing else may then stand in the footprint.
    if resources.is_empty()
        && footprint.retained.is_empty()
        && borrowed_child_of_borrowed_owner(subject, &expected.child)
    {
        if !footprint.directories.is_empty() {
            diagnostics.push(template_resource_set_diagnostic(
                &expected.child,
                "child footprint does not contain its exact required directory set",
            ));
        }
        return;
    }

    // Re-derive the payload from the child's own profile rather than trusting
    // the plan: the retained members must still be shapes the closed contract
    // recognises, and the directory topology must be exactly their closure.
    if !footprint
        .retained
        .iter()
        .all(|relative| crate::edit::typed_child_retained_payload(expected.profile, relative))
    {
        diagnostics.push(template_resource_set_diagnostic(
            &expected.child,
            "child footprint retains a payload member the closed contract does not allow",
        ));
    }
    let mut payload =
        crate::edit::typed_child_modelled_payload(expected.profile, module_count == 1, &html_pages);
    payload.extend(footprint.retained.iter().cloned());
    let expected_directories = crate::edit::typed_child_payload_directories(&payload)
        .iter()
        .map(|relative| crate::edit::typed_child_directory_kind(relative))
        .collect::<Vec<_>>();
    if !multiset_eq(&expected_directories, &footprint.directories) {
        diagnostics.push(template_resource_set_diagnostic(
            &expected.child,
            "child footprint does not contain its exact required directory set",
        ));
    }

    let mut expected_kinds = match expected.profile {
        unica_metadata::validation_subject::MetadataChildProfile::OrdinaryForm => {
            vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::OrdinaryFormContent,
                0,
            )]
        }
        unica_metadata::validation_subject::MetadataChildProfile::Form => {
            let mut kinds = vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::FormContent,
                0,
            )];
            if module_count == 1 {
                kinds.push((
                    unica_metadata::validation_subject::MetadataChildResourceKind::Module,
                    1,
                ));
            }
            kinds
        }
        unica_metadata::validation_subject::MetadataChildProfile::Command => (module_count == 1)
            .then_some((
                unica_metadata::validation_subject::MetadataChildResourceKind::Module,
                0,
            ))
            .into_iter()
            .collect(),
        unica_metadata::validation_subject::MetadataChildProfile::Template(template_type) => {
            let mut kinds = vec![(
                unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                    template_type,
                    part: unica_metadata::validation_subject::MetadataTemplateResourcePart::Primary,
                },
                0,
            )];
            kinds.extend((0..html_pages.len()).map(|index| {
                (
                    unica_metadata::validation_subject::MetadataChildResourceKind::TemplateContent {
                        template_type,
                        part: unica_metadata::validation_subject::MetadataTemplateResourcePart::HtmlPage,
                    },
                    index + 1,
                )
            }));
            kinds
        }
    };
    expected_kinds.extend((0..footprint.retained.len()).map(|ordinal| {
        (
            unica_metadata::validation_subject::MetadataChildResourceKind::Retained,
            ordinal,
        )
    }));
    if resources.len() != expected_kinds.len()
        || expected_kinds.iter().any(|expected_resource| {
            resources
                .iter()
                .filter(|observed| *observed == expected_resource)
                .count()
                != 1
        })
    {
        diagnostics.push(template_resource_set_diagnostic(
            &expected.child,
            "child payload does not contain its exact canonical resource and ordinal set",
        ));
    }
}

/// The registering root is an extension, and the owner descriptor and the
/// child's own descriptor both name a parent object through
/// `ExtendedConfigurationObject`.
pub fn borrowed_child_of_borrowed_owner(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    child: &MetadataAddress,
) -> bool {
    let extension = subject
        .resources
        .iter()
        .find(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Registration
            )
        })
        .is_some_and(|root| crate::xml_model::configuration_is_extension(&root.bytes));
    let owner_borrowed = extension
        && subject
            .resources
            .iter()
            .find(|resource| {
                matches!(
                    resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Descriptor
                )
            })
            .is_some_and(|owner| crate::xml_model::descriptor_extends_parent(&owner.bytes));
    owner_borrowed
        && subject.resources.iter().any(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Form { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Template { .. }
                    | unica_metadata::validation_subject::MetadataResourceRole::Command { .. }
            ) && matches!(
                parse_child_descriptor(&resource.bytes, &subject.target, MetadataAdmission::Read),
                Ok(Some(actual)) if &actual.child == child
            ) && crate::xml_model::descriptor_extends_parent(&resource.bytes)
        })
}

pub fn multiset_eq<T: Ord + Copy>(left: &[T], right: &[T]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

pub fn template_resource_set_diagnostic(child: &MetadataAddress, message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
    )
    .with_metadata_path(child.clone())
    .with_field("resources")
}

pub fn validate_event_subscription_binding_subject(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    descriptor: &[u8],
) -> Vec<MetaDiagnostic> {
    let fail = |field: &str, message: String| vec![validation_diagnostic(subject, field, message)];
    let (_, document) = match unica_xml_format::read::parse_metadata_image(descriptor) {
        Ok(parsed) => parsed,
        Err(message) => return fail("resources", message),
    };
    let source_node = match crate::xml_model::meta_event_subscription_source_node(&document) {
        Ok(source) => source,
        Err(message) => return fail("relations.source", message),
    };
    // `v8:TypeId` members have no event class here; the binding is checked
    // against the classes of this source set's members (#1361).
    let sources = match crate::xml_model::parse_meta_event_subscription_source(source_node) {
        Ok(observed) => observed.sources,
        Err(message) => return fail("relations.source", message),
    };
    let properties = match source_node.parent_element() {
        Some(properties) => properties,
        None => {
            return fail(
                "resources",
                "EventSubscription Properties are unavailable".to_string(),
            )
        }
    };
    let event = unica_xml_format::read::child_direct_text(properties, "Event").unwrap_or_default();
    if event.is_empty() {
        return fail(
            "properties.event",
            "EventSubscription Event is empty".to_string(),
        );
    }
    let handler =
        unica_xml_format::read::child_direct_text(properties, "Handler").unwrap_or_default();
    let handler_parts = handler.split('.').collect::<Vec<_>>();
    let ["CommonModule", module_name, procedure_name] = handler_parts.as_slice() else {
        return fail(
            "properties.handler",
            "EventSubscription Handler must use CommonModule.<Module>.<Procedure>".to_string(),
        );
    };

    let mut source_classes = Vec::new();
    let mut visiting = HashSet::new();
    for (index, source) in sources.iter().enumerate() {
        if let Err(message) =
            expand_event_source_classes(subject, source, &mut source_classes, &mut visiting)
        {
            return fail(&format!("relations.source[{index}]"), message);
        }
    }

    let module_target = match MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("CommonModule.{module_name}"),
    ) {
        Ok(target) => target,
        Err(_) => {
            return fail(
                "properties.handler",
                "EventSubscription Handler CommonModule is invalid".to_string(),
            )
        }
    };
    let module_descriptor = subject.resources.iter().find(|resource| {
        matches!(
            &resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Dependency { target } if target == &module_target
        )
    });
    let Some(module_descriptor) = module_descriptor else {
        return fail(
            "properties.handler",
            format!("CommonModule `{module_name}` descriptor evidence is unavailable"),
        );
    };
    let (_, module_document) =
        match unica_xml_format::read::parse_metadata_image(&module_descriptor.bytes) {
            Ok(parsed) => parsed,
            Err(message) => return fail("properties.handler", message),
        };
    let module_properties = module_document
        .root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "CommonModule")
        .and_then(|node| unica_xml_format::read::child_element(node, "Properties"));
    let Some(module_properties) = module_properties else {
        return fail(
            "properties.handler",
            format!("CommonModule `{module_name}` properties are unavailable"),
        );
    };
    let module_global = unica_xml_format::read::child_direct_text(module_properties, "Global")
        .as_deref()
        .and_then(explicit_boolean_property);
    let module_server = unica_xml_format::read::child_direct_text(module_properties, "Server")
        .as_deref()
        .and_then(explicit_boolean_property);
    let module = subject.resources.iter().find(|resource| {
        matches!(&resource.role, unica_metadata::validation_subject::MetadataResourceRole::Module { owner } if owner == &module_target)
    });
    let Some(module) = module else {
        return fail(
            "properties.handler",
            format!("CommonModule `{module_name}` BSL evidence is unavailable"),
        );
    };
    let content = match std::str::from_utf8(&module.bytes) {
        Ok(content) => content,
        Err(error) => {
            return fail(
                "properties.handler",
                format!("handler module is not UTF-8: {error}"),
            )
        }
    };
    let method = match exact_bsl_method_facts(content, procedure_name) {
        Ok(Some(method)) => method,
        Ok(None) => {
            return fail(
                "properties.handler",
                format!(
                    "procedure `{procedure_name}` was not found in CommonModule `{module_name}`"
                ),
            )
        }
        Err(message) => return fail("properties.handler", message),
    };
    let facts = EventHandlerFacts {
        module_global,
        module_server,
        method_kind: if method.is_procedure {
            EventHandlerMethodKind::Procedure
        } else {
            EventHandlerMethodKind::Function
        },
        exported: method.is_export,
        parameter_count: method.parameter_count,
    };
    match validate_event_subscription_binding(&source_classes, &event, facts) {
        Ok(_) => Vec::new(),
        Err(error) => {
            let (field, message) = event_binding_error_message(error);
            fail(field, message)
        }
    }
}

pub fn explicit_boolean_property(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

pub fn expand_event_source_classes(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    source: &MetaEventSource,
    classes: &mut Vec<EventSourceClass>,
    visiting: &mut HashSet<MetadataAddress>,
) -> Result<(), String> {
    if let Some(source_class) = source.event_source_class()? {
        classes.push(source_class);
        return Ok(());
    }
    let target = source
        .metadata_path()
        .cloned()
        .ok_or_else(|| "event source class is unavailable".to_string())?;
    if !visiting.insert(target.clone()) {
        return Err(format!(
            "DefinedType event source cycle contains `{target}`"
        ));
    }
    let descriptor = subject
        .resources
        .iter()
        .find(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Dependency { target: dependency } if dependency == &target
            )
        })
        .ok_or_else(|| format!("DefinedType event source `{target}` evidence is unavailable"))?;
    for member in crate::xml_model::parse_defined_type_event_sources(&descriptor.bytes)? {
        expand_event_source_classes(subject, &member, classes, visiting)?;
    }
    visiting.remove(&target);
    Ok(())
}

pub fn event_binding_error_message(error: EventBindingError) -> (&'static str, String) {
    match error {
        EventBindingError::EmptySource => (
            "relations.source",
            "EventSubscription Source must not be empty".to_string(),
        ),
        EventBindingError::EventUnavailable {
            source_class,
            event,
        } => (
            "properties.event",
            format!(
                "event `{event}` is unavailable for source class `{}`",
                source_class.as_str()
            ),
        ),
        EventBindingError::SignatureConflict {
            event,
            expected_class,
            expected_parameters,
            actual_class,
            actual_parameters,
        } => (
            "properties.event",
            format!(
                "event `{event}` has incompatible signatures: `{}` ({}) versus `{}` ({})",
                expected_class.as_str(),
                expected_parameters.join(", "),
                actual_class.as_str(),
                actual_parameters.join(", ")
            ),
        ),
        EventBindingError::HandlerModuleIsGlobal => (
            "properties.handler",
            "EventSubscription handler CommonModule must have Global=false".to_string(),
        ),
        EventBindingError::HandlerModuleIsNotServer => (
            "properties.handler",
            "EventSubscription handler CommonModule must have Server=true".to_string(),
        ),
        EventBindingError::HandlerIsNotProcedure => (
            "properties.handler",
            "EventSubscription handler must be a procedure".to_string(),
        ),
        EventBindingError::HandlerIsNotExported => (
            "properties.handler",
            "EventSubscription handler procedure must be exported".to_string(),
        ),
        EventBindingError::HandlerArity {
            event,
            event_parameter_count,
            expected,
            actual,
        } => (
            "properties.handler",
            format!(
                "handler for `{event}` must have {expected} parameters (Source plus {event_parameter_count} event parameters), got {actual}"
            ),
        ),
    }
}

pub fn failed_validation(
    diagnostics: Vec<MetaDiagnostic>,
) -> unica_metadata::validation_subject::MetadataValidationResult {
    MetaValidationData {
        status: MetaValidationStatus::Failed,
        diagnostics,
    }
}

pub fn provider_diagnostic(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    resource_index: usize,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(subject.target.clone())
        .with_field(format!("resources[{resource_index}].bytes"))
}

pub fn child_resource_diagnostic(
    child: &MetadataAddress,
    resource_index: usize,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::ProviderUnavailable,
        format!(
            "typed child resource {child} is invalid: {}",
            message.into()
        ),
    )
    .with_metadata_path(child.clone())
    .with_field(format!("resources[{resource_index}].bytes"))
}

pub fn validation_diagnostic(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    field: &str,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field(field)
}

pub fn validation_warning(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
    field: &str,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::warning(MetaDiagnosticCode::ValidationWarning, message)
        .with_metadata_path(subject.target.clone())
        .with_field(field)
}

pub fn subject_target_identity(
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
) -> (&str, &str) {
    let mut segments = subject.target.segments();
    (
        segments.next().unwrap_or_default(),
        segments.next().unwrap_or_default(),
    )
}

pub fn image_contains_reference(bytes: &[u8], target: &str) -> bool {
    let Ok((_, document)) = unica_xml_format::read::parse_metadata_image(bytes) else {
        return false;
    };
    document
        .descendants()
        .filter(roxmltree::Node::is_element)
        .any(|node| node.text().is_some_and(|text| text.trim() == target))
}

pub fn metadata_image_reference(bytes: &[u8]) -> Result<String, String> {
    let (_, document) = unica_xml_format::read::parse_metadata_image(bytes)?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some("http://v8.1c.ru/8.3/MDClasses")
        || root.tag_name().name() != "MetaDataObject"
    {
        return Err("image is not an MDClasses MetaDataObject".to_string());
    }
    let artifacts = root
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
        })
        .collect::<Vec<_>>();
    let [artifact] = artifacts.as_slice() else {
        return Err("image must contain exactly one metadata descriptor".to_string());
    };
    let name = unica_xml_format::read::child_element(*artifact, "Properties")
        .and_then(|properties| unica_xml_format::read::child_direct_text(properties, "Name"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "metadata descriptor has no Name".to_string())?;
    Ok(format!("{}.{}", artifact.tag_name().name(), name))
}

pub fn validation_field_for_legacy_error(error: &str) -> &'static str {
    let error = error
        .trim_start_matches("[ERROR] ")
        .trim_start_matches("[WARN]  ");
    match error.split_once('.').map(|(check, _)| check) {
        Some("1") => "descriptor",
        Some("2") => "internalInfo",
        Some("3" | "4" | "10" | "12" | "13") => "properties",
        Some("5") => "standardAttributes",
        Some("6" | "7" | "8" | "11") => "childObjects",
        Some("9") => "tabularSections",
        Some("14") => "columns",
        _ => "descriptor",
    }
}

impl MetaValidationReporter {
    pub fn new(max_errors: usize) -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            findings: Vec::new(),
            stopped: false,
            max_errors,
        }
    }

    pub fn ok(&mut self, _message: impl Into<String>) {}

    pub fn error(&mut self, message: impl Into<String>) {
        self.errors.push(format!("[ERROR] {}", message.into()));
        if self.errors.len() >= self.max_errors {
            self.stopped = true;
        }
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(format!("[WARN]  {}", message.into()));
    }

    pub fn warn_finding(
        &mut self,
        code: MetaDiagnosticCode,
        field: impl Into<String>,
        language: Option<&str>,
        message: impl Into<String>,
    ) {
        self.findings.push(MetaValidationFinding {
            code,
            field: field.into(),
            language: language.map(str::to_owned),
            message: message.into(),
        });
    }

    pub fn finalize(self) -> (bool, Vec<String>, Vec<String>, Vec<MetaValidationFinding>) {
        (
            self.errors.is_empty(),
            self.errors,
            self.warnings,
            self.findings,
        )
    }
}

pub fn metadata_address(raw: impl AsRef<str>) -> Result<MetadataAddress, String> {
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw.as_ref())
        .map_err(|error| error.to_string())
}

pub fn metadata_collection(object_type: &str) -> Option<&'static str> {
    Some(match object_type {
        "CommonModule" => "CommonModules",
        "AccumulationRegister" => "AccumulationRegisters",
        "InformationRegister" => "InformationRegisters",
        "AccountingRegister" => "AccountingRegisters",
        "CalculationRegister" => "CalculationRegisters",
        "Document" => "Documents",
        _ => return None,
    })
}

pub fn meta_validate_source(
    bytes: &[u8],
    options: &MetaValidationOptions,
    reference_inputs: &MetaValidationReferenceInputs,
) -> Result<MetaValidationRun, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let source = std::str::from_utf8(bytes)
        .map_err(|error| format!("metadata image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let mut report = MetaValidationReporter::new(options.max_errors);
            report.error(format!("1. XML parse failed: {err}"));
            return meta_validate_finish(report);
        }
    };

    let root = doc.root_element();
    let mut report = MetaValidationReporter::new(options.max_errors);
    let mut check1_ok = true;

    if root.tag_name().name() != "MetaDataObject" {
        report.error(format!(
            "1. Root element is '{}', expected 'MetaDataObject'",
            root.tag_name().name()
        ));
        return meta_validate_finish(report);
    }

    let root_ns = root.tag_name().namespace().unwrap_or("");
    if root_ns != MD_NS {
        report.error(format!(
            "1. Root namespace is '{root_ns}', expected '{MD_NS}'"
        ));
        check1_ok = false;
    }

    let version_literal = root_version_literal(source, root);
    match classify_root_version(version_literal.as_deref()) {
        Ok(FormatCompatibility::Supported { .. }) => report.ok("Export format: 2.20"),
        Ok(compatibility) => report.warn(
            unica_source_designer::format_profile::format_compatibility_warning(&compatibility),
        ),
        Err(error) => report.error(error.to_string()),
    }
    let version = version_literal.as_deref().unwrap_or("");

    let child_elements = root
        .children()
        .filter(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        .collect::<Vec<_>>();
    if child_elements.is_empty() {
        report.error("1. No metadata type element found inside MetaDataObject");
        return meta_validate_finish(report);
    }
    if child_elements.len() > 1 {
        let names = child_elements
            .iter()
            .map(|child| format!("'{}'", child.tag_name().name()))
            .collect::<Vec<_>>();
        report.error(format!(
            "1. Multiple type elements found: [{}]",
            names.join(", ")
        ));
        check1_ok = false;
    }

    let type_node = child_elements[0];
    let md_type = type_node.tag_name().name();
    if !crate::validation::meta_validate_valid_types().contains(&md_type) {
        report.error(format!("1. Unrecognized metadata type: {md_type}"));
        return meta_validate_finish(report);
    }

    let type_uuid = type_node.attribute("uuid").unwrap_or("");
    if type_uuid.is_empty() {
        report.error(format!("1. Missing uuid on <{md_type}> element"));
        check1_ok = false;
    } else if !crate::validation::is_guid(type_uuid) {
        report.error(format!("1. Invalid uuid '{type_uuid}' on <{md_type}>"));
        check1_ok = false;
    }

    let props_node = unica_xml_format::read::child_element(type_node, "Properties");
    let name_node =
        props_node.and_then(|props| unica_xml_format::read::child_element(props, "Name"));
    let obj_name = name_node
        .map(unica_xml_format::read::direct_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(unknown)".to_string());

    if let Some(subject) = reference_inputs.proof_subject {
        meta_validate_check_subject_owner_proof(&mut report, md_type, &obj_name, subject);
    }

    if check1_ok {
        report.ok(format!(
            "1. Root structure: MetaDataObject/{md_type}, version {version}"
        ));
    }
    if report.stopped {
        return meta_validate_finish(report);
    }

    meta_validate_check_internal_info(&mut report, md_type, type_node, &obj_name);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_properties(
        &mut report,
        md_type,
        props_node,
        name_node,
        &obj_name,
        &reference_inputs.language_codes,
    );
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_property_values(&mut report, props_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_standard_attributes(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(report);
    }

    let child_obj_node = unica_xml_format::read::child_element(type_node, "ChildObjects");
    meta_validate_check_child_objects(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_child_elements(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_reserved_attr_names(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_uniqueness(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_tabular_sections(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_cross_properties(
        &mut report,
        md_type,
        props_node,
        child_obj_node,
        reference_inputs.config_dir,
        reference_inputs.proof_subject,
        &obj_name,
    );
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_services(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_forbidden_properties(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_method_reference(
        &mut report,
        md_type,
        props_node,
        reference_inputs.config_dir,
        reference_inputs.proof_subject,
    );
    if report.stopped {
        return meta_validate_finish(report);
    }
    meta_validate_check_document_journal_columns(&mut report, md_type, child_obj_node);

    meta_validate_finish(report)
}

pub fn meta_validate_check_subject_owner_proof(
    report: &mut MetaValidationReporter,
    md_type: &str,
    obj_name: &str,
    subject: &unica_metadata::validation_subject::MetadataValidationSubject,
) {
    if matches!(md_type, "ExternalReport" | "ExternalDataProcessor") {
        return;
    }
    let registrations = subject
        .resources
        .iter()
        .filter(|resource| {
            matches!(
                resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Registration
            )
        })
        .filter_map(|resource| {
            crate::validation_context::inspect_metadata_registration_image(&resource.bytes).ok()
        })
        .collect::<Vec<_>>();
    let is_registered = registrations.iter().any(|registration| {
        registration
            .registrations
            .iter()
            .any(|(kind, name)| kind == md_type && name == obj_name)
    });
    if !is_registered {
        report.error(format!(
            "{md_type}.{obj_name} is not registered in the owner image"
        ));
        return;
    }
    if crate::validation_context::meta_validate_types_with_list_presentation().contains(&md_type)
        && registrations
            .iter()
            .all(|registration| registration.registered_languages.is_empty())
    {
        report.error("owner image has no registered language profile");
    }
}

pub fn meta_validate_finish(report: MetaValidationReporter) -> Result<MetaValidationRun, String> {
    let (ok, errors, warnings, findings) = report.finalize();
    Ok(MetaValidationRun {
        ok,
        errors,
        warnings,
        findings,
    })
}

pub fn meta_validate_localized_values(
    node: Option<roxmltree::Node<'_, '_>>,
) -> Vec<(Option<String>, String)> {
    const V8_CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";

    let Some(node) = node else {
        return Vec::new();
    };
    node.children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == "item"
                && child.tag_name().namespace() == Some(V8_CORE_NS)
        })
        .filter_map(|item| {
            let child_text = |name| {
                item.children()
                    .find(|child| {
                        child.is_element()
                            && child.tag_name().name() == name
                            && child.tag_name().namespace() == Some(V8_CORE_NS)
                    })
                    .map(unica_xml_format::read::direct_text)
            };
            let language = child_text("lang")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let text = child_text("content").unwrap_or_default();
            (!text.trim().is_empty()).then_some((language, text))
        })
        .collect()
}

pub fn meta_validate_check_internal_info(
    report: &mut MetaValidationReporter,
    md_type: &str,
    type_node: roxmltree::Node<'_, '_>,
    obj_name: &str,
) {
    let internal_info = unica_xml_format::read::child_element(type_node, "InternalInfo");
    if meta_validate_types_without_internal_info().contains(&md_type) {
        if let Some(internal_info) = internal_info {
            let gen_types = unica_xml_format::read::child_elements(internal_info, "GeneratedType");
            if gen_types.is_empty() {
                report.ok(format!(
                    "2. InternalInfo: absent or empty (correct for {md_type})"
                ));
            } else {
                report.warn(format!(
                    "2. InternalInfo: {md_type} should not have GeneratedType entries, found {}",
                    gen_types.len()
                ));
            }
        } else {
            report.ok(format!("2. InternalInfo: absent (correct for {md_type})"));
        }
        return;
    }

    let Some(expected_categories) = meta_validate_generated_categories(md_type) else {
        return;
    };
    let Some(internal_info) = internal_info else {
        report.error(format!(
            "2. InternalInfo: missing (expected {} GeneratedType)",
            expected_categories.len()
        ));
        return;
    };
    let gen_types = unica_xml_format::read::child_elements(internal_info, "GeneratedType");
    let mut check_ok = true;
    let mut found_categories = Vec::<String>::new();
    for generated_type in &gen_types {
        let name = generated_type.attribute("name").unwrap_or("");
        let category = generated_type.attribute("category").unwrap_or("");
        found_categories.push(category.to_string());
        if !name.is_empty() && obj_name != "(unknown)" && !name.ends_with(&format!(".{obj_name}")) {
            report.error(format!(
                "2. GeneratedType name '{name}' does not end with '.{obj_name}'"
            ));
            check_ok = false;
        }
        if !expected_categories.contains(&category) {
            report.warn(format!(
                "2. Unexpected GeneratedType category '{category}' for {md_type}"
            ));
        }
        if let Some(type_id) = unica_xml_format::read::child_element(*generated_type, "TypeId") {
            if !crate::validation::is_guid(&unica_xml_format::read::direct_text(type_id)) {
                report.error(format!(
                    "2. Invalid TypeId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
        if let Some(value_id) = unica_xml_format::read::child_element(*generated_type, "ValueId") {
            if !crate::validation::is_guid(&unica_xml_format::read::direct_text(value_id)) {
                report.error(format!(
                    "2. Invalid ValueId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
    }

    if md_type == "ExchangePlan" {
        if let Some(this_node) = unica_xml_format::read::child_element(internal_info, "ThisNode") {
            if !crate::validation::is_guid(&unica_xml_format::read::direct_text(this_node)) {
                report.error("2. ExchangePlan xr:ThisNode has invalid UUID");
                check_ok = false;
            }
        } else {
            report.warn("2. ExchangePlan missing xr:ThisNode in InternalInfo");
        }
    }

    let missing_categories = expected_categories
        .iter()
        .filter(|category| !found_categories.iter().any(|found| found == **category))
        .copied()
        .collect::<Vec<_>>();
    if !missing_categories.is_empty() {
        report.warn(format!(
            "2. Missing GeneratedType categories: {}",
            missing_categories.join(", ")
        ));
    }
    if check_ok {
        found_categories.sort();
        report.ok(format!(
            "2. InternalInfo: {} GeneratedType ({})",
            gen_types.len(),
            found_categories.join(", ")
        ));
    }
}

pub fn meta_validate_check_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    name_node: Option<roxmltree::Node<'_, '_>>,
    obj_name: &str,
    configured_language_codes: &[String],
) {
    let Some(props_node) = props_node else {
        report.error("3. Properties block missing");
        return;
    };
    let mut check_ok = true;
    if name_node.is_none() || obj_name.is_empty() {
        report.error("3. Properties: Name is missing or empty");
        check_ok = false;
    } else {
        if !unica_metadata::identifier::is_1c_identifier(obj_name) {
            report.error(format!(
                "3. Properties: Name '{obj_name}' is not a valid 1C identifier"
            ));
            check_ok = false;
        }
        if obj_name.chars().count() > 80 {
            report.warn(format!(
                "3. Properties: Name '{obj_name}' is longer than 80 characters ({})",
                obj_name.chars().count()
            ));
        }
    }
    let synonym_values = meta_validate_localized_values(unica_xml_format::read::child_element(
        props_node, "Synonym",
    ));
    let syn_present = !synonym_values.is_empty();

    if crate::validation_context::meta_validate_types_with_list_presentation().contains(&md_type) {
        meta_validate_check_command_texts(report, props_node, configured_language_codes);
    }
    if check_ok {
        let syn_info = if syn_present {
            "Synonym present"
        } else {
            "no Synonym"
        };
        report.ok(format!("3. Properties: Name=\"{obj_name}\", {syn_info}"));
    }
}

pub fn meta_validate_check_command_texts(
    report: &mut MetaValidationReporter,
    props_node: roxmltree::Node<'_, '_>,
    language_codes: &[String],
) {
    let synonyms = meta_validate_localized_values(unica_xml_format::read::child_element(
        props_node, "Synonym",
    ));
    let lists = meta_validate_localized_values(unica_xml_format::read::child_element(
        props_node,
        "ListPresentation",
    ));

    for language_code in language_codes {
        let list_values = lists
            .iter()
            .filter(|(language, text)| {
                language.as_deref() == Some(language_code.as_str()) && !text.trim().is_empty()
            })
            .collect::<Vec<_>>();
        // A list presentation that merely repeats the synonym adds nothing to the
        // command interface, so a value equal to the synonym is redundant.
        for (_, list_text) in &list_values {
            let duplicates_synonym = synonyms.iter().any(|(language, synonym_text)| {
                language.as_deref() == Some(language_code.as_str())
                    && !synonym_text.trim().is_empty()
                    && synonym_text == list_text
            });
            if duplicates_synonym {
                report.warn_finding(
                    MetaDiagnosticCode::RedundantListPresentation,
                    "properties.ListPresentation",
                    Some(language_code.as_str()),
                    format!(
                        "3. Properties: ListPresentation '{list_text}' duplicates the Synonym \
                         for the command interface, language '{language_code}' (a list \
                         presentation equal to the synonym is redundant)"
                    ),
                );
            }
        }
        let selected = if list_values.is_empty() {
            synonyms
                .iter()
                .filter(|(language, text)| {
                    language.as_deref() == Some(language_code.as_str()) && !text.trim().is_empty()
                })
                .map(|(_, text)| ("Synonym", text))
                .collect::<Vec<_>>()
        } else {
            list_values
                .into_iter()
                .map(|(_, text)| ("ListPresentation", text))
                .collect::<Vec<_>>()
        };
        for (source, text) in selected {
            meta_validate_warn_long_command_text(
                report,
                source,
                text,
                Some(language_code.as_str()),
            );
        }
    }
}

pub fn meta_validate_warn_long_command_text(
    report: &mut MetaValidationReporter,
    source: &str,
    text: &str,
    language: Option<&str>,
) {
    let length = text.chars().count();
    let (code, message) = if length > 38 {
        (
            MetaDiagnosticCode::CommandTextUpperLimit,
            format!(
                "3. Properties: {source} '{text}' is longer than the upper \
                 command-interface threshold of 38 characters ({length})"
            ),
        )
    } else if length > 30 {
        (
            MetaDiagnosticCode::CommandTextRecommendedLimit,
            format!(
                "3. Properties: {source} '{text}' is longer than the recommended \
                 command-interface length of 30 characters ({length})"
            ),
        )
    } else {
        return;
    };
    report.warn_finding(code, format!("properties.{source}"), language, message);
}

pub fn meta_validate_check_property_values(
    report: &mut MetaValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut enum_checked = 0usize;
    let mut check_ok = true;
    for (prop_name, allowed) in crate::validation::meta_validate_property_values() {
        if let Some(value) = unica_xml_format::read::child_direct_text(props_node, prop_name)
            .filter(|value| !value.is_empty())
        {
            if !allowed.contains(&value.as_str()) {
                report.error(format!(
                    "4. Property '{prop_name}' has invalid value '{value}' (allowed: {})",
                    allowed.join(", ")
                ));
                check_ok = false;
            }
            enum_checked += 1;
        }
    }
    if check_ok {
        report.ok(format!(
            "4. Property values: {enum_checked} enum properties checked"
        ));
    }
}

pub fn meta_validate_check_standard_attributes(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    if !meta_validate_types_with_std_attrs().contains(&md_type) {
        return;
    }
    let Some(props_node) = props_node else {
        return;
    };
    let Some(std_attr_node) =
        unica_xml_format::read::child_element(props_node, "StandardAttributes")
    else {
        report.ok(format!(
            "5. StandardAttributes: absent (optional for {md_type})"
        ));
        return;
    };
    let std_attrs = unica_xml_format::read::child_elements(std_attr_node, "StandardAttribute");
    let expected_std_attrs = meta_validate_standard_attributes(md_type).unwrap_or_default();
    let mut check_ok = true;
    let mut found_names = Vec::<String>::new();
    for standard_attr in &std_attrs {
        let name = standard_attr.attribute("name").unwrap_or("");
        if name.is_empty() {
            report.error("5. StandardAttribute without 'name' attribute");
            check_ok = false;
            continue;
        }
        found_names.push(name.to_string());
        if !expected_std_attrs.contains(&name)
            && !meta_validate_dynamic_standard_attr(md_type, name)
        {
            report.warn(format!(
                "5. Unexpected StandardAttribute '{name}' for {md_type}"
            ));
        }
    }
    let missing_attrs = expected_std_attrs
        .iter()
        .filter(|attr| !found_names.iter().any(|found| found == **attr))
        .copied()
        .collect::<Vec<_>>();
    if !missing_attrs.is_empty() {
        report.warn(format!(
            "5. Missing StandardAttributes: {}",
            missing_attrs.join(", ")
        ));
    }
    if check_ok {
        report.ok(format!(
            "5. StandardAttributes: {} entries",
            std_attrs.len()
        ));
    }
}

pub fn meta_validate_check_child_objects(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let allowed_children = meta_validate_child_rules(md_type).unwrap_or_default();
    if let Some(child_obj_node) = child_obj_node {
        let mut check_ok = true;
        let mut child_counts = BTreeMap::<String, usize>::new();
        for child in child_obj_node.children().filter(|child| child.is_element()) {
            let child_tag = child.tag_name().name();
            if !allowed_children.contains(&child_tag) {
                report.error(format!(
                    "6. ChildObjects: disallowed element '{child_tag}' for {md_type}"
                ));
                check_ok = false;
            }
            *child_counts.entry(child_tag.to_string()).or_default() += 1;
        }
        if check_ok {
            if child_counts.is_empty() {
                report.ok(format!("6. ChildObjects: empty (valid for {md_type})"));
            } else {
                let summary = child_counts
                    .iter()
                    .map(|(name, count)| format!("{name}({count})"))
                    .collect::<Vec<_>>()
                    .join(", ");
                report.ok(format!("6. ChildObjects types: {summary}"));
            }
        }
    } else if allowed_children.is_empty() {
        report.ok(format!("6. ChildObjects: absent (correct for {md_type})"));
    } else {
        report.ok("6. ChildObjects: absent");
    }
}

pub fn meta_validate_check_child_elements(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    let mut count = 0usize;
    for kind in ["Attribute", "Dimension", "Resource", "EnumValue", "Column"] {
        let require_type = !matches!(kind, "EnumValue" | "Column");
        for element in unica_xml_format::read::child_elements(child_obj_node, kind) {
            if !meta_validate_check_child_element(report, element, kind, require_type) {
                check_ok = false;
            }
            count += 1;
            if report.stopped {
                break;
            }
        }
    }
    if check_ok && count > 0 {
        report.ok(format!(
            "7. Child elements: {count} items checked (UUID, Name, Type)"
        ));
    } else if count == 0 {
        report.ok("7. Child elements: none to check");
    }
}

pub fn meta_validate_check_child_element(
    report: &mut MetaValidationReporter,
    node: roxmltree::Node<'_, '_>,
    kind: &str,
    require_type: bool,
) -> bool {
    let uuid = node.attribute("uuid").unwrap_or("");
    if uuid.is_empty() {
        report.error(format!("7. {kind} missing uuid"));
        return false;
    }
    if !crate::validation::is_guid(uuid) {
        report.error(format!("7. {kind} has invalid uuid '{uuid}'"));
        return false;
    }
    let Some(props) = unica_xml_format::read::child_element(node, "Properties") else {
        report.error(format!("7. {kind} (uuid={uuid}) missing Properties"));
        return false;
    };
    let name = unica_xml_format::read::child_direct_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("7. {kind} (uuid={uuid}) missing or empty Name"));
        return false;
    }
    if !unica_metadata::identifier::is_1c_identifier(&name) {
        report.error(format!("7. {kind} '{name}' has invalid identifier"));
        return false;
    }
    if require_type {
        let Some(type_el) = unica_xml_format::read::child_element(props, "Type") else {
            report.error(format!("7. {kind} '{name}' missing Type block"));
            return false;
        };
        // Designer writes an extension's reference to a type of the extended
        // configuration as `v8:TypeId` (UUID of the generated type), alone or
        // after `v8:Type`/`v8:TypeSet`. The platform keeps an unresolved but
        // well-formed identifier and rejects only a malformed one (#1361).
        let type_ids = unica_xml_format::read::child_elements(type_el, "TypeId");
        if unica_xml_format::read::child_elements(type_el, "Type").is_empty()
            && unica_xml_format::read::child_elements(type_el, "TypeSet").is_empty()
            && type_ids.is_empty()
        {
            report.error(format!(
                "7. {kind} '{name}' Type block has no v8:Type, v8:TypeSet or v8:TypeId"
            ));
            return false;
        }
        let mut seen_type_ids = HashSet::new();
        for type_id in type_ids
            .into_iter()
            .map(unica_xml_format::read::direct_text)
        {
            let type_id = type_id.trim();
            if !crate::validation::is_guid(type_id) {
                report.error(format!(
                    "7. {kind} '{name}' has invalid v8:TypeId '{type_id}'"
                ));
                return false;
            }
            if !seen_type_ids.insert(type_id.to_ascii_lowercase()) {
                report.error(format!(
                    "7. {kind} '{name}' has duplicate v8:TypeId '{type_id}'"
                ));
                return false;
            }
        }
    }
    true
}

pub fn meta_validate_check_reserved_attr_names(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for attr_node in unica_xml_format::read::child_elements(child_obj_node, "Attribute") {
        if let Some(name) = unica_xml_format::read::child_element(attr_node, "Properties")
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
            .filter(|value| meta_validate_reserved_attr_names().contains(&value.as_str()))
        {
            report.warn(format!(
                "7b. Attribute '{name}' conflicts with a standard attribute name"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("7b. Reserved attribute names: no conflicts");
    }
}

pub fn meta_validate_check_uniqueness(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for kind in [
        "Attribute",
        "TabularSection",
        "Dimension",
        "Resource",
        "EnumValue",
        "Column",
        "URLTemplate",
        "Operation",
    ] {
        if !meta_validate_names_unique(
            report,
            unica_xml_format::read::child_elements(child_obj_node, kind),
            kind,
        ) {
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("8. Name uniqueness: all names unique");
    }
}

pub fn meta_validate_names_unique(
    report: &mut MetaValidationReporter,
    nodes: Vec<roxmltree::Node<'_, '_>>,
    kind: &str,
) -> bool {
    let mut names = HashSet::<String>::new();
    let mut ok = true;
    for node in nodes {
        let Some(name) = unica_xml_format::read::child_element(node, "Properties")
            .and_then(|props| unica_xml_format::read::child_direct_text(props, "Name"))
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        if !names.insert(name.clone()) {
            report.error(format!("8. Duplicate {kind} name: '{name}'"));
            ok = false;
        }
    }
    ok
}

pub fn meta_validate_check_tabular_sections(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let sections = unica_xml_format::read::child_elements(child_obj_node, "TabularSection");
    if sections.is_empty() {
        report.ok("9. TabularSections: none present");
        return;
    }
    let mut check_ok = true;
    for (index, section) in sections.iter().enumerate() {
        let count = index + 1;
        let uuid = section.attribute("uuid").unwrap_or("");
        if uuid.is_empty() || !crate::validation::is_guid(uuid) {
            report.error(format!(
                "9. TabularSection #{count}: invalid or missing uuid"
            ));
            check_ok = false;
        }
        let props = unica_xml_format::read::child_element(*section, "Properties");
        let section_name = props
            .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unnamed)".to_string());
        if section_name == "(unnamed)" {
            report.error(format!("9. TabularSection #{count}: missing or empty Name"));
            check_ok = false;
        }
        if let Some(internal_info) = unica_xml_format::read::child_element(*section, "InternalInfo")
        {
            let generated = unica_xml_format::read::child_elements(internal_info, "GeneratedType");
            if generated.len() < 2 {
                report.warn(format!(
                    "9. TabularSection '{section_name}': expected 2 GeneratedType, found {}",
                    generated.len()
                ));
            }
        }
        if let Some(ts_child_obj) = unica_xml_format::read::child_element(*section, "ChildObjects")
        {
            let mut names = HashSet::<String>::new();
            for attr in unica_xml_format::read::child_elements(ts_child_obj, "Attribute") {
                if !meta_validate_check_child_element(
                    report,
                    attr,
                    &format!("TabularSection '{section_name}'.Attribute"),
                    true,
                ) {
                    check_ok = false;
                }
                if let Some(name) = unica_xml_format::read::child_element(attr, "Properties")
                    .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
                    .filter(|value| !value.is_empty())
                {
                    if !names.insert(name.clone()) {
                        report.error(format!(
                            "9. Duplicate attribute '{name}' in TabularSection '{section_name}'"
                        ));
                        check_ok = false;
                    }
                }
            }
            if let Some(props) = props {
                if let Some(std_attr) =
                    unica_xml_format::read::child_element(props, "StandardAttributes")
                {
                    let has_line_number =
                        unica_xml_format::read::child_elements(std_attr, "StandardAttribute")
                            .iter()
                            .any(|attr| attr.attribute("name") == Some("LineNumber"));
                    if !has_line_number {
                        report.warn(format!(
                            "9. TabularSection '{section_name}': missing LineNumber StandardAttribute"
                        ));
                    }
                }
            }
        }
    }
    if check_ok {
        report.ok(format!(
            "9. TabularSections: {} sections, structure valid",
            sections.len()
        ));
    }
}

pub fn meta_validate_check_cross_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<MetadataValidationSource<'_>>,
    proof_subject: Option<&unica_metadata::validation_subject::MetadataValidationSubject>,
    obj_name: &str,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let mut check_ok = true;
    let mut issues = 0usize;
    // HierarchyType is inactive while Hierarchical=false: platform 8.3.27
    // writes it for every catalog, and any value is valid in that state.
    if md_type == "CommonModule" {
        let any_enabled = [
            "Server",
            "ClientManagedApplication",
            "ClientOrdinaryApplication",
            "ExternalConnection",
            "ServerCall",
            "Global",
        ]
        .iter()
        .any(|name| {
            unica_xml_format::read::child_direct_text(props_node, name).as_deref() == Some("true")
        });
        if !any_enabled {
            report.warn("10. CommonModule: no execution context enabled");
            issues += 1;
        }
    }
    if md_type == "EventSubscription" {
        if unica_xml_format::read::child_direct_text(props_node, "Handler")
            .is_none_or(|value| value.trim().is_empty())
        {
            report.error("10. EventSubscription: empty Handler");
            check_ok = false;
            issues += 1;
        }
        let source = unica_xml_format::read::child_element(props_node, "Source");
        let count = |name: &str| {
            source.map_or(0, |node| {
                unica_xml_format::read::child_elements(node, name).len()
            })
        };
        let logical_sources = count("Type") + count("TypeSet");
        if logical_sources == 0 && count("TypeId") > 0 {
            // Platform 8.3.27 refuses to load it: «Имя события должно быть
            // задано» — the event is resolved against this source set (#1361).
            report.error(
                "10. EventSubscription: Source has only v8:TypeId types of the extended configuration; platform requires at least one source of this source set",
            );
            check_ok = false;
            issues += 1;
        } else if logical_sources == 0 {
            report.warn("10. EventSubscription: no Source types specified");
            issues += 1;
        }
    }
    if md_type == "ScheduledJob"
        && unica_xml_format::read::child_direct_text(props_node, "MethodName")
            .is_none_or(|value| value.trim().is_empty())
    {
        report.error("10. ScheduledJob: empty MethodName");
        check_ok = false;
        issues += 1;
    }
    for (type_name, property, message) in [
        (
            "AccountingRegister",
            "ChartOfAccounts",
            "10. AccountingRegister: empty ChartOfAccounts",
        ),
        (
            "CalculationRegister",
            "ChartOfCalculationTypes",
            "10. CalculationRegister: empty ChartOfCalculationTypes",
        ),
    ] {
        if md_type == type_name
            && unica_xml_format::read::child_direct_text(props_node, property)
                .is_none_or(|value| value.trim().is_empty())
        {
            report.error(message);
            check_ok = false;
            issues += 1;
        }
    }
    if md_type == "BusinessProcess"
        && unica_xml_format::read::child_direct_text(props_node, "Task")
            .is_none_or(|value| value.trim().is_empty())
    {
        report.warn("10. BusinessProcess: empty Task reference");
        issues += 1;
    }
    if md_type == "CalculationRegister"
        && unica_xml_format::read::child_direct_text(props_node, "ActionPeriod").as_deref()
            == Some("true")
        && unica_xml_format::read::child_direct_text(props_node, "Schedule")
            .is_none_or(|value| value.trim().is_empty())
    {
        report.warn(
            "10. CalculationRegister: ActionPeriod=true but Schedule is empty — platform requires a schedule register",
        );
        issues += 1;
    }
    if md_type == "DocumentJournal" {
        let has_registered =
            unica_xml_format::read::child_element(props_node, "RegisteredDocuments")
                .map(|node| !unica_xml_format::read::child_elements(node, "Type").is_empty())
                .unwrap_or(false);
        if !has_registered {
            report.warn("10. DocumentJournal: no RegisteredDocuments specified");
            issues += 1;
        }
    }
    if md_type == "ChartOfAccounts" {
        let max_ext_dim =
            unica_xml_format::read::child_direct_text(props_node, "MaxExtDimensionCount")
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0);
        if max_ext_dim > 0
            && unica_xml_format::read::child_direct_text(props_node, "ExtDimensionTypes")
                .is_none_or(|value| value.trim().is_empty())
        {
            report
                .warn("10. ChartOfAccounts: MaxExtDimensionCount>0 but ExtDimensionTypes is empty");
            issues += 1;
        }
    }
    if matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) {
        if let Some(child_obj_node) = child_obj_node {
            let count = unica_xml_format::read::child_elements(child_obj_node, "Dimension").len()
                + unica_xml_format::read::child_elements(child_obj_node, "Resource").len()
                + unica_xml_format::read::child_elements(child_obj_node, "Attribute").len();
            if count == 0 {
                report.warn(format!(
                    "10. {md_type}: no Dimensions, Resources, or Attributes — platform will reject"
                ));
                issues += 1;
            }
        }
    }
    if md_type == "InformationRegister"
        && unica_xml_format::read::child_direct_text(props_node, "WriteMode").as_deref()
            == Some("RecorderSubordinate")
        && unica_xml_format::read::child_direct_text(props_node, "UseStandardCommands").as_deref()
            == Some("true")
    {
        report.warn(
            "10. InformationRegister: WriteMode=RecorderSubordinate with UseStandardCommands=true (subordinate registers are not shown in the command interface)",
        );
        issues += 1;
    }
    meta_validate_check_document_register_records(
        report,
        md_type,
        props_node,
        config_dir,
        proof_subject,
        &mut issues,
    );
    meta_validate_check_register_registrar(
        report,
        md_type,
        props_node,
        config_dir,
        proof_subject,
        obj_name,
        &mut issues,
    );
    meta_validate_check_register_command_interface(
        report,
        md_type,
        props_node,
        proof_subject,
        obj_name,
        &mut issues,
    );
    if check_ok && issues == 0 {
        report.ok("10. Cross-property consistency");
    }
}

pub fn meta_validate_check_document_register_records(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<MetadataValidationSource<'_>>,
    proof_subject: Option<&unica_metadata::validation_subject::MetadataValidationSubject>,
    issues: &mut usize,
) {
    if md_type != "Document" {
        return;
    }
    let Some(register_records) =
        unica_xml_format::read::child_element(props_node, "RegisterRecords")
    else {
        return;
    };
    for item in unica_xml_format::read::child_elements(register_records, "Item") {
        let ref_value = unica_xml_format::read::direct_text(item).trim().to_string();
        let Some((ref_type, ref_name)) = ref_value.split_once('.') else {
            continue;
        };
        if let Some(subject) = proof_subject {
            let found = subject.resources.iter().any(|resource| {
                matches!(
                    &resource.role,
                    unica_metadata::validation_subject::MetadataResourceRole::Dependency { target }
                        if target.as_str() == ref_value
                )
            });
            if !found {
                report.warn(format!(
                    "10. Document.RegisterRecords references '{ref_value}' but object not found in config"
                ));
                *issues += 1;
            }
            continue;
        }
        let Some(source) = config_dir else {
            continue;
        };
        let config_dir = source.config_dir;
        let reader = source.reader;
        let ref_dir = match ref_type {
            "AccumulationRegister" => "AccumulationRegisters",
            "InformationRegister" => "InformationRegisters",
            "AccountingRegister" => "AccountingRegisters",
            "CalculationRegister" => "CalculationRegisters",
            _ => continue,
        };
        let ref_path = config_dir.join(ref_dir).join(ref_name);
        let ref_xml = config_dir.join(ref_dir).join(format!("{ref_name}.xml"));
        if !path_exists(reader, &ref_path) && !path_exists(reader, &ref_xml) {
            report.warn(format!(
                "10. Document.RegisterRecords references '{ref_value}' but object not found in config"
            ));
            *issues += 1;
        }
    }
}

pub fn meta_validate_check_register_registrar(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<MetadataValidationSource<'_>>,
    proof_subject: Option<&unica_metadata::validation_subject::MetadataValidationSubject>,
    obj_name: &str,
    issues: &mut usize,
) {
    if !matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) || obj_name == "(unknown)"
    {
        return;
    }
    if md_type == "InformationRegister"
        && unica_xml_format::read::child_direct_text(props_node, "WriteMode").as_deref()
            != Some("RecorderSubordinate")
    {
        return;
    }
    let reg_ref = format!("{md_type}.{obj_name}");
    let has_registrar = if let Some(subject) = proof_subject {
        subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Dependency { target }
                    if target.segments().next() == Some("Document")
            ) && image_contains_reference(&resource.bytes, &reg_ref)
        })
    } else if let Some(source) = config_dir {
        let config_dir = source.config_dir;
        let reader = source.reader;
        let docs_dir = config_dir.join("Documents");
        path_is_dir(reader, &docs_dir)
            && crate::validation_context::meta_validate_registrar_document_scan(
                reader, &docs_dir, &reg_ref,
            )
            .map(|(_, found)| found)
            .unwrap_or(false)
    } else {
        return;
    };
    if !has_registrar {
        report.warn(format!(
            "10. {md_type}: no registrar document found (none references '{reg_ref}' in RegisterRecords)"
        ));
        *issues += 1;
    }
}

pub fn meta_validate_check_register_command_interface(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    proof_subject: Option<&unica_metadata::validation_subject::MetadataValidationSubject>,
    obj_name: &str,
    issues: &mut usize,
) {
    if !matches!(
        md_type,
        "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "InformationRegister"
    ) || obj_name == "(unknown)"
    {
        return;
    }
    if unica_xml_format::read::child_direct_text(props_node, "UseStandardCommands").as_deref()
        != Some("true")
    {
        return;
    }
    // A subordinate information register is not shown in the command interface at
    // all, so subsystem membership is not required — that pairing has its own rule.
    if md_type == "InformationRegister"
        && unica_xml_format::read::child_direct_text(props_node, "WriteMode").as_deref()
            == Some("RecorderSubordinate")
    {
        return;
    }
    let Some(unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
        interface_subsystems,
        ..
    }) = proof_subject.and_then(|subject| subject.subsystem_evidence.as_ref())
    else {
        return;
    };
    let object_ref = format!("{md_type}.{obj_name}");
    let shown_in_command_interface = !interface_subsystems.is_empty();
    if !shown_in_command_interface {
        report.warn(format!(
            "10. {md_type}: UseStandardCommands=true but '{object_ref}' reaches no command interface section (a section is a subsystem whose whole chain to the root stays included; a service subsystem groups objects and is not made a section by its own flag)"
        ));
        *issues += 1;
    }
}

pub struct ServiceSemanticsFinding {
    pub error: bool,
    pub detail: String,
}

pub struct ServiceSemantics {
    pub findings: Vec<ServiceSemanticsFinding>,
    pub summary: String,
    pub clean: bool,
}

pub fn service_child_semantics(
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) -> Option<ServiceSemantics> {
    if md_type == "HTTPService" {
        let mut findings = Vec::new();
        let mut template_count = 0usize;
        let mut method_count = 0usize;
        if let Some(child_obj_node) = child_obj_node {
            let templates = unica_xml_format::read::child_elements(child_obj_node, "URLTemplate");
            template_count = templates.len();
            for template in &templates {
                let props = unica_xml_format::read::child_element(*template, "Properties");
                let name = props
                    .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
                    .unwrap_or_else(|| "(unnamed)".to_string());
                if props
                    .and_then(|node| unica_xml_format::read::child_direct_text(node, "Template"))
                    .is_none_or(|value| value.trim().is_empty())
                {
                    findings.push(ServiceSemanticsFinding {
                        error: true,
                        detail: format!("URLTemplate '{name}': empty Template"),
                    });
                }
                if let Some(child_objects) =
                    unica_xml_format::read::child_element(*template, "ChildObjects")
                {
                    for method in unica_xml_format::read::child_elements(child_objects, "Method") {
                        method_count += 1;
                        let props = unica_xml_format::read::child_element(method, "Properties");
                        let http_method = props.and_then(|node| {
                            unica_xml_format::read::child_direct_text(node, "HTTPMethod")
                        });
                        if let Some(http_method) = http_method.filter(|value| !value.is_empty()) {
                            if !crate::validation::meta_validate_valid_http_methods()
                                .contains(&http_method.as_str())
                            {
                                findings.push(ServiceSemanticsFinding {
                                    error: true,
                                    detail: format!(
                                        "URLTemplate '{name}': invalid HTTPMethod '{http_method}'"
                                    ),
                                });
                            }
                        } else {
                            findings.push(ServiceSemanticsFinding {
                                error: true,
                                detail: format!("URLTemplate '{name}': Method missing HTTPMethod"),
                            });
                        }
                    }
                }
            }
        }
        let clean = findings.iter().all(|finding| !finding.error);
        return Some(ServiceSemantics {
            findings,
            summary: format!("{template_count} URLTemplate(s), {method_count} method(s)"),
            clean,
        });
    }
    if md_type == "WebService" {
        let mut findings = Vec::new();
        let mut operation_count = 0usize;
        let mut param_count = 0usize;
        if let Some(child_obj_node) = child_obj_node {
            let operations = unica_xml_format::read::child_elements(child_obj_node, "Operation");
            operation_count = operations.len();
            for operation in &operations {
                let props = unica_xml_format::read::child_element(*operation, "Properties");
                let name = props
                    .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
                    .unwrap_or_else(|| "(unnamed)".to_string());
                if props
                    .and_then(|node| {
                        unica_xml_format::read::child_direct_text(node, "XDTOReturningValueType")
                    })
                    .is_none_or(|value| value.trim().is_empty())
                {
                    findings.push(ServiceSemanticsFinding {
                        error: false,
                        detail: format!("Operation '{name}': no XDTOReturningValueType"),
                    });
                }
                if let Some(child_objects) =
                    unica_xml_format::read::child_element(*operation, "ChildObjects")
                {
                    for param in unica_xml_format::read::child_elements(child_objects, "Parameter")
                    {
                        param_count += 1;
                        let direction = unica_xml_format::read::child_element(param, "Properties")
                            .and_then(|node| {
                                unica_xml_format::read::child_direct_text(node, "TransferDirection")
                            });
                        if let Some(direction) = direction.filter(|value| !value.is_empty()) {
                            if !meta_validate_valid_transfer_directions()
                                .contains(&direction.as_str())
                            {
                                findings.push(ServiceSemanticsFinding {
                                    error: true,
                                    detail: format!(
                                        "Operation '{name}': Parameter has invalid TransferDirection '{direction}'"
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
        let clean = findings.iter().all(|finding| !finding.error);
        return Some(ServiceSemantics {
            findings,
            summary: format!("{operation_count} operation(s), {param_count} parameter(s)"),
            clean,
        });
    }
    None
}

pub fn meta_validate_check_services(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    if child_obj_node.is_none() {
        return;
    }
    let Some(semantics) = service_child_semantics(md_type, child_obj_node) else {
        return;
    };
    for finding in &semantics.findings {
        let message = format!("11. {md_type} {}", finding.detail);
        if finding.error {
            report.error(message);
        } else {
            report.warn(message);
        }
    }
    if semantics.clean {
        report.ok(format!("11. {md_type}: {}", semantics.summary));
    }
}

pub fn meta_validate_check_forbidden_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let Some(forbidden) = meta_validate_forbidden_properties(md_type) else {
        return;
    };
    let mut check_ok = true;
    for property in forbidden {
        if unica_xml_format::read::child_element(props_node, property).is_some() {
            report.error(format!(
                "12. Forbidden property '{property}' present in {md_type} (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("12. Forbidden properties: none found");
    }
}

pub fn meta_validate_check_method_reference(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<MetadataValidationSource<'_>>,
    proof_subject: Option<&unica_metadata::validation_subject::MetadataValidationSubject>,
) {
    if !matches!(md_type, "EventSubscription" | "ScheduledJob") {
        return;
    }
    let Some(props_node) = props_node else {
        return;
    };
    let (property, method_ref) = if md_type == "EventSubscription" {
        (
            "Handler",
            unica_xml_format::read::child_direct_text(props_node, "Handler"),
        )
    } else {
        (
            "MethodName",
            unica_xml_format::read::child_direct_text(props_node, "MethodName"),
        )
    };
    let Some(method_ref) = method_ref.filter(|value| !value.is_empty()) else {
        return;
    };
    let parts = method_ref.split('.').collect::<Vec<_>>();
    let parsed = if parts.len() == 3 && parts[0] == "CommonModule" {
        Some((parts[1], parts[2]))
    } else if parts.len() == 2 {
        Some((parts[0], parts[1]))
    } else {
        None
    };
    let Some((module_name, proc_name)) = parsed else {
        report.error(format!(
            "13. {md_type}.{property} = '{method_ref}': expected format 'CommonModule.ModuleName.ProcedureName'"
        ));
        return;
    };
    if !unica_metadata::identifier::is_1c_identifier(module_name)
        || !unica_metadata::identifier::is_1c_identifier(proc_name)
    {
        report.error(format!(
            "13. {md_type}.{property} = '{method_ref}': expected format 'CommonModule.ModuleName.ProcedureName'"
        ));
        return;
    }
    if let Some(subject) = proof_subject {
        let module_reference = format!("CommonModule.{module_name}");
        let module_target = match MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &module_reference,
        ) {
            Ok(target) => target,
            Err(_) => {
                report.error(format!(
                    "13. {md_type}.{property} = '{method_ref}': expected format 'CommonModule.ModuleName.ProcedureName'"
                ));
                return;
            }
        };
        let descriptor_found = subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Dependency { target } if target == &module_target
            )
        });
        if !descriptor_found {
            report.error(format!(
                "13. {md_type}.{property}: CommonModule `{module_name}` referenced by {property} is unavailable"
            ));
            return;
        }
        let module = subject.resources.iter().find(|resource| {
            matches!(
                &resource.role,
                unica_metadata::validation_subject::MetadataResourceRole::Module { owner } if owner == &module_target
            )
        });
        let Some(module) = module else {
            report.error(format!(
                "13. {md_type}.{property}: BSL file not found, cannot verify procedure"
            ));
            return;
        };
        let content = std::str::from_utf8(&module.bytes)
            .expect("module encodings are validated before semantic reporting");
        if let Err(reason) =
            meta_validate_method_signature(content, md_type, property, module_name, proc_name)
        {
            report.error(reason);
            return;
        }
        report.ok(format!("13. Method reference: {property} = '{method_ref}'"));
        return;
    }
    let Some(source) = config_dir else {
        return;
    };
    let config_dir = source.config_dir;
    let reader = source.reader;
    let module_xml = config_dir
        .join("CommonModules")
        .join(format!("{module_name}.xml"));
    if !path_exists(reader, &module_xml) {
        report.error(format!(
            "13. {md_type}.{property}: CommonModule '{module_name}' not found (expected {})",
            module_xml.display()
        ));
        return;
    }
    let bsl_path = config_dir
        .join("CommonModules")
        .join(module_name)
        .join("Ext")
        .join("Module.bsl");
    if path_exists(reader, &bsl_path) {
        if let Ok(content) = read_utf8_sig(reader, &bsl_path) {
            if let Err(reason) =
                meta_validate_method_signature(&content, md_type, property, module_name, proc_name)
            {
                report.warn(reason);
                return;
            }
        }
    } else {
        report.warn(format!(
            "13. {md_type}.{property}: BSL file not found ({}), cannot verify procedure",
            bsl_path.display()
        ));
        return;
    }
    report.ok(format!("13. Method reference: {property} = '{method_ref}'"));
}

pub fn meta_validate_check_document_journal_columns(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    if md_type != "DocumentJournal" {
        return;
    }
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let columns = unica_xml_format::read::child_elements(child_obj_node, "Column");
    let mut check_ok = true;
    let mut empty_ref_count = 0usize;
    for column in &columns {
        let props = unica_xml_format::read::child_element(*column, "Properties");
        let name = props
            .and_then(|node| unica_xml_format::read::child_direct_text(node, "Name"))
            .unwrap_or_else(|| "(unnamed)".to_string());
        let has_items = props
            .and_then(|node| unica_xml_format::read::child_element(node, "References"))
            .map(|node| !unica_xml_format::read::child_elements(node, "Item").is_empty())
            .unwrap_or(false);
        if !has_items {
            report.error(format!(
                "14. DocumentJournal Column '{name}': empty References (will fail on LoadConfigFromFiles)"
            ));
            check_ok = false;
            empty_ref_count += 1;
        }
    }
    if check_ok && !columns.is_empty() {
        report.ok(format!(
            "14. DocumentJournal Columns: {} column(s), all have References",
            columns.len()
        ));
    } else if columns.is_empty() && empty_ref_count == 0 {
        report.ok("14. DocumentJournal Columns: none");
    }
}

pub fn meta_validate_method_signature(
    content: &str,
    md_type: &str,
    property: &str,
    module_name: &str,
    proc_name: &str,
) -> Result<(), String> {
    let scheduled = md_type == "ScheduledJob";
    let found = if scheduled {
        exported_bsl_procedure_matches(
            content.trim_start_matches('\u{feff}'),
            proc_name,
            BslProcedureRequirement::CallableWithoutArguments,
        ).map_err(|reason| format!(
            "13. {md_type}.{property}: cannot verify procedure '{proc_name}' in CommonModule '{module_name}': {reason}"
        ))?
    } else {
        meta_validate_bsl_has_export(content, proc_name)
    };
    if found {
        Ok(())
    } else {
        let signature = if scheduled {
            " and callable without arguments"
        } else {
            ""
        };
        Err(format!(
            "13. {md_type}.{property}: procedure '{proc_name}' not found as exported{signature} in CommonModule '{module_name}'"
        ))
    }
}

pub fn meta_validate_bsl_has_export(content: &str, proc_name: &str) -> bool {
    content.trim_start_matches('\u{feff}').lines().any(|line| {
        let trimmed = line.trim_start();
        let starts = ["Procedure", "Function", "Процедура", "Функция"]
            .iter()
            .any(|prefix| trimmed.starts_with(prefix));
        starts
            && trimmed.contains(proc_name)
            && (trimmed.contains(" Export") || trimmed.contains(" Экспорт"))
    })
}

pub fn meta_validate_generated_categories(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" | "Document" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "Enum" => Some(&["Ref", "Manager", "List"]),
        "Constant" => Some(&["Manager", "ValueManager", "ValueKey"]),
        "InformationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "RecordManager",
        ]),
        "AccumulationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
        ]),
        "AccountingRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "ExtDimensions",
        ]),
        "CalculationRegister" => Some(&[
            "Record",
            "Manager",
            "Selection",
            "List",
            "RecordSet",
            "RecordKey",
            "Recalcs",
        ]),
        "ChartOfAccounts" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "ExtDimensionTypes",
            "ExtDimensionTypesRow",
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "Characteristic",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "DisplacingCalculationTypes",
            "DisplacingCalculationTypesRow",
            "BaseCalculationTypes",
            "BaseCalculationTypesRow",
            "LeadingCalculationTypes",
            "LeadingCalculationTypesRow",
        ]),
        "BusinessProcess" => Some(&[
            "Object",
            "Ref",
            "Selection",
            "List",
            "Manager",
            "RoutePointRef",
        ]),
        "Task" | "ExchangePlan" => Some(&["Object", "Ref", "Selection", "List", "Manager"]),
        "DocumentJournal" => Some(&["Selection", "List", "Manager"]),
        "Report" | "DataProcessor" => Some(&["Object", "Manager"]),
        "ExternalReport" | "ExternalDataProcessor" => Some(&["Object"]),
        "DefinedType" => Some(&["DefinedType"]),
        _ => None,
    }
}

pub fn meta_validate_types_without_internal_info() -> &'static [&'static str] {
    &["CommonModule", "ScheduledJob", "EventSubscription"]
}

pub fn meta_validate_types_with_std_attrs() -> &'static [&'static str] {
    &[
        "Catalog",
        "Document",
        "Enum",
        "InformationRegister",
        "AccumulationRegister",
        "AccountingRegister",
        "CalculationRegister",
        "ChartOfAccounts",
        "ChartOfCharacteristicTypes",
        "ChartOfCalculationTypes",
        "BusinessProcess",
        "Task",
        "ExchangePlan",
        "DocumentJournal",
    ]
}

pub fn meta_validate_standard_attributes(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "Catalog" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "IsFolder",
            "Owner",
            "Parent",
            "Description",
            "Code",
        ]),
        "Document" => Some(&["Posted", "Ref", "DeletionMark", "Date", "Number"]),
        "Enum" => Some(&["Order", "Ref"]),
        "InformationRegister" => Some(&["Active", "LineNumber", "Recorder", "Period"]),
        "AccumulationRegister" => {
            Some(&["RecordType", "Active", "LineNumber", "Recorder", "Period"])
        }
        "AccountingRegister" => Some(&[
            "Account",
            "RecordType",
            "Active",
            "LineNumber",
            "Recorder",
            "Period",
        ]),
        "CalculationRegister" => Some(&[
            "RegistrationPeriod",
            "ReversingEntry",
            "Active",
            "EndOfBasePeriod",
            "BegOfBasePeriod",
            "EndOfActionPeriod",
            "BegOfActionPeriod",
            "ActionPeriod",
            "CalculationType",
            "LineNumber",
            "Recorder",
        ]),
        "ChartOfAccounts" => Some(&[
            "PredefinedDataName",
            "Order",
            "OffBalance",
            "Type",
            "Description",
            "Code",
            "Parent",
            "Predefined",
            "DeletionMark",
            "Ref",
        ]),
        "ChartOfCharacteristicTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "Description",
            "Code",
            "Parent",
            "IsFolder",
            "ValueType",
        ]),
        "ChartOfCalculationTypes" => Some(&[
            "PredefinedDataName",
            "Predefined",
            "Ref",
            "DeletionMark",
            "ActionPeriodIsBasic",
            "Description",
            "Code",
        ]),
        "BusinessProcess" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Started",
            "Completed",
            "HeadTask",
        ]),
        "Task" => Some(&[
            "Ref",
            "DeletionMark",
            "Date",
            "Number",
            "Executed",
            "Description",
            "RoutePoint",
            "BusinessProcess",
        ]),
        "ExchangePlan" => Some(&[
            "Ref",
            "DeletionMark",
            "Code",
            "Description",
            "ThisNode",
            "SentNo",
            "ReceivedNo",
        ]),
        "DocumentJournal" => Some(&["Type", "Ref", "Date", "Posted", "DeletionMark", "Number"]),
        _ => None,
    }
}

pub fn meta_validate_dynamic_standard_attr(md_type: &str, name: &str) -> bool {
    (md_type == "AccountingRegister"
        && (name == "PeriodAdjustment"
            || name
                .strip_prefix("ExtDimension")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))
            || name
                .strip_prefix("ExtDimensionType")
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_digit()))))
        || (md_type == "CalculationRegister"
            && matches!(
                name,
                "ActionPeriod"
                    | "BegOfActionPeriod"
                    | "EndOfActionPeriod"
                    | "BegOfBasePeriod"
                    | "EndOfBasePeriod"
            ))
}

pub fn meta_validate_child_rules(md_type: &str) -> Option<Vec<&'static str>> {
    if matches!(md_type, "ExternalReport" | "ExternalDataProcessor") {
        return Some(vec![
            "Attribute",
            "TabularSection",
            "Form",
            "Template",
            "Command",
        ]);
    }
    let kind = MetadataKind::parse(md_type).ok()?;
    let mut rules = metadata_kind_collections(kind)
        .iter()
        .filter_map(|collection| match collection {
            MetaCollection::Attributes => Some("Attribute"),
            MetaCollection::TabularSections => Some("TabularSection"),
            MetaCollection::Dimensions => Some("Dimension"),
            MetaCollection::Resources => Some("Resource"),
            MetaCollection::EnumValues => Some("EnumValue"),
            MetaCollection::Columns => Some("Column"),
            MetaCollection::Forms => Some("Form"),
            MetaCollection::Templates => Some("Template"),
            MetaCollection::Commands => Some("Command"),
            MetaCollection::PredefinedItems => None,
        })
        .collect::<Vec<_>>();
    match kind {
        MetadataKind::ChartOfAccounts => {
            rules.extend(["AccountingFlag", "ExtDimensionAccountingFlag"])
        }
        MetadataKind::Task => rules.push("AddressingAttribute"),
        MetadataKind::CalculationRegister => rules.push("Recalculation"),
        MetadataKind::HTTPService => rules.push("URLTemplate"),
        MetadataKind::WebService => rules.push("Operation"),
        _ => {}
    }
    Some(rules)
}

pub fn meta_validate_reserved_attr_names() -> &'static [&'static str] {
    &[
        "Ref",
        "DeletionMark",
        "Code",
        "Description",
        "Date",
        "Number",
        "Posted",
        "Parent",
        "Owner",
        "IsFolder",
        "Predefined",
        "PredefinedDataName",
        "Recorder",
        "Period",
        "LineNumber",
        "Active",
        "Order",
        "Type",
        "OffBalance",
        "Started",
        "Completed",
        "HeadTask",
        "Executed",
        "RoutePoint",
        "BusinessProcess",
        "ThisNode",
        "SentNo",
        "ReceivedNo",
        "CalculationType",
        "RegistrationPeriod",
        "ReversingEntry",
        "Account",
        "ValueType",
        "ActionPeriodIsBasic",
    ]
}

pub fn meta_validate_valid_transfer_directions() -> &'static [&'static str] {
    crate::validation::meta_validate_property_enum("TransferDirection")
}

pub fn meta_validate_forbidden_properties(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "ChartOfCharacteristicTypes" => Some(&["CodeType"]),
        "ChartOfAccounts" => Some(&["Autonumbering", "Hierarchical"]),
        "ChartOfCalculationTypes" => Some(&["CheckUnique", "Autonumbering"]),
        "ExchangePlan" => Some(&["CodeType", "CheckUnique", "Autonumbering"]),
        _ => None,
    }
}
