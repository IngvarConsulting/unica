"""Тест без возможности хоста виден пропуском, а не PASS (#1893).

Тест, которому нужна возможность хоста (симлинки, регистронезависимый том,
биты прав Unix), сообщает о её отсутствии через
`unica_process::test_support::skip_without_host_capability` и сразу выходит.
Раннер превращает такой PASS в пропуск с причиной и отвергает его на раннере,
где возможность обязательна (`scripts/ci/ci_rust_lanes.py`). Тест, которому
нужна возможность конкретной ОС, помечается
`#[cfg_attr(not(<ОС>), ignore = "host capability ...")]` там, где граница
платформы допускает OS-cfg (`tests/platform/`, фасады `src/platform/`);
в остальных местах — helper с именем возможности ОС.

Страж ищет ранний выход рядом с пробой возможности в обход helper. Места,
найденные до механизма, перечислены ниже. Их переводят пакетами по крейтам,
и каждый пакет удаляет свои строки: устаревшая строка роняет страж так же,
как новое место.
"""
from __future__ import annotations

import unittest

import host_capability_guard as guard

# Ранние выходы, существовавшие до механизма (#1893). Список только сокращается.
ALLOWLIST = frozenset({
    "crates/unica-coder/src/application/mod.rs::cf_edit_preserves_unix_mode_0600",
    "crates/unica-coder/src/application/mod.rs::cf_edit_rejects_symlink_configuration_without_touching_referent",
    "crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs::command_lifecycle_refuses_links_at_the_source_or_destination_module_route",
    "crates/unica-coder/tests/platform/code_intelligence_integration.rs::git_grep_scoped_preflight_refuses_invalid_routes_without_dispatch",
    "crates/unica-coder/tests/platform/code_patch_composition.rs::code_patch_symlink_module_is_rejected_during_preview",
    "crates/unica-workspace/src/actor_policy_contract_tests.rs::apply_policy_stable_deny_evidence_allows_unrelated_dry_run_and_real_publication",
})


class HostCapabilityGuardTests(unittest.TestCase):
    def test_no_new_early_exit_and_no_stale_allowlist_entry(self) -> None:
        found = guard.repository_offenses()
        self.assertEqual(sorted(found - ALLOWLIST), [], "test exits early on a host capability without the skip helper")
        self.assertEqual(sorted(ALLOWLIST - found), [], "converted tests must leave the allowlist")

    def test_detects_capability_exits_in_every_shape(self) -> None:
        shapes = {
            "let_else": "let Some(link) = create_file_symlink_for_test(&a, &b) else { return; };",
            "if_not": "if !can_swap_named_child_behind_retained_handle_for_test() {\n    return;\n}",
            "match_arm": "match outcome {\n    FileLinkFixtureOutcome::WindowsPrivilegeUnavailable => return,\n    _ => {}\n}",
            "raw_error": "if error.raw_os_error() == Some(1314) { return; }",
            "raw_symlink": "if std::os::unix::fs::symlink(&a, &b).is_err() { eprintln!(\"skip\"); return; }",
            "loop": "for case in cases {\n    if host_filesystem_case_sensitive(&root).unwrap() { continue; }\n}",
            "second_exit": "let Some(l) = create_file_symlink_for_test(&a, &b) else {\n"
                           "    skip_without_host_capability(HostCapability::Symlink, \"x\");\n    return;\n};\n"
                           "if l.is_err() { return; }",
        }
        for name, statement in shapes.items():
            with self.subTest(shape=name):
                source = "#[test]\nfn sample() {\n    " + statement + "\n    assert!(true);\n}\n"
                self.assertEqual(guard.offenses(source, "crates/demo/src/lib.rs"), ["crates/demo/src/lib.rs::sample"])

    def test_the_skip_helper_comments_strings_and_production_code_are_not_offenses(self) -> None:
        allowed = (
            "#[test]\nfn sample() {\n    if !can_swap_named_child_behind_retained_handle_for_test() {\n"
            "        skip_without_host_capability(\n            HostCapability::RenameUnderOpenHandle,\n"
            "            &format!(\"needs {}\", root.display()),\n        );\n        cleanup(&context);\n        return;\n    }\n}\n",
            "#[test]\nfn sample() {\n    let Some(link) = create_file_symlink_for_test(&a, &b) else {\n"
            "        skip_without_host_capability(HostCapability::Symlink, \"needs a privilege\");\n"
            "        return;\n    };\n}\n",
            "#[test]\nfn sample() {\n    // if !supports_retained_root_replacement_test() { return; }\n    run();\n}\n",
            "pub fn contract() -> Result<(), CapabilitySkip> {\n    if !can_swap_named_child_behind_retained_handle_for_test() {\n"
            "        return Err(CapabilitySkip { capability: \"rename-under-open-handle\", reason: \"x\" });\n    }\n    Ok(())\n}\n",
            "#[test]\nfn sample() {\n    let program = r#\"fn main() { if symlink(a, b).is_err() { return; } }\"#;\n}\n",
            "#[test]\nfn sample() {\n    let link = create_file_symlink_for_test(&a, &b).unwrap().unwrap();\n    assert!(link);\n}\n",
        )
        for source in allowed:
            with self.subTest(source=source):
                self.assertEqual(guard.offenses(source, "crates/demo/src/lib.rs"), [])
        production = "pub fn case_policy(root: &Path) -> bool {\n    if !host_filesystem_case_sensitive(root).unwrap() { return false; }\n    true\n}\n"
        self.assertEqual(guard.offenses(production, "crates/demo/src/policy.rs"), [])
        self.assertEqual(len(guard.offenses(production, "crates/demo/src/policy_tests.rs")), 1)


if __name__ == "__main__":
    unittest.main()
