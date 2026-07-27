use std::{
    collections::VecDeque,
    io::{self, Write},
};

use serde::Serialize;
use unica_format_core::{
    limits::{
        MAX_NAVIGATION_DIAGNOSTICS_BYTES as SNAPSHOT_CACHE_MAX_DIAGNOSTICS_BYTES,
        MAX_NAVIGATION_DIAGNOSTIC_DETAILS_BYTES as SNAPSHOT_CACHE_MAX_DIAGNOSTIC_DETAILS_BYTES,
        MAX_NAVIGATION_NESTING_DEPTH as SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_DEPTH,
        MAX_NAVIGATION_NODES as SNAPSHOT_CACHE_MAX_NODES,
        MAX_NAVIGATION_PROPERTIES_PER_NODE as SNAPSHOT_CACHE_MAX_PROPERTIES_PER_NODE,
        MAX_NAVIGATION_PROPERTY_BYTES as SNAPSHOT_CACHE_MAX_PROPERTY_BYTES,
        MAX_NAVIGATION_PROPERTY_VALUE_BYTES as SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_BYTES,
        MAX_NAVIGATION_RELATIONS as SNAPSHOT_CACHE_MAX_RELATIONS,
        MAX_NAVIGATION_SEMANTIC_STRING_BYTES as SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES,
    },
    navigation::{NavigationEnvelope, NavigationNode, ObjectRef},
    source::{
        SourceAdapterError, SourceAdapterErrorKind, SourceBinding, SourceId, SourceRevision,
        TargetIdentity,
    },
};

use crate::navigation::{resource_limit, source_unavailable};
const SNAPSHOT_CACHE_CAPACITY: usize = 64;
const SNAPSHOT_CACHE_MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;
const SNAPSHOT_CACHE_MAX_TOTAL_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy)]
pub(crate) struct SnapshotCacheLimits {
    max_entries: usize,
    max_snapshot_bytes: usize,
    max_total_bytes: usize,
    max_nodes: usize,
    max_relations: usize,
    max_properties_per_node: usize,
    max_property_bytes: usize,
    max_property_value_bytes: usize,
    max_semantic_string_bytes: usize,
    max_diagnostic_details_bytes: usize,
    max_diagnostics_bytes: usize,
    max_property_value_depth: usize,
}

pub(crate) const DEFAULT_SNAPSHOT_CACHE_LIMITS: SnapshotCacheLimits = SnapshotCacheLimits {
    max_entries: SNAPSHOT_CACHE_CAPACITY,
    max_snapshot_bytes: SNAPSHOT_CACHE_MAX_SNAPSHOT_BYTES,
    max_total_bytes: SNAPSHOT_CACHE_MAX_TOTAL_BYTES,
    max_nodes: SNAPSHOT_CACHE_MAX_NODES,
    max_relations: SNAPSHOT_CACHE_MAX_RELATIONS,
    max_properties_per_node: SNAPSHOT_CACHE_MAX_PROPERTIES_PER_NODE,
    max_property_bytes: SNAPSHOT_CACHE_MAX_PROPERTY_BYTES,
    max_property_value_bytes: SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_BYTES,
    max_semantic_string_bytes: SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES,
    max_diagnostic_details_bytes: SNAPSHOT_CACHE_MAX_DIAGNOSTIC_DETAILS_BYTES,
    max_diagnostics_bytes: SNAPSHOT_CACHE_MAX_DIAGNOSTICS_BYTES,
    max_property_value_depth: SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_DEPTH,
};

pub(crate) enum SnapshotCacheAdmission {
    Admitted(std::sync::Arc<unica_format_core::navigation::NavigationEnvelope>),
    ResourceLimit,
}

#[derive(Debug, Clone)]
pub(crate) struct CachedNavigation {
    scope: String,
    binding: SourceBinding,
    navigation: std::sync::Arc<unica_format_core::navigation::NavigationEnvelope>,
    charged_bytes: usize,
}

#[derive(Serialize)]
struct CachedNavigationCharge<'a> {
    scope: &'a str,
    binding: &'a SourceBinding,
    navigation: &'a unica_format_core::navigation::NavigationEnvelope,
    relation_index: &'a [unica_format_core::navigation::SemanticRelation],
}

pub(crate) struct SnapshotCache {
    pub(crate) limits: SnapshotCacheLimits,
    entries: VecDeque<CachedNavigation>,
    charged_bytes: usize,
}

impl Default for SnapshotCache {
    fn default() -> Self {
        Self::with_limits(DEFAULT_SNAPSHOT_CACHE_LIMITS)
    }
}

impl SnapshotCache {
    pub(crate) fn with_limits(limits: SnapshotCacheLimits) -> Self {
        Self {
            limits,
            entries: VecDeque::new(),
            charged_bytes: 0,
        }
    }

    pub(crate) fn admit(
        &mut self,
        entry: CachedNavigation,
    ) -> Result<SnapshotCacheAdmission, SourceAdapterError> {
        if self.limits.max_entries == 0
            || entry.charged_bytes > self.limits.max_snapshot_bytes
            || entry.charged_bytes > self.limits.max_total_bytes
        {
            return Ok(SnapshotCacheAdmission::ResourceLimit);
        }

        if let Some(index) = self.entries.iter().position(|existing| {
            existing.scope == entry.scope
                && existing.binding.source_id == entry.binding.source_id
                && existing.binding.revision == entry.binding.revision
                && existing.binding.target_identity == entry.binding.target_identity
        }) {
            self.remove_at(index)?;
        }

        loop {
            let projected_total = self
                .charged_bytes
                .checked_add(entry.charged_bytes)
                .ok_or_else(|| {
                    source_unavailable("navigation snapshot cache byte accounting overflow")
                })?;
            if self.entries.len() < self.limits.max_entries
                && projected_total <= self.limits.max_total_bytes
            {
                let navigation = std::sync::Arc::clone(&entry.navigation);
                self.charged_bytes = projected_total;
                self.entries.push_back(entry);
                return Ok(SnapshotCacheAdmission::Admitted(navigation));
            }
            if self.entries.is_empty() {
                return Ok(SnapshotCacheAdmission::ResourceLimit);
            }
            self.remove_at(0)?;
        }
    }

    pub(crate) fn navigation(
        &self,
        scope: &str,
        source_id: &SourceId,
        target_identity: &TargetIdentity,
        revision: &SourceRevision,
    ) -> Option<std::sync::Arc<unica_format_core::navigation::NavigationEnvelope>> {
        self.entries
            .iter()
            .find(|entry| {
                entry.scope == scope
                    && entry.binding.source_id == *source_id
                    && entry.binding.target_identity == *target_identity
                    && entry.binding.revision == *revision
            })
            .map(|entry| std::sync::Arc::clone(&entry.navigation))
    }

