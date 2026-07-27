use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use unica_format_core::{
    navigation::{NavigationSelection, NavigationTarget},
    ports::{
        CapturePort, CaptureResult, CapturedSource, FormatInspectionPort, FormatInspectionRequest,
        FormatInspectionResult,
        FormatReadRequest, OperationalAdapterRegistration, OperationalSourceSession,
        ObjectKindProjection, ObjectKindRegistryPort, ObjectKindSelector, OwnerResolutionMode,
        OwnerResolutionRequest, OwnerResolutionResult, OwnershipPort, ProbePort, ProbeResult,
        ReadPort, ReservedSourceArtifactKind, SemanticArtifactLease, SourceAdapterRegistration,
        SourceSetMatch, SupportEvidence, SupportInspectionRequest, SupportPort,
    },
    semantic_ids::SemanticObjectKind,
    source::{
        ConfiguredSourceSetKind, SourceAdapterError, SourceAdapterErrorKind, SourceContext,
    },
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
        let semantic_artifacts =
            Arc::new(crate::artifact_access::PlatformXmlSemanticArtifacts);
        OperationalAdapterRegistration::new(
            guards.clone(),
            guards.clone(),
            guards,
            object_kinds,
            semantic_artifacts,
            validation.clone(),
            validation,
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

    pub fn capture_object_source(
        self,
        source_root: &Path,
        selector: &ObjectKindSelector,
        object_name: &str,
        authorized_root: &Path,
        mode: OwnerResolutionMode,
    ) -> OperationalSourceSession {
        OperationalSourceSession::new(
            v2_20::operations::PlatformOperationSession::capture_object(
                source_root,
                selector,
                object_name,
                authorized_root,
                mode,
            ),
        )
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
    static PROJECTIONS: std::sync::OnceLock<Vec<ObjectKindProjection>> =
        std::sync::OnceLock::new();
    PROJECTIONS
        .get_or_init(|| {
            v2_20::semantic_map::writer_object_kinds()
                .into_iter()
                .map(|kind| {
                    let canonical = v2_20::semantic_map::writer_native_class(kind)
                        .expect("ordered writer kind has a canonical selector");
                    let collection =
                        v2_20::semantic_map::native_descriptor_directory(kind)
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
