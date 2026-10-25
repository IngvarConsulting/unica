use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::test_support::tree_snapshot;

const CATALOG: &str = "Catalogs/CorpusCatalog.xml";
const FORM_DESCRIPTOR: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm.xml";
const FORM_CONTENT: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form.xml";
const FORM_MODULE: &str = "Catalogs/CorpusCatalog/Forms/CorpusForm/Ext/Form/Module.bsl";
const NATIVE_CATALOG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/platform_8_3_27/new-xml/native-catalog-descriptor.xml"
));
const NATIVE_FORM_DESCRIPTOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/platform_8_3_27/new-xml/native-form-descriptor.xml"
));
const NATIVE_FORM_CONTENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/platform_8_3_27/new-xml/native-form-content.xml"
));

const NATIVE_FORM_MODULE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/platform_8_3_27/new-xml/native-form-module.bsl"
));

fn workspace(with_catalog: bool) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("src");
    for directory in ["Catalogs", "Languages", "Adjacent"] {
        std::fs::create_dir_all(source.join(directory)).unwrap();
    }
    std::fs::write(
        root.path().join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    let catalog_registration = if with_catalog {
        "<Catalog>CorpusCatalog</Catalog>"
    } else {
        ""
    };
    std::fs::write(source.join("Configuration.xml"), format!(r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration><Properties><Name>Main</Name><DefaultLanguage>Language.Русский</DefaultLanguage></Properties><ChildObjects><Language>Русский</Language>{catalog_registration}</ChildObjects></Configuration></MetaDataObject>"#)).unwrap();
    std::fs::write(source.join("Languages/Русский.xml"), r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Language uuid="33333333-3333-4333-8333-333333333333"><Properties><Name>Русский</Name><Comment/><LanguageCode>ru</LanguageCode></Properties></Language></MetaDataObject>"#).unwrap();
    std::fs::write(source.join("Adjacent/opaque.bin"), b"\0\xffadjacent\r\n").unwrap();
    if with_catalog {
        std::fs::write(source.join(CATALOG), NATIVE_CATALOG).unwrap();
    }
    root
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn tree_directories(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(root: &Path, current: &Path, directories: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                let path = entry.path();
                directories.insert(path.strip_prefix(root).unwrap().to_path_buf());
                visit(root, &path, directories);
            }
        }
    }
    let mut directories = BTreeSet::new();
    visit(root, root, &mut directories);
    directories
}

fn apply_pair(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    at: &str,
    ops: serde_json::Value,
) {
    let root = std::fs::canonicalize(root).unwrap();
    let source = root.join("src");
    let before = (tree_snapshot(&source), tree_directories(&source));
    let cache = root.join(".build/unica");
    let cache_before = cache
        .exists()
        .then(|| (tree_snapshot(&cache), tree_directories(&cache)));
    let call = |arguments| {
        direct_v5(
            runtime,
            InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                root.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let planned = call(json!({"at":at,"ops":ops}));
    assert!(planned.ok, "preview failed: {planned:?}");
    let plan = planned.data.as_ref().unwrap();
    assert_eq!(plan["mode"], "preview");
    assert!(plan["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    assert_eq!(
        (tree_snapshot(&source), tree_directories(&source)),
        before,
        "preview changed source bytes"
    );
    assert_eq!(
        cache
            .exists()
            .then(|| (tree_snapshot(&cache), tree_directories(&cache))),
        cache_before,
        "preview changed cache existence or bytes"
    );
    let published = call(json!({"executionToken":plan["executionToken"]}));
    assert!(published.ok, "publication failed: {published:?}");
    let publication = published.data.as_ref().unwrap();
    assert_eq!(publication["mode"], "published");
    assert_eq!(publication["planHash"], plan["planHash"]);
}

fn assert_effects(
    source: &Path,
    before: &BTreeMap<PathBuf, Vec<u8>>,
    created: &[&str],
    modified: &[&str],
) {
    let after = tree_snapshot(source);
    let added = after
        .keys()
        .filter(|path| !before.contains_key(*path))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(added, created.iter().map(PathBuf::from).collect());
    assert!(
        before.keys().all(|path| after.contains_key(path)),
        "publication removed a source file"
    );
    let changed = before
        .iter()
        .filter(|(path, bytes)| after.get(*path) != Some(*bytes))
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(changed, modified.iter().map(PathBuf::from).collect());
}

fn assert_view(runtime: &V5CanonicalInvocationRuntime, root: &Path, at: &str, kind: &str) {
    let root = std::fs::canonicalize(root).unwrap();
    let request = InvocationRequest::new(
        ToolIdentity::View,
        json!({"at":at}),
        root.to_string_lossy(),
        7_000,
    )
    .unwrap();
    let viewed = direct_v5(runtime, request).unwrap();
    assert!(viewed.ok, "published resource unreadable: {viewed:?}");
    assert_eq!(viewed.data.as_ref().unwrap()["kind"], kind);
}

fn publish_form(root: &Path) {
    let source = root.join("src");
    let before = tree_snapshot(&source);
    let runtime = runtime();
    apply_pair(
        &runtime,
        root,
        "main:Catalog.CorpusCatalog",
        json!([
            {"op":"form.add","args":{"items":[{"name":"CorpusForm","type":"ObjectForm"}]}}
        ]),
    );
    assert_effects(
        &source,
        &before,
        &[FORM_DESCRIPTOR, FORM_CONTENT, FORM_MODULE],
        &[CATALOG],
    );
    assert_eq!(
        std::fs::read(source.join(FORM_MODULE)).unwrap(),
        NATIVE_FORM_MODULE
    );
    for (at, kind) in [
        ("main:Catalog.CorpusCatalog.Form.CorpusForm", "Form"),
        (
            "main:Catalog.CorpusCatalog.Form.CorpusForm.Metadata",
            "Metadata",
        ),
        (
            "main:Catalog.CorpusCatalog.Form.CorpusForm.Module.Form",
            "Module",
        ),
    ] {
        assert_view(&runtime, root, at, kind);
    }
}

#[test]
fn canonical_form_add_publishes_native_descriptor_bytes() {
    let root = workspace(true);
    publish_form(root.path());
    let published = std::fs::read(root.path().join("src").join(FORM_DESCRIPTOR)).unwrap();
    assert_eq!(
        published,
        bind_native_identities(NATIVE_FORM_DESCRIPTOR, &published, "Form"),
        "new form descriptor must match raw native export apart from its generated UUID"
    );
}

#[test]
fn canonical_form_add_publishes_native_form_content_bytes() {
    let root = workspace(true);
    let cache = root.path().join(".build/unica");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("preview-sentinel"), b"retained cache\r\n").unwrap();
    publish_form(root.path());
    let published = std::fs::read(root.path().join("src").join(FORM_CONTENT)).unwrap();
    assert_eq!(
        published, NATIVE_FORM_CONTENT,
        "new managed Form.xml must match the raw native namespace and byte profile"
    );
}

#[test]
fn canonical_object_create_publishes_native_catalog_descriptor_bytes() {
    let root = workspace(false);
    let source = root.path().join("src");
    let before = tree_snapshot(&source);
    let runtime = runtime();
    apply_pair(
        &runtime,
        root.path(),
        "main:Configuration",
        json!([
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"CorpusCatalog"}}}
        ]),
    );
    let module = "Catalogs/CorpusCatalog/Ext/ObjectModule.bsl";
    assert_effects(&source, &before, &[CATALOG, module], &["Configuration.xml"]);
    assert_eq!(std::fs::read(source.join(module)).unwrap(), b"\xef\xbb\xbf");
    assert_view(
        &runtime,
        root.path(),
        "main:Catalog.CorpusCatalog",
        "Catalog",
    );
    let published = std::fs::read(source.join(CATALOG)).unwrap();
    assert_eq!(published, bind_native_identities(NATIVE_CATALOG, &published, "Catalog"), "new Catalog must match raw native export apart from its 11 declared generated identity slots");
}

#[test]
fn canonical_catalog_created_with_a_comment_keeps_the_native_descriptor_profile() {
    let root = workspace(false);
    let source = root.path().join("src");
    let before = tree_snapshot(&source);
    let runtime = runtime();
    apply_pair(
        &runtime,
        root.path(),
        "main:Configuration",
        json!([
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"CorpusCatalog"}}},
            {"op":"props.set","args":{"at":"main:Catalog.CorpusCatalog","values":{"Comment":"retained comment"}}}
        ]),
    );
    let module = "Catalogs/CorpusCatalog/Ext/ObjectModule.bsl";
    assert_effects(&source, &before, &[CATALOG, module], &["Configuration.xml"]);
    assert_eq!(std::fs::read(source.join(module)).unwrap(), b"\xef\xbb\xbf");
    let native = std::str::from_utf8(NATIVE_CATALOG).unwrap();
    assert_eq!(native.matches("<Comment/>").count(), 1);
    let with_comment = native.replacen("<Comment/>", "<Comment>retained comment</Comment>", 1);
    let published = std::fs::read(source.join(CATALOG)).unwrap();
    assert_eq!(
        published,
        bind_native_identities(with_comment.as_bytes(), &published, "Catalog"),
        "a Comment set at creation must keep the new descriptor's native byte profile"
    );
}