    pub(crate) fn resolve_target_identity(
        &self,
        scope: &str,
        source_id: &SourceId,
        revision: &SourceRevision,
        object_key: &unica_format_core::navigation::ObjectKey,
    ) -> Result<TargetIdentity, SourceAdapterError> {
        let mut identities = self
            .entries
            .iter()
            .filter(|entry| {
                entry.scope == scope
                    && entry.binding.source_id == *source_id
                    && entry.binding.revision == *revision
                    && entry.navigation.nodes.iter().any(|node| {
                        node.object_ref.source_id == *source_id
                            && node.object_ref.object_key == *object_key
                    })
            })
            .map(|entry| entry.binding.target_identity.clone())
            .collect::<Vec<_>>();
        identities.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        identities.dedup();
        match identities.as_slice() {
            [identity] => Ok(identity.clone()),
            [] => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "continuation target identity is absent from retained snapshots",
            )),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::IdentityCollision,
                "continuation target identity is ambiguous across retained snapshots",
            )),
        }
    }

    pub(crate) fn remove_at(
        &mut self,
        index: usize,
    ) -> Result<CachedNavigation, SourceAdapterError> {
        let entry = self.entries.remove(index).ok_or_else(|| {
            source_unavailable("navigation snapshot cache entry disappeared during eviction")
        })?;
        self.charged_bytes = self
            .charged_bytes
            .checked_sub(entry.charged_bytes)
            .ok_or_else(|| {
                source_unavailable("navigation snapshot cache byte accounting underflow")
            })?;
        Ok(entry)
    }
}

struct BoundedCountingWriter {
    limit: usize,
    bytes: usize,
    exceeded: bool,
}

impl BoundedCountingWriter {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            bytes: 0,
            exceeded: false,
        }
    }
}

impl Write for BoundedCountingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let next = match self.bytes.checked_add(buffer.len()) {
            Some(next) => next,
            None => {
                self.exceeded = true;
                return Err(io::Error::other("serialized navigation size overflow"));
            }
        };
        if next > self.limit {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "serialized navigation exceeds cache limit",
            ));
        }
        self.bytes = next;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl CachedNavigation {
    pub(crate) fn new(
        scope: String,
        binding: SourceBinding,
        navigation: unica_format_core::navigation::NavigationEnvelope,
        limits: SnapshotCacheLimits,
    ) -> Result<Self, SourceAdapterError> {
        validate_cached_navigation_key(&scope, &binding, limits)?;
        validate_navigation_cache_payload(&binding, &navigation, limits)?;
        let charge = CachedNavigationCharge {
            scope: &scope,
            binding: &binding,
            navigation: &navigation,
            relation_index: navigation.relation_index.as_slice(),
        };
        let charged_bytes = serialized_bytes_with_limit(&charge, limits.max_snapshot_bytes)?;
        Ok(Self {
            scope,
            binding,
            navigation: std::sync::Arc::new(navigation),
            charged_bytes,
        })
    }
}

pub(crate) fn serialized_bytes_with_limit<T: Serialize>(
    value: &T,
    limit: usize,
) -> Result<usize, SourceAdapterError> {
    let mut writer = BoundedCountingWriter::new(limit);
    let serialized = serde_json::to_writer(&mut writer, value);
    if writer.exceeded {
        return Err(resource_limit(
            "navigation cache payload exceeds its serialized byte limit",
        ));
    }
    serialized.map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("cannot serialize navigation cache payload: {error}"),
        )
    })?;
    Ok(writer.bytes)
}

pub(crate) fn validate_navigation_cache_payload(
    binding: &SourceBinding,
    navigation: &unica_format_core::navigation::NavigationEnvelope,
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    validate_identity_bearing_navigation(binding, navigation)?;
    let page_item_count = navigation
        .relations
        .iter()
        .try_fold(0usize, |count, page| {
            count.checked_add(page.items.len()).ok_or_else(|| {
                resource_limit("navigation relation pages have too many nodes for continuation")
            })
        })?;
    let node_count = navigation
        .nodes
        .len()
        .checked_add(page_item_count)
        .ok_or_else(|| resource_limit("navigation node count cannot be represented"))?;
    if node_count > limits.max_nodes {
        return Err(resource_limit(
            "navigation snapshot has too many nodes for continuation",
        ));
    }
    let relation_count = navigation
        .relation_index
        .len()
        .checked_add(navigation.relations.len())
        .ok_or_else(|| resource_limit("navigation relation count cannot be represented"))?;
    if relation_count > limits.max_relations {
        return Err(resource_limit(
            "navigation snapshot has too many relations for continuation",
        ));
    }

    validate_semantic_string(&navigation.schema_version, limits.max_semantic_string_bytes)?;
    validate_navigation_status(navigation.status)?;
    if let Some(snapshot) = &navigation.snapshot {
        validate_source_snapshot(snapshot, limits.max_semantic_string_bytes)?;
    }
    if let Some(root) = &navigation.root {
        validate_object_ref(root, limits.max_semantic_string_bytes)?;
    }
    for node in &navigation.nodes {
        validate_navigation_node(node, limits)?;
    }
    for page in &navigation.relations {
        validate_navigation_relation_page(page, limits)?;
    }
    for relation in navigation.relation_index.iter() {
        validate_semantic_relation(relation, limits.max_semantic_string_bytes)?;
    }
    validate_navigation_diagnostics(&navigation.diagnostics, limits)
}

#[cfg(test)]
#[derive(Default)]
struct NavigationValidationStats {
    max_active_depth: usize,
}

