//! JSON navigation contracts for semantic 1C metadata projections.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use hmac::{Hmac, Mac};
use serde::{
    de::{MapAccess, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use sha2::{Digest, Sha256};
#[cfg(test)]
use uuid::Uuid;

use crate::limits::{
    MAX_NAVIGATION_CURSOR_JSON_BYTES, MAX_NAVIGATION_CURSOR_STRING_BYTES,
    MAX_NAVIGATION_CURSOR_TOKEN_BYTES, MAX_NAVIGATION_NESTING_DEPTH,
    MAX_NAVIGATION_PROPERTY_SELECTORS, MAX_NAVIGATION_RELATION_SELECTORS,
};
use crate::source::{
    SnapshotConsistency, SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceId,
    SourceRevision, SourceSnapshot, TargetIdentity,
};
pub use crate::{
    facets::SemanticFacets,
    property::{
        property_definition, PropertyCapability, PropertyProvenance, PropertyValueState,
        SemanticProperty,
    },
    semantic_ids::{SemanticEnumValue, SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    value::{
        DateFractions, DateQualifiers, NumberQualifiers, NumberSign, PrimitiveTypeKind,
        PropertyType, PropertyValue, SemanticTypeTarget, SemanticValueError, StringLength,
        StringQualifiers, TypeQualifiers, TypeSetValue, TypeVariant,
    },
};

pub type NodeKind = SemanticObjectKind;
pub type RelationRole = SemanticRelationId;

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
    let has_path_separator = raw.bytes().any(|byte| matches!(byte, b'/' | b'\\'));
    if raw.is_empty()
        || raw.chars().any(char::is_control)
        || has_path_separator
        || matches!(raw, "." | "..")
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

impl CoverageState {
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Contains,
    References,
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
        NodeKind::Form => ActionProfile::Form,
        NodeKind::FormElement => ActionProfile::FormElement,
        NodeKind::TabularSection => ActionProfile::TabularSection,
        NodeKind::SpreadsheetDocumentTemplate => ActionProfile::MxlTemplate,
        NodeKind::Template => ActionProfile::UnmodeledTemplate,
        NodeKind::Attribute
        | NodeKind::Dimension
        | NodeKind::Resource
        | NodeKind::Command
        | NodeKind::FormAttribute
        | NodeKind::FormCommand
        | NodeKind::HttpServiceUrlTemplate
        | NodeKind::HttpServiceMethod
        | NodeKind::WebServiceOperation
        | NodeKind::WebServiceParameter
        | NodeKind::EnumerationValue => ActionProfile::UnmodeledChild,
        _ => ActionProfile::GenericMetadataObject,
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
    pub properties: BTreeMap<SemanticPropertyId, SemanticProperty>,
    pub facets: SemanticFacets,
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
            facets: SemanticFacets::default(),
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
            NavigationFacetVisibility::Full => 9,
            NavigationFacetVisibility::Summary => 6,
            NavigationFacetVisibility::None => 3,
        };
        let mut state = serializer.serialize_struct("NavigationNode", fields)?;
        state.serialize_field("objectRef", &self.object_ref)?;
        state.serialize_field("reference", &self.reference)?;
        state.serialize_field("properties", &self.properties)?;
        match self.facet_visibility {
            NavigationFacetVisibility::Full => {
                state.serialize_field("facets", &self.facets)?;
                state.serialize_field("capabilityState", &self.capability_state)?;
                state.serialize_field("capability", &self.capability)?;
                state.serialize_field("actionProfile", &self.action_profile)?;
                state.serialize_field("semanticActions", &self.semantic_actions)?;
                state.serialize_field("actions", &self.actions)?;
            }
            NavigationFacetVisibility::Summary => {
                state.serialize_field("facets", &self.facets.summary())?;
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
    Partial,
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

    pub fn reconcile_partial_coverage(&mut self) {
        if self.status == NavigationStatus::Unavailable {
            return;
        }
        let partial = self.status == NavigationStatus::Partial
            || self.nodes.iter().any(node_has_partial_coverage)
            || self
                .relations
                .iter()
                .flat_map(|page| page.items.iter())
                .any(node_has_partial_coverage)
            || self.relation_index.iter().any(|relation| {
                !relation.capability.coverage.is_complete()
                    || relation.capability.resolution == ResolutionState::Unresolved
            });
        if partial {
            self.mark_partial_coverage();
        }
    }

    pub fn mark_partial_coverage(&mut self) {
        if self.status == NavigationStatus::Unavailable {
            return;
        }
        self.status = NavigationStatus::Partial;
        if !self
            .diagnostics
            .iter()
            .any(SourceAdapterDiagnostic::explains_partial_coverage)
        {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "partialCoverage".to_string(),
                message: "requested semantic coverage is partial".to_string(),
                details: None,
            });
        }
    }
}

impl SourceAdapterDiagnostic {
    pub fn explains_partial_coverage(&self) -> bool {
        matches!(
            self.code.as_str(),
            "partialCoverage" | "unmappedSemanticFact" | "unresolvedSemanticFact"
        )
    }
}

fn node_has_partial_coverage(node: &NavigationNode) -> bool {
    !node.capability.coverage.is_complete()
        || node.capability.resolution == ResolutionState::Unresolved
        || node.properties.values().any(|property| {
            property.value_type() == PropertyType::Unknown
                || property.value_state() == PropertyValueState::Unresolved
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NavigationTarget {
    CapturedTarget(TargetIdentity),
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
    Named(BTreeSet<SemanticPropertyId>),
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
    pub fn new(
        role: SemanticRelationId,
        page_size: Option<u16>,
    ) -> Result<Self, SourceAdapterError> {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationCursor {
    pub schema_version: u16,
    pub source_id: SourceId,
    pub snapshot_revision: SourceRevision,
    pub target_identity: TargetIdentity,
    pub target: ObjectKey,
    pub relation: RelationKey,
    pub relation_role: RelationRole,
    pub relation_kind: RelationKind,
    pub selection: NavigationSelection,
    pub selection_hash: String,
    pub next_position: u64,
    opaque_token: String,
}

impl Serialize for NavigationCursor {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.opaque_token)
    }
}

#[derive(Debug, Clone)]
pub struct OpaqueNavigationCursor(String);

impl OpaqueNavigationCursor {
    pub fn from_token(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn authenticate(&self, secret: &[u8]) -> Result<NavigationCursor, SourceAdapterError> {
        let decoded = decode_cursor_frame(&self.0)?;
        authenticate_cursor_payload(secret, decoded.payload(), decoded.tag())?;
        preflight_cursor_payload_json(decoded.payload())?;
        let value = serde_json::from_slice::<StrictJsonValue>(decoded.payload())
            .map_err(|_| decode_cursor_error("navigation cursor payload JSON is invalid"))?
            .0;
        let parts = cursor_wire_parts(&value)?;
        let selection = decode_cursor_selection(
            value
                .get("selection")
                .ok_or_else(|| decode_cursor_error("navigation cursor has no selection"))?,
        )?;
        if parts.selection_hash != normalized_selection_hash(&selection)? {
            return Err(decode_cursor_error(
                "navigation cursor selectionHash is not normalized",
            ));
        }
        NavigationCursor::decode_claims(&value, &selection, self.0.clone())
    }
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
        let target_identity = TargetIdentity::from_authenticated_value(format!(
            "target:object-key:{}",
            target.as_str()
        ))?;
        Self::issue_bound(
            secret,
            source_id,
            snapshot_revision,
            target_identity,
            target,
            relation,
            selection,
            next_position,
        )
    }

    pub fn issue_bound(
        secret: &[u8],
        source_id: SourceId,
        snapshot_revision: SourceRevision,
        target_identity: TargetIdentity,
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
            target_identity,
            target,
            relation: relation.group_key,
            relation_role: relation.role,
            relation_kind: relation.kind,
            selection,
            selection_hash,
            next_position,
            opaque_token: String::new(),
        };
        cursor.opaque_token = encode_cursor_token(secret, &cursor)?;
        Ok(cursor)
    }

    pub fn opaque(&self) -> OpaqueNavigationCursor {
        OpaqueNavigationCursor::from_token(self.opaque_token.clone())
    }

    pub fn encoded_len(&self) -> usize {
        self.opaque_token.len()
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

    fn decode_claims(
        value: &serde_json::Value,
        expected_selection: &NavigationSelection,
        opaque_token: String,
    ) -> Result<Self, SourceAdapterError> {
        let parts = cursor_wire_parts(value)?;
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
        let relation_role = RelationRole::parse(parts.relation_role).ok_or_else(|| {
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
            target_identity: TargetIdentity::from_authenticated_value(parts.target_identity)?,
            target: ObjectKey::new(parts.target)?,
            relation: RelationKey::new(parts.relation)?,
            relation_role,
            relation_kind,
            selection: expected_selection.clone(),
            selection_hash,
            next_position: parts.next_position,
            opaque_token,
        };
        Ok(cursor)
    }

    pub fn validate_resume<F>(
        self,
        current_revision: &SourceRevision,
        re_resolve: F,
    ) -> Result<Self, SourceAdapterError>
    where
        F: FnOnce(&SourceId, &ObjectKey, &RelationKey, &RelationRole, &RelationKind) -> bool,
    {
        let cursor = self;
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CursorAuthClaims<'a> {
    schema_version: u16,
    source_id: &'a str,
    snapshot_revision: &'a str,
    target_identity: &'a str,
    target: &'a str,
    relation: &'a str,
    relation_role: &'a str,
    relation_kind: &'a str,
    selection: &'a NavigationSelection,
    selection_hash: &'a str,
    next_position: u64,
}

fn cursor_payload(cursor: &NavigationCursor) -> Result<Vec<u8>, SourceAdapterError> {
    let payload = serde_json::to_vec(&CursorAuthClaims {
        schema_version: cursor.schema_version,
        source_id: cursor.source_id.as_str(),
        snapshot_revision: cursor.snapshot_revision.as_str(),
        target_identity: cursor.target_identity.as_str(),
        target: cursor.target.as_str(),
        relation: cursor.relation.as_str(),
        relation_role: relation_role_token(cursor.relation_role),
        relation_kind: relation_kind_token(cursor.relation_kind),
        selection: &cursor.selection,
        selection_hash: &cursor.selection_hash,
        next_position: cursor.next_position,
    })
    .map_err(|error| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::ProjectionAmbiguous,
            format!("cannot serialize navigation cursor: {error}"),
        )
    })?;
    if payload.len() > MAX_NAVIGATION_CURSOR_JSON_BYTES {
        return Err(resource_limit(
            "navigation cursor payload exceeds its JSON byte limit",
        ));
    }
    Ok(payload)
}

fn cursor_payload_mac(secret: &[u8], payload: &[u8]) -> Result<Hmac<Sha256>, SourceAdapterError> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| {
        SourceAdapterError::new(
            SourceAdapterErrorKind::DecodeCorrupted,
            "navigation cursor key is invalid",
        )
    })?;
    mac.update(b"unica.navigation.cursor.payload.v1\0");
    mac.update(payload);
    Ok(mac)
}

const CURSOR_FRAME_MAGIC: &[u8; 4] = b"UNC1";
const CURSOR_TAG_BYTES: usize = 32;
const CURSOR_FRAME_OVERHEAD: usize = CURSOR_FRAME_MAGIC.len() + 4 + CURSOR_TAG_BYTES;

fn encode_cursor_token(
    secret: &[u8],
    cursor: &NavigationCursor,
) -> Result<String, SourceAdapterError> {
    encode_cursor_payload(secret, &cursor_payload(cursor)?)
}

fn encode_cursor_payload(secret: &[u8], payload: &[u8]) -> Result<String, SourceAdapterError> {
    if payload.len() > MAX_NAVIGATION_CURSOR_JSON_BYTES {
        return Err(resource_limit(
            "navigation cursor payload exceeds its JSON byte limit",
        ));
    }
    let payload_len = u32::try_from(payload.len())
        .map_err(|_| resource_limit("navigation cursor payload length overflows its frame"))?;
    let tag = cursor_payload_mac(secret, payload)?.finalize().into_bytes();
    let mut frame = Vec::with_capacity(payload.len() + CURSOR_FRAME_OVERHEAD);
    frame.extend_from_slice(CURSOR_FRAME_MAGIC);
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(payload);
    frame.extend_from_slice(&tag);
    Ok(URL_SAFE_NO_PAD.encode(frame))
}

struct DecodedCursorFrame {
    bytes: Vec<u8>,
    payload_start: usize,
    payload_end: usize,
}

impl DecodedCursorFrame {
    fn payload(&self) -> &[u8] {
        &self.bytes[self.payload_start..self.payload_end]
    }

    fn tag(&self) -> &[u8] {
        &self.bytes[self.payload_end..]
    }
}

fn decode_cursor_frame(token: &str) -> Result<DecodedCursorFrame, SourceAdapterError> {
    preflight_cursor_token(token)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| decode_cursor_error("navigation cursor is not valid base64url"))?;
    if decoded.len() > MAX_NAVIGATION_CURSOR_JSON_BYTES + CURSOR_FRAME_OVERHEAD {
        return Err(resource_limit(
            "decoded navigation cursor exceeds its byte limit",
        ));
    }
    if decoded.len() < CURSOR_FRAME_OVERHEAD
        || decoded.get(..CURSOR_FRAME_MAGIC.len()) != Some(CURSOR_FRAME_MAGIC)
    {
        return Err(decode_cursor_error(
            "navigation cursor has invalid payload framing",
        ));
    }
    let payload_len = u32::from_be_bytes(
        decoded[CURSOR_FRAME_MAGIC.len()..CURSOR_FRAME_MAGIC.len() + 4]
            .try_into()
            .expect("cursor frame length is four bytes"),
    ) as usize;
    if payload_len > MAX_NAVIGATION_CURSOR_JSON_BYTES {
        return Err(resource_limit(
            "navigation cursor payload exceeds its JSON byte limit",
        ));
    }
    let payload_start = CURSOR_FRAME_MAGIC.len() + 4;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or_else(|| resource_limit("navigation cursor payload length overflows its frame"))?;
    if payload_end
        .checked_add(CURSOR_TAG_BYTES)
        .filter(|expected| *expected == decoded.len())
        .is_none()
    {
        return Err(decode_cursor_error(
            "navigation cursor has invalid payload framing",
        ));
    }
    Ok(DecodedCursorFrame {
        bytes: decoded,
        payload_start,
        payload_end,
    })
}

fn preflight_cursor_token(token: &str) -> Result<(), SourceAdapterError> {
    if token.is_empty() {
        return Err(decode_cursor_error("navigation cursor token is empty"));
    }
    if token.len() > MAX_NAVIGATION_CURSOR_TOKEN_BYTES {
        return Err(resource_limit(
            "navigation cursor token exceeds its encoded byte limit",
        ));
    }
    if !token
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(decode_cursor_error(
            "navigation cursor contains invalid base64url characters",
        ));
    }
    Ok(())
}

fn authenticate_cursor_payload(
    secret: &[u8],
    payload: &[u8],
    tag: &[u8],
) -> Result<(), SourceAdapterError> {
    cursor_payload_mac(secret, payload)?
        .verify_slice(tag)
        .map_err(|_| decode_cursor_error("navigation cursor authentication failed"))
}

fn preflight_cursor_payload_json(payload: &[u8]) -> Result<(), SourceAdapterError> {
    if payload.len() > MAX_NAVIGATION_CURSOR_JSON_BYTES {
        return Err(resource_limit(
            "navigation cursor payload exceeds its JSON byte limit",
        ));
    }
    std::str::from_utf8(payload)
        .map_err(|_| decode_cursor_error("navigation cursor payload is not UTF-8"))?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut string_bytes = 0usize;
    for byte in payload {
        if in_string {
            if escaped {
                escaped = false;
                string_bytes += 1;
            } else {
                match byte {
                    b'\\' => {
                        escaped = true;
                        string_bytes += 1;
                    }
                    b'"' => in_string = false,
                    _ => string_bytes += 1,
                }
            }
            if string_bytes > MAX_NAVIGATION_CURSOR_STRING_BYTES {
                return Err(resource_limit(
                    "navigation cursor string exceeds its byte limit",
                ));
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                string_bytes = 0;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth > MAX_NAVIGATION_NESTING_DEPTH {
                    return Err(resource_limit("navigation cursor exceeds nesting limit"));
                }
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    Ok(())
}

struct StrictJsonValue(serde_json::Value);

impl<'de> Deserialize<'de> for StrictJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictJsonValueVisitor)
    }
}

struct StrictJsonValueVisitor;

impl<'de> Visitor<'de> for StrictJsonValueVisitor {
    type Value = StrictJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object fields")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let value =
            serde_json::Number::from_f64(value).ok_or_else(|| E::custom("invalid JSON number"))?;
        Ok(StrictJsonValue(serde_json::Value::Number(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_string(value.to_string())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictJsonValue(serde_json::Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<StrictJsonValue>()? {
            values.push(value.0);
        }
        Ok(StrictJsonValue(serde_json::Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON object field {key}"
                )));
            }
            let value = object.next_value::<StrictJsonValue>()?;
            values.insert(key, value.0);
        }
        Ok(StrictJsonValue(serde_json::Value::Object(values)))
    }
}

fn decode_cursor_selection(
    value: &serde_json::Value,
) -> Result<NavigationSelection, SourceAdapterError> {
    let selection = exact_cursor_object(
        value,
        &["properties", "facets", "relations"],
        "navigation cursor selection",
    )?;
    let properties = match selection.get("properties") {
        Some(serde_json::Value::String(value)) if value == "all" => PropertySelection::All,
        Some(value) => {
            let named =
                exact_cursor_object(value, &["named"], "navigation cursor property selection")?;
            let names = named
                .get("named")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| {
                    decode_cursor_error("navigation cursor has invalid property selection")
                })?;
            let mut unique = BTreeSet::new();
            for name in names {
                let name = name.as_str().ok_or_else(|| {
                    decode_cursor_error("navigation cursor has invalid property selection")
                })?;
                let id = SemanticPropertyId::parse(name).ok_or_else(|| {
                    decode_cursor_error("navigation cursor has invalid property selection")
                })?;
                if !unique.insert(id) {
                    return Err(decode_cursor_error(
                        "navigation cursor has repeated property selection",
                    ));
                }
            }
            PropertySelection::Named(unique)
        }
        None => {
            return Err(decode_cursor_error(
                "navigation cursor has no property selection",
            ))
        }
    };
    let facets = match selection.get("facets").and_then(serde_json::Value::as_str) {
        Some("none") => FacetSelection::None,
        Some("summary") => FacetSelection::Summary,
        Some("full") => FacetSelection::Full,
        _ => return Err(decode_cursor_error("navigation cursor has invalid facets")),
    };
    let relation_values = selection
        .get("relations")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| decode_cursor_error("navigation cursor has invalid relations"))?;
    let mut relations = Vec::with_capacity(relation_values.len());
    for value in relation_values {
        let relation = exact_cursor_object(
            value,
            &["kind", "role", "pageSize"],
            "navigation cursor relation selection",
        )?;
        let kind = match relation.get("kind").and_then(serde_json::Value::as_str) {
            Some("contains") => RelationKind::Contains,
            Some("references") => RelationKind::References,
            _ => {
                return Err(decode_cursor_error(
                    "navigation cursor has invalid relation kind",
                ))
            }
        };
        let role = relation
            .get("role")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| decode_cursor_error("navigation cursor has invalid relation role"))
            .and_then(|role| {
                SemanticRelationId::parse(role).ok_or_else(|| {
                    decode_cursor_error("navigation cursor has invalid relation role")
                })
            })?;
        let page_size = relation
            .get("pageSize")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .filter(|value| (1..=100).contains(value))
            .ok_or_else(|| {
                decode_cursor_error("navigation cursor has invalid relation page size")
            })?;
        relations.push(RelationSelection {
            kind,
            role,
            page_size,
        });
    }
    normalize_navigation_selection(NavigationSelection {
        properties,
        facets,
        relations,
    })
    .map_err(|error| match error.kind {
        SourceAdapterErrorKind::ResourceLimit => error,
        _ => decode_cursor_error("navigation cursor has invalid selection"),
    })
}

