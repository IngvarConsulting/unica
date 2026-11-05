use super::*;
fn descriptor(value: &str, bindings: &str) -> String {
    format!("<MetaDataObject xmlns='{MD}' xmlns:ui='{UI}' xmlns:xsi='{XSI}' {bindings} version='2.20'><StyleItem uuid='1de549ac-39a2-49ce-9f31-c13040173894'><Properties><Name>Accent</Name><Type>Color</Type>{value}</Properties></StyleItem></MetaDataObject>")
}
#[test]
fn value_alias_and_hex_case_are_semantic_noops() {
    for (value, bindings, desired) in [
        (
            "<Value xsi:type='ui:Color'>#abCdEF</Value>",
            String::new(),
            "#ABCDEF",
        ),
        (
            "<Value xsi:type='ui:Color'>c:Gray</Value>",
            format!("xmlns:c='{WEB}'"),
            "web:Gray",
        ),
    ] {
        let xml = descriptor(value, &bindings);
        let image =
            StyleColorImage::parse(&xml, "Accent", Path::new("StyleItems/Accent.xml")).unwrap();
        assert_eq!(
            image.replace(StyleItemColor::parse_argument(desired).unwrap()),
            xml
        );
    }
}
#[test]
fn type_prefix_collision_is_resolved_inside_value_only() {
    let xml = descriptor(
        "<Value xsi:type='web:Color' marker='>'>#123456</Value>",
        &format!("xmlns:web='{UI}'"),
    );
    let image = StyleColorImage::parse(&xml, "Accent", Path::new("StyleItems/Accent.xml")).unwrap();
    let updated = image.replace(StyleItemColor::parse_argument("web:Gray").unwrap());
    let expected = xml.replace(
        " marker='>'>#123456",
        &format!(" marker='>' xmlns:web1=\"{WEB}\">web1:Gray"),
    );
    assert_eq!(updated, expected);
    let image =
        StyleColorImage::parse(&updated, "Accent", Path::new("StyleItems/Accent.xml")).unwrap();
    assert_eq!(
        image.color(),
        StyleItemColor::parse_argument("web:Gray").unwrap()
    );
}
#[test]
fn opaque_forms_are_distinct_from_damaged_values() {
    for (text, binding, read_only) in [
        (
            "win:ButtonFace",
            format!("xmlns:win='{UI}/colors/windows'"),
            true,
        ),
        ("custom:Color", "xmlns:custom='urn:future'".into(), true),
        ("style:MyColor", format!("xmlns:style='{STYLE}'"), true),
        ("missing:Gray", String::new(), false),
        ("#GG0000", String::new(), false),
    ] {
        let xml = descriptor(
            &format!("<Value xsi:type='ui:Color'>{text}</Value>"),
            &binding,
        );
        let error = StyleColorImage::parse(&xml, "Accent", Path::new("StyleItems/Accent.xml"))
            .err()
            .unwrap();
        assert_eq!(
            matches!(error, ColorIssue::ReadOnly(_)),
            read_only,
            "{error}"
        );
    }
}

#[test]
fn malformed_target_fields_and_literal_versions_are_not_writable() {
    let base = descriptor("<Value xsi:type='ui:Color'>#123456</Value>", "");
    for (from, to) in [
        (
            "<Name>Accent</Name>",
            "<Name>Accent</Name><Name>Accent</Name>",
        ),
        ("<Name>Accent</Name>", "<Name xsi:nil='true'>Accent</Name>"),
        ("<Type>Color</Type>", "<Type>Color</Type><Type>Color</Type>"),
        ("<Type>Color</Type>", "<Type xsi:nil='true'>Color</Type>"),
        ("<Value xsi:type='ui:Color'>#123456</Value>", ""),
        (
            "<Value xsi:type='ui:Color'>#123456</Value>",
            "<Value xsi:type='ui:Color'>#123456</Value><Value xsi:type='ui:Color'>#123456</Value>",
        ),
        (
            "<Value xsi:type='ui:Color'>#123456</Value>",
            "<Value xmlns='urn:foreign' xsi:type='ui:Color'>#123456</Value>",
        ),
        (
            "<Value xsi:type='ui:Color'>#123456</Value>",
            "<Value xsi:type='ui:Color'><Nested/></Value>",
        ),
        (
            "<Value xsi:type='ui:Color'>#123456</Value>",
            "<Value xsi:type='ui:Color' xsi:nil='true'/>",
        ),
        (
            "uuid='1de549ac-39a2-49ce-9f31-c13040173894'",
            "uuid='broken'",
        ),
        ("version='2.20'", "version='2.19'"),
        ("version='2.20'", "version='2.&#50;0'"),
    ] {
        let xml = base.replace(from, to);
        assert!(
            matches!(
                StyleColorImage::parse(&xml, "Accent", Path::new("StyleItems/Accent.xml")),
                Err(ColorIssue::Invalid(_))
            ),
            "{xml}"
        );
    }
}

#[test]
fn default_color_qname_and_comment_noop_preserve_exact_source() {
    let xml = descriptor(
        &format!(
            "<m:Value xmlns:m='{MD}' xmlns='{WEB}' xsi:type=' ui:Color '>Gr<!--keep-->ay</m:Value>"
        ),
        "",
    );
    let image = StyleColorImage::parse(&xml, "Accent", Path::new("StyleItems/Accent.xml")).unwrap();
    assert_eq!(
        image.replace(StyleItemColor::parse_argument("web:Gray").unwrap()),
        xml
    );
    let updated = image.replace(StyleItemColor::parse_argument("style:SpecialTextColor").unwrap());
    let image =
        StyleColorImage::parse(&updated, "Accent", Path::new("StyleItems/Accent.xml")).unwrap();
    assert_eq!(
        image.color(),
        StyleItemColor::parse_argument("style:SpecialTextColor").unwrap()
    );
    assert!(updated.contains("</m:Value>"));
}
