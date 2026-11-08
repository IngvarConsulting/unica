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
        for directory in [
            "Reports/Totals/Forms/Own/Ext/Form",
            "Constants",
            "Languages",
            "CommonForms",
        ] {
            std::fs::create_dir_all(source.join(directory)).unwrap();
        }
        std::fs::write(root.path().join("v8project.yaml"),"format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n").unwrap();
        let wrap = |body: &str| {
            format!("\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">\r\n{body}\r\n</MetaDataObject>")
        };
        let mut registrations = String::new();
        for (name, main, purpose) in [
            ("Main", "ReportObject", Some("Main")),
            ("Settings", "ReportObject", Some("Settings")),
            ("Variant", "ReportObject", Some("Variant")),
            ("Constants", "ConstantsSet", None),
        ] {
            for suffix in ["A", "B"] {
                let name = format!("{name}{suffix}");
                registrations.push_str(&format!("<CommonForm>{name}</CommonForm>"));
                std::fs::write(source.join(format!("CommonForms/{name}.xml")),wrap(&format!("<CommonForm uuid=\"44444444-4444-4444-8444-444444444444\"><Properties><Name>{name}</Name><FormType>Managed</FormType></Properties></CommonForm>"))).unwrap();
                let directory = source.join(format!("CommonForms/{name}/Ext/Form"));
                std::fs::create_dir_all(&directory).unwrap();
                let purpose = purpose
                    .map(|p| format!("<ReportFormType>{p}</ReportFormType>"))
                    .unwrap_or_default();
                std::fs::write(directory.parent().unwrap().join("Form.xml"),format!("\u{feff}<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" xmlns:v8=\"http://v8.1c.ru/8.1/data/core\" xmlns:cfg=\"http://v8.1c.ru/8.1/data/enterprise/current-config\" version=\"2.20\">{purpose}<AutoCommandBar name=\"FormCommandBar\" id=\"-1\"/><Attributes><Attribute name=\"Object\" id=\"1\"><Type><v8:Type>cfg:{main}</v8:Type></Type><MainAttribute>true</MainAttribute></Attribute></Attributes></Form>")).unwrap();
                std::fs::write(directory.join("Module.bsl"), b"// retained\r\n").unwrap();
            }
        }
        std::fs::write(source.join("Configuration.xml"),wrap(&format!("<Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Constant>Flag</Constant><Report>Totals</Report>{registrations}</ChildObjects></Configuration>"))).unwrap();
        std::fs::write(source.join("Languages/Русский.xml"),wrap("<Language uuid=\"33333333-3333-4333-8333-333333333333\"><Properties><Name>Русский</Name><LanguageCode>ru</LanguageCode></Properties></Language>")).unwrap();
        std::fs::write(source.join("Reports/Totals.xml"),wrap("<Report uuid=\"11111111-1111-4111-8111-111111111111\"><Properties><Name>Totals</Name><Synonym/><Comment>first\nsecond</Comment><DefaultForm>Report.Totals.Form.Own</DefaultForm><DefaultSettingsForm/><DefaultVariantForm/></Properties><ChildObjects><Form>Own</Form></ChildObjects></Report>")).unwrap();
        std::fs::write(source.join("Constants/Flag.xml"),wrap("<Constant uuid=\"22222222-2222-4222-8222-222222222222\"><Properties><Name>Flag</Name><Synonym/><Comment>keep</Comment><DefaultForm/></Properties></Constant>")).unwrap();
        std::fs::write(source.join("Reports/Totals/Forms/Own.xml"),wrap("<Form uuid=\"55555555-5555-4555-8555-555555555555\"><Properties><Name>Own</Name><FormType>Managed</FormType></Properties></Form>")).unwrap();
        std::fs::write(
            source.join("Reports/Totals/Forms/Own/Ext/Form.xml"),
            "<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\"><Attributes/></Form>",
        )
        .unwrap();
        std::fs::write(
            source.join("Reports/Totals/Forms/Own/Ext/Form/Module.bsl"),
            b"// own module\r\n",
        )
        .unwrap();
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
    fn preview(
        &self,
        runtime: &V5CanonicalInvocationRuntime,
        at: &str,
        key: &str,
        value: &str,
    ) -> DomainResult {
        self.call(
            runtime,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"form.set","args":{"values":{key:value}}}]}),
        )
    }
    fn replace(&self, path: &str, from: &str, to: &str) {
        let path = self.source.join(path);
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(before.contains(from));
        std::fs::write(path, before.replace(from, to)).unwrap();
    }
}
fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

