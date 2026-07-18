# Task 5 adversarial preflight

## Verdict

Task 5 is not ready for an infrastructure-only implementation. The current
application contract still encodes the shortcut that Task 5 is supposed to
remove, and several provider outcomes are underspecified at the Task 4/Task 5
boundary. Starting with XML parsing would create a second incompatible model
and make green tests misleading.

The minimum safe slice is: synchronize the typed application contract first,
then implement the snapshot-bound catalog and providers, then refactor support
parsing and every legacy consumer. No public MCP registration belongs in this
task.

## Confirmed contradictions that must be resolved first

### 1. The accepted spec and product contract still require the wrong exchange edge

- `spec/architecture/extension-point-discovery.md:129-144` requires
  `BindingDetails::ExchangePlan + handles`.
- `docs/superpowers/plans/2026-07-17-project-discovery-receipts.md:282-299`
  repeats the same matrix.
- `tests/ci/test_product_contracts.py:459-517` rejects any matrix without that
  shortcut.
- `model.rs`, `determinism.rs`, and `ports.rs` implement and test the shortcut.
- The Task 5 brief instead requires
  `ExchangePlan --uses--> EventSubscription --subscribes--> Method`.

Required resolution: replace, do not add alongside, `ExchangePlan` details with
`SubscriptionSource`. Preserve one stable tag for the replacement, update both
spec and historical plan, and make the product contract reject the old variant.
Direct exchange-plan callbacks remain unsupported.

Important: a `uses` edge alone must not make an exchange plan runtime-reachable.
The current graph treats every incident `uses` edge as sufficient. The graph
must require the complete two-edge chain to an exact handler before an
ExchangePlan candidate/proposal is connected.

### 2. Snapshot-fatal malformed material and provider-failed material are conflated

The Task 4 contract and implementation parse `Configuration.xml` and every
registered root descriptor to derive the immutable manifest. The active spec
classifies malformed, duplicate, missing, unknown-kind, or identity-mismatched
registered XML as non-retryable `malformed_source_material` during capture
(`extension-point-discovery.md:761-774, 824-836`). The Task 5 brief says all
malformed registered metadata should become a failed provider check.

Both cannot be true for the same bytes. Use this exact split:

- malformed `Configuration.xml`, duplicate/unsafe registration, malformed
  registered root descriptor envelope, missing descriptor, wrong kind, or
  wrong `<Properties><Name>`: Task 4 capture error; no provider runs;
- a well-formed, identity-valid descriptor with missing/invalid
  mechanism-specific fields: the owning Task 5 evidence port is `failed`;
- malformed registered nested `Form/Ext/Form.xml`: `FormInspectionPort` is
  `failed`;
- malformed unregistered decoys: ignored because they are outside the manifest.

Duplicate registration and descriptor-identity tests therefore belong to the
snapshot/use-case boundary, not to a provider test expecting a normal report.

### 3. Raw support facts and proposal policy are still one enum

`ProviderFact::Support` currently carries the public `SupportState`, including
`ExtensionRequired` and `ExtensionOwned`. The graph treats two different values
for one target as conflicting evidence. That makes two proposals with different
mutation intents manufacture a false raw-source conflict.

Introduce a separate closed raw enum, for example:

```text
SupportFactState =
  Editable | Locked | ConfigurationReadOnly | Removed |
  BaseWithoutParentConfigurations | ExtensionWithoutParentConfigurations
```

The support provider emits one raw fact per source artifact. The proposal
validator projects that fact plus the proposal's exact `MutationIntent`,
analysis source kind, and destination identity into the existing public
`SupportState`. `ExtensionRequired` and `ExtensionOwned` must never be emitted
as raw evidence.

The active spec must also define which subset is valid for a candidate that has
no mutation intent. A single `Candidate.supportState` cannot truthfully express
two proposal-specific policies. Recommended rule: candidate state is the raw
direct-mutation projection and never `extension_required`; proposal facts carry
the intent-specific projection.

### 4. Freshness error policy needs a precise split

