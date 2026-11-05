use super::*;

const OWNERS: &[(&str, &str, &str)] = &[
    ("CommonAttribute", "CommonAttributes", "SharedValue"),
    ("SessionParameter", "SessionParameters", "SessionValue"),
    ("FilterCriterion", "FilterCriteria", "CriterionValue"),
    ("DefinedType", "DefinedTypes", "Existing"),
];

fn type_request(at: &str, metadata_type: Value) -> Value {
    json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Type":metadata_type}}}]})
}

#[test]
fn canonical_root_type_profiles_read_publish_and_preserve_other_bytes() {
    let fixture = Fixture::new("workspace-root-type-properties");
    let runtime = runtime();
    for (kind, folder, name) in OWNERS {
        let at = format!("main:{kind}.{name}");
        let original = view(&runtime, &fixture, &at);
        assert!(
            original["props"]["Type"]["variants"].is_array(),
            "{at}: {original}"
        );
        let contract = view(&runtime, &fixture, &format!("{at}.Can.props.set"));
        let fields =
            &contract["props"]["contract"]["argsSchema"]["properties"]["values"]["properties"];
        assert!(fields.get("Type").is_some(), "{at}: {contract}");
        if *kind != "DefinedType" {
            assert_eq!(fields.as_object().unwrap().len(), 3);
            assert!(fields.get("Comment").is_some());
            assert!(fields.get("comment").is_some());
        } else {
            assert!(fields.get("Synonym").is_some());
            assert!(fields.get("Comment").is_some());
        }
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let metadata_type =
            json!({"variants":[{"kind":"number","digits":12,"fraction":2,"sign":"any"}]});
        publish(&runtime, &fixture, type_request(&at, metadata_type.clone()));
        let path = PathBuf::from(format!("{folder}/{name}.xml"));
        let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>()
        );
        assert_eq!(
            before
                .iter()
                .filter(|(path, bytes)| after.get(*path) != Some(*bytes))
                .map(|(path, _)| path)
                .collect::<Vec<_>>(),
            vec![&path]
        );
        fn without_type(raw: &[u8]) -> String {
            let source = std::str::from_utf8(raw).unwrap();
            let document =
                roxmltree::Document::parse(source.trim_start_matches('\u{feff}')).unwrap();
            let node = document
                .descendants()
                .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Type")))
                .unwrap();
            let bom = source.len() - source.trim_start_matches('\u{feff}').len();
            let mut result = source.to_string();
            result.replace_range(node.range().start + bom..node.range().end + bom, "<Type/>");
            result
        }
        assert_eq!(
            without_type(&before[&path]),
            without_type(&after[&path]),
            "{at}"
        );
        let current = view(&runtime, &fixture, &at);
        assert_eq!(
            current["props"]["Type"]["variants"], metadata_type["variants"],
            "{at}: {current}"
        );
        assert_eq!(current["props"]["Type"]["mutationCapability"], "editable");
        publish(&runtime, &fixture, type_request(&at, metadata_type));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            after,
            "equivalent Type must be a byte no-op"
        );
    }
}

fn refused_without_writes(
    runtime: &V5CanonicalInvocationRuntime,
    fixture: &Fixture,
    args: Value,
) -> DomainResult {
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(runtime, fixture, ToolIdentity::Apply, args);
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    result
}

