//! JSON navigation contracts for semantic 1C metadata projections.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::str::FromStr;

use serde::{ser::SerializeStruct, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::limits::{
    MAX_NAVIGATION_PROPERTY_SELECTORS, MAX_NAVIGATION_RELATION_SELECTORS,
    MAX_NAVIGATION_SELECTOR_STRING_BYTES,
};
use crate::source::{
    SnapshotConsistency, SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceId,
    SourceRevision, SourceSnapshot,
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObjectKey(String);

impl ObjectKey {
    pub fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_opaque_key(&raw, "object key")?;
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
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
pub struct RelationKey(String);

impl RelationKey {
    pub fn new(raw: impl Into<String>) -> Result<Self, SourceAdapterError> {
        let raw = raw.into();
        validate_opaque_key(&raw, "relation key")?;
        Ok(Self(raw))
    }

    pub fn as_str(&self) -> &str {
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
    let windows_drive =
        raw.len() >= 2 && raw.as_bytes()[0].is_ascii_alphabetic() && raw.as_bytes()[1] == b':';
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
pub enum IdentityStrength {
    Persistent,
    Derived,
    SnapshotOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectIdentity {
    pub source_id: SourceId,
    pub object_key: ObjectKey,
}

/// A path-free, versioned semantic object reference.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    pub source_id: SourceId,
    pub object_key: ObjectKey,
    pub identity_strength: IdentityStrength,
    pub kind: NodeKind,
    pub display_name: String,
}

impl ObjectRef {
    pub fn new(
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

    pub fn identity(&self) -> ObjectIdentity {
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
pub enum NodeKind {
    SourceRoot,
    Document,
    MetadataObject {
        metadata_type: String,
    },
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
    pub fn metadata_object(metadata_type: impl Into<String>) -> Self {
        Self::MetadataObject {
            metadata_type: metadata_type.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    PlatformXml,
    Edt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    Resolved,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorability {
    Authorable,
    SupportLocked,
    ConfigurationReadOnly,
    UnknownSupportState,
    UnknownReadOnly,
    DerivedReadOnly,
}

/// Compatibility state of the source format with the selected adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FormatCompatibility {
    Compatible,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CoverageState {
    Complete,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapabilityBlockReason {
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
pub struct CapabilityVector {
    pub resolution: ResolutionState,
    pub identity: IdentityStrength,
    pub consistency: SnapshotConsistency,
    pub coverage: CoverageState,
    pub format: FormatCompatibility,
    pub source_access: SourceAccess,
    pub authorability: Authorability,
}

impl CapabilityVector {
    pub const fn permits_mutation(&self) -> bool {
        matches!(self.resolution, ResolutionState::Resolved)
            && !matches!(self.identity, IdentityStrength::SnapshotOnly)
            && matches!(self.consistency, SnapshotConsistency::Consistent)
            && matches!(self.coverage, CoverageState::Complete)
            && matches!(self.format, FormatCompatibility::Compatible)
            && matches!(self.source_access, SourceAccess::ReadWrite)
            && matches!(self.authorability, Authorability::Authorable)
    }

    pub fn blocking_reasons(&self) -> Vec<CapabilityBlockReason> {
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
pub struct CapabilityState {
    pub resolution_state: ResolutionState,
    pub authorability: Authorability,
}

impl CapabilityState {
    pub const fn new(resolution_state: ResolutionState, authorability: Authorability) -> Self {
        Self {
            resolution_state,
            authorability,
        }
    }

    pub const fn resolved_authorable() -> Self {
        Self::new(ResolutionState::Resolved, Authorability::Authorable)
    }

    pub const fn is_resolved_authorable(self) -> bool {
        matches!(self.resolution_state, ResolutionState::Resolved)
            && matches!(self.authorability, Authorability::Authorable)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Contains,
    References,
}

/// Closed, versioned ownership roles assigned by a certified projector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RelationRole {
    Children,
    Attributes,
    TabularSections,
    Forms,
    Commands,
    Templates,
}

impl RelationRole {
    pub fn parse(value: &str) -> Result<Self, SourceAdapterError> {
        match value {
            "children" => Ok(Self::Children),
            "attributes" => Ok(Self::Attributes),
            "tabularSections" => Ok(Self::TabularSections),
            "forms" => Ok(Self::Forms),
            "commands" => Ok(Self::Commands),
            "templates" => Ok(Self::Templates),
            _ => Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "invalid relation role",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationRef {
    pub source_id: SourceId,
    pub relation_key: RelationKey,
    pub kind: RelationKind,
}

/// Page identity for the exact owner/role/kind aggregate, distinct from edge identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationGroupRef {
    pub source_id: SourceId,
    pub group_key: RelationKey,
    pub owner: ObjectRef,
    pub role: RelationRole,
    pub kind: RelationKind,
}

/// A semantic relation is an independently addressable aggregate.  Its source
/// and target are opaque semantic references, never native locations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRelation {
    pub relation_ref: RelationRef,
    pub group_ref: RelationGroupRef,
    pub identity_strength: IdentityStrength,
    pub kind: RelationKind,
    pub role: RelationRole,
    pub source: ObjectRef,
    pub target: ObjectRef,
    pub capability: CapabilityVector,
}

impl RelationRef {
    pub fn new(
        source_id: SourceId,
        relation_key: impl Into<String>,
        kind: RelationKind,
    ) -> Result<Self, SourceAdapterError> {
        Ok(Self {
            source_id,
            relation_key: RelationKey::new(relation_key)?,
            kind,
        })
    }
}

impl RelationGroupRef {
    pub fn new(
        source_id: SourceId,
        owner: ObjectRef,
        role: RelationRole,
        kind: RelationKind,
    ) -> Result<Self, SourceAdapterError> {
        let canonical =
            serde_json::to_vec(&(&source_id, &owner.object_key, role, kind)).map_err(|error| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::ProjectionAmbiguous,
                    format!("cannot serialize relation group: {error}"),
                )
            })?;
        let mut digest = Sha256::new();
        digest.update(b"unica.navigation.relation-group.v1\0");
        digest.update(canonical);
        Ok(Self {
            source_id,
            group_key: RelationKey::new(format!("group:sha256:{:x}", digest.finalize()))?,
            owner,
            role,
            kind,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionAvailability {
    Modeled,
    Executable,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Atomicity {
    SingleFileAtomicReplace,
    AggregateSwapWithRecovery,
    BackendTransaction,
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationBinding {
    pub tool: String,
    pub schema_version: String,
}

impl OperationBinding {
    pub fn is_valid(&self) -> bool {
        self.tool.starts_with("unica.")
            && self.tool.len() > "unica.".len()
            && !self.tool.chars().any(char::is_control)
            && !self.schema_version.is_empty()
            && !self.schema_version.chars().any(char::is_control)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticActionKind {
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

pub type ActionKind = SemanticActionKind;

/// A capability-qualified semantic action, independent from a particular MCP
/// transport. Mutation actions are only executable with every capability
/// precondition and an explicit native operation binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticAction {
    pub kind: SemanticActionKind,
    pub target: Option<ObjectRef>,
    pub owning_relation: Option<RelationRef>,
    pub availability: ActionAvailability,
    pub blocking_reasons: Vec<CapabilityBlockReason>,
    pub operation_binding: Option<OperationBinding>,
    pub atomicity: Atomicity,
}

impl SemanticAction {
    pub fn modeled_clone(target: ObjectRef, owning_relation: Option<RelationRef>) -> Self {
        let mut blocking_reasons = Vec::new();
        if !owning_relation.as_ref().is_some_and(|relation| {
            relation.source_id == target.source_id
                && matches!(relation.kind, RelationKind::Contains)
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

    pub fn mutation(
        kind: SemanticActionKind,
        target: ObjectRef,
        capability: CapabilityVector,
        owning_relation: Option<RelationRef>,
        operation_binding: Option<OperationBinding>,
        atomicity: Atomicity,
    ) -> Self {
        let mut blocking_reasons = capability.blocking_reasons();
        if matches!(kind, SemanticActionKind::Clone)
            && !owning_relation.as_ref().is_some_and(|relation| {
                relation.source_id == target.source_id
                    && matches!(relation.kind, RelationKind::Contains)
            })
        {
            blocking_reasons.push(CapabilityBlockReason::OwningRelationMissing);
        }
        if operation_binding
            .as_ref()
            .is_some_and(|binding| !binding.is_valid())
        {
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
        Self {
            kind,
            target: Some(target),
            owning_relation,
            availability,
            blocking_reasons,
            operation_binding,
            atomicity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionExecutionPolicy {
    ReadOnly,
    AtomicNodeMutation,
    AtomicRelationMutation,
}

impl ActionExecutionPolicy {
    pub const fn is_mutation(self) -> bool {
        matches!(
            self,
            Self::AtomicNodeMutation | Self::AtomicRelationMutation
        )
    }

    pub const fn validates_before_commit(self) -> bool {
        self.is_mutation()
    }
    pub const fn allows_cross_action_changeset(self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticActionDescriptor {
    pub action: SemanticActionKind,
    pub execution_policy: ActionExecutionPolicy,
}

impl SemanticActionDescriptor {
    const fn read(action: SemanticActionKind) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::ReadOnly,
        }
    }
    const fn node_mutation(action: SemanticActionKind) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::AtomicNodeMutation,
        }
    }
    const fn relation_mutation(action: SemanticActionKind) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::AtomicRelationMutation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionProfile {
    DocumentMetadataObject,
    GenericMetadataObject,
    Form,
    FormElement,
    TabularSection,
    MxlTemplate,
    UnmodeledTemplate,
    UnmodeledChild,
}

pub fn action_profile_for(kind: &NodeKind) -> ActionProfile {
    match kind {
        NodeKind::Document => ActionProfile::DocumentMetadataObject,
        NodeKind::MetadataObject { metadata_type } if metadata_type == "Document" => {
            ActionProfile::DocumentMetadataObject
        }
        NodeKind::MetadataObject { .. } | NodeKind::SourceRoot => {
            ActionProfile::GenericMetadataObject
        }
        NodeKind::Form => ActionProfile::Form,
        NodeKind::FormElement => ActionProfile::FormElement,
        NodeKind::TabularSection => ActionProfile::TabularSection,
        NodeKind::Template {
            template_type: Some(template_type),
        } if template_type == "SpreadsheetDocument" => ActionProfile::MxlTemplate,
        NodeKind::Template { .. } => ActionProfile::UnmodeledTemplate,
        NodeKind::Attribute
        | NodeKind::Command
        | NodeKind::FormAttribute
        | NodeKind::FormCommand => ActionProfile::UnmodeledChild,
    }
}

pub fn semantic_actions_for(
    kind: &NodeKind,
    capability_state: CapabilityState,
) -> Vec<SemanticActionDescriptor> {
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
        ActionProfile::GenericMetadataObject
        | ActionProfile::UnmodeledTemplate
        | ActionProfile::UnmodeledChild => {
            vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)]
        }
    }
}

pub fn semantic_actions_for_relation(
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
        (
            RelationKind::References,
            NodeKind::FormElement,
            NodeKind::Attribute | NodeKind::FormAttribute,
        ) => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::BindData),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::RebindData),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::UnbindData),
        ],
        (
            RelationKind::References,
            NodeKind::FormElement,
            NodeKind::Command | NodeKind::FormCommand,
        ) => vec![
            SemanticActionDescriptor::read(SemanticActionKind::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::BindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::RebindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticActionKind::UnbindCommand),
        ],
        _ => vec![SemanticActionDescriptor::read(SemanticActionKind::Inspect)],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationFacetVisibility {
    Full,
    Summary,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationNode {
    pub object_ref: ObjectRef,
    pub reference: ObjectRef,
    pub capability_state: CapabilityState,
    pub capability: CapabilityVector,
    pub properties: BTreeMap<String, SemanticProperty>,
    pub action_profile: ActionProfile,
    pub semantic_actions: Vec<SemanticActionDescriptor>,
    pub actions: Vec<SemanticAction>,
    pub facet_visibility: NavigationFacetVisibility,
}

impl NavigationNode {
    pub fn new(reference: ObjectRef, capability_state: CapabilityState) -> Self {
        let action_profile = action_profile_for(&reference.kind);
        let semantic_actions = semantic_actions_for(&reference.kind, capability_state);
        let capability = CapabilityVector {
            resolution: capability_state.resolution_state,
            identity: IdentityStrength::Derived,
            consistency: SnapshotConsistency::Consistent,
            coverage: CoverageState::Complete,
            format: FormatCompatibility::Compatible,
            source_access: SourceAccess::ReadWrite,
            authorability: capability_state.authorability,
        };
        Self {
            object_ref: reference.clone(),
            reference,
            capability_state,
            capability,
            properties: BTreeMap::new(),
            action_profile,
            semantic_actions,
            actions: Vec::new(),
            facet_visibility: NavigationFacetVisibility::Full,
        }
    }
    pub fn semantic_actions(&self) -> &[SemanticActionDescriptor] {
        &self.semantic_actions
    }
}

impl Serialize for NavigationNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let fields = match self.facet_visibility {
            NavigationFacetVisibility::Full => 8,
            NavigationFacetVisibility::Summary => 5,
            NavigationFacetVisibility::None => 3,
        };
        let mut state = serializer.serialize_struct("NavigationNode", fields)?;
        state.serialize_field("objectRef", &self.object_ref)?;
        state.serialize_field("reference", &self.reference)?;
        state.serialize_field("properties", &self.properties)?;
        match self.facet_visibility {
            NavigationFacetVisibility::Full => {
                state.serialize_field("capabilityState", &self.capability_state)?;
                state.serialize_field("capability", &self.capability)?;
                state.serialize_field("actionProfile", &self.action_profile)?;
                state.serialize_field("semanticActions", &self.semantic_actions)?;
                state.serialize_field("actions", &self.actions)?;
            }
            NavigationFacetVisibility::Summary => {
                state.serialize_field("capabilityState", &self.capability_state)?;
                state.serialize_field("actionProfile", &self.action_profile)?;
            }
            NavigationFacetVisibility::None => {}
        }
        state.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationEdge {
    pub from: ObjectRef,
    pub to: ObjectRef,
    pub relation: RelationKind,
    pub capability_state: CapabilityState,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

impl NavigationEdge {
    pub fn new(
        from: ObjectRef,
        to: ObjectRef,
        relation: RelationKind,
        capability_state: CapabilityState,
    ) -> Self {
        let semantic_actions =
            semantic_actions_for_relation(&from.kind, &to.kind, relation, capability_state);
        Self {
            from,
            to,
            relation,
            capability_state,
            semantic_actions,
        }
    }
    pub fn semantic_actions(&self) -> &[SemanticActionDescriptor] {
        &self.semantic_actions
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSemantics {
    ModeledCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationGraph {
    pub prototype_version: u32,
    prototype: bool,
    action_semantics: ActionSemantics,
    pub representation: Representation,
    pub root: ObjectRef,
    pub nodes: Vec<NavigationNode>,
    pub edges: Vec<NavigationEdge>,
}

impl NavigationGraph {
    pub const PROTOTYPE_VERSION: u32 = 1;
    pub fn new(
        representation: Representation,
        root: ObjectRef,
        nodes: Vec<NavigationNode>,
        edges: Vec<NavigationEdge>,
    ) -> Self {
        Self {
            prototype_version: Self::PROTOTYPE_VERSION,
            prototype: true,
            action_semantics: ActionSemantics::ModeledCapabilities,
            representation,
            root,
            nodes,
            edges,
        }
    }
    pub const fn is_prototype(&self) -> bool {
        self.prototype
    }
    pub const fn action_semantics(&self) -> ActionSemantics {
        self.action_semantics
    }
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
        let nodes = self
            .nodes
            .iter()
            .map(|node| LegacyNavigationNode {
                reference: self.legacy_reference(&node.reference),
                capability_state: node.capability_state,
                action_profile: node.action_profile,
                semantic_actions: node.semantic_actions.clone(),
            })
            .collect::<Vec<_>>();
        let edges = self
            .edges
            .iter()
            .map(|edge| LegacyNavigationEdge {
                from: self.legacy_reference(&edge.from),
                to: self.legacy_reference(&edge.to),
                relation: edge.relation,
                capability_state: edge.capability_state,
                semantic_actions: edge.semantic_actions.clone(),
            })
            .collect::<Vec<_>>();
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
pub enum NavigationStatus {
    #[serde(rename = "ready")]
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAdapterDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl From<SourceAdapterError> for SourceAdapterDiagnostic {
    fn from(error: SourceAdapterError) -> Self {
        Self {
            code: error.code().to_string(),
            message: error.message,
            details: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationRelationPage {
    pub relation: RelationGroupRef,
    pub items: Vec<NavigationNode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<NavigationCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationEnvelope {
    pub schema_version: String,
    pub status: NavigationStatus,
    pub snapshot: Option<SourceSnapshot>,
    pub root: Option<ObjectRef>,
    pub nodes: Vec<NavigationNode>,
    pub relations: Vec<NavigationRelationPage>,
    pub diagnostics: Vec<SourceAdapterDiagnostic>,
    #[serde(skip)]
    pub relation_index: std::sync::Arc<Vec<SemanticRelation>>,
}

impl NavigationEnvelope {
    pub fn unavailable(error: SourceAdapterError) -> Self {
        Self {
            schema_version: "1".to_string(),
            status: NavigationStatus::Unavailable,
            snapshot: None,
            root: None,
            nodes: Vec::new(),
            relations: Vec::new(),
            diagnostics: vec![error.into()],
            relation_index: std::sync::Arc::new(Vec::new()),
        }
    }

    pub fn node_named(&self, kind: NodeKind, name: &str) -> Option<&NavigationNode> {
        self.nodes
            .iter()
            .find(|node| node.object_ref.kind == kind && node.object_ref.display_name == name)
    }

    pub fn owning_relation(&self, object: &ObjectRef) -> Option<&SemanticRelation> {
        self.relation_index.iter().find(|relation| {
            matches!(relation.kind, RelationKind::Contains) && relation.target == *object
        })
    }

    pub fn action(&self, kind: ActionKind, name: &str) -> Option<&SemanticAction> {
        self.nodes
            .iter()
            .find(|node| node.object_ref.display_name == name)
            .and_then(|node| node.actions.iter().find(|action| action.kind == kind))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyType {
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
pub struct TypeSetValue {
    pub variants: Vec<TypeVariant>,
}

/// A 1C type description normalized for consumers.  XML type expressions are
/// adapter-private evidence and never the canonical property value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TypeVariant {
    Primitive {
        kind: String,
        qualifiers: BTreeMap<String, PropertyValue>,
    },
    Reference {
        target: String,
    },
    Enumeration {
        target: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyValue {
    Boolean(bool),
    Integer(i64),
    Decimal(String),
    String(String),
    LocalizedString(BTreeMap<String, String>),
    Uuid(UuidValue),
    EnumSymbol(String),
    Date(String),
    TypeSet(TypeSetValue),
    ObjectRef(ObjectRef),
    List(Vec<PropertyValue>),
    Structure(BTreeMap<String, PropertyValue>),
    Null,
    Unknown { summary: String },
}

/// Parser-independent UUID value used by neutral semantic properties.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct UuidValue(String);

impl UuidValue {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for UuidValue {
    type Err = SourceAdapterError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let raw = raw.strip_prefix("urn:uuid:").unwrap_or(raw);
        let raw = raw
            .strip_prefix('{')
            .and_then(|value| value.strip_suffix('}'))
            .unwrap_or(raw);
        let valid_shape = (raw.len() == 32 && raw.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || (raw.len() == 36
                && raw.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_hexdigit()
                    }
                }));
        if !valid_shape {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "invalid UUID value",
            ));
        }
        let compact = raw.bytes().filter(|byte| *byte != b'-').collect::<Vec<_>>();
        let compact = String::from_utf8(compact)
            .expect("ASCII hexadecimal UUID validation guarantees UTF-8")
            .to_ascii_lowercase();
        Ok(Self(format!(
            "{}-{}-{}-{}-{}",
            &compact[0..8],
            &compact[8..12],
            &compact[12..16],
            &compact[16..20],
            &compact[20..32],
        )))
    }
}

impl Display for UuidValue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Serialize for PropertyValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Boolean(value) => serializer.serialize_bool(*value),
            Self::Integer(value) => serializer.serialize_i64(*value),
            Self::Decimal(value)
            | Self::String(value)
            | Self::EnumSymbol(value)
            | Self::Date(value) => serializer.serialize_str(value),
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
pub enum PropertyValueState {
    Explicit,
    Defaulted,
    Inherited,
    Computed,
    Absent,
    Unresolved,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectorProfile {
    PlatformXmlV1,
    EdtV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyProvenance {
    Descriptor,
    ProjectorDefault { profile: ProjectorProfile },
    Inherited,
    Computed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertyCapability {
    ReadOnly,
    Authorable,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticProperty {
    #[serde(rename = "type")]
    pub value_type: PropertyType,
    pub value_state: PropertyValueState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<PropertyValue>,
    pub provenance: PropertyProvenance,
    pub capability: PropertyCapability,
}

impl SemanticProperty {
    pub fn explicit(
        value_type: PropertyType,
        value: PropertyValue,
        provenance: PropertyProvenance,
    ) -> Result<Self, SourceAdapterError> {
        if !property_value_matches(&value_type, &value) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "property type does not match its value",
            ));
        }
        Ok(Self {
            value_type,
            value_state: PropertyValueState::Explicit,
            value: Some(value),
            provenance,
            capability: PropertyCapability::Unknown,
        })
    }

    pub fn absent(value_type: PropertyType, provenance: PropertyProvenance) -> Self {
        Self {
            value_type,
            value_state: PropertyValueState::Absent,
            value: None,
            provenance,
            capability: PropertyCapability::Unknown,
        }
    }

    pub fn unresolved(value_type: PropertyType, provenance: PropertyProvenance) -> Self {
        Self {
            value_type,
            value_state: PropertyValueState::Unresolved,
            value: None,
            provenance,
            capability: PropertyCapability::Unknown,
        }
    }

    pub fn defaulted(
        value_type: PropertyType,
        value: PropertyValue,
        projector_profile: ProjectorProfile,
    ) -> Result<Self, SourceAdapterError> {
        if !property_value_matches(&value_type, &value) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "defaulted property requires an exact projector profile and compatible value",
            ));
        }
        Ok(Self {
            value_type,
            value_state: PropertyValueState::Defaulted,
            value: Some(value),
            provenance: PropertyProvenance::ProjectorDefault {
                profile: projector_profile,
            },
            capability: PropertyCapability::Unknown,
        })
    }
}

fn property_value_matches(value_type: &PropertyType, value: &PropertyValue) -> bool {
    matches!(
        (value_type, value),
        (PropertyType::Boolean, PropertyValue::Boolean(_))
            | (PropertyType::Integer, PropertyValue::Integer(_))
            | (PropertyType::Decimal, PropertyValue::Decimal(_))
            | (PropertyType::String, PropertyValue::String(_))
            | (
                PropertyType::LocalizedString,
                PropertyValue::LocalizedString(_)
            )
            | (PropertyType::Uuid, PropertyValue::Uuid(_))
            | (PropertyType::Enum { .. }, PropertyValue::EnumSymbol(_))
            | (PropertyType::Date, PropertyValue::Date(_))
            | (PropertyType::TypeSet, PropertyValue::TypeSet(_))
            | (PropertyType::ObjectRef, PropertyValue::ObjectRef(_))
            | (PropertyType::List, PropertyValue::List(_))
            | (PropertyType::Structure, PropertyValue::Structure(_))
            | (PropertyType::Null, PropertyValue::Null)
            | (PropertyType::Unknown, PropertyValue::Unknown { .. })
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NavigationTarget {
    ObjectPath(String),
    ObjectRef {
        object_ref: ObjectRef,
        snapshot_revision: SourceRevision,
    },
    Cursor(NavigationCursor),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationQuery {
    pub target: NavigationTarget,
    pub select: NavigationSelection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationSelection {
    pub properties: PropertySelection,
    pub facets: FacetSelection,
    pub relations: Vec<RelationSelection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PropertySelection {
    All,
    Named(BTreeSet<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FacetSelection {
    None,
    Summary,
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationSelection {
    pub kind: RelationKind,
    pub role: RelationRole,
    pub page_size: u16,
}

impl RelationSelection {
    pub fn new(role: impl AsRef<str>, page_size: Option<u16>) -> Result<Self, SourceAdapterError> {
        let role = RelationRole::parse(role.as_ref())?;
        let page_size = page_size.unwrap_or(25);
        if page_size == 0 || page_size > 100 {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "invalid relation selection",
            ));
        }
        Ok(Self {
            kind: RelationKind::Contains,
            role,
            page_size,
        })
    }
}

/// Produces the sole canonical selection representation used for runtime
/// paging, cursor payloads, and selection hashes. Raw omitted kinds are
/// represented as `Contains` before this function is called.
pub fn normalize_navigation_selection(
    mut selection: NavigationSelection,
) -> Result<NavigationSelection, SourceAdapterError> {
    if let PropertySelection::Named(names) = &selection.properties {
        if names.len() > MAX_NAVIGATION_PROPERTY_SELECTORS {
            return Err(resource_limit(
                "navigation selection has too many property selectors",
            ));
        }
    }
    if selection.relations.len() > MAX_NAVIGATION_RELATION_SELECTORS {
        return Err(resource_limit(
            "navigation selection has too many relation selectors",
        ));
    }
    if let PropertySelection::Named(names) = &selection.properties {
        for name in names {
            validate_selection_token(name)?;
        }
    }
    for relation in &selection.relations {
        if relation.page_size == 0 || relation.page_size > 100 {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::ProjectionAmbiguous,
                "invalid relation selection",
            ));
        }
    }
    selection.relations.sort_by(|left, right| {
        serde_json::to_vec(left)
            .expect("relation selection is serializable")
            .cmp(&serde_json::to_vec(right).expect("relation selection is serializable"))
    });
    let mut relations = Vec::new();
    for relation in selection.relations {
        if let Some(existing) = relations
            .iter_mut()
            .find(|existing: &&mut RelationSelection| {
                existing.role == relation.role && existing.kind == relation.kind
            })
        {
            existing.page_size = existing.page_size.min(relation.page_size);
        } else {
            relations.push(relation);
        }
    }
    selection.relations = relations;
    Ok(selection)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationCursor {
    pub schema_version: u16,
    pub source_id: SourceId,
    pub snapshot_revision: SourceRevision,
    pub target: ObjectKey,
    pub relation: RelationKey,
    pub relation_role: RelationRole,
    pub relation_kind: RelationKind,
    pub selection: NavigationSelection,
    pub selection_hash: String,
    pub auth_tag: String,
    pub next_position: u64,
}

impl NavigationCursor {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn issue(
        secret: &[u8],
        source_id: SourceId,
        snapshot_revision: SourceRevision,
        target: ObjectKey,
        relation: RelationGroupRef,
        selection: NavigationSelection,
        next_position: u64,
    ) -> Result<Self, SourceAdapterError> {
        if source_id != relation.source_id || target != relation.owner.object_key {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SourceUnavailable,
                "navigation cursor relation belongs to another source",
            ));
        }
        let selection = normalize_navigation_selection(selection)?;
        let selection_hash = normalized_selection_hash(&selection)?;
        let mut cursor = Self {
            schema_version: Self::SCHEMA_VERSION,
            source_id,
            snapshot_revision,
            target,
            relation: relation.group_key,
            relation_role: relation.role,
            relation_kind: relation.kind,
            selection,
            selection_hash,
            auth_tag: String::new(),
            next_position,
        };
        cursor.auth_tag = cursor_auth_tag(secret, &cursor)?;
        Ok(cursor)
    }

    pub fn resume(&self, current_revision: &SourceRevision) -> Result<(), SourceAdapterError> {
        if &self.snapshot_revision != current_revision {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "navigation cursor snapshot revision is stale",
            ));
        }
        Ok(())
    }

    pub fn decode<F>(
        value: serde_json::Value,
        secret: &[u8],
        current_revision: &SourceRevision,
        expected_selection: &NavigationSelection,
        re_resolve: F,
    ) -> Result<Self, SourceAdapterError>
    where
        F: FnOnce(&SourceId, &ObjectKey, &RelationKey, &RelationRole, &RelationKind) -> bool,
    {
        Self::authenticate(&value, secret)?;
        Self::decode_authenticated(&value, current_revision, expected_selection, re_resolve)
    }

    pub fn authenticate(
        value: &serde_json::Value,
        secret: &[u8],
    ) -> Result<(), SourceAdapterError> {
        let parts = cursor_wire_parts(value)?;
        let tag = hex_decode(parts.auth_tag)?;
        let expected_tag = cursor_mac_parts(
            secret,
            parts.schema_version,
            parts.source_id,
            parts.snapshot_revision,
            parts.target,
            parts.relation,
            parts.relation_role,
            parts.relation_kind,
            parts.selection_hash,
            parts.next_position,
        )?;
        if constant_time_eq(&expected_tag, &tag) {
            Ok(())
        } else {
            Err(SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                "navigation cursor authentication failed",
            ))
        }
    }

    pub fn decode_authenticated<F>(
        value: &serde_json::Value,
        current_revision: &SourceRevision,
        expected_selection: &NavigationSelection,
        re_resolve: F,
    ) -> Result<Self, SourceAdapterError>
    where
        F: FnOnce(&SourceId, &ObjectKey, &RelationKey, &RelationRole, &RelationKind) -> bool,
    {
        let parts = cursor_wire_parts(value)?;
        let _object = value
            .as_object()
            .expect("cursor wire parts require an object");
        let relation_kind = match parts.relation_kind {
            "contains" => RelationKind::Contains,
            "references" => RelationKind::References,
            _ => {
                return Err(SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    "navigation cursor has invalid relationKind",
                ))
            }
        };
        let relation_role = RelationRole::parse(parts.relation_role).map_err(|_| {
            SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                "navigation cursor has invalid relationRole",
            )
        })?;
        let selection_hash = parts.selection_hash.to_string();
        if selection_hash != normalized_selection_hash(expected_selection)? {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::DecodeCorrupted,
                "navigation cursor selectionHash is not normalized",
            ));
        }
        let cursor = Self {
            schema_version: Self::SCHEMA_VERSION,
            source_id: SourceId::new(parts.source_id)?,
            snapshot_revision: SourceRevision::new(parts.snapshot_revision)?,
            target: ObjectKey::new(parts.target)?,
            relation: RelationKey::new(parts.relation)?,
            relation_role,
            relation_kind,
            selection: expected_selection.clone(),
            selection_hash,
            auth_tag: parts.auth_tag.to_string(),
            next_position: parts.next_position,
        };
        cursor.resume(current_revision)?;
        if !re_resolve(
            &cursor.source_id,
            &cursor.target,
            &cursor.relation,
            &cursor.relation_role,
            &cursor.relation_kind,
        ) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::SnapshotStale,
                "navigation cursor target or relation cannot be re-resolved",
            ));
        }
        Ok(cursor)
    }
}

fn cursor_mac(secret: &[u8], cursor: &NavigationCursor) -> Result<[u8; 32], SourceAdapterError> {
    cursor_mac_parts(
        secret,
        cursor.schema_version,
        cursor.source_id.as_str(),
        cursor.snapshot_revision.as_str(),
        cursor.target.as_str(),
        cursor.relation.as_str(),
        relation_role_token(cursor.relation_role),
        relation_kind_token(cursor.relation_kind),
        &cursor.selection_hash,
        cursor.next_position,
    )
}

#[derive(Serialize)]
struct CursorAuthClaims<'a> {
    schema_version: u16,
    source_id: &'a str,
    snapshot_revision: &'a str,
    target: &'a str,
    relation: &'a str,
    relation_role: &'a str,
    relation_kind: &'a str,
    selection_hash: &'a str,
    next_position: u64,
}

fn cursor_mac_parts(
    secret: &[u8],
    schema_version: u16,
    source_id: &str,
    snapshot_revision: &str,
    target: &str,
    relation: &str,
    relation_role: &str,
    relation_kind: &str,
    selection_hash: &str,
    next_position: u64,
) -> Result<[u8; 32], SourceAdapterError> {
    let canonical = serde_json::to_vec(&CursorAuthClaims {
        schema_version,
        source_id,
        snapshot_revision,
        target,
        relation,
        relation_role,
        relation_kind,
        selection_hash,
        next_position,
    })
    .map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("cannot serialize navigation cursor: {error}"),
        )
    })?;
    let mut message =
        Vec::with_capacity(b"unica.navigation.cursor.auth.v2\0".len() + canonical.len());
    message.extend_from_slice(b"unica.navigation.cursor.auth.v2\0");
    message.extend_from_slice(&canonical);
    Ok(hmac_sha256(secret, &message))
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK_BYTES: usize = 64;

    let mut key_block = [0_u8; BLOCK_BYTES];
    if key.len() > BLOCK_BYTES {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut inner_pad = [0x36_u8; BLOCK_BYTES];
    let mut outer_pad = [0x5c_u8; BLOCK_BYTES];
    for index in 0..BLOCK_BYTES {
        inner_pad[index] ^= key_block[index];
        outer_pad[index] ^= key_block[index];
    }

    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(message);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

struct CursorWireParts<'a> {
    schema_version: u16,
    source_id: &'a str,
    snapshot_revision: &'a str,
    target: &'a str,
    relation: &'a str,
    relation_role: &'a str,
    relation_kind: &'a str,
    selection_hash: &'a str,
    auth_tag: &'a str,
    next_position: u64,
}

fn cursor_wire_parts(value: &serde_json::Value) -> Result<CursorWireParts<'_>, SourceAdapterError> {
    let object = value.as_object().ok_or_else(|| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor must be a JSON object",
        )
    })?;
    let allowed = [
        "schemaVersion",
        "sourceId",
        "snapshotRevision",
        "target",
        "relation",
        "relationRole",
        "relationKind",
        "selection",
        "selectionHash",
        "authTag",
        "nextPosition",
    ];
    if object.len() != allowed.len() || object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor has unknown or missing fields",
        ));
    }
    let string = |name: &str| {
        object
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    format!("navigation cursor has no valid {name}"),
                )
            })
    };
    let schema_version = object
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64);
    if schema_version != Some(u64::from(NavigationCursor::SCHEMA_VERSION)) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "unsupported navigation cursor schema version",
        ));
    }
    Ok(CursorWireParts {
        schema_version: NavigationCursor::SCHEMA_VERSION,
        source_id: string("sourceId")?,
        snapshot_revision: string("snapshotRevision")?,
        target: string("target")?,
        relation: string("relation")?,
        relation_role: string("relationRole")?,
        relation_kind: string("relationKind")?,
        selection_hash: string("selectionHash")?,
        auth_tag: string("authTag")?,
        next_position: object
            .get("nextPosition")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    "navigation cursor has no valid nextPosition",
                )
            })?,
    })
}

fn relation_role_token(role: RelationRole) -> &'static str {
    match role {
        RelationRole::Children => "children",
        RelationRole::Attributes => "attributes",
        RelationRole::TabularSections => "tabularSections",
        RelationRole::Forms => "forms",
        RelationRole::Commands => "commands",
        RelationRole::Templates => "templates",
    }
}

fn relation_kind_token(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Contains => "contains",
        RelationKind::References => "references",
    }
}

fn cursor_auth_tag(secret: &[u8], cursor: &NavigationCursor) -> Result<String, SourceAdapterError> {
    Ok(hex_encode(&cursor_mac(secret, cursor)?))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_decode(value: &str) -> Result<Vec<u8>, SourceAdapterError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor has invalid authTag",
        ));
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| {
                SourceAdapterError::new(
                    SourceAdapterErrorKind::DecodeCorrupted,
                    "navigation cursor has invalid authTag",
                )
            })
        })
        .collect()
}

