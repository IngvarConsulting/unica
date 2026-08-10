#!/usr/bin/env python3
"""Run a packaged Unica binary through its public MCP source-resource flow."""

from __future__ import annotations

import argparse
import json
import os
import queue
import subprocess
import tempfile
import threading
from pathlib import Path


TOOL_SURFACE_REVIEW_RELATIVE = Path("spec/architecture/tool-surface-review.json")
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
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
        "type": "boolean"
      },
      "cursor": {
        "description": "Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
        "type": "string"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results).",
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "metadataPath": {
        "description": "Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics.",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "description": "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set",
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
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
        "type": "boolean"
      },
      "cwd": {
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
        "type": "string"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results).",
        "maximum": 65536,
        "minimum": 1,
        "type": "integer"
      },
      "offset": {
        "description": "Zero-based byte offset inside the immutable resource snapshot",
        "minimum": 0,
        "type": "integer"
      },
      "resourceId": {
        "description": "Opaque resource identifier returned inside one source.resources snapshot; valid only together with the snapshotId that issued it",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "snapshotId": {
        "description": "Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes",
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
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
        "type": "boolean"
      },
      "cursor": {
        "description": "Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
        "type": "string"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results).",
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "mode": {
        "description": "Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full|incremental|partial for dump, load|merge for load, designer-config|designer-modules|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze|status|catalog|file|workspace on unica.code.diagnostics) \u2014 always use the enum published in that tool's own schema.",
        "enum": [
          "exact",
          "prefix"
        ],
        "type": "string"
      },
      "query": {
        "description": "Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "description": "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "targetKind": {
        "description": "Optional `unica.source.resolve` filter: `metadataObject` or `module`; it narrows exact or prefix matches without changing their canonical metadataPath",
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
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
        "type": "boolean"
      },
      "cursor": {
        "description": "Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "cwd": {
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
        "type": "string"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results).",
        "maximum": 50,
        "minimum": 1,
        "type": "integer"
      },
      "metadataPath": {
        "description": "Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics.",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "scope": {
        "description": "Bounded source.resources manifest scope: self, aggregate, or registrations",
        "enum": [
          "self",
          "aggregate",
          "registrations"
        ],
        "type": "string"
      },
      "snapshotId": {
        "description": "Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes",
        "minLength": 1,
        "pattern": "\\S",
        "type": "string"
      },
      "sourceSet": {
        "description": "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set",
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
        "type": "boolean",
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does"
      },
      "cursor": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S+$",
        "description": "Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot"
      },
      "cwd": {
        "type": "string",
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it"
      },
      "limit": {
        "type": "integer",
        "minimum": 1,
        "maximum": 50,
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, code.graph nodes, code.diagnostics findings, standards results)."
      },
      "metadataPath": {
        "type": "string",
        "pattern": "^(?:XDTOPackage|ПакетXDTO)\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*$",
        "description": "Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected."
      },
      "sourceSet": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S(?:.*\\S)?$",
        "description": "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set"
      },
      "typeName": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$",
        "description": "Name of the XDTO valueType or objectType, or of the target objectType for a property operation."
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
      "base": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*:[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$",
        "description": "Prefixed lexical QName naming the base type of a new XDTO valueType in `unica.xdto.edit`, for example `xs:string`; an unprefixed name or surrounding whitespace is rejected."
      },
      "confirm": {
        "type": "boolean",
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does"
      },
      "cwd": {
        "type": "string",
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it"
      },
      "dryRun": {
        "type": "boolean",
        "default": true,
        "description": "Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution."
      },
      "metadataPath": {
        "type": "string",
        "pattern": "^(?:XDTOPackage|ПакетXDTO)\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*$",
        "description": "Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected."
      },
      "name": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$",
        "description": "Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section`"
      },
      "operation": {
        "type": "string",
        "enum": [
          "add-value-type",
          "add-object-type",
          "add-property",
          "remove-type",
          "remove-property"
        ],
        "description": "Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema."
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
        ],
        "description": "New XDTO property object for `unica.xdto.edit`: `name` must be an XML NCName and `type` a prefixed lexical QName; `minOccurs` is optional and must be 0 or 1."
      },
      "propertyPath": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*(?:\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*(?:\\\\\\.[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-0-9·̀-ͯ‿-⁀]*)*)*$",
        "description": "Property path to a nested XDTO `typeDef`: an unescaped dot separates segments and `\\.` denotes a literal dot inside one NCName, for example `A\\.B.Child`."
      },
      "sourceSet": {
        "type": "string",
        "minLength": 1,
        "pattern": "^\\S(?:.*\\S)?$",
        "description": "Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set"
      },
      "typeName": {
        "type": "string",
        "minLength": 1,
        "pattern": "^[A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�][A-Z_a-zÀ-ÖØ-öø-˿Ͱ-ͽͿ-῿‌-‍⁰-↏Ⰰ-⿯、-퟿豈-﷏ﷰ-�\\-.0-9·̀-ͯ‿-⁀]*$",
        "description": "Name of the XDTO valueType or objectType, or of the target objectType for a property operation."
      }
    },
    "required": [
      "sourceSet",
      "metadataPath",
      "operation"
    ],
    "oneOf": [
      {
        "properties": {
          "operation": {
            "const": "add-value-type"
          }
        },
        "required": [
          "operation",
          "name",
          "base"
        ],
        "not": {
          "anyOf": [
            {
              "required": [
                "typeName"
              ]
            },
            {
              "required": [
                "propertyPath"
              ]
            },
            {
              "required": [
                "property"
              ]
            }
          ]
        }
      },
      {
        "properties": {
          "operation": {
            "const": "add-object-type"
          }
        },
        "required": [
          "operation",
          "name"
        ],
        "not": {
          "anyOf": [
            {
              "required": [
                "base"
              ]
            },
            {
              "required": [
                "typeName"
              ]
            },
            {
              "required": [
                "propertyPath"
              ]
            },
            {
              "required": [
                "property"
              ]
            }
          ]
        }
      },
      {
        "properties": {
          "operation": {
            "const": "add-property"
          }
        },
        "required": [
          "operation",
          "typeName",
          "property"
        ],
        "not": {
          "anyOf": [
            {
              "required": [
                "name"
              ]
            },
            {
              "required": [
                "base"
              ]
            }
          ]
        }
      },
      {
        "properties": {
          "operation": {
            "const": "remove-type"
          }
        },
        "required": [
          "operation",
          "name"
        ],
        "not": {
          "anyOf": [
            {
              "required": [
                "base"
              ]
            },
            {
              "required": [
                "typeName"
              ]
            },
            {
              "required": [
                "propertyPath"
              ]
            },
            {
              "required": [
                "property"
              ]
            }
          ]
        }
      },
      {
        "properties": {
          "operation": {
            "const": "remove-property"
          }
        },
        "required": [
          "operation",
          "typeName",
          "name"
        ],
        "not": {
          "anyOf": [
            {
              "required": [
                "base"
              ]
            },
            {
              "required": [
                "property"
              ]
            }
          ]
        }
      }
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
        "\t\t\t<CommonModule>Shared</CommonModule>\n\t\t</ChildObjects>",
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
    ) -> None:
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=environment,
            cwd=cwd,
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        assert self.process.stderr is not None
        self.timeout_seconds = timeout_seconds
        self.lines: queue.Queue[str] = queue.Queue()
        self.diagnostics: list[str] = []
        self.reader = threading.Thread(target=self._read_stdout, daemon=True)
        self.reader.start()
        # The server stays open across the whole flow and writes diagnostics to
        # stderr, so an undrained pipe buffer would block it mid-request and
        # surface as a bare timeout.
        self.error_reader = threading.Thread(target=self._read_stderr, daemon=True)
        self.error_reader.start()

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

    def request(self, message: dict) -> dict:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        while True:
            try:
                line = self.lines.get(timeout=self.timeout_seconds)
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
            result = self.process.wait(timeout=self.timeout_seconds)
        except subprocess.TimeoutExpired as error:
            self.process.kill()
            raise SystemExit(
                f"Unica MCP smoke timed out after {self.timeout_seconds:g}s: {self._detail()}"
            ) from error
        self.reader.join(timeout=self.timeout_seconds)
        self.error_reader.join(timeout=self.timeout_seconds)
        detail = self._detail()
        for stream in (self.process.stdout, self.process.stderr):
            if stream is not None and not stream.closed:
                stream.close()
        if result != 0:
            raise SystemExit(f"Unica MCP exited with {result}: {detail}")


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


