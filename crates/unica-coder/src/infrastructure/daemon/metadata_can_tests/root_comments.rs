use super::*;

fn owners() -> Vec<(String, String, String)> {
    let manifest: Value = serde_json::from_slice(
        &std::fs::read(
            repo_root()
                .join("tests/fixtures/acceptance/workspace-root-comments/native-manifest.json"),
        )
        .unwrap(),
    )
    .unwrap();
    manifest["owners"]
        .as_array()
        .unwrap()
        .iter()
        .map(|owner| {
            (
                owner[0].as_str().unwrap().to_string(),
                owner[1].as_str().unwrap().to_string(),
                owner[2].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn outside_comment(bytes: &[u8]) -> Vec<u8> {
    let text = std::str::from_utf8(bytes).unwrap();
    let body = text.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse(body).unwrap();
    let properties = document
        .root_element()
        .children()
        .find(|n| n.is_element())
        .unwrap()
        .children()
        .find(|n| n.tag_name().name() == "Properties")
        .unwrap();
    let range = properties
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "Comment")
        .unwrap()
        .range();
    let offset = text.len() - body.len();
    [
        bytes[..offset + range.start].to_vec(),
        bytes[offset + range.end..].to_vec(),
    ]
    .concat()
}

fn assert_source_refusal(result: &DomainResult, at: &str) {
    assert!(!result.ok, "{at}: {result:?}");
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "invalid_source"),
        "{at}: expected the source defect, not an unrelated refusal: {result:?}"
    );
}

#[test]
fn root_comment_requires_registered_owner_even_when_descriptor_exists() {
    let runtime = runtime();
    for (kind, name) in [
        ("CommonAttribute", "Probe"),
        ("FunctionalOption", "Probe"),
        ("WSReference", "Probe"),
        ("CommonForm", "Probe"),
        ("Language", "Русский"),
    ] {
        let fixture = Fixture::new("workspace-root-comments");
        let root = fixture.source.join("Configuration.xml");
        let source = std::fs::read_to_string(&root).unwrap();
        let registration = format!("<{kind}>{name}</{kind}>");
        assert_eq!(source.matches(&registration).count(), 1);
        std::fs::write(&root, source.replace(&registration, "")).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let at = if kind == "CommonForm" {
            format!("main:{kind}.{name}.Metadata")
        } else {
            format!("main:{kind}.{name}")
        };
        let named = call(&runtime, &fixture, ToolIdentity::View, json!({"at":at}));
        let detailed = call(
            &runtime,
            &fixture,
            ToolIdentity::View,
            json!({"at":format!("{at}.Can.props.set")}),
        );
        for comment in ["orphan", ""] {
            let preview = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Comment":comment}}}]}),
            );
            let published = preview
                .data
                .as_ref()
                .and_then(|data| data.get("executionToken"))
                .map(|token| {
                    call(
                        &runtime,
                        &fixture,
                        ToolIdentity::Apply,
                        json!({"executionToken":token}),
                    )
                });
            assert!(!preview.ok,
                "{at}: named={named:?}; detailed={detailed:?}; preview={preview:?}; published={published:?}");
            assert!(
                preview
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic["code"] == "not_found"),
                "{at}: {preview:?}"
            );
            assert!(
                preview
                    .data
                    .as_ref()
                    .and_then(|data| data.get("executionToken"))
                    .is_none(),
                "{preview:?}"
            );
            assert!(preview.changed.is_empty(), "{preview:?}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before,
                "{at}"
            );
        }
        assert!(
            !named.ok && !detailed.ok,
            "{at}: named={named:?}; detailed={detailed:?}"
        );
    }
}

