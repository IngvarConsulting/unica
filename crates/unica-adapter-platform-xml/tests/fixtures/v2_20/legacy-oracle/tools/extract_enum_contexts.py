#!/usr/bin/env python3
"""Extract native enum aliases and applicability from frozen legacy sources.

This module deliberately knows nothing about the new adapter or its semantic
IDs.  It derives native property names and owner contexts from legacy Python
AST structure and from the native descriptor fixtures consumed by the legacy
tools.  The semantic crosswalk may select a source fact and attach semantic
IDs, but it cannot alter the native tuple produced here.
"""

from __future__ import annotations

import ast
import re
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import Any, Iterable


def _literal_assignment(tree: ast.AST, name: str) -> Any:
    matches = []
    for node in ast.walk(tree):
        if not isinstance(node, (ast.Assign, ast.AnnAssign)):
            continue
        targets = node.targets if isinstance(node, ast.Assign) else [node.target]
        if not any(isinstance(target, ast.Name) and target.id == name for target in targets):
            continue
        try:
            matches.append(ast.literal_eval(node.value))
        except (TypeError, ValueError):
            pass
    if len(matches) != 1:
        raise ValueError(
            f"legacy source assignment {name!r} is not uniquely literal: {len(matches)} matches"
        )
    return matches[0]


def _tree(path: Path) -> ast.AST:
    return ast.parse(path.read_text(encoding="utf-8-sig"), filename=str(path))


def _camel(value: str) -> str:
    parts = value.split("_")
    return parts[0] + "".join(part[:1].upper() + part[1:] for part in parts[1:])


def _joined_string_tag_and_names(node: ast.AST) -> tuple[str, set[str]] | None:
    if not isinstance(node, ast.Call):
        return None
    if not isinstance(node.func, ast.Name) or node.func.id != "X" or len(node.args) != 1:
        return None
    joined = node.args[0]
    if not isinstance(joined, ast.JoinedStr):
        return None
    literal = "".join(
        value.value for value in joined.values if isinstance(value, ast.Constant)
    )
    match = re.search(r"<([A-Za-z][A-Za-z0-9]*)>", literal)
    if match is None:
        return None
    names = {
        value.value.id
        for value in joined.values
        if isinstance(value, ast.FormattedValue)
        and isinstance(value.value, ast.Name)
    }
    return match.group(1), names


def _source_table_key(
    valid_values: dict[str, list[str]],
    owner_source_name: str,
    native_property: str,
    supplied_key: str,
) -> str:
    contextual = f"{owner_source_name}{native_property}"
    candidates = [
        key
        for key in (contextual, native_property, supplied_key)
        if key in valid_values
    ]
    if not candidates:
        raise ValueError(
            "legacy enum emission has no source alias table: "
            f"{owner_source_name}.{native_property} via {supplied_key}"
        )
    return candidates[0]


def _compile_contexts(
    compile_tree: ast.AST,
    valid_values: dict[str, list[str]],
) -> list[dict[str, Any]]:
    result = []
    for function in ast.walk(compile_tree):
        if not isinstance(function, ast.FunctionDef):
            continue
        match = re.fullmatch(r"emit_(.+)_properties", function.name)
        if match is None:
            continue
        owner_source_name = "".join(
            part[:1].upper() + part[1:] for part in match.group(1).split("_")
        )
        object_kind = _camel(match.group(1))
        variables: dict[str, str] = {}
        for node in ast.walk(function):
            if not isinstance(node, ast.Assign) or len(node.targets) != 1:
                continue
            target = node.targets[0]
            call = node.value
            if (
                isinstance(target, ast.Name)
                and isinstance(call, ast.Call)
                and isinstance(call.func, ast.Name)
                and call.func.id == "get_enum_prop"
                and call.args
                and isinstance(call.args[0], ast.Constant)
                and isinstance(call.args[0].value, str)
            ):
                variables[target.id] = call.args[0].value
        emissions: dict[str, str] = {}
        for node in ast.walk(function):
            parsed = _joined_string_tag_and_names(node)
            if parsed is None:
                continue
            native_property, names = parsed
            for name in names:
                if name in variables:
                    previous = emissions.setdefault(name, native_property)
                    if previous != native_property:
                        raise ValueError(
                            f"{function.name}.{name} emits multiple native properties"
                        )
        for variable, supplied_key in sorted(variables.items()):
            native_property = emissions.get(variable)
            if native_property is None:
                raise ValueError(
                    f"{function.name}.{variable} enum value is not emitted into a native tag"
                )
            source_key = _source_table_key(
                valid_values,
                owner_source_name,
                native_property,
                supplied_key,
            )
            result.append(
                {
                    "sourceFact": (
                        f"metaCompile:{function.name}:{native_property}"
                    ),
                    "sourceKey": source_key,
                    "nativeProperty": native_property,
                    "objectKinds": [object_kind],
                    "nativeAliases": sorted(set(valid_values[source_key])),
                    "sourceLocation": {
                        "function": function.name,
                        "line": function.lineno,
                    },
                }
            )
    return result


