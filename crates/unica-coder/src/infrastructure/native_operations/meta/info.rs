use crate::application::metadata::MetaFailure;
use crate::application::ports::{
    MetaLocalInfo, MetadataEvidenceAvailability, MetadataResourceImage, MetadataResourceRole,
    MetadataSubsystemEvidence, MetadataValidationSubject,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::{
    metadata_identifier_is_valid, MetaCollectionsData, MetaDiagnostic, MetaDiagnosticCode,
    MetaDiagnosticSeverity, MetaElementData, MetaEventSource, MetaInfoDetails,
    MetaInfoPropertyData, MetaInfoPropertyValue, MetaInfoPropertyValueKind, MetaRelationTargetData,
    MetaRelationsData, MetaSupportStatus, MetadataKind, META_INFO_PROPERTY_PROFILE,
};
use crate::domain::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::support_state::{ObjectSupportState, SupportStateReader};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_read_target, TargetKindPolicy,
};
use roxmltree::Document;
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::infrastructure::platform::secure_read::{
    capture_root_relative_regular_files, SecureTreeCaptureLimits,
};
use crate::infrastructure::subsystem_topology::{
    capture_registered_subsystem_topology, MetadataObjectIdentity,
};

const REGISTRAR_SCAN_MAX_ENTRIES: usize = 20_000;
const REGISTRAR_SCAN_MAX_FILES: usize = 20_000;
const REGISTRAR_SCAN_MAX_BYTES: usize = 64 * 1024 * 1024;

#[cfg(test)]
type MetaInfoDescriptorImageHook = Box<dyn FnOnce(&[u8]) -> Vec<u8>>;

