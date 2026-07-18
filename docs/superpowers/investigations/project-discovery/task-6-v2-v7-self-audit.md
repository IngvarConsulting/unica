# Task 6 v2 + v7 addendum self-audit

Status: **fresh frozen-tuple owner self-audit PASS; no owner-local or
cross-owner P0/P1. This file binds exact owner bytes but does not declare
package acceptance; only the external four-document ledger does**, 2026-07-18.

This is an owner self-audit, not an independent review and not acceptance
authority. It audits only:

```text
.superpowers/sdd/task-6-v2-design.md
.superpowers/sdd/task-6-v2-v7-addendum.md
```

against the complete peer owner contracts:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-5b-v7-contract.md
.superpowers/sdd/task-7-v7-addendum.md
```

This re-audit binds the exact frozen four-owner tuple listed below. Those hashes
identify the bytes audited; they are not acceptance status and no owner document
is edited to embed them. A separate reviewer must inspect that same tuple and
only the external ledger may accept it; this owner self-audit can never
substitute for either step.

## 1. Inputs and identity discipline

Immutable lineage checked:

```text
task-6-v2-design.md
SHA-256 = 5f2d859f77878b43e627930b46a99063972f0fe1a00b3bc692213beea76db4cc
```

The four owner bytes freshly re-audited are exactly:

```text
Task4 = 1581d0b737a9e4e856526d67987a292edd39404ec5dda1cb3299c6041409cde2
Task5B = 30430abeb69aeb83bd665a08b41fa1837675a651b3be736936c6e4e96e14f3ad
Task6 = 9f488f78ba20f188e1c28e5393eb9d5d16889cde8f8ca5363bb2ea476631fca0
Task7 = 708022ff0b179092d5f23609449dfa8a7415adaa2e404179b9a24b43d95c1b7d
```

Fresh SHA-256 recomputation matched all four values before semantic/stale/API
checks. A byte edit to one peer does not make an unchanged file's SHA-256
mathematically false, but it invalidates this four-document package tuple and
every derived generator, self-audit, review and ledger claim. One unchanged file
hash can never stand alone as accepted-package evidence.

Design and production identities remain separate. Design acceptance is one
atomic four-document ledger transition recorded outside the frozen owner docs
in `.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`. Before the first
Task 6 production RED, the production ledger/report must additionally contain:

```text
TASK5A_ACCEPTED_SHA
TASK4_V7_ACCEPTED_GIT_OID
TASK5B_V7_ACCEPTED_GIT_OID
```

Task 6 has no Task5C or Task 7 production dependency. Task 8 remains downstream
and cannot gate this design package.

## 2. Reproduced external authority

The prior detached audit of `https://github.com/itrous/bsl-analyzer` at
`5a02bb44dedaf29e0e29af1f740279d279199854` remains immutable Task 6
grammar-authority evidence. `HEAD` is tag `v0.2.55`; the seven pinned hashes are:

```text
3a05db2b2f00e61a24d5ecbd92861076e6de622900b3ea79245ef27855bb6b3d  crates/lexer/src/lib.rs
55d5c9acbb5d8a0f218a16f1b21d32fbc1312c00c58828dd7280fdea6fdbff7d  crates/parser/src/grammar.rs
a14dd0283860d74e64df8ad0fb428cbf4126e42ab2b189197e5f01482c923aa1  crates/parser/src/grammar/items.rs
77163356174a5e37fb04e901bfc69707f6b50fd59f0f04ba741b25f46ab43e9d  crates/parser/src/grammar/statements.rs
122c89847c4f3e09f9e324dfb4a82f872e011148cd63e9e5bcc7de1809383c3f  crates/parser/src/grammar/expressions.rs
4b9a9cb8a97b99a5dd194e273f46d22f9a387604b0ff62fe8603eebec71e577e  LICENSING.md
a5250ac6c47b5235c3483e1329cbd375fcece9c9ec4dd245dabf785a6b14e113  NOTICE
```

The lexer spelling is `&Вместо|&Instead`; `&Around` is not the accepted English
token. Lexer/parser are LGPL-3.0-or-later Tier B and remain grammar/fixture
authority only, never copied, vendored or used as a runtime dependency.
The inherited moving `bsl-parser/develop` links are historical bibliography,
not authority. New corpus bytes are locally authored synthetic fixtures under
the mandatory `tests/fixtures/project_discovery/bsl/PROVENANCE.md` inventory;
the exact dependency/license record is
`docs/third-party/project-discovery-dependencies.md`.

The separately reproduced identifier table remains:

```text
unicode-general-category = 1.1.0
archive SHA-256 = 0b993bddc193ae5bd0d623b49ec06ac3e9312875fdae725a975c51db1cc1677f
UNICODE_VERSION = 16.0.0
license = Apache-2.0
```

