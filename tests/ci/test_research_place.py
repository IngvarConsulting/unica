"""Исследовательские тесты живут в своём месте и в ворота не попадают.

Класс теста задаёт его место: `crates/unica-research` (цели `repro`,
`platform`) и `tests/research/{repro,platform}`. Регресс лежит в остальном
дереве и не носит номера issue в имени; `#[ignore]` вне исследовательского
места допустим только у вспомогательного процесса, которого запускает другой
тест.
"""

from __future__ import annotations

import importlib.util
import re
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
RESEARCH_CRATE = Path("crates/unica-research")
RESEARCH_PYTHON = Path("tests/research")
ALLOWED_TEST_SUPPORT = {"test-support"}

sys.path.insert(0, str(REPO_ROOT / "scripts" / "ci"))
import rule_marks  # noqa: E402

IGNORE = re.compile(r'#\[\s*ignore\s*(?:=\s*"((?:[^"\\]|\\.)*)")?\s*\]')
FUNCTION = re.compile(r"\bfn\s+(\w+)")
TEST_ATTRIBUTE = re.compile(r"#\[\s*(?:\w+::)*test\b[^\]]*\]")
ISSUE_IN_NAME = re.compile(r"(?:^|_)issue_?\d+", re.IGNORECASE)
HELPER_REASON = "helper process"

# Долг на момент введения стража: тесты, которые ещё не переехали в
# исследовательское место. Перечень только сокращается — переехавший тест
# убирается отсюда, новый сюда не добавляется.
IGNORE_DEBT = frozenset({
    ("crates/unica-bsl-types/src/tests.rs", "the_catalog_of_an_installation"),
    ("crates/unica-coder/tests/platform/persistent_provider_cwd.rs", "helper_inherited_cwd_subprocess_fixture"),
    ("crates/unica-coder-daemon/src/infrastructure/daemon/lsp_workspaces/tests.rs",
     "a_large_common_module_comes_whole_in_one_request"),
    ("crates/unica-documentation-sources/src/platform_help/provider.rs", "an_installation_lists_its_global_context"),
    ("crates/unica-documentation-sources/src/platform_help/real_installation.rs",
     "real_installation_answers_navigation_link_question"),
    ("crates/unica-documentation-sources/src/platform_help/real_installation.rs",
     "every_container_of_a_real_installation_parses"),
    ("crates/unica-documentation-sources/src/platform_help/real_installation.rs",
     "real_installation_answers_a_locale_it_does_not_ship"),
    ("crates/unica-documentation-sources/src/platform_help/retrieval_gate.rs",
     "retrieval_gate_finds_every_golden_query_within_budget"),
    ("crates/unica-documentation-sources/tests/platform/platform_help_oracle.rs",
     "checked_platform_event_catalog_matches_complete_8_3_27_vendor_oracle"),
    ("crates/unica-documentation-sources/tests/platform/platform_help_oracle.rs",
     "vendor_oracle_rejects_a_matching_hbk_from_the_wrong_installation_version"),
    ("crates/unica-documentation-sources/tests/platform/platform_help_oracle.rs",
     "bilingual_template_exceptions_are_exact_and_source_guarded"),
    ("crates/unica-documentation-sources/tests/platform/platform_help_oracle.rs",
     "every_vendor_event_markup_page_belongs_to_one_closed_partition_category"),
    ("crates/unica-documentation-sources/tests/platform/platform_help_oracle.rs",
     "regenerate_checked_platform_event_catalog"),
    ("crates/unica-product/tests/v13_workspace_bootstrap.rs",
     "canonical_stdio_bootstraps_an_empty_workspace_before_address_discovery"),
    ("crates/unica-workspace/src/selection_evidence.rs", "repeated_exact_observations_do_not_exhaust_a_pass_work_ceiling"),
})
ISSUE_NAME_DEBT = frozenset({
    "tests/dev/test_verify_issue_76_roundtrip.py",
})


