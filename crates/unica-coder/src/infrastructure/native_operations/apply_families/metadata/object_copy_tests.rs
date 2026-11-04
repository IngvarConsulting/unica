use super::*;
use unica_workspace::test_support::tree_snapshot;

#[test]
fn object_copy_contract_and_decoder_accept_only_configuration_catalog_and_a_new_identifier() {
    let fixture = complete_fixture();
    for at in [
        "main:Catalog.Inspectable",
        "main:Document.Order",
        "main:Configuration",
        "main:Catalog.Inspectable.Attribute.CustomCode",
    ] {
        let args = json!({"at":at,"values":{"name":"Copy"}});
        let parsed = parse_metadata_plan_operation("object.copy", &args, 0, &fixture.binding);
        assert_eq!(
            parsed.is_ok(),
            at == "main:Catalog.Inspectable",
            "{at}: {parsed:?}"
        );
    }
    let contract = unica_metadata::contracts::metadata_object_copy();
    let schema = jsonschema::validator_for(&contract.schema).unwrap();
    for values in [
        json!({"name":"Copy"}),
        json!({"name":"Копия"}),
        json!({"name":" Copy "}),
        json!({"name":"A.B"}),
        json!({"name":"Copy","kind":"Catalog"}),
        json!({"name":false}),
        json!({}),
    ] {
        let args = json!({"at":"main:Catalog.Inspectable","values":values});
        assert_eq!(
            parse_metadata_plan_operation("object.copy", &args, 0, &fixture.binding).is_ok(),
            schema.is_valid(&args),
            "{args}"
        );
    }
    let extension = MetadataFixture::with_kind_and_files(SourceSetKind::Extension, &[]);
    assert!(parse_metadata_plan_operation(
        "object.copy",
        &json!({"at":"main:Catalog.Inspectable","values":{"name":"Copy"}}),
        0,
        &extension.binding
    )
    .is_err());
}

