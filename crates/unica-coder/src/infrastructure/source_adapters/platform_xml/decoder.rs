use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    domain::{
        navigation::CoverageState,
        source_adapters::{
            SnapshotConsistency, SourceAdapterError, SourceAdapterErrorKind, SourceDescriptor,
            SourceFamily, SourceSnapshot,
        },
    },
    infrastructure::native_operations::common::is_1c_identifier,
};

use super::{
    native_model::{
        NativeChildKind, NativeContentEvidence, NativeDescriptorEvidence, NativeEvidenceState,
        NativeForm, NativeMetadataClass, NativeMetadataObject, NativeMxlRootKind,
        NativeNamedChild, NativeProperty, NativePropertyProvenance, NativePropertyValue,
        NativeRegistrationEvidence, NativeTemplate, PlatformXmlNativeSnapshot,
    },
    provider::PlatformXmlProvider,
    schema::{metadata_class_profile, ChildObjectsVocabulary},
};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const MANAGED_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";
const SPREADSHEET_DOCUMENT_NAMESPACE: &str = "http://v8.1c.ru/spreadsheet/document";
const LEGACY_SPREADSHEET_NAMESPACE: &str = "http://v8.1c.ru/8.2/data/spreadsheet";

pub(crate) fn decode(
    provider: &PlatformXmlProvider,
    descriptor: &SourceDescriptor,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError> {
    if descriptor.family != SourceFamily::PlatformXml || descriptor.format_version.to_string() != "2.20" {
        return Err(error(
            SourceAdapterErrorKind::FormatUnsupported,
            "decoder supports exactly Platform XML 2.20",
        ));
    }

    let evidence = descriptor.snapshot_evidence.as_ref().ok_or_else(|| {
        error(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "Platform XML descriptor has no typed snapshot evidence",
        )
    })?;
    let revision = provider.revision()?;
    if revision != evidence.revision {
        return Err(error(
            SourceAdapterErrorKind::SnapshotStale,
            "Platform XML aggregate revision no longer matches probe evidence",
        ));
    }

    let mut roots = provider
        .snapshot_files()
        .filter(|(_, bytes)| digest(bytes) == evidence.root_descriptor_digest);
    let (root_key, root_bytes) = roots.next().ok_or_else(|| {
        error(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "Platform XML root descriptor digest is absent from the snapshot",
        )
    })?;
    if roots.next().is_some() {
        return Err(error(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "Platform XML root descriptor digest identifies multiple snapshot files",
        ));
    }

    let xml = utf8(&root_bytes, "Platform XML root descriptor is not valid UTF-8")?;
    let document = Document::parse(xml)
        .map_err(|_| corrupted("Platform XML root descriptor is malformed"))?;
    let wrapper = document.root_element();
    if wrapper.tag_name().name() != "MetaDataObject"
        || wrapper.tag_name().namespace() != Some(METADATA_NAMESPACE)
    {
        return Err(corrupted("Platform XML root descriptor identity is invalid"));
    }
    if wrapper.attribute("version").map(str::trim) != Some("2.20") {
        return Err(error(
            SourceAdapterErrorKind::FormatUnsupported,
            "Platform XML root descriptor is not version 2.20",
        ));
    }

    let classes = wrapper
        .children()
        .filter(Node::is_element)
        .collect::<Vec<_>>();
    let class = match classes.as_slice() {
        [class] => *class,
        [] => return Err(corrupted("Platform XML root descriptor has no metadata class")),
        _ => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "Platform XML root descriptor has multiple metadata classes",
            ));
        }
    };
    if class.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(corrupted("Platform XML metadata class namespace is invalid"));
    }
    let profile = metadata_class_profile(class.tag_name().name())
        .ok_or_else(|| corrupted("Platform XML metadata class is not in the shared schema registry"))?;
    let properties_node = required_unique_child(class, "Properties")?;
    let name = required_name(properties_node)?;
    let expected_file_name = if profile.class_name == "Configuration" {
        "Configuration.xml".to_string()
    } else {
        format!("{name}.xml")
    };
    if root_key.rsplit('/').next() != Some(expected_file_name.as_str()) {
        return Err(error(
            SourceAdapterErrorKind::IdentityCollision,
            "Platform XML root descriptor filename does not match native identity",
        ));
    }

    let uuid = parse_optional_uuid(class)?;
    let properties = decode_properties(properties_node)?;
    let child_objects = optional_unique_child(class, "ChildObjects")?;
    let base_key = root_key
        .strip_suffix(".xml")
        .ok_or_else(|| corrupted("Platform XML root descriptor is not an XML file"))?;

    let (attributes, tabular_sections, commands, forms, templates, coverage) =
        match (profile.child_objects, child_objects) {
            (_, None) => (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                CoverageState::Complete,
            ),
            (ChildObjectsVocabulary::ConfigurationTopLevel, Some(children)) => {
                validate_configuration_children(children)?;
                (
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    CoverageState::Partial,
                )
            }
            (ChildObjectsVocabulary::Object, Some(children)) => {
                validate_object_child_namespaces(children)?;
                let attributes = collect_unique_named_children(
                    children
                        .children()
                        .filter(|node| is_metadata_element(*node, "Attribute")),
                    NativeChildKind::Attribute,
                )?;
                let tabular_sections = collect_unique_named_children(
                    children
                        .children()
                        .filter(|node| is_metadata_element(*node, "TabularSection")),
                    NativeChildKind::TabularSection,
                )?;
                let commands = collect_unique_named_children(
                    children
                        .children()
                        .filter(|node| is_metadata_element(*node, "Command")),
                    NativeChildKind::Command,
                )?;
                let forms = decode_forms(provider, base_key, children)?;
                let templates = decode_templates(provider, base_key, children)?;
                (
                    attributes,
                    tabular_sections,
                    commands,
                    forms,
                    templates,
                    CoverageState::Complete,
                )
            }
        };

    Ok(PlatformXmlNativeSnapshot {
        source: SourceSnapshot {
            source_id: descriptor.source_id.clone(),
            revision,
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "platform-xml-2.20".to_string(),
        },
        root: NativeMetadataObject {
            class: NativeMetadataClass {
                canonical_name: profile.class_name,
            },
            uuid,
            name,
            attributes,
            tabular_sections,
            commands,
            forms,
            templates,
            properties,
        },
        coverage,
    })
}

