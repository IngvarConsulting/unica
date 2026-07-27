use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Map, Value};
use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    navigation::Authorability,
    ports::{
        AuthorabilityRequest, AuthorabilityRequirement, CompatibilityIssueKind,
        CompatibilityRequest, FormatDiagnosticCode, OperationCancellation, OwnerResolutionMode,
        OperationalValidationRequest, PublicationCancellation, PublicationInvocation,
        PublicationRequest, PublicationStatus, SupportState, ValidationContextRequest,
        ValidationFindingCode, ValidationIssueKind, ValidationOptions,
    },
    source::{SourceContext, SourceFamily, SourceLocation},
};

fn source(root: &Path, target: &Path, family: SourceFamily) -> SourceContext {
    SourceContext::new(
        SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        family,
        None,
    )
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task7-fix1-{label}-{}-{nanos}",
        std::process::id()
    ))
}

fn write_owner(root: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(root).unwrap();
    let owner = root.join("Configuration.xml");
    fs::write(
        &owner,
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="{version}"><Configuration uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Configuration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#
        ),
    )
    .unwrap();
    owner
}

fn capture(root: &Path, target: &Path) -> unica_format_core::ports::OperationalSourceSession {
    PlatformXmlAdapterFactory::new().capture_operational_source(
        &source(root, target, SourceFamily::PlatformXml),
        OwnerResolutionMode::Existing,
    )
}

