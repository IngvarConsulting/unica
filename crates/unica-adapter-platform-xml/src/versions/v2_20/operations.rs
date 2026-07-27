use std::{
    collections::{BTreeSet, HashSet},
    path::{Path, PathBuf},
};

use roxmltree::Node;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityRequirement, AuthorabilityResult, CompatibilityIssue,
        CompatibilityIssueKind, CompatibilityResult, FormatDiagnostic, FormatDiagnosticCode,
        FormatDiagnosticDetail, ObjectKindSelector, OwnerResolutionMode, SemanticArtifactRole,
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

    pub(crate) fn capture_validation(
        source: &SourceContext,
        mode: OwnerResolutionMode,
    ) -> Self {
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

    pub(crate) fn capture_unscoped_tree(
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> Self {
        Self::capture_unscoped_with_scope(
            target,
            authorized_root,
            mode,
            EvidenceScope::Tree,
        )
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

    pub(crate) fn validation_subject(&self) -> Result<SafeArtifactRead, SafeRootError> {
        let provider = self.provider().map_err(|_| SafeRootError::Unauthorized)?;
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
            SemanticArtifactRole::FormDefinition => {
                (Some(xml::FORM_DEFINITION_NS), "Form")
            }
            SemanticArtifactRole::DataCompositionSchema => {
                (Some(xml::DATA_COMPOSITION_SCHEMA_NS), "DataCompositionSchema")
            }
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
struct LazyPlatformSource {
    root: SafeSourceRoot,
    target: BoundArtifact,
    descriptor_key: String,
}

impl LazyPlatformSource {
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
        let relative = relative.as_ref().to_str().ok_or(SafeRootError::Unreadable)?;
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
        self.collect_tree_xml_keys(
            self.target.relative(),
            0,
            &mut selected,
            &mut keys,
        )?;
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

    fn for_each_descriptor(
        &self,
        mut visitor: impl FnMut(&str, &[u8]),
    ) -> Result<(), SafeRootError> {
        let mut total = 0usize;
        self.root
            .visit_directory("", DirectoryPageLimit::MetadataRegistry, |name| {
                let Some(name) = name.to_str() else {
                    return Ok(DirectoryVisit::Ignore);
                };
                if Path::new(name).extension().and_then(|value| value.to_str()) != Some("xml") {
                    return Ok(DirectoryVisit::Ignore);
                }
                total = total
                    .checked_add(1)
                    .ok_or(SafeRootError::LimitExceeded)?;
                if total > DirectoryPageLimit::MetadataRegistry.entries() {
                    return Err(SafeRootError::LimitExceeded);
                }
                let bytes = self
                    .root
                    .read_relative(name, ArtifactReadLimit::Descriptor)?;
                visitor(name, bytes.bytes());
                Ok(DirectoryVisit::Selected)
            })?;
        for profile in semantic_map::top_level_descriptor_profiles() {
            let Some(directory) = profile.native_directory.as_deref() else {
                return Err(SafeRootError::Unreadable);
            };
            let visit = self.root.visit_directory(
                directory,
                DirectoryPageLimit::MetadataRegistry,
                |name| {
                    let Some(name) = name.to_str() else {
                        return Ok(DirectoryVisit::Ignore);
                    };
                    if Path::new(name).extension().and_then(|value| value.to_str()) != Some("xml") {
                        return Ok(DirectoryVisit::Ignore);
                    }
                    total = total
                        .checked_add(1)
                        .ok_or(SafeRootError::LimitExceeded)?;
                    if total > DirectoryPageLimit::MetadataRegistry.entries() {
                        return Err(SafeRootError::LimitExceeded);
                    }
                    let key = format!("{directory}/{name}");
                    let bytes = self
                        .root
                        .read_relative(&key, ArtifactReadLimit::Descriptor)?;
                    visitor(&key, bytes.bytes());
                    Ok(DirectoryVisit::Selected)
                },
            );
            match visit {
                Ok(()) => {}
                Err(SafeRootError::Missing) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
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
    let Some(object) = root_element.children().find(|node| {
        node.is_element() && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
    }) else {
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
    let descriptors = descriptor_index(provider)?;
    let Some(target) = target_descriptor(provider, &descriptors) else {
        return Ok(Vec::new());
    };
    let mut keys = vec![target.key.clone()];
    if let Some(configuration) = configuration_descriptor(provider)? {
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
) -> CompatibilityResult {
    CompatibilityResult::incompatible(CompatibilityIssue::new(
        kind,
        FormatDiagnostic::new(code, FormatDiagnosticDetail::Compatibility(kind))
            .expect("compatibility diagnostic mapping is closed"),
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
    let support_bytes = match provider.parent_configurations_bytes() {
        Ok(bytes) => bytes,
        Err(_) => return unreadable_authorability(),
    };
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
    let summary = match SupportSummary::new(
        state,
        facts.global_editing_enabled(),
        facts.vendors().len(),
        facts.rule_counts(),
    ) {
        Ok(summary) => summary,
        Err(_) => return unreadable_authorability(),
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
        None => AuthorabilityResult::allowed(summary)
            .unwrap_or_else(|_| unreadable_authorability()),
        Some(diagnostic) => AuthorabilityResult::denied(authorability, summary, diagnostic)
            .unwrap_or_else(|_| unreadable_authorability()),
    }
}

fn unreadable_authorability() -> AuthorabilityResult {
    AuthorabilityResult::denied(
        Authorability::UnknownSupportState,
        SupportSummary::new(SupportState::Unreadable, None, 0, [0; 3])
            .expect("unreadable support summary is valid"),
        support_violation(
            FormatDiagnosticCode::SupportStateUnreadable,
            SupportState::Unreadable,
        ),
    )
    .expect("unreadable support denial is valid")
}

fn support_violation(
    code: FormatDiagnosticCode,
    state: SupportState,
) -> FormatDiagnostic {
    FormatDiagnostic::new(code, FormatDiagnosticDetail::Support(state))
        .expect("support diagnostic mapping is closed")
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
    let descriptors = match descriptor_index(provider) {
        Ok(descriptors) => descriptors,
        Err(_) => return invalid_validation(ValidationIssueKind::SourceUnreadable),
    };
    let Some(target) = target_descriptor(provider, &descriptors) else {
        return invalid_validation(ValidationIssueKind::SourceUnreadable);
    };
    if matches!(
        target.native_type.as_str(),
        "ExternalReport" | "ExternalDataProcessor"
    ) {
        let method_reference_status = match method_reference_status(provider, target, &descriptors) {
            Ok(status) => status,
            Err(_) => return invalid_validation(ValidationIssueKind::SourceUnreadable),
        };
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
            method_reference_status,
        );
    }
    let configuration = match configuration_descriptor(provider) {
        Ok(configuration) => configuration,
        Err(_) => return invalid_validation(ValidationIssueKind::SourceUnreadable),
    };
    let Some(configuration) = configuration else {
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
            let Some(code) = descriptors.language_code(language_name) else {
                return invalid_validation(ValidationIssueKind::LanguageProfileMissing);
            };
            if seen.insert(code.clone()) {
                language_codes.push(code.clone());
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
    let method_reference_status = match method_reference_status(provider, target, &descriptors) {
        Ok(status) => status,
        Err(_) => return invalid_validation(ValidationIssueKind::SourceUnreadable),
    };
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
    ValidationContextResult::invalid(vec![FormatDiagnostic::new(
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
    .expect("validation diagnostic mapping is closed")])
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

fn descriptor_index(provider: &LazyPlatformSource) -> Result<DescriptorIndex, SafeRootError> {
    let mut index = DescriptorIndex::default();
    provider.for_each_descriptor(|key, bytes| {
        if let Some(identity) = descriptor_identity(key, bytes) {
            index
                .identities
                .insert((identity.native_type.clone(), identity.name.clone()));
            index.items.push(identity);
        }
    })?;
    Ok(index)
}

fn target_descriptor<'a>(
    provider: &LazyPlatformSource,
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

fn configuration_descriptor(
    provider: &LazyPlatformSource,
) -> Result<Option<ConfigurationDescriptor>, SafeRootError> {
    let Some(bytes) = provider.configuration_bytes()? else {
        return Ok(None);
    };
    let (_, document) =
        xml::parse_bounded_xml_document(&bytes).map_err(|_| SafeRootError::Unreadable)?;
    let root = document.root_element();
    let configuration = root.children().find(|node| {
        node.is_element()
            && node.tag_name().namespace() == Some(xml::MD_CLASSES_NS)
            && node.tag_name().name() == "Configuration"
    }).ok_or(SafeRootError::Unreadable)?;
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
    Ok(Some(ConfigurationDescriptor {
        registrations,
        languages,
        is_extension,
    }))
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
    provider: &LazyPlatformSource,
    target: &DescriptorIdentity,
    descriptors: &DescriptorIndex,
) -> Result<Option<ValidationMethodReferenceStatus>, SafeRootError> {
    let Some(reference) = target.method_reference.as_ref() else {
        return Ok(None);
    };
    let (module, procedure) = match reference {
        Ok(reference) => reference,
        Err(()) => return Ok(Some(ValidationMethodReferenceStatus::Invalid)),
    };
    let Some(module_descriptor) = descriptors
        .items
        .iter()
        .find(|item| item.native_type == "CommonModule" && item.name == *module)
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

#[cfg(test)]
mod authorization_order_tests {
    use super::*;
    use crate::safe_root::with_before_artifact_open;
    use std::time::{SystemTime, UNIX_EPOCH};
    use unica_format_core::source::{SourceLocation, SourceContext};

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
        assert!(matches!(result, AuthorabilityResult::Denied(_)));
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
        assert!(matches!(result, AuthorabilityResult::Denied(_)));
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_file(outside).unwrap();
    }
}
