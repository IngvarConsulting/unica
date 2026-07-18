#!/usr/bin/env python3
"""Standalone Task 6 query-v3 design-golden generator.

This file is review evidence, not production code.  It intentionally uses only
the Python standard library and constructs every v3 payload twice:

* path A is an imperative append encoder;
* path B is a declarative schema walker with separate validation/framing code.

Any mismatch, stale registry value, noncanonical final vector, unexpected
golden, or failed mutation/negative assertion aborts with a non-zero exit.

The explicit source/digest fixtures below are design-generator authority only.
They are deliberately not a model of the production constructor, whose sole
input authority is ``&PlatformCatalogContextV1``.  Production obtains an owned
``AtomicSourceIdentityV2`` from ``analysis.source_identity()`` and appends it
directly; the two paths here independently reproduce those owned bytes.
"""

from __future__ import annotations

from dataclasses import dataclass, replace
from hashlib import sha256
import json
from pathlib import Path
from typing import Iterable


if not __debug__:
    raise SystemExit(
        "Task 6 query-v3 generator rejects Python optimized mode; run without -O"
    )


DOMAIN_V2 = b"unica.snapshot-bsl-provider-query/v2"
DOMAIN_V3 = b"unica.snapshot-bsl-provider-query/v3"
SOURCE_IDENTITY_DOMAIN = "unica.source-set-identity.v1"

REGISTRY_MANIFEST_PATH = Path(__file__).with_name("task-6-v3-registry-manifest.json")
REGISTRY_MANIFEST_SHA256 = "13e5368de7f84af9ef649c5a82b7126345197dd0e29a79e252f163a604aa66b6"
REGISTRY_MANIFEST_SCHEMA = "unica.task6-query-v3-registry-manifest/v1"


def _reject_duplicate_json_keys(pairs: list[tuple[object, object]]) -> dict[object, object]:
    result: dict[object, object] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate manifest key: {key!r}")
        result[key] = value
    return result


def _reject_json_constant(value: str) -> object:
    raise ValueError(f"invalid manifest JSON constant: {value!r}")


def _validate_registry_tags(
    value: object,
    *,
    name: str,
    expected_keys: frozenset[str],
    minimum: int,
    maximum: int,
) -> dict[str, int]:
    if type(value) is not dict or not value:
        raise ValueError(f"manifest {name} must be a non-empty object")
    if set(value) != expected_keys:
        raise ValueError(f"manifest {name} has unknown or missing registry keys")

    validated: dict[str, int] = {}
    for key, tag in value.items():
        if (
            type(key) is not str
            or not key.isascii()
            or not key.isidentifier()
            or not key[0].isupper()
        ):
            raise ValueError(f"manifest {name} has an invalid registry key")
        if type(tag) is not int or not minimum <= tag <= maximum:
            raise ValueError(f"manifest {name} has an out-of-range registry tag")
        validated[key] = tag
    if len(set(validated.values())) != len(validated):
        raise ValueError(f"manifest {name} has duplicate registry tags")
    return validated


