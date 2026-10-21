use super::*;

#[test]
fn metadata_kind_branch_lists_registered_objects_with_canonical_addresses() {
    let fixture = RealReaderFixture::new();
    let cancellation = CancellationToken::new();
    let source_root =
        Arc::new(unica_workspace::read::open_source_directory(&fixture.source).unwrap());
    let authority = LogicalViewReadAuthority::new(
        &cancellation,
        "main",
        "actor-fixture-main",
        SourceSetKind::Configuration,
        source_root,
        PlatformProfile::v8_3_27(),
    );
    let service = ViewService::new(authority, ViewCursorStore::default());

    let branch = service.view(ViewRequest::new("main:Catalog").unwrap());
    assert!(branch.ok, "{:?}", refusal_codes(&branch));
    assert_eq!(
        branch.data.as_ref().unwrap()["items"],
        json!([
            {
                "at": "main:Catalog.Items",
                "kind": "Catalog",
                "title": "Items"
            },
            {
                "at": "main:Catalog.Владельцы",
                "kind": "Catalog",
                "title": "Владельцы"
            }
        ]),
    );
}

#[test]
fn metadata_node_props_carry_the_observed_object_properties() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());

    assert!(result.ok, "{:?}", refusal_codes(&result));
    let props = &result.data.as_ref().unwrap()["props"];
    // Свойства объекта отвечают на узле объекта: раньше проекция искала шесть
    // скаляров в `details`, где их нет ни у одного вида, и узел молчал.
    assert_eq!(props["Hierarchical"], json!(true));
    assert_eq!(props["CodeLength"], json!(11));
    assert_eq!(props["kind"], json!("Catalog"));
}