Current `collect_for_snapshot()` turns a record source-set/fingerprint mismatch
into fatal `ProviderContractViolation`. The Task 5 brief says hash/path mismatch
is retryable unavailable. Do not hide a provider bug as transient I/O. Use this
split:

- `read_verified()` reports live identity/content change: whole provider
  outcome is `unavailable(source_fingerprint_mismatch, retryable=true)` and no
  prefix is promoted;
- a requested source-set that does not equal the captured analysis source-set:
  reject before the first read as
  `unavailable(source_set_mismatch, retryable=true)`;
- a provider manufactures a record with freshness different from the snapshot
  it was given: fatal provider contract violation;
- an expected registered semantic file is absent from the immutable manifest:
  deterministic `failed(registered_material_missing, false)` unless it is the
  versioned optional tombstone read through `read_optional_verified()`.

This distinction needs direct tests with a reader spy proving zero I/O for the
source-set mismatch.

## Required application-model work

### Validated binding facts

Raw public enum variants cannot enforce endpoint compatibility. Replace the
freely constructible binding payload with a smart-constructed `ValidatedBinding`
owned by `ProviderFact`. The constructor must validate relation, supplying
port, subject/object kinds, shared ownership, and bounded semantic fields.

Minimum endpoint rules:

- `EventSubscription`: EventSubscription -> method in a CommonModule,
  relation `subscribes`, exact event and handler;
- `SubscriptionSource`: ExchangePlan -> EventSubscription, relation `uses`,
  exact QName-resolved source type;
- `FormCommand`: command -> method in the same registered form's FormModule,
  relation `handles`, exact action and typed optional `callType`;
- `CommonCommand`: CommonCommand -> method in that command's CommandModule;
- `ScheduledJob`: ScheduledJob -> CommonModule method; disabled remains
  observed but creates no runtime edge;
- `HttpRoute`: route -> method in the same HTTPService module, with typed verb
  and canonical combined RootURL/URLTemplate;
- `Structural`: only the versioned valid owner/child and module/method shapes.

The spec currently says only "bounded". Before GREEN it must state byte limits
for event/action/template/URL values and the exact RootURL+URLTemplate
normalization algorithm. Evidence digests must include the typed form call
type, including the distinction between absent and Before/After/Override.

There is already a `FormCallType { Before, After, Override }` registry in
`native_operations/form_event_registry.rs`. Move/reuse it from a neutral domain
module; do not add a second spelling list in discovery.

### ArtifactOwnershipChain and existence materiality

`proposal_validator.rs:27-61,199-216` currently:

- asks `DefinitionPort` for every artifact kind;
- returns the target itself as the owner for every non-method;
- coerces method owners to `metadata_object`, losing Report, DataProcessor,
  CommonCommand, ExchangePlan, form, and module identities;
- silently returns no owner when that coercion is rejected.

Add a tested `ArtifactOwnershipChain` smart constructor. It must preserve, as
applicable, root specialized owner, registered form, module, and method. At
minimum test these shapes:

```text
CommonModule.M.Handler
Document.D.ObjectModule.Handler
CommonCommand.C.CommandModule.Handler
ExchangePlan.P.ObjectModule.Handler
Report.R.Form.F.FormModule.Handler
DataProcessor.P.Form.F.FormModule.Handler
```

Existence policy must be a table, not another string-shape heuristic:

- method: exact Definition plus every registered owner required by its chain;
- declarative target: exact MetadataPresent; never DefinitionPort;
- form command: exact MetadataPresent, with FormInspection independently
  material for its exact action/handler runtime binding.

Relevant evidence and conflict lookup must include the whole ownership chain,
not just one owner.

### Callback gating is a join, not an unconditional edge

`evidence_graph.rs:144-158` currently promotes every PlatformCallback fact to a
`handles` edge. `PlatformCallbackShape` stores the script variant as a string,
has no `is_function`, and reuses named definition parameters. There is no
signature comparison anywhere.

Required model:

- typed `ScriptVariant = Russian | English | Unknown` from Configuration.xml;
- callback registry key: script variant, metadata kind, module kind, method;
- expected shape: procedure/function, export requirement, ordered arity and
  by-value/default flags; expected parameter names are not semantic;
