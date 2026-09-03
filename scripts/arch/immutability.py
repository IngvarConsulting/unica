#!/usr/bin/env python3
"""Что случилось с продуктовыми записями базы: замена, объяснение или подмена.

Продуктовая запись — обещание, на которое кто-то снаружи уже опёрся, и менять
его молча нельзя. Но виды обещаний живут по-разному, поэтому и дисциплины две.

Решение говорит, что было выбрано на его дату. Его не правят: заводят преемника,
а старому ставят `superseded` и имя заменившего. Переход `planned` в `active` и
отметка о реализации сюда не относятся: они говорят не о решении, а о мире
вокруг него, появляются позже самого решения и записываются атомарно.

Инвариант и контракт описывают действующее правило, а правило со временем
уточняется. Их править можно — но только вместе с решением, которое этой же
правкой заводится и объясняет, почему правило меняется. Инвариант, изменившийся
без нового основания, и есть тихая смена обещания.

Процессная запись под это не подпадает: её и заводят, чтобы перестроить в тот
день, когда разрабатывать стало неудобно.

Сторона берётся из базы, а не из рабочего дерева: иначе правку продуктового
правила достаточно было бы прикрыть переводом его в процессные.

Usage:
    immutability.py --base origin/main
    immutability.py            # база по умолчанию origin/main
"""

from __future__ import annotations

import argparse
import ast
import importlib.util
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

try:
    from tree_sitter import Language, Parser
    import tree_sitter_rust

    _RUST_PARSER = Parser(Language(tree_sitter_rust.language()))
    _RUST_PARSER_ERROR: str | None = None
except Exception as error:  # The guard must reject Rust evidence without its parser.
    _RUST_PARSER = None
    _RUST_PARSER_ERROR = f"tree-sitter Rust parser unavailable: {error}"

_SPEC = importlib.util.spec_from_file_location(
    "arch_registry_for_immutability", Path(__file__).resolve().parent / "registry.py"
)
_REGISTRY = importlib.util.module_from_spec(_SPEC)
# До exec_module: `dataclass` внутри registry.py ищет свой модуль в sys.modules,
# и без записи там разбор падает на определении класса.
sys.modules[_SPEC.name] = _REGISTRY
_SPEC.loader.exec_module(_REGISTRY)

ARCH_PREFIX = "arch/"
# Допустимые отметки трогают ровно две атомарные пары полей.
SUPERSESSION_FIELDS = ("status", "superseded-by")
# А это — всё, что можно проставить принятой записи, не переписав её.
#
# `realized` держит адрес свидетельства, что решение построено. Свидетельство
# появляется после решения — иначе решение принималось бы задним числом, — и
# запретить его записывать значит запретить реестру знать, что построено.
# Рассуждением оно не является, поэтому под запрет на правку не подпадает; что
# названный адрес существует, сторожит отдельное правило.
REALIZATION_FIELDS = ("status", "realized")
RECORDABLE_FIELDS = frozenset(SUPERSESSION_FIELDS + REALIZATION_FIELDS)


@dataclass(frozen=True)
class Verdict:
    offenders: tuple[str, ...]
    compared: int

    def report(self) -> str:
        head = f"сверено продуктовых записей: {self.compared}"
        if not self.offenders:
            return head + "\nправок принятых записей нет"
        return head + "\n" + "\n".join(f"  {line}" for line in self.offenders)


@dataclass(frozen=True)
class IntroducedRecord:
    kind: str
    path: str
    props: dict


def _git(repo: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", *args], cwd=repo, capture_output=True, text=True, check=True
    )
    return done.stdout


def _records_at(repo: Path, ref: str) -> dict[str, str]:
    """Пути и содержимое записей реестра в указанной ревизии."""
    listing = _git(repo, "ls-tree", "-r", "--name-only", ref, "--", ARCH_PREFIX).split()
    found = {}
    for path in listing:
        if not path.endswith(".md") or path.endswith("index.md") or path.endswith("README.md"):
            continue
        found[path] = _git(repo, "show", f"{ref}:{path}")
    return found


def _split(text: str) -> tuple[dict, str]:
    try:
        return _REGISTRY.parse_front_matter(text)
    except ValueError:
        return {}, text


# Свидетельство можно перенаправить, но только вслед за исчезнувшим адресом.
# Одно имя, за которым стояла обёртка, вызывавшая настоящие проверки, законно
# раскрывается в список этих проверок: обещание записи от этого не слабеет, а
# перестаёт держаться на функции, которая лишь повторяла уже сделанную работу.
# Опортунистическая подмена рабочего адреса под это не подпадает — старый адрес
# обязан исчезнуть из дерева.
EVIDENCE_FIELDS = ("check", "realized")
_CALL = re.compile(r"^\s*(?:crate::)?(?:[a-z_0-9]+::)*([a-z_0-9]+)\(\s*\)\s*;", re.M)


