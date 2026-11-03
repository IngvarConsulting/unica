use super::*;

#[test]
fn interface_reference_limits_classify_xml_representation_without_losing_controls() {
    let fixture = RealReaderFixture::new();
    let path = fixture
        .source
        .join("Subsystems/Sales/Ext/CommandInterface.xml");
    write(
        &path,
        r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><CommandsVisibility>
<Command name="CommonCommand.Open"><Visibility><xr:Common>false</xr:Common></Visibility></Command>
<Command name="Catalog.Items.Command.Refresh"><Visibility><xr:Common>true</xr:Common><xr:Value name="Role.One">false</xr:Value><xr:Value name="Role.Two">true</xr:Value><xr:Value name="Role.Three">false</xr:Value></Visibility></Command>
<Command name="DocumentJournal.Documents.StandardCommand.OpenList"><Visibility><xr:Common>true</xr:Common></Visibility></Command>
<Command name="Report.Contacts.StandardCommand.Open"><Visibility><xr:Common>false</xr:Common></Visibility></Command>
<Command name="Report.Contacts.StandardCommand.Bad-Name"><Visibility><xr:Common>false</xr:Common></Visibility></Command>
<Command name="0:bdebc888-2581-426c-9f5e-c09fbba5f6e2"><Visibility><xr:Common>true</xr:Common></Visibility></Command>
<Command name="broken-command"><Visibility><xr:Common>false</xr:Common></Visibility></Command>
</CommandsVisibility></CommandInterface>"#,
    );
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    for at in [
        "main:Subsystem.Sales.Interface",
        "main:Subsystem.Sales.Interface.Command",
    ] {
        let read = service.view(ViewRequest::new(at).unwrap());
        assert!(read.ok, "{:?}", refusal_codes(&read));
        let data = read.data.unwrap();
        let limits = data["limits"].as_array().unwrap();
        assert_eq!(
            limits.len(),
            3,
            "ID, unsupported named, and malformed must remain distinct"
        );
        assert!(limits.iter().any(
            |limit| limit.as_str().unwrap().contains("по идентификатору")
                && !limit.as_str().unwrap().contains("StandardCommand")
        ));
        assert!(limits
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("именован")
                && limit
                    .as_str()
                    .unwrap()
                    .contains("DocumentJournal.Documents.StandardCommand.OpenList")
                && limit
                    .as_str()
                    .unwrap()
                    .contains("Report.Contacts.StandardCommand.Open")
                && limit
                    .as_str()
                    .unwrap()
                    .contains("Report.Contacts.StandardCommand.Bad-Name")));
        assert!(limits
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("поврежд")
                && limit.as_str().unwrap().contains("broken-command")));
        if at.ends_with(".Command") {
            let items = data["items"].as_array().unwrap();
            assert_eq!(items.len(), 2);
            assert_eq!(items[0]["at"], "main:Catalog.Items.Command.Refresh");
            assert_eq!(items[0]["props"]["roleOverrides"], 3);
            assert_eq!(items[1]["at"], "main:CommonCommand.Open");
            assert_eq!(items[1]["props"]["visible"], false);
        }
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn malformed_interface_command_references_are_not_ids_or_supported_addresses() {
    for reference in [
        "0:not-a-uuid",
        ".Items.StandardCommand.Open",
        "Catalog.Items",
        "Catalog.Items.StandardCommand.123",
        "Catalog.Items.StandardCommand.Bad?Name",
        "Unknown.Items.StandardCommand.Open",
    ] {
        let fixture = RealReaderFixture::new();
        write(
            &fixture
                .source
                .join("Subsystems/Sales/Ext/CommandInterface.xml"),
            &format!(
                r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><CommandsVisibility><Command name="{reference}"><Visibility><xr:Common>false</xr:Common></Visibility></Command></CommandsVisibility></CommandInterface>"#
            ),
        );
        let read = fixture
            .view_service()
            .view(ViewRequest::new("main:Subsystem.Sales.Interface.Command").unwrap());
        assert!(read.ok, "{:?}", refusal_codes(&read));
        let data = read.data.unwrap();
        assert_eq!(data["items"], json!([]));
        let limits = data["limits"].as_array().unwrap();
        assert_eq!(limits.len(), 1);
        let reason = limits[0].as_str().unwrap();
        assert!(
            reason.contains("повреждённое или нераспознанное"),
            "{reference}"
        );
        assert!(reason.contains(reference));
        assert!(!reason.contains("по идентификатору"));
    }
}

