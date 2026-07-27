use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
};

use roxmltree::Node;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityRequirement, AuthorabilityResult, AuthorabilityViolation,
        CompatibilityIssue, CompatibilityIssueKind, CompatibilityResult, FormatDiagnostic,
        FormatDiagnosticCode, FormatDiagnosticDetail, OwnerResolutionMode,
        SupportState, SupportSummary, ValidationContext, ValidationContextResult,
        ValidationIssueKind, ValidationMethodReferenceStatus, ValidationOwnerKind,
    },
    source::{
        ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext,
        SourceFamily,
    },
};

use super::{profile, provider::PlatformXmlProvider, schema, support, xml};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureFailure {
    WrongFamily,
    UnauthorizedOrUnreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceScope {
    Operation,
    Validation,
}

#[derive(Debug)]
pub(crate) struct PlatformOperationSession {
    provider: Option<PlatformXmlProvider>,
    configured_kind: Option<ConfiguredSourceSetKind>,
    configured_source_set: bool,
    target_at_source_root: bool,
    allow_missing_target: bool,
    evidence_scope: EvidenceScope,
    failure: Option<CaptureFailure>,
}

impl PlatformOperationSession {
    pub(crate) fn capture(source: &SourceContext, mode: OwnerResolutionMode) -> Self {
        Self::capture_with_scope(source, mode, EvidenceScope::Operation)
    }

    pub(crate) fn capture_validation(
        source: &SourceContext,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_with_scope(source, mode, EvidenceScope::Validation)
    }

    fn capture_with_scope(
        source: &SourceContext,
        mode: OwnerResolutionMode,
        evidence_scope: EvidenceScope,
    ) -> Self {
        if source.declared_family() != &SourceFamily::PlatformXml {
            return Self::failed(CaptureFailure::WrongFamily);
        }
        Self::capture_paths(
            source.location().target(),
            source.location().source_root(),
            source.configured_source_set_kind(),
            source.configured_source_set().is_some(),
            matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
            evidence_scope,
        )
    }

    pub(crate) fn capture_unscoped(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_unscoped_with_scope(
            target,
            authorized_root,
            mode,
            EvidenceScope::Operation,
        )
    }

    pub(crate) fn capture_unscoped_validation(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_unscoped_with_scope(
            target,
            authorized_root,
            mode,
            EvidenceScope::Validation,
        )
    }

    fn capture_unscoped_with_scope(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
        evidence_scope: EvidenceScope,
    ) -> Self {
        let location =
            match PlatformXmlProvider::authorize_unscoped_target(target, authorized_root) {
                Ok(location) => location,
                Err(_) => return Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
            };
        let source_root = location.configuration_root.unwrap_or_else(|| {
            unscoped_source_root(
                &location.target,
                &location.boundary,
                location.target_is_directory,
                &location.nearest_existing_directory,
            )
        });
        Self::capture_paths(
            &location.target,
            &source_root,
            None,
            false,
            matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
            evidence_scope,
        )
    }

    fn capture_paths(
        target: &Path,
        source_root: &Path,
        configured_kind: Option<ConfiguredSourceSetKind>,
        configured_source_set: bool,
        allow_missing_target: bool,
        evidence_scope: EvidenceScope,
    ) -> Self {
        match PlatformXmlProvider::capture_operational(target, source_root, true) {
            Ok(provider) => {
                let target_at_source_root = fs::canonicalize(target)
                    .ok()
                    .is_some_and(|target| target == provider.captured_source_root());
                Self {
                    provider: Some(provider),
                    configured_kind,
                    configured_source_set,
                    target_at_source_root,
                    allow_missing_target,
                    evidence_scope,
                    failure: None,
                }
            }
            Err(_) => Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
        }
    }

    fn failed(failure: CaptureFailure) -> Self {
        Self {
            provider: None,
            configured_kind: None,
            configured_source_set: false,
            target_at_source_root: false,
            allow_missing_target: false,
            evidence_scope: EvidenceScope::Operation,
            failure: Some(failure),
        }
    }

    fn provider(&self) -> Result<&PlatformXmlProvider, CaptureFailure> {
        self.provider.as_ref().ok_or_else(|| {
            self.failure
                .unwrap_or(CaptureFailure::UnauthorizedOrUnreadable)
        })
    }
}

fn unscoped_source_root(
    target: &Path,
    boundary: &Path,
    target_is_directory: bool,
    nearest_existing_directory: &Path,
) -> PathBuf {
    if let Some(root) = embedded_content_capture_root(target) {
        if root.starts_with(boundary) && nearest_existing_directory.starts_with(&root) {
            return root;
        }
    }
    if target_is_directory && nearest_existing_directory == target {
        target.to_path_buf()
    } else {
        nearest_existing_directory.to_path_buf()
    }
}

fn embedded_content_capture_root(target: &Path) -> Option<PathBuf> {
    let file = target.file_name()?.to_str()?;
    let extension = target.parent()?;
    if extension.file_name()?.to_str()? != "Ext" {
        return None;
    }
    let item = extension.parent()?;
    let collection = item.parent()?;
    let expected_collection = match file {
        "Form.xml" => "Forms",
        "Template.xml" => "Templates",
        "Rights.xml" => "Roles",
        "CommandInterface.xml" => "Subsystems",
        _ => return None,
    };
    (collection.file_name()?.to_str()? == expected_collection).then(|| collection.to_path_buf())
}

pub(crate) fn compatibility(session: &PlatformOperationSession) -> CompatibilityResult {
    let provider = match session.provider() {
        Ok(provider) => provider,
        Err(CaptureFailure::WrongFamily) => {
            return incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceFamilyIncompatible,
                "The selected source belongs to another source family.",
            )
        }
        Err(CaptureFailure::UnauthorizedOrUnreadable) => {
            return incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceMalformed,
                "The selected source could not be authorized and captured.",
            )
        }
    };
    let keys = compatibility_keys(session, provider);
    if keys.is_empty() {
        if session.allow_missing_target
            && (!session.configured_source_set
                || session.target_at_source_root
                || provider
                    .snapshot_files()
                    .all(|(key, _)| !key.ends_with(".xml")))
        {
            return CompatibilityResult::compatible();
        }
        return incompatible(
            CompatibilityIssueKind::Malformed,
            FormatDiagnosticCode::SourceMalformed,
            "The selected source has no authoritative revision owner.",
        );
    }
    let mut older = None;
    let mut recognized_owner = false;
    for key in keys {
        let Ok(bytes) = provider.read_relative(&key) else {
            return incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceMalformed,
                "Captured compatibility evidence is incomplete.",
            );
        };
        let Some((owner, result)) = (match compatibility_bytes(&bytes) {
            Ok(result) => result,
            Err(result) => return result,
        }) else {
            continue;
        };
        recognized_owner = true;
        if let Some(expected) = session.configured_kind {
            let expected = match expected {
                ConfiguredSourceSetKind::Configuration => crate::owner::OwnerKind::Configuration,
                ConfiguredSourceSetKind::Extension => crate::owner::OwnerKind::Extension,
                ConfiguredSourceSetKind::ExternalProcessor => {
                    crate::owner::OwnerKind::ExternalProcessor
                }
                ConfiguredSourceSetKind::ExternalReport => crate::owner::OwnerKind::ExternalReport,
            };
            if owner.kind != crate::owner::OwnerKind::Standalone && owner.kind != expected {
                return incompatible(
                    CompatibilityIssueKind::Malformed,
                    FormatDiagnosticCode::SourceMalformed,
                    "The captured owner does not match the configured source kind.",
                );
            }
        }
        match result.issue().map(|issue| issue.kind()) {
            None => {}
            Some(CompatibilityIssueKind::Newer | CompatibilityIssueKind::Malformed) => {
                return result
            }
            Some(CompatibilityIssueKind::Older) if older.is_none() => older = Some(result),
            Some(CompatibilityIssueKind::Older) => {}
        }
    }
    if recognized_owner {
        older.unwrap_or_else(CompatibilityResult::compatible)
    } else {
        incompatible(
            CompatibilityIssueKind::Malformed,
            FormatDiagnosticCode::SourceMalformed,
            "The selected source has no authoritative revision owner.",
        )
    }
}