#[cfg(test)]
thread_local! {
    static META_INFO_DESCRIPTOR_IMAGE_HOOK:
        std::cell::RefCell<Option<MetaInfoDescriptorImageHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_meta_info_descriptor_image_hook<T>(
    hook: impl FnOnce(&[u8]) -> Vec<u8> + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<MetaInfoDescriptorImageHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

#[cfg(test)]
fn meta_info_descriptor_image_for_test(bytes: &[u8]) -> Option<Vec<u8>> {
    META_INFO_DESCRIPTOR_IMAGE_HOOK.with(|slot| slot.borrow_mut().take().map(|hook| hook(bytes)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrarProcessingPhase {
    AfterIdentityParse {
        logical_path: String,
        ordinal: usize,
        total: usize,
    },
    AfterRegistrarParse {
        logical_path: String,
        ordinal: usize,
        total: usize,
    },
    BeforeCompleteReturn,
}

#[cfg(test)]
type RegistrarProcessingHook = Box<dyn FnMut(&RegistrarProcessingPhase)>;

#[cfg(test)]
thread_local! {
    static REGISTRAR_PROCESSING_HOOK:
        std::cell::RefCell<Option<RegistrarProcessingHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_registrar_processing_hook<T>(
    hook: impl FnMut(&RegistrarProcessingPhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<RegistrarProcessingHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            REGISTRAR_PROCESSING_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous = REGISTRAR_PROCESSING_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn emit_registrar_processing_phase(phase: RegistrarProcessingPhase) {
    #[cfg(test)]
    REGISTRAR_PROCESSING_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubsystemEvidenceProcessingPhase {
    BeforeCompleteReturn,
}

#[cfg(test)]
type SubsystemEvidenceProcessingHook = Box<dyn FnMut(&SubsystemEvidenceProcessingPhase)>;

#[cfg(test)]
thread_local! {
    static SUBSYSTEM_EVIDENCE_PROCESSING_HOOK:
        std::cell::RefCell<Option<SubsystemEvidenceProcessingHook>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_subsystem_evidence_processing_hook<T>(
    hook: impl FnMut(&SubsystemEvidenceProcessingPhase) + 'static,
    action: impl FnOnce() -> T,
) -> T {
    struct Reset(Option<SubsystemEvidenceProcessingHook>);
    impl Drop for Reset {
        fn drop(&mut self) {
            SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| slot.replace(self.0.take()));
        }
    }

    let previous =
        SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| slot.replace(Some(Box::new(hook))));
    let _reset = Reset(previous);
    action()
}

fn emit_subsystem_evidence_processing_phase(phase: SubsystemEvidenceProcessingPhase) {
    #[cfg(test)]
    SUBSYSTEM_EVIDENCE_PROCESSING_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().as_mut() {
            hook(&phase);
        }
    });
    #[cfg(not(test))]
    let _ = phase;
}

use super::edit::ResolvedMetadataObject;
use super::xml_model::{
    meta_event_subscription_source_node, meta_info_child_text, meta_info_inner_text,
    meta_info_ml_text, meta_info_normalize_cfg_prefix, parse_defined_type_event_sources,
    parse_meta_event_subscription_source,
};

/// Parse the descriptor image already acquired by the logical resolver. The
/// same bytes are retained in the validation subject, so typed info never
/// performs a second descriptor read between structure and validation.
pub(crate) fn read_typed_meta_info(
    resolved: &ResolvedMetadataObject,
    source_set: &str,
    target: &MetadataAddress,
    context: &WorkspaceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
    support_reader: &dyn SupportStateReader,
) -> Result<(MetaLocalInfo, MetadataValidationSubject), MetaFailure> {
    #[cfg(test)]
    let test_descriptor_image = meta_info_descriptor_image_for_test(&resolved.descriptor_preimage);
    #[cfg(test)]
    let descriptor_preimage = test_descriptor_image
        .as_deref()
        .unwrap_or(&resolved.descriptor_preimage);
    #[cfg(not(test))]
    let descriptor_preimage = resolved.descriptor_preimage.as_slice();
    let identity = super::validation_context::inspect_metadata_image_identity(descriptor_preimage)
        .map_err(|error| {
            let diagnostic = MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                format!("metadata descriptor identity proof is invalid: {error}"),
            )
            .with_metadata_path(target.clone());
            MetaFailure::from(match error.field() {
                Some(field) => diagnostic.with_field(field),
                None => diagnostic,
            })
        })?;
    let text = std::str::from_utf8(descriptor_preimage)
        .expect("the exact metadata identity proof parsed these bytes as UTF-8");
    let xml = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(xml)
        .expect("the exact metadata identity proof parsed the same XML document");
    let object = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .expect("the exact metadata identity proof found one object");
    let kind = match identity.object_type.as_str() {
        "ExternalDataProcessor" => Ok(MetadataKind::DataProcessor),
        "ExternalReport" => Ok(MetadataKind::Report),
        other => MetadataKind::parse(other),
    }
    .map_err(|diagnostic| {
        MetaFailure::from(
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, diagnostic.message)
                .with_metadata_path(target.clone()),
        )
    })?;
    let mut expected = target.segments();
    let expected_type = expected.next().unwrap_or_default();
    let expected_name = expected.next().unwrap_or_default();
    if identity.object_type != expected_type || identity.object_name != expected_name {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            format!(
                "metadata descriptor identity {}.{} does not match target {target}",
                identity.object_type, identity.object_name
            ),
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let object_identity = MetadataObjectIdentity {
        address: target.clone(),
        uuid: identity.object_uuid,
    };
    let property_containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "Properties")
        .collect::<Vec<_>>();
    let [properties] = property_containers.as_slice() else {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor must contain exactly one direct Properties container",
        )
        .with_metadata_path(target.clone())
        .with_field("properties")
        .into());
    };
    if properties.tag_name().namespace() != Some(super::info_projection::MD_CLASSES_NAMESPACE) {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor Properties container has the wrong namespace",
        )
        .with_metadata_path(target.clone())
        .with_field("properties")
        .into());
    }
    let child_object_containers = object
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "ChildObjects")
        .collect::<Vec<_>>();
    if child_object_containers.len() > 1
        || child_object_containers.iter().any(|node| {
            node.tag_name().namespace() != Some(super::info_projection::MD_CLASSES_NAMESPACE)
        })
    {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor contains a duplicate or foreign ChildObjects container",
        )
        .with_metadata_path(target.clone())
        .with_field("collections")
        .into());
    }
    let properties = Some(*properties);
    let child_objects = child_object_containers.first().copied();
    let name_nodes = super::info_projection::direct_children_with_namespace(
        *properties.as_ref().expect("proved Properties"),
        super::info_projection::MD_CLASSES_NAMESPACE,
        "Name",
    );
    let name = match name_nodes.as_slice() {
        [node]
            if node.attributes().len() == 0 && !node.children().any(|child| child.is_element()) =>
        {
            super::info_projection::direct_text_content(*node)
        }
        _ => String::new(),
    };
    if name.is_empty() {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor has no object name",
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let synonym = properties.and_then(|properties| {
        let nodes = super::info_projection::direct_children_with_namespace(
            properties,
            super::info_projection::MD_CLASSES_NAMESPACE,
            "Synonym",
        );
        let [node] = nodes.as_slice() else {
            return None;
        };
        super::info_projection::meta_info_property_value_is_valid(
            *node,
            MetaInfoPropertyValueKind::LegacyLocalizedString,
        )
        .then(|| meta_info_ml_text(*node))
        .filter(|value| !value.is_empty())
    });

    let mut diagnostics = Vec::new();
    for error in super::info_projection::meta_info_profile_errors(kind, properties, child_objects) {
        diagnostics.push(
            MetaDiagnostic::error(error.code, error.message)
                .with_metadata_path(target.clone())
                .with_field(error.field),
        );
    }
    let details = super::info_projection::project_meta_info_details(
        kind,
        properties,
        child_objects,
        target,
        &mut diagnostics,
    );
    let details = match identity.object_type.as_str() {
        "ExternalDataProcessor" => MetaInfoDetails::ExternalDataProcessor {},
        "ExternalReport" => MetaInfoDetails::ExternalReport {},
        _ => details,
    };
    let declarations = super::info_projection::project_meta_info_declarations(
        kind,
        properties,
        target,
        &mut diagnostics,
    );
    let collection_route = |tag, nested_attributes, field| {
        TypedRootCollectionRoute::new(kind, tag, nested_attributes, field)
    };
    let support = typed_support_status(support_reader, &resolved.resolved_target)?;
    let mut local = MetaLocalInfo {
        metadata_path: target.clone(),
        kind,
        details,
        name,
        synonym,
        support,
        properties: typed_properties(properties, kind),
        declarations,
        predefined_code_type: predefined_code_type_for_info(properties, kind),
        relations: typed_relations(&doc, properties, kind, target, &mut diagnostics),
        collections: MetaCollectionsData {
            attributes: typed_root_collection(
                xml,
                child_objects,
                collection_route("Attribute", false, "collections.attributes"),
                target,
                &mut diagnostics,
            ),
            tabular_sections: typed_root_collection(
                xml,
                child_objects,
                collection_route("TabularSection", true, "collections.tabularSections"),
                target,
                &mut diagnostics,
            ),
            dimensions: typed_root_collection(
                xml,
                child_objects,
                collection_route("Dimension", false, "collections.dimensions"),
                target,
                &mut diagnostics,
            ),
            resources: typed_root_collection(
                xml,
                child_objects,
                collection_route("Resource", false, "collections.resources"),
                target,
                &mut diagnostics,
            ),
            recalculations: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route("Recalculation", false, "collections.recalculations"),
                target,
                &mut diagnostics,
            ),
            accounting_flags: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route("AccountingFlag", false, "collections.accountingFlags"),
                target,
                &mut diagnostics,
            ),
            ext_dimension_accounting_flags: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route(
                    "ExtDimensionAccountingFlag",
                    false,
                    "collections.extDimensionAccountingFlags",
                ),
                target,
                &mut diagnostics,
            ),
            addressing_attributes: typed_optional_root_collection(
                xml,
                child_objects,
                collection_route(
                    "AddressingAttribute",
                    false,
                    "collections.addressingAttributes",
                ),
                target,
                &mut diagnostics,
            ),
            enum_values: typed_root_collection(
                xml,
                child_objects,
                collection_route("EnumValue", false, "collections.enumValues"),
                target,
                &mut diagnostics,
            ),
            columns: typed_root_collection(
                xml,
                child_objects,
                collection_route("Column", false, "collections.columns"),
                target,
                &mut diagnostics,
            ),
            forms: typed_root_collection(
                xml,
                child_objects,
                collection_route("Form", false, "collections.forms"),
                target,
                &mut diagnostics,
            ),
            templates: typed_root_collection(
                xml,
                child_objects,
                collection_route("Template", false, "collections.templates"),
                target,
                &mut diagnostics,
            ),
            commands: typed_root_collection(
                xml,
                child_objects,
                collection_route("Command", false, "collections.commands"),
                target,
                &mut diagnostics,
            ),
        },
        diagnostics,
    };
    let mut validation_resources = vec![
        MetadataResourceImage {
            role: MetadataResourceRole::Descriptor,
            bytes: descriptor_preimage.to_vec(),
        },
        MetadataResourceImage {
            role: MetadataResourceRole::Registration,
            bytes: resolved.owner_preimage.clone(),
        },
    ];
    validation_resources.extend(typed_registered_language_images(resolved));
    let _ = super::validation::add_descriptor_references(
        &mut validation_resources,
        descriptor_preimage,
        Some(&resolved.source_root),
    );
    let (event_source_resources, event_source_diagnostics) = typed_event_source_dependency_images(
        resolved,
        source_set,
        &local.relations.source,
        context,
        deadline,
        cancellation,
    );
    validation_resources.extend(event_source_resources);
    local.diagnostics.extend(event_source_diagnostics);
    let (registrar_resources, registrar_evidence) =
        typed_registrar_document_images(resolved, kind, properties, target, deadline, cancellation);
    validation_resources.extend(registrar_resources);
    let subsystem_evidence = Some(typed_subsystem_evidence(
        resolved,
        &object_identity,
        deadline,
        cancellation,
    ));
    let child_resources = match super::edit::observe_typed_child_resources(
        &resolved.descriptor_path,
        target,
        &identity.object_type,
        &local.name,
        xml,
    ) {
        Ok(resources) => resources,
        Err(failure) => {
            local.diagnostics.extend(failure.diagnostics);
            super::edit::TypedChildObservation::default()
        }
    };
    local.diagnostics.extend(child_resources.diagnostics);
    validation_resources.extend(child_resources.validation_resources);
    let validation_subject = MetadataValidationSubject {
        target: target.clone(),
        resources: validation_resources,
        child_footprints: child_resources.validation_footprints,
        registrar_evidence,
        subsystem_evidence,
    };
    Ok((local, validation_subject))
}

