use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use unica_format_core::{
    navigation::{NavigationSelection, NavigationTarget},
    ports::{
        AdapterFormatProfile, CapturePort, CaptureResult, CapturedSource, EffectiveSupportRule,
        FormatCompatibility, FormatInspectionMode, FormatInspectionPort, FormatInspectionRequest,
        FormatInspectionResult, FormatReadRequest, OperationalAdapterRegistration,
        OperationalSourceSession, OwnerResolutionMode, OwnerResolutionRequest,
        OwnerResolutionResult, OwnershipPort, ProbePort, ProbeResult, ReadPort,
        ReservedSourceArtifactKind, SourceAdapterRegistration, SourceSetMatch, SupportEvidence,
        SupportInspectionRequest, SupportPort, SupportSourceState, SupportVendorEvidence,
    },
    source::{
        ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext,
        SourceFamily,
    },
};

use crate::versions::v2_20;

#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformXmlAdapterFactory;

impl PlatformXmlAdapterFactory {
    pub const fn new() -> Self {
        Self
    }

    pub fn registration(self) -> SourceAdapterRegistration {
        let adapter = Arc::new(PlatformXmlAdapter);
        SourceAdapterRegistration {
            manifest: v2_20::manifest(),
            profile: adapter_profile(),
            capture: adapter.clone(),
            probe: adapter.clone(),
            read: adapter.clone(),
            ownership: adapter.clone(),
            format_inspection: adapter.clone(),
            support: adapter,
        }
    }

    pub fn operational_registration(self) -> OperationalAdapterRegistration {
        let guards = Arc::new(crate::guards::PlatformXmlGuards);
        OperationalAdapterRegistration::new(
            guards.clone(),
            guards.clone(),
            guards,
            Arc::new(crate::validation::PlatformXmlValidation),
            Arc::new(crate::publication::PlatformXmlPublication::new()),
        )
    }

    pub fn capture_operational_source(
        self,
        source: &SourceContext,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(v2_20::operations::PlatformOperationSession::capture(
            source, mode,
        ))
    }

    pub fn capture_validation_source(
        self,
        source: &SourceContext,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_validation(source, mode),
        )
    }

    pub fn capture_unscoped_source(
        self,
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(v2_20::operations::PlatformOperationSession::capture_unscoped(
            target,
            authorized_root,
            mode,
        ))
    }

    pub fn capture_unscoped_validation_source(
        self,
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_unscoped_validation(
                target,
                authorized_root,
                mode,
            ),
        )
    }

    pub fn inspect_source_set(
        self,
        source_root: &Path,
        authorized_root: &Path,
        kind: ConfiguredSourceSetKind,
    ) -> Result<SourceSetMatch, SourceAdapterError> {
        v2_20::source_sets::inspect(source_root, authorized_root, kind)
    }

    pub fn classify_reserved_source_artifact(
        self,
        bytes: &[u8],
    ) -> ReservedSourceArtifactKind {
        v2_20::source_sets::classify_reserved_source_artifact(bytes)
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn capture_publication_session<R, S, L>(
        self,
        operation_name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
        workspace_root: &Path,
        cwd: &Path,
        run: R,
        resolve: S,
        lock: L,
    ) -> OperationalSourceSession
    where
        R: Fn(
                &Path,
                &[String],
                &Path,
                Option<Duration>,
                &unica_format_core::ports::OperationCancellation,
            ) -> Result<(bool, String, String, String, bool, bool, bool), String>
            + Send
            + Sync
            + 'static,
        S: Fn(&Path, &str, bool) -> Result<(PathBuf, Vec<String>), String>
            + Send
            + Sync
            + 'static,
        L: Fn(
                &[PathBuf],
                &mut dyn FnMut() -> Result<Vec<String>, String>,
            ) -> Result<Result<Vec<String>, String>, String>
            + Send
            + Sync
            + 'static,
    {
        crate::publication::capture_publication_session(
            operation_name,
            args,
            workspace_root,
            cwd,
            run,
            resolve,
            lock,
        )
    }

    pub fn compatibility_port(self) -> Arc<dyn unica_format_core::ports::CompatibilityPort> {
        Arc::new(crate::guards::PlatformXmlGuards)
    }

    pub fn source_compatibility_port(
        self,
    ) -> Arc<dyn unica_format_core::ports::SourceCompatibilityPort> {
        Arc::new(crate::guards::PlatformXmlGuards)
    }

    pub fn authorability_port(self) -> Arc<dyn unica_format_core::ports::AuthorabilityPort> {
        Arc::new(crate::guards::PlatformXmlGuards)
    }

    pub fn validation_context_port(
        self,
    ) -> Arc<dyn unica_format_core::ports::ValidationContextPort> {
        Arc::new(crate::validation::PlatformXmlValidation)
    }

    pub fn validate_coverage_manifest(raw: &str) -> Result<(), SourceAdapterError> {
        v2_20::validate_coverage_manifest(raw)
    }
}

