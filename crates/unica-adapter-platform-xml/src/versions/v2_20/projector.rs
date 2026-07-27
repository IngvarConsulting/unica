use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::{
    navigation::{
        action_profile_for, ActionAvailability, Atomicity, Authorability, CapabilityState,
        CapabilityVector, CoverageState, FormatCompatibility, IdentityStrength, NavigationEnvelope,
        NavigationFacetVisibility, NavigationNode, NodeKind, ObjectKey, ObjectRef,
        PropertyCapability, PropertyType, PropertyValue, RelationGroupRef, RelationKey,
        RelationKind, RelationRef, RelationRole, ResolutionState, SemanticAction,
        SemanticActionKind, SemanticFacets, SemanticProperty, SemanticPropertyId, SemanticRelation,
        SourceAdapterDiagnostic,
    },
    navigation_limits::{
        MAX_NAVIGATION_IDENTITY_ITEMS, MAX_NAVIGATION_NESTING_DEPTH, MAX_NAVIGATION_NODES,
        MAX_NAVIGATION_PROPERTIES_PER_NODE, MAX_NAVIGATION_RELATIONS, MAX_NAVIGATION_TYPE_VARIANTS,
    },
    source_adapters::{SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceId},
};

use super::{
    native_model::{
        NativeEvidenceState, NativeMetadataChild, NativeMetadataNode, NativeNodeBacking,
        NativeNodeState, NativeProperty, NativePropertyValue, NativeScalarType,
        PlatformXmlNativeSnapshot,
    },
    schema::{scalar_property_kind_2_20, MetadataClassRole, ScalarPropertyKind},
    support::SupportFacts,
};

const SCHEMA_VERSION: &str = "1";
const PROJECTOR_ID: &str = "platform-xml-2.20";

pub(crate) fn project(
    native: &PlatformXmlNativeSnapshot,
    support: &SupportFacts,
) -> Result<NavigationEnvelope, SourceAdapterError> {
    if native.source.adapter_id != PROJECTOR_ID {
        return Err(ambiguous(
            "Platform XML projection requires the exact 2.20 decoder",
        ));
    }

    preflight_native_snapshot(native)?;

    let mut graph = GraphBuilder::new(native, support);
    let root = graph.source_root()?;
    graph.project_node(&native.root, Some(&root), RelationRole::Children)?;
    graph.finish(root)
}

struct GraphBuilder<'a> {
    native: &'a PlatformXmlNativeSnapshot,
    support: &'a SupportFacts,
    nodes: Vec<NavigationNode>,
    relations: Vec<SemanticRelation>,
    object_keys: BTreeSet<String>,
    relation_keys: BTreeSet<String>,
    output_nodes: usize,
    output_relations: usize,
    output_properties: usize,
    output_identity_items: usize,
    diagnostics: Vec<SourceAdapterDiagnostic>,
    partial: bool,
}

impl<'a> GraphBuilder<'a> {
    fn new(native: &'a PlatformXmlNativeSnapshot, support: &'a SupportFacts) -> Self {
        Self {
            native,
            support,
            nodes: Vec::new(),
            relations: Vec::new(),
            object_keys: BTreeSet::new(),
            relation_keys: BTreeSet::new(),
            output_nodes: 0,
            output_relations: 0,
            output_properties: 0,
            output_identity_items: 0,
            diagnostics: Vec::new(),
            partial: false,
        }
    }

    fn source_root(&mut self) -> Result<ObjectRef, SourceAdapterError> {
        self.reserve_output_node()?;
        let (key, identity) = object_key(
            &self.native.source.source_id,
            None,
            NodeKind::SourceRoot,
            None,
            "source",
        )?;
        self.register_object_key(&key)?;
        let reference = ObjectRef::new(
            self.native.source.source_id.clone(),
            key,
            identity,
            NodeKind::SourceRoot,
            "Source",
        );
        self.nodes.push(NavigationNode {
            object_ref: reference.clone(),
            reference: reference.clone(),
            capability_state: CapabilityState::new(
                ResolutionState::Resolved,
                Authorability::DerivedReadOnly,
            ),
            capability: CapabilityVector {
                resolution: ResolutionState::Resolved,
                identity: reference.identity_strength.clone(),
                consistency: self.native.source.consistency.clone(),
                coverage: self.native.coverage,
                format: FormatCompatibility::Compatible,
                source_access: SourceAccess::ReadOnly,
                authorability: Authorability::DerivedReadOnly,
            },
            properties: BTreeMap::new(),
            facets: SemanticFacets::default(),
            action_profile: action_profile_for(&NodeKind::SourceRoot),
            semantic_actions: Vec::new(),
            actions: vec![modeled_action(
                SemanticActionKind::Inspect,
                reference.clone(),
                None,
            )],
            facet_visibility: NavigationFacetVisibility::Full,
        });
        Ok(reference)
    }

