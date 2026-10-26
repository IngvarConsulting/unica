#!/usr/bin/env python3
"""Render the acceptance scenario registry for contributors.

The corpus (`tests/fixtures/acceptance/`, loaded by
`scripts/ci/acceptance_corpus.py`) is the source of truth; the registry is its
rendered view.  It is not committed: its counts and coverage lists change with
every added scenario, so a committed copy would make any two such changes
conflict.  The site build publishes it as `acceptance-scenarios.html`.

Besides the catalogue itself the document cross-references the closed `apply`
registry (`crates/unica-coder/src/domain/apply.rs`) and the `check` profiles so
that a contributor can see which operations no scenario exercises yet and
propose the missing developer task.

    python scripts/ci/render-acceptance-registry.py                  # Markdown to stdout
    python scripts/ci/render-acceptance-registry.py --out registry.md
    python scripts/ci/render-acceptance-registry.py --html --out registry.html
"""

from __future__ import annotations

import argparse
import html
import importlib.util
import re
import sys
from collections import OrderedDict
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
CORPUS_STORAGE = REPO_ROOT / "scripts/ci/acceptance_corpus.py"
APPLY_REGISTRY = REPO_ROOT / "crates/unica-coder/src/domain/apply.rs"
CHECK_PROFILES = REPO_ROOT / "crates/unica-coder/src/application/v13/check.rs"
PAGE_NAME = "acceptance-scenarios.html"

# Composition fixes registration order; each physical family owns the literal
# table from which its enum and descriptor bindings are generated.
TYPED_FAMILIES = {
    "Role": "RoleOperation", "Support": "SupportOperation",
    "Metadata": "unica_metadata::apply_operations::MetadataOperation", "Form": "FormOperation",
    "Mxl": "MxlOperation", "Event": "EventOperation", "Dcs": "Primitive",
    "Properties": "PropertiesOperation", "Xdto": "XdtoOperation",
    "Code": "CodeOperation", "Subsystem": "SubsystemOperation",
    "Interface": "InterfaceOperation",
}
FAMILY_SOURCES = {
    "Role": REPO_ROOT / "crates/unica-roles/src/operations.rs",
    "Support": REPO_ROOT / "crates/unica-metadata/src/support.rs",
    "Metadata": REPO_ROOT / "crates/unica-metadata/src/apply_operations.rs",
    "Form": REPO_ROOT / "crates/unica-forms/src/operations.rs",
    "Mxl": REPO_ROOT / "crates/unica-templates/src/operations.rs",
    "Event": REPO_ROOT / "crates/unica-metadata/src/event.rs",
    "Dcs": REPO_ROOT / "crates/unica-dcs/src/operations.rs",
    "Properties": REPO_ROOT / "crates/unica-metadata/src/descriptor_operations.rs",
    "Xdto": REPO_ROOT / "crates/unica-xdto/src/operations.rs",
    "Code": REPO_ROOT / "crates/unica-bsl/src/operations.rs",
    "Subsystem": REPO_ROOT / "crates/unica-metadata/src/subsystem_operations.rs",
    "Interface": REPO_ROOT / "crates/unica-metadata/src/interface_operations.rs",
}

CLASS_TITLES = {
    "ok": "результат",
    "task": "длинная задача",
    "provider": "провайдер недоступен",
    "refused": "умышленный типизированный отказ",
    "unsupported": "закрытый список",
    "cancelled": "задача отменена",
    "failed": "задача завершилась ошибкой",
    "gap": "задокументированный пробел",
}

FAMILY_TITLES = OrderedDict(
    [
        ("Metadata", "Метаданные: объекты и коллекции"),
        ("Properties", "Свойства"),
        ("Form", "Формы"),
        ("Role", "Роли и права"),
        ("Dcs", "Схемы компоновки данных"),
        ("Mxl", "Табличные документы"),
        ("Xdto", "Пакеты XDTO"),
        ("Subsystem", "Подсистемы"),
        ("Support", "Поддержка поставщика"),
        ("Code", "Код BSL"),
        ("Event", "События"),
    ]
)