fn compatibility_bytes(
    bytes: &[u8],
) -> Result<
    Option<(crate::owner::SnapshotOwner, CompatibilityResult)>,
    CompatibilityResult,
> {
    let (_, document) = match xml::parse_bounded_xml_document(&bytes) {
        Ok(document) => document,
        Err(_) => {
            return Err(incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceMalformed,
                "The source revision owner is malformed.",
            ))
        }
    };
    let root = document.root_element();
    let owner = match crate::owner::parse_snapshot_owner(bytes) {
        Ok(owner) => owner,
        Err(_) if root.attribute("version").is_none() => return Ok(None),
        Err(_) => {
            return Err(incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceMalformed,
                "The captured revision owner is invalid.",
            ))
        }
    };
    if root.attribute("version").is_none()
        && (is_versionless_embedded(root) || version_is_inherited_when_missing(root))
    {
        return Ok(Some((owner, CompatibilityResult::compatible())));
    }
    let result = match profile::classify_root_version(owner.version.as_deref()) {
        Ok(profile::FormatCompatibility::Supported { .. }) => CompatibilityResult::compatible(),
        Ok(profile::FormatCompatibility::Older { .. }) => incompatible(
            CompatibilityIssueKind::Older,
            FormatDiagnosticCode::SourceRevisionOlder,
            "The source revision is older than the writable revision. Explicitly migrate it with its native producer before editing.",
        ),
        Ok(profile::FormatCompatibility::Newer { .. }) => incompatible(
            CompatibilityIssueKind::Newer,
            FormatDiagnosticCode::SourceRevisionNewer,
            "The source revision is newer than this adapter supports.",
        ),
        Err(_) => incompatible(
            CompatibilityIssueKind::Malformed,
            FormatDiagnosticCode::SourceMalformed,
            "The source revision declaration is malformed.",
        ),
    };
    Ok(Some((owner, result)))
}

