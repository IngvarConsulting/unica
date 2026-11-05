use super::*;

const OWNERS: &[(&str, &str, &[&str])] = &[
    ("FilterCriterion", "FilterCriteria", &["Form", "Command"]),
    ("SettingsStorage", "SettingsStorages", &["Form", "Template"]),
    ("HTTPService", "HTTPServices", &["URLTemplate"]),
    ("WebService", "WebServices", &["Operation"]),
];

fn install_empty_owners(fixture: &RealReaderFixture) {
    let path = fixture.source.join("Configuration.xml");
    let mut xml = fs::read_to_string(&path).unwrap();
    for (index, (kind, directory, _)) in OWNERS.iter().enumerate() {
        xml = xml.replace(
            "</ChildObjects>",
            &format!("<{kind}>Empty</{kind}></ChildObjects>"),
        );
        write(&fixture.source.join(format!("{directory}/Empty.xml")), &format!("<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><{kind} uuid=\"22240000-0000-4000-8000-00000000000{index}\"><Properties><Name>Empty</Name></Properties><ChildObjects/></{kind}></MetaDataObject>"));
    }
    write(&path, &xml);
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn modeled_empty_owner_branches_are_advertised_and_readable_in_platform_order() {
    let fixture = RealReaderFixture::new();
    install_empty_owners(&fixture);
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    for (kind, _, children) in OWNERS {
        let at = format!("main:{kind}.Empty");
        let read = service.view(ViewRequest::new(&at).unwrap());
        assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
        let branches: Vec<_> = read.data.unwrap()["branches"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|branch| {
                let name = branch["at"].as_str().unwrap().rsplit('.').next().unwrap();
                ["Form", "Command", "Template", "URLTemplate", "Operation"].contains(&name)
            })
            .cloned()
            .collect();
        assert_eq!(
            branches,
            children
                .iter()
                .map(|child| json!({"at":format!("{at}.{child}"),"count":0}))
                .collect::<Vec<_>>(),
            "{at}"
        );
        for branch in branches {
            let at = branch["at"].as_str().unwrap();
            let read = service.view(ViewRequest::new(at).unwrap());
            assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
            assert_eq!(read.data.unwrap()["items"], json!([]), "{at}");
        }
    }
    let unrelated = service.view(ViewRequest::new("main:EventSubscription.ПриЗаписи").unwrap());
    assert!(unrelated.ok);
    assert!(unrelated.data.unwrap()["branches"]
        .as_array()
        .unwrap()
        .iter()
        .all(|branch| {
            !["Form", "Command", "Template", "URLTemplate", "Operation"]
                .iter()
                .any(|kind| {
                    branch["at"]
                        .as_str()
                        .unwrap()
                        .ends_with(&format!(".{kind}"))
                })
        }));
    for (owner, child) in [
        ("main:HTTPService.ExternalAPI", "URLTemplate"),
        ("main:WebService.Exchange", "Operation"),
    ] {
        let root = service.view(ViewRequest::new(owner).unwrap());
        assert!(root.ok);
        let branch_at = format!("{owner}.{child}");
        let collection = service.view(ViewRequest::new(&branch_at).unwrap());
        assert!(collection.ok, "{collection:?}");
        let items = collection.data.unwrap()["items"]
            .as_array()
            .unwrap()
            .clone();
        assert!(!items.is_empty(), "{branch_at}");
        assert!(root.data.unwrap()["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at":branch_at,"count":items.len()})));
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn damaged_service_children_keep_owner_facts_without_claiming_empty_branches() {
    for (kind, directory, name, child, field) in [
        (
            "HTTPService",
            "HTTPServices",
            "ExternalAPI",
            "URLTemplate",
            "details.urlTemplates",
        ),
        (
            "WebService",
            "WebServices",
            "Exchange",
            "Operation",
            "details.operations",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join(format!("{directory}/{name}.xml"));
        let xml = fs::read_to_string(&path).unwrap();
        let end = xml.rfind("</ChildObjects>").unwrap();
        write(&path, &format!("{}<{child}/>{}", &xml[..end], &xml[end..]));
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let at = format!("main:{kind}.{name}");
        let read = service.view(ViewRequest::new(&at).unwrap());
        assert!(read.ok, "{at}: {:?}", refusal_codes(&read));
        let data = read.data.unwrap();
        assert_eq!(data["props"]["kind"], kind);
        assert!(
            data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains(field)),
            "{data}"
        );
        assert!(
            data["branches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| branch["at"] != format!("{at}.{child}")),
            "{data}"
        );
        let dependent = service.view(ViewRequest::new(&format!("{at}.{child}")).unwrap());
        assert!(!dependent.ok);
        assert_eq!(dependent.diagnostics[0]["detailCode"], "source_unreadable");
        assert!(dependent.data.is_none());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn unnamed_form_command_and_template_entries_are_not_reported_as_empty() {
    for (kind, directory, child, field, sibling) in [
        (
            "FilterCriterion",
            "FilterCriteria",
            "Form",
            "collections.forms",
            "Command",
        ),
        (
            "FilterCriterion",
            "FilterCriteria",
            "Command",
            "collections.commands",
            "Form",
        ),
        (
            "SettingsStorage",
            "SettingsStorages",
            "Form",
            "collections.forms",
            "Template",
        ),
        (
            "SettingsStorage",
            "SettingsStorages",
            "Template",
            "collections.templates",
            "Form",
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install_empty_owners(&fixture);
        let path = fixture.source.join(format!("{directory}/Empty.xml"));
        let xml = fs::read_to_string(&path).unwrap().replace(
            "<ChildObjects/>",
            &format!("<ChildObjects><{child}/></ChildObjects>"),
        );
        write(&path, &xml);
        let service = fixture.view_service();
        let at = format!("main:{kind}.Empty");
        let root = service.view(ViewRequest::new(&at).unwrap());
        assert!(root.ok, "{root:?}");
        let data = root.data.unwrap();
        assert!(
            data["branches"]
                .as_array()
                .unwrap()
                .iter()
                .all(|branch| branch["at"] != format!("{at}.{child}")),
            "damaged {child}: {data}"
        );
        assert!(
            data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains(field)),
            "{data}"
        );
        let damaged = service.view(ViewRequest::new(&format!("{at}.{child}")).unwrap());
        assert!(!damaged.ok, "{damaged:?}");
        assert_eq!(damaged.diagnostics[0]["detailCode"], "source_unreadable");
        let independent = service.view(ViewRequest::new(&format!("{at}.{sibling}")).unwrap());
        assert!(independent.ok, "{independent:?}");
        assert_eq!(independent.data.unwrap()["items"], json!([]));
    }
}

/// rule: CTR.SOURCE.EMPTY-BRANCHES
#[test]
fn incomplete_registrations_keep_known_rows_and_independent_empty_siblings() {
    for (fragment, known) in [
        (
            "<ChildObjects><Form>Known</Form><Form/></ChildObjects>",
            true,
        ),
        (
            "<ChildObjects><Form xmlns=\"urn:foreign\">Foreign</Form></ChildObjects>",
            false,
        ),
        (
            "<ChildObjects xmlns=\"urn:foreign\"><Form>Foreign</Form></ChildObjects>",
            false,
        ),
    ] {
        let fixture = RealReaderFixture::new();
        install_empty_owners(&fixture);
        let path = fixture.source.join("SettingsStorages/Empty.xml");
        let xml = fs::read_to_string(&path)
            .unwrap()
            .replace("<ChildObjects/>", fragment);
        write(&path, &xml);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let root = service.view(ViewRequest::new("main:SettingsStorage.Empty").unwrap());
        assert!(root.ok, "{root:?}");
        let data = root.data.unwrap();
        assert!(
            data["limits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|limit| limit.as_str().unwrap().contains("collections.forms")),
            "{data}"
        );
        assert_eq!(
            data["branches"]
                .as_array()
                .unwrap()
                .iter()
                .find(|branch| branch["at"] == "main:SettingsStorage.Empty.Form"),
            known
                .then(|| json!({"at":"main:SettingsStorage.Empty.Form","count":1}))
                .as_ref()
        );
        let forms = service.view(ViewRequest::new("main:SettingsStorage.Empty.Form").unwrap());
        if known {
            assert!(forms.ok, "{forms:?}");
            let data = forms.data.unwrap();
            assert_eq!(data["items"].as_array().unwrap().len(), 1);
            assert_eq!(
                data["items"][0]["at"],
                "main:SettingsStorage.Empty.Form.Known"
            );
            assert!(
                data["limits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|limit| limit.as_str().unwrap().contains("collections.forms")),
                "{data}"
            );
        } else {
            assert!(!forms.ok, "{forms:?}");
            assert_eq!(forms.diagnostics[0]["detailCode"], "source_unreadable");
        }
        let templates =
            service.view(ViewRequest::new("main:SettingsStorage.Empty.Template").unwrap());
        assert!(templates.ok, "{templates:?}");
        let data = templates.data.unwrap();
        assert_eq!(data["items"], json!([]));
        assert!(
            data.get("limits")
                .is_none_or(|limits| limits.as_array().is_some_and(Vec::is_empty)),
            "{data}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
