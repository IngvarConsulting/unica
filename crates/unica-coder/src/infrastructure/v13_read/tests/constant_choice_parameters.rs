use super::*;

const AT: &str = "main:Constant.MainCurrency";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn install(fixture: &RealReaderFixture, content: &str) -> PathBuf {
    let path = fixture.source.join("Constants/MainCurrency.xml");
    let xml = fs::read_to_string(&path).unwrap();
    write(
        &path,
        &xml.replace("</Properties>", &format!("{content}</Properties>")),
    );
    path
}

fn entries(values: &[&str]) -> String {
    let items = values.iter().map(|value| format!(r#"<app:item name="Отбор.ТипИзмеряемойВеличины"><app:value xsi:type="xr:DesignTimeRef">Enum.ТипыИзмеряемыхВеличин.EnumValue.{value}</app:value></app:item>"#)).collect::<String>();
    format!(
        r#"<ChoiceParameters xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:xr="{XR}">{items}</ChoiceParameters>"#
    )
}
fn expected(values: &[&str]) -> serde_json::Value {
    json!(values.iter().map(|value| json!({"name":"Отбор.ТипИзмеряемойВеличины","value":{"type":{"namespace":XR,"localName":"DesignTimeRef"},"reference":format!("Enum.ТипыИзмеряемыхВеличин.EnumValue.{value}")}})).collect::<Vec<_>>())
}

#[test]
fn constant_choice_parameters_preserve_six_values_order_and_duplicates() {
    let fixture = RealReaderFixture::new();
    let values = [
        "Вес",
        "Длина",
        "КоличествоШтук",
        "Объем",
        "Площадь",
        "Объем",
    ];
    let path = install(&fixture, &entries(&values));
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(AT).unwrap());
    assert!(result.ok, "{result:?}");
    assert_eq!(
        result.data.as_ref().unwrap()["props"]["choiceParameters"],
        expected(&values)
    );
    assert!(result.data.as_ref().unwrap()["props"].get("Type").is_some());
    let relations = service.view(ViewRequest::new(&format!("{AT}.Relation")).unwrap());
    assert!(relations.ok, "{relations:?}");
    let rows = relations.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(rows.len(), 6);
    for (row, value) in rows.iter().zip(values) {
        assert_eq!(
            *row,
            json!({"relation":"constantChoiceParameter","at":format!("main:Enum.ТипыИзмеряемыхВеличин.EnumValue.{value}"),"kind":"EnumValue"})
        );
    }
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn constant_choice_parameters_keep_absence_empty_damage_and_unsupported_distinct() {
    let valid = entries(&["Вес"]);
    let cases = vec![
        (String::new(), None, "absent"),
        ("<ChoiceParameters/>".to_string(), Some(json!([])), ""),
        (valid.replace("xmlns:xr=", "xmlns:alias=").replace("xr:DesignTimeRef","alias:DesignTimeRef"), Some(expected(&["Вес"])), ""),
        (valid.replace(XR,"urn:foreign"), None, "DesignTimeRef profile"),
        (valid.replace("xr:DesignTimeRef","xr:Unknown"), None, "DesignTimeRef profile"),
        (valid.replace("xsi:type=\"xr:DesignTimeRef\"", ""), None, "structure"),
        (valid.replace("name=\"Отбор.ТипИзмеряемойВеличины\"", ""), None, "named app:item"),
        (valid.replace("</app:value>","<Nested/></app:value>"), None, "structure"),
        (valid.replace("</app:item>","<app:value xsi:type=\"xr:DesignTimeRef\">Enum.X.EnumValue.Y</app:value></app:item>"), None, "one app:value"),
        (valid.replace("<app:item ","<app:item extra=\"x\" "), None, "named app:item"),
        (valid.replace("</app:item>","text</app:item>"), None, "named app:item"),
        (valid.replace("Enum.ТипыИзмеряемыхВеличин.EnumValue.Вес","Catalog.Any"), None, "concrete EnumValue"),
        (valid.replace("Enum.ТипыИзмеряемыхВеличин.EnumValue.Вес","Enum.ТипыИзмеряемыхВеличин.EnumValue.Bad-Name"), None, "concrete EnumValue"),
        ("<ChoiceParameters/><ChoiceParameters/>".to_string(), None, ""),
    ];
    for (raw, want, limit) in cases {
        let fixture = RealReaderFixture::new();
        let path = install(
            &fixture,
            &format!("{raw}<MarkNegatives>false</MarkNegatives><ChoiceParameterLinks/>"),
        );
        let before = fs::read(&path).unwrap();
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new(AT).unwrap());
        assert!(result.ok, "{result:?}");
        let data = result.data.as_ref().unwrap();
        assert_eq!(
            data["props"].get("choiceParameters"),
            want.as_ref(),
            "{raw}: {data}"
        );
        assert!(data["props"].get("Type").is_some());
        assert_eq!(data["props"]["MarkNegatives"], false);
        let limits = data["limits"].to_string();
        if want.is_some() {
            assert!(
                !limits.contains("choiceParameters") && !limits.contains("ChoiceParameters"),
                "{data}"
            );
        } else {
            assert!(
                limits.contains("choiceParameters") || limits.contains("ChoiceParameters"),
                "{data}"
            );
            assert!(limits.contains(limit), "{data}");
            let relation = service.view(ViewRequest::new(&format!("{AT}.Relation")).unwrap());
            assert!(relation.ok, "{relation:?}");
            assert!(relation.data.as_ref().unwrap()["limits"]
                .to_string()
                .contains("choiceParameters"));
        }
        assert_eq!(fs::read(path).unwrap(), before);
    }
}

#[test]
fn constant_choice_parameter_limit_keeps_whole_value_and_all_relation_pages() {
    let fixture = RealReaderFixture::new();
    let values = vec!["Объем"; 40];
    let path = install(&fixture, &entries(&values));
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new(AT).unwrap());
    assert!(result.ok, "{result:?}");
    let data = result.data.as_ref().unwrap();
    assert!(data["props"].get("choiceParameters").is_none());
    assert!(data["limits"]
        .to_string()
        .contains(&format!("{AT}.Props.choiceParameters")));
    let whole =
        crate::infrastructure::v13_read_projection::with_whole_prop("choiceParameters", || {
            service.view(ViewRequest::new(AT).unwrap())
        });
    assert_eq!(
        whole.data.as_ref().unwrap()["props"]["choiceParameters"],
        expected(&values)
    );
    let mut count = 0;
    let mut cursor = None;
    loop {
        let mut request = ViewRequest::new(&format!("{AT}.Relation"))
            .unwrap()
            .with_limit(7)
            .unwrap();
        if let Some(c) = cursor.take() {
            request = request.with_cursor(c);
        }
        let page = service.view(request);
        assert!(page.ok, "{page:?}");
        count += page.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .len();
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(count, 40);
    assert_eq!(fs::read(path).unwrap(), before);
}
