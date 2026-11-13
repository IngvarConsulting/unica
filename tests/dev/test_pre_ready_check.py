"""Проверки перед снятием PR с черновика: разбор итогов, сверка с базой, тесты вне ОС.

Выводы `cargo`, `rustfmt` и `unittest` — поддельные строки; сборка, `git`
и сеть не нужны. Главный поток проверяется с подменой запуска проверок.
"""

from __future__ import annotations

import importlib.util
import io
import os
import sys
import shutil
import subprocess
import tempfile
import threading
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "dev" / "pre-ready-check.py"
SPEC = importlib.util.spec_from_file_location("pre_ready_check", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
sys.modules["pre_ready_check"] = CHECK
SPEC.loader.exec_module(CHECK)

WINDOWS = CHECK.host_facts("win32")
LINUX = CHECK.host_facts("linux")

LIBTEST = """\
running 4 tests
test fs::tests::copies ... ok
test fs::tests::links ... ok
test fs::tests::moves ... FAILED
test fs::tests::slow ... ignored, heavy

successes:

---- fs::tests::links stdout ----
unica-host-capability-skip: symlink: no privilege

failures:

---- fs::tests::moves stdout ----
thread 'fs::tests::moves' panicked
test result: FAILED. 2 passed; 1 failed; 1 ignored
"""

UNITTEST = """\
test_a (test_mod.ATests.test_a) ... ok
test_b (test_mod.ATests.test_b) ... FAIL
test_c (test_mod.ATests.test_c)
Docstring line ... skipped 'needs unix'
test_d (test_mod.BTests.test_d) ... ERROR

======================================================================
FAIL: test_b (test_mod.ATests.test_b)
----------------------------------------------------------------------
ERROR: test_d (test_mod.BTests.test_d) (case='x')
"""

RUST_SOURCE = """\
#[cfg(test)]
mod tests {
    #[test]
    fn plain() {
        let text = "{ unbalanced";
        assert!(text.len() > 0);
    }

    #[cfg(unix)]
    #[test]
    fn unix_only() {
        let brace = '{';
    }

    #[test]
    #[cfg_attr(not(target_os = "linux"), ignore = "host capability symlink: {why}")]
    fn linux_only() {}

    #[cfg(all(unix, not(target_os = "macos")))]
    mod nested {
        #[test] fn nested_case() {
            let raw = r#"
            { "a": [1 }
            "#;
        }
    }

    #[test]
    fn links() {}
}
"""


class ParsingTests(unittest.TestCase):
    def test_libtest_failures_and_host_skips(self):
        result = CHECK.parse_libtest(LIBTEST, 101)
        self.assertEqual(result.failures, ["fs::tests::moves"])
        self.assertEqual(result.skipped, ["fs::tests::links", "fs::tests::slow"])

    def test_a_build_failure_is_a_failure_without_line_numbers(self):
        output = ("error[E0425]: cannot find value `x` in this scope\n --> src/lib.rs:3:5\n"
                  "error: could not compile `unica-x` (lib test) due to 1 previous error\n")
        result = CHECK.parse_libtest(output, 101)
        self.assertEqual(result.failures, ["сборка: cannot find value `x` in this scope"])
        self.assertEqual(CHECK.parse_libtest("killed\n", 1).failures, ["сборка или запуск не прошли, код 1"])

    def test_a_binary_that_dies_mid_run_is_a_failure_beyond_known_ones(self):
        output = "test a ... FAILED\nthread 'b' has overflowed its stack\n"
        self.assertEqual(CHECK.parse_libtest(output, 3221225725).failures,
                         ["a", "прогон оборвался или итог неполон, код 3221225725"])
        split = "test a ... FAILED\ntest b ... child output\nFAILED\ntest result: FAILED. 0 passed; 2 failed\n"
        self.assertEqual(CHECK.parse_libtest(split, 101).failures, ["a", "прогон оборвался или итог неполон, код 101"])

    def test_clippy_keys_ignore_lines_and_absolute_prefixes(self):
        root = Path(tempfile.gettempdir()).resolve()
        output = (f"{root / 'crates' / 'a' / 'src' / 'lib.rs'}:10:5: error: unused variable: `x`\n"
                  "crates/a/src/lib.rs:20:1: error[E0308]: mismatched types\n"
                  "error: could not compile `a` (lib) due to 2 previous errors\n")
        result = CHECK.parse_clippy(output, 101, root)
        self.assertEqual(result.failures, ["crates/a/src/lib.rs: mismatched types",
                                           "crates/a/src/lib.rs: unused variable: `x`"])
        self.assertEqual(CHECK.parse_clippy("warning: something\n", 0, root).failures, [])

    def test_fmt_lists_files_in_both_rustfmt_shapes(self):
        root = Path(tempfile.gettempdir()).resolve()
        output = (f"Diff in \\\\?\\{root / 'crates' / 'a' / 'src' / 'lib.rs'} at line 4:\n"
                  "Diff in crates/b/src/x.rs:12:\n")
        self.assertEqual(CHECK.parse_fmt(output, 1, root).failures, ["crates/a/src/lib.rs", "crates/b/src/x.rs"])
        self.assertEqual(CHECK.parse_fmt("", 0, root).failures, [])

    def test_unittest_failures_errors_and_skips(self):
        result = CHECK.parse_unittest(UNITTEST, 1)
        self.assertEqual(result.failures, ["test_mod.ATests.test_b", "test_mod.BTests.test_d"])
        self.assertEqual(result.skipped, ["test_mod.ATests.test_c"])
        self.assertEqual(CHECK.parse_unittest("ImportError\n", 1).failures, ["набор упал без списка тестов, код 1"])


class BaselineTests(unittest.TestCase):
    def test_only_failures_absent_on_the_base_are_new(self):
        current = {
            "python:tests/ci": CHECK.Result(["m.A.known", "m.A.fresh"]),
            "test:unica-x": CHECK.Result(["a::b"]),
            "fmt": CHECK.Result([]),
        }
        base = {"python:tests/ci": ["m.A.known", "m.A.fixed_on_branch"], "test:unica-x": ["a::b"]}
        self.assertEqual(CHECK.new_failures(current, base), {"python:tests/ci": ["m.A.fresh"]})

    def test_one_more_repeat_of_a_known_clippy_error_is_new(self):
        current = {"clippy:x": CHECK.Result(["a.rs: needless return"] * 2)}
        self.assertEqual(CHECK.new_failures(current, {"clippy:x": ["a.rs: needless return"]}),
                         {"clippy:x": ["a.rs: needless return"]})
        self.assertEqual(CHECK.new_failures(current, {"clippy:x": ["a.rs: needless return"] * 2}), {})

    def test_the_cache_belongs_to_one_base_and_platform(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "baseline.json"
            CHECK.save_cache(path, "aaa", {"fmt": ["x.rs"]})
            self.assertEqual(CHECK.load_cache(path, "aaa"), {"fmt": ["x.rs"]})
            self.assertEqual(CHECK.load_cache(path, "bbb"), {})
            with mock.patch.object(CHECK.sys, "platform", "other"):
                self.assertEqual(CHECK.load_cache(path, "aaa"), {})

    def test_baseline_runs_only_checks_missing_from_the_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            cache_dir = Path(directory)
            CHECK.save_cache(cache_dir / "baseline.json", "sha", {"fmt": []})
            ran = []

            def fake_run(check, root, env, log_dir):
                ran.append(check.key)
                return CHECK.Result(["base failure"])

            checks = [CHECK.Check("fmt", "fmt"), CHECK.Check("python:tests/ci", "python", "tests/ci")]
            with mock.patch.object(CHECK, "run_check", fake_run), mock.patch.object(CHECK, "add_base_tree"), \
                    mock.patch.object(CHECK, "remove_base_tree"), redirect_stdout(io.StringIO()):
                result = CHECK.baseline(checks, "sha", cache_dir, {})
            self.assertEqual(ran, ["python:tests/ci"])
            self.assertEqual(result, {"fmt": [], "python:tests/ci": ["base failure"]})
            self.assertEqual(CHECK.load_cache(cache_dir / "baseline.json", "sha"), result)


class RecheckTests(unittest.TestCase):
    def recheck(self, check, fresh, output, returncode):
        completed = mock.Mock(stdout=output, returncode=returncode)
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.object(CHECK.subprocess, "run", return_value=completed) as run, \
                redirect_stdout(io.StringIO()):
            result = CHECK.recheck(check, fresh, REPO_ROOT, {}, Path(directory))
        return result, run

    def test_a_test_that_passes_on_repeat_is_unstable_not_new(self):
        check = CHECK.Check("python:tests/ci", "python", "tests/ci")
        output = ("test_x (m.A.test_x) ... ok\ntest_y (m.A.test_y) ... FAIL\n\n"
                  "FAIL: test_y (m.A.test_y)\n")
        (stable, unstable), run = self.recheck(check, ["m.A.test_x", "m.A.test_y"], output, 1)
        self.assertEqual((stable, unstable), (["m.A.test_y"], ["m.A.test_x"]))
        self.assertEqual(run.call_args.args[0][-2:], ["m.A.test_x", "m.A.test_y"])

    def test_a_repeat_that_breaks_itself_keeps_every_failure(self):
        check = CHECK.Check("test:unica-x", "test", "unica-x")
        result, _ = self.recheck(check, ["a::b"], "error: could not compile\n", 101)
        self.assertEqual(result, (["a::b"], []))

    def test_rust_repeat_selects_exact_names_and_build_failures_are_not_repeated(self):
        check = CHECK.Check("test:unica-x", "test", "unica-x")
        (stable, unstable), run = self.recheck(check, ["a::b"], "test a::b ... ok\ntest result: ok. 1 passed; 0 failed\n", 0)
        self.assertEqual((stable, unstable), ([], ["a::b"]))
        self.assertEqual(run.call_args.args[0][-3:], ["--show-output", "--exact", "a::b"])
        with mock.patch.object(CHECK.subprocess, "run") as run:
            self.assertEqual(CHECK.recheck(check, ["сборка: boom"], REPO_ROOT, {}, REPO_ROOT),
                             (["сборка: boom"], []))
        run.assert_not_called()


def git_repository(directory: Path) -> str:
    """Репозиторий с одним коммитом; возвращает его sha."""
    environment = {**os.environ, "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
                   "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid"}
    for command in (["init", "-q"], ["commit", "-q", "--allow-empty", "-m", "base"]):
        subprocess.run(["git", "-c", "commit.gpgsign=false", *command], cwd=directory, check=True,
                       env=environment, capture_output=True)
    return CHECK.git(directory, "rev-parse", "HEAD").strip()


@unittest.skipUnless(shutil.which("git"), "нужен git")
class BaseTreeTests(unittest.TestCase):
    def setUp(self):
        self.directory = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.directory, ignore_errors=True)
        self.repository = self.directory / "repo"
        self.repository.mkdir()
        self.sha = git_repository(self.repository)

    def test_a_git_refusal_carries_its_code_and_text(self):
        with self.assertRaises(CHECK.Refusal) as caught:
            CHECK.git(self.repository, "worktree", "add", "--detach", str(self.directory / "x"), "no-such-ref")
        message = str(caught.exception)
        self.assertRegex(message, r"код \d+: \S")
        self.assertIn("no-such-ref", message)

    def test_parallel_base_trees_of_one_repository_do_not_fail(self):
        errors = []

        def create(index):
            try:
                for _ in range(2):
                    tree = self.directory / f"cache-{index}" / "base"
                    CHECK.add_base_tree(self.repository, tree, self.sha)
                    CHECK.remove_base_tree(self.repository, tree)
            except Exception as error:  # noqa: BLE001 - итог потока проверяется ниже
                errors.append(error)

        threads = [threading.Thread(target=create, args=(index,)) for index in range(3)]
        with redirect_stdout(io.StringIO()):
            for thread in threads:
                thread.start()
            for thread in threads:
                thread.join()
        self.assertEqual(errors, [])

    def test_a_held_lock_names_its_holder_and_what_to_do(self):
        held, release = threading.Event(), threading.Event()

        def holder():
            with CHECK.worktree_lock(self.repository):
                held.set()
                release.wait(10)

        thread = threading.Thread(target=holder)
        thread.start()
        self.addCleanup(thread.join)
        self.addCleanup(release.set)
        self.assertTrue(held.wait(10))
        with redirect_stdout(io.StringIO()), self.assertRaises(CHECK.Refusal) as caught:
            with CHECK.worktree_lock(self.repository, wait=0.2, poll=0.05):
                pass
        message = str(caught.exception)
        self.assertIn(f"pid {os.getpid()}", message)
        self.assertIn("Дождитесь", message)

    def test_a_released_lock_is_taken_again_and_its_file_stays(self):
        with CHECK.worktree_lock(self.repository, wait=0.2):
            pass
        with CHECK.worktree_lock(self.repository, wait=0.2):
            pass
        self.assertTrue((self.repository / ".git" / CHECK.WORKTREE_LOCK).exists())


class DiffTests(unittest.TestCase):
    def test_changed_lines_and_crates(self):
        diff = ("diff --git a/crates/x/src/lib.rs b/crates/x/src/lib.rs\n"
                "--- a/crates/x/src/lib.rs\n+++ b/crates/x/src/lib.rs\n"
                "@@ -3,0 +4,2 @@\n+a\n+b\n@@ -10,2 +12,0 @@\n-c\n-d\n"
                "--- a/crates/y/old.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-x\n"
                "--- a/tests/ci/test_m.py\n+++ b/tests/ci/test_m.py\n@@ -7 +7 @@\n-a\n+b\n")
        lines = CHECK.changed_lines(diff)
        self.assertEqual(lines, {"crates/x/src/lib.rs": {4, 5, 12}, "crates/y/old.rs": set(),
                                 "tests/ci/test_m.py": {7}})
        self.assertEqual(CHECK.touched_crates([*lines, "Cargo.toml", "crates/README.md"]),
                         ["crates/x", "crates/y"])

    def test_plan_tests_libraries_and_test_targets_of_crates_without_one(self):
        packages = {"crates/a": ("unica-a", True), "crates/b-tests": ("unica-b-tests", False)}
        checks, notes = CHECK.plan(["crates/a", "crates/b-tests", "crates/unica-research"], packages)
        commands = {check.key: check.command("py") for check in checks}
        self.assertEqual(commands["test:unica-a"][:5], ["cargo", "test", "-p", "unica-a", "--lib"])
        self.assertEqual(commands["test:unica-b-tests"][:5], ["cargo", "test", "-p", "unica-b-tests", "--tests"])
        self.assertIn("-D", commands["clippy:unica-a"])
        self.assertEqual({"fmt", "modules-fmt", "python:tests/ci", "python:tests/dev"} - set(commands), set())
        self.assertEqual(len(notes), 1)


class CfgTests(unittest.TestCase):
    def test_predicates_on_windows_and_linux(self):
        cases = {
            "unix": (False, True),
            "windows": (True, False),
            'target_os = "linux"': (False, True),
            'not(target_os = "windows")': (False, True),
            'all(unix, not(target_os = "macos"))': (False, True),
            'any(windows, feature = "x")': (True, None),
            'feature = "x"': (None, None),
            "all(test, unix)": (False, True),
        }
        for predicate, (windows, linux) in cases.items():
            with self.subTest(predicate=predicate):
                self.assertIs(CHECK.eval_cfg(predicate, WINDOWS), windows)
                self.assertIs(CHECK.eval_cfg(predicate, LINUX), linux)

    def test_rust_tests_carry_spans_and_what_disables_them_here(self):
        tests = {test.name: test for test in CHECK.rust_tests(RUST_SOURCE, WINDOWS)}
        self.assertEqual(list(tests), ["plain", "unix_only", "linux_only", "nested_case", "links"])
        self.assertEqual((tests["plain"].start, tests["plain"].end), (3, 7))
        self.assertEqual(tests["unix_only"].excluded_by, "cfg(unix)")
        self.assertIn("ignore", tests["linux_only"].ignored_by)
        self.assertEqual(tests["nested_case"].excluded_by, 'cfg(all(unix, not(target_os = "macos")))')
        self.assertIsNone(tests["links"].excluded_by)
        self.assertEqual(tests["links"].start, 28)
        on_linux = CHECK.rust_tests(RUST_SOURCE, LINUX)
        self.assertEqual([t.name for t in on_linux if t.excluded_by or t.ignored_by], [])

    def test_a_cfg_on_the_mod_declaration_excludes_the_module_file(self):
        sources = {
            "crates/x/src/lib.rs": "mod platform;\n",
            "crates/x/src/platform/mod.rs": "#[cfg(unix)]\n#[allow(dead_code)]\nmod links;\nmod common;\n",
        }
        self.assertEqual(CHECK.declared_exclusion("crates/x/src/platform/links.rs", sources.get, WINDOWS), "cfg(unix)")
        self.assertIsNone(CHECK.declared_exclusion("crates/x/src/platform/common.rs", sources.get, WINDOWS))
        self.assertIsNone(CHECK.declared_exclusion("crates/x/src/platform/links.rs", sources.get, LINUX))
        nested = {**sources, "crates/x/src/platform/links.rs": "mod deep;\n"}
        self.assertEqual(CHECK.declared_exclusion("crates/x/src/platform/links/deep.rs", nested.get, WINDOWS),
                         "cfg(unix)")

    def test_a_file_level_cfg_excludes_every_test(self):
        source = '#![cfg(target_os = "linux")]\n\n#[test]\nfn one() {}\n'
        self.assertEqual([t.excluded_by for t in CHECK.rust_tests(source, WINDOWS)], ['cfg(target_os = "linux")'])


class UnrunTests(unittest.TestCase):
    def test_only_tests_touched_by_the_diff_are_reported(self):
        python_source = (
            "import unittest\n\n\n"
            "class ATests(unittest.TestCase):\n"
            "    def test_a(self):\n        pass\n\n"
            "    def test_c(self):\n        pass\n\n\n"
            "@unittest.skipUnless(False, 'unix')\n"
            "class BTests(unittest.TestCase):\n"
            "    def test_d(self):\n        pass\n"
        )
        sources = {"crates/fs/src/lib.rs": RUST_SOURCE, "tests/ci/test_mod.py": python_source}
        lines = {
            "crates/fs/src/lib.rs": {11, 17, 29},  # unix_only, linux_only, links
            "tests/ci/test_mod.py": {9, 12},  # test_c body, BTests decorator
        }
        results = {
            "test:unica-fs": CHECK.Result([], ["tests::links"]),
            "python:tests/ci": CHECK.Result([], ["test_mod.ATests.test_c", "test_mod.ATests.test_a",
                                                 "test_mod.BTests.test_d"]),
        }
        report = CHECK.unrun_tests(lines, sources.get, results, {"crates/fs": "unica-fs"}, WINDOWS)
        self.assertEqual(report, [
            "crates/fs/src/lib.rs::unix_only — #[cfg(unix)]",
            'crates/fs/src/lib.rs::linux_only — #[cfg_attr(not(target_os = "linux"), '
            'ignore = "host capability symlink: why")]',
            "crates/fs/src/lib.rs::links — пропущен на хосте",
            "tests/ci/test_mod.py::ATests.test_c — skipped",
            "tests/ci/test_mod.py::BTests.test_d — skipped",
        ])


class MainTests(unittest.TestCase):
    def run_main(self, results, base):
        out, err = io.StringIO(), io.StringIO()
        with tempfile.TemporaryDirectory() as directory, \
                mock.patch.dict(os.environ, {"CARGO_TARGET_DIR": directory}), \
                mock.patch.object(CHECK, "git", side_effect=lambda root, *args: {
                    "merge-base": "abc123\n", "status": ""}.get(args[0], "")), \
                mock.patch.object(CHECK, "workspace_packages", return_value={}), \
                mock.patch.object(CHECK, "run_check", side_effect=lambda check, *rest: results.get(
                    check.key, CHECK.Result([]))), \
                mock.patch.object(CHECK, "baseline", return_value=base), \
                mock.patch.object(CHECK, "recheck", side_effect=lambda check, fresh, *rest: (fresh, [])), \
                redirect_stdout(out), redirect_stderr(err):
            code = CHECK.main([])
        return code, out.getvalue()

    def test_known_failures_pass_and_new_ones_fail(self):
        results = {"python:tests/ci": CHECK.Result(["m.A.known"])}
        code, output = self.run_main(results, {"python:tests/ci": ["m.A.known"]})
        self.assertEqual(code, 0)
        self.assertIn("Новые падения: нет", output)
        results = {"python:tests/ci": CHECK.Result(["m.A.known", "m.A.fresh"])}
        code, output = self.run_main(results, {"python:tests/ci": ["m.A.known"]})
        self.assertEqual(code, 1)
        self.assertIn("m.A.fresh", output)
        self.assertIn("Новые падения: 1", output)

    def test_without_the_build_directory_the_check_refuses(self):
        err = io.StringIO()
        with mock.patch.dict(os.environ), redirect_stderr(err):
            os.environ.pop("CARGO_TARGET_DIR", None)
            self.assertEqual(CHECK.main([]), 2)
        self.assertIn("CARGO_TARGET_DIR", err.getvalue())


if __name__ == "__main__":
    unittest.main()
