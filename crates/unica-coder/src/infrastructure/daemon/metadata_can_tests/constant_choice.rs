use super::*;

fn fixture(runtime: &V5CanonicalInvocationRuntime) -> Fixture {
    let fixture = catalog_and_constant(runtime);
    let path = fixture.source.join("Catalogs/Products.xml");
    let xml = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        xml.replace("</ChildObjects>", "<Form>Choice</Form></ChildObjects>"),
    )
    .unwrap();
    let form = std::fs::read_to_string(
        repo_root()
            .join("tests/fixtures/acceptance/workspace/src/Catalogs/Валюты/Forms/ФормаСписка.xml"),
    )
    .unwrap();
    let form_path = fixture.source.join("Catalogs/Products/Forms/Choice.xml");
    std::fs::create_dir_all(form_path.parent().unwrap()).unwrap();
    std::fs::write(form_path, form.replace("ФормаСписка", "Choice")).unwrap();
    publish(
        runtime,
        &fixture,
        json!({"at":"main:Constant.Rate","ops":[{"op":"props.set","args":{"values":{
            "Type":{"variants":[{"kind":"reference","metadataPath":"Catalog.Products"}]}
        }}}]}),
    );
    fixture
}

fn request(values: Value) -> Value {
    json!({"at":"main:Constant.Rate","ops":[{"op":"props.set","args":{"values":values}}]})
}

#[test]
fn constant_choice_roundtrip_preserves_other_settings_and_catalog_contract() {
    let runtime = runtime();
    let fixture = fixture(&runtime);
    let schema = contract(&runtime, &fixture, "main:Constant.Rate", "props.set");
    assert_eq!(
        schema["argsSchema"]["properties"]["values"]["properties"]["QuickChoice"]["enum"],
        json!(["Auto", "Use", "DontUse"])
    );
    let catalog = contract(&runtime, &fixture, "main:Catalog.Products", "props.set");
    assert_eq!(
        catalog["argsSchema"]["properties"]["values"]["properties"]["QuickChoice"]["type"],
        "boolean"
    );
    let path = fixture.source.join("Constants/Rate.xml");
    let before = std::fs::read_to_string(&path).unwrap();
    let tree_before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    publish(
        &runtime,
        &fixture,
        request(
            json!({"ChoiceFoldersAndItems":"FoldersAndItems","ChoiceHistoryOnInput":"DontUse","ChoiceForm":"Catalog.Products.Form.Choice"}),
        ),
    );
    for quick in ["Use", "DontUse", "Auto"] {
        publish(&runtime, &fixture, request(json!({"QuickChoice":quick})));
        let props = view(&runtime, &fixture, "main:Constant.Rate")["props"].clone();
        for (name, expected) in [
            ("QuickChoice", quick),
            ("ChoiceFoldersAndItems", "FoldersAndItems"),
            ("ChoiceHistoryOnInput", "DontUse"),
            ("ChoiceForm", "Catalog.Products.Form.Choice"),
        ] {
            assert_eq!(props[name], expected, "{props}");
        }
    }
    let after = std::fs::read_to_string(&path).unwrap();
    let mut tree_after = unica_workspace::test_support::tree_snapshot(&fixture.source);
    tree_after.insert(
        PathBuf::from("Constants/Rate.xml"),
        before.as_bytes().to_vec(),
    );
    assert_eq!(tree_after, tree_before);
    assert_eq!(
        after,
        before
            .replace(
                "<ChoiceFoldersAndItems>Items</ChoiceFoldersAndItems>",
                "<ChoiceFoldersAndItems>FoldersAndItems</ChoiceFoldersAndItems>"
            )
            .replace(
                "<ChoiceHistoryOnInput>Auto</ChoiceHistoryOnInput>",
                "<ChoiceHistoryOnInput>DontUse</ChoiceHistoryOnInput>"
            )
            .replace(
                "<ChoiceForm/>",
                "<ChoiceForm>Catalog.Products.Form.Choice</ChoiceForm>"
            )
    );
    publish(&runtime, &fixture, request(json!({"ChoiceForm":""})));
}

#[test]
fn constant_choice_accepts_platform_inactive_settings_and_checks_noop_references() {
    let runtime = runtime();
    let fixture = fixture(&runtime);
    publish(
        &runtime,
        &fixture,
        request(
            json!({"Type":{"variants":[{"kind":"string","length":10,"allowedLength":"variable"}]},"QuickChoice":"Use","ChoiceFoldersAndItems":"FoldersAndItems","ChoiceForm":"Catalog.Products.Form.Choice"}),
        ),
    );
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    publish(
        &runtime,
        &fixture,
        request(json!({"ChoiceForm":"Catalog.Products.Form.Choice"})),
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    std::fs::remove_file(fixture.source.join("Catalogs/Products/Forms/Choice.xml")).unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        request(json!({"ChoiceForm":"Catalog.Products.Form.Choice"})),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    let result = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        request(
            json!({"ChoiceForm":"Catalog.Products.Form.Choice","Comment":"must still validate"}),
        ),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
}

#[test]
fn constant_choice_rejects_invalid_values_and_forms_without_any_writes() {
    let runtime = runtime();
    let fixture = fixture(&runtime);
    for values in [
        json!({"QuickChoice":true}),
        json!({"QuickChoice":"Never"}),
        json!({"ChoiceFoldersAndItems":"All"}),
        json!({"ChoiceHistoryOnInput":"Use"}),
        json!({"ChoiceForm":"Catalog.Products.Form.Missing","Comment":"must not write"}),
        json!({"ChoiceForm":"../Choice"}),
        json!({"ChoiceForm":"Catalog.Products"}),
        json!({"ChoiceForm":"Constant.Rate.Form.Choice"}),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            request(values.clone()),
        );
        assert!(!result.ok, "{values}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn constant_choice_saved_token_guards_form_and_owner() {
    let runtime = runtime();
    for relative in [
        "Catalogs/Products.xml",
        "Catalogs/Products/Forms/Choice.xml",
        "Configuration.xml",
    ] {
        let fixture = fixture(&runtime);
        let planned = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            request(json!({"ChoiceForm":"Catalog.Products.Form.Choice"})),
        );
        assert!(planned.ok, "{planned:?}");
        let target = fixture.source.join(relative);
        let text = std::fs::read_to_string(&target).unwrap();
        std::fs::write(target, format!("{text}\n<!-- external change -->")).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":planned.data.unwrap()["executionToken"]}),
        );
        assert!(!result.ok, "{relative}: {result:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}
