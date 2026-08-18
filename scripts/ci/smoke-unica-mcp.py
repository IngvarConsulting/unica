#!/usr/bin/env python3
"""Run a packaged Unica binary through source and analyzer MCP flows."""

from __future__ import annotations

import argparse
import contextlib
import json
import os
import queue
import signal
import socket
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from typing import Callable


TOOL_SURFACE_REVIEW_RELATIVE = Path("docs/arch-v1/architecture/tool-surface-review.json")
CHECKOUT_MARKERS = (
    Path("Cargo.toml"),
    Path("plugins/unica/.codex-plugin/plugin.json"),
)
SOURCE_TOOL_NAMES = {
    "unica.source.resolve",
    "unica.source.children",
    "unica.source.resources",
    "unica.source.read",
}
META_TOOL_NAMES = {
    "unica.meta.info",
    "unica.meta.add",
    "unica.meta.edit",
    "unica.meta.remove",
}
ROLE_TYPED_TOOL_NAME = "unica.role.edit"
CODE_SEARCH_TYPED_TOOL_NAME = "unica.code.search"
CODE_SEARCH_PROVIDERS = ["rlm", "bsl-analyzer", "git-grep"]
DEFAULT_TOTAL_TIMEOUT_SECONDS = 120.0
UPSTREAM_SEARCH_FIELDS = {"root_id", "rootId", "roots", "cacheDir"}
EXPECTED_META_OUTPUT_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "properties": {
        "ok": {"type": "boolean"},
        "summary": {"type": "string"},
        "changes": {"type": "array", "items": {"type": "string"}},
        "warnings": {"type": "array", "items": {"type": "string"}},
        "errors": {"type": "array", "items": {"type": "string"}},
        "artifacts": {"type": "array", "items": {"type": "string"}},
        "cache": {
            "type": "object",
            "additionalProperties": False,
            "properties": {
                "mode": {"type": "string"},
                "root": {"type": "string"},
                "workspace_epoch": {"type": "integer", "minimum": 0},
                "events": {"type": "array", "items": {"type": "string"}},
                "invalidated": {"type": "array", "items": {"type": "string"}},
                "refreshed": {"type": "array", "items": {"type": "string"}},
                "lazy_rebuilt": {"type": "array", "items": {"type": "string"}},
                "stale": {"type": "array", "items": {"type": "string"}},
                "fresh": {"type": "array", "items": {"type": "string"}},
            },
            "required": [
                "mode",
                "root",
                "workspace_epoch",
                "events",
                "invalidated",
                "refreshed",
                "lazy_rebuilt",
                "stale",
                "fresh",
            ],
        },
        "stdout": {"type": "string"},
        "stderr": {"type": "string"},
        "command": {"type": "array", "items": {"type": "string"}},
        "diagnostics": {},
        "data": {},
        "job": {},
    },
    "required": [
        "ok",
        "summary",
        "changes",
        "warnings",
        "errors",
        "artifacts",
        "cache",
    ],
}


def _valid_unica_tool_name(value: object) -> bool:
    if not isinstance(value, str) or not 1 <= len(value) <= 128:
        return False
    parts = value.split(".")
    return (
        len(parts) >= 2
        and parts[0] == "unica"
        and all(
            part
            and all(
                character.isascii()
                and (character.isalnum() or character in {"_", "-"})
                for character in part
            )
            for part in parts[1:]
        )
    )


def _input_schema_shape_error(value: object) -> str | None:
    if not isinstance(value, dict):
        return "must be an object"
    if value.get("type") != "object":
        return "must declare type object"
    properties = value.get("properties")
    if not isinstance(properties, dict):
        return "must declare object properties"
    required = value.get("required")
    if not isinstance(required, list) or not all(
        isinstance(name, str) and name in properties for name in required
    ):
        return "must declare required as property names"
    if len(required) != len(set(required)):
        return "must not repeat required property names"
    if value.get("additionalProperties") is not False:
        return "must reject additional properties"
    return None


def _code_search_output_schema_shape_error(value: object) -> str | None:
    if not isinstance(value, dict) or value.get("type") != "object":
        return "must declare an object envelope"
    # ADR-0023: typed provider-neutral payload is carried by OperationResult.data.
    required = value.get("required")
    if not isinstance(required, list) or "data" not in required:
        return "must require data"
    properties = value.get("properties")
    if not isinstance(properties, dict):
        return "must declare envelope properties"
    data = properties.get("data")
    if not isinstance(data, dict) or data.get("type") != "object":
        return "must declare object data"
    data_properties = data.get("properties")
    required_data = {"coverage", "elapsedMs", "sections"}
    if not isinstance(data_properties, dict) or not required_data.issubset(data_properties):
        return "must declare coverage, elapsedMs, and sections"
    if set(data.get("required", [])) != required_data:
        return "must require coverage, elapsedMs, and sections"
    sections = data_properties["sections"]
    if (
        not isinstance(sections, dict)
        or sections.get("type") != "array"
        or sections.get("minItems") != 3
        or sections.get("maxItems") != 3
    ):
        return "must declare exactly three role sections"
    section = sections.get("items")
    section_fields = {
        "role",
        "provider",
        "status",
        "termination",
        "searchComplete",
        "ranking",
        "ordering",
        "matches",
        "hits",
        "diagnostics",
    }
    if not isinstance(section, dict) or section.get("type") != "object":
        return "must declare role-section objects"
    section_properties = section.get("properties")
    if not isinstance(section_properties, dict) or not section_fields.issubset(
        section_properties
    ):
        return "must declare the provider-neutral role-section fields"
    if not section_fields.issubset(set(section.get("required", []))):
        return "must require the provider-neutral role-section fields"
    termination = section_properties["termination"]
    termination_variants = (
        termination.get("oneOf") if isinstance(termination, dict) else None
    )
    if not isinstance(termination_variants, list) or len(termination_variants) != 2:
        return "must declare a nullable machine-readable termination reason"
    null_variant, terminal_variant = termination_variants
    terminal_properties = (
        terminal_variant.get("properties")
        if isinstance(terminal_variant, dict)
        else None
    )
    terminal_code = (
        terminal_properties.get("code")
        if isinstance(terminal_properties, dict)
        else None
    )
    retryable = (
        terminal_properties.get("retryable")
        if isinstance(terminal_properties, dict)
        else None
    )
    detail_code = (
        terminal_properties.get("detailCode")
        if isinstance(terminal_properties, dict)
        else None
    )
    expected_codes = {
        "limitReached",
        "deadlineExceeded",
        "dependencyPending",
        "unsupportedScope",
        "capacityExhausted",
        "providerUnavailable",
        "providerFailed",
    }
    if (
        null_variant != {"type": "null"}
        or not isinstance(terminal_variant, dict)
        or terminal_variant.get("type") != "object"
        or terminal_variant.get("additionalProperties") is not False
        or not isinstance(terminal_code, dict)
        or terminal_code.get("type") != "string"
        or set(terminal_code.get("enum", [])) != expected_codes
        or retryable != {"type": "boolean"}
        or not isinstance(detail_code, dict)
        or detail_code.get("type") != "string"
        or detail_code.get("minLength") != 1
        or set(terminal_variant.get("required", [])) != {"code", "retryable"}
    ):
        return "must close the machine-readable termination reason contract"
    matches = section_properties["matches"]
    if not isinstance(matches, dict) or matches.get("type") != "object":
        return "must declare matches as an object"
    match_properties = matches.get("properties")
    if not isinstance(match_properties, dict) or not {
        "returned",
        "total",
        "relation",
    }.issubset(match_properties):
        return "must declare returned, total, and relation counts"
    if not {"returned", "relation"}.issubset(set(matches.get("required", []))):
        return "must require returned and relation counts"
    return None


