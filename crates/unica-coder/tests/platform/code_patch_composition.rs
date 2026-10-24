use super::{
    arguments, hash, parse_code_plan_operation, patch_inner, plan_code_batch_owned_for_test,
    plan_code_postimage, unified_diff, CodeInsertArgs, CodePlanOperation, CodePosition,
    CodeReplaceArgs, CodeSelector, PatchMode, ValidationStatus,
};
use crate::application::SupportGuardRequirement;
use unica_source_contract::events::{DomainEvent, DomainEventKind};
use unica_source_contract::workspace::WorkspaceContext;
use unica_source_contract::{SourceFormat, SourceProfile, SourceSetKind};

use crate::infrastructure::native_operations::common::support_guard_violation;
use crate::infrastructure::source_workspace::WorkspaceActor;

use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};
use unica_source_contract::proposal::SupportPolicyMode;
use unica_source_contract::proposal::{
    ApplyPlanError, ApplyPlanErrorKind, ApplyStagingErrorKind, StagedFileState,
};
use unica_workspace::actor::{
    ApplyEffectDisposition, ProviderRootBinding, WorkspaceIdentity, WorkspaceSourceSetInput,
};
use unica_workspace::single_file_publisher::with_before_commit_hook;
use unica_workspace::test_support::{
    attempt_retained_directory_replacement_for_test, create_file_link_fixture_for_test,
    path_identity_for_test, FileLinkFixtureOutcome, RetainedDirectoryReplacementOutcome,
};

const MODULE: &str = "Процедура Первая()\n    Сообщить(\"один\");\nКонецПроцедуры\n\nФункция Вторая()\n    Возврат Истина;\nКонецФункции\n";
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn emitted_diff_is_accepted_by_git_and_reproduces_postimage() {
    let root = temp_root("git-diff-roundtrip");
    let relative = "src/CommonModules/X/Ext/Module.bsl";
    let target = root.join(relative);
    fs::create_dir_all(target.parent().unwrap()).unwrap();
    let before = b"Procedure Run()\r\nEndProcedure";
    let after = b"Procedure Run()\r\nEndProcedure\r\nProcedure Added()\r\nEndProcedure\r\n";
    fs::write(&target, before).unwrap();
    let diff = unified_diff(
        relative,
        std::str::from_utf8(before).unwrap(),
        std::str::from_utf8(after).unwrap(),
    )
    .unwrap();
    fs::write(root.join("change.diff"), diff).unwrap();
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["config", "core.autocrlf", "false"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["apply", "--check", "change.diff"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("git")
        .args(["apply", "change.diff"])
        .current_dir(&root)
        .status()
        .unwrap()
        .success());
    assert_eq!(fs::read(&target).unwrap(), after);
    fs::remove_dir_all(root).unwrap();
}

#[test]
pub(crate) fn applied_patch_returns_typed_data_and_repeated_apply_is_noop_with_stable_identity() {
    use unica_workspace::test_support::file_identity_for_test;

    let context = temp_context("applied-patch");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n// untouched suffix\r\n";
    let expected = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n\r\nProcedure Added()\r\nEndProcedure\r\n// untouched suffix\r\n";
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let preview = patch_inner(&args, &context, PatchMode::Preview);
    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    assert_eq!(fs::read(&module).unwrap(), before);

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(fs::read(&module).unwrap(), expected);
    assert!(applied.outcome.stdout.is_none());
    let data = applied.data.unwrap();
    assert_eq!(data.source_set, "main");
    assert_eq!(data.affected_target.owner, "CommonModule.Sample");
    assert_eq!(data.affected_target.module_role, "Module");
    assert_eq!(data.validation.status, ValidationStatus::Passed);
    assert_eq!(data.validation.validated_post_hash, data.post_hash);
    assert_eq!(data.changed_ranges[0].start_line, 3);
    assert!(data.diff.starts_with("--- a/"));
    let identity = file_identity_for_test(&module).unwrap();

    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
    assert_eq!(fs::read(&module).unwrap(), expected);
    assert_eq!(file_identity_for_test(&module).unwrap(), identity);
    assert!(repeated.outcome.changes.is_empty());
    let data = repeated.data.unwrap();
    assert_eq!(data.pre_hash, data.post_hash);
    assert!(data.changed_ranges.is_empty());
    assert!(data.diff.is_empty());
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_configuration_and_extension_preview_apply_preserve_unrelated_bytes() {
    let root = temp_root("code-patch-config-extension");
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        concat!(
            "format: DESIGNER\n",
            "source-set:\n",
            "  - name: main\n",
            "    type: CONFIGURATION\n",
            "    path: cfg\n",
            "  - name: addOn\n",
            "    type: EXTENSION\n",
            "    path: ext\n",
        ),
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let before = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n// untouched\r\n";
    let expected = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n\r\nProcedure Added()\r\nEndProcedure\r\n// untouched\r\n";
    let mut protected = Vec::new();
    for source_root in ["cfg", "ext"] {
        let source_root = root.join(source_root);
        let module = source_root.join("CommonModules/Shared/Ext/Module.bsl");
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(
                source_root.join("Configuration.xml"),
                r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
            )
            .unwrap();
        fs::write(
                source_root.join("CommonModules/Shared.xml"),
                b"\xef\xbb\xbf<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\"><CommonModule><Properties><Name>Shared</Name></Properties></CommonModule></MetaDataObject>\r\n",
            )
            .unwrap();
        fs::write(source_root.join("unrelated.bin"), [0, 1, 2, 255]).unwrap();
        fs::write(&module, before).unwrap();
        protected.push(source_root.join("Configuration.xml"));
        protected.push(source_root.join("CommonModules/Shared.xml"));
        protected.push(source_root.join("unrelated.bin"));
    }
    let protected_before = protected
        .iter()
        .map(|path| (path.clone(), fs::read(path).unwrap()))
        .collect::<Vec<_>>();

    for (source_set, source_root) in [("main", "cfg"), ("addOn", "ext")] {
        let module = root
            .join(source_root)
            .join("CommonModules/Shared/Ext/Module.bsl");
        let args = patch_args(
            source_set,
            "CommonModule.Shared.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), before);
        let preview_data = serde_json::to_value(preview.data.unwrap()).unwrap();
        assert_eq!(preview_data["sourceSet"], source_set);
        assert_eq!(preview_data["metadataPath"], "CommonModule.Shared.Module");
        assert_eq!(preview_data["targetKind"], "module");

        let applied = patch_inner(&args, &context, PatchMode::Apply);
        assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
        assert_eq!(fs::read(&module).unwrap(), expected);

        let repeated = patch_inner(&args, &context, PatchMode::Apply);
        assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
        assert!(repeated.data.unwrap().no_op);
        assert_eq!(fs::read(&module).unwrap(), expected);
    }
    for (path, bytes) in protected_before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn code_patch_data_has_an_exact_stable_serialization_contract() {
    let context = temp_context("typed-serialization");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = "Procedure Run()\nEndProcedure\n";
    let inserted = "Procedure Added()\nEndProcedure\n";
    let after = format!("{before}\n{inserted}");
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let preview = patch_inner(&args, &context, PatchMode::Preview);

    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    let serialized = serde_json::to_value(preview.data.unwrap()).unwrap();
    let pre_hash = hash(before.as_bytes());
    let post_hash = hash(after.as_bytes());
    let diff = concat!(
        "--- a/CommonModule.Sample.Module\n",
        "+++ b/CommonModule.Sample.Module\n",
        "@@ -1,2 +1,5 @@\n",
        " Procedure Run()\n",
        " EndProcedure\n",
        "+\n",
        "+Procedure Added()\n",
        "+EndProcedure\n",
    );
    assert_eq!(
        serialized,
        json!({
            "sourceSet": "main",
            "metadataPath": "CommonModule.Sample.Module",
            "targetKind": "module",
            "preHash": pre_hash,
            "postHash": post_hash.clone(),
            "noOp": false,
            "changedRanges": [{
                "startByte": before.len(),
                "endByte": after.len(),
                "startLine": 3,
                "startColumn": 1,
                "endLine": 6,
                "endColumn": 1
            }],
            "diff": diff,
            "affectedTarget": {
                "sourceSet": "main",
                "metadataPath": "CommonModule.Sample.Module",
                "targetKind": "module",
                "owner": "CommonModule.Sample",
                "moduleRole": "Module",
                "rawHash": post_hash.clone()
            },
            "validation": {
                "kind": "bsl-analyzer-parser",
                "status": "passed",
                "validatedPostHash": post_hash,
                "diagnostics": []
            }
        })
    );
    let serialized_text = serialized.to_string();
    assert!(!serialized_text.contains("src/CommonModules"));
    assert!(!serialized_text.contains("Module.bsl"));
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn anchor_before_preserves_indentation_byte_for_byte() {
    let context = temp_context("anchor-indentation");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(
        &module,
        "Procedure Run()\n    Message(\"old\");\nEndProcedure\n",
    )
    .unwrap();
    let args = patch_args_for_selector(
        "main",
        "CommonModule.Sample.Module",
        json!({"anchor": "Message(\"old\");"}),
        "before",
        "    Message(\"new\");",
    );

    let applied = patch_inner(&args, &context, PatchMode::Apply);

    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read_to_string(&module).unwrap(),
        "Procedure Run()\n    Message(\"new\");\n    Message(\"old\");\nEndProcedure\n"
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn closing_token_anchor_matches_crlf_and_repeats_as_noop() {
    let context = temp_context("closing-token-anchor");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = concat!(
        "Процедура Цель()\r\n",
        "\tЕсли Истина Тогда\r\n",
        "\tКонецЕсли;\r\n",
        "КонецПроцедуры\r\n",
        "// frame\r\n"
    );
    fs::write(&module, before).unwrap();
    let args = patch_args_for_selector(
        "main",
        "CommonModule.Sample.Module",
        json!({"anchor": "\tКонецЕсли;\nКонецПроцедуры\n"}),
        "after",
        "// inserted",
    );

    let applied = patch_inner(&args, &context, PatchMode::Apply);

    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read_to_string(&module).unwrap(),
        before.replacen(
            "КонецПроцедуры\r\n// frame",
            "КонецПроцедуры\r\n// inserted\r\n// frame",
            1
        )
    );
    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
    assert!(repeated.data.unwrap().no_op);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn patch_rejects_content_that_would_break_anchor_idempotence() {
    let before = b"Procedure Run()\n    Target();\nEndProcedure\n";
    let fixture = staged_code_fixture("unstable-anchor", before);
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "    Target();",
        Some(CodeSelector::Anchor("Target();".to_string())),
        Some(CodePosition::Before),
    )];

    let admission = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&admission, &fixture.binding, &operation)
        .expect_err("an insert that duplicates its anchor must be refused");

    assert_eq!(error.kind(), ApplyPlanErrorKind::Postcondition, "{error:?}");
    assert!(
        error
            .to_string()
            .contains("does not prove repeat-noop behavior"),
        "{error}"
    );
    assert_eq!(
        fs::read(fixture.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
        before
    );
    fixture.cleanup();
}

#[test]
fn patch_rejects_content_that_duplicates_the_selected_method() {
    let before = b"Procedure Run()\nEndProcedure\n";
    let fixture = staged_code_fixture("unstable-method", before);
    let operation = [staged_insert(
        "main:CommonModule.Sample.Method.Run",
        "Procedure Run()\nEndProcedure",
        Some(CodeSelector::Method("Run".to_string())),
        Some(CodePosition::After),
    )];

    let admission = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&admission, &fixture.binding, &operation)
        .expect_err("an insert that duplicates the addressed method must be refused");

    assert_eq!(error.kind(), ApplyPlanErrorKind::Postcondition, "{error:?}");
    assert!(
        error
            .to_string()
            .contains("does not prove repeat-noop behavior"),
        "{error}"
    );
    assert_eq!(
        fs::read(fixture.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
        before
    );
    fixture.cleanup();
}

#[test]
fn method_after_at_eof_inserts_a_separator_and_is_idempotent() {
    let context = temp_context("missing-terminal-eol");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Procedure Run()\nEndProcedure").unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read_to_string(&module).unwrap(),
        "Procedure Run()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n"
    );
    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok);
    assert!(repeated.data.unwrap().no_op);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

fn replace_args(metadata_path: &str, selector: Value, content: &str) -> Map<String, Value> {
    let mut args = Map::new();
    args.insert("sourceSet".to_string(), json!("main"));
    args.insert("metadataPath".to_string(), json!(metadata_path));
    args.insert("operation".to_string(), json!("replace"));
    args.insert("selector".to_string(), selector);
    args.insert("content".to_string(), json!(content));
    args
}

#[test]
fn code_patch_replaces_one_method_and_leaves_its_neighbours_untouched() {
    let context = temp_context("replace-method");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = "Процедура Первая()\r\nКонецПроцедуры\r\n\
                      Процедура Цель()\r\n\tСтарое = 1;\r\nКонецПроцедуры\r\n\
                      Процедура Третья()\r\nКонецПроцедуры\r\n";
    fs::write(&module, before).unwrap();
    let args = replace_args(
        "CommonModule.Sample.Module",
        json!({"method": "Цель"}),
        "Процедура Цель()\n\tНовое = 2;\nКонецПроцедуры",
    );

    let preview = patch_inner(&args, &context, PatchMode::Preview);
    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    assert_eq!(fs::read_to_string(&module).unwrap(), before);

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    let after = fs::read_to_string(&module).unwrap();
    assert_eq!(
        after,
        "Процедура Первая()\r\nКонецПроцедуры\r\n\
             Процедура Цель()\r\n\tНовое = 2;\r\nКонецПроцедуры\r\n\
             Процедура Третья()\r\nКонецПроцедуры\r\n",
        "only the selected method changes and the source CRLF survives"
    );

    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
    assert!(repeated.data.unwrap().no_op);
    assert_eq!(fs::read_to_string(&module).unwrap(), after);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_replace_that_consumes_its_selector_cannot_apply_twice() {
    let fixture = staged_code_fixture(
        "replace-consumes-selector",
        "Процедура Цель()\nКонецПроцедуры\n".as_bytes(),
    );
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    let operation = [staged_replace(
        "main:CommonModule.Sample.Method.Цель",
        "Процедура Переименована()\nКонецПроцедуры",
        None,
    )];

    let applied = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&applied, &fixture.binding, &operation).unwrap();
    fixture
        .actor
        .publish_prepared_apply(applied.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let after = fs::read_to_string(fixture.source.join(relative)).unwrap();
    assert_eq!(after, "Процедура Переименована()\nКонецПроцедуры\n");

    // The addressed method no longer resolves, so the repeat fails closed
    // rather than writing the edit a second time.
    let repeated = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&repeated, &fixture.binding, &operation)
        .expect_err("a consumed method address must not resolve again");
    assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
    assert_eq!(
        fs::read_to_string(fixture.source.join(relative)).unwrap(),
        after,
        "a refused repeat must leave the module byte-identical"
    );
    fixture.cleanup();
}

#[test]
fn code_patch_replace_keeps_an_anchor_edit_inline() {
    let context = temp_context("replace-anchor");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(
        &module,
        "Процедура Цель()\n\tЗначение = 1;\nКонецПроцедуры\n",
    )
    .unwrap();
    let args = replace_args(
        "CommonModule.Sample.Module",
        json!({"anchor": "Значение = 1;"}),
        "Значение = 42;",
    );

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read_to_string(&module).unwrap(),
        "Процедура Цель()\n\tЗначение = 42;\nКонецПроцедуры\n",
        "an inline anchor replacement must not add a line break"
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop() {
    let before = "Процедура Тест() КонецПроцедуры";
    let expected = "Процедура Тест() КонецПроцедуры\n\nПроцедура Добавлена() КонецПроцедуры\n";
    let fixture = staged_code_fixture("no-source-eol", before.as_bytes());
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Процедура Добавлена() КонецПроцедуры",
        Some(CodeSelector::Method("Тест".to_string())),
        Some(CodePosition::After),
    )];

    let preview = staged_code_admission(&fixture, true);
    let (mut state, effects) = plan_admitted_code(&preview, &fixture.binding, &operation).unwrap();
    assert_eq!(state.read(relative).unwrap().unwrap(), expected.as_bytes());
    fixture
        .actor
        .publish_prepared_apply(preview.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        before.as_bytes()
    );

    let applied = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&applied, &fixture.binding, &operation).unwrap();
    fixture
        .actor
        .publish_prepared_apply(applied.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        expected.as_bytes()
    );

    let repeated = staged_code_admission(&fixture, true);
    let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &operation).unwrap();
    assert!(state.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        expected.as_bytes()
    );
    fixture.cleanup();
}

