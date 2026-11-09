use super::*;

#[test]
pub(crate) fn one_read_parses_each_metadata_descriptor_once_per_actor_revision() {
    let fixture = RealReaderFixture::new();
    let authority = fixture.read_authority();
    assert_reader_reaches(
        &authority,
        &[
            "main:Catalog.Items",
            "main:Catalog.Items.TabularSection.Lines",
            "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity",
        ],
    );
    // Owner proof reads the descriptor once and the typed projection once
    // more; every logical address projected from that owner shares the parse.
    assert_eq!(
        authority.metadata_descriptor_read_count("Catalog.Items"),
        2,
        "one actor-owned revision must parse each metadata descriptor once per authority",
    );
}

#[test]
pub(crate) fn typed_reads_parse_the_support_marker_once_per_actor_revision() {
    use crate::application::v13::view::ViewReadAuthority;
    let fixture = RealReaderFixture::new();
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        fixture.source.join("Ext/ParentConfigurations.bin"),
    )
    .unwrap();
    let authority = fixture.read_authority();
    // Three identities under two owners: every typed metadata read asks for
    // the owner's support state, the marker itself is parsed once.
    for at in [
        "main:Catalog.Items",
        "main:Catalog.Items.TabularSection.Lines",
        "main:Report.ParityReport",
    ] {
        let at = QualifiedAddress::parse(at).unwrap();
        let admitted = authority.snapshot(&at).unwrap();
        authority
            .read_exact(&at, &admitted)
            .unwrap_or_else(|error| panic!("{at}: {error:?}"));
    }
    assert_eq!(
        authority.support_state_read_count(),
        1,
        "one actor-owned revision must parse Ext/ParentConfigurations.bin once, not once per owner",
    );
}

/// A support marker replaced after an object read of the same operation
/// must not reach the configuration root, and the retained root must never
/// pair the new marker's stamp with the state read before it.
/// rule: INV.SOURCE.CONFIGURATION-ROOT-CACHE-REUSE
#[test]
fn replaced_support_marker_never_reaches_a_retained_root_with_the_old_state() {
    use crate::infrastructure::v13_large_configuration::RegistrationCache;

    let fixture = RealReaderFixture::new();
    let marker = fixture.source.join("Ext/ParentConfigurations.bin");
    fs::create_dir_all(marker.parent().unwrap()).unwrap();
    fs::copy(
        fixture_path("platform_8_3_27/support-edit-bin-only/src/Ext/ParentConfigurations.bin"),
        &marker,
    )
    .unwrap();
    let reader = |artifacts: Arc<RegistrationCache>| {
        ProviderReadAuthority::new(
            "main",
            "actor-fixture-main",
            SourceSetKind::Configuration,
            Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap()),
        )
        .with_artifacts(artifacts)
    };
    let fresh = || reader(Arc::new(RegistrationCache::default()));
    let supported = configuration_payload(&fresh()).unwrap()["support"].clone();

    let retained = Arc::new(RegistrationCache::with_settle_window_for_test(
        std::time::Duration::ZERO,
    ));
    let operation = reader(Arc::clone(&retained));
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    operation.object_support(&target, &mut || Ok(())).unwrap();
    fs::remove_file(&marker).unwrap();

    let expected = configuration_payload(&fresh()).unwrap()["support"].clone();
    assert_ne!(
        expected, supported,
        "the fixture must change the support answer"
    );
    assert_eq!(
        configuration_payload(&operation).unwrap()["support"],
        expected,
        "the root combined a removed marker with the state read before it"
    );
    assert_eq!(
        configuration_payload(&reader(retained)).unwrap()["support"],
        expected,
        "the retained root kept the support state of a replaced marker"
    );
}

