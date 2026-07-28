use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Barrier},
    thread,
    time::Duration,
    time::{SystemTime, UNIX_EPOCH},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    commands::{
        ConfigurationInitialize, ConfigurationName, ModuleOwner, ModuleRole, MutationMode,
        WriterCommand, WriterFamily, WriterLifecycle, WriterSourceRole,
    },
    ports::{
        ModuleArtifactLocatorRequest, OperationCancellation, PublicationCancellation,
        PublicationRollback, WriterRequest,
    },
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
        WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(
            ConfigurationName::new("Cancelled").unwrap(),
        )),
        MutationMode::Apply,
        cancellation,
    );

    let result = factory
        .operational_registration()
        .writer()
        .execute(&request)
        .unwrap();

    assert!(matches!(result.lifecycle(), WriterLifecycle::Cancelled(_)));
    assert!(!output.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(feature = "test-support")]
fn task8_writer_cancellation_while_waiting_for_shared_publication_lock_is_typed() {
    let root = fixture_root("cancelled-lock-wait");
    fs::create_dir_all(&root).unwrap();
    let output = root.join("output");
    let factory = PlatformXmlAdapterFactory::new();

    let session_a = factory
        .capture_writer_session(
            [(WriterSourceRole::DestinationDirectory, output.clone())],
            &root,
            &root,
            &root.join(".cache-a"),
            0,
        )
        .unwrap();
    let request_a = WriterRequest::new(
        session_a,
        WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(
            ConfigurationName::new("LockOwner").unwrap(),
        )),
        MutationMode::Apply,
        OperationCancellation::new(),
    );
    let acquired = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let acquired_by_a = Arc::clone(&acquired);
    let release_by_a = Arc::clone(&release);
    let thread_a = thread::spawn(move || {
        PlatformXmlAdapterFactory::new().with_publication_lock_pause(
            acquired_by_a,
            release_by_a,
            || {
                PlatformXmlAdapterFactory::new()
                    .operational_registration()
                    .writer()
                    .execute(&request_a)
            },
        )
    });
    acquired.wait();

    let cancellation = OperationCancellation::new();
    let session_b = factory
        .capture_writer_session(
            [(WriterSourceRole::DestinationDirectory, output.clone())],
            &root,
            &root,
            &root.join(".cache-b"),
            0,
        )
        .unwrap();
    let request_b = WriterRequest::new(
        session_b,
        WriterCommand::ConfigurationInitialize(ConfigurationInitialize::new(
            ConfigurationName::new("CancelledWaiter").unwrap(),
        )),
        MutationMode::Apply,
        cancellation.clone(),
    );
    let (contended_sender, contended_receiver) = mpsc::channel();
    let thread_b = thread::spawn(move || {
        PlatformXmlAdapterFactory::new().with_publication_lock_contention_signal(
            contended_sender,
            || {
                PlatformXmlAdapterFactory::new()
                    .operational_registration()
                    .writer()
                    .execute(&request_b)
            },
        )
    });

    contended_receiver
        .recv_timeout(Duration::from_secs(2))
        .expect("the second public writer must contend on the shared lock");
    cancellation.cancel();
    let cancelled = thread_b
        .join()
        .expect("cancelled writer thread must not panic")
        .expect("lock-wait cancellation is a typed writer result");
    release.wait();
    let applied = thread_a
        .join()
        .expect("lock owner thread must not panic")
        .expect("lock owner must return a writer result");

    match cancelled.lifecycle() {
        WriterLifecycle::Cancelled(interruption) => {
            assert_eq!(
                interruption.cancellation(),
                PublicationCancellation::DuringExecution
            );
            assert_eq!(interruption.rollback(), PublicationRollback::NotNeeded);
        }
        lifecycle => panic!("expected typed cancellation, got {lifecycle:?}"),
    }
    assert!(matches!(applied.lifecycle(), WriterLifecycle::Applied));
    let descriptor = fs::read_to_string(output.join("Configuration.xml")).unwrap();
    assert!(
        descriptor.contains("<Name>LockOwner</Name>"),
        "{descriptor}"
    );
    assert!(!descriptor.contains("CancelledWaiter"), "{descriptor}");
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

    assert_eq!(
        location.location().owner().kind(),
        SemanticObjectKind::Catalog
    );
    assert_eq!(location.location().owner().name(), Some("Items"));
    assert_eq!(location.location().role(), ModuleRole::Object);
    assert!(matches!(
        location.location().owner(),
        ModuleOwner::Object { .. }
    ));

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
