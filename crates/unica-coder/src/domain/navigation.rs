//! JSON navigation contracts for semantic 1C metadata projections.

use std::collections::{BTreeMap, BTreeSet};

use serde::{ser::SerializeStruct, Serialize, Serializer};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::source_adapters::{
    SnapshotConsistency, SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceId,
    SourceRevision, SourceSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ObjectKey(String);

impl ObjectKey {
    pub(crate) fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_opaque_key(&raw, "object key")?;
        Ok(Self(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for ObjectKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct RelationKey(String);

impl RelationKey {
    pub(crate) fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_opaque_key(&raw, "relation key")?;
        Ok(Self(raw))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for RelationKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

fn validate_opaque_key(raw: &str, name: &str) -> Result<(), SourceAdapterError> {
    let windows_drive = raw.len() >= 2
        && raw.as_bytes()[0].is_ascii_alphabetic()
        && raw.as_bytes()[1] == b':';
    if raw.is_empty()
        || raw.chars().any(char::is_control)
        || raw.starts_with('/')
        || raw.starts_with(r"\\")
        || windows_drive
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("invalid opaque {name}"),
        ));
    }
    Ok(())
}

/// Stability of a semantic key supplied by an adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum IdentityStrength {
    Persistent,
    Derived,
    SnapshotOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObjectIdentity {
    pub(crate) source_id: SourceId,
    pub(crate) object_key: ObjectKey,
}

/// A path-free, versioned semantic object reference.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObjectRef {
    pub(crate) source_id: SourceId,
    pub(crate) object_key: ObjectKey,
    pub(crate) identity_strength: IdentityStrength,
    pub(crate) kind: NodeKind,
    pub(crate) display_name: String,
}

impl ObjectRef {
    pub(crate) fn new(
        source_id: SourceId,
        object_key: ObjectKey,
        identity_strength: IdentityStrength,
        kind: NodeKind,
        display_name: impl Into<String>,
    ) -> Self {
        Self {
            source_id,
            object_key,
            identity_strength,
            kind,
            display_name: display_name.into(),
        }
    }

    pub(crate) fn identity(&self) -> ObjectIdentity {
        ObjectIdentity {
            source_id: self.source_id.clone(),
            object_key: self.object_key.clone(),
        }
    }
}

impl PartialEq for ObjectRef {
    fn eq(&self, other: &Self) -> bool {
        self.identity() == other.identity()
    }
}

impl Eq for ObjectRef {}

/// Semantic class of a graph node. Source representation is intentionally not
/// encoded here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", rename_all_fields = "camelCase")]
pub(crate) enum NodeKind {
    SourceRoot,
    Document,
    MetadataObject { metadata_type: String },
    Attribute,
    TabularSection,
    Command,
    Form,
    FormAttribute,
    FormCommand,
    FormElement,
    Template {
        #[serde(skip_serializing_if = "Option::is_none")]
        template_type: Option<String>,
    },
}

impl NodeKind {
    pub(crate) fn metadata_object(metadata_type: impl Into<String>) -> Self {
        Self::MetadataObject {
            metadata_type: metadata_type.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Representation {
    PlatformXml,
    Edt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ResolutionState {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Authorability {
    Authorable,
    SupportLocked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    DerivedReadOnly,
}

/// Compatibility state of the source format with the selected adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FormatCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CoverageState {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CapabilityBlockReason {
    ResolutionUnresolved,
    IdentitySnapshotOnly,
    SnapshotInconsistent,
    CoverageIncomplete,
    FormatIncompatible,
    SourceReadOnly,
    NotAuthorable,
    OwningRelationMissing,
    OperationBindingInvalid,
}

/// All preconditions that must hold before a semantic mutation can execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityVector {
    pub(crate) resolution: ResolutionState,
    pub(crate) identity: IdentityStrength,
    pub(crate) consistency: SnapshotConsistency,
    pub(crate) coverage: CoverageState,
    pub(crate) format: FormatCompatibility,
    pub(crate) source_access: SourceAccess,
    pub(crate) authorability: Authorability,
}

impl CapabilityVector {
    pub(crate) const fn permits_mutation(&self) -> bool {
        matches!(self.resolution, ResolutionState::Resolved)
            && !matches!(self.identity, IdentityStrength::SnapshotOnly)
            && matches!(self.consistency, SnapshotConsistency::Consistent)
            && matches!(self.coverage, CoverageState::Complete)
            && matches!(self.format, FormatCompatibility::Compatible)
            && matches!(self.source_access, SourceAccess::ReadWrite)
            && matches!(self.authorability, Authorability::Authorable)
    }

    pub(crate) fn blocking_reasons(&self) -> Vec<CapabilityBlockReason> {
        let mut reasons = Vec::new();
        if !matches!(self.resolution, ResolutionState::Resolved) {
            reasons.push(CapabilityBlockReason::ResolutionUnresolved);
        }
        if matches!(self.identity, IdentityStrength::SnapshotOnly) {
            reasons.push(CapabilityBlockReason::IdentitySnapshotOnly);
        }
        if !matches!(self.consistency, SnapshotConsistency::Consistent) {
            reasons.push(CapabilityBlockReason::SnapshotInconsistent);
        }
        if !matches!(self.coverage, CoverageState::Complete) {
            reasons.push(CapabilityBlockReason::CoverageIncomplete);
        }
        if !matches!(self.format, FormatCompatibility::Compatible) {
            reasons.push(CapabilityBlockReason::FormatIncompatible);
        }
        if !matches!(self.source_access, SourceAccess::ReadWrite) {
            reasons.push(CapabilityBlockReason::SourceReadOnly);
        }
        if !matches!(self.authorability, Authorability::Authorable) {
            reasons.push(CapabilityBlockReason::NotAuthorable);
        }
        reasons
    }
}

/// Compatibility view retained for the Task 8 projection migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CapabilityState {
    pub(crate) resolution_state: ResolutionState,
    pub(crate) authorability: Authorability,
}

impl CapabilityState {
    pub(crate) const fn new(resolution_state: ResolutionState, authorability: Authorability) -> Self {
        Self { resolution_state, authorability }
    }

    pub(crate) const fn resolved_authorable() -> Self {
        Self::new(ResolutionState::Resolved, Authorability::Authorable)
    }

    pub(crate) const fn is_resolved_authorable(self) -> bool {
        matches!(self.resolution_state, ResolutionState::Resolved)
            && matches!(self.authorability, Authorability::Authorable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RelationKind {
    Contains,
    References,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationRef {
    pub(crate) source_id: SourceId,
    pub(crate) relation_key: RelationKey,
    pub(crate) kind: RelationKind,
}

impl RelationRef {
    pub(crate) fn new(
        source_id: SourceId,
        relation_key: impl Into<String>,
        kind: RelationKind,
    ) -> Result<Self, SourceAdapterError> {
        Ok(Self { source_id, relation_key: RelationKey::new(relation_key)?, kind })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionAvailability {
    Modeled,
    Executable,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Atomicity {
    SingleFileAtomicReplace,
    AggregateSwapWithRecovery,
    BackendTransaction,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OperationBinding {
    pub(crate) tool: String,
    pub(crate) schema_version: String,
}

impl OperationBinding {
    pub(crate) fn is_valid(&self) -> bool {
        self.tool.starts_with("unica.")
            && self.tool.len() > "unica.".len()
            && !self.tool.chars().any(char::is_control)
            && !self.schema_version.is_empty()
            && !self.schema_version.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SemanticActionKind {
    Inspect,
    EditProperties,
    Clone,
    Remove,
    AddAttribute,
    AddTabularSection,
    AddForm,
    AddMxl,
    AddCommand,
    AddFormAttribute,
    AddFormCommand,
    AddFormElement,
    Move,
    BindData,
    RebindData,
    UnbindData,
    BindCommand,
    RebindCommand,
    UnbindCommand,
    CreateHandler,
    EditMxl,
}

/// A capability-qualified semantic action, independent from a particular MCP
/// transport. Mutation actions are only executable with every capability
/// precondition and an explicit native operation binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticAction {
    pub(crate) kind: SemanticActionKind,
    pub(crate) target: Option<ObjectRef>,
    pub(crate) owning_relation: Option<RelationRef>,
    pub(crate) availability: ActionAvailability,
    pub(crate) blocking_reasons: Vec<CapabilityBlockReason>,
    pub(crate) operation_binding: Option<OperationBinding>,
    pub(crate) atomicity: Atomicity,
}

impl SemanticAction {
    pub(crate) fn modeled_clone(target: ObjectRef, owning_relation: Option<RelationRef>) -> Self {
        let mut blocking_reasons = Vec::new();
        if !owning_relation.as_ref().is_some_and(|relation| {
            relation.source_id == target.source_id && matches!(relation.kind, RelationKind::Contains)
        }) {
            blocking_reasons.push(CapabilityBlockReason::OwningRelationMissing);
        }
        Self {
            kind: SemanticActionKind::Clone,
            target: Some(target),
            owning_relation,
            availability: if blocking_reasons.is_empty() {
                ActionAvailability::Modeled
            } else {
                ActionAvailability::Blocked
            },
            blocking_reasons,
            operation_binding: None,
            atomicity: Atomicity::AggregateSwapWithRecovery,
        }
    }

    pub(crate) fn mutation(
        kind: SemanticActionKind,
        target: ObjectRef,
        capability: CapabilityVector,
        owning_relation: Option<RelationRef>,
        operation_binding: Option<OperationBinding>,
        atomicity: Atomicity,
    ) -> Self {
        let mut blocking_reasons = capability.blocking_reasons();
        if matches!(kind, SemanticActionKind::Clone) && !owning_relation.as_ref().is_some_and(|relation| {
            relation.source_id == target.source_id && matches!(relation.kind, RelationKind::Contains)
        }) {
            blocking_reasons.push(CapabilityBlockReason::OwningRelationMissing);
        }
        if operation_binding.as_ref().is_some_and(|binding| !binding.is_valid()) {
            blocking_reasons.push(CapabilityBlockReason::OperationBindingInvalid);
        }
        let availability = if blocking_reasons.is_empty() {
            if operation_binding.is_some() {
                ActionAvailability::Executable
            } else {
                ActionAvailability::Modeled
            }
        } else {
            ActionAvailability::Blocked
        };
        Self { kind, target: Some(target), owning_relation, availability, blocking_reasons, operation_binding, atomicity }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionExecutionPolicy {
    ReadOnly,
    AtomicNodeMutation,
    AtomicRelationMutation,
}

impl ActionExecutionPolicy {
    pub(crate) const fn is_mutation(self) -> bool {
        matches!(self, Self::AtomicNodeMutation | Self::AtomicRelationMutation)
    }

    pub(crate) const fn validates_before_commit(self) -> bool { self.is_mutation() }
    pub(crate) const fn allows_cross_action_changeset(self) -> bool { false }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticActionDescriptor {
    pub(crate) action: SemanticActionKind,
    pub(crate) execution_policy: ActionExecutionPolicy,
}

impl SemanticActionDescriptor {
    const fn read(action: SemanticActionKind) -> Self {
        Self { action, execution_policy: ActionExecutionPolicy::ReadOnly }
    }
    const fn node_mutation(action: SemanticActionKind) -> Self {
        Self { action, execution_policy: ActionExecutionPolicy::AtomicNodeMutation }
    }
    const fn relation_mutation(action: SemanticActionKind) -> Self {
        Self { action, execution_policy: ActionExecutionPolicy::AtomicRelationMutation }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionProfile {
    DocumentMetadataObject,
    GenericMetadataObject,
    Form,
    FormElement,
    TabularSection,
    MxlTemplate,
    UnmodeledTemplate,
    UnmodeledChild,
}

pub(crate) fn action_profile_for(kind: &NodeKind) -> ActionProfile {
    match kind {
        NodeKind::Document => ActionProfile::DocumentMetadataObject,
        NodeKind::MetadataObject { metadata_type } if metadata_type == "Document" => ActionProfile::DocumentMetadataObject,
        NodeKind::MetadataObject { .. } | NodeKind::SourceRoot => ActionProfile::GenericMetadataObject,
        NodeKind::Form => ActionProfile::Form,
        NodeKind::FormElement => ActionProfile::FormElement,
        NodeKind::TabularSection => ActionProfile::TabularSection,
        NodeKind::Template { template_type: Some(template_type) } if template_type == "SpreadsheetDocument" => ActionProfile::MxlTemplate,
        NodeKind::Template { .. } => ActionProfile::UnmodeledTemplate,
        NodeKind::Attribute | NodeKind::Command | NodeKind::FormAttribute | NodeKind::FormCommand => ActionProfile::UnmodeledChild,
    }
}

pub(crate) fn semantic_actions_for(kind: &NodeKind, capability_state: CapabilityState) -> Vec<SemanticActionDescriptor> {
    if !capability_state.is_resolved_authorable() {
        return vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)];
    }
    match action_profile_for(kind) {
        ActionProfile::DocumentMetadataObject => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Clone),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddAttribute),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddTabularSection),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddForm),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddMxl),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddCommand),
        ],
        ActionProfile::Form => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Clone),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddFormAttribute),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddFormCommand),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddFormElement),
        ],
        ActionProfile::FormElement => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Clone),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddFormElement),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::CreateHandler),
        ],
        ActionProfile::TabularSection => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Clone),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::AddAttribute),
        ],
        ActionProfile::MxlTemplate => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticActionKind::EditMxl),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Clone),
        ],
        ActionProfile::GenericMetadataObject | ActionProfile::UnmodeledTemplate | ActionProfile::UnmodeledChild => vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)],
    }
}