fn collect_unique_named_children<'a, 'input>(
    nodes: impl Iterator<Item = Node<'a, 'input>>,
    kind: NativeChildKind,
) -> Result<Vec<NativeNamedChild>, SourceAdapterError>
where
    'input: 'a,
{
    let mut names = BTreeSet::new();
    let mut children = Vec::new();
    for node in nodes {
        let properties = required_unique_child(node, "Properties")?;
        let name = required_name(properties)?;
        if !names.insert(name.clone()) {
            return Err(error(
                SourceAdapterErrorKind::IdentityCollision,
                "Platform XML owner has duplicate child identities of the same kind",
            ));
        }
        children.push(NativeNamedChild {
            kind,
            uuid: parse_optional_uuid(node)?,
            name,
        });
    }
    Ok(children)
}

fn decode_forms(
    provider: &PlatformXmlProvider,
    base_key: &str,
    child_objects: Node<'_, '_>,
) -> Result<Vec<NativeForm>, SourceAdapterError> {
    let registrations = collect_registrations(
        child_objects
            .children()
            .filter(|node| is_metadata_element(*node, "Form")),
    )?;
    registrations
        .into_iter()
        .map(|registration| {
            let descriptor_key = format!("{base_key}/Forms/{}.xml", registration.name);
            let content_key = format!("{base_key}/Forms/{}/Ext/Form.xml", registration.name);
            let (descriptor, _) = decode_registered_descriptor(
                provider,
                &descriptor_key,
                "Form",
                &registration.name,
            )?;
            let managed_content = match snapshot_file(provider, &content_key) {
                None => content_evidence(NativeEvidenceState::Absent, content_key, None),
                Some(bytes) if managed_form_source_is_valid_bytes(&bytes) => content_evidence(
                    NativeEvidenceState::Validated,
                    content_key,
                    Some(digest(&bytes)),
                ),
                Some(_) => content_evidence(NativeEvidenceState::Unresolved, content_key, None),
            };
            Ok(NativeForm {
                registration,
                descriptor,
                managed_content,
            })
        })
        .collect()
}

