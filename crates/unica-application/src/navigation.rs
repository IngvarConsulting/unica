use std::sync::{Arc, Mutex};

use unica_format_core::{
    navigation::{
        normalize_navigation_selection, FacetSelection, NavigationCursor, NavigationEnvelope,
        NavigationFacetVisibility, NavigationNode, NavigationQuery, NavigationRelationPage,
        NavigationSelection, NavigationStatus, NavigationTarget, ObjectKey, ObjectRef,
        PropertySelection, SemanticRelation, SemanticRelationId,
    },
    ports::{CaptureResult, FormatReadRequest, ProbeResult, SourceAdapterRegistration},
    source::{
        SourceAdapterError, SourceAdapterErrorKind, SourceBinding, SourceContext, SourceDescriptor,
        SourceId, SourceRevision, TargetIdentity,
    },
};

use crate::{
    commands::{MetadataNavigationCommand, MetadataNavigationTarget},
    snapshot_cache::{CachedNavigation, SnapshotCache, SnapshotCacheAdmission},
};

pub trait SourceRegistrationResolver: Send + Sync {
    fn locate(&self) -> Result<LocatedSource, SourceAdapterError>;

    fn authorize_continuation(
        &self,
        source_id: &SourceId,
    ) -> Result<CurrentSourceAuthorization, SourceAdapterError>;
}

#[derive(Clone)]
pub struct LocatedSource {
    pub source: SourceContext,
    pub expected_source_id: SourceId,
    pub target_identity: TargetIdentity,
    pub authorization_scope: String,
    pub registration: SourceAdapterRegistration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentSourceAuthorization {
    pub source_id: SourceId,
    pub authorization_scope: String,
}

pub struct MetadataNavigationService {
    cache: Mutex<SnapshotCache>,
    cursor_secret: Vec<u8>,
}

impl MetadataNavigationService {
    pub fn new(cursor_secret: impl Into<Vec<u8>>) -> Self {
        Self {
            cache: Mutex::new(SnapshotCache::default()),
            cursor_secret: cursor_secret.into(),
        }
    }

    pub fn inspect(
        &self,
        command: MetadataNavigationCommand,
        resolver: &dyn SourceRegistrationResolver,
    ) -> NavigationEnvelope {
        match self.inspect_inner(command, resolver) {
            Ok(navigation) => navigation,
            Err(error) => NavigationEnvelope::unavailable(error),
        }
    }

    fn inspect_inner(
        &self,
        command: MetadataNavigationCommand,
        resolver: &dyn SourceRegistrationResolver,
    ) -> Result<NavigationEnvelope, SourceAdapterError> {
        let (
            navigation,
            target_ref,
            target_identity,
            selection,
            cursor,
            include_cached_diagnostics,
        ) = match command.target {
            MetadataNavigationTarget::Source => {
                let located = resolver.locate()?;
                let (navigation, binding) = inspect_located_source(&located)?;
                if navigation.status == NavigationStatus::Unavailable {
                    return Ok(navigation);
                }
                let navigation = match self.cache_ready_navigation(
                    navigation,
                    &binding,
                    &located.authorization_scope,
                )? {
                    SnapshotCacheAdmission::Admitted(navigation) => navigation,
                    SnapshotCacheAdmission::ResourceLimit => {
                        return Err(resource_limit(
                            "navigation snapshot exceeds continuation cache limits",
                        ));
                    }
                };
                let target_ref = object_path_target(navigation.as_ref())?;
                (
                    navigation,
                    target_ref,
                    binding.target_identity,
                    normalize_navigation_selection(
                        command
                            .selection
                            .unwrap_or_else(default_navigation_selection),
                    )?,
                    None,
                    true,
                )
            }
            MetadataNavigationTarget::ObjectRef {
                source_id,
                object_key,
                snapshot_revision,
            } => {
                let (navigation, target_ref, target_identity) = self.cached_navigation_target(
                    &source_id,
                    &object_key,
                    &snapshot_revision,
                    None,
                    resolver,
                )?;
                (
                    navigation,
                    target_ref,
                    target_identity,
                    normalize_navigation_selection(
                        command
                            .selection
                            .unwrap_or_else(default_navigation_selection),
                    )?,
                    None,
                    false,
                )
            }
            MetadataNavigationTarget::Cursor(value) => {
                if command.selection.is_some() {
                    return Err(SourceAdapterError::new(
                        SourceAdapterErrorKind::ProjectionAmbiguous,
                        "cursor mode does not accept select",
                    ));
                }
                let cursor = value.authenticate(&self.cursor_secret)?;
                let (navigation, target_ref, target_identity) = self.cached_navigation_target(
                    &cursor.source_id,
                    &cursor.target,
                    &cursor.snapshot_revision,
                    Some(&cursor.target_identity),
                    resolver,
                )?;
                let snapshot = navigation.snapshot.as_ref().ok_or_else(|| {
                    source_unavailable("navigation cursor source has no truthful snapshot")
                })?;
                let selection = cursor.selection.clone();
                let cursor = cursor.validate_resume(
                    &snapshot.revision,
                    |source_id, target, relation, relation_role, relation_kind| {
                        source_id == &target_ref.source_id
                            && target == &target_ref.object_key
                            && navigation.relation_index.iter().any(|candidate| {
                                candidate.source == target_ref
                                    && &candidate.group_ref.group_key == relation
                                    && &candidate.role == relation_role
                                    && &candidate.kind == relation_kind
                            })
                    },
                )?;
                (
                    navigation,
                    target_ref,
                    target_identity,
                    selection,
                    Some(cursor),
                    false,
                )
            }
        };
        materialize_navigation_pages_with_secret(
            navigation.as_ref(),
            target_ref,
            target_identity,
            selection,
            cursor,
            &self.cursor_secret,
            include_cached_diagnostics,
        )
    }

    fn cache_ready_navigation(
        &self,
        navigation: NavigationEnvelope,
        binding: &SourceBinding,
        scope: &str,
    ) -> Result<SnapshotCacheAdmission, SourceAdapterError> {
        let snapshot = navigation
            .snapshot
            .as_ref()
            .ok_or_else(|| source_unavailable("ready navigation has no snapshot"))?;
        if snapshot.revision != binding.revision || snapshot.source_id != binding.source_id {
            return Err(source_unavailable(
                "navigation snapshot does not match the captured authorization binding",
            ));
        }
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| source_unavailable("navigation snapshot cache is unavailable"))?;
        let entry = match CachedNavigation::new(
            scope.to_string(),
            binding.clone(),
            navigation,
            cache.limits,
        ) {
            Ok(entry) => entry,
            Err(error) if error.kind == SourceAdapterErrorKind::ResourceLimit => {
                return Ok(SnapshotCacheAdmission::ResourceLimit);
            }
            Err(error) => return Err(error),
        };
        cache.admit(entry)
    }

