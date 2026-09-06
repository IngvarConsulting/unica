#!/usr/bin/env python3
"""Размер `medium` для Rust: выражение nextest по структуре дерева.

Размер объявляется вне теста — выражением фильтра в `.config/nextest.toml`.
Первый проход по структуре: интеграционные цели (`kind(test)`) и модули, в
чьих файлах есть процесс или сокет. Признак — только конструкции стандартной
библиотеки и Cargo: `std::process`, `Command::new`, `std::net`, `TcpStream`,
`UnixStream`, `UnixListener`, `CARGO_BIN_EXE_`. Ни одного нашего имени: их
переименование молча лишало бы стража покрытия.

`--write` переписывает выражение в `.config/nextest.toml` между метками, беря
имена тестов из `cargo nextest list --message-format json` (нужен `cargo`);
страж размера в `tests/ci` читает то же выражение без `cargo` и проверяет, что
каждый файл с процессом или сокетом в нём назван.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
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


def module_of(path: Path, src: Path) -> tuple[str, ...]:
    parts = list(path.relative_to(src).with_suffix("").parts)
    if parts[-1] in ("mod", "lib", "main"):
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


def flagged_modules(root: Path) -> list[tuple[str, str]]:
    """(крейт, модуль) файлов дерева с процессом или сокетом, в которых есть тесты."""
    flagged = []
    for crate, modules in source_modules(root).items():
        for module, (marked, path) in modules.items():
            text = path.read_text(encoding="utf-8", errors="replace")
            # Тест — атрибут в начале строки; упоминание в строке или комментарии не в счёт.
            if marked and re.search(r"^\s*#\[test\]", text, re.M) and declared(module, modules):
                flagged.append((crate, "::".join(module)))
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
            terms.append(f"test(/^{re.escape(prefix)}::({'|'.join(sorted(inline[key]))})::/)")
        if key in direct:
            terms.append(f"test(/^{re.escape(prefix)}::[^:]+$/)")
    return terms


def render(terms: list[str]) -> str:
    lines = ["kind(test)", *terms]
    return "\n    | ".join(lines)


def blocks(terms: list[str]) -> dict[str, str]:
    body = render(terms)
    return {
        "pr": f"default-filter = \'\'\'not (\n{body}\n)\'\'\'",
        "deadline": (
            "[[profile.default.overrides]]\n"
            f"filter = \'\'\'\n{body}\n\'\'\'\n"
            "slow-timeout = { period = \"300s\", terminate-after = 2 }"
        ),
    }


def write(root: Path, terms: list[str]) -> None:
    text = NEXTEST_TOML.read_text(encoding="utf-8")
    for name, body in blocks(terms).items():
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