The lexical cross-check also closes trivia to SPACE/TAB/U+00A0,
CRLF/bare-CR/bare-LF and `//` through the first line ending. U+000C and all
other Unicode whitespace remain unsupported rather than hidden. The eight
interceptor names are annotation tokens only immediately after `&`; those same
spellings remain ordinary default-state identifiers.

## 3. Cross-document boundary audit

| Required boundary | Current Task 6 disposition | Result |
| --- | --- | --- |
| four-owner co-freeze | exact frozen tuple is bound in this derived audit; owner docs contain no self/peer hash or status, and acceptance status lives only in the external ledger | PASS |
| production DAG includes Task4 v7 | `Task5A -> Task4-v7 -> Task5B-v7 -> Task6` and three OIDs | PASS |
| no Task5C/Task7 production dependency | sections 1, 7, 11 and 13 forbid both implementations while Task 7 owner bytes participate only in the design package | PASS |
| composite-bound context | `PlatformCatalogPort::build_context` accepts the composite `SourceSnapshotV2`; one context binds its composite ID, both catalog sets and all three witness sets, while Task6 execution receives only the exact Analysis atomic `SourceSetSnapshotV2` | PASS |
| restricted Analysis header view | `analysis_platform_catalog()` exposes only owned Analysis source identity plus borrowed fingerprint, two catalog digests and numeric registered-Form version; no Form iterator/lookup/view or any-source authority leaks to Task6 | PASS |
| Task7 whole-context witness boundary | frozen Task7 stores the same composite-bound context and imports all three configuration, registered-Form and Analysis-BSL witness sets explicitly; no half-context/Analysis-only facade exists | PASS |
| Task7 empty Definition invocation | Task5A's scheduled authoritative work plan with `methods=[]` produces the Task6-owned typed query, which registers/invokes exactly once with one nonempty-scoped Invocation root; an empty association scope remains invalid and cannot schedule I/O | PASS |
| Task7 BSL pass-through | Task7 names only the unified scan/dispatcher, preserves the exact counter/outcome rows, keeps per-item FileBytesLimit nonterminal, and reserves terminal suffix omission for FileCount/TotalBytes | PASS |
| sole query authority | every smart constructor starts only from `&PlatformCatalogContextV1` and `analysis_platform_catalog()`; no detached catalog/header inputs | PASS |
| owned source identity | `analysis.source_identity()` returns owned `AtomicSourceIdentityV2`, appended directly without a borrowed temporary or redundant frame | PASS |
| sole Analysis BSL enumeration | only `context.analysis_bsl_material_scan_plan(snapshot)` exposes the complete captured surface; Task 6 has no manifest/suffix/path scan or private item index | PASS |
| builder-only Task4 authority | Task4 `module()`/`admission_byte_length()` and location `to_verified_location()` are whitelisted only to the Task5B context/plan builder; Task 6 cannot name either capability and receives only final typed items/opaque locations/admissions/terminal | PASS |
| one canonical partition | claimed Present FormModule replaces its ordinary slot once; unsupported captured ordinary remains one visible item; uncaptured Form-shaped decoy creates none | PASS |
| plan-owned scope | CodeSearch/CallGraph use `select_all`; Definition validates supported ownership/identity, partitions all targets against one equality-only `items()` membership set, then invokes one `select_modules(&in_plan)` before limits | PASS |
| Definition zero-file semantics | valid supported absent module is authoritative zero-file and may become Absent only under complete authority; Missing/NA remain in-plan; unsupported/malformed never Absent; post-intersection selection error is handle-mismatch ContractViolation | PASS |
| one admission authority | consuming non-Clone exact-size cursor owns merged file/byte counters and one immutable precomputed terminal; no Task 6 second sort/counter/terminal | PASS |
| conservative CallGraph | one all-surface cursor is stored; callers then referenced targets are read without a second plan/pass/reread; terminal before either is caller-scoped Bounded, never false Complete/edge | PASS |
| CallGraph zero-file split | true absent caller is `missing_caller_definition`; true absent referenced target is Named Unresolved + `unresolved_bsl_call`; terminal omission is never absence/unresolved proof; all are caller-scoped Bounded/no-edge | PASS |
| sole material read | only `context.read_analysis_bsl_material_verified(source_reader, snapshot, item)`; Task 5B privately dispatches ordinary/registered reads and injected-port counters | PASS |
| per-item recording | Ordinary Present 0/1/1; Registered Present 1/1/1; Registered Missing 1/0/0; NotApplicable/unsupported/FileBytesLimit 0/0/0; direct filesystem calls always zero | PASS |
| opaque locations/cache | every gap/evidence location is `VerifiedBslSourceLocationV1`; Present alone exposes exact-byte range location and `VerifiedBslCacheLocatorV1`; no raw path/key/read capability | PASS |
| Missing later appears | exact `source_fingerprint_mismatch`; no stale DefinitionAbsent/prefix | PASS |
| semantic scan/material mismatch | exact pre-I/O `ProviderOutcome::ContractViolation("registered_material_handle_mismatch")`, collected only as `DiscoveryError::ProviderContractViolation`, with no batch/gap/prefix | PASS |
| external drift split | only post-validation filesystem appearance/disappearance/content/identity/topology drift is retryable `source_fingerprint_mismatch`; Task7 Hard STOP enforces both directions explicitly | PASS |
| query freshness | v3 binds source fingerprint, both catalog digests and numeric registered-Form version | PASS |
| atomic-source framing | direct 148-byte `AtomicSourceIdentityV2`; no redundant outer `bytes` frame | PASS |
| design-stage generator | standalone two-path encoder reproduced six positive and one negative exact golden without production smart queries | PASS |
| optimized generator safety | zero Python `assert` statements; normal stdout is unchanged; `python -O` exits 1 before stdout/`PASS` | PASS |
| mutation authority | detached eight-way mutations are generator-only; production uses valid coupled context recaptures and compile-fail privacy tests | PASS |
| cache wire authority | internal `BslIdentifierV1`/`BslFileAnalysis` graph is non-serde; v3 wire stores only primitive tags/spans/spelling hints, parses every identifier from verified bytes and replays call/shadow/gap associations before construction | PASS |
| closed lexical contexts | U+00A0/trivia/BOM rules are exact and interceptor names remain identifiers outside immediate after-`&` state | PASS |
| provenance/license targets | moving links are non-normative; exact corpus provenance and project-discovery dependency inventory paths are mandatory | PASS |
| later production proof | TDD smart-context queries must equal frozen design values | PASS as gate |
| provider/application split | conclusion identity stays outside Task 6 query/group/outcome/cache | PASS |
| Task 8 downstream | identifier/span/interceptor corrections remain non-gating | PASS |

