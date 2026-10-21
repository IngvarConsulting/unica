#!/usr/bin/env python3
"""Размер `medium` для Rust: выражение nextest по структуре дерева.

Размер объявляется вне теста — выражением фильтра в `.config/nextest.toml`.
Первый проход по структуре: интеграционные цели (`kind(test)`) и модули, в
чьих файлах есть процесс или сокет. Признак — только конструкции стандартной
библиотеки и Cargo: `std::process`, `Command::new`, `std::net`, `TcpStream`,
`UnixStream`, `UnixListener`, `CARGO_BIN_EXE_`. Ни одного нашего имени: их
переименование молча лишало бы стража покрытия.

Отдельные объёмные проверки с фактическим замером объявлены точным именем
и владельцем библиотеки. Генератор проверяет их присутствие и единственность
в списке Cargo, поэтому переименование требует обновить объявление.

`--write` переписывает выражение в `.config/nextest.toml` между метками, беря
имена тестов из `cargo nextest list --message-format json` (нужен `cargo`);
страж размера в `tests/ci` читает то же выражение без `cargo` и проверяет, что
каждый файл с процессом или сокетом в нём назван, а внутри терма названы все
встроенные модули тестов этого файла. Что считать тестом, решает разбор в
страже — здесь второго ответа на этот вопрос нет.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

from tree_sitter import Language, Parser
import tree_sitter_rust

REPO_ROOT = Path(__file__).resolve().parents[2]
NEXTEST_TOML = REPO_ROOT / ".config" / "nextest.toml"
MEDIUM_CONSTRUCTS = re.compile(
    r"std::process|Command::new|std::net|TcpStream|TcpListener|UnixStream|UnixListener|CARGO_BIN_EXE_"
)
BLOCKS = {
    "pr": ("# >>> размер medium (ворота pr): пишет scripts/ci/size-filters.py --write", "# <<< размер medium (ворота pr)"),
    "deadline": ("# >>> размер medium (срок): пишет scripts/ci/size-filters.py --write", "# <<< размер medium (срок)"),
}
TERM = re.compile(r"test\(/\^([A-Za-z0-9_:]+)::")
# 400 000 исходных наборов: 34.447 с отдельно, 41.729 с совместно, macOS.
# Это размер конкретной проверки; соседние layout-проверки остаются small.
MEASURED_MEDIUM_CASES = (
    (
        "unica-project-health",
        "unica_project_health",
        "inspection::layout::tests::health_source_layout_keeps_all_defaulted_rows_and_their_real_ambiguity",
    ),
)


TEST_ATTRIBUTE = re.compile(r"#\[(?:[A-Za-z0-9_]+::)*test(?:\(.*\))?\]")


def inline_test_modules(source: bytes) -> tuple[set[str], bool]:
    """Встроенные модули файла с тестами внутри и признак тестов на его верхнем уровне.

    Генератор относит тест к первому сегменту после модуля файла, поэтому
    вложенный модуль засчитывается внешнему, а не называется отдельно.
    """
    tree = Parser(Language(tree_sitter_rust.language())).parse(source)

    def is_test_attribute(node) -> bool:
        if node.type != "attribute_item":
            return False
        compact = re.sub(r"\s+", "", source[node.start_byte : node.end_byte].decode())
        return TEST_ATTRIBUTE.fullmatch(compact) is not None

    def holds_test(node) -> bool:
        stack = [node]
        while stack:
            current = stack.pop()
            if is_test_attribute(current):
                return True
            stack.extend(current.children)
        return False

    modules: set[str] = set()
    top_level = False
    children = tree.root_node.named_children
    for index, child in enumerate(children):
        # Модуль без тела — отдельный файл; его тесты принадлежат не этому терму.
        if child.type == "mod_item" and child.child_by_field_name("body") is not None:
            if holds_test(child):
                modules.add(child.child_by_field_name("name").text.decode())
        elif child.type == "function_item":
            # Атрибут — сиблинг перед элементом, а не его ребёнок.
            back = index - 1
            while back >= 0 and children[back].type == "attribute_item":
                top_level = top_level or is_test_attribute(children[back])
                back -= 1
    return modules, top_level



def source_modules(root: Path) -> dict[str, dict[tuple[str, ...], tuple[bool, Path]]]:
    """Resolve Rust module declarations, including moved #[path] unit targets."""
    found = {}
    parser = Parser(Language(tree_sitter_rust.language()))
    for crate in sorted((root / "crates").iterdir()):
        src = crate / "src"
        if not src.is_dir():
            continue
        modules = {}

        def visit_file(path, logical):
            if not path.is_file():
                return
            source = path.read_bytes()
            modules[logical] = (bool(MEDIUM_CONSTRUCTS.search(source.decode())), path)
            base = path.parent if path.stem in {"lib", "main", "mod"} else path.with_suffix("")
            walk(parser.parse(source).root_node, source, logical, base, path.parent)

        def walk(node, source, logical, default_base, explicit_base):
            attributes = []
            for child in node.named_children:
                if child.type == "attribute_item":
                    attributes.append(source[child.start_byte:child.end_byte].decode())
                    continue
                if child.type == "mod_item":
                    name = child.child_by_field_name("name").text.decode()
                    body = child.child_by_field_name("body")
                    if body is not None:
                        walk(body, source, (*logical, name), default_base / name, default_base / name)
                    else:
                        override = next((m[1] for attr in attributes if (m := re.fullmatch(
                            r'\s*#\[\s*path\s*=\s*"([^"]+)"\s*\]\s*', attr))), None)
                        if override is not None:
                            target = explicit_base / override
                        else:
                            target = default_base / (name + ".rs")
                            if not target.is_file():
                                target = default_base / name / "mod.rs"
                        visit_file(target, (*logical, name))
                attributes = []

        visit_file(src / "lib.rs", ())
        visit_file(src / "main.rs", ("main",))
        found[crate.name] = modules
    return found


