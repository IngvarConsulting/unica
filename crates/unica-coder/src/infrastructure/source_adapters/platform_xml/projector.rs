use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::{
    navigation::{
        action_profile_for, ActionAvailability, Atomicity, Authorability, CapabilityState,
        CapabilityVector, CoverageState, FormatCompatibility, IdentityStrength, NavigationEnvelope,
        NavigationFacetVisibility, NavigationNode, NodeKind, ObjectKey, ObjectRef, PropertyCapability, PropertyProvenance,
        PropertyType, PropertyValue, RelationGroupRef, RelationKey, RelationKind, RelationRef,
        RelationRole,
        ResolutionState, SemanticAction, SemanticActionKind, SemanticProperty, SemanticRelation,
    },
    source_adapters::{SourceAccess, SourceAdapterError, SourceAdapterErrorKind, SourceId},
};

use super::{
    native_model::{
        NativeEvidenceState, NativeMetadataChild, NativeMetadataNode, NativeNodeBacking, NativeNodeState,
        NativeProperty, NativePropertyProvenance, NativePropertyValue, PlatformXmlNativeSnapshot,
        NativeScalarType,
    },
    schema::{
        is_type_property_2_20, parse_type_description_2_20, scalar_property_kind_2_20,
        MetadataClassRole, ScalarPropertyKind,
    },
    support::SupportFacts,
};

const SCHEMA_VERSION: &str = "1";
const PROJECTOR_ID: &str = "platform-xml-2.20";

