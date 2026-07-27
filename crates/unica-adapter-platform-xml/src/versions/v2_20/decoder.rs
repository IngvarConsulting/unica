use std::collections::{BTreeMap, BTreeSet};

use roxmltree::{Document, Node};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    domain::{
        identifiers::is_1c_identifier,
        navigation::{CoverageState, RelationRole, SemanticObjectKind},
        navigation_limits::{
            MAX_NAVIGATION_IDENTITY_ITEMS, MAX_NAVIGATION_NESTING_DEPTH, MAX_NAVIGATION_NODES,
            MAX_NAVIGATION_PROPERTIES_PER_NODE, MAX_NAVIGATION_RELATIONS,
        },
        source_adapters::{
            SnapshotConsistency, SourceAdapterError, SourceAdapterErrorKind, SourceDescriptor,
            SourceFamily, SourceSnapshot,
        },
    },
    versions::v2_20::{ProbeOutcome, SourceInput},
};

use super::{
    native_model::{
        NativeContentEvidence, NativeDescriptorEvidence, NativeEvidenceState, NativeForm,
        NativeIdentityDiscriminator, NativeMetadataChild, NativeMetadataClass, NativeMetadataNode,
        NativeMxlRootKind, NativeNodeBacking, NativeNodeState, NativeProperty,
        NativePropertyProvenance, NativePropertyValue, NativeReferenceRelation,
        NativeRegistrationEvidence, NativeScalarAnnotationIssue, NativeScalarType,
        NativeSemanticReference, NativeTemplate, PlatformXmlNativeSnapshot,
    },
    probe::PlatformXmlProbe,
    provider::PlatformXmlProvider,
    schema::{
        child_metadata_class_profile, metadata_class_profile, parse_type_description_2_20,
        ChildObjectsVocabulary, MetadataClassProfile, MetadataClassRole,
    },
    semantic_map::{self, MappingSource, NativeValueKind},
};

const METADATA_NAMESPACE: &str = "http://v8.1c.ru/8.3/MDClasses";
const MANAGED_FORM_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/logform";
const SPREADSHEET_DOCUMENT_NAMESPACE: &str = "http://v8.1c.ru/spreadsheet/document";
const LEGACY_SPREADSHEET_NAMESPACE: &str = "http://v8.1c.ru/8.2/data/spreadsheet";
const DATA_CORE_NAMESPACE: &str = "http://v8.1c.ru/8.1/data/core";
const READABLE_NAMESPACE: &str = "http://v8.1c.ru/8.3/xcf/readable";
const XML_SCHEMA_INSTANCE_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema-instance";
const XML_SCHEMA_NAMESPACE: &str = "http://www.w3.org/2001/XMLSchema";
const RIGHTS_NAMESPACE: &str = "http://v8.1c.ru/8.2/roles";