fn exact_cursor_object<'a>(
    value: &'a serde_json::Value,
    fields: &[&str],
    name: &str,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, SourceAdapterError> {
    let object = value
        .as_object()
        .ok_or_else(|| decode_cursor_error(&format!("{name} must be a JSON object")))?;
    if object.len() != fields.len() || object.keys().any(|field| !fields.contains(&field.as_str()))
    {
        return Err(decode_cursor_error(&format!(
            "{name} has unknown or missing fields"
        )));
    }
    Ok(object)
}

struct CursorWireParts<'a> {
    source_id: &'a str,
    snapshot_revision: &'a str,
    target_identity: &'a str,
    target: &'a str,
    relation: &'a str,
    relation_role: &'a str,
    relation_kind: &'a str,
    selection_hash: &'a str,
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
        "targetIdentity",
        "target",
        "relation",
        "relationRole",
        "relationKind",
        "selection",
        "selectionHash",
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
        source_id: string("sourceId")?,
        snapshot_revision: string("snapshotRevision")?,
        target_identity: string("targetIdentity")?,
        target: string("target")?,
        relation: string("relation")?,
        relation_role: string("relationRole")?,
        relation_kind: string("relationKind")?,
        selection_hash: string("selectionHash")?,
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
    role.as_str()
}