fn compatibility_keys(
    session: &PlatformOperationSession,
    provider: &PlatformXmlProvider,
) -> Vec<String> {
    let mut keys = Vec::new();
    if session.target_at_source_root && provider.configuration_bytes().is_none() {
        keys.extend(
            provider
                .snapshot_files()
                .filter(|(key, _)| !key.contains('/') && key.ends_with(".xml"))
                .filter(|(_, bytes)| !is_config_dump_sidecar(bytes))
                .map(|(key, _)| key.to_string()),
        );
    } else {
        keys.extend(
            descriptor_candidates(provider.descriptor_key())
                .into_iter()
                .filter(|key| provider.read_relative(key).is_ok()),
        );
    }
    if provider.configuration_bytes().is_some() {
        keys.push("Configuration.xml".to_string());
    }
    if session.evidence_scope == EvidenceScope::Validation {
        keys.extend(validation_compatibility_keys(provider));
    }
    let mut seen = BTreeSet::new();
    keys.retain(|key| seen.insert(key.clone()));
    keys
}

fn validation_compatibility_keys(provider: &PlatformXmlProvider) -> Vec<String> {
    let descriptors = descriptor_index(provider);
    let Some(target) = target_descriptor(provider, &descriptors) else {
        return Vec::new();
    };
    let mut keys = vec![target.key.clone()];
    if let Some(configuration) = configuration_descriptor(provider) {
        if command_text_validation_required(&target.native_type) {
            for language in configuration.languages {
                if let Some(item) = descriptors
                    .items
                    .iter()
                    .find(|item| item.native_type == "Language" && item.name == language)
                {
                    keys.push(item.key.clone());
                }
            }
        }
    }
    if let Some(references) = &target.references {
        for reference in references {
            if let Some(item) = descriptors.items.iter().find(|item| {
                item.native_type == reference.0 && item.name == reference.1
            }) {
                keys.push(item.key.clone());
            }
        }
    }
    if let Some(reference) = &target.registrar_reference {
        for item in descriptors
            .items
            .iter()
            .filter(|item| item.native_type == "Document")
        {
            keys.push(item.key.clone());
            if item
                .references
                .as_ref()
                .is_some_and(|references| references.contains(reference))
            {
                break;
            }
        }
    }
    if let Some(Ok(reference)) = &target.method_reference {
        if let Some(item) = descriptors
            .items
            .iter()
            .find(|item| item.native_type == reference.0 && item.name == reference.1)
        {
            keys.push(item.key.clone());
        }
    }
    keys
}