pub(crate) fn decode_path(
    input: &SourceInput,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError> {
    let provider = PlatformXmlProvider::capture(&input.target, &input.source_root)?;
    let source_id = crate::domain::source_adapters::source_id_for_configured_source_set(
        input.configured_source_set.as_deref().ok_or_else(|| {
            error(
                SourceAdapterErrorKind::SourceUnavailable,
                "Platform XML decode requires a configured source-set identity",
            )
        })?,
    )?;
    let binding = crate::domain::source_adapters::SourceBinding::new(
        source_id,
        input.declared_family.clone(),
        input.declared_format.clone(),
        provider.target_identity().clone(),
        provider.revision()?,
    );
    let descriptor = match PlatformXmlProbe::new().probe_provider(
        &provider,
        provider.descriptor_key(),
        &binding,
    )? {
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
    let mut context = DecodeContext::default();
    decode_with_context(provider, descriptor, &mut context)
}

fn decode_with_context(
    provider: &PlatformXmlProvider,
    descriptor: &SourceDescriptor,
    context: &mut DecodeContext,
) -> Result<PlatformXmlNativeSnapshot, SourceAdapterError> {
    provider.prepare_navigation_snapshot()?;
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

    let (xml, document) = parse_bounded_xml_document(
        &root_bytes,
        "Platform XML root descriptor is not valid UTF-8",
        "Platform XML root descriptor is malformed",
    )?;
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
    let mut decoded = decode_scoped(context, |context| {
        decode_inline_node(provider, class, profile, base_key, xml, context)
    })?;
    let mut consumed = BTreeSet::from([root_key.to_string()]);
    collect_backing_keys(&decoded.node, &mut consumed);
    let backing_prefix = format!("{base_key}/");
    let unknown_backing_count = provider
        .snapshot_files()
        .filter(|(key, _)| key.starts_with(&backing_prefix) && !consumed.contains(*key))
        .count();
    if unknown_backing_count > 0 {
        decoded.node.properties.insert(
            "@unknownBacking".to_string(),
            native_property(
                "@unknownBacking",
                NativePropertyValue::StringList(
                    (0..unknown_backing_count)
                        .map(|_| "backing".to_string())
                        .collect(),
                ),
            ),
        );
        decoded.complete = false;
    }

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
    let properties = decode_properties_for_profile(properties_node, profile, context)?;
    let uuid = parse_optional_uuid(node)?;
    context.register_uuid(uuid)?;
    let target_discriminator = native_target_discriminator(node.tag_name().name(), &name);
    let mut children = decode_children(provider, node, profile, base_key, source_xml, context)?;
    let mut complete = properties.complete
        && children.complete
        && profile.source == MappingSource::Native
        && !matches!(
            profile.role,
            MetadataClassRole::Unknown | MetadataClassRole::Unsupported
        );
    let mut native_node = NativeMetadataNode {
        class: native_class(profile),
        uuid,
        name,
        target_discriminator,
        occurrence: None,
        state: NativeNodeState::ResolvedInline,
        properties: properties.properties,
        references: properties.references,
        children: Vec::new(),
        unmapped_facts: properties.unmapped_facts
            + children.unmapped_facts
            + usize::from(profile.source != MappingSource::Native),
        backing: NativeNodeBacking::None,
    };
    apply_inline_backing(
        provider,
        profile,
        base_key,
        &mut native_node,
        &mut children.nodes,
        &mut complete,
        context,
    )?;
    native_node.children = children.nodes;
    Ok(DecodedNode {
        node: native_node,
        complete,
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
            unmapped_facts: 0,
        });
    };
    let mut occurrences = BTreeMap::<(String, String, String), u32>::new();
    let mut nodes = Vec::new();
    let mut complete = true;
    let mut unmapped_facts = 0usize;
    for (position, child) in child_objects
        .children()
        .filter(Node::is_element)
        .enumerate()
    {
        context.register_relation()?;
        context.register_identity_item()?;
        if child.tag_name().namespace() != Some(METADATA_NAMESPACE) {
            return Err(corrupted("Platform XML child class namespace is invalid"));
        }
        let profile = child_metadata_class_profile(owner_profile, child.tag_name().name())
            .ok_or_else(|| {
                corrupted("Platform XML child class is not allowed by the schema registry")
            })?;
        if owner_profile.child_objects == ChildObjectsVocabulary::None
            && profile.role != MetadataClassRole::Unknown
        {
            return Err(corrupted(
                "Platform XML class contains a registered child forbidden by the schema registry",
            ));
        }
        let Some(role) = semantic_map::child_relation_role(owner_profile, profile) else {
            complete = false;
            unmapped_facts += 1;
            continue;
        };
        let mut decoded = decode_scoped(context, |context| {
            if matches!(
                profile.role,
                MetadataClassRole::Form | MetadataClassRole::Template
            ) {
                decode_backed_registration(provider, child, profile, base_key, source_xml, context)
            } else if !has_direct_child(child, "Properties") {
                if profile.role == MetadataClassRole::Unknown {
                    return decode_unresolved_registration(child, profile, context);
                }
                if owner_profile.child_objects != ChildObjectsVocabulary::ConfigurationTopLevel
                    || profile.role != MetadataClassRole::TopLevelObject
                {
                    return Err(corrupted(
                        "inline Platform XML child is missing required Properties",
                    ));
                }
                decode_unresolved_registration(child, profile, context)
            } else {
                decode_inline_node(provider, child, profile, base_key, source_xml, context)
            }
        })?;
        if profile.role == MetadataClassRole::Unknown {
            let occurrence = u32::try_from(position + 1).map_err(|_| {
                error(
                    SourceAdapterErrorKind::ResourceLimit,
                    "too many Platform XML child occurrences",
                )
            })?;
            decoded.node.occurrence = Some(occurrence);
            decoded.node.properties.insert(
                "@unknownOccurrence".to_string(),
                native_property(
                    "@unknownOccurrence",
                    NativePropertyValue::Scalar(occurrence.to_string()),
                ),
            );
            decoded.node.unmapped_facts += 1;
        }
        let occurrence_key = (
            role.as_str().to_string(),
            child.tag_name().name().to_string(),
            decoded.node.name.clone(),
        );
        let occurrence = occurrences.entry(occurrence_key).or_default();
        *occurrence = occurrence.checked_add(1).ok_or_else(|| {
            error(
                SourceAdapterErrorKind::ResourceLimit,
                "too many same-identity Platform XML child occurrences",
            )
        })?;
        let identity_discriminator = child_identity_discriminator(
            base_key,
            role,
            child.tag_name().name(),
            &decoded.node,
            *occurrence,
        );
        complete &=
            decoded.complete && !semantic_map::child_mapping_is_partial(owner_profile, profile);
        nodes.push(NativeMetadataChild {
            role,
            identity_discriminator,
            node: decoded.node,
        });
    }
    Ok(DecodedChildren {
        nodes,
        complete,
        unmapped_facts,
    })
}

fn decode_unresolved_registration(
    node: Node<'_, '_>,
    profile: &'static MetadataClassProfile,
    context: &mut DecodeContext,
) -> Result<DecodedNode, SourceAdapterError> {
    let registration = registration(node)?;
    context.register_uuid(registration.uuid)?;
    let target_discriminator =
        native_target_discriminator(node.tag_name().name(), &registration.name);
    Ok(DecodedNode {
        node: NativeMetadataNode {
            class: native_class(profile),
            uuid: registration.uuid,
            name: registration.name.clone(),
            target_discriminator,
            occurrence: None,
            state: NativeNodeState::UnresolvedRegistration {
                registration: registration.clone(),
            },
            properties: synthetic_name_property(&registration.name),
            references: Vec::new(),
            children: Vec::new(),
            unmapped_facts: 0,
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
    _source_xml: &str,
    context: &mut DecodeContext,
) -> Result<DecodedNode, SourceAdapterError> {
    let registration = registration(node)?;
    let registration_properties = match optional_unique_child(node, "Properties")? {
        Some(properties) => decode_properties_for_profile(properties, profile, context)?,
        None => DecodedProperties::synthetic_name(&registration.name),
    };
    let backing = semantic_map::backing_mapping(profile.kind).ok_or_else(|| {
        corrupted("schema-backed registration has no coverage-registry backing mapping")
    })?;
    match backing.kind {
        semantic_map::BackingKind::Form => {
            if profile.role != MetadataClassRole::Form {
                return Err(corrupted(
                    "form backing mapping disagrees with the metadata role",
                ));
            }
            let descriptor_key = format!("{base_key}/Forms/{}.xml", registration.name);
            let content_key = format!("{base_key}/Forms/{}/Ext/Form.xml", registration.name);
            let (descriptor, descriptor_properties) = match snapshot_file(provider, &descriptor_key)
            {
                Some(bytes) => {
                    let parsed =
                        parse_registered_descriptor(&bytes, profile, &registration.name, context)?;
                    (
                        descriptor_evidence(
                            NativeEvidenceState::Validated,
                            descriptor_key,
                            parsed.uuid,
                        ),
                        Some(parsed.properties),
                    )
                }
                None => (
                    descriptor_evidence(NativeEvidenceState::Absent, descriptor_key, None),
                    None,
                ),
            };
            let properties = descriptor_properties.unwrap_or(registration_properties);
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
            let complete = false;
            let effective_uuid = reconcile_registered_uuid(registration.uuid, descriptor.uuid)?;
            context.register_uuid(effective_uuid)?;
            let state = registration_state(&registration, complete);
            let target_discriminator =
                native_target_discriminator(node.tag_name().name(), &registration.name);
            Ok(DecodedNode {
                node: NativeMetadataNode {
                    class: native_class(profile),
                    uuid: effective_uuid,
                    name: registration.name.clone(),
                    target_discriminator,
                    occurrence: None,
                    state,
                    properties: properties.properties,
                    references: properties.references,
                    children: Vec::new(),
                    unmapped_facts: properties.unmapped_facts,
                    backing: NativeNodeBacking::Form(NativeForm {
                        registration,
                        descriptor,
                        managed_content,
                    }),
                },
                complete,
            })
        }
        semantic_map::BackingKind::Template => {
            if profile.role != MetadataClassRole::Template {
                return Err(corrupted(
                    "template backing mapping disagrees with the metadata role",
                ));
            }
            let descriptor_key = format!("{base_key}/Templates/{}.xml", registration.name);
            let (descriptor, descriptor_properties) = match snapshot_file(provider, &descriptor_key)
            {
                Some(bytes) => {
                    let parsed =
                        parse_registered_descriptor(&bytes, profile, &registration.name, context)?;
                    (
                        descriptor_evidence(
                            NativeEvidenceState::Validated,
                            descriptor_key,
                            parsed.uuid,
                        ),
                        Some(parsed.properties),
                    )
                }
                None => (
                    descriptor_evidence(NativeEvidenceState::Absent, descriptor_key, None),
                    None,
                ),
            };
            let properties = descriptor_properties.unwrap_or(registration_properties);
            let descriptor_type = properties
                .properties
                .get("TemplateType")
                .map(|property| property.value.clone())
                .unwrap_or(NativePropertyValue::Absent);
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
            let complete = false;
            let effective_uuid = reconcile_registered_uuid(registration.uuid, descriptor.uuid)?;
            context.register_uuid(effective_uuid)?;
            let state = registration_state(&registration, complete);
            let target_discriminator =
                native_target_discriminator(node.tag_name().name(), &registration.name);
            Ok(DecodedNode {
                node: NativeMetadataNode {
                    class: native_class(profile),
                    uuid: effective_uuid,
                    name: registration.name.clone(),
                    target_discriminator,
                    occurrence: None,
                    state,
                    properties: properties.properties,
                    references: properties.references,
                    children: Vec::new(),
                    unmapped_facts: properties.unmapped_facts,
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
        semantic_map::BackingKind::Rights => Err(corrupted(
            "rights backing cannot be decoded as a child registration",
        )),
    }
}

fn parse_registered_descriptor(
    bytes: &[u8],
    expected_profile: &'static MetadataClassProfile,
    expected_name: &str,
    context: &mut DecodeContext,
) -> Result<ParsedDescriptor, SourceAdapterError> {
    let (_, document) = parse_bounded_xml_document(
        bytes,
        "registered descriptor is not valid UTF-8",
        "registered descriptor is malformed XML",
    )?;
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
    let properties = decode_properties_for_profile(properties, expected_profile, context)?;
    Ok(ParsedDescriptor { uuid, properties })
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

fn parse_bounded_xml_document<'a>(
    bytes: &'a [u8],
    invalid_utf8_message: &str,
    malformed_message: &str,
) -> Result<(&'a str, Document<'a>), SourceAdapterError> {
    super::xml::parse_bounded_xml_document(bytes)
        .map_err(|error| map_bounded_xml_error(error, invalid_utf8_message, malformed_message))
}

#[cfg(test)]
fn preflight_xml_nesting(xml: &str, malformed_message: &str) -> Result<(), SourceAdapterError> {
    super::xml::preflight_xml_nesting(xml)
        .map_err(|error| map_bounded_xml_error(error, malformed_message, malformed_message))
}

fn map_bounded_xml_error(
    bounded_error: super::xml::BoundedXmlError,
    invalid_utf8_message: &str,
    malformed_message: &str,
) -> SourceAdapterError {
    match bounded_error {
        super::xml::BoundedXmlError::InvalidUtf8 => corrupted(invalid_utf8_message),
        super::xml::BoundedXmlError::Malformed => corrupted(malformed_message),
        super::xml::BoundedXmlError::ResourceLimit => error(
            SourceAdapterErrorKind::ResourceLimit,
            "Platform XML nesting depth exceeds navigation limit",
        ),
    }
}

fn single_metadata_class<'a, 'input>(
    wrapper: Node<'a, 'input>,
) -> Result<Node<'a, 'input>, SourceAdapterError> {
    let mut classes = wrapper.children().filter(Node::is_element);
    let class = classes
        .next()
        .ok_or_else(|| corrupted("Platform XML descriptor has no metadata class"))?;
    if classes.next().is_some() {
        return Err(error(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            "Platform XML descriptor has multiple metadata classes",
        ));
    }
    Ok(class)
}

fn profile_for_node(
    node: Node<'_, '_>,
) -> Result<&'static MetadataClassProfile, SourceAdapterError> {
    if node.tag_name().namespace() != Some(METADATA_NAMESPACE) {
        return Err(corrupted(
            "Platform XML metadata class namespace is invalid",
        ));
    }
    Ok(metadata_class_profile(node.tag_name().name())
        .unwrap_or_else(semantic_map::unknown_metadata_class_profile))
}

fn decode_properties_for_profile(
    properties: Node<'_, '_>,
    profile: &'static MetadataClassProfile,
    context: &mut DecodeContext,
) -> Result<DecodedProperties, SourceAdapterError> {
    let kind = semantic_map::object_kind(profile);
    let mut decoded = BTreeMap::new();
    let mut references = Vec::new();
    let mut complete = true;
    let mut unmapped_facts = 0usize;
    for property in properties.children().filter(Node::is_element) {
        context.register_property(decoded.len())?;
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
        if let Some(role) = semantic_map::relation_property_role(kind, canonical_id.as_str()) {
            let (targets, relation_unmapped) = decode_reference_targets(property);
            complete &= relation_unmapped == 0;
            unmapped_facts += relation_unmapped;
            if !targets.is_empty() {
                references.push(NativeReferenceRelation { role, targets });
            }
            continue;
        }
        let mapping = semantic_map::property_mapping(kind, canonical_id.as_str());
        if mapping.is_none() {
            complete = false;
        }
        let (value, value_complete) = decode_property_value(property, mapping)?;
        complete &= value_complete;
        if mapping.is_some() && !value_complete {
            unmapped_facts += 1;
        }
        let provenance = if matches!(value, NativePropertyValue::Absent) {
            NativePropertyProvenance::Absent
        } else if matches!(
            value,
            NativePropertyValue::Unresolved | NativePropertyValue::UnresolvedScalar { .. }
        ) {
            NativePropertyProvenance::Unresolved
        } else {
            NativePropertyProvenance::Explicit
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
    Ok(DecodedProperties {
        properties: decoded,
        references,
        complete,
        unmapped_facts,
    })
}

#[cfg(test)]
fn decode_properties(
    properties: Node<'_, '_>,
    _source_xml: &str,
    context: &mut DecodeContext,
) -> Result<BTreeMap<String, NativeProperty>, SourceAdapterError> {
    Ok(decode_properties_for_profile(
        properties,
        metadata_class_profile("Attribute").expect("registered test class"),
        context,
    )?
    .properties)
}

fn decode_property_value(
    property: Node<'_, '_>,
    mapping: Option<&semantic_map::PropertyMapping>,
) -> Result<(NativePropertyValue, bool), SourceAdapterError> {
    let Some(mapping) = mapping else {
        if property.children().any(|child| child.is_element()) {
            let values = readable_text_occurrences(property);
            return Ok((
                if values.is_empty() {
                    NativePropertyValue::Structured
                } else {
                    NativePropertyValue::StringList(values)
                },
                false,
            ));
        }
        let value = property.text().unwrap_or_default().trim();
        return Ok((scalar_property_value(None, property, value), false));
    };
    match mapping.value_kind {
        NativeValueKind::LocalizedString => Ok(decode_localized_string(property)),
        NativeValueKind::TypeSet => match parse_type_description_2_20(property) {
            Ok(value) => {
                let complete = !value.variants().iter().any(|variant| variant.is_unknown());
                Ok((NativePropertyValue::TypeSet(value), complete))
            }
            Err(error) if error.kind == SourceAdapterErrorKind::ResourceLimit => Err(error),
            Err(_) => Ok((NativePropertyValue::Unresolved, false)),
        },
        NativeValueKind::StringList => Ok(decode_string_list(property)),
        value_kind if property.children().any(|child| child.is_element()) => {
            let _ = value_kind;
            Ok((NativePropertyValue::Structured, false))
        }
        value_kind => {
            let value = property.text().unwrap_or_default().trim();
            let value = scalar_property_value(Some(value_kind), property, value);
            let complete = !matches!(
                value,
                NativePropertyValue::Unresolved
                    | NativePropertyValue::UnresolvedScalar { .. }
                    | NativePropertyValue::ReadableUnknownScalar { .. }
                    | NativePropertyValue::Structured
            );
            Ok((value, complete))
        }
    }
}

fn decode_localized_string(property: Node<'_, '_>) -> (NativePropertyValue, bool) {
    let elements = property
        .children()
        .filter(Node::is_element)
        .collect::<Vec<_>>();
    if elements.is_empty() {
        let value = property.text().unwrap_or_default().trim();
        return if value.is_empty() {
            (NativePropertyValue::Absent, true)
        } else {
            (
                NativePropertyValue::LocalizedString(BTreeMap::from([(
                    "und".to_string(),
                    value.to_string(),
                )])),
                true,
            )
        };
    }
    let mut values = BTreeMap::new();
    for item in elements {
        if item.tag_name().namespace() != Some(DATA_CORE_NAMESPACE)
            || item.tag_name().name() != "item"
        {
            return (NativePropertyValue::Structured, false);
        }
        let Some(language) = unique_data_core_text(item, "lang") else {
            return (NativePropertyValue::Structured, false);
        };
        let Some(content) = unique_data_core_text(item, "content") else {
            return (NativePropertyValue::Structured, false);
        };
        if language.is_empty()
            || language
                .bytes()
                .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
            || values.insert(language, content).is_some()
        {
            return (NativePropertyValue::Structured, false);
        }
    }
    (NativePropertyValue::LocalizedString(values), true)
}

fn unique_data_core_text(parent: Node<'_, '_>, name: &str) -> Option<String> {
    let mut children = parent.children().filter(|child| {
        child.is_element()
            && child.tag_name().namespace() == Some(DATA_CORE_NAMESPACE)
            && child.tag_name().name() == name
    });
    let child = children.next()?;
    if children.next().is_some() || child.children().any(|nested| nested.is_element()) {
        return None;
    }
    Some(child.text().unwrap_or_default().trim().to_string())
}

fn decode_string_list(property: Node<'_, '_>) -> (NativePropertyValue, bool) {
    let mut values = Vec::new();
    for item in property.children().filter(Node::is_element) {
        if item.tag_name().namespace() != Some(READABLE_NAMESPACE)
            || item.tag_name().name() != "Item"
            || item.children().any(|child| child.is_element())
        {
            return (NativePropertyValue::Structured, false);
        }
        let value = item.text().unwrap_or_default().trim();
        if value.is_empty() {
            return (NativePropertyValue::Structured, false);
        }
        values.push(value.to_string());
    }
    (NativePropertyValue::StringList(values), true)
}

fn decode_reference_targets(property: Node<'_, '_>) -> (Vec<NativeSemanticReference>, usize) {
    let mut targets = Vec::new();
    let mut unmapped = 0usize;
    for item in property.children().filter(Node::is_element) {
        if item.tag_name().namespace() != Some(READABLE_NAMESPACE)
            || item.tag_name().name() != "Item"
            || item.children().any(|child| child.is_element())
        {
            unmapped += 1;
            continue;
        }
        let raw = item.text().unwrap_or_default().trim();
        let Some((native_class, name)) = raw.split_once('.') else {
            unmapped += 1;
            continue;
        };
        if raw.split('.').count() != 2 || !is_1c_identifier(name) {
            unmapped += 1;
            continue;
        }
        let kind = semantic_map::reference_kind(native_class).unwrap_or_else(|| {
            unmapped += 1;
            SemanticObjectKind::Unknown
        });
        targets.push(NativeSemanticReference {
            kind,
            name: name.to_string(),
            uuid: None,
            target_discriminator: native_target_discriminator(native_class, name),
        });
    }
    (targets, unmapped)
}

fn readable_text_occurrences(node: Node<'_, '_>) -> Vec<String> {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|text| text.text().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn scalar_property_value(
    value_kind: Option<NativeValueKind>,
    property: Node<'_, '_>,
    value: &str,
) -> NativePropertyValue {
    if property.attribute((XML_SCHEMA_INSTANCE_NAMESPACE, "nil")) == Some("true") {
        return if value.is_empty() {
            NativePropertyValue::Null
        } else {
            unresolved_scalar(NativeScalarAnnotationIssue::Conflicting)
        };
    }
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
        if matches!(value_kind, Some(NativeValueKind::Polymorphic)) {
            return unresolved_scalar(NativeScalarAnnotationIssue::Missing);
        }
        return NativePropertyValue::Scalar(value.to_string());
    };
    let Some((prefix, local_name)) = annotation.split_once(':') else {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    };
    if prefix.is_empty() || local_name.is_empty() || local_name.contains(':') {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    }
    let Some(namespace) = property.lookup_namespace_uri(Some(prefix)) else {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    };
    if namespace == READABLE_NAMESPACE && local_name == "DesignTimeRef" {
        return design_time_reference_value(value);
    }
    if namespace != XML_SCHEMA_NAMESPACE {
        return unresolved_scalar(NativeScalarAnnotationIssue::Unknown);
    }
    let type_annotation = match local_name {
        "string" => NativeScalarType::String,
        "boolean" => NativeScalarType::Boolean,
        "decimal" => NativeScalarType::Decimal,
        "integer" => NativeScalarType::Integer,
        "stringUuid" => NativeScalarType::Uuid,
        "date" | "dateTime" => NativeScalarType::Date,
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

fn design_time_reference_value(value: &str) -> NativePropertyValue {
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.len() == 3
        && parts[2] == "EmptyRef"
        && is_1c_identifier(parts[1])
        && semantic_map::reference_kind(parts[0]).is_some()
    {
        return NativePropertyValue::EmptyReference;
    }
    let target = parts
        .len()
        .checked_sub(2)
        .and_then(|index| parts.get(index))
        .copied()
        .filter(|candidate| is_1c_identifier(candidate))
        .unwrap_or("readable-reference");
    NativePropertyValue::ReadableUnknownScalar {
        category: super::native_model::NativeUnknownScalarCategory::Reference,
        values: vec![target.to_string()],
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
    let properties = optional_unique_child(node, "Properties")?;
    let direct_text = node
        .children()
        .filter(Node::is_text)
        .filter_map(|child| child.text())
        .collect::<String>();
    let direct_text = direct_text.trim();
    let name = match properties {
        None if node.children().any(|child| child.is_element()) => {
            return Err(corrupted(
                "Platform XML registration has unsupported nested identity",
            ));
        }
        None => direct_text.to_string(),
        Some(properties) if direct_text.is_empty() => required_name(properties)?,
        Some(_) => {
            return Err(error(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "Platform XML registration has conflicting identity fields",
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
    let Some(node) = optional_unique_child(parent, local_name)? else {
        return Ok(None);
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
    let mut children = parent.children().filter(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(METADATA_NAMESPACE)
    });
    let child = children.next();
    if children.next().is_some() {
        return Err(error(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("Platform XML field `{local_name}` is ambiguous"),
        ));
    }
    Ok(child)
}

fn has_direct_child(parent: Node<'_, '_>, local_name: &str) -> bool {
    parent.children().any(|child| {
        child.is_element()
            && child.tag_name().name() == local_name
            && child.tag_name().namespace() == Some(METADATA_NAMESPACE)
    })
}

fn parse_optional_uuid(node: Node<'_, '_>) -> Result<Option<Uuid>, SourceAdapterError> {
    node.attribute("uuid")
        .map(|raw| {
            Uuid::parse_str(raw).map_err(|_| corrupted("Platform XML native UUID is invalid"))
        })
        .transpose()
}

fn validate_managed_form(bytes: &[u8]) -> Result<(), SourceAdapterError> {
    let (_, document) = parse_bounded_xml_document(
        bytes,
        "managed Form content is not valid UTF-8",
        "managed Form content is malformed XML",
    )?;
    let root = document.root_element();
    if root.tag_name().name() != "Form"
        || root.tag_name().namespace() != Some(MANAGED_FORM_NAMESPACE)
    {
        return Err(corrupted("managed Form content root identity is invalid"));
    }
    Ok(())
}

fn parse_mxl_root(bytes: &[u8]) -> Result<NativeMxlRootKind, SourceAdapterError> {
    let (_, document) = parse_bounded_xml_document(
        bytes,
        "MXL content is not valid UTF-8",
        "MXL content is malformed XML",
    )?;
    let root = document.root_element();
    match (root.tag_name().name(), root.tag_name().namespace()) {
        ("SpreadsheetDocument", Some(SPREADSHEET_DOCUMENT_NAMESPACE)) => {
            Ok(NativeMxlRootKind::SpreadsheetDocument)
        }
        ("document", Some(LEGACY_SPREADSHEET_NAMESPACE)) => Ok(NativeMxlRootKind::LegacyDocument),
        _ => Err(corrupted("MXL content root identity is invalid")),
    }
}

fn apply_inline_backing(
    provider: &PlatformXmlProvider,
    profile: &'static MetadataClassProfile,
    base_key: &str,
    node: &mut NativeMetadataNode,
    children: &mut Vec<NativeMetadataChild>,
    complete: &mut bool,
    context: &mut DecodeContext,
) -> Result<(), SourceAdapterError> {
    let Some(mapping) = semantic_map::backing_mapping(profile.kind) else {
        return Ok(());
    };
    match (mapping.kind, profile.source) {
        (semantic_map::BackingKind::Rights, _) => {
            let rights_key = format!("{base_key}/Ext/Rights.xml");
            let Some(bytes) = snapshot_file(provider, &rights_key) else {
                node.backing = NativeNodeBacking::Rights(content_evidence(
                    NativeEvidenceState::Absent,
                    rights_key,
                    None,
                ));
                *complete = false;
                return Ok(());
            };
            let decoded = decode_rights(&bytes, profile, context)?;
            node.properties.extend(decoded.properties);
            node.unmapped_facts += decoded.unmapped_facts;
            children.extend(decoded.children);
            node.backing = NativeNodeBacking::Rights(content_evidence(
                NativeEvidenceState::Validated,
                rights_key,
                Some(digest(&bytes)),
            ));
            *complete &= decoded.complete;
        }
        (semantic_map::BackingKind::Form, MappingSource::Native) => {
            let content_key = format!("{base_key}/Ext/Form.xml");
            let content = match snapshot_file(provider, &content_key) {
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
            node.backing = NativeNodeBacking::Form(NativeForm {
                registration: NativeRegistrationEvidence {
                    uuid: node.uuid,
                    name: node.name.clone(),
                },
                descriptor: descriptor_evidence(
                    NativeEvidenceState::Validated,
                    format!("{base_key}.xml"),
                    node.uuid,
                ),
                managed_content: content,
            });
            *complete = false;
        }
        (semantic_map::BackingKind::Template, MappingSource::Native) => {
            let descriptor_type = node
                .properties
                .get("TemplateType")
                .map(|property| property.value.clone())
                .unwrap_or(NativePropertyValue::Absent);
            let prefix = format!("{base_key}/Ext/");
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
            let (content, mxl_root_kind) = match candidates.pop() {
                None => (
                    content_evidence(
                        NativeEvidenceState::Absent,
                        format!("{prefix}Template.xml"),
                        None,
                    ),
                    None,
                ),
                Some((key, bytes)) => {
                    let mxl_root_kind = if matches!(
                        &descriptor_type,
                        NativePropertyValue::Scalar(value) if value == "SpreadsheetDocument"
                    ) {
                        Some(parse_mxl_root(&bytes)?)
                    } else {
                        None
                    };
                    (
                        content_evidence(
                            NativeEvidenceState::Validated,
                            key.to_string(),
                            Some(digest(&bytes)),
                        ),
                        mxl_root_kind,
                    )
                }
            };
            node.backing = NativeNodeBacking::Template(NativeTemplate {
                registration: NativeRegistrationEvidence {
                    uuid: node.uuid,
                    name: node.name.clone(),
                },
                descriptor: descriptor_evidence(
                    NativeEvidenceState::Validated,
                    format!("{base_key}.xml"),
                    node.uuid,
                ),
                descriptor_type,
                canonical_content: content,
                mxl_root_kind,
            });
            *complete = false;
        }
        (
            semantic_map::BackingKind::Form | semantic_map::BackingKind::Template,
            MappingSource::Derived,
        ) => {}
        _ => {
            return Err(corrupted(
                "coverage-registry backing mapping has no decoder dispatch",
            ))
        }
    }
    Ok(())
}

struct DecodedRights {
    properties: BTreeMap<String, NativeProperty>,
    children: Vec<NativeMetadataChild>,
    complete: bool,
    unmapped_facts: usize,
}

fn decode_rights(
    bytes: &[u8],
    role_profile: &'static MetadataClassProfile,
    context: &mut DecodeContext,
) -> Result<DecodedRights, SourceAdapterError> {
    let (_, document) = parse_bounded_xml_document(
        bytes,
        "access backing is not valid UTF-8",
        "access backing is malformed XML",
    )?;
    let root = document.root_element();
    if root.tag_name().name() != "Rights" || root.tag_name().namespace() != Some(RIGHTS_NAMESPACE) {
        return Err(corrupted("access backing root identity is invalid"));
    }
    let mut properties = BTreeMap::new();
    let mut complete = true;
    let mut unmapped_facts = 0usize;
    let mut root_unknown = RightsUnknownEvidence::default();
    audit_rights_attributes(
        root,
        &[
            (None, "version"),
            (None, "setForNewObjects"),
            (None, "setForAttributesByDefault"),
            (None, "independentRightsOfChildObjects"),
            (Some(XML_SCHEMA_INSTANCE_NAMESPACE), "type"),
        ],
        &mut root_unknown,
    );
    match root.attribute("version") {
        Some("2.17" | "2.20") => {}
        Some(value) => root_unknown.record_value(value),
        None => root_unknown.record_marker(),
    }
    match root.attribute((XML_SCHEMA_INSTANCE_NAMESPACE, "type")) {
        Some("Rights") => {}
        Some(value) => root_unknown.record_value(value),
        None => root_unknown.record_marker(),
    }
    audit_rights_children(
        root,
        &[
            "setForNewObjects",
            "setForAttributesByDefault",
            "independentRightsOfChildObjects",
            "object",
            "restrictionTemplate",
        ],
        &mut root_unknown,
    );
    for (native, semantic_key) in [
        ("setForNewObjects", "@rightsNewObjectsDefault"),
        ("setForAttributesByDefault", "@rightsAttributesDefault"),
        (
            "independentRightsOfChildObjects",
            "@rightsChildObjectsIndependent",
        ),
    ] {
        let raw = rights_scalar_checked(root, native, true, &mut root_unknown);
        let value = raw.as_deref().and_then(parse_boolean);
        if value.is_none() {
            match raw {
                Some(value) => root_unknown.record_value(&value),
                None => root_unknown.record_marker(),
            }
            properties.insert(
                semantic_key.to_string(),
                native_property(semantic_key, NativePropertyValue::Unresolved),
            );
            complete = false;
        } else {
            properties.insert(
                semantic_key.to_string(),
                native_property(
                    semantic_key,
                    NativePropertyValue::Scalar(value.unwrap().to_string()),
                ),
            );
        }
    }

    let permission_profile = semantic_map::derived_profile(SemanticObjectKind::AccessPermission);
    let permission_role = semantic_map::child_relation_role(role_profile, permission_profile)
        .expect("coverage registry maps access permissions");
    let template_profile =
        semantic_map::derived_profile(SemanticObjectKind::AccessRestrictionTemplate);
    let template_role = semantic_map::child_relation_role(role_profile, template_profile)
        .expect("coverage registry maps access restriction templates");
    let mut children = Vec::new();
    let mut ordinal = 0usize;
    for (object_ordinal, object) in rights_children(root, "object").enumerate() {
        let mut object_unknown = RightsUnknownEvidence::default();
        audit_rights_attributes(object, &[], &mut object_unknown);
        audit_rights_children(object, &["name", "right"], &mut object_unknown);
        let raw_target = rights_scalar_checked(object, "name", false, &mut object_unknown);
        let (target_kind, target_name, target_discriminator, target_complete) = match raw_target {
            Some(raw_target) => decode_rights_target(&raw_target),
            None => {
                object_unknown.record_marker();
                (
                    SemanticObjectKind::Unknown,
                    format!("unknown-target-{}", object_ordinal + 1),
                    native_target_discriminator(
                        "unknown-rights-target",
                        &format!("unknown-target-{}", object_ordinal + 1),
                    ),
                    false,
                )
            }
        };
        if !target_complete {
            complete = false;
        }
        for right in rights_children(object, "right") {
            ordinal += 1;
            let mut permission_unknown = RightsUnknownEvidence::default();
            audit_rights_attributes(right, &[], &mut permission_unknown);
            audit_rights_children(
                right,
                &["name", "value", "restrictionByCondition"],
                &mut permission_unknown,
            );
            let permission_name =
                rights_scalar_checked(right, "name", false, &mut permission_unknown)
                    .unwrap_or_else(|| {
                        permission_unknown.record_marker();
                        format!("unknown-permission-{ordinal}")
                    });
            let permission_value =
                rights_scalar_checked(right, "value", false, &mut permission_unknown);
            let permission_allowed = permission_value.as_deref().and_then(parse_boolean);
            if permission_allowed.is_none() {
                match permission_value {
                    Some(value) => permission_unknown.record_value(&value),
                    None => permission_unknown.record_marker(),
                }
                complete = false;
            }
            let restrictions = decode_rights_conditions(right, &mut permission_unknown);
            if permission_unknown.facts > 0 {
                complete = false;
            }
            let permission_identity_name = format!("{target_name}:{permission_name}:{ordinal}");
            let mut permission_properties = synthetic_name_property(&permission_identity_name);
            permission_properties.insert(
                "@permissionName".to_string(),
                native_property(
                    "@permissionName",
                    NativePropertyValue::Scalar(permission_name),
                ),
            );
            permission_properties.insert(
                "@permissionAllowed".to_string(),
                native_property(
                    "@permissionAllowed",
                    permission_allowed
                        .map(|value| NativePropertyValue::Scalar(value.to_string()))
                        .unwrap_or(NativePropertyValue::Unresolved),
                ),
            );
            if !restrictions.is_empty() {
                permission_properties.insert(
                    "@restrictionConditions".to_string(),
                    native_property(
                        "@restrictionConditions",
                        NativePropertyValue::StringList(restrictions),
                    ),
                );
            }
            insert_rights_unknown_evidence(
                &mut permission_properties,
                "@unknownRightsPermission",
                &permission_unknown,
            );
            context.register_relation()?;
            context.register_identity_item()?;
            let permission_node = decode_scoped(context, |_context| {
                Ok(NativeMetadataNode {
                    class: native_class(permission_profile),
                    uuid: None,
                    name: format!("{target_name}:{ordinal}"),
                    target_discriminator: private_discriminator(&[
                        "rights-permission",
                        target_discriminator.as_str(),
                        &ordinal.to_string(),
                    ]),
                    occurrence: None,
                    state: NativeNodeState::ResolvedInline,
                    properties: permission_properties,
                    references: vec![NativeReferenceRelation {
                        role: RelationRole::ACCESS_TARGET,
                        targets: vec![NativeSemanticReference {
                            kind: target_kind,
                            name: target_name.clone(),
                            uuid: None,
                            target_discriminator: target_discriminator.clone(),
                        }],
                    }],
                    children: Vec::new(),
                    unmapped_facts: usize::from(!target_complete)
                        + usize::from(permission_allowed.is_none())
                        + permission_unknown.facts,
                    backing: NativeNodeBacking::None,
                })
            })?;
            children.push(NativeMetadataChild {
                role: permission_role,
                identity_discriminator: private_discriminator(&[
                    "rights-permission-child",
                    permission_role.as_str(),
                    target_discriminator.as_str(),
                    &ordinal.to_string(),
                ]),
                node: permission_node,
            });
        }
        if object_unknown.facts > 0 {
            complete = false;
            root_unknown.extend(object_unknown);
        }
    }
    for template in rights_children(root, "restrictionTemplate") {
        let mut template_unknown = RightsUnknownEvidence::default();
        audit_rights_attributes(template, &[], &mut template_unknown);
        audit_rights_children(template, &["name", "condition"], &mut template_unknown);
        let name = rights_scalar_checked(template, "name", false, &mut template_unknown)
            .unwrap_or_else(|| {
                template_unknown.record_marker();
                format!("unknown-template-{}", children.len() + 1)
            });
        let restrictions = decode_direct_rights_conditions(template, &mut template_unknown);
        let mut template_properties = synthetic_name_property(&name);
        if !restrictions.is_empty() {
            template_properties.insert(
                "@restrictionConditions".to_string(),
                native_property(
                    "@restrictionConditions",
                    NativePropertyValue::StringList(restrictions),
                ),
            );
        }
        insert_rights_unknown_evidence(
            &mut template_properties,
            "@unknownRightsTemplate",
            &template_unknown,
        );
        if template_unknown.facts > 0 {
            complete = false;
        }
        context.register_relation()?;
        context.register_identity_item()?;
        let template_node = decode_scoped(context, |_context| {
            Ok(NativeMetadataNode {
                class: native_class(template_profile),
                uuid: None,
                name: name.clone(),
                target_discriminator: private_discriminator(&[
                    "rights-restriction-template",
                    &name,
                ]),
                occurrence: None,
                state: NativeNodeState::ResolvedInline,
                properties: template_properties,
                references: Vec::new(),
                children: Vec::new(),
                unmapped_facts: template_unknown.facts,
                backing: NativeNodeBacking::None,
            })
        })?;
        children.push(NativeMetadataChild {
            role: template_role,
            identity_discriminator: private_discriminator(&[
                "rights-restriction-template-child",
                template_role.as_str(),
                &name,
            ]),
            node: template_node,
        });
    }
    if root_unknown.facts > 0 {
        insert_rights_unknown_evidence(&mut properties, "@unknownRights", &root_unknown);
        complete = false;
        unmapped_facts += root_unknown.facts;
    }
    Ok(DecodedRights {
        properties,
        children,
        complete,
        unmapped_facts,
    })
}

#[derive(Default)]
struct RightsUnknownEvidence {
    values: Vec<String>,
    facts: usize,
}

impl RightsUnknownEvidence {
    fn record_value(&mut self, value: &str) {
        let value = value.trim();
        self.facts += 1;
        self.values.push(if value.is_empty() {
            format!("extension-occurrence-{}", self.facts)
        } else {
            value.to_string()
        });
    }

    fn record_marker(&mut self) {
        self.facts += 1;
        self.values
            .push(format!("extension-occurrence-{}", self.facts));
    }

    fn record_node(&mut self, node: Node<'_, '_>) {
        let values = readable_text_occurrences(node);
        if values.is_empty() {
            self.record_marker();
        } else {
            for value in values {
                self.record_value(&value);
            }
        }
    }

    fn extend(&mut self, other: Self) {
        self.facts += other.facts;
        self.values.extend(other.values);
    }
}

fn audit_rights_attributes(
    node: Node<'_, '_>,
    allowed: &[(Option<&str>, &str)],
    unknown: &mut RightsUnknownEvidence,
) {
    for attribute in node.attributes() {
        if !allowed.iter().any(|(namespace, name)| {
            attribute.namespace() == *namespace && attribute.name() == *name
        }) {
            unknown.record_value(attribute.value());
        }
    }
}

fn audit_rights_children(
    node: Node<'_, '_>,
    allowed: &[&str],
    unknown: &mut RightsUnknownEvidence,
) {
    for child in node.children().filter(Node::is_element) {
        if child.tag_name().namespace() != Some(RIGHTS_NAMESPACE)
            || !allowed.contains(&child.tag_name().name())
        {
            unknown.record_node(child);
        }
    }
}

fn rights_children<'a, 'input>(
    parent: Node<'a, 'input>,
    name: &'static str,
) -> impl Iterator<Item = Node<'a, 'input>> {
    parent.children().filter(move |child| {
        child.is_element()
            && child.tag_name().namespace() == Some(RIGHTS_NAMESPACE)
            && child.tag_name().name() == name
    })
}

fn rights_scalar_checked(
    parent: Node<'_, '_>,
    name: &str,
    allow_attribute: bool,
    unknown: &mut RightsUnknownEvidence,
) -> Option<String> {
    let attribute = allow_attribute.then(|| parent.attribute(name)).flatten();
    let children = parent
        .children()
        .filter(|child| {
            child.is_element()
                && child.tag_name().namespace() == Some(RIGHTS_NAMESPACE)
                && child.tag_name().name() == name
        })
        .collect::<Vec<_>>();
    if usize::from(attribute.is_some()) + children.len() != 1 {
        if let Some(value) = attribute {
            unknown.record_value(value);
        }
        for child in children {
            unknown.record_node(child);
        }
        return None;
    }
    if let Some(value) = attribute {
        return Some(value.trim().to_string()).filter(|value| !value.is_empty());
    }
    let child = children[0];
    audit_rights_attributes(child, &[], unknown);
    if child.children().any(|nested| nested.is_element()) {
        unknown.record_node(child);
        return None;
    }
    child
        .text()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn decode_rights_conditions(
    right: Node<'_, '_>,
    unknown: &mut RightsUnknownEvidence,
) -> Vec<String> {
    let mut conditions = Vec::new();
    for restriction in rights_children(right, "restrictionByCondition") {
        audit_rights_attributes(restriction, &[], unknown);
        audit_rights_children(restriction, &["condition"], unknown);
        let before = conditions.len();
        conditions.extend(decode_direct_rights_conditions(restriction, unknown));
        if conditions.len() == before {
            unknown.record_marker();
        }
    }
    conditions
}

fn decode_direct_rights_conditions(
    parent: Node<'_, '_>,
    unknown: &mut RightsUnknownEvidence,
) -> Vec<String> {
    let mut conditions = Vec::new();
    for condition in rights_children(parent, "condition") {
        audit_rights_attributes(condition, &[], unknown);
        if condition.children().any(|child| child.is_element()) {
            unknown.record_node(condition);
            continue;
        }
        match condition
            .text()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => conditions.push(value.to_string()),
            None => unknown.record_marker(),
        }
    }
    conditions
}

fn insert_rights_unknown_evidence(
    properties: &mut BTreeMap<String, NativeProperty>,
    key: &str,
    unknown: &RightsUnknownEvidence,
) {
    if unknown.facts > 0 {
        properties.insert(
            key.to_string(),
            native_property(key, NativePropertyValue::StringList(unknown.values.clone())),
        );
    }
}

fn parse_boolean(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn decode_rights_target(
    value: &str,
) -> (
    SemanticObjectKind,
    String,
    NativeIdentityDiscriminator,
    bool,
) {
    let Some((native_class, name)) = value.split_once('.') else {
        let name = if is_1c_identifier(value) {
            value.to_string()
        } else {
            "unknown-target".to_string()
        };
        return (
            SemanticObjectKind::Unknown,
            name.clone(),
            native_target_discriminator("unknown-reference", value),
            false,
        );
    };
    if value.split('.').count() != 2 || !is_1c_identifier(name) {
        let readable_name = value
            .split('.')
            .find(|part| is_1c_identifier(part))
            .unwrap_or("unknown-target");
        return (
            SemanticObjectKind::Unknown,
            readable_name.to_string(),
            native_target_discriminator("unknown-reference", value),
            false,
        );
    }
    match semantic_map::reference_kind(native_class) {
        Some(kind) => (
            kind,
            name.to_string(),
            native_target_discriminator(native_class, name),
            true,
        ),
        None => (
            SemanticObjectKind::Unknown,
            name.to_string(),
            native_target_discriminator(native_class, name),
            false,
        ),
    }
}

fn native_property(id: &str, value: NativePropertyValue) -> NativeProperty {
    NativeProperty {
        canonical_id: id.to_string(),
        provenance: if matches!(value, NativePropertyValue::Absent) {
            NativePropertyProvenance::Absent
        } else if matches!(
            value,
            NativePropertyValue::Unresolved | NativePropertyValue::UnresolvedScalar { .. }
        ) {
            NativePropertyProvenance::Unresolved
        } else {
            NativePropertyProvenance::Explicit
        },
        value,
    }
}

fn collect_backing_keys(node: &NativeMetadataNode, keys: &mut BTreeSet<String>) {
    match &node.backing {
        NativeNodeBacking::None => {}
        NativeNodeBacking::Rights(content) => {
            keys.insert(content.relative_key.clone());
        }
        NativeNodeBacking::Form(form) => {
            keys.insert(form.descriptor.relative_key.clone());
            keys.insert(form.managed_content.relative_key.clone());
        }
        NativeNodeBacking::Template(template) => {
            keys.insert(template.descriptor.relative_key.clone());
            keys.insert(template.canonical_content.relative_key.clone());
        }
    }
    for child in &node.children {
        collect_backing_keys(child, keys);
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
        canonical_name: profile.class_name.clone(),
        role: profile.role,
        kind: profile.kind,
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

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn private_discriminator(parts: &[&str]) -> NativeIdentityDiscriminator {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part.as_bytes());
    }
    NativeIdentityDiscriminator::new(format!("{:x}", digest.finalize()))
}

fn native_target_discriminator(
    native_class: &str,
    qualified_name: &str,
) -> NativeIdentityDiscriminator {
    private_discriminator(&["target", native_class, qualified_name])
}

fn child_identity_discriminator(
    owner_key: &str,
    role: RelationRole,
    native_class: &str,
    child: &NativeMetadataNode,
    occurrence: u32,
) -> NativeIdentityDiscriminator {
    if let Some(uuid) = child.uuid {
        return private_discriminator(&["child", "uuid", &uuid.to_string()]);
    }

    let stable_backing = match &child.backing {
        NativeNodeBacking::Form(form) => Some(form.descriptor.relative_key.as_str()),
        NativeNodeBacking::Template(template) => Some(template.descriptor.relative_key.as_str()),
        NativeNodeBacking::None | NativeNodeBacking::Rights(_) => None,
    };
    if let Some(path) = stable_backing {
        return private_discriminator(&["child", "path", path]);
    }

    private_discriminator(&[
        "child-occurrence",
        owner_key,
        role.as_str(),
        native_class,
        &child.name,
        &occurrence.to_string(),
    ])
}

struct DecodedNode {
    node: NativeMetadataNode,
    complete: bool,
}

struct DecodedProperties {
    properties: BTreeMap<String, NativeProperty>,
    references: Vec<NativeReferenceRelation>,
    complete: bool,
    unmapped_facts: usize,
}

impl DecodedProperties {
    fn synthetic_name(name: &str) -> Self {
        Self {
            properties: synthetic_name_property(name),
            references: Vec::new(),
            complete: true,
            unmapped_facts: 0,
        }
    }
}

#[derive(Default)]
struct DecodeContext {
    uuids: BTreeSet<Uuid>,
    native_nodes: usize,
    relations: usize,
    properties: usize,
    identity_items: usize,
    active_depth: usize,
    max_active_depth: usize,
}

impl DecodeContext {
    fn register_uuid(&mut self, uuid: Option<Uuid>) -> Result<(), SourceAdapterError> {
        if uuid.is_some() {
            self.register_identity_item()?;
        }
        if uuid.is_some_and(|uuid| !self.uuids.insert(uuid)) {
            return Err(error(
                SourceAdapterErrorKind::IdentityCollision,
                "Platform XML snapshot contains duplicate native UUID identity",
            ));
        }
        Ok(())
    }

    fn enter_node(&mut self) -> Result<(), SourceAdapterError> {
        let depth = self.active_depth.checked_add(1).ok_or_else(|| {
            error(
                SourceAdapterErrorKind::ResourceLimit,
                "Platform XML nesting depth overflow",
            )
        })?;
        if depth > MAX_NAVIGATION_NESTING_DEPTH {
            return Err(error(
                SourceAdapterErrorKind::ResourceLimit,
                "Platform XML nesting depth exceeds navigation limit",
            ));
        }
        Self::increment(&mut self.native_nodes, MAX_NAVIGATION_NODES, "native nodes")?;
        self.active_depth = depth;
        self.max_active_depth = self.max_active_depth.max(depth);
        Ok(())
    }

    fn leave_node(&mut self) {
        self.active_depth = self.active_depth.saturating_sub(1);
    }

    fn register_relation(&mut self) -> Result<(), SourceAdapterError> {
        Self::increment(
            &mut self.relations,
            MAX_NAVIGATION_RELATIONS,
            "child relations",
        )
    }

    fn register_property(&mut self, properties_in_node: usize) -> Result<(), SourceAdapterError> {
        if properties_in_node >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
            return Err(error(
                SourceAdapterErrorKind::ResourceLimit,
                "Platform XML node has too many scalar properties",
            ));
        }
        Self::increment(
            &mut self.properties,
            MAX_NAVIGATION_IDENTITY_ITEMS,
            "scalar properties",
        )
    }

    fn register_identity_item(&mut self) -> Result<(), SourceAdapterError> {
        Self::increment(
            &mut self.identity_items,
            MAX_NAVIGATION_IDENTITY_ITEMS,
            "identity items",
        )
    }

    fn increment(counter: &mut usize, limit: usize, label: &str) -> Result<(), SourceAdapterError> {
        let next = counter.checked_add(1).ok_or_else(|| {
            error(
                SourceAdapterErrorKind::ResourceLimit,
                format!("Platform XML {label} overflow"),
            )
        })?;
        if next > limit {
            return Err(error(
                SourceAdapterErrorKind::ResourceLimit,
                format!("Platform XML {label} exceed navigation limit {limit}"),
            ));
        }
        *counter = next;
        Ok(())
    }
}

fn decode_scoped<T>(
    context: &mut DecodeContext,
    decode: impl FnOnce(&mut DecodeContext) -> Result<T, SourceAdapterError>,
) -> Result<T, SourceAdapterError> {
    context.enter_node()?;
    let result = decode(context);
    context.leave_node();
    result
}

struct DecodedChildren {
    nodes: Vec<NativeMetadataChild>,
    complete: bool,
    unmapped_facts: usize,
}

struct ParsedDescriptor {
    uuid: Option<Uuid>,
    properties: DecodedProperties,
}

fn corrupted(message: impl Into<String>) -> SourceAdapterError {
    error(SourceAdapterErrorKind::DecodeCorrupted, message)
}

fn error(kind: SourceAdapterErrorKind, message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(kind, message)
}

#[cfg(test)]
mod direct_type_property_tests {
    use roxmltree::Document;

    use crate::{
        domain::source_adapters::SourceAdapterErrorKind,
        infrastructure::source_adapters::platform_xml::native_model::NativePropertyValue,
    };

    use super::{decode_properties, DecodeContext};

    #[test]
    fn direct_inherited_official_qname_is_a_type_set_not_a_scalar() {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xs="http://www.w3.org/2001/XMLSchema"><Type>xs:string</Type></Properties>"#,
        )
        .unwrap();

        let properties =
            decode_properties(document.root_element(), "", &mut DecodeContext::default()).unwrap();

        assert!(matches!(
            &properties["Type"].value,
            NativePropertyValue::TypeSet(_)
        ));
    }

    #[test]
    fn direct_foreign_qname_fails_closed_instead_of_becoming_a_scalar() {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:alien="urn:alien"><DataType>alien:string</DataType></Properties>"#,
        )
        .unwrap();

        let error = decode_properties(document.root_element(), "", &mut DecodeContext::default())
            .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn unbound_direct_qname_is_rejected_by_type_namespace_resolution() {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><Type>unbound:string</Type></Properties>"#,
        )
        .unwrap();

        let error = decode_properties(document.root_element(), "", &mut DecodeContext::default())
            .unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        fmt::Write,
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::domain::{
        navigation::CoverageState,
        navigation_limits::{
            MAX_NAVIGATION_NESTING_DEPTH, MAX_NAVIGATION_NODES, MAX_NAVIGATION_PROPERTIES_PER_NODE,
        },
        source_adapters::{
            FormatVersion, SnapshotEvidence, SourceAdapterErrorKind, SourceDescriptor,
            SourceFamily, SourceId, SourceRevision,
        },
    };

    use super::{
        decode, decode_with_context, DecodeContext, NativeEvidenceState, NativeMxlRootKind,
        NativeNodeBacking, NativeNodeState, NativePropertyProvenance, NativePropertyValue,
        PlatformXmlProvider,
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
    fn root_descriptor_depth_preflight_stops_before_dom_parse() {
        let fixture = fixture(
            "Shipment.xml",
            &[(
                "Shipment.xml",
                deeply_nested_unclosed_xml("MetaDataObject", super::METADATA_NAMESPACE),
            )],
        );

        assert_eq!(
            decode(&fixture.provider, &fixture.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::ResourceLimit
        );
    }

    #[test]
    fn companion_xml_depth_preflight_stops_before_dom_parse() {
        let fixtures = [
            document_fixture_with_files(
                "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
                &[(
                    "Shipment/Forms/ItemForm.xml",
                    deeply_nested_unclosed_xml("MetaDataObject", super::METADATA_NAMESPACE),
                )],
            ),
            document_fixture_with_files(
                "<ChildObjects><Template>Print</Template></ChildObjects>",
                &[(
                    "Shipment/Templates/Print.xml",
                    deeply_nested_unclosed_xml("MetaDataObject", super::METADATA_NAMESPACE),
                )],
            ),
            document_fixture_with_files(
                "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
                &[
                    ("Shipment/Forms/ItemForm.xml", form_descriptor("ItemForm")),
                    (
                        "Shipment/Forms/ItemForm/Ext/Form.xml",
                        deeply_nested_unclosed_xml("Form", super::MANAGED_FORM_NAMESPACE),
                    ),
                ],
            ),
            document_fixture_with_files(
                "<ChildObjects><Template>Print</Template></ChildObjects>",
                &[
                    (
                        "Shipment/Templates/Print.xml",
                        template_descriptor("Print", "SpreadsheetDocument"),
                    ),
                    (
                        "Shipment/Templates/Print/Ext/Template.xml",
                        deeply_nested_unclosed_xml(
                            "SpreadsheetDocument",
                            super::SPREADSHEET_DOCUMENT_NAMESPACE,
                        ),
                    ),
                ],
            ),
        ];

        for fixture in fixtures {
            assert_eq!(
                decode(&fixture.provider, &fixture.descriptor)
                    .unwrap_err()
                    .kind,
                SourceAdapterErrorKind::ResourceLimit
            );
        }
    }

    #[test]
    fn streaming_xml_depth_preflight_handles_xml_syntax_tokens_and_malformed_input() {
        let accepted = r#"<?xml version="1.0"?>
            <!DOCTYPE Form [<!ELEMENT Form ANY>]>
            <?platform processing?>
            <Form><!-- comment --><![CDATA[<not-an-element/>]]><Empty/></Form>"#;
        assert!(super::preflight_xml_nesting(accepted, "malformed").is_ok());

        let malformed =
            super::preflight_xml_nesting("<Form><Nested></Form>", "malformed").unwrap_err();
        assert_eq!(malformed.kind, SourceAdapterErrorKind::DecodeCorrupted);

        let deep = super::preflight_xml_nesting(
            &deeply_nested_unclosed_xml("Form", super::MANAGED_FORM_NAMESPACE),
            "malformed",
        )
        .unwrap_err();
        assert_eq!(deep.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn streaming_xml_depth_preflight_bounds_empty_elements_before_dom_parse() {
        let at_limit = nested_empty_leaf_xml(MAX_NAVIGATION_NESTING_DEPTH - 1, true);
        assert!(super::parse_bounded_xml_document(
            at_limit.as_bytes(),
            "invalid UTF-8",
            "malformed XML",
        )
        .is_ok());

        let over_limit = nested_empty_leaf_xml(MAX_NAVIGATION_NESTING_DEPTH, false);
        let error = super::parse_bounded_xml_document(
            over_limit.as_bytes(),
            "invalid UTF-8",
            "malformed XML",
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn companion_metadata_class_max_plus_one_stops_without_collecting_all_classes() {
        let mut classes = String::new();
        for _ in 0..=MAX_NAVIGATION_NODES {
            classes.push_str("<Form/>");
        }
        let descriptor = format!(
            "<MetaDataObject xmlns=\"{}\">{classes}</MetaDataObject>",
            super::METADATA_NAMESPACE
        );
        let fixture = document_fixture_with_files(
            "<ChildObjects><Form>ItemForm</Form></ChildObjects>",
            &[("Shipment/Forms/ItemForm.xml", descriptor)],
        );

        assert_eq!(
            decode(&fixture.provider, &fixture.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::ProjectionAmbiguous
        );
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
                type_annotation: crate::versions::v2_20::native_model::NativeScalarType::Decimal,
            },
        );
    }

    #[test]
    fn scalar_annotation_failures_are_local_and_do_not_stop_siblings() {
        use crate::versions::v2_20::native_model::NativeScalarAnnotationIssue;

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
        use crate::versions::v2_20::native_model::NativeScalarAnnotationIssue;

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
        use crate::versions::v2_20::native_model::{NativeScalarAnnotationIssue, NativeScalarType};

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

    #[test]
    fn native_child_and_property_limits_fail_before_over_limit_construction() {
        let mut children = String::new();
        for index in 0..=MAX_NAVIGATION_NODES {
            write!(
                children,
                "<Attribute><Properties><Name>Attribute{index}</Name></Properties></Attribute>"
            )
            .unwrap();
        }
        let fixture = document_fixture(&format!("<ChildObjects>{children}</ChildObjects>"));
        let mut context = DecodeContext::default();
        let error =
            decode_with_context(&fixture.provider, &fixture.descriptor, &mut context).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert_eq!(context.native_nodes, MAX_NAVIGATION_NODES);

        let mut properties = String::from("<Properties><Name>Shipment</Name>");
        for index in 0..MAX_NAVIGATION_PROPERTIES_PER_NODE {
            write!(properties, "<Property{index}>value</Property{index}>").unwrap();
        }
        properties.push_str("</Properties>");
        let fixture = document_fixture(&properties);
        let mut context = DecodeContext::default();
        let error =
            decode_with_context(&fixture.provider, &fixture.descriptor, &mut context).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert_eq!(context.properties, MAX_NAVIGATION_PROPERTIES_PER_NODE);
    }

    #[test]
    fn at_limit_navigation_fixture_stays_inside_provider_bytes() {
        let mut children = String::new();
        for index in 0..(MAX_NAVIGATION_NODES - 2) {
            write!(
                children,
                "<Attribute><Properties><Name>Bounded{index}</Name></Properties></Attribute>"
            )
            .unwrap();
        }
        let xml = metadata_document(&format!("<ChildObjects>{children}</ChildObjects>"));
        assert!((xml.len() as u64) < crate::safe_root::ArtifactReadLimit::Descriptor.bytes());
        let fixture = fixture("Shipment.xml", &[("Shipment.xml", xml)]);
        let navigation = crate::versions::v2_20::PlatformXmlReadAdapter::new()
            .inspect_provider(&fixture.provider, &fixture.descriptor)
            .unwrap();
        assert_eq!(navigation.nodes.len(), MAX_NAVIGATION_NODES);
    }

    #[test]
    fn deep_native_nesting_stops_before_growing_past_the_shared_limit() {
        let mut body = String::new();
        for index in 0..MAX_NAVIGATION_NESTING_DEPTH {
            write!(
                body,
                "<ChildObjects><TabularSection><Properties><Name>Level{index}</Name></Properties>"
            )
            .unwrap();
        }
        for _ in 0..MAX_NAVIGATION_NESTING_DEPTH {
            body.push_str("</TabularSection></ChildObjects>");
        }
        let fixture = document_fixture(&body);
        let error = decode(&fixture.provider, &fixture.descriptor).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn decoder_uses_the_exact_shared_identifier_grammar() {
        let valid = fixture(
            "Ёж_2.xml",
            &[(
                "Ёж_2.xml",
                metadata_document("<Properties><Name>Ёж_2</Name></Properties>"),
            )],
        );
        assert!(decode(&valid.provider, &valid.descriptor).is_ok());

        let invalid = fixture(
            "Delta.xml",
            &[(
                "Delta.xml",
                metadata_document("<Properties><Name>Δelta</Name></Properties>"),
            )],
        );
        assert_eq!(
            decode(&invalid.provider, &invalid.descriptor)
                .unwrap_err()
                .kind,
            SourceAdapterErrorKind::DecodeCorrupted
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

    fn deeply_nested_unclosed_xml(root: &str, namespace: &str) -> String {
        let mut xml = format!("<{root} xmlns=\"{namespace}\">");
        for _ in 1..MAX_NAVIGATION_NESTING_DEPTH {
            xml.push_str("<Nested>");
        }
        xml.push_str("<Leaf/>");
        xml
    }

    fn nested_empty_leaf_xml(container_depth: usize, close_containers: bool) -> String {
        let mut xml = String::new();
        for _ in 0..container_depth {
            xml.push_str("<Container>");
        }
        xml.push_str("<Leaf/>");
        if close_containers {
            for _ in 0..container_depth {
                xml.push_str("</Container>");
            }
        }
        xml
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
        let provider = PlatformXmlProvider::capture(root.join(root_key), &root).unwrap();
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
