use super::*;

const MEMBERS: [(&str, &str); 4] = [
    (
        "HTTPServices/ExternalAPI.xml",
        "main:HTTPService.ExternalAPI.URLTemplate.Metrics",
    ),
    (
        "HTTPServices/ExternalAPI.xml",
        "main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get",
    ),
    (
        "WebServices/Exchange.xml",
        "main:WebService.Exchange.Operation.Send",
    ),
    (
        "WebServices/Exchange.xml",
        "main:WebService.Exchange.Operation.Send.Parameter.Payload",
    ),
];

fn install(fixture: &RealReaderFixture) {
    for (file, names) in [
        ("HTTPServices/ExternalAPI.xml", ["Metrics", "Get"]),
        ("WebServices/Exchange.xml", ["Send", "Payload"]),
    ] {
        let path = fixture.source.join(file);
        let mut xml = fs::read_to_string(&path).unwrap().replace(
            "<MetaDataObject ",
            "<MetaDataObject xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" ",
        );
        for name in names {
            xml=xml.replace(&format!("<Name>{name}</Name>"),&format!("<Name>{name}</Name><Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Русский {name}</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>English {name}</v8:content></v8:item></Synonym><Comment>{}</Comment>{}",if name=="Send" {"Upload comment"} else {""},if name=="Send" {"<DataLockControlMode>Managed</DataLockControlMode>"} else {""}));
        }
        write(&path, &xml);
    }
}

