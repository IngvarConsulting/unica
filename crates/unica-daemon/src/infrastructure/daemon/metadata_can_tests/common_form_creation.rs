use super::*;

const FORM_AT: &str = "main:CommonForm.BlankForm";
const MODULE_AT: &str = "main:CommonForm.BlankForm.Module.Form";
const DESCRIPTOR: &str = "CommonForms/BlankForm.xml";
const FORM: &str = "CommonForms/BlankForm/Ext/Form.xml";
const MODULE: &str = "CommonForms/BlankForm/Ext/Form/Module.bsl";
const BOM: &[u8] = b"\xef\xbb\xbf";

fn create(at: &str) -> Value {
    json!({
        "at": at,
        "ops": [{"op":"object.create","args":{"values":{"kind":"CommonForm","name":"BlankForm"}}}]
    })
}

fn set_profile(fixture: &Fixture, format: &str, script: &str, text_language: &str) {
    let set_version =
        |text: String| text.replace("version=\"2.20\"", &format!("version=\"{format}\""));
    let configuration = fixture.source.join("Configuration.xml");
    let mut text = set_version(std::fs::read_to_string(&configuration).unwrap()).replace(
        "</Properties>",
        &format!("<ScriptVariant>{script}</ScriptVariant></Properties>"),
    );
    let russian = fixture.source.join("Languages/Русский.xml");
    let mut language = set_version(std::fs::read_to_string(&russian).unwrap());
    let language_path = if text_language == "en" {
        text = text.replace("Русский", "English");
        language = language.replace("Русский", "English").replace(
            "<LanguageCode>ru</LanguageCode>",
            "<LanguageCode>en</LanguageCode>",
        );
        std::fs::remove_file(&russian).unwrap();
        fixture.source.join("Languages/English.xml")
    } else {
        russian
    };
    std::fs::write(configuration, text).unwrap();
    std::fs::write(language_path, language).unwrap();
}