fn complete_fixture() -> MetadataFixture {
    let fixture = MetadataFixture::with_files(&[]);
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/acceptance/workspace-object-copy/src");
    fn copy_tree(source: &Path, destination: &Path) {
        std::fs::create_dir_all(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let target = destination.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_tree(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    copy_tree(&source, fixture.binding.source_root());
    fixture
}

fn plan_copy(
    fixture: &MetadataFixture,
) -> Result<
    unica_workspace::actor::PreparedApplyBatch,
    unica_source_contract::proposal::ApplyPlanError,
> {
    let args = json!({
        "at":"main:Catalog.Inspectable", "ops":[{"op":"object.copy","args":{"values":{"name":"Copy"}}}], "dryRun":true
    });
    let request =
        crate::domain::apply::ApplyRequest::parse(args.as_object().unwrap(), &["main"]).unwrap();
    let mut admission = fixture.admission();
    admission.bind_request_identity(
        &request.at().to_string(),
        request
            .ops()
            .iter()
            .map(|op| (op.name(), op.at().to_string(), op.args())),
    );
    let (staged, effects) =
        crate::infrastructure::native_operations::apply_families::plan_hidden_v13_apply(
            &request,
            &fixture.binding,
            &admission,
        )?;
    assert_eq!(effects.events().len(), 1);
    assert_eq!(
        effects.events()[0].kind,
        unica_source_contract::events::DomainEventKind::MetadataChanged
    );
    assert_eq!(effects.events()[0].artifact, "Catalog.Copy");
    admission
        .prepare_with_effects(staged, effects)
        .map_err(|error| {
            unica_source_contract::proposal::ApplyPlanError::new(
                ApplyPlanErrorKind::InvalidState,
                error.to_string(),
            )
        })
}

fn save_copy(fixture: &MetadataFixture) -> String {
    let before = tree_snapshot(fixture.binding.source_root());
    let plan = plan_copy(fixture).unwrap();
    let projected = fixture.actor.preview_prepared_apply(&plan).unwrap();
    let mut preview = unica_execution_contract::invocation::DomainResult::success("preview");
    preview.rev = Some(projected.rev().to_owned());
    let token = fixture.actor.save_prepared_apply(plan, preview).unwrap();
    assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
    token
}

fn execute_copy(
    fixture: &MetadataFixture,
    token: &str,
) -> Result<(), unica_workspace::actor::ApplyPublicationError> {
    let mut outcome = None;
    fixture
        .actor
        .execute_saved_apply(
            token,
            ProviderDeadline::from_budget(Duration::from_secs(10)),
            &CancellationToken::new(),
            |result, publication| {
                outcome = Some(publication.map(|_| ()));
                result
            },
        )
        .unwrap();
    outcome.unwrap_or(Ok(()))
}

#[test]
fn object_copy_publication_failure_rolls_back_the_complete_copy() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };
    for failpoint in [
        RetainedApplyFailpoint::Source(1),
        RetainedApplyFailpoint::AfterAllPostimages,
    ] {
        let fixture = complete_fixture();
        let token = save_copy(&fixture);
        let before = tree_snapshot(fixture.binding.source_root());
        set_retained_apply_failpoint(failpoint);
        assert!(execute_copy(&fixture, &token).is_err());
        assert_eq!(tree_snapshot(fixture.binding.source_root()), before);
        assert!(!fixture.binding.source_root().join("Catalogs/Copy").exists());
    }
}

#[test]
fn object_copy_preserves_empty_directories_and_guards_their_membership() {
    for stale in [false, true] {
        let fixture = complete_fixture();
        let source = fixture.binding.source_root();
        std::fs::create_dir_all(source.join("Catalogs/Inspectable/ExtraEmpty/Nested")).unwrap();
        let token = save_copy(&fixture);
        assert!(!source.join("Catalogs/Copy").exists());
        if stale {
            std::fs::create_dir(source.join("Catalogs/Inspectable/ExtraEmpty/Concurrent")).unwrap();
            let before = tree_snapshot(source);
            assert!(execute_copy(&fixture, &token).is_err());
            assert_eq!(tree_snapshot(source), before);
            assert!(!source.join("Catalogs/Copy").exists());
        } else {
            execute_copy(&fixture, &token).unwrap();
            assert!(source.join("Catalogs/Copy/ExtraEmpty/Nested").is_dir());
            assert_eq!(
                std::fs::read_dir(source.join("Catalogs/Copy/ExtraEmpty/Nested"))
                    .unwrap()
                    .count(),
                0
            );
        }
    }
}

#[test]
fn object_copy_rollback_preserves_a_foreign_file_and_reports_incomplete_cleanup() {
    let fixture = complete_fixture();
    let source = fixture.binding.source_root();
    std::fs::create_dir(source.join("Catalogs/Inspectable/ExtraEmpty")).unwrap();
    let owner_before = std::fs::read(source.join("Configuration.xml")).unwrap();
    let token = save_copy(&fixture);
    let foreign = source.join("Catalogs/Copy/ExtraEmpty/foreign.bin");
    let foreign_for_hook = foreign.clone();
    unica_workspace::compile_transaction::set_retained_apply_before_post_validation_hook(
        move || {
            std::fs::write(foreign_for_hook, b"foreign").unwrap();
        },
    );
    let error = execute_copy(&fixture, &token).unwrap_err();
    assert_eq!(
        error.kind(),
        unica_workspace::actor::ApplyPublicationErrorKind::RollbackIncomplete
    );
    assert_eq!(std::fs::read(foreign).unwrap(), b"foreign");
    assert_eq!(
        std::fs::read(source.join("Configuration.xml")).unwrap(),
        owner_before
    );
    assert!(!source.join("Catalogs/Copy.xml").exists());
}

#[test]
fn object_copy_saved_plan_preserves_resources_and_remaps_only_its_own_identities() {
    let fixture = complete_fixture();
    let source = fixture.binding.source_root();
    let before = tree_snapshot(source);
    let token = save_copy(&fixture);
    execute_copy(&fixture, &token).unwrap();
    for (path, bytes) in &before {
        if path != Path::new("Configuration.xml") {
            assert_eq!(
                std::fs::read(source.join(path)).unwrap(),
                *bytes,
                "source changed: {path:?}"
            );
        }
    }
    let copied = std::fs::read_to_string(source.join("Catalogs/Copy.xml")).unwrap();
    let original = std::fs::read_to_string(source.join("Catalogs/Inspectable.xml")).unwrap();
    let md = "http://v8.1c.ru/8.3/MDClasses";
    let xr = "http://v8.1c.ru/8.3/xcf/readable";
    let v8 = "http://v8.1c.ru/8.1/data/core";
    let document = roxmltree::Document::parse(&copied).unwrap();
    let original_doc = roxmltree::Document::parse(&original).unwrap();
    let owned = |document: &roxmltree::Document<'_>| {
        document
            .descendants()
            .filter_map(|node| node.attribute("uuid"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let originals = owned(&original_doc);
    let copies = owned(&document);
    assert_eq!(originals.len(), copies.len());
    assert!(originals.iter().all(|uuid| !copies.contains(uuid)));
    let ref_type = document
        .descendants()
        .find(|node| {
            node.has_tag_name((xr, "GeneratedType")) && node.attribute("category") == Some("Ref")
        })
        .unwrap();
    assert_eq!(ref_type.attribute("name"), Some("CatalogRef.Copy"));
    let new_type = ref_type
        .children()
        .find(|node| node.has_tag_name((xr, "TypeId")))
        .unwrap()
        .text()
        .unwrap();
    assert_ne!(new_type, "0c3c2bb5-c599-4fdf-80a9-fdaf6deb1761");
    assert!(document
        .descendants()
        .any(|node| node.has_tag_name((v8, "TypeId")) && node.text() == Some(new_type)));
    assert!(copied.contains("cfg:CatalogRef.External"));
    assert!(copied.contains("34447986-fb48-438a-aff5-a75628618ae0"));
    assert!(copied.contains("cfg:CatalogRef.Copy"));
    assert!(copied.contains("Catalog.Copy.Form.Main"));
    assert!(copied.contains("<xr:ChoiceForm>Catalog.Copy.Form.Main</xr:ChoiceForm>"));
    assert!(copied.contains("<FillValue xsi:type=\"xs:string\">Catalog.Inspectable</FillValue>"));
    assert!(document
        .descendants()
        .filter(|node| node.has_tag_name((xr, "Field")))
        .all(|node| !node
            .text()
            .unwrap_or_default()
            .starts_with("Catalog.Inspectable.")));
    assert_eq!(
        document
            .descendants()
            .find(|node| node.has_tag_name((md, "Comment")))
            .unwrap()
            .text(),
        Some("Catalog.Inspectable")
    );
    assert!(copied.contains("<!-- Catalog.Inspectable 0c3c2bb5-c599-4fdf-80a9-fdaf6deb1761 -->"));
    assert!(copied.contains("<v8:content>0c3c2bb5-c599-4fdf-80a9-fdaf6deb1761</v8:content>"));
    for relative in [
        "Ext/ObjectModule.bsl",
        "Templates/Blob/Ext/Template.bin",
        "Templates/Layout/Ext/Template.xml",
    ] {
        assert_eq!(
            std::fs::read(source.join("Catalogs/Inspectable").join(relative)).unwrap(),
            std::fs::read(source.join("Catalogs/Copy").join(relative)).unwrap()
        );
    }
    let after = tree_snapshot(source);
    execute_copy(&fixture, &token).unwrap();
    assert_eq!(
        tree_snapshot(source),
        after,
        "saved token replay must not publish twice"
    );
}

#[test]
fn object_copy_collisions_and_invalid_sources_leave_the_tree_unchanged() {
    for case in [
        "descriptor",
        "directory",
        "registration",
        "malformed",
        "wrong-name",
        "duplicate-id",
        "missing-form",
        "missing-form-content",
        "missing-template-content",
        "sidecar-profile",
        "form-profile",
    ] {
        let fixture = complete_fixture();
        let source = fixture.binding.source_root();
        let descriptor = source.join("Catalogs/Inspectable.xml");
        match case {
            "descriptor" => {
                std::fs::write(source.join("Catalogs/Copy.xml"), "occupied").unwrap();
            }
            "directory" => {
                std::fs::create_dir(source.join("Catalogs/Copy")).unwrap();
            }
            "registration" => {
                let path = source.join("Configuration.xml");
                let text = std::fs::read_to_string(&path).unwrap().replace(
                    "<Catalog>External</Catalog>",
                    "<Catalog>External</Catalog><Catalog>Copy</Catalog>",
                );
                std::fs::write(path, text).unwrap();
            }
            "malformed" => {
                std::fs::write(
                    source.join("Catalogs/Inspectable/Templates/Layout/Ext/Template.xml"),
                    "<broken>",
                )
                .unwrap();
            }
            "wrong-name" => {
                let text = std::fs::read_to_string(&descriptor)
                    .unwrap()
                    .replace("<Name>Inspectable</Name>", "<Name>Wrong</Name>");
                std::fs::write(&descriptor, text).unwrap();
            }
            "duplicate-id" => {
                let text = std::fs::read_to_string(&descriptor).unwrap().replace(
                    "5e51a879-ad50-4b17-bdac-1b4074ac2660",
                    "85efaec2-2aa4-4226-9cbf-8080039d9118",
                );
                std::fs::write(&descriptor, text).unwrap();
            }
            "missing-form-content" => {
                std::fs::remove_file(source.join("Catalogs/Inspectable/Forms/Main/Ext/Form.xml"))
                    .unwrap();
            }
            "missing-template-content" => {
                std::fs::remove_file(
                    source.join("Catalogs/Inspectable/Templates/Layout/Ext/Template.xml"),
                )
                .unwrap();
            }
            "sidecar-profile" => {
                std::fs::write(source.join("Catalogs/Inspectable/Ext/CommandInterface.xml"), r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.21"/>"#).unwrap();
            }
            "missing-form" => {
                std::fs::remove_file(source.join("Catalogs/Inspectable/Forms/Main.xml")).unwrap();
            }
            "form-profile" => {
                let path = source.join("Catalogs/Inspectable/Forms/Main.xml");
                let text = std::fs::read_to_string(&path)
                    .unwrap()
                    .replace("version=\"2.20\"", "version=\"2.&#50;0\"");
                std::fs::write(path, text).unwrap();
            }
            _ => unreachable!(),
        }
        let before = tree_snapshot(source);
        assert!(plan_copy(&fixture).is_err(), "accepted {case}");
        assert_eq!(tree_snapshot(source), before, "wrote during {case}");
    }
}

#[test]
fn object_copy_saved_plan_rejects_changed_payload_and_directory_membership() {
    for case in ["module", "binary", "added", "removed", "target-directory"] {
        let fixture = complete_fixture();
        let token = save_copy(&fixture);
        let source = fixture.binding.source_root();
        match case {
            "module" => std::fs::write(
                source.join("Catalogs/Inspectable/Ext/ObjectModule.bsl"),
                "// changed",
            )
            .unwrap(),
            "binary" => std::fs::write(
                source.join("Catalogs/Inspectable/Templates/Blob/Ext/Template.bin"),
                [0, 1, 2],
            )
            .unwrap(),
            "added" => {
                std::fs::write(source.join("Catalogs/Inspectable/Ext/New.bsl"), "// new").unwrap()
            }
            "removed" => {
                std::fs::remove_file(source.join("Catalogs/Inspectable/Forms/Main/Ext/Form.xml"))
                    .unwrap()
            }
            "target-directory" => std::fs::create_dir(source.join("Catalogs/Copy")).unwrap(),
            _ => unreachable!(),
        }
        let before = tree_snapshot(source);
        assert!(
            execute_copy(&fixture, &token).is_err(),
            "accepted stale {case}"
        );
        assert_eq!(tree_snapshot(source), before, "partial copy on {case}");
    }
}

#[test]
fn object_copy_previews_a_registered_catalog_without_writing_the_source() {
    let fixture = MetadataFixture::with_files(&[
        (
            "Configuration.xml",
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name></Properties><ChildObjects><Catalog>Items</Catalog></ChildObjects></Configuration></MetaDataObject>"#,
        ),
        (
            "Catalogs/Items.xml",
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Items</Name><Comment>keep</Comment></Properties><ChildObjects/></Catalog></MetaDataObject>"#,
        ),
    ]);
    let operation = fixture.parse(
        "object.copy",
        json!({"at":"main:Catalog.Items", "values":{"name":"ItemsCopy"}}),
        0,
    );
    let admission = fixture.admission();
    let (staged, effects) = plan_metadata_batch_owned_for_test(
        admission.staged_state().unwrap(),
        admission
            .metadata_planning_authority(&fixture.binding)
            .unwrap(),
        &[operation],
    )
    .unwrap();
    let changes = staged.planned_changes();
    let copy = changes
        .iter()
        .find(|change| change.relative_path == Path::new("Catalogs/ItemsCopy.xml"))
        .expect("copy descriptor");
    let StagedFileState::Bytes(bytes) = &copy.current else {
        panic!("copy bytes")
    };
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(text.contains("<Name>ItemsCopy</Name>"));
    assert!(!text.contains("11111111-1111-4111-8111-111111111111"));
    assert!(text.contains("<Comment>keep</Comment>"));
    assert_eq!(effects.len(), 1);
    assert!(!fixture
        .binding
        .source_root()
        .join("Catalogs/ItemsCopy.xml")
        .exists());
}
