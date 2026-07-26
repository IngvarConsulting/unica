use std::{
    collections::BTreeSet,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use unica_format_core::{
    navigation::{
        FacetSelection, IdentityStrength, NavigationCursor, NavigationQuery, NavigationSelection,
        NavigationStatus, NavigationTarget, NodeKind, ObjectKey, ObjectRef, PropertySelection,
        RelationKey, RelationKind, RelationRole,
    },
    ports::{
        CaptureResult, FormatReadRequest, OwnerResolutionMode, OwnerResolutionRequest, ProbeResult,
    },
    source::{
        ConfiguredSourceSetKind, SourceAdapterErrorKind, SourceContext, SourceFamily, SourceId,
        SourceLocation, SourceRevision,
    },
};

use crate::PlatformXmlAdapterFactory;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[test]
fn public_registration_reads_platform_xml_2_20_through_core_ports() {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-xml-certification-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ));
    let source_root = root.join("src");
    fs::create_dir_all(source_root.join("Documents")).unwrap();
    let target = source_root.join("Documents/Shipment.xml");
    fs::write(
        &target,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Shipment</Name></Properties></Document></MetaDataObject>"#,
    )
    .unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), source_root.clone(), target),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(snapshot) = registration.capture.capture(&source).unwrap() else {
        panic!("Platform XML source must be captured");
    };
    let ProbeResult::Match(descriptor) = registration.probe.probe(&source).unwrap() else {
        panic!("Platform XML descriptor must be recognized");
    };
    assert_eq!(descriptor.format_version.to_string(), "2.20");

    let envelope = registration
        .read
        .read(&FormatReadRequest {
            source,
            snapshot,
            query: NavigationQuery {
                target: NavigationTarget::ObjectPath("Documents/Shipment.xml".to_string()),
                select: NavigationSelection {
                    properties: PropertySelection::All,
                    facets: FacetSelection::Full,
                    relations: Vec::new(),
                },
            },
        })
        .unwrap();

    assert_eq!(envelope.status, NavigationStatus::Available);
    assert!(envelope
        .nodes
        .iter()
        .any(|node| node.object_ref.display_name == "Shipment"));
    assert_eq!(
        registration.manifest.required_features,
        BTreeSet::<String>::new()
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ownership_rejects_external_report_in_external_processor_source_set() {
    let root = fixture_root("wrong-external-kind");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("Demo.xml");
    fs::write(
        &target,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><ExternalReport/></MetaDataObject>"#,
    )
    .unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), root.clone(), root.clone()),
        Some("external".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
    .with_configured_source_set_kind(Some(ConfiguredSourceSetKind::ExternalProcessor));

    let error = PlatformXmlAdapterFactory::new()
        .registration()
        .ownership
        .resolve(&OwnerResolutionRequest {
            source,
            expected_artifact: None,
            mode: OwnerResolutionMode::Existing,
        })
        .unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    assert!(error.message.contains("external_processor"), "{error}");
    assert!(error.message.contains("external_report"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn read_port_rejects_every_unsupported_navigation_target() {
    let (root, source, snapshot) = captured_fixture("unsupported-query-targets");
    let registration = PlatformXmlAdapterFactory::new().registration();
    let selection = full_selection();
    let object_ref = ObjectRef::new(
        SourceId::new("workspace:main").unwrap(),
        ObjectKey::new("document:Shipment").unwrap(),
        IdentityStrength::Persistent,
        NodeKind::MetadataObject {
            metadata_type: "Document".to_string(),
        },
        "Shipment",
    );
    let cursor = NavigationCursor {
        schema_version: NavigationCursor::SCHEMA_VERSION,
        source_id: SourceId::new("workspace:main").unwrap(),
        snapshot_revision: SourceRevision::new("sha256:test").unwrap(),
        target: ObjectKey::new("document:Shipment").unwrap(),
        relation: RelationKey::new("children").unwrap(),
        relation_role: RelationRole::Children,
        relation_kind: RelationKind::Contains,
        selection: selection.clone(),
        selection_hash: "sha256:test".to_string(),
        auth_tag: "test".to_string(),
        next_position: 1,
    };
    let targets = [
        NavigationTarget::ObjectPath("Documents/Other.xml".to_string()),
        NavigationTarget::ObjectRef {
            object_ref,
            snapshot_revision: snapshot.revision.clone(),
        },
        NavigationTarget::Cursor(cursor),
    ];

    for target in targets {
        let error = registration
            .read
            .read(&FormatReadRequest {
                source: source.clone(),
                snapshot: snapshot.clone(),
                query: NavigationQuery {
                    target,
                    select: selection.clone(),
                },
            })
            .unwrap_err();
        assert_eq!(error.kind, SourceAdapterErrorKind::CapabilityBlocked);
    }
    fs::remove_dir_all(root).unwrap();
}

fn captured_fixture(
    label: &str,
) -> (
    std::path::PathBuf,
    SourceContext,
    unica_format_core::source::SourceSnapshot,
) {
    let root = fixture_root(label);
    let source_root = root.join("src");
    fs::create_dir_all(source_root.join("Documents")).unwrap();
    let target = source_root.join("Documents/Shipment.xml");
    fs::write(
        &target,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>Shipment</Name></Properties></Document></MetaDataObject>"#,
    )
    .unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), source_root, target),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
    .with_configured_source_set_kind(Some(ConfiguredSourceSetKind::Configuration));
    let CaptureResult::Captured(snapshot) = PlatformXmlAdapterFactory::new()
        .registration()
        .capture
        .capture(&source)
        .unwrap()
    else {
        panic!("fixture must be captured");
    };
    (root, source, snapshot)
}

fn fixture_root(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "unica-platform-xml-{label}-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ))
}

fn full_selection() -> NavigationSelection {
    NavigationSelection {
        properties: PropertySelection::All,
        facets: FacetSelection::Full,
        relations: Vec::new(),
    }
}
