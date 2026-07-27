#[test]
fn task7_operational_core_contracts_are_opaque_closed_and_path_free() {
    let ports = include_str!("../../unica-format-core/src/ports.rs");
    let operational = ports
        .split("pub struct FormatDiagnostic")
        .nth(1)
        .expect("Task 7 operational contracts");

    for forbidden in [
        "PathBuf",
        "SourceContext",
        "SourceFamily",
        "FormatVersion",
        "BTreeMap<String, String>",
        "actual_format",
        "target_format",
        "producer_version",
        "workspace_root",
        "source_set",
        "extension",
    ] {
        assert!(
            !operational.contains(forbidden),
            "operational core contract leaked `{forbidden}`"
        );
    }
    for required in [
        "pub struct OperationalSourceSession",
        "pub enum FormatDiagnosticCode",
        "pub enum PublicationStatus",
        "pub enum PublicationCancellation",
        "pub enum PublicationRollback",
        "pub enum PublicationCleanup",
        "pub enum PublicationRecovery",
    ] {
        assert!(
            ports.contains(required),
            "missing closed Task 7 contract `{required}`"
        );
    }
}

#[test]
fn task7_native_registry_and_queries_are_adapter_private() {
    let guards = include_str!("../src/guards.rs");
    let factory = include_str!("../src/factory.rs");
    let public_api = include_str!("../src/lib.rs");

    for forbidden in [
        "LegacyMetadataKind",
        "LEGACY_METADATA_KINDS",
        "pub fn legacy_metadata",
    ] {
        assert!(
            !guards.contains(forbidden),
            "guards retain duplicated native registry `{forbidden}`"
        );
        assert!(
            !public_api.contains(forbidden),
            "adapter public API leaks native registry `{forbidden}`"
        );
    }
    for forbidden in [
        "pub fn platform_line",
        "pub fn export_format",
        "pub fn profile",
        "#[doc(hidden)]",
        "registered_subsystem_names",
        "normalize_metadata_category",
    ] {
        assert!(
            !factory.contains(forbidden),
            "factory leaks native/version query `{forbidden}`"
        );
    }
}

#[test]
fn task7_host_has_no_moved_support_or_validation_readers() {
    let support_guard = include_str!("../../unica-coder/src/infrastructure/support_guard.rs");
    let common =
        include_str!("../../unica-coder/src/infrastructure/native_operations/common.rs");
    let meta = include_str!("../../unica-coder/src/infrastructure/native_operations/meta.rs");
    let validation = include_str!(
        "../../unica-coder/src/infrastructure/native_operations/meta_validation_context.rs"
    );
    let format_guard = include_str!("../../unica-coder/src/infrastructure/format_guard.rs");

    assert!(
        !support_guard.contains(".inspect(&request).ok()?"),
        "support inspection errors still fail open"
    );
    for forbidden in [
        "fn find_support_config_dir",
        "fn support_root_uuid(",
        "fn support_root_uuid_from_bytes",
        "fn support_uuid_dependency_paths",
    ] {
        assert!(
            !common.contains(forbidden),
            "host retains moved support reader `{forbidden}`"
        );
    }
    for forbidden in [
        "meta_validate_registrar_document_scan",
        "config_dir.join(ref_dir)",
    ] {
        assert!(
            !meta.contains(forbidden),
            "host retains moved validation branch `{forbidden}`"
        );
    }
    assert!(
        !validation.contains("#[doc(hidden)]"),
        "validation context still depends on hidden native queries"
    );
    assert!(
        !format_guard.contains("registered_subsystem_names"),
        "format guard still parses native subsystem registration"
    );
}

#[test]
fn task7_publication_state_is_never_inferred_from_messages() {
    let publication = include_str!("../src/publication.rs");

    for forbidden in [
        "summary.contains(\"cancelled\")",
        "error.contains(\"cancelled\")",
        "error.contains(\"recovery\")",
        "error.contains(\"quarantin\")",
    ] {
        assert!(
            !publication.contains(forbidden),
            "publication lifecycle is inferred from text: `{forbidden}`"
        );
    }
}

#[test]
fn task7_moved_flows_have_no_native_layout_and_remaining_joins_are_writer_locations() {
    fn production(source: &str) -> &str {
        source
            .split("\n#[cfg(test)]\nmod ")
            .next()
            .unwrap_or(source)
    }

    fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let marker = format!("fn {name}");
        source
            .split_once(&marker)
            .map(|(_, tail)| tail.split("\nfn ").next().unwrap_or(tail))
            .unwrap_or_else(|| panic!("missing function {name}"))
    }

    let support_guard =
        production(include_str!("../../unica-coder/src/infrastructure/support_guard.rs"));
    let validation = production(include_str!(
        "../../unica-coder/src/infrastructure/native_operations/meta_validation_context.rs"
    ));
    for (name, source) in [
        ("support guard", support_guard),
        ("validation context", validation),
    ] {
        for forbidden in [
            "Configuration.xml",
            "ParentConfigurations.bin",
            "MetaDataObject",
            "MDClasses",
        ] {
            assert!(
                !source.contains(forbidden),
                "{name} retains native read/layout vocabulary `{forbidden}`"
            );
        }
    }

    let format_guard =
        production(include_str!("../../unica-coder/src/infrastructure/format_guard.rs"));
    assert!(
        !format_guard.contains("ParentConfigurations.bin")
            && !format_guard.contains("MetaDataObject")
            && !format_guard.contains("MDClasses"),
        "format guard retains native parsing or support layout"
    );
    let join = ".join(\"Configuration.xml\")";
    let all_joins = format_guard.matches(join).count();
    let writer_location_joins = [
        "add_meta_remove_format_dependencies",
        "add_subsystem_compile_format_dependencies",
        "add_role_compile_format_dependencies",
    ]
    .into_iter()
    .map(|name| function_body(format_guard, name).matches(join).count())
    .sum::<usize>();
    assert_eq!(
        all_joins, writer_location_joins,
        "a moved read/guard/validation flow regained a Configuration.xml join"
    );
    assert_eq!(
        writer_location_joins, 3,
        "Task 8 writer source-location allowlist changed; classify it explicitly"
    );
}
