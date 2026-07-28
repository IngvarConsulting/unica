use std::{fs, path::PathBuf};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn source(path: &str) -> String {
    fs::read_to_string(workspace_root().join(path))
        .unwrap_or_else(|error| panic!("failed to read {path}: {error}"))
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let start = text
        .find(start)
        .unwrap_or_else(|| panic!("missing start marker {start:?}"));
    let tail = &text[start..];
    let end = tail
        .find(end)
        .unwrap_or_else(|| panic!("missing end marker {end:?}"));
    &tail[..end]
}

#[test]
fn core_writer_boundary_is_closed_and_format_neutral() {
    let commands = source("crates/unica-format-core/src/commands/writer_payloads.rs");
    let command_enum = source("crates/unica-format-core/src/commands/mod.rs");
    let forbidden = [
        "WriterArgument",
        "serde_json",
        "ModuleReference",
        "compatibility_mode: String",
        "definition: Vec<u8>",
        "definition: String",
        "raw_definition",
        "definition_bytes",
        "raw_payload",
        "payload: String",
        "operation: String",
        "operation_id",
        "tool_name",
        "value: String",
        "HashMap<",
        "BTreeMap<",
        "unica.",
        "Configuration.xml",
        "Form.xml",
        "version=\"",
    ];
    for token in forbidden {
        assert!(
            !commands.contains(token),
            "core writer boundary contains forbidden token {token:?}"
        );
    }
    for required in [
        "pub enum ExtensionModuleTarget",
        "pub enum CapabilityRequirement",
        "pub struct VersionNumber",
        "pub const fn metadata_kind_allows_property",
        "deny_unknown_fields",
        "deserialize_non_empty_vec",
    ] {
        assert!(
            commands.contains(required),
            "core writer boundary misses invariant {required:?}"
        );
    }
    assert!(command_enum.contains("pub enum WriterCommand"));
    assert!(!command_enum.contains("WriterArgument"));
}

#[test]
fn writer_session_contains_only_opaque_sources_and_execution_context() {
    let operations = source("crates/unica-adapter-platform-xml/src/operations/mod.rs");
    let session = between(
        &operations,
        "pub(crate) struct PlatformWriterSession",
        "impl PlatformWriterSession",
    );
    for forbidden in [
        "definition",
        "payload",
        "operation",
        "value:",
        "compatibility",
        "version",
        "serde_json",
        "serde_json::Map",
        "Map<String, Value>",
    ] {
        assert!(
            !session.contains(forbidden),
            "writer session contains forbidden field {forbidden:?}"
        );
    }
    for required in ["sources:", "context:", "extension_emitter:"] {
        assert!(
            session.contains(required),
            "writer session misses {required:?}"
        );
    }
}

#[test]
fn dispatch_matches_closed_variants_without_tool_ids_or_json_maps() {
    let dispatch =
        source("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/registry.rs");
    for forbidden in [
        "serde_json",
        "Map<",
        "\"cf-",
        "\"cfe-",
        "\"meta-",
        "\"form-",
        "\"dcs-",
        "\"mxl-",
        "\"unica.",
        "operation_id",
        "tool_name",
    ] {
        assert!(
            !dispatch.contains(forbidden),
            "writer dispatch reconstructs a legacy envelope via {forbidden:?}"
        );
    }
    assert!(dispatch.contains("WriterCommand::ConfigurationInitialize"));
    for variant in [
        "ConfigurationInitialize",
        "ConfigurationEdit",
        "ExtensionInitialize",
        "ExtensionBorrow",
        "ExtensionPatchMethod",
        "ExternalProcessorInitialize",
        "ExternalReportInitialize",
        "MetadataCreate",
        "MetadataEdit",
        "MetadataRemove",
        "FormCreate",
        "FormCompile",
        "FormEdit",
        "FormRemove",
        "TemplateCreate",
        "TemplateRemove",
        "HelpCreate",
        "InterfaceEdit",
        "RoleCreate",
        "SubsystemCreate",
        "SubsystemEdit",
        "SupportEdit",
        "DataCompositionCreate",
        "DataCompositionEdit",
        "SpreadsheetCreate",
    ] {
        assert!(
            dispatch.contains(&format!("WriterCommand::{variant}")),
            "writer dispatch misses typed command variant {variant}"
        );
    }
}

#[test]
fn production_writer_entrypoints_do_not_serialize_commands_back_to_legacy_inputs() {
    for path in [
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cf.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cfe.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/interface.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/meta.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mxl.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/role.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/subsystem.rs",
    ] {
        let writer = source(path);
        for forbidden in [
            "serde_json::to_value(command",
            "serde_json::to_string(command",
            "semantic_operations:",
            "semantic_patch:",
            "semantic_edit:",
        ] {
            assert!(
                !writer.contains(forbidden),
                "{path} reconstructs a legacy writer input via {forbidden:?}"
            );
        }
    }

    let dcs = source("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs");
    assert!(
        !dcs.contains("failed to encode DCS semantic mutation"),
        "DCS typed mutations still serialize through the legacy value grammar"
    );

    let deleted_projection_helpers = [
        "metadata_definition_native_value",
        "dcs_native_mutation(",
        "form_semantic_definition",
        "mxl_semantic_definition",
        "role_semantic_definition",
        "subsystem_semantic_definition",
        "interface_semantic_definition",
    ];
    for path in [
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cf.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/dcs.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/form.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/interface.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/meta.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/mxl.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/role.rs",
        "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/subsystem.rs",
    ] {
        let writer = source(path);
        for helper in deleted_projection_helpers {
            assert!(
                !writer.contains(helper),
                "{path} retains deleted legacy projection helper {helper:?}"
            );
        }
    }
}

#[test]
fn host_parses_public_module_path_into_closed_target_and_not_a_filesystem_path() {
    let registry = source("crates/unica-coder/src/infrastructure/native_operations/registry.rs");
    assert!(registry.contains("fn parse_extension_module_target"));
    assert!(registry.contains("ExtensionModuleTarget::Common"));
    assert!(registry.contains("ExtensionModuleTarget::Object"));
    assert!(registry.contains("ExtensionModuleTarget::Form"));
    assert!(!registry.contains("ModuleReference::new"));

    let descriptors = source("crates/unica-coder/src/application/operation_descriptors.rs");
    let path_groups = between(
        &descriptors,
        "const CFE_PATCH_METHOD_PATH_GROUPS",
        "const CFE_VALIDATE_PATH_GROUPS",
    );
    assert!(!path_groups.contains("MODULE_PATH"));
}

#[test]
fn preservation_matrix_executes_writer_and_reader_ports_and_compares_reader_facts() {
    let matrix =
        source("crates/unica-adapter-platform-xml/tests/task8_fix_round1_preservation_matrix.rs");
    for required in [
        ".execute(&request)",
        ".registration()",
        ".read(&FormatReadRequest",
        "SemanticFact",
        "expected_fact_digests",
        "fact_set_digest",
        "assert_declared_delta",
        "normalize_envelope",
        "normalize_standalone",
        "assert_eq!(covered, expected",
    ] {
        assert!(
            matrix.contains(required),
            "preservation matrix does not execute required behavior {required:?}"
        );
    }
    for forbidden in [
        "test_names",
        "contains(case_name)",
        "Command::new(\"rg\"",
        "ExpectedMarker",
        "TASK8_DUMP_FACT_DIGESTS",
    ] {
        assert!(
            !matrix.contains(forbidden),
            "preservation matrix uses source/name scanning via {forbidden:?}"
        );
    }
}
