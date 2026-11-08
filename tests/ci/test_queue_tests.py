"""Тесты записи очереди: весь small, medium изменённых крейтов, когда полный набор."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import yaml

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("queue_tests", ROOT / "scripts" / "ci" / "queue-tests.py")
assert SPEC is not None and SPEC.loader is not None
queue = importlib.util.module_from_spec(SPEC)
sys.modules["queue_tests"] = queue
SPEC.loader.exec_module(queue)

DIRS = {"crates/a": "unica-a", "crates/b": "unica-b", "crates/coder": "unica-coder", "vendor/ureq": "ureq"}
DEPENDENTS = {"unica-a": {"unica-b", "unica-coder"}, "unica-b": {"unica-coder"}}


def choose(paths, found=(), **options):
    return queue.choose(paths, DIRS, DEPENDENTS, find=lambda text: list(found), **options)


class ChoiceTests(unittest.TestCase):
    def test_any_file_of_a_crate_selects_that_crate(self) -> None:
        for path in ("crates/a/src/lib.rs", "crates/a/tests/t.rs", "crates/a/build.rs", "crates/a/Cargo.toml", "crates/a/README.md"):
            with self.subTest(path=path):
                choice = choose([path])
                self.assertFalse(choice.full)
                self.assertEqual(choice.packages, ("unica-a",))

    def test_unica_coder_is_selected_like_any_other_crate(self) -> None:
        self.assertEqual(choose(["crates/coder/src/lib.rs", "crates/a/src/lib.rs"]).packages, ("unica-a", "unica-coder"))

    def test_direct_dependents_run_integration_targets_and_transitive_ones_wait_for_the_night(self) -> None:
        self.assertEqual(choose(["crates/a/src/lib.rs"]).dependents, ("unica-b", "unica-coder"))
        # unica-coder здесь зависит от unica-b, а не транзитивно через него попадает.
        no_direct = queue.choose(["crates/a/src/lib.rs"], DIRS, {"unica-a": {"unica-b"}, "unica-b": {"unica-coder"}}, find=lambda text: [])
        self.assertEqual(no_direct.dependents, ("unica-b",))
        self.assertEqual(choose(["crates/a/src/lib.rs", "crates/coder/src/lib.rs"]).dependents, ("unica-b",))

    def test_a_crate_chosen_by_mention_does_not_pull_its_dependents(self) -> None:
        choice = choose(["scripts/dev/x.py"], ["crates/a/src/lib.rs"])
        self.assertEqual(choice.packages, ("unica-a",))
        self.assertEqual(choice.dependents, ())

    def test_the_dependent_filter_keeps_integration_targets_without_the_ledger(self) -> None:
        self.assertEqual(queue.DEPENDENT_FILTER, "kind(test) & not binary(daemon_receipt_ledger)")
        runner = mock.Mock(LEDGER_FEATURE_SELECTION=("ledger",), UREQ_RECYCLING_SELECTION=("ureq",))
        runner.rust_command.side_effect = lambda profile, selection: [profile, *selection]
        planned = queue.commands(("unica-a",), runner, ("unica-coder",))
        self.assertEqual(planned[-1], ["queue", "-p", "unica-coder", "-E", queue.DEPENDENT_FILTER, "--no-tests=pass"])
        self.assertNotIn(["queue", "ledger"], planned)

    def test_only_the_whole_build_runs_everything(self) -> None:
        for path in ("Cargo.lock", "Cargo.toml", "rust-toolchain.toml", ".cargo/config.toml"):
            with self.subTest(path=path):
                self.assertTrue(choose([path, "crates/a/src/lib.rs"]).full)

    def test_a_lock_changed_only_for_workspace_crates_selects_by_crate(self) -> None:
        choice = choose(["Cargo.lock", "crates/a/Cargo.toml", "crates/a/src/lib.rs"], lock_is_internal=True)
        self.assertFalse(choice.full)
        self.assertEqual(choice.packages, ("unica-a",))
        self.assertEqual(choice.dependents, ("unica-b", "unica-coder"))
        # Манифест рабочей области и тулчейн остаются полным набором и рядом с такой правкой.
        for path in ("Cargo.toml", "rust-toolchain.toml"):
            with self.subTest(path=path):
                self.assertTrue(choose(["Cargo.lock", path], lock_is_internal=True).full)

    def test_only_external_packages_of_the_lock_count(self) -> None:
        member = '[[package]]\nname = "unica-a"\nversion = "1.0.0"\ndependencies = ["serde"]\n'
        serde = '[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+x"\nchecksum = "aa"\n'
        members = {"unica-a", "unica-new"}
        same = queue.external_packages(serde + member, members)
        self.assertEqual(same, queue.external_packages(serde, members))
        self.assertEqual(same, queue.external_packages(member + '[[package]]\nname = "unica-new"\nversion = "1.0.0"\n' + serde, members))
        # Путевой патч не член рабочей области, хотя `source` у него тоже нет.
        patched = '[[package]]\nname = "ureq"\nversion = "2.0.0"\n'
        self.assertNotEqual(queue.external_packages(serde + patched, members), queue.external_packages(serde + patched.replace("2.0.0", "2.0.1"), members))
        for changed in (
            serde.replace('version = "1.0.0"', 'version = "1.0.1"'),
            serde.replace('checksum = "aa"', 'checksum = "bb"'),
            serde.replace('registry+x', 'git+y'),
            serde + 'dependencies = ["itoa"]\n',
            serde + '[[package]]\nname = "itoa"\nversion = "1.0.0"\nsource = "registry+x"\n',
            '[[package]]\nname = "other"\nversion = "1.0.0"\n',
        ):
            with self.subTest(changed=changed):
                self.assertNotEqual(same, queue.external_packages(changed, members))

    def test_the_lock_is_compared_between_the_two_revisions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            git = lambda *arguments: subprocess.check_output(["git", "-c", "user.name=t", "-c", "user.email=t@example.test", *arguments], cwd=root, text=True).strip()
            git("init", "-q")
            serde = '[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+x"\n'
            revisions = []
            for lock in (serde, serde + '[[package]]\nname = "unica-new"\nversion = "1.0.0"\n', serde.replace("1.0.0", "1.0.1")):
                (root / "Cargo.lock").write_text(lock, encoding="utf-8")
                git("add", "Cargo.lock")
                git("commit", "-qm", "x")
                revisions.append(git("rev-parse", "HEAD"))
            self.assertTrue(queue.lock_keeps_external(revisions[0], revisions[1], {"unica-new"}, root))
            self.assertFalse(queue.lock_keeps_external(revisions[1], revisions[2], {"unica-new"}, root))

    def test_a_lock_that_cannot_be_read_runs_everything(self) -> None:
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "o")}):
            with mock.patch.object(queue, "workspace", return_value=(DIRS, DEPENDENTS)), mock.patch.object(queue.subprocess, "check_output", side_effect=["Cargo.lock\n", subprocess.CalledProcessError(128, "git")]):
                self.assertEqual(queue.main(["--base", "a", "--head", "b", "--dry-run"]), 0)
            self.assertEqual((Path(directory) / "o").read_text(encoding="utf-8"), "full=true\n")

    def test_paths_rust_tests_read_select_the_crates_that_mention_them(self) -> None:
        for path in ("tests/fixtures/xdto/a.xml", "scripts/dev/x.py"):
            with self.subTest(path=path):
                self.assertEqual(choose([path], ["crates/b/tests/t.rs"]).packages, ("unica-b",))

    def test_the_search_is_for_the_data_set_not_the_single_file(self) -> None:
        self.assertEqual(queue.needle("tests/fixtures/xdto/deep/a.xml"), "tests/fixtures/xdto")
        self.assertEqual(queue.needle("scripts/dev/x.py"), "scripts/dev/x.py")

    def test_everything_else_runs_only_small(self) -> None:
        for path in (".github/workflows/ci.yml", ".github/CODEOWNERS", "scripts/ci/run-tests.py", ".config/nextest.toml", ".config/python-sizes.toml",
                     "tests/fixtures/acceptance/scenario-manifest/x.json", "tests/ci/test_x.py", "docs/release-runbook.md", ".gitattributes",
                     "plugins/unica/skills/x/SKILL.md"):
            with self.subTest(path=path):
                choice = choose([path], ["crates/b/tests/t.rs"])
                self.assertFalse(choice.full)
                self.assertEqual(choice.packages, ())

    def test_site_rules_skills_and_prose_alone_run_no_rust(self) -> None:
        for path in ("docs/pages/a.html", "docs/visual-kit/a.css", "arch/rules/core/a.md", ".agents/skills/x/SKILL.md",
                     "scripts/research/a.sh", "README.md", "AGENTS.md"):
            with self.subTest(path=path):
                choice = choose([path])
                self.assertTrue(choice.no_rust)
                self.assertFalse(choice.full)
        # Рядом с другим путём запись снова идёт по таблице.
        self.assertFalse(choose(["README.md", ".github/workflows/ci.yml"]).no_rust)
        self.assertEqual(choose(["README.md", "crates/a/src/lib.rs"]).packages, ("unica-a",))
        self.assertTrue(choose(["crates/a/README.md"]).packages)

    def test_an_empty_diff_and_the_switch_run_everything(self) -> None:
        self.assertTrue(choose([]).full)
        self.assertTrue(choose(["crates/a/src/lib.rs"], force_full=True).full)

    def test_every_crate_selected_runs_the_full_set_in_the_real_workspace(self) -> None:
        directories, dependents = queue.workspace()
        members = [path for path, name in directories.items() if name != "ureq"]
        choice = queue.choose([f"{path}/Cargo.toml" for path in members], directories, dependents)
        self.assertTrue(choice.full, choice.reason)
        # Один незатронутый крейт оставляет обычный выбор.
        choice = queue.choose([f"{path}/Cargo.toml" for path in members[1:]], directories, dependents)
        self.assertFalse(choice.full, choice.reason)
        self.assertNotIn(directories[members[0]], choice.packages)

    def test_small_runs_everywhere_and_selected_crates_are_not_run_twice(self) -> None:
        runner = mock.Mock(LEDGER_FEATURE_SELECTION=("ledger",), UREQ_RECYCLING_SELECTION=("ureq",))
        runner.rust_command.side_effect = lambda profile, selection: [profile, *selection]
        self.assertEqual(queue.commands((), runner), [["pr", "--workspace", "--no-tests=pass"]])
        self.assertEqual(queue.commands(("unica-a",), runner), [
            ["pr", "--workspace", "--exclude", "unica-a", "--no-tests=pass"],
            ["queue", "-p", "unica-a", "--no-tests=pass"],
        ])
        planned = queue.commands(("unica-coder-daemon", "ureq"), runner)
        self.assertEqual(planned[1], ["queue", "-p", "unica-coder-daemon", "--no-tests=pass"])
        self.assertEqual(planned[2:], [["queue", "ledger"], ["queue", "ureq"]])
        # Фасад журнала живёт в демоне: правка нижнего слоя его не запускает.
        self.assertNotIn(["queue", "ledger"], queue.commands(("unica-coder",), runner))

    def test_a_vendored_patch_is_selected_in_the_real_workspace(self) -> None:
        directories, dependents = queue.workspace()
        self.assertEqual(directories["vendor/ureq"], "ureq")
        choice = queue.choose(["vendor/ureq/src/stream.rs"], directories, dependents)
        self.assertFalse(choice.full)
        self.assertEqual(choice.packages, ("ureq",))
        # Cargo не принимает `-p` для пакета вне рабочей области.
        runner = queue.load_runner()
        planned = queue.commands(choice.packages, runner, choice.dependents)
        self.assertIn(runner.rust_command("queue", runner.UREQ_RECYCLING_SELECTION), planned)
        for command in planned:
            self.assertNotIn(["-p", "ureq"], [command[index:index + 2] for index in range(len(command) - 1)], command)
            self.assertNotIn(["--exclude", "ureq"], [command[index:index + 2] for index in range(len(command) - 1)], command)

    def test_a_failed_diff_runs_everything(self) -> None:
        with tempfile.TemporaryDirectory() as directory, mock.patch.dict(os.environ, {"GITHUB_OUTPUT": str(Path(directory) / "o")}):
            with mock.patch.object(queue.subprocess, "check_output", side_effect=OSError("no git")):
                self.assertEqual(queue.main(["--base", "a", "--head", "b", "--dry-run"]), 0)
            self.assertEqual((Path(directory) / "o").read_text(encoding="utf-8"), "full=true\n")


class GuardTests(unittest.TestCase):
    """Правило держится на том, что Rust читает эти пути только прямым упоминанием."""

    def grep(self, text: str) -> list[str]:
        return subprocess.run(["git", "grep", "-lF", text, "--", "crates/*.rs", "vendor/*.rs"], cwd=ROOT, capture_output=True, text=True, encoding="utf-8").stdout.split()

    def test_rust_does_not_read_the_paths_that_run_only_small(self) -> None:
        # Упоминания `scripts/ci/` в комментариях и текстах ошибок не читают файл: ищется строка пути.
        for text in ("scenario-manifest", "python-sizes.toml", "CODEOWNERS", '"scripts/ci', ".github/workflows", ".agents/", '"tests/ci/', '"tests/dev/', '"docs/', "../docs/", "docs/pages", "visual-kit", '"arch/rules', '"scripts/research', '"AGENTS.md"'):
            with self.subTest(text=text):
                # `bundled_tools.rs` создаёт этот путь во временном каталоге теста, а файл репозитория не читает.
                self.assertEqual([path for path in self.grep(text) if not path.endswith("infrastructure/bundled_tools.rs")], [])

    def test_fixtures_are_really_found_by_mention(self) -> None:
        self.assertTrue(self.grep("tests/fixtures/xdto"))

    def test_a_crate_that_assembles_fixture_paths_is_selected_by_any_fixture(self) -> None:
        # Склейка из одного `tests/fixtures` прячет каталог от поиска, поэтому её ищут отдельно.
        self.assertTrue(self.grep(queue.ASSEMBLED_FIXTURES))
        asked = []

        def find(text):
            asked.append(text)
            return ["crates/coder/tests/x.rs"] if text == queue.ASSEMBLED_FIXTURES else []

        self.assertEqual(queue.choose(["tests/fixtures/xdto/a.xml"], DIRS, DEPENDENTS, find=find).packages, ("unica-coder",))
        self.assertEqual(asked, ["tests/fixtures/xdto", queue.ASSEMBLED_FIXTURES])


class WorkflowTests(unittest.TestCase):
    def setUp(self) -> None:
        jobs = yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8"))["jobs"]
        self.steps = {step.get("name"): step for step in jobs["unica-ci"]["steps"]}
        self.windows = jobs["windows-clippy"]

    def test_clippy_of_the_whole_workspace_runs_on_linux_and_windows(self) -> None:
        self.assertIn("--workspace --all-targets", self.steps["Clippy"]["run"])
        self.assertEqual(self.steps["Clippy"]["if"], "env.KEPT != 'true'")
        self.assertEqual(self.windows["if"], "github.event_name == 'merge_group'")

    def test_the_queue_selects_rust_and_keeps_the_rollback_switch(self) -> None:
        rust = self.steps["Тесты Rust"]
        self.assertEqual(rust["id"], "selection")
        self.assertIn("python scripts/ci/queue-tests.py", rust["run"])
        self.assertEqual(rust["env"]["QUEUE_FULL_TESTS"], "${{ vars.QUEUE_FULL_TESTS }}")
        self.assertEqual(rust["env"]["BASE_SHA"], "${{ github.event.merge_group.base_sha }}")

    def test_python_medium_leaves_the_queue_unless_the_choice_is_full(self) -> None:
        run = self.steps["Тесты Python"]["run"]
        self.assertIn("python scripts/ci/run-static-tests.py --queue", run)
        self.assertIn('python scripts/ci/run-tests.py --profile "$GATE_PROFILE" --ecosystem python', run)
        self.assertEqual(self.steps["Тесты Python"]["env"]["FULL"], "${{ steps.selection.outputs.full }}")


if __name__ == "__main__":
    unittest.main()