def _role_output_schema_shape_error(value: object) -> str | None:
    """Return the first structural error in the role-edit output schema."""

    if not isinstance(value, dict):
        return "must be an object"
    if value.get("type") != "object" or value.get("additionalProperties") is not False:
        return "must be a closed object"
    properties = value.get("properties")
    expected = {
        "ok",
        "summary",
        "changes",
        "warnings",
        "errors",
        "artifacts",
        "cache",
        "data",
    }
    if not isinstance(properties, dict) or set(properties) != expected:
        return "must expose only the typed role envelope"
    if set(value.get("required", [])) != expected:
        return "must require the complete typed role envelope"
    cache = properties.get("cache")
    if not isinstance(cache, dict) or cache.get("properties", {}).get("root") != {"const": ""}:
        return "must redact the cache root"
    data = properties.get("data")
    if not isinstance(data, dict) or data.get("additionalProperties") is not False:
        return "must publish closed typed data"
    data_properties = data.get("properties")
    data_expected = {
        "metadataPath",
        "changed",
        "effects",
        "validation",
        "diagnostics",
    }
    if not isinstance(data_properties, dict) or set(data_properties) != data_expected:
        return "must publish the logical role data fields"
    if set(data.get("required", [])) != data_expected:
        return "must require every logical role data field"
    return None


def tool_surface_review_path(plugin_root: Path) -> Path:
    """Resolve the canonical public-tool ledger from any checkout-local package.

    Release smoke runs with both source plugin roots and nested generated package
    roots. The nearest checkout marker pair bounds lookup so a missing ledger
    cannot be replaced by an unrelated file above the checkout.
    """

    resolved = plugin_root.resolve()
    checkout_root = next(
        (
            root
            for root in (resolved, *resolved.parents)
            if all((root / marker).is_file() for marker in CHECKOUT_MARKERS)
        ),
        None,
    )
    if checkout_root is None:
        raise SystemExit(
            "cannot resolve validated checkout root from plugin root: "
            f"{resolved}"
        )
    candidate = checkout_root / TOOL_SURFACE_REVIEW_RELATIVE
    if candidate.is_file():
        return candidate
    raise SystemExit(
        "cannot resolve canonical tool-surface-review.json within checkout root: "
        f"{checkout_root}"
    )


def expected_tool_names(plugin_root: Path) -> set[str]:
    path = tool_surface_review_path(plugin_root)
    try:
        review = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"cannot read public tool ledger {path}: {error}") from error
    if not isinstance(review, dict) or not review:
        raise SystemExit(f"public tool ledger must be a non-empty object: {path}")
    invalid = sorted(name for name in review if not _valid_unica_tool_name(name))
    if invalid:
        raise SystemExit(f"public tool ledger has invalid names: {invalid}")
    return set(review)


EXPECTED_SOURCE_INPUT_SCHEMAS = json.loads(
    r'''
{
  "unica.source.children": {
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cursor": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "type": "string"
      },
      "limit": {
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "metadataPath": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      }
    },
    "required": [
      "sourceSet"
    ],
    "type": "object"
  },
  "unica.source.read": {
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cwd": {
        "type": "string"
      },
      "limit": {
        "maximum": 65536,
        "minimum": 1,
        "type": "integer"
      },
      "offset": {
        "minimum": 0,
        "type": "integer"
      },
      "resourceId": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "snapshotId": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      }
    },
    "required": [
      "snapshotId",
      "resourceId"
    ],
    "type": "object"
  },
  "unica.source.resolve": {
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cursor": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "type": "string"
      },
      "limit": {
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "mode": {
        "enum": [
          "exact",
          "prefix"
        ],
        "type": "string"
      },
      "query": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "targetKind": {
        "enum": [
          "metadataObject",
          "module"
        ],
        "type": "string"
      }
    },
    "required": [
      "sourceSet",
      "query"
    ],
    "type": "object"
  },
  "unica.source.resources": {
    "additionalProperties": false,
    "oneOf": [
      {
        "not": {
          "anyOf": [
            {
              "required": [
                "snapshotId"
              ]
            },
            {
              "required": [
                "cursor"
              ]
            }
          ]
        },
        "required": [
          "sourceSet"
        ]
      },
      {
        "not": {
          "anyOf": [
            {
              "required": [
                "sourceSet"
              ]
            },
            {
              "required": [
                "metadataPath"
              ]
            },
            {
              "required": [
                "scope"
              ]
            }
          ]
        },
        "required": [
          "snapshotId",
          "cursor"
        ]
      }
    ],
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cursor": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "type": "string"
      },
      "limit": {
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "metadataPath": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "scope": {
        "enum": [
          "self",
          "aggregate",
          "registrations"
        ],
        "type": "string"
      },
      "snapshotId": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      }
    },
    "required": [],
    "type": "object"
  }
}
'''
)


EXPECTED_XDTO_INPUT_SCHEMAS = json.loads(
    r'''
{
  "unica.xdto.info": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cursor": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S+$"
      },
      "cwd": {
        "type": "string"
      },
      "limit": {
        "type": "integer",
        "minimum": 1,
        "maximum": 50
      },
      "metadataPath": {
        "type": "string",
        "pattern": "^(?:XDTOPackage|ПакетXDTO)\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*$"
      },
      "sourceSet": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S(?:.*\\S)?$"
      },
      "typeName": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
      }
    },
    "required": [
      "sourceSet",
      "metadataPath"
    ],
    "not": {
      "anyOf": [
        {
          "required": [
            "typeName",
            "limit"
          ]
        },
        {
          "required": [
            "typeName",
            "cursor"
          ]
        }
      ]
    }
  },
  "unica.xdto.edit": {
    "type": "object",
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "type": "boolean"
      },
      "cwd": {
        "type": "string"
      },
      "dryRun": {
        "type": "boolean",
        "default": true
      },
      "metadataPath": {
        "type": "string",
        "pattern": "^(?:XDTOPackage|ПакетXDTO)\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*$"
      },
      "operations": {
        "type": "array",
        "minItems": 1,
        "items": {
          "oneOf": [
            {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "op": {
                  "enum": [
                    "addValueType"
                  ]
                },
                "name": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                },
                "base": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*:[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                }
              },
              "required": [
                "op",
                "name",
                "base"
              ]
            },
            {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "op": {
                  "enum": [
                    "addObjectType"
                  ]
                },
                "name": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                }
              },
              "required": [
                "op",
                "name"
              ]
            },
            {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "op": {
                  "enum": [
                    "addProperty"
                  ]
                },
                "typeName": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                },
                "property": {
                  "type": "object",
                  "additionalProperties": false,
                  "properties": {
                    "name": {
                      "type": "string",
                      "minLength": 1,
                      "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                    },
                    "type": {
                      "type": "string",
                      "minLength": 1,
                      "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*:[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                    },
                    "minOccurs": {
                      "type": "integer",
                      "minimum": 0,
                      "maximum": 1
                    }
                  },
                  "required": [
                    "name",
                    "type"
                  ]
                },
                "propertyPath": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*(?:\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*)*$"
                }
              },
              "required": [
                "op",
                "typeName",
                "property"
              ]
            },
            {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "op": {
                  "enum": [
                    "removeType"
                  ]
                },
                "name": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                }
              },
              "required": [
                "op",
                "name"
              ]
            },
            {
              "type": "object",
              "additionalProperties": false,
              "properties": {
                "op": {
                  "enum": [
                    "removeProperty"
                  ]
                },
                "typeName": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                },
                "name": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$"
                },
                "propertyPath": {
                  "type": "string",
                  "minLength": 1,
                  "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*(?:\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*)*$"
                }
              },
              "required": [
                "op",
                "typeName",
                "name"
              ]
            }
          ]
        }
      },
      "sourceSet": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S(?:.*\\S)?$"
      }
    },
    "required": [
      "sourceSet",
      "metadataPath",
      "operations"
    ]
  }
}
'''
)