    fn project_node(
        &mut self,
        native_node: &NativeMetadataNode,
        owner: Option<&ObjectRef>,
        owning_role: RelationRole,
    ) -> Result<ObjectRef, SourceAdapterError> {
        self.reserve_output_node()?;
        let kind = node_kind(native_node)?;
        let (key, identity) = object_key(
            &self.native.source.source_id,
            owner.map(|value| &value.object_key),
            kind.clone(),
            native_node.uuid,
            &native_node.name,
        )?;
        self.register_object_key(&key)?;
        let reference = ObjectRef::new(
            self.native.source.source_id.clone(),
            key,
            identity,
            kind.clone(),
            native_node.name.clone(),
        );
        let property_projection = self.project_properties(&kind, &native_node.properties)?;
        let mut coverage = node_coverage(native_node, self.native.coverage);
        if property_projection.incomplete {
            coverage = CoverageState::Partial;
            self.partial = true;
        }
        if property_projection.unmapped {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "unmappedSemanticFact".to_string(),
                message: "source contains a property outside the registered semantic vocabulary"
                    .to_string(),
                details: Some(serde_json::json!({"objectRef": reference.clone()})),
            });
        } else if property_projection.incomplete {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "partialCoverage".to_string(),
                message: "a registered semantic property could not be resolved".to_string(),
                details: Some(serde_json::json!({"objectRef": reference.clone()})),
            });
        }
        let resolution = node_resolution(native_node);
        let authorability = native_node
            .uuid
            .map(|uuid| self.support.authorability_for(&uuid.to_string()))
            .unwrap_or(Authorability::DerivedReadOnly);
        let capability = CapabilityVector {
            resolution,
            identity: reference.identity_strength.clone(),
            consistency: self.native.source.consistency.clone(),
            coverage,
            format: FormatCompatibility::Compatible,
            source_access: SourceAccess::ReadOnly,
            authorability,
        };
        let action_authorability = if matches!(coverage, CoverageState::Complete) {
            authorability
        } else {
            Authorability::UnknownReadOnly
        };
        let capability_state = CapabilityState::new(resolution, action_authorability);
        let owning_relation = owner
            .map(|parent| self.add_contains(parent, &reference, owning_role))
            .transpose()?;
        let actions = modeled_actions(&kind, &reference, capability_state, owning_relation.clone());
        self.nodes.push(NavigationNode {
            object_ref: reference.clone(),
            reference: reference.clone(),
            capability_state,
            capability,
            properties: property_projection.properties,
            facets: SemanticFacets::default(),
            action_profile: action_profile_for(&kind),
            semantic_actions: Vec::new(),
            actions,
            facet_visibility: NavigationFacetVisibility::Full,
        });

        for child in &native_node.children {
            self.project_child(child, &reference)?;
        }
        Ok(reference)
    }

    fn project_child(
        &mut self,
        child: &NativeMetadataChild,
        owner: &ObjectRef,
    ) -> Result<ObjectRef, SourceAdapterError> {
        self.project_node(&child.node, Some(owner), child.role)
    }

    fn add_contains(
        &mut self,
        owner: &ObjectRef,
        target: &ObjectRef,
        role: RelationRole,
    ) -> Result<RelationRef, SourceAdapterError> {
        self.reserve_output_relation()?;
        self.reserve_identity_item()?;
        let relation_key = relation_key(
            &self.native.source.source_id,
            &owner.object_key,
            RelationKind::Contains,
            &target.object_key,
        )?;
        if !self.relation_keys.insert(relation_key.as_str().to_string()) {
            return Err(SourceAdapterError::new(
                SourceAdapterErrorKind::IdentityCollision,
                "duplicate generated semantic relation key",
            ));
        }
        let relation_ref = RelationRef {
            source_id: self.native.source.source_id.clone(),
            relation_key,
            kind: RelationKind::Contains,
        };
        let group_ref = RelationGroupRef::new(
            self.native.source.source_id.clone(),
            owner.clone(),
            role,
            RelationKind::Contains,
        )?;
        self.relations.push(SemanticRelation {
            relation_ref: relation_ref.clone(),
            group_ref,
            identity_strength: IdentityStrength::Derived,
            kind: RelationKind::Contains,
            role,
            source: owner.clone(),
            target: target.clone(),
            capability: CapabilityVector {
                resolution: ResolutionState::Resolved,
                identity: IdentityStrength::Derived,
                consistency: self.native.source.consistency.clone(),
                coverage: self.native.coverage,
                format: FormatCompatibility::Compatible,
                source_access: SourceAccess::ReadOnly,
                authorability: Authorability::DerivedReadOnly,
            },
        });
        Ok(relation_ref)
    }

    fn register_object_key(&mut self, key: &ObjectKey) -> Result<(), SourceAdapterError> {
        self.reserve_identity_item()?;
        if self.object_keys.insert(key.as_str().to_string()) {
            Ok(())
        } else {
            Err(SourceAdapterError::new(
                SourceAdapterErrorKind::IdentityCollision,
                "duplicate generated semantic object key",
            ))
        }
    }

    fn reserve_output_node(&mut self) -> Result<(), SourceAdapterError> {
        reserve(
            &mut self.output_nodes,
            MAX_NAVIGATION_NODES,
            "semantic nodes",
        )
    }

    fn reserve_output_relation(&mut self) -> Result<(), SourceAdapterError> {
        reserve(
            &mut self.output_relations,
            MAX_NAVIGATION_RELATIONS,
            "semantic relations",
        )
    }

    fn reserve_identity_item(&mut self) -> Result<(), SourceAdapterError> {
        reserve(
            &mut self.output_identity_items,
            MAX_NAVIGATION_IDENTITY_ITEMS,
            "semantic identity items",
        )
    }

    fn project_properties(
        &mut self,
        kind: &NodeKind,
        native: &BTreeMap<String, NativeProperty>,
    ) -> Result<PropertyProjection, SourceAdapterError> {
        let mut projected = BTreeMap::new();
        let mut unmapped = false;
        let mut incomplete = false;
        for (id, native_property) in native {
            if projected.len() >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
                return Err(resource_limit("semantic node has too many properties"));
            }
            reserve(
                &mut self.output_properties,
                MAX_NAVIGATION_IDENTITY_ITEMS,
                "semantic properties",
            )?;
            let Some(semantic_id) = semantic_property_id(*kind, id) else {
                unmapped = true;
                incomplete = true;
                continue;
            };
            let property = project_property(semantic_id, native_property)?;
            incomplete |= matches!(property.value_type(), PropertyType::Unknown)
                || matches!(
                    property.value_state(),
                    crate::domain::navigation::PropertyValueState::Unresolved
                );
            if projected.insert(semantic_id, property).is_some() {
                return Err(ambiguous(
                    "multiple Platform XML properties map to one semantic property",
                ));
            }
        }
        Ok(PropertyProjection {
            properties: projected,
            unmapped,
            incomplete,
        })
    }

    fn finish(mut self, root: ObjectRef) -> Result<NavigationEnvelope, SourceAdapterError> {
        for node in &mut self.nodes {
            let relation_ids = self
                .relations
                .iter()
                .filter(|relation| relation.source == node.object_ref)
                .map(|relation| relation.role);
            node.facets =
                SemanticFacets::for_available(node.properties.keys().copied(), relation_ids);
        }
        let status = if self.partial || !matches!(self.native.coverage, CoverageState::Complete) {
            crate::domain::navigation::NavigationStatus::Partial
        } else {
            crate::domain::navigation::NavigationStatus::Available
        };
        let mut envelope = NavigationEnvelope {
            schema_version: SCHEMA_VERSION.to_string(),
            status,
            snapshot: Some(self.native.source.clone()),
            root: Some(root),
            nodes: self.nodes,
            relations: Vec::new(),
            diagnostics: self.diagnostics,
            relation_index: std::sync::Arc::new(self.relations),
        };
        envelope.reconcile_partial_coverage();
        Ok(envelope)
    }
}

fn object_key(
    source_id: &SourceId,
    owner: Option<&ObjectKey>,
    kind: NodeKind,
    native_uuid: Option<Uuid>,
    validated_name: &str,
) -> Result<(ObjectKey, IdentityStrength), SourceAdapterError> {
    validate_name(validated_name)?;
    if let Some(uuid) = native_uuid {
        return Ok((
            ObjectKey::new(format!("uuid:{uuid}"))?,
            IdentityStrength::Persistent,
        ));
    }
    let digest = digest_parts(&[
        source_id.as_str(),
        owner.map(ObjectKey::as_str).unwrap_or(""),
        &canonical_kind(&kind),
        validated_name,
    ]);
    Ok((
        ObjectKey::new(format!("derived:sha256:{digest}"))?,
        IdentityStrength::Derived,
    ))
}

fn relation_key(
    source_id: &SourceId,
    owner: &ObjectKey,
    kind: RelationKind,
    target: &ObjectKey,
) -> Result<RelationKey, SourceAdapterError> {
    let kind = match kind {
        RelationKind::Contains => "contains",
        RelationKind::References => "references",
    };
    RelationKey::new(format!(
        "derived:sha256:{}",
        digest_parts(&[source_id.as_str(), owner.as_str(), kind, target.as_str()])
    ))
}

