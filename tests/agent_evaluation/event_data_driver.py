"""Read-only agent evidence for document data visibility, with live platform help."""
from __future__ import annotations

import copy
import json
from pathlib import Path

from tests.agent_evaluation.dcs_driver import (
    REPO, allowed_help_command, evaluate as evaluate_contract,
    independent_review as review_contract, terminal_calls,
)
from tests.agent_evaluation.mcp_recorder import recorded_calls

FIXTURE = REPO / "tests/fixtures/acceptance/agent-event-data-visibility"
SOURCE_FIXTURE = REPO / "tests/fixtures/acceptance/workspace-event-data-visibility"
MODULE = "main:Document.VisibilityProbe.Module.Object"
SOURCE_FILE = "src/Documents/VisibilityProbe/Ext/ObjectModule.bsl"
HELP_REFS = (
    "skills/object-events/SKILL.md",
    "skills/document-posting/SKILL.md",
    "references/platform/object-events.md",
    "references/platform/document-posting.md",
    "references/platform/transactions-locks.md",
    "skills/platform-help/SKILL.md",
)
DOCUMENTS = {
    "beforeWrite": "platform-syntax-help:syntax-context:objects/catalog125/catalog143/object146/events/BeforeWrite81.html",
    "posting": "platform-syntax-help:syntax-context:objects/catalog125/catalog143/object146/events/Posting61.html",
    "postingMode": "platform-syntax-help:syntax-context:objects/catalog2/catalog1451/DocumentWriteMode/properties/Posting950.html",
}
RUBRIC = (
    "Разбор отвечает исходной жалобе, а не повторяет предположение о задержке на одну запись. "
    "Агент прочитал код, оба поставляемых навыка и официальные страницы линии8.3.27 через Unica, "
    "полные страницы отличены от search snippets, ссылки и версии evidence совпадают с протоколом. "
    "До записи объекта запрос сравнивается с состоянием до этой записи; в начале Posting документ "
    "уже записан в той же транзакции. Отдельное присваивание Marker внутри Posting меняет память "
    "и не доказывает ещё одну запись в БД: агент не обещает безусловного равенства объекта и запроса. "
    "Он не распространяет факт о документе на ещё не записанные движения или видимость другого "
    "сеанса, учитывает возможность блокировки/иного committed состояния. Данные формы отделены "
    "от серверного объекта; форма/конвертация в этой фикстуре не представлены, их корректность "
    "не объявлена проверенной. Причина реальной пользовательской жалобы без наблюдений остаётся "
    "гипотезой, предложены конкретные сравнения ссылки, параметров запроса, объекта и результата "
    "в одной точке, а не исправление кода наугад. runtimeExecuted=false; нет заявления о выполнении "
    "документа, компиляции/загрузке конфигурации или измеренных значениях. Исходники и карта проекта "
    "не менялись. Вердикт о смысле выводов оценивается по исходникам и фактически прочитанным "
    "страницам, а не по совпадению формулировок ответа с навыком."
)


def strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, list):
        for item in value:
            yield from strings(item)
    elif isinstance(value, dict):
        for item in value.values():
            yield from strings(item)


def text_envelopes(records):
    """Body uses a JSON header and a separate literal text content block."""
    records = copy.deepcopy(records)
    for record in records:
        if record["direction"] != "response":
            continue
        result = record["payload"].get("result") or {}
        content = result.get("content") or []
        if "structuredContent" in result or not content or content[0].get("type") != "text":
            continue
        try:
            header = json.loads(content[0]["text"])
        except (KeyError, ValueError):
            continue
        if not isinstance(header, dict) or header.get("ok") is not True:
            continue
        data = header.get("data") or {}
        if data.get("kind") != "Body" or not str(data.get("at", "")).startswith(MODULE):
            continue
        data["bodyText"] = "\n".join(item["text"] for item in content[1:] if item.get("type") == "text")
        result["structuredContent"] = header
    return records


