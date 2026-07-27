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

## Fix Round 1

Base: `9e8bb83a12d1957453b56c87ae8a824aea1d4ef8`.

Implementation commits:

- `71f366d95f419ccc736895b2042bf36192aee7cc` (`fix: make platform xml coverage authoritative`)
- `6463ec51efc10a79f47af0da204690606280203b` (`test: freeze platform xml legacy fact parity`)

The report and completion ledger are committed separately because a commit cannot contain its own SHA and existing commits must not be amended.

### Reviewer findings resolved

1. **Authoritative non-drifting coverage.** `coverage.json` is now the single serialized source consumed into a typed, immutable 2.20 registry. Decoder, schema, probe, projector, owner detection, and adapter profile metadata all query that registry. Runtime validation rejects duplicate or missing object mappings, per-kind/generic property drift, relation-property drift, overlapping or unused child rules, enum/type alias drift, backing-kind disagreement, and unreferenced intentional-partial cases. A runtime mapping cannot exist outside the registry because the former Rust mapping tables and legacy top-level string constant were removed.
2. **Role rights.** Domain-neutral access object kinds, properties, relations, facets, and validated value types were added to core. The adapter reads `Ext/Rights.xml`, preserves defaults, each target, permission name/value, all readable restriction/condition leaves, restriction templates, and backing availability. Unknown target classes remain readable `unknown` references and force neutral partial diagnostics. A role with absent rights backing is never complete.
3. **Type sets.** The closed type model now distinguishes UUID, opaque storage, table values, null, references, objects, record sets, managers, keys, enumerations, defined types, and unknown variants. The registry inventories XML Schema, data-core, and current-configuration aliases, including subscription object aliases and all supported register/manager/key families. String, number, and date qualifier groups can coexist and apply to their matching primitive variants. Unknown official aliases remain in the type set as `unknown` and force partial coverage.
4. **Hierarchy semantics.** `Hierarchical`, `HierarchyType`, `LimitLevelCount`, and `LevelCount` are explicit closed facts. The active hierarchy level limit is computed only when both hierarchy and level limiting are true; otherwise it is explicitly absent while configured controls/count remain visible.
5. **Unknown readable facts.** Structurally valid unknown roots and children probe and decode as closed `unknown` objects. Unknown children use the neutral `unknown` relation. Unknown property values, reference targets, type variants, and backing files remain readable through neutral structures without native class/property/file labels in output diagnostics.
6. **Forms/templates.** Common and object-owned forms/templates retain descriptor identity, `FormType`/`TemplateType`, descriptor availability/UUID, content availability, and opaque-content state. Valid opaque content remains intentionally partial; validation alone no longer implies semantic completeness.
7. **Independent parity.** The old inline synthetic snapshots were replaced by tracked 2.20 corpus files plus frozen semantic fact inventories. Tests compare information sets. The real BSP corpus assertions cover identity and useful facts for catalogs, common modules, documents, enumerations, information registers, languages, and reports. A valid tracked configuration registration corpus covers every supported top-level object kind. Separate adversarial fixtures cover rights, backed/common forms and templates, hierarchy controls, all type categories, subscription aliases, and all unknown root/child/property/relation/value/backing paths. Frozen inventories independently pin every known enum semantic and every registered type alias/category/target.

### Files

Adapter runtime:

- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/owner.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/native_model.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/probe.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`

Domain-neutral core:

- `crates/unica-format-core/src/facets.rs`
- `crates/unica-format-core/src/property.rs`
- `crates/unica-format-core/src/semantic_ids.rs`
- `crates/unica-format-core/src/value.rs`
- `crates/unica-format-core/tests/semantic_registry.rs`

Task-scoped tests and tracked corpus:

- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- `crates/unica-adapter-platform-xml/tests/unmapped_fact.rs`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/all_kinds/Configuration.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/artifacts/ArtifactReport.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/artifacts/ArtifactReport/Forms/MainForm.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/artifacts/ArtifactReport/Forms/MainForm/Ext/Form.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/artifacts/ArtifactReport/Templates/MainSchema.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/artifacts/ArtifactReport/Templates/MainSchema/Ext/Template.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/common_form/CommonDashboard.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/common_form/CommonDashboard/Ext/Form.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/common_template/CommonLayout.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/common_template/CommonLayout/Ext/Template.bin`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/expected-semantic-facts.json`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/hierarchy/DisabledContradiction.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/hierarchy/EnabledLimited.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/hierarchy/EnabledUnlimited.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/rights/SalesReader.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/types/AllTypes.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/types/EventSources.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/unknown_root/Mystery.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/unknowns/UnknownCases.xml`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/unknowns/UnknownCases/Ext/Future.bin`

SDD artifacts:

- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md`
- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/task-5-report.md`

### RED evidence

The Task 5 scoped command was used throughout:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Initial RED after writing the fix-round tests: exit 101. The tests referenced missing closed hierarchy/access/backing/unknown IDs, object kinds, enum symbols, and type variants. Three test-harness compile defects were corrected without adding behavior; the command remained RED on the missing production semantics.

First integration checkpoint: exit 101 because replacing the independent class constant exposed two consumers (`owner.rs` and `v2_20/mod.rs`) that had not yet been moved to the typed registry. They were changed to query the same registry rather than restoring a second list.

Behavior checkpoint: exit 101. Rights and form/template tests passed, while hierarchy fixtures exposed an invalid capability on an absent computed property and the complete type fixture exposed `ValueStorage`/`ValueTable` collapsing into one variant. The fixes kept inactive hierarchy limits absent and introduced the distinct domain-neutral table type.

Independent real-corpus inventory checkpoint: exit 101 with 6 parity tests passing and the real-corpus information-set test failing on its string canonicalization helper. The helper was corrected; production behavior was unchanged.

### Final GREEN evidence

Command:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Result: exit 0, no warnings.

```text
legacy_parity: 7 passed; 0 failed
unmapped_fact: 6 passed; 0 failed
```

Only the Task 5 scoped validation prescribed by the plan was run. No workspace-wide test, lint, format, or unrelated validation command was run.

### Parity inventory

- Every registered native top-level class has a closed object kind and is exercised through a tracked valid 2.20 configuration registration.
- Real tracked BSP facts: catalog hierarchy/code/description and specialized children; common-module execution flags; document numbering/posting and attributes/tabular sections/forms/templates; enumeration comments/values; information-register periodicity/write mode/dimensions; language identity; report data-composition-schema and backed template facts.
- Rights: three defaults, two object targets, six permission values, condition text, one restriction template, and backing availability from the real `SalesReader/Ext/Rights.xml`.
- Type aliases: XML primitives, `UUID`, `ValueStorage`, `ValueTable`, `Null`, all registered reference/object/record-set/manager/key aliases, enumeration/defined-type aliases, subscription source-object aliases, and simultaneous string/number/date qualifiers.
- Hierarchy adversaries: enabled/unlimited, enabled/limited, and disabled with contradictory configured limit controls, plus the real `Валюты.xml` case.
- Artifacts: object-owned and common forms/templates, descriptor UUID, form/template type, content availability, and explicit opaque content.
- Unknowns: root class, child class/relation, direct and nested property values, reference target, type alias, and backing file.
- Registry audit: object profiles, generic/per-kind properties, relation properties, child mappings, enum aliases, type aliases, backing artifacts, and intentional partial cases.

### Intentional gaps and concerns

- Form and template content internals remain opaque. All readable descriptor/type/backing facts are retained and the result is partial, as required; no unsupported semantic decomposition is fabricated.
- Future official aliases/classes/properties retain readable neutral values and partial diagnostics, but their private native labels are intentionally not exposed through core output.
- The checked-in BSP corpus has full real descriptors for seven top-level kinds. The remaining supported top-level kinds are exercised by a tracked structurally valid 2.20 registration corpus and the registry bijection audit, not by full BSP descriptors. This is a corpus-availability limitation, not a known loss of a legacy baseline fact.
- No known fact exercised by the approved legacy useful-information baseline remains silently omitted.

## Fix Round 2

Base: `d7fb6a785f872972a550243b71890d7a14e7d305`.

Implementation commit:

- `bb3c34e337dd350a8605f2c5d7ccc11d2e54ae5e` (`fix: complete platform xml task 5 parity`)

The report and completion ledger are committed separately so this report can
record the implementation SHA without amending an existing commit.

### Reviewer findings resolved

1. **Registry authority is complete.** `relationProperties` is a mandatory
   nonempty registry section. Property, relation, enum, backing, and
   intentional-partial applicability and alias arrays are validated. A
   candidate manifest is parsed into the typed registry and compared exactly
   with the immutable runtime registry. Mutation tests reject removed and extra
   properties, relation properties, aliases, owner roles, backing kinds,
   intentional-partial rules, empty applicability/alias arrays, and every
   empty mandatory section. Decoder and projector property/relation/type/enum
   dispatch use registry entries. Both inline and registered backing dispatch
   use the typed `BackingKind`. Intentional-partial reasons are a closed enum
   iterated from the registry through one exhaustive projector match.
2. **Empty references are lossless.** Core now has the domain-neutral
   `PropertyValue::EmptyReference` and `PropertyType::EmptyReference`.
   Serialization is payload-free as `emptyReference`, strict deserialization
   rejects a payload, and the value remains distinct from absent, unresolved,
   and null. The private 2.20 decoder recognizes the tracked
   `DesignTimeRef`/`EmptyRef` encoding in `Валюты.xml`. Unknown readable
   design-time references retain target evidence in a neutral structure and
   force partial coverage without exposing the native QName or class.
3. **Enum coverage is exhaustive and applicable.** Every closed semantic enum
   has nonempty native aliases and explicit semantic-property applicability.
   The frozen inventory includes the prior 39 symbols plus the legacy
   `WholeCatalog`, `ForFolder`, and `ForFolderAndItem` cases and the real-corpus
   `AutoDeleteOnUnpost` and `WriteSelected` cases. Runtime lookup requires both
   property ID and alias, preventing a valid alias for one property from being
   accepted on another. Unknown aliases remain unresolved, diagnostic, and
   partial.
4. **Rights parsing fails closed.** The adapter now audits attributes and
   direct children at the rights root, object, permission, restriction,
   condition, template, and scalar levels. Only explicit known elements can
   create restriction conditions. Unknown attributes, elements, malformed
   known scalars, nested conditions, and future values remain readable through
   neutral evidence and force partial coverage. Known targets, permission
   names/values, conditions, templates, defaults, and backing availability
   remain intact.
5. **Unknown children are readable for every owner role.** Unknown child
   classes route through the neutral unknown mapping even when the registered
   owner has no child vocabulary. Registered children forbidden for such an
   owner still fail as structural corruption. Unknown child occurrences carry
   neutral positional evidence and use occurrence-qualified derived identity,
   so duplicate native class/name occurrences do not collide or deduplicate.
   Matrix tests cover attribute, form, and template owners.
6. **Parity is exact and independent.** Real-fixture tests now compare exact
   normalized `BTreeSet` equality rather than `contains` subsets. The frozen
   inventories are external to the runtime registry and include every
   non-infrastructure legacy-baseline node, mapped property value/state,
   relation kind/role, envelope status, node coverage count, and diagnostic
   code count. Exact inventories cover seven tracked BSP descriptors, every
   supported top-level kind, rights, object-owned artifacts, common forms, and
   common templates. Unknown type variants carry positive neutral ordinals in
   the closed wire form; two distinct future aliases remain two distinct,
   payload-bearing unknown variants without native alias leakage.

### Files

Adapter runtime:

- `crates/unica-adapter-platform-xml/src/factory.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/coverage.json`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/decoder.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/mod.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/native_model.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/projector.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/schema.rs`
- `crates/unica-adapter-platform-xml/src/versions/v2_20/semantic_map.rs`