def _load_registry_manifest() -> tuple[
    dict[str, int], dict[str, int], dict[str, int], dict[str, int], int
]:
    try:
        raw = REGISTRY_MANIFEST_PATH.read_bytes()
    except OSError as exc:
        raise SystemExit("Task 6 query-v3 registry manifest is unavailable") from exc
    if sha256(raw).hexdigest() != REGISTRY_MANIFEST_SHA256:
        raise SystemExit("Task 6 query-v3 registry manifest SHA-256 mismatch")
    try:
        manifest = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
            parse_constant=_reject_json_constant,
        )
    except (UnicodeDecodeError, json.JSONDecodeError, ValueError) as exc:
        raise SystemExit(f"Task 6 query-v3 registry manifest is invalid: {exc}") from exc

    expected_keys = {
        "schema",
        "evidencePort",
        "sourceKind",
        "sourceFormat",
        "artifactKind",
        "registeredFormCatalogContractVersion",
    }
    if type(manifest) is not dict or set(manifest) != expected_keys:
        raise SystemExit("Task 6 query-v3 registry manifest has an invalid schema")
    if manifest["schema"] != REGISTRY_MANIFEST_SCHEMA:
        raise SystemExit("Task 6 query-v3 registry manifest has an unknown schema")
    try:
        port_tags = _validate_registry_tags(
            manifest["evidencePort"],
            name="evidencePort",
            expected_keys=frozenset({"CallGraph", "CodeSearch", "Definition"}),
            minimum=1,
            maximum=0xFFFF,
        )
        source_kind_tags = _validate_registry_tags(
            manifest["sourceKind"],
            name="sourceKind",
            expected_keys=frozenset({"Configuration"}),
            minimum=1,
            maximum=0xFF,
        )
        source_format_tags = _validate_registry_tags(
            manifest["sourceFormat"],
            name="sourceFormat",
            expected_keys=frozenset({"PlatformXml"}),
            minimum=1,
            maximum=0xFF,
        )
        artifact_kind_tags = _validate_registry_tags(
            manifest["artifactKind"],
            name="artifactKind",
            expected_keys=frozenset({"Method"}),
            minimum=1,
            maximum=0xFFFF,
        )
        registered_form_catalog_version = manifest["registeredFormCatalogContractVersion"]
        if (
            type(registered_form_catalog_version) is not int
            or not 1 <= registered_form_catalog_version <= 0xFFFF
        ):
            raise ValueError("manifest registeredFormCatalogContractVersion is out of range")
    except ValueError as exc:
        raise SystemExit(f"Task 6 query-v3 registry manifest is invalid: {exc}") from exc
    return (
        port_tags,
        source_kind_tags,
        source_format_tags,
        artifact_kind_tags,
        registered_form_catalog_version,
    )


(
    PORT_TAGS,
    SOURCE_KIND_TAGS,
    SOURCE_FORMAT_TAGS,
    ARTIFACT_KIND_TAGS,
    REGISTERED_FORM_CATALOG_VERSION,
) = _load_registry_manifest()

CONFIGURATION_CATALOG_DIGEST = (
    "279d317b18203fa02829d9dbfa19359913e310bddf3beee5bfd82fc5240046b9"
)
REGISTERED_FORM_CATALOG_DIGEST = (
    "cc7b8add787c08ad7678218574e5a9a55395c7959440208f9a635ed5ab222cd2"
)

EXPECTED_RESOLVED = (
    142,
    "e1d804d1e18f2d02679dce05b4e2a822c9a776cfd749a67754c9328fc48d9396",
)
EXPECTED_ATOMIC = (
    148,
    "8543b710e36b6393bd362435b76774cf62e59a24bc5b61ee3926a473a2234710",
)

# label -> (payload length, SHA-256(payload), H(v3 domain, payload))
EXPECTED_V3 = {
    "CodeSearch empty": (
        254,
        "96671f23b236f560865fe808ad2e12e9c99c19d7f0e0980a9939e8cc93f81112",
        "f0c11bd41c207547a9eb7bc8f5230edc04e6ae0bef8340039b66979c4db90683",
    ),
    "CodeSearch one": (
        264,
        "5be4fa92d5f5347458695e7127fe40eed5e5f286a8f2fcefa86642922fe12964",
        "b14163b7ec4244043e4c98919c1ab2f3393fa39d8696c67a3bc96838ca2fda1a",
    ),
    "Definition empty": (
        254,
        "785e54a281b9fd6359a14d54320f552ed5b9a8869ecb0b905418520203effe0a",
        "2ca861ede84017e0bf6e8e110bb079bf784329a15ef325c24fcd9c962af6a9ae",
    ),
    "Definition one": (
        281,
        "55b7c16f7fb1c9b2b44ca5dc331de40db66498c5a9c213bbb3874e34d44381e4",
        "61d0dc8d91a05346311fcf5b8a087b19f6b7eed3e0bf5f0118765e24c12a2049",
    ),
    "CallGraph empty": (
        254,
        "99b8abf1eb160ef255e78ed8033ed7199364defb6b877530104e36bf14e572f3",
        "ea5f488e008df9ac016bf78b6c60d06193164d3d6f3e2fc45a02185772510060",
    ),
    "CallGraph one": (
        281,
        "7eadca7ec37c752cbe05fe9fdf6bc924c05f17227433239932d0ee327dc2908e",
        "97f2faf6e7d9901b2b70d3d972492dfd19cfcc522c419bd68107034846008372",
    ),
}

