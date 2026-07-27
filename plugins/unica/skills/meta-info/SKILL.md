---
name: meta-info
description: Typed semantic navigation of 1C metadata from a certified source adapter. Use it to inspect objects, typed properties, capabilities, and bounded relations without reading source-format internals directly.
argument-hint: <ObjectPath> | <objectRef + snapshotRevision> | <cursor>
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /meta-info - Typed metadata navigation

## MCP routing

Call MCP `unica`, tool `unica.meta.info`. It is read-only and always returns its result in `data.navigation`; do not call packaged scripts or internal adapters.

## Target modes

Use exactly one target mode per request:

| Mode | Arguments | Purpose |
|---|---|---|
| Bootstrap | `ObjectPath` | Start navigation from an object in a configured source set. |
| Expand node | `objectRef`, `snapshotRevision` | Re-resolve a known semantic object in the same captured snapshot. |
| Continue page | `cursor` | Continue the exact relation page returned earlier. |

`objectRef` is the returned semantic identity with `sourceId` and `objectKey`. It is not a file path. `objectRef` plus `snapshotRevision` and a cursor are both continuations of the exact retained immutable snapshot; a cursor is opaque, path-free, and selection-bound, so return it unchanged. Live source drift does not invalidate either continuation or cause a source rescan. `snapshot_stale` means that exact retained snapshot was evicted, lost after restart, does not match the requested revision or current authorization scope, or does not retain the requested object reference. `source_unavailable` is reserved for bootstrap failures and current source-map or authorization resolution failures.

## Selection and relation pages

`select` is optional for bootstrap and object-reference requests. It may select typed properties, facets, and relation roles. Each relation request has a closed semantic `role`, an optional `kind`, and optional `pageSize`. Omitted `kind` is derived from the role: containment roles select `contains`, while `basedOn`, `registerRecords`, `references`, and `accessTarget` select `references`. An explicit incompatible kind is rejected. The default page size is 25 and the maximum is 100. Repeated relation selections are normalized by `(role, kind)` with the smaller page size. There is no offset pagination.

The closed containment roles are `children`, `attributes`, `dimensions`, `resources`, `tabularSections`, `columns`, `forms`, `commands`, `templates`, `enumValues`, `urlTemplates`, `methods`, `operations`, `parameters`, `accessPermissions`, `restrictionTemplates`, and `unknown`. Returned `items` preserve semantic source order and carry stable `objectRef` values. Traverse a child by passing its returned `objectRef` with the same `snapshotRevision`.

