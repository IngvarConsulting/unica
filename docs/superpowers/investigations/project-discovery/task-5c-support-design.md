# Task 5C design audit: ParentConfigurations and support evidence

Status: design-ready for the proven subset; unproven format and destructive-
policy classes are explicit stop conditions and must not be guessed.

Audit base: `20f6afa7a09430614babebc0cdeebeb94c8a0189` plus the accepted Task 4
snapshot contract. This document is read-only analysis of tracked code,
fixtures, tests, the active spec, and the Task 5 brief. It does not make the
historical donor scripts or PR #83 authoritative.

## Executive decision

Use one strict byte parser and one typed read result for discovery, legacy
renderers, the legacy support guard, and `unica.support.edit`. Missing material,
parsed material, malformed/unsupported present material, and I/O failure must
never collapse into one `Option`.

The v1 parser may claim only the positive under-support layout demonstrated by
the current fixture/test corpus. In particular:

- absence is authoritative only through Task 4's verified optional tombstone
  (or an equivalently race-aware live-read result for legacy consumers);
- a base source with a verified missing file projects to public
  `not_under_support`;
- an extension source with a verified missing file projects to public
  `extension_owned` for direct mutation;
- a known object UUID absent from the rules is a separate raw state and also
  projects to public `not_under_support` for direct mutation;
- a present file whose layout is not proven, including any guessed
  zero-vendor/"explicitly removed" encoding, produces no raw support fact and
  projects to public `unknown` with a blocking/inconclusive check;
- malformed or unavailable support material can never silently render or guard
  as "not under support".

There is **no fixture-proven canonical present-file representation for explicit
not-under-support in the current repository**. The current `vendor_count == 0`
and `len <= 32` behavior is a heuristic, not evidence. Task 5C must treat that
present layout as unsupported/unknown until a real Designer-exported fixture is
committed with provenance and expected semantics.

## Evidence inventory

### Current tracked fixtures

`git ls-files '*ParentConfigurations.bin'` returns exactly three files:

1. `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/form-compile/fixtures/on-support/Ext/ParentConfigurations.bin`
2. `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/meta-compile/fixtures/on-support/Ext/ParentConfigurations.bin`
3. `tests/fixtures/unica_mcp_script_parity/cc-1c-skills/cases/skd-compile/fixtures/on-support/Ext/ParentConfigurations.bin`

They are byte-for-byte identical (`sha256
6750bbf0b567b5bf475ee8a3b2b00c5391dba487358cf05c47c77c07e01e90e3`),
337 bytes, strict UTF-8 with one BOM, one line, and no final newline. The only
demonstrated semantic shape is:

- format marker `6`;
- global flag `0`;
- one vendor configuration;
- three object entries;
- root configuration `Locked`, catalog `Locked`, catalog `Removed`.

The three guard-deny parity cases exercise only the locked root/owner path. No
tracked file proves:

- global flag `1`;
- object rule `Editable`;
- zero vendors / explicit full support removal;
- an empty or short valid document;
- multiple vendors;
- duplicate or conflicting rules;
- a nested form/template rule;
- BOM-less input, leading/trailing whitespace, or another encoding;
- malformed, truncated, unreadable, or concurrently changed input.

Application tests synthesize a BOM-prefixed string with one vendor and object
rules `Editable`, `Locked`, and `Removed`, then obtain global flag `1` by text
replacement. That is useful compatibility evidence for current behavior but is
not an independent exported format fixture. The synthetic helper and the
tracked fixture also disagree on whether the first object record carries a
second mirrored UUID. Therefore a complete general ParentConfigurations
serialization grammar is not established by the current corpus.

The old PR #83 commit `639cc58` contains a synthetic, no-BOM fixture whose rule
list includes a nested form UUID. It is historical evidence that the prior
prototype expected nested support granularity, but it is not current source of
truth and cannot establish the production format by itself.

