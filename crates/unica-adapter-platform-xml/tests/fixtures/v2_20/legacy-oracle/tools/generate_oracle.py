#!/usr/bin/env python3
"""Generate the Platform XML 2.20 parity oracle from legacy tools only."""

from __future__ import annotations

import argparse
import ast
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from build_new_only_contract import build_contract as build_new_only_contract
from extract_enum_contexts import extract as extract_native_enum_contexts


ORACLE_DIR = Path(__file__).resolve().parents[1]
INPUTS_PATH = ORACLE_DIR / "inputs.json"
CROSSWALK_PATH = ORACLE_DIR / "crosswalk.json"
RIGHTS_TARGET_CROSSWALK_PATH = ORACLE_DIR / "rights-target-crosswalk.json"
ENUM_CONTEXTS_PATH = ORACLE_DIR / "enum-source-contexts.json"
ENUM_ALIAS_EXECUTIONS_PATH = ORACLE_DIR / "enum-alias-executions.json"
NEW_ONLY_CONTRACT_PATH = ORACLE_DIR / "new-only-contract.json"
NEW_ONLY_CONTRACT_SOURCE_PATH = ORACLE_DIR / "new-only-contract-source.json"
FULL_PUBLIC_CONTRACT_SPECIMEN_PATH = ORACLE_DIR / "full-public-contract-specimen.json"
PUBLIC_CONTRACT_VARIANT_SPECIMEN_PATH = (
    ORACLE_DIR / "public-contract-variant-specimen.json"
)
ORACLE_PATH = ORACLE_DIR / "legacy-semantic-oracle.json"
MANIFEST_PATH = ORACLE_DIR / "oracle-manifest.json"


def read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def json_bytes(value: Any) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode("utf-8")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def string_value(value: str) -> dict[str, Any]:
    return {"type": "string", "value": value}


def bool_value(value: bool) -> dict[str, Any]:
    return {"type": "boolean", "value": value}


def int_value(value: int) -> dict[str, Any]:
    return {"type": "integer", "value": value}


def enum_value(value: str) -> dict[str, Any]:
    return {"type": "enum", "value": value}


def localized_value(value: str) -> dict[str, Any]:
    return {"type": "localizedString", "value": {"ru": value}}


def type_set_value(variants: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "type": "typeSet",
        "value": {"variants": sorted(variants, key=canonical)},
    }


def empty_reference_value() -> dict[str, Any]:
    return {"type": "emptyReference"}


def node_fact(subject: str, kind: str, name: str) -> dict[str, Any]:
    return {
        "kind": "node",
        "subject": subject,
        "state": "present",
        "value": {"kind": kind, "name": name},
    }


def property_fact(subject: str, predicate: str, value: dict[str, Any]) -> dict[str, Any]:
    return {
        "kind": "property",
        "subject": subject,
        "predicate": predicate,
        "state": "present",
        "value": value,
    }


def relation_fact(
    subject: str,
    predicate: str,
    target: str,
    target_kind: str,
    target_name: str,
) -> dict[str, Any]:
    return {
        "kind": "relation",
        "subject": subject,
        "predicate": predicate,
        "state": "present",
        "value": {
            "target": target,
            "targetKind": target_kind,
            "targetName": target_name,
        },
    }


class FactBuilder:
    def __init__(self, case_id: str) -> None:
        self.case_id = case_id
        self.facts: list[dict[str, Any]] = []
        self.counts: Counter[tuple[str, str]] = Counter()
        self.nodes: dict[tuple[str, str, int], str] = {}

    def root(self, kind: str, name: str) -> str:
        subject = f"{self.case_id}/root"
        self.facts.append(node_fact(subject, kind, name))
        return subject

    def child(self, kind: str, name: str) -> str:
        key = (kind, name)
        self.counts[key] += 1
        ordinal = self.counts[key]
        subject = f"{self.case_id}/{kind}/{name}#{ordinal}"
        self.nodes[(kind, name, ordinal)] = subject
        self.facts.append(node_fact(subject, kind, name))
        return subject

    def prop(self, subject: str, predicate: str, value: dict[str, Any]) -> None:
        self.facts.append(property_fact(subject, predicate, value))

    def relation(
        self,
        subject: str,
        predicate: str,
        target: str,
        target_kind: str,
        target_name: str,
    ) -> None:
        self.facts.append(
            relation_fact(subject, predicate, target, target_kind, target_name)
        )

    def finish(self) -> list[dict[str, Any]]:
        return sorted(self.facts, key=canonical)


def literal_assignment(tree: ast.AST, name: str) -> Any:
    for node in ast.walk(tree):
        if not isinstance(node, (ast.Assign, ast.AnnAssign)):
            continue
        targets = node.targets if isinstance(node, ast.Assign) else [node.target]
        if not any(isinstance(target, ast.Name) and target.id == name for target in targets):
            continue
        try:
            return ast.literal_eval(node.value)
        except (ValueError, TypeError):
            continue
    raise ValueError(f"legacy source has no literal assignment {name}")


def source_tree(repo_root: Path, path: str) -> ast.AST:
    return ast.parse((repo_root / path).read_text(encoding="utf-8-sig"))


def extract_enum_coverage(
    crosswalk: dict[str, Any],
    source_contexts: list[dict[str, Any]],
) -> list[dict[str, Any]]:
    by_source_fact: dict[str, dict[str, Any]] = {}
    for context in source_contexts:
        source_fact = context["sourceFact"]
        if source_fact in by_source_fact:
            raise ValueError(f"duplicate extracted source enum fact {source_fact}")
        by_source_fact[source_fact] = context
    references = [
        source_fact
        for domain in crosswalk["enumDomains"].values()
        for source_fact in domain.get("sourceFacts", [])
    ]
    duplicates = sorted(
        source_fact
        for source_fact, count in Counter(references).items()
        if count != 1
    )
    if duplicates:
        raise ValueError(
            f"source enum contexts are not referenced exactly once: {duplicates}"
        )
    referenced = set(references)
    extracted = set(by_source_fact)
    if referenced != extracted:
        raise ValueError(
            "source enum context inventory is not exact: "
            f"unreferenced={sorted(extracted - referenced)}, "
            f"extra={sorted(referenced - extracted)}"
        )
    coverage: list[dict[str, Any]] = []
    for domain_name, domain in crosswalk["enumDomains"].items():
        forbidden = {"nativeProperty", "objectKinds", "sourceKeys", "extractors"}
        supplied = forbidden.intersection(domain)
        if supplied:
            raise ValueError(
                f"enum domain {domain_name} attempts to override source context: "
                f"{sorted(supplied)}"
            )
        source_facts = domain.get("sourceFacts")
        if not isinstance(source_facts, list) or not source_facts:
            raise ValueError(f"enum domain {domain_name} has no source facts")
        observed: set[str] = set()
        contexts = []
        for source_fact in source_facts:
            context = by_source_fact.get(source_fact)
            if context is None:
                raise ValueError(
                    f"enum domain {domain_name} references absent source fact {source_fact}"
                )
            contexts.append(context)
            observed.update(context["nativeAliases"])
        mapped = set(domain["semanticByAlias"])
        if observed != mapped:
            missing = sorted(observed - mapped)
            extra = sorted(mapped - observed)
            raise ValueError(
                f"enum crosswalk drift for {domain_name}: missing={missing}, extra={extra}"
            )
        for alias in sorted(observed):
            matching_contexts = [
                context for context in contexts if alias in context["nativeAliases"]
            ]
            if not matching_contexts:
                raise ValueError(
                    f"enum alias {domain_name}.{alias} has no extracted source context"
                )
            for context in matching_contexts:
                for object_kind in context["objectKinds"]:
                    coverage.append(
                        {
                            "nativeAlias": alias,
                            "nativeProperty": context["nativeProperty"],
                            "objectKind": object_kind,
                            "semantic": domain["semanticByAlias"][alias],
                            "semanticProperty": domain["semanticProperty"],
                        }
                    )
    unique = {canonical(item): item for item in coverage}
    if len(unique) != len(coverage):
        raise ValueError("legacy enum extraction produced duplicate applicability")
    return [unique[key] for key in sorted(unique)]


def validate_enum_alias_executions(
    executions: dict[str, Any],
    enum_coverage: list[dict[str, Any]],
) -> None:
    if executions.get("schemaVersion") != 1:
        raise ValueError("enum alias execution schema is unsupported")
    rows = executions.get("executions")
    if not isinstance(rows, list) or not rows:
        raise ValueError("enum alias execution inventory is empty")
    fields = (
        "nativeAlias",
        "nativeProperty",
        "objectKind",
        "semantic",
        "semanticProperty",
    )
    expected = Counter(tuple(fact[field] for field in fields) for fact in enum_coverage)
    actual = Counter(tuple(row.get(field) for field in fields) for row in rows)
    if actual != expected:
        raise ValueError(
            "enum alias execution inventory is not exact: "
            f"missing={sorted((expected - actual).elements())}, "
            f"extra={sorted((actual - expected).elements())}"
        )
    for row in rows:
        raw = row.get("rawLegacyOutput")
        raw_hex = row.get("rawOutputHex")
        digest = row.get("rawOutputSha256")
        if (
            not isinstance(row.get("inputXml"), str)
            or not row["inputXml"]
            or not isinstance(raw, str)
            or not raw
            or not isinstance(raw_hex, str)
            or bytes.fromhex(raw_hex).decode("utf-8-sig") != raw
            or digest != sha256(bytes.fromhex(raw_hex))
            or not isinstance(row.get("legacyFacts"), list)
            or not isinstance(row.get("lineClassifications"), list)
        ):
            raise ValueError("enum alias execution has incomplete legacy evidence")