fn decode_templates(
    provider: &PlatformXmlProvider,
    base_key: &str,
    child_objects: Node<'_, '_>,
) -> Result<Vec<NativeTemplate>, SourceAdapterError> {
    let registrations = collect_registrations(
        child_objects
            .children()
            .filter(|node| is_metadata_element(*node, "Template")),
    )?;
    registrations
        .into_iter()
        .map(|registration| {
            let descriptor_key = format!("{base_key}/Templates/{}.xml", registration.name);
            let (descriptor, descriptor_type) = decode_registered_descriptor(
                provider,
                &descriptor_key,
                "Template",
                &registration.name,
            )?;
            let prefix = format!("{base_key}/Templates/{}/Ext/", registration.name);
            let mut candidates = provider
                .snapshot_files()
                .filter(|(key, _)| direct_template_candidate(key, &prefix))
                .collect::<Vec<_>>();
            if candidates.len() > 1 {
                return Err(error(
                    SourceAdapterErrorKind::ProjectionAmbiguous,
                    "Platform XML template has multiple direct content candidates",
                ));
            }

            let (canonical_content, mxl_root_kind) = match candidates.pop() {
                None => (
                    content_evidence(
                        NativeEvidenceState::Absent,
                        format!("{prefix}Template.xml"),
                        None,
                    ),
                    None,
                ),
                Some((key, bytes)) => {
                    let mxl_root_kind = mxl_root_kind_bytes(&bytes);
                    let is_spreadsheet = matches!(
                        &descriptor_type,
                        NativePropertyValue::Scalar(value) if value == "SpreadsheetDocument"
                    );
                    let validated = if is_spreadsheet {
                        key == format!("{prefix}Template.xml") && mxl_root_kind.is_some()
                    } else {
                        true
                    };
                    (
                        content_evidence(
                            if validated {
                                NativeEvidenceState::Validated
                            } else {
                                NativeEvidenceState::Unresolved
                            },
                            key.to_string(),
                            validated.then(|| digest(&bytes)),
                        ),
                        mxl_root_kind,
                    )
                }
            };

            Ok(NativeTemplate {
                registration,
                descriptor,
                descriptor_type,
                canonical_content,
                mxl_root_kind,
            })
        })
        .collect()
}

fn collect_registrations<'a, 'input>(
    nodes: impl Iterator<Item = Node<'a, 'input>>,
) -> Result<Vec<NativeRegistrationEvidence>, SourceAdapterError>
where
    'input: 'a,
{
    let mut names = BTreeSet::new();
    let mut registrations = Vec::new();
    for node in nodes {
        let name = registration_name(node)?;
        if !names.insert(name.clone()) {
            return Err(error(
                SourceAdapterErrorKind::IdentityCollision,
                "Platform XML owner has duplicate registered child identities",
            ));
        }
        registrations.push(NativeRegistrationEvidence {
            uuid: parse_optional_uuid(node)?,
            name,
        });
    }
    Ok(registrations)
}

fn registration_name(node: Node<'_, '_>) -> Result<String, SourceAdapterError> {
    let properties = direct_children(node, "Properties");
    let direct_text = node
        .children()
        .filter(Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>();
    let direct_text = direct_text.trim();
    let name = match properties.as_slice() {
        [] if node.children().any(|child| child.is_element()) => {
            return Err(corrupted(
                "Platform XML registration has unsupported nested identity",
            ));
        }
        [] => direct_text.to_string(),
        [properties] if direct_text.is_empty() => required_name(*properties)?,
        [_] => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "Platform XML registration has conflicting identity fields",
            ));
        }
        _ => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "Platform XML registration has multiple Properties fields",
            ));
        }
    };
    validate_name(name)
}

