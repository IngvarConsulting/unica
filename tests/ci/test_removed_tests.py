"""Removed Rust tests are found by diff and need a declaration before the queue (#1816)."""
from __future__ import annotations

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "scripts/ci"))
import removed_tests  # noqa: E402

LIB = '''
pub fn product() {}

#[cfg(test)]
mod tests {
    #[test]
    fn keeps() {}

    #[test]
    #[ignore = "slow"]
    fn goes_away() {}

    #[tokio::test]
    async fn async_goes_away() {}
}
'''
INTEGRATION = "#[test]\nfn whole_binary() {}\n"
# Runs a script as `__main__` with the locale encoding forced to cp1251, as on a
# Windows host with a Russian locale (#2445); UTF-8 mode, when on, wins over it.
CP1251_LOCALE = ("import locale, runpy, sys; locale.getencoding = lambda: 'cp1251'; "
                 "sys.argv = sys.argv[1:]; runpy.run_path(sys.argv[0], run_name='__main__')")


class RemovedTestsTests(unittest.TestCase):
    def repository(self, temporary: str) -> Path:
        root = Path(temporary)

        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)

        git("init", "-q", "-b", "main")
        git("config", "user.email", "ci@example.invalid")
        git("config", "user.name", "CI")
        (root / "crates/demo/src/model").mkdir(parents=True)
        (root / "crates/demo/tests").mkdir(parents=True)
        (root / "crates/demo/src/lib.rs").write_text("mod model;\nmod moved;\n#[path = \"../tests/shared.rs\"]\nmod shared;\n")
        # Includes that are no Rust file of a package are not read.
        (root / "crates/demo/src/model.rs").write_text(
            'pub mod codec;\ninclude!("table.inc");\n#[path = "../../../outside.rs"]\nmod outside;\n')
        (root / "crates/demo/src/model/codec.rs").write_text(LIB)
        (root / "crates/demo/src/moved.rs").write_text("#[test]\nfn travels() {}\n")
        # A file outside `src/` reached through `#[path]`, and one no target reaches.
        (root / "crates/demo/tests/shared.rs").write_text("#[test]\nfn shared_case() {}\n")
        (root / "crates/demo/src/orphan.rs").write_text("#[test]\nfn compiled_nowhere() {}\n")
        (root / "crates/demo/tests/contract.rs").write_text(INTEGRATION)
        # A `[[test]]` target names its binary independently of the file.
        # Non-ASCII text: `И` is 0xD0 0x98 in UTF-8, and 0x98 has no cp1251 character.
        (root / "crates/demo/Cargo.toml").write_text(encoding="utf-8", data=
            '[package]\nname = "demo"\ndescription = "Испытание снятых тестов"\n'
            '[[test]]\nname = "contract_tests"\npath = "tests/contract.rs"\n'
            # No queue lane enables `research`: its binary is never built there.
            '[[test]]\nname = "research_tests"\npath = "tests/research.rs"\nrequired-features = ["research"]\n')
        (root / "crates/demo/tests/research.rs").write_text("#[test]\nfn outside_the_queue() {}\n")
        (root / "vendor/ureq/src").mkdir(parents=True)
        (root / "vendor/ureq/Cargo.toml").write_text('[package]\nname = "ureq"\n')
        (root / "vendor/ureq/src/lib.rs").write_text("mod pool;\n")
        (root / "vendor/ureq/src/pool.rs").write_text("/* #[test] fn commented() {} */\n#[test]\nfn vendored() {}\n")
        git("add", ".")
        git("commit", "-q", "-m", "base")
        git("switch", "-q", "-c", "change")
        (root / "crates/demo/src/model/codec.rs").write_text(LIB.replace(
            '    #[test]\n    #[ignore = "slow"]\n    fn goes_away() {}\n', "").replace(
            "    #[tokio::test]\n    async fn async_goes_away() {}\n", ""))
        (root / "crates/demo/src/moved.rs").write_text("")
        (root / "crates/demo/src/model.rs").write_text("pub mod codec;\n#[test]\nfn travels() {}\n")
        (root / "crates/demo/tests/shared.rs").write_text("")
        (root / "crates/demo/src/orphan.rs").write_text("")
        (root / "crates/demo/tests/research.rs").write_text("")
        (root / "crates/demo/tests/contract.rs").unlink()
        (root / "vendor/ureq/src/pool.rs").write_text("// gone\n")
        git("add", "-A")
        git("commit", "-q", "-m", "change")
        return root

    def check(self, description: str) -> list[str]:
        with tempfile.TemporaryDirectory() as temporary:
            return removed_tests.check(self.repository(temporary), "main", description, "change")

    def test_finds_the_case_ids_the_queue_inventory_loses(self):
        with tempfile.TemporaryDirectory() as temporary:
            found = removed_tests.removed_tests(self.repository(temporary), "main", "change")
        gone = ["demo::contract_tests"]  # the package's binaries that no longer build
        self.assertEqual([(package, case, gone) for package, case, _, gone in found], [
            ("demo", "demo::contract_tests::whole_binary", gone),
            ("demo", "demo::model::codec::tests::async_goes_away", gone),
            ("demo", "demo::model::codec::tests::goes_away", gone),
            # `demo::moved::travels` moved to `demo::model::travels`: the same
            # function arrived in another module, so it is moved, not removed.
            ("demo", "demo::shared::shared_case", gone),
            ("ureq", "ureq::pool::vendored", []),
        ])

    DECLARED = ("Removes-test: demo::model::codec::tests::goes_away\r\n"
                "- Removes-test: `demo::model::codec::tests::async_goes_away`\n"
                "Removes-test: demo::moved::travels\n"
                "Removes-test: demo::shared::shared_case\n"
                "Removes-test-target: demo::contract_tests\n"
                "Removes-test: ureq::pool::vendored\n")

    def test_each_removal_needs_its_own_declaration(self):
        self.assertEqual(len(self.check("")), 5)
        self.assertEqual(self.check(self.DECLARED), [])
        # An exact name in another module of the package is a typo, not a declaration.
        wrong = self.DECLARED.replace("demo::model::codec::tests::goes_away", "demo::model::other::goes_away")
        self.assertEqual(len(self.check(wrong)), 1)

    def test_a_prefix_is_literal_as_in_the_queue(self):
        rest = self.DECLARED.replace("Removes-test: demo::model::codec::tests::goes_away\r\n", "").replace(
            "- Removes-test: `demo::model::codec::tests::async_goes_away`\n", "")
        for prefix in ("demo::model::codec::tests::*", "demo::model::*", "demo::*", "demo::model::codec::tests::a*"):
            with self.subTest(prefix=prefix):
                problems = self.check(rest + f"Removes-test: {prefix}\n")
                expected = ["goes_away"] if prefix.endswith("a*") else []
                self.assertEqual([p for p in expected if not any(p in problem for problem in problems)], [])
                self.assertEqual(len(problems), len(expected), problems)
        # A prefix of another module, or a name the queue reads as a whole, is no declaration.
        for foreign in ("demo::other::*", "demolition::model::codec::*", "demo*", "demo::model::codec"):
            with self.subTest(foreign=foreign):
                self.assertEqual(len(self.check(rest + f"Removes-test: {foreign}\n")), 2)

    def test_a_target_declaration_counts_only_for_a_binary_that_is_gone(self):
        rest = self.DECLARED.replace("Removes-test-target: demo::contract_tests\n", "")
        self.assertEqual(len(self.check(rest)), 1)
        # The library still builds: declaring it gone explains none of its cases.
        library = rest.replace("Removes-test: demo::model::codec::tests::goes_away\r\n", "") + "Removes-test-target: demo\n"
        self.assertEqual(len(self.check(library)), 2)
        self.assertEqual(len(self.check(rest + "Removes-test-target: demo::contract\n")), 1)

    def test_patterns_the_queue_reads_literally_are_refused(self):
        # The queue gate has no braces or wildcards inside a name (`{a,b}` is not one).
        rest = self.DECLARED.replace("Removes-test: demo::model::codec::tests::goes_away\r\n", "")
        for pattern in ("demo::model::{codec,other}::*", "demo::model::codec::tests::goes_?way",
                        "demo::model::*::goes_away", "demo::model::codec::tests::[ag]*"):
            with self.subTest(pattern=pattern):
                problems = self.check(rest + f"Removes-test: {pattern}\n")
                self.assertTrue(any("does not understand" in problem for problem in problems), problems)
                self.assertTrue(any("goes_away" in problem for problem in problems), problems)

    def test_agrees_with_the_queue_gate_on_every_declaration(self):
        # Mutation guard: the local rule is the queue's `explained_case`, not an approximation of it.
        import ci_rust_lanes
        binaries = ["demo", "demo::contract_tests"]
        cases = ["demo::model::codec::tests::goes_away", "demo::contract_tests::whole_binary"]
        patterns = ["demo::model::codec::tests::goes_away", "demo::model::*", "demo::*", "demo::x::*",
                    "demo::model::codec::tests::goes", "demo::contract_tests::*", "demo::contract_tests::whole_binary",
                    "demo::model::codec::tests::goes_away::*", "demolition::*"]
        for case in cases:
            for pattern in patterns:
                for gone in ([], ["demo::contract_tests"]):
                    for targets in ([], ["demo::contract_tests"], ["demo"]):
                        with self.subTest(case=case, pattern=pattern, gone=gone, targets=targets):
                            local = removed_tests.explained(case, binaries, gone, {
                                "removedTests": [pattern], "removedTargets": targets})
                            queue = ci_rust_lanes.explained_case(case, binaries, set(gone) & set(targets), [pattern])
                            self.assertEqual(local, queue)

    def test_resolves_modules_through_path_include_and_inline_blocks(self):
        source = ('#[cfg(test)]\n#[path = "../tests/x.rs"]\nmod x;\n'
                  'mod outer {\n    mod inner { #[test] fn deep() {} }\n    #[test]\n    fn shallow() {}\n}\n'
                  'include!("generated.rs");\nmod plain;\n')
        self.assertEqual(removed_tests.tests_in(source), [("outer::inner", "deep"), ("outer", "shallow")])
        children = list(removed_tests.Packages.children("crates/demo/src/a.rs", source, "demo", "a", False))
        self.assertEqual(children, [
            ("crates/demo/tests/x.rs", "demo", "a::x", True),
            ("crates/demo/src/generated.rs", "demo", "a", False),
            ("crates/demo/src/a/plain.rs", "demo", "a::plain", False),
            ("crates/demo/src/a/plain/mod.rs", "demo", "a::plain", True),
        ])

    def test_resolves_the_modules_build_rs_declares_for_a_crate_root(self):
        # crates/unica-metadata-designer/build.rs lists `src/*.rs`; each module
        # keeps `<stem>/` for its own children, and the root is not its own child.
        source = 'include!(concat!(env!("OUT_DIR"), "/a_modules.rs"));\ndeclare_modules!();\n'
        files = ["crates/demo/src/lib.rs", "crates/demo/src/one.rs", "crates/demo/src/one/child.rs"]
        children = list(removed_tests.Packages.children("crates/demo/src/lib.rs", source, "demo", "", True, files))
        self.assertEqual(children, [("crates/demo/src/one.rs", "demo", "one", False)])
        nested = list(removed_tests.Packages.children(
            "crates/demo/src/one.rs", "mod child;\n", "demo", "one", False, files))
        self.assertEqual(nested[0][:3], ("crates/demo/src/one/child.rs", "demo", "one::child"))

    def test_resolves_the_modules_build_rs_declares_from_the_module_directory(self):
        # crates/unica-coder/build.rs declares `mod <stem>;` for each `*.rs` of
        # the module's own directory; the include must reach exactly those files.
        source = 'include!(concat!(env!("OUT_DIR"), "/a_modules.rs"));\n#[test]\nfn own() {}\n'
        files = ["crates/demo/src/a/one.rs", "crates/demo/src/a/two.rs", "crates/demo/src/a/deep/three.rs",
                 "crates/demo/src/b/other.rs", "crates/demo/src/a.rs"]
        children = list(removed_tests.Packages.children("crates/demo/src/a.rs", source, "demo", "a", False, files))
        self.assertEqual(children, [
            ("crates/demo/src/a/one.rs", "demo", "a::one", True),
            ("crates/demo/src/a/two.rs", "demo", "a::two", True),
        ])
        # Reached through `#[path]` (mod_rs), the file still owns `a/`, not `src/`.
        moved = list(removed_tests.Packages.children("crates/demo/src/a.rs", source, "demo", "a", True, files))
        self.assertEqual(moved, children)
        literal = list(removed_tests.Packages.children(
            "crates/demo/src/a.rs", 'include!(concat!(env!("OUT_DIR"), "/other.rs"));\n', "demo", "a", False, files))
        self.assertEqual(literal, [])

    def run_script(self, *args: str, utf8_mode: bool) -> subprocess.CompletedProcess:
        """The script as a pull request check runs it, under UTF-8 mode or a cp1251 locale."""
        environment = {**os.environ, "PYTHONUTF8": "1" if utf8_mode else "0",
                       "PYTHONIOENCODING": "utf-8" if utf8_mode else "cp1251"}
        environment.pop("GITHUB_ACTIONS", None)
        return subprocess.run([sys.executable, "-c", CP1251_LOCALE, str(REPO_ROOT / "scripts/ci/removed_tests.py"),
                               *args], cwd=REPO_ROOT, env=environment, capture_output=True, timeout=120)

    def test_reads_git_output_as_utf8_whatever_the_locale(self):
        # Under cp1251 a non-ASCII manifest broke the read of `git show`,
        # and the check fell with an AttributeError instead of a verdict.
        with tempfile.TemporaryDirectory() as temporary:
            arguments = ("--repo-root", str(self.repository(temporary)), "--base", "main", "--head", "change",
                         "--description", "")
            utf8 = self.run_script(*arguments, utf8_mode=True)
            cp1251 = self.run_script(*arguments, utf8_mode=False)
        self.assertEqual(utf8.returncode, 1, utf8.stderr)
        self.assertEqual(len(utf8.stdout.splitlines()), 5, utf8.stdout)
        self.assertEqual((cp1251.returncode, cp1251.stdout, cp1251.stderr), (utf8.returncode, utf8.stdout, b""))

    def test_a_failing_git_command_is_reported_not_raised(self):
        with tempfile.TemporaryDirectory() as temporary:
            subprocess.run(["git", "init", "-q"], cwd=temporary, check=True, capture_output=True)
            result = self.run_script("--repo-root", temporary, "--base", "no-such-revision", "--description", "",
                                     utf8_mode=False)
        stderr = result.stderr.decode("cp1251")
        self.assertEqual(result.returncode, 2, stderr)
        self.assertNotIn("Traceback", stderr)
        self.assertIn("git merge-base no-such-revision HEAD failed", stderr)

    def test_full_runs_check_removed_tests_against_the_inventory_baseline(self) -> None:
        workflow = yaml.safe_load((REPO_ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8"))
        steps = workflow["jobs"]["unica-ci"]["steps"]
        names = [step.get("run", "") for step in steps]
        index = next(i for i, run in enumerate(names) if "removed_tests.py" in run)
        step = steps[index]
        self.assertEqual(step["if"], "${{ !cancelled() && steps.classify.outputs.rust_required == 'true' }}")
        self.assertIn("--baseline .build/ci-baseline/baseline.json", step["run"])
        # After verify: its own failure never hides or confuses the inventory verdict.
        self.assertLess(next(i for i, run in enumerate(names) if "--phase verify" in run), index)
        self.assertIn("pull-requests", workflow["jobs"]["unica-ci"]["permissions"])

    def test_the_baseline_gives_the_base_and_the_declarations_merged_since(self) -> None:
        document = {
            "source": {"headSha": "a" * 40},
            "explanations": {"removedTests": ["pkg::bin::gone"], "removedTargets": ["pkg::old"], "reset": False},
        }
        base, description = removed_tests.baseline_scope(document)
        self.assertEqual(base, "a" * 40)
        self.assertEqual(description, "Removes-test: pkg::bin::gone\nRemoves-test-target: pkg::old")

    def test_no_baseline_and_a_declared_reset_have_nothing_to_compare(self) -> None:
        self.assertIsNone(removed_tests.baseline_scope({"source": None, "explanations": {}}))
        self.assertIsNone(removed_tests.baseline_scope({"source": {"headSha": "a" * 40}, "explanations": {"reset": True}}))


PYTHON_BASE = '''import unittest


class Shared:
    def test_inherited(self):
        pass


class KeptTests(Shared, unittest.TestCase):
    def test_keeps(self):
        pass

    def test_goes_away(self):
        pass

    def helper(self):
        pass


class TravelsTests(unittest.TestCase):
    def test_travels(self):
        pass


class NotATest:
    def test_looks_like_one(self):
        pass
'''


class RemovedPythonTestsTests(unittest.TestCase):
    """Python tests are named as `unittest` names them and checked like Rust ones."""

    def repository(self, temporary: str) -> Path:
        root = Path(temporary)

        def git(*args):
            subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)

        git("init", "-q", "-b", "main")
        git("config", "user.email", "ci@example.invalid")
        git("config", "user.name", "CI")
        (root / "tests/ci").mkdir(parents=True)
        (root / "tests/ci/test_source.py").write_text(PYTHON_BASE, encoding="utf-8")
        (root / "tests/ci/helpers.py").write_text("class HelperTests:\n    def test_not_collected(self):\n        pass\n")
        git("add", ".")
        git("commit", "-q", "-m", "base")
        git("switch", "-q", "-c", "change")
        changed = PYTHON_BASE.replace("    def test_goes_away(self):\n        pass\n\n", "")
        changed = changed.replace(
            "class TravelsTests(unittest.TestCase):\n    def test_travels(self):\n        pass\n\n\n", "")
        (root / "tests/ci/test_source.py").write_text(changed, encoding="utf-8")
        (root / "tests/ci/test_target.py").write_text(
            "import unittest\n\n\nclass TravelsTests(unittest.TestCase):\n    def test_travels(self):\n        pass\n",
            encoding="utf-8")
        (root / "tests/ci/helpers.py").write_text("")
        git("add", ".")
        git("commit", "-q", "-m", "change")
        return root

    def test_the_inventory_names_tests_as_unittest_does_without_importing(self) -> None:
        self.assertEqual(
            removed_tests.python_tests_in(PYTHON_BASE, "tests.ci.test_source"),
            [
                "tests.ci.test_source.KeptTests.test_goes_away",
                "tests.ci.test_source.KeptTests.test_inherited",
                "tests.ci.test_source.KeptTests.test_keeps",
                "tests.ci.test_source.TravelsTests.test_travels",
            ],
        )

    def test_a_removed_test_needs_a_declaration_and_a_moved_one_does_not(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self.repository(temporary)
            self.assertEqual(
                removed_tests.removed_python_tests(root, "main", "change"),
                ["tests.ci.test_source.KeptTests.test_goes_away"],
            )
            problems = removed_tests.check(root, "main", "", "change")
            self.assertEqual(len(problems), 1, problems)
            self.assertIn("python: test tests.ci.test_source.KeptTests.test_goes_away disappeared", problems[0])
            for declaration in ("Removes-test: tests.ci.test_source.KeptTests.test_goes_away",
                                "- Removes-test: `tests.ci.test_source.KeptTests.*`"):
                with self.subTest(declaration=declaration):
                    self.assertEqual(removed_tests.check(root, "main", declaration, "change"), [])

    def test_a_pair_that_arrives_twice_is_no_move(self) -> None:
        lost = ["tests.ci.test_a.Tests.test_x"]
        self.assertEqual(removed_tests.python_moved(lost, ["tests.ci.test_b.Tests.test_x"]), set(lost))
        self.assertEqual(removed_tests.python_moved(
            lost, ["tests.ci.test_b.Tests.test_x", "tests.ci.test_c.Tests.test_x"]), set())
        self.assertEqual(removed_tests.python_moved(lost, ["tests.ci.test_b.Other.test_x"]), set())


class NightlyInventoryTests(unittest.TestCase):
    """Pull request: removals counted also from the night's older inventory commit."""

    def test_the_night_inventory_names_what_the_pull_request_base_cannot_see(self):
        from unittest import mock
        with tempfile.TemporaryDirectory() as temporary:
            root = RemovedTestsTests.repository(self, temporary)
            night = subprocess.run(["git", "rev-parse", "main"], cwd=root, check=True,
                                   capture_output=True, text=True).stdout.strip()
            environment = {"GITHUB_REPOSITORY": "o/r", "GITHUB_REPOSITORY_ID": "1"}

            def document(removed):
                return {"source": {"headSha": night}, "explanations": {"removedTests": removed}}

            with mock.patch.dict(os.environ, environment),                  mock.patch.object(removed_tests.baseline, "baseline", return_value=document([])):
                problems = removed_tests.nightly_problems(root, "large", "HEAD")
            self.assertTrue(problems)
            self.assertTrue(all(problem.startswith("since the large inventory: ") for problem in problems))
            # Declarations merged since the night (or this pull request's own) explain it.
            declared = [line.split("Removes-test: ", 1)[1].split("'", 1)[0]
                        for line in problems if "Removes-test: " in line]
            with mock.patch.dict(os.environ, environment),                  mock.patch.object(removed_tests.baseline, "baseline", return_value=document(declared)):
                self.assertEqual(removed_tests.nightly_problems(root, "large", "HEAD"), [])
            # No inventory found: a warning, not a verdict.
            with mock.patch.dict(os.environ, environment),                  mock.patch.object(removed_tests.baseline, "baseline", side_effect=RuntimeError("gh api failed")):
                self.assertEqual(removed_tests.nightly_problems(root, "large", "HEAD"), [])


if __name__ == "__main__":
    unittest.main()
