use super::*;

/// #1381: платформа 8.3.27 выгружает заимствованные роль, формы, макеты и
/// пакет XDTO без `Rights.xml`, `Form.xml`, `Template.xml` и `Package.bin`.
/// Такой узел читается по описанию: заимствование, `content: "inherited"`,
/// ни счёта, ни ветвей содержимого. Адрес внутри содержимого отказывает
/// `not_found`, а не `source_unreadable`.
/// rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
#[test]
fn borrowed_objects_without_companion_files_read_their_content_as_inherited() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let service = fixture.extension_view_service();
    for (at, kind) in [
        ("main:Role.SalesReader", "Role"),
        ("main:CommonForm.ОбщаяФорма", "Form"),
        ("main:DataProcessor.Обработка.Form.Форма", "Form"),
        ("main:XDTOPackage.ПакетОбмена", "XDTOPackage"),
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: an inherited node must be readable");
        let data = view.data.unwrap();
        assert_eq!(data["kind"], kind, "{at}");
        assert_eq!(data["props"]["content"], "inherited", "{at}");
        assert_eq!(data["props"]["belonging"], "borrowed", "{at}");
        assert!(
            data["props"]["parentId"].is_string(),
            "{at}: the parent stays addressable"
        );
        if kind == "Form" {
            assert_eq!(
                data["branches"],
                json!([{"at":format!("{at}.Metadata"),"count":1}]),
                "{at}: only the local declaration is reachable, no inherited content or Module"
            );
        } else {
            assert!(
                data.get("branches").is_none(),
                "{at}: no inherited content branches"
            );
        }
    }
    assert_eq!(
        service
            .view(ViewRequest::new("main:Role.SalesReader").unwrap())
            .data
            .unwrap()["props"]["overrides"],
        "Rights"
    );
    for at in [
        "main:DataProcessor.Обработка.Template.Макет",
        "main:DataProcessor.Обработка.Template.СхемаКомпоновки",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: an inherited template must be readable");
        let data = view.data.unwrap();
        // The extension descriptor may carry the platform's default
        // TemplateType instead of the parent's: the format stays unknown.
        assert_eq!(data["props"]["format"], serde_json::Value::Null, "{at}");
        assert_eq!(data["props"]["content"], "inherited", "{at}");
        assert_eq!(data["props"]["belonging"], "borrowed", "{at}");
        assert!(data["props"]["parentId"].is_string(), "{at}");
        assert!(data.get("branches").is_none(), "{at}");
    }
    for (at, resource, owner) in [
        (
            "main:Role.SalesReader.Right",
            "Rights.xml",
            "main:Role.SalesReader",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Item",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Module",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:CommonForm.ОбщаяФорма.Module.Form",
            "Form.xml",
            "main:CommonForm.ОбщаяФорма",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма.Attribute",
            "Form.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Form.Форма.Module",
            "Form.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Template.СхемаКомпоновки.DataSet",
            "Template.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:DataProcessor.Обработка.Template.Макет.Area",
            "Template.xml",
            "main:DataProcessor.Обработка",
        ),
        (
            "main:XDTOPackage.ПакетОбмена.Type",
            "Package.bin",
            "main:XDTOPackage.ПакетОбмена",
        ),
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: inherited content has no interior here");
        assert_eq!(view.diagnostics[0]["code"], "not_found", "{at}");
        let way = format!("props.extends of `{owner}`");
        assert!(
            view.diagnostics[0]["message"]
                .as_str()
                .is_some_and(|message| message.contains(resource) && message.contains(&way)),
            "{at}: the refusal names the absent file and the extension object naming the parent"
        );
    }
    let owner = service.view(ViewRequest::new("main:DataProcessor.Обработка").unwrap());
    assert!(owner.ok, "the borrowed owner of inherited children reads");

    // Source location of a method reads the module projection directly, not
    // through the node reader: the form without `Form.xml` refuses it the same way.
    let authority = fixture.extension_read_authority();
    let method =
        QualifiedAddress::parse("main:CommonForm.ОбщаяФорма.Module.Form.Method.ПР_А").unwrap();
    let snapshot = authority.snapshot(&method).unwrap();
    let error = authority
        .source_lines(&method, &snapshot)
        .expect_err("a form module the platform does not keep has no lines");
    assert_eq!(error.code(), RefusalCode::NotFound);
    assert!(error.to_string().contains("Form.xml"));
}