## 4. Task 6-owned coverage retained

The prior semantic audit remains valid after capture-boundary reconciliation:

- the immutable Task 6 v2 design is extended, not replaced;
- pinned grammar/license and exact Unicode 16.0.0 L* authority remain explicit;
- moving grammar links are non-normative; exact fixture provenance and
  dependency/license inventory targets are mandatory;
- SPACE/TAB/U+00A0, CRLF/CR/LF, BOM and comment trivia are byte-exact; all eight
  interceptor names retain default-state Identifier behavior;
- `BslIdentifierV1` is the sole standalone identifier constructor and uses the
  file lexer's exact classifier, bounds, keyword table and comparison text;
- six parser-owned spans, LF/CRLF/CR and trailing header trivia have one
  consistent byte algebra;
- inline declarations get `unsupported_bsl_definition_layout`, never a guessed
  line ending or silent negative proof;
- local/maybe-local shadows, duplicate definitions and all four CFE interceptor
  kinds retain exact conditional/deleted semantics;
- parser contract v2/cache schema v3 rejects stale DTOs;
- the internal identifier/parser graph is non-serde and cache v3 reconstructs
  each identifier from exact verified bytes through the sole constructor;
- tag-8 multiplicity/polarity and CallTarget Cartesian closure retain the shared
  Task 5B v7 encoders;
- the complete captured Analysis BSL surface, claimed-Form replacement,
  unsupported ordinary visibility, opaque location/cache capabilities and sole
  context dispatcher are imported from one Task 5B scan plan;
- Definition validates supported ownership/identity and partitions all targets
  against one zero-I/O membership set before selecting in-plan modules; valid
  zero-file targets have conservative complete-authority absence semantics;
  CodeSearch and conservative CallGraph select all, and CallGraph has one stored
  admitted cursor with caller-scoped terminal gaps and no second pass/reread;
- local provider ceilings run only after complete semantic-group classification;
- the detached eight-way single-field mutation is generator-only; production
  uses valid coupled context recaptures and compile-fail privacy tests.

## 5. Contradictions found and disposition

### C1 — three documents versus the real four-owner boundary

The old draft described Task5B+Task6+Task7 acceptance although Task 6's unified
scan/dispatcher boundary transitively depends on Task 4 v7 capture, location and
reader authority through Task5B.

Disposition: the exact four owner files now co-freeze atomically. Frozen owner
docs never embed self/peer acceptance hashes; the separate package ledger does.

### C2 — production DAG skipped Task 4

The old chain jumped from Task5A to Task5B and required two implementation OIDs.

Disposition: `TASK4_V7_ACCEPTED_GIT_OID` is an explicit predecessor and one of
three mandatory production identities.

### C3 — Missing was confused with zero verification

The old section said Missing had “zero read” and was never probed. That erased
the required post-capture contained absence proof.

Disposition: verifier calls and byte reads are separate. Managed Missing is
`1/0/0`, Managed Present `1/1/1`, NotApplicable is `0/0/0`, and all are
returned only through the context-owned scan dispatcher; Task 6 never receives
a path or raw material state.

