#!/usr/bin/env python3

from __future__ import annotations

import copy
import json
import os
import posixpath
import re
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]

# Test expectations are intentionally literal rather than imported from the
# validator. A wrong validator catalog must not rewrite the test oracle.
EXPECTED_SHARDS = (
    "tests/fixtures/v013/domain-parity/view-find.json",
    "tests/fixtures/v013/domain-parity/apply-metadata.json",
    "tests/fixtures/v013/domain-parity/apply-form-resource.json",
    "tests/fixtures/v013/domain-parity/apply-dcs-mxl.json",
    "tests/fixtures/v013/domain-parity/check-diff.json",
    "tests/fixtures/v013/domain-parity/search-docs.json",
    "tests/fixtures/v013/domain-parity/run.json",
)

IMMUTABLE_BASELINE_NAMES = (
    "unica.build.dump",
    "unica.build.load",
    "unica.build.make",
    "unica.build.run",
    "unica.build.update",
    "unica.cf.edit",
    "unica.cf.info",
    "unica.cf.init",
    "unica.cf.validate",
    "unica.cfe.borrow",
    "unica.cfe.diff",
    "unica.cfe.init",
    "unica.cfe.patch_method",
    "unica.cfe.validate",
    "unica.code.definition",
    "unica.code.diagnostics",
    "unica.code.graph",
    "unica.code.outline",
    "unica.code.patch",
    "unica.code.search",
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
    "unica.documentation.get",
    "unica.documentation.search",
    "unica.epf.init",
    "unica.erf.init",
    "unica.form.add",
    "unica.form.compile",
    "unica.form.edit",
    "unica.form.info",
    "unica.form.remove",
    "unica.form.validate",
    "unica.help.add",
    "unica.interface.edit",
    "unica.interface.validate",
    "unica.meta.add",
    "unica.meta.edit",
    "unica.meta.info",
    "unica.meta.remove",
    "unica.mxl.compile",
    "unica.mxl.decompile",
    "unica.mxl.info",
    "unica.mxl.validate",
    "unica.project.map",
    "unica.project.status",
    "unica.role.compile",
    "unica.role.edit",
    "unica.role.info",
    "unica.role.validate",
    "unica.runtime.execute",
    "unica.runtime.job.cancel",
    "unica.runtime.job.list",
    "unica.runtime.job.logs",
    "unica.runtime.job.start",
    "unica.runtime.job.status",
    "unica.runtime.job.wait",
    "unica.source.children",
    "unica.source.locate",
    "unica.source.read",
    "unica.source.resolve",
    "unica.source.resources",
    "unica.standards.explain",
    "unica.standards.search",
    "unica.subsystem.compile",
    "unica.subsystem.edit",
    "unica.subsystem.info",
    "unica.subsystem.validate",
    "unica.support.edit",
    "unica.template.add",
    "unica.template.remove",
    "unica.xdto.edit",
    "unica.xdto.info",
)

IMMUTABLE_RUNTIME_JOB_NAMES = {
    "unica.runtime.job.cancel",
    "unica.runtime.job.list",
    "unica.runtime.job.logs",
    "unica.runtime.job.start",
    "unica.runtime.job.status",
    "unica.runtime.job.wait",
}


class InventoryError(ValueError):
    pass


PARITY_SHARDS = (
    "tests/fixtures/v013/domain-parity/view-find.json",
    "tests/fixtures/v013/domain-parity/apply-metadata.json",
    "tests/fixtures/v013/domain-parity/apply-form-resource.json",
    "tests/fixtures/v013/domain-parity/apply-dcs-mxl.json",
    "tests/fixtures/v013/domain-parity/check-diff.json",
    "tests/fixtures/v013/domain-parity/search-docs.json",
    "tests/fixtures/v013/domain-parity/run.json",
)

TOP_LEVEL_KEYS = {
    "schemaVersion",
    "complete",
    "baselineDispositions",
    "cases",
    "newCapabilities",
}
NATIVE_ENTRIES = {"view", "apply", "find", "search", "check", "diff", "run", "docs"}
OPERATION_ENTRIES = {"apply", "run"}
RUN_OPERATIONS = {
    "source.create",
    "source.attach",
    "infobase.create",
    "infobase.build",
    "source.dump",
    "source.convert",
    "artifact.make",
    "artifact.load",
    "syntax.check",
    "test.run",
    "client.run",
    "extension.sync",
}
DISPOSITIONS = {"mapped", "absorbed", "transport-replaced", "removed"}
RUNTIME_JOB_NAMES = {
    "unica.runtime.job.cancel",
    "unica.runtime.job.list",
    "unica.runtime.job.logs",
    "unica.runtime.job.start",
    "unica.runtime.job.status",
    "unica.runtime.job.wait",
}


def _unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise InventoryError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def load_json_document(path: Path) -> object:
    try:
        with path.open(encoding="utf-8") as source:
            return json.load(source, object_pairs_hook=_unique_json_object)
    except InventoryError:
        raise
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise InventoryError(f"cannot load JSON document {path}: {error}") from error


def _require_exact_keys(value: object, expected: set[str], label: str) -> dict[str, object]:
    if type(value) is not dict:
        raise InventoryError(f"{label} must be an object")
    actual = set(value)
    if actual != expected:
        raise InventoryError(
            f"{label} must have exact keys {sorted(expected)}; got {sorted(actual)}"
        )
    return value


def _require_nonempty_string(value: object, label: str) -> str:
    if type(value) is not str or not value:
        raise InventoryError(f"{label} must be a non-empty string")
    return value


def _validate_baseline_names(baseline_names: object) -> tuple[str, ...]:
    if type(baseline_names) not in (list, tuple):
        raise InventoryError("immutable baseline names must be a list or tuple")
    names = tuple(baseline_names)
    if len(names) != 74:
        raise InventoryError(f"immutable baseline must contain 74 names; got {len(names)}")
    if any(type(name) is not str or not name for name in names):
        raise InventoryError("immutable baseline names must be non-empty strings")
    if len(set(names)) != 74:
        raise InventoryError("immutable baseline must contain 74 unique names")
    runtime_jobs = {name for name in names if name.startswith("unica.runtime.job.")}
    if runtime_jobs != RUNTIME_JOB_NAMES:
        raise InventoryError(
            "immutable baseline must contain the exact six runtime job names"
        )
    return names


