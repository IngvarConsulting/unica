use super::*;

const MODULE: &str = "main:CommonModule.РеактивныйСервер";

#[test]
fn module_variables_are_current_named_declarations_with_truthful_branch_count() {
    let fixture = RealReaderFixture::new();
    let path = fixture
        .source
        .join("CommonModules/РеактивныйСервер/Ext/Module.bsl");
    fs::write(&path, "#Область Vars\r\nПерем А Экспорт, Б;\r\n#КонецОбласти\r\nПроцедура П()\r\nПерем Локальная;\r\nКонецПроцедуры").unwrap();
    let service = fixture.view_service();
    let read = |at: &str| {
        let result = service.view(ViewRequest::new(at).unwrap());
        assert!(result.ok, "{result:?}");
        result.data.unwrap()
    };
    let branch = format!("{MODULE}.Variable");
    let items = read(&branch);
    assert_eq!(items["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        items["items"][0]["props"],
        json!({"name":"А", "export":true, "line":2, "region":format!("{MODULE}.Region.Vars")})
    );
    assert_eq!(items["items"][1]["props"]["export"], false);
    let summary = read(MODULE);
    assert_eq!(
        summary["branches"]
            .as_array()
            .unwrap()
            .iter()
            .find(|b| b["at"] == branch)
            .unwrap()["count"],
        2
    );
    assert_eq!(
        read(&format!("{branch}.А"))["props"],
        items["items"][0]["props"]
    );
    fs::write(&path, "Перем Новая;").unwrap();
    // A new question owns a new authority; the prior one retains its snapshot.
    let current = fixture
        .view_service()
        .view(ViewRequest::new(&branch).unwrap());
    assert!(current.ok, "{current:?}");
    let current = current.data.unwrap();
    assert_eq!(current["items"].as_array().unwrap().len(), 1);
    assert_eq!(current["items"][0]["props"]["name"], "Новая");
}

#[test]
fn ambiguous_variable_names_and_regions_do_not_publish_false_addresses() {
    let fixture = RealReaderFixture::new();
    fs::write(fixture.source.join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
        "#Область Same\nПерем А;\n#КонецОбласти\n#Область Same\nПерем Б;\n#КонецОбласти\n#Если Сервер Тогда\nПерем Duplicate;\n#Иначе\nПерем duplicate;\n#КонецЕсли\n").unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(&format!("{MODULE}.Variable")).unwrap());
    assert!(result.ok, "{result:?}");
    let data = result.data.unwrap();
    let items = data["items"].as_array().unwrap();
    assert_eq!(items.len(), 4);
    for item in &items[..2] {
        assert!(item["props"].get("region").is_none());
        assert!(item["limits"][0].as_str().unwrap().contains("region"));
    }
    for item in &items[2..] {
        assert!(item.get("at").is_none());
        assert!(item["limits"][0].as_str().unwrap().contains("ambiguous"));
    }
    let result = service.view(ViewRequest::new(&format!("{MODULE}.Variable.Duplicate")).unwrap());
    assert!(!result.ok);
}

#[test]
fn variable_branch_is_empty_for_a_missing_module_file_and_refuses_extra_suffixes() {
    let fixture = RealReaderFixture::new();
    fs::remove_file(
        fixture
            .source
            .join("CommonModules/РеактивныйСервер/Ext/Module.bsl"),
    )
    .unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(&format!("{MODULE}.Variable")).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(result.data.unwrap()["items"], json!([]));
    for at in [
        format!("{MODULE}.Variable.Missing"),
        format!("{MODULE}.Variable.Missing.Body"),
        "main:CommonModule.Missing.Variable".to_string(),
    ] {
        assert!(!service.view(ViewRequest::new(&at).unwrap()).ok, "{at}");
    }
}
