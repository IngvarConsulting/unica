"""Размер тестов Python объявлен вне кода: манифест существует, конструкции покрыты.

Признак `medium` — тест зовёт `cargo`, берёт продукт через `ensure_binary`
или поднимает сокет: модуль с такими конструкциями обязан быть назван
в манифесте, а каждая запись манифеста — существовать в дереве. Наших имён страж не знает.
"""

from __future__ import annotations

import ast
import importlib.util
import io
import contextlib
import re
import tempfile
import tomllib
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = REPO_ROOT / ".config" / "python-sizes.toml"
SUITES = ("tests/ci", "tests/dev")
# Вызов, а не упоминание: `"cargo"` в ожидании мока или в утверждении — не сборка.
CARGO = re.compile(r'subprocess\.(?:run|Popen|check_output|check_call|call)\(\s*\[\s*"cargo"')
SOCKET = re.compile(r"^\s*(import socket|from socket import)", re.M)
PREPARED_BINARY = "scripts.ci.prepared_binary"


def uses_ensure_binary(source: str) -> bool:
    """Без манифеста `ensure_binary` собирает продукт: его потребитель тоже `medium`."""
    tree = ast.parse(source)
    module_aliases = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.ImportFrom) and node.module == PREPARED_BINARY:
            if any(alias.name in ("ensure_binary", "*") for alias in node.names):
                return True
        elif isinstance(node, ast.ImportFrom) and node.module == "scripts.ci":
            module_aliases |= {alias.asname or alias.name for alias in node.names if alias.name == "prepared_binary"}
        elif isinstance(node, ast.Import):
            module_aliases |= {alias.asname or alias.name for alias in node.names if alias.name == PREPARED_BINARY}
    return bool(module_aliases) and any(
        isinstance(node, ast.Attribute) and node.attr == "ensure_binary" for node in ast.walk(tree)
    )


def load_runner():
    spec = importlib.util.spec_from_file_location("run_unittest", REPO_ROOT / "scripts" / "ci" / "run-unittest.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def entries() -> list[str]:
    return tomllib.loads(MANIFEST.read_text(encoding="utf-8"))["medium"]["entries"]


def module_path(module: str) -> Path | None:
    for suite in SUITES:
        candidate = REPO_ROOT / suite / f"{module}.py"
        if candidate.is_file():
            return candidate
    return None


class PythonSizeManifestTests(unittest.TestCase):
    def test_every_entry_names_a_module_class_or_test_that_exists(self) -> None:
        """Переименование не должно молча терять размер: запись без цели — отказ."""
        for entry in entries():
            with self.subTest(entry=entry):
                module, *rest = entry.split(".")
                path = module_path(module)
                self.assertIsNotNone(path, f"модуля {module} нет ни в одном наборе")
                tree = ast.parse(path.read_text(encoding="utf-8"))
                classes = {node.name: node for node in tree.body if isinstance(node, ast.ClassDef)}
                if rest:
                    self.assertIn(rest[0], classes)
                if len(rest) > 1:
                    methods = {n.name for n in classes[rest[0]].body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))}
                    self.assertIn(rest[1], methods)

    def test_modules_that_build_or_connect_are_declared(self) -> None:
        """`cargo`, `ensure_binary` или сокет в модуле — продукт собирается или запускается: не `small` молча."""
        declared = {entry.split(".")[0] for entry in entries()}
        for suite in SUITES:
            for path in sorted((REPO_ROOT / suite).glob("test_*.py")):
                text = path.read_text(encoding="utf-8", errors="replace")
                if CARGO.search(text) or uses_ensure_binary(text) or SOCKET.search(text):
                    with self.subTest(module=path.stem):
                        self.assertIn(path.stem, declared)

    def test_ensure_binary_consumers_are_recognised_in_every_import_form(self) -> None:
        consumers = (
            "from scripts.ci.prepared_binary import binary_path, ensure_binary\n",
            "from scripts.ci.prepared_binary import (\n    binary_path,\n    ensure_binary,\n)\n",
            "from scripts.ci.prepared_binary import *\n",
            "import scripts.ci.prepared_binary as product\nproduct.ensure_binary(ROOT)\n",
            "from scripts.ci import prepared_binary\nprepared_binary.ensure_binary(ROOT)\n",
        )
        mentions = (
            "from scripts.ci.prepared_binary import binary_path\n",
            '"""from scripts.ci.prepared_binary import ensure_binary"""\n',
            "import scripts.ci.prepared_binary as product\nproduct.binary_path(ROOT)\n",
        )
        for source in consumers:
            with self.subTest(consumer=source):
                self.assertTrue(uses_ensure_binary(source))
        for source in mentions:
            with self.subTest(mention=source):
                self.assertFalse(uses_ensure_binary(source))

    def test_runner_admits_only_the_requested_sizes(self) -> None:
        """Ворота `pr` не видят `medium`: тест исключён из плана, а не пропущен."""
        runner = load_runner()
        root = Path(tempfile.mkdtemp(prefix="sizes-"))
        (root / "test_alpha.py").write_text(
            "import unittest\n"
            "class Cheap(unittest.TestCase):\n    def test_one(self):\n        pass\n"
            "class Heavy(unittest.TestCase):\n    def test_two(self):\n        pass\n    def test_three(self):\n        pass\n",
            encoding="utf-8",
        )
        manifest = root / "sizes.toml"
        manifest.write_text('[medium]\nentries = ["test_alpha.Heavy", "test_alpha.Cheap.test_missing"]\n', encoding="utf-8")

        def plan(admit: str) -> list[str]:
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                runner.main(["-s", str(root), "--plan-only", "--sizes", str(manifest), "--admit", admit])
            return sorted(line for line in out.getvalue().splitlines() if line)

        self.assertEqual(plan("small"), ["test_alpha.Cheap.test_one"])
        self.assertEqual(plan("medium"), ["test_alpha.Heavy.test_three", "test_alpha.Heavy.test_two"])
        self.assertEqual(len(plan("small,medium")), 3)
        sizes = runner.Sizes.load(manifest)
        self.assertEqual(sizes.size_of("test_alpha.Heavy.test_two"), "medium")
        self.assertEqual(sizes.size_of("test_alpha.Cheap.test_one"), "small")


if __name__ == "__main__":
    unittest.main()