#[test]
fn catalog_standard_attribute_properties_reach_collection_and_detail() {
    let fixture = RealReaderFixture::new();
    let xml = fixture_text("acceptance/workspace-standard-attributes/src/Catalogs/Items.xml");
    let path = fixture.source.join("Catalogs/Items.xml");
    write(&path, &xml);
    let service = fixture.view_service();
    let collection =
        service.view(ViewRequest::new("main:Catalog.Items.StandardAttribute").unwrap());
    assert!(collection.ok, "{collection:?}");
    let data = collection.data.unwrap();
    let items = data["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    let document = roxmltree::Document::parse(&xml).unwrap();
    for (index, name) in ["Code", "Description"].into_iter().enumerate() {
        let at = items[index]["at"].as_str().unwrap();
        assert_eq!(at, format!("main:Catalog.Items.StandardAttribute.{name}"));
        let detail = service.view(ViewRequest::new(at).unwrap());
        assert!(detail.ok, "{detail:?}");
        let detail = detail.data.unwrap();
        assert_eq!(detail["title"], name);
        assert_eq!(items[index]["props"], detail["props"]);
        let source = document
            .descendants()
            .find(|node| node.attribute("name") == Some(name))
            .unwrap();
        for key in ["Synonym", "ToolTip", "FillChecking"] {
            let value: serde_json::Value =
                serde_json::from_str(detail["props"][key].as_str().unwrap()).unwrap();
            let property = source
                .children()
                .find(|node| node.is_element() && node.tag_name().name() == key)
                .unwrap();
            if key == "FillChecking" {
                assert_eq!(
                    value,
                    json!({"kind":"text","value":property.text().unwrap()})
                );
            } else {
                let expected = property
                    .children()
                    .filter(|node| node.is_element())
                    .map(|item| {
                        let text = |tag| {
                            item.children()
                                .find(|node| node.is_element() && node.tag_name().name() == tag)
                                .unwrap()
                                .text()
                                .unwrap_or("")
                        };
                        json!({"language":text("lang"),"content":text("content")})
                    })
                    .collect::<Vec<_>>();
                assert_eq!(value, json!({"kind":"localizedString","values":expected}));
            }
        }
    }
    assert_eq!(items[2]["title"], "Ref");
    assert!(items[2]
        .get("props")
        .is_none_or(|props| props.as_object().unwrap().is_empty()));
    assert_eq!(fs::read_to_string(path).unwrap(), xml);
}

#[test]
fn catalog_standard_attribute_empty_absent_and_damaged_properties_stay_distinct() {
    for key in ["Synonym", "ToolTip", "FillChecking"] {
        for state in ["empty", "absent", "damaged", "bare-text"] {
            if state == "bare-text" && key == "FillChecking" {
                continue;
            }
            let fixture = RealReaderFixture::new();
            let path = fixture.source.join("Catalogs/Items.xml");
            let base =
                fixture_text("acceptance/workspace-standard-attributes/src/Catalogs/Items.xml");
            let document = roxmltree::Document::parse(&base).unwrap();
            let mut xml = base.clone();
            let mut ranges = document
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == key)
                .map(|node| node.range())
                .collect::<Vec<_>>();
            for range in ranges.drain(..).rev() {
                let replacement = match state {
                    "empty" => format!("<xr:{key}/>"),
                    "absent" => String::new(),
                    "damaged" => format!("<xr:{key}><xr:Unexpected/></xr:{key}>"),
                    "bare-text" => format!("<xr:{key}>broken</xr:{key}>"),
                    _ => unreachable!(),
                };
                xml.replace_range(range, &replacement);
            }
            write(&path, &xml);
            let service = fixture.view_service();
            let collection =
                service.view(ViewRequest::new("main:Catalog.Items.StandardAttribute").unwrap());
            if matches!(state, "damaged" | "bare-text") {
                assert!(!collection.ok, "{key}: {collection:?}");
                assert_eq!(collection.diagnostics[0]["detailCode"], "source_unreadable");
                for name in ["Code", "Description"] {
                    let detail = service.view(
                        ViewRequest::new(&format!("main:Catalog.Items.StandardAttribute.{name}"))
                            .unwrap(),
                    );
                    assert!(!detail.ok, "{detail:?}");
                    assert_eq!(detail.diagnostics[0]["detailCode"], "source_unreadable");
                }
                let root = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
                assert!(root.ok, "{root:?}");
                assert!(root.data.unwrap()["limits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|limit| limit.as_str().unwrap().contains("standardAttributes")));
            } else {
                assert!(collection.ok, "{collection:?}");
                for item in &collection.data.unwrap()["items"].as_array().unwrap()[..2] {
                    let detail =
                        service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
                    assert!(detail.ok, "{detail:?}");
                    let detail = detail.data.unwrap();
                    assert_eq!(item.get("props"), detail.get("props"));
                    if state == "absent" {
                        assert!(detail["props"].get(key).is_none());
                    } else {
                        let value: serde_json::Value =
                            serde_json::from_str(detail["props"][key].as_str().unwrap()).unwrap();
                        let expected = if key == "FillChecking" {
                            json!({"kind":"text","value":""})
                        } else {
                            json!({"kind":"localizedString","values":[]})
                        };
                        assert_eq!(value, expected);
                    }
                }
            }
            assert_eq!(fs::read_to_string(path).unwrap(), xml);
        }
    }
}

#[test]
fn standard_attribute_projection_keeps_the_closed_property_profile_and_byte_limit() {
    let at = QualifiedAddress::parse("main:Catalog.Items.StandardAttribute.Code").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = project_typed_payload(&route, json!({
        "collections": {},
        "declarations": {"standardAttributes": [{"name":"Code", "properties":[
            {"name":"FillChecking", "value":{"kind":"text", "value":"ShowError"}},
            {"name":"ToolTip", "value":{"kind":"localizedString", "values":[{"language":"ru", "content":"я".repeat(2048)}]}},
            {"name":"FutureProperty", "value":{"kind":"text", "value":"must-not-leak"}}
        ]}]}
    })).unwrap();
    let data = serde_json::to_value(view).unwrap();
    assert_eq!(data["title"], "Code");
    assert_eq!(data["props"].as_object().unwrap().len(), 1);
    assert_eq!(
        data["props"]["FillChecking"],
        r#"{"kind":"text","value":"ShowError"}"#
    );

    let at = QualifiedAddress::parse("main:Catalog.Items.Attribute.Code").unwrap();
    let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
    let view = project_typed_payload(&route, json!({
        "collections":{"attributes":[{"name":"Code", "properties":[{"name":"FillChecking", "value":{"kind":"text","value":"ShowError"}}]}]}
    })).unwrap();
    let data = serde_json::to_value(view).unwrap();
    assert!(data
        .get("props")
        .is_none_or(|props| props.as_object().unwrap().is_empty()));
}

#[test]
fn metadata_type_compact_limit_counts_utf8_bytes_and_explains_only_oversize() {
    for bytes in [2047, 2048, 2049] {
        let mut value = json!({"variants":[{"kind":"reference","metadataPath":"Document.Тип"}],"mutationCapability":"editable"});
        let padding = bytes - serde_json::to_string(&value).unwrap().len();
        value["variants"][0]["metadataPath"] = json!(format!(
            "Document.Тип{}{}",
            "я".repeat(padding / 2),
            "x".repeat(padding % 2)
        ));
        let rendered = serde_json::to_string(&value).unwrap();
        assert_eq!(rendered.len(), bytes);
        assert!(rendered.chars().count() < bytes);
        let payload = json!({"collections":{"attributes":[{"name":"Long","synonym":"Длинный тип","required":false,"type":value}]}});
        for (at, collection) in [
            ("main:Catalog.Items.Attribute", true),
            ("main:Catalog.Items.Attribute.Long", false),
        ] {
            let address = QualifiedAddress::parse(at).unwrap();
            let route = route_logical_address(&address, PlatformProfile::v8_3_27()).unwrap();
            let projected =
                serde_json::to_value(project_typed_payload(&route, payload.clone()).unwrap())
                    .unwrap();
            let item = if collection {
                &projected["items"][0]
            } else {
                &projected
            };
            assert_eq!(item["props"]["required"], false);
            assert_eq!(item["props"]["synonym"], "Длинный тип");
            assert!(item.get("branches").is_none());
            if bytes <= 2048 {
                assert_eq!(item["props"]["type"], rendered);
                assert!(item.get("limits").is_none());
            } else {
                assert!(item["props"].get("type").is_none());
                assert!(item["limits"].as_array().unwrap().iter().any(|limit| limit
                    .as_str()
                    .unwrap()
                    .contains("type")
                    && limit.as_str().unwrap().contains("2048")));
            }
        }
    }
    for ty in [None, Some(serde_json::Value::Null)] {
        let mut item = json!({"name":"Long","required":false});
        if let Some(value) = ty {
            item["type"] = value;
        }
        let at = QualifiedAddress::parse("main:Catalog.Items.Attribute.Long").unwrap();
        let route = route_logical_address(&at, PlatformProfile::v8_3_27()).unwrap();
        let projected = serde_json::to_value(
            project_typed_payload(&route, json!({"collections":{"attributes":[item]}})).unwrap(),
        )
        .unwrap();
        assert!(projected.get("limits").is_none());
    }
}

#[test]
fn metadata_long_type_read_preserves_other_facts_and_names_the_size_limit() {
    let fixture = RealReaderFixture::new();
    for relative in ["Configuration.xml", "AccumulationRegisters/Movements.xml"] {
        write(
            &fixture.source.join(relative),
            &fixture_text(&format!("acceptance/workspace-type-size/src/{relative}")),
        );
    }
    let xml =
        fixture_text("acceptance/workspace-type-size/src/AccumulationRegisters/Movements.xml");
    let source = roxmltree::Document::parse(&xml).unwrap();
    let references = source
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "Attribute")
        .flat_map(|node| node.descendants())
        .filter(|node| {
            node.is_element()
                && node.tag_name().name() == "Type"
                && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
        })
        .map(|node| node.text().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(references.len(), 20);
    for reference in &references {
        let name = reference.strip_prefix("cfg:DocumentRef.").unwrap();
        let relative = format!("Documents/{name}.xml");
        write(
            &fixture.source.join(&relative),
            &fixture_text(&format!("acceptance/workspace-type-size/src/{relative}")),
        );
    }
    assert_eq!(
        references
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        20
    );
    let service = fixture.view_service();
    let collection =
        service.view(ViewRequest::new("main:AccumulationRegister.Movements.Attribute").unwrap());
    assert!(collection.ok, "{collection:?}");
    let item = &collection.data.as_ref().unwrap()["items"][0];
    let detail = service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
    assert!(detail.ok, "{detail:?}");
    let data = detail.data.unwrap();
    assert_eq!(item["limits"], data["limits"]);
    assert!(data["limits"].as_array().unwrap().iter().any(|limit| limit
        .as_str()
        .unwrap()
        .contains("type")
        && limit.as_str().unwrap().contains("2048")));
    assert!(data["props"].get("type").is_none());
    assert_eq!(data["props"]["required"], false);
    assert_eq!(data["props"]["synonym"], "Long");
    let control = service
        .view(ViewRequest::new("main:AccumulationRegister.Movements.Dimension.Small").unwrap());
    assert!(control.ok, "{control:?}");
    let control = control.data.unwrap();
    assert!(control.get("limits").is_none());
    let ty: serde_json::Value =
        serde_json::from_str(control["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(ty["variants"].as_array().unwrap().len(), 9);
    for (variant, expected) in ty["variants"]
        .as_array()
        .unwrap()
        .iter()
        .zip(references.iter())
    {
        assert_eq!(
            variant["metadataPath"],
            expected.replace("cfg:DocumentRef.", "Document.")
        );
    }
    assert_eq!(
        fs::read_to_string(fixture.source.join("AccumulationRegisters/Movements.xml")).unwrap(),
        xml
    );
}

#[test]
fn metadata_missing_empty_and_malformed_types_do_not_claim_a_size_limit() {
    for (fragment, damaged) in [
        ("", false),
        ("<Type/>", true),
        ("<Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>broken</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>", true),
    ] {
        let fixture = RealReaderFixture::new();
        let path = fixture.source.join("Catalogs/Items.xml");
        let original = fs::read_to_string(&path).unwrap();
        let xml = original.replace("<Type><v8:Type>xs:string</v8:Type></Type>", fragment);
        assert_ne!(xml, original, "fixture must replace the existing type");
        write(&path, &xml);
        let service = fixture.view_service();
        let detail = service.view(ViewRequest::new("main:Catalog.Items.Attribute.Code").unwrap());
        assert!(detail.ok, "{detail:?}");
        let data = detail.data.unwrap();
        assert!(data["props"].get("type").is_none(), "{data}");
        // Отсутствующий, пустой и повреждённый тип не заявляют предел размера;
        // причина повреждения названа у самого реквизита (#1933).
        let item_limits = data["limits"].as_array().cloned().unwrap_or_default();
        assert!(
            item_limits.iter().all(|limit| {
                let limit = limit.as_str().unwrap();
                !limit.contains("2048") && !limit.contains("compact property limit")
            }),
            "a type limit of the attribute claims a size bound"
        );
        assert_eq!(
            item_limits
                .iter()
                .any(|limit| limit.as_str().unwrap().starts_with("collections.attributes[0].type: ")),
            damaged,
            "the attribute limits name the type damage exactly when the type is damaged"
        );
        assert_eq!(data["props"].get("incomplete").and_then(serde_json::Value::as_bool).unwrap_or(false), damaged);
        let root = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
        assert!(root.ok, "{root:?}");
        let limits = root.data.unwrap()["limits"].as_array().cloned().unwrap_or_default();
        assert_eq!(limits.iter().any(|limit| limit.as_str().unwrap().contains(".type не прочитано")), damaged, "{limits:?}");
    }
}

#[test]
fn predefined_long_type_read_preserves_other_facts_and_names_the_size_limit() {
    const FOLDER: &str = "10000000-0000-4000-8000-000000000101";
    const LONG: &str = "10000000-0000-4000-8000-000000000102";
    const SMALL: &str = "10000000-0000-4000-8000-000000000103";
    let fixture = RealReaderFixture::new();
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(
        &config_path,
        &config.replace(
            "</ChildObjects>",
            "<ChartOfCharacteristicTypes>Characteristics</ChartOfCharacteristicTypes></ChildObjects>",
        ),
    );
    write_metadata_owner(
        &fixture.source,
        "ChartsOfCharacteristicTypes",
        "ChartOfCharacteristicTypes",
        "Characteristics",
        "",
    );
    let type_xml = |references: &[String]| {
        references
            .iter()
            .map(|reference| format!("<v8:Type xmlns:d4p1=\"http://v8.1c.ru/8.1/data/enterprise/current-config\">d4p1:{reference}</v8:Type>"))
            .collect::<String>()
    };
    let long = (1..=39)
        .map(|index| format!("DocumentRef.ДокументРасчетовСКонтрагентомНомер{index:02}"))
        .collect::<Vec<_>>();
    let small = vec!["DocumentRef.ДокументРасчетовСКонтрагентомНомер01".to_string()];
    let path = fixture
        .source
        .join("ChartsOfCharacteristicTypes/Characteristics/Ext/Predefined.xml");
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"PlanOfCharacteristicKindPredefinedItems\" version=\"2.20\"><Item id=\"{FOLDER}\"><Name>Группа</Name><Code>000000001</Code><Description>Группа</Description><IsFolder>true</IsFolder><ChildItems><Item id=\"{LONG}\"><Name>Длинный</Name><Code>000000002</Code><Description>Длинный тип</Description><Type>{}</Type><IsFolder>false</IsFolder></Item><Item id=\"{SMALL}\"><Name>Малый</Name><Code>000000003</Code><Description>Малый тип</Description><Type>{}</Type><IsFolder>false</IsFolder></Item></ChildItems></Item></PredefinedData>",
        type_xml(&long),
        type_xml(&small),
    );
    write(&path, &xml);
    let source = roxmltree::Document::parse(&xml).unwrap();
    let variants = |name: &str| {
        source
            .descendants()
            .find(|node| {
                node.is_element()
                    && node.tag_name().name() == "Item"
                    && node.children().any(|child| {
                        child.tag_name().name() == "Name" && child.text() == Some(name)
                    })
            })
            .unwrap()
            .descendants()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == "Type"
                    && node.tag_name().namespace() == Some("http://v8.1c.ru/8.1/data/core")
            })
            .map(|node| {
                node.text()
                    .unwrap()
                    .replace("d4p1:DocumentRef.", "Document.")
            })
            .collect::<Vec<_>>()
    };
    let expected_long = variants("Длинный");
    assert_eq!(expected_long.len(), 39);
    // Независимая оценка компактного вида: одни пути вариантов уже больше предела.
    assert!(expected_long.iter().map(String::len).sum::<usize>() > 2048);
    let expected_small = variants("Малый");
    assert_eq!(expected_small.len(), 1);

    let service = fixture.view_service();
    let collection = service.view(
        ViewRequest::new("main:ChartOfCharacteristicTypes.Characteristics.PredefinedItem").unwrap(),
    );
    assert!(collection.ok, "{collection:?}");
    let items = collection.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .clone();
    let find = |name: &str| {
        items
            .iter()
            .find(|item| item["at"].as_str().unwrap().ends_with(&format!(".{name}")))
            .unwrap_or_else(|| panic!("{name} in {items:?}"))
            .clone()
    };
    let size_limit = |limits: &serde_json::Value| {
        limits.as_array().unwrap().iter().any(|limit| {
            let limit = limit.as_str().unwrap();
            limit.contains("type") && limit.contains("2048")
        })
    };
    let item = find("Длинный");
    let detail = service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
    assert!(detail.ok, "{detail:?}");
    let data = detail.data.unwrap();
    assert_eq!(item["limits"], data["limits"]);
    assert!(size_limit(&data["limits"]), "{data}");
    assert!(data["props"].get("type").is_none(), "{data}");
    assert!(data.get("branches").is_none(), "{data}");
    assert_eq!(data["props"]["id"], LONG);
    assert_eq!(data["props"]["parentId"], FOLDER);
    assert_eq!(data["props"]["code"], "000000002");
    assert_eq!(data["props"]["description"], "Длинный тип");
    assert_eq!(data["props"]["isFolder"], false);

    let item = find("Малый");
    assert!(item.get("limits").is_none(), "{item}");
    let control = service.view(ViewRequest::new(item["at"].as_str().unwrap()).unwrap());
    assert!(control.ok, "{control:?}");
    let control = control.data.unwrap();
    assert!(control.get("limits").is_none(), "{control}");
    assert_eq!(control["props"]["id"], SMALL);
    assert_eq!(control["props"]["parentId"], FOLDER);
    assert_eq!(control["props"]["code"], "000000003");
    assert_eq!(control["props"]["description"], "Малый тип");
    assert_eq!(control["props"]["isFolder"], false);
    let ty: serde_json::Value =
        serde_json::from_str(control["props"]["type"].as_str().unwrap()).unwrap();
    assert_eq!(
        ty,
        json!(unica_metadata::MetadataType::new(
            expected_small
                .iter()
                .map(|path| unica_metadata::MetadataTypeVariant::Reference {
                    metadata_path: MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, path)
                        .unwrap(),
                })
                .collect()
        )
        .unwrap())
    );

    let folder = find("Группа");
    assert!(folder.get("limits").is_none(), "{folder}");
    assert_eq!(fs::read_to_string(&path).unwrap(), xml);
}

