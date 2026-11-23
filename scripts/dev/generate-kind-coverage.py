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


def creation_evidence(repo: Path, roots: list[str], scenarios: tuple[str, ...]) -> dict:
    evidence = {kind: [] for kind in roots}
    for scenario in scenarios:
        path = SCENARIO_DIR / f"{scenario}.json"
        data = json.loads((repo / path).read_text(encoding="utf-8"))
        wire = data["wire"]
        manifest = (repo / MANIFEST_DIR / f"{scenario}.steps").read_text().splitlines()
        if data["id"] != scenario or [step["tool"] for step in wire] != manifest:
            raise ValueError(f"invalid scenario/step manifest: {path}")
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
                    scenarios: tuple[str, ...] = SCENARIOS) -> dict:
    roots = root_kinds(repo)
    creations = creation_evidence(repo, roots, scenarios)
    addresses, observations = address_evidence(repo, roots, address_report)
    rows = [{"kind": kind, "read": cell(creations[kind]), "write": cell(creations[kind]),
             "address": {**cell(addresses[kind]), "absence_observations": sorted(
                 observations[kind], key=canonical)}, "check": cell([])} for kind in roots]
    return {
        "schema_version": 1,
        "scope": {
            "write_operation": "object.create", "scenarios": list(scenarios),
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