def _declaration_in(text: str, name: str) -> bool:
    return re.search(rf"\bfn\s+{re.escape(name)}\s*[(<]", text) is not None


def _body_at(text: str, name: str) -> str | None:
    found = re.search(rf"\bfn\s+{re.escape(name)}\s*\(", text)
    if not found:
        return None
    start = text.index("{", found.end())
    depth = 0
    for index in range(start, len(text)):
        if text[index] == "{":
            depth += 1
        elif text[index] == "}":
            depth -= 1
            if depth == 0:
                return text[start + 1 : index]
    return None


def _covered_names(repo: Path, base_ref: str, reference: str, pool: set[str]) -> set[str]:
    """Проверки, которые старый адрес приводил в исполнение.

    Разворачивается только то, что этой же правкой исчезло из дерева: раз
    обёртки больше нет, названные ею проверки поднимаются в запись. Всё, что
    в дереве осталось, — лист, и его запись обязана назвать сама.
    """
    path_text, _, name = reference.partition("::")
    sources: dict[str, str] = {}
    for candidate in {path_text, *pool}:
        try:
            sources[candidate] = _git(repo, "show", f"{base_ref}:{candidate}")
        except subprocess.CalledProcessError:
            continue
    if path_text not in sources:
        return set()

    def survives(declaration: str) -> bool:
        for candidate in {path_text, *pool}:
            current = repo / candidate
            if current.is_file() and _declaration_in(
                current.read_text(encoding="utf-8"), declaration
            ):
                return True
        return False

    def host_of(declaration: str) -> str | None:
        return next(
            (file for file, text in sources.items() if _body_at(text, declaration) is not None),
            None,
        )

    def qualify(host: str | None, declaration: str) -> str:
        # Адрес сравнивается целиком. Одноимённая функция в другом файле — это
        # другая проверка, и принимать её за покрытую значит принять подмену.
        # Файл, которого нет среди путей нового списка, назвать нечем: такая
        # запись остаётся неквалифицированной и требует отдельного разбора.
        return f"{host}::{declaration}" if host else f"::{declaration}"

    seen: set[tuple[str, str]] = set()
    leaves: set[str] = set()
    pending = [(path_text, name)]
    while pending:
        where, what = pending.pop()
        if (where, what) in seen:
            continue
        seen.add((where, what))
        body = _body_at(sources.get(where, ""), what)
        calls = _CALL.findall(body) if body is not None else []
        if not calls:
            leaves.add(qualify(where, what))
            continue
        for called in calls:
            host = host_of(called)
            if host is None or survives(called):
                leaves.add(qualify(host, called))
            else:
                pending.append((host, called))
    return leaves


def _is_evidence_repoint(repo: Path, base_ref: str, before: str, after: str) -> bool:
    """Правка сводится к раскрытию исчезнувшего адреса в его составляющие."""
    old_props, old_body = _split(before)
    new_props, new_body = _split(after)
    if old_body != new_body or set(old_props) != set(new_props):
        return False
    changed = {key for key in old_props if old_props[key] != new_props[key]}
    if not changed or any(key not in EVIDENCE_FIELDS for key in changed):
        return False

    for key in changed:
        old_names = _REGISTRY.evidence_names(old_props[key])
        new_names = _REGISTRY.evidence_names(new_props[key])
        if len(old_names) != 1 or len(new_names) < 2:
            return False
        old_reference = old_names[0]
        old_path, _, old_declaration = old_reference.partition("::")
        current = repo / old_path
        # Старый адрес либо исчез вместе с обёрткой, либо уцелел и тогда обязан
        # остаться в списке: обещание расширяется, а не подменяется.
        if (
            current.is_file()
            and _declaration_in(current.read_text(encoding="utf-8"), old_declaration)
            and old_reference not in new_names
        ):
            return False
        pool = {reference.partition("::")[0] for reference in new_names}
        covered = _covered_names(repo, base_ref, old_reference, pool)
        if not covered:
            return False
        # Покрытие — включение, а не равенство: список вправе называть больше,
        # чем исполняло исчезнувшее имя, и уцелевший адрес входит в него сам.
        # Лишнее имя обещание не ослабляет, недостающее — ослабляет.
        unqualified = {name for name in covered if name.startswith("::")}
        declarations = {reference.partition("::")[2] for reference in new_names}
        if not (covered - unqualified) <= set(new_names):
            return False
        if not {name[2:] for name in unqualified} <= declarations:
            return False
    return True


