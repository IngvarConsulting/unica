use std::path::{Path, PathBuf};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{FacetSelection, NavigationEnvelope, NavigationQuery, NavigationSelection, NavigationStatus, NavigationTarget, PropertySelection},
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::{SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{SourceContext, SourceFamily, SourceLocation},
    value::PropertyValue,
};

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
    assert!(serde_json::to_string(types).unwrap().contains("\"kind\":\"unknown\""));
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

fn tracked_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v2_20") }
fn repo_root() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap() }