Domain-neutral core:

- `crates/unica-format-core/src/property.rs`
- `crates/unica-format-core/src/semantic_ids.rs`
- `crates/unica-format-core/src/value.rs`

Task-scoped tests and frozen inventories:

- `crates/unica-adapter-platform-xml/tests/legacy_parity.rs`
- `crates/unica-adapter-platform-xml/tests/unmapped_fact.rs`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/expected-semantic-facts.json`
- `crates/unica-adapter-platform-xml/tests/fixtures/v2_20/unknowns/UnknownCases.xml`

SDD artifacts:

- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/progress.md`
- `.superpowers/sdd/2026-07-26-versioned-format-adapter-boundary/task-5-report.md`

### RED evidence

All TDD runs used only the Task 5 command prescribed by the plan:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Focused-contract RED: exit 101. Compilation failed on seven intentionally
missing contracts: the coverage-candidate validator, `EmptyReference`, and the
three new closed semantic enum IDs referenced by tests. No production behavior
was added before this run.

Implementation checkpoint: exit 101 on one local decoder resource-limit helper
name. The call was changed to the existing typed adapter error constructor.

Focused-contract GREEN: exit 0:

```text
legacy_parity: 10 passed; 0 failed
unmapped_fact: 8 passed; 0 failed
```

Exact-parity RED: exit 101 with four intended failures. Rights, supported-kind,
form/template, and real-fixture tests still had empty/subset frozen inventories.
The failure output exposed the complete normalized information sets. The
normalizer was then fixed independently to exclude only infrastructure support
facts and non-baseline `unknown.facts` payloads while retaining partial status
and diagnostic counts.