fn publish_catalog_user_value(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    at: &str,
    ops: serde_json::Value,
) {
    let root = std::fs::canonicalize(root).unwrap();
    let before = (tree_snapshot(&root), tree_directories(&root));
    let call = |arguments| {
        direct_v5(
            runtime,
            InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                root.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let preview = call(json!({"at":at,"ops":ops}));
    assert!(preview.ok, "user-value preview failed: {preview:?}");
    let plan = preview.data.as_ref().unwrap();
    assert_eq!(plan["mode"], "preview");
    assert_eq!(plan["validated"], true);
    assert_eq!(plan["executable"], true);
    for field in ["planHash", "executionToken"] {
        assert!(plan[field].as_str().is_some_and(|value| !value.is_empty()));
    }
    assert_eq!(
        preview.changed,
        vec![json!({"at":"main:Catalog.CorpusCatalog","event":"MetadataChanged"})]
    );
    assert_eq!(plan["effects"], 1);
    assert_eq!(
        (tree_snapshot(&root), tree_directories(&root)),
        before,
        "preview changed source, project or cache bytes/directories"
    );
    let published = call(json!({"executionToken":plan["executionToken"]}));
    assert!(published.ok, "user-value publication failed: {published:?}");
    let publication = published.data.as_ref().unwrap();
    assert_eq!(publication["mode"], "published");
    for field in ["planHash", "operations", "effects"] {
        assert_eq!(publication[field], plan[field]);
    }
    assert_eq!(published.changed, preview.changed);
}

#[test]
fn canonical_catalog_comment_preserves_its_value_across_a_later_property_edit() {
    const COMMENT: &str = "first\nsecond\t<&>";
    let root = workspace(false);
    let source = root.path().join("src");
    let cache = root.path().join(".build/unica");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("user-value-sentinel"), b"cache\r\n").unwrap();
    let before = tree_snapshot(&source);
    let runtime = runtime();
    publish_catalog_user_value(
        &runtime,
        root.path(),
        "main:Configuration",
        json!([
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"CorpusCatalog"}}},
            {"op":"props.set","args":{"at":"main:Catalog.CorpusCatalog","values":{"Comment":COMMENT}}}
        ]),
    );
    assert_effects(
        &source,
        &before,
        &[CATALOG, "Catalogs/CorpusCatalog/Ext/ObjectModule.bsl"],
        &["Configuration.xml"],
    );
    assert_eq!(
        std::fs::read(source.join("Catalogs/CorpusCatalog/Ext/ObjectModule.bsl")).unwrap(),
        b"\xef\xbb\xbf"
    );
    let published = std::fs::read(source.join(CATALOG)).unwrap();
    let published_text = std::str::from_utf8(&published).unwrap();
    let xml = published_text
        .strip_prefix('\u{feff}')
        .unwrap_or(published_text);
    let document = roxmltree::Document::parse(xml).unwrap();
    let md = "http://v8.1c.ru/8.3/MDClasses";
    let owner = document
        .root_element()
        .children()
        .find(|node| node.has_tag_name((md, "Catalog")))
        .unwrap();
    let properties = owner
        .children()
        .find(|node| node.has_tag_name((md, "Properties")))
        .unwrap();
    let comments = properties
        .children()
        .filter(|node| node.has_tag_name((md, "Comment")))
        .collect::<Vec<_>>();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].text(), Some(COMMENT));
    let code_lengths = properties
        .children()
        .filter(|node| node.has_tag_name((md, "CodeLength")))
        .collect::<Vec<_>>();
    assert_eq!(code_lengths.len(), 1);
    assert_eq!(code_lengths[0].text(), Some("9"));
    let offset = published_text.len() - xml.len();
    let range = code_lengths[0].range();
    let range = range.start + offset..range.end + offset;
    assert_eq!(&published[range.clone()], b"<CodeLength>9</CodeLength>");
    let mut expected_second = published.clone();
    expected_second.splice(range, b"<CodeLength>12</CodeLength>".iter().copied());

    // The serialized LF spelling is not prescribed. A subsequent request
    // must preserve those actually published bytes outside CodeLength.
    let before_second = tree_snapshot(&source);
    publish_catalog_user_value(
        &runtime,
        root.path(),
        "main:Catalog.CorpusCatalog",
        json!([{"op":"props.set","args":{"values":{"CodeLength":12}}}]),
    );
    let mut expected_tree = before_second;
    expected_tree.insert(PathBuf::from(CATALOG), expected_second);
    assert_eq!(tree_snapshot(&source), expected_tree);
}

