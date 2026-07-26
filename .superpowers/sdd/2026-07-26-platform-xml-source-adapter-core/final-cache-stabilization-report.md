# Final Cache Stabilization Report

## Scope

Implemented findings 3, 6, and 7 only: truthful continuation-cache admission,
bounded cache retention, and cursor projection limited to its bound relation
group. No capture, support, or Platform XML type/projector behavior changed.

## Limits and Cache Policy

The process-lifetime continuation cache is FIFO. Reads do not refresh entry
order; insertion evicts from the front deterministically until both entry and
byte limits admit the new snapshot.

| Limit | Value |
| --- | ---: |
| Entries | 64 |
| Per snapshot serialized cache payload | 32 MiB |
| Total serialized cache payload | 128 MiB |
| Nodes per snapshot | 50,000 |
| Relations per snapshot | 100,000 |
| Properties per node | 512 |
| Serialized property | 1 MiB |
| Serialized property value | 768 KiB |
| Semantic string | 512 KiB |

The charged byte count is the exact bounded JSON serialization size of the
retained cache payload: scope, source ID, revision, `NavigationEnvelope`, and
its private relation index. A counting writer rejects a write before the limit
is exceeded and never builds a second unbounded serialized buffer. Cache byte
addition, subtraction, cursor conversion, and page-end calculation use checked
arithmetic.

## Behavior

- A ready `meta.info` response is issued only after its exact full semantic
  snapshot has been admitted. Count, property, string, or byte rejection returns
  the standard seven-key unavailable envelope with `resource_limit`, no root,
  nodes, relations, object references, cursor, or fallback.
- Retention validates property count, property/value serialized size, nested
  strings in type sets and structures, object references, diagnostics, and
  relation/snapshot semantic payloads before caching.
- Cursor HMAC authenticates typed claims: source, revision, owner target, role,
  kind, group key, next position, and the canonical `selectionHash`. The raw
  bounded selection is normalized only after authentication and must match that
  authenticated hash. A resume then materializes only the cursor-bound `(owner,
  role, kind, groupRef)` page. A missing or mismatched retained group is
  `snapshot_stale` and remains a path-free unavailable result.
- An evicted cursor fails closed with `snapshot_stale`; it never falls back to a
  live source read. `source_unavailable` remains reserved for bootstrap/current
  source-map or authorization failures.

## RED/GREEN Regression Counts

RED coverage added: 5 focused regression tests.

1. A real 5,000-attribute graph issues an initial cursor and returns the stable
   next page.
2. A per-snapshot byte rejection returns the exact public unavailable envelope.
3. A low-node oversized property value returns `resource_limit`.
4. Attribute and form cursors independently materialize only their own group;
   a missing group is stale.
5. Many low-node entries respect exact total byte charge, evict FIFO, and leave
   an evicted cursor closed.

GREEN results:

| Command focus | Result |
| --- | ---: |
| `cargo fmt` | passed |
| `navigation::tests` | 16 / 16 |
| `native_operations::meta::tests` | 22 / 22 |
| `native_operations::typed_result::tests` | 1 / 1 |
| `platform_xml::projector::tests` | 19 / 19 |
| `source_adapters::certification` | 4 / 4 |

## Concern Outside This Change

`application::tests` consistently reports 90 / 96. The six failures are
existing support/lock assertion messages in `application/mod.rs`:

- `cf_info_reports_configuration_support_state_from_parent_configurations_bin`
- `code_patch_apply_is_blocked_for_a_locked_supported_object`
- `mutating_cf_edit_blocks_locked_configuration_directory_target`
- `mutating_meta_edit_blocks_locked_vendor_object_by_default`
- `mutating_meta_remove_blocks_supported_object_until_off_support`
- `support_edit_set_requires_global_capability_on`

They reproduce with `--test-threads=1`, do not touch the changed navigation
files, and are intentionally not repaired in this findings-3/6/7 task.

## Fix Round 1

### Stabilized admission and retention