### C4 — direct Form demand versus the complete captured BSL surface

The old repair made FormModule demand opaque but still left Task 6 enumerating
ordinary manifest entries separately. A claimed Present FormModule could then
appear both as an ordinary `.bsl` entry and a registered relationship, while
unsupported captured ordinary material and uncaptured decoys had no singular
partition authority.

Disposition: Task 6 imports one zero-I/O
`AnalysisBslMaterialScanPlanV1` over Task 4's exact capture projection. A
claimed Present FormModule replaces its ordinary slot once, unsupported
captured ordinary remains visible and an uncaptured decoy creates no item.
Selection/admission precede the sole context-owned dispatcher; Task 6 imports no
Task 4/Form reader, handle or resolver.

### C5 — numeric contract version was not explicit in the golden fixture

The query grammar needs the numeric registered-Form catalog contract version;
inferring `1` from a `/v1` string would create a second, unreviewed encoder.

Disposition: the coordinated Task 5B contract and Task 6 fixture both state
exact `REGISTERED_FORM_CATALOG_CONTRACT_VERSION: u16 = 1`. The standalone
design generator consumes that explicit fixture value; later production must
derive the same value from the typed context. Closed.

### C6 — “generate later” had a production cycle

The earlier generator wording required future production smart queries before
design freeze, although production is blocked by design and predecessor OIDs.

Disposition: section 7.1 now defines a checked standalone two-path design
generator over explicit published fixture authorities. It reproduced all six
positive values and the forbidden extra-frame negative value without importing
production. Production TDD later reconstructs the same values through real
smart constructors. Closed.

### C7 — downstream parser identity/span/interceptor drift

A path-free parser cannot construct `ArtifactIdentityBytesV1`; token-tight
declaration end contradicts trailing trivia; semantic Around is lexically
`&Instead`, not `&Around`.

Disposition: Task 6 retains `BslIdentifierV1`, the full header-line span and
`&Вместо|&Instead`. These are non-gating Task 8 obligations.

### C8 — redundant AtomicSourceIdentity length frame

An intermediate v3 spelling wrapped `AtomicSourceIdentityV2` in another
`bytes(...)`. The type's accepted encoding already contains
`u16be(role) || bytes(ResolvedSourceSetIdentityBytesV1)`. A second outer frame
would silently break base-v2 field compatibility and all later v3 lengths.

Disposition: section 7 appends the accepted 148-byte Analysis atomic-source
encoding returned as the owned `analysis.source_identity()` directly and makes
the double-framed spelling a negative golden.

### C9 — internal catalog and Form capabilities leaked across Task 6

An intermediate Task 6 spelling directly named `RegisteredFormCatalogV1`,
`RegisteredFormAuthorityV1.form_module_material` and
`RegisteredFormMaterialAuthorityV1`, even though the final Task 6 boundary must
expose only the whole context, scan plan and context-owned dispatcher.

Disposition: every query constructor starts from `&PlatformCatalogContextV1`
and uses the restricted `AnalysisPlatformCatalogViewV1` only for the v3 query
header: owned source identity, fingerprint, two digests and numeric version.
It exposes no Form iterator/lookup/view. BSL enumeration/read uses only
`AnalysisBslMaterialScanPlanV1` plus
`context.read_analysis_bsl_material_verified(...)`. Direct internal catalog,
Form/material view/resolver, Task 4 handle/projection/state/key/path or detached
plan/item access is a compile/static failure.

### C10 — reader ownership and error classes were conflated

An unqualified `read_registered_material_verified(...)` looked like a free
function and an earlier row classified internal key/fingerprint/manifest
disagreement as retryable fingerprint drift.

Disposition: only
`context.read_analysis_bsl_material_verified(source_reader, snapshot, item)` can
read. Task 5B privately dispatches ordinary/registered material and the exact
injected port owns counters. Internal context/plan/selection/item/handle/
projection/state/key/fingerprint/manifest/ordinary-entry disagreement is
nonretryable `registered_material_handle_mismatch` before I/O. Only external
post-validation filesystem drift is retryable `source_fingerprint_mismatch`.

### C11 — blanket keyword closure contradicted interceptor annotations

The inherited closed lexer described one global keyword table, while mandatory
`&Перед|&Before`, `&После|&After`, `&Вместо|&Instead` and
`&ИзменениеИКонтроль|&ChangeAndValidate` annotations require a context the
default-state lexer did not distinguish. Treating those names as global
keywords would also reject valid ordinary identifiers.

Disposition: the eight spellings are annotation tokens only in the immediate
no-trivia state after `&`; the same spellings are default-state Identifiers.
`&Around` and whitespace/NBSP between `&` and a name remain unsupported.

### C12 — separate Form/ordinary limits made identity and receipts ambiguous