def _is_recordable_only(before: str, after: str) -> bool:
    """Правка сводится к простановке отметки и ничему больше."""
    old_props, old_body = _split(before)
    new_props, new_body = _split(after)
    if old_body != new_body:
        return False
    if set(old_props) != set(new_props):
        return False
    changed = {key for key in old_props if old_props[key] != new_props[key]}
    if not changed or any(key not in RECORDABLE_FIELDS for key in changed):
        return False

    if changed == set(SUPERSESSION_FIELDS):
        return (
            old_props["status"] in {"planned", "active"}
            and new_props["status"] == "superseded"
            and not old_props["superseded-by"]
            and bool(new_props["superseded-by"])
        )

    if changed == set(REALIZATION_FIELDS):
        return (
            old_props["status"] == "planned"
            and new_props["status"] == "active"
            and not old_props["realized"]
            and bool(new_props["realized"])
        )

    return False


def _records_introduced(repo: Path, base: dict[str, str]) -> dict[str, IntroducedRecord]:
    """Новые записи вместе с видом и props, нужными для проверки основания."""
    known = {
        props["id"]
        for text in base.values()
        if (props := _split(text)[0]).get("id")
    }
    introduced: dict[str, IntroducedRecord] = {}
    for directory, kind in (
        ("decisions", "decision"),
        ("invariants", "invariant"),
        ("contracts", "contract"),
    ):
        for path in sorted((repo / "arch" / directory).glob("*.md")):
            props, _ = _split(path.read_text(encoding="utf-8"))
            identifier = props.get("id")
            if identifier and identifier not in known:
                introduced[identifier] = IntroducedRecord(
                    kind=kind,
                    path=path.relative_to(repo).as_posix(),
                    props=props,
                )
    return introduced


def _python_defines(source: str, name: str) -> bool:
    try:
        tree = ast.parse(source)
    except SyntaxError:
        return False
    return any(
        isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == name
        for node in ast.walk(tree)
    )


def _is_test_attribute(attribute_item) -> bool:
    """Accept `#[test]` and a namespaced `#[...::test]`, never cfg_attr."""
    attribute = next(
        (child for child in attribute_item.named_children if child.type == "attribute"),
        None,
    )
    if attribute is None or not attribute.named_children:
        return False
    name = attribute.named_children[0]
    if name.type == "identifier":
        return name.text == b"test"
    if name.type == "scoped_identifier":
        final = name.child_by_field_name("name")
        return final is not None and final.text == b"test"
    return False


def _has_attached_test_attribute(node) -> bool:
    """Look at the contiguous preceding attribute chain in the syntax tree."""
    siblings = node.parent.children if node.parent is not None else ()
    try:
        index = siblings.index(node)
    except ValueError:
        return False
    found = False
    for sibling in reversed(siblings[:index]):
        if sibling.type in {"line_comment", "block_comment"}:
            continue
        if sibling.type != "attribute_item":
            break
        found = found or _is_test_attribute(sibling)
    return found


def _rust_nodes(node):
    yield node
    for child in node.children:
        yield from _rust_nodes(child)


def _rust_test_function_has_body(source: str, name: str) -> bool:
    """Resolve an exact attributed Rust function_item with a syntax-tree body."""
    if _RUST_PARSER is None:
        return False
    root = _RUST_PARSER.parse(source.encode("utf-8")).root_node
    if root.has_error:
        return False
    for node in _rust_nodes(root):
        if node.type != "function_item" or not _has_attached_test_attribute(node):
            continue
        function_name = node.child_by_field_name("name")
        if function_name is None or function_name.text.decode("utf-8") != name:
            continue
        if node.child_by_field_name("body") is not None:
            return True
    return False


def _evidence_resolves(repo: Path, evidence: object) -> bool:
    """The named evidence belongs to this repository and defines a test function."""
    if not isinstance(evidence, str):
        return False
    relative, separator, name = evidence.partition("::")
    if separator != "::" or not relative or not name:
        return False
    root = repo.resolve()
    target = (root / relative).resolve()
    if not target.is_relative_to(root) or not target.is_file():
        return False
    source = target.read_text(encoding="utf-8")
    if target.suffix == ".py":
        return _python_defines(source, name)
    if target.suffix == ".rs":
        return _rust_test_function_has_body(source, name)
    return False


