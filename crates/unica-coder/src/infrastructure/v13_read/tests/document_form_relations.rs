use super::*;

const SLOTS: [(&str, &str); 6] = [
    ("DefaultObjectForm", "defaultObjectForm"),
    ("DefaultListForm", "defaultListForm"),
    ("DefaultChoiceForm", "defaultChoiceForm"),
    ("AuxiliaryObjectForm", "auxiliaryObjectForm"),
    ("AuxiliaryListForm", "auxiliaryListForm"),
    ("AuxiliaryChoiceForm", "auxiliaryChoiceForm"),
];
const TARGET: &str = "Catalog.Items.Form.ItemForm";

fn install(fixture: &RealReaderFixture, properties: &str) {
    let path = fixture.source.join("Documents/Order.xml");
    write(
        &path,
        &fs::read_to_string(&path).unwrap().replace(
            "<Name>Order</Name>",
            &format!("<Name>Order</Name>{properties}"),
        ),
    );
}

#[test]
fn document_form_relations_preserve_each_role_and_read_the_returned_form() {
    let fixture = RealReaderFixture::new();
    let properties = SLOTS
        .iter()
        .map(|(property, _)| format!("<{property}>{TARGET}</{property}>"))
        .collect::<String>();
    install(&fixture, &properties);
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(owner.ok);
    let owner = owner.data.unwrap();
    for (property, _) in SLOTS {
        assert_eq!(owner["props"][property], TARGET);
    }
    assert!(owner["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at":"main:Document.Order.Relation", "count":8})));
    let mut cursor = None;
    let mut items = Vec::new();
    loop {
        let mut request = ViewRequest::new("main:Document.Order.Relation")
            .unwrap()
            .with_limit(2)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let page = service.view(request);
        assert!(page.ok, "{:?}", refusal_codes(&page));
        items.extend(page.data.unwrap()["items"].as_array().unwrap().clone());
        cursor = page.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(items.len(), 8);
    for (_, role) in SLOTS {
        let expected = json!({"relation":role,"at":format!("main:{TARGET}"),"kind":"Form"});
        assert_eq!(items.iter().filter(|item| **item == expected).count(), 1);
        let form = service.view(ViewRequest::new(expected["at"].as_str().unwrap()).unwrap());
        assert!(form.ok, "{:?}", refusal_codes(&form));
        assert_eq!(form.data.unwrap()["kind"], "Form");
    }
    assert!(items.iter().any(|item| item["relation"] == "basedOn"));
    assert!(items
        .iter()
        .any(|item| item["relation"] == "registerRecord"));
    let denied = service.view(ViewRequest::new("main:Document.Order.Relation.ItemForm").unwrap());
    assert!(!denied.ok);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn document_form_relations_keep_other_roles_when_one_slot_is_unknown_or_damaged() {
    for (property, role) in SLOTS {
        for (raw, limited) in [
            (String::new(), true),
            (format!("<{property}/>"), false),
            (
                format!("<{property}>{TARGET}</{property}><{property}/>"),
                true,
            ),
            (
                format!("<{property} xmlns=\"urn:foreign\">{TARGET}</{property}>"),
                true,
            ),
            (
                format!("<{property}><Nested>{TARGET}</Nested></{property}>"),
                true,
            ),
            (format!("<{property}>Catalog.Items</{property}>"), true),
            (
                format!("<{property}>Document.Missing.Form.Main</{property}>"),
                true,
            ),
            (format!("<{property}>CommonForm.Shared</{property}>"), true),
        ] {
            let fixture = RealReaderFixture::new();
            let properties = SLOTS
                .iter()
                .map(|(other, _)| {
                    if *other == property {
                        raw.clone()
                    } else {
                        format!("<{other}>{TARGET}</{other}>")
                    }
                })
                .collect::<String>();
            install(&fixture, &properties);
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let service = fixture.view_service();
            for at in ["main:Document.Order", "main:Document.Order.Relation"] {
                let result = service.view(ViewRequest::new(at).unwrap());
                assert!(result.ok, "{property}: {:?}", refusal_codes(&result));
                let node = result.data.unwrap();
                let limits = node["limits"].as_array().cloned().unwrap_or_default();
                assert_eq!(
                    limits.iter().any(|limit| limit
                        .as_str()
                        .is_some_and(|text| text.contains(role) || text.contains(property))),
                    limited,
                    "{property}: {raw}: {at}"
                );
                if raw.contains("CommonForm.Shared") {
                    assert!(limits.iter().any(|limit| limit.as_str().is_some_and(
                        |text| text.contains("outside the document Form relation read profile")
                    )));
                    assert!(!limits.iter().any(|limit| limit
                        .as_str()
                        .is_some_and(|text| text.contains("malformed"))));
                }
                if at.ends_with("Relation") {
                    let items = node["items"].as_array().unwrap();
                    assert_eq!(items.len(), 7, "{property}: {raw}");
                    assert!(!items.iter().any(|item| item["relation"] == role));
                    for (_, other) in SLOTS.into_iter().filter(|(_, other)| *other != role) {
                        assert_eq!(
                            items
                                .iter()
                                .filter(|item| item["relation"] == other)
                                .count(),
                            1
                        );
                    }
                }
            }
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
    }
}

#[test]
fn document_form_relations_prove_registration_and_identity_of_each_target() {
    for variant in ["unregistered", "missing", "wrong-name", "wrong-kind"] {
        let fixture = RealReaderFixture::new();
        let catalog = fixture.source.join("Catalogs/Items.xml");
        write(
            &catalog,
            &fs::read_to_string(&catalog)
                .unwrap()
                .replace("</ChildObjects>", "<Form>Second</Form></ChildObjects>"),
        );
        let descriptor = fixture.source.join("Catalogs/Items/Forms/Second.xml");
        write(
            &descriptor,
            &fs::read_to_string(fixture.source.join("Catalogs/Items/Forms/ItemForm.xml"))
                .unwrap()
                .replace("<Name>ItemForm</Name>", "<Name>Second</Name>"),
        );
        match variant {
            "unregistered" => write(
                &catalog,
                &fs::read_to_string(&catalog)
                    .unwrap()
                    .replace("<Form>Second</Form>", ""),
            ),
            "missing" => fs::remove_file(&descriptor).unwrap(),
            "wrong-name" => write(
                &descriptor,
                &fs::read_to_string(&descriptor)
                    .unwrap()
                    .replace("<Name>Second</Name>", "<Name>Other</Name>"),
            ),
            "wrong-kind" => write(
                &descriptor,
                &fs::read_to_string(&descriptor)
                    .unwrap()
                    .replace("<Form ", "<Template ")
                    .replace("</Form>", "</Template>"),
            ),
            _ => unreachable!(),
        }
        let properties = SLOTS
            .iter()
            .map(|(property, _)| {
                format!(
                    "<{property}>{}</{property}>",
                    if *property == "DefaultObjectForm" {
                        "Catalog.Items.Form.Second"
                    } else {
                        TARGET
                    }
                )
            })
            .collect::<String>();
        install(&fixture, &properties);
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Document.Order.Relation").unwrap());
        assert!(result.ok, "{variant}: {:?}", refusal_codes(&result));
        let node = result.data.unwrap();
        assert_eq!(node["items"].as_array().unwrap().len(), 7, "{variant}");
        assert!(!node["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["relation"] == "defaultObjectForm"));
        assert!(node["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|limit| limit.as_str().unwrap().contains("defaultObjectForm")));
    }
}

#[test]
fn document_form_diagnostic_alias_does_not_change_catalog_relations() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Catalogs/Items.xml");
    write(
        &path,
        &fs::read_to_string(&path).unwrap().replace(
            "<Name>Items</Name>",
            "<Name>Items</Name><DefaultObjectForm><Nested/></DefaultObjectForm>",
        ),
    );
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok);
    assert!(owner.data.unwrap()["limits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|limit| limit.as_str().unwrap().contains("DefaultObjectForm")));
    let relations = service.view(ViewRequest::new("main:Catalog.Items.Relation").unwrap());
    assert!(relations.ok);
    assert!(!relations.data.unwrap()["limits"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|limit| limit.as_str().unwrap().contains("documentForms")));
}

#[test]
fn document_form_relations_preserve_whole_owner_refusal_for_damaged_registered_own_form() {
    for wrong_kind in [false, true] {
        let fixture = RealReaderFixture::new();
        let owner = fixture.source.join("Documents/Order.xml");
        write(
            &owner,
            &fs::read_to_string(&owner).unwrap().replace(
                "<ChildObjects/>",
                "<ChildObjects><Form>Shared</Form></ChildObjects>",
            ),
        );
        install(&fixture,"<DefaultObjectForm>Document.Order.Form.Shared</DefaultObjectForm><DefaultListForm>Catalog.Items.Form.ItemForm</DefaultListForm>");
        if wrong_kind {
            let descriptor =
                fs::read_to_string(fixture.source.join("Catalogs/Items/Forms/ItemForm.xml"))
                    .unwrap()
                    .replace("<Name>ItemForm</Name>", "<Name>Shared</Name>")
                    .replace("<Form ", "<Template ")
                    .replace("</Form>", "</Template>");
            write(
                &fixture.source.join("Documents/Order/Forms/Shared.xml"),
                &descriptor,
            );
        }
        let service = fixture.view_service();
        for at in ["main:Document.Order", "main:Document.Order.Relation"] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(!result.ok, "{at}: {result:?}");
            assert_eq!(refusal_codes(&result), ["provider_unavailable"]);
        }
    }
}

#[test]
fn document_form_relations_omit_unreadable_foreign_form_content_and_keep_other_roles() {
    for missing in [true, false] {
        let fixture = RealReaderFixture::new();
        let catalog = fixture.source.join("Catalogs/Items.xml");
        write(
            &catalog,
            &fs::read_to_string(&catalog)
                .unwrap()
                .replace("</ChildObjects>", "<Form>Good</Form></ChildObjects>"),
        );
        let descriptor =
            fs::read_to_string(fixture.source.join("Catalogs/Items/Forms/ItemForm.xml"))
                .unwrap()
                .replace("<Name>ItemForm</Name>", "<Name>Good</Name>");
        write(
            &fixture.source.join("Catalogs/Items/Forms/Good.xml"),
            &descriptor,
        );
        let content = fixture
            .source
            .join("Catalogs/Items/Forms/ItemForm/Ext/Form.xml");
        write(
            &fixture
                .source
                .join("Catalogs/Items/Forms/Good/Ext/Form.xml"),
            &fs::read_to_string(&content).unwrap(),
        );
        if missing {
            fs::remove_file(&content).unwrap();
        } else {
            write(&content, "<Form>broken");
        }
        install(&fixture,"<DefaultObjectForm>Catalog.Items.Form.ItemForm</DefaultObjectForm><DefaultListForm>Catalog.Items.Form.Good</DefaultListForm>");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let broken = service.view(ViewRequest::new("main:Catalog.Items.Form.ItemForm").unwrap());
        assert!(!broken.ok);
        let result = service.view(ViewRequest::new("main:Document.Order.Relation").unwrap());
        assert!(result.ok, "{:?}", refusal_codes(&result));
        let data = result.data.unwrap();
        let good = data["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["relation"] == "defaultListForm")
            .unwrap();
        let followed = service.view(ViewRequest::new(good["at"].as_str().unwrap()).unwrap());
        assert!(followed.ok, "{:?}", refusal_codes(&followed));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );

        assert!(
            !data["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["relation"] == "defaultObjectForm"),
            "{data:#}"
        );
        assert!(data["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["relation"] == "defaultListForm"));
        assert!(data["limits"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().is_some_and(|v| v.contains("defaultObjectForm"))));
    }
}