def classify_enum_alias_output(
    context: dict[str, Any],
    native_alias: str,
    baseline: bytes,
    actual: bytes,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    baseline_lines = baseline.decode("utf-8-sig").splitlines()
    actual_lines = actual.decode("utf-8-sig").splitlines()
    ledger = LineLedger(
        f"{context['sourceFact']}:{native_alias}",
        actual_lines,
    )
    facts: list[dict[str, Any]] = []
    for line_number, line in enumerate(actual_lines, 1):
        expected = (
            baseline_lines[line_number - 1]
            if line_number <= len(baseline_lines)
            else None
        )
        if not line.strip():
            ledger.consume(line_number, "structural:blank")
            continue
        classification = (
            f"useful:source-baseline:{context['sourceFact']}:{line_number}"
            if expected is not None and line == expected
            else (
                f"useful:enum-output:{context['nativeProperty']}:"
                f"{native_alias}:{line_number}"
            )
        )
        ledger.consume(line_number, classification)
        facts.append(
            {
                "kind": "legacyOutputLine",
                "lineNumber": line_number,
                "classification": classification,
                "value": line,
            }
        )
    ledger.finish()
    return (
        facts,
        [
            {
                "line": actual_lines[number - 1],
                "lineNumber": number,
                "classification": classification,
            }
            for number, classification in sorted(ledger.classifications.items())
        ],
    )


def build_enum_alias_executions(
    repo_root: Path,
    inputs: dict[str, Any],
    crosswalk: dict[str, Any],
    source_contexts: list[dict[str, Any]],
    enum_coverage: list[dict[str, Any]],
    raw_outputs: dict[str, bytes],
) -> dict[str, Any]:
    all_cases = {
        case["id"]: case
        for case in [*inputs["cases"], *inputs.get("contextCases", [])]
    }
    domains_by_source: dict[str, dict[str, Any]] = {}
    for domain in crosswalk["enumDomains"].values():
        for source_fact in domain["sourceFacts"]:
            if source_fact in domains_by_source:
                raise ValueError(
                    f"enum execution source fact is shared: {source_fact}"
                )
            domains_by_source[source_fact] = domain

    executions: list[dict[str, Any]] = []
    with tempfile.TemporaryDirectory(prefix="unica-enum-alias-oracle-") as temp:
        temp_root = Path(temp)
        ordinal = 0
        for context in sorted(source_contexts, key=lambda item: item["sourceFact"]):
            domain = domains_by_source[context["sourceFact"]]
            evidence_by_owner = {
                evidence["objectKind"]: evidence
                for evidence in context["ownerEvidence"]
            }
            if set(evidence_by_owner) != set(context["objectKinds"]):
                raise ValueError(
                    f"enum execution owner evidence is not exact for "
                    f"{context['sourceFact']}"
                )
            for object_kind in context["objectKinds"]:
                evidence = evidence_by_owner[object_kind]
                source_case = all_cases[evidence["case"]]
                source_path = repo_root / evidence["input"]
                for native_alias in context["nativeAliases"]:
                    semantic = domain["semanticByAlias"].get(native_alias)
                    if semantic is None:
                        raise ValueError(
                            f"enum execution alias {native_alias!r} has no semantic "
                            f"mapping for {context['sourceFact']}"
                        )
                    document = ET.parse(source_path)
                    candidates: list[ET.Element] = []
                    for node in document.getroot().iter():
                        if node.tag.rsplit("}", 1)[-1] != evidence["nativeOwner"]:
                            continue
                        if evidence["ownerUuid"] and (
                            node.attrib.get("uuid") != evidence["ownerUuid"]
                        ):
                            continue
                        properties = next(
                            (
                                child
                                for child in node
                                if child.tag.rsplit("}", 1)[-1] == "Properties"
                            ),
                            None,
                        )
                        if properties is None:
                            continue
                        owner_name = next(
                            (
                                child.text.strip()
                                for child in properties
                                if child.tag.rsplit("}", 1)[-1] == "Name"
                                and child.text
                                and child.text.strip()
                            ),
                            "",
                        )
                        if (
                            not evidence["ownerUuid"]
                            and owner_name != evidence["ownerName"]
                        ):
                            continue
                        candidates.extend(
                            child
                            for child in properties
                            if child.tag.rsplit("}", 1)[-1]
                            == context["nativeProperty"]
                        )
                    if len(candidates) != 1:
                        raise ValueError(
                            f"enum execution property owner is ambiguous for "
                            f"{context['sourceFact']} owner={object_kind}: "
                            f"{len(candidates)} matches"
                        )
                    candidates[0].text = native_alias
                    document_root = document.getroot()
                    document_root.attrib.setdefault("version", "2.20")
                    root_class = document_root.tag.rsplit("}", 1)[-1]
                    descriptor = (
                        next(
                            child
                            for child in document_root
                            if child.tag.rsplit("}", 1)[-1] != "Properties"
                        )
                        if root_class == "MetaDataObject"
                        else document_root
                    )
                    descriptor_properties = next(
                        child
                        for child in descriptor
                        if child.tag.rsplit("}", 1)[-1] == "Properties"
                    )
                    descriptor_name = next(
                        child.text.strip()
                        for child in descriptor_properties
                        if child.tag.rsplit("}", 1)[-1] == "Name"
                        and child.text
                        and child.text.strip()
                    )
                    ordinal += 1
                    target = temp_root / f"enum-alias-{ordinal:04d}.xml"
                    document.write(target, encoding="utf-8", xml_declaration=True)
                    execution_case = dict(source_case)
                    execution_case["id"] = f"enumAlias{ordinal:04d}"
                    execution_case["input"] = str(target)
                    execution_case["adapterInput"] = str(target)
                    execution_case["sourceRoot"] = str(temp_root)
                    execution_case["rawOutput"] = str(
                        temp_root / f"enum-alias-{ordinal:04d}.txt"
                    )
                    raw = run_legacy_case(repo_root, inputs, execution_case)
                    legacy_facts, line_classifications = classify_enum_alias_output(
                        context,
                        native_alias,
                        raw_outputs[evidence["case"]],
                        raw,
                    )
                    raw_text = raw.decode("utf-8-sig")
                    executions.append(
                        {
                            "sourceFact": context["sourceFact"],
                            "nativeAlias": native_alias,
                            "nativeProperty": context["nativeProperty"],
                            "objectKind": object_kind,
                            "semantic": semantic,
                            "semanticProperty": domain["semanticProperty"],
                            "nativeOwner": evidence["nativeOwner"],
                            "ownerName": evidence["ownerName"],
                            "ownerUuid": evidence["ownerUuid"],
                            "inputFileName": f"{descriptor_name}.xml",
                            "inputXml": target.read_text(encoding="utf-8"),
                            "rawLegacyOutput": raw_text,
                            "rawOutputHex": raw.hex(),
                            "rawOutputSha256": sha256(raw),
                            "legacyFacts": legacy_facts,
                            "lineClassifications": line_classifications,
                        }
                    )
    result = {
        "schemaVersion": 1,
        "provenance": (
            "legacy-source-contexts-plus-mutated-native-inputs-and-real-legacy-runs"
        ),
        "executions": sorted(executions, key=canonical),
    }
    validate_enum_alias_executions(result, enum_coverage)
    return result


class LineLedger:
    """Require an explicit, unique classification for every output line."""

    def __init__(self, case_id: str, lines: list[str]) -> None:
        self.case_id = case_id
        self.lines = lines
        self.classifications: dict[int, str] = {}

    def consume(self, line_number: int, classification: str) -> None:
        if line_number in self.classifications:
            previous = self.classifications[line_number]
            raise ValueError(
                f"{self.case_id}:{line_number}: duplicate consumption "
                f"({previous}, {classification})"
            )
        self.classifications[line_number] = classification

    def finish(self) -> None:
        missing = [
            (number, line)
            for number, line in enumerate(self.lines, 1)
            if line.strip() and number not in self.classifications
        ]
        if missing:
            number, line = missing[0]
            raise ValueError(
                f"{self.case_id}:{number}: unmatched legacy output line {line!r}"
            )


SUPPORT_VALUES = {
    "не на поддержке": ("notSupported", False),
    "снято с поддержки (правки свободны)": ("removedFromSupport", False),
    "конфигурация read-only (возможность изменения выключена) — правки невозможны без включения": (
        "configurationReadOnly",
        True,
    ),
    "на замке — прямая правка сломает обновления; дорабатывай через cfe-* либо включи редактирование объекта": (
        "supportedLocked",
        True,
    ),
    "редактируется с сохранением поддержки": ("supportedEditable", True),
}


def add_support_facts(
    builder: FactBuilder,
    subject: str,
    phrase: str,
) -> None:
    try:
        state, active = SUPPORT_VALUES[phrase]
    except KeyError as error:
        raise ValueError(f"unknown support value {phrase!r}") from error
    builder.prop(subject, "support.state", enum_value(state))
    builder.prop(subject, "support.active", bool_value(active))


def validate_meta_output_lines(
    case: dict[str, Any],
    lines: list[str],
    crosswalk: dict[str, Any],
) -> list[dict[str, Any]]:
    ledger = LineLedger(case["id"], lines)
    if not lines:
        raise ValueError(f"legacy output for {case['id']} is empty")
    drill = re.fullmatch(r"(Реквизит|Измерение|Ресурс): ([^:]+)", lines[0])
    if drill:
        ledger.consume(1, "useful:drilldown-header")
        seen_drilldown_fields = set()
        for number, line in enumerate(lines[1:], 2):
            if not line.strip():
                ledger.consume(number, "structural:blank")
                continue
            if not line.startswith("  ") or line.startswith("   "):
                raise ValueError(
                    f"{case['id']}:{number}: malformed drill-down indentation"
                )
            stripped = line.strip()
            patterns = [
                r"Тип: .+",
                r"Обязательный: (?:да|нет)",
                r"Индексирование: (?:нет|Индекс|Индекс с доп\. упорядочиванием)",
                r"Значение заполнения: .+",
                r"Синоним: .+",
                r"Использование: (?:для папок|для папок и элементов)",
                r"Многострочный: (?:да|нет)",
                r"Ведущее: (?:да|нет)",
                r"Основной отбор: (?:да|нет)",
            ]
            matches = sum(re.fullmatch(pattern, stripped) is not None for pattern in patterns)
            if matches != 1:
                raise ValueError(
                    f"{case['id']}:{number}: unrepresentable drill-down line {line!r}"
                )
            field_name = stripped.split(":", 1)[0]
            if field_name in seen_drilldown_fields:
                raise ValueError(
                    f"{case['id']}:{number}: duplicate drill-down field {field_name!r}"
                )
            seen_drilldown_fields.add(field_name)
            if stripped.startswith("Тип:"):
                parse_type(stripped.split(":", 1)[1].strip(), crosswalk)
            ledger.consume(number, "useful:drilldown-property")
        ledger.finish()
        return [
            {"line": line, "lineNumber": number, "classification": classification}
            for number, classification in sorted(ledger.classifications.items())
            for line in [lines[number - 1]]
        ]

    parse_header(lines[0], crosswalk)
    ledger.consume(1, "useful:object-header")
    section: str | None = None
    section_expected: int | None = None
    section_seen = 0
    section_line = 0
    seen_singletons = set()

    def close_section(next_line: int) -> None:
        nonlocal section, section_expected, section_seen, section_line
        if section is not None and section_expected != section_seen:
            raise ValueError(
                f"{case['id']}:{next_line}: section declared at line {section_line} "
                f"contains {section_seen} rows, expected {section_expected}"
            )
        section = None
        section_expected = None
        section_seen = 0
        section_line = 0

    def singleton(key: str, line_number: int) -> None:
        if key in seen_singletons:
            raise ValueError(
                f"{case['id']}:{line_number}: duplicate legacy field {key!r}"
            )
        seen_singletons.add(key)

    for number, line in enumerate(lines[1:], 2):
        stripped = line.strip()
        if not stripped:
            close_section(number)
            ledger.consume(number, "structural:blank")
            continue
        if line[:1].isspace():
            if not line.startswith("  ") or line.startswith("   "):
                raise ValueError(
                    f"{case['id']}:{number}: malformed section indentation"
                )
            if section in {"attribute", "dimension", "resource"}:
                match = re.fullmatch(r"(\S+)\s+(.+?)(?:\s{2,}(\[.*\]))?", stripped)
                if match is None:
                    raise ValueError(
                        f"{case['id']}:{number}: malformed field row {line!r}"
                    )
                parse_type(match.group(2), crosswalk)
                flags = match.group(3)
                if flags:
                    if not flags.startswith("[") or not flags.endswith("]"):
                        raise ValueError(
                            f"{case['id']}:{number}: malformed field flags"
                        )
                    allowed_flags = {
                        "обязательный",
                        "индекс",
                        "индекс+доп",
                        "многострочный",
                        "для папок",
                        "для папок и элементов",
                        "ведущее",
                    }
                    observed_flags = {
                        value.strip()
                        for value in flags[1:-1].split(",")
                        if value.strip()
                    }
                    if not observed_flags or not observed_flags <= allowed_flags:
                        raise ValueError(
                            f"{case['id']}:{number}: unknown field flags "
                            f"{sorted(observed_flags - allowed_flags)}"
                        )
                section_seen += 1
                if section_seen > (section_expected or 0):
                    raise ValueError(
                        f"{case['id']}:{number}: section has more rows than declared"
                    )
                ledger.consume(number, "useful:field-row")
                continue
            if section == "enumerationValue":
                if re.fullmatch(r'(\S+)(?:\s+"([^"]+)")?\s*', stripped) is None:
                    raise ValueError(
                        f"{case['id']}:{number}: malformed enumeration row {line!r}"
                    )
                section_seen += 1
                if section_seen > (section_expected or 0):
                    raise ValueError(
                        f"{case['id']}:{number}: enumeration has more rows than declared"
                    )
                ledger.consume(number, "useful:enumeration-row")
                continue
            if section == "typeList":
                parse_type(stripped, crosswalk)
                section_seen += 1
                if section_seen > (section_expected or 0):
                    raise ValueError(
                        f"{case['id']}:{number}: type section has more rows than declared"
                    )
                ledger.consume(number, "useful:type-row")
                continue
            raise ValueError(
                f"{case['id']}:{number}: indented line outside a declared section"
            )

        close_section(number)
        support = re.fullmatch(r"Поддержка: (.+)", stripped)
        if support:
            if support.group(1) not in SUPPORT_VALUES:
                raise ValueError(
                    f"{case['id']}:{number}: unknown support value {support.group(1)!r}"
                )
            singleton("support", number)
            ledger.consume(number, "useful:support")
            continue
        presentation = re.fullmatch(
            r"(?:Представление типа|Представление объекта|"
            r"Расширенное представление объекта|Представление списка|"
            r"Расширенное представление списка): .+",
            stripped,
        )
        if presentation:
            singleton(stripped.split(":", 1)[0], number)
            ledger.consume(number, "useful:presentation")
            continue
        if re.fullmatch(r"Код\(\d+\)(?: \| Наименование\(\d+\))?", stripped) or re.fullmatch(
            r"Наименование\(\d+\)", stripped
        ):
            singleton("catalog-summary", number)
            ledger.consume(number, "useful:catalog-summary")
            continue
        if stripped.startswith("Номер:"):
            if re.fullmatch(
                r"Номер: (?:Строка|Число)\(\d+\), по "
                r"(?:дню|месяцу|кварталу|году|непериодический), "
                r"(?:авто|не авто) \| Проведение: (?:да|нет)",
                stripped,
            ) is None:
                raise ValueError(
                    f"{case['id']}:{number}: unrepresentable document summary"
                )
            singleton("document-summary", number)
            ledger.consume(number, "useful:document-summary")
            continue
        module_parts = stripped.split(" | ")
        module_values = {
            "Сервер",
            "Обычный клиент",
            "Управляемый клиент",
            "Клиент управляемое",
            "Внешнее соединение",
            "Вызов сервера",
            "Глобальный",
            "Привилегированный",
        }
        if any(part in module_values for part in module_parts) or any(
            part.startswith("Повторное использование:") for part in module_parts
        ):
            for part in module_parts:
                if part in module_values:
                    continue
                if re.fullmatch(
                    r"Повторное использование: (?:на время вызова|на время сеанса|нет)",
                    part,
                ):
                    continue
                raise ValueError(
                    f"{case['id']}:{number}: unknown module summary value {part!r}"
                )
            singleton("module-summary", number)
            ledger.consume(number, "useful:module-summary")
            continue
        if re.fullmatch(
            r"Периодичность: (?:Непериодический|Секунда|День|Месяц|"
            r"Квартал|Год|Позиция регистратора)"
            r"(?: \| Запись: (?:независимая|подчинение регистратору))?",
            stripped,
        ):
            singleton("register-summary", number)
            ledger.consume(number, "useful:register-summary")
            continue
        for prefix, classification in (
            ("Основная СКД:", "useful:main-schema"),
            ("Событие:", "useful:event"),
            ("Обработчик:", "useful:handler"),
        ):
            if stripped.startswith(prefix) and stripped.removeprefix(prefix).strip():
                singleton(prefix, number)
                ledger.consume(number, classification)
                break
        else:
            match = re.fullmatch(r"(Реквизиты|Измерения|Ресурсы) \((\d+)\):", stripped)
            if match:
                section = {
                    "Реквизиты": "attribute",
                    "Измерения": "dimension",
                    "Ресурсы": "resource",
                }[match.group(1)]
                section_expected = int(match.group(2))
                section_seen = 0
                section_line = number
                ledger.consume(number, "structural:field-section")
                continue
            tabular = re.fullmatch(
                r"ТЧ .+ \((\d+) колон(?:ка|ки|ок)\):", stripped
            )
            if tabular:
                section = "attribute"
                section_expected = int(tabular.group(1))
                section_seen = 0
                section_line = number
                ledger.consume(number, "useful:tabular-section")
                continue
            enumeration = re.fullmatch(r"Значения \((\d+)\):", stripped)
            if enumeration:
                section = "enumerationValue"
                section_expected = int(enumeration.group(1))
                section_seen = 0
                section_line = number
                ledger.consume(number, "structural:enumeration-section")
                continue
            types = re.fullmatch(r"(?:Типы|Источники) \((\d+)\):", stripped)
            if types:
                section = "typeList"
                section_expected = int(types.group(1))
                section_seen = 0
                section_line = number
                ledger.consume(number, "structural:type-section")
                continue
            children = re.fullmatch(
                r"(Формы|Макеты|Команды): [^,]+(?:, [^,]+)*", stripped
            )
            if children:
                singleton(children.group(1), number)
                ledger.consume(number, "useful:child-list")
                continue
            if re.fullmatch(r"Ввод на основании: [^,]+(?:, [^,]+)*", stripped):
                ledger.consume(number, "useful:based-on")
                continue
            raise ValueError(
                f"{case['id']}:{number}: unmatched legacy meta-info line {line!r}"
            )
            continue
        continue
    close_section(len(lines) + 1)
    ledger.finish()
    return [
        {"line": line, "lineNumber": number, "classification": classification}
        for number, classification in sorted(ledger.classifications.items())
        for line in [lines[number - 1]]
    ]


def parse_type(raw: str, crosswalk: dict[str, Any]) -> list[dict[str, Any]]:
    variants: list[dict[str, Any]] = []
    unknown_ordinal = 0
    for item in [part.strip() for part in raw.split(" | ") if part.strip()]:
        match = re.fullmatch(r"Строка\((\d+)\)", item)
        if match:
            variants.append(
                {
                    "kind": "primitive",
                    "primitive": "string",
                    "length": int(match.group(1)),
                }
            )
            continue
        match = re.fullmatch(r"Число\((\d+),(\d+)\)", item)
        if match:
            variants.append(
                {
                    "kind": "primitive",
                    "primitive": "number",
                    "digits": int(match.group(1)),
                    "fractionDigits": int(match.group(2)),
                }
            )
            continue
        primitive = {
            "Булево": ("boolean", None),
            "Дата": ("date", "date"),
            "ДатаВремя": ("date", "dateTime"),
            "УникальныйИдентификатор": ("uuid", None),
            "ХранилищеЗначения": ("opaque", None),
            "Null": ("null", None),
            "v8:ValueTable": ("table", None),
        }.get(item)
        if primitive is not None:
            value: dict[str, Any] = {"kind": "primitive", "primitive": primitive[0]}
            if primitive[1] is not None:
                value["dateFractions"] = primitive[1]
            variants.append(value)
            continue
        if "." in item:
            prefix, name = item.split(".", 1)
            target = crosswalk["typeTargets"].get(prefix)
            if target is not None:
                variants.append(
                    {
                        "kind": target[0],
                        "targetKind": target[1],
                        "targetName": name,
                    }
                )
                continue
        unknown_ordinal += 1
        variants.append({"kind": "unknown", "ordinal": unknown_ordinal})
    if not variants:
        raise ValueError(f"legacy type output is not parseable: {raw!r}")
    return variants


def parse_header(line: str, crosswalk: dict[str, Any]) -> tuple[str, str, str | None]:
    match = re.fullmatch(
        r'=== (.+?): (.+?)(?: (?:—|---) "([^"]+)")? ===',
        line,
    )
    if match is None:
        raise ValueError(f"legacy output has an unrecognized header: {line!r}")
    label, name, synonym = match.groups()
    kind = crosswalk["headerKinds"].get(label)
    if kind is None:
        raise ValueError(f"legacy output has an unreviewed header kind: {label!r}")
    return kind, name, synonym


def relation_for_child(kind: str) -> str:
    return {
        "attribute": "attributes",
        "dimension": "dimensions",
        "resource": "resources",
        "tabularSection": "tabularSections",
        "enumerationValue": "enumValues",
        "form": "forms",
        "template": "templates",
        "command": "commands",
    }[kind]


def add_child(
    builder: FactBuilder,
    parent: str,
    kind: str,
    name: str,
    relation: str | None = None,
) -> str:
    subject = builder.child(kind, name)
    builder.relation(
        parent,
        relation or relation_for_child(kind),
        subject,
        kind,
        name,
    )
    return subject


def add_field_facts(
    builder: FactBuilder,
    subject: str,
    raw_type: str,
    flags: str,
    crosswalk: dict[str, Any],
) -> None:
    builder.prop(subject, "field.type", type_set_value(parse_type(raw_type, crosswalk)))
    builder.prop(
        subject,
        "field.required",
        bool_value("обязательный" in flags),
    )
    indexing = "dontIndex"
    if "индекс+доп" in flags:
        indexing = "indexWithAdditionalOrder"
    elif "индекс" in flags:
        indexing = "index"
    builder.prop(subject, "field.indexing", enum_value(indexing))
    if "для папок и элементов" in flags:
        builder.prop(subject, "field.use", enum_value("groupsAndItems"))
    elif "для папок" in flags:
        builder.prop(subject, "field.use", enum_value("groupOnly"))
    if "многострочный" in flags:
        builder.prop(subject, "field.multiLine", bool_value(True))
    if "ведущее" in flags:
        builder.prop(subject, "field.master", bool_value(True))


def parse_meta_output(
    case: dict[str, Any],
    raw: bytes,
    crosswalk: dict[str, Any],
) -> dict[str, Any]:
    lines = raw.decode("utf-8-sig").splitlines()
    if not lines:
        raise ValueError(f"legacy output for {case['id']} is empty")
    classifications = validate_meta_output_lines(case, lines, crosswalk)
    drill_header = re.fullmatch(r"(Реквизит|Измерение|Ресурс): (.+)", lines[0])
    if drill_header is not None:
        kind = {
            "Реквизит": "attribute",
            "Измерение": "dimension",
            "Ресурс": "resource",
        }[drill_header.group(1)]
        builder = FactBuilder(case["id"])
        selected_field = builder.child(kind, drill_header.group(2))
        for line in lines[1:]:
            stripped = line.strip()
            if stripped.startswith("Тип:"):
                builder.prop(
                    selected_field,
                    "field.type",
                    type_set_value(
                        parse_type(stripped.split(":", 1)[1].strip(), crosswalk)
                    ),
                )
            elif stripped.startswith("Обязательный:"):
                builder.prop(
                    selected_field,
                    "field.required",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Индексирование:"):
                raw_indexing = stripped.split(":", 1)[1].strip()
                builder.prop(
                    selected_field,
                    "field.indexing",
                    enum_value(
                        {
                            "нет": "dontIndex",
                            "Индекс": "index",
                            "Индекс с доп. упорядочиванием": "indexWithAdditionalOrder",
                        }.get(raw_indexing, raw_indexing)
                    ),
                )
            elif stripped.startswith("Значение заполнения:"):
                fill = stripped.split(":", 1)[1].strip()
                if fill == "Пустая ссылка":
                    builder.prop(selected_field, "field.fillValue", empty_reference_value())
                elif fill != "—":
                    builder.prop(selected_field, "field.fillValue", string_value(fill))
            elif stripped.startswith("Синоним:"):
                builder.prop(
                    selected_field,
                    "metadata.synonym",
                    localized_value(stripped.split(":", 1)[1].strip()),
                )
            elif stripped.startswith("Использование:"):
                use = stripped.split(":", 1)[1].strip()
                builder.prop(
                    selected_field,
                    "field.use",
                    enum_value(
                        {
                            "для папок": "groupOnly",
                            "для папок и элементов": "groupsAndItems",
                        }.get(use, use)
                    ),
                )
            elif stripped.startswith("Многострочный:"):
                builder.prop(
                    selected_field,
                    "field.multiLine",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Ведущее:"):
                builder.prop(
                    selected_field,
                    "field.master",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Основной отбор:"):
                builder.prop(
                    selected_field,
                    "field.mainFilter",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped:
                raise ValueError(f"unreviewed drill-down output: {stripped!r}")
        return {
            "id": case["id"],
            "profile": case["profile"],
            "parentCase": case["parentCase"],
            "input": case["input"],
            "adapterInput": case.get("adapterInput", case["input"]),
            "sourceRoot": case["sourceRoot"],
            "rawOutput": case["rawOutput"],
            "rootKind": None,
            "rootName": None,
            "facts": builder.finish(),
            "lineClassifications": classifications,
        }
    root_kind, root_name, root_synonym = parse_header(lines[0], crosswalk)
    builder = FactBuilder(case["id"])
    root = builder.root(root_kind, root_name)
    if root_synonym:
        builder.prop(root, "metadata.synonym", localized_value(root_synonym))

    current_section: str | None = None
    section_parent = root
    selected_field: str | None = None
    type_list: list[dict[str, Any]] = []
    type_list_property: str | None = None

    for line in lines[1:]:
        stripped = line.strip()
        if not stripped:
            current_section = None
            continue
        if stripped.startswith("Поддержка:"):
            support = stripped.split(":", 1)[1].strip()
            add_support_facts(builder, root, support)
            continue
        for prefix, property_id in (
            ("Представление типа:", "presentation.type"),
            ("Представление объекта:", "presentation.object"),
            ("Расширенное представление объекта:", "presentation.extendedObject"),
            ("Представление списка:", "presentation.list"),
            ("Расширенное представление списка:", "presentation.extendedList"),
        ):
            if stripped.startswith(prefix):
                builder.prop(
                    root,
                    property_id,
                    localized_value(stripped.split(":", 1)[1].strip()),
                )
                break
        else:
            pass
        if stripped.startswith(
            (
                "Представление типа:",
                "Представление объекта:",
                "Расширенное представление объекта:",
                "Представление списка:",
                "Расширенное представление списка:",
            )
        ):
            continue

        if stripped.startswith("Код(") or "Наименование(" in stripped:
            code = re.search(r"Код\((\d+)\)", stripped)
            description = re.search(r"Наименование\((\d+)\)", stripped)
            if code:
                builder.prop(root, "catalog.code.length", int_value(int(code.group(1))))
            if description:
                builder.prop(
                    root,
                    "catalog.description.length",
                    int_value(int(description.group(1))),
                )
            continue

        if stripped.startswith("Номер:"):
            match = re.fullmatch(
                r"Номер: (Строка|Число)\((\d+)\), по (дню|месяцу|кварталу|году|непериодический), (авто|не авто) \| Проведение: (да|нет)",
                stripped,
            )
            if match is None:
                raise ValueError(f"unreviewed document summary: {stripped!r}")
            number_type, length, periodicity, automatic, posting = match.groups()
            builder.prop(
                root,
                "document.number.type",
                enum_value("string" if number_type == "Строка" else "number"),
            )
            builder.prop(root, "document.number.length", int_value(int(length)))
            builder.prop(
                root,
                "document.number.periodicity",
                enum_value(
                    {
                        "дню": "day",
                        "месяцу": "month",
                        "кварталу": "quarter",
                        "году": "year",
                        "непериодический": "nonperiodical",
                    }[periodicity]
                ),
            )
            builder.prop(
                root,
                "document.number.auto",
                bool_value(automatic == "авто"),
            )
            builder.prop(
                root,
                "document.posting.mode",
                enum_value("allow" if posting == "да" else "deny"),
            )
            continue

        module_contexts = {
            "Сервер": "module.server",
            "Обычный клиент": "module.clientOrdinaryApplication",
            "Управляемый клиент": "module.clientManagedApplication",
            "Клиент управляемое": "module.clientManagedApplication",
            "Внешнее соединение": "module.externalConnection",
            "Вызов сервера": "module.serverCall",
            "Глобальный": "module.global",
            "Привилегированный": "module.privileged",
        }
        if any(
            part in module_contexts or part.startswith("Повторное использование:")
            for part in stripped.split(" | ")
        ):
            for part in stripped.split(" | "):
                if part in module_contexts:
                    builder.prop(root, module_contexts[part], bool_value(True))
                elif part.startswith("Повторное использование:"):
                    reuse = part.split(":", 1)[1].strip()
                    builder.prop(
                        root,
                        "module.returnValuesReuse",
                        enum_value(
                            {
                                "нет": "dontUse",
                                "на время вызова": "duringRequest",
                                "на время сеанса": "duringSession",
                            }[reuse]
                        ),
                    )
                else:
                    raise ValueError(
                        f"{case['id']}: unreviewed module summary value {part!r}"
                    )
            continue

        if stripped.startswith("Периодичность:"):
            parts = [part.strip() for part in stripped.split(" | ")]
            periodicity = parts[0].split(":", 1)[1].strip()
            builder.prop(
                root,
                "register.periodicity",
                enum_value(
                    {
                        "Непериодический": "nonperiodical",
                        "Секунда": "second",
                        "День": "day",
                        "Месяц": "month",
                        "Квартал": "quarter",
                        "Год": "year",
                        "Позиция регистратора": "recorderPosition",
                    }.get(periodicity, periodicity)
                ),
            )
            if len(parts) > 1 and parts[1].startswith("Запись:"):
                write_mode = parts[1].split(":", 1)[1].strip()
                builder.prop(
                    root,
                    "register.writeMode",
                    enum_value(
                        {
                            "независимая": "independent",
                            "подчинение регистратору": "recorderSubordinate",
                        }.get(write_mode, write_mode)
                    ),
                )
            continue

        if stripped.startswith("Основная СКД:"):
            builder.prop(
                root,
                "report.mainDataCompositionSchema",
                string_value(stripped.split(":", 1)[1].strip()),
            )
            continue
        if stripped.startswith("Событие:"):
            builder.prop(
                root,
                "subscription.event",
                string_value(stripped.split(":", 1)[1].strip()),
            )
            continue
        if stripped.startswith("Обработчик:"):
            builder.prop(
                root,
                "subscription.handler",
                string_value(stripped.split(":", 1)[1].strip()),
            )
            continue

        match = re.fullmatch(r"(Реквизиты|Измерения|Ресурсы) \(\d+\):", stripped)
        if match:
            current_section = {
                "Реквизиты": "attribute",
                "Измерения": "dimension",
                "Ресурсы": "resource",
            }[match.group(1)]
            section_parent = root
            continue
        match = re.fullmatch(r"ТЧ (.+?) \(\d+ колон(?:ка|ки|ок)\):", stripped)
        if match:
            section_parent = add_child(builder, root, "tabularSection", match.group(1))
            current_section = "attribute"
            continue
        if re.fullmatch(r"Значения \(\d+\):", stripped):
            current_section = "enumerationValue"
            section_parent = root
            continue
        if re.fullmatch(r"Типы \(\d+\):", stripped):
            current_section = "typeList"
            type_list_property = "definedType.type"
            continue
        if re.fullmatch(r"Источники \(\d+\):", stripped):
            current_section = "typeList"
            type_list_property = "subscription.source.type"
            continue

        match = re.fullmatch(r"(Реквизит|Измерение|Ресурс): (.+)", stripped)
        if match:
            kind = {
                "Реквизит": "attribute",
                "Измерение": "dimension",
                "Ресурс": "resource",
            }[match.group(1)]
            selected_field = add_child(builder, root, kind, match.group(2))
            current_section = "drilldown"
            continue
        if current_section == "drilldown" and selected_field is not None:
            if stripped.startswith("Тип:"):
                builder.prop(
                    selected_field,
                    "field.type",
                    type_set_value(
                        parse_type(stripped.split(":", 1)[1].strip(), crosswalk)
                    ),
                )
            elif stripped.startswith("Обязательный:"):
                builder.prop(
                    selected_field,
                    "field.required",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Индексирование:"):
                raw_indexing = stripped.split(":", 1)[1].strip()
                builder.prop(
                    selected_field,
                    "field.indexing",
                    enum_value(
                        {
                            "нет": "dontIndex",
                            "Индекс": "index",
                            "Индекс с доп. упорядочиванием": "indexWithAdditionalOrder",
                        }.get(raw_indexing, raw_indexing)
                    ),
                )
            elif stripped.startswith("Значение заполнения:"):
                fill = stripped.split(":", 1)[1].strip()
                if fill == "Пустая ссылка":
                    builder.prop(selected_field, "field.fillValue", empty_reference_value())
                elif fill != "—":
                    builder.prop(selected_field, "field.fillValue", string_value(fill))
            elif stripped.startswith("Синоним:"):
                builder.prop(
                    selected_field,
                    "metadata.synonym",
                    localized_value(stripped.split(":", 1)[1].strip()),
                )
            elif stripped.startswith("Использование:"):
                use = stripped.split(":", 1)[1].strip()
                builder.prop(
                    selected_field,
                    "field.use",
                    enum_value(
                        {
                            "для папок": "groupOnly",
                            "для папок и элементов": "groupsAndItems",
                        }.get(use, use)
                    ),
                )
            elif stripped.startswith("Многострочный:"):
                builder.prop(
                    selected_field,
                    "field.multiLine",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Ведущее:"):
                builder.prop(
                    selected_field,
                    "field.master",
                    bool_value(stripped.endswith("да")),
                )
            elif stripped.startswith("Основной отбор:"):
                builder.prop(
                    selected_field,
                    "field.mainFilter",
                    bool_value(stripped.endswith("да")),
                )
            else:
                raise ValueError(
                    f"{case['id']}: unreviewed drill-down property {line!r}"
                )
            continue

        if current_section in {"attribute", "dimension", "resource"} and line.startswith("  "):
            field = re.fullmatch(r"(\S+)\s+(.+?)(?:\s{2,}(\[.*\]))?", stripped)
            if field is None:
                raise ValueError(f"unreviewed field line: {line!r}")
            name, raw_type, flags = field.groups()
            subject = add_child(
                builder,
                section_parent,
                current_section,
                name,
                "columns" if section_parent != root else None,
            )
            add_field_facts(builder, subject, raw_type, flags or "", crosswalk)
            continue
        if current_section == "enumerationValue" and line.startswith("  "):
            match = re.fullmatch(r'(\S+)(?:\s+"([^"]+)")?\s*', stripped)
            if match is None:
                raise ValueError(f"unreviewed enumeration value line: {line!r}")
            subject = add_child(builder, root, "enumerationValue", match.group(1))
            if match.group(2):
                builder.prop(
                    subject,
                    "metadata.synonym",
                    localized_value(match.group(2)),
                )
            continue
        if current_section == "typeList" and line.startswith("  "):
            type_list.extend(parse_type(stripped, crosswalk))
            continue

        match = re.fullmatch(r"(Формы|Макеты|Команды): (.+)", stripped)
        if match:
            kind = {"Формы": "form", "Макеты": "template", "Команды": "command"}[
                match.group(1)
            ]
            for name in [value.strip() for value in match.group(2).split(",")]:
                add_child(builder, root, kind, name)
            continue
        if stripped.startswith("Ввод на основании:"):
            for name in [
                value.strip()
                for value in stripped.split(":", 1)[1].split(",")
                if value.strip()
            ]:
                target = f"{case['id']}/external/unknown/{name}"
                builder.relation(root, "basedOn", target, "unknown", name)
            continue
        raise ValueError(
            f"{case['id']}: unmatched legacy meta-info line {line!r}"
        )

    if type_list:
        if type_list_property is None:
            raise ValueError("legacy type list has no reviewed semantic property")
        builder.prop(root, type_list_property, type_set_value(type_list))
    return {
        "id": case["id"],
        "profile": case["profile"],
        "input": case["input"],
        "adapterInput": case.get("adapterInput", case["input"]),
        "sourceRoot": case["sourceRoot"],
        "rawOutput": case["rawOutput"],
        "rootKind": root_kind,
        "rootName": root_name,
        "facts": builder.finish(),
        "lineClassifications": classifications,
    }


def parse_role_output(
    case: dict[str, Any],
    raw: bytes,
    crosswalk: dict[str, Any],
    target_crosswalk: dict[str, str],
) -> dict[str, Any]:
    lines = raw.decode("utf-8-sig").splitlines()
    if not lines:
        raise ValueError(f"legacy role output for {case['id']} is empty")
    ledger = LineLedger(case["id"], lines)
    root_kind, root_name, root_synonym = parse_header(lines[0], crosswalk)
    ledger.consume(1, "useful:role-header")
    builder = FactBuilder(case["id"])
    root = builder.root(root_kind, root_name)
    if root_synonym:
        builder.prop(root, "metadata.synonym", localized_value(root_synonym))

    mode: str | None = None
    restricted_targets: set[str] = set()
    permission_counts: Counter[tuple[str, str, str]] = Counter()
    observed_totals = Counter()
    allowed_total = denied_total = None
    saw_properties = False
    saw_separator = False
    saw_allowed_section = False
    index = 1

    def add_target_line(
        line_number: int,
        line: str,
        target_prefix: str,
        target_kind: str,
        permission_mode: str,
    ) -> None:
        if not line.startswith("    ") or line.startswith("     "):
            raise ValueError(
                f"{case['id']}:{line_number}: malformed right target indentation"
            )
        match = re.fullmatch(r"    ([^:]+): (.+)", line)
        if match is None:
            raise ValueError(
                f"{case['id']}:{line_number}: malformed right target line {line!r}"
            )
        target_name, rights_raw = match.groups()
        if "." in target_name:
            raise ValueError(
                f"{case['id']}:{line_number}: target line repeats or changes its group prefix"
            )
        target_identity = f"{case['id']}/external/{target_kind}/{target_name}"
        parsed_permissions = []
        for raw_permission in rights_raw.split(","):
            item = raw_permission.strip()
            restricted = item.endswith(" [RLS]")
            item = item.removesuffix(" [RLS]")
            if permission_mode == "denied":
                if not item.startswith("-"):
                    raise ValueError(
                        f"{case['id']}:{line_number}: denied right lacks '-' marker"
                    )
                item = item.removeprefix("-")
            elif item.startswith("-"):
                raise ValueError(
                    f"{case['id']}:{line_number}: allowed right has '-' marker"
                )
            if not re.fullmatch(r"[^,\s][^,]*", item):
                raise ValueError(
                    f"{case['id']}:{line_number}: malformed right name {item!r}"
                )
            parsed_permissions.append((item, restricted))
        if not parsed_permissions:
            raise ValueError(f"{case['id']}:{line_number}: right target has no rights")
        ledger.consume(line_number, f"useful:right-target:{target_prefix}")

        if any(restricted for _, restricted in parsed_permissions):
            if target_identity not in restricted_targets:
                builder.facts.append(node_fact(target_identity, target_kind, target_name))
                builder.prop(
                    target_identity,
                    "access.restriction.present",
                    bool_value(True),
                )
                restricted_targets.add(target_identity)
        for permission_name, _ in parsed_permissions:
            allowed = permission_mode == "allowed"
            observed_totals[permission_mode] += 1
            key = (target_kind, target_name, permission_name)
            permission_counts[key] += 1
            permission = (
                f"{case['id']}/accessPermission/"
                f"{target_kind}:{target_name}:{permission_name}"
                f"#{permission_counts[key]}"
            )
            builder.facts.append(
                node_fact(permission, "accessPermission", permission_name)
            )
            builder.prop(
                permission,
                "access.permission.name",
                string_value(permission_name),
            )
            builder.prop(
                permission,
                "access.permission.allowed",
                bool_value(allowed),
            )
            builder.relation(
                root,
                "accessPermissions",
                permission,
                "accessPermission",
                permission_name,
            )
            builder.relation(
                permission,
                "accessTarget",
                target_identity,
                target_kind,
                target_name,
            )

    while index < len(lines):
        line_number = index + 1
        line = lines[index]
        stripped = line.strip()
        if not stripped:
            ledger.consume(line_number, "structural:blank")
            index += 1
            continue
        support = re.fullmatch(r"Поддержка: (.+)", stripped)
        if support:
            if support.group(1) not in SUPPORT_VALUES:
                raise ValueError(
                    f"{case['id']}:{line_number}: unknown support value {support.group(1)!r}"
                )
            add_support_facts(builder, root, support.group(1))
            ledger.consume(line_number, "useful:support")
            index += 1
            continue
        properties = re.fullmatch(
            r"Properties: setForNewObjects=(true|false), "
            r"setForAttributesByDefault=(true|false), "
            r"independentRightsOfChildObjects=(true|false)",
            stripped,
        )
        if properties:
            if saw_properties:
                raise ValueError(
                    f"{case['id']}:{line_number}: duplicate role Properties line"
                )
            saw_properties = True
            for semantic, value in zip(
                (
                    "access.newObjects.defaultAllowed",
                    "access.attributes.defaultAllowed",
                    "access.childObjects.independent",
                ),
                properties.groups(),
            ):
                builder.prop(root, semantic, bool_value(value == "true"))
            ledger.consume(line_number, "useful:role-properties")
            index += 1
            continue
        if stripped == "Allowed rights:":
            if saw_allowed_section:
                raise ValueError(
                    f"{case['id']}:{line_number}: duplicate Allowed rights section"
                )
            saw_allowed_section = True
            mode = "allowed"
            ledger.consume(line_number, "structural:allowed-section")
            index += 1
            continue
        if stripped == "Denied rights:":
            mode = "denied"
            ledger.consume(line_number, "structural:denied-section")
            index += 1
            continue
        if stripped == "(no allowed rights)":
            mode = None
            saw_allowed_section = True
            ledger.consume(line_number, "useful:no-allowed-rights")
            index += 1
            continue
        denied_summary = re.fullmatch(
            r"Denied: (\d+) rights \(use -ShowDenied to list\)", stripped
        )
        if denied_summary:
            denied_total = int(denied_summary.group(1))
            mode = None
            ledger.consume(line_number, "useful:denied-summary")
            index += 1
            continue
        group = re.fullmatch(r"([A-Za-z][A-Za-z0-9]*) \((\d+)\):", stripped)
        if group:
            if line != f"  {stripped}":
                raise ValueError(
                    f"{case['id']}:{line_number}: malformed right group indentation"
                )
            if mode not in {"allowed", "denied"}:
                raise ValueError(
                    f"{case['id']}:{line_number}: right group outside a rights section"
                )
            target_prefix, target_count_raw = group.groups()
            target_kind = target_crosswalk.get(target_prefix)
            if target_kind is None:
                raise ValueError(
                    f"{case['id']}:{line_number}: unhandled right target prefix "
                    f"{target_prefix!r}"
                )
            target_count = int(target_count_raw)
            if target_count <= 0:
                raise ValueError(
                    f"{case['id']}:{line_number}: empty right target group"
                )
            ledger.consume(line_number, f"structural:right-group:{target_prefix}")
            index += 1
            for _ in range(target_count):
                if index >= len(lines):
                    raise ValueError(
                        f"{case['id']}:{line_number}: truncated right target group"
                    )
                add_target_line(
                    index + 1,
                    lines[index],
                    target_prefix,
                    target_kind,
                    mode,
                )
                index += 1
            if index < len(lines) and lines[index].startswith("    "):
                raise ValueError(
                    f"{case['id']}:{index + 1}: group contains more targets than declared"
                )
            continue
        if line[:1].isspace():
            raise ValueError(
                f"{case['id']}:{line_number}: right target appears without a local group"
            )
        rls = re.fullmatch(r"RLS: (\d+) restrictions", stripped)
        if rls:
            builder.prop(
                root,
                "access.restriction.count",
                int_value(int(rls.group(1))),
            )
            ledger.consume(line_number, "useful:restriction-count")
            mode = None
            index += 1
            continue
        templates = re.fullmatch(r"Templates: ([^,]+(?:, [^,]+)*)", stripped)
        if templates:
            for name in templates.group(1).split(", "):
                template = add_child(
                    builder,
                    root,
                    "accessRestrictionTemplate",
                    name,
                    "restrictionTemplates",
                )
                builder.prop(
                    template,
                    "access.restrictionTemplate.name",
                    string_value(name),
                )
            ledger.consume(line_number, "useful:restriction-templates")
            mode = None
            index += 1
            continue
        if stripped == "---":
            if saw_separator:
                raise ValueError(
                    f"{case['id']}:{line_number}: duplicate totals delimiter"
                )
            saw_separator = True
            ledger.consume(line_number, "structural:totals-delimiter")
            mode = None
            index += 1
            continue
        total = re.fullmatch(r"Total: (\d+) allowed, (\d+) denied", stripped)
        if total:
            if not saw_separator:
                raise ValueError(
                    f"{case['id']}:{line_number}: totals precede delimiter"
                )
            allowed_total, parsed_denied = map(int, total.groups())
            if denied_total is not None and denied_total != parsed_denied:
                raise ValueError(
                    f"{case['id']}:{line_number}: denied summary disagrees with totals"
                )
            denied_total = parsed_denied
            builder.prop(root, "access.allowed.count", int_value(allowed_total))
            builder.prop(root, "access.denied.count", int_value(denied_total))
            ledger.consume(line_number, "useful:right-totals")
            index += 1
            continue
        raise ValueError(
            f"{case['id']}:{line_number}: unmatched legacy role-info line {line!r}"
        )

    ledger.finish()
    if not saw_properties or not saw_allowed_section:
        raise ValueError(f"legacy role output for {case['id']} is structurally incomplete")
    if allowed_total is None or denied_total is None:
        raise ValueError("legacy role output has no totals")
    if observed_totals["allowed"] != allowed_total:
        raise ValueError(
            f"legacy role allowed total mismatch: parsed={observed_totals['allowed']} "
            f"declared={allowed_total}"
        )
    if "-ShowDenied" in case.get("arguments", []):
        if observed_totals["denied"] != denied_total:
            raise ValueError(
                f"legacy role denied total mismatch: parsed={observed_totals['denied']} "
                f"declared={denied_total}"
            )
    return {
        "id": case["id"],
        "profile": case["profile"],
        "input": case["input"],
        "adapterInput": case["adapterInput"],
        "sourceRoot": case["sourceRoot"],
        "rawOutput": case["rawOutput"],
        "rootKind": root_kind,
        "rootName": root_name,
        "facts": builder.finish(),
        "lineClassifications": [
            {
                "line": lines[number - 1],
                "lineNumber": number,
                "classification": classification,
            }
            for number, classification in sorted(ledger.classifications.items())
        ],
    }


def run_legacy_case(
    repo_root: Path,
    inputs: dict[str, Any],
    case: dict[str, Any],
) -> bytes:
    script = repo_root / inputs["referenceSources"][case["tool"]]
    input_path = repo_root / case["input"]
    with tempfile.TemporaryDirectory(prefix="unica-legacy-oracle-") as temp:
        output = Path(temp) / "legacy-output.txt"
        command = [
            sys.executable,
            str(script),
            "-Path",
            str(input_path),
            *case["arguments"],
            "-Limit",
            "0",
            "-OutFile",
            str(output),
        ]
        result = subprocess.run(
            command,
            cwd=repo_root,
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"legacy command failed for {case['id']}: {result.stderr}"
            )
        return output.read_bytes()


def provenance_entries(
    repo_root: Path,
    inputs: dict[str, Any],
    raw_outputs: dict[str, bytes],
    enum_contexts_data: bytes,
    enum_alias_executions_data: bytes,
    oracle_data: bytes,
    new_only_contract_data: bytes,
) -> list[dict[str, str]]:
    entries: dict[tuple[str, str], dict[str, str]] = {}

    def add(role: str, path: Path, data: bytes | None = None) -> None:
        relative = path.relative_to(repo_root).as_posix()
        payload = path.read_bytes() if data is None else data
        entries[(role, relative)] = {
            "role": role,
            "path": relative,
            "sha256": sha256(payload),
        }

    add("oracleGenerator", Path(__file__).resolve())
    add("enumSourceExtractor", Path(__file__).resolve().with_name("extract_enum_contexts.py"))
    add(
        "newOnlyContractBuilder",
        Path(__file__).resolve().with_name("build_new_only_contract.py"),
    )
    add("oracleInputs", INPUTS_PATH)
    add("independentCrosswalk", CROSSWALK_PATH)
    add("rightsTargetCrosswalk", RIGHTS_TARGET_CROSSWALK_PATH)
    add("fullPublicContractSpecimen", FULL_PUBLIC_CONTRACT_SPECIMEN_PATH)
    add(
        "publicContractVariantSpecimen",
        PUBLIC_CONTRACT_VARIANT_SPECIMEN_PATH,
    )
    add("newOnlyContractSource", NEW_ONLY_CONTRACT_SOURCE_PATH)
    add("newOnlyContract", NEW_ONLY_CONTRACT_PATH, new_only_contract_data)
    for path in inputs["referenceSources"].values():
        add("legacyReferenceSource", repo_root / path)
    for path in inputs.get("contractInputs", []):
        add("newOnlyContractInput", repo_root / path)
    for case in [*inputs["cases"], *inputs.get("contextCases", [])]:
        add("legacyInputFixture", repo_root / case["input"])
        if case.get("adapterInput") and case["adapterInput"] != case["input"]:
            add("legacyInputFixture", repo_root / case["adapterInput"])
        for artifact in case.get("inputArtifacts", []):
            add("legacyInputFixture", repo_root / artifact)
        add(
            "rawLegacyOutput",
            repo_root / case["rawOutput"],
            raw_outputs[case["id"]],
        )
    add("enumSourceContexts", ENUM_CONTEXTS_PATH, enum_contexts_data)
    add(
        "enumAliasExecutions",
        ENUM_ALIAS_EXECUTIONS_PATH,
        enum_alias_executions_data,
    )
    add("legacySemanticOracle", ORACLE_PATH, oracle_data)
    return [entries[key] for key in sorted(entries)]


def build(
    repo_root: Path,
) -> tuple[dict[str, bytes], bytes, bytes, bytes, bytes, bytes]:
    inputs = read_json(INPUTS_PATH)
    crosswalk = read_json(CROSSWALK_PATH)
    target_crosswalk = read_json(RIGHTS_TARGET_CROSSWALK_PATH)
    if (
        inputs.get("schemaVersion") != 1
        or crosswalk.get("schemaVersion") != 1
        or target_crosswalk.get("schemaVersion") != 1
    ):
        raise ValueError("legacy oracle source schema is unsupported")
    all_cases = [*inputs["cases"], *inputs.get("contextCases", [])]
    raw_outputs = {
        case["id"]: run_legacy_case(repo_root, inputs, case)
        for case in all_cases
    }
    source_contexts = extract_native_enum_contexts(repo_root, inputs, raw_outputs)
    enum_contexts_data = json_bytes(
        {
            "schemaVersion": 1,
            "provenance": "legacy-source-ast-and-native-descriptor-fixtures",
            "contexts": source_contexts,
        }
    )

    cases: list[dict[str, Any]] = []
    for case in inputs["cases"]:
        raw = raw_outputs[case["id"]]
        if case["tool"] == "metaInfo":
            cases.append(parse_meta_output(case, raw, crosswalk))
        elif case["tool"] == "roleInfo":
            cases.append(
                parse_role_output(
                    case,
                    raw,
                    crosswalk,
                    target_crosswalk["prefixes"],
                )
            )
        else:
            raise ValueError(f"unreviewed legacy tool {case['tool']}")

    enum_coverage = extract_enum_coverage(crosswalk, source_contexts)
    enum_alias_executions_data = json_bytes(
        build_enum_alias_executions(
            repo_root,
            inputs,
            crosswalk,
            source_contexts,
            enum_coverage,
            raw_outputs,
        )
    )
    oracle = {
        "schemaVersion": 1,
        "provenance": "legacy-tools-plus-independent-crosswalk",
        "enumCoverage": enum_coverage,
        "cases": sorted(cases, key=lambda case: case["id"]),
    }
    by_id = {case["id"]: case for case in oracle["cases"]}
    for case in oracle["cases"]:
        parent_id = case.get("parentCase")
        if parent_id is not None:
            parent = by_id[parent_id]
            case["rootKind"] = parent["rootKind"]
            case["rootName"] = parent["rootName"]
    oracle_data = json_bytes(oracle)
    new_only_contract_data = json_bytes(
        build_new_only_contract(
            repo_root,
            inputs,
            read_json(NEW_ONLY_CONTRACT_SOURCE_PATH),
        )
    )
    manifest = {
        "schemaVersion": 1,
        "hashAlgorithm": "SHA-256",
        "regenerationCommand": (
            "python3.12 crates/unica-adapter-platform-xml/tests/fixtures/"
            "v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --write"
        ),
        "entries": provenance_entries(
            repo_root,
            inputs,
            raw_outputs,
            enum_contexts_data,
            enum_alias_executions_data,
            oracle_data,
            new_only_contract_data,
        ),
    }
    return (
        raw_outputs,
        enum_contexts_data,
        enum_alias_executions_data,
        oracle_data,
        new_only_contract_data,
        json_bytes(manifest),
    )


def write_outputs(
    repo_root: Path,
    raw_outputs: dict[str, bytes],
    enum_contexts_data: bytes,
    enum_alias_executions_data: bytes,
    oracle_data: bytes,
    new_only_contract_data: bytes,
    manifest_data: bytes,
) -> None:
    inputs = read_json(INPUTS_PATH)
    paths = {
        case["id"]: repo_root / case["rawOutput"]
        for case in [*inputs["cases"], *inputs.get("contextCases", [])]
    }
    for case_id, data in raw_outputs.items():
        paths[case_id].parent.mkdir(parents=True, exist_ok=True)
        paths[case_id].write_bytes(data)
    ENUM_CONTEXTS_PATH.write_bytes(enum_contexts_data)
    ENUM_ALIAS_EXECUTIONS_PATH.write_bytes(enum_alias_executions_data)
    ORACLE_PATH.write_bytes(oracle_data)
    NEW_ONLY_CONTRACT_PATH.write_bytes(new_only_contract_data)
    MANIFEST_PATH.write_bytes(manifest_data)


def check_outputs(
    repo_root: Path,
    raw_outputs: dict[str, bytes],
    enum_contexts_data: bytes,
    enum_alias_executions_data: bytes,
    oracle_data: bytes,
    new_only_contract_data: bytes,
    manifest_data: bytes,
) -> None:
    inputs = read_json(INPUTS_PATH)
    failures: list[str] = []
    for case in [*inputs["cases"], *inputs.get("contextCases", [])]:
        path = repo_root / case["rawOutput"]
        if not path.exists() or path.read_bytes() != raw_outputs[case["id"]]:
            failures.append(f"raw legacy output drifted: {case['rawOutput']}")
    for path, expected, label in (
        (ENUM_CONTEXTS_PATH, enum_contexts_data, "enum source contexts"),
        (
            ENUM_ALIAS_EXECUTIONS_PATH,
            enum_alias_executions_data,
            "enum alias executions",
        ),
        (ORACLE_PATH, oracle_data, "legacy semantic oracle"),
        (NEW_ONLY_CONTRACT_PATH, new_only_contract_data, "new-only exact contract"),
        (MANIFEST_PATH, manifest_data, "oracle provenance manifest"),
    ):
        if not path.exists() or path.read_bytes() != expected:
            failures.append(f"{label} drifted: {path.relative_to(repo_root)}")
    if failures:
        raise RuntimeError("\n".join(failures))


def run_self_tests(repo_root: Path) -> None:
    crosswalk = read_json(CROSSWALK_PATH)
    target_crosswalk = read_json(RIGHTS_TARGET_CROSSWALK_PATH)["prefixes"]
    meta_case = {
        "id": "selfTestMeta",
        "profile": "meta-full",
        "input": "self-test.xml",
        "sourceRoot": ".",
        "rawOutput": "self-test.txt",
    }
    valid_meta = (
        "=== Language: SelfTest ===\n"
        "Поддержка: не на поддержке\n"
    ).encode()
    parse_meta_output(meta_case, valid_meta, crosswalk)

    role_case = {
        "id": "selfTestRole",
        "profile": "role-info",
        "input": "self-test/Rights.xml",
        "adapterInput": "self-test.xml",
        "sourceRoot": ".",
        "rawOutput": "self-test-role.txt",
        "arguments": ["-ShowDenied"],
    }
    valid_role = (
        "=== Role: SelfTest ===\n"
        "Поддержка: не на поддержке\n"
        "\n"
        "Properties: setForNewObjects=false, "
        "setForAttributesByDefault=false, "
        "independentRightsOfChildObjects=false\n"
        "\n"
        "Allowed rights:\n"
        "\n"
        "  Catalog (1):\n"
        "    Products: Read\n"
        "\n"
        "---\n"
        "Total: 1 allowed, 0 denied\n"
    ).encode()
    parse_role_output(role_case, valid_role, crosswalk, target_crosswalk)

    def expect_failure(label: str, callback: Any) -> None:
        try:
            callback()
        except (ValueError, RuntimeError):
            return
        raise AssertionError(f"negative oracle self-test unexpectedly passed: {label}")

    for label, line in (
        ("new property", "Future property: useful-value"),
        ("new section", "Future section (1):"),
        ("malformed indentation", "   Future"),
        ("unknown support value", "Поддержка: future-mode"),
    ):
        prefix = (
            "=== Language: SelfTest ===\n"
            if label == "unknown support value"
            else valid_meta.decode()
        )
        expect_failure(
            f"meta {label}",
            lambda payload=(prefix + line + "\n").encode(): parse_meta_output(
                meta_case, payload, crosswalk
            ),
        )
    for label, payload in {
        "declared section row count": (
            valid_meta.decode() + "Реквизиты (1):\n"
        ),
        "duplicate singleton field": (
            valid_meta.decode() + "Поддержка: не на поддержке\n"
        ),
        "unknown field flag": (
            valid_meta.decode()
            + "Реквизиты (1):\n"
            + "  Field   Строка(10)  [future-flag]\n"
        ),
    }.items():
        expect_failure(
            f"meta {label}",
            lambda payload=payload.encode(): parse_meta_output(
                meta_case, payload, crosswalk
            ),
        )

    role_mutations = {
        "new role property": valid_role.decode().replace(
            "Allowed rights:", "FutureRoleProperty: useful\nAllowed rights:"
        ),
        "new right line": valid_role.decode().replace(
            "    Products: Read\n",
            "    Products: Read\n    Services: Read\n",
        ),
        "unknown right heading": valid_role.decode().replace(
            "Allowed rights:", "Future rights:"
        ),
        "target without group": valid_role.decode().replace(
            "  Catalog (1):\n", ""
        ),
        "unhandled target prefix": valid_role.decode().replace(
            "Catalog (1)", "FutureObject (1)"
        ),
        "duplicate role properties": valid_role.decode().replace(
            "Allowed rights:",
            "Properties: setForNewObjects=false, "
            "setForAttributesByDefault=false, "
            "independentRightsOfChildObjects=false\nAllowed rights:",
        ),
    }
    for label, payload in role_mutations.items():
        expect_failure(
            label,
            lambda payload=payload.encode(): parse_role_output(
                role_case,
                payload,
                crosswalk,
                target_crosswalk,
            ),
        )

    ledger = LineLedger("duplicateConsumption", ["useful"])
    ledger.consume(1, "useful:first")
    expect_failure(
        "duplicate line consumption",
        lambda: ledger.consume(1, "useful:second"),
    )

    inputs = read_json(INPUTS_PATH)
    all_cases = [*inputs["cases"], *inputs.get("contextCases", [])]
    raw_outputs = {
        case["id"]: run_legacy_case(repo_root, inputs, case)
        for case in all_cases
    }
    contexts = extract_native_enum_contexts(repo_root, inputs, raw_outputs)
    enum_coverage = extract_enum_coverage(crosswalk, contexts)
    executions = build_enum_alias_executions(
        repo_root,
        inputs,
        crosswalk,
        contexts,
        enum_coverage,
        raw_outputs,
    )
    validate_enum_alias_executions(executions, enum_coverage)
    missing_execution = json.loads(json.dumps(executions))
    missing_execution["executions"].pop()
    expect_failure(
        "enum alias execution omission",
        lambda: validate_enum_alias_executions(missing_execution, enum_coverage),
    )
    duplicate_execution = json.loads(json.dumps(executions))
    duplicate_execution["executions"].append(
        dict(duplicate_execution["executions"][0])
    )
    expect_failure(
        "enum alias execution duplication",
        lambda: validate_enum_alias_executions(duplicate_execution, enum_coverage),
    )
    changed_execution_output = json.loads(json.dumps(executions))
    changed_execution_output["executions"][0]["rawOutputSha256"] = "0" * 64
    expect_failure(
        "enum alias execution output mutation",
        lambda: validate_enum_alias_executions(
            changed_execution_output,
            enum_coverage,
        ),
    )
    inferred_template_owner = json.loads(json.dumps(executions))
    template_row = next(
        row
        for row in inferred_template_owner["executions"]
        if row["nativeProperty"] == "TemplateType"
        and row["nativeAlias"] == "SpreadsheetDocument"
        and row["objectKind"] == "template"
    )
    template_row["objectKind"] = "spreadsheetDocumentTemplate"
    expect_failure(
        "TemplateType-derived owner rewrite",
        lambda: validate_enum_alias_executions(
            inferred_template_owner,
            enum_coverage,
        ),
    )
    context_override = json.loads(json.dumps(crosswalk))
    first_domain = next(iter(context_override["enumDomains"].values()))
    first_domain["nativeProperty"] = "CoordinatedWrongProperty"
    first_domain["objectKinds"] = ["document"]
    expect_failure(
        "crosswalk and coverage coordinated context override",
        lambda: extract_enum_coverage(context_override, contexts),
    )
    duplicate_contexts = contexts + [dict(contexts[0])]
    expect_failure(
        "ambiguous duplicate source context",
        lambda: extract_enum_coverage(crosswalk, duplicate_contexts),
    )
    coordinated_omission = json.loads(json.dumps(crosswalk))
    coordinated_omission["enumDomains"].pop("catalogChoiceMode", None)
    expect_failure(
        "crosswalk and coverage coordinated source-domain omission",
        lambda: extract_enum_coverage(coordinated_omission, contexts),
    )


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--repo-root", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    repo_root = args.repo_root.resolve()
    if args.self_test:
        run_self_tests(repo_root)
        print("verified fail-closed parser and source-context negative suite")
        return 0
    (
        raw_outputs,
        enum_contexts_data,
        enum_alias_executions_data,
        oracle_data,
        new_only_contract_data,
        manifest_data,
    ) = build(repo_root)
    if args.write:
        write_outputs(
            repo_root,
            raw_outputs,
            enum_contexts_data,
            enum_alias_executions_data,
            oracle_data,
            new_only_contract_data,
            manifest_data,
        )
        print(
            f"wrote {len(raw_outputs)} raw outputs, "
            f"{len(json.loads(enum_alias_executions_data)['executions'])} enum alias executions, "
            f"{len(json.loads(oracle_data)['cases'])} oracle cases, and provenance"
        )
    else:
        check_outputs(
            repo_root,
            raw_outputs,
            enum_contexts_data,
            enum_alias_executions_data,
            oracle_data,
            new_only_contract_data,
            manifest_data,
        )
        print(
            f"verified {len(raw_outputs)} raw outputs, "
            f"{len(json.loads(enum_alias_executions_data)['executions'])} enum alias executions, "
            "oracle facts, and SHA-256 provenance"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