#[test]
fn code_patch_preserves_two_leading_boms_for_preview_apply_and_repeat_noop() {
    let context = temp_context("two-leading-boms");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n";
    let expected =
        b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let preview = patch_inner(&args, &context, PatchMode::Preview);
    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    assert_eq!(fs::read(&module).unwrap(), before);

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(fs::read(&module).unwrap(), expected);

    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
    assert!(repeated.data.unwrap().no_op);
    assert_eq!(fs::read(&module).unwrap(), expected);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn mixed_eol_apply_preserves_untouched_bytes_and_uses_target_eol() {
    let before = b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n";
    let fixture = staged_code_fixture("mixed-eol", before);
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\r\nEndProcedure",
        Some(CodeSelector::Method("Second".to_string())),
        Some(CodePosition::After),
    )];

    let applied = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&applied, &fixture.binding, &operation).unwrap();
    fixture
        .actor
        .publish_prepared_apply(applied.prepare_with_effects(state, effects).unwrap())
        .unwrap();

    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n"
    );
    fixture.cleanup();
}

/// Модуль сам версии не задаёт; формат ему задают корень набора и описание
/// общего модуля. Устаревшая или неточная версия любого из них блокирует
/// канонический `code.insert` до записи.
#[test]
fn canonical_code_insert_inside_older_source_set_is_refused_without_writes() {
    for relative in ["Configuration.xml", "CommonModules/Sample.xml"] {
        for version in ["2.19", "2.20.0"] {
            let fixture = staged_code_fixture(
                &format!("older-owner-{}-{version}", relative.replace('/', "-")),
                b"Procedure Base()\nEndProcedure\n",
            );
            let owner = fixture.source.join(relative);
            let original = fs::read_to_string(&owner).unwrap();
            let older = original.replace("version=\"2.20\"", &format!("version=\"{version}\""));
            assert_ne!(
                older, original,
                "{relative}: fixture must declare its version"
            );
            fs::write(&owner, older).unwrap();
            let source_before = snapshot_staged_code_tree(&fixture.source);
            let cache = fixture.root.join(".build/unica");
            let cache_before = snapshot_staged_code_tree(&cache);
            let operation = [staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            )];

            for dry_run in [true, false] {
                let admission = staged_code_admission(&fixture, dry_run);
                let error = plan_admitted_code(&admission, &fixture.binding, &operation)
                    .expect_err("an older format owner must block the code insert");
                assert_eq!(
                    error.kind(),
                    ApplyPlanErrorKind::InvalidSource,
                    "{relative} version={version} dryRun={dry_run}: {error}"
                );
                // Причина — именно формат владельца, а не любая ошибка исходника.
                let message = error.to_string();
                assert!(
                    message.contains("serialization profile"),
                    "{relative} version={version} dryRun={dry_run}: {message}"
                );
            }
            assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
            assert_eq!(snapshot_staged_code_tree(&cache), cache_before);
            fixture.cleanup();
        }
    }
}

#[test]
fn object_and_manager_modules_report_owner_and_role() {
    let context = temp_context("module-roles");
    fs::create_dir_all(context.workspace_root.join("src/Catalogs")).unwrap();
    fs::write(
            context.workspace_root.join("src/Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    for role in ["ObjectModule", "ManagerModule"] {
        let relative = format!("src/Catalogs/Items/Ext/{role}.bsl");
        let module = context.workspace_root.join(&relative);
        fs::create_dir_all(module.parent().unwrap()).unwrap();
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let args = patch_args(
            "main",
            &format!("Catalog.Items.{role}"),
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);
        assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
        let target = preview.data.unwrap().affected_target;
        assert_eq!(target.owner, "Catalog.Items");
        assert_eq!(target.module_role, role);
    }
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn nested_form_and_command_modules_report_the_metadata_owner_and_role() {
    let context = temp_context("nested-module-roles");
    let src = context.workspace_root.join("src");
    fs::create_dir_all(src.join("Catalogs/Items/Forms/Main/Ext/Form")).unwrap();
    fs::create_dir_all(src.join("Catalogs/Items/Commands/Print/Ext")).unwrap();
    fs::write(
            src.join("Catalogs/Items.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Catalogs/Items/Forms/Main.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Form><Properties><Name>Main</Name></Properties></Form></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            src.join("Catalogs/Items/Commands/Print.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Command><Properties><Name>Print</Name></Properties></Command></MetaDataObject>"#,
        )
        .unwrap();
    let cases = [
        (
            "src/Catalogs/Items/Forms/Main/Ext/Form/Module.bsl",
            "Catalog.Items.Form.Main.FormModule",
            "FormModule",
        ),
        (
            "src/Catalogs/Items/Commands/Print/Ext/CommandModule.bsl",
            "Catalog.Items.Command.Print.CommandModule",
            "CommandModule",
        ),
    ];
    for (relative, metadata_path, role) in cases {
        let module = context.workspace_root.join(relative);
        fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
        let args = patch_args(
            "main",
            metadata_path,
            "Run",
            "Procedure Added()\nEndProcedure",
        );

        let preview = patch_inner(&args, &context, PatchMode::Preview);

        assert!(
            preview.outcome.ok,
            "{relative}: {:?}",
            preview.outcome.errors
        );
        let target = preview.data.unwrap().affected_target;
        assert_eq!(target.owner, "Catalog.Items");
        assert_eq!(target.module_role, role);
    }
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_requires_descriptor_evidence_and_a_proven_module_role() {
    let context = temp_context("target-contract");

    let missing_descriptor_module = context
        .workspace_root
        .join("src/CommonModules/Missing/Ext/Module.bsl");
    fs::create_dir_all(missing_descriptor_module.parent().unwrap()).unwrap();
    fs::write(
        &missing_descriptor_module,
        "Procedure Run()\nEndProcedure\n",
    )
    .unwrap();
    let missing_descriptor = patch_args(
        "main",
        "CommonModule.Missing.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let missing_result = patch_inner(&missing_descriptor, &context, PatchMode::Preview);
    assert!(!missing_result.outcome.ok);
    assert!(missing_result.outcome.errors[0].contains("metadata owner evidence"));

    fs::create_dir_all(context.workspace_root.join("src/Languages/Ru/Ext")).unwrap();
    fs::write(
        context.workspace_root.join("src/Languages/Ru.xml"),
        "<MetaDataObject/>",
    )
    .unwrap();
    fs::write(
        context
            .workspace_root
            .join("src/Languages/Ru/Ext/ManagerModule.bsl"),
        "Procedure Run()\nEndProcedure\n",
    )
    .unwrap();
    let unsupported_role = patch_args(
        "main",
        "Language.Ru.ManagerModule",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let unsupported_result = patch_inner(&unsupported_role, &context, PatchMode::Preview);
    assert!(!unsupported_result.outcome.ok);
    assert!(
        unsupported_result.outcome.errors[0].contains("supported canonical"),
        "{:?}",
        unsupported_result.outcome.errors
    );

    fs::remove_dir_all(&context.workspace_root).unwrap();
}

/// The resolver also answers metadata object addresses for the read-only
/// surface. `code.patch` must keep refusing them by declaration, so a
/// descriptor can never become a write target.
#[test]
fn code_patch_refuses_a_metadata_object_address() {
    let context = temp_context("object-address");
    let descriptor = context.workspace_root.join("src/Catalogs/Items.xml");
    fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
    fs::write(
            &descriptor,
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Catalog><Properties><Name>Items</Name></Properties></Catalog></MetaDataObject>"#,
        )
        .unwrap();
    let before = fs::read(&descriptor).unwrap();

    let result = patch_inner(
        &patch_args(
            "main",
            "Catalog.Items",
            "Run",
            "Procedure Added()\nEndProcedure",
        ),
        &context,
        PatchMode::Preview,
    );

    assert!(!result.outcome.ok);
    assert!(
        result.outcome.errors[0].contains("module terminal"),
        "{:?}",
        result.outcome.errors
    );
    assert_eq!(fs::read(&descriptor).unwrap(), before);

    fs::remove_dir_all(&context.workspace_root).unwrap();
}

/// The public writer refuses an absent address before the resolver is asked,
/// and the resolver's `ModuleOnly` policy refuses the source root that such
/// an address would name. Neither barrier lets a write plan see the root.
#[test]
fn code_patch_refuses_a_missing_or_empty_metadata_path_without_touching_the_workspace() {
    let context = temp_context("absent-address");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
    let module_before = fs::read(&module).unwrap();
    let root_before = fs::read(context.workspace_root.join("src/Configuration.xml")).unwrap();

    for (label, address) in [
        ("missing", None),
        ("empty", Some("")),
        ("blank", Some("  ")),
    ] {
        let mut args = patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Run",
            "Procedure Added()\nEndProcedure",
        );
        match address {
            Some(value) => {
                args.insert("metadataPath".to_string(), json!(value));
            }
            None => {
                args.remove("metadataPath");
            }
        }

        for mode in [PatchMode::Preview, PatchMode::Apply] {
            let result = patch_inner(&args, &context, mode);

            assert!(!result.outcome.ok, "{label} address must not be accepted");
            assert!(
                result.outcome.errors[0].contains("metadataPath must identify one existing module"),
                "{label}: {:?}",
                result.outcome.errors
            );
            assert!(
                result.data.is_none(),
                "{label} address must publish no plan"
            );
        }
    }

    assert_eq!(fs::read(&module).unwrap(), module_before);
    assert_eq!(
        fs::read(context.workspace_root.join("src/Configuration.xml")).unwrap(),
        root_before
    );

    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_symlink_module_is_rejected_during_preview() {
    let context = temp_context("symlink-target");
    let real = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/RealModule.bsl");
    let target = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(real.parent().unwrap()).unwrap();
    fs::write(&real, "Procedure Run()\nEndProcedure\n").unwrap();
    let outcome = create_file_link_fixture_for_test(&real, &target)
        .expect("unexpected file-link creation error must fail the fixture test");
    match outcome {
        FileLinkFixtureOutcome::Created => {}
        FileLinkFixtureOutcome::Unsupported => {
            fs::remove_dir_all(&context.workspace_root).unwrap();
            return;
        }
        FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => {
            fs::remove_dir_all(&context.workspace_root).unwrap();
            return;
        }
    }
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let result = patch_inner(&args, &context, PatchMode::Preview);

    assert!(!result.outcome.ok);
    assert!(result.outcome.errors[0].contains("containment"));
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_invalid_postimage_returns_failed_validation_without_writing() {
    let context = temp_context("validation-failure");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"Procedure Run()\n    Message(\"ok\");\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    let args = patch_args_for_selector(
        "main",
        "CommonModule.Sample.Module",
        json!({"anchor": "Message(\"ok\");"}),
        "after",
        "    If True Then",
    );

    let result = patch_inner(&args, &context, PatchMode::Apply);

    assert!(!result.outcome.ok);
    assert_eq!(fs::read(&module).unwrap(), before);
    let data = result.data.unwrap();
    let serialized = serde_json::to_value(&data).unwrap();
    let validation = data.validation;
    assert_eq!(validation.status, ValidationStatus::Failed);
    assert!(!validation.diagnostics.is_empty());
    let diagnostic = serialized["validation"]["diagnostics"][0]
        .as_object()
        .unwrap();
    assert_eq!(
        diagnostic.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "code",
            "message",
            "startByte",
            "endByte",
            "startLine",
            "startColumn",
            "endLine",
            "endColumn"
        ]
    );
    assert_eq!(serialized["validation"]["status"], "failed");
    assert_eq!(serialized["validation"]["kind"], "bsl-analyzer-parser");
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
pub(crate) fn code_patch_without_a_selector_appends_to_the_end_and_proves_the_repeat() {
    let before = "Procedure Run()\nEndProcedure\n";
    let expected = "Procedure Run()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n";
    let fixture = staged_code_fixture("tail-append", before.as_bytes());
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];

    let preview = staged_code_admission(&fixture, true);
    let (mut state, effects) = plan_admitted_code(&preview, &fixture.binding, &operation).unwrap();
    assert_eq!(state.read(relative).unwrap().unwrap(), expected.as_bytes());
    fixture
        .actor
        .publish_prepared_apply(preview.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        before.as_bytes()
    );

    let applied = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&applied, &fixture.binding, &operation).unwrap();
    fixture
        .actor
        .publish_prepared_apply(applied.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        expected.as_bytes()
    );

    // The end of the module is still addressable afterwards, so the repeat
    // is a proven no-op rather than a refusal after the write landed.
    let repeated = staged_code_admission(&fixture, true);
    let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &operation).unwrap();
    assert!(state.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        expected.as_bytes()
    );
    fixture.cleanup();
}

#[test]
pub(crate) fn code_patch_creates_a_module_file_the_platform_never_exported() {
    let context = temp_context("tail-absent");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    assert!(!module.exists());
    let args = tail_args(
        "main",
        "CommonModule.Sample.Module",
        "Procedure Run()\nEndProcedure",
    );

    // Preview stays read-only: an absent file is not created to be shown.
    let preview = patch_inner(&args, &context, PatchMode::Preview);
    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    assert!(!module.exists(), "preview created the module file");
    let data = preview.data.unwrap();
    assert_eq!(data.pre_hash, hash(b""));
    assert_eq!(data.validation.status, ValidationStatus::Passed);

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read(&module).unwrap(),
        b"Procedure Run()\nEndProcedure\n"
    );

    // Once materialised the module is ordinary, so the repeat is the same
    // proven no-op as for a module that was exported all along.
    let repeat = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeat.outcome.ok, "{:?}", repeat.outcome.errors);
    assert_eq!(
        fs::read(&module).unwrap(),
        b"Procedure Run()\nEndProcedure\n"
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
pub(crate) fn code_patch_refuses_a_module_role_the_metadata_kind_never_owns() {
    let context = temp_context("tail-absent-role");
    // A common module owns `Module`; it never owns an object module, so the
    // absent file is not an omitted empty one and must stay unaddressable.
    let args = tail_args(
        "main",
        "CommonModule.Sample.ObjectModule",
        "Procedure Run()\nEndProcedure",
    );
    let refused = patch_inner(&args, &context, PatchMode::Apply);
    assert!(!refused.outcome.ok);
    assert!(!context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/ObjectModule.bsl")
        .exists());
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
pub(crate) fn code_patch_writes_the_first_body_of_an_empty_or_bom_only_module() {
    for (label, before, expected) in [
        (
            "bom-only",
            b"\xef\xbb\xbf".to_vec(),
            b"\xef\xbb\xbfProcedure Run()\nEndProcedure\n".to_vec(),
        ),
        (
            "byte-empty",
            Vec::new(),
            b"Procedure Run()\nEndProcedure\n".to_vec(),
        ),
    ] {
        let fixture = staged_code_fixture(&format!("tail-first-{label}"), &before);
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Run()\nEndProcedure",
            None,
            None,
        )];

        let preview = staged_code_admission(&fixture, true);
        let (mut state, effects) =
            plan_admitted_code(&preview, &fixture.binding, &operation).unwrap();
        assert_eq!(state.read(relative).unwrap().unwrap(), expected, "{label}");
        fixture
            .actor
            .publish_prepared_apply(preview.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            before,
            "{label} preview wrote"
        );

        let applied = staged_code_admission(&fixture, false);
        let (state, effects) = plan_admitted_code(&applied, &fixture.binding, &operation).unwrap();
        fixture
            .actor
            .publish_prepared_apply(applied.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        // No blank line is owed: a byte order mark is not content.
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            expected,
            "{label}"
        );

        let repeated = staged_code_admission(&fixture, true);
        let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &operation).unwrap();
        assert!(state.planned_changes().is_empty(), "{label}");
        assert!(effects.events().is_empty(), "{label}");
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            expected,
            "{label} repeat wrote"
        );
        fixture.cleanup();
    }
}