fn decode_registered_descriptor(
    provider: &PlatformXmlProvider,
    relative_key: &str,
    expected_class: &str,
    expected_name: &str,
) -> Result<(NativeDescriptorEvidence, NativePropertyValue), SourceAdapterError> {
    let Some(bytes) = snapshot_file(provider, relative_key) else {
        return Ok((
            descriptor_evidence(NativeEvidenceState::Absent, relative_key, None),
            NativePropertyValue::Absent,
        ));
    };
    match parse_registered_descriptor(&bytes, expected_class, expected_name)? {
        Some(parsed) => Ok((
            descriptor_evidence(
                NativeEvidenceState::Validated,
                relative_key,
                parsed.uuid,
            ),
            parsed.template_type,
        )),
        None => Ok((
            descriptor_evidence(NativeEvidenceState::Unresolved, relative_key, None),
            NativePropertyValue::Unresolved,
        )),
    }
}

fn parse_registered_descriptor(
    bytes: &[u8],
    expected_class: &str,
    expected_name: &str,
) -> Result<Option<ParsedDescriptor>, SourceAdapterError> {
    let Ok(xml) = utf8(bytes, "registered descriptor is not valid UTF-8") else {
        return Ok(None);
    };
    let Ok(document) = Document::parse(xml) else {
        return Ok(None);
    };
    let wrapper = document.root_element();
    if wrapper.tag_name().name() != "MetaDataObject"
        || wrapper.tag_name().namespace() != Some(METADATA_NAMESPACE)
    {
        return Ok(None);
    }
    let classes = wrapper
        .children()
        .filter(Node::is_element)
        .collect::<Vec<_>>();
    let class = match classes.as_slice() {
        [class]
            if class.tag_name().name() == expected_class
                && class.tag_name().namespace() == Some(METADATA_NAMESPACE) =>
        {
            *class
        }
        [_] | [] => return Ok(None),
        _ => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "registered descriptor has multiple metadata classes",
            ));
        }
    };
    let Some(properties) = optional_unique_child(class, "Properties")? else {
        return Ok(None);
    };
    let name = match unique_scalar(properties, "Name")? {
        Some(name) => name,
        None => return Ok(None),
    };
    if name != expected_name || !is_1c_identifier(&name) {
        return Ok(None);
    }
    let uuid = match class.attribute("uuid") {
        Some(raw) => match Uuid::parse_str(raw) {
            Ok(uuid) => Some(uuid),
            Err(_) => return Ok(None),
        },
        None => None,
    };
    let template_type = if expected_class == "Template" {
        match unique_scalar(properties, "TemplateType")? {
            Some(value) => NativePropertyValue::Scalar(value),
            None => NativePropertyValue::Absent,
        }
    } else {
        NativePropertyValue::Absent
    };
    Ok(Some(ParsedDescriptor {
        uuid,
        template_type,
    }))
}

fn decode_properties(
    properties: Node<'_, '_>,
) -> Result<BTreeMap<String, NativeProperty>, SourceAdapterError> {
    let mut decoded = BTreeMap::new();
    for property in properties.children().filter(Node::is_element) {
        if property.tag_name().namespace() != Some(METADATA_NAMESPACE) {
            return Err(corrupted("Platform XML property namespace is invalid"));
        }
        let canonical_id = property.tag_name().name().to_string();
        if decoded.contains_key(&canonical_id) {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "Platform XML property occurs more than once",
            ));
        }
        let (value, provenance) = if property.children().any(|child| child.is_element()) {
            (
                NativePropertyValue::Unresolved,
                NativePropertyProvenance::Unresolved,
            )
        } else {
            let value = property.text().unwrap_or_default().trim();
            if value.is_empty() {
                (
                    NativePropertyValue::Absent,
                    NativePropertyProvenance::Absent,
                )
            } else {
                (
                    NativePropertyValue::Scalar(value.to_string()),
                    NativePropertyProvenance::Explicit,
                )
            }
        };
        decoded.insert(
            canonical_id.clone(),
            NativeProperty {
                canonical_id,
                value,
                provenance,
            },
        );
    }
    Ok(decoded)
}