/// Три роли дампа `PrintWebDAV`: заимствован с перекрытием, заимствован без
/// перекрытия, собственный объект расширения.
///
/// Признак заимствования — `ExtendedConfigurationObject`, а не `Adopted`:
/// корень `Configuration` самого расширения тоже `Adopted`, и читать его как
/// заимствованный объект — дефект. `xr:PropertyState` несёт **список**
/// перекрытых свойств, а не флаг: заимствованный объект без перекрытий его не
/// несёт вовсе.
#[test]
fn borrowing_props_name_the_three_roles_of_an_extension_source_set() {
    let fixture = RealReaderFixture::new();
    fixture.borrow_catalog("Catalogs/Владельцы.xml", Some("Synonym"));
    fixture.borrow_catalog("Documents/Order.xml", None);
    let service = fixture.extension_view_service();

    let overridden = service.view(ViewRequest::new("main:Catalog.Владельцы").unwrap());
    assert!(overridden.ok, "{:?}", refusal_codes(&overridden));
    let props = &overridden.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("borrowed"));
    assert_eq!(
        props["parentId"],
        json!("11111111-1111-4111-8111-111111111111")
    );
    assert_eq!(props["overrides"], json!("Synonym"));
    assert!(
        props.get("extends").is_none(),
        "UUID is not a readable parent address"
    );
    assert_eq!(props["parentStatus"], json!("unavailable"));

    let plain = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(plain.ok, "{:?}", refusal_codes(&plain));
    let props = &plain.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("borrowed"));
    assert_eq!(
        props["parentId"],
        json!("11111111-1111-4111-8111-111111111111")
    );
    assert!(
        props.get("overrides").is_none(),
        "заимствование без перекрытий ключа `overrides` не несёт: пустой список — это его отсутствие"
    );

    let own = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(own.ok, "{:?}", refusal_codes(&own));
    let props = &own.data.as_ref().unwrap()["props"];
    assert_eq!(props["belonging"], json!("own"));
    assert!(props.get("extends").is_none());
    assert!(props.get("overrides").is_none());
}