fn typed_event_source_dependency_images(
    resolved: &ResolvedMetadataObject,
    source_set: &str,
    sources: &[MetaEventSource],
    context: &WorkspaceContext,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> (Vec<MetadataResourceImage>, Vec<MetaDiagnostic>) {
    let mut resources = Vec::new();
    let mut diagnostics = Vec::new();
    let mut seen = HashSet::new();
    let mut queue = sources.to_vec();
    let mut cursor = 0usize;
    while let Some(source) = queue.get(cursor).cloned() {
        cursor += 1;
        if let Err(error) = registrar_scan_checkpoint(deadline, cancellation) {
            let message = if error.kind() == io::ErrorKind::TimedOut {
                "EventSubscription Source dependency scan deadline elapsed"
            } else {
                "EventSubscription Source dependency scan was interrupted"
            };
            diagnostics.push(
                MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
                    .with_metadata_path(resolved.metadata_path.clone())
                    .with_field("relations.source"),
            );
            break;
        }
        let Some(metadata_path) = source.metadata_path().cloned() else {
            continue;
        };
        if !seen.insert(metadata_path.clone()) {
            continue;
        }
        let source_target = SourceTarget {
            source_set: source_set.to_string(),
            metadata_path: Some(metadata_path.clone()),
        };
        let Ok(resolution) =
            resolve_platform_xml_read_target(context, &source_target, TargetKindPolicy::Any)
        else {
            continue;
        };
        if resolution.resolved.target_kind != TargetKind::MetadataObject {
            continue;
        }
        let Ok(evidence) = platform_xml_resource_evidence(context, &resolution.handle) else {
            continue;
        };
        if evidence.source_root != resolved.source_root
            || evidence.registration_path != resolved.owner_path
        {
            continue;
        }
        let Ok(bytes) = fs::read(&evidence.target_path) else {
            continue;
        };
        if let Err(error) = registrar_scan_checkpoint(deadline, cancellation) {
            let message = if error.kind() == io::ErrorKind::TimedOut {
                "EventSubscription Source dependency scan deadline elapsed"
            } else {
                "EventSubscription Source dependency scan was interrupted"
            };
            diagnostics.push(
                MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
                    .with_metadata_path(resolved.metadata_path.clone())
                    .with_field("relations.source"),
            );
            break;
        }
        if matches!(source, MetaEventSource::DefinedType { .. }) {
            if let Ok(members) = parse_defined_type_event_sources(&bytes) {
                queue.extend(members);
            }
        }
        resources.push(MetadataResourceImage {
            role: MetadataResourceRole::Dependency {
                target: metadata_path,
            },
            bytes,
        });
    }
    (resources, diagnostics)
}

pub(super) fn predefined_code_type_for_info(
    properties: Option<roxmltree::Node<'_, '_>>,
    kind: MetadataKind,
) -> Option<String> {
    match kind {
        MetadataKind::Catalog | MetadataKind::ChartOfCalculationTypes => {
            let code_type = properties.and_then(|properties| {
                let nodes = super::info_projection::direct_children_with_namespace(
                    properties,
                    super::info_projection::MD_CLASSES_NAMESPACE,
                    "CodeType",
                );
                let [node] = nodes.as_slice() else {
                    return None;
                };
                (node.attributes().len() == 0 && !node.children().any(|child| child.is_element()))
                    .then(|| {
                        node.children()
                            .filter(roxmltree::Node::is_text)
                            .filter_map(|child| child.text())
                            .collect::<String>()
                            .trim()
                            .to_string()
                    })
                    .filter(|value| !value.is_empty())
            });
            Some(code_type.unwrap_or_else(|| "String".to_string()))
        }
        MetadataKind::ChartOfAccounts | MetadataKind::ChartOfCharacteristicTypes => {
            Some("String".to_string())
        }
        _ => None,
    }
}

fn typed_registrar_document_images(
    resolved: &ResolvedMetadataObject,
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> (Vec<MetadataResourceImage>, MetadataEvidenceAvailability) {
    let reads_registrars = matches!(
        kind,
        MetadataKind::AccumulationRegister
            | MetadataKind::AccountingRegister
            | MetadataKind::CalculationRegister
    ) || (kind == MetadataKind::InformationRegister
        && properties
            .and_then(|node| meta_info_child_text(node, "WriteMode"))
            .as_deref()
            == Some("RecorderSubordinate"));
    if !reads_registrars {
        return (Vec::new(), MetadataEvidenceAvailability::Complete);
    }
    let checkpoint = || registrar_scan_checkpoint(deadline, cancellation);
    let entries = match capture_root_relative_regular_files(
        &resolved.source_root,
        Path::new("Documents"),
        SecureTreeCaptureLimits {
            maximum_depth: 0,
            maximum_entries: REGISTRAR_SCAN_MAX_ENTRIES,
            maximum_files: REGISTRAR_SCAN_MAX_FILES,
            maximum_bytes: REGISTRAR_SCAN_MAX_BYTES,
        },
        |_| false,
        |path| path.extension().and_then(|extension| extension.to_str()) == Some("xml"),
        &checkpoint,
    ) {
        Ok(entries) => entries,
        Err(_) => {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence cannot be scanned completely",
            )
        }
    };
    if entries.start_missing {
        emit_registrar_processing_phase(RegistrarProcessingPhase::BeforeCompleteReturn);
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        return (Vec::new(), MetadataEvidenceAvailability::Complete);
    }
    let mut resources = Vec::new();
    let register_reference = target.as_str().to_string();
    let total = entries.files.len();
    for (ordinal, entry) in entries.files.into_iter().enumerate() {
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        let logical_path = entry.logical_path;
        let bytes = entry.bytes;
        let identity_result = super::validation_context::inspect_metadata_image_identity(&bytes);
        emit_registrar_processing_phase(RegistrarProcessingPhase::AfterIdentityParse {
            logical_path: logical_path.clone(),
            ordinal,
            total,
        });
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        let identity = match identity_result {
            Ok(identity) if identity.object_type == "Document" => identity,
            _ => {
                return registrar_evidence_unavailable(
                    target,
                    "registrar evidence candidate is malformed",
                )
            }
        };
        let dependency_target = match MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("{}.{}", identity.object_type, identity.object_name),
        ) {
            Ok(target) => target,
            Err(_) => {
                return registrar_evidence_unavailable(
                    target,
                    "registrar evidence candidate has no logical identity",
                )
            }
        };
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        let registers_target = super::validation::document_registers(&bytes, &register_reference);
        emit_registrar_processing_phase(RegistrarProcessingPhase::AfterRegistrarParse {
            logical_path,
            ordinal,
            total,
        });
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        if registers_target {
            resources.push(MetadataResourceImage {
                role: MetadataResourceRole::Dependency {
                    target: dependency_target,
                },
                bytes,
            });
        }
    }
    emit_registrar_processing_phase(RegistrarProcessingPhase::BeforeCompleteReturn);
    if checkpoint().is_err() {
        return registrar_evidence_unavailable(
            target,
            "registrar evidence processing was interrupted",
        );
    }
    (resources, MetadataEvidenceAvailability::Complete)
}