#[test]
fn canonical_root_type_profiles_refuse_invalid_inputs_and_neighbor_writes() {
    let fixture = Fixture::new("workspace-root-type-properties");
    let runtime = runtime();
    for (kind, _, name) in OWNERS {
        let at = format!("main:{kind}.{name}");
        for metadata_type in [
            json!(null),
            json!({"variants":[]}),
            json!({"variants":[{"kind":"future"}]}),
            json!({"variants":[{"kind":"number","digits":2,"fraction":3,"sign":"any"}]}),
            json!({"variants":[{"kind":"reference","metadataPath":"Report.Items"}]}),
            json!({"variants":[{"kind":"reference","metadataPath":"other:Catalog.Items"}]}),
            json!({"variants":[{"kind":"typeId","typeId":"12345678-1234-4123-8123-123456789abc"}]}),
            json!({"variants":[{"kind":"characteristicTypes","metadataPath":"ChartOfCharacteristicTypes.Kinds"}]}),
        ] {
            refused_without_writes(&runtime, &fixture, type_request(&at, metadata_type));
        }
        if *kind != "DefinedType" {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let comment = call(&runtime, &fixture, ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Comment":"independent comment"}}}]}));
            assert!(comment.ok, "{comment:?}");
            assert_eq!(unica_workspace::test_support::tree_snapshot(&fixture.source), before);
            refused_without_writes(&runtime, &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Comment":"mixed", "Type":{"variants":[{"kind":"boolean"}]}}}}]}));
            refused_without_writes(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Synonym":"outside this profile"}}}]}),
            );
        }
    }
}

#[test]
fn canonical_root_type_reference_dependencies_fence_saved_tokens() {
    for (kind, _, name) in OWNERS {
        for dependency in ["Configuration.xml", "Catalogs/Items.xml"] {
            let fixture = Fixture::new("workspace-root-type-properties");
            let runtime = runtime();
            let at = format!("main:{kind}.{name}");
            let planned = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                type_request(
                    &at,
                    json!({"variants":[{"kind":"reference","metadataPath":"Catalog.Items"}]}),
                ),
            );
            assert!(planned.ok, "{at}: {planned:?}");
            let path = fixture.source.join(dependency);
            let mut bytes = std::fs::read(&path).unwrap();
            bytes.extend_from_slice(b"\n");
            std::fs::write(&path, bytes).unwrap();
            refused_without_writes(
                &runtime,
                &fixture,
                json!({"executionToken":planned.data.unwrap()["executionToken"]}),
            );
        }
    }
}

#[test]
fn canonical_root_type_references_require_registered_matching_final_descriptors() {
    for defect in [
        "missing",
        "unregistered",
        "wrongName",
        "wrongKind",
        "duplicateRegistration",
    ] {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        let path = fixture.source.join("Catalogs/Items.xml");
        match defect {
            "missing" => std::fs::remove_file(path).unwrap(),
            "unregistered" => rewrite(
                &fixture.source.join("Configuration.xml"),
                "<Catalog>Items</Catalog>",
                "",
            ),
            "wrongName" => rewrite(&path, "<Name>Items</Name>", "<Name>Different</Name>"),
            "wrongKind" => {
                rewrite(&path, "<Catalog uuid=", "<Document uuid=");
                rewrite(&path, "</Catalog>", "</Document>");
            }
            "duplicateRegistration" => rewrite(
                &fixture.source.join("Configuration.xml"),
                "<Catalog>Items</Catalog>",
                "<Catalog>Items</Catalog><Catalog>Items</Catalog>",
            ),
            _ => unreachable!(),
        }
        refused_without_writes(
            &runtime,
            &fixture,
            type_request(
                "main:CommonAttribute.SharedValue",
                json!({"variants":[{"kind":"reference","metadataPath":"Catalog.Items"}]}),
            ),
        );
    }
}

#[test]
fn canonical_root_type_defined_graph_rejects_reachable_cycles_and_fences_noop_dependencies() {
    let fixture = Fixture::new("workspace-root-type-properties");
    let runtime = runtime();
    let reference = |name: &str| json!({"variants":[{"kind":"definedType","metadataPath":format!("DefinedType.{name}")}]});
    publish(
        &runtime,
        &fixture,
        type_request("main:DefinedType.Helper", reference("Existing")),
    );
    refused_without_writes(
        &runtime,
        &fixture,
        type_request("main:DefinedType.Existing", reference("Helper")),
    );
    // Reset the reachable definition; the unchanged intermediate is still
    // read and fenced when the owner's next change is an exact no-op.
    publish(
        &runtime,
        &fixture,
        type_request(
            "main:DefinedType.Helper",
            json!({"variants":[{"kind":"string","length":9,"allowedLength":"variable"}]}),
        ),
    );
    publish(
        &runtime,
        &fixture,
        type_request("main:DefinedType.Existing", reference("Helper")),
    );
    let planned = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        type_request("main:DefinedType.Existing", reference("Helper")),
    );
    assert!(planned.ok, "{planned:?}");
    rewrite(
        &fixture.source.join("DefinedTypes/Helper.xml"),
        "<v8:Length>9</v8:Length>",
        "<v8:Length>10</v8:Length>",
    );
    refused_without_writes(
        &runtime,
        &fixture,
        json!({"executionToken":planned.data.unwrap()["executionToken"]}),
    );
}

