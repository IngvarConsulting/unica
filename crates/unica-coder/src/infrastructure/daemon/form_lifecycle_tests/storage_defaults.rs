use super::*;
use serde_json::{json, Value};
use unica_execution_contract::invocation::DomainResult;

struct Fixture {
    root: tempfile::TempDir,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("src");
        for directory in ["SettingsStorages/Store/Forms", "Languages"] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
        }
        std::fs::write(root.path().join("v8project.yaml"),"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        let wrap = |body: &str| {
            format!("\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n{body}\r\n</MetaDataObject>")
        };
        std::fs::write(source.join("Configuration.xml"),wrap("<Configuration><Properties><Name>Fixture</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><SettingsStorage>Store</SettingsStorage></ChildObjects></Configuration>")).unwrap();
        std::fs::write(source.join("Languages/Русский.xml"),wrap("<Language uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Русский</Name><LanguageCode>ru</LanguageCode></Properties></Language>")).unwrap();
        std::fs::write(source.join("SettingsStorages/Store.xml"),wrap("<SettingsStorage uuid=\"11111111-1111-4111-8111-111111111111\"><Properties><Name>Store</Name><Comment>first\nsecond</Comment><DefaultSaveForm/><DefaultLoadForm/><AuxiliarySaveForm/><AuxiliaryLoadForm/></Properties><ChildObjects><Form>A</Form><Form>B</Form></ChildObjects></SettingsStorage>")).unwrap();
        for (name, id) in [
            ("A", "22222222-2222-4222-8222-222222222222"),
            ("B", "44444444-4444-4444-8444-444444444444"),
        ] {
            std::fs::write(source.join(format!("SettingsStorages/Store/Forms/{name}.xml")),wrap(&format!("<Form uuid=\"{id}\"><Properties><Name>{name}</Name><FormType>Managed</FormType></Properties></Form>"))).unwrap();
            let dir = source.join(format!("SettingsStorages/Store/Forms/{name}/Ext/Form"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.parent().unwrap().join("Form.xml"),"<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\"><AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/><Attributes/></Form>").unwrap();
            std::fs::write(dir.join("Module.bsl"), b"// retained\r\n").unwrap();
        }
        Self { root, source }
    }
    fn call(
        &self,
        runtime: &V5CanonicalInvocationRuntime,
        tool: ToolIdentity,
        args: Value,
    ) -> DomainResult {
        let hint = std::fs::canonicalize(self.root.path()).unwrap();
        direct_v5(
            runtime,
            InvocationRequest::new(tool, args, hint.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    }
    fn plan(&self, runtime: &V5CanonicalInvocationRuntime, values: Value) -> DomainResult {
        self.call(runtime,ToolIdentity::Apply,json!({"at":"main:SettingsStorage.Store","ops":[{"op":"form.set","args":{"values":values}}]}))
    }
    fn publish(&self, runtime: &V5CanonicalInvocationRuntime, values: Value) -> DomainResult {
        let before = unica_workspace::test_support::tree_snapshot(&self.source);
        let plan = self.plan(runtime, values);
        assert!(plan.ok, "{plan:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&self.source),
            before
        );
        let done = self.call(
            runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(done.ok, "{done:?}");
        done
    }
    fn replace(&self, path: &str, from: &str, to: &str) {
        let path = self.source.join(path);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(from));
        std::fs::write(path, text.replace(from, to)).unwrap();
    }
}
fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

#[test]
fn settings_storage_defaults_are_exposed_and_assign_existing_owned_forms() {
    let f = Fixture::new();
    let r = runtime();
    let view = f.call(
        &r,
        ToolIdentity::View,
        json!({"at":"main:SettingsStorage.Store"}),
    );
    assert!(view.ok, "{view:?}");
    let data = view.data.unwrap();
    let can = data["can"].as_array().unwrap();
    assert!(can.iter().any(|op| op["op"] == "form.set"));
    for forbidden in [
        "form.add",
        "object.create",
        "object.remove",
        "props.set",
        "attribute.add",
        "template.add",
    ] {
        assert!(
            !can.iter().any(|op| op["op"] == forbidden),
            "{forbidden}: {can:?}"
        );
    }
    let contract = f.call(
        &r,
        ToolIdentity::View,
        json!({"at":"main:SettingsStorage.Store.Can.form.set"}),
    );
    assert!(contract.ok, "{contract:?}");
    for (key, property) in [
        ("defaultSaveForm", "DefaultSaveForm"),
        ("defaultLoadForm", "DefaultLoadForm"),
        ("auxiliarySaveForm", "AuxiliarySaveForm"),
        ("auxiliaryLoadForm", "AuxiliaryLoadForm"),
    ] {
        assert_eq!(data["props"][property], "");
        for name in ["A", "B"] {
            let before = unica_workspace::test_support::tree_snapshot(&f.source);
            let raw =
                String::from_utf8(before[Path::new("SettingsStorages/Store.xml")].clone()).unwrap();
            let doc = roxmltree::Document::parse(raw.trim_start_matches('\u{feff}')).unwrap();
            let slot = doc
                .descendants()
                .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", property)))
                .unwrap();
            let offset = raw.len() - raw.trim_start_matches('\u{feff}').len();
            let mut expected = raw.clone();
            expected.replace_range(
                slot.range().start + offset..slot.range().end + offset,
                &format!("<{property}>SettingsStorage.Store.Form.{name}</{property}>"),
            );
            f.publish(
                &r,
                json!({key:format!("main:SettingsStorage.Store.Form.{name}")}),
            );
            assert_exact_effects(
                &f.source,
                &before,
                &[],
                &["SettingsStorages/Store.xml"],
                &[],
            );
            assert_eq!(
                std::fs::read_to_string(f.source.join("SettingsStorages/Store.xml")).unwrap(),
                expected
            );
            let read = f.call(
                &r,
                ToolIdentity::View,
                json!({"at":"main:SettingsStorage.Store"}),
            );
            assert!(read.ok, "{read:?}");
            assert_eq!(
                read.data.unwrap()["props"][property],
                format!("SettingsStorage.Store.Form.{name}")
            );
            let before = unica_workspace::test_support::tree_snapshot(&f.source);
            let noop = f.plan(&r, json!({key:name}));
            assert!(noop.ok, "{noop:?}");
            assert_eq!(noop.data.as_ref().unwrap()["effects"], 0);
            let done = f.call(
                &r,
                ToolIdentity::Apply,
                json!({"executionToken":noop.data.unwrap()["executionToken"]}),
            );
            assert!(done.ok);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&f.source),
                before
            );
        }
    }
}

