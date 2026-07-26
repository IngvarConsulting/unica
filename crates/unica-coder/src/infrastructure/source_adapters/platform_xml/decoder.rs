use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    domain::{
        navigation::{CoverageState, RelationRole},
        source_adapters::{
            SnapshotConsistency, SourceAdapterError, SourceAdapterErrorKind, SourceDescriptor,
            SourceFamily, SourceSnapshot,
        },
    },
    infrastructure::{
        native_operations::common::is_1c_identifier,
        source_adapters::{ProbeOutcome, SourceInput},
    },
};

use super::{
    native_model::{
        NativeContentEvidence, NativeDescriptorEvidence, NativeEvidenceState, NativeForm,
        NativeMetadataChild, NativeMetadataClass, NativeMetadataNode, NativeMxlRootKind,
        NativeNodeBacking, NativeNodeState, NativeProperty, NativePropertyProvenance,
        NativePropertyValue, NativeRegistrationEvidence, NativeScalarAnnotationIssue,
        NativeScalarType, NativeTemplate, PlatformXmlNativeSnapshot,
    },
    probe::PlatformXmlProbe,
    provider::PlatformXmlProvider,
    schema::{
        child_metadata_class_profile, metadata_class_profile, parse_type_description_2_20,
        scalar_property_kind_2_20, ChildObjectsVocabulary, MetadataClassProfile, MetadataClassRole,
        ScalarPropertyKind,
    },
};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const MANAGED_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";
const SPREADSHEET_DOCUMENT_NAMESPACE: &str = "http://v8.1c.ru/spreadsheet/document";
const LEGACY_SPREADSHEET_NAMESPACE: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
const XML_SCHEMA_INSTANCE_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";

pub(crate) fn decode_path(
    input: &SourceInput,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError> {
    let provider = PlatformXmlProvider::capture(&input.target, &input.source_root)?;
    let descriptor_key = input
        .target
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| corrupted("Platform XML descriptor has no UTF-8 file name"))?;
    let descriptor =
        match PlatformXmlProbe::new().probe_provider(input, &provider, descriptor_key)? {
            ProbeOutcome::Match(descriptor) => descriptor,
            ProbeOutcome::NoMatch => {
                return Err(error(
                    SourceAdapterErrorKind::FormatUnsupported,
                    "source is not Platform XML",
                ));
            }
        };
    decode(&provider, &descriptor)
}

