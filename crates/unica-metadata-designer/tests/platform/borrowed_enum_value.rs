use super::*;
use sha2::{Digest, Sha256};
pub(crate) const PARENT: &str = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:core="http://v8.1c.ru/8.1/data/core" version="2.20"><Enum uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Kinds</Name><Synonym/><Comment/></Properties><ChildObjects><EnumValue uuid="22222222-2222-4222-8222-222222222222"><Properties><Name>First</Name><Synonym><core:item><core:lang>ru</core:lang><core:content>Первое &amp; значение</core:content></core:item></Synonym><Comment>Источник</Comment></Properties></EnumValue><EnumValue uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Second</Name><Synonym/><Comment/></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#;
const VALUE_UUID: &str = "44444444-4444-4444-8444-444444444444";

fn shell() -> Vec<u8> {
    crate::borrowed_object::test_support::plan_borrow_object(
        PARENT.as_bytes(),
        None,
        None,
        fixture_uuid_v4,
    )
    .unwrap()
    .bytes
}

#[test]
fn enum_value_borrow_preserves_siblings_namespaces_and_noop_bytes() {
    let original = shell();
    let first = plan_borrow_enum_value(PARENT.as_bytes(), &original, "First", VALUE_UUID).unwrap();
    let second = plan_borrow_enum_value(
        PARENT.as_bytes(),
        &first,
        "Second",
        "55555555-5555-4555-8555-555555555555",
    )
    .unwrap();
    assert_eq!(
        plan_borrow_enum_value(
            PARENT.as_bytes(),
            &second,
            "First",
            "66666666-6666-4666-8666-666666666666"
        )
        .unwrap(),
        second
    );
    let xml_text = std::str::from_utf8(&second)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let doc = Document::parse(xml_text).unwrap();
    let values = child(owner(&doc).unwrap(), "ChildObjects")
        .unwrap()
        .children()
        .filter(|n| n.has_tag_name((MD, "EnumValue")))
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].attribute("uuid"), Some(VALUE_UUID));
    assert_eq!(
        text(
            child(values[0], "Properties").unwrap(),
            "ExtendedConfigurationObject"
        )
        .unwrap(),
        "22222222-2222-4222-8222-222222222222"
    );
    assert!(values[0].descendants().any(|n| n
        .has_tag_name(("http://v8.1c.ru/8.1/data/core", "content"))
        && n.text() == Some("Первое & значение")));

    let mut local = String::from_utf8(second.clone())
        .unwrap()
        .replace("Первое &amp; значение", "Локальный синоним");
    let comment_range = {
        let document = Document::parse(local.trim_start_matches('\u{feff}')).unwrap();
        let value = named_value(owner(&document).unwrap(), "First")
            .unwrap()
            .unwrap();
        child(child(value, "Properties").unwrap(), "Comment")
            .unwrap()
            .range()
    };
    local.replace_range(comment_range, "<Comment>Локальный комментарий</Comment>");
    let refreshed_parent = PARENT
        .replace("Первое &amp; значение", "Обновлённое значение")
        .replace(
            "<Comment>Источник</Comment>",
            "<Comment>Новый комментарий</Comment>",
        );
    let refreshed = plan_borrow_enum_value(
        refreshed_parent.as_bytes(),
        local.as_bytes(),
        "First",
        "77777777-7777-4777-8777-777777777777",
    )
    .unwrap();
    let refreshed_text = std::str::from_utf8(&refreshed)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let refreshed_doc = Document::parse(refreshed_text).unwrap();
    let refreshed_values = child(owner(&refreshed_doc).unwrap(), "ChildObjects")
        .unwrap()
        .children()
        .filter(|n| n.has_tag_name((MD, "EnumValue")))
        .collect::<Vec<_>>();
    assert_eq!(refreshed_values.len(), 2);
    assert_eq!(refreshed_values[0].attribute("uuid"), Some(VALUE_UUID));
    assert_eq!(
        text(child(refreshed_values[0], "Properties").unwrap(), "Comment").unwrap(),
        "Локальный комментарий"
    );
    assert!(refreshed_values[0].descendants().any(|n| n
        .has_tag_name(("http://v8.1c.ru/8.1/data/core", "content"))
        && n.text() == Some("Локальный синоним")));
    assert_eq!(
        refreshed_values[1].attribute("uuid"),
        Some("55555555-5555-4555-8555-555555555555")
    );
    assert_eq!(
        plan_borrow_enum_value(
            refreshed_parent.as_bytes(),
            &refreshed,
            "First",
            "88888888-8888-4888-8888-888888888888",
        )
        .unwrap(),
        refreshed
    );
}