    fn cached_navigation_target(
        &self,
        source_id: &SourceId,
        object_key: &ObjectKey,
        revision: &SourceRevision,
        requested_target_identity: Option<&TargetIdentity>,
        resolver: &dyn SourceRegistrationResolver,
    ) -> Result<(Arc<NavigationEnvelope>, ObjectRef, TargetIdentity), SourceAdapterError> {
        let authorization = resolver.authorize_continuation(source_id)?;
        if authorization.source_id != *source_id {
            return Err(source_unavailable(
                "continuation authorization returned another source identity",
            ));
        }
        let cache = self
            .cache
            .lock()
            .map_err(|_| source_unavailable("navigation snapshot cache is unavailable"))?;
        let target_identity = match requested_target_identity {
            Some(target_identity) => target_identity.clone(),
            None => cache.resolve_target_identity(
                &authorization.authorization_scope,
                source_id,
                revision,
                object_key,
            )?,
        };
        let navigation = cache
            .navigation(
                &authorization.authorization_scope,
                source_id,
                &target_identity,
                revision,
            )
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::SnapshotStale,
                    "requested navigation snapshot is not retained for the current authorization scope",
                )
            })?;
        let matches = navigation
            .nodes
            .iter()
            .filter(|node| {
                node.object_ref.source_id == *source_id && node.object_ref.object_key == *object_key
            })
            .map(|node| node.object_ref.clone())
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [target] => Ok((navigation, target.clone(), target_identity)),
            [] => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "navigation target is absent from the retained snapshot",
            )),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::IdentityCollision,
                "cached navigation has duplicate object keys",
            )),
        }
    }
}

fn inspect_located_source(
    located: &LocatedSource,
) -> Result<(NavigationEnvelope, SourceBinding), SourceAdapterError> {
    let captured = match located.registration.capture.capture(&located.source)? {
        CaptureResult::NoMatch => {
            return Err(source_unavailable(
                "no source capture adapter recognized the target",
            ))
        }
        CaptureResult::Captured(captured) => captured,
    };
    let binding = captured.binding().clone();
    if binding.source_id != located.expected_source_id
        || binding.family != located.registration.manifest.source_family
        || binding.target_identity != located.target_identity
    {
        return Err(source_unavailable(
            "captured source binding does not match the authorized source set",
        ));
    }
    let descriptor = match located.registration.probe.probe(&captured)? {
        ProbeResult::NoMatch => {
            return Err(source_unavailable("no source probe recognized the target"))
        }
        ProbeResult::Match(descriptor) => descriptor,
    };
    validate_probe_descriptor(&binding, &descriptor)?;
    if !registration_supports(&located.registration, &descriptor) {
        return Ok((
            NavigationEnvelope::unavailable(SourceAdapterError::new(
                SourceAdapterErrorKind::FormatUnsupported,
                format!(
                    "no reader supports {:?} format {}",
                    descriptor.family, descriptor.format_version
                ),
            )),
            binding,
        ));
    }
    let mut envelope = located.registration.read.read(&FormatReadRequest {
        captured,
        query: NavigationQuery {
            target: NavigationTarget::CapturedTarget(binding.target_identity.clone()),
            select: NavigationSelection {
                properties: PropertySelection::All,
                facets: FacetSelection::Full,
                relations: Vec::new(),
            },
        },
    })?;
    envelope.reconcile_partial_coverage();
    validate_ready_envelope(&envelope, &binding, &located.registration)?;
    Ok((envelope, binding))
}

fn registration_supports(
    registration: &SourceAdapterRegistration,
    descriptor: &SourceDescriptor,
) -> bool {
    registration.manifest.source_family == descriptor.family
        && registration
            .manifest
            .required_features
            .is_subset(&descriptor.detected_features)
        && registration
            .manifest
            .excluded_features
            .is_disjoint(&descriptor.detected_features)
        && registration
            .manifest
            .supported_formats
            .iter()
            .any(|range| range.contains(&descriptor.format_version))
}

fn validate_probe_descriptor(
    binding: &SourceBinding,
    descriptor: &SourceDescriptor,
) -> Result<(), SourceAdapterError> {
    if descriptor.family != binding.family || descriptor.source_id != binding.source_id {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor does not match the captured source",
        ));
    }
    if let Some(format) = binding.format.as_ref() {
        if descriptor.format_version != *format {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "source probe descriptor format does not match the declared source",
            ));
        }
    }
    let evidence = descriptor.snapshot_evidence.as_ref().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "source probe descriptor has no immutable snapshot evidence",
        )
    })?;
    if evidence.revision != binding.revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "source probe descriptor revision differs from the captured source",
        ));
    }
    Ok(())
}

fn validate_ready_envelope(
    envelope: &NavigationEnvelope,
    binding: &SourceBinding,
    registration: &SourceAdapterRegistration,
) -> Result<(), SourceAdapterError> {
    if envelope.status == NavigationStatus::Unavailable {
        return Ok(());
    }
    let snapshot = envelope.snapshot.as_ref().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "ready navigation envelope has no snapshot",
        )
    })?;
    if snapshot.source_id != binding.source_id
        || snapshot.adapter_id != registration.manifest.adapter_id
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotInconsistent,
            "ready navigation snapshot identity does not match the selected reader",
        ));
    }
    if snapshot.revision != binding.revision {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::SnapshotStale,
            "ready navigation snapshot differs from the captured source",
        ));
    }
    crate::snapshot_cache::validate_identity_bearing_navigation(binding, envelope)
}

fn default_navigation_selection() -> NavigationSelection {
    NavigationSelection {
        properties: PropertySelection::All,
        facets: FacetSelection::Summary,
        relations: vec![unica_format_core::navigation::RelationSelection::new(
            SemanticRelationId::CHILDREN,
            None,
        )
        .expect("default relation selection")],
    }
}

pub(crate) fn object_path_target(
    navigation: &NavigationEnvelope,
) -> Result<ObjectRef, SourceAdapterError> {
    let root = navigation
        .root
        .as_ref()
        .ok_or_else(|| source_unavailable("navigation has no root"))?;
    navigation
        .relation_index
        .iter()
        .find(|relation| relation.source == *root)
        .map(|relation| relation.target.clone())
        .or_else(|| navigation.nodes.first().map(|node| node.object_ref.clone()))
        .ok_or_else(|| source_unavailable("navigation has no metadata object"))
}