struct PlatformXmlAdapter;

fn adapter_profile() -> AdapterFormatProfile {
    AdapterFormatProfile {
        platform_line: v2_20::PLATFORM_LINE,
        export_format: v2_20::EXPORT_FORMAT,
        legacy_metadata_classes: v2_20::metadata_classes(),
    }
}

impl CapturePort for PlatformXmlAdapter {
    fn capture(&self, source: &SourceContext) -> Result<CaptureResult, SourceAdapterError> {
        v2_20::capture(source)
    }
}

impl ProbePort for PlatformXmlAdapter {
    fn probe(&self, captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
        v2_20::probe(captured)
    }
}

impl ReadPort for PlatformXmlAdapter {
    fn read(
        &self,
        request: &FormatReadRequest,
    ) -> Result<unica_format_core::navigation::NavigationEnvelope, SourceAdapterError> {
        validate_query(request)?;
        v2_20::read(&request.captured)
    }
}

impl OwnershipPort for PlatformXmlAdapter {
    fn resolve(
        &self,
        request: &OwnerResolutionRequest,
    ) -> Result<OwnerResolutionResult, SourceAdapterError> {
        crate::owner::resolve(request)
    }
}

impl FormatInspectionPort for PlatformXmlAdapter {
    fn inspect(
        &self,
        request: &FormatInspectionRequest,
    ) -> Result<FormatInspectionResult, SourceAdapterError> {
        let path = authorized_target(&request.source)?;
        let raw = std::fs::read(&path).map_err(|read_error| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                format!("failed to read {}: {read_error}", path.display()),
            )
        })?;
        let text = std::str::from_utf8(&raw).map_err(|utf8_error| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                format!("failed to read {} as UTF-8: {utf8_error}", path.display()),
            )
        })?;
        let source = text.trim_start_matches('\u{feff}');
        let document = roxmltree::Document::parse(source).map_err(|parse_error| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                format!("failed to parse {}: {parse_error}", path.display()),
            )
        })?;
        let root = document.root_element();
        let version = root
            .attributes()
            .find(|attribute| attribute.namespace().is_none() && attribute.name() == "version")
            .and_then(|attribute| source.get(attribute.range_value()));
        match request.mode {
            FormatInspectionMode::Versionless if version.is_some() => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                format!(
                    "versionless platform XML root must not declare a version in {}",
                    path.display()
                ),
            )),
            FormatInspectionMode::Versionless => Ok(FormatInspectionResult {
                compatibility: None,
            }),
            FormatInspectionMode::Versioned => {
                let compatibility =
                    v2_20::profile::classify_root_version(version).map_err(|format_error| {
                        SourceAdapterError::new(
                            SourceAdapterErrorKind::DecodeCorrupted,
                            format!("{} in {}", format_error, path.display()),
                        )
                    })?;
                let actual = unica_format_core::source::FormatVersion::parse(
                    &compatibility.actual().to_string(),
                )?;
                let target = unica_format_core::source::FormatVersion::parse(v2_20::EXPORT_FORMAT)?;
                let compatibility = match compatibility {
                    v2_20::profile::FormatCompatibility::Older { .. } => {
                        FormatCompatibility::Older { actual, target }
                    }
                    v2_20::profile::FormatCompatibility::Supported { .. } => {
                        FormatCompatibility::Supported { actual, target }
                    }
                    v2_20::profile::FormatCompatibility::Newer { .. } => {
                        FormatCompatibility::Newer { actual, target }
                    }
                };
                Ok(FormatInspectionResult {
                    compatibility: Some(compatibility),
                })
            }
        }
    }
}

impl SupportPort for PlatformXmlAdapter {
    fn inspect(
        &self,
        request: &SupportInspectionRequest,
    ) -> Result<SupportEvidence, SourceAdapterError> {
        let path = authorized_target(&request.source)?;
        let object = request
            .object
            .as_ref()
            .map(|object| object.as_str())
            .unwrap_or("");
        Ok(support_evidence(
            v2_20::support::read_support_facts(&path),
            object,
        ))
    }
}