fn is_config_dump_sidecar(bytes: &[u8]) -> bool {
    xml::parse_bounded_xml_document(bytes)
        .ok()
        .is_some_and(|(_, document)| document.root_element().tag_name().name() == "ConfigDumpInfo")
}

fn incompatible(
    kind: CompatibilityIssueKind,
    code: FormatDiagnosticCode,
    message: &'static str,
) -> CompatibilityResult {
    CompatibilityResult::incompatible(CompatibilityIssue::new(
        kind,
        FormatDiagnostic::new(code, message)
            .with_detail(FormatDiagnosticDetail::Compatibility(kind)),
    ))
}

pub(crate) fn authorability(
    session: &PlatformOperationSession,
    requirement: AuthorabilityRequirement,
) -> AuthorabilityResult {
    let provider = match session.provider() {
        Ok(provider) => provider,
        Err(_) => return unreadable_authorability(),
    };
    let support_bytes = provider.parent_configurations_bytes();
    let facts = support::read_support_facts_bytes(support_bytes.as_deref());
    if facts.parse_error().is_some() {
        return unreadable_authorability();
    }
    let object_uuid = if support_bytes.is_some() {
        match descriptor_uuid(provider) {
            Ok(uuid) => uuid,
            Err(()) => return unreadable_authorability(),
        }
    } else {
        String::new()
    };
    let effective = facts.effective_rule_for(&object_uuid);
    let state = support_state(effective);
    let summary = SupportSummary::new(
        state,
        facts.global_editing_enabled(),
        facts.vendors().len(),
        facts.rule_counts(),
    );
    let authorability = facts.authorability_for(&object_uuid);
    let violation = if requirement == AuthorabilityRequirement::Removed {
        match effective {
            support::EffectiveSupportRule::Removed => None,
            support::EffectiveSupportRule::ConfigurationReadOnly => Some(support_violation(
                FormatDiagnosticCode::SupportCapabilityDisabled,
                state,
                "Source editing is disabled by support policy.",
            )),
            _ => Some(support_violation(
                FormatDiagnosticCode::SupportRemovalRequired,
                state,
                "The object must be removed from support before this operation.",
            )),
        }
    } else {
        match authorability {
            Authorability::Authorable => None,
            Authorability::ConfigurationReadOnly => Some(support_violation(
                FormatDiagnosticCode::SupportCapabilityDisabled,
                state,
                "Source editing is disabled by support policy.",
            )),
            Authorability::SupportLocked => Some(support_violation(
                FormatDiagnosticCode::SupportLocked,
                state,
                "The object is locked by support policy.",
            )),
            Authorability::UnknownSupportState
            | Authorability::UnknownReadOnly
            | Authorability::DerivedReadOnly => Some(support_violation(
                FormatDiagnosticCode::SupportStateUnreadable,
                SupportState::Unreadable,
                "Support state is ambiguous; edit safety is not established.",
            )),
        }
    };
    AuthorabilityResult::new(authorability, summary, violation)
}

fn unreadable_authorability() -> AuthorabilityResult {
    AuthorabilityResult::new(
        Authorability::UnknownSupportState,
        SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3]),
        Some(support_violation(
            FormatDiagnosticCode::SupportStateUnreadable,
            SupportState::Unreadable,
            "The source could not be authorized and captured; edit safety is not established.",
        )),
    )
}

fn support_violation(
    code: FormatDiagnosticCode,
    state: SupportState,
    message: &'static str,
) -> AuthorabilityViolation {
    AuthorabilityViolation::new(
        FormatDiagnostic::new(code, message).with_detail(FormatDiagnosticDetail::Support(state)),
    )
}

