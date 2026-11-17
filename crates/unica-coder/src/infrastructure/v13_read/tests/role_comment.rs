use super::*;

const ROLE: &str = "main:Role.SalesReader";

fn install_comment(fixture: &RealReaderFixture, comment: &str, prefixed: bool) -> PathBuf {
    let prefix = if prefixed { "md:" } else { "" };
    let path = fixture.source.join("Roles/SalesReader.xml");
    write(
        &path,
        &format!(
            r#"<{}MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:md="http://v8.1c.ru/8.3/MDClasses" version="2.20"><{prefix}Role uuid="00000000-0000-0000-0000-000000000051"><{prefix}Properties><{prefix}Name>SalesReader</{prefix}Name><{prefix}Synonym/>{comment}</{prefix}Properties></{prefix}Role></{prefix}MetaDataObject>"#,
            prefix
        ),
    );
    path
}

fn has_comment_limit(data: &serde_json::Value) -> bool {
    data.get("limits").is_some_and(|limits| {
        limits
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().is_some_and(|text| text.contains("Comment")))
    })
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
#[test]
fn role_comment_reads_the_descriptor_and_preserves_rights_and_source_bytes() {
    for prefixed in [false, true] {
        for (fragment, expected) in [
            ("<Comment>Назначение роли</Comment>", "Назначение роли"),
            (
                "<md:Comment>  &amp; &lt;текст&gt;<![CDATA[.]]>\n</md:Comment>",
                "  & <текст>.\n",
            ),
            ("<Comment/>", ""),
            ("<Comment></Comment>", ""),
            ("<Comment>  </Comment>", "  "),
        ] {
            let fixture = RealReaderFixture::new();
            let path = install_comment(&fixture, fragment, prefixed);
            let descriptor_before = fs::read(&path).unwrap();
            let rights_path = fixture.source.join("Roles/SalesReader/Ext/Rights.xml");
            let rights_before = fs::read(&rights_path).unwrap();
            let service = fixture.view_service();
            let result = service.view(ViewRequest::new(ROLE).unwrap());
            assert!(
                result.ok,
                "the role must be readable: {:?}",
                result.diagnostics
            );
            let data = result.data.unwrap();
            assert_eq!(data["props"]["Comment"], expected);
            assert!(data["props"]["totalallowed"].is_number());
            assert!(data["props"]["totaldenied"].is_number());
            assert!(!has_comment_limit(&data), "{data}");
            let right =
                service.view(ViewRequest::new(&format!("{ROLE}.Right.Catalog_Products")).unwrap());
            assert!(right.ok);
            let right = right.data.unwrap();
            assert_eq!(right["props"]["allowedCount"], 2);
            assert_eq!(right["props"]["deniedCount"], 1);
            assert_eq!(
                right["branches"][0]["at"],
                format!("{ROLE}.Right.Catalog_Products.RLS")
            );
            assert_eq!(fs::read(&path).unwrap(), descriptor_before);
            assert_eq!(fs::read(&rights_path).unwrap(), rights_before);
        }
    }
}

/// rule: INV.SOURCE.METADATA-OBJECT-FACTS
#[test]
fn role_comment_separates_absence_from_local_damage() {
    for (fragment, damaged) in [
        ("", false),
        ("<Comment/><Comment>second</Comment>", true),
        ("<Comment xmlns=\"urn:foreign\">foreign</Comment>", true),
        ("<Comment attr=\"x\"/>", true),
        ("<Comment><Nested>nested</Nested></Comment>", true),
    ] {
        let fixture = RealReaderFixture::new();
        install_comment(&fixture, fragment, false);
        let result = fixture.view_service().view(ViewRequest::new(ROLE).unwrap());
        assert!(result.ok, "one unreadable Comment must not hide the role");
        let data = result.data.unwrap();
        assert!(data["props"].get("Comment").is_none());
        assert!(data["props"]["totalallowed"].is_number());
        assert_eq!(has_comment_limit(&data), damaged, "{fragment}: {data}");
    }
}

/// rule: INV.SOURCE.METADATA-NODE-PROPERTIES
#[test]
fn role_comment_obeys_the_byte_budget_and_is_readable_whole() {
    for size in [1024, 1025] {
        let fixture = RealReaderFixture::new();
        let text = "Я".repeat(size);
        install_comment(&fixture, &format!("<Comment>{text}</Comment>"), false);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new(ROLE).unwrap());
        assert!(result.ok);
        let data = result.data.unwrap();
        if size == 1024 {
            assert_eq!(data["props"]["Comment"], text);
        } else {
            assert!(data["props"].get("Comment").is_none());
            assert!(data["limits"].as_array().unwrap().iter().any(|limit| {
                limit
                    .as_str()
                    .is_some_and(|text| text.contains(".Props.Comment"))
            }));
        }
        let whole = crate::infrastructure::v13_read_projection::with_whole_prop("Comment", || {
            service.view(ViewRequest::new(ROLE).unwrap())
        });
        assert!(whole.ok);
        assert_eq!(whole.data.unwrap()["props"]["Comment"], text);
    }
}

#[test]
fn role_comment_requires_the_registered_role_and_its_descriptor_identity() {
    for damage in ["registration", "namespace", "name"] {
        let fixture = RealReaderFixture::new();
        let path = install_comment(&fixture, "<Comment>Комментарий</Comment>", false);
        let original = fs::read_to_string(&path).unwrap();
        match damage {
            "registration" => {
                let path = fixture.source.join("Configuration.xml");
                write(
                    &path,
                    &fs::read_to_string(&path)
                        .unwrap()
                        .replace("<Role>SalesReader</Role>", ""),
                );
            }
            "namespace" => write(
                &path,
                &original.replace("http://v8.1c.ru/8.3/MDClasses", "urn:foreign"),
            ),
            "name" => write(
                &path,
                &original.replace("<Name>SalesReader</Name>", "<Name>Other</Name>"),
            ),
            _ => unreachable!(),
        }
        let result = fixture.view_service().view(ViewRequest::new(ROLE).unwrap());
        assert!(
            !result.ok,
            "{damage} must not be admitted for Comment reading"
        );
    }
}
