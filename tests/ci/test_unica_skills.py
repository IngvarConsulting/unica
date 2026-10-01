from __future__ import annotations

import html
import json
import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
MIN_RUNTIME_GUIDANCE_DOCS = 31
# Runtime guidance points at the `unica.run` dictionary instead of restating
# per-operation rules; the remaining fenced examples call operations the
# dictionary implements. Syntax lives in `unica.check`, test runs and EPF/ERF
# publication are outside the v0.13 surface, so their examples are gone.
MIN_RUNTIME_EXECUTE_EXAMPLES = 2
# Reading and writing names the wire never publishes; the package README keeps
# its migration table of removed selectors and is not scanned for these.
RETIRED_READ_WRITE_NAMES = re.compile(
    r"unica\.code\.|unica\.meta\.info|unica\.meta\.add|unica\.meta\.edit|"
    r"unica\.\*\.info"
)
RETIRED_RUNTIME_NAMES = re.compile(
    r"unica\.runtime\.|unica\.build\.|runtime_risk_|runtime_operation_unbounded|"
    r"INV-MCP-RUNTIME-RECEIPT|ADR-0074"
)


# Both ways a document points at another one: a backticked path, where the
# slash separates a link from a bare filename mentioned as prose (`SKILL.md`),
# and a markdown link, where the target is a path whether it has a slash or not.
DOCUMENT_LINK_PATTERNS = (
    re.compile(r"`([^`\s]*/[^`\s]*\.md)`"),
    re.compile(r"\]\((?!\w+:)([^)\s#]+\.md)(?:#[^)\s]*)?\)"),
)
XDTO_DONOR_EVIDENCE_START = "<!-- xdto-donor-evidence:start -->"
XDTO_DONOR_EVIDENCE_END = "<!-- xdto-donor-evidence:end -->"
XDTO_DONOR_EVIDENCE_PATH = Path(
    "plugins/unica/references/specs/1c-xdto-spec.md"
)
XDTO_DONOR_EVIDENCE = re.compile(
    re.escape(XDTO_DONOR_EVIDENCE_START)
    + r".*?"
    + re.escape(XDTO_DONOR_EVIDENCE_END),
    re.DOTALL,
)


def document_links(text: str) -> list[str]:
    return [match for pattern in DOCUMENT_LINK_PATTERNS for match in pattern.findall(text)]


FENCED_BLOCK_START = re.compile(
    r"^[ \t]*(?P<fence>`{3,}|~{3,})(?P<info>[^\r\n]*)$"
)
BLOCKQUOTE_PREFIX = re.compile(r"^(?:[ \t]*>[ \t]?)+")
LIST_ITEM_PREFIX = re.compile(
    r"^[ \t]*(?:[-+*]|[0-9]{1,9}[.)])[ \t]+"
)
INDENTED_CODE_LINE = re.compile(r"^(?: {4}|\t)(?P<content>.*)$")
DRY_RUN_FALSE = re.compile(r'"dryRun"\s*:\s*false\b')


def markdown_container_content(line: str) -> str:
    while True:
        content = BLOCKQUOTE_PREFIX.sub("", line)
        content = LIST_ITEM_PREFIX.sub("", content, count=1)
        if content == line:
            return content
        line = content


def fenced_json_blocks(text: str) -> list[str]:
    lines = text.splitlines()
    blocks = []
    line_number = 0
    while line_number < len(lines):
        opening = FENCED_BLOCK_START.fullmatch(
            markdown_container_content(lines[line_number])
        )
        if opening is None:
            line_number += 1
            continue

        fence = opening.group("fence")
        closing = re.compile(
            rf"^[ \t]*{re.escape(fence[0])}{{{len(fence)},}}[ \t]*$"
        )
        info = html.unescape(opening.group("info").strip())
        language = info.split(maxsplit=1)[0].casefold() if info else ""
        line_number += 1
        body = []
        while (
            line_number < len(lines)
            and closing.fullmatch(
                markdown_container_content(lines[line_number])
            )
            is None
        ):
            body.append(markdown_container_content(lines[line_number]))
            line_number += 1
        block = "\n".join(body)
        if language == "json" or block_mentions_runtime_tool(block):
            blocks.append(block)
        if line_number < len(lines):
            line_number += 1
    return blocks