EXPECTED_SOURCE_FLOW_PROJECTIONS = json.loads(
    r'''
{
  "extension": {
    "children": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "children": [
          {
            "addressability": "addressable",
            "completeness": "complete",
            "displayName": "Module",
            "location": {
              "kind": "addressed",
              "metadataPath": "CommonModule.Shared.Module",
              "sourceSet": "extension",
              "targetKind": "module"
            },
            "metadataPath": "CommonModule.Shared.Module",
            "nodeKind": "item",
            "targetKind": "module"
          }
        ],
        "completeness": "complete"
      },
      "errors": [],
      "ok": true,
      "summary": "source.children returned 1 immediate child node(s)",
      "warnings": []
    },
    "read": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "appliedLimit": 65536,
        "content": "\ufeffProcedure RunExtension()\r\nEndProcedure\r\n",
        "contentEncoding": "utf-8",
        "eof": true,
        "hash": "sha256:41e8d685fd708f331f099494d36fe1a0059ae144da2de497c8dc3f5629c900ea",
        "length": 43,
        "offset": 0,
        "size": 43,
        "textProfile": {
          "bomPrefixBytes": 3,
          "encoding": "utf-8",
          "eol": "crlf"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.read returned 43 byte(s)",
      "warnings": []
    },
    "resolve": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "candidates": [
          {
            "displayName": "Module",
            "location": {
              "kind": "addressed",
              "metadataPath": "CommonModule.Shared.Module",
              "sourceSet": "extension",
              "targetKind": "module"
            },
            "matchKind": "exact",
            "metadataPath": "CommonModule.Shared.Module",
            "targetKind": "module"
          }
        ],
        "completeness": "complete"
      },
      "errors": [],
      "ok": true,
      "summary": "source.resolve returned 1 canonical candidate(s)",
      "warnings": []
    },
    "resources": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "completeness": "complete",
        "resources": [
          {
            "access": [
              "read"
            ],
            "hash": "sha256:41e8d685fd708f331f099494d36fe1a0059ae144da2de497c8dc3f5629c900ea",
            "limits": {
              "maxReadBytes": 65536
            },
            "mediaType": "text/x-bsl",
            "role": "bslModule",
            "size": 43,
            "textProfile": {
              "bomPrefixBytes": 3,
              "encoding": "utf-8",
              "eol": "crlf"
            }
          }
        ],
        "scope": "self",
        "sourceSet": "extension",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "extension",
          "targetKind": "module"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.resources returned 1 resource(s)",
      "warnings": []
    },
    "sourceSet": "extension"
  },
  "main": {
    "children": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "children": [
          {
            "addressability": "addressable",
            "completeness": "complete",
            "displayName": "Module",
            "location": {
              "kind": "addressed",
              "metadataPath": "CommonModule.Shared.Module",
              "sourceSet": "main",
              "targetKind": "module"
            },
            "metadataPath": "CommonModule.Shared.Module",
            "nodeKind": "item",
            "targetKind": "module"
          }
        ],
        "completeness": "complete"
      },
      "errors": [],
      "ok": true,
      "summary": "source.children returned 1 immediate child node(s)",
      "warnings": []
    },
    "read": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "appliedLimit": 65536,
        "content": "\ufeffProcedure Run()\r\nEndProcedure\r\n",
        "contentEncoding": "utf-8",
        "eof": true,
        "hash": "sha256:87c24a6da821b5f96a884b7210133a30d7ee2c66cf281934bae1afc8281a8cbb",
        "length": 34,
        "offset": 0,
        "size": 34,
        "textProfile": {
          "bomPrefixBytes": 3,
          "encoding": "utf-8",
          "eol": "crlf"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.read returned 34 byte(s)",
      "warnings": []
    },
    "resolve": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "candidates": [
          {
            "displayName": "Module",
            "location": {
              "kind": "addressed",
              "metadataPath": "CommonModule.Shared.Module",
              "sourceSet": "main",
              "targetKind": "module"
            },
            "matchKind": "exact",
            "metadataPath": "CommonModule.Shared.Module",
            "targetKind": "module"
          }
        ],
        "completeness": "complete"
      },
      "errors": [],
      "ok": true,
      "summary": "source.resolve returned 1 canonical candidate(s)",
      "warnings": []
    },
    "resources": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "completeness": "complete",
        "resources": [
          {
            "access": [
              "read"
            ],
            "hash": "sha256:87c24a6da821b5f96a884b7210133a30d7ee2c66cf281934bae1afc8281a8cbb",
            "limits": {
              "maxReadBytes": 65536
            },
            "mediaType": "text/x-bsl",
            "role": "bslModule",
            "size": 34,
            "textProfile": {
              "bomPrefixBytes": 3,
              "encoding": "utf-8",
              "eol": "crlf"
            }
          }
        ],
        "scope": "self",
        "sourceSet": "main",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "main",
          "targetKind": "module"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.resources returned 1 resource(s)",
      "warnings": []
    },
    "sourceSet": "main"
  }
}
'''
)


def _source_workspace(root: Path) -> None:
    (root / "src/CommonModules/Shared/Ext").mkdir(parents=True)
    (root / "ext/CommonModules/Shared/Ext").mkdir(parents=True)
    (root / "v8project.yaml").write_text(
        "format: DESIGNER\nsource-set:\n"
        "  - name: main\n    type: CONFIGURATION\n    path: src\n"
        "  - name: extension\n    type: EXTENSION\n    path: ext\n",
        encoding="utf-8",
    )
    for source_set, name, module in [
        ("src", "Main", "Run"),
        ("ext", "Extension", "RunExtension"),
    ]:
        (root / source_set / "Configuration.xml").write_text(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
            "<Configuration><Properties><Name>"
            + name
            + "</Name>"
            + (
                "<ConfigurationExtensionPurpose>Customization</ConfigurationExtensionPurpose>"
                if source_set == "ext"
                else ""
            )
            + "</Properties><ChildObjects><CommonModule>Shared</CommonModule>"
            "</ChildObjects></Configuration></MetaDataObject>",
            encoding="utf-8",
        )
        (root / source_set / "CommonModules/Shared.xml").write_text(
            "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
            "<CommonModule><Properties><Name>Shared</Name></Properties></CommonModule>"
            "</MetaDataObject>",
            encoding="utf-8",
        )
        (root / source_set / "CommonModules/Shared/Ext/Module.bsl").write_bytes(
            ("\ufeffProcedure " + module + "()\r\nEndProcedure\r\n").encode("utf-8")
        )
    # ADR-0049: a subject reader must be reachable by address end to end, so
    # the smoke workspace carries one registered object with an attached body.
    (root / "src/Roles/SmokeRole/Ext").mkdir(parents=True)
    (root / "src/Roles/SmokeRole.xml").write_text(
        "<MetaDataObject xmlns=\"http://v8.1c.ru/8.3/MDClasses\" version=\"2.20\">"
        "<Role><Properties><Name>SmokeRole</Name></Properties></Role></MetaDataObject>",
        encoding="utf-8",
    )
    (root / "src/Roles/SmokeRole/Ext/Rights.xml").write_text(
        "<Rights xmlns=\"http://v8.1c.ru/8.2/roles\" setForNewObjects=\"false\" "
        "setForAttributesByDefault=\"true\" independentRightsOfChildObjects=\"false\"/>",
        encoding="utf-8",
    )
    meta_fixture = (
        Path(__file__).resolve().parents[2]
        / "tests/fixtures/unica_mcp_script_parity/meta-validate-language-aware"
    )
    for fixture_path in meta_fixture.rglob("*"):
        if fixture_path.is_file():
            target = root / "src" / fixture_path.relative_to(meta_fixture)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(fixture_path.read_bytes())
    configuration = root / "src/Configuration.xml"
    original = configuration.read_text(encoding="utf-8")
    registered = original.replace(
        "\t\t</ChildObjects>",
        "\t\t\t<CommonModule>Shared</CommonModule>\n"
        "\t\t\t<Role>SmokeRole</Role>\n\t\t</ChildObjects>",
    )
    if registered == original:
        raise SystemExit(
            "meta fixture Configuration.xml has no ChildObjects anchor to register Shared"
        )
    configuration.write_text(registered, encoding="utf-8")


