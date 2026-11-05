use super::*;

const SUBSCRIPTION: &str = "main:EventSubscription.ПриЗаписи";
const HANDLER: &str = "CommonModule.РеактивныйСервер.ПриЗаписи";
const TARGET: &str = "main:CommonModule.РеактивныйСервер.Method.ПриЗаписи";

fn replace_handler(fixture: &RealReaderFixture, value: &str) {
    let path = fixture.source.join("EventSubscriptions/ПриЗаписи.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(&path, &xml.replace(HANDLER, value));
}

#[test]
fn subscription_relation_preserves_sources_and_adds_one_method_link() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("EventSubscriptions/ПриЗаписи.xml");
    let xml = fs::read_to_string(&path).unwrap().replace(
        "</Source>",
        "<v8:Type>cfg:DocumentObject.Order</v8:Type></Source>",
    );
    write(&path, &xml);
    let module = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    write(
        &module,
        "Процедура ПриЗаписи(Источник, Отказ) Экспорт\nКонецПроцедуры\n",
    );
    let before = fs::read(&path).unwrap();
    let bsl = fs::read(&module).unwrap();
    let service = fixture.view_service();
    let root = service.view(ViewRequest::new(SUBSCRIPTION).unwrap());
    assert!(root.ok, "{:?}", refusal_codes(&root));
    let data = root.data.unwrap();
    assert_eq!(data["props"]["Handler"], HANDLER);
    assert!(data["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at": format!("{SUBSCRIPTION}.Relation"), "count": 3})));
    let relation = service.view(ViewRequest::new(&format!("{SUBSCRIPTION}.Relation")).unwrap());
    assert!(relation.ok, "{:?}", refusal_codes(&relation));
    let items = relation.data.unwrap()["items"].as_array().unwrap().clone();
    let sources: Vec<_> = items
        .iter()
        .filter(|item| item["relation"] == "source")
        .map(|item| item["at"].clone())
        .collect();
    assert_eq!(
        sources,
        [json!("main:Catalog.Items"), json!("main:Document.Order")]
    );
    let handlers: Vec<_> = items
        .iter()
        .filter(|item| item["relation"] == "handler")
        .collect();
    assert_eq!(
        handlers,
        [&json!({"relation": "handler", "at": TARGET, "kind": "Method"})]
    );
    let method = service.view(ViewRequest::new(handlers[0]["at"].as_str().unwrap()).unwrap());
    assert!(method.ok, "{:?}", refusal_codes(&method));
    assert_eq!(method.data.unwrap()["at"], TARGET);
    let inward =
        service.view(ViewRequest::new(&format!("{SUBSCRIPTION}.Relation.ПриЗаписи")).unwrap());
    assert_eq!(refusal_codes(&inward), ["not_found"]);
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read(module).unwrap(), bsl);
}

#[test]
fn malformed_handlers_do_not_invent_links_and_missing_methods_refuse_on_follow() {
    for handler in [
        "",
        "CommonModule",
        "CommonModule.Модуль",
        "CommonModule.Модуль.Метод.extra",
        "CommonModule.0Модуль.Метод",
        "main:CommonModule.Модуль.Метод",
        "CommonModule.Модуль.bad-name",
        "CommonModule.Модуль.НетМетода",
    ] {
        let fixture = RealReaderFixture::new();
        replace_handler(&fixture, handler);
        let service = fixture.view_service();
        let relation = service.view(ViewRequest::new(&format!("{SUBSCRIPTION}.Relation")).unwrap());
        assert!(relation.ok, "{handler}: {:?}", refusal_codes(&relation));
        let data = relation.data.unwrap();
        let links: Vec<_> = data["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["relation"] == "handler")
            .collect();
        if handler.ends_with(".НетМетода") {
            assert_eq!(links.len(), 1);
            assert_eq!(links[0]["at"], "main:CommonModule.Модуль.Method.НетМетода");
            let follow = service.view(ViewRequest::new(links[0]["at"].as_str().unwrap()).unwrap());
            assert_eq!(refusal_codes(&follow), ["not_found"]);
        } else {
            assert!(links.is_empty(), "{handler}: {links:?}");
        }
    }
}

#[test]
fn damaged_handler_xml_keeps_source_and_observation_limits() {
    for replacement in [
        "<Handler>CommonModule.РеактивныйСервер.ПриЗаписи</Handler><Handler>CommonModule.Other.Procedure</Handler>",
        "<Handler><nested>CommonModule.РеактивныйСервер.ПриЗаписи</nested></Handler>",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("EventSubscriptions/ПриЗаписи.xml");
        let xml = fs::read_to_string(&path).unwrap();
        write(&path, &xml.replace(&format!("<Handler>{HANDLER}</Handler>"), replacement));
        let service = fixture.view_service();
        let root = service.view(ViewRequest::new(SUBSCRIPTION).unwrap());
        assert!(root.ok, "{:?}", refusal_codes(&root));
        let data = root.data.unwrap();
        assert!(data["limits"].as_array().is_some_and(|limits| !limits.is_empty()), "{data:#}");
        let relation = service.view(ViewRequest::new(&format!("{SUBSCRIPTION}.Relation")).unwrap());
        assert!(relation.ok, "{:?}", refusal_codes(&relation));
        let items = relation.data.unwrap()["items"].as_array().unwrap().clone();
        assert_eq!(items.len(), 1, "{items:?}");
        assert_eq!(items[0]["relation"], "source");
        assert_eq!(items[0]["at"], "main:Catalog.Items");
    }
}