/// Заимствование не стоит второго чтения дескриптора.
///
/// Свойства объекта и его заимствование разбираются из одних и тех же байтов.
/// Второе чтение того же файла стоило бы лишнего доступа и, что хуже, могло бы
/// застать файл изменившимся между чтениями — тогда два ответа об одном
/// объекте описывали бы разные состояния источника.
#[test]
fn borrowing_costs_no_second_descriptor_read() {
    let fixture = RealReaderFixture::new();
    fixture.borrow_catalog("Catalogs/Владельцы.xml", Some("Synonym"));
    let authority = fixture.extension_read_authority();

    assert_reader_reaches(&authority, &["main:Catalog.Владельцы"]);

    assert_eq!(
        authority.metadata_descriptor_read_count("Catalog.Владельцы"),
        2,
        "заимствование читается тем же дескриптором, что и свойства объекта: \
         доказательство владельца плюс типизированная проекция, и ни чтением больше",
    );
}

/// В наборе вида `configuration` заимствования не бывает, и `belonging: own`
/// у каждого объекта было бы шумом.
#[test]
fn a_configuration_source_set_carries_no_borrowing_props() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(ViewRequest::new("main:Catalog.Items").unwrap());

    assert!(result.ok, "{:?}", refusal_codes(&result));
    let props = &result.data.as_ref().unwrap()["props"];
    for key in ["belonging", "extends", "overrides"] {
        assert!(props.get(key).is_none(), "{key} в конфигурации не отвечает");
    }
}

#[test]
fn metadata_attribute_fill_value_is_a_bounded_observed_fact() {
    let fixture = RealReaderFixture::new();
    let descriptor = fixture.source.join("Catalogs/Items.xml");
    let original = fs::read_to_string(&descriptor).unwrap();
    let document = roxmltree::Document::parse(&original).unwrap();
    let range = document
        .root_element()
        .first_element_child()
        .unwrap()
        .children()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "ChildObjects")))
        .unwrap()
        .range();
    let attributes = r#"<Attribute xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Properties><Name>Empty</Name><Type><v8:Type>cfg:CatalogRef.Items</v8:Type></Type><FillValue xsi:type="xr:DesignTimeRef">Catalog.Items.EmptyRef</FillValue></Properties></Attribute><Attribute xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><Properties><Name>Flag</Name><Type><v8:Type>cfg:EnumRef.Flags</v8:Type></Type><FillValue xsi:type="xr:DesignTimeRef">Enum.Flags.EnumValue.Yes</FillValue></Properties></Attribute>"#;
    write(
        &descriptor,
        &format!(
            "{}<ChildObjects>{attributes}</ChildObjects>{}",
            &original[..range.start],
            &original[range.end..]
        ),
    );
    let service = fixture.view_service();
    for (name, owner, item) in [
        ("Empty", "Catalog.Items", "EmptyRef"),
        ("Flag", "Enum.Flags", "EnumValue.Yes"),
    ] {
        let result = service
            .view(ViewRequest::new(&format!("main:Catalog.Items.Attribute.{name}")).unwrap());
        assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
        let props = &result.data.as_ref().unwrap()["props"];
        assert!(props.get("incomplete").is_none(), "{props}");
        let observed: serde_json::Value =
            serde_json::from_str(props["fillValue"].as_str().unwrap()).unwrap();
        assert_eq!(
            observed,
            json!({"kind": "reference", "metadataPath": owner, "item": item})
        );
    }
}

#[test]
fn metadata_node_props_lay_out_the_per_kind_facts_by_role() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Обработчик регламентного задания: два поля, две роли.
    let job = service.view(ViewRequest::new("main:ScheduledJob.MonthClose").unwrap());
    assert!(job.ok, "{:?}", refusal_codes(&job));
    let props = &job.data.as_ref().unwrap()["props"];
    assert_eq!(props["handlerModule"], json!("CommonModule.MonthClose"));
    assert_eq!(props["handlerMethod"], json!("RunScheduled"));

    // Расписание регистра расчёта: три поля, три роли.
    let register = service.view(ViewRequest::new("main:CalculationRegister.Payroll").unwrap());
    assert!(register.ok, "{:?}", refusal_codes(&register));
    let props = &register.data.as_ref().unwrap()["props"];
    assert_eq!(
        props["scheduleRegister"],
        json!("InformationRegister.WorkSchedules")
    );
    assert_eq!(
        props["scheduleValueField"],
        json!("InformationRegister.WorkSchedules.Resource.DayValue")
    );
    assert_eq!(
        props["scheduleDateField"],
        json!("InformationRegister.WorkSchedules.Dimension.Date")
    );

    // Тип константы: одно составное значение, компактной строкой — тем же
    // механизмом, каким отвечает тип реквизита.
    let constant = service.view(ViewRequest::new("main:Constant.MainCurrency").unwrap());
    assert!(constant.ok, "{:?}", refusal_codes(&constant));
    let rendered = constant.data.as_ref().unwrap()["props"]["type"]
        .as_str()
        .expect("constant type renders compactly");
    assert!(rendered.contains("string"), "{rendered}");
}

/// Сервис в том виде, в каком его выгружает Конфигуратор 8.3.27: у шаблона
/// и метода есть `Synonym` и `<Comment/>` (#634).
fn install_configurator_http_service(fixture: &RealReaderFixture, edit: impl Fn(&str) -> String) {
    let xml = fixture_text("platform_8_3_27/meta_info/edge/http-service-configurator.xml")
        .replace("<Name>ПроверкаСвязи</Name>", "<Name>ExternalAPI</Name>");
    write(
        &fixture.source.join("HTTPServices/ExternalAPI.xml"),
        &edit(&xml),
    );
}