fn support_state(rule: support::EffectiveSupportRule) -> SupportState {
    match rule {
        support::EffectiveSupportRule::Absent => SupportState::Absent,
        support::EffectiveSupportRule::Removed => SupportState::Removed,
        support::EffectiveSupportRule::Editable => SupportState::Editable,
        support::EffectiveSupportRule::Locked => SupportState::Locked,
        support::EffectiveSupportRule::ConfigurationReadOnly => {
            SupportState::ConfigurationReadOnly
        }
        support::EffectiveSupportRule::UnknownReadOnly => SupportState::UnknownReadOnly,
        support::EffectiveSupportRule::Unreadable => SupportState::Unreadable,
    }
}

pub(crate) fn validation(session: &PlatformOperationSession) -> ValidationContextResult {
    let provider = match session.provider() {
        Ok(provider) => provider,
        Err(_) => return invalid_validation(ValidationIssueKind::SourceUnreadable),
    };
    let descriptors = descriptor_index(provider);
    let Some(target) = target_descriptor(provider, &descriptors) else {
        return invalid_validation(ValidationIssueKind::SourceUnreadable);
    };
    if matches!(
        target.native_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        return valid_validation(
            ValidationOwnerKind::Standalone,
            Vec::new(),
            command_text_validation_required(&target.native_type),
            target.references.as_ref().map(|references| {
                references
                    .iter()
                    .all(|reference| descriptors.contains_identity(reference))
            }),
            None,
            method_reference_status(provider, target, &descriptors),
        );
    }
    let Some(configuration) = configuration_descriptor(provider) else {
        return invalid_validation(ValidationIssueKind::OwnerUnavailable);
    };
    if !configuration
        .registrations
        .contains(&(target.native_type.clone(), target.name.clone()))
    {
        return invalid_validation(ValidationIssueKind::RegistrationMissing);
    }
    let owner_kind = if session.configured_kind == Some(ConfiguredSourceSetKind::Extension)
        || configuration.is_extension
    {
        ValidationOwnerKind::Extension
    } else {
        ValidationOwnerKind::Aggregate
    };
    let requires_languages = command_text_validation_required(&target.native_type);
    let mut language_codes = Vec::new();
    if requires_languages {
        let mut seen = BTreeSet::new();
        for language_name in &configuration.languages {
            if let Some(code) = descriptors.language_code(language_name) {
                if seen.insert(code.clone()) {
                    language_codes.push(code.clone());
                }
            }
        }
        if language_codes.is_empty() {
            return invalid_validation(ValidationIssueKind::LanguageProfileMissing);
        }
    }
    let references_present = target.references.as_ref().map(|references| {
        references
            .iter()
            .all(|reference| descriptors.contains_identity(reference))
    });
    let registrar_present = target
        .registrar_reference
        .as_ref()
        .map(|reference| descriptors.document_references(reference));
    let method_reference_status = method_reference_status(provider, target, &descriptors);
    valid_validation(
        owner_kind,
        language_codes,
        requires_languages,
        references_present,
        registrar_present,
        method_reference_status,
    )
}

fn valid_validation(
    owner_kind: ValidationOwnerKind,
    language_codes: Vec<String>,
    command_text_validation_required: bool,
    references_present: Option<bool>,
    registrar_present: Option<bool>,
    method_reference_status: Option<ValidationMethodReferenceStatus>,
) -> ValidationContextResult {
    match ValidationContext::new(
        owner_kind,
        language_codes,
        command_text_validation_required,
        references_present,
        registrar_present,
        method_reference_status,
    ) {
        Ok(context) => ValidationContextResult::valid(context),
        Err(_) => invalid_validation(ValidationIssueKind::SourceUnreadable),
    }
}

fn invalid_validation(issue: ValidationIssueKind) -> ValidationContextResult {
    let message = match issue {
        ValidationIssueKind::SourceUnreadable => {
            "Validation source could not be authorized and captured."
        }
        ValidationIssueKind::OwnerUnavailable => {
            "The validation source has no authoritative aggregate owner."
        }
        ValidationIssueKind::RegistrationMissing => {
            "The metadata object is not registered by its aggregate owner."
        }
        ValidationIssueKind::LanguageProfileMissing => {
            "The aggregate has no complete registered language profile."
        }
        ValidationIssueKind::ReferenceMissing => "A referenced metadata object is missing.",
        ValidationIssueKind::RegistrarMissing => "A required registrar relationship is missing.",
    };
    ValidationContextResult::invalid(vec![FormatDiagnostic::new(
        FormatDiagnosticCode::ValidationContextUnavailable,
        message,
    )
    .with_detail(FormatDiagnosticDetail::Validation(issue))])
    .expect("validation diagnostics are non-empty")
}

