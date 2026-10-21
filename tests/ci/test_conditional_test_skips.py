"""Тест без своего стенда или родителя виден пропущенным, а не пройденным.

Раньше тесты против справки платформы (`UNICA_PLATFORM_HELP_DIR`), живой
площадки вендора (`UNICA_KB1CI_LIVE`) и дочерние процессы-помощники при
отсутствии переменной печатали строку или молча выходили через `return`,
и очередь записывала их PASS, ничего не проверив (#1838). Теперь такой тест
помечен `#[ignore = "..."]`: в отчёте очереди он `skipped` с причиной, а
исполняется на стенде или из родителя через `--ignored`. Без переменной он
падает, а не проходит пустым.

Страж держит три правила:

- переменную стенда читает только её helper, и `.expect` завершает само
  чтение; её имя вне helper встречается лишь в комментарии или причине
  `ignore`, поэтому обход через константу или обёртку не проходит;
  каждый тест, вызывающий helper, помечен `ignore`;
- тест без `ignore` не выходит пустым, когда переменной нет: ни
  `let ... = env::var(..) else { return }`, ни `if env::var(..).is_err()
  { return }`, ни тело, целиком спрятанное под `if env::var(..).is_some()`;
- родитель, запускающий ignored-помощника по имени, передаёт `--ignored`.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOT = REPO_ROOT / "crates"

# Переменная стенда → единственный helper, который её читает.
STAND_SWITCHES = {
    "UNICA_PLATFORM_HELP_DIR": "platform_help_dir",
    "UNICA_KB1CI_LIVE": "live_kb_switch",
}

FUNCTION = re.compile(
    r"^(?P<indent>[ \t]*)(?:pub(?:\([^)]*\))?\s+)?(?:(?:async|const|unsafe)\s+)*fn\s+(?P<name>\w+)\s*[<(]",
    re.M,
)
TEST_ATTRIBUTE = re.compile(r"#\[(?:[\w:]+::)?test\b")
IGNORE_ATTRIBUTE = re.compile(r'#\[ignore\s*=\s*"[^"]+"\]')
ENV_READ = r"(?:std::)?env::var(?:_os)?\((?:[^()]|\([^()]*\))*\)"
# Тело, которое ничего не делает, кроме печати: пустой выход.
QUIET = r"(?:\s*(?:eprintln|println)!\([^;]*?\);)*\s*"
EMPTY = r"(?:\{" + QUIET + r"\}|\(\)|return\b)"
SILENT_EXITS = (
    re.compile(r"let\s+(?:Ok|Some)\s*\([^=]*\)\s*=\s*" + ENV_READ + r"[^;{]*else\s*\{" + QUIET + r"return\b[^;]*;\s*\}"),
    re.compile(r"if\s+" + ENV_READ + r"\s*\.is_(?:err|none)\(\)\s*\{" + QUIET + r"return\b[^;]*;\s*\}"),
    re.compile(r"if\s+!\s*" + ENV_READ + r"\s*\.is_(?:ok|some)\(\)\s*\{" + QUIET + r"return\b[^;]*;\s*\}"),
    re.compile(r"if\s+let\s+(?:Err\s*\(\s*_\s*\)|None)\s*=\s*" + ENV_READ + r"[^{]*\{" + QUIET + r"return\b[^;]*;\s*\}"),
)
FALLBACK_ARM = re.compile(r"(?:Err\s*\(\s*_\s*\)|None|_)\s*=>\s*" + EMPTY)
# Условие, при ложности которого тест ничего не делает, если под ним всё тело.
GUARDED_BODY = re.compile(
    r"\A\s*\{\s*if\s+(?:" + ENV_READ + r"\s*\.is_(?:some|ok)\(\)|let\s+(?:Ok|Some)\s*\([^=]*\)\s*=\s*" + ENV_READ + r"[^{]*)\s*(?=\{)"
)


def attributes_above(text: str, start: int) -> list[str]:
    """Атрибуты над функцией; комментарии, пустые строки и многострочные
    атрибуты пропускаются, разбор останавливается на первом коде."""
    lines = text[:start].splitlines()
    attributes: list[str] = []
    pending: list[str] = []
    while lines:
        line = lines.pop().strip()
        if pending or (line.endswith("]") and not line.startswith("#[")):
            pending.insert(0, line)
            if line.startswith("#["):
                attributes.append(" ".join(pending))
                pending = []
            continue
        if line.startswith("#["):
            attributes.append(line)
        elif line and not line.startswith("//"):
            break
    return attributes


def functions(text: str) -> list[dict]:
    """Функции файла с атрибутами и телом до закрывающей скобки того же отступа."""
    found = []
    for match in FUNCTION.finditer(text):
        indent = match.group("indent")
        close = re.compile(r"^" + re.escape(indent) + r"\}[ \t]*$", re.M).search(text, match.end())
        end = close.end() if close else len(text)
        brace = text.find("{", match.end(), end)
        found.append({"name": match.group("name"), "start": match.start(), "end": end,
                      "attributes": attributes_above(text, match.start()),
                      "body": text[brace if brace >= 0 else match.end():end]})
    return found


def block(text: str, start: int) -> str:
    """Текст от первой `{` после `start` до парной ей `}`."""
    opening = text.find("{", start)
    depth = 0
    for index in range(opening, len(text)):
        depth += {"{": 1, "}": -1}.get(text[index], 0)
        if depth == 0:
            return text[opening:index + 1]
    return text[opening:]


def is_test(item: dict) -> bool:
    return any(TEST_ATTRIBUTE.match(attribute) for attribute in item["attributes"])


def is_ignored(item: dict) -> bool:
    return any(IGNORE_ATTRIBUTE.fullmatch(attribute) for attribute in item["attributes"])


def without_comments(text: str) -> str:
    """Комментарии заменяются пробелами той же длины: позиции сохраняются."""
    return re.sub(r"//[^\n]*", lambda match: " " * len(match.group()), text)


def stand_violations(text: str, label: str) -> list[str]:
    problems = []
    code = without_comments(text)
    found = functions(code)
    for variable, helper in STAND_SWITCHES.items():
        failing_read = re.compile(
            r"env::var(?:_os)?\(\s*\"" + variable + r"\"\s*\)"
            r"\s*(?:\.map\s*\(\s*PathBuf::from\s*\)\s*)?\.expect\s*\("
        )
        for mention in re.finditer(re.escape(variable), code):
            line_start = code.rfind("\n", 0, mention.start()) + 1
            if IGNORE_ATTRIBUTE.match(code[line_start:].lstrip()):
                continue
            owners = [item for item in found if item["start"] <= mention.start() < item["end"]]
            owner = min(owners, key=lambda item: item["end"] - item["start"]) if owners else None
            if owner is None or owner["name"] != helper:
                problems.append(f"{label}: {variable} is named outside {helper}()")
            elif not failing_read.search(owner["body"]):
                problems.append(f"{label}: {helper}() must fail without {variable}, not skip")
        for item in found:
            if is_test(item) and re.search(rf"\b{helper}\(\)", item["body"]) and not is_ignored(item):
                problems.append(f"{label}: test {item['name']} needs {variable} but is not #[ignore = \"...\"]")
    return problems


def guarded_whole_body(body: str) -> bool:
    """Всё тело под `if <есть переменная> { … }` без `else` и без кода после."""
    head = GUARDED_BODY.match(body)
    if not head:
        return False
    guarded = block(body, head.end())
    rest = body[head.end() + len(guarded):]
    return re.fullmatch(r"\s*\}\s*", rest) is not None


def silent_exit_violations(text: str, label: str) -> list[str]:
    problems = []
    for item in functions(without_comments(text)):
        if not is_test(item) or is_ignored(item):
            continue
        body = item["body"]
        silent = any(pattern.search(body) for pattern in SILENT_EXITS) or guarded_whole_body(body) or any(
            FALLBACK_ARM.search(block(body, match.end())) for match in re.finditer(r"match\s+" + ENV_READ, body)
        )
        if silent:
            problems.append(f"{label}: test {item['name']} passes empty without its environment; mark it #[ignore = \"...\"]")
    return problems


def ignored_test_names(sources: dict[str, str]) -> set[str]:
    return {item["name"] for text in sources.values() for item in functions(text)
            if is_test(item) and is_ignored(item)}


def launch_violations(sources: dict[str, str]) -> list[str]:
    """Каждый запуск ignored-теста по имени передаёт libtest `--ignored`.

    Без флага дочерний процесс отвечает `0 tests` с кодом 0, и родитель,
    который только наблюдает процесс, проходит, не запустив помощника.
    Запуск — массив аргументов с `"--exact"`; флаг ищется в том же массиве.
    Имя может прийти через константу, в том числе импортированную из
    другого файла.
    """
    problems = []
    names = ignored_test_names(sources)
    codes = {label: without_comments(text) for label, text in sources.items()}
    for label, code in codes.items():
        for literal in re.finditer(r'"(?:[\w:]*::)?(\w+)"', code):
            if literal.group(1) not in names:
                continue
            constant = re.search(r"const\s+(\w+)\s*:\s*&(?:'static\s+)?str\s*=\s*$",
                                 code[max(0, literal.start() - 160):literal.start()])
            if constant:
                name = constant.group(1)
                declared = code.rfind(name, 0, literal.start())
                sites = [(label, use.start()) for use in re.finditer(rf"\b{name}\b", code) if use.start() != declared]
                sites += [(other, use.start()) for other, text in codes.items()
                          if other != label and re.search(rf"\buse\s[^;]*\b{name}\b", text)
                          for use in re.finditer(rf"\b{name}\b", text)]
            else:
                sites = [(label, literal.start())]
            for where, site in sites:
                text = codes[where]
                arguments = text[text.rfind("[", 0, site):text.find("]", site) + 1]
                if '"--exact"' in arguments and '"--ignored"' not in arguments:
                    problems.append(f"{where}: launch of ignored test {literal.group(1)} lacks --ignored")
    return problems


def violations(text: str, label: str) -> list[str]:
    return stand_violations(text, label) + silent_exit_violations(text, label)


def rust_sources() -> list[Path]:
    return [path for path in sorted(SOURCE_ROOT.rglob("*.rs"))
            if "target" not in path.relative_to(SOURCE_ROOT).parts]


def repository_violations() -> list[str]:
    problems = []
    for path in rust_sources():
        text = path.read_text(encoding="utf-8")
        if "env::var" in text or any(variable in text for variable in STAND_SWITCHES):
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


class StandSwitchTests(unittest.TestCase):
    def test_an_ignored_caller_and_failing_helper_pass(self) -> None:
        self.assertEqual(violations(IGNORED, "fixture"), [])

    def test_a_caller_without_ignore_is_rejected(self) -> None:
        text = IGNORED.replace('#[ignore = "vendor help: needs UNICA_PLATFORM_HELP_DIR"]\n', "")
        self.assertEqual(len(violations(text, "fixture")), 1)

    def test_a_caller_behind_a_comment_and_multiline_attribute_is_still_seen(self) -> None:
        text = IGNORED.replace(
            '#[ignore = "vendor help: needs UNICA_PLATFORM_HELP_DIR"]\n',
            "// a comment between the attributes\n#[cfg_attr(\n    unix,\n    allow(dead_code)\n)]\n",
        )
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
        self.assertEqual(len(stand_violations(text, "fixture")), 1)
        self.assertEqual(len(silent_exit_violations(text, "fixture")), 1)

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

    def test_a_constant_or_wrapper_cannot_carry_the_name_around_the_helper(self) -> None:
        constant = IGNORED + '''
const VARIABLE: &str = "UNICA_PLATFORM_HELP_DIR";

#[test]
fn bypass() {
    let Ok(root) = std::env::var(VARIABLE) else {
        return;
    };
}
'''
        problems = violations(constant, "fixture")
        self.assertIn("fixture: UNICA_PLATFORM_HELP_DIR is named outside platform_help_dir()", problems)
        self.assertTrue(any("bypass passes empty" in problem for problem in problems), problems)
        wrapper = IGNORED + '''
fn installed() -> Option<String> {
    option_env!("UNICA_KB1CI_LIVE").map(str::to_string)
}
'''
        self.assertEqual(violations(wrapper, "fixture"),
                         ["fixture: UNICA_KB1CI_LIVE is named outside live_kb_switch()"])

    def test_comments_and_ignore_reasons_may_name_the_variable(self) -> None:
        text = "// UNICA_KB1CI_LIVE enables the live check\n" + IGNORED
        self.assertEqual(violations(text, "fixture"), [])


class SilentExitTests(unittest.TestCase):
    def assertSilent(self, body: str) -> None:
        text = "#[test]\nfn child() {\n" + body + "\n}\n"
        self.assertEqual(len(silent_exit_violations(text, "fixture")), 1, body)
        ignored = "#[test]\n#[ignore = \"helper process\"]\nfn child() {\n" + body + "\n}\n"
        self.assertEqual(silent_exit_violations(ignored, "fixture"), [], body)

    def test_let_else_return_is_silent(self) -> None:
        self.assertSilent('    let Some(state) = std::env::var_os(STATE) else {\n        return;\n    };\n    run(state);')

    def test_is_none_return_is_silent(self) -> None:
        self.assertSilent('    if std::env::var_os(MARKER).is_none() {\n        eprintln!("skipped");\n        return;\n    }\n    run();')

    def test_a_body_entirely_under_is_some_is_silent(self) -> None:
        self.assertSilent('    if std::env::var_os("CHILD").is_some() {\n        run();\n    }')

    def test_a_match_with_an_empty_fallback_is_silent(self) -> None:
        self.assertSilent('    match std::env::var(MODE).as_deref() {\n        Ok("a") => run(),\n        Err(_) => {}\n    }')

    def test_a_match_with_a_wildcard_fallback_is_silent(self) -> None:
        self.assertSilent('    match std::env::var(MODE).as_deref() {\n        Ok("a") => run(),\n        _ => {}\n    }')

    def test_a_later_unrelated_match_does_not_count(self) -> None:
        text = (
            "#[test]\nfn modes() {\n    match std::env::var(MODE).as_deref() {\n"
            '        Ok(mode) => run(mode),\n        other => panic!("{other:?}"),\n    }\n'
            "    match x {\n        _ => {}\n    }\n}\n"
        )
        self.assertEqual(silent_exit_violations(text, "fixture"), [])

    def test_equivalent_absence_forms_are_silent(self) -> None:
        self.assertSilent('    if let Ok(value) = std::env::var(CHILD) {\n        run(value);\n    }')
        self.assertSilent('    if let Err(_) = std::env::var(CHILD) {\n        return;\n    }\n    run();')
        self.assertSilent('    if !std::env::var(CHILD).is_ok() {\n        return;\n    }\n    run();')
        self.assertSilent('    match std::env::var(CHILD) {\n        Ok(v) => run(v),\n        Err(_) => (),\n    }')
        self.assertSilent('    let Ok(v) = std::env::var(format!("{}_X", PREFIX)) else {\n        return;\n    };\n    run(v);')

    def test_an_if_else_self_reexec_is_not_silent(self) -> None:
        text = "#[test]\nfn t() {\n    if std::env::var_os(C).is_some() {\n        a();\n    } else {\n        b();\n    }\n}\n"
        self.assertEqual(silent_exit_violations(text, "fixture"), [])

    def test_a_parent_branch_that_does_work_is_not_silent(self) -> None:
        text = '''
#[test]
fn self_reexec() {
    if std::env::var_os(CHILD).is_none() {
        let status = spawn_child();
        assert!(status.success());
        return;
    }
    run_child();
}

#[test]
fn child_or_parent() {
    if let Ok(value) = std::env::var(CHILD_CONFIG) {
        run_child(value);
        return;
    }
    run_parent();
}
'''
        self.assertEqual(silent_exit_violations(text, "fixture"), [])


class LaunchTests(unittest.TestCase):
    HELPER = '#[test]\n#[ignore = "helper process"]\nfn child_helper() {\n    run();\n}\n'

    def test_a_literal_launch_needs_the_ignored_flag(self) -> None:
        launch = 'fn parent() {\n    command.args([\n        "--exact",\n        "tests::child_helper",\n        "--nocapture",\n    ]);\n}\n'
        self.assertEqual(len(launch_violations({"a.rs": self.HELPER + launch})), 1)
        fixed = launch.replace('"tests::child_helper",\n', '"tests::child_helper",\n        "--ignored",\n')
        self.assertEqual(launch_violations({"a.rs": self.HELPER + fixed}), [])

    def test_a_neighbouring_launch_does_not_lend_its_flag(self) -> None:
        launch = ('fn parent() {\n    a.args(["--exact", "other", "--ignored"]);\n'
                  '    c.args(["--exact", "tests::child_helper"]);\n}\n')
        self.assertEqual(len(launch_violations({"a.rs": self.HELPER + launch})), 1)

    def test_a_static_constant_imported_elsewhere_needs_the_flag(self) -> None:
        declaration = "pub const CHILD: &'static str = \"tests::child_helper\";\n"
        launch = 'use crate::a::CHILD;\nfn parent() {\n' + " " * 300 + '\n    command.args(["--exact", CHILD]);\n}\n'
        self.assertEqual(launch_violations({"a.rs": self.HELPER + declaration, "b.rs": launch}),
                         ["b.rs: launch of ignored test child_helper lacks --ignored"])

    def test_a_launch_through_a_constant_needs_the_ignored_flag(self) -> None:
        launch = 'const CHILD: &str =\n    "tests::child_helper";\nfn parent() {\n    command.args(["--exact", CHILD, "--nocapture"]);\n}\n'
        self.assertEqual(len(launch_violations({"a.rs": self.HELPER + launch})), 1)
        fixed = launch.replace('CHILD, "--nocapture"', 'CHILD, "--ignored", "--nocapture"')
        self.assertEqual(launch_violations({"a.rs": self.HELPER + fixed}), [])


class RepositoryTests(unittest.TestCase):
    def test_repository_conditional_tests_are_visible_skips(self) -> None:
        self.assertEqual(repository_violations(), [])

    def test_repository_launches_ignored_helpers_with_the_flag(self) -> None:
        sources = {str(path.relative_to(REPO_ROOT)): path.read_text(encoding="utf-8") for path in rust_sources()}
        self.assertEqual(launch_violations(sources), [])

    def test_the_repository_still_has_stand_and_helper_tests(self) -> None:
        # Сканер, переставший находить тесты, проходил бы вечно.
        callers = {helper: 0 for helper in STAND_SWITCHES.values()}
        ignored_helpers = 0
        for path in rust_sources():
            for item in functions(path.read_text(encoding="utf-8")):
                if not is_test(item):
                    continue
                for helper in callers:
                    if re.search(rf"\b{helper}\(\)", item["body"]):
                        callers[helper] += 1
                if is_ignored(item) and re.search(r"env::var(?:_os)?\(", item["body"]):
                    ignored_helpers += 1
        self.assertGreaterEqual(callers["platform_help_dir"], 8)
        self.assertGreaterEqual(callers["live_kb_switch"], 1)
        self.assertGreaterEqual(ignored_helpers, 10)


if __name__ == "__main__":
    unittest.main()