#[test]
fn canonical_root_type_final_graph_accepts_forward_created_targets_and_overwritten_edits() {
    let fixture = Fixture::new("workspace-root-type-properties");
    let runtime = runtime();
    publish(
        &runtime,
        &fixture,
        json!({"at":"main:Configuration","ops":[
            {"op":"object.create","args":{"values":{"kind":"DefinedType","name":"Fresh"}}},
            {"op":"props.set","args":{"at":"main:DefinedType.Fresh","values":{"Type":{"variants":[{"kind":"reference","metadataPath":"Catalog.Future"}]}}}},
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"Future"}}}
        ]}),
    );
    let at = "main:DefinedType.Existing";
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[
            {"op":"props.set","args":{"values":{"Type":{"variants":[{"kind":"reference","metadataPath":"Catalog.DoesNotExist"}]}}}},
            {"op":"props.set","args":{"values":{"Type":{"variants":[{"kind":"boolean"}]},"Comment":"retained existing writer"}}}
        ]}),
    );
    let current = view(&runtime, &fixture, at);
    assert_eq!(
        current["props"]["Type"]["variants"],
        json!([{"kind":"boolean"}])
    );
    assert_eq!(current["props"]["Comment"], "retained existing writer");
}

#[test]
fn canonical_root_type_bad_owner_and_type_contexts_refuse_without_source_writes() {
    for (kind, folder, name) in OWNERS {
        for defect in [
            "missingType",
            "duplicateType",
            "nilType",
            "nilProperties",
            "wrongName",
            "missingRegistration",
        ] {
            let fixture = Fixture::new("workspace-root-type-properties");
            let runtime = runtime();
            let path = fixture.source.join(format!("{folder}/{name}.xml"));
            let text = std::fs::read_to_string(&path).unwrap();
            let type_start = text.find("<Type>").unwrap();
            let type_end =
                text[type_start..].find("</Type>").unwrap() + type_start + "</Type>".len();
            let changed = match defect {
                "missingType" => {
                    let mut x = text.clone();
                    x.replace_range(type_start..type_end, "");
                    x
                }
                "duplicateType" => text.replacen(
                    "</Type>",
                    "</Type><Type><v8:Type>xs:boolean</v8:Type></Type>",
                    1,
                ),
                "nilType" => text.replacen("<Type>", "<Type xsi:nil=\"true\">", 1),
                "nilProperties" => {
                    text.replacen("<Properties>", "<Properties xsi:nil=\"true\">", 1)
                }
                "wrongName" => {
                    text.replacen(&format!("<Name>{name}</Name>"), "<Name>Different</Name>", 1)
                }
                "missingRegistration" => {
                    rewrite(
                        &fixture.source.join("Configuration.xml"),
                        &format!("<{kind}>{name}</{kind}>"),
                        "",
                    );
                    text.clone()
                }
                _ => unreachable!(),
            };
            std::fs::write(path, changed).unwrap();
            refused_without_writes(
                &runtime,
                &fixture,
                type_request(
                    &format!("main:{kind}.{name}"),
                    json!({"variants":[{"kind":"boolean"}]}),
                ),
            );
        }
    }
}

