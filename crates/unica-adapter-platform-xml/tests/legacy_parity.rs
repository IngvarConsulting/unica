use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationEnvelope, NavigationNode, NavigationQuery,
        NavigationSelection, NavigationStatus, NavigationTarget, ObjectRef, PropertySelection,
        PropertyValueState,
    },
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
    assert_exact_oracle(&envelope, "allKinds");

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
        assert_exact_oracle(&envelope, case[0].as_str().unwrap());
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
    assert_value(
        node(&envelope, SemanticObjectKind::Catalog, "Валюты"),
        SemanticPropertyId::CATALOG_CODE_SERIES,
        PropertyValue::EnumSymbol(SemanticEnumValue::WHOLE_COLLECTION),
    );
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
    for (label, native, semantic) in [
        (
            "whole-catalog",
            "WholeCatalog",
            SemanticEnumValue::WHOLE_COLLECTION,
        ),
        (
            "within-owner",
            "WithinOwnerSubordination",
            SemanticEnumValue::WITHIN_OWNER_SCOPE,
        ),
        (
            "within-parent",
            "WithinSubordination",
            SemanticEnumValue::WITHIN_PARENT_SCOPE,
        ),
    ] {
        let catalog = read_inline(
            &format!("catalog-code-series-{label}"),
            "CatalogSeries.xml",
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="71000000-0000-0000-0000-000000000001"><Properties><Name>CatalogSeries</Name><CodeSeries>{native}</CodeSeries></Properties></Catalog></MetaDataObject>"#
            ),
        );
        assert_value(
            node(&catalog, SemanticObjectKind::Catalog, "CatalogSeries"),
            SemanticPropertyId::CATALOG_CODE_SERIES,
            PropertyValue::EnumSymbol(semantic),
        );
    }

    let document_modes = read_inline(
        "complete-document-enums",
        "DocumentModes.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="71000000-0000-0000-0000-000000000002"><Properties><Name>DocumentModes</Name><NumberPeriodicity>Nonperiodical</NumberPeriodicity><RegisterRecordsDeletion>AutoDelete</RegisterRecordsDeletion><RegisterRecordsWritingOnPost>WriteModified</RegisterRecordsWritingOnPost></Properties></Document></MetaDataObject>"#,
    );
    let document = node(
        &document_modes,
        SemanticObjectKind::Document,
        "DocumentModes",
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY,
        PropertyValue::EnumSymbol(SemanticEnumValue::NONPERIODICAL),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_DELETION_MODE,
        PropertyValue::EnumSymbol(SemanticEnumValue::DELETE_AUTOMATIC),
    );
    assert_value(
        document,
        SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE,
        PropertyValue::EnumSymbol(SemanticEnumValue::WRITE_MODIFIED),
    );

    for (deletion, deletion_semantic, writing, writing_semantic) in [
        (
            "AutoDeleteOnUnpost",
            SemanticEnumValue::DELETE_ON_REVERSAL,
            "WriteSelected",
            SemanticEnumValue::WRITE_SELECTED,
        ),
        (
            "AutoDeleteOff",
            SemanticEnumValue::DELETE_DISABLED,
            "WriteAll",
            SemanticEnumValue::WRITE_ALL,
        ),
    ] {
        let document = read_inline(
            &format!("document-modes-{deletion}-{writing}"),
            "OtherDocumentModes.xml",
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="71000000-0000-0000-0000-000000000005"><Properties><Name>OtherDocumentModes</Name><RegisterRecordsDeletion>{deletion}</RegisterRecordsDeletion><RegisterRecordsWritingOnPost>{writing}</RegisterRecordsWritingOnPost></Properties></Document></MetaDataObject>"#
            ),
        );
        let document = node(
            &document,
            SemanticObjectKind::Document,
            "OtherDocumentModes",
        );
        assert_value(
            document,
            SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_DELETION_MODE,
            PropertyValue::EnumSymbol(deletion_semantic),
        );
        assert_value(
            document,
            SemanticPropertyId::DOCUMENT_REGISTER_RECORDS_WRITING_ON_POST_MODE,
            PropertyValue::EnumSymbol(writing_semantic),
        );
    }

    let service = read_inline(
        "auto-use-sessions",
        "SessionService.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><HTTPService uuid="71000000-0000-0000-0000-000000000006"><Properties><Name>SessionService</Name><ReuseSessions>AutoUse</ReuseSessions></Properties></HTTPService></MetaDataObject>"#,
    );
    assert_value(
        node(
            &service,
            SemanticObjectKind::HttpService,
            "SessionService",
        ),
        SemanticPropertyId::HTTP_SERVICE_REUSE_SESSIONS,
        PropertyValue::EnumSymbol(SemanticEnumValue::USE),
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
fn enum_aliases_are_rejected_outside_their_declared_property_context() {
    for (label, class, property, value, kind, property_id) in [
        (
            "catalog-series-on-document",
            "Document",
            "NumberPeriodicity",
            "WholeCatalog",
            SemanticObjectKind::Document,
            SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY,
        ),
        (
            "document-period-on-catalog",
            "Catalog",
            "CodeSeries",
            "Year",
            SemanticObjectKind::Catalog,
            SemanticPropertyId::CATALOG_CODE_SERIES,
        ),
        (
            "module-reuse-on-service",
            "HTTPService",
            "ReuseSessions",
            "DuringRequest",
            SemanticObjectKind::HttpService,
            SemanticPropertyId::HTTP_SERVICE_REUSE_SESSIONS,
        ),
        (
            "session-reuse-on-module",
            "CommonModule",
            "ReturnValuesReuse",
            "AutoUse",
            SemanticObjectKind::CommonModule,
            SemanticPropertyId::MODULE_RETURN_VALUES_REUSE,
        ),
    ] {
        let envelope = read_inline(
            label,
            "CrossContext.xml",
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{class} uuid="71000000-0000-0000-0000-000000000007"><Properties><Name>CrossContext</Name><{property}>{value}</{property}></Properties></{class}></MetaDataObject>"#
            ),
        );
        assert_eq!(envelope.status, NavigationStatus::Partial, "{label}");
        let property = &node(&envelope, kind, "CrossContext").properties[&property_id];
        assert_eq!(
            property.value_state(),
            PropertyValueState::Unresolved,
            "{label}"
        );
        assert_eq!(property.value(), None, "{label}");
    }
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
    assert_exact_oracle(&envelope, "rights");
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
    assert_value(
        permission,
        SemanticPropertyId::UNKNOWN_FACTS,
        unknown_property_evidence(&[
            "right-readable-value",
            "right-child-readable-value",
            "restriction-readable-value",
            "not-a-condition",
            "condition-attribute-readable-value",
            "nested-condition",
            "nested-readable-value",
        ]),
    );
    let unknown = serde_json::to_string(
        permission.properties[&SemanticPropertyId::UNKNOWN_FACTS]
            .value()
            .expect("unknown right facts remain readable"),
    )
    .unwrap();
    assert!(!unknown.contains("futureRight"));
    assert!(!unknown.contains("futureCondition"));

    let role = node(&envelope, SemanticObjectKind::Role, "SalesReader");
    assert_value(
        role,
        SemanticPropertyId::UNKNOWN_FACTS,
        unknown_property_evidence(&[
            "root-readable-value",
            "root-child-readable-value",
            "object-readable-value",
        ]),
    );
    let template = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.kind == SemanticObjectKind::AccessRestrictionTemplate)
        .expect("known template survives future rights syntax");
    assert_value(
        template,
        SemanticPropertyId::UNKNOWN_FACTS,
        unknown_property_evidence(&[
            "template-readable-value",
            "template-child-readable-value",
        ]),
    );
    let all_output = serde_json::to_string(&envelope).unwrap();
    assert!(!all_output.contains("not-a-condition\",\"type\":\"string\"}]"),
        "unknown right children must not be reclassified as known restriction conditions");
}

