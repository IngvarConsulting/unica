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

## Fix Round 6 authority boundaries

The 62 enum source contexts are extracted from the legacy AST. Emitter owners
come from the legacy `property_emitters` dispatch table, field owners come from
the `get_attributes` call graph, and every owner/property pair must be observed
in a declared input with a generated raw legacy output. The spreadsheet
document template context has its own real descriptor and output.

`new-only-contract-source.json` preserves the independently reviewed Fix Round
5 inventory. `tools/build_new_only_contract.py` combines that inventory with
native fixtures and explicit closed public-contract rules to generate
`new-only-contract.json`. It does not import or invoke the adapter or Rust
normalizer. The generated v2 contract covers the complete normalized public
envelope and semantic-relation shapes, including all capability fields,
property provenance, actions, facets, diagnostics, backing evidence, rights
conditions/templates, support states, and opaque identities.

The generator writes this independently built contract and hashes its builder,
source inventory, inputs, output, enum extractor/artifact, rights crosswalk,
legacy scripts, raw outputs, and semantic oracle. No expected data is captured
from a `NavigationEnvelope`.

Regenerate and verify:

```sh
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --write
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --check
python3.12 crates/unica-adapter-platform-xml/tests/fixtures/v2_20/legacy-oracle/tools/generate_oracle.py --repo-root . --self-test
```

## Fix Round 7 evidence boundaries

Legacy output keeps spreadsheet-document templates under the generic
`Template` owner. `SpreadsheetDocument` is evidence for `template.type` only;
neither extraction nor the semantic adapter may derive a different owner from
that value.

`enum-alias-executions.json` records 174 independently generated executions:
every source-extracted native enum alias is inserted into a context-valid
fixture, run through the tracked legacy script, classified line by line, and
then decoded by the adapter test. `MultiTargetReader` similarly contains one
independently decoded rights target for every one of the 45 supported prefixes.
Both inventories are exact and fail on omissions, duplicates, context changes,
or raw-output hash drift.

`full-public-contract-specimen.json` is a static, hand-reviewed specimen of the
complete public JSON shape. It covers nonempty relation pages, recursive item
facets, opaque cursors, semantic action descriptors, operation bindings, and
all closed action variants. It is not generated from adapter output. The
Platform XML adapter intentionally produces none of those relation-page or
action/binding variants; focused tests assert that absence and the blocked
relation-selection behavior separately.

The provenance manifest hashes the execution inventory, all 45-target rights
inputs and outputs, the static public-contract specimen, extraction/generation
code, legacy scripts, and every resulting oracle artifact.