Separate item-kind loops could double-count claimed FormModule bytes, apply
file/total limits in different orders, and could not assign one authoritative
location to unsupported, Missing, NotApplicable and terminal gaps.

Disposition: Task4 exposes typed module classification, exact Present length and
verified diagnostic location only through builder-whitelisted `module()`/
`admission_byte_length()`/`to_verified_location()` to Task5B, never Task6. One
consuming scan cursor then owns the merged canonical order, exact 20,000/16
MiB/512 MiB semantics and immutable first-omitted terminal location. Every item uses
`VerifiedBslSourceLocationV1`; Present alone exposes verified range locations
and a typed cache locator. No Task 6 counter or raw location derivation remains.

### C13 — inherited cache serialization could not represent v2 identities

The base cache serialized `BslFileAnalysis` directly. After parser v2 embeds
non-serde `BslIdentifierV1`, that wire contract is both uncompilable and unsafe:
accepting a deserialized identity would bypass the sole identifier parser.

Disposition: cache schema v3 has an entirely separate primitive/tag/span wire
graph. Internal `BslIdentifierV1`, `BslFileAnalysis` and every internal
identifier-containing struct are non-serde. Ingress revalidates exact verified
bytes and reconstructs every identity only through
`parse_complete_bsl_identifier_v1`; legacy call/shadow/gap spelling hints are
accepted only after deterministic verified-token semantic replay. Any failure
is a local-parse miss.

### C14 — production single-field mutations required forging private authority

The design generator can mutate eight detached encoder fields independently,
but production `PlatformCatalogContextV1` deliberately couples and privatizes
source identity, fingerprints, catalog digests and registered-Form version.

Disposition: detached eight-way mutation remains generator-only. Production
varies query-owned vectors/ceilings directly, changes authority through valid
recaptures, proves equal valid contexts encode equally, and compile-fails on
caller-forged versions/digests/half-contexts.

### C15 — moving references and fixture provenance were underspecified

Inherited `bsl-parser/develop` links could move after acceptance, and a corpus
described only as grammar-derived could silently copy external bytes without
license/source traceability.

Disposition: moving links are historical non-normative bibliography. Fixtures
are locally authored minimal synthetic cases under exact
`tests/fixtures/project_discovery/bsl/PROVENANCE.md`; external bytes require
URL/tag/commit/path/source hash/license/attribution/local mapping. Exact runtime
dependency and behavior-oracle licenses live at
`docs/third-party/project-discovery-dependencies.md`.

### C16 — Python assertions made the golden gate disappear under `-O`

The standalone generator originally exited successfully and printed `PASS`
under optimized Python because executable checks used `assert`.

Disposition: the generator rejects optimized mode before stdout, contains no
Python `assert` statements and uses explicit invariant failures. Normal stdout
and its frozen hash remain byte-identical; the `python3.12 -O` negative is part
of the verification gate.

### C17 — semantic mismatch had multiple possible public outcomes

Earlier prose allowed a context/handle mismatch to resemble filesystem drift,
a gap or a partial provider batch, which could make a caller defect retryable or
permit stale prefix evidence.

Disposition: exact `registered_material_handle_mismatch` maps first and only to
`ProviderOutcome::ContractViolation`, then only to
`DiscoveryError::ProviderContractViolation`; it has no batch/record/gap/prefix.
Only post-validation external filesystem drift is retryable Unavailable.

### C18 — unconditional scope-before-limits was impossible for CallGraph

Definition knows its target modules before reading, but CallGraph learns static
target CommonModules only after parsing queried callers. Pretending both could
preselect the same way would require a hidden second admission pass or would
under-account the global budget.

Disposition: Definition alone selects exact modules before limits. CodeSearch
and CallGraph select all. CallGraph stores one admitted merged cursor, reads
callers then only referenced targets, and never re-admits/rereads; a terminal
before either creates deterministic caller-scoped Bounded and no false
Complete/edge. Earlier unrelated files may conservatively suppress CallGraph;
true zero-file caller/target map to missing-caller/Named-Unresolved gaps, while
terminal omission never becomes absence proof. A future two-phase optimization
is explicit P2 and outside v7.

### C19 — payload-less ModuleNotInPlan could lose mixed Definition targets

Calling `select_modules` with a multi-Method vector containing both present and
absent modules returns only payload-less `ModuleNotInPlan`; it cannot say which
target was absent or return a selection for the present targets. Per-target
selection/admission would violate the one-cursor limit authority. Treating the
error as ContractViolation would also regress base-v2's complete-capture
zero-file absence proof.

