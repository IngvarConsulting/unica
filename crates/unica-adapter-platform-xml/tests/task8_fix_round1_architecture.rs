use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap()
        .to_path_buf()
}

fn source(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_default()
}

#[test]
fn one_adapter_owned_publication_and_locking_implementation_exists() {
    assert!(
        !root()
            .join(
                "crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs"
            )
            .exists(),
        "the host duplicate publisher must be deleted"
    );

    let host_transaction =
        source("crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs");
    assert!(host_transaction.contains("ArtifactWritePort"));
    for forbidden in ["PROCESS_LOCKS", "fs2::", "publish_single_file("] {
        assert!(
            !host_transaction.contains(forbidden),
            "host transaction retains publication implementation `{forbidden}`"
        );
    }
}

#[test]
fn typed_writer_dispatch_never_reconstructs_transport_or_legacy_native_envelopes() {
    assert!(
        !root()
            .join(
                "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/writer_arguments.rs"
            )
            .exists(),
        "the generic writer argument carrier must be deleted"
    );
    let operations = source("crates/unica-adapter-platform-xml/src/operations/mod.rs");
    let registry =
        source("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/registry.rs");
    for (label, value) in [("operations", operations), ("registry", registry)] {
        for forbidden in [
            "WriterArguments",
            "writer_arguments",
            "ArgumentAccess",
            "writer_native_arguments",
            "writer_operation",
            "writer_tool_name",
            "Map<String, Value>",
            "operation: &str",
            "\"cf-init\"",
            "\"meta-compile\"",
            "\"unica.",
        ] {
            assert!(
                !value.contains(forbidden),
                "{label} retains legacy writer envelope `{forbidden}`"
            );
        }
    }

    for family in [
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
    ] {
        let implementation = source(&format!(
            "crates/unica-adapter-platform-xml/src/versions/v2_20/writers/{family}.rs"
        ));
        assert!(
            !implementation.contains("_command: &unica_format_core::commands::"),
            "{family} typed entrypoint ignores its semantic command"
        );
    }
}

#[test]
fn bsl_generation_is_host_owned_and_platform_artifacts_are_adapter_owned() {
    let adapter_cfe = source("crates/unica-adapter-platform-xml/src/versions/v2_20/writers/cfe.rs");
    let adapter_cfe = adapter_cfe
        .split("#[cfg(test)]\npub(crate) fn patch_extension_method")
        .next()
        .unwrap_or(&adapter_cfe);
    for forbidden in [
        "generate_bsl",
        "&Перед",
        "&После",
        "Procedure ",
        "Function ",
        "EndProcedure",
        "EndFunction",
    ] {
        assert!(
            !adapter_cfe.contains(forbidden),
            "adapter CFE writer still interprets or generates BSL `{forbidden}`"
        );
    }

    let host_code = source("crates/unica-coder/src/infrastructure/native_operations/code.rs");
    let host_code = host_code
        .split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or(&host_code);
    let host_registry =
        source("crates/unica-coder/src/infrastructure/native_operations/registry.rs");
    assert!(host_code.contains("ExtensionPatchEmissionPlan"));
    assert!(host_code.contains("render_extension_method_patch"));
    assert!(host_code.contains("ArtifactWriteRequest"));
    assert!(host_code.contains(".artifact_write()"));
    assert!(host_code.contains(".write(&request)"));
    assert!(
        host_registry.contains("code::render_extension_method_patch"),
        "the host must supply the BSL emitter to the adapter call path"
    );
    assert!(
        host_registry.contains("capture_writer_session_with_extension_emitter"),
        "the adapter call path must accept host-produced BSL content"
    );
    for forbidden in [
        "with_extension(\"xml\")",
        "read_dir(",
        "Configuration.xml",
        "ChildObjects",
    ] {
        assert!(
            !host_code.contains(forbidden),
            "host BSL logic retains Platform XML topology `{forbidden}`"
        );
    }
}

#[test]
fn core_writer_commands_use_closed_operations_and_purpose_specific_operands() {
    let commands = source("crates/unica-format-core/src/commands/mod.rs");
    for forbidden in [
        "WriterArgument",
        "ConfigurationMutationValue",
        "MetadataMutationValue",
        "InterfaceMutationValue",
        "SubsystemMutationValue",
        "DataCompositionMutationValue",
        "semantic_text!(ExtensionPurpose",
    ] {
        assert!(
            !commands.contains(forbidden),
            "core writer contract still exposes generic or open operand {forbidden}"
        );
    }
    for required in [
        "pub enum ConfigurationMutation",
        "pub enum MetadataMutation",
        "pub enum InterfaceEdit",
        "pub enum SubsystemEdit",
        "pub enum DataCompositionMutation",
        "pub enum ExtensionPurpose",
        "pub enum FormPurpose",
        "pub enum DefaultFormAssignment",
        "pub enum FormCompileSource",
    ] {
        assert!(
            commands.contains(required),
            "closed writer contract is missing {required}"
        );
    }
}

#[test]
fn public_adapter_surface_remains_factory_plus_neutral_ports() {
    let lib = source("crates/unica-adapter-platform-xml/src/lib.rs");
    let factory = source("crates/unica-adapter-platform-xml/src/factory.rs");
    assert_eq!(
        lib.lines()
            .filter(|line| line.trim_start().starts_with("pub use "))
            .collect::<Vec<_>>(),
        ["pub use factory::PlatformXmlAdapterFactory;"]
    );
    for hook in [
        "with_publication_lock_pause",
        "with_publication_lock_contention_signal",
    ] {
        let declaration = format!("#[cfg(feature = \"test-support\")]\n    pub fn {hook}");
        assert!(
            factory.contains(&declaration),
            "{hook} must not exist in the default production adapter API"
        );
    }

    let core = source("crates/unica-format-core/src/commands/mod.rs");
    for forbidden in [
        "PathBuf",
        "serde_json",
        "AdapterOutcome",
        "native",
        "xmlns",
        ".xml",
        "2.20",
    ] {
        assert!(
            !core.contains(forbidden),
            "public writer contract leaks `{forbidden}`"
        );
    }
}
