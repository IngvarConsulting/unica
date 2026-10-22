"""Removed Rust tests are found by diff and need a declaration before the queue (#1816)."""
from __future__ import annotations

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
        (root / "crates/demo/src/model/codec.rs").write_text(LIB)
        (root / "crates/demo/src/moved.rs").write_text("#[test]\nfn travels() {}\n")
        (root / "crates/demo/tests/contract.rs").write_text(INTEGRATION)
        # A `[[test]]` target names its binary independently of the file.
        (root / "crates/demo/Cargo.toml").write_text(
            '[package]\nname = "demo"\n[[test]]\nname = "contract_tests"\npath = "tests/contract.rs"\n')
        (root / "vendor/ureq/src").mkdir(parents=True)
        (root / "vendor/ureq/Cargo.toml").write_text('[package]\nname = "ureq"\n')
        (root / "vendor/ureq/src/pool.rs").write_text("/* #[test] fn commented() {} */\n#[test]\nfn vendored() {}\n")
        git("add", ".")
        git("commit", "-q", "-m", "base")
        git("switch", "-q", "-c", "change")
        (root / "crates/demo/src/model/codec.rs").write_text(LIB.replace(
            '    #[test]\n    #[ignore = "slow"]\n    fn goes_away() {}\n', "").replace(
            "    #[tokio::test]\n    async fn async_goes_away() {}\n", ""))
        (root / "crates/demo/src/moved.rs").unlink()
        (root / "crates/demo/src/landed.rs").write_text("#[test]\nfn travels() {}\n")
        (root / "crates/demo/tests/contract.rs").unlink()
        (root / "vendor/ureq/src/pool.rs").write_text("// gone\n")
        git("add", "-A")
        git("commit", "-q", "-m", "change")
        return root

    def check(self, description: str) -> list[str]:
        with tempfile.TemporaryDirectory() as temporary:
            return removed_tests.check(self.repository(temporary), "main", description, "change")

    def test_finds_removed_test_functions_but_not_moved_ones(self):
        with tempfile.TemporaryDirectory() as temporary:
            found = removed_tests.removed_tests(self.repository(temporary), "main", "change")
        self.assertEqual(found, [
            ("demo", "async_goes_away", ("demo", "model::codec")),
            ("demo", "goes_away", ("demo", "model::codec")),
            ("demo", "whole_binary", ("demo::contract_tests", "")),
            ("ureq", "vendored", ("ureq", "pool")),
        ])

    VENDOR = "Removes-test: ureq::pool::vendored\n"

    def test_each_removal_needs_its_own_declaration(self):
        self.assertEqual(len(self.check("")), 4)
        declared = ("Removes-test: demo::model::codec::tests::goes_away\r\n"
                    "- Removes-test: `demo::model::codec::tests::async_goes_away`\n"
                    "Removes-test-target: demo::contract_tests\n" + self.VENDOR)
        self.assertEqual(self.check(declared), [])
        # An exact name in another module of the package is a typo, not a declaration.
        wrong = declared.replace("demo::model::codec::tests::goes_away", "demo::model::other::goes_away")
        self.assertEqual(len(self.check(wrong)), 1)

    def test_a_prefix_must_agree_with_the_module_of_the_file(self):
        target = "Removes-test-target: demo::contract_tests\n" + self.VENDOR
        self.assertEqual(self.check(target + "Removes-test: demo::model::codec::tests::*\n"), [])
        self.assertEqual(self.check(target + "Removes-test: demo::model::*\n"), [])
        foreign = self.check(target + "Removes-test: demo::other::*\nRemoves-test: demolition::model::codec::*\n")
        self.assertEqual(len(foreign), 2)
        self.assertTrue(all("goes_away" in problem for problem in foreign))
        # A target name of the library or a file-derived name does not explain the binary,
        # and `demo*` is no declaration in the queue either.
        self.assertEqual(len(self.check(self.VENDOR + "Removes-test-target: demo\nRemoves-test: demo::model::*\n")), 1)
        self.assertEqual(len(self.check(self.VENDOR + "Removes-test-target: demo::contract\nRemoves-test: demo::model::*\n")), 1)
        self.assertEqual(len(self.check(target + "Removes-test: demo*\n")), 2)

    def test_patterns_the_queue_reads_literally_are_refused(self):
        # The queue gate has no braces or wildcards inside a name (#1955 used `{a,b}`).
        target = "Removes-test-target: demo::contract_tests\n" + self.VENDOR
        for pattern in ("demo::model::{codec,other}::*", "demo::model::codec::tests::goes_?way",
                        "demo::model::*::goes_away", "demo::model::codec::tests::[ag]*"):
            with self.subTest(pattern=pattern):
                problems = self.check(target + f"Removes-test: {pattern}\n")
                self.assertTrue(any("does not understand" in problem for problem in problems), problems)
                self.assertTrue(any("goes_away" in problem for problem in problems), problems)

    def test_an_unknown_module_path_needs_the_name_or_a_prefix_below_the_package(self):
        def declared(*patterns):
            return removed_tests.declarations_in("".join(f"Removes-test: {item}\n" for item in patterns))
        explained = removed_tests.explained
        self.assertTrue(explained("demo", "case", None, declared("demo::x::tests::case")))
        self.assertTrue(explained("demo", "case", None, declared("demo::x::*")))
        self.assertFalse(explained("demo", "case", None, declared("demo::*")))
        self.assertFalse(explained("demo", "case", None, declared("demo::x::other")))
        self.assertFalse(explained("demo", "case", None, declared("demo::x::{case,other}")))

    def test_pull_request_runs_check_removed_tests_against_their_base(self):
        workflow = yaml.safe_load((REPO_ROOT / ".github/workflows/unica-plugin-release.yml").read_text())
        steps = workflow["jobs"]["test-rust-platforms"]["steps"]
        names = [step.get("run", "") for step in steps]
        index = next(i for i, run in enumerate(names) if "removed_tests.py" in run)
        step = steps[index]
        self.assertEqual(step["if"], "${{ !cancelled() && github.event_name == 'pull_request' }}")
        self.assertIn("--base HEAD^1", step["run"])
        # After verify: its own failure never hides or confuses the inventory verdict.
        self.assertLess(next(i for i, run in enumerate(names) if "--phase verify" in run), index)
        self.assertIn("pull-requests", workflow["jobs"]["test-rust-platforms"]["permissions"])


if __name__ == "__main__":
    unittest.main()