Disposition: after exact supported-module ownership/identity validation,
Definition builds one equality-only available-module membership set from zero-I/O
`plan.items()`, partitions all target modules into `in_plan` and valid
authoritative zero-file, and calls `select_modules(&in_plan)` plus `admit`
exactly once. Registered Missing/NotApplicable obligations remain in-plan.
Zero-file targets contribute to Absent only if selected/query-wide authority is
complete; malformed/unsupported identity is rejected/gapped, never Absent. A
post-intersection `ModuleNotInPlan` is impossible and therefore exact zero-prefix
handle-mismatch ContractViolation. Mixed present+absent and permutation REDs
freeze the behavior.

### C20 — Task7 said “both” while Task5B owns three witness sets

An intermediate Task7 owner sentence could be read as importing only two exact
witness sets, while frozen Task5B context construction owns configuration,
registered-Form and Analysis-BSL witnesses. That ambiguity could permit an
orchestrator/context implementation to omit the scan-plan witness authority.

Disposition: frozen Task7 now names all three snapshot-bound witness sets
explicitly and stores/passes the one whole context. Fresh hash and stale scans
confirm the ambiguous two-set spelling is absent.

### C21 — Task7 retained pre-scan Form-demand vocabulary

An intermediate Task7 pass-through paragraph still described Known Ordinary/
Inconclusive and Managed Present/Missing demand, which was behaviorally similar
but no longer named the unified Task6 scan/dispatcher boundary and could invite
reintroduction of the deleted view/resolver/handle chain.

Disposition: frozen Task7 now preserves the exact imported matrix only as
Ordinary Present, Registered Present/Missing/NotApplicable, unsupported
Ordinary and FileBytesLimit results. It names
`analysis_bsl_material_scan_plan`/`read_analysis_bsl_material_verified`, imports
no old resolver/read symbol and never reclassifies an outcome.

### C22 — “skip empty invocation” contradicted Definition methods=[]

The application builder once allowed an empty provider plan to be skipped,
while Task5A's authoritative Definition schedule and Task6's frozen empty-query
golden require `methods=[]` to remain a real typed invocation. Skipping it would
change invocation roots, provider-call count and raw outcome identity.

Disposition: frozen Task7 distinguishes absence of a scheduled typed query from
an empty canonical member vector. A scheduled Definition `methods=[]` registers
and invokes exactly once and receives exactly one Invocation root. Fresh scans
find no empty-plan skip rule.

### C23 — Task7 called per-item FileBytesLimit terminal

An intermediate Task7 matrix grouped `FileBytesLimit` with terminal admission,
contradicting Task5B/Task6: an oversized Present consumes one file, yields one
per-item gap, consumes zero total bytes and leaves later selected items eligible.
Only FileCount and TotalBytes omit the remaining suffix.

Disposition: frozen Task7 now states the per-item/nonterminal FileBytesLimit
semantics separately and names only precomputed FileCount/TotalBytes as
terminal. Its acceptance matrix repeats the same split. Fresh scans find no
scan-limit use that conflates those meanings with the unrelated terminal raw
provider outcome lifecycle.

### C24 — Task7 drift Hard STOP was one-sided/ambiguous

One intermediate Hard STOP clause clearly rejected mapping non-drift failures
to `source_fingerprint_mismatch` but did not unambiguously require the converse:
the exact post-validation external filesystem drift must map to that code.
Reading the compressed sentence could permit either missing or contradictory
drift handling.

Disposition: frozen Task7 now stops both when any non-drift condition maps to
`source_fingerprint_mismatch` and when the exact post-validation external drift
fails to map to it. It separately keeps semantic mismatch nonretryable and both
failure classes zero-prefix. This is byte-consistent with Task4/Task5B/Task6.

### C25 — “atomic catalog context” confused composite and Analysis authority

Earlier prose called `PlatformCatalogContextV1` atomic even though Task4's
`SourceSnapshotV2` is the composite Analysis-plus-Destinations authority and
Task5B builds one context over both catalog sets and three witness sets. That
wording could allow construction from an Analysis `SourceSetSnapshotV2` alone
or make Task6 query/header authority appear detached from the composite.

Disposition: the frozen owners consistently call the context composite-bound.
`PlatformCatalogPort::build_context` accepts only `&SourceSnapshotV2`; the
context binds its composite ID, both complete catalog sets and configuration,
registered-Form and Analysis-BSL witnesses. Task6 query constructors borrow that
whole context, while provider execution receives only its exact Analysis atomic
`SourceSetSnapshotV2`. No half-context or Analysis-only construction exists.

### C26 — broad Analysis view could reopen the deleted Form seam

If `analysis_platform_catalog()` exposed the general any-source catalog view,
Task6 could regain registered-Form iteration/lookup and bypass the unified
scan-plan boundary even while query headers needed only five values.

Disposition: frozen Task5B/Task6 use the restricted context-bound
`AnalysisPlatformCatalogViewV1`, exposing only owned Analysis source identity,
borrowed source fingerprint, configuration digest, registered-Form digest and
numeric registered-Form version. All material enumeration/read remains solely
in the plan/item/dispatcher API; the richer any-source view is a separate
Task8/future-consumer surface.

