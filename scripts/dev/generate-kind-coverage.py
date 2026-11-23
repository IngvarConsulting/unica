#!/usr/bin/env python3
"""Generate the witnessed root-kind slice (read / object.create / address / check).

Example: generate-kind-coverage.py --address-report .session-temp/addresses.json
The address input is produced by generate-address-coverage.py --native-only.
Without that input, address cells retain no evidence. --check validates a saved
slice against current declarations and its evidence inputs. This measures the
selected corpus contracts, not a new execution or complete product support.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import runpy
import sys

REPO = Path(__file__).resolve().parents[2]
DECLARATIONS = runpy.run_path(str(REPO / "scripts/ci/check-kind-lists.py"))["declared"]
ADDRESS = runpy.run_path(str(REPO / "scripts/dev/generate-address-coverage.py"))
SCENARIOS = tuple(f"S-2994-{name}-create" for name in (
    "configurator-kinds", "declared-kinds", "descriptor-kinds", "queue",
))
READ_SELECTIONS = (
    ("S-1568-identity-metadata-facts", "CommonPicture", 8, "/data/props/Synonym"),
    ("S-1568-identity-metadata-facts", "FilterCriterion", 12, "/data/props/Type"),
    ("S-1568-identity-metadata-facts", "IntegrationService", 28, "/data/props/Synonym"),
    ("S-3097-role-comment-subsystem-help", "Role", 2, "/data/props/Comment"),
    ("S-3097-role-comment-subsystem-help", "Subsystem", 8, "/data/props/IncludeHelpInContents"),
    ("S-3098-xdto-synonym", "XDTOPackage", 1, "/data/props/Synonym"),
)
CHECK_SELECTIONS = (
    ("S-1507-nested-subsystem-check", "Subsystem", 3,
     "main:Subsystem.Parent", "subsystem", "passed", {"/data/at": "main:Subsystem.Parent"}),
    ("S347", "DocumentJournal", 2, "main:DocumentJournal.WarningJournal", "meta", "warning", {}),
    ("S-1370-legacy-inline-warning", "Catalog", 2, "main:Catalog.Editable", "meta", "warning",
     {"/data/status": "passed", "/data/diagnosticCount": 2}),
    ("S-1627-constant-family-typeset", "Constant", 33, "main:Constant.ForeignFamily", "meta", "failed",
     {"/data/diagnostics/0/code": "validation_failed", "/data/diagnostics/0/severity": "error",
      "/data/diagnostics/0/field": "details.type"}),
    ("S-1663-metadata-edit-type", "InformationRegister", 31,
     "main:InformationRegister.History", "meta", "passed", {"/data/diagnosticCount": 1}),
)
SCENARIO_DIR = Path("tests/fixtures/acceptance/scenarios")
MANIFEST_DIR = Path("tests/fixtures/acceptance/scenario-manifest")
ROOT_TEST = "native_configuration_root_address_table_matches_published_branches"
ROOT_TEST_FILE = "crates/unica-coder/src/infrastructure/v13_read/tests/address_coverage.rs"
COLUMNS = ("read", "write", "address", "check")


def canonical(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True)


def root_kinds(repo: Path) -> list[str]:
    kinds, errors = DECLARATIONS(repo)
    if errors:
        raise ValueError("; ".join(errors))
    roots = sorted(kind.tag for kind in kinds if kind.root)
    if not roots:
        raise ValueError("root-kind registry is empty")
    return roots


def load_scenario(repo: Path, scenario: str) -> tuple[Path, dict]:
    path = SCENARIO_DIR / f"{scenario}.json"
    data = json.loads((repo / path).read_text(encoding="utf-8"))
    manifest = (repo / MANIFEST_DIR / f"{scenario}.steps").read_text().splitlines()
    if data["id"] != scenario or [step["tool"] for step in data["wire"]] != manifest:
        raise ValueError(f"invalid scenario/step manifest: {path}")
    return path, data


def creation_evidence(repo: Path, roots: list[str], scenarios: tuple[str, ...]) -> dict:
    evidence = {kind: [] for kind in roots}
    for scenario in scenarios:
        path, data = load_scenario(repo, scenario)
        wire = data["wire"]
        found = False
        for prepare, step in enumerate(wire):
            if step["tool"] != "unica.apply" or step.get("expect") != ["ok"]:
                continue
            args = step.get("args", {})
            ops = args.get("ops", [])
            creates = [(i, op) for i, op in enumerate(ops) if op.get("op") == "object.create"]
            if not creates:
                continue
            owner = args.get("at", "")
            captures = [name for name, value in step.get("captures", {}).items()
                        if value == {"pointer": "/data/executionToken"}]
            if not re.fullmatch(r"[^:]+:Configuration", owner) or len(captures) != 1:
                raise ValueError(f"object.create lacks a root/token witness: {path}:{prepare + 1}")
            capture = captures[0]
            execute = None
            for index in range(prepare + 1, len(wire)):
                candidate = wire[index]
                if capture in candidate.get("captures", {}):
                    break
                if (candidate["tool"] == "unica.apply"
                        and candidate.get("args") == {"executionToken": f"$capture.{capture}"}
                        and candidate.get("expect") == ["ok"]):
                    execute = index
                    break
            if execute is None:
                raise ValueError(f"object.create is only prepared, not executed: {path}:{prepare + 1}")
            for operation, op in creates:
                values = op["args"]["values"]
                kind, name = values["kind"], values["name"]
                if kind not in evidence:
                    raise ValueError(f"unregistered root kind {kind!r} in {path}")
                escaped = name.replace("\\", "\\\\").replace(".", "\\.")
                at = f"{owner.split(':')[0]}:{kind}.{escaped}"
                views = [index for index in range(execute + 1, len(wire))
                         if wire[index]["tool"] == "unica.view"
                         and wire[index].get("args", {}).get("at") == at
                         and wire[index].get("expect") == ["ok"]
                         and wire[index].get("assertions")]
                if not views:
                    raise ValueError(f"no asserted view after object.create: {path}:{at}")
                for view in views:
                    for assertion in wire[view]["assertions"]:
                        if assertion.get("pointer") == "/data/kind" and assertion.get("eq") != kind:
                            raise ValueError(f"view asserts another kind: {path}:{view + 1}")
                    evidence[kind].append({
                        "scenario": path.as_posix(), "declared_formats": data["formats"],
                        "workspace": data["workspace"], "at": at,
                        "prepare_step": prepare + 1, "operation_index": operation + 1,
                        "execute_step": execute + 1, "view_step": view + 1,
                    })
                    found = True
        if not found:
            raise ValueError(f"scenario has no executed creation/read evidence: {path}")
    return evidence


def read_evidence(repo: Path, roots: list[str], selections: tuple) -> dict:
    evidence = {kind: [] for kind in roots}
    for scenario, kind, view_step, pointer in selections:
        if kind not in evidence:
            raise ValueError(f"unregistered read kind: {kind}")
        path, data = load_scenario(repo, scenario)
        wire = data["wire"]
        if not 1 <= view_step <= len(wire):
            raise ValueError(f"invalid read step: {path}:{view_step}")
        view = wire[view_step - 1]
        if view["tool"] != "unica.view" or view.get("expect") != ["ok"]:
            raise ValueError(f"read witness is not a successful view: {path}:{view_step}")
        at = view.get("args", {}).get("at", "")
        capture = None
        producer = None
        if at.startswith("$capture."):
            name = at.removeprefix("$capture.")
            # The most recent binding wins, including a malformed binding.
            for index in range(view_step - 2, -1, -1):
                if name in wire[index].get("captures", {}):
                    producer = wire[index]
                    binding = producer["captures"][name]
                    address_pointer = binding.get("pointer")
                    assertions = [a for a in producer.get("assertions", [])
                                  if a.get("pointer") == address_pointer and "eq" in a]
                    if (producer["tool"] != "unica.view" or producer.get("expect") != ["ok"]
                            or not isinstance(address_pointer, str)
                            or not re.fullmatch(r"/data/items/(0|[1-9]\d*)/at", address_pointer)
                            or len(assertions) != 1 or not isinstance(assertions[0]["eq"], str)):
                        raise ValueError(f"capture has no asserted list address: {path}:{name}")
                    at = assertions[0]["eq"]
                    capture = {"name": name, "producer_step": index + 1,
                               "pointer": address_pointer, "assertion": assertions[0]}
                    break
            if capture is None:
                raise ValueError(f"unresolved read address capture: {path}:{name}")
        address = re.fullmatch(rf"([^:\r\n]+):{re.escape(kind)}\.(?:[^.\\\r\n]|\\[.\\])+", at)
        if address is None:
            raise ValueError(f"read address is not a root object of {kind}: {path}:{at}")
        if producer is not None and producer.get("args", {}).get("at") != f"{address[1]}:{kind}":
            raise ValueError(f"capture belongs to another list: {path}:{at}")
        assertions = view.get("assertions", [])
        fields = [a for a in assertions if a.get("pointer") == pointer and "eq" in a]
        if not pointer.startswith("/data/props/") or len(fields) != 1:
            raise ValueError(f"read has no confirming data assertion: {path}:{view_step}:{pointer}")
        if any(a.get("pointer") == "/data/kind" and a.get("eq") != kind for a in assertions):
            raise ValueError(f"read asserts another kind: {path}:{view_step}")
        evidence[kind].append({
            "scenario": path.as_posix(), "declared_formats": data.get("formats"),
            "workspace": data["workspace"], "at": at, "view_step": view_step,
            "assertion": fields[0], "address_capture": capture,
        })
    return evidence


def require_result_assertions(assertions: list[dict], required: dict, label: Path) -> None:
    for pointer, value in required.items():
        actual = [a for a in assertions if a.get("pointer") == pointer]
        # JSON equality also distinguishes an integer diagnostic count from true.
        if canonical(actual) != canonical([{"pointer": pointer, "eq": value}]):
            raise ValueError(f"check has no exact result/validator assertion: {label}:{pointer}")


def check_evidence(repo: Path, roots: list[str], selections: tuple) -> dict:
    evidence = {kind: [] for kind in roots}
    for scenario, kind, check_step, at, validator, outcome, extra_assertions in selections:
        if kind not in evidence or not re.fullmatch(
                rf"([^:\r\n]+):{re.escape(kind)}\.(?:[^.\\\r\n]|\\[.\\])+", at):
            raise ValueError(f"check target is not a registered root object of {kind}: {at}")
        path, data = load_scenario(repo, scenario)
        if not 1 <= check_step <= len(data["wire"]):
            raise ValueError(f"invalid check step: {path}:{check_step}")
        step = data["wire"][check_step - 1]
        if (step["tool"] != "unica.check" or step.get("expect") != ["ok"]
                or step.get("args") != {"at": at}):
            raise ValueError(f"check witness must target the exact object: {path}:{check_step}")
        if outcome not in {"passed", "failed", "warning"}:
            raise ValueError(f"unsupported check outcome: {outcome}")
        status = "passed" if outcome == "warning" else outcome
        if (step.get("status", status) != status
                or step.get("validators", [validator]) != [validator]):
            raise ValueError(f"check witness is readable or has another validator: {path}:{check_step}")
        assertions = step.get("assertions", [])
        required_assertions = dict(extra_assertions)
        if outcome in {"passed", "failed"}:
            required_assertions.update({"/data/status": outcome, "/data/validators": [validator]})
        # Exact named requests bind the direct cases. A response address, when
        # asserted, must agree; Subsystem separately requires its presence.
        if any(a.get("pointer") == "/data/at" for a in assertions):
            required_assertions["/data/at"] = at
        require_result_assertions(assertions, required_assertions, path)
        if outcome == "warning":
            required = {"metadataPath": at.split(":", 1)[1], "validator": validator,
                        "code": "validation_warning", "severity": "warning"}
            diagnostics = [a for a in assertions if a.get("pointer") == "/data/diagnostics"
                           and ("contains" in a or "eq" in a)]
            if not diagnostics:
                raise ValueError(f"check has no asserted warning: {path}:{check_step}")
            observed = set()
            for assertion in diagnostics:
                values = [assertion["contains"]] if "contains" in assertion else assertion["eq"]
                matching = []
                if isinstance(values, list):
                    matching = [item for item in values if isinstance(item, dict)
                                and all(item.get(k) == v for k, v in required.items())]
                if not matching:
                    raise ValueError(f"check warning has another object/validator/result: {path}:{check_step}")
                observed.update(canonical(item) for item in matching)
            count = extra_assertions.get("/data/diagnosticCount")
            if count is not None and len(observed) != count:
                raise ValueError(f"check warning count lacks distinct diagnostic assertions: {path}:{check_step}")
        evidence[kind].append({
            "scenario": path.as_posix(), "workspace": data["workspace"],
            "declared_formats": data.get("formats"), "at": at, "check_step": check_step,
            "validator": validator, "outcome": outcome, "assertions": assertions,
        })
    return evidence


def repository_file(repo: Path, relative: str) -> None:
    path = Path(relative)
    if path.is_absolute() or ".." in path.parts or not (repo / path).is_file():
        raise ValueError(f"invalid evidence source: {relative!r}")


def address_evidence(repo: Path, roots: list[str], report: dict | None) -> tuple[dict, dict]:
    positive = {kind: [] for kind in roots}
    observations = {kind: [] for kind in roots}
    if report is None:
        return positive, observations
    if report.get("schema_version") != 1:
        raise ValueError("unsupported address report schema")
    producers = {}
    test_root = repo / "crates/unica-coder/src/infrastructure/v13_read/tests"
    for path in sorted(test_root.rglob("*.rs")):
        for name in re.findall(r"\bfn\s+(\w+)\s*\(", path.read_text()):
            if name in ADDRESS["NATIVE_SHARDS"]:
                if name in producers:
                    raise ValueError(f"ambiguous address evidence test: {name}")
                producers[name] = path.relative_to(repo).as_posix()
    seen = set()
    present = set()
    for row in report["rows"]:
        key = row["key"]
        if (key["source_kind"] != "configuration" or key["role"] is not None
                or key["owner_shape"] != [{"kind": "Configuration", "named": False, "role": None}]
                or key["branch"] not in positive):
            continue
        structural = ADDRESS["structural_key"](key)
        if structural in seen:
            raise ValueError("duplicate root address row")
        seen.add(structural)
        kind = key["branch"]
        if not row["witnesses"]:
            raise ValueError(f"root address row has no witness: {kind}")
        for witness in row["witnesses"]:
            # Reuse the address map's native producers, while keeping only
            # Configuration-root keys. Child/extension evidence stays out.
            producer = witness["test_case"]
            if producer not in producers:
                raise ValueError(f"invalid root address evidence test: {producer}")
            ADDRESS["validate_record"]({"schema_version": 1, "record_type": "evidence",
                                        "key": key, "witness": witness}, producer)
            repository_file(repo, witness["origin"])
            if witness["owner"] != "main:Configuration":
                raise ValueError("root address witness has another owner")
            if witness["reason"] is None and not (
                    witness["at"] == f"main:{kind}"
                    or witness["at"].startswith(f"main:{kind}.")):
                raise ValueError("root address witness names another kind")
            item = {"key": key, "witness": witness, "test_file": producers[producer]}
            target = observations if witness["reason"] is not None else positive
            target[kind].append(item)
        present.add(kind)
    if missing := set(roots) - present:
        raise ValueError(f"address report lacks root rows: {', '.join(sorted(missing))}")
    return positive, observations


def cell(evidence: list[dict]) -> dict:
    return {"status": "evidenced" if evidence else "no_evidence",
            "evidence": sorted(evidence, key=canonical)}


def generate_report(repo: Path = REPO, address_report: dict | None = None,
                    scenarios: tuple[str, ...] = SCENARIOS,
                    read_selections: tuple = READ_SELECTIONS,
                    check_selections: tuple = CHECK_SELECTIONS) -> dict:
    roots = root_kinds(repo)
    creations = creation_evidence(repo, roots, scenarios)
    reads = read_evidence(repo, roots, read_selections)
    checks = check_evidence(repo, roots, check_selections)
    addresses, observations = address_evidence(repo, roots, address_report)
    rows = [{"kind": kind, "read": cell(creations[kind] + reads[kind]), "write": cell(creations[kind]),
             "address": {**cell(addresses[kind]), "absence_observations": sorted(
                 observations[kind], key=canonical)}, "check": cell(checks[kind])} for kind in roots]
    return {
        "schema_version": 1,
        "scope": {
            "write_operation": "object.create", "scenarios": list(scenarios),
            "read_only_scenarios": sorted({selection[0] for selection in read_selections}),
            "check_scenarios": sorted({selection[0] for selection in check_selections}),
            "corpus_evidence": "References to existing asserted corpus steps; no new execution implied.",
            "declared_formats": "Scenario admission, not evidence of execution on every format.",
            "address": None if address_report is None else {
                field: address_report[field] for field in ("revision", "dirty", "scope")},
            "address_boundary": "Configuration roots only; preserve line, owner and read/roundtrip level.",
            "no_evidence": "No witness in this slice; neither unsupported nor unplanned.",
            "complete": False,
        },
        "rows": rows,
    }


def validate_report(report: dict, expected: dict) -> None:
    if report.get("schema_version") != 1 or report.get("scope") != expected["scope"]:
        raise ValueError("invalid kind coverage schema or scope")
    rows = {}
    for row in report["rows"]:
        kind = row["kind"]
        if kind in rows:
            raise ValueError(f"duplicate kind row: {kind}")
        rows[kind] = row
    expected_rows = {row["kind"]: row for row in expected["rows"]}
    if missing := expected_rows.keys() - rows.keys():
        raise ValueError(f"missing registered kind row: {', '.join(sorted(missing))}")
    if extra := rows.keys() - expected_rows.keys():
        raise ValueError(f"unregistered kind row: {', '.join(sorted(extra))}")
    for kind, row in rows.items():
        if set(row) != {"kind", *COLUMNS}:
            raise ValueError(f"invalid columns: {kind}")
        for column in COLUMNS:
            if row[column] != expected_rows[kind][column]:
                raise ValueError(f"invalid or missing evidence: {kind}.{column}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--address-report", type=Path, help="JSON from generate-address-coverage.py")
    parser.add_argument("--check", type=Path, help="validate a saved slice against the current evidence")
    args = parser.parse_args()
    try:
        addresses = json.loads(args.address_report.read_text()) if args.address_report else None
        expected = generate_report(address_report=addresses)
        if args.check:
            validate_report(json.loads(args.check.read_text()), expected)
        else:
            print(json.dumps(expected, ensure_ascii=False, sort_keys=True, indent=2))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