#[test]
fn canonical_root_type_read_only_observation_does_not_become_an_input_variant() {
    for (kind, folder, name) in OWNERS {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        let path = fixture.source.join(format!("{folder}/{name}.xml"));
        let mut text = std::fs::read_to_string(&path).unwrap();
        let start = text.find("<Type>").unwrap();
        let end = text[start..].find("</Type>").unwrap() + start + "</Type>".len();
        text.replace_range(
            start..end,
            "<Type><v8:TypeId>12345678-1234-4123-8123-123456789abc</v8:TypeId></Type>",
        );
        std::fs::write(path, text).unwrap();
        let at = format!("main:{kind}.{name}");
        let observed = view(&runtime, &fixture, &at);
        assert_eq!(
            observed["props"]["Type"]["mutationCapability"], "readOnly",
            "{at}: {observed}"
        );
        refused_without_writes(
            &runtime,
            &fixture,
            type_request(&at, observed["props"]["Type"].clone()),
        );
        // The observed value is not a write operand. An explicit supported
        // replacement retains the existing editor's replacement semantics.
        publish(
            &runtime,
            &fixture,
            type_request(&at, json!({"variants":[{"kind":"boolean"}]})),
        );
        assert_eq!(
            view(&runtime, &fixture, &at)["props"]["Type"]["variants"],
            json!([{"kind":"boolean"}])
        );
    }
}

#[test]
fn canonical_root_type_unmodelled_and_broken_values_keep_independent_identity() {
    for (replacement, code) in [
        ("<v8:Type>v8:FutureOpaque</v8:Type>", "validation_warning"),
        ("<v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>bad</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers>", "validation_failed"),
    ] {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        let path = fixture.source.join("SessionParameters/SessionValue.xml");
        rewrite(
            &path,
            "<v8:Type>v8:UUID</v8:Type>",
            replacement,
        );
        let current = view(&runtime, &fixture, "main:SessionParameter.SessionValue");
        assert_eq!(current["title"], "SessionValue");
        assert_eq!(current["props"]["Comment"], "");
        assert!(current["props"].get("Type").is_none());
        assert!(
            limits(&current)
                .iter()
                .any(|limit| limit.contains(code) && limit.contains("Type")),
            "{current}"
        );
    }
}

#[test]
fn canonical_root_type_preserves_an_active_fill_value_and_refuses_incompatible_types() {
    let fixture = Fixture::new("workspace-root-type-properties");
    let runtime = runtime();
    let path = fixture.source.join("CommonAttributes/SharedValue.xml");
    rewrite(
        &path,
        "<FillFromFillingValue>false</FillFromFillingValue>",
        "<FillFromFillingValue>true</FillFromFillingValue>",
    );
    rewrite(
        &path,
        "<FillValue xsi:type=\"xs:string\"/>",
        "<FillValue xsi:type=\"xs:string\">keep me</FillValue>",
    );
    let at = "main:CommonAttribute.SharedValue";
    refused_without_writes(
        &runtime,
        &fixture,
        type_request(
            at,
            json!({"variants":[{"kind":"reference","metadataPath":"Catalog.Items"}]}),
        ),
    );
    publish(
        &runtime,
        &fixture,
        type_request(
            at,
            json!({"variants":[{"kind":"string","length":32,"allowedLength":"variable"}]}),
        ),
    );
    let current = std::fs::read_to_string(path).unwrap();
    assert!(current.contains("<FillValue xsi:type=\"xs:string\">keep me</FillValue>"));
}

#[test]
fn canonical_root_type_xml_space_is_normalized_without_treating_nbsp_as_empty() {
    for (kind, folder, name) in OWNERS {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        let path = fixture.source.join(format!("{folder}/{name}.xml"));
        rewrite(&path, "<Properties>", "<Properties xsi:nil=\" false \" >");
        if *kind == "CommonAttribute" {
            rewrite(
                &path,
                "<FillFromFillingValue>false</FillFromFillingValue>",
                "<FillFromFillingValue> false </FillFromFillingValue>",
            );
        }
        publish(
            &runtime,
            &fixture,
            type_request(
                &format!("main:{kind}.{name}"),
                json!({"variants":[{"kind":"boolean"}]}),
            ),
        );
    }
}

