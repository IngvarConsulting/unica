//! Prototype ontology for navigating 1C metadata as semantic objects.

use serde::Serialize;

/// A logical reference to a metadata object or a semantic child of one.
///
/// `ObjectRef` is deliberately independent of XML paths, filenames, and other
/// storage details. Its identity is exactly the source set, owner chain, node
/// kind, and name. This lets a client keep a stable semantic target while the
/// representation changes from platform XML to EDT or another adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectRef {
    /// Name of the configured source set that owns this logical object.
    pub source_set: String,
    /// Logical ancestors, ordered from the top-level owner to the immediate owner.
    pub owner_chain: Vec<OwnerSegment>,
    /// Semantic class of the referenced object.
    pub kind: NodeKind,
    /// Object name within its owner scope.
    pub name: String,
}

impl ObjectRef {
    pub fn new(
        source_set: impl Into<String>,
        owner_chain: Vec<OwnerSegment>,
        kind: NodeKind,
        name: impl Into<String>,
    ) -> Self {
        Self {
            source_set: source_set.into(),
            owner_chain,
            kind,
            name: name.into(),
        }
    }
}

/// One logical ancestor in [`ObjectRef::owner_chain`].
///
/// The source set is inherited from the enclosing [`ObjectRef`], so it is not
/// repeated here. Like [`ObjectRef`], this contains no representation path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerSegment {
    pub kind: NodeKind,
    pub name: String,
}

impl OwnerSegment {
    pub fn new(kind: NodeKind, name: impl Into<String>) -> Self {
        Self {
            kind,
            name: name.into(),
        }
    }
}

/// Semantic class of a graph node.
///
/// `MetadataObject` deliberately stores the platform metadata type as data
/// instead of duplicating the canonical metadata-kind registry in the domain
/// layer. Adapters validate that type against their source-specific registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum NodeKind {
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

/// The physical representation from which this graph was projected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Representation {
    PlatformXml,
    Edt,
}

/// Whether the adapter resolved a semantic target to a backing source object.
///
/// This describes source resolution only. Whether that resolved source may be
/// authored is represented separately by [`Authorability`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    /// The adapter found the backing source object.
    Resolved,
    /// The parent declares the target, but its backing source object is absent.
    Unresolved,
}

/// Whether an independently resolved aggregate may be authored in this scope.
///
/// Read-only authorability is explicit rather than inferred from source
/// resolution. A source can be present but locked by support or the active
/// configuration, and such a source must not advertise mutation semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Authorability {
    Authorable,
    SupportLocked,
    ConfigurationReadOnly,
    UnknownReadOnly,
    DerivedReadOnly,
}

/// Source resolution and authorability needed to derive action capabilities.
///
/// A node or relation is mutable only when the source is both resolved and
/// explicitly [`Authorability::Authorable`]. This state is deliberately shared
/// by node and relation models so a resolved reference cannot bypass a support
/// or configuration lock through a relation-level mutation.
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

/// A semantic relationship between two graph nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Contains,
    References,
}

/// A semantic user intention, independent of any current MCP tool.
///
/// These are ontology/prototype declarations. They are not tool names, do not
/// expose a direct script path, and do not promise that an implementation is
/// already callable. A later capability-to-tool mapping may make a modeled
/// action executable for a particular representation and node instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticAction {
    Inspect,
    EditProperties,
    /// Creates a new sibling through the source node's owning `contains` or
    /// registration relation. The action is discovered on its source node, but
    /// its mutation target is that relation plus sibling creation, so it must
    /// use [`ActionExecutionPolicy::AtomicRelationMutation`].
    Clone,
    /// Reserved for a future profile that carries action-specific support
    /// removal eligibility. `Authorable` alone is insufficient: native
    /// removal requires the target to be off support/removed from support.
    /// The current prototype intentionally never advertises this action.
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
    /// Creates or updates the handler in the enclosing form's module. It is
    /// exposed on a form element, but its mutation target is the one semantic
    /// `Form` aggregate (form source, module, and children). Therefore this is
    /// an [`ActionExecutionPolicy::AtomicNodeMutation`], not an element plus
    /// unrelated-module cross-action.
    CreateHandler,
    EditMxl,
}

