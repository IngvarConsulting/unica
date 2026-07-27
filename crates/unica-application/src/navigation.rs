use std::sync::{Arc, Mutex};

use serde_json::Value;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationCursor, NavigationEnvelope, NavigationFacetVisibility,
        NavigationNode, NavigationQuery, NavigationRelationPage, NavigationSelection,
        NavigationStatus, NavigationTarget, ObjectKey, ObjectRef, PropertySelection,
        SemanticRelation,
    },
    ports::{CaptureResult, FormatReadRequest, ProbeResult, SourceAdapterRegistration},
    source::{
        SourceAdapterError, SourceAdapterErrorKind, SourceBinding, SourceContext, SourceDescriptor,
        SourceId, SourceRevision, TargetIdentity,
    },
};

use crate::{
    commands::{MetadataNavigationCommand, MetadataNavigationTarget},
    selection::{parse_navigation_selection, preflight_navigation_command},
    snapshot_cache::{CachedNavigation, SnapshotCache, SnapshotCacheAdmission},
};

pub trait SourceRegistrationResolver: Send + Sync {
    fn locate(&self, object_path: &str) -> Result<LocatedSource, SourceAdapterError>;

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
        let cursor_value = match &command.target {
            MetadataNavigationTarget::Cursor(value) => Some(value),
            _ => None,
        };
        preflight_navigation_command(command.selection.as_ref(), cursor_value)?;
        let (navigation, target_ref, selection, cursor, include_cached_diagnostics) =
            match command.target {
                MetadataNavigationTarget::ObjectPath(path) => {
                    let located = resolver.locate(&path)?;
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
                        parse_navigation_selection(command.selection.as_ref())?,
                        None,
                        true,
                    )
                }
                MetadataNavigationTarget::ObjectRef {
                    source_id,
                    object_key,
                    snapshot_revision,
                } => {
                    let (navigation, target_ref) = self.cached_navigation_target(
                        &source_id,
                        &object_key,
                        &snapshot_revision,
                        resolver,
                    )?;
                    (
                        navigation,
                        target_ref,
                        parse_navigation_selection(command.selection.as_ref())?,
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
                    NavigationCursor::authenticate(&value, &self.cursor_secret)?;
                    let source_id = cursor_source_id(&value)?;
                    let target_key = cursor_target_key(&value)?;
                    let requested_revision = cursor_snapshot_revision(&value)?;
                    let (navigation, target_ref) = self.cached_navigation_target(
                        &source_id,
                        &target_key,
                        &requested_revision,
                        resolver,
                    )?;
                    let snapshot = navigation.snapshot.as_ref().ok_or_else(|| {
                        source_unavailable("navigation cursor source has no truthful snapshot")
                    })?;
                    let selection = parse_navigation_selection(value.get("selection"))?;
                    let cursor = NavigationCursor::decode_authenticated(
                        &value,
                        &snapshot.revision,
                        &selection,
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
                    (navigation, target_ref, selection, Some(cursor), false)
                }
            };
        materialize_navigation_pages_with_secret(
            navigation.as_ref(),
            target_ref,
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
        resolver: &dyn SourceRegistrationResolver,
    ) -> Result<(Arc<NavigationEnvelope>, ObjectRef), SourceAdapterError> {
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
        let navigation = cache
            .navigation(&authorization.authorization_scope, source_id, revision)
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
            [target] => Ok((navigation, target.clone())),
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
    let snapshot = match located.registration.capture.capture(&located.source)? {
        CaptureResult::NoMatch => {
            return Err(source_unavailable(
                "no source capture adapter recognized the target",
            ))
        }
        CaptureResult::Captured(snapshot) => snapshot,
    };
    let binding = SourceBinding::new(
        snapshot.source_id.clone(),
        located.source.declared_family().clone(),
        located.source.declared_format().cloned(),
        located.target_identity.clone(),
        snapshot.revision.clone(),
    );
    if binding.source_id != located.expected_source_id
        || binding.family != located.registration.manifest.source_family
    {
        return Err(source_unavailable(
            "captured source binding does not match the authorized source set",
        ));
    }
    let descriptor = match located.registration.probe.probe(&located.source)? {
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
    let target_path = source_target_path(&located.source)?;
    let envelope = located.registration.read.read(&FormatReadRequest {
        source: located.source.clone(),
        snapshot,
        query: NavigationQuery {
            target: NavigationTarget::ObjectPath(target_path),
            select: NavigationSelection {
                properties: PropertySelection::All,
                facets: FacetSelection::Full,
                relations: Vec::new(),
            },
        },
    })?;
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
    if envelope.status != NavigationStatus::Available {
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

fn source_target_path(source: &SourceContext) -> Result<String, SourceAdapterError> {
    let path = source
        .location()
        .target()
        .strip_prefix(source.location().source_root())
        .map_err(|_| source_unavailable("source target is outside its source root"))?
        .to_str()
        .ok_or_else(|| source_unavailable("source target path is not UTF-8"))?
        .replace('\\', "/");
    Ok(if path.is_empty() {
        "source".to_string()
    } else {
        path
    })
}

fn cursor_source_id(value: &Value) -> Result<SourceId, SourceAdapterError> {
    SourceId::new(
        value
            .get("sourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| decode_error("navigation cursor has no valid sourceId"))?,
    )
}

fn cursor_target_key(value: &Value) -> Result<ObjectKey, SourceAdapterError> {
    ObjectKey::new(
        value
            .get("target")
            .and_then(Value::as_str)
            .ok_or_else(|| decode_error("navigation cursor has no valid target"))?,
    )
}

fn cursor_snapshot_revision(value: &Value) -> Result<SourceRevision, SourceAdapterError> {
    SourceRevision::new(
        value
            .get("snapshotRevision")
            .and_then(Value::as_str)
            .ok_or_else(|| decode_error("navigation cursor has no valid snapshotRevision"))?,
    )
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
    let target_node = project_selected_node(target_node, &selection);
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
                NavigationCursor::issue(
                    cursor_secret,
                    snapshot.source_id.clone(),
                    snapshot.revision.clone(),
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
    Ok(NavigationEnvelope {
        schema_version: navigation.schema_version.clone(),
        status: navigation.status,
        snapshot: Some(snapshot),
        root: Some(target),
        nodes: vec![target_node],
        relations: pages,
        diagnostics: include_cached_diagnostics
            .then(|| navigation.diagnostics.clone())
            .unwrap_or_default(),
        relation_index: Arc::clone(&navigation.relation_index),
    })
}

pub(crate) fn project_selected_node(
    mut node: NavigationNode,
    selection: &NavigationSelection,
) -> NavigationNode {
    if let PropertySelection::Named(names) = &selection.properties {
        node.properties.retain(|name, _| names.contains(name));
    }
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
    use serde_json::json;
    use std::{collections::BTreeSet, path::PathBuf, sync::Mutex};
    use unica_format_core::{
        navigation::{
            Authorability, CapabilityState, IdentityStrength, NodeKind, PropertyCapability,
            PropertyProvenance, PropertyType, PropertyValue, PropertyValueState, RelationGroupRef,
            RelationKind, RelationRef, RelationRole, ResolutionState, SemanticProperty,
            SemanticRelation, SourceAdapterDiagnostic,
        },
        ports::{
            AdapterFormatProfile, CapturePort, FormatInspectionPort, FormatInspectionRequest,
            FormatInspectionResult, OwnerResolutionRequest, OwnerResolutionResult, OwnershipPort,
            ProbePort, ReadPort, SupportEvidence, SupportInspectionRequest, SupportPort,
        },
        source::{
            AdapterManifest, AdapterMaturity, FormatRange, FormatVersion, SnapshotConsistency,
            SnapshotEvidence, SourceAccess, SourceFamily, SourceLocation, SourceSnapshot,
        },
    };

    struct FakePort {
        envelope: Mutex<NavigationEnvelope>,
        format: FormatVersion,
    }

    impl CapturePort for FakePort {
        fn capture(&self, _source: &SourceContext) -> Result<CaptureResult, SourceAdapterError> {
            Ok(CaptureResult::Captured(
                self.envelope.lock().unwrap().snapshot.clone().unwrap(),
            ))
        }
    }

    impl ProbePort for FakePort {
        fn probe(&self, _source: &SourceContext) -> Result<ProbeResult, SourceAdapterError> {
            let snapshot = self.envelope.lock().unwrap().snapshot.clone().unwrap();
            Ok(ProbeResult::Match(SourceDescriptor {
                source_id: snapshot.source_id,
                family: SourceFamily::PlatformXml,
                format_version: self.format.clone(),
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
            _request: &FormatReadRequest,
        ) -> Result<NavigationEnvelope, SourceAdapterError> {
            Ok(self.envelope.lock().unwrap().clone())
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
        located: LocatedSource,
        scope: Mutex<String>,
        unavailable: bool,
    }

    impl SourceRegistrationResolver for FakeResolver {
        fn locate(&self, _object_path: &str) -> Result<LocatedSource, SourceAdapterError> {
            if self.unavailable {
                Err(source_unavailable("project source map cannot be resolved"))
            } else {
                Ok(self.located.clone())
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
            let envelope = fixture_envelope(attribute_count);
            let port = Arc::new(FakePort {
                envelope: Mutex::new(envelope),
                format: FormatVersion::parse(format).unwrap(),
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
                    located,
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
            self.inspect(path_command(Some(json!({
                "relations": [{"role": "attributes", "pageSize": page_size}]
            }))))
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
            profile: AdapterFormatProfile {
                platform_line: "8.3",
                export_format: "2.20",
                legacy_metadata_classes: &[],
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
        item_node
            .properties
            .insert("name".to_string(), string_property("Items"));
        item_node
            .properties
            .insert("synonym".to_string(), string_property("Items synonym"));
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
            attribute_node
                .properties
                .insert("name".to_string(), string_property(&name));
            attribute_node.properties.insert(
                "synonym".to_string(),
                string_property(&format!("{name} synonym")),
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
            NodeKind::MetadataObject {
                metadata_type: "Catalog".to_string(),
            },
            name,
        )
    }

    fn node(reference: ObjectRef) -> NavigationNode {
        NavigationNode::new(
            reference,
            CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
        )
    }

    fn string_property(value: &str) -> SemanticProperty {
        SemanticProperty {
            value_type: PropertyType::String,
            value_state: PropertyValueState::Explicit,
            value: Some(PropertyValue::String(value.to_string())),
            provenance: PropertyProvenance::Descriptor,
            capability: PropertyCapability::ReadOnly,
        }
    }

    fn relation(
        source_id: &SourceId,
        source: &ObjectRef,
        target: &ObjectRef,
        role: RelationRole,
        index: usize,
    ) -> SemanticRelation {
        let group_ref = RelationGroupRef::new(
            source_id.clone(),
            source.clone(),
            role,
            RelationKind::Contains,
        )
        .unwrap();
        SemanticRelation {
            relation_ref: RelationRef::new(
                source_id.clone(),
                format!("contains:{index}"),
                RelationKind::Contains,
            )
            .unwrap(),
            group_ref,
            identity_strength: IdentityStrength::Persistent,
            kind: RelationKind::Contains,
            role,
            source: source.clone(),
            target: target.clone(),
            capability: node(target.clone()).capability,
        }
    }

    fn path_command(selection: Option<Value>) -> MetadataNavigationCommand {
        MetadataNavigationCommand {
            target: MetadataNavigationTarget::ObjectPath("Catalogs/Items.xml".to_string()),
            selection,
        }
    }

    fn cursor_command(cursor: NavigationCursor) -> MetadataNavigationCommand {
        MetadataNavigationCommand {
            target: MetadataNavigationTarget::Cursor(serde_json::to_value(cursor).unwrap()),
            selection: None,
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
        let mut cursor =
            serde_json::to_value(first.relations[0].next_cursor.clone().unwrap()).unwrap();
        cursor["nextPosition"] = json!(99);
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(cursor),
                selection: None,
            }),
            "decode_corrupted",
        );
    }

    #[test]
    fn authenticated_cursor_rejects_recomputed_public_selection_hash_and_u64_max_position() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let continuation = first.relations[0].next_cursor.clone().unwrap();
        let mut forged = serde_json::to_value(continuation).unwrap();
        forged["nextPosition"] = json!(u64::MAX);
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(forged),
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
    fn public_selection_limits_are_checked_before_normalization() {
        let harness = Harness::new(1, "2.20");
        let properties = (0..=unica_format_core::limits::MAX_NAVIGATION_PROPERTY_SELECTORS)
            .map(|index| format!("property{index}"))
            .collect::<Vec<_>>();
        assert_unavailable(
            &harness.inspect(path_command(Some(json!({"properties": properties})))),
            "resource_limit",
        );
    }

    #[test]
    fn cursor_is_bounded_and_authenticated_before_selection_normalization() {
        let harness = Harness::new(2, "2.20");
        let first = harness.first_page(1);
        let mut cursor =
            serde_json::to_value(first.relations[0].next_cursor.clone().unwrap()).unwrap();
        cursor["selection"] = json!({"properties": ["duplicate", "duplicate"]});
        assert_unavailable(
            &harness.inspect(MetadataNavigationCommand {
                target: MetadataNavigationTarget::Cursor(cursor),
                selection: None,
            }),
            "decode_corrupted",
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
        let first = harness.inspect(path_command(Some(json!({
            "properties": ["name"],
            "facets": "none",
            "relations": [{"role": "attributes", "pageSize": 1}]
        }))));
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
    fn select_filters_properties_facets_and_relation_kind_at_runtime() {
        let harness = Harness::new(1, "2.20");
        let selected = harness.inspect(path_command(Some(json!({
            "properties": ["name"],
            "facets": "none",
            "relations": [{"role": "attributes", "kind": "contains", "pageSize": 1}]
        }))));
        assert_eq!(selected.nodes[0].properties.len(), 1);
        assert_eq!(
            selected.nodes[0].facet_visibility,
            NavigationFacetVisibility::None
        );
        let references = harness.inspect(path_command(Some(json!({
            "relations": [{"role": "attributes", "kind": "references", "pageSize": 1}]
        }))));
        assert!(references.relations.is_empty());
    }
}
