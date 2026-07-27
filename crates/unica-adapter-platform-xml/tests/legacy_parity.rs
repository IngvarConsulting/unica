use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        ActionAvailability, ActionExecutionPolicy, ActionProfile, Atomicity, Authorability,
        CapabilityBlockReason, CapabilityState, CapabilityVector, CoverageState, FacetSelection,
        FormatCompatibility, IdentityStrength, NavigationCursor, NavigationEnvelope,
        NavigationFacetVisibility, NavigationNode, NavigationQuery, NavigationRelationPage,
        NavigationSelection, NavigationStatus, NavigationTarget, ObjectKey, ObjectRef,
        OperationBinding, PropertyCapability, PropertySelection, PropertyValueState,
        RelationGroupRef, RelationKey, RelationKind, RelationRef, RelationSelection,
        ResolutionState, SemanticAction, SemanticActionDescriptor, SemanticActionKind,
        SemanticFacets, SemanticProperty, SemanticRelation,
    },
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{
        SemanticEnumValue, SemanticObjectKind, SemanticPropertyId, SemanticRelationId,
    },
    source::{
        SnapshotConsistency, SourceAccess, SourceAdapterErrorKind, SourceContext, SourceFamily,
        SourceId, SourceLocation, SourceRevision, SourceSnapshot,
    },
    value::{PrimitiveTypeKind, PropertyType, PropertyValue},
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyOracle {
    schema_version: u32,
    provenance: String,
    enum_coverage: Vec<Value>,
    cases: Vec<LegacyCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyCase {
    id: String,
    profile: String,
    #[serde(default)]
    parent_case: Option<String>,
    input: String,
    adapter_input: String,
    source_root: String,
    raw_output: String,
    root_kind: String,
    root_name: String,
    facts: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq)]
struct FactDiff {
    missing: Vec<(String, usize)>,
    unexpected: Vec<(String, usize)>,
    first_difference: Option<String>,
}

impl fmt::Display for FactDiff {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "firstDifference={:?}\nmissingFacts={}\nunexpectedFacts={}",
            self.first_difference,
            self.missing.iter().map(|(_, count)| count).sum::<usize>(),
            self.unexpected.iter().map(|(_, count)| count).sum::<usize>(),
        )
    }
}