#[derive(Debug, Clone)]
struct DescriptorIdentity {
    key: String,
    native_type: String,
    name: String,
    language_code: Option<String>,
    references: Option<Vec<(String, String)>>,
    registrar_reference: Option<(String, String)>,
    method_reference: Option<Result<(String, String), ()>>,
}

#[derive(Debug, Default)]
struct DescriptorIndex {
    items: Vec<DescriptorIdentity>,
    identities: HashSet<(String, String)>,
}

impl DescriptorIndex {
    fn contains_identity(&self, identity: &(String, String)) -> bool {
        self.identities.contains(identity)
    }

    fn language_code(&self, name: &str) -> Option<&String> {
        self.items
            .iter()
            .find(|item| item.native_type == "Language" && item.name == name)
            .and_then(|item| item.language_code.as_ref())
    }

    fn document_references(&self, reference: &(String, String)) -> bool {
        self.items.iter().any(|item| {
            item.native_type == "Document"
                && item
                    .references
                    .as_ref()
                    .is_some_and(|references| references.contains(reference))
        })
    }
}

fn descriptor_index(provider: &PlatformXmlProvider) -> DescriptorIndex {
    let mut index = DescriptorIndex::default();
    for (key, bytes) in provider.snapshot_files() {
        if !key.ends_with(".xml") {
            continue;
        }
        if let Some(identity) = descriptor_identity(key, &bytes) {
            index
                .identities
                .insert((identity.native_type.clone(), identity.name.clone()));
            index.items.push(identity);
        }
    }
    index
}

fn target_descriptor<'a>(
    provider: &PlatformXmlProvider,
    descriptors: &'a DescriptorIndex,
) -> Option<&'a DescriptorIdentity> {
    let candidates = descriptor_candidates(provider.descriptor_key());
    candidates
        .iter()
        .find_map(|candidate| descriptors.items.iter().find(|item| &item.key == candidate))
        .or_else(|| {
            (provider.descriptor_key() == "Configuration.xml")
                .then(|| {
                    descriptors
                        .items
                        .iter()
                        .find(|item| item.native_type == "Configuration")
                })
                .flatten()
        })
}

fn descriptor_candidates(target_key: &str) -> Vec<String> {
    let target = Path::new(target_key);
    let mut candidates = Vec::new();
    if target.extension().and_then(|value| value.to_str()) == Some("xml") {
        candidates.push(target_key.to_string());
    }
    let mut current = if target.extension().is_some() {
        target.parent()
    } else {
        Some(target)
    };
    while let Some(path) = current {
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            let candidate = path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(format!("{name}.xml"));
            if let Some(candidate) = candidate.to_str() {
                candidates.push(candidate.replace('\\', "/"));
            }
        }
        current = path.parent();
    }
    candidates
}