pub(crate) fn decode(
    provider: &PlatformXmlProvider,
    descriptor: &SourceDescriptor,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError> {
    if descriptor.family != SourceFamily::PlatformXml
        || descriptor.format_version.to_string() != "2.20"
    {
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

    let xml = utf8(
        &root_bytes,
        "Platform XML root descriptor is not valid UTF-8",
    )?;
    let document =
        Document::parse(xml).map_err(|_| corrupted("Platform XML root descriptor is malformed"))?;
    let wrapper = document.root_element();
    validate_metadata_wrapper(wrapper)?;
    if wrapper.attribute("version").map(str::trim) != Some("2.20") {
        return Err(error(
            SourceAdapterErrorKind::FormatUnsupported,
            "Platform XML root descriptor is not version 2.20",
        ));
    }

    let class = single_metadata_class(wrapper)?;
    let profile = profile_for_node(class)?;
    let properties = required_properties(class)?;
    let name = required_name(properties)?;
    let expected_file_name = if profile.role == MetadataClassRole::Configuration {
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
    let base_key = root_key
        .strip_suffix(".xml")
        .ok_or_else(|| corrupted("Platform XML root descriptor is not an XML file"))?;
    let mut context = DecodeContext::default();
    let decoded = decode_inline_node(provider, class, profile, base_key, xml, &mut context)?;

    Ok(PlatformXmlNativeSnapshot {
        source: SourceSnapshot {
            source_id: descriptor.source_id.clone(),
            revision,
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "platform-xml-2.20".to_string(),
        },
        root: decoded.node,
        coverage: if decoded.complete {
            CoverageState::Complete
        } else {
            CoverageState::Partial
        },
    })
}

fn decode_inline_node(
    provider: &PlatformXmlProvider,
    node: Node<'_, '_>,
    profile: &'static MetadataClassProfile,
    base_key: &str,
    source_xml: &str,
    context: &mut DecodeContext,
) -> Result<DecodedNode, SourceAdapterError> {
    let properties_node = required_properties(node)?;
    let name = required_name(properties_node)?;
    let properties = decode_properties(properties_node, source_xml)?;
    let uuid = parse_optional_uuid(node)?;
    context.register_uuid(uuid)?;
    let children = decode_children(provider, node, profile, base_key, source_xml, context)?;
    Ok(DecodedNode {
        node: NativeMetadataNode {
            class: native_class(profile),
            uuid,
            name,
            state: NativeNodeState::ResolvedInline,
            properties,
            children: children.nodes,
            backing: NativeNodeBacking::None,
        },
        complete: children.complete,
    })
}

fn decode_children(
    provider: &PlatformXmlProvider,
    owner: Node<'_, '_>,
    owner_profile: &'static MetadataClassProfile,
    base_key: &str,
    source_xml: &str,
    context: &mut DecodeContext,
) -> Result<DecodedChildren, SourceAdapterError> {
    let Some(child_objects) = optional_unique_child(owner, "ChildObjects")? else {
        return Ok(DecodedChildren {
            nodes: Vec::new(),
            complete: true,
        });
    };
    if owner_profile.child_objects == ChildObjectsVocabulary::None {
        return Err(corrupted(
            "Platform XML class contains ChildObjects forbidden by the schema registry",
        ));
    }

    let mut identities = BTreeSet::new();
    let mut nodes = Vec::new();
    let mut complete = true;
    for child in child_objects.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(METADATA_NAMESPACE) {
            return Err(corrupted("Platform XML child class namespace is invalid"));
        }
        let profile = child_metadata_class_profile(owner_profile, child.tag_name().name())
            .ok_or_else(|| {
                corrupted("Platform XML child class is not allowed by the schema registry")
            })?;
        let decoded = if matches!(
            profile.role,
            MetadataClassRole::Form | MetadataClassRole::Template
        ) {
            decode_backed_registration(provider, child, profile, base_key, source_xml, context)?
        } else if direct_children(child, "Properties").is_empty() {
            if owner_profile.child_objects != ChildObjectsVocabulary::ConfigurationTopLevel
                || profile.role != MetadataClassRole::TopLevelObject
            {
                return Err(corrupted(
                    "inline Platform XML child is missing required Properties",
                ));
            }
            decode_unresolved_registration(child, profile, context)?
        } else {
            decode_inline_node(provider, child, profile, base_key, source_xml, context)?
        };
        let identity = (profile.class_name, decoded.node.name.clone());
        if !identities.insert(identity) {
            return Err(error(
                SourceAdapterErrorKind::IdentityCollision,
                "Platform XML owner has duplicate child identities of the same class",
            ));
        }
        complete &= decoded.complete;
        nodes.push(NativeMetadataChild {
            role: relation_role_for_child_collection(owner_profile, profile),
            node: decoded.node,
        });
    }
    Ok(DecodedChildren { nodes, complete })
}

fn decode_unresolved_registration(
    node: Node<'_, '_>,
    profile: &'static MetadataClassProfile,
    context: &mut DecodeContext,
) -> Result<DecodedNode, SourceAdapterError> {
    let registration = registration(node)?;
    context.register_uuid(registration.uuid)?;
    Ok(DecodedNode {
        node: NativeMetadataNode {
            class: native_class(profile),
            uuid: registration.uuid,
            name: registration.name.clone(),
            state: NativeNodeState::UnresolvedRegistration {
                registration: registration.clone(),
            },
            properties: synthetic_name_property(&registration.name),
            children: Vec::new(),
            backing: NativeNodeBacking::None,
        },
        complete: false,
    })
}

fn decode_backed_registration(
    provider: &PlatformXmlProvider,
    node: Node<'_, '_>,
    profile: &'static MetadataClassProfile,
    base_key: &str,
    source_xml: &str,
    context: &mut DecodeContext,
) -> Result<DecodedNode, SourceAdapterError> {
    let registration = registration(node)?;
    let properties = match optional_unique_child(node, "Properties")? {
        Some(properties) => decode_properties(properties, source_xml)?,
        None => synthetic_name_property(&registration.name),
    };
    match profile.role {
        MetadataClassRole::Form => {
            let descriptor_key = format!("{base_key}/Forms/{}.xml", registration.name);
            let content_key = format!("{base_key}/Forms/{}/Ext/Form.xml", registration.name);
            let descriptor = match snapshot_file(provider, &descriptor_key) {
                Some(bytes) => {
                    let parsed = parse_registered_descriptor(&bytes, profile, &registration.name)?;
                    descriptor_evidence(NativeEvidenceState::Validated, descriptor_key, parsed.uuid)
                }
                None => descriptor_evidence(NativeEvidenceState::Absent, descriptor_key, None),
            };
            let managed_content = match snapshot_file(provider, &content_key) {
                Some(bytes) => {
                    validate_managed_form(&bytes)?;
                    content_evidence(
                        NativeEvidenceState::Validated,
                        content_key,
                        Some(digest(&bytes)),
                    )
                }
                None => content_evidence(NativeEvidenceState::Absent, content_key, None),
            };
            let complete = descriptor.state == NativeEvidenceState::Validated
                && managed_content.state == NativeEvidenceState::Validated;
            let effective_uuid = reconcile_registered_uuid(registration.uuid, descriptor.uuid)?;
            context.register_uuid(effective_uuid)?;
            let state = registration_state(&registration, complete);
            Ok(DecodedNode {
                node: NativeMetadataNode {
                    class: native_class(profile),
                    uuid: effective_uuid,
                    name: registration.name.clone(),
                    state,
                    properties,
                    children: Vec::new(),
                    backing: NativeNodeBacking::Form(NativeForm {
                        registration,
                        descriptor,
                        managed_content,
                    }),
                },
                complete,
            })
        }
        MetadataClassRole::Template => {
            let descriptor_key = format!("{base_key}/Templates/{}.xml", registration.name);
            let (descriptor, descriptor_type) = match snapshot_file(provider, &descriptor_key) {
                Some(bytes) => {
                    let parsed = parse_registered_descriptor(&bytes, profile, &registration.name)?;
                    (
                        descriptor_evidence(
                            NativeEvidenceState::Validated,
                            descriptor_key,
                            parsed.uuid,
                        ),
                        parsed.template_type,
                    )
                }
                None => (
                    descriptor_evidence(NativeEvidenceState::Absent, descriptor_key, None),
                    NativePropertyValue::Absent,
                ),
            };
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
                    if matches!(
                        &descriptor_type,
                        NativePropertyValue::Scalar(value) if value == "SpreadsheetDocument"
                    ) {
                        if key != format!("{prefix}Template.xml") {
                            return Err(corrupted(
                                "SpreadsheetDocument template content is not canonical Template.xml",
                            ));
                        }
                        let root_kind = parse_mxl_root(&bytes)?;
                        (
                            content_evidence(
                                NativeEvidenceState::Validated,
                                key.to_string(),
                                Some(digest(&bytes)),
                            ),
                            Some(root_kind),
                        )
                    } else if matches!(descriptor_type, NativePropertyValue::Absent) {
                        (
                            content_evidence(
                                NativeEvidenceState::Unresolved,
                                key.to_string(),
                                None,
                            ),
                            None,
                        )
                    } else {
                        (
                            content_evidence(
                                NativeEvidenceState::Validated,
                                key.to_string(),
                                Some(digest(&bytes)),
                            ),
                            None,
                        )
                    }
                }
            };
            let complete = descriptor.state == NativeEvidenceState::Validated
                && !matches!(
                    descriptor_type,
                    NativePropertyValue::Absent | NativePropertyValue::Unresolved
                )
                && canonical_content.state == NativeEvidenceState::Validated;
            let effective_uuid = reconcile_registered_uuid(registration.uuid, descriptor.uuid)?;
            context.register_uuid(effective_uuid)?;
            let state = registration_state(&registration, complete);
            Ok(DecodedNode {
                node: NativeMetadataNode {
                    class: native_class(profile),
                    uuid: effective_uuid,
                    name: registration.name.clone(),
                    state,
                    properties,
                    children: Vec::new(),
                    backing: NativeNodeBacking::Template(NativeTemplate {
                        registration,
                        descriptor,
                        descriptor_type,
                        canonical_content,
                        mxl_root_kind,
                    }),
                },
                complete,
            })
        }
        _ => unreachable!("schema-backed registrations are Form or Template"),
    }
}

