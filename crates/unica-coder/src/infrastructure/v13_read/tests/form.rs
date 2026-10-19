use super::*;

#[test]
fn form_table_column_event_consumes_arbitrary_depth_and_preserves_owner_address() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();
    let event_at = "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange";

    let event = service.view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event_data = event.data.as_ref().unwrap();
    assert_eq!(event_data["at"], event_at);
    assert_eq!(event_data["kind"], "Event");

    let method = service.view(
        ViewRequest::new(
            "main:Report.ParityReport.Form.MainForm.Module.Form.Method.QuantityOnChange",
        )
        .unwrap(),
    );
    assert!(method.ok, "{} {:?}", method.summary, method.diagnostics);
    let method_data = method.data.as_ref().unwrap();
    assert!(
        method_data["props"]["handles"]
            .as_array()
            .is_some_and(|handles| handles.iter().any(|handle| {
                handle["owner"] == "column"
                    && handle["at"]
                        == "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity"
                    && handle["event"] == "OnChange"
            })),
        "{method_data}"
    );
}

#[test]
fn pure_event_source_resolver_returns_relative_platform_and_nested_form_identities() {
    use crate::infrastructure::event_projection::resolve_property_event_source;
    use crate::infrastructure::logical_event_source::{
        resolve_event_source, LogicalEventSource, PropertyEventOwnerKind,
    };

    let platform = resolve_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(platform) = platform else {
        panic!("platform event must resolve to a platform module")
    };
    assert_eq!(
        platform.module_at.to_string(),
        "main:Document.Заказ.Module.Object"
    );
    assert_eq!(
        platform.module_target.as_str(),
        "Document.Заказ.ObjectModule"
    );
    assert_eq!(
        platform.module_relative,
        PathBuf::from("Documents/Заказ/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        platform.descriptor_requirements,
        vec![PathBuf::from("Documents/Заказ.xml")]
    );

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let property = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        PlatformProfile::v8_3_27(),
        &property_at,
        &form_xml,
    )
    .unwrap();
    assert_eq!(
        property.form_xml_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form.xml")
    );
    assert_eq!(
        property.module_relative,
        PathBuf::from("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl")
    );
    assert_eq!(
        property
            .owner_chain
            .iter()
            .map(|owner| owner.kind)
            .collect::<Vec<_>>(),
        vec![
            PropertyEventOwnerKind::Form,
            PropertyEventOwnerKind::Table,
            PropertyEventOwnerKind::Column,
        ]
    );
    assert_eq!(
        property.descriptor_requirements,
        vec![
            PathBuf::from("Reports/ParityReport.xml"),
            PathBuf::from("Reports/ParityReport/Forms/MainForm.xml"),
        ]
    );

    let external = resolve_event_source(
        "epf",
        SourceSetKind::ExternalProcessor,
        PlatformProfile::v8_3_27(),
        &QualifiedAddress::parse(
            "epf:ExternalDataProcessor.Import.Module.Object.Event.BeforeWrite",
        )
        .unwrap(),
    )
    .unwrap();
    let LogicalEventSource::Platform(external) = external else {
        panic!("external object event must resolve to its object module")
    };
    assert_eq!(
        external.module_relative,
        PathBuf::from("Import/Ext/ObjectModule.bsl")
    );
    assert_eq!(
        external.descriptor_requirements,
        vec![PathBuf::from("Import.xml")]
    );
}

#[test]
fn pure_event_source_resolver_requires_form_evidence_for_every_item_depth() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for raw in [
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.Page.Item.Field.Event.OnChange",
    ] {
        let error = resolve_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error.code().as_str(),
            "provider_unavailable",
            "{raw}: {error}"
        );
    }
}