pub(super) fn registrar_scan_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> io::Result<()> {
    if cancellation.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "registrar evidence scan was cancelled",
        ))
    } else if deadline.remaining().is_zero() {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "registrar evidence scan deadline elapsed",
        ))
    } else {
        Ok(())
    }
}

fn registrar_evidence_unavailable(
    target: &MetadataAddress,
    message: &str,
) -> (Vec<MetadataResourceImage>, MetadataEvidenceAvailability) {
    (
        Vec::new(),
        MetadataEvidenceAvailability::Unavailable(vec![MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            message,
        )
        .with_metadata_path(target.clone())
        .with_field("registrarEvidence")]),
    )
}

fn subsystem_evidence_unavailable(
    target: &MetadataAddress,
    message: &str,
) -> MetadataSubsystemEvidence {
    MetadataSubsystemEvidence::Unavailable(vec![MetaDiagnostic::error(
        MetaDiagnosticCode::ProviderUnavailable,
        message,
    )
    .with_metadata_path(target.clone())
    .with_field("subsystemEvidence")])
}

/// Collects all registered subsystem memberships of the current object. The
/// same complete evidence is serialized by `meta.info` and consumed by the
/// command-interface rule when the object is an eligible register.
fn typed_subsystem_evidence(
    resolved: &ResolvedMetadataObject,
    identity: &MetadataObjectIdentity,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> MetadataSubsystemEvidence {
    if resolved
        .owner_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some("Configuration.xml")
    {
        emit_subsystem_evidence_processing_phase(
            SubsystemEvidenceProcessingPhase::BeforeCompleteReturn,
        );
        if registrar_scan_checkpoint(deadline, cancellation).is_err() {
            return subsystem_evidence_unavailable(
                &identity.address,
                "registered subsystem topology processing was interrupted",
            );
        }
        return MetadataSubsystemEvidence::Complete {
            functional_subsystems: Vec::new(),
            interface_subsystems: Vec::new(),
        };
    }
    let topology = match capture_registered_subsystem_topology(&resolved.source_root, || {
        registrar_scan_checkpoint(deadline, cancellation)
    }) {
        Ok(topology) => topology,
        Err(_) => {
            return subsystem_evidence_unavailable(
                &identity.address,
                "registered subsystem topology cannot be proved completely",
            )
        }
    };
    emit_subsystem_evidence_processing_phase(
        SubsystemEvidenceProcessingPhase::BeforeCompleteReturn,
    );
    if registrar_scan_checkpoint(deadline, cancellation).is_err() {
        return subsystem_evidence_unavailable(
            &identity.address,
            "registered subsystem topology processing was interrupted",
        );
    }
    MetadataSubsystemEvidence::Complete {
        functional_subsystems: topology.functional_memberships_for(identity),
        interface_subsystems: topology.interface_memberships_for(identity),
    }
}

fn typed_registered_language_images(
    resolved: &ResolvedMetadataObject,
) -> Vec<MetadataResourceImage> {
    let Ok(owner) = std::str::from_utf8(&resolved.owner_preimage) else {
        return Vec::new();
    };
    let Ok(document) = Document::parse(owner.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let configuration = document.root_element().children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
            && node.tag_name().name() == "Configuration"
    });
    let child_objects = configuration.and_then(|configuration| {
        configuration.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
                && node.tag_name().name() == "ChildObjects"
        })
    });
    child_objects
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
                && node.tag_name().name() == "Language"
        })
        .map(super::info_projection::direct_text_content)
        .filter_map(|name| (!name.trim().is_empty()).then_some(name))
        .filter(|name| metadata_identifier_is_valid(name))
        .filter_map(|name| {
            let metadata_path = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("Language.{name}"),
            )
            .ok()?;
            let bytes = fs::read(
                resolved
                    .source_root
                    .join("Languages")
                    .join(format!("{name}.xml")),
            )
            .ok()?;
            Some(MetadataResourceImage {
                role: MetadataResourceRole::Dependency {
                    target: metadata_path,
                },
                bytes,
            })
        })
        .collect()
}

