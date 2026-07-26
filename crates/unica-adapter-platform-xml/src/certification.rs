use std::{
    collections::BTreeSet,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use unica_format_core::{
    navigation::{
        FacetSelection, NavigationQuery, NavigationSelection, NavigationStatus, NavigationTarget,
        PropertySelection,
    },
    ports::{CaptureResult, FormatReadRequest, ProbeResult},
    source::{SourceContext, SourceFamily, SourceLocation},
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
