use super::{install_form_metadata_test_sources, write, RealReaderFixture, ViewRequest};
use serde_json::{json, Value};
use unica_workspace::test_support::tree_snapshot;

const XML: &str = include_str!("../../../../../../tests/fixtures/acceptance/form-conditional-appearance/src/CommonForms/Appearance/Ext/Form.xml");
const FORM: &str = "main:CommonForm.ОбщаяФорма";
const SETTING: &str = "main:CommonForm.ОбщаяФорма.Setting.ConditionalAppearance";

fn appearance_fixture(xml: &str) -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    install_form_metadata_test_sources(&fixture);
    write(
        &fixture.source.join("CommonForms/ОбщаяФорма/Ext/Form.xml"),
        xml,
    );
    fixture
}

fn read(fixture: &RealReaderFixture, at: &str) -> Value {
    let result = fixture.view_service().view(ViewRequest::new(at).unwrap());
    assert!(result.ok, "{at}: the appearance view failed");
    result.data.unwrap()
}

/// Rule facts are nested JSON, never a JSON text packed into a string.
fn fact(node: &Value, key: &str) -> Value {
    let value = node["props"][key].clone();
    assert!(
        value.is_array() || value.is_object(),
        "{key}: a fact is nested JSON"
    );
    value
}

#[test]
fn form_conditional_appearance_reaches_public_view_with_all_typed_facts_without_writes() {
    let fixture = appearance_fixture(XML);
    let before = tree_snapshot(&fixture.source);
    let form = read(&fixture, FORM);
    assert!(
        form["branches"]
            .as_array()
            .unwrap()
            .iter()
            .any(|b| b["at"] == format!("{FORM}.Setting")),
        "form must announce its conditional appearance"
    );
    let settings = read(&fixture, &format!("{FORM}.Setting"));
    assert_eq!(settings["items"][0]["at"], SETTING);
    let setting = read(&fixture, SETTING);
    assert_eq!(
        setting["props"]["sourcePath"],
        "Attributes/ConditionalAppearance"
    );
    let rules = read(&fixture, &format!("{SETTING}.Item"));
    assert_eq!(rules["items"].as_array().unwrap().len(), 1);
    let rule = read(&fixture, rules["items"][0]["at"].as_str().unwrap());
    assert_eq!(rules["items"][0], rule);
    assert_eq!(
        fact(&rule, "selection"),
        json!([{"field":"ДополнительныеРеквизитыКонтрагент"}])
    );
    assert_eq!(
        fact(&rule, "filter"),
        json!([{"type":"FilterItemComparison", "left":{"type":"Field","value":"ДополнительныеРеквизиты.Контрагент"},"comparisonType":"NotFilled"}])
    );
    assert_eq!(
        fact(&rule, "appearance"),
        json!([
            {"type":"SettingsParameterValue","parameter":"ЦветТекста","value":{"type":"Color","value":"win:DisabledText"}},
            {"type":"SettingsParameterValue","parameter":"Текст","value":{"kind":"localizedString","values":[{"language":"ru","content":"Все контрагенты"}]}}
        ])
    );
    assert!(rule
        .get("limits")
        .is_none_or(|v| v.as_array().unwrap().is_empty()));
    assert_eq!(
        read(&fixture, &format!("{FORM}.Attribute"))["items"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert_eq!(tree_snapshot(&fixture.source), before);
}

#[test]
fn form_conditional_appearance_keeps_nested_filters_false_zero_and_parameter_use() {
    let xml = XML.replace("<dcsset:filter>", "<dcsset:use>false</dcsset:use><dcsset:filter><dcsset:item xsi:type=\"dcsset:FilterItemGroup\"><dcsset:groupType>AndGroup</dcsset:groupType><dcsset:use>false</dcsset:use>")
        .replace("</dcsset:filter>", "<dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"xs:boolean\">false</dcsset:left><dcsset:comparisonType>Equal</dcsset:comparisonType><dcsset:right xsi:type=\"xs:decimal\">0</dcsset:right></dcsset:item></dcsset:item></dcsset:filter>")
        .replace("<dcscor:parameter>Текст", "<dcscor:use>false</dcscor:use><dcscor:parameter>Текст");
    let fixture = appearance_fixture(&xml);
    let rule = read(&fixture, &format!("{SETTING}.Item.Rule1"));
    assert_eq!(rule["props"]["use"], false);
    assert_eq!(
        fact(&rule, "filter"),
        json!([{"type":"FilterItemGroup", "use":false, "groupType":"AndGroup", "items":[
            {"type":"FilterItemComparison", "left":{"type":"Field","value":"ДополнительныеРеквизиты.Контрагент"},"comparisonType":"NotFilled"},
            {"type":"FilterItemComparison", "left":{"type":"boolean","value":false},"comparisonType":"Equal","right":{"type":"decimal","value":"0"}}
        ]}])
    );
    assert_eq!(fact(&rule, "appearance")[1]["use"], false);
}

#[test]
fn form_conditional_appearance_absent_empty_aliases_and_bad_values_are_distinct() {
    let start = XML.find("<ConditionalAppearance>").unwrap();
    let end = XML.find("</ConditionalAppearance>").unwrap() + "</ConditionalAppearance>".len();
    for (replacement, present) in [("", false), ("<ConditionalAppearance/>", true)] {
        let mut xml = XML.to_string();
        xml.replace_range(start..end, replacement);
        let fixture = appearance_fixture(&xml);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(SETTING).unwrap());
        assert_eq!(result.ok, present);
        if present {
            assert_eq!(result.data.unwrap()["props"]["ruleCount"], 0);
        }
        assert_eq!(
            read(&fixture, &format!("{FORM}.Attribute"))["items"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }
    let fixture = appearance_fixture(
        &XML.replace("dcsset", "s")
            .replace("dcscor", "c")
            .replace("v8ui", "u"),
    );
    assert_eq!(
        fact(
            &read(&fixture, &format!("{SETTING}.Item.Rule1")),
            "appearance"
        )[0]["value"]["type"],
        "Color"
    );

    for xml in [
        XML.replace("xsi:type=\"v8ui:Color\"", "xsi:type=\"xs:Unknown\""),
        XML.replace(
            "<dcscor:value xsi:type=\"v8ui:Color\"",
            "<dcscor:value xsi:nil=\"true\" xsi:type=\"v8ui:Color\"",
        ),
        XML.replace("http://v8.1c.ru/8.1/data/ui\"", "urn:spoof\""),
        XML.replace(
            "<v8:lang>ru</v8:lang>",
            "<v8:lang>ru</v8:lang><v8:lang>en</v8:lang>",
        ),
        XML.replace("Все контрагенты", &"Я".repeat(1100)),
    ] {
        let fixture = appearance_fixture(&xml);
        let rule = read(&fixture, &format!("{SETTING}.Item.Rule1"));
        assert!(
            rule["props"].get("appearance").is_none(),
            "bad fact must not be silently partial"
        );
        assert!(rule["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("appearance")));
        assert_eq!(
            fact(&rule, "selection")[0]["field"],
            "ДополнительныеРеквизитыКонтрагент"
        );
        assert_eq!(
            read(&fixture, &format!("{FORM}.Attribute"))["items"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }
}

#[test]
fn form_conditional_appearance_pages_in_source_order_and_opens_every_rule() {
    let start = XML.find("<dcsset:item>").unwrap();
    let end = XML.find("</ConditionalAppearance>").unwrap();
    let rules_xml = &XML[start..end];
    let ordered = (1..=4)
        .map(|i| rules_xml.replace("ДополнительныеРеквизитыКонтрагент", &format!("Target{i}")))
        .collect::<String>();
    let xml = XML.replacen(rules_xml, &ordered, 1);
    let fixture = appearance_fixture(&xml);
    let service = fixture.view_service();
    let at = format!("{SETTING}.Item");
    let mut request = ViewRequest::new(&at).unwrap().with_limit(1).unwrap();
    for ordinal in 1..=4 {
        let result = service.view(request);
        assert!(result.ok, "page {ordinal} of the rules failed");
        let data = result.data.unwrap();
        let items = data["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["at"], format!("{at}.Rule{ordinal}"));
        assert_eq!(
            fact(&items[0], "selection")[0]["field"],
            format!("Target{ordinal}")
        );
        assert_eq!(read(&fixture, items[0]["at"].as_str().unwrap()), items[0]);
        if ordinal < 4 {
            request = ViewRequest::new(&at)
                .unwrap()
                .with_limit(1)
                .unwrap()
                .with_cursor(result.cursor.unwrap());
        } else {
            break;
        }
    }
}

#[test]
fn form_conditional_appearance_reports_unknown_container_and_rule_structure() {
    for (xml, at, field) in [
        (
            XML.replace(
                "<ConditionalAppearance>",
                "<ConditionalAppearance><Unknown/>",
            ),
            SETTING.to_string(),
            "ConditionalAppearance",
        ),
        (
            XML.replace("<dcsset:filter>", "<Unknown/><dcsset:filter>"),
            format!("{SETTING}.Item.Rule1"),
            "Rule1: неподдержанный дочерний элемент Unknown",
        ),
        (
            XML.replace(
                "<dcsset:comparisonType>NotFilled</dcsset:comparisonType>",
                "<dcsset:comparisonType>Alien</dcsset:comparisonType>",
            ),
            format!("{SETTING}.Item.Rule1"),
            "filter",
        ),
        (
            XML.replace(
                "<dcsset:filter>",
                "<dcsset:use>garbage</dcsset:use><dcsset:filter>",
            ),
            format!("{SETTING}.Item.Rule1"),
            "use",
        ),
    ] {
        let fixture = appearance_fixture(&xml);
        let node = read(&fixture, &at);
        assert!(
            node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().unwrap().contains(field)),
            "the limit names {field}"
        );
        assert_eq!(
            read(&fixture, &format!("{FORM}.Attribute"))["items"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
    }
}

#[test]
fn form_conditional_appearance_keeps_localized_text_segments_and_empty_strings() {
    let xml = XML
        .replace(
            "Все контрагенты",
            "Все <!-- note -->контрагенты &amp; партнёры",
        )
        .replace(
            "</v8:item></dcscor:value>",
            "</v8:item><v8:item><v8:lang>en</v8:lang><v8:content/></v8:item></dcscor:value>",
        )
        .replace("xmlns:win=", "xmlns:windows=")
        .replace("win:DisabledText", "windows:DisabledText");
    let fixture = appearance_fixture(&xml);
    let appearance = fact(
        &read(&fixture, &format!("{SETTING}.Item.Rule1")),
        "appearance",
    );
    assert_eq!(appearance[0]["value"]["value"], "win:DisabledText");
    assert_eq!(
        appearance[1]["value"]["values"],
        json!([
            {"language":"ru","content":"Все контрагенты & партнёры"},
            {"language":"en","content":""}
        ])
    );
    let fixture = appearance_fixture(&XML.replace(
        "xsi:type=\"v8ui:Color\">win:DisabledText",
        "xsi:type=\"xs:string\">",
    ));
    assert_eq!(
        fact(
            &read(&fixture, &format!("{SETTING}.Item.Rule1")),
            "appearance"
        )[0]["value"],
        json!({"type":"string","value":""})
    );
}

#[test]
fn form_conditional_appearance_bounds_depth_count_and_never_reads_dynamic_list_settings() {
    for (replacement, expected) in [
        (
            "<dcsset:item><dcsset:field>X</dcsset:field></dcsset:item>".repeat(130),
            "256",
        ),
        (
            format!(
                "{}<dcsset:item><dcsset:field>X</dcsset:field></dcsset:item>{}",
                "<dcsset:item>".repeat(33),
                "</dcsset:item>".repeat(33)
            ),
            "32",
        ),
    ] {
        let xml = XML.replace("<dcsset:item><dcsset:field>ДополнительныеРеквизитыКонтрагент</dcsset:field></dcsset:item>", &replacement);
        let fixture = appearance_fixture(&xml);
        let rule = read(&fixture, &format!("{SETTING}.Item.Rule1"));
        assert!(rule["props"].get("selection").is_none());
        assert!(rule["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains(expected)));
        assert!(rule["props"].get("appearance").is_some());
    }
    let xml = XML
        .replace(
            "<ConditionalAppearance>",
            "<Attribute name=\"Список\" id=\"5\"><Settings><ConditionalAppearance>",
        )
        .replace(
            "</ConditionalAppearance>",
            "</ConditionalAppearance></Settings></Attribute>",
        );
    let fixture = appearance_fixture(&xml);
    assert!(
        !fixture
            .view_service()
            .view(ViewRequest::new(SETTING).unwrap())
            .ok
    );
}

/// Values the platform writes in managed-form conditional appearance of
/// 8.3.27 exports: list comparisons repeat `right`, an undefined right value
/// is omitted, fonts are attributes, and a rule may carry its own caption.
#[test]
fn form_conditional_appearance_reads_platform_value_kinds_and_rule_presentation() {
    let filter = concat!(
        "<dcsset:filter>",
        "<dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"dcscor:Field\">Статус</dcsset:left><dcsset:comparisonType>InList</dcsset:comparisonType>",
        "<dcsset:right xsi:type=\"dcscor:DesignTimeValue\">Перечисление.Статусы.Новый</dcsset:right>",
        "<dcsset:right xsi:type=\"dcscor:DesignTimeValue\">Перечисление.Статусы.Закрыт</dcsset:right></dcsset:item>",
        "<dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"dcscor:Field\">Номер</dcsset:left><dcsset:comparisonType>NotBeginsWith</dcsset:comparisonType><dcsset:right xsi:type=\"xs:string\">Т</dcsset:right></dcsset:item>",
        "<dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"dcscor:Field\">Склад</dcsset:left><dcsset:comparisonType>Equal</dcsset:comparisonType></dcsset:item>",
        "<dcsset:item xsi:type=\"dcsset:FilterItemComparison\"><dcsset:left xsi:type=\"dcscor:Field\">Дата</dcsset:left><dcsset:comparisonType>Less</dcsset:comparisonType>",
        "<dcsset:right xsi:type=\"v8:StandardBeginningDate\"><v8:variant xsi:type=\"v8:StandardBeginningDateVariant\">Custom</v8:variant><v8:date>2020-01-01T00:00:00</v8:date></dcsset:right></dcsset:item>",
        "</dcsset:filter>",
    );
    let appearance = concat!(
        "<dcsset:appearance>",
        "<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\"><dcscor:parameter>Шрифт</dcscor:parameter><dcscor:value xsi:type=\"v8ui:Font\" ref=\"style:ВажныйШрифт\" bold=\"true\" italic=\"false\" height=\"10\" kind=\"StyleItem\"/></dcscor:item>",
        "<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\"><dcscor:parameter>Шрифт</dcscor:parameter><dcscor:value xsi:type=\"v8ui:Font\" ref=\"sys:DefaultGUIFont\" faceName=\"Arial\" scale=\"100\" kind=\"WindowsFont\"/></dcscor:item>",
        "<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\"><dcscor:parameter>ГоризонтальноеПоложение</dcscor:parameter><dcscor:value xsi:type=\"v8ui:HorizontalAlign\">Right</dcscor:value></dcscor:item>",
        "<dcscor:item xsi:type=\"dcsset:SettingsParameterValue\"><dcscor:parameter>ЦветФона</dcscor:parameter><dcscor:value xsi:type=\"v8ui:Color\">auto</dcscor:value></dcscor:item>",
        "</dcsset:appearance>",
        "<dcsset:presentation xsi:type=\"v8:LocalStringType\"><v8:item><v8:lang>ru</v8:lang><v8:content>Закрытые серым</v8:content></v8:item></dcsset:presentation>",
    );
    let start = XML.find("<dcsset:filter>").unwrap();
    let end = XML.find("</dcsset:appearance>").unwrap() + "</dcsset:appearance>".len();
    let mut xml = XML.replace(
        "xmlns:win=",
        "xmlns:style=\"http://v8.1c.ru/8.1/data/ui/style\" xmlns:sys=\"http://v8.1c.ru/8.1/data/ui/fonts/system\" xmlns:win=",
    );
    let shift = xml.len() - XML.len();
    xml.replace_range(start + shift..end + shift, &format!("{filter}{appearance}"));
    let fixture = appearance_fixture(&xml);
    let rule = read(&fixture, &format!("{SETTING}.Item.Rule1"));
    assert!(
        rule.get("limits")
            .is_none_or(|v| v.as_array().unwrap().is_empty()),
        "the rule has no limits"
    );
    assert_eq!(
        fact(&rule, "presentation"),
        json!({"kind":"localizedString","values":[{"language":"ru","content":"Закрытые серым"}]})
    );
    assert_eq!(
        fact(&rule, "filter"),
        json!([
            {"type":"FilterItemComparison","left":{"type":"Field","value":"Статус"},"comparisonType":"InList","right":[
                {"type":"DesignTimeValue","value":"Перечисление.Статусы.Новый"},
                {"type":"DesignTimeValue","value":"Перечисление.Статусы.Закрыт"}
            ]},
            {"type":"FilterItemComparison","left":{"type":"Field","value":"Номер"},"comparisonType":"NotBeginsWith","right":{"type":"string","value":"Т"}},
            {"type":"FilterItemComparison","left":{"type":"Field","value":"Склад"},"comparisonType":"Equal"},
            {"type":"FilterItemComparison","left":{"type":"Field","value":"Дата"},"comparisonType":"Less","right":
                {"type":"StandardBeginningDate","value":{"variant":"Custom","date":"2020-01-01T00:00:00"}}}
        ])
    );
    let values: Vec<Value> = fact(&rule, "appearance")
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["value"].clone())
        .collect();
    assert_eq!(
        values,
        [
            json!({"type":"Font","value":{"ref":"style:ВажныйШрифт","bold":true,"italic":false,"height":"10","kind":"StyleItem"}}),
            json!({"type":"Font","value":{"ref":"sys:DefaultGUIFont","faceName":"Arial","scale":"100","kind":"WindowsFont"}}),
            json!({"type":"HorizontalAlign","value":"Right"}),
            json!({"type":"Color","value":"auto"}),
        ]
    );

    let plain = XML.replace(
        "</dcsset:appearance>",
        "</dcsset:appearance><dcsset:presentation xsi:type=\"xs:string\">Пусто</dcsset:presentation>",
    );
    let rule = read(
        &appearance_fixture(&plain),
        &format!("{SETTING}.Item.Rule1"),
    );
    assert_eq!(rule["props"]["presentation"], "Пусто");

    for (bad, field) in [
        ("bold=\"true\"", "bold=\"yes\""),
        ("ref=\"style:ВажныйШрифт\"", "ref=\"urn:ВажныйШрифт\""),
        ("kind=\"StyleItem\"", "kind=\"StyleItem\" color=\"red\""),
        (">Right<", ">Sideways<"),
    ] {
        let rule = read(
            &appearance_fixture(&xml.replacen(bad, field, 1)),
            &format!("{SETTING}.Item.Rule1"),
        );
        assert!(
            rule["props"].get("appearance").is_none(),
            "{field}: the damaged appearance is dropped"
        );
        assert!(
            rule["props"].get("filter").is_some(),
            "{field}: the filter stays"
        );
    }
}
