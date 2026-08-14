import importlib.util
import json
import stat
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/dev/benchmark-rlm-index.py"
SPEC = importlib.util.spec_from_file_location("benchmark_rlm_index", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = MODULE
SPEC.loader.exec_module(MODULE)


class BenchmarkRlmIndexTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name)
        self.fixture_number = 0

    def run_git(self, repo: Path, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", *args],
            cwd=repo,
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )

    def git_fixture(
        self,
        *,
        bsl_count: int = 1,
        root_xml_count: int = 1,
        form_xml_count: int = 1,
    ) -> Path:
        self.fixture_number += 1
        repo = self.root / f"repo-{self.fixture_number}"
        repo.mkdir()

        for index in range(bsl_count):
            module_name = "One" if index == 0 else f"Module{index:03d}"
            module = repo / "src" / "CommonModules" / module_name / "Module.bsl"
            module.parent.mkdir(parents=True, exist_ok=True)
            module.write_text(
                f"Процедура Module{index:03d}()\nКонецПроцедуры\n",
                encoding="utf-8",
            )

        for index in range(root_xml_count):
            root_xml = repo / "src" / "Documents" / f"Document{index:03d}.xml"
            root_xml.parent.mkdir(parents=True, exist_ok=True)
            root_xml.write_text(f"<Document index=\"{index}\"/>\n", encoding="utf-8")

        for index in range(form_xml_count):
            form = (
                repo
                / "src"
                / "Documents"
                / "Document000"
                / "Forms"
                / f"Form{index:03d}"
                / "Ext"
                / "Form.xml"
            )
            form.parent.mkdir(parents=True, exist_ok=True)
            form.write_text(f"<Form index=\"{index}\"/>\n", encoding="utf-8")

        configuration = repo / "src" / "Configuration.xml"
        configuration.write_text("<Configuration/>\n", encoding="utf-8")

        self.run_git(repo, "init", "-q")
        self.run_git(repo, "config", "user.email", "unica-test@example.invalid")
        self.run_git(repo, "config", "user.name", "Unica Test")
        self.run_git(repo, "add", ".")
        self.run_git(
            repo,
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "fixture",
        )
        self.assertEqual(self.run_git(repo, "status", "--porcelain").stdout, "")
        return repo

    def sample_document(self, *, selected: dict[str, list[Path]] | None = None) -> dict:
        samples = {
            scenario.name: [
                MODULE.Sample(
                    duration_seconds=float(index + 1),
                    peak_rss_bytes=120_000_000 if scenario.name == "cold-build" else None,
                    git_fast_path=scenario.name != "cold-build",
                    final_status="fresh",
                    modules=12_290 if scenario.name == "cold-build" else None,
                    methods=220_748 if scenario.name == "cold-build" else None,
                    db_size_bytes=747_100_000 if scenario.name == "cold-build" else None,
                    index_size_bytes=759_100_000 if scenario.name == "cold-build" else None,
                    stdout_tail="measured stdout",
                    stderr_tail="",
                    info_stdout_tail="Status: fresh",
                    info_stderr_tail="",
                )
                for index in range(scenario.repeats)
            ]
            for scenario in MODULE.SCENARIOS
        }
        return MODULE.result_document(
            label="source-v1.33.0",
            source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
            executable_sha256="a" * 64,
            repo_head="b" * 40,
            selected=selected
            or {"bsl-1": [Path("src/CommonModules/One/Module.bsl")]},
            samples=samples,
            final_clean=True,
        )

    def test_refuses_a_dirty_tracked_tree(self) -> None:
        repo = self.git_fixture()
        (repo / "src" / "CommonModules" / "One" / "Module.bsl").write_text(
            "Процедура One(Changed)\nКонецПроцедуры\n",
            encoding="utf-8",
        )
        with self.assertRaisesRegex(RuntimeError, "tracked Git tree must be clean"):
            MODULE.ensure_clean(repo)

    def test_selects_exact_deterministic_scenario_sizes(self) -> None:
        repo = self.git_fixture(bsl_count=120, root_xml_count=12, form_xml_count=2)
        selected = MODULE.select_inputs(repo)
        self.assertEqual(len(selected["bsl-1"]), 1)
        self.assertEqual(len(selected["bsl-10"]), 10)
        self.assertEqual(len(selected["bsl-100"]), 100)
        self.assertEqual(len(selected["xml-form-1"]), 1)
        self.assertEqual(len(selected["xml-root-10"]), 10)
        self.assertEqual(selected, MODULE.select_inputs(repo))
        self.assertTrue(
            all(not path.is_absolute() for paths in selected.values() for path in paths)
        )

    def test_selection_uses_only_tracked_files(self) -> None:
        repo = self.git_fixture(bsl_count=120, root_xml_count=12, form_xml_count=2)
        untracked = repo / "000-first.bsl"
        untracked.write_text(
            "Процедура Untracked()\nКонецПроцедуры\n", encoding="utf-8"
        )

        selected = MODULE.select_inputs(repo)

        self.assertNotIn(Path("000-first.bsl"), selected["bsl-100"])

    def test_restores_files_and_runs_reverse_update_after_a_failed_measurement(self) -> None:
        repo = self.git_fixture()
        selected = MODULE.select_inputs(repo)["bsl-1"]
        calls = []
        with self.assertRaisesRegex(RuntimeError, "measured update failed"):
            MODULE.run_incremental_scenario(
                repo=repo,
                paths=selected,
                marker="UNICA_RLM_BENCHMARK_MARKER",
                measured_update=lambda: (_ for _ in ()).throw(
                    RuntimeError("measured update failed")
                ),
                reverse_update=lambda: calls.append("reverse"),
            )
        MODULE.ensure_clean(repo)
        self.assertEqual(calls, ["reverse"])

    def test_result_keeps_raw_samples_and_provenance(self) -> None:
        result = MODULE.result_document(
            label="packaged-v1.33.0",
            source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
            executable_sha256="a" * 64,
            repo_head="b" * 40,
            selected={"bsl-1": [Path("src/CommonModules/One/Module.bsl")]},
            samples={
                "bsl-1": [MODULE.Sample(5.2, 120_000_000, True, "fresh")]
            },
            final_clean=True,
        )
        self.assertEqual(result["schemaVersion"], 1)
        self.assertEqual(result["label"], "packaged-v1.33.0")
        self.assertEqual(
            result["sourceCommit"],
            "3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
        )
        self.assertEqual(result["executableSha256"], "a" * 64)
        self.assertEqual(result["repoHead"], "b" * 40)
        self.assertEqual(result["selected"]["bsl-1"], ["src/CommonModules/One/Module.bsl"])
        self.assertEqual(result["samples"]["bsl-1"][0]["durationSeconds"], 5.2)
        self.assertTrue(result["finalClean"])
        self.assertIn("python", result)
        self.assertIn("platform", result)

    def test_raw_sample_schema_keeps_command_evidence_and_index_statistics(self) -> None:
        result = MODULE.result_document(
            label="packaged-v1.33.0",
            source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
            executable_sha256="a" * 64,
            repo_head="b" * 40,
            selected={"bsl-1": [Path("src/CommonModules/One/Module.bsl")]},
            samples={
                "bsl-1": [
                    MODULE.Sample(
                        5.2,
                        120_000_000,
                        True,
                        "fresh",
                        modules=12_290,
                        methods=220_748,
                        db_size_bytes=747_100_000,
                        index_size_bytes=759_100_000,
                        stdout_tail="Changed: 1",
                        stderr_tail="warning",
                        info_stdout_tail="Status: fresh",
                        info_stderr_tail="",
                    )
                ]
            },
            final_clean=True,
        )

        self.assertEqual(
            result["samples"]["bsl-1"][0],
            {
                "durationSeconds": 5.2,
                "peakRssBytes": 120_000_000,
                "gitFastPath": True,
                "finalStatus": "fresh",
                "modules": 12_290,
                "methods": 220_748,
                "dbSizeBytes": 747_100_000,
                "indexSizeBytes": 759_100_000,
                "stdoutTail": "Changed: 1",
                "stderrTail": "warning",
                "infoStdoutTail": "Status: fresh",
                "infoStderrTail": "",
            },
        )

    def test_markdown_summary_rejects_absolute_workspace_paths(self) -> None:
        document = MODULE.result_document(
            label="packaged-v1.33.0",
            source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
            executable_sha256="a" * 64,
            repo_head="b" * 40,
            selected={"bsl-1": [Path("/client/workspace/Secret/Module.bsl")]},
            samples={
                "bsl-1": [MODULE.Sample(5.2, 120_000_000, True, "fresh")]
            },
            final_clean=True,
        )
        with self.assertRaisesRegex(RuntimeError, "summary contains an absolute path"):
            MODULE.markdown_summary([document])

    def test_markdown_summary_uses_raw_samples_without_selected_names(self) -> None:
        document = self.sample_document(
            selected={"bsl-1": [Path("src/Clients/SecretObject/Module.bsl")]}
        )

        summary = MODULE.markdown_summary([document])

        self.assertTrue(summary.startswith("## Замер RLM v1.33.0\n"))
        self.assertIn("`rlm-tools-bsl-v1.33.0-build.1`", summary)
        self.assertIn("| source-v1.33.0 | No-op update | 5 | 3,00 с | 1,00–5,00 с |", summary)
        self.assertIn("12 290", summary)
        self.assertIn("220 748", summary)
        self.assertNotIn("SecretObject", summary)
        self.assertNotIn("src/", summary)

    def test_summary_section_replacement_is_idempotent(self) -> None:
        summary = MODULE.markdown_summary([self.sample_document()])
        original = "# Existing issue\n\n## Existing section\n\nKeep me.\n"

        once = MODULE.replace_summary_section(original, summary)
        twice = MODULE.replace_summary_section(once, summary)

        self.assertEqual(once, twice)
        self.assertEqual(once.count("## Замер RLM v1.33.0"), 1)
        self.assertIn("Keep me.", once)

    def test_rejects_overlapping_or_nonempty_index_directories(self) -> None:
        repo = self.git_fixture()
        inside = repo / "index"
        inside.mkdir()
        with self.assertRaisesRegex(RuntimeError, "must not overlap"):
            MODULE.validate_index_dir(repo, inside)

        parent = repo.parent
        with self.assertRaisesRegex(RuntimeError, "must not overlap"):
            MODULE.validate_index_dir(repo, parent)

        sibling = self.root / "index"
        sibling.mkdir()
        (sibling / "existing.db").write_text("occupied", encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "must be empty"):
            MODULE.validate_index_dir(repo, sibling)

    def test_run_benchmark_executes_all_scenarios_and_restores_the_repo(self) -> None:
        repo = self.git_fixture(bsl_count=120, root_xml_count=12, form_xml_count=2)
        index_dir = self.root / "rlm-index"
        index_dir.mkdir()
        executable = self.root / "fake-rlm-bsl-index"
        executable.write_text(
            textwrap.dedent(
                """\
                #!/usr/bin/env python3
                import json
                import os
                import sys
                from pathlib import Path

                index_dir = Path(os.environ["RLM_INDEX_DIR"])
                action = sys.argv[2]
                repo = sys.argv[3]
                with (index_dir / "commands.jsonl").open("a", encoding="utf-8") as stream:
                    stream.write(json.dumps({"action": action, "repo": repo}) + "\\n")
                if action == "build":
                    (index_dir / "bsl_index.db").write_bytes(b"database")
                    print("Index built in 0.01s")
                    print("  Modules: 120")
                    print("  Methods: 240")
                    print("  DB size: 8 B")
                elif action == "update":
                    print("Changed: 1")
                    print("Fast path: True")
                    print("  Modules: 120")
                    print("  Methods: 240")
                    print("  DB size: 8 B")
                elif action == "info":
                    print("Status: fresh")
                    print("Modules: 120")
                    print("Methods: 240")
                    print("DB size: 8 B")
                else:
                    raise SystemExit(3)
                """
            ),
            encoding="utf-8",
        )
        executable.chmod(executable.stat().st_mode | stat.S_IXUSR)

        document = MODULE.run_benchmark(
            repo=repo,
            executable=executable,
            label="packaged-v1.33.0",
            source_commit="3e6920cd015a61af4ba7aa1a5f1fedd8bc935549",
            index_dir=index_dir,
        )

        self.assertEqual(
            {name: len(samples) for name, samples in document["samples"].items()},
            {scenario.name: scenario.repeats for scenario in MODULE.SCENARIOS},
        )
        self.assertEqual(document["samples"]["cold-build"][0]["modules"], 120)
        self.assertEqual(document["samples"]["cold-build"][0]["methods"], 240)
        self.assertEqual(document["samples"]["cold-build"][0]["dbSizeBytes"], 8)
        self.assertGreaterEqual(document["samples"]["cold-build"][0]["indexSizeBytes"], 8)
        self.assertTrue(document["finalClean"])
        MODULE.ensure_clean(repo)
        marker = "UNICA_RLM_BENCHMARK_MARKER"
        for path in self.run_git(repo, "ls-files", "-z").stdout.split("\0"):
            if path:
                self.assertNotIn(marker, (repo / path).read_text(encoding="utf-8"))
        commands = [
            json.loads(line)
            for line in (index_dir / "commands.jsonl").read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(commands[0]["action"], "build")
        self.assertTrue(all(command["repo"] == str(repo.resolve()) for command in commands))


if __name__ == "__main__":
    unittest.main()