#[test]
fn form_evidence_resolver_returns_complete_typed_owner_chains_at_arbitrary_depth() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::PropertyEventOwnerKind;
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">FieldChanged</Event></Events></InputField>
    <Table name="Rows"><Events><Event name="Selection">RowsSelected</Event></Events><ChildItems>
      <InputField name="Column"><Events><Event name="OnChange">ColumnChanged</Event></Events></InputField>
    </ChildItems></Table>
    <UsualGroup name="Group"><ChildItems>
      <InputField name="GroupedField"><Events><Event name="OnChange">GroupedFieldChanged</Event></Events></InputField>
    </ChildItems></UsualGroup>
    <Pages name="Tabs"><ChildItems><Page name="FirstPage"><ChildItems>
      <InputField name="PageField"><Events><Event name="OnChange">PageFieldChanged</Event></Events></InputField>
    </ChildItems></Page></ChildItems></Pages>
  </ChildItems>
</Form>"#,
        "",
    );
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let cases = [
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Field",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Table,
                PropertyEventOwnerKind::Column,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Rows",
                "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Group",
                "main:Report.ParityReport.Form.MainForm.Item.Group.Item.GroupedField",
            ],
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField.Event.OnChange",
            vec![
                PropertyEventOwnerKind::Form,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
                PropertyEventOwnerKind::Element,
            ],
            vec![
                "main:Report.ParityReport.Form.MainForm",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage",
                "main:Report.ParityReport.Form.MainForm.Item.Tabs.Item.FirstPage.Item.PageField",
            ],
        ),
    ];
    let service = fixture.view_service();
    for (raw, expected_kinds, expected_addresses) in cases {
        let at = QualifiedAddress::parse(raw).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.kind)
                .collect::<Vec<_>>(),
            expected_kinds,
            "{raw}"
        );
        assert_eq!(
            source
                .owner_chain
                .iter()
                .map(|owner| owner.at.to_string())
                .collect::<Vec<_>>(),
            expected_addresses,
            "{raw}"
        );
        let pure = project_property_event(&source, &form_xml, Some("")).unwrap();
        let physical = service.view(ViewRequest::new(raw).unwrap());
        assert!(
            physical.ok,
            "{raw}: {} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], "missing");
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn pure_event_source_resolver_fails_closed_for_unproved_event_layouts() {
    use crate::infrastructure::logical_event_source::resolve_event_source;

    for (kind, raw) in [
        (
            SourceSetKind::Extension,
            "main:Document.Заказ.Module.Object.Event.BeforeWrite",
        ),
        (
            SourceSetKind::Configuration,
            "main:WebSocketClient.Телефония.Module.WebSocketClient.Event.OnMessage",
        ),
        (
            SourceSetKind::Configuration,
            "main:CommonModule.ЗаказыСервер.Event.BeforeStart",
        ),
        (
            SourceSetKind::Configuration,
            "main:EventSubscription.Изменение.Event.Change",
        ),
        (
            SourceSetKind::Configuration,
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        ),
    ] {
        let error = resolve_event_source(
            "main",
            kind,
            PlatformProfile::v8_3_27(),
            &QualifiedAddress::parse(raw).unwrap(),
        )
        .unwrap_err();
        assert_eq!(
            error.code().as_str(),
            "provider_unavailable",
            "{raw}: {error}"
        );
    }
}

#[test]
fn pure_event_projectors_match_physical_view_service_event_nodes() {
    use crate::infrastructure::event_projection::{
        project_platform_event, project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::logical_event_source::{resolve_event_source, LogicalEventSource};
    use crate::infrastructure::v13_read::event_node_value;

    let profile = PlatformProfile::v8_3_27();
    let platform_at =
        QualifiedAddress::parse("main:Document.Заказ.Module.Object.Event.BeforeWrite").unwrap();
    for source in [
        None,
        Some("Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецПроцедуры"),
        Some("Функция ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецФункции"),
    ] {
        let fixture = RealReaderFixture::new();
        fixture.install_accepted_profile_sources();
        if let Some(source) = source {
            write(
                &fixture.source.join("Documents/Заказ/Ext/ObjectModule.bsl"),
                source,
            );
        }
        let LogicalEventSource::Platform(resolved) =
            resolve_event_source("main", SourceSetKind::Configuration, profile, &platform_at)
                .unwrap()
        else {
            panic!("platform event must resolve to a module source")
        };
        let capability = profile.module_prefix_capability(&platform_at).unwrap();
        let pure = project_platform_event(&resolved, capability, source).unwrap();
        let physical = fixture
            .view_service()
            .view(ViewRequest::new(&platform_at.to_string()).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }

    let fixture = RealReaderFixture::new();
    let property_at = QualifiedAddress::parse(
        "main:Report.ParityReport.Form.MainForm.Item.Goods.Item.Quantity.Event.OnChange",
    )
    .unwrap();
    let form_xml = fs::read_to_string(
        fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
    )
    .unwrap();
    let resolved = resolve_property_event_source(
        "main",
        SourceSetKind::Configuration,
        profile,
        &property_at,
        &form_xml,
    )
    .unwrap();
    let module_bsl = fs::read_to_string(fixture.source.join(&resolved.module_relative)).unwrap();
    let pure = project_property_event(&resolved, &form_xml, Some(&module_bsl)).unwrap();
    let physical = fixture
        .view_service()
        .view(ViewRequest::new(&property_at.to_string()).unwrap());
    assert!(
        physical.ok,
        "{} {:?}",
        physical.summary, physical.diagnostics
    );
    assert_eq!(physical.data.unwrap(), event_node_value(&pure));
}

#[test]
fn pure_form_projector_matches_every_physical_owner_and_all_event_states() {
    use crate::infrastructure::event_projection::{
        project_property_event, resolve_property_event_source,
    };
    use crate::infrastructure::v13_read::event_node_value;

    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"/>
    <Table name="Rows"><DataPath>Rows</DataPath>
      <Events><Event name="Selection">MissingSelection</Event></Events>
      <ChildItems><InputField name="Column"><Events><Event name="OnChange">WrongColumn</Event></Events></InputField></ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen">FormOpen</Event></Events>
  <Commands><Command name="Run" id="1"><Action>RunAction</Action></Command></Commands>
</Form>"#,
        concat!(
            "&AtClient\nProcedure FormOpen(Cancel)\nEndProcedure\n\n",
            "Procedure WrongColumn()\nEndProcedure\n\n",
            "&AtClient\nProcedure RunAction(Command)\nEndProcedure\n",
        ),
    );
    let form_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml");
    let module_path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form/Module.bsl");
    let form_xml = fs::read_to_string(form_path).unwrap();
    let module_bsl = fs::read_to_string(module_path).unwrap();
    let service = fixture.view_service();
    for (event_at, state) in [
        (
            "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
            "implemented",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
            "available",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
            "missing",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
            "invalid",
        ),
        (
            "main:Report.ParityReport.Form.MainForm.Command.Run.Event.Execute",
            "implemented",
        ),
    ] {
        let at = QualifiedAddress::parse(event_at).unwrap();
        let source = resolve_property_event_source(
            "main",
            SourceSetKind::Configuration,
            PlatformProfile::v8_3_27(),
            &at,
            &form_xml,
        )
        .unwrap();
        let pure = project_property_event(&source, &form_xml, Some(&module_bsl)).unwrap();
        let physical = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            physical.ok,
            "{} {:?}",
            physical.summary, physical.diagnostics
        );
        assert_eq!(physical.data.as_ref().unwrap()["props"]["state"], state);
        assert_eq!(physical.data.unwrap(), event_node_value(&pure));
    }
}

#[test]
fn production_form_command_execute_has_one_semantic_identity_across_view_and_find() {
    let fixture = RealReaderFixture::new();
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="Zero" id="1"/>
    <Command name="Missing" id="2"><Action>MissingAction</Action></Command>
    <Command name="Implemented" id="3"><Action>ImplementedAction</Action></Command>
    <Command name="Duplicate" id="4"><Action>FirstAction</Action><Action>SecondAction</Action></Command>
  </Commands>
</Form>"#,
        "&НаКлиенте\nПроцедура ImplementedAction(Команда)\nКонецПроцедуры\n",
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    let expected_contexts = json!([
        "thinClient",
        "webClient",
        "thickClientManaged",
        "mobileClient",
        "mobileAppClient"
    ]);
    for (command, state, handler, has_can) in [
        ("Zero", "available", Some("ОбработкаКоманды"), true),
        ("Missing", "missing", Some("MissingAction"), true),
        (
            "Implemented",
            "implemented",
            Some("ImplementedAction"),
            false,
        ),
        ("Duplicate", "invalid", None, false),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state);
        assert_eq!(items[0]["props"]["contexts"], expected_contexts);
        if let Some(handler) = handler {
            assert_eq!(items[0]["props"]["handler"], handler);
            let expected_signature = format!("Процедура {handler}(Команда)");
            assert_eq!(items[0]["props"]["signature"], expected_signature);
        }
        assert_eq!(
            items[0].get("can").is_some(),
            has_can,
            "{command}: {}",
            items[0]
        );
        if has_can {
            assert_eq!(items[0]["can"].as_array().unwrap().len(), 1);
            assert_eq!(items[0]["can"][0]["op"], "event.implement");
            assert_eq!(items[0]["can"][0]["args"], json!({"at": event_at}));
        }
        if state == "missing" {
            assert!(items[0]["props"]["implementationAt"].is_null());
        }
        if state == "implemented" {
            assert_eq!(
                items[0]["props"]["implementationAt"],
                format!("{form_at}.Module.Form.Method.ImplementedAction")
            );
        }

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for command in ["Zero", "Missing", "Implemented", "Duplicate"] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_form_command_execute_requires_an_exact_client_directive() {
    let fixture = RealReaderFixture::new();
    let module_bsl = concat!(
        "&НаКлиенте\n",
        "Процедура RussianClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&AtClient\n",
        "Процедура EnglishClientAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиенте\n",
        "Процедура RussianLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&нАкЛиЕнТе\n",
        "Процедура RussianMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&atclient\n",
        "Процедура EnglishLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnT\n",
        "Процедура EnglishMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "  &AtClient  \r\n",
        "Процедура WhitespaceCrLfAction(Команда)\r\n",
        "КонецПроцедуры\r\n\r\n",
        "Процедура MissingDirectiveAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&наклиентенасервере\n",
        "Процедура RussianClientServerLowerAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "&aTcLiEnTaTsErVeR\n",
        "Процедура EnglishClientServerMixedAction(Команда)\n",
        "КонецПроцедуры\n\n",
        "#Если Клиент Тогда\n",
        "Процедура GuardOnlyAction(Команда)\n",
        "КонецПроцедуры\n",
        "#КонецЕсли\n",
    );
    fixture.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands>
    <Command name="RussianClient" id="1"><Action>RussianClientAction</Action></Command>
    <Command name="EnglishClient" id="2"><Action>EnglishClientAction</Action></Command>
    <Command name="RussianLower" id="3"><Action>RussianLowerAction</Action></Command>
    <Command name="RussianMixed" id="4"><Action>RussianMixedAction</Action></Command>
    <Command name="EnglishLower" id="5"><Action>EnglishLowerAction</Action></Command>
    <Command name="EnglishMixed" id="6"><Action>EnglishMixedAction</Action></Command>
    <Command name="WhitespaceCrLf" id="7"><Action>WhitespaceCrLfAction</Action></Command>
    <Command name="MissingDirective" id="8"><Action>MissingDirectiveAction</Action></Command>
    <Command name="RussianClientServerLower" id="9"><Action>RussianClientServerLowerAction</Action></Command>
    <Command name="EnglishClientServerMixed" id="10"><Action>EnglishClientServerMixedAction</Action></Command>
    <Command name="GuardOnly" id="11"><Action>GuardOnlyAction</Action></Command>
  </Commands>
</Form>"#,
        module_bsl,
    );
    let service = fixture.view_service();
    let form_at = "main:Report.ParityReport.Form.MainForm";
    for (command, handler, state) in [
        ("RussianClient", "RussianClientAction", "implemented"),
        ("EnglishClient", "EnglishClientAction", "implemented"),
        ("RussianLower", "RussianLowerAction", "implemented"),
        ("RussianMixed", "RussianMixedAction", "implemented"),
        ("EnglishLower", "EnglishLowerAction", "implemented"),
        ("EnglishMixed", "EnglishMixedAction", "implemented"),
        ("WhitespaceCrLf", "WhitespaceCrLfAction", "implemented"),
        ("MissingDirective", "MissingDirectiveAction", "invalid"),
        (
            "RussianClientServerLower",
            "RussianClientServerLowerAction",
            "invalid",
        ),
        (
            "EnglishClientServerMixed",
            "EnglishClientServerMixedAction",
            "invalid",
        ),
        ("GuardOnly", "GuardOnlyAction", "invalid"),
    ] {
        let command_at = format!("{form_at}.Command.{command}");
        let event_at = format!("{command_at}.Event.Execute");
        let collection = service.view(ViewRequest::new(&format!("{command_at}.Event")).unwrap());
        assert!(
            collection.ok,
            "{command}: {} {:?}",
            collection.summary, collection.diagnostics
        );
        let items = collection.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap();
        assert_eq!(items.len(), 1, "{command}: {items:?}");
        assert_eq!(items[0]["at"], event_at);
        assert_eq!(items[0]["props"]["eventId"], "Execute");
        assert_eq!(items[0]["props"]["state"], state, "{command}: {}", items[0]);
        assert_eq!(
            items[0]["props"]["implementationAt"],
            format!("{form_at}.Module.Form.Method.{handler}")
        );
        assert!(items[0].get("can").is_none(), "{command}: {}", items[0]);

        let direct = service.view(ViewRequest::new(&event_at).unwrap());
        assert!(
            direct.ok,
            "{command}: {} {:?}",
            direct.summary, direct.diagnostics
        );
        assert_eq!(direct.data.as_ref().unwrap(), &items[0]);
    }

    let authority = fixture.operation_read_authority();
    let index = ReaderReach::new(vec![("main", &authority)]);
    for command in [
        "RussianClient",
        "EnglishClient",
        "RussianLower",
        "RussianMixed",
        "EnglishLower",
        "EnglishMixed",
        "WhitespaceCrLf",
        "MissingDirective",
        "RussianClientServerLower",
        "EnglishClientServerMixed",
        "GuardOnly",
    ] {
        let event_at = format!("{form_at}.Command.{command}.Event.Execute");
        let found = index.find(
            FindRequest::new(&event_at)
                .unwrap()
                .with_limit(100)
                .unwrap(),
        );
        assert!(!found.is_nearest(), "{event_at}: {found:?}");
        assert_eq!(
            found
                .candidates()
                .iter()
                .filter(|candidate| candidate.at() == event_at)
                .count(),
            1,
            "{event_at}: {found:?}"
        );
    }
}

#[test]
fn production_borrowed_command_defaults_after_and_base_form_only_has_no_can() {
    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"/></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert_eq!(event["can"][0]["op"], "event.implement");
    assert_eq!(
        event["can"][0]["args"],
        json!({"at": event_at, "callType": "After"})
    );

    let base_only = RealReaderFixture::new();
    base_only.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event = base_only
        .view_service()
        .view(ViewRequest::new("main:Report.ParityReport.Form.MainForm.Event.OnOpen").unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "available");
    assert!(event.get("can").is_none(), "{event}");
}

#[test]
fn production_form_empty_handlers_are_invalid_for_every_property_owner() {
    let regular = RealReaderFixture::new();
    regular.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <ChildItems>
    <InputField name="Field"><Events><Event name="OnChange">   </Event></Events></InputField>
    <Table name="Rows">
      <DataPath>Rows</DataPath>
      <Events><Event name="Selection"></Event></Events>
      <ChildItems>
        <InputField name="Column"><Events><Event name="OnChange">
        </Event></Events></InputField>
      </ChildItems>
    </Table>
  </ChildItems>
  <Events><Event name="OnOpen"> </Event></Events>
  <Commands><Command name="Regular" id="1"><Action>   </Action></Command></Commands>
</Form>"#,
        "",
    );
    let service = regular.view_service();
    for event_at in [
        "main:Report.ParityReport.Form.MainForm.Event.OnOpen",
        "main:Report.ParityReport.Form.MainForm.Item.Field.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Event.Selection",
        "main:Report.ParityReport.Form.MainForm.Item.Rows.Item.Column.Event.OnChange",
        "main:Report.ParityReport.Form.MainForm.Command.Regular.Event.Execute",
    ] {
        let event = service.view(ViewRequest::new(event_at).unwrap());
        assert!(
            event.ok,
            "{event_at}: {} {:?}",
            event.summary, event.diagnostics
        );
        let event = event.data.as_ref().unwrap();
        assert_eq!(event["props"]["state"], "invalid", "{event_at}: {event}");
        assert!(event.get("can").is_none(), "{event_at}: {event}");
        assert!(
            event["props"]
                .as_object()
                .unwrap()
                .contains_key("implementationAt"),
            "{event_at}: {event}"
        );
    }

    let borrowed = RealReaderFixture::new();
    borrowed.install_main_form_sources(
        r#"<?xml version="1.0" encoding="utf-8"?>
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
  <Commands><Command name="Borrowed" id="1"><Action callType="After">   </Action></Command></Commands>
  <BaseForm version="2.20"/>
</Form>"#,
        "",
    );
    let event_at = "main:Report.ParityReport.Form.MainForm.Command.Borrowed.Event.Execute";
    let event = borrowed
        .view_service()
        .view(ViewRequest::new(event_at).unwrap());
    assert!(event.ok, "{} {:?}", event.summary, event.diagnostics);
    let event = event.data.as_ref().unwrap();
    assert_eq!(event["props"]["state"], "invalid", "{event}");
    assert!(event.get("can").is_none(), "{event}");
    assert!(event["props"]
        .as_object()
        .unwrap()
        .contains_key("implementationAt"));
}

