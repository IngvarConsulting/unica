#!/usr/bin/env python3
"""Generate the independently witnessed slice of the #484 address map."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import uuid
import xml.etree.ElementTree as ET
from pathlib import Path


REPO = Path(__file__).resolve().parents[2]
EMITTER_ENV = "UNICA_484_ADDRESS_JSONL_DIR"
SCHEMA_VERSION = 1
EXTENSION_LANGUAGE_SHARD = "native_extension_root_language_matches_registration"
EXTERNAL_ROOT_SHARD = "native_external_root_descriptors_match_published_addresses"
METADATA_SHARD = "native_metadata_address_table_matches_declared_and_observed_child_branches"
FORM_TEMPLATE_SHARD = "native_form_and_template_addresses_follow_text_registrations"
NATIVE_SHARDS = frozenset({
    "native_configuration_root_address_table_matches_published_branches",
    "native_ordinary_interface_has_a_registration_only_root_branch",
    "native_recalculation_module_address_table_matches_registered_owner",
    METADATA_SHARD,
    FORM_TEMPLATE_SHARD,
    "native_registered_modules_match_profile_and_physical_paths",
    EXTENSION_LANGUAGE_SHARD,
    EXTERNAL_ROOT_SHARD,
})
DAEMON_SHARD = "medium_published_document_and_bsl_branches_have_canonical_addresses"
ROOT_KEYS_SHARD = "required_configuration_root_keys"
EXTENSION_LANGUAGE_KEYS_SHARD = "required_native_adopted_extension_language_keys"
METADATA_CHILD_KEYS_SHARD = "required_native_metadata_child_keys"
EXTERNAL_CHILD_KEYS_SHARD = "required_native_external_child_keys"
EXTERNAL_INLINE_KEYS_SHARD = "required_native_external_inline_keys"
EXTERNAL_OWNER_KINDS = {
    "external_processor": ("epf", "ExternalDataProcessor"),
    "external_report": ("erf", "ExternalReport"),
}
MODULE_ROLE_KEYS_SHARD = "required_configuration_module_role_keys"
EDS_MODULE_KEYS_SHARD = "required_native_eds_module_keys"
SUBSYSTEM_INTERFACE_KEYS_SHARD = "required_native_subsystem_interface_keys"
DCS_QUERY_KEYS_SHARD = "required_native_dcs_query_keys"
ROLE_RIGHT_KEYS_SHARD = "required_native_role_right_keys"
XDTO_KEYS_SHARD = "required_native_xdto_keys"
SUBSYSTEM_CONTENT_KEYS_SHARD = "required_native_subsystem_content_keys"
WSDL_KEYS_SHARD = "required_native_wsdl_keys"
MXL_KEYS_SHARD = "required_native_mxl_keys"
SCHEDULE_KEYS_SHARD = "required_native_schedule_keys"
XDTO_OWNER_SHAPES = {
    "Namespace": {("XDTOPackage",)},
    "Type": {("XDTOPackage",)},
    "Property": {("XDTOPackage",), ("XDTOPackage", "Type"), ("XDTOPackage", "Type", "Property")},
}
NATIVE_ADDRESS_NAME = r"[^\W\d]\w*"
REQUIRED_KEY_SHARDS = frozenset({
    EXTENSION_LANGUAGE_KEYS_SHARD,
    ROOT_KEYS_SHARD, METADATA_CHILD_KEYS_SHARD, EXTERNAL_CHILD_KEYS_SHARD, EXTERNAL_INLINE_KEYS_SHARD,
    MODULE_ROLE_KEYS_SHARD,
    EDS_MODULE_KEYS_SHARD, SUBSYSTEM_INTERFACE_KEYS_SHARD, DCS_QUERY_KEYS_SHARD, ROLE_RIGHT_KEYS_SHARD,
    XDTO_KEYS_SHARD, SUBSYSTEM_CONTENT_KEYS_SHARD, WSDL_KEYS_SHARD, MXL_KEYS_SHARD, SCHEDULE_KEYS_SHARD,
})
NATIVE_SCOPE = (
    "Configuration source sets and native adopted Language registrations in extension source sets "
    "across three platform lines, plus native external processor and report root descriptors "
    "in the unica-coder address_coverage tests. Module role and path checks cover roles with "
    "physical BSL witnesses in native configuration and external artifact fixtures. "
    "Read-only direct module "
    "witnesses verify the logical address of registered owners separately from a missing "
    "physical BSL file. For the external root witnesses, key.line "
    "records the native XML export line. Read checks use the actual workspace profile selector "
    "and establish neither write support nor a match with the runtime profile."
    " The native report checks declaration closure for configuration roots and top-level metadata children."
    " It requires external Form, Template, and Command collections witnessed against native XML,"
    " exact reads of external inline attributes, tabular sections, columns and standard attributes,"
    " and ExternalDataSource module collections and round-trips for the native files on each line."
    " It also requires exact read witnesses for registered native Report and DataProcessor DCS queries"
    " and the role-right collection and members in native SalesReader fixtures."
    " Native XDTO requirements trace Namespace, Type and Property reads to registered Package.bin files."
    " Registered subsystem descriptors independently require reads of Relation and Subsystem branches."
    " Registered native WSDL companions require exact WSReference.Definition round-trips."
    " Native spreadsheet companions require registered Area reads and row Body and Parameter reads."
    " Registered ScheduledJob.Schedule keeps descriptor-only reads and requires companion round-trips."
)
DAEMON_SCOPE = (
    "Read witnesses from the selected document and BSL coverage test of unica-daemon, "
    "including observed 8.3.27 examples for extensions, external processors and external reports. "
    "The report checks required configuration module roles from the 8.3.27 address profile "
    "against native or daemon witnesses."
    " The 8.3.27 spreadsheet reads require PlatformExport bytes in the daemon fixture."
)
NATIVE_ONLY_MODULE_SCOPE = (
    "Native-only mode validates the 8.3.27 module-role declarations "
    "and skips their witness check because some roles depend on daemon fixtures."
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


def validate_key(key: object) -> tuple[int, ...]:
    exact_fields(key, {"line", "source_kind", "owner_shape", "branch", "role"}, "key")
    version = line_number(key["line"])
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
    return version


def validate_record_type(record: dict, kind: str) -> None:
    if type(record["schema_version"]) is not int or record["schema_version"] != SCHEMA_VERSION:
        raise ValueError("unsupported address evidence schema_version")
    if record["record_type"] != kind:
        raise ValueError(f"expected record_type {kind!r}")


def validate_record(record: object, test_case: str) -> None:
    exact_fields(record, {"schema_version", "record_type", "key", "witness"}, "record")
    validate_record_type(record, "evidence")
    key = record["key"]
    version = validate_key(key)
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
    if key["branch"] == "Module" and key["role"] is not None:
        raise ValueError("a module-role witness must verify a positive address")
    if witness["at"] is not None or witness["level"] is not None:
        raise ValueError("an absent address cannot have at or level")
    validate_introduction(reason, version, "absence reason")


def validate_introduction(value: object, version: tuple[int, ...], label: str) -> None:
    exact_fields(value, {"kind", "line"}, label)
    if value["kind"] != "introduced_in" or line_number(value["line"]) <= version:
        raise ValueError("an introduced_in reason must name a later platform line")


def validate_required_record(record: object, shard: str) -> None:
    exact_fields(record, {"schema_version", "record_type", "key", "requirement"}, "record")
    validate_record_type(record, "required")
    key = record["key"]
    version = validate_key(key)
    if shard in {EXTERNAL_CHILD_KEYS_SHARD, EXTERNAL_INLINE_KEYS_SHARD}:
        _, owner_kind = EXTERNAL_OWNER_KINDS.get(key["source_kind"], (None, None))
        if owner_kind is None:
            raise ValueError("required external key must name an external source set")
        if shard == EXTERNAL_CHILD_KEYS_SHARD and key["owner_shape"] != [
            {"kind": owner_kind, "named": True, "role": None},
        ]:
            raise ValueError("required external child key must name the matching external owner kind")
    elif shard != EXTENSION_LANGUAGE_KEYS_SHARD and key["source_kind"] != "configuration":
        raise ValueError("required keys must belong to a configuration source set")
    shape = key["owner_shape"]
    requirement = record["requirement"]
    configuration = [{"kind": "Configuration", "named": False, "role": None}]
    if shard == EXTENSION_LANGUAGE_KEYS_SHARD:
        exact_fields(requirement, {"kind", "observations"}, "adopted extension Language requirement")
        if (
            key["source_kind"] != "extension" or shape != configuration
            or key["branch"] != "Language" or key["role"] is not None or requirement["kind"] != "read"
        ):
            raise ValueError("required adopted Language key must name an extension root read")
        observations = requirement["observations"]
        if not isinstance(observations, list) or len(observations) < 2:
            raise ValueError("required adopted Language reads must include the branch and its members")
        addresses = set()
        for index, observation in enumerate(observations):
            exact_fields(observation, {"owner", "origin", "addresses"}, "adopted Language observation")
            required_text(observation["origin"], "origin")
            reads = observation["addresses"]
            pattern = "ext:Language" if index == 0 else rf"ext:Language\.{NATIVE_ADDRESS_NAME}"
            if (
                observation["owner"] != "ext:Configuration" or not isinstance(reads, list) or len(reads) != 1
                or not re.fullmatch(pattern, required_text(reads[0], "address")) or reads[0] in addresses
            ):
                raise ValueError("invalid or duplicate required adopted Language address")
            addresses.add(reads[0])
        return
    if shard == SUBSYSTEM_INTERFACE_KEYS_SHARD:
        exact_fields(requirement, {"kind", "at", "origin"}, "subsystem interface requirement")
        subsystem = [{"kind": "Subsystem", "named": True, "role": None}]
        interface = {"kind": "Interface", "named": False, "role": None}
        branch = key["branch"]
        valid_shape = (
            (branch == "Interface" and shape == subsystem)
            or (branch in {"Command", "Group", "Subsystem"} and shape == [*subsystem, interface])
        )
        if key["role"] is not None or not valid_shape or requirement["kind"] != "read":
            raise ValueError("required subsystem interface key must name an interface read branch")
        if source_prefix(requirement["at"], "address") != "main":
            raise ValueError("required subsystem interface address must name the main source set")
        required_text(requirement["origin"], "origin")
        return
    if shard == DCS_QUERY_KEYS_SHARD:
        exact_fields(requirement, {"kind", "at", "origin"}, "DCS query requirement")
        owner_kinds = tuple(segment["kind"] for segment in shape)
        if (
            key["branch"] != "Query" or key["role"] is not None
            or owner_kinds not in {("DataProcessor", "Template", "DataSet"), ("Report", "Template", "DataSet")}
            or any(not segment["named"] or segment["role"] is not None for segment in shape)
            or requirement["kind"] != "read"
        ):
            raise ValueError("required DCS query key must name a dataset Query read")
        if source_prefix(requirement["at"], "address") != "main":
            raise ValueError("required DCS query address must name the main source set")
        required_text(requirement["origin"], "origin")
        return
    if shard == ROLE_RIGHT_KEYS_SHARD:
        exact_fields(requirement, {"kind", "owner", "origin", "addresses"}, "role right requirement")
        if (
            key["branch"] != "Right" or key["role"] is not None
            or shape != [{"kind": "Role", "named": True, "role": None}]
            or requirement["kind"] != "read"
        ):
            raise ValueError("required role right key must name a Role.Right read")
        owner = required_text(requirement["owner"], "role owner")
        if not re.fullmatch(r"main:Role\.[^\W\d]\w*", owner):
            raise ValueError("required role right owner must name a role in the main source set")
        branch = f"{owner}.Right"
        addresses = requirement["addresses"]
        if not isinstance(addresses, list) or len(addresses) < 2 or addresses[0] != branch:
            raise ValueError("required role right addresses must include the collection and members")
        for address in addresses[1:]:
            if not re.fullmatch(rf"{re.escape(branch)}\.[A-Za-z][A-Za-z0-9]*_[^\W\d]\w*",
                                required_text(address, "right address")):
                raise ValueError("required role right member must belong to the required collection")
        if len(addresses) != len(set(addresses)):
            raise ValueError("duplicate required role right address")
        required_text(requirement["origin"], "origin")
        return
    if shard in {
        XDTO_KEYS_SHARD, SUBSYSTEM_CONTENT_KEYS_SHARD, WSDL_KEYS_SHARD,
        MXL_KEYS_SHARD, SCHEDULE_KEYS_SHARD, EXTERNAL_INLINE_KEYS_SHARD,
    }:
        family = {
            XDTO_KEYS_SHARD: "XDTO", SUBSYSTEM_CONTENT_KEYS_SHARD: "subsystem content", WSDL_KEYS_SHARD: "WSDL",
            MXL_KEYS_SHARD: "MXL", SCHEDULE_KEYS_SHARD: "Schedule",
            EXTERNAL_INLINE_KEYS_SHARD: "external inline",
        }[shard]
        level = "roundtrip" if shard in {WSDL_KEYS_SHARD, SCHEDULE_KEYS_SHARD} else "read"
        exact_fields(requirement, {"kind", "observations"}, f"{family} requirement")
        kinds = tuple(segment["kind"] for segment in shape)
        branch_kind = key["branch"]
        if shard == XDTO_KEYS_SHARD:
            known_shape = kinds in XDTO_OWNER_SHAPES.get(branch_kind, set())
        elif shard == WSDL_KEYS_SHARD:
            known_shape = branch_kind == "Definition" and kinds == ("WSReference",)
        elif shard == SCHEDULE_KEYS_SHARD:
            known_shape = branch_kind == "Schedule" and kinds == ("ScheduledJob",)
        elif shard == EXTERNAL_INLINE_KEYS_SHARD:
            known_shape = (
                (branch_kind in {"Attribute", "TabularSection"} and kinds == (owner_kind,))
                or (branch_kind in {"Attribute", "StandardAttribute"}
                    and kinds == (owner_kind, "TabularSection"))
            )
        elif shard == MXL_KEYS_SHARD:
            known_shape = (
                key["line"] in {"8.3.27", "8.5.1", "8.5.4"} and (
                    (branch_kind == "Area" and kinds == ("Report", "Template"))
                    or (branch_kind in {"Body", "Parameter"} and kinds == ("Report", "Template", "Area"))
                )
            )
        else:
            known_shape = branch_kind in {"Relation", "Subsystem"} and all(kind == "Subsystem" for kind in kinds)
        if (
            not known_shape or key["role"] is not None
            or any(not segment["named"] or segment["role"] is not None for segment in shape)
            or requirement["kind"] != level
        ):
            raise ValueError(f"required {family} key must name a supported {level} branch")
        observations = requirement["observations"]
        if not isinstance(observations, list) or not observations:
            raise ValueError(f"expected nonempty required {family} observations")
        alias = "main"
        if shard == EXTERNAL_INLINE_KEYS_SHARD:
            alias = EXTERNAL_OWNER_KINDS[key["source_kind"]][0]
        elif shard == MXL_KEYS_SHARD and key["line"] == "8.3.27":
            alias = "cf"
        owner_pattern = f"{alias}:" + r"\.".join(kind + r"\." + NATIVE_ADDRESS_NAME for kind in kinds)
        owners = set()
        for observation in observations:
            fields = {"owner", "origin", "addresses"}
            if shard == SCHEDULE_KEYS_SHARD:
                fields.add("descriptor_origin")
            exact_fields(observation, fields, f"{family} observation")
            owner = required_text(observation["owner"], f"{family} owner")
            if not re.fullmatch(owner_pattern, owner) or owner in owners:
                raise ValueError(f"required {family} owner is invalid or duplicated")
            owners.add(owner)
            origin = required_text(observation["origin"], "origin")
            if shard == SCHEDULE_KEYS_SHARD and required_text(
                observation["descriptor_origin"], "descriptor origin",
            ) == origin:
                raise ValueError("required Schedule read and roundtrip must have different origins")
            branch = f"{owner}.{branch_kind}"
            addresses = observation["addresses"]
            if not isinstance(addresses, list) or not addresses or addresses[0] != branch:
                raise ValueError(f"required {family} addresses must include their branch")
            for address in addresses[1:]:
                if (shard == MXL_KEYS_SHARD
                    or branch_kind in {"Namespace", "Relation", "Definition", "Schedule"}
                    or not re.fullmatch(
                    rf"{re.escape(branch)}\.{NATIVE_ADDRESS_NAME}", required_text(address, f"{family} member address")
                )):
                    raise ValueError(f"required {family} member must belong to its branch")
            if len(addresses) != len(set(addresses)):
                raise ValueError(f"duplicate required {family} address")
        return
    if shard == EDS_MODULE_KEYS_SHARD:
        owner_kinds = tuple(segment["kind"] for segment in shape)
        if (
            key["branch"] != "Module"
            or owner_kinds not in {
                ("ExternalDataSource", "Table"),
                ("ExternalDataSource", "Cube"),
                ("ExternalDataSource", "Cube", "DimensionTable"),
            }
            or any(not segment["named"] or segment["role"] is not None for segment in shape)
        ):
            raise ValueError("required EDS module key must name a table, cube or dimension table owner")
        exact_fields(requirement, {"kind"}, "EDS module requirement")
        expected = "read" if key["role"] is None else "roundtrip"
        if requirement["kind"] != expected:
            raise ValueError(f"required EDS module key needs {expected} evidence")
        return
    if shard == MODULE_ROLE_KEYS_SHARD:
        if key["line"] != "8.3.27" or key["branch"] != "Module" or key["role"] is None:
            raise ValueError("required module role key must name an 8.3.27 Module role")
        if shape != configuration and any(
            not segment["named"] or segment["kind"] == "Configuration" or segment["role"] is not None
            for segment in shape
        ):
            raise ValueError("required module role key must have a configuration root or named owner")
        exact_fields(requirement, {"kind"}, "module role requirement")
        if requirement["kind"] != "address":
            raise ValueError("a module role requires a positive address")
        return
    if key["role"] is not None:
        raise ValueError("required metadata keys cannot name a semantic role")
    if shard == ROOT_KEYS_SHARD:
        if shape != configuration:
            raise ValueError("required root key must have the Configuration root owner")
    elif (
        len(shape) != 1
        or not shape[0]["named"]
        or shape[0]["kind"] == "Configuration"
        or shape[0]["role"] is not None
    ):
        raise ValueError("required metadata child key must have a named top-level owner kind")
    if isinstance(requirement, dict) and requirement.get("kind") == "introduced_in":
        if shard != ROOT_KEYS_SHARD:
            raise ValueError("an introduced_in requirement is only valid for a configuration root kind")
        validate_introduction(requirement, version, "requirement")
    else:
        exact_fields(requirement, {"kind"}, "requirement")
        if requirement["kind"] != "address":
            raise ValueError("expected an address or introduced_in requirement")


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


def check_required_keys(requirements: dict, groups: dict, *, native_only: bool) -> None:
    missing = {"address": 0, "introduced_in": 0}
    missing_module_roles = []
    missing_eds_modules = []
    missing_interfaces = []
    missing_queries = []
    metadata_errors = {
        ROLE_RIGHT_KEYS_SHARD: ("Role.Right", []),
        XDTO_KEYS_SHARD: ("XDTO", []),
        SUBSYSTEM_CONTENT_KEYS_SHARD: ("subsystem content", []),
        WSDL_KEYS_SHARD: ("WSDL", []),
        MXL_KEYS_SHARD: ("MXL", []),
        SCHEDULE_KEYS_SHARD: ("Schedule", []),
        EXTERNAL_INLINE_KEYS_SHARD: ("external inline", []),
        EXTENSION_LANGUAGE_KEYS_SHARD: ("adopted extension Language", []),
    }
    diagnostics = []
    for key, (shard, record) in sorted(requirements.items()):
        witnesses = groups.get(key, {}).get("witnesses", ())
        if shard in {SUBSYSTEM_INTERFACE_KEYS_SHARD, DCS_QUERY_KEYS_SHARD}:
            requirement = record["requirement"]
            emitter = DAEMON_SHARD if shard == SUBSYSTEM_INTERFACE_KEYS_SHARD else FORM_TEMPLATE_SHARD
            missing_reads = missing_interfaces if shard == SUBSYSTEM_INTERFACE_KEYS_SHARD else missing_queries
            if not any(
                w["test_case"] == emitter and w["reason"] is None and w["level"] == "read"
                and w["at"] == requirement["at"] and w["origin"] == requirement["origin"]
                for w in witnesses
            ):
                missing_reads.append(
                    f"{requirement['at']} ({record['key']['line']}): expected read from {requirement['origin']}"
                )
            continue
        if shard in metadata_errors:
            requirement = record["requirement"]
            level = requirement["kind"]
            observations = [requirement] if shard == ROLE_RIGHT_KEYS_SHARD else requirement["observations"]
            _, missing_witnesses = metadata_errors[shard]
            emitter = METADATA_SHARD
            if shard == MXL_KEYS_SHARD:
                emitter = DAEMON_SHARD if record["key"]["line"] == "8.3.27" else FORM_TEMPLATE_SHARD
            elif shard == EXTERNAL_INLINE_KEYS_SHARD:
                emitter = EXTERNAL_ROOT_SHARD
            elif shard == EXTENSION_LANGUAGE_KEYS_SHARD:
                emitter = EXTENSION_LANGUAGE_SHARD
            observed = {
                (w["level"], w["owner"], w["origin"], w["at"]) for w in witnesses
                if w["test_case"] == emitter and w["reason"] is None
            }
            inline_roundtrips = (
                {w["at"] for w in witnesses if w["level"] == "roundtrip"}
                if shard == EXTERNAL_INLINE_KEYS_SHARD else set()
            )
            for observation in observations:
                expected = [(level, observation["origin"])]
                if shard == SCHEDULE_KEYS_SHARD:
                    expected.append(("read", observation["descriptor_origin"]))
                for evidence_level, origin in expected:
                    for at in observation["addresses"]:
                        if at in inline_roundtrips:
                            missing_witnesses.append(
                                f"{at} ({record['key']['line']}): inline resource cannot round-trip independently"
                            )
                        if (evidence_level, observation["owner"], origin, at) not in observed:
                            missing_witnesses.append(
                                f"{at} ({record['key']['line']}): expected {evidence_level} from {origin}"
                            )
            continue
        if shard == EDS_MODULE_KEYS_SHARD:
            level = record["requirement"]["kind"]
            levels = {
                witness["level"] for witness in witnesses
                if witness["test_case"] == METADATA_SHARD and witness["reason"] is None
            }
            if level not in levels:
                found = ", ".join(sorted(levels)) if levels else "no native evidence"
                missing_eds_modules.append(
                    f"{canonical_json(record['key'])}: expected {level}, found {found}",
                )
            continue
        if shard == MODULE_ROLE_KEYS_SHARD:
            if not native_only and not any(witness["reason"] is None for witness in witnesses):
                missing_module_roles.append(canonical_json(record["key"]))
            continue
        requirement = record["requirement"]
        kind = requirement["kind"]
        reasons = [witness["reason"] for witness in witnesses if witness["test_case"] in NATIVE_SHARDS]
        expected_reason = None if kind == "address" else requirement
        if expected_reason not in reasons:
            missing[kind] += 1
            found = canonical_json(reasons[0]) if reasons else "no native evidence"
            diagnostics.append(f"{canonical_json(record['key'])}: expected {canonical_json(requirement)}, found {found}")
    errors = []
    if diagnostics:
        errors.append(
            f"missing required native coverage: {missing['address']} address, "
            f"{missing['introduced_in']} introduced_in\n" + "\n".join(diagnostics)
        )
    if missing_eds_modules:
        errors.append("missing required native EDS module coverage:\n" + "\n".join(missing_eds_modules))
    if missing_module_roles:
        errors.append(
            f"missing required configuration module roles: {len(missing_module_roles)} address\n"
            + "\n".join(missing_module_roles)
        )
    if missing_interfaces:
        errors.append(
            "missing required native subsystem interface coverage:\n" + "\n".join(missing_interfaces)
        )
    if missing_queries:
        errors.append("missing required native DCS Query coverage:\n" + "\n".join(missing_queries))
    for family, failures in metadata_errors.values():
        if failures:
            errors.append(f"missing required native {family} coverage:\n" + "\n".join(failures))
    if errors:
        raise ValueError("\n".join(errors))


def merge_shards(
    directory: Path, required_shards: frozenset[str], *, native_only: bool = False,
) -> list[dict]:
    expected = {f"{test_case}.jsonl" for test_case in required_shards}
    actual = {path.name for path in directory.iterdir()}
    if actual != expected:
        raise ValueError(f"evidence shards missing={sorted(expected - actual)}, extra={sorted(actual - expected)}")

    groups = {}
    requirements = {}
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
                if shard.stem in REQUIRED_KEY_SHARDS:
                    validate_required_record(record, shard.stem)
                    key = structural_key(record["key"])
                    if key in requirements:
                        prior = requirements[key][1]["requirement"]
                        kind = "duplicate" if prior == record["requirement"] else "conflicting"
                        raise ValueError(f"{kind} requirement for structural key {canonical_json(record['key'])}")
                    requirements[key] = (shard.stem, record)
                    continue
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

    check_required_keys(requirements, groups, native_only=native_only)
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


def native_subsystem_interface_requirements(repo: Path) -> list[dict]:
    """Discover the branches ordered by INV.SOURCE.COMMAND-INTERFACE-ORDERS from native XML."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    ext = "{http://v8.1c.ru/8.3/xcf/extrnprops}"
    command_reference = re.compile(
        r"CommonCommand\.[^\W\d]\w*|[A-Z][A-Za-z]*\.[^\W\d]\w*\.(?:StandardCommand|Command)\.[^\W\d]\w*"
    )
    fixtures = (
        ("2.20", "8.3.27", "workspace-command-groups"),
        ("2.21", "8.5.1", "workspace-native-subsystem-interface-2.21"),
        ("2.22", "8.5.4", "workspace-native-subsystem-interface-2.22"),
    )
    # These fixtures prove one named representative per structural key and format.
    # Additional subsystems in the same fixture may share that structural key.
    name = "Подсистема1"
    rows = []
    for format_version, line, fixture in fixtures:
        source = repo / "tests/fixtures/acceptance" / fixture / "src"
        configuration = ET.parse(source / "Configuration.xml").getroot()
        if configuration.get("version") != format_version:
            raise ValueError(f"{fixture}: wrong configuration format")
        registrations = configuration.findall(f"{md}Configuration/{md}ChildObjects/{md}Subsystem")
        if sum(node.text == name for node in registrations) != 1:
            raise ValueError(f"{fixture}: missing or duplicate registration for {name}")
        companion = source / "Subsystems" / name / "Ext/CommandInterface.xml"
        descriptor = ET.parse(source / "Subsystems" / f"{name}.xml").getroot()
        interface = ET.parse(companion).getroot()
        if (
            descriptor.get("version") != format_version
            or descriptor.findtext(f"{md}Subsystem/{md}Properties/{md}Name") != name
            or interface.tag != f"{ext}CommandInterface"
            or interface.get("version") != format_version
        ):
            raise ValueError(f"{fixture}: mismatched subsystem or command interface")
        commands = (
            command.get("name", "")
            for section in ("CommandsVisibility", "CommandsPlacement")
            for command in interface.findall(f"{ext}{section}/{ext}Command")
        )
        at = f"main:Subsystem.{name}.Interface"
        origin = companion.relative_to(repo).as_posix()
        owner = [{"kind": "Subsystem", "named": True, "role": None}]
        nested = [*owner, {"kind": "Interface", "named": False, "role": None}]
        branches = [("Interface", owner, at)]
        if any(command_reference.fullmatch(command) for command in commands):
            branches.append(("Command", nested, f"{at}.Command"))
        if interface.findall(f"{ext}GroupsOrder/{ext}Group"):
            branches.append(("Group", nested, f"{at}.Group"))
        ordered_subsystems = interface.findall(f"{ext}SubsystemsOrder/{ext}Subsystem")
        if ordered_subsystems:
            prefix = f"Subsystem.{name}.Subsystem."
            registered = descriptor.findall(f"{md}Subsystem/{md}ChildObjects/{md}Subsystem")
            for reference in ordered_subsystems:
                child = (reference.text or "").removeprefix(prefix)
                if (
                    reference.text != prefix + child
                    or re.fullmatch(r"[^\W\d]\w*", child) is None
                    or sum(node.text == child for node in registered) != 1
                ):
                    raise ValueError(f"{fixture}: SubsystemsOrder requires a registered direct child")
                child_path = source / "Subsystems" / name / "Subsystems" / f"{child}.xml"
                child_descriptor = ET.parse(child_path).getroot()
                if (
                    child_descriptor.tag != f"{md}MetaDataObject"
                    or child_descriptor.get("version") != format_version
                    or child_descriptor.findtext(f"{md}Subsystem/{md}Properties/{md}Name") != child
                ):
                    raise ValueError(f"{fixture}: mismatched registered child {child}")
            branches.append(("Subsystem", nested, f"{at}.Subsystem"))
        for branch, shape, address in branches:
            rows.append({
                "schema_version": SCHEMA_VERSION, "record_type": "required",
                "key": {"line": line, "source_kind": "configuration", "owner_shape": shape,
                        "branch": branch, "role": None},
                "requirement": {"kind": "read", "at": address, "origin": origin},
            })
    return rows


def native_dcs_query_requirements(repo: Path) -> list[dict]:
    """Pick a native registered Query witness per covered owner shape and export line."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    dcs = "{http://v8.1c.ru/8.1/data-composition-system/schema}"
    xsi_type = "{http://www.w3.org/2001/XMLSchema-instance}type"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = repo / "tests/fixtures/formats/platform_xml" / version / "all_kinds/src"
        configuration = ET.parse(source / "Configuration.xml").getroot()
        if configuration.tag != f"{md}MetaDataObject" or configuration.get("version") != version:
            raise ValueError(f"native DCS Query: wrong configuration format in {source}")
        for kind, directory in (("DataProcessor", "DataProcessors"), ("Report", "Reports")):
            queries = []
            for registration in configuration.findall(f"{md}Configuration/{md}ChildObjects/{md}{kind}"):
                owner = required_text(registration.text, "registered DCS owner")
                descriptor = source / directory / f"{owner}.xml"
                artifact = ET.parse(descriptor).getroot().find(f"{md}{kind}")
                if artifact is None or artifact.findtext(f"{md}Properties/{md}Name") != owner:
                    raise ValueError(f"native DCS Query: wrong registered owner {descriptor}")
                for template in artifact.findall(f"{md}ChildObjects/{md}Template"):
                    name = required_text(template.text, "registered DCS template")
                    root = descriptor.with_suffix("") / "Templates"
                    template_descriptor = ET.parse(root / f"{name}.xml").getroot()
                    properties = template_descriptor.find(f"{md}Template/{md}Properties")
                    if properties is None or properties.findtext(f"{md}Name") != name:
                        raise ValueError(f"native DCS Query: wrong template registration {name}")
                    if properties.findtext(f"{md}TemplateType") != "DataCompositionSchema":
                        continue
                    origin = root / name / "Ext/Template.xml"
                    schema = ET.parse(origin).getroot()
                    if schema.tag != f"{dcs}DataCompositionSchema":
                        raise ValueError(f"native DCS Query: wrong XML root {origin}")
                    for dataset in schema.findall(f"{dcs}dataSet"):
                        statements = dataset.findall(f"{dcs}query")
                        if not statements:
                            continue
                        if (
                            dataset.get(xsi_type) != "DataSetQuery" or len(statements) != 1
                            or not (statements[0].text or "").strip()
                        ):
                            raise ValueError(f"native DCS Query: invalid query {origin}")
                        dataset_name = required_text(dataset.findtext(f"{dcs}name"), "DCS dataset name")
                        at = f"main:{kind}.{owner}.Template.{name}.DataSet.{dataset_name}.Query"
                        queries.append((at, origin.relative_to(repo).as_posix()))
            if queries:
                at, origin = min(queries)
                shape = [{"kind": part, "named": True, "role": None} for part in (kind, "Template", "DataSet")]
                rows.append({
                    "schema_version": SCHEMA_VERSION, "record_type": "required",
                    "key": {"line": line, "source_kind": "configuration", "owner_shape": shape,
                            "branch": "Query", "role": None},
                    "requirement": {"kind": "read", "at": at, "origin": origin},
                })
    return rows


def native_role_right_requirements(repo: Path) -> list[dict]:
    """Derive the required Role.Right reads from the registered native SalesReader role."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    ns = "{http://v8.1c.ru/8.2/roles}"
    name = "SalesReader"
    owner = f"main:Role.{name}"
    branch = f"{owner}.Right"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = repo / "tests/fixtures/formats/platform_xml" / version / "all_kinds/src"
        configuration = ET.parse(source / "Configuration.xml").getroot()
        descriptor_path = source / "Roles" / f"{name}.xml"
        descriptor = ET.parse(descriptor_path).getroot()
        origin = descriptor_path.with_suffix("") / "Ext/Rights.xml"
        rights = ET.parse(origin).getroot()
        if any(
            node.tag != tag or node.get("version") != version
            for node, tag in ((configuration, md + "MetaDataObject"),
                              (descriptor, md + "MetaDataObject"), (rights, ns + "Rights"))
        ):
            raise ValueError(f"native Role.Right: wrong XML root or format in {source}")
        registrations = configuration.find(f"{md}Configuration/{md}ChildObjects")
        if registrations is None or sum(node.text == name for node in registrations.findall(f"{md}Role")) != 1:
            raise ValueError(f"native Role.Right: expected one registration for {name} in {source}")
        roles = descriptor.findall(f"{md}Role")
        if len(roles) != 1 or roles[0].findtext(f"{md}Properties/{md}Name") != name:
            raise ValueError(f"native Role.Right: wrong role descriptor {descriptor_path}")
        try:
            identity = uuid.UUID(roles[0].get("uuid", ""))
        except ValueError as error:
            raise ValueError(f"native Role.Right: invalid role UUID in {descriptor_path}") from error
        if not identity.int:
            raise ValueError(f"native Role.Right: nil role UUID in {descriptor_path}")

        members = set()
        for entry in rights.findall(f"{ns}object"):
            target = required_text(entry.findtext(f"{ns}name"), "native role right target")
            match = re.fullmatch(r"([A-Za-z][A-Za-z0-9]*)\.([^\W\d]\w*)", target)
            if match is None:
                raise ValueError(f"native Role.Right: invalid target {target} in {origin}")
            kind, object_name = match.groups()
            if kind == "Configuration":
                registered = configuration.findtext(f"{md}Configuration/{md}Properties/{md}Name") == object_name
            else:
                registered = sum(node.text == object_name for node in registrations.findall(f"{md}{kind}")) == 1
            if not registered:
                raise ValueError(f"native Role.Right: unregistered target {target} in {origin}")
            address = f"{branch}.{kind}_{object_name}"
            if address in members or not entry.findall(f"{ns}right"):
                raise ValueError(f"native Role.Right: duplicate or empty target {target} in {origin}")
            members.add(address)
        if not members:
            raise ValueError(f"native Role.Right: no registered right targets in {origin}")
        rows.append({
            "schema_version": SCHEMA_VERSION, "record_type": "required",
            "key": {"line": line, "source_kind": "configuration",
                    "owner_shape": [{"kind": "Role", "named": True, "role": None}],
                    "branch": "Right", "role": None},
            "requirement": {"kind": "read", "owner": owner, "addresses": [branch, *sorted(members)],
                            "origin": origin.relative_to(repo).as_posix()},
        })
    return rows


def native_xdto_requirements(repo: Path) -> list[dict]:
    """Derive XDTO branch and named-member requirements from native registrations and package XML."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    xdto = "{http://v8.1c.ru/8.1/xdto}"
    xsi_type = "{http://www.w3.org/2001/XMLSchema-instance}type"
    rows = []

    def name(value: str | None) -> str:
        text = required_text(value, "native XDTO name")
        if not re.fullmatch(NATIVE_ADDRESS_NAME, text):
            raise ValueError(f"invalid native XDTO name {text}")
        return text

    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = repo / "tests/fixtures/formats/platform_xml" / version / "all_kinds/src"
        config = ET.parse(source / "Configuration.xml").getroot()
        if config.tag != f"{md}MetaDataObject" or config.get("version") != version:
            raise ValueError(f"native XDTO: wrong configuration format in {source}")
        packages = [
            name(node.text) for node in config.findall(f"{md}Configuration/{md}ChildObjects/{md}XDTOPackage")
        ]
        if not packages or len(packages) != len(set(packages)):
            raise ValueError(f"native XDTO: missing or duplicate package registrations in {source}")
        branches = {}
        for package_name in sorted(packages):
            descriptor_path = source / "XDTOPackages" / f"{package_name}.xml"
            descriptor = ET.parse(descriptor_path).getroot()
            artifacts = list(descriptor)
            if (
                descriptor.tag != f"{md}MetaDataObject" or descriptor.get("version") != version
                or len(artifacts) != 1 or artifacts[0].tag != f"{md}XDTOPackage"
                or artifacts[0].findtext(f"{md}Properties/{md}Name") != package_name
            ):
                raise ValueError(f"native XDTO: wrong package descriptor {descriptor_path}")
            try:
                identity = uuid.UUID(artifacts[0].get("uuid", ""))
            except ValueError as error:
                raise ValueError(f"native XDTO: invalid UUID in {descriptor_path}") from error
            namespace = required_text(artifacts[0].findtext(f"{md}Properties/{md}Namespace"), "XDTO namespace")
            payload = descriptor_path.with_suffix("") / "Ext/Package.bin"
            package = ET.parse(payload).getroot()
            if not identity.int or package.tag != f"{xdto}package" or package.get("targetNamespace") != namespace:
                raise ValueError(f"native XDTO: identity or namespace mismatch in {payload}")
            origin = payload.relative_to(repo).as_posix()

            def observe(kinds, owner, branch, members):
                at = f"{owner}.{branch}"
                branches.setdefault((kinds, branch), []).append({
                    "owner": owner, "origin": origin, "addresses": [at, *sorted(members)],
                })

            def inspect_properties(node, owner, kinds):
                properties = node.findall(xdto + "property")
                if not properties:
                    return
                members = {}
                for prop in properties:
                    if "name" not in prop.attrib:
                        # Ref-only rows participate in the collection without their own address.
                        if not prop.get("ref"):
                            raise ValueError(f"native XDTO: unnamed property without a reference in {payload}")
                        continue
                    at = f"{owner}.Property.{name(prop.get('name'))}"
                    if at in members:
                        raise ValueError(f"native XDTO: duplicate property {at}")
                    members[at] = prop
                observe(kinds, owner, "Property", members)
                for at, prop in sorted(members.items()):
                    definitions = prop.findall(xdto + "typeDef")
                    if len(definitions) > 1 or (definitions and not definitions[0].get(xsi_type)):
                        raise ValueError(f"native XDTO: ambiguous anonymous type at {at}")
                    if definitions:
                        inspect_properties(definitions[0], at, (*kinds, "Property"))

            owner = f"main:XDTOPackage.{package_name}"
            kinds = ("XDTOPackage",)
            observe(kinds, owner, "Namespace", ())
            types = {}
            for node in package:
                if node.tag in {xdto + "objectType", xdto + "valueType"}:
                    at = f"{owner}.Type.{name(node.get('name'))}"
                    if at in types:
                        raise ValueError(f"native XDTO: duplicate type {at}")
                    types[at] = node
            if types:
                observe(kinds, owner, "Type", types)
            inspect_properties(package, owner, kinds)
            for at, node in sorted(types.items()):
                inspect_properties(node, at, (*kinds, "Type"))
        for (kinds, branch), observations in sorted(branches.items()):
            rows.append({
                "schema_version": SCHEMA_VERSION, "record_type": "required",
                "key": {"line": line, "source_kind": "configuration",
                        "owner_shape": [{"kind": kind, "named": True, "role": None} for kind in kinds],
                        "branch": branch, "role": None},
                "requirement": {"kind": "read", "observations": observations},
            })
    return rows


def native_subsystem_content_requirements(repo: Path) -> list[dict]:
    """Require native Subsystem and Relation reads without implying resource resolution."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    readable = "{http://v8.1c.ru/8.3/xcf/readable}"
    root_name = "СтандартныеПодсистемы"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = repo / "tests/fixtures/formats/platform_xml" / version / "all_kinds/src"

        def artifact(path, kind):
            xml = ET.parse(path).getroot()
            nodes = list(xml)
            if (
                xml.tag != md + "MetaDataObject" or xml.get("version") != version
                or len(nodes) != 1 or nodes[0].tag != md + kind
            ):
                raise ValueError(f"native subsystem content: wrong descriptor {path}")
            return nodes[0]

        def descriptor(path, kind, name):
            node = artifact(path, kind)
            if node.findtext(f"{md}Properties/{md}Name") != name or not uuid.UUID(node.get("uuid", "")).int:
                raise ValueError(f"native subsystem content: wrong descriptor identity in {path}")
            return node

        config = artifact(source / "Configuration.xml", "Configuration")
        registrations = config.findall(md + "ChildObjects")
        if len(registrations) != 1 or sum(
            item.text == root_name for item in registrations[0].findall(md + "Subsystem")
        ) != 1:
            raise ValueError(f"native subsystem content: missing unique registration for {root_name}")
        registrations = registrations[0]
        branches = {}

        def visit(path, name, owner, depth):
            node = descriptor(path, "Subsystem", name)
            child_objects = node.findall(md + "ChildObjects")
            if len(child_objects) != 1:
                raise ValueError(f"native subsystem content: missing unique ChildObjects in {path}")
            children = []
            for child in child_objects[0]:
                if child.tag != md + "Subsystem" or not re.fullmatch(NATIVE_ADDRESS_NAME, child.text or ""):
                    raise ValueError(f"native subsystem content: invalid child registration in {path}")
                children.append(child.text)
            if len(children) != len(set(children)):
                raise ValueError(f"native subsystem content: duplicate child registration in {path}")
            origin = path.relative_to(repo).as_posix()
            collection = f"{owner}.Subsystem"
            branches.setdefault((depth, "Subsystem"), []).append({
                "owner": owner, "origin": origin, "addresses": [collection, *(f"{collection}.{c}" for c in children)],
            })
            contents = node.findall(f"{md}Properties/{md}Content")
            if len(contents) > 1:
                raise ValueError(f"native subsystem content: duplicate Content in {path}")
            if contents and len(contents[0]):
                for item in contents[0]:
                    match = re.fullmatch(rf"CommonModule\.({NATIVE_ADDRESS_NAME})", item.text or "")
                    if item.tag != readable + "Item" or not match or sum(
                        module.text == match[1] for module in registrations.findall(md + "CommonModule")
                    ) != 1:
                        raise ValueError(f"native subsystem content: unexpected Content target in {path}")
                    descriptor(source / "CommonModules" / f"{match[1]}.xml", "CommonModule", match[1])
                branches.setdefault((depth, "Relation"), []).append({
                    "owner": owner, "origin": origin, "addresses": [f"{owner}.Relation"],
                })
            for child in children:
                visit(path.with_suffix("") / "Subsystems" / f"{child}.xml", child, f"{collection}.{child}", depth + 1)

        visit(source / "Subsystems" / f"{root_name}.xml", root_name, f"main:Subsystem.{root_name}", 1)
        for (depth, branch), observations in sorted(branches.items()):
            rows.append({
                "schema_version": SCHEMA_VERSION, "record_type": "required",
                "key": {"line": line, "source_kind": "configuration", "branch": branch, "role": None,
                        "owner_shape": [{"kind": "Subsystem", "named": True, "role": None} for _ in range(depth)]},
                "requirement": {"kind": "read", "observations": observations},
            })
    return rows


def native_wsdl_requirements(repo: Path) -> list[dict]:
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    wsdl = "{http://schemas.xmlsoap.org/wsdl/}"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = repo / "tests/fixtures/formats/platform_xml" / version / "all_kinds/src"
        configuration = ET.parse(source / "Configuration.xml").getroot()
        if configuration.tag != f"{md}MetaDataObject" or configuration.get("version") != version:
            raise ValueError(f"native WSDL: wrong configuration format in {source}")
        registrations = configuration.findall(f"{md}Configuration/{md}ChildObjects/{md}WSReference")
        if not registrations:
            raise ValueError(f"native WSDL: no registered WSReference in {source}")
        observations = []
        for registration in registrations:
            name = required_text(registration.text, "registered WSReference name")
            if not re.fullmatch(NATIVE_ADDRESS_NAME, name):
                raise ValueError(f"native WSDL: invalid WSReference name {name}")
            path = source / "WSReferences" / f"{name}.xml"
            descriptor = ET.parse(path).getroot()
            if (
                descriptor.tag != f"{md}MetaDataObject" or descriptor.get("version") != version
                or descriptor.findtext(f"{md}WSReference/{md}Properties/{md}Name") != name
            ):
                raise ValueError(f"native WSDL: wrong registered descriptor {path}")
            origin = path.with_suffix("") / "Ext/WSDefinition.xml"
            definitions = ET.parse(origin).getroot()
            if definitions.tag != f"{wsdl}definitions" or not definitions.get("targetNamespace", "").strip():
                raise ValueError(f"native WSDL: invalid definitions in {origin}")
            owner = f"main:WSReference.{name}"
            observations.append({"owner": owner, "origin": origin.relative_to(repo).as_posix(),
                                 "addresses": [f"{owner}.Definition"]})
        rows.append({
            "schema_version": SCHEMA_VERSION, "record_type": "required",
            "key": {"line": line, "source_kind": "configuration",
                    "owner_shape": [{"kind": "WSReference", "named": True, "role": None}],
                    "branch": "Definition", "role": None},
            "requirement": {"kind": "roundtrip", "observations": observations},
        })
    return rows


def native_mxl_requirements(repo: Path, *, native_only: bool) -> list[dict]:
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    mxl = "{http://v8.1c.ru/8.2/data/spreadsheet}"
    xsi_type = "{http://www.w3.org/2001/XMLSchema-instance}type"
    export_root = repo / "tests/fixtures/formats/platform_xml"
    sources = []
    for version, line in (("2.21", "8.5.1"), ("2.22", "8.5.4")):
        fixture = export_root / version / "mxl-areas"
        manifest = json.loads((fixture / "manifest.json").read_text(encoding="utf-8"))
        if (
            manifest["format"] != version or not manifest["build"].startswith(f"{line}.")
            or manifest["seed"] != "../all_kinds/src"
        ):
            raise ValueError(f"native MXL: wrong platform or registration source in {fixture}")
        resource = required_text(manifest["exportedResource"], "MXL exported resource")
        origin = fixture / resource
        if not origin.resolve().is_relative_to(fixture.resolve()):
            raise ValueError(f"native MXL: export leaves its fixture: {origin}")
        raw = origin.read_bytes()
        if hashlib.sha256(raw).hexdigest() != manifest["exportedResourceSha256"]:
            raise ValueError(f"native MXL: export does not match manifest: {origin}")
        sources.append((version, line, "main", fixture.parent / "all_kinds/src", resource, origin, raw))

    if not native_only:
        # The daemon workspace uses synthetic metadata. Only PlatformExport's
        # spreadsheet bytes were exported by 8.3.27.2074, as documented by the
        # mxl_edit fixture. The synthetic F06Template can share its key.
        source = repo / "tests/fixtures/acceptance/workspace-mxl/src/cf"
        resource = "Reports/F06Report/Templates/PlatformExport/Ext/Template.xml"
        origin = source / resource
        raw = origin.read_bytes()
        donor = repo / "crates/unica-templates-designer/tests/fixtures/mxl_edit/Template.xml"
        if raw != donor.read_bytes():
            raise ValueError(f"native MXL: PlatformExport differs from the 8.3.27 export: {origin}")
        sources.append(("2.20", "8.3.27", "cf", source, resource, origin, raw))

    rows = []
    for version, line, prefix, source, resource, origin, raw in sources:
        names = re.fullmatch(
            rf"Reports/({NATIVE_ADDRESS_NAME})/Templates/({NATIVE_ADDRESS_NAME})/Ext/Template\.xml",
            resource,
        )
        if names is None:
            raise ValueError(f"native MXL: expected a report template companion: {origin}")
        report, template = names.groups()

        def descriptor(relative):
            root = ET.parse(source / relative).getroot()
            if root.tag != f"{md}MetaDataObject" or root.get("version") != version:
                raise ValueError(f"native MXL: wrong descriptor format: {source / relative}")
            return root

        configuration = descriptor("Configuration.xml").find(f"{md}Configuration")
        report_node = descriptor(f"Reports/{report}.xml").find(f"{md}Report")
        template_node = descriptor(f"Reports/{report}/Templates/{template}.xml").find(f"{md}Template")
        if (
            configuration is None or report_node is None or template_node is None
            or [node.text for node in configuration.findall(f"{md}ChildObjects/{md}Report")].count(report) != 1
            or report_node.findtext(f"{md}Properties/{md}Name") != report
            or [node.text for node in report_node.findall(f"{md}ChildObjects/{md}Template")].count(template) != 1
            or template_node.findtext(f"{md}Properties/{md}Name") != template
            or template_node.findtext(f"{md}Properties/{md}TemplateType") != "SpreadsheetDocument"
        ):
            raise ValueError(f"native MXL: missing registered spreadsheet owner for {origin}")

        document = ET.fromstring(raw)
        if document.tag != f"{mxl}document":
            raise ValueError(f"native MXL: expected spreadsheet document: {origin}")
        area_names = set()
        row_names = []
        for item in document.findall(f"{mxl}namedItem"):
            if item.get(xsi_type) != "NamedItemCells":
                continue
            name = required_text(item.findtext(f"{mxl}name"), "MXL area name")
            area = item.find(f"{mxl}area")
            if not re.fullmatch(NATIVE_ADDRESS_NAME, name) or name in area_names or area is None:
                raise ValueError(f"native MXL: invalid or duplicate area {name} in {origin}")
            area_names.add(name)
            if area.findtext(f"{mxl}type") == "Rows":
                row_names.append(name)
        if not row_names:
            raise ValueError(f"native MXL: no named row areas in {origin}")

        template_owner = f"{prefix}:Report.{report}.Template.{template}"
        row_owners = [f"{template_owner}.Area.{name}" for name in sorted(row_names)]
        for branch, kinds, owners in (
            ("Area", ("Report", "Template"), [template_owner]),
            ("Body", ("Report", "Template", "Area"), row_owners),
            ("Parameter", ("Report", "Template", "Area"), row_owners),
        ):
            rows.append({
                "schema_version": SCHEMA_VERSION, "record_type": "required",
                "key": {"line": line, "source_kind": "configuration", "branch": branch, "role": None,
                        "owner_shape": [{"kind": kind, "named": True, "role": None} for kind in kinds]},
                "requirement": {"kind": "read", "observations": [
                    {"owner": owner, "origin": origin.relative_to(repo).as_posix(),
                     "addresses": [f"{owner}.{branch}"]} for owner in owners
                ]},
            })
    return rows


def native_schedule_requirements(repo: Path) -> list[dict]:
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    external = "{http://v8.1c.ru/8.3/xcf/extrnprops}"
    fixture_root = repo / "tests/fixtures"
    name = "KeyProbe"
    owner = f"main:ScheduledJob.{name}"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = fixture_root / "formats/platform_xml" / version / "all_kinds/src"
        configuration = ET.parse(source / "Configuration.xml").getroot()
        registrations = configuration.findall(f"{md}Configuration/{md}ChildObjects/{md}ScheduledJob")
        if (
            configuration.tag != f"{md}MetaDataObject" or configuration.get("version") != version
            or [node.text for node in registrations].count(name) != 1
        ):
            raise ValueError(f"native Schedule: missing registered {name} in {source}")
        descriptor = source / "ScheduledJobs" / f"{name}.xml"
        root = ET.parse(descriptor).getroot()
        properties = root.find(f"{md}ScheduledJob/{md}Properties")
        if (
            root.tag != f"{md}MetaDataObject" or root.get("version") != version
            or properties is None or properties.findtext(f"{md}Name") != name
            or properties.find(f"{md}Schedule") is not None
            or (descriptor.with_suffix("") / "Ext/Schedule.xml").exists()
        ):
            raise ValueError(f"native Schedule: expected registered descriptor without a schedule: {descriptor}")
        # The separate platform export adds the companion to this unchanged seed.
        origin = fixture_root / "acceptance/native-scheduled-job-schedule" / version / "Schedule.xml"
        root = ET.parse(origin).getroot()
        schedule = root.findall(f"{external}Schedule")
        if (
            root.tag != f"{external}JobSchedule" or root.get("version") != version
            or len(schedule) != 1 or not schedule[0].get("BeginTime")
        ):
            raise ValueError(f"native Schedule: invalid companion for {version}: {origin}")
        rows.append({
            "schema_version": SCHEMA_VERSION, "record_type": "required",
            "key": {"line": line, "source_kind": "configuration", "branch": "Schedule", "role": None,
                    "owner_shape": [{"kind": "ScheduledJob", "named": True, "role": None}]},
            "requirement": {"kind": "roundtrip", "observations": [{
                "owner": owner, "origin": origin.relative_to(repo).as_posix(),
                "descriptor_origin": descriptor.relative_to(repo).as_posix(),
                "addresses": [f"{owner}.Schedule"],
            }]},
        })
    return rows


def native_adopted_extension_language_requirements(repo: Path) -> list[dict]:
    """Require existing reads for the adopted languages in the native empty-extension corpus."""
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    owner = "ext:Configuration"
    branch = "ext:Language"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        source = Path("tests/fixtures/formats/platform_xml") / version / "empty_extension/ext"
        origin = source / "Configuration.xml"
        root = ET.parse(repo / origin).getroot()
        configuration = root.find(f"{md}Configuration")
        if (
            root.tag != f"{md}MetaDataObject" or root.get("version") != version
            or configuration is None
            or configuration.findtext(f"{md}Properties/{md}Name") != "CorpusExtension"
            or not configuration.findtext(f"{md}Properties/{md}ConfigurationExtensionPurpose")
        ):
            raise ValueError(f"{origin}: expected the native XML {version} extension")
        registrations = configuration.findall(f"{md}ChildObjects/{md}Language")
        names = [required_text(node.text, "Language registration name") for node in registrations]
        if not names or len(names) != len(set(names)) or any(
            len(node) or not re.fullmatch(NATIVE_ADDRESS_NAME, name)
            for node, name in zip(registrations, names, strict=True)
        ):
            raise ValueError(f"{origin}: invalid native Language registrations")
        observations = [{"owner": owner, "origin": origin.as_posix(), "addresses": [branch]}]
        for name in names:
            descriptor = source / "Languages" / f"{name}.xml"
            root = ET.parse(repo / descriptor).getroot()
            properties = root.find(f"{md}Language/{md}Properties")
            if (
                root.tag != f"{md}MetaDataObject" or root.get("version") != version
                or properties is None or properties.findtext(f"{md}Name") != name
                or properties.findtext(f"{md}ObjectBelonging") != "Adopted"
            ):
                raise ValueError(f"{descriptor}: expected registered adopted Language.{name}")
            linked = properties.findtext(f"{md}ExtendedConfigurationObject")
            if not linked or not uuid.UUID(linked).int:
                raise ValueError(f"{descriptor}: missing extended configuration object UUID")
            observations.append({
                "owner": owner, "origin": descriptor.as_posix(), "addresses": [f"{branch}.{name}"],
            })
        rows.append({
            "schema_version": SCHEMA_VERSION, "record_type": "required",
            "key": {"line": line, "source_kind": "extension", "branch": "Language", "role": None,
                    "owner_shape": [{"kind": "Configuration", "named": False, "role": None}]},
            "requirement": {"kind": "read", "observations": observations},
        })
    return rows


def native_external_inline_requirements(repo: Path) -> list[dict]:
    md = "{http://v8.1c.ru/8.3/MDClasses}"
    xr = "{http://v8.1c.ru/8.3/xcf/readable}"
    export_root = repo / "tests/fixtures/formats/platform_xml"
    rows = []
    for version, line in (("2.20", "8.3.27"), ("2.21", "8.5.1"), ("2.22", "8.5.4")):
        fixture = export_root / version / "external_artifacts"
        manifest = json.loads((fixture / "manifest.json").read_text(encoding="utf-8"))
        if manifest["formatVersion"] != version or not manifest["platformVersion"].startswith(f"{line}."):
            raise ValueError(f"native external inline: wrong export provenance in {fixture}")
        for source_kind, (alias, kind) in EXTERNAL_OWNER_KINDS.items():
            descriptors = [Path(path) for path in manifest["files"]
                           if re.fullmatch(rf"{alias}/{NATIVE_ADDRESS_NAME}\.xml", path)]
            if len(descriptors) != 1:
                raise ValueError(f"native external inline: expected one {alias} descriptor in {fixture}")
            relative, = descriptors
            origin = fixture / relative
            raw = origin.read_bytes()
            recorded = manifest["files"][relative.as_posix()]
            if len(raw) != recorded["size"] or hashlib.sha256(raw).hexdigest() != recorded["sha256"]:
                raise ValueError(f"native external inline: unexpected descriptor bytes in {origin}")
            root = ET.fromstring(raw)
            if (root.tag != f"{md}MetaDataObject" or root.get("version") != version
                    or len(root) != 1 or root[0].tag != f"{md}{kind}"
                    or root[0].findtext(f"{md}Properties/{md}Name") != relative.stem):
                raise ValueError(f"native external inline: invalid registered root in {origin}")
            artifact = root[0]
            owner = f"{alias}:{kind}.{relative.stem}"
            child_path = f"{md}ChildObjects"
            property_name = f"{md}Properties/{md}Name"

            def observation(parent: str, branch: str, names: list[str]) -> dict:
                if len(names) != len(set(names)) or any(not re.fullmatch(NATIVE_ADDRESS_NAME, name) for name in names):
                    raise ValueError(f"native external inline: invalid or duplicate {branch} members in {origin}")
                at = f"{parent}.{branch}"
                return {"owner": parent, "origin": origin.relative_to(repo).as_posix(),
                        "addresses": [at, *(f"{at}.{name}" for name in names)]}

            def named_children(node: ET.Element, kind: str) -> list[str]:
                return [required_text(item.findtext(property_name), kind)
                        for item in node.findall(f"{child_path}/{md}{kind}")]

            tables = artifact.findall(f"{child_path}/{md}TabularSection")
            table_names = named_children(artifact, "TabularSection")
            columns = []
            standards = []
            for table, name in zip(tables, table_names, strict=True):
                table_at = f"{owner}.TabularSection.{name}"
                columns.append(observation(table_at, "Attribute", named_children(table, "Attribute")))
                declared = table.find(f"{md}Properties/{md}StandardAttributes")
                if declared is not None:
                    standards.append(observation(table_at, "StandardAttribute", [
                        required_text(item.get("name"), "standard attribute name")
                        for item in declared.findall(f"{xr}StandardAttribute")
                    ]))
            for branch, kinds, observations in (
                ("Attribute", (kind,), [observation(owner, "Attribute", named_children(artifact, "Attribute"))]),
                ("TabularSection", (kind,), [observation(owner, "TabularSection", table_names)]),
                ("Attribute", (kind, "TabularSection"), columns),
                ("StandardAttribute", (kind, "TabularSection"), standards),
            ):
                if not observations:
                    continue
                rows.append({
                    "schema_version": SCHEMA_VERSION, "record_type": "required",
                    "key": {"line": line, "source_kind": source_kind, "branch": branch, "role": None,
                            "owner_shape": [{"kind": item, "named": True, "role": None} for item in kinds]},
                    "requirement": {"kind": "read", "observations": observations},
                })
    return rows


def generate_report(native_only: bool) -> dict:
    revision, status = git_state()
    cases = [("unica-coder", "address_coverage")]
    required_shards = NATIVE_SHARDS | (REQUIRED_KEY_SHARDS - {SUBSYSTEM_INTERFACE_KEYS_SHARD})
    scope = NATIVE_SCOPE
    if native_only:
        scope += f" {NATIVE_ONLY_MODULE_SCOPE}"
    else:
        cases.append(("unica-daemon", DAEMON_SHARD))
        required_shards |= {DAEMON_SHARD, SUBSYSTEM_INTERFACE_KEYS_SHARD}
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
        for shard, producer in (
            (DCS_QUERY_KEYS_SHARD, native_dcs_query_requirements),
            (ROLE_RIGHT_KEYS_SHARD, native_role_right_requirements),
            (XDTO_KEYS_SHARD, native_xdto_requirements),
            (SUBSYSTEM_CONTENT_KEYS_SHARD, native_subsystem_content_requirements),
            (WSDL_KEYS_SHARD, native_wsdl_requirements),
            (SCHEDULE_KEYS_SHARD, native_schedule_requirements),
            (EXTERNAL_INLINE_KEYS_SHARD, native_external_inline_requirements),
            (EXTENSION_LANGUAGE_KEYS_SHARD, native_adopted_extension_language_requirements),
            (MXL_KEYS_SHARD, lambda repo: native_mxl_requirements(repo, native_only=native_only)),
        ):
            records = producer(REPO)
            path = Path(scratch) / f"{shard}.jsonl"
            path.write_text("".join(f"{canonical_json(row)}\n" for row in records), encoding="utf-8")
        if not native_only:
            records = native_subsystem_interface_requirements(REPO)
            path = Path(scratch) / f"{SUBSYSTEM_INTERFACE_KEYS_SHARD}.jsonl"
            path.write_text("".join(f"{canonical_json(row)}\n" for row in records), encoding="utf-8")
        rows = merge_shards(Path(scratch), required_shards, native_only=native_only)
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
    except (OSError, ValueError, ET.ParseError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    output = render_markdown(report) if args.markdown else json.dumps(
        report, ensure_ascii=False, sort_keys=True, indent=2
    )
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    print(output)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
