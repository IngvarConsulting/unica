#!/usr/bin/env python3
"""Build the exact new-only contract without invoking the new adapter.

The input inventory was independently reviewed from native fixtures and closed
core contracts in Task 5 fix round 5.  This module upgrades that inventory to
the complete public JSON shape using explicit core-contract rules.  It imports
neither the Platform XML adapter nor Rust normalization code.
"""

from __future__ import annotations

import json
import re
import xml.etree.ElementTree as ET
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


PUBLIC_SCHEMA = {
    "NavigationEnvelope": {
        "required": [
            "schemaVersion",
            "status",
            "snapshot",
            "root",
            "nodes",
            "relations",
            "diagnostics",
        ],
        "optional": [],
    },
    "NavigationNode": {
        "required": [
            "objectRef",
            "reference",
            "properties",
            "facets",
            "capabilityState",
            "capability",
            "actionProfile",
            "semanticActions",
            "actions",
        ],
        "optional": [],
    },
    "SemanticProperty": {
        "required": ["type", "valueState", "provenance", "capability"],
        "optional": ["value"],
    },
    "CapabilityState": {
        "required": ["resolutionState", "authorability"],
        "optional": [],
    },
    "CapabilityVector": {
        "required": [
            "resolution",
            "identity",
            "consistency",
            "coverage",
            "format",
            "sourceAccess",
            "authorability",
        ],
        "optional": [],
    },
    "SemanticRelation": {
        "required": [
            "relationRef",
            "groupRef",
            "identityStrength",
            "kind",
            "role",
            "source",
            "target",
            "capability",
        ],
        "optional": [],
    },
    "RelationRef": {
        "required": ["sourceId", "relationKey", "kind"],
        "optional": [],
    },
    "RelationGroupRef": {
        "required": ["sourceId", "groupKey", "owner", "role", "kind"],
        "optional": [],
    },
    "ObjectRef": {
        "required": [
            "sourceId",
            "objectKey",
            "identityStrength",
            "kind",
            "displayName",
        ],
        "optional": [],
    },
    "SourceSnapshot": {
        "required": ["sourceId", "revision", "consistency", "adapterId"],
        "optional": [],
    },
    "SourceAdapterDiagnostic": {
        "required": ["code", "message"],
        "optional": ["details"],
    },
    "SemanticAction": {
        "required": [
            "kind",
            "target",
            "owningRelation",
            "availability",
            "blockingReasons",
            "operationBinding",
            "atomicity",
        ],
        "optional": [],
    },
}

NEW_ENUM_PROPERTIES = {
    "ChoiceMode": (
        "choice.mode",
        "presentation",
        {"BothWays": "bothWays", "FromForm": "fromForm", "QuickChoice": "quickChoice"},
    ),
    "CodeAllowedLength": (
        "code.allowedLength",
        "numbering",
        {"Fixed": "fixed", "Variable": "variable"},
    ),
    "CodeType": (
        "code.type",
        "numbering",
        {"Number": "number", "String": "string"},
    ),
    "DataLockControlMode": (
        "locking.controlMode",
        "structure",
        {"Automatic": "automatic", "Managed": "managed"},
    ),
    "DefaultPresentation": (
        "presentation.defaultMode",
        "presentation",
        {"AsCode": "asCode", "AsDescription": "asDescription"},
    ),
    "DependenceOnCalculationTypes": (
        "calculation.dependenceMode",
        "structure",
        {"DontUse": "dontUse", "OnActionPeriod": "onActionPeriod"},
    ),
    "EditType": (
        "editing.mode",
        "presentation",
        {"BothWays": "bothWays", "InDialog": "inDialog", "InList": "inList"},
    ),
    "FullTextSearch": (
        "search.fullText.mode",
        "structure",
        {"DontUse": "dontUse", "Use": "use"},
    ),
    "NumberAllowedLength": (
        "number.allowedLength",
        "numbering",
        {"Fixed": "fixed", "Variable": "variable"},
    ),
    "NumberType": (
        "number.type",
        "numbering",
        {"Number": "number", "String": "string"},
    ),
    "SubordinationUse": (
        "subordination.use",
        "structure",
        {
            "ToFolders": "toGroups",
            "ToFoldersAndItems": "toGroupsAndItems",
            "ToItems": "toItems",
        },
    ),
}

