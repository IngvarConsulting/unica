use crate::application::metadata::MetaFailure;
use crate::application::ports::{
    MetaLocalInfo, MetadataEvidenceAvailability, MetadataResourceImage, MetadataResourceRole,
    MetadataSubsystemEvidence, MetadataValidationSubject,
};
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::{
    metadata_identifier_is_valid, MetaCollectionsData, MetaDiagnostic, MetaDiagnosticCode,
    MetaElementData, MetaEventSource, MetaPropertyData, MetaPropertyValue, MetaRelationTargetData,
    MetaRelationsData, MetaSupportStatus, MetadataKind, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::{
    MetadataAddress, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_source_targets::{
    platform_xml_resource_evidence, resolve_platform_xml_target, TargetKindPolicy,
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

use super::super::common::object_support_state;
use super::edit::ResolvedMetadataObject;
use super::xml_model::{
    meta_event_subscription_source_node, meta_info_child, meta_info_child_text, meta_info_children,
    meta_info_inner_text, meta_info_ml_text, meta_info_normalize_cfg_prefix,
    parse_defined_type_event_sources, parse_meta_event_subscription_source,
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
    let kind = MetadataKind::parse(&identity.object_type).map_err(|diagnostic| {
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
    let properties = meta_info_child(object, "Properties");
    let child_objects = meta_info_child(object, "ChildObjects");
    let name = properties
        .and_then(|node| meta_info_child_text(node, "Name"))
        .unwrap_or_default();
    if name.is_empty() {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor has no object name",
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let synonym = properties
        .and_then(|node| meta_info_child(node, "Synonym"))
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty());

    let mut diagnostics = Vec::new();
    let mut local = MetaLocalInfo {
        metadata_path: target.clone(),
        kind,
        name,
        synonym,
        support: typed_support_status(&resolved.descriptor_path),
        properties: typed_properties(properties, kind),
        relations: typed_relations(&doc, properties, target, &mut diagnostics),
        collections: MetaCollectionsData {
            attributes: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Attribute",
                false,
                "collections.attributes",
                target,
                &mut diagnostics,
            ),
            tabular_sections: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "TabularSection",
                true,
                "collections.tabularSections",
                target,
                &mut diagnostics,
            ),
            dimensions: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Dimension",
                false,
                "collections.dimensions",
                target,
                &mut diagnostics,
            ),
            resources: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Resource",
                false,
                "collections.resources",
                target,
                &mut diagnostics,
            ),
            enum_values: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "EnumValue",
                false,
                "collections.enumValues",
                target,
                &mut diagnostics,
            ),
            columns: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Column",
                false,
                "collections.columns",
                target,
                &mut diagnostics,
            ),
            forms: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Form",
                false,
                "collections.forms",
                target,
                &mut diagnostics,
            ),
            templates: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Template",
                false,
                "collections.templates",
                target,
                &mut diagnostics,
            ),
            commands: typed_elements_with_diagnostics(
                xml,
                child_objects,
                "Command",
                false,
                "collections.commands",
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
    let child_resources = match super::edit::plan_typed_child_resources(
        &resolved.descriptor_path,
        target,
        kind.as_str(),
        &local.name,
        &[],
        xml,
    ) {
        Ok(resources) => resources,
        Err(failure) => {
            local.diagnostics.extend(failure.diagnostics);
            super::edit::TypedChildResourcePlan::default()
        }
    };
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
            resolve_platform_xml_target(context, &source_target, TargetKindPolicy::Any)
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
        .filter_map(|node| node.text().map(str::trim).filter(|name| !name.is_empty()))
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

fn typed_support_status(path: &Path) -> MetaSupportStatus {
    match object_support_state(path).state {
        "locked" | "configurationReadOnly" => MetaSupportStatus::Locked,
        "removedFromSupport" => MetaSupportStatus::Unsupported,
        _ => MetaSupportStatus::Supported,
    }
}

pub(super) fn typed_properties(
    properties: Option<roxmltree::Node<'_, '_>>,
    kind: MetadataKind,
) -> Vec<MetaPropertyData> {
    let Some(properties) = properties else {
        return Vec::new();
    };
    METADATA_PROPERTY_SPECS
        .iter()
        .filter(|spec| spec.allowed_kinds.contains(&kind))
        .filter_map(|spec| {
            let node = meta_info_child(properties, spec.xml_name)?;
            let value = match spec.value_kind {
                crate::domain::metadata::MetaPropertyValueKind::String => {
                    let value = if spec.key == crate::domain::metadata::MetaPropertyKey::Synonym {
                        meta_info_ml_text(node)
                    } else {
                        node.text().unwrap_or_default().to_string()
                    };
                    MetaPropertyValue::String(value)
                }
                crate::domain::metadata::MetaPropertyValueKind::Boolean => match node.text()? {
                    "true" => MetaPropertyValue::Boolean(true),
                    "false" => MetaPropertyValue::Boolean(false),
                    _ => return None,
                },
                crate::domain::metadata::MetaPropertyValueKind::UnsignedInteger => {
                    MetaPropertyValue::UnsignedInteger(node.text()?.parse().ok()?)
                }
            };
            Some(MetaPropertyData {
                key: spec.key,
                value,
            })
        })
        .collect()
}

pub(super) fn typed_relations(
    document: &Document<'_>,
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaRelationsData {
    let (owners, register_records, based_on, input_by_string) = {
        let mut read = |tag: &str, public_name: &str, kind: &str| {
            let Some(container) = properties.and_then(|node| meta_info_child(node, tag)) else {
                return Vec::new();
            };
            container
                .children()
                .filter(|node| node.is_element())
                .enumerate()
                .filter_map(|(index, node)| {
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
                            .with_field(format!("relations.{public_name}[{index}]")),
                        );
                        return None;
                    }
                    Some(MetaRelationTargetData {
                        kind: kind.to_string(),
                        value: normalized.to_string(),
                    })
                })
                .collect()
        };
        (
            read("Owners", "owners", "object"),
            read("RegisterRecords", "registerRecords", "object"),
            read("BasedOn", "basedOn", "object"),
            read("InputByString", "inputByString", "field"),
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
        source,
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
    meta_info_children(parent, tag)
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
    let Some(properties) = meta_info_child(node, "Properties") else {
        let name = meta_info_inner_text(node).trim().to_string();
        let simple_reference = matches!(node.tag_name().name(), "Form" | "Template" | "Command");
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
            attributes: Vec::new(),
        });
    };
    let raw_name = meta_info_child_text(properties, "Name").unwrap_or_default();
    let mut incomplete = raw_name.trim().is_empty();
    let name = if incomplete { String::new() } else { raw_name };
    let synonym = meta_info_child(properties, "Synonym")
        .map(meta_info_ml_text)
        .filter(|value| !value.is_empty());
    let comment = meta_info_child_text(properties, "Comment").filter(|value| !value.is_empty());
    let properties_text = &xml[properties.range()];
    let r#type = if meta_info_child(properties, "Type").is_some() {
        match super::edit::parse_typed_metadata_type(properties_text) {
            Ok(value) => Some(value),
            Err(diagnostic) => {
                incomplete = true;
                let read_only = typed_read_only_platform_type(properties_text);
                let typed = if read_only {
                    MetaDiagnostic::warning(
                        MetaDiagnosticCode::ValidationFailed,
                        "metadata type is valid but outside the public mutation algebra",
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
    let fill_value = if meta_info_child(properties, "FillValue").is_some() {
        match super::edit::parse_typed_fill_value(properties_text) {
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
    let required = match meta_info_child_text(properties, "FillChecking").as_deref() {
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
    let attributes = if nested_attributes {
        typed_elements_with_diagnostics(
            xml,
            meta_info_child(node, "ChildObjects"),
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
        attributes,
    })
}

fn typed_read_only_platform_type(properties_text: &str) -> bool {
    const WRAPPER_START: &str = r#"<Root xmlns:v8="http://v8.1c.ru/8.1/data/core">"#;
    let wrapped = format!("{WRAPPER_START}{properties_text}</Root>");
    let Ok(document) = Document::parse(&wrapped) else {
        return false;
    };
    document.descendants().any(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
            && node.tag_name().name() == "Type"
            && matches!(node.text(), Some("v8:UUID"))
    })
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