def _function(tree: ast.AST, name: str) -> ast.FunctionDef:
    matches = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.FunctionDef) and node.name == name
    ]
    if len(matches) != 1:
        raise ValueError(f"legacy source function {name!r} is not unique")
    return matches[0]


def _find_properties(function: ast.FunctionDef) -> set[str]:
    result = set()
    for node in ast.walk(function):
        if not isinstance(node, ast.Call) or len(node.args) < 2:
            continue
        if not isinstance(node.func, ast.Name) or node.func.id != "find":
            continue
        argument = node.args[1]
        if (
            isinstance(argument, ast.Constant)
            and isinstance(argument.value, str)
            and argument.value.startswith("md:")
        ):
            result.add(argument.value.removeprefix("md:"))
    return result


def _field_contexts(
    info_tree: ast.AST,
    valid_values: dict[str, list[str]],
) -> list[dict[str, Any]]:
    function = _function(info_tree, "format_flags")
    properties = _find_properties(function)
    result = []
    for native_property in ("FillChecking", "Indexing"):
        if native_property not in properties:
            raise ValueError(
                f"meta-info format_flags no longer reads {native_property}"
            )
        result.append(
            {
                "sourceFact": f"metaInfo:format_flags:{native_property}",
                "sourceKey": native_property,
                "nativeProperty": native_property,
                "objectKinds": ["attribute", "dimension", "resource"],
                "nativeAliases": sorted(set(valid_values[native_property])),
                "sourceLocation": {
                    "function": function.name,
                    "line": function.lineno,
                },
            }
        )
    constants = {
        node.value
        for node in ast.walk(info_tree)
        if isinstance(node, ast.Constant)
        and isinstance(node.value, str)
        and re.fullmatch(r"For(?:Item|Folder|FolderAndItem)", node.value)
    }
    if constants != {"ForItem", "ForFolder", "ForFolderAndItem"}:
        raise ValueError(f"meta-info field Use aliases drifted: {sorted(constants)}")
    result.append(
        {
            "sourceFact": "metaInfo:format_flags:Use",
            "sourceKey": "Use",
            "nativeProperty": "Use",
            "objectKinds": ["attribute", "dimension", "resource"],
            "nativeAliases": sorted(constants),
            "sourceLocation": {
                "function": function.name,
                "line": function.lineno,
            },
        }
    )
    return result


def _transfer_direction_context(
    info_tree: ast.AST,
    valid_values: dict[str, list[str]],
) -> dict[str, Any]:
    function = _function(info_tree, "get_ws_operations")
    if "TransferDirection" not in _find_properties(function):
        raise ValueError("meta-info WebService operation parser lost TransferDirection")
    parameter_tags = {
        node.args[1].value.removeprefix("md:")
        for node in ast.walk(function)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "find_all"
        and len(node.args) >= 2
        and isinstance(node.args[1], ast.Constant)
        and isinstance(node.args[1].value, str)
        and node.args[1].value.startswith("md:")
    }
    if "Parameter" not in parameter_tags:
        raise ValueError("meta-info TransferDirection has no Parameter owner")
    return {
        "sourceFact": "metaInfo:get_ws_operations:TransferDirection",
        "sourceKey": "TransferDirection",
        "nativeProperty": "TransferDirection",
        "objectKinds": ["webServiceParameter"],
        "nativeAliases": sorted(set(valid_values["TransferDirection"])),
        "sourceLocation": {
            "function": function.name,
            "line": function.lineno,
        },
    }