#[test]
fn ambiguous_duplicate_rights_fields_are_retained_and_partial() {
    let rights = r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
        xsi:type="Rights" version="2.17" setForNewObjects="false"
        setForAttributesByDefault="true" independentRightsOfChildObjects="false">
  <object>
    <name>Catalog.Products</name>
    <right>
      <name>View</name>
      <name>Update</name>
      <value>true</value>
      <value>false</value>
    </right>
  </object>
</Rights>"#;
    let envelope = read_rights_inline("duplicate-rights-fields", rights);
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let permission = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.kind == SemanticObjectKind::AccessPermission)
        .expect("ambiguous permission remains readable");
    assert_value(
        permission,
        SemanticPropertyId::UNKNOWN_FACTS,
        unknown_property_evidence(&[
            "View",
            "Update",
            "extension-occurrence-3",
            "true",
            "false",
            "extension-occurrence-6",
        ]),
    );
    for invalid_name in ["View", "Update"] {
        assert_ne!(
            permission
                .properties
                .get(&SemanticPropertyId::ACCESS_PERMISSION_NAME)
                .and_then(|property| property.value()),
            Some(&PropertyValue::String(invalid_name.to_string())),
            "an ambiguous field must not be projected as typed"
        );
    }
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

    for (envelope, inventory) in [
        (&owned, "ownedArtifacts"),
        (&common_form, "commonForm"),
        (&common_template, "commonTemplate"),
    ] {
        assert_exact_oracle(envelope, inventory);
    }
}