#[test]
fn canonical_object_create_publishes_native_chart_of_accounts_descriptor_bytes() {
    let root = workspace(false);
    let source = root.path().join("src");
    let before = tree_snapshot(&source);
    let runtime = runtime();
    apply_pair(
        &runtime,
        root.path(),
        "main:Configuration",
        json!([
            {"op":"object.create","args":{"values":{"kind":"ChartOfAccounts","name":"CorpusAccounts"}}}
        ]),
    );
    let descriptor = "ChartsOfAccounts/CorpusAccounts.xml";
    let module = "ChartsOfAccounts/CorpusAccounts/Ext/ObjectModule.bsl";
    assert_effects(
        &source,
        &before,
        &[descriptor, module],
        &["Configuration.xml"],
    );
    assert_eq!(std::fs::read(source.join(module)).unwrap(), b"\xef\xbb\xbf");
    assert_view(
        &runtime,
        root.path(),
        "main:ChartOfAccounts.CorpusAccounts",
        "ChartOfAccounts",
    );
    let published = std::fs::read(source.join(descriptor)).unwrap();
    let native = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/platform_8_3_27/new-xml/native-chart-of-accounts-descriptor.xml"
    ));
    assert_eq!(published, bind_native_identities(native, &published, "ChartOfAccounts"), "new ChartOfAccounts must match raw native export apart from its 15 declared generated identity slots");
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