#[test]
fn code_patch_separates_an_appended_method_from_a_module_without_a_trailing_eol() {
    let context = temp_context("tail-separator");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Procedure Run()\nEndProcedure").unwrap();
    let args = tail_args(
        "main",
        "CommonModule.Sample.Module",
        "Procedure Added()\nEndProcedure",
    );

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    assert_eq!(
        fs::read(&module).unwrap(),
        b"Procedure Run()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n"
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

/// #420. The writer must not be the source of a dirty diff: content that
/// carries no trailing whitespace must not gain any on the way in, and the
/// separator the writer adds itself is a bare EOL, never an indented blank
/// line. What the caller wrote is preserved verbatim — the writer neither
/// adds whitespace nor silently edits the body it was given.
#[test]
fn code_patch_insert_adds_no_trailing_whitespace_of_its_own() {
    let context = temp_context("insert-trailing-whitespace");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(
        &module,
        "Процедура Первая()\r\nКонецПроцедуры\r\n\
             Процедура Цель()\r\n\tСтарое = 1;\r\nКонецПроцедуры\r\n",
    )
    .unwrap();
    let mut args = tail_args(
        "main",
        "CommonModule.Sample.Module",
        "Процедура Помощник()\n\tЗначение = 1;\n\n\tВозврат Значение;\nКонецПроцедуры",
    );
    args.insert("selector".to_string(), json!({"method": "Цель"}));
    args.insert("position".to_string(), json!("before"));

    let applied = patch_inner(&args, &context, PatchMode::Apply);

    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    let after = fs::read_to_string(&module).unwrap();
    let dirty = after
        .split("\r\n")
        .enumerate()
        .filter(|(_, line)| line.len() != line.trim_end().len())
        .collect::<Vec<_>>();
    assert!(dirty.is_empty(), "git diff --check would flag {dirty:?}");
    assert!(after.contains("\tВозврат Значение;"), "{after}");
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

fn tail_args(source_set: &str, metadata_path: &str, content: &str) -> Map<String, Value> {
    Map::from_iter([
        ("sourceSet".to_string(), json!(source_set)),
        ("metadataPath".to_string(), json!(metadata_path)),
        ("operation".to_string(), json!("insert")),
        ("content".to_string(), json!(content)),
    ])
}

#[test]
fn code_patch_dry_run_reports_the_same_postimage_without_writing() {
    let context = temp_context("dry-run");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"Procedure Run()\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );

    let preview = patch_inner(&args, &context, PatchMode::Preview);

    assert!(preview.outcome.ok, "{:?}", preview.outcome.errors);
    assert!(preview.outcome.changes.is_empty());
    assert!(preview.outcome.stdout.is_none());
    let data = preview.data.unwrap();
    assert_ne!(data.pre_hash, data.post_hash);
    assert_eq!(data.validation.status, ValidationStatus::Passed);
    assert_eq!(fs::read(&module).unwrap(), before);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_refuses_a_stale_preimage_without_overwriting_concurrent_change() {
    let context = temp_context("stale-preimage");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Procedure Run()\nEndProcedure\n").unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let replacement = "Procedure Run()\n    Message(\"concurrent\");\nEndProcedure\n";

    let result = with_before_commit_hook(
        move |path| fs::write(path, replacement).unwrap(),
        || patch_inner(&args, &context, PatchMode::Apply),
    );

    assert!(!result.outcome.ok);
    assert!(result.outcome.errors[0].contains("publish BSL module"));
    assert_eq!(fs::read_to_string(&module).unwrap(), replacement);
    assert_eq!(
        result.data.unwrap().validation.status,
        ValidationStatus::Passed
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_rolls_back_if_v8project_changes_the_owner_source_set_before_commit() {
    let context = temp_context("source-map-race");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"Procedure Run()\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    fs::write(
            context.workspace_root.join("src/CommonModules.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.21"><ExternalDataProcessor/></MetaDataObject>"#,
        )
        .unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let v8project = context.workspace_root.join("v8project.yaml");
    let concurrent_source_map = "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n";
    let v8project_for_hook = v8project.clone();

    let result = with_before_commit_hook(
        move |_| fs::write(&v8project_for_hook, concurrent_source_map).unwrap(),
        || patch_inner(&args, &context, PatchMode::Apply),
    );

    assert!(!result.outcome.ok);
    let error = result.outcome.errors.join("\n");
    assert!(error.contains("read guard"), "{error}");
    assert!(error.contains("v8project.yaml"), "{error}");
    assert_eq!(fs::read(&module).unwrap(), before);
    assert_eq!(
        fs::read_to_string(&v8project).unwrap(),
        concurrent_source_map
    );
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
pub(crate) fn code_patch_rolls_back_if_owner_descriptor_changes_before_commit() {
    let context = temp_context("owner-descriptor-race");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    let before = b"Procedure Run()\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let descriptor = context.workspace_root.join("src/CommonModules/Sample.xml");
    let replacement = "<MetaDataObject concurrent=\"true\"/>";
    let descriptor_for_hook = descriptor.clone();

    let result = with_before_commit_hook(
        move |_| fs::write(&descriptor_for_hook, replacement).unwrap(),
        || patch_inner(&args, &context, PatchMode::Apply),
    );

    assert!(!result.outcome.ok);
    assert!(
        result.outcome.errors.join("\n").contains("read guard"),
        "{:?}",
        result.outcome.errors
    );
    assert_eq!(fs::read(&module).unwrap(), before);
    assert_eq!(fs::read_to_string(&descriptor).unwrap(), replacement);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn code_patch_rolls_back_if_absent_support_state_appears_before_commit() {
    let context = temp_context("support-state-appearance-race");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::create_dir_all(context.workspace_root.join("src/Ext")).unwrap();
    fs::write(
        context.workspace_root.join("src/Configuration.xml"),
        concat!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
            "<Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\">",
            "<Properties><Name>Main</Name></Properties>",
            "</Configuration>",
            "</MetaDataObject>"
        ),
    )
    .unwrap();
    fs::write(
        context.workspace_root.join("src/CommonModules/Sample.xml"),
        concat!(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">",
            "<CommonModule uuid=\"bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb\">",
            "<Properties><Name>Sample</Name></Properties>",
            "</CommonModule>",
            "</MetaDataObject>"
        ),
    )
    .unwrap();
    let before = b"Procedure Run()\nEndProcedure\n";
    fs::write(&module, before).unwrap();
    let args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Run",
        "Procedure Added()\nEndProcedure",
    );
    let support_path = context
        .workspace_root
        .join("src/Ext/ParentConfigurations.bin");
    let concurrent_support = concat!(
        "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
        "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
        "cccccccc-cccc-cccc-cccc-cccccccccccc,",
        "cccccccc-cccc-cccc-cccc-cccccccccccc}"
    )
    .as_bytes()
    .to_vec();
    let support_path_for_hook = support_path.clone();
    let concurrent_support_for_hook = concurrent_support.clone();

    let result = with_before_commit_hook(
        move |_| fs::write(&support_path_for_hook, &concurrent_support_for_hook).unwrap(),
        || patch_inner(&args, &context, PatchMode::Apply),
    );

    assert!(!result.outcome.ok);
    assert!(
        result.outcome.errors.join("\n").contains("absence guard"),
        "{:?}",
        result.outcome.errors
    );
    assert_eq!(fs::read(&module).unwrap(), before);
    assert_eq!(fs::read(&support_path).unwrap(), concurrent_support);
    let violation = support_guard_violation(&module, SupportGuardRequirement::Editable).unwrap();
    assert_eq!(violation.code, "locked");
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

#[test]
fn inserted_method_spacing_covers_positions_eol_eof_and_repeat() {
    for eol in ["\n", "\r\n"] {
        for terminal in ["", eol] {
            for separator in [eol, &eol.repeat(2), &format!("{eol} \t{eol}")] {
                let before = format!("Procedure First(){eol}EndProcedure{separator}// Second docs.{eol}&AtServer{eol}Procedure Second(){eol}EndProcedure{terminal}");
                for (selector, position, expected) in [
                    (Some("First"), Some(CodePosition::Before), format!("Procedure Added(){eol}EndProcedure{eol}{eol}{before}")),
                    (Some("First"), Some(CodePosition::After), format!("Procedure First(){eol}EndProcedure{eol}{eol}Procedure Added(){eol}EndProcedure{separator}{}// Second docs.{eol}&AtServer{eol}Procedure Second(){eol}EndProcedure{terminal}", if separator == eol { eol } else { "" })),
                    (Some("Second"), Some(CodePosition::Before), format!("Procedure First(){eol}EndProcedure{separator}{}Procedure Added(){eol}EndProcedure{eol}{eol}// Second docs.{eol}&AtServer{eol}Procedure Second(){eol}EndProcedure{terminal}", if separator == eol { eol } else { "" })),
                    (None, None, format!("{before}{}Procedure Added(){eol}EndProcedure{eol}", if terminal.is_empty() { eol.repeat(2) } else { eol.to_string() })),
                ] {
                    let operation = staged_insert("main:CommonModule.Sample", "Procedure Added()\nEndProcedure", selector.map(|name| CodeSelector::Method(name.to_string())), position);
                    let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
                    assert_eq!(post.after, expected.as_bytes(), "{eol:?} {terminal:?} {separator:?} {selector:?} {position:?}");
                    let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
                    assert!(repeated.no_op);
                    assert_eq!(repeated.after, post.after);
                }
            }
        }
    }
}

/// Модуль, где метод уже стоит вплотную к соседу (написан вручную или прежней
/// версией вставки), повтор той же вставки не дублирует: проверка присутствия
/// принимает и текст без добавленных разделителей.
#[test]
fn inserted_method_spacing_recognizes_an_existing_method_without_separators() {
    for eol in ["\n", "\r\n"] {
        let added = format!("Procedure Added(){eol}EndProcedure{eol}");
        let first = format!("Procedure First(){eol}EndProcedure{eol}");
        let second = format!("Procedure Second(){eol}EndProcedure{eol}");
        for (before, selector, position) in [
            (
                format!("{first}{added}{second}"),
                "Second",
                CodePosition::Before,
            ),
            (
                format!("{first}{added}{second}"),
                "First",
                CodePosition::After,
            ),
            (format!("{added}{first}"), "First", CodePosition::Before),
            (format!("{first}{added}"), "First", CodePosition::After),
        ] {
            let operation = staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                Some(CodeSelector::Method(selector.to_string())),
                Some(position),
            );
            let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
            assert!(post.no_op, "{eol:?} {selector} {position:?}");
            assert_eq!(
                post.after,
                before.as_bytes(),
                "{eol:?} {selector} {position:?}"
            );
        }
    }
}

#[test]
fn inserted_method_spacing_preserves_empty_bom_payload_and_mixed_bytes() {
    for (before, content, selector, expected) in [
        ("", "Procedure Added()\nEndProcedure", None, "Procedure Added()\nEndProcedure\n"),
        ("\u{feff}", "Procedure Added()\nEndProcedure", None, "\u{feff}Procedure Added()\nEndProcedure\n"),
        ("Procedure First()\nEndProcedure", "\nProcedure Added()\nEndProcedure\n\n", None, "Procedure First()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n\n"),
        ("Procedure First()\nEndProcedure\n\n", "Procedure Added()\nEndProcedure\n\n", None, "Procedure First()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n\n"),
        ("\u{feff}Procedure First()\r\nEndProcedure\r\n// Second.\n&AtServer\nProcedure Second()\nEndProcedure", "// Added.\n&AtServer\nProcedure Added()\nEndProcedure", Some("First"), "\u{feff}Procedure First()\r\nEndProcedure\r\n\r\n// Added.\r\n&AtServer\r\nProcedure Added()\r\nEndProcedure\r\n\r\n// Second.\n&AtServer\nProcedure Second()\nEndProcedure"),
    ] {
        let operation = staged_insert("main:CommonModule.Sample", content,
            selector.map(|name| CodeSelector::Method(name.to_string())), selector.map(|_| CodePosition::After));
        let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
        assert_eq!(std::str::from_utf8(&post.after).unwrap(), expected);
        let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
        assert!(repeated.no_op);
        assert_eq!(repeated.after, post.after);
    }
}

#[test]
fn staged_code_insert_then_replace_reads_prior_postimage() {
    let fixture = staged_code_fixture("insert-then-replace", b"Procedure Base()\nEndProcedure\n");
    let operations = vec![
        staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            Some(CodeSelector::Method("Base".to_string())),
            Some(CodePosition::After),
        ),
        staged_replace(
            "main:CommonModule.Sample.Method.Added",
            "Procedure Final()\nEndProcedure",
            None,
        ),
    ];
    let admitted = staged_code_admission(&fixture, true);
    let (mut state, effects) = plan_admitted_code(&admitted, &fixture.binding, &operations)
        .expect("placeholder cannot compose the staged insert and replace postimages");

    assert_eq!(
        state
            .read(Path::new("CommonModules/Sample/Ext/Module.bsl"))
            .unwrap()
            .unwrap(),
        b"Procedure Base()\nEndProcedure\n\nProcedure Final()\nEndProcedure\n"
    );
    assert_eq!(
        effects.events(),
        &[DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:CommonModule.Sample",
        )]
    );

    let reverse = vec![operations[1].clone(), operations[0].clone()];
    let reverse_admission = staged_code_admission(&fixture, true);
    let error = plan_admitted_code(&reverse_admission, &fixture.binding, &reverse)
        .expect_err("reverse order unexpectedly resolved an unstaged method");
    assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
    fixture.cleanup();
}

#[test]
fn staged_code_poisoned_second_op_publishes_nothing() {
    for dry_run in [true, false] {
        let fixture = staged_code_fixture(
            if dry_run { "poison-dry" } else { "poison-real" },
            b"Procedure Base()\nEndProcedure\n",
        );
        let source_before = snapshot_staged_code_tree(&fixture.source);
        let cache = fixture.root.join(".build/unica");
        let cache_before = snapshot_staged_code_tree(&cache);
        let admitted = fixture
            .actor
            .admit_apply(
                &fixture.binding,
                None,
                dry_run,
                unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                    Duration::from_secs(5),
                ),
                &unica_work_control::cancellation::CancellationToken::new(),
            )
            .unwrap();
        let operations = vec![
            staged_insert(
                "main:CommonModule.Sample",
                "Procedure Added()\nEndProcedure",
                None,
                None,
            ),
            staged_replace(
                "main:CommonModule.Sample.Method.Missing",
                "Procedure Never()\nEndProcedure",
                None,
            ),
        ];

        let error = plan_admitted_code(&admitted, &fixture.binding, &operations)
            .expect_err("poisoned staged code batch unexpectedly returned effects");

        assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
        assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
        assert_eq!(snapshot_staged_code_tree(&cache), cache_before);
        assert!(!cache.join("state.json").exists());
        fixture.cleanup();
    }
}

#[test]
fn staged_code_dry_run_and_real_apply_share_postimages_and_effect_receipt() {
    let dry = staged_code_fixture("dry-real-dry", b"Procedure Base()\nEndProcedure\n");
    let real = staged_code_fixture("dry-real-real", b"Procedure Base()\nEndProcedure\n");
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    let dry_admission = staged_code_admission(&dry, true);
    let real_admission = staged_code_admission(&real, false);
    let (mut dry_state, dry_effects) = plan_admitted_code(&dry_admission, &dry.binding, &operation)
        .expect("dry staged code planning is unavailable");
    let (mut real_state, real_effects) =
        plan_admitted_code(&real_admission, &real.binding, &operation)
            .expect("real staged code planning is unavailable");
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    let expected_postimage = dry_state.read(relative).unwrap().unwrap();
    assert_eq!(
        Some(expected_postimage.clone()),
        real_state.read(relative).unwrap()
    );
    assert_eq!(dry_effects.events(), real_effects.events());
    let dry_result = dry
        .actor
        .publish_prepared_apply(
            dry_admission
                .prepare_with_effects(dry_state, dry_effects)
                .unwrap(),
        )
        .unwrap();
    let real_result = real
        .actor
        .publish_prepared_apply(
            real_admission
                .prepare_with_effects(real_state, real_effects)
                .unwrap(),
        )
        .unwrap();

    assert_eq!(
        dry_result.effects().disposition(),
        ApplyEffectDisposition::Projected
    );
    assert_eq!(dry_result.commit_count_for_test(), 0);
    assert_eq!(
        fs::read(dry.source.join(relative)).unwrap(),
        b"Procedure Base()\nEndProcedure\n"
    );
    assert_eq!(
        real_result.effects().disposition(),
        ApplyEffectDisposition::Committed
    );
    assert_eq!(real_result.commit_count_for_test(), 1);
    assert_eq!(
        fs::read(real.source.join(relative)).unwrap(),
        expected_postimage
    );
    assert_eq!(
        dry_result.effects().events(),
        real_result.effects().events()
    );
    dry.cleanup();
    real.cleanup();
}

#[test]
fn staged_code_reuses_actor_revision_and_race_fences() {
    use unica_workspace::actor::ApplyPublicationErrorKind;

    let fixture = staged_code_fixture("actor-fences", b"Procedure Base()\nEndProcedure\n");
    let observed = staged_code_admission(&fixture, true);
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];

    let (state, effects) = plan_admitted_code(&observed, &fixture.binding, &operation).unwrap();
    let preview = fixture
        .actor
        .publish_prepared_apply(observed.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let rev = preview.rev();
    let exact = fixture
        .actor
        .admit_apply(
            &fixture.binding,
            Some(rev),
            true,
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                Duration::from_secs(5),
            ),
            &unica_work_control::cancellation::CancellationToken::new(),
        )
        .unwrap();

    let (state, effects) = plan_admitted_code(&exact, &fixture.binding, &operation)
        .expect("planner did not reproduce the previewed plan");
    let prepared = exact.prepare_with_effects(state, effects).unwrap();

    let stale = fixture.actor.admit_apply(
        &fixture.binding,
        Some("sha256:999:stale"),
        true,
        unica_work_control::provider_deadline::ProviderDeadline::from_budget(Duration::from_secs(
            5,
        )),
        &unica_work_control::cancellation::CancellationToken::new(),
    );
    let stale = stale.unwrap();
    let (state, effects) = plan_admitted_code(&stale, &fixture.binding, &operation).unwrap();
    let error = stale.prepare_with_effects(state, effects).unwrap_err();
    assert_eq!(
        error.kind(),
        unica_source_contract::proposal::ApplyStagingErrorKind::ConcurrentRevision
    );
    assert!(error.to_string().contains("ifRev is stale"));

    fs::write(
        fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
        b"Procedure Foreign()\nEndProcedure\n",
    )
    .unwrap();
    let raced = fixture.actor.publish_prepared_apply(prepared).unwrap_err();
    assert_eq!(raced.kind(), ApplyPublicationErrorKind::ConcurrentRevision);

    let cancelled = unica_work_control::cancellation::CancellationToken::new();
    cancelled.cancel();
    assert!(fixture
        .actor
        .admit_apply(
            &fixture.binding,
            None,
            true,
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                Duration::from_secs(5),
            ),
            &cancelled,
        )
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
    assert!(fixture
        .actor
        .admit_apply(
            &fixture.binding,
            None,
            true,
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(Duration::ZERO),
            &unica_work_control::cancellation::CancellationToken::new(),
        )
        .unwrap_err()
        .to_string()
        .contains("deadline"));
    assert!(!fixture.root.join(".build/unica/state.json").exists());
    fixture.cleanup();
}

#[test]
fn staged_addressed_method_body_preserves_signature_neighbors_and_repeat() {
    let before = concat!(
        "\u{feff}// module\r\nProcedure Base(\r\n    Value\r\n) Export // signature\r\n",
        "    // old body\r\n    Value = 1;\r\n  EndProcedure\r\n",
        "\r\nFunction Neighbor() Export\r\n    Return 7;\r\nEndFunction\r\n"
    );
    let fixture = staged_code_fixture("addressed-body", before.as_bytes());
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at":"main:CommonModule.Sample.Method.Base.Body", "text":"    Value = 2;"}),
        0,
        fixture.binding.source_set_name(),
    )
    .expect("documented addressed replacement must parse without selector");
    let admission = staged_code_admission(&fixture, false);
    let (mut state, effects) = plan_admitted_code(
        &admission,
        &fixture.binding,
        std::slice::from_ref(&operation),
    )
    .unwrap();
    let expected = before.replace(
        "    // old body\r\n    Value = 1;\r\n",
        "    Value = 2;\r\n",
    );
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    assert_eq!(state.read(relative).unwrap().unwrap(), expected.as_bytes());
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        before.as_bytes()
    );
    assert_eq!(
        effects.events(),
        &[DomainEvent::new(
            DomainEventKind::ModuleChanged,
            "main:CommonModule.Sample"
        )]
    );
    fixture
        .actor
        .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let repeated = staged_code_admission(&fixture, true);
    let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &[operation]).unwrap();
    assert!(state.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    assert_eq!(
        fs::read(fixture.source.join(relative)).unwrap(),
        expected.as_bytes()
    );
    fixture.cleanup();
}

#[test]
fn method_body_text_read_by_view_replaces_the_same_span_and_repeats_as_noop() {
    // `view …Method.Base.Body` отдаёт комментарий, директиву, объявление,
    // тело и окончание; тот же текст с правкой заменяет ровно этот участок.
    let before = concat!(
        "// Отдельный комментарий модуля.\r\n",
        "\r\n",
        "// Документирует Base.\r\n",
        "&НаСервере\r\n",
        "Procedure Base() Export\r\n",
        "    Value = 1;\r\n",
        "EndProcedure\r\n",
        "\r\n",
        "Function Neighbor() Export\r\n    Return 7;\r\nEndFunction\r\n"
    );
    let read = "// Документирует Base.\r\n&НаСервере\r\nProcedure Base() Export\r\n    Value = 1;\r\nEndProcedure\r\n";
    assert!(before.contains(read));
    let edited = read.replace("Value = 1;", "Value = 2;");
    let fixture = staged_code_fixture("view-roundtrip", before.as_bytes());
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at": "main:CommonModule.Sample.Method.Base.Body", "text": edited}),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let (mut state, effects) = plan_admitted_code(
        &admission,
        &fixture.binding,
        std::slice::from_ref(&operation),
    )
    .unwrap();
    let expected = before.replace(read, &edited);
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    assert_eq!(state.read(relative).unwrap().unwrap(), expected.as_bytes());
    fixture
        .actor
        .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let repeated = staged_code_admission(&fixture, true);
    let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &[operation]).unwrap();
    assert!(state.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    fixture.cleanup();
}