- definition shape retains actual names for provenance, but compatibility
  ignores names;
- module kind and specialized owner must match the callback key;
- only a compatible PlatformCallback + Definition pair creates the runtime
  edge.

A complete incompatible definition is explicit negative runtime evidence, not
a conflict and not generic absence. Preserve
`callback_signature_mismatch` in checks/verdict diagnostics. The current
`build_checks()` collapses every graph conflict to `conflicting_evidence`, so a
separate typed runtime-rejection/mismatch collection is needed. With complete
coverage it should make runtime reachability `no` and the exact proposal
`contradicted`; with unavailable definition coverage it remains `unknown`.

The active spec has no authoritative callback registry rows. Do not guess them.
Before GREEN, document and fixture-verify the exact Russian/English method,
module, function/export, arity, by-value, and default flags for every v1
platform-owned lifecycle/command callback actually admitted.

### Support collection must be staged

The current use case calls all six evidence ports once with the original query.
In explore mode, `SupportStatePort` therefore cannot know candidates discovered
by metadata/definition/binding providers. Emitting support facts for the whole
configuration is unbounded and may exceed `maxEvidence`.

Change the support port to accept an exact canonical subject set and stage the
use case:

1. collect non-support evidence;
2. build the preliminary connected target set plus explicit proposals;
3. query support once for the sorted/deduplicated exact ownership subjects;
4. rebuild/finalize graph and proposal validation.

The use case owns this orchestration; one infrastructure adapter must not call
another adapter.

## Shared Platform XML catalog requirements

Task 4 already has the shared parser at
`infrastructure/platform_xml.rs:19-121`. Do not create a second registration
parser under `infrastructure/discovery/platform_xml.rs`.

Refactor the shared parser to return a typed catalog containing at least:

- direct configuration registrations;
- descriptor-validated direct nested form/template/command registrations;
- typed ScriptVariant without rejecting unknown variants needed for an
  inconclusive provider outcome;
- 1-based provenance for each material registration/name field.

Task 4 consumes only the registration/path subset. Task 5 consumes the same
catalog plus semantic fields. Concrete port adapters live under
`infrastructure/discovery/`; the shared parser remains at its existing neutral
path.

Every provider path is built from `SourceSetSnapshot.source_set.relative_root`
and a validated catalog registration, checked against `manifest.entries()`,
and read only through `read_verified`/`read_optional_verified`. Freshness is
constructed only from the supplied snapshot. No `std::fs`, `Path::exists`,
`support_object_uuid_for_path`, or legacy display helper is allowed in a
discovery provider.

The spec still needs exact v1 XML field paths and normalization rules for:

- EventSubscription event, handler, and source types;
- CommonCommand callback module;
- ScheduledJob handler and enabled state;
- HTTPService RootURL, URLTemplate, verb, and handler;
- ExchangePlan source QName to EventSubscription binding;
- managed form command Action elements and optional callType.

QName resolution must use the in-scope namespace URI, never prefix spelling.
Unbound prefixes, wrong namespace URI, ambiguous duplicate direct fields, and
control/oversize payloads are failed material. Comments, strings, descendants
outside the exact direct path, and unregistered files are decoys.

Provider parsing also needs deterministic file/byte/XML/result bounds. Task 4
bounded capture does not make parsing a potentially huge nested Form.xml safe.
At minimum preflight the manifest byte length before reading/parsing, keep a
64 MiB per-XML ceiling, traverse catalog paths in canonical order, and return a
bounded outcome with no negative proof when a provider limit is reached.

## ParentConfigurations refactor requirements

The current parser (`native_operations/common.rs:1695-1765`) fails open in all
of these ways:

- missing, metadata error, read I/O, and malformed bytes all collapse to
  `None`;
- every file of 32 bytes or less is treated as removed from support;
- invalid UTF-8 is accepted through lossy decoding;
- arbitrary global flags are interpreted as read-only instead of malformed;
- object rules are numeric and conflicting duplicates are silently reduced;
- display helpers render malformed/I/O as "not under support";
- `support_guard_violation()` returns `Option`, so parse/I/O failure authorizes
  the mutation;
