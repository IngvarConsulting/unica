"""Проверки против справки вендора не проходят пустыми.

Справка платформы (`UNICA_PLATFORM_HELP_DIR`) на раннерах CI не установлена.
Раньше тесты оракула каталога событий и реальной установки при отсутствии
переменной печатали строку и выходили через `return`, и очередь записывала
их PASS, ничего не проверив (#1838). Теперь переменную читает только
`platform_help_dir()`, который без неё падает, а каждый вызывающий его тест
помечен `#[ignore = "..."]`: в отчёте очереди он `skipped` с причиной, а на
стенде со справкой исполняется через `--run-ignored only`.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "crates"
HELPER = "platform_help_dir"

ENV_READ = re.compile(r'env::var(?:_os)?\(\s*"UNICA_PLATFORM_HELP_DIR"\s*\)')
# Падение обязано относиться к самому чтению: `.expect` на другом выражении
# рядом с умолчанием для переменной снова дал бы тихий проход.
FAILING_READ = re.compile(ENV_READ.pattern + r"\s*(?:\.map\s*\(\s*PathBuf::from\s*\)\s*)?\.expect\s*\(")
FUNCTION = re.compile(r"^(?P<indent>[ \t]*)(?:pub(?:\([^)]*\))?\s+)?fn\s+(?P<name>\w+)\s*\(", re.M)
ATTRIBUTE_LINE = re.compile(r"^[ \t]*#\[.*\][ \t]*$")


def functions(text: str) -> list[dict]:
    """Функции файла с атрибутами и телом до закрывающей скобки того же отступа."""
    found = []
    for match in FUNCTION.finditer(text):
        indent = match.group("indent")
        close = re.compile(r"^" + re.escape(indent) + r"\}[ \t]*$", re.M).search(text, match.end())
        end = close.end() if close else len(text)
        lines = text[: match.start()].splitlines()
        attributes = []
        while lines and (ATTRIBUTE_LINE.match(lines[-1]) or lines[-1].strip().startswith("///")):
            line = lines.pop().strip()
            if line.startswith("#["):
                attributes.append(line)
        found.append({"name": match.group("name"), "start": match.start(), "end": end,
                      "attributes": attributes, "body": text[match.end():end]})
    return found


def violations(text: str, label: str) -> list[str]:
    problems = []
    found = functions(text)
    for read in ENV_READ.finditer(text):
        owners = [item for item in found if item["start"] <= read.start() < item["end"]]
        owner = min(owners, key=lambda item: item["end"] - item["start"]) if owners else None
        if owner is None or owner["name"] != HELPER:
            problems.append(f"{label}: UNICA_PLATFORM_HELP_DIR is read outside {HELPER}()")
        elif not FAILING_READ.match(text, read.start()):
            problems.append(f"{label}: {HELPER}() must fail without the variable, not skip")
    for item in found:
        if "#[test]" not in item["attributes"] or not re.search(rf"\b{HELPER}\(\)", item["body"]):
            continue
        if not any(re.fullmatch(r'#\[ignore\s*=\s*"[^"]+"\]', attribute) for attribute in item["attributes"]):
            problems.append(f"{label}: test {item['name']} needs vendor help but is not #[ignore = \"...\"]")
    return problems


def repository_violations() -> list[str]:
    problems = []
    for path in sorted(SOURCE_ROOT.rglob("*.rs")):
        if "target" in path.relative_to(SOURCE_ROOT).parts:
            continue
        text = path.read_text(encoding="utf-8")
        if "UNICA_PLATFORM_HELP_DIR" in text or f"{HELPER}(" in text:
            problems.extend(violations(text, str(path.relative_to(REPO_ROOT))))
    return problems


IGNORED = '''
fn platform_help_dir() -> PathBuf {
    std::env::var_os("UNICA_PLATFORM_HELP_DIR").map(PathBuf::from).expect("set it")
}

#[test]
#[ignore = "vendor help: needs UNICA_PLATFORM_HELP_DIR"]
fn oracle() {
    let root = platform_help_dir();
}
'''


class VendorHelpSkipTests(unittest.TestCase):
    def test_repository_vendor_help_tests_are_visible_skips(self) -> None:
        self.assertEqual(repository_violations(), [])

    def test_the_repository_still_has_vendor_help_tests(self) -> None:
        # Сканер, переставший находить тесты, проходил бы вечно.
        callers = sum(
            1
            for path in SOURCE_ROOT.rglob("*.rs")
            if "target" not in path.relative_to(SOURCE_ROOT).parts
            for item in functions(path.read_text(encoding="utf-8"))
            if "#[test]" in item["attributes"] and re.search(rf"\b{HELPER}\(\)", item["body"])
        )
        self.assertGreaterEqual(callers, 8)

    def test_an_ignored_caller_and_failing_helper_pass(self) -> None:
        self.assertEqual(violations(IGNORED, "fixture"), [])

    def test_a_caller_without_ignore_is_rejected(self) -> None:
        text = IGNORED.replace('#[ignore = "vendor help: needs UNICA_PLATFORM_HELP_DIR"]\n', "")
        self.assertEqual(len(violations(text, "fixture")), 1)

    def test_a_silent_early_return_is_rejected(self) -> None:
        text = '''
#[test]
fn oracle() {
    let Ok(root) = std::env::var("UNICA_PLATFORM_HELP_DIR") else {
        return;
    };
}
'''
        self.assertEqual(len(violations(text, "fixture")), 1)

    def test_a_helper_that_does_not_fail_is_rejected(self) -> None:
        text = IGNORED.replace('.expect("set it")', "")
        self.assertEqual(len(violations(text, "fixture")), 1)

    def test_an_unrelated_expect_does_not_excuse_a_defaulted_read(self) -> None:
        text = IGNORED.replace(
            '    std::env::var_os("UNICA_PLATFORM_HELP_DIR").map(PathBuf::from).expect("set it")\n',
            '    let _ = "1".parse::<u8>().expect("unrelated");\n'
            '    std::env::var_os("UNICA_PLATFORM_HELP_DIR").map(PathBuf::from).unwrap_or_default()\n',
        )
        self.assertNotEqual(text, IGNORED)
        self.assertEqual(len(violations(text, "fixture")), 1)


if __name__ == "__main__":
    unittest.main()