#[test]
fn root_comment_saved_plan_rejects_registration_drift_without_overwriting_it() {
    let fixture = Fixture::new("workspace-root-comments");
    let runtime = runtime();
    let plan = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:CommonAttribute.Probe","ops":[{"op":"props.set","args":{"values":{"Comment":"planned"}}}]}),
    );
    assert!(plan.ok, "{plan:?}");
    let root = fixture.source.join("Configuration.xml");
    let source = std::fs::read_to_string(&root).unwrap();
    std::fs::write(
        &root,
        source.replace("<CommonAttribute>Probe</CommonAttribute>", ""),
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let refused = call(
        &runtime,
        &fixture,
        ToolIdentity::Apply,
        json!({"executionToken":plan.data.unwrap()["executionToken"]}),
    );
    assert!(!refused.ok, "{refused:?}");
    assert_eq!(
        refused.diagnostics[0]["code"], "stale_revision",
        "{refused:?}"
    );
    assert!(refused.changed.is_empty(), "{refused:?}");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}

#[test]
fn medium_root_comments_preview_publish_read_and_repeat_preserve_native_companions() {
    let fixture = Fixture::new("workspace-root-comments");
    let form_dir = fixture.source.join("CommonForms/Probe/Ext");
    std::fs::create_dir_all(form_dir.join("Form")).unwrap();
    std::fs::write(
        form_dir.join("Form.xml"),
        std::fs::read(
            repo_root().join("tests/fixtures/platform_8_3_27/new-xml/native-form-content.xml"),
        )
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        form_dir.join("Form/Module.bsl"),
        "// Preserve form module\r\n",
    )
    .unwrap();
    let style_dir = fixture.source.join("Styles/Probe/Ext");
    std::fs::create_dir_all(&style_dir).unwrap();
    std::fs::write(
        style_dir.join("Style.xml"),
        "<Style>untouched companion</Style>\n",
    )
    .unwrap();
    let runtime = runtime();
    for (kind, name, directory) in owners() {
        let owner = format!("main:{kind}.{name}");
        let at = if kind == "CommonForm" {
            format!("{owner}.Metadata")
        } else {
            owner
        };
        let can = contract(&runtime, &fixture, &at, "props.set");
        assert_eq!(
            can["argsSchema"]["properties"]["values"]["properties"]
                .as_object()
                .unwrap()
                .len(),
            if kind == "FunctionalOption" {
                6
            } else if kind == "WSReference" {
                4
            } else if kind == "StyleItem" || kind == "Language" {
                // Comment, its alias and Value or LanguageCode.
                3
            } else if kind == "DocumentNumerator" {
                // Five numerator scalars plus Comment and its alias.
                7
            } else {
                2
            },
            "{at}"
        );
        let relative = PathBuf::from(directory).join(format!("{name}.xml"));
        let path = fixture.source.join(&relative);
        let source = std::fs::read_to_string(&path).unwrap();
        let source = source
            .trim_start_matches('\u{feff}')
            .replace("<Name>", "\n<Name>");
        let bilingual = "<Synonym><v8:item><v8:lang>ru</v8:lang><v8:content>Русский</v8:content></v8:item><v8:item><v8:lang>en</v8:lang><v8:content>English</v8:content></v8:item></Synonym>";
        let source = if source.contains("<Synonym/>") {
            source.replacen("<Synonym/>", bilingual, 1)
        } else {
            let start = source.find("<Synonym>").unwrap();
            let end = source[start..].find("</Synonym>").unwrap() + start + "</Synonym>".len();
            format!("{}{bilingual}{}", &source[..start], &source[end..])
        };
        std::fs::write(&path, format!("\u{feff}{source}")).unwrap();
        for text in ["Назначение & < > \"\nстрока\r\nконец\t.", "Замена", ""]
        {
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Comment":text}}}]}),
            );
            let after = unica_workspace::test_support::tree_snapshot(&fixture.source);
            assert_eq!(
                outside_comment(&before[&relative]),
                outside_comment(&after[&relative]),
                "{at}: descriptor bytes outside Comment changed"
            );
            let mut other_before = before.clone();
            other_before.remove(&relative);
            let mut other_after = after.clone();
            other_after.remove(&relative);
            assert_eq!(
                other_before, other_after,
                "{at}: companion or neighbor changed"
            );
            let comment = call(
                &runtime,
                &fixture,
                ToolIdentity::View,
                json!({"at":if kind == "CommonForm" { at.clone() } else { format!("{at}.Props.Comment") }}),
            );
            assert!(comment.ok, "{at}: {comment:?}");
            assert_eq!(
                comment.data.as_ref().unwrap()["props"]["Comment"],
                text,
                "{at}"
            );
            let repeated = publish(
                &runtime,
                &fixture,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"comment":text}}}]}),
            );
            assert_eq!(repeated.data.as_ref().unwrap()["effects"], 0, "{at}");
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                after
            );
        }
    }
}