def flagged_modules(root: Path) -> list[tuple[str, str, Path]]:
    """(крейт, модуль, файл) файлов дерева с процессом или сокетом.

    Есть ли в файле тесты, здесь не решается: файл отдаётся вместе с путём, а
    ответ страж берёт из своего разбора. Регулярка на этом месте была второй
    копией правила «что считать тестом» и расходилась со стражем — она не видела
    `#[tokio::test]`, и такой файл выпадал из проверки целиком.
    """
    flagged = []
    for crate, modules in source_modules(root).items():
        for module, (marked, path) in modules.items():
            if marked:
                flagged.append((crate, "::".join(module), path))
    return flagged


def owner(modules: dict[tuple[str, ...], tuple[bool, Path]], name: str) -> tuple[str, ...] | None:
    """Файл, которому принадлежит тест: самый глубокий модуль-файл в его имени."""
    parts = tuple(name.split("::")[:-1])
    for n in range(len(parts), -1, -1):
        if parts[:n] in modules:
            return parts[:n]
    return None


def nextest_list(root: Path) -> dict:
    completed = subprocess.run(
        ["cargo", "nextest", "list", "--workspace", "--run-ignored", "all", "--message-format", "json"],
        cwd=root, capture_output=True, text=True, check=True,
    )
    return json.loads(completed.stdout)