- `CachedNavigation::new` now runs all count, semantic-string, property/value, depth, and bounded serialized-byte preflight checks against `&NavigationEnvelope`. It creates the retained `Arc<NavigationEnvelope>` only after those checks succeed, so a rejected graph is never deep-cloned.
- Cache lookup returns the retained `Arc`; page materialization reads that snapshot and copies only the selected target/page nodes. The private relation index is allocated for the response only after successful admission/preflight.
- `PropertyValue` validation is iterative and covers `List`, `Structure`, and nested `TypeSet` primitive qualifier values. `SNAPSHOT_CACHE_MAX_PROPERTY_VALUE_DEPTH = 64`; depth `65` is a typed `resource_limit`, with no recursive traversal or panic.
- `MAX_SEMANTIC_STRING_BYTES = 512 KiB` is once again an individual-string constraint. The accidental whole-node 512 KiB serialization rejection was removed; property (`1 MiB`), property value (`768 KiB`), and snapshot (`32 MiB`) budgets govern aggregate retained payload size.

### Exact process-lifetime FIFO cache limits

| Limit | Value |
| --- | ---: |
| Entry count | 64 |
| Per snapshot serialized charge | 32 MiB |
| Total serialized charge | 128 MiB |
| Nodes per snapshot | 50,000 |
| Relations per snapshot | 100,000 |
| Properties per node | 512 |
| Serialized property | 1 MiB |
| Serialized property value | 768 KiB |
| Individual semantic string | 512 KiB |
| `PropertyValue` nesting depth | 64 |

### RED/GREEN evidence

- RED: the first targeted application rerun reached `mutating_cf_edit_blocks_locked_configuration_directory_target` and failed `1/3`: a previously synthetic record layout was first rejected as duplicate evidence, then the supplied configuration state `1` correctly made the configuration editable rather than locked.
- GREEN: the shared helper now emits the certified counted v6 sequence: configuration `1,0,<uuid>`; locked object `0,0,<uuid>,<uuid>`; removed object `2,0,<uuid>,<uuid>`. The configuration-directory lock case deliberately uses the valid state-`0` configuration variant, so its existing lock assertion is preserved.
- GREEN: targeted application regressions `6/6`; full `application::tests` `96/96`.
- GREEN: navigation `16/16`; meta `25/25`; typed_result `1/1`; projector `19/19`; certification `4/4`.
- GREEN: `cargo fmt` completed.

### Added regressions

- A real 5,000-attribute graph still admits, issues a cursor, and resumes its next page.
- A depth-65 value returns public `resource_limit` unavailable without a panic.
- A structured value of roughly 600 KiB, composed of 600 individual 1 KiB strings, admits under the 768 KiB value and 1 MiB property budgets.
- An individual semantic string larger than 512 KiB rejects retention as `resource_limit`.

## Fix Round 2

### Findings closed

- Structural validation of every `PropertyValue` now completes before any `serde_json::to_writer` call for its enclosing `SemanticProperty` or for the value itself. Property/value byte measurements therefore happen only after string, depth, and nested-shape validation has succeeded.
- The former width-proportional `Vec<(&PropertyValue, usize)>` is replaced by a bounded recursive visitor. It checks child depth immediately before descent, has at most 65 frames for the documented maximum depth 64, and visits `List`, `Structure`, and `TypeSet` qualifier siblings sequentially with O(depth) auxiliary memory.
- `SNAPSHOT_CACHE_MAX_SEMANTIC_STRING_BYTES = 512 KiB` is now used only for individual strings. `SourceSnapshot` is checked field-by-field (`source_id`, `revision`, `adapter_id`); object references, property fields/values, relations, diagnostics, and schema version are likewise individually checked. No aggregate struct is serialized against that limit. Aggregate admission remains governed by the 1 MiB property, 768 KiB value, and 32 MiB snapshot counting-writer budgets.

### RED/GREEN evidence

- RED findings confirmed: `3/3`. The pre-fix audit found one snapshot aggregate measurement at 512 KiB, property serialization before nested-value validation, and a validation stack proportional to sibling width.
- RED compile feedback: `1` local gap, `SourceRevision` had no string accessor for direct individual validation. A read-only `as_str()` accessor resolved it without altering source capture/support behavior.
- GREEN retained regressions: depth-65 returns `resource_limit` unavailable without panic; an individual string above 512 KiB rejects retention.
- GREEN added regressions: a 100,000-item shallow `null` list admits under existing property/value budgets, and aggregate snapshot adapter metadata plus two diagnostics above 512 KiB admits when every individual string is 300 KiB and total charge is below 32 MiB.
- GREEN results: `cargo fmt`; navigation `16/16`; meta `27/27`; typed_result `1/1`; projector `19/19`; certification `4/4`; application `96/96`.

