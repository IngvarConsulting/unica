//! Existing metadata info enrichment and evidence algorithms over issued observations.
use crate::relations::MetadataDependencyRead;
use crate::resource_plan::ResolvedMetadataObject;
use crate::validation::{metadata_address, metadata_collection};
use roxmltree::Document;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use unica_address::source_target::{
    MetadataAddress, ResolvedTarget, SourceTarget, TargetKind, PLATFORM_XML_8_3_27_FORMAT_2_20,
};
use unica_metadata::subsystem::SUBSYSTEM_ADDRESS_MAX_DEPTH;
use unica_metadata::subsystem_topology::MetadataObjectIdentity;
use unica_metadata::support_state::{ObjectSupportState, SupportStateReader};
use unica_metadata::{
    MetaDiagnostic, MetaDiagnosticCode, MetaEventSource, MetaSupportStatus, MetadataKind,
};
use unica_source_contract::discovery::ProjectDiscoveryRead;
use unica_source_contract::read::{
    RetainedSourceRead, SecureFileSnapshot, SecureTreeCaptureLimits,
};
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
/// Bounded IO factories and the existing observation hooks; never semantic callbacks.
pub trait MetadataInfoRead: MetadataDependencyRead {
    fn capture_regular_files(
        &self,
        root: &Path,
        start: &Path,
        limits: SecureTreeCaptureLimits,
        descend: fn(&Path) -> bool,
        select: fn(&Path) -> bool,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<SecureFileSnapshot>;
    fn open_retained_read(
        &self,
        root: &Path,
        limits: SecureTreeCaptureLimits,
        checkpoint: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Box<dyn RetainedSourceRead>>;
    fn registrar_processing_phase(&self, phase: RegistrarProcessingPhase);
    fn subsystem_processing_phase(&self, phase: SubsystemEvidenceProcessingPhase);
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrarProcessingPhase {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsystemEvidenceProcessingPhase {
    BeforeCompleteReturn,
}

const REGISTRAR_SCAN_MAX_ENTRIES: usize = 20_000;

const REGISTRAR_SCAN_MAX_FILES: usize = 20_000;

const REGISTRAR_SCAN_MAX_BYTES: usize = 64 * 1024 * 1024;

/// The source set the descriptor belongs to: its name for dependency reads
/// and its kind for the member property profile.
pub struct MetadataInfoSourceSet<'a> {
    pub name: &'a str,
    pub kind: unica_source_contract::SourceSetKind,
}

/// The exact controls supplied for this read, without additional policy/checkpoints.
pub struct MetadataInfoControl<'a> {
    pub deadline: ProviderDeadline,
    pub cancellation: &'a CancellationToken,
}

/// Parse the descriptor image already acquired by the logical resolver. The
/// same bytes are retained in the validation subject, so typed info never
/// performs a second descriptor read between structure and validation.
pub fn read_typed_meta_info<G>(
    reader: &dyn MetadataInfoRead<Guard = G>,
    resolved: &ResolvedMetadataObject<G>,
    descriptor_preimage: &[u8],
    source_set: MetadataInfoSourceSet<'_>,
    target: &MetadataAddress,
    control: MetadataInfoControl<'_>,
    support_reader: &dyn SupportStateReader,
) -> Result<
    (
        unica_metadata::validation_subject::MetaLocalInfo,
        unica_metadata::validation_subject::MetadataValidationSubject,
    ),
    unica_metadata::request::MetaFailure,
> {
    let MetadataInfoControl {
        deadline,
        cancellation,
    } = control;

    // The local typed projection is shared with capability-bound readers. V12
    // computes support from its resolved target after the descriptor has been
    // proved, preserving the previous error precedence for malformed images.
    // The source set kind selects the member property profile exactly as the
    // canonical view does; without it `check` missed damaged member
    // properties (#1941).
    let mut local = crate::info::parse_typed_meta_local_info_for_source(
        descriptor_preimage,
        target,
        MetaSupportStatus::Supported,
        source_set.kind,
    )?;
    local.support = typed_support_status(support_reader, &resolved.resolved_target)?;

    let identity = crate::validation_context::inspect_metadata_image_identity(descriptor_preimage)
        .expect("the shared local parser proved the exact descriptor image");
    let text =
        std::str::from_utf8(descriptor_preimage).expect("the shared local parser proved UTF-8");
    let xml = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(xml).expect("the shared local parser proved the XML document");
    let object = doc
        .root_element()
        .children()
        .find(|node| node.is_element())
        .expect("the shared local parser proved one object");
    let kind = local.kind;
    let object_identity = MetadataObjectIdentity {
        address: target.clone(),
        uuid: identity.object_uuid,
    };
    let properties = object
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(crate::info_projection::MD_CLASSES_NAMESPACE)
                && node.tag_name().name() == "Properties"
        })
        .expect("the shared local parser proved Properties");
    let properties = Some(properties);
    let mut validation_resources = vec![
        unica_metadata::validation_subject::MetadataResourceImage {
            role: unica_metadata::validation_subject::MetadataResourceRole::Descriptor,
            bytes: descriptor_preimage.to_vec(),
        },
        unica_metadata::validation_subject::MetadataResourceImage {
            role: unica_metadata::validation_subject::MetadataResourceRole::Registration,
            bytes: resolved.owner_preimage.clone(),
        },
    ];
    validation_resources.extend(typed_registered_language_images(
        reader.ordinary_read(),
        resolved,
    ));
    let _ = add_descriptor_references(
        reader.ordinary_read(),
        &mut validation_resources,
        descriptor_preimage,
        Some(&resolved.source_root),
    );
    let (event_source_resources, event_source_diagnostics) = typed_event_source_dependency_images(
        reader,
        resolved,
        source_set.name,
        &local.relations.source,
        deadline,
        cancellation,
    );
    validation_resources.extend(event_source_resources);
    local.diagnostics.extend(event_source_diagnostics);
    let (registrar_resources, registrar_evidence) = typed_registrar_document_images(
        reader,
        &resolved.source_root,
        kind,
        properties,
        target,
        deadline,
        cancellation,
    );
    validation_resources.extend(registrar_resources);
    let subsystem_evidence = Some(typed_subsystem_evidence(
        reader,
        resolved,
        &object_identity,
        deadline,
        cancellation,
    ));
    let child_resources = match crate::child_resources::observe_typed_child_resources(
        reader.ordinary_read(),
        &resolved.descriptor_path,
        target,
        kind.as_str(),
        &local.name,
        xml,
    ) {
        Ok(resources) => resources,
        Err(failure) => {
            local.diagnostics.extend(failure.diagnostics);
            crate::resource_plan::TypedChildObservation::default()
        }
    };
    local.diagnostics.extend(child_resources.diagnostics);
    validation_resources.extend(child_resources.validation_resources);
    let validation_subject = unica_metadata::validation_subject::MetadataValidationSubject {
        target: target.clone(),
        resources: validation_resources,
        child_footprints: child_resources.validation_footprints,
        registrar_evidence,
        subsystem_evidence,
    };
    Ok((local, validation_subject))
}

