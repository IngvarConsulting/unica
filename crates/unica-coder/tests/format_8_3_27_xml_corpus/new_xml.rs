use super::*;

#[test]
fn typed_catalog_corpus_publishes_native_descriptor_bytes() {
    let root = tempfile::tempdir().unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "meta-compile-catalog")
        .unwrap();
    let mut gate = SequentialCallGate::default();
    let generated = run_corpus_case(root.path(), case, &mut gate).unwrap();
    let manifest = serde_json::to_value(generated).unwrap();
    let suffix = "src/Catalogs/CorpusCatalog.xml";
    let descriptor = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.ends_with(suffix))
        })
        .expect("retained typed publication must inventory its created Catalog descriptor");
    assert_eq!(descriptor["seed"], false);
    assert_eq!(descriptor["delta"], "created");
    let workspace = root.path().join("cases/meta-compile-catalog/workspace");
    assert_eq!(
        fs::read(workspace.join("src/Catalogs/CorpusCatalog/Ext/ObjectModule.bsl")).unwrap(),
        b"\xef\xbb\xbf"
    );
    let published = fs::read(workspace.join(suffix)).unwrap();
    let native = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/new-xml/native-catalog-descriptor.xml"
    ));
    assert_eq!(published, bind_native_identities(native, &published, "Catalog"), "retained typed Catalog publication must match raw native export apart from its 11 declared generated identity slots");
}

#[test]
fn typed_chart_of_accounts_corpus_publishes_native_descriptor_bytes() {
    let root = tempfile::tempdir().unwrap();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "meta-compile-chart-of-accounts")
        .unwrap();
    let mut gate = SequentialCallGate::default();
    let generated = run_corpus_case(root.path(), case, &mut gate).unwrap();
    let manifest = serde_json::to_value(generated).unwrap();
    let suffix = "src/ChartsOfAccounts/CorpusAccounts.xml";
    let descriptor = manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.ends_with(suffix))
        })
        .expect("retained typed publication must inventory its created ChartOfAccounts descriptor");
    assert_eq!(descriptor["seed"], false);
    assert_eq!(descriptor["delta"], "created");
    let workspace = root
        .path()
        .join("cases/meta-compile-chart-of-accounts/workspace");
    assert_eq!(
        fs::read(workspace.join("src/ChartsOfAccounts/CorpusAccounts/Ext/ObjectModule.bsl"))
            .unwrap(),
        b"\xef\xbb\xbf"
    );
    let published = fs::read(workspace.join(suffix)).unwrap();
    let native = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/new-xml/native-chart-of-accounts-descriptor.xml"
    ));
    assert_eq!(published, bind_native_identities(native, &published, "ChartOfAccounts"), "retained typed ChartOfAccounts publication must match raw native export apart from its 15 declared generated identity slots");
}

#[test]
fn typed_catalog_creation_with_comment_preserves_native_descriptor_profile() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "meta-compile-catalog")
        .unwrap();
    let mut args = prepare_target(case, workspace).unwrap();
    args.insert(
        "operations".to_owned(),
        json!([
            {"op":"setProperties","values":{"Comment":"retained comment"}}
        ]),
    );
    let source = workspace.join("src");
    let before_xml = capture_xml_payloads(&source).unwrap();
    let before_non_xml = capture_non_xml_payloads(case, workspace).unwrap();
    call_public_tool(case.tool, &args).unwrap();
    let after_xml = capture_xml_payloads(&source).unwrap();
    let delta = enforce_xml_impact(
        XmlImpactClass::CreateOrModify,
        &hashes_for_xml_payloads(&before_xml),
        &hashes_for_xml_payloads(&after_xml),
    )
    .unwrap();
    let descriptor = "Catalogs/CorpusCatalog.xml";
    assert_eq!(delta.created, [descriptor]);
    assert_eq!(delta.modified, ["Configuration.xml"]);
    assert!(delta.removed.is_empty());
    for (path, bytes) in &before_xml {
        if path != "Configuration.xml" {
            assert_eq!(
                after_xml.get(path),
                Some(bytes),
                "changed adjacent XML: {path}"
            );
        }
    }
    let mut expected_non_xml = before_non_xml;
    assert!(expected_non_xml
        .insert(
            "src/Catalogs/CorpusCatalog/Ext/ObjectModule.bsl".to_owned(),
            b"\xef\xbb\xbf".to_vec(),
        )
        .is_none());
    assert_eq!(
        capture_non_xml_payloads(case, workspace).unwrap(),
        expected_non_xml
    );
    let published = after_xml.get(descriptor).unwrap();
    let native = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/new-xml/native-catalog-descriptor.xml"
    ));
    let native_text = std::str::from_utf8(native).unwrap();
    assert_eq!(native_text.matches("<Comment/>").count(), 1);
    let with_comment = native_text.replacen("<Comment/>", "<Comment>retained comment</Comment>", 1);
    assert_eq!(*published, bind_native_identities(with_comment.as_bytes(), published, "Catalog"), "actual typed setProperties must inherit the new descriptor's native byte profile while publishing the requested Comment");
}

