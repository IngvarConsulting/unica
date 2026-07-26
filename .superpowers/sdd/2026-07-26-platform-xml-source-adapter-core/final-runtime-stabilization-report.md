# Final runtime-boundary stabilization

## Shared limits

| Boundary | Limit |
|---|---:|
| Native and semantic nodes | 50,000 |
| Native and semantic relations | 100,000 |
| Properties per node | 512 |
| Type variants per type set | 512 |
| Identity-bearing items | 1,000,000 |
| Native/property/JSON nesting | 64 |
| Public property selectors | 256 |
| Public relation selectors | 64 |
| Selector string bytes | 256 |
| Cursor field string bytes | 1 KiB |
| `select` JSON bytes | 128 KiB |
| Cursor JSON bytes | 128 KiB |

Cache property, value, semantic-string, and diagnostic byte limits retain their existing values and now source them from `domain::navigation_limits`.

## Evidence and counters

- Decoder charges native node, relation, property, UUID/child identity, type-variant, and active-depth counters before constructing or inserting the corresponding item. Its test context records native-node/property counts; a separate XML-depth preflight rejects a child at depth 65 before recursion.
- Projector recursively preflights the native snapshot before creating semantic maps or vectors, then charges semantic node, relation, property, and identity counters before each output insertion.
- Cache retains `Arc<[SemanticRelation]>` and charges that relation index once; response materialization shares the index in O(1) and scans only the selected page range instead of cloning the matching edge set.
- Public `select` and cursor inputs receive iterative depth/string/byte preflight. Cursor HMAC covers the bounded raw selection before selection parsing or normalization.
- The exact identifier grammar permits ASCII and Russian Cyrillic identifiers only; decoder names and TypeReference/Enumeration QName targets use that single domain implementation.

## Regression coverage

- Decoder: 50,001 inline children and 513 properties fail as `resource_limit`; the context never exceeds 50,000 constructed native nodes; a 49,999-node descriptor remains within the provider file bound and projects to exactly 50,000 semantic nodes; 65 nested native nodes fail before deeper stack growth.
- Schema and decoder: matching valid Cyrillic/ASCII names are accepted; unsupported-script names are rejected in both paths.
- Navigation/meta/application: maximum selectors are accepted, max-plus-one arrays and JSON bytes are `resource_limit`, unauthenticated/oversized cursors fail before normalization, and duplicate relation normalization remains canonical.
- Cache: retained-object misses are `snapshot_stale` and continuation responses share the private relation-index allocation.

## Verification evidence

- `cargo fmt` completed successfully.
- Focused Rust filters completed successfully for decoder (31 tests), schema, projector, domain navigation, native `meta` (41 tests), typed result, application contract, and certification (5 tests).
- `python3.12 tests/ci/test_unica_skills.py` completed successfully: 32 tests passed.
