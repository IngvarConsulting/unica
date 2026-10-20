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

fn user_value_workspace_image(root: &Path) -> (BTreeMap<PathBuf, Vec<u8>>, BTreeSet<PathBuf>) {
    fn visit(
        root: &Path,
        directory: &Path,
        files: &mut BTreeMap<PathBuf, Vec<u8>>,
        directories: &mut BTreeSet<PathBuf>,
    ) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().unwrap().is_dir() {
                directories.insert(relative);
                visit(root, &path, files, directories);
            } else {
                files.insert(relative, fs::read(path).unwrap());
            }
        }
    }
    let mut files = BTreeMap::new();
    let mut directories = BTreeSet::new();
    visit(root, root, &mut files, &mut directories);
    (files, directories)
}

fn publish_typed_user_value(
    workspace: &Path,
    tool: &str,
    args: &Map<String, Value>,
    effect_operations: &[&str],
) {
    let before = user_value_workspace_image(workspace);
    let mut preview_args = args.clone();
    preview_args.insert("dryRun".to_owned(), json!(true));
    let preview = call_public_tool_result(tool, &preview_args).unwrap();
    let plan = preview.data.as_ref().unwrap();
    assert_eq!(plan["changed"], true);
    assert_eq!(plan["validation"]["status"], "passed");
    let effects = plan["effects"].as_array().unwrap();
    assert!(!effects.is_empty());
    assert_eq!(
        effects
            .iter()
            .map(|effect| effect["operation"].as_str().unwrap())
            .collect::<Vec<_>>(),
        effect_operations
    );
    assert!(!plan["changedPaths"].as_array().unwrap().is_empty());
    assert_eq!(
        user_value_workspace_image(workspace),
        before,
        "retained preview changed source, project or cache bytes/directories"
    );
    let mut apply_args = args.clone();
    apply_args.insert("dryRun".to_owned(), json!(false));
    let published = call_public_tool_result(tool, &apply_args).unwrap();
    let data = published.data.as_ref().unwrap();
    assert_eq!(data["changed"], true);
    assert_eq!(data["validation"]["status"], "passed");
    for field in ["metadataPath", "effects", "changedPaths", "publicationPlan"] {
        assert_eq!(
            data[field], plan[field],
            "retained receipt differs: {field}"
        );
    }
    assert!(!preview.changes.is_empty());
    assert!(!published.changes.is_empty());
}

#[test]
fn typed_catalog_comment_preserves_its_value_across_a_later_property_edit() {
    const COMMENT: &str = "first\nsecond\t<&>";
    const DESCRIPTOR: &str = "Catalogs/CorpusCatalog.xml";
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path();
    let case = EXECUTABLE_CASES
        .iter()
        .find(|case| case.id == "meta-compile-catalog")
        .unwrap();
    let mut args = prepare_target(case, workspace).unwrap();
    args.insert(
        "operations".to_owned(),
        json!([{"op":"setProperties","values":{"Comment":COMMENT}}]),
    );
    let source = workspace.join("src");
    let cache = workspace.join(".build/unica");
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("user-value-sentinel"), b"cache\r\n").unwrap();
    let before = user_value_workspace_image(&source).0;
    publish_typed_user_value(
        workspace,
        case.tool,
        &args,
        &["createTemplate", "setProperties"],
    );
    let after = user_value_workspace_image(&source).0;
    let added = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        added,
        BTreeSet::from([
            PathBuf::from(DESCRIPTOR),
            PathBuf::from("Catalogs/CorpusCatalog/Ext/ObjectModule.bsl"),
        ])
    );
    for (path, bytes) in &before {
        if path == Path::new("Configuration.xml") {
            assert_ne!(after.get(path), Some(bytes));
        } else {
            assert_eq!(after.get(path), Some(bytes), "changed adjacent {path:?}");
        }
    }
    assert_eq!(
        after[Path::new("Catalogs/CorpusCatalog/Ext/ObjectModule.bsl")],
        b"\xef\xbb\xbf"
    );
    let published = &after[Path::new(DESCRIPTOR)];
    let published_text = std::str::from_utf8(published).unwrap();
    let xml = published_text
        .strip_prefix('\u{feff}')
        .unwrap_or(published_text);
    let document = roxmltree::Document::parse(xml).unwrap();
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((MD, "Catalog")))
        .unwrap();
    let properties = owner
        .children()
        .find(|node| node.has_tag_name((MD, "Properties")))
        .unwrap();
    let comments = properties
        .children()
        .filter(|node| node.has_tag_name((MD, "Comment")))
        .collect::<Vec<_>>();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text(), Some(COMMENT));
    let code_lengths = properties
        .children()
        .filter(|node| node.has_tag_name((MD, "CodeLength")))
        .collect::<Vec<_>>();
    assert_eq!(code_lengths.len(), 1);
    assert_eq!(code_lengths[0].text(), Some("9"));
    let offset = published_text.len() - xml.len();
    let range = code_lengths[0].range();
    let range = range.start + offset..range.end + offset;
    assert_eq!(&published[range.clone()], b"<CodeLength>9</CodeLength>");
    let mut expected_second = published.clone();
    expected_second.splice(range, b"<CodeLength>12</CodeLength>".iter().copied());

    // Re-enter through the real retained editor without reseeding or
    // normalizing the just-published descriptor.
    let edit = Map::from_iter([
        ("cwd".to_owned(), json!(workspace.display().to_string())),
        ("sourceSet".to_owned(), json!("main")),
        ("metadataPath".to_owned(), json!("Catalog.CorpusCatalog")),
        (
            "operations".to_owned(),
            json!([{"op":"setProperties","values":{"CodeLength":12}}]),
        ),
    ]);
    publish_typed_user_value(workspace, "unica.meta.edit", &edit, &["setProperties"]);
    let mut expected_tree = after;
    expected_tree.insert(PathBuf::from(DESCRIPTOR), expected_second);
    assert_eq!(user_value_workspace_image(&source).0, expected_tree);
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
