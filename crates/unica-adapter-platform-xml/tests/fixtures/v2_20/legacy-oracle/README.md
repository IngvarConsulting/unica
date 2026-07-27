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
