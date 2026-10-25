//! Original pure codec and semantic cross-owner contracts. No coder dependency.
use unica_address::address::QualifiedAddress;
use unica_bindings::events::{
    property_call_type, require_postcondition, EventImplementationOwner, EventImplementationShape,
};
use unica_bsl::event_text::{emit_event_module, RequiredEventDirective};
use unica_bsl::projection::{EventProjection, EventState, MethodKind};
use unica_forms::event_targets::{PropertyEventOwner, PropertyEventOwnerKind};
use unica_forms::events::FormDefinitionKind;
use unica_forms_designer::event_patch::patch_form_event;
use unica_metadata::event::{BindingFact, FormCallType};
use unica_metadata::event_input::parse_event_implement_args;
use unica_source_contract::proposal::ApplyPlanErrorKind;
struct PropertyEventFixture {
    _event_at: QualifiedAddress,
    owner_chain: Vec<PropertyEventOwner>,
}
fn shape(
    owner: EventImplementationOwner,
    handler: &str,
    signature: &str,
) -> EventImplementationShape {
    EventImplementationShape {
        projection: EventProjection {
            at: "main:CommonForm.Main.Event.OnOpen".to_string(),
            event_id: "OnOpen".to_string(),
            state: EventState::Available,
            signature: signature.to_string(),
            contexts: vec!["thinClient".to_string()],
            binding: BindingFact::Property,
            handler: handler.to_string(),
            handler_en: String::new(),
            implementation_at: None,
            call_type: None,
            handlers: Vec::new(),
            can: Vec::new(),
        },
        owner,
        method_kind: MethodKind::Procedure,
        handler: handler.to_string(),
        implementation_at: format!("main:CommonForm.Main.Module.Form.Method.{handler}"),
        signature: signature.to_string(),
        directive: RequiredEventDirective::Client,
        region: match owner {
            EventImplementationOwner::Form => "ОбработчикиСобытийФормы",
            _ => "ОбработчикиСобытий",
        },
    }
}

fn form_source() -> PropertyEventFixture {
    let form_at = QualifiedAddress::parse("main:CommonForm.Main").unwrap();
    PropertyEventFixture {
        _event_at: QualifiedAddress::parse("main:CommonForm.Main.Event.OnOpen").unwrap(),

        owner_chain: vec![PropertyEventOwner {
            kind: PropertyEventOwnerKind::Form,
            at: form_at,
        }],
    }
}

#[test]
fn absent_module_emission_has_exact_platform_bom_lf_and_terminal_newline() {
    let actual = {
        let method = &shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        emit_event_module(
            None,
            &method.handler,
            &method.signature,
            method.method_kind,
            method.directive,
            method.region,
        )
    }
    .unwrap();
    assert_eq!(
        actual,
        concat!(
            "\u{feff}#Область ОбработчикиСобытийФормы\n",
            "\n",
            "&НаКлиенте\n",
            "Процедура ПриОткрытии(Отказ)\n",
            "\n",
            "КонецПроцедуры\n",
            "\n",
            "#КонецОбласти\n"
        )
        .as_bytes()
    );
}

#[test]
fn module_emitter_preserves_profiles_untouched_bytes_and_exact_directives() {
    let mut client = shape(
        EventImplementationOwner::Form,
        "ПриОткрытии",
        "Процедура ПриОткрытии(Отказ)",
    );
    let crlf = concat!(
        "\u{feff}// keep\r\n",
        "#Область ОбработчикиСобытийФормы\r\n",
        "// inside\r\n",
        "#КонецОбласти\r\n",
        "// tail"
    );
    let actual = {
        let method = &client;
        emit_event_module(
            Some(crlf.as_bytes()),
            &method.handler,
            &method.signature,
            method.method_kind,
            method.directive,
            method.region,
        )
    }
    .unwrap();
    assert!(actual.starts_with("\u{feff}// keep\r\n".as_bytes()));
    let actual = String::from_utf8(actual).unwrap();
    assert!(actual.contains("&НаКлиенте\r\nПроцедура ПриОткрытии(Отказ)\r\n"));
    assert!(actual.ends_with("#КонецОбласти\r\n// tail"));

    client.directive = RequiredEventDirective::Server;
    client.handler = "ПриСозданииНаСервере".to_string();
    client.signature = "Процедура ПриСозданииНаСервере(Отказ, СтандартнаяОбработка)".to_string();
    let no_bom = b"// untouched";
    let server = {
        let method = &client;
        emit_event_module(
            Some(no_bom),
            &method.handler,
            &method.signature,
            method.method_kind,
            method.directive,
            method.region,
        )
    }
    .unwrap();
    assert!(!server.starts_with(b"\xef\xbb\xbf"));
    let server = String::from_utf8(server).unwrap();
    assert!(server.starts_with("// untouched\n\n#Область"));
    assert!(server.contains("&НаСервере\nПроцедура ПриСозданииНаСервере"));

    client.directive = RequiredEventDirective::None;
    client.method_kind = MethodKind::Function;
    client.handler = "Result".to_string();
    client.signature = "Функция Result()".to_string();
    let function = String::from_utf8(
        {
            let method = &client;
            emit_event_module(
                Some(b""),
                &method.handler,
                &method.signature,
                method.method_kind,
                method.directive,
                method.region,
            )
        }
        .unwrap(),
    )
    .unwrap();
    assert!(!function.contains("&На"));
    assert!(function.contains("Функция Result()\n\nКонецФункции\n"));
}