pub(crate) fn semantic_actions_for_relation(
    from_kind: &NodeKind,
    to_kind: &NodeKind,
    relation: RelationKind,
    capability_state: CapabilityState,
) -> Vec<SemanticActionDescriptor> {
    if !capability_state.is_resolved_authorable() {
        return vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)];
    }
    match (relation, from_kind, to_kind) {
        (RelationKind::Contains, NodeKind::Form, NodeKind::FormElement) => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::Move),
        ],
        (RelationKind::References, NodeKind::FormElement, NodeKind::Attribute | NodeKind::FormAttribute) => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::BindData),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::RebindData),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::UnbindData),
        ],
        (RelationKind::References, NodeKind::FormElement, NodeKind::Command | NodeKind::FormCommand) => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::BindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::RebindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::UnbindCommand),
        ],
        _ => vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)],
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationNode {
    pub(crate) reference: ObjectRef,
    pub(crate) capability_state: CapabilityState,
    pub(crate) action_profile: ActionProfile,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

impl NavigationNode {
    pub(crate) fn new(reference: ObjectRef, capability_state: CapabilityState) -> Self {
        let action_profile = action_profile_for(&reference.kind);
        let semantic_actions = semantic_actions_for(&reference.kind, capability_state);
        Self { reference, capability_state, action_profile, semantic_actions }
    }
    pub(crate) fn semantic_actions(&self) -> &[SemanticActionDescriptor] { &self.semantic_actions }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationEdge {
    pub(crate) from: ObjectRef,
    pub(crate) to: ObjectRef,
    pub(crate) relation: RelationKind,
    pub(crate) capability_state: CapabilityState,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

impl NavigationEdge {
    pub(crate) fn new(from: ObjectRef, to: ObjectRef, relation: RelationKind, capability_state: CapabilityState) -> Self {
        let semantic_actions = semantic_actions_for_relation(&from.kind, &to.kind, relation, capability_state);
        Self { from, to, relation, capability_state, semantic_actions }
    }
    pub(crate) fn semantic_actions(&self) -> &[SemanticActionDescriptor] { &self.semantic_actions }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ActionSemantics { ModeledCapabilities }

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavigationGraph {
    pub(crate) prototype_version: u32,
    prototype: bool,
    action_semantics: ActionSemantics,
    pub(crate) representation: Representation,
    pub(crate) root: ObjectRef,
    pub(crate) nodes: Vec<NavigationNode>,
    pub(crate) edges: Vec<NavigationEdge>,
}

impl NavigationGraph {
    pub(crate) const PROTOTYPE_VERSION: u32 = 1;
    pub(crate) fn new(representation: Representation, root: ObjectRef, nodes: Vec<NavigationNode>, edges: Vec<NavigationEdge>) -> Self {
        Self { prototype_version: Self::PROTOTYPE_VERSION, prototype: true, action_semantics: ActionSemantics::ModeledCapabilities, representation, root, nodes, edges }
    }
    pub(crate) const fn is_prototype(&self) -> bool { self.prototype }
    pub(crate) const fn action_semantics(&self) -> ActionSemantics { self.action_semantics }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyGraphObjectRef {
    source_set: String,
    owner_chain: Vec<LegacyOwnerSegment>,
    kind: NodeKind,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyOwnerSegment {
    kind: NodeKind,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyNavigationNode {
    reference: LegacyGraphObjectRef,
    capability_state: CapabilityState,
    action_profile: ActionProfile,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LegacyNavigationEdge {
    from: LegacyGraphObjectRef,
    to: LegacyGraphObjectRef,
    relation: RelationKind,
    capability_state: CapabilityState,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

impl NavigationGraph {
    fn legacy_reference(&self, reference: &ObjectRef) -> LegacyGraphObjectRef {
        let mut owner_chain = Vec::new();
        let mut current = reference;
        while let Some(parent) = self.edges.iter().find_map(|edge| {
            (matches!(edge.relation, RelationKind::Contains) && edge.to == *current)
                .then_some(&edge.from)
        }) {
            owner_chain.push(LegacyOwnerSegment {
                kind: parent.kind.clone(),
                name: parent.display_name.clone(),
            });
            current = parent;
        }
        owner_chain.reverse();
        let source_set = serde_json::to_value(&reference.source_id)
            .expect("source id is serializable")
            .as_str()
            .expect("source id serializes as a string")
            .to_string();
        LegacyGraphObjectRef {
            source_set,
            owner_chain,
            kind: reference.kind.clone(),
            name: reference.display_name.clone(),
        }
    }
}

impl Serialize for NavigationGraph {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let nodes = self.nodes.iter().map(|node| LegacyNavigationNode {
            reference: self.legacy_reference(&node.reference),
            capability_state: node.capability_state,
            action_profile: node.action_profile,
            semantic_actions: node.semantic_actions.clone(),
        }).collect::<Vec<_>>();
        let edges = self.edges.iter().map(|edge| LegacyNavigationEdge {
            from: self.legacy_reference(&edge.from),
            to: self.legacy_reference(&edge.to),
            relation: edge.relation,
            capability_state: edge.capability_state,
            semantic_actions: edge.semantic_actions.clone(),
        }).collect::<Vec<_>>();
        let mut state = serializer.serialize_struct("NavigationGraph", 7)?;
        state.serialize_field("prototypeVersion", &self.prototype_version)?;
        state.serialize_field("prototype", &self.prototype)?;
        state.serialize_field("actionSemantics", &self.action_semantics)?;
        state.serialize_field("representation", &self.representation)?;
        state.serialize_field("root", &self.legacy_reference(&self.root))?;
        state.serialize_field("nodes", &nodes)?;
        state.serialize_field("edges", &edges)?;
        state.end()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NavigationStatus { Available, Unavailable }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceAdapterDiagnostic {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl From<SourceAdapterError> for SourceAdapterDiagnostic {
    fn from(error: SourceAdapterError) -> Self {
        Self { code: error.code().to_string(), message: error.message }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationRelationPage {
    pub(crate) relation: RelationRef,
    pub(crate) nodes: Vec<NavigationNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) next_cursor: Option<NavigationCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationEnvelope {
    pub(crate) schema_version: String,
    pub(crate) status: NavigationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) snapshot: Option<SourceSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) root: Option<ObjectRef>,
    pub(crate) nodes: Vec<NavigationNode>,
    pub(crate) relations: Vec<NavigationRelationPage>,
    pub(crate) diagnostics: Vec<SourceAdapterDiagnostic>,
}

impl NavigationEnvelope {
    pub(crate) fn unavailable(error: SourceAdapterError) -> Self {
        Self { schema_version: "1".to_string(), status: NavigationStatus::Unavailable, snapshot: None, root: None, nodes: Vec::new(), relations: Vec::new(), diagnostics: vec![error.into()] }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PropertyType {
    Boolean,
    Integer,
    Decimal,
    String,
    LocalizedString,
    Uuid,
    Enum { enum_type: String },
    Date,
    TypeSet,
    ObjectRef,
    List,
    Structure,
    Null,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TypeSetValue { pub(crate) types: BTreeSet<String> }

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PropertyValue {
    Boolean(bool),
    Integer(i64),
    Decimal(String),
    String(String),
    LocalizedString(BTreeMap<String, String>),
    Uuid(Uuid),
    EnumSymbol(String),
    Date(String),
    TypeSet(TypeSetValue),
    ObjectRef(ObjectRef),
    List(Vec<PropertyValue>),
    Structure(BTreeMap<String, PropertyValue>),
    Null,
    Unknown { summary: String },
}

impl Serialize for PropertyValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        match self {
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Decimal(value) | Self::String(value) | Self::EnumSymbol(value) | Self::Date(value) => serializer.serialize_str(value),
            Self::LocalizedString(value) => value.serialize(serializer),
            Self::Uuid(value) => serializer.serialize_str(&value.to_string()),
            Self::TypeSet(value) => value.serialize(serializer),
            Self::ObjectRef(value) => value.serialize(serializer),
            Self::List(value) => value.serialize(serializer),
            Self::Structure(value) => value.serialize(serializer),
            Self::Null => serializer.serialize_none(),
            Self::Unknown { summary } => {
                let mut map = BTreeMap::new();
                map.insert("summary", summary);
                map.serialize(serializer)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PropertyValueState { Explicit, Defaulted, Inherited, Computed, Absent, Unresolved }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProjectorProfile {
    PlatformXmlV1,
    EdtV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PropertyProvenance {
    Descriptor,
    ProjectorDefault { profile: ProjectorProfile },
    Inherited,
    Computed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PropertyCapability { ReadOnly, Authorable, Unknown }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticProperty {
    #[serde(rename = "type")]
    pub(crate) value_type: PropertyType,
    pub(crate) value_state: PropertyValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<PropertyValue>,
    pub(crate) provenance: PropertyProvenance,
    pub(crate) capability: PropertyCapability,
}

impl SemanticProperty {
    pub(crate) fn explicit(value_type: PropertyType, value: PropertyValue, provenance: PropertyProvenance) -> Result<Self, SourceAdapterError> {
        if !property_value_matches(&value_type, &value) {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, "property type does not match its value"));
        }
        Ok(Self { value_type, value_state: PropertyValueState::Explicit, value: Some(value), provenance, capability: PropertyCapability::Unknown })
    }

    pub(crate) fn absent(value_type: PropertyType, provenance: PropertyProvenance) -> Self {
        Self { value_type, value_state: PropertyValueState::Absent, value: None, provenance, capability: PropertyCapability::Unknown }
    }

    pub(crate) fn unresolved(value_type: PropertyType, provenance: PropertyProvenance) -> Self {
        Self { value_type, value_state: PropertyValueState::Unresolved, value: None, provenance, capability: PropertyCapability::Unknown }
    }

    pub(crate) fn defaulted(value_type: PropertyType, value: PropertyValue, projector_profile: ProjectorProfile) -> Result<Self, SourceAdapterError> {
        if !property_value_matches(&value_type, &value) {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, "defaulted property requires an exact projector profile and compatible value"));
        }
        Ok(Self { value_type, value_state: PropertyValueState::Defaulted, value: Some(value), provenance: PropertyProvenance::ProjectorDefault { profile: projector_profile }, capability: PropertyCapability::Unknown })
    }
}

fn property_value_matches(value_type: &PropertyType, value: &PropertyValue) -> bool {
    matches!((value_type, value),
        (PropertyType::Boolean, PropertyValue::Boolean(_)) |
        (PropertyType::Integer, PropertyValue::Integer(_)) |
        (PropertyType::Decimal, PropertyValue::Decimal(_)) |
        (PropertyType::String, PropertyValue::String(_)) |
        (PropertyType::LocalizedString, PropertyValue::LocalizedString(_)) |
        (PropertyType::Uuid, PropertyValue::Uuid(_)) |
        (PropertyType::Enum { .. }, PropertyValue::EnumSymbol(_)) |
        (PropertyType::Date, PropertyValue::Date(_)) |
        (PropertyType::TypeSet, PropertyValue::TypeSet(_)) |
        (PropertyType::ObjectRef, PropertyValue::ObjectRef(_)) |
        (PropertyType::List, PropertyValue::List(_)) |
        (PropertyType::Structure, PropertyValue::Structure(_)) |
        (PropertyType::Null, PropertyValue::Null) |
        (PropertyType::Unknown, PropertyValue::Unknown { .. })
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NavigationTarget {
    ObjectPath(String),
    ObjectRef { object_ref: ObjectRef, snapshot_revision: SourceRevision },
    Cursor(NavigationCursor),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationQuery { pub(crate) target: NavigationTarget, pub(crate) select: NavigationSelection }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationSelection {
    pub(crate) properties: PropertySelection,
    pub(crate) facets: FacetSelection,
    pub(crate) relations: Vec<RelationSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PropertySelection { All, Named(BTreeSet<String>) }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FacetSelection { None, Summary, Full }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RelationSelection {
    pub(crate) kind: Option<RelationKind>,
    pub(crate) role: String,
    pub(crate) page_size: u16,
}

impl RelationSelection {
    pub(crate) fn new(role: impl Into<String>, page_size: Option<u16>) -> Result<Self, SourceAdapterError> {
        let role = role.into();
        let page_size = page_size.unwrap_or(25);
        if role.is_empty() || role.chars().any(char::is_control) || page_size == 0 || page_size > 100 {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, "invalid relation selection"));
        }
        Ok(Self { kind: None, role, page_size })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NavigationCursor {
    pub(crate) schema_version: u16,
    pub(crate) source_id: SourceId,
    pub(crate) snapshot_revision: SourceRevision,
    pub(crate) target: ObjectKey,
    pub(crate) relation: RelationKey,
    pub(crate) selection_hash: String,
    pub(crate) next_position: u64,
}

impl NavigationCursor {
    pub(crate) const SCHEMA_VERSION: u16 = 1;

    pub(crate) fn issue(
        source_id: SourceId,
        snapshot_revision: SourceRevision,
        target: ObjectKey,
        relation: RelationRef,
        selection: NavigationSelection,
        next_position: u64,
    ) -> Result<Self, SourceAdapterError> {
        if source_id != relation.source_id {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, "navigation cursor relation belongs to another source"));
        }
        Ok(Self {
            schema_version: Self::SCHEMA_VERSION,
            source_id,
            snapshot_revision,
            target,
            relation: relation.relation_key,
            selection_hash: normalized_selection_hash(&selection)?,
            next_position,
        })
    }

    pub(crate) fn resume(&self, current_revision: &SourceRevision) -> Result<(), SourceAdapterError> {
        if &self.snapshot_revision != current_revision {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::SnapshotStale, "navigation cursor snapshot revision is stale"));
        }
        Ok(())
    }

    pub(crate) fn decode<F>(
        value: serde_json::Value,
        current_revision: &SourceRevision,
        expected_selection: &NavigationSelection,
        re_resolve: F,
    ) -> Result<Self, SourceAdapterError>
    where
        F: FnOnce(&SourceId, &ObjectKey, &RelationKey) -> bool,
    {
        let object = value.as_object().ok_or_else(|| SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor must be a JSON object",
        ))?;
        let string = |name: &str| -> Result<&str, SourceAdapterError> {
            object.get(name).and_then(serde_json::Value::as_str).ok_or_else(|| SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                format!("navigation cursor has no valid {name}"),
            ))
        };
        let schema_version = object.get("schemaVersion").and_then(serde_json::Value::as_u64).ok_or_else(|| SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor has no valid schemaVersion",
        ))?;
        if schema_version != u64::from(Self::SCHEMA_VERSION) {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, "unsupported navigation cursor schema version"));
        }
        let source_id = SourceId::new(string("sourceId")?)?;
        let snapshot_revision = SourceRevision::new(string("snapshotRevision")?)?;
        let target = ObjectKey::new(string("target")?)?;
        let relation = RelationKey::new(string("relation")?)?;
        let selection_hash = string("selectionHash")?.to_string();
        if selection_hash != normalized_selection_hash(expected_selection)? {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, "navigation cursor selectionHash is not normalized"));
        }
        let next_position = object.get("nextPosition").and_then(serde_json::Value::as_u64).ok_or_else(|| SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor has no valid nextPosition",
        ))?;
        let cursor = Self { schema_version: Self::SCHEMA_VERSION, source_id, snapshot_revision, target, relation, selection_hash, next_position };
        cursor.resume(current_revision)?;
        if !re_resolve(&cursor.source_id, &cursor.target, &cursor.relation) {
            return Err(SourceAdapterError::new(SourceAdapterErrorKind::SourceUnavailable, "navigation cursor target or relation cannot be re-resolved"));
        }
        Ok(cursor)
    }
}

fn normalized_selection_hash(selection: &NavigationSelection) -> Result<String, SourceAdapterError> {
    let properties = match &selection.properties {
        PropertySelection::All => "all".to_string(),
        PropertySelection::Named(names) => format!(
            "named:{}",
            names.iter().map(|name| normalize_selection_token(name)).collect::<Result<Vec<_>, _>>()?.join(","),
        ),
    };
    let facets = match selection.facets {
        FacetSelection::None => "none",
        FacetSelection::Summary => "summary",
        FacetSelection::Full => "full",
    };
    let mut relations = selection.relations.iter().map(|relation| {
        let kind = match relation.kind {
            Some(RelationKind::Contains) => "contains",
            Some(RelationKind::References) => "references",
            None => "any",
        };
        Ok(format!("{kind}:{}:{}", normalize_selection_token(&relation.role)?, relation.page_size))
    }).collect::<Result<Vec<_>, SourceAdapterError>>()?;
    relations.sort();
    let mut digest = Sha256::new();
    digest.update(b"unica.navigation.selection.v1\0");
    digest.update(properties.as_bytes());
    digest.update([0]);
    digest.update(facets.as_bytes());
    digest.update([0]);
    digest.update(relations.join("|").as_bytes());
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn normalize_selection_token(value: &str) -> Result<String, SourceAdapterError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, "selection contains an invalid token"));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_id(raw: &str) -> SourceId { SourceId::new(raw).unwrap() }
    fn object_key(raw: &str) -> ObjectKey { ObjectKey::new(raw).unwrap() }
    fn node_ref() -> ObjectRef {
        ObjectRef::new(source_id("workspace:main"), object_key("uuid:11111111-1111-1111-1111-111111111111"), IdentityStrength::Persistent, NodeKind::Document, "Order")
    }
    fn relation_ref() -> RelationRef { RelationRef::new(source_id("workspace:main"), "contains:document-attributes", RelationKind::Contains).unwrap() }
    fn selection() -> NavigationSelection {
        NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from(["name".to_string()])),
            facets: FacetSelection::Summary,
            relations: vec![RelationSelection::new("attributes", Some(25)).unwrap()],
        }
    }

    #[test]
    fn object_identity_is_not_derived_from_display_name_alone() {
        let left = node_ref();
        let renamed = ObjectRef::new(source_id("workspace:main"), object_key("uuid:11111111-1111-1111-1111-111111111111"), IdentityStrength::Persistent, NodeKind::Document, "CustomerOrder");
        assert_eq!(left.identity(), renamed.identity());
        assert_eq!(left, renamed);
    }

    #[test]
    fn resolved_authorable_but_format_incompatible_is_not_executable() {
        let capability = CapabilityVector { resolution: ResolutionState::Resolved, identity: IdentityStrength::Persistent, consistency: SnapshotConsistency::Consistent, coverage: CoverageState::Complete, format: FormatCompatibility::Incompatible, source_access: SourceAccess::ReadWrite, authorability: Authorability::Authorable };
        assert!(!capability.permits_mutation());
        assert_eq!(capability.blocking_reasons(), vec![CapabilityBlockReason::FormatIncompatible]);
    }

    #[test]
    fn clone_requires_an_explicit_owning_relation() {
        let action = SemanticAction::modeled_clone(node_ref(), None);
        assert_eq!(action.availability, ActionAvailability::Blocked);
        assert_eq!(action.blocking_reasons, vec![CapabilityBlockReason::OwningRelationMissing]);

        let reference_relation = RelationRef::new(source_id("workspace:main"), "reference:document-owner", RelationKind::References).unwrap();
        let wrong_kind = SemanticAction::modeled_clone(node_ref(), Some(reference_relation));
        assert_eq!(wrong_kind.availability, ActionAvailability::Blocked);

        let foreign_relation = RelationRef::new(source_id("workspace:other"), "contains:foreign-owner", RelationKind::Contains).unwrap();
        let wrong_source = SemanticAction::modeled_clone(node_ref(), Some(foreign_relation));
        assert_eq!(wrong_source.availability, ActionAvailability::Blocked);
    }

    #[test]
    fn navigation_envelope_always_has_a_schema_version_and_status() {
        let envelope = NavigationEnvelope::unavailable(SourceAdapterError::new(SourceAdapterErrorKind::FormatUnsupported, "Platform XML 2.19 has no certified reader"));
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["schemaVersion"], "1");
        assert_eq!(value["status"], "unavailable");
        assert_eq!(value["diagnostics"][0]["code"], "format_unsupported");
    }

    #[test]
    fn properties_preserve_type_value_and_value_state() {
        let property = SemanticProperty::explicit(PropertyType::Integer, PropertyValue::Integer(11), PropertyProvenance::Descriptor).unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["type"], "integer");
        assert_eq!(value["value"], 11);
        assert_eq!(value["valueState"], "explicit");
    }

    #[test]
    fn incompatible_property_type_and_value_are_rejected() {
        let error = SemanticProperty::explicit(PropertyType::Integer, PropertyValue::String("11".to_string()), PropertyProvenance::Descriptor).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn relation_page_size_is_bounded() {
        assert_eq!(RelationSelection::new("attributes", None).unwrap().page_size, 25);
        assert!(RelationSelection::new("attributes", Some(101)).is_err());
    }

    #[test]
    fn cursor_is_bound_to_snapshot_revision() {
        let cursor = NavigationCursor::issue(source_id("workspace:main"), SourceRevision::new("sha256:one").unwrap(), object_key("uuid:11111111-1111-1111-1111-111111111111"), relation_ref(), selection(), 0).unwrap();
        let error = cursor.resume(&SourceRevision::new("sha256:two").unwrap()).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn cursor_decode_validates_schema_hash_and_semantic_resolution() {
        let cursor = NavigationCursor::issue(source_id("workspace:main"), SourceRevision::new("sha256:one").unwrap(), object_key("uuid:11111111-1111-1111-1111-111111111111"), relation_ref(), selection(), 0).unwrap();
        let decoded = NavigationCursor::decode(
            serde_json::to_value(cursor).unwrap(),
            &SourceRevision::new("sha256:one").unwrap(),
            &selection(),
            |_source, _target, _relation| true,
        ).unwrap();
        assert_eq!(decoded.schema_version, NavigationCursor::SCHEMA_VERSION);
    }

    #[test]
    fn cursor_rejects_relation_from_another_source_and_preserves_target() {
        let target = object_key("uuid:11111111-1111-1111-1111-111111111111");
        let foreign_relation = RelationRef::new(source_id("workspace:other"), "contains:foreign-owner", RelationKind::Contains).unwrap();
        let error = NavigationCursor::issue(
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            target.clone(),
            foreign_relation,
            selection(),
            44,
        ).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);

        let cursor = NavigationCursor::issue(
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            target.clone(),
            relation_ref(),
            selection(),
            44,
        ).unwrap();
        assert_eq!(cursor.target, target);
        assert_eq!(cursor.next_position, 44);
    }

    #[test]
    fn cursor_rejects_well_formed_hash_for_a_different_selection() {
        let cursor = NavigationCursor::issue(
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_ref(),
            selection(),
            0,
        ).unwrap();
        let mut value = serde_json::to_value(cursor).unwrap();
        value["selectionHash"] = serde_json::Value::String(format!("sha256:{}", "0".repeat(64)));
        let error = NavigationCursor::decode(
            value,
            &SourceRevision::new("sha256:one").unwrap(),
            &selection(),
            |_source, _target, _relation| true,
        ).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn opaque_keys_reject_path_shaped_values() {
        for value in ["", "/tmp/object", r"\\server\\share", "C:\\object", "line\nfeed"] {
            assert!(ObjectKey::new(value).is_err(), "{value:?} must not be an object key");
            assert!(RelationKey::new(value).is_err(), "{value:?} must not be a relation key");
        }
    }

    #[test]
    fn action_is_executable_only_with_capability_and_binding() {
        let capability = CapabilityVector { resolution: ResolutionState::Resolved, identity: IdentityStrength::Persistent, consistency: SnapshotConsistency::Consistent, coverage: CoverageState::Complete, format: FormatCompatibility::Compatible, source_access: SourceAccess::ReadWrite, authorability: Authorability::Authorable };
        let modeled = SemanticAction::mutation(SemanticActionKind::EditProperties, node_ref(), capability.clone(), None, None, Atomicity::SingleFileAtomicReplace);
        assert_eq!(modeled.availability, ActionAvailability::Modeled);
        let executable = SemanticAction::mutation(SemanticActionKind::EditProperties, node_ref(), capability, None, Some(OperationBinding { tool: "unica.meta.edit".to_string(), schema_version: "1".to_string() }), Atomicity::SingleFileAtomicReplace);
        assert_eq!(executable.availability, ActionAvailability::Executable);

        let invalid = SemanticAction::mutation(SemanticActionKind::EditProperties, node_ref(), CapabilityVector { resolution: ResolutionState::Resolved, identity: IdentityStrength::Persistent, consistency: SnapshotConsistency::Consistent, coverage: CoverageState::Complete, format: FormatCompatibility::Compatible, source_access: SourceAccess::ReadWrite, authorability: Authorability::Authorable }, None, Some(OperationBinding { tool: "other.meta.edit".to_string(), schema_version: "".to_string() }), Atomicity::SingleFileAtomicReplace);
        assert_eq!(invalid.availability, ActionAvailability::Blocked);
        assert_eq!(invalid.blocking_reasons, vec![CapabilityBlockReason::OperationBindingInvalid]);
    }

    #[test]
    fn defaulted_property_keeps_a_typed_exact_projector_profile() {
        let property = SemanticProperty::defaulted(
            PropertyType::Integer,
            PropertyValue::Integer(11),
            ProjectorProfile::PlatformXmlV1,
        ).unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["valueState"], "defaulted");
        assert_eq!(value["provenance"]["projectorDefault"]["profile"], "platform_xml_v1");
    }

    #[test]
    fn action_profiles_keep_remove_unadvertised_and_clone_relation_atomic() {
        let actions = semantic_actions_for(&NodeKind::Document, CapabilityState::resolved_authorable());
        assert!(actions.iter().any(|action| action.action == SemanticActionKind::Clone && action.execution_policy == ActionExecutionPolicy::AtomicRelationMutation));
        assert!(!actions.iter().any(|action| action.action == SemanticActionKind::Remove));
    }

    #[test]
    fn legacy_graph_serialization_keeps_meta_info_contract_until_task_eight() {
        let root = node_ref();
        let child = ObjectRef::new(source_id("workspace:main"), object_key("uuid:22222222-2222-2222-2222-222222222222"), IdentityStrength::Derived, NodeKind::Attribute, "Number");
        let graph = NavigationGraph::new(Representation::PlatformXml, root.clone(), vec![NavigationNode::new(root.clone(), CapabilityState::resolved_authorable()), NavigationNode::new(child.clone(), CapabilityState::resolved_authorable())], vec![NavigationEdge::new(root, child, RelationKind::Contains, CapabilityState::resolved_authorable())]);
        let value = serde_json::to_value(graph).unwrap();
        assert_eq!(value["root"]["sourceSet"], "workspace:main");
        assert_eq!(value["nodes"][1]["reference"]["ownerChain"][0]["name"], "Order");
        assert!(value["root"].get("objectKey").is_none());
    }
}