#[test]
fn form_projection_uses_a_positive_nested_scalar_allowlist() {
    let at = QualifiedAddress::parse("main:CommonForm.Test.Item.Secret").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = project_typed_payload(
        &route,
        json!({
            "name": "Test",
            "elements": [{
                "name": "Secret",
                "tag": "[Input]",
                "visible": true,
                "enabled": true,
                "readOnly": false,
                "mysteryProviderScalar": "must-not-leak",
                "events": [],
                "children": []
            }],
            "attributes": [],
            "parameters": [],
            "commands": [],
            "events": []
        }),
    )
    .unwrap();

    let serialized = serde_json::to_string(&view).unwrap();
    assert!(
        !serialized.contains("mysteryProviderScalar"),
        "{serialized}"
    );
    assert!(!serialized.contains("must-not-leak"), "{serialized}");
}

fn install_form_metadata_test_sources(fixture: &RealReaderFixture) {
    let config = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(&fixture.source.join("Configuration.xml"), &config.replace("</ChildObjects>", "<CommonForm>ОбщаяФорма</CommonForm><DataProcessor>Обработка</DataProcessor></ChildObjects>"));
    write_metadata_owner(
        &fixture.source,
        "DataProcessors",
        "DataProcessor",
        "Обработка",
        "<Form>Форма</Form>",
    );
    for (descriptor, artifact, name, content) in [
        (
            "CommonForms/ОбщаяФорма.xml",
            "CommonForm",
            "ОбщаяФорма",
            "CommonForms/ОбщаяФорма/Ext/Form.xml",
        ),
        (
            "DataProcessors/Обработка/Forms/Форма.xml",
            "Form",
            "Форма",
            "DataProcessors/Обработка/Forms/Форма/Ext/Form.xml",
        ),
    ] {
        write(
            &fixture.source.join(descriptor),
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{artifact} uuid="e9455333-3333-4333-8333-333333333333"><Properties><Name>{name}</Name><FormType>Managed</FormType></Properties></{artifact}></MetaDataObject>"#
            ),
        );
        write(
            &fixture.source.join(content),
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20"><ChildItems/></Form>"#,
        );
    }
}

