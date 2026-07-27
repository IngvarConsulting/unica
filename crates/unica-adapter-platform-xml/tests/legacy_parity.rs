use std::{collections::BTreeSet, fs, path::{Path, PathBuf}, sync::atomic::{AtomicU64, Ordering}};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{FacetSelection, NavigationEnvelope, NavigationNode, NavigationQuery, NavigationSelection, NavigationStatus, NavigationTarget, PropertySelection, PropertyValueState},
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{SemanticEnumValue, SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{SourceContext, SourceFamily, SourceLocation},
    value::{PrimitiveTypeKind, PropertyValue},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[test]
fn typed_registry_manifest_is_runtime_checked_and_corpus_covers_every_top_level_kind() {
    let envelope = read_tracked("all_kinds/Configuration.xml");
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let expected = expected_inventory();
    let configuration = node(
        &envelope,
        SemanticObjectKind::Configuration,
        "CorpusConfiguration",
    );
    let actual_top_level = envelope
        .relation_index
        .iter()
        .filter(|relation| {
            relation.source == configuration.object_ref
                && relation.role == SemanticRelationId::CHILDREN
        })
        .map(|relation| {
            (
                relation.target.kind.as_str().to_string(),
                relation.target.display_name.clone(),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected_top_level = expected["allKinds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pair| pair[0].as_str() != Some("configuration"))
        .map(|pair| {
            (
                pair[0].as_str().unwrap().to_string(),
                pair[1].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_top_level, expected_top_level,
        "supported top-level kind corpus must be an exact inventory"
    );
    let contract_facts = envelope_contract_fact_set(&envelope);
    assert_eq!(
        contract_facts,
        expected_fact_set(&expected["allKindsEnvelopeFacts"]),
        "supported-kind coverage/status/diagnostics drifted"
    );

    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/coverage.json");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schemaVersion"], 2);
    for section in ["objects", "properties", "relationProperties", "children", "enumAliases", "typeVariants", "backingArtifacts", "intentionalPartialCases"] {
        assert!(manifest[section].as_array().is_some_and(|entries| !entries.is_empty()), "missing typed coverage section {section}");
    }
    let expected_enums = expected["enumSemanticInventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let actual_enums = manifest["enumAliases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["semantic"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_enums, expected_enums, "enum coverage drifted from the frozen legacy inventory");
    let expected_types = expected["typeAliasInventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let actual_types = manifest["typeVariants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            format!(
                "{}:{}:{}:{}",
                entry["namespace"].as_str().unwrap(),
                entry["alias"].as_str().unwrap(),
                entry["category"].as_str().unwrap(),
                entry["targetKind"].as_str().unwrap_or("")
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_types, expected_types, "type alias coverage drifted from the frozen legacy inventory");
}

#[test]
fn coverage_manifest_mutations_cannot_drift_from_the_runtime_registry() {
    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/coverage.json");
    let raw = fs::read_to_string(manifest_path).unwrap();
    PlatformXmlAdapterFactory::validate_2_20_coverage_manifest(&raw)
        .expect("checked-in coverage must exactly match runtime dispatch");

    assert_manifest_mutation_rejected(&raw, "removed property", |manifest| {
        manifest["properties"].as_array_mut().unwrap().remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "extra property", |manifest| {
        let extra = manifest["properties"][0].clone();
        manifest["properties"].as_array_mut().unwrap().push(extra);
    });
    assert_manifest_mutation_rejected(&raw, "removed relation property", |manifest| {
        manifest["relationProperties"].as_array_mut().unwrap().remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "extra relation property", |manifest| {
        let extra = manifest["relationProperties"][0].clone();
        manifest["relationProperties"].as_array_mut().unwrap().push(extra);
    });
    assert_manifest_mutation_rejected(&raw, "removed enum alias", |manifest| {
        manifest["enumAliases"][0]["nativeAliases"].as_array_mut().unwrap().remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "extra enum alias", |manifest| {
        manifest["enumAliases"][0]["nativeAliases"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!("FutureAlias"));
    });
    assert_manifest_mutation_rejected(&raw, "removed owner role", |manifest| {
        let entry = manifest["children"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["ownerRoles"].as_array().is_some_and(|roles| !roles.is_empty()))
            .unwrap();
        entry["ownerRoles"].as_array_mut().unwrap().remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "changed backing kind", |manifest| {
        manifest["backingArtifacts"][0]["kind"] = serde_json::json!("future");
    });
    assert_manifest_mutation_rejected(&raw, "removed intentional partial rule", |manifest| {
        manifest["intentionalPartialCases"].as_array_mut().unwrap().remove(0);
    });

    for (label, section, entry, field) in [
        ("property applicability", "properties", 0, "objectKinds"),
        ("property aliases", "properties", 0, "nativeNames"),
        ("relation applicability", "relationProperties", 0, "objectKinds"),
        ("relation aliases", "relationProperties", 0, "nativeNames"),
        ("enum applicability", "enumAliases", 0, "propertyIds"),
        ("enum aliases", "enumAliases", 0, "nativeAliases"),
        ("backing applicability", "backingArtifacts", 0, "objectKinds"),
        ("partial applicability", "intentionalPartialCases", 0, "objectKinds"),
    ] {
        assert_manifest_mutation_rejected(&raw, label, |manifest| {
            manifest[section][entry][field] = serde_json::json!([]);
        });
    }
    for section in [
        "objects",
        "properties",
        "relationProperties",
        "children",
        "enumAliases",
        "typeVariants",
        "backingArtifacts",
        "intentionalPartialCases",
    ] {
        assert_manifest_mutation_rejected(
            &raw,
            &format!("empty mandatory section {section}"),
            |manifest| {
                manifest[section] = serde_json::json!([]);
            },
        );
    }
}

#[test]
fn real_tracked_platform_xml_files_match_frozen_identity_inventory() {
    let expected = expected_inventory();
    let source_root = repo_root().join("tests/fixtures/unica_mcp_script_parity/bsp/meta");
    for case in expected["realFixtureNodes"].as_array().unwrap() {
        let target = source_root.join(case[0].as_str().unwrap());
        let envelope = read_path(&source_root, &target);
        let kind = SemanticObjectKind::parse(case[1].as_str().unwrap()).unwrap();
        node(&envelope, kind, case[2].as_str().unwrap());
        assert_ne!(envelope.status, NavigationStatus::Unavailable);
        let facts = legacy_baseline_fact_set(&envelope);
        let expected_facts =
            expected_fact_set(&expected["realFixtureFacts"][case[0].as_str().unwrap()]);
        assert_eq!(
            facts,
            expected_facts,
            "{} drifted from its exact frozen legacy information set",
            case[0].as_str().unwrap()
        );
    }
}

#[test]
fn real_currencies_fixture_preserves_hierarchy_controls_without_false_activation() {
    let source_root = repo_root().join("tests/fixtures/unica_mcp_script_parity/bsp/meta");
    let envelope = read_path(&source_root, &source_root.join("Catalogs/Валюты.xml"));
    let facts = semantic_fact_set(&envelope);
    for expected in expected_inventory()["currencyFacts"].as_array().unwrap() {
        assert!(facts.contains(expected.as_str().unwrap()), "missing independent legacy fact {expected}");
    }

    let attribute = node(&envelope, SemanticObjectKind::Attribute, "ОсновнаяВалюта");
    let fill_value = attribute.properties[&SemanticPropertyId::FIELD_FILL_VALUE]
        .value()
        .expect("tracked DesignTimeRef EmptyRef must not be absent or unresolved");
    assert_eq!(fill_value, &PropertyValue::EmptyReference);
    assert_eq!(
        serde_json::from_str::<PropertyValue>(&serde_json::to_string(fill_value).unwrap()).unwrap(),
        PropertyValue::EmptyReference,
        "empty reference must round-trip distinctly from null"
    );
    assert_ne!(fill_value, &PropertyValue::Null);
}

#[test]
fn adversarial_hierarchy_combinations_keep_configured_and_active_facts_distinct() {
    let unlimited = read_tracked("hierarchy/EnabledUnlimited.xml");
    let limited = read_tracked("hierarchy/EnabledLimited.xml");
    let disabled = read_tracked("hierarchy/DisabledContradiction.xml");

    let unlimited = node(&unlimited, SemanticObjectKind::Catalog, "EnabledUnlimited");
    assert_value(unlimited, SemanticPropertyId::CATALOG_HIERARCHICAL, PropertyValue::Boolean(true));
    assert_value(unlimited, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMITED, PropertyValue::Boolean(false));
    assert_absent(unlimited, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT);
    assert_value(unlimited, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_COUNT, PropertyValue::Integer(7));

    let limited = node(&limited, SemanticObjectKind::Catalog, "EnabledLimited");
    assert_value(limited, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT, PropertyValue::Integer(4));

    let disabled = node(&disabled, SemanticObjectKind::Catalog, "DisabledContradiction");
    assert_value(disabled, SemanticPropertyId::CATALOG_HIERARCHICAL, PropertyValue::Boolean(false));
    assert_value(disabled, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMITED, PropertyValue::Boolean(true));
    assert_absent(disabled, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT);
    assert_value(disabled, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_COUNT, PropertyValue::Integer(9));
}

#[test]
fn complete_legacy_enum_aliases_map_with_property_specific_applicability() {
    let document = read_inline(
        "enum-whole-catalog",
        "WholeCatalog.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="71000000-0000-0000-0000-000000000001"><Properties><Name>WholeCatalog</Name><NumberPeriodicity>WholeCatalog</NumberPeriodicity></Properties></Document></MetaDataObject>"#,
    );
    assert_value(
        node(&document, SemanticObjectKind::Document, "WholeCatalog"),
        SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY,
        PropertyValue::EnumSymbol(SemanticEnumValue::WHOLE_COLLECTION),
    );

    let catalog = read_inline(
        "enum-folder-use",
        "FolderUse.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="71000000-0000-0000-0000-000000000002"><Properties><Name>FolderUse</Name></Properties><ChildObjects><Attribute uuid="71000000-0000-0000-0000-000000000003"><Properties><Name>FolderOnly</Name><Use>ForFolder</Use></Properties></Attribute><Attribute uuid="71000000-0000-0000-0000-000000000004"><Properties><Name>FolderAndItem</Name><Use>ForFolderAndItem</Use></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#,
    );
    assert_value(
        node(&catalog, SemanticObjectKind::Attribute, "FolderOnly"),
        SemanticPropertyId::FIELD_USE,
        PropertyValue::EnumSymbol(SemanticEnumValue::GROUP_ONLY),
    );
    assert_value(
        node(&catalog, SemanticObjectKind::Attribute, "FolderAndItem"),
        SemanticPropertyId::FIELD_USE,
        PropertyValue::EnumSymbol(SemanticEnumValue::GROUPS_AND_ITEMS),
    );

    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/versions/v2_20/coverage.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let actual = manifest["enumAliases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["semantic"].as_str().unwrap().to_string(),
                entry["nativeAliases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<BTreeSet<_>>(),
                entry["propertyIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected = expected_inventory()["enumAliasInventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["semantic"].as_str().unwrap().to_string(),
                entry["nativeAliases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<BTreeSet<_>>(),
                entry["propertyIds"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_string())
                    .collect::<BTreeSet<_>>(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "native enum aliases and applicability must be an exact frozen legacy bijection");
}

#[test]
fn real_rights_backing_projects_permissions_targets_conditions_and_templates() {
    let root = temp_root("rights");
    let source_root = root.join("src");
    fs::create_dir_all(source_root.join("SalesReader/Ext")).unwrap();
    fs::copy(tracked_root().join("rights/SalesReader.xml"), source_root.join("SalesReader.xml")).unwrap();
    fs::copy(repo_root().join("tests/fixtures/unica_mcp_script_parity/role-info/SalesReader/Ext/Rights.xml"), source_root.join("SalesReader/Ext/Rights.xml")).unwrap();
    let envelope = read_path(&source_root, &source_root.join("SalesReader.xml"));
    fs::remove_dir_all(root).unwrap();

    assert_eq!(envelope.status, NavigationStatus::Available);
    let role = node(&envelope, SemanticObjectKind::Role, "SalesReader");
    assert_value(role, SemanticPropertyId::BACKING_CONTENT_AVAILABLE, PropertyValue::Boolean(true));
    assert_value(role, SemanticPropertyId::ACCESS_NEW_OBJECTS_DEFAULT, PropertyValue::Boolean(false));
    assert_value(role, SemanticPropertyId::ACCESS_ATTRIBUTES_DEFAULT, PropertyValue::Boolean(true));
    assert_value(role, SemanticPropertyId::ACCESS_CHILD_OBJECTS_INDEPENDENT, PropertyValue::Boolean(false));

    let view = envelope.nodes.iter().find(|node| node.object_ref.kind == SemanticObjectKind::AccessPermission
        && node.properties.get(&SemanticPropertyId::ACCESS_PERMISSION_NAME).and_then(|property| property.value()) == Some(&PropertyValue::String("View".to_string()))).expect("View permission");
    assert_value(view, SemanticPropertyId::ACCESS_PERMISSION_ALLOWED, PropertyValue::Boolean(true));
    assert_value(view, SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS, PropertyValue::List(vec![PropertyValue::String("Products.Owner = &CurrentUser".to_string())]));
    assert!(envelope.relation_index.iter().any(|relation| relation.role == SemanticRelationId::ACCESS_TARGET
        && relation.source == view.object_ref && relation.target.kind == SemanticObjectKind::Catalog
        && relation.target.display_name == "Products"));
    let template = envelope.nodes.iter().find(|node| node.object_ref.kind == SemanticObjectKind::AccessRestrictionTemplate).expect("restriction template");
    assert_value(template, SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS, PropertyValue::List(vec![PropertyValue::String("Owner = &CurrentUser".to_string())]));
    let facts = legacy_baseline_fact_set(&envelope);
    assert_eq!(
        facts,
        expected_fact_set(&expected_inventory()["rightsFacts"]),
        "rights parity must compare the complete normalized information set"
    );
}

#[test]
fn rights_extensions_fail_closed_without_becoming_restrictions() {
    let rights = r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:type="Rights" version="2.17" setForNewObjects="false"
        setForAttributesByDefault="true" independentRightsOfChildObjects="false"
        futureRoot="root-readable-value">
  <object futureObject="object-readable-value">
    <name>Catalog.Products</name>
    <right futureRight="right-readable-value">
      <name>View</name>
      <value>true</value>
      <restrictionByCondition futureRestriction="restriction-readable-value">
        <condition>Known = true</condition>
        <condition futureConditionAttribute="condition-attribute-readable-value">nested-condition<futureNested>nested-readable-value</futureNested></condition>
        <futureCondition>not-a-condition</futureCondition>
      </restrictionByCondition>
      <futureRightChild>right-child-readable-value</futureRightChild>
    </right>
  </object>
  <restrictionTemplate futureTemplate="template-readable-value">
    <name>KnownTemplate</name>
    <condition>Template = true</condition>
    <futureTemplateChild>template-child-readable-value</futureTemplateChild>
  </restrictionTemplate>
  <futureRootChild>root-child-readable-value</futureRootChild>
</Rights>"#;
    let envelope = read_rights_inline("future-rights", rights);
    assert_eq!(envelope.status, NavigationStatus::Partial);

    let permission = envelope
        .nodes
        .iter()
        .find(|node| {
            node.object_ref.kind == SemanticObjectKind::AccessPermission
                && node
                    .properties
                    .get(&SemanticPropertyId::ACCESS_PERMISSION_NAME)
                    .and_then(|property| property.value())
                    == Some(&PropertyValue::String("View".to_string()))
        })
        .expect("known permission survives future rights syntax");
    assert_value(
        permission,
        SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS,
        PropertyValue::List(vec![PropertyValue::String("Known = true".to_string())]),
    );
    let unknown = serde_json::to_string(
        permission.properties[&SemanticPropertyId::UNKNOWN_FACTS]
            .value()
            .expect("unknown right facts remain readable"),
    )
    .unwrap();
    for value in [
        "right-readable-value",
        "restriction-readable-value",
        "condition-attribute-readable-value",
        "nested-readable-value",
        "not-a-condition",
        "right-child-readable-value",
    ] {
        assert!(unknown.contains(value), "missing readable neutral rights evidence {value}");
    }
    assert!(!unknown.contains("futureRight"));
    assert!(!unknown.contains("futureCondition"));

    let all_output = serde_json::to_string(&envelope).unwrap();
    for value in [
        "root-readable-value",
        "object-readable-value",
        "template-readable-value",
        "template-child-readable-value",
        "root-child-readable-value",
    ] {
        assert!(all_output.contains(value), "missing readable rights extension evidence {value}");
    }
    assert!(!all_output.contains("not-a-condition\",\"type\":\"string\"}]"),
        "unknown right children must not be reclassified as known restriction conditions");
}

#[test]
fn complete_type_set_inventory_preserves_builtins_alias_categories_and_qualifier_combinations() {
    let envelope = read_tracked("types/AllTypes.xml");
    assert_eq!(envelope.status, NavigationStatus::Available);
    let type_node = node(&envelope, SemanticObjectKind::DefinedType, "AllTypes");
    let PropertyValue::TypeSet(types) = type_node.properties[&SemanticPropertyId::DEFINED_TYPE].value().unwrap() else { panic!("type set"); };
    assert_eq!(types.variants().len(), 14);
    let json = serde_json::to_string(&types).unwrap();
    for category in ["uuid", "opaque", "null", "reference", "object", "recordSet", "manager", "key", "enumeration", "definedType"] {
        assert!(json.contains(category), "missing semantic type category {category}: {json}");
    }
    for primitive in [PrimitiveTypeKind::String, PrimitiveTypeKind::Number, PrimitiveTypeKind::Date] {
        assert!(types.variants().iter().any(|variant| variant.primitive_kind() == Some(primitive) && variant.qualifiers().is_some()), "missing qualifiers for {primitive:?}");
    }

    let subscription = read_tracked("types/EventSources.xml");
    let source = node(&subscription, SemanticObjectKind::EventSubscription, "EventSources");
    let json = serde_json::to_string(source.properties[&SemanticPropertyId::SUBSCRIPTION_SOURCE_TYPE].value().unwrap()).unwrap();
    assert!(json.matches("\"kind\":\"object\"").count() >= 4, "subscription object aliases were not preserved: {json}");
}

#[test]
fn owned_and_common_forms_and_templates_expose_descriptor_type_and_opaque_backing_truthfully() {
    let owned = read_tracked("artifacts/ArtifactReport.xml");
    assert_eq!(owned.status, NavigationStatus::Partial);
    let form = node(&owned, SemanticObjectKind::Form, "MainForm");
    assert_value(form, SemanticPropertyId::FORM_TYPE, PropertyValue::EnumSymbol(SemanticEnumValue::MANAGED));
    assert_value(form, SemanticPropertyId::BACKING_DESCRIPTOR_AVAILABLE, PropertyValue::Boolean(true));
    assert_value(form, SemanticPropertyId::BACKING_CONTENT_AVAILABLE, PropertyValue::Boolean(true));
    assert_value(form, SemanticPropertyId::BACKING_CONTENT_OPAQUE, PropertyValue::Boolean(true));
    assert_value(form, SemanticPropertyId::BACKING_DESCRIPTOR_UUID, PropertyValue::Uuid("20000000-0000-0000-0000-000000000002".parse().unwrap()));
    let template = node(&owned, SemanticObjectKind::Template, "MainSchema");
    assert_value(template, SemanticPropertyId::TEMPLATE_TYPE, PropertyValue::EnumSymbol(SemanticEnumValue::DATA_COMPOSITION_SCHEMA));
    assert_value(template, SemanticPropertyId::BACKING_CONTENT_OPAQUE, PropertyValue::Boolean(true));

    let common_form = read_tracked("common_form/CommonDashboard.xml");
    assert_eq!(common_form.status, NavigationStatus::Partial);
    let form = node(&common_form, SemanticObjectKind::CommonForm, "CommonDashboard");
    assert_value(form, SemanticPropertyId::FORM_TYPE, PropertyValue::EnumSymbol(SemanticEnumValue::MANAGED));
    assert_value(form, SemanticPropertyId::BACKING_CONTENT_AVAILABLE, PropertyValue::Boolean(true));

    let common_template = read_tracked("common_template/CommonLayout.xml");
    assert_eq!(common_template.status, NavigationStatus::Partial);
    let template = node(&common_template, SemanticObjectKind::CommonTemplate, "CommonLayout");
    assert_value(template, SemanticPropertyId::TEMPLATE_TYPE, PropertyValue::EnumSymbol(SemanticEnumValue::BINARY_DATA));
    assert_value(template, SemanticPropertyId::BACKING_CONTENT_AVAILABLE, PropertyValue::Boolean(true));

    let expected = expected_inventory();
    for (label, envelope, inventory) in [
        ("owned artifacts", &owned, "ownedArtifactFacts"),
        ("common form", &common_form, "commonFormFacts"),
        ("common template", &common_template, "commonTemplateFacts"),
    ] {
        let facts = legacy_baseline_fact_set(envelope);
        assert_eq!(
            facts,
            expected_fact_set(&expected[inventory]),
            "{label} parity must compare the complete normalized information set"
        );
    }
}

fn expected_inventory() -> serde_json::Value {
    serde_json::from_slice(&fs::read(tracked_root().join("expected-semantic-facts.json")).unwrap()).unwrap()
}

fn assert_manifest_mutation_rejected(
    raw: &str,
    label: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) {
    let mut manifest: serde_json::Value = serde_json::from_str(raw).unwrap();
    mutate(&mut manifest);
    let candidate = serde_json::to_string(&manifest).unwrap();
    assert!(
        PlatformXmlAdapterFactory::validate_2_20_coverage_manifest(&candidate).is_err(),
        "{label} must not be accepted by runtime coverage validation"
    );
}

fn semantic_fact_set(envelope: &NavigationEnvelope) -> BTreeSet<String> {
    let mut facts = BTreeSet::new();
    for node in &envelope.nodes {
        let kind = node.object_ref.kind.as_str();
        let name = &node.object_ref.display_name;
        facts.insert(format!("node:{kind}:{name}"));
        for (id, property) in &node.properties {
            let value = match property.value() {
                Some(PropertyValue::Boolean(value)) => value.to_string(),
                Some(PropertyValue::Integer(value)) => value.to_string(),
                Some(PropertyValue::EnumSymbol(value)) => value.as_str().to_string(),
                Some(PropertyValue::String(value)) => serde_json::to_string(value).unwrap(),
                Some(value) => serde_json::to_string(value).unwrap(),
                None if property.value_state() == PropertyValueState::Absent => "absent".to_string(),
                None => "unresolved".to_string(),
            };
            facts.insert(format!("property:{kind}:{name}:{id}={value}"));
        }
    }
    for relation in envelope.relation_index.iter() {
        facts.insert(format!("relation:{}:{}:{}:{}:{}", relation.role, relation.source.kind, relation.source.display_name, relation.target.kind, relation.target.display_name));
    }
    facts
}

fn legacy_baseline_fact_set(envelope: &NavigationEnvelope) -> BTreeSet<String> {
    let mut facts = envelope_contract_fact_set(envelope);
    for node in &envelope.nodes {
        if node.object_ref.kind == SemanticObjectKind::SourceRoot {
            continue;
        }
        let kind = node.object_ref.kind.as_str();
        let name = &node.object_ref.display_name;
        facts.insert(format!("node:{kind}:{name}"));
        for (id, property) in &node.properties {
            if matches!(
                *id,
                SemanticPropertyId::METADATA_KIND
                    | SemanticPropertyId::METADATA_NAME
                    | SemanticPropertyId::METADATA_UUID
                    | SemanticPropertyId::SUPPORT_STATE
                    | SemanticPropertyId::SUPPORT_AUTHORABILITY
                    | SemanticPropertyId::SUPPORT_EDIT_CAPABILITY
                    | SemanticPropertyId::UNKNOWN_FACTS
            ) {
                continue;
            }
            let value = match property.value() {
                Some(value) => serde_json::to_string(value).unwrap(),
                None if *id == SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT
                    && property.value_state() == PropertyValueState::Absent =>
                {
                    "absent".to_string()
                }
                None if property.value_state() == PropertyValueState::Unresolved => {
                    "unresolved".to_string()
                }
                None => continue,
            };
            facts.insert(format!("property:{kind}:{name}:{id}={value}"));
        }
    }
    for relation in envelope.relation_index.iter() {
        if relation.source.kind == SemanticObjectKind::SourceRoot {
            continue;
        }
        facts.insert(format!(
            "relation:{}:{:?}:{}:{}:{}:{}",
            relation.role,
            relation.kind,
            relation.source.kind,
            relation.source.display_name,
            relation.target.kind,
            relation.target.display_name
        ));
    }
    facts
}

fn envelope_contract_fact_set(envelope: &NavigationEnvelope) -> BTreeSet<String> {
    let mut facts = BTreeSet::from([format!("status:{:?}", envelope.status)]);
    let mut coverage = std::collections::BTreeMap::<String, usize>::new();
    for node in &envelope.nodes {
        *coverage
            .entry(format!("{:?}", node.capability.coverage))
            .or_default() += 1;
    }
    for (state, count) in coverage {
        facts.insert(format!("coverage:{state}={count}"));
    }
    let mut diagnostics = std::collections::BTreeMap::<String, usize>::new();
    for diagnostic in &envelope.diagnostics {
        let value = serde_json::to_value(diagnostic).unwrap();
        let code = value["code"]
            .as_str()
            .expect("diagnostic code must be serialized")
            .to_string();
        *diagnostics.entry(code).or_default() += 1;
    }
    for (code, count) in diagnostics {
        facts.insert(format!("diagnostic:{code}={count}"));
    }
    facts
}

fn expected_fact_set(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("every exact parity case needs a frozen fact array")
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

fn read_tracked(relative: &str) -> NavigationEnvelope {
    let target = tracked_root().join(relative);
    let source_root = target.parent().expect("tracked fixture parent").to_path_buf();
    read_path(&source_root, &target)
}

fn read_path(source_root: &Path, target: &Path) -> NavigationEnvelope {
    let source = SourceContext::new(SourceLocation::new(repo_root(), source_root.to_path_buf(), target.to_path_buf()), Some("main".to_string()), SourceFamily::PlatformXml, None);
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else { panic!("fixture must be captured"); };
    registration.read.read(&FormatReadRequest { captured: captured.clone(), query: NavigationQuery { target: NavigationTarget::CapturedTarget(captured.binding().target_identity.clone()), select: NavigationSelection { properties: PropertySelection::All, facets: FacetSelection::Full, relations: Vec::new() } } }).expect("structurally readable Platform XML must project")
}

fn read_inline(label: &str, file_name: &str, xml: &str) -> NavigationEnvelope {
    let root = temp_root(label);
    fs::create_dir_all(&root).unwrap();
    let target = root.join(file_name);
    fs::write(&target, xml).unwrap();
    let envelope = read_path(&root, &target);
    fs::remove_dir_all(root).unwrap();
    envelope
}

fn read_rights_inline(label: &str, rights: &str) -> NavigationEnvelope {
    let root = temp_root(label);
    fs::create_dir_all(root.join("SalesReader/Ext")).unwrap();
    fs::copy(
        tracked_root().join("rights/SalesReader.xml"),
        root.join("SalesReader.xml"),
    )
    .unwrap();
    fs::write(root.join("SalesReader/Ext/Rights.xml"), rights).unwrap();
    let envelope = read_path(&root, &root.join("SalesReader.xml"));
    fs::remove_dir_all(root).unwrap();
    envelope
}

fn node<'a>(envelope: &'a NavigationEnvelope, kind: SemanticObjectKind, name: &str) -> &'a NavigationNode {
    envelope.nodes.iter().find(|node| node.object_ref.kind == kind && node.object_ref.display_name == name).unwrap_or_else(|| panic!("missing {kind} node {name}"))
}

fn assert_value(node: &NavigationNode, id: SemanticPropertyId, expected: PropertyValue) {
    assert_eq!(node.properties.get(&id).unwrap_or_else(|| panic!("missing property {id}")).value(), Some(&expected), "unexpected value for {id}");
}

fn assert_absent(node: &NavigationNode, id: SemanticPropertyId) {
    let property = node.properties.get(&id).unwrap_or_else(|| panic!("missing property {id}"));
    assert_eq!(property.value_state(), PropertyValueState::Absent, "{id} must be explicitly inactive");
}

fn tracked_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_20") }
fn repo_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
fn temp_root(label: &str) -> PathBuf { std::env::temp_dir().join(format!("unica-platform-xml-task5-fix1-{label}-{}-{}", std::process::id(), NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed))) }