fn descriptor_identity(key: &str, bytes: &[u8]) -> Option<DescriptorIdentity> {
    let (_, document) = xml::parse_bounded_xml_document(bytes).ok()?;
    let root = document.root_element();
    if root.tag_name().namespace() != Some(xml::MD_CLASSES_NS)
        || root.tag_name().name() != "MetaDataObject"
    {
        return None;
    }
    let mut artifacts = root.children().filter(|node| {
        node.is_element() && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
    });
    let artifact = artifacts.next()?;
    if artifacts.next().is_some() {
        return None;
    }
    let native_type = artifact.tag_name().name();
    schema::metadata_class_profile(native_type)?;
    let properties = child(artifact, "Properties");
    let name = properties
        .and_then(|properties| child(properties, "Name"))
        .map(inner_text)
        .filter(|name| !name.is_empty())?;
    let references = properties
        .and_then(|properties| child(properties, "RegisterRecords"))
        .map(reference_values);
    let reads_registrars = matches!(
        native_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (native_type == "InformationRegister"
        && properties
            .and_then(|properties| child(properties, "WriteMode"))
            .map(inner_text)
            .as_deref()
            == Some("RecorderSubordinate"));
    let registrar_reference =
        reads_registrars.then(|| (native_type.to_string(), name.clone()));
    let language_code = (native_type == "Language")
        .then(|| {
            properties
                .and_then(|properties| child(properties, "LanguageCode"))
                .map(inner_text)
                .filter(|value| !value.is_empty())
        })
        .flatten();
    let method_reference = match native_type {
        "EventSubscription" => properties
            .and_then(|properties| child(properties, "Handler"))
            .map(inner_text)
            .filter(|value| !value.is_empty())
            .map(|value| parse_method_reference(&value)),
        "ScheduledJob" => properties
            .and_then(|properties| child(properties, "MethodName"))
            .map(inner_text)
            .filter(|value| !value.is_empty())
            .map(|value| parse_method_reference(&value)),
        _ => None,
    };
    Some(DescriptorIdentity {
        key: key.to_string(),
        native_type: native_type.to_string(),
        name,
        language_code,
        references,
        registrar_reference,
        method_reference,
    })
}

#[derive(Debug)]
struct ConfigurationDescriptor {
    registrations: HashSet<(String, String)>,
    languages: Vec<String>,
    is_extension: bool,
}

fn configuration_descriptor(provider: &PlatformXmlProvider) -> Option<ConfigurationDescriptor> {
    let bytes = provider.configuration_bytes()?;
    let (_, document) = xml::parse_bounded_xml_document(&bytes).ok()?;
    let root = document.root_element();
    let configuration = root.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
            && node.tag_name().name() == "Configuration"
    })?;
    let properties = child(configuration, "Properties");
    let is_extension =
        properties.is_some_and(|properties| child(properties, "ConfigurationExtensionPurpose").is_some());
    let mut registrations = HashSet::new();
    let mut languages = Vec::new();
    if let Some(children) = child(configuration, "ChildObjects") {
        for item in children.children().filter(Node::is_element) {
            if item.tag_name().namespace() != Some(xml::MD_CLASSES_NS)
                || schema::metadata_class_profile(item.tag_name().name()).is_none()
            {
                continue;
            }
            let name = inner_text(item);
            if name.is_empty() {
                continue;
            }
            if item.tag_name().name() == "Language" {
                languages.push(name);
            } else {
                registrations.insert((item.tag_name().name().to_string(), name));
            }
        }
    }
    Some(ConfigurationDescriptor {
        registrations,
        languages,
        is_extension,
    })
}

fn reference_values(node: Node<'_, '_>) -> Vec<(String, String)> {
    let items = node
        .children()
        .filter(|item| {
            item.is_element()
                && item.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
                && item.tag_name().name() == "Item"
        })
        .filter_map(|item| inner_text(item).split_once('.').map(|(kind, name)| {
            (kind.to_string(), name.to_string())
        }))
        .collect::<Vec<_>>();
    if !items.is_empty() {
        return items;
    }
    inner_text(node)
        .split_once('.')
        .map(|(kind, name)| vec![(kind.to_string(), name.to_string())])
        .unwrap_or_default()
}

fn child<'a>(node: Node<'a, 'a>, name: &str) -> Option<Node<'a, 'a>> {
    node.children().find(|candidate| {
        candidate.is_element()
            && candidate.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
            && candidate.tag_name().name() == name
    })
}

fn inner_text(node: Node<'_, '_>) -> String {
    node.descendants()
        .filter(Node::is_text)
        .filter_map(|descendant| descendant.text())
        .collect::<String>()
        .trim()
        .to_string()
}

fn command_text_validation_required(native_type: &str) -> bool {
    matches!(
        native_type,
        "ExchangePlan"
            | "Catalog"
            | "Document"
            | "DocumentJournal"
            | "Enum"
            | "ChartOfCharacteristicTypes"
            | "ChartOfAccounts"
            | "ChartOfCalculationTypes"
            | "InformationRegister"
            | "AccumulationRegister"
            | "AccountingRegister"
            | "CalculationRegister"
            | "BusinessProcess"
            | "Task"
    )
}

