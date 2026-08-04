use crate::application::metadata::MetaFailure;
use crate::application::ports::{
    MetaLocalInfo, MetadataEvidenceAvailability, MetadataResourceImage, MetadataResourceRole,
    MetadataValidationSubject,
};
use crate::application::AdapterOutcome;
use crate::domain::cancellation::CancellationToken;
use crate::domain::code_intelligence::ProviderDeadline;
use crate::domain::metadata::{
    MetaCollectionsData, MetaDiagnostic, MetaDiagnosticCode, MetaDiagnosticSeverity,
    MetaElementData, MetaPropertyData, MetaPropertyValue, MetaRelationTargetData,
    MetaRelationsData, MetaSupportStatus, MetadataKind, METADATA_PROPERTY_SPECS,
};
use crate::domain::source_target::ResolvedTarget;
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use roxmltree::Document;
use serde_json::{Map, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::infrastructure::platform::secure_read::{
    capture_root_relative_regular_files, SecureTreeCaptureLimits,
};

const REGISTRAR_SCAN_MAX_ENTRIES: usize = 20_000;
const REGISTRAR_SCAN_MAX_FILES: usize = 20_000;
const REGISTRAR_SCAN_MAX_BYTES: usize = 64 * 1024 * 1024;

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

use super::super::common::{
    object_support_state, read_utf8_sig, resolve_metadata_object_descriptor, ObjectSupportData,
};
use super::edit::ResolvedMetadataObject;
use super::xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
    meta_info_ml_text, meta_info_normalize_cfg_prefix,
};

#[derive(Clone)]
pub(super) struct MetaInfoAttr<'a, 'input> {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) flags: String,
    pub(crate) _marker: std::marker::PhantomData<roxmltree::Node<'a, 'input>>,
}