fn parse_registered_descriptor(
    bytes: &[u8],
    expected_profile: &'static MetadataClassProfile,
    expected_name: &str,
) -> Result<ParsedDescriptor, SourceAdapterError> {
    let xml = utf8(bytes, "registered descriptor is not valid UTF-8")?;
    let document =
        Document::parse(xml).map_err(|_| corrupted("registered descriptor is malformed XML"))?;
    let wrapper = document.root_element();
    validate_metadata_wrapper(wrapper)?;
    let class = single_metadata_class(wrapper)?;
    let actual_profile = profile_for_node(class)?;
    if actual_profile != expected_profile {
        return Err(corrupted(
            "registered descriptor class does not match its registration",
        ));
    }
    let properties = required_properties(class)?;
    let name = required_name(properties)?;
    if name != expected_name {
        return Err(corrupted(
            "registered descriptor name does not match its registration",
        ));
    }
    let uuid = parse_optional_uuid(class)?;
    let template_type = if expected_profile.role == MetadataClassRole::Template {
        unique_scalar(properties, "TemplateType")?
            .map(NativePropertyValue::Scalar)
            .unwrap_or(NativePropertyValue::Absent)
    } else {
        NativePropertyValue::Absent
    };
    Ok(ParsedDescriptor {
        uuid,
        template_type,
    })
}

fn validate_metadata_wrapper(wrapper: Node<'_, '_>) -> Result<(), SourceAdapterError> {
    if wrapper.tag_name().name() != "MetaDataObject"
        || wrapper.tag_name().namespace() != Some(METADATA_NAMESPACE)
    {
        return Err(corrupted(
            "Platform XML descriptor root identity is invalid",
        ));
    }
    Ok(())
}

fn single_metadata_class<'a, 'input>(
    wrapper: Node<'a, 'input>,
) -> Result<Node<'a, 'input>, SourceAdapterError> {
    let classes = wrapper
        .children()
        .filter(Node::is_element)
        .collect::<Vec<_>>();
    match classes.as_slice() {
        [class] => Ok(*class),
        [] => Err(corrupted("Platform XML descriptor has no metadata class")),
        _ => Err(error(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            "Platform XML descriptor has multiple metadata classes",
        )),
    }
}