fn typed_support_status(
    support_reader: &dyn SupportStateReader,
    target: &ResolvedTarget,
) -> Result<MetaSupportStatus, MetaFailure> {
    let support = support_reader.object_support(target).map_err(|error| {
        let mut diagnostic =
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, error.to_string());
        if let Some(metadata_path) = target.metadata_path.clone() {
            diagnostic = diagnostic.with_metadata_path(metadata_path);
        }
        MetaFailure::from(diagnostic)
    })?;
    Ok(match support.state {
        ObjectSupportState::Locked | ObjectSupportState::ConfigurationReadOnly => {
            MetaSupportStatus::Locked
        }
        ObjectSupportState::RemovedFromSupport => MetaSupportStatus::Unsupported,
        ObjectSupportState::EditableWithSupport | ObjectSupportState::NotSupported => {
            MetaSupportStatus::Supported
        }
    })
}

pub(super) fn typed_properties(
    properties: Option<roxmltree::Node<'_, '_>>,
    kind: MetadataKind,
) -> Vec<MetaInfoPropertyData> {
    let Some(properties) = properties else {
        return Vec::new();
    };
    let mut counts = std::collections::HashMap::new();
    for node in properties.children().filter(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(super::info_projection::MD_CLASSES_NAMESPACE)
    }) {
        *counts.entry(node.tag_name().name()).or_insert(0usize) += 1;
    }
    properties
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|node| {
            node.tag_name().namespace() == Some(super::info_projection::MD_CLASSES_NAMESPACE)
        })
        .filter_map(|node| {
            if counts.get(node.tag_name().name()) != Some(&1) {
                return None;
            }
            let spec = META_INFO_PROPERTY_PROFILE.resolve(kind, node.tag_name().name())?;
            if !super::info_projection::meta_info_property_value_is_valid(node, spec.value_kind) {
                return None;
            }
            let value = match spec.value_kind {
                MetaInfoPropertyValueKind::String => {
                    let value = super::info_projection::direct_text_content(node);
                    MetaInfoPropertyValue::String(value)
                }
                MetaInfoPropertyValueKind::LegacyLocalizedString => {
                    MetaInfoPropertyValue::String(meta_info_ml_text(node))
                }
                MetaInfoPropertyValueKind::LocalizedString => MetaInfoPropertyValue::Structured(
                    super::info_projection::parsed_localized_meta_info_property_value(node),
                ),
                MetaInfoPropertyValueKind::Boolean => {
                    match super::info_projection::direct_text_content(node).as_str() {
                        "true" => MetaInfoPropertyValue::Boolean(true),
                        "false" => MetaInfoPropertyValue::Boolean(false),
                        _ => return None,
                    }
                }
                MetaInfoPropertyValueKind::UnsignedInteger => {
                    MetaInfoPropertyValue::UnsignedInteger(
                        super::info_projection::direct_text_content(node)
                            .parse()
                            .ok()?,
                    )
                }
                MetaInfoPropertyValueKind::TypedValue => MetaInfoPropertyValue::Structured(
                    super::info_projection::parsed_typed_meta_info_property_value(node)?,
                ),
            };
            Some(MetaInfoPropertyData {
                key: spec.key,
                value,
            })
        })
        .collect()
}

