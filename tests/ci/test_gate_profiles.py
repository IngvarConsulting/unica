"""Страж состава: ворота, профили nextest и размеры наборов согласованы.

Отбор объявлен профилями: `pr` пропускает только `small`, очередь и `main` —
всё, кроме `large`, релиз гоняет всё, `large` — названные тесты ёмкости и
нагрузки ReceiptLedger и карты исходников, а на Windows — весь набор. Меняется этот
страж осознанно, вместе с выражениями профилей.
"""

from __future__ import annotations

import importlib.util
import re
import tomllib
import unittest
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust


REPO_ROOT = Path(__file__).resolve().parents[2]
GATES = ("pr", "queue", "main", "release")
# Library modules whose named tests may enter the large tier, with the source
# file that declares them: map capacity checks and the real-time daemon run
# past the former operation deadlines (#1251).
LARGE_LIBRARY_MODULES = {
    "infrastructure::source_selection_evidence::tests::": "src/infrastructure/source_selection_evidence.rs",
    "infrastructure::daemon::runtime_v5::tests::": "src/infrastructure/daemon/runtime_v5/tests.rs",
}


def large_groups(expression: str) -> dict[str, list[str]]:
    """Only exact named tests of the two declared targets enter the large tier."""
    group = r"\(\s*binary\({target}\)\s*&\s*\((?P<{label}>.*?)\)\s*\)"
    pattern = (
        group.format(target="daemon_receipt_ledger", label="ledger")
        + r"\s*\|\s*"
        + group.format(target="unica_coder", label="selection")
    )
    matched = re.fullmatch(pattern, expression.strip(), re.S)
    if matched is None:
        raise ValueError("large must name only the declared ledger and library targets")
    result = {}
    for target, label in [("daemon_receipt_ledger", "ledger"), ("unica_coder", "selection")]:
        terms = matched[label].strip().split("|")
        names = []
        for term in terms:
            parsed = re.fullmatch(r"\s*test\(/\^([\w:]+)\$/\)\s*", term)
            if parsed is None:
                raise ValueError("large members must be exact anchored test names")
            name = parsed[1]
            if target == "unica_coder":
                if not any(
                    name.startswith(prefix) and "::" not in name[len(prefix):]
                    for prefix in LARGE_LIBRARY_MODULES
                ):
                    raise ValueError("large library tests must belong to a declared large module")
            elif "::" in name:
                raise ValueError("large ledger tests must belong to the contract root")
            names.append(name)
        if names != sorted(set(names)):
            raise ValueError("large members must be unique and sorted within each target")
        result[target] = names
    return result


def attributed_test_functions(source: str) -> set[str]:
    """Read test attributes and module ownership from Rust syntax, not comments."""
    encoded = source.encode()
    tree = Parser(Language(tree_sitter_rust.language())).parse(encoded)
    found = set()
    stack = [(tree.root_node, ())]
    while stack:
        parent, modules = stack.pop()
        attributes = []
        for child in parent.named_children:
            if child.type == "attribute_item":
                attributes.append(re.sub(r"\s+", "", encoded[child.start_byte:child.end_byte].decode()))
                continue
            if child.type == "mod_item":
                body = child.child_by_field_name("body")
                if body is not None:
                    name = child.child_by_field_name("name").text.decode()
                    stack.append((body, (*modules, name)))
            elif child.type == "function_item" and "#[test]" in attributes:
                name = child.child_by_field_name("name").text.decode()
                found.add("::".join((*modules, name)))
            attributes = []
    return found