#[test]
fn module_emitter_fails_closed_on_source_region_and_unicode_method_ambiguity() {
    let shape = shape(
        EventImplementationOwner::Form,
        "ПриОткрытии",
        "Процедура ПриОткрытии(Отказ)",
    );
    for source in [
        "// lone\rcr",
        "Процедура Broken(\n",
        "#Область ОбработчикиСобытийФормы\n#Область Inner\n#КонецОбласти\n",
        "#Область ОбработчикиСобытийФормы\n#КонецОбласти\n#Область обработчикисобытийформы\n#КонецОбласти\n",
        "#Область Outer\n#Область ОбработчикиСобытийФормы\n#КонецОбласти\n#КонецОбласти\n",
    ] {
        let error = { let method = &shape; emit_event_module(Some(source.as_bytes()), &method.handler, &method.signature, method.method_kind, method.directive, method.region) }.unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource, "{source:?}");
    }
    for source in [
        "Процедура приоткрытии(Отказ)\nКонецПроцедуры\n",
        "Procedure ПРИОТКРЫТИИ(Cancel)\nEndProcedure\n",
    ] {
        let error = {
            let method = &shape;
            emit_event_module(
                Some(source.as_bytes()),
                &method.handler,
                &method.signature,
                method.method_kind,
                method.directive,
                method.region,
            )
        }
        .unwrap_err();
        assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{source:?}");
    }
}

#[test]
fn module_emitter_uses_the_terminal_site_in_mixed_sources_and_rejects_an_ambiguous_site() {
    let shape = shape(
        EventImplementationOwner::Form,
        "ПриОткрытии",
        "Процедура ПриОткрытии(Отказ)",
    );
    let unambiguous_terminal = "// preserved CRLF\r\n// preserved LF\n// terminal";
    let emitted = String::from_utf8(
        {
            let method = &shape;
            emit_event_module(
                Some(unambiguous_terminal.as_bytes()),
                &method.handler,
                &method.signature,
                method.method_kind,
                method.directive,
                method.region,
            )
        }
        .expect("the terminal insertion site has an unambiguous LF profile"),
    )
    .unwrap();
    assert!(emitted.starts_with(unambiguous_terminal));
    assert!(emitted.contains("// terminal\n\n#Область ОбработчикиСобытийФормы\n\n&НаКлиенте\n"));

    let ambiguous_region = concat!(
        "// preserved CRLF\r\n",
        "#Область ОбработчикиСобытийФормы\n",
        "// previous LF\n",
        "#КонецОбласти\r\n",
        "// tail CRLF\r\n"
    );
    let error = {
        let method = &shape;
        emit_event_module(
            Some(ambiguous_region.as_bytes()),
            &method.handler,
            &method.signature,
            method.method_kind,
            method.directive,
            method.region,
        )
    }
    .expect_err("different EOLs adjacent to the real region site are ambiguous");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
}

#[test]
fn direct_form_event_patch_changes_only_the_root_owner_range() {
    let before = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\r\n",
        "\t<ChildItems/>\r\n",
        "</Form>"
    );
    let actual = {
        let source = &form_source();
        let method = &shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        patch_form_event(
            before.as_bytes(),
            &source.owner_chain,
            &method.projection.event_id,
            &method.handler,
            None,
        )
    }
    .unwrap();
    let text = String::from_utf8(actual).unwrap();
    assert!(text.contains(
        "\r\n\t<Events>\r\n\t\t<Event name=\"OnOpen\">ПриОткрытии</Event>\r\n\t</Events>\r\n"
    ));
    assert!(text.ends_with("\t<ChildItems/>\r\n</Form>"));
}