### C27 — empty association scope was conflated with empty query members

A Hard STOP against “empty scope causes provider I/O” did not distinguish an
invalid empty conclusion-scope vector from a valid scheduled typed query whose
canonical member vector is empty. It also attributed the authoritative
Definition plan ambiguously between Task5A scheduling and Task6 query ownership.

Disposition: frozen Task7 now says Task5A owns the scheduled authoritative work
plan, which produces the Task6-owned typed `DefinitionQuery`. `methods=[]`
still registers/invokes once with one nonempty-scoped Invocation root. An empty
association scope is invalid, is never silently dropped and cannot itself
schedule I/O; absence of any scheduled typed query alone creates no plan.

## 6. Findings

### P0

None.

### P1

None. The exact composite-bound context/restricted Analysis header view,
one-plan selection/admission/dispatcher seam, merged limits and opaque
locations, mixed-target Definition zero-file partition, conservative one-cursor
CallGraph rule, cache-v3 reconstruction boundary, context-sensitive annotations,
numeric query version, six positive query-v3 goldens and redundant-frame
negative golden are all present and mechanically reproducible.

### Downstream Task 8 obligations — non-gating

Task 8 must consume `BslIdentifierV1`, the full header-line declaration span
and exact `&Вместо|&Instead`; it must reject `&Around`. These are not Task 6 or
four-document co-freeze prerequisites.

### P2

One documented nonsemantic opportunity remains: a future two-phase CallGraph
cursor could avoid conservative suppression by unrelated earlier material, but
only under a separately reviewed contract that preserves one deterministic
global budget. It is intentionally outside v7 and does not affect correctness.
The global workspace cache-envelope version is likewise deliberately not
guessed; implementation changes it only if live code proves it embeds the
versioned discovery DTO. Old discovery cache DTOs remain unconditional misses.

### External package gates — not Task 6 findings

This audit and the refreshed generator evidence bind the exact four-owner tuple
from section 1. Separate independent reviews and the one atomic acceptance-
ledger transition remain external requirements. Missing or stale review/ledger
evidence means the package is not accepted; it does not mutate this owner result,
create a hidden Task 6 semantic P1 or authorize production.

## 7. Negative/static audit

Old v2 encoder/path/parser strings remain only as historical negative fixtures
or deleted algorithms. Current executable-contract prose contains no rule to:

- scan a manifest/`.bsl` suffix, accept a raw FormModule path/key/tuple,
  partition Ordinary/Registered material or construct a private item index;
- build `PlatformCatalogContextV1` from an atomic Analysis snapshot/half-context,
  substitute a different Analysis atom at Task6 execution, or detach it from the
  composite snapshot ID and three witness sets;
- begin a query from a detached catalog/source/fingerprint/digest/version or
  use the general any-source catalog view/registered-Form lookup instead of the
  five-field restricted Analysis header view, or directly name Task 5B
  Form/material views/resolvers or Task 4 handles/readers;
- bypass `analysis_bsl_material_scan_plan`, select CodeSearch/CallGraph partially,
  omit Definition's supported-identity/one-available-set target partition,
  apply its limits before one exact `select_modules(&in_plan)`, or own a second
  file/byte counter/order/terminal;
- turn malformed/unsupported identity into Absent, drop valid authoritative
  zero-file absence under complete authority, treat Missing/NotApplicable as
  zero-file, accept zero-file Absent under incomplete authority, or expose a
  post-intersection selection error as anything but exact ContractViolation;
- double-count a claimed Present FormModule, hide an unsupported captured
  ordinary item, or create an item/gap for an uncaptured Form-shaped decoy;
- give CallGraph a second plan/counter/cursor/admission pass/reread or report
  Complete/an edge when the one terminal precedes caller/referenced target;
- confuse true zero-file CallGraph caller/target with terminal omission, omit the
  exact missing-caller/Named-Unresolved scoped gaps, or turn either into an edge;
- call any material reader except
  `context.read_analysis_bsl_material_verified(source_reader, snapshot, item)`,
  hide reader/root capability, or record counters outside the injected port;
- send unsupported/`FileBytesLimit` to the dispatcher, parse/read Missing bytes,
  or read a claimed Present FormModule through both registered and ordinary
  paths;
- derive evidence/gap/cache authority from a raw path rather than exact opaque
  `VerifiedBslSourceLocationV1`/`VerifiedBslCacheLocatorV1`;
- borrow/reconstruct Analysis source identity instead of consuming the owned
  `analysis.source_identity()` directly, infer numeric catalog version from
  `/v1`, or double-frame `AtomicSourceIdentityV2`;
- serialize the internal identifier/parser graph, accept wire comparison text,
  assign a call/shadow/gap spelling without verified-token semantic replay, or
  reconstruct a cache identity without exact verified bytes and the sole
  identifier constructor;
