use super::*;

/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_keeps_missing_privileged_unknown() {
    let borrowed = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml");
    let props =
        super::super::common_module_properties(borrowed.as_bytes(), SourceSetKind::Extension)
            .expect("platform borrowed CommonModule omits Privileged");
    let serialized = serde_json::to_value(props).unwrap();
    assert_eq!(serialized["privileged"], serde_json::Value::Null);

    let ordinary = fixture_text("platform_8_3_27/cfe_borrow/parent-common-module.xml");
    let missing = ordinary.replace("<Privileged>false</Privileged>", "");
    assert_eq!(
        super::super::common_module_properties(missing.as_bytes(), SourceSetKind::Configuration)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable,
    );
    assert_eq!(
        super::super::common_module_properties(borrowed.as_bytes(), SourceSetKind::Configuration)
            .unwrap_err()
            .code(),
        RefusalCode::ProviderUnavailable,
    );
    let own_extension = borrowed.replace(
        "<ExtendedConfigurationObject>ac847dc9-e222-45cf-af4a-6fa863c919a8</ExtendedConfigurationObject>",
        "",
    );
    assert!(
        super::super::common_module_properties(own_extension.as_bytes(), SourceSetKind::Extension)
            .is_err(),
        "an extension's own CommonModule still requires Privileged",
    );
    for (raw, expected) in [("true", true), ("false", false)] {
        let explicit = borrowed.replace(
            "<ReturnValuesReuse>",
            &format!("<Privileged>{raw}</Privileged><ReturnValuesReuse>"),
        );
        let props =
            super::super::common_module_properties(explicit.as_bytes(), SourceSetKind::Extension)
                .unwrap();
        assert_eq!(props.privileged, Some(expected));
    }
    for raw in ["", "unknown"] {
        let invalid = borrowed.replace(
            "<ReturnValuesReuse>",
            &format!("<Privileged>{raw}</Privileged><ReturnValuesReuse>"),
        );
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "explicit invalid Privileged must not become null",
        );
    }
}

/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_view_serializes_unknown_privileged_without_losing_contexts() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    write(
        &fixture.source.join("CommonModules/CorpusModule.xml"),
        &fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml"),
    );
    write(
        &fixture
            .source
            .join("CommonModules/CorpusModule/Ext/Module.bsl"),
        "Процедура Проверка() Экспорт\nКонецПроцедуры\n",
    );
    let view = fixture
        .extension_view_service()
        .view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    assert_eq!(
        data["props"]["commonModule"]["privileged"],
        serde_json::Value::Null
    );
    assert!(data["props"]["commonModule"]["server"].is_boolean());
    assert!(data["branches"].is_array());
}

