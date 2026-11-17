"""A fixture's Drop does not panic while a test already panics (#3065).

A test that fails unwinds through the guards that clean up after it. A guard
whose `drop` panics then panics a second time, and Rust aborts the whole test
binary: the other tests of that binary print no result, and the run reads as
a crash instead of one failed test.

The detector below reads every `impl Drop` in test code. A body that can panic
(`.unwrap()`, `.expect(`, `assert*!`, `panic!`, `unreachable!`) must ask
`std::thread::panicking()` first and report instead of panicking while a panic
is in progress. A poisoned lock is recovered with
`unwrap_or_else(PoisonError::into_inner)`, which never panics.

A panic hidden in a function the body calls is outside what a source check
sees; such a guard calls a fallible variant and reports its error the same way.
"""
from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "crates"

DROP_IMPL = re.compile(r"^[ \t]*impl(?:<[^>]*>)?\s+Drop\s+for\s+([A-Za-z0-9_]+)", re.M)
TEST_MODULE = re.compile(
    r"#\[cfg\([^\]]*\btest\b[^\]]*\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{"
)
OWN_TEST_CFG = re.compile(r"#\[cfg\([^\]]*\btest\b[^\]]*\)\]\s*$")
PANICS = re.compile(r"\.unwrap\(\)|\.expect\(|\bassert(?:_eq|_ne)?!|\bpanic!|\bunreachable!")
TEST_PATH = re.compile(r"(?:^|/)tests?/|_tests?\.rs$|(?:^|/)tests\.rs$|test_support")


def strip_comments(text: str) -> str:
    """Comments and string contents vanish; offsets stay put."""
    out = []
    index = 0
    length = len(text)
    while index < length:
        char = text[index]
        if text.startswith("//", index):
            end = text.find("\n", index)
            end = length if end == -1 else end
            out.append(" " * (end - index))
            index = end
        elif text.startswith("/*", index):
            end = text.find("*/", index + 2)
            end = length if end == -1 else end + 2
            out.append(re.sub(r"[^\n]", " ", text[index:end]))
            index = end
        elif char == '"':
            end = index + 1
            while end < length and text[end] != '"':
                end += 2 if text[end] == "\\" else 1
            out.append('"' + re.sub(r"[^\n]", " ", text[index + 1:end]) + '"')
            index = end + 1
        else:
            out.append(char)
            index += 1
    return "".join(out)