#[test]
fn canonical_root_type_nbsp_is_not_an_empty_definition_type() {
    for text_content in ["&#160;", "&#160; &#160;"] {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        let path = fixture.source.join("DefinedTypes/Existing.xml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        let start = text.find("<Type>").unwrap();
        let end = text[start..].find("</Type>").unwrap() + start + "</Type>".len();
        text.replace_range(start..end, &format!("<Type>{text_content}</Type>"));
        std::fs::write(path, text).unwrap();
        refused_without_writes(
            &runtime,
            &fixture,
            type_request(
                "main:DefinedType.Existing",
                json!({"variants":[{"kind":"boolean"}]}),
            ),
        );
    }
}

#[test]
fn canonical_root_type_adopted_separator_is_immutable_but_ordinary_attribute_is_writable() {
    for separated in [false, true] {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        rewrite(
            &fixture.workspace.join("v8project.yaml"),
            "type: CONFIGURATION",
            "type: EXTENSION",
        );
        rewrite(&fixture.source.join("Configuration.xml"), "<Properties>", "<Properties><ObjectBelonging>Adopted</ObjectBelonging><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>");
        let path = fixture.source.join("CommonAttributes/SharedValue.xml");
        rewrite(&path, "<Properties>", "<Properties><ObjectBelonging>Adopted</ObjectBelonging><ExtendedConfigurationObject>12345678-1234-4123-8123-123456789abc</ExtendedConfigurationObject>");
        if separated {
            rewrite(
                &path,
                "<DataSeparation>DontUse</DataSeparation>",
                "<DataSeparation>Separate</DataSeparation>",
            );
            let refused = refused_without_writes(
                &runtime,
                &fixture,
                type_request(
                    "main:CommonAttribute.SharedValue",
                    json!({"variants":[{"kind":"boolean"}]}),
                ),
            );
            assert!(
                refused.summary.contains("borrowed data separator"),
                "{refused:?}"
            );
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            publish(
                &runtime,
                &fixture,
                type_request(
                    "main:CommonAttribute.SharedValue",
                    json!({"variants":[{"kind":"string","length":0,"allowedLength":"variable"}]}),
                ),
            );
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        } else {
            publish(
                &runtime,
                &fixture,
                type_request(
                    "main:CommonAttribute.SharedValue",
                    json!({"variants":[{"kind":"boolean"}]}),
                ),
            );
        }
    }
}

#[test]
fn canonical_root_type_malformed_parent_reference_is_not_an_owned_extension_attribute() {
    for parent in [
        "not-a-uuid",
        "",
        "&#160;12345678-1234-4123-8123-123456789abc&#160;",
    ] {
        let fixture = Fixture::new("workspace-root-type-properties");
        let runtime = runtime();
        rewrite(
            &fixture.workspace.join("v8project.yaml"),
            "type: CONFIGURATION",
            "type: EXTENSION",
        );
        rewrite(&fixture.source.join("Configuration.xml"), "<Properties>", "<Properties><ObjectBelonging>Adopted</ObjectBelonging><ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>");
        let path = fixture.source.join("CommonAttributes/SharedValue.xml");
        rewrite(&path, "<Properties>", &format!("<Properties><ObjectBelonging>Adopted</ObjectBelonging><ExtendedConfigurationObject>{parent}</ExtendedConfigurationObject>"));
        rewrite(
            &path,
            "<DataSeparation>DontUse</DataSeparation>",
            "<DataSeparation>Separate</DataSeparation>",
        );
        refused_without_writes(
            &runtime,
            &fixture,
            type_request(
                "main:CommonAttribute.SharedValue",
                json!({"variants":[{"kind":"boolean"}]}),
            ),
        );
    }
}
