"""Content checks shared by acceptance wires and their counterexample tests."""
from __future__ import annotations

import hashlib
import json
import math
import re
from pathlib import Path, PurePosixPath
import xml.etree.ElementTree as ET

MISSING = object()
OPERATORS = {"eq", "notEq", "contains", "absent"}
KNOWN_GAP = "form-handler-signature-unchecked"
FORM_AT = "main:Catalog.Валюты.Form.ФормаЭлемента"
FORM_FILE = "src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form.xml"
MODULE_FILE = "src/Catalogs/Валюты/Forms/ФормаЭлемента/Ext/Form/Module.bsl"


def load_corpus(path):
    def unique_object(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate corpus key {key!r}")
            result[key] = value
        return result

    def invalid_constant(value):
        raise ValueError(f"non-JSON corpus number {value}")

    def finite_float(value):
        result = float(value)
        if not math.isfinite(result):
            invalid_constant(value)
        return result

    return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=unique_object,
                      parse_constant=invalid_constant, parse_float=finite_float)


def pointer_tokens(pointer):
    if not isinstance(pointer, str) or (pointer and not pointer.startswith("/")):
        raise ValueError("JSON pointer must be empty or start with /")
    if re.search(r"~(?![01])", pointer):
        raise ValueError("JSON pointer has an invalid escape")
    return [part.replace("~1", "/").replace("~0", "~") for part in pointer.split("/")[1:]]


def pointer_value(document, pointer):
    value = document
    tokens = pointer_tokens(pointer)
    for index, token in enumerate(tokens):
        if isinstance(value, dict):
            child = value.get(token, MISSING)
        elif isinstance(value, list) and re.fullmatch(r"0|[1-9][0-9]*", token):
            child = value[int(token)] if int(token) < len(value) else MISSING
        else:
            raise ValueError(f"pointer {pointer!r} traverses a non-container or invalid array index")
        if child is MISSING and index != len(tokens) - 1:
            raise ValueError(f"pointer {pointer!r} has a missing parent")
        value = child
    return value


def json_equal(left, right):
    # Python treats True as 1, including inside lists and dicts. JSON does not.
    numeric = (int, float)
    if type(left) in numeric and type(right) in numeric:
        return left == right
    if type(left) is not type(right):
        return False
    if isinstance(left, list):
        return len(left) == len(right) and all(json_equal(a, b) for a, b in zip(left, right))
    if isinstance(left, dict):
        return left.keys() == right.keys() and all(json_equal(left[k], right[k]) for k in left)
    return left == right


def substitute_capture(value, context):
    if isinstance(value, str) and value.startswith("$capture."):
        name = value[len("$capture."):]
        if name not in context.get("captures", {}):
            raise ValueError(f"capture {name!r} is not available")
        return context["captures"][name]
    if isinstance(value, list):
        return [substitute_capture(item, context) for item in value]
    if isinstance(value, dict):
        return {key: substitute_capture(item, context) for key, item in value.items()}
    return value


def relative_file(path):
    if not isinstance(path, str) or not path or "\\" in path or ":" in path:
        raise ValueError("file path must be a portable workspace-relative path")
    value = PurePosixPath(path)
    if value.is_absolute() or ".." in value.parts or not value.parts:
        raise ValueError("file path must stay inside the workspace")
    return value


def observation_path(workspace, path):
    relative = relative_file(path)
    root = Path(workspace).resolve(strict=True)
    current = root
    for part in relative.parts:
        current /= part
        if current.is_symlink():
            raise ValueError("file observations must not follow symlinks")
    current.resolve(strict=False).relative_to(root)
    return current


def read_file(workspace, path):
    return observation_path(workspace, path).read_bytes()


def xml_root(raw):
    text = raw.decode("utf-8-sig")
    if "\x00" in text or re.search(r"<!\s*(?:DOCTYPE|ENTITY)\b", text, re.IGNORECASE):
        raise ValueError("XML observations must not contain DTD or entities")
    return ET.fromstring(text)


def file_value(workspace, descriptor):
    if descriptor["kind"] == "exists":
        return observation_path(workspace, descriptor["path"]).exists()
    raw = read_file(workspace, descriptor.get("path", descriptor.get("file")))
    if descriptor["kind"] == "sha256":
        return hashlib.sha256(raw).hexdigest()
    nodes = xml_root(raw).findall(descriptor["select"])
    return ["".join(node.itertext()) for node in nodes]


