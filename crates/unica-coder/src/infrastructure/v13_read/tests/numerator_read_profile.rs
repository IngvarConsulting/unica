use super::*;

const OWNER: &str = "main:DocumentNumerator.Нумератор";

fn install(fixture: &RealReaderFixture, properties: &str) -> PathBuf {
    let configuration = fixture.source.join("Configuration.xml");
    write(
        &configuration,
        &fs::read_to_string(&configuration).unwrap().replace(
            "</ChildObjects>",
            "<DocumentNumerator>Нумератор</DocumentNumerator></ChildObjects>",
        ),
    );
    let path = fixture.source.join("DocumentNumerators/Нумератор.xml");
    write(
        &path,
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><DocumentNumerator uuid="16210000-0000-4000-8000-000000000001"><Properties><Name>Нумератор</Name>{properties}<NumberLength>11</NumberLength><FutureProperty>must not leak</FutureProperty></Properties></DocumentNumerator></MetaDataObject>"#
        ),
    );
    path
}

#[test]
fn numerator_read_profile_reaches_public_detail_with_typed_false_without_writes() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, "<NumberType>String</NumberType><NumberAllowedLength>Fixed</NumberAllowedLength><NumberPeriodicity>Year</NumberPeriodicity><CheckUnique>false</CheckUnique>");
    let before = fs::read(&path).unwrap();
    let configuration = fs::read(fixture.source.join("Configuration.xml")).unwrap();
    let service = fixture.view_service();
    let collection = service.view(ViewRequest::new("main:DocumentNumerator").unwrap());
    assert!(collection.ok);
    let at = collection.data.as_ref().unwrap()["items"][0]["at"]
        .as_str()
        .unwrap();
    assert_eq!(at, OWNER);
    let detail = service.view(ViewRequest::new(at).unwrap());
    assert!(detail.ok);
    let node = detail.data.as_ref().unwrap();
    assert_eq!(node["props"]["NumberType"], "String");
    assert_eq!(node["props"]["NumberAllowedLength"], "Fixed");
    assert_eq!(node["props"]["NumberPeriodicity"], "Year");
    assert_eq!(node["props"]["CheckUnique"], false);
    assert_eq!(node["props"]["kind"], "DocumentNumerator");
    assert!(node["props"].get("FutureProperty").is_none());
    assert!(node.get("limits").is_none());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(
        fs::read(fixture.source.join("Configuration.xml")).unwrap(),
        configuration
    );
}

#[test]
fn numerator_read_profile_preserves_absence_and_localizes_present_damage() {
    for (raw, damaged) in [
        ("", false),
        ("<NumberType/>", true),
        ("<NumberType>Unknown</NumberType>", true),
        ("<NumberType><Nested>String</Nested></NumberType>", true),
        (
            "<NumberType>String</NumberType><NumberType>Number</NumberType>",
            true,
        ),
        (
            "<NumberType xmlns=\"urn:foreign\">String</NumberType>",
            true,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, &format!("{raw}<CheckUnique>false</CheckUnique>"));
        let result = fixture
            .view_service()
            .view(ViewRequest::new(OWNER).unwrap());
        assert!(
            result.ok,
            "a damaged numbering field must retain independent facts"
        );
        let node = result.data.as_ref().unwrap();
        assert!(node["props"].get("NumberType").is_none());
        assert!(node["props"].get("NumberAllowedLength").is_none());
        assert!(node["props"].get("NumberPeriodicity").is_none());
        assert_eq!(node["props"]["CheckUnique"], false);
        assert_eq!(node["props"]["kind"], "DocumentNumerator");
        assert_eq!(
            node.get("limits")
                .is_some_and(|v| v.as_array().is_some_and(|items| items
                    .iter()
                    .any(|v| v.as_str().is_some_and(|s| s.contains("NumberType"))))),
            damaged
        );
    }
}