### Current parser defects

`crates/unica-coder/src/infrastructure/native_operations/common.rs` currently:

- returns `None` for missing, metadata error, read error, and malformed header;
- calls `Path::is_file`, which also hides metadata errors;
- treats every file of at most 32 bytes as fully removed from support;
- decodes invalid UTF-8 lossily;
- accepts any `u8` global flag and treats every value except `0` as read-only;
- validates only the first three comma-separated header fields;
- scans arbitrary bytes for `([0-2]),0,<uuid>` rather than parsing records;
- silently reduces conflicting rules for one UUID to the lowest numeric flag;
- counts duplicate matches while storing only one selected rule;
- extracts every quoted string in the document and groups them by threes,
  without tying them to the declared vendor count;
- does not validate closing structure, record counts, trailing data, or a
  supported schema variant.

`crates/unica-coder/src/infrastructure/native_operations/support.rs` has a
second semantic boundary:

- strict UTF-8 decode is followed by a second filesystem read through the
  lossy parser;
- `len <= 32` is independently treated as a safe no-op;
- mutation scans text for patterns again rather than using validated record
  spans;
- input framing is not preserved: every write adds a BOM;
- a concurrent replacement between reads can be parsed and edited as two
  different documents.

`support_guard_violation()` returns `Option`; every parser/read failure becomes
`None` and therefore `Allow`. Both display helpers likewise render failure as
"not under support".

## Typed boundary

Place the neutral parser in
`crates/unica-coder/src/infrastructure/parent_configurations.rs`. Discovery
must not depend on `native_operations/common.rs`, and legacy native operations
must not keep a second parser.

```text
ParentConfigurationsRead =
  Missing |
  Parsed(ParentConfigurationsDocument) |
  Malformed(ParentConfigurationsProblem) |
  IoFailure(ParentConfigurationsIoFailure)

ParentConfigurationsDocument =
  UnderSupport {
    framing,
    globalEditing,
    vendors,
    objectRules: BTreeMap<Uuid, ObjectRule>,
    validatedEditSpans
  }

GlobalEditing = Enabled | Disabled
ObjectRule = Locked | Editable | Removed
```

`ObjectRule` is never a number outside the parser/serializer boundary. The
wire mapping is fixture-backed and closed: `0 -> Locked`, `1 -> Editable`,
`2 -> Removed`.

`Parsed(ExplicitNotUnderSupport)` is deliberately **not a constructible v1
parser result yet**. Adding it requires a real fixture. A present zero-vendor or
short file currently becomes:

```text
Malformed(reason = unsupported_parent_configurations_variant)
```

This is semantically "unsupported/unknown", not a claim that the real platform
file is corrupt. If the type retains a reserved `ExplicitNotUnderSupport`
variant for the already approved high-level design, its constructor remains
private and no byte parser path/test may fabricate it until the stop condition
is cleared.

### Proven text/binary boundary

The parser accepts bytes, never a pre-decoded lossy string.

For the v1 proven subset:

- strict UTF-8 only;
- one optional UTF-8 BOM may be retained as framing for read compatibility;
- the first semantic byte is `{` and the top-level marker is exactly `6`;
- global flag is exactly `0` or `1`;
- only structurally tokenized top-level fields are inspected; quoted content
  cannot create object rules;
- UUIDs are canonicalized to lowercase for identity, while original bytes and
  validated byte spans are retained for an edit;
- the whole accepted document must be consumed; truncation, extra top-level
  tokens, invalid quoting, and unbalanced braces are not partially accepted;
- embedded NUL, UTF-16, arbitrary binary containers, and invalid UTF-8 are
  malformed/unsupported, never "empty support".

The unique current fixture plus the application helper are enough to support
their one-vendor observed record variants. They are not enough to support
multiple vendors or to assert what a zero-vendor document contains. Unknown
structural variants fail closed instead of falling back to byte scanning.

