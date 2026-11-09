//! Сопоставление диагностик с узлами и модулями.

use super::*;

impl<'a> LogicalViewReadAuthority<'a> {
    pub fn diagnostic_mapping(
        &self,
        at: &QualifiedAddress,
        workspace: &unica_source_contract::workspace::WorkspaceContext,
    ) -> Result<crate::infrastructure::diagnostics::CanonicalDiagnosticMapping, ViewError> {
        use unica_address::source_target::TargetKind;
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let capability = self
            .profile
            .module_prefix_capability(&canonical)
            .ok_or_else(|| {
                ViewError::new(
                    RefusalCode::BadValue,
                    "BSL diagnostics requires a module or its body",
                )
            })?;
        let (module_at, _) = shared_module_prefix(&canonical, self.profile, capability)
            .map_err(|error| ViewError::new(RefusalCode::BadValue, error))?;
        // Registration, owner identity, format and module containment are proved
        // by the same reader as view, using this invocation's retained root.
        self.read_exact(&module_at, &admitted)?;
        let target = module_source_address(&module_at, capability)?;
        let relative = self.read.module_export_path(&target)?;
        let context = self.diagnostic_context(workspace, Some(target), TargetKind::Module);
        crate::infrastructure::diagnostics::CanonicalDiagnosticMapping::from_proven_module(
            context,
            relative.into(),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    /// Mapping of a metadata object or the configuration root, proved by the
    /// same typed read as `view`. Findings on its own elements keep their
    /// metadata focus; separately addressable children stay their own targets.
    pub fn diagnostic_node_mapping(
        &self,
        at: &QualifiedAddress,
        workspace: &unica_source_contract::workspace::WorkspaceContext,
    ) -> Result<crate::infrastructure::diagnostics::CanonicalDiagnosticMapping, ViewError> {
        use unica_address::source_target::TargetKind;
        let admitted = self.snapshot(at)?;
        let canonical = self.canonical_address(at, &admitted)?;
        let route = route_logical_address(&canonical, self.profile)
            .map_err(|error| ViewError::new(RefusalCode::NotFound, error.to_string()))?;
        let (target, kind) = match (route.reader(), canonical.segments()) {
            (LogicalReader::Configuration, [_]) => (None, TargetKind::SourceRoot),
            // Объект любого вида, включая общий модуль, чей маршрут ведёт в
            // читатель модулей: целью остаётся сам объект.
            (_, [owner]) if owner.name().is_some() => (
                Some(
                    MetadataAddress::parse_logical(&format!(
                            "{}.{}",
                            owner.kind().as_str(),
                            owner.name().unwrap_or_default()
                        ),
                    )
                    .map_err(|error| ViewError::new(RefusalCode::BadValue, error.to_string()))?,
                ),
                TargetKind::MetadataObject,
            ),
            _ => {
                return Err(ViewError::new(
                    RefusalCode::BadValue,
                    "provider diagnostics requires a module, a metadata object or the configuration root",
                ))
            }
        };
        self.read_exact(&canonical, &admitted)?;
        crate::infrastructure::diagnostics::CanonicalDiagnosticMapping::from_proven_node(
            self.diagnostic_context(workspace, target, kind),
        )
        .map_err(|error| ViewError::detailed(RefusalDetail::SourceUnreadable, error))
    }

    pub(super) fn diagnostic_context(
        &self,
        workspace: &unica_source_contract::workspace::WorkspaceContext,
        metadata_path: Option<MetadataAddress>,
        target_kind: unica_address::source_target::TargetKind,
    ) -> unica_diagnostics::model::DiagnosticContext {
        use unica_address::source_target::ResolvedTarget;
        use unica_diagnostics::model::DiagnosticContext;
        use unica_source_contract::workspace::ResolvedSourceRoot;
        use unica_source_contract::{ProjectSourceSet, SourceFormat, SourceSetState};
        let source_set = self.read.source_set().to_owned();
        DiagnosticContext::new(
            workspace.clone(),
            ProjectSourceSet {
                name: source_set.clone(),
                kind: self.read.source_set_kind(),
                path: self.read.root_path().to_string_lossy().into_owned(),
                source_format: SourceFormat::PlatformXml,
                source_state: SourceSetState::Supported,
                format_evidence: Vec::new(),
                format_probe_error: None,
            },
            ResolvedSourceRoot {
                source_set: Some(source_set.clone()),
                path: self.read.root_path().to_path_buf(),
            },
            ResolvedTarget {
                source_set,
                metadata_path,
                target_kind,
            },
        )
    }
}