#[test]
fn legacy_oracle_regenerates_and_hashes_every_declared_source_without_adapter_dependencies() {
    let root = oracle_root();
    let generator = root.join("tools/generate_oracle.py");
    let output = Command::new("python3.12")
        .arg(&generator)
        .arg("--repo-root")
        .arg(repo_root())
        .arg("--check")
        .output()
        .expect("run the legacy-only oracle generator");
    assert!(
        output.status.success(),
        "legacy-only oracle regeneration failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("oracle-manifest.json")).expect("oracle provenance manifest"),
    )
    .unwrap();
    assert_eq!(manifest["schemaVersion"], 1);
    assert_eq!(manifest["hashAlgorithm"], "SHA-256");
    let entries = manifest["entries"].as_array().expect("provenance entries");
    let roles = entries
        .iter()
        .map(|entry| entry["role"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    for required in [
        "oracleGenerator",
        "enumSourceExtractor",
        "enumAliasExecutions",
        "fullPublicContractSpecimen",
        "newOnlyContractBuilder",
        "newOnlyContractSource",
        "enumSourceContexts",
        "oracleInputs",
        "independentCrosswalk",
        "rightsTargetCrosswalk",
        "legacyReferenceSource",
        "legacyInputFixture",
        "newOnlyContract",
        "newOnlyContractInput",
        "rawLegacyOutput",
        "legacySemanticOracle",
    ] {
        assert!(roles.contains(required), "missing provenance role {required}");
    }
    for entry in entries {
        let path = repo_root().join(entry["path"].as_str().unwrap());
        let digest = format!("{:x}", Sha256::digest(fs::read(&path).unwrap()));
        assert_eq!(digest, entry["sha256"], "hash drift for {}", path.display());
    }

    for source_path in [
        generator.clone(),
        root.join("tools/extract_enum_contexts.py"),
        root.join("tools/build_new_only_contract.py"),
    ] {
        let source = fs::read_to_string(&source_path).unwrap();
        for forbidden in [
            "unica_adapter_platform_xml",
            "unica_format_core",
            "normalized_actual",
            "target/debug",
            "cargo run",
        ] {
            assert!(
                !source.contains(forbidden),
                "{} contains forbidden adapter dependency {forbidden}",
                source_path.display()
            );
        }
    }
    let generator_source = fs::read_to_string(&generator).unwrap();
    assert!(generator_source.contains("build_new_only_contract"));
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    assert!(readme.contains("generate_oracle.py --repo-root . --write"));
}

#[test]
fn source_extracted_enum_aliases_are_bijective_with_runtime_property_and_object_context() {
    let expected = oracle().enum_coverage;
    let manifest = coverage_manifest();
    let actual = expanded_registry_enum_coverage(&manifest);
    compare_fact_multisets(&expected, &actual)
        .unwrap_or_else(|diff| panic!("source-derived enum coverage drifted:\n{diff}"));

    let whole_catalog = expected
        .iter()
        .find(|fact| fact["nativeAlias"] == "WholeCatalog")
        .expect("legacy CatalogCodeSeries alias");
    assert_eq!(whole_catalog["nativeProperty"], "CodeSeries");
    assert_eq!(whole_catalog["semanticProperty"], "catalog.code.series");
    assert_eq!(whole_catalog["objectKind"], "catalog");

    assert!(expected.iter().any(|fact| {
        fact["nativeAlias"] == "ShowWarning"
            && fact["semantic"] == "showWarning"
            && fact["semanticProperty"] == "field.fillChecking"
    }));
    assert!(!expected.iter().any(|fact| {
        fact["nativeAlias"]
            .as_str()
            .is_some_and(|alias| alias.chars().next().is_some_and(char::is_lowercase))
    }));
}

#[test]
fn coordinated_crosswalk_and_coverage_context_drift_fails_against_source_and_hashes() {
    let root = oracle_root();
    let source_contexts: Value =
        serde_json::from_slice(&fs::read(root.join("enum-source-contexts.json")).unwrap())
            .unwrap();
    assert_eq!(source_contexts["schemaVersion"], 1);
    let contexts = source_contexts["contexts"].as_array().unwrap();
    let source_facts = contexts
        .iter()
        .map(|context| {
            assert!(!context["nativeAliases"].as_array().unwrap().is_empty());
            assert!(!context["objectKinds"].as_array().unwrap().is_empty());
            context["sourceFact"].as_str().unwrap()
        })
        .collect::<BTreeSet<_>>();

    let mut crosswalk = legacy_crosswalk().clone();
    for domain in crosswalk["enumDomains"].as_object().unwrap().values() {
        assert!(domain.get("nativeProperty").is_none());
        assert!(domain.get("objectKinds").is_none());
        assert!(!domain["sourceFacts"].as_array().unwrap().is_empty());
        for source_fact in domain["sourceFacts"].as_array().unwrap() {
            assert!(source_facts.contains(source_fact.as_str().unwrap()));
        }
    }

    let code_series = &mut crosswalk["enumDomains"]["catalogCodeSeries"];
    code_series["nativeProperty"] = json!("NumberPeriodicity");
    code_series["objectKinds"] = json!(["document"]);
    code_series["sourceFacts"] =
        json!(["metaCompile:emit_document_properties:NumberPeriodicity"]);

    let mut coordinated_coverage = expanded_registry_enum_coverage(&coverage_manifest());
    let whole_catalog = coordinated_coverage
        .iter_mut()
        .find(|fact| fact["nativeAlias"] == "WholeCatalog")
        .unwrap();
    whole_catalog["nativeProperty"] = json!("NumberPeriodicity");
    whole_catalog["objectKind"] = json!("document");
    whole_catalog["semanticProperty"] = json!("document.number.periodicity");
    assert_comparator_rejects(
        &oracle().enum_coverage,
        &coordinated_coverage,
        "coordinated crosswalk and coverage context drift",
    );

    let manifest: Value =
        serde_json::from_slice(&fs::read(root.join("oracle-manifest.json")).unwrap()).unwrap();
    let pinned = manifest["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["role"] == "independentCrosswalk")
        .unwrap()["sha256"]
        .as_str()
        .unwrap();
    let mutated = serde_json::to_vec(&crosswalk).unwrap();
    assert_ne!(format!("{:x}", Sha256::digest(mutated)), pinned);
}

#[test]
fn fix_round6_every_source_enum_context_is_consumed_once_and_fixture_observed() {
    let root = oracle_root();
    let source_contexts: Value =
        serde_json::from_slice(&fs::read(root.join("enum-source-contexts.json")).unwrap())
            .unwrap();
    let contexts = source_contexts["contexts"].as_array().unwrap();
    let crosswalk = legacy_crosswalk();
    let referenced = crosswalk["enumDomains"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|domain| domain["sourceFacts"].as_array().unwrap())
        .map(|fact| fact.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let extracted = contexts
        .iter()
        .map(|context| context["sourceFact"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert_eq!(contexts.len(), 62, "legacy AST context inventory drifted");
    assert_eq!(
        referenced.iter().collect::<BTreeSet<_>>(),
        extracted.iter().collect::<BTreeSet<_>>(),
        "every extracted context must be referenced by exactly one semantic domain"
    );
    assert_eq!(
        referenced.len(),
        referenced.iter().collect::<BTreeSet<_>>().len(),
        "a source context cannot be shared by semantic domains"
    );
    for context in contexts {
        let evidence = context["ownerEvidence"]
            .as_array()
            .expect("owner applicability must come from generated legacy fixtures");
        assert!(!evidence.is_empty());
        assert!(evidence.iter().all(|item| {
            item["input"].as_str().is_some()
                && item["rawOutput"].as_str().is_some()
                && item["nativeOwner"].as_str().is_some()
        }));
    }

    let spreadsheet = contexts
        .iter()
        .find(|context| {
            context["nativeProperty"] == "TemplateType"
                && context["nativeAliases"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("SpreadsheetDocument"))
        })
        .unwrap();
    assert!(spreadsheet["observedAliasEvidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["objectKind"] == "template"
                && item["nativeOwner"] == "Template"
                && item["nativeValue"] == "SpreadsheetDocument"
                && item["input"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("EnumContextSpreadsheetDocument.xml"))
        }));

    let mut removed_crosswalk = crosswalk.clone();
    let (removed_domain, removed_value) = removed_crosswalk["enumDomains"]
        .as_object_mut()
        .unwrap()
        .shift_remove_entry("catalogChoiceMode")
        .expect("source-derived catalog choice domain");
    let removed_source_facts = removed_value["sourceFacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|fact| fact.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let removed_semantic_property = removed_value["semanticProperty"]
        .as_str()
        .expect("removed enum domain semantic property");
    let mut removed_coverage = expanded_registry_enum_coverage(&coverage_manifest());
    removed_coverage.retain(|fact| {
        fact["semanticProperty"].as_str() != Some(removed_semantic_property)
    });
    assert!(
        extracted
            .iter()
            .any(|fact| removed_source_facts.contains(fact.as_str())),
        "source extraction must remain authoritative when both consumers omit {removed_domain}"
    );
    assert_comparator_rejects(
        &oracle().enum_coverage,
        &removed_coverage,
        "coordinated enum-domain omission",
    );
}

#[test]
fn multi_target_role_oracle_keeps_each_source_group_identity_and_restriction() {
    let oracle = oracle();
    let case = oracle_case(&oracle, "rightsMultiTarget");
    let target_crosswalk: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("rights-target-crosswalk.json")).unwrap(),
    )
    .unwrap();
    let targets = case
        .facts
        .iter()
        .filter(|fact| fact["kind"] == "relation" && fact["predicate"] == "accessTarget")
        .map(|fact| {
            (
                fact["value"]["targetKind"].as_str().unwrap(),
                fact["value"]["targetName"].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    let expected = target_crosswalk["prefixes"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(prefix, kind)| {
            (
                kind.as_str().unwrap(),
                format!("{prefix}Target"),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(targets, expected);
    assert!(case.facts.iter().any(|fact| {
        fact["kind"] == "property"
            && fact["predicate"] == "access.restriction.present"
            && fact["subject"]
                .as_str()
                .unwrap()
                .contains("calculationRegister/CalculationRegisterTarget")
    }));
    assert!(case.facts.iter().any(|fact| {
        fact["kind"] == "node"
            && fact["value"]["kind"] == "accessRestrictionTemplate"
            && fact["value"]["name"] == "CalculationRegisterTemplate"
    }));
}

#[test]
fn fix_round6_rights_target_crosswalk_equals_runtime_supported_top_level_registry() {
    let manifest = coverage_manifest();
    let expected = manifest["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["role"] == "topLevelObject" && entry["source"] == "native")
        .map(|entry| {
            (
                entry["nativeClass"].as_str().unwrap().to_string(),
                entry["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let actual: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("rights-target-crosswalk.json")).unwrap(),
    )
    .unwrap();
    let actual = actual["prefixes"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(prefix, kind)| {
            (
                prefix.clone(),
                kind.as_str().expect("semantic target kind").to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        actual, expected,
        "rights target syntax must cover exactly the runtime-accepted native top-level registry"
    );
    let expected_targets = actual
        .iter()
        .map(|(prefix, kind)| (kind.clone(), format!("{prefix}Target")))
        .collect::<BTreeSet<_>>();

    let oracle = oracle();
    let case = oracle_case(&oracle, "rightsMultiTarget");
    let targets = case
        .facts
        .iter()
        .filter(|fact| fact["kind"] == "relation" && fact["predicate"] == "accessTarget")
        .map(|fact| {
            (
                fact["value"]["targetKind"].as_str().unwrap().to_string(),
                fact["value"]["targetName"].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(targets, expected_targets);
    for (kind, name) in &expected_targets {
        let target = format!("{}/external/{kind}/{name}", case.id);
        assert!(case.facts.iter().any(|fact| {
            fact["kind"] == "property"
                && fact["subject"] == target
                && fact["predicate"] == "access.restriction.present"
                && fact["value"]["value"] == true
        }));
    }
    assert_eq!(
        case.facts
            .iter()
            .filter(|fact| {
                fact["kind"] == "node"
                    && fact["value"]["kind"] == "accessRestrictionTemplate"
            })
            .count(),
        expected_targets.len()
    );
    let projected = read_path(
        &repo_root()
            .join("crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights"),
        &repo_root().join(
            "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights/MultiTargetReader.xml",
        ),
    );
    assert_eq!(
        serde_json::to_value(projected.status).unwrap(),
        json!("ready"),
        "fully registered rights fixture must be complete: {:#?}",
        projected.diagnostics
    );
}

#[test]
fn coverage_manifest_is_runtime_checked_and_rejects_every_authority_mutation() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/coverage.json");
    let raw = fs::read_to_string(path).unwrap();
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
        manifest["enumAliases"][0]["nativeAliases"]
            .as_array_mut()
            .unwrap()
            .remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "extra enum alias", |manifest| {
        manifest["enumAliases"][0]["nativeAliases"]
            .as_array_mut()
            .unwrap()
            .push(json!("FutureAlias"));
    });
    assert_manifest_mutation_rejected(&raw, "changed enum owner", |manifest| {
        manifest["enumAliases"][0]["objectKinds"] = json!(["catalog"]);
    });
    assert_manifest_mutation_rejected(&raw, "removed derived enum value", |manifest| {
        manifest["derivedEnumValues"]
            .as_array_mut()
            .unwrap()
            .remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "changed derived enum case", |manifest| {
        manifest["derivedEnumValues"][0]["case"] = json!("future");
    });
    assert_manifest_mutation_rejected(&raw, "removed owner role", |manifest| {
        let entry = manifest["children"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| {
                entry["ownerRoles"]
                    .as_array()
                    .is_some_and(|roles| !roles.is_empty())
            })
            .unwrap();
        entry["ownerRoles"].as_array_mut().unwrap().remove(0);
    });
    assert_manifest_mutation_rejected(&raw, "changed backing kind", |manifest| {
        manifest["backingArtifacts"][0]["kind"] = json!("future");
    });
    assert_manifest_mutation_rejected(&raw, "removed intentional partial rule", |manifest| {
        manifest["intentionalPartialCases"]
            .as_array_mut()
            .unwrap()
            .remove(0);
    });

    for (label, section, entry, field) in [
        ("property applicability", "properties", 0, "objectKinds"),
        ("property aliases", "properties", 0, "nativeNames"),
        (
            "relation applicability",
            "relationProperties",
            0,
            "objectKinds",
        ),
        (
            "relation aliases",
            "relationProperties",
            0,
            "nativeNames",
        ),
        ("enum properties", "enumAliases", 0, "propertyIds"),
        ("enum owners", "enumAliases", 0, "objectKinds"),
        ("enum aliases", "enumAliases", 0, "nativeAliases"),
        (
            "backing applicability",
            "backingArtifacts",
            0,
            "objectKinds",
        ),
        (
            "partial applicability",
            "intentionalPartialCases",
            0,
            "objectKinds",
        ),
    ] {
        assert_manifest_mutation_rejected(&raw, label, |manifest| {
            manifest[section][entry][field] = json!([]);
        });
    }
    for section in [
        "objects",
        "properties",
        "relationProperties",
        "children",
        "enumAliases",
        "derivedEnumValues",
        "typeVariants",
        "backingArtifacts",
        "intentionalPartialCases",
    ] {
        assert_manifest_mutation_rejected(
            &raw,
            &format!("empty mandatory section {section}"),
            |manifest| manifest[section] = json!([]),
        );
    }
}

#[test]
fn every_frozen_legacy_case_matches_the_corresponding_new_projection_exactly() {
    let oracle = oracle();
    assert_eq!(oracle.schema_version, 1);
    assert_eq!(
        oracle.provenance,
        "legacy-tools-plus-independent-crosswalk"
    );
    let declared_inputs: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("inputs.json")).unwrap(),
    )
    .unwrap();
    let declared_case_ids = declared_inputs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| case["id"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    let oracle_case_ids = oracle
        .cases
        .iter()
        .map(|case| case.id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(oracle_case_ids, declared_case_ids);

    for case in &oracle.cases {
        assert!(
            repo_root().join(&case.input).is_file(),
            "missing declared legacy input {}",
            case.input
        );
        assert!(
            repo_root().join(&case.raw_output).is_file(),
            "missing raw output {}",
            case.raw_output
        );
        if case.profile == "meta-drilldown" {
            assert!(case.parent_case.is_some());
        }
        let envelope = read_oracle_case(case);
        let actual = legacy_comparable_facts(case, &envelope);
        compare_fact_multisets(&case.facts, &actual).unwrap_or_else(|diff| {
            panic!(
                "legacy-comparable parity failed for {} ({})\n{}",
                case.id, case.raw_output, diff
            )
        });
    }
}

#[test]
fn the_real_comparator_rejects_all_required_semantic_mutations() {
    let oracle = oracle();

    let enum_expected = &oracle.enum_coverage;
    let mut wrong_enum_context = expanded_registry_enum_coverage(&coverage_manifest());
    let code_series = wrong_enum_context
        .iter_mut()
        .find(|fact| fact["nativeAlias"] == "WholeCatalog")
        .unwrap();
    code_series["semanticProperty"] = json!("document.number.periodicity");
    assert_comparator_rejects(enum_expected, &wrong_enum_context, "wrong enum context");

    let unknown_case = oracle_case(&oracle, "unknownCases");
    let unknown_envelope = read_oracle_case(unknown_case);
    let unknown_actual = legacy_comparable_facts(unknown_case, &unknown_envelope);
    let mut removed_unknown = unknown_actual.clone();
    let unknown_index = removed_unknown
        .iter()
        .position(|fact| {
            fact["predicate"] == "field.type"
                && canonical_json(&fact["value"]).contains("unknown")
        })
        .expect("legacy unknown type fact");
    removed_unknown.remove(unknown_index);
    assert_comparator_rejects(&unknown_case.facts, &removed_unknown, "removed unknown fact");

    let mut duplicate_node = unknown_actual.clone();
    duplicate_node.push(
        duplicate_node
            .iter()
            .find(|fact| fact["kind"] == "node")
            .unwrap()
            .clone(),
    );
    assert_comparator_rejects(&unknown_case.facts, &duplicate_node, "duplicate node");

    let currency_case = oracle_case(&oracle, "catalogCurrencies");
    let currency_envelope = read_oracle_case(currency_case);
    let currency_actual = legacy_comparable_facts(currency_case, &currency_envelope);
    for (label, mutate) in [
        ("changed value", "value"),
        ("changed type", "type"),
        ("changed state", "state"),
    ] {
        let mut candidate = currency_actual.clone();
        let fact = candidate
            .iter_mut()
            .find(|fact| fact["predicate"] == "catalog.code.length")
            .unwrap();
        match mutate {
            "value" => fact["value"]["value"] = json!(999),
            "type" => fact["value"]["type"] = json!("string"),
            "state" => fact["state"] = json!("absent"),
            _ => unreachable!(),
        }
        assert_comparator_rejects(&currency_case.facts, &candidate, label);
    }

    let artifact_case = oracle_case(&oracle, "ownedArtifacts");
    let artifact_envelope = read_oracle_case(artifact_case);
    let artifact_actual = legacy_comparable_facts(artifact_case, &artifact_envelope);
    let mut missing_relation = artifact_actual.clone();
    let relation_index = missing_relation
        .iter()
        .position(|fact| fact["kind"] == "relation" && fact["predicate"] == "forms")
        .unwrap();
    missing_relation.remove(relation_index);
    assert_comparator_rejects(&artifact_case.facts, &missing_relation, "missing relation");

}

#[test]
fn enum_values_are_projected_only_in_source_declared_property_and_owner_context() {
    let show_warning = read_inline(
        "show-warning",
        "Warnings.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="71000000-0000-0000-0000-000000000001"><Properties><Name>Warnings</Name></Properties><ChildObjects><Attribute uuid="71000000-0000-0000-0000-000000000002"><Properties><Name>Warned</Name><FillChecking>ShowWarning</FillChecking></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#,
    );
    assert_value(
        node(&show_warning, SemanticObjectKind::Attribute, "Warned"),
        SemanticPropertyId::FIELD_FILL_CHECKING,
        PropertyValue::EnumSymbol(SemanticEnumValue::SHOW_WARNING),
    );

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
            "recorder-position-on-calculation-register",
            "CalculationRegister",
            "Periodicity",
            "RecorderPosition",
            SemanticObjectKind::CalculationRegister,
            SemanticPropertyId::REGISTER_PERIODICITY,
        ),
        (
            "information-only-second-on-calculation-register",
            "CalculationRegister",
            "Periodicity",
            "Second",
            SemanticObjectKind::CalculationRegister,
            SemanticPropertyId::REGISTER_PERIODICITY,
        ),
        (
            "module-reuse-on-service",
            "HTTPService",
            "ReuseSessions",
            "DuringRequest",
            SemanticObjectKind::HttpService,
            SemanticPropertyId::HTTP_SERVICE_REUSE_SESSIONS,
        ),
    ] {
        let envelope = read_inline(
            label,
            "CrossContext.xml",
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{class} uuid="71000000-0000-0000-0000-000000000007"><Properties><Name>CrossContext</Name><{property}>{value}</{property}></Properties></{class}></MetaDataObject>"#
            ),
        );
        let property = &node(&envelope, kind, "CrossContext").properties[&property_id];
        assert_eq!(property.value_state(), PropertyValueState::Unresolved, "{label}");
        assert_eq!(property.value(), None, "{label}");
        assert_eq!(envelope.status, NavigationStatus::Partial, "{label}");
    }
}

#[test]
fn hierarchy_and_empty_reference_contracts_remain_adapter_only_and_truthful() {
    let source_root = repo_root().join("tests/fixtures/unica_mcp_script_parity/bsp/meta");
    let envelope = read_path(&source_root, &source_root.join("Catalogs/Валюты.xml"));
    let catalog = node(&envelope, SemanticObjectKind::Catalog, "Валюты");
    assert_value(
        catalog,
        SemanticPropertyId::CATALOG_HIERARCHICAL,
        PropertyValue::Boolean(false),
    );
    assert_value(
        catalog,
        SemanticPropertyId::CATALOG_CODE_SERIES,
        PropertyValue::EnumSymbol(SemanticEnumValue::WHOLE_COLLECTION),
    );
    let attribute = node(&envelope, SemanticObjectKind::Attribute, "ОсновнаяВалюта");
    let fill_value = attribute.properties[&SemanticPropertyId::FIELD_FILL_VALUE]
        .value()
        .expect("tracked EmptyRef must be present");
    assert_eq!(fill_value, &PropertyValue::EmptyReference);
    assert_eq!(
        serde_json::from_str::<PropertyValue>(&serde_json::to_string(fill_value).unwrap())
            .unwrap(),
        PropertyValue::EmptyReference
    );
    assert_ne!(fill_value, &PropertyValue::Null);

    let unlimited = read_tracked("hierarchy/EnabledUnlimited.xml");
    let limited = read_tracked("hierarchy/EnabledLimited.xml");
    let disabled = read_tracked("hierarchy/DisabledContradiction.xml");
    let unlimited = node(&unlimited, SemanticObjectKind::Catalog, "EnabledUnlimited");
    assert_value(
        unlimited,
        SemanticPropertyId::CATALOG_HIERARCHICAL,
        PropertyValue::Boolean(true),
    );
    assert_value(
        unlimited,
        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMITED,
        PropertyValue::Boolean(false),
    );
    assert_absent(unlimited, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT);
    assert_value(
        unlimited,
        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_COUNT,
        PropertyValue::Integer(7),
    );
    assert_value(
        node(&limited, SemanticObjectKind::Catalog, "EnabledLimited"),
        SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT,
        PropertyValue::Integer(4),
    );
    let disabled = node(&disabled, SemanticObjectKind::Catalog, "DisabledContradiction");
    assert_value(
        disabled,
        SemanticPropertyId::CATALOG_HIERARCHICAL,
        PropertyValue::Boolean(false),
    );
    assert_absent(disabled, SemanticPropertyId::CATALOG_HIERARCHY_LEVEL_LIMIT);
}

#[test]
fn rights_mixed_content_remains_typed_where_known_and_opaque_where_unknown() {
    let real = read_real_rights();
    assert_eq!(real.status, NavigationStatus::Available);
    let role = node(&real, SemanticObjectKind::Role, "SalesReader");
    assert_value(
        role,
        SemanticPropertyId::BACKING_CONTENT_AVAILABLE,
        PropertyValue::Boolean(true),
    );
    let view = real
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
        .expect("View permission");
    assert_value(
        view,
        SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS,
        PropertyValue::List(vec![PropertyValue::String(
            "Products.Owner = &CurrentUser".to_string(),
        )]),
    );

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
        <condition futureConditionAttribute="condition-attribute-readable-value">nested-condition<futureNested>nested-readable-value</futureNested>direct-tail</condition>
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
        .find(|node| node.object_ref.kind == SemanticObjectKind::AccessPermission)
        .unwrap();
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
            "direct-tail",
        ]),
    );
    let output = serde_json::to_string(&envelope).unwrap();
    for retained in [
        "nested-condition",
        "nested-readable-value",
        "direct-tail",
        "not-a-condition",
    ] {
        assert!(output.contains(retained), "missing mixed-content evidence {retained}");
    }
    assert!(!output.contains("futureRight"));
    assert!(!output.contains("futureCondition"));
}

#[test]
fn complete_type_variants_and_distinct_unknown_ordinals_remain_structured() {
    let envelope = read_tracked("types/AllTypes.xml");
    assert_eq!(envelope.status, NavigationStatus::Available);
    let type_node = node(&envelope, SemanticObjectKind::DefinedType, "AllTypes");
    let PropertyValue::TypeSet(types) =
        type_node.properties[&SemanticPropertyId::DEFINED_TYPE].value().unwrap()
    else {
        panic!("defined type set")
    };
    assert_eq!(types.variants().len(), 14);
    let serialized = serde_json::to_string(types).unwrap();
    for category in [
        "uuid",
        "opaque",
        "null",
        "reference",
        "object",
        "recordSet",
        "manager",
        "key",
        "enumeration",
        "definedType",
    ] {
        assert!(serialized.contains(category), "missing type category {category}");
    }
    for primitive in [
        PrimitiveTypeKind::String,
        PrimitiveTypeKind::Number,
        PrimitiveTypeKind::Date,
    ] {
        assert!(types.variants().iter().any(|variant| {
            variant.primitive_kind() == Some(primitive) && variant.qualifiers().is_some()
        }));
    }

    let unknowns = read_tracked("unknowns/UnknownCases.xml");
    assert_eq!(unknowns.status, NavigationStatus::Partial);
    let field = node(&unknowns, SemanticObjectKind::Attribute, "MysteryType");
    let serialized = serde_json::to_string(
        field.properties[&SemanticPropertyId::FIELD_TYPE]
            .value()
            .unwrap(),
    )
    .unwrap();
    assert!(serialized.contains(r#""ordinal":1"#));
    assert!(serialized.contains(r#""ordinal":2"#));
    assert!(!unknowns.diagnostics.is_empty());
}

#[test]
fn form_template_backing_and_adapter_only_status_facets_are_checked_separately() {
    let owned = read_tracked("artifacts/ArtifactReport.xml");
    assert_eq!(owned.status, NavigationStatus::Partial);
    let form = node(&owned, SemanticObjectKind::Form, "MainForm");
    assert_value(
        form,
        SemanticPropertyId::FORM_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::MANAGED),
    );
    assert_value(
        form,
        SemanticPropertyId::BACKING_DESCRIPTOR_AVAILABLE,
        PropertyValue::Boolean(true),
    );
    assert_value(
        form,
        SemanticPropertyId::BACKING_CONTENT_AVAILABLE,
        PropertyValue::Boolean(true),
    );
    assert_value(
        form,
        SemanticPropertyId::BACKING_CONTENT_OPAQUE,
        PropertyValue::Boolean(true),
    );
    let template = node(&owned, SemanticObjectKind::Template, "MainSchema");
    assert_value(
        template,
        SemanticPropertyId::TEMPLATE_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::DATA_COMPOSITION_SCHEMA),
    );
    assert_value(
        template,
        SemanticPropertyId::BACKING_CONTENT_OPAQUE,
        PropertyValue::Boolean(true),
    );
    assert!(!owned.diagnostics.is_empty());

    let common_form = read_tracked("common_form/CommonDashboard.xml");
    let form = node(
        &common_form,
        SemanticObjectKind::CommonForm,
        "CommonDashboard",
    );
    assert_value(
        form,
        SemanticPropertyId::BACKING_CONTENT_AVAILABLE,
        PropertyValue::Boolean(true),
    );
    let common_template = read_tracked("common_template/CommonLayout.xml");
    let template = node(
        &common_template,
        SemanticObjectKind::CommonTemplate,
        "CommonLayout",
    );
    assert_value(
        template,
        SemanticPropertyId::BACKING_CONTENT_AVAILABLE,
        PropertyValue::Boolean(true),
    );
}

fn oracle() -> LegacyOracle {
    serde_json::from_slice(
        &fs::read(oracle_root().join("legacy-semantic-oracle.json")).unwrap(),
    )
    .unwrap()
}

fn oracle_case<'a>(oracle: &'a LegacyOracle, id: &str) -> &'a LegacyCase {
    oracle
        .cases
        .iter()
        .find(|case| case.id == id)
        .unwrap_or_else(|| panic!("missing legacy oracle case {id}"))
}

fn coverage_manifest() -> Value {
    serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/versions/v2_20/coverage.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn expanded_registry_enum_coverage(manifest: &Value) -> Vec<Value> {
    let mut facts = Vec::new();
    for alias in manifest["enumAliases"].as_array().unwrap() {
        for property_id in alias["propertyIds"].as_array().unwrap() {
            for object_kind in alias["objectKinds"].as_array().unwrap() {
                for property in manifest["properties"].as_array().unwrap() {
                    if property["semanticProperty"] != *property_id
                        || !property["objectKinds"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .any(|kind| kind == object_kind)
                    {
                        continue;
                    }
                    for native_alias in alias["nativeAliases"].as_array().unwrap() {
                        for native_property in property["nativeNames"].as_array().unwrap() {
                            facts.push(json!({
                                "nativeAlias": native_alias,
                                "nativeProperty": native_property,
                                "objectKind": object_kind,
                                "semantic": alias["semantic"],
                                "semanticProperty": property_id,
                            }));
                        }
                    }
                }
            }
        }
    }
    facts
}

fn compare_fact_multisets(expected: &[Value], actual: &[Value]) -> Result<(), FactDiff> {
    let counts = |values: &[Value]| {
        let mut result = BTreeMap::<String, usize>::new();
        for value in values {
            *result.entry(canonical_json(value)).or_default() += 1;
        }
        result
    };
    let expected = counts(expected);
    let actual = counts(actual);
    let mut missing = Vec::new();
    let mut unexpected = Vec::new();
    for (fact, expected_count) in &expected {
        let actual_count = actual.get(fact).copied().unwrap_or_default();
        if actual_count < *expected_count {
            missing.push((fact.clone(), expected_count - actual_count));
        }
    }
    for (fact, actual_count) in &actual {
        let expected_count = expected.get(fact).copied().unwrap_or_default();
        if expected_count < *actual_count {
            unexpected.push((fact.clone(), actual_count - expected_count));
        }
    }
    if missing.is_empty() && unexpected.is_empty() {
        Ok(())
    } else {
        let first_difference = missing.iter().find_map(|(missing, _)| {
            let expected: Value = serde_json::from_str(missing).ok()?;
            let expected_case = expected.get("case");
            let expected_kind = expected.get("kind");
            unexpected.iter().find_map(|(unexpected, _)| {
                let actual: Value = serde_json::from_str(unexpected).ok()?;
                if actual.get("case") != expected_case || actual.get("kind") != expected_kind {
                    return None;
                }
                first_json_difference(&expected, &actual, "$")
            })
        });
        Err(FactDiff {
            missing,
            unexpected,
            first_difference,
        })
    }
}

fn first_json_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let keys = expected
                .keys()
                .chain(actual.keys())
                .cloned()
                .collect::<BTreeSet<_>>();
            for key in keys {
                let next = format!("{path}.{key}");
                match (expected.get(&key), actual.get(&key)) {
                    (Some(expected), Some(actual)) => {
                        if let Some(difference) =
                            first_json_difference(expected, actual, &next)
                        {
                            return Some(difference);
                        }
                    }
                    (Some(_), None) => return Some(format!("{next}: missing")),
                    (None, Some(_)) => return Some(format!("{next}: unexpected")),
                    (None, None) => unreachable!(),
                }
            }
            None
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                return Some(format!(
                    "{path}: expected {} items, got {}",
                    expected.len(),
                    actual.len()
                ));
            }
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                if let Some(difference) =
                    first_json_difference(expected, actual, &format!("{path}[{index}]"))
                {
                    return Some(difference);
                }
            }
            None
        }
        _ if expected == actual => None,
        _ => Some(format!(
            "{path}: expected {}, got {}",
            canonical_json(expected),
            canonical_json(actual)
        )),
    }
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap(),
                        canonical_json(value)
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn assert_comparator_rejects(expected: &[Value], actual: &[Value], label: &str) {
    let diff = compare_fact_multisets(expected, actual)
        .unwrap_err_or_else(|| panic!("{label} was accepted by the real comparator"));
    assert!(
        !diff.missing.is_empty() || !diff.unexpected.is_empty(),
        "{label} returned an empty structured diff"
    );
}

trait ResultExt<T, E> {
    fn unwrap_err_or_else(self, on_ok: impl FnOnce() -> E) -> E;
}

impl<T, E> ResultExt<T, E> for Result<T, E> {
    fn unwrap_err_or_else(self, on_ok: impl FnOnce() -> E) -> E {
        match self {
            Ok(_) => on_ok(),
            Err(error) => error,
        }
    }
}

fn node_fact(subject: &str, kind: &str, name: &str) -> Value {
    json!({
        "kind": "node",
        "subject": subject,
        "state": "present",
        "value": {"kind": kind, "name": name},
    })
}

fn property_fact(subject: &str, predicate: &str, value: Value) -> Value {
    json!({
        "kind": "property",
        "subject": subject,
        "predicate": predicate,
        "state": "present",
        "value": value,
    })
}

fn relation_fact(
    subject: &str,
    predicate: &str,
    target: &str,
    target_kind: &str,
    target_name: &str,
) -> Value {
    json!({
        "kind": "relation",
        "subject": subject,
        "predicate": predicate,
        "state": "present",
        "value": {
            "target": target,
            "targetKind": target_kind,
            "targetName": target_name,
        },
    })
}

fn legacy_comparable_facts(case: &LegacyCase, envelope: &NavigationEnvelope) -> Vec<Value> {
    if case.profile == "role-info" {
        return legacy_role_facts(case, envelope);
    }
    legacy_meta_facts(case, envelope)
}

fn legacy_meta_facts(case: &LegacyCase, envelope: &NavigationEnvelope) -> Vec<Value> {
    let root_kind = SemanticObjectKind::parse(&case.root_kind).unwrap();
    let root = node(envelope, root_kind, &case.root_name);
    let selected = if case.profile == "meta-drilldown" {
        let descriptor = case
            .facts
            .iter()
            .find(|fact| fact["kind"] == "node")
            .expect("drill-down node fact");
        Some((
            descriptor["value"]["kind"].as_str().unwrap(),
            descriptor["value"]["name"].as_str().unwrap(),
        ))
    } else {
        None
    };

    let mut projected_nodes = Vec::<&NavigationNode>::new();
    if selected.is_none() {
        projected_nodes.push(root);
    }
    for candidate in &envelope.nodes {
        if candidate.object_ref == root.object_ref {
            continue;
        }
        let kind = legacy_kind(candidate.object_ref.kind);
        let relevant = matches!(
            kind,
            "attribute"
                | "dimension"
                | "resource"
                | "tabularSection"
                | "enumerationValue"
                | "form"
                | "template"
                | "command"
        );
        if !relevant {
            continue;
        }
        if let Some((selected_kind, selected_name)) = selected {
            if kind != selected_kind || candidate.object_ref.display_name != selected_name {
                continue;
            }
        }
        projected_nodes.push(candidate);
    }

    let mut counts = BTreeMap::<(String, String), usize>::new();
    let mut identities = Vec::<(ObjectRef, String, String, String)>::new();
    for candidate in &projected_nodes {
        let kind = legacy_kind(candidate.object_ref.kind).to_string();
        let name = candidate.object_ref.display_name.clone();
        let identity = if candidate.object_ref == root.object_ref && selected.is_none() {
            format!("{}/root", case.id)
        } else {
            let count = counts.entry((kind.clone(), name.clone())).or_default();
            *count += 1;
            format!("{}/{kind}/{name}#{}", case.id, *count)
        };
        identities.push((candidate.object_ref.clone(), identity, kind, name));
    }
    let identity = |reference: &ObjectRef| {
        identities
            .iter()
            .find(|(candidate, _, _, _)| candidate == reference)
            .map(|(_, identity, _, _)| identity.clone())
    };

    let mut facts = Vec::new();
    for (reference, subject, kind, name) in &identities {
        facts.push(node_fact(subject, kind, name));
        let candidate = envelope
            .nodes
            .iter()
            .find(|node| &node.object_ref == reference)
            .unwrap();
        if reference == &root.object_ref {
            add_root_properties(&mut facts, subject, candidate);
        } else {
            add_child_properties(&mut facts, subject, candidate, &case.profile);
        }
    }

    let comparable_relations = BTreeSet::from([
        "attributes",
        "dimensions",
        "resources",
        "tabularSections",
        "columns",
        "forms",
        "templates",
        "commands",
        "enumValues",
        "basedOn",
    ]);
    for relation in envelope.relation_index.iter() {
        let role = relation.role.as_str();
        if !comparable_relations.contains(role) {
            continue;
        }
        let Some(source) = identity(&relation.source) else {
            continue;
        };
        if role == "basedOn" {
            let name = relation.target.display_name.clone();
            facts.push(relation_fact(
                &source,
                role,
                &format!("{}/external/unknown/{name}", case.id),
                "unknown",
                &name,
            ));
            continue;
        }
        let Some(target) = identity(&relation.target) else {
            continue;
        };
        facts.push(relation_fact(
            &source,
            role,
            &target,
            legacy_kind(relation.target.kind),
            &relation.target.display_name,
        ));
    }
    facts
}

fn add_root_properties(facts: &mut Vec<Value>, subject: &str, node: &NavigationNode) {
    add_distinct_synonym(facts, subject, node);
    add_support_fact(facts, subject, node);
    match node.object_ref.kind {
        SemanticObjectKind::Catalog => {
            add_type_presentation(facts, subject, node);
            for id in [
                SemanticPropertyId::PRESENTATION_OBJECT,
                SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT,
                SemanticPropertyId::PRESENTATION_LIST,
                SemanticPropertyId::PRESENTATION_EXTENDED_LIST,
                SemanticPropertyId::CATALOG_CODE_LENGTH,
                SemanticPropertyId::CATALOG_DESCRIPTION_LENGTH,
            ] {
                add_direct_property(facts, subject, node, id);
            }
        }
        SemanticObjectKind::Document => {
            add_type_presentation(facts, subject, node);
            for id in [
                SemanticPropertyId::PRESENTATION_OBJECT,
                SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT,
                SemanticPropertyId::PRESENTATION_LIST,
                SemanticPropertyId::PRESENTATION_EXTENDED_LIST,
                SemanticPropertyId::DOCUMENT_NUMBER_TYPE,
                SemanticPropertyId::DOCUMENT_NUMBER_LENGTH,
                SemanticPropertyId::DOCUMENT_NUMBER_PERIODICITY,
                SemanticPropertyId::DOCUMENT_NUMBER_AUTO,
                SemanticPropertyId::DOCUMENT_POSTING_MODE,
            ] {
                add_direct_property(facts, subject, node, id);
            }
        }
        SemanticObjectKind::Enumeration => {
            add_type_presentation(facts, subject, node);
            for id in [
                SemanticPropertyId::PRESENTATION_OBJECT,
                SemanticPropertyId::PRESENTATION_EXTENDED_OBJECT,
                SemanticPropertyId::PRESENTATION_LIST,
                SemanticPropertyId::PRESENTATION_EXTENDED_LIST,
            ] {
                add_direct_property(facts, subject, node, id);
            }
        }
        SemanticObjectKind::CommonModule => {
            for id in [
                SemanticPropertyId::MODULE_GLOBAL,
                SemanticPropertyId::MODULE_CLIENT_MANAGED_APPLICATION,
                SemanticPropertyId::MODULE_SERVER,
                SemanticPropertyId::MODULE_EXTERNAL_CONNECTION,
                SemanticPropertyId::MODULE_CLIENT_ORDINARY_APPLICATION,
                SemanticPropertyId::MODULE_SERVER_CALL,
            ] {
                if node.properties.get(&id).and_then(|property| property.value())
                    == Some(&PropertyValue::Boolean(true))
                {
                    add_direct_property(facts, subject, node, id);
                }
            }
        }
        SemanticObjectKind::InformationRegister => {
            add_direct_property(facts, subject, node, SemanticPropertyId::REGISTER_PERIODICITY);
            add_direct_property(facts, subject, node, SemanticPropertyId::REGISTER_WRITE_MODE);
        }
        SemanticObjectKind::Report => add_direct_property(
            facts,
            subject,
            node,
            SemanticPropertyId::REPORT_MAIN_DATA_COMPOSITION_SCHEMA,
        ),
        SemanticObjectKind::DefinedType => {
            add_direct_property(facts, subject, node, SemanticPropertyId::DEFINED_TYPE)
        }
        SemanticObjectKind::EventSubscription => {
            add_legacy_subscription_property(
                facts,
                subject,
                node,
                SemanticPropertyId::SUBSCRIPTION_EVENT,
            );
            add_legacy_subscription_property(
                facts,
                subject,
                node,
                SemanticPropertyId::SUBSCRIPTION_HANDLER,
            );
            add_direct_property(
                facts,
                subject,
                node,
                SemanticPropertyId::SUBSCRIPTION_SOURCE_TYPE,
            );
        }
        _ => {}
    }
}

fn add_type_presentation(facts: &mut Vec<Value>, subject: &str, node: &NavigationNode) {
    let value = [
        SemanticPropertyId::PRESENTATION_OBJECT,
        SemanticPropertyId::METADATA_SYNONYM,
    ]
    .into_iter()
    .find_map(|id| node.properties.get(&id).and_then(|property| property.value()))
    .map(legacy_value)
    .unwrap_or_else(|| json!({"type": "localizedString", "value": {"ru": node.object_ref.display_name}}));
    facts.push(property_fact(subject, "presentation.type", value));
}

fn add_support_fact(facts: &mut Vec<Value>, subject: &str, node: &NavigationNode) {
    let Some(PropertyValue::EnumSymbol(state)) = node
        .properties
        .get(&SemanticPropertyId::SUPPORT_STATE)
        .and_then(|property| property.value())
    else {
        return;
    };
    facts.push(property_fact(
        subject,
        "support.active",
        json!({
            "type": "boolean",
            "value": !matches!(
                *state,
                unica_format_core::semantic_ids::SemanticEnumValue::NOT_SUPPORTED
                    | unica_format_core::semantic_ids::SemanticEnumValue::REMOVED_FROM_SUPPORT
            ),
        }),
    ));
    facts.push(property_fact(
        subject,
        "support.state",
        json!({"type": "enum", "value": state.as_str()}),
    ));
}

fn add_legacy_subscription_property(
    facts: &mut Vec<Value>,
    subject: &str,
    node: &NavigationNode,
    id: SemanticPropertyId,
) {
    let Some(PropertyValue::String(value)) =
        node.properties.get(&id).and_then(|property| property.value())
    else {
        return;
    };
    let normalized = if id == SemanticPropertyId::SUBSCRIPTION_EVENT {
        legacy_crosswalk()["valueMappings"]["subscriptionEvent"]
            .get(value)
            .and_then(Value::as_str)
            .unwrap_or(value)
            .to_string()
    } else {
        value
            .strip_prefix("CommonModule.")
            .unwrap_or(value)
            .to_string()
    };
    facts.push(property_fact(
        subject,
        id.as_str(),
        json!({"type": "string", "value": normalized}),
    ));
}

fn legacy_crosswalk() -> &'static Value {
    static CROSSWALK: OnceLock<Value> = OnceLock::new();
    CROSSWALK.get_or_init(|| {
        serde_json::from_slice(&fs::read(oracle_root().join("crosswalk.json")).unwrap()).unwrap()
    })
}

fn add_distinct_synonym(facts: &mut Vec<Value>, subject: &str, node: &NavigationNode) {
    let Some(PropertyValue::LocalizedString(values)) = node
        .properties
        .get(&SemanticPropertyId::METADATA_SYNONYM)
        .and_then(|property| property.value())
    else {
        return;
    };
    if values.values().all(|value| value == &node.object_ref.display_name) {
        return;
    }
    add_direct_property(
        facts,
        subject,
        node,
        SemanticPropertyId::METADATA_SYNONYM,
    );
}

fn add_child_properties(
    facts: &mut Vec<Value>,
    subject: &str,
    node: &NavigationNode,
    profile: &str,
) {
    match node.object_ref.kind {
        SemanticObjectKind::Attribute
        | SemanticObjectKind::Dimension
        | SemanticObjectKind::Resource => {
            for id in [
                SemanticPropertyId::FIELD_TYPE,
                SemanticPropertyId::FIELD_REQUIRED,
                SemanticPropertyId::FIELD_INDEXING,
            ] {
                add_direct_property(facts, subject, node, id);
            }
            if profile == "meta-drilldown" {
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_FILL_VALUE);
                add_distinct_synonym(facts, subject, node);
            }
            if node
                .properties
                .get(&SemanticPropertyId::FIELD_USE)
                .and_then(|property| property.value())
                .is_some_and(|value| {
                    value != &PropertyValue::EnumSymbol(SemanticEnumValue::FOR_ITEM)
                })
            {
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_USE);
            }
            if node
                .properties
                .get(&SemanticPropertyId::FIELD_MULTI_LINE)
                .and_then(|property| property.value())
                == Some(&PropertyValue::Boolean(true))
            {
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_MULTI_LINE);
            }
            if node.object_ref.kind == SemanticObjectKind::Dimension && profile == "meta-drilldown" {
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_MASTER);
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_MAIN_FILTER);
            } else if node
                .properties
                .get(&SemanticPropertyId::FIELD_MASTER)
                .and_then(|property| property.value())
                == Some(&PropertyValue::Boolean(true))
            {
                add_direct_property(facts, subject, node, SemanticPropertyId::FIELD_MASTER);
            }
        }
        SemanticObjectKind::EnumerationValue => {
            add_distinct_synonym(facts, subject, node);
        }
        _ => {}
    }
}

fn add_direct_property(
    facts: &mut Vec<Value>,
    subject: &str,
    node: &NavigationNode,
    id: SemanticPropertyId,
) {
    let Some(value) = node.properties.get(&id).and_then(|property| property.value()) else {
        return;
    };
    facts.push(property_fact(subject, id.as_str(), legacy_value(value)));
}

fn legacy_value(value: &PropertyValue) -> Value {
    match value {
        PropertyValue::TypeSet(types) => {
            let serialized = serde_json::to_value(types).unwrap();
            let mut variants = serialized["variants"]
                .as_array()
                .unwrap()
                .iter()
                .map(legacy_type_variant)
                .collect::<Vec<_>>();
            variants.sort_by_key(canonical_json);
            json!({"type": "typeSet", "value": {"variants": variants}})
        }
        _ => serde_json::to_value(value).unwrap(),
    }
}

fn legacy_type_variant(variant: &Value) -> Value {
    let kind = variant["kind"].as_str().unwrap();
    if kind == "primitive" {
        let primitive = variant["primitive"].as_str().unwrap();
        let mut result = serde_json::Map::from_iter([
            ("kind".to_string(), json!("primitive")),
            ("primitive".to_string(), json!(primitive)),
        ]);
        let qualifiers = &variant["qualifiers"];
        match primitive {
            "string" => {
                if let Some(length) = qualifiers["string"]["length"].as_u64() {
                    result.insert("length".to_string(), json!(length));
                }
            }
            "number" => {
                if let Some(digits) = qualifiers["number"]["digits"].as_u64() {
                    result.insert("digits".to_string(), json!(digits));
                }
                if let Some(fraction) = qualifiers["number"]["fractionDigits"].as_u64() {
                    result.insert("fractionDigits".to_string(), json!(fraction));
                }
            }
            "date" => {
                if let Some(fractions) = qualifiers["date"]["dateFractions"].as_str() {
                    result.insert("dateFractions".to_string(), json!(fractions));
                }
            }
            _ => {}
        }
        return Value::Object(result);
    }
    if kind == "unknown" {
        return json!({"kind": "unknown", "ordinal": variant["ordinal"]});
    }
    json!({
        "kind": kind,
        "targetKind": variant["target"]["kind"],
        "targetName": variant["target"]["name"],
    })
}

fn legacy_role_facts(case: &LegacyCase, envelope: &NavigationEnvelope) -> Vec<Value> {
    let role = node(envelope, SemanticObjectKind::Role, &case.root_name);
    let root = format!("{}/root", case.id);
    let mut facts = vec![node_fact(&root, "role", &case.root_name)];
    for id in [
        SemanticPropertyId::ACCESS_NEW_OBJECTS_DEFAULT,
        SemanticPropertyId::ACCESS_ATTRIBUTES_DEFAULT,
        SemanticPropertyId::ACCESS_CHILD_OBJECTS_INDEPENDENT,
    ] {
        add_direct_property(&mut facts, &root, role, id);
    }
    add_distinct_synonym(&mut facts, &root, role);
    add_support_fact(&mut facts, &root, role);

    let permissions = envelope
        .nodes
        .iter()
        .filter(|node| node.object_ref.kind == SemanticObjectKind::AccessPermission)
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::<(String, String, String), usize>::new();
    let mut allowed_count = 0;
    let mut denied_count = 0;
    let mut restricted = BTreeMap::<String, (String, String)>::new();
    for permission in permissions {
        let name = match permission.properties[&SemanticPropertyId::ACCESS_PERMISSION_NAME]
            .value()
            .unwrap()
        {
            PropertyValue::String(value) => value.clone(),
            value => panic!("permission name is not a string: {value:?}"),
        };
        let allowed = permission.properties[&SemanticPropertyId::ACCESS_PERMISSION_ALLOWED]
            .value()
            == Some(&PropertyValue::Boolean(true));
        if allowed {
            allowed_count += 1;
        } else {
            denied_count += 1;
        }
        let target = envelope
            .relation_index
            .iter()
            .find(|relation| {
                relation.source == permission.object_ref
                    && relation.role.as_str() == "accessTarget"
            })
            .expect("permission target");
        let target_kind = legacy_kind(target.target.kind).to_string();
        let target_name = target.target.display_name.clone();
        let count = counts
            .entry((target_kind.clone(), target_name.clone(), name.clone()))
            .or_default();
        *count += 1;
        let subject = format!(
            "{}/accessPermission/{}:{}:{}#{}",
            case.id, target_kind, target_name, name, count
        );
        let target_identity = format!(
            "{}/external/{}/{}",
            case.id, target_kind, target_name
        );
        facts.push(node_fact(&subject, "accessPermission", &name));
        facts.push(property_fact(
            &subject,
            "access.permission.name",
            json!({"type": "string", "value": name}),
        ));
        facts.push(property_fact(
            &subject,
            "access.permission.allowed",
            json!({"type": "boolean", "value": allowed}),
        ));
        facts.push(relation_fact(
            &root,
            "accessPermissions",
            &subject,
            "accessPermission",
            &name,
        ));
        facts.push(relation_fact(
            &subject,
            "accessTarget",
            &target_identity,
            &target_kind,
            &target_name,
        ));
        if permission
            .properties
            .get(&SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS)
            .and_then(|property| property.value())
            .is_some_and(|value| matches!(value, PropertyValue::List(values) if !values.is_empty()))
        {
            restricted.insert(target_identity, (target_kind, target_name));
        }
    }
    for (target, (kind, name)) in &restricted {
        facts.push(node_fact(target, kind, name));
        facts.push(property_fact(
            target,
            "access.restriction.present",
            json!({"type": "boolean", "value": true}),
        ));
    }
    facts.push(property_fact(
        &root,
        "access.restriction.count",
        json!({"type": "integer", "value": restricted.len()}),
    ));
    facts.push(property_fact(
        &root,
        "access.allowed.count",
        json!({"type": "integer", "value": allowed_count}),
    ));
    facts.push(property_fact(
        &root,
        "access.denied.count",
        json!({"type": "integer", "value": denied_count}),
    ));

    for template in envelope
        .nodes
        .iter()
        .filter(|node| node.object_ref.kind == SemanticObjectKind::AccessRestrictionTemplate)
    {
        let name = template
            .object_ref
            .display_name
            .split_once('(')
            .map_or(template.object_ref.display_name.as_str(), |(name, _)| name);
        let subject = format!("{}/accessRestrictionTemplate/{}#1", case.id, name);
        facts.push(node_fact(&subject, "accessRestrictionTemplate", name));
        facts.push(property_fact(
            &subject,
            "access.restrictionTemplate.name",
            json!({"type": "string", "value": name}),
        ));
        facts.push(relation_fact(
            &root,
            "restrictionTemplates",
            &subject,
            "accessRestrictionTemplate",
            name,
        ));
    }
    facts
}

fn legacy_kind(kind: SemanticObjectKind) -> &'static str {
    if kind == SemanticObjectKind::SpreadsheetDocumentTemplate {
        "template"
    } else {
        kind.as_str()
    }
}

fn assert_manifest_mutation_rejected(
    raw: &str,
    label: &str,
    mutate: impl FnOnce(&mut Value),
) {
    let mut manifest: Value = serde_json::from_str(raw).unwrap();
    mutate(&mut manifest);
    assert!(
        PlatformXmlAdapterFactory::validate_2_20_coverage_manifest(
            &serde_json::to_string(&manifest).unwrap()
        )
        .is_err(),
        "{label} must not be accepted by runtime coverage validation"
    );
}

fn read_oracle_case(case: &LegacyCase) -> NavigationEnvelope {
    if case.profile == "role-info" {
        let root = temp_root("legacy-rights");
        let descriptor_name = Path::new(&case.adapter_input)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap();
        fs::create_dir_all(root.join(descriptor_name).join("Ext")).unwrap();
        fs::copy(
            repo_root().join(&case.adapter_input),
            root.join(format!("{descriptor_name}.xml")),
        )
        .unwrap();
        fs::copy(
            repo_root().join(&case.input),
            root.join(descriptor_name).join("Ext/Rights.xml"),
        )
        .unwrap();
        let envelope = read_path(&root, &root.join(format!("{descriptor_name}.xml")));
        fs::remove_dir_all(root).unwrap();
        envelope
    } else {
        read_path(
            &repo_root().join(&case.source_root),
            &repo_root().join(&case.adapter_input),
        )
    }
}

fn read_real_rights() -> NavigationEnvelope {
    let case = oracle();
    read_oracle_case(oracle_case(&case, "rights"))
}

fn read_tracked(relative: &str) -> NavigationEnvelope {
    let target = tracked_root().join(relative);
    let source_root = target.parent().expect("tracked fixture parent").to_path_buf();
    read_path(&source_root, &target)
}

fn read_path(source_root: &Path, target: &Path) -> NavigationEnvelope {
    let source = SourceContext::new(
        SourceLocation::new(repo_root(), source_root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("fixture must be captured")
    };
    registration
        .read
        .read(&FormatReadRequest {
            captured: captured.clone(),
            query: NavigationQuery {
                target: NavigationTarget::CapturedTarget(
                    captured.binding().target_identity.clone(),
                ),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
        })
        .expect("structurally readable Platform XML must project")
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

fn node<'a>(
    envelope: &'a NavigationEnvelope,
    kind: SemanticObjectKind,
    name: &str,
) -> &'a NavigationNode {
    envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.kind == kind && node.object_ref.display_name == name)
        .unwrap_or_else(|| panic!("missing {kind} node {name}"))
}

fn assert_value(node: &NavigationNode, id: SemanticPropertyId, expected: PropertyValue) {
    assert_eq!(
        node.properties
            .get(&id)
            .unwrap_or_else(|| panic!("missing property {id}"))
            .value(),
        Some(&expected),
        "unexpected value for {id}"
    );
}

fn assert_absent(node: &NavigationNode, id: SemanticPropertyId) {
    let property = node
        .properties
        .get(&id)
        .unwrap_or_else(|| panic!("missing property {id}"));
    assert_eq!(
        property.value_state(),
        PropertyValueState::Absent,
        "{id} must be explicitly inactive"
    );
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

fn oracle_root() -> PathBuf {
    tracked_root().join("legacy-oracle")
}

fn tracked_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_20")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "unica-platform-xml-task5-fix4-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ))
}
#[test]
fn fix_round5_legacy_oracle_negative_suite_is_fail_closed() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let generator = repo_root.join(
        "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py",
    );
    let output = std::process::Command::new("python3.12")
        .arg(generator)
        .arg("--repo-root")
        .arg(&repo_root)
        .arg("--self-test")
        .output()
        .expect("run legacy oracle negative suite");

    assert!(
        output.status.success(),
        "legacy oracle negative suite failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn fix_round5_static_new_only_contract_is_exact_and_mutation_sensitive() {
    let contract_path = oracle_root().join("new-only-contract.json");
    let contract: Value =
        serde_json::from_slice(&fs::read(&contract_path).expect("static new-only contract"))
            .unwrap();
    assert_eq!(contract["schemaVersion"], 2);
    assert_eq!(
        contract["provenance"],
        "independent-v1-inventory-plus-native-fixtures-and-closed-core-contracts"
    );
    let schema = contract["publicSchema"].as_object().unwrap();
    let cases = contract["cases"].as_array().expect("contract cases");
    assert!(!cases.is_empty());

    let mut all_expected = Vec::new();
    let mut all_actual = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let source_root = repo_root().join(case["sourceRoot"].as_str().unwrap());
        let input = repo_root().join(case["input"].as_str().unwrap());
        let envelope = read_path(&source_root, &input);
        let expected = case["facts"].as_array().expect("contract facts");
        let actual = adapter_only_contract_facts(id, &envelope);
        validate_contract_public_schema(schema, &actual).unwrap_or_else(|error| {
            panic!("adapter-only public schema drifted for {id}: {error}")
        });
        compare_fact_multisets(expected, &actual).unwrap_or_else(|diff| {
            panic!("adapter-only exact contract drifted for {id}:\n{diff}")
        });
        all_expected.extend(expected.iter().cloned());
        all_actual.extend(actual);
    }

    let envelope_index = all_actual
        .iter()
        .position(|fact| {
            fact["kind"] == "envelope"
                && fact["value"]["nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.iter().any(|node| {
                        !node["properties"].as_object().unwrap().is_empty()
                    }))
                && !fact["value"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .is_empty()
        })
        .expect("contract envelope with properties and diagnostics");
    let relation_index = all_actual
        .iter()
        .position(|fact| fact["kind"] == "semanticRelation")
        .expect("contract semantic relation");

    let mut omitted = all_actual.clone();
    omitted.remove(envelope_index);
    assert_comparator_rejects(&all_expected, &omitted, "omitted envelope");
    let mut added = all_actual.clone();
    added.push(all_actual[envelope_index].clone());
    assert_comparator_rejects(&all_expected, &added, "duplicate envelope");

    for path in [
        &["status"][..],
        &["snapshot", "consistency"][..],
        &["root", "objectKey"][..],
        &["nodes", "0", "capabilityState", "authorability"][..],
        &["nodes", "0", "capability", "resolution"][..],
        &["nodes", "0", "capability", "identity"][..],
        &["nodes", "0", "capability", "consistency"][..],
        &["nodes", "0", "capability", "coverage"][..],
        &["nodes", "0", "capability", "format"][..],
        &["nodes", "0", "capability", "sourceAccess"][..],
        &["nodes", "0", "capability", "authorability"][..],
        &["nodes", "0", "actionProfile"][..],
        &["nodes", "0", "actions", "0", "availability"][..],
        &["diagnostics", "0", "code"][..],
    ] {
        let mut changed = all_actual.clone();
        mutate_json_path(
            &mut changed[envelope_index]["value"],
            path,
            json!("must-fail"),
        );
        assert_comparator_rejects(
            &all_expected,
            &changed,
            &format!("changed envelope field {}", path.join(".")),
        );
    }

    let property_node_index = all_actual[envelope_index]["value"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .position(|node| !node["properties"].as_object().unwrap().is_empty())
        .unwrap();
    let property_name = all_actual[envelope_index]["value"]["nodes"][property_node_index]
        ["properties"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    for field in ["type", "valueState", "value", "provenance", "capability"] {
        let mut changed = all_actual.clone();
        changed[envelope_index]["value"]["nodes"][property_node_index]["properties"]
            [&property_name][field] = json!("must-fail");
        assert_comparator_rejects(
            &all_expected,
            &changed,
            &format!("changed property {field}"),
        );
    }

    for path in [
        &["relationRef", "relationKey"][..],
        &["groupRef", "groupKey"][..],
        &["groupRef", "owner", "objectKey"][..],
        &["identityStrength"][..],
        &["kind"][..],
        &["role"][..],
        &["source", "objectKey"][..],
        &["target", "objectKey"][..],
        &["capability", "resolution"][..],
        &["capability", "identity"][..],
        &["capability", "consistency"][..],
        &["capability", "coverage"][..],
        &["capability", "format"][..],
        &["capability", "sourceAccess"][..],
        &["capability", "authorability"][..],
    ] {
        let mut changed = all_actual.clone();
        mutate_json_path(
            &mut changed[relation_index]["value"],
            path,
            json!("must-fail"),
        );
        assert_comparator_rejects(
            &all_expected,
            &changed,
            &format!("changed relation field {}", path.join(".")),
        );
    }

    let mut added_field = all_actual.clone();
    added_field[envelope_index]["value"]["futureField"] = json!(true);
    assert!(validate_contract_public_schema(schema, &added_field).is_err());
    let mut removed_field = all_actual.clone();
    removed_field[envelope_index]["value"]
        .as_object_mut()
        .unwrap()
        .remove("status");
    assert!(validate_contract_public_schema(schema, &removed_field).is_err());
}

#[test]
fn fix_round6_adapter_only_normalizer_covers_complete_public_schema() {
    let contract: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("new-only-contract.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(contract["schemaVersion"], 2);
    let schema = contract["publicSchema"]
        .as_object()
        .expect("checked-in public schema guard");
    for required in [
        "NavigationEnvelope",
        "NavigationNode",
        "SemanticProperty",
        "CapabilityVector",
        "SemanticRelation",
        "RelationRef",
        "RelationGroupRef",
        "ObjectRef",
        "SourceSnapshot",
        "SourceAdapterDiagnostic",
    ] {
        assert!(schema.contains_key(required), "missing schema guard {required}");
    }

    let case = contract["cases"].as_array().unwrap().first().unwrap();
    let id = case["id"].as_str().unwrap();
    let source_root = repo_root().join(case["sourceRoot"].as_str().unwrap());
    let input = repo_root().join(case["input"].as_str().unwrap());
    let facts = adapter_only_contract_facts(id, &read_path(&source_root, &input));
    let envelope = facts
        .iter()
        .find(|fact| fact["kind"] == "envelope")
        .expect("whole public envelope fact");
    let envelope_value = envelope["value"].as_object().unwrap();
    assert_eq!(
        envelope_value.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "diagnostics".to_string(),
            "nodes".to_string(),
            "relations".to_string(),
            "root".to_string(),
            "schemaVersion".to_string(),
            "snapshot".to_string(),
            "status".to_string(),
        ])
    );
    let node = envelope_value["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| !node["properties"].as_object().unwrap().is_empty())
        .expect("contract node with semantic properties");
    let property = node["properties"].as_object().unwrap().values().next().unwrap();
    assert!(property.get("provenance").is_some());
    assert!(property.get("capability").is_some());
    let capability = node["capability"].as_object().unwrap();
    assert_eq!(
        capability.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "authorability".to_string(),
            "consistency".to_string(),
            "coverage".to_string(),
            "format".to_string(),
            "identity".to_string(),
            "resolution".to_string(),
            "sourceAccess".to_string(),
        ])
    );
    let relation = facts
        .iter()
        .find(|fact| fact["kind"] == "semanticRelation")
        .expect("complete semantic relation fact");
    for required in [
        "relationRef",
        "groupRef",
        "identityStrength",
        "kind",
        "role",
        "source",
        "target",
        "capability",
    ] {
        assert!(relation["value"].get(required).is_some());
    }
}