/// Transaction contract of a modeled semantic action.
///
/// Every mutation is independently atomic at exactly one node or relation
/// aggregate. Implementations must validate the affected aggregate before the
/// commit and never rely on a cross-action changeset. This is a model contract,
/// not a claim about the availability of a transport endpoint.
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

/// A modeled action paired with its atomicity contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticActionDescriptor {
    pub action: SemanticAction,
    pub execution_policy: ActionExecutionPolicy,
}

impl SemanticActionDescriptor {
    const fn read(action: SemanticAction) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::ReadOnly,
        }
    }

    const fn node_mutation(action: SemanticAction) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::AtomicNodeMutation,
        }
    }

    const fn relation_mutation(action: SemanticAction) -> Self {
        Self {
            action,
            execution_policy: ActionExecutionPolicy::AtomicRelationMutation,
        }
    }
}

/// Explicit semantic-action coverage modeled by this prototype.
///
/// A `GenericMetadataObject` intentionally exposes only inspection. The
/// adapter must opt into a named profile before the model advertises any
/// mutation, so an unknown metadata type never inherits `Document` actions by
/// accident. This is serialized on every [`NavigationNode`] to make the
/// prototype boundary visible to graph consumers.
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

/// Return the explicit semantic-action coverage profile for a node class.
pub fn action_profile_for(kind: &NodeKind) -> ActionProfile {
    match kind {
        NodeKind::MetadataObject { metadata_type } if metadata_type == "Document" => {
            ActionProfile::DocumentMetadataObject
        }
        NodeKind::MetadataObject { .. } => ActionProfile::GenericMetadataObject,
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

/// Return modeled semantic actions for a node class in its current state.
///
/// The returned descriptors are a pure ontology decision. They deliberately do
/// not consult the MCP registry or advertise a callable tool. Unresolved or
/// non-authorable nodes retain only inspection: exposing a mutation for either
/// state would falsely promise an authorable aggregate.
pub fn semantic_actions_for(
    kind: &NodeKind,
    capability_state: CapabilityState,
) -> Vec<SemanticActionDescriptor> {
    if !capability_state.is_resolved_authorable() {
        return vec![SemanticActionDescriptor::read(SemanticAction::Inspect)];
    }

    match action_profile_for(kind) {
        ActionProfile::DocumentMetadataObject => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticAction::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Clone),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddAttribute),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddTabularSection),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddForm),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddMxl),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddCommand),
        ],
        ActionProfile::Form => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticAction::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Clone),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddFormAttribute),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddFormCommand),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddFormElement),
        ],
        ActionProfile::FormElement => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticAction::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Clone),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddFormElement),
            SemanticActionDescriptor::node_mutation(SemanticAction::CreateHandler),
        ],
        ActionProfile::TabularSection => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticAction::EditProperties),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Clone),
            SemanticActionDescriptor::node_mutation(SemanticAction::AddAttribute),
        ],
        ActionProfile::MxlTemplate => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::node_mutation(SemanticAction::EditMxl),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Clone),
        ],
        ActionProfile::GenericMetadataObject
        | ActionProfile::UnmodeledTemplate
        | ActionProfile::UnmodeledChild => {
            vec![SemanticActionDescriptor::read(SemanticAction::Inspect)]
        }
    }
}