- make interceptor names global keywords, allow trivia after `&`, hide unsupported
  Unicode whitespace, or treat a moving grammar link as authority;
- classify semantic context/plan/selection/item/key/manifest disagreement as
  retryable source drift or map `registered_material_handle_mismatch` through a
  batch/gap/prefix/non-contract operation error;
- forge private catalog authority in production to copy generator mutations,
  require production code to generate design-stage goldens, or rely on Python
  assertions that disappear under `-O`;
- treat Task6 `DefinitionQuery { methods: [] }` as an empty association scope,
  skip its Task5A-scheduled invocation, give it an empty-scoped root, or
  attribute the Task5A work plan itself to Task6;
- import Task5C, Task 7 or application conclusion association, copy/vendor the
  pinned LGPL parser, omit exact fixture/dependency provenance, or publish a
  provisional query/peer hash as accepted.

## 8. Checks required on immutable bytes

```text
sha256(task-6-v2-design.md) == 5f2d859f...db4cc
Task4 owner == 1581d0b737a9e4e856526d67987a292edd39404ec5dda1cb3299c6041409cde2
Task5B owner == 30430abeb69aeb83bd665a08b41fa1837675a651b3be736936c6e4e96e14f3ad
Task6 owner == 9f488f78ba20f188e1c28e5393eb9d5d16889cde8f8ca5363bb2ea476631fca0
Task7 owner == 708022ff0b179092d5f23609449dfa8a7415adaa2e404179b9a24b43d95c1b7d
Task5B numeric registered-Form version/accessor is exact u16 value 1
PlatformCatalogPort build_context accepts composite SourceSnapshotV2 only
Task6 query borrows composite-bound context; execution receives its exact Analysis atomic snapshot
restricted Analysis header view exposes only source identity/fingerprint/two digests/numeric version
registered-Form lookup and any-source catalog authority are absent from Task6 header view
positive compile fixture uses only context + snapshot + injected reader + canonical typed modules
static/compile-fail fixtures reject private catalogs/Form views/Task4 handles/readers and raw locations
Task4 module/admission_byte_length/to_verified_location accessors are callable only by Task5B builder and never Task6
scan partition proves claimed Form once, unsupported captured ordinary once, uncaptured decoy zero
CodeSearch/CallGraph select_all; Definition select_modules occurs before limits
Definition one membership-set partition handles mixed present+absent and all permutations
valid absent CommonModule is eligible Absent only under complete authority; unsupported is never Absent
Missing/NotApplicable remain in-plan; post-intersection selection error is zero-prefix ContractViolation
one merged N/N+1 cursor owns file/byte counters and immutable terminal locations
CallGraph one stored cursor reads callers/referenced targets without second pass/reread
terminal before CallGraph caller/target is caller-scoped Bounded; same Definition target is unaffected
true zero-file CallGraph caller/target map to missing-caller/Named-Unresolved; terminal never absence proof
Definition/CallGraph zero-file and terminal cases are invariant under construction permutations
Task7 names all three configuration/registered-Form/Analysis-BSL witness sets
Task5A work plan methods=[] produces one Task6 typed query/invocation with nonempty root scope
empty association scope is invalid and distinct from an empty typed query member vector
Task7 FileBytesLimit is per-item/nonterminal; FileCount/TotalBytes alone omit suffix
Task7 drift Hard STOP rejects non-drift mapping and missing exact-drift mapping symmetrically
standalone two-path generator reproduces all six positive v3 goldens
standalone generator rejects/reproduces the redundant-frame negative golden
normal generator stdout hash is frozen and python3.12 -O exits nonzero with zero stdout/PASS
static scan finds no Python assert statement in the generator
later production smart-context tests reproduce frozen design values
ordinary/registered Present, Missing, NotApplicable/unsupported/FileBytesLimit spies match section 7.2
cache-v3 wire rebuilds every identifier from exact verified bytes and replays call/shadow/gap relations; internal graph is non-serde
exact fixture provenance and dependency/license inventory targets are complete
independent reviewer names the same tuple and has different identity
git diff --check
```

## 9. Audit decision

**PASS OWNER SELF-AUDIT / DOES NOT DECLARE PACKAGE ACCEPTANCE.** Task 6's owned
lexer/parser/provider semantics, composite-bound context/restricted Analysis
header view, unified Analysis BSL plan/item/dispatcher boundary, cache-v3
reconstruction and frozen query-v3 values are internally consistent with the
complete coordinated Task 4, Task 5B and Task 7 owner contracts at the exact
section-1 tuple. No owner-local or cross-owner P0/P1 remains. This audit and
generator evidence bind those bytes; only the external ledger can accept them
after independent-review evidence names the same tuple. Production remains
separately prohibited until all three exact implementation identities from
section 1 exist.
