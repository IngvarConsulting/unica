use std::{fs, path::Path};

const FAMILIES: &[&str] = &[
    "common",
    "compile_transaction",
    "cf",
    "cfe",
    "dcs",
    "external",
    "form",
    "help",
    "interface",
    "meta",
    "mxl",
    "role",
    "subsystem",
    "support",
    "template",
];

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate belongs to the workspace")
}

#[test]
fn task8_platform_xml_writer_implementation_is_adapter_owned() {
    let root = repository_root();

    for family in FAMILIES {
        let adapter = root
            .join("crates/unica-adapter-platform-xml/src/versions/v2_20/writers")
            .join(format!("{family}.rs"));
        assert!(
            adapter.is_file(),
            "adapter writer family is missing: {family}"
        );

        let orchestration_copy = root
            .join("crates/unica-adapter-platform-xml/src/operations")
            .join(format!("{family}.rs"));
        assert!(
            !orchestration_copy.exists(),
            "{family} must not be duplicated outside the private versioned writer"
        );

        let host = root
            .join("crates/unica-coder/src/infrastructure/native_operations")
            .join(format!("{family}.rs"));
        let host_source = fs::read_to_string(&host).unwrap_or_default();
        for forbidden in [
            "roxmltree",
            "quick_xml",
            "Document::parse",
            "xmlns:",
            "Configuration.xml",
            "ChildObjects",
            "ParentConfigurations.bin",
        ] {
            assert!(
                !host_source.contains(forbidden),
                "{family} retains native writer vocabulary `{forbidden}` in the host"
            );
        }
    }

    let host_code = fs::read_to_string(
        root.join("crates/unica-coder/src/infrastructure/native_operations/code.rs"),
    )
    .expect("BSL host implementation");
    for forbidden in [
        "fn module_identity",
        "fn metadata_descriptor",
        "fn direct_role_is_supported",
        "fn nested_modules_are_supported",
    ] {
        assert!(
            !host_code.contains(forbidden),
            "BSL host retains Platform XML topology `{forbidden}`"
        );
    }

    let locator =
        root.join("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/module_locator.rs");
    assert!(
        locator.is_file(),
        "versioned BSL artifact locator is missing"
    );

    let factory = fs::read_to_string(root.join("crates/unica-adapter-platform-xml/src/factory.rs"))
        .expect("adapter factory source");
    let writer_capture = factory
        .split("pub fn capture_writer_session")
        .nth(1)
        .and_then(|tail| tail.split("pub fn capture_inspection_session").next())
        .expect("writer session capture API");
    for forbidden in ["serde_json::Map", "operation: &str", "tool_name"] {
        assert!(
            !writer_capture.contains(forbidden),
            "writer factory boundary exposes transport vocabulary `{forbidden}`"
        );
    }

    let operations =
        fs::read_to_string(root.join("crates/unica-adapter-platform-xml/src/operations/mod.rs"))
            .expect("adapter operation source");
    assert!(
        !operations.contains("struct AdapterOutcome"),
        "MCP AdapterOutcome must remain host-owned"
    );
    let writer_session = operations
        .split("struct PlatformWriterSession")
        .nth(1)
        .and_then(|tail| {
            tail.split("pub(crate) struct PlatformInspectionSession")
                .next()
        })
        .expect("private writer session");
    for forbidden in [
        "Map<String, Value>",
        "operation: String",
        "tool_name: String",
    ] {
        assert!(
            !writer_session.contains(forbidden),
            "writer session retains transport escape hatch `{forbidden}`"
        );
    }
}
