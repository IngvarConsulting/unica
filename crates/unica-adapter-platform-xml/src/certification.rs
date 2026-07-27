use std::{
    collections::BTreeSet,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use unica_format_core::{
    navigation::{
        FacetSelection, IdentityStrength, NavigationCursor, NavigationQuery, NavigationSelection,
        NavigationStatus, NavigationTarget, NodeKind, ObjectKey, ObjectRef, PropertySelection,
        RelationGroupRef, RelationKind, RelationRole,
    },
    ports::{
        CaptureResult, FormatInspectionMode, FormatInspectionRequest, FormatReadRequest,
        OwnerResolutionMode, OwnerResolutionRequest, ProbeResult, SupportInspectionRequest,
        SupportSourceState,
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
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("Platform XML source must be captured");
    };
    let ProbeResult::Match(descriptor) = registration.probe.probe(&captured).unwrap() else {
        panic!("Platform XML descriptor must be recognized");
    };
    assert_eq!(descriptor.format_version.to_string(), "2.20");

    let envelope = registration
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
fn captured_session_projects_initial_bytes_across_capture_probe_and_read_mutation() {
    let root = fixture_root("immutable-captured-session");
    let source_root = root.join("src");
    fs::create_dir_all(source_root.join("Documents")).unwrap();
    let target = source_root.join("Documents/Shipment.xml");
    let document = |name: &str| {
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document uuid="11111111-1111-1111-1111-111111111111"><Properties><Name>{name}</Name></Properties></Document></MetaDataObject>"#
        )
    };
    fs::write(&target, document("Shipment")).unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), source_root, target.clone()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("Platform XML source must be captured");
    };

    fs::write(&target, document("MutatedBeforeProbe")).unwrap();
    let ProbeResult::Match(_) = registration.probe.probe(&captured).unwrap() else {
        panic!("captured Platform XML descriptor must remain recognized");
    };
    fs::write(&target, document("MutatedBeforeRead")).unwrap();

    let envelope = registration
        .read
        .read(&FormatReadRequest {
            captured: captured.clone(),
            query: NavigationQuery {
                target: NavigationTarget::CapturedTarget(
                    captured.binding().target_identity.clone(),
                ),
                select: full_selection(),
            },
        })
        .expect("read must use the retained capture instead of reopening the filesystem");

    assert!(envelope
        .nodes
        .iter()
        .any(|node| node.object_ref.display_name == "Shipment"));
    assert!(!envelope.nodes.iter().any(|node| {
        matches!(
            node.object_ref.display_name.as_str(),
            "MutatedBeforeProbe" | "MutatedBeforeRead"
        )
    }));
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
            mode: OwnerResolutionMode::Existing,
        })
        .unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::DecodeCorrupted);
    assert!(error.message.contains("external_processor"), "{error}");
    assert!(error.message.contains("external_report"), "{error}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ownership_rejects_mixed_external_kinds_when_correct_owner_sorts_first() {
    assert_mixed_external_owners(
        "mixed-correct-then-wrong",
        &[
            ("A.xml", "ExternalDataProcessor"),
            ("B.xml", "ExternalReport"),
        ],
        false,
    );
}

#[test]
fn ownership_rejects_mixed_external_kinds_when_wrong_owner_sorts_first() {
    assert_mixed_external_owners(
        "mixed-wrong-then-correct",
        &[
            ("A.xml", "ExternalReport"),
            ("B.xml", "ExternalDataProcessor"),
        ],
        false,
    );
}

#[test]
fn ownership_accepts_multiple_external_owners_of_the_configured_kind() {
    assert_mixed_external_owners(
        "multiple-correct",
        &[
            ("A.xml", "ExternalDataProcessor"),
            ("B.xml", "ExternalDataProcessor"),
        ],
        true,
    );
}

