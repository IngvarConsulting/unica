use unica_bindings::projection::*;
use unica_bindings::test_support::{base_contexts, evaluate_guard, project_declarative_binding};

#[cfg(test)]
mod tests {
    use super::{
        evaluate_guard, project_declarative_binding,
        project_form_events as project_form_events_with_context,
        project_form_owner_events as project_form_owner_events_with_context, project_module,
        DeclarativeBinding, FormBindingOwner, FormEventBindingInput, FormEventOwnerInput,
        FormMethodFact, ModuleProjectionRequest, PlatformEventWriteCapability,
    };
    use unica_bsl::projection::{EventState, ExtensionKind, InterfaceKind, MethodArea};
    use unica_metadata::event::BindingFact;

    use serde::Deserialize;
    use serde_json::json;
    use unica_address::address::{NodeKind, QualifiedAddress};
    use unica_bsl::profile::PlatformProfile;
    use unica_bsl_parser::parse as parse_bsl_syntax;
    use unica_forms::events::{FormDefinitionKind, FormElementKind};
    use unica_forms_designer::event_context::context_from_root;

    /// Declared parameters of a test handler, all required. The slice lives
    /// for the test process, like the handler facts that borrow it.
    fn params(names: &[&str]) -> &'static [unica_bsl::projection::MethodParameter] {
        names
            .iter()
            .map(|name| unica_bsl::projection::MethodParameter {
                name: (*name).to_string(),
                optional: false,
            })
            .collect::<Vec<_>>()
            .leak()
    }

    const COMPLEX: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/complex.bsl"
    ));
    const EXTENSION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/extension.bsl"
    ));
    const SYNTAX_BOUNDARIES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/syntax-boundaries.bsl"
    ));
    const NESTED_REGION_ADDRESSES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/nested-region-addresses.bsl"
    ));
    const DECLARATIVE_HTTP: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/declarative-http.bsl"
    ));
    const DECLARATIVE_SOAP: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/declarative-soap.bsl"
    ));
    const DECLARATIVE_INTEGRATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/v013/modules/declarative-integration.bsl"
    ));

    #[derive(Debug, Deserialize)]
    struct ModuleFixture {
        profile: String,
        cases: Vec<ModuleCase>,
    }

    #[derive(Debug, Deserialize)]
    struct ModuleCase {
        case: String,
        at: String,
        role: String,
        source: bool,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FormFixture {
        bindings: Vec<FormBindingCase>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FormBindingCase {
        case: String,
        owner: String,
        owner_at: String,
        event: String,
        handler: String,
        call_type: Option<String>,
        element_kind: Option<String>,
    }

    #[derive(Debug, Deserialize)]
    struct DeclarativeFixture {
        handlers: Vec<DeclarativeCase>,
    }

    #[derive(Debug, Deserialize)]
    struct DeclarativeCase {
        owner: String,
        at: String,
        event: String,
        handler: String,
    }

    fn module_fixture() -> ModuleFixture {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/v013/modules/module-cases.json"
        )))
        .unwrap()
    }

    fn form_fixture() -> FormFixture {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/v013/modules/form-bindings.json"
        )))
        .unwrap()
    }

    fn declarative_fixture() -> DeclarativeFixture {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/fixtures/v013/modules/declarative-handlers.json"
        )))
        .unwrap()
    }

    fn declarative_source(owner: &str) -> &'static str {
        match owner {
            "httpMethod" => DECLARATIVE_HTTP,
            "webServiceOperation" => DECLARATIVE_SOAP,
            "integrationServiceChannel" => DECLARATIVE_INTEGRATION,
            other => panic!("unknown declarative fixture owner `{other}`"),
        }
    }

    fn form_context(xml: &str, owner_kind: NodeKind) -> unica_forms::events::FormEventContext {
        let document = roxmltree::Document::parse(xml).unwrap();
        let mut context = context_from_root(document.root_element());
        context.metadata_owner = Some(owner_kind);
        context
    }

    fn default_form_context() -> unica_forms::events::FormEventContext {
        form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
            NodeKind::CommonForm,
        )
    }

    fn project_form_events(
        bindings: &[FormEventBindingInput],
        methods: &[FormMethodFact<'_>],
    ) -> Vec<unica_bsl::projection::EventProjection> {
        project_form_events_with_context(&default_form_context(), bindings, methods)
    }

    fn project_form_owner_events(
        owners: &[FormEventOwnerInput],
        bindings: &[FormEventBindingInput],
        methods: &[FormMethodFact<'_>],
    ) -> Vec<unica_bsl::projection::EventProjection> {
        project_form_owner_events_with_context(&default_form_context(), owners, bindings, methods)
    }

    fn project(at: &str, source: Option<&str>) -> unica_bsl::projection::ModuleProjectionSet {
        let at = QualifiedAddress::parse(at).unwrap();
        let profile = PlatformProfile::v8_3_27();
        let capability = profile.module_capability(&at).unwrap();
        let common_module = (capability.role() == unica_address::module_routes::ModuleRole::Common)
            .then(|| unica_bsl::projection::CommonModuleProperties {
                global: Some(false),
                client_managed_application: Some(true),
                server: Some(true),
                external_connection: Some(true),
                client_ordinary_application: Some(false),
                server_call: Some(false),
                privileged: Some(false),
                return_values_reuse: Some("DuringRequest".to_string()),
            });
        project_module(ModuleProjectionRequest {
            at: &at,
            capability,
            title: format!("{} module", capability.role().as_str()),
            rev: "fixture-rev",
            source,
            common_module,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap()
    }

    #[test]
    fn every_approved_module_role_projects_without_a_parallel_role_registry() {
        let fixture = module_fixture();
        assert_eq!(fixture.profile, "8.3.27");
        let actual_cases = fixture
            .cases
            .iter()
            .map(|case| (case.case.as_str(), case.at.as_str(), case.role.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            actual_cases,
            vec![
                ("object", "main:Document.Заказ.Module.Object", "Object"),
                ("manager", "main:Document.Заказ.Module.Manager", "Manager"),
                (
                    "record-set",
                    "main:InformationRegister.Цены.Module.RecordSet",
                    "RecordSet"
                ),
                (
                    "value-manager",
                    "main:Constant.ОсновнаяВалюта.Module.ValueManager",
                    "ValueManager"
                ),
                ("common", "main:CommonModule.ЗаказыСервер", "Common"),
                (
                    "form",
                    "main:Document.Заказ.Form.ФормаДокумента.Module.Form",
                    "Form"
                ),
                (
                    "command",
                    "main:Document.Заказ.Command.ПровестиИЗакрыть.Module.Command",
                    "Command"
                ),
                (
                    "managed-application",
                    "main:Module.ManagedApplication",
                    "ManagedApplication"
                ),
                (
                    "ordinary-application",
                    "main:Module.OrdinaryApplication",
                    "OrdinaryApplication"
                ),
                (
                    "external-connection",
                    "main:Module.ExternalConnection",
                    "ExternalConnection"
                ),
                ("session", "main:Module.Session", "Session"),
                (
                    "http",
                    "main:HTTPService.API.Module.HTTPService",
                    "HTTPService"
                ),
                (
                    "soap",
                    "main:WebService.Обмен.Module.WebService",
                    "WebService"
                ),
                (
                    "integration-service",
                    "main:IntegrationService.Шина.Module.IntegrationService",
                    "IntegrationService"
                ),
                ("bot", "main:Bot.Помощник.Module.Bot", "Bot"),
                (
                    "websocket-client",
                    "main:WebSocketClient.Телефония.Module.WebSocketClient",
                    "WebSocketClient"
                ),
                (
                    "epf-object",
                    "epf:ExternalDataProcessor.Импорт.Module.Object",
                    "Object"
                ),
                (
                    "epf-form",
                    "epf:ExternalDataProcessor.Импорт.Form.Основная.Module.Form",
                    "Form"
                ),
                (
                    "epf-command",
                    "epf:ExternalDataProcessor.Импорт.Command.Выполнить.Module.Command",
                    "Command"
                ),
                (
                    "erf-object",
                    "erf:ExternalReport.Продажи.Module.Object",
                    "Object"
                ),
                (
                    "erf-form",
                    "erf:ExternalReport.Продажи.Form.Основная.Module.Form",
                    "Form"
                ),
                (
                    "erf-command",
                    "erf:ExternalReport.Продажи.Command.Сформировать.Module.Command",
                    "Command"
                ),
                (
                    "extension-object",
                    "sales:Document.Заказ.Module.Object",
                    "Object"
                ),
                (
                    "valid-missing-file",
                    "main:Document.Пустой.Module.Object",
                    "Object"
                ),
            ]
        );
        for case in fixture.cases {
            let projection = project(&case.at, case.source.then_some(""));
            assert_eq!(projection.summary().props.role, case.role, "{}", case.case);
            assert_ne!(projection.summary().props.role, "Service", "{}", case.case);
            assert_eq!(
                projection.summary().branches[3].count,
                projection.events().len(),
                "{}",
                case.case
            );
            for (branch, actual) in projection.summary().branches.iter().zip([
                projection.methods().len(),
                projection.regions().len(),
                projection.interfaces().len(),
                projection.events().len(),
                projection.compilation().len(),
                projection.body().len(),
            ]) {
                assert_eq!(branch.count, actual, "{} {:?}", case.case, branch.kind);
            }
        }
    }

    #[test]
    fn multiline_signature_and_real_empty_body_boundaries_come_from_the_projector() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(SYNTAX_BOUNDARIES));
        let multiline = projection
            .methods()
            .iter()
            .find(|method| method.name == "Многострочная")
            .unwrap();
        assert_eq!(
            multiline.signature,
            "Процедура Многострочная(\n    Первый,\n    Знач Второй = Неопределено\n) Экспорт"
        );
        assert_eq!(
            multiline.doc.as_deref(),
            Some("Exact multiline declaration.\nThe body starts after the complete signature.")
        );
        assert_eq!(multiline.text_from_line, 1);
        assert_eq!(
            projection.method_text(multiline),
            SYNTAX_BOUNDARIES
                .split_inclusive('\n')
                .take(8)
                .collect::<String>()
        );

        let empty = projection
            .methods()
            .iter()
            .find(|method| method.name == "Пустая")
            .unwrap();
        assert_eq!(
            projection.method_text(empty),
            "Процедура Пустая(\n)\nКонецПроцедуры\n"
        );
        let serialized = serde_json::to_value(empty).unwrap();
        assert_eq!(serialized["branches"][1]["count"], 3);
    }

    #[test]
    fn method_compilation_count_and_nested_guards_match_actual_ranges() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(SYNTAX_BOUNDARIES));
        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == "Контекстная")
            .unwrap();
        let serialized = serde_json::to_value(method).unwrap();
        assert_eq!(serialized["branches"][0]["count"], 3);
        assert_eq!(method.compile.guard.as_deref(), Some("Client"));
        assert!(method.compile.conditional_body);
        let ranges = projection
            .compilation()
            .iter()
            .filter(|range| range.from_line >= 15 && range.to_line <= 22)
            .map(|range| (range.from_line, range.to_line, range.guard.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            ranges,
            vec![
                (15, 21, "Client"),
                (17, 17, "Client And WebClient"),
                (19, 19, "Client And Not (WebClient)"),
            ]
        );
    }

    #[test]
    fn crlf_method_text_keeps_the_source_line_endings() {
        let source = SYNTAX_BOUNDARIES.replace('\n', "\r\n");
        let projection = project("main:CommonModule.ЗаказыСервер", Some(&source));
        let multiline = projection
            .methods()
            .iter()
            .find(|method| method.name == "Многострочная")
            .unwrap();
        assert!(
            multiline.signature.contains("\r\n"),
            "source-proven signatures retain the source EOL profile"
        );
        assert_eq!(projection.source_text(), source);
        assert_eq!(
            projection.method_text(multiline),
            source.split_inclusive('\n').take(8).collect::<String>()
        );
    }

    #[test]
    fn form_catalog_is_emitted_once_for_two_bindings_on_one_owner() {
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Тест.Item.Поле.Event.OnChange",
                "OnChange",
                "ПолеПриИзменении",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Тест.Item.Поле.Event.StartChoice",
                "StartChoice",
                "ПолеНачалоВыбора",
                None,
            ),
        ];
        let methods = [
            FormMethodFact::new(
                "ПолеПриИзменении",
                unica_bsl::projection::MethodKind::Procedure,
                params(&[]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
            FormMethodFact::new(
                "ПолеНачалоВыбора",
                unica_bsl::projection::MethodKind::Procedure,
                params(&["ДанныеВыбора", "ВыборДобавлением", "СтандартнаяОбработка"]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
        ];
        let events = project_form_events(&bindings, &methods);
        assert_eq!(events.len(), 16);
        let unique = events
            .iter()
            .map(|event| event.at.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), events.len());
        assert_eq!(
            events
                .iter()
                .filter(|event| event.state == EventState::Implemented)
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["OnChange", "StartChoice"]
        );
    }

    #[test]
    fn form_event_state_requires_the_method_kind_and_accepts_an_earlier_shorter_signature() {
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Поле.Event.OnChange",
            "OnChange",
            "ПолеПриИзменении",
            None,
        );
        let contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let procedure = [FormMethodFact::new(
            "ПолеПриИзменении",
            unica_bsl::projection::MethodKind::Procedure,
            params(&[]),
            contexts.clone(),
        )
        .with_directive(Some("&НаКлиенте"))];
        assert_eq!(
            project_form_events(std::slice::from_ref(&binding), &procedure)
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Implemented
        );
        let function = [FormMethodFact::new(
            "ПолеПриИзменении",
            unica_bsl::projection::MethodKind::Function,
            params(&[]),
            contexts,
        )];
        assert_eq!(
            project_form_events(&[binding], &function)
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn method_parameters_keep_names_without_by_value_and_mark_defaults() {
        let module = project(
            "main:Catalog.Products.Form.Main.Module.Form",
            Some(
                "&НаКлиенте\nПроцедура Поле(Знач Элемент, Режим = Неопределено)\nКонецПроцедуры\n",
            ),
        );
        let parameters = &module.methods()[0].parameters;
        assert_eq!(
            parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.optional))
                .collect::<Vec<_>>(),
            [("Элемент", false), ("Режим", true)]
        );
    }

    #[test]
    fn form_handler_parameters_are_compared_with_the_item_and_the_form_check_rule() {
        let client = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let input = FormBindingOwner::Element(FormElementKind::InputField);
        let item_event = |event: &str, handler: &str, parameters: &[&str]| {
            let at = format!("main:CommonForm.Тест.Item.Поле.Event.{event}");
            let binding = FormEventBindingInput::property(input, &at, event, handler, None);
            let methods = [FormMethodFact::new(
                handler,
                unica_bsl::projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(&[binding], &methods)
                .into_iter()
                .find(|projected| projected.at == at)
                .unwrap()
        };
        // The current signature with the item, a renamed item and an
        // earlier shorter signature are all handlers the platform calls.
        for parameters in [&["Элемент"][..], &["Поле"][..], &[][..]] {
            assert_eq!(
                item_event("OnChange", "ПолеПриИзменении", parameters).state,
                EventState::Implemented,
                "{parameters:?}"
            );
        }
        let on_change = item_event("OnChange", "ПолеПриИзменении", &["Элемент"]);
        assert_eq!(on_change.signature, "Процедура ПриИзменении(Элемент)");
        assert_eq!(
            item_event(
                "StartChoice",
                "ПолеНачалоВыбора",
                &[
                    "Элемент",
                    "ДанныеВыбора",
                    "ВыборДобавлением",
                    "СтандартнаяОбработка"
                ]
            )
            .state,
            EventState::Implemented
        );
        // The signature of another event of the item and more required
        // parameters than the platform passes are not this event's handler.
        for parameters in [
            &["Элемент", "Текст", "СтандартнаяОбработка"][..],
            &["Элемент", "Лишний"][..],
        ] {
            assert_eq!(
                item_event("OnChange", "ПолеПриИзменении", parameters).state,
                EventState::Invalid,
                "{parameters:?}"
            );
        }
        // A form event handler does not receive the item.
        let open_at = "main:CommonForm.Тест.Event.OnOpen";
        let open = FormEventBindingInput::property(
            FormBindingOwner::Form,
            open_at,
            "OnOpen",
            "ПриОткрытии",
            None,
        );
        let open_event = |parameters: &[&str]| {
            let methods = [FormMethodFact::new(
                "ПриОткрытии",
                unica_bsl::projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(std::slice::from_ref(&open), &methods)
                .into_iter()
                .find(|projected| projected.at == open_at)
                .unwrap()
        };
        let implemented = open_event(&["Отказ"]);
        assert_eq!(implemented.state, EventState::Implemented);
        assert_eq!(implemented.signature, "Процедура ПриОткрытии(Отказ)");
        assert_eq!(open_event(&["Элемент", "Отказ"]).state, EventState::Invalid);
        // A command action follows the same rule with its own `(Команда)`.
        let action_at = "main:CommonForm.Тест.Command.Обновить.Event.Execute";
        let action = FormEventBindingInput::property(
            FormBindingOwner::Command,
            action_at,
            "Execute",
            "Обновить",
            None,
        );
        let action_state = |parameters: &[&str]| {
            let methods = [FormMethodFact::new(
                "Обновить",
                unica_bsl::projection::MethodKind::Procedure,
                params(parameters),
                client.clone(),
            )
            .with_directive(Some("&НаКлиенте"))];
            project_form_events(std::slice::from_ref(&action), &methods)
                .into_iter()
                .find(|projected| projected.at == action_at)
                .unwrap()
                .state
        };
        assert_eq!(action_state(&["Команда"]), EventState::Implemented);
        assert_eq!(action_state(&[]), EventState::Implemented);
        assert_eq!(action_state(&["Команда", "Лишний"]), EventState::Invalid);
    }

    #[test]
    fn duplicate_case_insensitive_event_methods_are_invalid_for_platform_and_property_sources() {
        let platform = project(
            "main:Catalog.Products.Module.Object",
            Some(
                "Процедура ПередЗаписью(Отказ)\nКонецПроцедуры\n\nПроцедура передзаписью(Отказ)\nКонецПроцедуры\n",
            ),
        );
        assert_eq!(
            platform
                .events()
                .iter()
                .find(|event| event.event_id == "BeforeWrite")
                .unwrap()
                .state,
            EventState::Invalid
        );

        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Field",
        );
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Field.Event.OnChange",
            "OnChange",
            "FieldOnChange",
            None,
        );
        let contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        let methods = [
            FormMethodFact::new(
                "FieldOnChange",
                unica_bsl::projection::MethodKind::Procedure,
                params(&[]),
                contexts.clone(),
            )
            .with_directive(Some("&AtClient")),
            FormMethodFact::new(
                "fieldonchange",
                unica_bsl::projection::MethodKind::Procedure,
                params(&[]),
                contexts,
            )
            .with_directive(Some("&AtClient")),
        ];
        let property = project_form_owner_events(&[owner], &[binding], &methods);
        assert_eq!(
            property
                .iter()
                .find(|event| event.event_id == "OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn form_binding_retains_actual_element_and_nested_column_kinds() {
        let label = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::LabelField),
            "main:CommonForm.Тест.Item.Метка.Event.OnChange",
            "OnChange",
            "МеткаПриИзменении",
            None,
        );
        let events = project_form_events(&[label], &[]);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            ["Click", "OnChange", "URLProcessing"]
        );

        let column = FormEventBindingInput::property(
            FormBindingOwner::Column(FormElementKind::CheckBoxField),
            "main:CommonForm.Тест.Item.Таблица.Item.Пометка.Event.OnChange",
            "OnChange",
            "ПометкаПриИзменении",
            None,
        );
        let events = project_form_events(&[column], &[]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, "OnChange");
    }

    #[test]
    fn form_owner_without_bindings_still_projects_its_closed_available_catalog() {
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Element(FormElementKind::LabelField),
            "main:CommonForm.Тест.Item.Метка",
        );
        let events = project_form_owner_events(&[owner], &[], &[]);
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            ["Click", "OnChange", "URLProcessing"]
        );
        assert!(events
            .iter()
            .all(|event| event.state == EventState::Available));
        assert_eq!(
            events
                .iter()
                .map(|event| event.at.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            events.len()
        );
    }

    #[test]
    fn borrowed_available_form_event_skeleton_defaults_call_type_after() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");

        let event = project_form_owner_events_with_context(&context, &[owner], &[], &[])
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();

        assert_eq!(event.state, EventState::Available);
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {
                    "at": "main:CommonForm.Test.Event.OnOpen",
                    "callType": "After"
                }
            })
        );
    }

    #[test]
    fn base_form_only_available_event_is_not_advertised_as_directly_writable() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");

        let event = project_form_owner_events_with_context(&context, &[owner], &[], &[])
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();

        assert_eq!(event.state, EventState::Available);
        assert!(event.can.is_empty(), "{event:?}");
    }

    #[test]
    fn borrowed_missing_event_preserves_binding_and_offers_no_new_call_type() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Events/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("Before"),
        );

        let event = project_form_owner_events_with_context(
            &context,
            &[owner],
            std::slice::from_ref(&binding),
            &[],
        )
        .into_iter()
        .find(|event| event.event_id == "OnOpen")
        .unwrap();

        assert_eq!(event.state, EventState::Missing);
        assert_eq!(event.call_type.as_deref(), Some("Before"));
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {"at": "main:CommonForm.Test.Event.OnOpen"}
            })
        );
    }

    #[test]
    fn valid_missing_form_event_keeps_its_direct_implementation_capability() {
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Test.Item.Value.Event.OnChange",
            "OnChange",
            "ValueOnChange",
            None,
        );

        let event = project_form_events(std::slice::from_ref(&binding), &[])
            .into_iter()
            .find(|event| event.event_id == "OnChange")
            .unwrap();

        assert_eq!(event.state, EventState::Missing);
        assert_eq!(
            serde_json::to_value(&event.can[0]).unwrap(),
            json!({
                "op": "event.implement",
                "args": {"at": "main:CommonForm.Test.Item.Value.Event.OnChange"}
            })
        );
    }

    #[test]
    fn duplicate_property_bindings_are_invalid_and_never_advertise_implementation() {
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Test.Item.Value.Event.OnChange",
                "OnChange",
                "FirstHandler",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:CommonForm.Test.Item.Value.Event.OnChange",
                "OnChange",
                "SecondHandler",
                None,
            ),
        ];

        let event = project_form_events(&bindings, &[])
            .into_iter()
            .find(|event| event.event_id == "OnChange")
            .unwrap();

        assert_eq!(event.state, EventState::Invalid);
        assert!(event.can.is_empty());
        assert!(event.implementation_at.is_none());
        // Both bindings stay visible; none is chosen as the handler.
        assert_eq!(event.handler, "");
        assert_eq!(
            event
                .handlers
                .iter()
                .map(|binding| binding.handler.as_str())
                .collect::<Vec<_>>(),
            ["FirstHandler", "SecondHandler"]
        );
    }

    #[test]
    fn form_command_execute_uses_action_handler_and_form_signature() {
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh",
        );
        let available = project_form_owner_events(std::slice::from_ref(&owner), &[], &[]);
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].event_id, "Execute");
        assert_eq!(available[0].state, EventState::Available);
        assert_eq!(
            available[0].signature,
            "Процедура ОбработкаКоманды(Команда)"
        );
        assert_eq!(
            available[0].contexts,
            [
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ]
        );

        let action = FormEventBindingInput::property(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh.Event.Execute",
            "Execute",
            "RefreshAction",
            None,
        );
        let missing = project_form_owner_events(
            std::slice::from_ref(&owner),
            std::slice::from_ref(&action),
            &[],
        );
        assert_eq!(missing[0].state, EventState::Missing);
        assert_eq!(missing[0].handler, "RefreshAction");
        assert_eq!(missing[0].signature, "Процедура RefreshAction(Команда)");

        let method = [FormMethodFact::new(
            "RefreshAction",
            unica_bsl::projection::MethodKind::Procedure,
            params(&["Команда"]),
            vec![
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ],
        )
        .with_directive(Some("&НаКлиенте"))];
        let implemented = project_form_owner_events(
            std::slice::from_ref(&owner),
            std::slice::from_ref(&action),
            &method,
        );
        assert_eq!(implemented[0].state, EventState::Implemented);

        let duplicate = project_form_owner_events(
            std::slice::from_ref(&owner),
            &[action.clone(), action.clone()],
            &method,
        );
        assert_eq!(duplicate[0].state, EventState::Invalid);
        assert!(duplicate[0].can.is_empty());
        // The same Action twice is two bindings in the source: both stay.
        assert_eq!(duplicate[0].handlers.len(), 2);
        assert!(duplicate[0].handlers.iter().all(
            |binding| binding.handler == "RefreshAction" && binding.implementation_at.is_some()
        ));

        // Two Action elements, as an extension leaves them: the event names
        // both, with the implementation each one has, and not the catalog
        // handler name.
        let extension_action = FormEventBindingInput::property(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh.Event.Execute",
            "Execute",
            "Extension_RefreshActionAfter",
            None,
        );
        let conflict = project_form_owner_events(&[owner], &[action, extension_action], &method);
        assert_eq!(conflict[0].state, EventState::Invalid);
        assert!(conflict[0].can.is_empty());
        assert_eq!(conflict[0].handler, "");
        assert_eq!(conflict[0].signature, "");
        assert_eq!(conflict[0].handler_en, "");
        assert!(conflict[0].implementation_at.is_none());
        let handlers = &conflict[0].handlers;
        assert_eq!(handlers.len(), 2, "{handlers:?}");
        assert_eq!(handlers[0].handler, "RefreshAction");
        assert!(handlers[0].implementation_at.is_some());
        assert_eq!(handlers[1].handler, "Extension_RefreshActionAfter");
        assert!(handlers[1].implementation_at.is_none());
    }

    #[test]
    fn every_form_owner_requires_the_exact_versioned_directive() {
        let owner = FormEventOwnerInput::new(FormBindingOwner::Form, "main:CommonForm.Test");
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            None,
        );
        let client_contexts = vec![
            "thinClient",
            "webClient",
            "thickClientManaged",
            "mobileClient",
            "mobileAppClient",
        ];
        for directive in [None, Some("&НаСервере"), Some("&AtClientAtServer")] {
            let methods = [FormMethodFact::new(
                "FormOnOpen",
                unica_bsl::projection::MethodKind::Procedure,
                params(&["Cancel"]),
                client_contexts.clone(),
            )
            .with_directive(directive)];
            let event = project_form_owner_events(
                std::slice::from_ref(&owner),
                std::slice::from_ref(&binding),
                &methods,
            )
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
            assert_eq!(event.state, EventState::Invalid, "{directive:?}");
            assert!(event.can.is_empty());
            assert!(event.implementation_at.is_some());
        }
        for directive in ["&НаКлИеНтЕ", "  &aTcLiEnT\r\n"] {
            let methods = [FormMethodFact::new(
                "FormOnOpen",
                unica_bsl::projection::MethodKind::Procedure,
                params(&["Cancel"]),
                client_contexts.clone(),
            )
            .with_directive(Some(directive))];
            let event = project_form_owner_events(
                std::slice::from_ref(&owner),
                std::slice::from_ref(&binding),
                &methods,
            )
            .into_iter()
            .find(|event| event.event_id == "OnOpen")
            .unwrap();
            assert_eq!(event.state, EventState::Implemented, "{directive:?}");
        }

        let server_binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnCreateAtServer",
            "OnCreateAtServer",
            "OnCreateAtServer",
            None,
        );
        let server = [FormMethodFact::new(
            "OnCreateAtServer",
            unica_bsl::projection::MethodKind::Procedure,
            params(&["Cancel", "StandardProcessing"]),
            vec!["server"],
        )
        .with_directive(Some(" &AtServer "))];
        let event = project_form_owner_events(&[owner], &[server_binding], &server)
            .into_iter()
            .find(|event| event.event_id == "OnCreateAtServer")
            .unwrap();
        assert_eq!(event.state, EventState::Implemented);
    }

    #[test]
    fn metadata_command_platform_event_requires_the_client_directive() {
        let at = "main:Document.Заказ.Command.Провести.Module.Command";
        let no_directive = project(
            at,
            Some(
                "Процедура ОбработкаКоманды(ПараметрКоманды, ПараметрыВыполненияКоманды)\nКонецПроцедуры",
            ),
        );
        assert_eq!(
            no_directive
                .events()
                .iter()
                .find(|event| event.event_id == "CommandProcessing")
                .unwrap()
                .state,
            EventState::Invalid
        );
        let client = project(
            at,
            Some(
                "&AtClient\r\nProcedure CommandProcessing(CommandParameter, CommandExecuteParameters)\nEndProcedure",
            ),
        );
        assert_eq!(
            client
                .events()
                .iter()
                .find(|event| event.event_id == "CommandProcessing")
                .unwrap()
                .state,
            EventState::Implemented
        );
    }

    #[test]
    fn borrowed_form_command_with_multiple_call_type_actions_fails_closed() {
        let context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><Commands/><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        let owner = FormEventOwnerInput::new(
            FormBindingOwner::Command,
            "main:CommonForm.Test.Command.Refresh",
        );
        let bindings = [
            FormEventBindingInput::property(
                FormBindingOwner::Command,
                "main:CommonForm.Test.Command.Refresh.Event.Execute",
                "Execute",
                "BeforeRefresh",
                Some("Before"),
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Command,
                "main:CommonForm.Test.Command.Refresh.Event.Execute",
                "Execute",
                "AfterRefresh",
                Some("After"),
            ),
        ];

        let event = project_form_owner_events_with_context(&context, &[owner], &bindings, &[])
            .into_iter()
            .next()
            .unwrap();

        assert_eq!(event.event_id, "Execute");
        assert_eq!(event.state, EventState::Invalid);
        assert!(event.can.is_empty());
    }

    #[test]
    fn form_possible_events_are_exact_for_document_and_dynamic_list_sources_without_bsl() {
        let document_context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
                    xmlns:v8="http://v8.1c.ru/8.1/data/core">
                <Attributes><Attribute name="Object" id="1">
                    <Type><v8:Type>cfg:DocumentObject.Order</v8:Type></Type>
                    <MainAttribute>true</MainAttribute>
                </Attribute></Attributes>
            </Form>"#,
            NodeKind::Document,
        );
        assert_eq!(document_context.definition, FormDefinitionKind::Regular);
        let form =
            FormEventOwnerInput::new(FormBindingOwner::Form, "main:Document.Order.Form.Main");
        let document_events =
            project_form_owner_events_with_context(&document_context, &[form], &[], &[]);
        assert_eq!(
            document_events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            [
                "ActivationProcessing",
                "AddInDetachmentOnError",
                "AfterWrite",
                "AfterWriteAtServer",
                "BeforeClose",
                "BeforeLoadDataFromSettingsAtServer",
                "BeforeReopenFromOtherServer",
                "BeforeWrite",
                "BeforeWriteAtServer",
                "ChoiceProcessing",
                "CollaborationSystemUsersAutoComplete",
                "CollaborationSystemUsersChoiceFormGetProcessing",
                "ExternalEvent",
                "FillCheckProcessingAtServer",
                "NavigationProcessing",
                "NewWriteProcessing",
                "NotificationProcessing",
                "OnChangeDisplaySettings",
                "OnClose",
                "OnCreateAtServer",
                "OnLoadDataFromSettingsAtServer",
                "OnMainServerAvailabilityChange",
                "OnOpen",
                "OnPasteFromClipboard",
                "OnReadAtServer",
                "OnReopen",
                "OnReopenFromOtherServer",
                "OnSaveDataInSettingsAtServer",
                "OnWriteAtServer",
                "URLGetProcessing",
                "URLListGetProcessing",
                "URLProcessing",
                "ValueChoice",
            ]
        );
        assert!(document_events
            .iter()
            .all(|event| event.state == EventState::Available));

        let dynamic_context = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"
                    xmlns:v8="http://v8.1c.ru/8.1/data/core">
                <Attributes><Attribute name="List" id="1">
                    <Type><v8:Type>cfg:DynamicList</v8:Type></Type>
                    <MainAttribute>true</MainAttribute>
                </Attribute></Attributes>
            </Form>"#,
            NodeKind::Catalog,
        );
        let table = FormEventOwnerInput::new(
            FormBindingOwner::Table,
            "main:Catalog.Items.Form.List.Item.List",
        )
        .with_data_path("List");
        let table_events =
            project_form_owner_events_with_context(&dynamic_context, &[table], &[], &[]);
        assert_eq!(
            table_events
                .iter()
                .map(|event| event.event_id.as_str())
                .collect::<Vec<_>>(),
            [
                "AfterDeleteRow",
                "BeforeAddRow",
                "BeforeCollapse",
                "BeforeDeleteRow",
                "BeforeEditEnd",
                "BeforeExpand",
                "BeforeLoadUserSettingsAtServer",
                "BeforeRowChange",
                "ChoiceProcessing",
                "Drag",
                "DragCheck",
                "DragEnd",
                "DragStart",
                "NewWriteProcessing",
                "OnActivateCell",
                "OnActivateField",
                "OnActivateRow",
                "OnChange",
                "OnCurrentParentChange",
                "OnEditEnd",
                "OnGetDataAtServer",
                "OnLoadUserSettingsAtServer",
                "OnSaveUserSettingsAtServer",
                "OnStartEdit",
                "OnUpdateUserSettingSetAtServer",
                "RefreshRequestProcessing",
                "Selection",
                "URLGetProcessing",
                "URLListGetProcessing",
                "ValueChoice",
            ]
        );
    }

    #[test]
    fn form_event_implementation_requires_call_type_valid_for_the_form_definition() {
        let regular = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"/>"#,
            NodeKind::CommonForm,
        );
        let binding = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("After"),
        );
        let methods = [FormMethodFact::new(
            "FormOnOpen",
            unica_bsl::projection::MethodKind::Procedure,
            params(&["Cancel"]),
            vec![
                "thinClient",
                "webClient",
                "thickClientManaged",
                "mobileClient",
                "mobileAppClient",
            ],
        )
        .with_directive(Some("&AtClient"))];
        assert_eq!(
            project_form_events_with_context(&regular, std::slice::from_ref(&binding), &methods)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Invalid
        );

        let extension = form_context(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform"><BaseForm/></Form>"#,
            NodeKind::CommonForm,
        );
        assert_eq!(extension.definition, FormDefinitionKind::Extension);
        assert_eq!(
            project_form_events_with_context(&extension, std::slice::from_ref(&binding), &methods,)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Implemented
        );

        let unknown = FormEventBindingInput::property(
            FormBindingOwner::Form,
            "main:CommonForm.Test.Event.OnOpen",
            "OnOpen",
            "FormOnOpen",
            Some("Instead"),
        );
        assert_eq!(
            project_form_events_with_context(&extension, &[unknown], &methods)
                .iter()
                .find(|event| event.event_id == "OnOpen")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn equal_nested_region_names_are_resolved_by_full_logical_address() {
        let projection = project(
            "main:CommonModule.ЗаказыСервер",
            Some(NESTED_REGION_ADDRESSES),
        );
        let first = "main:CommonModule.ЗаказыСервер.Region.Первый.Region.Общая";
        let second = "main:CommonModule.ЗаказыСервер.Region.Второй.Region.Общая";
        assert_eq!(projection.region(first).unwrap().at.as_deref(), Some(first));
        assert_eq!(
            projection.region(second).unwrap().at.as_deref(),
            Some(second)
        );
    }

    #[test]
    fn declarative_service_projection_must_use_real_fixture_syntax() {
        let case = &declarative_fixture().handlers[0];
        let projection = project_declarative_binding(
            DeclarativeBinding {
                owner: &case.owner,
                at: &case.at,
                event: &case.event,
                handler: &case.handler,
            },
            declarative_source(&case.owner),
        )
        .unwrap();
        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == case.handler)
            .unwrap();
        assert_eq!(method.signature, "Процедура ОбработатьGET(Запрос, Ответ)");
        assert_eq!(
            projection.method_text(method),
            "Процедура ОбработатьGET(Запрос, Ответ)\n    Ответ.КодСостояния = 200;\nКонецПроцедуры\n"
        );
        assert!(declarative_source(&case.owner).contains(&method.signature));
    }

    #[test]
    fn valid_missing_physical_file_keeps_possible_events_but_no_source_projection() {
        let projection = project("main:Document.Пустой.Module.Object", None);
        assert!(projection.methods().is_empty());
        assert!(projection.regions().is_empty());
        assert!(projection.interfaces().is_empty());
        assert!(projection.compilation().is_empty());
        assert!(projection.body().is_empty());
        assert!(!projection.events().is_empty());
        assert!(projection.events().iter().all(|event| {
            event.state == EventState::Available
                && event
                    .can
                    .iter()
                    .any(|capability| capability.op() == "event.implement")
        }));
        let json = serde_json::to_string(projection.summary()).unwrap();
        for forbidden in [
            "sourceState",
            "fileExists",
            "\"set\"",
            "\"role\":\"Service\"",
        ] {
            assert!(!json.contains(forbidden), "{forbidden}: {json}");
        }
    }

    #[test]
    fn platform_event_state_requires_exact_kind_parameter_shape_and_effective_contexts() {
        let exact = project(
            "main:Document.Заказ.Module.Object",
            Some("Процедура ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецПроцедуры"),
        );
        let event = exact
            .events()
            .iter()
            .find(|event| event.event_id == "BeforeWrite")
            .unwrap();
        assert_eq!(event.state, EventState::Implemented);
        assert_eq!(
            event.contexts,
            [
                "server",
                "externalConnection",
                "thickClientOrdinary",
                "mobileAppServer",
                "mobileStandaloneServer",
            ]
        );

        for source in [
            "Процедура ПередЗаписью(РежимЗаписи, Отказ, РежимПроведения)\nКонецПроцедуры",
            "Функция ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)\nКонецФункции",
        ] {
            let projection = project("main:Document.Заказ.Module.Object", Some(source));
            assert_eq!(
                projection
                    .events()
                    .iter()
                    .find(|event| event.event_id == "BeforeWrite")
                    .unwrap()
                    .state,
                EventState::Invalid,
                "{source}"
            );
        }
    }

    /// #1381 (по PR #1364): без любого из четырёх флагов среды контексты
    /// методов и областей неизвестны, тело без фильтра доступно; все флаги
    /// `false` дают известный пустой список.
    #[test]
    fn unknown_execution_flags_do_not_become_empty_or_universal_contexts() {
        use unica_bsl::projection::CommonModuleProperties;
        let known = CommonModuleProperties {
            global: None,
            server_call: None,
            privileged: None,
            return_values_reuse: None,
            client_managed_application: Some(false),
            client_ordinary_application: Some(false),
            server: Some(true),
            external_connection: Some(false),
        };
        let at = QualifiedAddress::parse("ext:CommonModule.Probe").unwrap();
        let capability = PlatformProfile::v8_3_27().module_capability(&at).unwrap();
        for missing in 0..5 {
            let mut props = known.clone();
            match missing {
                0 => props.server = None,
                1 => props.client_managed_application = None,
                2 => props.client_ordinary_application = None,
                3 => props.external_connection = None,
                _ => {}
            }
            let projection = project_module(ModuleProjectionRequest {
                at: &at, capability, title: "Probe".into(), rev: "fixture", source: Some("#Если Сервер Тогда\nПроцедура Test() Экспорт\nСообщить(1);\nКонецПроцедуры\n#КонецЕсли"),
                common_module: Some(props), handles: &[], declarative_bindings: &[], extension_targets: &[], platform_event_write: PlatformEventWriteCapability::Unproved,
            }).unwrap();
            let method = &projection.methods()[0];
            assert_eq!(method.compile.contexts.is_none(), missing < 4);
            assert_eq!(projection.compilation()[0].contexts.is_none(), missing < 4);
            assert!(projection.method_text(method).contains("Сообщить(1);"));
        }
        let mut no_execution = known;
        no_execution.server = Some(false);
        assert_eq!(
            super::base_contexts(capability, Some(&no_execution)),
            Some(Vec::new())
        );
    }

    #[test]
    fn common_module_requires_all_normalized_flags() {
        let at = QualifiedAddress::parse("main:CommonModule.ЗаказыСервер").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let error = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Общий модуль".to_string(),
            rev: "fixture-rev",
            source: Some(""),
            common_module: None,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap_err();
        assert!(error.contains("common-module flags"), "{error}");
    }

    #[test]
    fn method_projection_uses_ast_and_omits_body_text() {
        let handles = [
            FormEventBindingInput::property(
                FormBindingOwner::Element(FormElementKind::InputField),
                "main:Document.Заказ.Form.ФормаДокумента.Item.Склад.Event.OnChange",
                "OnChange",
                "ОбщийОбработчик",
                None,
            ),
            FormEventBindingInput::property(
                FormBindingOwner::Column(FormElementKind::InputField),
                "main:Document.Заказ.Form.ФормаДокумента.Item.Товары.Item.Количество.Event.OnChange",
                "OnChange",
                "ОбщийОбработчик",
                Some("After"),
            ),
        ];
        let at =
            QualifiedAddress::parse("main:Document.Заказ.Form.ФормаДокумента.Module.Form").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let projection = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Модуль формы".to_string(),
            rev: "fixture-rev",
            source: Some(COMPLEX),
            common_module: None,
            handles: &handles,
            declarative_bindings: &[],
            extension_targets: &[],
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap();

        let method = projection
            .methods()
            .iter()
            .find(|method| method.name == "ОбщийОбработчик")
            .unwrap();
        assert_eq!(
            method.signature,
            "Процедура ОбщийОбработчик(Элемент, Отказ = Ложь) Экспорт"
        );
        assert_eq!(
            method.doc.as_deref(),
            Some("Пересчитывает сумму строки.\nНе загружает данные повторно.")
        );
        assert_eq!(method.method_kind.as_str(), "procedure");
        assert!(method.export);
        assert_eq!(method.compile.directive.as_deref(), Some("&НаКлиенте"));
        assert_eq!(method.compile.guard.as_deref(), Some("Client"));
        assert!(method
            .compile
            .contexts
            .as_ref()
            .unwrap()
            .contains(&"thinClient".to_string()));
        assert_eq!(method.handles.len(), 2);
        assert_eq!(method.handles[1].call_type.as_deref(), Some("After"));
        assert!(method.extension.is_none());
        let json = serde_json::to_string(method).unwrap();
        assert!(!json.contains("Сообщить"));
        assert!(!json.contains("source"));
    }

    #[test]
    fn method_text_spans_doc_comment_directive_declaration_and_end_without_preprocessor() {
        let source = concat!(
            "// Отделён пустой строкой: не входит.\n",
            "\n",
            "// Документирует метод.\n",
            "//\tВторая строка.\n",
            "&НаСервере\n",
            "Процедура Документированная() Экспорт\n",
            "    Сообщить(1);\n",
            "КонецПроцедуры\n",
            "\n",
            "// Комментарий над препроцессором не относится к методу.\n",
            "#Если Сервер Тогда\n",
            "Функция ВнутриУсловия()\n",
            "    Возврат 1;\n",
            "КонецФункции\n",
            "#КонецЕсли\n",
            "// Висящий комментарий.\n",
            "\n",
            "Процедура БезКомментария()\n",
            "КонецПроцедуры",
        );
        let projection = project("main:CommonModule.ЗаказыСервер", Some(source));
        let text = |name: &str| {
            let method = projection
                .methods()
                .iter()
                .find(|method| method.name == name)
                .unwrap();
            projection.method_text(method)
        };
        assert_eq!(
            text("Документированная"),
            "// Документирует метод.\n//\tВторая строка.\n&НаСервере\nПроцедура Документированная() Экспорт\n    Сообщить(1);\nКонецПроцедуры\n"
        );
        assert_eq!(
            text("ВнутриУсловия"),
            "Функция ВнутриУсловия()\n    Возврат 1;\nКонецФункции\n"
        );
        // Последняя строка файла без разделителя остаётся без него.
        assert_eq!(
            text("БезКомментария"),
            "Процедура БезКомментария()\nКонецПроцедуры"
        );
        assert_eq!(projection.source_text(), source);
        assert_eq!(
            projection
                .methods()
                .iter()
                .filter(|candidate| candidate.name == "Документированная")
                .count(),
            1
        );
    }

    #[test]
    fn nested_regions_interface_projections_and_ambiguity_are_exact() {
        let projection = project("main:CommonModule.ЗаказыСервер", Some(COMPLEX));
        let public = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Public)
            .unwrap();
        let library = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Library)
            .unwrap();
        let overrides = projection
            .interfaces()
            .iter()
            .find(|interface| interface.interface == InterfaceKind::Override)
            .unwrap();
        assert_eq!(public.methods.len(), 2);
        assert_eq!(library.methods.len(), 3);
        assert_eq!(overrides.methods.len(), 1);
        assert!(!library
            .methods
            .iter()
            .any(|method| method.ends_with("НеклассифицированныйЭкспорт")));
        let nested = projection
            .regions()
            .iter()
            .find(|region| region.name.as_deref() == Some("Внутренние"))
            .unwrap();
        assert!(nested
            .at
            .as_ref()
            .unwrap()
            .contains("ПрограммныйИнтерфейс.Region.Внутренние"));
        let duplicates = projection
            .regions()
            .iter()
            .filter(|region| region.name.as_deref() == Some("Повтор"))
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 2);
        assert!(duplicates
            .iter()
            .all(|region| !region.addressable && region.at.is_none()));
        assert!(projection.region("Повтор").unwrap_err().is_ambiguous());
    }

    #[test]
    fn method_area_is_the_nearest_normative_region_whatever_the_export() {
        let source = "#Область ПрограммныйИнтерфейс\n\
            #Область ДанныеВБазе\n\
            Функция Открытая() Экспорт\nКонецФункции\n\
            #КонецОбласти\n\
            #Область УстаревшиеПроцедурыИФункции\n\
            Процедура Старая() Экспорт\nКонецПроцедуры\n\
            #КонецОбласти\n\
            #КонецОбласти\n\
            #Область СлужебныйПрограммныйИнтерфейс\n\
            Процедура Служебная() Экспорт\nКонецПроцедуры\n\
            #Область ДляВызоваИзДругихПодсистем\n\
            Процедура Соседям() Экспорт\nКонецПроцедуры\n\
            #КонецОбласти\n\
            #КонецОбласти\n\
            #Область ПереопределяемыеПроцедурыИФункции\n\
            Процедура Переопределить()\nКонецПроцедуры\n\
            #КонецОбласти\n\
            #Область СлужебныеПроцедурыИФункции\n\
            Процедура Внутренняя() Экспорт\nКонецПроцедуры\n\
            #КонецОбласти\n\
            Процедура ВнеОбластей() Экспорт\nКонецПроцедуры\n\
            #Область ПрограммныйИнтерфейс\n\
            Процедура Дубль() Экспорт\nКонецПроцедуры\n\
            #КонецОбласти\n\
            Процедура Дубль()\nКонецПроцедуры\n";
        let projection = project("main:CommonModule.ОбщегоНазначения", Some(source));
        let area = |name: &str| {
            projection
                .methods()
                .iter()
                .find(|method| method.name == name)
                .unwrap_or_else(|| panic!("{name} is projected"))
                .area
        };
        // A non-normative region in between does not hide the normative one.
        assert_eq!(area("Открытая"), Some(MethodArea::Public));
        // The nearest normative region wins over the enclosing one.
        assert_eq!(area("Старая"), Some(MethodArea::Deprecated));
        assert_eq!(area("Служебная"), Some(MethodArea::Internal));
        assert_eq!(area("Соседям"), Some(MethodArea::CrossSubsystem));
        // Export does not change the area, in either direction.
        assert_eq!(area("Переопределить"), Some(MethodArea::Override));
        assert_eq!(area("Внутренняя"), Some(MethodArea::Private));
        assert_eq!(area("ВнеОбластей"), None);
        let serialized = serde_json::to_value(projection.methods()).unwrap();
        assert_eq!(serialized[0]["props"]["area"], json!("public"));
        assert_eq!(serialized[3]["props"]["area"], json!("crossSubsystem"));
        assert!(serialized[6]["props"].get("area").is_none());
        // Same-named methods share an address; each still takes the area of
        // its own lines.
        let duplicates = projection
            .methods()
            .iter()
            .filter(|method| method.name == "Дубль")
            .map(|method| method.area)
            .collect::<Vec<_>>();
        assert_eq!(duplicates, vec![Some(MethodArea::Public), None]);
    }

    #[test]
    fn extension_annotations_resolve_independently_from_compilation_directives() {
        let targets = [
            (
                "Проверить",
                "main:Document.Заказ.Module.Object.Method.Проверить",
            ),
            (
                "Записать",
                "main:Document.Заказ.Module.Object.Method.Записать",
            ),
            (
                "Рассчитать",
                "main:Document.Заказ.Module.Object.Method.Рассчитать",
            ),
        ];
        let at = QualifiedAddress::parse("sales:Document.Заказ.Module.Object").unwrap();
        let profile = PlatformProfile::v8_3_27();
        let projection = project_module(ModuleProjectionRequest {
            at: &at,
            capability: profile.module_capability(&at).unwrap(),
            title: "Расширение".to_string(),
            rev: "fixture-rev",
            source: Some(EXTENSION),
            common_module: None,
            handles: &[],
            declarative_bindings: &[],
            extension_targets: &targets,
            platform_event_write: PlatformEventWriteCapability::Proven,
        })
        .unwrap();
        let kinds = projection
            .methods()
            .iter()
            .map(|method| method.extension.as_ref().unwrap().kind)
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                ExtensionKind::Instead,
                ExtensionKind::Before,
                ExtensionKind::After,
                ExtensionKind::ChangeAndValidate,
            ]
        );
        let instead = &projection.methods()[0];
        assert_eq!(instead.compile.directive.as_deref(), Some("&НаСервере"));
        assert_eq!(
            instead.extension.as_ref().unwrap().directive,
            "&Вместо(\"Проверить\")"
        );
        assert_eq!(
            instead.extension.as_ref().unwrap().target_at.as_deref(),
            Some("main:Document.Заказ.Module.Object.Method.Проверить")
        );
    }

    #[test]
    fn epf_erf_and_extension_sources_project_end_to_end() {
        let epf = project(
            "epf:ExternalDataProcessor.Импорт.Module.Object",
            Some(
                "Процедура ОбработкаПроверкиЗаполнения(Отказ, ПроверяемыеРеквизиты)\nКонецПроцедуры",
            ),
        );
        assert_eq!(
            epf.events()
                .iter()
                .find(|event| event.event_id == "FillCheckProcessing")
                .unwrap()
                .state,
            EventState::Implemented
        );
        assert_eq!(
            epf.methods()[0].at,
            "epf:ExternalDataProcessor.Импорт.Module.Object.Method.ОбработкаПроверкиЗаполнения"
        );

        let erf = project(
            "erf:ExternalReport.Продажи.Module.Object",
            Some("Процедура ПриКомпоновкеРезультата()\nКонецПроцедуры"),
        );
        assert_eq!(
            erf.events()
                .iter()
                .find(|event| event.event_id == "OnComposeResult")
                .unwrap()
                .state,
            EventState::Implemented
        );
        assert_eq!(
            erf.methods()[0].at,
            "erf:ExternalReport.Продажи.Module.Object.Method.ПриКомпоновкеРезультата"
        );

        extension_annotations_resolve_independently_from_compilation_directives();
    }

    #[test]
    fn russian_mobile_application_client_guard_normalizes_and_evaluates() {
        let parsed = parse_bsl_syntax(
            "#Если МобильноеПриложениеКлиент Тогда\nСообщить(\"mobile\");\n#КонецЕсли",
        );
        let guard = parsed.conditional_branches()[0].guard.clone();

        assert_eq!(guard, "MobileAppClient");
        assert!(evaluate_guard(&guard, "mobileAppClient"));
        assert!(!evaluate_guard(&guard, "server"));
    }

    #[test]
    fn form_binding_owners_and_all_four_event_states_are_projected() {
        let fixture = form_fixture();
        let bindings = fixture
            .bindings
            .iter()
            .map(|case| {
                FormEventBindingInput::property(
                    FormBindingOwner::parse(
                        &case.owner,
                        case.element_kind
                            .as_deref()
                            .and_then(FormElementKind::from_dsl_key),
                    )
                    .unwrap(),
                    &format!("{}.Event.{}", case.owner_at, case.event),
                    &case.event,
                    &case.handler,
                    case.call_type.as_deref(),
                )
            })
            .collect::<Vec<_>>();
        let methods = [
            FormMethodFact::new(
                "ОбщийОбработчик",
                unica_bsl::projection::MethodKind::Procedure,
                params(&[]),
                vec![
                    "thinClient",
                    "webClient",
                    "thickClientManaged",
                    "mobileClient",
                    "mobileAppClient",
                    "server",
                ],
            )
            .with_directive(Some("&НаКлиенте")),
            FormMethodFact::new(
                "Неверный",
                unica_bsl::projection::MethodKind::Function,
                params(&["Элемент"]),
                vec!["server"],
            ),
        ];
        let events = project_form_events(&bindings, &methods);
        for case in fixture.bindings {
            let at = format!("{}.Event.{}", case.owner_at, case.event);
            let event = events.iter().find(|event| event.at == at).unwrap();
            assert_eq!(
                event.implementation_at.as_deref(),
                Some("main:Document.Заказ.Form.ФормаДокумента.Module.Form.Method.ОбщийОбработчик"),
                "{}",
                case.case
            );
            if case.case == "form" {
                assert_eq!(event.call_type, None);
            }
        }
        let states = events.iter().map(|event| event.state).collect::<Vec<_>>();
        assert!(states.contains(&EventState::Implemented));
        assert!(states.contains(&EventState::Available));

        let missing = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Поле.Event.OnChange",
            "OnChange",
            "Отсутствует",
            None,
        );
        let invalid = FormEventBindingInput::property(
            FormBindingOwner::Element(FormElementKind::InputField),
            "main:CommonForm.Тест.Item.Другое.Event.OnChange",
            "OnChange",
            "Неверный",
            None,
        );
        let broken = project_form_events(&[missing, invalid], &methods);
        assert_eq!(
            broken
                .iter()
                .find(|event| event.at == "main:CommonForm.Тест.Item.Поле.Event.OnChange")
                .unwrap()
                .state,
            EventState::Missing
        );
        assert_eq!(
            broken
                .iter()
                .find(|event| event.at == "main:CommonForm.Тест.Item.Другое.Event.OnChange")
                .unwrap()
                .state,
            EventState::Invalid
        );
    }

    #[test]
    fn declarative_service_handlers_remain_on_exact_owners_without_synthetic_events() {
        let fixture = declarative_fixture();
        for case in fixture.handlers {
            let binding = DeclarativeBinding {
                owner: &case.owner,
                at: &case.at,
                event: &case.event,
                handler: &case.handler,
            };
            let projection =
                project_declarative_binding(binding, declarative_source(&case.owner)).unwrap();
            assert!(projection.events().is_empty(), "{}", case.at);
            let method = projection
                .methods()
                .iter()
                .find(|method| method.name == case.handler)
                .unwrap();
            assert_eq!(method.handles.len(), 1);
            assert_eq!(method.handles[0].owner, case.owner);
            assert_eq!(method.handles[0].binding, BindingFact::Property);
        }
    }
}
