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
    for pair in expected["allKinds"].as_array().unwrap() {
        let kind = SemanticObjectKind::parse(pair[0].as_str().unwrap()).unwrap();
        let name = pair[1].as_str().unwrap();
        node(&envelope, kind, name);
    }

    let manifest_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/coverage.json");
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["schemaVersion"], 2);
    for section in ["objects", "properties", "children", "enumAliases", "typeVariants", "backingArtifacts", "intentionalPartialCases"] {
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
fn real_tracked_platform_xml_files_match_frozen_identity_inventory() {
    let expected = expected_inventory();
    let source_root = repo_root().join("tests/fixtures/unica_mcp_script_parity/bsp/meta");
    for case in expected["realFixtureNodes"].as_array().unwrap() {
        let target = source_root.join(case[0].as_str().unwrap());
        let envelope = read_path(&source_root, &target);
        let kind = SemanticObjectKind::parse(case[1].as_str().unwrap()).unwrap();
        node(&envelope, kind, case[2].as_str().unwrap());
        assert_ne!(envelope.status, NavigationStatus::Unavailable);
        let facts = semantic_fact_set(&envelope);
        for expected_fact in expected["realFixtureFacts"][case[0].as_str().unwrap()]
            .as_array()
            .expect("every real fixture needs an independent fact inventory")
        {
            assert!(
                facts.contains(expected_fact.as_str().unwrap()),
                "{} is missing frozen legacy fact {}",
                case[0].as_str().unwrap(),
                expected_fact
            );
        }
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
}

fn expected_inventory() -> serde_json::Value {
    serde_json::from_slice(&fs::read(tracked_root().join("expected-semantic-facts.json")).unwrap()).unwrap()
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
