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


ORACLE_DIR = Path(__file__).resolve().parents[1]
INPUTS_PATH = ORACLE_DIR / "inputs.json"
CROSSWALK_PATH = ORACLE_DIR / "crosswalk.json"
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
    repo_root: Path,
    inputs: dict[str, Any],
    crosswalk: dict[str, Any],
) -> list[dict[str, Any]]:
    sources = inputs["referenceSources"]
    validate_tree = source_tree(repo_root, sources["metaValidate"])
    meta_info_tree = source_tree(repo_root, sources["metaInfo"])
    template_tree = source_tree(repo_root, sources["templateAdd"])
    valid_values = literal_assignment(validate_tree, "valid_property_values")
    template_map = literal_assignment(template_tree, "TYPE_MAP")

    info_constants = {
        node.value
        for node in ast.walk(meta_info_tree)
        if isinstance(node, ast.Constant) and isinstance(node.value, str)
    }
    field_use_aliases = {
        value for value in info_constants if re.fullmatch(r"For(?:Item|Folder|FolderAndItem)", value)
    }
    register_display_aliases: set[str] = set()
    for node in ast.walk(meta_info_tree):
        if not isinstance(node, ast.Dict):
            continue
        try:
            value = ast.literal_eval(node)
        except (ValueError, TypeError):
            continue
        if isinstance(value, dict) and {"остатки", "обороты"}.intersection(value.values()):
            register_display_aliases.update(
                key for key in value if isinstance(key, str)
            )

    form_type_aliases: set[str] = set()
    for source_name in ("formAdd", "metaEdit"):
        text = (repo_root / sources[source_name]).read_text(encoding="utf-8-sig")
        form_type_aliases.update(re.findall(r"<FormType>([^<]+)</FormType>", text))

    template_type_aliases = {
        value["TemplateType"]
        for value in template_map.values()
        if isinstance(value, dict) and isinstance(value.get("TemplateType"), str)
    }
    extractor_values = {
        "fieldUseAliases": field_use_aliases,
        "registerDisplayAliases": register_display_aliases,
        "formTypeAliases": form_type_aliases,
        "templateTypeAliases": template_type_aliases,
    }

    coverage: list[dict[str, Any]] = []
    for domain_name, domain in crosswalk["enumDomains"].items():
        observed: set[str] = set()
        for source_key in domain.get("sourceKeys", []):
            if source_key not in valid_values:
                raise ValueError(
                    f"enum domain {domain_name} references absent legacy table key {source_key}"
                )
            observed.update(valid_values[source_key])
        for extractor in domain.get("extractors", []):
            observed.update(extractor_values[extractor])
        mapped = set(domain["semanticByAlias"])
        if observed != mapped:
            missing = sorted(observed - mapped)
            extra = sorted(mapped - observed)
            raise ValueError(
                f"enum crosswalk drift for {domain_name}: missing={missing}, extra={extra}"
            )
        for alias in sorted(observed):
            for object_kind in domain["objectKinds"]:
                coverage.append(
                    {
                        "nativeAlias": alias,
                        "nativeProperty": domain["nativeProperty"],
                        "objectKind": object_kind,
                        "semantic": domain["semanticByAlias"][alias],
                        "semanticProperty": domain["semanticProperty"],
                    }
                )
    unique = {canonical(item): item for item in coverage}
    if len(unique) != len(coverage):
        raise ValueError("legacy enum extraction produced duplicate applicability")
    return [unique[key] for key in sorted(unique)]


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
            builder.prop(
                root,
                "support.active",
                bool_value(support != "не на поддержке"),
            )
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
            "Внешнее соединение": "module.externalConnection",
            "Вызов сервера": "module.serverCall",
            "Глобальный": "module.global",
        }
        if " | " in stripped and any(part in module_contexts for part in stripped.split(" | ")):
            for part in stripped.split(" | "):
                if part in module_contexts:
                    builder.prop(root, module_contexts[part], bool_value(True))
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
        match = re.fullmatch(r"ТЧ (.+?) \(\d+ колонки\):", stripped)
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
    }


