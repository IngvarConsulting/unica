use super::*;

fn color_fixture(value: &str) -> Fixture {
    let fixture = Fixture::blank();
    let configuration = fixture.source.join("Configuration.xml");
    let xml = std::fs::read_to_string(&configuration).unwrap();
    std::fs::write(
        &configuration,
        xml.replace(
            "</ChildObjects>",
            "<StyleItem>Accent</StyleItem></ChildObjects>",
        ),
    )
    .unwrap();
    std::fs::create_dir_all(fixture.source.join("StyleItems")).unwrap();
    std::fs::write(fixture.source.join("StyleItems/Accent.xml"), format!("\u{feff}<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" xmlns:ui=\"http://v8.1c.ru/8.1/data/ui\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\" version=\"2.20\">\r\n<StyleItem uuid=\"1de549ac-39a2-49ce-9f31-c13040173894\"><Properties><Name>Accent</Name><Synonym/><Comment>keep</Comment><Type>Color</Type><Value xsi:type=\"ui:Color\">{value}</Value></Properties></StyleItem>\r\n</MetaDataObject>")).unwrap();
    fixture
}

#[test]
fn canonical_style_color_is_discovered_and_published_without_other_byte_changes() {
    let fixture = color_fixture("#F0F0F0");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
    assert!(observed.ok, "{observed:?}");
    assert!(
        observed.data.as_ref().unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["op"] == "props.set"),
        "Color must expose its value operation: {observed:?}"
    );
    let contract = call(
        &runtime,
        &fixture,
        ToolIdentity::View,
        json!({"at":format!("{at}.Can.props.set")}),
    );
    assert!(contract.ok, "{contract:?}");
    let before = std::fs::read(fixture.source.join("StyleItems/Accent.xml")).unwrap();
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray","Comment":"changed & retained"}}}]}),
    );
    let expected = String::from_utf8(before)
        .unwrap()
        .replace("#F0F0F0", "web:Gray")
        .replace(
            "<Comment>keep</Comment>",
            "<Comment>changed &amp; retained</Comment>",
        );
    assert_eq!(
        std::fs::read(fixture.source.join("StyleItems/Accent.xml")).unwrap(),
        expected.as_bytes()
    );
    let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
    assert_eq!(
        observed.data.as_ref().unwrap()["props"]["Value"],
        "web:Gray"
    );
}

#[test]
fn canonical_style_color_cycles_three_forms_and_repeats_without_writes() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    for value in ["web:Cream", "style:SpecialTextColor", "#ABCDEF"] {
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":value,"Comment":"mixed no-op"}}}]}),
        );
        let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        assert_eq!(observed.data.unwrap()["props"]["Value"], value);
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let plan = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":value,"Comment":"mixed no-op"}}}]}),
        );
        assert!(plan.ok, "{plan:?}");
        let data = plan.data.unwrap();
        assert_eq!(data["effects"], 0);
        let executed = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"executionToken":data["executionToken"]}),
        );
        assert!(executed.ok, "{executed:?}");
        assert!(executed.changed.is_empty());
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn canonical_non_color_and_opaque_values_have_no_color_contract() {
    for (old, new, value, code) in [
        (
            "<Type>Color</Type>",
            "<Type>Font</Type>",
            "#123456",
            "invalid_state",
        ),
        (
            "<Type>Color</Type>",
            "<Type>Border</Type>",
            "#123456",
            "invalid_state",
        ),
        (
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            "xmlns:web=\"urn:future\"",
            "web:Gray",
            "invalid_state",
        ),
        (
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/colors/web\"",
            "xmlns:web=\"http://v8.1c.ru/8.1/data/ui/style\"",
            "web:CustomColor",
            "invalid_state",
        ),
        (
            "<Type>Color</Type>",
            "<Type>Color</Type>",
            "#GG0000",
            "invalid_source",
        ),
    ] {
        let fixture = color_fixture(value);
        let runtime = runtime();
        let path = fixture.source.join("StyleItems/Accent.xml");
        let raw = std::fs::read_to_string(&path).unwrap().replace(old, new);
        std::fs::write(&path, raw).unwrap();
        let at = "main:StyleItem.Accent";
        let observed = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        assert!(observed.ok, "{observed:?}");
        assert!(observed.data.unwrap()["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["op"] == "props.set"));
        let contract = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.props.set")}),
        );
        assert!(contract.ok, "{contract:?}");
        let properties = &contract.data.as_ref().unwrap()["props"]["contract"]["argsSchema"]
            ["properties"]["values"]["properties"];
        assert!(properties.get("Value").is_none(), "{contract:?}");
        assert_eq!(properties["Comment"]["type"], "string");
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let rejected = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"#FFFFFF"}}}]}),
        );
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(rejected.diagnostics[0]["code"], code, "{rejected:?}");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
        let bad_arg = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":null}}}]}),
        );
        assert!(!bad_arg.ok);
        assert_eq!(
            bad_arg.diagnostics[0]["code"], "bad_value",
            "argument validation precedes descriptor state: {bad_arg:?}"
        );
        let before = std::fs::read(&path).unwrap();
        publish(
            &runtime,
            &fixture,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"comment":"comment without Color admission"}}}]}),
        );
        assert_eq!(
            std::fs::read(&path).unwrap(),
            String::from_utf8(before)
                .unwrap()
                .replace(
                    "<Comment>keep</Comment>",
                    "<Comment>comment without Color admission</Comment>"
                )
                .as_bytes()
        );
    }
}

