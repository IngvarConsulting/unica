use super::*;

const AT: &str = "main:CommonAttribute.ОбластьДанных";
const OWNER: &str = "CommonAttributes/ОбластьДанных.xml";

fn args(values: Value) -> Value {
    json!({"at":AT,"ops":[{"op":"props.set","args":{"values":values}}]})
}

fn active() -> Value {
    json!({"DataSeparation":"Separate", "DataSeparationValue":"SessionParameter.ЗначениеОбласти", "DataSeparationUse":"SessionParameter.ИспользоватьРазделение"})
}

fn replace_type(path: &Path, type_xml: &str) {
    let original = std::fs::read_to_string(path).unwrap();
    let body = original.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(body).unwrap();
    let node = document
        .descendants()
        .find(|node| node.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", "Type")))
        .unwrap();
    let mut updated = body.to_string();
    updated.replace_range(node.range(), type_xml);
    if original.starts_with('\u{feff}') {
        updated.insert(0, '\u{feff}');
    }
    std::fs::write(path, updated).unwrap();
}

#[test]
fn common_attribute_unhashable_types_remain_readable_but_cannot_become_separators() {
    for type_xml in [
        "<Type><v8:Type>v8:ValueStorage</v8:Type></Type>",
        "<Type><v8:Type>xs:string</v8:Type><v8:StringQualifiers><v8:Length>0</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:StringQualifiers></Type>",
        "<Type><v8:Type>xs:base64Binary</v8:Type><v8:BinaryDataQualifiers><v8:Length>10</v8:Length><v8:AllowedLength>Variable</v8:AllowedLength></v8:BinaryDataQualifiers></Type>",
    ] {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    for relative in [OWNER, "SessionParameters/ЗначениеОбласти.xml"] {
        replace_type(&fixture.source.join(relative), type_xml);
    }
    publish(&runtime, &fixture, args(json!({"AutoUse":"Use"})));
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(&runtime, &fixture, ToolIdentity::Apply, args(active()));
    assert!(!result.ok, "type cannot be hashed as a separator: {result:?}");
    assert_eq!(unica_workspace::test_support::tree_snapshot(&fixture.source), before);
    assert_eq!(view(&runtime, &fixture, AT)["props"]["DataSeparation"], "DontUse");
    }
}

#[test]
fn common_attribute_named_types_are_resolved_guarded_and_cannot_hide_storage() {
    for (prefix, kind, directory, name, target_fixture) in [
        (
            "alias-string",
            "DefinedType",
            "DefinedTypes",
            "ТипРазделителя",
            "alias-string-definedtype.xml",
        ),
        (
            "characteristic-string",
            "ChartOfCharacteristicTypes",
            "ChartsOfCharacteristicTypes",
            "Characteristics",
            "characteristic-string-chart.xml",
        ),
    ] {
        let fixture = Fixture::new("workspace-common-attribute");
        let runtime = runtime();
        std::fs::copy(
            fixture
                .workspace
                .join(format!("{prefix}-commonattribute.xml")),
            fixture.source.join(OWNER),
        )
        .unwrap();
        std::fs::create_dir_all(fixture.source.join(directory)).unwrap();
        let target = fixture.source.join(directory).join(format!("{name}.xml"));
        std::fs::copy(fixture.workspace.join(target_fixture), &target).unwrap();
        let root_path = fixture.source.join("Configuration.xml");
        let root = std::fs::read_to_string(&root_path).unwrap();
        std::fs::write(
            &root_path,
            root.replace(
                "</ChildObjects>",
                &format!("<{kind}>{name}</{kind}></ChildObjects>"),
            ),
        )
        .unwrap();
        // A native named String(10) and a direct String(10) parameter agree.
        publish(&runtime, &fixture, args(json!({"AutoUse":"Use"})));
        let preview = call(&runtime, &fixture, ToolIdentity::Apply, args(active()));
        assert!(preview.ok, "{preview:?}");
        let original_type = std::fs::read_to_string(&target).unwrap();
        std::fs::write(
            &target,
            original_type.replace("<v8:Length>10</v8:Length>", "<v8:Length>20</v8:Length>"),
        )
        .unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let stale = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":preview.data.unwrap()["executionToken"]}),
        );
        assert!(
            !stale.ok,
            "named type no-op dependency was not guarded: {stale:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        std::fs::write(&target, &original_type).unwrap();
        std::fs::copy(
            fixture
                .workspace
                .join(format!("{prefix}-sessionparameter.xml")),
            fixture.source.join("SessionParameters/ЗначениеОбласти.xml"),
        )
        .unwrap();
        publish(&runtime, &fixture, args(json!({"AutoUse":"DontUse"})));
        replace_type(&target, "<Type><v8:Type>v8:ValueStorage</v8:Type></Type>");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert!(!call(&runtime, &fixture, ToolIdentity::Apply, args(active())).ok);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        publish(
            &runtime,
            &fixture,
            args(json!({"DataSeparation":"DontUse"})),
        );
        assert_eq!(
            view(&runtime, &fixture, AT)["props"]["DataSeparation"],
            "DontUse"
        );
        // Cycles are refused before a publication, including an inactive edit.
        let type_name = if kind == "DefinedType" {
            format!("cfg:DefinedType.{name}")
        } else {
            format!("cfg:Characteristic.{name}")
        };
        replace_type(
            &target,
            &format!("<Type><v8:TypeSet>{type_name}</v8:TypeSet></Type>"),
        );
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert!(
            !call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                args(json!({"AutoUse":"Use"}))
            )
            .ok
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn common_attribute_each_root_field_changes_only_its_declaration() {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    let schema = contract(&runtime, &fixture, AT, "props.set");
    assert_eq!(
        schema["argsSchema"]["properties"]["values"]["additionalProperties"],
        false
    );
    let path = fixture.source.join(OWNER);
    for (field, value) in [
        ("AutoUse", "Use"),
        ("DataSeparationValue", "SessionParameter.ЗначениеОбласти"),
        (
            "DataSeparationUse",
            "SessionParameter.ИспользоватьРазделение",
        ),
        ("DataSeparation", "Separate"),
        ("SeparatedDataUse", "IndependentlyAndSimultaneously"),
        ("ConditionalSeparation", "Constant.УсловноеРазделение"),
        ("UsersSeparation", "Separate"),
        ("AuthenticationSeparation", "Separate"),
        ("ConfigurationExtensionsSeparation", "Separate"),
    ] {
        let before = std::fs::read_to_string(&path).unwrap();
        let doc = roxmltree::Document::parse(before.trim_start_matches('\u{feff}')).unwrap();
        let node = doc
            .descendants()
            .find(|node| node.tag_name().name() == field)
            .unwrap();
        let mut expected = before.trim_start_matches('\u{feff}').to_string();
        expected.replace_range(node.range(), &format!("<{field}>{value}</{field}>"));
        let before_tree = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(&runtime, &fixture, args(json!({field:value})));
        assert_eq!(
            std::fs::read_to_string(&path)
                .unwrap()
                .trim_start_matches('\u{feff}'),
            expected
        );
        assert_eq!(view(&runtime, &fixture, AT)["props"][field], value);
        for (relative, bytes) in before_tree {
            if relative != Path::new(OWNER) {
                assert_eq!(std::fs::read(fixture.source.join(relative)).unwrap(), bytes);
            }
        }
        if field == "AutoUse" {
            publish(&runtime, &fixture, args(json!({"AutoUse":"DontUse"})));
        }
    }
    // Inactive settings retain all references and policies, as 8.3.27 does.
    publish(
        &runtime,
        &fixture,
        args(json!({"DataSeparation":"DontUse"})),
    );
    for field in [
        "DataSeparationValue",
        "DataSeparationUse",
        "ConditionalSeparation",
    ] {
        publish(&runtime, &fixture, args(json!({field:""})));
        assert_eq!(view(&runtime, &fixture, AT)["props"][field], "");
    }
}

#[test]
fn common_attribute_validates_the_final_batch_and_refuses_invalid_composition_without_writes() {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    let initial = unica_workspace::test_support::tree_snapshot(&fixture.source);
    for values in [
        json!({"DataSeparation":"Separate"}),
        json!({"AutoUse":"Auto"}),
        json!({"Type":"String"}),
        json!({"DataSeparationValue":"SessionParameter.Missing"}),
        json!({"DataSeparationValue":"SessionParameter.ИспользоватьРазделение"}),
        json!({"DataSeparationUse":"SessionParameter.ЗначениеОбласти"}),
        json!({"ConditionalSeparation":"Constant.Missing"}),
        json!({"ConditionalSeparation":"SessionParameter.ИспользоватьРазделение"}),
        json!({"AutoUse":"Use","autoUse":"Use"}),
    ] {
        let result = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
        assert!(!result.ok, "{result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            initial
        );
    }
    publish(
        &runtime,
        &fixture,
        json!({"at":AT,"ops":[
            {"op":"props.set","args":{"values":{"DataSeparation":"Separate"}}},
            {"op":"props.set","args":{"values":{"DataSeparationValue":"SessionParameter.ЗначениеОбласти"}}},
            {"op":"props.set","args":{"values":{"DataSeparationUse":"SessionParameter.ИспользоватьРазделение"}}}
        ]}),
    );
    assert_eq!(
        view(&runtime, &fixture, AT)["props"]["DataSeparation"],
        "Separate"
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let mut batch = args(json!({"AutoUse":"Use"}));
    batch["ops"]
        .as_array_mut()
        .unwrap()
        .push(json!({"op":"props.set","args":{"values":{"DataSeparationUse":""}}}));
    assert!(!call(&runtime, &fixture, ToolIdentity::Apply, batch).ok);
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn common_attribute_saved_plan_guards_noop_dependencies_registration_and_owner() {
    for dependency in [
        OWNER,
        "SessionParameters/ЗначениеОбласти.xml",
        "SessionParameters/ИспользоватьРазделение.xml",
        "Constants/УсловноеРазделение.xml",
        "Configuration.xml",
    ] {
        let fixture = Fixture::new("workspace-common-attribute");
        let runtime = runtime();
        let mut values = active();
        values["ConditionalSeparation"] = json!("Constant.УсловноеРазделение");
        publish(&runtime, &fixture, args(values.clone()));
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        publish(&runtime, &fixture, args(values.clone()));
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
        let plan = call(&runtime, &fixture, ToolIdentity::Apply, args(values));
        assert!(plan.ok, "{plan:?}");
        let path = fixture.source.join(dependency);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n<!-- external change -->\n");
        std::fs::write(path, bytes).unwrap();
        let external = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]}),
        );
        assert!(
            !result.ok,
            "saved no-op plan accepted changed {dependency}: {result:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            external
        );
    }
}

#[test]
fn common_attribute_reference_types_compare_qualified_names_and_exact_qualifiers() {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    let path = fixture.source.join("SessionParameters/ЗначениеОбласти.xml");
    let source = std::fs::read_to_string(&path).unwrap();
    let alias = source
        .replace("xmlns:xs=", "xmlns:alias=")
        .replace("xs:string", "alias:string");
    std::fs::write(&path, &alias).unwrap();
    publish(&runtime, &fixture, args(active()));
    for bad in [
        alias.replace("<v8:Length>10</v8:Length>", "<v8:Length>20</v8:Length>"),
        alias.replace("http://www.w3.org/2001/XMLSchema", "urn:foreign"),
    ] {
        std::fs::write(&path, bad).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        assert!(!call(&runtime, &fixture, ToolIdentity::Apply, args(active())).ok);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn common_attribute_borrowed_extension_changes_only_existing_autouse() {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    std::fs::write(
        fixture.workspace.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTENSION\n    path: src\n",
    )
    .unwrap();
    let owner = std::fs::read(fixture.workspace.join("borrowed-autouse.xml")).unwrap();
    std::fs::write(fixture.source.join(OWNER), owner).unwrap();
    std::fs::copy(
        fixture.workspace.join("borrowed-Configuration.xml"),
        fixture.source.join("Configuration.xml"),
    )
    .unwrap();
    std::fs::create_dir_all(fixture.source.join("Roles")).unwrap();
    std::fs::copy(
        fixture.workspace.join("borrowed-Role.xml"),
        fixture.source.join("Roles/ОсновнаяРоль.xml"),
    )
    .unwrap();
    let contract = contract(&runtime, &fixture, AT, "props.set");
    assert!(contract["argsSchema"]["properties"]["values"]["properties"]
        .get("DataSeparation")
        .is_none());
    let path = fixture.source.join(OWNER);
    let before = std::fs::read_to_string(&path).unwrap();
    publish(&runtime, &fixture, args(json!({"AutoUse":"DontUse"})));
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        before.replace("<AutoUse>Use</AutoUse>", "<AutoUse>DontUse</AutoUse>")
    );
    assert_eq!(view(&runtime, &fixture, AT)["props"]["AutoUse"], "DontUse");
    assert!(
        !call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(json!({"DataSeparation":"Separate"}))
        )
        .ok
    );
}

#[test]
fn common_attribute_conditional_constant_respects_content_overrides_without_rewriting_rows() {
    let fixture = Fixture::new("workspace-common-attribute");
    let runtime = runtime();
    let mut values = active();
    values["ConditionalSeparation"] = json!("Constant.УсловноеРазделение");
    publish(&runtime, &fixture, args(values));
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    assert!(
        !call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(json!({"AutoUse":"Use"}))
        )
        .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
    let path = fixture.source.join(OWNER);
    let source = std::fs::read_to_string(&path).unwrap();
    let excluded = source.replace("<Content/>", "<Content><xr:Item><xr:Metadata>Constant.УсловноеРазделение</xr:Metadata><xr:Use>DontUse</xr:Use><xr:ConditionalSeparation/></xr:Item></Content>");
    std::fs::write(&path, &excluded).unwrap();
    publish(&runtime, &fixture, args(json!({"AutoUse":"Use"})));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        excluded.replace("<AutoUse>DontUse</AutoUse>", "<AutoUse>Use</AutoUse>")
    );
    let included = excluded.replace("<xr:Use>DontUse</xr:Use>", "<xr:Use>Use</xr:Use>");
    std::fs::write(&path, &included).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    assert!(
        !call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            args(json!({"AutoUse":"DontUse"}))
        )
        .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