#[test]
fn task7_compatibility_port_classifies_supported_older_newer_and_malformed_profiles() {
    let factory = PlatformXmlAdapterFactory::new();
    let operations = factory.operational_registration();

    for (version, expected) in [
        ("2.20", None),
        ("2.19", Some(CompatibilityIssueKind::Older)),
        ("2.21", Some(CompatibilityIssueKind::Newer)),
        ("latest", Some(CompatibilityIssueKind::Malformed)),
    ] {
        let root = temp_root(version);
        let owner = write_owner(&root, version);
        let result = operations
            .compatibility()
            .inspect(
                &CompatibilityRequest::new(vec![capture(&root, &owner)]).unwrap(),
            )
            .unwrap();
        assert_eq!(result.issue().map(|issue| issue.kind()), expected, "{version}");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn task7_authorability_distinguishes_absent_support_from_unreadable_support() {
    let root = temp_root("support-states");
    let owner = write_owner(&root, "2.20");
    let operations = PlatformXmlAdapterFactory::new().operational_registration();

    let absent = operations
        .authorability()
        .inspect(&AuthorabilityRequest::new(
            capture(&root, &owner),
            AuthorabilityRequirement::Editable,
        ))
        .unwrap();
    assert_eq!(absent.summary().state(), SupportState::Absent);
    assert_eq!(absent.authorability(), Authorability::Authorable);
    assert!(absent.is_allowed());

    fs::create_dir_all(root.join("Ext")).unwrap();
    fs::write(root.join("Ext/ParentConfigurations.bin"), "<broken-support").unwrap();
    let unreadable = operations
        .authorability()
        .inspect(&AuthorabilityRequest::new(
            capture(&root, &owner),
            AuthorabilityRequirement::Editable,
        ))
        .unwrap();
    assert_eq!(unreadable.summary().state(), SupportState::Unreadable);
    assert_eq!(
        unreadable.denial().unwrap().diagnostic().code(),
        FormatDiagnosticCode::SupportStateUnreadable
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_authorability_binds_absent_and_present_support_evidence_across_races() {
    let root = temp_root("support-races");
    let owner = write_owner(&root, "2.20");
    let factory = PlatformXmlAdapterFactory::new();
    let operations = factory.operational_registration();
    let session = capture(&root, &owner);
    let inspect = |session| {
        operations
            .authorability()
            .inspect(&AuthorabilityRequest::new(
                session,
                AuthorabilityRequirement::Editable,
            ))
            .unwrap()
    };

    assert_eq!(inspect(session.clone()).summary().state(), SupportState::Absent);
    fs::create_dir_all(root.join("Ext")).unwrap();
    let support = "{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",1,1,0,00000000-0000-0000-0000-000000000001}";
    fs::write(root.join("Ext/ParentConfigurations.bin"), support).unwrap();
    let appeared = inspect(session);
    assert_eq!(appeared.summary().state(), SupportState::Unreadable);
    assert_eq!(
        appeared.denial().unwrap().diagnostic().code(),
        FormatDiagnosticCode::SupportStateUnreadable
    );

    let present_session = capture(&root, &owner);
    assert_ne!(
        inspect(present_session.clone()).summary().state(),
        SupportState::Unreadable
    );
    fs::write(
        root.join("Ext/ParentConfigurations.bin"),
        support.replace("VendorConf", "ChangedConf"),
    )
    .unwrap();
    let changed = inspect(present_session);
    assert_eq!(changed.summary().state(), SupportState::Unreadable);
    assert_eq!(
        changed.denial().unwrap().diagnostic().code(),
        FormatDiagnosticCode::SupportStateUnreadable
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_authorability_fails_closed_for_capture_containment_family_and_read_errors() {
    let root = temp_root("support-errors");
    let owner = write_owner(&root, "2.20");
    let outside = temp_root("outside");
    let outside_owner = write_owner(&outside, "2.20");
    let factory = PlatformXmlAdapterFactory::new();
    let operations = factory.operational_registration();

    let mut sessions = vec![
        factory.capture_operational_source(
            &source(&root, &owner, SourceFamily::Edt),
            OwnerResolutionMode::Existing,
        ),
        factory.capture_operational_source(
            &source(&root, &outside_owner, SourceFamily::PlatformXml),
            OwnerResolutionMode::Existing,
        ),
    ];
    fs::create_dir_all(root.join("Ext/ParentConfigurations.bin")).unwrap();
    sessions.push(capture(&root, &owner));

    for session in sessions {
        let result = operations
            .authorability()
            .inspect(&AuthorabilityRequest::new(
                session,
                AuthorabilityRequirement::Editable,
            ))
            .unwrap();
        assert_eq!(result.summary().state(), SupportState::Unreadable);
        assert_eq!(
            result.denial().unwrap().diagnostic().code(),
            FormatDiagnosticCode::SupportStateUnreadable
        );
    }

    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn task7_authorability_fails_closed_when_present_support_cannot_bind_to_target() {
    let root = temp_root("support-target-inspection");
    let owner = write_owner(&root, "2.20");
    fs::create_dir_all(root.join("Ext")).unwrap();
    fs::write(
        root.join("Ext/ParentConfigurations.bin"),
        "{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",\"VendorConf\",1,1,0,00000000-0000-0000-0000-000000000001}",
    )
    .unwrap();
    fs::write(
        &owner,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Configuration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();

    let result = PlatformXmlAdapterFactory::new()
        .operational_registration()
        .authorability()
        .inspect(&AuthorabilityRequest::new(
            capture(&root, &owner),
            AuthorabilityRequirement::Editable,
        ))
        .unwrap();

    assert_eq!(result.summary().state(), SupportState::Unreadable);
    assert_eq!(
        result.denial().unwrap().diagnostic().code(),
        FormatDiagnosticCode::SupportStateUnreadable
    );
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn task7_authorability_rejects_symlinked_support_evidence() {
    use std::os::unix::fs::symlink;

    let root = temp_root("support-symlink");
    let owner = write_owner(&root, "2.20");
    let outside = temp_root("support-symlink-outside");
    fs::create_dir_all(root.join("Ext")).unwrap();
    fs::write(&outside, b"support").unwrap();
    symlink(&outside, root.join("Ext/ParentConfigurations.bin")).unwrap();

    let operations = PlatformXmlAdapterFactory::new().operational_registration();
    let result = operations
        .authorability()
        .inspect(&AuthorabilityRequest::new(
            capture(&root, &owner),
            AuthorabilityRequirement::Editable,
        ))
        .unwrap();
    assert_eq!(result.summary().state(), SupportState::Unreadable);

    fs::remove_dir_all(root).unwrap();
    fs::remove_file(outside).unwrap();
}

#[test]
fn task7_validation_context_returns_closed_diagnostic_for_malformed_metadata() {
    let root = temp_root("validation-malformed");
    fs::create_dir_all(&root).unwrap();
    let object = root.join("Broken.xml");
    fs::write(&object, "<MetaDataObject").unwrap();
    let operations = PlatformXmlAdapterFactory::new().operational_registration();

    let result = operations
        .validation_context()
        .inspect(&ValidationContextRequest::new(capture(&root, &object)))
        .unwrap();

    assert!(result.context().is_none());
    assert_eq!(result.diagnostics().len(), 1);
    assert_eq!(
        result.diagnostics()[0].detail(),
        unica_format_core::ports::FormatDiagnosticDetail::Validation(
            ValidationIssueKind::SourceUnreadable
        )
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_operational_validation_uses_private_registry_value_constraints() {
    let root = temp_root("validation-values");
    fs::create_dir_all(&root).unwrap();
    let object = root.join("Items.xml");
    fs::write(
        &object,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000002"><Properties><Name>Items</Name><Hierarchical>not-a-boolean</Hierarchical><CodeType>not-an-enum-value</CodeType></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
    )
    .unwrap();
    let factory = PlatformXmlAdapterFactory::new();
    let session = factory.capture_validation_source(
        &source(&root, &object, SourceFamily::PlatformXml),
        OwnerResolutionMode::Existing,
    );
    let result = factory
        .operational_registration()
        .validation()
        .validate(
            &OperationalValidationRequest::new(
                vec![session],
                ValidationOptions::new(true, 30).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

    assert!(result.reports()[0]
        .findings()
        .iter()
        .any(|finding| finding.code() == ValidationFindingCode::SemanticValueInvalid));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn task7_publication_cancellation_is_explicit_and_public_result_is_path_free() {
    let root = temp_root("publication-cancel");
    fs::create_dir_all(&root).unwrap();
    let cancellation = OperationCancellation::new();
    cancellation.cancel();
    let called = Arc::new(AtomicBool::new(false));
    let called_by_runner = called.clone();
    let args = Map::from_iter([
        ("config".to_string(), json!("/private/unix/Configuration.xml")),
        ("workdir".to_string(), json!(r"C:\private\workspace")),
        ("nativeTag".to_string(), Value::String("MetaDataObject".to_string())),
    ]);
    let factory = PlatformXmlAdapterFactory::new();
    let session = factory.capture_publication_session(
        "alternate.publish",
        &args,
        &root,
        &root,
        move |_, _, _, _, _| {
            called_by_runner.store(true, Ordering::SeqCst);
            Err("runner must not be called".to_string())
        },
        |_, _, _| Err("resolver must not be called".to_string()),
        |_, _| Err("lock must not be called".to_string()),
    );
    let result = factory
        .operational_registration()
        .publication()
        .publish(&PublicationRequest::new(
            session,
            PublicationInvocation::BuildDump,
            cancellation,
        ))
        .unwrap();

    assert_eq!(result.status(), PublicationStatus::Cancelled);
    assert_eq!(
        result.cancellation(),
        PublicationCancellation::BeforeExecution
    );
    assert!(!called.load(Ordering::SeqCst));
    let public = format!("{result:?}");
    for forbidden in [
        root.to_string_lossy().as_ref(),
        "/private/unix",
        r"C:\private\workspace",
        "Configuration.xml",
        "MetaDataObject",
        "2.20",
        "8.3.27",
    ] {
        assert!(!public.contains(forbidden), "leaked {forbidden}: {public}");
    }
    fs::remove_dir_all(root).unwrap();
}
