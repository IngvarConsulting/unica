from __future__ import annotations

import html
import json
import re
import unittest
from pathlib import Path

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
MIN_RUNTIME_GUIDANCE_DOCS = 31
# The retired v8-runner skill carried eleven duplicate examples. Remaining
# examples belong to subject skills and shared references until their v0.13
# operation is implemented and migrated to unica.run.
MIN_RUNTIME_EXECUTE_EXAMPLES = 10


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
    return "unica.runtime.execute" in decode_active_json_unicode_escapes(block)


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


def reject_indented_applied_runtime_examples(text: str) -> None:
    for block_number, block in enumerate(indented_code_blocks(text), start=1):
        if block_mentions_runtime_tool(block) and DRY_RUN_FALSE.search(block):
            raise ValueError(
                f"indented runtime JSON example #{block_number} uses dryRun false"
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
            if params.get("name") != "unica.runtime.execute":
                continue
            examples.append(candidate)
    return examples


def runtime_guidance_document(text: str) -> tuple[bool, list[dict]]:
    reject_indented_applied_runtime_examples(text)
    examples = runtime_execute_json_examples(text)
    return (
        bool(examples)
        or "unica.runtime.execute" in text
        or "v8-runner" in text,
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
                and payload.get("params", {}).get("name") == "unica.runtime.execute"
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


IN_SCOPE_TOOLS = {
    "cf-edit": "unica.cf.edit",
    "cf-info": "unica.cf.info",
    "cf-init": "unica.cf.init",
    "cf-validate": "unica.cf.validate",
    "cfe-borrow": "unica.cfe.borrow",
    "cfe-diff": "unica.cfe.diff",
    "cfe-init": "unica.cfe.init",
    "cfe-patch-method": "unica.cfe.patch_method",
    "cfe-validate": "unica.cfe.validate",
    "epf-init": "unica.epf.init",
    "erf-init": "unica.erf.init",
    "meta-add": "unica.meta.add",
    "meta-edit": "unica.meta.edit",
    "meta-info": "unica.meta.info",
    "meta-remove": "unica.meta.remove",
    "form-add": "unica.form.add",
    "form-compile": "unica.form.compile",
    "form-edit": "unica.form.edit",
    "form-info": "unica.form.info",
    "form-remove": "unica.form.remove",
    "form-validate": "unica.form.validate",
    "interface-edit": "unica.interface.edit",
    "interface-validate": "unica.interface.validate",
    "subsystem-compile": "unica.subsystem.compile",
    "subsystem-edit": "unica.subsystem.edit",
    "subsystem-info": "unica.subsystem.info",
    "subsystem-validate": "unica.subsystem.validate",
    "dcs-compile": "unica.dcs.compile",
    "dcs-edit": "unica.dcs.edit",
    "dcs-info": "unica.dcs.info",
    "dcs-validate": "unica.dcs.validate",
    "mxl-compile": "unica.mxl.compile",
    "mxl-decompile": "unica.mxl.decompile",
    "mxl-info": "unica.mxl.info",
    "mxl-validate": "unica.mxl.validate",
    "role-compile": "unica.role.compile",
    "role-edit": "unica.role.edit",
    "role-info": "unica.role.info",
    "role-validate": "unica.role.validate",
}

SCENARIO_SKILLS = {
    "api-design": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.diagnostics",
        "unica.view",
        "unica.subsystem.info",
        "unica.meta.info",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "code-search": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.outline",
        "unica.meta.info",
        "unica.view",
    ],
    "code-diagnostics": [
        "unica.code.diagnostics",
        "unica.code.search",
        "unica.standards.explain",
        "unica.standards.search",
        "unica.runtime.execute",
    ],
    "code-review": [
        "unica.code.search",
        "unica.code.definition",
        "unica.code.outline",
        "unica.code.diagnostics",
        "unica.meta.info",
        "unica.standards.explain",
        "unica.standards.search",
        "unica.view",
        "unica.runtime.execute",
    ],
    "query-optimize": [
        "unica.code.search",
        "unica.code.outline",
        "unica.dcs.info",
        "unica.meta.info",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "test-authoring": [
        "unica.code.search",
        "unica.view",
        "unica.runtime.execute",
    ],
    "platform-help": [
        "unica.code.search",
        "unica.view",
        "unica.runtime.execute",
    ],
    "bsp-patterns": [
        "unica.code.search",
        "unica.meta.info",
        "unica.form.info",
        "unica.role.info",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "integration-implement": [
        "unica.view",
        "unica.meta.info",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.code.search",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "autonomous-server": [
        "unica.view",
        "unica.runtime.execute",
        "unica.meta.info",
        "unica.code.search",
        "unica.code.diagnostics",
    ],
    "log-analysis": [
        "unica.code.search",
        "unica.meta.info",
        "unica.view",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
    ],
    "background-jobs": [
        "unica.view",
        "unica.code.search",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "data-exchange": [
        "unica.view",
        "unica.code.search",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "db-performance": [
        "unica.view",
        "unica.code.search",
        "unica.code.outline",
        "unica.meta.info",
        "unica.dcs.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "security-auth-crypto": [
        "unica.view",
        "unica.code.search",
        "unica.meta.info",
        "unica.role.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "data-separation": [
        "unica.view",
        "unica.code.search",
        "unica.meta.info",
        "unica.role.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "release-support": [
        "unica.view",
        "unica.code.search",
        "unica.cf.info",
        "unica.cfe.diff",
        "unica.meta.info",
        "unica.code.diagnostics",
        "unica.standards.search",
        "unica.standards.explain",
        "unica.runtime.execute",
    ],
    "source-access": [
        "unica.source.resolve",
        "unica.source.children",
        "unica.source.resources",
        "unica.source.read",
        "unica.source.locate",
    ],
    "document-posting": [
        "unica.view",
        "unica.meta.info",
        "unica.meta.edit",
        "unica.code.definition",
        "unica.code.outline",
        "unica.code.patch",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "register-design": [
        "unica.view",
        "unica.meta.info",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.code.search",
        "unica.dcs.info",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "object-events": [
        "unica.view",
        "unica.meta.info",
        "unica.code.search",
        "unica.code.definition",
        "unica.code.outline",
        "unica.code.graph",
        "unica.code.patch",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "form-events": [
        "unica.view",
        "unica.form.info",
        "unica.form.edit",
        "unica.meta.info",
        "unica.code.outline",
        "unica.code.patch",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "module-placement": [
        "unica.view",
        "unica.meta.info",
        "unica.meta.add",
        "unica.subsystem.info",
        "unica.code.graph",
        "unica.code.patch",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "metadata-modeling": [
        "unica.view",
        "unica.cf.info",
        "unica.meta.info",
        "unica.meta.add",
        "unica.meta.edit",
        "unica.subsystem.info",
        "unica.code.diagnostics",
        "unica.runtime.execute",
    ],
    "transactions-locks": [
        "unica.view",
        "unica.code.search",
        "unica.code.outline",
        "unica.code.graph",
        "unica.code.patch",
        "unica.code.diagnostics",
        "unica.meta.info",
        "unica.runtime.execute",
    ],
    "object-locks": [
        "unica.view",
        "unica.code.search",
        "unica.code.graph",
        "unica.code.patch",
        "unica.form.info",
        "unica.code.diagnostics",
        "unica.runtime.execute",
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
    "test-authoring": ['"testRunner": "yaxunit"', '"testRunner": "va"'],
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
        "unica.code.patch",
        "unica.source.locate",
        "только `read`",
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
    "cf-edit": ["ConfigPath", "Operation", "Value"],
    "cf-info": ["ConfigPath"],
    "cf-init": ["Name", "OutputDir"],
    "cf-validate": ["ConfigPath"],
    "cfe-borrow": ["ExtensionPath", "ConfigPath", "Object"],
    "cfe-diff": ["ExtensionPath", "ConfigPath"],
    "cfe-init": ["Name", "OutputDir"],
    "cfe-patch-method": ["ExtensionPath", "ModulePath", "MethodName"],
    "cfe-validate": ["ExtensionPath"],
    "epf-init": ["Name", "OutputDir", "FormName"],
    "erf-init": ["Name", "OutputDir", "FormName"],
    "meta-add": ["sourceSet", "kind", "name"],
    "meta-edit": ["sourceSet", "metadataPath", "operations"],
    "meta-info": ["sourceSet", "metadataPath"],
    "meta-remove": ["sourceSet", "metadataPath", "dryRun"],
    "form-add": ["ObjectPath", "FormName", "Purpose"],
    "form-compile": ["JsonPath", "OutputPath"],
    "form-edit": ["FormPath", "JsonPath"],
    "form-info": ["FormPath"],
    "form-remove": ["ObjectName", "FormName", "SrcDir"],
    "form-validate": ["FormPath"],
    "interface-edit": ["CIPath", "Operation", "Value"],
    "interface-validate": ["CIPath"],
    "subsystem-compile": ["Value", "OutputDir"],
    "subsystem-edit": ["SubsystemPath", "Operation", "Value"],
    "subsystem-info": ["SubsystemPath"],
    "subsystem-validate": ["SubsystemPath"],
    "dcs-compile": ["DefinitionFile", "OutputPath"],
    "dcs-edit": ["TemplatePath", "Operation", "Value"],
    "dcs-info": ["TemplatePath"],
    "dcs-validate": ["TemplatePath"],
    "mxl-compile": ["JsonPath", "OutputPath"],
    "mxl-decompile": ["TemplatePath"],
    "mxl-info": ["TemplatePath", "WithText"],
    "mxl-validate": ["TemplatePath"],
    "role-compile": ["JsonPath", "OutputDir"],
    "role-edit": ["sourceSet", "metadataPath", "operations"],
    "role-info": ["RightsPath"],
    "role-validate": ["RightsPath"],
}

SCENARIO_PRESERVING_MIN_MCP_CALLS = {
    "cf-edit": 6,
    "cf-info": 6,
    "cf-init": 6,
    "cf-validate": 2,
    "cfe-borrow": 7,
    "cfe-diff": 3,
    "cfe-init": 6,
    "cfe-patch-method": 4,
    "cfe-validate": 2,
    "meta-add": 2,
    "meta-edit": 3,
    "meta-info": 6,
    "meta-remove": 1,
    "form-add": 6,
    "form-compile": 4,
    "form-validate": 2,
    "interface-edit": 8,
    "interface-validate": 2,
    "subsystem-compile": 4,
    "subsystem-edit": 6,
    "subsystem-info": 5,
    "subsystem-validate": 2,
    "dcs-compile": 5,
    "dcs-info": 12,
    "dcs-validate": 2,
    "mxl-info": 3,
    "mxl-validate": 2,
    "role-info": 2,
    "role-edit": 1,
    "dcs-edit": 4,
    "role-compile": 3,
}

ALLOWED_ADDITIONAL_MCP_TOOL_NAMES = {
    "cf-init": {"unica.cf.info", "unica.cf.validate"},
    "cfe-borrow": {"unica.cfe.validate"},
    "cfe-init": {"unica.cfe.validate"},
    "epf-init": {"unica.runtime.execute"},
    "erf-init": {"unica.runtime.execute"},
    "form-compile": {"unica.form.info", "unica.form.validate"},
    "interface-edit": {"unica.interface.validate"},
    "role-compile": {"unica.role.info", "unica.role.validate"},
    "dcs-compile": {"unica.dcs.info", "unica.dcs.validate"},
    "dcs-edit": {"unica.dcs.info", "unica.dcs.validate"},
}

SCENARIO_PRESERVING_TOKENS = {
    "cf-edit": [
        '"Operation": "modify-property"',
        '"Value": "Version=1.0.0.1 ;; Vendor=Фирма 1С"',
        '"Operation": "add-childObject"',
        '"Operation": "remove-childObject"',
        '"Operation": "add-defaultRole"',
        '"Operation": "set-defaultRoles"',
    ],
    # ADR-0023: cf.info answers with typed data, so the prose-size levers are
    # gone from the skill and the documented fields take their place.
    "cf-info": [
        '"ConfigPath": "src"',
        "`support`",
        "`childObjects`",
        "`homePage`",
    ],
    "cf-init": [
        '"Name": "МояКонфигурация"',
        '"Version": "1.0.0.1"',
        '"Vendor": "Фирма 1С"',
        "Режим совместимости (default: `Version8_3_27`)",
        '"CompatibilityMode": "Version8_3_27"',
        '"name": "unica.cf.info"',
        '"name": "unica.cf.validate"',
    ],
    "cfe-borrow": [
        '"Object": "Catalog.Контрагенты"',
        '"Object": "Catalog.Контрагенты.Form.ФормаЭлемента"',
        '"Object": "Catalog.Контрагенты ;; CommonModule.ОбщийМодуль ;; Enum.ВидыОплат"',
        '"BorrowMainAttribute": true',
        '"BorrowMainAttribute": "All"',
        '"name": "unica.cfe.validate"',
    ],
    "cfe-diff": ["`transfer[]`", "`objects[].status`"],
    "cfe-init": [
        '"ConfigPath": "C:\\\\WS\\\\tasks\\\\cfsrc\\\\erp_8.3.24"',
        '"Purpose": "Patch"',
        '"CompatibilityMode": "Version8_3_17"',
        '"Version": "1.0.0.1"',
        '"NamePrefix": "ИБ_"',
        '"NoRole": true',
        '"name": "unica.cfe.validate"',
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
    "meta-edit": [
        '"op": "setProperties"',
        '"op": "add"',
        '"collection": "attributes"',
        '"allowedLength": "variable"',
        '"op": "editRelations"',
        '"relation": "basedOn"',
        '"relation": "source"',
        '"kind": "recordSet"',
        '"metadataPath": "InformationRegister.ИсторияИзменений"',
        '"mode": "replace"',
        '"targets": [',
    ],
    # `Name` and `Mode` were report selectors. The typed answer carries the
    # whole object, so the scenarios are preserved by the addresses they read,
    # not by the drill-down argument that no longer exists (ADR-0023).
    "meta-info": [
        '"metadataPath": "Catalog.Валюты"',
        '"metadataPath": "Document.АвансовыйОтчет"',
        '"metadataPath": "HTTPService.ExternalAPI"',
        '"metadataPath": "WebService.EnterpriseDataUpload_1_0_1_1"',
        '"metadataPath": "DefinedType.GLN"',
        '"metadataPath": "EventSubscription.ОбработкаИзменений"',
    ],
    "meta-remove": [
        '"metadataPath": "Catalog.Устаревший"',
        '`force: true`, `confirm: true`, `dryRun: false`',
        '"dryRun": true',
    ],
    "form-add": [
        '"ObjectPath": "Documents/АвансовыйОтчет.xml"',
        '"Purpose": "List"',
        '"Purpose": "Record"',
        '"Purpose": "Choice"',
        '"Synonym": "Выбор номенклатуры"',
        '"SetDefault": true',
    ],
    "form-compile": [
        '"OutputPath": "<.../TypePlural/ObjectName/Forms/FormName/Ext/Form.xml>"',
        '"name": "unica.form.validate"',
        '"name": "unica.form.info"',
    ],
    "interface-edit": [
        '"Operation": "hide"',
        '"Operation": "show"',
        '"Operation": "place"',
        '"Operation": "subsystem-order"',
        '"CreateIfMissing": true',
        '"name": "unica.interface.validate"',
    ],
    "subsystem-compile": [
        '"Value": "{\\"name\\":\\"Тест\\"}"',
        'CommonPicture.Продажи',
        '"Parent": "config/Subsystems/Продажи.xml"',
    ],
    "subsystem-edit": [
        '"Operation": "add-content"',
        '"Operation": "remove-content"',
        '"Operation": "add-child"',
        '"Operation": "set-property"',
    ],
    "subsystem-info": [
        '"SubsystemPath": "Subsystems"',
        "`commandInterface`",
        "`tree`",
    ],
    "role-compile": [
        '"name": "unica.role.validate"',
        '"name": "unica.role.info"',
    ],
    "dcs-compile": [
        '"DefinitionFile": "<json>"',
        '"Value": "<json-string>"',
        '"name": "unica.dcs.validate"',
        '"name": "unica.dcs.info"',
    ],
    "dcs-edit": [
        '"Operation": "add-field"',
        '"Value": "Цена: decimal(15,2) ;; Количество: decimal(15,3) ;; Сумма: decimal(15,2)"',
        '"name": "unica.dcs.validate"',
        '"name": "unica.dcs.info"',
    ],
    # Eleven `Mode` values selected eleven reports. The typed answer carries
    # every section at once, so the scenarios are preserved by the sections the
    # skill names, not by the selector that no longer exists (ADR-0023).
    "dcs-info": [
        "`dataSets`",
        "`links`",
        "`calculatedFields`",
        "`totalFields`",
        "`parameters`",
        "`variants`",
        "`templates`",
    ],
    "mxl-info": [
        '"WithText": true',
        "`columnSets`",
        "`outside`",
    ],
    "role-info": ["`data.denied`", "`restrictedObjects`"],
}

# Arguments the MCP contract used to publish and now rejects. The packaged skill
# must not keep advertising them: the server would answer such a call with
# "does not accept argument", so a leftover example is a broken instruction.
SCENARIO_RETIRED_TOKENS = {
    "meta-add": ['"JsonPath"', '"OutputDir"', '"DefinitionFile"'],
    "meta-edit": ['"ObjectPath"', '"Operation"', '"Value"', '"DefinitionFile"'],
    "meta-remove": ['"ConfigDir"', '"Object"', '"Force"', '"KeepFiles"', '"keepFiles"'],
    "cf-info": ['"Mode"', '"Section"', '"Limit"', '"Offset"'],
    "role-info": ['"ShowDenied"', '"Limit"', '"Offset"'],
    "subsystem-info": ['"Mode"', '"Name"', '"Limit"', '"Offset"'],
    "mxl-info": ['"Format"', '"MaxParams"', '"Limit"', '"Offset"'],
    "cfe-diff": ['"Mode"'],
    # A published example that still selects by path shows a call the server
    # now rejects with `legacy_target_removed`.
    "meta-info": ['"ObjectPath"', '"objectPath"', '"Detailed"', '"detailed"'],
}


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
            "cf-info",
            "cf-validate",
            "cfe-validate",
            "meta-info",
            "interface-validate",
            "subsystem-info",
            "subsystem-validate",
            "dcs-info",
            "dcs-validate",
            "role-info",
            "role-validate",
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

    def test_meta_skill_surface_is_exactly_four_typed_operations(self) -> None:
        expected = {
            "meta-info": "unica.meta.info",
            "meta-add": "unica.meta.add",
            "meta-edit": "unica.meta.edit",
            "meta-remove": "unica.meta.remove",
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

    def test_meta_examples_follow_final_typed_contracts(self) -> None:
        documents = {
            skill: (self.skill_root() / skill / "SKILL.md").read_text(
                encoding="utf-8"
            )
            for skill in ("meta-info", "meta-add", "meta-edit", "meta-remove")
        }
        calls = {
            skill: [
                json.loads(block)
                for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
                if '"method": "tools/call"' in block
            ]
            for skill, text in documents.items()
        }

        self.assertTrue(calls["meta-add"])
        for call in calls["meta-add"]:
            arguments = call["params"]["arguments"]
            self.assertEqual(call["params"]["name"], "unica.meta.add")
            self.assertTrue({"sourceSet", "kind", "name"}.issubset(arguments))
            self.assertLessEqual(
                set(arguments),
                {"cwd", "sourceSet", "kind", "name", "operations", "dryRun"},
            )
            if "operations" in arguments:
                self.assertIsInstance(arguments["operations"], list)
                self.assertTrue(arguments["operations"])
                self.assertTrue(
                    all(
                        isinstance(operation, dict)
                        for operation in arguments["operations"]
                    )
                )

        self.assertTrue(calls["meta-edit"])
        edit_operations = []
        for call in calls["meta-edit"]:
            arguments = call["params"]["arguments"]
            self.assertEqual(call["params"]["name"], "unica.meta.edit")
            self.assertEqual(
                set(arguments)
                - {"cwd", "sourceSet", "metadataPath", "operations", "dryRun"},
                set(),
            )
            self.assertIsInstance(arguments["operations"], list)
            self.assertTrue(arguments["operations"])
            self.assertTrue(
                all(isinstance(operation, dict) for operation in arguments["operations"])
            )
            edit_operations.extend(arguments["operations"])

        self.assertEqual(
            {operation.get("op") for operation in edit_operations},
            {"setProperties", "add", "update", "remove", "editRelations"},
        )
        for operation in edit_operations:
            with self.subTest(edit_operation=operation.get("op")):
                if operation.get("collection") == "predefinedItems":
                    allowed_fields = {
                        "add": {"op", "collection", "elements"},
                        "update": {"op", "collection", "elements"},
                        "remove": {"op", "collection", "ids"},
                    }[operation["op"]]
                else:
                    allowed_fields = {
                        "setProperties": {"op", "values"},
                        "add": {"op", "collection", "scope", "elements"},
                        "update": {"op", "collection", "scope", "elements"},
                        "remove": {"op", "collection", "scope", "names"},
                        "editRelations": {"op", "relation", "mode", "targets"},
                    }[operation["op"]]
                self.assertLessEqual(set(operation), allowed_fields)

        predefined_operations = [
            operation
            for operation in edit_operations
            if operation.get("collection") == "predefinedItems"
        ]
        self.assertEqual(
            {operation["op"] for operation in predefined_operations},
            {"add", "update", "remove"},
        )
        for operation in predefined_operations:
            with self.subTest(predefined_operation=operation["op"]):
                self.assertNotIn("scope", operation)
                self.assertNotIn("names", operation)
                if operation["op"] == "remove":
                    self.assertEqual(set(operation), {"op", "collection", "ids"})
                    self.assertTrue(operation["ids"])
                else:
                    self.assertEqual(
                        set(operation), {"op", "collection", "elements"}
                    )
                    self.assertTrue(operation["elements"])

        for scoped_operation in ("update", "remove"):
            matching = [
                operation
                for operation in edit_operations
                if operation.get("op") == scoped_operation
            ]
            self.assertTrue(matching)
            self.assertTrue(
                any(
                    set(operation.get("scope", {})) == {"tabularSection"}
                    and bool(operation["scope"]["tabularSection"])
                    for operation in matching
                )
            )

        source_operations = [
            operation
            for operation in edit_operations
            if operation.get("relation") == "source"
        ]
        self.assertTrue(source_operations)
        for operation in source_operations:
            self.assertEqual(operation["op"], "editRelations")
            self.assertEqual(operation["mode"], "replace")
            self.assertIsInstance(operation["targets"], list)
            for target in operation["targets"]:
                self.assertIn(
                    target.get("kind"),
                    {
                        "string",
                        "number",
                        "boolean",
                        "date",
                        "valueStorage",
                        "object",
                        "reference",
                        "recordSet",
                        "definedType",
                    },
                )
        self.assertTrue(
            any(
                target.get("kind") == "recordSet"
                and target.get("metadataPath")
                == "InformationRegister.ИсторияИзменений"
                for operation in source_operations
                for target in operation["targets"]
            )
        )
        self.assertIn("wire-массив набора", documents["meta-edit"])
        self.assertIn(
            "порядок его членов семантически незначим", documents["meta-edit"]
        )
        self.assertIn("exact-byte no-op", documents["meta-edit"])

        add_operations = [
            operation
            for call in calls["meta-add"]
            for operation in call["params"]["arguments"].get("operations", [])
        ]
        self.assertTrue(
            any(
                operation.get("op") == "editRelations"
                and operation.get("relation") == "source"
                and operation.get("mode") == "replace"
                for operation in add_operations
            )
        )

        info = documents["meta-info"]
        # `meta.info` no longer consults an index, so the fields that only made
        # sense for a possibly-stale provider are gone from the answer and must
        # not be promised by the prose either.
        for token in ("`validation`", "status", "diagnostics", "usage", "predefinedItems"):
            with self.subTest(info_token=token):
                self.assertIn(token, info)
        for retired in ("freshness", "completeness", "soft-fail", "related"):
            with self.subTest(info_retired=retired):
                self.assertNotIn(retired, info)
        self.assertNotIn("`total`, `limit`", info)
        self.assertTrue(
            any("sections" not in call["params"]["arguments"] for call in calls["meta-info"])
        )
        self.assertTrue(
            any(
                call["params"]["arguments"].get("sections")
                and 1 <= call["params"]["arguments"].get("limit", 0) <= 50
                for call in calls["meta-info"]
            )
        )

        for skill in ("meta-add", "meta-edit", "meta-info", "meta-remove"):
            with self.subTest(skill=skill, contract="structured result"):
                text = documents[skill]
                self.assertIn("structuredContent", text)
                self.assertIn("isError == !structuredContent.ok", text)
                self.assertIn(
                    "не является вторым контрактом", " ".join(text.split())
                )

        self.assertNotIn("upsert-predefined", documents["meta-edit"])

        for skill in ("meta-add", "meta-edit", "meta-remove"):
            with self.subTest(skill=skill, contract="preview effects"):
                text = documents[skill]
                self.assertIn("data.effects", text)
                self.assertIn("полный XML", text)

        remove = documents["meta-remove"]
        self.assertIn("`sourceSet + metadataPath`", remove)
        self.assertIn("`force: true`, `confirm: true`, `dryRun: false`", remove)

    def test_role_edit_skill_uses_only_the_logical_typed_contract(self) -> None:
        text = (self.skill_root() / "role-edit" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        calls = [
            json.loads(block)
            for block in re.findall(r"```json\n(.*?)\n```", text, flags=re.S)
            if '"method": "tools/call"' in block
        ]

        self.assertEqual(len(calls), 1)
        params = calls[0]["params"]
        self.assertEqual(params["name"], "unica.role.edit")
        arguments = params["arguments"]
        self.assertEqual(
            set(arguments), {"sourceSet", "metadataPath", "operations", "dryRun"}
        )
        self.assertRegex(arguments["metadataPath"], r"^Role\.[^.]+$")
        self.assertIs(arguments["dryRun"], True)
        self.assertTrue(arguments["operations"])
        for operation in arguments["operations"]:
            self.assertEqual(
                set(operation), {"op", "objectName", "right", "value"}
            )
            self.assertEqual(operation["op"], "setRight")
            self.assertIsInstance(operation["value"], bool)

        encoded_call = json.dumps(calls[0], ensure_ascii=False)
        for legacy in ("RightsPath", "Path", "ObjectName", "Name", "Value"):
            with self.subTest(legacy=legacy):
                self.assertNotIn(f'"{legacy}"', encoded_call)
        for token in (
            "structuredContent.data",
            "metadataPath",
            "changed",
            "effects",
            "operationIndex",
            "validation",
            "diagnostics",
        ):
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

    def test_in_scope_skills_route_to_single_unica_mcp(self) -> None:
        for skill, tool_name in IN_SCOPE_TOOLS.items():
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
                self.assertIn("## MCP routing", text)
                self.assertIn("MCP `unica`", text)
                self.assertIn(tool_name, text)
                self.assertNotIn("unica-coder", text)
                self.assertNotIn("unica-v8-runner", text)
                self.assertNotIn("unica-bsl-workspace", text)
                self.assertNotIn("unica-rlm-tools-bsl", text)
                self.assertNotIn("unica-v8std", text)

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

    def test_code_diagnostics_describes_operational_config_fallback(self) -> None:
        text = (self.skill_root() / "code-diagnostics" / "SKILL.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("operational.code_diagnostics.analyze_timeout_seconds", text)
        self.assertIn("unica.local.toml", text)
        self.assertIn("unica.toml", text)
        self.assertIn("compiled 120-second fallback", text)
        self.assertIn("do not read this operational config", text)

    def test_code_diagnostics_routes_providers_internally(self) -> None:
        text = (self.skill_root() / "code-diagnostics" / "SKILL.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("no_applicable_provider", text)
        self.assertIn("Provider execution is routed internally", text)
        self.assertNotIn('providers: ["bsl-analyzer"]', text)
        self.assertIn("inline/range disable markers", text)
        self.assertIn("suppression-комментарии", text)

    def test_code_diagnostics_examples_use_logical_action_contract(self) -> None:
        calls = []
        for path in sorted(self.skill_root().glob("*/SKILL.md")):
            text = path.read_text(encoding="utf-8")
            for block in re.findall(r"```(?:json|jsonc)\n(.*?)\n```", text, flags=re.S):
                try:
                    payload = json.loads(block)
                except json.JSONDecodeError:
                    continue
                if not isinstance(payload, dict):
                    continue
                params = payload.get("params", {})
                if params.get("name") == "unica.code.diagnostics":
                    calls.append((path, params.get("arguments", {})))

        self.assertGreaterEqual(len(calls), 2)
        forbidden = {
            "mode",
            "sourceDir",
            "path",
            "codes",
            "minSeverity",
            "rangeStart",
            "rangeEnd",
            "config",
            "format",
            "detail",
            "maxFiles",
        }
        for path, arguments in calls:
            with self.subTest(
                path=path.relative_to(self.repo_root()), arguments=arguments
            ):
                self.assertIn(
                    arguments.get("action"),
                    {"analyze", "findings", "status", "catalog"},
                )
                self.assertIsInstance(arguments.get("sourceSet"), str)
                self.assertTrue(arguments["sourceSet"].strip())
                self.assertEqual(forbidden.intersection(arguments), set())
                if arguments["action"] == "findings":
                    self.assertRegex(
                        arguments.get("metadataPath", ""),
                        r"^[^.]+\.[^.]+(?:\..+)?$",
                    )
                for code in arguments.get("filter", {}).get("codes", []):
                    self.assertEqual(set(code), {"provider", "code"})
                    self.assertTrue(code["provider"])
                    self.assertTrue(code["code"])

        code_diagnostics_calls = [
            arguments
            for path, arguments in calls
            if path.parent.name == "code-diagnostics"
        ]
        self.assertTrue(
            any(call["action"] == "analyze" for call in code_diagnostics_calls)
        )
        findings = next(
            call
            for call in code_diagnostics_calls
            if call["action"] == "findings"
        )
        self.assertEqual(findings["sourceSet"], "main")
        self.assertIn("metadataPath", findings)
        self.assertTrue(findings.get("filter", {}).get("codes"))

    def test_unica_owned_guidance_contains_required_operational_concepts(self) -> None:
        docs = {
            "code-search": self.skill_root() / "code-search" / "SKILL.md",
            "code-diagnostics": self.skill_root() / "code-diagnostics" / "SKILL.md",
            "test-authoring": self.skill_root() / "test-authoring" / "SKILL.md",
            "background-jobs": self.skill_root() / "background-jobs" / "SKILL.md",
            "db-performance": self.skill_root() / "db-performance" / "SKILL.md",
            "integration-implement": self.skill_root() / "integration-implement" / "SKILL.md",
            "platform-mechanics": self.reference_root() / "platform" / "platform-mechanics.md",
            "runtime-diagnostics": self.reference_root() / "platform" / "runtime-diagnostics.md",
            "db-performance-ref": self.reference_root() / "platform" / "db-performance.md",
            "integration-contracts": self.reference_root() / "platform" / "integration-contracts.md",
        }
        joined = "\n".join(path.read_text(encoding="utf-8") for path in docs.values())

        for token in [
            "MCP-first",
            "what was tried",
            "verification gate",
            "impact analysis",
            "managed locks",
            "lock order",
            "structured logging",
            "DCS",
            "idempotency key",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, joined)

    def test_compatibility_guidance_preserves_effective_version_contract(self) -> None:
        reference_path = self.reference_root() / "platform" / "compatibility-modes.md"
        self.assertTrue(reference_path.is_file())
        reference = reference_path.read_text(encoding="utf-8")

        for token in [
            "runtime platform line",
            "configured compatibility mode",
            "effective compatibility version",
            "`DontUse` -> runtime platform line",
            "`VersionX` -> `X`",
            "`CompatibilityMode`",
            "`ConfigurationExtensionCompatibilityMode`",
            "`InterfaceCompatibilityMode`",
            "code location does not select the mode family",
            "corroborating implementation evidence",
            "not complete old-platform equivalence",
        ]:
            with self.subTest(token=token):
                self.assertIn(token, reference)

        for skill in ["platform-help", "release-support", "bsp-patterns"]:
            skill_text = (self.skill_root() / skill / "SKILL.md").read_text(
                encoding="utf-8"
            )
            with self.subTest(skill=skill):
                self.assertIn(
                    "references/platform/compatibility-modes.md",
                    skill_text,
                )

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

    def test_platform_help_uses_one_contract_gap_label(self) -> None:
        platform_help = (self.skill_root() / "platform-help" / "SKILL.md").read_text(
            encoding="utf-8"
        )

        self.assertNotIn("Unica MCP contract gap", platform_help)
        self.assertIn("platform-help contract gap", platform_help)

    def test_all_skills_do_not_expose_internal_mcp_names(self) -> None:
        forbidden = [
            "unica-coder",
            "unica-v8-runner",
            "unica-bsl-reference",
            "unica-bsl-workspace",
            "unica-rlm-tools-bsl",
            "unica-v8std",
        ]
        for skill_path in self.skill_root().glob("*/SKILL.md"):
            with self.subTest(skill=skill_path.parent.name):
                text = skill_path.read_text(encoding="utf-8")
                for name in forbidden:
                    self.assertNotIn(name, text)

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

        scanned_docs = [
            self.repo_root() / "README.md",
            self.repo_root() / "plugins" / "unica" / "README.md",
            self.reference_root() / "README.md",
            self.reference_root() / "tooling" / "v8project.md",
            self.reference_root() / "tooling" / "runtime-build.md",
            self.reference_root() / "use-cases" / "workspace-runtime.md",
            self.reference_root() / "use-cases" / "forms-ui.md",
            self.reference_root() / "use-cases" / "reports-printing.md",
        ]
        for doc in scanned_docs:
            text = doc.read_text(encoding="utf-8")
            self.assertNotIn("`v8-runner` skill", text)
            self.assertNotIn("Use the `v8-runner` skill", text)
            for skill in REPLACED_RUNTIME_SKILLS:
                with self.subTest(path=doc.relative_to(self.repo_root()), skill=skill):
                    self.assertNotIn(f"/{skill}", text)
                    self.assertNotIn(f"`{skill}`", text)



    def test_db_auth_check_numbers_only_the_two_credential_candidates(self) -> None:
        text = (
            self.skill_root() / "db-auth-check" / "SKILL.md"
        ).read_text(encoding="utf-8")
        credential_rule = text.split(
            "## Правило пустых учетных данных", maxsplit=1
        )[1].split("## Workflow", maxsplit=1)[0]

        self.assertEqual(
            re.findall(r"(?m)^(\d+)\.\s+(.+)$", credential_rule),
            [
                ("1", "`Администратор` с пустым паролем."),
                ("2", "`Admin` с пустым паролем."),
            ],
        )
        self.assertRegex(
            credential_rule,
            r"(?m)^После двух подтверждённых отказов остановись и спроси "
            r"пользователя, из-под кого подключаться\.$",
        )

    def test_runtime_json_guard_accepts_case_insensitive_fence_labels(self) -> None:
        example = """```JSON
{
  "method": "tools/call",
  "params": {
    "name": "unica.runtime.execute",
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
                        "name": "unica.runtime.execute",
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
    "name": "unica.runtime.execute",
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
    "name": "unica\u002eruntime.execute",
    "arguments": {"operation": "build", "dryRun": false}
  }
}
```"""

        payloads = runtime_execute_json_examples(example)

        self.assertEqual(len(payloads), 1)
        self.assertEqual(payloads[0]["params"]["name"], "unica.runtime.execute")
        self.assertIs(payloads[0]["params"]["arguments"]["dryRun"], False)

    def test_runtime_json_guard_keeps_malformed_block_boundary(self) -> None:
        self.assertEqual(
            runtime_execute_json_examples("```json\n{not runtime JSON}\n```"),
            [],
        )
        self.assertEqual(
            runtime_execute_json_examples(
                r'''```json
{"note":"unica\\u002eruntime.execute",
```'''
            ),
            [],
        )

        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                '```json\n{"name":"unica.runtime.execute",\n```'
            )

    def test_runtime_json_guard_rejects_malformed_escaped_tool_name(self) -> None:
        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                r'''```json
{"params":{"name":"unica\u002eruntime.execute","arguments":{"dryRun":false}},
```'''
            )

        with self.assertRaisesRegex(
            ValueError, r"invalid fenced runtime JSON example #1"
        ):
            runtime_execute_json_examples(
                r'''```json
{"params":{"name":"unica\u002eruntime.execute
```'''
            )

    def test_runtime_json_guard_handles_commonmark_fences_and_batches(self) -> None:
        examples = r'''~~~JSON
[{"method":"tools/call","params":{"name":"unica\u002eruntime.execute","arguments":{"dryRun":false}}}]
~~~
``` json
{"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
```
````json
{"note":"```","method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
````'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 3)
        self.assertTrue(
            all(
                payload["params"]["name"] == "unica.runtime.execute"
                and payload["params"]["arguments"]["dryRun"] is False
                for payload in payloads
            )
        )

    def test_runtime_json_guard_handles_commonmark_containers_and_info(self) -> None:
        examples = '''> ```json
> {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
> ```
- example:

    ```json
    {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
    ```
```json title=request
{"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
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
  {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
  ```
> - ```json
>   {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
>   ```'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 2)

    def test_runtime_json_guard_handles_nested_indent_and_info_entities(self) -> None:
        examples = '''123. outer
     - ```json
       {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
       ```
123. outer
     > ```json
     > {"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
     > ```
```j&#x73;on
{"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}}
```'''

        payloads = runtime_execute_json_examples(examples)

        self.assertEqual(len(payloads), 3)

    def test_runtime_guidance_document_detects_decoded_tool_name(self) -> None:
        example = r'''```json
{"method":"tools/call","params":{"name":"unica\u002eruntime.execute","arguments":{"dryRun":false}}}
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
        "name": "unica.runtime.execute",
        "arguments": {"operation": "build", "dryRun": false}
      }
    }
"""

        with self.assertRaisesRegex(ValueError, "indented runtime JSON example"):
            runtime_guidance_document(example)

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
{"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":true}}}
```''',
                ),
                (
                    bad,
                    '''```json
{"method":"tools/call","params":{"name":"unica.runtime.execute","arguments":{"dryRun":false}}
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
                operation=arguments.get("operation"),
            ):
                self.assertIs(arguments.get("dryRun"), True)

        # ADR-0074: applied execution is admitted, so the shipped guidance has to
        # name the applied mode, its risk vocabulary and the durable alternative.
        contract_tokens = (
            "INV-MCP-RUNTIME-RECEIPT",
            "ADR-0074",
            "`dryRun: false`",
            "runtime_risk_",
            "`unica.build.*`",
            "`unica.runtime.job.start`",
        )
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


    def test_shipped_guidance_never_routes_runtime_refusal_through_fallbacks(
        self,
    ) -> None:
        shipped_docs = list(self.skill_root().glob("**/*.md")) + list(
            self.reference_root().glob("**/*.md")
        )
        namespace_pattern = (
            r"`(?:unica\.build|unica\.runtime\.job)\."
            r"(?:\*|[a-z][a-z0-9.-]*)`"
        )
        namespace = re.compile(namespace_pattern)
        fallback_context = re.compile(
            r"(?i)\bfallback\b|\bfall\s+back\b|\bcontinuation\b|"
            r"\bretry\b|\binstead\b|\bworkaround\b|"
            r"(?:after|around|when).{0,60}\b(?:runtime )?refus\w*\b|"
            r"запасн\w*\s+пут\w*|продолжени\w*|повтор\w*|вместо|обход\w*|"
            r"(?:после|при).{0,60}отказ\w*"
        )
        negative_fallback = tuple(
            re.compile(pattern)
            for pattern in (
                rf"(?i)\b(?:do not|don't|never|must not)\s+"
                rf"(?:use|call|invoke|route)\b[^,.;!?]{{0,80}}{namespace_pattern}",
                rf"(?i)\b(?:нельзя|запрещено)\s+"
                rf"(?:использовать|вызывать|направлять)\b[^,.;!?]{{0,80}}"
                rf"{namespace_pattern}",
                rf"(?i)\bне\s+(?:используй|вызывай|вызови|направь|перейди)\b"
                rf"[^,.;!?]{{0,80}}{namespace_pattern}",
                rf"(?i)\b(?:do not|don't|never|must not)\b[^.;!?]{{0,120}}"
                rf"\bbypass\b[^.;!?]{{0,120}}{namespace_pattern}",
                rf"(?i)\bне\s+обходи\b[^.;!?]{{0,160}}{namespace_pattern}",
                rf"(?i){namespace_pattern}[^.;!?]{{0,100}}"
                rf"(?:\bis not\b|\bisn't\b|\bnot\s+as\b|"
                rf"\bdo not use (?:it|them)\b)[^.;!?]{{0,60}}"
                rf"(?:fallback|continuation|retry|workaround)",
                rf"(?i){namespace_pattern}[^.;!?]{{0,100}}"
                rf"\bне\s+(?:является|служит|используй (?:его|их))\b"
                rf"[^.;!?]{{0,60}}(?:запасн\w*\s+пут\w*|fallback|продолжени\w*)",
            )
        )

        def logical_clauses(text: str) -> list[str]:
            return [
                clause.strip()
                for clause in re.split(
                    r"(?<=[.!?;])\s+|^\s*(?:[-*]|\d+[.)])\s+",
                    text,
                    flags=re.MULTILINE,
                )
                if clause.strip()
            ]

        def is_fallback_route(clause: str) -> bool:
            return bool(
                namespace.search(clause)
                and fallback_context.search(clause)
                and not any(pattern.search(clause) for pattern in negative_fallback)
            )

        unsafe_examples = (
            "Use `unica.build.load` as fallback.",
            "Route through `unica.runtime.job.start` as continuation after runtime refusal.",
            "Используй `unica.build.*` как запасной путь.",
            "После отказа перейди в `unica.runtime.job.status`.",
            "Fall back to `unica.build.load` after runtime refusal.",
            "After runtime refusal, call `unica.runtime.job.start`.",
            "После отказа вызови `unica.build.load`.",
            "If runtime.execute cannot run, use `unica.runtime.job.start` as fallback.",
            "Не вызывай runtime.execute повторно, используй `unica.runtime.job.start` как запасной путь.",
            "Если runtime.execute нельзя вызвать, используй `unica.build.load` вместо него.",
        )
        safe_examples = (
            "Do not use `unica.build.load` as fallback.",
            "Не используй `unica.runtime.job.start` после отказа.",
            "Use `unica.runtime.job.start` for an explicitly requested durable job.",
            "Используй `unica.build.load` для независимого build workflow.",
            "Use `unica.runtime.job.start` for an explicitly requested durable job; do not use it as fallback.",
            "Используй `unica.build.load` для отдельного workflow; не используй его как запасной путь.",
        )
        for example in unsafe_examples:
            self.assertTrue(
                any(is_fallback_route(clause) for clause in logical_clauses(example)),
                example,
            )
        for example in safe_examples:
            self.assertFalse(
                any(is_fallback_route(clause) for clause in logical_clauses(example)),
                example,
            )
        for doc in shipped_docs:
            text = doc.read_text(encoding="utf-8")
            with self.subTest(path=doc.relative_to(self.repo_root())):
                violating = [
                    clause
                    for clause in logical_clauses(text)
                    if is_fallback_route(clause)
                ]
                self.assertEqual(violating, [])


    def test_code_quality_runtime_preview_preserves_test_first_order(self) -> None:
        doc = self.reference_root() / "use-cases" / "code-quality-review.md"
        text = doc.read_text(encoding="utf-8")
        test_first = re.search(
            r"write a reproducing test.{0,160}confirm that it fails.{0,160}before changing",
            text,
            flags=re.DOTALL,
        )
        self.assertIsNotNone(test_first)
        self.assertLess(test_first.start(), text.index("preview intended syntax/test"))






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
        dcs_info = (self.skill_root() / "dcs-info" / "SKILL.md").read_text(encoding="utf-8")
        dcs_dsl = (self.reference_root() / "specs" / "dcs-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        dcs_spec = (self.reference_root() / "specs" / "1c-dcs-spec.md").read_text(
            encoding="utf-8"
        )

        for text in [dcs_compile, dcs_edit, dcs_info]:
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
        # `Raw` existed because pagination mangled the query; `data` carries the
        # exact text always, so the promise moved into the section description.
        self.assertNotIn('"Raw": true', dcs_info)
        self.assertIn("сырой текст запроса целиком", dcs_info)
        self.assertIn("unica.dcs.edit", dcs_info)
        self.assertIn("patch-query", dcs_edit)
        self.assertIn("@once", dcs_edit)
        self.assertIn("availableValue=", dcs_edit)
        self.assertIn("value=", dcs_edit)

    def test_form_skills_track_upstream_dsl_features_through_unica_boundary(self) -> None:
        form_compile = (self.skill_root() / "form-compile" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        form_edit = (self.skill_root() / "form-edit" / "SKILL.md").read_text(encoding="utf-8")
        form_info = (self.skill_root() / "form-info" / "SKILL.md").read_text(encoding="utf-8")
        form_validate = (self.skill_root() / "form-validate" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        form_dsl = (self.reference_root() / "specs" / "form-dsl-spec.md").read_text(
            encoding="utf-8"
        )
        form_patterns = (self.reference_root() / "specs" / "form-patterns.md").read_text(
            encoding="utf-8"
        )

        for text in [form_compile, form_edit, form_info, form_validate]:
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
        self.assertIn("unica.form.info", form_edit)
        self.assertIn("unica.form.validate", form_edit)

    def test_form_patterns_ux_guidance_is_mirrored_and_uses_supported_dsl(self) -> None:
        heading = "## UX-правила для элементов и компоновки форм"
        legacy_heading = "## UX-правила для элементов форм"

        def ux_section(path: Path) -> str:
            text = path.read_text(encoding="utf-8")
            start = text.index(heading if heading in text else legacy_heading)
            end = text.index("\n---", start)
            return text[start:end]

        reference_path = self.reference_root() / "specs" / "form-patterns.md"
        skill_path = self.skill_root() / "form-patterns" / "SKILL.md"
        reference_section = ux_section(reference_path)
        skill_section = ux_section(skill_path)

        self.assertIn(heading, reference_path.read_text(encoding="utf-8"))
        self.assertIn(heading, skill_path.read_text(encoding="utf-8"))
        self.assertEqual(skill_section, reference_section)
        self.assertIn(
            "https://github.com/Oxotka/1CDesignGuide/tree/edc05eaf5c191250a184b0e185006bf4b412f7a5",
            reference_section,
        )
        for token in [
            "Обычная группа",
            "прижатия элементов и заголовков к краю",
            "Сильное",
            "Обычное",
            "Слабое",
            "Сворачиваемая группа",
            "не отображайте отступ слева",
            '"showLeftMargin": false',
            "`collapsed` задаёт начальное состояние",
            "`Группа.Показать()`",
            "`Группа.Скрыть()`",
            "только в коде формы",
            "Всплывающая группа",
            "DSL пока не может настроить `ControlRepresentation`",
            "как подсказку",
            "подобно гиперссылке",
            "Командная панель",
            '"commandSource": "Form"',
            '"commandSource": "FormCommandPanelGlobalCommands"',
            '"commandName": "CommonCommand.ОткрытьПараметры"',
            "вручную устраните дубли",
            "не выдавайте `popup` или `buttonGroup` за исполнимые нативные элементы",
            "Команды формы",
            "Шапка формы",
            "функциональным опциям",
            "автозаполняются или сохраняют предыдущее значение",
            "изменяющее форму, ставьте первым",
            "Подвал формы",
            "Комментарий и Ответственный последними",
            "строковых полей с доступным выбором",
            '"choiceButton": true',
            "очевидных полей",
            '"titleLocation": "none"',
            '"titleLocation": "top"',
            '"inputHint": "По всем организациям"',
            '"showInHeader": false',
            '"readOnly": true',
            '"horizontalStretch": true',
            '"headerHorizontalAlign": "Right"',
            '"horizontalAlign": "Right"',
            "не используйте много разных стилей и цветов",
            "достаточно длинное название",
            "двойное отрицание",
            "Проводить документ при записи",
            '"tooltip": "Пояснение"',
            '"tooltipRepresentation": "Button"',
            '"checkBoxType": "switcher"',
            "3–5 значений",
            "на весь экран",
            "слева вверху",
            "модальной",
            "справа внизу",
            '"font": { "bold": true }',
            '"backColor": "#FFFF00"',
        ]:
            with self.subTest(token=token):
                self.assertIn(token, reference_section)

        self.assertEqual(reference_section.count("defaultButton"), 1)
        self.assertNotIn("buttonHint", reference_section)
        self.assertNotIn("RGB(", reference_section)
        self.assertNotRegex(reference_section, r'"radio"\s*:')
        self.assertNotRegex(reference_section, r'"(?:popup|buttonGroup)"\s*:')
        self.assertNotRegex(reference_section, r'"(?:leftIndent|showLeftIndent)"\s*:')
        self.assertNotIn("нет нативного DSL-ключа для левого отступа", reference_section)
        self.assertNotIn('"representation": "Picture"', reference_section)
        self.assertNotIn("Кнопки действий внизу", reference_section)

    def test_form_dsl_keeps_tooltip_and_command_binding_contracts_unambiguous(self) -> None:
        form_dsl = (self.reference_root() / "specs" / "form-dsl-spec.md").read_text(
            encoding="utf-8"
        )

        self.assertIn("Обычный `<Title>` поля и `<ToolTip>`", form_dsl)
        self.assertRegex(form_dsl, r"передавать им `\{text, formatted\}`\s+нельзя")
        self.assertIn("приоритетом `command` → `commandName` → `stdCommand`", form_dsl)
        self.assertIn("`popup` и `buttonGroup` зарезервированы", form_dsl)
        self.assertNotRegex(form_dsl, r'"(?:popup|buttonGroup)"\s*:')

    def test_meta_info_tracks_upstream_type_presentation_through_unica_boundary(self) -> None:
        meta_info = (self.skill_root() / "meta-info" / "SKILL.md").read_text(encoding="utf-8")

        self.assertIn("MCP `unica`", meta_info)
        self.assertIn("unica.meta.info", meta_info)
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
        self.assertIn("unica.meta.add", meta_add)
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
            "cf-info",
            "meta-info",
            "form-info",
            "dcs-info",
            "mxl-info",
            "role-info",
            "subsystem-info",
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
        self.assertIn("unica.cf.info", release_support)
        self.assertIn("unica.meta.info", release_support)

    def test_source_set_format_detection_contract_is_documented(self) -> None:
        docs = {
            "workspace-runtime": self.reference_root()
            / "use-cases"
            / "workspace-runtime.md",
            "metadata-modeling": self.reference_root()
            / "use-cases"
            / "metadata-modeling.md",
            "v8project": self.reference_root() / "tooling" / "v8project.md",
            "format-index": self.reference_root() / "specs" / "format-index.md",
            "invariants": self.repo_root() / "docs" / "arch-v1" / "architecture" / "invariants.md",
        }
        joined = "\n".join(path.read_text(encoding="utf-8") for path in docs.values())

        self.assertIn("unica.view", joined)
        self.assertIn("sourceSets[]", joined)
        self.assertIn("sourceFormat", joined)
        self.assertIn("platform_xml", joined)
        self.assertIn("EDT configuration", joined)
        self.assertIn("platform XML external", joined)
        # The load-bearing claim: the format belongs to one source set, not to
        # the workspace. The plugin references state it in English for skill
        # users; the invariant registry states it in Russian. Either wording
        # satisfies the contract, but one of them has to be present.
        self.assertTrue(
            any(
                phrase in joined
                for phrase in (
                    "not of the whole workspace",
                    "а не всего рабочего пространства",
                )
            ),
            "the source-set format contract must be documented somewhere",
        )
        self.assertNotIn("sourceFormat=mixed", joined)
        self.assertNotIn("source_format=mixed", joined)

    def test_workspace_runtime_routes_project_health_without_granting_mutation(self) -> None:
        joined = (
            self.reference_root() / "use-cases" / "workspace-runtime.md"
        ).read_text(encoding="utf-8")
        normalized = " ".join(joined.split())

        for token in (
            "unica.view {}",
            "ready",
            "repositoryReady",
            "remediation",
            "sourceSet.path: .",
        ):
            self.assertIn(token, joined)
        self.assertIn("does not mean Unica is unusable without Git", normalized)
        self.assertIn("never execute them automatically", normalized)
        self.assertIn("call `unica.view {}` again", normalized)
        self.assertNotIn("it does not inspect repository health", joined)

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


    def test_verified_full_dump_documents_its_publication_risk_contract(self) -> None:
        docs = [
            self.reference_root() / "tooling" / "runtime-build.md",
            self.reference_root() / "tooling" / "v8project.md",
        ]
        required = {
            "Windows": re.compile(r"\bWindows\b", re.IGNORECASE),
            "macOS": re.compile(r"\bmacOS\b", re.IGNORECASE),
            "Linux": re.compile(r"\bLinux\b", re.IGNORECASE),
            "synchronous": re.compile(
                r"\b(?:synchronous|синхронн\w*)\b",
                re.IGNORECASE,
            ),
            "full dump": re.compile(
                r"(?:\bfull\s+dump\b|\bmode\s*=\s*full\b)",
                re.IGNORECASE,
            ),
            "CONFIGURATION": re.compile(r"\bCONFIGURATION\b"),
            "EXTENSION": re.compile(r"\bEXTENSION\b"),
            "verified transactional publication": re.compile(
                r"\bverified\s+transactional\s+publication\b",
                re.IGNORECASE,
            ),
        }
        # Публикация полного дампа исполняется и несёт названный риск записи без
        # ограниченного восстановления (ADR-0074), поэтому абзац контракта
        # обязан говорить о риске, а не о былом отказе.
        publication_risk = re.compile(
            r"(?:bounded recovery|proved (?:terminal )?receipt|ограниченн\w*\s+восстановлени\w*|"
            r"названн\w*\s+риск\w*|named risk)",
            re.I,
        )

        def markdown_paragraphs(text: str) -> list[str]:
            return re.split(r"\n(?:[ \t]*|>[ \t]*)\n", text)

        def contract_paragraphs(text: str) -> list[str]:
            return [
                paragraph
                for paragraph in markdown_paragraphs(text)
                if all(pattern.search(paragraph) for pattern in required.values())
                and publication_risk.search(paragraph)
            ]

        def contract_errors(text: str) -> list[str]:
            errors = []
            if not contract_paragraphs(text):
                errors.append("missing complete full dump publication-risk paragraph")
            return errors

        document_texts = {
            path: path.read_text(encoding="utf-8")
            for path in docs
        }
        for path in docs:
            with self.subTest(document=path.name):
                self.assertEqual([], contract_errors(document_texts[path]))

        mixed_claims = (
            "On Windows, macOS, and Linux, synchronous full dump mode=full for "
            "CONFIGURATION and EXTENSION runs with a named risk while verified "
            "transactional publication lacks bounded recovery."
        )
        self.assertEqual(
            [],
            contract_errors(mixed_claims),
            "the full-dump contract must combine publication and lifecycle scope",
        )

        for path, text in document_texts.items():
            complete_paragraphs = contract_paragraphs(text)
            for missing, pattern in required.items():
                mutated = text
                for paragraph in complete_paragraphs:
                    mutated_paragraph = pattern.sub("", paragraph)
                    mutated = mutated.replace(paragraph, mutated_paragraph, 1)
                with self.subTest(document=path.name, missing=missing):
                    self.assertTrue(contract_errors(mutated), missing)

    def test_code_patch_skill_uses_only_logical_configuration_and_extension_targets(
        self,
    ) -> None:
        path = self.skill_root() / "code-patch" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        calls = []
        for block in re.findall(r"```json\s*(.*?)```", text, re.DOTALL):
            payload = json.loads(block)
            params = payload.get("params", {})
            if params.get("name") == "unica.code.patch":
                calls.append(params.get("arguments", {}))

        self.assertGreaterEqual(len(calls), 2)
        for arguments in calls:
            with self.subTest(arguments=arguments):
                self.assertIn("sourceSet", arguments)
                self.assertIn("metadataPath", arguments)
                self.assertNotIn("path", arguments)
                self.assertNotIn("sourceDir", arguments)
        self.assertRegex(text, r"Configuration.{0,120}Extension")
        self.assertIn("sourceSet", text)
        self.assertIn("metadataPath", text)

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

        # A selector-less insert is the whole point of the current surface, so
        # the body must say where the content lands when the selector is absent.
        self.assertRegex(text, r"(?is)`selector` is optional for `insert`")
        self.assertIn("end of the module", text)

    def test_xdto_skill_uses_one_confirmed_info_preview_apply_mcp_flow(self) -> None:
        path = self.skill_root() / "xdto" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        blocks = list(re.finditer(r"```json\s*(.*?)```", text, re.DOTALL))
        calls = [json.loads(block.group(1)) for block in blocks]

        self.assertEqual(len(calls), 3)
        self.assertEqual(
            [call.get("method") for call in calls],
            ["tools/call", "tools/call", "tools/call"],
        )
        params = [call["params"] for call in calls]
        self.assertEqual(
            [item["name"] for item in params],
            ["unica.xdto.info", "unica.xdto.edit", "unica.xdto.edit"],
        )
        self.assertEqual(
            {item["name"] for item in params},
            {"unica.xdto.info", "unica.xdto.edit"},
        )

        preview = dict(params[1]["arguments"])
        apply = dict(params[2]["arguments"])
        self.assertIs(preview.pop("dryRun"), True)
        self.assertIs(apply.pop("dryRun"), False)
        self.assertEqual(preview, apply)
        confirmation_text = text[blocks[1].end() : blocks[2].start()].casefold()
        self.assertIn("явного подтверждения", confirmation_text)

        for item in params:
            with self.subTest(tool=item["name"]):
                arguments = item["arguments"]
                self.assertEqual(arguments.get("sourceSet"), "main")
                self.assertEqual(
                    arguments.get("metadataPath"),
                    "XDTOPackage.EnterpriseData_1_17_3",
                )
                self.assertNotIn("path", arguments)
                self.assertNotIn("Package.bin", json.dumps(arguments, ensure_ascii=False))
        self.assertEqual(len(preview["operations"]), 1)
        self.assertEqual(preview["operations"][0]["op"], "addProperty")
        self.assertEqual(
            preview["operations"][0]["property"]["type"],
            "tns:Документ_ЗаказКлиента",
        )
        # ADR-0071: a coherent change travels as one ordered transactional
        # operations array, and effects are reported by operationIndex.
        self.assertIn("одним вызовом", text)
        self.assertIn("operationIndex", text)
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

    def test_source_access_skill_routes_reads_and_sends_writes_to_code_patch(
        self,
    ) -> None:
        path = self.skill_root() / "source-access" / "SKILL.md"
        text = path.read_text(encoding="utf-8")
        writer = text.index("unica.code.patch")
        inspect = text.index("unica.source.resources")

        self.assertLess(writer, inspect, "the specialized writer is named first")
        # The resource surface is read-only, so the skill must not promise a
        # write through it and must send edits to unica.code.patch.
        self.assertNotIn("unica.source.apply", text)
        self.assertRegex(text, r"(?s)dryRun.{0,80}true.{0,400}dryRun.{0,80}false")
        self.assertIn("только `read`", text)
        self.assertIn("unica.source.locate", text)
        self.assertIn("replace", text)

    def test_package_readme_documents_code_patch_target_migration(self) -> None:
        text = (
            self.repo_root() / "plugins" / "unica" / "README.md"
        ).read_text(encoding="utf-8")
        self.assertRegex(
            text,
            r"(?s)\|\s*`path`\s*\+\s*`sourceDir`\s*\|"
            r"\s*`sourceSet`\s*\+\s*`metadataPath`\s*\|",
        )
        self.assertIn("legacy_target_removed", text)


    def test_config_dump_info_version_is_documented_as_opaque_platform_state(self) -> None:
        configuration_spec = (
            self.reference_root() / "specs" / "1c-configuration-spec.md"
        ).read_text(encoding="utf-8")

        self.assertIn("`configVersion` — непрозрачное значение платформы", configuration_spec)
        self.assertNotRegex(configuration_spec, r"`configVersion`\s*\|\s*Хеш версии")

    def test_config_dump_info_docs_preserve_same_named_metadata_source(self) -> None:
        docs = [
            self.reference_root() / "tooling" / "v8project.md",
            self.reference_root() / "use-cases" / "metadata-modeling.md",
        ]

        for path in docs:
            text = path.read_text(encoding="utf-8")
            with self.subTest(path=path.relative_to(self.repo_root())):
                self.assertIn("platform-generated CDFI sidecar", text)
                self.assertIn("legitimate metadata descriptor", text)
                self.assertIn("remains source", text)

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

    def test_migrated_skills_do_not_reference_skill_local_operation_scripts(self) -> None:
        forbidden = [
            "powershell.exe",
            ".ps1",
            ".py",
            "Current Python/PowerShell scripts",
            "fallback implementation details",
            "Native execution path",
        ]
        for skill in IN_SCOPE_TOOLS:
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
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


class ExecutableSkillExampleInvariantTests(unittest.TestCase):
    def test_invariant_distinguishes_reader_execution_from_preview(self) -> None:
        invariants = (
            REPO_ROOT / "docs" / "arch-v1" / "architecture" / "invariants.md"
        ).read_text(encoding="utf-8")
        section = invariants.split(
            "### INV-SKILL-EXECUTABLE-EXAMPLES", 1
        )[1].split("\n### ", 1)[0]

        self.assertIn("ADR-0044", section)
        self.assertIn("предпросмотр", section)
        self.assertIn("настоящее чтение MCP", section)
        self.assertIn("детерминирован", section)
        normalized = " ".join(section.casefold().split())
        self.assertNotRegex(
            normalized,
            r"(?:все|кажд\w*)\s+(?:mcp\s+)?(?:пример\w*|вызов\w*)\s+"
            r"(?:—\s+|это\s+)?(?:сух\w*\s+прогон\w*|dry\s*run\w*|предпросмотр\w*)",
        )


class PlatformHelpRoutingTests(unittest.TestCase):
    """Скилл получил источник: отказ перестаёт быть штатным ответом."""

    def setUp(self) -> None:
        self.text = (
            REPO_ROOT / "plugins" / "unica" / "skills" / "platform-help" / "SKILL.md"
        ).read_text(encoding="utf-8")

    def test_routes_platform_questions_to_documentation_search(self) -> None:
        self.assertIn("unica.documentation.search", self.text)

    def test_keeps_the_standards_reading_rule(self) -> None:
        # Секция стандартов может прийти в том же ответе; правило вызова
        # превращается в правило чтения и должно остаться дословным.
        self.assertIn("development-standard", self.text)
        self.assertIn("не закрывает вопрос", self.text)

    def test_contract_gap_is_no_longer_the_default_answer(self) -> None:
        # Отказ сохраняется только для случая, когда ни один поставщик не
        # подтвердил ответ.
        self.assertNotIn(
            "Until it is exposed by public MCP `unica`, report this as a `platform-help` contract gap",
            self.text,
        )

    def test_states_the_source_boundary(self) -> None:
        self.assertIn("версию установки", self.text)

    def test_filters_platform_questions_by_source_kind(self) -> None:
        # ADR-0032 п.5: вопрос об API платформы зовётся с фильтром по смыслу
        # источника, а не полагается на то, что секция стандартов не помешает.
        self.assertIn('"sourceKinds"', self.text)
        self.assertIn('"platform-help"', self.text)

    def test_explains_policy_denied_as_a_user_choice(self) -> None:
        # Запрет сетевого выхода — решение пользователя (unica.toml), и ответ
        # обязан называть его, а не выдавать за сбой площадки.
        self.assertIn("policy-denied", self.text)
        self.assertIn("unica.toml", self.text)

    def test_confirms_answers_with_the_opened_document(self) -> None:
        # ADR-0029 п.4: доказательство — текст открытой страницы, и с
        # ADR-0033 он достижим маршрутом MCP. Проверяются сами примеры, а не
        # подстроки прозы: скилл обязан нести исполнимый вызов
        # unica.documentation.get с обязательным documentId рядом с вызовом
        # search — поток «нашёл → открыл» закреплён формой, а не упоминанием.
        self.assertIn("фрагмент выдачи доказательством не является", self.text)
        calls = [
            json.loads(block)
            for block in re.findall(r"```json\n(.*?)\n```", self.text, flags=re.S)
            if '"method": "tools/call"' in block
        ]
        names = [call["params"]["name"] for call in calls]
        self.assertIn("unica.documentation.search", names)
        self.assertIn("unica.documentation.get", names)
        get_calls = [
            call for call in calls if call["params"]["name"] == "unica.documentation.get"
        ]
        for call in get_calls:
            self.assertIn(
                "documentId",
                call["params"]["arguments"],
                "пример get обязан нести обязательный documentId",
            )

    def test_routes_configuration_domain_questions_to_configuration_help(self) -> None:
        # ADR-0034: доменный вопрос о самой конфигурации — назначение
        # объекта, роль документа в учёте — закрывает её встроенная справка,
        # а не справка платформы; скилл обязан назвать и фильтр, и формат
        # локатора нового корпуса.
        self.assertIn('"configuration-documentation"', self.text)
        self.assertIn("configuration-help:", self.text)

    def test_requires_naming_the_answering_locale(self) -> None:
        # ADR-0029 п.3: подстановка соседней локали разрешена и обязана быть
        # названной в ответе. Данные называют её полем `language` секции, но
        # без правила чтения агент не обязан пересказать подстановку: на
        # русскоязычной установке запрос en возвращает русские страницы молча.
        self.assertIn("`language`", self.text)
        self.assertIn("назовите подстановку локали в ответе", self.text)


# ADR-0049: a bridged reader accepts a logical selector beside its path, so a
# skill whose parameter table still names only the path — or still marks it
# unconditionally required — contradicts the schema the tool publishes.
CONDITIONAL_MARKER = "один из двух"

BRIDGED_SKILL_SELECTORS = {
    "cf-info": ("ConfigPath", False),
    "cf-validate": ("ConfigPath", False),
    "subsystem-info": ("SubsystemPath", True),
    "subsystem-validate": ("SubsystemPath", True),
    "role-info": ("RightsPath", True),
    "role-validate": ("RightsPath", True),
    "form-info": ("FormPath", True),
    "form-validate": ("FormPath", True),
    "dcs-info": ("TemplatePath", True),
    "dcs-validate": ("TemplatePath", True),
    "mxl-info": ("TemplatePath", True),
    "mxl-validate": ("TemplatePath", True),
    "mxl-decompile": ("TemplatePath", True),
}


def _tables(text: str) -> list[list[list[str]]]:
    """Every contiguous run of Markdown table rows, cells stripped of backticks."""
    tables: list[list[list[str]]] = []
    current: list[list[str]] = []
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("|"):
            cells = [cell.strip().strip("`") for cell in stripped.strip("|").split("|")]
            if set("".join(cells)) <= set("-: "):
                continue  # the header separator carries no argument
            current.append(cells)
            continue
        if current:
            tables.append(current)
            current = []
    if current:
        tables.append(current)
    return tables


def _parameter_table(text: str, legacy: str) -> list[list[str]] | None:
    """The one table that documents the tool's target selector."""
    for table in _tables(text):
        if any(row and row[0] == legacy for row in table):
            return table
    return None


class BridgedSkillSelectorDocumentationTests(unittest.TestCase):
    """The parameter table must describe the contract the tool publishes.

    Weak assertions here would pass on a table that dropped the legacy row, or
    marked a selector plainly optional, or lost the exclusivity rule — each of
    which leaves a caller unable to build a valid call from the document.
    """

    def skill_root(self) -> Path:
        return REPO_ROOT / "plugins" / "unica" / "skills"

    def parameter_table(self, skill: str, legacy: str) -> list[list[str]]:
        text = (self.skill_root() / skill / "SKILL.md").read_text(encoding="utf-8")
        table = _parameter_table(text, legacy)
        self.assertIsNotNone(
            table, f"{skill}: не найдена таблица параметров со строкой `{legacy}`"
        )
        return table

    def test_the_parameter_table_carries_every_selector(self) -> None:
        for skill, (legacy, takes_address) in BRIDGED_SKILL_SELECTORS.items():
            with self.subTest(skill=skill):
                table = self.parameter_table(skill, legacy)
                names = {row[0] for row in table if row}
                expected = {legacy, "sourceSet"}
                if takes_address:
                    expected.add("metadataPath")
                self.assertLessEqual(
                    expected,
                    names,
                    f"{skill}: таблица параметров не описывает все селекторы",
                )

    def test_no_selector_is_documented_as_required_or_as_optional(self) -> None:
        for skill, (legacy, takes_address) in BRIDGED_SKILL_SELECTORS.items():
            with self.subTest(skill=skill):
                table = self.parameter_table(skill, legacy)
                if len(table[0]) < 3:
                    continue  # the table has no obligation column to check
                selectors = {legacy, "sourceSet"}
                if takes_address:
                    selectors.add("metadataPath")
                for row in table:
                    if not row or row[0] not in selectors:
                        continue
                    obligation = row[1].lower()
                    self.assertNotIn(
                        obligation,
                        {"да", "нет"},
                        f"{skill}: `{row[0]}` не безусловно обязателен и не просто"
                        " необязателен — он часть взаимоисключающей ветви",
                    )
                    self.assertEqual(
                        obligation,
                        CONDITIONAL_MARKER,
                        f"{skill}: `{row[0]}` обязан нести маркер ветви",
                    )

    def test_each_skill_states_the_exclusivity_rule(self) -> None:
        for skill, (legacy, takes_address) in BRIDGED_SKILL_SELECTORS.items():
            with self.subTest(skill=skill):
                text = (self.skill_root() / skill / "SKILL.md").read_text(
                    encoding="utf-8"
                )
                rule = [
                    line
                    for line in text.splitlines()
                    if line.startswith("Селектор цели ровно один")
                ]
                self.assertTrue(
                    rule, f"{skill}: правило единственного селектора не сформулировано"
                )
                statement = " ".join(
                    text.split("Селектор цели ровно один", 1)[1].splitlines()[:3]
                )
                for name in [legacy, "sourceSet"] + (
                    ["metadataPath"] if takes_address else []
                ):
                    self.assertIn(
                        name,
                        statement,
                        f"{skill}: правило не называет `{name}`",
                    )
                self.assertIn(
                    "selector_conflict",
                    statement,
                    f"{skill}: правило не называет стабильный код отказа",
                )

    def test_subsystem_info_states_that_the_address_is_optional(self) -> None:
        # Its logical branch takes `sourceSet` alone; the schema forbids
        # `metadataPath` in the path branch, so it is neither required nor
        # freely optional.
        table = self.parameter_table("subsystem-info", "SubsystemPath")
        row = next(row for row in table if row and row[0] == "metadataPath")
        description = row[-1].lower()
        self.assertIn(
            "без него",
            description,
            f"subsystem-info: не сказано, что адрес можно опустить: {row}",
        )


if __name__ == "__main__":
    unittest.main()