def medium_terms(listed: dict, modules_by_crate: dict) -> list[str]:
    """Термы фильтра: по одному на помеченный файл, точно по его тестам.

    Файл владеет тестами своих встроенных модулей (`M::tests::…`) и тестами
    на своём верхнем уровне (`M::имя`); тесты вложенных файлов — не его.
    """
    inline: dict[tuple[str, tuple[str, ...]], set[str]] = {}
    direct: set[tuple[str, tuple[str, ...]]] = set()
    binaries = {}
    for suite in listed["rust-suites"].values():
        if suite.get("kind") != "lib":
            continue
        crate = suite["binary-id"].split("::")[0]
        modules = modules_by_crate.get(crate, {})
        binaries[crate] = suite["binary-name"]
        for name in suite["testcases"]:
            module = owner(modules, name)
            if module is None or not modules[module][0]:
                continue
            rest = name.split("::")[len(module):]
            if len(rest) == 1:
                direct.add((crate, module))
            else:
                inline.setdefault((crate, module), set()).add(rest[0])
    # Platform-gated unit modules are still declared in this source tree.
    # Include their real attributed modules without claiming their execution on
    # the current host. Cargo's inventory supplies macro-generated test names.
    for crate, modules in modules_by_crate.items():
        for module, (marked, path) in modules.items():
            if not marked:
                continue
            named, top_level = inline_test_modules(path.read_bytes())
            key = (crate, module)
            if named:
                inline.setdefault(key, set()).update(named)
            if top_level:
                direct.add(key)
    terms = []
    for key in sorted(set(inline) | direct):
        crate, module = key
        prefix = "::".join(module)
        stem = re.escape(prefix) + "::" if prefix else ""
        binary = binaries.get(crate, crate.replace("-", "_"))
        if key in inline:
            terms.append((crate, f"(binary({binary}) & test(/^{stem}({'|'.join(sorted(inline[key]))})::/))"))
        if key in direct:
            terms.append((crate, f"(binary({binary}) & test(/^{stem}[^:]+$/))"))
    for (binary_id, _, _), term in zip(MEASURED_MEDIUM_CASES, measured_medium_terms(listed), strict=True):
        terms.append((binary_id, term))
    return [term for _, term in sorted(set(terms))]


def measured_medium_terms(listed: dict) -> list[str]:
    terms = []
    for binary_id, binary_name, name in MEASURED_MEDIUM_CASES:
        matches = [suite for suite in listed["rust-suites"].values() if name in suite["testcases"]]
        if len(matches) != 1:
            raise ValueError(f"measured medium test must exist once: {binary_id}::{name}")
        suite = matches[0]
        if (suite.get("kind"), suite.get("binary-id"), suite.get("binary-name")) != (
            "lib", binary_id, binary_name
        ):
            raise ValueError(f"measured medium test belongs to another binary: {binary_id}::{name}")
        terms.append(f"(binary({binary_name}) & test(/^{re.escape(name)}$/))")
    return terms


def render(terms: list[str]) -> str:
    lines = ["kind(test)", *terms]
    return "\n    | ".join(lines)


def blocks(terms: list[str], large: str) -> dict[str, str]:
    body = render(terms)
    return {
        "pr": f"default-filter = \'\'\'not (\n{body}\n) & not (\n{large}\n)\'\'\'",
        "deadline": (
            "[[profile.default.overrides]]\n"
            f"filter = \'\'\'\n{body}\n\'\'\'\n"
            "slow-timeout = { period = \"300s\", terminate-after = 2 }"
        ),
    }


def write(root: Path, terms: list[str]) -> None:
    text = NEXTEST_TOML.read_text(encoding="utf-8")
    large = tomllib.loads(text)["profile"]["large"]["default-filter"].strip()
    for name, body in blocks(terms, large).items():
        begin, end = BLOCKS[name]
        start, stop = text.index(begin), text.index(end) + len(end)
        text = text[:start] + f"{begin}\n{body}\n{end}" + text[stop:]
    NEXTEST_TOML.write_text(text, encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--write", action="store_true", help="переписать выражение в .config/nextest.toml")
    args = parser.parse_args(argv)
    terms = medium_terms(nextest_list(REPO_ROOT), source_modules(REPO_ROOT))
    if args.write:
        write(REPO_ROOT, terms)
        print(f"термов medium: {len(terms)}, записано в {NEXTEST_TOML.relative_to(REPO_ROOT)}", file=sys.stderr)
    else:
        print(render(terms))
    return 0


if __name__ == "__main__":
    sys.exit(main())