/// Платформа 8.3.27 выгружает заимствованный общий модуль без любого
/// из восьми свойств. Отсутствие читается как неизвестное, не как значение
/// по умолчанию; у обычного модуля и у собственного модуля расширения оно
/// остаётся ошибкой источника, записанное некорректное значение — тоже.
/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_reads_platform_absent_properties_as_unknown() {
    let inherited =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-inherited.xml");
    let props =
        super::super::common_module_properties(inherited.as_bytes(), SourceSetKind::Extension)
            .expect("platform borrowed CommonModule without properties");
    let serialized = serde_json::to_value(&props).unwrap();
    for key in COMMON_MODULE_PROPERTY_KEYS {
        assert_eq!(serialized[key], serde_json::Value::Null, "{key}");
    }
    assert!(!props.execution_environment_known());

    let environment =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml");
    let props =
        super::super::common_module_properties(environment.as_bytes(), SourceSetKind::Extension)
            .expect("platform borrowed CommonModule without ReturnValuesReuse");
    assert_eq!(props.global, Some(false));
    assert_eq!(props.client_managed_application, Some(false));
    assert_eq!(props.server, Some(true));
    assert_eq!(props.external_connection, Some(true));
    assert_eq!(props.client_ordinary_application, Some(false));
    assert_eq!(props.server_call, Some(true));
    assert_eq!(props.privileged, None);
    assert_eq!(props.return_values_reuse, None);
    assert!(props.execution_environment_known());

    let parent = fixture_text("platform_8_3_27/cfe_borrow/parent-common-module-cache.xml");
    let props =
        super::super::common_module_properties(parent.as_bytes(), SourceSetKind::Configuration)
            .unwrap();
    assert_eq!(props.return_values_reuse.as_deref(), Some("DuringRequest"));
    assert_eq!(props.privileged, Some(true));
    for (tag, value) in [
        ("ReturnValuesReuse", "DuringRequest"),
        ("Global", "false"),
        ("Server", "true"),
    ] {
        let missing = parent.replace(&format!("<{tag}>{value}</{tag}>"), "");
        assert_ne!(missing, parent, "{tag}");
        let error = super::super::common_module_properties(
            missing.as_bytes(),
            SourceSetKind::Configuration,
        )
        .unwrap_err();
        assert_eq!(error.code(), RefusalCode::ProviderUnavailable, "{tag}");
        assert!(error.to_string().contains(tag), "{tag}: {error}");
    }
    assert!(
        super::super::common_module_properties(inherited.as_bytes(), SourceSetKind::Configuration)
            .is_err(),
        "outside an extension the borrowing marker proves nothing",
    );
    let own_extension = inherited.replace(
        "<ExtendedConfigurationObject>3f6c2a10-5b7e-4c1d-9e2f-0a1b2c3d4e51</ExtendedConfigurationObject>",
        "",
    );
    assert_ne!(own_extension, inherited);
    assert!(
        super::super::common_module_properties(own_extension.as_bytes(), SourceSetKind::Extension)
            .is_err(),
        "an extension's own CommonModule still requires every property",
    );
    for (tag, original, invalid) in [
        (
            "ReturnValuesReuse",
            "<ServerCall>true</ServerCall>",
            "<ServerCall>true</ServerCall><ReturnValuesReuse></ReturnValuesReuse>",
        ),
        ("Global", "<Global>false</Global>", "<Global>maybe</Global>"),
        ("Server", "<Server>true</Server>", "<Server></Server>"),
    ] {
        let invalid = environment.replace(original, invalid);
        assert_ne!(invalid, environment, "{tag}");
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "explicit invalid {tag} must not become null",
        );
    }
}

/// Заимствованный модуль без свойств читается, его методы доступны.
/// Контексты без флагов среды неизвестны и сериализуются `null`, фильтр по
/// контексту отказывает типизированно; при записанных флагах контексты
/// вычисляются как прежде, а неизвестные свойства остаются `null`.
/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_view_keeps_absent_properties_and_contexts_unknown() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>КешЗначений</CommonModule><CommonModule>СерверныеФункции</CommonModule></ChildObjects>",
        ),
    );
    for (name, descriptor) in [
        ("КешЗначений", "extension-common-module-inherited.xml"),
        (
            "СерверныеФункции",
            "extension-common-module-environment-only.xml",
        ),
    ] {
        write(
            &fixture.source.join(format!("CommonModules/{name}.xml")),
            &fixture_text(&format!("platform_8_3_27/cfe_borrow/{descriptor}")),
        );
        write(
            &fixture
                .source
                .join(format!("CommonModules/{name}/Ext/Module.bsl")),
            "Функция ПР_Удвоенное() Экспорт\n\tВозврат 2;\nКонецФункции\n",
        );
    }
    let service = fixture.extension_view_service();

    let view = service.view(ViewRequest::new("main:CommonModule.КешЗначений").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    for key in COMMON_MODULE_PROPERTY_KEYS {
        assert_eq!(
            data["props"]["commonModule"][key],
            serde_json::Value::Null,
            "{key}"
        );
    }
    let method = service
        .view(ViewRequest::new("main:CommonModule.КешЗначений.Method.ПР_Удвоенное").unwrap());
    assert!(method.ok, "method must succeed");
    let method = method.data.unwrap();
    assert_eq!(method["kind"], "Method");
    assert_eq!(method["props"]["export"], true);
    assert_eq!(
        method["props"]["compile"]["contexts"],
        serde_json::Value::Null,
        "contexts of a borrowed module without environment flags are unknown"
    );
    let body = service
        .view(ViewRequest::new("main:CommonModule.КешЗначений.Method.ПР_Удвоенное.Body").unwrap());
    assert!(body.ok, "body must succeed");
    let methods = service.view(ViewRequest::new("main:CommonModule.КешЗначений.Method").unwrap());
    assert!(methods.ok, "methods of a borrowed module stay readable");
    assert_eq!(
        methods.data.unwrap()["items"][0]["props"]["compile"]["contexts"],
        serde_json::Value::Null,
        "the collection keeps the unknown contexts of its methods"
    );

    let view = service.view(ViewRequest::new("main:CommonModule.СерверныеФункции").unwrap());
    assert!(view.ok, "view must succeed");
    let common = &view.data.as_ref().unwrap()["props"]["commonModule"];
    assert_eq!(common["server"], true);
    assert_eq!(common["externalConnection"], true);
    assert_eq!(common["returnValuesReuse"], serde_json::Value::Null);
    assert_eq!(common["privileged"], serde_json::Value::Null);
    let method = service
        .view(ViewRequest::new("main:CommonModule.СерверныеФункции.Method.ПР_Удвоенное").unwrap());
    assert!(method.ok, "method must succeed");
    assert_eq!(
        method.data.unwrap()["props"]["compile"]["contexts"],
        json!([
            "server",
            "mobileAppServer",
            "mobileStandaloneServer",
            "externalConnection"
        ])
    );
}

