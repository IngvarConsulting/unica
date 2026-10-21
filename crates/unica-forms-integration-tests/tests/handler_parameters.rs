//! Original lazy Forms handler oracles, using actual readonly source owner.
mod form_handler_parameter_tests {

    use serde_json::{json, Map};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use unica_execution_contract::adapter_outcome::AdapterOutcome;
    use unica_forms_designer::codec::{validate_form_image, HandlerSignatures};
    use unica_source_contract::workspace::WorkspaceContext;
    use unica_workspace::discovery::ProjectDiscoveryReader;

    static NONCE: AtomicU64 = AtomicU64::new(0);

    const CHOICE_LIST: &str =
        "Элемент, ВыбранноеЗначение, ДополнительныеДанные, ВыборДобавлением, СтандартнаяОбработка";

    /// A catalog item form with one input field, one bound table and form
    /// events; `module` is the form module text or `None` for no module.
    fn validate(
        events: &str,
        table_events: &str,
        form_events: &str,
        module: Option<&str>,
    ) -> AdapterOutcome {
        validate_with_base_form(events, table_events, form_events, module, "")
    }

    fn validate_with_base_form(
        events: &str,
        table_events: &str,
        form_events: &str,
        module: Option<&str>,
        base_form: &str,
    ) -> AdapterOutcome {
        validate_form_fixture(FormFixture {
            events,
            table_events,
            form_events,
            module,
            base_form,
            main_type: "cfg:CatalogObject.Items",
            table_path: "Объект.Строки",
        })
    }

