use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use unica_adapter_platform_xml::PlatformXmlAdapterFactory;
use unica_format_core::{
    ports::{CaptureResult, OperationalValidationRequest, OwnerResolutionMode, ValidationOptions},
    source::{SourceContext, SourceFamily, SourceLocation, SourceRevision},
};

fn temp_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-task7-fix3-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn source(root: &Path, target: &Path) -> SourceContext {
    SourceContext::new(
        SourceLocation::new(root.to_path_buf(), root.to_path_buf(), target.to_path_buf()),
        Some("main".to_string()),
        SourceFamily::PlatformXml,
        None,
    )
}

fn write_configuration(root: &Path) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="00000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Catalog>Target</Catalog><Catalog>Unrelated</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
}

fn write_catalog(path: &Path, name: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="00000000-0000-0000-0000-000000000002"><Properties><Name>{name}</Name></Properties><ChildObjects/></Catalog></MetaDataObject>"#
        ),
    )
    .unwrap();
}

fn validation_report(root: &Path) -> unica_format_core::ports::ValidationReport {
    let target = root.join("Catalogs/Target.xml");
    let factory = PlatformXmlAdapterFactory::new();
    let session =
        factory.capture_validation_source(&source(root, &target), OwnerResolutionMode::Existing);
    factory
        .operational_registration()
        .validation()
        .validate(
            &OperationalValidationRequest::new(
                vec![session],
                ValidationOptions::new(true, 100).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .reports()[0]
        .clone()
}

fn validation_evidence(root: &Path) -> unica_format_core::ports::OperationalEvidenceRevision {
    let target = root.join("Catalogs/Target.xml");
    let factory = PlatformXmlAdapterFactory::new();
    let session =
        factory.capture_validation_source(&source(root, &target), OwnerResolutionMode::Existing);
    factory
        .operational_registration()
        .validation()
        .validate(
            &OperationalValidationRequest::new(
                vec![session],
                ValidationOptions::new(true, 100).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .evidence_revision()
        .clone()
}

#[test]
fn validation_never_opens_unrelated_registered_descriptors() {
    let baseline = temp_root("validation-baseline");
    write_configuration(&baseline);
    write_catalog(&baseline.join("Catalogs/Target.xml"), "Target");
    write_catalog(&baseline.join("Catalogs/Unrelated.xml"), "Unrelated");
    let expected = validation_report(&baseline);

    let malformed = temp_root("validation-malformed-unrelated");
    write_configuration(&malformed);
    write_catalog(&malformed.join("Catalogs/Target.xml"), "Target");
    fs::create_dir_all(malformed.join("Catalogs")).unwrap();
    fs::write(malformed.join("Catalogs/Unrelated.xml"), b"<broken").unwrap();
    assert_eq!(validation_report(&malformed), expected);

    let oversized = temp_root("validation-oversized-unrelated");
    write_configuration(&oversized);
    write_catalog(&oversized.join("Catalogs/Target.xml"), "Target");
    File::create(oversized.join("Catalogs/Unrelated.xml"))
        .unwrap()
        .set_len(70 * 1024 * 1024)
        .unwrap();
    assert_eq!(validation_report(&oversized), expected);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;

        let unreadable = temp_root("validation-unreadable-unrelated");
        let outside = temp_root("validation-outside");
        write_configuration(&unreadable);
        write_catalog(&unreadable.join("Catalogs/Target.xml"), "Target");
        fs::create_dir_all(unreadable.join("Catalogs")).unwrap();
        fs::write(&outside, b"outside bytes").unwrap();
        symlink(&outside, unreadable.join("Catalogs/Unrelated.xml")).unwrap();
        assert_eq!(validation_report(&unreadable), expected);
        fs::remove_dir_all(unreadable).unwrap();
        fs::remove_file(outside).unwrap();
    }

    fs::remove_dir_all(baseline).unwrap();
    fs::remove_dir_all(malformed).unwrap();
    fs::remove_dir_all(oversized).unwrap();
}

fn captured_revision(root: &Path, target: &Path) -> SourceRevision {
    let registration = PlatformXmlAdapterFactory::new().registration();
    let CaptureResult::Captured(captured) =
        registration.capture.capture(&source(root, target)).unwrap()
    else {
        panic!("fixture must be captured");
    };
    captured.snapshot().revision.clone()
}

#[test]
fn navigation_revision_covers_every_exposed_companion_but_not_unrelated_files() {
    let root = temp_root("navigation-revision");
    write_configuration(&root);
    let catalog = root.join("Catalogs/Target.xml");
    write_catalog(&catalog, "Target");

    let companions = [
        root.join("Catalogs/Target/Forms/Main/Ext/Form.xml"),
        root.join("Catalogs/Target/Templates/Print/Ext/Template.xml"),
        root.join("Catalogs/Target/Ext/ObjectModule.bsl"),
    ];
    for companion in &companions {
        fs::create_dir_all(companion.parent().unwrap()).unwrap();
        fs::write(companion, b"before").unwrap();
    }

    let initial = captured_revision(&root, &catalog);
    for companion in &companions {
        fs::write(companion, b"after").unwrap();
        let changed = captured_revision(&root, &catalog);
        assert_ne!(
            changed,
            initial,
            "companion {} must participate in the revision",
            companion.display()
        );
        fs::write(companion, b"before").unwrap();
    }

    fs::write(root.join("unrelated.bin"), b"first").unwrap();
    let before_unrelated_change = captured_revision(&root, &catalog);
    fs::write(root.join("unrelated.bin"), b"second").unwrap();
    let after_unrelated_change = captured_revision(&root, &catalog);
    assert_eq!(before_unrelated_change, after_unrelated_change);

    let role = root.join("Roles/Reader.xml");
    fs::create_dir_all(role.parent().unwrap()).unwrap();
    fs::write(
        &role,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="00000000-0000-0000-0000-000000000003"><Properties><Name>Reader</Name></Properties><ChildObjects/></Role></MetaDataObject>"#,
    )
    .unwrap();
    let rights = root.join("Roles/Reader/Ext/Rights.xml");
    fs::create_dir_all(rights.parent().unwrap()).unwrap();
    fs::write(&rights, b"<Rights>before</Rights>").unwrap();
    let before_rights = captured_revision(&root, &role);
    fs::write(&rights, b"<Rights>after</Rights>").unwrap();
    assert_ne!(captured_revision(&root, &role), before_rights);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn operational_evidence_changes_only_with_the_validation_read_set() {
    let root = temp_root("validation-evidence");
    write_configuration(&root);
    let target = root.join("Catalogs/Target.xml");
    write_catalog(&target, "Target");
    write_catalog(&root.join("Catalogs/Unrelated.xml"), "Unrelated");

    let initial = validation_evidence(&root);
    fs::write(root.join("outside-operation.bin"), b"changed").unwrap();
    assert_eq!(validation_evidence(&root), initial);

    write_catalog(&target, "ChangedTarget");
    assert_ne!(validation_evidence(&root), initial);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn direct_multi_register_validation_ignores_unrelated_registered_documents() {
    let root = temp_root("direct-register-records");
    for directory in [
        "Documents",
        "InformationRegisters",
        "AccumulationRegisters",
        "Languages",
    ] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(
        root.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration uuid="20000000-0000-0000-0000-000000000001"><Properties><Name>Main</Name></Properties><ChildObjects><Language>English</Language><Document>Target</Document><Document>Malformed</Document><Document>Oversized</Document><InformationRegister>Stock</InformationRegister><AccumulationRegister>Balance</AccumulationRegister></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(
        root.join("Languages/English.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="20000000-0000-0000-0000-000000000002"><Properties><Name>English</Name><LanguageCode>en</LanguageCode></Properties></Language></MetaDataObject>"#,
    )
    .unwrap();
    let target = root.join("Documents/Target.xml");
    fs::write(
        &target,
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Document uuid="20000000-0000-0000-0000-000000000003"><Properties><Name>Target</Name><RegisterRecords><xr:Item>InformationRegister.Stock</xr:Item><xr:Item>AccumulationRegister.Balance</xr:Item></RegisterRecords></Properties><ChildObjects/></Document></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(
        root.join("InformationRegisters/Stock.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><InformationRegister uuid="20000000-0000-0000-0000-000000000004"><Properties><Name>Stock</Name><WriteMode>RecorderSubordinate</WriteMode></Properties><ChildObjects/></InformationRegister></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(
        root.join("AccumulationRegisters/Balance.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><AccumulationRegister uuid="20000000-0000-0000-0000-000000000005"><Properties><Name>Balance</Name></Properties><ChildObjects/></AccumulationRegister></MetaDataObject>"#,
    )
    .unwrap();
    fs::write(root.join("Documents/Malformed.xml"), b"<broken").unwrap();
    File::create(root.join("Documents/Oversized.xml"))
        .unwrap()
        .set_len(70 * 1024 * 1024)
        .unwrap();

    let factory = PlatformXmlAdapterFactory::new();
    let context = factory
        .operational_registration()
        .validation_context()
        .inspect(&unica_format_core::ports::ValidationContextRequest::new(
            factory
                .capture_validation_source(&source(&root, &target), OwnerResolutionMode::Existing),
        ))
        .unwrap();
    let context = context.context().expect("direct reference context");
    assert_eq!(context.references_present(), Some(true));
    assert_eq!(context.registrar_present(), Some(true));

    fs::remove_dir_all(root).unwrap();
}