#[test]
fn common_form_targets_assign_report_and_constant_slots_without_rewriting_forms() {
    let f = Fixture::new();
    let runtime = runtime();
    for (at, path, key, xml, stem) in [
        (
            "main:Report.Totals",
            "Reports/Totals.xml",
            "defaultForm",
            "DefaultForm",
            "Main",
        ),
        (
            "main:Report.Totals",
            "Reports/Totals.xml",
            "defaultSettingsForm",
            "DefaultSettingsForm",
            "Settings",
        ),
        (
            "main:Report.Totals",
            "Reports/Totals.xml",
            "defaultVariantForm",
            "DefaultVariantForm",
            "Variant",
        ),
        (
            "main:Constant.Flag",
            "Constants/Flag.xml",
            "defaultForm",
            "DefaultForm",
            "Constants",
        ),
    ] {
        let contract = f.call(
            &runtime,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.form.set")}),
        );
        assert!(contract.ok, "{contract:?}");
        for suffix in ["A", "B"] {
            let before = unica_workspace::test_support::tree_snapshot(&f.source);
            let value = format!("main:CommonForm.{stem}{suffix}");
            let plan = f.preview(&runtime, at, key, &value);
            assert!(plan.ok, "{plan:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&f.source),
                before
            );
            let published = f.call(
                &runtime,
                ToolIdentity::Apply,
                json!({"executionToken":plan.data.as_ref().unwrap()["executionToken"]}),
            );
            assert!(published.ok, "{published:?}");
            assert_exact_effects(&f.source, &before, &[], &[path], &[]);
            let before_text = String::from_utf8(before[Path::new(path)].clone()).unwrap();
            let after = std::fs::read_to_string(f.source.join(path)).unwrap();
            let doc =
                roxmltree::Document::parse(before_text.trim_start_matches('\u{feff}')).unwrap();
            let old = doc
                .descendants()
                .find(|n| n.has_tag_name(("http://v8.1c.ru/8.3/MDClasses", xml)))
                .unwrap();
            let replacement = format!("<{xml}>CommonForm.{stem}{suffix}</{xml}>");
            let offset = before_text.len() - before_text.trim_start_matches('\u{feff}').len();
            let mut expected = before_text.clone();
            expected.replace_range(
                old.range().start + offset..old.range().end + offset,
                &replacement,
            );
            assert_eq!(after, expected, "only addressed slot may change");
            let read = f.call(&runtime, ToolIdentity::View, json!({"at":at}));
            assert!(read.ok, "{read:?}");
            let unchanged = unica_workspace::test_support::tree_snapshot(&f.source);
            let noop = f.preview(&runtime, at, key, &value);
            assert!(noop.ok, "{noop:?}");
            assert_eq!(noop.data.as_ref().unwrap()["effects"], 0);
            let published = f.call(
                &runtime,
                ToolIdentity::Apply,
                json!({"executionToken":noop.data.unwrap()["executionToken"]}),
            );
            assert!(published.ok);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&f.source),
                unchanged
            );
        }
    }
    for value in [
        "main:Report.Totals.Form.Own",
        "main:CommonForm.MainA",
        "Own",
    ] {
        let plan = f.preview(&runtime, "main:Report.Totals", "defaultForm", value);
        assert!(plan.ok, "{plan:?}");
        assert!(
            f.call(
                &runtime,
                ToolIdentity::Apply,
                json!({"executionToken":plan.data.unwrap()["executionToken"]})
            )
            .ok
        );
    }
    let report = std::fs::read_to_string(f.source.join("Reports/Totals.xml")).unwrap();
    assert!(report.contains("<DefaultForm>Report.Totals.Form.Own</DefaultForm>"));
    assert!(report.contains("<DefaultSettingsForm>CommonForm.SettingsB</DefaultSettingsForm>"));
    assert!(report.contains("<DefaultVariantForm>CommonForm.VariantB</DefaultVariantForm>"));
    assert_eq!(report.matches("<Form>Own</Form>").count(), 1);
}