pub(super) fn typed_relations(
    document: &Document<'_>,
    properties: Option<roxmltree::Node<'_, '_>>,
    owner_kind: MetadataKind,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaRelationsData {
    let (owners, register_records, based_on, input_by_string, data_lock_fields) = {
        let mut read = |tag: &str, public_name: &str, kind: &str| {
            if !super::info_projection::meta_info_relation_is_applicable(owner_kind, tag) {
                return None;
            }
            let Some(properties) = properties else {
                return Some(None);
            };
            let containers = super::info_projection::direct_children_with_namespace(
                properties,
                super::info_projection::MD_CLASSES_NAMESPACE,
                tag,
            );
            let [container] = containers.as_slice() else {
                if containers.len() > 1 {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ProviderUnavailable,
                            "metadata relation container occurs more than once",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(format!("relations.{public_name}")),
                    );
                }
                return Some(None);
            };
            if container.attributes().len() != 0
                || container
                    .children()
                    .filter(roxmltree::Node::is_text)
                    .any(|node| node.text().is_some_and(|text| !text.trim().is_empty()))
            {
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata relation container is malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("relations.{public_name}")),
                );
                return Some(None);
            }
            let mut values = Vec::new();
            let mut complete = true;
            for (index, node) in container
                .children()
                .filter(|node| node.is_element())
                .enumerate()
            {
                let expected_tag = if kind == "field" { "Field" } else { "Item" };
                let base_field = format!("relations.{public_name}[{index}]");
                if node.tag_name().namespace() != Some(super::info_projection::READABLE_NAMESPACE)
                    || node.tag_name().name() != expected_tag
                    || node.children().any(|child| child.is_element())
                {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata relation entry has an unexpected structure",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                if kind == "object" {
                    let xsi_type = node
                        .attributes()
                        .find(|attribute| {
                            attribute.namespace()
                                == Some("http://www.w3.org/2001/XMLSchema-instance")
                                && attribute.name() == "type"
                        })
                        .map(|attribute| attribute.value());
                    if node.attributes().len() != 1
                        || !xsi_type.is_some_and(|value| {
                            super::info_projection::qname_resolves_to(
                                node,
                                value,
                                super::info_projection::READABLE_NAMESPACE,
                                "MDObjectRef",
                            )
                        })
                    {
                        diagnostics.push(
                            MetaDiagnostic::error(
                                MetaDiagnosticCode::ValidationFailed,
                                "metadata object relation has the wrong xsi:type",
                            )
                            .with_metadata_path(target.clone())
                            .with_field(base_field),
                        );
                        complete = false;
                        continue;
                    }
                } else if node.attributes().len() != 0 {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata field relation has unexpected attributes",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                let raw = meta_info_inner_text(node);
                let normalized = meta_info_normalize_cfg_prefix(raw.trim());
                let normalized = normalized.strip_prefix("cfg:").unwrap_or(&normalized);
                let valid = if kind == "field" {
                    crate::domain::metadata::MetadataFieldPath::parse(normalized).is_ok()
                } else {
                    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, normalized).is_ok()
                };
                if !valid {
                    diagnostics.push(
                        MetaDiagnostic::error(
                            MetaDiagnosticCode::ValidationFailed,
                            "metadata relation target is malformed",
                        )
                        .with_metadata_path(target.clone())
                        .with_field(base_field),
                    );
                    complete = false;
                    continue;
                }
                values.push(MetaRelationTargetData {
                    kind: kind.to_string(),
                    value: normalized.to_string(),
                });
            }
            Some(complete.then_some(values))
        };
        let owners = read("Owners", "owners", "object")
            .flatten()
            .unwrap_or_default();
        let register_records = read("RegisterRecords", "registerRecords", "object")
            .flatten()
            .unwrap_or_default();
        let based_on = read("BasedOn", "basedOn", "object")
            .flatten()
            .unwrap_or_default();
        let input_by_string = read("InputByString", "inputByString", "field")
            .flatten()
            .unwrap_or_default();
        let data_lock_fields = read("DataLockFields", "dataLockFields", "field");
        (
            owners,
            register_records,
            based_on,
            input_by_string,
            data_lock_fields,
        )
    };
    let source = match meta_event_subscription_source_node(document)
        .and_then(parse_meta_event_subscription_source)
    {
        Ok(source) => source,
        Err(message) if target.segments().next() == Some("EventSubscription") => {
            diagnostics.push(
                MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
                    .with_metadata_path(target.clone())
                    .with_field("relations.source"),
            );
            Vec::new()
        }
        Err(_) => Vec::new(),
    };
    MetaRelationsData {
        owners,
        register_records,
        based_on,
        input_by_string,
        data_lock_fields,
        source,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct TypedRootCollectionRoute<'a> {
    owner_kind: MetadataKind,
    tag: &'a str,
    nested_attributes: bool,
    field: &'a str,
}