fn digest_parts(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update(part.as_bytes());
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn canonical_kind(kind: &NodeKind) -> String {
    kind.as_str().to_string()
}

fn validate_name(name: &str) -> Result<(), SourceAdapterError> {
    if name.is_empty() || name.chars().any(char::is_control) || name.contains(['/', '\\']) {
        return Err(ambiguous("Platform XML node has an invalid semantic name"));
    }
    Ok(())
}

fn node_kind(node: &NativeMetadataNode) -> Result<NodeKind, SourceAdapterError> {
    let kind = match node.class.role {
        MetadataClassRole::Configuration => NodeKind::Configuration,
        MetadataClassRole::TopLevelObject => match node.class.canonical_name {
            "Language" => NodeKind::Language,
            "Subsystem" => NodeKind::Subsystem,
            "StyleItem" => NodeKind::StyleItem,
            "Style" => NodeKind::Style,
            "CommonPicture" => NodeKind::CommonPicture,
            "SessionParameter" => NodeKind::SessionParameter,
            "Role" => NodeKind::Role,
            "CommonTemplate" => NodeKind::CommonTemplate,
            "FilterCriterion" => NodeKind::FilterCriterion,
            "CommonModule" => NodeKind::CommonModule,
            "Bot" => NodeKind::Bot,
            "CommonAttribute" => NodeKind::CommonAttribute,
            "ExchangePlan" => NodeKind::ExchangePlan,
            "XDTOPackage" => NodeKind::XdtoPackage,
            "WebService" => NodeKind::WebService,
            "HTTPService" => NodeKind::HttpService,
            "WSReference" => NodeKind::WebServiceReference,
            "EventSubscription" => NodeKind::EventSubscription,
            "ScheduledJob" => NodeKind::ScheduledJob,
            "SettingsStorage" => NodeKind::SettingsStorage,
            "FunctionalOption" => NodeKind::FunctionalOption,
            "FunctionalOptionsParameter" => NodeKind::FunctionalOptionsParameter,
            "DefinedType" => NodeKind::DefinedType,
            "CommonCommand" => NodeKind::CommonCommand,
            "CommandGroup" => NodeKind::CommandGroup,
            "Constant" => NodeKind::Constant,
            "CommonForm" => NodeKind::CommonForm,
            "Catalog" => NodeKind::Catalog,
            "Document" => NodeKind::Document,
            "DocumentNumerator" => NodeKind::DocumentNumerator,
            "Sequence" => NodeKind::Sequence,
            "DocumentJournal" => NodeKind::DocumentJournal,
            "Enum" | "Enumeration" => NodeKind::Enumeration,
            "Report" => NodeKind::Report,
            "DataProcessor" => NodeKind::DataProcessor,
            "InformationRegister" => NodeKind::InformationRegister,
            "AccumulationRegister" => NodeKind::AccumulationRegister,
            "ChartOfCharacteristicTypes" => NodeKind::ChartOfCharacteristicTypes,
            "ChartOfAccounts" => NodeKind::ChartOfAccounts,
            "AccountingRegister" => NodeKind::AccountingRegister,
            "ChartOfCalculationTypes" => NodeKind::ChartOfCalculationTypes,
            "CalculationRegister" => NodeKind::CalculationRegister,
            "BusinessProcess" => NodeKind::BusinessProcess,
            "Task" => NodeKind::Task,
            "IntegrationService" => NodeKind::IntegrationService,
            _ => {
                return Err(ambiguous(
                    "Platform XML object kind has no registered semantic mapping",
                ))
            }
        },
        MetadataClassRole::Attribute => match node.class.canonical_name {
            "Dimension" => NodeKind::Dimension,
            "Resource" => NodeKind::Resource,
            "EnumValue" => NodeKind::EnumerationValue,
            "Parameter" => NodeKind::WebServiceParameter,
            _ => NodeKind::Attribute,
        },
        MetadataClassRole::TabularSection => NodeKind::TabularSection,
        MetadataClassRole::Command => NodeKind::Command,
        MetadataClassRole::Form => NodeKind::Form,
        MetadataClassRole::Template
            if template_type(node).as_deref() == Some("SpreadsheetDocument") =>
        {
            NodeKind::SpreadsheetDocumentTemplate
        }
        MetadataClassRole::Template => NodeKind::Template,
    };
    Ok(kind)
}

fn template_type(node: &NativeMetadataNode) -> Option<String> {
    let NativeNodeBacking::Template(template) = &node.backing else {
        return None;
    };
    match (
        &template.descriptor.state,
        &template.canonical_content.state,
        &template.descriptor_type,
        template.mxl_root_kind,
    ) {
        (
            NativeEvidenceState::Validated,
            NativeEvidenceState::Validated,
            NativePropertyValue::Scalar(value),
            Some(_),
        ) if value == "SpreadsheetDocument" => Some(value.clone()),
        _ => None,
    }
}

fn node_resolution(node: &NativeMetadataNode) -> ResolutionState {
    match node.state {
        NativeNodeState::UnresolvedRegistration { .. } => ResolutionState::Unresolved,
        NativeNodeState::ResolvedInline | NativeNodeState::ResolvedRegistration { .. } => {
            ResolutionState::Resolved
        }
    }
}

fn node_coverage(node: &NativeMetadataNode, snapshot_coverage: CoverageState) -> CoverageState {
    if !matches!(snapshot_coverage, CoverageState::Complete) {
        return CoverageState::Partial;
    }
    if matches!(node.class.role, MetadataClassRole::Form) {
        return CoverageState::Partial;
    }
    match &node.backing {
        NativeNodeBacking::Form(form)
            if !matches!(form.descriptor.state, NativeEvidenceState::Validated)
                || !matches!(form.managed_content.state, NativeEvidenceState::Validated) =>
        {
            CoverageState::Partial
        }
        NativeNodeBacking::Template(template)
            if !matches!(template.descriptor.state, NativeEvidenceState::Validated)
                || !matches!(
                    template.canonical_content.state,
                    NativeEvidenceState::Validated
                ) =>
        {
            CoverageState::Partial
        }
        _ => CoverageState::Complete,
    }
}

fn modeled_actions(
    kind: &NodeKind,
    target: &ObjectRef,
    capability_state: CapabilityState,
    owning_relation: Option<RelationRef>,
) -> Vec<SemanticAction> {
    crate::domain::navigation::semantic_actions_for(kind, capability_state)
        .into_iter()
        .map(|descriptor| {
            if matches!(descriptor.action, SemanticActionKind::Clone) {
                SemanticAction::modeled_clone(target.clone(), owning_relation.clone())
            } else {
                modeled_action(descriptor.action, target.clone(), owning_relation.clone())
            }
        })
        .collect()
}

fn modeled_action(
    kind: SemanticActionKind,
    target: ObjectRef,
    owning_relation: Option<RelationRef>,
) -> SemanticAction {
    SemanticAction {
        kind,
        target: Some(target),
        owning_relation,
        availability: ActionAvailability::Modeled,
        blocking_reasons: Vec::new(),
        operation_binding: None,
        atomicity: Atomicity::ReadOnly,
    }
}

#[derive(Default)]
struct NativePreflight {
    output_nodes: usize,
    output_relations: usize,
    properties: usize,
    identity_items: usize,
}

fn preflight_native_snapshot(native: &PlatformXmlNativeSnapshot) -> Result<(), SourceAdapterError> {
    let mut budget = NativePreflight::default();
    reserve(
        &mut budget.output_nodes,
        MAX_NAVIGATION_NODES,
        "semantic nodes",
    )?;
    reserve(
        &mut budget.identity_items,
        MAX_NAVIGATION_IDENTITY_ITEMS,
        "semantic identity items",
    )?;
    preflight_native_node(&native.root, 1, &mut budget)
}

fn preflight_native_node(
    node: &NativeMetadataNode,
    depth: usize,
    budget: &mut NativePreflight,
) -> Result<(), SourceAdapterError> {
    if depth > MAX_NAVIGATION_NESTING_DEPTH {
        return Err(resource_limit(
            "native snapshot exceeds navigation nesting limit",
        ));
    }
    reserve(
        &mut budget.output_nodes,
        MAX_NAVIGATION_NODES,
        "semantic nodes",
    )?;
    reserve(
        &mut budget.output_relations,
        MAX_NAVIGATION_RELATIONS,
        "semantic relations",
    )?;
    reserve(
        &mut budget.identity_items,
        MAX_NAVIGATION_IDENTITY_ITEMS,
        "semantic identity items",
    )?;
    if node.properties.len() > MAX_NAVIGATION_PROPERTIES_PER_NODE {
        return Err(resource_limit("native node has too many properties"));
    }
    for property in node.properties.values() {
        reserve(
            &mut budget.properties,
            MAX_NAVIGATION_IDENTITY_ITEMS,
            "semantic properties",
        )?;
        if let NativePropertyValue::TypeSet(type_set) = &property.value {
            if type_set.variants.len() > MAX_NAVIGATION_TYPE_VARIANTS {
                return Err(resource_limit("native type set has too many variants"));
            }
        }
    }
    let child_depth = depth
        .checked_add(1)
        .ok_or_else(|| resource_limit("native snapshot nesting depth cannot be represented"))?;
    for child in &node.children {
        preflight_native_node(&child.node, child_depth, budget)?;
    }
    Ok(())
}

fn reserve(counter: &mut usize, limit: usize, label: &str) -> Result<(), SourceAdapterError> {
    let next = counter
        .checked_add(1)
        .ok_or_else(|| resource_limit(&format!("{label} accounting overflow")))?;
    if next > limit {
        return Err(resource_limit(&format!(
            "{label} exceed navigation limit {limit}"
        )));
    }
    *counter = next;
    Ok(())
}

struct PropertyProjection {
    properties: BTreeMap<SemanticPropertyId, SemanticProperty>,
    unmapped: bool,
    incomplete: bool,
}

fn semantic_property_id(kind: NodeKind, id: &str) -> Option<SemanticPropertyId> {
    let contextual = match (kind, id) {
        (NodeKind::Document, "NumberType") => Some(SemanticPropertyId::DOCUMENT_NUMBER_TYPE),
        (NodeKind::Document, "NumberLength") => Some(SemanticPropertyId::DOCUMENT_NUMBER_LENGTH),
        (NodeKind::Document, "NumberPeriodicity") => {
            Some(SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY)
        }
        (NodeKind::Document, "AutoNumbering") => Some(SemanticPropertyId::DOCUMENT_NUMBER_AUTO),
        (NodeKind::Document, "Posting") => Some(SemanticPropertyId::DOCUMENT_POSTING_MODE),
        (NodeKind::Document, "RealTimePosting") => {
            Some(SemanticPropertyId::DOCUMENT_REAL_TIME_POSTING_MODE)
        }
        (NodeKind::Document, "RegisterRecordsDeletion") => {
            Some(SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_DELETION_MODE)
        }
        (NodeKind::Document, "RegisterRecordsWritingOnPost") => {
            Some(SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE)
        }
        (NodeKind::Catalog, "HierarchyType") => Some(SemanticPropertyId::CATALOG_HIERARCHY_TYPE),
        (NodeKind::Catalog, "HierarchyLevelCount" | "LevelCount") => {
            Some(SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT)
        }
        (NodeKind::Catalog, "CodeLength") => Some(SemanticPropertyId::CATALOG_CODE_LENGTH),
        (NodeKind::Catalog, "DescriptionLength") => {
            Some(SemanticPropertyId::CATALOG_DESCRIPTION_LENGTH)
        }
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "Periodicity",
        ) => Some(SemanticPropertyId::REGISTER_PERIODICITY),
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "WriteMode",
        ) => Some(SemanticPropertyId::REGISTER_WRITE_MODE),
        (
            NodeKind::InformationRegister
            | NodeKind::AccumulationRegister
            | NodeKind::AccountingRegister
            | NodeKind::CalculationRegister,
            "RegisterType",
        ) => Some(SemanticPropertyId::REGISTER_TYPE),
        (NodeKind::Constant, "Type" | "TypeDescription" | "DataType") => {
            Some(SemanticPropertyId::CONSTANT_VALUE_TYPE)
        }
        (NodeKind::DefinedType, "Type" | "TypeDescription" | "DataType") => {
            Some(SemanticPropertyId::DEFINED_TYPE)
        }
        (NodeKind::Report, "MainDataCompositionSchema") => {
            Some(SemanticPropertyId::REPORT_MAIN_DATA_COMPOSITION_SCHEMA)
        }
        (NodeKind::CommonModule, "Global") => Some(SemanticPropertyId::MODULE_GLOBAL),
        (NodeKind::CommonModule, "ClientManagedApplication") => {
            Some(SemanticPropertyId::MODULE_CLIENT_MANAGED_APPLICATION)
        }
        (NodeKind::CommonModule, "Server") => Some(SemanticPropertyId::MODULE_SERVER),
        (NodeKind::CommonModule, "ExternalConnection") => {
            Some(SemanticPropertyId::MODULE_EXTERNAL_CONNECTION)
        }
        (NodeKind::CommonModule, "ClientOrdinaryApplication") => {
            Some(SemanticPropertyId::MODULE_CLIENT_ORDINARY_APPLICATION)
        }
        (NodeKind::CommonModule, "ServerCall") => Some(SemanticPropertyId::MODULE_SERVER_CALL),
        (NodeKind::CommonModule, "Privileged") => Some(SemanticPropertyId::MODULE_PRIVILEGED),
        (NodeKind::CommonModule, "ReturnValuesReuse") => {
            Some(SemanticPropertyId::MODULE_RETURN_VALUES_REUSE)
        }
        (NodeKind::ScheduledJob, "MethodName") => Some(SemanticPropertyId::JOB_METHOD),
        (NodeKind::ScheduledJob, "Use") => Some(SemanticPropertyId::JOB_USE),
        (NodeKind::ScheduledJob, "Predefined") => Some(SemanticPropertyId::JOB_PREDEFINED),
        (NodeKind::ScheduledJob, "RestartCountOnFailure") => {
            Some(SemanticPropertyId::JOB_RESTART_COUNT)
        }
        (NodeKind::ScheduledJob, "RestartIntervalOnFailure") => {
            Some(SemanticPropertyId::JOB_RESTART_INTERVAL)
        }
        (NodeKind::ScheduledJob, "Key") => Some(SemanticPropertyId::JOB_KEY),
        (NodeKind::EventSubscription, "Event") => Some(SemanticPropertyId::SUBSCRIPTION_EVENT),
        (NodeKind::EventSubscription, "Handler") => Some(SemanticPropertyId::SUBSCRIPTION_HANDLER),
        (NodeKind::EventSubscription, "Source") => {
            Some(SemanticPropertyId::SUBSCRIPTION_SOURCE_TYPE)
        }
        (NodeKind::HttpService, "RootURL" | "RootUrl") => {
            Some(SemanticPropertyId::HTTP_SERVICE_ROOT_URL)
        }
        (NodeKind::HttpService, "ReuseSessions") => {
            Some(SemanticPropertyId::HTTP_SERVICE_REUSE_SESSIONS)
        }
        (NodeKind::HttpService, "SessionMaxAge") => {
            Some(SemanticPropertyId::HTTP_SERVICE_SESSION_MAX_AGE)
        }
        (NodeKind::HttpServiceUrlTemplate, "Template") => {
            Some(SemanticPropertyId::HTTP_SERVICE_URL_TEMPLATE)
        }
        (NodeKind::HttpServiceMethod, "HTTPMethod") => {
            Some(SemanticPropertyId::HTTP_SERVICE_METHOD)
        }
        (NodeKind::HttpServiceMethod, "Handler") => Some(SemanticPropertyId::HTTP_SERVICE_HANDLER),
        (NodeKind::WebService, "Namespace") => Some(SemanticPropertyId::WEB_SERVICE_NAMESPACE),
        (NodeKind::WebService, "XDTOPackages") => {
            Some(SemanticPropertyId::WEB_SERVICE_XDTO_PACKAGES)
        }
        (NodeKind::WebService, "DescriptorFileName") => {
            Some(SemanticPropertyId::WEB_SERVICE_DESCRIPTOR_FILE_NAME)
        }
        (NodeKind::WebService, "ReuseSessions") => {
            Some(SemanticPropertyId::WEB_SERVICE_REUSE_SESSIONS)
        }
        (NodeKind::WebService, "SessionMaxAge") => {
            Some(SemanticPropertyId::WEB_SERVICE_SESSION_MAX_AGE)
        }
        (NodeKind::WebServiceOperation, "XDTOReturningValueType") => {
            Some(SemanticPropertyId::WEB_SERVICE_OPERATION_RETURN_TYPE)
        }
        (NodeKind::WebServiceOperation, "Nillable") => {
            Some(SemanticPropertyId::WEB_SERVICE_OPERATION_NILLABLE)
        }
        (NodeKind::WebServiceOperation, "Transactioned") => {
            Some(SemanticPropertyId::WEB_SERVICE_OPERATION_TRANSACTIONED)
        }
        (NodeKind::WebServiceOperation, "ProcedureName") => {
            Some(SemanticPropertyId::WEB_SERVICE_OPERATION_PROCEDURE_NAME)
        }
        (NodeKind::WebServiceParameter, "XDTOValueType") => {
            Some(SemanticPropertyId::WEB_SERVICE_PARAMETER_TYPE)
        }
        (NodeKind::WebServiceParameter, "Nillable") => {
            Some(SemanticPropertyId::WEB_SERVICE_PARAMETER_NILLABLE)
        }
        (NodeKind::WebServiceParameter, "TransferDirection") => {
            Some(SemanticPropertyId::WEB_SERVICE_PARAMETER_DIRECTION)
        }
        (
            NodeKind::Attribute
            | NodeKind::Dimension
            | NodeKind::Resource
            | NodeKind::WebServiceParameter,
            "Type" | "TypeDescription" | "DataType",
        ) => Some(SemanticPropertyId::FIELD_TYPE),
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "FillChecking") => {
            Some(SemanticPropertyId::FIELD_FILL_CHECKING)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "Indexing") => {
            Some(SemanticPropertyId::FIELD_INDEXING)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "MultiLine") => {
            Some(SemanticPropertyId::FIELD_MULTI_LINE)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "Use") => {
            Some(SemanticPropertyId::FIELD_USE)
        }
        (NodeKind::Attribute | NodeKind::Dimension | NodeKind::Resource, "FillValue") => {
            Some(SemanticPropertyId::FIELD_FILL_VALUE)
        }
        (NodeKind::Dimension, "Master") => Some(SemanticPropertyId::FIELD_MASTER),
        (NodeKind::Dimension, "MainFilter") => Some(SemanticPropertyId::FIELD_MAIN_FILTER),
        (NodeKind::Dimension, "DenyIncompleteValues") => {
            Some(SemanticPropertyId::FIELD_DENY_INCOMPLETE_VALUES)
        }
        (NodeKind::TabularSection, "Order") => Some(SemanticPropertyId::TABULAR_SECTION_ORDER),
        (NodeKind::TabularSection, "LineNumberLength") => {
            Some(SemanticPropertyId::TABULAR_SECTION_LINE_NUMBER_LENGTH)
        }
        (NodeKind::Form, "FormType") => Some(SemanticPropertyId::FORM_TYPE),
        (NodeKind::Template | NodeKind::SpreadsheetDocumentTemplate, "TemplateType") => {
            Some(SemanticPropertyId::TEMPLATE_TYPE)
        }
        (NodeKind::Command, "Group") => Some(SemanticPropertyId::COMMAND_GROUP),
        (NodeKind::Command, "Representation") => Some(SemanticPropertyId::COMMAND_REPRESENTATION),
        _ => None,
    };
    contextual.or_else(|| match id {
        "Name" => Some(SemanticPropertyId::METADATA_NAME),
        "Uuid" | "UUID" => Some(SemanticPropertyId::METADATA_UUID),
        "Synonym" => Some(SemanticPropertyId::METADATA_SYNONYM),
        "Comment" => Some(SemanticPropertyId::METADATA_COMMENT),
        "Code" => Some(SemanticPropertyId::METADATA_CODE),
        "Description" => Some(SemanticPropertyId::METADATA_DESCRIPTION),
        "ObjectPresentation" => Some(SemanticPropertyId::PRESENTATION_OBJECT),
        "ExtendedObjectPresentation" => Some(SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT),
        "ListPresentation" => Some(SemanticPropertyId::PRESENTATION_LIST),
        "ExtendedListPresentation" => Some(SemanticPropertyId::PRESENTATION_EXTENDED_LIST),
        "Length" => Some(SemanticPropertyId::FIELD_LENGTH),
        "Digits" => Some(SemanticPropertyId::FIELD_DIGITS),
        "FractionDigits" => Some(SemanticPropertyId::FIELD_FRACTION_DIGITS),
        "FillValue" => Some(SemanticPropertyId::FIELD_FILL_VALUE),
        "UseStandardCommands" => Some(SemanticPropertyId::COMMAND_USE_STANDARD),
        "IncludeHelpInContents" => Some(SemanticPropertyId::HELP_INCLUDE_IN_CONTENTS),
        _ => None,
    })
}