def operator(assertion):
    names = OPERATORS & assertion.keys()
    if len(names) != 1:
        raise ValueError("an assertion requires exactly one operator")
    name = next(iter(names))
    if name == "absent" and assertion[name] is not True:
        raise ValueError("absent must be true")
    return name


def assert_value(value, assertion, context):
    op = operator(assertion)
    expected = substitute_capture(assertion[op], context)
    if op == "absent":
        passed = value is MISSING
    elif value is MISSING:
        passed = False
    elif op == "eq":
        passed = json_equal(value, expected)
    elif op == "notEq":
        passed = not json_equal(value, expected)
    elif isinstance(value, str) and isinstance(expected, str):
        passed = expected in value
    elif isinstance(value, list):
        passed = any(json_equal(item, expected) for item in value)
    else:
        passed = False
    if not passed:
        observed = "missing" if value is MISSING else repr(value)
        raise ValueError(f"{op} expected {expected!r}, got {observed}")


def validate_file_operand(assertion, expected):
    op = operator(assertion)
    if assertion["kind"] == "exists":
        if op != "eq" or type(expected) is not bool:
            raise ValueError("exists requires boolean eq")
    elif assertion["kind"] == "sha256":
        if not isinstance(expected, str) or not re.fullmatch(r"[a-f0-9]{64}", expected):
            raise ValueError("SHA256 operand must be a 64-character lowercase digest")
    elif op == "contains":
        if not isinstance(expected, str):
            raise ValueError("XML contains operand must be one text string")
    elif not isinstance(expected, list) or not all(isinstance(item, str) for item in expected):
        raise ValueError("XML eq/notEq operand must be a list of text strings")


def validate_controls(step, declared=None):
    declared = set() if declared is None else declared
    allowed = {"tool", "args", "expect", "form", "gap", "knownGap", "refusal", "status",
               "validators", "diagnostic", "assertions", "fileAssertions", "captures"}
    if set(step) - allowed:
        raise ValueError("unknown acceptance step field")
    # References are to earlier successful steps, never to this step itself.
    available = {"captures": {name: None for name in declared}}
    for key in ("args", "assertions", "fileAssertions"):
        substitute_capture(step.get(key), available)
    for key in ("assertions", "fileAssertions"):
        controls = step.get(key, [])
        if not isinstance(controls, list):
            raise ValueError(f"{key} must be an array")
        for assertion in controls:
            if not isinstance(assertion, dict):
                raise ValueError("an assertion must be an object")
            op = operator(assertion)
            if key == "assertions":
                if set(assertion) != {"pointer", op}:
                    raise ValueError("unknown JSON assertion field")
                pointer_tokens(assertion["pointer"])
            else:
                relative_file(assertion.get("path"))
                kind = assertion.get("kind")
                if kind == "exists":
                    if set(assertion) != {"path", "kind", "eq"}:
                        raise ValueError("exists requires only boolean eq")
                elif kind == "sha256":
                    if set(assertion) != {"path", "kind", op} or op not in {"eq", "notEq"}:
                        raise ValueError("sha256 requires eq or notEq")
                    expected = assertion[op]
                    if not isinstance(expected, str) or not (re.fullmatch(r"[a-f0-9]{64}", expected) or expected.startswith("$capture.")):
                        raise ValueError("sha256 requires a digest or an earlier capture")
                elif kind == "xml":
                    if set(assertion) != {"path", "kind", "select", op} or op == "absent":
                        raise ValueError("unknown XML assertion field or operator")
                    if not isinstance(assertion["select"], str) or not assertion["select"]:
                        raise ValueError("XML select must be nonempty")
                    try:
                        ET.Element("root").findall(assertion["select"])
                    except (SyntaxError, TypeError, KeyError) as error:
                        raise ValueError("invalid XML select") from error
                else:
                    raise ValueError("unknown file observation kind")
                expected = assertion[op]
                if not (isinstance(expected, str) and expected.startswith("$capture.")):
                    validate_file_operand(assertion, expected)
    captures = step.get("captures", {})
    if not isinstance(captures, dict):
        raise ValueError("captures must be an object")
    for name, descriptor in captures.items():
        if not re.fullmatch(r"[a-zA-Z][a-zA-Z0-9_]*", name) or name in {"task", "rev", "executionToken"} or name in declared:
            raise ValueError("capture name is invalid, reserved or already declared")
        if not isinstance(descriptor, dict):
            raise ValueError("capture descriptor must be an object")
        if set(descriptor) == {"pointer"}:
            pointer_tokens(descriptor["pointer"])
        elif set(descriptor) == {"file", "kind"} and descriptor["kind"] == "sha256":
            relative_file(descriptor["file"])
        else:
            raise ValueError("unknown capture descriptor")
    if "knownGap" in step:
        gap = step["knownGap"]
        if not isinstance(gap, dict) or type(gap.get("issue")) is not int or gap != {"name": KNOWN_GAP, "observedClass": "ok", "issue": 791}:
            raise ValueError("knownGap is not a registered observation")
        if (step.get("expect") != ["gap"] or step.get("tool") != "unica.check"
                or step.get("args") != {"at": FORM_AT} or step.get("status") != "passed"
                or step.get("validators") != ["form"] or not isinstance(step.get("gap"), str)
                or not step["gap"].strip()):
            raise ValueError("knownGap requires its exact target, verdict and reason")
    declared.update(captures)


