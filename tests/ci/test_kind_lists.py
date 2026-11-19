"""Списки видов объектов собираются из объявлений: страж и его проверка."""
from __future__ import annotations

import importlib.util
from pathlib import Path
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("kind_lists", ROOT / "scripts/ci/check-kind-lists.py")
guard = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = guard
SPEC.loader.exec_module(guard)

TAGS = [f"Kind{index}" for index in range(8)]


def declaration(tag: str, root: bool = True) -> str:
    place = "    root: Some(RootPlace { child_objects: 1, tree: None }),\n" if root else ""
    return (
        f"//! «{tag}».\nuse super::*;\n\npub(super) const KIND: KindDeclaration = KindDeclaration {{\n"
        f"{place}    ..KindDeclaration::new(\"{tag}\")\n}};\n"
    )


class KindListTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.kinds_dir = self.root / guard.DECLARATIONS
        self.kinds_dir.mkdir(parents=True)
        for tag in TAGS:
            self.declare(f"{tag.lower()}.rs", tag)
        self.declare("member.rs", "Member", root=False)

    def declare(self, file: str, tag: str, root: bool = True) -> None:
        (self.kinds_dir / file).write_text(declaration(tag, root), encoding="utf-8")

    def listed(self, source: str) -> list[tuple[int, set[str]]]:
        kinds, errors = guard.declared(self.root)
        self.assertEqual(errors, [])
        return guard.lists(source, kinds)

    def test_the_real_workspace_lists_no_kinds_by_hand(self) -> None:
        self.assertEqual(guard.check(ROOT), [])

    def test_a_list_of_three_quarters_of_root_kinds_is_refused(self) -> None:
        # Восемь видов корня: порог — шесть.
        six = ", ".join(f'"{tag}"' for tag in TAGS[:6])
        five = ", ".join(f'"{tag}"' for tag in TAGS[:5])
        self.assertEqual(self.listed(f"const A: &[&str] = &[{six}];\n"), [(1, set(TAGS[:6]))])
        self.assertEqual(self.listed(f"const A: &[&str] = &[{five}];\n"), [])

    def test_variants_match_arms_and_multiline_entries_count(self) -> None:
        arms = "\n".join(f"        NodeKind::{tag} => Some(&[\n            (\"x\", \"y\"),\n        ])," for tag in TAGS)
        found = self.listed(f"fn f(kind: NodeKind) {{\n    match kind {{\n{arms}\n    }}\n}}\n")
        self.assertEqual([(line, len(named)) for line, named in found], [(2, 8)])
        enum = "\n".join(f"    {tag}," for tag in TAGS)
        self.assertEqual(len(self.listed(f"pub enum Mine {{\n{enum}\n}}\n")), 1)

    def test_member_kinds_comments_and_strings_with_brackets_do_not_count(self) -> None:
        self.assertEqual(self.listed('const A: &[&str] = &["Member"; 6];\n'), [])
        commented = " ".join(f'"{tag}"' for tag in TAGS)
        self.assertEqual(self.listed(f"// {commented}\n/* {commented} */\nfn f() {{}}\n"), [])
        braces = "\n".join(f'    ("{{", "{tag}"),' for tag in TAGS[:3])
        self.assertEqual(self.listed(f'const A: &[(&str, &str)] = &[\n{braces}\n    (r#"}}"#, "x"),\n];\n'), [])

    def test_only_the_narrowest_list_is_reported(self) -> None:
        six = ", ".join(f'"{tag}"' for tag in TAGS[:6])
        found = self.listed(f"mod m {{\n    fn f() {{\n        let a = [{six}];\n    }}\n}}\n")
        self.assertEqual([line for line, _ in found], [3])

    def test_a_kind_declared_twice_is_refused(self) -> None:
        self.declare("kind0_again.rs", "Kind0")
        _, errors = guard.declared(self.root)
        self.assertEqual(len(errors), 1)
        self.assertIn("`Kind0` is already declared", errors[0])


if __name__ == "__main__":
    unittest.main()