- support-edit decodes and then rereads the same file through another parser.

Create one pure byte parser, preferably in a neutral
`infrastructure/parent_configurations.rs`, and a typed read boundary:

```text
ParentConfigurationsRead =
  Missing |
  Parsed(UnderSupport | ExplicitNotUnderSupport) |
  Malformed(reason) |
  IoFailure(reason, retryable)

ObjectRule = Locked | Editable | Removed
```

Use `BTreeMap` for rules and reject conflicting duplicate UUID rules. Invalid
UTF-8 and short non-empty garbage are Malformed. Only a fixture-proven canonical
empty/zero-vendor representation may become ExplicitNotUnderSupport.

Required consumers of the same typed result:

- discovery SupportStatePort through the Task 4 tombstone and verified reader;
- `support_state_lines_for_configuration`;
- `support_status_for_path`;
- `support_guard_violation` and application `support_guard_check`;
- `native_operations/support.rs` (`unica.support.edit`).

The guard API must return `Result`/a typed decision so malformed and I/O cannot
become `Allow`. The active spec must state how an unavailable support file
interacts with `off`, `warn`, and `deny`; absent an explicit exception, parser
failure should block an applied mutation rather than silently downgrade it.

## Required files beyond the original brief

At minimum expect changes in:

- `application/discovery/contract.rs`
- `application/discovery/model.rs`
- `application/discovery/determinism.rs`
- `application/discovery/ports.rs`
- `application/discovery/evidence_graph.rs`
- `application/discovery/proposal_validator.rs`
- `application/discovery/use_case.rs`
- `application/discovery/mod.rs`
- `application/mod.rs` for the error-aware legacy support guard
- `domain/discovery_registry.rs` for shared ScriptVariant/FormCallType if kept
  domain-neutral
- existing `infrastructure/platform_xml.rs`
- new `infrastructure/discovery/{mod,platform_xml,platform_callbacks,support}.rs`
- preferably new neutral `infrastructure/parent_configurations.rs`
- `infrastructure/mod.rs`
- `infrastructure/native_operations/common.rs`
- `infrastructure/native_operations/support.rs`
- `infrastructure/native_operations/form_event_registry.rs` if FormCallType is
  centralized
- existing ScriptVariant validators in `native_operations/cf.rs` and `cfe.rs`
  if the new enum becomes the single registry
- `spec/architecture/extension-point-discovery.md`
- historical implementation plan and `tests/ci/test_product_contracts.py`
- fixtures under `tests/fixtures/project_discovery/platform_xml/` and
  `tests/fixtures/project_discovery/support/`.

No new dependency appears necessary: `roxmltree`, `sha2`, and serde are already
available.

## Mandatory test ownership and gaps

### Application contract tests

- old ExchangePlan/handles combination rejected; SubscriptionSource/uses is the
  only accepted replacement and has a stable digest tag;
- every binding constructor rejects wrong endpoint kinds, cross-owner form
  handlers, mismatched service/route, wrong CommonModule handler, blank,
  control, and oversize payloads;
- Form callType absent/Before/After/Override changes the digest; invalid spelling
  is rejected;
- specialized ownership chains preserve exact identities;
- declarative existence does not consult DefinitionPort;
- form command keeps FormInspection material;
- ExchangePlan uses-only is not reachable; full uses+subscribes chain is;
- callback edge requires compatible definition; function/export/arity/
  by-value/default/module mismatches produce callback_signature_mismatch;
- parameter rename remains compatible and does not change the compatibility
  result;
- two mutation intents over one raw support fact do not create a graph conflict.

### Snapshot/catalog boundary tests

- root duplicate/unknown registration and descriptor kind/name/malformed XML:
  capture error and zero provider calls;
- shared parser emits identical registrations for snapshot selection and
  provider catalog, including BOM and prefix variation;
- unregistered valid/malformed decoy never enters the manifest;
- plan/snapshot source-set mismatch performs zero reads;
- mutation before a verified provider read becomes retryable unavailable and
  promotes no record.

### Seven provider-flow tests