/// Контексты известны только при всех четырёх флагах среды.
/// Без любого одного из них контексты метода — `null`, а остальные
/// записанные флаги читаются как есть.
/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_without_any_single_environment_flag_has_unknown_contexts() {
    let environment =
        fixture_text("platform_8_3_27/cfe_borrow/extension-common-module-environment-only.xml");
    for (flag, element) in [
        (
            "clientManagedApplication",
            "<ClientManagedApplication>false</ClientManagedApplication>",
        ),
        (
            "clientOrdinaryApplication",
            "<ClientOrdinaryApplication>false</ClientOrdinaryApplication>",
        ),
        ("server", "<Server>true</Server>"),
        (
            "externalConnection",
            "<ExternalConnection>true</ExternalConnection>",
        ),
    ] {
        let descriptor = environment.replace(element, "");
        assert_ne!(descriptor, environment, "{flag}");
        let props =
            super::super::common_module_properties(descriptor.as_bytes(), SourceSetKind::Extension)
                .unwrap();
        assert!(!props.execution_environment_known(), "{flag}");

        let fixture = RealReaderFixture::new();
        let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
        write(
            &fixture.source.join("Configuration.xml"),
            &configuration.replace(
                "</ChildObjects>",
                "<CommonModule>СерверныеФункции</CommonModule></ChildObjects>",
            ),
        );
        write(
            &fixture.source.join("CommonModules/СерверныеФункции.xml"),
            &descriptor,
        );
        write(
            &fixture
                .source
                .join("CommonModules/СерверныеФункции/Ext/Module.bsl"),
            "Функция ПР_Разность(А, Б) Экспорт\n\tВозврат А - Б;\nКонецФункции\n",
        );
        let service = fixture.extension_view_service();
        let module = service.view(ViewRequest::new("main:CommonModule.СерверныеФункции").unwrap());
        assert!(module.ok, "{flag}: module view must succeed");
        let common = &module.data.as_ref().unwrap()["props"]["commonModule"];
        assert_eq!(common[flag], serde_json::Value::Null, "{flag}");
        assert_eq!(common["serverCall"], true, "{flag}");
        let method = service.view(
            ViewRequest::new("main:CommonModule.СерверныеФункции.Method.ПР_Разность").unwrap(),
        );
        assert!(method.ok, "{flag}: method view must succeed");
        assert_eq!(
            method.data.unwrap()["props"]["compile"]["contexts"],
            serde_json::Value::Null,
            "{flag}"
        );
    }
}

