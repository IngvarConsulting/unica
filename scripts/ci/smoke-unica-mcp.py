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


REQUIRED_DCS_TOOLS = {
    "unica.dcs.compile",
    "unica.dcs.edit",
    "unica.dcs.info",
    "unica.dcs.validate",
}
REQUIRED_TOOLS = REQUIRED_DCS_TOOLS | {
    "unica.project.status",
    "unica.standards.search",
    "unica.standards.explain",
    "unica.source.resolve",
    "unica.source.children",
    "unica.source.resources",
    "unica.source.read",
    "unica.source.apply",
}
REMOVED_DCS_TOOL_ALIASES = {
    name.replace(".dcs.", ".s" + "kd.") for name in REQUIRED_DCS_TOOLS
}
SOURCE_TOOL_NAMES = {
    "unica.source.resolve",
    "unica.source.children",
    "unica.source.resources",
    "unica.source.read",
    "unica.source.apply",
}
EXPECTED_SOURCE_INPUT_SCHEMAS = json.loads(
    r'''
{
  "unica.source.apply": {
    "additionalProperties": false,
    "properties": {
      "confirm": {
        "description": "Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does",
        "type": "boolean"
      },
      "content": {
        "description": "Complete UTF-8 BSL replacement text; decoded bytes are capped again by the provider",
        "maxLength": 1048576,
        "type": "string"
      },
      "contentEncoding": {
        "const": "utf-8",
        "description": "unica.source.apply replacement encoding; the first contract accepts only utf-8",
        "type": "string"
      },
      "cwd": {
        "description": "Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it",
        "type": "string"
      },
      "dryRun": {
        "description": "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
        "type": "boolean"
      },
      "expectedHash": {
        "description": "Exact SHA-256 hash returned for the resource by source.resources; source.apply fails closed when either the argument or current preimage differs",
        "minLength": 1,
        "pattern": "^sha256:[0-9a-f]{64}$",
        "type": "string"
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
      "resourceId",
      "expectedHash",
      "content"
    ],
    "type": "object"
  },
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
      "dryRun": {
        "description": "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
        "type": "boolean"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results).",
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
      "dryRun": {
        "description": "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
        "type": "boolean"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results).",
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
      "dryRun": {
        "description": "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
        "type": "boolean"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results).",
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
      "dryRun": {
        "description": "Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution.",
        "type": "boolean"
      },
      "limit": {
        "description": "Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results).",
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


EXPECTED_SOURCE_FLOW_PROJECTIONS = json.loads(
    r'''
{
  "extension": {
    "applied": {
      "artifacts": [],
      "cache": {
        "events": [
          "SourceResourcesReplaced"
        ],
        "fresh": [],
        "invalidated": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "lazy_rebuilt": [],
        "mode": "applied",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [
        "extension + CommonModule.Shared.Module: replaced BSL resource"
      ],
      "data": {
        "changedRanges": [
          {
            "endByte": 20,
            "endColumn": 19,
            "endLine": 1,
            "startByte": 13,
            "startColumn": 12,
            "startLine": 1
          }
        ],
        "diff": "--- a/CommonModule.Shared.Module\n+++ b/CommonModule.Shared.Module\n@@ -1,2 +1,2 @@\n-\ufeffProcedure RunExtension()\r\n+\ufeffProcedure Changed()\r\n EndProcedure\r\n",
        "noOp": false,
        "postHash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "preHash": "sha256:41e8d685fd708f331f099494d36fe1a0059ae144da2de497c8dc3f5629c900ea",
        "role": "bslModule",
        "sourceSet": "extension",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "extension",
          "targetKind": "module"
        },
        "validation": {
          "kind": "bsl-analyzer-parser",
          "status": "passed"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "unica.source.apply replaced one BSL resource",
      "warnings": []
    },
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
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
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
    "current": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "completeness": "complete",
        "resources": [
          {
            "access": [
              "read",
              "replace"
            ],
            "hash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
            "limits": {
              "maxReadBytes": 65536
            },
            "mediaType": "text/x-bsl",
            "role": "bslModule",
            "size": 38,
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
    "postimage": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "appliedLimit": 65536,
        "content": "\ufeffProcedure Changed()\r\nEndProcedure\r\n",
        "contentEncoding": "utf-8",
        "eof": true,
        "hash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "length": 38,
        "offset": 0,
        "size": 38,
        "textProfile": {
          "bomPrefixBytes": 3,
          "encoding": "utf-8",
          "eol": "crlf"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.read returned 38 byte(s)",
      "warnings": []
    },
    "preview": {
      "artifacts": [],
      "cache": {
        "events": [
          "SourceResourcesReplaced"
        ],
        "fresh": [],
        "invalidated": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "lazy_rebuilt": [],
        "mode": "dry-run",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "changedRanges": [
          {
            "endByte": 20,
            "endColumn": 19,
            "endLine": 1,
            "startByte": 13,
            "startColumn": 12,
            "startLine": 1
          }
        ],
        "diff": "--- a/CommonModule.Shared.Module\n+++ b/CommonModule.Shared.Module\n@@ -1,2 +1,2 @@\n-\ufeffProcedure RunExtension()\r\n+\ufeffProcedure Changed()\r\n EndProcedure\r\n",
        "noOp": false,
        "postHash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "preHash": "sha256:41e8d685fd708f331f099494d36fe1a0059ae144da2de497c8dc3f5629c900ea",
        "role": "bslModule",
        "sourceSet": "extension",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "extension",
          "targetKind": "module"
        },
        "validation": {
          "kind": "bsl-analyzer-parser",
          "status": "passed"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "dry run: unica.source.apply planned one BSL resource replacement",
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
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
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
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
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
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "completeness": "complete",
        "resources": [
          {
            "access": [
              "read",
              "replace"
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
    "applied": {
      "artifacts": [],
      "cache": {
        "events": [
          "SourceResourcesReplaced"
        ],
        "fresh": [],
        "invalidated": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "lazy_rebuilt": [],
        "mode": "applied",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [
        "main + CommonModule.Shared.Module: replaced BSL resource"
      ],
      "data": {
        "changedRanges": [
          {
            "endByte": 20,
            "endColumn": 19,
            "endLine": 1,
            "startByte": 13,
            "startColumn": 12,
            "startLine": 1
          }
        ],
        "diff": "--- a/CommonModule.Shared.Module\n+++ b/CommonModule.Shared.Module\n@@ -1,2 +1,2 @@\n-\ufeffProcedure Run()\r\n+\ufeffProcedure Changed()\r\n EndProcedure\r\n",
        "noOp": false,
        "postHash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "preHash": "sha256:87c24a6da821b5f96a884b7210133a30d7ee2c66cf281934bae1afc8281a8cbb",
        "role": "bslModule",
        "sourceSet": "main",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "main",
          "targetKind": "module"
        },
        "validation": {
          "kind": "bsl-analyzer-parser",
          "status": "passed"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "unica.source.apply replaced one BSL resource",
      "warnings": []
    },
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
    "current": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "completeness": "complete",
        "resources": [
          {
            "access": [
              "read",
              "replace"
            ],
            "hash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
            "limits": {
              "maxReadBytes": 65536
            },
            "mediaType": "text/x-bsl",
            "role": "bslModule",
            "size": 38,
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
    "postimage": {
      "artifacts": [],
      "cache": {
        "events": [],
        "fresh": [],
        "invalidated": [],
        "lazy_rebuilt": [],
        "mode": "read",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "appliedLimit": 65536,
        "content": "\ufeffProcedure Changed()\r\nEndProcedure\r\n",
        "contentEncoding": "utf-8",
        "eof": true,
        "hash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "length": 38,
        "offset": 0,
        "size": 38,
        "textProfile": {
          "bomPrefixBytes": 3,
          "encoding": "utf-8",
          "eol": "crlf"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "source.read returned 38 byte(s)",
      "warnings": []
    },
    "preview": {
      "artifacts": [],
      "cache": {
        "events": [
          "SourceResourcesReplaced"
        ],
        "fresh": [],
        "invalidated": [
          "bsl_diagnostics",
          "bsl_index"
        ],
        "lazy_rebuilt": [],
        "mode": "dry-run",
        "refreshed": [],
        "root": "<cache-root>",
        "stale": [],
        "workspace_epoch": "<workspace-epoch>"
      },
      "changes": [],
      "data": {
        "changedRanges": [
          {
            "endByte": 20,
            "endColumn": 19,
            "endLine": 1,
            "startByte": 13,
            "startColumn": 12,
            "startLine": 1
          }
        ],
        "diff": "--- a/CommonModule.Shared.Module\n+++ b/CommonModule.Shared.Module\n@@ -1,2 +1,2 @@\n-\ufeffProcedure Run()\r\n+\ufeffProcedure Changed()\r\n EndProcedure\r\n",
        "noOp": false,
        "postHash": "sha256:524a90a52def76c469ea1bc6a7dcf3b3524249e9b52cbc48caba768a186de762",
        "preHash": "sha256:87c24a6da821b5f96a884b7210133a30d7ee2c66cf281934bae1afc8281a8cbb",
        "role": "bslModule",
        "sourceSet": "main",
        "target": {
          "metadataPath": "CommonModule.Shared.Module",
          "sourceSet": "main",
          "targetKind": "module"
        },
        "validation": {
          "kind": "bsl-analyzer-parser",
          "status": "passed"
        }
      },
      "errors": [],
      "ok": true,
      "summary": "dry run: unica.source.apply planned one BSL resource replacement",
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
              "read",
              "replace"
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
    preview: dict,
    applied: dict,
    current: dict,
    postimage: dict,
) -> dict:
    return {
        "sourceSet": source_set,
        "resolve": canonical_source_projection(resolve, cache_root),
        "children": canonical_source_projection(children, cache_root),
        "resources": canonical_source_projection(resources, cache_root),
        "read": canonical_source_projection(read, cache_root),
        "preview": canonical_source_projection(preview, cache_root),
        "applied": canonical_source_projection(applied, cache_root),
        "current": canonical_source_projection(current, cache_root),
        "postimage": canonical_source_projection(postimage, cache_root),
    }


def expected_source_flow_projection(source_set: str) -> dict:
    return EXPECTED_SOURCE_FLOW_PROJECTIONS[source_set]


class McpSession:
    def __init__(self, command: list[str], environment: dict[str, str], timeout_seconds: float) -> None:
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            env=environment,
        )
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        self.timeout_seconds = timeout_seconds
        self.lines: queue.Queue[str] = queue.Queue()
        self.reader = threading.Thread(target=self._read_stdout, daemon=True)
        self.reader.start()

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put("")

    def request(self, message: dict) -> dict:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        while True:
            try:
                line = self.lines.get(timeout=self.timeout_seconds)
            except queue.Empty as error:
                raise SystemExit(f"Unica MCP smoke timed out after {self.timeout_seconds:g}s") from error
            if not line:
                raise SystemExit("Unica MCP exited before the expected response")
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
            raise SystemExit(f"Unica MCP smoke timed out after {self.timeout_seconds:g}s") from error
        if result != 0:
            assert self.process.stderr is not None
            detail = self.process.stderr.read().strip() or "no process output"
            raise SystemExit(f"Unica MCP exited with {result}: {detail}")


def _tool_payload(response: dict) -> dict:
    if "error" in response:
        raise SystemExit(f"Unica MCP tools/call failed: {response['error']}")
    try:
        payload = json.loads(response["result"]["content"][0]["text"])
    except (KeyError, IndexError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"Unica MCP tools/call has no JSON payload: {response}") from error
    if not payload.get("ok"):
        raise SystemExit(f"Unica MCP tools/call rejected source flow: {payload}")
    _assert_path_free(payload)
    return payload


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


def _stable_tool_contract(tools: list[dict]) -> None:
    by_name = {tool.get("name"): tool for tool in tools if isinstance(tool, dict)}
    for name in sorted(REQUIRED_TOOLS):
        if name not in by_name:
            raise SystemExit(f"Unica MCP tools/list is missing: {name}")
    projected = {}
    for name in sorted(SOURCE_TOOL_NAMES):
        schema = by_name[name].get("inputSchema")
        if not isinstance(schema, dict):
            raise SystemExit(f"Unica MCP tools/list has no input schema for {name}")
        _assert_path_free(schema)
        projected[name] = schema
    if projected != EXPECTED_SOURCE_INPUT_SCHEMAS:
        raise SystemExit("Unica MCP source input schema projection drifted")


def _exercise_source_set(
    session: McpSession,
    request_id: int,
    workspace: Path,
    cache_root: Path,
    source_set: str,
) -> tuple[int, dict]:
    target = "CommonModule.Shared.Module"
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
    content = "Procedure Changed()\nEndProcedure\n"
    apply_args = {
        "cwd": str(workspace), "snapshotId": resources["data"]["snapshotId"], "resourceId": resource["resourceId"],
        "expectedHash": resource["hash"], "content": content, "contentEncoding": "utf-8",
    }
    before_preview = _workspace_snapshot(workspace)
    preview = _call(session, request_id, "unica.source.apply", apply_args)
    request_id += 1
    after_preview = _workspace_snapshot(workspace)
    if after_preview != before_preview:
        raise SystemExit("source.apply preview changed workspace bytes")
    expected_after_apply = dict(before_preview)
    module_root = "src" if source_set == "main" else "ext"
    module_path = f"{module_root}/CommonModules/Shared/Ext/Module.bsl"
    expected_after_apply[module_path] = (
        "\ufeff" + content.replace("\n", "\r\n")
    ).encode("utf-8")
    applied = _call(session, request_id, "unica.source.apply", {**apply_args, "dryRun": False})
    request_id += 1
    after_apply = _workspace_snapshot(workspace)
    if after_apply != expected_after_apply:
        raise SystemExit("source.apply changed bytes outside its complete expected workspace map")
    if preview["data"]["postHash"] != applied["data"]["postHash"]:
        raise SystemExit("source.apply preview and apply plans differ")
    for payload, mode in ((preview, "dry-run"), (applied, "applied")):
        if (
            payload["cache"]["mode"] != mode
            or payload["cache"]["events"] != ["SourceResourcesReplaced"]
            or payload["cache"]["invalidated"] != ["bsl_diagnostics", "bsl_index"]
        ):
            raise SystemExit(f"source.apply did not publish logical cache impact: {payload}")
    current = _call(session, request_id, "unica.source.resources", {
        "cwd": str(workspace), "sourceSet": source_set, "metadataPath": target, "scope": "self",
    })
    request_id += 1
    if current["data"]["snapshotId"] == resources["data"]["snapshotId"]:
        raise SystemExit("source.resources did not issue a fresh postimage snapshot")
    current_resource = current["data"]["resources"][0]
    postimage = _call(session, request_id, "unica.source.read", {
        "cwd": str(workspace), "snapshotId": current["data"]["snapshotId"], "resourceId": current_resource["resourceId"],
    })
    request_id += 1
    projection = source_flow_projection(
        source_set,
        cache_root,
        resolve,
        children,
        resources,
        read,
        preview,
        applied,
        current,
        postimage,
    )
    if projection != expected_source_flow_projection(source_set):
        raise SystemExit(f"packaged source flow differs from the stable oracle: {projection}")
    return request_id, projection


def smoke(command: list[str], plugin_root: Path, timeout_seconds: float) -> None:
    environment = os.environ.copy()
    environment["UNICA_PLUGIN_ROOT"] = str(plugin_root.resolve())
    with tempfile.TemporaryDirectory(prefix="unica-packaged-source-smoke-") as temporary:
        root = Path(temporary)
        workspace = root / "workspace"
        workspace.mkdir()
        environment["UNICA_CACHE_DIR"] = str(root / "cache")
        _source_workspace(workspace)
        before = _workspace_snapshot(workspace)
        session = McpSession(command, environment, timeout_seconds)
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
            _stable_tool_contract(tools)
            names = {tool.get("name") for tool in tools if isinstance(tool, dict)}
            removed_aliases = sorted(REMOVED_DCS_TOOL_ALIASES & names)
            if removed_aliases:
                raise SystemExit("Unica MCP tools/list exposes removed DCS aliases: " + ", ".join(removed_aliases))
            cache_root = root / "cache"
            next_id, _ = _exercise_source_set(
                session, 3, workspace, cache_root, "main"
            )
            _, _ = _exercise_source_set(
                session, next_id, workspace, cache_root, "extension"
            )
        finally:
            session.close()
        after = _workspace_snapshot(workspace)
        expected = dict(before)
        final_module = b"\xef\xbb\xbfProcedure Changed()\r\nEndProcedure\r\n"
        expected["src/CommonModules/Shared/Ext/Module.bsl"] = final_module
        expected["ext/CommonModules/Shared/Ext/Module.bsl"] = final_module
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