EXPECTED_NEGATIVE = (
    258,
    "42c3d46a2b338e08628e9e6404829d43db176a8f25897a2c92b5edb91af1b1b9",
    "1ea160f0b9bfacbb134047a6d1be1d23dc45961ca0a8311505420e6d25520c18",
)

EXPECTED_V2 = (
    220,
    "3d363a007dacb05ffaeabf40ce645e793979b8b5f5391e86224e9fd79582b709",
    "78cc2f7fa751f7e5c52c669e668c2031abcbc6919ce137fe6fc4f1d41329a0cc",
)


@dataclass(frozen=True)
class SourceFixture:
    role: int = 1
    name: str = "analysis"
    kind: str = "Configuration"
    source_format: str = "PlatformXml"
    relative_root: str = "."
    mapping_digest: str = "sha256:" + ("a" * 64)


@dataclass(frozen=True, order=True)
class MethodFixture:
    kind: str
    canonical_ref: str


@dataclass(frozen=True)
class QueryFixture:
    port: str
    source: SourceFixture
    source_fingerprint: str
    configuration_catalog_digest: str
    registered_form_catalog_version: int
    registered_form_catalog_digest: str
    members: tuple[str | MethodFixture, ...]
    max_records: int


SOURCE = SourceFixture()
SOURCE_FINGERPRINT = "sha256:" + ("b" * 64)
METHOD = MethodFixture("Method", "commonmodule.flow.run")


def _domain_hash(domain: bytes, payload: bytes) -> str:
    framed = (
        len(domain).to_bytes(4, "big")
        + domain
        + len(payload).to_bytes(4, "big")
        + payload
    )
    return sha256(framed).hexdigest()


def _summary(domain: bytes, payload: bytes) -> tuple[int, str, str]:
    return len(payload), sha256(payload).hexdigest(), _domain_hash(domain, payload)


def _require(condition: bool, message: str) -> None:
    if not condition:
        raise AssertionError(message)


def _expect_raises(callable_) -> None:
    try:
        callable_()
    except (TypeError, ValueError):
        return
    raise AssertionError("expected canonicality/contract rejection")


# ---------------------------------------------------------------------------
# Path A: imperative append encoder.


def _a_uint(value: int, width: int) -> bytes:
    if not isinstance(value, int) or isinstance(value, bool):
        raise TypeError("integer required")
    if value < 0 or value >= 1 << (8 * width):
        raise ValueError("integer overflow")
    return value.to_bytes(width, "big")


def _a_utf8_frame(value: str, width: int) -> bytes:
    if not isinstance(value, str):
        raise TypeError("string required")
    encoded = value.encode("utf-8")
    return _a_uint(len(encoded), width) + encoded


def _a_hex32(value: str) -> bytes:
    if len(value) != 64 or value != value.lower():
        raise ValueError("canonical lowercase hex digest required")
    try:
        decoded = bytes.fromhex(value)
    except ValueError as exc:
        raise ValueError("canonical lowercase hex digest required") from exc
    if len(decoded) != 32:
        raise ValueError("32-byte digest required")
    return decoded


def _a_fingerprint32(value: str) -> bytes:
    if not value.startswith("sha256:") or len(value) != 71:
        raise ValueError("canonical sha256 fingerprint required")
    return _a_hex32(value[7:])