fn required_name(properties: Node<'_, '_>) -> Result<String, SourceAdapterError> {
    let name = unique_scalar(properties, "Name")?
        .ok_or_else(|| corrupted("Platform XML native identity has no Name"))?;
    validate_name(name)
}

fn validate_name(name: String) -> Result<String, SourceAdapterError> {
    if name.is_empty() || !is_1c_identifier(&name) {
        return Err(corrupted("Platform XML native identity is not a 1C identifier"));
    }
    Ok(name)
}

fn unique_scalar(
    parent: Node<'_, '_>,
    local_name: &str,
) -> Result<Option<String>, SourceAdapterError> {
    let nodes = direct_children(parent, local_name);
    let node = match nodes.as_slice() {
        [] => return Ok(None),
        [node] => *node,
        _ => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                format!("Platform XML field `{local_name}` is ambiguous"),
            ));
        }
    };
    if node.children().any(|child| child.is_element()) {
        return Ok(None);
    }
    let value = node.text().unwrap_or_default().trim();
    Ok((!value.is_empty()).then(|| value.to_string()))
}

fn required_unique_child<'a, 'input>(
    parent: Node<'a, 'input>,
    local_name: &str,
) -> Result<Node<'a, 'input>, SourceAdapterError> {
    optional_unique_child(parent, local_name)?.ok_or_else(|| {
        corrupted(format!(
            "Platform XML metadata class has no `{local_name}` field"
        ))
    })
}

fn optional_unique_child<'a, 'input>(
    parent: Node<'a, 'input>,
    local_name: &str,
) -> Result<Option<Node<'a, 'input>>, SourceAdapterError> {
    let children = direct_children(parent, local_name);
    match children.as_slice() {
        [] => Ok(None),
        [child] => Ok(Some(*child)),
        _ => Err(error(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("Platform XML field `{local_name}` is ambiguous"),
        )),
    }
}

fn direct_children<'a, 'input>(
    parent: Node<'a, 'input>,
    local_name: &str,
) -> Vec<Node<'a, 'input>> {
    parent
        .children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().name() == local_name
                && child.tag_name().namespace() == Some(METADATA_NAMESPACE)
        })
        .collect()
}

fn validate_object_child_namespaces(child_objects: Node<'_, '_>) -> Result<(), SourceAdapterError> {
    for child in child_objects.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(METADATA_NAMESPACE)
            || !matches!(
                child.tag_name().name(),
                "Attribute" | "TabularSection" | "Form" | "Template" | "Command"
            )
        {
            return Err(corrupted(
                "Platform XML object contains an unsupported child structure",
            ));
        }
    }
    Ok(())
}

fn validate_configuration_children(child_objects: Node<'_, '_>) -> Result<(), SourceAdapterError> {
    for child in child_objects.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(METADATA_NAMESPACE)
            || metadata_class_profile(child.tag_name().name()).is_none()
        {
            return Err(corrupted(
                "Platform XML configuration contains an unsupported child class",
            ));
        }
    }
    Ok(())
}

fn is_metadata_element(node: Node<'_, '_>, local_name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == local_name
        && node.tag_name().namespace() == Some(METADATA_NAMESPACE)
}

fn parse_optional_uuid(node: Node<'_, '_>) -> Result<Option<Uuid>, SourceAdapterError> {
    node.attribute("uuid")
        .map(|raw| {
            Uuid::parse_str(raw)
                .map_err(|_| corrupted("Platform XML native UUID is invalid"))
        })
        .transpose()
}

fn snapshot_file(provider: &PlatformXmlProvider, key: &str) -> Option<std::sync::Arc<[u8]>> {
    provider
        .snapshot_files()
        .find_map(|(candidate, bytes)| (candidate == key).then_some(bytes))
}

fn direct_template_candidate(key: &str, prefix: &str) -> bool {
    let Some(file_name) = key.strip_prefix(prefix) else {
        return false;
    };
    !file_name.contains('/')
        && file_name
            .strip_prefix("Template.")
            .is_some_and(|extension| !extension.is_empty())
}

