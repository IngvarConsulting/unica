"""The format pass reaches the modules that build.rs declares."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location(
    "discovered_modules_format", ROOT / "scripts/ci/check-discovered-modules-format.py"
)
checker = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = checker
SPEC.loader.exec_module(checker)

INCLUDE = 'include!(concat!(env!("OUT_DIR"), "/a_modules.rs"));\n'


class DiscoveredModulesFormatTests(unittest.TestCase):
    def write(self, root: Path, files: dict[str, str]) -> list[str]:
        for name, text in files.items():
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text, encoding="utf-8")
        return sorted(files)

    def test_a_file_declares_the_directory_named_after_it(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = self.write(root, {
                "crates/a/src/lib.rs": "mod owner;\n",
                "crates/a/src/owner.rs": INCLUDE,
                "crates/a/src/owner/one.rs": "",
                "crates/a/src/owner/deep/two.rs": "",
                "crates/a/src/other.rs": "",
            })
            self.assertEqual(checker.discovered_modules(root, sources), ["crates/a/src/owner/one.rs"])

    def test_a_crate_root_declares_its_own_directory_without_itself(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = self.write(root, {
                "crates/a/src/lib.rs": INCLUDE,
                "crates/a/src/one.rs": "",
                "crates/a/src/one/child.rs": "",
            })
            self.assertEqual(checker.discovered_modules(root, sources), ["crates/a/src/one.rs"])

    def test_another_include_declares_nothing(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = self.write(root, {
                "crates/a/src/lib.rs": 'include!(concat!(env!("OUT_DIR"), "/table.rs"));\n',
                "crates/a/src/one.rs": "",
            })
            self.assertEqual(checker.discovered_modules(root, sources), [])

    def test_the_real_tree_reaches_the_inverted_crate(self) -> None:
        modules = checker.discovered_modules(ROOT, checker.tracked_sources(ROOT))
        self.assertIn("crates/unica-metadata-designer/src/edit.rs", modules)
        self.assertNotIn("crates/unica-metadata-designer/src/lib.rs", modules)


if __name__ == "__main__":
    unittest.main()