/// #1381: послабление касается только заимствованного объекта расширения.
/// В конфигурации, у собственного объекта расширения и у заимствованного
/// объекта со своим файлом отсутствие файла остаётся ошибкой источника, а
/// присутствующий файл читается как прежде.
/// rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
#[test]
fn missing_companion_file_stays_a_source_error_outside_borrowing() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let configuration = fixture.view_service();
    for at in [
        "main:Role.SalesReader",
        "main:CommonForm.ОбщаяФорма",
        "main:XDTOPackage.ПакетОбмена",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let view = configuration.view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: a configuration object needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    for (descriptor, at) in [
        ("Roles/SalesReader.xml", "main:Role.SalesReader"),
        ("CommonForms/ОбщаяФорма.xml", "main:CommonForm.ОбщаяФорма"),
        (
            "XDTOPackages/ПакетОбмена.xml",
            "main:XDTOPackage.ПакетОбмена",
        ),
        (
            "DataProcessors/Обработка/Templates/Макет.xml",
            "main:DataProcessor.Обработка.Template.Макет",
        ),
    ] {
        let own = RealReaderFixture::new();
        install_inherited_objects(&own);
        let path = own.source.join(descriptor);
        let text = fs::read_to_string(&path).unwrap();
        let start = text.find("<ExtendedConfigurationObject>").unwrap();
        let end = text.find("</ExtendedConfigurationObject>").unwrap()
            + "</ExtendedConfigurationObject>".len();
        write(&path, &format!("{}{}", &text[..start], &text[end..]));
        let view = own
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: an own extension object needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    // Заимствованный ребёнок собственного владельца ничего не наследует:
    // у владельца нет родителя.
    let own_owner = RealReaderFixture::new();
    install_inherited_objects(&own_owner);
    let path = own_owner.source.join("DataProcessors/Обработка.xml");
    let text = fs::read_to_string(&path).unwrap();
    let start = text.find("<ExtendedConfigurationObject>").unwrap();
    let end = text.find("</ExtendedConfigurationObject>").unwrap()
        + "</ExtendedConfigurationObject>".len();
    write(&path, &format!("{}{}", &text[..start], &text[end..]));
    for at in [
        "main:DataProcessor.Обработка.Form.Форма",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let view = own_owner
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(!view.ok, "{at}: a child of an own owner needs its file");
        assert_eq!(
            view.diagnostics[0]["detailCode"], "source_unreadable",
            "{at}"
        );
    }

    // Маркер без UUID родителя заимствования не доказывает.
    let marked = RealReaderFixture::new();
    install_inherited_objects(&marked);
    let path = marked.source.join("Roles/SalesReader.xml");
    let text = fs::read_to_string(&path).unwrap();
    let start =
        text.find("<ExtendedConfigurationObject>").unwrap() + "<ExtendedConfigurationObject>".len();
    let end = text.find("</ExtendedConfigurationObject>").unwrap();
    write(&path, &format!("{}parent{}", &text[..start], &text[end..]));
    let view = marked
        .extension_view_service()
        .view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(
        !view.ok,
        "a marker without a parent UUID proves no borrowing"
    );
    assert_eq!(view.diagnostics[0]["detailCode"], "source_unreadable");

    copy_fixture(
        "acceptance/workspace-borrowed/src/cf/Roles/SalesReader/Ext/Rights.xml",
        &fixture.source.join("Roles/SalesReader/Ext/Rights.xml"),
    );
    let view = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:Role.SalesReader").unwrap());
    assert!(view.ok, "a borrowed role with its own rights reads them");
    let data = view.data.unwrap();
    assert!(data["props"].get("content").is_none());
    assert_eq!(data["props"]["totalallowed"], 0);
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn missing_or_damaged_code_type_limits_only_dependent_predefined_read() {
    // Synthetic combinations exercise the reader; platform acceptance of
    // borrowed predefined data without CodeType has not been established.
    for replacement in [
        "",
        "<CodeType/>",
        "<CodeType>broken</CodeType>",
        "<CodeType>String</CodeType><CodeType>Number</CodeType>",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains("<CodeType>String</CodeType>"));
        let descriptor = original.replace("<CodeType>String</CodeType>", replacement);
        write(&path, &descriptor);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
        assert!(result.ok, "the read must succeed");
        let data = result.data.unwrap();
        assert_eq!(data["props"]["CodeLength"], 11);
        assert!(data["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("predefinedItems")));
        assert_eq!(
            data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains("properties.CodeType")),
            !replacement.is_empty()
        );
        let dependent =
            service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
        assert!(!dependent.ok, "the dependent read must be refused");
        assert_eq!(fs::read_to_string(path).unwrap(), descriptor);
    }
}

/// #1381 (вторая вычитка): неизвестное сообщается пределом узла. Регистр без
/// `WriteMode` не утверждает список регистраторов, модуль без флагов среды —
/// контексты; ребёнок заимствованного владельца называет, где разрешается
/// его родитель.
/// rule: INV.SOURCE.BORROWED-OBJECT-INHERITED-CONTENT
/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn unknown_facts_of_borrowed_objects_are_named_as_node_limits() {
    let fixture = RealReaderFixture::new();
    install_inherited_objects(&fixture);
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<InformationRegister>CorpusRegister</InformationRegister><CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let register =
        fixture_text("platform_8_3_27/incomplete_borrowed/extension-information-register.xml");
    let register_path = fixture
        .source
        .join("InformationRegisters/CorpusRegister.xml");
    write(&register_path, &register);
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml"),
    );
    write(
        &fixture
            .source
            .join("CommonModules/CorpusModule/Ext/Module.bsl"),
        "Процедура Проверка() Экспорт\nКонецПроцедуры\n",
    );
    let limits = |at: &str| -> Vec<String> {
        let view = fixture
            .extension_view_service()
            .view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
        view.data.unwrap()["limits"]
            .as_array()
            .map(|limits| {
                limits
                    .iter()
                    .filter_map(|limit| limit.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let registrars = |limits: &[String]| {
        limits
            .iter()
            .any(|limit| limit.contains("relations.registrars") && limit.contains("WriteMode"))
    };
    assert!(registrars(&limits(
        "main:InformationRegister.CorpusRegister"
    )));
    assert!(registrars(&limits(
        "main:InformationRegister.CorpusRegister.Relation"
    )));
    write(
        &register_path,
        &register.replace(
            "</Properties>",
            "<WriteMode>Independent</WriteMode></Properties>",
        ),
    );
    assert!(!registrars(&limits(
        "main:InformationRegister.CorpusRegister"
    )));

    let contexts = |limits: &[String]| {
        limits
            .iter()
            .any(|limit| limit == super::super::UNKNOWN_CONTEXTS_LIMIT)
    };
    assert!(contexts(&limits("main:CommonModule.CorpusModule")));
    assert!(contexts(&limits("main:CommonModule.CorpusModule.Method")));
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml")
            .replace("СерверныеФункции", "CorpusModule"),
    );
    assert!(!contexts(&limits("main:CommonModule.CorpusModule")));

    let service = fixture.extension_view_service();
    for at in [
        "main:DataProcessor.Обработка.Form.Форма",
        "main:DataProcessor.Обработка.Template.Макет",
    ] {
        let data = service.view(ViewRequest::new(at).unwrap()).data.unwrap();
        assert!(
            data["props"]["parentReason"].as_str().is_some_and(
                |reason| reason.contains("props.extends of `main:DataProcessor.Обработка`")
            ),
            "{at}: the child names where its parent is resolved"
        );
    }
    let role = service
        .view(ViewRequest::new("main:Role.SalesReader").unwrap())
        .data
        .unwrap();
    assert!(role["props"].get("parentReason").is_none());
}

/// #1924: заимствованный план обмена без своего `Ext/Content.xml` не знает
/// состава в расширении — его несёт план родителя. Чтение не выдаёт пустой
/// список за известный состав.
#[test]
fn a_borrowed_exchange_plan_without_its_own_content_does_not_read_as_empty() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<ExchangePlan>Узлы</ExchangePlan><ExchangePlan>Собственный</ExchangePlan></ChildObjects>",
        ),
    );
    // Собственный план расширения без `Ext/Content.xml` — доказанно пустой состав.
    write(
        &fixture.source.join("ExchangePlans/Собственный.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
	<ExchangePlan uuid="2f0e0000-0000-4000-8000-000000000302">
		<Properties>
			<Name>Собственный</Name>
			<Comment/>
		</Properties>
		<ChildObjects/>
	</ExchangePlan>
</MetaDataObject>"#,
    );
    write(
        &fixture.source.join("ExchangePlans/Узлы.xml"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
	<ExchangePlan uuid="2f0e0000-0000-4000-8000-000000000301">
		<Properties>
			<ObjectBelonging>Adopted</ObjectBelonging>
			<Name>Узлы</Name>
			<Comment/>
			<ExtendedConfigurationObject>1f0e0000-0000-4000-8000-000000000301</ExtendedConfigurationObject>
		</Properties>
		<ChildObjects/>
	</ExchangePlan>
</MetaDataObject>"#,
    );
    let service = fixture.extension_view_service();
    let content_limit = "relations.content is absent; the relation list may be incomplete";
    let limits = |at: &str| {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}");
        view.data.unwrap()["limits"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };
    for at in ["main:ExchangePlan.Узлы", "main:ExchangePlan.Узлы.Relation"] {
        assert!(limits(at).contains(&json!(content_limit)), "{at}");
    }
    for at in [
        "main:ExchangePlan.Собственный",
        "main:ExchangePlan.Собственный.Relation",
    ] {
        assert!(!limits(at).contains(&json!(content_limit)), "{at}");
    }
}