def audit(records):
    allowed, raw_calls = recorded_calls(text_envelopes(records))
    for call in raw_calls:
        name = call["params"]["name"]
        if name not in allowed or name not in {"unica.view", "unica.docs", "unica.task.result"}:
            raise ValueError("event-data agent used a tool outside read-only view/docs")
        before, after = call.get("filesBefore"), call.get("filesAfter")
        if (not isinstance(before, dict) or not before or before != after
                or "v8project.yaml" not in before
                or not any(path.startswith("src/") for path in before)
                or call["before"] != call["after"]):
            raise ValueError("read-only evidence changed or omitted source/project bytes")
        if call.get("rootsBefore") != {"main": "src"} or call.get("rootsAfter") != {"main": "src"}:
            raise ValueError("read-only evidence changed or omitted the source map")
    calls = terminal_calls(raw_calls)
    read_lines, documents = set(), {}
    for call in calls:
        result = call["result"] or {}
        if result.get("ok") is not True or call["isError"]:
            continue
        data = result.get("data") or {}
        if call["params"]["name"] == "unica.view" and str(call["params"]["arguments"].get("at", "")).startswith(MODULE):
            read_lines.update(line.strip() for text in strings(data) for line in text.splitlines() if line.strip())
        document = data.get("document") if call["params"]["name"] == "unica.docs" else None
        if document is not None:
            if (document.get("sourceKind") != "platform-help" or document.get("authority") != "vendor"
                    or not str(document.get("applicableVersion", "")).startswith("8.3.27.")
                    or not document.get("text") or result.get("cursor")):
                raise ValueError("event-data evidence needs full vendor pages of the selected platform line")
            documents[document["documentId"]] = document
    expected = {line.strip() for line in (SOURCE_FIXTURE / SOURCE_FILE).read_text().splitlines() if line.strip()}
    if not expected <= read_lines:
        raise ValueError("agent did not read the complete synthetic object module")
    if not set(DOCUMENTS.values()) <= documents.keys():
        raise ValueError("agent did not open the event and posting-mode platform pages")
    return allowed, raw_calls


def evaluate(server, scenario, proof_root):
    proof = evaluate_contract(
        server, scenario, proof_root, fixture=FIXTURE, auditor=audit,
        changed_paths=set(), source_roots={"main": "src"},
        help_ref=HELP_REFS[0], extra_help_refs=HELP_REFS[1:],
    )
    packet = proof / "plugin"
    events = [json.loads(line) for line in (proof / "agent.jsonl").read_text().splitlines()]
    read_skills = set()
    for event in events:
        item = event.get("item") or {}
        if event.get("type") == "item.completed" and item.get("type") == "command_execution":
            for reference in HELP_REFS[:2]:
                if allowed_help_command(item.get("command", ""), packet / reference):
                    read_skills.add(reference)
    if read_skills != set(HELP_REFS[:2]):
        raise ValueError("agent did not read both candidate workflow skills")
    final = json.loads((proof / "final.json").read_text())
    if final.get("runtimeExecuted") is not False:
        raise ValueError("read-only agent claimed runtime execution")
    documents = {}
    records = [json.loads(line) for line in (proof / "mcp.jsonl").read_text().splitlines()]
    _, calls = audit(records)
    for call in terminal_calls(calls):
        document = ((call.get("result") or {}).get("data") or {}).get("document")
        if document:
            documents[document["documentId"]] = document["applicableVersion"]
    evidence = final.get("evidence")
    if not isinstance(evidence, list) or not evidence:
        raise ValueError("agent answer did not bind conclusions to observed documents")
    for item in evidence:
        if (not isinstance(item, dict) or not item.get("claim")
                or documents.get(item.get("documentId")) != item.get("version")):
            raise ValueError("agent answer cited unobserved documentation or another version")
    return proof


def independent_review(proof):
    # The common reviewer receives MCP evidence, not the CLI transcript. Give
    # it the actual completed help reads too, so it can verify the skill input.
    events = [json.loads(line) for line in (proof / "agent.jsonl").read_text().splitlines()]
    reads = [event["item"] for event in events
             if event.get("type") == "item.completed"
             and event.get("item", {}).get("type") == "command_execution"
             and allowed_help_command(event["item"].get("command", ""),
                                      tuple(proof / "plugin" / ref for ref in HELP_REFS))]
    (proof / "file-observations.json").write_text(json.dumps({"helpReads": reads}, ensure_ascii=False, indent=2))
    review_contract(proof, rubric=RUBRIC)