#[test]
fn canonical_style_color_refuses_inexact_owner_versions_and_missing_values() {
    for (file, from, to, code) in [
        (
            "StyleItems/Accent.xml",
            "version=\"2.20\"",
            "version=\"2.19\"",
            "invalid_source",
        ),
        (
            "StyleItems/Accent.xml",
            "version=\"2.20\"",
            "version=\"2.&#50;0\"",
            "invalid_source",
        ),
        (
            "Configuration.xml",
            "version=\"2.20\"",
            "version=\"2.19\"",
            "invalid_source",
        ),
        (
            "StyleItems/Accent.xml",
            "<Value xsi:type=\"ui:Color\">#123456</Value>",
            "",
            "invalid_source",
        ),
        (
            "Configuration.xml",
            "<StyleItem>Accent</StyleItem>",
            "",
            "not_found",
        ),
    ] {
        let fixture = color_fixture("#123456");
        let runtime = runtime();
        let p = fixture.source.join(file);
        let raw = std::fs::read_to_string(&p).unwrap();
        assert!(raw.contains(from));
        std::fs::write(&p, raw.replace(from, to)).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let rejected = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:StyleItem.Accent","ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray"}}}]}),
        );
        assert!(!rejected.ok, "{rejected:?}");
        assert_eq!(
            rejected.diagnostics[0]["code"], code,
            "{file}: {rejected:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}

#[test]
fn canonical_style_color_bad_arguments_are_closed_and_saved_input_changes_are_fenced() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    for values in [
        json!({"Value":"web:NotAColor"}),
        json!({"Value":"#123"}),
        json!({"Value":false}),
        json!({"Value":123}),
        json!({"Value":{}}),
        json!({"Value":[]}),
        json!({"Value":"#FFFFFF","Type":"Font"}),
        json!({"Comment":false}),
        json!({"Comment":null}),
        json!({"Name":"unknown"}),
        json!({"Comment":"one","comment":"two"}),
        json!({"Value":"#123","Comment":"valid"}),
    ] {
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[{"op":"props.set","args":{"values":values}}]}),
        );
        assert!(!refused.ok, "{refused:?}");
        assert_eq!(refused.diagnostics[0]["code"], "bad_value");
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"#FFFFFF","Comment":"invalid\u{0001}"}}}]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "invalid_source");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before
    );
    let missing = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:StyleItem.Missing","ops":[{"op":"props.set","args":{"values":{"Value":"#FFFFFF"}}}]}),
    );
    assert!(!missing.ok);
    assert_eq!(missing.diagnostics[0]["code"], "not_found", "{missing:?}");
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"web:Gray","Comment":"planned"}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    let path = fixture.source.join("StyleItems/Accent.xml");
    let external = std::fs::read_to_string(&path)
        .unwrap()
        .replace("<Type>Color</Type>", "<Type>Font</Type>");
    std::fs::write(&path, external.as_bytes()).unwrap();
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "stale_revision");
    assert_eq!(std::fs::read(&path).unwrap(), external.as_bytes());
}

