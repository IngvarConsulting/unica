//! Outline of the module that is on disk: the reads go through the workspace
//! read port, so these cases need a real source tree.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use unica_bsl::outline::CodeOutlineResult;
use unica_code_intelligence::current_outline::render_current_source_outline;
use unica_code_intelligence::model::CodeIntelligenceContext;
use unica_source_contract::workspace::ResolvedSourceRoot;
use unica_source_contract::workspace::WorkspaceContext;
use unica_work_control::cancellation::CancellationToken;
use unica_work_control::provider_deadline::ProviderDeadline;
use unica_workspace::discovery::ProjectDiscoveryReader;

struct Workspace {
    root: PathBuf,
    context: CodeIntelligenceContext,
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn workspace(label: &str, path: &str, contents: &str) -> Workspace {
    let root = std::env::temp_dir().join(format!(
        "unica-bsl-outline-{label}-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    let source_root = root.join("src");
    let module = source_root.join(path);
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, contents).unwrap();
    let context = CodeIntelligenceContext::new(
        WorkspaceContext {
            cwd: root.clone(),
            workspace_root: root.clone(),
            cache_root: root.join(".build/unica"),
            workspace_epoch: 1,
        },
        ResolvedSourceRoot {
            source_set: Some("main".to_string()),
            path: source_root,
        },
        Arc::new(ProjectDiscoveryReader::new(&root)),
    );
    Workspace { root, context }
}

fn outline(
    workspace: &Workspace,
    path: &str,
    include_methods: bool,
) -> Result<CodeOutlineResult, String> {
    render_current_source_outline(
        path,
        include_methods,
        &workspace.context,
        ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
        &CancellationToken::new(),
    )
    .map(|(result, _)| result)
}

#[test]
fn outline_reads_the_file_again_after_it_changed() {
    let workspace = workspace(
        "refresh",
        "CommonModules/X/Ext/Module.bsl",
        "#Область Р\nПроцедура Старый() Экспорт\nКонецПроцедуры\n#КонецОбласти\n",
    );
    let first = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap();
    assert_eq!(first.regions[0].methods[0].name, "Старый");
    assert_eq!(first.regions[0].methods[0].line, 2);
    assert_eq!(first.regions[0].methods[0].end_line, 3);

    fs::write(
        workspace.context.source_root.path.join("CommonModules/X/Ext/Module.bsl"),
        "#Область Р\nПроцедура Старый() Экспорт\nКонецПроцедуры\nПроцедура Новый() Экспорт\nКонецПроцедуры\n#КонецОбласти\n",
    )
    .unwrap();

    let second = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap();
    assert_eq!(second.regions[0].methods[1].name, "Новый");
    assert_eq!(second.regions[0].methods[1].line, 4);
    assert_eq!(second.regions[0].methods[1].end_line, 5);
    assert_eq!(second.totals.methods, 2);
    assert_eq!(second.totals.exports, 2);
}

#[test]
fn parser_diagnostics_fail_the_call_without_a_partial_outline() {
    let workspace = workspace(
        "broken",
        "CommonModules/X/Ext/Module.bsl",
        "Процедура П(\nКонецПроцедуры\nЕсли Тогда\n",
    );

    let error = outline(&workspace, "CommonModules/X/Ext/Module.bsl", true).unwrap_err();

    assert!(error.contains("parser reported"), "{error}");
    assert!(!error.contains("region"), "{error}");
}

#[test]
fn a_missing_module_fails_with_the_requested_path() {
    let workspace = workspace("missing", "CommonModules/X/Ext/Module.bsl", "");

    let error = outline(&workspace, "CommonModules/Нет/Ext/Module.bsl", true).unwrap_err();

    assert!(
        error.contains("CommonModules/Нет/Ext/Module.bsl"),
        "{error}"
    );
    assert!(error.contains("does not exist"), "{error}");
    assert!(error.contains("BSL module file"), "{error}");
}

#[test]
fn a_path_that_names_a_directory_says_which_argument_is_wrong() {
    // The raw OS message for this mistake is "Is a directory", which names
    // neither the argument nor what it expects. A bare object name lands
    // here too, because the index used to resolve such names and the current
    // source path cannot.
    let workspace = workspace("directory", "CommonModules/X/Ext/Module.bsl", "");

    for path in ["CommonModules/X", "CommonModules/X/Ext"] {
        let error = outline(&workspace, path, true).unwrap_err();

        assert!(error.contains("is a directory"), "{path}: {error}");
        assert!(error.contains("BSL module file"), "{path}: {error}");
        assert!(!error.contains("os error"), "{path}: {error}");
    }
}

#[test]
fn a_path_outside_the_source_root_is_rejected_before_reading() {
    let workspace = workspace("escape", "CommonModules/X/Ext/Module.bsl", "");
    fs::write(
        workspace.root.join("outside.bsl"),
        "Процедура П()\nКонецПроцедуры\n",
    )
    .unwrap();

    let error = outline(&workspace, "../outside.bsl", true).unwrap_err();

    assert!(
        error.contains("outside the selected source root"),
        "{error}"
    );
}

#[test]
fn no_deadline_outline_reads_source_but_preserves_scope_and_explicit_cancel() {
    let workspace = workspace(
        "no-deadline",
        "CommonModules/X/Ext/Module.bsl",
        "Procedure Post()\nEndProcedure\n",
    );
    let deadline = ProviderDeadline::no_deadline();
    let read = |path: &str, cancellation: &CancellationToken| {
        render_current_source_outline(path, true, &workspace.context, deadline, cancellation)
    };
    let (outline, _) = read("CommonModules/X/Ext/Module.bsl", &CancellationToken::new()).unwrap();
    assert_eq!(outline.totals.methods, 1);
    fs::write(
        workspace.root.join("outside.bsl"),
        "Procedure Foreign()\nEndProcedure\n",
    )
    .unwrap();
    assert!(read("../outside.bsl", &CancellationToken::new())
        .unwrap_err()
        .contains("outside the selected source root"));
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(read("CommonModules/X/Ext/Module.bsl", &cancellation)
        .unwrap_err()
        .starts_with("cancelled:"));
}

#[test]
fn cancellation_and_an_exhausted_deadline_stop_before_reading() {
    let workspace = workspace("stop", "CommonModules/X/Ext/Module.bsl", "");
    let cancelled = CancellationToken::new();
    cancelled.cancel();

    let error = render_current_source_outline(
        "CommonModules/X/Ext/Module.bsl",
        true,
        &workspace.context,
        ProviderDeadline::new(Instant::now() + Duration::from_secs(30)),
        &cancelled,
    )
    .unwrap_err();
    assert!(error.contains("stopped before reading"), "{error}");

    let error = render_current_source_outline(
        "CommonModules/X/Ext/Module.bsl",
        true,
        &workspace.context,
        ProviderDeadline::new(Instant::now()),
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert!(error.contains("deadline exceeded"), "{error}");
}