fn project_property(
    semantic_id: SemanticPropertyId,
    property: &NativeProperty,
) -> Result<SemanticProperty, SourceAdapterError> {
    let mut projected = match &property.value {
        NativePropertyValue::Absent => SemanticProperty::absent(semantic_id),
        NativePropertyValue::Unresolved => SemanticProperty::unresolved(semantic_id),
        NativePropertyValue::UnresolvedScalar { .. } => SemanticProperty::unresolved(semantic_id),
        NativePropertyValue::Scalar(value) => {
            scalar_property(semantic_id, &property.canonical_id, value, None)?
        }
        NativePropertyValue::AnnotatedScalar {
            value,
            type_annotation,
        } => scalar_property(
            semantic_id,
            &property.canonical_id,
            value,
            Some(*type_annotation),
        )?,
        NativePropertyValue::TypeSet(type_set) => {
            SemanticProperty::explicit(semantic_id, PropertyValue::TypeSet(type_set.clone()))?
        }
        NativePropertyValue::Structured => SemanticProperty::unresolved(semantic_id),
    };
    if projected.value().is_some() {
        projected = projected.with_capability(PropertyCapability::ReadOnly)?;
    }
    Ok(projected)
}

fn scalar_property(
    semantic_id: SemanticPropertyId,
    canonical_id: &str,
    value: &str,
    type_annotation: Option<NativeScalarType>,
) -> Result<SemanticProperty, SourceAdapterError> {
    let definition = crate::domain::navigation::property_definition(semantic_id);
    if definition.allowed_types() == [PropertyType::Enum] {
        return Ok(semantic_enum_value(value)
            .map(PropertyValue::EnumSymbol)
            .map(|value| SemanticProperty::explicit(semantic_id, value))
            .transpose()?
            .unwrap_or_else(|| SemanticProperty::unresolved(semantic_id)));
    }
    let Some(kind) = scalar_property_kind_2_20(canonical_id) else {
        return Ok(SemanticProperty::unresolved(semantic_id));
    };
    let value = match kind {
        ScalarPropertyKind::Boolean => match value {
            "true" => PropertyValue::Boolean(true),
            "false" => PropertyValue::Boolean(false),
            _ => return Err(ambiguous("invalid boolean Platform XML scalar property")),
        },
        ScalarPropertyKind::Integer => PropertyValue::Integer(
            value
                .parse()
                .map_err(|_| ambiguous("invalid integer Platform XML scalar property"))?,
        ),
        ScalarPropertyKind::Uuid => PropertyValue::Uuid(
            value
                .parse()
                .map_err(|_| ambiguous("invalid UUID Platform XML scalar property"))?,
        ),
        ScalarPropertyKind::String => PropertyValue::String(value.to_string()),
        ScalarPropertyKind::PolymorphicFillValue => match type_annotation {
            Some(NativeScalarType::Decimal) => match normalize_xml_schema_decimal(value) {
                Some(value) => PropertyValue::Decimal(value),
                None => return Ok(SemanticProperty::unresolved(semantic_id)),
            },
            Some(NativeScalarType::String) => PropertyValue::String(value.to_string()),
            None | Some(_) => {
                return Ok(SemanticProperty::unresolved(semantic_id));
            }
        },
    };
    if definition.accepts(value.value_type()) {
        SemanticProperty::explicit(semantic_id, value)
    } else {
        Ok(SemanticProperty::unresolved(semantic_id))
    }
}

