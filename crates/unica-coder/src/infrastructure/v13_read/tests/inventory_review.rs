use super::*;

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
pub(crate) fn object_commands_are_registered_inline_without_descriptor_files() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let owner = fs::read_to_string(&descriptor).unwrap();
    // The owner's own `ChildObjects` closes last; nested tabular sections
    // close theirs earlier.
    let close = owner.rfind("</ChildObjects>").unwrap();
    let owner = format!(
        "{}{}{}",
        &owner[..close],
        r#"<Command uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Inline</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Инлайн</v8:content></v8:item></Synonym></Properties></Command>"#,
        &owner[close..]
    );
    fs::write(&descriptor, owner).unwrap();
    write(
        &fixture.source.join("Catalogs/Items/Commands/Inline/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{:?}", owner.diagnostics);
    // The shared fixture registers `Refresh` by text and the test adds an
    // inline definition; neither has a `Commands/<Name>.xml` on disk.
    for (at, title) in [
        ("main:Catalog.Items.Command.Refresh", "Refresh"),
        ("main:Catalog.Items.Command.Inline", "Инлайн"),
    ] {
        let command = service.view(ViewRequest::new(at).unwrap());
        assert!(command.ok, "{at}: {:?}", command.diagnostics);
        let data = command.data.as_ref().unwrap();
        assert_eq!(data["kind"], "Command", "{at}");
        assert_eq!(data["title"], title, "{at}");
    }

    let authority = fixture.read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for expected in [
        "main:Catalog.Items.Command.Refresh",
        "main:Catalog.Items.Command.Inline",
    ] {
        let found = index.find(FindRequest::new(expected).unwrap());
        assert!(
            !found.is_nearest()
                && found
                    .candidates()
                    .iter()
                    .any(|candidate| candidate.at() == expected),
            "inline command identity is missing from find: {expected}: {found:?}"
        );
    }
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn review_rejects_orphan_nested_module_owners_not_registered_by_parent() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Catalogs/Items/Forms/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form uuid="10000000-0000-4000-8000-000000000077"><Properties><Name>Orphan</Name><FormType>Managed</FormType></Properties></Form></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Catalogs/Items/Commands/Orphan/Ext/CommandModule.bsl"),
        "&AtClient\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure\n",
    );
    let service = fixture.view_service();

    for address in [
        "main:Catalog.Items.Form.Orphan.Module",
        "main:Catalog.Items.Command.Orphan.Module",
    ] {
        let result = service.view(ViewRequest::new(address).unwrap());
        assert!(
            !result.ok,
            "orphan owner was invented for {address}: {result:?}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found");
    }
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn review_rejects_direct_typed_owner_absent_from_configuration_inventory() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("Roles/Orphan.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Role uuid="10000000-0000-4000-8000-000000000079"><Properties><Name>Orphan</Name></Properties></Role></MetaDataObject>"#,
    );
    write(
        &fixture.source.join("Roles/Orphan/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20"/>"#,
    );
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.Orphan").unwrap());
    assert!(
        !result.ok,
        "unregistered role entered the logical tree: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "not_found");
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn registered_physical_child_with_wrong_descriptor_fails_direct_and_parent_navigation() {
    let fixture = RealReaderFixture::new();
    let owner_path = fixture.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>Wrong</Form></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    write(
        &fixture.source.join("Reports/ParityReport/Forms/Wrong.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Command><Properties><Name>Wrong</Name></Properties></Command></MetaDataObject>"#,
    );
    let service = fixture.view_service();

    for at in [
        "main:Report.ParityReport.Form.Wrong",
        "main:Report.ParityReport",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "wrong child descriptor escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    }
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn registered_top_level_owner_without_descriptor_fails_kind_branch_and_direct_view() {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    let owner = fs::read_to_string(&configuration).unwrap().replacen(
        "</ChildObjects>",
        "<Role>Missing</Role></ChildObjects>",
        1,
    );
    fs::write(configuration, owner).unwrap();
    let service = fixture.view_service();

    let root = service.view(ViewRequest::new("main:Configuration").unwrap());
    assert!(root.ok, "the root proves only Configuration.xml");
    for at in ["main:Role", "main:Role.Missing"] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing top-level descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn orphan_and_missing_physical_children_fail_closed_across_reader_families() {
    let orphan = RealReaderFixture::new();
    for (directory, kind, name) in [
        ("Forms", "Form", "OrphanForm"),
        ("Templates", "Template", "OrphanDcs"),
        ("Templates", "Template", "OrphanMxl"),
        ("Commands", "Command", "OrphanCommand"),
    ] {
        write(
            &orphan
                .source
                .join(format!("Reports/ParityReport/{directory}/{name}.xml")),
            &format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{kind}><Properties><Name>{name}</Name><TemplateType>DataCompositionSchema</TemplateType></Properties></{kind}></MetaDataObject>"#
            ),
        );
    }
    let service = orphan.view_service();
    for at in [
        "main:Report.ParityReport.Form.OrphanForm",
        "main:Report.ParityReport.Template.OrphanDcs.DataSet",
        "main:Report.ParityReport.Template.OrphanMxl.Area",
        "main:Report.ParityReport.Command.OrphanCommand",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(!result.ok, "orphan physical child escaped through {at}");
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }

    let missing = RealReaderFixture::new();
    let owner_path = missing.source.join("Reports/ParityReport.xml");
    let owner = fs::read_to_string(&owner_path).unwrap().replacen(
        "</ChildObjects>",
        "<Form>MissingForm</Form><Template>MissingDcs</Template><Template>MissingMxl</Template></ChildObjects>",
        1,
    );
    fs::write(owner_path, owner).unwrap();
    let service = missing.view_service();
    for at in [
        "main:Report.ParityReport.Form.MissingForm",
        "main:Report.ParityReport.Template.MissingDcs.DataSet",
        "main:Report.ParityReport.Template.MissingMxl.Area",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "missing registered descriptor escaped through {at}"
        );
        assert_eq!(
            result.diagnostics[0]["code"], "provider_unavailable",
            "{at}"
        );
    }
}

/// rule: INV.SOURCE.LOGICAL-READER-PARITY
#[test]
fn unregistered_top_level_descriptors_cannot_enter_any_typed_reader_family() {
    let fixture = RealReaderFixture::new();
    write_metadata_owner(&fixture.source, "Catalogs", "Catalog", "Orphan", "");
    write_metadata_owner(&fixture.source, "Subsystems", "Subsystem", "Orphan", "");
    write_metadata_owner(&fixture.source, "XDTOPackages", "XDTOPackage", "Orphan", "");
    write_metadata_owner(&fixture.source, "CommonForms", "CommonForm", "Orphan", "");
    write(
        &fixture.source.join("CommonForms/Orphan/Ext/Form.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
    );
    let service = fixture.view_service();
    for at in [
        "main:Catalog.Orphan",
        "main:Catalog.Orphan.Module",
        "main:Subsystem.Orphan",
        "main:Subsystem.Orphan.Interface",
        "main:XDTOPackage.Orphan",
        "main:CommonForm.Orphan",
    ] {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(
            !result.ok,
            "unregistered top-level owner escaped through {at}"
        );
        assert_eq!(result.diagnostics[0]["code"], "not_found", "{at}");
    }
}

#[test]
fn review_revision_authority_cannot_be_swapped_after_named_identity_validation() {
    let fixture = RealReaderFixture::new();
    let replacement = fixture.root.path().join("replacement");
    fs::create_dir_all(&replacement).unwrap();
    fs::write(
        replacement.join("Configuration.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?><MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>ReplacementConfiguration</Name></Properties><ChildObjects/></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    let saved = fixture.root.path().join("retained-a");
    let source_before = fixture.source.clone();
    let replacement_before = replacement.clone();
    let saved_before = saved.clone();
    let source_after = fixture.source.clone();
    let replacement_after = replacement;
    let saved_after = saved;
    review_set_revision_identity_hooks(
        move || {
            if saved_before.exists() {
                fs::rename(&source_before, &replacement_before).unwrap();
                fs::rename(&saved_before, &source_before).unwrap();
            }
        },
        move || {
            fs::rename(&source_after, &saved_after).unwrap();
            fs::rename(&replacement_after, &source_after).unwrap();
        },
    );

    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Configuration").unwrap());
    review_clear_revision_identity_hooks();

    assert!(
        !result.ok,
        "revision came from replacement B while retained bytes came from A: {result:?}"
    );
}

/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
fn review_role_canonical_encoding_cannot_collapse_distinct_kind_name_pairs() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>Catalog.Orders_X</name><right><name>Read</name><value>true</value></right></object>
  <object><name>Catalog_Orders.X</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(!result.ok, "invalid platform kinds must fail closed");
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
    assert!(result.diagnostics[0]["message"]
        .as_str()
        .is_some_and(|message| message.contains("Catalog_Orders")));
}

/// rule: INV.SOURCE.ROLE-RIGHT-ADDRESSES
#[test]
fn review_role_rejects_non_platform_metadata_node_kinds() {
    let fixture = RealReaderFixture::new();
    fs::write(
        fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Rights xmlns="http://v8.1c.ru/8.2/roles" version="2.20">
  <object><name>WebSocketClient.Telephony</name><right><name>Read</name><value>true</value></right></object>
</Rights>"#,
    )
    .unwrap();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Role.SalesReader.Right").unwrap());
    assert!(
        !result.ok,
        "non-platform role object kind escaped: {result:?}"
    );
    assert_eq!(result.diagnostics[0]["code"], "provider_unavailable");
}

#[test]
fn review_production_read_port_has_no_nocancel_inventory_entrypoint() {
    let infrastructure = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/infrastructure");
    let mut sources = vec![infrastructure.join("v13_read_port.rs")];
    sources.extend(
        fs::read_dir(infrastructure.join("v13_read_port"))
            .unwrap()
            .map(|entry| entry.unwrap().path()),
    );
    assert!(sources.len() > 1, "read port kind files are not visible");
    for path in sources {
        let source = fs::read_to_string(&path).unwrap();
        assert!(
            !source.contains("configuration_payload_with_checkpoint(&mut || Ok(()))"),
            "production-compiled read port retains a no-op cancellation bypass: {}",
            path.display()
        );
    }
}