def encode_resolved_a(source: SourceFixture) -> bytes:
    if source.kind not in SOURCE_KIND_TAGS:
        raise ValueError("unknown source kind")
    if source.source_format not in SOURCE_FORMAT_TAGS:
        raise ValueError("unknown source format")
    out = bytearray()
    out += _a_utf8_frame(SOURCE_IDENTITY_DOMAIN, 8)
    out += _a_utf8_frame(source.name, 8)
    out += _a_uint(SOURCE_KIND_TAGS[source.kind], 1)
    out += _a_uint(SOURCE_FORMAT_TAGS[source.source_format], 1)
    out += _a_utf8_frame(source.relative_root, 8)
    out += _a_utf8_frame(source.mapping_digest, 8)
    _a_fingerprint32(source.mapping_digest)
    return bytes(out)


def encode_atomic_a(source: SourceFixture) -> bytes:
    resolved = encode_resolved_a(source)
    return _a_uint(source.role, 2) + _a_uint(len(resolved), 4) + resolved


def _validate_query_a(query: QueryFixture) -> None:
    if query.port not in PORT_TAGS:
        raise ValueError("unknown port")
    if not 1 <= query.max_records <= 2000:
        raise ValueError("invalid max_records")
    _a_fingerprint32(query.source_fingerprint)
    _a_hex32(query.configuration_catalog_digest)
    _a_uint(query.registered_form_catalog_version, 2)
    _a_hex32(query.registered_form_catalog_digest)
    if query.port == "CodeSearch":
        if len(query.members) > 128 or not all(isinstance(x, str) for x in query.members):
            raise ValueError("CodeSearch requires terms")
        keys = []
        for term in query.members:
            encoded = term.encode("utf-8")
            if not term.strip() or len(encoded) > 256:
                raise ValueError("invalid term")
            keys.append(encoded)
    else:
        if len(query.members) > 2000 or not all(
            isinstance(x, MethodFixture) for x in query.members
        ):
            raise ValueError("Definition/CallGraph require Methods")
        keys = []
        for method in query.members:
            if method.kind != "Method" or method.canonical_ref != method.canonical_ref.lower():
                raise ValueError("invalid canonical Method")
            keys.append(
                _a_uint(ARTIFACT_KIND_TAGS[method.kind], 2)
                + _a_utf8_frame(method.canonical_ref, 4)
            )
    if keys != sorted(keys) or len(keys) != len(set(keys)):
        raise ValueError("final vector must be strictly sorted and unique")


def encode_query_a(query: QueryFixture) -> bytes:
    _validate_query_a(query)
    out = bytearray()
    out += _a_uint(PORT_TAGS[query.port], 2)
    out += encode_atomic_a(query.source)
    out += _a_fingerprint32(query.source_fingerprint)
    out += _a_hex32(query.configuration_catalog_digest)
    out += _a_uint(query.registered_form_catalog_version, 2)
    out += _a_hex32(query.registered_form_catalog_digest)
    out += _a_uint(len(query.members), 4)
    if query.port == "CodeSearch":
        for term in query.members:
            out += _a_utf8_frame(term, 4)
    else:
        for method in query.members:
            out += _a_uint(ARTIFACT_KIND_TAGS[method.kind], 2)
            out += _a_utf8_frame(method.canonical_ref, 4)
    out += _a_uint(query.max_records, 2)
    return bytes(out)


# ---------------------------------------------------------------------------
# Path B: declarative schema walker.  It shares no path-A field helpers.


Schema = tuple