#[test]
fn emitted_enum_value_matches_platform_8_3_27_export_byte_for_byte() {
    let expected = include_bytes!(
        "../../../../tests/fixtures/platform_8_3_27/cfe_borrow/extension-enum-value.xml"
    );
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/platform_8_3_27/cfe_borrow/manifest.json"
    ))
    .unwrap();
    let digest = Sha256::digest(expected)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(
        manifest["files"]["extension-enum-value.xml"]["sha256"].as_str(),
        Some(digest.as_str()),
        "платформенная фикстура расходится с закреплённым manifest"
    );
    let expected_text = std::str::from_utf8(expected).unwrap();
    let bomless = expected_text.trim_start_matches('\u{feff}');
    let document = Document::parse(bomless).unwrap();
    let children = child(owner(&document).unwrap(), "ChildObjects").unwrap();
    let mut shell = bomless.to_string();
    shell.replace_range(children.range(), "<ChildObjects/>");
    let mut shell_bytes = vec![0xef, 0xbb, 0xbf];
    shell_bytes.extend_from_slice(shell.as_bytes());
    let parent = r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:v8="http://v8.1c.ru/8.1/data/core" version="2.20"><Enum uuid="67e85b9a-a138-49ea-b474-04475f0f8c90"><Properties><Name>ВажностьПроблемыУчета</Name></Properties><ChildObjects><EnumValue uuid="5d990585-02dd-4a13-8dfa-920df7d1dbb0"><Properties><Name>Ошибка</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Ошибка</v8:content></v8:item></Synonym><Comment>Комментарий родителя</Comment></Properties></EnumValue></ChildObjects></Enum></MetaDataObject>"#;

    let actual = plan_borrow_enum_value(
        parent.as_bytes(),
        &shell_bytes,
        "Ошибка",
        "d358236d-65b3-8aa1-9d6f-61d4694dcda9",
    )
    .unwrap();

    assert_eq!(actual, expected);
}

#[test]
fn enum_value_borrow_uses_the_platform_slot_before_later_child_kinds() {
    let shell = String::from_utf8(shell()).unwrap();
    let with_command = shell.replace(
        "<ChildObjects/>",
        "<ChildObjects>\n\t\t\t<Command>LocalCommand</Command>\n\t\t</ChildObjects>",
    );

    let borrowed = plan_borrow_enum_value(
        PARENT.as_bytes(),
        with_command.as_bytes(),
        "First",
        VALUE_UUID,
    )
    .unwrap();
    let text = std::str::from_utf8(&borrowed)
        .unwrap()
        .trim_start_matches('\u{feff}');
    let document = Document::parse(text).unwrap();
    let tags = child(owner(&document).unwrap(), "ChildObjects")
        .unwrap()
        .children()
        .filter(|node| node.is_element())
        .map(|node| node.tag_name().name())
        .collect::<Vec<_>>();
    assert_eq!(tags, ["EnumValue", "Command"]);
    assert!(text.contains("<Command>LocalCommand</Command>"));
}

#[test]
fn enum_value_borrow_refuses_missing_duplicate_and_foreign_values() {
    let original = shell();
    let missing = enum_value_source_uuid(PARENT.as_bytes(), "Missing").unwrap_err();
    assert_eq!(missing.kind, EnumValueLookupErrorKind::NotFound);
    let damaged_owner = PARENT.replace("<ChildObjects>", "<MissingChildObjects>");
    let damaged = enum_value_source_uuid(damaged_owner.as_bytes(), "First").unwrap_err();
    assert_eq!(damaged.kind, EnumValueLookupErrorKind::InvalidSource);
    assert!(plan_borrow_enum_value(PARENT.as_bytes(), &original, "Missing", VALUE_UUID).is_err());
    let duplicate = PARENT.replace("<Name>Second</Name>", "<Name>First</Name>");
    assert!(plan_borrow_enum_value(duplicate.as_bytes(), &original, "First", VALUE_UUID).is_err());
    let first = plan_borrow_enum_value(PARENT.as_bytes(), &original, "First", VALUE_UUID).unwrap();
    let original_text = String::from_utf8(first.clone()).unwrap();
    let damaged = original_text.replace(
        &format!("uuid=\"{VALUE_UUID}\""),
        "uuid=\"22222222-2222-4222-8222-222222222222\"",
    );
    assert!(
        plan_borrow_enum_value(PARENT.as_bytes(), damaged.as_bytes(), "First", VALUE_UUID).is_err()
    );
    let own = original_text.replace(
        "<ObjectBelonging>Adopted</ObjectBelonging>",
        "<ObjectBelonging>Own</ObjectBelonging>",
    );
    assert!(
        plan_borrow_enum_value(PARENT.as_bytes(), own.as_bytes(), "First", VALUE_UUID).is_err()
    );
    let foreign = original_text.replace(
        "22222222-2222-4222-8222-222222222222",
        "99999999-9999-4999-8999-999999999999",
    );
    assert!(
        plan_borrow_enum_value(PARENT.as_bytes(), foreign.as_bytes(), "First", VALUE_UUID).is_err()
    );
    assert!(plan_borrow_enum_value(PARENT.as_bytes(), &first, "Second", VALUE_UUID).is_err());
    assert!(plan_borrow_enum_value(
        PARENT.as_bytes(),
        &original,
        "First",
        "22222222-2222-4222-8222-222222222222"
    )
    .is_err());
}

fn fixture_uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