#[test]
fn configurator_http_service_view_lists_its_templates_and_methods() {
    let fixture = RealReaderFixture::new();
    install_configurator_http_service(&fixture, str::to_string);
    let service = fixture.view_service();

    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let node = http.data.as_ref().unwrap();
    assert!(
        node["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": "main:HTTPService.ExternalAPI.URLTemplate", "count": 2})),
        "{node:#}"
    );
    assert!(node.get("limits").is_none(), "{node:#}");

    let templates =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate").unwrap());
    assert!(templates.ok, "{:?}", refusal_codes(&templates));
    let names = templates.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["at"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "main:HTTPService.ExternalAPI.URLTemplate.Пинг",
            "main:HTTPService.ExternalAPI.URLTemplate.ПринятьДанные",
        ]
    );

    let method = service.view(
        ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.ПринятьДанные.Method.POST")
            .unwrap(),
    );
    assert!(method.ok, "{:?}", refusal_codes(&method));
    let method = method.data.as_ref().unwrap();
    assert_eq!(method["props"]["httpMethod"], json!("POST"));
    assert_eq!(method["props"]["handler"], json!("ПринятьДанныеPOST"));
}

/// Веб-сервис в том виде, в каком его выгружает Конфигуратор 8.3.27:
/// у операции есть `Synonym`, `<Comment/>` и `DataLockControlMode`,
/// у параметра — `Synonym` и `<Comment/>`.
#[test]
fn configurator_web_service_view_opens_its_operations_and_parameters() {
    let fixture = RealReaderFixture::new();
    write(
        &fixture.source.join("WebServices/Exchange.xml"),
        &fixture_text("platform_8_3_27/meta_info/edge/web-service-configurator.xml")
            .replace("<Name>ОбменДанными</Name>", "<Name>Exchange</Name>"),
    );
    let service = fixture.view_service();

    let web = service.view(ViewRequest::new("main:WebService.Exchange").unwrap());
    assert!(web.ok, "{:?}", refusal_codes(&web));
    let node = web.data.as_ref().unwrap();
    assert!(
        node["branches"]
            .as_array()
            .unwrap()
            .contains(&json!({"at": "main:WebService.Exchange.Operation", "count": 2})),
        "{node:#}"
    );
    assert!(node.get("limits").is_none(), "{node:#}");

    let operation =
        service.view(ViewRequest::new("main:WebService.Exchange.Operation.ПринятьПакет").unwrap());
    assert!(operation.ok, "{:?}", refusal_codes(&operation));
    let operation = operation.data.as_ref().unwrap();
    assert_eq!(operation["props"]["procedure"], json!("ПринятьПакет"));
    assert_eq!(
        operation["branches"],
        json!([{"at": "main:WebService.Exchange.Operation.ПринятьПакет.Parameter", "count": 3}]),
    );

    let parameter = service.view(
        ViewRequest::new("main:WebService.Exchange.Operation.ПринятьПакет.Parameter.Ошибка")
            .unwrap(),
    );
    assert!(parameter.ok, "{:?}", refusal_codes(&parameter));
    assert_eq!(
        parameter.data.as_ref().unwrap()["props"]["direction"],
        json!("out")
    );
}

/// Непрочитанный шаблон не выдаётся за пустой сервис: узел называет поле
/// в `limits`, а ветвь шаблонов отказывает с причиной вместо «нет такой».
#[test]
fn an_unreadable_http_template_is_named_rather_than_reported_absent() {
    let fixture = RealReaderFixture::new();
    install_configurator_http_service(&fixture, |xml| {
        xml.replacen("<Name>Пинг</Name>", "<Name>bad name</Name>", 1)
    });
    let service = fixture.view_service();

    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let node = http.data.as_ref().unwrap();
    let limits = node["limits"]
        .as_array()
        .expect("the unread field is named");
    assert!(
        limits.iter().any(|limit| limit
            .as_str()
            .is_some_and(|limit| limit.starts_with("details.urlTemplates[0].name"))),
        "{node:#}"
    );

    let templates =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate").unwrap());
    assert!(!templates.ok, "{:#?}", templates.data);
    assert_eq!(refusal_codes(&templates), ["provider_unavailable"]);
    let message = templates.diagnostics[0]["message"].as_str().unwrap();
    assert!(
        message.contains("details.urlTemplates[0].name"),
        "{message}"
    );
}

#[test]
fn declared_service_kinds_finally_get_their_subject() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Виды `URLTemplate` и `Operation` были объявлены в грамматике и не
    // подключены ни к чему; здесь у них появляется предмет.
    let http = service.view(ViewRequest::new("main:HTTPService.ExternalAPI").unwrap());
    assert!(http.ok, "{:?}", refusal_codes(&http));
    let branches = http.data.as_ref().unwrap()["branches"].as_array().unwrap();
    assert!(
        branches.contains(&json!({
            "at": "main:HTTPService.ExternalAPI.URLTemplate",
            "count": 1
        })),
        "{branches:#?}"
    );

    let template =
        service.view(ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics").unwrap());
    assert!(template.ok, "{:?}", refusal_codes(&template));
    let template = template.data.as_ref().unwrap();
    assert_eq!(template["props"]["template"], json!("/v1/kpi/"));
    // Последовательность методов — ветвь, а не строка в `props`: коллекция
    // обязана быть адресуемой.
    assert_eq!(
        template["branches"],
        json!([{
            "at": "main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method",
            "count": 1
        }]),
    );

    let method = service.view(
        ViewRequest::new("main:HTTPService.ExternalAPI.URLTemplate.Metrics.Method.Get").unwrap(),
    );
    assert!(method.ok, "{:?}", refusal_codes(&method));
    let method = method.data.as_ref().unwrap();
    assert_eq!(method["props"]["httpMethod"], json!("GET"));
    assert_eq!(method["props"]["handler"], json!("MetricsGet"));

    let operation =
        service.view(ViewRequest::new("main:WebService.Exchange.Operation.Send").unwrap());
    assert!(operation.ok, "{:?}", refusal_codes(&operation));
    let operation = operation.data.as_ref().unwrap();
    assert_eq!(operation["props"]["procedure"], json!("SendData"));
    assert_eq!(operation["props"]["transactioned"], json!(true));
    assert!(
        operation["props"]["returnType"]
            .as_str()
            .is_some_and(|rendered| rendered.contains("boolean")),
        "{operation:#}"
    );
    assert_eq!(
        operation["branches"],
        json!([{"at": "main:WebService.Exchange.Operation.Send.Parameter", "count": 1}]),
    );

    let parameter = service.view(
        ViewRequest::new("main:WebService.Exchange.Operation.Send.Parameter.Payload").unwrap(),
    );
    assert!(parameter.ok, "{:?}", refusal_codes(&parameter));
    assert_eq!(
        parameter.data.as_ref().unwrap()["props"]["direction"],
        json!("in")
    );
}