pub(crate) fn materialize_navigation_pages_with_secret(
    navigation: &NavigationEnvelope,
    target: ObjectRef,
    target_identity: TargetIdentity,
    selection: NavigationSelection,
    cursor: Option<NavigationCursor>,
    cursor_secret: &[u8],
    include_cached_diagnostics: bool,
) -> Result<NavigationEnvelope, SourceAdapterError> {
    let snapshot = navigation
        .snapshot
        .clone()
        .ok_or_else(|| source_unavailable("ready navigation has no snapshot"))?;
    let original_relations = &navigation.relation_index;
    let target_node = navigation
        .nodes
        .iter()
        .find(|node| node.object_ref == target)
        .cloned()
        .ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "navigation target cannot be re-resolved from the retained snapshot",
            )
        })?;
    let mut target_node = project_selected_node(target_node, &selection);
    let bound_group = cursor
        .as_ref()
        .map(|cursor| {
            if cursor.source_id != target.source_id || cursor.target != target.object_key {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::SnapshotStale,
                    "navigation cursor target no longer matches the retained snapshot",
                ));
            }
            original_relations
                .iter()
                .find(|relation| {
                    relation.source == target
                        && relation.group_ref.owner == target
                        && relation.group_ref.source_id == target.source_id
                        && relation.group_ref.group_key == cursor.relation
                        && relation.group_ref.role == cursor.relation_role
                        && relation.group_ref.kind == cursor.relation_kind
                })
                .map(|relation| relation.group_ref.clone())
                .ok_or_else(|| {
                    SourceAdapterError::new(
                        SourceAdapterErrorKind::SnapshotStale,
                        "navigation cursor relation is absent from the retained snapshot",
                    )
                })
        })
        .transpose()?;
    let mut pages = Vec::new();
    for relation_selection in &selection.relations {
        if bound_group.as_ref().is_some_and(|group| {
            group.role != relation_selection.role || group.kind != relation_selection.kind
        }) {
            continue;
        }
        let matches_selection = |relation: &SemanticRelation| {
            relation.source == target
                && relation.role == relation_selection.role
                && relation.kind == relation_selection.kind
                && bound_group
                    .as_ref()
                    .is_none_or(|group| relation.group_ref == *group)
        };
        let Some(first) = original_relations
            .iter()
            .find(|relation| matches_selection(relation))
        else {
            continue;
        };
        let matching_len = original_relations
            .iter()
            .filter(|relation| matches_selection(relation))
            .count();
        let start = match (&cursor, &bound_group) {
            (Some(cursor), Some(group)) if first.group_ref == *group => {
                usize::try_from(cursor.next_position).map_err(|_| {
                    decode_error("navigation cursor position overflows this process")
                })?
            }
            (None, None) => 0,
            _ => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::SnapshotStale,
                    "navigation cursor relation does not match the retained page group",
                ))
            }
        };
        if start > matching_len {
            return Err(decode_error(
                "navigation cursor position is outside the relation page",
            ));
        }
        let end = start
            .checked_add(usize::from(relation_selection.page_size))
            .ok_or_else(|| decode_error("navigation cursor position overflow"))?
            .min(matching_len);
        let items = original_relations
            .iter()
            .filter(|relation| matches_selection(relation))
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|relation| {
                navigation
                    .nodes
                    .iter()
                    .find(|node| node.object_ref == relation.target)
                    .cloned()
                    .map(|node| project_selected_node(node, &selection))
                    .ok_or_else(|| {
                        SourceAdapterError::new(
                            SourceAdapterErrorKind::IdentityCollision,
                            "relation target has no navigation node",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let next_position = u64::try_from(end)
            .map_err(|_| decode_error("navigation cursor position cannot be represented"))?;
        let next_cursor = (end < matching_len)
            .then(|| {
                NavigationCursor::issue_bound(
                    cursor_secret,
                    snapshot.source_id.clone(),
                    snapshot.revision.clone(),
                    target_identity.clone(),
                    target.object_key.clone(),
                    first.group_ref.clone(),
                    selection.clone(),
                    next_position,
                )
            })
            .transpose()?;
        pages.push(NavigationRelationPage {
            relation: first.group_ref.clone(),
            items,
            next_cursor,
        });
    }
    let requested_property_missing = match &selection.properties {
        PropertySelection::All => false,
        PropertySelection::Named(names) => std::iter::once(&target_node)
            .chain(pages.iter().flat_map(|page| page.items.iter()))
            .any(|node| names.iter().any(|name| !node.properties.contains_key(name))),
    };
    target_node.facets = unica_format_core::facets::SemanticFacets::for_available(
        target_node.properties.keys().copied(),
        pages.iter().map(|page| page.relation.role),
    );
    let diagnostics =
        if include_cached_diagnostics || navigation.status == NavigationStatus::Partial {
            navigation.diagnostics.clone()
        } else {
            Vec::new()
        };
    let mut projected = NavigationEnvelope {
        schema_version: navigation.schema_version.clone(),
        status: navigation.status,
        snapshot: Some(snapshot),
        root: Some(target),
        nodes: vec![target_node],
        relations: pages,
        diagnostics,
        relation_index: Arc::clone(&navigation.relation_index),
    };
    if requested_property_missing {
        projected.mark_partial_coverage();
    }
    projected.reconcile_partial_coverage();
    Ok(projected)
}

pub(crate) fn project_selected_node(
    mut node: NavigationNode,
    selection: &NavigationSelection,
) -> NavigationNode {
    if let PropertySelection::Named(names) = &selection.properties {
        node.properties.retain(|name, _| names.contains(name));
    }
    node.facets = unica_format_core::facets::SemanticFacets::for_available(
        node.properties.keys().copied(),
        std::iter::empty(),
    );
    node.facet_visibility = match selection.facets {
        FacetSelection::None => NavigationFacetVisibility::None,
        FacetSelection::Summary => NavigationFacetVisibility::Summary,
        FacetSelection::Full => NavigationFacetVisibility::Full,
    };
    node
}

pub(crate) fn resource_limit(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

pub(crate) fn source_unavailable(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, message)
}

fn decode_error(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{any::Any, collections::BTreeSet, path::PathBuf, sync::Mutex};
    use unica_format_core::{
        navigation::{
            Authorability, CapabilityState, IdentityStrength, NodeKind, PropertyCapability,
            PropertyValue, RelationGroupRef, RelationKind, RelationRef, RelationRole,
            ResolutionState, SemanticProperty, SemanticPropertyId, SemanticRelation,
            SourceAdapterDiagnostic,
        },
        ports::{
            CapturePort, CapturedSource, CapturedSourceSession, FormatInspectionPort,
            FormatInspectionRequest, FormatInspectionResult, OwnerResolutionRequest,
            OwnerResolutionResult, OwnershipPort, ProbePort, ReadPort, SupportEvidence,
            SupportInspectionRequest, SupportPort,
        },
        source::{
            AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency,
            SnapshotEvidence, SourceAccess, SourceFamily, SourceLocation, SourceSnapshot,
        },
    };

    struct FakePort {
        envelope: Mutex<NavigationEnvelope>,
        format: FormatVersion,
        target_identity: Mutex<TargetIdentity>,
    }

    struct FakeCapturedSession {
        source: SourceContext,
        snapshot: SourceSnapshot,
        binding: SourceBinding,
        envelope: NavigationEnvelope,
        format: FormatVersion,
    }

    impl CapturedSourceSession for FakeCapturedSession {
        fn source(&self) -> &SourceContext {
            &self.source
        }

        fn snapshot(&self) -> &SourceSnapshot {
            &self.snapshot
        }

        fn binding(&self) -> &SourceBinding {
            &self.binding
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl CapturePort for FakePort {
        fn capture(&self, source: &SourceContext) -> Result<CaptureResult, SourceAdapterError> {
            let envelope = self.envelope.lock().unwrap().clone();
            let snapshot = envelope.snapshot.clone().unwrap();
            let binding = SourceBinding::new(
                snapshot.source_id.clone(),
                SourceFamily::PlatformXml,
                None,
                self.target_identity.lock().unwrap().clone(),
                snapshot.revision.clone(),
            );
            Ok(CaptureResult::Captured(CapturedSource::new(
                FakeCapturedSession {
                    source: source.clone(),
                    snapshot,
                    binding,
                    envelope,
                    format: self.format.clone(),
                },
            )))
        }
    }

    impl ProbePort for FakePort {
        fn probe(&self, captured: &CapturedSource) -> Result<ProbeResult, SourceAdapterError> {
            let session = captured.adapter_state::<FakeCapturedSession>().unwrap();
            let snapshot = session.snapshot.clone();
            Ok(ProbeResult::Match(SourceDescriptor {
                source_id: snapshot.source_id,
                family: SourceFamily::PlatformXml,
                format_version: session.format.clone(),
                producer_version: None,
                detected_features: BTreeSet::new(),
                probe_evidence: vec!["fake erased probe".to_string()],
                snapshot_evidence: Some(SnapshotEvidence {
                    revision: snapshot.revision,
                    root_descriptor_digest: "sha256:root".to_string(),
                }),
            }))
        }
    }

    impl ReadPort for FakePort {
        fn read(
            &self,
            request: &FormatReadRequest,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(request
                .captured
                .adapter_state::<FakeCapturedSession>()
                .unwrap()
                .envelope
                .clone())
        }
    }

    impl OwnershipPort for FakePort {
        fn resolve(
            &self,
            _request: &OwnerResolutionRequest,
        ) -> Result<OwnerResolutionResult, SourceAdapterError> {
            Err(source_unavailable("unused fake ownership port"))
        }
    }

    impl FormatInspectionPort for FakePort {
        fn inspect(
            &self,
            _request: &FormatInspectionRequest,
        ) -> Result<FormatInspectionResult, SourceAdapterError> {
            Err(source_unavailable("unused fake format inspection port"))
        }
    }

    impl SupportPort for FakePort {
        fn inspect(
            &self,
            _request: &SupportInspectionRequest,
        ) -> Result<SupportEvidence, SourceAdapterError> {
            Err(source_unavailable("unused fake support port"))
        }
    }

    struct FakeResolver {
        located: Mutex<LocatedSource>,
        scope: Mutex<String>,
        unavailable: bool,
    }

    impl SourceRegistrationResolver for FakeResolver {
        fn locate(&self) -> Result<LocatedSource, SourceAdapterError> {
            if self.unavailable {
                Err(source_unavailable("project source map cannot be resolved"))
            } else {
                Ok(self.located.lock().unwrap().clone())
            }
        }

        fn authorize_continuation(
            &self,
            source_id: &SourceId,
        ) -> Result<CurrentSourceAuthorization, SourceAdapterError> {
            if self.unavailable {
                return Err(source_unavailable("project source map cannot be resolved"));
            }
            Ok(CurrentSourceAuthorization {
                source_id: source_id.clone(),
                authorization_scope: self.scope.lock().unwrap().clone(),
            })
        }
    }

    struct Harness {
        service: MetadataNavigationService,
        resolver: FakeResolver,
        port: Arc<FakePort>,
    }

    impl Harness {
        fn new(attribute_count: usize, format: &str) -> Self {
            Self::with_envelope(fixture_envelope(attribute_count), format)
        }

        fn with_envelope(envelope: NavigationEnvelope, format: &str) -> Self {
            let port = Arc::new(FakePort {
                envelope: Mutex::new(envelope),
                format: FormatVersion::parse(format).unwrap(),
                target_identity: Mutex::new(
                    TargetIdentity::from_normalized_relative_path("Catalogs/Items.xml").unwrap(),
                ),
            });
            let registration = registration(port.clone());
            let workspace = PathBuf::from("/authorized/workspace");
            let source_root = workspace.join("src");
            let target = source_root.join("Catalogs/Items.xml");
            let source = SourceContext::new(
                SourceLocation::new(workspace, source_root, target),
                Some("main".to_string()),
                SourceFamily::PlatformXml,
                None,
            );
            let located = LocatedSource {
                source,
                expected_source_id: SourceId::new("workspace:main").unwrap(),
                target_identity: TargetIdentity::from_normalized_relative_path(
                    "Catalogs/Items.xml",
                )
                .unwrap(),
                authorization_scope: "scope:one".to_string(),
                registration,
            };
            Self {
                service: MetadataNavigationService::new(b"application-test-cursor-secret".to_vec()),
                resolver: FakeResolver {
                    located: Mutex::new(located),
                    scope: Mutex::new("scope:one".to_string()),
                    unavailable: false,
                },
                port,
            }
        }

        fn inspect(&self, command: MetadataNavigationCommand) -> NavigationEnvelope {
            self.service.inspect(command, &self.resolver)
        }

        fn first_page(&self, page_size: u16) -> NavigationEnvelope {
            self.inspect(path_command(Some(selection(
                PropertySelection::All,
                FacetSelection::Summary,
                RelationKind::Contains,
                page_size,
            ))))
        }
    }

    fn registration(port: Arc<FakePort>) -> SourceAdapterRegistration {
        SourceAdapterRegistration {
            manifest: AdapterManifest {
                adapter_id: "fake-erased-adapter",
                adapter_version: "1",
                source_family: SourceFamily::PlatformXml,
                supported_formats: vec![FormatRange::exact(FormatVersion::parse("2.20").unwrap())],
                required_features: BTreeSet::new(),
                excluded_features: BTreeSet::new(),
                source_access: SourceAccess::ReadOnly,
                maturity: AdapterMaturity::SemanticParity,
            },
            capture: port.clone(),
            probe: port.clone(),
            read: port.clone(),
            ownership: port.clone(),
            format_inspection: port.clone(),
            support: port,
        }
    }

    fn fixture_envelope(attribute_count: usize) -> NavigationEnvelope {
        let source_id = SourceId::new("workspace:main").unwrap();
        let revision = SourceRevision::new("sha256:fixture").unwrap();
        let configuration = object_ref(&source_id, "uuid:configuration", "Configuration");
        let item = object_ref(&source_id, "uuid:items", "Items");
        let mut item_node = node(item.clone());
        item_node.properties.insert(
            SemanticPropertyId::METADATA_NAME,
            string_property(SemanticPropertyId::METADATA_NAME, "Items"),
        );
        item_node.properties.insert(
            SemanticPropertyId::METADATA_SYNONYM,
            string_property(SemanticPropertyId::METADATA_SYNONYM, "Items synonym"),
        );
        let mut nodes = vec![item_node];
        let mut relations = vec![relation(
            &source_id,
            &configuration,
            &item,
            RelationRole::Children,
            0,
        )];
        for index in 0..attribute_count {
            let name = if index == 0 {
                "Code".to_string()
            } else if index == 1 {
                "Description".to_string()
            } else {
                format!("Attribute{index:04}")
            };
            let attribute = object_ref(&source_id, &format!("uuid:attribute-{index}"), &name);
            let mut attribute_node = node(attribute.clone());
            attribute_node.properties.insert(
                SemanticPropertyId::METADATA_NAME,
                string_property(SemanticPropertyId::METADATA_NAME, &name),
            );
            attribute_node.properties.insert(
                SemanticPropertyId::METADATA_SYNONYM,
                string_property(
                    SemanticPropertyId::METADATA_SYNONYM,
                    &format!("{name} synonym"),
                ),
            );
            nodes.push(attribute_node);
            relations.push(relation(
                &source_id,
                &item,
                &attribute,
                RelationRole::Attributes,
                index + 1,
            ));
        }
        NavigationEnvelope {
            schema_version: "1".to_string(),
            status: NavigationStatus::Available,
            snapshot: Some(SourceSnapshot {
                source_id,
                revision,
                consistency: SnapshotConsistency::Consistent,
                adapter_id: "fake-erased-adapter".to_string(),
            }),
            root: Some(configuration),
            nodes,
            relations: Vec::new(),
            diagnostics: vec![SourceAdapterDiagnostic {
                code: "fake_probe".to_string(),
                message: "bounded fake diagnostic".to_string(),
                details: None,
            }],
            relation_index: Arc::new(relations),
        }
    }

    fn object_ref(source_id: &SourceId, key: &str, name: &str) -> ObjectRef {
        ObjectRef::new(
            source_id.clone(),
            ObjectKey::new(key).unwrap(),
            IdentityStrength::Persistent,
            NodeKind::Catalog,
            name,
        )
    }

    fn node(reference: ObjectRef) -> NavigationNode {
        NavigationNode::new(
            reference,
            CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
        )
    }

    fn string_property(id: SemanticPropertyId, value: &str) -> SemanticProperty {
        let value = if id == SemanticPropertyId::METADATA_SYNONYM {
            PropertyValue::LocalizedString(std::collections::BTreeMap::from([(
                "und".to_string(),
                value.to_string(),
            )]))
        } else {
            PropertyValue::String(value.to_string())
        };
        SemanticProperty::explicit(id, value)
            .unwrap()
            .with_capability(PropertyCapability::ReadOnly)
            .unwrap()
    }

    fn relation(
        source_id: &SourceId,
        source: &ObjectRef,
        target: &ObjectRef,
        role: RelationRole,
        index: usize,
    ) -> SemanticRelation {
        relation_with_kind(
            source_id,
            source,
            target,
            role,
            RelationKind::Contains,
            index,
        )
    }

    fn relation_with_kind(
        source_id: &SourceId,
        source: &ObjectRef,
        target: &ObjectRef,
        role: RelationRole,
        kind: RelationKind,
        index: usize,
    ) -> SemanticRelation {
        let group_ref =
            RelationGroupRef::new(source_id.clone(), source.clone(), role, kind).unwrap();
        let kind_label = match kind {
            RelationKind::Contains => "contains",
            RelationKind::References => "references",
        };
        SemanticRelation {
            relation_ref: RelationRef::new(
                source_id.clone(),
                format!("{kind_label}:{index}"),
                kind,
            )
            .unwrap(),
            group_ref,
            identity_strength: IdentityStrength::Persistent,
            kind,
            role,
            source: source.clone(),
            target: target.clone(),
            capability: node(target.clone()).capability,
        }
    }

    fn path_command(selection: Option<NavigationSelection>) -> MetadataNavigationCommand {
        MetadataNavigationCommand {
            target: MetadataNavigationTarget::Source,
            selection,
        }
    }

    fn cursor_command(cursor: NavigationCursor) -> MetadataNavigationCommand {
        MetadataNavigationCommand {
            target: MetadataNavigationTarget::Cursor(cursor.opaque()),
            selection: None,
        }
    }

    fn opaque_cursor(
        value: impl Into<String>,
    ) -> unica_format_core::navigation::OpaqueNavigationCursor {
        unica_format_core::navigation::OpaqueNavigationCursor::from_token(value)
    }

    fn cursor_token(cursor: &NavigationCursor) -> String {
        serde_json::to_value(cursor)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    fn tamper_cursor_token(cursor: &NavigationCursor) -> String {
        let mut token = cursor_token(cursor).into_bytes();
        let last = token.last_mut().unwrap();
        *last = if *last == b'A' { b'B' } else { b'A' };
        String::from_utf8(token).unwrap()
    }

    fn selection(
        properties: PropertySelection,
        facets: FacetSelection,
        kind: RelationKind,
        page_size: u16,
    ) -> NavigationSelection {
        let mut relation = unica_format_core::navigation::RelationSelection::new(
            SemanticRelationId::ATTRIBUTES,
            Some(page_size),
        )
        .unwrap();
        relation.kind = kind;
        NavigationSelection {
            properties,
            facets,
            relations: vec![relation],
        }
    }

    fn role_selection(
        role: RelationRole,
        facets: FacetSelection,
        page_size: u16,
    ) -> NavigationSelection {
        NavigationSelection {
            properties: PropertySelection::All,
            facets,
            relations: vec![unica_format_core::navigation::RelationSelection::new(
                role,
                Some(page_size),
            )
            .unwrap()],
        }
    }

    fn object_command(
        navigation: &NavigationEnvelope,
        key: ObjectKey,
    ) -> MetadataNavigationCommand {
        MetadataNavigationCommand {
            target: MetadataNavigationTarget::ObjectRef {
                source_id: navigation.snapshot.as_ref().unwrap().source_id.clone(),
                object_key: key,
                snapshot_revision: navigation.snapshot.as_ref().unwrap().revision.clone(),
            },
            selection: None,
        }
    }

    fn object_selection_command(
        navigation: &NavigationEnvelope,
        key: ObjectKey,
        selection: NavigationSelection,
    ) -> MetadataNavigationCommand {
        let mut command = object_command(navigation, key);
        command.selection = Some(selection);
        command
    }

    fn task6_relation_envelope() -> NavigationEnvelope {
        let source_id = SourceId::new("workspace:main").unwrap();
        let revision = SourceRevision::new("sha256:task6-relations").unwrap();
        let root = object_ref(&source_id, "uuid:task6-root", "Task6Root");
        let mut root_node = node(root.clone());
        root_node.properties.insert(
            SemanticPropertyId::METADATA_NAME,
            string_property(SemanticPropertyId::METADATA_NAME, "Task6Root"),
        );
        let mut nodes = vec![root_node];
        let mut relations = Vec::new();
        let roles = [
            SemanticRelationId::DIMENSIONS,
            SemanticRelationId::RESOURCES,
            SemanticRelationId::ENUM_VALUES,
            SemanticRelationId::URL_TEMPLATES,
            SemanticRelationId::METHODS,
            SemanticRelationId::OPERATIONS,
            SemanticRelationId::PARAMETERS,
            SemanticRelationId::BASED_ON,
            SemanticRelationId::REGISTER_RECORDS,
        ];
        let mut relation_index = 0usize;
        for role in roles {
            let owner = object_ref(
                &source_id,
                &format!("uuid:owner-{}", role.as_str()),
                &format!("Owner-{}", role.as_str()),
            );
            let mut owner_node = node(owner.clone());
            owner_node.properties.insert(
                SemanticPropertyId::METADATA_NAME,
                string_property(
                    SemanticPropertyId::METADATA_NAME,
                    &format!("Owner-{}", role.as_str()),
                ),
            );
            nodes.push(owner_node);
            for item_index in 0..2 {
                let shared_cross_role = item_index == 0
                    && matches!(
                        role,
                        SemanticRelationId::DIMENSIONS | SemanticRelationId::RESOURCES
                    );
                let name = if shared_cross_role {
                    "Shared".to_string()
                } else {
                    format!("{}-{item_index}", role.as_str())
                };
                let target = ObjectRef::new(
                    source_id.clone(),
                    ObjectKey::new(format!("derived:task6:{}:{item_index}", role.as_str()))
                        .unwrap(),
                    IdentityStrength::Derived,
                    NodeKind::Catalog,
                    name.clone(),
                );
                let mut target_node = node(target.clone());
                target_node.properties.insert(
                    SemanticPropertyId::METADATA_NAME,
                    string_property(SemanticPropertyId::METADATA_NAME, &name),
                );
                if role == SemanticRelationId::REGISTER_RECORDS {
                    target_node.capability_state = CapabilityState::new(
                        ResolutionState::Unresolved,
                        Authorability::UnknownReadOnly,
                    );
                    target_node.capability.resolution = ResolutionState::Unresolved;
                    target_node.capability.coverage =
                        unica_format_core::navigation::CoverageState::Partial;
                    target_node.capability.authorability = Authorability::UnknownReadOnly;
                }
                let kind = if role.is_reference_role() {
                    RelationKind::References
                } else {
                    RelationKind::Contains
                };
                let mut edge =
                    relation_with_kind(&source_id, &owner, &target, role, kind, relation_index);
                edge.capability = target_node.capability.clone();
                relations.push(edge);
                nodes.push(target_node);
                relation_index += 1;
            }
        }
        NavigationEnvelope {
            schema_version: "1".to_string(),
            status: NavigationStatus::Partial,
            snapshot: Some(SourceSnapshot {
                source_id,
                revision,
                consistency: SnapshotConsistency::Consistent,
                adapter_id: "fake-erased-adapter".to_string(),
            }),
            root: Some(root),
            nodes,
            relations: Vec::new(),
            diagnostics: vec![SourceAdapterDiagnostic {
                code: "referenceTargetUnresolved".to_string(),
                message: "a semantic reference target is outside the captured graph".to_string(),
                details: None,
            }],
            relation_index: Arc::new(relations),
        }
    }

    fn assert_unavailable(navigation: &NavigationEnvelope, code: &str) {
        assert_eq!(navigation.status, NavigationStatus::Unavailable);
        assert_eq!(navigation.diagnostics[0].code, code);
        assert!(navigation.snapshot.is_none());
        assert!(navigation.root.is_none());
        assert!(navigation.nodes.is_empty());
        assert!(navigation.relations.is_empty());
    }

    #[test]
    fn meta_info_returns_ready_navigation_for_platform_xml_2_20() {
        let harness = Harness::new(1, "2.20");
        let result = harness.inspect(path_command(None));
        assert_eq!(result.status, NavigationStatus::Available);
        assert_eq!(result.schema_version, "1");
        assert_eq!(result.root.as_ref().unwrap().display_name, "Items");
    }

    #[test]
    fn partial_navigation_still_validates_snapshot_binding() {
        let harness = Harness::new(1, "2.20");
        {
            let mut envelope = harness.port.envelope.lock().unwrap();
            envelope.status = NavigationStatus::Partial;
            envelope.snapshot.as_mut().unwrap().adapter_id = "wrong-adapter".to_string();
        }

        assert_unavailable(
            &harness.inspect(path_command(None)),
            "snapshot_inconsistent",
        );
    }

    #[test]
    fn partial_node_forces_partial_envelope_and_neutral_diagnostic() {
        let harness = Harness::new(1, "2.20");
        harness.port.envelope.lock().unwrap().nodes[0]
            .capability
            .coverage = unica_format_core::navigation::CoverageState::Partial;

        let result = harness.first_page(1);

        assert_eq!(result.status, NavigationStatus::Partial);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partialCoverage"));
    }

    #[test]
    fn unknown_node_coverage_forces_partial_envelope_and_neutral_diagnostic() {
        let harness = Harness::new(1, "2.20");
        harness.port.envelope.lock().unwrap().nodes[0]
            .capability
            .coverage = unica_format_core::navigation::CoverageState::Unknown;

        let result = harness.first_page(1);

        assert_eq!(result.status, NavigationStatus::Partial);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partialCoverage"));
    }

    #[test]
    fn unknown_relation_coverage_forces_partial_envelope_and_neutral_diagnostic() {
        let harness = Harness::new(1, "2.20");
        let mut envelope = harness.port.envelope.lock().unwrap();
        std::sync::Arc::make_mut(&mut envelope.relation_index)[0]
            .capability
            .coverage = unica_format_core::navigation::CoverageState::Unknown;
        drop(envelope);

        let result = harness.first_page(1);

        assert_eq!(result.status, NavigationStatus::Partial);
        assert!(result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partialCoverage"));
    }

    #[test]
    fn partial_cursor_retains_explanatory_diagnostics() {
        let harness = Harness::new(2, "2.20");
        {
            let mut envelope = harness.port.envelope.lock().unwrap();
            envelope.status = NavigationStatus::Partial;
            envelope.nodes[0].capability.coverage =
                unica_format_core::navigation::CoverageState::Partial;
            envelope.diagnostics.push(SourceAdapterDiagnostic {
                code: "unmappedSemanticFact".to_string(),
                message: "a semantic fact is not mapped".to_string(),
                details: None,
            });
        }
        let first = harness.first_page(1);

        let continued = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));

        assert_eq!(continued.status, NavigationStatus::Partial);
        assert!(continued
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unmappedSemanticFact"));
    }

    #[test]
    fn selected_facets_reference_only_returned_properties_and_relation_pages() {
        let harness = Harness::new(2, "2.20");
        {
            let mut envelope = harness.port.envelope.lock().unwrap();
            let item = &mut envelope.nodes[0];
            item.facets = unica_format_core::facets::SemanticFacets::for_available(
                item.properties.keys().copied(),
                [SemanticRelationId::ATTRIBUTES, SemanticRelationId::COMMANDS],
            );
        }
        let result = harness.inspect(path_command(Some(selection(
            PropertySelection::Named(BTreeSet::from([SemanticPropertyId::METADATA_NAME])),
            FacetSelection::Full,
            RelationKind::Contains,
            1,
        ))));

        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value["nodes"][0]["facets"]["identity"],
            serde_json::json!(["metadata.name"])
        );
        assert_eq!(
            value["nodes"][0]["facets"]["fields"],
            serde_json::json!(["attributes"])
        );
        assert!(!value["nodes"][0]["facets"].to_string().contains("commands"));
    }

    #[test]
    fn unsupported_version_returns_only_navigation_unavailability() {
        let harness = Harness::new(1, "2.19");
        assert_unavailable(&harness.inspect(path_command(None)), "format_unsupported");
    }

    #[test]
    fn project_map_failure_is_not_replaced_with_ad_hoc_identity() {
        let mut harness = Harness::new(1, "2.20");
        harness.resolver.unavailable = true;
        let result = harness.inspect(path_command(None));
        assert_unavailable(&result, "source_unavailable");
        assert!(!serde_json::to_string(&result).unwrap().contains("ad-hoc:"));
    }

    #[test]
    fn meta_info_relation_cursor_preserves_child_nodes_and_changes_page() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        assert_eq!(first.relations[0].items[0].object_ref.display_name, "Code");
        assert!(serde_json::to_value(&first).unwrap()["relations"][0]["nextCursor"].is_string());
        let second = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));
        assert_eq!(
            second.relations[0].items[0].object_ref.display_name,
            "Description"
        );
    }

    #[test]
    fn every_target_mode_is_path_free() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let by_ref = harness.inspect(object_command(
            &first,
            first.root.as_ref().unwrap().object_key.clone(),
        ));
        let by_cursor = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));
        for result in [&first, &by_ref, &by_cursor] {
            assert!(!serde_json::to_string(result)
                .unwrap()
                .contains("/authorized/workspace"));
        }
    }

    #[test]
    fn tampered_cursor_is_structured_unavailable() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let cursor = tamper_cursor_token(first.relations[0].next_cursor.as_ref().unwrap());
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor(cursor)),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn malformed_base64_cursor_is_structured_unavailable() {
        let harness = Harness::new(2, "2.20");
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor("not+base64url")),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn cursor_payload_tampering_is_structured_unavailable() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let tampered = tamper_cursor_token(first.relations[0].next_cursor.as_ref().unwrap());
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor(tampered)),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn opaque_cursor_tampering_fails_before_continuation_lookup() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let continuation = first.relations[0].next_cursor.clone().unwrap();
        let forged = tamper_cursor_token(&continuation);
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor(forged)),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn retained_object_ref_absence_is_snapshot_stale() {
        let harness = Harness::new(1, "2.20");
        let first = harness.inspect(path_command(None));
        assert_unavailable(
            &harness.inspect(object_command(
                &first,
                ObjectKey::new("uuid:absent").unwrap(),
            )),
            "snapshot_stale",
        );
    }

    #[test]
    fn object_ref_cache_miss_and_cross_scope_are_structured_unavailable() {
        let harness = Harness::new(1, "2.20");
        let first = harness.inspect(path_command(None));
        *harness.resolver.scope.lock().unwrap() = "scope:other".to_string();
        assert_unavailable(
            &harness.inspect(object_command(
                &first,
                first.root.as_ref().unwrap().object_key.clone(),
            )),
            "snapshot_stale",
        );
    }

    #[test]
    fn continuation_scope_includes_workspace_epoch_and_source_set_kind() {
        let harness = Harness::new(1, "2.20");
        let first = harness.inspect(path_command(None));
        *harness.resolver.scope.lock().unwrap() = "scope:epoch-or-kind-changed".to_string();
        assert_unavailable(
            &harness.inspect(object_command(
                &first,
                first.root.as_ref().unwrap().object_key.clone(),
            )),
            "snapshot_stale",
        );
    }

    #[test]
    fn retained_cursor_and_object_ref_use_the_original_snapshot_after_live_source_mutation() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        *harness.port.envelope.lock().unwrap() = fixture_envelope(0);
        let continued = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));
        assert_eq!(
            continued.relations[0].items[0].object_ref.display_name,
            "Description"
        );
        let by_ref = harness.inspect(object_command(
            &first,
            first.root.as_ref().unwrap().object_key.clone(),
        ));
        assert_eq!(by_ref.root, first.root);
    }

    #[test]
    fn shared_source_and_revision_never_cross_hit_between_target_identities() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let first_cursor = first.relations[0].next_cursor.clone().unwrap();
        let second_identity =
            TargetIdentity::from_normalized_relative_path("Catalogs/Other.xml").unwrap();
        *harness.port.target_identity.lock().unwrap() = second_identity.clone();
        harness.resolver.located.lock().unwrap().target_identity = second_identity;
        let mut second_envelope = fixture_envelope(2);
        second_envelope.nodes[0].properties.insert(
            SemanticPropertyId::METADATA_NAME,
            string_property(SemanticPropertyId::METADATA_NAME, "Other"),
        );
        *harness.port.envelope.lock().unwrap() = second_envelope;
        let second = harness.first_page(1);
        assert_eq!(
            second.nodes[0].properties[&SemanticPropertyId::METADATA_NAME].value(),
            Some(&PropertyValue::String("Other".to_string()))
        );

        let continued = harness.inspect(cursor_command(first_cursor));
        assert_eq!(continued.root, first.root);
        assert_eq!(
            continued.nodes[0].properties[&SemanticPropertyId::METADATA_NAME].value(),
            Some(&PropertyValue::String("Items".to_string()))
        );
        assert_unavailable(
            &harness.inspect(object_command(
                &first,
                first.root.as_ref().unwrap().object_key.clone(),
            )),
            "identity_collision",
        );
    }

    #[test]
    fn continuation_scope_changes_when_configured_symlink_retargets() {
        let harness = Harness::new(1, "2.20");
        let first = harness.inspect(path_command(None));
        *harness.resolver.scope.lock().unwrap() = "scope:retargeted".to_string();
        assert_unavailable(
            &harness.inspect(object_command(
                &first,
                first.root.as_ref().unwrap().object_key.clone(),
            )),
            "snapshot_stale",
        );
    }

    #[test]
    fn captured_object_path_binding_projects_original_bytes_after_source_mutation_and_retarget() {
        retained_cursor_and_object_ref_use_the_original_snapshot_after_live_source_mutation();
    }

    #[test]
    fn relation_pages_group_explicit_roles_and_keep_edge_refs_unique() {
        let harness = Harness::new(2, "2.20");
        let result = harness.first_page(2);
        assert_eq!(result.relations[0].relation.role, RelationRole::Attributes);
        assert_ne!(
            result.relation_index[1].relation_ref.relation_key,
            result.relation_index[2].relation_ref.relation_key,
        );
    }

    #[test]
    fn relation_cursor_uses_stable_group_identity() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let cursor = first.relations[0].next_cursor.clone().unwrap();
        assert_eq!(cursor.relation, first.relations[0].relation.group_key);
        let second = harness.inspect(cursor_command(cursor));
        assert_eq!(second.relations[0].relation, first.relations[0].relation);
    }

    #[test]
    fn five_thousand_attributes_keep_a_truthful_cursor_and_stable_next_page() {
        let harness = Harness::new(5_000, "2.20");
        let first = harness.first_page(100);
        assert_eq!(first.relations[0].items.len(), 100);
        let second = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));
        assert_eq!(second.relations[0].items.len(), 100);
        assert_eq!(
            second.relations[0].items[0].object_ref.display_name,
            "Attribute0100"
        );
    }

    #[test]
    fn cursor_is_bounded_before_strict_selection_decoding() {
        let harness = Harness::new(2, "2.20");
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor(
                    "A".repeat(unica_format_core::limits::MAX_NAVIGATION_CURSOR_TOKEN_BYTES + 1),
                )),
                selection: None,
            }),
            "resource_limit",
        );
    }

    #[test]
    fn response_shares_the_retained_relation_index_without_copying_pages() {
        let harness = Harness::new(2, "2.20");
        let source = harness.port.envelope.lock().unwrap().relation_index.clone();
        let response = harness.first_page(1);
        assert!(Arc::ptr_eq(&response.relation_index, &source));
    }

    #[test]
    fn bounded_diagnostics_are_initial_only_for_object_ref_and_cursor_continuations() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        assert_eq!(first.diagnostics.len(), 1);
        let by_ref = harness.inspect(object_command(
            &first,
            first.root.as_ref().unwrap().object_key.clone(),
        ));
        let by_cursor = harness.inspect(cursor_command(
            first.relations[0].next_cursor.clone().unwrap(),
        ));
        assert!(by_ref.diagnostics.is_empty());
        assert!(by_cursor.diagnostics.is_empty());
    }

    #[test]
    fn cursor_resume_materializes_only_its_bound_group_and_keeps_full_selection() {
        let harness = Harness::new(3, "2.20");
        let first = harness.inspect(path_command(Some(selection(
            PropertySelection::Named(BTreeSet::from([SemanticPropertyId::METADATA_NAME])),
            FacetSelection::None,
            RelationKind::Contains,
            1,
        ))));
        let cursor = first.relations[0].next_cursor.clone().unwrap();
        assert_eq!(cursor.selection.relations.len(), 1);
        let second = harness.inspect(cursor_command(cursor));
        assert_eq!(second.relations.len(), 1);
        assert_eq!(second.relations[0].items[0].properties.len(), 1);
        assert_eq!(
            second.relations[0].items[0].facet_visibility,
            NavigationFacetVisibility::None
        );
    }

    #[test]
    fn task6_fix1_materializes_every_specialized_role_with_stable_opaque_pages() {
        let harness = Harness::with_envelope(task6_relation_envelope(), "2.20");
        let bootstrap = harness.inspect(path_command(None));
        let roles = [
            SemanticRelationId::DIMENSIONS,
            SemanticRelationId::RESOURCES,
            SemanticRelationId::ENUM_VALUES,
            SemanticRelationId::URL_TEMPLATES,
            SemanticRelationId::METHODS,
            SemanticRelationId::OPERATIONS,
            SemanticRelationId::PARAMETERS,
            SemanticRelationId::BASED_ON,
            SemanticRelationId::REGISTER_RECORDS,
        ];
        let mut first_items = std::collections::BTreeMap::new();
        let mut authenticated_cursor = None;

        for role in roles {
            let owner_key = ObjectKey::new(format!("uuid:owner-{}", role.as_str())).unwrap();
            let first = harness.inspect(object_selection_command(
                &bootstrap,
                owner_key,
                role_selection(role, FacetSelection::Full, 1),
            ));
            assert_eq!(first.status, NavigationStatus::Partial);
            assert_eq!(first.relations.len(), 1);
            let page = &first.relations[0];
            assert_eq!(page.relation.role, role);
            assert_eq!(
                page.relation.kind,
                if role.is_reference_role() {
                    RelationKind::References
                } else {
                    RelationKind::Contains
                }
            );
            assert_eq!(page.items.len(), 1);
            assert_eq!(
                page.items[0].facet_visibility,
                NavigationFacetVisibility::Full
            );
            let cursor = page.next_cursor.clone().expect("second page cursor");
            let token = cursor_token(&cursor);
            assert!(!token.contains(role.as_str()));
            assert!(!token.contains(page.items[0].object_ref.object_key.as_str()));

            let second = harness.inspect(cursor_command(cursor.clone()));
            assert_eq!(second.relations.len(), 1);
            assert_eq!(second.relations[0].relation, page.relation);
            assert_ne!(
                second.relations[0].items[0].object_ref,
                page.items[0].object_ref
            );
            first_items.insert(role, page.items[0].object_ref.clone());
            authenticated_cursor = Some(cursor);
        }

        assert_eq!(
            first_items[&SemanticRelationId::DIMENSIONS].display_name,
            "Shared"
        );
        assert_eq!(
            first_items[&SemanticRelationId::RESOURCES].display_name,
            "Shared"
        );
        assert_ne!(
            first_items[&SemanticRelationId::DIMENSIONS].object_key,
            first_items[&SemanticRelationId::RESOURCES].object_key
        );
        assert_eq!(
            first_items[&SemanticRelationId::BASED_ON].kind,
            NodeKind::Catalog
        );
        let stub = &first_items[&SemanticRelationId::REGISTER_RECORDS];
        let stub_page = harness.inspect(object_selection_command(
            &bootstrap,
            ObjectKey::new("uuid:owner-registerRecords").unwrap(),
            role_selection(
                SemanticRelationId::REGISTER_RECORDS,
                FacetSelection::Full,
                1,
            ),
        ));
        assert_eq!(stub_page.relations[0].items[0].object_ref, *stub);
        assert_eq!(
            stub_page.relations[0].items[0].capability.resolution,
            ResolutionState::Unresolved
        );

        let forged = tamper_cursor_token(authenticated_cursor.as_ref().unwrap());
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(opaque_cursor(forged)),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn select_filters_properties_facets_and_relation_kind_at_runtime() {
        let harness = Harness::new(1, "2.20");
        let selected = harness.inspect(path_command(Some(selection(
            PropertySelection::Named(BTreeSet::from([SemanticPropertyId::METADATA_NAME])),
            FacetSelection::None,
            RelationKind::Contains,
            1,
        ))));
        assert_eq!(selected.nodes[0].properties.len(), 1);
        assert_eq!(
            selected.nodes[0].facet_visibility,
            NavigationFacetVisibility::None
        );
        let references = harness.inspect(path_command(Some(selection(
            PropertySelection::All,
            FacetSelection::Summary,
            RelationKind::References,
            1,
        ))));
        assert!(references.relations.is_empty());
    }
}