fn typed_event_source_dependency_images<G>(
    reader: &dyn MetadataInfoRead<Guard = G>,
    resolved: &ResolvedMetadataObject<G>,
    source_set: &str,
    sources: &[MetaEventSource],
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> (
    Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    Vec<MetaDiagnostic>,
) {
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
        let Ok(resolution) = reader.resolve_object(&source_target) else {
            continue;
        };
        if resolution.0.target_kind != TargetKind::MetadataObject {
            continue;
        }
        let Ok(evidence) = reader.resource_evidence(&resolution.1) else {
            continue;
        };
        if evidence.source_root != resolved.source_root
            || evidence.registration_path != resolved.owner_path
        {
            continue;
        }
        let Ok(bytes) =
            unica_source_designer::read::read_bytes(reader.ordinary_read(), &evidence.target_path)
        else {
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
            if let Ok(members) = crate::xml_model::parse_defined_type_event_sources(&bytes) {
                queue.extend(members);
            }
        }
        resources.push(unica_metadata::validation_subject::MetadataResourceImage {
            role: unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                target: metadata_path,
            },
            bytes,
        });
    }
    (resources, diagnostics)
}

pub fn typed_registrar_document_images<G>(
    reader: &dyn MetadataInfoRead<Guard = G>,
    source_root: &Path,
    kind: MetadataKind,
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> (
    Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    unica_metadata::validation_subject::MetadataEvidenceAvailability,
) {
    let reads_registrars = if kind == MetadataKind::InformationRegister {
        match properties
            .map(|properties| {
                crate::info_projection::observe_meta_info_property(properties, kind, "WriteMode")
            })
            .transpose()
        {
            Ok(Some(Some(unica_metadata::MetaInfoPropertyValue::String(mode)))) => {
                mode.trim() == "RecorderSubordinate"
            }
            Err(error) => return registrar_evidence_unavailable(target, &error.message),
            _ => {
                return registrar_evidence_unavailable(
                    target,
                    "registrar applicability is unknown because WriteMode is absent",
                )
            }
        }
    } else {
        matches!(
            kind,
            MetadataKind::AccumulationRegister
                | MetadataKind::AccountingRegister
                | MetadataKind::CalculationRegister
        )
    };
    if !reads_registrars {
        return (
            Vec::new(),
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete,
        );
    }
    let mut checkpoint = || registrar_scan_checkpoint(deadline, cancellation);
    let entries = match reader.capture_regular_files(
        source_root,
        Path::new("Documents"),
        SecureTreeCaptureLimits {
            maximum_depth: 0,
            maximum_entries: REGISTRAR_SCAN_MAX_ENTRIES,
            maximum_files: REGISTRAR_SCAN_MAX_FILES,
            maximum_bytes: REGISTRAR_SCAN_MAX_BYTES,
        },
        |_| false,
        |path| path.extension().and_then(|extension| extension.to_str()) == Some("xml"),
        &mut checkpoint,
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
        reader.registrar_processing_phase(RegistrarProcessingPhase::BeforeCompleteReturn);
        if checkpoint().is_err() {
            return registrar_evidence_unavailable(
                target,
                "registrar evidence processing was interrupted",
            );
        }
        return (
            Vec::new(),
            unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete,
        );
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
        let identity_result = crate::validation_context::inspect_metadata_image_identity(&bytes);
        reader.registrar_processing_phase(RegistrarProcessingPhase::AfterIdentityParse {
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
        let registers_target = crate::validation::document_registers(&bytes, &register_reference);
        reader.registrar_processing_phase(RegistrarProcessingPhase::AfterRegistrarParse {
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
            resources.push(unica_metadata::validation_subject::MetadataResourceImage {
                role: unica_metadata::validation_subject::MetadataResourceRole::Dependency {
                    target: dependency_target,
                },
                bytes,
            });
        }
    }
    reader.registrar_processing_phase(RegistrarProcessingPhase::BeforeCompleteReturn);
    if checkpoint().is_err() {
        return registrar_evidence_unavailable(
            target,
            "registrar evidence processing was interrupted",
        );
    }
    (
        resources,
        unica_metadata::validation_subject::MetadataEvidenceAvailability::Complete,
    )
}

pub fn registrar_scan_checkpoint(
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> io::Result<()> {
    if cancellation.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "registrar evidence scan was cancelled",
        ))
    } else if deadline.is_elapsed() {
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
) -> (
    Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    unica_metadata::validation_subject::MetadataEvidenceAvailability,
) {
    (
        Vec::new(),
        unica_metadata::validation_subject::MetadataEvidenceAvailability::Unavailable(vec![
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
                .with_metadata_path(target.clone())
                .with_field("registrarEvidence"),
        ]),
    )
}

fn subsystem_evidence_unavailable(
    target: &MetadataAddress,
    message: &str,
) -> unica_metadata::validation_subject::MetadataSubsystemEvidence {
    unica_metadata::validation_subject::MetadataSubsystemEvidence::Unavailable(vec![
        MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
            .with_metadata_path(target.clone())
            .with_field("subsystemEvidence"),
    ])
}

/// Collects all registered subsystem memberships of the current object. The
/// same complete evidence is serialized by `meta.info` and consumed by the
/// command-interface rule when the object is an eligible register.
fn typed_subsystem_evidence<G>(
    reader: &dyn MetadataInfoRead<Guard = G>,
    resolved: &ResolvedMetadataObject<G>,
    identity: &MetadataObjectIdentity,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> unica_metadata::validation_subject::MetadataSubsystemEvidence {
    if resolved
        .owner_path
        .file_name()
        .and_then(|name| name.to_str())
        != Some("Configuration.xml")
    {
        reader.subsystem_processing_phase(SubsystemEvidenceProcessingPhase::BeforeCompleteReturn);
        if registrar_scan_checkpoint(deadline, cancellation).is_err() {
            return subsystem_evidence_unavailable(
                &identity.address,
                "registered subsystem topology processing was interrupted",
            );
        }
        return unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
            functional_subsystems: Vec::new(),
            interface_subsystems: Vec::new(),
        };
    }
    let mut checkpoint = || registrar_scan_checkpoint(deadline, cancellation);
    let topology = match reader
        .open_retained_read(
            &resolved.source_root,
            SecureTreeCaptureLimits {
                maximum_depth: SUBSYSTEM_ADDRESS_MAX_DEPTH * 2,
                maximum_entries: crate::subsystem_topology::SUBSYSTEM_SCAN_MAX_ENTRIES,
                maximum_files: crate::subsystem_topology::SUBSYSTEM_SCAN_MAX_FILES,
                maximum_bytes: crate::subsystem_topology::SUBSYSTEM_SCAN_MAX_BYTES,
            },
            &mut checkpoint,
        )
        .map_err(crate::subsystem_topology::SubsystemTopologyError::from)
        .and_then(|retained| {
            crate::subsystem_topology::read_registered_subsystem_topology(retained, &mut checkpoint)
        }) {
        Ok(topology) => topology,
        Err(_) => {
            return subsystem_evidence_unavailable(
                &identity.address,
                "registered subsystem topology cannot be proved completely",
            )
        }
    };
    reader.subsystem_processing_phase(SubsystemEvidenceProcessingPhase::BeforeCompleteReturn);
    if registrar_scan_checkpoint(deadline, cancellation).is_err() {
        return subsystem_evidence_unavailable(
            &identity.address,
            "registered subsystem topology processing was interrupted",
        );
    }
    unica_metadata::validation_subject::MetadataSubsystemEvidence::Complete {
        functional_subsystems: topology.functional_memberships_for(identity),
        interface_subsystems: topology.interface_memberships_for(identity),
    }
}

fn typed_registered_language_images<G>(
    reader: &dyn ProjectDiscoveryRead,
    resolved: &ResolvedMetadataObject<G>,
) -> Vec<unica_metadata::validation_subject::MetadataResourceImage> {
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
        .map(crate::info_projection::direct_text_content)
        .filter_map(|name| (!name.trim().is_empty()).then_some(name))
        .filter(|name| unica_metadata::identifier::is_1c_identifier(name))
        .filter_map(|name| {
            let metadata_path = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                &format!("Language.{name}"),
            )
            .ok()?;
            let bytes = unica_source_designer::read::read_bytes(
                reader,
                &resolved
                    .source_root
                    .join("Languages")
                    .join(format!("{name}.xml")),
            )
            .ok()?;
            Some(unica_metadata::validation_subject::MetadataResourceImage {
                role: unica_metadata::validation_subject::MetadataResourceRole::Dependency {
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
) -> Result<MetaSupportStatus, unica_metadata::request::MetaFailure> {
    let support = support_reader.object_support(target).map_err(|error| {
        let mut diagnostic =
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, error.to_string());
        if let Some(metadata_path) = target.metadata_path.clone() {
            diagnostic = diagnostic.with_metadata_path(metadata_path);
        }
        unica_metadata::request::MetaFailure::from(diagnostic)
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

pub fn resolve_meta_info_path(
    reader: &dyn ProjectDiscoveryRead,
    mut object_path: PathBuf,
) -> Result<PathBuf, String> {
    if unica_source_designer::read::path_is_dir(reader, &object_path) {
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
        if reader.path_is_file(&candidate) {
            object_path = candidate;
        } else if reader.path_is_file(&sibling) {
            object_path = sibling;
        } else {
            let mut xml_files = reader
                .read_entries(&object_path)
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

    if !unica_source_designer::read::path_exists(reader, &object_path) {
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
                if unica_source_designer::read::path_exists(reader, &candidate) {
                    object_path = candidate;
                }
            }
        }
    }

    if !unica_source_designer::read::path_exists(reader, &object_path) {
        return Err(format!("[ERROR] File not found: {}", object_path.display()));
    }
    Ok(object_path)
}

fn push_dependency_from_path(
    reader: &dyn ProjectDiscoveryRead,
    resources: &mut Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    target: MetadataAddress,
    path: &Path,
) -> Result<(), String> {
    if resources.iter().any(|resource| {
        matches!(
            &resource.role,
            unica_metadata::validation_subject::MetadataResourceRole::Dependency { target: existing } if existing == &target
        )
    }) {
        return Ok(());
    }
    if reader.path_is_file(path) {
        resources.push(unica_metadata::validation_subject::MetadataResourceImage {
            role: unica_metadata::validation_subject::MetadataResourceRole::Dependency { target },
            bytes: unica_source_designer::read::read_bytes(reader, path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
        });
    }
    Ok(())
}

pub fn add_descriptor_references(
    reader: &dyn ProjectDiscoveryRead,
    resources: &mut Vec<unica_metadata::validation_subject::MetadataResourceImage>,
    descriptor: &[u8],
    config_dir: Option<&Path>,
) -> Result<(), String> {
    let Some(config_dir) = config_dir else {
        return Ok(());
    };
    let (_, document) = unica_xml_format::read::parse_metadata_image(descriptor)?;
    let Some(object) = document.root_element().children().find(|node| {
        node.is_element() && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
    }) else {
        return Ok(());
    };
    let object_type = object.tag_name().name();
    let properties = unica_xml_format::read::child_element(object, "Properties");

    if object_type == "Document" {
        for reference in properties
            .and_then(|properties| {
                unica_xml_format::read::child_element(properties, "RegisterRecords")
            })
            .map(|records| unica_xml_format::read::child_elements(records, "Item"))
            .unwrap_or_default()
            .into_iter()
            .map(unica_xml_format::read::direct_text)
        {
            let Some((kind, name)) = reference.split_once('.') else {
                continue;
            };
            let Some(collection) = metadata_collection(kind) else {
                continue;
            };
            let target = metadata_address(&reference)?;
            let path = config_dir.join(collection).join(format!("{name}.xml"));
            push_dependency_from_path(reader, resources, target, &path)?;
        }
    }

    if matches!(object_type, "EventSubscription" | "ScheduledJob") {
        let property = if object_type == "EventSubscription" {
            "Handler"
        } else {
            "MethodName"
        };
        if let Some(reference) = properties
            .and_then(|properties| unica_xml_format::read::child_direct_text(properties, property))
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
                let owner = metadata_address(format!("CommonModule.{module_name}"))?;
                let descriptor_path = config_dir
                    .join("CommonModules")
                    .join(format!("{module_name}.xml"));
                push_dependency_from_path(reader, resources, owner.clone(), &descriptor_path)?;
                let module_path = config_dir
                    .join("CommonModules")
                    .join(module_name)
                    .join("Ext")
                    .join("Module.bsl");
                if reader.path_is_file(&module_path) {
                    resources.push(unica_metadata::validation_subject::MetadataResourceImage {
                        role: unica_metadata::validation_subject::MetadataResourceRole::Module {
                            owner,
                        },
                        bytes: unica_source_designer::read::read_bytes(reader, &module_path)
                            .map_err(|error| {
                                format!("failed to read {}: {error}", module_path.display())
                            })?,
                    });
                }
            }
        }
    }
    Ok(())
}

pub fn registered_language_images(
    reader: &dyn ProjectDiscoveryRead,
    source_root: &Path,
    owner_preimage: &[u8],
) -> Result<
    Vec<(
        PathBuf,
        unica_address::source_target::MetadataAddress,
        Vec<u8>,
    )>,
    String,
> {
    let xml = std::str::from_utf8(owner_preimage)
        .map_err(|_| "metadata owner image is not UTF-8".to_string())?
        .trim_start_matches('\u{feff}');
    let document =
        Document::parse(xml).map_err(|_| "metadata owner image is not valid XML".to_string())?;
    let configuration = document.root_element().children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(crate::info_projection::MD_CLASSES_NAMESPACE)
            && node.tag_name().name() == "Configuration"
    });
    let child_objects = configuration.and_then(|configuration| {
        configuration.children().find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(crate::info_projection::MD_CLASSES_NAMESPACE)
                && node.tag_name().name() == "ChildObjects"
        })
    });
    let mut images = Vec::new();
    for node in child_objects
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(crate::info_projection::MD_CLASSES_NAMESPACE)
                && node.tag_name().name() == "Language"
        })
    {
        let Some(name) = node.text().map(str::trim).filter(|name| !name.is_empty()) else {
            continue;
        };
        if !unica_metadata::identifier::is_1c_identifier(name) {
            return Err(format!(
                "registered language `{name}` is not a valid 1C identifier"
            ));
        }
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            &format!("Language.{name}"),
        )
        .map_err(|_| "registered language has an invalid logical identity".to_string())?;
        let path = source_root.join("Languages").join(format!("{name}.xml"));
        let bytes = unica_source_designer::read::read_bytes(reader, &path)
            .map_err(|_| format!("registered language `{name}` image is unavailable"))?;
        images.push((path, target, bytes));
    }
    Ok(images)
}