def _walk_b(node: Schema) -> bytes:
    tag = node[0]
    if tag == "sequence":
        return b"".join(_walk_b(child) for child in node[1])
    if tag == "uint":
        width, value = node[1], node[2]
        if not isinstance(value, int) or isinstance(value, bool):
            raise TypeError("schema integer required")
        if value < 0 or value.bit_length() > width * 8:
            raise ValueError("schema integer overflow")
        return value.to_bytes(width, byteorder="big", signed=False)
    if tag == "utf8-frame":
        width, value = node[1], node[2]
        if not isinstance(value, str):
            raise TypeError("schema string required")
        raw = value.encode("utf-8")
        if len(raw).bit_length() > width * 8:
            raise ValueError("schema frame overflow")
        return len(raw).to_bytes(width, "big") + raw
    if tag == "frame":
        width, child = node[1], node[2]
        raw = _walk_b(child)
        if len(raw).bit_length() > width * 8:
            raise ValueError("schema frame overflow")
        return len(raw).to_bytes(width, "big") + raw
    if tag == "hex32":
        value = node[1]
        if not isinstance(value, str) or len(value) != 64 or value.lower() != value:
            raise ValueError("schema canonical digest required")
        try:
            raw = bytes.fromhex(value)
        except ValueError as exc:
            raise ValueError("schema canonical digest required") from exc
        if len(raw) != 32:
            raise ValueError("schema 32-byte digest required")
        return raw
    if tag == "fingerprint32":
        value = node[1]
        if not isinstance(value, str) or not value.startswith("sha256:"):
            raise ValueError("schema canonical fingerprint required")
        return _walk_b(("hex32", value[7:]))
    raise ValueError(f"unknown schema node: {tag!r}")


def _resolved_schema_b(source: SourceFixture) -> Schema:
    if source.kind not in SOURCE_KIND_TAGS or source.source_format not in SOURCE_FORMAT_TAGS:
        raise ValueError("unknown source registry tag")
    return (
        "sequence",
        (
            ("utf8-frame", 8, SOURCE_IDENTITY_DOMAIN),
            ("utf8-frame", 8, source.name),
            ("uint", 1, SOURCE_KIND_TAGS[source.kind]),
            ("uint", 1, SOURCE_FORMAT_TAGS[source.source_format]),
            ("utf8-frame", 8, source.relative_root),
            ("utf8-frame", 8, source.mapping_digest),
        ),
    )


def encode_resolved_b(source: SourceFixture) -> bytes:
    _walk_b(("fingerprint32", source.mapping_digest))
    return _walk_b(_resolved_schema_b(source))


def _atomic_schema_b(source: SourceFixture) -> Schema:
    return (
        "sequence",
        (("uint", 2, source.role), ("frame", 4, _resolved_schema_b(source))),
    )


def encode_atomic_b(source: SourceFixture) -> bytes:
    _walk_b(("fingerprint32", source.mapping_digest))
    return _walk_b(_atomic_schema_b(source))


def _query_schema_b(query: QueryFixture) -> Schema:
    if query.port not in PORT_TAGS:
        raise ValueError("unknown query port")
    if not isinstance(query.max_records, int) or isinstance(query.max_records, bool):
        raise TypeError("schema max_records required")
    if not 1 <= query.max_records <= 2000:
        raise ValueError("schema invalid max_records")

    member_nodes: list[Schema] = []
    member_keys: list[bytes] = []
    if query.port == "CodeSearch":
        if len(query.members) > 128:
            raise ValueError("schema term limit")
        for member in query.members:
            if not isinstance(member, str):
                raise TypeError("schema term required")
            raw = member.encode("utf-8")
            if not member.strip() or len(raw) > 256:
                raise ValueError("schema invalid term")
            member_keys.append(raw)
            member_nodes.append(("utf8-frame", 4, member))
    else:
        if len(query.members) > 2000:
            raise ValueError("schema Method limit")
        for member in query.members:
            if not isinstance(member, MethodFixture):
                raise TypeError("schema Method required")
            if member.kind not in ARTIFACT_KIND_TAGS or member.kind != "Method":
                raise ValueError("schema unknown artifact tag")
            if member.canonical_ref != member.canonical_ref.lower():
                raise ValueError("schema noncanonical Method")
            item = (
                "sequence",
                (
                    ("uint", 2, ARTIFACT_KIND_TAGS[member.kind]),
                    ("utf8-frame", 4, member.canonical_ref),
                ),
            )
            member_nodes.append(item)
            member_keys.append(_walk_b(item))
    if member_keys != sorted(member_keys) or len(member_keys) != len(set(member_keys)):
        raise ValueError("schema final vector must be strictly sorted and unique")

    return (
        "sequence",
        (
            ("uint", 2, PORT_TAGS[query.port]),
            _atomic_schema_b(query.source),
            ("fingerprint32", query.source_fingerprint),
            ("hex32", query.configuration_catalog_digest),
            ("uint", 2, query.registered_form_catalog_version),
            ("hex32", query.registered_form_catalog_digest),
            ("uint", 4, len(member_nodes)),
            *member_nodes,
            ("uint", 2, query.max_records),
        ),
    )