#[test]
fn interface_reference_fixture_preserves_two_owners_all_orders_and_settings() {
    let fixture = RealReaderFixture::new();
    for relative in [
        "Configuration.xml",
        "Subsystems/CRMИМаркетинг.xml",
        "Subsystems/ОписаниеПроцессов.xml",
        "Subsystems/CRMИМаркетинг/Ext/CommandInterface.xml",
        "Subsystems/ОписаниеПроцессов/Ext/CommandInterface.xml",
        "CommonCommands/Open.xml",
        "CommonCommands/Settings.xml",
        "Catalogs/Items.xml",
    ] {
        write(
            &fixture.source.join(relative),
            &fixture_text(&format!(
                "acceptance/workspace-interface-reference-kind/src/{relative}"
            )),
        );
    }
    let service = fixture.view_service();
    for (owner, names) in [
        (
            "CRMИМаркетинг",
            [
                "DocumentJournal.РеестрТорговыхДокументов.StandardCommand.OpenList",
                "Report.КонтактнаяИнформация.StandardCommand.Open",
            ],
        ),
        (
            "ОписаниеПроцессов",
            [
                "Catalog.Глоссарий.StandardCommand.OpenList",
                "Constant.ИспользоватьДемонстрационныеСценарии.StandardCommand.Open",
            ],
        ),
    ] {
        let read =
            service.view(ViewRequest::new(&format!("main:Subsystem.{owner}.Interface")).unwrap());
        assert!(read.ok, "{:?}", refusal_codes(&read));
        let data = read.data.unwrap();
        for name in names {
            let reason = data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(serde_json::Value::as_str)
                .find(|text| text.contains(name))
                .unwrap();
            assert!(reason.contains("именованных"));
            assert!(!reason.contains("по идентификатору"));
        }
        if owner == "ОписаниеПроцессов" {
            assert!(data.get("branches").is_none_or(|branches| branches
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| !branch["at"].as_str().unwrap().ends_with(".Command"))));
        }
    }
    let at = "main:Subsystem.CRMИМаркетинг.Interface";
    let commands = service.view(ViewRequest::new(&format!("{at}.Command")).unwrap());
    assert!(commands.ok);
    let commands = commands.data.unwrap();
    let rows = commands["items"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["props"]["roleOverrides"], 3);
    assert_eq!(rows[1]["props"]["group"], "NavigationPanelOrdinary");
    assert_eq!(rows[1]["props"]["placement"], "Auto");
    assert_eq!(rows[2]["props"]["visible"], false);
    let groups = [
        "NavigationPanelSeeAlso",
        "NavigationPanelOrdinary",
        "NavigationPanelImportant",
        "ActionsPanelTools",
        "ActionsPanelCreate",
    ];
    let expected = [
        "DocumentJournal.РеестрТорговыхДокументов.StandardCommand.OpenList",
        "Report.КонтактнаяИнформация.StandardCommand.Open",
        "Catalog.Глоссарий.StandardCommand.OpenList",
        "Constant.ИспользоватьДемонстрационныеСценарии.StandardCommand.Open",
        "0:bdebc888-2581-426c-9f5e-c09fbba5f6e2",
        "CommonCommand.Open",
        "CommonCommand.Settings",
        "Catalog.Items.Command.Refresh",
        "CommonCommand.Open",
        "DocumentJournal.РеестрТорговыхДокументов.StandardCommand.OpenList",
    ];
    let read = service.view(ViewRequest::new(&format!("{at}.Group")).unwrap());
    assert!(read.ok);
    let data = read.data.unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 5);
    for (index, group) in groups.iter().enumerate() {
        let row = &data["items"][index];
        assert_eq!(row["title"], *group);
        assert_eq!(row["order"], index + 1);
        let read = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
        assert!(read.ok);
        assert_eq!(
            read.data.unwrap()["items"],
            json!([{"order":1,"command":expected[index*2]}, {"order":2,"command":expected[index*2+1]}])
        );
    }
    let read = service.view(ViewRequest::new(&format!("{at}.Subsystem")).unwrap());
    assert!(read.ok);
    let data = read.data.unwrap();
    let rows = data["items"].as_array().unwrap();
    assert_eq!(rows.len(), 12);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row["at"], format!("main:Subsystem.Order{index:02}"));
        assert_eq!(row["order"], index + 1);
    }
}

#[test]
fn interface_command_reference_classification_preserves_existing_unicode_names() {
    let fixture = RealReaderFixture::new();
    let names = [
        "CommonCommand.Éclair",
        "Catalog.Éléments.Command.Refresh",
        "Catalog.Items.Command.Éclair",
        "CommonCommand.Ωmega",
        "CommonCommand.E\u{0301}clair",
        "CommonCommand.Ω\u{0301}mega",
        "Catalog.Items.Command.E\u{0301}clair",
        "Catalog.E\u{0301}lements.Command.Refresh",
        "Catalog.Items.Command.Bad-Name",
    ];
    let commands = names.iter().map(|name| format!(r#"<Command name="{name}"><Visibility><xr:Common>false</xr:Common></Visibility></Command>"#)).collect::<String>();
    write(
        &fixture
            .source
            .join("Subsystems/Sales/Ext/CommandInterface.xml"),
        &format!(
            r#"<CommandInterface xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20"><CommandsVisibility>{commands}</CommandsVisibility></CommandInterface>"#
        ),
    );
    let read = fixture
        .view_service()
        .view(ViewRequest::new("main:Subsystem.Sales.Interface.Command").unwrap());
    assert!(read.ok);
    let data = read.data.unwrap();
    let items = data["items"].as_array().unwrap();
    assert_eq!(
        items.len(),
        names.len(),
        "classification must not narrow existing addressable names"
    );
    for name in names {
        let item = items
            .iter()
            .find(|item| item["at"] == format!("main:{name}"))
            .unwrap();
        assert_eq!(item["props"]["visible"], false);
    }
    assert!(data.get("limits").is_none());
}