## Fix Round 3

### Stabilization changes

- Cache preflight now validates both `navigation.nodes` and every `navigation.relations[*].items` node before any whole-envelope serialization. Relation-page group references, cursors, and page fields are validated in the same pass.
- The retained navigation domain is covered with explicit exhaustive enum validators: object-reference kinds and display data; node/relation refs and roles; capabilities and action profiles; semantic descriptors, operation bindings, availability reasons and atomicity; property types, values, TypeSet references/enums/qualifiers; source snapshot fields; cursor fields; and diagnostics.
- `SourceAdapterDiagnostic.details` is an optional JSON value. It is omitted on the wire when absent, while present JSON keys and strings are individually bounded and recursively depth-checked before serialization.
- Individual semantic strings are capped at `524288` bytes (512 KiB). All nested `PropertyValue`, TypeSet qualifier, and diagnostic JSON traversal has a maximum active nesting depth of `64`; traversal is recursive with at most 65 call frames and loops siblings sequentially, so it has O(depth) auxiliary memory and no width-proportional work stack.
- Serialization remains after structural validation only. The whole retained snapshot remains bounded by the existing `33554432` byte (32 MiB) counting-writer budget; aggregate valid fields are not subject to the individual-string limit.

### RED/GREEN evidence

- RED: the initial relation-page-only depth test selected a root node without properties and panicked while constructing its fixture. The fixture now selects a page item with an actual semantic property; this preserves the intended page-only preflight assertion.
- GREEN: relation-page-only depth 65, oversized ObjectRef kind payload, oversized action operation binding, and oversized diagnostic JSON detail all return typed `resource_limit` before serialization.
- GREEN: a `SourceSnapshot` with `adapter_id` and revision strings of 300 KiB each is larger than 512 KiB in aggregate, below 32 MiB, and is admitted. An individual string over 512 KiB remains rejected.
- GREEN: the test-only observer records maximum active validation depth `2` for 100,000 shallow list siblings, proving sibling width does not become retained traversal state.
- GREEN test counts: `domain::navigation` 16/16; `native_operations::meta` 31/31; `typed_result` 1/1; `platform_xml::projector` 19/19; `certification` 4/4; `application::tests` 96/96.

## Fix Round 4

### Diagnostic and continuation bounds

- Diagnostic `details` is structurally validated before any serde measurement, then bounded by a counting writer at `262144` bytes (256 KiB) per value. The fully serialized diagnostics array, including JSON delimiters, is bounded at `1048576` bytes (1 MiB) per retained snapshot.
- Individual strings in diagnostics remain subject to the `524288` byte semantic-string limit; nested JSON remains depth-limited to `64` with O(depth) recursion.
- Only an initial `ObjectPath` materialization copies the already-admitted diagnostic vector. `objectRef` and cursor continuations pass an empty diagnostic vector unless a future continuation-specific diagnostic is explicitly constructed, so cached diagnostic details are not deep-cloned per page.

### Cache-key and validation changes

- `CachedNavigation::new` now validates the charged outer `scope`, `source_id`, and `revision` before constructing `CachedNavigationCharge` or invoking serde. Each is capped at `524288` bytes; source ID and revision are reconstructed through their newtypes to enforce their exact nonempty/control-character invariants.
- Retained `SourceSnapshot.adapter_id` now uses the same nonempty/control-character and individual-string validation as cache metadata.
- `NavigationValidationStats` and every stats update are `#[cfg(test)]`. Production property traversal has the simple `(value, limits, depth)` signature and no max-depth bookkeeping; the test-only observer is separate and O(depth).

### RED/GREEN evidence

- RED: the first compile after cache-key validation used a dynamic message with the static `resource_limit` helper and failed with one `E0308`; it was corrected without changing error kind or exposing metadata values.
- GREEN: a near-256 KiB details payload is retained for the initial response, while real `objectRef` and cursor continuations return `diagnostics: []`. A single over-limit details value and five individually valid diagnostics over the 1 MiB vector limit return typed `resource_limit` before cache serde.
- GREEN: direct constructor tests reject oversized scope, source ID, revision, and adapter ID before charging/serialization.
- GREEN test counts: `domain::navigation` 16/16; `native_operations::meta` 34/34; `typed_result` 1/1; `platform_xml::projector` 19/19; `certification` 4/4; `application::tests` 96/96.
