use super::*;

fn replace_characteristics(fixture: &RealReaderFixture, replacement: impl FnOnce(&str) -> String) {
    let path = fixture.source.join("Catalogs/Items.xml");
    let xml = fs::read_to_string(&path).unwrap();
    let start = xml.find("<Characteristics>").unwrap();
    let end = xml.find("</Characteristics>").unwrap() + "</Characteristics>".len();
    write(
        &path,
        &format!(
            "{}{}{}",
            &xml[..start],
            replacement(&xml[start..end]),
            &xml[end..]
        ),
    );
}

#[test]
fn characteristic_relations_preserve_roles_pages_and_anonymous_declarations() {
    let fixture = RealReaderFixture::new();
    replace_characteristics(&fixture, |xml| {
        let inner = xml
            .strip_prefix("<Characteristics>")
            .unwrap()
            .strip_suffix("</Characteristics>")
            .unwrap();
        format!("<Characteristics>{inner}{inner}</Characteristics>")
    });
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    let branches = owner.data.as_ref().unwrap()["branches"].as_array().unwrap();
    assert!(branches.contains(&json!({"at":"main:Catalog.Items.Relation","count":18})));
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let mut request = ViewRequest::new("main:Catalog.Items.Relation")
            .unwrap()
            .with_limit(3)
            .unwrap();
        if let Some(value) = cursor.take() {
            request = request.with_cursor(value);
        }
        let result = service.view(request);
        assert!(result.ok, "{result:?}");
        rows.extend(
            result.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .iter()
                .cloned(),
        );
        cursor = result.cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(rows.len(), 18);
    let facts: Vec<_> = rows
        .iter()
        .filter(|row| {
            row["relation"]
                .as_str()
                .unwrap()
                .starts_with("characteristic")
        })
        .collect();
    assert_eq!(facts.len(), 14);
    assert_eq!(facts[..7], facts[7..]);
    assert_eq!(facts[0]["kind"], "Attribute");
    assert_eq!(facts[3]["kind"], "Catalog");
    let repeated = service.view(
        ViewRequest::new("main:Catalog.Items.Relation")
            .unwrap()
            .with_limit(50)
            .unwrap(),
    );
    assert_eq!(repeated.data.as_ref().unwrap()["items"], json!(rows));
    let anonymous = service.view(ViewRequest::new("main:Catalog.Items.Characteristic").unwrap());
    let declarations = anonymous.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(declarations.len(), 2);
    assert!(declarations.iter().all(|row| row.get("at").is_none()));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn characteristic_relation_unknown_empty_and_damage_keep_ordinary_relations() {
    for (replacement, expected_limit) in [
        ("",Some("absent")),
        ("<Characteristics/>",None),
        ("<Characteristics><xr:Characteristic><xr:Unexpected/></xr:Characteristic></Characteristics>",Some("не прочитано")),
        ("<Characteristics/><Characteristics/>",Some("не прочитано")),
    ] {
        let fixture=RealReaderFixture::new();
        replace_characteristics(&fixture, |_| replacement.to_string());
        let before=unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service=fixture.view_service();
        let result=service.view(ViewRequest::new("main:Catalog.Items.Relation").unwrap());
        assert!(result.ok,"{result:?}");
        let data=result.data.as_ref().unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(),4);
        let limits=data["limits"].to_string();
        if let Some(expected)=expected_limit {
            assert!(limits.contains("characteristics"),"{limits}");
            assert!(limits.contains(expected),"{limits}");
        } else { assert!(!limits.contains("characteristics"),"{limits}"); }
        assert_eq!(unica_workspace::test_support::tree_snapshot(&fixture.source),before);
    }
}

#[test]
fn characteristic_relations_exclude_sentinels_filters_and_unqualified_fields() {
    let fixture = RealReaderFixture::new();
    replace_characteristics(&fixture, |xml| {
        xml.replace(
            "from=\"Catalog.Items.TabularSection.Lines.Attribute.Quantity\"",
            "from=\"-1\"",
        )
        .replace(
            "<xr:KeyField>Catalog.Items.Attribute.Code</xr:KeyField>",
            "<xr:KeyField>Code</xr:KeyField>",
        )
        .replace(
            "<xr:TypesFilterField>Catalog.Items.Attribute.Code</xr:TypesFilterField>",
            "<xr:TypesFilterField>Catalog</xr:TypesFilterField>",
        )
        .replace(">Catalog_Items<", ">Catalog.Decoy<")
        .replace(
            "<xr:DataPathField>-1</xr:DataPathField>",
            "<xr:DataPathField>Catalog.Decoy</xr:DataPathField>",
        )
        .replace(
            "<xr:MultipleValuesUseField>-1</xr:MultipleValuesUseField>",
            "<xr:MultipleValuesUseField>Catalog.Decoy</xr:MultipleValuesUseField>",
        )
    });
    let service = fixture.view_service();
    let result = service.view(ViewRequest::new("main:Catalog.Items.Relation").unwrap());
    assert!(result.ok, "{result:?}");
    let rows = result.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(rows.len(), 8);
    assert!(rows
        .iter()
        .all(|row| !row["at"].as_str().unwrap().contains("Decoy")));
    assert!(rows
        .iter()
        .all(|row| row["kind"] != "Catalog" || row["at"] != "main:Catalog"));
}

#[test]
fn characteristic_relations_reject_lexical_addresses_outside_metadata_fields() {
    for target in [
        "Attribute.Code",
        "Catalog.Items.Props.Name",
        "Catalog.Items.Catalog.Other",
    ] {
        let fixture = RealReaderFixture::new();
        replace_characteristics(&fixture, |xml| {
            xml.replace(
                "<xr:KeyField>Catalog.Items.Attribute.Code</xr:KeyField>",
                &format!("<xr:KeyField>{target}</xr:KeyField>"),
            )
        });
        let service = fixture.view_service();
        let result = service.view(ViewRequest::new("main:Catalog.Items.Relation").unwrap());
        assert!(result.ok, "{result:?}");
        assert!(
            result.data.as_ref().unwrap()["items"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["relation"] != "characteristicTypesKeyField"),
            "{target}: {result:?}"
        );
        let raw = service.view(ViewRequest::new("main:Catalog.Items.Characteristic").unwrap());
        assert_eq!(
            raw.data.as_ref().unwrap()["items"][0]["types"]["keyField"],
            target
        );
    }
}