#[test]
fn framed_method_body_text_replaces_its_closing_comment_and_repeats_as_noop() {
    for (open, statement, close) in [
        ("Procedure Base()", "Value = 1;", "EndProcedure"),
        ("Function Base()", "Return 1;", "EndFunction"),
    ] {
        for eol in ["\n", "\r\n"] {
            for (terminal_eol, neighbor) in [
                ("", ""),
                ("\n", ""),
                ("\n", "\nProcedure Neighbor()\nEndProcedure\n"),
            ] {
                let read = format!(
                        "//+ Synthetic frame\n&AtServer\n{open}\n    {statement}\n{close} //- Synthetic frame{terminal_eol}"
                    ).replace('\n', eol);
                let neighbor = neighbor.replace('\n', eol);
                let before = format!("{read}{neighbor}");
                let edited = read.replace("1;", "2;");
                let operation = CodePlanOperation::Replace(CodeReplaceArgs {
                    at: unica_address::address::QualifiedAddress::parse(
                        "main:CommonModule.Sample.Method.Base.Body",
                    )
                    .unwrap(),
                    anchor: None,
                    text: edited.clone(),
                });
                let post = plan_code_postimage(before.as_bytes(), &operation, 0)
                    .expect("view/read-edit-replace preserves the frame without doubling its tail");
                assert_eq!(post.after, before.replace("1;", "2;").as_bytes());
                let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
                assert!(repeated.no_op);
                assert_eq!(repeated.after, post.after);
            }
        }
    }
}

#[test]
fn method_body_replacement_keeps_executable_tail_on_its_closing_line() {
    for eol in ["", "\n", "\r\n"] {
        let before =
            format!("Procedure Base() EndProcedure Procedure Neighbor() EndProcedure{eol}");
        let operation = CodePlanOperation::Replace(CodeReplaceArgs {
            at: unica_address::address::QualifiedAddress::parse(
                "main:CommonModule.Sample.Method.Base.Body",
            )
            .unwrap(),
            anchor: None,
            text: "Procedure Base() Value = 2; EndProcedure".to_owned(),
        });
        let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
        assert_eq!(
            post.after,
            format!(
                "Procedure Base() Value = 2; EndProcedure Procedure Neighbor() EndProcedure{eol}"
            )
            .as_bytes(),
            "a Body replacement must not consume the next method on the same line"
        );
        let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
        assert!(repeated.no_op);
        assert_eq!(repeated.after, post.after);
    }
}

#[test]
fn method_text_that_does_not_parse_is_refused_as_the_input_text() {
    let before = "Procedure Base()\n    Value = 1;\nEndProcedure\n";
    let fixture = staged_code_fixture("broken-method-text", before.as_bytes());
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({
            "at": "main:CommonModule.Sample.Method.Base.Body",
            "text": "// Комментарий.\nProcedure Base(\n    Value = 2;\nEndProcedure\n"
        }),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&admission, &fixture.binding, &[operation])
        .expect_err("a broken method text must not reach the module");
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert!(
        error.to_string().contains("text declares a method"),
        "{error}"
    );
    assert!(!error.to_string().contains("patched BSL module"), "{error}");
    fixture.cleanup();
}