impl<'a> TypedRootCollectionRoute<'a> {
    pub(super) const fn new(
        owner_kind: MetadataKind,
        tag: &'a str,
        nested_attributes: bool,
        field: &'a str,
    ) -> Self {
        Self {
            owner_kind,
            tag,
            nested_attributes,
            field,
        }
    }
}

pub(super) fn typed_root_collection(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    route: TypedRootCollectionRoute<'_>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<MetaElementData> {
    if !super::info_projection::meta_info_collection_is_applicable(route.owner_kind, route.tag) {
        return Vec::new();
    }
    typed_elements_with_diagnostics(
        xml,
        parent,
        route.tag,
        route.nested_attributes,
        route.field,
        target,
        diagnostics,
    )
}

pub(super) fn typed_optional_root_collection(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    route: TypedRootCollectionRoute<'_>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Option<Option<Vec<MetaElementData>>> {
    if !super::info_projection::meta_info_collection_is_applicable(route.owner_kind, route.tag) {
        return None;
    }
    let Some(parent) = parent else {
        return Some(None);
    };
    let expected = super::info_projection::direct_children_with_namespace(
        parent,
        super::info_projection::MD_CLASSES_NAMESPACE,
        route.tag,
    )
    .len();
    let errors_before = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .count();
    let values = typed_elements_with_diagnostics(
        xml,
        Some(parent),
        route.tag,
        route.nested_attributes,
        route.field,
        target,
        diagnostics,
    );
    let errors_after = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .count();
    if values.len() != expected && errors_after == errors_before {
        diagnostics.push(
            MetaDiagnostic::error(
                MetaDiagnosticCode::ValidationFailed,
                "metadata child collection contains an element without a proved identity",
            )
            .with_metadata_path(target.clone())
            .with_field(route.field),
        );
    }
    if errors_after != errors_before || values.len() != expected {
        Some(None)
    } else {
        Some(Some(values))
    }
}

