use std::collections::BTreeSet;

use sha2::{Digest, Sha256};
use unica_format_core::{
    ports::{
        OperationalEvidenceRevision, OperationalValidationResult, SemanticArtifactId,
        ValidationFinding, ValidationFindingCode, ValidationFindingSeverity, ValidationIssueKind,
        ValidationOptions, ValidationReport,
    },
    source::{SourceAdapterError, SourceAdapterErrorKind},
};

use super::{operations::PlatformOperationSession, profile, schema, semantic_map, xml};

pub(crate) fn validate(
    sessions: &[unica_format_core::ports::OperationalSourceSession],
    options: ValidationOptions,
) -> Result<OperationalValidationResult, SourceAdapterError> {
    let outcomes = sessions
        .iter()
        .map(|session| {
            let session = super::operations::session_from_handle(session)?;
            validate_one(session, options)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut evidence = Sha256::new();
    evidence.update(b"unica:platform-xml:validation-request:v1\0");
    let mut reports = Vec::with_capacity(outcomes.len());
    for (report, revision) in outcomes {
        evidence.update(revision.digest());
        reports.push(report);
    }
    OperationalValidationResult::new(
        reports,
        OperationalEvidenceRevision::from_digest(evidence.finalize().into()),
    )
    .map_err(contract_error)
}

fn validate_one(
    session: &PlatformOperationSession,
    options: ValidationOptions,
) -> Result<(ValidationReport, OperationalEvidenceRevision), SourceAdapterError> {
    let provider = match session.validation_provider() {
        Ok(provider) => provider,
        Err(_) => {
            return Ok((
                report(
                    SemanticArtifactId::new("artifact:unavailable").expect("constant semantic id"),
                    1,
                    vec![error(ValidationFindingCode::SourceUnreadable)],
                )?,
                session.failure_evidence(b"validation"),
            ))
        }
    };
    let report = validate_one_with_provider(session, &provider, options)?;
    let evidence = provider
        .finalize_evidence(b"validation")
        .unwrap_or_else(|_| session.failure_evidence(b"validation"));
    Ok((report, evidence))
}

fn validate_one_with_provider(
    session: &PlatformOperationSession,
    provider: &super::operations::LazyPlatformSource,
    options: ValidationOptions,
) -> Result<ValidationReport, SourceAdapterError> {
    let subject = match PlatformOperationSession::validation_subject(provider) {
        Ok(subject) => subject,
        Err(_) => {
            return report(
                SemanticArtifactId::new("artifact:unavailable").expect("constant semantic id"),
                1,
                vec![error(ValidationFindingCode::SourceUnreadable)],
            )
        }
    };
    let subject_id = subject.id().clone();
    let bytes = subject.bytes();
    let mut checks = 1u16;
    let mut findings = Vec::new();
    let (_, document) = match xml::parse_bounded_xml_document(bytes) {
        Ok(document) => document,
        Err(_) => {
            return report(
                subject_id,
                checks,
                vec![error(ValidationFindingCode::SourceMalformed)],
            )
        }
    };
    let root = document.root_element();
    if root.tag_name().namespace() != Some(xml::MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        findings.push(error(ValidationFindingCode::SemanticStructureInvalid));
    }
    checks += 1;
    match profile::classify_root_version(root.attribute("version")) {
        Ok(profile::FormatCompatibility::Supported { .. }) => {}
        Ok(_) | Err(_) => findings.push(error(ValidationFindingCode::RevisionUnsupported)),
    }
    checks += 1;

    let children = root
        .children()
        .filter(|node| node.is_element() && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS))
        .collect::<Vec<_>>();
    if children.len() != 1 {
        findings.push(error(ValidationFindingCode::SemanticStructureInvalid));
    }
    checks += 1;
    if let Some(object) = children.first().copied() {
        let object_profile = schema::metadata_class_profile(object.tag_name().name());
        if object_profile.is_none() {
            findings.push(error(ValidationFindingCode::SemanticStructureInvalid));
        }
        checks += 1;
        match object.attribute("uuid") {
            None | Some("") => findings.push(error(ValidationFindingCode::IdentityMissing)),
            Some(value) if uuid::Uuid::parse_str(value).is_err() => {
                findings.push(error(ValidationFindingCode::IdentityInvalid))
            }
            Some(_) => {}
        }
        checks += 1;
        let properties = child(object, "Properties");
        let name = properties
            .and_then(|properties| child(properties, "Name"))
            .map(inner_text)
            .filter(|name| !name.trim().is_empty());
        if name.is_none() {
            findings.push(error(ValidationFindingCode::NameMissing));
        }
        checks += 1;

        if let Some(children) = child(object, "ChildObjects") {
            let mut identities = BTreeSet::new();
            for item in children.children().filter(|node| node.is_element()) {
                let identity = (item.tag_name().name(), inner_text(item));
                if !identity.1.is_empty() && !identities.insert(identity) {
                    findings.push(error(ValidationFindingCode::DuplicateSemanticItem));
                }
                checks = checks.saturating_add(1);
            }
        }

        for native_object in object.descendants().filter(roxmltree::Node::is_element) {
            let Some(profile) =
                semantic_map::metadata_class_profile(native_object.tag_name().name())
            else {
                continue;
            };
            let Some(properties) = child(native_object, "Properties") else {
                continue;
            };
            for property in properties.children().filter(roxmltree::Node::is_element) {
                checks = checks.saturating_add(1);
                let Some(mapping) = semantic_map::property_mapping(
                    semantic_map::object_kind(profile),
                    property.tag_name().name(),
                ) else {
                    continue;
                };
                let value = inner_text(property);
                let valid = match mapping.value_kind {
                    semantic_map::NativeValueKind::Boolean => {
                        matches!(value.as_str(), "true" | "false")
                    }
                    semantic_map::NativeValueKind::Enum if value.is_empty() => true,
                    semantic_map::NativeValueKind::Enum => semantic_map::enum_value(
                        semantic_map::object_kind(profile),
                        mapping.semantic_id,
                        &value,
                    )
                    .is_some(),
                    _ => true,
                };
                if !valid {
                    findings.push(error(ValidationFindingCode::SemanticValueInvalid));
                }
            }
        }
    }

    let context = super::operations::validation_context_for_provider(session, provider);
    if let Err(issue) = context {
        let code = match issue {
            ValidationIssueKind::SourceUnreadable | ValidationIssueKind::OwnerUnavailable => {
                ValidationFindingCode::SourceUnreadable
            }
            ValidationIssueKind::RegistrationMissing => ValidationFindingCode::RegistrationMissing,
            ValidationIssueKind::LanguageProfileMissing => {
                ValidationFindingCode::LanguageProfileMissing
            }
            ValidationIssueKind::ReferenceMissing => ValidationFindingCode::ReferenceMissing,
            ValidationIssueKind::RegistrarMissing => ValidationFindingCode::RegistrarMissing,
        };
        findings.push(error(code));
        checks = checks.saturating_add(1);
    } else if let Ok(context) = context {
        if context.command_text_validation_required() {
            if let Some(object) = children.first().copied() {
                append_command_presentation_findings(
                    object,
                    context.language_codes(),
                    &mut findings,
                );
                checks = checks.saturating_add(1);
            }
        }
        if context.references_present() == Some(false) {
            findings.push(error(ValidationFindingCode::ReferenceMissing));
        }
        if context.registrar_present() == Some(false) {
            findings.push(error(ValidationFindingCode::RegistrarMissing));
        }
        if context.method_reference_status().is_some_and(|status| {
            status != unica_format_core::ports::ValidationMethodReferenceStatus::Valid
        }) {
            findings.push(error(ValidationFindingCode::MethodReferenceInvalid));
        }
        checks = checks.saturating_add(3);
    }

    findings.truncate(usize::from(options.max_findings()));
    report(subject_id, checks.max(1), findings)
}

fn report(
    subject: SemanticArtifactId,
    checks: u16,
    findings: Vec<ValidationFinding>,
) -> Result<ValidationReport, SourceAdapterError> {
    ValidationReport::new(subject, checks.max(findings.len() as u16), findings)
        .map_err(contract_error)
}

const fn error(code: ValidationFindingCode) -> ValidationFinding {
    ValidationFinding::new(ValidationFindingSeverity::Error, code)
}

const fn warning(code: ValidationFindingCode) -> ValidationFinding {
    ValidationFinding::new(ValidationFindingSeverity::Warning, code)
}

fn append_command_presentation_findings(
    object: roxmltree::Node<'_, '_>,
    language_codes: &[String],
    findings: &mut Vec<ValidationFinding>,
) {
    let Some(properties) = child(object, "Properties") else {
        return;
    };
    let list_presentation = child(properties, "ListPresentation");
    let synonym = child(properties, "Synonym");
    for language in language_codes {
        let text = localized_text(list_presentation, language)
            .filter(|value| !value.is_empty())
            .or_else(|| localized_text(synonym, language).filter(|value| !value.is_empty()));
        if text.is_some_and(|value| value.chars().count() > 38) {
            findings.push(warning(ValidationFindingCode::CommandPresentationTooLong));
        }
    }
}

fn localized_text<'a>(
    property: Option<roxmltree::Node<'a, 'a>>,
    language: &str,
) -> Option<&'a str> {
    const CORE_NS: &str = "http://v8.1c.ru/8.1/data/core";
    property?
        .children()
        .filter(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(CORE_NS)
                && node.tag_name().name() == "item"
        })
        .find_map(|item| {
            let lang = item.children().find(|node| {
                node.is_element()
                    && node.tag_name().namespace() == Some(CORE_NS)
                    && node.tag_name().name() == "lang"
            })?;
            (lang.text() == Some(language)).then(|| {
                item.children()
                    .find(|node| {
                        node.is_element()
                            && node.tag_name().namespace() == Some(CORE_NS)
                            && node.tag_name().name() == "content"
                    })
                    .and_then(|node| node.text())
                    .unwrap_or("")
            })
        })
}

fn child<'a>(node: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
            && child.tag_name().name() == name
    })
}

fn inner_text(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants()
        .filter_map(|child| child.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn contract_error(error: impl std::fmt::Display) -> SourceAdapterError {
    SourceAdapterError::new(
        SourceAdapterErrorKind::ProjectionAmbiguous,
        error.to_string(),
    )
}