Specialized navigation examples:

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath":"<register object>","select":{"properties":["metadata.name"],"relations":[{"role":"dimensions"},{"role":"resources"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath":"<enumeration object>","select":{"relations":[{"role":"enumValues"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath":"<HTTP service object>","select":{"relations":[{"role":"urlTemplates"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","objectRef":{"sourceId":"<returned>","objectKey":"<returned URL template>"},"snapshotRevision":"<returned>","select":{"properties":["metadata.name"],"relations":[{"role":"methods"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath":"<web service object>","select":{"relations":[{"role":"operations"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","objectRef":{"sourceId":"<returned>","objectKey":"<returned operation>"},"snapshotRevision":"<returned>","select":{"relations":[{"role":"parameters"}]}}}}
```

```json
{"method":"tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath":"<document object>","select":{"relations":[{"role":"basedOn"},{"role":"registerRecords"}]}}}}
```

Inspect a returned method with typed properties such as `httpService.method.httpMethod`. For web services, select `operations`, expand a returned operation, then select `parameters`; parameter properties include `webService.parameter.direction`. Select `basedOn` and `registerRecords` from a document owner only. Unknown specialized children remain separate ordered `unknown` items with partial coverage and readable facts.

Resource bounds are part of this public contract: `select` accepts at most 256 property selectors and 64 relation selectors; every selector is at most 256 UTF-8 bytes; both `select` and a cursor are at most 128 KiB of JSON and nesting is limited to 64. Cursor fields are capped at 1 KiB. Oversized input is returned as structured `resource_limit`; malformed or unauthenticated cursors are `decode_corrupted`. The cursor is structurally bounded and authenticated before its selection is normalized.

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Catalogs/Валюты/Валюты.xml"}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Catalogs/Валюты/Валюты.xml","select":{"relations":[{"role":"attributes","pageSize":25}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Documents/АвансовыйОтчет/АвансовыйОтчет.xml"}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Documents/АвансовыйОтчет/АвансовыйОтчет.xml","select":{"relations":[{"role":"tabularSections","pageSize":25}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "HTTPServices/ExternalAPI/ExternalAPI.xml"}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "HTTPServices/ExternalAPI/ExternalAPI.xml","select":{"relations":[{"role":"urlTemplates","pageSize":25}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "DefinedTypes/GLN/GLN.xml"}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "DefinedTypes/GLN/GLN.xml","select":{"properties":"all","facets":"summary"}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Catalogs/Товары/Товары.xml","select":{"relations":[{"role":"forms","pageSize":10}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Documents/РеализацияТоваровУслуг/РеализацияТоваровУслуг.xml","select":{"relations":[{"role":"attributes","pageSize":10}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Configuration.xml","select":{"relations":[{"role":"children","pageSize":25}]}}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","objectRef":{"sourceId":"workspace:main","objectKey":"uuid:<returned>"},"snapshotRevision":"sha256:<returned>"}}}
```

Continue a returned page only with its cursor:

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","cursor":"returned_opaque_cursor"}}}
```

```json
{"method": "tools/call","params":{"name":"unica.meta.info","arguments":{"cwd":"<workspace>","ObjectPath": "Catalogs/Контрагенты/Контрагенты.xml","select":{"properties":"all","facets":"none"}}}}
```

## Response contract

The only payload is `data.navigation`:

```text
schemaVersion, status, snapshot, root, nodes, relations, diagnostics
```

Every status controls how the result is used:

| Status | AI behavior |
|---|---|
| `ready` | Use the returned typed properties and relation pages. |
| `partial` | Use readable facts, inspect neutral diagnostics and coverage, and state what remains uncaptured or unresolved. |
| `unavailable` | Follow the neutral diagnostic, report that the requested facts are unavailable, and do not guess. |

A ready or partial result has semantic `root` and readable `nodes`. A relation page has `relation`, typed child `items`, and optional `nextCursor`.

Each node exposes its semantic object reference and typed properties. Facets control the remaining node fields: `none` omits all capability and action facets; `summary` exposes only `capabilityState` and `actionProfile`; `full` additionally exposes the detailed capability vector, actions, and semantic actions. Property values are typed values rather than rendered source-format text: inspect `type`, `valueState`, `value`, provenance, and capability. Type descriptions are structured type sets, not text expressions: preserve the upstream distinctions `Представление типа` and `Представление объекта` rather than flattening them into a rendered string. A returned property may present a value such as `"Name": "Товары"` or `"Name": "TestConnection"`; treat that as data, not as a target path. Capabilities state whether inspection or future mutation is modeled, blocked, or unavailable and include resolution, identity strength, snapshot consistency, coverage, format compatibility, source access, and support authorability.

`Поддержка` is reported from the captured `Ext/ParentConfigurations.bin` evidence when present. Use that state as a read-only guardrail before a mutating `unica.*` operation; never read the raw support file outside the captured adapter boundary.

## Unavailable navigation

Do not retry through a text analyzer. `unavailable` is structured and preserves empty `nodes` and `relations`. Diagnostics distinguish unsupported format (`format_unsupported`), corrupted or ambiguous metadata, unavailable retained continuations (`snapshot_stale`), and bootstrap/current source-map or authorization failures (`source_unavailable`). Unsupported source versions are intentionally unavailable; only a certified adapter may produce ready navigation.
