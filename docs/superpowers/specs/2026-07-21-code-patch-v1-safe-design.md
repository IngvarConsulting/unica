# Safe `unica.code.patch` v1

Status: approved for implementation on 2026-07-21.

Related: `Refs #73`. This slice does not close #73.

## Intent and boundary

`unica.code.patch` v1 makes one safe insertion into one existing BSL module in
a selected platform-XML Configuration source set. It is a source mutation tool,
not a general BSL editor.

The target may be any existing regular `*Module.bsl` in a supported canonical
platform-XML layout in that source set, including `Module.bsl`,
`ObjectModule.bsl`, and `ManagerModule.bsl`. Empty modules,
`replace`/`delete`, batches, external processors/reports, EDT, extensions,
platform syntax execution, and durable runtime-delivery events remain follow-up
work for #73.

## Public contract

The tool accepts the normal workspace/source selection arguments and applies
the project support guard internally, plus:

| Argument | Rule |
| --- | --- |
| `path` | Required workspace-relative path to an existing `*Module.bsl` in the selected source set. |
| `operation` | Required; only `insert`. |
| `selector` | Exactly one of `{ "method": "Name" }` or `{ "anchor": "exact text" }`. |
| `content` | Required non-empty BSL fragment. |
| `position` | `before` or `after`. |

The JSON Schema and runtime validation share the same tagged-selector model.
The schema must accept each documented selector and reject zero, both, unknown,
or empty selector members. A selector must resolve exactly once; ambiguity is an
error before staging or mutation.

## Mutation pipeline

1. Resolve the workspace path and selected Configuration source set, applying
   existing path and support guards.
2. Read the target as bytes and parse a BSL structural index. The index handles
   UTF-8 BOM, Russian and English procedure/function keywords, comments and
   string literals, and method boundaries.
3. Resolve the selector to a byte-exact insertion point. An anchor must lie
   wholly in exactly one method.
4. Build a postimage that preserves every untouched byte. Preserve BOM and the
   local EOL at the insertion boundary; never normalise the entire file.
5. Compute SHA-256 pre/post hashes, byte and line/column changed ranges, and a
   valid unified diff from those exact two images. A repeated identical request
   is a no-op with equal hashes, empty diff/ranges, and no changed target.
6. Validate the exact in-memory postimage with the pinned `bsl-analyzer` parser
   before staging or publication. Return a terminal source-validation status;
   this v1 does not claim a 1C platform syntax check.
7. For an applied, non-no-op call, publish through the shared
   `single_file_publisher` with an exact preimage. This preserves its existing
   permissions, symlink/reparse, cleanup, locking, and concurrent-change
   guarantees. Dry runs never publish.

## Result contract

Both preview and apply return a typed data object containing canonical path,
selected source set, module role, `preHash`, `postHash`, byte and line/column
changed ranges, unified diff, affected target, and validation status.

An applied mutation reports exactly one changed target. Preview and no-op
results report no changed target, so they cannot emit a source-change event.

## Tests and acceptance evidence

Implementation starts with failing contract/integration tests for:

1. JSON Schema acceptance/rejection for both selector variants.
2. Existing `Module.bsl`, `ObjectModule.bsl`, and `ManagerModule.bsl` targets.
3. BOM, CRLF, LF, and mixed-EOL preservation outside the insertion.
4. Russian/English methods; comments and strings that resemble keywords.
5. Unique, missing, and ambiguous selectors; anchor containment.
6. Dry-run, applied mutation, and byte-identical no-op behavior.
7. Unified-diff round-trip, typed result fields, and source/support selection.
8. Invalid postimage and publication failure leaving the original unchanged.

## Follow-up work

#73 remains open for empty-module creation, `replace`, explicit
`expectedCount`, arbitrary module declarations, platform syntax/diagnostics,
and durable workspace-event delivery. Those features must be specified and
tested in later slices rather than silently implied by v1.