#[test]
fn form_patcher_uses_the_actual_site_in_mixed_sources_and_rejects_an_ambiguous_site() {
    let unambiguous_site = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
        "\t<ChildItems/>\n",
        "</Form>\n"
    );
    let patched = String::from_utf8(
        {
            let source = &form_source();
            let method = &shape(
                EventImplementationOwner::Form,
                "ПриОткрытии",
                "Процедура ПриОткрытии(Отказ)",
            );
            patch_form_event(
                unambiguous_site.as_bytes(),
                &source.owner_chain,
                &method.projection.event_id,
                &method.handler,
                None,
            )
        }
        .expect("the actual insertion site is locally LF even though the owner start is not"),
    )
    .unwrap();
    assert!(patched.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<Form"));
    assert!(patched.contains(
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n\t<Events>\n\t\t<Event name=\"OnOpen\">ПриОткрытии</Event>\n\t</Events>\n"
    ));

    let ambiguous_site = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n",
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\r\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
        "\t<ChildItems/>\r\n",
        "</Form>\r\n"
    );
    let error = {
        let source = &form_source();
        let method = &shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        patch_form_event(
            ambiguous_site.as_bytes(),
            &source.owner_chain,
            &method.projection.event_id,
            &method.handler,
            None,
        )
    }
    .expect_err("different EOLs adjacent to the real insertion site are ambiguous");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
}

#[test]
fn form_patcher_uses_complete_item_chain_and_command_action_without_touching_siblings() {
    let xml = concat!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
        "\t<ChildItems>\n",
        "\t\t<Table name=\"First\" id=\"1\"><ChildItems><InputField name=\"Value\" id=\"2\"/></ChildItems></Table>\n",
        "\t\t<Table name=\"Second\" id=\"3\"><ChildItems><InputField name=\"Value\" id=\"4\"/></ChildItems></Table>\n",
        "\t</ChildItems>\n",
        "\t<Commands><Command name=\"Заполнить\"><Title>keep</Title></Command></Commands>\n",
        "</Form>\n"
    );
    let form_at = QualifiedAddress::parse("main:Catalog.Products.Form.Main").unwrap();
    let nested_source = PropertyEventFixture {
        _event_at: QualifiedAddress::parse(
            "main:Catalog.Products.Form.Main.Item.Second.Item.Value.Event.OnChange",
        )
        .unwrap(),

        owner_chain: vec![
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Form,
                at: form_at.clone(),
            },
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Table,
                at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Item.Second").unwrap(),
            },
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Column,
                at: QualifiedAddress::parse(
                    "main:Catalog.Products.Form.Main.Item.Second.Item.Value",
                )
                .unwrap(),
            },
        ],
    };
    let mut nested_shape = shape(
        EventImplementationOwner::Column,
        "ValueПриИзменении",
        "Процедура ValueПриИзменении(Элемент)",
    );
    nested_shape.projection.event_id = "OnChange".to_string();
    let patched = String::from_utf8(
        {
            let source = &nested_source;
            let method = &nested_shape;
            patch_form_event(
                xml.as_bytes(),
                &source.owner_chain,
                &method.projection.event_id,
                &method.handler,
                None,
            )
        }
        .unwrap(),
    )
    .unwrap();
    let first = patched.find("name=\"First\"").unwrap();
    let second = patched.find("name=\"Second\"").unwrap();
    let event = patched.find("ValueПриИзменении").unwrap();
    assert!(first < second && second < event);
    assert_eq!(patched.matches("ValueПриИзменении").count(), 1);
    assert!(patched.contains("<Title>keep</Title>"));

    let command_source = PropertyEventFixture {
        _event_at: QualifiedAddress::parse(
            "main:Catalog.Products.Form.Main.Command.Заполнить.Event.Execute",
        )
        .unwrap(),
        owner_chain: vec![
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Form,
                at: form_at.clone(),
            },
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Command,
                at: QualifiedAddress::parse("main:Catalog.Products.Form.Main.Command.Заполнить")
                    .unwrap(),
            },
        ],
    };
    let mut command_shape = shape(
        EventImplementationOwner::Command,
        "ЗаполнитьОбработкаКоманды",
        "Процедура ЗаполнитьОбработкаКоманды(Команда)",
    );
    command_shape.projection.event_id = "Execute".to_string();
    let command = String::from_utf8(
        {
            let source = &command_source;
            let method = &command_shape;
            patch_form_event(
                xml.as_bytes(),
                &source.owner_chain,
                &method.projection.event_id,
                &method.handler,
                Some(FormCallType::After),
            )
        }
        .unwrap(),
    )
    .unwrap();
    assert!(command.contains("<Action callType=\"After\">ЗаполнитьОбработкаКоманды</Action>"));
    assert!(!command.contains("<Event name=\"Execute\""));
    assert!(command.find("<Title>keep</Title>").unwrap() < command.find("<Action").unwrap());
}