### Duplicate and conflict behavior

Parse rule records into `BTreeMap<Uuid, BTreeSet<ObjectRule>>` first:

- repeated identical `(uuid, rule)` records are semantically deduplicated in
  deterministic order, while the serialized record count is still validated;
- two distinct rules for the same UUID are
  `Malformed(conflicting_parent_configuration_rules)`;
- never select `min(rule)` and never emit two raw support facts for the same
  subject;
- duplicate/ambiguous vendor records or a declared count that cannot be
  validated are `unsupported_parent_configurations_variant` until real
  multi-vendor fixtures establish composition rules.

This is intentionally conservative. Multiple parent configurations may make a
duplicate rule meaningful; the current corpus does not prove how to aggregate
it. Rejecting the unproven conflict prevents a false supported conclusion.

## Read adapters and retry policy

There are two readers over the same pure byte parser.

### Snapshot-bound discovery reader

It receives the already selected `SourceSetSnapshot` and calls only
`read_optional_verified` for the exact workspace-relative
`Ext/ParentConfigurations.bin` path.

| Snapshot result | Typed read | Provider outcome | retryable |
| --- | --- | --- | --- |
| `Ok(None)` matching Task 4 tombstone | `Missing` | complete raw fact by source kind | no |
| `Ok(Some(bytes))`, proven layout | `Parsed(UnderSupport)` | complete raw object fact | no |
| `Ok(Some(bytes))`, invalid/unsupported layout | `Malformed(stable reason)` | `Failed` | no |
| `SourceFingerprintMismatch` | `IoFailure(source_fingerprint_mismatch)` | `Unavailable` | yes |
| `SnapshotUnavailable` | `IoFailure(source_snapshot_unavailable)` | `Unavailable` | yes |
| `NotInManifest` for the declared optional path | provider contract violation | fatal contract error | no |

No prefix records are promoted on a read error. Do not emit
`SupportFactState::Unknown`; absence of a fact plus the failed/unavailable port
produces public `SupportState::Unknown` and an ineligible receipt.

### Legacy live reader

`Missing` is returned only when absence is positively observed for the exact
optional leaf under a resolved configuration root. A dangling symlink, special
file, directory, metadata error, permission error, or not-found-after-presence
race is an `IoFailure`, not `Missing`.

Retry classification is closed:

- retryable: interrupted, would-block, timed-out, or a proven read/identity
  race;
- non-retryable: permission denied, invalid path/topology, non-regular leaf,
  unsupported operation, and deterministic malformed bytes;
- unknown OS error kinds default to non-retryable for policy reporting; the
  guard still fails closed regardless of the retry bit.

The retry flag affects provider/operator diagnostics, never whether `deny`
authorizes a mutation.

## Raw support facts

The raw enum must describe source observations, not proposal policy:

```text
SupportFactState =
  Editable |
  Locked |
  ConfigurationReadOnly |
  Removed |
  ObjectNotListed |
  BaseWithoutParentConfigurations |
  ExtensionWithoutParentConfigurations
```

Meanings:

- `ConfigurationReadOnly`: parsed under-support document with global editing
  disabled; it dominates any per-object rule;
- `Editable`, `Locked`, `Removed`: global editing enabled and the exact resolved
  subject UUID has that rule;
- `ObjectNotListed`: global editing enabled, the exact subject UUID was
  successfully resolved, and that UUID is absent from a completely parsed rule
  map;
- `BaseWithoutParentConfigurations`: verified tombstone for a configuration
  source;
- `ExtensionWithoutParentConfigurations`: verified tombstone for an extension
  source.

Do not use `ObjectNotListed` when subject UUID resolution failed. That is a
failed support assessment and public `unknown`. Do not add raw
`ExtensionRequired` or raw `ExtensionOwned`; those are intent/source projection
results. Do not add a raw explicit-present-not-under-support fact until its
bytes are fixture-proven.