fn parse_method_reference(value: &str) -> Result<(String, String), ()> {
    let parts = value.split('.').collect::<Vec<_>>();
    match parts.as_slice() {
        ["CommonModule", module, procedure] if !module.is_empty() && !procedure.is_empty() => {
            Ok(((*module).to_string(), (*procedure).to_string()))
        }
        [module, procedure] if !module.is_empty() && !procedure.is_empty() => {
            Ok(((*module).to_string(), (*procedure).to_string()))
        }
        _ => Err(()),
    }
}

fn method_reference_status(
    provider: &PlatformXmlProvider,
    target: &DescriptorIdentity,
    descriptors: &DescriptorIndex,
) -> Option<ValidationMethodReferenceStatus> {
    let reference = target.method_reference.as_ref()?;
    let (module, procedure) = match reference {
        Ok(reference) => reference,
        Err(()) => return Some(ValidationMethodReferenceStatus::Invalid),
    };
    let Some(module_descriptor) = descriptors
        .items
        .iter()
        .find(|item| item.native_type == "CommonModule" && item.name == *module)
    else {
        return Some(ValidationMethodReferenceStatus::TargetMissing);
    };
    let descriptor = Path::new(&module_descriptor.key);
    let Some(stem) = descriptor.file_stem().and_then(|value| value.to_str()) else {
        return Some(ValidationMethodReferenceStatus::ImplementationMissing);
    };
    let implementation = descriptor
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(stem)
        .join("Ext")
        .join("Module.bsl");
    let Ok(bytes) = provider.read_relative(&implementation) else {
        return Some(ValidationMethodReferenceStatus::ImplementationMissing);
    };
    let Ok(source) = std::str::from_utf8(&bytes) else {
        return Some(ValidationMethodReferenceStatus::ImplementationMissing);
    };
    if bsl_has_export(source, procedure) {
        Some(ValidationMethodReferenceStatus::Valid)
    } else {
        Some(ValidationMethodReferenceStatus::EntryPointMissing)
    }
}

fn bsl_has_export(source: &str, procedure: &str) -> bool {
    let procedure = procedure.to_ascii_lowercase();
    source.lines().any(|line| {
        let normalized = line.trim().to_ascii_lowercase();
        (normalized.starts_with(&format!("procedure {procedure}("))
            && normalized.contains(" export"))
            || (normalized.starts_with(&format!("процедура {procedure}("))
                && normalized.contains(" экспорт"))
    })
}

fn descriptor_uuid(provider: &PlatformXmlProvider) -> Result<String, ()> {
    let bytes = descriptor_candidates(provider.descriptor_key())
        .into_iter()
        .find_map(|candidate| provider.read_relative(candidate).ok())
        .or_else(|| provider.configuration_bytes())
        .ok_or(())?;
    let (_, document) = xml::parse_bounded_xml_document(&bytes).map_err(|_| ())?;
    let root = document.root_element();
    let raw = root
        .attribute("uuid")
        .or_else(|| {
            root.children()
                .find(|node| node.is_element() && node.attribute("uuid").is_some())
                .and_then(|node| node.attribute("uuid"))
        })
        .ok_or(())?;
    uuid::Uuid::parse_str(raw)
        .map(|uuid| uuid.to_string())
        .map_err(|_| ())
}

fn is_versionless_embedded(root: Node<'_, '_>) -> bool {
    matches!(
        (root.tag_name().namespace(), root.tag_name().name()),
        (Some(xml::SPREADSHEET_DOCUMENT_NS), "document")
            | (Some(xml::DATA_COMPOSITION_SCHEMA_NS), "DataCompositionSchema")
    )
}

fn version_is_inherited_when_missing(root: Node<'_, '_>) -> bool {
    (
        root.tag_name().namespace(),
        root.tag_name().name(),
    ) == (
        Some("http://v8.1c.ru/8.2/managed-application/core"),
        "ClientApplicationInterface",
    )
}

pub(crate) fn session_from_handle(
    session: &unica_format_core::ports::OperationalSourceSession,
) -> Result<&PlatformOperationSession, SourceAdapterError> {
    session
        .adapter_state::<PlatformOperationSession>()
        .ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "operational session belongs to another adapter",
            )
        })
}