#[test]
fn every_reference_of_an_object_answers_in_one_relation_branch() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let document = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(document.ok, "{:?}", refusal_codes(&document));
    let branches = document.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap();
    // Платформа держит движение и основание в разных местах, но вопрос один:
    // на что объект показывает. Одна ветвь, имя связи — поле элемента.
    assert!(
        branches.contains(&json!({"at": "main:Document.Order.Relation", "count": 2})),
        "{branches:#?}"
    );

    let relations = service.view(ViewRequest::new("main:Document.Order.Relation").unwrap());
    assert!(relations.ok, "{:?}", refusal_codes(&relations));
    assert_eq!(
        relations.data.as_ref().unwrap()["items"],
        json!([
            {
                "relation": "registerRecord",
                "at": "main:CalculationRegister.Payroll",
                "kind": "CalculationRegister"
            },
            {
                "relation": "basedOn",
                "at": "main:Catalog.Items",
                "kind": "Catalog"
            }
        ]),
    );

    // Правило обещает всякую ссылку, поэтому проверяются все её источники, а
    // не те два, что лежали ближе. `relations` и пофактовая часть вида
    // приходят из разных мест платформы и должны сойтись в одной ветви.
    let sources = |at: &str| -> Vec<(String, String)> {
        let node = service.view(ViewRequest::new(at).unwrap());
        assert!(node.ok, "{at}: {:?}", refusal_codes(&node));
        node.data.as_ref().unwrap()["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| {
                (
                    item["relation"].as_str().unwrap().to_string(),
                    item["at"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    };
    assert_eq!(
        sources("main:Catalog.Items.Relation"),
        [
            ("owner".to_string(), "main:Catalog.Владельцы".to_string()),
            (
                "inputByString".to_string(),
                "main:Catalog.Items.Attribute.Code".to_string()
            ),
            (
                "dataLockField".to_string(),
                "main:Catalog.Items.Attribute.Code".to_string()
            ),
        ]
    );
    assert_eq!(
        sources("main:DocumentJournal.Журнал.Relation"),
        [(
            "registeredDocument".to_string(),
            "main:Document.Order".to_string()
        )]
    );
    assert_eq!(
        sources("main:ChartOfCalculationTypes.ВидыРасчета.Relation"),
        [(
            "baseCalculationType".to_string(),
            "main:ChartOfCalculationTypes.ВидыРасчета".to_string()
        )]
    );
    assert_eq!(
        sources("main:EventSubscription.ПриЗаписи.Relation"),
        [("source".to_string(), "main:Catalog.Items".to_string())]
    );
    // Пакет XDTO бывает ссылкой и бывает пространством имён; в ветвь попадает
    // только адресуемый вариант, и это названная граница, а не потеря.
    assert_eq!(
        sources("main:WebService.Exchange.Relation"),
        [(
            "xdtoPackage".to_string(),
            "main:XDTOPackage.Exchange".to_string()
        )]
    );

    // Элемент ветви показывает наружу, поэтому спускаться в него нельзя:
    // цель читается по своему адресу.
    let inward = service.view(ViewRequest::new("main:Document.Order.Relation.Items").unwrap());
    assert!(!inward.ok, "{:?}", refusal_codes(&inward));
    assert_eq!(refusal_codes(&inward), ["not_found"]);
}

#[test]
fn root_declarations_get_branches_and_only_the_named_one_is_addressable() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    // Стандартная табличная часть носит имя, поэтому адресуется, а её
    // собственная последовательность становится вложенной ветвью.
    let section = service.view(
        ViewRequest::new("main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes")
            .unwrap(),
    );
    assert!(section.ok, "{:?}", refusal_codes(&section));
    let section = section.data.as_ref().unwrap();
    assert_eq!(section["kind"], "StandardTabularSection");
    assert_eq!(
        section["branches"],
        json!([{
            "at": "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute",
            "count": 1
        }]),
    );

    let attribute = service.view(
        ViewRequest::new(
            "main:ChartOfAccounts.Accounts.StandardTabularSection.ExtDimensionTypes.StandardAttribute.ExtDimensionType",
        )
        .unwrap(),
    );
    assert!(attribute.ok, "{:?}", refusal_codes(&attribute));
    assert_eq!(
        attribute.data.as_ref().unwrap()["title"],
        "ExtDimensionType"
    );

    // Характеристика имени не носит: ветвь адресуема, а её элементы —
    // строки данных, и адреса у них нет.
    let characteristics =
        service.view(ViewRequest::new("main:Catalog.Items.Characteristic").unwrap());
    assert!(characteristics.ok, "{:?}", refusal_codes(&characteristics));
    let items = characteristics.data.as_ref().unwrap()["items"]
        .as_array()
        .unwrap();
    assert_eq!(items.len(), 1);
    assert!(items[0].get("at").is_none(), "{items:#?}");
    assert_eq!(items[0]["values"]["source"], "Catalog.Items");

    let named = service.view(ViewRequest::new("main:Catalog.Items.Characteristic.Any").unwrap());
    assert!(!named.ok, "{:?}", refusal_codes(&named));
    assert_eq!(refusal_codes(&named), ["not_found"]);
}

#[test]
fn predefined_items_beyond_first_page_answer_by_address() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{owner:?}");
    assert!(owner.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap()
        .contains(&json!({"at": "main:Catalog.Items.PredefinedItem", "count": 72})));
    for index in [1, 16, 17, 19, 21, 50, 51, 53, 54, 55, 72] {
        let result = service.view(
            ViewRequest::new(&format!("main:Catalog.Items.PredefinedItem.Item{index:03}")).unwrap(),
        );
        assert!(result.ok, "item {index}: {result:?}");
        let data = result.data.unwrap();
        assert_eq!(data["title"], format!("Item{index:03}"));
        assert_eq!(
            data["props"]["id"],
            format!("00000000-0000-4000-8000-{index:012}")
        );
        assert_eq!(data["props"]["code"], format!("{index:03}"));
        assert_eq!(data["props"]["description"], format!("Description {index}"));
        assert_eq!(data["props"]["isFolder"], predefined_page_is_folder(index));
        assert_eq!(
            data["props"]["parentId"],
            predefined_page_parent(index).map_or(json!(null), |parent| json!(format!(
                "00000000-0000-4000-8000-{parent:012}"
            )))
        );
    }
    let missing =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Absent").unwrap());
    assert_eq!(refusal_codes(&missing), ["not_found"]);
}

#[test]
fn predefined_items_pages_preserve_complete_document_order() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    for limit in [1, 17, 50] {
        let mut cursor = None;
        let mut names = Vec::new();
        loop {
            let mut request = ViewRequest::new("main:Catalog.Items.PredefinedItem")
                .unwrap()
                .with_limit(limit)
                .unwrap();
            if let Some(previous) = cursor.take() {
                request = request.with_cursor(previous);
            }
            let page = service.view(request);
            assert!(page.ok, "{page:?}");
            let data = page.data.unwrap();
            let items = data["items"].as_array().unwrap();
            assert!(!items.is_empty() && items.len() <= limit);
            names.extend(
                items
                    .iter()
                    .map(|item| item["at"].as_str().unwrap().to_owned()),
            );
            cursor = page.cursor;
            if cursor.is_none() {
                break;
            }
            assert!(names.len() < 72, "continuation must make progress");
        }
        let expected = (1..=72)
            .map(|index| format!("main:Catalog.Items.PredefinedItem.Item{index:03}"))
            .collect::<Vec<_>>();
        assert_eq!(
            names, expected,
            "limit {limit}: no missing or duplicate items"
        );
    }
}