const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
const XR: &str = "http://v8.1c.ru/8.3/xcf/readable";

fn generated_identity_slots(bytes: &[u8], kind: &str) -> Vec<String> {
    let text = std::str::from_utf8(bytes).unwrap();
    let document = roxmltree::Document::parse(text.strip_prefix('\u{feff}').unwrap()).unwrap();
    let owners = document
        .root_element()
        .children()
        .filter(|node| node.has_tag_name((MD, kind)))
        .collect::<Vec<_>>();
    assert_eq!(owners.len(), 1, "exactly one {kind} owner");
    let owner = owners[0];
    let mut slots = vec![owner.attribute("uuid").unwrap().to_owned()];
    let (name, roles): (&str, &[&str]) = match kind {
        "Catalog" => (
            "CorpusCatalog",
            &["Object", "Ref", "Selection", "List", "Manager"],
        ),
        "ChartOfAccounts" => (
            "CorpusAccounts",
            &[
                "Object",
                "Ref",
                "Selection",
                "List",
                "Manager",
                "ExtDimensionTypes",
                "ExtDimensionTypesRow",
            ],
        ),
        "Form" => ("CorpusForm", &[]),
        _ => panic!("no declared identity binding for {kind}"),
    };
    if !roles.is_empty() {
        let internal = owner
            .children()
            .filter(|node| node.has_tag_name((MD, "InternalInfo")))
            .collect::<Vec<_>>();
        assert_eq!(internal.len(), 1);
        let generated = internal[0]
            .children()
            .filter(|node| node.is_element())
            .collect::<Vec<_>>();
        assert_eq!(generated.len(), roles.len());
        for (node, &role) in generated.iter().zip(roles) {
            assert!(node.has_tag_name((XR, "GeneratedType")));
            assert_eq!(node.attribute("category"), Some(role));
            assert_eq!(
                node.attribute("name"),
                Some(format!("{kind}{role}.{name}").as_str())
            );
            let fields = node
                .children()
                .filter(|child| child.is_element())
                .collect::<Vec<_>>();
            assert_eq!(fields.len(), 2);
            for (field, name) in fields.iter().zip(["TypeId", "ValueId"]) {
                assert!(field.has_tag_name((XR, name)));
                slots.push(field.text().unwrap().to_owned());
            }
        }
    }
    assert_eq!(slots.len(), 1 + 2 * roles.len());
    let mut unique = std::collections::BTreeSet::new();
    for slot in &slots {
        let identity = uuid::Uuid::parse_str(slot).expect("generated identity must be a UUID");
        assert!(!identity.is_nil());
        assert!(
            unique.insert(identity),
            "generated identities must be distinct"
        );
        assert_eq!(
            text.matches(slot).count(),
            1,
            "identity must occupy only its declared slot"
        );
    }
    slots
}

fn bind_native_identities(native: &[u8], published: &[u8], kind: &str) -> Vec<u8> {
    let expected = generated_identity_slots(native, kind);
    let actual = generated_identity_slots(published, kind);
    let mut result = std::str::from_utf8(native).unwrap().to_owned();
    let mut replacements = expected
        .iter()
        .zip(&actual)
        .map(|(old, new)| (result.find(old).unwrap(), old.len(), new.as_str()))
        .collect::<Vec<_>>();
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, length, new) in replacements {
        result.replace_range(start..start + length, new);
    }
    result.into_bytes()
}
