#!/usr/bin/env python3
"""Generate the independently witnessed slice of the #484 address map."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
EMITTER_ENV = "UNICA_484_ADDRESS_JSONL_DIR"
SCHEMA_VERSION = 1
EXTENSION_LANGUAGE_SHARD = "native_extension_root_language_matches_registration"
EXTERNAL_ROOT_SHARD = "native_external_root_descriptors_match_published_addresses"
NATIVE_SHARDS = frozenset({
    "native_configuration_root_address_table_matches_published_branches",
    "native_ordinary_interface_has_a_registration_only_root_branch",
    "native_recalculation_module_address_table_matches_registered_owner",
    "native_metadata_address_table_matches_declared_and_observed_child_branches",
    "native_form_and_template_addresses_follow_text_registrations",
    EXTENSION_LANGUAGE_SHARD,
    EXTERNAL_ROOT_SHARD,
})
DAEMON_SHARD = "medium_published_document_and_bsl_branches_have_canonical_addresses"
NATIVE_SCOPE = (
    "Configuration source sets and native Language registrations in extension source sets "
    "across three platform lines, plus native external processor and report root descriptors "
    "in the unica-coder address_coverage tests. For these external root witnesses, key.line "
    "records the native XML export line. Read checks use the actual workspace profile selector "
    "and establish neither write support nor a match with the runtime profile."
)
DAEMON_SCOPE = (
    "Read witnesses from the selected document and BSL coverage test of unica-daemon, "
    "including observed 8.3.27 examples for extensions, external processors and external reports."
)
SCOPE_BOUNDARY = "This slice is not the complete #484 map or the other #2958 evidence columns."


def exact_fields(value: object, fields: set[str], label: str) -> None:
    if not isinstance(value, dict) or set(value) != fields:
        raise ValueError(f"{label} must have exactly these fields: {', '.join(sorted(fields))}")


def required_text(value: object, label: str) -> str:
    if not isinstance(value, str) or not value.strip() or "\r" in value or "\n" in value:
        raise ValueError(f"{label} must be a nonempty single-line string")
    return value


def token(value: object, label: str) -> None:
    if not re.fullmatch(r"[A-Za-z][A-Za-z0-9]*", required_text(value, label)):
        raise ValueError(f"{label} must be a canonical kind or role token")


def line_number(value: object) -> tuple[int, ...]:
    value = required_text(value, "platform line")
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", value, re.ASCII):
        raise ValueError(f"invalid platform line {value!r}")
    return tuple(map(int, value.split(".")))


def source_prefix(value: object, label: str) -> str:
    prefix, colon, logical = required_text(value, label).partition(":")
    if not colon or not prefix or not logical:
        raise ValueError(f"{label} must be a qualified address")
    return prefix


def validate_record(record: object, test_case: str) -> None:
    exact_fields(record, {"schema_version", "record_type", "key", "witness"}, "record")
    if type(record["schema_version"]) is not int or record["schema_version"] != SCHEMA_VERSION:
        raise ValueError("unsupported address evidence schema_version")
    if record["record_type"] != "evidence":
        raise ValueError("expected an evidence record")
    key = record["key"]
    exact_fields(key, {"line", "source_kind", "owner_shape", "branch", "role"}, "key")
    version = line_number(key["line"])
    source_kind = key["source_kind"]
    allowed_kinds = ("configuration",)
    if test_case == EXTENSION_LANGUAGE_SHARD:
        allowed_kinds = ("extension",)
    elif test_case == EXTERNAL_ROOT_SHARD:
        allowed_kinds = ("external_processor", "external_report")
    elif test_case == DAEMON_SHARD:
        allowed_kinds = ("configuration", "extension", "external_processor", "external_report")
    if source_kind not in allowed_kinds:
        raise ValueError(f"unexpected source-set kind {source_kind!r} in shard {test_case!r}")
    token(key["branch"], "branch kind")
    if key["role"] is not None:
        token(key["role"], "branch role")
    shape = key["owner_shape"]
    if not isinstance(shape, list) or not shape:
        raise ValueError("owner_shape must contain at least one parsed segment")
    for segment in shape:
        exact_fields(segment, {"kind", "named", "role"}, "owner segment")
        token(segment["kind"], "owner kind")
        if type(segment["named"]) is not bool:
            raise ValueError("owner named marker must be a boolean")
        if segment["role"] is not None:
            token(segment["role"], "owner role")

    witness = record["witness"]
    exact_fields(witness, {"owner", "at", "level", "origin", "test_case", "reason"}, "witness")
    owner_prefix = source_prefix(witness["owner"], "owner")
    required_text(witness["origin"], "origin")
    if witness["test_case"] != test_case:
        raise ValueError(f"unexpected test_case {witness['test_case']!r}; expected {test_case!r}")
    reason = witness["reason"]
    if reason is None:
        if witness["level"] not in ("read", "roundtrip"):
            raise ValueError("a verified address needs a read or roundtrip level")
        if source_prefix(witness["at"], "address") != owner_prefix:
            raise ValueError("witness address and owner must belong to the same source set")
        return
    if test_case not in NATIVE_SHARDS or source_kind != "configuration":
        raise ValueError("absence reasons are only supported in native configuration shards")
    if witness["at"] is not None or witness["level"] is not None:
        raise ValueError("an absent address cannot have at or level")
    exact_fields(reason, {"kind", "line"}, "absence reason")
    if reason["kind"] != "introduced_in" or line_number(reason["line"]) <= version:
        raise ValueError("an introduced_in reason must name a later platform line")


def json_object(pairs: list[tuple[str, object]]) -> dict:
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON field {key!r}")
        result[key] = value
    return result


def structural_key(key: dict) -> tuple:
    return (
        line_number(key["line"]),
        key["source_kind"],
        tuple((segment["kind"], segment["named"], segment["role"] or "") for segment in key["owner_shape"]),
        key["branch"],
        key["role"] or "",
    )


def canonical_json(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"))


def merge_shards(directory: Path, required_shards: frozenset[str]) -> list[dict]:
    expected = {f"{test_case}.jsonl" for test_case in required_shards}
    actual = {path.name for path in directory.iterdir()}
    if actual != expected:
        raise ValueError(f"evidence shards missing={sorted(expected - actual)}, extra={sorted(actual - expected)}")

    groups = {}
    seen = set()
    for name in sorted(expected):
        shard = directory / name
        if not shard.is_file() or shard.is_symlink():
            raise ValueError(f"evidence shard is not a regular file: {name}")
        lines = shard.read_text(encoding="utf-8").splitlines()
        if not lines:
            raise ValueError(f"empty evidence shard: {name}")
        for index, line in enumerate(lines, 1):
            try:
                record = json.loads(line, object_pairs_hook=json_object)
                validate_record(record, shard.stem)
                signature = canonical_json(record)
                if signature in seen:
                    raise ValueError("duplicate evidence record")
                seen.add(signature)
                key = structural_key(record["key"])
                witness = record["witness"]
                if key in groups:
                    group = groups[key]
                    if witness["reason"] != group["witnesses"][0]["reason"]:
                        raise ValueError("conflicting address evidence for one structural key")
                    group["witnesses"].append(witness)
                else:
                    groups[key] = {"key": record["key"], "witnesses": [witness]}
            except ValueError as error:
                raise ValueError(f"{name}:{index}: {error}") from error

    result = [groups[key] for key in sorted(groups)]
    for group in result:
        group["witnesses"].sort(key=canonical_json)
    return result


def render_markdown(report: dict) -> str:
    def cell(value: str) -> str:
        return value.replace("|", r"\|")

    rows = [
        "# Address coverage",
        "",
        f"Revision `{report['revision']}` ({'dirty' if report['dirty'] else 'clean'} checkout).",
        report["scope"],
        "",
        "| Line | Source set | Owner shape | Branch | Witnesses |",
        "| --- | --- | --- | --- | --- |",
    ]
    for group in report["rows"]:
        key = group["key"]
        shape = ".".join(
            segment["kind"] + (f".{segment['role']}" if segment["role"] else ".&lt;name&gt;" if segment["named"] else "")
            for segment in key["owner_shape"]
        )
        branch = key["branch"] + (f".{key['role']}" if key["role"] else "")
        witnesses = []
        for witness in group["witnesses"]:
            outcome = (
                f"{witness['at']} ({witness['level']})" if witness["at"] is not None
                else f"introduced in {witness['reason']['line']}"
            )
            witnesses.append(f"{witness['owner']} → {outcome} [{witness['origin']}, {witness['test_case']}]")
        cells = map(cell, (key["line"], key["source_kind"], shape, branch, "<br>".join(witnesses)))
        rows.append("| " + " | ".join(cells) + " |")
    return "\n".join(rows)


def git_state() -> tuple[str, bytes]:
    revision = subprocess.check_output(["git", "rev-parse", "--verify", "HEAD"], cwd=REPO)
    status = subprocess.check_output(["git", "status", "--porcelain=v1", "-z"], cwd=REPO)
    return revision.decode("ascii").strip(), status


def generate_report(native_only: bool) -> dict:
    revision, status = git_state()
    cases = [("unica-coder", "address_coverage")]
    required_shards = NATIVE_SHARDS
    scope = NATIVE_SCOPE
    if not native_only:
        cases.append(("unica-daemon", DAEMON_SHARD))
        required_shards |= {DAEMON_SHARD}
        scope += f" {DAEMON_SCOPE}"
    with tempfile.TemporaryDirectory(prefix="unica-address-coverage-") as scratch:
        env = {**os.environ, EMITTER_ENV: scratch}
        for package, test_filter in cases:
            subprocess.run(
                ["cargo", "test", "--locked", "-p", package, "--lib", test_filter],
                cwd=REPO,
                env=env,
                stdout=sys.stderr,
                check=True,
            )
        rows = merge_shards(Path(scratch), required_shards)
    if git_state() != (revision, status):
        raise ValueError("Git revision or worktree status changed during the address coverage run")
    return {"schema_version": SCHEMA_VERSION, "revision": revision, "dirty": bool(status),
            "scope": f"{scope} {SCOPE_BOUNDARY}", "rows": rows}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--markdown", action="store_true", help="render the report as a Markdown table")
    parser.add_argument("--native-only", action="store_true", help="run only the native unica-coder tests")
    args = parser.parse_args()
    try:
        report = generate_report(args.native_only)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    output = render_markdown(report) if args.markdown else json.dumps(
        report, ensure_ascii=False, sort_keys=True, indent=2
    )
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
