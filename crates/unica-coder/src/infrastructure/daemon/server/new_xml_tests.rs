use super::actor_capacity_tests::direct_v5;
use super::*;
use crate::application::invocation_store::ToolIdentity;
use crate::test_support::tree_snapshot;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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