fn descriptor_evidence(
    state: NativeEvidenceState,
    relative_key: impl Into<String>,
    uuid: Option<Uuid>,
) -> NativeDescriptorEvidence {
    NativeDescriptorEvidence {
        state,
        relative_key: relative_key.into(),
        uuid,
    }
}

fn content_evidence(
    state: NativeEvidenceState,
    relative_key: impl Into<String>,
    digest: Option<String>,
) -> NativeContentEvidence {
    NativeContentEvidence {
        state,
        relative_key: relative_key.into(),
        digest,
    }
}

fn utf8<'a>(bytes: &'a [u8], message: &str) -> Result<&'a str, SourceAdapterError> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|_| corrupted(message))
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn managed_form_source_is_valid_bytes(bytes: &[u8]) -> bool {
    let Ok(xml) = utf8(bytes, "managed form source is not valid UTF-8") else {
        return false;
    };
    let Ok(document) = Document::parse(xml) else {
        return false;
    };
    let root = document.root_element();
    root.tag_name().name() == "Form"
        && root.tag_name().namespace() == Some(MANAGED_FORM_NAMESPACE)
}

fn mxl_root_kind_bytes(bytes: &[u8]) -> Option<NativeMxlRootKind> {
    let xml = utf8(bytes, "MXL source is not valid UTF-8").ok()?;
    let document = Document::parse(xml).ok()?;
    let root = document.root_element();
    match (root.tag_name().name(), root.tag_name().namespace()) {
        ("SpreadsheetDocument", Some(SPREADSHEET_DOCUMENT_NAMESPACE)) => {
            Some(NativeMxlRootKind::SpreadsheetDocument)
        }
        ("document", Some(LEGACY_SPREADSHEET_NAMESPACE)) => {
            Some(NativeMxlRootKind::LegacyDocument)
        }
        _ => None,
    }
}

pub(crate) fn descriptor_is_valid(
    xml: &str,
    expected_class: &str,
    expected_name: &str,
) -> bool {
    parse_registered_descriptor(xml.as_bytes(), expected_class, expected_name)
        .is_ok_and(|descriptor| descriptor.is_some())
}

pub(crate) fn template_type_if_valid(xml: &str, expected_name: &str) -> Option<String> {
    let descriptor =
        parse_registered_descriptor(xml.as_bytes(), "Template", expected_name).ok()??;
    match descriptor.template_type {
        NativePropertyValue::Scalar(value) => Some(value),
        NativePropertyValue::Absent | NativePropertyValue::Unresolved => None,
    }
}

pub(crate) fn managed_form_source_is_valid(xml: &str) -> bool {
    managed_form_source_is_valid_bytes(xml.as_bytes())
}

pub(crate) fn mxl_source_is_valid(xml: &str) -> bool {
    mxl_root_kind_bytes(xml.as_bytes()).is_some()
}

pub(crate) fn unique_direct_child_in_namespace<'a, 'input>(
    node: Node<'a, 'input>,
    local_name: &str,
    namespace: &str,
) -> Option<Node<'a, 'input>> {
    let mut children = node.children().filter(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(namespace)
    });
    let child = children.next()?;
    children.next().is_none().then_some(child)
}

struct ParsedDescriptor {
    uuid: Option<Uuid>,
    template_type: NativePropertyValue,
}

fn corrupted(message: impl Into<String>) -> SourceAdapterError {
    error(SourceAdapterErrorKind::DecodeCorrupted, message)
}