/// Return modeled actions for an existing semantic relation.
///
/// Relation actions are intentionally not exposed through the endpoint node:
/// moving containment and changing a binding must mutate the relation
/// aggregate, not the serialized representation of either endpoint. An
/// unresolved or read-only relation is always inspection-only.
pub fn semantic_actions_for_relation(
    from_kind: &NodeKind,
    to_kind: &NodeKind,
    relation: RelationKind,
    capability_state: CapabilityState,
) -> Vec<SemanticActionDescriptor> {
    if !capability_state.is_resolved_authorable() {
        return vec![SemanticActionDescriptor::read(SemanticAction::Inspect)];
    }

    match (relation, from_kind, to_kind) {
        (RelationKind::Contains, NodeKind::Form, NodeKind::FormElement) => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticAction::Move),
        ],
        (
            RelationKind::References,
            NodeKind::FormElement,
            NodeKind::Attribute | NodeKind::FormAttribute,
        ) => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticAction::BindData),
            SemanticActionDescriptor::relation_mutation(SemanticAction::RebindData),
            SemanticActionDescriptor::relation_mutation(SemanticAction::UnbindData),
        ],
        (
            RelationKind::References,
            NodeKind::FormElement,
            NodeKind::Command | NodeKind::FormCommand,
        ) => vec![
            SemanticActionDescriptor::read(SemanticAction::Inspect),
            SemanticActionDescriptor::relation_mutation(SemanticAction::BindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticAction::RebindCommand),
            SemanticActionDescriptor::relation_mutation(SemanticAction::UnbindCommand),
        ],
        _ => vec![SemanticActionDescriptor::read(SemanticAction::Inspect)],
    }
}

/// One semantic node and its currently modeled actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NavigationNode {
    pub reference: ObjectRef,
    pub capability_state: CapabilityState,
    pub action_profile: ActionProfile,
    semantic_actions: Vec<SemanticActionDescriptor>,
}

impl NavigationNode {
    pub fn new(reference: ObjectRef, capability_state: CapabilityState) -> Self {
        let action_profile = action_profile_for(&reference.kind);
        let semantic_actions = semantic_actions_for(&reference.kind, capability_state);
        Self {
            reference,
            capability_state,
            action_profile,
            semantic_actions,
        }
    }

    pub fn semantic_actions(&self) -> &[SemanticActionDescriptor] {
        &self.semantic_actions
    }
}

/// A directed semantic relationship in a navigation graph.
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

/// Declares how `semanticActions` in a graph must be interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSemantics {
    ModeledCapabilities,
}

/// A representation-specific projection of semantic metadata nodes and links.
///
/// The model is intentionally marked as a prototype. `semanticActions` are
/// modeled capabilities with an atomicity contract, not an assertion that a
/// corresponding public command exists today.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
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

#[cfg(test)]
mod tests {
    use super::{
        action_profile_for, semantic_actions_for, semantic_actions_for_relation,
        ActionExecutionPolicy, ActionProfile, ActionSemantics, Authorability, CapabilityState,
        NavigationEdge, NavigationGraph, NavigationNode, NodeKind, ObjectRef, OwnerSegment,
        RelationKind, Representation, ResolutionState, SemanticAction,
    };
    use serde_json::Value;

    fn document_reference() -> ObjectRef {
        ObjectRef::new(
            "main",
            Vec::new(),
            NodeKind::metadata_object("Document"),
            "Order",
        )
    }

    fn resolved_authorable() -> CapabilityState {
        CapabilityState::resolved_authorable()
    }

    fn action_is_present(
        actions: &[super::SemanticActionDescriptor],
        action: SemanticAction,
        policy: ActionExecutionPolicy,
    ) -> bool {
        actions
            .iter()
            .any(|descriptor| descriptor.action == action && descriptor.execution_policy == policy)
    }

