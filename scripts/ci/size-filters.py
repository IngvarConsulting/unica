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

Тот же проход пишет профиль `queue-platform`: тесты модулей, чьи файлы лежат
в платформенных каталогах правила INV.PLATFORM.OS-BEHIND-FACADE. Его очередь
исполняет на macOS вместо полного набора.

`--write` переписывает выражение в `.config/nextest.toml` между метками, беря
имена тестов из `cargo nextest list --message-format json` (нужен `cargo`);
страж размера в `tests/ci` читает то же выражение без `cargo` и проверяет, что
каждый файл с процессом или сокетом в нём назван, а внутри терма названы все
встроенные модули тестов этого файла. Что считать тестом, решает разбор в
страже — здесь второго ответа на этот вопрос нет.
"""

from __future__ import annotations

import argparse
import importlib.util
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
    "platform": ("# >>> платформа (очередь macOS): пишет scripts/ci/size-filters.py --write", "# <<< платформа (очередь macOS)"),
}
# Тест в исходнике: `#[test]`, `#[tokio::test]`, они же с аргументами.
TEST_ATTRIBUTE_IN_SOURCE = re.compile(r"#\[\s*(?:[A-Za-z0-9_]+\s*::\s*)*test\s*[\](]")
TERM = re.compile(r"test\(/\^([A-Za-z0-9_:]+)::")
# 400 000 исходных наборов: 34.447 с отдельно, 41.729 с совместно, macOS.
# Это размер конкретной проверки; соседние layout-проверки остаются small.
MEASURED_MEDIUM_CASES = (
    (
        "unica-project-health",
        "unica_project_health",
        "inspection::layout::tests::health_source_layout_keeps_all_defaulted_rows_and_their_real_ambiguity",
    ),
    # Библиотечные проверки от 3 с в ворота `pr`: PR-прогон 39649205116,
    # ubuntu, 21.10.2026. Вместе ~150 с из ~330 с суммарного (не настенного)
    # времени тестов PR; настенная фаза исполнения тогда шла 92 с.
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_main_corpus_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_resolve_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_code_and_extension_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_dcs_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_mxl_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_the_predefined_workspace"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::v13_can_honesty_tests::honest_can_on_external_sources"),
    ("unica-execution", "unica_execution", "receipt_store::tests::default_recovery_completes_after_former_automatic_timeout"),
    ("unica-coder", "unica_coder", "infrastructure::native_operations::apply_families::metadata::tests::contract_shape_tests::nested_type_and_position_shapes_reach_each_typed_member_reader"),
    ("unica-execution", "unica_execution", "lifecycle_link_store::tests::real_snapshot_over_8mib_can_reopen_mutate"),
    ("unica-coder", "unica_coder", "infrastructure::native_operations::apply_families::metadata::tests::member_operation_contracts_and_the_planner_agree_on_structure"),
    ("unica-coder", "unica_coder", "interfaces::task_projection::tests::error_fallback_is_bounded_without_losing_structured_diagnostics"),
    ("unica-coder", "unica_coder", "infrastructure::native_operations::apply_families::metadata::tests::contract_shape_tests::all_twenty_one_member_shapes_match_the_productive_parser_without_source_writes"),
    ("unica-execution", "unica_execution", "store::file_tests::raw_frame_exact_result_bound_reopens_and_rejects_corruption"),
    ("unica-coder", "unica_coder", "application::v13::view::tests::actual_cursor_size_is_reserved_for_late_items_and_metadata"),
    ("unica-coder", "unica_coder", "infrastructure::native_operations::apply_families::metadata::tests::contract_shape_tests::fill_shape_uses_the_same_owner_and_tabular_context_as_the_productive_reader"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::server::invocation_service::registration_cache_tests::logical_read_retains_root_artifacts_across_calls_and_isolates_sources"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::tests::canonical_invocation_authority_keeps_root_capabilities_without_a_tree_revision"),
    ("unica-execution", "unica_execution", "lifecycle_link_store::tests::production_links_reserve_and_materialize_past_4096_and_reopen"),
    ("unica-coder", "unica_coder", "infrastructure::v13_read::tests::named_external_owner_view_has_no_hidden_eight_mib_descriptor_fallback"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::tests::injected_hidden_v13_service_executes_real_view_and_find_through_actor_capabilities"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::tests::daemon_executes_one_canonical_invocation_and_poll_cancel_never_relaunches_it"),
    # apply над файлами больше прежних потолков #1119: настоящие 16 и 32 МиБ.
    # Локально, macOS arm64, 25.10.2026: около 3 с и 22 с.
    ("unica-coder", "unica_coder", "infrastructure::daemon::server::apply_large_input_tests::object_remove_scans_a_module_larger_than_the_former_bound"),
    ("unica-coder", "unica_coder", "infrastructure::daemon::server::apply_large_input_tests::apply_edits_a_descriptor_larger_than_the_former_staging_bound"),
    # Замер задержки MCP под потоком hover (#2173). Локально, macOS arm64,
    # 26.10.2026: 4,5–7 с.
    ("unica-coder", "unica_coder", "interfaces::lsp::tests::an_lsp_hover_stream_does_not_delay_a_parallel_mcp_call"),
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



def crate_root(path: Path) -> Path:
    for parent in path.parents:
        if (parent / "Cargo.toml").is_file():
            return parent
    raise ValueError(f"no Cargo.toml above {path}")


def included_file(text: str, file_dir: Path) -> Path | None:
    """Файл `include!`: литерал от каталога файла или от `CARGO_MANIFEST_DIR`."""
    compact = re.sub(r"\s+", "", text)
    literal = re.fullmatch(r'include!\("([^"]+)"\);?', compact)
    if literal:
        return file_dir / literal[1]
    manifest = re.fullmatch(r'include!\(concat!\(env!\("CARGO_MANIFEST_DIR"\),"([^"]+)"\)\);?', compact)
    if manifest:
        return crate_root(file_dir / "x") / manifest[1].lstrip("/")
    return None


def module_files(root_file: Path, logical_root: tuple[str, ...] = (),
                 includes: list[tuple[tuple[str, ...], Path]] | None = None) -> dict[tuple[str, ...], Path]:
    """Файлы модулей одной цели от её корня, включая перенесённые `#[path]`.

    Файлы `include!` вставляются в модуль, где стоит макрос: в `includes`
    уходит пара (модуль, файл).
    """
    modules: dict[tuple[str, ...], Path] = {}
    parser = Parser(Language(tree_sitter_rust.language()))

    def visit_file(path, logical):
        if not path.is_file():
            return
        source = path.read_bytes()
        modules[logical] = path
        base = path.parent if path.stem in {"lib", "main", "mod"} or logical == logical_root else path.with_suffix("")
        walk(parser.parse(source).root_node, source, logical, base, path.parent, path.parent)

    def walk(node, source, logical, default_base, explicit_base, file_dir):
        attributes = []
        for child in node.named_children:
            if child.type == "attribute_item":
                attributes.append(source[child.start_byte:child.end_byte].decode())
                continue
            macro = child.named_children[0] if child.type == "expression_statement" and child.named_children else child
            if includes is not None and macro.type == "macro_invocation":
                target = included_file(source[macro.start_byte:macro.end_byte].decode(), file_dir)
                if target is not None:
                    includes.append((logical, target))
            if child.type == "mod_item":
                name = child.child_by_field_name("name").text.decode()
                body = child.child_by_field_name("body")
                if body is not None:
                    walk(body, source, (*logical, name), default_base / name, default_base / name, file_dir)
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

    visit_file(root_file, logical_root)
    return modules


def source_modules(root: Path) -> dict[str, dict[tuple[str, ...], tuple[bool, Path]]]:
    """Resolve Rust module declarations, including moved #[path] unit targets."""
    found = {}
    for crate in sorted((root / "crates").iterdir()):
        src = crate / "src"
        if not src.is_dir():
            continue
        files = {**module_files(src / "main.rs", ("main",)), **module_files(src / "lib.rs")}
        found[crate.name] = {
            logical: (bool(MEDIUM_CONSTRUCTS.search(path.read_bytes().decode())), path)
            for logical, path in files.items()
        }
    return found


