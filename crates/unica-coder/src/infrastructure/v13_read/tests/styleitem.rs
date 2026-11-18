use super::*;

#[test]
fn styleitem_font_border_values_reach_flat_public_props() {
    let fixture = RealReaderFixture::new();
    let configuration = fixture.source.join("Configuration.xml");
    let mut xml = fs::read_to_string(&configuration).unwrap();
    for (name, expected) in [
        (
            "StyleFont",
            json!({"Type":"Font", "fontRef":"style:NormalTextFont", "fontKind":"StyleItem", "fontBold":true, "fontItalic":false, "fontUnderline":false, "fontStrikeout":false}),
        ),
        (
            "AbsoluteFont",
            json!({"Type":"Font", "fontFaceName":"Arial", "fontHeight":16, "fontScale":100, "fontKind":"Absolute", "fontBold":true, "fontItalic":false, "fontUnderline":false, "fontStrikeout":false}),
        ),
        (
            "SingleBorder",
            json!({"Type":"Border", "borderWidth":1, "borderStyle":"Single"}),
        ),
    ] {
        xml = xml.replace(
            "</ChildObjects>",
            &format!("<StyleItem>{name}</StyleItem></ChildObjects>"),
        );
        write(
            &fixture.source.join(format!("StyleItems/{name}.xml")),
            &fixture_text(&format!("styleitem_values/{name}.xml")),
        );
        write(&configuration, &xml);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(&format!("main:StyleItem.{name}")).unwrap());
        assert!(result.ok, "the call failed");
        let props = &result.data.as_ref().unwrap()["props"];
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&props[key], value, "{name}.{key}");
        }
        assert_eq!(props["kind"], "StyleItem");
        assert!(props
            .as_object()
            .unwrap()
            .values()
            .all(|value| !value.is_object() && !value.is_array()));
    }
}

#[test]
fn styleitem_preserves_absent_false_and_damaged_overrides_independently() {
    for (edit, expected_flag, limited) in [
        ("italic=\"false\"", Some(false), false),
        ("italic=\"true\"", Some(true), false),
        ("", None, false),
        ("italic=\"broken\"", None, true),
        ("italic=\"\"", None, true),
    ] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        let xml = fs::read_to_string(&configuration).unwrap();
        write(
            &configuration,
            &xml.replace(
                "</ChildObjects>",
                "<StyleItem>AbsoluteFont</StyleItem></ChildObjects>",
            ),
        );
        let source = fixture_text("styleitem_values/AbsoluteFont.xml")
            .replace("italic=\"false\"", edit)
            .replace("scale=\"100\"", "scale=\"100\" vendor=\"private\"");
        write(&fixture.source.join("StyleItems/AbsoluteFont.xml"), &source);
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:StyleItem.AbsoluteFont").unwrap());
        assert!(result.ok, "the call failed");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["props"]["fontHeight"], 16);
        assert_eq!(data["props"]["Type"], "Font");
        assert!(data["props"]["Synonym"].is_string());
        assert_eq!(
            data["props"].get("fontItalic"),
            expected_flag.map(serde_json::Value::Bool).as_ref()
        );
        assert!(data["props"].get("vendor").is_none());
        let limits = data.get("limits").and_then(serde_json::Value::as_array);
        assert_eq!(
            limits.is_some_and(|v| v
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s.contains("fontItalic")))),
            limited,
            "{data}"
        );
    }
}

#[test]
fn styleitem_value_type_is_namespace_aware_and_damage_preserves_identity() {
    for (from, to, visible) in [
        ("v8ui", "aliased", true),
        ("http://v8.1c.ru/8.1/data/ui", "urn:vendor", false),
        ("xsi:type=\"v8ui:Font\"", "xsi:type=\"v8ui:Border\"", false),
        (
            "<Type>Font</Type>",
            "<Type>Font</Type><Type>Border</Type>",
            false,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        let configuration = fixture.source.join("Configuration.xml");
        let xml = fs::read_to_string(&configuration).unwrap();
        write(
            &configuration,
            &xml.replace(
                "</ChildObjects>",
                "<StyleItem>AbsoluteFont</StyleItem></ChildObjects>",
            ),
        );
        write(
            &fixture.source.join("StyleItems/AbsoluteFont.xml"),
            &fixture_text("styleitem_values/AbsoluteFont.xml").replace(from, to),
        );
        let result = fixture
            .view_service()
            .view(ViewRequest::new("main:StyleItem.AbsoluteFont").unwrap());
        assert!(result.ok, "the call failed");
        let data = result.data.as_ref().unwrap();
        assert_eq!(data["props"].get("fontHeight").is_some(), visible, "{data}");
        assert_eq!(data["props"]["kind"], "StyleItem");
        assert!(data["props"]["Synonym"].is_string());
        if !visible {
            assert!(!data["limits"].as_array().unwrap().is_empty());
        }
    }
}
