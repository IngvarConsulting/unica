# Draft upstream issue: typed integration API gaps in RLM v1.29.1

> Draft only. Review this text before publishing it in
> `Dach-Coin/rlm-tools-bsl`.

## Proposed title

Expose typed read operations and machine-readable runtime/index metadata

## Proposed body

### Context

We integrated the unmodified `rlm-tools-bsl` v1.29.1 release
(`8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed`) into a long-running MCP client.
The existing API works, but the adapter has to reproduce several protocol
conventions that would be safer and more stable as typed interfaces.

This report is based on the released v1.29.1 source and executable. It does not
include a downstream source patch.

### 1. Read operations require Python source and nested JSON

`tools/list` exposes:

```text
rlm_start
rlm_execute
rlm_end
rlm_help
rlm_projects
rlm_index
```

There are no typed MCP tools for search, definition lookup, module outline, or
object profile. A client therefore has to:

1. call `rlm_start`;
2. send a fixed Python program to `rlm_execute`;
3. serialize all user arguments into a data literal;
4. call a sandbox helper such as `search(...)`;
5. `print(json.dumps(...))`;
6. parse the MCP content JSON, the `rlm_execute` envelope, and then JSON from
   `stdout`;
7. call `rlm_end`.

For example, the search body is necessarily equivalent to:

```python
import json
args = json.loads(SERIALIZED_ARGUMENTS)
result = search(args["query"], scope="all", limit=args["limit"])
print(json.dumps(result, ensure_ascii=False))
```

Expected: typed MCP operations with JSON schemas and structured results, while
keeping `rlm_execute` for exploratory use.

Suggested minimum operations:

- `rlm_search`;
- `rlm_find_definition`;
- `rlm_get_module_outline`;
- `rlm_get_object_profile`.

This would remove Python-code generation from ordinary read-only integrations,
make argument validation explicit, and avoid treating `stdout` as a second
transport.

### 2. `get_object_profile` cannot request predefined items

On the same v1.29.1 session:

```python
get_object_profile(
    "Anything",
    sections=["predefined_items"],
    include_flow=False,
    include_code_usages=False,
    limit=10,
)
```

returns successfully but with an empty `sections` object. Predefined items are
available only through the separate `find_predefined(...)` helper. A client
that exposes one object-profile response must call both helpers and compose
their pagination/status metadata itself.

Expected: either support `predefined_items` as a profile section or document a
typed composition contract that preserves `limit`, `has_more`, object category,
and section status.

### 3. `index info` is human-readable and missing is exit code 0

Reproduction:

```console
$ RLM_INDEX_DIR=/tmp/empty-index rlm-bsl-index index info /path/to/config
Index not found: /tmp/empty-index/<id>/bsl_index.db
$ echo $?
0
```

This is useful for an interactive CLI, but an orchestrator must parse prose to
distinguish `missing`, `fresh`, `stale`, and `incomplete`.

Expected: an optional stable JSON mode, for example:

```console
rlm-bsl-index index info --format json /path/to/config
```

with a documented status field. The existing human-readable output and exit
code behavior can remain compatible.

### 4. MCP server version does not identify the installed release

The executable built from v1.29.1 returns:

```json
{
  "serverInfo": {
    "name": "rlm-tools-bsl",
    "version": "1.26.0"
  }
}
```

Expected: `serverInfo.version` reports `1.29.1`, or the initialize result
contains a separate package/API version field. Consumers need this identity to
select matching parsers and produce actionable compatibility diagnostics.

### Why these changes help

The current API remains usable and session reuse works. The requested additions
would make production integrations less coupled to sandbox helper names,
Python syntax, stdout framing, and CLI prose without removing the general
`rlm_execute` workflow.

### Out of scope

A PyInstaller `multiprocessing.freeze_support()` change was required in our
generic downstream packager. It is not an RLM source defect and is deliberately
excluded from this issue.

## Evidence retained downstream

- pinned source: v1.29.1,
  `8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed`;
- released assets: `rlm-tools-bsl-v1.29.1-build.2`;
- successful real session:
  `rlm_start → rlm_execute(search) → rlm_end`;
- packaged Unica E2E:
  unified search plus definition and outline through the persistent RLM MCP
  process;
- no direct SQLite reads in the consumer.