#[test]
fn form_metadata_has_its_own_descriptor_properties_and_resource_identity() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    for (root, descriptor, artifact, name, uuid, content) in [
        (
            "main:CommonForm.ОбщаяФорма",
            "CommonForms/ОбщаяФорма.xml",
            "CommonForm",
            "ОбщаяФорма",
            "e9455111-1111-4111-8111-111111111111",
            "CommonForms/ОбщаяФорма/Ext/Form.xml",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма",
            "DataProcessors/Обработка/Forms/Форма.xml",
            "Form",
            "Форма",
            "e9455222-2222-4222-8222-222222222222",
            "DataProcessors/Обработка/Forms/Форма/Ext/Form.xml",
        ),
    ] {
        write(
            &fixture.source.join(descriptor),
            &format!(
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" version="2.20"><{artifact} uuid="{uuid}"><Properties><Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Описание формы</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Form declaration</v8:content></v8:item></Synonym><ExtendedPresentation><v8:item><v8:lang>ru</v8:lang><v8:content>Расширенное представление</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Extended presentation</v8:content></v8:item></ExtendedPresentation><Explanation><v8:item><v8:lang>ru</v8:lang><v8:content>Пояснение</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>Explanation</v8:content></v8:item></Explanation><FutureProperty><Nested>preserved source</Nested></FutureProperty><Comment>Only descriptor properties</Comment><FormType>Managed</FormType><UseStandardCommands>false</UseStandardCommands><UsePurposes><v8:Value xsi:type="app:ApplicationUsePurpose">PlatformApplication</v8:Value><v8:Value xsi:type="app:ApplicationUsePurpose">MobilePlatformApplication</v8:Value></UsePurposes></Properties></{artifact}></MetaDataObject>"#
            ),
        );
        let service = fixture.view_service();
        let metadata_at = format!("{root}.Metadata");
        let view = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(view.ok, "{metadata_at}: {view:?}");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert_eq!(data["props"]["uuid"], uuid);
        assert_eq!(data["props"]["Name"], name);
        assert_eq!(data["props"]["Comment"], "Only descriptor properties");
        assert_eq!(data["props"]["FormType"], "Managed");
        assert_eq!(data["props"]["UseStandardCommands"], false);
        assert_eq!(
            data["props"]["UsePurposes"],
            json!(["PlatformApplication", "MobilePlatformApplication"])
        );
        assert_eq!(
            data["props"]["Synonym"],
            json!({"kind":"localizedString","values":[{"language":"ru","content":"Описание формы"},{"language":"en","content":"Form declaration"}]})
        );
        for (key, ru, en) in [
            (
                "ExtendedPresentation",
                "Расширенное представление",
                "Extended presentation",
            ),
            ("Explanation", "Пояснение", "Explanation"),
        ] {
            assert_eq!(
                data["props"][key],
                json!({"kind":"localizedString","values":[{"language":"ru","content":ru},{"language":"en","content":en}]})
            );
        }
        assert!(data["limits"].as_array().unwrap().iter().any(|limit| limit
            .as_str()
            .is_some_and(|text| text.contains("FutureProperty"))));
        assert!(data["props"].get("FutureProperty").is_none());
        let root_view = service.view(ViewRequest::new(root).unwrap());
        assert!(root_view.ok);
        let root_data = root_view.data.unwrap();
        assert_eq!(root_data["kind"], "Form");
        assert!(root_data["props"].get("Comment").is_none());
        assert!(root_data["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|branch| branch["at"] == metadata_at));
        let authority = fixture.read_authority();
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(root).unwrap(), None)
                .unwrap(),
            content
        );
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(&metadata_at).unwrap(), None)
                .unwrap(),
            descriptor
        );
    }
}

