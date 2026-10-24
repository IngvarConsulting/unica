//! `panels.set` and `homePage.set` on the configuration root through the
//! canonical runtime. The expected documents are the 8.3.27 re-export of the
//! edited configuration (`tests/fixtures/platform_8_3_27/client_interface`).
use super::actor_capacity_tests::direct_v5;
use super::*;
use serde_json::json;
use std::path::{Path, PathBuf};
use unica_execution::invocation_store::ToolIdentity;
use unica_workspace::test_support::tree_snapshot;

const PANELS: &str = "Ext/ClientApplicationInterface.xml";
const HOME_PAGE: &str = "Ext/HomePageWorkArea.xml";

fn repository(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// A copy of the native 8.3.27 workspace with a configuration and an
/// extension; the configuration holds a common form, a processor form and a
/// role for the home page.
fn workspace() -> tempfile::TempDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }
    let root = tempfile::tempdir().unwrap();
    copy(
        &repository("tests/fixtures/acceptance/workspace-borrowed"),
        root.path(),
    );
    root
}

fn runtime() -> V5CanonicalInvocationRuntime {
    V5CanonicalInvocationRuntime::new(
        Arc::new(crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default()),
        Arc::new(TokioClock),
    )
}

fn call(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    tool: ToolIdentity,
    arguments: serde_json::Value,
) -> DomainResult {
    let root = std::fs::canonicalize(root).unwrap();
    let request = InvocationRequest::new(tool, arguments, root.to_string_lossy(), 7_000).unwrap();
    direct_v5(runtime, request).unwrap()
}

/// Plans `ops` on `at`, proves the preview wrote nothing and publishes it;
/// returns the number of planned effects.
fn apply(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    at: &str,
    ops: serde_json::Value,
) -> u64 {
    let before = tree_snapshot(&root.join("src"));
    let planned = call(
        runtime,
        root,
        ToolIdentity::Apply,
        json!({"at":at,"ops":ops}),
    );
    assert!(planned.ok, "preview failed: {planned:?}");
    let data = planned.data.as_ref().unwrap();
    assert_eq!(tree_snapshot(&root.join("src")), before, "preview wrote");
    let published = call(
        runtime,
        root,
        ToolIdentity::Apply,
        json!({"executionToken":data["executionToken"]}),
    );
    assert!(published.ok, "publication failed: {published:?}");
    data["effects"].as_u64().unwrap()
}

fn refused(
    runtime: &V5CanonicalInvocationRuntime,
    root: &Path,
    at: &str,
    ops: serde_json::Value,
) -> DomainResult {
    let before = tree_snapshot(&root.join("src"));
    let result = call(
        runtime,
        root,
        ToolIdentity::Apply,
        json!({"at":at,"ops":ops}),
    );
    assert!(!result.ok, "{result:?}");
    assert_eq!(tree_snapshot(&root.join("src")), before, "a refusal wrote");
    result
}

fn root_props(runtime: &V5CanonicalInvocationRuntime, root: &Path) -> serde_json::Value {
    let view = call(
        runtime,
        root,
        ToolIdentity::View,
        json!({"at":"main:Configuration"}),
    );
    assert!(view.ok, "{view:?}");
    view.data.unwrap()["props"].clone()
}

fn assert_platform_bytes(root: &Path, case: &str) {
    for relative in [PANELS, HOME_PAGE] {
        let name = Path::new(relative).file_name().unwrap();
        assert_eq!(
            std::fs::read(root.join("src/cf").join(relative)).unwrap(),
            std::fs::read(
                repository("tests/fixtures/platform_8_3_27/client_interface")
                    .join(case)
                    .join(name)
            )
            .unwrap(),
            "{case}: {relative} differs from the platform re-export"
        );
    }
}

#[test]
fn panels_and_home_page_publish_the_platform_bytes_and_read_back() {
    let root = workspace();
    let runtime = runtime();
    assert_eq!(
        root_props(&runtime, root.path())["panels"],
        json!({"top":["open"],"left":["sections"]})
    );
    let panels = json!({"top":[["tools","open"]],"left":["sections"],"bottom":["favorites"]});
    let home_page = json!({"template":"TwoColumnsEqualWidth",
        "left":[{"form":"CommonForm.ОбщаяФорма","height":15,"common":false,
            "roles":[{"role":"Role.SalesReader","visible":true}]}],
        "right":[{"form":"DataProcessor.Обработка.Form.Форма"}]});
    let ops = json!([
        {"op":"panels.set","args":{"values":panels}},
        {"op":"homePage.set","args":{"values":home_page}},
    ]);
    assert_ne!(apply(&runtime, root.path(), "main:Configuration", ops), 0);
    assert_platform_bytes(root.path(), "two");
    let props = root_props(&runtime, root.path());
    assert_eq!(props["panels"], panels);
    assert_eq!(props["homePage"]["template"], "TwoColumnsEqualWidth");
    assert_eq!(
        props["homePage"]["left"][0]["roles"],
        json!([{"role":"Role.SalesReader","visible":true}])
    );
    // The read layout goes back as is and the repeat writes nothing.
    let unchanged = tree_snapshot(&root.path().join("src"));
    let repeat = json!([
        {"op":"panels.set","args":{"values":props["panels"].clone()}},
        {"op":"homePage.set","args":{"values":home_page}},
    ]);
    assert_eq!(
        apply(&runtime, root.path(), "main:Configuration", repeat),
        0
    );
    assert_eq!(tree_snapshot(&root.path().join("src")), unchanged);

    // One column and a group of two panels, on a fresh copy.
    let root = workspace();
    assert_ne!(
        apply(
            &runtime,
            root.path(),
            "main:Configuration",
            json!([
                {"op":"panels.set","args":{"values":{"left":[["sections","history"]],"right":["functions"]}}},
                {"op":"homePage.set","args":{"values":{"template":"OneColumn",
                    "left":[{"form":"DataProcessor.Обработка.Form.Форма","height":5}]}}},
            ])
        ),
        0
    );
    assert_platform_bytes(root.path(), "one");
}