def _assert_path_free(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in {
                "path",
                "sourceDir",
                "provider",
                "providerId",
                "providerRevision",
                "handle",
            }:
                raise SystemExit(f"Unica MCP source contract leaks {key}")
            _assert_path_free(child)
    elif isinstance(value, list):
        for child in value:
            _assert_path_free(child)


def canonical_source_projection(value: object, cache_root: Path) -> object:
    """Keep every stable field while normalizing declared process-local values."""

    if isinstance(value, dict):
        projected = {}
        for key, child in value.items():
            if key in {"snapshotId", "resourceId"}:
                continue
            if key == "root":
                if not isinstance(child, str) or Path(child).resolve() != cache_root.resolve():
                    raise SystemExit(f"Unica MCP cache root drifted: {child!r}")
                projected[key] = "<cache-root>"
            elif key == "workspace_epoch":
                if not isinstance(child, int):
                    raise SystemExit(f"Unica MCP workspace epoch is not an integer: {child!r}")
                projected[key] = "<workspace-epoch>"
            else:
                projected[key] = canonical_source_projection(child, cache_root)
        return projected
    if isinstance(value, list):
        return [canonical_source_projection(child, cache_root) for child in value]
    return value


def _workspace_snapshot(root: Path) -> dict[str, bytes]:
    return {
        path.relative_to(root).as_posix(): path.read_bytes()
        for path in root.rglob("*")
        if path.is_file()
    }


def source_flow_projection(
    source_set: str,
    cache_root: Path,
    resolve: dict,
    children: dict,
    resources: dict,
    read: dict,
) -> dict:
    return {
        "sourceSet": source_set,
        "resolve": canonical_source_projection(resolve, cache_root),
        "children": canonical_source_projection(children, cache_root),
        "resources": canonical_source_projection(resources, cache_root),
        "read": canonical_source_projection(read, cache_root),
    }


def expected_source_flow_projection(source_set: str) -> dict:
    return EXPECTED_SOURCE_FLOW_PROJECTIONS[source_set]


class McpSession:
    def __init__(
        self,
        command: list[str],
        environment: dict[str, str],
        timeout_seconds: float,
        *,
        cwd: Path,
        deadline: float | None = None,
    ) -> None:
        popen_options = {}
        if os.name == "posix":
            popen_options["start_new_session"] = True
        elif os.name == "nt" and hasattr(subprocess, "CREATE_NEW_PROCESS_GROUP"):
            popen_options["creationflags"] = subprocess.CREATE_NEW_PROCESS_GROUP
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=environment,
            cwd=cwd,
            **popen_options,
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        self.timeout_seconds = timeout_seconds
        self.deadline = deadline
        self.lines: queue.Queue[str] = queue.Queue()
        self.diagnostics: list[str] = []
        self.reader = threading.Thread(target=self._read_stdout, daemon=True)
        self.error_reader = threading.Thread(target=self._read_stderr, daemon=True)
        started_readers: list[threading.Thread] = []
        try:
            self.reader.start()
            started_readers.append(self.reader)
            # The server stays open across the whole flow and writes diagnostics to
            # stderr, so an undrained pipe buffer would block it mid-request and
            # surface as a bare timeout.
            self.error_reader.start()
            started_readers.append(self.error_reader)
        except BaseException:
            # The constructor has not returned, so neither the admission owner
            # nor the watchdog can see this process yet. Reap it here before
            # exposing the original thread-start failure.
            _terminate_unregistered_process_tree(self.process)
            for reader in started_readers:
                reader.join(timeout=5)
            for stream in (
                self.process.stdin,
                self.process.stdout,
                self.process.stderr,
            ):
                if stream is not None and not stream.closed:
                    stream.close()
            raise

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put("")

    def _read_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            self.diagnostics.append(line)

    def _detail(self) -> str:
        return "".join(self.diagnostics).strip() or "no process output"

    def terminate_tree(
        self,
        cache_root: Path,
        known_service_pids: set[int] | None = None,
    ) -> None:
        service_pids = set(known_service_pids or ())
        service_pids.update(_workspace_service_pids(cache_root))
        owned_pids: set[int] = set()
        try:
            if os.name == "posix":
                # The public Unica process starts a new session. Detached
                # workspace services create their own process groups inside
                # that session, so killing only the public process group would
                # leak those services and their provider descendants.
                owned_pids = _posix_owned_process_pids(
                    self.process.pid,
                    service_pids,
                    public_running=self.process.poll() is None,
                )
                _signal_processes(owned_pids, signal.SIGTERM)
            elif os.name == "nt":
                # The public process may already have exited while a detached
                # service keeps inherited pipe handles open. Kill recorded
                # service trees independently before targeting the parent PID.
                for pid in sorted(service_pids):
                    _taskkill_process_tree(pid)
                _taskkill_process_tree(self.process.pid)
            else:
                if self.process.poll() is None:
                    self.process.terminate()
        except (OSError, ProcessLookupError, subprocess.TimeoutExpired):
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        if os.name == "posix":
            survivors = {
                pid
                for pid in owned_pids
                if _process_is_running(pid)
            }
            _signal_processes(survivors, signal.SIGKILL)
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
            _wait_for_process_pids(owned_pids, 5)
            return
        try:
            if self.process.poll() is None:
                self.process.kill()
        except (OSError, ProcessLookupError, subprocess.TimeoutExpired):
            pass
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        _wait_for_process_pids(service_pids, 5)

    def request(self, message: dict) -> dict:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        deadline = min(
            time.monotonic() + self.timeout_seconds,
            getattr(self, "deadline", None) or float("inf"),
        )
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise SystemExit(
                    f"Unica MCP smoke timed out after {self.timeout_seconds:g}s: {self._detail()}"
                )
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty as error:
                raise SystemExit(
                    f"Unica MCP smoke timed out after {self.timeout_seconds:g}s: {self._detail()}"
                ) from error
            if not line:
                raise SystemExit(
                    f"Unica MCP exited before the expected response: {self._detail()}"
                )
            try:
                response = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(f"Unica MCP emitted invalid JSON: {error}: {line}") from error
            if response.get("id") == message.get("id"):
                return response

    def notify(self, message: dict) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def close(self) -> None:
        if self.process.stdin is not None and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            result = self.process.wait(timeout=self._remaining_timeout())
        except subprocess.TimeoutExpired as error:
            self.process.kill()
            raise SystemExit(
                f"Unica MCP smoke timed out after {self.timeout_seconds:g}s: {self._detail()}"
            ) from error
        self.reader.join(timeout=self._remaining_timeout())
        self.error_reader.join(timeout=self._remaining_timeout())
        if self.reader.is_alive() or self.error_reader.is_alive():
            raise SystemExit(
                "Unica MCP reader threads did not stop before the aggregate deadline: "
                f"{self._detail()}"
            )
        detail = self._detail()
        for stream in (self.process.stdout, self.process.stderr):
            if stream is not None and not stream.closed:
                stream.close()
        if result != 0:
            raise SystemExit(f"Unica MCP exited with {result}: {detail}")

    def _remaining_timeout(self) -> float:
        deadline = getattr(self, "deadline", None)
        if deadline is None:
            return self.timeout_seconds
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SystemExit(
                "Unica MCP smoke exceeded its aggregate deadline: "
                f"{self._detail()}"
            )
        return min(self.timeout_seconds, remaining)


def _workspace_service_pids(cache_root: Path) -> set[int]:
    pids: set[int] = set()
    for record_path in (cache_root / "services").glob("*/service.json"):
        try:
            record = json.loads(record_path.read_text(encoding="utf-8"))
        except (FileNotFoundError, OSError, json.JSONDecodeError):
            continue
        pid = record.get("pid") if isinstance(record, dict) else None
        if (
            not isinstance(pid, bool)
            and isinstance(pid, int)
            and 1 <= pid <= 0xFFFFFFFF
        ):
            pids.add(pid)
    return pids


def _terminate_unregistered_process_tree(process: subprocess.Popen[str]) -> None:
    if os.name == "posix":
        owned_pids = _posix_owned_process_pids(
            process.pid,
            set(),
            public_running=process.poll() is None,
        )
        _signal_processes(owned_pids, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        survivors = {pid for pid in owned_pids if _process_is_running(pid)}
        _signal_processes(survivors, signal.SIGKILL)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        _wait_for_process_pids(owned_pids, 5)
        return
    if os.name == "nt":
        _taskkill_process_tree(process.pid)
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        try:
            process.kill()
        except OSError:
            pass
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass


def _taskkill_process_tree(pid: int) -> None:
    try:
        subprocess.run(
            ["taskkill", "/PID", str(pid), "/T", "/F"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.TimeoutExpired):
        pass


def _signal_processes(pids: set[int], signal_number: int) -> None:
    for pid in sorted(pids, reverse=True):
        try:
            os.kill(pid, signal_number)
        except OSError:
            pass


def _posix_owned_process_pids(
    public_pid: int,
    service_pids: set[int],
    *,
    public_running: bool,
) -> set[int]:
    try:
        snapshot = subprocess.run(
            ["ps", "-axo", "pid=,ppid="],
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=5,
            check=True,
        ).stdout
    except (OSError, subprocess.SubprocessError):
        roots = set(service_pids)
        if public_running:
            roots.add(public_pid)
        return roots
    processes: dict[int, int] = {}
    for line in snapshot.splitlines():
        fields = line.split()
        if len(fields) != 2:
            continue
        try:
            pid, parent_pid = map(int, fields)
        except ValueError:
            continue
        processes[pid] = parent_pid
    owned = {pid for pid in service_pids if pid in processes}
    if public_running and public_pid in processes:
        owned.add(public_pid)
    while True:
        descendants = {
            pid
            for pid, parent_pid in processes.items()
            if parent_pid in owned
        }
        expanded = owned | descendants
        if expanded == owned:
            return owned
        owned = expanded


def _tool_payload(response: dict) -> dict:
    if "error" in response:
        raise SystemExit(f"Unica MCP tools/call failed: {response['error']}")
    result = response.get("result")
    if not isinstance(result, dict) or "structuredContent" in result:
        raise SystemExit(
            f"non-Meta Unica MCP call unexpectedly changed wire representation: {response}"
        )
    try:
        payload = json.loads(result["content"][0]["text"])
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"Unica MCP tools/call has no JSON payload: {response}") from error
    if not payload.get("ok"):
        raise SystemExit(f"Unica MCP tools/call rejected source flow: {payload}")
    _assert_path_free(payload)
    return payload


def _meta_payload(response: dict, *, expected_ok: bool) -> dict:
    if "error" in response:
        raise SystemExit(f"Unica MCP Meta call failed as JSON-RPC: {response['error']}")
    try:
        result = response["result"]
        structured = result["structuredContent"]
        text = json.loads(result["content"][0]["text"])
        is_error = result["isError"]
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(
            f"Unica MCP Meta call has no structured result: {response}"
        ) from error
    if structured != text:
        raise SystemExit("Unica MCP Meta text and structuredContent diverged")
    if structured.get("ok") is not expected_ok or is_error is not (not expected_ok):
        raise SystemExit(f"Unica MCP Meta call has inconsistent success state: {response}")
    return structured


def _assert_no_upstream_search_fields(value: object) -> None:
    if isinstance(value, dict):
        for key, child in value.items():
            if key in UPSTREAM_SEARCH_FIELDS:
                raise SystemExit(
                    f"Unica MCP code search leaks upstream field {key}"
                )
            _assert_no_upstream_search_fields(child)
    elif isinstance(value, list):
        for child in value:
            _assert_no_upstream_search_fields(child)


def _code_search_payload(response: dict) -> dict:
    if "error" in response:
        raise SystemExit(f"Unica MCP code search failed as JSON-RPC: {response['error']}")
    result = response.get("result")
    try:
        payload = json.loads(result["content"][0]["text"])
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(
            f"Unica MCP code search has no JSON operation payload: {response}"
        ) from error
    if not isinstance(payload, dict):
        raise SystemExit(f"Unica MCP code search payload is not an object: {payload!r}")
    ok = payload.get("ok")
    is_error = result.get("isError")
    if (
        not isinstance(ok, bool)
        or not isinstance(is_error, bool)
        or is_error is not (not ok)
    ):
        raise SystemExit(
            f"Unica MCP code search has inconsistent success state: {response}"
        )
    _assert_no_upstream_search_fields(payload)
    return payload


def _bsl_search_is_ready(payload: dict) -> bool:
    try:
        sections = payload["data"]["sections"]
    except (KeyError, TypeError) as error:
        raise SystemExit(
            f"Unica MCP code search has no provider sections: {payload}"
        ) from error
    if not isinstance(sections, list):
        raise SystemExit(f"Unica MCP code search sections are not a list: {payload}")
    providers = [
        section.get("provider") if isinstance(section, dict) else None
        for section in sections
    ]
    if providers != CODE_SEARCH_PROVIDERS or len(set(providers)) != len(providers):
        raise SystemExit(
            "Unica MCP code search provider sections differ from "
            f"{CODE_SEARCH_PROVIDERS}: {providers}"
        )
    bsl = sections[1]
    status = bsl.get("status")
    # A still-building index is a typed fact: `timedOut` with a retryable
    # `dependencyPending` termination. The prose in the diagnostics is for
    # people; deciding retry from it is what previously let a permanent
    # `unavailable` masquerade as a wait.
    if status == "timedOut":
        termination = bsl.get("termination")
        code = termination.get("code") if isinstance(termination, dict) else None
        if code == "dependencyPending" and termination.get("retryable") is True:
            return False
        raise SystemExit(
            f"Unica MCP bsl-analyzer timed out without a retryable dependency: {bsl}"
        )
    if status == "unavailable":
        raise SystemExit(f"Unica MCP bsl-analyzer is unavailable: {bsl}")
    if status != "ok":
        raise SystemExit(
            f"Unica MCP bsl-analyzer section must be ok, got {status!r}: {bsl}"
        )
    hits = bsl.get("hits")
    if not isinstance(hits, list) or not any(
        "Run" in json.dumps(hit, ensure_ascii=False) for hit in hits
    ):
        raise SystemExit(
            f"Unica MCP bsl-analyzer did not find fixture symbol Run: {bsl}"
        )
    return True


def _exercise_bsl_search(
    session: McpSession,
    request_id: int,
    timeout_seconds: float,
    smoke_deadline: float | None = None,
) -> int:
    deadline = time.monotonic() + max(10.0, timeout_seconds * 3)
    if smoke_deadline is not None:
        deadline = min(deadline, smoke_deadline)
    while True:
        payload = _code_search_payload(
            session.request(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": "tools/call",
                    "params": {
                        "name": "unica.code.search",
                        "arguments": {
                            "sourceDir": "src",
                            "query": "Run",
                            "limit": 10,
                        },
                    },
                }
            )
        )
        request_id += 1
        if _bsl_search_is_ready(payload):
            if payload.get("ok") is not True:
                raise SystemExit(
                    "Unica MCP code search has inconsistent success state: "
                    f"{payload}"
                )
            return request_id
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SystemExit(
                "Unica MCP bsl-analyzer stayed not_ready until the smoke deadline"
            )
        time.sleep(min(0.5, remaining))


def _process_is_running(pid: int) -> bool:
    if os.name == "nt":
        import ctypes

        synchronize = 0x00100000
        wait_timeout = 0x00000102
        kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        open_process = kernel32.OpenProcess
        open_process.argtypes = [ctypes.c_uint32, ctypes.c_int, ctypes.c_uint32]
        open_process.restype = ctypes.c_void_p
        wait_for_single_object = kernel32.WaitForSingleObject
        wait_for_single_object.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        wait_for_single_object.restype = ctypes.c_uint32
        close_handle = kernel32.CloseHandle
        close_handle.argtypes = [ctypes.c_void_p]
        close_handle.restype = ctypes.c_int
        handle = open_process(synchronize, False, pid)
        if not handle:
            # Access denied still proves that a process owns the PID. Other
            # failures mean there is no process left for this smoke to await.
            return ctypes.get_last_error() == 5
        try:
            return wait_for_single_object(handle, 0) == wait_timeout
        finally:
            close_handle(handle)

    try:
        completed, _ = os.waitpid(pid, os.WNOHANG)
        if completed == pid:
            return False
    except ChildProcessError:
        pass
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _wait_for_process_pids(pids: set[int], timeout_seconds: float) -> None:
    deadline = time.monotonic() + timeout_seconds
    while any(_process_is_running(pid) for pid in pids):
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            return
        time.sleep(min(0.05, remaining))


def _capture_posix_owned_process_pids(
    session: McpSession, service_pids: set[int]
) -> set[int]:
    if os.name != "posix":
        return set()
    process = getattr(session, "process", None)
    if process is None:
        return set()
    return _posix_owned_process_pids(
        process.pid,
        service_pids,
        public_running=process.poll() is None,
    )


def _quiesce_posix_owned_process_pids(
    owned_pids: set[int], timeout_seconds: float
) -> None:
    if not owned_pids:
        return
    wait_limit = min(1.0, timeout_seconds)
    _wait_for_process_pids(owned_pids, wait_limit)
    survivors = {pid for pid in owned_pids if _process_is_running(pid)}
    if not survivors:
        return
    _signal_processes(survivors, signal.SIGTERM)
    _wait_for_process_pids(survivors, wait_limit)
    survivors = {pid for pid in survivors if _process_is_running(pid)}
    if survivors:
        _signal_processes(survivors, signal.SIGKILL)
        _wait_for_process_pids(survivors, wait_limit)
    survivors = {pid for pid in survivors if _process_is_running(pid)}
    if survivors:
        rendered = ", ".join(str(pid) for pid in sorted(survivors))
        raise SystemExit(
            "Unica MCP owned provider processes survived smoke cleanup: "
            f"{rendered}"
        )


def _wait_for_workspace_services(
    cache_root: Path,
    timeout_seconds: float,
    service_pids: set[int] | None = None,
) -> None:
    deadline = time.monotonic() + timeout_seconds
    services_root = cache_root / "services"
    service_pids = service_pids or set()
    while True:
        records_remain = any(services_root.glob("*/service.json"))
        running_pids = {pid for pid in service_pids if _process_is_running(pid)}
        if not records_remain and not running_pids:
            return
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SystemExit(
                "Unica MCP workspace service did not exit before smoke cleanup"
            )
        time.sleep(min(0.05, remaining))


def _shutdown_workspace_services(
    cache_root: Path, timeout_seconds: float
) -> set[int]:
    deadline = time.monotonic() + timeout_seconds
    services_root = cache_root / "services"
    service_pids: set[int] = set()
    for record_path in sorted(services_root.glob("*/service.json")):
        try:
            record = json.loads(record_path.read_text(encoding="utf-8"))
        except FileNotFoundError:
            continue
        except (OSError, json.JSONDecodeError) as error:
            raise SystemExit(
                f"Unica MCP workspace service record is unreadable: {record_path}"
            ) from error
        if not isinstance(record, dict):
            raise SystemExit(
                f"Unica MCP workspace service record is not an object: {record_path}"
            )
        port = record.get("port")
        pid = record.get("pid")
        token = record.get("token")
        if (
            isinstance(pid, bool)
            or not isinstance(pid, int)
            or not 1 <= pid <= 0xFFFFFFFF
            or isinstance(port, bool)
            or not isinstance(port, int)
            or not 1 <= port <= 65535
            or not isinstance(token, str)
            or not token
        ):
            raise SystemExit(
                "Unica MCP workspace service record has no valid pid, port, and token: "
                f"{record_path}"
            )
        service_pids.add(pid)
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise SystemExit(
                "Unica MCP workspace service shutdown exceeded the smoke deadline"
            )
        connection_timeout = min(2.0, remaining)
        request = {
            "token": token,
            "kind": {"type": "shutdown"},
        }
        try:
            with socket.create_connection(
                ("127.0.0.1", port), timeout=connection_timeout
            ) as connection:
                connection.settimeout(connection_timeout)
                connection.sendall(
                    (
                        json.dumps(request, separators=(",", ":")) + "\n"
                    ).encode("utf-8")
                )
                with connection.makefile("rb") as response_stream:
                    response_line = response_stream.readline(65537)
        except OSError as error:
            if not record_path.exists():
                continue
            raise SystemExit(
                f"Unica MCP workspace service rejected shutdown: {record_path}"
            ) from error
        if len(response_line) > 65536 or not response_line.endswith(b"\n"):
            raise SystemExit(
                f"Unica MCP workspace service returned an invalid shutdown frame: {record_path}"
            )
        try:
            response = json.loads(response_line)
        except json.JSONDecodeError as error:
            raise SystemExit(
                f"Unica MCP workspace service returned invalid shutdown JSON: {record_path}"
            ) from error
        if (
            not isinstance(response, dict)
            or response.get("ok") is not True
            or response.get("shutdown") is not True
        ):
            raise SystemExit(
                f"Unica MCP workspace service did not confirm shutdown: {response!r}"
            )
    return service_pids


def _close_session_and_workspace_services(
    session: McpSession,
    cache_root: Path,
    timeout_seconds: float,
    deadline: float | None = None,
) -> None:
    # Capture PID roots before authenticated shutdown: a failed shutdown or
    # TemporaryDirectory cleanup may remove the record that makes an orphan
    # reachable for emergency cleanup.
    service_pids = _workspace_service_pids(cache_root)
    owned_pids = _capture_posix_owned_process_pids(session, service_pids)
    try:
        try:
            # On Windows, a detached workspace service may inherit extra copies of
            # the MCP process' pipe handles. Closing the MCP session first then
            # waits for reader EOF while the service that owns those copies is
            # still alive, so cleanup never reaches its shutdown call.
            # Ask the authenticated services to stop before waiting for MCP EOF.
            service_pids.update(
                _shutdown_workspace_services(
                    cache_root,
                    _remaining_smoke_timeout(deadline, timeout_seconds),
                )
            )
        finally:
            try:
                session.close()
            finally:
                _wait_for_workspace_services(
                    cache_root,
                    _remaining_smoke_timeout(deadline, timeout_seconds),
                    service_pids,
                )
                _quiesce_posix_owned_process_pids(
                    owned_pids,
                    _remaining_smoke_timeout(deadline, timeout_seconds),
                )
    except BaseException:
        session.terminate_tree(cache_root, service_pids)
        raise


def _remaining_smoke_timeout(deadline: float | None, cap: float) -> float:
    if deadline is None:
        return cap
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise SystemExit("Unica MCP smoke exceeded its aggregate deadline")
    return min(cap, remaining)


def _call(
    session: McpSession,
    request_id: int,
    name: str,
    arguments: dict,
    *,
    expect_ok: bool = True,
) -> dict:
    response = session.request(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }
    )
    if expect_ok:
        return _tool_payload(response)
    # A refusal is the thing under test, so its payload is returned as-is; the
    # caller asserts the stable code rather than the shape of a success.
    if "error" in response:
        return response
    result = response.get("result")
    if not isinstance(result, dict):
        raise SystemExit(f"Unica MCP tools/call has no result: {response}")
    try:
        payload = json.loads(result["content"][0]["text"])
    except (KeyError, IndexError, TypeError, json.JSONDecodeError):
        return result
    if payload.get("ok"):
        raise SystemExit(f"Unica MCP tools/call unexpectedly succeeded: {payload}")
    return payload