#[test]
fn large_support_marker_reads_late_rules_once_for_multiple_owners() {
    use unica_metadata::support_state::ObjectSupportState;

    let fixture = RealReaderFixture::new();
    let catalog_uuid = crate::infrastructure::family_codecs::metadata_read_codec()
        .support_root_uuid(&fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap())
        .unwrap();
    let report_uuid = crate::infrastructure::family_codecs::metadata_read_codec()
        .support_root_uuid(&fs::read(fixture.source.join("Reports/ParityReport.xml")).unwrap())
        .unwrap();
    assert_ne!(catalog_uuid, report_uuid);
    let mut marker = format!("{{6,0,1,0,0,{catalog_uuid},").into_bytes();
    // The final rule straddles a 64 KiB window after the former 8 MiB ceiling.
    marker.resize(8 * 1024 * 1024 + 64 * 1024 - 13, b' ');
    marker.extend_from_slice(format!("1,0,{report_uuid},}}").as_bytes());
    fs::create_dir_all(fixture.source.join("Ext")).unwrap();
    fs::write(fixture.source.join("Ext/ParentConfigurations.bin"), marker).unwrap();
    let authority = fixture.read_authority();
    for (path, expected) in [
        ("Catalog.Items", ObjectSupportState::Locked),
        (
            "Report.ParityReport",
            ObjectSupportState::EditableWithSupport,
        ),
        ("Catalog.Items", ObjectSupportState::Locked),
    ] {
        let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path).unwrap();
        assert_eq!(
            authority
                .read
                .object_support(&target, &mut || Ok(()))
                .unwrap()
                .state,
            expected
        );
    }
    let configuration = configuration_payload(&authority.read).unwrap();
    assert_eq!(
        configuration["support"]["objects"],
        json!({"locked": 1, "editable": 1, "removed": 0})
    );
    assert_eq!(authority.support_state_read_count(), 1);
}

#[test]
fn support_marker_stream_preserves_terminal_errors_and_never_memoizes_interruption() {
    use unica_execution_contract::view_error::ViewError;
    let fixture = RealReaderFixture::new();
    let marker_path = fixture.source.join("Ext/ParentConfigurations.bin");
    fs::create_dir_all(marker_path.parent().unwrap()).unwrap();
    let mut marker = b"{6,0,1,".to_vec();
    marker.resize(64 * 1024 + 1, b' ');
    fs::write(&marker_path, marker).unwrap();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        // Before opening, between chunks, after EOF, and immediately before memo publication.
        for stop_at in [1, 3, 5, 6] {
            let authority = fixture.read_authority();
            let mut calls = 0;
            let error = authority
                .read
                .object_support(&target, &mut || {
                    calls += 1;
                    if calls == stop_at {
                        Err(ViewError::new(code, "original operation stopped"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code, "checkpoint {stop_at}");
            assert_eq!(calls, stop_at);
            let reads_before_retry = authority.support_state_read_count();
            authority
                .read
                .object_support(&target, &mut || Ok(()))
                .unwrap();
            assert_eq!(
                authority.support_state_read_count(),
                reads_before_retry + 1,
                "interrupted state was not published"
            );
            let mut calls = 0;
            let error = authority
                .read
                .object_support(&target, &mut || {
                    calls += 1;
                    if calls == 2 {
                        Err(ViewError::new(code, "cached read stopped"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code);
            assert_eq!(calls, 2, "cached return keeps the caller checkpoint");
            assert_eq!(authority.support_state_read_count(), reads_before_retry + 1);
        }
    }
    fs::remove_file(marker_path).unwrap();
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        let authority = fixture.read_authority();
        let mut calls = 0;
        let error = authority
            .read
            .object_support(&target, &mut || {
                calls += 1;
                if calls == 2 {
                    Err(ViewError::new(code, "absent marker read stopped"))
                } else {
                    Ok(())
                }
            })
            .unwrap_err();
        assert_eq!(error.code(), code);
        authority
            .read
            .object_support(&target, &mut || Ok(()))
            .unwrap();
        assert_eq!(
            authority.support_state_read_count(),
            0,
            "an absent marker has nothing to parse"
        );
    }
}