#[test]
fn canonical_style_color_contract_does_not_outlive_parent_format_admission() {
    for file in ["Configuration.xml", "StyleItems/Accent.xml"] {
        for version in ["2.19", "2.21", "2.&#50;0"] {
            let fixture = color_fixture("#123456");
            let runtime = runtime();
            let path = fixture.source.join(file);
            let source = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                &path,
                source.replace("version=\"2.20\"", &format!("version=\"{version}\"")),
            )
            .unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
            let observed = call(
                &runtime,
                &fixture,
                ToolIdentity::View,
                json!({"at":"main:StyleItem.Accent"}),
            );
            assert!(observed.ok, "{version}: {observed:?}");
            let data = observed.data.unwrap();
            assert_eq!(data["props"]["Value"], "#123456");
            assert!(
                !data["can"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|operation| operation["op"] == "props.set"),
                "{file}/{version}: a read-only owner chain cannot grant either writer: {data}"
            );
            let contract = call(
                &runtime,
                &fixture,
                ToolIdentity::View,
                json!({"at":"main:StyleItem.Accent.Can.props.set"}),
            );
            assert!(!contract.ok, "{file}/{version}: {contract:?}");
            for values in [
                json!({"Comment":"cannot publish"}),
                json!({"Value":"#ABCDEF"}),
            ] {
                let refused = call(
                    &runtime,
                    &fixture,
                    ToolIdentity::Apply,
                    json!({"at":"main:StyleItem.Accent","ops":[{"op":"props.set","args":{"values":values}}]}),
                );
                assert!(!refused.ok, "{file}/{version}: {refused:?}");
                assert_eq!(refused.diagnostics[0]["code"], "invalid_source");
                assert_eq!(
                    unica_workspace::test_support::tree_snapshot(&fixture.workspace),
                    before
                );
            }
        }
    }
}

#[test]
fn canonical_style_color_read_failure_after_node_projection_is_not_an_absent_capability() {
    use crate::infrastructure::v13_read::CompletedViewProjectionProbeForTest;
    use crate::infrastructure::v13_read_port::{
        review_clear_revision_identity_hooks, review_set_revision_identity_hooks,
    };
    use std::cell::Cell;
    use std::rc::Rc;
    struct RestoreSource {
        source: PathBuf,
        saved: PathBuf,
    }
    impl Drop for RestoreSource {
        fn drop(&mut self) {
            review_clear_revision_identity_hooks();
            if self.saved.exists() {
                std::fs::rename(&self.saved, &self.source).unwrap();
            }
        }
    }
    struct RestoreAfterRead {
        inner: crate::infrastructure::daemon::v13_service::CanonicalV13ReadService,
        source: PathBuf,
        saved: PathBuf,
    }
    impl CanonicalInvocationService for RestoreAfterRead {
        fn prepare(
            &self,
            invocation: &ActorBoundInvocation,
        ) -> Result<ExecutionClass, Box<DomainResult>> {
            self.inner.prepare(invocation)
        }
        fn execute(
            &self,
            invocation: &ActorBoundExecution,
            cancellation: CancellationToken,
        ) -> Result<DomainResult, InvocationFailure> {
            let result = self.inner.execute(invocation, cancellation);
            // The read failed transiently. Restore before the outer receipt
            // validates the root, so that validation cannot mask a lost error.
            if self.saved.exists() {
                std::fs::rename(&self.saved, &self.source).unwrap();
            }
            result
        }
    }
    for at in [
        "main:StyleItem.Accent",
        "main:StyleItem.Accent.Can.props.set",
    ] {
        let fixture = color_fixture("#123456");

        let probe = Rc::new(CompletedViewProjectionProbeForTest::start(
            "main:StyleItem.Accent",
        ));
        let fired = Rc::new(Cell::new(false));
        let fired_hook = Rc::clone(&fired);
        let probe_hook = Rc::clone(&probe);
        let source = fixture.source.clone();
        let saved = fixture.workspace.join("retained-source");
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(RestoreAfterRead {
                inner: crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(
                ),
                source: source.clone(),
                saved: saved.clone(),
            }),
            Arc::new(TokioClock),
        );
        let restore = RestoreSource {
            source: source.clone(),
            saved: saved.clone(),
        };
        review_set_revision_identity_hooks(
            move || {
                if !probe_hook.revisions().is_empty() && !fired_hook.replace(true) {
                    std::fs::rename(&source, &saved).unwrap();
                }
            },
            || {},
        );
        let result = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        drop(restore);
        assert!(
            fired.get(),
            "the failure must occur after the successful node projection"
        );
        assert!(
            !result.ok,
            "Color read I/O failure must not become a successful incomplete Can: {result:?}"
        );
        assert_eq!(
            result
                .diagnostics
                .first()
                .and_then(|d| d.get("code"))
                .and_then(Value::as_str),
            Some("provider_unavailable"),
            "{result:?}"
        );
        assert!(result.data.is_none(), "{result:?}");
    }
}