def _stable_tool_contract(tools: list[object], expected_names: set[str]) -> None:
    unledgered_source_tools = sorted(SOURCE_TOOL_NAMES - expected_names)
    if unledgered_source_tools:
        raise SystemExit(
            "source tools are absent from tool-surface-review.json: "
            + ", ".join(unledgered_source_tools)
        )
    unledgered_xdto_tools = sorted(
        set(EXPECTED_XDTO_INPUT_SCHEMAS) - expected_names
    )
    if unledgered_xdto_tools:
        raise SystemExit(
            "XDTO tools are absent from tool-surface-review.json: "
            + ", ".join(unledgered_xdto_tools)
        )
    by_name = {}
    name_counts = {}
    malformed_count = 0
    for tool in tools:
        if not isinstance(tool, dict) or not _valid_unica_tool_name(tool.get("name")):
            malformed_count += 1
            continue
        name = tool["name"]
        schema_error = _input_schema_shape_error(tool.get("inputSchema"))
        if schema_error is not None:
            raise SystemExit(
                f"Unica MCP tools/list has malformed input schema for {name}: "
                f"{schema_error}"
            )
        by_name[name] = tool
        name_counts[name] = name_counts.get(name, 0) + 1
    actual_names = set(by_name)
    actual_meta_names = {
        name for name in actual_names if name.startswith("unica.meta.")
    }
    if actual_meta_names != META_TOOL_NAMES:
        raise SystemExit(
            "Unica MCP Meta surface differs from INV-MCP-META-SURFACE "
            f"(expected: {sorted(META_TOOL_NAMES)}; actual: {sorted(actual_meta_names)})"
        )
    missing = sorted(expected_names - actual_names)
    unexpected = sorted(actual_names - expected_names)
    duplicates = sorted(name for name, count in name_counts.items() if count > 1)
    if missing or unexpected or duplicates or malformed_count:
        diagnostics = []
        if missing:
            diagnostics.append("missing: " + ", ".join(missing))
        if unexpected:
            diagnostics.append("unexpected: " + ", ".join(unexpected))
        if duplicates:
            diagnostics.append("duplicate: " + ", ".join(duplicates))
        if malformed_count:
            diagnostics.append(f"malformed entries: {malformed_count}")
        raise SystemExit(
            "Unica MCP tools/list differs from tool-surface-review.json ("
            + "; ".join(diagnostics)
            + ")"
        )
    projected = {}
    for name in sorted(SOURCE_TOOL_NAMES):
        schema = by_name[name]["inputSchema"]
        _assert_path_free(schema)
        projected[name] = schema
    if projected != EXPECTED_SOURCE_INPUT_SCHEMAS:
        raise SystemExit("Unica MCP source input schema projection drifted")
    xdto_projected = {}
    for name in sorted(EXPECTED_XDTO_INPUT_SCHEMAS):
        schema = by_name[name]["inputSchema"]
        _assert_path_free(schema)
        xdto_projected[name] = schema
    if xdto_projected != EXPECTED_XDTO_INPUT_SCHEMAS:
        raise SystemExit("Unica MCP XDTO input schema projection drifted")
    for name, tool in by_name.items():
        if name in META_TOOL_NAMES:
            if tool.get("outputSchema") != EXPECTED_META_OUTPUT_SCHEMA:
                raise SystemExit(f"Unica MCP Meta output schema drifted for {name}")
        elif name == ROLE_TYPED_TOOL_NAME:
            # Source-flow unit fixtures intentionally project only the input
            # surface. A real advertised role schema, when present, is still
            # checked here; the generated ledger separately requires it.
            if "outputSchema" in tool:
                error = _role_output_schema_shape_error(tool["outputSchema"])
                if error is not None:
                    raise SystemExit(f"Unica MCP role.edit output schema {error}")
        elif name == CODE_SEARCH_TYPED_TOOL_NAME:
            if "outputSchema" in tool:
                error = _code_search_output_schema_shape_error(tool["outputSchema"])
                if error is not None:
                    raise SystemExit(f"Unica MCP code.search output schema {error}")
        elif "outputSchema" in tool:
            raise SystemExit(f"non-Meta tool unexpectedly publishes outputSchema: {name}")


