#![allow(dead_code, unused_imports)]

use crate::application::ports::{
    MetadataAuxiliaryXmlKind, MetadataChildDirectoryKind, MetadataChildProfile,
    MetadataChildResourceKind, MetadataResourceRole, MetadataTemplateResourcePart,
    MetadataTemplateType, MetadataValidationResult, MetadataValidationSubject,
};
use crate::application::AdapterOutcome;
use crate::domain::format_profile::{
    classify_root_version, FormatCompatibility, ACTIVE_FORMAT_PROFILE,
};
use crate::domain::metadata::{
    metadata_kind_collections, MetaCollection, MetaDiagnostic, MetaDiagnosticCode,
    MetaDiagnosticSeverity, MetaValidationData, MetaValidationStatus, MetadataKind,
};
use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
use crate::domain::workspace::WorkspaceContext;
use crate::infrastructure::platform_xml_owner::{
    root_version_literal, PlatformXmlRootExpectation, DCS_ROOT, MXL_ROOT,
};
use roxmltree::Document;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use super::super::common::{
    absolutize, bool_arg, child_text, format_compatibility_warning, int_arg, is_1c_identifier,
    read_utf8_sig, required_path,
};
use super::info::resolve_meta_info_path;
use super::validation_context::{
    inspect_meta_validation_reads, inspect_meta_validation_subject_reads,
    inspect_metadata_image_identity, inspect_metadata_language_image,
    inspect_metadata_registration_image, meta_validate_registrar_document_scan,
    meta_validate_types_with_list_presentation, MetaValidationOwnerKind,
};
use super::xml_model::{
    meta_info_child, meta_info_child_text, meta_info_children, meta_info_inner_text,
    parse_metadata_image,
};

const MD_CLASSES_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

pub(super) struct MetaValidationReporter {
    pub(crate) errors: usize,
    pub(crate) warnings: usize,
    pub(crate) ok_count: usize,
    pub(crate) stopped: bool,
    pub(crate) max_errors: usize,
    pub(crate) detailed: bool,
    pub(crate) lines: Vec<String>,
    pub(crate) md_type: String,
    pub(crate) obj_name: String,
    pub(crate) suppress_artifact: bool,
}

pub(super) struct MetaValidationRun {
    pub(crate) ok: bool,
    pub(crate) stdout: String,
    pub(crate) artifacts: Vec<PathBuf>,
    pub(crate) errors: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub(super) struct MetaValidationOptions {
    pub(crate) detailed: bool,
    pub(crate) max_errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MetaValidationScope {
    PublicOwnerAware,
    PostWriteLocal,
}

struct MetaValidationReferenceInputs<'a> {
    config_dir: Option<PathBuf>,
    language_codes: Vec<String>,
    proof_subject: Option<&'a MetadataValidationSubject>,
}

pub(crate) struct MetadataValidator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChildGraphValidation {
    Closed,
    DescriptorOnly,
}

impl MetadataValidator {
    pub(crate) fn validate(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
    ) -> MetadataValidationResult {
        self.evaluate(
            subject,
            context,
            &MetaValidationOptions {
                detailed: true,
                max_errors: 30,
            },
            None,
            ChildGraphValidation::Closed,
        )
        .0
    }

    pub(crate) fn validate_complete_read(
        &self,
        subject: &MetadataValidationSubject,
        context: &WorkspaceContext,
    ) -> MetadataValidationResult {
        let mut result = self.validate(subject, context);
        let completeness = complete_read_proof_diagnostics(subject);
        if !completeness.is_empty() {
            result.status = MetaValidationStatus::Failed;
            result.diagnostics.extend(completeness);
        }
        result
    }