#[test]
fn inherited_form_metadata_is_local_without_claiming_a_local_form_payload() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let service = fixture.extension_view_service();
    for (root, descriptor) in [
        ("main:CommonForm.ОбщаяФорма", "CommonForms/ОбщаяФорма.xml"),
        (
            "main:DataProcessor.Обработка.Form.Форма",
            "DataProcessors/Обработка/Forms/Форма.xml",
        ),
    ] {
        let metadata_at = format!("{root}.Metadata");
        let metadata = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(metadata.ok, "{metadata_at}: {metadata:?}");
        let data = metadata.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert!(data["props"]["uuid"].is_string());
        assert!(data["props"]["Name"].is_string());
        assert!(data["props"].get("content").is_none());
        let form = service.view(ViewRequest::new(root).unwrap());
        assert!(form.ok);
        assert_eq!(form.data.unwrap()["props"]["content"], "inherited");
        let authority = fixture.extension_read_authority();
        assert_eq!(
            authority
                .existing_export_path(&QualifiedAddress::parse(&metadata_at).unwrap(), None)
                .unwrap(),
            descriptor
        );
        assert!(authority
            .existing_export_path(&QualifiedAddress::parse(root).unwrap(), None)
            .is_err());
    }
}

#[test]
fn form_metadata_rejects_orphans_and_does_not_follow_descriptor_links() {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    copy_fixture(
        "acceptance/workspace-resolve-resources/src/CommonForms/Orphan.xml",
        &fixture.source.join("CommonForms/Orphan.xml"),
    );
    let service = fixture.view_service();
    let orphan = service.view(ViewRequest::new("main:CommonForm.Orphan.Metadata").unwrap());
    assert!(!orphan.ok);
    assert_eq!(orphan.diagnostics[0]["code"], "not_found");
    for raw in [
        "main:CommonForm.ОбщаяФорма.Metadata.Item.X",
        "main:DataProcessor.Обработка.Metadata",
        "main:DataProcessor.Обработка.Form.Metadata",
    ] {
        let refused = ViewRequest::new(raw).map(|request| service.view(request));
        assert!(refused.is_err() || !refused.unwrap().ok, "{raw}");
    }
    {
        use crate::infrastructure::platform::testing::{
            create_file_link_fixture_for_test, FileLinkFixtureOutcome,
        };
        let descriptor = fixture.source.join("CommonForms/ОбщаяФорма.xml");
        let outside = fixture.root.path().join("outside-form.xml");
        fs::rename(&descriptor, &outside).unwrap();
        let outcome = create_file_link_fixture_for_test(&outside, &descriptor).unwrap();
        if outcome != FileLinkFixtureOutcome::Created {
            eprintln!("[SKIPPED FIXTURE] file link unavailable: {outcome:?}");
            return;
        }
        let fresh = fixture.view_service();
        let refused = fresh.view(ViewRequest::new("main:CommonForm.ОбщаяФорма.Metadata").unwrap());
        assert!(
            !refused.ok,
            "metadata must stay under the retained no-follow root"
        );
    }
}