#[test]
fn style_item_contract_requires_independent_comment_and_color_evidence() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let at = "main:StyleItem.Accent";
    let path = fixture.source.join("StyleItems/Accent.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, original.replace("<Comment>keep</Comment>", "")).unwrap();
    let can = contract(&runtime, &fixture, at, "props.set");
    let fields = &can["argsSchema"]["properties"]["values"]["properties"];
    assert!(
        fields.get("Value").is_some(),
        "existing Color capability remains"
    );
    assert!(
        fields.get("Comment").is_none(),
        "missing Comment is not a write capability: {can}"
    );
    assert!(fields.get("comment").is_none());
    publish(
        &runtime,
        &fixture,
        json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Value":"#ABCDEF"}}}]}),
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        original
            .replace("<Comment>keep</Comment>", "")
            .replace("#123456", "#ABCDEF")
    );
    std::fs::write(
        &path,
        original
            .replace("<Comment>keep</Comment>", "")
            .replace("<Type>Color</Type>", "<Type>Font</Type>"),
    )
    .unwrap();
    let observed = view(&runtime, &fixture, at);
    assert!(
        !observed["can"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["op"] == "props.set"),
        "neither capability was proven: {observed}"
    );
}

#[test]
fn style_item_argument_errors_precede_failed_descriptor_evidence() {
    let fixture = color_fixture("#123456");
    let runtime = runtime();
    let path = fixture.source.join("StyleItems/Accent.xml");
    let original = std::fs::read_to_string(&path).unwrap();
    for damaged in [
        original.replace("1de549ac-39a2-49ce-9f31-c13040173894", "invalid-uuid"),
        original.replace("<Name>Accent</Name>", "<Name>Other</Name>"),
        "<broken".into(),
    ] {
        std::fs::write(&path, &damaged).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);
        for values in [
            json!({"Value":"invalid"}),
            json!({"Value":null}),
            json!({"Comment":"a","comment":"b"}),
            json!({"Unknown":"x"}),
        ] {
            let refused = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":"main:StyleItem.Accent","ops":[{"op":"props.set","args":{"values":values}}]}),
            );
            assert!(!refused.ok, "{refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "bad_value",
                "arguments first: {refused:?}"
            );
        }
        let batch = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:StyleItem.Accent","ops":[
                {"op":"props.set","args":{"values":{"Comment":"valid"}}},
                {"op":"props.set","args":{"values":{"Value":"invalid"}}}
            ]}),
        );
        assert!(!batch.ok, "{batch:?}");
        assert_eq!(
            batch.diagnostics[0]["code"], "bad_value",
            "all arguments precede descriptor state: {batch:?}"
        );
        assert!(
            batch
                .at
                .as_deref()
                .is_some_and(|path| path.starts_with("ops[1].args")),
            "{batch:?}"
        );
        let refused = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":"main:StyleItem.Accent","ops":[{"op":"props.set","args":{"values":{"Comment":"valid"}}}]}),
        );
        assert!(
            !refused.ok,
            "descriptor identity must be proven: {refused:?}"
        );
        assert_eq!(refused.diagnostics[0]["code"], "invalid_source");
        let read = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":"main:StyleItem.Accent"}),
        );
        assert!(
            !read.ok,
            "unreadable identity is not an absent capability: {read:?}"
        );
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.workspace),
            before
        );
    }
}