#[test]
fn form_patcher_rejects_wrong_namespace_owner_lookalikes() {
    let xml = concat!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:foreign=\"urn:foreign\" version=\"2.20\">\n",
        "\t<AutoCommandBar name=\"Bar\" id=\"-1\"/>\n",
        "\t<ChildItems><foreign:InputField name=\"Field\" id=\"1\"/></ChildItems>\n",
        "</Form>\n"
    );
    let form_at = QualifiedAddress::parse("main:CommonForm.Main").unwrap();
    let source = PropertyEventFixture {
        _event_at: QualifiedAddress::parse("main:CommonForm.Main.Item.Field.Event.OnChange")
            .unwrap(),
        owner_chain: vec![
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Form,
                at: form_at.clone(),
            },
            PropertyEventOwner {
                kind: PropertyEventOwnerKind::Element,
                at: QualifiedAddress::parse("main:CommonForm.Main.Item.Field").unwrap(),
            },
        ],
    };
    let error = {
        let source = &source;
        let method = &shape(
            EventImplementationOwner::Element,
            "FieldПриИзменении",
            "Процедура FieldПриИзменении(Элемент)",
        );
        patch_form_event(
            xml.as_bytes(),
            &source.owner_chain,
            &method.projection.event_id,
            &method.handler,
            None,
        )
    }
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
}

#[test]
fn form_patcher_rejects_duplicate_existing_event_slots_even_off_target() {
    let xml = concat!(
        "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\">\n",
        "\t<Events>\n",
        "\t\t<Event name=\"OnClose\">First</Event>\n",
        "\t\t<Event name=\"OnClose\">Second</Event>\n",
        "\t</Events>\n",
        "</Form>\n"
    );
    let error = {
        let source = &form_source();
        let method = &shape(
            EventImplementationOwner::Form,
            "ПриОткрытии",
            "Процедура ПриОткрытии(Отказ)",
        );
        patch_form_event(
            xml.as_bytes(),
            &source.owner_chain,
            &method.projection.event_id,
            &method.handler,
            None,
        )
    }
    .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
}

#[test]
fn call_type_rules_and_argument_paths_are_closed_and_typed() {
    let available = shape(
        EventImplementationOwner::Form,
        "ПриОткрытии",
        "Процедура ПриОткрытии(Отказ)",
    );
    for call_type in [
        FormCallType::Before,
        FormCallType::After,
        FormCallType::Override,
    ] {
        assert_eq!(
            property_call_type(
                &available,
                &FormDefinitionKind::Extension,
                true,
                Some(call_type),
            )
            .unwrap(),
            Some(call_type)
        );
    }
    assert_eq!(
        property_call_type(&available, &FormDefinitionKind::Extension, true, None,)
            .unwrap_err()
            .kind(),
        ApplyPlanErrorKind::BadValue
    );
    assert_eq!(
        property_call_type(
            &available,
            &FormDefinitionKind::Regular,
            true,
            Some(FormCallType::After),
        )
        .unwrap_err()
        .kind(),
        ApplyPlanErrorKind::BadValue
    );
    assert_eq!(
        property_call_type(
            &available,
            &FormDefinitionKind::Extension,
            false,
            Some(FormCallType::After),
        )
        .unwrap_err()
        .kind(),
        ApplyPlanErrorKind::InvalidState
    );

    let bad = parse_event_implement_args(
        &serde_json::json!({"at": "main:Catalog.Products.Form.Main.Event.OnOpen", "extra": true}),
        "ops[4].args",
    )
    .unwrap_err();
    assert_eq!(bad.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(bad.path(), Some("ops[4].args.extra"));
    let bad = parse_event_implement_args(
        &serde_json::json!({"at": "main:Catalog.Products.Form.Main.Event.OnOpen", "callType": "Later"}),
        "ops[5].args",
    )
    .unwrap_err();
    assert_eq!(bad.path(), Some("ops[5].args.callType"));
}

#[test]
fn postprojection_rejects_a_wrong_handler_or_missing_implementation_identity() {
    let before = shape(
        EventImplementationOwner::Form,
        "ПриОткрытии",
        "Процедура ПриОткрытии(Отказ)",
    );
    let mut after = before.clone();
    after.projection.state = EventState::Implemented;
    after.projection.handler = "ЧужойОбработчик".to_string();
    after.projection.implementation_at = None;
    let error = require_postcondition(&before, &after, None).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::Postcondition);

    let mut wrong_identity = before.clone();
    wrong_identity.projection.state = EventState::Implemented;
    wrong_identity.projection.binding = BindingFact::Platform;
    wrong_identity.projection.implementation_at =
        Some("main:CommonForm.Main.Module.Form.Method.Other".to_string());
    let error = require_postcondition(&before, &wrong_identity, None).unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::Postcondition);
}
