use super::*;

#[test]
fn dcs_validation_input_keeps_the_authority_cancellation_and_deadline() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let mut authority = fixture.read_authority();
    authority.deadline = ProviderDeadline::from_budget(std::time::Duration::ZERO);
    assert_eq!(
        authority
            .dcs_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::DeadlineExceeded
    );
    let authority = fixture.read_authority();
    fixture.cancellation.cancel();
    assert_eq!(
        authority
            .dcs_validation_input(&address)
            .err()
            .unwrap()
            .code(),
        RefusalCode::Cancelled
    );
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_dcs_validation_keeps_captured_format_evidence_after_replacement() {
    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let authority = fixture.read_authority();
    let input = authority.dcs_validation_input(&address).unwrap();
    let wrapper = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema.xml");
    let original = fs::read_to_string(&wrapper).unwrap();
    fs::write(&wrapper, original.replace("2.20", "2.21")).unwrap();
    fs::write(&input.artifact, "not XML: a later writer owns these bytes").unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(
        checked.diagnostics().iter().all(|diagnostic| !matches!(
            diagnostic.code(),
            "formatVersionInvalid" | "platformVersionUnsupported"
        )),
        "format diagnostics must describe the captured input: {:?}",
        checked.diagnostics()
    );
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn canonical_dcs_validation_keeps_captured_owner_format_warnings() {
    for relative in ["Configuration.xml"] {
        for (version, expected) in [
            ("2.21", "platformVersionUnsupported"),
            ("2.19", "formatMigrationAvailable"),
            ("2.&#50;0", "formatVersionInvalid"),
        ] {
            let fixture = RealReaderFixture::new();
            let owner = fixture.source.join(relative);
            let original = fs::read_to_string(&owner).unwrap();
            fs::write(&owner, original.replace("2.20", version)).unwrap();
            let address =
                QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
            let captured = fixture.read_authority().dcs_validation_input(&address);
            let input = captured.unwrap();
            fs::write(&owner, original).unwrap();
            let checked = crate::application::v13::check::normalize_native_outcome(
                &address,
                "Template",
                crate::application::v13::check::CheckValidator::Dcs,
                crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
            )
            .unwrap();
            assert_eq!(
                checked.diagnostics()[0].code(),
                expected,
                "{relative}: {version}"
            );
        }
    }

    let fixture = RealExternalReaderFixture::new();
    let template_owner =
        crate_fixture_text("template-remove/ParityReport/Templates/MainSchema.xml");
    let dcs = fixture_text("unica_mcp_script_parity/dcs-validate/BadPrefix.xml").replace(
        "xmlns:bad=\"http://example.com\">bad:CatalogRef.X",
        ">xs:string",
    );
    for (source_set, kind, object_kind, name, source) in [
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport",
            "Sales",
            &fixture.report,
        ),
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor",
            "Import",
            &fixture.processor,
        ),
    ] {
        let owner = source.join(format!("{name}.xml"));
        let original = fs::read_to_string(&owner).unwrap().replace(
            "</ChildObjects>",
            "<Template>MainSchema</Template></ChildObjects>",
        );
        write(
            &source.join(format!("{name}/Templates/MainSchema.xml")),
            &template_owner,
        );
        write(
            &source.join(format!("{name}/Templates/MainSchema/Ext/Template.xml")),
            &dcs,
        );
        // Only the external root has the older format; its Template stays 2.20.
        fs::write(
            &owner,
            original.replace("version=\"2.20\"", "version=\"2.19\""),
        )
        .unwrap();
        let address = QualifiedAddress::parse(&format!(
            "{source_set}:{object_kind}.{name}.Template.MainSchema"
        ))
        .unwrap();
        let input = fixture
            .read_authority(source_set, kind)
            .dcs_validation_input(&address)
            .unwrap();
        fs::write(&owner, &original).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Template",
            crate::application::v13::check::CheckValidator::Dcs,
            crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
        )
        .unwrap();
        assert_eq!(
            checked.diagnostics()[0].code(),
            "formatMigrationAvailable",
            "{source_set}: restored owner bytes must not replace captured format evidence"
        );
        assert_eq!(fs::read_to_string(&owner).unwrap(), original);
    }
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_dcs_validation_does_not_follow_links_after_input_capture() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    let fixture = RealReaderFixture::new();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let outside = fixture.root.path().join("foreign.xml");
    fs::write(&outside, "not XML: foreign input must never participate").unwrap();
    for path in [
        input.artifact.clone(),
        fixture.source.join("Configuration.xml"),
        fixture
            .source
            .join("Reports/ParityReport/Templates/MainSchema.xml"),
    ] {
        fs::remove_file(&path).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &path)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
    }
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(checked
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.code() != "formatVersionInvalid"));
    assert_eq!(
        fs::read_to_string(outside).unwrap(),
        "not XML: foreign input must never participate"
    );
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_dcs_validation_refuses_linked_format_owners_before_capture() {
    use unica_workspace::test_support::{
        create_file_link_fixture_for_test, FileLinkFixtureOutcome,
    };

    for relative in [
        "Configuration.xml",
        "Reports/ParityReport/Templates/MainSchema.xml",
    ] {
        let fixture = RealReaderFixture::new();
        let owner = fixture.source.join(relative);
        let outside = fixture.root.path().join("foreign.xml");
        fs::rename(&owner, &outside).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &owner)
            .expect("unexpected file-link creation error must fail the fixture test");
        if outcome != FileLinkFixtureOutcome::Created {
            unica_process::test_support::skip_without_host_capability(
                unica_process::test_support::HostCapability::Symlink,
                "creating a link fixture needs the symlink privilege",
            );
            return;
        }
        let address =
            QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
        assert!(
            fixture
                .read_authority()
                .dcs_validation_input(&address)
                .is_err(),
            "{relative}"
        );
    }
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn canonical_dcs_validation_keeps_large_configuration_owner_format_warning() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Configuration.xml");
    let xml = fs::read_to_string(&path).unwrap().replace("2.20", "2.21");
    fs::write(
        &path,
        xml.replace(
            "</MetaDataObject>",
            &format!("<!--{}--></MetaDataObject>", "x".repeat(8 * 1024 * 1024)),
        ),
    )
    .unwrap();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert_eq!(
        checked.diagnostics()[0].code(),
        "platformVersionUnsupported"
    );
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn canonical_dcs_validation_keeps_exact_versionless_dcs_root_guard() {
    let fixture = RealReaderFixture::new();
    let artifact = fixture
        .source
        .join("Reports/ParityReport/Templates/MainSchema/Ext/Template.xml");
    fs::write(&artifact, r#"<DataCompositionSchema xmlns="http://v8.1c.ru/8.1/data-composition-system/schema" version="2.20"/>"#).unwrap();
    let address = QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
    let input = fixture
        .read_authority()
        .dcs_validation_input(&address)
        .unwrap();
    let checked = crate::application::v13::check::normalize_native_outcome(
        &address,
        "Template",
        crate::application::v13::check::CheckValidator::Dcs,
        crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
    )
    .unwrap();
    assert!(
        checked
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "formatVersionInvalid"),
        "versionless DCS publication policy must survive the retained-input bridge"
    );
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
#[test]
fn canonical_dcs_validation_reports_readable_template_owner_versions() {
    for (version, expected) in [
        (Some("2.19"), "formatMigrationAvailable"),
        (Some("2.21"), "platformVersionUnsupported"),
        (None, "formatMigrationAvailable"),
    ] {
        let fixture = RealReaderFixture::new();
        let owner = fixture
            .source
            .join("Reports/ParityReport/Templates/MainSchema.xml");
        let original = fs::read_to_string(&owner).unwrap();
        let xml = original.replace(
            "version=\"2.20\"",
            &version
                .map(|v| format!("version=\"{v}\""))
                .unwrap_or_default(),
        );
        fs::write(&owner, &xml).unwrap();
        let address =
            QualifiedAddress::parse("main:Report.ParityReport.Template.MainSchema").unwrap();
        let input = fixture
            .read_authority()
            .dcs_validation_input(&address)
            .unwrap();
        // Format evidence belongs to the captured input, not a later reread.
        fs::write(&owner, &original).unwrap();
        let checked = crate::application::v13::check::normalize_native_outcome(
            &address,
            "Template",
            crate::application::v13::check::CheckValidator::Dcs,
            crate::infrastructure::native_operations::v13_analysis::validate_dcs_input(input),
        )
        .unwrap();
        assert_eq!(checked.diagnostics()[0].code(), expected);
        assert_eq!(fs::read_to_string(&owner).unwrap(), original);
    }
}

/// rule: INV.SURFACE.CHECK-INFERS-VALIDATORS
#[test]
fn canonical_dcs_format_owner_read_stops_at_typed_midstream_checkpoint() {
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        let fixture = RealReaderFixture::new();
        let owner = fixture.source.join("Configuration.xml");
        let original = fs::read_to_string(&owner).unwrap();
        fs::write(
            &owner,
            original.replace(
                "</MetaDataObject>",
                &format!("<!--{}--></MetaDataObject>", "x".repeat(256 * 1024)),
            ),
        )
        .unwrap();
        let authority = fixture.read_authority();
        let target = unica_address::source_target::MetadataAddress::parse(
            unica_address::source_target::PLATFORM_XML_8_3_27_FORMAT_2_20,
            "Report.ParityReport.Template.MainSchema",
        )
        .unwrap();
        let mut checkpoints = 0;
        let error = authority
            .read
            .dcs_validation_input(&target, &mut || {
                checkpoints += 1;
                if checkpoints == 2 {
                    Err(ViewError::new(
                        code,
                        "interrupted after the first owner chunk",
                    ))
                } else {
                    Ok(())
                }
            })
            .err()
            .unwrap();
        assert_eq!(error.code(), code);
        assert_eq!(checkpoints, 2, "read must stop at the rejecting checkpoint");
        assert!(
            authority
                .read
                .dcs_validation_input(&target, &mut || Ok(()))
                .is_ok(),
            "partial capture must not be published or cached"
        );
    }
}
