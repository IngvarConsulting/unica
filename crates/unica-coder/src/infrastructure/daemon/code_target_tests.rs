//! Канонические `code.insert` и `code.replace` пишут в модуль, а не в узел,
//! которому модуль принадлежит (#1964, INV.SOURCE.WRITE-TARGET-KIND).

use super::actor_capacity_tests::direct_v5;
use super::*;
use unica_execution::invocation_store::ToolIdentity;

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use unica_execution_contract::invocation::DomainResult;

struct Fixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    runtime: V5CanonicalInvocationRuntime,
}

impl Fixture {
    fn code() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            unica_workspace::test_support::canonical_path(root.path()).join("workspace");
        let image = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/acceptance/workspace-code");
        for (relative, bytes) in unica_workspace::test_support::tree_snapshot(&image) {
            let path = workspace.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        Self {
            _root: root,
            workspace,
            runtime: V5CanonicalInvocationRuntime::new(
                Arc::new(
                    crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
                ),
                Arc::new(TokioClock),
            ),
        }
    }

    fn call(&self, tool: ToolIdentity, args: Value) -> DomainResult {
        direct_v5(
            &self.runtime,
            InvocationRequest::new(tool, args, self.workspace.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    }

    /// A preview of one code operation at `at`, with `extra` arguments.
    fn preview(&self, op: &str, at: &str, extra: Value) -> DomainResult {
        let mut args = json!({"at": at, "text": "Процедура Добавлена()\nКонецПроцедуры"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        self.call(
            ToolIdentity::Apply,
            json!({"at": at, "ops": [{"op": op, "args": args}]}),
        )
    }
}

#[test]
fn canonical_code_edits_write_only_at_a_module_address() {
    let fixture = Fixture::code();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);

    // A module address, its `Body` or `Method.M` is the write target. The
    // common module is its own module: its object address is a module
    // address, not an owner.
    let object = "main:Catalog.Пример.Module.Object";
    let method = "main:Catalog.Пример.Module.Object.Method.ОбработатьПример";
    for (op, at, extra, module) in [
        ("code.insert", object.to_string(), json!({}), object),
        (
            "code.insert",
            "main:CommonModule.Пример".to_string(),
            json!({}),
            "main:CommonModule.Пример",
        ),
        ("code.insert", format!("{object}.Body"), json!({}), object),
        (
            "code.insert",
            method.to_string(),
            json!({"position": "after"}),
            object,
        ),
        ("code.replace", method.to_string(), json!({}), object),
        ("code.replace", format!("{object}.Body"), json!({}), object),
        ("code.replace", format!("{method}.Body"), json!({}), object),
    ] {
        let at = at.as_str();
        let planned = fixture.preview(op, at, extra);
        assert!(planned.ok, "{op} {at}: {planned:?}");
        assert_eq!(
            planned.changed,
            vec![json!({"at": module, "event": "ModuleChanged"})],
            "{op} {at}"
        );
    }

    // The source-set root, an object that owns modules, the module branch
    // and a node without modules do not name one module. The refusal comes
    // before any staging; a node with modules leads to their addresses.
    for op in ["code.insert", "code.replace"] {
        for (at, branch, module) in [
            (
                "main:Configuration",
                Some("main:Module"),
                "main:Module.Session",
            ),
            (
                "main:Catalog.Пример",
                Some("main:Catalog.Пример.Module"),
                "main:Catalog.Пример.Module.Object",
            ),
            ("main:Catalog.Пример.Module", None, ""),
            ("main:Language.Русский", None, ""),
        ] {
            let refused = fixture.preview(op, at, json!({}));
            assert!(!refused.ok, "{op} {at}: {refused:?}");
            assert_eq!(refused.diagnostics.len(), 1, "{op} {at}: {refused:?}");
            assert_eq!(
                refused.diagnostics[0]["code"], "bad_value",
                "{op} {at}: {refused:?}"
            );
            let message = refused.diagnostics[0]["message"].as_str().unwrap();
            assert!(
                message.contains("does not apply to")
                    && message.contains(&format!("`{at}`"))
                    && message.contains("module address"),
                "{op} {at}: {message}"
            );
            assert!(refused.changed.is_empty(), "{op} {at}: {refused:?}");
            assert!(
                refused
                    .data
                    .as_ref()
                    .is_none_or(|data| data.get("executionToken").is_none()),
                "{op} {at}: {refused:?}"
            );
            let views = refused
                .next
                .iter()
                .filter(|next| next["tool"] == "unica.view")
                .map(|next| next["args"]["at"].as_str().unwrap().to_string())
                .collect::<Vec<_>>();
            match branch {
                Some(branch) => {
                    assert_eq!(
                        views.first().map(String::as_str),
                        Some(branch),
                        "{op} {at}: {refused:?}"
                    );
                    // The branch lists the module the agent writes next.
                    let listed = fixture.call(ToolIdentity::View, json!({"at": branch}));
                    assert!(listed.ok, "{branch}: {listed:?}");
                    let listed = serde_json::to_string(&listed.data).unwrap();
                    assert!(
                        listed.contains(&format!("\"{module}\"")),
                        "{branch}: {listed}"
                    );
                }
                None => assert!(
                    views.iter().all(|view| view == at),
                    "{op} {at}: only the node itself is offered: {refused:?}"
                ),
            }
        }
    }
    assert_eq!(
        unica_workspace::test_support::tree_snapshot(&fixture.workspace),
        before,
        "previews and refusals write nothing"
    );
}

/// The saved plan keeps the owner descriptors it was planned against: an
/// owner changed after the preview refuses execution by token, and the
/// module stays as it was.
#[test]
fn canonical_code_execution_refuses_a_module_owner_changed_after_preview() {
    for owner in ["CommonModules/Пример.xml", "Configuration.xml"] {
        let fixture = Fixture::code();
        let module = fixture
            .workspace
            .join("src/cf/CommonModules/Пример/Ext/Module.bsl");
        let original = std::fs::read(&module).unwrap();
        let planned = fixture.preview("code.insert", "main:CommonModule.Пример", json!({}));
        assert!(planned.ok, "{owner}: {planned:?}");
        let token = planned.data.as_ref().unwrap()["executionToken"].clone();
        let path = fixture.workspace.join("src/cf").join(owner);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            text.replace("</MetaDataObject>", "<!-- changed --></MetaDataObject>"),
        )
        .unwrap();

        let executed = fixture.call(ToolIdentity::Apply, json!({"executionToken": token}));
        assert!(!executed.ok, "{owner}: {executed:?}");
        assert_eq!(
            executed.diagnostics[0]["code"], "stale_revision",
            "{owner}: {executed:?}"
        );
        let message = executed.diagnostics[0]["message"].as_str().unwrap();
        assert!(message.contains(owner), "{owner}: {message}");
        assert_eq!(std::fs::read(&module).unwrap(), original, "{owner}");
    }
}