def platform_facade_roots(root: Path) -> tuple[str, ...]:
    """Закрытый набор владельцев правила INV.PLATFORM.OS-BEHIND-FACADE — из его стража."""
    spec = importlib.util.spec_from_file_location("rust_platform_boundary", root / "scripts" / "ci" / "check-rust-platform-boundary.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.PLATFORM_FACADE_ROOTS


def is_platform_source(root: Path, path: Path) -> bool:
    relative = path.resolve().relative_to(root.resolve()).as_posix()
    parts = relative.split("/")
    return (any(relative.startswith(prefix) for prefix in platform_facade_roots(root))
            or (len(parts) >= 5 and parts[0] == "crates" and parts[2] == "tests" and parts[3] == "platform"))


def test_targets(crate: Path) -> tuple[str, dict[str, Path]]:
    """Интеграционные цели пакета по правилам Cargo: автообнаружение и `[[test]]`."""
    manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    package = manifest["package"]["name"]
    targets: dict[str, Path] = {}
    if manifest["package"].get("autotests", True) and (crate / "tests").is_dir():
        targets.update({path.stem: path for path in sorted((crate / "tests").glob("*.rs"))})
        targets.update({path.parent.name: path for path in sorted((crate / "tests").glob("*/main.rs"))})
    for target in manifest.get("test", []):
        targets[target["name"]] = crate / target.get("path", f"tests/{target['name']}.rs")
    return package, targets


def bin_targets(crate: Path, package: str) -> dict[str, Path]:
    """Бинарные цели пакета по правилам Cargo: `src/main.rs`, `src/bin/*` и `[[bin]]`."""
    manifest = tomllib.loads((crate / "Cargo.toml").read_text(encoding="utf-8"))
    targets: dict[str, Path] = {}
    if manifest["package"].get("autobins", True):
        if (crate / "src" / "main.rs").is_file():
            targets[package] = crate / "src" / "main.rs"
        targets.update({path.stem: path for path in sorted((crate / "src" / "bin").glob("*.rs"))})
        targets.update({path.parent.name: path for path in sorted((crate / "src" / "bin").glob("*/main.rs"))})
    for target in manifest.get("bin", []):
        targets[target["name"]] = crate / target.get("path", f"src/bin/{target['name']}.rs")
    return targets


def test_items(source: bytes) -> tuple[set[str], set[str]]:
    """Встроенные модули с тестами и тесты верхнего уровня одного файла.

    Файл `include!` разбирается на одну глубину: его `mod x;` и вложенные
    `include!` не прослеживаются; сегодня таких нет.
    """
    named, _ = inline_test_modules(source)
    tree = Parser(Language(tree_sitter_rust.language())).parse(source)
    children = tree.root_node.named_children
    top_level = set()
    for index, child in enumerate(children):
        if child.type != "function_item":
            continue
        back = index - 1
        while back >= 0 and children[back].type in ("attribute_item", "line_comment", "block_comment"):
            node = children[back]
            if node.type == "attribute_item" and TEST_ATTRIBUTE.fullmatch(re.sub(r"\s+", "", node.text.decode())):
                top_level.add(child.child_by_field_name("name").text.decode())
            back -= 1
    return named, top_level


def platform_terms(root: Path) -> list[str]:
    """Тесты платформенных фасадов: модули, чей файл лежит в каталоге фасада.

    Выбор структурный, как у правила границы: верхний модуль на платформенном
    файле забирает всё своё поддерево, если в нём есть хоть один тест. Тесты
    под `cfg` другой ОС тоже называются — генератор не зависит от хоста.
    """
    terms = set()

    def collect(binary: str, files: dict[tuple[str, ...], Path], includes: list[tuple[tuple[str, ...], Path]]) -> None:
        platform = {logical for logical, path in files.items() if is_platform_source(root, path)}
        # `include!` вставляет платформенный файл в чужой модуль: выбор — его
        # собственные тесты, по точному имени или встроенному модулю.
        for logical, path in includes:
            if not path.is_file() or not is_platform_source(root, path) or any(logical[:n] in platform for n in range(len(logical) + 1)):
                continue
            prefix = "".join(f"{part}::" for part in logical)
            named, top_level_names = test_items(path.read_bytes())
            for name in sorted(named):
                terms.add(f"({binary} & test(/^{re.escape(prefix + name)}::/))")
            for name in sorted(top_level_names):
                terms.add(f"({binary} & test(/^{re.escape(prefix + name)}$/))")
        for logical in platform:
            if any(logical[:n] in platform for n in range(len(logical))):
                continue
            subtree = [path for other, path in files.items() if other[:len(logical)] == logical]
            if not any(TEST_ATTRIBUTE_IN_SOURCE.search(path.read_text(encoding="utf-8")) for path in subtree):
                continue
            prefix = "::".join(logical)
            terms.add(f"({binary} & test(/^{re.escape(prefix)}::/))" if prefix else f"({binary})")

    for crate in sorted((root / "crates").iterdir()):
        if not (crate / "Cargo.toml").is_file():
            continue
        package, targets = test_targets(crate)
        roots = [(f"binary({crate.name.replace('-', '_')})", crate / "src" / "lib.rs")]
        roots += [(f"binary_id({package}::bin/{name})", path) for name, path in bin_targets(crate, package).items()]
        roots += [(f"binary_id({package}::{name})", path) for name, path in sorted(targets.items())]
        for binary, path in roots:
            if path.is_file():
                includes: list[tuple[tuple[str, ...], Path]] = []
                collect(binary, module_files(path, includes=includes), includes)
    return sorted(terms)


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


def blocks(terms: list[str], large: str, platform: list[str]) -> dict[str, str]:
    body = render(terms)
    platform_body = "\n    | ".join(platform)
    return {
        "platform": f"default-filter = \'\'\'(\n{platform_body}\n) & not (\n{large}\n)\'\'\'",
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
    for name, body in blocks(terms, large, platform_terms(root)).items():
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
