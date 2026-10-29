use super::*;

const OWNER: &str = "main:SessionParameter.Параметр";

fn install(fixture: &RealReaderFixture, comment: &str) -> PathBuf {
    let config = fixture.source.join("Configuration.xml");
    write(
        &config,
        &fs::read_to_string(&config).unwrap().replace(
            "</ChildObjects>",
            "<SessionParameter>Параметр</SessionParameter></ChildObjects>",
        ),
    );
    let path = fixture.source.join("SessionParameters/Параметр.xml");
    write(
        &path,
        &format!(
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><SessionParameter uuid="16280000-0000-4000-8000-000000000001"><Properties><Name>Параметр</Name>{comment}<FutureProperty>must not leak</FutureProperty></Properties></SessionParameter></MetaDataObject>"#
        ),
    );
    path
}

#[test]
fn session_parameter_comment_reaches_public_detail_exactly_without_writes() {
    for (raw, expected) in [
        (
            "<Comment>Короткий комментарий.</Comment>",
            "Короткий комментарий.",
        ),
        (
            "<Comment>  Первая &amp; вторая\n<![CDATA[Следующая строка.]]>\n</Comment>",
            "  Первая & вторая\nСледующая строка.\n",
        ),
        ("<Comment/>", ""),
    ] {
        let fixture = RealReaderFixture::new();
        let path = install(&fixture, raw);
        let before = fs::read(&path).unwrap();
        let config = fs::read(fixture.source.join("Configuration.xml")).unwrap();
        let service = fixture.view_service();
        let collection = service.view(ViewRequest::new("main:SessionParameter").unwrap());
        assert!(collection.ok);
        let at = collection.data.as_ref().unwrap()["items"][0]["at"]
            .as_str()
            .unwrap();
        assert_eq!(at, OWNER);
        let detail = service.view(ViewRequest::new(at).unwrap());
        assert!(detail.ok, "the SessionParameter must remain readable");
        let node = detail.data.as_ref().unwrap();
        assert_eq!(node["props"]["Comment"], expected);
        assert_eq!(node["props"]["kind"], "SessionParameter");
        assert_eq!(node["title"], "Параметр");
        assert!(node["props"].get("FutureProperty").is_none());
        assert!(node.get("limits").is_none());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(
            fs::read(fixture.source.join("Configuration.xml")).unwrap(),
            config
        );
    }
}

#[test]
fn session_parameter_comment_distinguishes_absence_from_local_damage() {
    for (raw, damaged) in [
        ("", false),
        ("<Comment><Nested>bad</Nested></Comment>", true),
        ("<Comment>One</Comment><Comment>Two</Comment>", true),
        (
            "<Comment xmlns=\"urn:foreign\">wrong namespace</Comment>",
            true,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install(&fixture, raw);
        let result = fixture
            .view_service()
            .view(ViewRequest::new(OWNER).unwrap());
        assert!(
            result.ok,
            "one unreadable property must retain independent identity"
        );
        let node = result.data.as_ref().unwrap();
        assert!(node["props"].get("Comment").is_none());
        assert_eq!(node["props"]["kind"], "SessionParameter");
        assert_eq!(node["at"], OWNER);
        assert_eq!(
            node.get("limits")
                .is_some_and(|v| v.as_array().is_some_and(|items| items
                    .iter()
                    .any(|v| v.as_str().is_some_and(|s| s.contains("Comment"))))),
            damaged
        );
    }
}

#[test]
fn session_parameter_comment_uses_the_same_value_for_compact_and_whole_views() {
    for size in [2048, 2049] {
        let fixture = RealReaderFixture::new();
        let text = "x".repeat(size);
        install(&fixture, &format!("<Comment>{text}</Comment>"));
        let service = fixture.view_service();
        let compact = service.view(ViewRequest::new(OWNER).unwrap());
        assert!(compact.ok);
        let node = compact.data.as_ref().unwrap();
        if size == 2048 {
            assert_eq!(node["props"]["Comment"], text);
        } else {
            assert!(node["props"].get("Comment").is_none());
            assert!(node["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str().is_some_and(|s| s.contains(".Props.Comment"))));
        }
        let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Comment", || {
            service.view(ViewRequest::new(OWNER).unwrap())
        });
        assert!(whole.ok);
        assert_eq!(whole.data.as_ref().unwrap()["props"]["Comment"], text);
    }
}

#[test]
fn session_parameter_comment_does_not_hide_identity_when_properties_are_ambiguous() {
    let fixture = RealReaderFixture::new();
    let path = install(&fixture, "<Comment>Known</Comment>");
    write(
        &path,
        &fs::read_to_string(&path)
            .unwrap()
            .replace("</SessionParameter>", "<Properties/></SessionParameter>"),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new(OWNER).unwrap());
    assert!(result.ok);
    let node = result.data.as_ref().unwrap();
    assert_eq!(node["props"]["kind"], "SessionParameter");
    assert!(node["props"].get("Comment").is_none());
    assert!(node["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v.as_str().is_some_and(|s| s.contains("Comment"))));
}
