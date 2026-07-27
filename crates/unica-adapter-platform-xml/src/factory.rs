use std::{path::PathBuf, sync::Arc};

use unica_format_core::{
    navigation::{NavigationSelection, NavigationTarget},
    ports::{
        AdapterFormatProfile, CapturePort, CaptureResult, CapturedSource, EffectiveSupportRule,
        FormatCompatibility, FormatInspectionMode, FormatInspectionPort, FormatInspectionRequest,
        FormatInspectionResult, FormatReadRequest, OwnerResolutionRequest, OwnerResolutionResult,
        OwnershipPort, ProbePort, ProbeResult, ReadPort, SourceAdapterRegistration,
        SupportEvidence, SupportInspectionRequest, SupportPort, SupportSourceState,
        SupportVendorEvidence,
    },
    source::{SourceAdapterError, SourceAdapterErrorKind, SourceContext, SourceFamily},
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
            profile: Self::profile(),
            capture: adapter.clone(),
            probe: adapter.clone(),
            read: adapter.clone(),
            ownership: adapter.clone(),
            format_inspection: adapter.clone(),
            support: adapter,
        }
    }

    pub fn profile() -> AdapterFormatProfile {
        AdapterFormatProfile {
            platform_line: v2_20::PLATFORM_LINE,
            export_format: v2_20::EXPORT_FORMAT,
            legacy_metadata_classes: v2_20::metadata_classes(),
        }
    }
}

struct PlatformXmlAdapter;

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

fn authorized_target(source: &SourceContext) -> Result<PathBuf, SourceAdapterError> {
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