SUPPORT_PROFILES = {
    "supportNotSupportedContract": ("notSupported", "authorable", "authorable"),
    "supportRemovedContract": ("removedFromSupport", "authorable", "authorable"),
    "supportConfigurationReadOnlyContract": (
        "configurationReadOnly",
        "configurationReadOnly",
        "readOnly",
    ),
    "supportLockedContract": ("supportedLocked", "supportLocked", "readOnly"),
    "supportEditableContract": ("supportedEditable", "authorable", "authorable"),
}

SUPPORT_INPUT_CASES = {
    "supportNotSupportedContract": "supportNotSupported",
    "supportRemovedContract": "supportRemoved",
    "supportConfigurationReadOnlyContract": "supportConfigurationReadOnly",
    "supportLockedContract": "supportLocked",
    "supportEditableContract": "supportEditable",
}

REFERENCE_ROLES = {"accessTarget", "basedOn", "references", "registerRecords"}


def canonical(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True)


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def child_text(node: ET.Element, name: str) -> str:
    child = next((item for item in node if local_name(item.tag) == name), None)
    if child is None or child.text is None:
        return ""
    return child.text.strip()


def direct_properties(node: ET.Element) -> ET.Element | None:
    return next(
        (item for item in node if local_name(item.tag) == "Properties"),
        None,
    )


def property_envelope(
    value_type: str,
    value_state: str,
    value: dict[str, Any] | None,
) -> dict[str, Any]:
    provenance = {
        "explicit": "declared",
        "defaulted": "default",
        "inherited": "inherited",
        "computed": "derived",
        "absent": "declared",
        "unresolved": "unknown",
    }[value_state]
    capability = {
        "absent": "unavailable",
        "unresolved": "unknown",
    }.get(value_state, "readOnly")
    result = {
        "type": value_type,
        "valueState": value_state,
        "provenance": provenance,
        "capability": capability,
    }
    if value is not None:
        result["value"] = value
    return result


def enum_property(value: str, state: str = "explicit") -> dict[str, Any]:
    return property_envelope(
        "enum",
        state,
        {"type": "enum", "value": value},
    )


def string_property(value: str) -> dict[str, Any]:
    return property_envelope(
        "string",
        "computed",
        {"type": "string", "value": value},
    )


def explicit_string_property(value: str) -> dict[str, Any]:
    return property_envelope(
        "string",
        "explicit",
        {"type": "string", "value": value},
    )


def boolean_property(value: bool) -> dict[str, Any]:
    return property_envelope(
        "boolean",
        "explicit",
        {"type": "boolean", "value": value},
    )


def computed_boolean_property(value: bool) -> dict[str, Any]:
    return property_envelope(
        "boolean",
        "computed",
        {"type": "boolean", "value": value},
    )


def uuid_property(value: str) -> dict[str, Any]:
    return property_envelope(
        "uuid",
        "explicit",
        {"type": "uuid", "value": value},
    )


def localized_property(values: dict[str, str]) -> dict[str, Any]:
    return property_envelope(
        "localizedString",
        "explicit",
        {"type": "localizedString", "value": dict(sorted(values.items()))},
    )


def object_ref(
    case_id: str,
    identity: str,
    identity_strength: str,
    kind: str,
    name: str,
) -> dict[str, Any]:
    return {
        "sourceId": f"source:{case_id}",
        "objectKey": identity,
        "identityStrength": identity_strength,
        "kind": kind,
        "displayName": name,
    }


def relation_kind(role: str) -> str:
    return "references" if role in REFERENCE_ROLES else "contains"


def relation_key(source: str, role: str, target: str, kind: str) -> str:
    return f"relation:{source}:{role}:{target}:{kind}"


def group_key(source: str, role: str, kind: str) -> str:
    return f"group:{source}:{role}:{kind}"


