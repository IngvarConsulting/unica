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
        "unica-coder",
        "unica_coder",
        "infrastructure::project_health::layout::tests::health_source_layout_keeps_all_defaulted_rows_and_their_real_ambiguity",
    ),
)


def module_of(path: Path, src: Path) -> tuple[str, ...]:
    """Модуль файла в дереве библиотеки; пустой кортеж — корень, и он за `lib.rs`.

    `main.rs` — корень другой цели, двоичной, и делить пустой кортеж с `lib.rs`
    ему нельзя: словарь модулей один на крейт, а `main.rs` идёт по сортировке
    позже и затирал бы `lib.rs` со всеми объявлениями `mod ...;` в нём. Со своим
    ключом файл остаётся виден `declared`, но термов не получает: `mod main;`
    никто не объявляет, а размер считается только по целям `kind == "lib"`.
    """
    parts = list(path.relative_to(src).with_suffix("").parts)
    if parts[-1] in ("mod", "lib"):
        parts = parts[:-1]
    return tuple(parts)


def source_modules(root: Path) -> dict[str, dict[tuple[str, ...], tuple[bool, Path]]]:
    """Крейт → модуль файла → (есть ли процесс или сокет, путь). Только `src/`."""
    found: dict[str, dict[tuple[str, ...], tuple[bool, Path]]] = {}
    for crate in sorted((root / "crates").iterdir()):
        src = crate / "src"
        if not src.is_dir():
            continue
        found[crate.name] = {}
        for path in sorted(src.rglob("*.rs")):
            text = path.read_text(encoding="utf-8", errors="replace")
            found[crate.name][module_of(path, src)] = (bool(MEDIUM_CONSTRUCTS.search(text)), path)
    return found


def declared(module: tuple[str, ...], sources: dict[tuple[str, ...], tuple[bool, Path]]) -> bool:
    """Входит ли файл в дерево модулей: кто-то объявил его `mod имя;`.

    Файл-сирота не компилируется, и его тесты не идут ни в одних воротах —
    объявлять ему размер нечего.
    """
    if not module:
        return True
    pattern = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+" + re.escape(module[-1]) + r"\s*;", re.M)
    return any(pattern.search(path.read_text(encoding="utf-8", errors="replace")) for _, path in sources.values())


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
            if marked and declared(module, modules):
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
    for suite in listed["rust-suites"].values():
        if suite.get("kind") != "lib":
            continue
        crate = suite["binary-id"].split("::")[0]
        modules = modules_by_crate.get(crate, {})
        for name in suite["testcases"]:
            module = owner(modules, name)
            if module is None or not modules[module][0]:
                continue
            rest = name.split("::")[len(module):]
            if len(rest) == 1:
                direct.add((crate, module))
            else:
                inline.setdefault((crate, module), set()).add(rest[0])
    terms = []
    for key in sorted(set(inline) | direct):
        crate, module = key
        prefix = "::".join(module)
        if key in inline:
            terms.append((crate, f"test(/^{re.escape(prefix)}::({'|'.join(sorted(inline[key]))})::/)"))
        if key in direct:
            terms.append((crate, f"test(/^{re.escape(prefix)}::[^:]+$/)"))
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
        terms.append(f"test(/^{re.escape(name)}$/)")
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