    fn evaluate(
        &self,
        subject: &MetadataValidationSubject,
        _context: &WorkspaceContext,
        options: &MetaValidationOptions,
        artifact: Option<PathBuf>,
        child_graph_validation: ChildGraphValidation,
    ) -> (MetadataValidationResult, Option<MetaValidationRun>) {
        let mut diagnostics = Vec::new();
        let mut legacy_run = None;
        let descriptor_indices = subject
            .resources
            .iter()
            .enumerate()
            .filter_map(|(index, resource)| {
                matches!(resource.role, MetadataResourceRole::Descriptor).then_some(index)
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
                MetadataResourceRole::Descriptor
                | MetadataResourceRole::Registration
                | MetadataResourceRole::Dependency { .. }
                | MetadataResourceRole::Form { .. }
                | MetadataResourceRole::Template { .. }
                | MetadataResourceRole::Command { .. } => {
                    if let Err(error) = parse_metadata_image(&resource.bytes) {
                        diagnostics.push(provider_diagnostic(subject, index, error));
                        continue;
                    }
                }
                MetadataResourceRole::Module { .. } => {
                    if let Err(error) = std::str::from_utf8(&resource.bytes) {
                        diagnostics.push(provider_diagnostic(
                            subject,
                            index,
                            format!("module image is not UTF-8: {error}"),
                        ));
                        continue;
                    }
                }
                MetadataResourceRole::AuxiliaryXml { kind, .. } => {
                    if let Err(error) = validate_auxiliary_xml(*kind, &resource.bytes) {
                        diagnostics.push(provider_diagnostic(subject, index, error));
                        continue;
                    }
                }
                MetadataResourceRole::ChildResource { child, kind, .. } => {
                    if let Err(error) = validate_child_resource(*kind, &resource.bytes) {
                        diagnostics.push(child_resource_diagnostic(child, index, error));
                        continue;
                    }
                }
            }

            if matches!(resource.role, MetadataResourceRole::Registration) {
                match inspect_metadata_registration_image(&resource.bytes) {
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
            } else if let MetadataResourceRole::Dependency { target } = &resource.role {
                match inspect_metadata_language_image(&resource.bytes) {
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

        if child_graph_validation == ChildGraphValidation::Closed {
            validate_child_footprints(subject, &mut diagnostics);
        }

        if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable)
        {
            return (failed_validation(diagnostics), legacy_run);
        }

        let (target_type, target_name) = subject_target_identity(subject);
        if let Some(descriptor_index) = descriptor_indices.first().copied() {
            let descriptor = &subject.resources[descriptor_index];
            if let Ok(identity) = inspect_metadata_image_identity(&descriptor.bytes) {
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

            let has_registration_image = subject
                .resources
                .iter()
                .any(|resource| matches!(resource.role, MetadataResourceRole::Registration));
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
            if meta_validate_types_with_list_presentation().contains(&target_type) {
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
                return (failed_validation(diagnostics), legacy_run);
            }

            let inputs = MetaValidationReferenceInputs {
                config_dir: None,
                language_codes,
                proof_subject: Some(subject),
            };
            match meta_validate_source(&descriptor.bytes, options, &inputs, artifact) {
                Ok(run) => {
                    diagnostics.extend(run.errors.iter().map(|error| {
                        let field = if error.contains("is not registered in the owner image") {
                            subject
                                .resources
                                .iter()
                                .position(|resource| {
                                    matches!(resource.role, MetadataResourceRole::Registration)
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
                    legacy_run = Some(run);
                }
                Err(error) => {
                    diagnostics.push(provider_diagnostic(subject, descriptor_index, error))
                }
            }
        } else {
            for (index, resource) in subject.resources.iter().enumerate() {
                match resource.role {
                    MetadataResourceRole::Registration => {
                        if inspect_metadata_registration_image(&resource.bytes).is_ok_and(
                            |registration| {
                                registration
                                    .registrations
                                    .iter()
                                    .any(|(kind, name)| kind == target_type && name == target_name)
                            },
                        ) {
                            diagnostics.push(validation_diagnostic(
                                subject,
                                &format!("resources[{index}].bytes"),
                                format!("surviving registration still contains {}", subject.target),
                            ));
                        }
                    }
                    MetadataResourceRole::Dependency { .. } => {
                        if image_contains_reference(&resource.bytes, subject.target.as_str()) {
                            diagnostics.push(validation_diagnostic(
                                subject,
                                &format!("resources[{index}].bytes"),
                                format!("surviving reference still targets {}", subject.target),
                            ));
                        }
                    }
                    MetadataResourceRole::Descriptor
                    | MetadataResourceRole::Module { .. }
                    | MetadataResourceRole::Form { .. }
                    | MetadataResourceRole::Template { .. }
                    | MetadataResourceRole::Command { .. }
                    | MetadataResourceRole::ChildResource { .. }
                    | MetadataResourceRole::AuxiliaryXml { .. } => {}
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
        (result, legacy_run)
    }
}

fn complete_read_proof_diagnostics(subject: &MetadataValidationSubject) -> Vec<MetaDiagnostic> {
    let Some(descriptor) = subject
        .resources
        .iter()
        .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
    else {
        return vec![complete_read_missing(
            subject,
            "metadata descriptor proof is unavailable",
        )];
    };
    let Ok((_, document)) = parse_metadata_image(&descriptor.bytes) else {
        return Vec::new();
    };
    let Some(object) = document
        .root_element()
        .children()
        .find(|node| node.is_element() && node.tag_name().namespace() == Some(MD_CLASSES_NS))
    else {
        return Vec::new();
    };
    let object_type = object.tag_name().name();
    let properties = meta_info_child(object, "Properties");
    let object_name = properties
        .and_then(|node| meta_info_child_text(node, "Name"))
        .unwrap_or_default();
    let mut diagnostics = Vec::new();

    for language in subject
        .resources
        .iter()
        .find(|resource| matches!(resource.role, MetadataResourceRole::Registration))
        .and_then(|resource| inspect_metadata_registration_image(&resource.bytes).ok())
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
            .and_then(|node| meta_info_child(node, "RegisterRecords"))
            .map(|node| meta_info_children(node, "Item"))
            .unwrap_or_default()
            .into_iter()
            .map(meta_info_inner_text)
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

    if matches!(object_type, "EventSubscription" | "ScheduledJob") {
        let property = if object_type == "EventSubscription" {
            "Handler"
        } else {
            "MethodName"
        };
        if let Some(reference) = properties
            .and_then(|node| meta_info_child_text(node, property))
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
                        MetadataResourceRole::Module { owner } if owner.as_str() == target
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
            .and_then(|node| meta_info_child_text(node, "WriteMode"))
            .as_deref()
            == Some("RecorderSubordinate"));
    if reads_registrar && !object_name.is_empty() {
        let reference = format!("{object_type}.{object_name}");
        let has_registrar = subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                MetadataResourceRole::Dependency { target }
                    if target.segments().next() == Some("Document")
            ) && document_registers(&resource.bytes, &reference)
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
        let complete = meta_info_child(child, "Properties")
            .and_then(|node| meta_info_child_text(node, "Name"))
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

fn has_dependency(subject: &MetadataValidationSubject, expected: &str) -> bool {
    subject.resources.iter().any(|resource| {
        matches!(
            &resource.role,
            MetadataResourceRole::Dependency { target } if target.as_str() == expected
        )
    })
}

fn document_registers(bytes: &[u8], expected: &str) -> bool {
    let Ok((_, document)) = parse_metadata_image(bytes) else {
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
        .and_then(|object| meta_info_child(object, "Properties"))
        .and_then(|properties| meta_info_child(properties, "RegisterRecords"))
        .is_some_and(|records| {
            meta_info_children(records, "Item")
                .into_iter()
                .any(|item| meta_info_inner_text(item).trim() == expected)
        })
}

fn complete_read_missing(
    subject: &MetadataValidationSubject,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(subject.target.clone())
        .with_field("resources")
}

fn complete_read_invalid(
    subject: &MetadataValidationSubject,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field("resources")
}

fn validate_auxiliary_xml(kind: MetadataAuxiliaryXmlKind, bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| format!("auxiliary XML image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let document = Document::parse(text)
        .map_err(|error| format!("auxiliary XML image is malformed: {error}"))?;
    let root = document.root_element();
    let (expected_name, expected_namespace) = match kind {
        MetadataAuxiliaryXmlKind::ExchangePlanContent => {
            ("ExchangePlanContent", "http://v8.1c.ru/8.3/xcf/extrnprops")
        }
        MetadataAuxiliaryXmlKind::BusinessProcessFlowchart => {
            ("GraphicalSchema", "http://v8.1c.ru/8.3/xcf/scheme")
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

fn validate_child_resource(kind: MetadataChildResourceKind, bytes: &[u8]) -> Result<(), String> {
    match kind {
        MetadataChildResourceKind::FormContent => {
            let text = std::str::from_utf8(bytes)
                .map_err(|error| format!("form content is not UTF-8: {error}"))?;
            let source = text.trim_start_matches('\u{feff}');
            require_utf8_xml_declaration(source, "form content")?;
            let document = Document::parse(source)
                .map_err(|error| format!("form content is not valid XML: {error}"))?;
            let root = document.root_element();
            super::super::form::require_form_root(root)?;
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
        MetadataChildResourceKind::TemplateContent {
            template_type,
            part,
        } => validate_template_resource(template_type, part, bytes),
        MetadataChildResourceKind::Module => std::str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|error| format!("child module is not UTF-8: {error}")),
    }
}

fn validate_child_footprints(
    subject: &MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
) {
    let expected = final_owner_child_graph(subject, diagnostics);
    let descriptors = actual_child_descriptor_graph(subject, diagnostics);

    for child in &expected {
        let matching_descriptors = descriptors
            .iter()
            .filter(|candidate| candidate.0.child == child.child)
            .collect::<Vec<_>>();
        let descriptor = matching_descriptors
            .first()
            .filter(|descriptor| {
                matching_descriptors.len() == 1
                    && child.kind.matches(descriptor.0.profile)
                    && child_descriptor_role_matches(
                        &descriptor.1,
                        &child.child,
                        descriptor.0.profile,
                    )
            })
            .map(|descriptor| &descriptor.0);
        let Some(descriptor) = descriptor else {
            diagnostics.push(template_resource_set_diagnostic(
                &child.child,
                "final child is not backed by exactly one byte-matching descriptor",
            ));
            continue;
        };

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
            MetadataResourceRole::ChildResource { child, .. } => Some(child),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClosedChildDescriptor {
    child: MetadataAddress,
    profile: MetadataChildProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosedChildKind {
    Form,
    Template,
    Command,
}

impl ClosedChildKind {
    fn matches(self, profile: MetadataChildProfile) -> bool {
        matches!(
            (self, profile),
            (Self::Form, MetadataChildProfile::Form)
                | (Self::Template, MetadataChildProfile::Template(_))
                | (Self::Command, MetadataChildProfile::Command)
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClosedOwnerChild {
    child: MetadataAddress,
    kind: ClosedChildKind,
}

fn final_owner_child_graph(
    subject: &MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<ClosedOwnerChild> {
    let owners = subject
        .resources
        .iter()
        .filter(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
        .collect::<Vec<_>>();
    let [owner] = owners.as_slice() else {
        if !subject.child_footprints.is_empty()
            || subject.resources.iter().any(|resource| {
                matches!(
                    resource.role,
                    MetadataResourceRole::Form { .. }
                        | MetadataResourceRole::Template { .. }
                        | MetadataResourceRole::Command { .. }
                        | MetadataResourceRole::ChildResource { .. }
                )
            })
        {
            diagnostics.push(template_resource_set_diagnostic(
                &subject.target,
                "child validation requires exactly one final owner descriptor",
            ));
        }
        return Vec::new();
    };
    let Ok(identity) = inspect_metadata_image_identity(&owner.bytes) else {
        return Vec::new();
    };
    if subject.target.as_str() != format!("{}.{}", identity.object_type, identity.object_name) {
        return Vec::new();
    }
    match parse_final_owner_children(&owner.bytes, &subject.target) {
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

fn actual_child_descriptor_graph(
    subject: &MetadataValidationSubject,
    diagnostics: &mut Vec<MetaDiagnostic>,
) -> Vec<(ClosedChildDescriptor, MetadataResourceRole)> {
    let mut descriptors = Vec::new();
    for resource in &subject.resources {
        if matches!(resource.role, MetadataResourceRole::Descriptor) {
            continue;
        }
        let declared_child = matches!(
            resource.role,
            MetadataResourceRole::Form { .. }
                | MetadataResourceRole::Template { .. }
                | MetadataResourceRole::Command { .. }
        );
        match parse_child_descriptor(&resource.bytes, &subject.target) {
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

fn parse_final_owner_children(
    bytes: &[u8],
    owner: &MetadataAddress,
) -> Result<Vec<ClosedOwnerChild>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document)?;
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

fn validate_exact_owner_identity(
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
    let name = meta_info_inner_text(*name).trim().to_string();
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

fn parse_owner_child_artifact(
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
            meta_info_inner_text(artifact).trim().to_string()
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
    Ok(ClosedOwnerChild { child, kind })
}

fn parse_child_descriptor(
    bytes: &[u8],
    owner: &MetadataAddress,
) -> Result<Option<ClosedChildDescriptor>, String> {
    let (_, document) = parse_metadata_image(bytes)?;
    let artifact = exact_metadata_artifact(&document)?;
    if !matches!(artifact.tag_name().name(), "Form" | "Template" | "Command") {
        return Ok(None);
    }
    parse_child_artifact(artifact, owner).map(Some)
}

fn exact_metadata_artifact<'a, 'input>(
    document: &'a Document<'input>,
) -> Result<roxmltree::Node<'a, 'input>, String> {
    let root = document.root_element();
    if root.tag_name().namespace() != Some(MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
        || root.attribute("version") != Some(ACTIVE_FORMAT_PROFILE.export_format)
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

fn parse_child_artifact(
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
        "Form" => MetadataChildProfile::Form,
        "Command" => MetadataChildProfile::Command,
        "Template" => {
            let template_types =
                exact_mdclasses_children(*properties, "TemplateType", "Template descriptor")?;
            let [template_type] = template_types.as_slice() else {
                return Err("Template descriptor must contain exactly one TemplateType".into());
            };
            let value = meta_info_inner_text(*template_type);
            MetadataChildProfile::Template(
                MetadataTemplateType::from_descriptor_value(value.trim()).ok_or_else(|| {
                    "Template descriptor has unsupported TemplateType".to_string()
                })?,
            )
        }
        _ => unreachable!(),
    };
    let child = MetadataAddress::parse(
        PLATFORM_XML_8_3_27_FORMAT_2_20,
        &format!("{owner}.{child_kind}.{name}"),
    )
    .map_err(|_| "child descriptor identity is not representable".to_string())?;
    Ok(ClosedChildDescriptor { child, profile })
}

fn exact_child_name(properties: roxmltree::Node<'_, '_>, kind: &str) -> Result<String, String> {
    let names = exact_mdclasses_children(properties, "Name", &format!("{kind} descriptor"))?;
    let [name] = names.as_slice() else {
        return Err(format!("{kind} descriptor must contain exactly one Name"));
    };
    let name = meta_info_inner_text(*name).trim().to_string();
    if name.is_empty() {
        return Err(format!("{kind} descriptor Name is empty"));
    }
    Ok(name)
}

fn exact_mdclasses_children<'a, 'input>(
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

fn child_descriptor_role_matches(
    role: &MetadataResourceRole,
    child: &MetadataAddress,
    profile: MetadataChildProfile,
) -> bool {
    match (role, profile) {
        (MetadataResourceRole::Form { owner, name }, MetadataChildProfile::Form) => {
            child.as_str() == format!("{owner}.Form.{name}")
        }
        (MetadataResourceRole::Command { owner, name }, MetadataChildProfile::Command) => {
            child.as_str() == format!("{owner}.Command.{name}")
        }
        (MetadataResourceRole::Template { owner, name }, MetadataChildProfile::Template(_)) => {
            child.as_str() == format!("{owner}.Template.{name}")
        }
        _ => false,
    }
}

fn validate_one_child_footprint(
    subject: &MetadataValidationSubject,
    expected: &ClosedChildDescriptor,
    footprint: &crate::application::ports::MetadataChildFootprintEvidence,
    diagnostics: &mut Vec<MetaDiagnostic>,
) {
    let resources = subject
        .resources
        .iter()
        .filter_map(|resource| match resource.role {
            MetadataResourceRole::ChildResource {
                ref child,
                kind,
                ordinal,
            } if child == &expected.child => Some((kind, ordinal)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let module_count = resources
        .iter()
        .filter(|(kind, _)| matches!(kind, MetadataChildResourceKind::Module))
        .count();
    let expected_directories = match expected.profile {
        MetadataChildProfile::Command if module_count == 0 => Vec::new(),
        MetadataChildProfile::Form | MetadataChildProfile::Command => vec![
            MetadataChildDirectoryKind::Root,
            MetadataChildDirectoryKind::Extension,
        ],
        MetadataChildProfile::Template(MetadataTemplateType::HtmlDocument) => vec![
            MetadataChildDirectoryKind::Root,
            MetadataChildDirectoryKind::Extension,
            MetadataChildDirectoryKind::HtmlPages,
        ],
        MetadataChildProfile::Template(_) => vec![
            MetadataChildDirectoryKind::Root,
            MetadataChildDirectoryKind::Extension,
        ],
    }
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    let observed_directories = footprint
        .directories
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if observed_directories != expected_directories
        || observed_directories.len() != footprint.directories.len()
    {
        diagnostics.push(template_resource_set_diagnostic(
            &expected.child,
            "child footprint does not contain its exact required directory set",
        ));
    }

    let expected_kinds = match expected.profile {
        MetadataChildProfile::Form => {
            let mut kinds = vec![(MetadataChildResourceKind::FormContent, 0)];
            if module_count == 1 {
                kinds.push((MetadataChildResourceKind::Module, 1));
            }
            kinds
        }
        MetadataChildProfile::Command => (module_count == 1)
            .then_some((MetadataChildResourceKind::Module, 0))
            .into_iter()
            .collect(),
        MetadataChildProfile::Template(template_type) => {
            let mut kinds = vec![(
                MetadataChildResourceKind::TemplateContent {
                    template_type,
                    part: MetadataTemplateResourcePart::Primary,
                },
                0,
            )];
            if template_type == MetadataTemplateType::HtmlDocument {
                kinds.push((
                    MetadataChildResourceKind::TemplateContent {
                        template_type,
                        part: MetadataTemplateResourcePart::HtmlPage,
                    },
                    1,
                ));
            }
            kinds
        }
    };
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

fn template_resource_set_diagnostic(child: &MetadataAddress, message: &str) -> MetaDiagnostic {
    MetaDiagnostic::error(
        MetaDiagnosticCode::ProviderUnavailable,
        format!("{message}: {child}"),
    )
    .with_metadata_path(child.clone())
    .with_field("resources")
}

fn validate_template_resource(
    template_type: MetadataTemplateType,
    part: MetadataTemplateResourcePart,
    bytes: &[u8],
) -> Result<(), String> {
    match (template_type, part) {
        (MetadataTemplateType::SpreadsheetDocument, MetadataTemplateResourcePart::Primary) => {
            validate_versionless_xml(bytes, MXL_ROOT, "spreadsheet template")
        }
        (MetadataTemplateType::DataCompositionSchema, MetadataTemplateResourcePart::Primary) => {
            validate_versionless_xml(bytes, DCS_ROOT, "data composition schema template")
        }
        (MetadataTemplateType::TextDocument, MetadataTemplateResourcePart::Primary) => {
            std::str::from_utf8(bytes)
                .map(|_| ())
                .map_err(|error| format!("text template is not UTF-8: {error}"))
        }
        (MetadataTemplateType::BinaryData, MetadataTemplateResourcePart::Primary) => Ok(()),
        (MetadataTemplateType::HtmlDocument, MetadataTemplateResourcePart::Primary) => {
            validate_exact_version_xml(
                bytes,
                PlatformXmlRootExpectation::new("http://v8.1c.ru/8.3/xcf/extrnprops", "Help"),
                "HTML template descriptor",
            )?;
            let (_, document) = parse_typed_payload_xml(bytes, "HTML template descriptor")?;
            let root = document.root_element();
            let pages = root
                .children()
                .filter(|node| {
                    node.is_element()
                        && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/xcf/extrnprops")
                        && node.tag_name().name() == "Page"
                })
                .filter_map(|node| node.text())
                .map(str::trim)
                .collect::<Vec<_>>();
            if pages.as_slice() != ["ru"] {
                return Err(
                    "HTML template descriptor must declare exactly the ru language page"
                        .to_string(),
                );
            }
            Ok(())
        }
        (MetadataTemplateType::HtmlDocument, MetadataTemplateResourcePart::HtmlPage) => {
            let text = std::str::from_utf8(bytes)
                .map_err(|error| format!("HTML template page is not UTF-8: {error}"))?;
            let source = text.trim_start_matches('\u{feff}');
            require_utf8_xml_declaration(source, "HTML template page")?;
            let document = Document::parse(source)
                .map_err(|error| format!("HTML template page is malformed: {error}"))?;
            let root = document.root_element();
            if root.tag_name().namespace().is_some() || root.tag_name().name() != "html" {
                return Err("HTML template page root must be html without a namespace".to_string());
            }
            Ok(())
        }
        _ => Err("template resource part does not match its TemplateType".to_string()),
    }
}

fn validate_versionless_xml(
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

fn validate_exact_version_xml(
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

fn parse_typed_payload_xml<'a>(
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

fn require_utf8_xml_declaration(source: &str, role: &str) -> Result<(), String> {
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

fn require_exact_payload_root(
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

fn failed_validation(diagnostics: Vec<MetaDiagnostic>) -> MetadataValidationResult {
    MetaValidationData {
        status: MetaValidationStatus::Failed,
        diagnostics,
    }
}

fn provider_diagnostic(
    subject: &MetadataValidationSubject,
    resource_index: usize,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ProviderUnavailable, message)
        .with_metadata_path(subject.target.clone())
        .with_field(format!("resources[{resource_index}].bytes"))
}

fn child_resource_diagnostic(
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

fn validation_diagnostic(
    subject: &MetadataValidationSubject,
    field: &str,
    message: impl Into<String>,
) -> MetaDiagnostic {
    MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field(field)
}

fn validation_warning(
    subject: &MetadataValidationSubject,
    field: &str,
    message: impl Into<String>,
) -> MetaDiagnostic {
    let mut diagnostic = MetaDiagnostic::error(MetaDiagnosticCode::ValidationFailed, message)
        .with_metadata_path(subject.target.clone())
        .with_field(field);
    diagnostic.severity = MetaDiagnosticSeverity::Warning;
    diagnostic
}

fn subject_target_identity(subject: &MetadataValidationSubject) -> (&str, &str) {
    let mut segments = subject.target.segments();
    (
        segments.next().unwrap_or_default(),
        segments.next().unwrap_or_default(),
    )
}

fn image_contains_reference(bytes: &[u8], target: &str) -> bool {
    let Ok((_, document)) = parse_metadata_image(bytes) else {
        return false;
    };
    document
        .descendants()
        .filter(roxmltree::Node::is_element)
        .any(|node| node.text().is_some_and(|text| text.trim() == target))
}

fn metadata_image_reference(bytes: &[u8]) -> Result<String, String> {
    let (_, document) = parse_metadata_image(bytes)?;
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
    let name = meta_info_child(*artifact, "Properties")
        .and_then(|properties| meta_info_child_text(properties, "Name"))
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "metadata descriptor has no Name".to_string())?;
    Ok(format!("{}.{}", artifact.tag_name().name(), name))
}

fn validation_field_for_legacy_error(error: &str) -> &'static str {
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
    pub(super) fn new(max_errors: usize, detailed: bool) -> Self {
        Self {
            errors: 0,
            warnings: 0,
            ok_count: 0,
            stopped: false,
            max_errors,
            detailed,
            lines: vec![String::new()],
            md_type: "(unknown)".to_string(),
            obj_name: "(unknown)".to_string(),
            suppress_artifact: false,
        }
    }

    pub(super) fn ok(&mut self, message: impl Into<String>) {
        self.ok_count += 1;
        if self.detailed {
            self.lines.push(format!("[OK]    {}", message.into()));
        }
    }

    pub(super) fn error(&mut self, message: impl Into<String>) {
        self.errors += 1;
        self.lines.push(format!("[ERROR] {}", message.into()));
        if self.errors >= self.max_errors {
            self.stopped = true;
        }
    }

    pub(super) fn warn(&mut self, message: impl Into<String>) {
        self.warnings += 1;
        self.lines.push(format!("[WARN]  {}", message.into()));
    }

    pub(super) fn finalize(mut self) -> (bool, String, Vec<String>, Vec<String>) {
        let checks = self.ok_count + self.errors + self.warnings;
        let ok = self.errors == 0;
        if ok && self.warnings == 0 && !self.detailed {
            return (
                true,
                format!(
                    "=== Validation OK: {}.{} ({checks} checks) ===",
                    self.md_type, self.obj_name
                ),
                Vec::new(),
                Vec::new(),
            );
        }
        self.lines.insert(
            0,
            format!("=== Validation: {}.{} ===", self.md_type, self.obj_name),
        );
        self.lines.push(String::new());
        self.lines.push(format!(
            "=== Result: {} errors, {} warnings ({checks} checks) ===",
            self.errors, self.warnings
        ));
        let errors = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[ERROR]"))
            .cloned()
            .collect::<Vec<_>>();
        let warnings = self
            .lines
            .iter()
            .filter(|line| line.starts_with("[WARN]"))
            .cloned()
            .collect::<Vec<_>>();
        (ok, self.lines.join("\n"), errors, warnings)
    }
}

pub(crate) fn validate_meta(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> AdapterOutcome {
    let result = (|| -> Result<MetaValidationRun, String> {
        let raw_path = required_path(
            args,
            &["objectPath", "ObjectPath", "path", "Path"],
            "ObjectPath",
        )?;
        let raw_path_text = raw_path.to_string_lossy();
        let paths = raw_path_text
            .split('|')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return Err("[ERROR] No ObjectPath values were provided".to_string());
        }

        let options = meta_validation_options(args);
        if paths.len() > 1 {
            meta_validate_batch(paths, &options, context)
        } else {
            meta_validate_one(paths[0].clone(), &options, context)
        }
    })();

    match result {
        Ok(run) => {
            let artifacts = run
                .artifacts
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            AdapterOutcome {
                ok: run.ok,
                summary: if run.ok {
                    "unica.meta.validate completed with native metadata validator".to_string()
                } else {
                    "unica.meta.validate failed in native metadata validator".to_string()
                },
                changes: Vec::new(),
                warnings: Vec::new(),
                errors: run.errors,
                artifacts,
                stdout: Some(run.stdout),
                stderr: Some(String::new()),
                command: None,
            }
        }
        Err(error) => AdapterOutcome {
            ok: false,
            summary: "unica.meta.validate failed in native metadata validator".to_string(),
            changes: Vec::new(),
            warnings: Vec::new(),
            errors: vec![error.clone()],
            artifacts: Vec::new(),
            stdout: Some(format!("{error}\n")),
            stderr: Some(String::new()),
            command: None,
        },
    }
}

pub(super) fn meta_validation_options(args: &Map<String, Value>) -> MetaValidationOptions {
    MetaValidationOptions {
        detailed: bool_arg(args, &["detailed", "Detailed"]),
        max_errors: int_arg(args, &["maxErrors", "MaxErrors"])
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(30),
    }
}

/// Return the platform XML documents whose contents `meta.validate` reads,
/// including each member of a batch and the registrar documents inspected for
/// register cross-reference diagnostics.
pub(crate) fn meta_validate_format_dependency_paths(
    args: &Map<String, Value>,
    context: &WorkspaceContext,
) -> Result<Vec<PathBuf>, String> {
    let raw_path = required_path(
        args,
        &["objectPath", "ObjectPath", "path", "Path"],
        "ObjectPath",
    )?;
    let raw_path_text = raw_path.to_string_lossy();
    let mut dependencies = Vec::new();
    for raw in raw_path_text
        .split('|')
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        let candidate = absolutize(PathBuf::from(raw), &context.cwd);
        let object_path = resolve_meta_info_path(candidate.clone()).unwrap_or(candidate);
        let inspection = inspect_meta_validation_reads(&object_path, context);
        for path in inspection.paths {
            if !dependencies.contains(&path) {
                dependencies.push(path);
            }
        }
    }
    Ok(dependencies)
}

pub(super) fn meta_validate_batch(
    paths: Vec<PathBuf>,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    let total = paths.len();
    let mut passed = 0usize;
    let mut failed = 0usize;
    let mut stdout_blocks = Vec::<String>::new();
    let mut errors = Vec::<String>::new();
    let mut warnings = Vec::<String>::new();
    let mut artifacts = Vec::<PathBuf>::new();

    for path in paths {
        match meta_validate_one(path.clone(), options, context) {
            Ok(run) => {
                if run.ok {
                    passed += 1;
                } else {
                    failed += 1;
                }
                errors.extend(run.errors);
                warnings.extend(run.warnings);
                artifacts.extend(run.artifacts);
                stdout_blocks.push(format!("--- {} ---", path.display()));
                stdout_blocks.push(run.stdout.trim_end().to_string());
            }
            Err(error) => {
                failed += 1;
                let message = format!("[ERROR] {}: {error}", path.display());
                errors.push(message.clone());
                stdout_blocks.push(message);
            }
        }
    }

    stdout_blocks.push(String::new());
    stdout_blocks.push("=== meta-validate batch summary ===".to_string());
    stdout_blocks.push(format!("Validated: {total}"));
    stdout_blocks.push(format!("Passed:    {passed}"));
    stdout_blocks.push(format!("Failed:    {failed}"));

    Ok(MetaValidationRun {
        ok: failed == 0,
        stdout: format!("{}\n", stdout_blocks.join("\n")),
        artifacts,
        errors,
        warnings,
    })
}

pub(super) fn meta_validate_one(
    raw_path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
) -> Result<MetaValidationRun, String> {
    meta_validate_one_with_scope(
        raw_path,
        options,
        context,
        MetaValidationScope::PublicOwnerAware,
    )
}

fn metadata_address(raw: impl AsRef<str>) -> Result<MetadataAddress, String> {
    MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, raw.as_ref())
        .map_err(|error| error.to_string())
}

fn metadata_collection(object_type: &str) -> Option<&'static str> {
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

fn push_dependency_from_path(
    resources: &mut Vec<crate::application::ports::MetadataResourceImage>,
    target: MetadataAddress,
    path: &Path,
) -> Result<(), String> {
    if resources.iter().any(|resource| {
        matches!(
            &resource.role,
            MetadataResourceRole::Dependency { target: existing } if existing == &target
        )
    }) {
        return Ok(());
    }
    if path.is_file() {
        resources.push(crate::application::ports::MetadataResourceImage {
            role: MetadataResourceRole::Dependency { target },
            bytes: fs::read(path)
                .map_err(|error| format!("failed to read {}: {error}", path.display()))?,
        });
    }
    Ok(())
}

pub(super) fn add_descriptor_references(
    resources: &mut Vec<crate::application::ports::MetadataResourceImage>,
    descriptor: &[u8],
    config_dir: Option<&Path>,
) -> Result<(), String> {
    let Some(config_dir) = config_dir else {
        return Ok(());
    };
    let (_, document) = parse_metadata_image(descriptor)?;
    let Some(object) = document.root_element().children().find(|node| {
        node.is_element() && node.tag_name().namespace() == Some("http://v8.1c.ru/8.3/MDClasses")
    }) else {
        return Ok(());
    };
    let object_type = object.tag_name().name();
    let properties = meta_info_child(object, "Properties");

    if object_type == "Document" {
        for reference in properties
            .and_then(|properties| meta_info_child(properties, "RegisterRecords"))
            .map(|records| meta_info_children(records, "Item"))
            .unwrap_or_default()
            .into_iter()
            .map(meta_info_inner_text)
        {
            let Some((kind, name)) = reference.split_once('.') else {
                continue;
            };
            let Some(collection) = metadata_collection(kind) else {
                continue;
            };
            let target = metadata_address(&reference)?;
            let path = config_dir.join(collection).join(format!("{name}.xml"));
            push_dependency_from_path(resources, target, &path)?;
        }
    }

    if matches!(object_type, "EventSubscription" | "ScheduledJob") {
        let property = if object_type == "EventSubscription" {
            "Handler"
        } else {
            "MethodName"
        };
        if let Some(reference) = properties
            .and_then(|properties| meta_info_child_text(properties, property))
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
                push_dependency_from_path(resources, owner.clone(), &descriptor_path)?;
                let module_path = config_dir
                    .join("CommonModules")
                    .join(module_name)
                    .join("Ext")
                    .join("Module.bsl");
                if module_path.is_file() {
                    resources.push(crate::application::ports::MetadataResourceImage {
                        role: MetadataResourceRole::Module { owner },
                        bytes: fs::read(&module_path).map_err(|error| {
                            format!("failed to read {}: {error}", module_path.display())
                        })?,
                    });
                }
            }
        }
    }
    Ok(())
}

fn metadata_validation_subject_from_paths(
    resolved_path: &Path,
    inspection_paths: &[PathBuf],
    owner_context: &super::validation_context::MetaValidationOwnerContext,
) -> Result<MetadataValidationSubject, String> {
    use crate::application::ports::MetadataResourceImage;

    let target = metadata_address(format!(
        "{}.{}",
        owner_context.object_type, owner_context.object_name
    ))?;
    let descriptor = fs::read(resolved_path)
        .map_err(|error| format!("failed to read {}: {error}", resolved_path.display()))?;
    let mut resources = vec![MetadataResourceImage {
        role: MetadataResourceRole::Descriptor,
        bytes: descriptor.clone(),
    }];
    let owner_path = owner_context
        .owner_path
        .canonicalize()
        .unwrap_or_else(|_| owner_context.owner_path.clone());
    for path in inspection_paths {
        if path == resolved_path {
            continue;
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if path == &owner_path {
            resources.push(MetadataResourceImage {
                role: MetadataResourceRole::Registration,
                bytes,
            });
            continue;
        }
        let dependency_target = match inspect_metadata_language_image(&bytes)? {
            Some((name, _)) => metadata_address(format!("Language.{name}"))?,
            None => {
                let identity = inspect_metadata_image_identity(&bytes)?;
                metadata_address(format!("{}.{}", identity.object_type, identity.object_name))?
            }
        };
        resources.push(MetadataResourceImage {
            role: MetadataResourceRole::Dependency {
                target: dependency_target,
            },
            bytes,
        });
    }
    let config_dir = match owner_context.owner_kind {
        MetaValidationOwnerKind::Configuration | MetaValidationOwnerKind::Extension => {
            owner_context.owner_path.parent()
        }
        MetaValidationOwnerKind::External => None,
    };
    add_descriptor_references(&mut resources, &descriptor, config_dir)?;
    Ok(MetadataValidationSubject {
        target,
        resources,
        child_footprints: Vec::new(),
    })
}

fn metadata_validation_run(
    subject: &MetadataValidationSubject,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
    artifact: PathBuf,
) -> MetaValidationRun {
    let (result, legacy_run) = MetadataValidator.evaluate(
        subject,
        context,
        options,
        Some(artifact),
        ChildGraphValidation::DescriptorOnly,
    );
    let ok = result.status == MetaValidationStatus::Passed;
    if let Some(run) = legacy_run {
        debug_assert_eq!(run.ok, ok, "report and validation status diverged");
        return run;
    }
    let mut errors = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Error)
        .take(options.max_errors)
        .map(|diagnostic| format!("[ERROR] {}", diagnostic.message))
        .collect::<Vec<_>>();
    let warnings = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == MetaDiagnosticSeverity::Warning)
        .map(|diagnostic| format!("[WARN]  {}", diagnostic.message))
        .collect::<Vec<_>>();
    if errors.is_empty() && !ok {
        errors.push("[ERROR] metadata validation failed".to_string());
    }
    let stdout = if errors.is_empty() && warnings.is_empty() && !options.detailed {
        format!("=== Validation OK: {} ===", subject.target)
    } else {
        let mut lines = vec![format!("=== Validation: {} ===", subject.target)];
        lines.extend(errors.iter().cloned());
        lines.extend(warnings.iter().cloned());
        lines.push(String::new());
        lines.push(format!(
            "=== Result: {} errors, {} warnings ===",
            errors.len(),
            warnings.len()
        ));
        lines.join("\n")
    };
    MetaValidationRun {
        ok,
        stdout,
        artifacts: Vec::new(),
        errors,
        warnings,
    }
}

pub(super) fn meta_validate_one_with_scope(
    raw_path: PathBuf,
    options: &MetaValidationOptions,
    context: &WorkspaceContext,
    scope: MetaValidationScope,
) -> Result<MetaValidationRun, String> {
    let object_path = resolve_meta_info_path(absolutize(raw_path, &context.cwd))?;
    let resolved_path = object_path
        .canonicalize()
        .unwrap_or_else(|_| object_path.clone());
    let owner_inspection = match scope {
        MetaValidationScope::PublicOwnerAware => Some(inspect_meta_validation_subject_reads(
            &resolved_path,
            context,
        )),
        MetaValidationScope::PostWriteLocal => None,
    };

    if let Some(inspection) = owner_inspection {
        let owner_context = inspection.context?;
        let subject = metadata_validation_subject_from_paths(
            &resolved_path,
            &inspection.paths,
            &owner_context,
        )?;
        return Ok(metadata_validation_run(
            &subject,
            options,
            context,
            resolved_path,
        ));
    }

    let text = read_utf8_sig(&resolved_path)?;
    let reference_inputs = match scope {
        MetaValidationScope::PublicOwnerAware => unreachable!("handled above"),
        MetaValidationScope::PostWriteLocal => MetaValidationReferenceInputs {
            config_dir: None,
            language_codes: Vec::new(),
            proof_subject: None,
        },
    };
    meta_validate_source(
        text.as_bytes(),
        options,
        &reference_inputs,
        Some(resolved_path),
    )
}

fn meta_validate_source(
    bytes: &[u8],
    options: &MetaValidationOptions,
    reference_inputs: &MetaValidationReferenceInputs,
    artifact: Option<PathBuf>,
) -> Result<MetaValidationRun, String> {
    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    let source = std::str::from_utf8(bytes)
        .map_err(|error| format!("metadata image is not UTF-8: {error}"))?
        .trim_start_matches('\u{feff}');
    let resolved_path = artifact.unwrap_or_default();
    let doc = match Document::parse(source) {
        Ok(doc) => doc,
        Err(err) => {
            let mut report = MetaValidationReporter::new(options.max_errors, options.detailed);
            report.md_type = "(parse failed)".to_string();
            report.obj_name.clear();
            report.error(format!("1. XML parse failed: {err}"));
            return meta_validate_finish(report, resolved_path);
        }
    };

    let root = doc.root_element();
    let mut report = MetaValidationReporter::new(options.max_errors, options.detailed);
    let mut check1_ok = true;

    if root.tag_name().name() != "MetaDataObject" {
        report.error(format!(
            "1. Root element is '{}', expected 'MetaDataObject'",
            root.tag_name().name()
        ));
        return meta_validate_finish(report, resolved_path);
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
        Ok(compatibility) => report.warn(format_compatibility_warning(&compatibility)),
        Err(error) => report.error(error.to_string()),
    }
    let version = version_literal.as_deref().unwrap_or("");

    let child_elements = root
        .children()
        .filter(|child| child.is_element() && child.tag_name().namespace() == Some(MD_NS))
        .collect::<Vec<_>>();
    if child_elements.is_empty() {
        report.error("1. No metadata type element found inside MetaDataObject");
        return meta_validate_finish(report, resolved_path);
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
    report.md_type = md_type.to_string();
    if !meta_validate_valid_types().contains(&md_type) {
        report.error(format!("1. Unrecognized metadata type: {md_type}"));
        return meta_validate_finish(report, resolved_path);
    }

    let type_uuid = type_node.attribute("uuid").unwrap_or("");
    if type_uuid.is_empty() {
        report.error(format!("1. Missing uuid on <{md_type}> element"));
        check1_ok = false;
    } else if !is_guid(type_uuid) {
        report.error(format!("1. Invalid uuid '{type_uuid}' on <{md_type}>"));
        check1_ok = false;
    }

    let props_node = meta_info_child(type_node, "Properties");
    let name_node = props_node.and_then(|props| meta_info_child(props, "Name"));
    let obj_name = name_node
        .map(meta_info_inner_text)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "(unknown)".to_string());
    report.obj_name = obj_name.clone();

    if let Some(subject) = reference_inputs.proof_subject {
        meta_validate_check_subject_owner_proof(&mut report, md_type, &obj_name, subject);
    }

    if check1_ok {
        report.ok(format!(
            "1. Root structure: MetaDataObject/{md_type}, version {version}"
        ));
    }
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }

    meta_validate_check_internal_info(&mut report, md_type, type_node, &obj_name);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
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
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_property_values(&mut report, props_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_standard_attributes(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }

    let child_obj_node = meta_info_child(type_node, "ChildObjects");
    meta_validate_check_child_objects(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_child_elements(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_reserved_attr_names(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_uniqueness(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_tabular_sections(&mut report, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_cross_properties(
        &mut report,
        md_type,
        props_node,
        child_obj_node,
        reference_inputs.config_dir.as_deref(),
        reference_inputs.proof_subject,
        &obj_name,
    );
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_services(&mut report, md_type, child_obj_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_forbidden_properties(&mut report, md_type, props_node);
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_method_reference(
        &mut report,
        md_type,
        props_node,
        reference_inputs.config_dir.as_deref(),
        reference_inputs.proof_subject,
    );
    if report.stopped {
        return meta_validate_finish(report, resolved_path);
    }
    meta_validate_check_document_journal_columns(&mut report, md_type, child_obj_node);

    meta_validate_finish(report, resolved_path)
}

fn meta_validate_check_subject_owner_proof(
    report: &mut MetaValidationReporter,
    md_type: &str,
    obj_name: &str,
    subject: &MetadataValidationSubject,
) {
    if matches!(md_type, "ExternalReport" | "ExternalDataProcessor") {
        return;
    }
    let registrations = subject
        .resources
        .iter()
        .filter(|resource| matches!(resource.role, MetadataResourceRole::Registration))
        .filter_map(|resource| inspect_metadata_registration_image(&resource.bytes).ok())
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
        report.suppress_artifact = true;
        return;
    }
    if meta_validate_types_with_list_presentation().contains(&md_type)
        && registrations
            .iter()
            .all(|registration| registration.registered_languages.is_empty())
    {
        report.error("owner image has no registered language profile");
        report.suppress_artifact = true;
    }
}

pub(super) fn meta_validate_finish(
    report: MetaValidationReporter,
    artifact: PathBuf,
) -> Result<MetaValidationRun, String> {
    let suppress_artifact = report.suppress_artifact;
    let (ok, result_text, errors, warnings) = report.finalize();
    let artifacts = if artifact.as_os_str().is_empty() || suppress_artifact {
        Vec::new()
    } else {
        vec![artifact]
    };
    Ok(MetaValidationRun {
        ok,
        stdout: format!("{result_text}\n"),
        artifacts,
        errors,
        warnings,
    })
}

pub(super) fn meta_validate_localized_values(
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
                    .map(meta_info_inner_text)
            };
            let language = child_text("lang")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            let text = child_text("content").unwrap_or_default();
            (!text.trim().is_empty()).then_some((language, text))
        })
        .collect()
}

pub(super) fn meta_validate_check_internal_info(
    report: &mut MetaValidationReporter,
    md_type: &str,
    type_node: roxmltree::Node<'_, '_>,
    obj_name: &str,
) {
    let internal_info = meta_info_child(type_node, "InternalInfo");
    if meta_validate_types_without_internal_info().contains(&md_type) {
        if let Some(internal_info) = internal_info {
            let gen_types = meta_info_children(internal_info, "GeneratedType");
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
    let gen_types = meta_info_children(internal_info, "GeneratedType");
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
        if let Some(type_id) = meta_info_child(*generated_type, "TypeId") {
            if !is_guid(&meta_info_inner_text(type_id)) {
                report.error(format!(
                    "2. Invalid TypeId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
        if let Some(value_id) = meta_info_child(*generated_type, "ValueId") {
            if !is_guid(&meta_info_inner_text(value_id)) {
                report.error(format!(
                    "2. Invalid ValueId UUID in GeneratedType '{category}'"
                ));
                check_ok = false;
            }
        }
    }

    if md_type == "ExchangePlan" {
        if let Some(this_node) = meta_info_child(internal_info, "ThisNode") {
            if !is_guid(&meta_info_inner_text(this_node)) {
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

pub(super) fn meta_validate_check_properties(
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
        if !is_1c_identifier(obj_name) {
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
    let synonym_values = meta_validate_localized_values(meta_info_child(props_node, "Synonym"));
    let syn_present = !synonym_values.is_empty();

    if meta_validate_types_with_list_presentation().contains(&md_type) {
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

fn meta_validate_check_command_texts(
    report: &mut MetaValidationReporter,
    props_node: roxmltree::Node<'_, '_>,
    language_codes: &[String],
) {
    let synonyms = meta_validate_localized_values(meta_info_child(props_node, "Synonym"));
    let lists = meta_validate_localized_values(meta_info_child(props_node, "ListPresentation"));

    for language_code in language_codes {
        let list_values = lists
            .iter()
            .filter(|(language, text)| {
                language.as_deref() == Some(language_code.as_str()) && !text.trim().is_empty()
            })
            .collect::<Vec<_>>();
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
            meta_validate_warn_long_command_text(report, source, text, Some(language_code));
        }
    }
}

fn meta_validate_warn_long_command_text(
    report: &mut MetaValidationReporter,
    source: &str,
    text: &str,
    language: Option<&String>,
) {
    let length = text.chars().count();
    if length <= 38 {
        return;
    }
    let language_suffix = language
        .map(|language| format!(", language '{language}'"))
        .unwrap_or_default();
    report.warn(format!(
        "3. Properties: {source} '{text}' is longer than 38 characters ({length}) for the command interface{language_suffix}"
    ));
}

pub(super) fn meta_validate_check_property_values(
    report: &mut MetaValidationReporter,
    props_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(props_node) = props_node else {
        report.warn("4. No Properties block to check");
        return;
    };
    let mut enum_checked = 0usize;
    let mut check_ok = true;
    for (prop_name, allowed) in meta_validate_property_values() {
        if let Some(value) =
            meta_info_child_text(props_node, prop_name).filter(|value| !value.is_empty())
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

pub(super) fn meta_validate_check_standard_attributes(
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
    let Some(std_attr_node) = meta_info_child(props_node, "StandardAttributes") else {
        report.ok(format!(
            "5. StandardAttributes: absent (optional for {md_type})"
        ));
        return;
    };
    let std_attrs = meta_info_children(std_attr_node, "StandardAttribute");
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

pub(super) fn meta_validate_check_child_objects(
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

pub(super) fn meta_validate_check_child_elements(
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
        for element in meta_info_children(child_obj_node, kind) {
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

pub(super) fn meta_validate_check_child_element(
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
    if !is_guid(uuid) {
        report.error(format!("7. {kind} has invalid uuid '{uuid}'"));
        return false;
    }
    let Some(props) = meta_info_child(node, "Properties") else {
        report.error(format!("7. {kind} (uuid={uuid}) missing Properties"));
        return false;
    };
    let name = meta_info_child_text(props, "Name").unwrap_or_default();
    if name.is_empty() {
        report.error(format!("7. {kind} (uuid={uuid}) missing or empty Name"));
        return false;
    }
    if !is_1c_identifier(&name) {
        report.error(format!("7. {kind} '{name}' has invalid identifier"));
        return false;
    }
    if require_type {
        let Some(type_el) = meta_info_child(props, "Type") else {
            report.error(format!("7. {kind} '{name}' missing Type block"));
            return false;
        };
        if meta_info_children(type_el, "Type").is_empty()
            && meta_info_children(type_el, "TypeSet").is_empty()
        {
            report.error(format!(
                "7. {kind} '{name}' Type block has no v8:Type or v8:TypeSet"
            ));
            return false;
        }
    }
    true
}

pub(super) fn meta_validate_check_reserved_attr_names(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let mut check_ok = true;
    for attr_node in meta_info_children(child_obj_node, "Attribute") {
        if let Some(name) = meta_info_child(attr_node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
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

pub(super) fn meta_validate_check_uniqueness(
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
        if !meta_validate_names_unique(report, meta_info_children(child_obj_node, kind), kind) {
            check_ok = false;
        }
    }
    if check_ok {
        report.ok("8. Name uniqueness: all names unique");
    }
}

pub(super) fn meta_validate_names_unique(
    report: &mut MetaValidationReporter,
    nodes: Vec<roxmltree::Node<'_, '_>>,
    kind: &str,
) -> bool {
    let mut names = HashSet::<String>::new();
    let mut ok = true;
    for node in nodes {
        let Some(name) = meta_info_child(node, "Properties")
            .and_then(|props| meta_info_child_text(props, "Name"))
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

pub(super) fn meta_validate_check_tabular_sections(
    report: &mut MetaValidationReporter,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    let sections = meta_info_children(child_obj_node, "TabularSection");
    if sections.is_empty() {
        report.ok("9. TabularSections: none present");
        return;
    }
    let mut check_ok = true;
    for (index, section) in sections.iter().enumerate() {
        let count = index + 1;
        let uuid = section.attribute("uuid").unwrap_or("");
        if uuid.is_empty() || !is_guid(uuid) {
            report.error(format!(
                "9. TabularSection #{count}: invalid or missing uuid"
            ));
            check_ok = false;
        }
        let props = meta_info_child(*section, "Properties");
        let section_name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "(unnamed)".to_string());
        if section_name == "(unnamed)" {
            report.error(format!("9. TabularSection #{count}: missing or empty Name"));
            check_ok = false;
        }
        if let Some(internal_info) = meta_info_child(*section, "InternalInfo") {
            let generated = meta_info_children(internal_info, "GeneratedType");
            if generated.len() < 2 {
                report.warn(format!(
                    "9. TabularSection '{section_name}': expected 2 GeneratedType, found {}",
                    generated.len()
                ));
            }
        }
        if let Some(ts_child_obj) = meta_info_child(*section, "ChildObjects") {
            let mut names = HashSet::<String>::new();
            for attr in meta_info_children(ts_child_obj, "Attribute") {
                if !meta_validate_check_child_element(
                    report,
                    attr,
                    &format!("TabularSection '{section_name}'.Attribute"),
                    true,
                ) {
                    check_ok = false;
                }
                if let Some(name) = meta_info_child(attr, "Properties")
                    .and_then(|node| meta_info_child_text(node, "Name"))
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
                if let Some(std_attr) = meta_info_child(props, "StandardAttributes") {
                    let has_line_number = meta_info_children(std_attr, "StandardAttribute")
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

pub(super) fn meta_validate_check_cross_properties(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
    proof_subject: Option<&MetadataValidationSubject>,
    obj_name: &str,
) {
    let Some(props_node) = props_node else {
        return;
    };
    let mut check_ok = true;
    let mut issues = 0usize;
    if meta_info_child_text(props_node, "Hierarchical").as_deref() == Some("false") {
        if let Some(hierarchy_type) =
            meta_info_child_text(props_node, "HierarchyType").filter(|value| !value.is_empty())
        {
            report.warn(format!(
                "10. HierarchyType='{hierarchy_type}' but Hierarchical=false"
            ));
            issues += 1;
        }
    }
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
        .any(|name| meta_info_child_text(props_node, name).as_deref() == Some("true"));
        if !any_enabled {
            report.warn("10. CommonModule: no execution context enabled");
            issues += 1;
        }
    }
    if md_type == "EventSubscription" {
        if meta_info_child_text(props_node, "Handler").is_none_or(|value| value.trim().is_empty()) {
            report.error("10. EventSubscription: empty Handler");
            check_ok = false;
            issues += 1;
        }
        let has_source = meta_info_child(props_node, "Source")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_source {
            report.warn("10. EventSubscription: no Source types specified");
            issues += 1;
        }
    }
    if md_type == "ScheduledJob"
        && meta_info_child_text(props_node, "MethodName")
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
            && meta_info_child_text(props_node, property)
                .is_none_or(|value| value.trim().is_empty())
        {
            report.error(message);
            check_ok = false;
            issues += 1;
        }
    }
    if md_type == "BusinessProcess"
        && meta_info_child_text(props_node, "Task").is_none_or(|value| value.trim().is_empty())
    {
        report.warn("10. BusinessProcess: empty Task reference");
        issues += 1;
    }
    if md_type == "CalculationRegister"
        && meta_info_child_text(props_node, "ActionPeriod").as_deref() == Some("true")
        && meta_info_child_text(props_node, "Schedule").is_none_or(|value| value.trim().is_empty())
    {
        report.warn(
            "10. CalculationRegister: ActionPeriod=true but Schedule is empty — platform requires a schedule register",
        );
        issues += 1;
    }
    if md_type == "DocumentJournal" {
        let has_registered = meta_info_child(props_node, "RegisteredDocuments")
            .map(|node| !meta_info_children(node, "Type").is_empty())
            .unwrap_or(false);
        if !has_registered {
            report.warn("10. DocumentJournal: no RegisteredDocuments specified");
            issues += 1;
        }
    }
    if md_type == "ChartOfAccounts" {
        let max_ext_dim = meta_info_child_text(props_node, "MaxExtDimensionCount")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        if max_ext_dim > 0
            && meta_info_child_text(props_node, "ExtDimensionTypes")
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
            let count = meta_info_children(child_obj_node, "Dimension").len()
                + meta_info_children(child_obj_node, "Resource").len()
                + meta_info_children(child_obj_node, "Attribute").len();
            if count == 0 {
                report.warn(format!(
                    "10. {md_type}: no Dimensions, Resources, or Attributes — platform will reject"
                ));
                issues += 1;
            }
        }
    }
    if md_type == "InformationRegister"
        && meta_info_child_text(props_node, "WriteMode").as_deref() == Some("RecorderSubordinate")
        && meta_info_child_text(props_node, "UseStandardCommands").as_deref() == Some("true")
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
    if check_ok && issues == 0 {
        report.ok("10. Cross-property consistency");
    }
}

pub(super) fn meta_validate_check_document_register_records(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    proof_subject: Option<&MetadataValidationSubject>,
    issues: &mut usize,
) {
    if md_type != "Document" {
        return;
    }
    let Some(register_records) = meta_info_child(props_node, "RegisterRecords") else {
        return;
    };
    for item in meta_info_children(register_records, "Item") {
        let ref_value = meta_info_inner_text(item).trim().to_string();
        let Some((ref_type, ref_name)) = ref_value.split_once('.') else {
            continue;
        };
        if let Some(subject) = proof_subject {
            let found = subject.resources.iter().any(|resource| {
                matches!(
                    &resource.role,
                    MetadataResourceRole::Dependency { target }
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
        let Some(config_dir) = config_dir else {
            continue;
        };
        let ref_dir = match ref_type {
            "AccumulationRegister" => "AccumulationRegisters",
            "InformationRegister" => "InformationRegisters",
            "AccountingRegister" => "AccountingRegisters",
            "CalculationRegister" => "CalculationRegisters",
            _ => continue,
        };
        let ref_path = config_dir.join(ref_dir).join(ref_name);
        let ref_xml = config_dir.join(ref_dir).join(format!("{ref_name}.xml"));
        if !ref_path.exists() && !ref_xml.exists() {
            report.warn(format!(
                "10. Document.RegisterRecords references '{ref_value}' but object not found in config"
            ));
            *issues += 1;
        }
    }
}

pub(super) fn meta_validate_check_register_registrar(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: roxmltree::Node<'_, '_>,
    config_dir: Option<&Path>,
    proof_subject: Option<&MetadataValidationSubject>,
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
        && meta_info_child_text(props_node, "WriteMode").as_deref() != Some("RecorderSubordinate")
    {
        return;
    }
    let reg_ref = format!("{md_type}.{obj_name}");
    let has_registrar = if let Some(subject) = proof_subject {
        subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                MetadataResourceRole::Dependency { target }
                    if target.segments().next() == Some("Document")
            ) && image_contains_reference(&resource.bytes, &reg_ref)
        })
    } else if let Some(config_dir) = config_dir {
        let docs_dir = config_dir.join("Documents");
        docs_dir.is_dir()
            && meta_validate_registrar_document_scan(&docs_dir, &reg_ref)
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

pub(super) fn meta_validate_check_services(
    report: &mut MetaValidationReporter,
    md_type: &str,
    child_obj_node: Option<roxmltree::Node<'_, '_>>,
) {
    let Some(child_obj_node) = child_obj_node else {
        return;
    };
    if md_type == "HTTPService" {
        let templates = meta_info_children(child_obj_node, "URLTemplate");
        let mut check_ok = true;
        let mut method_count = 0usize;
        for template in &templates {
            let props = meta_info_child(*template, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "Template"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.error(format!(
                    "11. HTTPService URLTemplate '{name}': empty Template"
                ));
                check_ok = false;
            }
            if let Some(child_objects) = meta_info_child(*template, "ChildObjects") {
                for method in meta_info_children(child_objects, "Method") {
                    method_count += 1;
                    let props = meta_info_child(method, "Properties");
                    let http_method =
                        props.and_then(|node| meta_info_child_text(node, "HTTPMethod"));
                    if let Some(http_method) = http_method.filter(|value| !value.is_empty()) {
                        if !meta_validate_valid_http_methods().contains(&http_method.as_str()) {
                            report.error(format!(
                                "11. HTTPService URLTemplate '{name}': invalid HTTPMethod '{http_method}'"
                            ));
                            check_ok = false;
                        }
                    } else {
                        report.error(format!(
                            "11. HTTPService URLTemplate '{name}': Method missing HTTPMethod"
                        ));
                        check_ok = false;
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. HTTPService: {} URLTemplate(s), {method_count} method(s)",
                templates.len()
            ));
        }
    } else if md_type == "WebService" {
        let operations = meta_info_children(child_obj_node, "Operation");
        let mut check_ok = true;
        let mut param_count = 0usize;
        for operation in &operations {
            let props = meta_info_child(*operation, "Properties");
            let name = props
                .and_then(|node| meta_info_child_text(node, "Name"))
                .unwrap_or_else(|| "(unnamed)".to_string());
            if props
                .and_then(|node| meta_info_child_text(node, "XDTOReturningValueType"))
                .is_none_or(|value| value.trim().is_empty())
            {
                report.warn(format!(
                    "11. WebService Operation '{name}': no XDTOReturningValueType"
                ));
            }
            if let Some(child_objects) = meta_info_child(*operation, "ChildObjects") {
                for param in meta_info_children(child_objects, "Parameter") {
                    param_count += 1;
                    let direction = meta_info_child(param, "Properties")
                        .and_then(|node| meta_info_child_text(node, "TransferDirection"));
                    if let Some(direction) = direction.filter(|value| !value.is_empty()) {
                        if !["In", "Out", "InOut"].contains(&direction.as_str()) {
                            report.error(format!(
                                "11. WebService Operation '{name}': Parameter has invalid TransferDirection '{direction}'"
                            ));
                            check_ok = false;
                        }
                    }
                }
            }
        }
        if check_ok {
            report.ok(format!(
                "11. WebService: {} operation(s), {param_count} parameter(s)",
                operations.len()
            ));
        }
    }
}

pub(super) fn meta_validate_check_forbidden_properties(
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
        if meta_info_child(props_node, property).is_some() {
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

pub(super) fn meta_validate_check_method_reference(
    report: &mut MetaValidationReporter,
    md_type: &str,
    props_node: Option<roxmltree::Node<'_, '_>>,
    config_dir: Option<&Path>,
    proof_subject: Option<&MetadataValidationSubject>,
) {
    if !matches!(md_type, "EventSubscription" | "ScheduledJob") {
        return;
    }
    let Some(props_node) = props_node else {
        return;
    };
    let (property, method_ref) = if md_type == "EventSubscription" {
        ("Handler", meta_info_child_text(props_node, "Handler"))
    } else {
        ("MethodName", meta_info_child_text(props_node, "MethodName"))
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
    if let Some(subject) = proof_subject {
        let module_reference = format!("CommonModule.{module_name}");
        let module_target =
            MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, &module_reference)
                .expect("validated CommonModule metadata address");
        let descriptor_found = subject.resources.iter().any(|resource| {
            matches!(
                &resource.role,
                MetadataResourceRole::Dependency { target } if target == &module_target
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
                MetadataResourceRole::Module { owner } if owner == &module_target
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
        if !meta_validate_bsl_has_export(content, proc_name) {
            report.error(format!(
                "13. {md_type}.{property}: procedure '{proc_name}' not found as exported in CommonModule '{module_name}'"
            ));
            return;
        }
        report.ok(format!("13. Method reference: {property} = '{method_ref}'"));
        return;
    }
    let Some(config_dir) = config_dir else {
        return;
    };
    let module_xml = config_dir
        .join("CommonModules")
        .join(format!("{module_name}.xml"));
    if !module_xml.exists() {
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
    if bsl_path.exists() {
        if let Ok(content) = read_utf8_sig(&bsl_path) {
            if !meta_validate_bsl_has_export(&content, proc_name) {
                report.warn(format!(
                    "13. {md_type}.{property}: procedure '{proc_name}' not found as exported in CommonModule '{module_name}'"
                ));
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

pub(super) fn meta_validate_check_document_journal_columns(
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
    let columns = meta_info_children(child_obj_node, "Column");
    let mut check_ok = true;
    let mut empty_ref_count = 0usize;
    for column in &columns {
        let props = meta_info_child(*column, "Properties");
        let name = props
            .and_then(|node| meta_info_child_text(node, "Name"))
            .unwrap_or_else(|| "(unnamed)".to_string());
        let has_items = props
            .and_then(|node| meta_info_child(node, "References"))
            .map(|node| !meta_info_children(node, "Item").is_empty())
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

pub(super) fn meta_validate_bsl_has_export(content: &str, proc_name: &str) -> bool {
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

pub(super) fn is_guid(value: &str) -> bool {
    let bytes = value.as_bytes();
    value.len() == 36
        && [8, 13, 18, 23].iter().all(|index| bytes[*index] == b'-')
        && value
            .chars()
            .enumerate()
            .all(|(index, ch)| [8, 13, 18, 23].contains(&index) || ch.is_ascii_hexdigit())
}

pub(super) fn meta_validate_valid_types() -> &'static [&'static str] {
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

pub(super) fn meta_validate_generated_categories(md_type: &str) -> Option<&'static [&'static str]> {
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

pub(super) fn meta_validate_types_without_internal_info() -> &'static [&'static str] {
    &["CommonModule", "ScheduledJob", "EventSubscription"]
}

pub(super) fn meta_validate_types_with_std_attrs() -> &'static [&'static str] {
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

pub(super) fn meta_validate_standard_attributes(md_type: &str) -> Option<&'static [&'static str]> {
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

pub(super) fn meta_validate_dynamic_standard_attr(md_type: &str, name: &str) -> bool {
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

pub(super) fn meta_validate_child_rules(md_type: &str) -> Option<Vec<&'static str>> {
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
        .map(|collection| match collection {
            MetaCollection::Attributes => "Attribute",
            MetaCollection::TabularSections => "TabularSection",
            MetaCollection::Dimensions => "Dimension",
            MetaCollection::Resources => "Resource",
            MetaCollection::EnumValues => "EnumValue",
            MetaCollection::Columns => "Column",
            MetaCollection::Forms => "Form",
            MetaCollection::Templates => "Template",
            MetaCollection::Commands => "Command",
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

pub(super) fn meta_validate_property_values() -> &'static [(&'static str, &'static [&'static str])]
{
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

pub(super) fn meta_validate_reserved_attr_names() -> &'static [&'static str] {
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

pub(super) fn meta_validate_valid_http_methods() -> &'static [&'static str] {
    &[
        "GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "MERGE", "CONNECT",
    ]
}

pub(super) fn meta_validate_forbidden_properties(md_type: &str) -> Option<&'static [&'static str]> {
    match md_type {
        "ChartOfCharacteristicTypes" => Some(&["CodeType"]),
        "ChartOfAccounts" => Some(&["Autonumbering", "Hierarchical"]),
        "ChartOfCalculationTypes" => Some(&["CheckUnique", "Autonumbering"]),
        "ExchangePlan" => Some(&["CodeType", "CheckUnique", "Autonumbering"]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::{
        MetadataAuxiliaryXmlKind, MetadataChildFootprintEvidence, MetadataResourceImage,
        MetadataResourceRole, MetadataValidationSubject,
    };
    use crate::domain::metadata::{MetaDiagnosticCode, MetaValidationStatus};
    use crate::domain::source_target::{MetadataAddress, PLATFORM_XML_8_3_27_FORMAT_2_20};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const MD_NS: &str = "http://v8.1c.ru/8.3/MDClasses";

    fn context() -> WorkspaceContext {
        WorkspaceContext {
            cwd: PathBuf::from("/workspace"),
            workspace_root: PathBuf::from("/workspace"),
            cache_root: PathBuf::from("/workspace/.unica/cache"),
            workspace_epoch: 1,
        }
    }

    fn temp_context(name: &str) -> WorkspaceContext {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("unica-validator-{name}-{nanos}"));
        fs::create_dir_all(&root).unwrap();
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        }
    }

    fn address(value: &str) -> MetadataAddress {
        MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, value).unwrap()
    }

    fn image(role: MetadataResourceRole, xml: impl Into<Vec<u8>>) -> MetadataResourceImage {
        MetadataResourceImage {
            role,
            bytes: xml.into(),
        }
    }

    fn subject(
        target: &str,
        descriptor: Option<&str>,
        registration: &str,
        dependencies: &[(&str, &str)],
    ) -> MetadataValidationSubject {
        let mut resources = Vec::new();
        if let Some(descriptor) = descriptor {
            resources.push(image(
                MetadataResourceRole::Descriptor,
                descriptor.as_bytes().to_vec(),
            ));
        }
        resources.push(image(
            MetadataResourceRole::Registration,
            registration.as_bytes().to_vec(),
        ));
        resources.extend(dependencies.iter().map(|(target, dependency)| {
            image(
                MetadataResourceRole::Dependency {
                    target: address(target),
                },
                dependency.as_bytes().to_vec(),
            )
        }));
        MetadataValidationSubject {
            target: address(target),
            resources,
            child_footprints: Vec::new(),
        }
    }

    fn common_module(name: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<CommonModule uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>{name}</Name></Properties><ChildObjects/>
</CommonModule></MetaDataObject>"#
        )
    }

    fn scheduled_job(name: &str, method: &str) -> String {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<ScheduledJob uuid="55555555-5555-4555-8555-555555555555">
<Properties><Name>{name}</Name><MethodName>{method}</MethodName></Properties><ChildObjects/>
</ScheduledJob></MetaDataObject>"#
        )
    }

    fn owner(registrations: &[(&str, &str)]) -> String {
        let registrations = registrations
            .iter()
            .map(|(kind, name)| format!("<{kind}>{name}</{kind}>"))
            .collect::<String>();
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<Configuration uuid="22222222-2222-4222-8222-222222222222">
<Properties><Name>Owner</Name></Properties><ChildObjects>{registrations}</ChildObjects>
</Configuration></MetaDataObject>"#
        )
    }

    fn dependency_with_reference(reference: Option<&str>) -> String {
        let content = reference
            .map(|reference| format!("<Content><Item>{reference}</Item></Content>"))
            .unwrap_or_default();
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<Subsystem uuid="33333333-3333-4333-8333-333333333333">
<Properties><Name>Area</Name>{content}</Properties><ChildObjects/>
</Subsystem></MetaDataObject>"#
        )
    }

    fn metadata_collection(kind: &str) -> &'static str {
        match kind {
            "CommonModule" => "CommonModules",
            "ScheduledJob" => "ScheduledJobs",
            "HTTPService" => "HTTPServices",
            "DocumentJournal" => "DocumentJournals",
            "InformationRegister" => "InformationRegisters",
            "Document" => "Documents",
            "Language" => "Languages",
            "Subsystem" => "Subsystems",
            other => panic!("missing test collection for {other}"),
        }
    }

    fn metadata_path(root: &Path, target: &MetadataAddress) -> PathBuf {
        let mut segments = target.segments();
        let kind = segments.next().unwrap();
        let name = segments.next().unwrap();
        root.join(metadata_collection(kind))
            .join(format!("{name}.xml"))
    }

    fn write_bytes(path: &Path, bytes: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, bytes).unwrap();
    }

    fn real_legacy_outcome(name: &str, subject: &MetadataValidationSubject) -> AdapterOutcome {
        let context = temp_context(name);
        let descriptor_path = metadata_path(&context.cwd, &subject.target);
        for resource in &subject.resources {
            match &resource.role {
                MetadataResourceRole::Descriptor => write_bytes(&descriptor_path, &resource.bytes),
                MetadataResourceRole::Registration => {
                    write_bytes(&context.cwd.join("Configuration.xml"), &resource.bytes)
                }
                MetadataResourceRole::Dependency { target } => {
                    write_bytes(&metadata_path(&context.cwd, target), &resource.bytes)
                }
                MetadataResourceRole::Module { owner } => {
                    let mut segments = owner.segments();
                    let kind = segments.next().unwrap();
                    let object_name = segments.next().unwrap();
                    write_bytes(
                        &context
                            .cwd
                            .join(metadata_collection(kind))
                            .join(object_name)
                            .join("Ext/Module.bsl"),
                        &resource.bytes,
                    );
                }
                MetadataResourceRole::Form { owner, name }
                | MetadataResourceRole::Template { owner, name }
                | MetadataResourceRole::Command { owner, name } => {
                    let mut segments = owner.segments();
                    let kind = segments.next().unwrap();
                    let object_name = segments.next().unwrap();
                    let collection = match &resource.role {
                        MetadataResourceRole::Form { .. } => "Forms",
                        MetadataResourceRole::Template { .. } => "Templates",
                        MetadataResourceRole::Command { .. } => "Commands",
                        _ => unreachable!(),
                    };
                    write_bytes(
                        &context
                            .cwd
                            .join(metadata_collection(kind))
                            .join(object_name)
                            .join(collection)
                            .join(format!("{name}.xml")),
                        &resource.bytes,
                    );
                }
                MetadataResourceRole::AuxiliaryXml { owner, kind } => {
                    let mut segments = owner.segments();
                    let object_kind = segments.next().unwrap();
                    let object_name = segments.next().unwrap();
                    let file_name = match kind {
                        MetadataAuxiliaryXmlKind::ExchangePlanContent => "Content.xml",
                        MetadataAuxiliaryXmlKind::BusinessProcessFlowchart => "Flowchart.xml",
                    };
                    write_bytes(
                        &context
                            .cwd
                            .join(metadata_collection(object_kind))
                            .join(object_name)
                            .join("Ext")
                            .join(file_name),
                        &resource.bytes,
                    );
                }
                MetadataResourceRole::ChildResource { .. } => {}
            }
        }
        let outcome = validate_meta(&meta_validate_args(&descriptor_path), &context);
        let _ = fs::remove_dir_all(&context.cwd);
        outcome
    }

    fn meta_validate_args(path: &Path) -> Map<String, Value> {
        Map::from_iter([
            (
                "ObjectPath".to_string(),
                Value::String(path.display().to_string()),
            ),
            ("Detailed".to_string(), Value::Bool(true)),
        ])
    }

    fn assert_real_legacy_equivalent(
        name: &str,
        subject: &MetadataValidationSubject,
        expected: MetaValidationStatus,
    ) -> crate::domain::metadata::MetaValidationData {
        let context = context();
        let internal = MetadataValidator.validate(subject, &context);
        let legacy = real_legacy_outcome(name, subject);
        assert_eq!(internal.status, expected);
        assert_eq!(legacy.ok, expected == MetaValidationStatus::Passed);
        internal
    }

    fn assert_report_summary_matches_lines(stdout: &str) {
        let summary = stdout
            .lines()
            .find(|line| line.starts_with("=== Result:"))
            .unwrap_or_else(|| panic!("missing reporter summary: {stdout}"));
        let parts = summary.split_whitespace().collect::<Vec<_>>();
        let reported_errors = parts[2].parse::<usize>().unwrap();
        let reported_warnings = parts[4].parse::<usize>().unwrap();
        let reported_checks = parts[6].trim_start_matches('(').parse::<usize>().unwrap();
        let actual_errors = stdout
            .lines()
            .filter(|line| line.starts_with("[ERROR] "))
            .count();
        let actual_warnings = stdout
            .lines()
            .filter(|line| line.starts_with("[WARN]  "))
            .count();
        let actual_checks = stdout
            .lines()
            .filter(|line| {
                line.starts_with("[OK]    ")
                    || line.starts_with("[ERROR] ")
                    || line.starts_with("[WARN]  ")
            })
            .count();
        assert_eq!(reported_errors, actual_errors, "{stdout}");
        assert_eq!(reported_warnings, actual_warnings, "{stdout}");
        assert_eq!(reported_checks, actual_checks, "{stdout}");
    }

    #[test]
    fn retained_form_content_rejects_wrong_namespace_and_export_version() {
        for (label, bytes) in [
            (
                "namespace",
                br#"<Form xmlns="urn:not-logform" version="2.20"/>"#.as_slice(),
            ),
            (
                "version",
                br#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.21"/>"#.as_slice(),
            ),
            ("malformed", br#"<Form"#.as_slice()),
            (
                "root",
                br#"<NotForm xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.as_slice(),
            ),
            (
                "encoding",
                br#"<?xml version="1.0" encoding="windows-1251"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#
                    .as_slice(),
            ),
        ] {
            let subject = MetadataValidationSubject {
                target: address("Catalog.Editable"),
                resources: vec![image(
                    MetadataResourceRole::ChildResource {
                        child: address("Catalog.Editable.Form.Main"),
                        kind: MetadataChildResourceKind::FormContent,
                        ordinal: 0,
                    },
                    bytes,
                )],
                child_footprints: Vec::new(),
            };

            let result = MetadataValidator.validate(&subject, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed, "{label}");
            assert_eq!(
                result.diagnostics[0].field.as_deref(),
                Some("resources[0].bytes"),
                "{label}"
            );
        }
    }

    fn template_subject(
        child: &str,
        resources: Vec<(MetadataTemplateType, MetadataTemplateResourcePart, Vec<u8>)>,
    ) -> MetadataValidationSubject {
        let child_address = address(child);
        let name = child_address.segments().last().unwrap().to_string();
        let template_type = resources.first().unwrap().0;
        let template_type_value = match template_type {
            MetadataTemplateType::HtmlDocument => "HTMLDocument",
            MetadataTemplateType::TextDocument => "TextDocument",
            MetadataTemplateType::SpreadsheetDocument => "SpreadsheetDocument",
            MetadataTemplateType::BinaryData => "BinaryData",
            MetadataTemplateType::DataCompositionSchema => "DataCompositionSchema",
        };
        let descriptor = typed_child_xml("Template", &name, Some(template_type_value));
        let directories = if template_type == MetadataTemplateType::HtmlDocument {
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::HtmlPages,
            ]
        } else {
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
            ]
        };
        let kinds = resources
            .iter()
            .enumerate()
            .map(|(ordinal, (template_type, part, _))| {
                (
                    MetadataChildResourceKind::TemplateContent {
                        template_type: *template_type,
                        part: *part,
                    },
                    ordinal,
                )
            })
            .collect();
        let mut subject = closed_child_subject(
            "Template",
            &name,
            Some(template_type_value),
            &descriptor,
            directories,
            kinds,
        );
        for (ordinal, (_, _, bytes)) in resources.into_iter().enumerate() {
            if let Some(resource) = subject.resources.iter_mut().find(|resource| {
                matches!(
                    resource.role,
                    MetadataResourceRole::ChildResource {
                        ordinal: candidate,
                        ..
                    } if candidate == ordinal
                )
            }) {
                resource.bytes = bytes;
            }
        }
        subject
    }

    fn typed_child_xml(kind: &str, name: &str, template_type: Option<&str>) -> String {
        let template_type = template_type
            .map(|value| format!("<TemplateType>{value}</TemplateType>"))
            .unwrap_or_default();
        format!(
            r#"<{kind} uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>{name}</Name>{template_type}</Properties><ChildObjects/></{kind}>"#
        )
    }

    fn typed_owner_descriptor(children: &[String]) -> Vec<u8> {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Catalog uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Editable</Name></Properties><ChildObjects>{}</ChildObjects></Catalog></MetaDataObject>"#,
            children.join("")
        )
        .into_bytes()
    }

    fn typed_owner_descriptor_with_structure(structure: &str) -> Vec<u8> {
        format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><Catalog uuid="66666666-6666-4666-8666-666666666666"><Properties><Name>Editable</Name></Properties>{structure}</Catalog></MetaDataObject>"#
        )
        .into_bytes()
    }

    fn typed_child_descriptor(child: &str) -> Vec<u8> {
        format!(r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">{child}</MetaDataObject>"#)
            .into_bytes()
    }

    fn closed_child_subject(
        kind: &str,
        name: &str,
        template_type: Option<&str>,
        descriptor_child: &str,
        directories: Vec<MetadataChildDirectoryKind>,
        child_resources: Vec<(MetadataChildResourceKind, usize)>,
    ) -> MetadataValidationSubject {
        let owner = address("Catalog.Editable");
        let expected_child = typed_child_xml(kind, name, template_type);
        let child = address(&format!("Catalog.Editable.{kind}.{name}"));
        let profile = match kind {
            "Form" => MetadataChildProfile::Form,
            "Command" => MetadataChildProfile::Command,
            "Template" => MetadataChildProfile::Template(
                MetadataTemplateType::from_descriptor_value(template_type.unwrap()).unwrap(),
            ),
            _ => unreachable!(),
        };
        let descriptor_role = match kind {
            "Form" => MetadataResourceRole::Form {
                owner: owner.clone(),
                name: name.to_string(),
            },
            "Template" => MetadataResourceRole::Template {
                owner: owner.clone(),
                name: name.to_string(),
            },
            "Command" => MetadataResourceRole::Command {
                owner,
                name: name.to_string(),
            },
            _ => unreachable!(),
        };
        let mut resources = vec![
            image(
                MetadataResourceRole::Descriptor,
                typed_owner_descriptor(&[expected_child]),
            ),
            image(descriptor_role, typed_child_descriptor(descriptor_child)),
        ];
        resources.extend(child_resources.into_iter().map(|(kind, ordinal)| {
            image(
                MetadataResourceRole::ChildResource {
                    child: child.clone(),
                    kind,
                    ordinal,
                },
                Vec::new(),
            )
        }));
        MetadataValidationSubject {
            target: address("Catalog.Editable"),
            resources,
            child_footprints: vec![MetadataChildFootprintEvidence {
                child,
                profile,
                directories,
            }],
        }
    }

    fn closed_child_diagnostics(subject: &MetadataValidationSubject) -> Vec<String> {
        let mut messages = subject
            .resources
            .iter()
            .filter_map(|resource| match resource.role {
                MetadataResourceRole::ChildResource { kind, .. } => {
                    validate_child_resource(kind, &resource.bytes).err()
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut diagnostics = Vec::new();
        validate_child_footprints(subject, &mut diagnostics);
        messages.extend(diagnostics.into_iter().map(|diagnostic| diagnostic.message));
        messages
    }

    fn closed_validator_subject(
        kind: &str,
        template_type: Option<&str>,
        owner_entry: &str,
        descriptor_child: &str,
    ) -> MetadataValidationSubject {
        let (directories, resources) = match kind {
            "Form" => (
                vec![
                    MetadataChildDirectoryKind::Root,
                    MetadataChildDirectoryKind::Extension,
                ],
                vec![(MetadataChildResourceKind::FormContent, 0)],
            ),
            "Template" => (
                vec![
                    MetadataChildDirectoryKind::Root,
                    MetadataChildDirectoryKind::Extension,
                ],
                vec![(
                    MetadataChildResourceKind::TemplateContent {
                        template_type: MetadataTemplateType::TextDocument,
                        part: MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            "Command" => (Vec::new(), Vec::new()),
            _ => unreachable!(),
        };
        let mut subject = closed_child_subject(
            kind,
            "Main",
            template_type,
            descriptor_child,
            directories,
            resources,
        );
        subject.resources[0].bytes = typed_owner_descriptor(&[owner_entry.to_string()]);
        for resource in &mut subject.resources {
            if let MetadataResourceRole::ChildResource { kind, .. } = resource.role {
                resource.bytes = match kind {
                    MetadataChildResourceKind::FormContent => br#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"/>"#.to_vec(),
                    MetadataChildResourceKind::TemplateContent { .. } => b"text".to_vec(),
                    MetadataChildResourceKind::Module => unreachable!(),
                };
            }
        }
        subject.resources.push(image(
            MetadataResourceRole::Registration,
            owner(&[("Catalog", "Editable")]),
        ));
        subject
    }

    fn assert_exact_closed_diagnostic(subject: &MetadataValidationSubject, expected: &str) {
        let result = MetadataValidator.validate(subject, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed, "{result:?}");
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.message == expected)
            .unwrap_or_else(|| panic!("missing `{expected}` in {:?}", result.diagnostics));
        assert_eq!(diagnostic.code, MetaDiagnosticCode::ProviderUnavailable);
        assert_eq!(diagnostic.metadata_path, Some(subject.target.clone()));
        assert_eq!(diagnostic.field.as_deref(), Some("resources"));
        assert_eq!(diagnostic.operation_index, None);
        assert!(!diagnostic.message.contains("/workspace"));
    }

    fn closed_owner_subject(descriptor: &str) -> MetadataValidationSubject {
        subject(
            "CommonModule.Service",
            Some(descriptor),
            &owner(&[("CommonModule", "Service")]),
            &[],
        )
    }

    #[test]
    fn closed_validator_rejects_foreign_properties_in_the_owner_descriptor() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><x:Properties xmlns:x="urn:foreign"><x:Name>Service</x:Name></x:Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Properties outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_name_in_exact_owner_properties() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><x:Name xmlns:x="urn:foreign">Service</x:Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Name outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_owner_properties_duplicates() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Service</Name></Properties><x:Properties xmlns:x="urn:foreign"><x:Name>Service</x:Name></x:Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Properties outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_owner_name_duplicates() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20"><CommonModule uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Service</Name><x:Name xmlns:x="urn:foreign">Service</x:Name></Properties><ChildObjects/></CommonModule></MetaDataObject>"#
        );
        let subject = closed_owner_subject(&descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains Name outside the MDClasses namespace: CommonModule.Service",
        );
    }

    #[test]
    fn closed_validator_accepts_a_canonical_mdclasses_owner_identity() {
        let descriptor = common_module("Service");
        let subject = closed_owner_subject(&descriptor);

        let result = MetadataValidator.validate(&subject, &context());

        assert_eq!(result.status, MetaValidationStatus::Passed, "{result:?}");
    }

    #[test]
    fn closed_validator_rejects_foreign_childobjects_in_the_final_owner_graph() {
        let child = typed_child_xml("Command", "Main", None);
        let owner_entry =
            format!(r#"<x:ChildObjects xmlns:x="urn:foreign">{child}</x:ChildObjects>"#);
        let descriptor = typed_child_xml("Command", "Main", None);
        let mut subject = closed_validator_subject(
            "Command",
            None,
            &typed_child_xml("Command", "Main", None),
            &descriptor,
        );
        subject.resources[0].bytes = typed_owner_descriptor_with_structure(&owner_entry);

        assert_exact_closed_diagnostic(
            &subject,
            "owner descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_properties_in_each_child_descriptor_kind() {
        for (kind, template_type, foreign_properties) in [
            (
                "Form",
                None,
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties>"#,
            ),
            (
                "Template",
                Some("TextDocument"),
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name><x:TemplateType>TextDocument</x:TemplateType></x:Properties>"#,
            ),
            (
                "Command",
                None,
                r#"<x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties>"#,
            ),
        ] {
            let owner_entry = typed_child_xml(kind, "Main", template_type);
            let descriptor = format!(
                r#"<{kind} uuid="77777777-7777-4777-8777-777777777777">{foreign_properties}<ChildObjects/></{kind}>"#
            );
            let subject = closed_validator_subject(kind, template_type, &owner_entry, &descriptor);

            assert_exact_closed_diagnostic(
                &subject,
                &format!(
                    "declared child descriptor is invalid: {kind} descriptor contains Properties outside the MDClasses namespace: Catalog.Editable"
                ),
            );
        }
    }

    #[test]
    fn closed_validator_rejects_foreign_name_under_exact_properties() {
        let owner_entry = typed_child_xml("Form", "Main", None);
        let descriptor = r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><x:Name xmlns:x="urn:foreign">Main</x:Name></Properties><ChildObjects/></Form>"#;
        let subject = closed_validator_subject("Form", None, &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Form descriptor contains Name outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_templatetype_under_exact_template_properties() {
        let owner_entry = typed_child_xml("Template", "Main", Some("TextDocument"));
        let descriptor = r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#;
        let subject =
            closed_validator_subject("Template", Some("TextDocument"), &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_childobjects_in_a_child_descriptor() {
        let owner_entry = typed_child_xml("Form", "Main", None);
        let descriptor = r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name></Properties><x:ChildObjects xmlns:x="urn:foreign"/></Form>"#;
        let subject = closed_validator_subject("Form", None, &owner_entry, descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: Form descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_foreign_templatetype_in_an_inline_owner_entry() {
        let owner_entry = r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#;
        let descriptor = typed_child_xml("Template", "Main", Some("TextDocument"));
        let subject =
            closed_validator_subject("Template", Some("TextDocument"), owner_entry, &descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_an_additional_foreign_root_artifact() {
        let owner_entry = typed_child_xml("Command", "Main", None);
        let valid = typed_child_xml("Command", "Main", None);
        let descriptor = format!(r#"{valid}<x:Ignored xmlns:x="urn:foreign"/>"#);
        let subject = closed_validator_subject("Command", None, &owner_entry, &descriptor);

        assert_exact_closed_diagnostic(
            &subject,
            "declared child descriptor is invalid: metadata descriptor must contain exactly one element artifact: Catalog.Editable",
        );
    }

    #[test]
    fn closed_validator_rejects_mdclasses_and_foreign_structural_duplicates() {
        let cases = [
            (
                "ChildObjects",
                "Command",
                None,
                format!(
                    r#"<ChildObjects>{}</ChildObjects><x:ChildObjects xmlns:x="urn:foreign"/>"#,
                    typed_child_xml("Command", "Main", None)
                ),
                typed_child_xml("Command", "Main", None),
                "owner descriptor contains ChildObjects outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "Properties",
                "Command",
                None,
                typed_child_xml("Command", "Main", None),
                r#"<Command uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name></Properties><x:Properties xmlns:x="urn:foreign"><x:Name>Main</x:Name></x:Properties><ChildObjects/></Command>"#.to_string(),
                "declared child descriptor is invalid: Command descriptor contains Properties outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "Name",
                "Form",
                None,
                typed_child_xml("Form", "Main", None),
                r#"<Form uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><x:Name xmlns:x="urn:foreign">Main</x:Name></Properties><ChildObjects/></Form>"#.to_string(),
                "declared child descriptor is invalid: Form descriptor contains Name outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
            (
                "TemplateType",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Main", Some("TextDocument")),
                r#"<Template uuid="77777777-7777-4777-8777-777777777777"><Properties><Name>Main</Name><TemplateType>TextDocument</TemplateType><x:TemplateType xmlns:x="urn:foreign">TextDocument</x:TemplateType></Properties><ChildObjects/></Template>"#.to_string(),
                "declared child descriptor is invalid: Template descriptor contains TemplateType outside the MDClasses namespace: Catalog.Editable".to_string(),
            ),
        ];
        for (label, kind, template_type, owner_entry, descriptor, expected) in cases {
            let mut subject =
                closed_validator_subject(kind, template_type, &owner_entry, &descriptor);
            if label == "ChildObjects" {
                subject.resources[0].bytes = typed_owner_descriptor_with_structure(&owner_entry);
            }
            assert_exact_closed_diagnostic(&subject, &expected);
            assert!(!expected.contains('/'), "{label}");
        }
    }

    #[test]
    fn closed_child_graph_accepts_real_text_refs_and_typed_inline_owner_entries() {
        for (label, owner_entry) in [
            ("real-ref", "<Command>Main</Command>".to_string()),
            ("typed-inline", typed_child_xml("Command", "Main", None)),
        ] {
            let descriptor = typed_child_xml("Command", "Main", None);
            let subject = closed_validator_subject("Command", None, &owner_entry, &descriptor);

            let mut diagnostics = Vec::new();
            validate_child_footprints(&subject, &mut diagnostics);

            assert!(diagnostics.is_empty(), "{label}: {diagnostics:?}");
        }
    }

    #[test]
    fn final_child_graph_rejects_wholly_omitted_evidence_for_every_profile() {
        for (label, mut subject) in [
            (
                "form",
                closed_child_subject(
                    "Form",
                    "Main",
                    None,
                    &typed_child_xml("Form", "Main", None),
                    vec![
                        MetadataChildDirectoryKind::Root,
                        MetadataChildDirectoryKind::Extension,
                    ],
                    vec![(MetadataChildResourceKind::FormContent, 0)],
                ),
            ),
            (
                "template",
                closed_child_subject(
                    "Template",
                    "Main",
                    Some("TextDocument"),
                    &typed_child_xml("Template", "Main", Some("TextDocument")),
                    vec![
                        MetadataChildDirectoryKind::Root,
                        MetadataChildDirectoryKind::Extension,
                    ],
                    vec![(
                        MetadataChildResourceKind::TemplateContent {
                            template_type: MetadataTemplateType::TextDocument,
                            part: MetadataTemplateResourcePart::Primary,
                        },
                        0,
                    )],
                ),
            ),
            (
                "command",
                closed_child_subject(
                    "Command",
                    "Main",
                    None,
                    &typed_child_xml("Command", "Main", None),
                    Vec::new(),
                    Vec::new(),
                ),
            ),
        ] {
            subject.child_footprints.clear();
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics);

            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("footprint evidence")),
                "{label}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn child_descriptor_bytes_reject_forged_role_kind_and_name_for_every_profile() {
        let cases = [
            (
                "form-name",
                "Form",
                None,
                typed_child_xml("Form", "Other", None),
                vec![(MetadataChildResourceKind::FormContent, 0)],
            ),
            (
                "form-kind",
                "Form",
                None,
                typed_child_xml("Template", "Main", Some("TextDocument")),
                vec![(MetadataChildResourceKind::FormContent, 0)],
            ),
            (
                "template-name",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Other", Some("TextDocument")),
                vec![(
                    MetadataChildResourceKind::TemplateContent {
                        template_type: MetadataTemplateType::TextDocument,
                        part: MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "template-kind",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Form", "Main", None),
                vec![(
                    MetadataChildResourceKind::TemplateContent {
                        template_type: MetadataTemplateType::TextDocument,
                        part: MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "template-type",
                "Template",
                Some("TextDocument"),
                typed_child_xml("Template", "Main", Some("HTMLDocument")),
                vec![(
                    MetadataChildResourceKind::TemplateContent {
                        template_type: MetadataTemplateType::TextDocument,
                        part: MetadataTemplateResourcePart::Primary,
                    },
                    0,
                )],
            ),
            (
                "command-name",
                "Command",
                None,
                typed_child_xml("Command", "Other", None),
                Vec::new(),
            ),
            (
                "command-kind",
                "Command",
                None,
                typed_child_xml("Form", "Main", None),
                Vec::new(),
            ),
        ];
        for (label, kind, template_type, forged_descriptor, resources) in cases {
            let directories = if kind == "Command" {
                Vec::new()
            } else {
                vec![
                    MetadataChildDirectoryKind::Root,
                    MetadataChildDirectoryKind::Extension,
                ]
            };
            let subject = closed_child_subject(
                kind,
                "Main",
                template_type,
                &forged_descriptor,
                directories,
                resources,
            );
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics);

            let expected_message = if label == "template-type" {
                "footprint evidence"
            } else {
                "byte-matching descriptor"
            };
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains(expected_message)),
                "{label}: {diagnostics:?}"
            );
        }

        let form = typed_child_xml("Form", "Main", None);
        for (label, descriptor) in [
            (
                "root-namespace",
                String::from_utf8(typed_child_descriptor(&form))
                    .unwrap()
                    .replace(MD_CLASSES_NS, "urn:foreign"),
            ),
            (
                "root-version",
                String::from_utf8(typed_child_descriptor(&form))
                    .unwrap()
                    .replace("version=\"2.20\"", "version=\"2.19\""),
            ),
            (
                "artifact-namespace",
                format!(
                    r#"<MetaDataObject xmlns="{MD_CLASSES_NS}" version="2.20"><x:Form xmlns:x="urn:foreign"><x:Properties><x:Name>Main</x:Name></x:Properties><x:ChildObjects/></x:Form></MetaDataObject>"#
                ),
            ),
        ] {
            let subject = closed_child_subject(
                "Form",
                "Main",
                None,
                &form,
                vec![
                    MetadataChildDirectoryKind::Root,
                    MetadataChildDirectoryKind::Extension,
                ],
                vec![(MetadataChildResourceKind::FormContent, 0)],
            );
            let mut subject = subject;
            subject.resources[1].bytes = descriptor.into_bytes();
            let mut diagnostics = Vec::new();

            validate_child_footprints(&subject, &mut diagnostics);

            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("byte-matching descriptor")),
                "{label}: {diagnostics:?}"
            );
        }
    }

    #[test]
    fn child_footprint_sets_are_order_independent_but_ordinals_are_canonical() {
        let html = typed_child_xml("Template", "Main", Some("HTMLDocument"));
        let primary = MetadataChildResourceKind::TemplateContent {
            template_type: MetadataTemplateType::HtmlDocument,
            part: MetadataTemplateResourcePart::Primary,
        };
        let page = MetadataChildResourceKind::TemplateContent {
            template_type: MetadataTemplateType::HtmlDocument,
            part: MetadataTemplateResourcePart::HtmlPage,
        };
        let mut reordered = closed_child_subject(
            "Template",
            "Main",
            Some("HTMLDocument"),
            &html,
            vec![
                MetadataChildDirectoryKind::HtmlPages,
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
            ],
            vec![(page, 1), (primary, 0)],
        );
        reordered.resources.reverse();
        let mut diagnostics = Vec::new();
        validate_child_footprints(&reordered, &mut diagnostics);
        assert!(diagnostics.is_empty(), "reordered: {diagnostics:?}");

        for (label, resources) in [
            ("duplicate", vec![(primary, 0), (page, 0)]),
            ("gap", vec![(primary, 0), (page, 2)]),
            ("swapped", vec![(primary, 1), (page, 0)]),
        ] {
            let subject = closed_child_subject(
                "Template",
                "Main",
                Some("HTMLDocument"),
                &html,
                vec![
                    MetadataChildDirectoryKind::Root,
                    MetadataChildDirectoryKind::Extension,
                    MetadataChildDirectoryKind::HtmlPages,
                ],
                resources,
            );
            let mut diagnostics = Vec::new();
            validate_child_footprints(&subject, &mut diagnostics);
            assert!(
                diagnostics.iter().any(|diagnostic| diagnostic
                    .message
                    .contains("canonical resource and ordinal set")),
                "{label}: {diagnostics:?}"
            );
        }

        let mut duplicate_directory = closed_child_subject(
            "Template",
            "Main",
            Some("HTMLDocument"),
            &html,
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::HtmlPages,
                MetadataChildDirectoryKind::Root,
            ],
            vec![(primary, 0), (page, 1)],
        );
        duplicate_directory.child_footprints.reverse();
        let mut diagnostics = Vec::new();
        validate_child_footprints(&duplicate_directory, &mut diagnostics);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("directory set")));

        let form = typed_child_xml("Form", "Main", None);
        let subject = closed_child_subject(
            "Form",
            "Main",
            None,
            &form,
            vec![
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::Root,
            ],
            vec![
                (MetadataChildResourceKind::Module, 0),
                (MetadataChildResourceKind::FormContent, 1),
            ],
        );
        let mut diagnostics = Vec::new();
        validate_child_footprints(&subject, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("canonical resource and ordinal set")));
    }

    #[test]
    fn complete_multi_child_graph_is_permutation_safe_and_rejects_duplicate_or_orphan_groups() {
        let form_xml = typed_child_xml("Form", "Entry", None);
        let template_xml = typed_child_xml("Template", "Print", Some("TextDocument"));
        let command_xml = typed_child_xml("Command", "Run", None);
        let mut form = closed_child_subject(
            "Form",
            "Entry",
            None,
            &form_xml,
            vec![
                MetadataChildDirectoryKind::Root,
                MetadataChildDirectoryKind::Extension,
            ],
            vec![(MetadataChildResourceKind::FormContent, 0)],
        );
        let template = closed_child_subject(
            "Template",
            "Print",
            Some("TextDocument"),
            &template_xml,
            vec![
                MetadataChildDirectoryKind::Extension,
                MetadataChildDirectoryKind::Root,
            ],
            vec![(
                MetadataChildResourceKind::TemplateContent {
                    template_type: MetadataTemplateType::TextDocument,
                    part: MetadataTemplateResourcePart::Primary,
                },
                0,
            )],
        );
        let command =
            closed_child_subject("Command", "Run", None, &command_xml, Vec::new(), Vec::new());
        form.resources[0].bytes = typed_owner_descriptor(&[
            "<Command>Run</Command>".to_string(),
            "<Form>Entry</Form>".to_string(),
            "<Template>Print</Template>".to_string(),
        ]);
        form.resources
            .extend(template.resources.into_iter().skip(1));
        form.resources.extend(command.resources.into_iter().skip(1));
        form.child_footprints.extend(template.child_footprints);
        form.child_footprints.extend(command.child_footprints);
        form.resources.reverse();
        form.child_footprints.reverse();

        let mut exact = Vec::new();
        validate_child_footprints(&form, &mut exact);
        assert!(exact.is_empty(), "exact permutation: {exact:?}");

        let mut duplicate_descriptor = form.clone();
        let descriptor = duplicate_descriptor
            .resources
            .iter()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Form { .. }))
            .unwrap()
            .clone();
        duplicate_descriptor.resources.push(descriptor);
        let mut diagnostics = Vec::new();
        validate_child_footprints(&duplicate_descriptor, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("exactly one byte-matching descriptor")));

        let mut orphan_descriptor = form.clone();
        let orphan_xml = typed_child_xml("Form", "Orphan", None);
        orphan_descriptor.resources.push(image(
            MetadataResourceRole::Form {
                owner: address("Catalog.Editable"),
                name: "Orphan".to_string(),
            },
            typed_child_descriptor(&orphan_xml),
        ));
        let mut diagnostics = Vec::new();
        validate_child_footprints(&orphan_descriptor, &mut diagnostics);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("descriptor is orphaned")));

        let mut duplicate_evidence = form.clone();
        duplicate_evidence
            .child_footprints
            .push(duplicate_evidence.child_footprints[0].clone());
        let mut diagnostics = Vec::new();
        validate_child_footprints(&duplicate_evidence, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("exactly one matching footprint evidence")));

        let mut orphan_evidence = form.clone();
        orphan_evidence
            .child_footprints
            .push(MetadataChildFootprintEvidence {
                child: address("Catalog.Editable.Command.Orphan"),
                profile: MetadataChildProfile::Command,
                directories: Vec::new(),
            });
        let mut diagnostics = Vec::new();
        validate_child_footprints(&orphan_evidence, &mut diagnostics);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("evidence is orphaned")));

        let mut duplicate_resource = form.clone();
        let resource = duplicate_resource
            .resources
            .iter()
            .find(|resource| {
                matches!(
                    resource.role,
                    MetadataResourceRole::ChildResource {
                        kind: MetadataChildResourceKind::FormContent,
                        ..
                    }
                )
            })
            .unwrap()
            .clone();
        duplicate_resource.resources.push(resource);
        let mut diagnostics = Vec::new();
        validate_child_footprints(&duplicate_resource, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("canonical resource and ordinal set")));

        let mut mislabeled_resource = form.clone();
        let resource = mislabeled_resource
            .resources
            .iter_mut()
            .find(|resource| {
                matches!(
                    resource.role,
                    MetadataResourceRole::ChildResource {
                        kind: MetadataChildResourceKind::FormContent,
                        ..
                    }
                )
            })
            .unwrap();
        let MetadataResourceRole::ChildResource { child, .. } = &mut resource.role else {
            unreachable!()
        };
        *child = address("Catalog.Editable.Form.Orphan");
        let mut diagnostics = Vec::new();
        validate_child_footprints(&mislabeled_resource, &mut diagnostics);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .contains("canonical resource and ordinal set")
        }));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("resource is orphaned")));

        let mut duplicate_owner_child = form.clone();
        duplicate_owner_child
            .resources
            .iter_mut()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Descriptor))
            .unwrap()
            .bytes = typed_owner_descriptor(&[
            typed_child_xml("Form", "Entry", None),
            typed_child_xml("Form", "Entry", None),
            typed_child_xml("Template", "Print", Some("TextDocument")),
            typed_child_xml("Command", "Run", None),
        ]);
        let mut diagnostics = Vec::new();
        validate_child_footprints(&duplicate_owner_child, &mut diagnostics);
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("duplicate child identity")));
    }

    #[test]
    fn template_payload_profiles_accept_representative_xml_text_and_binary_images() {
        let cases = [
            template_subject(
                "Catalog.Editable.Template.Spreadsheet",
                vec![(
                    MetadataTemplateType::SpreadsheetDocument,
                    MetadataTemplateResourcePart::Primary,
                    super::super::super::mxl::empty_spreadsheet_document_xml().into_bytes(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Schema",
                vec![(
                    MetadataTemplateType::DataCompositionSchema,
                    MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#
                        .to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Text",
                vec![(
                    MetadataTemplateType::TextDocument,
                    MetadataTemplateResourcePart::Primary,
                    "Текстовый макет".as_bytes().to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Binary",
                vec![(
                    MetadataTemplateType::BinaryData,
                    MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Html",
                vec![
                    (
                        MetadataTemplateType::HtmlDocument,
                        MetadataTemplateResourcePart::Primary,
                        br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#.to_vec(),
                    ),
                    (
                        MetadataTemplateType::HtmlDocument,
                        MetadataTemplateResourcePart::HtmlPage,
                        br#"<html><head><meta charset="utf-8"/></head><body/></html>"#.to_vec(),
                    ),
                ],
            ),
        ];

        for subject in cases {
            let diagnostics = closed_child_diagnostics(&subject);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
        }
    }

    #[test]
    fn non_binary_template_profiles_reject_binary_or_wrong_xml_images() {
        let cases = [
            template_subject(
                "Catalog.Editable.Template.Schema",
                vec![(
                    MetadataTemplateType::DataCompositionSchema,
                    MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Text",
                vec![(
                    MetadataTemplateType::TextDocument,
                    MetadataTemplateResourcePart::Primary,
                    vec![0xff, 0x00, 0x80],
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.Spreadsheet",
                vec![(
                    MetadataTemplateType::SpreadsheetDocument,
                    MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema"/>"#
                        .to_vec(),
                )],
            ),
            template_subject(
                "Catalog.Editable.Template.VersionedSchema",
                vec![(
                    MetadataTemplateType::DataCompositionSchema,
                    MetadataTemplateResourcePart::Primary,
                    br#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#
                        .to_vec(),
                )],
            ),
        ];

        for subject in cases {
            let diagnostics = closed_child_diagnostics(&subject);
            assert!(!diagnostics.is_empty());
        }
    }

    #[test]
    fn html_template_requires_both_its_descriptor_and_language_page() {
        for (label, resources) in [
            (
                "missing page",
                vec![(
                    MetadataTemplateType::HtmlDocument,
                    MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                        .to_vec(),
                )],
            ),
            (
                "missing descriptor",
                vec![(
                    MetadataTemplateType::HtmlDocument,
                    MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body/></html>"#.to_vec(),
                )],
            ),
            (
                "descriptor names a different page",
                vec![
                    (
                        MetadataTemplateType::HtmlDocument,
                        MetadataTemplateResourcePart::Primary,
                        br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>de</Page></Help>"#.to_vec(),
                    ),
                    (
                        MetadataTemplateType::HtmlDocument,
                        MetadataTemplateResourcePart::HtmlPage,
                        br#"<html><body/></html>"#.to_vec(),
                    ),
                ],
            ),
        ] {
            let subject = template_subject("Catalog.Editable.Template.Html", resources);

            let diagnostics = closed_child_diagnostics(&subject);

            assert!(!diagnostics.is_empty(), "{label}");
        }
    }

    #[test]
    fn child_footprint_evidence_is_independently_checked_against_resources_and_directories() {
        let mut subject = template_subject(
            "Catalog.Editable.Template.Html",
            vec![
                (
                    MetadataTemplateType::HtmlDocument,
                    MetadataTemplateResourcePart::Primary,
                    br#"<Help xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"><Page>ru</Page></Help>"#
                        .to_vec(),
                ),
                (
                    MetadataTemplateType::HtmlDocument,
                    MetadataTemplateResourcePart::HtmlPage,
                    br#"<html><body/></html>"#.to_vec(),
                ),
            ],
        );
        subject.child_footprints[0]
            .directories
            .retain(|directory| *directory != MetadataChildDirectoryKind::HtmlPages);

        let mut wrong_directories = Vec::new();
        validate_child_footprints(&subject, &mut wrong_directories);
        assert!(wrong_directories
            .iter()
            .any(|diagnostic| diagnostic.message.contains("directory set")));

        subject.child_footprints[0]
            .directories
            .push(MetadataChildDirectoryKind::HtmlPages);
        subject.resources.pop();
        let mut missing_part = Vec::new();
        validate_child_footprints(&subject, &mut missing_part);
        assert!(missing_part
            .iter()
            .any(|diagnostic| diagnostic.message.contains("canonical resource")));

        subject.resources.push(image(
            MetadataResourceRole::ChildResource {
                child: address("Catalog.Editable.Template.Html"),
                kind: MetadataChildResourceKind::TemplateContent {
                    template_type: MetadataTemplateType::HtmlDocument,
                    part: MetadataTemplateResourcePart::HtmlPage,
                },
                ordinal: 1,
            },
            br#"<html><body/></html>"#.to_vec(),
        ));
        let forged = typed_child_xml("Template", "Html", Some("SpreadsheetDocument"));
        subject
            .resources
            .iter_mut()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Template { .. }))
            .unwrap()
            .bytes = typed_child_descriptor(&forged);
        let mut forged_profile = Vec::new();
        validate_child_footprints(&subject, &mut forged_profile);
        assert!(forged_profile
            .iter()
            .any(|diagnostic| diagnostic.message.contains("footprint evidence")));

        let exact = typed_child_xml("Template", "Html", Some("HTMLDocument"));
        subject
            .resources
            .iter_mut()
            .find(|resource| matches!(resource.role, MetadataResourceRole::Template { .. }))
            .unwrap()
            .bytes = typed_child_descriptor(&exact);
        let mut exact = Vec::new();
        validate_child_footprints(&subject, &mut exact);
        assert!(exact.is_empty(), "{exact:?}");
    }

    #[test]
    fn internal_valid_and_semantically_invalid_images_match_legacy_classification() {
        let registration = owner(&[("CommonModule", "Service")]);
        let valid_descriptor = common_module("Service");
        let valid = subject(
            "CommonModule.Service",
            Some(&valid_descriptor),
            &registration,
            &[],
        );
        assert_real_legacy_equivalent("valid", &valid, MetaValidationStatus::Passed);

        let invalid_descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<CommonModule uuid="11111111-1111-4111-8111-111111111111">
<Properties/><ChildObjects/>
</CommonModule></MetaDataObject>"#
        );
        let invalid = subject(
            "CommonModule.Service",
            Some(&invalid_descriptor),
            &registration,
            &[],
        );
        let result = assert_real_legacy_equivalent(
            "semantic-invalid",
            &invalid,
            MetaValidationStatus::Failed,
        );
        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(
            result.diagnostics[0].metadata_path,
            Some(address("CommonModule.Service"))
        );
        assert_eq!(result.diagnostics[0].operation_index, None);
        assert!(result.diagnostics[0].field.is_some());
    }

    #[test]
    fn real_public_adapter_preserves_detailed_legacy_stdout_and_artifact() {
        let descriptor = common_module("Service");
        let registration = owner(&[("CommonModule", "Service")]);
        let valid = subject(
            "CommonModule.Service",
            Some(&descriptor),
            &registration,
            &[],
        );

        let outcome = real_legacy_outcome("detailed-stdout", &valid);
        let stdout = outcome.stdout.clone().unwrap_or_default();

        assert!(outcome.ok, "{outcome:?}");
        assert!(stdout.contains("[OK]"), "{stdout}");
        assert!(stdout.contains("checks) ==="), "{stdout}");
        assert_eq!(outcome.artifacts.len(), 1, "{outcome:?}");
    }

    #[test]
    fn real_public_method_proof_is_counted_by_the_single_reporter() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Target.Run");
        let registration = owner(&[("ScheduledJob", "Nightly"), ("CommonModule", "Target")]);
        let module_descriptor = common_module("Target");
        let mut valid = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[("CommonModule.Target", &module_descriptor)],
        );
        valid.resources.push(image(
            MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            b"Procedure Run() Export\nEndProcedure".to_vec(),
        ));

        let outcome = real_legacy_outcome("method-reporter", &valid);
        let stdout = outcome.stdout.clone().unwrap_or_default();

        assert!(outcome.ok, "{outcome:?}");
        assert!(
            stdout.contains("[OK]    13. Method reference: MethodName = 'CommonModule.Target.Run'"),
            "{stdout}"
        );
        assert!(!stdout.contains("Subject proof diagnostics"), "{stdout}");
        assert_report_summary_matches_lines(&stdout);
    }

    #[test]
    fn real_public_registrar_proof_is_counted_by_the_single_reporter() {
        let descriptor = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let registrar = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Documents/Регистратор.xml"
        ))
        .replace(
            "InformationRegister.SubordinateRegister",
            "InformationRegister.Other",
        );
        let invalid = subject(
            "InformationRegister.SubordinateRegister",
            Some(descriptor),
            registration,
            &[
                ("Language.Русский", language),
                ("Document.Регистратор", &registrar),
            ],
        );

        let outcome = real_legacy_outcome("registrar-reporter", &invalid);
        let stdout = outcome.stdout.clone().unwrap_or_default();

        assert!(outcome.ok, "{outcome:?}");
        assert!(stdout.contains("no registrar document found"), "{stdout}");
        assert!(
            !stdout.contains("[OK]    10. Cross-property consistency"),
            "{stdout}"
        );
        assert!(!stdout.contains("Subject proof diagnostics"), "{stdout}");
        assert_report_summary_matches_lines(&stdout);
    }

    #[test]
    fn internal_malformed_xml_is_a_hard_failure_matching_legacy_classification() {
        let registration = owner(&[("CommonModule", "Service")]);
        let malformed = subject(
            "CommonModule.Service",
            Some("<MetaDataObject"),
            &registration,
            &[],
        );

        let result = assert_real_legacy_equivalent(
            "malformed-descriptor",
            &malformed,
            MetaValidationStatus::Failed,
        );

        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(
            result.diagnostics[0].field.as_deref(),
            Some("resources[0].bytes")
        );
    }

    #[test]
    fn internal_owner_registration_failure_matches_legacy_classification() {
        let descriptor = common_module("Service");
        let registration = owner(&[("CommonModule", "Other")]);
        let invalid = subject(
            "CommonModule.Service",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result = assert_real_legacy_equivalent(
            "missing-registration",
            &invalid,
            MetaValidationStatus::Failed,
        );

        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ValidationFailed
        );
        assert_eq!(
            result.diagnostics[0].field.as_deref(),
            Some("resources[1].bytes")
        );
        assert!(result.diagnostics[0].message.contains("not registered"));
    }

    #[test]
    fn internal_nonexternal_descriptor_requires_owner_image() {
        let descriptor = common_module("Service");
        let incomplete = MetadataValidationSubject {
            target: address("CommonModule.Service"),
            resources: vec![image(
                MetadataResourceRole::Descriptor,
                descriptor.into_bytes(),
            )],
            child_footprints: Vec::new(),
        };

        let result = MetadataValidator.validate(&incomplete, &context());

        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert_eq!(
            result.diagnostics[0].code,
            MetaDiagnosticCode::ProviderUnavailable
        );
        assert_eq!(result.diagnostics[0].field.as_deref(), Some("resources"));
    }

    #[test]
    fn internal_duplicate_child_failure_matches_legacy_classification() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<HTTPService uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Api</Name></Properties><ChildObjects>
<URLTemplate><Properties><Name>route</Name><Template>/one</Template></Properties><ChildObjects/></URLTemplate>
<URLTemplate><Properties><Name>route</Name><Template>/two</Template></Properties><ChildObjects/></URLTemplate>
</ChildObjects></HTTPService></MetaDataObject>"#
        );
        let registration = owner(&[("HTTPService", "Api")]);
        let duplicate = subject("HTTPService.Api", Some(&descriptor), &registration, &[]);

        let result = assert_real_legacy_equivalent(
            "duplicate-child",
            &duplicate,
            MetaValidationStatus::Failed,
        );

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("Duplicate URLTemplate")));
    }

    #[test]
    fn internal_invalid_reference_failure_matches_legacy_classification() {
        let descriptor = format!(
            r#"<MetaDataObject xmlns="{MD_NS}" version="2.20">
<DocumentJournal uuid="11111111-1111-4111-8111-111111111111">
<Properties><Name>Journal</Name></Properties><ChildObjects>
<Column uuid="44444444-4444-4444-8444-444444444444"><Properties><Name>Broken</Name><References/></Properties></Column>
</ChildObjects></DocumentJournal></MetaDataObject>"#
        );
        let registration = owner(&[("DocumentJournal", "Journal")]);
        let invalid = subject(
            "DocumentJournal.Journal",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result = assert_real_legacy_equivalent(
            "invalid-reference",
            &invalid,
            MetaValidationStatus::Failed,
        );

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("empty References")));
    }

    #[test]
    fn internal_method_validation_associates_export_with_the_referenced_common_module() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Target.Run");
        let registration = owner(&[
            ("ScheduledJob", "Nightly"),
            ("CommonModule", "Target"),
            ("CommonModule", "Wrong"),
        ]);
        let target_module = common_module("Target");
        let wrong_module = common_module("Wrong");
        let mut resources = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[
                ("CommonModule.Target", &target_module),
                ("CommonModule.Wrong", &wrong_module),
            ],
        )
        .resources;
        resources.push(image(
            MetadataResourceRole::Module {
                owner: address("CommonModule.Wrong"),
            },
            b"Procedure Run() Export\nEndProcedure".to_vec(),
        ));
        resources.push(image(
            MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            b"Procedure Different() Export\nEndProcedure".to_vec(),
        ));
        let inaccessible = MetadataValidationSubject {
            target: address("ScheduledJob.Nightly"),
            resources,
            child_footprints: Vec::new(),
        };

        let result = MetadataValidator.validate(&inaccessible, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && diagnostic.message.contains("not found as exported")
        }));

        let mut accessible = inaccessible.clone();
        accessible.resources.pop();
        accessible.resources.push(image(
            MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            "\u{feff}Procedure Run() Export\nEndProcedure"
                .as_bytes()
                .to_vec(),
        ));
        let result = MetadataValidator.validate(&accessible, &context());
        assert_eq!(result.status, MetaValidationStatus::Passed);
        assert!(!result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("not found as exported")));
    }

    #[test]
    fn internal_missing_method_dependency_matches_real_legacy_failure() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Missing.Run");
        let registration = owner(&[("ScheduledJob", "Nightly")]);
        let missing = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[],
        );

        let result = assert_real_legacy_equivalent(
            "missing-method-dependency",
            &missing,
            MetaValidationStatus::Failed,
        );
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("CommonModule `Missing`")));
    }

    #[test]
    fn internal_corrupt_method_dependency_matches_real_legacy_provider_failure() {
        let descriptor = scheduled_job("Nightly", "CommonModule.Target.Run");
        let registration = owner(&[("ScheduledJob", "Nightly"), ("CommonModule", "Target")]);
        let mut invalid = subject(
            "ScheduledJob.Nightly",
            Some(&descriptor),
            &registration,
            &[("CommonModule.Target", "not XML")],
        );
        invalid.resources.push(image(
            MetadataResourceRole::Module {
                owner: address("CommonModule.Target"),
            },
            b"Procedure Run() Export\nEndProcedure".to_vec(),
        ));

        let result = assert_real_legacy_equivalent(
            "corrupt-method-dependency",
            &invalid,
            MetaValidationStatus::Failed,
        );

        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
    }

    #[test]
    fn internal_reverse_registrar_inconsistency_is_not_lost() {
        let descriptor = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let registrar = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Documents/Регистратор.xml"
        ))
        .replace(
            "InformationRegister.SubordinateRegister",
            "InformationRegister.Other",
        );
        let invalid = subject(
            "InformationRegister.SubordinateRegister",
            Some(descriptor),
            registration,
            &[
                ("Language.Русский", language),
                ("Document.Регистратор", &registrar),
            ],
        );

        let result = MetadataValidator.validate(&invalid, &context());

        assert_eq!(result.status, MetaValidationStatus::Passed);
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Warning
                && diagnostic.message.contains("no registrar document found")
        }));

        let complete_read = MetadataValidator.validate_complete_read(&invalid, &context());
        assert_eq!(complete_read.status, MetaValidationStatus::Failed);
        assert!(complete_read.diagnostics.iter().any(|diagnostic| {
            diagnostic.severity == MetaDiagnosticSeverity::Error
                && diagnostic.code == MetaDiagnosticCode::ValidationFailed
                && diagnostic.metadata_path.as_ref() == Some(&invalid.target)
                && diagnostic.field.as_deref() == Some("resources")
        }));
    }

    #[test]
    fn complete_read_proof_accepts_consistent_forward_and_reverse_dependencies() {
        let document = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Documents/Регистратор.xml"
        ));
        let register = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/InformationRegisters/SubordinateRegister.xml"
        ));
        let registration = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Configuration.xml"
        ));
        let language = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/unica_mcp_script_parity/meta-validate-subordinate-register/Languages/Русский.xml"
        ));
        let document_subject = subject(
            "Document.Регистратор",
            Some(document),
            registration,
            &[
                ("Language.Русский", language),
                ("InformationRegister.SubordinateRegister", register),
            ],
        );
        let register_subject = subject(
            "InformationRegister.SubordinateRegister",
            Some(register),
            registration,
            &[
                ("Language.Русский", language),
                ("Document.Регистратор", document),
            ],
        );

        assert!(complete_read_proof_diagnostics(&document_subject).is_empty());
        assert!(complete_read_proof_diagnostics(&register_subject).is_empty());
    }

    #[test]
    fn internal_declared_resource_encodings_fail_closed() {
        let descriptor = common_module("Service");
        let registration = owner(&[("CommonModule", "Service")]);
        let corrupt_roles = [
            MetadataResourceRole::Dependency {
                target: address("Subsystem.Area"),
            },
            MetadataResourceRole::Module {
                owner: address("CommonModule.Other"),
            },
            MetadataResourceRole::Form {
                owner: address("CommonModule.Service"),
                name: "Main".to_string(),
            },
            MetadataResourceRole::Template {
                owner: address("CommonModule.Service"),
                name: "Layout".to_string(),
            },
            MetadataResourceRole::Command {
                owner: address("CommonModule.Service"),
                name: "Execute".to_string(),
            },
            MetadataResourceRole::AuxiliaryXml {
                owner: address("ExchangePlan.Sync"),
                kind: MetadataAuxiliaryXmlKind::ExchangePlanContent,
            },
        ];
        for role in corrupt_roles {
            let mut invalid = subject(
                "CommonModule.Service",
                Some(&descriptor),
                &registration,
                &[],
            );
            let bytes = if matches!(role, MetadataResourceRole::Module { .. }) {
                vec![0xff, 0xfe]
            } else {
                b"not XML".to_vec()
            };
            invalid.resources.push(image(role, bytes));

            let result = MetadataValidator.validate(&invalid, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed);
            assert!(result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
        }
    }

    #[test]
    fn internal_remove_validates_surviving_images_without_deleted_descriptor() {
        let clean_registration = owner(&[("CommonModule", "Other")]);
        let clean_dependency = dependency_with_reference(Some("CommonModule.Other"));
        let clean = subject(
            "CommonModule.Service",
            None,
            &clean_registration,
            &[("Subsystem.Area", &clean_dependency)],
        );
        assert_eq!(
            MetadataValidator.validate(&clean, &context()).status,
            MetaValidationStatus::Passed
        );

        let stale_registration = owner(&[("CommonModule", "Service")]);
        let registration_failure = subject(
            "CommonModule.Service",
            None,
            &stale_registration,
            &[("Subsystem.Area", &clean_dependency)],
        );
        let result = MetadataValidator.validate(&registration_failure, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("surviving registration")));

        let stale_dependency = dependency_with_reference(Some("CommonModule.Service"));
        let dependency_failure = subject(
            "CommonModule.Service",
            None,
            &clean_registration,
            &[("Subsystem.Area", &stale_dependency)],
        );
        let result = MetadataValidator.validate(&dependency_failure, &context());
        assert_eq!(result.status, MetaValidationStatus::Failed);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("surviving reference")));
    }

    #[test]
    fn internal_remove_rejects_every_corrupt_dependency_encoding() {
        let clean_registration = owner(&[("CommonModule", "Other")]);
        for bytes in [vec![0xff, 0xfe], b"not XML".to_vec(), b"<broken".to_vec()] {
            let invalid = MetadataValidationSubject {
                target: address("CommonModule.Service"),
                resources: vec![
                    image(
                        MetadataResourceRole::Registration,
                        clean_registration.as_bytes().to_vec(),
                    ),
                    image(
                        MetadataResourceRole::Dependency {
                            target: address("Subsystem.Area"),
                        },
                        bytes,
                    ),
                ],
                child_footprints: Vec::new(),
            };

            let result = MetadataValidator.validate(&invalid, &context());

            assert_eq!(result.status, MetaValidationStatus::Failed);
            assert!(result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == MetaDiagnosticCode::ProviderUnavailable));
        }
    }
}