def load_immutable_baseline_names(path: Path) -> tuple[str, ...]:
    document = load_json_document(path)
    if type(document) is not dict:
        raise InventoryError("immutable baseline document must be an object")
    schema_version = document.get("schemaVersion")
    if type(schema_version) is not int or schema_version != 1:
        raise InventoryError("immutable baseline schemaVersion must be integer 1")
    wire = document.get("wire")
    if type(wire) is not dict:
        raise InventoryError("immutable baseline wire must be an object")
    tool_count = wire.get("toolCount")
    if type(tool_count) is not int or tool_count != 74:
        raise InventoryError("immutable baseline wire.toolCount must be integer 74")
    names = _validate_baseline_names(wire.get("toolNames"))
    if tool_count != len(names):
        raise InventoryError("immutable baseline toolCount must match toolNames")
    return names


def tracked_repository_paths(repo_root: Path) -> frozenset[str]:
    completed = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=repo_root,
        check=True,
        capture_output=True,
    )
    return frozenset(
        os.fsdecode(raw_path)
        for raw_path in completed.stdout.split(b"\0")
        if raw_path
    )


def load_repository_inputs(
    repo_root: Path,
) -> tuple[dict[str, object], tuple[str, ...], frozenset[str]]:
    documents: dict[str, object] = {}
    for relative in PARITY_SHARDS:
        path = repo_root.joinpath(*relative.split("/"))
        if not path.exists():
            raise InventoryError(f"missing parity shard: {relative}")
        documents[relative] = load_json_document(path)

    tracked_paths = tracked_repository_paths(repo_root)
    untracked_shards = [path for path in PARITY_SHARDS if path not in tracked_paths]
    if untracked_shards:
        raise InventoryError(f"parity shard must be tracked: {untracked_shards[0]}")

    baseline_names = load_immutable_baseline_names(
        repo_root / "tests/fixtures/migration/v0.12.3-baseline.json"
    )
    return documents, baseline_names, tracked_paths


def _validate_operation(entry: str, value: object, label: str) -> str | None:
    if entry not in OPERATION_ENTRIES:
        return None
    operation = _require_nonempty_string(value, f"{label} operation")
    if entry == "run" and operation not in RUN_OPERATIONS:
        raise InventoryError(f"{label} operation is not in the exact run dictionary")
    return operation


def _validate_successor(value: object, label: str) -> tuple[str, str | None]:
    if type(value) is not dict:
        raise InventoryError(f"{label} successor must be an object")
    entry = _require_nonempty_string(value.get("entry"), f"{label} successor entry")
    if entry not in NATIVE_ENTRIES:
        raise InventoryError(f"{label} successor entry must be one of the eight native entries")
    expected_keys = {"entry", "operation"} if entry in OPERATION_ENTRIES else {"entry"}
    _require_exact_keys(value, expected_keys, f"{label} successor")
    operation = _validate_operation(entry, value.get("operation"), f"{label} successor")
    return entry, operation


def _validate_case_ids(value: object, label: str) -> tuple[str, ...]:
    if type(value) is not list or not value:
        raise InventoryError(f"{label} caseIds must be a non-empty array")
    case_ids: list[str] = []
    for index, raw_case_id in enumerate(value):
        case_id = _require_nonempty_string(
            raw_case_id, f"{label} caseIds[{index}]"
        )
        if case_id in case_ids:
            raise InventoryError(f"{label} caseIds must be unique")
        case_ids.append(case_id)
    return tuple(case_ids)


def _projection_identity(value: object, label: str) -> tuple[str, str]:
    if type(value) is not dict:
        raise InventoryError(f"{label} projection must be an object")
    kind = _require_nonempty_string(value.get("kind"), f"{label} projection kind")
    if kind == "native-task":
        _require_exact_keys(value, {"kind", "method"}, f"{label} projection")
        method = _require_nonempty_string(value["method"], f"{label} projection method")
        if method not in {"tasks/get", "tasks/cancel"}:
            raise InventoryError(f"{label} projection has unsupported native Task method")
        return kind, method
    if kind == "compatibility-tool":
        _require_exact_keys(value, {"kind", "tool"}, f"{label} projection")
        tool = _require_nonempty_string(value["tool"], f"{label} projection tool")
        if tool not in {"unica.task.get", "unica.task.result", "unica.task.cancel"}:
            raise InventoryError(f"{label} projection has unsupported compatibility tool")
        return kind, tool
    raise InventoryError(f"{label} projection has unsupported kind")


def _validate_fixture_path(
    fixture: str,
    *,
    repo_root: Path,
    tracked_paths: set[str] | frozenset[str],
    label: str,
) -> None:
    if "\\" in fixture:
        raise InventoryError(f"{label} fixture must use canonical POSIX separators")
    if fixture.startswith("/") or re.match(r"^[A-Za-z]:", fixture):
        raise InventoryError(f"{label} fixture must be repository-relative")
    parts = fixture.split("/")
    if not fixture or any(part in {"", ".", ".."} for part in parts):
        raise InventoryError(f"{label} fixture must be a normalized non-empty path")
    if posixpath.normpath(fixture) != fixture:
        raise InventoryError(f"{label} fixture must be a normalized POSIX path")

    root = repo_root.resolve()
    candidate = root.joinpath(*parts)
    resolved = candidate.resolve(strict=False)
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise InventoryError(f"{label} fixture escapes the repository root") from error

    current = root
    for part in parts:
        current = current / part
        if current.is_symlink():
            raise InventoryError(f"{label} fixture path contains a symlink")
    if not candidate.exists():
        raise InventoryError(f"{label} fixture does not exist")
    if not candidate.is_file():
        raise InventoryError(f"{label} fixture must be a regular file")
    if fixture not in tracked_paths:
        raise InventoryError(f"{label} fixture must be tracked by git")


