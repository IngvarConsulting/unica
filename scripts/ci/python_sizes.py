"""Размер тестов Python — из объявлений в самих тестовых модулях.

Модуль набора (`tests/ci`, `tests/dev`) объявляет на верхнем уровне то, что
отличает его от размера набора, литералом:

- `MEDIUM = ("Класс", "Класс.тест", ...)` — тесты `medium`: зовут `cargo`,
  берут продукт через `ensure_binary` или поднимают сокет, то есть собирают
  или запускают продукт. Побеждает самое длинное подходящее имя.
- `PR_SKIP = "причина"` — модуль не идёт на уровне PR
  (`scripts/ci/run-static-tests.py`): его малым тестам нужен `cargo` или
  продукт, или он дольше бюджета модуля. Его исполняет очередь.
- `QUEUE_KEEPS_MEDIUM = ("Класс", ...)` — `medium`, которые очередь
  исполняет при выборе тестов: смоук продукта и снимки контрактов.

Сборщик читает литералы через `ast` и модуль не импортирует. Страж
`tests/ci/test_python_sizes.py` держит имена существующими, а модули,
которые собирают или запускают продукт, — объявленными.
"""

from __future__ import annotations

import ast
import warnings
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SUITES = ("tests/ci", "tests/dev")
MEDIUM = "MEDIUM"
PR_SKIP = "PR_SKIP"
QUEUE_KEEPS_MEDIUM = "QUEUE_KEEPS_MEDIUM"
NAMES = (MEDIUM, PR_SKIP, QUEUE_KEEPS_MEDIUM)


@dataclass(frozen=True)
class Declarations:
    """Разметка набора: имена тестов — `модуль.Класс[.тест]`, модули — `tests.ci.модуль`."""

    medium: tuple[str, ...] = ()
    pr_skip: frozenset[str] = frozenset()
    keep_medium: tuple[str, ...] = ()


def module_declarations(path: Path) -> dict[str, object]:
    """Объявления размера верхнего уровня модуля, по имени."""
    found: dict[str, object] = {}
    with warnings.catch_warnings():
        # Чужие предупреждения разбора (escape в строке теста) — не дело сборщика.
        warnings.simplefilter("ignore", SyntaxWarning)
        tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
    for node in tree.body:
        if isinstance(node, (ast.Assign, ast.AnnAssign)):
            targets = node.targets if isinstance(node, ast.Assign) else [node.target]
            for target in targets:
                if isinstance(target, ast.Name) and target.id in NAMES:
                    if node.value is None:
                        raise ValueError(f"{path}: {target.id} без значения")
                    found[target.id] = ast.literal_eval(node.value)
    names = found.get(MEDIUM, ()), found.get(QUEUE_KEEPS_MEDIUM, ())
    if any(not isinstance(value, tuple) or not all(isinstance(item, str) and item for item in value) for value in names):
        raise ValueError(f"{path}: {MEDIUM} и {QUEUE_KEEPS_MEDIUM} — кортежи непустых строк")
    if PR_SKIP in found and not (isinstance(found[PR_SKIP], str) and found[PR_SKIP].strip()):
        raise ValueError(f"{path}: {PR_SKIP} — непустая строка с причиной")
    return found


def modules(directory: Path) -> list[Path]:
    """Тестовые модули каталога набора — те, что находит `unittest discover`."""
    return sorted(directory.glob("test*.py"))


def suite_medium(directory: Path) -> list[str]:
    """Тесты `medium` одного каталога набора: `модуль.Класс[.тест]`."""
    return [f"{path.stem}.{name}" for path in modules(directory)
            for name in module_declarations(path).get(MEDIUM, ())]


def collect(root: Path = ROOT, suites: tuple[str, ...] = SUITES) -> Declarations:
    """Разметка всех наборов репозитория."""
    medium: list[str] = []
    pr_skip: set[str] = set()
    keep: list[str] = []
    for suite in suites:
        for path in modules(root / suite):
            declared = module_declarations(path)
            medium += [f"{path.stem}.{name}" for name in declared.get(MEDIUM, ())]
            keep += [f"{path.stem}.{name}" for name in declared.get(QUEUE_KEEPS_MEDIUM, ())]
            if PR_SKIP in declared:
                pr_skip.add(f"{suite.replace('/', '.')}.{path.stem}")
    return Declarations(tuple(medium), frozenset(pr_skip), tuple(keep))