For each flow: positive, valid alternative, exact wrong binding, disabled where
applicable, semantic malformed material, lexical decoy, registered hard decoy,
unregistered valid/malformed decoy, stable ordering, exact 1-based location,
source mismatch, and verified-read mutation. Parameterize common safety cases
rather than copy them seven times.

Additional mandatory cases: Russian/English/unknown ScriptVariant; QName prefix
renaming with identical namespace URI; wrong QName URI/unbound prefix; direct
exchange callback remains unsupported; Report and DataProcessor owner chains
remain specialized.

### Support tests

- Missing base, Missing extension, valid under-support global disabled/enabled,
  Locked/Editable/Removed object rules, canonical explicit-not-under-support;
- invalid UTF-8, empty-vs-short-garbage boundary, invalid header/global flag,
  truncated input, conflicting duplicate UUID, I/O failure;
- every legacy renderer says malformed/unavailable rather than not-under-support;
- support guard blocks malformed/I/O and never calls the mutation handler;
- support-edit uses the same parse result and does not reread/reinterpret;
- snapshot fingerprint mutation becomes retryable unavailable;
- same raw fact plus different proposal intent yields different policy without
  conflicting evidence.

## Minimal correct RED -> GREEN order

1. **RED: synchronize contract expectations.** Add product-contract and Rust
   tests that reject ExchangePlan/handles and demand SubscriptionSource/uses,
   typed FormCallType, ScriptVariant, bounded constructors, and
   ArtifactOwnershipChain. Run the focused application tests and verify the
   expected failures.

2. **GREEN: application primitives only.** Implement the validated binding,
   raw support fact, ownership chain, stable tags, canonical encoding, and port
   compatibility. No XML provider yet. Re-run application discovery tests.

3. **RED: graph/validator/use-case behavior.** Add tests for artifact-specific
   existence, specialized owner materiality, uses-only versus full exchange
   chain, callback signature join/mismatch, and staged support subjects.

4. **GREEN: graph/validator/use-case.** Add typed runtime rejection, exact
   material-port calculation, full ownership evidence, and staged support
   query. Re-run all application discovery tests.

5. **RED: shared catalog extension.** Test ScriptVariant, registration/name
   provenance, direct-child semantics, and Task 4 compatibility in the existing
   `infrastructure/platform_xml.rs`. Verify the RED is caused by missing typed
   fields, not by a broken fixture.

6. **GREEN: shared parser without provider semantics.** Refactor Task 4 to
   consume the same typed catalog subset and run all `source_snapshot` and
   `project_sources` tests before continuing.

7. **RED/GREEN vertical provider slices.** Implement in this order because each
   adds one reusable parser primitive: EventSubscription, FormCommand,
   CommonCommand callback, ScheduledJob, HTTPRoute, SubscriptionSource chain,
   Report/DataProcessor ownership. For each slice, observe the positive test
   fail before code, then add wrong-binding/malformed/decoy/location cases.

8. **RED: pure ParentConfigurations parser.** Add the complete raw outcome and
   object-rule matrix using bytes, including invalid UTF-8 and short garbage.

9. **GREEN: parser, then consumers one by one.** First pure parser; then
   snapshot-bound SupportStatePort; then legacy renderers; then error-aware
   guard; then support-edit. Add a failing regression before each consumer
   change.

10. **REFACTOR and full verification.** Remove duplicate ScriptVariant/
    FormCallType lists, update spec/plan/product contract, run focused provider,
    support, snapshot, and application suites, then full test, fmt, clippy,
    product contract, and `git diff --check`. Record every RED and final command
    in `task-5-report.md`.

Suggested focused commands after the tests exist:

```text
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder infrastructure::platform_xml -- --nocapture
cargo test --locked -p unica-coder infrastructure::discovery::platform_xml -- --nocapture
cargo test --locked -p unica-coder parent_configurations -- --nocapture
cargo test --locked -p unica-coder support_guard -- --nocapture
cargo test --locked -p unica-coder source_snapshot -- --nocapture
```

The original single filter `discovery::platform_xml` is insufficient: it would
miss application materiality, callback gating, legacy guard fail-open, and Task
4 regressions.
