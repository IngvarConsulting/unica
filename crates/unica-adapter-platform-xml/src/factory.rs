use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use unica_format_core::{
    commands::WriterSourceRole,
    navigation::{NavigationSelection, NavigationTarget},
    ports::{
        CapturePort, CaptureResult, CapturedSource, FormatInspectionPort, FormatInspectionRequest,
        FormatInspectionResult, FormatReadRequest, ObjectKindProjection, ObjectKindRegistryPort,
        ObjectKindSelector, OperationalAdapterRegistration, OperationalSourceSession,
        OwnerResolutionMode, OwnerResolutionRequest, OwnerResolutionResult, OwnershipPort,
        ProbePort, ProbeResult, ReadPort, ReservedSourceArtifactKind, SemanticArtifactLease,
        SourceAdapterRegistration, SourceSetMatch, SupportEvidence, SupportInspectionRequest,
        SupportPort,
    },
    semantic_ids::SemanticObjectKind,
    source::{ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext},
};

use crate::versions::v2_20;

#[derive(Debug, Clone, Copy, Default)]
pub struct PlatformXmlAdapterFactory;

#[derive(Debug)]
struct WriterCollectionCapability {
    kind: SemanticObjectKind,
}

impl PlatformXmlAdapterFactory {
    pub const fn new() -> Self {
        Self
    }

    #[cfg(feature = "test-support")]
    pub fn with_publication_lock_pause<T>(
        self,
        acquired: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
        action: impl FnOnce() -> T,
    ) -> T {
        v2_20::writers::single_file_publisher::with_publication_lock_pause(
            acquired, release, action,
        )
    }

    #[cfg(feature = "test-support")]
    pub fn with_publication_lock_contention_signal<T>(
        self,
        sender: std::sync::mpsc::Sender<()>,
        action: impl FnOnce() -> T,
    ) -> T {
        v2_20::writers::single_file_publisher::with_publication_lock_contention_signal(
            sender, action,
        )
    }

    pub fn registration(self) -> SourceAdapterRegistration {
        let adapter = Arc::new(PlatformXmlAdapter);
        SourceAdapterRegistration {
            manifest: v2_20::manifest(),
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
        let validation = Arc::new(crate::validation::PlatformXmlValidation);
        let object_kinds = Arc::new(PlatformXmlObjectKinds);
        let semantic_artifacts = Arc::new(crate::artifact_access::PlatformXmlSemanticArtifacts);
        OperationalAdapterRegistration::new(
            guards.clone(),
            guards.clone(),
            guards,
            object_kinds,
            semantic_artifacts,
            validation.clone(),
            validation,
            Arc::new(crate::publication::PlatformXmlPublication::new()),
            Arc::new(crate::operations::PlatformXmlWriter),
            Arc::new(v2_20::writers::module_locator::PlatformModuleArtifactLocator),
            Arc::new(v2_20::writers::artifact_write::PlatformArtifactWriter),
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
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_unscoped(
                target,
                authorized_root,
                mode,
            ),
        )
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

    pub fn capture_object_source(
        self,
        source_root: &Path,
        selector: &ObjectKindSelector,
        object_name: &str,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(v2_20::operations::PlatformOperationSession::capture_object(
            source_root,
            selector,
            object_name,
            authorized_root,
            mode,
        ))
    }

    pub fn capture_named_object_source(
        self,
        source_root: &Path,
        object_name: &str,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_named_object(
                source_root,
                object_name,
                authorized_root,
                mode,
            ),
        )
    }

    pub fn capture_unscoped_tree_source(
        self,
        target: &Path,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_unscoped_tree(
                target,
                authorized_root,
                mode,
            ),
        )
    }

