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
        }
    }

    fn call(&self, tool: ToolIdentity, args: Value) -> DomainResult {
        let runtime = V5CanonicalInvocationRuntime::new(
            Arc::new(
                crate::infrastructure::daemon::v13_service::CanonicalV13ReadService::default(),
            ),
            Arc::new(TokioClock),
        );
        direct_v5(
            &runtime,
            InvocationRequest::new(tool, args, self.workspace.to_string_lossy(), 7_000).unwrap(),
        )
        .unwrap()
    }

    /// A preview of one code operation at `at`.
    fn preview(&self, op: &str, at: &str) -> DomainResult {
        let text = "Процедура Добавлена()\nКонецПроцедуры";
        self.call(
            ToolIdentity::Apply,
            json!({"at": at, "ops": [{"op": op, "args": {"at": at, "text": text}}]}),
        )
    }
}

#[test]
fn canonical_code_edits_write_only_at_a_module_address() {
    let fixture = Fixture::code();
    let before = unica_workspace::test_support::tree_snapshot(&fixture.workspace);

    // A module address is the write target. The common module is its own
    // module: its object address is a module address, not an owner.
    for (op, at, module) in [
        (
            "code.insert",
            "main:Catalog.Пример.Module.Object",
            "main:Catalog.Пример.Module.Object",
        ),
        (
            "code.insert",
            "main:CommonModule.Пример",
            "main:CommonModule.Пример",
        ),
        (
            "code.replace",
            "main:Catalog.Пример.Module.Object.Method.ОбработатьПример",
            "main:Catalog.Пример.Module.Object",
        ),
    ] {
        let planned = fixture.preview(op, at);
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
            let refused = fixture.preview(op, at);
            assert!(!refused.ok, "{op} {at}: {refused:?}");
            assert_eq!(refused.diagnostics.len(), 1, "{op} {at}: {refused:?}");
            assert_eq!(refused.diagnostics[0]["code"], "bad_value", "{op} {at}");
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