pub fn normalized_selection_hash(
    selection: &NavigationSelection,
) -> Result<String, SourceAdapterError> {
    let normalized = normalize_navigation_selection(selection.clone())?;
    let canonical_json = serde_json::to_vec(&normalized).map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("cannot serialize normalized navigation selection: {error}"),
        )
    })?;
    let mut digest = Sha256::new();
    digest.update(b"unica.navigation.selection.v1\0");
    digest.update(canonical_json);
    Ok(format!("sha256:{:x}", digest.finalize()))
}

fn validate_selection_token(value: &str) -> Result<(), SourceAdapterError> {
    if value.is_empty()
        || value.len() > MAX_NAVIGATION_SELECTOR_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            "selection contains an invalid token",
        ));
    }
    Ok(())
}

fn resource_limit(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor_test_secret() -> &'static [u8] {
        b"navigation-cursor-test-secret-32b"
    }

    fn source_id(raw: &str) -> SourceId {
        SourceId::new(raw).unwrap()
    }
    fn object_key(raw: &str) -> ObjectKey {
        ObjectKey::new(raw).unwrap()
    }
    fn node_ref() -> ObjectRef {
        ObjectRef::new(
            source_id("workspace:main"),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            IdentityStrength::Persistent,
            NodeKind::Document,
            "Order",
        )
    }
    fn relation_group_ref() -> RelationGroupRef {
        RelationGroupRef::new(
            source_id("workspace:main"),
            node_ref(),
            RelationRole::Attributes,
            RelationKind::Contains,
        )
        .unwrap()
    }
    fn selection() -> NavigationSelection {
        NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from(["name".to_string()])),
            facets: FacetSelection::Summary,
            relations: vec![RelationSelection::new("attributes", Some(25)).unwrap()],
        }
    }

    #[test]
    fn uuid_values_normalize_supported_forms_and_reject_misplaced_hyphens() {
        let value = UuidValue::from_str("{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}").unwrap();
        assert_eq!(value.as_str(), "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
        assert!(UuidValue::from_str("aaaaaaaa-bbbbcccc-dddd-eeee-eeeeeeee").is_err());
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_case_one() {
        assert_eq!(
            hex_encode(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn object_identity_is_not_derived_from_display_name_alone() {
        let left = node_ref();
        let renamed = ObjectRef::new(
            source_id("workspace:main"),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            IdentityStrength::Persistent,
            NodeKind::Document,
            "CustomerOrder",
        );
        assert_eq!(left.identity(), renamed.identity());
        assert_eq!(left, renamed);
    }

    #[test]
    fn resolved_authorable_but_format_incompatible_is_not_executable() {
        let capability = CapabilityVector {
            resolution: ResolutionState::Resolved,
            identity: IdentityStrength::Persistent,
            consistency: SnapshotConsistency::Consistent,
            coverage: CoverageState::Complete,
            format: FormatCompatibility::Incompatible,
            source_access: SourceAccess::ReadWrite,
            authorability: Authorability::Authorable,
        };
        assert!(!capability.permits_mutation());
        assert_eq!(
            capability.blocking_reasons(),
            vec![CapabilityBlockReason::FormatIncompatible]
        );
    }

    #[test]
    fn clone_requires_an_explicit_owning_relation() {
        let action = SemanticAction::modeled_clone(node_ref(), None);
        assert_eq!(action.availability, ActionAvailability::Blocked);
        assert_eq!(
            action.blocking_reasons,
            vec![CapabilityBlockReason::OwningRelationMissing]
        );

        let reference_relation = RelationRef::new(
            source_id("workspace:main"),
            "reference:document-owner",
            RelationKind::References,
        )
        .unwrap();
        let wrong_kind = SemanticAction::modeled_clone(node_ref(), Some(reference_relation));
        assert_eq!(wrong_kind.availability, ActionAvailability::Blocked);

        let foreign_relation = RelationRef::new(
            source_id("workspace:other"),
            "contains:foreign-owner",
            RelationKind::Contains,
        )
        .unwrap();
        let wrong_source = SemanticAction::modeled_clone(node_ref(), Some(foreign_relation));
        assert_eq!(wrong_source.availability, ActionAvailability::Blocked);
    }

    #[test]
    fn navigation_envelope_always_has_a_schema_version_and_status() {
        let envelope = NavigationEnvelope::unavailable(SourceAdapterError::new(
            SourceAdapterErrorKind::FormatUnsupported,
            "Platform XML 2.19 has no certified reader",
        ));
        let value = serde_json::to_value(envelope).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            keys,
            BTreeSet::from([
                "diagnostics".to_string(),
                "nodes".to_string(),
                "relations".to_string(),
                "root".to_string(),
                "schemaVersion".to_string(),
                "snapshot".to_string(),
                "status".to_string()
            ])
        );
        assert_eq!(value["schemaVersion"], "1");
        assert_eq!(value["status"], "unavailable");
        assert!(value["snapshot"].is_null());
        assert!(value["root"].is_null());
        assert_eq!(value["nodes"], serde_json::json!([]));
        assert_eq!(value["relations"], serde_json::json!([]));
        assert!(value.get("graph").is_none());
        assert_eq!(value["diagnostics"][0]["code"], "format_unsupported");
    }

    #[test]
    fn properties_preserve_type_value_and_value_state() {
        let property = SemanticProperty::explicit(
            PropertyType::Integer,
            PropertyValue::Integer(11),
            PropertyProvenance::Descriptor,
        )
        .unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["type"], "integer");
        assert_eq!(value["value"], 11);
        assert_eq!(value["valueState"], "explicit");
    }

    #[test]
    fn incompatible_property_type_and_value_are_rejected() {
        let error = SemanticProperty::explicit(
            PropertyType::Integer,
            PropertyValue::String("11".to_string()),
            PropertyProvenance::Descriptor,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn relation_page_size_is_bounded() {
        assert_eq!(
            RelationSelection::new("attributes", None)
                .unwrap()
                .page_size,
            25
        );
        assert!(RelationSelection::new("attributes", Some(101)).is_err());
    }

    #[test]
    fn cursor_is_bound_to_snapshot_revision() {
        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_group_ref(),
            selection(),
            0,
        )
        .unwrap();
        let error = cursor
            .resume(&SourceRevision::new("sha256:two").unwrap())
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::SnapshotStale);
    }

    #[test]
    fn cursor_decode_validates_schema_hash_and_semantic_resolution() {
        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_group_ref(),
            selection(),
            0,
        )
        .unwrap();
        let decoded = NavigationCursor::decode(
            serde_json::to_value(cursor).unwrap(),
            cursor_test_secret(),
            &SourceRevision::new("sha256:one").unwrap(),
            &selection(),
            |_source, _target, _relation, _role, _kind| true,
        )
        .unwrap();
        assert_eq!(decoded.schema_version, NavigationCursor::SCHEMA_VERSION);
    }

    #[test]
    fn cursor_accepts_semantically_identical_selection_with_reordered_keys() {
        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_group_ref(),
            selection(),
            0,
        )
        .unwrap();
        let mut value = serde_json::to_value(cursor).unwrap();
        let selection_fields = value["selection"].as_object_mut().unwrap();
        let mut fields = std::mem::take(selection_fields)
            .into_iter()
            .collect::<Vec<_>>();
        fields.reverse();
        selection_fields.extend(fields);

        let decoded = NavigationCursor::decode(
            value,
            cursor_test_secret(),
            &SourceRevision::new("sha256:one").unwrap(),
            &selection(),
            |_source, _target, _relation, _role, _kind| true,
        )
        .unwrap();

        assert_eq!(decoded.selection, selection());
    }

    #[test]
    fn cursor_rejects_relation_from_another_source_and_preserves_target() {
        let target = object_key("uuid:11111111-1111-1111-1111-111111111111");
        let foreign_relation = RelationGroupRef::new(
            source_id("workspace:other"),
            ObjectRef::new(
                source_id("workspace:other"),
                object_key("uuid:22222222-2222-2222-2222-222222222222"),
                IdentityStrength::Persistent,
                NodeKind::Document,
                "Foreign",
            ),
            RelationRole::Attributes,
            RelationKind::Contains,
        )
        .unwrap();
        let error = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            target.clone(),
            foreign_relation,
            selection(),
            44,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);

        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            target.clone(),
            relation_group_ref(),
            selection(),
            44,
        )
        .unwrap();
        assert_eq!(cursor.target, target);
        assert_eq!(cursor.next_position, 44);
    }

    #[test]
    fn cursor_rejects_well_formed_hash_for_a_different_selection() {
        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_group_ref(),
            selection(),
            0,
        )
        .unwrap();
        let mut value = serde_json::to_value(cursor).unwrap();
        value["selectionHash"] = serde_json::Value::String(format!("sha256:{}", "0".repeat(64)));
        let error = NavigationCursor::decode(
            value,
            cursor_test_secret(),
            &SourceRevision::new("sha256:one").unwrap(),
            &selection(),
            |_source, _target, _relation, _role, _kind| true,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn cursor_hash_is_injective_for_separator_containing_selection_values() {
        let first = NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from(["alpha,beta".to_string()])),
            facets: FacetSelection::Summary,
            relations: vec![RelationSelection::new("attributes", Some(25)).unwrap()],
        };
        let second = NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from([
                "alpha".to_string(),
                "beta".to_string(),
            ])),
            facets: FacetSelection::Summary,
            relations: vec![RelationSelection::new("attributes", Some(25)).unwrap()],
        };
        assert_ne!(
            normalized_selection_hash(&first).unwrap(),
            normalized_selection_hash(&second).unwrap()
        );

        let cursor = NavigationCursor::issue(
            cursor_test_secret(),
            source_id("workspace:main"),
            SourceRevision::new("sha256:one").unwrap(),
            object_key("uuid:11111111-1111-1111-1111-111111111111"),
            relation_group_ref(),
            first,
            0,
        )
        .unwrap();
        let error = NavigationCursor::decode(
            serde_json::to_value(cursor).unwrap(),
            cursor_test_secret(),
            &SourceRevision::new("sha256:one").unwrap(),
            &second,
            |_source, _target, _relation, _role, _kind| true,
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn opaque_keys_reject_path_shaped_values() {
        for value in [
            "",
            "/tmp/object",
            r"\\server\\share",
            "C:\\object",
            "line\nfeed",
        ] {
            assert!(
                ObjectKey::new(value).is_err(),
                "{value:?} must not be an object key"
            );
            assert!(
                RelationKey::new(value).is_err(),
                "{value:?} must not be a relation key"
            );
        }
    }

    #[test]
    fn action_is_executable_only_with_capability_and_binding() {
        let capability = CapabilityVector {
            resolution: ResolutionState::Resolved,
            identity: IdentityStrength::Persistent,
            consistency: SnapshotConsistency::Consistent,
            coverage: CoverageState::Complete,
            format: FormatCompatibility::Compatible,
            source_access: SourceAccess::ReadWrite,
            authorability: Authorability::Authorable,
        };
        let modeled = SemanticAction::mutation(
            SemanticActionKind::EditProperties,
            node_ref(),
            capability.clone(),
            None,
            None,
            Atomicity::SingleFileAtomicReplace,
        );
        assert_eq!(modeled.availability, ActionAvailability::Modeled);
        let executable = SemanticAction::mutation(
            SemanticActionKind::EditProperties,
            node_ref(),
            capability,
            None,
            Some(OperationBinding {
                tool: "unica.meta.edit".to_string(),
                schema_version: "1".to_string(),
            }),
            Atomicity::SingleFileAtomicReplace,
        );
        assert_eq!(executable.availability, ActionAvailability::Executable);

        let invalid = SemanticAction::mutation(
            SemanticActionKind::EditProperties,
            node_ref(),
            CapabilityVector {
                resolution: ResolutionState::Resolved,
                identity: IdentityStrength::Persistent,
                consistency: SnapshotConsistency::Consistent,
                coverage: CoverageState::Complete,
                format: FormatCompatibility::Compatible,
                source_access: SourceAccess::ReadWrite,
                authorability: Authorability::Authorable,
            },
            None,
            Some(OperationBinding {
                tool: "other.meta.edit".to_string(),
                schema_version: "".to_string(),
            }),
            Atomicity::SingleFileAtomicReplace,
        );
        assert_eq!(invalid.availability, ActionAvailability::Blocked);
        assert_eq!(
            invalid.blocking_reasons,
            vec![CapabilityBlockReason::OperationBindingInvalid]
        );
    }

    #[test]
    fn defaulted_property_keeps_a_typed_exact_projector_profile() {
        let property = SemanticProperty::defaulted(
            PropertyType::Integer,
            PropertyValue::Integer(11),
            ProjectorProfile::PlatformXmlV1,
        )
        .unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["valueState"], "defaulted");
        assert_eq!(
            value["provenance"]["projectorDefault"]["profile"],
            "platform_xml_v1"
        );
    }

    #[test]
    fn action_profiles_keep_remove_unadvertised_and_clone_relation_atomic() {
        let actions =
            semantic_actions_for(&NodeKind::Document, CapabilityState::resolved_authorable());
        assert!(actions
            .iter()
            .any(|action| action.action == SemanticActionKind::Clone
                && action.execution_policy == ActionExecutionPolicy::AtomicRelationMutation));
        assert!(!actions
            .iter()
            .any(|action| action.action == SemanticActionKind::Remove));
    }
}