#[test]
fn fix_round6_support_states_are_lossless_and_removed_is_inactive() {
    let oracle = oracle();
    let expected = BTreeSet::from([
        "configurationReadOnly",
        "notSupported",
        "removedFromSupport",
        "supportedEditable",
        "supportedLocked",
    ]);
    let states = oracle
        .cases
        .iter()
        .flat_map(|case| case.facts.iter())
        .filter(|fact| fact["kind"] == "property" && fact["predicate"] == "support.state")
        .map(|fact| fact["value"]["value"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(states, expected);

    for case_id in ["supportNotSupported", "supportRemoved"] {
        let case = oracle_case(&oracle, case_id);
        assert!(case.facts.iter().any(|fact| {
            fact["predicate"] == "support.active" && fact["value"]["value"] == false
        }));
    }

    let currencies = read_tracked("contract/ContractCatalog.xml");
    let support = node(&currencies, SemanticObjectKind::Catalog, "ContractCatalog")
        .properties
        .get(&SemanticPropertyId::SUPPORT_STATE)
        .expect("support state");
    assert_eq!(support.value_type(), PropertyType::Enum);
    assert_eq!(
        serde_json::to_value(support.value().unwrap()).unwrap(),
        json!({"type": "enum", "value": "notSupported"})
    );
}

#[test]
fn fix_round7_template_type_never_rewrites_the_legacy_template_owner() {
    let contexts: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("enum-source-contexts.json")).unwrap(),
    )
    .unwrap();
    let context = contexts["contexts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|context| context["nativeProperty"] == "TemplateType")
        .expect("legacy TemplateType context");
    assert_eq!(
        context["objectKinds"],
        json!(["commonTemplate", "template"]),
        "TemplateType values must not create inferred owner kinds"
    );
    assert!(context["observedAliasEvidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|evidence| {
            evidence["nativeOwner"] == "Template"
                && evidence["objectKind"] == "template"
                && evidence["nativeValue"] == "SpreadsheetDocument"
        }));
    let raw = fs::read_to_string(
        oracle_root().join("enum-context-output/SpreadsheetDocumentTemplate.txt"),
    )
    .unwrap();
    assert!(raw
        .trim_start_matches('\u{feff}')
        .starts_with("=== Template: EnumContextSpreadsheetDocument ==="));

    let spreadsheet = read_tracked(
        "legacy-oracle/enum-context-inputs/EnumContextSpreadsheetDocument.xml",
    );
    let template = node(
        &spreadsheet,
        SemanticObjectKind::Template,
        "EnumContextSpreadsheetDocument",
    );
    assert_value(
        template,
        SemanticPropertyId::TEMPLATE_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::SPREADSHEET_DOCUMENT),
    );
    assert!(!spreadsheet.nodes.iter().any(|node| {
        node.object_ref.kind == SemanticObjectKind::SpreadsheetDocumentTemplate
    }));

    let binary = read_inline(
        "generic-template-binary",
        "GenericTemplate.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Template uuid="61000000-0000-0000-0000-000000000099"><Properties><Name>GenericTemplate</Name><TemplateType>BinaryData</TemplateType></Properties></Template></MetaDataObject>"#,
    );
    assert_value(
        node(&binary, SemanticObjectKind::Template, "GenericTemplate"),
        SemanticPropertyId::TEMPLATE_TYPE,
        PropertyValue::EnumSymbol(SemanticEnumValue::BINARY_DATA),
    );
}