fn error(
    kind: SourceAdapterErrorKind,
    message: impl Into<String>,
) -> SourceAdapterError {
    SourceAdapterError::new(kind, message)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::domain::source_adapters::{
        FormatVersion, SnapshotEvidence, SourceAdapterErrorKind, SourceDescriptor, SourceFamily,
        SourceId, SourceRevision,
    };

    use super::{
        decode, NativeEvidenceState, NativeMxlRootKind, NativePropertyProvenance,
        NativePropertyValue, PlatformXmlProvider,
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn duplicate_inline_child_names_are_identity_collisions() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
              <TabularSection><Properties><Name>Lines</Name></Properties></TabularSection>
            </ChildObjects>
            "#,
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
    }

    #[test]
    fn invalid_inline_child_name_is_not_promoted_to_a_mutable_native_node() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <Attribute><Properties><Name>../Bad</Name></Properties></Attribute>
            </ChildObjects>
            "#,
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn conflicting_descriptor_fields_are_projection_ambiguity() {
        let fixture = fixture(
            "Shipment.xml",
            &[(
                "Shipment.xml",
                metadata_document(
                    "<Properties><Name>Shipment</Name><Name>Other</Name></Properties>",
                ),
            )],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn stale_revision_is_rejected_before_decode() {
        let mut fixture = fixture(
            "Shipment.xml",
            &[("Shipment.xml", "not XML".to_string())],
        );
        fixture.descriptor.snapshot_evidence.as_mut().unwrap().revision =
            SourceRevision::new("sha256:stale").unwrap();

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn inconsistent_root_digest_is_rejected_before_decode() {
        let mut fixture = fixture(
            "Shipment.xml",
            &[("Shipment.xml", "not XML".to_string())],
        );
        fixture
            .descriptor
            .snapshot_evidence
            .as_mut()
            .unwrap()
            .root_descriptor_digest = "sha256:inconsistent".to_string();

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotInconsistent);
    }

    #[test]
    fn root_identity_uses_shared_class_profile_uuid_and_source_snapshot() {
        let fixture = document_fixture(
            r#"
            <Properties>
              <Name>Shipment</Name>
              <Comment/>
              <Synonym><item>Shipment</item></Synonym>
            </Properties>
            <ChildObjects>
              <Attribute uuid="22222222-2222-2222-2222-222222222222">
                <Properties><Name>Number</Name></Properties>
              </Attribute>
              <Command><Properties><Name>Post</Name></Properties></Command>
            </ChildObjects>
            "#,
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(decoded.root.class.canonical_name, "Document");
        assert_eq!(
            decoded.root.uuid.unwrap().to_string(),
            "11111111-1111-1111-1111-111111111111"
        );
        assert_eq!(decoded.root.name, "Shipment");
        assert_eq!(decoded.source.source_id, fixture.descriptor.source_id);
        assert_eq!(
            decoded.root.attributes[0].uuid.unwrap().to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
        assert_eq!(decoded.root.attributes[0].name, "Number");
        assert_eq!(decoded.root.commands[0].name, "Post");
        assert_eq!(
            decoded.root.properties["Comment"].provenance,
            NativePropertyProvenance::Absent
        );
        assert_eq!(
            decoded.root.properties["Synonym"].value,
            NativePropertyValue::Unresolved
        );
    }

    #[test]
    fn root_filename_must_correspond_to_native_name() {
        let fixture = fixture(
            "Other.xml",
            &[("Other.xml", metadata_document(""))],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
    }

    #[test]
    fn form_registration_descriptor_and_managed_content_are_validated() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor("ItemForm"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();
        let form = &decoded.root.forms[0];

        assert_eq!(form.registration.name, "ItemForm");
        assert_eq!(form.descriptor.state, NativeEvidenceState::Validated);
        assert_eq!(form.managed_content.state, NativeEvidenceState::Validated);
    }

    #[test]
    fn malformed_managed_form_content_fails_closed() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor("ItemForm"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    "<Form xmlns=\"http://v8.1c.ru/8.3/MDClasses\"/>".to_string(),
                ),
            ],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(
            decoded.root.forms[0].managed_content.state,
            NativeEvidenceState::Unresolved
        );
    }

    #[test]
    fn conflicting_template_types_are_projection_ambiguity() {
        let descriptor = "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Template><Properties><Name>Print</Name><TemplateType>SpreadsheetDocument</TemplateType><TemplateType>Text</TemplateType></Properties></Template></MetaDataObject>";
        let fixture = document_fixture_with_files(
            "<ChildObjects><Template>Print</Template></ChildObjects>",
            &[("Shipment/Templates/Print.xml", descriptor.to_string())],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn spreadsheet_templates_require_canonical_content_and_known_mxl_roots() {
        for (file_name, content, expected_state, expected_root) in [
            (
                "Template.xml",
                spreadsheet_document_source(),
                NativeEvidenceState::Validated,
                Some(NativeMxlRootKind::SpreadsheetDocument),
            ),
            (
                "Template.xml",
                "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"/>",
                NativeEvidenceState::Validated,
                Some(NativeMxlRootKind::LegacyDocument),
            ),
            (
                "Template.mxl",
                spreadsheet_document_source(),
                NativeEvidenceState::Unresolved,
                Some(NativeMxlRootKind::SpreadsheetDocument),
            ),
            (
                "Template.xml",
                "<SpreadsheetDocument/>",
                NativeEvidenceState::Unresolved,
                None,
            ),
        ] {
            let content_key = format!("Shipment/Templates/Print/Ext/{file_name}");
            let fixture = document_fixture_with_files(
                "<ChildObjects><Template>Print</Template></ChildObjects>",
                &[
                    (
                        "Shipment/Templates/Print.xml",
                        template_descriptor("Print", "SpreadsheetDocument"),
                    ),
                    (&content_key, content.to_string()),
                ],
            );

            let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();
            let template = &decoded.root.templates[0];

            assert_eq!(template.descriptor.state, NativeEvidenceState::Validated);
            assert_eq!(
                template.descriptor_type,
                NativePropertyValue::Scalar("SpreadsheetDocument".to_string())
            );
            assert_eq!(template.canonical_content.state, expected_state);
            assert_eq!(template.mxl_root_kind, expected_root);
        }
    }

    #[test]
    fn invalid_registered_identity_is_rejected_before_snapshot_lookup() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>../../outside</Form></ChildObjects>",
            &[(
                "outside/Ext/Form.xml",
                managed_form_source().to_string(),
            )],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    fn document_fixture(body: &str) -> Fixture {
        fixture(
            "Shipment.xml",
            &[("Shipment.xml", metadata_document(body))],
        )
    }

    fn document_fixture_with_files(body: &str, other_files: &[(&str, String)]) -> Fixture {
        let root = metadata_document(body);
        let mut files = vec![("Shipment.xml", root)];
        files.extend(other_files.iter().cloned());
        fixture("Shipment.xml", &files)
    }

    fn metadata_document(body: &str) -> String {
        let body_has_properties = body.contains("<Properties>");
        let properties = if body_has_properties && body.trim_start().starts_with("<Properties>") {
            ""
        } else {
            "<Properties><Name>Shipment</Name></Properties>"
        };
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Document uuid="11111111-1111-1111-1111-111111111111">
    {properties}
    {body}
  </Document>
</MetaDataObject>"#
        )
    }

    fn form_descriptor(name: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Form><Properties><Name>{name}</Name></Properties></Form></MetaDataObject>"
        )
    }

    fn template_descriptor(name: &str, template_type: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Template><Properties><Name>{name}</Name><TemplateType>{template_type}</TemplateType></Properties></Template></MetaDataObject>"
        )
    }

    fn managed_form_source() -> &'static str {
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\"/>"
    }

    fn spreadsheet_document_source() -> &'static str {
        "<SpreadsheetDocument xmlns=\"http://v8.1c.ru/spreadsheet/document\"/>"
    }

    fn fixture(root_key: &str, files: &[(&str, String)]) -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-decoder-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        for (relative, content) in files {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
        }
        let provider = PlatformXmlProvider::open(&root).unwrap();
        let descriptor = SourceDescriptor {
            source_id: SourceId::new("workspace:main").unwrap(),
            family: SourceFamily::PlatformXml,
            format_version: FormatVersion::parse("2.20").unwrap(),
            producer_version: None,
            detected_features: BTreeSet::new(),
            probe_evidence: Vec::new(),
            snapshot_evidence: Some(SnapshotEvidence {
                revision: provider.revision().unwrap(),
                root_descriptor_digest: provider.digest_relative(root_key).unwrap(),
            }),
        };
        Fixture {
            root,
            provider,
            descriptor,
        }
    }

    struct Fixture {
        root: PathBuf,
        provider: PlatformXmlProvider,
        descriptor: SourceDescriptor,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
