use super::*;

const FORM: &str = "main:Report.ParityReport.Form.MainForm";

fn install(fixture: &RealReaderFixture, extra: &str) {
    write(
        &fixture
            .source
            .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml"),
        &format!(
            r#"<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" version="2.20">
<ChildItems><UsualGroup name="Группа" id="1"><ChildItems>
<InputField name="Код" id="2"><DataPath>Объект.Code</DataPath><Visible>false</Visible></InputField>
<Table name="Товары" id="3"><DataPath>Объект.Товары</DataPath><ChildItems><InputField name="Количество" id="4"><DataPath>Объект.Товары.Quantity</DataPath></InputField></ChildItems></Table>
<Button name="Заполнить" id="5"><CommandName>Form.Command.Заполнить</CommandName></Button>
<Button name="Закрыть" id="6"><CommandName>Form.StandardCommand.Close</CommandName></Button>
<Button name="Общая" id="7"><DataPath>Объект.Ref</DataPath><CommandName>CommonCommand.ОбщаяКоманда</CommandName></Button>
<UsualGroup name="Свободная" id="8"/>
{extra}</ChildItems></UsualGroup></ChildItems></Form>"#
        ),
    );
}

#[test]
fn form_item_bindings_reach_nested_collection_and_detail_with_exact_targets() {
    let fixture = RealReaderFixture::new();
    install(&fixture, "");
    let path = fixture
        .source
        .join("Reports/ParityReport/Forms/MainForm/Ext/Form.xml");
    let before = fs::read(&path).unwrap();
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new(&format!("{FORM}.Item.Группа.Item")).unwrap());
    assert!(collection.ok, "the nested Item collection must be readable");
    let rows = collection.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    for (name, data_path, command_name) in [
        ("Код", Some("Объект.Code"), None),
        ("Товары", Some("Объект.Товары"), None),
        ("Заполнить", None, Some("Form.Command.Заполнить")),
        ("Закрыть", None, Some("Form.StandardCommand.Close")),
        (
            "Общая",
            Some("Объект.Ref"),
            Some("CommonCommand.ОбщаяКоманда"),
        ),
        ("Свободная", None, None),
    ] {
        let at = format!("{FORM}.Item.Группа.Item.{name}");
        let row = rows.iter().find(|row| row["at"] == at).unwrap();
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(result.ok, "the addressed item must remain readable");
        let detail = result.data.as_ref().unwrap();
        for node in [row, detail] {
            assert_eq!(
                node["props"]
                    .get("DataPath")
                    .and_then(serde_json::Value::as_str),
                data_path
            );
            assert_eq!(
                node["props"]
                    .get("CommandName")
                    .and_then(serde_json::Value::as_str),
                command_name
            );
            assert_eq!(node["at"], at);
            assert!(node["props"]["enabled"].is_boolean());
        }
    }
    let column = service.view(
        ViewRequest::new(&format!("{FORM}.Item.Группа.Item.Товары.Item.Количество")).unwrap(),
    );
    assert!(column.ok);
    assert_eq!(
        column.data.as_ref().unwrap()["props"]["DataPath"],
        "Объект.Товары.Quantity"
    );
    let field = service.view(ViewRequest::new(&format!("{FORM}.Item.Группа.Item.Код")).unwrap());
    assert_eq!(field.data.as_ref().unwrap()["props"]["visible"], false);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn form_item_bindings_distinguish_absence_empty_and_local_damage() {
    for (raw, damaged) in [
        ("", false),
        ("<DataPath/>", false),
        ("<DataPath> \n\t </DataPath>", false),
        ("<DataPath><Nested>bad</Nested></DataPath>", true),
        ("<DataPath>One</DataPath><DataPath>Two</DataPath>", true),
        ("<DataPath xmlns=\"urn:foreign\">Foreign</DataPath>", true),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &format!("<Button name=\"Проба\" id=\"9\">{raw}<CommandName>Form.Command.Заполнить</CommandName></Button>"));
        let service = fixture.view_service();
        let at = format!("{FORM}.Item.Группа.Item.Проба");
        let result = service.view(ViewRequest::new(&at).unwrap());
        assert!(
            result.ok,
            "damage to one binding must retain independent facts"
        );
        let node = result.data.as_ref().unwrap();
        assert!(node["props"].get("DataPath").is_none());
        assert_eq!(node["props"]["CommandName"], "Form.Command.Заполнить");
        assert_eq!(
            node.get("limits")
                .is_some_and(|v| v.as_array().is_some_and(|a| a
                    .iter()
                    .any(|v| v.as_str().is_some_and(|s| s.contains("DataPath"))))),
            damaged
        );
        assert!(node["props"]["visible"].is_boolean());
    }
}