pub(super) struct MetaInfoTabularSection<'a, 'input> {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttr<'a, 'input>>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
/// Typed answer of `unica.meta.info` (ADR-0023). The report translated platform
/// properties into Russian prose (`Номер: Строка(9), помесячно, авто`); the data
/// carries the platform's own property names and values instead, so the twenty
/// three metadata kinds need one shape rather than fifteen bespoke sections.
pub(crate) struct MetaInfoData {
    /// The logical address this call resolved. Flattened, because ADR-0021
    /// fixed `sourceSet` and `metadataPath` at the top level of `data` and
    /// `unica.source.locate` answers with the same shape.
    #[serde(flatten)]
    pub(crate) target: ResolvedTarget,
    /// The platform's metadata kind: `Catalog`, `Document`, `CommonModule`, …
    pub(crate) kind: String,
    pub(crate) name: String,
    /// The object's synonym; `null` when it declares none.
    pub(crate) synonym: Option<String>,
    pub(crate) support: ObjectSupportData,
    /// Scalar properties under `Properties`, by their platform names.
    pub(crate) properties: Vec<MetaInfoProperty>,
    /// Owners of a subordinate catalog; empty for everything else.
    pub(crate) owners: Vec<String>,
    pub(crate) attributes: Vec<MetaInfoAttrData>,
    /// Register dimensions; empty for every other kind.
    pub(crate) dimensions: Vec<MetaInfoAttrData>,
    /// Register resources; empty for every other kind.
    pub(crate) resources: Vec<MetaInfoAttrData>,
    pub(crate) tabular_sections: Vec<MetaInfoTabularSectionData>,
    /// Enumeration values; empty for every other kind.
    pub(crate) enum_values: Vec<String>,
    pub(crate) forms: Vec<String>,
    pub(crate) templates: Vec<String>,
    pub(crate) commands: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoProperty {
    pub(crate) name: String,
    pub(crate) value: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoAttrData {
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) type_name: Option<String>,
    /// Platform flags the report rendered inline, one entry each.
    pub(crate) flags: Vec<String>,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MetaInfoTabularSectionData {
    pub(crate) name: String,
    pub(crate) columns: Vec<MetaInfoAttrData>,
}

fn meta_info_attr_data(attrs: Vec<MetaInfoAttr<'_, '_>>) -> Vec<MetaInfoAttrData> {
    attrs
        .into_iter()
        .map(|attr| MetaInfoAttrData {
            name: attr.name,
            type_name: (!attr.type_name.is_empty()).then_some(attr.type_name),
            // `meta_info_format_flags` renders `  [обязательный, индекс]`;
            // splitting it raw left the brackets on the first and last flag.
            flags: attr
                .flags
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(str::trim)
                .filter(|flag| !flag.is_empty())
                .map(str::to_string)
                .collect(),
        })
        .collect()
}

/// Scalar `Properties` children, by their platform names. Composite children
/// (`Synonym`, `Type`, `Owners`, …) have their own typed places and are skipped
/// here so the map stays flat.
fn meta_info_properties(props: Option<roxmltree::Node<'_, '_>>) -> Vec<MetaInfoProperty> {
    let Some(props) = props else {
        return Vec::new();
    };
    props
        .children()
        .filter(|child| child.is_element())
        .filter(|child| child.children().all(|node| !node.is_element()))
        .filter_map(|child| {
            let name = child.tag_name().name().to_string();
            if matches!(name.as_str(), "Name") {
                return None;
            }
            let value = child.text().unwrap_or("").trim().to_string();
            (!value.is_empty()).then_some(MetaInfoProperty { name, value })
        })
        .collect()
}

fn meta_info_owner_names(props: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(owners_node) = props.and_then(|node| meta_info_child(node, "Owners")) else {
        return Vec::new();
    };
    meta_info_children(owners_node, "Item")
        .into_iter()
        .map(meta_info_inner_text)
        .map(|owner| owner.trim().to_string())
        .filter(|owner| !owner.is_empty())
        .collect()
}

pub(crate) struct MetaInfoExecution {
    pub(crate) outcome: AdapterOutcome,
    pub(crate) data: Option<MetaInfoData>,
}

/// Parse the descriptor image already acquired by the logical resolver. The
/// same bytes are retained in the validation subject, so typed info never
/// performs a second descriptor read between structure and validation.
pub(crate) fn read_typed_meta_info(
    resolved: &ResolvedMetadataObject,
    target: &MetadataAddress,
    deadline: ProviderDeadline,
    cancellation: &CancellationToken,
) -> Result<(MetaLocalInfo, MetadataValidationSubject), MetaFailure> {
    let text = std::str::from_utf8(&resolved.descriptor_preimage).map_err(|_| {
        MetaFailure::from(
            MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is not UTF-8",
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let xml = text.trim_start_matches('\u{feff}');
    let doc = Document::parse(xml).map_err(|_| {
        MetaFailure::from(
            MetaDiagnostic::error(
                MetaDiagnosticCode::ProviderUnavailable,
                "metadata descriptor image is not valid XML",
            )
            .with_metadata_path(target.clone()),
        )
    })?;
    let root = doc.root_element();
    if root.tag_name().name() != "MetaDataObject" {
        return Err(MetaDiagnostic::error(
            MetaDiagnosticCode::ProviderUnavailable,
            "metadata descriptor root is not MetaDataObject",
        )
        .with_metadata_path(target.clone())
        .into());
    }
    let object = root
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
        })
        .ok_or_else(|| {
            MetaFailure::from(
                MetaDiagnostic::error(
                    MetaDiagnosticCode::ProviderUnavailable,
                    "metadata descriptor has no MDClasses object",
                )
                .with_metadata_path(target.clone()),
            )
        })?;
    let kind = MetadataKind::parse(object.tag_name().name()).map_err(|diagnostic| {
        MetaFailure::from(
            MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, diagnostic.message)
                .with_metadata_path(target.clone()),
        )
    })?;
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
    let local = MetaLocalInfo {
        metadata_path: target.clone(),
        kind,
        name,
        synonym,
        support: typed_support_status(&resolved.descriptor_path),
        properties: typed_properties(properties, kind),
        relations: typed_relations(properties, target, &mut diagnostics),
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
            bytes: resolved.descriptor_preimage.clone(),
        },
        MetadataResourceImage {
            role: MetadataResourceRole::Registration,
            bytes: resolved.owner_preimage.clone(),
        },
    ];
    validation_resources.extend(typed_registered_language_images(resolved));
    let _ = super::validation::add_descriptor_references(
        &mut validation_resources,
        &resolved.descriptor_preimage,
        Some(&resolved.source_root),
    );
    let (registrar_resources, registrar_evidence) =
        typed_registrar_document_images(resolved, kind, properties, target, deadline, cancellation);
    validation_resources.extend(registrar_resources);
    let child_resources = super::edit::plan_typed_child_resources(
        &resolved.descriptor_path,
        target,
        kind.as_str(),
        &local.name,
        &[],
        xml,
    )?;
    validation_resources.extend(child_resources.validation_resources);
    let validation_subject = MetadataValidationSubject {
        target: target.clone(),
        resources: validation_resources,
        child_footprints: child_resources.validation_footprints,
        registrar_evidence,
    };
    Ok((local, validation_subject))
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

fn typed_registered_language_images(
    resolved: &ResolvedMetadataObject,
) -> Vec<MetadataResourceImage> {
    let Ok(owner) = std::str::from_utf8(&resolved.owner_preimage) else {
        return Vec::new();
    };
    let Ok(document) = Document::parse(owner.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Language")
        .filter_map(|node| node.text().map(str::trim).filter(|name| !name.is_empty()))
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
    properties: Option<roxmltree::Node<'_, '_>>,
    target: &MetadataAddress,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> MetaRelationsData {
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
    MetaRelationsData {
        owners: read("Owners", "owners", "object"),
        register_records: read("RegisterRecords", "registerRecords", "object"),
        based_on: read("BasedOn", "basedOn", "object"),
        input_by_string: read("InputByString", "inputByString", "field"),
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
                diagnostics.push(MetaDiagnostic {
                    code: MetaDiagnosticCode::ValidationFailed,
                    severity: if read_only {
                        MetaDiagnosticSeverity::Warning
                    } else {
                        MetaDiagnosticSeverity::Error
                    },
                    message: if read_only {
                        "metadata type is valid but outside the public mutation algebra".to_string()
                    } else {
                        diagnostic.message
                    },
                    metadata_path: Some(target.clone()),
                    operation_index: None,
                    field: Some(format!("{field}.type")),
                });
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

/// The resolved logical target rides in typed data rather than in the printed
/// report: ADR-0021 asks every exact operation to name the source set it
/// actually resolved, and a machine reader should not have to parse prose for
/// it.
pub(crate) fn analyze_meta_info_with_data(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> MetaInfoExecution {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let result = (|| -> Result<(MetaInfoData, PathBuf), String> {
        let (resolved, object_path) = resolve_metadata_object_descriptor(args, context)?;
        let text = read_utf8_sig(&object_path)?;
        let doc = Document::parse(text.trim_start_matches('\u{feff}'))
            .map_err(|err| format!("XML parse error in {}: {err}", object_path.display()))?;
        let root = doc.root_element();
        if root.tag_name().name() != "MetaDataObject" {
            return Err("[ERROR] Not a valid 1C metadata XML file".to_string());
        }

        let Some(type_node) = root
            .children()
            .find(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        else {
            return Err("[ERROR] Cannot detect metadata type".to_string());
        };
        let md_type = type_node.tag_name().name();
        let props = meta_info_child(type_node, "Properties");
        let child_objs = meta_info_child(type_node, "ChildObjects");
        let obj_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_default();
        let synonym = props
            .and_then(|node| meta_info_child(node, "Synonym"))
            .map(meta_info_ml_text)
            .unwrap_or_default();
        // Mode and Name sliced one object into shorter reports. Data answers
        // with the whole object once; a caller projects what it needs.
        let is_register = md_type.ends_with("Register");
        let data = MetaInfoData {
            target: resolved,
            kind: md_type.to_string(),
            name: obj_name,
            synonym: (!synonym.is_empty()).then_some(synonym),
            support: object_support_state(&object_path),
            properties: meta_info_properties(props),
            owners: meta_info_owner_names(props),
            attributes: if md_type == "Enum" {
                Vec::new()
            } else {
                meta_info_attr_data(meta_info_attributes(child_objs, "Attribute", false))
            },
            dimensions: if is_register {
                meta_info_attr_data(meta_info_attributes(child_objs, "Dimension", true))
            } else {
                Vec::new()
            },
            resources: if is_register {
                meta_info_attr_data(meta_info_attributes(child_objs, "Resource", false))
            } else {
                Vec::new()
            },
            tabular_sections: meta_info_tabular_sections(child_objs)
                .into_iter()
                .map(|section| MetaInfoTabularSectionData {
                    name: section.name,
                    columns: meta_info_attr_data(section.columns),
                })
                .collect(),
            enum_values: meta_info_enum_values(child_objs),
            forms: meta_info_simple_children(child_objs, "Form"),
            templates: meta_info_simple_children(child_objs, "Template"),
            commands: meta_info_simple_children(child_objs, "Command"),
        };
        Ok((data, object_path))
    })();

    match result {
        Ok((data, artifact)) => MetaInfoExecution {
            outcome: AdapterOutcome {
                ok: true,
                summary: format!(
                    "unica.meta.info described {} {} with {} attribute(s)",
                    data.kind,
                    data.name,
                    data.attributes.len()
                ),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: Vec::new(),
                artifacts: vec![artifact.display().to_string()],
                stdout: None,
                stderr: Some(String::new()),
                command: None,
            },
            data: Some(data),
        },
        Err(error) => MetaInfoExecution {
            outcome: AdapterOutcome {
                ok: false,
                summary: "unica.meta.info failed in native metadata analyzer".to_string(),
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: vec![error.clone()],
                artifacts: Vec::new(),
                stdout: None,
                stderr: Some(format!("{error}\n")),
                command: None,
            },
            data: None,
        },
    }
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

pub(super) fn meta_info_attributes<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
    child_tag: &str,
    is_dimension: bool,
) -> Vec<MetaInfoAttr<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, child_tag)
        .into_iter()
        .filter_map(|attr| {
            let props = meta_info_child(attr, "Properties")?;
            let name = meta_info_child_text(props, "Name").unwrap_or_default();
            let type_name = meta_info_child(props, "Type")
                .map(meta_info_format_type)
                .unwrap_or_default();
            let flags = meta_info_format_flags(props, is_dimension);
            Some(MetaInfoAttr {
                name,
                type_name,
                flags,
                _marker: std::marker::PhantomData,
            })
        })
        .collect()
}

pub(super) fn meta_info_tabular_sections<'a, 'input>(
    parent_node: Option<roxmltree::Node<'a, 'input>>,
) -> Vec<MetaInfoTabularSection<'a, 'input>> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "TabularSection")
        .into_iter()
        .map(|section| {
            let props = meta_info_child(section, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_default();
            let columns =
                meta_info_attributes(meta_info_child(section, "ChildObjects"), "Attribute", false);
            MetaInfoTabularSection { name, columns }
        })
        .collect()
}

pub(super) fn meta_info_format_type(type_node: roxmltree::Node<'_, '_>) -> String {
    let mut types = Vec::new();
    for type_item in meta_info_children(type_node, "Type") {
        types.push(meta_info_format_single_type(
            meta_info_inner_text(type_item),
            type_node,
        ));
    }
    for type_set in meta_info_children(type_node, "TypeSet") {
        let raw = meta_info_inner_text(type_set);
        if let Some(name) = raw.strip_prefix("cfg:DefinedType.") {
            types.push(format!("ОпределяемыйТип.{name}"));
        } else if let Some(name) = raw.strip_prefix("cfg:Characteristic.") {
            types.push(format!("Характеристика.{name}"));
        } else {
            types.push(raw);
        }
    }
    types.join(" | ")
}

pub(super) fn meta_info_format_single_type(
    raw: String,
    parent_node: roxmltree::Node<'_, '_>,
) -> String {
    match raw.as_str() {
        "xs:string" => {
            let length = meta_info_child(parent_node, "StringQualifiers")
                .and_then(|node| meta_info_child_text(node, "Length"))
                .unwrap_or_default();
            if length.is_empty() {
                "Строка".to_string()
            } else {
                format!("Строка({length})")
            }
        }
        "xs:decimal" => {
            let qualifiers = meta_info_child(parent_node, "NumberQualifiers");
            let digits = qualifiers
                .and_then(|node| meta_info_child_text(node, "Digits"))
                .unwrap_or_default();
            let fraction = qualifiers
                .and_then(|node| meta_info_child_text(node, "FractionDigits"))
                .unwrap_or_else(|| "0".to_string());
            if digits.is_empty() {
                "Число".to_string()
            } else {
                format!("Число({digits},{fraction})")
            }
        }
        "xs:boolean" => "Булево".to_string(),
        "xs:dateTime" => {
            let date_fraction = meta_info_child(parent_node, "DateQualifiers")
                .and_then(|node| meta_info_child_text(node, "DateFractions"));
            match date_fraction.as_deref() {
                Some("Date") => "Дата".to_string(),
                Some("Time") => "Время".to_string(),
                Some("DateTime") => "ДатаВремя".to_string(),
                Some(_) => "Дата".to_string(),
                None => "ДатаВремя".to_string(),
            }
        }
        "v8:ValueStorage" => "ХранилищеЗначения".to_string(),
        "v8:UUID" => "УникальныйИдентификатор".to_string(),
        "v8:Null" => "Null".to_string(),
        _ => meta_info_format_cfg_type(&raw),
    }
}

pub(super) fn meta_info_format_cfg_type(raw: &str) -> String {
    let normalized = meta_info_normalize_cfg_prefix(raw);
    if let Some(rest) = normalized.strip_prefix("cfg:") {
        if let Some((prefix, name)) = rest.split_once('.') {
            if let Some(ref_type) = meta_info_ref_type_ru(prefix) {
                return format!("{ref_type}.{name}");
            }
            if prefix == "Characteristic" {
                return format!("Характеристика.{name}");
            }
            if prefix == "DefinedType" {
                return format!("ОпределяемыйТип.{name}");
            }
        }
        return rest.to_string();
    }
    normalized
}

pub(super) fn meta_info_format_flags(props: roxmltree::Node<'_, '_>, is_dimension: bool) -> String {
    let mut flags = Vec::new();
    if meta_info_child_text(props, "FillChecking").as_deref() == Some("ShowError") {
        flags.push("обязательный");
    }
    if let Some(indexing) = meta_info_child_text(props, "Indexing") {
        match indexing.as_str() {
            "Index" => flags.push("индекс"),
            "IndexWithAdditionalOrder" => flags.push("индекс+доп"),
            _ => {}
        }
    }
    if is_dimension && meta_info_child_text(props, "Master").as_deref() == Some("true") {
        flags.push("ведущее");
    }
    if meta_info_child_text(props, "MultiLine").as_deref() == Some("true") {
        flags.push("многострочный");
    }
    if let Some(use_value) = meta_info_child_text(props, "Use") {
        match use_value.as_str() {
            "ForFolder" => flags.push("для папок"),
            "ForFolderAndItem" => flags.push("для папок и элементов"),
            _ => {}
        }
    }
    if flags.is_empty() {
        String::new()
    } else {
        format!("  [{}]", flags.join(", "))
    }
}

pub(super) fn meta_info_simple_children(
    parent_node: Option<roxmltree::Node<'_, '_>>,
    tag: &str,
) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, tag)
        .into_iter()
        .map(meta_info_inner_text)
        .collect()
}

pub(super) fn meta_info_enum_values(parent_node: Option<roxmltree::Node<'_, '_>>) -> Vec<String> {
    let Some(parent_node) = parent_node else {
        return Vec::new();
    };
    meta_info_children(parent_node, "EnumValue")
        .into_iter()
        .filter_map(|value| {
            meta_info_child(value, "Properties")
                .and_then(|props| meta_info_child_text(props, "Name"))
        })
        .collect()
}

pub(super) fn meta_info_ref_type_ru(prefix: &str) -> Option<&'static str> {
    match prefix {
        "CatalogRef" => Some("СправочникСсылка"),
        "DocumentRef" => Some("ДокументСсылка"),
        "EnumRef" => Some("ПеречислениеСсылка"),
        "ChartOfAccountsRef" => Some("ПланСчетовСсылка"),
        "ChartOfCharacteristicTypesRef" => Some("ПВХСсылка"),
        "ChartOfCalculationTypesRef" => Some("ПВРСсылка"),
        "ExchangePlanRef" => Some("ПланОбменаСсылка"),
        "BusinessProcessRef" => Some("БизнесПроцессСсылка"),
        "TaskRef" => Some("ЗадачаСсылка"),
        _ => None,
    }
}

pub(super) struct MetaRemoveError {
    pub(crate) stderr: String,
    pub(crate) message: String,
}
