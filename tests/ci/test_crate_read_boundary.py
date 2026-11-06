"""The guard counts Rust reads outside the crate and refuses new ones."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "crate_read_boundary", ROOT / "scripts/ci/check-crate-read-boundary.py"
)
guard = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = guard
SPEC.loader.exec_module(guard)


class Workspace:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.members: list[str] = []
        (root / "tests/fixtures/shared").mkdir(parents=True)
        (root / "tests/fixtures/shared/a.xml").write_text("<a/>", encoding="utf-8")
        (root / "tests/fixtures/other").mkdir()

    def crate(self, name: str, source: str, metadata: str = "", path: str = "src/lib.rs") -> str:
        directory = f"crates/{name}"
        target = self.root / directory / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(source, encoding="utf-8")
        manifest = f'[package]\nname = "{name}"\n'
        if metadata:
            manifest += f"\n[package.metadata.unica]\n{metadata}\n"
        (self.root / directory / "Cargo.toml").write_text(manifest, encoding="utf-8")
        if directory not in self.members:
            self.members.append(directory)
        members = ", ".join(f'"{member}"' for member in self.members)
        (self.root / "Cargo.toml").write_text(f"[workspace]\nmembers = [{members}]\n", encoding="utf-8")
        return directory

    def reads(self, name: str) -> list:
        crates = {crate.name: crate for crate in guard.load_crates(self.root)}
        return guard.undeclared_reads(self.root, crates[name])


class CrateReadBoundaryTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.workspace = Workspace(Path(temporary.name))

    def targets(self, source: str, path: str = "src/lib.rs") -> list[str]:
        self.workspace.crate("sample", source, path=path)
        return [read.target for read in self.workspace.reads("sample")]

    def test_the_real_workspace_has_no_reads_over_its_baselines(self) -> None:
        self.assertEqual(guard.check(ROOT), [])
        crates = guard.load_crates(ROOT)
        if any(crate.baseline for crate in crates):
            self.assertTrue(any(guard.crate_reads(ROOT, crate) for crate in crates))

    def test_an_input_is_compared_in_normal_form(self) -> None:
        self.workspace.crate(
            "sample",
            'const A: &str = include_str!("../../../tests/fixtures/shared/a.xml");\n',
            'inputs = ["./tests//fixtures/shared/"]',
        )
        self.assertEqual(guard.check(self.workspace.root), [])

    def test_a_read_outside_the_crate_is_counted_in_every_form(self) -> None:
        cases = {
            'concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/shared/a.xml")': "tests/fixtures/shared/a.xml",
            'Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/shared")': "tests/fixtures/shared",
            'Path::new(env!("CARGO_MANIFEST_DIR"))\n    .join("../../plugins/unica")': "plugins/unica",
            'include_str!("../../../tests/fixtures/shared/a.xml")': "tests/fixtures/shared/a.xml",
            'include_bytes!("../../other/src/lib.rs")': "crates/other/src/lib.rs",
            'repo_root().join("tests/fixtures/shared/a.xml")': "tests/fixtures/shared/a.xml",
            'Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap()': ".",
            'Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")': ".",
            'Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)': ".",
            'Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("tests/fixtures/shared")': "tests/fixtures/shared",
            'format!("{}/../other/a.txt", env!("CARGO_MANIFEST_DIR"))': "crates/other/a.txt",
        }
        for expression, target in cases.items():
            with self.subTest(expression):
                self.assertEqual(self.targets(f"fn f() {{ let _ = {expression}; }}\n"), [target])

    def test_a_manifest_variable_covers_its_block_and_not_a_path_built_from_it(self) -> None:
        cases = (
            'fn f() { let m = env!("CARGO_MANIFEST_DIR"); }\nfn g(m: &Path) { let _ = m.parent(); }',
            'fn f() { let m = Path::new(env!("CARGO_MANIFEST_DIR")); let p = m.join("a/b"); let _ = p.parent(); }',
        )
        for source in cases:
            with self.subTest(source):
                self.assertEqual(self.targets(source), [])

    def test_a_variable_bound_to_the_manifest_directory_is_followed(self) -> None:
        cases = {
            'const BASE: &str = env!("CARGO_MANIFEST_DIR");\nfn f() { let _ = Path::new(BASE).join("../../plugins/unica"); }': ["plugins/unica"],
            'fn f() { let m = Path::new(env!("CARGO_MANIFEST_DIR")); let r = m.parent().unwrap(); }': ["."],
            'fn f() { let m = std::env::var("CARGO_MANIFEST_DIR").unwrap(); let p = PathBuf::from(m).join("../../plugins"); }': ["plugins"],
            'fn f() { let m = env!("CARGO_MANIFEST_DIR"); let r = Path::new(m); let p = r.join("../../plugins"); }': ["plugins"],
            'fn f() { let m = env!("CARGO_MANIFEST_DIR"); let p = Path::new(m).join("..").join("x"); }': ["."],
        }
        for source, targets in cases.items():
            with self.subTest(source):
                self.assertEqual(self.targets(source), targets)

    def test_what_stays_in_the_crate_or_is_only_data_is_not_counted(self) -> None:
        cases = (
            '// see "../../tests/fixtures/shared/a.xml"\nfn f() {}',
            '/* include_str!("../../../tests/fixtures/shared") */ fn f() {}',
            '#[path = "../tests/platform/a.rs"] mod a;',
            'const A: &str = include_str!("../tests/data/a.txt");',
            'fn f() { let _ = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/own/a.xml"); }',
            'fn f() { header.set_link_name("../../escape").unwrap(); }',
            'fn f() { let _ = plugin_root.join("..").join("cache"); }',
            'fn f() { if name.contains("..") {} }',
            'fn f() { panic!("differs from tests/fixtures/shared"); }',
            'fn f() { let _ = env!("CARGO_MANIFEST_DIR"); }',
            'fn f() { panic!("tests/fixtures/shared/a.xml is missing"); }',
            'fn f() { let _ = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/own").parent(); }',
            'fn f() { let m = env!("CARGO_MANIFEST_DIR"); let _ = other.join("../x"); }',
        )
        for source in cases:
            with self.subTest(source):
                self.assertEqual(self.targets(source), [])

    def test_a_test_module_named_fixtures_is_read_as_code(self) -> None:
        source = 'macro_rules! m { () => { concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/shared/", "a.xml") }; }\n'
        self.assertEqual(self.targets(source, path="tests/fixtures/mod.rs"), ["tests/fixtures/shared/"])

    def test_a_crate_fixture_of_the_same_name_is_its_own_file(self) -> None:
        directory = self.workspace.crate("sample", 'fn f() { let _ = root.join("tests/fixtures/own.txt"); }\n')
        self.assertEqual(self.targets('fn f() { let _ = root.join("tests/fixtures/own.txt"); }\n'), ["tests/fixtures/own.txt"])
        own = self.workspace.root / directory / "tests/fixtures/own.txt"
        own.parent.mkdir(parents=True)
        own.write_text("own", encoding="utf-8")
        self.assertEqual(self.targets('fn f() { let _ = root.join("tests/fixtures/own.txt"); }\n'), [])

    def test_a_string_in_a_raw_literal_is_read_and_a_quote_in_a_comment_does_not_derail(self) -> None:
        source = '// a " quote\nfn f() { let _ = r#"../../tests/fixtures/shared/a.xml"#; let c = \'"\'; }\n'
        self.assertEqual(self.targets(source), ["tests/fixtures/shared/a.xml"])

    def test_the_kind_of_a_read_follows_its_place(self) -> None:
        read = 'const A: &str = include_str!("../../../tests/fixtures/shared/a.xml");\n'
        self.workspace.crate("sample", "#[cfg(test)]\nmod tests {\n" + read + "}\n")
        self.assertEqual([r.kind for r in self.workspace.reads("sample")], ["test"])
        self.workspace.crate("sample", read)
        self.assertEqual([r.kind for r in self.workspace.reads("sample")], ["production"])
        (self.workspace.root / "crates/sample/src/lib.rs").unlink()
        self.workspace.crate("sample", read, path="tests/a.rs")
        self.assertEqual([r.kind for r in self.workspace.reads("sample")], ["test"])

    def test_a_declared_input_allows_the_read_and_a_new_undeclared_one_fails(self) -> None:
        source = (
            'const A: &str = include_str!("../../../tests/fixtures/shared/a.xml");\n'
            'const B: &str = include_str!("../../../tests/fixtures/other/b.xml");\n'
        )
        self.workspace.crate("sample", source)
        self.assertEqual(len(guard.check(self.workspace.root)), 3)

        self.workspace.crate("sample", source, 'inputs = ["tests/fixtures/shared"]')
        errors = guard.check(self.workspace.root)
        self.assertIn("1 reads outside the crate, baseline 0", errors[0])
        self.assertIn("crates/sample/src/lib.rs:2 -> tests/fixtures/other/b.xml", errors[1])

        self.workspace.crate(
            "sample", source, 'inputs = ["tests/fixtures/shared"]\nread-outside-baseline = 1'
        )
        self.assertEqual(guard.check(self.workspace.root), [])

    def test_an_input_is_matched_by_path_segments(self) -> None:
        self.workspace.crate(
            "sample",
            'const A: &str = include_str!("../../../tests/fixtures/shared-more/a.xml");\n',
            'inputs = ["tests/fixtures/shared"]',
        )
        self.assertEqual([r.target for r in self.workspace.reads("sample")], ["tests/fixtures/shared-more/a.xml"])

    def test_an_input_must_be_an_existing_path_below_the_root(self) -> None:
        for item, message in (
            ("tests/fixtures/absent", "does not exist"),
            (".", "below the root"),
            ("../outside", "below the root"),
            ("/absolute", "below the root"),
        ):
            with self.subTest(item):
                self.workspace.crate("sample", "fn f() {}\n", f'inputs = ["{item}"]')
                self.assertIn(message, guard.check(self.workspace.root)[0])

    def test_the_walk_up_has_no_input_that_declares_it(self) -> None:
        self.workspace.crate(
            "sample",
            'fn f() { let _ = Path::new(env!("CARGO_MANIFEST_DIR")).parent(); }\n',
            'inputs = ["tests/fixtures/shared"]',
        )
        self.assertEqual([r.target for r in self.workspace.reads("sample")], ["."])

    def test_a_lowered_count_passes_and_a_written_baseline_is_exact(self) -> None:
        source = 'const A: &str = include_str!("../../../tests/fixtures/shared/a.xml");\n'
        self.workspace.crate("sample", source, "read-outside-baseline = 5")
        self.assertEqual(guard.check(self.workspace.root), [])
        guard.write_baseline(self.workspace.root)
        manifest = (self.workspace.root / "crates/sample/Cargo.toml").read_text(encoding="utf-8")
        self.assertIn("read-outside-baseline = 1\n", manifest)
        self.workspace.crate("sample", "fn f() {}\n", "read-outside-baseline = 1")
        guard.write_baseline(self.workspace.root)
        manifest = (self.workspace.root / "crates/sample/Cargo.toml").read_text(encoding="utf-8")
        self.assertNotIn("read-outside-baseline", manifest)

    def test_the_baseline_edit_keeps_the_rest_of_the_manifest(self) -> None:
        self.assertEqual(
            guard.with_baseline('[package]\nname = "a"\n', 3),
            '[package]\nname = "a"\n\n[package.metadata.unica]\nread-outside-baseline = 3\n',
        )
        self.assertEqual(
            guard.with_baseline('[package.metadata.unica]\ninputs = ["x"]\n', 2),
            '[package.metadata.unica]\nread-outside-baseline = 2\ninputs = ["x"]\n',
        )
        self.assertEqual(
            guard.with_baseline('[package]\r\nname = "a"\r\n', 1),
            '[package]\r\nname = "a"\r\n\r\n[package.metadata.unica]\r\nread-outside-baseline = 1\r\n',
        )
        self.assertEqual(guard.with_baseline('[package]\nname = "a"\n', 0), '[package]\nname = "a"\n')

    def test_the_report_names_each_crate_with_reads_and_the_total(self) -> None:
        self.workspace.crate("sample", 'const A: &str = include_str!("../../../tests/fixtures/shared/a.xml");\n')
        self.workspace.crate("clean", "fn f() {}\n")
        report = guard.report(self.workspace.root)
        self.assertIn("sample", report)
        self.assertNotIn("clean", report)
        self.assertRegex(report.splitlines()[-1], r"^total\s+1\s")


if __name__ == "__main__":
    unittest.main()
