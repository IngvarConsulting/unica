use super::*;

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn real_external_sources_are_traversable_without_configuration_xml_and_hide_root_runtime_modules() {
    let fixture = RealExternalReaderFixture::new();
    for (source, kind, owner) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor.Import",
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport.Sales",
        ),
    ] {
        let authority = fixture.read_authority(source, kind);
        let service = ViewService::new(authority, ViewCursorStore::default());
        let address = format!("{source}:Configuration");
        let root = service.view(ViewRequest::new(&address).unwrap());
        assert!(root.ok, "{source} root failed: {:?}", root.diagnostics);
        let root = root.data.unwrap();
        let owner_branch = root["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|branch| branch["at"] == format!("{source}:{}", owner.split('.').next().unwrap()))
            .unwrap();
        assert_eq!(owner_branch["count"], 2);
        assert!(
            root["branches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| branch["at"] != format!("{source}:Module")),
            "external source root exposed configuration runtime modules: {root:?}",
        );
    }

    let epf = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let erf = fixture.read_authority("artifact_report", SourceSetKind::ExternalReport);
    let index = ReaderReach::new(vec![
        ("artifact_processor", &epf),
        ("artifact_report", &erf),
    ]);
    for expected in [
        "artifact_processor:ExternalDataProcessor.Import",
        "artifact_processor:ExternalDataProcessor.Import.Form.Main",
        "artifact_processor:ExternalDataProcessor.Import.Command.Run",
        "artifact_processor:ExternalDataProcessor.Import.Module.Object",
        "artifact_report:ExternalReport.Sales",
        "artifact_report:ExternalReport.Sales.Form.Main",
        "artifact_report:ExternalReport.Sales.Command.Run",
        "artifact_report:ExternalReport.Sales.Module.Object",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap().with_limit(64).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "missing real external identity {expected}: {found:?}",
        );
    }

    // The export-path half of this contract is proven against the find
    // directory in `v13_find`, where a candidate carries its real reason;
    // through the reader probe every candidate reads "reader".
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inventory_skips_runtime_sidecar_and_fails_closed_on_malformed_or_ambiguous_owner() {
    let malformed_fixture = RealExternalReaderFixture::new();
    fs::write(malformed_fixture.processor.join("Broken.xml"), "<broken>").unwrap();
    let malformed = ViewService::new(
        malformed_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!malformed.ok);
    assert_eq!(malformed.diagnostics[0]["code"], "provider_unavailable");

    let ambiguous_fixture = RealExternalReaderFixture::new();
    fs::copy(
        ambiguous_fixture.processor.join("Import.xml"),
        ambiguous_fixture.processor.join("Alias.xml"),
    )
    .unwrap();
    let ambiguous = ViewService::new(
        ambiguous_fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor),
        ViewCursorStore::default(),
    )
    .view(ViewRequest::new("artifact_processor:Configuration").unwrap());
    assert!(!ambiguous.ok);
    assert_eq!(ambiguous.diagnostics[0]["code"], "provider_unavailable");
    assert!(ambiguous.diagnostics[0]["message"]
        .as_str()
        .unwrap()
        .contains("ambiguous"));
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn retained_external_inventory_is_cancellable() {
    let cancelled_fixture = RealExternalReaderFixture::new();
    let root = Arc::new(
        unica_workspace::read::open_source_directory(&cancelled_fixture.processor).unwrap(),
    );
    let reader = ProviderReadAuthority::new(
        "artifact_processor",
        "actor-external-cancel",
        SourceSetKind::ExternalProcessor,
        root,
    );
    let cancellation = CancellationToken::new();
    let mut checkpoints = 0_usize;
    let error = reader
        .configuration_payload_with_checkpoint(&mut || {
            checkpoints += 1;
            if checkpoints == 8 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(unica_execution_contract::view_error::ViewError::new(
                    RefusalCode::Cancelled,
                    "external inventory cancelled",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code().as_str(), "cancelled");
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn retained_external_inventory_crosses_former_aggregate_bytes_without_losing_owners() {
    let fixture = large_external_inventory_fixture();
    let reader = external_inventory_reader(&fixture);
    let payload =
        configuration_payload(&reader).expect("all sequential descriptors remain readable");
    assert_complete_large_external_inventory(&payload);
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn retained_external_inventory_reads_malformed_tail_after_former_aggregate_bytes() {
    let fixture = large_external_inventory_fixture();
    // Retained names are sorted. The five Large*.xml files precede this entry
    // and contain more than 32 MiB in total.
    fs::write(fixture.processor.join("ZZBroken.xml"), "<broken>").unwrap();
    let reader = external_inventory_reader(&fixture);
    let error = configuration_payload(&reader).unwrap_err();
    assert_eq!(error.code().as_str(), "provider_unavailable");
    assert!(error.to_string().contains("ZZBroken.xml"), "{error}");
    assert!(!error.to_string().contains("read limit"), "{error}");
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn retained_external_inventory_cancels_inside_a_large_descriptor_without_publishing_prefix() {
    // This stays below the former aggregate limit, so its RED cannot be
    // masked by an unrelated size refusal.
    let fixture = RealExternalReaderFixture::new();
    add_large_external_descriptors(&fixture, 1);
    assert!(fs::read_dir(&fixture.processor).unwrap().count() < 32);
    let reader = external_inventory_reader(&fixture);
    let mut checkpoints = 0;
    let error = reader
        .configuration_payload_with_checkpoint(&mut || {
            checkpoints += 1;
            // Enumeration and the preceding small descriptors use fewer calls;
            // the first 7-MiB descriptor needs more than 100 actual read chunks.
            if checkpoints == 64 {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "cancel inside retained XML read",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(error.code(), RefusalCode::Cancelled, "{error}");
    assert_eq!(checkpoints, 64);
    let payload =
        configuration_payload(&reader).expect("cancel did not publish a partial inventory");
    assert_eq!(
        payload["registeredObjects"],
        json!([
            {"kind": "ExternalDataProcessor", "name": "Import"},
            {"kind": "ExternalDataProcessor", "name": "Large0"},
            {"kind": "ExternalDataProcessor", "name": "Импорт"}
        ])
    );
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn external_parent_childobjects_are_the_only_nested_owner_authority() {
    let fixture = RealExternalReaderFixture::new();
    write(
        &fixture.processor.join("Импорт/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Orphan</Name></Properties></Form></MetaDataObject>"#,
    );
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let orphan = service.view(
        ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт.Form.Orphan").unwrap(),
    );
    assert!(!orphan.ok, "orphan external form became addressable");
    assert_eq!(orphan.diagnostics[0]["code"], "not_found");

    let descriptor = fixture.processor.join("Импорт.xml");
    let owner = fs::read_to_string(&descriptor).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Missing</Form></ChildObjects>",
        1,
    );
    fs::write(descriptor, owner).unwrap();
    let authority = fixture.read_authority("artifact_processor", SourceSetKind::ExternalProcessor);
    let service = ViewService::new(authority, ViewCursorStore::default());
    let parent =
        service.view(ViewRequest::new("artifact_processor:ExternalDataProcessor.Импорт").unwrap());
    assert!(
        !parent.ok,
        "external parent published a missing registered child"
    );
    assert_eq!(parent.diagnostics[0]["code"], "provider_unavailable");
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inventory_accepts_a_single_owner_descriptor_beyond_eight_mib() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        let original = external_leaf_descriptor(xml_kind, "Large", 9 * 1024 * 1024);
        fs::write(source.join("Large.xml"), &original).unwrap();
        let payload = configuration_payload(&external_leaf_read(&source, kind))
            .expect("valid external owner must not fail the old per-file cap");
        assert_eq!(
            payload["registeredObjects"],
            json!([{"kind":xml_kind,"name":"Large"}])
        );
        assert_eq!(payload["format"], "PlatformXml");
        assert_eq!(payload["name"], "artifact");
        assert_eq!(
            fs::read_to_string(source.join("Large.xml")).unwrap(),
            original
        );
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn medium_named_external_owner_view_has_no_hidden_eight_mib_descriptor_fallback() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        fs::write(
            source.join("Large.xml"),
            external_leaf_descriptor(xml_kind, "Large", 9 * 1024 * 1024),
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, kind),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let at = format!("artifact:{xml_kind}.Large");
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "large external named view refused: {result:?}");
        let node = result.data.unwrap();
        assert_eq!(node["at"], at);
        assert_eq!(node["kind"], xml_kind);
        assert_eq!(node["title"], "Large");
        assert_eq!(node["props"]["kind"], xml_kind);
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inventory_retains_all_257_immediate_owners_and_the_last_named_view() {
    for (kind, xml_kind) in [
        (SourceSetKind::ExternalProcessor, "ExternalDataProcessor"),
        (SourceSetKind::ExternalReport, "ExternalReport"),
    ] {
        let (_workspace, source) = external_leaf_fixture(kind);
        let mut expected = Vec::new();
        for index in 0..257 {
            let name = format!("Owner{index:03}");
            fs::write(
                source.join(format!("{name}.xml")),
                external_leaf_descriptor(xml_kind, &name, 0),
            )
            .unwrap();
            expected.push(json!({"kind":xml_kind,"name":name}));
        }
        let payload = configuration_payload(&external_leaf_read(&source, kind))
            .expect("all immediate owner names must be inspected without a count quota");
        assert_eq!(
            payload["registeredObjects"],
            serde_json::Value::Array(expected)
        );
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, kind),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let at = format!("artifact:{xml_kind}.Owner256");
        let viewed = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new(&at).unwrap());
        assert!(viewed.ok, "last real owner was lost: {viewed:?}");
        assert_eq!(viewed.data.as_ref().unwrap()["at"], at);
        assert_eq!(viewed.data.as_ref().unwrap()["title"], "Owner256");
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inventory_does_not_limit_noise_before_a_valid_owner() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    for index in 0..257 {
        fs::write(source.join(format!("noise{index:03}.txt")), "unrelated").unwrap();
    }
    fs::write(
        source.join("Useful.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Useful", 0),
    )
    .unwrap();
    let payload = configuration_payload(&external_leaf_read(
        &source,
        SourceSetKind::ExternalProcessor,
    ))
    .unwrap();
    assert_eq!(
        payload["registeredObjects"],
        json!([{"kind":"ExternalDataProcessor","name":"Useful"}])
    );
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_inventory_rejects_malformed_and_non_utf8_late_tails_without_partial_rows() {
    for invalid_utf8 in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        fs::write(
            source.join("AGood.xml"),
            external_leaf_descriptor("ExternalDataProcessor", "AGood", 0),
        )
        .unwrap();
        let mut bytes =
            external_leaf_descriptor("ExternalDataProcessor", "ZLarge", 9 * 1024 * 1024)
                .into_bytes();
        if invalid_utf8 {
            bytes.push(0xff);
        } else {
            bytes.extend_from_slice(b"<unfinished>");
        }
        fs::write(source.join("ZLarge.xml"), &bytes).unwrap();
        let error = configuration_payload(&external_leaf_read(
            &source,
            SourceSetKind::ExternalProcessor,
        ))
        .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable);
        assert!(
            !error.to_string().contains("read limit"),
            "must reach actual full parser beyond former cap: {error}"
        );
        let cancellation = CancellationToken::new();
        let authority = LogicalViewReadAuthority::with_read_authority(
            &cancellation,
            external_leaf_read(&source, SourceSetKind::ExternalProcessor),
            PlatformProfile::v8_3_27(),
            ProviderDeadline::no_deadline(),
        );
        let result = ViewService::new(authority, ViewCursorStore::default())
            .view(ViewRequest::new("artifact:Configuration").unwrap());
        assert!(!result.ok);
        assert!(
            result.data.is_none(),
            "malformed late descriptor cannot publish earlier partial rows"
        );
        assert_eq!(fs::read(source.join("ZLarge.xml")).unwrap(), bytes);
    }
}

#[test]
fn external_inventory_preserves_explicit_cancellation_during_a_large_owner_read() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Large.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let mut checks = 0;
    let error = external_leaf_read(&source, SourceSetKind::ExternalProcessor)
        .configuration_payload_with_checkpoint(&mut || {
            checks += 1;
            if checks == 16 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "manual external cancellation",
                ))
            } else {
                Ok(())
            }
        })
        .unwrap_err();
    assert_eq!(checks, 16);
    assert_eq!(error.code(), RefusalCode::Cancelled);
}

#[test]
fn named_external_view_rejects_replaced_retained_root_without_reading_ambient_owner() {
    if !supports_retained_root_replacement_test() {
        unica_process::test_support::skip_without_host_capability(
            unica_process::test_support::HostCapability::RenameUnderOpenHandle,
            "the platform keeps an open handle from being renamed or replaced",
        );
        return;
    }
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Original.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Original", 0),
    )
    .unwrap();
    let read = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
    let saved = source.with_file_name("retained-external");
    fs::rename(&source, &saved).unwrap();
    fs::create_dir(&source).unwrap();
    let foreign = external_leaf_descriptor("ExternalDataProcessor", "Foreign", 0);
    fs::write(source.join("Foreign.xml"), &foreign).unwrap();
    let cancellation = CancellationToken::new();
    let authority = LogicalViewReadAuthority::with_read_authority(
        &cancellation,
        read,
        PlatformProfile::v8_3_27(),
        ProviderDeadline::no_deadline(),
    );
    let result = ViewService::new(authority, ViewCursorStore::default())
        .view(ViewRequest::new("artifact:Configuration").unwrap());
    assert!(!result.ok);
    assert!(result.data.is_none());
    assert_eq!(
        fs::read_to_string(source.join("Foreign.xml")).unwrap(),
        foreign
    );
    assert!(saved.join("Original.xml").is_file());
}

#[test]
fn external_inventory_leaf_preserves_actor_admission_membership_finality() {
    let (workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    fs::write(
        source.join("Useful.xml"),
        external_leaf_descriptor("ExternalDataProcessor", "Useful", 0),
    )
    .unwrap();
    for index in 0..257 {
        fs::write(source.join(format!("noise{index:03}.txt")), "unrelated").unwrap();
    }
    let admission = crate::infrastructure::source_workspace::discover_project_source_admission(
        workspace.path(),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(admission.map().source_sets.len(), 1);
    assert_eq!(
        admission.map().source_sets[0].kind,
        SourceSetKind::ExternalProcessor
    );
    let before_format = admission.map().source_sets[0].source_format;
    let before_evidence = admission.map().source_sets[0].format_evidence.clone();
    let evidence = admission.into_evidence();
    evidence
        .validate(ProviderDeadline::no_deadline(), &CancellationToken::new())
        .unwrap();
    fs::write(
        source.join("noise-after-admission.txt"),
        "late unrelated member",
    )
    .unwrap();
    let current = crate::infrastructure::project_sources::discover_project_source_map_controlled(
        workspace.path(),
        &mut || Ok(()),
    )
    .unwrap();
    assert_eq!(current.source_sets[0].source_format, before_format);
    assert_eq!(current.source_sets[0].format_evidence, before_evidence);
    assert_eq!(
        evidence
            .validate(ProviderDeadline::no_deadline(), &CancellationToken::new())
            .unwrap_err()
            .kind(),
        unica_workspace::selection_evidence::SourceSelectionEvidenceErrorKind::Changed
    );
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn named_external_descriptor_and_owner_evidence_preserve_chunk_cancellation() {
    for evidence_read in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        fs::write(
            source.join("Large.xml"),
            external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024),
        )
        .unwrap();
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        assert_eq!(
            configuration_payload(&reader).unwrap()["registeredObjects"],
            json!([{"kind":"ExternalDataProcessor","name":"Large"}])
        );
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Large",
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let mut checks = 0;
        let mut checkpoint = || {
            checks += 1;
            if checks == 16 {
                cancellation.cancel();
            }
            if cancellation.is_cancelled() {
                Err(ViewError::new(
                    RefusalCode::Cancelled,
                    "manual named owner cancellation",
                ))
            } else {
                Ok(())
            }
        };
        let error = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut checkpoint)
                .expect_err("cancelled named proof cannot succeed")
        } else {
            reader
                .external_metadata_payload(&target, &mut checkpoint)
                .expect_err("cancelled named payload cannot succeed")
        };
        assert_eq!(checks, 16);
        assert_eq!(error.code(), RefusalCode::Cancelled);
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn named_external_descriptor_distinguishes_missing_from_unreadable_source() {
    for (evidence_read, has_neighbour) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        if has_neighbour {
            fs::write(
                source.join("Existing.xml"),
                external_leaf_descriptor("ExternalDataProcessor", "Existing", 0),
            )
            .unwrap();
        }
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Missing",
        )
        .unwrap();
        let missing = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut || Ok(()))
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut || Ok(()))
                .unwrap_err()
        };
        assert_eq!(missing.code(), RefusalCode::NotFound);
        assert_eq!(missing.detail(), None);
        assert_eq!(
            missing.to_string(),
            "external owner `ExternalDataProcessor.Missing` is not registered"
        );
        assert_eq!(reader.metadata_descriptor_read_count(target.as_str()), 1);

        // A successful positive lookup must not turn a later missing-name
        // lookup into a cached proof of absence.
        if has_neighbour {
            let existing = MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "ExternalDataProcessor.Existing",
            )
            .unwrap();
            reader
                .external_metadata_payload(&existing, &mut || Ok(()))
                .unwrap();
        }
        fs::create_dir(source.join("Missing.xml")).unwrap();
        let unreadable = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut || Ok(()))
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut || Ok(()))
                .unwrap_err()
        };
        assert_eq!(
            unreadable.detail(),
            Some(unica_refusal::RefusalDetail::SourceUnreadable)
        );
        assert_ne!(unreadable.code(), RefusalCode::NotFound);
        assert_eq!(reader.metadata_descriptor_read_count(target.as_str()), 2);
        assert!(source.join("Missing.xml").is_dir());
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn named_external_missing_descriptor_preserves_explicit_cancellation() {
    for evidence_read in [false, true] {
        let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
        let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
        let target = MetadataAddress::parse(
            PLATFORM_XML_8_3_27_FORMAT_2_20,
            "ExternalDataProcessor.Missing",
        )
        .unwrap();
        let expected = ViewError::new(RefusalCode::Cancelled, "manual missing owner cancellation");
        let mut checks = 0;
        let mut checkpoint = || {
            checks += 1;
            Err(expected.clone())
        };
        let error = if evidence_read {
            reader
                .metadata_owner_evidence(&target, &mut checkpoint)
                .unwrap_err()
        } else {
            reader
                .external_metadata_payload(&target, &mut checkpoint)
                .unwrap_err()
        };
        assert_eq!(error, expected);
        assert_eq!(checks, 1);
        assert!(!source.join("Missing.xml").exists());
    }
}

/// rule: INV.SOURCE.EXTERNAL-ARTIFACT-ROOTS
#[test]
fn external_leaf_reads_a_relative_file_whole_whatever_its_size() {
    let (_workspace, source) = external_leaf_fixture(SourceSetKind::ExternalProcessor);
    let bytes =
        external_leaf_descriptor("ExternalDataProcessor", "Large", 9 * 1024 * 1024).into_bytes();
    fs::write(source.join("Large.xml"), &bytes).unwrap();
    let reader = external_leaf_read(&source, SourceSetKind::ExternalProcessor);
    assert!(
        reader.read_relative_whole(Path::new("Large.xml")).unwrap() == bytes,
        "large relative file differs from its source bytes"
    );
    assert_eq!(
        configuration_payload(&reader).unwrap()["registeredObjects"],
        json!([{"kind":"ExternalDataProcessor","name":"Large"}])
    );
}