def _descriptor_enum_contexts(
    repo_root: Path,
    cases: Iterable[dict[str, Any]],
    form_aliases: set[str],
    template_aliases: set[str],
) -> list[dict[str, Any]]:
    contexts: dict[str, set[str]] = {"FormType": set(), "TemplateType": set()}
    aliases: dict[str, set[str]] = {"FormType": set(), "TemplateType": set()}
    locations: dict[str, list[str]] = {"FormType": [], "TemplateType": []}
    paths = []
    for case in cases:
        paths.append(case["input"])
        paths.extend(case.get("inputArtifacts", []))
    for relative in sorted(set(paths)):
        path = repo_root / relative
        if path.suffix.lower() != ".xml":
            continue
        try:
            root = ET.parse(path).getroot()
        except ET.ParseError:
            continue
        classes = [
            node
            for node in root.iter()
            if node.tag.rsplit("}", 1)[-1]
            in {"Form", "CommonForm", "Template", "CommonTemplate"}
        ]
        for node in classes:
            kind = node.tag.rsplit("}", 1)[-1]
            semantic_kind = kind[:1].lower() + kind[1:]
            for descendant in node.iter():
                property_name = descendant.tag.rsplit("}", 1)[-1]
                if property_name not in contexts:
                    continue
                contexts[property_name].add(semantic_kind)
                if descendant.text and descendant.text.strip():
                    aliases[property_name].add(descendant.text.strip())
                locations[property_name].append(relative)
    aliases["FormType"].update(form_aliases)
    aliases["TemplateType"].update(template_aliases)
    if "SpreadsheetDocument" in aliases["TemplateType"]:
        contexts["TemplateType"].add("spreadsheetDocumentTemplate")
    if not contexts["FormType"] or not contexts["TemplateType"]:
        raise ValueError("legacy descriptor corpus has no form/template enum contexts")
    return [
        {
            "sourceFact": "legacyDescriptors:FormType",
            "sourceKey": "FormType",
            "nativeProperty": "FormType",
            "objectKinds": sorted(contexts["FormType"]),
            "nativeAliases": sorted(aliases["FormType"]),
            "sourceLocation": {"fixtures": sorted(set(locations["FormType"]))},
        },
        {
            "sourceFact": "legacyDescriptors:TemplateType",
            "sourceKey": "TemplateType",
            "nativeProperty": "TemplateType",
            "objectKinds": sorted(contexts["TemplateType"]),
            "nativeAliases": sorted(aliases["TemplateType"]),
            "sourceLocation": {"fixtures": sorted(set(locations["TemplateType"]))},
        },
    ]


def extract(repo_root: Path, inputs: dict[str, Any]) -> list[dict[str, Any]]:
    sources = inputs["referenceSources"]
    validate_tree = _tree(repo_root / sources["metaValidate"])
    info_tree = _tree(repo_root / sources["metaInfo"])
    compile_tree = _tree(repo_root / sources["metaCompile"])
    form_tree = _tree(repo_root / sources["formAdd"])
    template_tree = _tree(repo_root / sources["templateAdd"])
    valid_values = _literal_assignment(validate_tree, "valid_property_values")
    template_map = _literal_assignment(template_tree, "TYPE_MAP")

    form_aliases = {
        match
        for node in ast.walk(form_tree)
        if isinstance(node, ast.Constant) and isinstance(node.value, str)
        for match in re.findall(r"<FormType>([^<]+)</FormType>", node.value)
    }
    edit_source = (repo_root / sources["metaEdit"]).read_text(encoding="utf-8-sig")
    form_aliases.update(re.findall(r"<FormType>([^<]+)</FormType>", edit_source))
    template_aliases = {
        value["TemplateType"]
        for value in template_map.values()
        if isinstance(value, dict) and isinstance(value.get("TemplateType"), str)
    }

    contexts = _compile_contexts(compile_tree, valid_values)
    register_display_aliases = set()
    for node in ast.walk(info_tree):
        if not isinstance(node, ast.Dict):
            continue
        try:
            value = ast.literal_eval(node)
        except (TypeError, ValueError):
            continue
        if isinstance(value, dict) and {"остатки", "обороты"}.intersection(value.values()):
            register_display_aliases.update(
                key for key in value if isinstance(key, str)
            )
    register_context = next(
        context
        for context in contexts
        if context["sourceFact"]
        == "metaCompile:emit_accumulation_register_properties:RegisterType"
    )
    register_context["nativeAliases"] = sorted(
        set(register_context["nativeAliases"]) | register_display_aliases
    )
    contexts.extend(_field_contexts(info_tree, valid_values))
    contexts.append(_transfer_direction_context(info_tree, valid_values))
    contexts.extend(
        _descriptor_enum_contexts(
            repo_root,
            inputs["cases"],
            form_aliases,
            template_aliases,
        )
    )
    by_id: dict[str, dict[str, Any]] = {}
    for context in contexts:
        source_fact = context["sourceFact"]
        if source_fact in by_id:
            raise ValueError(f"duplicate source enum fact {source_fact}")
        if not context["nativeAliases"] or not context["objectKinds"]:
            raise ValueError(f"empty source enum applicability for {source_fact}")
        by_id[source_fact] = context
    return [by_id[key] for key in sorted(by_id)]