fn semantic_enum_value(value: &str) -> Option<crate::domain::navigation::SemanticEnumValue> {
    use crate::domain::navigation::SemanticEnumValue;

    match value {
        "String" | "string" => Some(SemanticEnumValue::STRING),
        "Number" | "number" => Some(SemanticEnumValue::NUMBER),
        "Nonperiodical" | "nonperiodical" => Some(SemanticEnumValue::NONPERIODICAL),
        "Second" | "second" => Some(SemanticEnumValue::SECOND),
        "Day" | "day" => Some(SemanticEnumValue::DAY),
        "Month" | "month" => Some(SemanticEnumValue::MONTH),
        "Quarter" | "quarter" => Some(SemanticEnumValue::QUARTER),
        "Year" | "year" => Some(SemanticEnumValue::YEAR),
        "RecorderPosition" | "recorderPosition" => Some(SemanticEnumValue::RECORDER_POSITION),
        "Allow" | "allow" => Some(SemanticEnumValue::ALLOW),
        "Deny" | "deny" => Some(SemanticEnumValue::DENY),
        "HierarchyOfItems" | "hierarchyOfItems" => Some(SemanticEnumValue::HIERARCHY_OF_ITEMS),
        "HierarchyOfGroupsAndItems" | "hierarchyOfGroupsAndItems" => {
            Some(SemanticEnumValue::HIERARCHY_OF_GROUPS_AND_ITEMS)
        }
        "Balance" | "balance" => Some(SemanticEnumValue::BALANCE),
        "Turnovers" | "turnovers" => Some(SemanticEnumValue::TURNOVERS),
        "Independent" | "independent" => Some(SemanticEnumValue::INDEPENDENT),
        "RecorderSubordinate" | "recorderSubordinate" => {
            Some(SemanticEnumValue::RECORDER_SUBORDINATE)
        }
        "DontCheck" | "dontCheck" => Some(SemanticEnumValue::DONT_CHECK),
        "ShowError" | "showError" => Some(SemanticEnumValue::SHOW_ERROR),
        "DontIndex" | "dontIndex" => Some(SemanticEnumValue::DONT_INDEX),
        "Index" | "index" => Some(SemanticEnumValue::INDEX),
        "IndexWithAdditionalOrder" | "indexWithAdditionalOrder" => {
            Some(SemanticEnumValue::INDEX_WITH_ADDITIONAL_ORDER)
        }
        "Use" | "use" => Some(SemanticEnumValue::USE),
        "DontUse" | "dontUse" => Some(SemanticEnumValue::DONT_USE),
        "ForItem" | "forItem" => Some(SemanticEnumValue::FOR_ITEM),
        "DuringRequest" | "duringRequest" => Some(SemanticEnumValue::DURING_REQUEST),
        "DuringSession" | "duringSession" => Some(SemanticEnumValue::DURING_SESSION),
        "In" | "in" => Some(SemanticEnumValue::IN),
        "Out" | "out" => Some(SemanticEnumValue::OUT),
        "InOut" | "inOut" => Some(SemanticEnumValue::IN_OUT),
        _ => None,
    }
}

