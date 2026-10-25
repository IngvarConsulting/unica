use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;

const OWNER: &str = "Catalog.Editable";

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
}

impl Fixture {
    fn new(existing_cache: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-legacy-inline");
        for (relative, bytes) in unica_workspace::test_support::tree_snapshot(&fixture) {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        if existing_cache {
            let cache = workspace.join(".build/unica");
            std::fs::create_dir_all(cache.join("untouched-empty")).unwrap();
            std::fs::write(cache.join("unrelated.bin"), b"\0\xffexisting cache\r\n").unwrap();
        }
        Self {
            _root: root,
            workspace,
        }
    }

    fn owner_path(&self) -> PathBuf {
        self.workspace.join("src/Catalogs/Editable.xml")
    }

    fn use_name_registrations(&self) {
        let mut owner = std::fs::read_to_string(self.owner_path()).unwrap();
        for (kind, collection, name) in [
            ("Form", "Forms", "Main"),
            ("Template", "Templates", "Print"),
        ] {
            let descriptor = std::fs::read_to_string(
                self.workspace
                    .join(format!("src/Catalogs/Editable/{collection}/{name}.xml")),
            )
            .unwrap();
            let document = roxmltree::Document::parse(&descriptor).unwrap();
            let child = document
                .root_element()
                .children()
                .find(|node| node.is_element())
                .unwrap();
            let inline = &descriptor[child.range()];
            assert_eq!(owner.matches(inline).count(), 1);
            owner = owner.replace(inline, &format!("<{kind}>{name}</{kind}>"));
        }
        assert!(owner.contains("<Command uuid="));
        assert!(!self
            .workspace
            .join("src/Catalogs/Editable/Commands/Open.xml")
            .exists());
        std::fs::write(self.owner_path(), owner).unwrap();
    }

    fn check_without_writes(&self) -> Value {
        let files_before = unica_workspace::test_support::tree_snapshot(&self.workspace);
        let directories_before = directory_membership(&self.workspace);
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let checked = direct_v5(
            &runtime,
            InvocationRequest::new(
                ToolIdentity::Check,
                json!({"at": format!("main:{OWNER}")}),
                self.workspace.to_string_lossy(),
                7_000,
            )
            .unwrap(),
        )
        .unwrap();

        // Include cache bytes and empty directories: a files-only snapshot
        // would miss a check that creates a previously absent cache directory.
        assert_eq!(
            files_before,
            unica_workspace::test_support::tree_snapshot(&self.workspace),
            "check must preserve source and cache bytes and file membership"
        );
        assert_eq!(
            directories_before,
            directory_membership(&self.workspace),
            "check must preserve source and cache directory membership"
        );
        assert!(
            checked.ok,
            "check must return its validation verdict: {checked:?}"
        );
        let data = checked.data.unwrap();
        assert_eq!(data["validators"], json!(["meta"]), "{data}");
        data
    }
}

fn directory_membership(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(root: &Path, current: &Path, found: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                let path = entry.path();
                found.insert(path.strip_prefix(root).unwrap().to_owned());
                visit(root, &path, found);
            }
        }
    }
    let mut found = BTreeSet::new();
    visit(root, root, &mut found);
    found
}