#[test]
fn common_form_targets_refuse_wrong_addresses_and_incompatible_forms_without_writes() {
    let f = Fixture::new();
    let runtime = runtime();
    for (at, key, value, code) in [
        (
            "main:Report.Totals",
            "defaultForm",
            "main:CommonForm",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "main:Report.Totals.Form",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "main:Report.Other.Form.Own",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "other:CommonForm.MainA",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "main:Report.Totals",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "main:CommonForm.Absent",
            "not_found",
        ),
        (
            "main:Report.Totals",
            "defaultSettingsForm",
            "main:CommonForm.MainA",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultVariantForm",
            "main:CommonForm.SettingsA",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "defaultForm",
            "main:CommonForm.ConstantsA",
            "bad_value",
        ),
        (
            "main:Constant.Flag",
            "defaultForm",
            "main:CommonForm.MainA",
            "bad_value",
        ),
        (
            "main:Report.Totals",
            "auxiliaryForm",
            "main:CommonForm.MainA",
            "bad_value",
        ),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let x = f.preview(&runtime, at, key, value);
        assert!(!x.ok, "{value}: {x:?}");
        assert_eq!(x.diagnostics[0]["code"], code, "{value}: {x:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&f.source),
            before
        );
    }
}

#[test]
fn common_form_targets_reject_ambiguous_or_malformed_evidence() {
    for (path,from,to,code) in [
        ("CommonForms/MainA/Ext/Form.xml","<Attribute name=\"Object\"","<Attribute xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\" name=\"Object\"","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","<Form xmlns=","<Form xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\" xmlns=","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","cfg:ReportObject","cfg:ReportObject<!--x-->Wrong","bad_value"),
        ("CommonForms/MainA/Ext/Form.xml","<ReportFormType>Main</ReportFormType>","<ReportFormType>Main<!--x-->Wrong</ReportFormType>","bad_value"),
        ("CommonForms/MainA/Ext/Form.xml","<MainAttribute>true</MainAttribute>","<MainAttribute>true<!--x-->false</MainAttribute>","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","version=\"2.20\"","version=\"2.19\"","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","version=\"2.20\"","version=\"2.21\"","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","version=\"2.20\"","version=\"2.&#50;0\"","invalid_source"),
        ("Reports/Totals.xml","<DefaultForm>Report.Totals.Form.Own</DefaultForm>","<DefaultForm/><DefaultForm/>","invalid_source"),
        ("Reports/Totals.xml","<DefaultForm>Report.Totals.Form.Own</DefaultForm>","<DefaultForm xmlns=\"urn:foreign\"/>","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","<Type><v8:Type>","<Type xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\"true\"><v8:Type>","invalid_source"),
        ("CommonForms/MainA.xml","<Name>MainA</Name>","<Name>Different</Name>","invalid_source"),
        ("CommonForms/MainA.xml","<Name>MainA</Name>","<Name>MainA</Name><Name>MainA</Name>","invalid_source"),
        ("CommonForms/MainA.xml","<FormType>Managed</FormType>","<FormType>Ordinary</FormType>","bad_value"),
        ("CommonForms/MainA.xml","version=\"2.20\"","version=\"2.19\"","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","cfg:ReportObject",":ReportObject","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","<MainAttribute>true</MainAttribute>","<MainAttribute>true</MainAttribute><MainAttribute>false</MainAttribute>","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","</Attributes>","<Attribute name=\"Other\" id=\"2\"><MainAttribute>true</MainAttribute><MainAttribute>false</MainAttribute></Attribute></Attributes>","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","<MainAttribute>true</MainAttribute>","<MainAttribute>maybe</MainAttribute>","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","cfg:ReportObject","cfg:ReportObject|cfg:ReportObject","invalid_source"),
        ("CommonForms/MainA/Ext/Form.xml","http://v8.1c.ru/8.1/data/enterprise/current-config","urn:foreign","bad_value"),
        ("Configuration.xml","<CommonForm>MainA</CommonForm>","","not_found"),
    ] {let f=Fixture::new();let runtime=runtime();f.replace(path,from,to);let before=unica_workspace::test_support::tree_snapshot(&f.source);let x=f.preview(&runtime,"main:Report.Totals","defaultForm","main:CommonForm.MainA");assert!(!x.ok,"{path}/{from}: {x:?}");assert_eq!(x.diagnostics[0]["code"],code,"{x:?}");assert_eq!(unica_workspace::test_support::tree_snapshot(&f.source),before);}
}

