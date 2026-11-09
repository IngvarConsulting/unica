from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts/ci"))
import crate_declarations  # noqa: E402


def load(name: str, path: str):
    spec = importlib.util.spec_from_file_location(name, ROOT / path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


components = load("component_boundaries", "scripts/ci/check-component-boundaries.py")
platform = load("rust_platform_boundary", "scripts/ci/check-rust-platform-boundary.py")


class CrateDeclarationTests(unittest.TestCase):
    def test_declarations_name_the_guards_package_sets(self) -> None:
        """The sets a guard derives from declarations are the ones it names today."""
        declared = crate_declarations.repository()
        short = lambda names: frozenset(name.removeprefix("unica-") for name in names)  # noqa: E731
        # Today the declarations hold exactly the crates the guards held by name.
        self.assertEqual(short(crate_declarations.packages(declared, component="family")), components.FAMILY_FLOOR)
        self.assertEqual(short(crate_declarations.packages(declared, component="abstraction")), components.ABSTRACTIONS)
        self.assertEqual(short(crate_declarations.packages(declared, component="coordinator")), components.COORDINATOR_FLOOR)
        self.assertEqual(components.FAMILIES, components.FAMILY_FLOOR)
        self.assertEqual(components.PORT_CALLERS, components.COORDINATOR_FLOOR)
        designer = frozenset(name for name in declared if platform.DESIGNER_CODEC_PACKAGE.fullmatch(name))
        self.assertEqual(crate_declarations.packages(declared, file_io="domain"), platform.DOMAIN_FLOOR)
        self.assertEqual(crate_declarations.packages(declared, file_io="none"), platform.FILE_IO_FLOOR | designer)
        self.assertEqual(platform.DOMAIN_COMPONENTS, platform.DOMAIN_FLOOR)
        self.assertEqual(platform.FILE_IO_COMPONENTS - designer, platform.FILE_IO_FLOOR)

    def test_guards_derive_their_sets_from_the_tree_they_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name, unica in (("unica-probe", 'component = "coordinator"\nfile-io = "domain"'),):
                (root / "crates" / name).mkdir(parents=True)
                (root / "crates" / name / "Cargo.toml").write_text(
                    f'[package]\nname = "{name}"\n\n[package.metadata.unica]\n{unica}\n', encoding="utf-8"
                )
            try:
                components.use_declarations(root)
                platform.use_declarations(root)
                self.assertIn("probe", components.PORT_CALLERS)
                self.assertIn("unica-probe", platform.DOMAIN_COMPONENTS)
                self.assertTrue(components.edge_errors("unica-probe", "unica-adapter-lexical", "dependencies"))
                self.assertTrue(platform.is_file_io_free_package("unica-probe"))
            finally:
                components.use_declarations(ROOT)
                platform.use_declarations(ROOT)

    def test_reads_component_and_file_io_of_each_crate(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "crates/demo").mkdir(parents=True)
            (root / "crates/demo/Cargo.toml").write_text(
                '[package]\nname = "unica-demo"\n\n[package.metadata.unica]\ncomponent = "service"\nfile-io = "none"\n',
                encoding="utf-8",
            )
            (root / "crates/bare").mkdir()
            (root / "crates/bare/Cargo.toml").write_text('[package]\nname = "unica-bare"\n', encoding="utf-8")
            declared = crate_declarations.read(root)
        self.assertEqual(declared["unica-demo"], crate_declarations.Declaration("service", "none"))
        self.assertEqual(declared["unica-bare"], crate_declarations.Declaration(None, None))
        self.assertEqual(crate_declarations.packages(declared, file_io="none"), {"unica-demo"})


if __name__ == "__main__":
    unittest.main()