fn assert_legacy_warnings(data: &Value) {
    let warnings = data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
        .collect::<Vec<_>>();
    assert_eq!(
        warnings.len(),
        2,
        "one warning for each legacy registration: {data}"
    );
    let fields = warnings
        .iter()
        .map(|diagnostic| {
            assert_eq!(diagnostic["code"], "validation_warning", "{diagnostic}");
            assert_eq!(diagnostic["metadataPath"], OWNER, "{diagnostic}");
            assert_eq!(diagnostic["validator"], "meta", "{diagnostic}");
            assert!(
                diagnostic["message"].as_str().unwrap().contains("8.3.27"),
                "the warning must identify the affected platform: {diagnostic}"
            );
            diagnostic["field"].as_str().unwrap()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        fields,
        BTreeSet::from(["childObjects.Form.Main", "childObjects.Template.Print"]),
        "identify the inline Form/Template, never the valid inline Command"
    );
}

#[test]
fn canonical_check_warns_about_legacy_inline_form_and_template_without_writes() {
    for existing_cache in [false, true] {
        let fixture = Fixture::new(existing_cache);
        let data = fixture.check_without_writes();
        assert_eq!(data["status"], "passed", "{data}");
        assert_legacy_warnings(&data);
        assert_eq!(data["diagnosticCount"], 2, "{data}");
    }
}

#[test]
fn canonical_check_accepts_by_name_form_template_and_inline_command_without_writes() {
    for existing_cache in [false, true] {
        let fixture = Fixture::new(existing_cache);
        fixture.use_name_registrations();
        let data = fixture.check_without_writes();
        assert_eq!(data["status"], "passed", "{data}");
        assert_eq!(data["diagnostics"], json!([]), "{data}");
        assert_eq!(data["diagnosticCount"], 0, "{data}");
    }
}

#[test]
fn canonical_check_keeps_errors_and_legacy_inline_warnings_in_a_failed_verdict() {
    let fixture = Fixture::new(true);
    let owner = std::fs::read_to_string(fixture.owner_path()).unwrap();
    let broken = owner.replace(
        "<CodeType>String</CodeType>",
        "<CodeType>NotACodeType</CodeType>",
    );
    assert_ne!(broken, owner, "the control injects a real property error");
    std::fs::write(fixture.owner_path(), broken).unwrap();

    let data = fixture.check_without_writes();
    assert_eq!(data["status"], "failed", "{data}");
    assert_legacy_warnings(&data);
    let errors = data["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "error")
        .collect::<Vec<_>>();
    assert_eq!(errors.len(), 1, "{data}");
    assert_eq!(errors[0]["code"], "validation_failed", "{data}");
    assert_eq!(errors[0]["metadataPath"], OWNER, "{data}");
    assert!(errors[0]["message"].as_str().unwrap().contains("CodeType"));
    assert_eq!(data["diagnosticCount"], 3, "{data}");
}

#[test]
fn canonical_check_keeps_malformed_form_payload_an_error_with_legacy_registration() {
    let fixture = Fixture::new(true);
    std::fs::write(
        fixture
            .workspace
            .join("src/Catalogs/Editable/Forms/Main/Ext/Form.xml"),
        b"<Form xmlns=\"http://v8.1c.ru/8.3/xcf/logform\" version=\"2.20\"><ChildItems>",
    )
    .unwrap();

    let data = fixture.check_without_writes();
    assert_eq!(data["status"], "failed", "{data}");
    assert!(
        data["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|diagnostic| {
                diagnostic["severity"] == "error"
                    && diagnostic["code"] == "provider_unavailable"
                    && diagnostic["metadataPath"] == "Catalog.Editable.Form.Main"
            }),
        "a valid separate descriptor cannot hide the malformed form payload: {data}"
    );
}

/// Any edit of the owner registers its legacy inline forms and templates by
/// name (#1370): the preview names them in a warning, the publication writes
/// the references, the inline command stays and check is quiet afterwards.
#[test]
fn an_owner_edit_registers_legacy_inline_forms_and_templates_by_name() {
    for ops in [
        json!([{"op": "props.set", "args": {"values": {"CodeLength": 10}}}]),
        json!([{"op": "form.add", "args": {"items": [{"name": "Extra", "type": "ObjectForm"}]}}]),
    ] {
        let fixture = Fixture::new(false);
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        let call = |tool, args| {
            direct_v5(
                &runtime,
                InvocationRequest::new(tool, args, fixture.workspace.to_string_lossy(), 7_000)
                    .unwrap(),
            )
            .unwrap()
        };
        let planned = call(
            ToolIdentity::Apply,
            json!({"at": format!("main:{OWNER}"), "ops": ops}),
        );
        assert!(planned.ok, "{ops}: {planned:?}");
        let warnings = planned
            .warnings
            .iter()
            .filter(|warning| warning["code"] == "legacy_inline_registration")
            .collect::<Vec<_>>();
        assert_eq!(warnings.len(), 1, "{ops}: {planned:?}");
        assert_eq!(warnings[0]["at"], format!("main:{OWNER}"));
        let message = warnings[0]["message"].as_str().unwrap();
        assert!(
            message.contains("Form.Main") && message.contains("Template.Print"),
            "{message}"
        );
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        let published = call(ToolIdentity::Apply, json!({"executionToken": token}));
        assert!(published.ok, "{ops}: {published:?}");

        let owner = std::fs::read_to_string(fixture.owner_path()).unwrap();
        assert!(owner.contains("<Form>Main</Form>"), "{owner}");
        assert!(owner.contains("<Template>Print</Template>"), "{owner}");
        assert!(!owner.contains("<Form uuid="), "{owner}");
        assert!(!owner.contains("<Template uuid="), "{owner}");
        assert!(owner.contains("<Command uuid="), "{owner}");

        let checked = fixture.check_without_writes();
        let diagnostics = checked["data"]["diagnostics"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        assert!(
            diagnostics.iter().all(|diagnostic| !diagnostic
                .to_string()
                .contains("Legacy inline registration")),
            "{ops}: {checked}"
        );
    }
}
