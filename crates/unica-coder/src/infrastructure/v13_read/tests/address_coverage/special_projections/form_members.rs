use super::*;

pub(super) fn bound_command_events(
    native: &mut NativeProjections<'_, '_>,
    form_at: &str,
    form: roxmltree::Node<'_, '_>,
    origin: &str,
) {
    let commands = native_form_children(form, "Commands")
        .flat_map(|container| native_form_children(container, "Command"));
    for command in commands {
        let Some(handler) = native_form_children(command, "Action")
            .next()
            .and_then(|action| action.text())
            .filter(|handler| !handler.is_empty())
        else {
            continue;
        };
        let name = command.attribute("name").expect("native form command name");
        let owner = format!("{form_at}.Command.{name}");
        let execute = format!("{owner}.Event.Execute");
        native.collection(&owner, "Event", origin, BTreeSet::from([execute.clone()]));
        let view = read(native.authority, &execute);
        assert_eq!(view["props"]["handler"], handler, "{execute}");
        assert_eq!(view["props"]["state"], "implemented", "{execute}");
        let implementation = format!("{form_at}.Module.Form.Method.{handler}");
        assert_eq!(view["props"]["implementationAt"], implementation);
        assert_eq!(read(native.authority, &implementation)["kind"], "Method");
        check_native_projection_refusal(
            native.authority,
            &format!("{owner}.Event.NoSuchEvent"),
            "form Event `NoSuchEvent` was not found",
        );
    }
}

pub(super) fn native_columns(
    evidence: &mut EvidenceRows,
    required: &mut RequiredProjections,
    version: &str,
    line: PlatformLine,
) {
    const FORM: &str = "main:CommonForm.MainA";
    const NATIVE_FORM_PATH: &str = "CommonForms/MainA/Ext/Form.xml";
    const DATA_CORE: &str = "http://v8.1c.ru/8.1/data/core";

    let origin = fixture_path(&format!(
        "acceptance/native-form-columns/{version}/Form.xml"
    ));
    let reused = fixture_path(&format!(
        "acceptance/workspace-native-form-parameter/src-{version}"
    ));
    let source = tempfile::tempdir().unwrap();
    for relative in [
        "Configuration.xml",
        "CommonForms/MainA.xml",
        "Languages/Русский.xml",
    ] {
        let target = source.path().join(relative);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(reused.join(relative), target).unwrap();
    }
    let target = source.path().join(NATIVE_FORM_PATH);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    fs::copy(&origin, target).unwrap();
    assert!(root_registrations(source.path(), version)
        .iter()
        .any(|(kind, name)| kind == "CommonForm" && name == "MainA"));

    let raw = read_fixture_descriptor(&origin);
    let xml = parse_fixture_descriptor(&raw, &origin);
    let form = xml.root_element();
    assert!(form.has_tag_name((NATIVE_FORM_NS, "Form")));
    assert_eq!(form.attribute("version"), Some(version));
    let attributes = child(form, NATIVE_FORM_NS, "Attributes");
    let cancellation = CancellationToken::new();
    let authority = native_authority(
        &cancellation,
        source.path(),
        PlatformProfile::for_line(line).unwrap(),
    );
    let mut native = NativeProjections {
        authority: &authority,
        evidence,
        required,
    };
    let origin = fixture_origin(&origin);
    let primitive_type = |node| {
        child(child(node, NATIVE_FORM_NS, "Type"), DATA_CORE, "Type")
            .text()
            .unwrap()
    };
    let mut families = BTreeSet::new();
    for attribute in native_form_children(attributes, "Attribute") {
        let name = attribute.attribute("name").unwrap();
        let owner = format!("{FORM}.Attribute.{name}");
        let declared_type = primitive_type(attribute).strip_prefix("v8:").unwrap();
        assert!(families.insert(declared_type), "{owner}");
        assert_eq!(read(&authority, &owner)["props"]["type"], declared_type);

        let container = child(attribute, NATIVE_FORM_NS, "Columns");
        let columns: Vec<_> = native_form_children(container, "Column")
            .map(|node| {
                let name = node.attribute("name").unwrap();
                (name, primitive_type(node).strip_prefix("xs:").unwrap())
            })
            .collect();
        assert!(!columns.is_empty(), "{owner}");
        let members = named_addresses(&owner, "Column", columns.iter().map(|&(name, _)| name));
        native.collection(&owner, "Column", &origin, members);
        for (name, expected_type) in columns {
            let at = format!("{owner}.Column.{name}");
            let view = read(&authority, &at);
            assert_eq!(view["props"]["type"], expected_type, "{at}");
            assert_eq!(view["props"]["parentType"], declared_type, "{at}");
        }
        check_native_projection_refusal(
            &authority,
            &format!("{owner}.Column.Unregistered"),
            "form Column `Unregistered` was not found",
        );
    }
    assert_eq!(families, BTreeSet::from(["ValueTable", "ValueTree"]));
}