def ignored_tests(source: str) -> list[tuple[str, str]]:
    """`(функция, причина)` каждого `#[ignore]` в коде, без строк и комментариев."""
    code = rule_marks._blank_code(source)
    found = []
    for match in re.finditer(r"#\[\s*ignore\b", code):
        attribute = IGNORE.match(source, match.start())
        function = FUNCTION.search(code, match.end())
        found.append((function.group(1) if function else "?", (attribute.group(1) or "") if attribute else ""))
    return found


def rust_test_names(source: str) -> list[str]:
    code = rule_marks._blank_code(source)
    names = []
    for match in TEST_ATTRIBUTE.finditer(code):
        function = FUNCTION.search(code, match.end())
        if function:
            names.append(function.group(1))
    return names


def python_test_names(source: str) -> list[str]:
    return re.findall(r"^\s*(?:async\s+)?def\s+(test\w*)", source, re.MULTILINE)


def outside_research(pattern: str) -> list[Path]:
    return [
        path for path in sorted(REPO_ROOT.glob(pattern))
        if not path.relative_to(REPO_ROOT).is_relative_to(RESEARCH_CRATE)
        and not path.relative_to(REPO_ROOT).is_relative_to(RESEARCH_PYTHON)
        and "target" not in path.relative_to(REPO_ROOT).parts
    ]


def manifest(path: Path) -> dict:
    return tomllib.loads(path.read_text(encoding="utf-8"))


def dependency_tables(document: dict) -> list[dict]:
    tables = [document.get(name, {}) for name in ("dependencies", "dev-dependencies", "build-dependencies")]
    for target in document.get("target", {}).values():
        tables += [target.get(name, {}) for name in ("dependencies", "dev-dependencies", "build-dependencies")]
    return tables