pub(super) fn typed_elements_with_diagnostics(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
    nested_attributes: bool,
    field: &str,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<MetaElementData> {
    let Some(parent) = parent else {
        return Vec::new();
    };
    super::info_projection::direct_children_with_namespace(
        parent,
        super::info_projection::MD_CLASSES_NAMESPACE,
        tag,
    )
    .into_iter()
    .enumerate()
    .filter_map(|(index, node)| {
        typed_element(
            xml,
            node,
            nested_attributes,
            &format!("{field}[{index}]"),
            target,
            diagnostics,
        )
    })
    .collect()
}

fn typed_element(
    xml: &str,
    node: roxmltree::Node<'_, '_>,
    nested_attributes: bool,
    field: &str,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Option<MetaElementData> {
    let properties_readback =
        match super::info_projection::project_optional_collection_item_properties(node, field) {
            Ok(properties) => properties,
            Err(error) => {
                diagnostics.push(
                    MetaDiagnostic::error(error.code, error.message)
                        .with_metadata_path(target.clone())
                        .with_field(error.field),
                );
                return None;
            }
        };
    let Some(properties) = super::info_projection::direct_md_child(node, "Properties") else {
        let name = meta_info_inner_text(node).trim().to_string();
        let simple_reference = matches!(
            node.tag_name().name(),
            "Form" | "Template" | "Command" | "Recalculation"
        );
        if simple_reference && name.is_empty() {
            return None;
        }
        return Some(MetaElementData {
            name,
            incomplete: !simple_reference,
            synonym: None,
            comment: None,
            r#type: None,
            required: None,
            fill_value: None,
            addressing_dimension: None,
            properties: properties_readback,
            attributes: Vec::new(),
        });
    };
    let raw_name =
        super::info_projection::direct_md_child_text(properties, "Name").unwrap_or_default();
    let mut incomplete = raw_name.trim().is_empty();
    let name = if incomplete { String::new() } else { raw_name };
    let synonym = super::info_projection::direct_md_child(properties, "Synonym")
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty());
    let comment = super::info_projection::direct_md_child_text(properties, "Comment")
        .filter(|value| !value.is_empty());
    let r#type = if super::info_projection::direct_md_child(properties, "Type").is_some() {
        match super::info_projection::parse_observed_metadata_type_node(properties) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                incomplete = true;
                let unmodelled =
                    super::info_projection::observed_type_is_strict_but_unmodelled(properties);
                let typed = if unmodelled {
                    MetaDiagnostic::warning(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata type is syntactically valid but not modelled by this format profile",
                    )
                } else {
                    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, diagnostic.message)
                };
                diagnostics.push(
                    typed
                        .with_metadata_path(target.clone())
                        .with_field(format!("{field}.type")),
                );
                None
            }
        }
    } else {
        None
    };
    let fill_value = if super::info_projection::direct_md_child(properties, "FillValue").is_some() {
        match super::edit::parse_typed_fill_value_node(properties) {
            Ok(value) => value,
            Err(diagnostic) => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, diagnostic.message)
                        .with_metadata_path(target.clone())
                        .with_field(format!("{field}.fillValue")),
                );
                None
            }
        }
    } else {
        None
    };
    let required =
        match super::info_projection::direct_md_child_text(properties, "FillChecking").as_deref() {
            Some("ShowError") => Some(true),
            Some("DontCheck") => Some(false),
            None => None,
            Some(_) => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata required flag is malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("{field}.required")),
                );
                None
            }
        };
    let addressing_dimension = if node.tag_name().name() == "AddressingAttribute" {
        match super::info_projection::direct_md_child_text(properties, "AddressingDimension")
            .filter(|value| !value.trim().is_empty())
        {
            Some(value) if addressing_dimension_is_valid(value.trim()) => Some(value),
            _ => {
                incomplete = true;
                diagnostics.push(
                    MetaDiagnostic::error(
                        MetaDiagnosticCode::ValidationFailed,
                        "addressing attribute dimension is missing or malformed",
                    )
                    .with_metadata_path(target.clone())
                    .with_field(format!("{field}.addressingDimension")),
                );
                None
            }
        }
    } else {
        None
    };
    let attributes = if nested_attributes {
        typed_elements_with_diagnostics(
            xml,
            super::info_projection::direct_md_child(node, "ChildObjects"),
            "Attribute",
            false,
            &format!("{field}.attributes"),
            target,
            diagnostics,
        )
    } else {
        Vec::new()
    };
    Some(MetaElementData {
        name,
        incomplete,
        synonym,
        comment,
        r#type,
        required,
        fill_value,
        addressing_dimension,
        properties: properties_readback,
        attributes,
    })
}

fn addressing_dimension_is_valid(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 4
        && parts[0] == "InformationRegister"
        && parts[2] == "Dimension"
        && metadata_identifier_is_valid(parts[1])
        && metadata_identifier_is_valid(parts[3])
}

#[cfg(test)]
pub(super) fn typed_elements(
    xml: &str,
    parent: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
    nested_attributes: bool,
) -> Vec<MetaElementData> {
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Test").unwrap();
    typed_elements_with_diagnostics(
        xml,
        parent,
        tag,
        nested_attributes,
        "collections.test",
        &target,
        &mut Vec::new(),
    )
}

pub(crate) fn resolve_meta_info_path(mut object_path: PathBuf) -> Result<PathBuf, String> {
    if object_path.is_dir() {
        let dir_name = object_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let candidate = object_path.join(format!("{dir_name}.xml"));
        let sibling = object_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(format!("{dir_name}.xml"));
        if candidate.is_file() {
            object_path = candidate;
        } else if sibling.is_file() {
            object_path = sibling;
        } else {
            let mut xml_files = fs::read_dir(&object_path)
                .map_err(|err| format!("failed to read {}: {err}", object_path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("xml"))
                })
                .collect::<Vec<_>>();
            xml_files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
            if let Some(xml_file) = xml_files.into_iter().next() {
                object_path = xml_file;
            } else {
                return Err(format!(
                    "[ERROR] No XML file found in directory: {}",
                    object_path.display()
                ));
            }
        }
    }

    if !object_path.exists() {
        let file_name = object_path.file_stem().and_then(|name| name.to_str());
        let parent_dir = object_path.parent();
        let parent_dir_name = parent_dir
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str());
        if file_name == parent_dir_name {
            if let (Some(parent_dir), Some(file_name)) = (parent_dir, file_name) {
                let candidate = parent_dir
                    .parent()
                    .unwrap_or_else(|| Path::new(""))
                    .join(format!("{file_name}.xml"));
                if candidate.exists() {
                    object_path = candidate;
                }
            }
        }
    }

    if !object_path.exists() {
        return Err(format!("[ERROR] File not found: {}", object_path.display()));
    }
    Ok(object_path)
}

#[cfg(test)]
mod predefined_code_type_tests {
    use super::*;

    #[test]
    fn info_uses_the_complete_direct_code_type_text() {
        let document = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><CodeType>Num<!--keep-->ber</CodeType></Properties>"#,
        )
        .unwrap();
        assert_eq!(
            predefined_code_type_for_info(Some(document.root_element()), MetadataKind::Catalog)
                .as_deref(),
            Some("Number")
        );

        let suffixed = Document::parse(
            r#"<Properties xmlns="http://v8.1c.ru/8.3/MDClasses"><CodeType>Number<!--keep-->Suffix</CodeType></Properties>"#,
        )
        .unwrap();
        assert_eq!(
            predefined_code_type_for_info(Some(suffixed.root_element()), MetadataKind::Catalog)
                .as_deref(),
            Some("NumberSuffix")
        );
    }
}