#[test]
fn form_item_bindings_use_public_property_names_for_compact_and_whole_reads() {
    for (property, source_key) in [("DataPath", "dataPath"), ("CommandName", "commandName")] {
        let fixture = RealReaderFixture::new();
        let text = "Ц".repeat(1025);
        install(
            &fixture,
            &format!("<Button name=\"Длинная\" id=\"9\"><{property}>{text}</{property}></Button>"),
        );
        let at = format!("{FORM}.Item.Группа.Item.Длинная");
        let service = fixture.view_service();
        let compact = service.view(ViewRequest::new(&at).unwrap());
        assert!(compact.ok);
        let node = compact.data.as_ref().unwrap();
        assert!(node["props"].get(property).is_none());
        assert!(node["props"].get(source_key).is_none());
        assert!(node["limits"].as_array().unwrap().iter().any(|v| v
            .as_str()
            .is_some_and(|s| s.contains(&format!(".Props.{property}")))));
        let whole = crate::infrastructure::v13_read_projection::with_whole_prop(property, || {
            service.view(ViewRequest::new(&at).unwrap())
        });
        assert!(whole.ok);
        assert_eq!(whole.data.as_ref().unwrap()["props"][property], text);
    }
}

#[test]
fn form_item_bindings_preserve_baseline_size_limits_with_both_targets() {
    let fixture = RealReaderFixture::new();
    let title = "З".repeat(1025);
    install(
        &fixture,
        &format!(
            "<Button name=\"Контроль\" id=\"9\">\
             <Title xmlns:v8=\"http://v8.1c.ru/8.1/data/core\"><v8:item>\
             <v8:lang>ru</v8:lang><v8:content>{title}</v8:content></v8:item></Title>\
             <DataPath>Объект.Code</DataPath>\
             <CommandName>CommonCommand.Проба</CommandName></Button>"
        ),
    );
    let at = format!("{FORM}.Item.Группа.Item.Контроль");
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new(&format!("{FORM}.Item.Группа.Item")).unwrap());
    assert!(collection.ok);
    let row = collection.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["at"] == at)
        .unwrap();
    let detail = service.view(ViewRequest::new(&at).unwrap());
    assert!(detail.ok);
    for node in [row, detail.data.as_ref().unwrap()] {
        assert!(node["props"].get("title").is_none());
        assert!(
            node["limits"]
                .as_array()
                .is_some_and(|limits| limits.iter().any(|limit| {
                    limit
                        .as_str()
                        .is_some_and(|text| text.contains(&format!("{at}.Props.title")))
                })),
            "the baseline oversized title must retain its whole-property address"
        );
        assert_eq!(node["props"]["DataPath"], "Объект.Code");
        assert_eq!(node["props"]["CommandName"], "CommonCommand.Проба");
    }
    let whole = crate::infrastructure::v13_read_projection::with_whole_prop("title", || {
        service.view(ViewRequest::new(&at).unwrap())
    });
    assert!(whole.ok);
    assert_eq!(whole.data.as_ref().unwrap()["props"]["title"], title);
    assert_eq!(
        whole.data.as_ref().unwrap()["props"]["DataPath"],
        "Объект.Code"
    );
    assert_eq!(
        whole.data.as_ref().unwrap()["props"]["CommandName"],
        "CommonCommand.Проба"
    );
}