/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn borrowed_common_module_missing_reuse_preserves_methods_and_source() {
    // Synthetic omission; it is not a platform export.
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let descriptor = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml")
        .replace("<ReturnValuesReuse>DontUse</ReturnValuesReuse>", "");
    let path = fixture.source.join("CommonModules/CorpusModule.xml");
    write(&path, &descriptor);
    let module = fixture
        .source
        .join("CommonModules/CorpusModule/Ext/Module.bsl");
    let source = "Процедура Проверка() Экспорт\nКонецПроцедуры\n";
    write(&module, source);
    let service = fixture.extension_view_service();
    let view = service.view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    assert_eq!(
        data["props"]["commonModule"]["returnValuesReuse"],
        serde_json::Value::Null
    );
    for at in [
        "main:CommonModule.CorpusModule.Method",
        "main:CommonModule.CorpusModule.Method.Проверка",
        "main:CommonModule.CorpusModule.Body",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
    }
    assert_eq!(fs::read_to_string(path).unwrap(), descriptor);
    assert_eq!(fs::read_to_string(module).unwrap(), source);
}

/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn common_module_present_damage_is_not_missing() {
    // Synthetic corruptions exercise reader boundaries, not platform validity.
    let borrowed = fixture_text("platform_8_3_27/cfe_borrow/extension-common-module.xml");
    for value in [
        "<Server/>",
        "<Server>maybe</Server>",
        "<Server>false</Server><Server>true</Server>",
        "<Server xmlns=\"urn:foreign\">false</Server>",
        "<Server><Nested>false</Nested></Server>",
        "<Server extra=\"false\">false</Server>",
    ] {
        let invalid = borrowed.replace("<Server>false</Server>", value);
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "{value}"
        );
    }
    for value in [
        "<ReturnValuesReuse/>",
        "<ReturnValuesReuse><Nested>DontUse</Nested></ReturnValuesReuse>",
    ] {
        let invalid = borrowed.replace("<ReturnValuesReuse>DontUse</ReturnValuesReuse>", value);
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "{value}"
        );
    }
    // The identity is proved on the exact structure: one CommonModule object
    // with one direct Properties container of the metadata namespace.
    let structural = [
        borrowed
            .replacen("<CommonModule ", "<Catalog ", 1)
            .replacen("</CommonModule>", "</Catalog>", 1),
        borrowed.replacen("<Properties>", "<Properties xmlns=\"urn:foreign\">", 1),
        borrowed.replacen("</Properties>", "</Properties><Properties/>", 1),
    ];
    for invalid in structural {
        assert_ne!(invalid, borrowed);
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "a damaged descriptor structure must not open the borrowed absence policy"
        );
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn platform_borrowed_common_module_preserves_unknown_contexts_and_independent_facts() {
    let fixture = RealReaderFixture::new();
    let configuration = fs::read_to_string(fixture.source.join("Configuration.xml")).unwrap();
    write(
        &fixture.source.join("Configuration.xml"),
        &configuration.replace(
            "</ChildObjects>",
            "<CommonModule>CorpusModule</CommonModule></ChildObjects>",
        ),
    );
    let descriptor =
        fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml");
    let path = fixture.source.join("CommonModules/CorpusModule.xml");
    write(&path, &descriptor);
    let module = fixture
        .source
        .join("CommonModules/CorpusModule/Ext/Module.bsl");
    let source = "#Если Сервер Тогда\nПроцедура Проверка() Экспорт\nКонецПроцедуры\n#КонецЕсли\n";
    write(&module, source);
    let service = fixture.extension_view_service();
    let view = service.view(ViewRequest::new("main:CommonModule.CorpusModule").unwrap());
    assert!(view.ok, "view must succeed");
    let data = view.data.unwrap();
    let props = data["props"]["commonModule"].as_object().unwrap();
    assert_eq!(props.len(), 8);
    assert!(props.values().all(serde_json::Value::is_null));
    for at in [
        "main:CommonModule.CorpusModule.Method",
        "main:CommonModule.CorpusModule.Method.Проверка",
        "main:CommonModule.CorpusModule.Method.Проверка.Body",
        "main:CommonModule.CorpusModule.Body",
    ] {
        let view = service.view(ViewRequest::new(at).unwrap());
        assert!(view.ok, "{at}: view must succeed");
    }
    let method = service
        .view(ViewRequest::new("main:CommonModule.CorpusModule.Method.Проверка").unwrap())
        .data
        .unwrap();
    assert_eq!(
        method["props"]["compile"]["contexts"],
        serde_json::Value::Null
    );
    assert_eq!(method["props"]["compile"]["guard"], "Server");
    assert_eq!(method["props"]["export"], true);
    let compilation = service
        .view(ViewRequest::new("main:CommonModule.CorpusModule.Compilation").unwrap())
        .data
        .unwrap();
    assert_eq!(compilation["items"][0]["contexts"], serde_json::Value::Null);
    assert_eq!(fs::read_to_string(&path).unwrap(), descriptor);
    assert_eq!(fs::read_to_string(&module).unwrap(), source);

    // A synthetic unknown descriptive property cannot prevent BSL exploration.
    write(
        &path,
        &descriptor.replace(
            "</Properties>",
            "<FutureDescription>Observed</FutureDescription></Properties>",
        ),
    );
    assert!(
        fixture
            .extension_view_service()
            .view(ViewRequest::new("main:CommonModule.CorpusModule.Method.Проверка").unwrap())
            .ok
    );

    // Synthetic damage must not turn into permission to omit module properties.
    for (name, valid) in [
        (
            "ExtendedConfigurationObject",
            "ac847dc9-e222-45cf-af4a-6fa863c919a8",
        ),
        ("ObjectBelonging", "Adopted"),
    ] {
        let original = format!("<{name}>{valid}</{name}>");
        assert!(descriptor.contains(&original));
        for replacement in [
            format!("{original}<{name}>broken</{name}>"),
            format!("<{name}>broken</{name}>{original}"),
            format!("<foreign:{name} xmlns:foreign=\"urn:foreign\">{valid}</foreign:{name}>"),
            format!("<{name} unexpected=\"true\">{valid}</{name}>"),
            format!("<{name}>{valid}<Nested/></{name}>"),
            format!("<{name}>broken</{name}>"),
        ] {
            let damaged = descriptor.replace(&original, &replacement);
            write(&path, &damaged);
            let service = fixture.extension_view_service();
            for at in [
                "main:CommonModule.CorpusModule",
                "main:CommonModule.CorpusModule.Method",
                "main:CommonModule.CorpusModule.Method.Проверка",
            ] {
                let result = service.view(ViewRequest::new(at).unwrap());
                assert!(
                    !result.ok,
                    "{replacement}: {at}: damaged identity must be refused"
                );
                assert_eq!(result.diagnostics[0]["detailCode"], "source_unreadable");
                assert!(result.diagnostics[0]["message"]
                    .as_str()
                    .unwrap()
                    .contains(name));
            }
            assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
            assert_eq!(fs::read_to_string(&module).unwrap(), source);
        }
    }
}