def block_end(text: str, open_brace: int) -> int:
    depth = 0
    for index in range(open_brace, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return index
    return len(text)


def test_spans(text: str) -> list[tuple[int, int]]:
    return [(match.start(), block_end(text, match.end() - 1)) for match in TEST_MODULE.finditer(text)]


TEST_MODULE_FILE = re.compile(
    r"#\[cfg\([^\]]*\btest\b[^\]]*\)\]\s*(?:#\[path\s*=\s*\"([^\"]+)\"\]\s*)?"
    r"(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;"
)
INNER_TEST_CFG = re.compile(r"^\s*#!\[cfg\([^\]]*\btest\b[^\]]*\)\]", re.M)


def test_module_roots(sources: dict[str, str]) -> set[str]:
    """Files and directories compiled only for tests, by their declarations.

    A module declared under `#[cfg(test)]` or a test-support feature, and a
    file whose inner attribute says so, are test code with all they contain.
    """
    roots = set()
    for relative, source in sources.items():
        path = Path(relative)
        if INNER_TEST_CFG.search(source):
            roots.add(relative)
        directory = path.parent if path.name in {"mod.rs", "lib.rs", "main.rs"} else path.parent / path.stem
        for match in TEST_MODULE_FILE.finditer(source):
            explicit, name = match.groups()
            if explicit:
                target = path.parent / explicit
                roots.add(target.as_posix())
                # A file loaded by `#[path]` keeps its children beside it.
                roots.add((target.parent / target.stem).as_posix() + "/")
                continue
            roots.add((directory / f"{name}.rs").as_posix())
            roots.add((directory / name / "mod.rs").as_posix())
            roots.add((directory / name).as_posix() + "/")
    return roots


def under_test_root(relative: str, roots: set[str]) -> bool:
    return relative in roots or any(root.endswith("/") and relative.startswith(root) for root in roots)


def panicking_drops(relative: str, source: str, in_test_file: bool = False) -> list[str]:
    """Names of guards in test code whose Drop may panic during unwinding."""
    text = strip_comments(source)
    in_test_file = in_test_file or bool(TEST_PATH.search(relative))
    spans = [] if in_test_file else test_spans(text)
    found = []
    for match in DROP_IMPL.finditer(text):
        line_start = text.rfind("\n", 0, match.start()) + 1
        preceding = text[max(0, line_start - 200):line_start].rstrip()
        in_test = (
            in_test_file
            or any(start <= match.start() <= end for start, end in spans)
            or bool(OWN_TEST_CFG.search(preceding))
        )
        if not in_test:
            continue
        body = text[match.end():block_end(text, text.find("{", match.end()))]
        if PANICS.search(body) and "panicking()" not in body:
            found.append(match.group(1))
    return found


class FixtureDropPanicTests(unittest.TestCase):
    def test_the_detector_finds_a_panicking_guard_and_accepts_the_reporting_forms(self) -> None:
        source = """
        struct Plain;
        impl Drop for Plain {
            fn drop(&mut self) { GATE.lock().unwrap().remove(&1); }
        }
        struct Reports;
        impl Drop for Reports {
            fn drop(&mut self) {
                // assert!(not code) and "panic!" in a string do not count
                if let Err(error) = restore() {
                    if std::thread::panicking() { eprintln!("{error}") } else { panic!("{error}") }
                }
            }
        }
        struct Recovers;
        impl Drop for Recovers {
            fn drop(&mut self) {
                GATE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&1);
            }
        }
        """
        self.assertEqual(panicking_drops("crates/x/tests/fixture.rs", source), ["Plain"])

    def test_a_test_module_inside_production_code_is_test_code(self) -> None:
        source = """
        struct Product;
        impl Drop for Product { fn drop(&mut self) { self.0.take().unwrap(); } }
        #[cfg(test)]
        mod tests {
            struct Guard;
            impl Drop for Guard { fn drop(&mut self) { self.0.take().unwrap(); } }
        }
        """
        self.assertEqual(panicking_drops("crates/x/src/lib.rs", source), ["Guard"])

    def test_a_module_declared_for_tests_is_test_code_with_its_directory(self) -> None:
        sources = {
            "crates/x/src/server.rs": '#[cfg(test)]\nmod checks;\n#[cfg(feature = "x-test-support")]\npub mod hooks;\nmod product;\n',
            "crates/x/src/contracts.rs": '#![cfg(feature = "x-test-support")]\n',
        }
        roots = test_module_roots(sources)
        self.assertTrue(under_test_root("crates/x/src/server/checks/style.rs", roots))
        self.assertTrue(under_test_root("crates/x/src/server/hooks.rs", roots))
        self.assertTrue(under_test_root("crates/x/src/contracts.rs", roots))
        self.assertFalse(under_test_root("crates/x/src/server/product.rs", roots))

    def test_no_fixture_drop_in_the_repository_panics_during_unwinding(self) -> None:
        sources = {}
        for path in sorted(SOURCE_ROOT.rglob("*.rs")):
            relative = path.relative_to(REPO_ROOT).as_posix()
            if "/target/" not in relative:
                sources[relative] = path.read_text(encoding="utf-8", errors="replace")
        roots = test_module_roots(sources)
        offenders = []
        for relative, source in sources.items():
            for name in panicking_drops(relative, source, under_test_root(relative, roots)):
                offenders.append(f"{relative}: {name}")
        self.assertEqual(
            offenders,
            [],
            "a fixture Drop must not panic while a test already panics: check "
            "std::thread::panicking() and report, or recover the poisoned lock",
        )


if __name__ == "__main__":
    unittest.main()