#[cfg(test)]
impl NavigationValidationStats {
    fn observe(&mut self, depth: usize) -> Result<(), SourceAdapterError> {
        let active_depth = depth
            .checked_add(1)
            .ok_or_else(|| resource_limit("navigation validation depth cannot be represented"))?;
        self.max_active_depth = self.max_active_depth.max(active_depth);
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn property_value_validation_max_active_depth(
    value: &unica_format_core::navigation::PropertyValue,
    limits: SnapshotCacheLimits,
) -> Result<usize, SourceAdapterError> {
    let mut stats = NavigationValidationStats::default();
    observe_property_value_depth(value, limits, 0, &mut stats)?;
    validate_property_value(value, limits, 0)?;
    Ok(stats.max_active_depth)
}

#[cfg(test)]
fn observe_property_value_depth(
    value: &unica_format_core::navigation::PropertyValue,
    limits: SnapshotCacheLimits,
    depth: usize,
    stats: &mut NavigationValidationStats,
) -> Result<(), SourceAdapterError> {
    use unica_format_core::navigation::PropertyValue;

    stats.observe(depth)?;
    match value {
        PropertyValue::TypeSet(_) => {}
        PropertyValue::List(values) => {
            for value in values {
                observe_property_value_depth_child(value, limits, depth, stats)?;
            }
        }
        PropertyValue::Structure(values) => {
            for value in values.values() {
                observe_property_value_depth_child(value, limits, depth, stats)?;
            }
        }
        PropertyValue::Decimal(_)
        | PropertyValue::String(_)
        | PropertyValue::LocalizedString(_)
        | PropertyValue::EnumSymbol(_)
        | PropertyValue::ObjectRef(_)
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::Uuid(_)
        | PropertyValue::Date(_)
        | PropertyValue::Unknown { .. }
        | PropertyValue::Null => {}
    }
    Ok(())
}

#[cfg(test)]
fn observe_property_value_depth_child(
    value: &unica_format_core::navigation::PropertyValue,
    limits: SnapshotCacheLimits,
    parent_depth: usize,
    stats: &mut NavigationValidationStats,
) -> Result<(), SourceAdapterError> {
    let depth = validation_child_depth(parent_depth, limits)?;
    observe_property_value_depth(value, limits, depth, stats)
}

fn validate_navigation_node(
    node: &NavigationNode,
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    if node.properties.len() > limits.max_properties_per_node {
        return Err(resource_limit(
            "navigation node has too many properties for continuation",
        ));
    }
    validate_object_ref(&node.object_ref, limits.max_semantic_string_bytes)?;
    validate_object_ref(&node.reference, limits.max_semantic_string_bytes)?;
    validate_capability_state(node.capability_state)?;
    validate_capability_vector(&node.capability)?;
    validate_action_profile(node.action_profile)?;
    validate_navigation_facet_visibility(node.facet_visibility)?;
    for (name, property) in &node.properties {
        validate_semantic_string(name.as_str(), limits.max_semantic_string_bytes)?;
        validate_semantic_property(property, limits)?;
    }
    for descriptor in &node.semantic_actions {
        validate_semantic_action_descriptor(descriptor)?;
    }
    for action in &node.actions {
        validate_semantic_action(action, limits.max_semantic_string_bytes)?;
    }
    Ok(())
}

fn validate_semantic_property(
    property: &unica_format_core::navigation::SemanticProperty,
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    validate_property_type(&property.value_type, limits.max_semantic_string_bytes)?;
    validate_property_value_state(property.value_state)?;
    validate_property_provenance(property.provenance)?;
    validate_property_capability(property.capability)?;
    if let Some(value) = &property.value {
        validate_property_value(value, limits, 0)?;
    }
    serialized_bytes_with_limit(property, limits.max_property_bytes)?;
    if let Some(value) = &property.value {
        serialized_bytes_with_limit(value, limits.max_property_value_bytes)?;
    }
    Ok(())
}

fn validate_navigation_relation_page(
    page: &unica_format_core::navigation::NavigationRelationPage,
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    validate_relation_group_ref(&page.relation, limits.max_semantic_string_bytes)?;
    for item in &page.items {
        validate_navigation_node(item, limits)?;
    }
    if let Some(cursor) = &page.next_cursor {
        validate_navigation_cursor(cursor, limits.max_semantic_string_bytes)?;
    }
    Ok(())
}

fn validate_navigation_cursor(
    cursor: &unica_format_core::navigation::NavigationCursor,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_string(cursor.source_id.as_str(), limit)?;
    validate_semantic_string(cursor.snapshot_revision.as_str(), limit)?;
    validate_semantic_string(cursor.target_identity.as_str(), limit)?;
    validate_semantic_string(cursor.target.as_str(), limit)?;
    validate_semantic_string(cursor.relation.as_str(), limit)?;
    validate_relation_role(cursor.relation_role)?;
    validate_relation_kind(cursor.relation_kind)?;
    validate_navigation_selection(&cursor.selection, limit)?;
    validate_semantic_string(&cursor.selection_hash, limit)?;
    if cursor.encoded_len() > unica_format_core::limits::MAX_NAVIGATION_CURSOR_TOKEN_BYTES {
        return Err(resource_limit(
            "navigation cursor token exceeds its encoded byte limit",
        ));
    }
    let _ = cursor.schema_version;
    let _ = cursor.next_position;
    Ok(())
}

fn validate_navigation_selection(
    selection: &unica_format_core::navigation::NavigationSelection,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    match &selection.properties {
        unica_format_core::navigation::PropertySelection::All => {}
        unica_format_core::navigation::PropertySelection::Named(names) => {
            for name in names {
                validate_semantic_string(name.as_str(), limit)?;
            }
        }
    }
    match selection.facets {
        unica_format_core::navigation::FacetSelection::None
        | unica_format_core::navigation::FacetSelection::Summary
        | unica_format_core::navigation::FacetSelection::Full => {}
    }
    for relation in &selection.relations {
        validate_relation_kind(relation.kind)?;
        validate_relation_role(relation.role)?;
        let _ = relation.page_size;
    }
    Ok(())
}

fn validate_navigation_diagnostics(
    diagnostics: &[unica_format_core::navigation::SourceAdapterDiagnostic],
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    for diagnostic in diagnostics {
        validate_source_adapter_diagnostic(diagnostic, limits)?;
    }
    serialized_bytes_with_limit(&diagnostics, limits.max_diagnostics_bytes).map(|_| ())
}

fn validate_source_adapter_diagnostic(
    diagnostic: &unica_format_core::navigation::SourceAdapterDiagnostic,
    limits: SnapshotCacheLimits,
) -> Result<usize, SourceAdapterError> {
    validate_semantic_string(&diagnostic.code, limits.max_semantic_string_bytes)?;
    validate_semantic_string(&diagnostic.message, limits.max_semantic_string_bytes)?;
    if let Some(details) = &diagnostic.details {
        validate_json_value(details, limits, 0)?;
        serialized_bytes_with_limit(details, limits.max_diagnostic_details_bytes)?;
    }
    serialized_bytes_with_limit(diagnostic, limits.max_diagnostics_bytes)
}

fn validate_json_value(
    value: &serde_json::Value,
    limits: SnapshotCacheLimits,
    depth: usize,
) -> Result<(), SourceAdapterError> {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
        serde_json::Value::String(value) => {
            validate_semantic_string(value, limits.max_semantic_string_bytes)?;
        }
        serde_json::Value::Array(values) => {
            for value in values {
                validate_json_value_child(value, limits, depth)?;
            }
        }
        serde_json::Value::Object(values) => {
            for (name, value) in values {
                validate_semantic_string(name, limits.max_semantic_string_bytes)?;
                validate_json_value_child(value, limits, depth)?;
            }
        }
    }
    Ok(())
}

fn validate_json_value_child(
    value: &serde_json::Value,
    limits: SnapshotCacheLimits,
    parent_depth: usize,
) -> Result<(), SourceAdapterError> {
    let depth = validation_child_depth(parent_depth, limits)?;
    validate_json_value(value, limits, depth)
}

fn validate_source_snapshot(
    snapshot: &unica_format_core::source::SourceSnapshot,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_source_id(snapshot.source_id.as_str(), limit)?;
    validate_source_revision(snapshot.revision.as_str(), limit)?;
    validate_cache_metadata_string(&snapshot.adapter_id, "adapter id", limit)?;
    match snapshot.consistency {
        unica_format_core::source::SnapshotConsistency::Consistent
        | unica_format_core::source::SnapshotConsistency::Partial
        | unica_format_core::source::SnapshotConsistency::Changed
        | unica_format_core::source::SnapshotConsistency::Unverifiable => {}
    }
    Ok(())
}

fn validate_object_ref(reference: &ObjectRef, limit: usize) -> Result<(), SourceAdapterError> {
    validate_semantic_string(reference.source_id.as_str(), limit)?;
    validate_semantic_string(reference.object_key.as_str(), limit)?;
    validate_node_kind(&reference.kind, limit)?;
    validate_semantic_string(&reference.display_name, limit)?;
    match reference.identity_strength {
        unica_format_core::navigation::IdentityStrength::Persistent
        | unica_format_core::navigation::IdentityStrength::Derived
        | unica_format_core::navigation::IdentityStrength::SnapshotOnly => {}
    }
    Ok(())
}

fn validate_node_kind(
    kind: &unica_format_core::navigation::NodeKind,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_string(kind.as_str(), limit)
}

fn validate_capability_state(
    state: unica_format_core::navigation::CapabilityState,
) -> Result<(), SourceAdapterError> {
    match state.resolution_state {
        unica_format_core::navigation::ResolutionState::Resolved
        | unica_format_core::navigation::ResolutionState::Unresolved => {}
    }
    validate_authorability(state.authorability)
}

fn validate_capability_vector(
    capability: &unica_format_core::navigation::CapabilityVector,
) -> Result<(), SourceAdapterError> {
    match capability.resolution {
        unica_format_core::navigation::ResolutionState::Resolved
        | unica_format_core::navigation::ResolutionState::Unresolved => {}
    }
    match capability.identity {
        unica_format_core::navigation::IdentityStrength::Persistent
        | unica_format_core::navigation::IdentityStrength::Derived
        | unica_format_core::navigation::IdentityStrength::SnapshotOnly => {}
    }
    match capability.consistency {
        unica_format_core::source::SnapshotConsistency::Consistent
        | unica_format_core::source::SnapshotConsistency::Partial
        | unica_format_core::source::SnapshotConsistency::Changed
        | unica_format_core::source::SnapshotConsistency::Unverifiable => {}
    }
    match capability.coverage {
        unica_format_core::navigation::CoverageState::Complete
        | unica_format_core::navigation::CoverageState::Partial
        | unica_format_core::navigation::CoverageState::Unknown => {}
    }
    match capability.format {
        unica_format_core::navigation::FormatCompatibility::Compatible
        | unica_format_core::navigation::FormatCompatibility::Incompatible
        | unica_format_core::navigation::FormatCompatibility::Unknown => {}
    }
    match capability.source_access {
        unica_format_core::source::SourceAccess::ReadOnly
        | unica_format_core::source::SourceAccess::ReadWrite => {}
    }
    validate_authorability(capability.authorability)
}

fn validate_authorability(
    authorability: unica_format_core::navigation::Authorability,
) -> Result<(), SourceAdapterError> {
    match authorability {
        unica_format_core::navigation::Authorability::Authorable
        | unica_format_core::navigation::Authorability::SupportLocked
        | unica_format_core::navigation::Authorability::ConfigurationReadOnly
        | unica_format_core::navigation::Authorability::UnknownSupportState
        | unica_format_core::navigation::Authorability::UnknownReadOnly
        | unica_format_core::navigation::Authorability::DerivedReadOnly => {}
    }
    Ok(())
}

fn validate_action_profile(
    profile: unica_format_core::navigation::ActionProfile,
) -> Result<(), SourceAdapterError> {
    match profile {
        unica_format_core::navigation::ActionProfile::DocumentMetadataObject
        | unica_format_core::navigation::ActionProfile::GenericMetadataObject
        | unica_format_core::navigation::ActionProfile::Form
        | unica_format_core::navigation::ActionProfile::FormElement
        | unica_format_core::navigation::ActionProfile::TabularSection
        | unica_format_core::navigation::ActionProfile::MxlTemplate
        | unica_format_core::navigation::ActionProfile::UnmodeledTemplate
        | unica_format_core::navigation::ActionProfile::UnmodeledChild => {}
    }
    Ok(())
}

fn validate_navigation_facet_visibility(
    visibility: unica_format_core::navigation::NavigationFacetVisibility,
) -> Result<(), SourceAdapterError> {
    match visibility {
        unica_format_core::navigation::NavigationFacetVisibility::Full
        | unica_format_core::navigation::NavigationFacetVisibility::Summary
        | unica_format_core::navigation::NavigationFacetVisibility::None => {}
    }
    Ok(())
}

fn validate_semantic_action_descriptor(
    descriptor: &unica_format_core::navigation::SemanticActionDescriptor,
) -> Result<(), SourceAdapterError> {
    validate_semantic_action_kind(descriptor.action)?;
    match descriptor.execution_policy {
        unica_format_core::navigation::ActionExecutionPolicy::ReadOnly
        | unica_format_core::navigation::ActionExecutionPolicy::AtomicNodeMutation
        | unica_format_core::navigation::ActionExecutionPolicy::AtomicRelationMutation => {}
    }
    Ok(())
}

fn validate_semantic_action(
    action: &unica_format_core::navigation::SemanticAction,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_action_kind(action.kind)?;
    if let Some(target) = &action.target {
        validate_object_ref(target, limit)?;
    }
    if let Some(owning_relation) = &action.owning_relation {
        validate_relation_ref(owning_relation, limit)?;
    }
    match action.availability {
        unica_format_core::navigation::ActionAvailability::Modeled
        | unica_format_core::navigation::ActionAvailability::Executable
        | unica_format_core::navigation::ActionAvailability::Blocked => {}
    }
    for reason in &action.blocking_reasons {
        match reason {
            unica_format_core::navigation::CapabilityBlockReason::ResolutionUnresolved
            | unica_format_core::navigation::CapabilityBlockReason::IdentitySnapshotOnly
            | unica_format_core::navigation::CapabilityBlockReason::SnapshotInconsistent
            | unica_format_core::navigation::CapabilityBlockReason::CoverageIncomplete
            | unica_format_core::navigation::CapabilityBlockReason::FormatIncompatible
            | unica_format_core::navigation::CapabilityBlockReason::SourceReadOnly
            | unica_format_core::navigation::CapabilityBlockReason::NotAuthorable
            | unica_format_core::navigation::CapabilityBlockReason::OwningRelationMissing
            | unica_format_core::navigation::CapabilityBlockReason::OperationBindingInvalid => {}
        }
    }
    if let Some(binding) = &action.operation_binding {
        validate_semantic_string(&binding.tool, limit)?;
        validate_semantic_string(&binding.schema_version, limit)?;
    }
    match action.atomicity {
        unica_format_core::navigation::Atomicity::SingleFileAtomicReplace
        | unica_format_core::navigation::Atomicity::AggregateSwapWithRecovery
        | unica_format_core::navigation::Atomicity::BackendTransaction
        | unica_format_core::navigation::Atomicity::ReadOnly => {}
    }
    Ok(())
}

fn validate_semantic_action_kind(
    kind: unica_format_core::navigation::SemanticActionKind,
) -> Result<(), SourceAdapterError> {
    match kind {
        unica_format_core::navigation::SemanticActionKind::Inspect
        | unica_format_core::navigation::SemanticActionKind::EditProperties
        | unica_format_core::navigation::SemanticActionKind::Clone
        | unica_format_core::navigation::SemanticActionKind::Remove
        | unica_format_core::navigation::SemanticActionKind::AddAttribute
        | unica_format_core::navigation::SemanticActionKind::AddTabularSection
        | unica_format_core::navigation::SemanticActionKind::AddForm
        | unica_format_core::navigation::SemanticActionKind::AddMxl
        | unica_format_core::navigation::SemanticActionKind::AddCommand
        | unica_format_core::navigation::SemanticActionKind::AddFormAttribute
        | unica_format_core::navigation::SemanticActionKind::AddFormCommand
        | unica_format_core::navigation::SemanticActionKind::AddFormElement
        | unica_format_core::navigation::SemanticActionKind::Move
        | unica_format_core::navigation::SemanticActionKind::BindData
        | unica_format_core::navigation::SemanticActionKind::RebindData
        | unica_format_core::navigation::SemanticActionKind::UnbindData
        | unica_format_core::navigation::SemanticActionKind::BindCommand
        | unica_format_core::navigation::SemanticActionKind::RebindCommand
        | unica_format_core::navigation::SemanticActionKind::UnbindCommand
        | unica_format_core::navigation::SemanticActionKind::CreateHandler
        | unica_format_core::navigation::SemanticActionKind::EditMxl => {}
    }
    Ok(())
}

fn validate_property_type(
    value_type: &unica_format_core::navigation::PropertyType,
    _limit: usize,
) -> Result<(), SourceAdapterError> {
    match value_type {
        unica_format_core::navigation::PropertyType::Boolean
        | unica_format_core::navigation::PropertyType::Integer
        | unica_format_core::navigation::PropertyType::Decimal
        | unica_format_core::navigation::PropertyType::String
        | unica_format_core::navigation::PropertyType::LocalizedString
        | unica_format_core::navigation::PropertyType::Uuid
        | unica_format_core::navigation::PropertyType::Enum
        | unica_format_core::navigation::PropertyType::Date
        | unica_format_core::navigation::PropertyType::TypeSet
        | unica_format_core::navigation::PropertyType::ObjectRef
        | unica_format_core::navigation::PropertyType::List
        | unica_format_core::navigation::PropertyType::Structure
        | unica_format_core::navigation::PropertyType::Null
        | unica_format_core::navigation::PropertyType::Unknown => {}
    }
    Ok(())
}

fn validate_property_value_state(
    state: unica_format_core::navigation::PropertyValueState,
) -> Result<(), SourceAdapterError> {
    match state {
        unica_format_core::navigation::PropertyValueState::Explicit
        | unica_format_core::navigation::PropertyValueState::Defaulted
        | unica_format_core::navigation::PropertyValueState::Inherited
        | unica_format_core::navigation::PropertyValueState::Computed
        | unica_format_core::navigation::PropertyValueState::Absent
        | unica_format_core::navigation::PropertyValueState::Unresolved => {}
    }
    Ok(())
}

fn validate_property_provenance(
    provenance: unica_format_core::navigation::PropertyProvenance,
) -> Result<(), SourceAdapterError> {
    match provenance {
        unica_format_core::navigation::PropertyProvenance::Declared
        | unica_format_core::navigation::PropertyProvenance::Default
        | unica_format_core::navigation::PropertyProvenance::Inherited
        | unica_format_core::navigation::PropertyProvenance::Derived
        | unica_format_core::navigation::PropertyProvenance::Unknown => {}
    }
    Ok(())
}

fn validate_property_capability(
    capability: unica_format_core::navigation::PropertyCapability,
) -> Result<(), SourceAdapterError> {
    match capability {
        unica_format_core::navigation::PropertyCapability::ReadOnly
        | unica_format_core::navigation::PropertyCapability::Authorable
        | unica_format_core::navigation::PropertyCapability::Unavailable
        | unica_format_core::navigation::PropertyCapability::Unknown => {}
    }
    Ok(())
}

fn validate_property_value(
    value: &unica_format_core::navigation::PropertyValue,
    limits: SnapshotCacheLimits,
    depth: usize,
) -> Result<(), SourceAdapterError> {
    use unica_format_core::navigation::{PropertyValue, TypeVariant};

    match value {
        PropertyValue::Decimal(value)
        | PropertyValue::String(value)
        | PropertyValue::Date(value) => {
            validate_semantic_string(value, limits.max_semantic_string_bytes)?;
        }
        PropertyValue::LocalizedString(values) => {
            for (locale, value) in values {
                validate_semantic_string(locale, limits.max_semantic_string_bytes)?;
                validate_semantic_string(value, limits.max_semantic_string_bytes)?;
            }
        }
        PropertyValue::TypeSet(value) => {
            for variant in &value.variants {
                match variant {
                    TypeVariant::Primitive { .. } => {}
                    TypeVariant::Reference { target }
                    | TypeVariant::Enumeration { target }
                    | TypeVariant::DefinedType { target } => {
                        validate_semantic_string(target, limits.max_semantic_string_bytes)?;
                    }
                }
            }
        }
        PropertyValue::ObjectRef(reference) => {
            validate_object_ref(reference, limits.max_semantic_string_bytes)?;
        }
        PropertyValue::List(nested) => {
            for value in nested {
                validate_property_value_child(value, limits, depth)?;
            }
        }
        PropertyValue::Structure(nested) => {
            for (name, value) in nested {
                validate_semantic_string(name, limits.max_semantic_string_bytes)?;
                validate_property_value_child(value, limits, depth)?;
            }
        }
        PropertyValue::Unknown { summary } => {
            validate_semantic_string(summary, limits.max_semantic_string_bytes)?;
        }
        PropertyValue::EnumSymbol(_)
        | PropertyValue::Boolean(_)
        | PropertyValue::Integer(_)
        | PropertyValue::Uuid(_)
        | PropertyValue::Null => {}
    }
    Ok(())
}

fn validate_property_value_child(
    value: &unica_format_core::navigation::PropertyValue,
    limits: SnapshotCacheLimits,
    parent_depth: usize,
) -> Result<(), SourceAdapterError> {
    let depth = validation_child_depth(parent_depth, limits)?;
    validate_property_value(value, limits, depth)
}

fn validation_child_depth(
    parent_depth: usize,
    limits: SnapshotCacheLimits,
) -> Result<usize, SourceAdapterError> {
    let depth = parent_depth
        .checked_add(1)
        .ok_or_else(|| resource_limit("navigation validation depth cannot be represented"))?;
    if depth > limits.max_property_value_depth {
        return Err(resource_limit(
            "navigation value exceeds continuation cache nesting limit",
        ));
    }
    Ok(depth)
}

fn validate_semantic_relation(
    relation: &unica_format_core::navigation::SemanticRelation,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_relation_ref(&relation.relation_ref, limit)?;
    validate_relation_group_ref(&relation.group_ref, limit)?;
    match relation.identity_strength {
        unica_format_core::navigation::IdentityStrength::Persistent
        | unica_format_core::navigation::IdentityStrength::Derived
        | unica_format_core::navigation::IdentityStrength::SnapshotOnly => {}
    }
    validate_relation_kind(relation.kind)?;
    validate_relation_role(relation.role)?;
    validate_object_ref(&relation.source, limit)?;
    validate_object_ref(&relation.target, limit)?;
    validate_capability_vector(&relation.capability)
}

fn validate_relation_ref(
    relation: &unica_format_core::navigation::RelationRef,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_string(relation.source_id.as_str(), limit)?;
    validate_semantic_string(relation.relation_key.as_str(), limit)?;
    validate_relation_kind(relation.kind)
}

fn validate_relation_group_ref(
    relation: &unica_format_core::navigation::RelationGroupRef,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_string(relation.source_id.as_str(), limit)?;
    validate_semantic_string(relation.group_key.as_str(), limit)?;
    validate_object_ref(&relation.owner, limit)?;
    validate_relation_role(relation.role)?;
    validate_relation_kind(relation.kind)
}

fn validate_relation_kind(
    kind: unica_format_core::navigation::RelationKind,
) -> Result<(), SourceAdapterError> {
    match kind {
        unica_format_core::navigation::RelationKind::Contains
        | unica_format_core::navigation::RelationKind::References => {}
    }
    Ok(())
}

fn validate_relation_role(
    role: unica_format_core::navigation::RelationRole,
) -> Result<(), SourceAdapterError> {
    let _ = role.as_str();
    Ok(())
}

fn validate_navigation_status(
    status: unica_format_core::navigation::NavigationStatus,
) -> Result<(), SourceAdapterError> {
    match status {
        unica_format_core::navigation::NavigationStatus::Available
        | unica_format_core::navigation::NavigationStatus::Partial
        | unica_format_core::navigation::NavigationStatus::Unavailable => {}
    }
    Ok(())
}

fn validate_semantic_string(value: &str, limit: usize) -> Result<(), SourceAdapterError> {
    if value.len() > limit {
        return Err(resource_limit(
            "navigation semantic string exceeds continuation cache limit",
        ));
    }
    Ok(())
}

fn validate_cached_navigation_key(
    scope: &str,
    binding: &SourceBinding,
    limits: SnapshotCacheLimits,
) -> Result<(), SourceAdapterError> {
    validate_cache_metadata_string(
        scope,
        "authorization scope",
        limits.max_semantic_string_bytes,
    )?;
    validate_source_id(binding.source_id.as_str(), limits.max_semantic_string_bytes)?;
    validate_source_revision(binding.revision.as_str(), limits.max_semantic_string_bytes)?;
    validate_cache_metadata_string(
        binding.target_identity.as_str(),
        "target identity",
        limits.max_semantic_string_bytes,
    )
}

fn validate_source_id(value: &str, limit: usize) -> Result<(), SourceAdapterError> {
    validate_cache_metadata_string(value, "source id", limit)?;
    SourceId::new(value.to_string())
        .map(|_| ())
        .map_err(|_| resource_limit("navigation cache source id violates its newtype invariant"))
}

fn validate_source_revision(value: &str, limit: usize) -> Result<(), SourceAdapterError> {
    validate_cache_metadata_string(value, "source revision", limit)?;
    unica_format_core::source::SourceRevision::new(value.to_string())
        .map(|_| ())
        .map_err(|_| resource_limit("navigation cache revision violates its newtype invariant"))
}

fn validate_cache_metadata_string(
    value: &str,
    _name: &str,
    limit: usize,
) -> Result<(), SourceAdapterError> {
    validate_semantic_string(value, limit)?;
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(resource_limit("navigation cache metadata is invalid"));
    }
    Ok(())
}