#[test]
fn method_body_text_with_a_preprocessor_wrapper_is_refused_in_wrapped_and_plain_modules() {
    for (before, name) in [
        ("Procedure Base()\n    Value = 1;\nEndProcedure\n", "plain"),
        (
            "#Если Сервер Тогда\nProcedure Base()\n    Value = 1;\nEndProcedure\n#КонецЕсли\n",
            "wrapped",
        ),
    ] {
        let fixture = staged_code_fixture(&format!("wrapper-{name}"), before.as_bytes());
        for text in [
                "#Если Сервер Тогда\nProcedure Base()\n    Value = 2;\nEndProcedure\n#КонецЕсли\n",
                "#Область Основная\n// Комментарий.\nProcedure Base()\n    Value = 2;\nEndProcedure\n#КонецОбласти\n",
            ] {
                let operation = parse_code_plan_operation(
                    "code.replace",
                    &json!({"at": "main:CommonModule.Sample.Method.Base.Body", "text": text}),
                    0,
                    fixture.binding.source_set_name(),
                )
                .unwrap();
                let admission = staged_code_admission(&fixture, false);
                let error = plan_admitted_code(&admission, &fixture.binding, &[operation])
                    .expect_err("a wrapper is not part of the method text");
                assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{name}");
                assert!(error.to_string().contains("preprocessor"), "{name}: {error}");
            }
        // Условие внутри тела метода — обычное тело, а не обёртка.
        let operation = parse_code_plan_operation(
            "code.replace",
            &json!({
                "at": "main:CommonModule.Sample.Method.Base.Body",
                "text": "#Если Сервер Тогда\n    Value = 2;\n#КонецЕсли"
            }),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap();
        let admission = staged_code_admission(&fixture, false);
        assert!(
            plan_admitted_code(&admission, &fixture.binding, &[operation]).is_ok(),
            "{name}"
        );
        fixture.cleanup();
    }
}

#[test]
fn a_comment_inserted_before_a_method_joins_its_head_and_repeats_as_noop() {
    let fixture = staged_code_fixture("comment-head", b"Procedure Base()\nEndProcedure\n");
    let before = "Procedure Other()\nEndProcedure\n\n// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n";
    for (text, expected) in [
            (
                "// TODO\n",
                "Procedure Other()\nEndProcedure\n\n// TODO\n// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n",
            ),
            (
                "Procedure Added()\nEndProcedure\n// Описание Target.\n",
                "Procedure Other()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n\n// Описание Target.\n// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n",
            ),
        ] {
            let operation = parse_code_plan_operation(
                "code.insert",
                &json!({"at": "main:CommonModule.Sample.Method.Target", "text": text, "position": "before"}),
                0,
                fixture.binding.source_set_name(),
            )
            .unwrap();
            let post = plan_code_postimage(before.as_bytes(), &operation, 0)
                .unwrap_or_else(|error| panic!("{text:?}: {error}"));
            assert_eq!(std::str::from_utf8(&post.after).unwrap(), expected);
            let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
            assert!(repeated.no_op, "{text:?}");
        }
    fixture.cleanup();
}

#[test]
fn addressed_replacement_handles_empty_inline_and_whole_module_bodies() {
    for (before, at, text, expected) in [
        (
            "Procedure Base()\nEndProcedure\n",
            "main:CommonModule.Sample.Method.Base.Body",
            "    Value = 2;",
            "Procedure Base()\n    Value = 2;\nEndProcedure\n",
        ),
        (
            "Procedure Base() Value = 1; EndProcedure",
            "main:CommonModule.Sample.Method.Base.Body",
            "// replacement\nValue = 2;",
            "Procedure Base()\n// replacement\nValue = 2;\nEndProcedure",
        ),
        (
            "Procedure Base() Value = 1;\n    Value = 3;\n  EndProcedure\n",
            "main:CommonModule.Sample.Method.Base.Body",
            "    Value = 2;",
            "Procedure Base()\n    Value = 2;\n  EndProcedure\n",
        ),
        (
            "Procedure Base() EndProcedure Procedure Neighbor() EndProcedure",
            "main:CommonModule.Sample.Method.Base",
            "Procedure Base() Value = 2; EndProcedure",
            "Procedure Base() Value = 2; EndProcedure Procedure Neighbor() EndProcedure",
        ),
        (
            "\u{feff}Procedure Base()\r\nEndProcedure\r\n",
            "main:CommonModule.Sample.Body",
            "Procedure Changed()\nEndProcedure",
            "\u{feff}Procedure Changed()\r\nEndProcedure\r\n",
        ),
    ] {
        let fixture = staged_code_fixture("addressed-shapes", before.as_bytes());
        let operation = parse_code_plan_operation(
            "code.replace",
            &json!({"at":at, "text":text}),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap();
        let admission = staged_code_admission(&fixture, false);
        let (mut state, effects) = plan_admitted_code(
            &admission,
            &fixture.binding,
            std::slice::from_ref(&operation),
        )
        .unwrap();
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        assert_eq!(state.read(relative).unwrap().unwrap(), expected.as_bytes());
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let repeated = staged_code_admission(&fixture, true);
        let (state, effects) =
            plan_admitted_code(&repeated, &fixture.binding, &[operation]).unwrap();
        assert!(state.planned_changes().is_empty());
        assert!(effects.events().is_empty());
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            expected.as_bytes()
        );
        fixture.cleanup();
    }
}

#[test]
fn addressed_replacement_refuses_missing_ambiguous_and_nonwritable_nodes() {
    let fixture = staged_code_fixture("addressed-refusals", b"Procedure Base()\nEndProcedure\n");
    let admission = staged_code_admission(&fixture, false);
    for at in [
        "main:CommonModule.Sample.Method.Missing.Body",
        "main:CommonModule.Sample.Compilation.Server.Body",
        "main:CommonModule.Sample.Method.Base.Signature",
    ] {
        let parsed = parse_code_plan_operation(
            "code.replace",
            &json!({"at":at,"text":"Value = 2;"}),
            0,
            fixture.binding.source_set_name(),
        );
        let result = parsed
            .and_then(|operation| plan_admitted_code(&admission, &fixture.binding, &[operation]));
        assert!(result.is_err(), "{at}");
    }
    let error = parse_code_plan_operation("code.replace", &json!({"at":"main:CommonModule.Sample.Method.Base.Body","text":"Value = 2;","selector":{"method":"Base"}}), 0, fixture.binding.source_set_name()).unwrap_err();
    assert_eq!(error.path(), Some("ops[0].args.selector"));
    assert_eq!(
        fs::read(fixture.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
        b"Procedure Base()\nEndProcedure\n"
    );
    fixture.cleanup();

    let duplicate = staged_code_fixture(
        "addressed-duplicate",
        b"Procedure Base()\nEndProcedure\nProcedure Base()\nEndProcedure\n",
    );
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at":"main:CommonModule.Sample.Method.Base.Body","text":"Value = 2;"}),
        0,
        duplicate.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&duplicate, false);
    assert!(plan_admitted_code(&admission, &duplicate.binding, &[operation]).is_err());
    duplicate.cleanup();
}

/// Метод целиком задаёт адрес `…Method.M`: селектор метода у замены снят
/// и отказывает в своём поле, называя адрес (#1353). Якорь остаётся.
#[test]
fn code_replace_refuses_a_method_selector_and_names_the_method_address() {
    let fixture = staged_code_fixture(
        "retired-method-selector",
        b"Procedure Base()\nEndProcedure\n",
    );
    let error = parse_code_plan_operation(
            "code.replace",
            &json!({"at":"main:CommonModule.Sample", "selector":{"method":"Base"}, "text":"Procedure Base()\nEndProcedure"}),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(error.path(), Some("ops[0].args.selector.method"));
    let message = error.to_string();
    assert!(
        message.contains("`main:CommonModule.Sample.Method.Base`"),
        "{message}"
    );
    let anchored = parse_code_plan_operation(
            "code.replace",
            &json!({"at":"main:CommonModule.Sample", "selector":{"anchor":"EndProcedure"}, "text":"EndProcedure"}),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap();
    assert!(matches!(
        anchored,
        CodePlanOperation::Replace(CodeReplaceArgs {
            anchor: Some(_),
            ..
        })
    ));
    fixture.cleanup();
}

#[test]
fn whole_method_replacement_extracts_helpers_and_repeats_as_noop() {
    // #895: the "extract method" refactor replaces a method with itself
    // plus new helpers; the repeat finds the method and its helpers in
    // place instead of planning the helpers a second time.
    let target = "Function Target()\r\n    Return 1 + 2;\r\nEndFunction\r\n";
    let before = format!(
        "Procedure First()\r\nEndProcedure\r\n\r\n{target}\r\nProcedure Last()\r\nEndProcedure\r\n"
    );
    let extracted = "Function Target()\n    Return One() + Two();\nEndFunction\n\n\
                         Function One()\n    Return 1;\nEndFunction\n\n\
                         Function Two()\n    Return 2;\nEndFunction";
    let commented = format!("// Extracted.\n{extracted}");
    for (args, text) in [
        (
            json!({"at":"main:CommonModule.Sample.Method.Target", "text":extracted}),
            extracted.to_owned(),
        ),
        (
            json!({"at":"main:CommonModule.Sample.Method.Target", "text":commented}),
            commented.clone(),
        ),
    ] {
        let expected = before.replace(target, &format!("{}\r\n", text.replace('\n', "\r\n")));
        let fixture = staged_code_fixture("extract-method", before.as_bytes());
        let operation =
            parse_code_plan_operation("code.replace", &args, 0, fixture.binding.source_set_name())
                .unwrap();
        let admission = staged_code_admission(&fixture, false);
        let (mut state, effects) = plan_admitted_code(
            &admission,
            &fixture.binding,
            std::slice::from_ref(&operation),
        )
        .unwrap_or_else(|error| panic!("{args}: {error:?}"));
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        assert_eq!(
            String::from_utf8(state.read(relative).unwrap().unwrap()).unwrap(),
            expected,
            "{args}"
        );
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap();
        let repeated = staged_code_admission(&fixture, true);
        let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &[operation])
            .unwrap_or_else(|error| panic!("repeat {args}: {error:?}"));
        assert!(state.planned_changes().is_empty(), "{args}");
        assert!(effects.events().is_empty(), "{args}");
        assert_eq!(
            fs::read(fixture.source.join(relative)).unwrap(),
            expected.as_bytes()
        );
        fixture.cleanup();
    }
}

#[test]
fn whole_method_replacement_refuses_a_helper_the_module_already_defines() {
    let before = "Function Target()\n    Return 1;\nEndFunction\n\nFunction One()\n    Return 1;\nEndFunction\n";
    let text = "Function Target()\n    Return One();\nEndFunction\n\nFunction one()\n    Return 1;\nEndFunction";
    let args = json!({"at":"main:CommonModule.Sample.Method.Target", "text":text});
    let fixture = staged_code_fixture("extract-duplicate", before.as_bytes());
    let operation =
        parse_code_plan_operation("code.replace", &args, 0, fixture.binding.source_set_name())
            .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&admission, &fixture.binding, &[operation])
        .expect_err("a second `One` must not be written");
    assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
    assert_eq!(error.path(), Some("ops[0].args.text"));
    assert_eq!(
        fs::read(fixture.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
        before.as_bytes()
    );
    fixture.cleanup();
}

#[test]
fn module_body_replacement_that_prefixes_the_module_still_writes() {
    // Only a whole method carries extracted helpers; a module body that
    // merely starts with the replacement is not already replaced.
    let before = "Procedure Base()\nEndProcedure\n\nProcedure Other()\nEndProcedure\n";
    let fixture = staged_code_fixture("module-body-prefix", before.as_bytes());
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at":"main:CommonModule.Sample.Body", "text":"Procedure Base()\nEndProcedure"}),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let (mut state, _) = plan_admitted_code(&admission, &fixture.binding, &[operation]).unwrap();
    assert_eq!(
        state
            .read(Path::new("CommonModules/Sample/Ext/Module.bsl"))
            .unwrap()
            .unwrap(),
        b"Procedure Base()\nEndProcedure\n"
    );
    fixture.cleanup();
}

#[test]
fn conditional_compilation_twins_are_not_redeclarations() {
    // `#Если … #Иначе` branches legitimately declare one name twice; the
    // parser cannot tell them from duplicates, so neither a module body
    // nor an extracted helper with such twins is refused.
    let twins = "#Если Сервер Тогда\nProcedure Helper()\nEndProcedure\n#Иначе\nProcedure Helper()\nEndProcedure\n#КонецЕсли";
    let body_fixture = staged_code_fixture("twins-body", b"Procedure Base()\nEndProcedure\n");
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at":"main:CommonModule.Sample.Body", "text":twins}),
        0,
        body_fixture.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&body_fixture, false);
    let (mut state, _) = plan_admitted_code(&admission, &body_fixture.binding, &[operation])
        .expect("a module body with #Если twins is written");
    assert_eq!(
        state
            .read(Path::new("CommonModules/Sample/Ext/Module.bsl"))
            .unwrap()
            .unwrap(),
        format!("{twins}\n").as_bytes()
    );
    body_fixture.cleanup();

    let before = "Procedure Base()\nEndProcedure\n";
    let text = format!("Procedure Base()\n    Helper();\nEndProcedure\n\n{twins}");
    let fixture = staged_code_fixture("twins-helper", before.as_bytes());
    let operation = parse_code_plan_operation(
        "code.replace",
        &json!({"at":"main:CommonModule.Sample.Method.Base", "text":text}),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let (mut state, effects) = plan_admitted_code(
        &admission,
        &fixture.binding,
        std::slice::from_ref(&operation),
    )
    .expect("a helper in both #Если branches is written");
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    assert_eq!(
        state.read(relative).unwrap().unwrap(),
        format!("{text}\n").as_bytes()
    );
    fixture
        .actor
        .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let repeated = staged_code_admission(&fixture, true);
    let (state, _) = plan_admitted_code(&repeated, &fixture.binding, &[operation]).unwrap();
    assert!(state.planned_changes().is_empty());
    fixture.cleanup();
}

#[test]
fn inline_tail_after_the_method_is_not_a_fixed_point() {
    // The method ends mid-line, so the text after it is not whole lines:
    // the repeat would append the tail again and must be refused.
    let before = "Procedure Base()\nEndProcedure // к\n";
    let fixture = staged_code_fixture("inline-tail", before.as_bytes());
    let operation = parse_code_plan_operation(
            "code.replace",
            &json!({"at":"main:CommonModule.Sample.Method.Base", "text":"Procedure Base()\n    Value = 1;\nEndProcedure // к"}),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap();
    let admission = staged_code_admission(&fixture, false);
    let error = plan_admitted_code(&admission, &fixture.binding, &[operation])
        .expect_err("a doubled inline tail must not pass as a repeat no-op");
    assert_eq!(error.kind(), ApplyPlanErrorKind::Postcondition);
    assert_eq!(
        fs::read(fixture.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
        before.as_bytes()
    );
    fixture.cleanup();
}

#[test]
fn code_patch_replace_with_extracted_helpers_repeats_as_noop() {
    let context = temp_context("replace-extract-method");
    let module = context
        .workspace_root
        .join("src/CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(module.parent().unwrap()).unwrap();
    fs::write(&module, "Функция Цель()\n\tВозврат 3;\nКонецФункции\n").unwrap();
    let args = replace_args(
            "CommonModule.Sample.Module",
            json!({"method": "Цель"}),
            "Функция Цель()\n\tВозврат Один() + 2;\nКонецФункции\n\nФункция Один()\n\tВозврат 1;\nКонецФункции",
        );

    let applied = patch_inner(&args, &context, PatchMode::Apply);
    assert!(applied.outcome.ok, "{:?}", applied.outcome.errors);
    let after = fs::read_to_string(&module).unwrap();
    assert_eq!(
            after,
            "Функция Цель()\n\tВозврат Один() + 2;\nКонецФункции\n\nФункция Один()\n\tВозврат 1;\nКонецФункции\n"
        );

    let repeated = patch_inner(&args, &context, PatchMode::Apply);
    assert!(repeated.outcome.ok, "{:?}", repeated.outcome.errors);
    assert!(repeated.data.unwrap().no_op);
    assert_eq!(fs::read_to_string(&module).unwrap(), after);
    fs::remove_dir_all(&context.workspace_root).unwrap();
}

/// Метод, названный адресом, — место вставки до или после него (#1354):
/// текст встаёт рядом с методом, повтор ничего не меняет, а
/// отсутствующий метод отвечает `not_found` по адресу.
#[test]
fn insertion_around_an_addressed_method_needs_a_position_and_the_method() {
    let before = "Procedure First()\nEndProcedure\n\nProcedure Second()\nEndProcedure\n";
    let fixture = staged_code_fixture("around", before.as_bytes());
    let insert = |at: &str, position: &str| {
        parse_code_plan_operation(
            "code.insert",
            &json!({"at": at, "text": "Procedure Added()\nEndProcedure\n", "position": position}),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap()
    };
    for (method, position, earlier, later) in [
        (
            "Second",
            "before",
            "Procedure Added()",
            "Procedure Second()",
        ),
        ("First", "after", "Procedure First()", "Procedure Added()"),
    ] {
        let operation = insert(
            &format!("main:CommonModule.Sample.Method.{method}"),
            position,
        );
        let post = plan_code_postimage(before.as_bytes(), &operation, 0).unwrap();
        let text = std::str::from_utf8(&post.after).unwrap();
        let start = |needle: &str| text.find(needle).unwrap_or_else(|| panic!("{needle}"));
        assert!(start(earlier) < start(later), "{position} {method}");
        assert_eq!(text.matches("Procedure Added()").count(), 1);
        let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
        assert!(repeated.no_op, "{position} {method}");
    }
    // Без `position` метод не место: отказ называет именно её.
    let error = parse_code_plan_operation(
        "code.insert",
        &json!({"at": "main:CommonModule.Sample.Method.First", "text": "x"}),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap_err();
    assert_eq!(error.path(), Some("ops[0].args.position"));
    assert!(error.to_string().contains("at a method needs `position`"));
    let missing = insert("main:CommonModule.Sample.Method.Third", "after");
    let Err(error) = plan_code_postimage(before.as_bytes(), &missing, 0) else {
        panic!("a missing method is not a place to insert");
    };
    assert_eq!(error.kind(), ApplyPlanErrorKind::NotFound);
    assert_eq!(error.path(), Some("ops[0].args.at"));
}

#[test]
fn insertion_around_a_method_keeps_its_head_together_and_respects_its_preprocessor() {
    let fixture = staged_code_fixture("around-head", b"Procedure Base()\nEndProcedure\n");
    let added = "Procedure Added()\nEndProcedure\n";
    let insert = |method: &str, position: &str| {
        parse_code_plan_operation(
            "code.insert",
            &json!({
                "at": format!("main:CommonModule.Sample.Method.{method}"),
                "text": added,
                "position": position
            }),
            0,
            fixture.binding.source_set_name(),
        )
        .unwrap()
    };
    let cases: [(&str, &str, &str, &str); 10] = [
            // Вложенные условия только вокруг метода обходятся все, а
            // комментарий над внешним условием идёт вместе с ним.
            (
                "// О блоке.\n#Если Сервер Тогда\n#Если НЕ ВебКлиент Тогда\nProcedure Target()\nEndProcedure\n#КонецЕсли\n#КонецЕсли\n",
                "Target",
                "before",
                "Procedure Added()\nEndProcedure\n\n// О блоке.\n#Если Сервер Тогда\n#Если НЕ ВебКлиент Тогда\nProcedure Target()\nEndProcedure\n#КонецЕсли\n#КонецЕсли\n",
            ),
            // Пустая строка между условием и головой оставляет вставку внутри.
            (
                "#Если Сервер Тогда\n\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
                "Target",
                "before",
                "#Если Сервер Тогда\n\nProcedure Added()\nEndProcedure\n\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
            ),
            // Комментарий и директива остаются со своим методом.
            (
                "// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n",
                "Target",
                "before",
                "Procedure Added()\nEndProcedure\n\n// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n",
            ),
            (
                "// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n",
                "Target",
                "after",
                "// Документ.\n&НаСервере\nProcedure Target()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n",
            ),
            // Аннотация расширения — тоже голова метода.
            (
                "&Перед(\"Base\")\nProcedure Target()\nEndProcedure\n",
                "Target",
                "before",
                "Procedure Added()\nEndProcedure\n\n&Перед(\"Base\")\nProcedure Target()\nEndProcedure\n",
            ),
            (
                "Procedure Other()\nEndProcedure\n// Документ.\n&Перед(\"Base\")\nProcedure Target()\nEndProcedure\n",
                "Target",
                "before",
                "Procedure Other()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n\n// Документ.\n&Перед(\"Base\")\nProcedure Target()\nEndProcedure\n",
            ),
            // Условие только вокруг метода обходится целиком.
            (
                "#Если Сервер Тогда\n// Документ.\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
                "Target",
                "before",
                "Procedure Added()\nEndProcedure\n\n#Если Сервер Тогда\n// Документ.\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
            ),
            (
                "#Если Сервер Тогда\n// Документ.\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
                "Target",
                "after",
                "#Если Сервер Тогда\n// Документ.\nProcedure Target()\nEndProcedure\n#КонецЕсли\n\nProcedure Added()\nEndProcedure\n",
            ),
            // В условии с другим методом вставка остаётся внутри него.
            (
                "#Если Сервер Тогда\nProcedure Target()\nEndProcedure\nProcedure Other()\nEndProcedure\n#КонецЕсли\n",
                "Target",
                "before",
                "#Если Сервер Тогда\nProcedure Added()\nEndProcedure\n\nProcedure Target()\nEndProcedure\nProcedure Other()\nEndProcedure\n#КонецЕсли\n",
            ),
            (
                "#Если Сервер Тогда\nProcedure Other()\nEndProcedure\nProcedure Target()\nEndProcedure\n#КонецЕсли\n",
                "Target",
                "after",
                "#Если Сервер Тогда\nProcedure Other()\nEndProcedure\nProcedure Target()\nEndProcedure\n\nProcedure Added()\nEndProcedure\n#КонецЕсли\n",
            ),
        ];
    for (before, method, position, expected) in cases {
        let operation = insert(method, position);
        let post = plan_code_postimage(before.as_bytes(), &operation, 0)
            .unwrap_or_else(|error| panic!("{position} {before:?}: {error}"));
        assert_eq!(
            std::str::from_utf8(&post.after).unwrap(),
            expected,
            "{position} {before:?}"
        );
        let repeated = plan_code_postimage(&post.after, &operation, 0).unwrap();
        assert!(repeated.no_op, "{position} {before:?}");
    }
    fixture.cleanup();
}

#[test]
fn staged_code_args_report_exact_paths_and_reject_legacy_fields() {
    let fixture = staged_code_fixture("args", b"Procedure Base()\nEndProcedure\n");
    let valid = parse_code_plan_operation(
        "code.insert",
        &json!({
            "at": "main:CommonModule.Sample",
            "text": "Procedure Added()\nEndProcedure"
        }),
        0,
        fixture.binding.source_set_name(),
    )
    .expect("placeholder parser rejected the valid closed code.insert shape");
    assert!(matches!(valid, CodePlanOperation::Insert(_)));
    // Метод, названный адресом, становится местом вставки до или после.
    let addressed = parse_code_plan_operation(
        "code.insert",
        &json!({
            "at": "main:CommonModule.Sample.Method.Base",
            "text": "Procedure Added()\nEndProcedure",
            "position": "before"
        }),
        0,
        fixture.binding.source_set_name(),
    )
    .expect("a method address with a position is a place to insert");
    let CodePlanOperation::Insert(addressed) = addressed else {
        panic!("code.insert parses as an insertion");
    };
    assert_eq!(
        addressed.selector,
        Some(CodeSelector::Method("Base".to_string()))
    );
    assert_eq!(addressed.position, Some(CodePosition::Before));

    let cases = [
        ("code.insert", json!(null), "ops[1].args"),
        ("code.insert", json!({"text": "x"}), "ops[1].args.at"),
        (
            "code.insert",
            json!({"at": "", "text": "x"}),
            "ops[1].args.at",
        ),
        (
            "code.insert",
            json!({"at": 42, "text": "x"}),
            "ops[1].args.at",
        ),
        (
            "code.insert",
            json!({"at": "not-an-address", "text": "x"}),
            "ops[1].args.at",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample"}),
            "ops[1].args.text",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": ""}),
            "ops[1].args.text",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": 1}),
            "ops[1].args.text",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": "Run"}),
            "ops[1].args.selector",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {}}),
            "ops[1].args.selector",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run", "anchor": "x"}, "position": "after"}),
            "ops[1].args.selector",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": ""}, "position": "after"}),
            "ops[1].args.selector.method",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"unknown": "Run"}, "position": "after"}),
            "ops[1].args.selector.unknown",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"anchor": "Run"}}),
            "ops[1].args.position",
        ),
        // Метод называет адрес (#1354): селектор метода снят, а у
        // адресованного метода место — обязательная `position`.
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}, "position": "after"}),
            "ops[1].args.selector.method",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample.Method.Run", "text": "x"}),
            "ops[1].args.position",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample.Method.Run", "text": "x", "selector": {"anchor": "Run"}, "position": "after"}),
            "ops[1].args.selector",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample.Method.Run.Body", "text": "x"}),
            "ops[1].args.at",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "position": "after"}),
            "ops[1].args.position",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"anchor": "Run"}, "position": 1}),
            "ops[1].args.position",
        ),
        (
            "code.insert",
            json!({"at": "main:CommonModule.Sample.Method.Run", "text": "x", "position": "middle"}),
            "ops[1].args.position",
        ),
        (
            "code.replace",
            json!({"at": "main:CommonModule.Sample", "text": "x"}),
            "ops[1].args.selector",
        ),
        (
            "code.replace",
            json!({"at": "main:CommonModule.Sample", "text": "x", "selector": {"method": "Run"}, "position": "after"}),
            "ops[1].args.position",
        ),
        (
            "code.insert",
            json!({"at": "foreign:CommonModule.Sample", "text": "x"}),
            "ops[1].args.at",
        ),
    ];
    for (operation, args, expected_path) in cases {
        let error =
            parse_code_plan_operation(operation, &args, 1, fixture.binding.source_set_name())
                .expect_err("invalid staged code arguments were accepted");
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue, "{args}");
        assert_eq!(error.path(), Some(expected_path), "{args}");
    }
    for legacy in [
        "operation",
        "content",
        "sourceSet",
        "sourceKind",
        "path",
        "metadataPath",
        "targetPath",
        "jsonPath",
        "provider",
        "providerIdentity",
        "providerProfile",
    ] {
        let mut args = json!({
            "at": "main:CommonModule.Sample",
            "text": "Procedure Added()\nEndProcedure"
        });
        args.as_object_mut()
            .unwrap()
            .insert(legacy.to_string(), json!("legacy"));
        let error =
            parse_code_plan_operation("code.insert", &args, 2, fixture.binding.source_set_name())
                .expect_err("legacy/physical code argument was accepted");
        assert_eq!(error.kind(), ApplyPlanErrorKind::BadValue);
        assert_eq!(error.path(), Some(format!("ops[2].args.{legacy}").as_str()));
    }
    fixture.cleanup();
}

