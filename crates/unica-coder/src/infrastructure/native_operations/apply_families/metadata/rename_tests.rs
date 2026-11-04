use super::*;

const CATALOG: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><Catalog uuid="22222222-2222-4222-8222-222222222222"><InternalInfo><xr:GeneratedType name="CatalogRef.Old" category="Ref"><xr:TypeId>33333333-3333-4333-8333-333333333333</xr:TypeId><xr:ValueId>44444444-4444-4444-8444-444444444444</xr:ValueId></xr:GeneratedType></InternalInfo><Properties><Name>Old</Name><Synonym/><Comment/></Properties><ChildObjects/></Catalog></MetaDataObject>"#;
fn fixture(extra: &[(&str, &str)]) -> MetadataFixture {
    let configuration = RU_CONFIGURATION_XML.replace(
        "<Document>Order</Document>",
        "<Document>Order</Document><Catalog>Old</Catalog>",
    );
    let order = ORDER_XML.replace("<BasedOn/>", "<BasedOn/><Type><v8:Type>cfg:CatalogRef.Old</v8:Type><v8:Type>cfg:CatalogRef.OldLong</v8:Type></Type>");
    let mut files = vec![
        ("Configuration.xml", configuration.as_str()),
        ("Catalogs/Old.xml", CATALOG),
        ("Documents/Order.xml", order.as_str()),
        ("Catalogs/Old/Ext/ObjectModule.bsl", "// preserved module\n"),
    ];
    files.extend_from_slice(extra);
    MetadataFixture::with_files(&files)
}
fn plan(
    fixture: &MetadataFixture,
) -> Result<unica_workspace::actor::PreparedApplyBatch, super::super::ApplyPlanError> {
    plan_with_cancellation(fixture, &CancellationToken::new())
}
fn plan_with_cancellation(
    fixture: &MetadataFixture,
    cancellation: &CancellationToken,
) -> Result<unica_workspace::actor::PreparedApplyBatch, super::super::ApplyPlanError> {
    let admission = fixture
        .actor
        .admit_apply(
            &fixture.binding,
            None,
            false,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            cancellation,
        )
        .unwrap();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .metadata_planning_authority(&fixture.binding)
        .unwrap();
    let operation = fixture.parse(
        "object.rename",
        json!({"at":"main:Catalog.Old","values":{"newName":"New"}}),
        0,
    );
    let (staged, provisional) =
        plan_metadata_batch_owned_for_test(staged, authority, &[operation])?;
    let effects =
        unica_source_contract::proposal::reconcile_effects(staged.planned_changes(), provisional);
    admission
        .prepare_with_effects(staged, effects)
        .map_err(|error| super::super::ApplyPlanError::staging(error, "ops"))
}
#[test]
fn catalog_rename_stages_then_atomically_publishes_identity_and_references() {
    let fixture = fixture(&[]);
    let root = fixture.descriptor.parent().unwrap().parent().unwrap();
    let staged = plan(&fixture).unwrap();
    assert!(root.join("Catalogs/Old.xml").is_file());
    assert!(!root.join("Catalogs/New.xml").exists());
    fixture.actor.publish_prepared_apply(staged).unwrap();
    assert!(!root.join("Catalogs/Old.xml").exists());
    assert!(!root.join("Catalogs/Old").exists());
    assert_eq!(
        std::fs::read_to_string(root.join("Catalogs/New.xml")).unwrap(),
        CATALOG
            .replace("CatalogRef.Old", "CatalogRef.New")
            .replace("<Name>Old</Name>", "<Name>New</Name>")
    );
    assert_eq!(
        std::fs::read_to_string(root.join("Catalogs/New/Ext/ObjectModule.bsl")).unwrap(),
        "// preserved module\n"
    );
    let order = std::fs::read_to_string(root.join("Documents/Order.xml")).unwrap();
    assert!(order.contains("cfg:CatalogRef.New<"));
    assert!(order.contains("cfg:CatalogRef.OldLong<"));
}
#[test]
fn catalog_rename_preserves_the_default_form_identity_and_payload() {
    let form_descriptor = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-object-rename/src/Catalogs/СтарыйСклад/Forms/CorpusForm.xml"));
    let form_xml = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-object-rename/src/Catalogs/СтарыйСклад/Forms/CorpusForm/Ext/Form.xml")).replace("CatalogObject.СтарыйСклад", "CatalogObject.Old");
    let module = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/acceptance/workspace-object-rename/src/Catalogs/СтарыйСклад/Forms/CorpusForm/Ext/Form/Module.bsl"));
    let catalog = CATALOG.replace("<Comment/>", "<Comment/><DefaultObjectForm>Catalog.Old.Form.CorpusForm</DefaultObjectForm>").replace("<ChildObjects/>", "<ChildObjects><Form>CorpusForm</Form></ChildObjects>");
    let fixture = fixture(&[
        ("Catalogs/Old.xml", &catalog),
        ("Catalogs/Old/Forms/CorpusForm.xml", form_descriptor),
        ("Catalogs/Old/Forms/CorpusForm/Ext/Form.xml", &form_xml),
        ("Catalogs/Old/Forms/CorpusForm/Ext/Form/Module.bsl", module),
    ]);
    let root = fixture.descriptor.parent().unwrap().parent().unwrap();
    let staged = plan(&fixture).unwrap();
    assert_eq!(std::fs::read_to_string(root.join("Catalogs/Old.xml")).unwrap(), catalog);
    assert!(!root.join("Catalogs/New").exists());
    fixture.actor.publish_prepared_apply(staged).unwrap();
    assert!(!root.join("Catalogs/Old").exists());
    assert_eq!(std::fs::read_to_string(root.join("Catalogs/New.xml")).unwrap(), catalog.replace("<Name>Old</Name>", "<Name>New</Name>").replace("CatalogRef.Old", "CatalogRef.New").replace("Catalog.Old.Form.CorpusForm", "Catalog.New.Form.CorpusForm"));
    assert_eq!(std::fs::read_to_string(root.join("Catalogs/New/Forms/CorpusForm.xml")).unwrap(), form_descriptor);
    assert_eq!(std::fs::read_to_string(root.join("Catalogs/New/Forms/CorpusForm/Ext/Form.xml")).unwrap(), form_xml.replace("CatalogObject.Old", "CatalogObject.New"));
    assert_eq!(std::fs::read_to_string(root.join("Catalogs/New/Forms/CorpusForm/Ext/Form/Module.bsl")).unwrap(), module);
}
#[test]
fn catalog_rename_refuses_collision_code_unknown_xml_and_empty_directories_without_writing() {
    for extra in [
        vec![("Catalogs/new.xml", "collision")],
        vec![(
            "CommonModules/Module/Ext/Module.bsl",
            "Справочники . Old.Выбрать();",
        )],
        vec![(
            "Ext/Unknown.xml",
            "<root><Unknown>Catalog.Old</Unknown></root>",
        )],
    ] {
        let fixture = fixture(&extra);
        assert!(plan(&fixture).is_err());
        assert_eq!(fixture.disk_bytes(), ORDER_XML.replace("<BasedOn/>", "<BasedOn/><Type><v8:Type>cfg:CatalogRef.Old</v8:Type><v8:Type>cfg:CatalogRef.OldLong</v8:Type></Type>").as_bytes());
    }
    let fixture = fixture(&[]);
    let root = fixture.descriptor.parent().unwrap().parent().unwrap();
    std::fs::create_dir_all(root.join("Catalogs/Old/Forms/Empty")).unwrap();
    assert!(plan(&fixture)
        .expect_err("rename must refuse")
        .to_string()
        .contains("empty resource directory"));
    assert!(root.join("Catalogs/Old/Forms/Empty").is_dir());
}
#[test]
fn catalog_rename_saved_inputs_refuse_changed_dependency_and_changed_namespaces() {
    for mode in 0..4 {
        let fixture = fixture(&[]);
        let root = fixture.descriptor.parent().unwrap().parent().unwrap();
        let prepared = plan(&fixture).unwrap();
        match mode {
            0 => std::fs::write(root.join("Documents/Order.xml"), ORDER_XML).unwrap(),
            1 => std::fs::write(root.join("Catalogs/Old/Ext/Late.bsl"), "// late").unwrap(),
            2 => std::fs::create_dir_all(root.join("Catalogs/New/Empty")).unwrap(),
            _ => std::fs::write(root.join("Catalogs/Old/Ext/ObjectModule.bsl"), "// changed")
                .unwrap(),
        }
        let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
        assert_eq!(
            error.kind(),
            unica_workspace::actor::ApplyPublicationErrorKind::ConcurrentRevision,
            "mode {mode}: {error}"
        );
        assert!(root.join("Catalogs/Old.xml").is_file());
        assert!(!root.join("Catalogs/New.xml").exists());
    }
}
#[test]
fn catalog_rename_decoder_refuses_invalid_names_unknown_fields_and_wrong_kinds() {
    let fixture = fixture(&[]);
    for args in [
        json!({"at":"main:Catalog.Old","values":{"newName":" New "}}),
        json!({"at":"main:Catalog.Old","values":{"newName":"../New"}}),
        json!({"at":"main:Catalog.Old","values":{"newName":"New", "name":"Old"}}),
        json!({"at":"main:Document.Order","values":{"newName":"New"}}),
        json!({"at":"main:Catalog.Old.Attribute.X","values":{"newName":"New"}}),
    ] {
        assert!(
            parse_metadata_plan_operation("object.rename", &args, 0, &fixture.binding).is_err(),
            "{args}"
        );
    }
}
#[test]
fn catalog_rename_refuses_locked_referrer_and_incompatible_referrer_format() {
    for locked in [false, true] {
        let fixture = fixture(&[]);
        let root = fixture.binding.source_root();
        if locked {
            std::fs::create_dir_all(root.join("Ext")).unwrap();
            std::fs::write(root.join("Ext/ParentConfigurations.bin"), "{6,0,1,dddddddd-dddd-4ddd-8ddd-dddddddddddd,0,11111111-1111-4111-8111-111111111111,\"1.0\",\"Vendor\",\"VendorConf\",3,1,0,22222222-2222-4222-8222-222222222222,22222222-2222-4222-8222-222222222222,0,0,11111111-1111-4111-8111-111111111111,11111111-1111-4111-8111-111111111111}").unwrap();
        } else {
            let bytes = std::fs::read_to_string(&fixture.descriptor)
                .unwrap()
                .replace("version=\"2.20\"", "version=\"2.21\"");
            std::fs::write(&fixture.descriptor, bytes).unwrap();
        }
        let before = unica_workspace::test_support::tree_snapshot(root);
        let error = plan(&fixture).expect_err("unsafe dependent owner must refuse");
        assert!(
            error.to_string().contains(if locked {
                "locked vendor objects"
            } else {
                "format validation failed"
            }),
            "{error:?}"
        );
        assert_eq!(unica_workspace::test_support::tree_snapshot(root), before);
    }
}
#[test]
fn catalog_rename_rolls_back_a_late_failure_with_resource_directories() {
    let fixture = fixture(&[]);
    let root = fixture.binding.source_root();
    let before = unica_workspace::test_support::tree_snapshot(root);
    let cancellation = CancellationToken::new();
    let prepared = plan_with_cancellation(&fixture, &cancellation).unwrap();
    let cancel_after_writes = cancellation.clone();
    let published = root.join("Catalogs/New.xml");
    unica_workspace::compile_transaction::set_retained_apply_before_post_validation_hook(
        move || {
            assert!(
                published.is_file(),
                "cancellation must happen after publication of source postimages"
            );
            cancel_after_writes.cancel();
        },
    );
    let error = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(
        error.kind(),
        unica_workspace::actor::ApplyPublicationErrorKind::Cancelled
    );
    assert_eq!(unica_workspace::test_support::tree_snapshot(root), before);
    assert!(!root.join("Catalogs/New").exists());
}
#[test]
fn catalog_rename_refuses_after_another_staged_operation_without_publishing() {
    let fixture = fixture(&[]);
    let root = fixture.binding.source_root();
    let before = unica_workspace::test_support::tree_snapshot(root);
    let admission = fixture.admission();
    let staged = admission.staged_state().unwrap();
    let authority = admission
        .metadata_planning_authority(&fixture.binding)
        .unwrap();
    let operations = [
        fixture.parse(
            "props.set",
            json!({"at":"main:Document.Order","values":{"Comment":"earlier change"}}),
            0,
        ),
        fixture.parse(
            "object.rename",
            json!({"at":"main:Catalog.Old","values":{"newName":"New"}}),
            1,
        ),
    ];
    let error = plan_metadata_batch_owned_for_test(staged, authority, &operations)
        .expect_err("root scan cannot inspect a prior overlay");
    assert!(error.to_string().contains("must run in its own apply"));
    assert_eq!(unica_workspace::test_support::tree_snapshot(root), before);
}
