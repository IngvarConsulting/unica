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
    assert!(result.ok, "{at}: {result:?}");
    result.data.unwrap()
}

fn compact(node: &Value, key: &str) -> Value {
    serde_json::from_str(node["props"][key].as_str().unwrap()).unwrap()
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
        compact(&rule, "selection"),
        json!([{"field":"ДополнительныеРеквизитыКонтрагент"}])
    );
    assert_eq!(
        compact(&rule, "filter"),
        json!([{"type":"FilterItemComparison", "left":{"type":"Field","value":"ДополнительныеРеквизиты.Контрагент"},"comparisonType":"NotFilled"}])
    );
    assert_eq!(
        compact(&rule, "appearance"),
        json!([
            {"type":"SettingsParameterValue","parameter":"ЦветТекста","value":{"type":"Color","value":"win:DisabledText"}},
            {"type":"SettingsParameterValue","parameter":"Текст","value":{"type":"LocalStringType","values":[{"language":"ru","content":"Все контрагенты"}]}}
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
        compact(&rule, "filter"),
        json!([{"type":"FilterItemGroup", "use":false, "groupType":"AndGroup", "items":[
            {"type":"FilterItemComparison", "left":{"type":"Field","value":"ДополнительныеРеквизиты.Контрагент"},"comparisonType":"NotFilled"},
            {"type":"FilterItemComparison", "left":{"type":"boolean","value":false},"comparisonType":"Equal","right":{"type":"decimal","value":"0"}}
        ]}])
    );
    assert_eq!(compact(&rule, "appearance")[1]["use"], false);
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
        compact(
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
            "bad fact must not be silently partial: {rule}"
        );
        assert!(rule["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("appearance")));
        assert_eq!(
            compact(&rule, "selection")[0]["field"],
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
        assert!(result.ok, "{result:?}");
        let data = result.data.unwrap();
        let items = data["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["at"], format!("{at}.Rule{ordinal}"));
        assert_eq!(
            compact(&items[0], "selection")[0]["field"],
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
            "Rule1",
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
            "{node}"
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
    let appearance = compact(
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
        compact(
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