def _exercise_source_set(
    session: McpSession,
    request_id: int,
    workspace: Path,
    cache_root: Path,
    source_set: str,
) -> tuple[int, dict]:
    target = "CommonModule.Shared.Module"
    before_flow = _workspace_snapshot(workspace)
    resolve = _call(session, request_id, "unica.source.resolve", {
        "cwd": str(workspace), "sourceSet": source_set, "query": target, "mode": "exact", "targetKind": "module",
    })
    request_id += 1
    candidates = resolve["data"]["candidates"]
    if [candidate.get("metadataPath") for candidate in candidates] != [target]:
        raise SystemExit(f"source.resolve did not return the canonical module: {resolve}")
    children = _call(session, request_id, "unica.source.children", {
        "cwd": str(workspace), "sourceSet": source_set, "metadataPath": "CommonModule.Shared",
    })
    request_id += 1
    if target not in [child.get("metadataPath") for child in children["data"]["children"]]:
        raise SystemExit(f"source.children did not return the module child: {children}")
    resources = _call(session, request_id, "unica.source.resources", {
        "cwd": str(workspace), "sourceSet": source_set, "metadataPath": target, "scope": "self",
    })
    request_id += 1
    if resources["cache"]["events"] or resources["cache"]["invalidated"]:
        raise SystemExit(f"source.resources must be read-only: {resources}")
    resource = resources["data"]["resources"][0]
    read = _call(session, request_id, "unica.source.read", {
        "cwd": str(workspace), "snapshotId": resources["data"]["snapshotId"], "resourceId": resource["resourceId"],
    })
    request_id += 1
    text_profile = read["data"]["textProfile"]
    if text_profile.get("bomPrefixBytes") != 3 or text_profile.get("eol") != "crlf":
        raise SystemExit(f"source.read lost the observed BSL text profile: {read}")
    # The bounded resource surface is read-only: BSL mutation belongs to
    # unica.code.patch, so the whole flow must leave every byte in place.
    if _workspace_snapshot(workspace) != before_flow:
        raise SystemExit("the read-only source flow changed workspace bytes")
    projection = source_flow_projection(
        source_set,
        cache_root,
        resolve,
        children,
        resources,
        read,
    )
    if projection != expected_source_flow_projection(source_set):
        raise SystemExit(f"packaged source flow differs from the stable oracle: {projection}")
    return request_id, projection