def capture_values(step, structured, workspace, context):
    captured = dict(context.get("captures", {}))
    for name, descriptor in step.get("captures", {}).items():
        if name in captured:
            raise ValueError(f"capture {name!r} was already set")
        value = (pointer_value(structured, descriptor["pointer"]) if "pointer" in descriptor
                 else file_value(workspace, descriptor))
        if value is MISSING:
            raise ValueError(f"capture {name!r} points at missing data")
        captured[name] = value
    return captured


def known_gap_witness(workspace):
    ns = "{http://v8.1c.ru/8.3/xcf/logform}"
    root = xml_root(read_file(workspace, FORM_FILE))
    events = root.findall(f".//{ns}InputField[@name='ПолеНаименование']/{ns}Events/{ns}Event[@name='ChoiceProcessing']")
    if len(events) != 1 or events[0].text != "ПолеНаименованиеОбработкаВыбора":
        raise ValueError("knownGap fixture lacks its bound ChoiceProcessing event")
    module = read_file(workspace, MODULE_FILE).decode("utf-8-sig")
    signature = r"^Процедура ПолеНаименованиеОбработкаВыбора\(Элемент, ВыбранноеЗначение, СтандартнаяОбработка\)\r?$"
    if len(re.findall(signature, module, re.MULTILINE)) != 1:
        raise ValueError("knownGap fixture lacks its incorrect three-parameter signature")


def content_mismatches(step, structured, actual, note, workspace=None, context=None):
    failures = []
    data = structured.get("data") or {}
    if actual == "refused" and note != step.get("refusal"):
        failures.append(f"expected refusal {step.get('refusal')!r}, got {note!r}")
    if actual == "ok":
        for name in ("status", "validators"):
            if name in step and data.get(name) != step[name]:
                failures.append(f"expected {name} {step[name]!r}, got {data.get(name)!r}")
        if "diagnostic" in step:
            codes = [item.get("code") for item in data.get("diagnostics") or []]
            if step["diagnostic"] not in codes:
                failures.append(f"expected diagnostic {step['diagnostic']!r}, got {codes!r}")
    context = context or {}
    for assertion in step.get("assertions", []):
        try:
            assert_value(pointer_value(structured, assertion["pointer"]), assertion, context)
        except (ValueError, KeyError, TypeError) as error:
            failures.append(f"JSON {assertion.get('pointer')!r}: {error}")
    for assertion in step.get("fileAssertions", []):
        try:
            validate_file_operand(assertion, substitute_capture(assertion[operator(assertion)], context))
            assert_value(file_value(workspace, assertion), assertion, context)
        except (ValueError, OSError, KeyError, TypeError, SyntaxError, ET.ParseError) as error:
            failures.append(f"file {assertion.get('path')!r}: {error}")
    if "knownGap" in step:
        try:
            validate_controls(step, set(context.get("captures", {})))
            if actual != "ok" or structured.get("ok") is not True:
                raise ValueError("knownGap requires strictly ok:true")
            known_gap_witness(workspace)
        except (ValueError, OSError, TypeError, ET.ParseError) as error:
            failures.append(f"knownGap: {error}")
    return failures


def matches_step(step, actual):
    if "knownGap" in step:
        return actual == step["knownGap"]["observedClass"]
    return actual in step["expect"] or ("gap" in step["expect"] and actual in {"refused", "gap-candidate", "provider"})
