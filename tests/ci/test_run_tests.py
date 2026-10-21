"""Шов прогона тестов управляет наборами CI, кроме локальной агентной приёмки."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "run-tests.py"

# Expected command is independent of the runner's production constant.
UREQ_SELECTION = (
    "--manifest-path", "vendor/ureq/Cargo.toml", "--locked", "--lib",
    "--features", "json,serde_json/preserve_order,serde_json/float_roundtrip",
    "--config-file", "vendor/ureq/.config/nextest.toml", "--target-dir", "target",
    "-E", "test(/^completed_response_recycling::/)",
)


def load_module():
    spec = importlib.util.spec_from_file_location("run_tests", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {MODULE_PATH}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RunTestsSeamTests(unittest.TestCase):
    def test_documentation_selects_small_in_both_suites_without_changing_queue_profile(self) -> None:
        module = load_module()
        self.assertEqual(module.python_matrix("queue", docs_only=True), [
            {"suite": "tests/ci", "lane": "small", "slug": "ci-small"},
            {"suite": "tests/dev", "lane": "small", "slug": "dev-small"},
        ])
        for results in (None, Path("results")):
            for profile in ("pr", "queue"):
                for suite in ("tests/ci", "tests/dev"):
                    with self.subTest(profile=profile, suite=suite, results=results):
                        command, = module.python_commands(
                            profile, results=results, suite=suite, only_size="small", docs_only=True,
                        )
                        self.assertEqual(command[command.index("--admit") + 1], "small")
                        self.assertEqual(command[command.index("--profile") + 1], profile)
                        self.assertEqual("--scope-skip-medium" in command, profile == "queue")

    def test_documentation_override_cannot_shorten_other_profiles_or_run_rust(self) -> None:
        module = load_module()
        for profile in ("all", "main", "release", "large"):
            with self.subTest(profile=profile), self.assertRaises(ValueError):
                module.python_matrix(profile, docs_only=True)
        for ecosystem in ("rust", "all"):
            with self.subTest(ecosystem=ecosystem), self.assertRaises(ValueError):
                module.commands("queue", ecosystem, docs_only=True)
        with self.assertRaises(ValueError):
            module.python_commands("queue", only_size="medium", docs_only=True)

    def test_profile_all_runs_every_ci_managed_suite(self) -> None:
        """Команды закреплены дословно: правка гейта не должна прятаться в шве.

        Rust идёт через nextest одним профилем; число потоков и отчёт живут в
        `.config/nextest.toml`. Локальная агентная приёмка в этот профиль не входит.
        """
        module = load_module()

        rust = module.commands("all", "rust", interpreter="python")
        python = module.commands("all", "python", interpreter="python")

        self.assertEqual(
            rust,
            [
                ["cargo", "nextest", "run", "--workspace", "--profile", "default"],
                [
                    "cargo", "nextest", "run", "-p", "unica-coder",
                    "--features", "receipt-ledger-test-support",
                    "--lib", "--test", "daemon_receipt_ledger", "-E",
                    "binary(daemon_receipt_ledger) | "
                    "test(/^infrastructure::receipt_scenario::tests::/)",
                    "--profile", "default",
                ],
                ["cargo", "nextest", "run", *UREQ_SELECTION, "--profile", "ureq-recycling"],
            ],
        )
        runner_script = str(MODULE_PATH.with_name("run-unittest.py"))
        self.assertEqual(
            python[:2],
            [
                ["python", runner_script, "-s", "tests/ci", "--durations", "20"],
                ["python", runner_script, "-s", "tests/dev", "--durations", "20"],
            ],
        )
        self.assertEqual(len(python), 2)

    def test_all_ecosystems_keep_rust_before_python(self) -> None:
        module = load_module()

        planned = module.commands("all", "all", interpreter="python")

        # Workspace, ReceiptLedger и dependency-local регрессии идут до Python.
        self.assertEqual([command[0] for command in planned[:3]], ["cargo"] * 3)
        self.assertTrue(all(command[0] == "python" for command in planned[3:]))
        self.assertTrue(all(command[1].endswith("run-unittest.py") for command in planned[3:]))
        self.assertEqual(len(planned), 5)

    def test_results_directory_turns_emission_on_for_python_suites(self) -> None:
        """Без `--results` набор идёт как раньше; с ним пишет результаты и знает раннер."""
        module = load_module()
        from pathlib import Path as P

        quiet = module.commands("all", "python", interpreter="python")
        emitting = module.commands("all", "python", interpreter="python", results=P("out"), runner="macos-14")

        self.assertTrue(all("--results" not in command for command in quiet))
        for command in emitting:
            self.assertIn("--results", command)
            self.assertIn("macos-14", command)

    def test_own_runner_imports_every_module_the_module_runner_could(self) -> None:
        """`python -m unittest` видит `scripts.ci.*` от корня; наш раннер обязан тоже.

        Иначе модуль, импортирующий скрипт конвейера, падает на импорте и
        считается упавшим тестом — так и было на первом прогоне.
        """
        import subprocess
        import sys

        runner = MODULE_PATH.with_name("run-unittest.py")
        listing = subprocess.run(
            [sys.executable, str(runner), "-s", "tests/ci", "--plan-only"],
            cwd=MODULE_PATH.parents[2], capture_output=True, text=True, check=True,
        ).stdout

        self.assertNotIn("_FailedTest", listing)
        self.assertIn("test_donor_parity_contract", listing)

    def test_run_signature_is_written_once_per_invocation_and_names_the_ecosystem(self) -> None:
        """`--ecosystem all` — одна подпись с `all`, а не две, где вторая стирает первую."""
        import json
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))
        calls = []

        code = module.execute("all", "all", out, "ubuntu-latest",
                              run_commands=lambda planned: calls.append(planned) or 0,
                              junit=out / "no-such-junit.xml")

        self.assertEqual(code, 0)
        # Три вызова nextest снимают JUnit; затем один список Python.
        self.assertEqual(len(calls), 4)
        self.assertEqual([len(planned) for planned in calls[:3]], [1, 1, 1])
        signature = json.loads((out / "run.json").read_text(encoding="utf-8"))
        self.assertEqual(signature["ecosystem"], "all")
        self.assertEqual(signature["runner"], "ubuntu-latest")

    def test_rust_failure_before_junit_still_leaves_the_signature(self) -> None:
        """Подпись описывает вызов, а не исход: nextest упал до JUnit — она всё равно есть."""
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))

        code = module.execute("all", "rust", out, "macos-14",
                              run_commands=lambda planned: 101,
                              junit=out / "no-such-junit.xml")

        self.assertEqual(code, 101)
        self.assertTrue((out / "run.json").is_file())
        self.assertEqual(list(out.glob("*-result.json")), [])

    def test_stale_junit_from_an_earlier_run_is_never_emitted_as_fresh(self) -> None:
        """nextest упал до отчёта, а от прошлого прогона остался JUnit — он не результат."""
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))
        stale = out / "junit.xml"
        stale.write_text(
            '<testsuites name="unica"><testsuite name="unica-coder">'
            '<testcase name="old::passed" classname="unica-coder" time="0.1"/>'
            "</testsuite></testsuites>",
            encoding="utf-8",
        )

        code = module.execute("all", "rust", out, "ubuntu-latest",
                              run_commands=lambda planned: 101, junit=stale)

        self.assertEqual(code, 101)
        self.assertFalse(stale.exists(), "старый отчёт обязан быть снят до прогона")
        self.assertEqual(list(out.glob("*-result.json")), [])

    def test_unknown_profile_or_ecosystem_is_a_refusal_not_an_empty_run(self) -> None:
        """Пустой прогон зелёный по устройству; отказ — единственный честный ответ."""
        module = load_module()

        with self.assertRaises(ValueError):
            module.commands("weekly", "rust")
        with self.assertRaises(ValueError):
            module.commands("all", "go")

    def test_dry_run_prints_commands_and_runs_nothing(self) -> None:
        module = load_module()
        import contextlib
        import io

        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            code = module.main(["--profile", "all", "--ecosystem", "rust", "--dry-run"])

        self.assertEqual(code, 0)
        self.assertEqual(
            captured.getvalue().strip().splitlines(),
            [
                "cargo nextest run --workspace --profile default",
                "cargo nextest run -p unica-coder --features receipt-ledger-test-support"
                " --lib --test daemon_receipt_ledger"
                " -E binary(daemon_receipt_ledger) |"
                " test(/^infrastructure::receipt_scenario::tests::/)"
                " --profile default",
                "cargo nextest run --manifest-path vendor/ureq/Cargo.toml --locked --lib"
                " --features json,serde_json/preserve_order,serde_json/float_roundtrip"
                " --config-file vendor/ureq/.config/nextest.toml --target-dir target"
                " -E test(/^completed_response_recycling::/) --profile ureq-recycling",
            ],
        )


class GateProfileTests(unittest.TestCase):
    """Ворота ложатся на одноимённые профили nextest; Python отбирает наборы по размеру."""

    def test_each_gate_maps_to_its_own_nextest_profile_and_junit_directory(self) -> None:
        module = load_module()

        ledger = [
            "-p", "unica-coder", "--features", "receipt-ledger-test-support",
            "--lib", "--test", "daemon_receipt_ledger", "-E",
            "binary(daemon_receipt_ledger) | "
            "test(/^infrastructure::receipt_scenario::tests::/)",
        ]
        for gate in ("pr", "queue", "main", "release"):
            with self.subTest(gate=gate):
                commands = module.rust_commands(gate)
                self.assertEqual(commands[0], ["cargo", "nextest", "run", "--workspace", "--profile", gate])
                if gate == "pr":
                    # Контракт ReceiptLedger — `medium` и `large` целиком; ворота
                    # `small` его не гоняют и не вызывают nextest впустую.
                    self.assertEqual(len(commands), 1)
                else:
                    # Контракт и сценарии — один вызов: одна сборка с признаком.
                    self.assertEqual(commands[1], ["cargo", "nextest", "run", *ledger, "--profile", gate])
                    self.assertEqual(commands[2], ["cargo", "nextest", "run", *UREQ_SELECTION, "--profile", "ureq-recycling"])
                    self.assertEqual(len(commands), 3)
                    self.assertEqual(
                        module.nextest_junit(gate, UREQ_SELECTION).parts[-3:],
                        ("nextest", "ureq-recycling", "junit.xml"),
                    )
                self.assertEqual(module.nextest_junit(gate).parts[-3:], ("nextest", gate, "junit.xml"))
        self.assertEqual(module.nextest_junit("all").parts[-3:], ("nextest", "default", "junit.xml"))

    def test_plan_lists_exactly_the_selections_the_run_executes(self) -> None:
        """Состав и прогон — один отбор: `list` повторяет `run` флаг в флаг."""
        module = load_module()

        for gate in module.PROFILES:
            with self.subTest(gate=gate):
                runs = module.rust_commands(gate)
                lists = module.rust_list_commands(gate)
                self.assertEqual(len(runs), len(lists))
                for run, listing in zip(runs, lists):
                    selection = run[3:run.index("--profile")]
                    self.assertEqual(listing[:3], ["cargo", "nextest", "list"])
                    self.assertEqual(listing[3:3 + len(selection)], selection)
                    self.assertEqual(listing[-4:], ["--run-ignored", "all", "--message-format", "json"])

    def test_scenario_library_plan_and_results_share_the_exact_medium_boundary(self) -> None:
        from unittest.mock import patch

        module = load_module()
        prefix = module.SCENARIO_TEST_PREFIX
        cases = [
            {"binary": "unica-coder", "name": prefix + "owned", "ignored": False},
            {"binary": "unica-coder", "name": "unrelated::sibling", "ignored": False},
            {"binary": "other-coder", "name": prefix + "foreign", "ignored": False},
            # Контракт идёт тем же вызовом; переразметка его не касается.
            {"binary": "unica-coder::daemon_receipt_ledger", "name": prefix + "contract", "ignored": False},
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(module.allure_results, "nextest_list", side_effect=lambda _, command: [dict(case) for case in cases] if "receipt-ledger-test-support" in command else []):
                self.assertEqual(module.write_rust_plan(root, "queue"), 4)
            planned = json.loads((root / "plan.json").read_text())
            self.assertEqual(planned[0]["size"], "medium")
            self.assertNotIn("size", planned[1])
            self.assertNotIn("size", planned[2])
            self.assertNotIn("size", planned[3])
            junit = root / "junit.xml"
            junit.write_text(
                '<testsuites><testsuite name="unica-coder">'
                f'<testcase name="{prefix}owned" time="0.1"/>'
                '<testcase name="unrelated::sibling" time="0.1"/>'
                '</testsuite><testsuite name="other-coder">'
                f'<testcase name="{prefix}foreign" time="0.1"/>'
                '</testsuite><testsuite name="unica-coder::daemon_receipt_ledger">'
                f'<testcase name="{prefix}contract" time="0.1"/>'
                '</testsuite></testsuites>'
            )
            contract_sizes = set()
            for selection, expected_owned_size in [(module.LEDGER_FEATURE_SELECTION, "medium"), (("--workspace",), "small")]:
                output = root / expected_owned_size
                self.assertEqual(module.emit_rust(output, "queue", "linux", junit, selection=selection), 4)
                results = [json.loads(path.read_text()) for path in output.glob("*-result.json")]
                sizes = {entry["fullName"]: next(label["value"] for label in entry["labels"] if label["name"] == "size") for entry in results}
                self.assertEqual(sizes["unica-coder::" + prefix + "owned"], expected_owned_size)
                self.assertEqual(sizes["unica-coder::unrelated::sibling"], "small")
                self.assertEqual(sizes["other-coder::" + prefix + "foreign"], "small")
                contract_sizes.add(sizes["unica-coder::daemon_receipt_ledger::" + prefix + "contract"])
                for entry in results:
                    self.assertIn({"name": "profile", "value": "queue"}, entry["labels"])
                    self.assertEqual(entry["parameters"], [{"name": "runner", "value": "linux"}])
            # Размер контракта не зависит от того, каким вызовом он пришёл.
            self.assertEqual(len(contract_sizes), 1)

    def test_each_rust_invocation_gets_its_own_junit_snapshot(self) -> None:
        """JUnit одного каталога снимается после каждого вызова, а не раз в конце."""
        import tempfile
        from pathlib import Path as P

        module = load_module()
        out = P(tempfile.mkdtemp(prefix="run-tests-"))
        junit = out / "junit.xml"
        seen = []

        def run_commands(planned):
            command = planned[0]
            vendor = "--manifest-path" in command
            featured = "receipt-ledger-test-support" in command
            seen.append("--manifest-path" if vendor else "--features" if featured else "--workspace")
            if vendor:
                suites = (
                    '<testsuite name="ureq">'
                    '<testcase name="completed_response_recycling::case" time="0.1"/>'
                    '</testsuite>'
                )
            elif featured:
                # Один вызов с признаком пишет обе цели: сценарии библиотеки
                # и контракт.
                suites = (
                    '<testsuite name="unica-coder">'
                    '<testcase name="' + module.SCENARIO_TEST_PREFIX + 'case" classname="unica-coder" time="0.1"/>'
                    '</testsuite><testsuite name="unica-coder::daemon_receipt_ledger">'
                    '<testcase name="case" classname="unica-coder::daemon_receipt_ledger" time="0.1"/>'
                    '</testsuite>'
                )
            else:
                suites = (
                    '<testsuite name="unica-coder"><testcase name="case" classname="unica-coder" time="0.1">'
                    '<failure message="assertion failed: first invocation"/></testcase></testsuite>'
                )
            junit.write_text('<testsuites name="unica">' + suites + "</testsuites>", encoding="utf-8")
            return 0 if featured or vendor else 1

        code = module.execute("queue", "rust", out, "ubuntu-latest", run_commands=run_commands, junit=junit)

        self.assertEqual(code, 1, "later successful JUnit must not hide the first failure")
        self.assertEqual(seen, ["--workspace", "--features", "--manifest-path"])
        names = sorted(json.loads(path.read_text(encoding="utf-8"))["fullName"] for path in out.glob("*-result.json"))
        self.assertEqual(names, sorted([
            "unica-coder::case", "unica-coder::daemon_receipt_ledger::case",
            "unica-coder::" + module.SCENARIO_TEST_PREFIX + "case",
            "ureq::completed_response_recycling::case",
        ]))
        entries = [json.loads(path.read_text(encoding="utf-8")) for path in out.glob("*-result.json")]
        statuses = {entry["fullName"]: entry["status"] for entry in entries}
        self.assertEqual(statuses["unica-coder::case"], "failed")
        self.assertEqual(statuses["unica-coder::daemon_receipt_ledger::case"], "passed")
        self.assertEqual(statuses["unica-coder::" + module.SCENARIO_TEST_PREFIX + "case"], "passed")
        self.assertEqual(statuses["ureq::completed_response_recycling::case"], "passed")
        library_entry = next(entry for entry in entries if entry["fullName"].endswith(module.SCENARIO_TEST_PREFIX + "case"))
        self.assertIn({"name": "size", "value": "medium"}, library_entry["labels"])

    def test_surviving_products_are_listed_before_each_feature_rebuild_only(self) -> None:
        """Отчёт идёт перед пересборкой с признаком, а не перед workspace-вызовом."""
        module = load_module()
        out = Path(tempfile.mkdtemp(prefix="run-tests-"))
        events = []

        def run_commands(planned):
            command = planned[0]
            events.append("--manifest-path" if "--manifest-path" in command else
                          "--features" if "receipt-ledger-test-support" in command else "--workspace")
            return 0

        code = module.execute(
            "large", "rust", out, "windows-latest", run_commands=run_commands,
            junit=out / "junit.xml", before_feature_build=lambda: events.append("report"),
        )

        self.assertEqual(code, 0)
        self.assertEqual(events, ["--workspace", "report", "--features", "--manifest-path"])

    def test_vendor_plan_and_results_use_medium_and_the_gate_identity_only_for_owned_cases(self) -> None:
        from unittest.mock import patch

        module = load_module()
        owned = "completed_response_recycling::complete_empty_response_survives_read_reset_failure"
        cases = [
            {"binary": "ureq", "name": owned, "ignored": False},
            {"binary": "ureq", "name": "fixture::unrelated", "ignored": False},
            {"binary": "other", "name": owned, "ignored": False},
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def list_cases(_, command):
                return [dict(case) for case in cases] if "--manifest-path" in command else []
            with patch.object(module.allure_results, "nextest_list", side_effect=list_cases):
                self.assertEqual(module.write_rust_plan(root, "queue"), 3)
            self.assertEqual(json.loads((root / "plan.json").read_text()), [
                {"binary": "ureq", "name": owned, "ignored": False, "size": "medium"},
                {"binary": "ureq", "name": "fixture::unrelated", "ignored": False},
                {"binary": "other", "name": owned, "ignored": False},
            ])
            junit = root / "junit.xml"
            junit.write_text(
                '<testsuites><testsuite name="ureq">'
                f'<testcase name="{owned}" time="0.1"/>'
                '<testcase name="fixture::unrelated" time="0.1"/>'
                '</testsuite><testsuite name="other">'
                f'<testcase name="{owned}" time="0.1"/>'
                '</testsuite></testsuites>', encoding="utf-8",
            )
            output = root / "results"
            self.assertEqual(module.emit_rust(output, "queue", "linux", junit, selection=UREQ_SELECTION), 3)
            results = [json.loads(path.read_text()) for path in output.glob("*-result.json")]
            sizes = {entry["fullName"]: next(label["value"] for label in entry["labels"] if label["name"] == "size") for entry in results}
            self.assertEqual(sizes, {
                "ureq::" + owned: "medium",
                "ureq::fixture::unrelated": "small",
                "other::" + owned: "small",
            })
            for entry in results:
                self.assertIn({"name": "profile", "value": "queue"}, entry["labels"])
                self.assertIn({"name": "tag", "value": "queue"}, entry["labels"])
                self.assertNotIn({"name": "profile", "value": "ureq-recycling"}, entry["labels"])
                self.assertEqual(entry["parameters"], [{"name": "runner", "value": "linux"}])

    def test_vendor_junit_is_own_and_empty_selection_cannot_reuse_an_old_success(self) -> None:
        from unittest.mock import patch

        # The executor's nonzero case models nextest refusing an empty selection.
        # It does not stand in for root's actual nextest selection check.
        for vendor_code in (0, 4):
            with self.subTest(vendor_code=vendor_code), tempfile.TemporaryDirectory() as temporary:
                module = load_module()
                root = Path(temporary)
                shared = root / "target/nextest/queue/junit.xml"
                vendor = root / "vendor/ureq/target/nextest/ureq-recycling/junit.xml"
                for path in (shared, vendor):
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text('<testsuites><testsuite name="ureq"><testcase name="stale::success"/></testsuite></testsuites>')
                seen = []
                def execute(planned):
                    command = planned[0]
                    if "--manifest-path" in command:
                        seen.append("vendor")
                        self.assertFalse(vendor.exists(), "the vendor's stale JUnit must be removed before its run")
                        self.assertNotIn("--no-tests=pass", command)
                        if vendor_code:
                            return vendor_code
                        path, binary, name = vendor, "ureq", "completed_response_recycling::fresh"
                    else:
                        self.assertFalse(shared.exists())
                        featured = "receipt-ledger-test-support" in command
                        seen.append("ledger" if featured else "workspace")
                        path, binary, name = shared, "unica-coder", "ledger::fresh" if featured else "workspace::fresh"
                    path.write_text(f'<testsuites><testsuite name="{binary}"><testcase name="{name}" time="0.1"/></testsuite></testsuites>')
                    return 0
                with patch.object(module, "REPO_ROOT", root):
                    self.assertEqual(module.nextest_junit("queue", UREQ_SELECTION), vendor)
                    code = module.execute("queue", "rust", root / "results", "linux", run_commands=execute)
                self.assertEqual(code, vendor_code)
                self.assertEqual(seen, ["workspace", "ledger", "vendor"])
                names = sorted(json.loads(path.read_text())["fullName"] for path in (root / "results").glob("*-result.json"))
                expected = ["unica-coder::workspace::fresh", "unica-coder::ledger::fresh"]
                if not vendor_code:
                    expected.append("ureq::completed_response_recycling::fresh")
                self.assertEqual(names, sorted(expected))
                self.assertEqual(vendor.exists(), vendor_code == 0)

    def test_surviving_product_report_is_windows_only_and_never_fails_the_run(self) -> None:
        module = load_module()
        self.assertIsNone(module.surviving_product_report("posix"))
        report = module.surviving_product_report("nt")
        self.assertIs(report, module.report_surviving_product_processes)

        calls = []
        module.report_surviving_product_processes(
            run_query=lambda command, **options: calls.append((command, options)),
        )
        command, options = calls[0]
        self.assertEqual(command[0], "powershell")
        self.assertIn("Win32_Process", command[-1])
        self.assertIn("Name='unica.exe'", command[-1])
        self.assertIn("CommandLine", command[-1])
        self.assertFalse(options["check"])

        def missing_shell(command, **options):
            raise FileNotFoundError("powershell")

        module.report_surviving_product_processes(run_query=missing_shell)

    def test_every_gate_runs_suites_of_its_admitted_sizes(self) -> None:
        """Каждый набор запускается именно в воротах, допускающих его размер."""
        module = load_module()

        for gate in module.PROFILES:
            with self.subTest(gate=gate):
                suites = [command[3] for command in module.python_commands(gate, "python3")]
                self.assertEqual(suites, [suite for suite, size, _ in module.PYTHON_SUITES
                                         if size in module.ADMITTED[gate]])
        self.assertEqual(module.python_commands("large", "python3"), module.python_commands("release", "python3"))
        with self.assertRaises(ValueError):
            module.python_commands("all", "python3", suite="tests/agent_evaluation")

    def test_plan_only_for_python_writes_the_composition_of_the_suite(self) -> None:
        """Состав набора — до запуска, тем же швом: джоба, умершая на середине, его не унесёт."""
        module = load_module()
        results = Path(tempfile.mkdtemp(prefix="plan-py-"))

        code = module.main(["--profile", "pr", "--ecosystem", "python", "--suite", "tests/dev", "--plan-only", "--results", str(results), "--runner", "ubuntu-latest"])

        self.assertEqual(code, 0)
        self.assertTrue((results / "run.json").is_file())
        planned = [entry for entry in json.loads((results / "plan.json").read_text(encoding="utf-8")) if entry.get("ecosystem") == "python"]
        self.assertGreater(len(planned), 50)
        self.assertTrue(all(entry["suite"] == "tests/dev" for entry in planned))
        self.assertTrue(all(entry["id"].count(".") >= 2 and entry["subSuite"] and entry["size"] for entry in planned))

    def test_size_lanes_split_a_suite_by_the_sizes_the_gate_admits(self) -> None:
        """Полоса гоняет один размер внутри набора; недопущенный воротами размер — пустой список."""
        module = load_module()

        medium = module.python_commands("queue", "python3", suite="tests/ci", only_size="medium")
        small = module.python_commands("queue", "python3", suite="tests/ci", only_size="small")
        self.assertEqual(len(medium), 1)
        self.assertEqual(medium[0][medium[0].index("--admit") + 1], "medium")
        self.assertEqual(medium[0][medium[0].index("--lane") + 1], "medium")
        self.assertEqual(small[0][small[0].index("--admit") + 1], "small")
        # Ворота pr не принимают medium: полоса пуста, а не «все».
        self.assertEqual(module.python_commands("pr", "python3", suite="tests/ci", only_size="medium"), [])
        with self.assertRaises(ValueError):
            module.python_commands("queue", "python3", suite="tests/ci", only_size="huge")
        # Матрица ворот: полосатый набор — по размерам, остальные — одной джобой.
        self.assertEqual(
            [entry["slug"] for entry in module.python_matrix("queue")],
            ["ci-small", "ci-medium", "dev"],
        )
        self.assertEqual([entry["slug"] for entry in module.python_matrix("pr")], ["ci-small", "dev"])
        for entry in module.python_matrix("main"):
            self.assertEqual(entry["slug"], module.python_slug(entry["suite"], entry["lane"]))

    def test_one_suite_per_job_narrows_the_seam_without_changing_the_command(self) -> None:
        """В CI наборы идут параллельными джобами: `--suite` даёт ровно одну команду, ту же самую."""
        module = load_module()

        every = module.python_commands("main", "python3")
        for suite, _, _ in module.PYTHON_SUITES:
            with self.subTest(suite=suite):
                only = module.python_commands("main", "python3", suite=suite)
                self.assertEqual(only, [command for command in every if command[3] == suite])
        self.assertEqual(module.commands("main", "python", "python3", suite="tests/dev"), module.python_commands("main", "python3", suite="tests/dev"))
        with self.assertRaises(ValueError):
            module.python_commands("main", "python3", suite="tests/nowhere")
        # Rust от набора не зависит: `--suite` сужает только Python.
        self.assertEqual(module.commands("main", "all", "python3", suite="tests/ci")[0], module.rust_commands("main")[0])


if __name__ == "__main__":
    unittest.main()
