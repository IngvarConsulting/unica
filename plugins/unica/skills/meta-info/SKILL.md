---
name: meta-info
description: Typed semantic navigation of 1C metadata from a certified source adapter. Use it to inspect objects, typed properties, capabilities, and bounded relations without parsing XML directly.
argument-hint: <ObjectPath> | <objectRef + snapshotRevision> | <cursor>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /meta-info - Typed metadata navigation

## MCP routing

Call only MCP tool `unica.meta.info`. It is read-only and always returns its result in `data.navigation`; do not call packaged scripts or internal adapters.

## Target modes

Use exactly one target mode per request:

| Mode | Arguments | Purpose |
|---|---|---|
| Bootstrap | `ObjectPath` | Start navigation from an object in a configured source set. |
| Expand node | `objectRef`, `snapshotRevision` | Re-resolve a known semantic object in the same captured snapshot. |
| Continue page | `cursor` | Continue the exact relation page returned earlier. |

`objectRef` is the returned semantic identity with `sourceId` and `objectKey`. It is not a file path. `snapshotRevision` prevents an expansion from silently reading changed metadata. A cursor is opaque, path-free, selection-bound, and snapshot-bound; return it unchanged. Continuations are available only for the process lifetime of the captured navigation snapshot. If that bounded snapshot is unavailable after restart or eviction, start again with `ObjectPath`; the result is structured `source_unavailable` or stale-snapshot diagnostics, never a filesystem fallback.

## Selection and relation pages

`select` is optional for bootstrap and object-reference requests. It may select typed properties, facets, and relation roles. Each relation request has a `role`, an optional `kind`, and optional `pageSize`; omitted `kind` is exactly `contains` (it never selects reference edges), the default page size is 25 and the maximum is 100. Select `kind: "references"` explicitly for reference edges. Repeated relation selections are normalized by `(role, kind)` with the smaller page size. There is no offset pagination.

```json
{
  "name": "unica.meta.info",
  "arguments": {
    "cwd": "<workspace>",
    "ObjectPath": "src/Catalogs/Items.xml",
    "select": {
      "properties": "all",
      "facets": "summary",
      "relations": [{ "role": "attributes", "pageSize": 25 }]
    }
  }
}
```

Continue a returned page only with its cursor:

```json
{
  "name": "unica.meta.info",
  "arguments": {
    "cwd": "<workspace>",
    "cursor": { "schemaVersion": 1, "...": "returned cursor" }
  }
}
```

## Response contract

The only payload is `data.navigation`:

```text
schemaVersion, status, snapshot, root, nodes, relations, diagnostics
```

`status` is `ready` or `unavailable`. A ready result has semantic `root` and `nodes`. A relation page has `relation`, typed child `items`, and optional `nextCursor`.

Each node exposes its semantic object reference and typed properties. Facets control the remaining node fields: `none` omits all capability and action facets; `summary` exposes only `capabilityState` and `actionProfile`; `full` additionally exposes the detailed capability vector, actions, and semantic actions. Property values are typed values rather than rendered XML: inspect `valueState`, `valueType`, `value`, provenance, and capability. Type descriptions are structured type sets, not text expressions. Capabilities state whether inspection or future mutation is modeled, blocked, or unavailable and include resolution, identity strength, snapshot consistency, coverage, format compatibility, source access, and support authorability.

## Unavailable navigation

Do not retry through a text analyzer. `unavailable` is structured and preserves empty `nodes` and `relations`. Diagnostics distinguish unsupported format (`format_unsupported`), corrupted or ambiguous metadata, stale snapshots/cursors, and unavailable sources. Platform XML 2.19 is intentionally unavailable; only a certified adapter may produce ready navigation.