#[test]
fn numerator_read_profile_preserves_identity_when_the_new_frame_observation_fails() {
    let fixture = RealReaderFixture::new();
    let path = install(
        &fixture,
        "<NumberType>String</NumberType><CheckUnique>false</CheckUnique>",
    );
    write(
        &path,
        &fs::read_to_string(&path)
            .unwrap()
            .replace("</DocumentNumerator>", "<Properties/></DocumentNumerator>"),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(OWNER).unwrap());
    assert!(result.ok);
    let node = result.data.as_ref().unwrap();
    assert_eq!(node["props"]["kind"], "DocumentNumerator");
    assert!(node["props"].get("NumberType").is_none());
    assert!(node["props"].get("CheckUnique").is_none());
    assert!(node["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.contains("Properties"))));
}

#[test]
fn document_numerator_relation_reaches_the_registered_target_without_writes() {
    let fixture = RealReaderFixture::new();
    install(&fixture, "<NumberType>String</NumberType>");
    let path = fixture.source.join("Documents/Order.xml");
    write(
        &path,
        &fs::read_to_string(&path).unwrap().replace(
            "<Name>Order</Name>",
            "<Name>Order</Name><Numerator>DocumentNumerator.Нумератор</Numerator>",
        ),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(owner.ok);
    let owner = owner.data.unwrap();
    assert_eq!(owner["props"]["Numerator"], "DocumentNumerator.Нумератор");
    assert!(owner["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:Document.Order.Relation", "count":3})));
    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let mut request = ViewRequest::new("main:Document.Order.Relation")
            .unwrap()
            .with_limit(1)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let page = service.view(request);
        assert!(page.ok);
        items.extend(page.data.unwrap()["items"].as_array().unwrap().clone());
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(items.len(), 3);
    let expected = json!({"relation":"numerator", "at":OWNER, "kind":"DocumentNumerator"});
    assert_eq!(items.iter().filter(|item| **item == expected).count(), 1);
    assert!(
        service
            .view(ViewRequest::new(expected["at"].as_str().unwrap()).unwrap())
            .ok
    );
    assert!(
        !service
            .view(ViewRequest::new("main:Document.Order.Relation.Нумератор").unwrap())
            .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn document_numerator_relation_preserves_unknown_empty_and_damaged_observations() {
    for (raw, limited) in [
        ("", true),
        ("<Numerator/>", false),
        (
            "<Numerator>DocumentNumerator.Нумератор</Numerator><Numerator/>",
            true,
        ),
        (
            "<Numerator xmlns=\"urn:foreign\">DocumentNumerator.Нумератор</Numerator>",
            true,
        ),
        (
            "<Numerator><Nested>DocumentNumerator.Нумератор</Nested></Numerator>",
            true,
        ),
        ("<Numerator>Catalog.Items</Numerator>", true),
        (
            "<Numerator>DocumentNumerator.Нумератор.Extra.X</Numerator>",
            true,
        ),
        ("<Numerator>DocumentNumerator.Missing</Numerator>", true),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, "<NumberType>String</NumberType>");
        let path = fixture.source.join("Documents/Order.xml");
        write(
            &path,
            &fs::read_to_string(&path)
                .unwrap()
                .replace("<Name>Order</Name>", &format!("<Name>Order</Name>{raw}")),
        );
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        for at in ["main:Document.Order", "main:Document.Order.Relation"] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "{raw}: {:?}", refusal_codes(&result));
            let node = result.data.unwrap();
            let has_limit = node["limits"].as_array().is_some_and(|limits| {
                limits.iter().any(|limit| {
                    limit.as_str().is_some_and(|text| {
                        text.contains("numerator") || text.contains("Numerator")
                    })
                })
            });
            assert_eq!(has_limit, limited, "{raw}: {at}");
            if at.ends_with("Relation") {
                let items = node["items"].as_array().unwrap();
                assert_eq!(items.len(), 2);
                assert!(!items.iter().any(|item| item["relation"] == "numerator"));
            } else {
                assert_eq!(node["props"]["kind"], "Document");
            }
        }
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn document_numerator_relation_requires_local_registration_and_descriptor_identity() {
    for variant in ["unregistered", "missing", "foreign-name", "foreign-kind"] {
        let fixture = RealReaderFixture::new();
        let numerator = install(&fixture, "<NumberType>String</NumberType>");
        let path = fixture.source.join("Documents/Order.xml");
        write(
            &path,
            &fs::read_to_string(&path).unwrap().replace(
                "<Name>Order</Name>",
                "<Name>Order</Name><Numerator>DocumentNumerator.Нумератор</Numerator>",
            ),
        );
        match variant {
            "unregistered" => {
                let path = fixture.source.join("Configuration.xml");
                write(
                    &path,
                    &fs::read_to_string(&path)
                        .unwrap()
                        .replace("<DocumentNumerator>Нумератор</DocumentNumerator>", ""),
                );
            }
            "missing" => fs::remove_file(&numerator).unwrap(),
            "foreign-name" => write(
                &numerator,
                &fs::read_to_string(&numerator)
                    .unwrap()
                    .replace("<Name>Нумератор</Name>", "<Name>Other</Name>"),
            ),
            "foreign-kind" => write(
                &numerator,
                &fs::read_to_string(&numerator)
                    .unwrap()
                    .replace("DocumentNumerator", "Catalog"),
            ),
            _ => unreachable!(),
        }
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Document.Order.Relation").unwrap());
        assert!(result.ok, "{variant}: {:?}", refusal_codes(&result));
        let node = result.data.unwrap();
        assert_eq!(node["items"].as_array().unwrap().len(), 2, "{variant}");
        assert!(node["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("numerator")));
    }
}