fn normalize_xml_schema_decimal(value: &str) -> Option<String> {
    let (negative, value) = match value.strip_prefix('+') {
        Some(value) => (false, value),
        None => match value.strip_prefix('-') {
            Some(value) => (true, value),
            None => (false, value),
        },
    };
    let (integer, fraction) = match value.split_once('.') {
        Some((integer, fraction)) if !fraction.contains('.') => (integer, fraction),
        None => (value, ""),
        _ => return None,
    };
    if (integer.is_empty() && fraction.is_empty())
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = fraction.trim_end_matches('0');
    let fraction = if fraction.is_empty() { "0" } else { fraction };
    let is_zero = integer == "0" && fraction == "0";
    Some(format!(
        "{}{}.{}",
        if negative && !is_zero { "-" } else { "" },
        integer,
        fraction
    ))
}

fn ambiguous(message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}

fn resource_limit(message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ResourceLimit, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use roxmltree::Document;

    use super::*;
    use crate::{
        domain::{
            navigation::{
                ActionKind, FormatCompatibility, PrimitiveTypeKind, PropertyType, PropertyValue,
                PropertyValueState, RelationKind, StringLength, StringQualifiers, TypeQualifiers,
                TypeVariant,
            },
            source_adapters::{SnapshotConsistency, SourceId, SourceRevision, SourceSnapshot},
        },
        infrastructure::source_adapters::platform_xml::{
            native_model::{NativeMetadataClass, NativePropertyProvenance},
            schema::MetadataClassRole,
            support,
        },
    };

    fn type_description(
        body: &str,
    ) -> Result<crate::domain::navigation::TypeSetValue, SourceAdapterError> {
        let xml = format!(
            "<DataType xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">{body}</DataType>"
        );
        let document = Document::parse(&xml).expect("test type XML");
        crate::versions::v2_20::schema::parse_type_description_2_20(document.root_element())
    }

    #[test]
    fn root_metadata_object_has_an_owning_relation() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();
        let owning = envelope.owning_relation(&document.object_ref).unwrap();

        assert_eq!(owning.kind, RelationKind::Contains);
        assert_eq!(owning.source.kind, NodeKind::SourceRoot);
    }

    #[test]
    fn every_known_top_level_metadata_class_has_a_closed_semantic_kind() {
        let expected = [
            ("Language", "language"),
            ("Subsystem", "subsystem"),
            ("StyleItem", "styleItem"),
            ("Style", "style"),
            ("CommonPicture", "commonPicture"),
            ("SessionParameter", "sessionParameter"),
            ("Role", "role"),
            ("CommonTemplate", "commonTemplate"),
            ("FilterCriterion", "filterCriterion"),
            ("CommonModule", "commonModule"),
            ("Bot", "bot"),
            ("CommonAttribute", "commonAttribute"),
            ("ExchangePlan", "exchangePlan"),
            ("XDTOPackage", "xdtoPackage"),
            ("WebService", "webService"),
            ("HTTPService", "httpService"),
            ("WSReference", "webServiceReference"),
            ("EventSubscription", "eventSubscription"),
            ("ScheduledJob", "scheduledJob"),
            ("SettingsStorage", "settingsStorage"),
            ("FunctionalOption", "functionalOption"),
            ("FunctionalOptionsParameter", "functionalOptionsParameter"),
            ("DefinedType", "definedType"),
            ("CommonCommand", "commonCommand"),
            ("CommandGroup", "commandGroup"),
            ("Constant", "constant"),
            ("CommonForm", "commonForm"),
            ("Catalog", "catalog"),
            ("Document", "document"),
            ("DocumentNumerator", "documentNumerator"),
            ("Sequence", "sequence"),
            ("DocumentJournal", "documentJournal"),
            ("Enum", "enumeration"),
            ("Report", "report"),
            ("DataProcessor", "dataProcessor"),
            ("InformationRegister", "informationRegister"),
            ("AccumulationRegister", "accumulationRegister"),
            ("ChartOfCharacteristicTypes", "chartOfCharacteristicTypes"),
            ("ChartOfAccounts", "chartOfAccounts"),
            ("AccountingRegister", "accountingRegister"),
            ("ChartOfCalculationTypes", "chartOfCalculationTypes"),
            ("CalculationRegister", "calculationRegister"),
            ("BusinessProcess", "businessProcess"),
            ("Task", "task"),
            ("IntegrationService", "integrationService"),
        ];

        for (native_class, semantic_kind) in expected {
            let native = node(
                native_class,
                MetadataClassRole::TopLevelObject,
                None,
                native_class,
                BTreeMap::new(),
                Vec::new(),
            );
            assert_eq!(
                node_kind(&native).map(|kind| kind.as_str()),
                Ok(semantic_kind),
                "{native_class}"
            );
        }
    }

    #[test]
    fn serialized_graph_contains_no_physical_paths() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let text = serde_json::to_string(&envelope).unwrap();

        assert!(!text.contains("/tmp/"));
        assert!(!text.contains("\\\\Users\\\\"));
        assert!(!text.contains("Ext/Template.xml"));
    }

    #[test]
    fn no_writer_means_mutations_are_modeled_not_executable() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let clone = envelope.action(ActionKind::Clone, "Order").unwrap();

        assert_eq!(clone.availability, ActionAvailability::Modeled);
        assert!(clone.operation_binding.is_none());
        assert!(clone.owning_relation.is_some());
    }

    #[test]
    fn format_compatibility_is_part_of_every_node_capability() {
        let envelope = project_fixture(document_fixture()).unwrap();

        assert!(envelope
            .nodes
            .iter()
            .all(|node| { node.capability.format == FormatCompatibility::Compatible }));
    }

    #[test]
    fn native_snapshot_is_preflighted_before_semantic_output_allocation() {
        use crate::domain::navigation_limits::MAX_NAVIGATION_NODES;

        let mut root = document_fixture();
        let template = root.children.pop().unwrap();
        root.children = (0..MAX_NAVIGATION_NODES)
            .map(|index| {
                let mut child = template.clone();
                child.node.uuid = None;
                child.node.name = format!("Bounded{index}");
                child
            })
            .collect();
        let error = project_fixture(root).unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::ResourceLimit);
    }

    #[test]
    fn document_properties_are_typed_for_ai_consumption() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();

        assert_eq!(
            document.properties[&SemanticPropertyId::DOCUMENT_NUMBER_LENGTH].value_type(),
            PropertyType::Integer
        );
        assert_eq!(
            document.properties[&SemanticPropertyId::DOCUMENT_NUMBER_LENGTH].value(),
            Some(&PropertyValue::Integer(11))
        );
        assert_eq!(
            document.properties[&SemanticPropertyId::DOCUMENT_NUMBER_LENGTH].value_state(),
            PropertyValueState::Explicit
        );
    }

    #[test]
    fn one_c_type_descriptions_are_structured_not_strings() {
        let envelope = project_fixture(attribute_fixture()).unwrap();
        let attribute = envelope.node_named(NodeKind::Attribute, "Product").unwrap();

        let Some(PropertyValue::TypeSet(type_set)) =
            attribute.properties[&SemanticPropertyId::FIELD_TYPE].value()
        else {
            panic!("expected structured type set");
        };
        assert_eq!(
            type_set.variants[0],
            TypeVariant::Reference {
                target: "Catalog.Products".to_string()
            }
        );
    }

    #[test]
    fn type_descriptions_accept_declared_qualifiers_and_enum_references() {
        let type_set = type_description(
            "<v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers><v8:Type>cfg:EnumRef.Statuses</v8:Type>",
        )
        .unwrap();

        assert_eq!(
            type_set.variants,
            vec![
                TypeVariant::Enumeration {
                    target: "Enum.Statuses".to_string()
                },
                TypeVariant::Primitive {
                    kind: PrimitiveTypeKind::String,
                    qualifiers: Some(TypeQualifiers::String(StringQualifiers {
                        length: Some(10),
                        allowed_length: Some(StringLength::Variable),
                    })),
                },
            ],
        );
    }

    #[test]
    fn incompatible_type_qualifiers_fail_closed() {
        for raw in [
            "<v8:Type>xs:boolean</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length></v8:StringQualifiers>",
            "<v8:Type>cfg:CatalogRef.Products</v8:Type><v8:NumberQualifiers><v8:Digits>10</v8:Digits></v8:NumberQualifiers>",
        ] {
            assert_eq!(
                type_description(raw).unwrap_err().kind,
                SourceAdapterErrorKind::ProjectionAmbiguous,
            );
        }
    }

    #[test]
    fn fill_value_uses_exact_native_scalar_annotation_not_text() {
        use crate::versions::v2_20::native_model::NativeScalarType;

        let mut decimal = document_fixture();
        decimal.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::AnnotatedScalar {
                    value: "0".to_string(),
                    type_annotation: NativeScalarType::Decimal,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );
        let mut string = decimal.clone();
        string.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::AnnotatedScalar {
                    value: "true".to_string(),
                    type_annotation: NativeScalarType::String,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );

        let decimal = project_fixture(decimal).unwrap();
        let string = project_fixture(string).unwrap();
        assert_eq!(
            decimal
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .properties[&SemanticPropertyId::FIELD_FILL_VALUE]
                .value(),
            Some(&PropertyValue::Decimal("0.0".to_string())),
        );
        assert_eq!(
            string
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .properties[&SemanticPropertyId::FIELD_FILL_VALUE]
                .value(),
            Some(&PropertyValue::String("true".to_string())),
        );
    }

    #[test]
    fn fill_value_without_a_known_annotation_is_unresolved() {
        let mut root = document_fixture();
        root.properties
            .insert("FillValue".to_string(), scalar("FillValue", "true"));

        let envelope = project_fixture(root).unwrap();
        let property = &envelope
            .node_named(NodeKind::Document, "Order")
            .unwrap()
            .properties[&SemanticPropertyId::FIELD_FILL_VALUE];
        assert_eq!(property.value_state(), PropertyValueState::Unresolved);
        assert_eq!(property.value(), None);
    }

    #[test]
    fn empty_annotated_fill_value_preserves_string_but_not_invalid_decimal() {
        use crate::versions::v2_20::native_model::{NativeScalarAnnotationIssue, NativeScalarType};

        let mut string = document_fixture();
        string.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::AnnotatedScalar {
                    value: String::new(),
                    type_annotation: NativeScalarType::String,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );
        let mut decimal = string.clone();
        decimal.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::UnresolvedScalar {
                    issue: NativeScalarAnnotationIssue::InvalidLexical,
                },
                provenance: NativePropertyProvenance::Unresolved,
            },
        );

        let string = project_fixture(string).unwrap();
        assert_eq!(
            string
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .properties[&SemanticPropertyId::FIELD_FILL_VALUE]
                .value(),
            Some(&PropertyValue::String(String::new())),
        );
        let decimal = project_fixture(decimal).unwrap();
        let property = &decimal
            .node_named(NodeKind::Document, "Order")
            .unwrap()
            .properties[&SemanticPropertyId::FIELD_FILL_VALUE];
        assert_eq!(property.value_state(), PropertyValueState::Unresolved);
        assert_eq!(property.value(), None);
    }

    #[test]
    fn fill_value_accepts_only_lossless_decimal_or_string_annotations() {
        use crate::versions::v2_20::native_model::NativeScalarType;

        let mut root = document_fixture();
        root.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::AnnotatedScalar {
                    value: "+001.2300".to_string(),
                    type_annotation: NativeScalarType::Decimal,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );
        let decimal = project_fixture(root.clone()).unwrap();
        assert_eq!(
            decimal
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .properties[&SemanticPropertyId::FIELD_FILL_VALUE]
                .value(),
            Some(&PropertyValue::Decimal("1.23".to_string())),
        );

        for annotation in [
            NativeScalarType::Boolean,
            NativeScalarType::Integer,
            NativeScalarType::Uuid,
        ] {
            root.properties.insert(
                "FillValue".to_string(),
                NativeProperty {
                    canonical_id: "FillValue".to_string(),
                    value: NativePropertyValue::AnnotatedScalar {
                        value: "true".to_string(),
                        type_annotation: annotation,
                    },
                    provenance: NativePropertyProvenance::Explicit,
                },
            );
            let envelope = project_fixture(root.clone()).unwrap();
            let property = &envelope
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .properties[&SemanticPropertyId::FIELD_FILL_VALUE];
            assert_eq!(property.value_state(), PropertyValueState::Unresolved);
            assert_eq!(property.value(), None);
        }
    }

    #[test]
    fn malformed_decimal_and_local_scalar_failure_remain_property_local() {
        use crate::versions::v2_20::native_model::NativeScalarAnnotationIssue;

        let mut root = document_fixture();
        root.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::UnresolvedScalar {
                    issue: NativeScalarAnnotationIssue::Unknown,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );
        let envelope = project_fixture(root).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();
        assert_eq!(
            document.properties[&SemanticPropertyId::FIELD_FILL_VALUE].value_state(),
            PropertyValueState::Unresolved
        );
        assert!(envelope
            .node_named(NodeKind::Attribute, "Product")
            .is_some());

        let mut malformed = document_fixture();
        malformed.properties.insert(
            "FillValue".to_string(),
            NativeProperty {
                canonical_id: "FillValue".to_string(),
                value: NativePropertyValue::AnnotatedScalar {
                    value: "1e3".to_string(),
                    type_annotation: NativeScalarType::Decimal,
                },
                provenance: NativePropertyProvenance::Explicit,
            },
        );
        let malformed = project_fixture(malformed).unwrap();
        let property = &malformed
            .node_named(NodeKind::Document, "Order")
            .unwrap()
            .properties[&SemanticPropertyId::FIELD_FILL_VALUE];
        assert_eq!(property.value_state(), PropertyValueState::Unresolved);
        assert_eq!(property.value(), None);
    }

    #[test]
    fn malformed_or_path_like_type_descriptions_fail_closed_without_leakage() {
        let error =
            type_description("<v8:Type>cfg:CatalogRef../../tmp/secret</v8:Type>").unwrap_err();

        assert_eq!(error.kind, SourceAdapterErrorKind::ProjectionAmbiguous);
        assert!(!error.message.contains("/tmp/"));
        assert!(!error.message.contains("secret"));
    }

    #[test]
    fn form_is_always_partial_and_inspection_only_before_form_internals_exist() {
        let form = node(
            "Form",
            MetadataClassRole::Form,
            Some("33333333-3333-3333-3333-333333333333"),
            "OrderForm",
            BTreeMap::new(),
            Vec::new(),
        );
        let mut root = document_fixture();
        root.children = vec![NativeMetadataChild {
            role: RelationRole::Forms,
            node: form,
        }];

        let envelope = project_fixture(root).unwrap();
        let form = envelope.node_named(NodeKind::Form, "OrderForm").unwrap();

        assert_eq!(
            envelope.status,
            crate::domain::navigation::NavigationStatus::Partial
        );
        assert!(envelope
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "partialCoverage"));
        assert_eq!(form.capability.coverage, CoverageState::Partial);
        assert_eq!(form.capability.resolution, ResolutionState::Resolved);
        assert_eq!(form.capability.authorability, Authorability::Authorable);
        assert_eq!(form.actions.len(), 1);
        assert_eq!(form.actions[0].kind, ActionKind::Inspect);
    }

    #[test]
    fn scalar_values_follow_the_property_schema_not_their_text() {
        let mut root = document_fixture();
        root.properties
            .insert("Code".to_string(), scalar("Code", "001"));
        root.properties
            .insert("UnknownScalar".to_string(), scalar("UnknownScalar", "42"));

        let envelope = project_fixture(root).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();

        assert_eq!(
            document.properties[&SemanticPropertyId::METADATA_CODE].value_type(),
            PropertyType::String
        );
        assert_eq!(
            document.properties[&SemanticPropertyId::METADATA_CODE].value(),
            Some(&PropertyValue::String("001".to_string()))
        );
        assert_eq!(
            envelope.status,
            crate::domain::navigation::NavigationStatus::Partial
        );
        assert_eq!(envelope.diagnostics[0].code, "unmappedSemanticFact");
        assert!(!document
            .properties
            .keys()
            .any(|id| id.as_str() == "unknownScalar"));
    }

    #[test]
    fn generated_contains_relations_are_derived_even_for_uuid_targets() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let attribute = envelope.node_named(NodeKind::Attribute, "Product").unwrap();
        let owning = envelope.owning_relation(&attribute.object_ref).unwrap();

        assert_eq!(owning.identity_strength, IdentityStrength::Derived);
        assert_eq!(owning.capability.identity, IdentityStrength::Derived);
    }

    #[test]
    fn support_capability_comes_from_immutable_provider_snapshot_bytes() {
        use crate::versions::v2_20::provider::PlatformXmlProvider;

        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-projector-support-{}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let provider = PlatformXmlProvider::open(&root).unwrap();
        let captured =
            support::read_support_facts_bytes(provider.parent_configurations_bytes().as_deref());
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(
            root.join("Ext/ParentConfigurations.bin"),
            b"changed-after-open",
        )
        .unwrap();
        let after_change =
            support::read_support_facts_bytes(provider.parent_configurations_bytes().as_deref());
        let snapshot = PlatformXmlNativeSnapshot {
            source: SourceSnapshot {
                source_id: SourceId::new("workspace:main").unwrap(),
                revision: provider.revision().unwrap(),
                consistency: SnapshotConsistency::Consistent,
                adapter_id: PROJECTOR_ID.to_string(),
            },
            root: document_fixture(),
            coverage: CoverageState::Complete,
        };

        let before = project(&snapshot, &captured).unwrap();
        let after = project(&snapshot, &after_change).unwrap();
        assert_eq!(
            before
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .capability
                .authorability,
            after
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .capability
                .authorability,
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn adapter_projects_locked_snapshot_support_after_live_file_becomes_editable() {
        use crate::{
            domain::source_adapters::{
                FormatVersion, SnapshotEvidence, SourceDescriptor, SourceFamily,
            },
            infrastructure::source_adapters::platform_xml::{
                provider::PlatformXmlProvider, PlatformXmlReadAdapter,
            },
        };

        const UUID: &str = "cccccccc-cccc-cccc-cccc-cccccccccccc";
        const PROVIDER: &str = "dddddddd-dddd-dddd-dddd-dddddddddddd";
        const VENDOR_CONFIGURATION: &str = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee";
        const CONFIGURATION: &str = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
        const SECOND: &str = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-projector-toctou-{}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Order.xml"),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Document uuid=\"{UUID}\"><Properties><Name>Order</Name></Properties></Document></MetaDataObject>",
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><Configuration uuid=\"{CONFIGURATION}\"><Properties><Name>Configuration</Name></Properties></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
        let support = |first_state: &str| {
            format!(
            "{{6,0,1,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,{CONFIGURATION},{first_state},0,{UUID},{UUID},1,0,{SECOND},{SECOND}}}"
        )
        };
        std::fs::create_dir_all(root.join("Ext")).unwrap();
        std::fs::write(root.join("Ext/ParentConfigurations.bin"), support("0")).unwrap();
        let provider = PlatformXmlProvider::open(&root).unwrap();
        let descriptor = SourceDescriptor {
            source_id: SourceId::new("workspace:main").unwrap(),
            family: SourceFamily::PlatformXml,
            format_version: FormatVersion::parse("2.20").unwrap(),
            producer_version: None,
            detected_features: BTreeSet::new(),
            probe_evidence: Vec::new(),
            snapshot_evidence: Some(SnapshotEvidence {
                revision: provider.revision().unwrap(),
                root_descriptor_digest: provider.digest_relative("Order.xml").unwrap(),
            }),
        };
        std::fs::write(root.join("Ext/ParentConfigurations.bin"), support("1")).unwrap();

        let envelope = PlatformXmlReadAdapter::new()
            .inspect_provider(&provider, &descriptor)
            .unwrap();
        assert_eq!(
            envelope
                .node_named(NodeKind::Document, "Order")
                .unwrap()
                .capability
                .authorability,
            Authorability::SupportLocked,
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    fn project_fixture(root: NativeMetadataNode) -> Result<NavigationEnvelope, SourceAdapterError> {
        let snapshot = PlatformXmlNativeSnapshot {
            source: SourceSnapshot {
                source_id: SourceId::new("workspace:main").unwrap(),
                revision: SourceRevision::new("sha256:fixture").unwrap(),
                consistency: SnapshotConsistency::Consistent,
                adapter_id: PROJECTOR_ID.to_string(),
            },
            root,
            coverage: CoverageState::Complete,
        };
        project(
            &snapshot,
            &support::read_support_facts(std::path::Path::new("/definitely/not/a/support/file")),
        )
    }

    fn document_fixture() -> NativeMetadataNode {
        node(
            "Document",
            MetadataClassRole::TopLevelObject,
            Some("11111111-1111-1111-1111-111111111111"),
            "Order",
            BTreeMap::from([
                ("Name".to_string(), scalar("Name", "Order")),
                ("NumberLength".to_string(), scalar("NumberLength", "11")),
            ]),
            vec![node(
                "Attribute",
                MetadataClassRole::Attribute,
                Some("22222222-2222-2222-2222-222222222222"),
                "Product",
                BTreeMap::from([("Name".to_string(), scalar("Name", "Product"))]),
                Vec::new(),
            )],
        )
    }

    fn attribute_fixture() -> NativeMetadataNode {
        node(
            "Document",
            MetadataClassRole::TopLevelObject,
            Some("11111111-1111-1111-1111-111111111111"),
            "Order",
            BTreeMap::new(),
            vec![node(
                "Attribute",
                MetadataClassRole::Attribute,
                Some("22222222-2222-2222-2222-222222222222"),
                "Product",
                BTreeMap::from([(
                    "DataType".to_string(),
                    NativeProperty {
                        canonical_id: "DataType".to_string(),
                        value: NativePropertyValue::TypeSet(
                            crate::domain::navigation::TypeSetValue {
                                variants: vec![TypeVariant::Reference {
                                    target: "Catalog.Products".to_string(),
                                }],
                            },
                        ),
                        provenance: NativePropertyProvenance::Explicit,
                    },
                )]),
                Vec::new(),
            )],
        )
    }

    fn node(
        class: &'static str,
        role: MetadataClassRole,
        uuid: Option<&str>,
        name: &str,
        properties: BTreeMap<String, NativeProperty>,
        children: Vec<NativeMetadataNode>,
    ) -> NativeMetadataNode {
        NativeMetadataNode {
            class: NativeMetadataClass {
                canonical_name: class,
                role,
            },
            uuid: uuid.map(|value| value.parse().unwrap()),
            name: name.to_string(),
            state: NativeNodeState::ResolvedInline,
            properties,
            children: children
                .into_iter()
                .map(|node| NativeMetadataChild {
                    role: fixture_child_role(&node),
                    node,
                })
                .collect(),
            backing: NativeNodeBacking::None,
        }
    }

    fn fixture_child_role(node: &NativeMetadataNode) -> RelationRole {
        match node.class.role {
            MetadataClassRole::Attribute => RelationRole::Attributes,
            MetadataClassRole::TabularSection => RelationRole::TabularSections,
            MetadataClassRole::Form => RelationRole::Forms,
            MetadataClassRole::Template => RelationRole::Templates,
            MetadataClassRole::Command => RelationRole::Commands,
            _ => RelationRole::Children,
        }
    }

    fn scalar(id: &str, value: &str) -> NativeProperty {
        NativeProperty {
            canonical_id: id.to_string(),
            value: NativePropertyValue::Scalar(value.to_string()),
            provenance: NativePropertyProvenance::Explicit,
        }
    }
}