def encode_query_b(query: QueryFixture) -> bytes:
    _walk_b(("fingerprint32", query.source.mapping_digest))
    return _walk_b(_query_schema_b(query))


def canonical_terms(values: Iterable[str]) -> tuple[str, ...]:
    materialized = tuple(values)
    if len(materialized) != len(set(materialized)):
        raise ValueError("duplicate term input")
    return tuple(sorted(materialized, key=lambda item: item.encode("utf-8")))


def _method_artifact_identity_bytes(method: MethodFixture) -> bytes:
    if method.kind not in ARTIFACT_KIND_TAGS or method.kind != "Method":
        raise ValueError("unknown Method artifact tag")
    canonical_ref = method.canonical_ref.encode("utf-8")
    return (
        ARTIFACT_KIND_TAGS[method.kind].to_bytes(2, "big")
        + len(canonical_ref).to_bytes(4, "big")
        + canonical_ref
    )


def canonical_methods(values: Iterable[str]) -> tuple[MethodFixture, ...]:
    materialized = tuple(MethodFixture("Method", item.lower()) for item in values)
    keys = tuple(_method_artifact_identity_bytes(item) for item in materialized)
    if len(keys) != len(set(keys)):
        raise ValueError("duplicate Method input")
    return tuple(sorted(materialized, key=_method_artifact_identity_bytes))


def query(port: str, members: tuple[str | MethodFixture, ...]) -> QueryFixture:
    return QueryFixture(
        port=port,
        source=SOURCE,
        source_fingerprint=SOURCE_FINGERPRINT,
        configuration_catalog_digest=CONFIGURATION_CATALOG_DIGEST,
        registered_form_catalog_version=REGISTERED_FORM_CATALOG_VERSION,
        registered_form_catalog_digest=REGISTERED_FORM_CATALOG_DIGEST,
        members=members,
        max_records=7,
    )


def _negative_extra_frame_a(definition_empty: QueryFixture) -> bytes:
    atomic = encode_atomic_a(definition_empty.source)
    direct = encode_query_a(definition_empty)
    return direct[:2] + _a_uint(len(atomic), 4) + direct[2:]


def _negative_extra_frame_b(definition_empty: QueryFixture) -> bytes:
    schema = _query_schema_b(definition_empty)
    fields = list(schema[1])
    fields[1] = ("frame", 4, fields[1])
    return _walk_b(("sequence", tuple(fields)))


def _assert_direct_atomic(payload: bytes, source: SourceFixture) -> None:
    expected_prefix = source.role.to_bytes(2, "big")
    if payload[2:4] != expected_prefix:
        raise ValueError("redundant AtomicSourceIdentityV2 frame")


def _encode_v2_a(definition_empty: QueryFixture) -> bytes:
    out = bytearray()
    out += _a_uint(PORT_TAGS["Definition"], 2)
    out += encode_atomic_a(definition_empty.source)
    out += _a_fingerprint32(definition_empty.source_fingerprint)
    out += _a_hex32("c" * 64)
    out += _a_uint(0, 4)
    out += _a_uint(definition_empty.max_records, 2)
    return bytes(out)


def _encode_v2_b(definition_empty: QueryFixture) -> bytes:
    return _walk_b(
        (
            "sequence",
            (
                ("uint", 2, PORT_TAGS["Definition"]),
                _atomic_schema_b(definition_empty.source),
                ("fingerprint32", definition_empty.source_fingerprint),
                ("hex32", "c" * 64),
                ("uint", 4, 0),
                ("uint", 2, definition_empty.max_records),
            ),
        )
    )