    struct FormFixture<'a> {
        events: &'a str,
        table_events: &'a str,
        form_events: &'a str,
        module: Option<&'a str>,
        base_form: &'a str,
        main_type: &'a str,
        table_path: &'a str,
    }

    fn validate_form_fixture(fixture: FormFixture<'_>) -> AdapterOutcome {
        let FormFixture {
            events,
            table_events,
            form_events,
            module,
            base_form,
            main_type,
            table_path,
        } = fixture;
        let root = std::env::temp_dir().join(format!(
            "unica-form-handler-parameters-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::Relaxed)
        ));
        let ext = root.join("Catalogs/Items/Forms/Item/Ext");
        fs::create_dir_all(ext.join("Form")).unwrap();
        let form_path = ext.join("Form.xml");
        let xml = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" version=\"2.20\">\n\t<AutoCommandBar name=\"ФормаКоманднаяПанель\" id=\"-1\"/>\n\t<Events>{form_events}</Events>\n\t<ChildItems>\n\t\t<InputField name=\"Поле\" id=\"1\">\n\t\t\t<DataPath>Объект.Description</DataPath>\n\t\t\t<ContextMenu name=\"ПолеКонтекстноеМеню\" id=\"2\"/>\n\t\t\t<ExtendedTooltip name=\"ПолеРасширеннаяПодсказка\" id=\"3\"/>\n\t\t\t<Events>{events}</Events>\n\t\t</InputField>\n\t\t<Table name=\"Список\" id=\"4\">\n\t\t\t<DataPath>{table_path}</DataPath>\n\t\t\t<ContextMenu name=\"СписокКонтекстноеМеню\" id=\"5\"/>\n\t\t\t<AutoCommandBar name=\"СписокКоманднаяПанель\" id=\"6\"/>\n\t\t\t<ExtendedTooltip name=\"СписокРасширеннаяПодсказка\" id=\"7\"/>\n\t\t\t<SearchStringAddition name=\"СписокСтрокаПоиска\" id=\"8\"/>\n\t\t\t<ViewStatusAddition name=\"СписокСостояниеПросмотра\" id=\"9\"/>\n\t\t\t<SearchControlAddition name=\"СписокУправлениеПоиском\" id=\"10\"/>\n\t\t\t<Events>{table_events}</Events>\n\t\t</Table>\n\t</ChildItems>\n\t<Attributes>\n\t\t<Attribute name=\"Объект\" id=\"1\">\n\t\t\t<Type><v8:Type>{main_type}</v8:Type></Type>\n\t\t\t<MainAttribute>true</MainAttribute>\n\t\t</Attribute>\n\t</Attributes>\n{base_form}</Form>\n"
        );
        fs::write(&form_path, &xml).unwrap();
        if let Some(module) = module {
            fs::write(ext.join("Form/Module.bsl"), format!("\u{feff}{module}")).unwrap();
        }
        let context = WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let args = Map::from_iter([(
            "FormPath".to_string(),
            json!(form_path.display().to_string()),
        )]);
        let outcome = observe(&args, &context, &form_path, &xml, HandlerSignatures::Check);
        // `form.edit` keeps the verdict but never reads the module.
        let edit = observe(&args, &context, &form_path, &xml, HandlerSignatures::Skip);
        assert_eq!(edit.ok, outcome.ok);
        assert!(edit.warnings.is_empty());
        let _ = fs::remove_dir_all(&root);
        outcome
    }

    fn input_event(event: &str, handler: &str) -> String {
        format!("<Event name=\"{event}\">{handler}</Event>")
    }

    fn client_method(name: &str, parameters: &str) -> String {
        format!("&НаКлиенте\nПроцедура {name}({parameters})\nКонецПроцедуры\n")
    }

    const START_CHOICE_LIST: &str = "Элемент, ДанныеВыбора, ВыборДобавлением, СтандартнаяОбработка";

    #[test]
    fn legacy_shorter_signatures_give_no_finding() {
        for parameters in [
            "Элемент, ВыбранноеЗначение, СтандартнаяОбработка",
            "Элемент, ВыбранноеЗначение, ДополнительныеДанные, СтандартнаяОбработка",
            CHOICE_LIST,
            "Знач Элемент, ВыбранноеЗначение, ДополнительныеДанные, ВыборДобавлением, СтандартнаяОбработка, Лишний = Неопределено",
        ] {
            let outcome = validate(
                &input_event("ChoiceProcessing", "ПолеОбработкаВыбора"),
                "",
                "",
                Some(&client_method("ПолеОбработкаВыбора", parameters)),
            );
            assert!(outcome.ok);
            assert!(outcome.warnings.is_empty(), "unexpected finding for {parameters:?}");
        }
    }

    #[test]
    fn foreign_event_signature_is_a_warning_naming_both_events() {
        let outcome = validate(
            &input_event("AutoComplete", "поленачаловыбора"),
            "",
            "",
            Some(&client_method("ПолеНачалоВыбора", START_CHOICE_LIST)),
        );
        assert!(outcome.ok, "a foreign signature keeps the verdict");
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.warnings.len(), 1);
        let warning = &outcome.warnings[0];
        for expected in [
            "Event 'АвтоПодбор' of InputField element 'Поле'",
            "handler 'ПолеНачалоВыбора'",
            &format!("matches the signature of event 'НачалоВыбора' ({START_CHOICE_LIST})"),
            "not the bound event (Элемент, Текст, ДанныеВыбора, ПараметрыПолученияДанных, Ожидание, СтандартнаяОбработка)",
        ] {
            assert!(warning.contains(expected), "missing {expected:?}");
        }
        assert!(outcome
            .stdout
            .as_deref()
            .is_some_and(|stdout| stdout.contains("[WARN]  Event 'АвтоПодбор'")));
    }

    #[test]
    fn same_count_renames_pass_and_table_foreign_signature_is_found() {
        let renamed = validate(
            "",
            &input_event("ValueChoice", "СписокВыборЗначения"),
            "",
            Some(&client_method(
                "СписокВыборЗначения",
                "Элемент, ВыбранноеЗначение, СтандартнаяОбработка",
            )),
        );
        assert!(renamed.warnings.is_empty());
        let foreign = validate(
            "",
            &input_event("ValueChoice", "СписокВыбор"),
            "",
            Some(&client_method(
                "СписокВыбор",
                "Элемент, ВыбраннаяСтрока, Поле, СтандартнаяОбработка",
            )),
        );
        assert_eq!(foreign.warnings.len(), 1);
        assert!(foreign.warnings[0].contains("Table element 'Список'"));
        assert!(foreign.warnings[0].contains("event 'Выбор'"));
    }

    #[test]
    fn required_parameter_beyond_every_known_list_is_a_warning() {
        let form_event = input_event("URLProcessing", "ОбработкаНавигационнойСсылки");
        let item_style = validate(
            "",
            "",
            &form_event,
            Some(&client_method(
                "ОбработкаНавигационнойСсылки",
                "Элемент, НавигационнаяСсылкаФорматированнойСтроки, СтандартнаяОбработка",
            )),
        );
        assert_eq!(item_style.warnings.len(), 1);
        let warning = &item_style.warnings[0];
        assert!(warning.contains("Event 'ОбработкаНавигационнойСсылки' of form"));
        assert!(warning.contains("requires 3 parameter(s)"));
        assert!(warning.contains(
            "platform passes (НавигационнаяСсылкаФорматированнойСтроки, СтандартнаяОбработка)"
        ));
        let optional = validate(
            "",
            "",
            &form_event,
            Some(&client_method(
                "ОбработкаНавигационнойСсылки",
                "Ссылка, СтандартнаяОбработка, Режим = Неопределено",
            )),
        );
        assert!(optional.warnings.is_empty());
    }

    #[test]
    fn extension_forms_compare_only_bindings_without_call_type() {
        let method = client_method("ПолеНачалоВыбора", START_CHOICE_LIST);
        // A borrowed form wraps the base handler: a callType binding is not
        // compared, and a binding without callType is already an error there.
        let base_form = "\t<BaseForm version=\"2.20\"/>\n";
        let wrapped = validate_with_base_form(
            "<Event name=\"AutoComplete\" callType=\"After\">ПолеНачалоВыбора</Event>",
            "",
            "",
            Some(&method),
            base_form,
        );
        assert!(wrapped.ok, "the extension fixture must validate");
        assert!(wrapped.warnings.is_empty());
        let unwrapped = validate_with_base_form(
            "<Event name=\"AutoComplete\">ПолеНачалоВыбора</Event>",
            "",
            "",
            Some(&method),
            base_form,
        );
        assert!(!unwrapped.ok);
        assert!(unwrapped.warnings.is_empty());
        // A form of the extension's own has no BaseForm and is compared like
        // any configuration form.
        let own = validate(
            &input_event("AutoComplete", "ПолеНачалоВыбора"),
            "",
            "",
            Some(&method),
        );
        assert_eq!(own.warnings.len(), 1);
    }

    #[test]
    fn object_form_events_follow_the_owner_found_by_the_dump_path() {
        // The fixture lives under `Catalogs/Items/Forms/Item`, so the object
        // form's write events are known and a legacy document-style signature
        // has more required parameters than the platform passes.
        let outcome = validate(
            "",
            "",
            &input_event("BeforeWrite", "ПередЗаписью"),
            Some(&client_method(
                "ПередЗаписью",
                "Отказ, РежимЗаписи, РежимПроведения",
            )),
        );
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("Event 'ПередЗаписью' of form"));
        assert!(outcome.warnings[0].contains("platform passes (Отказ, ПараметрыЗаписи)"));
    }

    #[test]
    fn a_table_bound_to_the_dynamic_list_gets_its_extra_events() {
        let event = input_event(
            "OnUpdateUserSettingSetAtServer",
            "СписокПриОбновленииСоставаПользовательскихНастроекНаСервере",
        );
        let module = "&НаСервере\nПроцедура СписокПриОбновленииСоставаПользовательскихНастроекНаСервере(Элемент, СтандартнаяОбработка, Лишний)\nКонецПроцедуры\n";
        let outcome = validate_form_fixture(FormFixture {
            events: "",
            table_events: &event,
            form_events: "",
            module: Some(module),
            base_form: "",
            main_type: "cfg:DynamicList",
            table_path: "Объект",
        });
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("requires 3 parameter(s)"));
        assert!(outcome.warnings[0].contains("platform passes (Элемент, СтандартнаяОбработка)"));
    }

    #[test]
    fn by_value_names_and_async_handlers_are_compared_by_their_names() {
        // Without stripping `Знач` no name would match StartChoice.
        let by_value = validate(
            &input_event("AutoComplete", "ПолеНачалоВыбора"),
            "",
            "",
            Some(&client_method(
                "ПолеНачалоВыбора",
                "Знач Элемент, Знач ДанныеВыбора, Знач ВыборДобавлением, Знач СтандартнаяОбработка",
            )),
        );
        assert_eq!(by_value.warnings.len(), 1);
        assert!(by_value.warnings[0].contains("event 'НачалоВыбора'"));
        let asynchronous = validate(
            &input_event("AutoComplete", "ПолеНачалоВыбора"),
            "",
            "",
            Some(&format!(
                "&НаКлиенте\nАсинх Процедура ПолеНачалоВыбора({START_CHOICE_LIST})\nКонецПроцедуры\n"
            )),
        );
        assert_eq!(asynchronous.warnings.len(), 1);
        assert!(asynchronous.warnings[0].contains("handler 'ПолеНачалоВыбора'"));
    }

    #[test]
    fn missing_or_unreadable_module_and_absent_or_ambiguous_method_give_no_finding() {
        let event = input_event("AutoComplete", "ПолеНачалоВыбора");
        let foreign = client_method("ПолеНачалоВыбора", START_CHOICE_LIST);
        assert_eq!(validate(&event, "", "", Some(&foreign)).warnings.len(), 1);
        assert!(validate(&event, "", "", None).warnings.is_empty());
        assert!(
            validate(&event, "", "", Some("Процедура ПолеНачалоВыбора(\n"))
                .warnings
                .is_empty()
        );
        assert!(validate(
            &input_event("AutoComplete", "ДругойОбработчик"),
            "",
            "",
            Some(&foreign)
        )
        .warnings
        .is_empty());
        let duplicated = format!("{foreign}\n{foreign}");
        assert!(validate(&event, "", "", Some(&duplicated))
            .warnings
            .is_empty());
    }

    fn observe(
        args: &Map<String, serde_json::Value>,
        context: &WorkspaceContext,
        path: &Path,
        xml: &str,
        mode: HandlerSignatures,
    ) -> AdapterOutcome {
        let reader = ProjectDiscoveryReader::new(&context.cwd);
        let (ok, stdout, artifact, errors, warnings) =
            validate_form_image(args, &reader, path, xml, mode).unwrap();
        AdapterOutcome {
            ok,
            summary: String::new(),
            changes: Vec::new(),
            warnings,
            errors,
            artifacts: vec![artifact.display().to_string()],
            stdout: Some(stdout),
            stderr: Some(String::new()),
            command: None,
        }
    }
}