Both current positive sources include an explicit configuration-root rule.
Therefore a parsed under-support document that omits the known configuration
root is not `ObjectNotListed`; it is an unsupported/invariant failure until a
real counterexample proves that shape valid. `ObjectNotListed` is reserved for
a successfully resolved registered child object that is absent from the
complete rule map.

Each raw fact needs its own stable tag and canonical spelling. The snapshot
source-set identity/fingerprint remains part of evidence freshness, so the two
missing-file variants cannot collide.

## Public projection matrix

### Candidate/direct-mutation projection

| Raw state | Public `SupportState` | Blocker |
| --- | --- | --- |
| `Editable` | `editable` | none |
| `Locked` | `locked` | direct mutation blocked |
| `ConfigurationReadOnly` | `configuration_read_only` | direct mutation blocked |
| `Removed` | `removed` | none for compatible direct mutation |
| `ObjectNotListed` | `not_under_support` | none |
| `BaseWithoutParentConfigurations` | `not_under_support` | none |
| `ExtensionWithoutParentConfigurations` | `extension_owned` | none for mutation in that extension |
| no fact because present layout is malformed/unsupported | `unknown` | `support_state_inconclusive` |
| no fact because read/fingerprint is unavailable | `unknown` | `support_state_inconclusive` |

Candidate projection is always the direct-mutation view and never reports
`extension_required`.

### Current proposal projection (`unica.cfe.patch_method`)

The public scalar represents whether the actual mutation location is safe, not
an average of analysis and destination facts.

| Analysis target | Destination extension | Public proposal support |
| --- | --- | --- |
| exact base target has any known raw state; exact destination is safely writable and ownership chain proves the patch belongs there | `ExtensionWithoutParentConfigurations`, `ObjectNotListed`, `Editable`, or `Removed` | `extension_required` |
| target is already owned by the same exact destination extension and destination is safely writable | same safe destination states | `extension_owned` |
| destination exact subject is `Locked` | any analysis state | `locked` + blocking check |
| destination is `ConfigurationReadOnly` | any analysis state | `configuration_read_only` + blocking check |
| destination or analysis support is malformed, unavailable, unresolved, conflicting, or source ownership is ambiguous | any | `unknown`; proposal unknown and receipt ineligible |

`BaseWithoutParentConfigurations`/`ObjectNotListed` on the analysis target do
not by themselves turn a CFE proposal into direct-editable; the explicit
mutation intent still writes the destination and therefore projects to
`extension_required` when that destination is proven safe. Conversely, a safe
analysis target cannot override a locked/read-only destination.

This matrix is scoped to the only current `MutationIntent`. New direct-edit or
other destination intents require new explicit rows, not a default arm.

## Legacy support guard

Replace `Option<SupportGuardViolation>` with an error-aware assessment:

```text
SupportGuardAssessment =
  Safe |
  Violation(SupportGuardViolation) |
  Indeterminate(SupportAssessmentProblem)
```

Resolve configuration root, exact target identity, and configured guard mode
before mapping the typed support read. `ObjectNotListed` is safe only after the
target UUID was resolved; UUID/XML resolution failure is `Indeterminate`.

| `editingAllowedCheck` | known violation | malformed/unsupported present file | I/O/identity failure |
| --- | --- | --- | --- |
| `off` | intentional `Allow`; guard evaluation disabled | intentional `Allow`; do not pretend evidence was safe | intentional `Allow` |
| `warn` | allow with existing support warning | allow with `support_state_malformed` warning | allow with `support_state_unavailable` warning |
| `deny` (also missing/invalid config value) | block | block before handler | block before handler |

Thus `warn` remains deliberately permissive and `off` remains an explicit
operator bypass; neither is a silent parser fall-open. `deny` never calls the
mutation handler when support evidence is indeterminate. Retryability changes
the diagnostic only.

Known state rules remain:

- global `ConfigurationReadOnly` blocks all guarded mutations;
- `Editable` requirement blocks only `Locked`;
- `Removed` requirement blocks `Locked` and `Editable`, allows `Removed`;
- `ObjectNotListed`, base missing, and a safely writable owned extension allow
  compatible mutation;
- unresolved target UUID is indeterminate, not `ObjectNotListed`.

`unica.support.edit` is not allowed to use `editingAllowedCheck` to bypass its
own parser. `Missing` remains a safe no-op; parsed under-support state may be
edited; malformed/unsupported/I/O fails the operation without a write. It
patches validated byte spans from the same parse result and preserves input
framing. It must not reread the file through a second semantic parser.

## Granularity and destructive-policy stop decision

The active fixture proves rules for configuration/catalog UUIDs only. Current
code resolves an existing nested `Form.xml` to the form UUID, but
`form-remove`/`template-remove` guard resolution deliberately targets the root
owner because those operations also rewrite the root descriptor. `meta-remove`
requires `Removed`; nested removers require only `Editable`.

Historical PR #83's synthetic fixture includes a nested form UUID, so it is not
safe to conclude that ParentConfigurations contains root objects only.
However, there is no current real fixture proving nested form/template rule
semantics or which rules must jointly authorize a removal.

Task 5C therefore:

- preserves existing operation-descriptor requirements during parser/error
  migration;
- does not mechanically change `form-remove`/`template-remove` to `Removed`;
- adds an explicit regression showing which UUID each current resolver checks;
- records a stop/follow-up decision: changing destructive nested policy
  requires a real configuration fixture containing owner plus nested
  form/template rules and an agreed rule-composition matrix (owner only, child
  only, or both).

This avoids both unsafe assumptions: that nested rules never exist, and that
deleting a child is semantically identical to deleting its root owner.

## Exact legacy consumers to migrate

### Parser/render consumers

- `native_operations/common.rs::support_state_lines_for_configuration`
  (`unica.cf.info` via `native_operations/cf.rs`);
- `native_operations/common.rs::support_status_for_path`, consumed by:
  - `unica.form.info` (`native_operations/form.rs`),
  - `unica.meta.info` (`native_operations/meta.rs`),
  - `unica.skd.info` (`native_operations/skd.rs`),
  - `unica.mxl.info` (`native_operations/mxl.rs`),
  - `unica.role.info` (`native_operations/role.rs`),
  - `unica.subsystem.info` (`native_operations/subsystem.rs`);
- `native_operations/support.rs` (`unica.support.edit`).

Read-only renderers may remain `ok=true`, but must render explicit
`неизвестно: некорректный/неподдерживаемый ParentConfigurations.bin` or
`состояние поддержки недоступно`, never `не на поддержке` for malformed/I/O.
The user-facing text is a renderer over the typed result; discovery never
parses that text.

### Support-guard operation descriptors

The shared application guard currently covers:

- `cf-edit`;
- `meta-compile`, `meta-edit`, `meta-remove`;
- `help-add`;
- `form-add`, `form-compile`, `form-edit`, `form-remove`;
- `interface-edit`;
- `subsystem-compile`, `subsystem-edit`;
- `template-add`, `template-remove`;
- `skd-compile`, `skd-edit`;
- `mxl-compile`;
- `role-compile`.

Every descriptor must observe the same `Safe/Violation/Indeterminate` mapping.
The detailed compile preview path, which calls the same support guard after
resolving its output target, must receive the same behavior.

Reference Python/PowerShell scripts under the parity fixture contain their own
"errors degrade to allow" implementation. They are donor/parity material, not
the prompt-visible runtime, and must not be copied back into native Unica.

## RED test matrix

Write each test first and record its actual RED output. Tests marked **BLOCKED
FIXTURE** must not be implemented with invented bytes.

### A. Pure byte parser