fn profile_for_node(
    node: Node<'_, '_>,
) -> Result<&'static MetadataClassProfile, SourceAdapterError> {
    if node.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(corrupted(
            "Platform XML metadata class namespace is invalid",
        ));
    }
    metadata_class_profile(node.tag_name().name())
        .ok_or_else(|| corrupted("Platform XML metadata class is absent from the schema registry"))
}

fn decode_properties(
    properties: Node<'_, '_>,
    _source_xml: &str,
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
            let value = if crate::infrastructure::source_adapters::platform_xml::schema::is_type_property_2_20(&canonical_id) {
                NativePropertyValue::TypeSet(parse_type_description_2_20(property)?)
            } else {
                NativePropertyValue::Structured
            };
            (value, NativePropertyProvenance::Explicit)
        } else {
            let value = property.text().unwrap_or_default().trim();
            let scalar = scalar_property_value(&canonical_id, property, value);
            let provenance = if matches!(scalar, NativePropertyValue::Absent) {
                NativePropertyProvenance::Absent
            } else if matches!(scalar, NativePropertyValue::UnresolvedScalar { .. }) {
                NativePropertyProvenance::Unresolved
            } else {
                NativePropertyProvenance::Explicit
            };
            (scalar, provenance)
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

fn scalar_property_value(
    canonical_id: &str,
    property: Node<'_, '_>,
    value: &str,
) -> NativePropertyValue {
    let annotation = property.attribute((XML_SCHEMA_INSTANCE_NAMESPACE, "type"));
    if annotation.is_some() && property.attribute("type").is_some() {
        return unresolved_scalar(NativeScalarAnnotationIssue::Conflicting);
    }
    let Some(annotation) = annotation else {
        if property.attribute("type").is_some() {
            return unresolved_scalar(NativeScalarAnnotationIssue::Unqualified);
        }
        if value.is_empty() {
            return NativePropertyValue::Absent;
        }
        if matches!(
            scalar_property_kind_2_20(canonical_id),
            Some(ScalarPropertyKind::PolymorphicFillValue)
        ) {
            return unresolved_scalar(NativeScalarAnnotationIssue::Missing);
        }
        return NativePropertyValue::Scalar(value.to_string());
    };
    let Some((prefix, local_name)) = annotation.split_once(':') else {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    };
    if prefix.is_empty()
        || local_name.is_empty()
        || local_name.contains(':')
        || property.lookup_namespace_uri(Some(prefix)) != Some(XML_SCHEMA_NAMESPACE)
    {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    }
    let type_annotation = match local_name {
        "string" => NativeScalarType::String,
        "boolean" => NativeScalarType::Boolean,
        "decimal" => NativeScalarType::Decimal,
        "integer" => NativeScalarType::Integer,
        "stringUuid" => NativeScalarType::Uuid,
        _ => return unresolved_scalar(NativeScalarAnnotationIssue::Unknown),
    };
    if value.is_empty() && !matches!(type_annotation, NativeScalarType::String) {
        return unresolved_scalar(NativeScalarAnnotationIssue::InvalidLexical);
    }
    NativePropertyValue::AnnotatedScalar {
        value: value.to_string(),
        type_annotation,
    }
}

fn unresolved_scalar(issue: NativeScalarAnnotationIssue) -> NativePropertyValue {
    NativePropertyValue::UnresolvedScalar { issue }
}

fn synthetic_name_property(name: &str) -> BTreeMap<String, NativeProperty> {
    BTreeMap::from([(
        "Name".to_string(),
        NativeProperty {
            canonical_id: "Name".to_string(),
            value: NativePropertyValue::Scalar(name.to_string()),
            provenance: NativePropertyProvenance::Explicit,
        },
    )])
}

fn registration(node: Node<'_, '_>) -> Result<NativeRegistrationEvidence, SourceAdapterError> {
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
    Ok(NativeRegistrationEvidence {
        uuid: parse_optional_uuid(node)?,
        name: validate_name(name)?,
    })
}

fn required_properties<'a, 'input>(
    node: Node<'a, 'input>,
) -> Result<Node<'a, 'input>, SourceAdapterError> {
    required_unique_child(node, "Properties")
}

fn required_name(properties: Node<'_, '_>) -> Result<String, SourceAdapterError> {
    let name = unique_scalar(properties, "Name")?
        .ok_or_else(|| corrupted("Platform XML native identity has no Name"))?;
    validate_name(name)
}

fn validate_name(name: String) -> Result<String, SourceAdapterError> {
    if !is_1c_identifier(&name) {
        return Err(corrupted(
            "Platform XML native identity is not a 1C identifier",
        ));
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
        return Err(corrupted(format!(
            "Platform XML scalar field `{local_name}` contains nested XML"
        )));
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

fn parse_optional_uuid(node: Node<'_, '_>) -> Result<Option<Uuid>, SourceAdapterError> {
    node.attribute("uuid")
        .map(|raw| {
            Uuid::parse_str(raw).map_err(|_| corrupted("Platform XML native UUID is invalid"))
        })
        .transpose()
}

fn validate_managed_form(bytes: &[u8]) -> Result<(), SourceAdapterError> {
    let xml = utf8(bytes, "managed Form content is not valid UTF-8")?;
    let document =
        Document::parse(xml).map_err(|_| corrupted("managed Form content is malformed XML"))?;
    let root = document.root_element();
    if root.tag_name().name() != "Form"
        || root.tag_name().namespace() != Some(MANAGED_FORM_NAMESPACE)
    {
        return Err(corrupted("managed Form content root identity is invalid"));
    }
    Ok(())
}

fn parse_mxl_root(bytes: &[u8]) -> Result<NativeMxlRootKind, SourceAdapterError> {
    let xml = utf8(bytes, "MXL content is not valid UTF-8")?;
    let document = Document::parse(xml).map_err(|_| corrupted("MXL content is malformed XML"))?;
    let root = document.root_element();
    match (root.tag_name().name(), root.tag_name().namespace()) {
        ("SpreadsheetDocument", Some(SPREADSHEET_DOCUMENT_NAMESPACE)) => {
            Ok(NativeMxlRootKind::SpreadsheetDocument)
        }
        ("document", Some(LEGACY_SPREADSHEET_NAMESPACE)) => Ok(NativeMxlRootKind::LegacyDocument),
        _ => Err(corrupted("MXL content root identity is invalid")),
    }
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

fn native_class(profile: &'static MetadataClassProfile) -> NativeMetadataClass {
    NativeMetadataClass {
        canonical_name: profile.class_name,
        role: profile.role,
    }
}

fn registration_state(
    registration: &NativeRegistrationEvidence,
    resolved: bool,
) -> NativeNodeState {
    if resolved {
        NativeNodeState::ResolvedRegistration {
            registration: registration.clone(),
        }
    } else {
        NativeNodeState::UnresolvedRegistration {
            registration: registration.clone(),
        }
    }
}

fn reconcile_registered_uuid(
    registration_uuid: Option<Uuid>,
    descriptor_uuid: Option<Uuid>,
) -> Result<Option<Uuid>, SourceAdapterError> {
    match (registration_uuid, descriptor_uuid) {
        (Some(registration), Some(descriptor)) if registration != descriptor => Err(error(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            "registered node UUID differs between registration and descriptor",
        )),
        (Some(uuid), _) | (_, Some(uuid)) => Ok(Some(uuid)),
        (None, None) => Ok(None),
    }
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

struct DecodedNode {
    node: NativeMetadataNode,
    complete: bool,
}

#[derive(Default)]
struct DecodeContext {
    uuids: BTreeSet<Uuid>,
}

impl DecodeContext {
    fn register_uuid(&mut self, uuid: Option<Uuid>) -> Result<(), SourceAdapterError> {
        if uuid.is_some_and(|uuid| !self.uuids.insert(uuid)) {
            return Err(error(
                SourceAdapterErrorKind::IdentityCollision,
                "Platform XML snapshot contains duplicate native UUID identity",
            ));
        }
        Ok(())
    }
}

struct DecodedChildren {
    nodes: Vec<NativeMetadataChild>,
    complete: bool,
}

fn relation_role_for_child_collection(
    _owner: &'static MetadataClassProfile,
    child: &'static MetadataClassProfile,
) -> RelationRole {
    match child.role {
        MetadataClassRole::Attribute => RelationRole::Attributes,
        MetadataClassRole::TabularSection => RelationRole::TabularSections,
        MetadataClassRole::Form => RelationRole::Forms,
        MetadataClassRole::Template => RelationRole::Templates,
        MetadataClassRole::Command => RelationRole::Commands,
        _ => RelationRole::Children,
    }
}

struct ParsedDescriptor {
    uuid: Option<Uuid>,
    template_type: NativePropertyValue,
}

fn corrupted(message: impl Into<String>) -> SourceAdapterError {
    error(SourceAdapterErrorKind::DecodeCorrupted, message)
}

fn error(kind: SourceAdapterErrorKind, message: impl Into<String>) -> SourceAdapterError {
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

    use crate::domain::{
        navigation::CoverageState,
        source_adapters::{
            FormatVersion, SnapshotEvidence, SourceAdapterErrorKind, SourceDescriptor,
            SourceFamily, SourceId, SourceRevision,
        },
    };

    use super::{
        decode, NativeEvidenceState, NativeMxlRootKind, NativeNodeBacking, NativeNodeState,
        NativePropertyProvenance, NativePropertyValue, PlatformXmlProvider,
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
        let mut fixture = fixture("Shipment.xml", &[("Shipment.xml", "not XML".to_string())]);
        fixture
            .descriptor
            .snapshot_evidence
            .as_mut()
            .unwrap()
            .revision = SourceRevision::new("sha256:stale").unwrap();

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn inconsistent_root_digest_is_rejected_before_decode() {
        let mut fixture = fixture("Shipment.xml", &[("Shipment.xml", "not XML".to_string())]);
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
        let number = decoded
            .root
            .children
            .iter()
            .find(|child| child.name == "Number")
            .unwrap();
        assert_eq!(
            number.uuid.unwrap().to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
        assert!(decoded
            .root
            .children
            .iter()
            .any(|child| child.name == "Post"));
        assert_eq!(
            decoded.root.properties["Comment"].provenance,
            NativePropertyProvenance::Absent
        );
        assert_eq!(
            decoded.root.properties["Synonym"].value,
            NativePropertyValue::Structured
        );
    }

    #[test]
    fn root_filename_must_correspond_to_native_name() {
        let fixture = fixture("Other.xml", &[("Other.xml", metadata_document(""))]);

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
    }

    #[test]
    fn recursive_tabular_section_children_are_preserved() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <TabularSection>
                <Properties><Name>Lines</Name></Properties>
                <ChildObjects>
                  <Attribute><Properties><Name>Sku</Name></Properties></Attribute>
                </ChildObjects>
              </TabularSection>
            </ChildObjects>
            "#,
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();
        let lines = &decoded.root.children[0];

        assert_eq!(lines.class.canonical_name, "TabularSection");
        assert_eq!(lines.name, "Lines");
        assert_eq!(lines.children[0].class.canonical_name, "Attribute");
        assert_eq!(lines.children[0].name, "Sku");
    }

    #[test]
    fn invalid_root_namespace_and_duplicate_classes_are_typed_failures() {
        let invalid_namespace = fixture(
            "Shipment.xml",
            &[(
                "Shipment.xml",
                "<MetaDataObject version=\"2.20\"><Document><Properties><Name>Shipment</Name></Properties></Document></MetaDataObject>".to_string(),
            )],
        );
        assert_eq!(
            decode(&invalid_namespace.provider, &invalid_namespace.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::DecodeCorrupted
        );

        let duplicate_class = fixture(
            "Shipment.xml",
            &[(
                "Shipment.xml",
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Document><Properties><Name>Shipment</Name></Properties></Document><Document><Properties><Name>Shipment</Name></Properties></Document></MetaDataObject>".to_string(),
            )],
        );
        assert_eq!(
            decode(&duplicate_class.provider, &duplicate_class.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::ProjectionAmbiguous
        );
    }

    #[test]
    fn form_registration_descriptor_and_managed_content_are_validated() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[
                ("Shipment/Forms/ItemForm.xml", form_descriptor("ItemForm")),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();
        let NativeNodeBacking::Form(form) = &decoded.root.children[0].backing else {
            panic!("Form registration must retain Form backing evidence");
        };

        assert_eq!(form.registration.name, "ItemForm");
        assert_eq!(form.descriptor.state, NativeEvidenceState::Validated);
        assert_eq!(form.managed_content.state, NativeEvidenceState::Validated);
    }

    #[test]
    fn malformed_registered_descriptor_and_managed_form_are_typed_failures() {
        let malformed_descriptor = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[("Shipment/Forms/ItemForm.xml", "<MetaDataObject".to_string())],
        );
        assert_eq!(
            decode(
                &malformed_descriptor.provider,
                &malformed_descriptor.descriptor,
            )
            .unwrap_err()
            .kind,
            SourceAdapterErrorKind::DecodeCorrupted
        );

        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[
                ("Shipment/Forms/ItemForm.xml", form_descriptor("ItemForm")),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    "<Form xmlns=\"http://v8.1c.ru/8.3/MDClasses\"/>".to_string(),
                ),
            ],
        );

        assert_eq!(
            decode(&fixture.provider, &fixture.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::DecodeCorrupted
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
        for (file_name, content, expected_root) in [
            (
                "Template.xml",
                spreadsheet_document_source(),
                Some(NativeMxlRootKind::SpreadsheetDocument),
            ),
            (
                "Template.xml",
                "<document xmlns=\"http://v8.1c.ru/8.2/data/spreadsheet\"/>",
                Some(NativeMxlRootKind::LegacyDocument),
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
            let NativeNodeBacking::Template(template) = &decoded.root.children[0].backing else {
                panic!("Template registration must retain Template backing evidence");
            };

            assert_eq!(template.descriptor.state, NativeEvidenceState::Validated);
            assert_eq!(
                template.descriptor_type,
                NativePropertyValue::Scalar("SpreadsheetDocument".to_string())
            );
            assert_eq!(
                template.canonical_content.state,
                NativeEvidenceState::Validated
            );
            assert_eq!(template.mxl_root_kind, expected_root);
        }

        for (file_name, content) in [
            ("Template.mxl", spreadsheet_document_source()),
            ("Template.xml", "<SpreadsheetDocument/>"),
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

            assert_eq!(
                decode(&fixture.provider, &fixture.descriptor)
                    .unwrap_err()
                    .kind,
                SourceAdapterErrorKind::DecodeCorrupted
            );
        }
    }

    #[test]
    fn malformed_mxl_content_is_decode_corrupted() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Template>Print</Template></ChildObjects>",
            &[
                (
                    "Shipment/Templates/Print.xml",
                    template_descriptor("Print", "SpreadsheetDocument"),
                ),
                (
                    "Shipment/Templates/Print/Ext/Template.xml",
                    "<SpreadsheetDocument".to_string(),
                ),
            ],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn invalid_registered_identity_is_rejected_before_snapshot_lookup() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>../../outside</Form></ChildObjects>",
            &[("outside/Ext/Form.xml", managed_form_source().to_string())],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn fully_materialized_recursive_tree_has_complete_coverage() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <TabularSection uuid="22222222-2222-2222-2222-222222222222">
                <Properties><Name>Lines</Name></Properties>
                <ChildObjects>
                  <Attribute uuid="33333333-3333-3333-3333-333333333333">
                    <Properties><Name>Sku</Name></Properties>
                  </Attribute>
                </ChildObjects>
              </TabularSection>
            </ChildObjects>
            "#,
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(decoded.coverage, CoverageState::Complete);
        assert!(matches!(
            decoded.root.state,
            NativeNodeState::ResolvedInline
        ));
        assert!(matches!(
            decoded.root.children[0].children[0].state,
            NativeNodeState::ResolvedInline
        ));
    }

    #[test]
    fn valid_configuration_registration_is_unresolved_and_partial() {
        let xml = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
          <Configuration uuid="11111111-1111-1111-1111-111111111111">
            <Properties><Name>Application</Name></Properties>
            <ChildObjects>
              <Document uuid="22222222-2222-2222-2222-222222222222">Shipment</Document>
            </ChildObjects>
          </Configuration>
        </MetaDataObject>"#;
        let fixture = fixture(
            "Configuration.xml",
            &[("Configuration.xml", xml.to_string())],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(decoded.coverage, CoverageState::Partial);
        assert!(matches!(
            decoded.root.children[0].state,
            NativeNodeState::UnresolvedRegistration { .. }
        ));
    }

    #[test]
    fn inline_child_without_properties_is_corrupted() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <TabularSection uuid="22222222-2222-2222-2222-222222222222">
                <ChildObjects>
                  <Attribute uuid="33333333-3333-3333-3333-333333333333">
                    <Properties><Name>Sku</Name></Properties>
                  </Attribute>
                </ChildObjects>
              </TabularSection>
            </ChildObjects>
            "#,
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn duplicate_uuid_is_rejected_source_wide() {
        let fixture = document_fixture(
            r#"
            <ChildObjects>
              <TabularSection uuid="22222222-2222-2222-2222-222222222222">
                <Properties><Name>First</Name></Properties>
                <ChildObjects>
                  <Attribute uuid="44444444-4444-4444-4444-444444444444">
                    <Properties><Name>Code</Name></Properties>
                  </Attribute>
                </ChildObjects>
              </TabularSection>
              <TabularSection uuid="33333333-3333-3333-3333-333333333333">
                <Properties><Name>Second</Name></Properties>
                <ChildObjects>
                  <Attribute uuid="44444444-4444-4444-4444-444444444444">
                    <Properties><Name>OtherCode</Name></Properties>
                  </Attribute>
                </ChildObjects>
              </TabularSection>
            </ChildObjects>
            "#,
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
    }

    #[test]
    fn descriptor_only_uuid_is_promoted_to_registered_node_identity() {
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor_with_uuid("ItemForm", "22222222-2222-2222-2222-222222222222"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(
            decoded.root.children[0].uuid.unwrap().to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
    }

    #[test]
    fn descriptor_only_uuid_collides_with_any_other_native_node() {
        let fixture = document_fixture_with_files(
            r#"<ChildObjects>
              <Attribute uuid="22222222-2222-2222-2222-222222222222">
                <Properties><Name>Number</Name></Properties>
              </Attribute>
              <Form>ItemForm</Form>
            </ChildObjects>"#,
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor_with_uuid("ItemForm", "22222222-2222-2222-2222-222222222222"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::IdentityCollision);
    }

    #[test]
    fn registration_and_descriptor_uuid_mismatch_is_projection_ambiguous() {
        let fixture = document_fixture_with_files(
            r#"<ChildObjects>
              <Form uuid="22222222-2222-2222-2222-222222222222">ItemForm</Form>
            </ChildObjects>"#,
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor_with_uuid("ItemForm", "33333333-3333-3333-3333-333333333333"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn matching_registration_and_descriptor_uuid_is_indexed_once() {
        let fixture = document_fixture_with_files(
            r#"<ChildObjects>
              <Form uuid="22222222-2222-2222-2222-222222222222">ItemForm</Form>
            </ChildObjects>"#,
            &[
                (
                    "Shipment/Forms/ItemForm.xml",
                    form_descriptor_with_uuid("ItemForm", "22222222-2222-2222-2222-222222222222"),
                ),
                (
                    "Shipment/Forms/ItemForm/Ext/Form.xml",
                    managed_form_source().to_string(),
                ),
            ],
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(
            decoded.root.children[0].uuid.unwrap().to_string(),
            "22222222-2222-2222-2222-222222222222"
        );
    }

    #[test]
    fn scalar_xsi_type_annotation_is_preserved_without_raw_attributes() {
        let fixture = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xmlSchema="http://www.w3.org/2001/XMLSchema" xsi:type="xmlSchema:decimal">0</FillValue></Properties>"#,
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();
        assert_eq!(
            decoded.root.properties["FillValue"].value,
            NativePropertyValue::AnnotatedScalar {
                value: "0".to_string(),
                type_annotation: crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarType::Decimal,
            },
        );
    }

    #[test]
    fn scalar_annotation_failures_are_local_and_do_not_stop_siblings() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarAnnotationIssue;

        let fixture = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="unknown:decimal">0</FillValue><Description>Still projected</Description></Properties><ChildObjects><Attribute><Properties><Name>Number</Name></Properties></Attribute></ChildObjects>"#,
        );

        let decoded = decode(&fixture.provider, &fixture.descriptor).unwrap();

        assert_eq!(
            decoded.root.properties["FillValue"].value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::Unknown,
            },
        );
        assert_eq!(
            decoded.root.properties["Description"].value,
            NativePropertyValue::Scalar("Still projected".to_string()),
        );
        assert_eq!(decoded.root.children[0].name, "Number");
    }

    #[test]
    fn scalar_annotation_rejects_alien_or_conflicting_qnames_locally() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarAnnotationIssue;

        let alien = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="urn:alien" xsi:type="xs:decimal">0</FillValue></Properties>"#,
        );
        let conflicting = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xs="http://www.w3.org/2001/XMLSchema" xsi:type="xs:decimal" type="xs:string">0</FillValue></Properties>"#,
        );
        let missing = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue>0</FillValue></Properties>"#,
        );
        let unqualified = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue type="xs:decimal">0</FillValue></Properties>"#,
        );

        assert_eq!(
            decode(&alien.provider, &alien.descriptor)
                .unwrap()
                .root
                .properties["FillValue"]
                .value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::Unknown,
            },
        );
        assert_eq!(
            decode(&conflicting.provider, &conflicting.descriptor)
                .unwrap()
                .root
                .properties["FillValue"]
                .value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::Conflicting,
            },
        );
        assert_eq!(
            decode(&missing.provider, &missing.descriptor)
                .unwrap()
                .root
                .properties["FillValue"]
                .value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::Missing,
            },
        );
        assert_eq!(
            decode(&unqualified.provider, &unqualified.descriptor)
                .unwrap()
                .root
                .properties["FillValue"]
                .value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::Unqualified,
            },
        );
    }

    #[test]
    fn scalar_annotations_classify_empty_values_before_absence() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::{
            NativeScalarAnnotationIssue, NativeScalarType,
        };

        let string = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="http://www.w3.org/2001/XMLSchema" xsi:type="x:string"></FillValue><Description>Sibling</Description></Properties>"#,
        );
        let decimal = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="http://www.w3.org/2001/XMLSchema" xsi:type="x:decimal"></FillValue><Description>Sibling</Description></Properties>"#,
        );
        let alien = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="urn:alien" xsi:type="x:decimal"></FillValue></Properties>"#,
        );
        let unbound = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="x:decimal"></FillValue></Properties>"#,
        );
        let conflicting = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="http://www.w3.org/2001/XMLSchema" xsi:type="x:string" type="x:decimal"></FillValue></Properties>"#,
        );
        let unsupported = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:x="http://www.w3.org/2001/XMLSchema" xsi:type="x:float"></FillValue></Properties>"#,
        );
        let absent = document_fixture(
            r#"<Properties><Name>Shipment</Name><FillValue></FillValue></Properties>"#,
        );

        let decoded = decode(&string.provider, &string.descriptor).unwrap();
        assert_eq!(
            decoded.root.properties["FillValue"].value,
            NativePropertyValue::AnnotatedScalar {
                value: String::new(),
                type_annotation: NativeScalarType::String,
            },
        );
        assert_eq!(
            decoded.root.properties["Description"].value,
            NativePropertyValue::Scalar("Sibling".to_string())
        );
        let decoded = decode(&decimal.provider, &decimal.descriptor).unwrap();
        assert_eq!(
            decoded.root.properties["FillValue"].value,
            NativePropertyValue::UnresolvedScalar {
                issue: NativeScalarAnnotationIssue::InvalidLexical
            },
        );
        assert_eq!(
            decoded.root.properties["Description"].value,
            NativePropertyValue::Scalar("Sibling".to_string())
        );
        for fixture in [&alien, &unbound, &conflicting, &unsupported] {
            assert!(matches!(
                decode(&fixture.provider, &fixture.descriptor)
                    .unwrap()
                    .root
                    .properties["FillValue"]
                    .value,
                NativePropertyValue::UnresolvedScalar { .. },
            ));
        }
        assert_eq!(
            decode(&absent.provider, &absent.descriptor)
                .unwrap()
                .root
                .properties["FillValue"]
                .value,
            NativePropertyValue::Absent,
        );
    }

    fn document_fixture(body: &str) -> Fixture {
        fixture("Shipment.xml", &[("Shipment.xml", metadata_document(body))])
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

    fn form_descriptor_with_uuid(name: &str, uuid: &str) -> String {
        format!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\"><Form uuid=\"{uuid}\"><Properties><Name>{name}</Name></Properties></Form></MetaDataObject>"
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