    pub fn capture_module_artifact_source(
        self,
        source_root: &Path,
        target: &Path,
        authorized_root: &Path,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::writers::module_locator::PlatformModuleLocatorSession::new(
                source_root,
                target,
                authorized_root,
            ),
        )
    }

    pub fn capture_workspace_module_artifact_source(
        self,
        workspace_root: &Path,
        cwd: &Path,
        target: &Path,
        explicit_source: Option<&str>,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::writers::module_locator::PlatformModuleLocatorSession::from_workspace(
                workspace_root,
                cwd,
                target,
                explicit_source,
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

    pub fn classify_reserved_source_artifact(self, bytes: &[u8]) -> ReservedSourceArtifactKind {
        v2_20::source_sets::classify_reserved_source_artifact(bytes)
    }

    #[allow(clippy::type_complexity, clippy::too_many_arguments)]
    pub fn capture_publication_session<R, S>(
        self,
        operation_name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
        workspace_root: &Path,
        cwd: &Path,
        run: R,
        resolve: S,
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
        S: Fn(&Path, &str, bool) -> Result<(PathBuf, Vec<String>), String> + Send + Sync + 'static,
    {
        crate::publication::capture_publication_session(
            operation_name,
            args,
            workspace_root,
            cwd,
            run,
            resolve,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_writer_session<I>(
        self,
        sources: I,
        inline_definition: Option<Vec<u8>>,
        adapter_hint: Option<String>,
        workspace_root: &Path,
        cwd: &Path,
        cache_root: &Path,
        workspace_epoch: u64,
    ) -> Result<OperationalSourceSession, SourceAdapterError>
    where
        I: IntoIterator<Item = (WriterSourceRole, PathBuf)>,
    {
        let session = crate::operations::PlatformWriterSession::new(
            sources,
            inline_definition,
            adapter_hint,
            crate::operations::WorkspaceContext {
                cwd: cwd.to_path_buf(),
                workspace_root: workspace_root.to_path_buf(),
                cache_root: cache_root.to_path_buf(),
                workspace_epoch,
            },
        )
        .map_err(|message| {
            SourceAdapterError::new(SourceAdapterErrorKind::CapabilityBlocked, message)
        })?;
        Ok(OperationalSourceSession::new(session))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_writer_session_with_extension_emitter<I, E>(
        self,
        sources: I,
        inline_definition: Option<Vec<u8>>,
        adapter_hint: Option<String>,
        workspace_root: &Path,
        cwd: &Path,
        cache_root: &Path,
        workspace_epoch: u64,
        emitter: E,
    ) -> Result<OperationalSourceSession, SourceAdapterError>
    where
        I: IntoIterator<Item = (WriterSourceRole, PathBuf)>,
        E: Fn(
                &unica_format_core::commands::ExtensionPatchEmissionPlan,
                Option<&[u8]>,
            ) -> Result<Vec<u8>, String>
            + Send
            + Sync
            + 'static,
    {
        let session = crate::operations::PlatformWriterSession::new(
            sources,
            inline_definition,
            adapter_hint,
            crate::operations::WorkspaceContext {
                cwd: cwd.to_path_buf(),
                workspace_root: workspace_root.to_path_buf(),
                cache_root: cache_root.to_path_buf(),
                workspace_epoch,
            },
        )
        .map_err(|message| {
            SourceAdapterError::new(SourceAdapterErrorKind::CapabilityBlocked, message)
        })?
        .with_extension_emitter(Arc::new(emitter));
        Ok(OperationalSourceSession::new(session))
    }

    #[allow(clippy::type_complexity)]
    pub fn capture_artifact_write_session(
        self,
        replacement: Option<(PathBuf, Vec<u8>, Vec<u8>)>,
        exact_guards: Vec<(PathBuf, Vec<u8>)>,
        absence_guards: Vec<PathBuf>,
        membership_guards: Vec<(PathBuf, u8, Vec<std::ffi::OsString>)>,
    ) -> Result<OperationalSourceSession, SourceAdapterError> {
        use crate::versions::v2_20::writers::artifact_write::{
            ArtifactMembershipSelector, PlatformArtifactWriteSession, StagedArtifactReplacement,
        };
        let replacement =
            replacement.map(
                |(path, expected_preimage, replacement)| StagedArtifactReplacement {
                    path,
                    expected_preimage,
                    replacement,
                },
            );
        let mut session = PlatformArtifactWriteSession {
            replacement,
            exact_guards: exact_guards.into_iter().collect(),
            absence_guards: absence_guards.into_iter().collect(),
            membership_guards: Default::default(),
        };
        for (directory, selector, expected) in membership_guards {
            let selector = match selector {
                0 => ArtifactMembershipSelector::StructuredDescriptors,
                1 => ArtifactMembershipSelector::ConfigurationArtifacts,
                2 => ArtifactMembershipSelector::DirectEntries,
                _ => {
                    return Err(SourceAdapterError::new(
                        SourceAdapterErrorKind::CapabilityBlocked,
                        "unknown semantic directory-membership selector",
                    ))
                }
            };
            session
                .membership_guards
                .insert((directory, selector), expected);
        }
        Ok(OperationalSourceSession::new(session))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_inspection_session(
        self,
        operation: &str,
        tool_name: &str,
        args: &serde_json::Map<String, serde_json::Value>,
        workspace_root: &Path,
        cwd: &Path,
        cache_root: &Path,
        workspace_epoch: u64,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(crate::operations::PlatformInspectionSession::new(
            operation,
            tool_name,
            args.clone(),
            crate::operations::WorkspaceContext {
                cwd: cwd.to_path_buf(),
                workspace_root: workspace_root.to_path_buf(),
                cache_root: cache_root.to_path_buf(),
                workspace_epoch,
            },
        ))
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

    pub fn inspection_port(self) -> Arc<dyn unica_format_core::commands::InspectionPort> {
        Arc::new(crate::operations::PlatformXmlInspector)
    }

    pub fn validation_context_port(
        self,
    ) -> Arc<dyn unica_format_core::ports::ValidationContextPort> {
        Arc::new(crate::validation::PlatformXmlValidation)
    }
}

struct PlatformXmlObjectKinds;

impl ObjectKindRegistryPort for PlatformXmlObjectKinds {
    fn resolve(&self, selector: &ObjectKindSelector) -> Option<SemanticObjectKind> {
        v2_20::semantic_map::writer_object_kind(selector.as_str())
    }

    fn ordered_kinds(&self) -> Vec<SemanticObjectKind> {
        v2_20::semantic_map::writer_object_kinds()
    }

    fn lease(&self, kind: SemanticObjectKind) -> Option<SemanticArtifactLease> {
        v2_20::semantic_map::writer_native_class(kind)?;
        Some(SemanticArtifactLease::new(WriterCollectionCapability {
            kind,
        }))
    }

    fn project(&self, lease: &SemanticArtifactLease) -> Option<&'static ObjectKindProjection> {
        let capability = lease.adapter_state::<WriterCollectionCapability>()?;
        object_kind_projections()
            .iter()
            .find(|projection| projection.kind() == capability.kind)
    }
}

fn object_kind_projections() -> &'static [ObjectKindProjection] {
    static PROJECTIONS: std::sync::OnceLock<Vec<ObjectKindProjection>> = std::sync::OnceLock::new();
    PROJECTIONS
        .get_or_init(|| {
            v2_20::semantic_map::writer_object_kinds()
                .into_iter()
                .map(|kind| {
                    let canonical = v2_20::semantic_map::writer_native_class(kind)
                        .expect("ordered writer kind has a canonical selector");
                    let collection = v2_20::semantic_map::native_descriptor_directory(kind)
                        .expect("ordered writer kind has a collection selector");
                    let display = v2_20::semantic_map::metadata_class_profile(canonical)
                        .and_then(|profile| profile.display_name_ru.as_deref())
                        .expect("ordered writer kind has a display label");
                    ObjectKindProjection::new(
                        kind,
                        ObjectKindSelector::new(canonical)
                            .expect("registry canonical selector is valid"),
                        ObjectKindSelector::new(collection)
                            .expect("registry collection selector is valid"),
                        display,
                    )
                    .expect("private object-kind registry projection is valid")
                })
                .collect()
        })
        .as_slice()
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
        v2_20::inspection::inspect_format(request)
    }
}

impl SupportPort for PlatformXmlAdapter {
    fn inspect(
        &self,
        request: &SupportInspectionRequest,
    ) -> Result<SupportEvidence, SourceAdapterError> {
        v2_20::inspection::inspect_support(request)
    }
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
