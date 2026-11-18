use super::*;

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn missing_relation_preserves_known_targets_and_reports_incompleteness() {
    // Synthetic relation variants isolate reader behavior; the raw platform
    // borrowed catalog separately proves that these containers may be absent.
    for (owners, expected_unknown) in [
        ("", true),
        ("<Owners/>", false),
        ("<Owners><Broken/></Owners>", true),
        ("<Owners xmlns=\"urn:foreign\"/>", true),
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let xml = fs::read_to_string(&path).unwrap();
        let document = roxmltree::Document::parse(&xml).unwrap();
        let owner_range = document
            .descendants()
            .find(|node| node.has_tag_name("Owners"))
            .unwrap()
            .range();
        let mut xml = xml.clone();
        xml.replace_range(owner_range, "");
        let xml = xml.replace("</Properties>", &format!(r#"{owners}<BasedOn xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><xr:Item xsi:type="xr:MDObjectRef">Document.Order</xr:Item></BasedOn></Properties>"#));
        write(&path, &xml);
        let service = fixture.view_service();
        for at in ["main:Catalog.Items", "main:Catalog.Items.Relation"] {
            let result = service.view(ViewRequest::new(at).unwrap());
            assert!(result.ok, "the read must succeed");
            let data = result.data.unwrap();
            let has_limit = data["limits"].as_array().is_some_and(|limits| {
                limits
                    .iter()
                    .any(|limit| limit.as_str().unwrap().contains("relations.owners"))
            });
            assert_eq!(
                has_limit, expected_unknown,
                "relation limits must follow the relation state"
            );
            if at.ends_with(".Relation") {
                assert!(
                    data["items"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| item["relation"] == "basedOn"
                            && item["at"] == "main:Document.Order"),
                    "relation limits must follow the relation state"
                );
            }
        }
        assert_eq!(fs::read_to_string(path).unwrap(), xml);
    }
}

/// rule: INV.SOURCE.INCOMPLETE-METADATA-READ
#[test]
fn specialized_relation_limits_distinguish_absence_empty_and_damage() {
    // Synthetic edits exercise each reader route. Absence of BaseCalculationTypes
    // also occurs in the unmodified platform export used by acceptance S389.
    for (directory, kind, name, property, field) in [
        (
            "DocumentJournals",
            "DocumentJournal",
            "Журнал",
            "RegisteredDocuments",
            "registeredDocuments",
        ),
        (
            "ChartsOfCalculationTypes",
            "ChartOfCalculationTypes",
            "ВидыРасчета",
            "BaseCalculationTypes",
            "baseCalculationTypes",
        ),
        (
            "WebServices",
            "WebService",
            "Exchange",
            "XDTOPackages",
            "xdtoPackages",
        ),
    ] {
        for variant in ["missing", "empty", "broken", "duplicate", "foreign"] {
            let fixture = RealReaderFixture::new();
            let path = fixture.source.join(format!("{directory}/{name}.xml"));
            let xml = fs::read_to_string(&path).unwrap();
            let doc = roxmltree::Document::parse(&xml).unwrap();
            let range = doc
                .descendants()
                .find(|node| node.has_tag_name(property))
                .unwrap()
                .range();
            let replacement = match variant {
                "missing" => String::new(),
                "empty" => format!("<{property}/>"),
                "broken" => format!("<{property}><Broken/></{property}>"),
                "duplicate" => format!("<{property}/><{property}/>"),
                "foreign" => format!("<{property} xmlns=\"urn:foreign\"/>"),
                _ => unreachable!(),
            };
            let mut xml = xml.clone();
            xml.replace_range(range, &replacement);
            write(&path, &xml);
            let service = fixture.view_service();
            for suffix in ["", ".Relation"] {
                let result =
                    service.view(ViewRequest::new(&format!("main:{kind}.{name}{suffix}")).unwrap());
                assert!(result.ok, "{variant}: the read must succeed");
                let data = result.data.unwrap();
                let relevant = data["limits"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(serde_json::Value::as_str)
                    .filter(|limit| {
                        limit.contains(&format!("details.{field}"))
                            || limit.contains(&format!("properties.{property}"))
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    !relevant.is_empty(),
                    variant != "empty",
                    "{variant}: limits must follow the property state"
                );
                assert_eq!(
                    relevant.iter().any(|limit| limit.contains(" is absent;")),
                    variant == "missing",
                    "{variant}: limits must follow the property state"
                );
            }
            assert_eq!(fs::read_to_string(path).unwrap(), xml);
        }
    }
}