    #[test]
    fn object_reference_identity_is_logical_and_path_free() {
        let reference = ObjectRef::new(
            "main",
            vec![OwnerSegment::new(
                NodeKind::metadata_object("Document"),
                "Order",
            )],
            NodeKind::Form,
            "ItemForm",
        );
        let same_identity = ObjectRef::new(
            "main",
            vec![OwnerSegment::new(
                NodeKind::metadata_object("Document"),
                "Order",
            )],
            NodeKind::Form,
            "ItemForm",
        );
        let another_owner = ObjectRef::new(
            "main",
            vec![OwnerSegment::new(
                NodeKind::metadata_object("Document"),
                "Invoice",
            )],
            NodeKind::Form,
            "ItemForm",
        );

        assert_eq!(reference, same_identity);
        assert_ne!(reference, another_owner);

        let serialized = serde_json::to_value(&reference).expect("reference is serializable");
        assert_eq!(serialized["sourceSet"], "main");
        assert_eq!(serialized["name"], "ItemForm");
        assert!(serialized.get("ownerChain").is_some());
        assert!(!contains_key(&serialized, "path"));
        assert!(!contains_key(&serialized, "xmlPath"));
    }

    #[test]
    fn semantic_actions_are_specific_to_object_form_and_form_element() {
        let object_actions = semantic_actions_for(
            &NodeKind::metadata_object("Document"),
            resolved_authorable(),
        );
        assert!(action_is_present(
            &object_actions,
            SemanticAction::AddAttribute,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        assert!(action_is_present(
            &object_actions,
            SemanticAction::AddForm,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        assert!(!object_actions
            .iter()
            .any(|descriptor| descriptor.action == SemanticAction::AddFormElement));
        assert_eq!(
            action_profile_for(&NodeKind::metadata_object("Document")),
            ActionProfile::DocumentMetadataObject
        );

        let generic_object_actions = semantic_actions_for(
            &NodeKind::metadata_object("Configuration"),
            resolved_authorable(),
        );
        assert_eq!(
            action_profile_for(&NodeKind::metadata_object("Configuration")),
            ActionProfile::GenericMetadataObject
        );
        assert!(!generic_object_actions
            .iter()
            .any(|descriptor| descriptor.action == SemanticAction::AddAttribute));
        assert!(!generic_object_actions
            .iter()
            .any(|descriptor| descriptor.action == SemanticAction::AddForm));
        assert_eq!(generic_object_actions.len(), 1);
        assert!(action_is_present(
            &generic_object_actions,
            SemanticAction::Inspect,
            ActionExecutionPolicy::ReadOnly,
        ));
        assert!(generic_object_actions
            .iter()
            .all(|descriptor| !descriptor.execution_policy.is_mutation()));

        let form_actions = semantic_actions_for(&NodeKind::Form, resolved_authorable());
        assert!(action_is_present(
            &form_actions,
            SemanticAction::AddFormAttribute,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        assert!(action_is_present(
            &form_actions,
            SemanticAction::AddFormElement,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        assert!(!form_actions
            .iter()
            .any(|descriptor| descriptor.action == SemanticAction::AddAttribute));

        let element_actions = semantic_actions_for(&NodeKind::FormElement, resolved_authorable());
        assert!(action_is_present(
            &element_actions,
            SemanticAction::CreateHandler,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        assert!(!element_actions
            .iter()
            .any(|descriptor| descriptor.action == SemanticAction::AddFormAttribute));
    }

    #[test]
    fn unresolved_and_read_only_nodes_expose_only_read_semantics() {
        for state in [
            CapabilityState::new(ResolutionState::Unresolved, Authorability::Authorable),
            CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
        ] {
            let actions = semantic_actions_for(&NodeKind::FormElement, state);

            assert_eq!(actions.len(), 1);
            assert!(action_is_present(
                &actions,
                SemanticAction::Inspect,
                ActionExecutionPolicy::ReadOnly,
            ));
            assert!(actions
                .iter()
                .all(|descriptor| !descriptor.execution_policy.is_mutation()));
        }
    }

    #[test]
    fn graph_serializes_modeled_actions_and_atomicity_contract() {
        let document = document_reference();
        let form = ObjectRef::new(
            "main",
            vec![OwnerSegment::new(
                document.kind.clone(),
                document.name.clone(),
            )],
            NodeKind::Form,
            "ItemForm",
        );
        let graph = NavigationGraph::new(
            Representation::PlatformXml,
            document.clone(),
            vec![
                NavigationNode::new(document.clone(), resolved_authorable()),
                NavigationNode::new(form.clone(), resolved_authorable()),
            ],
            vec![
                NavigationEdge::new(
                    document.clone(),
                    form.clone(),
                    RelationKind::Contains,
                    resolved_authorable(),
                ),
                NavigationEdge::new(
                    form,
                    document,
                    RelationKind::References,
                    resolved_authorable(),
                ),
            ],
        );

        assert!(graph.is_prototype());
        assert_eq!(
            graph.action_semantics(),
            ActionSemantics::ModeledCapabilities
        );

        let edit = graph.nodes[0]
            .semantic_actions()
            .iter()
            .find(|descriptor| descriptor.action == SemanticAction::EditProperties)
            .expect("resolved object has an edit semantic action");
        assert_eq!(
            edit.execution_policy,
            ActionExecutionPolicy::AtomicNodeMutation
        );
        assert!(edit.execution_policy.validates_before_commit());
        assert!(!edit.execution_policy.allows_cross_action_changeset());

        let serialized = serde_json::to_value(&graph).expect("graph is serializable");
        assert_eq!(serialized["prototype"], true);
        assert_eq!(serialized["actionSemantics"], "modeled_capabilities");
        assert_eq!(serialized["representation"], "platform_xml");
        assert_eq!(
            serialized["nodes"][0]["actionProfile"],
            "document_metadata_object"
        );
        assert_eq!(serialized["edges"][0]["relation"], "contains");
        assert_eq!(serialized["edges"][1]["relation"], "references");
        assert_eq!(
            serialized["edges"][0]["capabilityState"]["authorability"],
            "authorable"
        );
        assert!(serialized["edges"][0]["semanticActions"]
            .as_array()
            .expect("edge actions are included in the graph output")
            .iter()
            .any(|action| action["action"] == "inspect"));
        assert!(serialized["nodes"][0]["semanticActions"]
            .as_array()
            .expect("actions are included in the graph output")
            .iter()
            .any(|action| {
                action["action"] == "add_attribute"
                    && action["executionPolicy"] == "atomic_node_mutation"
            }));
    }

    #[test]
    fn node_mutations_require_both_resolved_source_and_authorability() {
        let document_kind = NodeKind::metadata_object("Document");
        let authorable = CapabilityState::resolved_authorable();
        let unresolved =
            CapabilityState::new(ResolutionState::Unresolved, Authorability::Authorable);

        assert!(action_is_present(
            &semantic_actions_for(&document_kind, authorable),
            SemanticAction::AddAttribute,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));
        for state in [
            CapabilityState::new(ResolutionState::Resolved, Authorability::SupportLocked),
            CapabilityState::new(
                ResolutionState::Resolved,
                Authorability::ConfigurationReadOnly,
            ),
            CapabilityState::new(ResolutionState::Resolved, Authorability::UnknownReadOnly),
            CapabilityState::new(ResolutionState::Resolved, Authorability::DerivedReadOnly),
            unresolved,
        ] {
            let actions = semantic_actions_for(&document_kind, state);
            assert_eq!(actions.len(), 1);
            assert!(action_is_present(
                &actions,
                SemanticAction::Inspect,
                ActionExecutionPolicy::ReadOnly,
            ));
        }

        let node = NavigationNode::new(
            document_reference(),
            CapabilityState::new(ResolutionState::Resolved, Authorability::SupportLocked),
        );
        let serialized = serde_json::to_value(node).expect("node is serializable");
        assert_eq!(serialized["capabilityState"]["resolutionState"], "resolved");
        assert_eq!(
            serialized["capabilityState"]["authorability"],
            "support_locked"
        );
    }

    #[test]
    fn relation_actions_are_stateful_and_not_node_actions() {
        let authorable = CapabilityState::resolved_authorable();
        let form_element_actions = semantic_actions_for(&NodeKind::FormElement, authorable);
        for relation_action in [
            SemanticAction::Move,
            SemanticAction::BindData,
            SemanticAction::RebindData,
            SemanticAction::UnbindData,
            SemanticAction::BindCommand,
            SemanticAction::RebindCommand,
            SemanticAction::UnbindCommand,
        ] {
            assert!(!form_element_actions
                .iter()
                .any(|descriptor| descriptor.action == relation_action));
        }

        let move_actions = semantic_actions_for_relation(
            &NodeKind::Form,
            &NodeKind::FormElement,
            RelationKind::Contains,
            authorable,
        );
        assert!(action_is_present(
            &move_actions,
            SemanticAction::Move,
            ActionExecutionPolicy::AtomicRelationMutation,
        ));

        for target_kind in [NodeKind::Attribute, NodeKind::FormAttribute] {
            let data_actions = semantic_actions_for_relation(
                &NodeKind::FormElement,
                &target_kind,
                RelationKind::References,
                authorable,
            );
            for action in [
                SemanticAction::BindData,
                SemanticAction::RebindData,
                SemanticAction::UnbindData,
            ] {
                assert!(action_is_present(
                    &data_actions,
                    action,
                    ActionExecutionPolicy::AtomicRelationMutation,
                ));
            }
        }

        for target_kind in [NodeKind::FormCommand, NodeKind::Command] {
            let command_actions = semantic_actions_for_relation(
                &NodeKind::FormElement,
                &target_kind,
                RelationKind::References,
                authorable,
            );
            for action in [
                SemanticAction::BindCommand,
                SemanticAction::RebindCommand,
                SemanticAction::UnbindCommand,
            ] {
                assert!(action_is_present(
                    &command_actions,
                    action,
                    ActionExecutionPolicy::AtomicRelationMutation,
                ));
            }
        }

        let form = ObjectRef::new("main", Vec::new(), NodeKind::Form, "ItemForm");
        let element = ObjectRef::new(
            "main",
            vec![OwnerSegment::new(NodeKind::Form, "ItemForm")],
            NodeKind::FormElement,
            "Group",
        );
        let edge = NavigationEdge::new(form, element, RelationKind::Contains, authorable);
        assert!(action_is_present(
            edge.semantic_actions(),
            SemanticAction::Move,
            ActionExecutionPolicy::AtomicRelationMutation,
        ));
        let serialized_edge = serde_json::to_value(&edge).expect("edge is serializable");
        assert_eq!(
            serialized_edge["capabilityState"]["resolutionState"],
            "resolved"
        );
        assert!(serialized_edge["semanticActions"]
            .as_array()
            .expect("edge actions are serialized")
            .iter()
            .any(|action| {
                action["action"] == "move"
                    && action["executionPolicy"] == "atomic_relation_mutation"
            }));

        let unrelated_reference_actions = semantic_actions_for_relation(
            &NodeKind::FormElement,
            &NodeKind::Template {
                template_type: Some("TextDocument".to_string()),
            },
            RelationKind::References,
            authorable,
        );
        assert_eq!(unrelated_reference_actions.len(), 1);
        assert!(action_is_present(
            &unrelated_reference_actions,
            SemanticAction::Inspect,
            ActionExecutionPolicy::ReadOnly,
        ));

        let locked_actions = semantic_actions_for_relation(
            &NodeKind::Form,
            &NodeKind::FormElement,
            RelationKind::Contains,
            CapabilityState::new(ResolutionState::Resolved, Authorability::SupportLocked),
        );
        assert_eq!(locked_actions.len(), 1);
        assert!(action_is_present(
            &locked_actions,
            SemanticAction::Inspect,
            ActionExecutionPolicy::ReadOnly,
        ));
    }

    #[test]
    fn mxl_is_a_dedicated_profile_and_other_templates_fail_closed() {
        let authorable = CapabilityState::resolved_authorable();
        let document_actions =
            semantic_actions_for(&NodeKind::metadata_object("Document"), authorable);
        assert!(action_is_present(
            &document_actions,
            SemanticAction::AddMxl,
            ActionExecutionPolicy::AtomicNodeMutation,
        ));

        let mxl_kind = NodeKind::Template {
            template_type: Some("SpreadsheetDocument".to_string()),
        };
        assert_eq!(action_profile_for(&mxl_kind), ActionProfile::MxlTemplate);
        let mxl_actions = semantic_actions_for(&mxl_kind, authorable);
        for action in [
            SemanticAction::Inspect,
            SemanticAction::EditMxl,
            SemanticAction::Clone,
        ] {
            let policy = match action {
                SemanticAction::Inspect => ActionExecutionPolicy::ReadOnly,
                SemanticAction::Clone => ActionExecutionPolicy::AtomicRelationMutation,
                _ => ActionExecutionPolicy::AtomicNodeMutation,
            };
            assert!(action_is_present(&mxl_actions, action, policy,));
        }

        let text_template = NodeKind::Template {
            template_type: Some("TextDocument".to_string()),
        };
        assert_eq!(
            action_profile_for(&text_template),
            ActionProfile::UnmodeledTemplate
        );
        let text_actions = semantic_actions_for(&text_template, authorable);
        assert_eq!(text_actions.len(), 1);
        assert!(action_is_present(
            &text_actions,
            SemanticAction::Inspect,
            ActionExecutionPolicy::ReadOnly,
        ));

        let unproven_child = semantic_actions_for(&NodeKind::Attribute, authorable);
        assert_eq!(unproven_child.len(), 1);
        assert!(action_is_present(
            &unproven_child,
            SemanticAction::Inspect,
            ActionExecutionPolicy::ReadOnly,
        ));
    }

    #[test]
    fn remove_is_deferred_until_support_removal_eligibility_is_modeled() {
        let authorable = CapabilityState::resolved_authorable();
        let modeled_kinds = [
            NodeKind::metadata_object("Document"),
            NodeKind::Form,
            NodeKind::FormElement,
            NodeKind::TabularSection,
            NodeKind::Template {
                template_type: Some("SpreadsheetDocument".to_string()),
            },
        ];

        for kind in modeled_kinds {
            assert!(
                !semantic_actions_for(&kind, authorable)
                    .iter()
                    .any(|descriptor| descriptor.action == SemanticAction::Remove),
                "{kind:?} must not advertise removal without support removal eligibility"
            );
        }
    }

    #[test]
    fn clone_is_discoverable_on_a_node_but_mutates_its_owning_relation() {
        let authorable = CapabilityState::resolved_authorable();
        let modeled_kinds = [
            NodeKind::metadata_object("Document"),
            NodeKind::Form,
            NodeKind::FormElement,
            NodeKind::TabularSection,
            NodeKind::Template {
                template_type: Some("SpreadsheetDocument".to_string()),
            },
        ];

        for kind in modeled_kinds {
            let actions = semantic_actions_for(&kind, authorable);
            assert!(action_is_present(
                &actions,
                SemanticAction::Clone,
                ActionExecutionPolicy::AtomicRelationMutation,
            ));
            assert!(!action_is_present(
                &actions,
                SemanticAction::Clone,
                ActionExecutionPolicy::AtomicNodeMutation,
            ));
        }
    }

    fn contains_key(value: &Value, expected_key: &str) -> bool {
        match value {
            Value::Object(entries) => entries
                .iter()
                .any(|(key, value)| key == expected_key || contains_key(value, expected_key)),
            Value::Array(entries) => entries
                .iter()
                .any(|value| contains_key(value, expected_key)),
            _ => false,
        }
    }
}