#[test]
fn medium_root_comment_refusals_and_late_batch_failure_do_not_publish_any_file() {
    let fixture = Fixture::new("workspace-root-comments");
    let runtime = runtime();
    for (kind, name, directory) in owners() {
        let owner = format!("main:{kind}.{name}");
        let at = if kind == "CommonForm" {
            format!("{owner}.Metadata")
        } else {
            owner
        };
        let path = fixture.source.join(directory).join(format!("{name}.xml"));
        let original = std::fs::read(&path).unwrap();
        let source = String::from_utf8(original.clone()).unwrap();
        for damaged in [
            source.replace("<Comment/>", ""),
            source.replace("<Comment/>", "<Comment/><Comment/>"),
            source.replace("<Comment/>", "<Comment><Child/></Comment>"),
            source.replace("<Comment/>", "<Comment xmlns=\"foreign\"/>"),
            source.replace("<Comment/>", "<Comment key=\"x\"/>"),
            source.replace("version=\"2.20\"", "version=\"2.21\""),
            source.replace(&format!("<Name>{name}</Name>"), "<Name>Other</Name>"),
        ] {
            std::fs::write(&path, damaged).unwrap();
            let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
            let result = call(
                &runtime,
                &fixture,
                ToolIdentity::Apply,
                json!({"at":at,"ops":[{"op":"props.set","args":{"values":{"Comment":"rejected"}}}]}),
            );
            assert_source_refusal(&result, &at);
            assert_eq!(
                unica_workspace::test_support::tree_snapshot(&fixture.source),
                before
            );
        }
        std::fs::write(&path, original).unwrap();
        let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
        let result = call(
            &runtime,
            &fixture,
            ToolIdentity::Apply,
            json!({"at":at,"ops":[
                {"op":"props.set","args":{"values":{"Comment":"must roll back"}}},
                {"op":"props.set","args":{"values":{"Comment":"invalid\u{0}text"}}}
            ]}),
        );
        assert_source_refusal(&result, &at);
        assert_eq!(
            unica_workspace::test_support::tree_snapshot(&fixture.source),
            before
        );
    }
}

#[test]
fn root_comment_refuses_legacy_inline_children_instead_of_repairing_their_bytes() {
    let fixture = Fixture::new("workspace-root-comments");
    let descriptor = fixture.source.join("CommonAttributes/Probe.xml");
    let source = std::fs::read_to_string(&descriptor).unwrap();
    std::fs::write(&descriptor, source.replace("</CommonAttribute>",
        "<ChildObjects><Form><Properties><Name>Card</Name><Comment>child</Comment></Properties></Form></ChildObjects></CommonAttribute>")).unwrap();
    let forms = fixture.source.join("CommonAttributes/Probe/Forms");
    std::fs::create_dir_all(&forms).unwrap();
    std::fs::write(
        forms.join("Card.xml"),
        "<MetaDataObject><Form/></MetaDataObject>",
    )
    .unwrap();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.source);
    let result = call(
        &runtime(),
        &fixture,
        ToolIdentity::Apply,
        json!({"at":"main:CommonAttribute.Probe","ops":[{"op":"props.set","args":{"values":{"Comment":"root"}}}]}),
    );
    assert_source_refusal(&result, "main:CommonAttribute.Probe");
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.source),
        before
    );
}