def _call(session: McpSession, request_id: int, name: str, arguments: dict) -> dict:
    return _tool_payload(
        session.request(
            {
                "jsonrpc": "2.0",
                "id": request_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            }
        )
    )


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


def smoke(command: list[str], plugin_root: Path, timeout_seconds: float) -> None:
    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(plugin_root.resolve())
    expected_names = expected_tool_names(plugin_root)
    with tempfile.TemporaryDirectory(prefix="unica-packaged-source-smoke-") as temporary:
        root = Path(temporary)
        workspace = root / "workspace"
        workspace.mkdir()
        environment["UNICA_CACHE_DIR"] = str(root / "cache")
        _source_workspace(workspace)
        before = _workspace_snapshot(workspace)
        executable = Path(command[0])
        if executable.exists():
            command = [str(executable.resolve()), *command[1:]]
        session = McpSession(command, environment, timeout_seconds, cwd=workspace)
        try:
            initialize = session.request({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "unica-release-smoke", "version": "1"},
            }})
            if "result" not in initialize:
                raise SystemExit("Unica MCP initialize response is missing")
            session.notify({"jsonrpc": "2.0", "method": "notifications/initialized", "params": {}})
            listed = session.request({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            tools = listed.get("result", {}).get("tools")
            if not isinstance(tools, list):
                raise SystemExit("Unica MCP tools/list response is missing")
            _stable_tool_contract(tools, expected_names)
            cache_root = root / "cache"
            next_id, _ = _exercise_source_set(
                session, 3, workspace, cache_root, "main"
            )
            next_id, _ = _exercise_source_set(
                session, next_id, workspace, cache_root, "extension"
            )
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
            session.close()
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
    args = parser.parse_args()
    smoke([args.binary, *args.binary_arg], args.plugin_root, args.timeout_seconds)
    print("verified packaged Unica MCP source-resource flow")


if __name__ == "__main__":
    main()