def validate_inventory(
    documents: object,
    *,
    repo_root: Path,
    baseline_names: object,
    tracked_paths: set[str] | frozenset[str],
) -> None:
    if type(documents) is not dict or set(documents) != set(PARITY_SHARDS):
        raise InventoryError("inventory must contain the exact parity shard set")
    names = _validate_baseline_names(baseline_names)
    baseline_set = set(names)

    complete_values: list[bool] = []
    seen_legacy_tools: set[str] = set()
    seen_new_capability_ids: set[str] = set()
    seen_case_ids: set[str] = set()
    case_entries: set[str] = set()
    run_case_operations: set[str] = set()
    case_references: dict[str, tuple[tuple[str, str | None], str]] = {}
    case_identities: dict[str, tuple[str, str | None]] = {}

    for shard in PARITY_SHARDS:
        document = _require_exact_keys(documents[shard], TOP_LEVEL_KEYS, shard)
        schema_version = document["schemaVersion"]
        if type(schema_version) is not int:
            raise InventoryError(f"{shard} schemaVersion must be an integer")
        if schema_version != 2:
            raise InventoryError(f"{shard} schemaVersion must be 2")
        complete = document["complete"]
        if type(complete) is not bool:
            raise InventoryError(f"{shard} complete must be a boolean")
        complete_values.append(complete)

        dispositions = document["baselineDispositions"]
        if type(dispositions) is not list:
            raise InventoryError(f"{shard} baselineDispositions must be an array")
        for index, raw_row in enumerate(dispositions):
            label = f"{shard} baselineDispositions[{index}]"
            if type(raw_row) is not dict:
                raise InventoryError(f"{label} disposition row object is required")
            legacy_tool = _require_nonempty_string(
                raw_row.get("legacyTool"), f"{label} legacyTool"
            )
            if legacy_tool not in baseline_set:
                raise InventoryError(f"{label} legacyTool is not in the immutable baseline")
            if legacy_tool in seen_legacy_tools:
                raise InventoryError(f"duplicate legacyTool across parity shards: {legacy_tool}")
            seen_legacy_tools.add(legacy_tool)

            row = _require_exact_keys(raw_row, {"legacyTool", "variants"}, label)
            variants = row["variants"]
            if type(variants) is not list or not variants:
                raise InventoryError(f"{label} variants must be a non-empty array")
            seen_variants: set[str] = set()
            for variant_index, raw_variant in enumerate(variants):
                variant_label = f"{label} variants[{variant_index}]"
                if type(raw_variant) is not dict:
                    raise InventoryError(f"{variant_label} must be an object")
                legacy_variant = _require_nonempty_string(
                    raw_variant.get("legacyVariant"),
                    f"{variant_label} legacyVariant",
                )
                if legacy_variant in seen_variants:
                    raise InventoryError(f"{label} has duplicate legacyVariant")
                seen_variants.add(legacy_variant)
                disposition = _require_nonempty_string(
                    raw_variant.get("disposition"), f"{variant_label} disposition"
                )
                if disposition not in DISPOSITIONS:
                    raise InventoryError(f"{variant_label} has unsupported disposition")
                if disposition in {"mapped", "absorbed"}:
                    variant = _require_exact_keys(
                        raw_variant,
                        {
                            "legacyVariant",
                            "disposition",
                            "successor",
                            "caseIds",
                        },
                        variant_label,
                    )
                    successor_identity = _validate_successor(
                        variant["successor"], variant_label
                    )
                    for case_id in _validate_case_ids(
                        variant["caseIds"], variant_label
                    ):
                        previous = case_references.get(case_id)
                        if previous is not None:
                            raise InventoryError(
                                f"caseId {case_id} is referenced by multiple capabilities"
                            )
                        case_references[case_id] = (
                            successor_identity,
                            variant_label,
                        )
                elif disposition == "transport-replaced":
                    variant = _require_exact_keys(
                        raw_variant,
                        {"legacyVariant", "disposition", "projections"},
                        variant_label,
                    )
                    projections = variant["projections"]
                    if type(projections) is not list or not projections:
                        raise InventoryError(
                            f"{variant_label} projections must be a non-empty array"
                        )
                    projection_ids: set[tuple[str, str]] = set()
                    for projection_index, projection in enumerate(projections):
                        projection_id = _projection_identity(
                            projection,
                            f"{variant_label} projections[{projection_index}]",
                        )
                        if projection_id in projection_ids:
                            raise InventoryError(
                                f"{variant_label} has duplicate projection"
                            )
                        projection_ids.add(projection_id)
                else:
                    variant = _require_exact_keys(
                        raw_variant,
                        {"legacyVariant", "disposition", "rejectionEvidence"},
                        variant_label,
                    )
                    _require_nonempty_string(
                        variant["rejectionEvidence"],
                        f"{variant_label} rejectionEvidence",
                    )

        new_capabilities = document["newCapabilities"]
        if type(new_capabilities) is not list:
            raise InventoryError(f"{shard} newCapabilities must be an array")
        for index, raw_capability in enumerate(new_capabilities):
            label = f"{shard} newCapabilities[{index}]"
            capability = _require_exact_keys(
                raw_capability,
                {"capabilityId", "successor", "caseIds", "rationale"},
                label,
            )
            capability_id = _require_nonempty_string(
                capability["capabilityId"], f"{label} capabilityId"
            )
            if capability_id in seen_new_capability_ids:
                raise InventoryError(
                    f"duplicate capabilityId across parity shards: {capability_id}"
                )
            seen_new_capability_ids.add(capability_id)
            successor_identity = _validate_successor(capability["successor"], label)
            _require_nonempty_string(capability["rationale"], f"{label} rationale")
            for case_id in _validate_case_ids(capability["caseIds"], label):
                previous = case_references.get(case_id)
                if previous is not None:
                    raise InventoryError(
                        f"caseId {case_id} is referenced by multiple capabilities"
                    )
                case_references[case_id] = (successor_identity, label)

        cases = document["cases"]
        if type(cases) is not list:
            raise InventoryError(f"{shard} cases must be an array")
        for index, raw_case in enumerate(cases):
            label = f"{shard} cases[{index}]"
            if type(raw_case) is not dict:
                raise InventoryError(f"{label} must be an object")
            entry = _require_nonempty_string(raw_case.get("entry"), f"{label} entry")
            if entry not in NATIVE_ENTRIES:
                raise InventoryError(f"{label} entry must be one of the eight native entries")
            expected_keys = {"caseId", "entry", "mode", "fixture", "expected"}
            if entry in OPERATION_ENTRIES:
                expected_keys.add("operation")
            case = _require_exact_keys(
                raw_case,
                expected_keys,
                label,
            )
            case_id = _require_nonempty_string(case["caseId"], f"{label} caseId")
            if case_id in seen_case_ids:
                raise InventoryError(f"duplicate caseId across parity shards: {case_id}")
            seen_case_ids.add(case_id)
            operation = _validate_operation(entry, case.get("operation"), label)
            case_entries.add(entry)
            if entry == "run":
                run_case_operations.add(operation)
            case_identities[case_id] = (entry, operation)
            _require_nonempty_string(case["mode"], f"{label} mode")
            fixture = _require_nonempty_string(case["fixture"], f"{label} fixture")
            expected = case["expected"]
            if type(expected) is not dict or not expected:
                raise InventoryError(f"{label} expected must be a non-empty object")
            _validate_fixture_path(
                fixture,
                repo_root=repo_root,
                tracked_paths=tracked_paths,
                label=label,
            )

    for case_id, (successor_identity, label) in case_references.items():
        case_identity = case_identities.get(case_id)
        if case_identity is None:
            raise InventoryError(f"{label} references unknown caseId {case_id}")
        if case_identity != successor_identity:
            raise InventoryError(
                f"{label} caseId {case_id} does not match successor identity"
            )

    if len(set(complete_values)) != 1:
        raise InventoryError("parity shards have mixed complete values")
    if complete_values[0]:
        missing_runtime_jobs = RUNTIME_JOB_NAMES - seen_legacy_tools
        if missing_runtime_jobs:
            raise InventoryError(
                "complete inventory is missing runtime job dispositions: "
                + ", ".join(sorted(missing_runtime_jobs))
            )
        if len(seen_legacy_tools) != 74 or seen_legacy_tools != baseline_set:
            raise InventoryError("complete inventory must account for all 74 baseline names")
        if case_entries != NATIVE_ENTRIES:
            raise InventoryError("complete inventory must cover all eight native entries")
        if run_case_operations != RUN_OPERATIONS:
            raise InventoryError(
                "complete inventory must cover all twelve run operations"
            )
        unowned_cases = set(case_identities) - set(case_references)
        if unowned_cases:
            raise InventoryError(
                "complete inventory contains unowned executable case: "
                + sorted(unowned_cases)[0]
            )