#[test]
fn staged_code_rejects_raw_policy_and_same_root_foreign_binding_substitution() {
    let locked = staged_code_fixture(
        "authority-policy-substitution",
        b"Procedure Base()\nEndProcedure\n",
    );
    write_locked_support_state(&locked.source);
    let denied = staged_code_admission(&locked, true);
    assert_eq!(denied.support_policy_mode(), SupportPolicyMode::Deny);
    fs::write(
        locked.root.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"off"}"#,
    )
    .unwrap();
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    let source_before = snapshot_staged_code_tree(&locked.source);
    let cache_before = snapshot_staged_code_tree(&locked.root.join(".build/unica"));

    let authority = denied
        .code_planning_authority(&locked.binding)
        .expect("the original actor binding must remain admitted");
    let substituted =
        plan_code_batch_owned_for_test(denied.staged_state().unwrap(), authority, &operation);

    let error = substituted
        .expect_err("a Deny admission followed ambient Off policy instead of its sealed evidence");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
    assert_eq!(snapshot_staged_code_tree(&locked.source), source_before);
    assert_eq!(
        snapshot_staged_code_tree(&locked.root.join(".build/unica")),
        cache_before
    );
    assert!(!locked.root.join(".build/unica/state.json").exists());
    locked.cleanup();

    let fixture = staged_code_fixture(
        "authority-foreign-binding",
        b"Procedure Base()\nEndProcedure\n",
    );
    let foreign_actor = staged_code_actor(&fixture.root, &fixture.source);
    let foreign_binding = foreign_actor
        .bind_provider_root("main", &fixture.source)
        .unwrap();
    let admission = staged_code_admission(&fixture, true);
    let source_before = snapshot_staged_code_tree(&fixture.source);
    let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));

    let substituted = admission.code_planning_authority(&foreign_binding);

    let error = substituted
        .err()
        .expect("same-root binding from another actor instance was accepted");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
    assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
    assert_eq!(
        snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
        cache_before
    );
    assert!(!fixture.root.join(".build/unica/state.json").exists());
    fixture.cleanup();

    let writer = staged_code_fixture(
        "authority-writer-substitution",
        b"Procedure Base()\nEndProcedure\n",
    );
    let first = staged_code_admission(&writer, true);
    let second = staged_code_admission(&writer, true);
    let authority = first.code_planning_authority(&writer.binding).unwrap();
    let error =
        plan_code_batch_owned_for_test(second.staged_state().unwrap(), authority, &operation)
            .expect_err("staged state from another admission writer was accepted");
    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState);
    assert!(!writer.root.join(".build/unica/state.json").exists());
    writer.cleanup();
}

/// #955: a registered owner's module exists logically without its file.
/// A dumped empty common module keeps an empty `Ext/` without the file;
/// `object.borrow` registers a common module without its directory.
/// The first write materializes the missing chain on publication only.
#[test]
fn staged_code_first_body_materializes_a_missing_module_chain() {
    for (name, missing) in [
        ("missing-file", "CommonModules/Sample/Ext/Module.bsl"),
        ("missing-ext", "CommonModules/Sample/Ext"),
        ("missing-owner-directory", "CommonModules/Sample"),
    ] {
        let fixture = staged_code_fixture(name, b"Procedure Base()\nEndProcedure\n");
        let removed = fixture.source.join(missing);
        if removed.is_dir() {
            fs::remove_dir_all(&removed).unwrap();
        } else {
            fs::remove_file(&removed).unwrap();
        }
        let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
        let operation = [staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )];
        let source_before = snapshot_staged_code_tree(&fixture.source);
        let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));

        let preview = staged_code_admission(&fixture, true);
        let (mut state, effects) = plan_admitted_code(&preview, &fixture.binding, &operation)
            .unwrap_or_else(|error| panic!("{missing}: {error:?}"));
        let changes = state.planned_changes();
        assert_eq!(changes.len(), 1, "{missing}");
        assert_eq!(changes[0].relative_path, relative, "{missing}");
        assert_eq!(changes[0].original, StagedFileState::Absent, "{missing}");
        let expected = state.read(relative).unwrap().expect("staged first body");
        assert!(
            String::from_utf8_lossy(&expected).contains("Procedure Added()"),
            "{missing}"
        );
        assert_eq!(
            effects.events(),
            &[DomainEvent::new(
                DomainEventKind::ModuleChanged,
                "main:CommonModule.Sample"
            )]
        );
        assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
        assert_eq!(
            snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
            cache_before
        );
        drop(state);

        let admission = staged_code_admission(&fixture, false);
        let (state, effects) =
            plan_admitted_code(&admission, &fixture.binding, &operation).unwrap();
        fixture
            .actor
            .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
            .unwrap_or_else(|error| panic!("{missing}: {error:?}"));
        assert_eq!(fs::read(fixture.source.join(relative)).unwrap(), expected);

        let repeated = staged_code_admission(&fixture, true);
        let (state, effects) = plan_admitted_code(&repeated, &fixture.binding, &operation).unwrap();
        assert!(state.planned_changes().is_empty(), "{missing}");
        assert!(effects.events().is_empty(), "{missing}");
        fixture.cleanup();
    }
}

/// A module chain that appears after planning belongs to someone else:
/// publication refuses and keeps the foreign file instead of overwriting it.
#[test]
fn staged_code_missing_module_chain_keeps_a_foreign_appearance() {
    let fixture = staged_code_fixture("foreign-chain", b"Procedure Base()\nEndProcedure\n");
    fs::remove_dir_all(fixture.source.join("CommonModules/Sample")).unwrap();
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    let admission = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&admission, &fixture.binding, &operation).unwrap();
    let prepared = admission.prepare_with_effects(state, effects).unwrap();
    let foreign = fixture.source.join("CommonModules/Sample/Ext/Module.bsl");
    fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    fs::write(&foreign, b"// foreign body\n").unwrap();

    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert_eq!(fs::read(&foreign).unwrap(), b"// foreign body\n");
    assert!(!fixture.root.join(".build/unica/state.json").exists());
    fixture.cleanup();
}

#[test]
fn staged_code_support_guard_matches_v12_compatibility_for_all_marker_bytes() {
    let cases: Vec<(&str, Option<Vec<u8>>, bool)> = vec![
            ("absent", None, false),
            ("empty", Some(Vec::new()), false),
            ("short", Some(b"short marker".to_vec()), false),
            ("malformed-non-utf8", Some(vec![0xff; 40]), false),
            (
                "malformed-header",
                Some(b"not a support header but longer than thirty-two bytes".to_vec()),
                false,
            ),
            (
                "removed-zero-vendor",
                Some(b"{6,0,0,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                false,
            ),
            (
                "global-deny",
                Some(b"{6,1,1,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                true,
            ),
            (
                "global-allow",
                Some(b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd}".to_vec()),
                false,
            ),
            (
                "owner-deny",
                Some(
                    b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
                        .to_vec(),
                ),
                true,
            ),
            (
                "owner-allow",
                Some(
                    b"{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,1,0,bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb}"
                        .to_vec(),
                ),
                false,
            ),
        ];
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    for (name, marker, expected_blocked) in cases {
        let fixture = staged_code_fixture(
            &format!("support-compat-{name}"),
            b"Procedure Base()\nEndProcedure\n",
        );
        let marker_path = fixture.source.join("Ext/ParentConfigurations.bin");
        match marker {
            Some(bytes) => fs::write(&marker_path, bytes).unwrap(),
            None => assert!(!marker_path.exists()),
        }
        let module = fixture.source.join("CommonModules/Sample/Ext/Module.bsl");
        let v12_blocked =
            support_guard_violation(&module, SupportGuardRequirement::Editable).is_some();
        assert_eq!(v12_blocked, expected_blocked, "invalid V12 oracle: {name}");
        let v12_context = WorkspaceContext {
            cwd: fixture.root.clone(),
            workspace_root: fixture.root.clone(),
            cache_root: fixture.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let v12 = patch_inner(
            &patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Base",
                "Procedure Added()\nEndProcedure",
            ),
            &v12_context,
            PatchMode::Preview,
        );
        assert_eq!(
            !v12.outcome.ok, v12_blocked,
            "V12 patch path disagrees with its support guard for {name}: {:?}",
            v12.outcome.errors
        );
        let admission = staged_code_admission(&fixture, true);

        let c1 = plan_admitted_code(&admission, &fixture.binding, &operation);

        assert_eq!(c1.is_err(), v12_blocked, "V12/C1 mismatch for {name}");
        if let Err(error) = c1 {
            assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidState, "{name}");
        }
        fixture.cleanup();
    }
}

#[test]
fn staged_code_selector_kind_does_not_depend_on_error_message_words() {
    let error = super::code_selector_error(
        super::SelectorResolutionError::invalid_source(
            "invalid source contains the word matched but has no match count",
        ),
        3,
        &super::Selector::Method("Base".to_string()),
    );

    assert_eq!(error.kind(), ApplyPlanErrorKind::InvalidSource);
    assert_eq!(error.path(), Some("ops[3].args.selector.method"));

    for (name, before, selector, expected) in [
        (
            "zero-method",
            b"Procedure Base()\nEndProcedure\n".as_slice(),
            CodeSelector::Method("Missing".to_string()),
            ApplyPlanErrorKind::NotFound,
        ),
        (
            "multiple-method",
            b"Procedure Base()\nEndProcedure\nProcedure Base()\nEndProcedure\n".as_slice(),
            CodeSelector::Method("Base".to_string()),
            ApplyPlanErrorKind::InvalidState,
        ),
        (
            "zero-anchor",
            b"Procedure Base()\nEndProcedure\n".as_slice(),
            CodeSelector::Anchor("// marker".to_string()),
            ApplyPlanErrorKind::NotFound,
        ),
        (
            "multiple-anchor",
            b"Procedure Base()\n    // marker\n    // marker\nEndProcedure\n".as_slice(),
            CodeSelector::Anchor("// marker".to_string()),
            ApplyPlanErrorKind::InvalidState,
        ),
    ] {
        for operation_kind in ["insert", "replace"] {
            let fixture =
                staged_code_fixture(&format!("selector-cause-{operation_kind}-{name}"), before);
            let admission = staged_code_admission(&fixture, true);
            let operation = [match operation_kind {
                "insert" => staged_insert(
                    "main:CommonModule.Sample",
                    "Procedure Added()\nEndProcedure",
                    Some(selector.clone()),
                    Some(CodePosition::After),
                ),
                // Метод целиком заменяется по адресу, якорь — селектором.
                "replace" => match &selector {
                    CodeSelector::Method(name) => staged_replace(
                        &format!("main:CommonModule.Sample.Method.{name}"),
                        "Procedure Added()\nEndProcedure",
                        None,
                    ),
                    CodeSelector::Anchor(anchor) => staged_replace(
                        "main:CommonModule.Sample",
                        "Procedure Added()\nEndProcedure",
                        Some(anchor),
                    ),
                },
                _ => unreachable!(),
            }];
            let failure = format!("{operation_kind} {name}");
            let projected =
                plan_admitted_code(&admission, &fixture.binding, &operation).expect_err(&failure);
            assert_eq!(projected.kind(), expected, "{operation_kind} {name}");
            fixture.cleanup();
        }
    }
}

#[test]
fn staged_code_preserves_module_owner_support_format_and_preimage_guards() {
    let valid = staged_code_fixture("guards-valid", b"Procedure Base()\nEndProcedure\n");
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    let admission = staged_code_admission(&valid, true);
    let (state, effects) = plan_admitted_code(&admission, &valid.binding, &operation)
        .expect("valid actor-issued profile was rejected by the placeholder planner");
    assert_eq!(effects.events().len(), 1);
    drop(state);
    drop(admission);

    let descriptor_target = [staged_insert(
        "main:CommonModule.Sample.Module",
        "Procedure Added()\nEndProcedure",
        None,
        None,
    )];
    let admission = staged_code_admission(&valid, true);
    assert_eq!(
        plan_admitted_code(&admission, &valid.binding, &descriptor_target)
            .unwrap_err()
            .kind(),
        ApplyPlanErrorKind::InvalidSource
    );

    fs::write(
        valid.source.join("CommonModules/Sample/Ext/Module.bsl"),
        b"Procedure Broken(\n",
    )
    .unwrap();
    let malformed_admission = staged_code_admission(&valid, true);
    assert_eq!(
        plan_admitted_code(&malformed_admission, &valid.binding, &operation)
            .unwrap_err()
            .kind(),
        ApplyPlanErrorKind::InvalidSource
    );
    valid.cleanup();

    let locked = staged_code_fixture("guards-locked", b"Procedure Base()\nEndProcedure\n");
    write_locked_support_state(&locked.source);
    let denied = staged_code_admission(&locked, true);
    assert_eq!(denied.support_policy_mode(), SupportPolicyMode::Deny);
    assert_eq!(
        plan_admitted_code(&denied, &locked.binding, &operation)
            .unwrap_err()
            .kind(),
        ApplyPlanErrorKind::InvalidState
    );
    fs::write(
        locked.root.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"warn"}"#,
    )
    .unwrap();
    let warned = staged_code_admission(&locked, true);
    assert_eq!(warned.support_policy_mode(), SupportPolicyMode::Warn);
    assert!(plan_admitted_code(&warned, &locked.binding, &operation).is_ok());
    fs::write(
        locked.root.join(".v8-project.json"),
        br#"{"editingAllowedCheck":"off"}"#,
    )
    .unwrap();
    let off = staged_code_admission(&locked, true);
    assert_eq!(off.support_policy_mode(), SupportPolicyMode::Off);
    assert!(plan_admitted_code(&off, &locked.binding, &operation).is_ok());
    locked.cleanup();

    let absent = staged_code_fixture("guards-absent", b"Procedure Base()\nEndProcedure\n");
    fs::remove_file(absent.source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap();
    let absent_admission = staged_code_admission(&absent, true);
    let (state, _) = plan_admitted_code(&absent_admission, &absent.binding, &operation)
        .expect("profile-permitted absent module was rejected");
    assert!(matches!(
        state.planned_changes()[0].original,
        StagedFileState::Absent
    ));
    absent.cleanup();

    let raced = staged_code_fixture("guards-owner-race", b"Procedure Base()\nEndProcedure\n");
    let raced_admission = staged_code_admission(&raced, false);
    let (state, effects) =
        plan_admitted_code(&raced_admission, &raced.binding, &operation).unwrap();
    fs::write(
        raced.source.join("CommonModules/Sample.xml"),
        b"<MetaDataObject concurrent=\"true\"/>",
    )
    .unwrap();
    assert!(raced_admission
        .prepare_with_effects(state, effects)
        .is_err());
    raced.cleanup();

    let replaced_root =
        staged_code_fixture("guards-root-race", b"Procedure Base()\nEndProcedure\n");
    let root_admission = staged_code_admission(&replaced_root, false);
    let (state, effects) =
        plan_admitted_code(&root_admission, &replaced_root.binding, &operation).unwrap();
    let prepared = root_admission.prepare_with_effects(state, effects).unwrap();
    let retained_source = replaced_root.root.join("retained-source");
    let retained_identity = path_identity_for_test(&replaced_root.source)
        .unwrap()
        .expect("source root identity must be available on supported CI platforms");
    let replacement =
        attempt_retained_directory_replacement_for_test(&replaced_root.source, &retained_source)
            .unwrap();
    match replacement {
        RetainedDirectoryReplacementOutcome::Replaced => {
            fs::create_dir(&replaced_root.source).unwrap();
            assert!(replaced_root
                .actor
                .publish_prepared_apply(prepared)
                .is_err());
            assert_eq!(
                fs::read(retained_source.join("CommonModules/Sample/Ext/Module.bsl")).unwrap(),
                b"Procedure Base()\nEndProcedure\n"
            );
            assert!(fs::read_dir(&replaced_root.source)
                .unwrap()
                .next()
                .is_none());
        }
        RetainedDirectoryReplacementOutcome::PreventedByRetainedHandle => {
            assert_eq!(
                path_identity_for_test(&replaced_root.source)
                    .unwrap()
                    .as_deref(),
                Some(retained_identity.as_str())
            );
            assert!(!retained_source.exists());
            let result = replaced_root
                .actor
                .publish_prepared_apply(prepared)
                .unwrap();
            assert_eq!(result.effects().events().len(), 1);
            let module = fs::read(
                replaced_root
                    .source
                    .join("CommonModules/Sample/Ext/Module.bsl"),
            )
            .unwrap();
            assert!(module
                .windows(b"Procedure Added()".len())
                .any(|window| { window == b"Procedure Added()" }));
        }
    }
    replaced_root.cleanup();

    let linked = staged_code_fixture("guards-link", b"Procedure Base()\nEndProcedure\n");
    let link_admission = staged_code_admission(&linked, true);
    let module = linked.source.join("CommonModules/Sample/Ext/Module.bsl");
    let real = linked
        .source
        .join("CommonModules/Sample/Ext/RealModule.bsl");
    fs::rename(&module, &real).unwrap();
    if create_file_link_fixture_for_test(&real, &module).unwrap() == FileLinkFixtureOutcome::Created
    {
        let error = plan_admitted_code(&link_admission, &linked.binding, &operation).unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ApplyPlanErrorKind::Staging(
                    ApplyStagingErrorKind::ContainmentIdentity
                        | ApplyStagingErrorKind::UnsupportedProvider
                )
            ),
            "{error:?}"
        );
    }
    linked.cleanup();

    let linked_support =
        staged_code_fixture("guards-support-link", b"Procedure Base()\nEndProcedure\n");
    let support_admission = staged_code_admission(&linked_support, true);
    let real_support = linked_support.root.join("retained-support-marker.bin");
    fs::write(&real_support, b"short marker").unwrap();
    let support_marker = linked_support.source.join("Ext/ParentConfigurations.bin");
    if create_file_link_fixture_for_test(&real_support, &support_marker).unwrap()
        == FileLinkFixtureOutcome::Created
    {
        let error = plan_admitted_code(&support_admission, &linked_support.binding, &operation)
            .unwrap_err();
        assert!(
            matches!(
                error.kind(),
                ApplyPlanErrorKind::Staging(
                    ApplyStagingErrorKind::ContainmentIdentity
                        | ApplyStagingErrorKind::UnsupportedProvider
                )
            ),
            "{error:?}"
        );
        assert!(!linked_support.root.join(".build/unica/state.json").exists());
    }
    linked_support.cleanup();

    let generated = staged_code_fixture("guards-generated", b"Procedure Base()\nEndProcedure\n");
    let mut staged = staged_code_admission(&generated, true)
        .staged_state()
        .unwrap();
    assert_eq!(
        staged
            .read(Path::new(".build/unica/foreign"))
            .unwrap_err()
            .kind(),
        ApplyStagingErrorKind::ContainmentIdentity
    );
    generated.cleanup();
}