def decode_active_json_unicode_escapes(text: str) -> str:
    decoded = []
    index = 0
    while index < len(text):
        if text[index] != "\\":
            decoded.append(text[index])
            index += 1
            continue

        slash_start = index
        while index < len(text) and text[index] == "\\":
            index += 1
        slash_count = index - slash_start
        decoded.append("\\" * (slash_count // 2))
        if (
            slash_count % 2 == 1
            and index + 5 <= len(text)
            and text[index] == "u"
            and all(
                character in "0123456789abcdefABCDEF"
                for character in text[index + 1 : index + 5]
            )
        ):
            decoded.append(chr(int(text[index + 1 : index + 5], 16)))
            index += 5
        elif slash_count % 2 == 1:
            decoded.append("\\")
    return "".join(decoded)


def block_mentions_runtime_tool(block: str) -> bool:
    return '"unica.run' in decode_active_json_unicode_escapes(block)


def indented_code_blocks(text: str) -> list[str]:
    blocks = []
    current = []
    for line in text.splitlines():
        indented = INDENTED_CODE_LINE.match(line)
        if indented is not None:
            current.append(indented.group("content"))
            continue
        if current and not line.strip():
            current.append("")
            continue
        if current:
            blocks.append("\n".join(current))
            current = []
    if current:
        blocks.append("\n".join(current))
    return blocks


def runtime_arguments_have_if_rev(block: str) -> bool:
    """`ifRev` counts only inside `params.arguments`; a malformed block is
    judged by its text, because it cannot be judged by its shape."""
    try:
        payload = json.loads(decode_active_json_unicode_escapes(block))
    except json.JSONDecodeError:
        return '"ifRev"' in block
    candidates = payload if isinstance(payload, list) else [payload]
    for candidate in candidates:
        if not isinstance(candidate, dict):
            continue
        arguments = candidate.get("params", {}).get("arguments", {})
        if not isinstance(arguments, dict):
            return False
        if arguments.get("dryRun") is False and not (
            isinstance(arguments.get("ifRev"), str) and arguments["ifRev"]
        ):
            return False
    return True


def reject_indented_applied_runtime_examples(text: str) -> None:
    for block_number, block in enumerate(indented_code_blocks(text), start=1):
        if (
            block_mentions_runtime_tool(block)
            and DRY_RUN_FALSE.search(block)
            and not runtime_arguments_have_if_rev(block)
        ):
            raise ValueError(
                f"indented runtime JSON example #{block_number} applies without ifRev"
            )


def runtime_execute_json_examples(text: str) -> list[dict]:
    examples = []
    for block_number, block in enumerate(fenced_json_blocks(text), start=1):
        try:
            payload = json.loads(block)
        except json.JSONDecodeError as error:
            if block_mentions_runtime_tool(block):
                raise ValueError(
                    f"invalid fenced runtime JSON example #{block_number}: {error}"
                ) from error
            continue
        candidates = payload if isinstance(payload, list) else [payload]
        for candidate in candidates:
            if not isinstance(candidate, dict):
                continue
            params = candidate.get("params")
            if not isinstance(params, dict):
                continue
            if params.get("name") != "unica.run":
                continue
            examples.append(candidate)
    return examples


def runtime_guidance_document(text: str) -> tuple[bool, list[dict]]:
    reject_indented_applied_runtime_examples(text)
    examples = runtime_execute_json_examples(text)
    return (
        bool(examples) or "`unica.run`" in text or "v8-runner" in text,
        examples,
    )


def collect_runtime_guidance(
    docs: list[tuple[Path, str]],
) -> tuple[list[tuple[Path, str]], list[tuple[Path, dict]], list[tuple[Path, str]]]:
    runtime_docs = []
    runtime_examples = []
    parse_failures = []
    for doc, text in docs:
        try:
            is_runtime_document, payloads = runtime_guidance_document(text)
        except ValueError as error:
            parse_failures.append((doc, str(error)))
            continue
        if not is_runtime_document:
            continue
        runtime_docs.append((doc, text))
        for payload in payloads:
            if (
                payload.get("method") == "tools/call"
                and payload.get("params", {}).get("name") == "unica.run"
            ):
                runtime_examples.append((doc, payload["params"]["arguments"]))
    return runtime_docs, runtime_examples, parse_failures


def stale_route_guard_text(path: Path, text: str) -> str:
    """Subtract the one audited donor inventory, never a reusable marker block."""

    if path != XDTO_DONOR_EVIDENCE_PATH:
        return text
    matches = list(XDTO_DONOR_EVIDENCE.finditer(text))
    if (
        text.count(XDTO_DONOR_EVIDENCE_START) != 1
        or text.count(XDTO_DONOR_EVIDENCE_END) != 1
        or len(matches) != 1
    ):
        raise ValueError(
            f"{XDTO_DONOR_EVIDENCE_PATH} must contain exactly one donor evidence block"
        )
    match = matches[0]
    return text[: match.start()] + text[match.end() :]


PROMPT_FRONTMATTER_PATTERN = re.compile(
    r"\A---(?:\r\n|\r|\n)(?P<body>.*?)(?:\r\n|\r|\n)"
    r"---(?=\r\n|\r|\n|\Z)",
    re.DOTALL,
)


class UniqueKeySafeLoader(yaml.SafeLoader):
    """Safe YAML loader that also rejects duplicate and merge keys."""

    def construct_mapping(self, node, deep=False):
        if any(
            key_node.tag == "tag:yaml.org,2002:merge"
            for key_node, _ in node.value
        ):
            raise yaml.constructor.ConstructorError(
                "while constructing a mapping",
                node.start_mark,
                "YAML merge keys are not allowed in skill frontmatter",
                node.start_mark,
            )
        self.flatten_mapping(node)
        mapping = {}
        for key_node, value_node in node.value:
            key = self.construct_object(key_node, deep=deep)
            try:
                duplicate = key in mapping
            except TypeError as error:
                raise yaml.constructor.ConstructorError(
                    "while constructing a mapping",
                    node.start_mark,
                    "found an unhashable key",
                    key_node.start_mark,
                ) from error
            if duplicate:
                raise yaml.constructor.ConstructorError(
                    "while constructing a mapping",
                    node.start_mark,
                    f"found duplicate key {key!r}",
                    key_node.start_mark,
                )
            mapping[key] = self.construct_object(value_node, deep=deep)
        return mapping


def prompt_frontmatter_body(document: str) -> str | None:
    """Return the raw YAML frontmatter body when the document has one."""

    match = PROMPT_FRONTMATTER_PATTERN.match(document.removeprefix("\ufeff"))
    return None if match is None else match.group("body")


def prompt_frontmatter(document: str) -> dict[str, str]:
    """Return prompt-visible single-line scalar metadata from YAML frontmatter."""

    frontmatter = prompt_frontmatter_body(document)
    if frontmatter is None:
        return {}

    try:
        # UniqueKeySafeLoader derives from SafeLoader and cannot construct
        # arbitrary Python objects.
        values = yaml.load(frontmatter, Loader=UniqueKeySafeLoader)
        syntax = yaml.compose(frontmatter, Loader=yaml.SafeLoader)
        # An alias reprints a value written somewhere else, so the text a reader
        # sees next to the key is `*anchor` rather than the value this helper
        # would return. Frontmatter has no use for that indirection, so one
        # alias anywhere disqualifies the whole document.
        aliased = any(
            isinstance(event, yaml.AliasEvent)
            for event in yaml.parse(frontmatter, Loader=yaml.SafeLoader)
        )
    except yaml.YAMLError:
        return {}
    if aliased:
        return {}
    if not isinstance(values, dict) or not isinstance(syntax, yaml.MappingNode):
        return {}

    scalar_nodes = {
        key_node.value: (key_node, value_node)
        for key_node, value_node in syntax.value
        if isinstance(key_node, yaml.ScalarNode)
        and key_node.tag == "tag:yaml.org,2002:str"
    }
    fields = {}
    for field in ("description", "argument-hint"):
        value = values.get(field)
        nodes = scalar_nodes.get(field)
        if not isinstance(value, str) or nodes is None:
            continue
        key_node, value_node = nodes
        if not isinstance(value_node, yaml.ScalarNode):
            continue
        if value_node.style not in (None, "'", '"'):
            continue
        # A plain scalar may open on the line below its key, which puts the
        # text somewhere the reader does not look for it. Requiring the key and
        # value to begin on the same physical line rejects that.
        if key_node.start_mark.line != value_node.start_mark.line:
            continue
        if value_node.start_mark.line != value_node.end_mark.line:
            continue
        if not value.strip() or any(
            line_break in value for line_break in "\r\n\x85\u2028\u2029"
        ):
            continue
        fields[field] = value
    return fields


# Их предмет поверхность не создаёт: корень конфигурации и расширения,
# перехват метода, дескриптор внешней обработки или отчёта.
# Скилл остаётся справочником формата и обязан назвать пробел вслух.
SKILLS_WITHOUT_A_CANONICAL_ENTRY = {
    "cf-init",
    "cfe-init",
    "cfe-patch-method",
    "epf-init",
    "erf-init",
}

IN_SCOPE_TOOLS = {
    "cf-edit": "unica.apply",
    "cf-init": "unica.check",
    "cfe-borrow": "unica.apply",
    "cfe-init": "unica.check",
    "cfe-patch-method": "unica.check",
    "epf-init": "unica.check",
    "erf-init": "unica.check",
    "meta-add": "unica.apply",
    "meta-edit": "unica.apply",
    "meta-info": "unica.view",
    "form-compile": "unica.apply",
    "form-edit": "unica.apply",
    "interface-edit": "unica.apply",
    "subsystem-compile": "unica.apply",
    "subsystem-edit": "unica.apply",
    "dcs-compile": "unica.apply",
    "dcs-edit": "unica.apply",
    "mxl-compile": "unica.apply",
    "mxl-decompile": "unica.view",
    "mxl-info": "unica.view",
    "role-compile": "unica.apply",
    "role-edit": "unica.apply",
}

SCENARIO_SKILLS = {
    "api-design": [
        "unica.search",
        "unica.search",
        "unica.check",
        "unica.view",
        "unica.view",
        "unica.docs",
        "unica.run",
    ],
        # Поиск по тексту и по именам — один `search`, различается свод;
    # чтение узла и профиль объекта — один `view`.
    "code-search": [
        "unica.search",
        "unica.view",
        "unica.resolve",
    ],
    # Диагностика узла — канонический `check`; поиск — `search`; стандарт —
    # `docs`. Прогон платформы этому скиллу не нужен: он читает и судит.
    "code-diagnostics": [
        "unica.check",
        "unica.search",
        "unica.docs",
        "unica.view",
    ],
    "code-review": [
        "unica.search",
        "unica.search",
        "unica.check",
        "unica.view",
        "unica.docs",
        "unica.view",
        "unica.run",
    ],
    "query-optimize": [
        "unica.search",
        "unica.view",
        "unica.view",
        "unica.docs",
        "unica.run",
    ],
    "test-authoring": [
        "unica.search",
        "unica.view",
        "unica.check",
    ],
    "platform-help": [
        "unica.docs",
        "unica.search",
        "unica.view",
        "unica.run",
    ],
    "bsp-patterns": [
        "unica.search",
        "unica.view",
        "unica.view",
        "unica.docs",
        "unica.run",
    ],
    "integration-implement": [
        "unica.view",
        "unica.view",
        "unica.apply",
        "unica.apply",
        "unica.search",
        "unica.docs",
        "unica.run",
    ],
    "autonomous-server": [
        "unica.view",
        "unica.run",
        "unica.view",
        "unica.search",
        "unica.check",
    ],
    "log-analysis": [
        "unica.search",
        "unica.view",
        "unica.view",
        "unica.check",
        "unica.docs",
    ],
    "background-jobs": [
        "unica.view",
        "unica.search",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "data-exchange": [
        "unica.view",
        "unica.search",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "db-performance": [
        "unica.view",
        "unica.search",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "security-auth-crypto": [
        "unica.view",
        "unica.search",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "data-separation": [
        "unica.view",
        "unica.search",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "release-support": [
        "unica.view",
        "unica.search",
        "unica.diff",
        "unica.view",
        "unica.check",
        "unica.docs",
        "unica.run",
    ],
    "source-access": [
        "unica.resolve",
        "unica.view",
        "unica.search",
        "unica.diff",
        "unica.check",
    ],
    "document-posting": [
        "unica.view",
        "unica.view",
        "unica.apply",
        "unica.search",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
    "register-design": [
        "unica.view",
        "unica.view",
        "unica.apply",
        "unica.apply",
        "unica.search",
        "unica.check",
        "unica.run",
    ],
    "object-events": [
        "unica.view",
        "unica.view",
        "unica.search",
        "unica.search",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
    "form-events": [
        "unica.view",
        "unica.apply",
        "unica.view",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
    "module-placement": [
        "unica.view",
        "unica.view",
        "unica.apply",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
    "metadata-modeling": [
        "unica.view",
        "unica.view",
        "unica.apply",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
    "transactions-locks": [
        "unica.view",
        "unica.search",
        "unica.apply",
        "unica.check",
        "unica.view",
        "unica.run",
    ],
    "object-locks": [
        "unica.view",
        "unica.search",
        "unica.apply",
        "unica.check",
        "unica.run",
    ],
}

SCENARIO_REQUIRED_TOKENS = {
    "api-design": [
        "483",
        "543",
        "551",
        "553",
        "644",
        "Программный интерфейс",
        "Служебный программный интерфейс",
        "Переопределяемый интерфейс",
        "Устаревшие процедуры и функции",
        "API-first",
    ],
    "code-search": ["MCP-first", "what was tried"],
    "code-diagnostics": ["АПК", "EDT", "BSL LS", "отключ", "v8std"],
    "code-review": ["Findings first", "severity", "file/line"],
    "query-optimize": ["СКД", "virtual", "query-in-loop"],
    "test-authoring": ["YaXUnit", "Vanessa Automation"],
    "platform-help": [
        "platform-help contract gap",
        "development-standard",
        "method signatures",
    ],
    "bsp-patterns": ["БСП", "СведенияОВнешнейОбработке"],
    "integration-implement": ["HTTP-сервис", "webhook", "secrets"],
    "autonomous-server": ["HTTP-сервис", "веб-клиент", "external browser-testing tool"],
    "log-analysis": ["журнала регистрации", "технологического журнала", "ЖР", "ТЖ"],
    "background-jobs": ["Фоновые", "регламентные", "idempotency", "retry"],
    "data-exchange": ["планы обмена", "РИБ", "регистрация изменений", "контракт обмена"],
    "db-performance": ["SQL/DBMS trace", "индексы", "блокировки", "TEMPDB/WAL"],
    "security-auth-crypto": ["OpenID", "сертификаты", "CryptoPro", "секреты"],
    "data-separation": ["tenant-boundaries", "RLS", "разделители", "безопасные запросы"],
    # v8std governs posting shape, and the two rules an agent gets wrong by
    # default are std450 (the platform writes the sets, not the handler) and
    # std661 (control queries run *after* the controlled write, not before).
    # Guidance that drops either one reproduces the textbook antipattern.
    "document-posting": [
        "RegisterRecords",
        "RealTimePosting",
        "RegisterRecordsDeletion",
        "РежимПроведенияДокумента.Оперативный",
        "БлокироватьДляИзменения",
        # Lock order itself is owned by transactions-locks; what this skill
        # must not lose is the deferral to it.
        "transactions-locks",
        "std450",
        "std661",
        "АПК:105",
        "АПК:226",
    ],
    # std664 (separate totals for write concurrency) and std733 (no separation
    # for the cheapest balance read) pull opposite ways. Guidance that names
    # only one of them turns a trade-off into a false rule, so both ids and the
    # dimension/resource split they hang off stay pinned here.
    "register-design": [
        "RegisterType",
        "EnableTotalsSplitting",
        "EnableTotalsSliceLast",
        "WriteMode",
        "DenyIncompleteValues",
        "std664",
        "std733",
        "std708",
        "std792",
        "АПК:229",
    ],
    # Both cross-cutting rules are ones a handler silently violates: std773
    # (the exchange guard, which subscriptions forget as often as modules) and
    # std686 (assigning anything but Истина to Отказ clears another
    # subscriber's refusal). std463's remove-from-ПроверяемыеРеквизиты shape is
    # pinned because the inverse reads as correct and hides the condition.
    "object-events": [
        "ОбменДанными.Загрузка",
        "ПроверяемыеРеквизиты",
        "std773",
        "std686",
        "std463",
        "std465",
        "АПК:75",
        "АПК:144",
    ],
    # The form module is the one place directives are mandatory (std439) and
    # the one place a careless line costs a round trip (std487). Both are
    # invisible in review without being named. `Подключаемый_` is pinned
    # because nothing else in the surface mentions it.
    "form-events": [
        "&НаСервереБезКонтекста",
        "Подключаемый_",
        "Параметры.Свойство()",
        "std439",
        "std487",
        "std492",
        "std741",
        "АПК:547",
        "АПК:1410",
    ],
    # std469 admits exactly four flag combinations and std679 makes `Вызов
    # сервера` an exposure decision rather than a convenience — the flag gets
    # set to make a call compile otherwise. The scope line is pinned because
    # this skill and api-design describe adjacent halves of one subject.
    "module-placement": [
        "ВызовСервера",
        "КлиентСервер",
        "ПовтИсп",
        "Вызов сервера",
        "std469",
        "std486",
        "std679",
        "std724",
        "std746",
        "АПК:125",
        "api-design",
    ],
    # The class choice hangs on who may change the value set, and the
    # enumeration-versus-characteristic-types mistake is the one that costs a
    # migration later. std728's two composite rules are pinned because
    # `ЛюбаяСсылка` and a mixed type set both read as convenient shortcuts.
    "metadata-modeling": [
        "ЛюбаяСсылка",
        "ХранилищеЗначения",
        "ОбновлениеПредопределенныхДанных",
        "std432",
        "std697",
        "std704",
        "std728",
        "АПК:1329",
        "АПК:1330",
        "register-design",
    ],
    # This skill is the owner four others defer to, so the deferral is pinned
    # alongside the rules. std783's "an exception does not roll back" is the
    # single most load-bearing fact here: code written without it looks correct
    # and leaves the transaction open. std648's definition of a responsible
    # read is what decides whether a lock is needed at all.
    "transactions-locks": [
        "std648",
        "std783",
        "std460",
        "std659",
        "Заблокировать()",
        "ОтменитьТранзакцию",
        "lock order",
        "does not roll the transaction back",
        "АПК:1319",
        "АПК:1327",
        "Scope boundary",
        # A failed object lock is the documented exception to the rollback
        # reflex, so the two owners must keep pointing at each other.
        "object-locks",
    ],
    # Unlike the rest of this layer, the behaviour here is platform behaviour,
    # not a standards cluster: v8std carries one rule (std490) and the 8.3.27
    # Developer Guide carries the mechanism. Two facts are load-bearing and
    # counter-intuitive: the pessimistic lock stops other *locks*, not other
    # writes, and a failed object lock does not require a rollback.
    "object-locks": [
        "std490",
        "cooperative",
        "ЗаблокироватьДанныеДляРедактирования",
        "form id",
        "Optimistic locking guarantees only non-overwriting",
        "does not prevent the object in the database",
        "transactions-locks",
    ],
    "release-support": ["сравнение/объединение", "Поставка", "поддержка", "совместимость"],
    "source-access": [
        "предметн",
        "dryRun",
        "unica.apply",
        "unica.resolve",
        "invalid_cursor",
    ],
}

REPLACED_RUNTIME_SKILLS = {
    "db-create",
    "db-list",
    "db-dump-xml",
    "db-dump-cf",
    "db-load-xml",
    "db-load-cf",
    "db-load-git",
    "db-update",
    "db-run",
    "workspace-init",
    "epf-build",
    "epf-dump",
    "epf-validate",
    "erf-build",
    "erf-dump",
    "erf-validate",
}

TASK_EXAMPLE_ARGUMENT_KEYS = {
    "cfe-borrow": ["at", "ops", "dryRun"],
    "cf-edit": ["at", "ops"],
    "meta-add": ["at", "ops"],
    "meta-edit": ["at", "ops"],
    "meta-info": ["at"],
    "form-compile": ["at", "ops"],
    "form-edit": ["at", "ops"],
    "interface-edit": ["at", "ops"],
    "subsystem-compile": ["at", "ops"],
    "subsystem-edit": ["at", "ops"],
    "dcs-compile": ["at", "ops"],
    "dcs-edit": ["at", "ops"],
    "mxl-compile": ["at", "ops"],
    "mxl-decompile": ["at"],
    # Читающий макет адресуется логически: файлового селектора у `view` нет.
    "mxl-info": ["at"],
    "role-compile": ["at", "ops"],
    "role-edit": ["at", "ops"],
}

SCENARIO_PRESERVING_MIN_MCP_CALLS = {
    "meta-add": 2,
    "meta-edit": 4,
    "meta-info": 3,
    "form-compile": 4,
    "interface-edit": 3,
    "subsystem-compile": 3,
    "subsystem-edit": 2,
    "dcs-compile": 5,
    "mxl-info": 3,
    "role-edit": 1,
    "dcs-edit": 4,
    "role-compile": 4,
}

ALLOWED_ADDITIONAL_MCP_TOOL_NAMES = {
    "cfe-borrow": {"unica.view"},
    "form-compile": {"unica.view", "unica.check"},
    "role-compile": {"unica.view", "unica.check"},
    "dcs-compile": {"unica.view", "unica.check"},
    "dcs-edit": {"unica.view", "unica.check"},
    "meta-info": {"unica.check"},
}

SCENARIO_PRESERVING_TOKENS = {
    "cf-edit": [
        '"op": "props.set"',
        '"op": "object.create"',
    ],
    "cf-init": [
        '"Name": "МояКонфигурация"',
        '"Version": "1.0.0.1"',
        '"Vendor": "Фирма 1С"',
        "Режим совместимости (default: `Version8_3_27`)",
        '"CompatibilityMode": "Version8_3_27"',
        '"name": "unica.view"',
        '"name": "unica.check"',
    ],
    "cfe-init": [
        '"ConfigPath": "C:\\\\WS\\\\tasks\\\\cfsrc\\\\erp_8.3.24"',
        '"Purpose": "Patch"',
        '"CompatibilityMode": "Version8_3_17"',
        '"Version": "1.0.0.1"',
        '"NamePrefix": "ИБ_"',
        '"NoRole": true',
        '"name": "unica.check"',
    ],
    "cfe-patch-method": [
        '"InterceptorType": "Before"',
        '"InterceptorType": "After"',
        '"Context": "НаКлиенте"',
        '"IsFunction": false',
    ],
    "meta-add": [
        '"kind": "Catalog"',
        '"name": "НовыйСправочник"',
        '"kind": "EventSubscription"',
        '"relation": "source"',
        '"dryRun": true',
    ],
    # Режим операции стал её именем: `editRelations` с `mode: "replace"`
    # свёлся к `relation.replace`, а коллекция — к префиксу имени.
    "meta-edit": [
        '"op": "props.set"',
        '"op": "attribute.set"',
        '"op": "attribute.remove"',
        '"op": "predefinedItem.add"',
        '"op": "relation.replace"',
        '"relation": "source"',
        '"kind": "recordSet"',
        '"metadataPath": "InformationRegister.ИсторияИзменений"',
        '"targets": [',
    ],
    # `Name` and `Mode` were report selectors. The typed answer carries the
    # whole object, so the scenarios are preserved by the addresses they read,
    # not by the drill-down argument that no longer exists.
    # Путь метаданных стал логическим адресом, а вердикт ушёл в свой вход.
    "meta-info": [
        '"at": "main:Catalog.Валюты"',
        '"at": "main:Document.Заказ.Relation"',
        '"name": "unica.check"',
    ],
    "form-compile": [
        '"op": "form.create"',
        '"op": "formAttribute.add"',
    ],
    # Действие стало именем операции. `hide` и `show` свелись к одному
    # `commandVisibility.set` с булевым значением: платформа хранит одно поле,
    # и двух операций для него не нужно.
    "interface-edit": [
        '"op": "commandVisibility.set"',
        '"op": "commandPlacement.set"',
        '"op": "commandOrder.set"',
        '"op": "subsystemOrder.set"',
        '"visible": false',
        '"visible": true',
    ],
    # Определение стало типизированными операциями, а родитель — адресом:
    # JSON-строки внутри JSON и путей к XML на канонической поверхности нет.
    "subsystem-compile": [
        '"op": "subsystem.create"',
        'CommonPicture.Продажи',
        '"at": "main:Subsystem.Продажи"',
    ],
    # Операция стала именем операции, а не значением поля `Operation`.
    "subsystem-edit": [
        '"op": "content.add"',
        '"op": "content.remove"',
        '"op": "childSubsystem.add"',
        '"op": "props.set"',
    ],
    # Пресет разворачивает скилл: инструмент принимает одно право за
    # операцию, и предпросмотр показывает их поимённо, а не имя пресета.
    "role-compile": [
        '"op": "role.create"',
        '"op": "right.set"',
        '"name": "unica.check"',
        '"name": "unica.view"',
    ],
    "dcs-compile": [
        '"templateType": "DataCompositionSchema"',
        '"op": "query.set"',
    ],
    "dcs-edit": [
        '"op": "field.add"',
    ],
    # Eleven `Mode` values selected eleven reports. The typed answer carries
    # every section at once, so the scenarios are preserved by the sections the
    # skill names, not by the selector that no longer exists.
    # Содержимое ячеек стало отдельным адресом, а не признаком в аргументах,
    # поэтому сценарий сохраняется адресом ветви, а не селектором состава.
    "mxl-info": [
        "Area.Шапка.Body",
        "columnSets",
        "contentCount",
    ],
}

# Arguments the MCP contract used to publish and now rejects. The packaged skill
# must not keep advertising them: the server would answer such a call with
# "does not accept argument", so a leftover example is a broken instruction.
SCENARIO_RETIRED_TOKENS = {
    "meta-add": ['"JsonPath"', '"OutputDir"', '"DefinitionFile"', '"sourceSet"'],
    "meta-edit": [
        '"ObjectPath"',
        '"Operation"',
        '"Value"',
        '"DefinitionFile"',
        '"sourceSet"',
    ],
    "mxl-info": [
        '"Format"',
        '"MaxParams"',
        '"Limit"',
        '"Offset"',
        '"TemplatePath"',
        '"WithText"',
    ],
    "meta-info": [
        '"ObjectPath"',
        '"objectPath"',
        '"Detailed"',
        '"detailed"',
        '"sourceSet"',
        '"metadataPath"',
    ],
}


def implemented_apply_operations(repo_root: Path) -> set[str]:
    """Имена, которые `unica.apply` действительно исполняет.

    Список живёт в Rust и меняется вместе с продуктом; переписать его здесь
    значило бы держать второй реестр, который устаревает молча. Пример скилла,
    назвавший имя вне этого списка, учит вызову, отвечающему отказом.
    """
    source = (
        repo_root / "crates/unica-coder/src/domain/apply.rs"
    ).read_text(encoding="utf-8")
    marker = "pub(crate) const IMPLEMENTED_APPLY_OPERATIONS: &[&str] = &["
    start = source.index(marker) + len(marker)
    body = source[start : source.index("];", start)]
    return set(re.findall(r'"([^"]+)"', body))


def markdown_routing_units(text: str) -> list[str]:
    units = []
    current = []
    item_start = re.compile(r"^\s*(?:[-*+]|\d+[.)])\s+")

    def flush() -> None:
        if current:
            units.append(" ".join(current))
            current.clear()

    for line in text.splitlines():
        stripped = line.strip()
        if not stripped:
            flush()
            continue
        if item_start.match(line):
            flush()
        current.append(stripped)
    flush()
    return [
        claim.strip()
        for unit in units
        for claim in re.split(r"(?<=[.!?;])\s+", unit)
        if claim.strip()
    ]


def find_unsafe_platform_evidence_routes(
    documents: list[tuple[str, str]],
) -> list[str]:
    safe_boundaries = [
        "development-standard",
        "development standards",
        "not platform",
        "do not infer",
        "do not present",
    ]
    unsafe_routes = []

    for display_path, text in documents:
        for normalized in markdown_routing_units(text):
            lowered = normalized.casefold()
            mentions_standards_tool = "unica.standards." in lowered
            mentions_platform_evidence = "platform" in lowered or "платформ" in lowered
            marks_the_source_boundary = any(
                boundary in lowered for boundary in safe_boundaries
            )
            if (
                mentions_standards_tool
                and mentions_platform_evidence
                and not marks_the_source_boundary
            ):
                unsafe_routes.append(f"{display_path}: {normalized}")

    return unsafe_routes


class PromptFrontmatterParsingTests(unittest.TestCase):
    def test_empty_value_does_not_capture_the_next_frontmatter_field(self) -> None:
        document = "---\ndescription:\nargument-hint: insert replace\n---\n"

        self.assertNotIn("description", prompt_frontmatter(document))

    def test_direct_scalar_alias_is_not_a_physical_field_value(self) -> None:
        documents = {
            "plain": (
                "---\ndefault: &description insert replace\n"
                "description: *description\n"
                "argument-hint: insert replace\n---\n"
            ),
            "quoted": (
                '---\ndefault: &description "insert replace"\n'
                "description: *description\n"
                "argument-hint: insert replace\n---\n"
            ),
            # A flow mapping puts the anchor on the key's own physical line, so
            # the line comparison alone would read the anchored text as if it
            # stood next to `description`.
            "flow-mapping": (
                "---\n{default: &description insert replace, "
                "description: *description}\n---\n"
            ),
        }

        for style, document in documents.items():
            with self.subTest(style=style):
                self.assertNotIn("description", prompt_frontmatter(document))

    def test_invalid_prompt_metadata_values_are_rejected(self) -> None:
        invalid_descriptions = {
            "whitespace": (
                "---\ndescription:   \nargument-hint: insert replace\n---\n"
            ),
            "missing-yaml-separation": (
                "---\ndescription:insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "quoted-empty": (
                '---\ndescription: "" # insert replace\n'
                "argument-hint: insert replace\n---\n"
            ),
            "quoted-concatenation": (
                '---\ndescription: "" "insert replace"\n'
                "argument-hint: insert replace\n---\n"
            ),
            "flow-sequence": (
                "---\ndescription: [insert, replace]\n"
                "argument-hint: insert replace\n---\n"
            ),
            "flow-mapping": (
                "---\ndescription: {insert: replace}\n"
                "argument-hint: insert replace\n---\n"
            ),
            "invalid-plain-colon": (
                "---\ndescription: insert: replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "invalid-trailing-colon": (
                "---\ndescription: insert replace:\n"
                "argument-hint: insert replace\n---\n"
            ),
            "quoted-duplicate-key": (
                "---\ndescription: insert replace\n"
                '"description": initialize\n'
                "argument-hint: insert replace\n---\n"
            ),
            "invalid-unrelated-field": (
                "---\ndescription: insert replace\n"
                "argument-hint: insert replace\n"
                "unrelated: [unterminated\n---\n"
            ),
            "control-line-separator": (
                "---\vdescription: insert replace\v"
                "argument-hint: insert replace\v---"
            ),
            "internal-control-separator": (
                "---\ndescription: insert replace\v"
                "argument-hint: insert replace\n---\n"
            ),
            "duplicate-unrelated-key": (
                "---\ndescription: insert replace\n"
                "argument-hint: insert replace\n"
                "unrelated:\n  key: one\n  'key': two\n---\n"
            ),
            "yaml-merge-key": (
                "---\ndefaults: &defaults {unrelated: true}\n"
                "<<: *defaults\n"
                "description: insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "block-sequence-indicator": (
                "---\ndescription: - insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "explicit-key-indicator": (
                "---\ndescription: ? insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "null-with-comment": (
                "---\ndescription: null # insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "body-decoy": (
                "---\nargument-hint: insert replace\n---\n"
                "description: insert replace\n"
            ),
            "folded-scalar": (
                "---\ndescription: >\n  insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "multiline-plain-scalar": (
                "---\ndescription: insert\n  replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "value-below-its-key": (
                "---\ndescription:\n  insert replace\n"
                "argument-hint: insert replace\n---\n"
            ),
            "escaped-line-break": (
                '---\ndescription: "insert \\L replace"\n'
                "argument-hint: insert replace\n---\n"
            ),
        }

        for case, document in invalid_descriptions.items():
            with self.subTest(case=case):
                self.assertNotIn("description", prompt_frontmatter(document))

    def test_single_line_prompt_metadata_values_are_accepted(self) -> None:
        documents = {
            "double-quoted": (
                '---\ndescription: "insert replace" # visible value\n'
                "argument-hint: insert replace\n---\n"
            ),
            "single-quoted": (
                "---\ndescription: 'insert replace' # visible value\n"
                "argument-hint: insert replace\n---\n"
            ),
            "escaped-character": (
                '---\ndescription: "insert \\x72eplace"\n'
                "argument-hint: insert replace\n---\n"
            ),
            "quoted-keys": (
                '---\n"description": insert replace\n'
                "'argument-hint': insert replace\n---\n"
            ),
        }

        for case, document in documents.items():
            with self.subTest(case=case):
                self.assertEqual(
                    prompt_frontmatter(document),
                    {
                        "description": "insert replace",
                        "argument-hint": "insert replace",
                    },
                )


class UnicaSkillRoutingTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def skill_root(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "skills"

    def reference_root(self) -> Path:
        return self.repo_root() / "plugins" / "unica" / "references"

    def test_read_only_skills_do_not_offer_outfile(self) -> None:
        read_only_skills = [
            "meta-info",
            "mxl-info",
            "mxl-decompile",
        ]

        for skill in read_only_skills:
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(
                    encoding="utf-8"
                )
                self.assertNotIn("OutFile", text)
                self.assertNotIn("outFile", text)

    def unica_reference_models_root(self) -> Path:
        return (
            self.repo_root()
            / "tests"
            / "fixtures"
            / "unica_mcp_script_parity"
            / "unica_reference_models"
        )

    def test_meta_skill_surface_is_exactly_three_canonical_entries(self) -> None:
        expected = {
            "meta-info": "unica.view",
            "meta-add": "unica.apply",
            "meta-edit": "unica.apply",
        }
        actual = {
            path.name
            for path in self.skill_root().glob("meta-*")
            if (path / "SKILL.md").is_file()
        }

        self.assertEqual(actual, set(expected))
        for skill, tool in expected.items():
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(
                    encoding="utf-8"
                )
                self.assertIn("## MCP routing", text)
                self.assertIn("MCP `unica`", text)
                self.assertIn(tool, text)
                # Снятые имена не должны остаться ни в одном маршруте: сервер
                # ответит на них `unknown unica tool`.
                for retired in ("unica.meta.info", "unica.meta.add", "unica.meta.edit"):
                    self.assertNotIn(retired, text)

    def test_meta_examples_follow_the_canonical_contracts(self) -> None:
        documents = {
            skill: (self.skill_root() / skill / "SKILL.md").read_text(
                encoding="utf-8"
            )
            for skill in ("meta-info", "meta-add", "meta-edit")
        }
        calls = {
            skill: [
                json.loads(block)
                for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
                if '"method": "tools/call"' in block
            ]
            for skill, text in documents.items()
        }

        # Читатель называет адрес и ничего больше: набора исходников и пути
        # метаданных во входе канонического `view` нет.
        self.assertTrue(calls["meta-info"])
        for call in calls["meta-info"]:
            arguments = call["params"]["arguments"]
            self.assertIn(call["params"]["name"], {"unica.view", "unica.check"})
            self.assertLessEqual(set(arguments), {"at", "filter", "limit", "cursor"})
            self.assertTrue(arguments["at"].startswith("main:"))
        self.assertTrue(
            any(call["params"]["name"] == "unica.check" for call in calls["meta-info"]),
            "вердикт об объекте спрашивает свой вход",
        )

        written = []
        for skill in ("meta-add", "meta-edit"):
            self.assertTrue(calls[skill], skill)
            for call in calls[skill]:
                arguments = call["params"]["arguments"]
                self.assertEqual(call["params"]["name"], "unica.apply")
                self.assertLessEqual(
                    set(arguments), {"at", "ops", "dryRun", "ifRev"}
                )
                self.assertTrue(arguments["ops"])
                for operation in arguments["ops"]:
                    self.assertLessEqual(set(operation), {"op", "args"})
                    self.assertTrue(operation["args"]["at"].startswith("main:"))
                    written.append((skill, operation["op"]))
                # Применение без забора ревизии не бывает: предпросмотр и
                # применение связывает `ifRev`.
                if arguments.get("dryRun") is False:
                    self.assertIn("ifRev", arguments)

        names = {op for _skill, op in written}
        self.assertIn("object.create", names)
        self.assertTrue(
            names >= {"props.set", "attribute.add"},
            f"создание настраивает объект теми же операциями: {sorted(names)}",
        )
        self.assertTrue(
            names >= {"attribute.set", "attribute.remove", "predefinedItem.add"},
            f"правка адресует элемент коллекции: {sorted(names)}",
        )
        self.assertIn("relation.replace", names)

        # Каждое имя операции примера должно быть реализованным именем реестра,
        # иначе пример учит вызову, который отвечает отказом.
        implemented = implemented_apply_operations(self.repo_root())
        for skill, op in written:
            with self.subTest(skill=skill, op=op):
                self.assertIn(op, implemented)

        # Источник подписки остаётся закрытым объединением целей.
        for skill in ("meta-add", "meta-edit"):
            for call in calls[skill]:
                for operation in call["params"]["arguments"]["ops"]:
                    values = operation["args"].get("values", {})
                    if values.get("relation") != "source":
                        continue
                    self.assertEqual(operation["op"], "relation.replace")
                    for target in values["targets"]:
                        self.assertIn(
                            target.get("kind"),
                            {
                                "object",
                                "manager",
                                "recordSet",
                                "definedType",
                                "family",
                            },
                        )

        self.assertIn("wire-массив набора", documents["meta-edit"])
        self.assertIn(
            "порядок его членов семантически незначим", documents["meta-edit"]
        )
        self.assertIn("exact-byte no-op", documents["meta-edit"])
        self.assertNotIn("upsert-predefined", documents["meta-edit"])

        # Названный пробел важнее гладкой прозы: читателя у предопределённых
        # элементов на канонической поверхности нет, и оба скилла это говорят.
        for skill in ("meta-info", "meta-edit"):
            with self.subTest(skill=skill, gap="predefined items have no reader"):
                self.assertIn("предопределённых элементов", documents[skill])

        for skill in ("meta-add", "meta-edit"):
            with self.subTest(skill=skill, contract="preview effects"):
                text = documents[skill]
                self.assertIn("`effects`", text)
                self.assertIn("полный XML", text)

    def test_role_edit_skill_uses_only_the_logical_typed_contract(self) -> None:
        text = (self.skill_root() / "role-edit" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        calls = [
            json.loads(block)
            for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
            if '"method": "tools/call"' in block
        ]

        self.assertTrue(calls)
        previews = 0
        for call in calls:
            params = call["params"]
            self.assertEqual(params["name"], "unica.apply")
            arguments = params["arguments"]
            # Роль называет адрес; ни набора, ни пути в аргументах нет.
            self.assertRegex(arguments["at"], r"^[^:]+:Role\.[^.]+$")
            self.assertNotIn("sourceSet", arguments)
            self.assertNotIn("metadataPath", arguments)
            if arguments.get("dryRun") is True:
                previews += 1
                self.assertNotIn("ifRev", arguments)
            else:
                self.assertTrue(arguments["ifRev"])
            self.assertTrue(arguments["ops"])
            for operation in arguments["ops"]:
                self.assertEqual(operation["op"], "right.set")
                values = operation["args"]["values"]
                self.assertIn("object", values)
                self.assertIn("right", values)
                # Одно из двух обязано быть: право без значения и без
                # ограничения ничего не говорит.
                self.assertTrue({"value", "rls"} & set(values))
        self.assertTrue(previews, "скилл обязан показать предпросмотр")

        encoded = json.dumps(calls, ensure_ascii=False)
        for legacy in ("RightsPath", "objectName", "setRight", "ObjectName"):
            with self.subTest(legacy=legacy):
                self.assertNotIn(f'"{legacy}"', encoded)
        # Что читать в ответе. Словарь v0.12 (`structuredContent.data`,
        # `metadataPath`, `operationIndex`, `validation`) ушёл вместе с
        # инструментом; канонический `apply` отвечает изменениями, эффектами
        # по порядку операций и забором ревизии.
        for token in ("changed", "effects", "ifRev", "dryRun", "диагностик"):
            with self.subTest(result_token=token):
                self.assertIn(token, text)

    def test_prompt_visible_meta_routes_have_no_retired_contract_grammar(self) -> None:
        prompt_documents = list(self.skill_root().glob("meta-*/**/*.md")) + [
            self.repo_root() / "README.md",
            self.repo_root() / "CLAUDE.md",
            self.reference_root() / "platform" / "metadata-conventions.md",
            self.skill_root() / "cf-edit" / "SKILL.md",
            self.skill_root() / "cf-edit" / "reference.md",
        ]
        retired_routes = re.compile(
            r"(?:/unica:|/)(?:meta-compile|meta-validate|meta-profile)\b|"
            r"unica\.meta\.(?:compile|validate|profile)\b"
        )
        offenders = []
        for path in prompt_documents:
            text = path.read_text(encoding="utf-8")
            if retired_routes.search(text):
                offenders.append(path.relative_to(self.repo_root()).as_posix())
        self.assertEqual(offenders, [])

    def test_scenario_skills_cover_requested_unica_workflows(self) -> None:
        for skill, tool_names in SCENARIO_SKILLS.items():
            with self.subTest(skill=skill):
                path = self.skill_root() / skill / "SKILL.md"
                self.assertTrue(path.is_file())
                text = path.read_text(encoding="utf-8")
                self.assertIn(f"name: {skill}", text)
                self.assertRegex(text, r"(?m)^description:\s+")
                self.assertIn("## MCP routing", text)
                self.assertIn("MCP `unica`", text)
                for tool_name in tool_names:
                    self.assertIn(tool_name, text)
                for token in SCENARIO_REQUIRED_TOKENS.get(skill, []):
                    self.assertIn(token, text)

    def test_skill_guidance_never_reintroduces_removed_code_grep_tool(self) -> None:
        offenders = [
            path.relative_to(self.repo_root()).as_posix()
            for path in sorted(self.skill_root().glob("*/SKILL.md"))
            if "unica.code.grep" in path.read_text(encoding="utf-8")
        ]

        self.assertEqual(offenders, [])


    def test_code_diagnostics_routes_providers_internally(self) -> None:
        text = (self.skill_root() / "code-diagnostics" / "SKILL.md").read_text(
            encoding="utf-8"
        )

        # Валидатор следует из вида узла, и выбрать его вызывающему нечем.
        self.assertIn("Валидаторы следуют из вида узла", text)
        self.assertNotIn('providers: ["bsl-analyzer"]', text)
        # Чистый ответ и неотработавший провайдер обязаны быть различимы:
        # пустой список находок при незавершённом прогоне выглядел бы как
        # «проверено и чисто».
        self.assertIn("provider_unavailable", text)
        self.assertIn("inline/range disable markers", text)
        self.assertIn("suppression-комментарии", text)

    def test_code_diagnostics_examples_call_the_canonical_check(self) -> None:
        """Пример — это маршрут, по которому пойдёт модель.

        Пример на снятое имя учит звать то, чего на проводе нет: любое имя
        v0.12 отвечает `-32602`. Проверка требует, чтобы примеры скилла звали
        `unica.check` одним логическим адресом и ничем больше.
        """
        text = (self.skill_root() / "code-diagnostics" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        calls = []
        for block in re.findall(r"```(?:json|jsonc)\n(.*?)\n```", text, flags=re.S):
            try:
                payload = json.loads(block)
            except json.JSONDecodeError:
                continue
            if isinstance(payload, dict):
                calls.append(payload.get("params", {}))

        checks = [call for call in calls if call.get("name") == "unica.check"]
        self.assertTrue(checks, "скилл диагностик обязан показать вызов check")
        for call in checks:
            arguments = call.get("arguments", {})
            self.assertEqual(set(arguments), {"at"})
            self.assertRegex(arguments["at"], r"^[^:]+:.+")

        for call in calls:
            self.assertNotEqual(call.get("name"), "unica.code.diagnostics")


    def test_platform_evidence_is_not_routed_to_standards_tools(self) -> None:
        docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        unsafe_routes = find_unsafe_platform_evidence_routes(
            [
                (
                    str(doc_path.relative_to(self.repo_root())),
                    doc_path.read_text(encoding="utf-8"),
                )
                for doc_path in docs
            ]
        )

        self.assertEqual(
            unsafe_routes,
            [],
            "standards tools must not be presented as platform evidence:\n"
            + "\n".join(unsafe_routes),
        )

    def test_route_linter_checks_adjacent_markdown_items_independently(self) -> None:
        unsafe_routes = find_unsafe_platform_evidence_routes(
            [
                (
                    "masking-fixture.md",
                    "- Use `unica.standards.search` for platform API rules.\n"
                    "- Use `unica.standards.search` only for a "
                    "`development-standard`, not platform evidence.\n",
                )
            ]
        )

        self.assertEqual(len(unsafe_routes), 1)
        self.assertIn("platform API rules", unsafe_routes[0])
        self.assertNotIn("development-standard", unsafe_routes[0])

    def test_route_linter_checks_claims_within_one_markdown_item(self) -> None:
        unsafe_routes = find_unsafe_platform_evidence_routes(
            [
                (
                    "same-item-fixture.md",
                    "- `unica.standards.search` is a `development-standard`, "
                    "not platform evidence. Use `unica.standards.search` for "
                    "platform API rules.\n",
                )
            ]
        )

        self.assertEqual(len(unsafe_routes), 1)
        self.assertIn("platform API rules", unsafe_routes[0])
        self.assertNotIn("development-standard", unsafe_routes[0])


    def test_skills_and_references_do_not_instruct_direct_rlm_mcp_calls(self) -> None:
        forbidden = ["rlm_index", "rlm_start", "rlm_execute", "rlm_end"]
        docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        for doc in docs:
            text = doc.read_text(encoding="utf-8")
            for token in forbidden:
                with self.subTest(path=doc.relative_to(self.repo_root()), token=token):
                    self.assertNotIn(token, text)

    def test_runtime_is_tool_native_and_v8_runner_skill_is_not_shipped(self) -> None:
        skill_dir = self.skill_root() / "v8-runner"
        self.assertFalse((skill_dir / "SKILL.md").exists())
        self.assertFalse(any(path.is_file() for path in skill_dir.rglob("*")))
        for skill in REPLACED_RUNTIME_SKILLS:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill).exists())



    def test_runtime_json_guard_accepts_case_insensitive_fence_labels(self) -> None:
        example = """```JSON
{
  "method": "tools/call",
  "params": {
    "name": "unica.run",
    "arguments": {"operation": "build", "dryRun": false}
  }
}
```"""

        self.assertEqual(
            runtime_execute_json_examples(example),
            [
                {
                    "method": "tools/call",
                    "params": {
                        "name": "unica.run",
                        "arguments": {"operation": "build", "dryRun": False},
                    },
                }
            ],
        )

    def test_runtime_json_guard_checks_jsonc_runtime_blocks_before_language_filtering(
        self,
    ) -> None:
        example = """```jsonc
{
  "method": "tools/call",
  "params": {
    "name": "unica.run",
    "arguments": {"operation": "build", "dryRun": false}
  }
}
```"""

        payloads = runtime_execute_json_examples(example)

        self.assertEqual(len(payloads), 1)
        self.assertIs(payloads[0]["params"]["arguments"]["dryRun"], False)

    def test_runtime_json_guard_decodes_tool_name_before_classification(self) -> None:
        example = r"""```json
{
  "method": "tools/call",
  "params": {
    "name": "unica\u002erun",
    "arguments": {"operation": "build", "dryRun": false}
  }
}
```"""

        payloads = runtime_execute_json_examples(example)

        self.assertEqual(len(payloads), 1)
        self.assertEqual(payloads[0]["params"]["name"], "unica.run")
        self.assertIs(payloads[0]["params"]["arguments"]["dryRun"], False)

    def test_runtime_json_guard_keeps_malformed_block_boundary(self) -> None:
        self.assertEqual(
            runtime_execute_json_examples("```json\n{not runtime JSON}\n```"),
            [],
        )
        self.assertEqual(
            runtime_execute_json_examples(
                r'''```json
{"note":"unica\\u002erun",
```'''
            ),
            [],
        )

        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                '```json\n{"name":"unica.run",\n```'
            )

    def test_runtime_json_guard_rejects_malformed_escaped_tool_name(self) -> None:
        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                r'''```json
{"params":{"name":"unica\u002erun","arguments":{"dryRun":false}},
```'''
            )

        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                r'''```json
{"params":{"name":"unica\u002erun
```'''
            )

    def test_runtime_json_guard_handles_commonmark_fences_and_batches(self) -> None:
        examples = r'''~~~JSON
[{"method":"tools/call","params":{"name":"unica\u002erun","arguments":{"dryRun":false}}}]
~~~
``` json
{"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
```
````json
{"note":"```","method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
````'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 3)
        self.assertTrue(
            all(
                payload["params"]["name"] == "unica.run"
                and payload["params"]["arguments"]["dryRun"] is False
                for payload in payloads
            )
        )

    def test_runtime_json_guard_handles_commonmark_containers_and_info(self) -> None:
        examples = '''> ```json
> {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
> ```
- example:

    ```json
    {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
    ```
```json title=request
{"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
```'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 3)
        self.assertTrue(
            all(
                payload["params"]["arguments"]["dryRun"] is False
                for payload in payloads
            )
        )

    def test_runtime_json_guard_handles_fence_after_list_marker(self) -> None:
        examples = '''- ```json
  {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
  ```
> - ```json
>   {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
>   ```'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 2)

    def test_runtime_json_guard_handles_nested_indent_and_info_entities(self) -> None:
        examples = '''123. outer
     - ```json
       {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
       ```
123. outer
     > ```json
     > {"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
     > ```
```j&#x73;on
{"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}}
```'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 3)

    def test_runtime_guidance_document_detects_decoded_tool_name(self) -> None:
        example = r'''```json
{"method":"tools/call","params":{"name":"unica\u002erun","arguments":{"dryRun":false}}}
```'''

        is_runtime_document, payloads = runtime_guidance_document(example)

        self.assertTrue(is_runtime_document)
        self.assertEqual(len(payloads), 1)

    def test_runtime_guidance_document_rejects_indented_applied_runtime_example(
        self,
    ) -> None:
        example = """
    {
      "method": "tools/call",
      "params": {
        "name": "unica.run",
        "arguments": {"operation": "build", "dryRun": false}
      }
    }
"""

        with self.assertRaisesRegex(ValueError, "indented runtime JSON example"):
            runtime_guidance_document(example)

    def test_indented_runtime_example_needs_if_rev_inside_arguments(self) -> None:
        outside = (
            '    {"ifRev": "note", "params": {"name": "unica.run", '
            '"arguments": {"op": "upload", "dryRun": false}}}\n'
        )
        with self.assertRaisesRegex(ValueError, "applies without ifRev"):
            reject_indented_applied_runtime_examples(outside)
        inside = (
            '    {"params": {"name": "unica.run", "arguments": '
            '{"op": "upload", "dryRun": false, "ifRev": "unica-cf-import-sha256-v1:abc"}}}\n'
        )
        reject_indented_applied_runtime_examples(inside)

    def test_runtime_guidance_collection_skips_parse_failures_without_stale_payloads(
        self,
    ) -> None:
        good = Path("good.md")
        bad = Path("bad.md")

        runtime_docs, runtime_examples, parse_failures = collect_runtime_guidance(
            [
                (
                    good,
                    '''```json
{"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":true}}}
```''',
                ),
                (
                    bad,
                    '''```json
{"method":"tools/call","params":{"name":"unica.run","arguments":{"dryRun":false}}
```''',
                ),
            ]
        )

        self.assertEqual([doc for doc, _text in runtime_docs], [good])
        self.assertEqual([doc for doc, _arguments in runtime_examples], [good])
        self.assertEqual([doc for doc, _error in parse_failures], [bad])

    def test_all_runtime_execute_skill_guidance_states_the_applied_contract(self) -> None:
        shipped_docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        runtime_docs, runtime_examples, parse_failures = collect_runtime_guidance(
            [
                (doc, doc.read_text(encoding="utf-8"))
                for doc in sorted(shipped_docs)
            ]
        )
        if parse_failures:
            for doc, error in parse_failures:
                with self.subTest(path=doc.relative_to(self.repo_root())):
                    self.fail(error)
            return

        required_runtime_references = {
            self.reference_root() / relative_path
            for relative_path in (
                "use-cases/workspace-runtime.md",
                "use-cases/autonomous-server-debug.md",
                "use-cases/reports-printing.md",
                "use-cases/integrations.md",
                "use-cases/code-quality-review.md",
                "use-cases/extensions-cfe.md",
                "use-cases/rights-access.md",
                "tooling/v8project.md",
                "tooling/runtime-build.md",
            )
        }
        required_runtime_skills = {
            self.skill_root() / relative_path
            for relative_path in (
                "db-auth-check/SKILL.md",
                "epf-bsp-init/SKILL.md",
                "epf-init/SKILL.md",
                "erf-init/SKILL.md",
            )
        }
        self.assertTrue(
            required_runtime_references.issubset(
                {doc for doc, _text in runtime_docs}
            )
        )
        self.assertTrue(
            required_runtime_skills.issubset({doc for doc, _text in runtime_docs})
        )
        self.assertGreaterEqual(len(runtime_docs), MIN_RUNTIME_GUIDANCE_DOCS)
        self.assertGreaterEqual(
            len(runtime_examples), MIN_RUNTIME_EXECUTE_EXAMPLES
        )
        for doc, arguments in runtime_examples:
            with self.subTest(
                path=doc.relative_to(self.repo_root()),
                operation=arguments.get("op"),
            ):
                # An apply carries the revision its preview returned.
                if arguments.get("op") != "launch":
                    self.assertIn("dryRun", arguments)
                if arguments.get("dryRun") is False:
                    self.assertIn("ifRev", arguments)

        # The dictionary owns the contract: guidance points at `unica.run` and
        # does not restate per-operation risk codes or dryRun/ifRev rules.
        contract_tokens = ("`unica.run`",)
        forbidden_applied_claims = (
            r"запускай следующую необходимую операцию",
            r"`operation=init` допустима",
            r"подготовить external source-set .* через `v8-runner`",
            r"Проверь `Администратор` с пустым паролем",
            r"допускается только два предположения",
            r"When credentials are absent, try only",
            r"Authentication failure without credentials allows only",
            r"Use `dump` to bring database changes into Git-visible files",
        )
        for doc, text in runtime_docs:
            with self.subTest(path=doc.relative_to(self.repo_root())):
                for token in contract_tokens:
                    self.assertIn(token, text)
                for claim in forbidden_applied_claims:
                    self.assertNotRegex(text, claim)

    def test_shipped_guidance_names_no_retired_reading_or_writing_tool(self) -> None:
        """C-1: `unica.code.*` and `unica.meta.*` route to search/view/check/apply."""
        shipped_docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        offenders = {}
        for doc in sorted(shipped_docs):
            hits = sorted(
                {
                    match.group(0)
                    for match in RETIRED_READ_WRITE_NAMES.finditer(
                        doc.read_text(encoding="utf-8")
                    )
                }
            )
            if hits:
                offenders[doc.relative_to(self.repo_root()).as_posix()] = hits
        self.assertEqual(offenders, {})

    def test_shipped_guidance_names_no_retired_runtime_tool(self) -> None:
        """#702: the wire has no `unica.runtime.execute`, so no skill may teach it."""
        shipped_docs = (
            list(self.skill_root().glob("**/*.md"))
            + list(self.reference_root().glob("**/*.md"))
            + [self.repo_root() / "plugins" / "unica" / "README.md"]
        )
        offenders = {}
        for doc in sorted(shipped_docs):
            hits = sorted(
                {
                    match.group(0)
                    for match in RETIRED_RUNTIME_NAMES.finditer(
                        doc.read_text(encoding="utf-8")
                    )
                }
            )
            if hits:
                offenders[doc.relative_to(self.repo_root()).as_posix()] = hits
        self.assertEqual(offenders, {})








    def test_references_are_structured_by_unica_use_cases(self) -> None:
        reference_root = self.reference_root()
        self.assertFalse((reference_root / "cc-1c-skills").exists())
        self.assertFalse((reference_root / "ai-rules-1c").exists())

        required_paths = [
            "README.md",
            "use-cases/workspace-runtime.md",
            "use-cases/metadata-modeling.md",
            "use-cases/forms-ui.md",
            "use-cases/reports-printing.md",
            "use-cases/extensions-cfe.md",
            "use-cases/rights-access.md",
            "use-cases/autonomous-server-debug.md",
            "use-cases/code-quality-review.md",
            "use-cases/integrations.md",
            "specs/README.md",
            "platform/development-standards.md",
            "platform/platform-solutions.md",
            "platform/runtime-diagnostics.md",
            "platform/db-performance.md",
            "platform/integration-contracts.md",
            "platform/platform-mechanics.md",
            "tooling/v8project.md",
            "tooling/runtime-build.md",
        ]
        for relative_path in required_paths:
            with self.subTest(path=relative_path):
                path = reference_root / relative_path
                self.assertTrue(path.is_file())
                text = path.read_text(encoding="utf-8")
                if relative_path.startswith("use-cases/"):
                    self.assertIn("## When to use", text)
                    self.assertIn("## Primary path", text)

    def test_web_publish_skill_surface_is_replaced_by_autonomous_server(self) -> None:
        self.assertTrue((self.skill_root() / "autonomous-server" / "SKILL.md").is_file())
        for skill in ["web-publish", "web-info", "web-stop", "web-unpublish"]:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill).exists())

        docs = [
            self.repo_root() / "plugins" / "unica" / "README.md",
            self.reference_root() / "README.md",
            *self.skill_root().glob("*/SKILL.md"),
            *self.reference_root().glob("use-cases/*.md"),
        ]
        forbidden = [
            "web-publish",
            "web-info",
            "web-stop",
            "web-unpublish",
            "web-publication-testing.md",
        ]
        for doc in docs:
            text = doc.read_text(encoding="utf-8")
            for token in forbidden:
                with self.subTest(path=doc.relative_to(self.repo_root()), token=token):
                    self.assertNotIn(token, text)

    def test_unica_reference_models_retain_reviewed_runtime_portability_fixes(self) -> None:
        dcs_scripts = [
            self.unica_reference_models_root()
            / "dcs-edit"
            / "scripts"
            / "dcs-edit.py",
        ]
        for path in dcs_scripts:
            with self.subTest(path=path.relative_to(self.repo_root())):
                text = path.read_text(encoding="utf-8")
                self.assertIn("dcs-edit v1.28", text)
                self.assertIn("expr_start = esc_xml", text)
                self.assertIn("expr_end = esc_xml", text)
                self.assertNotRegex(text, r"<expression>\{esc_xml\('&' \+ param_name")

        subsystem_compile = (
            self.unica_reference_models_root()
            / "subsystem-compile"
            / "scripts"
            / "subsystem-compile.py"
        ).read_text(encoding="utf-8")
        self.assertIn("subsystem-compile v1.8", subsystem_compile)
        self.assertIn("import subprocess", subsystem_compile)
        self.assertIn("subsystem-validate.py", subsystem_compile)
        self.assertIn("subprocess.run([sys.executable, validate_script, \"-SubsystemPath\", target_xml])", subsystem_compile)
        self.assertNotIn("powershell.exe", subsystem_compile)
        self.assertNotIn("subsystem-validate.ps1", subsystem_compile)

    def test_dcs_skills_track_upstream_dsl_features_through_unica_boundary(self) -> None:
        dcs_compile = (self.skill_root() / "dcs-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        dcs_edit = (self.skill_root() / "dcs-edit" / "SKILL.md").read_text(encoding="utf-8")
        dcs_dsl = (self.reference_root() / "specs" / "dcs-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        dcs_spec = (self.reference_root() / "specs" / "1c-dcs-spec.md").read_text(
            encoding="utf-8"
        )

        for text in [dcs_compile, dcs_edit]:
            self.assertIn("MCP `unica`", text)
            self.assertNotIn("CLAUDE_SKILL_DIR", text)
            self.assertNotIn("powershell.exe", text)
            self.assertNotIn(".ps1", text)
            self.assertNotIn(".py", text)

        for token in [
            "TypeSet",
            "balanceGroupName",
            "orderExpression",
            "valueListAllowed",
            "availableValues",
            "dataSetLinks",
            "additionalProperties",
            "parameterListAllowed",
            "startExpression",
            "linkConditionExpression",
            "viewMode",
            "itemsViewMode",
            "use: false",
            "placement",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, dcs_dsl)

        self.assertIn("Значение-список", dcs_spec)
        self.assertIn("valueListAllowed", dcs_spec)
        # `Raw` existed because pagination mangled the query; the canonical
        # `Query` projection carries the exact text always, so the promise
        # moved into the writer that consumes it.
        self.assertNotIn('"Raw": true', dcs_edit)
        self.assertIn("сырой текст запроса целиком", dcs_edit)
        # Точечная правка запроса осталась возможностью скилла, но зовётся
        # канонической операцией: прежнее имя DSL вызовом больше не выглядит.
        self.assertIn("query.patch", dcs_edit)
        self.assertNotIn("используй `patch-query`", dcs_edit)
        self.assertIn("@once", dcs_edit)
        self.assertIn("availableValue=", dcs_edit)
        self.assertIn("value=", dcs_edit)

    def test_form_skills_track_upstream_dsl_features_through_unica_boundary(self) -> None:
        form_compile = (self.skill_root() / "form-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        form_edit = (self.skill_root() / "form-edit" / "SKILL.md").read_text(encoding="utf-8")
        form_dsl = (self.reference_root() / "specs" / "form-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        form_patterns = (self.reference_root() / "specs" / "form-patterns.md").read_text(
            encoding="utf-8"
        )

        for text in [form_compile, form_edit]:
            self.assertIn("MCP `unica`", text)
            self.assertNotIn("CLAUDE_SKILL_DIR", text)
            self.assertNotIn("powershell.exe", text)
            self.assertNotIn(".ps1", text)
            self.assertNotIn(".py", text)

        for token in [
            "mobileCommandBarContent",
            "reportResult",
            "reportFormType",
            "choiceParameters",
            "choiceParameterLinks",
            "availableTypes",
            "extendedTooltip",
            "commandBar",
            "contextMenu",
            "roles",
            "CommandInterface",
            "NavigationPanel",
            "GanttChart",
            "chart",
            "dynamicDataRead",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, form_dsl)

        self.assertIn("Связанные действия командной панели", form_patterns)
        self.assertIn("mobileCommandBarContent", form_compile)
        self.assertIn("choiceParameters", form_compile)
        self.assertIn("availableTypes", form_compile)
        self.assertIn("unica.view", form_edit)
        self.assertIn("`unica.check`", form_edit)
        self.assertNotIn("unica.form.validate", form_edit)

    def test_meta_info_tracks_upstream_type_presentation_through_unica_boundary(self) -> None:
        meta_info = (self.skill_root() / "meta-info" / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("MCP `unica`", meta_info)
        self.assertIn("unica.view", meta_info)
        self.assertIn("Представление типа", meta_info)
        self.assertIn("Представление объекта", meta_info)
        self.assertNotIn("CLAUDE_SKILL_DIR", meta_info)
        self.assertNotIn("powershell.exe", meta_info)
        self.assertNotIn(".ps1", meta_info)
        self.assertNotIn(".py", meta_info)

    def test_meta_add_routes_minimal_creation_without_erasing_upstream_facts(self) -> None:
        meta_add = (self.skill_root() / "meta-add" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        format_facts = (
            self.reference_root() / "specs" / "1c-config-objects-spec.md"
        ).read_text(encoding="utf-8")

        self.assertIn("MCP `unica`", meta_add)
        self.assertIn("unica.apply", meta_add)
        self.assertNotIn("unica.meta.compile", meta_add)
        self.assertNotIn('"JsonPath"', meta_add)
        self.assertIn("ChoiceHistoryOnInput", format_facts)
        self.assertNotIn("CLAUDE_SKILL_DIR", meta_add)
        self.assertNotIn("powershell.exe", meta_add)
        self.assertNotIn(".ps1", meta_add)
        self.assertNotIn(".py", meta_add)

    def test_top_level_skills_never_route_to_retired_meta_tools(self) -> None:
        retired = {
            "unica.meta.compile",
            "unica.meta.profile",
            "unica.meta.validate",
        }
        offenders = {
            path.relative_to(self.repo_root()).as_posix(): sorted(
                name for name in retired if name in path.read_text(encoding="utf-8")
            )
            for path in sorted(self.skill_root().glob("*/SKILL.md"))
            if any(name in path.read_text(encoding="utf-8") for name in retired)
        }
        self.assertEqual(offenders, {})

    def test_support_state_reporting_is_documented_for_info_skills(self) -> None:
        for skill in (
            "meta-info",
            "mxl-info",
        ):
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertIn("Поддержка", text)
                self.assertIn("ParentConfigurations.bin", text)
                self.assertIn("unica.", text)
                self.assertNotIn("support-edit.py", text)
                self.assertNotIn("ParentConfigurations.bin` raw", text)

        release_support = (self.skill_root() / "release-support" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("support-state", release_support)
        self.assertIn("unica.view", release_support)
        self.assertIn("ParentConfigurations.bin", release_support)



    def test_references_do_not_contain_stale_upstream_instructions(self) -> None:
        forbidden_patterns = [
            r"references/(cc-1c-skills|ai-rules-1c)",
            r"\bClaude\b",
            r"\bclaude\b",
            r"Anthropic",
            r"\.claude",
            r"/db-(?!performance\.md\b)",
            r"/epf-(init|build|dump|validate)",
            r"/erf-(init|build|dump|validate)",
            r"1c-code-metadata-mcp",
            r"1c-metadata-manage",
            r"deploy_and_test",
            r'"mode"\s*:\s*"update"',
        ]
        scanned_roots = [self.reference_root(), self.skill_root()]
        for root in scanned_roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                relative_path = path.relative_to(self.repo_root())
                # A pinned source inventory is evidence, not prompt routing.
                # Only the exact XDTO specification owns that exception; marker
                # reuse in any other prompt-visible document is itself a failure.
                if relative_path == XDTO_DONOR_EVIDENCE_PATH:
                    self.assertEqual(text.count(XDTO_DONOR_EVIDENCE_START), 1)
                    self.assertEqual(text.count(XDTO_DONOR_EVIDENCE_END), 1)
                else:
                    self.assertNotIn(XDTO_DONOR_EVIDENCE_START, text)
                    self.assertNotIn(XDTO_DONOR_EVIDENCE_END, text)
                try:
                    text = stale_route_guard_text(relative_path, text)
                except ValueError as error:
                    self.fail(str(error))
                for pattern in forbidden_patterns:
                    with self.subTest(path=relative_path, pattern=pattern):
                        self.assertIsNone(re.search(pattern, text))

    def test_xdto_donor_evidence_markers_cannot_mask_other_documents(self) -> None:
        foreign_document = """<!-- xdto-donor-evidence:start -->
Use `.claude/commands/xdto.md` as the execution route.
<!-- xdto-donor-evidence:end -->
"""

        guarded = stale_route_guard_text(
            Path("plugins/unica/skills/foreign/SKILL.md"),
            foreign_document,
        )

        self.assertIn(".claude", guarded)




    def test_code_patch_skill_uses_only_logical_configuration_and_extension_targets(
        self,
    ) -> None:
        path = self.skill_root() / "code-patch" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        calls = []
        for block in re.findall(r"```json\s*(.*?)```", text, re.DOTALL):
            payload = json.loads(block)
            params = payload.get("params", {})
            if params.get("name") == "unica.apply":
                calls.append(params.get("arguments", {}))

        self.assertGreaterEqual(len(calls), 2)
        previews = 0
        for arguments in calls:
            with self.subTest(arguments=arguments):
                # Цель называет адрес, а не пара селекторов и уж точно не путь.
                self.assertRegex(arguments["at"], r"^[^:]+:.+")
                self.assertNotIn("path", arguments)
                self.assertNotIn("sourceDir", arguments)
                self.assertNotIn("sourceSet", arguments)
                self.assertNotIn("metadataPath", arguments)
                for operation in arguments["ops"]:
                    self.assertIn(operation["op"], {"code.insert", "code.replace"})
                    self.assertRegex(operation["args"]["at"], r"^[^:]+:.+")
                    self.assertTrue(operation["args"]["text"])
                if arguments.get("dryRun") is True:
                    previews += 1
                    self.assertNotIn("ifRev", arguments)
                else:
                    # Применение связано с предпросмотром забором ревизии.
                    self.assertTrue(arguments["ifRev"])
        self.assertTrue(previews, "скилл обязан показать предпросмотр")

    def test_code_patch_prompt_metadata_covers_every_public_operation(self) -> None:
        """Prompt metadata names the published operations and only those.

        `description` and `argument-hint` are what a host shows before the body
        is ever read, so an operation missing there is invisible at the moment
        of choosing the skill, and a retired one advertises a call that now
        fails. The published enum is the source of truth; this keeps the two
        from drifting apart in either direction.
        """
        path = self.skill_root() / "code-patch" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        published = ("insert", "replace")
        retired = ("initialize",)

        fields = prompt_frontmatter(text)
        self.assertEqual(
            set(fields),
            {"description", "argument-hint"},
            msg=f"invalid prompt frontmatter:\n{prompt_frontmatter_body(text)}",
        )
        for field, value in fields.items():
            for operation in published:
                with self.subTest(field=field, operation=operation):
                    self.assertRegex(value, rf"\b{operation}\b")
            for operation in retired:
                with self.subTest(field=field, retired=operation):
                    self.assertNotRegex(value, rf"\b{operation}\b")


    def test_xdto_skill_uses_one_confirmed_info_preview_apply_mcp_flow(self) -> None:
        path = self.skill_root() / "xdto" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        blocks = list(re.finditer(r"```json\s*(.*?)```", text, re.DOTALL))
        calls = [json.loads(block.group(1)) for block in blocks]

        self.assertEqual(len(calls), 4)
        self.assertEqual(
            [call.get("method") for call in calls],
            ["tools/call", "tools/call", "tools/call", "tools/call"],
        )
        params = [call["params"] for call in calls]
        self.assertEqual(
            [item["name"] for item in params],
            ["unica.view", "unica.view", "unica.apply", "unica.apply"],
        )
        self.assertEqual(
            {item["name"] for item in params},
            {"unica.view", "unica.apply"},
        )

        preview = dict(params[2]["arguments"])
        apply = dict(params[3]["arguments"])
        self.assertIs(preview.pop("dryRun"), True)
        self.assertIs(apply.pop("dryRun"), False)
        self.assertEqual(preview, apply)

        # Reader and writer address the same node with one canonical `at`.
        self.assertEqual(
            params[0]["arguments"].get("at"),
            "main:XDTOPackage.EnterpriseData_1_17_3",
        )
        self.assertEqual(
            params[1]["arguments"].get("at"),
            "main:XDTOPackage.EnterpriseData_1_17_3.Type.ЛюбаяСсылка",
        )
        for item in params:
            with self.subTest(tool=item["name"]):
                arguments = item["arguments"]
                self.assertNotIn("path", arguments)
                self.assertNotIn("Package.bin", json.dumps(arguments, ensure_ascii=False))
        for item in params[2:]:
            with self.subTest(tool=item["name"], role="writer"):
                self.assertTrue(
                    item["arguments"]["at"].startswith(
                        "main:XDTOPackage.EnterpriseData_1_17_3."
                    ),
                    item["arguments"]["at"],
                )
        self.assertEqual(len(preview["ops"]), 1)
        self.assertEqual(preview["ops"][0]["op"], "property.add")
        self.assertEqual(
            preview["ops"][0]["args"]["values"]["property"]["type"],
            "tns:Документ_ЗаказКлиента",
        )
        # A coherent change travels as one ordered transactional `ops` array,
        # and a rejected element is named by its position in it.
        for forbidden in (
            "unica.xdto.validate",
            "xdto-compile",
            "xdto-decompile",
            "xdto-validate",
            "scripts/",
            "powershell.exe",
            ".ps1",
            ".py",
            "```bash",
            "```shell",
        ):
            with self.subTest(forbidden=forbidden):
                self.assertNotIn(forbidden, text)






    def test_skills_and_references_do_not_expose_restricted_research_sources(self) -> None:
        forbidden_patterns = [
            r"docs/its",
            r"\.pdf\b",
            r"Документация\.pdf",
            r"Методическая поддержка",
            r":: 1С:Предприятие",
        ]
        scanned_roots = [self.reference_root(), self.skill_root()]
        for root in scanned_roots:
            for path in root.rglob("*.md"):
                text = path.read_text(encoding="utf-8")
                for pattern in forbidden_patterns:
                    with self.subTest(path=path.relative_to(self.repo_root()), pattern=pattern):
                        self.assertIsNone(re.search(pattern, text, flags=re.I))

    def test_documented_paths_resolve_from_the_document_that_carries_them(self) -> None:
        """A documented link is only unambiguous when it is document-relative.

        The reader of a skill or reference doc has that doc's directory as its
        only stable anchor: the repository root is absent once the plugin is
        packaged, and the plugin root is not knowable from the prose. So every
        link resolves from its own document, and nothing resolves from a root.
        """
        roots = [
            self.repo_root() / "README.md",
            self.repo_root() / "plugins" / "unica" / "README.md",
            *self.skill_root().glob("*/**/*.md"),
            *self.reference_root().rglob("*.md"),
        ]
        seen = 0
        for doc in sorted(roots):
            text = doc.read_text(encoding="utf-8")
            for match in document_links(text):
                seen += 1
                with self.subTest(doc=doc.relative_to(self.repo_root()), reference=match):
                    self.assertTrue((doc.parent / match).is_file())

        self.assertGreater(seen, 0)

    def test_skills_do_not_use_model_specific_assistant_names(self) -> None:
        forbidden = ["Claude", "claude", "Anthropic", ".claude", "CLAUDE.md"]
        for skill_doc in self.skill_root().glob("*/**/*.md"):
            with self.subTest(path=skill_doc.relative_to(self.skill_root())):
                text = skill_doc.read_text(encoding="utf-8")
                for token in forbidden:
                    self.assertNotIn(token, text)


    def test_migrated_skills_do_not_ship_skill_local_operation_scripts(self) -> None:
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                self.assertFalse((self.skill_root() / skill / "scripts").exists())

    def test_unica_reference_models_are_test_only_fixtures(self) -> None:
        models_root = self.unica_reference_models_root()
        modelled_skills = {
            path.parent.parent.name for path in models_root.glob("*/scripts/*.py")
        }
        self.assertEqual(
            modelled_skills,
            set(IN_SCOPE_TOOLS)
            - {"epf-init", "erf-init", "meta-add", "meta-edit", "role-edit"},
        )
        allowed_suffixes = {".json", ".md", ".ps1", ".py"}
        for path in models_root.rglob("*"):
            if path.is_file():
                with self.subTest(path=path.relative_to(models_root)):
                    self.assertNotIn("__pycache__", path.parts)
                    self.assertIn(path.suffix, allowed_suffixes)

    def test_migrated_skill_verification_sections_use_mcp_examples(self) -> None:
        slash_command = re.compile(r"(?m)^/[a-z][a-z-]+\b")
        verification_section = re.compile(r"(?ms)^## Верификация\s*\n(.*?)(?=^## |\Z)")
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                match = verification_section.search(text)
                if match is None:
                    continue
                section = match.group(1)
                self.assertIsNone(slash_command.search(section))
                self.assertNotIn("powershell.exe", section)
                self.assertNotIn(".ps1", section)
                self.assertNotIn(".py", section)
                if "```" in section:
                    self.assertIn('"method": "tools/call"', section)


    def test_migrated_skills_use_task_parameterized_mcp_examples(self) -> None:
        generic_arguments = '"arguments": {\n      "cwd": "<workspace>"\n    }'
        for skill, tool_name in IN_SCOPE_TOOLS.items():
            if skill in SKILLS_WITHOUT_A_CANONICAL_ENTRY:
                continue
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertNotIn(generic_arguments, text)
                for key in TASK_EXAMPLE_ARGUMENT_KEYS[skill]:
                    self.assertIn(f'"{key}"', text)
                mcp_blocks = [
                    block
                    for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
                    if '"method": "tools/call"' in block
                ]
                self.assertGreater(len(mcp_blocks), 0)
                if skill in SCENARIO_PRESERVING_MIN_MCP_CALLS:
                    self.assertGreaterEqual(
                        len(mcp_blocks), SCENARIO_PRESERVING_MIN_MCP_CALLS[skill]
                    )
                for token in SCENARIO_PRESERVING_TOKENS.get(skill, []):
                    self.assertIn(token, text)
                for token in SCENARIO_RETIRED_TOKENS.get(skill, []):
                    self.assertNotIn(token, text)
                for block in mcp_blocks:
                    payload = json.loads(block)
                    params = payload["params"]
                    allowed_tool_names = {
                        tool_name,
                        *ALLOWED_ADDITIONAL_MCP_TOOL_NAMES.get(skill, set()),
                    }
                    self.assertIn(params["name"], allowed_tool_names)
                    self.assertNotEqual(set(params["arguments"].keys()), {"cwd"})


class PlatformHelpRoutingTests(unittest.TestCase):
    """Скилл получил источник: отказ перестаёт быть штатным ответом."""

    def setUp(self) -> None:
        self.text = (
            REPO_ROOT / "plugins" / "unica" / "skills" / "platform-help" / "SKILL.md"
        ).read_text(encoding="utf-8")

    def test_routes_platform_questions_to_documentation_search(self) -> None:
        self.assertIn("unica.docs", self.text)
        self.assertNotIn("unica.documentation.search", self.text)




    def test_filters_platform_questions_by_source_kind(self) -> None:
        # Вопрос об API платформы задаётся с фильтром по смыслу
        # источника, а не полагается на то, что секция стандартов не помешает.
        # На канонической поверхности фильтр — скаляр `source`.
        self.assertIn('"source": "platform-help"', self.text)


    def test_confirms_answers_with_the_opened_document(self) -> None:
        # Страница открывается через unica.docs с локатором в query.
        # Примеры используют этот вход, а не снятый documentation.get.
        calls = [
            json.loads(block)
            for block in re.findall(r"```json\n(.*?)\n```", self.text, flags=re.S)
            if '"method": "tools/call"' in block
        ]
        names = {call["params"]["name"] for call in calls}
        self.assertIn("unica.docs", names)
        self.assertNotIn("unica.documentation.get", names)

    def test_routes_configuration_domain_questions_to_configuration_help(self) -> None:
        # Доменный вопрос о самой конфигурации закрывает её
        # встроенная справка, а не справка платформы. На канонической
        # поверхности этот источник пока отвечает отказом, и скилл обязан
        # назвать и сам источник, и названную причину его недоступности,
        # чтобы читатель не принял отказ за отсутствие ответа.
        self.assertIn('"configuration-documentation"', self.text)
        self.assertIn("unsupported_source", self.text)


if __name__ == "__main__":
    unittest.main()