#[test]
fn format_inspection_uses_the_authorized_source_target_not_an_unrestricted_path() {
    let root = fixture_root("authorized-format-target");
    fs::create_dir_all(&root).unwrap();
    let authorized = root.join("Authorized.xml");
    fs::write(
        &authorized,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Document/></MetaDataObject>"#,
    )
    .unwrap();
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), root.clone(), authorized),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );

    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .format_inspection
        .inspect(&FormatInspectionRequest {
            source,
            mode: FormatInspectionMode::Versioned,
        })
        .unwrap();

    assert_eq!(result.compatibility.unwrap().actual().to_string(), "2.20");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn support_inspection_rejects_a_target_outside_the_authorized_source_root() {
    let root = fixture_root("support-source-boundary");
    let source_root = root.join("source");
    fs::create_dir_all(&source_root).unwrap();
    let outside = root.join("outside.bin");
    fs::write(&outside, []).unwrap();
    let registration = PlatformXmlAdapterFactory::new().registration();

    let error = registration
        .support
        .inspect(&SupportInspectionRequest {
            source: SourceContext::new(
                SourceLocation::new(root.clone(), source_root, outside),
                None,
                SourceFamily::PlatformXml,
                None,
            ),
            object: None,
        })
        .unwrap_err();

    assert_eq!(error.kind, SourceAdapterErrorKind::SourceUnavailable);

    let authorized_root = root.join("authorized");
    fs::create_dir_all(&authorized_root).unwrap();
    let evidence = registration
        .support
        .inspect(&SupportInspectionRequest {
            source: SourceContext::new(
                SourceLocation::new(
                    root.clone(),
                    authorized_root.clone(),
                    authorized_root.join("Ext").join("ParentConfigurations.bin"),
                ),
                None,
                SourceFamily::PlatformXml,
                None,
            ),
            object: None,
        })
        .unwrap();
    assert_eq!(evidence.source, SupportSourceState::Absent);
    fs::remove_dir_all(root).unwrap();
}

fn assert_mixed_external_owners(label: &str, owners: &[(&str, &str)], expected_success: bool) {
    let root = fixture_root(label);
    fs::create_dir_all(&root).unwrap();
    for (name, kind) in owners {
        fs::write(
            root.join(name),
            format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}/></MetaDataObject>"#
            ),
        )
        .unwrap();
    }
    let source = SourceContext::new(
        SourceLocation::new(root.clone(), root.clone(), root.clone()),
        Some("external".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
    .with_configured_source_set_kind(Some(ConfiguredSourceSetKind::ExternalProcessor));

    let result = PlatformXmlAdapterFactory::new()
        .registration()
        .ownership
        .resolve(&OwnerResolutionRequest {
            source,
            mode: OwnerResolutionMode::Existing,
        });

    assert_eq!(result.is_ok(), expected_success, "{result:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn read_port_rejects_every_unsupported_navigation_target() {
    let (root, captured) = captured_fixture("unsupported-query-targets");
    let registration = PlatformXmlAdapterFactory::new().registration();
    let selection = full_selection();
    let source_id = SourceId::new("workspace:main").unwrap();
    let object_ref = ObjectRef::new(
        source_id.clone(),
        ObjectKey::new("document:Shipment").unwrap(),
        IdentityStrength::Persistent,
        NodeKind::Document,
        "Shipment",
    );
    let group = RelationGroupRef::new(
        source_id.clone(),
        object_ref.clone(),
        RelationRole::Children,
        RelationKind::Contains,
    )
    .unwrap();
    let cursor = NavigationCursor::issue(
        b"certification-cursor",
        source_id,
        SourceRevision::new("sha256:test").unwrap(),
        object_ref.object_key.clone(),
        group,
        selection.clone(),
        1,
    )
    .unwrap();
    let targets = [
        NavigationTarget::ObjectPath("Documents/Other.xml".to_string()),
        NavigationTarget::ObjectRef {
            object_ref,
            snapshot_revision: captured.snapshot().revision.clone(),
        },
        NavigationTarget::Cursor(cursor),
    ];

    for target in targets {
        let error = registration
            .read
            .read(&FormatReadRequest {
                captured: captured.clone(),
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

fn captured_fixture(label: &str) -> (std::path::PathBuf, unica_format_core::ports::CapturedSource) {
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
    let CaptureResult::Captured(captured) = PlatformXmlAdapterFactory::new()
        .registration()
        .capture
        .capture(&source)
        .unwrap()
    else {
        panic!("fixture must be captured");
    };
    (root, captured)
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