ARGS_TITLES = {
    "Values": "`values`",
    "Items": "`items`",
    "Text": "`text`",
    "Target": "без аргументов; цель задаёт `at` вызова",
}


def load_corpus() -> dict:
    spec = importlib.util.spec_from_file_location("acceptance_corpus", CORPUS_STORAGE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.load()


def _metadata_source(source: str, *, mask_strings: bool = False) -> str:
    """Ignore comments and literal decoys while locating a metadata table."""
    tokens = re.compile(r'''//[^\n]*|/\*|\b(?:br|cr|r)(?P<hashes>\#*)"|'(?:\\.|[^'\\])'|"''')
    ordinary_string = re.compile(r'"(?:\\.|[^"\\])*"', re.S)
    parts = []
    position = 0
    while match := tokens.search(source, position):
        parts.append(source[position:match.start()])
        token = match[0]
        is_string = token != "/*" and not token.startswith("//")
        end = match.end()
        if token == "/*":
            depth = 1
            while depth:
                boundary = re.search(r"/\*|\*/", source[end:])
                if boundary is None:
                    raise RuntimeError("unterminated metadata comment")
                depth += 1 if boundary[0] == "/*" else -1
                end += boundary.end()
        elif match["hashes"] is not None:
            closing = '"' + match["hashes"]
            end = source.find(closing, end)
            if end < 0:
                raise RuntimeError("unterminated metadata raw string")
            end += len(closing)
        elif token == '"':
            literal = ordinary_string.match(source, match.start())
            if literal is None:
                raise RuntimeError("unterminated metadata string")
            end = literal.end()
        parts.append(" " * (end - match.start()) if mask_strings or not is_string else source[match.start():end])
        position = end
    parts.append(source[position:])
    return "".join(parts)


def _literal_table(source: str, name: str) -> str:
    syntax = _metadata_source(source, mask_strings=True)
    tables = list(re.finditer(rf"\b{re.escape(name)}\s*!\s*\(", syntax))
    if len(tables) != 1:
        raise RuntimeError(f"exactly one {name} metadata table is required")
    start = tables[0].end()
    closing = re.search(r"\)\s*;", syntax[start:])
    if closing is None:
        raise RuntimeError(f"the {name} metadata table is incomplete")
    return _metadata_source(source[start:start + closing.start()])


def _registration_metadata(family: str) -> dict[str, tuple[str, str, str, str]]:
    stem = family.lower()
    path = FAMILY_SOURCES[family]
    try:
        source = path.read_text(encoding="utf-8")
    except OSError as error:
        raise RuntimeError(f"typed {family} metadata is unavailable: {path}") from error
    table_name = f"{stem}_operations"
    table = _literal_table(source, table_name)
    payload = (
        r'\s*,\s*(?:\(\s*\)|[A-Za-z_]\w*(?:\s*<\s*[A-Za-z_]\w*\s*>)?)'
        if family == "Dcs" else ""
    )
    row = re.compile(
        r'\s*([A-Za-z_]\w*)\s*=>\s*\(\s*"([A-Za-z]+\.[A-Za-z]+)"\s*,'
        r'\s*([A-Za-z_]\w*)\s*,\s*([A-Za-z_]\w*)'
        + payload + r'\s*\)\s*(?:,|$)'
    )
    entries = {}
    names = set()
    position = 0
    while table[position:].strip():
        match = row.match(table, position)
        if match is None:
            raise RuntimeError(f"unsupported or incomplete {table_name} metadata row")
        variant, name, applicability, skeleton = match.groups()
        if variant in entries or name in names:
            raise RuntimeError(f"{table_name} variants and names must be unique")
        entries[variant] = (name, family, applicability, skeleton)
        names.add(name)
        position = match.end()
    if not entries:
        raise RuntimeError(f"the {table_name} metadata table is empty")
    return entries


def load_apply_registry() -> tuple[list[tuple[str, str, str, str]], set[str]]:
    """Read raw descriptors and literal typed metadata without skipping entries."""
    source = APPLY_REGISTRY.read_text(encoding="utf-8")
    table = _literal_table(source, "operation_descriptors")
    pattern = re.compile(
        r'\(\s*(?:"([A-Za-z]+\.[A-Za-z]+)"\s*,\s*(\w+)\s*,\s*(\w+)\s*,\s*(\w+)'
        r'|([A-Za-z_]\w*(?:::[A-Za-z_]\w*)*)::([A-Za-z_]\w*))\s*\)'
    )
    entries = []
    typed = {operation_type: _registration_metadata(family) for family, operation_type in TYPED_FAMILIES.items()}
    typed_names = {(entry[1], entry[0]) for metadata in typed.values() for entry in metadata.values()}
    registered = {operation_type: [] for operation_type in typed}
    position = 0
    for match in pattern.finditer(table):
        if table[position:match.start()].strip(" \t\r\n,"):
            raise RuntimeError("unsupported apply registration; no entry may be skipped")
        if operation_type := match[5]:
            variant = match[6]
            if operation_type not in typed or variant not in typed[operation_type]:
                raise RuntimeError(f"unknown typed registration: {operation_type}::{variant}")
            registered[operation_type].append(variant)
            entries.append(typed[operation_type][variant])
        else:
            entry = match.group(1, 2, 3, 4)
            if entry[1] in TYPED_FAMILIES or (entry[1], entry[0]) in typed_names:
                raise RuntimeError(f"{entry[1]} registrations must use their typed operation")
            entries.append(entry)
        position = match.end()
    if table[position:].strip(" \t\r\n,"):
        raise RuntimeError("unsupported apply registration; no entry may be skipped")
    if not entries:
        raise RuntimeError("the apply registry table was not found in domain/apply.rs")
    for family, operation_type in TYPED_FAMILIES.items():
        if (
            len(registered[operation_type]) != len(typed[operation_type])
            or set(registered[operation_type]) != set(typed[operation_type])
        ):
            raise RuntimeError(f"typed {family} registrations must name every variant exactly once")
    # Every registered operation is implemented: the registry is the dictionary.
    implemented = {entry[0] for entry in entries}
    return entries, implemented


def load_check_validators() -> list[str]:
    source = CHECK_PROFILES.read_text(encoding="utf-8")
    start = source.index("const NATIVE_VALIDATORS: &[CheckValidator] = &[")
    end = source.index("];", start)
    names = re.findall(r"CheckValidator::(\w+)", source[start:end])
    validators = [name.lower() for name in names]
    # `meta` runs through the typed metadata validator, outside CheckValidator.
    validators.append("meta")
    return validators


def step_wire(step: dict) -> str:
    if step["tool"].startswith("lsp:"):
        return f"LSP `{step['tool'][len('lsp:'):]}`"
    tool = step["tool"].replace("unica.", "")
    args = step["args"]
    parts = [f"`{tool}`"]
    ops = [op.get("op", "?") for op in (args.get("ops") or [])]
    if ops:
        parts.append(" ".join(f"`{op}`" for op in ops))
    if step.get("validators"):
        parts.append("валидаторы " + ", ".join(f"`{name}`" for name in step["validators"]))
    if args.get("dryRun"):
        parts.append("(превью)")
    if step.get("form"):
        parts.append("(как форма клиента)")
    return " ".join(parts)


def step_classes(step: dict) -> str:
    if "knownGap" in step:
        gap = step["knownGap"]
        return f"`gap`: известный ошибочный `{gap['observedClass']}` (#{gap['issue']})"
    return " / ".join(f"`{name}`" for name in step["expect"])


def scenario_ops(scenario: dict) -> set[str]:
    ops = set()
    for step in scenario["wire"]:
        for op in step["args"].get("ops") or []:
            if op.get("op"):
                ops.add(op["op"])
    return ops


def scenario_validators(scenario: dict) -> set[str]:
    validators = set()
    for step in scenario["wire"]:
        if step["tool"] == "unica.check":
            validators.update(step.get("validators") or [])
    return validators


def render(corpus: dict, registry: list[tuple[str, str, str, str]], implemented: set[str], profiles: list[str]) -> str:
    scenarios = corpus["scenarios"]
    steps = [step for scenario in scenarios for step in scenario["wire"]]
    class_counts: dict[str, int] = {}
    for step in steps:
        key = " / ".join(step["expect"])
        class_counts[key] = class_counts.get(key, 0) + 1
    gaps = [scenario["id"] for scenario in scenarios if any("gap" in step["expect"] for step in scenario["wire"])]
    gap_steps = [(scenario["id"], step) for scenario in scenarios for step in scenario["wire"] if "gap" in step["expect"]]

    op_scenarios: dict[str, list[str]] = {}
    for scenario in scenarios:
        for op in scenario_ops(scenario):
            op_scenarios.setdefault(op, []).append(scenario["id"])
    profile_scenarios: dict[str, list[str]] = {}
    for scenario in scenarios:
        for profile in scenario_validators(scenario):
            profile_scenarios.setdefault(profile, []).append(scenario["id"])
    tool_scenarios: dict[str, list[str]] = {}
    for scenario in scenarios:
        for tool in {step["tool"] for step in scenario["wire"]}:
            tool_scenarios.setdefault(tool, []).append(scenario["id"])

    uncovered = [op for op, _, _, _ in registry if op not in op_scenarios]

    lines: list[str] = []
    out = lines.append
    out("# Реестр приёмочных сценариев поверхности v0.13")
    out("")
    out("<!-- Сгенерировано scripts/ci/render-acceptance-registry.py из корпуса")
    out("     tests/fixtures/acceptance/. Документ не хранится в репозитории:")
    out("     его публикует сборка сайта. -->")
    out("")
    out(
        f"Корпус держит **{len(scenarios)} сценариев** реальных задач разработчика конфигурации "
        f"и **{len(steps)} шагов** канонических вызовов `unica.*`. Каждый шаг заморожен в одном из "
        "классов ответа. Исходный профиль `tests/ci/test_acceptance_scenarios.py` исполняется "
        "против собранного `target/debug/unica`; основное рабочее пространство — "
        "`tests/fixtures/acceptance/workspace/`; сценарии форматных проб (выгрузка 2.21, без версии, "
        "без файла поддержки) идут на выведенном из него пространстве "
        "`tests/fixtures/acceptance/workspace-format/`, а сценарии ответа до допуска наборов — на пустом "
        "`tests/fixtures/acceptance/workspace-bare/`. Профиль delivery использует отдельную "
        "`tests/fixtures/acceptance/workspace-symbol/` с контрольным методом. "
        "СКД, MXL и вставка BSL проверяются на отдельных `workspace-dcs/`, `workspace-mxl/` и `workspace-code/`; точное расположение узлов — на `workspace-resolve/`; "
        "заимствованные объекты из выгрузки платформы — на `workspace-borrowed/`; "
        "неоднозначное имя командной панели формы — на `workspace-form-names/`; "
        "отображение команды формы из набора кнопки — на `workspace-form-command-representation/`; "
        "агентные сценарии выбираются локальным набором `tests/agent_evaluation/`. "
        "Источник истины — файлы корпуса; этот документ — его "
        "рендер для людей, собранный вместе с сайтом."
    )
    out("")
    source_count = sum(s.get("profile", "source") == "source" for s in scenarios)
    delivery_count = sum(s.get("profile") == "delivery" for s in scenarios)
    agent_count = sum(s.get("profile") == "agent-evaluation" for s in scenarios)
    fault_count = sum(s.get("driver") == "analyzer-jsonl-fault" for s in scenarios)
    delivery_failure_count = sum(s.get("driver") == "analyzer-delivery-failure" for s in scenarios)
    runtime_fault_count = sum(s.get("profile") == "runtime-fault" for s in scenarios)
    out(f"Профили исполнения: `source` — {source_count} сценария; `delivery/bsl-analyzer` — {delivery_count}; `agent-evaluation/codex` — {agent_count}; `fault-injection/analyzer-jsonl-fault` — {fault_count}; `fault-injection/analyzer-delivery-failure` — {delivery_failure_count}; `runtime-fault/pull-validation-fault` — {runtime_fault_count}. "
        "Runtime-fault запускается явно на POSIX с настоящей локальной платформой 8.3.27 для аттестации; управляемый provider проверяет отказы pull. Отсутствие платформы означает отказ, а не skip. Этот профиль не входит в обычный CI. "
        "Delivery-драйвер скачивает закреплённый движок с проверкой SHA256 и выполняет MCP на отдельной "
        "синтетической фикстуре. Ошибка доставки или незавершённая задача проваливает прогон. "
        "По решению владельца агентный профиль не входит в CI: его запускают локально при изменении проверяемых навыков или поверхности. "
        "Профиль использует реальный Codex, закрытый аудит инструментов, "
        "проверки итогового XML и независимый смысловой reviewer; отсутствие CLI или авторизации означает отказ прогона. "
        "Fault-injection исполняет управляемый нативный producer повреждённого JSONL и аварийного завершения через настоящий процесс и публичный check, а также убирает его или подменяет его контрольную сумму в манифесте, проверяя отказ об отсутствующем движке; это не проверка опубликованного анализатора. "
        "Драйвер analyzer-delivery-failure ставит пакет, который называет анализатор без его файла и без манифеста выпуска, и проверяет, что ветвь графа вызовов заказывает поставку и называет её провал типизированным отказом. "
        "Этот medium-профиль обязателен в очереди и не скачивает движок. Runtime добавляется с исполняемым драйвером; неизвестный профиль не пропускается.")
    out("")
    out("## Классы ответа шага")
    out("")
    out("| Класс | Что означает | Шагов |")
    out("| --- | --- | ---: |")
    for key in sorted(class_counts, key=lambda item: (-class_counts[item], item)):
        names = key.split(" / ")
        meaning = "; ".join(CLASS_TITLES.get(name, name) for name in names)
        if len(names) > 1:
            meaning = "любой из: " + meaning
        out(f"| {' / '.join(f'`{name}`' for name in names)} | {meaning} | {class_counts[key]} |")
    out("")
    out(
        "Провод проходной, когда каждый его шаг отвечает результатом или типизированным отказом своего "
        "класса. Сырой транспортный сбой и незадокументированный типизированный отказ роняют прогон. "
        f"Шагов с пробелом (`gap`) сейчас: **{len(gap_steps)}** в **{len(gaps)} сценариях**"
        + (f" ({', '.join(gaps)})." if gaps else ".")
    )
    out("")
    out("### Известные пробелы")
    out("")
    out("Потолок корпуса — три gap-шага. Известный ошибочный вердикт остаётся дефектом и не означает поддержку сценария.")
    out("")
    out("| Сценарий | Наблюдение | Причина |")
    out("| --- | --- | --- |")
    for identifier, step in gap_steps:
        reason = step["gap"].replace("|", "\\|").replace("\n", " ")
        out(f"| {identifier} | {step_classes(step)} | {reason} |")
    out("")
    out("## Как предложить сценарий или операцию")
    out("")
    out(
        "Идеи новых задач приветствуются именно здесь: если в таблице покрытия ниже операция помечена "
        "как непокрытая, или вы видите задачу разработчика, которую поверхность должна решать одним "
        "проводом, а сценария нет, — это и есть предложение."
    )
    out("")
    out("1. Откройте issue по форме [«Новая операция»](https://github.com/IngvarConsulting/unica/issues/new?template=new_operation.yml) ")
    out("   (она сама ставит метку `area:surface`): задача словами разработчика 1С и, если можете, провод ")
    out("   вызовов, которым её должно быть можно решить. Этого достаточно: сценарий допишем вместе.")
    out("2. Или добавьте сценарий сами. Каждый новый сценарий — отдельный файл ")
    out("   `tests/fixtures/acceptance/scenarios/<id>.json` с идентификатором `S-<issue>-<slug>`: ")
    out("   номер задачи и короткое имя латиницей через дефис. Сквозного номера нет, поэтому два ")
    out("   изменения со своими сценариями не правят одни и те же строки. Сценарии `S001`–`S440` ")
    out("   сохраняют свои номера в `scenario-corpus.json`; этот блок закрыт для новых. ")
    out("   Файл держит один сценарий: область из списка ниже, задача одной фразой и провод шагов. ")
    out("   Шаг — это `tool`, `args` и `expect`.")
    out("3. Заморозьте шаги сценария в манифесте: ")
    out("   `python scripts/ci/acceptance_corpus.py --freeze S-1489-required-attribute` пишет ")
    out("   `tests/fixtures/acceptance/scenario-manifest/<id>.steps`. Тест корпуса сверяет каждый ")
    out("   сценарий с его манифестом: пропавший сценарий или шаг без правки манифеста проваливает прогон.")
    out("4. Прогоните `python -m unittest tests/ci/test_acceptance_scenarios.py` и заморозьте тот класс, ")
    out("   которым поверхность действительно ответила. Умышленный отказ фиксируется с полем `refusal` — ")
    out("   точным текстом `код: сообщение` ответа; шаг `check` с профилем — с полем `status` (`passed` или ")
    out("   `failed`); пробел — с полем `gap` (причина); пробелов допускается не больше трёх, и каждый — ")
    out("   заявка на починку поверхности, а не на переутверждение корпуса.")
    out("")
    out("```json")
    out("{")
    out('  "id": "S-1489-required-attribute",')
    out('  "area": "Реквизиты и состав",')
    out('  "task": "Сделать реквизит обязательным для заполнения",')
    out('  "wire": [')
    out("    {")
    out('      "tool": "unica.apply",')
    out('      "args": {')
    out('        "at": "main:Catalog.Валюты.TabularSection.Представления.Attribute.КодЯзыка",')
    out('        "ops": [{"op": "attribute.set", "args": {"values": {"required": true}}}],')
    out('        "dryRun": true')
    out("      },")
    out('      "expect": ["ok"]')
    out("    }")
    out("  ]")
    out("}")
    out("```")
    out("")
    out("Адреса и аргументы в примере — реальные узлы фикстурного пространства; его состав описан в ")
    out("`tests/fixtures/acceptance/README.md`.")
    out("")
    out("## Покрытие реестра `apply`")
    out("")
    def operations(count: int) -> str:
        """Число с винительным падежом слова «операция»: 1 операцию, 2 операции, 5 операций."""
        if count % 10 == 1 and count % 100 != 11:
            word = "операцию"
        elif count % 10 in (2, 3, 4) and count % 100 not in (12, 13, 14):
            word = "операции"
        else:
            word = "операций"
        return f"**{count}** {word}"

    covered_count = len(registry) - len(uncovered)
    out(
        f"Закрытый реестр содержит {operations(len(registry))}; сценарии покрывают "
        f"{operations(covered_count)}. "
        "Операция без сценария — первая кандидатура для новой задачи."
    )
    out("")
    families: dict[str, list[tuple[str, str, str, str]]] = OrderedDict()
    for entry in registry:
        families.setdefault(entry[1], []).append(entry)
    for family, entries in families.items():
        out(f"### {FAMILY_TITLES.get(family, family)}")
        out("")
        out("| Операция | Аргументы | Сценарии |")
        out("| --- | --- | --- |")
        for op, _, _, args in entries:
            ids = op_scenarios.get(op, [])
            covered = ", ".join(ids) if ids else "**не покрыто — предложите задачу**"
            out(
                f"| `{op}` | {ARGS_TITLES.get(args, f'`{args}`')} | {covered} |"
            )
        out("")
    out("## Покрытие профилей `check`")
    out("")
    out("| Профиль | Сценарии |")
    out("| --- | --- |")
    for profile in profiles:
        ids = profile_scenarios.get(profile, [])
        out(f"| `{profile}` | {', '.join(ids) if ids else '**не покрыто — предложите задачу**'} |")
    out("")
    out("## Инструменты в проводах")
    out("")
    out("| Инструмент | Сценариев |")
    out("| --- | ---: |")
    for tool in sorted(tool_scenarios, key=lambda name: (-len(tool_scenarios[name]), name)):
        out(f"| `{tool}` | {len(tool_scenarios[tool])} |")
    out("")
    out("## Каталог сценариев")
    out("")
    areas: dict[str, list[dict]] = OrderedDict()
    for scenario in scenarios:
        areas.setdefault(scenario["area"], []).append(scenario)
    out("Области: " + " · ".join(f"{name} ({len(items)})" for name, items in areas.items()))
    out("")
    for name, items in areas.items():
        out(f"### {name}")
        out("")
        out("| № | Задача | Профиль | Провод | Классы шагов |")
        out("| --- | --- | --- | --- | --- |")
        for scenario in items:
            wire = "<br>".join(step_wire(step) for step in scenario["wire"])
            classes = "<br>".join(step_classes(step) for step in scenario["wire"])
            task = scenario["task"].replace("|", "\\|")
            execution = scenario.get("profile", "source")
            if scenario.get("driver"):
                execution += "/" + scenario["driver"]
            out(f"| {scenario['id']} | {task} | `{execution}` | {wire} | {classes} |")
        out("")
    return "\n".join(line.rstrip() for line in lines).rstrip("\n") + "\n"


def render_document() -> str:
    registry, implemented = load_apply_registry()
    return render(load_corpus(), registry, implemented, load_check_validators())


INLINE = re.compile(r"`([^`]+)`|\*\*(.+?)\*\*|\[([^\]]+)\]\(([^)\s]+)\)|<br>")
SCENARIO_ROW = re.compile(r"S[0-9]{3}|S-[1-9][0-9]*-[a-z0-9-]+")


def inline_html(text: str) -> str:
    """The inline Markdown the renderer emits: code, bold, links and `<br>`."""
    parts, position = [], 0
    for match in INLINE.finditer(text):
        parts.append(html.escape(text[position:match.start()], quote=False))
        code, bold, label, url = match.groups()
        if code is not None:
            parts.append(f"<code>{html.escape(code, quote=False)}</code>")
        elif bold is not None:
            parts.append(f"<strong>{inline_html(bold)}</strong>")
        elif label is not None:
            parts.append(f'<a href="{html.escape(url)}">{inline_html(label)}</a>')
        else:
            parts.append("<br>")
        position = match.end()
    parts.append(html.escape(text[position:], quote=False))
    return "".join(parts)


def table_cells(line: str) -> list[str]:
    return [cell.strip().replace("\\|", "|") for cell in re.split(r"(?<!\\)\|", line.strip())[1:-1]]


def render_html(markdown: str, stylesheet: str = "") -> str:
    """Render the registry Markdown as a standalone page.

    Only the subset `render` emits is understood: headings, paragraphs,
    pipe tables, ordered lists, fenced code and the leading comment.  A row
    of the catalogue carries its scenario id as an anchor, so
    `acceptance-scenarios.html#S047` points at the scenario."""
    lines = markdown.split("\n")
    body: list[str] = []
    index = 0
    title = ""
    while index < len(lines):
        line = lines[index]
        if not line.strip():
            index += 1
        elif line.startswith("<!--"):
            while "-->" not in lines[index]:
                index += 1
            index += 1
        elif line.startswith("```"):
            index += 1
            code = []
            while not lines[index].startswith("```"):
                code.append(lines[index])
                index += 1
            index += 1
            body.append("<pre><code>" + html.escape("\n".join(code), quote=False) + "</code></pre>")
        elif line.startswith("#"):
            level = len(line) - len(line.lstrip("#"))
            text = line[level:].strip()
            title = title or text
            body.append(f"<h{level}>{inline_html(text)}</h{level}>")
            index += 1
        elif line.startswith("|"):
            header = table_cells(line)
            index += 2
            rows = []
            while index < len(lines) and lines[index].startswith("|"):
                cells = table_cells(lines[index])
                anchor = f' id="{cells[0]}"' if header[0] == "№" and SCENARIO_ROW.fullmatch(cells[0]) else ""
                rows.append(f"<tr{anchor}>" + "".join(f"<td>{inline_html(cell)}</td>" for cell in cells) + "</tr>")
                index += 1
            body.append(
                "<table><thead><tr>" + "".join(f"<th>{inline_html(cell)}</th>" for cell in header)
                + "</tr></thead><tbody>" + "".join(rows) + "</tbody></table>"
            )
        elif re.match(r"[0-9]+\. ", line):
            items = []
            while index < len(lines) and re.match(r"[0-9]+\. ", lines[index]):
                item = [lines[index].split(". ", 1)[1]]
                index += 1
                while index < len(lines) and lines[index].startswith("   "):
                    item.append(lines[index].strip())
                    index += 1
                items.append("<li>" + inline_html(" ".join(item)) + "</li>")
            body.append("<ol>" + "".join(items) + "</ol>")
        else:
            paragraph = [line]
            index += 1
            # A continuation line may start with inline code; only a block
            # opener ends the paragraph.
            while index < len(lines) and lines[index].strip() and not re.match(
                r"#|\||```|<!--|[0-9]+\. ", lines[index]
            ):
                paragraph.append(lines[index])
                index += 1
            body.append("<p>" + inline_html(" ".join(paragraph)) + "</p>")
    style = stylesheet + """
  .registry { max-width: var(--content, 1080px); margin: 0 auto; padding: 24px 16px 64px; }
  .registry table { border-collapse: collapse; width: 100%; margin: 12px 0 24px; font-size: 14px; }
  .registry th, .registry td { border: 1px solid var(--line, #D8DEE8); padding: 6px 8px; vertical-align: top; text-align: left; }
  .registry .scroll { overflow-x: auto; }
  .registry tr:target { background: var(--accent, #93C5FD); }
  .registry pre { overflow-x: auto; }
"""
    tables = "\n".join(
        f'<div class="scroll">{block}</div>' if block.startswith("<table") else block for block in body
    )
    return (
        "<!doctype html>\n<html lang=\"ru\">\n<head>\n<meta charset=\"utf-8\">\n"
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n"
        f"<title>{html.escape(title)}</title>\n<style>{style}</style>\n</head>\n"
        f"<body>\n<main class=\"registry\">\n{tables}\n</main>\n</body>\n</html>\n"
    )


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--html", action="store_true", help="render the standalone HTML page")
    parser.add_argument("--out", type=Path, help="where to write; stdout by default")
    args = parser.parse_args(argv)
    rendered = render_document()
    if args.html:
        rendered = render_html(rendered)
    if args.out:
        args.out.write_text(rendered, encoding="utf-8")
        print(f"wrote {args.out}")
        return 0
    sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