def _evidence_dependency_error(evidence: object) -> str | None:
    if (
        _RUST_PARSER_ERROR is not None
        and isinstance(evidence, str)
        and evidence.partition("::")[0].endswith(".rs")
    ):
        return f"{_RUST_PARSER_ERROR}; install tests/ci/requirements.txt"
    return None


def _ground_error(repo: Path, ground: IntroducedRecord) -> str | None:
    """A new product-rule ground must be an implemented product decision."""
    if ground.kind != "decision":
        return f"основание {ground.path} не является decision"
    if ground.props.get("status") != "active":
        return f"решение {ground.path} имеет status {ground.props.get('status')!r}, не active"
    if ground.props.get("governs") != "product":
        return f"решение {ground.path} governs {ground.props.get('governs')!r}, не product"
    evidence = ground.props.get("realized")
    if not evidence:
        return f"решение {ground.path} не имеет realized evidence"
    if dependency_error := _evidence_dependency_error(evidence):
        return dependency_error
    if not _evidence_resolves(repo, evidence):
        return f"realized evidence {evidence!r} решения {ground.path} не разрешается"
    return None


def _list_property(value: object) -> tuple[str, ...]:
    if isinstance(value, list):
        return tuple(str(item) for item in value)
    if not isinstance(value, str) or not value.startswith("[") or not value.endswith("]"):
        return ()
    return tuple(item.strip() for item in value[1:-1].split(",") if item.strip())


def _surface_change_has_product_ground(
    repo: Path,
    introduced: dict[str, IntroducedRecord],
) -> bool:
    """A changed generated surface needs a new implemented wire decision."""
    for identifier, decision in introduced.items():
        if decision.kind != "decision" or _ground_error(repo, decision) is not None:
            continue
        if "CTR.WIRE.TOOL-SURFACE" not in _list_property(
            decision.props.get("changes")
        ):
            continue
        for established in _list_property(decision.props.get("establishes")):
            rule = introduced.get(established)
            if rule is None or rule.kind not in {"invariant", "contract"}:
                continue
            if rule.props.get("decision") != identifier:
                continue
            if rule.props.get("governs") != "product":
                continue
            if "wire" not in _list_property(rule.props.get("scope")):
                continue
            return True
    return False


def inspect(repo: Path, base_ref: str) -> Verdict:
    """Сверить принятые продуктовые записи базы с рабочим деревом."""
    base = _records_at(repo, base_ref)
    introduced = _records_introduced(repo, base)
    offenders: list[str] = []
    compared = 0

    for path, before in sorted(base.items()):
        props, _ = _split(before)
        if props.get("governs") != "product":
            continue
        compared += 1

        current = repo / path
        if not current.is_file():
            offenders.append(f"{path}: продуктовая запись удалена, а не заменена")
            continue

        after = current.read_text(encoding="utf-8")
        if after == before:
            continue

        if path.startswith("arch/decisions/"):
            if not _is_recordable_only(before, after) and not _is_evidence_repoint(
                repo, base_ref, before, after
            ):
                offenders.append(f"{path}: продуктовое решение отредактировано, а не заменено")
            continue

        # Инвариант и контракт: правка законна, если этой же правкой заведено
        # решение, на которое запись теперь и ссылается. Существующее основание
        # не годится — оно писалось раньше и этой перемены не предвидело.
        if _is_evidence_repoint(repo, base_ref, before, after):
            continue

        ground = _split(after)[0].get("decision")
        introduced_ground = introduced.get(ground)
        if introduced_ground is None:
            offenders.append(
                f"{path}: продуктовое правило изменено без нового решения о причине"
            )
            continue
        if error := _ground_error(repo, introduced_ground):
            offenders.append(f"{path}: продуктовое правило изменено, но {error}")

    surface_path = "arch/tool-surface.md"
    before_surface = base.get(surface_path)
    current_surface = repo / surface_path
    if (
        before_surface is not None
        and current_surface.is_file()
        and current_surface.read_text(encoding="utf-8") != before_surface
        and not _surface_change_has_product_ground(repo, introduced)
    ):
        offenders.append(
            f"{surface_path}: публичная поверхность изменена без нового "
            "продуктового решения с changes: [CTR.WIRE.TOOL-SURFACE] "
            "и wire-правила"
        )

    return Verdict(tuple(offenders), compared)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", default="origin/main")
    parser.add_argument("--repo", default=".")
    arguments = parser.parse_args(argv)

    verdict = inspect(Path(arguments.repo).resolve(), arguments.base)
    print(verdict.report())
    return 1 if verdict.offenders else 0


if __name__ == "__main__":
    raise SystemExit(main())
