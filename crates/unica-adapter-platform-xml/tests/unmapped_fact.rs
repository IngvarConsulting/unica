use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::{
        FacetSelection, NavigationQuery, NavigationSelection, NavigationStatus, NavigationTarget,
        PropertySelection,
    },
    ports::{CaptureResult, FormatReadRequest},
    semantic_ids::SemanticPropertyId,
    source::{SourceContext, SourceFamily, SourceLocation},
    value::PropertyValue,
};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

#[test]
fn meaningful_unmapped_property_and_known_child_role_are_partial_and_neutral() {
    let root = std::env::temp_dir().join(format!(
        "unica-platform-xml-task5-unmapped-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ));
    let source_root = root.join("src");
    fs::create_dir_all(&source_root).unwrap();
    let target = source_root.join("Ledger.xml");
    fs::write(
        &target,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
<ChartOfAccounts uuid="11111111-1111-1111-1111-111111111111">
  <Properties>
    <Name>Ledger</Name>
    <Comment>Mapped sibling remains visible</Comment>
    <NativeOnlyMeaningfulFact>preserve-me-internally</NativeOnlyMeaningfulFact>
  </Properties>
  <ChildObjects>
    <AccountingFlag uuid="22222222-2222-2222-2222-222222222222">
      <Properties><Name>TrackQuantity</Name></Properties>
    </AccountingFlag>
  </ChildObjects>
</ChartOfAccounts>
</MetaDataObject>"#,
    )
    .unwrap();

    let source = SourceContext::new(
        SourceLocation::new(root.clone(), source_root, target),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    );
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) = registration.capture.capture(&source).unwrap() else {
        panic!("fixture must be captured");
    };
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
        .expect("recoverable coverage gaps must not make the source unreadable");

    assert_eq!(envelope.status, NavigationStatus::Partial);
    let ledger = envelope
        .nodes
        .iter()
        .find(|node| node.object_ref.display_name == "Ledger")
        .expect("mapped node");
    assert_eq!(
        ledger.properties[&SemanticPropertyId::METADATA_COMMENT].value(),
        Some(&PropertyValue::String(
            "Mapped sibling remains visible".to_string()
        ))
    );
    assert!(!ledger
        .properties
        .keys()
        .any(|id| id.as_str().contains("native")));

    let unmapped = envelope
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "unmappedSemanticFact")
        .collect::<Vec<_>>();
    assert_eq!(
        unmapped.len(),
        2,
        "each unmapped property or known child role needs auditable evidence"
    );
    let public_diagnostics = serde_json::to_string(&unmapped).unwrap();
    for forbidden in [
        "NativeOnlyMeaningfulFact",
        "AccountingFlag",
        "preserve-me-internally",
        "MetaDataObject",
        "http://",
        root.to_str().unwrap(),
    ] {
        assert!(
            !public_diagnostics.contains(forbidden),
            "public diagnostic leaked native evidence: {forbidden}"
        );
    }

    fs::remove_dir_all(root).unwrap();
}