def main() -> None:
    resolved_a = encode_resolved_a(SOURCE)
    resolved_b = encode_resolved_b(SOURCE)
    atomic_a = encode_atomic_a(SOURCE)
    atomic_b = encode_atomic_b(SOURCE)
    _require(resolved_a == resolved_b, "resolved-source encoders disagree")
    _require(atomic_a == atomic_b, "atomic-source encoders disagree")
    _require(
        (len(resolved_a), sha256(resolved_a).hexdigest()) == EXPECTED_RESOLVED,
        "resolved-source golden changed",
    )
    _require(
        (len(atomic_a), sha256(atomic_a).hexdigest()) == EXPECTED_ATOMIC,
        "atomic-source golden changed",
    )

    rows = {
        "CodeSearch empty": query("CodeSearch", ()),
        "CodeSearch one": query("CodeSearch", canonical_terms(("Needle",))),
        "Definition empty": query("Definition", ()),
        "Definition one": query("Definition", (METHOD,)),
        "CallGraph empty": query("CallGraph", ()),
        "CallGraph one": query("CallGraph", (METHOD,)),
    }
    payloads: dict[str, bytes] = {}
    for label, fixture in rows.items():
        path_a = encode_query_a(fixture)
        path_b = encode_query_b(fixture)
        _require(path_a == path_b, f"query encoders disagree for {label}")
        _assert_direct_atomic(path_a, fixture.source)
        _require(
            _summary(DOMAIN_V3, path_a) == EXPECTED_V3[label],
            f"query golden changed for {label}",
        )
        payloads[label] = path_a

    definition_empty = rows["Definition empty"]
    negative_a = _negative_extra_frame_a(definition_empty)
    negative_b = _negative_extra_frame_b(definition_empty)
    _require(negative_a == negative_b, "redundant-frame encoders disagree")
    _require(
        _summary(DOMAIN_V3, negative_a) == EXPECTED_NEGATIVE,
        "redundant-frame negative golden changed",
    )
    _require(
        negative_a != payloads["Definition empty"],
        "redundant-frame negative equals the canonical query",
    )
    _expect_raises(lambda: _assert_direct_atomic(negative_a, SOURCE))

    # A reverse input order must canonicalize to identical final vectors.
    terms_forward = canonical_terms(("Alpha", "Zulu"))
    terms_reverse = canonical_terms(("Zulu", "Alpha"))
    _require(terms_forward == terms_reverse, "term canonicalization depends on input order")
    _require(
        encode_query_a(query("CodeSearch", terms_forward))
        == encode_query_a(query("CodeSearch", terms_reverse)),
        "CodeSearch bytes depend on input order",
    )
    # These refs deliberately have unequal encoded lengths. Lexical tuple order
    # would put "aa" before "z", while exact ArtifactIdentityBytesV1 order puts
    # the shorter encoded "z" identity first because string() starts with u32be
    # byte length. Reverse input must therefore still produce that byte order.
    methods_forward = canonical_methods(("CommonModule.AA.Run", "CommonModule.Z.Run"))
    methods_reverse = canonical_methods(("CommonModule.Z.Run", "CommonModule.AA.Run"))
    _require(
        methods_forward == methods_reverse,
        "Method canonicalization depends on input order",
    )
    _require(
        methods_forward
        == (
            MethodFixture("Method", "commonmodule.z.run"),
            MethodFixture("Method", "commonmodule.aa.run"),
        ),
        "Method canonicalization does not use exact ArtifactIdentityBytesV1 bytes",
    )
    _require(
        encode_query_b(query("Definition", methods_forward))
        == encode_query_b(query("Definition", methods_reverse)),
        "Definition bytes depend on input order",
    )

    # A duplicate in a direct final vector is invalid, never silently dropped.
    duplicate_terms = query("CodeSearch", ("Needle", "Needle"))
    duplicate_methods = query("Definition", (METHOD, METHOD))
    _expect_raises(lambda: encode_query_a(duplicate_terms))
    _expect_raises(lambda: encode_query_b(duplicate_terms))
    _expect_raises(lambda: encode_query_a(duplicate_methods))
    _expect_raises(lambda: encode_query_b(duplicate_methods))

    # Every specified single-field mutation changes bytes and the domain hash,
    # while both independent paths continue to agree on the mutated encoding.
    baseline_fixture = rows["Definition one"]
    baseline = payloads["Definition one"]
    mutations = {
        "port": replace(baseline_fixture, port="CallGraph"),
        "source-logical-identity": replace(
            baseline_fixture,
            source=replace(baseline_fixture.source, name="analysis-mutated"),
        ),
        "source-fingerprint": replace(
            baseline_fixture, source_fingerprint="sha256:" + ("d" * 64)
        ),
        "configuration-catalog-digest": replace(
            baseline_fixture, configuration_catalog_digest="3" + CONFIGURATION_CATALOG_DIGEST[1:]
        ),
        "registered-form-contract-version": replace(
            baseline_fixture, registered_form_catalog_version=2
        ),
        "registered-form-catalog-digest": replace(
            baseline_fixture, registered_form_catalog_digest="d" * 64
        ),
        "vector-member": replace(
            baseline_fixture,
            members=(MethodFixture("Method", "commonmodule.flow.other"),),
        ),
        "max-records": replace(baseline_fixture, max_records=8),
    }
    baseline_domain_hash = _domain_hash(DOMAIN_V3, baseline)
    for mutation_name, mutated in mutations.items():
        mutation_a = encode_query_a(mutated)
        mutation_b = encode_query_b(mutated)
        _require(
            mutation_a == mutation_b,
            f"mutation encoders disagree for {mutation_name}",
        )
        _require(
            mutation_a != baseline,
            f"mutation did not change payload bytes: {mutation_name}",
        )
        _require(
            _domain_hash(DOMAIN_V3, mutation_a) != baseline_domain_hash,
            f"mutation did not change domain hash: {mutation_name}",
        )

    v2_a = _encode_v2_a(definition_empty)
    v2_b = _encode_v2_b(definition_empty)
    _require(v2_a == v2_b, "historical-v2 encoders disagree")
    _require(
        _summary(DOMAIN_V2, v2_a) == EXPECTED_V2,
        "historical-v2 negative golden changed",
    )
    _require(
        all(v2_a != payload for payload in payloads.values()),
        "historical-v2 payload equals a v3 payload",
    )

    print("Task 6 query-v3 standalone golden generator: PASS")
    print(
        f"ResolvedSourceSetIdentityBytesV1|length={len(resolved_a)}|"
        f"sha256={sha256(resolved_a).hexdigest()}|hex={resolved_a.hex()}"
    )
    print(
        f"AtomicSourceIdentityV2|length={len(atomic_a)}|"
        f"sha256={sha256(atomic_a).hexdigest()}|hex={atomic_a.hex()}"
    )
    for label, payload in payloads.items():
        length, payload_sha, domain_sha = _summary(DOMAIN_V3, payload)
        print(
            f"{label}|length={length}|sha256={payload_sha}|"
            f"domain_hash={domain_sha}|payload_hex={payload.hex()}"
        )
    negative_length, negative_sha, negative_domain_sha = _summary(DOMAIN_V3, negative_a)
    print(
        "Definition empty forbidden extra frame|"
        f"length={negative_length}|sha256={negative_sha}|"
        f"domain_hash={negative_domain_sha}|payload_hex={negative_a.hex()}"
    )
    v2_length, v2_sha, v2_domain_sha = _summary(DOMAIN_V2, v2_a)
    print(
        f"historical v2 negative|length={v2_length}|sha256={v2_sha}|"
        f"domain_hash={v2_domain_sha}"
    )
    print(
        "assertions|two_paths=PASS|owned_atomic_direct=PASS|registries=PASS|"
        "canonical_order=PASS|"
        "duplicate_rejection=PASS|single_field_mutations=8/8|"
        "redundant_frame_rejection=PASS|historical_v2_inequality=PASS"
    )


if __name__ == "__main__":
    main()
