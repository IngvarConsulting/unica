"""The guard counts version literals in Rust production code and refuses new ones."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "version_literals", ROOT / "scripts/ci/check-version-literals.py"
)
guard = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = guard
SPEC.loader.exec_module(guard)


class Workspace:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.members: list[str] = []

    def crate(self, name: str, files: dict[str, str], baseline: int = 0) -> None:
        directory = f"crates/{name}"
        for path, source in files.items():
            target = self.root / directory / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(source, encoding="utf-8")
        manifest = f'[package]\nname = "{name}"\n'
        if baseline:
            manifest += f"\n[package.metadata.unica]\nversion-literal-baseline = {baseline}\n"
        (self.root / directory / "Cargo.toml").write_text(manifest, encoding="utf-8")
        self.members.append(directory)
        members = ", ".join(f'"{member}"' for member in self.members)
        (self.root / "Cargo.toml").write_text(f"[workspace]\nmembers = [{members}]\n", encoding="utf-8")


class VersionLiteralTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.workspace = Workspace(Path(temporary.name))

    def test_the_real_workspace_has_no_literals_over_its_baselines(self) -> None:
        self.assertEqual(guard.check(ROOT), [])

    def test_a_new_version_comparison_over_the_baseline_fails(self) -> None:
        """rule: INV.SOURCE.PROFILE-CAPABILITIES"""
        source = 'fn writable(version: &str) -> bool { version == "2.20" }\n'
        self.workspace.crate("sample", {"src/lib.rs": source}, baseline=0)
        errors = guard.check(self.workspace.root)
        self.assertEqual(len(errors), 1)
        self.assertIn("sample: 1 version literals", errors[0])
        self.assertIn("INV.SOURCE.PROFILE-CAPABILITIES", errors[0])

    def test_tests_comments_and_owners_are_not_counted(self) -> None:
        files = {
            "src/lib.rs": (
                "// the 8.3.27 line and \"2.20\" in a comment\n"
                "pub fn line() -> u32 { 27 }\n"
                "#[cfg(test)]\nmod tests {\n    const V: &str = \"2.20\";\n}\n"
            ),
            "src/reader_tests.rs": 'const V: &str = "2.21";\n',
            "tests/integration.rs": 'const V: &str = "2.22";\n',
            "src/dialect/v2_20.rs": 'pub const VERSION: &str = "2.20";\n',
        }
        self.workspace.crate("sample", files)
        self.assertEqual(guard.check(self.workspace.root), [])
        self.assertEqual(guard.report(self.workspace.root), "")

    def test_every_class_of_version_literal_is_counted(self) -> None:
        source = "\n".join(
            [
                'let a = "2.20";',
                'let b = "8.3.27";',
                "let c = PLATFORM_XML_8_3_27_FORMAT_2_20;",
                "let d = form_event_catalog_8_3_27();",
                'let e = r#"<x version="2.20"/>"#;',
                "let f = ACTIVE_FORMAT_PROFILE.export_format;",
                "let g = Format2_20;",
                'let h = "8.5.4";',
            ]
        )
        found = guard.source_literals("crates/sample/src/lib.rs", source)
        self.assertEqual({item.line for item in found}, set(range(1, 9)))

    def test_a_lower_count_passes_and_write_baseline_lowers_it(self) -> None:
        source = 'fn writable(version: &str) -> bool { version == "2.20" }\n'
        self.workspace.crate("sample", {"src/lib.rs": source}, baseline=3)
        self.assertEqual(guard.check(self.workspace.root), [])
        guard.write_baseline(self.workspace.root)
        manifest = (self.workspace.root / "crates/sample/Cargo.toml").read_text(encoding="utf-8")
        self.assertIn("version-literal-baseline = 1", manifest)


if __name__ == "__main__":
    unittest.main()