#[test]
fn exact_oracle_covers_types_unknowns_and_legacy_provenance() {
    for (fixture, case) in [
        ("types/AllTypes.xml", "allTypes"),
        ("types/EventSources.xml", "eventSources"),
        ("unknowns/UnknownCases.xml", "unknowns"),
    ] {
        assert_exact_oracle(&read_tracked(fixture), case);
    }

    let oracle = exact_oracle();
    for case in oracle["cases"].as_object().unwrap().values() {
        let Some(output) = case.get("legacyOutput").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let frozen = fs::read(tracked_root().join(output))
            .unwrap_or_else(|_| panic!("missing frozen legacy output {output}"));
        assert!(
            frozen.iter().any(|byte| !byte.is_ascii_whitespace()),
            "frozen legacy output is empty: {output}"
        );
    }
}

#[test]
fn exact_parity_comparator_rejects_wrong_enum_omission_and_duplicate_node_mutations() {
    let source_root = repo_root().join("tests/fixtures/unica_mcp_script_parity/bsp/meta");
    let currencies = read_path(&source_root, &source_root.join("Catalogs/Валюты.xml"));
    let currency_facts = normalized_actual_fact_multiset(&currencies);
    let mut wrong_property = currency_facts.clone();
    let fact = wrong_property
        .iter_mut()
        .find(|fact| fact.contains(":catalog.code.series="))
        .expect("catalog code-series fact");
    *fact = fact.replace(
        ":catalog.code.series=",
        ":document.number.periodicity=",
    );
    assert!(!same_fact_multiset(wrong_property, currency_facts));

    let unknowns = normalized_actual_fact_multiset(&read_tracked("unknowns/UnknownCases.xml"));
    let mut omitted_unknown = unknowns.clone();
    let index = omitted_unknown
        .iter()
        .position(|fact| fact.contains(":unknown.facts="))
        .expect("unknown fact in adversarial fixture");
    omitted_unknown.remove(index);
    assert!(!same_fact_multiset(omitted_unknown, unknowns.clone()));

    let mut duplicate_node = unknowns.clone();
    let node = duplicate_node
        .iter()
        .find(|fact| fact.starts_with("node:"))
        .expect("node fact")
        .clone();
    duplicate_node.push(node);
    assert!(!same_fact_multiset(duplicate_node, unknowns));
}

fn expected_inventory() -> serde_json::Value {
    serde_json::from_slice(&fs::read(tracked_root().join("expected-semantic-facts.json")).unwrap()).unwrap()
}

fn exact_oracle() -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(tracked_root().join("legacy-oracle/exact-semantic-facts.json")).unwrap(),
    )
    .unwrap()
}