#[test]
fn canonical_object_create_then_form_add_preserves_native_new_file_bytes() {
    let root = workspace(false);
    let root_path = std::fs::canonicalize(root.path()).unwrap();
    let source = root_path.join("src");
    let before = tree_snapshot(&source);
    let directories_before = tree_directories(&source);
    let cache = root_path.join(".build/unica");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::write(cache.join("preview-sentinel"), b"retained cache\r\n").unwrap();
    let cache_before = (tree_snapshot(&cache), tree_directories(&cache));
    let runtime = runtime();
    let call = |arguments| {
        direct_v5(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Apply,
                arguments,
                root_path.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap()
    };
    let planned = call(json!({
        "at": "main:Configuration",
        "ops": [
            {"op":"object.create","args":{"values":{"kind":"Catalog","name":"CorpusCatalog"}}},
            {"op":"form.add","args":{"at":"main:Catalog.CorpusCatalog","items":[{"name":"CorpusForm","type":"ObjectForm"}]}}
        ]
    }));
    assert!(planned.ok, "joint preview failed: {planned:?}");
    let plan = planned.data.as_ref().unwrap();
    assert!(plan["planHash"]
        .as_str()
        .is_some_and(|hash| !hash.is_empty()));
    assert_eq!(plan["mode"], "preview");
    assert_eq!(plan["validated"], true);
    assert_eq!(plan["executable"], true);
    assert_eq!(
        plan["operations"],
        json!([
            {"index":0,"op":"object.create","at":"main:Configuration"},
            {"index":1,"op":"form.add","at":"main:Catalog.CorpusCatalog"}
        ])
    );
    assert_eq!(plan["effects"], 2);
    assert_eq!(
        planned.changed,
        vec![
            json!({"at":"main:Catalog.CorpusCatalog","event":"MetadataChanged"}),
            json!({"at":"main:Catalog.CorpusCatalog","event":"FormChanged"})
        ]
    );
    assert!(plan["executionToken"]
        .as_str()
        .is_some_and(|token| !token.is_empty()));
    assert_eq!(tree_snapshot(&source), before, "joint preview wrote source");
    assert_eq!(tree_directories(&source), directories_before);
    assert_eq!(
        (tree_snapshot(&cache), tree_directories(&cache)),
        cache_before,
        "joint preview wrote cache"
    );

    // This is the only execution request: no intermediate object publication
    // and no second preview may provide the owner to form.add.
    let published = call(json!({"executionToken":plan["executionToken"]}));
    assert!(published.ok, "joint publication failed: {published:?}");
    let publication = published.data.as_ref().unwrap();
    assert_eq!(publication["mode"], "published");
    assert_eq!(publication["planHash"], plan["planHash"]);
    assert_eq!(publication["operations"], plan["operations"]);
    assert_eq!(publication["effects"], plan["effects"]);
    assert_eq!(published.changed, planned.changed);

    let object_module = "Catalogs/CorpusCatalog/Ext/ObjectModule.bsl";
    let created = [
        CATALOG,
        object_module,
        FORM_DESCRIPTOR,
        FORM_CONTENT,
        FORM_MODULE,
    ];
    assert_effects(&source, &before, &created, &["Configuration.xml"]);
    let mut expected_directories = directories_before;
    for relative in created {
        let mut parent = Path::new(relative).parent();
        while let Some(directory) = parent {
            if !directory.as_os_str().is_empty() {
                expected_directories.insert(directory.to_path_buf());
            }
            parent = directory.parent();
        }
    }
    assert_eq!(tree_directories(&source), expected_directories);

    let descriptor = std::fs::read(source.join(FORM_DESCRIPTOR)).unwrap();
    assert_eq!(
        descriptor,
        bind_native_identities(NATIVE_FORM_DESCRIPTOR, &descriptor, "Form")
    );
    assert_eq!(
        std::fs::read(source.join(FORM_CONTENT)).unwrap(),
        NATIVE_FORM_CONTENT
    );
    assert_eq!(
        std::fs::read(source.join(FORM_MODULE)).unwrap(),
        NATIVE_FORM_MODULE
    );
    assert_eq!(
        std::fs::read(source.join(object_module)).unwrap(),
        b"\xef\xbb\xbf"
    );

    let owner = std::fs::read(source.join(CATALOG)).unwrap();
    let native_with_ids = bind_native_identities(NATIVE_CATALOG, &owner, "Catalog");
    let mut expected_owner = String::from_utf8(native_with_ids).unwrap();
    // The minimal native Catalog is the independent object.create baseline.
    // The registration takes its own line, as the 8.3.27 export writes the
    // first form of an empty container (#1404).
    let eol = if expected_owner.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let registration =
        format!("<ChildObjects>{eol}\t\t\t<Form>CorpusForm</Form>{eol}\t\t</ChildObjects>");
    for (before, after) in [
        (
            "<DefaultObjectForm/>",
            "<DefaultObjectForm>Catalog.CorpusCatalog.Form.CorpusForm</DefaultObjectForm>",
        ),
        ("<ChildObjects/>", registration.as_str()),
    ] {
        assert_eq!(expected_owner.matches(before).count(), 1);
        expected_owner = expected_owner.replacen(before, after, 1);
    }
    assert_eq!(
        owner,
        expected_owner.as_bytes(),
        "form.add may alter only the declared default and registration bytes of the newly staged Catalog"
    );
    for (at, kind) in [
        ("main:Catalog.CorpusCatalog", "Catalog"),
        ("main:Catalog.CorpusCatalog.Form.CorpusForm", "Form"),
        (
            "main:Catalog.CorpusCatalog.Form.CorpusForm.Metadata",
            "Metadata",
        ),
        (
            "main:Catalog.CorpusCatalog.Form.CorpusForm.Module.Form",
            "Module",
        ),
    ] {
        assert_view(&runtime, root.path(), at, kind);
    }
}