/// rule: INV.SOURCE.MODULE-COMMON-PROPERTIES
#[test]
fn common_module_absence_policy_keeps_ordinary_properties_required() {
    let ordinary = fixture_text("platform_8_3_27/incomplete_borrowed/parent-common-module.xml");
    let borrowed = fixture_text("platform_8_3_27/incomplete_borrowed/extension-common-module.xml");
    for name in [
        "Global",
        "ClientManagedApplication",
        "Server",
        "ExternalConnection",
        "ClientOrdinaryApplication",
        "ServerCall",
        "Privileged",
        "ReturnValuesReuse",
    ] {
        let doc = roxmltree::Document::parse(ordinary.trim_start_matches('\u{feff}')).unwrap();
        let node = doc
            .descendants()
            .find(|node| node.has_tag_name(name))
            .unwrap();
        let omitted = ordinary.replace(&doc.input_text()[node.range()], "");
        assert!(
            super::super::common_module_properties(
                omitted.as_bytes(),
                SourceSetKind::Configuration
            )
            .is_err(),
            "{name}"
        );
        let raw = if name == "ReturnValuesReuse" {
            "DuringSession"
        } else {
            "false"
        };
        let explicit = borrowed.replace(
            "</Properties>",
            &format!("<{name}>{raw}</{name}></Properties>"),
        );
        let observed = serde_json::to_value(
            super::super::common_module_properties(explicit.as_bytes(), SourceSetKind::Extension)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            observed
                .as_object()
                .unwrap()
                .values()
                .filter(|value| !value.is_null())
                .count(),
            1
        );
        let invalid = borrowed.replace(
            "</Properties>",
            &format!("<{name}>broken</{name}></Properties>"),
        );
        assert!(
            super::super::common_module_properties(invalid.as_bytes(), SourceSetKind::Extension)
                .is_err(),
            "{name}"
        );
    }
}