#[test]
fn external_form_metadata_uses_the_local_descriptor_without_a_companion() {
    let fixture = RealExternalReaderFixture::new();
    for (source, kind, root, descriptor) in [
        (
            "artifact_processor",
            SourceSetKind::ExternalProcessor,
            "ExternalDataProcessor.Import.Form.Main",
            fixture.processor.join("Import/Forms/Main.xml"),
        ),
        (
            "artifact_report",
            SourceSetKind::ExternalReport,
            "ExternalReport.Sales.Form.Main",
            fixture.report.join("Sales/Forms/Main.xml"),
        ),
    ] {
        let form_xml = descriptor.with_extension("").join("Ext/Form.xml");
        fs::remove_file(form_xml).unwrap();
        let service = ViewService::new(
            fixture.read_authority(source, kind),
            ViewCursorStore::default(),
        );
        let metadata_at = format!("{source}:{root}.Metadata");
        let view = service.view(ViewRequest::new(&metadata_at).unwrap());
        assert!(view.ok, "{metadata_at}: {view:?}");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], "Metadata");
        assert_eq!(data["props"]["Name"], "Main");
        assert!(data["props"].get("Comment").is_none());
        assert!(data["props"].get("UseStandardCommands").is_none());
        assert_eq!(
            data["props"]["uuid"],
            "10000000-0000-4000-8000-000000000011"
        );
        assert!(
            !service
                .view(ViewRequest::new(&format!("{source}:{root}")).unwrap())
                .ok
        );
    }
}
