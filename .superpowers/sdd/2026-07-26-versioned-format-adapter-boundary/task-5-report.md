# Task 5 implementation report

## Status

Task 5 is complete. Platform XML 2.20 now owns an explicit semantic mapping and coverage inventory, projects the useful legacy `Mode=full` and drill-down facts into the closed core vocabulary, and reports meaningful unmapped native facts as `partial` with format-neutral diagnostics instead of silently dropping them.

Implementation commit: `2d5d7aa7d20a056c3124dca6f11b0c40849d9c6d` (`feat: complete platform xml semantic projection`).

The report cannot contain the SHA of the commit that contains the report itself. To satisfy the no-amend constraint, the implementation was committed first and this report plus the progress update are committed separately.

## Changed implementation files

- `crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json`: version-owned, auditable inventory of supported core object kinds, properties, relations, facets, read selections, known partial areas, and parity fixtures.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`: exhaustive Platform XML 2.20 native class/property/role/value mapping into closed core identifiers.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs`: registers the private version-owned semantic map.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`: expands 2.20 native class profiles and keeps native type/property knowledge inside the adapter version.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/native_model.rs`: preserves localized values, lists, structured references, nulls, dates, and unmapped-fact counts until projection.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`: decodes mapped properties, localized values, type sets, reference relations, and known unsupported facts without silent loss.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`: emits typed properties, semantic relations, support/identity facts, derived field requirements, partial status, and neutral diagnostics.
- `crates/unica-adapter-platform-xml/src/versions/v2_20/provider.rs`: embeds and exposes the adapter coverage manifest.
- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`: public-boundary coverage and useful-information parity tests.
- `crates/unica-adapter-platform-xml/tests/unmapped_fact.rs`: public-boundary regression test for partial status and neutral diagnostics.

No core, application, coder, package metadata, or non-Task-5 code was changed.

## Design decisions

- The mapping authority is `versions/v2_20/semantic_map.rs`; XML class names, property names, role names, aliases, and native value kinds do not cross the adapter boundary.
- `coverage.json` contains only closed core vocabulary identifiers. A test parses every listed object, property, relation, and facet identifier through the public core parsers and rejects native XML vocabulary leakage.
- Decoder output preserves native lexical information in private native-model types. Semantic typing happens in projection, so failed boolean, integer, UUID, date, enum, and type-set normalization becomes unresolved/partial rather than a hard decode failure or fabricated value.
- Localized strings remain localized values, structured reference properties become semantic relations, and child native classes become closed relation roles and object kinds.
- Identity and support facts that were implicit in the native envelope are projected as explicit closed properties, including metadata kind/UUID and effective support/authorability/edit capability.
- Field `required` is derived from the mapped fill-checking rule, matching the useful legacy drill-down behavior without preserving legacy text shape.
- Every unknown direct property and every recognized-but-unsupported child role increments the unmapped-fact inventory. Projection emits one format-neutral unmapped diagnostic per fact and forces readable output to `partial`.
- Native names are intentionally absent from diagnostics. The diagnostic contract says that a meaningful fact was unmapped; adapter-internal names remain private.
- No legacy prose or `Mode=full` text compatibility was retained; parity is semantic and structural.

## Legacy useful-information parity inventory

- Identity: object kind, name, UUID, localized synonym, localized comment, object presentation, and list presentation.
- Support: effective support state, authorability, and edit capability.
- Documents: number type, number length, number periodicity, autonumbering, posting, `BasedOn`, and `RegisterRecords` references.
- Catalogs: hierarchy type, owners, code/description types and lengths, and input length limits.
- Registers: dimensions, resources, information-register periodicity, and accumulation-register type.
- Constants and defined types: ordered closed type sets, including native configuration references represented as semantic kinds.
- Reports: main data-composition-schema reference.
- Common modules: global, client/server/external-connection/ordinary-application flags, privileged mode, return-value reuse, and server-call mode.
- Scheduled jobs: method name, predefined flag, and restart count/interval.
- Event subscriptions: event, handler, and source type set.
- HTTP services: URL templates and methods with semantic child roles.
- Web services: namespace, reuse policy, operations, parameters, direction, and parameter types.
- Enumerations: enumeration values as semantic children.
- Fields: attributes, dimensions, resources, type sets, localized captions/tooltips, fill checking/value, indexing, full-text-search and master-data flags, and derived requiredness.
- Tabular sections: columns use the closed `column` role rather than the generic field role.
- Forms, templates, and commands: semantic object kinds and closed child roles are retained for full-object and drill-down navigation.

## Intentional known gaps

- Form and template payload internals remain opaque backing content. Their objects and relations are projected, but the adapter does not invent a semantic decomposition that the closed core vocabulary does not define.
- Native value variants outside the explicit 2.20 aliases remain unresolved and make the result partial; they are not coerced into a nearby core enum.
- Known accounting-plan child roles such as accounting flags remain recognized but unsupported by the current closed vocabulary. They produce partial status and unmapped diagnostics.
- The coverage guarantee is for the known Platform XML 2.20 useful-information baseline represented by the manifest and parity fixtures, not for arbitrary future native classes or properties.

## TDD and validation evidence

### RED

Command:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Result: exit 101. Compilation succeeded and all three `legacy_parity` tests failed for the intended missing behavior:

- coverage manifest did not exist;
- catalog hierarchy native value remained unresolved;
- document output was `partial` because useful native properties and relations were not mapped.

Cargo stopped after the failing parity binary, so the unmapped-fact binary did not run in that RED invocation.

### First GREEN

The same command passed all tests: 3 parity/coverage tests and 1 unmapped-fact test. The compiler reported three cleanup warnings.

A warning-only cleanup initially renamed the wrong same-named decoder parameter and produced `E0425` at `decoder.rs:202`. The compiler identified the used parameter and the actually unused parameter; the edit was corrected before final validation.

### Final GREEN

Command:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Result: exit 0, no warnings.

```text
legacy_parity: 3 passed; 0 failed
unmapped_fact: 1 passed; 0 failed
```

Only the Task 5 validation command prescribed by the plan was run. No workspace-wide validation was run.