def _exercise_reader_bridge(
    session: McpSession, request_id: int, workspace: Path
) -> int:
    """An address found by `unica.source.resolve` reaches a subject reader.

    This is the whole point of ADR-0049: the caller never has to know that a
    role's rights live two directories below its descriptor.
    """
    resolved = _call(session, request_id, "unica.source.resolve", {
        "cwd": str(workspace), "sourceSet": "main", "query": "Role.SmokeRole",
        "mode": "exact", "targetKind": "metadataObject",
    })
    request_id += 1
    candidates = [c.get("metadataPath") for c in resolved["data"]["candidates"]]
    if candidates != ["Role.SmokeRole"]:
        raise SystemExit(f"source.resolve did not return the role address: {resolved}")

    logical = _call(session, request_id, "unica.role.info", {
        "cwd": str(workspace), "sourceSet": "main", "metadataPath": "Role.SmokeRole",
    })
    request_id += 1
    if not logical.get("ok") or logical["data"].get("name") != "SmokeRole":
        raise SystemExit(f"role.info did not accept the resolved address: {logical}")

    conflict = _call(session, request_id, "unica.role.info", {
        "cwd": str(workspace), "sourceSet": "main", "metadataPath": "Role.SmokeRole",
        "RightsPath": "src/Roles/SmokeRole/Ext/Rights.xml",
    }, expect_ok=False)
    request_id += 1
    if "selector_conflict" not in json.dumps(conflict, ensure_ascii=False):
        raise SystemExit(f"role.info accepted two selectors at once: {conflict}")

    missing = _call(session, request_id, "unica.role.info", {
        "cwd": str(workspace), "sourceSet": "main", "metadataPath": "Role.Absent",
    }, expect_ok=False)
    request_id += 1
    if "target_not_found" not in json.dumps(missing, ensure_ascii=False):
        raise SystemExit(f"an unknown address must be a missing target: {missing}")
    return request_id