def parse_role_output(case: dict[str, Any], raw: bytes, crosswalk: dict[str, Any]) -> dict[str, Any]:
    lines = raw.decode("utf-8-sig").splitlines()
    root_kind, root_name, root_synonym = parse_header(lines[0], crosswalk)
    builder = FactBuilder(case["id"])
    root = builder.root(root_kind, root_name)
    if root_synonym:
        builder.prop(root, "metadata.synonym", localized_value(root_synonym))

    mode: str | None = None
    target_kind: str | None = None
    restricted_targets: set[str] = set()
    permission_counts: Counter[tuple[str, str, str]] = Counter()
    allowed_total = denied_total = None

    for line in lines[1:]:
        stripped = line.strip()
        if not stripped:
            continue
        if stripped.startswith("Поддержка:"):
            builder.prop(root, "support.active", bool_value(False))
            continue
        if stripped.startswith("Properties:"):
            values = dict(
                item.split("=", 1)
                for item in stripped.split(":", 1)[1].strip().split(", ")
            )
            for native, semantic in (
                ("setForNewObjects", "access.newObjects.defaultAllowed"),
                ("setForAttributesByDefault", "access.attributes.defaultAllowed"),
                (
                    "independentRightsOfChildObjects",
                    "access.childObjects.independent",
                ),
            ):
                builder.prop(root, semantic, bool_value(values[native] == "true"))
            continue
        if stripped == "Allowed rights:":
            mode = "allowed"
            continue
        if stripped == "Denied rights:":
            mode = "denied"
            continue
        match = re.fullmatch(r"(Catalog|Document) \(\d+\):", stripped)
        if match:
            target_kind = {"Catalog": "catalog", "Document": "document"}[
                match.group(1)
            ]
            continue
        if stripped.startswith("RLS:"):
            count = int(re.search(r"\d+", stripped).group())
            builder.prop(root, "access.restriction.count", int_value(count))
            continue
        if stripped.startswith("Templates:"):
            for name in [
                value.strip() for value in stripped.split(":", 1)[1].split(",")
            ]:
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
            continue
        match = re.fullmatch(r"Total: (\d+) allowed, (\d+) denied", stripped)
        if match:
            allowed_total, denied_total = map(int, match.groups())
            builder.prop(root, "access.allowed.count", int_value(allowed_total))
            builder.prop(root, "access.denied.count", int_value(denied_total))
            continue
        match = re.fullmatch(r"([^:]+): (.+)", stripped)
        if mode in {"allowed", "denied"} and target_kind and match:
            target_name, rights_raw = match.groups()
            has_restriction = rights_raw.endswith(" [RLS]")
            rights_raw = rights_raw.removesuffix(" [RLS]")
            target_identity = (
                f"{case['id']}/external/{target_kind}/{target_name}"
            )
            if has_restriction and target_identity not in restricted_targets:
                builder.facts.append(
                    node_fact(target_identity, target_kind, target_name)
                )
                builder.prop(
                    target_identity,
                    "access.restriction.present",
                    bool_value(True),
                )
                restricted_targets.add(target_identity)
            for permission_raw in [value.strip() for value in rights_raw.split(",")]:
                allowed = mode == "allowed"
                permission_name = permission_raw.removeprefix("-")
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
            continue
    if allowed_total is None or denied_total is None:
        raise ValueError("legacy role output has no totals")
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
    oracle_data: bytes,
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
    add("oracleInputs", INPUTS_PATH)
    add("independentCrosswalk", CROSSWALK_PATH)
    for path in inputs["referenceSources"].values():
        add("legacyReferenceSource", repo_root / path)
    for case in inputs["cases"]:
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
    add("legacySemanticOracle", ORACLE_PATH, oracle_data)
    return [entries[key] for key in sorted(entries)]


def build(repo_root: Path) -> tuple[dict[str, bytes], bytes, bytes]:
    inputs = read_json(INPUTS_PATH)
    crosswalk = read_json(CROSSWALK_PATH)
    if inputs.get("schemaVersion") != 1 or crosswalk.get("schemaVersion") != 1:
        raise ValueError("legacy oracle source schema is unsupported")

    raw_outputs: dict[str, bytes] = {}
    cases: list[dict[str, Any]] = []
    for case in inputs["cases"]:
        raw = run_legacy_case(repo_root, inputs, case)
        raw_outputs[case["id"]] = raw
        if case["tool"] == "metaInfo":
            cases.append(parse_meta_output(case, raw, crosswalk))
        elif case["tool"] == "roleInfo":
            cases.append(parse_role_output(case, raw, crosswalk))
        else:
            raise ValueError(f"unreviewed legacy tool {case['tool']}")

    oracle = {
        "schemaVersion": 1,
        "provenance": "legacy-tools-plus-independent-crosswalk",
        "enumCoverage": extract_enum_coverage(repo_root, inputs, crosswalk),
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
    manifest = {
        "schemaVersion": 1,
        "hashAlgorithm": "SHA-256",
        "regenerationCommand": (
            "python3.12 crates/unica-adapter-platform-xml/tests/fixtures/"
            "v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --write"
        ),
        "entries": provenance_entries(repo_root, inputs, raw_outputs, oracle_data),
    }
    return raw_outputs, oracle_data, json_bytes(manifest)


def write_outputs(
    repo_root: Path,
    raw_outputs: dict[str, bytes],
    oracle_data: bytes,
    manifest_data: bytes,
) -> None:
    inputs = read_json(INPUTS_PATH)
    paths = {case["id"]: repo_root / case["rawOutput"] for case in inputs["cases"]}
    for case_id, data in raw_outputs.items():
        paths[case_id].parent.mkdir(parents=True, exist_ok=True)
        paths[case_id].write_bytes(data)
    ORACLE_PATH.write_bytes(oracle_data)
    MANIFEST_PATH.write_bytes(manifest_data)


def check_outputs(
    repo_root: Path,
    raw_outputs: dict[str, bytes],
    oracle_data: bytes,
    manifest_data: bytes,
) -> None:
    inputs = read_json(INPUTS_PATH)
    failures: list[str] = []
    for case in inputs["cases"]:
        path = repo_root / case["rawOutput"]
        if not path.exists() or path.read_bytes() != raw_outputs[case["id"]]:
            failures.append(f"raw legacy output drifted: {case['rawOutput']}")
    for path, expected, label in (
        (ORACLE_PATH, oracle_data, "legacy semantic oracle"),
        (MANIFEST_PATH, manifest_data, "oracle provenance manifest"),
    ):
        if not path.exists() or path.read_bytes() != expected:
            failures.append(f"{label} drifted: {path.relative_to(repo_root)}")
    if failures:
        raise RuntimeError("\n".join(failures))


def main() -> int:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--repo-root", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--write", action="store_true")
    mode.add_argument("--check", action="store_true")
    args = parser.parse_args()
    repo_root = args.repo_root.resolve()
    raw_outputs, oracle_data, manifest_data = build(repo_root)
    if args.write:
        write_outputs(repo_root, raw_outputs, oracle_data, manifest_data)
        print(
            f"wrote {len(raw_outputs)} raw outputs, "
            f"{len(json.loads(oracle_data)['cases'])} oracle cases, and provenance"
        )
    else:
        check_outputs(repo_root, raw_outputs, oracle_data, manifest_data)
        print(
            f"verified {len(raw_outputs)} raw outputs, oracle facts, and SHA-256 provenance"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
