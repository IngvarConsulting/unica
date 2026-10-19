use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

type SourceImage = BTreeMap<PathBuf, Vec<u8>>;

fn assert_exact_effects(
    source: &Path,
    before: &SourceImage,
    created: &[&str],
    replaced: &[&str],
    removed: &[&str],
) -> SourceImage {
    let after = crate::test_support::tree_snapshot(source);
    let paths = |values: &[&str]| values.iter().map(PathBuf::from).collect::<BTreeSet<_>>();
    let actual_created = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    let actual_replaced = after
        .iter()
        .filter(|(path, bytes)| before.get(*path).is_some_and(|old| old != *bytes))
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    let actual_removed = before
        .keys()
        .filter(|path| !after.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_created, paths(created), "created source inventory");
    assert_eq!(actual_replaced, paths(replaced), "changed source bytes");
    assert_eq!(actual_removed, paths(removed), "removed source inventory");
    after
}

fn owner_forms_and_slots(source: &Path) -> (Vec<String>, BTreeMap<String, String>) {
    let xml = std::fs::read_to_string(source.join("Documents/Order.xml")).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let forms = doc
        .descendants()
        .filter(|node| {
            node.has_tag_name("Form")
                && node
                    .parent_element()
                    .is_some_and(|p| p.has_tag_name("ChildObjects"))
        })
        .map(|node| node.text().unwrap().to_owned())
        .collect();
    let slots = doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name().starts_with("Default"))
        .map(|node| {
            (
                node.tag_name().name().to_owned(),
                node.text().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    (forms, slots)
}

#[test]
fn canonical_form_lifecycle_preserves_companions_and_removes_the_entire_resource() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("src");
    for directory in ["Documents/Order/Ext", "Languages", "Adjacent"] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
    }
    std::fs::write(
        workspace.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    std::fs::write(
        source.join("Configuration.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Store</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language><Document>Order</Document></ChildObjects></Configuration></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Languages/Русский.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Documents/Order.xml"),
        r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" xmlns:app="http://v8.1c.ru/8.2/managed-application/core" xmlns:cfg="http://v8.1c.ru/8.1/data/enterprise/current-config" xmlns:v8="http://v8.1c.ru/8.1/data/core" xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" xmlns:xs="http://www.w3.org/2001/XMLSchema" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" version="2.20"><Document uuid="11111111-1111-4111-8111-111111111111"><Properties><Name>Order</Name><Synonym/><Comment/><DefaultObjectForm/><DefaultListForm/><DefaultChoiceForm/></Properties><ChildObjects/></Document></MetaDataObject>"#,
    )
    .unwrap();
    std::fs::write(
        source.join("Documents/Order/Ext/ObjectModule.bsl"),
        b"// adjacent owner module\r\n",
    )
    .unwrap();
    std::fs::write(source.join("Adjacent/opaque.bin"), b"\0\xffneighbor\r\n").unwrap();

    let runtime = V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    );
    let workspace_hint = std::fs::canonicalize(workspace.path()).unwrap();
    let call = |tool, arguments| {
        let request =
            InvocationRequest::new(tool, arguments, workspace_hint.to_string_lossy(), 7_000)
                .unwrap();
        direct_v5(&runtime, request).unwrap()
    };
    let preview = |at: &str, ops: serde_json::Value| {
        let before = crate::test_support::tree_snapshot(&source);
        let result = call(ToolIdentity::Apply, serde_json::json!({"at":at,"ops":ops}));
        assert!(result.ok, "preview failed: {result:?}");
        assert_eq!(result.data.as_ref().unwrap()["mode"], "preview");
        assert!(result.data.as_ref().unwrap()["executionToken"].is_string());
        assert_eq!(crate::test_support::tree_snapshot(&source), before);
        result
    };
    let apply_pair = |at: &str, ops: serde_json::Value| {
        let planned = preview(at, ops);
        let published = call(
            ToolIdentity::Apply,
            serde_json::json!({"executionToken":planned.data.as_ref().unwrap()["executionToken"]}),
        );
        assert!(published.ok, "publication failed: {published:?}");
        assert_eq!(
            planned.data.as_ref().unwrap()["planHash"],
            published.data.as_ref().unwrap()["planHash"]
        );
    };
    let form_at = "main:Document.Order.Form.Main";
    let descriptor = "Documents/Order/Forms/Main.xml";
    let content = "Documents/Order/Forms/Main/Ext/Form.xml";
    let module = "Documents/Order/Forms/Main/Ext/Form/Module.bsl";
    let sidecar = "Documents/Order/Forms/Main/Ext/Form/Items/nested/opaque.bin";
    let before_add = crate::test_support::tree_snapshot(&source);
    apply_pair(
        "main:Document.Order",
        serde_json::json!([{"op":"form.add","args":{"items":[{"name":"Main","type":"ObjectForm"}]}}]),
    );
    assert_exact_effects(
        &source,
        &before_add,
        &[descriptor, content, module],
        &["Documents/Order.xml"],
        &[],
    );
    let (forms, slots) = owner_forms_and_slots(&source);
    assert_eq!(forms, ["Main"]);
    assert_eq!(slots["DefaultObjectForm"], "Document.Order.Form.Main");
    assert_eq!(slots["DefaultChoiceForm"], "");
    let xml = std::fs::read_to_string(source.join(descriptor)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    assert!(doc
        .descendants()
        .any(|n| n.has_tag_name("Name") && n.text() == Some("Main")));
    assert!(doc
        .descendants()
        .any(|n| n.has_tag_name("FormType") && n.text() == Some("Managed")));
    let xml = std::fs::read_to_string(source.join(content)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    assert_eq!(doc.root_element().tag_name().name(), "Form");
    assert!(doc
        .descendants()
        .any(|n| n.has_tag_name("MainAttribute") && n.text() == Some("true")));
    assert!(!std::fs::read(source.join(module)).unwrap().is_empty());
    for (at, kind) in [
        (form_at.to_owned(), "Form"),
        (format!("{form_at}.Metadata"), "Metadata"),
        (format!("{form_at}.Module.Form"), "Module"),
    ] {
        let viewed = call(ToolIdentity::View, serde_json::json!({"at":at}));
        assert!(viewed.ok, "new form resource is unreadable: {viewed:?}");
        assert_eq!(viewed.data.as_ref().unwrap()["kind"], kind);
    }

    // Fixture preparation makes accidental companion rewrites observable.
    let retained_bsl = b"&AtClient\r\nProcedure Keep(Command)\r\nEndProcedure\r\n";
    std::fs::write(source.join(module), retained_bsl).unwrap();
    std::fs::create_dir_all(source.join(sidecar).parent().unwrap()).unwrap();
    std::fs::write(source.join(sidecar), b"\0\xffform sidecar\r\n").unwrap();
    let prepared = crate::test_support::tree_snapshot(&source);
    apply_pair(
        "main:Document.Order",
        serde_json::json!([{"op":"form.set","args":{"values":{"defaultChoiceForm":"Main"}}}]),
    );
    let mut image = assert_exact_effects(&source, &prepared, &[], &["Documents/Order.xml"], &[]);
    let (forms, slots) = owner_forms_and_slots(&source);
    assert_eq!(forms, ["Main"]);
    assert_eq!(slots["DefaultObjectForm"], "Document.Order.Form.Main");
    assert_eq!(slots["DefaultChoiceForm"], "Document.Order.Form.Main");

    apply_pair(
        form_at,
        serde_json::json!([{"op":"item.add","args":{"items":[{"name":"Container","type":"Group"},{"name":"Marker","type":"LabelField"}]}}]),
    );
    image = assert_exact_effects(&source, &image, &[], &[content], &[]);
    let xml = std::fs::read_to_string(source.join(content)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    for (name, tag) in [("Container", "UsualGroup"), ("Marker", "LabelField")] {
        assert!(doc
            .descendants()
            .any(|n| n.has_tag_name(tag) && n.attribute("name") == Some(name)));
    }
    apply_pair(
        &format!("{form_at}.Item.Marker"),
        serde_json::json!([{"op":"item.move","args":{"values":{"into":"Container"}}}]),
    );
    image = assert_exact_effects(&source, &image, &[], &[content], &[]);
    let moved_at = format!("{form_at}.Item.Container.Item.Marker");
    let viewed = call(ToolIdentity::View, serde_json::json!({"at":moved_at}));
    assert!(viewed.ok, "moved item is unreadable: {viewed:?}");
    let xml = std::fs::read_to_string(source.join(content)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    let marker = doc
        .descendants()
        .find(|n| n.attribute("name") == Some("Marker"))
        .unwrap();
    assert_eq!(
        marker
            .parent_element()
            .unwrap()
            .parent_element()
            .unwrap()
            .attribute("name"),
        Some("Container")
    );
    apply_pair(
        &format!("{form_at}.Event.OnOpen"),
        serde_json::json!([{"op":"event.bind","args":{"values":{"handler":"Keep"}}}]),
    );
    image = assert_exact_effects(&source, &image, &[], &[content], &[]);
    let xml = std::fs::read_to_string(source.join(content)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    assert!(doc.descendants().any(|n| n.has_tag_name("Event")
        && n.attribute("name") == Some("OnOpen")
        && n.text() == Some("Keep")));
    assert_eq!(std::fs::read(source.join(module)).unwrap(), retained_bsl);
    apply_pair(&moved_at, serde_json::json!([{"op":"item.remove"}]));
    image = assert_exact_effects(&source, &image, &[], &[content], &[]);
    let xml = std::fs::read_to_string(source.join(content)).unwrap();
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).unwrap();
    assert!(!doc
        .descendants()
        .any(|n| n.attribute("name") == Some("Marker")));
    assert!(doc
        .descendants()
        .any(|n| n.attribute("name") == Some("Container")));

    let refused = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":form_at,"ops":[
            {"op":"item.add","args":{"items":[{"name":"Unpublished","type":"Group"}]}},
            {"op":"form.remove"}
        ]}),
    );
    assert!(!refused.ok, "edit plus removal was accepted: {refused:?}");
    assert_eq!(refused.at.as_deref(), Some("ops[1].args.at"));
    assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
    assert_eq!(refused.diagnostics[0]["message"], "`form.remove` inspects the source root and must run in its own apply call before other changes");
    assert!(refused.changed.is_empty());
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|d| d.get("executionToken").is_none()));
    assert_eq!(crate::test_support::tree_snapshot(&source), image);

    let pending = preview(
        "main:Document.Order",
        serde_json::json!([{"op":"form.set","args":{"values":{"defaultListForm":"Main"}}}]),
    );
    // This external edit is fixture input, before the refused execution begins.
    let owner_path = source.join("Documents/Order.xml");
    let xml = std::fs::read_to_string(&owner_path).unwrap();
    std::fs::write(
        &owner_path,
        xml.replace(
            "</MetaDataObject>",
            "<!-- external fixture edit --></MetaDataObject>",
        ),
    )
    .unwrap();
    let externally_changed = crate::test_support::tree_snapshot(&source);
    let stale = call(
        ToolIdentity::Apply,
        serde_json::json!({"executionToken":pending.data.as_ref().unwrap()["executionToken"]}),
    );
    assert!(!stale.ok, "changed owner accepted a stale token: {stale:?}");
    assert_eq!(stale.diagnostics[0]["code"], "stale_revision");
    assert!(stale.changed.is_empty());
    assert_eq!(
        crate::test_support::tree_snapshot(&source),
        externally_changed
    );

    // A binary outside the admitted resource layout is not ours to remove.
    let foreign = "Documents/Order/Forms/Main/Extras/nested/opaque.bin";
    std::fs::create_dir_all(source.join(foreign).parent().unwrap()).unwrap();
    std::fs::write(source.join(foreign), b"foreign binary").unwrap();
    let with_foreign = crate::test_support::tree_snapshot(&source);
    let refused = call(
        ToolIdentity::Apply,
        serde_json::json!({"at":form_at,"ops":[{"op":"form.remove"}]}),
    );
    assert!(!refused.ok, "foreign binary was accepted: {refused:?}");
    assert_eq!(refused.diagnostics[0]["code"], "invalid_state");
    assert!(refused.summary.contains("supported source profile"));
    assert!(refused
        .summary
        .contains(&Path::new(foreign).display().to_string()));
    assert!(refused.changed.is_empty());
    assert!(refused
        .data
        .as_ref()
        .is_none_or(|d| d.get("executionToken").is_none()));
    assert_eq!(crate::test_support::tree_snapshot(&source), with_foreign);
    // Fixture cleanup restores the admitted inputs; the product did not write.
    std::fs::remove_file(source.join(foreign)).unwrap();
    assert_eq!(
        crate::test_support::tree_snapshot(&source),
        externally_changed
    );

    apply_pair(form_at, serde_json::json!([{"op":"form.remove"}]));
    assert_exact_effects(
        &source,
        &externally_changed,
        &[],
        &["Documents/Order.xml"],
        &[descriptor, content, module, sidecar],
    );
    let (forms, slots) = owner_forms_and_slots(&source);
    assert!(forms.is_empty());
    assert_eq!(slots["DefaultObjectForm"], "");
    assert_eq!(slots["DefaultChoiceForm"], "");
    assert_eq!(slots["DefaultListForm"], "");
    let owner = call(
        ToolIdentity::View,
        serde_json::json!({"at":"main:Document.Order"}),
    );
    assert!(owner.ok, "owner was lost with its form: {owner:?}");
    let removed = call(ToolIdentity::View, serde_json::json!({"at":form_at}));
    assert!(!removed.ok, "removed form remains readable: {removed:?}");
}