| Test | Expected RED assertion |
| --- | --- |
| unique current tracked fixture | strict parse to global enabled, one vendor, root/catalog rules |
| application helper `Editable/Locked/Removed` | typed rules, no numeric public values |
| optional BOM framing | BOM and BOM-less compatibility parse to the same semantics; framing remains distinguishable for edit preservation |
| invalid UTF-8 / UTF-16 / NUL | `Malformed(invalid_parent_configurations_encoding)` |
| empty bytes | malformed/unsupported, never removed |
| each short garbage length `1..=32` | malformed/unsupported, never removed |
| marker other than `6` | `unsupported_parent_configurations_variant` |
| global flag outside `0|1` | `invalid_parent_configurations_global_flag` |
| truncated quote/brace/UUID | stable non-retryable malformed reason |
| lexical `0,0,<uuid>` inside vendor string | no object rule emitted |
| extra trailing semantic token | malformed; no prefix acceptance |
| identical duplicate `(uuid, rule)` | one canonical raw rule, deterministic result |
| same UUID with two rules | `conflicting_parent_configuration_rules`; no min selection |
| declared object/vendor count mismatch | malformed/unsupported; no partial rules |
| multi-vendor composition | **BLOCKED FIXTURE**; unsupported until real exported examples exist |
| present explicit-not-under-support | **BLOCKED FIXTURE**; no synthetic `{6,*,0}` acceptance |

### B. Snapshot-bound support provider

| Test | Expected outcome |
| --- | --- |
| verified missing base | complete `BaseWithoutParentConfigurations` |
| verified missing extension | complete `ExtensionWithoutParentConfigurations` |
| global disabled | complete `ConfigurationReadOnly`, object rule ignored |
| exact Locked/Editable/Removed UUID | matching raw fact |
| resolved UUID absent from complete rule map | complete `ObjectNotListed` |
| UUID resolution failed | failed `support_subject_unresolved`, no fact |
| malformed/unsupported present file | failed, retryable false, no fact |
| fingerprint mutation before read | unavailable `source_fingerprint_mismatch`, retryable true, zero records |
| optional path unexpectedly not in manifest | provider contract violation |
| base/extension facts at identical bytes | distinct stable evidence due raw tag/source identity |
| stable ordering | canonical subject order independent of request order |

### C. Projection

| Test | Expected outcome |
| --- | --- |
| `ObjectNotListed` candidate | public `not_under_support`, no blocker |
| unsupported present file candidate/proposal | public `unknown`, support inconclusive, receipt denied |
| same raw base fact with direct candidate and CFE proposal | direct public state vs `extension_required`, no graph conflict |
| writable extension destination | CFE proposal `extension_required` or `extension_owned` per ownership |
| locked/read-only destination | destination blocker wins over writable base |
| unknown destination | proposal unknown, no receipt |

### D. Legacy renderer

- Parameterize missing, each parsed state, `ObjectNotListed`, malformed,
  unsupported, retryable I/O, and non-retryable I/O across configuration and
  object renderers.
- Assert malformed/I/O output never contains `не на поддержке`, `правки
  свободны`, or `снята с поддержки`.
- Retain current positive display assertions for `cf.info`, `meta.info`,
  `form.info`, `skd.info`, `mxl.info`, `role.info`, and `subsystem.info`.

### E. Guard matrix

Parameterize `off|warn|deny` over:

- locked object;
- global read-only;
- malformed bytes;
- unsupported present variant;
- read/metadata failure;
- unresolved target UUID;
- verified missing base;
- exact `ObjectNotListed`;
- exact `Removed` under both `Editable` and `Removed` requirements.

Assertions must include whether the handler was invoked, exact decision reason,
warning/block envelope, and zero source writes on every deny. Invalid/missing
guard configuration aliases to `deny`. Add an explicit `off` test; none exists
in the current Rust suite.

### F. `unica.support.edit`

