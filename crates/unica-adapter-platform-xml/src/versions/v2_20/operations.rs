use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use roxmltree::Node;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityRequirement, AuthorabilityResult, CompatibilityIssue, CompatibilityIssueKind,
        CompatibilityResult, FormatDiagnostic, FormatDiagnosticCode, FormatDiagnosticDetail,
        ObjectKindSelector, OperationalEvidenceRevision, OwnerResolutionMode, SemanticArtifactRole,
        SupportState, SupportSummary, ValidationContext, ValidationContextResult,
        ValidationIssueKind, ValidationMethodReferenceStatus, ValidationOwnerKind,
    },
    source::{
        ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext,
        SourceFamily,
    },
};

use crate::safe_root::{
    ArtifactReadLimit, BoundArtifact, DirectoryPageLimit, DirectoryVisit, SafeArtifactRead,
    SafeRootError, SafeSourceRoot,
};

use super::{profile, schema, semantic_map, support, xml};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureFailure {
    WrongFamily,
    UnauthorizedOrUnreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EvidenceScope {
    Operation,
    Validation,
    Tree,
}

#[derive(Debug)]
pub(crate) struct PlatformOperationSession {
    provider: Option<LazyPlatformSource>,
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

    pub(crate) fn capture_validation(source: &SourceContext, mode: OwnerResolutionMode) -> Self {
        Self::capture_with_scope(source, mode, EvidenceScope::Validation)
    }

    pub(crate) fn capture_object(
        source_root: &Path,
        selector: &ObjectKindSelector,
        object_name: &str,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        if !crate::domain::identifiers::is_1c_identifier(object_name) {
            return Self::failed(CaptureFailure::UnauthorizedOrUnreadable);
        }
        let Some(kind) = semantic_map::writer_object_kind(selector.as_str()) else {
            return Self::failed(CaptureFailure::UnauthorizedOrUnreadable);
        };
        let Some(directory) = semantic_map::native_descriptor_directory(kind) else {
            return Self::failed(CaptureFailure::UnauthorizedOrUnreadable);
        };
        Self::capture_paths(
            &source_root
                .join(directory)
                .join(format!("{object_name}.xml")),
            source_root,
            authorized_root,
            None,
            false,
            matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
            EvidenceScope::Operation,
        )
    }

    pub(crate) fn capture_named_object(
        source_root: &Path,
        object_name: &str,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        if !crate::domain::identifiers::is_1c_identifier(object_name) {
            return Self::failed(CaptureFailure::UnauthorizedOrUnreadable);
        }
        let root = match SafeSourceRoot::capture(authorized_root, source_root) {
            Ok(root) => root,
            Err(_) => return Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
        };
        let direct = format!("{object_name}.xml");
        let mut candidates = match root.exists_regular(&direct) {
            Ok(true) => vec![source_root.join(&direct)],
            Ok(false) => Vec::new(),
            Err(_) => return Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
        };
        let nested = semantic_map::top_level_descriptor_profiles()
            .filter_map(|profile| {
                let directory = profile.native_directory.as_deref()?;
                let relative = format!("{directory}/{object_name}.xml");
                match root.exists_regular(&relative) {
                    Ok(true) => Some(Ok(source_root.join(relative))),
                    Ok(false) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect::<Result<Vec<_>, _>>();
        let Ok(nested) = nested else {
            return Self::failed(CaptureFailure::UnauthorizedOrUnreadable);
        };
        candidates.extend(nested);
        let target = match candidates.as_mut_slice() {
            [target] => target.clone(),
            _ => return Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
        };
        Self::capture_paths(
            &target,
            source_root,
            authorized_root,
            None,
            false,
            matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
            EvidenceScope::Operation,
        )
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
            source.location().workspace_root(),
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
        Self::capture_unscoped_with_scope(target, authorized_root, mode, EvidenceScope::Operation)
    }

    pub(crate) fn capture_unscoped_validation(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_unscoped_with_scope(target, authorized_root, mode, EvidenceScope::Validation)
    }

    pub(crate) fn capture_unscoped_tree(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_unscoped_with_scope(target, authorized_root, mode, EvidenceScope::Tree)
    }

    fn capture_unscoped_with_scope(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
        evidence_scope: EvidenceScope,
    ) -> Self {
        let provider = match LazyPlatformSource::capture_unscoped(
            target,
            authorized_root,
            matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
        ) {
            Ok(provider) => provider,
            Err(_) => return Self::failed(CaptureFailure::UnauthorizedOrUnreadable),
        };
        let target_at_source_root = provider.target.is_source_root();
        Self {
            provider: Some(provider),
            configured_kind: None,
            configured_source_set: false,
            target_at_source_root,
            allow_missing_target: matches!(mode, OwnerResolutionMode::ExistingForNewOutput),
            evidence_scope,
            failure: None,
        }
    }

    fn capture_paths(
        target: &Path,
        source_root: &Path,
        authorized_root: &Path,
        configured_kind: Option<ConfiguredSourceSetKind>,
        configured_source_set: bool,
        allow_missing_target: bool,
        evidence_scope: EvidenceScope,
    ) -> Self {
        match LazyPlatformSource::capture(
            target,
            source_root,
            authorized_root,
            allow_missing_target,
        ) {
            Ok(provider) => {
                let target_at_source_root = provider.target.is_source_root();
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

    fn provider(&self) -> Result<&LazyPlatformSource, CaptureFailure> {
        self.provider.as_ref().ok_or_else(|| {
            self.failure
                .unwrap_or(CaptureFailure::UnauthorizedOrUnreadable)
        })
    }

    fn operation_provider(&self) -> Result<LazyPlatformSource, CaptureFailure> {
        self.provider()?
            .fork()
            .map_err(|_| CaptureFailure::UnauthorizedOrUnreadable)
    }

    pub(super) fn validation_provider(&self) -> Result<LazyPlatformSource, SafeRootError> {
        self.operation_provider()
            .map_err(|_| SafeRootError::Unauthorized)
    }

    pub(super) fn validation_subject(
        provider: &LazyPlatformSource,
    ) -> Result<SafeArtifactRead, SafeRootError> {
        if !provider.target.is_directory() {
            return provider
                .root
                .read_bound(&provider.target, ArtifactReadLimit::Descriptor);
        }
        for candidate in descriptor_candidates(provider.descriptor_key()) {
            match provider
                .root
                .read_relative(&candidate, ArtifactReadLimit::Descriptor)
            {
                Ok(read) => return Ok(read),
                Err(SafeRootError::Missing) => {}
                Err(error) => return Err(error),
            }
        }
        provider
            .root
            .read_relative("Configuration.xml", ArtifactReadLimit::Descriptor)
    }

    pub(crate) fn semantic_artifact_bytes(
        &self,
        role: SemanticArtifactRole,
    ) -> Result<Option<Vec<u8>>, SafeRootError> {
        let provider = self.provider().map_err(|_| SafeRootError::Unauthorized)?;
        if provider.target.is_missing() {
            return Ok(None);
        }
        let bytes = provider.read_target()?;
        let (_, document) =
            xml::parse_bounded_xml_document(&bytes).map_err(|_| SafeRootError::Unreadable)?;
        let root = document.root_element();
        let expected = match role {
            SemanticArtifactRole::FormDefinition => (Some(xml::FORM_DEFINITION_NS), "Form"),
            SemanticArtifactRole::DataCompositionSchema => (
                Some(xml::DATA_COMPOSITION_SCHEMA_NS),
                "DataCompositionSchema",
            ),
            SemanticArtifactRole::SpreadsheetDocument => {
                (Some(xml::SPREADSHEET_DOCUMENT_NS), "document")
            }
        };
        if (root.tag_name().namespace(), root.tag_name().name()) != expected {
            return Err(SafeRootError::Unreadable);
        }
        Ok(Some(bytes))
    }
}

#[derive(Debug)]
pub(super) struct LazyPlatformSource {
    root: SafeSourceRoot,
    target: BoundArtifact,
    descriptor_key: String,
}

impl LazyPlatformSource {
    fn fork(&self) -> Result<Self, SafeRootError> {
        Ok(Self {
            root: self.root.fork()?,
            target: self.target.clone(),
            descriptor_key: self.descriptor_key.clone(),
        })
    }

    pub(super) fn finalize_evidence(
        &self,
        operation: &'static [u8],
    ) -> Result<OperationalEvidenceRevision, SafeRootError> {
        self.root.finalize_evidence(operation)
    }

    fn capture(
        target: &Path,
        source_root: &Path,
        authorized_root: &Path,
        allow_missing_target: bool,
    ) -> Result<Self, SafeRootError> {
        let root = SafeSourceRoot::capture(authorized_root, source_root)?;
        let target = root.bind_target(target, allow_missing_target)?;
        let descriptor_key = target.relative_key().unwrap_or_default();
        Ok(Self {
            root,
            target,
            descriptor_key,
        })
    }

    fn capture_unscoped(
        target: &Path,
        authorized_root: &Path,
        allow_missing_target: bool,
    ) -> Result<Self, SafeRootError> {
        let boundary_root = SafeSourceRoot::capture(authorized_root, authorized_root)?;
        let target = boundary_root.bind_target(target, allow_missing_target)?;
        let target_is_directory = target.is_directory();
        let nearest_existing_directory = if target_is_directory {
            target.relative().to_path_buf()
        } else {
            target
                .relative()
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .to_path_buf()
        };
        let mut candidate = nearest_existing_directory.as_path();
        let mut source_prefix = standalone_source_prefix(&boundary_root, &target)?;
        if source_prefix.is_none() {
            loop {
                let key = if candidate.as_os_str().is_empty() {
                    "Configuration.xml".to_string()
                } else {
                    format!(
                        "{}/Configuration.xml",
                        candidate
                            .to_str()
                            .ok_or(SafeRootError::Unauthorized)?
                            .replace('\\', "/")
                    )
                };
                if boundary_root.exists_regular(&key)? {
                    source_prefix = Some(candidate.to_path_buf());
                    break;
                }
                if candidate.as_os_str().is_empty() {
                    break;
                }
                candidate = candidate.parent().unwrap_or_else(|| Path::new(""));
            }
        }
        let source_prefix = source_prefix.unwrap_or_else(|| {
            unscoped_source_prefix(
                target.relative(),
                target_is_directory,
                &nearest_existing_directory,
            )
        });
        let root = boundary_root.subroot(&source_prefix)?;
        let target = boundary_root.rebase_artifact(&target, &source_prefix)?;
        let descriptor_key = target.relative_key().unwrap_or_default();
        Ok(Self {
            root,
            target,
            descriptor_key,
        })
    }

    fn descriptor_key(&self) -> &str {
        if self.descriptor_key.is_empty() {
            "Configuration.xml"
        } else {
            &self.descriptor_key
        }
    }

    fn read_relative(&self, relative: impl AsRef<Path>) -> Result<Vec<u8>, SafeRootError> {
        let relative = relative
            .as_ref()
            .to_str()
            .ok_or(SafeRootError::Unreadable)?;
        self.root
            .read_relative(relative, ArtifactReadLimit::Descriptor)
            .map(|read| read.into_bytes())
    }

    fn read_target(&self) -> Result<Vec<u8>, SafeRootError> {
        self.root
            .read_bound(&self.target, ArtifactReadLimit::Descriptor)
            .map(|read| read.into_bytes())
    }

    fn tree_xml_keys(&self) -> Result<Vec<String>, SafeRootError> {
        if !self.target.is_directory() {
            return Ok(vec![self.descriptor_key().to_string()]);
        }
        let mut keys = Vec::new();
        let mut selected = 0usize;
        self.collect_tree_xml_keys(self.target.relative(), 0, &mut selected, &mut keys)?;
        Ok(keys)
    }

    fn collect_tree_xml_keys(
        &self,
        directory: &Path,
        depth: usize,
        selected: &mut usize,
        keys: &mut Vec<String>,
    ) -> Result<(), SafeRootError> {
        if depth > 64 {
            return Err(SafeRootError::LimitExceeded);
        }
        let directory_key = directory
            .to_str()
            .ok_or(SafeRootError::Unreadable)?
            .replace('\\', "/");
        let mut child_directories = Vec::new();
        self.root.visit_directory(
            &directory_key,
            DirectoryPageLimit::MetadataRegistry,
            |name| {
                let Some(name) = name.to_str() else {
                    return Ok(DirectoryVisit::Ignore);
                };
                let child = directory.join(name);
                if Path::new(name)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
                {
                    keys.push(
                        child
                            .to_str()
                            .ok_or(SafeRootError::Unreadable)?
                            .replace('\\', "/"),
                    );
                    *selected = selected
                        .checked_add(1)
                        .ok_or(SafeRootError::LimitExceeded)?;
                    if *selected > DirectoryPageLimit::MetadataRegistry.entries() {
                        return Err(SafeRootError::LimitExceeded);
                    }
                    return Ok(DirectoryVisit::Selected);
                }
                match self.root.is_directory(&child) {
                    Ok(true) => {
                        child_directories.push(child);
                        *selected = selected
                            .checked_add(1)
                            .ok_or(SafeRootError::LimitExceeded)?;
                        if *selected > DirectoryPageLimit::MetadataRegistry.entries() {
                            return Err(SafeRootError::LimitExceeded);
                        }
                        Ok(DirectoryVisit::Selected)
                    }
                    Ok(false)
                    | Err(SafeRootError::LinkOrReparsePoint)
                    | Err(SafeRootError::NotRegular) => Ok(DirectoryVisit::Ignore),
                    Err(error) => Err(error),
                }
            },
        )?;
        for child in child_directories {
            self.collect_tree_xml_keys(&child, depth + 1, selected, keys)?;
        }
        Ok(())
    }

    fn configuration_bytes(&self) -> Result<Option<Vec<u8>>, SafeRootError> {
        optional_read(
            &self.root,
            "Configuration.xml",
            ArtifactReadLimit::Descriptor,
        )
    }

    fn parent_configurations_bytes(&self) -> Result<Option<Vec<u8>>, SafeRootError> {
        optional_read(
            &self.root,
            "Ext/ParentConfigurations.bin",
            ArtifactReadLimit::SupportEvidence,
        )
    }

    fn root_xml_keys(&self) -> Result<Vec<String>, SafeRootError> {
        let mut keys = Vec::new();
        self.root
            .visit_directory("", DirectoryPageLimit::RootDiscovery, |name| {
                let Some(name) = name.to_str() else {
                    return Ok(DirectoryVisit::Ignore);
                };
                if Path::new(name).extension().and_then(|value| value.to_str()) != Some("xml") {
                    return Ok(DirectoryVisit::Ignore);
                }
                let bytes = self
                    .root
                    .read_relative(name, ArtifactReadLimit::Descriptor)?
                    .into_bytes();
                if !is_config_dump_sidecar(&bytes) {
                    keys.push(name.to_string());
                }
                Ok(DirectoryVisit::Selected)
            })?;
        Ok(keys)
    }
}

fn standalone_source_prefix(
    root: &SafeSourceRoot,
    target: &BoundArtifact,
) -> Result<Option<PathBuf>, SafeRootError> {
    if target.is_directory() || target.is_missing() {
        return Ok(None);
    }
    let bytes = root
        .read_bound(target, ArtifactReadLimit::Descriptor)?
        .into_bytes();
    let (_, document) = match xml::parse_bounded_xml_document(&bytes) {
        Ok(document) => document,
        Err(_) => return Ok(None),
    };
    let root_element = document.root_element();
    let Some(object) = root_element
        .children()
        .find(|node| node.is_element() && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS))
    else {
        return Ok(None);
    };
    let is_standalone = semantic_map::metadata_class_profile(object.tag_name().name())
        .is_some_and(|profile| profile.role == schema::MetadataClassRole::StandaloneObject);
    Ok(is_standalone.then(|| {
        target
            .relative()
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf()
    }))
}

fn optional_read(
    root: &SafeSourceRoot,
    relative: &str,
    limit: ArtifactReadLimit,
) -> Result<Option<Vec<u8>>, SafeRootError> {
    match root.read_relative(relative, limit) {
        Ok(read) => Ok(Some(read.into_bytes())),
        Err(SafeRootError::Missing) => Ok(None),
        Err(error) => Err(error),
    }
}

fn unscoped_source_prefix(
    target: &Path,
    target_is_directory: bool,
    nearest_existing_directory: &Path,
) -> PathBuf {
    if let Some(root) = embedded_content_capture_root(target) {
        if nearest_existing_directory.starts_with(&root) {
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

#[derive(Debug)]
enum CompatibilityDecision {
    Compatible,
    Incompatible(CompatibilityIssue),
}

impl CompatibilityDecision {
    fn issue_kind(&self) -> Option<CompatibilityIssueKind> {
        match self {
            Self::Compatible => None,
            Self::Incompatible(issue) => Some(issue.kind()),
        }
    }

    fn into_result(self, evidence_revision: OperationalEvidenceRevision) -> CompatibilityResult {
        match self {
            Self::Compatible => CompatibilityResult::compatible(evidence_revision),
            Self::Incompatible(issue) => {
                CompatibilityResult::incompatible(issue, evidence_revision)
            }
        }
    }
}

pub(crate) fn compatibility(
    session: &PlatformOperationSession,
) -> Result<CompatibilityResult, SourceAdapterError> {
    let provider = match session.operation_provider() {
        Ok(provider) => provider,
        Err(_) => return Err(source_unavailable("compatibility source is unavailable")),
    };
    let decision = compatibility_with_provider(session, &provider);
    let evidence_revision = provider
        .finalize_evidence(b"compatibility")
        .map_err(|_| source_unavailable("compatibility evidence could not be finalized"))?;
    Ok(decision.into_result(evidence_revision))
}

fn compatibility_with_provider(
    session: &PlatformOperationSession,
    provider: &LazyPlatformSource,
) -> CompatibilityDecision {
    let keys = match compatibility_keys(session, provider) {
        Ok(keys) => keys,
        Err(_) => {
            return incompatible(
                CompatibilityIssueKind::Malformed,
                FormatDiagnosticCode::SourceMalformed,
                "Compatibility evidence could not be read safely.",
            )
        }
    };
    if keys.is_empty() {
        let root_is_empty = match provider.root_xml_keys() {
            Ok(keys) => keys.is_empty(),
            Err(_) => {
                return incompatible(
                    CompatibilityIssueKind::Malformed,
                    FormatDiagnosticCode::SourceMalformed,
                    "Source-root evidence could not be read safely.",
                )
            }
        };
        if session.allow_missing_target
            && (!session.configured_source_set
                || session.target_at_source_root
                || root_is_empty
                || provider.target.is_missing())
        {
            return CompatibilityDecision::Compatible;
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
        match result.issue_kind() {
            None => {}
            Some(CompatibilityIssueKind::Newer | CompatibilityIssueKind::Malformed) => {
                return result
            }
            Some(CompatibilityIssueKind::Older) if older.is_none() => older = Some(result),
            Some(CompatibilityIssueKind::Older) => {}
        }
    }
    if recognized_owner {
        older.unwrap_or(CompatibilityDecision::Compatible)
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
) -> Result<Option<(crate::owner::SnapshotOwner, CompatibilityDecision)>, CompatibilityDecision> {
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
        return Ok(Some((owner, CompatibilityDecision::Compatible)));
    }
    let result = match profile::classify_root_version(owner.version.as_deref()) {
        Ok(profile::FormatCompatibility::Supported { .. }) => CompatibilityDecision::Compatible,
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
    provider: &LazyPlatformSource,
) -> Result<Vec<String>, SafeRootError> {
    if session.evidence_scope == EvidenceScope::Tree {
        return provider.tree_xml_keys();
    }
    let mut keys = Vec::new();
    let configuration = provider.configuration_bytes()?;
    if session.target_at_source_root && configuration.is_none() {
        keys.extend(provider.root_xml_keys()?);
    } else {
        for key in descriptor_candidates(provider.descriptor_key()) {
            match provider.read_relative(&key) {
                Ok(_) => keys.push(key),
                Err(SafeRootError::Missing) => {}
                Err(error) => return Err(error),
            }
        }
    }
    if configuration.is_some() {
        keys.push("Configuration.xml".to_string());
    }
    if session.evidence_scope == EvidenceScope::Validation {
        keys.extend(validation_compatibility_keys(provider)?);
    }
    let mut seen = BTreeSet::new();
    keys.retain(|key| seen.insert(key.clone()));
    Ok(keys)
}

fn validation_compatibility_keys(
    provider: &LazyPlatformSource,
) -> Result<Vec<String>, SafeRootError> {
    let Some(target) = target_descriptor(provider)? else {
        return Ok(Vec::new());
    };
    let mut keys = vec![target.key.clone()];
    if let Some(configuration) = configuration_descriptor(provider)? {
        if command_text_validation_required(&target.native_type) {
            for language in configuration.languages {
                if let Some(item) = read_semantic_descriptor(provider, "Language", &language)? {
                    keys.push(item.key.clone());
                }
            }
        }
    }
    keys.extend(resolve_target_dependencies(provider, &target)?.keys);
    if let Some(Ok((module, _))) = &target.method_reference {
        if let Some(item) = read_semantic_descriptor(provider, "CommonModule", module)? {
            keys.push(item.key.clone());
        }
    }
    Ok(keys)
}

fn is_config_dump_sidecar(bytes: &[u8]) -> bool {
    xml::parse_bounded_xml_document(bytes)
        .ok()
        .is_some_and(|(_, document)| document.root_element().tag_name().name() == "ConfigDumpInfo")
}

fn incompatible(
    kind: CompatibilityIssueKind,
    code: FormatDiagnosticCode,
    _internal_message: &'static str,
) -> CompatibilityDecision {
    CompatibilityDecision::Incompatible(CompatibilityIssue::new(
        kind,
        FormatDiagnostic::new(code, FormatDiagnosticDetail::Compatibility(kind))
            .expect("compatibility diagnostic mapping is closed"),
    ))
}

pub(crate) fn authorability(
    session: &PlatformOperationSession,
    requirement: AuthorabilityRequirement,
) -> Result<AuthorabilityResult, SourceAdapterError> {
    let provider = match session.operation_provider() {
        Ok(provider) => provider,
        Err(_) => return Ok(AuthorabilityResult::source_unreadable()),
    };
    enum Decision {
        Allowed(SupportSummary),
        Denied(Authorability, SupportSummary, FormatDiagnostic),
        Unreadable,
    }
    let decision = (|| {
        let support_bytes = match provider.parent_configurations_bytes() {
            Ok(bytes) => bytes,
            Err(_) => return Decision::Unreadable,
        };
        let facts = support::read_support_facts_bytes(support_bytes.as_deref());
        if facts.parse_error().is_some() {
            return Decision::Unreadable;
        }
        let object_uuid = if support_bytes.is_some() {
            match descriptor_uuid(&provider) {
                Ok(uuid) => uuid,
                Err(()) => return Decision::Unreadable,
            }
        } else {
            String::new()
        };
        let effective = facts.effective_rule_for(&object_uuid);
        let state = support_state(effective);
        let summary = match SupportSummary::new(
            state,
            facts.global_editing_enabled(),
            facts.vendors().len(),
            facts.rule_counts(),
        ) {
            Ok(summary) => summary,
            Err(_) => return Decision::Unreadable,
        };
        let authorability = facts.authorability_for(&object_uuid);
        let violation = if requirement == AuthorabilityRequirement::Removed {
            match effective {
                support::EffectiveSupportRule::Removed => None,
                support::EffectiveSupportRule::ConfigurationReadOnly => Some(support_violation(
                    FormatDiagnosticCode::SupportCapabilityDisabled,
                    state,
                )),
                _ => Some(support_violation(
                    FormatDiagnosticCode::SupportRemovalRequired,
                    state,
                )),
            }
        } else {
            match authorability {
                Authorability::Authorable => None,
                Authorability::ConfigurationReadOnly => Some(support_violation(
                    FormatDiagnosticCode::SupportCapabilityDisabled,
                    state,
                )),
                Authorability::SupportLocked => Some(support_violation(
                    FormatDiagnosticCode::SupportLocked,
                    state,
                )),
                Authorability::UnknownSupportState
                | Authorability::UnknownReadOnly
                | Authorability::DerivedReadOnly => Some(support_violation(
                    FormatDiagnosticCode::SupportStateUnreadable,
                    SupportState::Unreadable,
                )),
            }
        };
        match violation {
            None => Decision::Allowed(summary),
            Some(diagnostic) => Decision::Denied(authorability, summary, diagnostic),
        }
    })();
    let evidence = match provider.finalize_evidence(b"authorability") {
        Ok(evidence) => evidence,
        Err(_) => return Ok(AuthorabilityResult::source_unreadable()),
    };
    Ok(match decision {
        Decision::Allowed(summary) => AuthorabilityResult::allowed(summary, evidence.clone())
            .unwrap_or_else(|_| unreadable_authorability(evidence)),
        Decision::Denied(authorability, summary, diagnostic) => {
            AuthorabilityResult::denied(authorability, summary, diagnostic, evidence.clone())
                .unwrap_or_else(|_| unreadable_authorability(evidence))
        }
        Decision::Unreadable => unreadable_authorability(evidence),
    })
}

fn unreadable_authorability(evidence: OperationalEvidenceRevision) -> AuthorabilityResult {
    AuthorabilityResult::denied(
        Authorability::UnknownSupportState,
        SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3])
            .expect("unreadable support summary is valid"),
        support_violation(
            FormatDiagnosticCode::SupportStateUnreadable,
            SupportState::Unreadable,
        ),
        evidence,
    )
    .expect("unreadable support denial is valid")
}

fn support_violation(code: FormatDiagnosticCode, state: SupportState) -> FormatDiagnostic {
    FormatDiagnostic::new(code, FormatDiagnosticDetail::Support(state))
        .expect("support diagnostic mapping is closed")
}

fn support_state(rule: support::EffectiveSupportRule) -> SupportState {
    match rule {
        support::EffectiveSupportRule::Absent => SupportState::Absent,
        support::EffectiveSupportRule::Removed => SupportState::Removed,
        support::EffectiveSupportRule::Editable => SupportState::Editable,
        support::EffectiveSupportRule::Locked => SupportState::Locked,
        support::EffectiveSupportRule::ConfigurationReadOnly => SupportState::ConfigurationReadOnly,
        support::EffectiveSupportRule::UnknownReadOnly => SupportState::UnknownReadOnly,
        support::EffectiveSupportRule::Unreadable => SupportState::Unreadable,
    }
}

pub(crate) fn validation(
    session: &PlatformOperationSession,
) -> Result<ValidationContextResult, SourceAdapterError> {
    let provider = match session.validation_provider() {
        Ok(provider) => provider,
        Err(_) => return Err(source_unavailable("validation source is unavailable")),
    };
    let result = validation_context_for_provider(session, &provider);
    let evidence = provider
        .finalize_evidence(b"validation-context")
        .map_err(|_| source_unavailable("validation evidence could not be finalized"))?;
    Ok(match result {
        Ok(context) => ValidationContextResult::valid(context, evidence),
        Err(issue) => invalid_validation(issue, evidence),
    })
}

pub(super) fn validation_context_for_provider(
    session: &PlatformOperationSession,
    provider: &LazyPlatformSource,
) -> Result<ValidationContext, ValidationIssueKind> {
    let target = target_descriptor(provider)
        .map_err(|_| ValidationIssueKind::SourceUnreadable)?
        .ok_or(ValidationIssueKind::SourceUnreadable)?;
    if matches!(
        target.native_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        let method_reference_status = method_reference_status(provider, &target)
            .map_err(|_| ValidationIssueKind::SourceUnreadable)?;
        let dependencies = resolve_target_dependencies(provider, &target)
            .map_err(|_| ValidationIssueKind::SourceUnreadable)?;
        return ValidationContext::new(
            ValidationOwnerKind::Standalone,
            Vec::new(),
            command_text_validation_required(&target.native_type),
            dependencies.references_present,
            dependencies.registrar_coverage,
            method_reference_status,
        )
        .map_err(|_| ValidationIssueKind::SourceUnreadable);
    }
    let configuration = match configuration_descriptor(provider) {
        Ok(configuration) => configuration,
        Err(_) => return Err(ValidationIssueKind::SourceUnreadable),
    };
    let Some(configuration) = configuration else {
        return Err(ValidationIssueKind::OwnerUnavailable);
    };
    if !configuration
        .registrations
        .contains(&(target.native_type.clone(), target.name.clone()))
    {
        return Err(ValidationIssueKind::RegistrationMissing);
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
            let language = read_semantic_descriptor(provider, "Language", language_name)
                .map_err(|_| ValidationIssueKind::SourceUnreadable)?;
            let Some(code) = language.and_then(|language| language.language_code) else {
                return Err(ValidationIssueKind::LanguageProfileMissing);
            };
            if seen.insert(code.clone()) {
                language_codes.push(code);
            }
        }
        if language_codes.is_empty() {
            return Err(ValidationIssueKind::LanguageProfileMissing);
        }
    }
    let dependencies = resolve_target_dependencies(provider, &target)
        .map_err(|_| ValidationIssueKind::SourceUnreadable)?;
    let method_reference_status = method_reference_status(provider, &target)
        .map_err(|_| ValidationIssueKind::SourceUnreadable)?;
    ValidationContext::new(
        owner_kind,
        language_codes,
        requires_languages,
        dependencies.references_present,
        dependencies.registrar_coverage,
        method_reference_status,
    )
    .map_err(|_| ValidationIssueKind::SourceUnreadable)
}

pub(super) fn invalid_validation(
    issue: ValidationIssueKind,
    evidence: OperationalEvidenceRevision,
) -> ValidationContextResult {
    let _internal_message = match issue {
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
    ValidationContextResult::invalid(
        vec![FormatDiagnostic::new(
            match issue {
                ValidationIssueKind::ReferenceMissing => {
                    FormatDiagnosticCode::ValidationReferenceMissing
                }
                ValidationIssueKind::RegistrarMissing => {
                    FormatDiagnosticCode::ValidationRegistrarMissing
                }
                _ => FormatDiagnosticCode::ValidationContextUnavailable,
            },
            FormatDiagnosticDetail::Validation(issue),
        )
        .expect("validation diagnostic mapping is closed")],
        evidence,
    )
    .expect("validation diagnostics are non-empty")
}

#[derive(Debug, Clone)]
struct DescriptorIdentity {
    key: String,
    native_type: String,
    name: String,
    language_code: Option<String>,
    references: Option<Vec<(String, String)>>,
    requires_registrar: bool,
    method_reference: Option<Result<(String, String), ()>>,
}

fn target_descriptor(
    provider: &LazyPlatformSource,
) -> Result<Option<DescriptorIdentity>, SafeRootError> {
    let candidates = descriptor_candidates(provider.descriptor_key());
    for candidate in candidates {
        match provider.read_relative(&candidate) {
            Ok(bytes) => {
                return descriptor_identity(&candidate, &bytes)
                    .map(Some)
                    .ok_or(SafeRootError::Unreadable)
            }
            Err(SafeRootError::Missing) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}

fn semantic_descriptor_key(native_type: &str, name: &str) -> Option<String> {
    if !crate::domain::identifiers::is_1c_identifier(name) {
        return None;
    }
    let profile = semantic_map::metadata_class_profile(native_type)?;
    let directory = profile.native_directory.as_deref()?;
    Some(format!("{directory}/{name}.xml"))
}

fn read_semantic_descriptor(
    provider: &LazyPlatformSource,
    native_type: &str,
    name: &str,
) -> Result<Option<DescriptorIdentity>, SafeRootError> {
    let Some(key) = semantic_descriptor_key(native_type, name) else {
        return Ok(None);
    };
    let bytes = match provider.read_relative(&key) {
        Ok(bytes) => bytes,
        Err(SafeRootError::Missing) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(descriptor_identity(&key, &bytes)
        .filter(|descriptor| descriptor.native_type == native_type && descriptor.name == name))
}

#[derive(Debug)]
struct TargetDependencyResolution {
    keys: Vec<String>,
    references_present: Option<bool>,
    registrar_coverage: unica_format_core::ports::ValidationRelationCoverage,
}

fn resolve_target_dependencies(
    provider: &LazyPlatformSource,
    target: &DescriptorIdentity,
) -> Result<TargetDependencyResolution, SafeRootError> {
    let mut keys = BTreeSet::new();
    let references_present = match target.references.as_ref() {
        None => None,
        Some(references) => {
            let mut present = true;
            for (native_type, name) in references {
                match read_semantic_descriptor(provider, native_type, name)? {
                    Some(descriptor) => {
                        keys.insert(descriptor.key);
                    }
                    None => present = false,
                }
            }
            Some(present)
        }
    };
    let registrar_coverage = if target.requires_registrar {
        unica_format_core::ports::ValidationRelationCoverage::NotEvaluated
    } else {
        unica_format_core::ports::ValidationRelationCoverage::NotApplicable
    };
    Ok(TargetDependencyResolution {
        keys: keys.into_iter().collect(),
        references_present,
        registrar_coverage,
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
    let requires_registrar = matches!(
        native_type,
        "AccumulationRegister" | "AccountingRegister" | "CalculationRegister"
    ) || (native_type == "InformationRegister"
        && properties
            .and_then(|properties| child(properties, "WriteMode"))
            .map(inner_text)
            .as_deref()
            == Some("RecorderSubordinate"));
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
        requires_registrar,
        method_reference,
    })
}

#[derive(Debug)]
struct ConfigurationDescriptor {
    registrations: BTreeSet<(String, String)>,
    languages: Vec<String>,
    is_extension: bool,
}

fn configuration_descriptor(
    provider: &LazyPlatformSource,
) -> Result<Option<ConfigurationDescriptor>, SafeRootError> {
    let Some(bytes) = provider.configuration_bytes()? else {
        return Ok(None);
    };
    let (_, document) =
        xml::parse_bounded_xml_document(&bytes).map_err(|_| SafeRootError::Unreadable)?;
    let root = document.root_element();
    let configuration = root
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
                && node.tag_name().name() == "Configuration"
        })
        .ok_or(SafeRootError::Unreadable)?;
    let properties = child(configuration, "Properties");
    let is_extension = properties
        .is_some_and(|properties| child(properties, "ConfigurationExtensionPurpose").is_some());
    let mut registrations = BTreeSet::new();
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
    Ok(Some(ConfigurationDescriptor {
        registrations,
        languages,
        is_extension,
    }))
}

fn reference_values(node: Node<'_, '_>) -> Vec<(String, String)> {
    let items = node
        .children()
        .filter(|item| item.is_element() && item.tag_name().name() == "Item")
        .filter_map(|item| {
            inner_text(item)
                .split_once('.')
                .map(|(kind, name)| (kind.to_string(), name.to_string()))
        })
        .collect::<Vec<_>>();
    if !items.is_empty() {
        return items;
    }
    inner_text(node)
        .split_once('.')
        .map(|(kind, name)| vec![(kind.to_string(), name.to_string())])
        .unwrap_or_default()
}

pub(super) fn source_unavailable(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
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
    provider: &LazyPlatformSource,
    target: &DescriptorIdentity,
) -> Result<Option<ValidationMethodReferenceStatus>, SafeRootError> {
    let Some(reference) = target.method_reference.as_ref() else {
        return Ok(None);
    };
    let (module, procedure) = match reference {
        Ok(reference) => reference,
        Err(()) => return Ok(Some(ValidationMethodReferenceStatus::Invalid)),
    };
    let Some(module_descriptor) = read_semantic_descriptor(provider, "CommonModule", module)?
    else {
        return Ok(Some(ValidationMethodReferenceStatus::TargetMissing));
    };
    let descriptor = Path::new(&module_descriptor.key);
    let Some(stem) = descriptor.file_stem().and_then(|value| value.to_str()) else {
        return Ok(Some(ValidationMethodReferenceStatus::ImplementationMissing));
    };
    let implementation = descriptor
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(stem)
        .join("Ext")
        .join("Module.bsl");
    let bytes = match provider.read_relative(&implementation) {
        Ok(bytes) => bytes,
        Err(SafeRootError::Missing) => {
            return Ok(Some(ValidationMethodReferenceStatus::ImplementationMissing))
        }
        Err(error) => return Err(error),
    };
    let Ok(source) = std::str::from_utf8(&bytes) else {
        return Ok(Some(ValidationMethodReferenceStatus::Invalid));
    };
    if bsl_has_export(source, procedure) {
        Ok(Some(ValidationMethodReferenceStatus::Valid))
    } else {
        Ok(Some(ValidationMethodReferenceStatus::EntryPointMissing))
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

fn descriptor_uuid(provider: &LazyPlatformSource) -> Result<String, ()> {
    let mut bytes = None;
    for candidate in descriptor_candidates(provider.descriptor_key()) {
        match provider.read_relative(candidate) {
            Ok(read) => {
                bytes = Some(read);
                break;
            }
            Err(SafeRootError::Missing) => {}
            Err(_) => return Err(()),
        }
    }
    let bytes = match bytes {
        Some(bytes) => bytes,
        None => provider.configuration_bytes().map_err(|_| ())?.ok_or(())?,
    };
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
            | (
                Some(xml::DATA_COMPOSITION_SCHEMA_NS),
                "DataCompositionSchema"
            )
    )
}

fn version_is_inherited_when_missing(root: Node<'_, '_>) -> bool {
    (root.tag_name().namespace(), root.tag_name().name())
        == (
            Some("http://v8.1c.ru/8.2/managed-application/core"),
            "ClientApplicationInterface",
        )
}

#[cfg(test)]
mod fix_round3_tests {
    use std::{
        fs::{self, File},
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::safe_root::{with_artifact_open_log, with_evidence_finalization_failure};
    use unica_format_core::source::{SourceContext, SourceLocation};

    fn fixture_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-validation-read-log-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn write_fixture(root: &Path, unrelated: impl FnOnce(&Path)) -> PlatformOperationSession {
        fs::create_dir_all(root.join("Catalogs")).unwrap();
        fs::create_dir_all(root.join("Languages")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Language>English</Language><Catalog>Target</Catalog><Catalog>Unrelated</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Languages/English.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="00000000-0000-0000-0000-000000000004"><Properties><Name>English</Name><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let target = root.join("Catalogs/Target.xml");
        fs::write(
            &target,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000002"><Properties><Name>Target</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        )
        .unwrap();
        unrelated(&root.join("Catalogs/Unrelated.xml"));
        let source = SourceContext::new(
            SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        );
        PlatformOperationSession::capture_validation(&source, OwnerResolutionMode::Existing)
    }

    #[test]
    fn validation_open_log_excludes_every_unrelated_registered_descriptor_state() {
        let mut fixtures = Vec::new();

        let readable = fixture_root("readable");
        let readable_session = write_fixture(&readable, |path| {
            fs::write(
                path,
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000003"><Properties><Name>Unrelated</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
            )
            .unwrap();
        });
        fixtures.push((readable, readable_session));

        let malformed = fixture_root("malformed");
        let malformed_session = write_fixture(&malformed, |path| {
            fs::write(path, b"<broken").unwrap();
        });
        fixtures.push((malformed, malformed_session));

        let oversized = fixture_root("oversized");
        let oversized_session = write_fixture(&oversized, |path| {
            File::create(path)
                .unwrap()
                .set_len(70 * 1024 * 1024)
                .unwrap();
        });
        fixtures.push((oversized, oversized_session));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let linked = fixture_root("linked");
            let outside = fixture_root("outside");
            fs::write(&outside, b"outside").unwrap();
            let linked_session = write_fixture(&linked, |path| {
                symlink(&outside, path).unwrap();
            });
            fixtures.push((linked, linked_session));
            fs::remove_file(outside).unwrap();
        }

        for (root, session) in fixtures {
            let (_, opens) = with_artifact_open_log(|| validation(&session));
            assert!(
                opens
                    .iter()
                    .all(|path| path != Path::new("Catalogs/Unrelated.xml")),
                "unrelated descriptor was opened: {opens:?}"
            );
            assert!(
                opens
                    .iter()
                    .any(|path| path == Path::new("Catalogs/Target.xml")),
                "target descriptor was not opened: {opens:?}"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    fn assert_source_unavailable(error: SourceAdapterError) {
        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);
    }

    #[test]
    fn evidence_finalization_failure_cannot_return_cacheable_success() {
        let compatibility_root = fixture_root("finalize-compatibility");
        let compatibility_session = write_fixture(&compatibility_root, |path| {
            fs::write(path, b"<broken").unwrap();
        });
        assert_source_unavailable(
            with_evidence_finalization_failure(|| compatibility(&compatibility_session))
                .unwrap_err(),
        );
        fs::remove_dir_all(compatibility_root).unwrap();

        let authorability_root = fixture_root("finalize-authorability");
        let authorability_session = write_fixture(&authorability_root, |path| {
            fs::write(path, b"<broken").unwrap();
        });
        let denied = with_evidence_finalization_failure(|| {
            authorability(&authorability_session, AuthorabilityRequirement::Editable)
        })
        .unwrap();
        assert!(!denied.is_allowed());
        assert_eq!(denied.authorability(), Authorability::UnknownSupportState);
        assert_eq!(denied.summary().state(), SupportState::Unreadable);
        assert!(denied.evidence_revision().is_none());
        fs::remove_dir_all(authorability_root).unwrap();

        let context_root = fixture_root("finalize-context");
        let context_session = write_fixture(&context_root, |path| {
            fs::write(path, b"<broken").unwrap();
        });
        assert_source_unavailable(
            with_evidence_finalization_failure(|| validation(&context_session)).unwrap_err(),
        );
        fs::remove_dir_all(context_root).unwrap();

        let validation_root = fixture_root("finalize-validation");
        let validation_session = write_fixture(&validation_root, |path| {
            fs::write(path, b"<broken").unwrap();
        });
        let handle = unica_format_core::ports::OperationalSourceSession::new(validation_session);
        assert_source_unavailable(
            with_evidence_finalization_failure(|| {
                super::super::validation::validate(
                    &[handle],
                    unica_format_core::ports::ValidationOptions::new(true, 100).unwrap(),
                )
            })
            .unwrap_err(),
        );
        fs::remove_dir_all(validation_root).unwrap();
    }

    fn write_direct_reference_fixture(
        root: &Path,
        unrelated: impl FnOnce(&Path, &Path, &Path),
    ) -> PlatformOperationSession {
        for directory in [
            "Documents",
            "InformationRegisters",
            "AccumulationRegisters",
            "Languages",
        ] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="10000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Language>English</Language><Document>Target</Document><Document>Malformed</Document><Document>Oversized</Document><Document>Unreadable</Document><InformationRegister>Stock</InformationRegister><AccumulationRegister>Balance</AccumulationRegister></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Languages/English.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="10000000-0000-0000-0000-000000000002"><Properties><Name>English</Name><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let target = root.join("Documents/Target.xml");
        fs::write(
            &target,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Document uuid="10000000-0000-0000-0000-000000000003"><Properties><Name>Target</Name><RegisterRecords><xr:Item>InformationRegister.Stock</xr:Item><xr:Item>AccumulationRegister.Balance</xr:Item></RegisterRecords></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("InformationRegisters/Stock.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><InformationRegister uuid="10000000-0000-0000-0000-000000000004"><Properties><Name>Stock</Name><WriteMode>RecorderSubordinate</WriteMode></Properties><ChildObjects/></InformationRegister></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("AccumulationRegisters/Balance.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><AccumulationRegister uuid="10000000-0000-0000-0000-000000000005"><Properties><Name>Balance</Name></Properties><ChildObjects/></AccumulationRegister></MetaDataObject>"#,
        )
        .unwrap();
        unrelated(
            &root.join("Documents/Malformed.xml"),
            &root.join("Documents/Oversized.xml"),
            &root.join("Documents/Unreadable.xml"),
        );
        let source = SourceContext::new(
            SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        );
        PlatformOperationSession::capture_validation(&source, OwnerResolutionMode::Existing)
    }

    #[test]
    fn direct_multi_register_facts_open_only_explicit_references() {
        let root = fixture_root("direct-register-records");
        let outside = fixture_root("direct-register-records-outside");
        fs::write(&outside, b"outside").unwrap();
        let session = write_direct_reference_fixture(&root, |malformed, oversized, unreadable| {
            fs::write(malformed, b"<broken").unwrap();
            File::create(oversized)
                .unwrap()
                .set_len(70 * 1024 * 1024)
                .unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&outside, unreadable).unwrap();
            #[cfg(not(unix))]
            fs::write(unreadable, b"<broken").unwrap();
        });

        let (context, validation_opens) = with_artifact_open_log(|| validation(&session).unwrap());
        let context = context
            .context()
            .expect("direct-reference context is valid");
        assert_eq!(context.references_present(), Some(true));
        assert_eq!(
            context.registrar_coverage(),
            unica_format_core::ports::ValidationRelationCoverage::NotApplicable
        );
        for expected in [
            "Documents/Target.xml",
            "InformationRegisters/Stock.xml",
            "AccumulationRegisters/Balance.xml",
        ] {
            assert!(
                validation_opens
                    .iter()
                    .any(|path| path == Path::new(expected)),
                "missing targeted read {expected}: {validation_opens:?}"
            );
        }
        for forbidden in [
            "Documents/Malformed.xml",
            "Documents/Oversized.xml",
            "Documents/Unreadable.xml",
        ] {
            assert!(
                validation_opens
                    .iter()
                    .all(|path| path != Path::new(forbidden)),
                "opened unrelated registrar candidate {forbidden}: {validation_opens:?}"
            );
        }

        let compatibility_session =
            write_direct_reference_fixture(&root, |_malformed, _oversized, _unreadable| {});
        let (compatibility, compatibility_opens) =
            with_artifact_open_log(|| compatibility(&compatibility_session).unwrap());
        assert!(compatibility.issue().is_none(), "{compatibility:?}");
        for forbidden in [
            "Documents/Malformed.xml",
            "Documents/Oversized.xml",
            "Documents/Unreadable.xml",
        ] {
            assert!(
                compatibility_opens
                    .iter()
                    .all(|path| path != Path::new(forbidden)),
                "compatibility opened unrelated registrar candidate {forbidden}: {compatibility_opens:?}"
            );
        }

        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    fn write_register_target_fixture(
        root: &Path,
        unrelated: impl FnOnce(&Path, &Path, &Path),
    ) -> PlatformOperationSession {
        for directory in ["Documents", "InformationRegisters", "Languages"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="30000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Language>English</Language><Document>RecorderA</Document><Document>RecorderB</Document><Document>Malformed</Document><Document>Oversized</Document><Document>Unreadable</Document><InformationRegister>Stock</InformationRegister></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Languages/English.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="30000000-0000-0000-0000-000000000005"><Properties><Name>English</Name><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let target = root.join("InformationRegisters/Stock.xml");
        fs::write(
            &target,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><InformationRegister uuid="30000000-0000-0000-0000-000000000002"><Properties><Name>Stock</Name><WriteMode>RecorderSubordinate</WriteMode></Properties><ChildObjects/></InformationRegister></MetaDataObject>"#,
        )
        .unwrap();
        for (name, uuid) in [
            ("RecorderA", "30000000-0000-0000-0000-000000000003"),
            ("RecorderB", "30000000-0000-0000-0000-000000000004"),
        ] {
            fs::write(
                root.join(format!("Documents/{name}.xml")),
                format!(
                    r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Document uuid="{uuid}"><Properties><Name>{name}</Name><RegisterRecords><xr:Item>InformationRegister.Stock</xr:Item></RegisterRecords></Properties><ChildObjects/></Document></MetaDataObject>"#
                ),
            )
            .unwrap();
        }
        unrelated(
            &root.join("Documents/Malformed.xml"),
            &root.join("Documents/Oversized.xml"),
            &root.join("Documents/Unreadable.xml"),
        );
        let source = SourceContext::new(
            SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        );
        PlatformOperationSession::capture_validation(&source, OwnerResolutionMode::Existing)
    }

    #[test]
    fn register_target_without_reverse_fields_is_partial_and_opens_no_documents() {
        let register_profile =
            super::super::semantic_map::metadata_class_profile("InformationRegister").unwrap();
        let write_mode = super::super::semantic_map::property_mapping(
            super::super::semantic_map::object_kind(register_profile),
            "WriteMode",
        )
        .unwrap();
        assert!(
            super::super::semantic_map::enum_value(
                super::super::semantic_map::object_kind(register_profile),
                write_mode.semantic_id,
                "RecorderSubordinate",
            )
            .is_some(),
            "the authoritative registry must recognize recorder-subordinate write mode"
        );
        let root = fixture_root("register-target-local");
        let outside = fixture_root("register-target-local-outside");
        fs::write(&outside, b"outside").unwrap();
        let session = write_register_target_fixture(&root, |malformed, oversized, unreadable| {
            fs::write(malformed, b"<broken").unwrap();
            File::create(oversized)
                .unwrap()
                .set_len(70 * 1024 * 1024)
                .unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&outside, unreadable).unwrap();
            #[cfg(not(unix))]
            fs::write(unreadable, b"<broken").unwrap();
        });

        let (context, context_opens) = with_artifact_open_log(|| validation(&session).unwrap());
        let context = context
            .context()
            .unwrap_or_else(|| panic!("register context diagnostics: {:?}", context.diagnostics()));
        assert_eq!(
            context.registrar_coverage(),
            unica_format_core::ports::ValidationRelationCoverage::NotEvaluated
        );
        assert!(context_opens
            .iter()
            .all(|path| !path.starts_with("Documents")));

        let handle = unica_format_core::ports::OperationalSourceSession::new(session);
        let (result, report_opens) = with_artifact_open_log(|| {
            super::super::validation::validate(
                &[handle],
                unica_format_core::ports::ValidationOptions::new(true, 100).unwrap(),
            )
            .unwrap()
        });
        let report = &result.reports()[0];
        assert_eq!(
            report.coverage(),
            unica_format_core::ports::ValidationCoverage::Partial
        );
        assert_eq!(
            report.status(),
            unica_format_core::ports::ValidationStatus::Partial
        );
        assert!(report.findings().iter().any(|finding| {
            finding.code()
                == unica_format_core::ports::ValidationFindingCode::RegistrarCoverageNotEvaluated
                && finding.severity()
                    == unica_format_core::ports::ValidationFindingSeverity::Warning
        }));
        assert!(report.findings().iter().all(|finding| {
            finding.code() != unica_format_core::ports::ValidationFindingCode::RegistrarMissing
        }));
        assert!(report_opens
            .iter()
            .all(|path| !path.starts_with("Documents")));

        fs::remove_dir_all(root).unwrap();
        fs::remove_file(outside).unwrap();
    }

    #[test]
    fn document_missing_explicit_register_target_is_reference_missing() {
        let root = fixture_root("missing-forward-register");
        fs::create_dir_all(root.join("Documents")).unwrap();
        fs::create_dir_all(root.join("InformationRegisters")).unwrap();
        fs::create_dir_all(root.join("Languages")).unwrap();
        fs::write(
            root.join("Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="40000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Language>English</Language><Document>Target</Document><InformationRegister>Missing</InformationRegister></ChildObjects></Configuration></MetaDataObject>"#,
        )
        .unwrap();
        fs::write(
            root.join("Languages/English.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="40000000-0000-0000-0000-000000000003"><Properties><Name>English</Name><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
        )
        .unwrap();
        let target = root.join("Documents/Target.xml");
        fs::write(
            &target,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Document uuid="40000000-0000-0000-0000-000000000002"><Properties><Name>Target</Name><RegisterRecords><xr:Item>InformationRegister.Missing</xr:Item></RegisterRecords></Properties><ChildObjects/></Document></MetaDataObject>"#,
        )
        .unwrap();
        let source = SourceContext::new(
            SourceLocation::new(root.clone(), root.clone(), target),
            Some("main".to_string()),
            SourceFamily::PlatformXml,
            None,
        );
        let session =
            PlatformOperationSession::capture_validation(&source, OwnerResolutionMode::Existing);

        let context = validation(&session).unwrap();
        let context = context.context().expect("document context is valid");
        assert_eq!(context.references_present(), Some(false));
        assert_eq!(
            context.registrar_coverage(),
            unica_format_core::ports::ValidationRelationCoverage::NotApplicable
        );

        let result = super::super::validation::validate(
            &[unica_format_core::ports::OperationalSourceSession::new(
                session,
            )],
            unica_format_core::ports::ValidationOptions::new(true, 100).unwrap(),
        )
        .unwrap();
        let report = &result.reports()[0];
        assert_eq!(
            report.coverage(),
            unica_format_core::ports::ValidationCoverage::Complete
        );
        assert!(report.findings().iter().any(|finding| {
            finding.code() == unica_format_core::ports::ValidationFindingCode::ReferenceMissing
        }));
        assert!(report.findings().iter().all(|finding| {
            finding.code() != unica_format_core::ports::ValidationFindingCode::RegistrarMissing
        }));

        fs::remove_dir_all(root).unwrap();
    }
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

#[cfg(test)]
mod authorization_order_tests {
    use super::*;
    use crate::safe_root::with_before_artifact_open;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_format_core::source::{SourceContext, SourceLocation};

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "unica-task7-authorization-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn wrong_family_is_denied_before_any_artifact_open() {
        let root = temp_root("wrong-family");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("untrusted.xml");
        std::fs::write(&target, b"must not be read").unwrap();
        let source = SourceContext::new(
            SourceLocation::new(root.clone(), root.clone(), target),
            None,
            SourceFamily::Edt,
            None,
        );

        let result = with_before_artifact_open(
            |_| panic!("artifact opened before family authorization"),
            || {
                let session =
                    PlatformOperationSession::capture(&source, OwnerResolutionMode::Existing);
                authorability(&session, AuthorabilityRequirement::Editable)
            },
        );
        assert!(matches!(result, Ok(AuthorabilityResult::Denied(_))));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn out_of_root_target_is_denied_before_any_artifact_open() {
        let root = temp_root("root");
        let outside = temp_root("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&outside, b"must not be read").unwrap();
        let source = SourceContext::new(
            SourceLocation::new(root.clone(), root.clone(), outside.clone()),
            None,
            SourceFamily::PlatformXml,
            None,
        );

        let result = with_before_artifact_open(
            |_| panic!("outside artifact opened before containment authorization"),
            || {
                let session =
                    PlatformOperationSession::capture(&source, OwnerResolutionMode::Existing);
                authorability(&session, AuthorabilityRequirement::Editable)
            },
        );
        assert!(matches!(result, Ok(AuthorabilityResult::Denied(_))));
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_file(outside).unwrap();
    }
}