def smoke(
    command: list[str],
    plugin_root: Path,
    timeout_seconds: float,
    deadline: float,
    session_started: Callable[[McpSession, Path], None] | None = None,
    admission_lock: threading.Lock | None = None,
) -> None:
    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(plugin_root.resolve())
    expected_names = expected_tool_names(plugin_root)
    with tempfile.TemporaryDirectory(prefix="unica-packaged-source-smoke-") as temporary:
        root = Path(temporary)
        workspace = root / "workspace"
        workspace.mkdir()
        cache_root = root / "cache"
        environment["UNICA_CACHE_DIR"] = str(cache_root)
        _source_workspace(workspace)
        before = _workspace_snapshot(workspace)
        executable = Path(command[0])
        if executable.exists():
            command = [str(executable.resolve()), *command[1:]]
        admission = admission_lock or contextlib.nullcontext()
        with admission:
            session = McpSession(
                command,
                environment,
                timeout_seconds,
                cwd=workspace,
                deadline=deadline,
            )
            if session_started is not None:
                session_started(session, cache_root)
        try:
            initialize = session.request({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "unica-release-smoke", "version": "1"},
            }})
            if "result" not in initialize:
                raise SystemExit("Unica MCP initialize response is missing")
            # INV-MCP-SERVER-NAME. The smoke runs against the artifact that is
            # about to ship, so the published identity is checked here and not
            # only in the runtime unit tests.
            server_name = initialize["result"].get("serverInfo", {}).get("name")
            if server_name != "unica":
                raise SystemExit(
                    "Unica MCP initialize serverInfo.name must be 'unica', "
                    f"got {server_name!r}"
                )
            session.notify({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
            listed = session.request({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            tools = listed.get("result", {}).get("tools")
            if not isinstance(tools, list):
                raise SystemExit("Unica MCP tools/list response is missing")
            _stable_tool_contract(tools, expected_names)
            next_id, _ = _exercise_source_set(
                session, 3, workspace, cache_root, "main"
            )
            next_id, _ = _exercise_source_set(
                session, next_id, workspace, cache_root, "extension"
            )
            next_id = _exercise_bsl_search(
                session,
                next_id,
                timeout_seconds,
                deadline,
            )
            next_id = _exercise_reader_bridge(session, next_id, workspace)
            success = _meta_payload(
                session.request(
                    {
                        "jsonrpc": "2.0",
                        "id": next_id,
                        "method": "tools/call",
                        "params": {
                            "name": "unica.meta.info",
                            "arguments": {
                                "sourceSet": "main",
                                "metadataPath": "Enum.LanguageAware",
                            },
                        },
                    }
                ),
                expected_ok=True,
            )
            invalid = _meta_payload(
                session.request(
                    {
                        "jsonrpc": "2.0",
                        "id": next_id + 1,
                        "method": "tools/call",
                        "params": {"name": "unica.meta.info", "arguments": {}},
                    }
                ),
                expected_ok=False,
            )
            diagnostics = invalid.get("diagnostics")
            if not isinstance(diagnostics, list) or not diagnostics:
                raise SystemExit(f"invalid Meta smoke returned no diagnostics: {invalid}")
            if diagnostics[0].get("code") != "invalid_arguments":
                raise SystemExit(f"invalid Meta smoke returned unstable diagnostics: {invalid}")
        finally:
            _close_session_and_workspace_services(
                session,
                cache_root,
                max(5.0, timeout_seconds),
                deadline,
            )
        after = _workspace_snapshot(workspace)
        # The whole public source surface is read-only, so the packaged smoke
        # must end with the byte map it started from.
        expected = dict(before)
        if after != expected:
            changed = {
                path
                for path in set(before) | set(after)
                if before.get(path) != after.get(path)
            }
            raise SystemExit(
                "packaged source smoke did not match the complete expected byte map: "
                + ", ".join(sorted(changed))
            )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True)
    parser.add_argument("--binary-arg", action="append", default=[])
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=20)
    parser.add_argument(
        "--total-timeout-seconds",
        type=float,
        default=DEFAULT_TOTAL_TIMEOUT_SECONDS,
    )
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or args.total_timeout_seconds <= 0:
        parser.error("timeout values must be positive")
    deadline = time.monotonic() + args.total_timeout_seconds

    active_session: list[tuple[McpSession, Path]] = []
    cleanup_done = threading.Event()
    outcome_lock = threading.Lock()
    admission_lock = threading.Lock()
    outcome = ["running"]

    def cleanup_expired_smoke() -> None:
        message = (
            "Unica MCP smoke exceeded its aggregate deadline of "
            f"{args.total_timeout_seconds:g}s\n"
        ).encode("utf-8", errors="replace")
        try:
            os.write(2, message)
        finally:
            try:
                # Popen and session publication share this lock, so the
                # watchdog cannot observe the spawned child without its handle.
                with admission_lock:
                    active = active_session[0] if active_session else None
                if active is not None:
                    session, cache_root = active
                    session.terminate_tree(cache_root)
            finally:
                cleanup_done.set()
                os._exit(124)

    def expire_smoke() -> None:
        with outcome_lock:
            if outcome[0] != "running":
                return
            outcome[0] = "expired"
        cleanup_expired_smoke()

    watchdog = threading.Timer(args.total_timeout_seconds, expire_smoke)
    watchdog.daemon = True
    watchdog.start()
    try:
        try:
            smoke(
                [args.binary, *args.binary_arg],
                args.plugin_root,
                args.timeout_seconds,
                deadline,
                lambda session, cache_root: active_session.append((session, cache_root)),
                admission_lock,
            )
            with outcome_lock:
                if outcome[0] == "running":
                    outcome[0] = "completed"
                    completed = True
                else:
                    completed = False
            if not completed:
                # The watchdog already owns the outcome. A foreground success
                # racing its slower tree cleanup must not commit exit code 0.
                cleanup_done.wait(timeout=30)
                os._exit(124)
        except BaseException:
            with outcome_lock:
                if outcome[0] == "running" and time.monotonic() < deadline:
                    outcome[0] = "failed"
                    failure_action = "raise"
                elif outcome[0] == "running":
                    outcome[0] = "expired"
                    failure_action = "cleanup"
                elif outcome[0] == "expired":
                    failure_action = "wait"
                else:
                    failure_action = "raise"
            if failure_action == "cleanup":
                cleanup_expired_smoke()
            if failure_action == "wait":
                # Killing the owned process tree can make the foreground request
                # observe EOF before the watchdog reaches os._exit(124). Do not
                # let that cleanup race turn a deadline into an ordinary error.
                cleanup_done.wait(timeout=30)
                os._exit(124)
            raise
    finally:
        watchdog.cancel()
    print("verified packaged Unica MCP source-resource flow and bsl-analyzer search")


if __name__ == "__main__":
    main()
