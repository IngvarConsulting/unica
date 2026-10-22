"""Страж состава: ворота, профили nextest и размеры наборов согласованы.

Отбор объявлен профилями: `pr` пропускает только `small`, очередь и `main` —
всё, кроме `large` и временного каталога `nightly-only`, релиз гоняет всё,
`large` — названные тесты ёмкости и нагрузки ReceiptLedger и карты исходников, а на
Windows — весь набор. Меняется этот страж осознанно, вместе с выражениями профилей.
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
    ("unica_coder", "infrastructure::daemon::receipt_runtime_tests::"): (
        "crates/unica-coder/tests/platform/receipt_runtime.rs",
        "infrastructure::daemon::receipt_runtime_tests::",
    ),
    ("selection", ""): (
        "crates/unica-source-integration-tests/tests/selection.rs",
        "",
    ),
    ("unica_workspace", "selection_evidence::tests::"): (
        "crates/unica-workspace/src/selection_evidence.rs", "selection_evidence::",
    ),
}

# Expected command is independent of the runner's production constant.
UREQ_SELECTION = (
    "--manifest-path", "vendor/ureq/Cargo.toml", "--locked", "--lib",
    "--features", "json,serde_json/preserve_order,serde_json/float_roundtrip",
    "--config-file", "vendor/ureq/.config/nextest.toml", "--target-dir", "target",
    "-E", "test(/^completed_response_recycling::/)",
)


# Integration targets whose root tests stay in the nightly-only catalogue
# until full nightly shows them green on Windows (#1835, #1820), with the
# source file that declares them.
NIGHTLY_ONLY_TARGETS = {
    "platform_project_health": "crates/unica-coder/tests/platform/project_health.rs",
    "v13_workspace_bootstrap": "crates/unica-coder/tests/platform/v13_canonical_symlinked_workspace.rs",
}


def exact_groups(expression: str, label: str, targets: set[str]) -> dict[str, list[str]]:
    """`(binary(T) & (test(/^name$/) | ...)) | ...`: every declared target once, exact sorted names."""
    group = re.compile(r"\(\s*binary\(([A-Za-z0-9_-]+)\)\s*&\s*\((.*?)(?<!/)\)\s*\)", re.S)
    result = {}
    end = 0
    for match in group.finditer(expression):
        between = expression[end:match.start()].strip()
        if between != ("" if not result else "|"):
            raise ValueError(f"{label} must consist of declared binary/test groups")
        target, body = match.groups()
        if target not in targets or target in result:
            raise ValueError(f"{label} has an unknown or repeated target")
        names = []
        for term in body.strip().split("|"):
            parsed = re.fullmatch(r"\s*test\(/\^([\w:]+)\$/\)\s*", term)
            if parsed is None:
                raise ValueError(f"{label} members must be exact anchored test names")
            names.append(parsed[1])
        if not names or names != sorted(set(names)):
            raise ValueError(f"{label} members must be unique and sorted within each target")
        result[target] = names
        end = match.end()
    if expression[end:].strip() or set(result) != targets:
        raise ValueError(f"{label} must name every declared target exactly once")
    return result


def large_groups(expression: str) -> dict[str, list[str]]:
    """Exact named tests remain bound to their actual Cargo binary owners."""
    result = exact_groups(expression, "large", {"daemon_receipt_ledger", "unica_coder", "unica_workspace", "selection"})
    for target, names in result.items():
        for name in names:
            if target == "daemon_receipt_ledger":
                if "::" in name:
                    raise ValueError("large ledger tests must belong to the contract root")
            elif not any(
                binary == target and name.startswith(prefix) and "::" not in name[len(prefix):]
                for binary, prefix in LARGE_LIBRARY_MODULES
            ):
                raise ValueError("large library tests must belong to their declared binary and module")
    return result


def nightly_only_groups(expression: str) -> dict[str, list[str]]:
    """Root tests of the declared integration targets, no module paths."""
    result = exact_groups(expression, "nightly-only", set(NIGHTLY_ONLY_TARGETS))
    if any("::" in name for names in result.values() for name in names):
        raise ValueError("nightly-only members must be root tests of their target")
    return result


def attributed_test_functions(source: str) -> set[str]:
    """Read test attributes and module ownership from Rust syntax, not comments."""
    return set(test_function_attributes(source))


def test_function_attributes(source: str) -> dict[str, list[str]]:
    """Every `#[test]` function with its compacted outer attributes."""
    encoded = source.encode()
    tree = Parser(Language(tree_sitter_rust.language())).parse(encoded)
    found = {}
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
                found["::".join((*modules, name))] = attributes
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

    def test_ledger_feature_selection_builds_no_library_test_target(self) -> None:
        # Признак включается одному пакету и одной цели — контракту. Проверки
        # обвязки сценариев идут в его бинаре, поэтому `--lib` (тестовая
        # сборка всего `unica-coder`) варианту не нужен.
        expected = (
            "-p", "unica-coder", "--features", "receipt-ledger-test-support",
            "--test", "daemon_receipt_ledger",
            "-E", "binary(daemon_receipt_ledger)",
        )
        self.assertEqual(self.run_tests.LEDGER_FEATURE_SELECTION, expected)
        self.assertEqual(self.run_tests.rust_selections("pr"), [("--workspace",)])
        for profile in ("queue", "main", "release", "all", "large"):
            with self.subTest(profile=profile):
                selections = self.run_tests.rust_selections(profile)
                self.assertEqual(selections, [("--workspace",), expected, UREQ_SELECTION])
                self.assertNotIn("--features", selections[0])
                self.assertNotIn("--workspace", selections[1])
                self.assertNotIn("--lib", selections[1])

    def test_scenario_harness_checks_are_tests_of_the_contract_binary(self) -> None:
        # Тела проверок обвязки — в библиотеке без `#[test]`; тест каждой из
        # них разворачивает бинарь контракта одним макросом. Список в макросе
        # единственный, и обе известные проверки в нём есть.
        coder = REPO_ROOT / "crates/unica-coder"
        harness = (coder / "tests/support/receipt_scenario/tests.rs").read_text(encoding="utf-8")
        self.assertEqual(attributed_test_functions(harness), set())
        listed = re.search(r"(?m)^harness_checks!\((.*?)\);", harness, re.S)
        self.assertIsNotNone(listed)
        checks = {name.strip() for name in listed[1].split(",") if name.strip()}
        self.assertEqual(len(checks), 34)
        self.assertIn("fresh_task_listener_after_restart_survives_its_prepare_barrier_release", checks)
        self.assertIn("successor_closes_and_joins_old_gate_waiters_without_cancelling_the_new_life", checks)
        ledger = (coder / "tests/daemon_receipt_ledger.rs").read_text(encoding="utf-8")
        self.assertIn('#[path = "daemon_receipt_ledger/receipt_scenario_harness.rs"]\nmod receipt_scenario_harness;', ledger)
        wrapper = (coder / "tests/daemon_receipt_ledger/receipt_scenario_harness.rs").read_text(encoding="utf-8")
        self.assertIn("unica_coder::receipt_scenario_harness_tests!();", wrapper)
        # Бинарь контракта — `kind(test)`: размер `medium` идёт из каталога,
        # особой переразметки у запускателя для обвязки нет.
        self.assertIn("kind(test)", self.config["profile"]["pr"]["default-filter"])
        self.assertFalse(hasattr(self.run_tests, "SCENARIO_TEST_PREFIX"))
        # Под признаком у библиотеки ровно три входа: замороженный фасад,
        # демон перезапуска и проверки обвязки. Вход проверки — без аргументов:
        # он только запускает проверку и ничего не передаёт внутрь.
        lib = (coder / "src/lib.rs").read_text(encoding="utf-8")
        gated = re.findall(
            r'#\[cfg\(feature = "receipt-ledger-test-support"\)\]\s*(?:#\[doc\(hidden\)\]\s*)?'
            r"pub (?:mod (\w+);|use [\w:]+ as (\w+);)",
            lib,
        )
        self.assertEqual(
            sorted(name for pair in gated for name in pair if name),
            ["frontend_restart_test_support", "receipt_ledger_test_support", "receipt_scenario_harness_checks"],
        )
        self.assertEqual(lib.count("receipt-ledger-test-support"), len(gated))
        self.assertRegex(harness, r"pub fn \$check\(\)\s*\{\s*super::\$check\(\);\s*\}")

    def test_every_gate_has_a_nextest_profile_that_admits_its_sizes(self) -> None:
        profiles = self.config["profile"]

        large = profiles["large"]["default-filter"].strip()
        nightly_only = profiles["nightly-only"]["default-filter"].strip()
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
                    # Очередь и `main` — всё, кроме ночного яруса и каталога
                    # «только ночь», теми же выражениями.
                    self.assertEqual(profiles[gate]["default-filter"].strip(),
                                     f"not (\n{large}\n) & not (\n{nightly_only}\n)")
        self.assertEqual(set(self.run_tests.PROFILES), {"all", "large", *GATES})
        self.assertEqual(self.run_tests.nextest_profile("large"), "nightly-full")
        self.assertEqual(profiles["nightly-full"]["default-filter"], "all()")
        self.assertEqual(profiles["nightly-full"]["overrides"], profiles["large"]["overrides"])
        # Ночной ярус на ubuntu и macOS — только названные тесты своих целей;
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
            if "--manifest-path" in command:
                self.assertEqual(command, ["cargo", "nextest", "run", *UREQ_SELECTION, "--profile", "ureq-recycling"])
                self.assertNotIn("--no-tests=pass", command, "the named vendor regressions must never be empty")
            else:
                self.assertNotIn("--no-tests=pass", command, "full nightly inventory must never be empty")

    def test_dependency_regressions_have_a_standalone_config_and_no_retry_at_admitting_gates(self) -> None:
        config = tomllib.loads((REPO_ROOT / "vendor/ureq/.config/nextest.toml").read_text(encoding="utf-8"))
        profile = config["profile"]["ureq-recycling"]
        self.assertEqual(profile["default-filter"], "test(/^completed_response_recycling::/)")
        self.assertEqual(profile["retries"], 0)
        self.assertFalse(profile["fail-fast"])
        self.assertEqual(profile["junit"]["path"], "junit.xml")
        self.assertEqual(profile["junit"]["report-skipped"], "none")
        self.assertEqual(self.run_tests.UREQ_RECYCLING_SELECTION, UREQ_SELECTION)
        self.assertEqual(self.run_tests.rust_selections("pr"), [("--workspace",)])
        for gate in ("queue", "main", "release", "all", "large"):
            with self.subTest(gate=gate):
                self.assertEqual(self.run_tests.rust_selections(gate)[2], UREQ_SELECTION)
                self.assertEqual(self.run_tests.rust_list_commands(gate)[2], [
                    "cargo", "nextest", "list", *UREQ_SELECTION, "--profile", "ureq-recycling",
                    "--run-ignored", "all", "--message-format", "json",
                ])
                command = self.run_tests.rust_commands(gate)[2]
                self.assertEqual(command, ["cargo", "nextest", "run", *UREQ_SELECTION, "--profile", "ureq-recycling"])
                self.assertNotIn("--no-tests=pass", command)
                self.assertEqual(self.run_tests.nextest_junit(gate, UREQ_SELECTION), REPO_ROOT / "vendor/ureq/target/nextest/ureq-recycling/junit.xml")

    def test_large_tier_names_only_attributed_tests_of_the_declared_targets(self) -> None:
        """Ярус объявлен именами: переименованный тест выпал бы из ночи молча."""
        large = self.config["profile"]["large"]["default-filter"]
        groups = large_groups(large)
        self.assertEqual(sum(map(len, groups.values())), large.count("test("))
        ledger = (REPO_ROOT / "crates" / "unica-coder" / "tests/daemon_receipt_ledger.rs").read_text(encoding="utf-8")
        declared_by_target = {"daemon_receipt_ledger": attributed_test_functions(ledger), "unica_coder": set(), "unica_workspace": set(), "selection": set()}
        for (binary, _), (path, module) in LARGE_LIBRARY_MODULES.items():
            source = (REPO_ROOT / path).read_text(encoding="utf-8")
            declared_by_target[binary] |= {module + name for name in attributed_test_functions(source)}
        for target, names in groups.items():
            declared = declared_by_target[target]
            for name in names:
                with self.subTest(target=target, test=name):
                    self.assertIn(name, declared, "large member must have a test attribute in its declared module")

    def test_nightly_only_catalogue_names_runnable_root_tests_of_its_targets(self) -> None:
        """Каталог «только ночь» — точные имена неигнорируемых тестов: `ignore`
        спрятал бы их и от ночи, а переименование выпало бы из неё молча."""
        expression = self.config["profile"]["nightly-only"]["default-filter"].strip()
        groups = nightly_only_groups(expression)
        self.assertEqual(sum(map(len, groups.values())), expression.count("test("))
        for target, names in groups.items():
            # The declared source must be what the target compiles, or the
            # catalogue would vouch for a file that no longer runs.
            included = Path(NIGHTLY_ONLY_TARGETS[target]).relative_to("crates/unica-coder/tests").as_posix()
            root = (REPO_ROOT / "crates/unica-coder/tests" / f"{target}.rs").read_text(encoding="utf-8")
            self.assertIn(f'include!("{included}");', root, f"{target} must compile its declared source")
            declared = test_function_attributes((REPO_ROOT / NIGHTLY_ONLY_TARGETS[target]).read_text(encoding="utf-8"))
            for name in names:
                with self.subTest(target=target, test=name):
                    self.assertIn(name, declared, "nightly-only member must have a test attribute in its target")
                    self.assertFalse(any(re.search(r"\bignore\b", attribute) for attribute in declared[name]),
                                     "nightly-only member must not be ignored, also not through cfg_attr")
        name = groups["v13_workspace_bootstrap"][0]
        exact = f"test(/^{name}$/)"
        for replacement in [f"test(/^{name}/)", f"test(/^module::{name}$/)", exact + " | " + exact]:
            with self.subTest(replacement=replacement):
                with self.assertRaises(ValueError):
                    nightly_only_groups(expression.replace(exact, replacement, 1))
        with self.assertRaises(ValueError):
            nightly_only_groups(expression.replace("binary(v13_workspace_bootstrap)", "binary(unica_coder)", 1))
        self.assertEqual(test_function_attributes('#[test]\n#[ignore = "why"]\nfn quiet() {}\n#[test]\nfn loud() {}'),
                         {"quiet": ["#[test]", '#[ignore="why"]'], "loud": ["#[test]"]})

    def test_large_filter_rejects_broad_wrong_target_and_duplicate_members(self) -> None:
        expression = self.config["profile"]["large"]["default-filter"].strip()
        name = large_groups(expression)["unica_coder"][-1]
        exact = f"test(/^{name}$/)"
        for replacement in [
            f"test(/^{name}/)",
            "test(/^infrastructure::source_selection_evidence::tests::/)",
            "test(/^thirty_two_lazy_cancel_actor_batch_finishes_within_125ms$/)",
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
            for job in ("unica-ci", "rust-os")
            for step in release["jobs"][job]["steps"]
            if step.get("uses", "").startswith("taiki-e/install-action@") and "cargo-nextest@" in step["with"]["tool"]
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
        self.assertEqual({size for _, size, _ in self.run_tests.PYTHON_SUITES}, {"small"})

    def test_real_agent_suite_is_absent_from_every_gate(self) -> None:
        self.assertNotIn("tests/agent_evaluation", self.run_tests.python_suite_names())
        for gate in self.run_tests.PROFILES:
            with self.subTest(gate=gate):
                self.assertNotIn(
                    "tests/agent_evaluation",
                    {entry["suite"] for entry in self.run_tests.python_matrix(gate)},
                )
                self.assertFalse(
                    any("tests/agent_evaluation" in command for command in self.run_tests.python_commands(gate))
                )
        with self.assertRaises(ValueError):
            self.run_tests.python_commands("all", suite="tests/agent_evaluation")


# Стражи фасада ReceiptLedger test-support: заморозка ABI и отрицательные
# проверки полномочий. До #1895 они жили `#[cfg(test)]` в библиотеке и не
# исполнялись ни одним вариантом. Список закрыт: пропавший страж роняет
# эту проверку, а не сужает отбор молча.
FACADE_GUARDS = (
    "authority_wrappers_match_the_frozen_application_vectors",
    "crash_is_a_staged_control_and_does_not_mint_receipt_transition_evidence",
    "every_malformed_envelope_case_is_routed_individually_through_the_production_pipeline",
    "facade_and_non_owner_helpers_have_no_direct_receipt_ledger_authority",
    "facade_authority_guard_audits_mixed_cfg_subtrees",
    "facade_authority_guard_does_not_concatenate_unrelated_macro_literals",
    "facade_authority_guard_fails_closed_on_dynamic_string_construction",
    "facade_authority_guard_ignores_exact_cfg_test_associated_items",
    "facade_authority_guard_ignores_only_exact_cfg_test_subtrees",
    "facade_authority_guard_reconstructs_nested_concat_literals",
    "facade_authority_guard_reconstructs_split_concat_literals",
    "facade_authority_guard_rejects_production_string_literals",
    "facade_authority_guard_scans_every_named_production_source",
    "facade_source_keeps_the_exact_abi_and_has_no_production_authority",
    "rust_authority_guard_finds_store_identifiers_inside_formatting_macros",
    "scenario_driver_cannot_mint_runtime_telemetry_events",
    "scenario_wire_is_closed_at_the_scenario_and_action_levels",
)


class FacadeGuardInventoryTests(unittest.TestCase):
    def test_facade_guards_are_contract_tests_the_receipt_lane_runs(self) -> None:
        coder = REPO_ROOT / "crates/unica-coder"
        guards = (coder / "tests/daemon_receipt_ledger/facade_guards.rs").read_text(encoding="utf-8")
        self.assertEqual(sorted(attributed_test_functions(guards)), sorted(FACADE_GUARDS))
        # Страж исполняется, а не только объявлен: ни `#[ignore]`, ни `cfg`
        # не выключают его молча — у каждого ровно один атрибут `#[test]`.
        for name in FACADE_GUARDS:
            with self.subTest(guard=name):
                attributes = re.search(rf"((?:^[ \t]*#\[[^\n]*\]\n)*)fn {name}\(", guards, re.M)
                self.assertIsNotNone(attributes)
                self.assertEqual(attributes[1].strip(), "#[test]")
        ledger = (coder / "tests/daemon_receipt_ledger.rs").read_text(encoding="utf-8")
        module = re.search(r'((?:^#\[[^\n]*\]\n)*)mod facade_guards;', ledger, re.M)
        self.assertIsNotNone(module)
        self.assertEqual(module[1].strip(), '#[path = "daemon_receipt_ledger/facade_guards.rs"]')
        # Тестовый модуль в самом фасаде снова собирался бы только в
        # тестовой библиотеке, которую вариант Receipt не исполняет.
        facade = (coder / "src/receipt_ledger_test_support.rs").read_text(encoding="utf-8")
        self.assertNotIn("#[cfg(test)]", facade)
        self.assertEqual(attributed_test_functions(facade), set())
        selection = load_run_tests().LEDGER_FEATURE_SELECTION
        self.assertEqual(selection[selection.index("--test") + 1], "daemon_receipt_ledger")
        self.assertIn("binary(daemon_receipt_ledger)", selection[selection.index("-E") + 1])


if __name__ == "__main__":
    unittest.main()