fn relation_kind_token(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::Contains => "contains",
        RelationKind::References => "references",
    }
}

fn decode_cursor_error(message: &str) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::DecodeCorrupted, message)
}

#[cfg(test)]
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
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
            properties: PropertySelection::Named(BTreeSet::from([
                SemanticPropertyId::METADATA_NAME,
            ])),
            facets: FacetSelection::Summary,
            relations: vec![
                RelationSelection::new(SemanticRelationId::ATTRIBUTES, Some(25)).unwrap(),
            ],
        }
    }

    #[test]
    fn uuid_values_use_mature_parsing_and_preserve_canonical_serialization() {
        let value = "{AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE}"
            .parse::<Uuid>()
            .unwrap();
        assert_eq!(
            serde_json::to_value(PropertyValue::Uuid(value)).unwrap(),
            serde_json::json!({
                "type": "uuid",
                "value": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
            })
        );
        assert!("aaaaaaaa-bbbbcccc-dddd-eeee-eeeeeeee"
            .parse::<Uuid>()
            .is_err());
    }

    #[test]
    fn hmac_sha256_matches_rfc_4231_case_one() {
        let mut mac = Hmac::<Sha256>::new_from_slice(&[0x0b; 20]).unwrap();
        mac.update(b"Hi There");
        assert_eq!(
            hex_encode(&mac.finalize().into_bytes()),
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
            SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
            PropertyValue::Integer(11),
        )
        .unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["type"], "integer");
        assert_eq!(
            value["value"],
            serde_json::json!({"type": "integer", "value": 11})
        );
        assert_eq!(value["valueState"], "explicit");
    }

    #[test]
    fn incompatible_property_type_and_value_are_rejected() {
        let error = SemanticProperty::explicit(
            SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
            PropertyValue::String("11".to_string()),
        )
        .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
    }

    #[test]
    fn relation_page_size_is_bounded() {
        assert_eq!(
            RelationSelection::new(SemanticRelationId::ATTRIBUTES, None)
                .unwrap()
                .page_size,
            25
        );
        assert!(RelationSelection::new(SemanticRelationId::ATTRIBUTES, Some(101)).is_err());
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
    fn cursor_serializes_as_a_non_empty_opaque_string() {
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
        let value = serde_json::to_value(cursor).unwrap();
        assert!(value.as_str().is_some_and(|token| !token.is_empty()));
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
        let decoded = cursor
            .opaque()
            .authenticate(cursor_test_secret())
            .unwrap()
            .validate_resume(
                &SourceRevision::new("sha256:one").unwrap(),
                |_source, _target, _relation, _role, _kind| true,
            )
            .unwrap();
        assert_eq!(decoded.schema_version, NavigationCursor::SCHEMA_VERSION);
    }

    fn cursor_token(cursor: &NavigationCursor) -> String {
        serde_json::to_value(cursor)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    }

    fn framed_test_token(payload: &[u8], tag: &[u8]) -> String {
        let mut frame = Vec::new();
        frame.extend_from_slice(CURSOR_FRAME_MAGIC);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        frame.extend_from_slice(tag);
        URL_SAFE_NO_PAD.encode(frame)
    }

    fn token_with_original_tag(cursor: &NavigationCursor, payload: &[u8]) -> String {
        let original = decode_cursor_frame(&cursor_token(cursor)).unwrap();
        framed_test_token(payload, original.tag())
    }

    #[test]
    fn cursor_accepts_authenticated_selection_with_reordered_object_keys() {
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
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&cursor_payload(&cursor).unwrap()).unwrap();
        let selection_fields = value["selection"].as_object_mut().unwrap();
        let mut fields = std::mem::take(selection_fields)
            .into_iter()
            .collect::<Vec<_>>();
        fields.reverse();
        selection_fields.extend(fields);
        let payload = serde_json::to_vec(&value).unwrap();
        let token = encode_cursor_payload(cursor_test_secret(), &payload).unwrap();

        let decoded = OpaqueNavigationCursor::from_token(token)
            .authenticate(cursor_test_secret())
            .unwrap();

        assert_eq!(decoded.selection, selection());
    }

    #[test]
    fn cursor_rejects_unknown_fields_at_every_selection_nesting_level() {
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

        for path in ["selection", "properties", "relation"] {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&cursor_payload(&cursor).unwrap())
                    .unwrap();
            match path {
                "selection" => value["selection"]["unknown"] = serde_json::json!(true),
                "properties" => value["selection"]["properties"]["unknown"] = serde_json::json!([]),
                "relation" => {
                    value["selection"]["relations"][0]["unknown"] = serde_json::json!(true)
                }
                _ => unreachable!(),
            }
            let payload = serde_json::to_vec(&value).unwrap();

            for token in [
                token_with_original_tag(&cursor, &payload),
                encode_cursor_payload(cursor_test_secret(), &payload).unwrap(),
            ] {
                let error = OpaqueNavigationCursor::from_token(token)
                    .authenticate(cursor_test_secret())
                    .unwrap_err();
                assert_eq!(
                    error.kind,
                    SourceAdapterErrorKind::DecodeCorrupted,
                    "{path} unknown field must fail closed"
                );
            }
        }
    }

    #[test]
    fn cursor_rejects_duplicate_named_object_fields_with_original_or_valid_tag() {
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
        let raw = String::from_utf8(cursor_payload(&cursor).unwrap()).unwrap();
        let tampered = raw.replacen(
            "\"named\":[\"metadata.name\"]",
            "\"named\":[\"metadata.name\"],\"named\":[\"metadata.name\"]",
            1,
        );
        assert_ne!(tampered, raw);

        for token in [
            token_with_original_tag(&cursor, tampered.as_bytes()),
            encode_cursor_payload(cursor_test_secret(), tampered.as_bytes()).unwrap(),
        ] {
            let error = OpaqueNavigationCursor::from_token(token)
                .authenticate(cursor_test_secret())
                .unwrap_err();
            assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
        }
    }

    #[test]
    fn cursor_preserves_authenticated_relation_array_repetition_normalization() {
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
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&cursor_payload(&cursor).unwrap()).unwrap();
        let repeated = value["selection"]["relations"][0].clone();
        value["selection"]["relations"]
            .as_array_mut()
            .unwrap()
            .push(repeated);
        let payload = serde_json::to_vec(&value).unwrap();
        let token = encode_cursor_payload(cursor_test_secret(), &payload).unwrap();

        let decoded = OpaqueNavigationCursor::from_token(token)
            .authenticate(cursor_test_secret())
            .unwrap();
        assert_eq!(decoded.selection, cursor.selection);
    }

    #[test]
    fn cursor_token_preflight_and_payload_resource_order_fail_closed() {
        for token in ["", "not+base64url", "A"] {
            let error = OpaqueNavigationCursor::from_token(token)
                .authenticate(cursor_test_secret())
                .unwrap_err();
            assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
        }
        let error =
            OpaqueNavigationCursor::from_token("A".repeat(MAX_NAVIGATION_CURSOR_TOKEN_BYTES + 1))
                .authenticate(cursor_test_secret())
                .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);

        let mut oversized_frame = Vec::new();
        oversized_frame.extend_from_slice(CURSOR_FRAME_MAGIC);
        oversized_frame
            .extend_from_slice(&((MAX_NAVIGATION_CURSOR_JSON_BYTES + 1) as u32).to_be_bytes());
        oversized_frame.extend_from_slice(&[0; CURSOR_TAG_BYTES]);
        let error = OpaqueNavigationCursor::from_token(URL_SAFE_NO_PAD.encode(oversized_frame))
            .authenticate(cursor_test_secret())
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);

        let nested = format!(
            "{}0{}",
            "[".repeat(MAX_NAVIGATION_NESTING_DEPTH + 1),
            "]".repeat(MAX_NAVIGATION_NESTING_DEPTH + 1)
        );
        let token = encode_cursor_payload(cursor_test_secret(), nested.as_bytes()).unwrap();
        let error = OpaqueNavigationCursor::from_token(token)
            .authenticate(cursor_test_secret())
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn cursor_authenticates_exact_payload_bytes_before_private_json_parsing() {
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
        let deeply_nested = format!(
            "{}0{}",
            "[".repeat(MAX_NAVIGATION_NESTING_DEPTH + 1),
            "]".repeat(MAX_NAVIGATION_NESTING_DEPTH + 1)
        );
        let token = token_with_original_tag(&cursor, deeply_nested.as_bytes());
        let error = OpaqueNavigationCursor::from_token(token)
            .authenticate(cursor_test_secret())
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
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
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&cursor_payload(&cursor).unwrap()).unwrap();
        value["selectionHash"] = serde_json::Value::String(format!("sha256:{}", "0".repeat(64)));
        let payload = serde_json::to_vec(&value).unwrap();
        let token = encode_cursor_payload(cursor_test_secret(), &payload).unwrap();
        let error = OpaqueNavigationCursor::from_token(token)
            .authenticate(cursor_test_secret())
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    }

    #[test]
    fn cursor_hash_distinguishes_registered_property_selections() {
        let first = NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from([
                SemanticPropertyId::METADATA_NAME,
                SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
            ])),
            facets: FacetSelection::Summary,
            relations: vec![
                RelationSelection::new(SemanticRelationId::ATTRIBUTES, Some(25)).unwrap(),
            ],
        };
        let second = NavigationSelection {
            properties: PropertySelection::Named(BTreeSet::from([
                SemanticPropertyId::METADATA_NAME,
                SemanticPropertyId::METADATA_SYNONYM,
            ])),
            facets: FacetSelection::Summary,
            relations: vec![
                RelationSelection::new(SemanticRelationId::ATTRIBUTES, Some(25)).unwrap(),
            ],
        };
        assert_ne!(
            normalized_selection_hash(&first).unwrap(),
            normalized_selection_hash(&second).unwrap()
        );
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
    fn opaque_keys_reject_path_shaped_values() {
        for value in [
            "",
            "/tmp/object",
            r"\\server\\share",
            "C:\\object",
            "Catalogs/Items.xml",
            "../object",
            "./object",
            r"dir\object",
            ".",
            "..",
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
        for value in [
            "uuid:11111111-1111-1111-1111-111111111111",
            "group:sha256:abc",
            "reference:document-owner",
        ] {
            assert!(
                ObjectKey::new(value).is_ok(),
                "{value:?} must remain a valid object key"
            );
            assert!(
                RelationKey::new(value).is_ok(),
                "{value:?} must remain a valid relation key"
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
            SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
            PropertyValue::Integer(11),
        )
        .unwrap();
        let value = serde_json::to_value(property).unwrap();
        assert_eq!(value["valueState"], "defaulted");
        assert_eq!(value["provenance"], "default");
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
