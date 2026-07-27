# Platform XML 2.20 legacy parity oracle

This directory contains only evidence produced from the tracked legacy
`meta-info.py` and `role-info.py` tools plus the independently reviewed
`crosswalk.json`. It must not be generated from a `NavigationEnvelope`.

Regenerate raw legacy outputs, extracted semantic facts, enum coverage, and the
provenance manifest from the repository root:

```sh
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --write
```

Re-run the same legacy commands in a temporary directory and verify every
checked-in byte and SHA-256:

```sh
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --check
```

The generator imports only Python standard-library modules. `inputs.json`
declares every reference source and fixture. `oracle-manifest.json` hashes the
generator, crosswalk, input declaration, every reference source, every input
fixture, every raw legacy output, and the resulting
`legacy-semantic-oracle.json`.
## Fix Round 5 evidence boundaries

`generate_oracle.py` classifies every legacy output line with its 1-based line
number. A line is accepted only as a useful semantic fact, an enumerated
structural header/delimiter, or a blank separator. Unknown headings, unknown
values, duplicate fields, bad indentation, inconsistent section counts, stale
rights-group state, and unhandled rights prefixes abort generation.

Native enum property and owner contexts come from
`tools/extract_enum_contexts.py`, which analyzes the frozen legacy Python ASTs
and descriptor fixtures. `crosswalk.json` contains semantic IDs only and is
rejected if it attempts to supply `nativeProperty` or `objectKinds`.
`rights-target-crosswalk.json` is the independent, fail-closed prefix map.

`new-only-contract.json` is hand reviewed and is never written by the
generator. It covers adapter-only status, identities, property value/type/state,
relation and node coverage, complete facet membership, backing evidence, and
the full diagnostic multiset.

Regenerate and verify:

```sh
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --write
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --check
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --self-test
```