#[test]
fn layouts_the_platform_drops_or_refuses_are_refused_before_writing() {
    let root = workspace();
    let runtime = runtime();
    for (op, values, at, code) in [
        (
            "panels.set",
            json!({"left":["tools"]}),
            "ops[0].args.values.left[0]",
            "bad_value",
        ),
        (
            "panels.set",
            json!({"top":["open"],"left":[["sections","open"]]}),
            "ops[0].args.values.left[0]",
            "bad_value",
        ),
        (
            "homePage.set",
            json!({"template":"OneColumn","left":[{"form":"CommonForm.Нет"}]}),
            "ops[0].args.values.left[0].form",
            "not_found",
        ),
        (
            "homePage.set",
            json!({"template":"OneColumn","left":[{"form":"CommonForm.ОбщаяФорма",
            "roles":[{"role":"Role.Нет","visible":true}]}]}),
            "ops[0].args.values.left[0].roles[0].role",
            "not_found",
        ),
        (
            "homePage.set",
            json!({"template":"OneColumn","left":[{"form":"Catalog.Нет"}]}),
            "ops[0].args.values.left[0].form",
            "bad_value",
        ),
        (
            "homePage.set",
            json!({"template":"OneColumn","right":[{"form":"CommonForm.ОбщаяФорма"}]}),
            "ops[0].args.values.right",
            "bad_value",
        ),
        // The published schema is the parser's: a misspelled key is refused,
        // not dropped.
        (
            "panels.set",
            json!({"Top":["open"]}),
            "ops[0].args.values.Top",
            "bad_value",
        ),
        (
            "homePage.set",
            json!({"template":"OneColumn","left":[{"form":"CommonForm.ОбщаяФорма","heigth":15}]}),
            "ops[0].args.values.left[0].heigth",
            "bad_value",
        ),
    ] {
        let result = refused(
            &runtime,
            root.path(),
            "main:Configuration",
            json!([{"op":op,"args":{"values":values}}]),
        );
        assert_eq!(result.diagnostics[0]["code"], code, "{values}: {result:?}");
        assert_eq!(result.at.as_deref(), Some(at), "{values}: {result:?}");
    }
    // An extension root has no panel layout or home page of its own.
    for op in ["panels.set", "homePage.set"] {
        let values = if op == "panels.set" {
            json!({"top":["open"]})
        } else {
            json!({"template":"OneColumn"})
        };
        refused(
            &runtime,
            root.path(),
            "ext:Configuration",
            json!([{"op":op,"args":{"values":values}}]),
        );
    }
}

/// A reference the platform wrote by identifier and an unknown panel come
/// back from the read: the first is accepted as stored, the second is shown
/// and keeps `panels.set` from silently dropping it.
#[test]
fn stored_references_and_unknown_panels_survive_the_read() {
    let root = workspace();
    let runtime = runtime();
    let ext = root.path().join("src/cf/Ext");
    let page = "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\r\n<HomePageWorkArea xmlns=\"http://v8.1c.ru/8.3/xcf/extrnprops\" xmlns:xr=\"http://v8.1c.ru/8.3/xcf/readable\" xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" version=\"2.20\">\r\n\t<WorkingAreaTemplate>OneColumn</WorkingAreaTemplate>\r\n\t<Column>\r\n\t\t<Item>\r\n\t\t\t<Form>30c48a9e-40d9-4939-8c8e-ab0824960295</Form>\r\n\t\t\t<Height>10</Height>\r\n\t\t\t<Visibility>\r\n\t\t\t\t<xr:Common>true</xr:Common>\r\n\t\t\t</Visibility>\r\n\t\t</Item>\r\n\t</Column>\r\n</HomePageWorkArea>";
    std::fs::write(ext.join("HomePageWorkArea.xml"), page).unwrap();
    let mut props = root_props(&runtime, root.path());
    let mut home_page = props["homePage"].clone();
    assert_eq!(
        home_page["left"][0]["form"],
        "30c48a9e-40d9-4939-8c8e-ab0824960295"
    );
    home_page["left"]
        .as_array_mut()
        .unwrap()
        .push(json!({"form":"CommonForm.ОбщаяФорма"}));
    assert_ne!(
        apply(
            &runtime,
            root.path(),
            "main:Configuration",
            json!([{"op":"homePage.set","args":{"values":home_page}}])
        ),
        0
    );
    props = root_props(&runtime, root.path());
    assert_eq!(props["homePage"]["left"].as_array().unwrap().len(), 2);

    let layout = std::fs::read_to_string(ext.join("ClientApplicationInterface.xml"))
        .unwrap()
        .replace(
            "<uuid>b553047f-c9aa-4157-978d-448ecad24248</uuid>",
            "<uuid>11111111-2222-4333-8444-555555555555</uuid>",
        );
    std::fs::write(ext.join("ClientApplicationInterface.xml"), layout).unwrap();
    assert_eq!(
        root_props(&runtime, root.path())["panels"],
        json!({"top":["open"],"unknown":["11111111-2222-4333-8444-555555555555"]})
    );
    let result = refused(
        &runtime,
        root.path(),
        "main:Configuration",
        json!([{"op":"panels.set","args":{"values":{"top":["open"]}}}]),
    );
    assert_eq!(
        result.diagnostics[0]["code"], "invalid_source",
        "{result:?}"
    );
}