def action_profile(kind: str) -> str:
    return {
        "document": "document_metadata_object",
        "form": "form",
        "formElement": "form_element",
        "tabularSection": "tabular_section",
        "spreadsheetDocumentTemplate": "mxl_template",
        "template": "unmodeled_template",
        "attribute": "unmodeled_child",
        "dimension": "unmodeled_child",
        "resource": "unmodeled_child",
        "command": "unmodeled_child",
        "formAttribute": "unmodeled_child",
        "formCommand": "unmodeled_child",
        "httpServiceUrlTemplate": "unmodeled_child",
        "httpServiceMethod": "unmodeled_child",
        "webServiceOperation": "unmodeled_child",
        "webServiceParameter": "unmodeled_child",
        "enumerationValue": "unmodeled_child",
    }.get(kind, "generic_metadata_object")


def support_properties(
    state: str,
    authorability: str,
    edit_capability: str,
) -> dict[str, dict[str, Any]]:
    return {
        "support.state": enum_property(state, "computed"),
        "support.authorability": string_property(authorability),
        "support.editCapability": string_property(edit_capability),
    }


def relation_from_fact(
    case_id: str,
    fact: dict[str, Any],
    node_refs: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    source = node_refs[fact["source"]]
    target = node_refs.get(fact["target"])
    if target is None:
        target = object_ref(
            case_id,
            fact["target"],
            "derived",
            fact["targetKind"],
            fact["targetName"],
        )
    kind = relation_kind(fact["role"])
    relation_ref = {
        "sourceId": f"source:{case_id}",
        "relationKey": relation_key(
            fact["source"], fact["role"], fact["target"], kind
        ),
        "kind": kind,
    }
    return {
        "relationRef": relation_ref,
        "groupRef": {
            "sourceId": f"source:{case_id}",
            "groupKey": group_key(fact["source"], fact["role"], kind),
            "owner": source,
            "role": fact["role"],
            "kind": kind,
        },
        "identityStrength": "derived",
        "kind": kind,
        "role": fact["role"],
        "source": source,
        "target": target,
        "capability": {
            "resolution": fact["resolution"],
            "identity": "derived",
            "consistency": "consistent",
            "coverage": fact["coverage"],
            "format": "compatible",
            "sourceAccess": "readOnly",
            "authorability": "derived_read_only",
        },
    }


def augment_diagnostic_details(
    value: Any,
    case_id: str,
    node_refs: dict[str, dict[str, Any]],
) -> Any:
    if isinstance(value, list):
        return [
            augment_diagnostic_details(item, case_id, node_refs) for item in value
        ]
    if not isinstance(value, dict):
        return value
    result = {
        key: augment_diagnostic_details(item, case_id, node_refs)
        for key, item in value.items()
    }
    if {"objectKey", "kind", "displayName"}.issubset(result):
        identity = result["objectKey"]
        reference = node_refs.get(identity)
        if reference is None:
            reference = object_ref(
                case_id,
                identity,
                "derived",
                result["kind"],
                result["displayName"],
            )
        return reference
    return result


def supplement_fixture_properties(
    repo_root: Path,
    source_root: str,
    nodes: dict[str, dict[str, Any]],
    properties: dict[str, dict[str, dict[str, Any]]],
    facets: dict[str, dict[str, set[str]]],
) -> None:
    root = repo_root / source_root
    if not root.exists():
        return
    for path in sorted(root.rglob("*.xml")):
        try:
            document = ET.parse(path).getroot()
        except ET.ParseError:
            continue
        for native_node in document.iter():
            props = direct_properties(native_node)
            if props is None:
                continue
            uuid = native_node.attrib.get("uuid")
            identity = f"uuid:{uuid}" if uuid else None
            if identity not in nodes:
                continue
            synonym = next(
                (
                    child
                    for child in props
                    if local_name(child.tag) == "Synonym"
                ),
                None,
            )
            if synonym is not None:
                values = {}
                for item in synonym:
                    language = next(
                        (
                            child.text.strip()
                            for child in item
                            if local_name(child.tag) == "lang"
                            and child.text
                            and child.text.strip()
                        ),
                        None,
                    )
                    content = next(
                        (
                            child.text.strip()
                            for child in item
                            if local_name(child.tag) == "content"
                            and child.text
                            and child.text.strip()
                        ),
                        None,
                    )
                    if language is not None and content is not None:
                        values[language] = content
                if values:
                    properties[identity]["metadata.synonym"] = localized_property(
                        values
                    )
            for native_property in props:
                mapping = NEW_ENUM_PROPERTIES.get(local_name(native_property.tag))
                if mapping is None or native_property.text is None:
                    continue
                semantic_property, facet, aliases = mapping
                alias = native_property.text.strip()
                if alias not in aliases:
                    raise ValueError(
                        f"new-only contract has unknown enum alias {alias!r} "
                        f"for {local_name(native_property.tag)}"
                    )
                properties[identity][semantic_property] = enum_property(
                    aliases[alias]
                )
                facets[identity][facet].add(semantic_property)


def supplement_rights_properties(
    repo_root: Path,
    source_root: str,
    nodes: dict[str, dict[str, Any]],
    properties: dict[str, dict[str, dict[str, Any]]],
) -> None:
    root = repo_root / source_root
    for identity, node in nodes.items():
        if node["objectKind"] != "role":
            continue
        rights_path = root / node["name"] / "Ext/Rights.xml"
        if not rights_path.exists():
            continue
        rights = ET.parse(rights_path).getroot()
        for attribute, semantic in (
            ("setForNewObjects", "access.newObjects.defaultAllowed"),
            ("setForAttributesByDefault", "access.attributes.defaultAllowed"),
            (
                "independentRightsOfChildObjects",
                "access.childObjects.independent",
            ),
        ):
            value = rights.attrib.get(attribute)
            if value not in {"true", "false"}:
                raise ValueError(
                    f"new-only rights contract has invalid {attribute}"
                )
            properties[identity][semantic] = boolean_property(value == "true")
        ordinal = 0
        for native_object in [
            child for child in rights if local_name(child.tag) == "object"
        ]:
            target = child_text(native_object, "name")
            target_name = target.split(".", 1)[-1]
            for right in [
                child
                for child in native_object
                if local_name(child.tag) == "right"
            ]:
                ordinal += 1
                permission_identity = (
                    f"derived:accessPermission:{target_name}:{ordinal}#1"
                )
                if permission_identity not in nodes:
                    raise ValueError(
                        "new-only rights permission identity is absent: "
                        f"{permission_identity}"
                    )
                permission_name = child_text(right, "name")
                permission_value = child_text(right, "value")
                if permission_value not in {"true", "false"}:
                    raise ValueError(
                        f"new-only rights permission has invalid value {permission_value!r}"
                    )
                properties[permission_identity][
                    "access.permission.name"
                ] = explicit_string_property(permission_name)
                properties[permission_identity][
                    "access.permission.allowed"
                ] = boolean_property(permission_value == "true")
                properties[permission_identity][
                    "metadata.name"
                ] = explicit_string_property(
                    f"{target_name}:{permission_name}:{ordinal}"
                )


def supplement_field_defaults(
    nodes: dict[str, dict[str, Any]],
    properties: dict[str, dict[str, dict[str, Any]]],
    facets: dict[str, dict[str, set[str]]],
) -> None:
    for identity, node in nodes.items():
        if node["objectKind"] not in {"attribute", "dimension", "resource"}:
            continue
        fields = properties[identity]
        fields.setdefault(
            "field.fillChecking",
            enum_property("dontCheck", "computed"),
        )
        fields.setdefault(
            "field.indexing",
            enum_property("dontIndex", "computed"),
        )
        fill_checking = fields["field.fillChecking"].get("value", {}).get("value")
        fields["field.required"] = computed_boolean_property(
            fill_checking == "showError"
        )
        facets[identity]["fields"].update(
            {"field.fillChecking", "field.indexing", "field.required"}
        )


def base_case_to_contract(
    repo_root: Path,
    source_case: dict[str, Any],
    support_profile: tuple[str, str, str] = (
        "notSupported",
        "authorable",
        "authorable",
    ),
) -> dict[str, Any]:
    case_id = source_case["id"]
    facts = source_case["facts"]
    nodes = {
        fact["identity"]: fact
        for fact in facts
        if fact["kind"] == "node"
    }
    coverage = {
        fact["identity"]: fact
        for fact in facts
        if fact["kind"] == "nodeCoverage"
    }
    properties: dict[str, dict[str, dict[str, Any]]] = defaultdict(dict)
    facets: dict[str, dict[str, set[str]]] = defaultdict(
        lambda: defaultdict(set)
    )
    for fact in facts:
        if fact["kind"] == "property":
            properties[fact["identity"]][fact["property"]] = property_envelope(
                fact["valueType"],
                fact["valueState"],
                fact.get("value"),
            )
        elif fact["kind"] == "facetMember":
            facets[fact["identity"]][fact["facet"]].add(fact["member"])

    for identity, node in nodes.items():
        if node["objectKind"] == "sourceRoot":
            continue
        properties[identity].setdefault(
            "metadata.kind",
            string_property(node["objectKind"]),
        )
        properties[identity].setdefault(
            "metadata.name",
            explicit_string_property(node["name"]),
        )
        if node["identityStrength"] == "persistent":
            uuid = identity.removeprefix("uuid:")
            properties[identity].setdefault("metadata.uuid", uuid_property(uuid))

    state, supported_authorability, edit_capability = support_profile
    for identity, node in nodes.items():
        if node["objectKind"] == "sourceRoot":
            continue
        authorability = (
            supported_authorability
            if node["identityStrength"] == "persistent"
            else "derivedReadOnly"
        )
        edit = edit_capability if authorability != "derivedReadOnly" else "readOnly"
        properties[identity].update(
            support_properties(state, authorability, edit)
        )

    supplement_fixture_properties(
        repo_root,
        source_case["sourceRoot"],
        nodes,
        properties,
        facets,
    )
    supplement_rights_properties(
        repo_root,
        source_case["sourceRoot"],
        nodes,
        properties,
    )
    supplement_field_defaults(nodes, properties, facets)

    node_refs = {
        identity: object_ref(
            case_id,
            identity,
            node["identityStrength"],
            node["objectKind"],
            node["name"],
        )
        for identity, node in nodes.items()
    }
    relation_facts = [fact for fact in facts if fact["kind"] == "relation"]
    relations = [
        relation_from_fact(case_id, fact, node_refs)
        for fact in relation_facts
    ]
    owning_relations = {
        relation["target"]["objectKey"]: relation["relationRef"]
        for relation in relations
        if relation["kind"] == "contains"
        and relation["target"]["objectKey"] in node_refs
    }
    serialized_nodes = []
    for identity, fact in nodes.items():
        node_coverage = coverage[identity]
        actual_authorability = (
            "derived_read_only"
            if fact["objectKind"] == "sourceRoot"
            or fact["identityStrength"] != "persistent"
            else {
                "authorable": "authorable",
                "supportLocked": "support_locked",
                "configurationReadOnly": "configuration_read_only",
            }[supported_authorability]
        )
        capability_state_authorability = (
            "derived_read_only"
            if fact["objectKind"] == "sourceRoot"
            else actual_authorability
            if node_coverage["coverage"] == "complete"
            else "unknown_read_only"
        )
        reference = node_refs[identity]
        owning = owning_relations.get(identity)
        action = {
            "kind": "inspect",
            "target": reference,
            "owningRelation": owning,
            "availability": "modeled",
            "blockingReasons": [],
            "operationBinding": None,
            "atomicity": "read_only",
        }
        serialized_nodes.append(
            {
                "objectRef": reference,
                "reference": reference,
                "properties": dict(sorted(properties[identity].items())),
                "facets": {
                    facet: sorted(members)
                    for facet, members in sorted(facets[identity].items())
                    if members
                },
                "capabilityState": {
                    "resolutionState": node_coverage["resolution"],
                    "authorability": capability_state_authorability,
                },
                "capability": {
                    "resolution": node_coverage["resolution"],
                    "identity": fact["identityStrength"],
                    "consistency": "consistent",
                    "coverage": node_coverage["coverage"],
                    "format": "compatible",
                    "sourceAccess": "readOnly",
                    "authorability": actual_authorability,
                },
                "actionProfile": action_profile(fact["objectKind"]),
                "semanticActions": [],
                "actions": [action],
            }
        )
    serialized_nodes.sort(key=canonical)

    diagnostics = []
    for fact in facts:
        if fact["kind"] != "diagnostic":
            continue
        diagnostic = {"code": fact["code"], "message": fact["message"]}
        if fact.get("details") is not None:
            diagnostic["details"] = augment_diagnostic_details(
                fact["details"], case_id, node_refs
            )
        diagnostics.append(diagnostic)
    diagnostics.sort(key=canonical)
    root = next(
        reference
        for reference in node_refs.values()
        if reference["kind"] == "sourceRoot"
    )
    status = next(fact["value"] for fact in facts if fact["kind"] == "status")
    envelope = {
        "schemaVersion": "1",
        "status": status,
        "snapshot": {
            "sourceId": f"source:{case_id}",
            "revision": f"revision:{case_id}",
            "consistency": "consistent",
            "adapterId": "platform-xml-2.20",
        },
        "root": root,
        "nodes": serialized_nodes,
        "relations": [],
        "diagnostics": diagnostics,
    }
    contract_facts = [
        {"case": case_id, "kind": "envelope", "value": envelope},
        *[
            {"case": case_id, "kind": "semanticRelation", "value": relation}
            for relation in relations
        ],
    ]
    return {
        "id": case_id,
        "input": source_case["input"],
        "sourceRoot": source_case["sourceRoot"],
        "facts": sorted(contract_facts, key=canonical),
    }


def facet_members(
    kind: str,
    has_conditions: bool = False,
    relation_roles: set[str] | None = None,
) -> dict[str, set[str]]:
    relation_roles = relation_roles or set()
    result: dict[str, set[str]] = defaultdict(set)
    result["identity"].update({"metadata.kind", "metadata.name"})
    result["support"].update(
        {"support.state", "support.authorability", "support.editCapability"}
    )
    if kind == "role":
        result["identity"].add("metadata.uuid")
        result["access"].update(
            {
                "access.newObjects.defaultAllowed",
                "access.attributes.defaultAllowed",
                "access.childObjects.independent",
            }
        )
        result["backing"].add("backing.content.available")
    elif kind == "accessPermission":
        result["access"].update(
            {"access.permission.name", "access.permission.allowed"}
        )
    if has_conditions:
        result["access"].add("access.restriction.conditions")
    for role in relation_roles:
        if role in {"accessPermissions", "accessTarget", "restrictionTemplates"}:
            result["access"].add(role)
        else:
            result["structure"].add(role)
    return result


def build_multitarget_source_case(repo_root: Path) -> dict[str, Any]:
    case_id = "rightsMultiTargetContract"
    descriptor_path = (
        repo_root
        / "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights/MultiTargetReader.xml"
    )
    rights_path = (
        repo_root
        / "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights/MultiTargetReader/Ext/Rights.xml"
    )
    descriptor = ET.parse(descriptor_path).getroot()
    role = next(node for node in descriptor.iter() if local_name(node.tag) == "Role")
    role_uuid = role.attrib["uuid"]
    role_name = child_text(direct_properties(role), "Name")
    rights = ET.parse(rights_path).getroot()
    prefix_map = json.loads(
        (
            rights_path.parents[3]
            / "legacy-oracle/rights-target-crosswalk.json"
        ).read_text(encoding="utf-8")
    )["prefixes"]

    facts = [
        {
            "case": case_id,
            "kind": "status",
            "value": "ready",
        }
    ]
    role_identity = f"uuid:{role_uuid}"
    source_identity = "derived:sourceRoot:Source#1"

    def add_node(identity: str, strength: str, kind: str, name: str) -> None:
        facts.append(
            {
                "case": case_id,
                "kind": "node",
                "identity": identity,
                "identityStrength": strength,
                "name": name,
                "objectKind": kind,
            }
        )
        facts.append(
            {
                "case": case_id,
                "kind": "nodeCoverage",
                "identity": identity,
                "coverage": "complete",
                "resolution": "resolved",
            }
        )

    def add_facets(identity: str, members: dict[str, set[str]]) -> None:
        for facet, values in members.items():
            for member in values:
                facts.append(
                    {
                        "case": case_id,
                        "kind": "facetMember",
                        "identity": identity,
                        "facet": facet,
                        "member": member,
                    }
                )

    def add_relation(
        source: str,
        role_name_value: str,
        target: str,
        target_kind: str,
        target_name: str,
    ) -> None:
        facts.append(
            {
                "case": case_id,
                "kind": "relation",
                "source": source,
                "role": role_name_value,
                "target": target,
                "targetKind": target_kind,
                "targetName": target_name,
                "coverage": "complete",
                "resolution": "resolved",
            }
        )

    add_node(role_identity, "persistent", "role", role_name)
    facts.append(
        {
            "case": case_id,
            "kind": "property",
            "identity": role_identity,
            "property": "backing.content.available",
            "value": {"type": "boolean", "value": True},
            "valueState": "computed",
            "valueType": "boolean",
        }
    )
    role_facets = facet_members(
        "role",
        relation_roles={"accessPermissions", "restrictionTemplates"},
    )
    if any(
        local_name(node.tag) == "content"
        and node.text
        and node.text.strip()
        for node in direct_properties(role).iter()
    ):
        role_facets["identity"].add("metadata.synonym")
    add_facets(role_identity, role_facets)

    permission_index = 0
    external_counts: Counter[tuple[str, str]] = Counter()
    for native_object in [
        node for node in rights if local_name(node.tag) == "object"
    ]:
        target = child_text(native_object, "name")
        prefix, target_name = target.split(".", 1)
        if prefix not in prefix_map:
            raise ValueError(f"unreviewed rights target prefix {prefix}")
        target_kind = prefix_map[prefix]
        external_key = (target_kind, target_name)
        if external_key not in external_counts:
            external_counts[external_key] += 1
        external_identity = (
            f"external:{target_kind}:{target_name}#{external_counts[external_key]}"
        )
        for right in [
            node for node in native_object if local_name(node.tag) == "right"
        ]:
            permission_index += 1
            permission_name = child_text(right, "name")
            display_name = f"{target_name}:{permission_index}"
            identity = f"derived:accessPermission:{display_name}#1"
            conditions = [
                text
                for restriction in right
                if local_name(restriction.tag) == "restrictionByCondition"
                for condition in restriction.iter()
                if local_name(condition.tag) == "condition"
                for text in [(condition.text or "").strip()]
                if text
            ]
            add_node(identity, "derived", "accessPermission", display_name)
            if conditions:
                facts.append(
                    {
                        "case": case_id,
                        "kind": "property",
                        "identity": identity,
                        "property": "access.restriction.conditions",
                        "value": {
                            "type": "list",
                            "value": [
                                {"type": "string", "value": value}
                                for value in conditions
                            ],
                        },
                        "valueState": "explicit",
                        "valueType": "list",
                    }
                )
            add_facets(
                identity,
                facet_members(
                    "accessPermission",
                    has_conditions=bool(conditions),
                    relation_roles={"accessTarget"},
                ),
            )
            add_relation(
                role_identity,
                "accessPermissions",
                identity,
                "accessPermission",
                display_name,
            )
            add_relation(
                identity,
                "accessTarget",
                external_identity,
                target_kind,
                target_name,
            )

    for template in [
        node for node in rights if local_name(node.tag) == "restrictionTemplate"
    ]:
        name = child_text(template, "name")
        conditions = [
            (condition.text or "").strip()
            for condition in template.iter()
            if local_name(condition.tag) == "condition"
            and (condition.text or "").strip()
        ]
        identity = f"derived:accessRestrictionTemplate:{name}#1"
        add_node(identity, "derived", "accessRestrictionTemplate", name)
        facts.append(
            {
                "case": case_id,
                "kind": "property",
                "identity": identity,
                "property": "access.restriction.conditions",
                "value": {
                    "type": "list",
                    "value": [
                        {"type": "string", "value": value}
                        for value in conditions
                    ],
                },
                "valueState": "explicit",
                "valueType": "list",
            }
        )
        add_facets(
            identity,
            facet_members("accessRestrictionTemplate", has_conditions=True),
        )
        add_relation(
            role_identity,
            "restrictionTemplates",
            identity,
            "accessRestrictionTemplate",
            name,
        )

    add_node(source_identity, "derived", "sourceRoot", "Source")
    add_facets(source_identity, {"structure": {"children"}})
    add_relation(
        source_identity,
        "children",
        role_identity,
        "role",
        role_name,
    )
    return {
        "id": case_id,
        "input": descriptor_path.relative_to(repo_root).as_posix(),
        "sourceRoot": (
            repo_root
            / "crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights"
        )
        .relative_to(repo_root)
        .as_posix(),
        "facts": facts,
    }


def build_support_source_case(
    repo_root: Path,
    inputs: dict[str, Any],
    contract_id: str,
) -> dict[str, Any]:
    input_id = SUPPORT_INPUT_CASES[contract_id]
    input_case = next(case for case in inputs["cases"] if case["id"] == input_id)
    descriptor = ET.parse(repo_root / input_case["input"]).getroot()
    language = next(
        node for node in descriptor.iter() if local_name(node.tag) == "Language"
    )
    name = child_text(direct_properties(language), "Name")
    identity = f"uuid:{language.attrib['uuid']}"
    source_identity = "derived:sourceRoot:Source#1"
    facts = [
        {"case": contract_id, "kind": "status", "value": "ready"},
        {
            "case": contract_id,
            "kind": "node",
            "identity": identity,
            "identityStrength": "persistent",
            "name": name,
            "objectKind": "language",
        },
        {
            "case": contract_id,
            "kind": "nodeCoverage",
            "identity": identity,
            "coverage": "complete",
            "resolution": "resolved",
        },
        {
            "case": contract_id,
            "kind": "node",
            "identity": source_identity,
            "identityStrength": "derived",
            "name": "Source",
            "objectKind": "sourceRoot",
        },
        {
            "case": contract_id,
            "kind": "nodeCoverage",
            "identity": source_identity,
            "coverage": "complete",
            "resolution": "resolved",
        },
        {
            "case": contract_id,
            "kind": "relation",
            "source": source_identity,
            "role": "children",
            "target": identity,
            "targetKind": "language",
            "targetName": name,
            "coverage": "complete",
            "resolution": "resolved",
        },
    ]
    for node_identity, members in (
        (
            identity,
            {
                "identity": {"metadata.kind", "metadata.name", "metadata.uuid"},
                "support": {
                    "support.state",
                    "support.authorability",
                    "support.editCapability",
                },
            },
        ),
        (source_identity, {"structure": {"children"}}),
    ):
        for facet, values in members.items():
            for member in values:
                facts.append(
                    {
                        "case": contract_id,
                        "kind": "facetMember",
                        "identity": node_identity,
                        "facet": facet,
                        "member": member,
                    }
                )
    return {
        "id": contract_id,
        "input": input_case["input"],
        "sourceRoot": input_case["sourceRoot"],
        "facts": facts,
    }


def build_contract(
    repo_root: Path,
    inputs: dict[str, Any],
    source_contract: dict[str, Any],
) -> dict[str, Any]:
    if source_contract.get("schemaVersion") != 1:
        raise ValueError("new-only source contract schema is unsupported")
    source_cases = list(source_contract["cases"])
    source_cases.append(build_multitarget_source_case(repo_root))
    for contract_id in SUPPORT_PROFILES:
        source_cases.append(
            build_support_source_case(repo_root, inputs, contract_id)
        )
    cases = []
    for source_case in source_cases:
        support_profile = SUPPORT_PROFILES.get(
            source_case["id"],
            ("notSupported", "authorable", "authorable"),
        )
        cases.append(
            base_case_to_contract(repo_root, source_case, support_profile)
        )
    return {
        "schemaVersion": 2,
        "provenance": (
            "independent-v1-inventory-plus-native-fixtures-and-closed-core-contracts"
        ),
        "publicSchema": PUBLIC_SCHEMA,
        "cases": sorted(cases, key=lambda case: case["id"]),
    }