/// Родитель элемента страничной фикстуры. Группы стоят в середине списка и
/// вложены в два уровня, а границы страниц 17 и 50 проходят внутри их
/// поддеревьев: обход в ширину переставил бы детей в хвост, и порядок
/// `Item001..Item072` перестал бы совпадать с документным.
fn predefined_page_parent(index: usize) -> Option<usize> {
    match index {
        17 | 18 | 21 => Some(16),
        19 | 20 => Some(18),
        51 | 52 | 54 => Some(50),
        53 => Some(52),
        _ => None,
    }
}

fn predefined_page_is_folder(index: usize) -> bool {
    matches!(index, 16 | 18 | 50 | 52)
}

fn push_predefined_page_item(items: &mut String, index: usize) {
    items.push_str(&format!(
        "<Item id=\"00000000-0000-4000-8000-{index:012}\"><Name>Item{index:03}</Name><Code>{index:03}</Code><Description>Description {index}</Description><IsFolder>{}</IsFolder>",
        predefined_page_is_folder(index),
    ));
    let children = (index + 1..=72)
        .filter(|child| predefined_page_parent(*child) == Some(index))
        .collect::<Vec<_>>();
    if !children.is_empty() {
        items.push_str("<ChildItems>");
        for child in children {
            push_predefined_page_item(items, child);
        }
        items.push_str("</ChildItems>");
    }
    items.push_str("</Item>");
}

fn predefined_page_fixture() -> RealReaderFixture {
    let fixture = RealReaderFixture::new();
    let mut items = String::new();
    for index in (1..=72).filter(|index| predefined_page_parent(*index).is_none()) {
        push_predefined_page_item(&mut items, index);
    }
    write(&fixture.source.join("Catalogs/Items/Ext/Predefined.xml"), &format!(
        "<PredefinedData xmlns=\"http://v8.1c.ru/8.3/xcf/predef\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"CatalogPredefinedItems\" version=\"2.20\">{items}</PredefinedData>"
    ));
    fixture
}

#[test]
fn predefined_items_empty_inapplicable_and_unreadable_are_distinct() {
    let fixture = RealReaderFixture::new();
    let path = fixture.source.join("Catalogs/Items/Ext/Predefined.xml");
    fs::remove_file(&path).unwrap();
    let service = fixture.view_service();
    let owner = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(owner.ok, "{owner:?}");
    let authority = fixture.read_authority();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let empty_data = authority
        .read
        .predefined_items(
            &target,
            unica_metadata::MetadataKind::Catalog,
            Some("String"),
            &mut || Ok(()),
        )
        .unwrap()
        .unwrap();
    assert_eq!(empty_data.total, 0);
    assert!(!empty_data.truncated);
    let empty = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(empty.ok, "{empty:?}");
    assert_eq!(empty.data.unwrap()["items"], json!([]));
    assert!(empty.cursor.is_none());
    let absent =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Absent").unwrap());
    assert_eq!(refusal_codes(&absent), ["not_found"]);
    let inapplicable =
        service.view(ViewRequest::new("main:Document.Order.PredefinedItem").unwrap());
    // Неприменимому виду отказано в самой коллекции, а не в элементе:
    // пустой список он не получает.
    assert!(!inapplicable.ok, "{inapplicable:?}");
    assert_eq!(refusal_codes(&inapplicable), ["not_found"]);
    assert_eq!(
        inapplicable.diagnostics[0]["message"],
        "metadata has no PredefinedItem collection"
    );
    assert!(inapplicable.cursor.is_none());

    write(&path, "<PredefinedData><broken>");
    let unreadable = fixture
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(!unreadable.ok, "{unreadable:?}");
    assert_eq!(refusal_codes(&unreadable), ["provider_unavailable"]);
    assert_eq!(unreadable.diagnostics[0]["detailCode"], "source_unreadable");
    assert!(unreadable.cursor.is_none());
}

#[test]
fn predefined_items_cursor_replays_after_source_removal() {
    let fixture = predefined_page_fixture();
    let service = fixture.view_service();
    let request = || {
        ViewRequest::new("main:Catalog.Items.PredefinedItem")
            .unwrap()
            .with_limit(50)
            .unwrap()
    };
    let first = service.view(request());
    assert!(first.ok, "{first:?}");
    let cursor = first.cursor.expect("72 elements need a continuation");
    let before = service.view(request().with_cursor(cursor.clone()));
    assert!(before.ok, "{before:?}");
    fs::remove_file(fixture.source.join("Catalogs/Items/Ext/Predefined.xml")).unwrap();
    let after = service.view(request().with_cursor(cursor.clone()));
    assert!(after.ok, "{after:?}");
    assert_eq!(before.data, after.data);
    assert_eq!(before.cursor, after.cursor);
    let items = after.data.unwrap()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 22);
    assert_eq!(items[0]["at"], "main:Catalog.Items.PredefinedItem.Item051");
    assert_eq!(items[21]["at"], "main:Catalog.Items.PredefinedItem.Item072");
    let tampered = service.view(request().with_cursor(format!("{cursor}x")));
    assert_eq!(refusal_codes(&tampered), ["invalid_cursor"]);
    let fresh = fixture.view_service().view(request());
    assert!(fresh.ok, "{fresh:?}");
    assert_eq!(fresh.data.unwrap()["items"], json!([]));
}

#[test]
fn predefined_items_refuse_file_and_snapshot_budget_without_cursor() {
    let fixture = predefined_page_fixture();
    let service = ViewService::new(fixture.read_authority(), ViewCursorStore::new(512));
    let result = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert_eq!(refusal_codes(&result), ["result_too_large"]);
    assert!(result.cursor.is_none());

    write(
        &fixture.source.join("Catalogs/Items/Ext/Predefined.xml"),
        &"x".repeat(8 * 1024 * 1024 + 1),
    );
    let result = fixture
        .view_service()
        .view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(!result.ok, "{result:?}");
    assert!(
        serde_json::to_string(&result.diagnostics)
            .unwrap()
            .contains("8388608-byte read limit"),
        "{result:?}"
    );
    assert!(result.cursor.is_none());
}

