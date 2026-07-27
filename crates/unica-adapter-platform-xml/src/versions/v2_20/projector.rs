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
        NativeSemanticReference, PlatformXmlNativeSnapshot,
    },
    schema::MetadataClassRole,
    semantic_map::{self, NativeValueKind, PropertyMapping},
    support::{EffectiveSupportRule, SupportFacts},
};

const SCHEMA_VERSION: &str = "1";
const PROJECTOR_ID: &str = "platform-xml-2.20";

pub(crate) fn project(
    native: &PlatformXmlNativeSnapshot,
    support: &SupportFacts,
) -> Result<NavigationEnvelope, SourceAdapterError> {
    semantic_map::validate_coverage_registry()?;
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
        let resolution = node_resolution(native_node);
        let authorability = native_node
            .uuid
            .map(|uuid| self.support.authorability_for(&uuid.to_string()))
            .unwrap_or(Authorability::DerivedReadOnly);
        let mut property_projection = self.project_properties(&kind, &native_node.properties)?;
        self.add_synthetic_properties(
            kind,
            native_node,
            authorability,
            &mut property_projection,
        )?;
        let mut coverage = node_coverage(native_node, self.native.coverage);
        if property_projection.incomplete || native_node.unmapped_facts > 0 {
            coverage = CoverageState::Partial;
            self.partial = true;
        }
        for _ in 0..property_projection.unmapped_count {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "unmappedSemanticFact".to_string(),
                message: "source contains a property outside the registered semantic vocabulary"
                    .to_string(),
                details: Some(serde_json::json!({"objectRef": reference.clone()})),
            });
        }
        for _ in 0..native_node.unmapped_facts {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "unmappedSemanticFact".to_string(),
                message: "source contains a fact outside the registered semantic vocabulary"
                    .to_string(),
                details: Some(serde_json::json!({"objectRef": reference.clone()})),
            });
        }
        if property_projection.unmapped_count == 0
            && native_node.unmapped_facts == 0
            && property_projection.incomplete
        {
            self.diagnostics.push(SourceAdapterDiagnostic {
                code: "partialCoverage".to_string(),
                message: "a registered semantic property could not be resolved".to_string(),
                details: Some(serde_json::json!({"objectRef": reference.clone()})),
            });
        }
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

        for relation in &native_node.references {
            for target in &relation.targets {
                self.add_reference(&reference, target, relation.role, coverage)?;
            }
        }
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

    fn add_reference(
        &mut self,
        owner: &ObjectRef,
        native_target: &NativeSemanticReference,
        role: RelationRole,
        coverage: CoverageState,
    ) -> Result<RelationRef, SourceAdapterError> {
        self.reserve_output_relation()?;
        self.reserve_identity_item()?;
        self.reserve_identity_item()?;
        let (target_key, target_identity) = object_key(
            &self.native.source.source_id,
            None,
            native_target.kind,
            None,
            &native_target.name,
        )?;
        let target = ObjectRef::new(
            self.native.source.source_id.clone(),
            target_key,
            target_identity,
            native_target.kind,
            native_target.name.clone(),
        );
        let relation_key = relation_key(
            &self.native.source.source_id,
            &owner.object_key,
            RelationKind::References,
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
            kind: RelationKind::References,
        };
        let group_ref = RelationGroupRef::new(
            self.native.source.source_id.clone(),
            owner.clone(),
            role,
            RelationKind::References,
        )?;
        self.relations.push(SemanticRelation {
            relation_ref: relation_ref.clone(),
            group_ref,
            identity_strength: IdentityStrength::Derived,
            kind: RelationKind::References,
            role,
            source: owner.clone(),
            target,
            capability: CapabilityVector {
                resolution: ResolutionState::Resolved,
                identity: IdentityStrength::Derived,
                consistency: self.native.source.consistency.clone(),
                coverage,
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
        let mut unknown_facts = Vec::new();
        let mut unmapped_count = 0usize;
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
            let Some(mapping) = semantic_map::property_mapping(*kind, id) else {
                unmapped_count += 1;
                incomplete = true;
                unknown_facts.push(readable_unknown_fact(native_property));
                continue;
            };
            let property = project_property(mapping, native_property)?;
            incomplete |= matches!(property.value_type(), PropertyType::Unknown)
                || matches!(
                    property.value_state(),
                    crate::domain::navigation::PropertyValueState::Unresolved
                );
            if projected.insert(mapping.semantic_id, property).is_some() {
                return Err(ambiguous(
                    "multiple Platform XML properties map to one semantic property",
                ));
            }
        }
        if !unknown_facts.is_empty() {
            if projected.len() >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
                return Err(resource_limit("semantic node has too many properties"));
            }
            reserve(
                &mut self.output_properties,
                MAX_NAVIGATION_IDENTITY_ITEMS,
                "semantic properties",
            )?;
            projected.insert(
                SemanticPropertyId::UNKNOWN_FACTS,
                SemanticProperty::explicit(
                    SemanticPropertyId::UNKNOWN_FACTS,
                    PropertyValue::List(unknown_facts),
                )?
                .with_capability(PropertyCapability::ReadOnly)?,
            );
        }
        if semantic_map::is_field_kind(*kind) {
            let required = projected
                .get(&SemanticPropertyId::FIELD_FILL_CHECKING)
                .map(|fill_checking| match fill_checking.value() {
                    Some(PropertyValue::EnumSymbol(value))
                        if *value
                            == crate::domain::navigation::SemanticEnumValue::SHOW_ERROR =>
                    {
                        SemanticProperty::computed(
                            SemanticPropertyId::FIELD_REQUIRED,
                            PropertyValue::Boolean(true),
                        )
                    }
                    Some(PropertyValue::EnumSymbol(_)) => SemanticProperty::computed(
                        SemanticPropertyId::FIELD_REQUIRED,
                        PropertyValue::Boolean(false),
                    ),
                    _ => Ok(SemanticProperty::unresolved(
                        SemanticPropertyId::FIELD_REQUIRED,
                    )),
                })
                .transpose()?;
            if let Some(required) = required {
                if projected.len() >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
                    return Err(resource_limit("semantic node has too many properties"));
                }
                reserve(
                    &mut self.output_properties,
                    MAX_NAVIGATION_IDENTITY_ITEMS,
                    "semantic properties",
                )?;
                incomplete |= matches!(
                    required.value_state(),
                    crate::domain::navigation::PropertyValueState::Unresolved
                );
                let required = if required.value().is_some() {
                    required.with_capability(PropertyCapability::ReadOnly)?
                } else {
                    required
                };
                projected.insert(
                    SemanticPropertyId::FIELD_REQUIRED,
                    required,
                );
            }
        }
        if *kind == NodeKind::Catalog {
            let hierarchy_active = matches!(
                projected
                    .get(&SemanticPropertyId::CATALOG_HIERARCHICAL)
                    .and_then(SemanticProperty::value),
                Some(PropertyValue::Boolean(true))
            );
            let level_limit_active = matches!(
                projected
                    .get(&SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMITED)
                    .and_then(SemanticProperty::value),
                Some(PropertyValue::Boolean(true))
            );
            let active_limit = if hierarchy_active && level_limit_active {
                projected
                    .get(&SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_COUNT)
                    .and_then(SemanticProperty::value)
                    .and_then(|value| match value {
                        PropertyValue::Integer(value) => Some(*value),
                        _ => None,
                    })
            } else {
                None
            };
            let limit = match active_limit {
                Some(value) => SemanticProperty::computed(
                        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT,
                        PropertyValue::Integer(value),
                    )?
                    .with_capability(PropertyCapability::ReadOnly)?,
                None => {
                    SemanticProperty::absent(
                        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT,
                    )
                }
            };
            if projected.len() >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
                return Err(resource_limit("semantic node has too many properties"));
            }
            reserve(
                &mut self.output_properties,
                MAX_NAVIGATION_IDENTITY_ITEMS,
                "semantic properties",
            )?;
            projected.insert(
                SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT,
                limit,
            );
        }
        Ok(PropertyProjection {
            properties: projected,
            unmapped_count,
            incomplete,
        })
    }

    fn add_synthetic_properties(
        &mut self,
        kind: NodeKind,
        native_node: &NativeMetadataNode,
        authorability: Authorability,
        projection: &mut PropertyProjection,
    ) -> Result<(), SourceAdapterError> {
        self.insert_synthetic(
            projection,
            SemanticProperty::computed(
                SemanticPropertyId::METADATA_KIND,
                PropertyValue::String(kind.as_str().to_string()),
            )?,
        )?;
        if let Some(uuid) = native_node.uuid {
            self.insert_synthetic(
                projection,
                SemanticProperty::explicit(
                    SemanticPropertyId::METADATA_UUID,
                    PropertyValue::Uuid(uuid),
                )?,
            )?;
            let rule = self.support.effective_rule_for(&uuid.to_string());
            for property in [
                SemanticProperty::computed(
                    SemanticPropertyId::SUPPORT_STATE,
                    PropertyValue::String(support_rule_label(rule).to_string()),
                )?,
                SemanticProperty::computed(
                    SemanticPropertyId::SUPPORT_AUTHORABILITY,
                    PropertyValue::String(authorability_label(authorability).to_string()),
                )?,
                SemanticProperty::computed(
                    SemanticPropertyId::SUPPORT_EDIT_CAPABILITY,
                    PropertyValue::String(edit_capability_label(authorability).to_string()),
                )?,
            ] {
                self.insert_synthetic(projection, property)?;
            }
        }
        if kind == NodeKind::Unknown
            && !projection
                .properties
                .contains_key(&SemanticPropertyId::UNKNOWN_FACTS)
        {
            self.insert_synthetic(
                projection,
                SemanticProperty::computed(
                    SemanticPropertyId::UNKNOWN_FACTS,
                    PropertyValue::List(vec![PropertyValue::Structure(BTreeMap::from([
                        (
                            "category".to_string(),
                            PropertyValue::String("object".to_string()),
                        ),
                        (
                            "value".to_string(),
                            PropertyValue::String("readable-unknown-object".to_string()),
                        ),
                    ]))]),
                )?,
            )?;
        }
        self.add_backing_properties(kind, native_node, projection)?;
        Ok(())
    }

    fn add_backing_properties(
        &mut self,
        kind: NodeKind,
        native_node: &NativeMetadataNode,
        projection: &mut PropertyProjection,
    ) -> Result<(), SourceAdapterError> {
        let Some(mapping) = semantic_map::backing_mapping(kind) else {
            return Ok(());
        };
        let backing_matches_registry = matches!(
            (mapping.kind, &native_node.backing),
            (_, NativeNodeBacking::None)
                | (
                    semantic_map::BackingKind::Rights,
                    NativeNodeBacking::Rights(_)
                )
                | (
                    semantic_map::BackingKind::Form,
                    NativeNodeBacking::Form(_)
                )
                | (
                    semantic_map::BackingKind::Template,
                    NativeNodeBacking::Template(_)
                )
        );
        if !backing_matches_registry {
            return Err(ambiguous(
                "native backing evidence disagrees with the coverage registry",
            ));
        }
        let (descriptor, content) = match &native_node.backing {
            NativeNodeBacking::None => (None, None),
            NativeNodeBacking::Rights(content) => (None, Some(content)),
            NativeNodeBacking::Form(form) => {
                (Some(&form.descriptor), Some(&form.managed_content))
            }
            NativeNodeBacking::Template(template) => {
                (Some(&template.descriptor), Some(&template.canonical_content))
            }
        };
        if mapping.descriptor {
            let available = descriptor
                .is_some_and(|value| value.state == NativeEvidenceState::Validated);
            self.insert_synthetic(
                projection,
                SemanticProperty::computed(
                    SemanticPropertyId::BACKING_DESCRIPTOR_AVAILABLE,
                    PropertyValue::Boolean(available),
                )?,
            )?;
            if let Some(uuid) = descriptor
                .filter(|value| value.state == NativeEvidenceState::Validated)
                .and_then(|value| value.uuid)
            {
                self.insert_synthetic(
                    projection,
                    SemanticProperty::explicit(
                        SemanticPropertyId::BACKING_DESCRIPTOR_UUID,
                        PropertyValue::Uuid(uuid),
                    )?,
                )?;
            }
        }
        if mapping.content {
            let available =
                content.is_some_and(|value| value.state == NativeEvidenceState::Validated);
            self.insert_synthetic(
                projection,
                SemanticProperty::computed(
                    SemanticPropertyId::BACKING_CONTENT_AVAILABLE,
                    PropertyValue::Boolean(available),
                )?,
            )?;
            if mapping.opaque {
                self.insert_synthetic(
                    projection,
                    SemanticProperty::computed(
                        SemanticPropertyId::BACKING_CONTENT_OPAQUE,
                        PropertyValue::Boolean(available),
                    )?,
                )?;
            }
        }
        Ok(())
    }

    fn insert_synthetic(
        &mut self,
        projection: &mut PropertyProjection,
        property: SemanticProperty,
    ) -> Result<(), SourceAdapterError> {
        if projection.properties.len() >= MAX_NAVIGATION_PROPERTIES_PER_NODE {
            return Err(resource_limit("semantic node has too many properties"));
        }
        reserve(
            &mut self.output_properties,
            MAX_NAVIGATION_IDENTITY_ITEMS,
            "semantic properties",
        )?;
        let id = property.id();
        if projection
            .properties
            .insert(id, property.with_capability(PropertyCapability::ReadOnly)?)
            .is_some()
        {
            return Err(ambiguous(
                "native and derived Platform XML facts map to one semantic property",
            ));
        }
        Ok(())
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
    let mut kind = node.class.kind;
    if kind == NodeKind::Template
        && template_type(node).as_deref() == Some("SpreadsheetDocument")
    {
        kind = NodeKind::SpreadsheetDocumentTemplate;
    }
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
    if node.class.role == MetadataClassRole::Unknown
        && semantic_map::is_intentionally_partial(node.class.kind, "unknownSemantic")
    {
        return CoverageState::Partial;
    }
    if semantic_map::is_intentionally_partial(node.class.kind, "opaqueContent")
        && matches!(
            &node.backing,
            NativeNodeBacking::Form(_) | NativeNodeBacking::Template(_)
        )
    {
        return CoverageState::Partial;
    }
    if semantic_map::is_intentionally_partial(node.class.kind, "unknownValueVariant")
        && node.properties.values().any(|property| {
            matches!(
                &property.value,
                NativePropertyValue::TypeSet(value)
                    if value.variants().iter().any(|variant| variant.is_unknown())
            )
        })
    {
        return CoverageState::Partial;
    }
    match &node.backing {
        NativeNodeBacking::Rights(content)
            if !matches!(content.state, NativeEvidenceState::Validated) =>
        {
            CoverageState::Partial
        }
        NativeNodeBacking::Form(form)
            if !matches!(form.descriptor.state, NativeEvidenceState::Validated)
                || !matches!(form.managed_content.state, NativeEvidenceState::Validated)
                || semantic_map::backing_mapping(node.class.kind)
                    .is_some_and(|mapping| mapping.opaque) =>
        {
            CoverageState::Partial
        }
        NativeNodeBacking::Template(template)
            if !matches!(template.descriptor.state, NativeEvidenceState::Validated)
                || !matches!(
                    template.canonical_content.state,
                    NativeEvidenceState::Validated
                )
                || semantic_map::backing_mapping(node.class.kind)
                    .is_some_and(|mapping| mapping.opaque) =>
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
            if type_set.variants().len() > MAX_NAVIGATION_TYPE_VARIANTS {
                return Err(resource_limit("native type set has too many variants"));
            }
        }
    }
    for relation in &node.references {
        for _ in &relation.targets {
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
            reserve(
                &mut budget.identity_items,
                MAX_NAVIGATION_IDENTITY_ITEMS,
                "semantic identity items",
            )?;
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
    unmapped_count: usize,
    incomplete: bool,
}

fn readable_unknown_fact(property: &NativeProperty) -> PropertyValue {
    let value = match &property.value {
        NativePropertyValue::Scalar(value)
        | NativePropertyValue::AnnotatedScalar { value, .. } => {
            PropertyValue::String(value.clone())
        }
        NativePropertyValue::TypeSet(value) => PropertyValue::TypeSet(value.clone()),
        NativePropertyValue::LocalizedString(value) => {
            PropertyValue::LocalizedString(value.clone())
        }
        NativePropertyValue::StringList(values) => PropertyValue::List(
            values
                .iter()
                .cloned()
                .map(PropertyValue::String)
                .collect(),
        ),
        NativePropertyValue::Null => PropertyValue::Null,
        NativePropertyValue::Absent => PropertyValue::Unknown {
            summary: "absent".to_string(),
        },
        NativePropertyValue::Unresolved
        | NativePropertyValue::UnresolvedScalar { .. }
        | NativePropertyValue::Structured => PropertyValue::Unknown {
            summary: "readable-unresolved-value".to_string(),
        },
    };
    PropertyValue::Structure(BTreeMap::from([
        (
            "category".to_string(),
            PropertyValue::String("property".to_string()),
        ),
        ("value".to_string(), value),
    ]))
}

fn project_property(
    mapping: &PropertyMapping,
    property: &NativeProperty,
) -> Result<SemanticProperty, SourceAdapterError> {
    let semantic_id = mapping.semantic_id;
    let mut projected = match &property.value {
        NativePropertyValue::Absent => SemanticProperty::absent(semantic_id),
        NativePropertyValue::Unresolved => SemanticProperty::unresolved(semantic_id),
        NativePropertyValue::UnresolvedScalar { .. } => SemanticProperty::unresolved(semantic_id),
        NativePropertyValue::Scalar(value) => {
            scalar_property(mapping, value, None)?
        }
        NativePropertyValue::AnnotatedScalar {
            value,
            type_annotation,
        } => scalar_property(mapping, value, Some(*type_annotation))?,
        NativePropertyValue::TypeSet(type_set) => {
            SemanticProperty::explicit(semantic_id, PropertyValue::TypeSet(type_set.clone()))?
        }
        NativePropertyValue::LocalizedString(values) => SemanticProperty::explicit(
            semantic_id,
            PropertyValue::LocalizedString(values.clone()),
        )?,
        NativePropertyValue::StringList(values) => SemanticProperty::explicit(
            semantic_id,
            PropertyValue::List(
                values
                    .iter()
                    .cloned()
                    .map(PropertyValue::String)
                    .collect(),
            ),
        )?,
        NativePropertyValue::Null => {
            SemanticProperty::explicit(semantic_id, PropertyValue::Null)?
        }
        NativePropertyValue::Structured => SemanticProperty::unresolved(semantic_id),
    };
    if projected.value().is_some() {
        projected = projected.with_capability(PropertyCapability::ReadOnly)?;
    }
    Ok(projected)
}

fn scalar_property(
    mapping: &PropertyMapping,
    value: &str,
    type_annotation: Option<NativeScalarType>,
) -> Result<SemanticProperty, SourceAdapterError> {
    let semantic_id = mapping.semantic_id;
    let definition = crate::domain::navigation::property_definition(semantic_id);
    if definition.allowed_types() == [PropertyType::Enum] {
        return Ok(semantic_map::enum_value(value)
            .map(PropertyValue::EnumSymbol)
            .map(|value| SemanticProperty::explicit(semantic_id, value))
            .transpose()?
            .unwrap_or_else(|| SemanticProperty::unresolved(semantic_id)));
    }
    let value = match mapping.value_kind {
        NativeValueKind::Boolean => match value {
            "true" => PropertyValue::Boolean(true),
            "false" => PropertyValue::Boolean(false),
            _ => return Ok(SemanticProperty::unresolved(semantic_id)),
        },
        NativeValueKind::Integer => match value.parse() {
            Ok(value) => PropertyValue::Integer(value),
            Err(_) => return Ok(SemanticProperty::unresolved(semantic_id)),
        },
        NativeValueKind::Uuid => match value.parse() {
            Ok(value) => PropertyValue::Uuid(value),
            Err(_) => return Ok(SemanticProperty::unresolved(semantic_id)),
        },
        NativeValueKind::String => PropertyValue::String(value.to_string()),
        NativeValueKind::Polymorphic => match type_annotation {
            Some(NativeScalarType::Decimal) => match normalize_xml_schema_decimal(value) {
                Some(value) => PropertyValue::Decimal(value),
                None => return Ok(SemanticProperty::unresolved(semantic_id)),
            },
            Some(NativeScalarType::String) => PropertyValue::String(value.to_string()),
            Some(NativeScalarType::Boolean) => match value {
                "true" => PropertyValue::Boolean(true),
                "false" => PropertyValue::Boolean(false),
                _ => return Ok(SemanticProperty::unresolved(semantic_id)),
            },
            Some(NativeScalarType::Integer) => match value.parse() {
                Ok(value) => PropertyValue::Integer(value),
                Err(_) => return Ok(SemanticProperty::unresolved(semantic_id)),
            },
            Some(NativeScalarType::Uuid) => match value.parse() {
                Ok(value) => PropertyValue::Uuid(value),
                Err(_) => return Ok(SemanticProperty::unresolved(semantic_id)),
            },
            Some(NativeScalarType::Date) => PropertyValue::Date(value.to_string()),
            None => {
                return Ok(SemanticProperty::unresolved(semantic_id));
            }
        },
        NativeValueKind::Enum
        | NativeValueKind::LocalizedString
        | NativeValueKind::TypeSet
        | NativeValueKind::StringList => return Ok(SemanticProperty::unresolved(semantic_id)),
    };
    if definition.accepts(value.value_type()) {
        Ok(SemanticProperty::explicit(semantic_id, value)
            .unwrap_or_else(|_| SemanticProperty::unresolved(semantic_id)))
    } else {
        Ok(SemanticProperty::unresolved(semantic_id))
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

fn support_rule_label(rule: EffectiveSupportRule) -> &'static str {
    match rule {
        EffectiveSupportRule::Absent => "absent",
        EffectiveSupportRule::Removed => "removed",
        EffectiveSupportRule::Editable => "editable",
        EffectiveSupportRule::Locked => "locked",
        EffectiveSupportRule::ConfigurationReadOnly => "configurationReadOnly",
        EffectiveSupportRule::UnknownReadOnly => "unknownReadOnly",
        EffectiveSupportRule::Unreadable => "unreadable",
    }
}

fn authorability_label(authorability: Authorability) -> &'static str {
    match authorability {
        Authorability::Authorable => "authorable",
        Authorability::SupportLocked => "supportLocked",
        Authorability::ConfigurationReadOnly => "configurationReadOnly",
        Authorability::UnknownSupportState => "unknownSupportState",
        Authorability::UnknownReadOnly => "unknownReadOnly",
        Authorability::DerivedReadOnly => "derivedReadOnly",
    }
}

fn edit_capability_label(authorability: Authorability) -> &'static str {
    match authorability {
        Authorability::Authorable => "authorable",
        Authorability::SupportLocked
        | Authorability::ConfigurationReadOnly
        | Authorability::DerivedReadOnly => "readOnly",
        Authorability::UnknownSupportState | Authorability::UnknownReadOnly => "unknown",
    }
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
            type_set.variants()[0],
            TypeVariant::reference(NodeKind::Catalog, "Products").unwrap()
        );
    }

    #[test]
    fn type_descriptions_accept_declared_qualifiers_and_enum_references() {
        let type_set = type_description(
            "<v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers><v8:Type>cfg:EnumRef.Statuses</v8:Type>",
        )
        .unwrap();

        assert_eq!(
            type_set.variants(),
            vec![
                TypeVariant::enumeration("Statuses").unwrap(),
                TypeVariant::primitive(
                    PrimitiveTypeKind::String,
                    Some(TypeQualifiers::String(
                        StringQualifiers::new(Some(10), Some(StringLength::Variable)).unwrap(),
                    )),
                )
                .unwrap(),
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
                            crate::domain::navigation::TypeSetValue::new(vec![
                                TypeVariant::reference(NodeKind::Catalog, "Products").unwrap(),
                            ])
                            .unwrap(),
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
            references: Vec::new(),
            children: children
                .into_iter()
                .map(|node| NativeMetadataChild {
                    role: fixture_child_role(&node),
                    node,
                })
                .collect(),
            unmapped_facts: 0,
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