class V013ParityInventoryTest(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.documents = {
            shard: {
                "schemaVersion": 2,
                "complete": False,
                "baselineDispositions": [],
                "cases": [],
                "newCapabilities": [],
            }
            for shard in EXPECTED_SHARDS
        }
        self.tracked_paths: set[str] = set()
        self.fixture = "fixtures/case.json"
        self._write_fixture(self.fixture)

    def _write_fixture(self, relative: str, *, tracked: bool = True) -> Path:
        path = self.root.joinpath(*relative.split("/"))
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("{}\n", encoding="utf-8")
        if tracked:
            self.tracked_paths.add(relative)
        return path

    def _validate(self, documents: dict[str, object] | None = None) -> None:
        validate_inventory(
            documents if documents is not None else self.documents,
            repo_root=self.root,
            baseline_names=IMMUTABLE_BASELINE_NAMES,
            tracked_paths=self.tracked_paths,
        )

    def _one_mapped_row(self) -> dict[str, object]:
        return {
            "legacyTool": "unica.meta.add",
            "variants": [
                {
                    "legacyVariant": "default",
                    "disposition": "mapped",
                    "successor": {"entry": "apply", "operation": "object.create"},
                    "caseIds": ["meta-object-create-basic"],
                }
            ],
        }

    @staticmethod
    def _first_variant(row: dict[str, object]) -> dict[str, object]:
        return row["variants"][0]

    def _one_case(self) -> dict[str, object]:
        return {
            "caseId": "meta-object-create-basic",
            "entry": "apply",
            "operation": "object.create",
            "mode": "fast",
            "fixture": self.fixture,
            "expected": {"outcome": "changed"},
        }

    def _set_first_row(self, row: object) -> None:
        self.documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]

    def _set_first_case(self, case: object) -> None:
        self.documents[EXPECTED_SHARDS[0]]["cases"] = [case]

    def _complete_documents(self) -> dict[str, object]:
        documents = copy.deepcopy(self.documents)
        for shard in EXPECTED_SHARDS:
            documents[shard]["complete"] = True

        rows: list[dict[str, object]] = []
        cases: list[dict[str, object]] = []
        mappable_index = 0
        legacy_run_variants = (
            ("operation=config-init", "source.create"),
            ("operation=config-init;sourceSet=external", "source.attach"),
            ("operation=init", "infobase.create"),
            ("operation=build", "infobase.build"),
            ("operation=dump", "source.dump"),
            ("operation=convert", "source.convert"),
            ("operation=make", "artifact.make"),
            ("operation=load", "artifact.load"),
            ("operation=syntax", "syntax.check"),
            ("operation=test", "test.run"),
            ("operation=launch", "client.run"),
            ("operation=extensions", "extension.sync"),
        )
        non_run_entries = ("view", "apply", "find", "search", "check", "diff", "docs")
        for index, legacy_tool in enumerate(IMMUTABLE_BASELINE_NAMES):
            if legacy_tool == "unica.runtime.job.status":
                rows.append(
                    {
                        "legacyTool": legacy_tool,
                        "variants": [
                            {
                                "legacyVariant": "default",
                                "disposition": "transport-replaced",
                                "projections": [
                                    {"kind": "native-task", "method": "tasks/get"},
                                    {
                                        "kind": "compatibility-tool",
                                        "tool": "unica.task.get",
                                    },
                                ],
                            }
                        ],
                    }
                )
            elif legacy_tool == "unica.runtime.execute":
                variants = []
                for variant_index, (legacy_variant, operation) in enumerate(
                    legacy_run_variants
                ):
                    case_id = f"runtime-variant-{variant_index:02d}"
                    variants.append(
                        {
                            "legacyVariant": legacy_variant,
                            "disposition": "mapped",
                            "successor": {"entry": "run", "operation": operation},
                            "caseIds": [case_id],
                        }
                    )
                    cases.append(
                        {
                            "caseId": case_id,
                            "entry": "run",
                            "operation": operation,
                            "mode": "direct",
                            "fixture": self.fixture,
                            "expected": {"outcome": "ok"},
                        }
                    )
                variants.append(
                    {
                        "legacyVariant": "operation=tools-download",
                        "disposition": "removed",
                        "rejectionEvidence": "package rejects removed engine delivery operation",
                    }
                )
                rows.append({"legacyTool": legacy_tool, "variants": variants})
            else:
                entry = non_run_entries[mappable_index % len(non_run_entries)]
                operation = "object.create" if entry == "apply" else None
                mappable_index += 1
                successor = {"entry": entry}
                if operation is not None:
                    successor["operation"] = operation
                case_id = f"legacy-{index:02d}"
                rows.append(
                    {
                        "legacyTool": legacy_tool,
                        "variants": [
                            {
                                "legacyVariant": "default",
                                "disposition": (
                                    "mapped"
                                    if legacy_tool == "unica.meta.add"
                                    else "absorbed"
                                ),
                                "successor": successor,
                                "caseIds": [case_id],
                            }
                        ],
                    }
                )
                case = {
                    "caseId": case_id,
                    "entry": entry,
                    "mode": "direct",
                    "fixture": self.fixture,
                    "expected": {"outcome": "ok"},
                }
                if operation is not None:
                    case["operation"] = operation
                cases.append(case)
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = rows
        documents[EXPECTED_SHARDS[1]]["cases"] = cases
        documents[EXPECTED_SHARDS[1]]["newCapabilities"] = []
        return documents

    def test_repository_inventory_is_valid_and_uses_exact_seven_shards(self) -> None:
        documents, baseline_names, tracked_paths = load_repository_inputs(REPO_ROOT)
        self.assertEqual(tuple(documents), EXPECTED_SHARDS)
        validate_inventory(
            documents,
            repo_root=REPO_ROOT,
            baseline_names=baseline_names,
            tracked_paths=tracked_paths,
        )

    def test_repository_adapter_preserves_newline_in_tracked_filename(self) -> None:
        repository = self.root / "repository"
        repository.mkdir()
        subprocess.run(
            ["git", "init", "--quiet"],
            cwd=repository,
            check=True,
            capture_output=True,
        )
        relative = "fixtures/line\nbreak.json"
        try:
            path = repository.joinpath(*relative.split("/"))
            path.parent.mkdir()
            path.write_text("{}\n", encoding="utf-8")
        except OSError as error:
            self.skipTest(f"filenames containing newlines are unavailable: {error}")
        subprocess.run(
            ["git", "add", "--", relative],
            cwd=repository,
            check=True,
            capture_output=True,
        )

        self.assertEqual(tracked_repository_paths(repository), {relative})

    def test_exact_empty_w0_skeleton_is_valid_synthetic_input(self) -> None:
        self._validate()

    def test_valid_partial_inventory_can_remain_incomplete(self) -> None:
        self._set_first_row(self._one_mapped_row())
        self._set_first_case(self._one_case())
        self._validate()

    def test_one_legacy_tool_can_own_distinct_variants_and_new_capability_is_separate(
        self,
    ) -> None:
        documents = copy.deepcopy(self.documents)
        for document in documents.values():
            document["schemaVersion"] = 2
            document["newCapabilities"] = []
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [
            {
                "legacyTool": "unica.runtime.execute",
                "variants": [
                    {
                        "legacyVariant": "operation=config-init",
                        "disposition": "mapped",
                        "successor": {"entry": "run", "operation": "source.create"},
                        "caseIds": ["runtime-config-init"],
                    },
                    {
                        "legacyVariant": "operation=syntax",
                        "disposition": "mapped",
                        "successor": {"entry": "run", "operation": "syntax.check"},
                        "caseIds": ["runtime-syntax"],
                    },
                ],
            }
        ]
        documents[EXPECTED_SHARDS[0]]["cases"] = [
            {
                "caseId": "runtime-config-init",
                "entry": "run",
                "operation": "source.create",
                "mode": "direct",
                "fixture": self.fixture,
                "expected": {"outcome": "ok"},
            },
            {
                "caseId": "runtime-syntax",
                "entry": "run",
                "operation": "syntax.check",
                "mode": "direct",
                "fixture": self.fixture,
                "expected": {"outcome": "ok"},
            },
            {
                "caseId": "run-artifact-make-new",
                "entry": "run",
                "operation": "artifact.make",
                "mode": "direct",
                "fixture": self.fixture,
                "expected": {"outcome": "ok"},
            },
        ]
        documents[EXPECTED_SHARDS[0]]["newCapabilities"] = [
            {
                "capabilityId": "run.artifact.make.new",
                "successor": {"entry": "run", "operation": "artifact.make"},
                "caseIds": ["run-artifact-make-new"],
                "rationale": "Synthetic new capability without a legacy predecessor.",
            }
        ]

        self._validate(documents)

    def test_new_capabilities_have_exact_unique_identity_and_case_ownership(self) -> None:
        capability = {
            "capabilityId": "apply.object.create.new-variant",
            "successor": {"entry": "apply", "operation": "object.create"},
            "caseIds": ["meta-object-create-basic"],
            "rationale": "This synthetic capability has no legacy predecessor.",
        }
        documents = copy.deepcopy(self.documents)
        documents[EXPECTED_SHARDS[0]]["newCapabilities"] = [capability]
        documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
        self._validate(documents)

        invalid_capabilities: tuple[object, ...] = (
            "apply.object.create.new-variant",
            {key: value for key, value in capability.items() if key != "rationale"},
            {**capability, "capabilityId": ""},
            {**capability, "successor": {"entry": "unknown"}},
            {**capability, "caseIds": []},
            {**capability, "rationale": ""},
            {**capability, "owner": "worker"},
        )
        for invalid in invalid_capabilities:
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["newCapabilities"] = [invalid]
            documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
            with self.subTest(capability=invalid), self.assertRaises(InventoryError):
                self._validate(documents)

        documents = copy.deepcopy(self.documents)
        documents[EXPECTED_SHARDS[0]]["newCapabilities"] = [capability]
        documents[EXPECTED_SHARDS[1]]["newCapabilities"] = [copy.deepcopy(capability)]
        documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
        with self.assertRaisesRegex(InventoryError, "duplicate capabilityId"):
            self._validate(documents)

        documents = copy.deepcopy(self.documents)
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [
            self._one_mapped_row()
        ]
        documents[EXPECTED_SHARDS[0]]["newCapabilities"] = [capability]
        documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
        with self.assertRaisesRegex(InventoryError, "multiple capabilities"):
            self._validate(documents)

    def test_loader_rejects_duplicate_json_keys_at_any_nesting_level(self) -> None:
        duplicate_json = self.root / "duplicate.json"
        duplicate_json.write_text(
            '{"schemaVersion":1,"complete":false,'
            '"baselineDispositions":[],"cases":['
            '{"caseId":"one","caseId":"two"}]}\n',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(InventoryError, "duplicate JSON key: caseId"):
            load_json_document(duplicate_json)

    def test_top_level_shape_and_strict_types_are_enforced(self) -> None:
        mutations = (
            ("must be an object", []),
            (
                "must have exact keys",
                {
                    "schemaVersion": 2,
                    "complete": False,
                    "baselineDispositions": [],
                    "cases": [],
                    "newCapabilities": [],
                    "owner": "worker",
                },
            ),
            (
                "schemaVersion must be an integer",
                {
                    "schemaVersion": True,
                    "complete": False,
                    "baselineDispositions": [],
                    "cases": [],
                    "newCapabilities": [],
                },
            ),
            (
                "schemaVersion must be 2",
                {
                    "schemaVersion": 3,
                    "complete": False,
                    "baselineDispositions": [],
                    "cases": [],
                    "newCapabilities": [],
                },
            ),
            (
                "complete must be a boolean",
                {
                    "schemaVersion": 2,
                    "complete": 0,
                    "baselineDispositions": [],
                    "cases": [],
                    "newCapabilities": [],
                },
            ),
            (
                "baselineDispositions must be an array",
                {
                    "schemaVersion": 2,
                    "complete": False,
                    "baselineDispositions": {},
                    "cases": [],
                    "newCapabilities": [],
                },
            ),
            (
                "cases must be an array",
                {
                    "schemaVersion": 2,
                    "complete": False,
                    "baselineDispositions": [],
                    "cases": {},
                    "newCapabilities": [],
                },
            ),
            (
                "newCapabilities must be an array",
                {
                    "schemaVersion": 2,
                    "complete": False,
                    "baselineDispositions": [],
                    "cases": [],
                    "newCapabilities": {},
                },
            ),
        )
        for message, replacement in mutations:
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]] = replacement
            with self.subTest(message=message), self.assertRaisesRegex(
                InventoryError, message
            ):
                self._validate(documents)

    def test_shard_set_is_exact(self) -> None:
        documents = copy.deepcopy(self.documents)
        documents.pop(EXPECTED_SHARDS[-1])
        with self.assertRaisesRegex(InventoryError, "exact parity shard set"):
            self._validate(documents)

        documents = copy.deepcopy(self.documents)
        documents["tests/fixtures/v013/domain-parity/extra.json"] = copy.deepcopy(
            documents[EXPECTED_SHARDS[0]]
        )
        with self.assertRaisesRegex(InventoryError, "exact parity shard set"):
            self._validate(documents)

    def test_disposition_requires_known_unique_legacy_tool(self) -> None:
        row = self._one_mapped_row()
        row["legacyTool"] = "unica.unknown"
        self._set_first_row(row)
        with self.assertRaisesRegex(InventoryError, "immutable baseline"):
            self._validate()

        duplicate = self._one_mapped_row()
        self.documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [duplicate]
        self.documents[EXPECTED_SHARDS[1]]["baselineDispositions"] = [
            copy.deepcopy(duplicate)
        ]
        with self.assertRaisesRegex(InventoryError, "duplicate legacyTool"):
            self._validate()

    def test_disposition_variants_have_exact_key_sets(self) -> None:
        invalid_variants = (
            {"legacyVariant": "default", "disposition": "unsupported"},
            {"legacyVariant": "default", "disposition": "mapped"},
            {
                "legacyVariant": "default",
                "disposition": "absorbed",
                "successor": {"entry": "view"},
                "projections": [],
            },
            {
                "legacyVariant": "default",
                "disposition": "transport-replaced",
                "projections": [],
            },
            {
                "legacyVariant": "default",
                "disposition": "transport-replaced",
                "projections": [{"kind": "native-task", "method": "tasks/get"}],
                "successor": {"entry": "view"},
            },
            {
                "legacyVariant": "default",
                "disposition": "removed",
                "rejectionEvidence": "",
            },
            {
                "legacyVariant": "default",
                "disposition": "removed",
                "rejectionEvidence": "tests/rejects_removed",
                "successor": {"entry": "view"},
            },
        )
        invalid_rows: list[object] = [
            {"legacyTool": "unica.meta.add"},
            {"legacyTool": "unica.meta.add", "variants": []},
            {
                "legacyTool": "unica.meta.add",
                "variants": [
                    {
                        "legacyVariant": "default",
                        "disposition": "removed",
                        "rejectionEvidence": "one",
                    },
                    {
                        "legacyVariant": "default",
                        "disposition": "removed",
                        "rejectionEvidence": "two",
                    },
                ],
            },
        ]
        invalid_rows.extend(
            {"legacyTool": "unica.meta.add", "variants": [variant]}
            for variant in invalid_variants
        )
        for row in invalid_rows:
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            with self.subTest(row=row), self.assertRaises(InventoryError):
                self._validate(documents)

    def test_disposition_rows_are_objects(self) -> None:
        self._set_first_row("unica.meta.add")
        with self.assertRaisesRegex(InventoryError, "disposition row object"):
            self._validate()

    def test_nested_discriminators_reject_non_string_json_values_cleanly(self) -> None:
        invalid_documents = []

        documents = copy.deepcopy(self.documents)
        row = self._one_mapped_row()
        self._first_variant(row)["disposition"] = []
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
        invalid_documents.append(documents)

        documents = copy.deepcopy(self.documents)
        row = self._one_mapped_row()
        self._first_variant(row)["successor"] = {"entry": []}
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
        invalid_documents.append(documents)

        documents = copy.deepcopy(self.documents)
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [
            {
                "legacyTool": "unica.runtime.job.status",
                "variants": [
                    {
                        "legacyVariant": "default",
                        "disposition": "transport-replaced",
                        "projections": [
                            {"kind": "native-task", "method": []},
                        ],
                    }
                ],
            }
        ]
        invalid_documents.append(documents)

        documents = copy.deepcopy(self.documents)
        case = self._one_case()
        case["entry"] = []
        documents[EXPECTED_SHARDS[0]]["cases"] = [case]
        invalid_documents.append(documents)

        for documents in invalid_documents:
            with self.subTest(documents=documents), self.assertRaises(InventoryError):
                self._validate(documents)

    def test_successor_shape_depends_on_entry(self) -> None:
        invalid_successors = (
            {"entry": "apply"},
            {"entry": "apply", "operation": ""},
            {"entry": "run"},
            {"entry": "run", "operation": ""},
            {"entry": "run", "operation": "arbitrary.command"},
            {"entry": "view", "operation": "object.create"},
            {"entry": "unknown"},
            {"entry": "view", "owner": "worker"},
            "view",
        )
        for successor in invalid_successors:
            row = self._one_mapped_row()
            self._first_variant(row)["successor"] = successor
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            with self.subTest(successor=successor), self.assertRaises(InventoryError):
                self._validate(documents)

    def test_run_successor_accepts_only_an_exact_typed_operation(self) -> None:
        row = self._one_mapped_row()
        self._first_variant(row)["successor"] = {
            "entry": "run",
            "operation": "source.create",
        }
        case = self._one_case()
        case["entry"] = "run"
        case["operation"] = "source.create"
        self._set_first_row(row)
        self._set_first_case(case)
        self._validate()

    def test_all_twelve_run_operations_are_the_literal_test_oracle(self) -> None:
        expected = (
            "source.create",
            "source.attach",
            "infobase.create",
            "infobase.build",
            "source.dump",
            "source.convert",
            "artifact.make",
            "artifact.load",
            "syntax.check",
            "test.run",
            "client.run",
            "extension.sync",
        )
        self.assertEqual(RUN_OPERATIONS, set(expected))
        for operation in expected:
            row = self._one_mapped_row()
            self._first_variant(row)["successor"] = {
                "entry": "run",
                "operation": operation,
            }
            case = self._one_case()
            case["entry"] = "run"
            case["operation"] = operation
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            documents[EXPECTED_SHARDS[0]]["cases"] = [case]
            with self.subTest(operation=operation):
                self._validate(documents)

    def test_apply_and_run_cases_require_their_exact_operation_identity(self) -> None:
        for entry in ("apply", "run"):
            case = self._one_case()
            case["entry"] = entry
            case.pop("operation")
            self._set_first_case(case)
            with self.subTest(entry=entry), self.assertRaisesRegex(
                InventoryError, "operation"
            ):
                self._validate()

        for entry, operation in (
            ("apply", "object.create"),
            ("run", "source.create"),
        ):
            case = self._one_case()
            case["entry"] = entry
            case["operation"] = operation
            self._set_first_case(case)
            with self.subTest(entry=entry, operation=operation):
                self._validate()

    def test_exact_projection_enum_and_duplicates_are_enforced(self) -> None:
        invalid_projection_lists = (
            [{"kind": "native-task", "method": "tasks/result"}],
            [{"kind": "compatibility-tool", "tool": "unica.task.unknown"}],
            [{"kind": "native-task", "method": "tasks/get", "extra": True}],
            [{"kind": "unknown", "method": "tasks/get"}],
            ["tasks/get"],
            [
                {"kind": "native-task", "method": "tasks/get"},
                {"kind": "native-task", "method": "tasks/get"},
            ],
        )
        for projections in invalid_projection_lists:
            row = {
                "legacyTool": "unica.runtime.job.status",
                "variants": [
                    {
                        "legacyVariant": "default",
                        "disposition": "transport-replaced",
                        "projections": projections,
                    }
                ],
            }
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            with self.subTest(projections=projections), self.assertRaises(
                InventoryError
            ):
                self._validate(documents)

    def test_all_five_projection_spellings_are_accepted(self) -> None:
        valid_projections = (
            {"kind": "native-task", "method": "tasks/get"},
            {"kind": "native-task", "method": "tasks/cancel"},
            {"kind": "compatibility-tool", "tool": "unica.task.get"},
            {"kind": "compatibility-tool", "tool": "unica.task.result"},
            {"kind": "compatibility-tool", "tool": "unica.task.cancel"},
        )
        for projection in valid_projections:
            row = {
                "legacyTool": "unica.runtime.job.status",
                "variants": [
                    {
                        "legacyVariant": "default",
                        "disposition": "transport-replaced",
                        "projections": [projection],
                    }
                ],
            }
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            with self.subTest(projection=projection):
                self._validate(documents)

    def test_case_has_exact_keys_and_strict_nonempty_fields(self) -> None:
        valid = self._one_case()
        invalid_cases: list[object] = [
            "meta-object-create-basic",
            {key: value for key, value in valid.items() if key != "expected"},
            {**valid, "owner": "worker"},
            {**valid, "caseId": ""},
            {**valid, "caseId": 1},
            {**valid, "entry": "unknown"},
            {**valid, "entry": "view"},
            {**valid, "mode": ""},
            {**valid, "mode": False},
            {**valid, "fixture": ""},
            {**valid, "fixture": 1},
            {**valid, "expected": {}},
            {**valid, "expected": []},
        ]
        for case in invalid_cases:
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["cases"] = [case]
            with self.subTest(case=case), self.assertRaises(InventoryError):
                self._validate(documents)

    def test_case_ids_are_globally_unique(self) -> None:
        case = self._one_case()
        self.documents[EXPECTED_SHARDS[0]]["cases"] = [case]
        self.documents[EXPECTED_SHARDS[1]]["cases"] = [copy.deepcopy(case)]
        with self.assertRaisesRegex(InventoryError, "duplicate caseId"):
            self._validate()

    def test_mapped_and_absorbed_rows_require_nonempty_unique_case_ids(self) -> None:
        for case_ids in (None, [], [""], ["meta-object-create-basic"] * 2):
            row = self._one_mapped_row()
            variant = self._first_variant(row)
            if case_ids is None:
                variant.pop("caseIds")
            else:
                variant["caseIds"] = case_ids
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
            documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
            with self.subTest(case_ids=case_ids), self.assertRaisesRegex(
                InventoryError, "caseIds"
            ):
                self._validate(documents)

    def test_each_referenced_case_exists_matches_successor_and_has_one_owner(self) -> None:
        documents = copy.deepcopy(self.documents)
        row = self._one_mapped_row()
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [row]
        documents[EXPECTED_SHARDS[0]]["cases"] = []
        with self.assertRaisesRegex(InventoryError, "unknown caseId"):
            self._validate(documents)

        documents[EXPECTED_SHARDS[0]]["cases"] = [
            {**self._one_case(), "entry": "view", "operation": None}
        ]
        documents[EXPECTED_SHARDS[0]]["cases"][0].pop("operation")
        with self.assertRaisesRegex(InventoryError, "successor identity"):
            self._validate(documents)

        documents[EXPECTED_SHARDS[0]]["cases"] = [self._one_case()]
        second = copy.deepcopy(row)
        second["legacyTool"] = "unica.meta.edit"
        documents[EXPECTED_SHARDS[1]]["baselineDispositions"] = [second]
        with self.assertRaisesRegex(InventoryError, "referenced by multiple"):
            self._validate(documents)

    def test_fixture_lexical_path_safety(self) -> None:
        unsafe_paths = (
            ("", "fixture must be a non-empty string"),
            ("/tmp/case.json", "repository-relative"),
            ("C:/case.json", "repository-relative"),
            ("C:\\case.json", "canonical POSIX separators"),
            ("//server/share/case.json", "repository-relative"),
            ("\\\\server\\share\\case.json", "canonical POSIX separators"),
            ("./fixtures/case.json", "normalized non-empty path"),
            ("fixtures/./case.json", "normalized non-empty path"),
            ("fixtures/../case.json", "normalized non-empty path"),
            ("../case.json", "normalized non-empty path"),
            ("fixtures//case.json", "normalized non-empty path"),
            ("fixtures/case.json/", "normalized non-empty path"),
        )
        for fixture, error_pattern in unsafe_paths:
            case = self._one_case()
            case["fixture"] = fixture
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["cases"] = [case]
            with self.subTest(fixture=fixture), self.assertRaisesRegex(
                InventoryError, error_pattern
            ):
                self._validate(documents)

    def test_fixture_must_exist_be_regular_and_be_tracked(self) -> None:
        self._write_fixture("fixtures/untracked.json", tracked=False)
        self.tracked_paths.add("fixtures")
        invalid_paths = (
            ("fixtures/missing.json", "does not exist"),
            ("fixtures", "regular file"),
            ("fixtures/untracked.json", "tracked by git"),
        )
        for fixture, error_pattern in invalid_paths:
            case = self._one_case()
            case["fixture"] = fixture
            documents = copy.deepcopy(self.documents)
            documents[EXPECTED_SHARDS[0]]["cases"] = [case]
            with self.subTest(fixture=fixture), self.assertRaisesRegex(
                InventoryError, error_pattern
            ):
                self._validate(documents)

    def test_fixture_final_symlink_is_rejected(self) -> None:
        target = self._write_fixture("fixtures/target.json")
        link = self.root / "fixtures/link.json"
        try:
            link.symlink_to(target)
        except OSError as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.tracked_paths.add("fixtures/link.json")
        case = self._one_case()
        case["fixture"] = "fixtures/link.json"
        self._set_first_case(case)
        with self.assertRaisesRegex(InventoryError, "symlink"):
            self._validate()

    def test_fixture_symlink_ancestor_inside_repository_is_rejected(self) -> None:
        self._write_fixture("fixtures/internal-target/case.json")
        link = self.root / "linked-inside"
        try:
            link.symlink_to(
                self.root / "fixtures/internal-target", target_is_directory=True
            )
        except OSError as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.tracked_paths.add("linked-inside/case.json")
        case = self._one_case()
        case["fixture"] = "linked-inside/case.json"
        self._set_first_case(case)
        with self.assertRaisesRegex(InventoryError, "symlink"):
            self._validate()

    def test_fixture_resolution_escape_is_rejected_independently(self) -> None:
        outside = self.root.parent / f"{self.root.name}-outside"
        outside.mkdir()
        self.addCleanup(outside.rmdir)
        (outside / "case.json").write_text("{}\n", encoding="utf-8")
        self.addCleanup((outside / "case.json").unlink)
        link = self.root / "linked"
        try:
            link.symlink_to(outside, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"symlinks unavailable: {error}")
        self.tracked_paths.add("linked/case.json")
        case = self._one_case()
        case["fixture"] = "linked/case.json"
        self._set_first_case(case)
        with self.assertRaisesRegex(InventoryError, "escapes"):
            self._validate()

    def test_complete_is_one_uniform_distributed_gate(self) -> None:
        self.documents[EXPECTED_SHARDS[0]]["complete"] = True
        with self.assertRaisesRegex(InventoryError, "mixed complete"):
            self._validate()

    def test_all_true_rejects_incomplete_baseline_accounting(self) -> None:
        for document in self.documents.values():
            document["complete"] = True
        self._set_first_row(self._one_mapped_row())
        self._set_first_case(self._one_case())
        with self.assertRaisesRegex(InventoryError, "missing runtime job dispositions"):
            self._validate()

    def test_all_true_rejects_missing_non_job_baseline_accounting(self) -> None:
        documents = self._complete_documents()
        rows = documents[EXPECTED_SHARDS[0]]["baselineDispositions"]
        documents[EXPECTED_SHARDS[0]]["baselineDispositions"] = [
            row for row in rows if row["legacyTool"] != "unica.xdto.info"
        ]
        with self.assertRaisesRegex(InventoryError, "all 74 baseline names"):
            self._validate(documents)

    def test_all_true_rejects_duplicate_baseline_accounting(self) -> None:
        documents = self._complete_documents()
        rows = documents[EXPECTED_SHARDS[0]]["baselineDispositions"]
        rows[-1]["legacyTool"] = rows[0]["legacyTool"]
        with self.assertRaisesRegex(InventoryError, "duplicate legacyTool"):
            self._validate(documents)

    def test_all_true_rejects_missing_native_entry_case(self) -> None:
        documents = self._complete_documents()
        docs_case_ids = {
            case["caseId"]
            for case in documents[EXPECTED_SHARDS[1]]["cases"]
            if case["entry"] == "docs"
        }
        for case in documents[EXPECTED_SHARDS[1]]["cases"]:
            if case["caseId"] in docs_case_ids:
                case["entry"] = "view"
        for row in documents[EXPECTED_SHARDS[0]]["baselineDispositions"]:
            for variant in row["variants"]:
                if variant.get("caseIds", [None])[0] in docs_case_ids:
                    variant["successor"] = {"entry": "view"}
        with self.assertRaisesRegex(InventoryError, "all eight native entries"):
            self._validate(documents)

    def test_all_true_requires_executable_coverage_of_all_twelve_run_operations(
        self,
    ) -> None:
        documents = self._complete_documents()
        case = next(
            case
            for case in documents[EXPECTED_SHARDS[1]]["cases"]
            if case.get("operation") == "extension.sync"
        )
        case["operation"] = "source.create"
        owning_variant = next(
            variant
            for row in documents[EXPECTED_SHARDS[0]]["baselineDispositions"]
            for variant in row["variants"]
            if case["caseId"] in variant.get("caseIds", [])
        )
        owning_variant["successor"] = {
            "entry": "run",
            "operation": "source.create",
        }
        with self.assertRaisesRegex(InventoryError, "all twelve run operations"):
            self._validate(documents)

    def test_all_true_cross_links_exact_successor_operation_to_a_case(self) -> None:
        documents = self._complete_documents()
        variant = next(
            variant
            for row in documents[EXPECTED_SHARDS[0]]["baselineDispositions"]
            for variant in row["variants"]
            if variant["disposition"] == "absorbed"
        )
        variant["successor"] = {"entry": "run", "operation": "source.create"}
        with self.assertRaisesRegex(
            InventoryError, "successor identity"
        ):
            self._validate(documents)

    def test_all_true_rejects_unowned_executable_cases(self) -> None:
        documents = self._complete_documents()
        extra = self._one_case()
        extra["caseId"] = "unowned-extra"
        documents[EXPECTED_SHARDS[1]]["cases"].append(extra)
        with self.assertRaisesRegex(InventoryError, "unowned executable case"):
            self._validate(documents)

    def test_all_true_still_rejects_invalid_transport_projection(self) -> None:
        documents = self._complete_documents()
        transport_variant = next(
            variant
            for row in documents[EXPECTED_SHARDS[0]]["baselineDispositions"]
            for variant in row["variants"]
            if variant["disposition"] == "transport-replaced"
        )
        transport_variant["projections"] = [
            {"kind": "native-task", "method": "tasks/result"}
        ]
        with self.assertRaisesRegex(InventoryError, "projection"):
            self._validate(documents)

    def test_structurally_complete_synthetic_inventory_passes_shape_validation(self) -> None:
        self._validate(self._complete_documents())

    def test_immutable_release_baseline_has_74_unique_names_and_six_jobs(self) -> None:
        names = load_immutable_baseline_names(
            REPO_ROOT / "tests/fixtures/migration/v0.12.3-baseline.json"
        )
        self.assertEqual(names, IMMUTABLE_BASELINE_NAMES)
        self.assertEqual(len(names), 74)
        self.assertEqual(len(set(names)), 74)
        self.assertEqual(
            {name for name in names if name.startswith("unica.runtime.job.")},
            IMMUTABLE_RUNTIME_JOB_NAMES,
        )


if __name__ == "__main__":
    unittest.main()
