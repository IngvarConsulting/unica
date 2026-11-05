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