class ResearchPlaceTests(unittest.TestCase):
    def test_the_research_crate_is_its_own_workspace_outside_the_product_one(self) -> None:
        # Каждая команда конвейера собирает `--workspace`: член пространства
        # попал бы в сборку, clippy и опись, а его `test-support` включился бы
        # у крейтов продукта в обычных прогонах.
        root = manifest(REPO_ROOT / "Cargo.toml")
        self.assertIn(RESEARCH_CRATE.as_posix(), root["workspace"].get("exclude", []))
        research = manifest(REPO_ROOT / RESEARCH_CRATE / "Cargo.toml")
        self.assertIn("workspace", research)
        self.assertIs(research["package"]["publish"], False)
        self.assertEqual(sorted(target["name"] for target in research["test"]), ["platform", "repro"])

    def test_no_crate_depends_on_the_research_crate(self) -> None:
        dependents = [
            path.relative_to(REPO_ROOT).as_posix()
            for path in outside_research("crates/*/Cargo.toml")
            if any("unica-research" in table for table in dependency_tables(manifest(path)))
        ]
        self.assertEqual(dependents, [])

    def test_the_research_crate_reaches_product_internals_only_through_test_support(self) -> None:
        research = manifest(REPO_ROOT / RESEARCH_CRATE / "Cargo.toml")
        for table in dependency_tables(research):
            for name, entry in table.items():
                if isinstance(entry, dict):
                    with self.subTest(dependency=name):
                        self.assertLessEqual(set(entry.get("features", [])), ALLOWED_TEST_SUPPORT)
        for path in sorted((REPO_ROOT / RESEARCH_CRATE).rglob("*.rs")):
            code = rule_marks._blank_code(path.read_text(encoding="utf-8"))
            with self.subTest(path=path.relative_to(REPO_ROOT).as_posix()):
                self.assertNotRegex(code, r"#\[\s*path\b")
                self.assertNotRegex(code, r"\binclude(?:_str|_bytes)?!\s*\(")

    def test_gate_suites_do_not_collect_research_tests(self) -> None:
        spec = importlib.util.spec_from_file_location("run_static_tests", REPO_ROOT / "scripts/ci/run-static-tests.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        collected = module.modules(REPO_ROOT, skip=set())
        self.assertTrue(collected)
        self.assertEqual([name for name in collected if name.startswith("tests.research")], [])

    def test_ignore_outside_research_is_only_a_helper_process(self) -> None:
        found = set()
        for path in outside_research("crates/**/*.rs"):
            relative = path.relative_to(REPO_ROOT).as_posix()
            for function, reason in ignored_tests(path.read_text(encoding="utf-8", errors="replace")):
                if not reason.startswith(HELPER_REASON):
                    found.add((relative, function))
        self.assertEqual(sorted(found - IGNORE_DEBT), [], "move these tests to crates/unica-research")
        self.assertEqual(sorted(IGNORE_DEBT - found), [], "remove fixed entries from IGNORE_DEBT")

    def test_an_issue_number_names_only_a_research_test(self) -> None:
        found = set()
        for path in outside_research("crates/**/*.rs"):
            relative = path.relative_to(REPO_ROOT).as_posix()
            found |= {f"{relative}::{name}" for name in rust_test_names(path.read_text(encoding="utf-8", errors="replace"))
                      if ISSUE_IN_NAME.search(name)}
        for path in outside_research("tests/**/test_*.py"):
            relative = path.relative_to(REPO_ROOT).as_posix()
            if ISSUE_IN_NAME.search(path.stem):
                found.add(relative)
            found |= {f"{relative}::{name}" for name in python_test_names(path.read_text(encoding="utf-8"))
                      if ISSUE_IN_NAME.search(name)}
        self.assertEqual(sorted(found - ISSUE_NAME_DEBT), [], "move these tests to the research place")
        self.assertEqual(sorted(ISSUE_NAME_DEBT - found), [], "remove fixed entries from ISSUE_NAME_DEBT")

    def test_build_profile_names_research_packages_off_the_root_versions(self) -> None:
        spec = importlib.util.spec_from_file_location("run_research", REPO_ROOT / "scripts/dev/run-research.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)

        def lock(*packages: tuple[str, str]) -> str:
            return "".join(f'[[package]]\nname = "{name}"\nversion = "{version}"\n\n' for name, version in packages)

        with tempfile.TemporaryDirectory() as temporary:
            root, research = Path(temporary) / "root.lock", Path(temporary) / "research.lock"
            root.write_text(lock(("serde", "1.0.200"), ("unica-domain", "0.15.0")), encoding="utf-8")
            research.write_text(
                lock(("serde", "1.0.201"), ("unica-domain", "0.15.0"), ("unica-research", "0.0.0")),
                encoding="utf-8",
            )
            self.assertEqual(module.drifted(root, research), ["serde: ['1.0.201'] vs root ['1.0.200']"])
            research.write_text(lock(("serde", "1.0.200"), ("unica-research", "0.0.0")), encoding="utf-8")
            self.assertEqual(module.drifted(root, research), [])

    def test_detection_reads_code_not_comments_or_strings(self) -> None:
        source = (
            '/// помечен `#[ignore]` в комментарии\n'
            'const TEXT: &str = "#[ignore]";\n'
            '#[test]\n#[ignore = "helper process: spawned by a parent"]\nfn helper() {}\n'
            '#[test]\n#[ignore]\nfn bare() {}\n'
            '#[test]\n#[ignore = "needs a platform"]\nfn measured() {}\n'
            '#[tokio::test]\nasync fn issue_1234_hangs() {}\n'
            '#[test]\nfn reissue_payload_keeps_bytes() {}\n'
        )
        self.assertEqual(
            ignored_tests(source),
            [("helper", "helper process: spawned by a parent"), ("bare", ""), ("measured", "needs a platform")],
        )
        self.assertEqual(
            [name for name in rust_test_names(source) if ISSUE_IN_NAME.search(name)],
            ["issue_1234_hangs"],
        )


if __name__ == "__main__":
    unittest.main()