#[test]
fn staged_code_v12_parity_preserves_bytes_errors_and_noop() {
    let before = b"\xef\xbb\xbfProcedure Base()\r\nEndProcedure\r\n// untouched\r\n";
    let v12 = staged_code_fixture("v12-parity-v12", before);
    let staged = staged_code_fixture("v12-parity-v13", before);
    let v12_context = WorkspaceContext {
        cwd: v12.root.clone(),
        workspace_root: v12.root.clone(),
        cache_root: v12.root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let v12_args = patch_args(
        "main",
        "CommonModule.Sample.Module",
        "Base",
        "Procedure Added()\nEndProcedure",
    );
    let v12_result = patch_inner(&v12_args, &v12_context, PatchMode::Apply);
    assert!(v12_result.outcome.ok, "{:?}", v12_result.outcome.errors);
    let operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        Some(CodeSelector::Method("Base".to_string())),
        Some(CodePosition::After),
    )];
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    {
        let admission = staged_code_admission(&staged, true);
        let (mut state, effects) = plan_admitted_code(&admission, &staged.binding, &operation)
            .expect("placeholder planner cannot produce the V12 parity postimage");
        assert_eq!(
            state.read(relative).unwrap().unwrap(),
            fs::read(v12.source.join(relative)).unwrap()
        );
        assert_eq!(effects.events().len(), 1);
    }

    let invalid_v12 = patch_inner(
        &patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Missing",
            "Procedure Added()\nEndProcedure",
        ),
        &v12_context,
        PatchMode::Preview,
    );
    assert!(!invalid_v12.outcome.ok);
    let invalid_operation = [staged_insert(
        "main:CommonModule.Sample",
        "Procedure Added()\nEndProcedure",
        Some(CodeSelector::Method("Missing".to_string())),
        Some(CodePosition::After),
    )];
    {
        let invalid_admission = staged_code_admission(&staged, true);
        assert_eq!(
            plan_admitted_code(&invalid_admission, &staged.binding, &invalid_operation)
                .unwrap_err()
                .kind(),
            ApplyPlanErrorKind::NotFound
        );
    }

    let repeated_v12 = patch_inner(&v12_args, &v12_context, PatchMode::Apply);
    assert!(repeated_v12.outcome.ok);
    assert!(repeated_v12.data.unwrap().no_op);
    let real_admission = staged_code_admission(&staged, false);
    let (state, effects) = plan_admitted_code(&real_admission, &staged.binding, &operation)
        .expect("first staged parity publication planning failed");
    staged
        .actor
        .publish_prepared_apply(real_admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();
    let repeat_admission = staged_code_admission(&staged, true);
    let (repeat_state, repeat_effects) =
        plan_admitted_code(&repeat_admission, &staged.binding, &operation).unwrap();
    assert!(repeat_state.planned_changes().is_empty());
    assert!(repeat_effects.events().is_empty());

    let malformed_v12 = staged_code_fixture("v12-malformed", b"Procedure Broken(\n");
    let malformed_context = WorkspaceContext {
        cwd: malformed_v12.root.clone(),
        workspace_root: malformed_v12.root.clone(),
        cache_root: malformed_v12.root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let malformed_result = patch_inner(&v12_args, &malformed_context, PatchMode::Preview);
    assert!(!malformed_result.outcome.ok);
    let malformed_admission = staged_code_admission(&malformed_v12, true);
    assert_eq!(
        plan_admitted_code(&malformed_admission, &malformed_v12.binding, &operation)
            .unwrap_err()
            .kind(),
        ApplyPlanErrorKind::InvalidSource
    );
    malformed_v12.cleanup();
    v12.cleanup();
    staged.cleanup();
}

#[test]
fn staged_code_v12_parity_covers_insert_replace_method_and_anchor() {
    assert_v12_c1_operation_parity(
        "insert-method",
        b"Procedure Base()\nEndProcedure\n",
        patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Base",
            "Procedure Added()\nEndProcedure",
        ),
        staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            Some(CodeSelector::Method("Base".to_string())),
            Some(CodePosition::After),
        ),
        true,
    );
    assert_v12_c1_operation_parity(
        "insert-anchor",
        b"Procedure Base()\n    // marker\nEndProcedure\n",
        patch_args_for_selector(
            "main",
            "CommonModule.Sample.Module",
            json!({"anchor": "// marker"}),
            "after",
            "Message(\"added\");",
        ),
        staged_insert(
            "main:CommonModule.Sample",
            "Message(\"added\");",
            Some(CodeSelector::Anchor("// marker".to_string())),
            Some(CodePosition::After),
        ),
        true,
    );
    assert_v12_c1_operation_parity(
        "replace-method",
        b"Procedure Base()\nEndProcedure\n",
        replace_args(
            "CommonModule.Sample.Module",
            json!({"method": "Base"}),
            "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
        ),
        staged_replace(
            "main:CommonModule.Sample.Method.Base",
            "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
            None,
        ),
        true,
    );
    assert_v12_c1_operation_parity(
        "replace-anchor",
        "Procedure Base()\n    Значение = 1;\nEndProcedure\n".as_bytes(),
        replace_args(
            "CommonModule.Sample.Module",
            json!({"anchor": "Значение = 1;"}),
            "Значение = 2;",
        ),
        staged_replace(
            "main:CommonModule.Sample",
            "Значение = 2;",
            Some("Значение = 1;"),
        ),
        false,
    );
}

#[test]
fn staged_code_v12_parity_rejects_malformed_postimage_and_owner_evidence() {
    let malformed_v12 = staged_code_fixture("parity-post-v12", b"Procedure Base()\nEndProcedure\n");
    let malformed_c1 = staged_code_fixture("parity-post-c1", b"Procedure Base()\nEndProcedure\n");
    let context = WorkspaceContext {
        cwd: malformed_v12.root.clone(),
        workspace_root: malformed_v12.root.clone(),
        cache_root: malformed_v12.root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let v12 = patch_inner(
        &patch_args(
            "main",
            "CommonModule.Sample.Module",
            "Base",
            "Procedure Broken(",
        ),
        &context,
        PatchMode::Preview,
    );
    assert!(!v12.outcome.ok);
    let admission = staged_code_admission(&malformed_c1, true);
    let c1 = plan_admitted_code(
        &admission,
        &malformed_c1.binding,
        &[staged_insert(
            "main:CommonModule.Sample",
            "Procedure Broken(",
            Some(CodeSelector::Method("Base".to_string())),
            Some(CodePosition::After),
        )],
    )
    .unwrap_err();
    assert_eq!(c1.kind(), ApplyPlanErrorKind::Postcondition);
    malformed_v12.cleanup();
    malformed_c1.cleanup();

    for case in ["absent", "malformed", "wrong-format"] {
        let v12 = staged_code_fixture(
            &format!("owner-{case}-v12"),
            b"Procedure Base()\nEndProcedure\n",
        );
        let c1 = staged_code_fixture(
            &format!("owner-{case}-c1"),
            b"Procedure Base()\nEndProcedure\n",
        );
        for fixture in [&v12, &c1] {
            let descriptor = fixture.source.join("CommonModules/Sample.xml");
            match case {
                "absent" => fs::remove_file(descriptor).unwrap(),
                "malformed" => fs::write(descriptor, b"<not-xml").unwrap(),
                "wrong-format" => {
                    let before = fs::read_to_string(&descriptor).unwrap();
                    fs::write(descriptor, before.replace("2.20", "2.21")).unwrap();
                }
                _ => unreachable!(),
            }
        }
        let context = WorkspaceContext {
            cwd: v12.root.clone(),
            workspace_root: v12.root.clone(),
            cache_root: v12.root.join(".build/unica"),
            workspace_epoch: 1,
        };
        let v12_result = patch_inner(
            &patch_args(
                "main",
                "CommonModule.Sample.Module",
                "Base",
                "Procedure Added()\nEndProcedure",
            ),
            &context,
            PatchMode::Preview,
        );
        assert!(!v12_result.outcome.ok, "V12 accepted owner case {case}");
        let admission = staged_code_admission(&c1, true);
        assert!(
            plan_admitted_code(
                &admission,
                &c1.binding,
                &[staged_insert(
                    "main:CommonModule.Sample",
                    "Procedure Added()\nEndProcedure",
                    Some(CodeSelector::Method("Base".to_string())),
                    Some(CodePosition::After),
                )],
            )
            .is_err(),
            "C1 accepted owner case {case}"
        );
        v12.cleanup();
        c1.cleanup();
    }
}

#[test]
fn staged_code_effects_track_changed_modules_only_in_stable_order() {
    use unica_workspace::compile_transaction::{
        set_retained_apply_failpoint, RetainedApplyFailpoint,
    };

    let fixture = staged_code_fixture("effects", b"Procedure Base()\nEndProcedure\n");
    let operations = [
        staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        ),
        staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        ),
        staged_insert(
            "main:CommonModule.Other",
            "Procedure OtherAdded()\nEndProcedure",
            None,
            None,
        ),
    ];
    let admission = staged_code_admission(&fixture, false);
    let (state, effects) = plan_admitted_code(&admission, &fixture.binding, &operations)
        .expect("placeholder planner cannot produce stable changed-module effects");
    assert_eq!(
        effects.events(),
        &[
            DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Sample"),
            DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Other"),
        ]
    );
    let prepared = admission.prepare_with_effects(state, effects).unwrap();
    let source_before = snapshot_staged_code_tree(&fixture.source);
    let cache_before = snapshot_staged_code_tree(&fixture.root.join(".build/unica"));
    set_retained_apply_failpoint(RetainedApplyFailpoint::AfterAllPostimages);
    assert!(fixture.actor.publish_prepared_apply(prepared).is_err());
    assert_eq!(snapshot_staged_code_tree(&fixture.source), source_before);
    assert_eq!(
        snapshot_staged_code_tree(&fixture.root.join(".build/unica")),
        cache_before
    );

    let poisoned = staged_code_admission(&fixture, true);
    let poisoned_ops = [
        operations[0].clone(),
        staged_replace(
            "main:CommonModule.Sample.Method.Missing",
            "Procedure Never()\nEndProcedure",
            None,
        ),
    ];
    assert!(plan_admitted_code(&poisoned, &fixture.binding, &poisoned_ops).is_err());
    fixture.cleanup();
}

#[test]
fn staged_code_effects_omit_a_module_restored_to_its_original_preimage() {
    let fixture = staged_code_fixture(
        "effects-restored-preimage",
        b"Procedure Base()\nEndProcedure\n",
    );
    let admission = staged_code_admission(&fixture, true);
    let operations = [
        staged_replace(
            "main:CommonModule.Sample.Method.Base",
            "Procedure Base()\n    Message(\"changed\");\nEndProcedure",
            None,
        ),
        staged_replace(
            "main:CommonModule.Sample.Method.Base",
            "Procedure Base()\nEndProcedure",
            None,
        ),
    ];

    let (state, effects) = plan_admitted_code(&admission, &fixture.binding, &operations).unwrap();

    assert!(state.planned_changes().is_empty());
    assert!(effects.events().is_empty());
    fixture.cleanup();
}