#[cfg(test)]
fn test_cache_binding(snapshot: &unica_format_core::source::SourceSnapshot) -> SourceBinding {
    SourceBinding::new(
        snapshot.source_id.clone(),
        unica_format_core::source::SourceFamily::PlatformXml,
        None,
        unica_format_core::source::TargetIdentity::from_normalized_relative_path(
            "Configuration.xml",
        )
        .unwrap(),
        snapshot.revision.clone(),
    )
}

#[cfg(test)]
fn test_cache_binding_with(
    source_id: SourceId,
    revision: unica_format_core::source::SourceRevision,
) -> SourceBinding {
    SourceBinding::new(
        source_id,
        unica_format_core::source::SourceFamily::PlatformXml,
        None,
        unica_format_core::source::TargetIdentity::from_normalized_relative_path(
            "Configuration.xml",
        )
        .unwrap(),
        revision,
    )
}

const MAX_BINDING_VALIDATION_DEPTH: usize = unica_format_core::limits::MAX_NAVIGATION_NESTING_DEPTH;
/// Shared maximum count of typed navigation fields and containers that can
/// carry source or snapshot identity during validation and cache preflight.
pub(crate) const MAX_IDENTITY_BEARING_VALIDATION_ITEMS: usize =
    unica_format_core::limits::MAX_NAVIGATION_IDENTITY_ITEMS;