#[test]
fn fix_round7_full_public_contract_specimen_covers_every_nested_shape() {
    let specimen_path = oracle_root().join("full-public-contract-specimen.json");
    let specimen: Value = serde_json::from_slice(
        &fs::read(&specimen_path).expect("independent full public contract specimen"),
    )
    .unwrap();
    assert_eq!(specimen["schemaVersion"], 1);
    assert_eq!(
        specimen["provenance"],
        "independently-hand-reviewed-closed-public-contract"
    );
    let schema = specimen["publicSchema"].as_object().unwrap();
    for nested in [
        "NavigationRelationPage",
        "SemanticActionDescriptor",
        "OperationBinding",
    ] {
        assert!(schema.contains_key(nested), "missing nested schema {nested}");
    }
    let facts = specimen["facts"].as_array().unwrap();
    let actual = full_public_contract_specimen_facts();
    validate_contract_public_schema(schema, &actual).unwrap();
    compare_fact_multisets(facts, &actual)
        .unwrap_or_else(|diff| panic!("full public contract specimen drifted:\n{diff}"));
    let envelope = &facts
        .iter()
        .find(|fact| fact["kind"] == "envelope")
        .expect("full specimen envelope")["value"];
    assert!(!envelope["relations"].as_array().unwrap().is_empty());
    assert!(!envelope["relations"][0]["items"][0]["facets"]
        .as_object()
        .unwrap()
        .is_empty());
    assert!(envelope["relations"][0]["nextCursor"]
        .as_str()
        .is_some_and(|cursor| !cursor.is_empty()));
    assert!(!envelope["nodes"][0]["semanticActions"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(envelope["nodes"][0]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|action| action["operationBinding"].is_object()));

    let envelope_index = actual
        .iter()
        .position(|fact| fact["kind"] == "envelope")
        .unwrap();
    for path in [
        &["relations", "0", "relation", "sourceId"][..],
        &["relations", "0", "relation", "groupKey"][..],
        &["relations", "0", "relation", "owner", "objectKey"][..],
        &["relations", "0", "relation", "role"][..],
        &["relations", "0", "relation", "kind"][..],
        &["relations", "0", "items", "0", "facets", "identity", "0"][..],
        &["relations", "0", "nextCursor"][..],
        &["nodes", "0", "semanticActions", "0", "action"][..],
        &["nodes", "0", "semanticActions", "0", "executionPolicy"][..],
        &["nodes", "0", "actions", "0", "blockingReasons", "0"][..],
        &["nodes", "0", "actions", "1", "operationBinding", "tool"][..],
        &["nodes", "0", "actions", "1", "operationBinding", "schemaVersion"][..],
    ] {
        let mut changed = actual.clone();
        mutate_json_path(
            &mut changed[envelope_index]["value"],
            path,
            json!("must-fail"),
        );
        assert_comparator_rejects(
            facts,
            &changed,
            &format!("changed full public field {}", path.join(".")),
        );
    }

    let mut missing_descriptor_field = actual.clone();
    missing_descriptor_field[envelope_index]["value"]["nodes"][0]["semanticActions"][0]
        .as_object_mut()
        .unwrap()
        .remove("executionPolicy");
    assert!(validate_contract_public_schema(schema, &missing_descriptor_field).is_err());

    let mut extra_binding_field = actual.clone();
    extra_binding_field[envelope_index]["value"]["nodes"][0]["actions"][1]
        ["operationBinding"]["futureField"] = json!("must-fail");
    assert!(validate_contract_public_schema(schema, &extra_binding_field).is_err());

    for field in ["relation", "items"] {
        let mut missing = actual.clone();
        missing[envelope_index]["value"]["relations"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(
            validate_contract_public_schema(schema, &missing).is_err(),
            "relation page accepted missing {field}"
        );
    }
    for (object_path, fields) in [
        (
            &["nodes", "0", "semanticActions", "0"][..],
            &["action", "executionPolicy"][..],
        ),
        (
            &["nodes", "0", "actions", "1", "operationBinding"][..],
            &["tool", "schemaVersion"][..],
        ),
    ] {
        for field in fields {
            let mut missing = actual.clone();
            let mut object = &mut missing[envelope_index]["value"];
            for segment in object_path {
                object = if let Ok(index) = segment.parse::<usize>() {
                    &mut object.as_array_mut().unwrap()[index]
                } else {
                    &mut object.as_object_mut().unwrap()[*segment]
                };
            }
            object.as_object_mut().unwrap().remove(*field);
            assert!(
                validate_contract_public_schema(schema, &missing).is_err(),
                "{} accepted missing {field}",
                object_path.join(".")
            );
        }
    }
    let mut removed_cursor = actual.clone();
    removed_cursor[envelope_index]["value"]["relations"][0]
        .as_object_mut()
        .unwrap()
        .remove("nextCursor");
    assert_comparator_rejects(
        facts,
        &removed_cursor,
        "relation page missing optional nonempty cursor specimen",
    );
}

#[test]
fn fix_round7_every_source_enum_alias_is_executed_by_legacy_and_adapter() {
    let executions: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("enum-alias-executions.json"))
            .expect("frozen legacy enum-alias executions"),
    )
    .unwrap();
    assert_eq!(executions["schemaVersion"], 1);
    let rows = executions["executions"].as_array().unwrap();
    let expected = oracle().enum_coverage;
    let actual = rows
        .iter()
        .map(|row| {
            json!({
                "nativeAlias": row["nativeAlias"],
                "nativeProperty": row["nativeProperty"],
                "objectKind": row["objectKind"],
                "semantic": row["semantic"],
                "semanticProperty": row["semanticProperty"],
            })
        })
        .collect::<Vec<_>>();
    compare_fact_multisets(&expected, &actual)
        .unwrap_or_else(|diff| panic!("enum alias execution inventory drifted:\n{diff}"));
    assert!(rows.iter().all(|row| {
        row["inputXml"].as_str().is_some_and(|value| !value.is_empty())
            && row["rawLegacyOutput"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
            && row["rawOutputSha256"]
                .as_str()
                .is_some_and(|value| value.len() == 64)
    }));
    for (index, row) in rows.iter().enumerate() {
        let envelope = read_inline(
            &format!("enum-alias-{index:04}"),
            row["inputFileName"].as_str().unwrap(),
            row["inputXml"].as_str().unwrap(),
        );
        let owner_kind =
            SemanticObjectKind::parse(row["objectKind"].as_str().unwrap()).unwrap();
        let owner = node(&envelope, owner_kind, row["ownerName"].as_str().unwrap());
        let property =
            SemanticPropertyId::parse(row["semanticProperty"].as_str().unwrap()).unwrap();
        let semantic = SemanticEnumValue::parse(row["semantic"].as_str().unwrap()).unwrap();
        assert_value(
            owner,
            property,
            PropertyValue::EnumSymbol(semantic),
        );
    }
}

#[test]
fn fix_round7_multitarget_executes_every_supported_rights_prefix() {
    let target_crosswalk: Value = serde_json::from_slice(
        &fs::read(oracle_root().join("rights-target-crosswalk.json")).unwrap(),
    )
    .unwrap();
    let expected = target_crosswalk["prefixes"]
        .as_object()
        .unwrap()
        .values()
        .map(|kind| kind.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 45);

    let legacy = oracle();
    let case = oracle_case(&legacy, "rightsMultiTarget");
    let legacy_targets = case
        .facts
        .iter()
        .filter(|fact| fact["kind"] == "relation" && fact["predicate"] == "accessTarget")
        .map(|fact| fact["value"]["targetKind"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(legacy_targets, expected);
    assert_eq!(
        case.facts
            .iter()
            .filter(|fact| {
                fact["kind"] == "relation" && fact["predicate"] == "accessTarget"
            })
            .count(),
        45
    );

    let envelope = read_oracle_case(case);
    let runtime_relations = envelope
        .relation_index
        .iter()
        .filter(|relation| relation.role.as_str() == "accessTarget")
        .collect::<Vec<_>>();
    let runtime_targets = envelope
        .relation_index
        .iter()
        .filter(|relation| relation.role.as_str() == "accessTarget")
        .map(|relation| relation.target.kind.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(runtime_targets, expected);
    assert_eq!(runtime_relations.len(), 45);
    for (prefix, kind) in target_crosswalk["prefixes"].as_object().unwrap() {
        let target_name = format!("{prefix}Target");
        let relation = runtime_relations
            .iter()
            .find(|relation| {
                relation.target.kind.as_str() == kind.as_str().unwrap()
                    && relation.target.display_name == target_name
            })
            .unwrap_or_else(|| panic!("missing independently decoded target {prefix}"));
        let permission = envelope
            .nodes
            .iter()
            .find(|node| node.object_ref == relation.source)
            .expect("right target relation source permission");
        assert!(permission
            .properties
            .contains_key(&SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS));
    }
    assert_eq!(
        envelope
            .nodes
            .iter()
            .filter(|node| node.object_ref.kind == SemanticObjectKind::AccessRestrictionTemplate)
            .count(),
        45
    );
    assert_eq!(
        envelope
            .nodes
            .iter()
            .filter(|node| node.object_ref.kind == SemanticObjectKind::AccessPermission)
            .filter(|node| node
                .properties
                .contains_key(&SemanticPropertyId::ACCESS_RESTRICTION_CONDITIONS))
            .count(),
        45
    );
}

#[test]
fn fix_round7_adapter_intentionally_exposes_no_projected_pages_or_bindings() {
    let envelope = read_tracked("contract/ContractCatalog.xml");
    assert!(!envelope.relation_index.is_empty());
    assert!(envelope.relations.is_empty());
    assert!(envelope
        .nodes
        .iter()
        .all(|node| node.semantic_actions.is_empty()));
    assert!(envelope.nodes.iter().flat_map(|node| &node.actions).all(|action| {
        action.operation_binding.is_none()
    }));

    let target = tracked_root().join("contract/ContractCatalog.xml");
    let source_root = target.parent().unwrap();
    let source = SourceContext::new(
        SourceLocation::new(repo_root(), source_root.to_path_buf(), target),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("contract fixture must be captured")
    };
    let error = registration
        .read
        .read(&FormatReadRequest {
            captured: captured.clone(),
            query: NavigationQuery {
                target: NavigationTarget::CapturedTarget(
                    captured.binding().target_identity.clone(),
                ),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: vec![
                        RelationSelection::new(
                            SemanticRelationId::ATTRIBUTES,
                            Some(1),
                        )
                        .unwrap(),
                    ],
                },
            },
        })
        .unwrap_err();
    assert_eq!(error.kind, SourceAdapterErrorKind::CapabilityBlocked);
}

fn mutate_json_path(value: &mut Value, path: &[&str], replacement: Value) {
    let Some((head, tail)) = path.split_first() else {
        *value = replacement;
        return;
    };
    let next = match value {
        Value::Array(values) => &mut values[head.parse::<usize>().unwrap()],
        Value::Object(values) => values
            .get_mut(*head)
            .unwrap_or_else(|| panic!("mutation path is absent: {}", path.join("."))),
        _ => panic!("mutation path is not traversable: {}", path.join(".")),
    };
    mutate_json_path(next, tail, replacement);
}

fn validate_contract_public_schema(
    schema: &serde_json::Map<String, Value>,
    facts: &[Value],
) -> Result<(), String> {
    fn validate_keys(
        schema: &serde_json::Map<String, Value>,
        name: &str,
        value: &Value,
    ) -> Result<(), String> {
        let object = value
            .as_object()
            .ok_or_else(|| format!("{name} is not an object"))?;
        let definition = schema
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{name} has no schema definition"))?;
        let required = definition["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let optional = definition["optional"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
        if !required.is_subset(&actual) {
            return Err(format!(
                "{name} is missing fields: {:?}",
                required.difference(&actual).collect::<Vec<_>>()
            ));
        }
        let allowed = required.union(&optional).copied().collect::<BTreeSet<_>>();
        if !actual.is_subset(&allowed) {
            return Err(format!(
                "{name} has unregistered fields: {:?}",
                actual.difference(&allowed).collect::<Vec<_>>()
            ));
        }
        Ok(())
    }

    fn validate_object_ref(
        schema: &serde_json::Map<String, Value>,
        value: &Value,
    ) -> Result<(), String> {
        validate_keys(schema, "ObjectRef", value)
    }

    fn validate_capability(
        schema: &serde_json::Map<String, Value>,
        value: &Value,
    ) -> Result<(), String> {
        validate_keys(schema, "CapabilityVector", value)
    }

    fn validate_node(
        schema: &serde_json::Map<String, Value>,
        node: &Value,
    ) -> Result<(), String> {
        validate_keys(schema, "NavigationNode", node)?;
        validate_object_ref(schema, &node["objectRef"])?;
        validate_object_ref(schema, &node["reference"])?;
        validate_keys(schema, "CapabilityState", &node["capabilityState"])?;
        validate_capability(schema, &node["capability"])?;
        for property in node["properties"].as_object().unwrap().values() {
            validate_keys(schema, "SemanticProperty", property)?;
        }
        for descriptor in node["semanticActions"].as_array().unwrap() {
            validate_keys(schema, "SemanticActionDescriptor", descriptor)?;
        }
        for action in node["actions"].as_array().unwrap() {
            validate_keys(schema, "SemanticAction", action)?;
            if !action["target"].is_null() {
                validate_object_ref(schema, &action["target"])?;
            }
            if !action["owningRelation"].is_null() {
                validate_keys(schema, "RelationRef", &action["owningRelation"])?;
            }
            if !action["operationBinding"].is_null() {
                validate_keys(schema, "OperationBinding", &action["operationBinding"])?;
            }
        }
        Ok(())
    }

    for fact in facts {
        match fact["kind"].as_str() {
            Some("envelope") => {
                let envelope = &fact["value"];
                validate_keys(schema, "NavigationEnvelope", envelope)?;
                if !envelope["snapshot"].is_null() {
                    validate_keys(schema, "SourceSnapshot", &envelope["snapshot"])?;
                }
                if !envelope["root"].is_null() {
                    validate_object_ref(schema, &envelope["root"])?;
                }
                for node in envelope["nodes"].as_array().unwrap() {
                    validate_node(schema, node)?;
                }
                for page in envelope["relations"].as_array().unwrap() {
                    validate_keys(schema, "NavigationRelationPage", page)?;
                    validate_keys(schema, "RelationGroupRef", &page["relation"])?;
                    validate_object_ref(schema, &page["relation"]["owner"])?;
                    for item in page["items"].as_array().unwrap() {
                        validate_node(schema, item)?;
                    }
                    if let Some(cursor) = page.get("nextCursor") {
                        if !cursor
                            .as_str()
                            .is_some_and(|value| !value.is_empty())
                        {
                            return Err(
                                "NavigationRelationPage nextCursor is not opaque text"
                                    .to_string(),
                            );
                        }
                    }
                }
                for diagnostic in envelope["diagnostics"].as_array().unwrap() {
                    validate_keys(schema, "SourceAdapterDiagnostic", diagnostic)?;
                }
            }
            Some("semanticRelation") => {
                let relation = &fact["value"];
                validate_keys(schema, "SemanticRelation", relation)?;
                validate_keys(schema, "RelationRef", &relation["relationRef"])?;
                validate_keys(schema, "RelationGroupRef", &relation["groupRef"])?;
                validate_object_ref(schema, &relation["groupRef"]["owner"])?;
                validate_object_ref(schema, &relation["source"])?;
                validate_object_ref(schema, &relation["target"])?;
                validate_capability(schema, &relation["capability"])?;
            }
            Some(other) => return Err(format!("unknown contract fact kind {other}")),
            None => return Err("contract fact has no kind".to_string()),
        }
    }
    Ok(())
}

fn full_public_contract_specimen_facts() -> Vec<Value> {
    let source_id = SourceId::new("source:full-public-contract").unwrap();
    let revision = SourceRevision::new("revision:full-public-contract").unwrap();
    let owner_ref = ObjectRef::new(
        source_id.clone(),
        ObjectKey::new("uuid:11111111-1111-1111-1111-111111111111").unwrap(),
        IdentityStrength::Persistent,
        SemanticObjectKind::Document,
        "FullContractOwner",
    );
    let child_ref = ObjectRef::new(
        source_id.clone(),
        ObjectKey::new("uuid:22222222-2222-2222-2222-222222222222").unwrap(),
        IdentityStrength::Derived,
        SemanticObjectKind::Attribute,
        "FullContractChild",
    );
    let relation_ref = RelationRef {
        source_id: source_id.clone(),
        relation_key: RelationKey::new("relation:full-public-contract").unwrap(),
        kind: RelationKind::Contains,
    };
    let group_ref = RelationGroupRef {
        source_id: source_id.clone(),
        group_key: RelationKey::new("group:full-public-contract").unwrap(),
        owner: owner_ref.clone(),
        role: SemanticRelationId::ATTRIBUTES,
        kind: RelationKind::Contains,
    };
    let capability = CapabilityVector {
        resolution: ResolutionState::Resolved,
        identity: IdentityStrength::Persistent,
        consistency: SnapshotConsistency::Consistent,
        coverage: CoverageState::Complete,
        format: FormatCompatibility::Compatible,
        source_access: SourceAccess::ReadWrite,
        authorability: Authorability::Authorable,
    };
    let descriptor_specs = [
        (SemanticActionKind::Inspect, ActionExecutionPolicy::ReadOnly),
        (
            SemanticActionKind::EditProperties,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::Clone,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::Remove,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddAttribute,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddTabularSection,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddForm,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddMxl,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddCommand,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddFormAttribute,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddFormCommand,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::AddFormElement,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::Move,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::BindData,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::RebindData,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::UnbindData,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::BindCommand,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::RebindCommand,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::UnbindCommand,
            ActionExecutionPolicy::AtomicRelationMutation,
        ),
        (
            SemanticActionKind::CreateHandler,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
        (
            SemanticActionKind::EditMxl,
            ActionExecutionPolicy::AtomicNodeMutation,
        ),
    ];
    let semantic_actions = descriptor_specs
        .into_iter()
        .map(|(action, execution_policy)| SemanticActionDescriptor {
            action,
            execution_policy,
        })
        .collect::<Vec<_>>();
    let actions = vec![
        SemanticAction {
            kind: SemanticActionKind::Inspect,
            target: Some(owner_ref.clone()),
            owning_relation: None,
            availability: ActionAvailability::Modeled,
            blocking_reasons: vec![CapabilityBlockReason::ResolutionUnresolved],
            operation_binding: None,
            atomicity: Atomicity::ReadOnly,
        },
        SemanticAction {
            kind: SemanticActionKind::EditProperties,
            target: Some(owner_ref.clone()),
            owning_relation: None,
            availability: ActionAvailability::Executable,
            blocking_reasons: Vec::new(),
            operation_binding: Some(OperationBinding {
                tool: "unica.meta.edit".to_string(),
                schema_version: "1".to_string(),
            }),
            atomicity: Atomicity::SingleFileAtomicReplace,
        },
        SemanticAction {
            kind: SemanticActionKind::Clone,
            target: None,
            owning_relation: Some(relation_ref.clone()),
            availability: ActionAvailability::Blocked,
            blocking_reasons: vec![
                CapabilityBlockReason::ResolutionUnresolved,
                CapabilityBlockReason::IdentitySnapshotOnly,
                CapabilityBlockReason::SnapshotInconsistent,
                CapabilityBlockReason::CoverageIncomplete,
                CapabilityBlockReason::FormatIncompatible,
                CapabilityBlockReason::SourceReadOnly,
                CapabilityBlockReason::NotAuthorable,
                CapabilityBlockReason::OwningRelationMissing,
                CapabilityBlockReason::OperationBindingInvalid,
            ],
            operation_binding: None,
            atomicity: Atomicity::AggregateSwapWithRecovery,
        },
        SemanticAction {
            kind: SemanticActionKind::Move,
            target: Some(child_ref.clone()),
            owning_relation: Some(relation_ref.clone()),
            availability: ActionAvailability::Modeled,
            blocking_reasons: Vec::new(),
            operation_binding: None,
            atomicity: Atomicity::BackendTransaction,
        },
    ];
    let mut owner_properties = BTreeMap::new();
    owner_properties.insert(
        SemanticPropertyId::METADATA_NAME,
        SemanticProperty::explicit(
            SemanticPropertyId::METADATA_NAME,
            PropertyValue::String("FullContractOwner".to_string()),
        )
        .unwrap()
        .with_capability(PropertyCapability::Authorable)
        .unwrap(),
    );
    owner_properties.insert(
        SemanticPropertyId::METADATA_COMMENT,
        SemanticProperty::defaulted(
            SemanticPropertyId::METADATA_COMMENT,
            PropertyValue::String("default comment".to_string()),
        )
        .unwrap()
        .with_capability(PropertyCapability::ReadOnly)
        .unwrap(),
    );
    owner_properties.insert(
        SemanticPropertyId::METADATA_DESCRIPTION,
        SemanticProperty::inherited(
            SemanticPropertyId::METADATA_DESCRIPTION,
            PropertyValue::String("inherited description".to_string()),
        )
        .unwrap()
        .with_capability(PropertyCapability::ReadOnly)
        .unwrap(),
    );
    owner_properties.insert(
        SemanticPropertyId::SUPPORT_AUTHORABILITY,
        SemanticProperty::computed(
            SemanticPropertyId::SUPPORT_AUTHORABILITY,
            PropertyValue::String("authorable".to_string()),
        )
        .unwrap()
        .with_capability(PropertyCapability::ReadOnly)
        .unwrap(),
    );
    owner_properties.insert(
        SemanticPropertyId::METADATA_CODE,
        SemanticProperty::absent(SemanticPropertyId::METADATA_CODE),
    );
    owner_properties.insert(
        SemanticPropertyId::METADATA_SYNONYM,
        SemanticProperty::unresolved(SemanticPropertyId::METADATA_SYNONYM),
    );
    let owner_facets = SemanticFacets::for_available(
        owner_properties.keys().copied(),
        [SemanticRelationId::ATTRIBUTES],
    );
    let owner = NavigationNode {
        object_ref: owner_ref.clone(),
        reference: owner_ref.clone(),
        capability_state: CapabilityState::resolved_authorable(),
        capability: capability.clone(),
        properties: owner_properties,
        facets: owner_facets,
        action_profile: ActionProfile::DocumentMetadataObject,
        semantic_actions,
        actions,
        facet_visibility: NavigationFacetVisibility::Full,
    };
    let mut child_properties = BTreeMap::new();
    child_properties.insert(
        SemanticPropertyId::METADATA_NAME,
        SemanticProperty::explicit(
            SemanticPropertyId::METADATA_NAME,
            PropertyValue::String("FullContractChild".to_string()),
        )
        .unwrap()
        .with_capability(PropertyCapability::ReadOnly)
        .unwrap(),
    );
    let child = NavigationNode {
        object_ref: child_ref.clone(),
        reference: child_ref.clone(),
        capability_state: CapabilityState::new(
            ResolutionState::Unresolved,
            Authorability::DerivedReadOnly,
        ),
        capability: CapabilityVector {
            resolution: ResolutionState::Unresolved,
            identity: IdentityStrength::Derived,
            consistency: SnapshotConsistency::Changed,
            coverage: CoverageState::Partial,
            format: FormatCompatibility::Unknown,
            source_access: SourceAccess::ReadOnly,
            authorability: Authorability::DerivedReadOnly,
        },
        facets: SemanticFacets::for_available(
            child_properties.keys().copied(),
            std::iter::empty(),
        ),
        properties: child_properties,
        action_profile: ActionProfile::UnmodeledChild,
        semantic_actions: vec![SemanticActionDescriptor {
            action: SemanticActionKind::Inspect,
            execution_policy: ActionExecutionPolicy::ReadOnly,
        }],
        actions: vec![SemanticAction {
            kind: SemanticActionKind::Inspect,
            target: Some(child_ref.clone()),
            owning_relation: Some(relation_ref.clone()),
            availability: ActionAvailability::Modeled,
            blocking_reasons: Vec::new(),
            operation_binding: None,
            atomicity: Atomicity::ReadOnly,
        }],
        facet_visibility: NavigationFacetVisibility::Full,
    };
    let cursor = NavigationCursor::issue(
        b"full-public-contract-cursor-secret",
        source_id.clone(),
        revision.clone(),
        owner_ref.object_key.clone(),
        group_ref.clone(),
        NavigationSelection {
            properties: PropertySelection::All,
            facets: FacetSelection::Full,
            relations: vec![RelationSelection {
                kind: RelationKind::Contains,
                role: SemanticRelationId::ATTRIBUTES,
                page_size: 1,
            }],
        },
        1,
    )
    .unwrap();
    let relation = SemanticRelation {
        relation_ref,
        group_ref: group_ref.clone(),
        identity_strength: IdentityStrength::Persistent,
        kind: RelationKind::Contains,
        role: SemanticRelationId::ATTRIBUTES,
        source: owner_ref.clone(),
        target: child_ref,
        capability,
    };
    let envelope = NavigationEnvelope {
        schema_version: "1".to_string(),
        status: NavigationStatus::Available,
        snapshot: Some(SourceSnapshot {
            source_id,
            revision,
            consistency: SnapshotConsistency::Consistent,
            adapter_id: "public-contract-specimen".to_string(),
        }),
        root: Some(owner_ref),
        nodes: vec![owner],
        relations: vec![NavigationRelationPage {
            relation: group_ref,
            items: vec![child],
            next_cursor: Some(cursor),
        }],
        diagnostics: vec![unica_format_core::navigation::SourceAdapterDiagnostic {
            code: "specimenDiagnostic".to_string(),
            message: "full public contract specimen".to_string(),
            details: Some(json!({"category": "contract", "ordinal": 1})),
        }],
        relation_index: std::sync::Arc::new(vec![relation.clone()]),
    };
    let mut envelope_value = serde_json::to_value(envelope).unwrap();
    envelope_value["relations"][0]["nextCursor"] = json!("opaque:cursor");
    let mut facts = vec![
        json!({"case": "fullPublicContract", "kind": "envelope", "value": envelope_value}),
        json!({
            "case": "fullPublicContract",
            "kind": "semanticRelation",
            "value": serde_json::to_value(relation).unwrap(),
        }),
    ];
    facts.sort_by_key(canonical_json);
    facts
}

fn adapter_only_contract_facts(case_id: &str, envelope: &NavigationEnvelope) -> Vec<Value> {
    let identities = contract_identities(envelope);
    let source_id = format!("source:{case_id}");
    let mut relation_ids = BTreeMap::new();
    let mut group_ids = BTreeMap::new();
    for relation in envelope.relation_index.iter() {
        let source = &identities[relation.source.object_key.as_str()];
        let target = &identities[relation.target.object_key.as_str()];
        let kind = serde_json::to_value(relation.kind)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        relation_ids.insert(
            relation.relation_ref.relation_key.as_str().to_string(),
            format!("relation:{source}:{}:{target}:{kind}", relation.role.as_str()),
        );
        group_ids.insert(
            relation.group_ref.group_key.as_str().to_string(),
            format!("group:{source}:{}:{kind}", relation.role.as_str()),
        );
    }
    let mut envelope_value = serde_json::to_value(envelope).unwrap();
    for node in envelope_value["nodes"]
        .as_array_mut()
        .expect("serialized navigation nodes")
    {
        for members in node["facets"]
            .as_object_mut()
            .expect("serialized semantic facets")
            .values_mut()
        {
            members
                .as_array_mut()
                .expect("serialized facet members")
                .sort_by_key(canonical_json);
        }
    }
    normalize_contract_value(
        &mut envelope_value,
        case_id,
        &source_id,
        &identities,
        &relation_ids,
        &group_ids,
    );
    envelope_value["nodes"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(canonical_json);
    envelope_value["diagnostics"]
        .as_array_mut()
        .unwrap()
        .sort_by_key(canonical_json);
    let mut facts = vec![json!({
        "case": case_id,
        "kind": "envelope",
        "value": envelope_value,
    })];
    for relation in envelope.relation_index.iter() {
        let mut value = serde_json::to_value(relation).unwrap();
        normalize_contract_value(
            &mut value,
            case_id,
            &source_id,
            &identities,
            &relation_ids,
            &group_ids,
        );
        facts.push(json!({
            "case": case_id,
            "kind": "semanticRelation",
            "value": value,
        }));
    }
    facts.sort_by_key(canonical_json);
    facts
}

fn normalize_contract_value(
    value: &mut Value,
    case_id: &str,
    source_id: &str,
    identities: &BTreeMap<String, String>,
    relation_ids: &BTreeMap<String, String>,
    group_ids: &BTreeMap<String, String>,
) {
    match value {
        Value::Array(values) => {
            for value in values {
                normalize_contract_value(
                    value,
                    case_id,
                    source_id,
                    identities,
                    relation_ids,
                    group_ids,
                );
            }
        }
        Value::Object(values) => {
            if let Some(Value::String(value)) = values.get_mut("sourceId") {
                *value = source_id.to_string();
            }
            if let Some(Value::String(value)) = values.get_mut("revision") {
                *value = format!("revision:{case_id}");
            }
            if let Some(Value::String(value)) = values.get_mut("objectKey") {
                if let Some(identity) = identities.get(value) {
                    *value = identity.clone();
                }
            }
            if let Some(Value::String(value)) = values.get_mut("relationKey") {
                if let Some(identity) = relation_ids.get(value) {
                    *value = identity.clone();
                }
            }
            if let Some(Value::String(value)) = values.get_mut("groupKey") {
                if let Some(identity) = group_ids.get(value) {
                    *value = identity.clone();
                }
            }
            for value in values.values_mut() {
                normalize_contract_value(
                    value,
                    case_id,
                    source_id,
                    identities,
                    relation_ids,
                    group_ids,
                );
            }
        }
        _ => {}
    }
}

fn contract_identities(envelope: &NavigationEnvelope) -> BTreeMap<String, String> {
    let mut identities = BTreeMap::new();
    let mut counts = BTreeMap::<(String, String, &'static str), usize>::new();
    let mut register = |
        object_ref: &ObjectRef,
        category: &'static str,
        identities: &mut BTreeMap<String, String>,
    | {
        let native_key = object_ref.object_key.as_str().to_string();
        if identities.contains_key(&native_key) {
            return;
        }
        let stable = if native_key.starts_with("uuid:") {
            native_key.clone()
        } else {
            let key = (
                object_ref.kind.as_str().to_string(),
                object_ref.display_name.clone(),
                category,
            );
            let count = counts.entry(key).or_default();
            *count += 1;
            format!(
                "{category}:{}:{}#{}",
                object_ref.kind.as_str(),
                object_ref.display_name,
                count
            )
        };
        identities.insert(native_key, stable);
    };
    for node in &envelope.nodes {
        register(&node.object_ref, "derived", &mut identities);
    }
    for relation in envelope.relation_index.iter() {
        register(&relation.source, "derived", &mut identities);
        register(&relation.target, "external", &mut identities);
    }
    identities
}