fn assert_exact_oracle(envelope: &NavigationEnvelope, case: &str) {
    let actual = normalized_actual_fact_multiset(envelope);
    let expected = exact_oracle()["cases"][case]["facts"]
        .as_array()
        .unwrap_or_else(|| panic!("missing frozen exact oracle case {case}"))
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "{case} drifted from its independent frozen semantic oracle"
    );
}

fn same_fact_multiset(mut left: Vec<String>, mut right: Vec<String>) -> bool {
    left.sort();
    right.sort();
    left == right
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

fn normalized_actual_fact_multiset(envelope: &NavigationEnvelope) -> Vec<String> {
    let identities = envelope
        .nodes
        .iter()
        .enumerate()
        .map(|(ordinal, node)| {
            let identity = match node
                .properties
                .get(&SemanticPropertyId::METADATA_UUID)
                .and_then(|property| property.value())
            {
                Some(PropertyValue::Uuid(uuid)) => format!("uuid:{uuid}"),
                _ => format!(
                    "node:{}:{}:{ordinal}",
                    node.object_ref.kind,
                    serde_json::to_string(&node.object_ref.display_name).unwrap()
                ),
            };
            (node.object_ref.clone(), identity)
        })
        .collect::<Vec<_>>();
    let identity = |reference: &ObjectRef| {
        identities
            .iter()
            .find(|(candidate, _)| candidate == reference)
            .map(|(_, identity)| identity.clone())
            .unwrap_or_else(|| {
                format!(
                    "external:{}:{}",
                    reference.kind,
                    serde_json::to_string(&reference.display_name).unwrap()
                )
            })
    };

    let mut facts = vec![
        format!("schema:{}", envelope.schema_version),
        format!("status:{:?}", envelope.status),
        format!(
            "root:{}",
            envelope
                .root
                .as_ref()
                .map(&identity)
                .unwrap_or_else(|| "none".to_string())
        ),
    ];
    for node in &envelope.nodes {
        let node_identity = identity(&node.object_ref);
        facts.push(format!(
            "node:{node_identity}={}",
            serde_json::to_string(&serde_json::json!({
                "kind": node.object_ref.kind,
                "name": node.object_ref.display_name,
                "referenceKind": node.reference.kind,
                "referenceName": node.reference.display_name,
                "capabilityState": node.capability_state,
                "capability": node.capability,
                "facets": node.facets,
                "facetVisibility": format!("{:?}", node.facet_visibility),
            }))
            .unwrap()
        ));
        for (id, property) in &node.properties {
            facts.push(format!(
                "property:{node_identity}:{id}={}",
                serde_json::to_string(property).unwrap()
            ));
        }
    }
    for relation in envelope.relation_index.iter() {
        facts.push(format!(
            "relation={}",
            serde_json::to_string(&serde_json::json!({
                "kind": relation.kind,
                "role": relation.role,
                "source": identity(&relation.source),
                "target": identity(&relation.target),
            }))
            .unwrap()
        ));
    }
    for diagnostic in &envelope.diagnostics {
        facts.push(format!(
            "diagnostic={}",
            serde_json::to_string(diagnostic).unwrap()
        ));
    }
    facts.sort();
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

fn unknown_property_evidence(values: &[&str]) -> PropertyValue {
    PropertyValue::List(vec![PropertyValue::Structure(BTreeMap::from([
        (
            "category".to_string(),
            PropertyValue::String("property".to_string()),
        ),
        (
            "value".to_string(),
            PropertyValue::List(
                values
                    .iter()
                    .map(|value| PropertyValue::String((*value).to_string()))
                    .collect(),
            ),
        ),
    ]))])
}

fn assert_absent(node: &NavigationNode, id: SemanticPropertyId) {
    let property = node.properties.get(&id).unwrap_or_else(|| panic!("missing property {id}"));
    assert_eq!(property.value_state(), PropertyValueState::Absent, "{id} must be explicitly inactive");
}

fn tracked_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_20") }
fn repo_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
fn temp_root(label: &str) -> PathBuf { std::env::temp_dir().join(format!("unica-platform-xml-task5-fix1-{label}-{}-{}", std::process::id(), NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed))) }
