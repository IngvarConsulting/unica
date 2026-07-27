use std::{fs, path::PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn source(relative: &str) -> String {
    fs::read_to_string(repo_root().join(relative)).unwrap()
}

#[test]
fn adapter_public_surface_has_no_native_registry_or_profile_queries() {
    let factory = source("crates/unica-adapter-platform-xml/src/factory.rs");
    let lib = source("crates/unica-adapter-platform-xml/src/lib.rs");

    for forbidden in [
        "validate_coverage_manifest",
        "metadata_classes",
        "PLATFORM_LINE",
        "EXPORT_FORMAT",
        "support_decision",
        "support_status",
        "support_header",
    ] {
        assert!(!factory.contains(forbidden), "factory exposes {forbidden}");
        assert!(!lib.contains(forbidden), "library exposes {forbidden}");
    }
}

#[test]
fn operational_core_contracts_contain_no_filesystem_or_native_wire_vocabulary() {
    let ports = source("crates/unica-format-core/src/ports.rs");
    let operational = &ports[ports.find("pub struct OperationalSourceSession").unwrap()..];

    for forbidden in [
        "PathBuf",
        "std::path",
        "BTreeMap<String, String>",
        "Configuration.xml",
        "MetaDataObject",
        "MDClasses",
        "8.3.27",
        "2.20",
        "message: String",
        "summary: String",
    ] {
        assert!(!operational.contains(forbidden), "{forbidden}");
    }
}