#[test]
fn service_member_nine_facts_reach_collections_and_details_without_changing_technical_fields() {
    let fixture = RealReaderFixture::new();
    install(&fixture);
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    for (_, at) in MEMBERS {
        let name = at.rsplit('.').next().unwrap();
        let collection = at.rsplit_once('.').unwrap().0;
        let list = service.view(ViewRequest::new(collection).unwrap());
        assert!(list.ok, "{:?}", refusal_codes(&list));
        let row = list.data.unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["at"] == at)
            .unwrap()
            .clone();
        let detail = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
        assert!(detail.ok, "{:?}", refusal_codes(&detail));
        for node in [row, detail.data.unwrap()] {
            assert_eq!(
                node["props"]["Synonym"],
                json!([{"language":"ru","content":format!("Русский {name}")},{"language":"en","content":format!("English {name}")}]),
                "{node:#}"
            );
            assert_eq!(
                node["props"]["Comment"],
                if name == "Send" {
                    json!("Upload comment")
                } else {
                    json!("")
                }
            );
            match name {
                "Metrics" => assert_eq!(node["props"]["template"], "/v1/kpi/"),
                "Get" => assert_eq!(node["props"]["handler"], "MetricsGet"),
                "Send" => {
                    assert_eq!(node["props"]["DataLockControlMode"], "Managed");
                    assert_eq!(node["props"]["procedure"], "SendData");
                    assert_eq!(node["props"]["nillable"], false);
                }
                "Payload" => assert_eq!(
                    node["props"]["type"],
                    json!({"namespace":"urn:example:service","localName":"Payload"})
                ),
                _ => unreachable!(),
            }
        }
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

fn change(fixture: &RealReaderFixture, index: usize, mutate: impl FnOnce(&str) -> String) {
    let (file, at) = MEMBERS[index];
    let name = at.rsplit('.').next().unwrap();
    let path = fixture.source.join(file);
    let xml = fs::read_to_string(&path).unwrap();
    let start = xml.find(&format!("<Name>{name}</Name>")).unwrap();
    let end = start + xml[start..].find("</Properties>").unwrap();
    write(
        &path,
        &format!(
            "{}{}{}",
            &xml[..start],
            mutate(&xml[start..end]),
            &xml[end..]
        ),
    );
}

#[test]
fn service_member_damage_limits_only_the_named_fact_and_keeps_owner_diagnostics() {
    for (index, &(_, at)) in MEMBERS.iter().enumerate() {
        for variant in [
            "nested-comment",
            "duplicate-comment",
            "foreign-comment",
            "nested-synonym",
            "duplicate-synonym",
            "foreign-synonym",
        ] {
            let fixture = RealReaderFixture::new();
            install(&fixture);
            let bad = if variant.ends_with("comment") {
                "Comment"
            } else {
                "Synonym"
            };
            change(&fixture, index, |xml| match variant {
                "nested-comment" => xml.replace("<Comment>", "<Comment><Hidden/>"),
                "duplicate-comment" => format!("{xml}<Comment>duplicate</Comment>"),
                "foreign-comment" => xml.replace("<Comment>", "<Comment xmlns=\"urn:foreign\">"),
                "nested-synonym" => xml.replacen("<v8:content>", "<v8:content><Hidden/>", 1),
                "duplicate-synonym" => format!("{xml}<Synonym/>"),
                "foreign-synonym" => xml.replace("<Synonym>", "<Synonym xmlns=\"urn:foreign\">"),
                _ => unreachable!(),
            });
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let service = fixture.view_service();

            let owner = at.split('.').take(2).collect::<Vec<_>>().join(".");
            let root = service.view(ViewRequest::new(&owner).unwrap());
            assert!(root.ok);
            assert!(
                root.data.unwrap()["limits"]
                    .as_array()
                    .is_some_and(|limits| limits
                        .iter()
                        .any(|l| l.as_str().is_some_and(|s| s.contains(&bad.to_lowercase())))),
                "{variant}"
            );
            let list = service.view(ViewRequest::new(at.rsplit_once('.').unwrap().0).unwrap());
            assert!(list.ok, "{:?}", refusal_codes(&list));
            let row = list.data.unwrap()["items"][0].clone();
            let detail = service.view(ViewRequest::new(row["at"].as_str().unwrap()).unwrap());
            assert!(detail.ok);
            for node in [row, detail.data.unwrap()] {
                assert!(node["props"].get(bad).is_none(), "{node:#}");
                let good = if bad == "Comment" {
                    "Synonym"
                } else {
                    "Comment"
                };
                assert!(node["props"].get(good).is_some(), "{node:#}");
                assert!(
                    node["limits"].as_array().is_some_and(|l| !l.is_empty()),
                    "{node:#}"
                );
            }
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
}

#[test]
fn service_member_compact_values_and_props_keep_full_multilingual_and_text_values() {
    for (index, &(_, at)) in MEMBERS.iter().enumerate() {
        let fixture = RealReaderFixture::new();
        install(&fixture);
        let long = "я".repeat(1100);
        change(&fixture, index, |xml| {
            let start = xml.find("<Comment>").unwrap();
            let end = xml.find("</Comment>").unwrap();
            format!(
                "{}<Comment>{long}</Comment>{}",
                &xml[..start],
                &xml[end + 10..]
            )
            .replace("English ", &long)
        });
        let service = fixture.view_service();

        let list = service.view(ViewRequest::new(at.rsplit_once('.').unwrap().0).unwrap());
        assert!(list.ok);
        let row = list.data.unwrap()["items"][0].clone();
        let detail = service.view(ViewRequest::new(at).unwrap());
        assert!(detail.ok);
        for node in [row, detail.data.unwrap()] {
            for key in ["Synonym", "Comment"] {
                assert!(node["props"].get(key).is_none(), "{node:#}");
                assert!(node["limits"].as_array().unwrap().iter().any(|l| l
                    .as_str()
                    .is_some_and(|s| s.contains(&format!("{at}.Props.{key}")))));
            }
        }
        let comment =
            crate::infrastructure::v13_read_projection::with_whole_prop("Comment", || {
                service.view(ViewRequest::new(at).unwrap())
            });
        assert!(comment.ok, "{:?}", refusal_codes(&comment));
        assert_eq!(comment.data.unwrap()["props"]["Comment"], long);
        let synonym =
            crate::infrastructure::v13_read_projection::with_whole_prop("Synonym", || {
                service.view(ViewRequest::new(at).unwrap())
            });
        assert!(synonym.ok, "{:?}", refusal_codes(&synonym));
        assert_eq!(
            synonym.data.unwrap()["props"]["Synonym"][1]["content"],
            format!("{long}{}", at.rsplit('.').next().unwrap())
        );
    }
}

#[test]
fn service_operation_keeps_two_diagnostics_parameters_and_false_flags() {
    let fixture = RealReaderFixture::new();
    install(&fixture);
    change(&fixture, 2, |xml| {
        xml.replace("<Comment>", "<Comment><Hidden/>")
            .replace(
                "<DataLockControlMode>Managed</DataLockControlMode>",
                "<DataLockControlMode>AutomaticAndManaged</DataLockControlMode>",
            )
            .replace(
                "<Transactioned>true</Transactioned>",
                "<Transactioned>false</Transactioned>",
            )
    });
    let service = fixture.view_service();
    let root = service.view(ViewRequest::new("main:WebService.Exchange").unwrap());
    assert!(root.ok);
    let root = root.data.unwrap();
    for field in [
        "details.operations[0].comment",
        "details.operations[0].dataLockControlMode",
    ] {
        assert!(
            root["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l.as_str().is_some_and(|s| s.contains(field))),
            "{root:#}"
        );
    }
    let operation = service.view(ViewRequest::new(MEMBERS[2].1).unwrap());
    assert!(operation.ok);
    let node = operation.data.unwrap();
    assert_eq!(node["props"]["transactioned"], false);
    assert_eq!(node["props"]["nillable"], false);
    assert!(node["props"].get("Comment").is_none());
    assert!(node["props"].get("DataLockControlMode").is_none());
    assert!(node["props"]["Synonym"].is_array());
    assert_eq!(node["limits"].as_array().unwrap().len(), 2);
    assert_eq!(node["branches"][0]["count"], 1);
    let parameters =
        service.view(ViewRequest::new(node["branches"][0]["at"].as_str().unwrap()).unwrap());
    assert!(parameters.ok);
    assert_eq!(
        parameters.data.unwrap()["items"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn service_member_optional_absence_and_empty_translation_do_not_create_defaults_or_damage() {
    for (index, &(_, at)) in MEMBERS.iter().enumerate() {
        for (replacement, expected) in [
            ("", None),
            ("<Synonym/>", None),
            (
                "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content/></v8:item></Synonym>",
                Some(json!([{"language":"ru","content":""}])),
            ),
        ] {
            let fixture = RealReaderFixture::new();
            install(&fixture);
            change(&fixture, index, |xml| {
                let start = xml.find("<Synonym>").unwrap();
                let end = xml.find("</Synonym>").unwrap() + 10;
                format!("{}{replacement}{}", &xml[..start], &xml[end..])
                    .replace("<Comment></Comment>", "")
                    .replace("<Comment>Upload comment</Comment>", "")
            });
            let service = fixture.view_service();
            let node = service.view(ViewRequest::new(at).unwrap());
            assert!(node.ok);
            let node = node.data.unwrap();
            if let Some(value) = expected {
                assert_eq!(node["props"]["Synonym"], value);
            } else {
                assert!(node["props"].get("Synonym").is_none());
            }
            assert!(node["props"].get("Comment").is_none());
            assert!(node.get("limits").is_none(), "{node:#}");
        }
    }
}

#[test]
fn service_member_comment_compact_limit_uses_the_exact_byte_boundary() {
    for (index, &(_, at)) in MEMBERS.iter().enumerate() {
        for size in [2048, 2049] {
            let fixture = RealReaderFixture::new();
            install(&fixture);
            let text = "x".repeat(size);
            change(&fixture, index, |xml| {
                let start = xml.find("<Comment>").unwrap();
                let end = xml.find("</Comment>").unwrap() + 10;
                format!("{}<Comment>{text}</Comment>{}", &xml[..start], &xml[end..])
            });
            let service = fixture.view_service();
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok);
            let node = result.data.unwrap();
            if size == 2048 {
                assert_eq!(node["props"]["Comment"], text);
            } else {
                assert!(node["props"].get("Comment").is_none());
            }
        }
    }
}
