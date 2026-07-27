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
fn operational_paths_do_not_call_the_whole_tree_snapshot_provider() {
    let operations =
        source("crates/unica-adapter-platform-xml/src/versions/v2_20/operations.rs");
    let source_sets =
        source("crates/unica-adapter-platform-xml/src/versions/v2_20/source_sets.rs");
    let provider =
        source("crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs");

    for text in [&operations, &source_sets] {
        assert!(!text.contains("capture_operational"));
        assert!(!text.contains("capture_authorized_root"));
        assert!(!text.contains("snapshot_files"));
    }
    assert!(operations.contains("SafeSourceRoot"));
    assert!(source_sets.contains("SafeSourceRoot"));
    for forbidden in [
        "MAX_CAPTURED_FILES",
        "MAX_CAPTURED_TOTAL_BYTES",
        "capture_contents(",
        "verify_contents(",
        "files: BTreeMap<String, Arc<[u8]>>",
    ] {
        assert!(
            !provider.contains(forbidden),
            "navigation provider retained whole-root capture state `{forbidden}`"
        );
    }
    assert!(provider.contains("prepare_navigation_snapshot"));
    assert!(provider.contains("NavigationReadLimit::SelectedTarget"));
}

#[test]
fn host_has_no_native_metadata_layout_registry_or_validation_reader_call_path() {
    let root = repo_root();
    assert!(
        !root
            .join("crates/unica-coder/src/infrastructure/metadata_kinds.rs")
            .exists(),
        "the adapter registry must be the only native metadata registry"
    );
    let infrastructure = source("crates/unica-coder/src/infrastructure/mod.rs");
    let guard = source("crates/unica-coder/src/infrastructure/format_guard.rs");
    let guard = guard.split("#[cfg(test)]").next().unwrap();
    let support_guard = source("crates/unica-coder/src/infrastructure/support_guard.rs");
    let support_guard = support_guard.split("#[cfg(test)]").next().unwrap();
    let meta = source("crates/unica-coder/src/infrastructure/native_operations/meta.rs");
    assert!(
        !root
            .join(
                "crates/unica-coder/src/infrastructure/native_operations/meta_validation_context.rs"
            )
            .exists(),
        "the stale host validation-context bridge must be deleted"
    );

    assert!(!infrastructure.contains("metadata_kinds"));
    for forbidden in [
        "Configuration.xml",
        "MetaDataObject",
        "MDClasses",
        "meta_validate_format_dependency_paths",
        "roxmltree",
        "PlatformXmlProvider",
    ] {
        assert!(!guard.contains(forbidden), "format guard contains {forbidden}");
    }
    for forbidden in [
        "meta_remove_type_plural",
        "template_add_object_type_folders",
        ".exists()",
        "\"Catalogs\"",
        "\"Documents\"",
        "\"Reports\"",
    ] {
        assert!(
            !support_guard.contains(forbidden),
            "support guard contains host layout dependency {forbidden}"
        );
    }
    let validation_start = meta.find("pub(crate) fn validate_meta").unwrap();
    let validation_end = validation_start
        + meta[validation_start..]
            .find("pub(crate) fn meta_info_localized_values")
            .unwrap();
    let validation_entry = &meta[validation_start..validation_end];
    assert!(validation_entry.contains("OperationalPolicyService::validate"));
    assert!(validation_entry.contains("capture_unscoped_validation_source"));
    for forbidden in [
        "Document::parse",
        "MetaDataObject",
        "MDClasses",
        "tag_name()",
        "Configuration.xml",
    ] {
        assert!(
            !validation_entry.contains(forbidden),
            "meta.validate host path contains {forbidden}"
        );
    }
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