### Final GREEN evidence

Command:

```text
cargo test -p unica-adapter-platform-xml --test legacy_parity --test unmapped_fact
```

Result: exit 0, no warnings.

```text
legacy_parity: 10 passed; 0 failed; 0 ignored
unmapped_fact: 8 passed; 0 failed; 0 ignored
```

Only this Task 5 scoped validation command was run. No workspace-wide tests,
lint, format, or unrelated validation command was run.

### Exact parity and coverage inventory

- 66 typed object profiles and an exact 46-kind tracked corpus inventory
  including the configuration root and all 45 supported top-level children.
- 91 property mappings, two relation-property mappings, 19 child rules, 44
  property-applicable enum symbols, 60 type aliases, three backing kinds, and
  three typed intentional-partial reasons.
- Seven real BSP descriptors: catalog, common module, document, enumeration,
  information register, language, and report. Their complete normalized
  baseline facts, statuses, coverage counts, and diagnostics are frozen.
- Real `Валюты.xml`: hierarchy controls and conditional active limit,
  specialized attributes/tabular sections/forms, qualifiers, fill flags, and
  the exact empty-reference fill value.
- Rights: three defaults, two targets, six permissions and values, one
  permission condition, one restriction template condition, and backing
  availability. Future attributes/elements/nested conditions are retained
  separately and cannot become known restrictions.
- Forms/templates: descriptor identity, form/template type, descriptor/content
  availability, opaque state, owned and common ownership relations, and
  partial coverage.
- Types: all registered primitive/reference/object/record-set/manager/key/
  enumeration/defined-type aliases, qualifier combinations, subscription
  aliases, and distinct neutral ordinals for multiple unknown variants.
- Unknowns: readable root, child, duplicate occurrence, property, reference
  relation, design-time reference value, type variants, and backing file;
  owner-role matrix coverage and format-neutral diagnostics.

### Intentional gaps and concerns

- Form and template content internals remain opaque. Descriptor/type/backing
  facts are complete, and opaque content explicitly remains partial.
- Real BSP descriptors contain many readable native facts outside the approved
  legacy useful-information baseline. They are retained as neutral
  `unknown.facts`, counted in exact partial/diagnostic contracts, and are not
  silently omitted. Their native labels remain private to the adapter.
- Future aliases/classes/rights syntax remain readable and partial until a
  closed semantic mapping is added.
- The exact parity normalizer intentionally excludes derived support properties
  and raw `unknown.facts` payloads because neither belongs to the independent
  legacy useful-information baseline; it still freezes their aggregate
  coverage and diagnostic effects.
- No fact in the tracked legacy useful-information baseline remains unable to
  meet semantic parity.