def load_run_tests():
    spec = importlib.util.spec_from_file_location("run_tests", REPO_ROOT / "scripts" / "ci" / "run-tests.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class GateProfileCompositionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.config = tomllib.loads((REPO_ROOT / ".config" / "nextest.toml").read_text(encoding="utf-8"))
        self.run_tests = load_run_tests()

    def test_private_scenario_feature_selection_is_scoped_to_its_library_module(self) -> None:
        prefix = "infrastructure::daemon::runtime_v5::receipt_scenario_v5::tests::"
        expected = (
            "-p", "unica-coder", "--features", "receipt-ledger-test-support", "--lib",
            "-E", f"test(/^{prefix}/)",
        )
        self.assertEqual(self.run_tests.LEDGER_SCENARIO_LIBRARY, expected)
        self.assertEqual(self.run_tests.rust_selections("pr"), [("--workspace",)])
        for profile in ("queue", "main", "release", "all", "large"):
            with self.subTest(profile=profile):
                selections = self.run_tests.rust_selections(profile)
                self.assertEqual(selections[0], ("--workspace",))
                self.assertEqual(selections[2], expected)
                self.assertNotIn("--features", selections[0])
                self.assertNotIn("--workspace", selections[2])
        source = (REPO_ROOT / "crates/unica-coder/src/infrastructure/daemon/runtime_v5/receipt_scenario_v5/tests.rs").read_text()
        declared = attributed_test_functions(source)
        self.assertIn("fresh_task_listener_after_restart_survives_its_prepare_barrier_release", declared)
        self.assertIn("successor_closes_and_joins_old_gate_waiters_without_cancelling_the_new_life", declared)

    def test_every_gate_has_a_nextest_profile_that_admits_its_sizes(self) -> None:
        profiles = self.config["profile"]

        large = profiles["large"]["default-filter"].strip()
        for gate in GATES:
            with self.subTest(gate=gate):
                self.assertIn(gate, profiles)
                self.assertEqual(self.run_tests.nextest_profile(gate), gate)
                if gate == "pr":
                    # Pull request — только small: всё, что не объявлено medium.
                    self.assertTrue(profiles[gate]["default-filter"].startswith("not ("))
                elif gate == "release":
                    self.assertEqual(profiles[gate].get("default-filter"), "all()")
                else:
                    # Очередь и `main` — всё, кроме ночного яруса, тем же выражением.
                    self.assertEqual(profiles[gate]["default-filter"].strip(), f"not (\n{large}\n)")
        self.assertEqual(set(self.run_tests.PROFILES), {"all", "large", *GATES})
        # Ночной ярус на ubuntu и macOS — только названные тесты двух целей;
        # тот же список стоит и у срока `large`.
        large_groups(large)
        deadline = next(o for o in profiles["default"]["overrides"] if o.get("threads-required"))
        self.assertEqual(deadline["filter"].strip(), large)
        self.assertEqual(deadline["slow-timeout"], {"period": "900s", "terminate-after": 2})
        self.assertEqual(deadline["threads-required"], "num-cpus")
        # Ночью Windows гоняет всё, кроме детерминированной модели горизонта
        # нагрузки: она платформе безразлична и на двух ядрах не укладывается
        # в два срока `large`.
        self.assertEqual(
            profiles["large"]["overrides"],
            [{
                "platform": "cfg(windows)",
                "default-filter": "all() - test(/^deterministic_horizon_load_does_not_saturate$/)",
            }],
        )
        for command in self.run_tests.rust_commands("large"):
            self.assertIn("--no-tests=pass", command)

    def test_large_tier_names_only_attributed_tests_of_the_declared_targets(self) -> None:
        """Ярус объявлен именами: переименованный тест выпал бы из ночи молча."""
        large = self.config["profile"]["large"]["default-filter"]
        groups = large_groups(large)
        self.assertEqual(sum(map(len, groups.values())), large.count("test("))
        ledger = (REPO_ROOT / "crates" / "unica-coder" / "tests/daemon_receipt_ledger.rs").read_text(encoding="utf-8")
        declared_by_target = {"daemon_receipt_ledger": attributed_test_functions(ledger), "unica_coder": set()}
        for prefix, path in LARGE_LIBRARY_MODULES.items():
            source = (REPO_ROOT / "crates" / "unica-coder" / path).read_text(encoding="utf-8")
            # A `tests.rs` file is the module body itself; an inline module
            # contributes its own `tests::` segment.
            module = prefix if path.endswith("/tests.rs") else prefix.removesuffix("tests::")
            declared_by_target["unica_coder"] |= {module + name for name in attributed_test_functions(source)}
        for target, names in groups.items():
            declared = declared_by_target[target]
            for name in names:
                with self.subTest(target=target, test=name):
                    self.assertIn(name, declared, "large member must have a test attribute in its declared module")

    def test_large_filter_rejects_broad_wrong_target_and_duplicate_members(self) -> None:
        expression = self.config["profile"]["large"]["default-filter"].strip()
        name = large_groups(expression)["unica_coder"][-1]
        exact = f"test(/^{name}$/)"
        for replacement in [
            f"test(/^{name}/)",
            "test(/^infrastructure::source_selection_evidence::tests::/)",
            "test(/^wall_clock_writer_sustains_32_receipts_per_second_on_posix$/)",
            exact + " | " + exact,
        ]:
            with self.subTest(replacement=replacement):
                with self.assertRaises(ValueError):
                    large_groups(expression.replace(exact, replacement, 1))

    def test_large_source_binding_rejects_helpers_comments_and_wrong_modules(self) -> None:
        found = attributed_test_functions('''
            mod tests {
                fn helper() {}
                // #[test] fn comment() {}
                #[test] fn capacity() {}
            }
            mod other { #[test] fn capacity() {} }
        ''')
        self.assertEqual(found, {"tests::capacity", "other::capacity"})
        self.assertNotIn("tests::helper", found)
        self.assertNotIn("tests::comment", found)
        self.assertNotIn("tests::missing", found)
        self.assertNotIn("tests::capacity", attributed_test_functions(
            "mod other { #[test] fn capacity() {} }"
        ))

    def test_python_matrix_names_every_suite_of_the_seam_and_lanes_only_admitted_sizes(self) -> None:
        """Матрица ворот: каждый набор шва, полосатый — по допущенным размерам, не больше."""
        for gate in ("pr", "queue", "main", "release"):
            with self.subTest(gate=gate):
                matrix = self.run_tests.python_matrix(gate)
                self.assertEqual(sorted({entry["suite"] for entry in matrix}), sorted(
                    suite for suite, size, _ in self.run_tests.PYTHON_SUITES
                    if size in self.run_tests.ADMITTED[gate]))
                lanes = {entry["lane"] for entry in matrix if entry["suite"] in self.run_tests.LANED_SUITES}
                self.assertEqual(lanes, set(self.run_tests.ADMITTED[gate]))
                self.assertTrue(all(not entry["lane"] for entry in matrix if entry["suite"] not in self.run_tests.LANED_SUITES))

    def test_nextest_version_in_config_matches_the_workflow_install(self) -> None:
        """Одна версия исполнителя для CI и локального прогона."""
        import yaml

        release = yaml.safe_load((REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml").read_text(encoding="utf-8"))
        tools = next(
            step["with"]["tool"]
            for step in release["jobs"]["test-rust-platforms"]["steps"]
            if step.get("uses", "").startswith("taiki-e/install-action@")
        )
        installed = next(part.split("@")[1] for part in tools.split(",") if part.startswith("cargo-nextest@"))

        self.assertEqual(self.config["nextest-version"], {"recommended": installed})

    def test_default_profile_carries_the_small_deadline_for_everyone(self) -> None:
        """Срок на тест — то, чем размер держится честным; пока он один на всех."""
        default = self.config["profile"]["default"]

        self.assertEqual(default["slow-timeout"], {"period": "60s", "terminate-after": 2})
        self.assertEqual(default["junit"]["report-skipped"], "ignored")

    def test_python_suites_declare_a_size_the_gates_understand(self) -> None:
        sizes = set(self.run_tests.SIZES)

        self.assertEqual(set(self.run_tests.ADMITTED), set(self.run_tests.PROFILES))
        for gate, admitted in self.run_tests.ADMITTED.items():
            with self.subTest(gate=gate):
                self.assertTrue(set(admitted) <= sizes)
        for suite, size, _ in self.run_tests.PYTHON_SUITES:
            with self.subTest(suite=suite):
                self.assertIn(size, sizes)
                self.assertTrue((REPO_ROOT / suite).is_dir())
        self.assertEqual({size for _, size, _ in self.run_tests.PYTHON_SUITES}, {"small", "large"})

    def test_real_agent_suite_is_required_by_complete_gates(self) -> None:
        for gate in ("all", "release", "large"):
            self.assertIn("tests/agent_evaluation", {entry["suite"] for entry in self.run_tests.python_matrix(gate)})
            self.assertTrue(any("tests/agent_evaluation" in command for command in self.run_tests.python_commands(gate)))
        for gate in ("pr", "queue", "main"):
            self.assertNotIn("tests/agent_evaluation", {entry["suite"] for entry in self.run_tests.python_matrix(gate)})

    def test_large_agent_gate_really_selects_the_case_without_allure(self) -> None:
        import subprocess
        command, = self.run_tests.python_commands("large", suite="tests/agent_evaluation")
        result = subprocess.run([*command, "--plan-only"], cwd=REPO_ROOT, capture_output=True, text=True, check=True)
        self.assertEqual(result.stdout.splitlines(), [
            "test_dcs.DcsAgentAcceptanceTests.test_agent_uses_current_contract_without_retired_skills"])


if __name__ == "__main__":
    unittest.main()
