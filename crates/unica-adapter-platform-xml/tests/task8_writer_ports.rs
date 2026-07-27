use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::{
        ConfigurationCommand, ModuleOwner, ModuleRole, MutationMode, WriterCommand, WriterFamily,
        WriterSourceRole, WriterStatus,
    },
    ports::{ModuleArtifactLocatorRequest, OperationCancellation, WriterRequest},
    semantic_ids::SemanticObjectKind,
};

#[test]
fn task8_factory_registers_every_existing_writer_family() {
    let registration = PlatformXmlAdapterFactory::new().operational_registration();
    let actual = registration
        .writer()
        .families()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let expected = WriterFamily::ALL.into_iter().collect::<BTreeSet<_>>();

    assert_eq!(actual, expected);
}

#[test]
fn task8_cancelled_writer_cannot_publish_or_validate_native_arguments() {
    let root = fixture_root("cancelled-writer");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("output");

    let factory = PlatformXmlAdapterFactory::new();
    let session = factory
        .capture_writer_session(
            [(WriterSourceRole::DestinationDirectory, output.clone())],
            None,
            None,
            &root,
            &root,
            &root.join(".cache"),
            0,
        )
        .unwrap();
    let cancellation = OperationCancellation::new();
    cancellation.cancel();
    let request = WriterRequest::new(
        session,
        WriterCommand::configuration(ConfigurationCommand::Initialize),
        MutationMode::Apply,
        cancellation,
    );

    let result = factory
        .operational_registration()
        .writer()
        .execute(&request)
        .unwrap();

    assert_eq!(result.status(), WriterStatus::Cancelled);
    assert!(!output.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task8_bsl_artifact_locator_is_semantic_and_failure_closed() {
    let root = fixture_root("module-locator");
    let source = root.join("src");
    let descriptor = source.join("Catalogs/Items.xml");
    let module = source.join("Catalogs/Items/Ext/ObjectModule.bsl");
    write(&descriptor, b"<descriptor/>");
    write(&module, b"Procedure Run()\nEndProcedure\n");

    let factory = PlatformXmlAdapterFactory::new();
    let session = factory.capture_module_artifact_source(&source, &module, &root);
    let request = ModuleArtifactLocatorRequest::new(session, OperationCancellation::new());
    let location = factory
        .operational_registration()
        .module_artifacts()
        .locate(&request)
        .unwrap();

    assert_eq!(location.owner().kind(), SemanticObjectKind::Catalog);
    assert_eq!(location.owner().name(), Some("Items"));
    assert_eq!(location.role(), ModuleRole::Object);
    assert!(matches!(location.owner(), ModuleOwner::Object { .. }));

    let cancellation = OperationCancellation::new();
    cancellation.cancel();
    let cancelled = ModuleArtifactLocatorRequest::new(
        factory.capture_module_artifact_source(&source, &module, &root),
        cancellation,
    );
    assert!(factory
        .operational_registration()
        .module_artifacts()
        .locate(&cancelled)
        .unwrap_err()
        .message
        .starts_with("cancelled:"));

    let outside = root.join("outside/Ext/ObjectModule.bsl");
    write(&outside, b"Procedure Run()\nEndProcedure\n");
    let escaped = ModuleArtifactLocatorRequest::new(
        factory.capture_module_artifact_source(&source, &outside, &root),
        OperationCancellation::new(),
    );
    assert!(factory
        .operational_registration()
        .module_artifacts()
        .locate(&escaped)
        .is_err());

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn task8_bsl_artifact_locator_rejects_symlinked_descriptors() {
    use std::os::unix::fs::symlink;

    let root = fixture_root("module-locator-symlink");
    let source = root.join("src");
    let descriptor = source.join("Catalogs/Items.xml");
    let module = source.join("Catalogs/Items/Ext/ObjectModule.bsl");
    let outside = root.join("outside.xml");
    write(&outside, b"<descriptor/>");
    write(&module, b"Procedure Run()\nEndProcedure\n");
    fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
    symlink(&outside, &descriptor).unwrap();

    let factory = PlatformXmlAdapterFactory::new();
    let request = ModuleArtifactLocatorRequest::new(
        factory.capture_module_artifact_source(&source, &module, &root),
        OperationCancellation::new(),
    );
    let error = factory
        .operational_registration()
        .module_artifacts()
        .locate(&request)
        .unwrap_err();

    assert!(error.message.contains("must not traverse a link"));
    fs::remove_dir_all(root).unwrap();
}

fn fixture_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task8-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}