#[test]
fn shared_apply_plan_error_and_effect_contract_is_complete() {
    let fixture = staged_code_fixture("error-projections", b"Procedure Base()\nEndProcedure\n");
    let bad_value = parse_code_plan_operation(
        "code.insert",
        &json!({"text": "x"}),
        0,
        fixture.binding.source_set_name(),
    )
    .unwrap_err();
    assert_closed_code_error(
        &bad_value,
        ApplyPlanErrorKind::BadValue,
        Some("ops[0].args.at"),
        &fixture.root,
    );

    let missing = [staged_replace(
        "main:CommonModule.Sample.Method.Missing",
        "Procedure Added()\nEndProcedure",
        None,
    )];
    let admission = staged_code_admission(&fixture, true);
    let not_found = plan_admitted_code(&admission, &fixture.binding, &missing).unwrap_err();
    assert_closed_code_error(
        &not_found,
        ApplyPlanErrorKind::NotFound,
        Some("ops[0].args.at"),
        &fixture.root,
    );

    write_locked_support_state(&fixture.source);
    let admission = staged_code_admission(&fixture, true);
    let invalid_state = plan_admitted_code(
        &admission,
        &fixture.binding,
        &[staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )],
    )
    .unwrap_err();
    assert_closed_code_error(
        &invalid_state,
        ApplyPlanErrorKind::InvalidState,
        Some("ops[0].args.at"),
        &fixture.root,
    );
    fs::remove_file(fixture.source.join("Ext/ParentConfigurations.bin")).unwrap();

    fs::write(
        fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
        b"Procedure Broken(\n",
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, true);
    let invalid_source = plan_admitted_code(
        &admission,
        &fixture.binding,
        &[staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )],
    )
    .unwrap_err();
    assert_closed_code_error(
        &invalid_source,
        ApplyPlanErrorKind::InvalidSource,
        Some("ops[0].args.at"),
        &fixture.root,
    );

    fs::write(
        fixture.source.join("CommonModules/Sample/Ext/Module.bsl"),
        b"Procedure Base()\nEndProcedure\n",
    )
    .unwrap();
    let admission = staged_code_admission(&fixture, true);
    let postcondition = plan_admitted_code(
        &admission,
        &fixture.binding,
        &[staged_insert(
            "main:CommonModule.Sample",
            "Procedure Base()\nEndProcedure",
            Some(CodeSelector::Method("Base".to_string())),
            Some(CodePosition::After),
        )],
    )
    .unwrap_err();
    assert_closed_code_error(
        &postcondition,
        ApplyPlanErrorKind::Postcondition,
        Some("ops[0].args.text"),
        &fixture.root,
    );

    fs::remove_dir_all(fixture.source.join("CommonModules/Sample/Ext")).unwrap();
    fs::write(fixture.source.join("CommonModules/Sample/Ext"), b"occupied").unwrap();
    let admission = staged_code_admission(&fixture, true);
    let staging = plan_admitted_code(
        &admission,
        &fixture.binding,
        &[staged_insert(
            "main:CommonModule.Sample",
            "Procedure Added()\nEndProcedure",
            None,
            None,
        )],
    )
    .unwrap_err();
    assert_closed_code_error(
        &staging,
        ApplyPlanErrorKind::Staging(ApplyStagingErrorKind::AbsentChainOccupied),
        Some("ops[0].args.at"),
        &fixture.root,
    );
    fixture.cleanup();

    let unsupported = staged_code_fixture(
        "error-provider-unavailable",
        b"Procedure Base()\nEndProcedure\n",
    );
    fs::write(
            unsupported.root.join("v8project.yaml"),
            "format: DESIGNER\nsource-set:\n  - name: main\n    type: EXTERNAL_DATA_PROCESSORS\n    path: src\n",
        )
        .unwrap();
    let context = WorkspaceContext {
        cwd: unsupported.root.clone(),
        workspace_root: unsupported.root.clone(),
        cache_root: unsupported.root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            &unsupported.source,
            SourceSetKind::ExternalProcessor,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "staged-code-test-provider",
    )
    .unwrap();
    let actor = crate::infrastructure::source_workspace::new_actor(identity, context).unwrap();
    let binding = actor
        .bind_provider_root("main", &unsupported.source)
        .unwrap();
    let admission = actor
        .admit_apply(
            &binding,
            None,
            true,
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                Duration::from_secs(5),
            ),
            &unica_work_control::cancellation::CancellationToken::new(),
        )
        .unwrap();
    let provider = admission
        .code_planning_authority(&binding)
        .err()
        .expect("unsupported admitted source unexpectedly issued Code planning authority");
    assert_closed_code_error(
        &provider,
        ApplyPlanErrorKind::ProviderUnavailable,
        None,
        &unsupported.root,
    );
    unsupported.cleanup();

    let mut effects = unica_source_contract::proposal::PlannedApplyEffects::default();
    effects.append(DomainEvent::new(
        DomainEventKind::ModuleChanged,
        "main:CommonModule.Sample",
    ));
    effects.append(DomainEvent::new(
        DomainEventKind::FormChanged,
        "main:CommonForm.Main",
    ));
    effects.append(DomainEvent::new(
        DomainEventKind::ModuleChanged,
        "main:CommonModule.Sample",
    ));
    assert_eq!(
        effects.into_events(),
        vec![
            DomainEvent::new(DomainEventKind::ModuleChanged, "main:CommonModule.Sample"),
            DomainEvent::new(DomainEventKind::FormChanged, "main:CommonForm.Main"),
        ]
    );
}

fn assert_closed_code_error(
    error: &ApplyPlanError,
    kind: ApplyPlanErrorKind,
    path: Option<&str>,
    physical_root: &Path,
) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.path(), path);
    let rendered = error.to_string();
    let canonical_root = fs::canonicalize(physical_root).unwrap();
    for forbidden in [
        physical_root.display().to_string(),
        canonical_root.display().to_string(),
        "staged-code-test-provider".to_owned(),
        "unica.code.patch".to_owned(),
        "provider profile".to_owned(),
    ] {
        assert!(
            !rendered.contains(&forbidden),
            "closed error leaked `{forbidden}`: {rendered}"
        );
    }
}

struct StagedCodeFixture {
    root: PathBuf,
    source: PathBuf,
    actor: Arc<WorkspaceActor>,
    binding: ProviderRootBinding,
}

impl StagedCodeFixture {
    fn cleanup(self) {
        let _ = fs::remove_dir_all(self.root);
    }
}

fn staged_code_fixture(name: &str, sample_module: &[u8]) -> StagedCodeFixture {
    const MD: &str = "http://v8.1c.ru/8.3/MDClasses";
    let root = temp_root(&format!("staged-code-{name}"));
    let source = root.join("src");
    fs::create_dir_all(source.join("Ext")).unwrap();
    fs::create_dir_all(source.join("CommonModules/Sample/Ext")).unwrap();
    fs::create_dir_all(source.join("CommonModules/Other/Ext")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            source.join("Configuration.xml"),
            format!(
                "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><Configuration uuid=\"aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa\"><Properties><Name>Main</Name></Properties><ChildObjects><CommonModule>Sample</CommonModule><CommonModule>Other</CommonModule></ChildObjects></Configuration></MetaDataObject>"
            ),
        )
        .unwrap();
    for (module, uuid) in [
        ("Sample", "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"),
        ("Other", "cccccccc-cccc-cccc-cccc-cccccccccccc"),
    ] {
        fs::write(
                source.join(format!("CommonModules/{module}.xml")),
                format!(
                    "<MetaDataObject xmlns=\"{MD}\" version=\"2.20\"><CommonModule uuid=\"{uuid}\"><Properties><Name>{module}</Name></Properties></CommonModule></MetaDataObject>"
                ),
            )
            .unwrap();
    }
    fs::write(
        source.join("CommonModules/Sample/Ext/Module.bsl"),
        sample_module,
    )
    .unwrap();
    fs::write(
        source.join("CommonModules/Other/Ext/Module.bsl"),
        b"Procedure Other()\nEndProcedure\n",
    )
    .unwrap();
    let context = WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            &source,
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "staged-code-test-provider",
    )
    .unwrap();
    let actor =
        Arc::new(crate::infrastructure::source_workspace::new_actor(identity, context).unwrap());
    let binding = actor.bind_provider_root("main", &source).unwrap();
    StagedCodeFixture {
        root,
        source,
        actor,
        binding,
    }
}

fn staged_code_actor(root: &Path, source: &Path) -> Arc<WorkspaceActor> {
    let context = WorkspaceContext {
        cwd: root.to_path_buf(),
        workspace_root: root.to_path_buf(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    };
    let identity = WorkspaceIdentity::new(
        &context,
        [WorkspaceSourceSetInput::new(
            "main",
            source,
            SourceSetKind::Configuration,
            SourceFormat::PlatformXml,
            SourceProfile::platform_xml_8_3_27_format_2_20(),
        )],
        "staged-code-test-provider",
    )
    .unwrap();
    Arc::new(crate::infrastructure::source_workspace::new_actor(identity, context).unwrap())
}

fn staged_code_admission(
    fixture: &StagedCodeFixture,
    dry_run: bool,
) -> unica_workspace::actor::ApplyAdmission {
    fixture
        .actor
        .admit_apply(
            &fixture.binding,
            None,
            dry_run,
            unica_work_control::provider_deadline::ProviderDeadline::from_budget(
                Duration::from_secs(5),
            ),
            &unica_work_control::cancellation::CancellationToken::new(),
        )
        .unwrap()
}

fn plan_admitted_code(
    admission: &unica_workspace::actor::ApplyAdmission,
    binding: &ProviderRootBinding,
    operations: &[CodePlanOperation],
) -> Result<
    (
        unica_workspace::apply::ApplyStagedState,
        unica_source_contract::proposal::PlannedApplyEffects,
    ),
    ApplyPlanError,
> {
    let staged = admission.staged_state().unwrap();
    let authority = admission.code_planning_authority(binding)?;
    plan_code_batch_owned_for_test(staged, authority, operations)
}

fn assert_v12_c1_operation_parity(
    name: &str,
    before: &[u8],
    v12_args: Map<String, Value>,
    operation: CodePlanOperation,
    repeat_is_noop: bool,
) {
    let v12 = staged_code_fixture(&format!("parity-{name}-v12"), before);
    let c1 = staged_code_fixture(&format!("parity-{name}-c1"), before);
    let context = WorkspaceContext {
        cwd: v12.root.clone(),
        workspace_root: v12.root.clone(),
        cache_root: v12.root.join(".build/unica"),
        workspace_epoch: 1,
    };

    let v12_first = patch_inner(&v12_args, &context, PatchMode::Apply);
    assert!(
        v12_first.outcome.ok,
        "{name}: {:?}",
        v12_first.outcome.errors
    );
    assert!(!v12_first.data.as_ref().unwrap().no_op, "{name}");

    let admission = staged_code_admission(&c1, false);
    let (mut state, effects) =
        plan_admitted_code(&admission, &c1.binding, std::slice::from_ref(&operation))
            .unwrap_or_else(|error| panic!("{name}: {error:?}"));
    let relative = Path::new("CommonModules/Sample/Ext/Module.bsl");
    assert_eq!(
        state.read(relative).unwrap().unwrap(),
        fs::read(v12.source.join(relative)).unwrap(),
        "{name}"
    );
    assert_eq!(effects.events().len(), 1, "{name}");
    c1.actor
        .publish_prepared_apply(admission.prepare_with_effects(state, effects).unwrap())
        .unwrap();

    let v12_repeat = patch_inner(&v12_args, &context, PatchMode::Apply);
    let repeat_admission = staged_code_admission(&c1, true);
    let c1_repeat = plan_admitted_code(
        &repeat_admission,
        &c1.binding,
        std::slice::from_ref(&operation),
    );
    if repeat_is_noop {
        assert!(
            v12_repeat.outcome.ok,
            "{name}: {:?}",
            v12_repeat.outcome.errors
        );
        assert!(v12_repeat.data.unwrap().no_op, "{name}");
        let (state, effects) = c1_repeat.unwrap_or_else(|error| panic!("{name}: {error:?}"));
        assert!(state.planned_changes().is_empty(), "{name}");
        assert!(effects.events().is_empty(), "{name}");
    } else {
        assert!(!v12_repeat.outcome.ok, "{name}");
        assert_eq!(
            c1_repeat.unwrap_err().kind(),
            ApplyPlanErrorKind::NotFound,
            "{name}"
        );
    }
    assert_eq!(
        fs::read(v12.source.join(relative)).unwrap(),
        fs::read(c1.source.join(relative)).unwrap(),
        "{name}"
    );
    v12.cleanup();
    c1.cleanup();
}

fn staged_insert(
    at: &str,
    text: &str,
    selector: Option<CodeSelector>,
    position: Option<CodePosition>,
) -> CodePlanOperation {
    CodePlanOperation::Insert(CodeInsertArgs {
        at: unica_address::address::QualifiedAddress::parse(at).unwrap(),
        text: text.to_string(),
        selector,
        position,
    })
}

fn staged_replace(at: &str, text: &str, anchor: Option<&str>) -> CodePlanOperation {
    CodePlanOperation::Replace(CodeReplaceArgs {
        at: unica_address::address::QualifiedAddress::parse(at).unwrap(),
        text: text.to_string(),
        anchor: anchor.map(str::to_string),
    })
}

fn write_locked_support_state(source: &Path) {
    fs::create_dir_all(source.join("Ext")).unwrap();
    fs::write(
        source.join("Ext/ParentConfigurations.bin"),
        concat!(
            "\u{feff}{6,0,1,dddddddd-dddd-dddd-dddd-dddddddddddd,0,",
            "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee,\"1.0\",\"Vendor\",",
            "\"VendorConf\",3,1,0,aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa,0,0,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb,2,0,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc,",
            "cccccccc-cccc-cccc-cccc-cccccccccccc}"
        ),
    )
    .unwrap();
}

fn snapshot_staged_code_tree(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
    if !root.exists() {
        return Vec::new();
    }
    let mut pending = vec![root.to_path_buf()];
    let mut observed = Vec::new();
    while let Some(path) = pending.pop() {
        let relative = path.strip_prefix(root).unwrap().to_path_buf();
        let metadata = fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            observed.push((relative, None));
            let mut children = fs::read_dir(&path)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            children.sort();
            pending.extend(children.into_iter().rev());
        } else {
            observed.push((relative, Some(fs::read(path).unwrap())));
        }
    }
    observed
}

fn patch_args(
    source_set: &str,
    metadata_path: &str,
    method: &str,
    content: &str,
) -> Map<String, Value> {
    patch_args_for_selector(
        source_set,
        metadata_path,
        json!({"method": method}),
        "after",
        content,
    )
}

fn patch_args_for_selector(
    source_set: &str,
    metadata_path: &str,
    selector: Value,
    position: &str,
    content: &str,
) -> Map<String, Value> {
    let mut args = arguments(selector, position);
    args.insert("sourceSet".to_string(), json!(source_set));
    args.insert("metadataPath".to_string(), json!(metadata_path));
    args.insert("content".to_string(), json!(content));
    args
}

fn temp_context(name: &str) -> WorkspaceContext {
    let root = temp_root(&format!("code-patch-{name}"));
    fs::create_dir_all(root.join("src/CommonModules")).unwrap();
    fs::write(
        root.join("v8project.yaml"),
        "format: DESIGNER\nsource-set:\n  - name: main\n    type: CONFIGURATION\n    path: src\n",
    )
    .unwrap();
    fs::write(
            root.join("src/Configuration.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><Configuration/></MetaDataObject>"#,
        )
        .unwrap();
    fs::write(
            root.join("src/CommonModules/Sample.xml"),
            r#"<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20"><CommonModule><Properties><Name>Sample</Name></Properties></CommonModule></MetaDataObject>"#,
        )
        .unwrap();
    WorkspaceContext {
        cwd: root.clone(),
        workspace_root: root.clone(),
        cache_root: root.join(".build/unica"),
        workspace_epoch: 1,
    }
}

fn temp_root(name: &str) -> std::path::PathBuf {
    let nonce = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "unica-{name}-{}-{nanos}-{nonce}",
        std::process::id()
    ))
}
