use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus, NavigationTarget, PropertySelection},
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{SourceContext, SourceFamily, SourceLocation},
    value::PropertyValue,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[test]
fn unknown_native_root_is_readable_unknown_and_partial() {
    let envelope = read_tracked("unknown_root/Mystery.xml");
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let root = envelope.nodes.iter().find(|node| node.object_ref.kind == SemanticObjectKind::Unknown && node.object_ref.display_name == "Mystery").expect("unknown root node");
    assert!(root.properties.contains_key(&SemanticPropertyId::UNKNOWN_FACTS));
    assert_neutral(&envelope);
}

#[test]
fn unknown_child_is_retained_through_the_neutral_relation() {
    let envelope = read_tracked("unknowns/UnknownCases.xml");
    assert!(envelope.relation_index.iter().any(|relation| relation.role == SemanticRelationId::UNKNOWN
        && relation.target.kind == SemanticObjectKind::Unknown && relation.target.display_name == "NestedUnknown"));
    assert_neutral(&envelope);
}

#[test]
fn unknown_children_under_no_vocabulary_owners_preserve_occurrence_and_position() {
    let expected_native_profiles = [
        "AccountingFlag",
        "AddressingAttribute",
        "Attribute",
        "Column",
        "Command",
        "Dimension",
        "EnumValue",
        "ExtDimensionAccountingFlag",
        "Form",
        "Method",
        "Parameter",
        "Resource",
        "Template",
    ];
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("src/versions/v2_20/coverage.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let actual_native_profiles = manifest["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|profile| {
            profile["source"] == "native" && profile["childVocabulary"] == "none"
        })
        .map(|profile| profile["nativeClass"].as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_native_profiles,
        expected_native_profiles.into_iter().collect(),
        "the independent no-vocabulary owner inventory drifted"
    );

    for (owner_class, owner_name, owner_kind) in [
        (
            "AccountingFlag",
            "AccountingFlagOwner",
            SemanticObjectKind::Unknown,
        ),
        (
            "AddressingAttribute",
            "AddressingAttributeOwner",
            SemanticObjectKind::Unknown,
        ),
        ("Attribute", "AttributeOwner", SemanticObjectKind::Attribute),
        ("Column", "ColumnOwner", SemanticObjectKind::Attribute),
        ("Command", "CommandOwner", SemanticObjectKind::Command),
        ("Dimension", "DimensionOwner", SemanticObjectKind::Dimension),
        (
            "EnumValue",
            "EnumValueOwner",
            SemanticObjectKind::EnumerationValue,
        ),
        (
            "ExtDimensionAccountingFlag",
            "ExtDimensionAccountingFlagOwner",
            SemanticObjectKind::Unknown,
        ),
        ("Form", "FormOwner", SemanticObjectKind::Form),
        (
            "Method",
            "MethodOwner",
            SemanticObjectKind::HttpServiceMethod,
        ),
        (
            "Parameter",
            "ParameterOwner",
            SemanticObjectKind::WebServiceParameter,
        ),
        ("Resource", "ResourceOwner", SemanticObjectKind::Resource),
        ("Template", "TemplateOwner", SemanticObjectKind::Template),
    ] {
        let xml = format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{owner_class} uuid="72000000-0000-0000-0000-000000000001"><Properties><Name>{owner_name}</Name></Properties><ChildObjects><FutureChild><Properties><Name>Repeated</Name><Payload>first-readable-value</Payload></Properties></FutureChild><FutureChild><Properties><Name>Repeated</Name><Payload>second-readable-value</Payload></Properties></FutureChild></ChildObjects></{owner_class}></MetaDataObject>"#
        );
        let envelope = read_inline(
            &format!("unknown-owner-{owner_name}"),
            &format!("{owner_name}.xml"),
            &xml,
        );
        assert_eq!(envelope.status, NavigationStatus::Partial, "{owner_name}");
        assert!(envelope.nodes.iter().any(|node| {
            node.object_ref.kind == owner_kind && node.object_ref.display_name == owner_name
        }));
        let unknown = envelope
            .nodes
            .iter()
            .filter(|node| {
                node.object_ref.kind == SemanticObjectKind::Unknown
                    && node.object_ref.display_name == "Repeated"
            })
            .collect::<Vec<_>>();
        assert_eq!(unknown.len(), 2, "{owner_name} must retain both occurrences");
        assert_ne!(unknown[0].object_ref.object_key, unknown[1].object_ref.object_key);
        let ordinals = unknown
            .iter()
            .map(|node| {
                let value = node.properties[&SemanticPropertyId::UNKNOWN_FACTS]
                    .value()
                    .expect("unknown occurrence evidence");
                let json = serde_json::to_string(value).unwrap();
                assert!(!json.contains("FutureChild"));
                json
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(ordinals.len(), 2, "neutral occurrence evidence must distinguish duplicates");
        assert!(ordinals.iter().any(|value| value.contains("first-readable-value")));
        assert!(ordinals.iter().any(|value| value.contains("second-readable-value")));
        assert_neutral(&envelope);
    }
}

#[test]
fn unknown_property_keeps_its_readable_value_without_native_label() {
    let envelope = read_tracked("unknowns/UnknownCases.xml");
    let document = envelope.nodes.iter().find(|node| node.object_ref.kind == SemanticObjectKind::Document).unwrap();
    let value = document.properties[&SemanticPropertyId::UNKNOWN_FACTS].value().expect("unknown facts");
    let text = serde_json::to_string(value).unwrap();
    assert!(text.contains("property-readable-value"));
    assert!(!text.contains("NativeOnlyFact"));
}

#[test]
fn unknown_relation_target_is_retained_as_unknown_reference() {
    let envelope = read_tracked("unknowns/UnknownCases.xml");
    assert!(envelope.relation_index.iter().any(|relation| relation.role == SemanticRelationId::BASED_ON
        && relation.kind == unica_format_core::navigation::RelationKind::References
        && relation.target.kind == SemanticObjectKind::Unknown && relation.target.display_name == "Target"));
}

#[test]
fn unknown_type_variant_remains_in_the_type_set_and_marks_partial() {
    let envelope = read_tracked("unknowns/UnknownCases.xml");
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let attribute = envelope.nodes.iter().find(|node| node.object_ref.display_name == "MysteryType").unwrap();
    let PropertyValue::TypeSet(types) = attribute.properties[&SemanticPropertyId::FIELD_TYPE].value().expect("readable type set") else { panic!("type set"); };
    let json = serde_json::to_string(types).unwrap();
    assert_eq!(json.matches("\"kind\":\"unknown\"").count(), 2);
    assert!(json.contains("\"ordinal\":1"));
    assert!(json.contains("\"ordinal\":2"));
    assert!(!json.contains("FutureRecord"));
    assert!(!json.contains("AnotherFuture"));
}

#[test]
fn unknown_design_time_reference_retains_neutral_readable_evidence() {
    let envelope = read_inline(
        "unknown-design-time-reference",
        "UnknownReference.xml",
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Catalog uuid="73000000-0000-0000-0000-000000000001"><Properties><Name>UnknownReference</Name></Properties><ChildObjects><Attribute uuid="73000000-0000-0000-0000-000000000002"><Properties><Name>FirstReference</Name><FillValue xsi:type="xr:DesignTimeRef">Catalog.FirstTarget.FutureRef</FillValue></Properties></Attribute><Attribute uuid="73000000-0000-0000-0000-000000000003"><Properties><Name>SecondReference</Name><FillValue xsi:type="xr:DesignTimeRef">Catalog.SecondTarget.AnotherRef</FillValue></Properties></Attribute></ChildObjects></Catalog></MetaDataObject>"#,
    );
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let first = node_by_name(&envelope, "FirstReference");
    let second = node_by_name(&envelope, "SecondReference");
    let first = serde_json::to_string(
        first.properties[&SemanticPropertyId::FIELD_FILL_VALUE]
            .value()
            .expect("first readable unknown reference"),
    )
    .unwrap();
    let second = serde_json::to_string(
        second.properties[&SemanticPropertyId::FIELD_FILL_VALUE]
            .value()
            .expect("second readable unknown reference"),
    )
    .unwrap();
    assert_ne!(first, second);
    assert!(first.contains("FirstTarget"));
    assert!(second.contains("SecondTarget"));
    for native in ["Catalog", "DesignTimeRef", "FutureRef", "AnotherRef", "xr:"] {
        assert!(!first.contains(native));
        assert!(!second.contains(native));
    }
    assert_neutral(&envelope);
}

#[test]
fn unknown_backing_file_is_explicit_opaque_fact_not_silent_success() {
    let envelope = read_tracked("unknowns/UnknownCases.xml");
    assert_eq!(envelope.status, NavigationStatus::Partial);
    let document = envelope.nodes.iter().find(|node| node.object_ref.kind == SemanticObjectKind::Document).unwrap();
    let text = serde_json::to_string(document.properties[&SemanticPropertyId::UNKNOWN_FACTS].value().unwrap()).unwrap();
    assert!(text.contains("backing"));
    assert!(!text.contains("Future.bin"));
}

fn assert_neutral(envelope: &NavigationEnvelope) {
    let diagnostics = serde_json::to_string(&envelope.diagnostics).unwrap();
    for forbidden in ["FutureObject", "FutureChild", "FutureReference", "FutureRecord", "NativeOnlyFact", "Future.bin", "MetaDataObject", "http://"] {
        assert!(!diagnostics.contains(forbidden), "diagnostic leaked native vocabulary: {forbidden}");
    }
}

fn read_tracked(relative: &str) -> NavigationEnvelope {
    let target = tracked_root().join(relative);
    read_path(target.parent().unwrap(), &target)
}

fn read_path(source_root: &Path, target: &Path) -> NavigationEnvelope {
    let source = SourceContext::new(SourceLocation::new(repo_root(), source_root.to_path_buf(), target.to_path_buf()), Some("main".to_string()), SourceFamily::PlatformXml, None);
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).expect("unknown readable XML must probe") else { panic!("fixture must be captured"); };
    registration.read.read(&FormatReadRequest { captured: captured.clone(), query: NavigationQuery { target: NavigationTarget::CapturedTarget(captured.binding().target_identity.clone()), select: NavigationSelection { properties: PropertySelection::All, facets: FacetSelection::Full, relations: Vec::new() } } }).expect("unknown readable XML must project")
}

fn read_inline(label: &str, file_name: &str, xml: &str) -> NavigationEnvelope {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-xml-task5-fix2-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let target = root.join(file_name);
    fs::write(&target, xml).unwrap();
    let envelope = read_path(&root, &target);
    fs::remove_dir_all(root).unwrap();
    envelope
}

fn node_by_name<'a>(envelope: &'a NavigationEnvelope, name: &str) -> &'a unica_format_core::navigation::NavigationNode {
    envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.display_name == name)
        .unwrap_or_else(|| panic!("missing node {name}"))
}

fn tracked_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_20") }
fn repo_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