pub(crate) fn project(
    native: &PlatformXmlNativeSnapshot,
    support: &SupportFacts,
) -> Result<NavigationEnvelope, SourceAdapterError> {
    if native.source.adapter_id != PROJECTOR_ID {
        return Err(ambiguous("Platform XML projection requires the exact 2.20 decoder"));
    }

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
        }
    }

    fn source_root(&mut self) -> Result<ObjectRef, SourceAdapterError> {
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
            action_profile: action_profile_for(&NodeKind::SourceRoot),
            semantic_actions: Vec::new(),
            actions: vec![modeled_action(SemanticActionKind::Inspect, reference.clone(), None)],
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
        let kind = node_kind(native_node);
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
        let coverage = node_coverage(native_node, self.native.coverage);
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
            properties: properties(&native_node.properties)?,
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

    fn project_child(&mut self, child: &NativeMetadataChild, owner: &ObjectRef) -> Result<ObjectRef, SourceAdapterError> {
        self.project_node(&child.node, Some(owner), child.role)
    }

    fn add_contains(
        &mut self,
        owner: &ObjectRef,
        target: &ObjectRef,
        role: RelationRole,
    ) -> Result<RelationRef, SourceAdapterError> {
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
            self.native.source.source_id.clone(), owner.clone(), role, RelationKind::Contains,
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
        if self.object_keys.insert(key.as_str().to_string()) {
            Ok(())
        } else {
            Err(SourceAdapterError::new(
                SourceAdapterErrorKind::IdentityCollision,
                "duplicate generated semantic object key",
            ))
        }
    }

    fn finish(self, root: ObjectRef) -> Result<NavigationEnvelope, SourceAdapterError> {
        Ok(NavigationEnvelope {
            schema_version: SCHEMA_VERSION.to_string(),
            status: crate::domain::navigation::NavigationStatus::Available,
            snapshot: Some(self.native.source.clone()),
            root: Some(root),
            nodes: self.nodes,
            relations: Vec::new(),
            diagnostics: Vec::new(),
            relation_index: self.relations,
        })
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
        return Ok((ObjectKey::new(format!("uuid:{uuid}"))?, IdentityStrength::Persistent));
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
    match kind {
        NodeKind::SourceRoot => "sourceRoot".to_string(),
        NodeKind::Document => "document".to_string(),
        NodeKind::MetadataObject { metadata_type } => format!("metadataObject:{metadata_type}"),
        NodeKind::Attribute => "attribute".to_string(),
        NodeKind::TabularSection => "tabularSection".to_string(),
        NodeKind::Command => "command".to_string(),
        NodeKind::Form => "form".to_string(),
        NodeKind::FormAttribute => "formAttribute".to_string(),
        NodeKind::FormCommand => "formCommand".to_string(),
        NodeKind::FormElement => "formElement".to_string(),
        NodeKind::Template { template_type } => format!("template:{}", template_type.as_deref().unwrap_or("unknown")),
    }
}

fn validate_name(name: &str) -> Result<(), SourceAdapterError> {
    if name.is_empty() || name.chars().any(char::is_control) || name.contains(['/', '\\']) {
        return Err(ambiguous("Platform XML node has an invalid semantic name"));
    }
    Ok(())
}

fn node_kind(node: &NativeMetadataNode) -> NodeKind {
    match node.class.role {
        MetadataClassRole::TopLevelObject if node.class.canonical_name == "Document" => NodeKind::Document,
        MetadataClassRole::TopLevelObject | MetadataClassRole::Configuration => {
            NodeKind::metadata_object(node.class.canonical_name)
        }
        MetadataClassRole::Attribute => NodeKind::Attribute,
        MetadataClassRole::TabularSection => NodeKind::TabularSection,
        MetadataClassRole::Command => NodeKind::Command,
        MetadataClassRole::Form => NodeKind::Form,
        MetadataClassRole::Template => NodeKind::Template {
            template_type: template_type(node),
        },
    }
}

fn template_type(node: &NativeMetadataNode) -> Option<String> {
    let NativeNodeBacking::Template(template) = &node.backing else {
        return None;
    };
    match (&template.descriptor.state, &template.canonical_content.state, &template.descriptor_type, template.mxl_root_kind) {
        (NativeEvidenceState::Validated, NativeEvidenceState::Validated, NativePropertyValue::Scalar(value), Some(_))
            if value == "SpreadsheetDocument" => Some(value.clone()),
        _ => None,
    }
}

fn node_resolution(node: &NativeMetadataNode) -> ResolutionState {
    match node.state {
        NativeNodeState::UnresolvedRegistration { .. } => ResolutionState::Unresolved,
        NativeNodeState::ResolvedInline | NativeNodeState::ResolvedRegistration { .. } => ResolutionState::Resolved,
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
                || !matches!(form.managed_content.state, NativeEvidenceState::Validated) => CoverageState::Partial,
        NativeNodeBacking::Template(template)
            if !matches!(template.descriptor.state, NativeEvidenceState::Validated)
                || !matches!(template.canonical_content.state, NativeEvidenceState::Validated) => CoverageState::Partial,
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

fn properties(
    native: &BTreeMap<String, NativeProperty>,
) -> Result<BTreeMap<String, SemanticProperty>, SourceAdapterError> {
    native
        .iter()
        .map(|(id, native_property)| Ok((property_name(id), project_property(native_property)?)))
        .collect()
}

fn property_name(id: &str) -> String {
    let mut chars = id.chars();
    let Some(first) = chars.next() else { return String::new() };
    first.to_lowercase().chain(chars).collect()
}

fn project_property(property: &NativeProperty) -> Result<SemanticProperty, SourceAdapterError> {
    let provenance = match property.provenance {
        NativePropertyProvenance::Explicit => PropertyProvenance::Descriptor,
        NativePropertyProvenance::Absent => PropertyProvenance::Descriptor,
        NativePropertyProvenance::Unresolved => PropertyProvenance::Unknown,
    };
    let mut projected = match &property.value {
        NativePropertyValue::Absent => SemanticProperty::absent(PropertyType::Unknown, provenance),
        NativePropertyValue::Unresolved => SemanticProperty::unresolved(PropertyType::Unknown, provenance),
        NativePropertyValue::UnresolvedScalar { .. } => {
            SemanticProperty::unresolved(PropertyType::Unknown, provenance)
        }
        NativePropertyValue::Scalar(value) => {
            scalar_property(&property.canonical_id, value, None, provenance)?
        }
        NativePropertyValue::AnnotatedScalar { value, type_annotation } => {
            scalar_property(&property.canonical_id, value, Some(*type_annotation), provenance)?
        }
        NativePropertyValue::RawXml(xml) if is_type_property_2_20(&property.canonical_id) => {
            SemanticProperty::explicit(
                PropertyType::TypeSet,
                PropertyValue::TypeSet(parse_type_description_2_20(xml)?),
                provenance,
            )?
        }
        NativePropertyValue::RawXml(_) => SemanticProperty::explicit(
            PropertyType::Unknown,
            PropertyValue::Unknown { summary: "non-scalar XML property".to_string() },
            provenance,
        )?,
    };
    projected.capability = PropertyCapability::ReadOnly;
    Ok(projected)
}

fn scalar_property(
    canonical_id: &str,
    value: &str,
    type_annotation: Option<NativeScalarType>,
    provenance: PropertyProvenance,
) -> Result<SemanticProperty, SourceAdapterError> {
    let Some(kind) = scalar_property_kind_2_20(canonical_id) else {
        return SemanticProperty::explicit(
            PropertyType::Unknown,
            PropertyValue::Unknown { summary: "unrecognized Platform XML scalar property".to_string() },
            provenance,
        );
    };
    let (value_type, value) = match kind {
        ScalarPropertyKind::Boolean => match value {
            "true" => (PropertyType::Boolean, PropertyValue::Boolean(true)),
            "false" => (PropertyType::Boolean, PropertyValue::Boolean(false)),
            _ => return Err(ambiguous("invalid boolean Platform XML scalar property")),
        },
        ScalarPropertyKind::Integer => (
            PropertyType::Integer,
            PropertyValue::Integer(value.parse().map_err(|_| ambiguous("invalid integer Platform XML scalar property"))?),
        ),
        ScalarPropertyKind::Uuid => (
            PropertyType::Uuid,
            PropertyValue::Uuid(value.parse().map_err(|_| ambiguous("invalid UUID Platform XML scalar property"))?),
        ),
        ScalarPropertyKind::String => (PropertyType::String, PropertyValue::String(value.to_string())),
        ScalarPropertyKind::PolymorphicFillValue => match type_annotation {
            Some(NativeScalarType::Decimal) => match normalize_xml_schema_decimal(value) {
                Some(value) => (PropertyType::Decimal, PropertyValue::Decimal(value)),
                None => return Ok(SemanticProperty::unresolved(PropertyType::Unknown, provenance)),
            },
            Some(NativeScalarType::String) => {
                (PropertyType::String, PropertyValue::String(value.to_string()))
            }
            None | Some(_) => {
                return Ok(SemanticProperty::unresolved(PropertyType::Unknown, provenance));
            }
        },
    };
    SemanticProperty::explicit(value_type, value, provenance)
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
    Some(format!("{}{}.{}", if negative && !is_zero { "-" } else { "" }, integer, fraction))
}

fn ambiguous(message: impl Into<String>) -> SourceAdapterError {
    SourceAdapterError::new(SourceAdapterErrorKind::ProjectionAmbiguous, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        domain::{
            navigation::{ActionKind, FormatCompatibility, PropertyType, PropertyValue, PropertyValueState, RelationKind, TypeVariant},
            source_adapters::{SnapshotConsistency, SourceId, SourceRevision, SourceSnapshot},
        },
        infrastructure::source_adapters::platform_xml::{
            native_model::{NativeMetadataClass, NativePropertyProvenance},
            schema::MetadataClassRole,
            support,
        },
    };

    #[test]
    fn root_metadata_object_has_an_owning_relation() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();
        let owning = envelope.owning_relation(&document.object_ref).unwrap();

        assert_eq!(owning.kind, RelationKind::Contains);
        assert_eq!(owning.source.kind, NodeKind::SourceRoot);
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

        assert!(envelope.nodes.iter().all(|node| {
            node.capability.format == FormatCompatibility::Compatible
        }));
    }

    #[test]
    fn document_properties_are_typed_for_ai_consumption() {
        let envelope = project_fixture(document_fixture()).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();

        assert_eq!(document.properties["numberLength"].value_type, PropertyType::Integer);
        assert_eq!(document.properties["numberLength"].value, Some(PropertyValue::Integer(11)));
        assert_eq!(document.properties["numberLength"].value_state, PropertyValueState::Explicit);
    }

    #[test]
    fn one_c_type_descriptions_are_structured_not_strings() {
        let envelope = project_fixture(attribute_fixture()).unwrap();
        let attribute = envelope.node_named(NodeKind::Attribute, "Product").unwrap();

        let PropertyValue::TypeSet(type_set) = attribute.properties["dataType"].value.clone().unwrap() else {
            panic!("expected structured type set");
        };
        assert_eq!(type_set.variants[0], TypeVariant::Reference { target: "Catalog.Products".to_string() });
    }

    #[test]
    fn type_descriptions_accept_declared_qualifiers_and_enum_references() {
        let type_set = parse_type_description_2_20(
            "<DataType><Type>xs:string</Type><StringQualifiers><Length>10</Length><AllowedLength>Variable</AllowedLength></StringQualifiers><Type>cfg:EnumRef.Statuses</Type></DataType>",
        )
        .unwrap();

        assert_eq!(
            type_set.variants,
            vec![
                TypeVariant::Enumeration { target: "Enum.Statuses".to_string() },
                TypeVariant::Primitive {
                    kind: "String".to_string(),
                    qualifiers: BTreeMap::from([
                        ("allowedLength".to_string(), PropertyValue::EnumSymbol("Variable".to_string())),
                        ("length".to_string(), PropertyValue::Integer(10)),
                    ]),
                },
            ],
        );
    }

    #[test]
    fn incompatible_type_qualifiers_fail_closed() {
        for raw in [
            "<DataType><Type>xs:boolean</Type><StringQualifiers><Length>10</Length></StringQualifiers></DataType>",
            "<DataType><Type>CatalogRef.Products</Type><NumberQualifiers><Digits>10</Digits></NumberQualifiers></DataType>",
        ] {
            assert_eq!(
                parse_type_description_2_20(raw).unwrap_err().kind,
                SourceAdapterErrorKind::ProjectionAmbiguous,
            );
        }
    }

    #[test]
    fn fill_value_uses_exact_native_scalar_annotation_not_text() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarType;

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
            decimal.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"].value,
            Some(PropertyValue::Decimal("0.0".to_string())),
        );
        assert_eq!(
            string.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"].value,
            Some(PropertyValue::String("true".to_string())),
        );
    }

    #[test]
    fn fill_value_without_a_known_annotation_is_unresolved() {
        let mut root = document_fixture();
        root.properties.insert("FillValue".to_string(), scalar("FillValue", "true"));

        let envelope = project_fixture(root).unwrap();
        let property = &envelope.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"];
        assert_eq!(property.value_state, PropertyValueState::Unresolved);
        assert_eq!(property.value, None);
    }

    #[test]
    fn empty_annotated_fill_value_preserves_string_but_not_invalid_decimal() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::{
            NativeScalarAnnotationIssue, NativeScalarType,
        };

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
            string.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"].value,
            Some(PropertyValue::String(String::new())),
        );
        let decimal = project_fixture(decimal).unwrap();
        let property = &decimal.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"];
        assert_eq!(property.value_state, PropertyValueState::Unresolved);
        assert_eq!(property.value, None);
    }

    #[test]
    fn fill_value_accepts_only_lossless_decimal_or_string_annotations() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarType;

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
            decimal.node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"].value,
            Some(PropertyValue::Decimal("1.23".to_string())),
        );

        for annotation in [NativeScalarType::Boolean, NativeScalarType::Integer, NativeScalarType::Uuid] {
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
                .node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"];
            assert_eq!(property.value_state, PropertyValueState::Unresolved);
            assert_eq!(property.value, None);
        }
    }

    #[test]
    fn malformed_decimal_and_local_scalar_failure_remain_property_local() {
        use crate::infrastructure::source_adapters::platform_xml::native_model::NativeScalarAnnotationIssue;

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
        assert_eq!(document.properties["fillValue"].value_state, PropertyValueState::Unresolved);
        assert!(envelope.node_named(NodeKind::Attribute, "Product").is_some());

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
            .node_named(NodeKind::Document, "Order").unwrap().properties["fillValue"];
        assert_eq!(property.value_state, PropertyValueState::Unresolved);
        assert_eq!(property.value, None);
    }

    #[test]
    fn malformed_or_path_like_type_descriptions_fail_closed_without_leakage() {
        let error = parse_type_description_2_20(
            "<DataType><Type>CatalogRef../../tmp/secret</Type></DataType>",
        )
        .unwrap_err();

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
        root.children = vec![NativeMetadataChild { role: RelationRole::Forms, node: form }];

        let envelope = project_fixture(root).unwrap();
        let form = envelope.node_named(NodeKind::Form, "OrderForm").unwrap();

        assert_eq!(form.capability.coverage, CoverageState::Partial);
        assert_eq!(form.capability.resolution, ResolutionState::Resolved);
        assert_eq!(form.capability.authorability, Authorability::Authorable);
        assert_eq!(form.actions.len(), 1);
        assert_eq!(form.actions[0].kind, ActionKind::Inspect);
    }

    #[test]
    fn scalar_values_follow_the_property_schema_not_their_text() {
        let mut root = document_fixture();
        root.properties.insert("Code".to_string(), scalar("Code", "001"));
        root.properties.insert("UnknownScalar".to_string(), scalar("UnknownScalar", "42"));

        let envelope = project_fixture(root).unwrap();
        let document = envelope.node_named(NodeKind::Document, "Order").unwrap();

        assert_eq!(document.properties["code"].value_type, PropertyType::String);
        assert_eq!(document.properties["code"].value, Some(PropertyValue::String("001".to_string())));
        assert_eq!(document.properties["unknownScalar"].value_type, PropertyType::Unknown);
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
        use crate::infrastructure::source_adapters::platform_xml::provider::PlatformXmlProvider;

        let root = std::env::temp_dir().join(format!(
            "unica-platform-xml-projector-support-{}",
            std::process::id(),
        ));
        std::fs::create_dir_all(&root).unwrap();
        let provider = PlatformXmlProvider::open(&root).unwrap();
        let captured = support::read_support_facts_bytes(
            provider.parent_configurations_bytes().as_deref(),
        );
        std::fs::write(root.join("ParentConfigurations.bin"), b"changed-after-open").unwrap();
        let after_change = support::read_support_facts_bytes(
            provider.parent_configurations_bytes().as_deref(),
        );
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
            before.node_named(NodeKind::Document, "Order").unwrap().capability.authorability,
            after.node_named(NodeKind::Document, "Order").unwrap().capability.authorability,
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
        let support = |first_state: &str| format!(
            "{{6,0,1,{PROVIDER},0,{VENDOR_CONFIGURATION},\"1.0\",\"Vendor\",\"VendorConf\",3,1,1,{CONFIGURATION},{CONFIGURATION},0,{first_state},{UUID},{UUID},2,1,{SECOND},{SECOND}}}"
        );
        std::fs::write(root.join("ParentConfigurations.bin"), support("0")).unwrap();
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
        std::fs::write(root.join("ParentConfigurations.bin"), support("1")).unwrap();

        let envelope = PlatformXmlReadAdapter::new()
            .inspect_provider(&provider, &descriptor)
            .unwrap();
        assert_eq!(
            envelope.node_named(NodeKind::Document, "Order").unwrap().capability.authorability,
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
            &support::read_support_facts(std::path::Path::new(
                "/definitely/not/a/support/file",
            )),
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
                        value: NativePropertyValue::RawXml(
                            "<DataType><Type>CatalogRef.Products</Type></DataType>".to_string(),
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
            class: NativeMetadataClass { canonical_name: class, role },
            uuid: uuid.map(|value| value.parse().unwrap()),
            name: name.to_string(),
            state: NativeNodeState::ResolvedInline,
            properties,
            children: children.into_iter().map(|node| NativeMetadataChild { role: fixture_child_role(&node), node }).collect(),
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