#[test]
fn settings_storage_defaults_refuse_wrong_input_and_roll_back_a_failed_later_slot() {
    let f = Fixture::new();
    let r = runtime();
    for (values, code) in [
        (
            json!({"defaultSaveForm":"main:SettingsStorage.Other.Form.A"}),
            "bad_value",
        ),
        (
            json!({"defaultSaveForm":"other:SettingsStorage.Store.Form.A"}),
            "bad_value",
        ),
        (json!({"defaultSaveForm":"main:CommonForm.A"}), "bad_value"),
        (
            json!({"defaultSaveForm":"main:SettingsStorage.Store.Form"}),
            "bad_value",
        ),
        (json!({"defaultSaveForm":"Missing"}), "not_found"),
        (json!({"defaultForm":"A"}), "bad_value"),
        (json!({"defaultSaveForm":null}), "bad_value"),
        (json!({"defaultSaveForm":true}), "bad_value"),
        (json!({"defaultSaveForm":"  "}), "bad_value"),
        (json!({}), "bad_value"),
        (
            json!({"defaultLoadForm":"A","defaultSaveForm":"Missing"}),
            "not_found",
        ),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let x = f.plan(&r, values);
        assert!(!x.ok, "{x:?}");
        assert_eq!(x.diagnostics[0]["code"], code, "{x:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
    for operation in ["form.add", "object.create", "object.remove"] {
        let x = f.call(
            &r,
            ToolIdentity::View,
            json!({"at":format!("main:SettingsStorage.Store.Can.{operation}")}),
        );
        assert!(!x.ok, "{x:?}");
    }
}

#[test]
fn settings_storage_defaults_refuse_damaged_owners_forms_and_versions() {
    for (path,from,to,code) in [
        ("Configuration.xml","<SettingsStorage>Store</SettingsStorage>","","not_found"),
        ("Configuration.xml","<SettingsStorage>Store</SettingsStorage>","<SettingsStorage>Store<!--x-->Wrong</SettingsStorage>","not_found"),
        ("SettingsStorages/Store.xml","<Name>Store</Name>","<Name>Store<!--x-->Wrong</Name>","invalid_source"),
        ("SettingsStorages/Store.xml","<DefaultSaveForm/>","<DefaultSaveForm/><DefaultSaveForm/>","invalid_source"),
        ("SettingsStorages/Store.xml","<DefaultSaveForm/>","<DefaultSaveForm xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"/>","invalid_source"),
        ("SettingsStorages/Store.xml","<Form>A</Form>","","not_found"),
        ("SettingsStorages/Store/Forms/A.xml","<Name>A</Name>","<Name>Wrong</Name>","invalid_source"),
        ("SettingsStorages/Store/Forms/A.xml","<FormType>Managed</FormType>","<FormType>Ordinary</FormType>","bad_value"),
        ("SettingsStorages/Store/Forms/A.xml","<FormType>Managed</FormType>","<FormType>Managed<!--x-->Wrong</FormType>","bad_value"),
        ("SettingsStorages/Store/Forms/A/Ext/Form.xml","http://v8.1c.ru/8.3/xcf/logform","urn:foreign","invalid_source"),
        ("SettingsStorages/Store/Forms/A/Ext/Form.xml","<Form xmlns=","<Form xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\" xmlns=","invalid_source"),
    ]{let f=Fixture::new();let r=runtime();f.replace(path,from,to);let before=unica_workspace::test_support::tree_snapshot(&f.source);let x=f.plan(&r,json!({"defaultSaveForm":"A"}));assert!(!x.ok,"{path}: {x:?}");assert_eq!(x.diagnostics[0]["code"],code,"{path}: {x:?}");assert_eq!(unica_workspace::test_support::tree_snapshot(&f.source),before);}
    for path in [
        "Configuration.xml",
        "SettingsStorages/Store.xml",
        "SettingsStorages/Store/Forms/A.xml",
        "SettingsStorages/Store/Forms/A/Ext/Form.xml",
    ] {
        for version in ["2.19", "2.21", "2.&#50;0"] {
            let f = Fixture::new();
            let r = runtime();
            f.replace(path, "version=\"2.20\"", &format!("version=\"{version}\""));
            let before = unica_workspace::test_support::tree_snapshot(&f.source);
            let x = f.plan(&r, json!({"defaultSaveForm":"A"}));
            assert!(!x.ok, "{path}/{version}: {x:?}");
            assert_eq!(x.diagnostics[0]["code"], "invalid_source", "{x:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&f.source),
                before
            );
        }
    }
    for path in [
        "SettingsStorages/Store/Forms/A.xml",
        "SettingsStorages/Store/Forms/A/Ext/Form.xml",
    ] {
        let f = Fixture::new();
        let r = runtime();
        std::fs::remove_file(f.source.join(path)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let x = f.plan(&r, json!({"defaultSaveForm":"A"}));
        assert!(!x.ok, "{x:?}");
        assert_eq!(x.diagnostics[0]["code"], "not_found");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
}

#[test]
fn settings_storage_defaults_bind_owner_form_and_content_revisions() {
    for path in [
        "Configuration.xml",
        "SettingsStorages/Store.xml",
        "SettingsStorages/Store/Forms/A.xml",
        "SettingsStorages/Store/Forms/A/Ext/Form.xml",
    ] {
        let f = Fixture::new();
        let r = runtime();
        let p = f.plan(&r, json!({"defaultSaveForm":"A"}));
        assert!(p.ok, "{p:?}");
        let file = f.source.join(path);
        let mut bytes = std::fs::read(&file).unwrap();
        bytes.extend_from_slice(b"\r\n<!-- external -->\r\n");
        std::fs::write(file, bytes).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let x = f.call(
            &r,
            ToolIdentity::Apply,
            json!({"executionToken":p.data.unwrap()["executionToken"]}),
        );
        assert!(!x.ok, "{x:?}");
        assert_eq!(x.diagnostics[0]["code"], "stale_revision");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
}

#[test]
fn settings_storage_defaults_keep_independent_read_limits_and_preserve_headers_when_clearing() {
    let f = Fixture::new();
    let r = runtime();
    f.replace(
        "SettingsStorages/Store.xml",
        "<DefaultLoadForm/>",
        "<DefaultLoadForm><Future/></DefaultLoadForm>",
    );
    f.replace("SettingsStorages/Store.xml", "\u{feff}", "");
    f.replace("SettingsStorages/Store.xml","<DefaultSaveForm/>","<d:DefaultSaveForm xmlns:d=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\" false \"></d:DefaultSaveForm>");
    f.replace(
        "SettingsStorages/Store/Forms/A.xml",
        "<FormType>Managed</FormType>",
        "<FormType>Man<!--keep-->aged</FormType>",
    );
    let viewed = f.call(
        &r,
        ToolIdentity::View,
        json!({"at":"main:SettingsStorage.Store"}),
    );
    assert!(viewed.ok, "{viewed:?}");
    let data = viewed.data.unwrap();
    assert!(data["props"].get("DefaultLoadForm").is_none());
    assert_eq!(data["props"]["DefaultSaveForm"], "");
    assert!(data["limits"].to_string().contains("DefaultLoadForm"));
    let before = std::fs::read(f.source.join("SettingsStorages/Store.xml")).unwrap();
    f.publish(&r, json!({"defaultSaveForm":"A"}));
    let expected = String::from_utf8(before).unwrap().replace(
        "></d:DefaultSaveForm>",
        ">SettingsStorage.Store.Form.A</d:DefaultSaveForm>",
    );
    assert_eq!(
        std::fs::read_to_string(f.source.join("SettingsStorages/Store.xml")).unwrap(),
        expected
    );
    f.publish(&r, json!({"defaultSaveForm":""}));
    assert_eq!(
        std::fs::read_to_string(f.source.join("SettingsStorages/Store.xml")).unwrap(),
        expected.replace(
            ">SettingsStorage.Store.Form.A</d:DefaultSaveForm>",
            "></d:DefaultSaveForm>"
        )
    );
    let before = unica_workspace::test_support::tree_snapshot(&f.source);
    let p = f.plan(&r, json!({"defaultSaveForm":""}));
    assert!(p.ok, "{p:?}");
    assert_eq!(p.data.as_ref().unwrap()["effects"], 0);
    assert!(
        f.call(
            &r,
            ToolIdentity::Apply,
            json!({"executionToken":p.data.unwrap()["executionToken"]})
        )
        .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&f.source),
        before
    );
}