#[test]
fn common_form_targets_bind_descriptor_and_content_revisions() {
    for path in ["CommonForms/MainA.xml", "CommonForms/MainA/Ext/Form.xml"] {
        let f = Fixture::new();
        let runtime = runtime();
        let plan = f.preview(
            &runtime,
            "main:Report.Totals",
            "defaultForm",
            "main:CommonForm.MainA",
        );
        assert!(plan.ok, "{plan:?}");
        let p = f.source.join(path);
        let mut text = std::fs::read_to_string(&p).unwrap();
        text.push_str("\r\n<!-- external -->\r\n");
        std::fs::write(p, text).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&f.source);
        let x = f.call(
            &runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]}),
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
fn common_form_targets_accept_namespace_aliases_boolean_one_and_absent_bom() {
    let f = Fixture::new();
    let runtime = runtime();
    f.replace("CommonForms/MainA/Ext/Form.xml", "xmlns:cfg=", "xmlns:q=");
    f.replace(
        "CommonForms/MainA/Ext/Form.xml",
        "cfg:ReportObject",
        "q:ReportObject",
    );
    f.replace(
        "CommonForms/MainA/Ext/Form.xml",
        "<MainAttribute>true</MainAttribute>",
        "<MainAttribute>1</MainAttribute>",
    );
    f.replace(
        "CommonForms/MainA/Ext/Form.xml",
        "<ReportFormType>Main</ReportFormType>",
        "",
    );
    f.replace("Reports/Totals.xml", "\u{feff}", "");
    let plan = f.preview(
        &runtime,
        "main:Report.Totals",
        "defaultForm",
        "main:CommonForm.MainA",
    );
    assert!(plan.ok, "{plan:?}");
    assert!(
        f.call(
            &runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]})
        )
        .ok
    );
    let before = unica_workspace::test_support::tree_snapshot(&f.source);
    assert!(!before[Path::new("Reports/Totals.xml")].starts_with(b"\xef\xbb\xbf"));
    let plan = f.preview(
        &runtime,
        "main:Report.Totals",
        "defaultForm",
        "CommonForm.MainA",
    );
    assert!(plan.ok, "{plan:?}");
    assert_eq!(plan.data.as_ref().unwrap()["effects"], 0);
    assert!(
        f.call(
            &runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]})
        )
        .ok
    );
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&f.source),
        before
    );
}

#[test]
fn common_form_targets_preserve_full_scalar_text_and_prefixed_slot_headers() {
    let f = Fixture::new();
    let runtime = runtime();
    f.replace(
        "CommonForms/MainA/Ext/Form.xml",
        "cfg:ReportObject",
        "cfg:Report<!--keep-->Object",
    );
    f.replace(
        "CommonForms/MainA/Ext/Form.xml",
        "<ReportFormType>Main</ReportFormType>",
        "<ReportFormType>Ma<![CDATA[in]]></ReportFormType>",
    );
    f.replace("CommonForms/MainA/Ext/Form.xml","<MainAttribute>true</MainAttribute>","<MainAttribute xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:nil=\" false \">tr<![CDATA[ue]]></MainAttribute>");
    f.replace("Reports/Totals.xml","<DefaultForm>Report.Totals.Form.Own</DefaultForm>","<d:DefaultForm xmlns:d=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:keep=\"urn:keep\">Report.Totals.Form.Own</d:DefaultForm>");
    let before = unica_workspace::test_support::tree_snapshot(&f.source);
    let plan = f.preview(
        &runtime,
        "main:Report.Totals",
        "defaultForm",
        "main:CommonForm.MainA",
    );
    assert!(plan.ok, "{plan:?}");
    assert!(
        f.call(
            &runtime,
            ToolIdentity::Apply,
            json!({"executionToken":plan.data.unwrap()["executionToken"]})
        )
        .ok
    );
    assert_exact_effects(&f.source, &before, &[], &["Reports/Totals.xml"], &[]);
    assert_eq!(
        std::fs::read(f.source.join("Reports/Totals.xml")).unwrap(),
        String::from_utf8(before[Path::new("Reports/Totals.xml")].clone())
            .unwrap()
            .replace("Report.Totals.Form.Own", "CommonForm.MainA")
            .as_bytes()
    );
}
