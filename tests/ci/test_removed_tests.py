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
        (root / "crates/demo/Cargo.toml").write_text(
            '[package]\nname = "demo"\n[[test]]\nname = "contract_tests"\npath = "tests/contract.rs"\n'
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
            # Moved to another module: a new case ID, the old one is gone.
            ("demo", "demo::moved::travels", gone),
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
        self.assertEqual(len(self.check("")), 6)
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
        library = rest.replace("Removes-test: demo::moved::travels\n", "") + "Removes-test-target: demo\n"
        self.assertEqual(len(self.check(library)), 2)
        self.assertEqual(len(self.check(rest + "Removes-test-target: demo::contract\n")), 1)

    def test_patterns_the_queue_reads_literally_are_refused(self):
        # The queue gate has no braces or wildcards inside a name (#1955 used `{a,b}`).
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

    def test_pull_request_runs_check_removed_tests_against_their_base(self):
        workflow = yaml.safe_load((REPO_ROOT / ".github/workflows/unica-plugin-release.yml").read_text())
        steps = workflow["jobs"]["unica-ci"]["steps"]
        names = [step.get("run", "") for step in steps]
        index = next(i for i, run in enumerate(names) if "removed_tests.py" in run)
        step = steps[index]
        self.assertEqual(step["if"], "${{ !cancelled() && github.event_name == 'pull_request' && steps.classify.outputs.rust_required == 'true' }}")
        self.assertIn("--base HEAD^1", step["run"])
        # After verify: its own failure never hides or confuses the inventory verdict.
        self.assertLess(next(i for i, run in enumerate(names) if "--phase verify" in run), index)
        self.assertIn("pull-requests", workflow["jobs"]["unica-ci"]["permissions"])


if __name__ == "__main__":
    unittest.main()