pub(crate) fn authorized_target(source: &SourceContext) -> Result<PathBuf, SourceAdapterError> {
    if source.declared_family() != &SourceFamily::PlatformXml {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "Platform XML adapter received a source from another family",
        ));
    }
    let location = source.location();
    let workspace = canonical_directory(location.workspace_root())?;
    let source_root = canonical_directory(location.source_root())?;
    if !source_root.starts_with(&workspace) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "authorized source root is outside the workspace root",
        ));
    }
    let target = canonical_target(location.target())?;
    if !target.starts_with(&source_root) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            "authorized target is outside the source root",
        ));
    }
    Ok(target)
}

fn canonical_target(path: &std::path::Path) -> Result<PathBuf, SourceAdapterError> {
    let mut existing = path;
    let mut suffix = Vec::new();
    while !existing.exists() {
        suffix.push(
            existing
                .file_name()
                .ok_or_else(|| {
                    SourceAdapterError::new(
                        SourceAdapterErrorKind::SourceUnavailable,
                        "authorized target has no file name",
                    )
                })?
                .to_os_string(),
        );
        existing = existing.parent().ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "authorized target has no existing ancestor",
            )
        })?;
    }
    let mut canonical = std::fs::canonicalize(existing).map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            format!(
                "failed to resolve authorized target ancestor {}: {error}",
                existing.display()
            ),
        )
    })?;
    for component in suffix.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

fn canonical_directory(path: &std::path::Path) -> Result<PathBuf, SourceAdapterError> {
    std::fs::canonicalize(path).map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SourceUnavailable,
            format!(
                "failed to resolve authorized directory {}: {error}",
                path.display()
            ),
        )
    })
}

fn validate_query(request: &FormatReadRequest) -> Result<(), SourceAdapterError> {
    match &request.query.target {
        NavigationTarget::CapturedTarget(identity)
            if identity == &request.captured.binding().target_identity => {}
        NavigationTarget::CapturedTarget(_) => {
            return Err(capability_error(
                "Platform XML reads only the captured target identity",
            ))
        }
        NavigationTarget::ObjectPath(_) => {
            return Err(capability_error(
                "Platform XML reads require a captured target identity",
            ))
        }
        NavigationTarget::ObjectRef { .. } => {
            return Err(capability_error(
                "Platform XML object-reference navigation is not implemented",
            ))
        }
        NavigationTarget::Cursor(_) => {
            return Err(capability_error(
                "Platform XML continuation navigation is not implemented",
            ))
        }
    }
    if request.query.select != full_selection() {
        return Err(capability_error(
            "Platform XML projected selections are not implemented",
        ));
    }
    Ok(())
}

fn full_selection() -> NavigationSelection {
    use unica_format_core::navigation::{FacetSelection, PropertySelection};
    NavigationSelection {
        properties: PropertySelection::All,
        facets: FacetSelection::Full,
        relations: Vec::new(),
    }
}

fn capability_error(message: &'static str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::CapabilityBlocked, message)
}

fn support_evidence(facts: v2_20::support::SupportFacts, object_uuid: &str) -> SupportEvidence {
    use v2_20::support::{EffectiveSupportRule as NativeRule, SupportSourceState as NativeState};
    let effective = facts.effective_rule_for(object_uuid);
    let source = match &facts.source {
        NativeState::Absent => SupportSourceState::Absent,
        NativeState::Removed => SupportSourceState::Removed,
        NativeState::Parsed => SupportSourceState::Parsed,
        NativeState::Unreadable { error } => SupportSourceState::Unreadable {
            context: error.context.to_string(),
            offset: error.offset,
        },
    };
    let effective_rule = match effective {
        NativeRule::Absent => EffectiveSupportRule::Absent,
        NativeRule::Removed => EffectiveSupportRule::Removed,
        NativeRule::Editable => EffectiveSupportRule::Editable,
        NativeRule::Locked => EffectiveSupportRule::Locked,
        NativeRule::ConfigurationReadOnly => EffectiveSupportRule::ConfigurationReadOnly,
        NativeRule::UnknownReadOnly => EffectiveSupportRule::UnknownReadOnly,
        NativeRule::Unreadable => EffectiveSupportRule::Unreadable,
    };
    SupportEvidence {
        source,
        effective_rule,
        authorability: facts.authorability_for(object_uuid),
        global_editing_enabled: facts.global_editing_enabled(),
        rule_counts: facts.rule_counts(),
        vendors: facts
            .vendors()
            .iter()
            .map(|vendor| SupportVendorEvidence {
                version: vendor.version.clone(),
                vendor: vendor.vendor.clone(),
                name: vendor.name.clone(),
            })
            .collect(),
    }
}
