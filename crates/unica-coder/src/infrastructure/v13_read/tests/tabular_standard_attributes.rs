use super::*;

#[test]
fn tabular_sections_expose_source_standard_attributes_without_losing_applied_attributes() {
    let fixture = RealReaderFixture::new();
    copy_fixture(
        "acceptance/workspace-tabular-standard/src/Catalogs/Items.xml",
        &fixture.source.join("Catalogs/Items.xml"),
    );
    let before = fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap();
    let service = fixture.view_service();
    let at = "main:Catalog.Items.TabularSection.Rows";
    let section = service.view(ViewRequest::new(at).unwrap());
    assert_eq!(
        view_branch_count(&section, &format!("{at}.StandardAttribute")),
        Some(1)
    );
    let applied = view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap()));
    let xml = std::str::from_utf8(&before).unwrap();
    let doc = roxmltree::Document::parse(xml).unwrap();
    let rows = doc
        .descendants()
        .find(|n| n.has_tag_name("TabularSection"))
        .unwrap();
    let names = rows
        .descendants()
        .filter(|n| n.has_tag_name("Attribute"))
        .map(|n| {
            n.descendants()
                .find(|c| c.has_tag_name("Name"))
                .unwrap()
                .text()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 10);
    assert_eq!(
        applied
            .iter()
            .map(|v| v["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        names
    );
    let standard =
        view_items(&service.view(ViewRequest::new(&format!("{at}.StandardAttribute")).unwrap()));
    assert_eq!(standard.len(), 1);
    let detail_at = standard[0]["at"].as_str().unwrap();
    assert_eq!(detail_at, format!("{at}.StandardAttribute.LineNumber"));
    let detail = service.view(ViewRequest::new(detail_at).unwrap());
    assert!(detail.ok, "the call failed");
    let data = detail.data.unwrap();
    assert_eq!(data["kind"], "StandardAttribute");
    // Пустые наблюдения (`empty`, `nil`, многоязычная строка без переводов)
    // свойства не создают; остальные приходят без обёртки наблюдения (#2031).
    assert_eq!(data["props"].as_object().unwrap().len(), 16);
    for (key, expected) in [
        ("LinkByType", None),
        ("FillChecking", Some(json!("DontCheck"))),
        ("MultiLine", Some(json!(false))),
        ("FillFromFillingValue", Some(json!(false))),
        ("CreateOnInput", Some(json!("Auto"))),
        ("TypeReductionMode", Some(json!("TransformValues"))),
        ("MaxValue", None),
        ("ToolTip", None),
        ("ExtendedEdit", Some(json!(false))),
        ("Format", None),
        ("ChoiceForm", Some(json!(""))),
        ("QuickChoice", Some(json!("Auto"))),
        ("ChoiceHistoryOnInput", Some(json!("Auto"))),
        ("EditFormat", None),
        ("PasswordMode", Some(json!(false))),
        ("DataHistory", Some(json!("Use"))),
        ("MarkNegatives", Some(json!(false))),
        ("MinValue", None),
        (
            "Synonym",
            Some(json!([{"language":"ru","content":"Номер строки"}])),
        ),
        ("Comment", Some(json!("Line-number note"))),
        ("FullTextSearch", Some(json!("Use"))),
        ("ChoiceParameterLinks", None),
        ("FillValue", None),
        ("Mask", Some(json!(""))),
        ("ChoiceParameters", None),
    ] {
        match expected {
            Some(expected) => assert_eq!(data["props"][key], expected, "{key}"),
            None => {
                assert!(data["props"].get(key).is_none(), "{key} must stay absent");
                assert!(
                    !data["limits"].to_string().contains(key),
                    "an empty {key} is not a limit"
                );
            }
        }
        assert_eq!(data["props"].get(key), standard[0]["props"].get(key));
    }
    assert!(data["props"].get("type").is_none());
    let missing =
        service.view(ViewRequest::new(&format!("{at}.StandardAttribute.Missing")).unwrap());
    assert_eq!(missing.diagnostics[0]["code"], "not_found");
    assert_eq!(
        fs::read(fixture.source.join("Catalogs/Items.xml")).unwrap(),
        before
    );
}

#[test]
fn tabular_standard_attribute_damage_is_local_and_not_an_empty_observation() {
    for variant in [
        "missing",
        "empty",
        "no-properties",
        "bad-entry",
        "duplicate-container",
        "foreign-container",
        "container-attributes",
        "foreign-entry",
        "duplicate-property",
        "bare-localized-text",
        "invalid-name",
        "whitespace-name",
        "duplicate-name",
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let original = fixture_text("acceptance/workspace-tabular-standard/src/Catalogs/Items.xml");
        let doc = roxmltree::Document::parse(&original).unwrap();
        let container = doc
            .descendants()
            .find(|n| n.has_tag_name("StandardAttributes"))
            .unwrap();
        let observed = &original[container.range()];
        let replacement = match variant {
            "missing" => String::new(),
            "empty" => "<StandardAttributes/>".into(),
            "no-properties" => "<StandardAttributes><xr:StandardAttribute name=\"LineNumber\"/></StandardAttributes>".into(),
            "bad-entry" => "<StandardAttributes><xr:StandardAttribute/></StandardAttributes>".into(),
            "duplicate-container" => format!("{observed}{observed}"),
            "foreign-container" => observed.replace("<StandardAttributes>", "<StandardAttributes xmlns=\"urn:foreign\">"),
            "container-attributes" => observed.replace("<StandardAttributes>", "<StandardAttributes unexpected=\"yes\">"),
            "foreign-entry" => observed.replace("<xr:StandardAttribute ", "<xr:StandardAttribute xmlns:xr=\"urn:foreign\" "),
            "bare-localized-text" => observed.replace("<xr:ToolTip/>", "<xr:ToolTip>broken</xr:ToolTip>"),
            "duplicate-name" => "<StandardAttributes><xr:StandardAttribute name=\"LineNumber\"><xr:Comment>first</xr:Comment></xr:StandardAttribute><xr:StandardAttribute name=\"LineNumber\"><xr:Comment>second</xr:Comment></xr:StandardAttribute></StandardAttributes>".into(),
            "invalid-name" => observed.replace("name=\"LineNumber\"", "name=\"bad name\""),
            "whitespace-name" => observed.replace("name=\"LineNumber\"", "name=\"   \""),
            "duplicate-property" => observed.replace("<xr:ToolTip/>", "<xr:ToolTip/><xr:ToolTip/>"),
            _ => unreachable!(),
        };
        let damaged = !matches!(variant, "missing" | "empty" | "no-properties");
        let mut xml = original.clone();
        xml.replace_range(container.range(), &replacement);
        write(&path, &xml);
        let before = fs::read(&path).unwrap();
        let service = fixture.view_service();
        let sections = view_items(
            &service.view(ViewRequest::new("main:Catalog.Items.TabularSection").unwrap()),
        );
        for name in ["Rows", "Healthy"] {
            let at = format!("main:Catalog.Items.TabularSection.{name}");
            let section = service.view(ViewRequest::new(&at).unwrap());
            assert!(section.ok, "{variant} {name}: the call failed");
            let data = section.data.as_ref().unwrap();
            let is_damaged = damaged && name == "Rows";
            assert_eq!(
                data["limits"].to_string().contains("standardAttributes"),
                is_damaged,
                "{variant} {name}: {data}"
            );
            let listed = sections.iter().find(|v| v["title"] == name).unwrap();
            assert_eq!(
                listed.get("limits"),
                data.get("limits"),
                "{variant}: collection and detail disagree"
            );
            assert_eq!(
                view_branch_count(&section, &format!("{at}.Attribute")),
                Some(10)
            );
            assert_eq!(
                view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap()))
                    .len(),
                10
            );
            let collection =
                service.view(ViewRequest::new(&format!("{at}.StandardAttribute")).unwrap());
            let detail = service
                .view(ViewRequest::new(&format!("{at}.StandardAttribute.LineNumber")).unwrap());
            if name == "Healthy" || variant == "no-properties" {
                let items = view_items(&collection);
                assert_eq!(items.len(), 1);
                assert!(detail.ok, "{variant}: the call failed");
                assert_eq!(detail.data.as_ref().unwrap()["kind"], "StandardAttribute");
                if variant == "no-properties" && name == "Rows" {
                    assert!(
                        detail.data.as_ref().unwrap().get("props").is_none(),
                        "missing properties must not be invented"
                    );
                }
            } else if variant == "empty" {
                assert!(view_items(&collection).is_empty());
                assert_eq!(detail.diagnostics[0]["code"], "not_found");
            } else if variant == "missing" {
                assert_eq!(collection.diagnostics[0]["code"], "not_found");
                assert_eq!(detail.diagnostics[0]["code"], "not_found");
            } else {
                for refused in [&collection, &detail] {
                    assert!(
                        !refused.ok,
                        "{variant}: the call was expected to be refused"
                    );
                    assert_eq!(refused.diagnostics[0]["detailCode"], "source_unreadable");
                    assert!(refused.diagnostics[0]["message"]
                        .as_str()
                        .unwrap()
                        .contains("collections.tabularSections[0].standardAttributes"));
                }
            }
        }
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}

#[test]
fn tabular_standard_attributes_preserve_profiled_member_facts_and_independent_limits() {
    for (damaged_standard, damaged_member) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let fixture = RealReaderFixture::new();
        let original = fixture_text("acceptance/workspace-tabular-standard/src/Catalogs/Items.xml");
        let use_value = if damaged_member { "broken" } else { "ForItem" };
        let mut xml = original.replacen(
            "<Name>Rows</Name>",
            &format!(
                "<Name>Rows</Name><LineNumberLength>5</LineNumberLength><Use>{use_value}</Use>"
            ),
            1,
        );
        if damaged_standard {
            xml = xml.replacen("<xr:ToolTip/>", "<xr:ToolTip>broken</xr:ToolTip>", 1);
        }
        write(&fixture.source.join("Catalogs/Items.xml"), &xml);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let service = fixture.view_service();
        let at = "main:Catalog.Items.TabularSection.Rows";
        let section = service.view(ViewRequest::new(at).unwrap());
        assert!(section.ok, "the call failed");
        let data = section.data.as_ref().unwrap();
        assert_eq!(data["props"]["LineNumberLength"], 5);
        if damaged_member {
            assert!(data["props"].get("Use").is_none());
        } else {
            assert_eq!(data["props"]["Use"], "ForItem");
        }
        let limits = data["limits"].to_string();
        assert_eq!(limits.contains(".properties.Use"), damaged_member);
        assert_eq!(limits.contains(".standardAttributes"), damaged_standard);
        let sections = view_items(
            &service.view(ViewRequest::new("main:Catalog.Items.TabularSection").unwrap()),
        );
        let listed = sections.iter().find(|item| item["at"] == at).unwrap();
        assert_eq!(listed["props"], data["props"]);
        assert_eq!(listed.get("limits"), data.get("limits"));
        assert_eq!(
            view_items(&service.view(ViewRequest::new(&format!("{at}.Attribute")).unwrap())).len(),
            10
        );
        let standard =
            service.view(ViewRequest::new(&format!("{at}.StandardAttribute.LineNumber")).unwrap());
        if damaged_standard {
            assert!(!standard.ok, "the call was expected to be refused");
            assert_eq!(standard.diagnostics[0]["detailCode"], "source_unreadable");
        } else {
            assert!(standard.ok, "the call failed");
            assert_eq!(
                standard.data.as_ref().unwrap()["props"]
                    .as_object()
                    .unwrap()
                    .len(),
                16
            );
        }
        let healthy_at = "main:Catalog.Items.TabularSection.Healthy";
        let healthy = service.view(ViewRequest::new(healthy_at).unwrap());
        assert!(healthy.ok, "the call failed");
        let healthy_limits = healthy.data.as_ref().unwrap()["limits"].to_string();
        assert!(!healthy_limits.contains(".properties.Use"));
        assert!(!healthy_limits.contains(".standardAttributes"));
        assert!(
            service
                .view(
                    ViewRequest::new(&format!("{healthy_at}.StandardAttribute.LineNumber"))
                        .unwrap()
                )
                .ok
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}