- missing file remains no-op;
- malformed, unsupported, invalid UTF-8, and I/O return `ok=false` and preserve
  bytes;
- capability/object edits use the typed parsed document and validated spans;
- conflict never rewrites all matching substrings;
- BOM/no-BOM framing is preserved;
- one semantic read result is used (test hook changes the file after read and
  must cause a stale/write refusal, not edit a second interpretation);
- output reparses to the intended typed state before replacement;
- explicit-not-under-support edit is **BLOCKED FIXTURE**.

### G. Nested granularity

- existing form edit resolves form UUID when present;
- new form/template compilation resolves owner UUID;
- current form/template removal resolver identifies owner UUID;
- a real owner+child ParentConfigurations decision matrix is **BLOCKED
  FIXTURE/POLICY** and must not be synthesized from PR #83.

## Migration sequence

1. **Freeze evidence.** Copy the three identical current fixtures into one
   canonical Task 5 support fixture with provenance; keep parity fixtures
   unchanged. Add a manifest test proving there is only one semantic fixture,
   not three independent format variants.
2. **RED pure boundary.** Add byte-parser tests for the proven positive layout,
   strict encoding, short garbage, invalid header/global flag, lexical decoy,
   duplicate/conflict, truncation, and unsupported zero/multi-vendor variants.
3. **GREEN pure parser.** Add `parent_configurations.rs`, typed enums,
   structural tokenizer, canonical rule map, stable reasons, and validated edit
   spans. No filesystem, rendering, provider, or guard logic in this module.
4. **RED/GREEN snapshot reader.** Implement Task 4
   `read_optional_verified` mapping, exact base/extension raw facts,
   `ObjectNotListed`, and Failed/Unavailable/ContractViolation split.
5. **RED/GREEN application projection.** Add the direct candidate and current
   CFE proposal matrices; verify one raw fact can project differently by intent
   without becoming conflicting evidence.
6. **RED/GREEN legacy renderers.** Migrate `cf.info` first, then the shared
   object status renderer and all six object-level info consumers. Remove the
   old `Option<SupportState>` parser only after every renderer test is green.
7. **RED/GREEN guard.** Change the API to
   `Safe|Violation|Indeterminate`, resolve mode before assessment, add the full
   `off|warn|deny` matrix, and prove the handler is not called for deny+
   indeterminate.
8. **RED/GREEN support-edit.** Consume the same parsed document/spans, preserve
   framing, refuse stale/conflicting/unsupported inputs, and delete all pattern
   replacement parsers from `support.rs`.
9. **Granularity lock.** Add tests documenting current target UUID resolution
   without broadening destructive policy. Open/record the fixture/policy stop
   for nested removal.
10. **Contract synchronization.** Update the active spec, historical plan, and
    product-contract test with raw variants, public projection, unsupported
    explicit-present behavior, and guard matrix. Do not document a zero-vendor
    byte grammar.
11. **Full verification.** Run focused parser/provider/guard/edit tests, the
    complete `unica-coder` suite, fmt, clippy, product contracts, and
    `git diff --check`. Record actual RED/GREEN commands and commit SHA in the
    Task 5 report.

## Stop conditions

Task 5C must stop rather than guess if implementation needs any of the
following to pass:

1. a present-file explicit-not-under-support encoding;
2. multiple parent/vendor configuration composition;
3. conflicting per-vendor rules for the same UUID;
4. nested form/template removal authorization semantics;
5. another encoding/container/layout not represented by a provenance-backed
   real fixture.

Clearing a format stop requires the smallest redistributable real
Designer-exported fixture plus before/after platform behavior. Clearing the
destructive-policy stop additionally requires an explicit product decision.

Until then, present unknown layouts are `Failed/Unknown` for discovery,
visible-unknown for info tools, warn in `warn`, block in `deny`, and hard error
for `support.edit`. That is the only behavior consistent with Task 4 freshness,
receipt fail-closed invariants, and the available evidence.