#[derive(Default)]
pub(crate) struct IdentityValidationBudget {
    items: usize,
}

impl IdentityValidationBudget {
    pub(crate) fn charge(&mut self, count: usize) -> Result<(), SourceAdapterError> {
        self.items = self.items.checked_add(count).ok_or_else(|| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                "navigation identity-bearing validation item accounting overflow",
            )
        })?;
        if self.items > MAX_IDENTITY_BEARING_VALIDATION_ITEMS {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                format!(
                    "navigation identity-bearing validation item count {} exceeds limit {}",
                    self.items, MAX_IDENTITY_BEARING_VALIDATION_ITEMS
                ),
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_identity_bearing_navigation(
    binding: &SourceBinding,
    envelope: &NavigationEnvelope,
) -> Result<(), SourceAdapterError> {
    BindingValidator::new(binding).validate_envelope(envelope)
}

struct BindingValidator<'a> {
    binding: &'a SourceBinding,
    budget: IdentityValidationBudget,
}

impl<'a> BindingValidator<'a> {
    fn new(binding: &'a SourceBinding) -> Self {
        Self {
            binding,
            budget: IdentityValidationBudget::default(),
        }
    }

    fn validate_envelope(
        mut self,
        envelope: &NavigationEnvelope,
    ) -> Result<(), SourceAdapterError> {
        if let Some(root) = &envelope.root {
            self.validate_object_ref(root)?;
        }
        self.charge(envelope.nodes.len())?;
        for node in &envelope.nodes {
            self.validate_node(node)?;
        }
        self.charge(envelope.relations.len())?;
        for page in &envelope.relations {
            self.validate_group_ref(&page.relation)?;
            self.charge(page.items.len())?;
            for item in &page.items {
                self.validate_node(item)?;
            }
            if let Some(cursor) = &page.next_cursor {
                self.validate_cursor(cursor, &page.relation)?;
            }
        }
        self.charge(envelope.relation_index.len())?;
        for relation in envelope.relation_index.iter() {
            self.validate_relation(relation)?;
        }
        Ok(())
    }