#[test]
fn predefined_items_checkpoint_preserves_cancellation_and_deadline() {
    use unica_metadata::MetadataKind;
    let fixture = predefined_page_fixture();
    let authority = fixture.read_authority();
    let target = MetadataAddress::parse(PLATFORM_XML_8_3_27_FORMAT_2_20, "Catalog.Items").unwrap();
    let mut completed_checkpoints = 0;
    let data = authority
        .read
        .predefined_items(&target, MetadataKind::Catalog, Some("String"), &mut || {
            completed_checkpoints += 1;
            Ok(())
        })
        .unwrap()
        .unwrap();
    assert_eq!(data.total, 72);
    assert!(completed_checkpoints > 2 * data.total);
    for code in [RefusalCode::Cancelled, RefusalCode::DeadlineExceeded] {
        for stop_at in [2, 16, completed_checkpoints - 10] {
            let mut checkpoints = 0;
            let error = authority
                .read
                .predefined_items(&target, MetadataKind::Catalog, Some("String"), &mut || {
                    checkpoints += 1;
                    if checkpoints == stop_at {
                        Err(ViewError::new(code, "interrupted predefined read"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.code(), code, "checkpoint {stop_at}");
            assert_eq!(checkpoints, stop_at);
        }
    }
}

#[test]
fn predefined_items_other_owner_fields_survive_addressed_reads() {
    use unica_metadata::{MetadataType, MetadataTypeVariant};
    let fixture = RealReaderFixture::new();
    let config_path = fixture.source.join("Configuration.xml");
    let config = fs::read_to_string(&config_path).unwrap();
    write(&config_path, &config.replace("</ChildObjects>",
        "<ChartOfCharacteristicTypes>Characteristics</ChartOfCharacteristicTypes><ChartOfCalculationTypes>Calculations</ChartOfCalculationTypes></ChildObjects>"));
    for (kind, directory, name) in [
        (
            "ChartOfCharacteristicTypes",
            "ChartsOfCharacteristicTypes",
            "Characteristics",
        ),
        (
            "ChartOfCalculationTypes",
            "ChartsOfCalculationTypes",
            "Calculations",
        ),
    ] {
        write_metadata_owner(&fixture.source, directory, kind, name, "");
        if kind == "ChartOfCalculationTypes" {
            let path = fixture.source.join(format!("{directory}/{name}.xml"));
            let xml = fs::read_to_string(&path).unwrap();
            write(
                &path,
                &xml.replace("</Properties>", "<CodeType>String</CodeType></Properties>"),
            );
        }
    }
    for (directory, name, fixture_file) in [
        ("ChartsOfAccounts", "Accounts", "ChartOfAccounts.xml"),
        (
            "ChartsOfCharacteristicTypes",
            "Characteristics",
            "ChartOfCharacteristicTypes.xml",
        ),
        (
            "ChartsOfCalculationTypes",
            "Calculations",
            "ChartOfCalculationTypes.xml",
        ),
    ] {
        write(
            &fixture
                .source
                .join(format!("{directory}/{name}/Ext/Predefined.xml")),
            &fixture_text(&format!("platform_8_3_27/predefined/{fixture_file}")),
        );
    }
    let service = fixture.view_service();
    let account = service.view(
        ViewRequest::new("main:ChartOfAccounts.Accounts.PredefinedItem.ОСвОрганизации").unwrap(),
    );
    assert!(account.ok, "{account:?}");
    let props = &account.data.as_ref().unwrap()["props"];
    assert_eq!(props["accountType"], "Active");
    assert_eq!(props["offBalance"], false);
    assert_eq!(props["order"], " 01.01");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(props["accountingFlags"].as_str().unwrap())
            .unwrap(),
        json!({
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.Валютный": false,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.Количественный": false,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.УчетПоПодразделениям": true,
            "ChartOfAccounts.Хозрасчетный.AccountingFlag.НалоговыйУчет": true,
        })
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(props["extDimensionTypes"].as_str().unwrap())
            .unwrap(),
        json!([{
            "name": "ChartOfCharacteristicTypes.ВидыСубконтоХозрасчетные.ОсновныеСредства",
            "turnover": false,
            "accountingFlags": {
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Суммовой": true,
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Валютный": true,
                "ChartOfAccounts.Хозрасчетный.ExtDimensionAccountingFlag.Количественный": true,
            },
        }])
    );
    let characteristic = service.view(ViewRequest::new("main:ChartOfCharacteristicTypes.Characteristics.PredefinedItem.УдалитьБухгалтерскийУчет").unwrap());
    assert!(characteristic.ok, "{characteristic:?}");
    assert_eq!(
        characteristic.data.as_ref().unwrap()["props"]["isFolder"],
        false
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            characteristic.data.as_ref().unwrap()["props"]["type"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
        json!(MetadataType::new(vec![MetadataTypeVariant::Reference {
            metadata_path: MetadataAddress::parse(
                PLATFORM_XML_8_3_27_FORMAT_2_20,
                "ChartOfCharacteristicTypes.РазделыДатЗапретаИзменения"
            )
            .unwrap(),
        }])
        .unwrap())
    );
    let calculation = service.view(
        ViewRequest::new("main:ChartOfCalculationTypes.Calculations.PredefinedItem.ОкладПоДням")
            .unwrap(),
    );
    assert!(calculation.ok, "{calculation:?}");
    assert_eq!(
        calculation.data.as_ref().unwrap()["props"]["actionPeriodIsBase"],
        false
    );
}

#[test]
fn predefined_items_answer_by_address_with_the_count_from_the_reader() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let node = service.view(ViewRequest::new("main:Catalog.Items").unwrap());
    assert!(node.ok, "{:?}", refusal_codes(&node));
    let branches = node.data.as_ref().unwrap()["branches"].as_array().unwrap();
    // Счёт пришёл из ответа читателя, а не из длины страницы.
    assert!(
        branches.contains(&json!({"at": "main:Catalog.Items.PredefinedItem", "count": 1})),
        "{branches:#?}"
    );

    let page = service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem").unwrap());
    assert!(page.ok, "{:?}", refusal_codes(&page));
    let items = page.data.as_ref().unwrap()["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["at"], "main:Catalog.Items.PredefinedItem.Основной");
    assert_eq!(items[0]["kind"], "PredefinedItem");

    let item =
        service.view(ViewRequest::new("main:Catalog.Items.PredefinedItem.Основной").unwrap());
    assert!(item.ok, "{:?}", refusal_codes(&item));
    let item = item.data.as_ref().unwrap();
    // Заголовок называет элемент его именем; представление — отдельный факт.
    assert_eq!(item["title"], "Основной");
    assert_eq!(item["props"]["description"], "Основной элемент");
    assert_eq!(item["props"]["id"], "c7d2e6fc-3824-4b56-b4be-ae6be4944c0e");

    // Вид без этой коллекции ветви не получает: неприменимое не отвечает
    // пустотой, его просто нет.
    let document = service.view(ViewRequest::new("main:Document.Order").unwrap());
    assert!(document.ok, "{:?}", refusal_codes(&document));
    let branch_addresses: Vec<&str> = document.data.as_ref().unwrap()["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|branch| branch["at"].as_str())
        .collect();
    assert!(
        !branch_addresses
            .iter()
            .any(|at| at.ends_with(".PredefinedItem")),
        "{branch_addresses:?}"
    );
}

#[test]
fn metadata_tabular_section_attribute_consumes_the_complete_suffix() {
    let fixture = RealReaderFixture::new();
    let service = fixture.view_service();

    let result = service.view(
        ViewRequest::new("main:Catalog.Items.TabularSection.Lines.Attribute.Quantity").unwrap(),
    );

    assert!(result.ok, "{} {:?}", result.summary, result.diagnostics);
    let data = result.data.as_ref().unwrap();
    assert_eq!(
        data["at"],
        "main:Catalog.Items.TabularSection.Lines.Attribute.Quantity"
    );
    assert_eq!(data["kind"], "Attribute");
    assert_eq!(data["title"], "Quantity");
}
