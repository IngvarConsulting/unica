use std::path::PathBuf;

use unica_format_core::{
    ports::{
        CapabilityPort, CapturePort, FormatWriteCommand, ProbePort, ReadPort, ValidationPort,
        WritePort,
    },
    semantic_ids::{SemanticFacetId, SemanticObjectKind, SemanticPropertyId, SemanticRelationId},
    source::{FormatVersion, SourceContext, SourceFamily, SourceLocation},
};

#[test]
fn source_context_carries_locations_without_a_host_workspace_type() {
    let location = SourceLocation::new(
        PathBuf::from("/workspace"),
        PathBuf::from("/workspace/src"),
        PathBuf::from("/workspace/src/Documents/Order.xml"),
    );
    let context = SourceContext::new(
        location,
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        Some(FormatVersion::parse("2.20").unwrap()),
    );

    assert_eq!(
        context.location().workspace_root(),
        PathBuf::from("/workspace")
    );
    assert_eq!(
        context.location().source_root(),
        PathBuf::from("/workspace/src")
    );
    assert_eq!(
        context.location().target(),
        PathBuf::from("/workspace/src/Documents/Order.xml")
    );
}

#[test]
fn semantic_ids_and_ports_form_a_closed_compiler_contract() {
    assert_eq!(SemanticPropertyId::NAME.as_str(), "name");
    assert_eq!(SemanticRelationId::ATTRIBUTES.as_str(), "attributes");
    assert_eq!(SemanticFacetId::SUMMARY.as_str(), "summary");
    assert_eq!(SemanticObjectKind::DOCUMENT.as_str(), "document");

    fn assert_port<T: ?Sized + Send + Sync>() {}
    assert_port::<dyn ProbePort>();
    assert_port::<dyn CapturePort>();
    assert_port::<dyn ReadPort>();
    assert_port::<dyn WritePort>();
    assert_port::<dyn ValidationPort>();
    assert_port::<dyn CapabilityPort>();

    let _typed_writer_boundary = std::mem::size_of::<FormatWriteCommand>();
}