    fn charge(&mut self, count: usize) -> Result<(), SourceAdapterError> {
        self.budget.charge(count)
    }

    fn validate_node(
        &mut self,
        node: &unica_format_core::navigation::NavigationNode,
    ) -> Result<(), SourceAdapterError> {
        self.validate_object_ref(&node.object_ref)?;
        self.validate_object_ref(&node.reference)?;
        self.charge(node.properties.len())?;
        for property in node.properties.values() {
            if let Some(value) = &property.value {
                self.validate_property_value(value, 0)?;
            }
        }
        self.charge(node.actions.len())?;
        for action in &node.actions {
            if let Some(target) = &action.target {
                self.validate_object_ref(target)?;
            }
            if let Some(relation) = &action.owning_relation {
                self.validate_relation_ref(relation)?;
            }
        }
        Ok(())
    }

    fn validate_relation(
        &mut self,
        relation: &unica_format_core::navigation::SemanticRelation,
    ) -> Result<(), SourceAdapterError> {
        self.validate_relation_ref(&relation.relation_ref)?;
        self.validate_group_ref(&relation.group_ref)?;
        self.validate_object_ref(&relation.source)?;
        self.validate_object_ref(&relation.target)
    }

    fn validate_group_ref(
        &self,
        relation: &unica_format_core::navigation::RelationGroupRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&relation.source_id)?;
        self.validate_object_ref(&relation.owner)
    }

    fn validate_relation_ref(
        &self,
        relation: &unica_format_core::navigation::RelationRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&relation.source_id)
    }

    fn validate_object_ref(
        &self,
        reference: &unica_format_core::navigation::ObjectRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&reference.source_id)
    }

    fn validate_source_id(
        &self,
        source_id: &unica_format_core::source::SourceId,
    ) -> Result<(), SourceAdapterError> {
        if source_id != &self.binding.source_id {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "ready navigation contains a foreign source reference",
            ));
        }
        Ok(())
    }

    fn validate_cursor(
        &self,
        cursor: &unica_format_core::navigation::NavigationCursor,
        group: &unica_format_core::navigation::RelationGroupRef,
    ) -> Result<(), SourceAdapterError> {
        self.validate_source_id(&cursor.source_id)?;
        if cursor.target_identity != self.binding.target_identity {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "ready navigation cursor belongs to another captured target",
            ));
        }
        if cursor.snapshot_revision != self.binding.revision {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "ready navigation contains a cursor for another snapshot revision",
            ));
        }
        if cursor.target != group.owner.object_key
            || cursor.relation != group.group_key
            || cursor.relation_role != group.role
            || cursor.relation_kind != group.kind
        {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotInconsistent,
                "ready navigation cursor does not belong to its relation group",
            ));
        }
        Ok(())
    }

    fn validate_property_value(
        &mut self,
        value: &unica_format_core::navigation::PropertyValue,
        depth: usize,
    ) -> Result<(), SourceAdapterError> {
        if depth > MAX_BINDING_VALIDATION_DEPTH {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ResourceLimit,
                "navigation binding validation exceeds the property nesting limit",
            ));
        }
        use unica_format_core::navigation::{PropertyValue, TypeVariant};
        match value {
            PropertyValue::ObjectRef(reference) => self.validate_object_ref(reference),
            PropertyValue::List(values) => {
                self.charge(values.len())?;
                for value in values {
                    self.validate_property_value(value, depth + 1)?;
                }
                Ok(())
            }
            PropertyValue::Structure(values) => {
                self.charge(values.len())?;
                for value in values.values() {
                    self.validate_property_value(value, depth + 1)?;
                }
                Ok(())
            }
            PropertyValue::TypeSet(types) => {
                self.charge(types.variants.len())?;
                for variant in &types.variants {
                    if let TypeVariant::Primitive { qualifiers, .. } = variant {
                        self.charge(usize::from(qualifiers.is_some()))?;
                    }
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, sync::Arc};
    use unica_format_core::{
        navigation::{
            ActionAvailability, Atomicity, Authorability, CapabilityState, IdentityStrength,
            NavigationRelationPage, NavigationStatus, NodeKind, ObjectKey, ObjectRef,
            OperationBinding, PropertyCapability, PropertyProvenance, PropertyType, PropertyValue,
            PropertyValueState, RelationGroupRef, RelationKind, RelationRole, ResolutionState,
            SemanticAction, SemanticProperty, SemanticPropertyId, SourceAdapterDiagnostic,
        },
        source::{SnapshotConsistency, SourceRevision, SourceSnapshot},
    };

    fn fixture() -> (SourceBinding, NavigationEnvelope) {
        let source_id = SourceId::new("workspace:main").unwrap();
        let snapshot = SourceSnapshot {
            source_id: source_id.clone(),
            revision: SourceRevision::new("sha256:fixture").unwrap(),
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "fake-erased-adapter".to_string(),
        };
        let reference = ObjectRef::new(
            source_id,
            ObjectKey::new("uuid:items").unwrap(),
            IdentityStrength::Persistent,
            NodeKind::Catalog,
            "Items",
        );
        let mut node = NavigationNode::new(
            reference.clone(),
            CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
        );
        node.properties
            .insert(SemanticPropertyId::METADATA_NAME, string_property("Items"));
        (
            test_cache_binding(&snapshot),
            NavigationEnvelope {
                schema_version: "1".to_string(),
                status: NavigationStatus::Available,
                snapshot: Some(snapshot),
                root: Some(reference),
                nodes: vec![node],
                relations: Vec::new(),
                diagnostics: Vec::new(),
                relation_index: Arc::new(Vec::new()),
            },
        )
    }

    fn string_property(value: &str) -> SemanticProperty {
        SemanticProperty {
            value_type: PropertyType::String,
            value_state: PropertyValueState::Explicit,
            value: Some(PropertyValue::String(value.to_string())),
            provenance: PropertyProvenance::Declared,
            capability: PropertyCapability::ReadOnly,
        }
    }

    fn first_property(navigation: &mut NavigationEnvelope) -> &mut SemanticProperty {
        navigation.nodes[0].properties.values_mut().next().unwrap()
    }

    #[test]
    fn resource_limited_snapshot_is_public_unavailable_without_references_or_cursor() {
        let (binding, navigation) = fixture();
        let limits = SnapshotCacheLimits {
            max_snapshot_bytes: 1,
            max_total_bytes: 1,
            ..DEFAULT_SNAPSHOT_CACHE_LIMITS
        };
        let entry =
            CachedNavigation::new("resource-limited".to_string(), binding, navigation, limits);
        assert_eq!(
            entry.unwrap_err().kind,
            SourceAdapterErrorKind::ResourceLimit
        );
        let unavailable = NavigationEnvelope::unavailable(resource_limit(
            "navigation snapshot exceeds continuation cache limits",
        ));
        assert_eq!(unavailable.diagnostics[0].code, "resource_limit");
        assert!(unavailable.snapshot.is_none());
        assert!(unavailable.nodes.is_empty());
    }

    #[test]
    fn cache_preflight_accepts_a_twenty_five_thousand_node_identity_graph() {
        let (binding, mut navigation) = fixture();
        let mut node = navigation.nodes[0].clone();
        node.actions.clear();
        node.semantic_actions.clear();
        node.properties.insert(
            SemanticPropertyId::METADATA_COMMENT,
            string_property("value"),
        );
        navigation.nodes = (0..25_000).map(|_| node.clone()).collect();
        let cached = CachedNavigation::new(
            "ordinary-identity-graph".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap();
        assert!(cached.charged_bytes <= SNAPSHOT_CACHE_MAX_SNAPSHOT_BYTES);
    }

    #[test]
    fn cache_preflight_rejects_the_shared_identity_item_limit() {
        let (binding, mut navigation) = fixture();
        navigation.nodes[0].properties = BTreeMap::from([(
            SemanticPropertyId::FIELD_FILL_VALUE,
            SemanticProperty {
                value_type: PropertyType::List,
                value_state: PropertyValueState::Explicit,
                value: Some(PropertyValue::List(vec![
                    PropertyValue::Null;
                    MAX_IDENTITY_BEARING_VALIDATION_ITEMS
                        + 1
                ])),
                provenance: PropertyProvenance::Declared,
                capability: PropertyCapability::ReadOnly,
            },
        )]);
        let error = CachedNavigation::new(
            "over-limit-identity-graph".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
        assert!(error
            .message
            .contains("identity-bearing validation item count"));
    }

    #[test]
    fn oversized_low_node_property_value_is_public_unavailable() {
        let (binding, navigation) = fixture();
        let limits = SnapshotCacheLimits {
            max_property_value_bytes: 1,
            ..DEFAULT_SNAPSHOT_CACHE_LIMITS
        };
        let error =
            CachedNavigation::new("property-limit".to_string(), binding, navigation, limits)
                .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn property_value_depth_limit_returns_resource_limit_without_recursion() {
        let (binding, mut navigation) = fixture();
        let mut value = PropertyValue::String("leaf".to_string());
        for _ in 0..=SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_DEPTH {
            value = PropertyValue::List(vec![value]);
        }
        first_property(&mut navigation).value = Some(value);
        let error = CachedNavigation::new(
            "depth-limit".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn broad_shallow_property_value_is_cacheable_without_width_proportional_validation_stack() {
        let (binding, mut navigation) = fixture();
        let shallow = PropertyValue::List(vec![PropertyValue::Null; 100_000]);
        assert_eq!(
            property_value_validation_max_active_depth(&shallow, DEFAULT_SNAPSHOT_CACHE_LIMITS)
                .unwrap(),
            2,
        );
        first_property(&mut navigation).value = Some(shallow);
        let entry = CachedNavigation::new(
            "broad-shallow".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap();
        assert!(matches!(
            SnapshotCache::default().admit(entry).unwrap(),
            SnapshotCacheAdmission::Admitted(_)
        ));
    }

    #[test]
    fn aggregate_source_snapshot_over_semantic_string_limit_is_cacheable() {
        let (_, mut navigation) = fixture();
        let large = "x".repeat(300 * 1024);
        let snapshot = navigation.snapshot.as_mut().unwrap();
        snapshot.adapter_id = large.clone();
        snapshot.revision = SourceRevision::new(large).unwrap();
        let binding = test_cache_binding(snapshot);
        let entry = CachedNavigation::new(
            "aggregate-snapshot".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap();
        assert!(entry.charged_bytes > SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES);
    }

    #[test]
    fn structured_property_larger_than_semantic_string_limit_is_cacheable() {
        let (binding, mut navigation) = fixture();
        let values = (0..600)
            .map(|index| {
                (
                    format!("field-{index:04}"),
                    PropertyValue::String("x".repeat(1024)),
                )
            })
            .collect();
        first_property(&mut navigation).value = Some(PropertyValue::Structure(values));
        let entry = CachedNavigation::new(
            "structured-value".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap();
        assert!(entry.charged_bytes > SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES);
    }

    #[test]
    fn individual_semantic_string_larger_than_limit_is_not_cacheable() {
        let (binding, mut navigation) = fixture();
        first_property(&mut navigation).value = Some(PropertyValue::String(
            "x".repeat(SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES + 1),
        ));
        let error = CachedNavigation::new(
            "semantic-string".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn relation_page_only_depth_limit_returns_resource_limit_without_serializing_page_items() {
        let (binding, mut navigation) = fixture();
        let mut item = navigation.nodes[0].clone();
        let mut value = PropertyValue::String("leaf".to_string());
        for _ in 0..=SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_DEPTH {
            value = PropertyValue::List(vec![value]);
        }
        item.properties.values_mut().next().unwrap().value = Some(value);
        let owner = navigation.root.clone().unwrap();
        navigation.relations.push(NavigationRelationPage {
            relation: RelationGroupRef::new(
                owner.source_id.clone(),
                owner,
                RelationRole::Attributes,
                RelationKind::Contains,
            )
            .unwrap(),
            items: vec![item],
            next_cursor: None,
        });
        let error = CachedNavigation::new(
            "relation-page-depth".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn oversized_action_operation_binding_returns_resource_limit() {
        let (binding, mut navigation) = fixture();
        navigation.nodes[0].actions.push(SemanticAction {
            kind: unica_format_core::navigation::SemanticActionKind::Inspect,
            target: None,
            owning_relation: None,
            availability: ActionAvailability::Modeled,
            blocking_reasons: Vec::new(),
            operation_binding: Some(OperationBinding {
                tool: "x".repeat(SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES + 1),
                schema_version: "1".to_string(),
            }),
            atomicity: Atomicity::ReadOnly,
        });
        let error = CachedNavigation::new(
            "action-binding".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn oversized_diagnostic_detail_returns_resource_limit() {
        let (binding, mut navigation) = fixture();
        navigation.diagnostics = vec![SourceAdapterDiagnostic {
            code: "diagnostic".to_string(),
            message: "detail".to_string(),
            details: Some(serde_json::json!({
                "detail": "x".repeat(SNAPSHOT_CACHE_MAX_DIAGNOSTIC_DETAILS_BYTES + 1)
            })),
        }];
        let error = CachedNavigation::new(
            "diagnostic-detail".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn aggregate_diagnostics_over_total_limit_return_resource_limit() {
        let (binding, mut navigation) = fixture();
        let detail = "x".repeat(SNAPSHOT_CACHE_MAX_DIAGNOSTIC_DETAILS_BYTES - 64);
        navigation.diagnostics = (0..5)
            .map(|index| SourceAdapterDiagnostic {
                code: format!("diagnostic-{index}"),
                message: "aggregate".to_string(),
                details: Some(serde_json::json!({"detail": detail})),
            })
            .collect();
        let error = CachedNavigation::new(
            "diagnostic-total".to_string(),
            binding,
            navigation,
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn oversized_cached_navigation_outer_metadata_returns_resource_limit() {
        let (binding, navigation) = fixture();
        let oversized = "x".repeat(SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES + 1);
        assert_eq!(
            CachedNavigation::new(
                oversized,
                binding.clone(),
                navigation.clone(),
                DEFAULT_SNAPSHOT_CACHE_LIMITS,
            )
            .unwrap_err()
            .kind,
            SourceAdapterErrorKind::ResourceLimit,
        );
        let revision =
            SourceRevision::new("x".repeat(SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES + 1)).unwrap();
        assert_eq!(
            CachedNavigation::new(
                "outer-revision".to_string(),
                test_cache_binding_with(binding.source_id, revision),
                navigation,
                DEFAULT_SNAPSHOT_CACHE_LIMITS,
            )
            .unwrap_err()
            .kind,
            SourceAdapterErrorKind::ResourceLimit,
        );
    }

    #[test]
    fn snapshot_cache_evicts_fifo_by_exact_byte_charge_and_evicted_cursor_fails_closed() {
        let (binding, navigation) = fixture();
        let probe = CachedNavigation::new(
            "scope-0".to_string(),
            binding.clone(),
            navigation.clone(),
            DEFAULT_SNAPSHOT_CACHE_LIMITS,
        )
        .unwrap();
        let entry_bytes = probe.charged_bytes;
        let mut cache = SnapshotCache::with_limits(SnapshotCacheLimits {
            max_entries: 3,
            max_snapshot_bytes: entry_bytes,
            max_total_bytes: entry_bytes * 2,
            ..DEFAULT_SNAPSHOT_CACHE_LIMITS
        });
        for index in 0..8 {
            cache
                .admit(
                    CachedNavigation::new(
                        format!("scope-{index}"),
                        binding.clone(),
                        navigation.clone(),
                        cache.limits,
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(
                cache.charged_bytes,
                cache
                    .entries
                    .iter()
                    .map(|entry| entry.charged_bytes)
                    .sum::<usize>()
            );
        }
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.entries.front().unwrap().scope, "scope-6");
        assert_eq!(cache.entries.back().unwrap().scope, "scope-7");
        assert!(cache
            .navigation(
                "scope-0",
                &binding.source_id,
                &binding.target_identity,
                &binding.revision,
            )
            .is_none());
    }

    #[test]
    fn lookup_fails_closed_when_targets_share_scope_source_and_revision() {
        let (first_binding, first_navigation) = fixture();
        let mut second_binding = first_binding.clone();
        second_binding.target_identity =
            unica_format_core::source::TargetIdentity::from_normalized_relative_path(
                "Catalogs/Other.xml",
            )
            .unwrap();
        let mut second_navigation = first_navigation.clone();
        second_navigation.root.as_mut().unwrap().display_name = "Other".to_string();
        second_navigation.nodes[0].object_ref.display_name = "Other".to_string();
        second_navigation.nodes[0].reference.display_name = "Other".to_string();
        let mut cache = SnapshotCache::default();
        let second_identity = second_binding.target_identity.clone();
        for (binding, navigation) in [
            (first_binding.clone(), first_navigation),
            (second_binding, second_navigation),
        ] {
            cache
                .admit(
                    CachedNavigation::new(
                        "shared-scope".to_string(),
                        binding,
                        navigation,
                        cache.limits,
                    )
                    .unwrap(),
                )
                .unwrap();
        }

        let first = cache
            .navigation(
                "shared-scope",
                &first_binding.source_id,
                &first_binding.target_identity,
                &first_binding.revision,
            )
            .unwrap();
        let second = cache
            .navigation(
                "shared-scope",
                &first_binding.source_id,
                &second_identity,
                &first_binding.revision,
            )
            .unwrap();
        assert_ne!(
            first.root.as_ref().unwrap().display_name,
            second.root.as_ref().unwrap().display_name,
            "target-aware lookups must not cross-hit"
        );
    }
}