/// rule: INV.SOURCE.WRITABLE-PROFILE
/// rule: INV.SOURCE.TAIL-INSERT
#[test]
fn common_form_creation_and_first_insert_preserve_the_native_shape() {
    for (format, script, language, panel, compatibility_property) in [
        ("2.20", "Russian", "ru", "ФормаКоманднаяПанель", false),
        ("2.21", "English", "en", "FormCommandBar", true),
        ("2.22", "English", "ru", "FormCommandBar", true),
    ] {
        let fixture = Fixture::blank();
        set_profile(&fixture, format, script, language);
        let runtime = runtime();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let can = contract(&runtime, &fixture, "main:Configuration", "object.create");
        let values = &can["argsSchema"]["properties"]["values"];
        let kinds = values["properties"]["kind"]["enum"].as_array().unwrap();
        assert!(kinds.contains(&json!("CommonForm")), "{kinds:?}");
        assert_eq!(values["additionalProperties"], false);
        assert_eq!(values["properties"].as_object().unwrap().len(), 2);

        publish(&runtime, &fixture, create("main:Configuration"));
        let created = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let added = created
            .keys()
            .filter(|k| !before.contains_key(*k))
            .collect::<BTreeSet<_>>();
        let expected = [DESCRIPTOR, FORM, MODULE].map(PathBuf::from);
        assert_eq!(added, expected.iter().collect::<BTreeSet<_>>());
        for (path, bytes) in &before {
            if path == Path::new("Configuration.xml") {
                let configuration = std::str::from_utf8(&created[path]).unwrap();
                assert_eq!(
                    configuration
                        .matches("<CommonForm>BlankForm</CommonForm>")
                        .count(),
                    1
                );
            } else {
                assert_eq!(&created[path], bytes, "{}", path.display());
            }
        }

        let descriptor = std::str::from_utf8(&created[Path::new(DESCRIPTOR)]).unwrap();
        let descriptor =
            roxmltree::Document::parse(descriptor.trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(descriptor.root_element().attribute("version"), Some(format));
        let properties = descriptor
            .descendants()
            .find(|n| n.has_tag_name("Properties"))
            .unwrap();
        let names: Vec<_> = properties
            .children()
            .filter(roxmltree::Node::is_element)
            .map(|n| n.tag_name().name())
            .collect();
        let mut expected_properties = vec![
            "Name",
            "Synonym",
            "Comment",
            "FormType",
            "IncludeHelpInContents",
            "UsePurposes",
            "UseStandardCommands",
            "ExtendedPresentation",
            "Explanation",
        ];
        if compatibility_property {
            expected_properties.insert(6, "UseInInterfaceCompatibilityMode");
            assert!(properties
                .children()
                .any(|n| n.has_tag_name("UseInInterfaceCompatibilityMode")
                    && n.text() == Some("Any")));
        }
        assert_eq!(names, expected_properties);
        assert!(properties
            .children()
            .any(|n| n.has_tag_name("FormType") && n.text() == Some("Managed")));
        assert!(properties
            .descendants()
            .any(|n| n.has_tag_name("lang") && n.text() == Some(language)));

        let form = std::str::from_utf8(&created[Path::new(FORM)]).unwrap();
        let form = roxmltree::Document::parse(form.trim_start_matches('\u{feff}')).unwrap();
        let root = form.root_element();
        assert!(root.has_tag_name(("http://v8.1c.ru/8.3/xcf/logform", "Form")));
        assert_eq!(root.attribute("version"), Some(format));
        let children: Vec<_> = root
            .children()
            .filter(roxmltree::Node::is_element)
            .collect();
        assert_eq!(children.len(), 2);
        assert!(children[0].has_tag_name("AutoCommandBar"));
        assert_eq!(children[0].attribute("name"), Some(panel));
        assert_eq!(children[0].attribute("id"), Some("-1"));
        assert!(children[1].has_tag_name("Attributes"));
        assert_eq!(children[1].children().count(), 0);
        assert_eq!(created[Path::new(MODULE)], BOM);
        assert_eq!(view(&runtime, &fixture, FORM_AT)["kind"], "Form");
        assert_eq!(view(&runtime, &fixture, MODULE_AT)["kind"], "Module");

        contract(&runtime, &fixture, MODULE_AT, "code.insert");
        let text = "// First text\n";
        publish(
            &runtime,
            &fixture,
            json!({
                "at": MODULE_AT,
                "ops": [{"op":"code.insert", "args":{"text":text}}]
            }),
        );
        let mut expected = created;
        let module = std::fs::read(fixture.source.join(MODULE)).unwrap();
        assert!(module.starts_with(BOM));
        assert_eq!(
            std::str::from_utf8(&module[BOM.len()..])
                .unwrap()
                .replace("\r\n", "\n"),
            text
        );
        expected.insert(PathBuf::from(MODULE), module);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            expected
        );
    }
}

#[test]
fn common_form_creation_is_refused_in_an_extension_and_on_conflicting_files() {
    let runtime = runtime();
    let extension = Fixture::with_extension();
    let can = contract(&runtime, &extension, "ext:Configuration", "object.create");
    let kinds = can["argsSchema"]["properties"]["values"]["properties"]["kind"]["enum"]
        .as_array()
        .unwrap();
    assert!(!kinds.contains(&json!("CommonForm")));
    let ext_source = extension.workspace.join("src-ext");
    let ext_before = unica_workspace::test_support::tree_snapshot(&ext_source);
    let refused = call(
        &runtime,
        &extension,
        ToolIdentity::Apply,
        create("ext:Configuration"),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[0].args.values.kind"));
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&ext_source),
        ext_before
    );

    let fixture = Fixture::blank();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let preview = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        create("main:Configuration"),
    );
    assert!(preview.ok, "{preview:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );

    let unrelated = fixture.source.join(MODULE);
    std::fs::create_dir_all(unrelated.parent().unwrap()).unwrap();
    std::fs::write(&unrelated, b"// intervening module").unwrap();
    let with_conflict = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({
            "executionToken":preview.data.unwrap()["executionToken"]
        }),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        with_conflict
    );
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        create("main:Configuration"),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        with_conflict
    );
}
