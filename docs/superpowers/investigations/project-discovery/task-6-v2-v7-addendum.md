# Task 6 v2 + v7 addendum — snapshot BSL evidence

Status: **owner contract; this file never declares candidate or accepted
package state. The external package ledger is the sole design-status authority;
production is separately gated by the implementation OIDs in section 1**,
2026-07-18.

This addendum is read together with, and overrides only the conflicting clauses
of, the immutable base design:

```text
task-6-v2-design.md
SHA-256 = 5f2d859f77878b43e627930b46a99063972f0fe1a00b3bc692213beea76db4cc
```

The base file is not edited. None of the four coordinated successor documents
is frozen by this file. Freeze uses one acyclic **four-document co-freeze
candidate protocol** over exactly:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-5b-v7-contract.md
.superpowers/sdd/task-6-v2-v7-addendum.md
.superpowers/sdd/task-7-v7-addendum.md
```

First, all four owner documents stop changing. A coordinator computes their
provisional content hashes and records the tuple only in external generator/
review evidence, explicitly labelled provisional rather than accepted. Task 6
then mechanically generates its v3 query goldens against the exact stopped Task
5B context/catalog API and incorporates them into this owner contract. A
coordinator recomputes the final four-file tuple externally. Any later byte
change to any one document invalidates the four-document candidate package
tuple and every derived golden, affected
self-audit, package review and ledger claim. The SHA-256 of any unchanged file
remains mathematically correct, but it cannot stand alone as evidence for the
changed package tuple. Fresh self-audits and independent reviews inspect the final exact
candidates, after which one ledger transition accepts the complete
Task4-v7+Task5B-v7+Task6-v7+Task7-v7 design package together. The frozen owner
documents are never edited to embed their own or one another's accepted hashes.
The separate package ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` records all four
exact owner SHA-256 values plus their exact self-audit and independent-review
SHA-256 values after the files are immutable. A branch, mtime, dirty diff or
current `HEAD` is not an acceptance identity. This owner contract deliberately
contains no self/peer candidate or accepted hash.

> **For agentic workers:** REQUIRED SUB-SKILL: use
> `superpowers:subagent-driven-development` or
> `superpowers:executing-plans`; implement each RED/GREEN slice below in order.

**Goal:** make Task 6's authoritative snapshot parser and three BSL providers
conform to the Task 5B v7 identity, grouping, catalog and application-association
boundaries without importing Task 7 or waiting for Task 5C.

**Architecture:** Task 6 remains a smaller, closed, handwritten parser over
verified snapshot bytes. The pinned `bsl-analyzer` sources below are a reviewed
grammar authority and fixture oracle, not a runtime dependency and not license
to copy an unchecked parser. Provider query bytes and semantic groups contain
only provider material; Request/Proposal/Mechanism association is owned later
by Task 7's separate map.

## 1. Dependency and acceptance DAG

The four coordinated owner documents may complete the external design-package
protocol before any Task 5A production commit exists. Production is a
different gate:

```text
accepted four-document Task4/Task5B/Task6/Task7 design package
  -> TASK5A_ACCEPTED_SHA (shared v7 seams implemented and accepted)
  -> TASK4_V7_ACCEPTED_GIT_OID
     (dynamic registered-material capture/read authority implemented,
      independently reviewed and accepted)
  -> TASK5B_V7_ACCEPTED_GIT_OID (v7 provider/context implementation accepted)
  -> Task6 v2 + this addendum implementation
```

The execution-binding production edge is exact and acyclic:

```text
Task5B PlatformCatalogContextV1::execution_binding_v1
  -> Task6 CodeSearchQuery / DefinitionQuery / CallGraphQuery smart constructors
  -> Task6 owner-minted query association authorities
  -> Task7 closed six-authority validation dispatch
```

Each of the three Task6 constructors is one direct caller of the Task5B context
projection and obtains one owned `PlatformCatalogExecutionBindingV1`. Task6
imports no Task7 type; the final arrow is a downstream call-site contract only.
Because Task5B owns the projection's exact call-site whitelist, this new edge
and the corresponding Task5B whitelist addition must co-freeze in the same
four-owner package tuple. A Task6-only acceptance against the older three-query
Task5B whitelist is contradictory and forbidden.

Task 6 has **no Task 5C dependency**. In particular, it imports neither a Task
5C type nor a Task 5C commit/OID. `TASK5C_EVIDENCE_ACCEPTED_GIT_OID` is a Task 7
precondition only. Task 7 production is downstream of Task 6. Its owner-design
bytes participate only in the four-document design package; no Task 7 type,
implementation or OID may be imported, mocked as a production dependency, or
used as a Task 6 production-acceptance gate.

Before the first production RED the implementation ledger must contain exact
40-lowercase-hex values for:

```text
TASK5A_ACCEPTED_SHA
TASK4_V7_ACCEPTED_GIT_OID
TASK5B_V7_ACCEPTED_GIT_OID
```

The design may freeze while these values are not yet available; the production
implementation may not start. Whole Task 5C, Task 7 behavior, public MCP
registration, a receipt issuer and a Task 8 consumer are all outside Task 6.

## 2. Imported v7 contracts

Task 6 imports the exact accepted implementations and encoders; it must not
declare local lookalikes:

- `SourceFingerprintV1([u8; 32])`, constructed only from exact
  `sha256:<64 lowercase hex>` and rendered back canonically;
- `AtomicSourceIdentityV2`, including role and the complete logical
  `ResolvedSourceSetIdentityBytesV1`, with freshness excluded;
- `SourceScopedArtifact` and `ProviderGapScope::{Artifacts, QueryWide,
  SourceSetWide}`; `Artifacts` is a nonempty sorted unique vector of actual
  source-qualified artifacts, never a pair key or application scope;
- the closed nine-tag `SemanticAtomicGroupIdV2` registry and exact v7 group,
  semantic-record, physical-record and secondary encoders;
- `ArtifactIdentityBytesV1`, including Rust Unicode-lowercase expansion;
- the one accepted Task 5A/domain
  `ExactArtifactSpellingRegistryV1`. Task 6 uses one provider-local instance
  per invocation, constructed only through `empty_v1`, and calls the shared
  `validate_occurrence(&AtomicSourceIdentityV2, &ArtifactRef)` API; typed query
  rechecks use the same registry's read-only `require_occurrence` API. The
  registry alone derives semantic `ArtifactIdentityBytesV1` and the exact
  `u16` kind tag/canonical-ref value from the same validated artifact before
  Task 6 canonicalizes any parsed provider material. Task 6 neither declares a local
  spelling registry, supplies identity/tag/spelling fields separately, nor
  adds exact spelling to a query/group/cache identity;
- the Task 5B comparison-lowering build gate
  `ARTIFACT_IDENTITY_UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0)`, whose
  component-wise compile-time assertion against `std::char::UNICODE_VERSION`
  is the authority for `ArtifactIdentityBytesV1` lowering. Task 6 imports this
  gate; it neither restates the assertion nor substitutes its lexer table;
- the once-built composite-bound catalog context from the exact Task 5B v7 contract,
  `PlatformCatalogContextV1`,
  containing both `PlatformConfigurationCatalogSetV1` and
  `RegisteredFormCatalogSetV1`; Task 6 borrows this whole non-forgeable context,
  never accepts detached catalog copies and performs no second
  `Configuration.xml`/Form-registration parser;
- the Task5B-owned opaque `PlatformCatalogExecutionBindingV1` and sole whole-
  context projection `PlatformCatalogContextV1::execution_binding_v1()`. Each
  of the three Task6 smart query constructors calls that projection exactly
  once, owns the returned typed binding privately and never reconstructs its
  composite/configuration-set/registered-Form-set components. The binding has
  no Task6 raw/string/digest/component getter, serde path, local constructor or
  local encoder. Task6 also imports the shared closed
  `ProviderQueryAssociationViolationV1`, including exact
  `PlatformCatalogExecutionMismatch=3`; it does not redeclare the earlier
  two-variant subset;
- the sole Task 6 catalog entry point
  `PlatformCatalogContextV1::analysis_platform_catalog() ->
  AnalysisPlatformCatalogViewV1`. Every provider query constructor begins only
  with `&PlatformCatalogContextV1`, evaluates
  `let analysis = context.analysis_platform_catalog()`, takes the owned typed
  `analysis.source_identity() -> AtomicSourceIdentityV2`, and borrows only
  `source_fingerprint()`, `configuration_catalog_digest()`,
  `registered_form_catalog_contract_version()` and
  `registered_form_catalog_digest()` from that view. There is no query
  constructor from raw/detached source identity, fingerprint, digest, numeric
  version, internal catalog or authority value;
- the sole Task 6 snapshot-enumeration entry point
  `context.analysis_bsl_material_scan_plan(snapshot) ->
  AnalysisBslMaterialScanPlanV1`. Its non-reading `items()` views expose only
  `AnalysisBslMaterialScanItemKindV1::{Ordinary, RegisteredFormModule}`, an
  optional typed module and opaque `VerifiedBslSourceLocationV1`. The complete
  Task 4-captured ordinary-BSL/FormModule partition and its one total canonical
  order remain private; Task 6 never scans a manifest, tests `.bsl`, derives a
  Form suffix or sees a key/path/relationship/state/fingerprint tuple;
- the exact plan-owned scope/admission API:
  `select_all()`, `select_modules(&[ArtifactRef])`, then
  `plan.admit(selection) -> AnalysisBslMaterialAdmissionCursorV1`.
  CodeSearch and CallGraph use `select_all`. Definition first applies the exact
  base supported-module ownership/identity validator to the canonical exact
  Method vector. It projects each supported Method to its module `ArtifactRef`
  within the already-bound Analysis source, rejects two module spellings that
  are semantically equal but not byte-identical, collapses only byte-identical
  module projections repeated by different Methods, and exact-sorts the
  resulting unique module vector. It
  then uses zero-I/O `items()` to build an equality-only canonical available-
  module membership set and partitions that whole unique module vector into
  `in_plan` and valid authoritative zero-file modules.
  Missing/NotApplicable registered Form items are `in_plan`. It invokes exactly
  one `select_modules(&in_plan)` before limits and exactly one admission cursor;
  a `ModuleNotInPlan` after that intersection is an internal contract mismatch,
  while valid zero-file targets may contribute to `DefinitionAbsent` only when
  the entire selected/query-wide authority is complete. The one cursor owns the
  merged file/byte counters, immutable precomputed
  `terminal_limit()` and per-item
  `AnalysisBslMaterialAdmissionV1::{Process, FileBytesLimit}`. Task 6 owns no
  second sort, item-kind counter, terminal calculation or raw index selection.
  Task4's typed module classification, exact Present length and receipt-grade
  diagnostic location reach only the Task5B context/plan builder through
  builder-whitelisted
  `CapturedAnalysisBslMaterialHandleV1::module()` and
  `CapturedAnalysisBslMaterialHandleV1::admission_byte_length()`, plus
  `CapturedBslLocationRefV1::to_verified_location()`. Task 6 cannot name either
  Task4 capability/accessor and receives only final typed items, opaque verified
  locations, admissions and terminal;
- the sole Task 6 material read
  `context.read_analysis_bsl_material_verified(source_reader, snapshot, item)`.
  It consumes one admitted item by value and returns only
  `AnalysisBslMaterialVerificationV1::{Present, Missing, NotApplicable}`.
  Present exposes `bytes()`, its typed module,
  `location_for_range(start_byte, end_byte_exclusive)` and opaque
  `cache_locator()`; Missing/NotApplicable expose only typed module plus opaque
  diagnostic location. Task 5B privately dispatches Ordinary to Task 4's
  captured-BSL reader and RegisteredFormModule to the relationship-bound reader;
  Task 6 imports neither Task 4 handle/reader method nor Task 5B Form/material
  view/resolver. The exact injected `&dyn SourceSnapshotPort` owns all counters
  and no plan/item/context contains a reader/callback/root capability;
- any semantic context/plan/selection/item/handle/projection/state/key/ordinary-
  entry mismatch maps before I/O to exact
  `ProviderOutcome::ContractViolation("registered_material_handle_mismatch".to_owned())`
  and then only `DiscoveryError::ProviderContractViolation`. Only drift in the
  external filesystem after a semantically valid item reaches the injected port
  is retryable `source_fingerprint_mismatch`;
- `ProviderFact::stable_tag()` as the exact first field of
  `StandaloneFact`. There is no separate fact-family registry. Existing tags
  1..=12 retain their meanings and append-only
  `ScheduledJobNonPredefined` is tag 13;
- tag 8 `DefinitionObservationCluster` and its multiplicity-aware semantic
  encoder;
- the v7 source-free `CallTarget`, Definition and Support encoders.

`DefinitionPresent`/`DefinitionAbsent` for one exact queried Method always use
tag 8, not `StandaloneFact`. Every other BSL fact uses the v7 classifier before
the provider-local `max_records` ceiling. No code path may first retain records
and only then reconstruct a group.

The exact Rust paths, composite-context/restricted-Analysis-view/atomic-
snapshot/capability names, stable contract version, digest accessors and
injected-port method above are normative. The
Task 5B v7 implementation must expose them verbatim and Task 6 imports them
without a compatibility wrapper. A different implementation spelling or
framing is nonconforming and requires a separately reviewed successor owner
contract; frozen owner bytes are never edited after external package
acceptance.

## 3. Pinned grammar authority and license boundary

The compatibility authority is pinned immutably:

```text
repository = https://github.com/itrous/bsl-analyzer
tag = v0.2.55
commit = 5a02bb44dedaf29e0e29af1f740279d279199854
applicable grammar tier license = LGPL-3.0-or-later
```

The reviewed files and exact SHA-256 values are:

```text
3a05db2b2f00e61a24d5ecbd92861076e6de622900b3ea79245ef27855bb6b3d  crates/lexer/src/lib.rs
55d5c9acbb5d8a0f218a16f1b21d32fbc1312c00c58828dd7280fdea6fdbff7d  crates/parser/src/grammar.rs
a14dd0283860d74e64df8ad0fb428cbf4126e42ab2b189197e5f01482c923aa1  crates/parser/src/grammar/items.rs
77163356174a5e37fb04e901bfc69707f6b50fd59f0f04ba741b25f46ab43e9d  crates/parser/src/grammar/statements.rs
122c89847c4f3e09f9e324dfb4a82f872e011148cd63e9e5bcc7de1809383c3f  crates/parser/src/grammar/expressions.rs
4b9a9cb8a97b99a5dd194e273f46d22f9a387604b0ff62fe8603eebec71e577e  LICENSING.md
a5250ac6c47b5235c3483e1329cbd375fcece9c9ec4dd245dabf785a6b14e113  NOTICE
```

`LICENSING.md` classifies `lexer` and `parser` as Tier B,
`LGPL-3.0-or-later`. This design therefore constrains understanding, token
inventory, compatibility fixtures and review. Production remains the narrower
handwritten subset below; an implementer may not paste or vendor pinned source,
copy an unchecked parser, add `bsl-analyzer` as a runtime dependency, or claim
full-language coverage. Any later authority update requires a new addendum with
new commit, file hashes, compatibility review and license decision.

The two moving `1c-syntax/bsl-parser` `develop` links inherited from base-v2
section 17 are historical bibliography only. They are non-normative, are not an
implementation or fixture source, and must not be fetched to fill a gap in this
closed contract. The pinned `bsl-analyzer` commit/files above, this addendum and
the cited official 1C documentation are the only Task 6 grammar authorities.
Any later use of a moving branch requires a separately pinned, hashed and
licensed successor review.

All new Task 6 BSL fixture bytes are locally authored minimal synthetic inputs.
The pinned `bsl-analyzer` tests and fixtures are a behavior/grammar oracle only;
their bytes are not copied, translated or mechanically transformed into the
Unica corpus. The exact corpus provenance target is
`tests/fixtures/project_discovery/bsl/PROVENANCE.md`; it records every Task 6
fixture subtree as `locally-authored synthetic; no upstream fixture bytes`.
If an external fixture ever becomes unavoidable, acceptance stops until that
same file names its URL, immutable tag/commit, repository path, source SHA-256,
license, attribution and exact local derived-file mapping. An undocumented
copy is a release blocker even when the upstream and Unica licenses appear
compatible.

### 3.1 Pinned implementable Unicode identifier table

The pinned Logos lexer delegates `\p{L}` to regex-syntax Unicode tables. Task 6
does not depend on Logos at runtime, but prose `General_Category Letter` without
a table/version is not implementable deterministically. Production therefore
uses this exact small data authority:

```text
crate = unicode-general-category
version requirement = =1.1.0
crates.io archive SHA-256 =
  0b993bddc193ae5bd0d623b49ec06ac3e9312875fdae725a975c51db1cc1677f
Unicode data version = 16.0.0
license = Apache-2.0
```

The accepted L* predicate is exactly the union
`UppercaseLetter | LowercaseLetter | TitlecaseLetter | ModifierLetter |
OtherLetter`. It excludes `LetterNumber`, all Mark categories and every other
category. `Cargo.toml` pins the exact `=1.1.0`, `Cargo.lock` records the archive
checksum, and production defines only
`UNICODE_GENERAL_CATEGORY_VERSION: (u64, u64, u64) =
unicode_general_category::UNICODE_VERSION` with a component-wise const assertion
that it equals `(16, 0, 0)`. The exact dependency/
package-license inventory target
`docs/third-party/project-discovery-dependencies.md` records package name,
exact requirement, resolved version, archive checksum, Unicode data version,
Apache-2.0 license and Unicode-table-only purpose. That document also records
the pinned `bsl-analyzer` commit as an LGPL-3.0-or-later behavior oracle and
explicitly as no runtime dependency/copy source. The plugin's
`plugins/unica/third-party/tools.lock.json` remains the bundled-binary contract;
it is not repurposed as a Rust-library or grammar-oracle inventory. The Unicode
crate is a data table only, not a BSL parser or permission to copy the LGPL
grammar sources.

Mandatory differential REDs reject U+2160 ROMAN NUMERAL ONE (`Nl`) and U+0345
COMBINING GREEK YPOGEGRAMMENI (`Mn`) even though Rust
`char::is_alphabetic()` returns true for both; they also reject U+200C even
though XID-style classifiers may accept it as a continuation. U+10400 (`Lu`),
ASCII/Russian letters and underscore controls pass in the allowed positions.
The standalone constructor and file lexer call the same one predicate. A
toolchain Unicode update cannot silently change accepted identifiers.

This Unicode-16 table is solely the BSL lexical membership predicate for L*.
It is not the comparison-lowercase authority. Comparison of validated
`ArtifactRef` values and construction of `ArtifactIdentityBytesV1` import Task
5B's exact
`ARTIFACT_IDENTITY_UNICODE_VERSION: (u8, u8, u8) = (17, 0, 0)` gate and its
component-wise const assertion against `std::char::UNICODE_VERSION`. Thus the
two deliberate pins are separate and non-substitutable:

```text
BSL identifier L* membership = UNICODE_GENERAL_CATEGORY_VERSION (16, 0, 0)
ArtifactIdentity UnicodeLowercase comparison =
  ARTIFACT_IDENTITY_UNICODE_VERSION (17, 0, 0)
```

The Task 6 compile suite must import the Task 5B constant and fail if the
standard-library gate is absent or differs from `(17, 0, 0)`; the product-
contract suite simultaneously asserts the exact `unicode-general-category`
archive/version and `(16, 0, 0)` L* table. Neither test may infer one version
from the other, route lexical membership through `char::to_lowercase`, or route
artifact comparison through the Unicode-16 category crate.

## 4. Exact lexical subset

The base design's broad phrase “follows the primary grammar” is superseded by
this closed token inventory. Recognition is case-insensitive only for the
listed RU/EN keyword pairs. Identifier comparison still uses the v7
Unicode-lowercase rule; numeric and punctuation comparison uses exact bytes.

Default-state trivia is closed byte-for-byte:

```text
horizontal trivia = U+0020 SPACE | U+0009 TAB | U+00A0 NO-BREAK SPACE
line ending        = CRLF | bare CR | bare LF
line comment       = "//" through, but excluding, the first CR or LF
```

CRLF advances one line once; bare CR and bare LF each advance one line. A tab
and U+00A0 each advance the 1-based Unicode-scalar column by one before the next
line ending. No other Unicode `White_Space` scalar is trivia: U+000C FORM FEED,
U+0085, U+1680, U+2000..U+200A, U+2028, U+2029, U+202F, U+205F and U+3000 are
explicit unsupported tokens in a file. The optional leading UTF-8 BOM inherited
from base section 6.1 is an encoding marker removed only at byte offset zero;
the first token still has line 1/column 1 and its span starts after the three BOM
bytes. A second/interior BOM is `unsupported_bsl_encoding`. No implementation
may delegate trivia to `char::is_whitespace`, Unicode regex whitespace or a
toolchain-dependent table.

### 4.1 Literals and identifiers

```text
Identifier = [_\p{L}][_\p{L}0-9]*
Float      = [0-9]+\.[0-9]*
Decimal    = [0-9]+
Boolean    = Истина | True | Ложь | False             (case-insensitive)
Undefined  = Неопределено | Undefined                 (case-insensitive)
Null       = Null                                      (case-insensitive; EN only)
Date       = '[0-9.,: -]*'
```

`Float` is attempted before `Decimal`, so `1.` is one Float. A comma is never
a decimal separator. `Нуль`, `Пусто`, `Nil`, a signed token fused into the
number, exponents, hex and numeric suffixes are outside the subset. `+`/`-` are
separate tokens.

One-line string is `"([^"\n\r]|"")*"`. A multiline string is exactly one
`StringStart`, zero or more `StringPart` lines beginning with `|`, and one
`StringTail` beginning with `|` and ending in `"`; doubled `""` is the only
quote escape. An unterminated or structurally invalid string/date is malformed
for every capability whose negative proof could change.

```text
ConstLiteral = [Plus | Minus] (Float | Decimal)
             | complete String
             | Date
             | Boolean
             | Undefined
             | Null
```

The optional sign applies only to Float/Decimal. No identifier, call, New,
collection, expression, unary Not or parenthesized value is a default
`ConstLiteral`. A non-ConstLiteral default makes the declaration unsupported or
malformed according to whether the complete balanced extent is known; it can
never yield `DefinitionAbsent` or a compatible positive Definition by skipping
the expression.

Every structurally valid terminated `Date` is one `DateLiteral` token over its
exact source span and bytes, with empty legacy `comparison_text`. Definition
header replay accepts that token only in the parameter-default `ConstLiteral`
position. Call and source-CodeSearch replay treat it as a lexical decoy without
emitting a call, match or unsupported-token gap; query-mode lexing rejects it as
`unsupported_bsl_search_term`. An unterminated or structurally invalid date is
always `malformed_bsl_syntax` for every affected capability, never
`Unsupported` or `DateLiteral`.

### 4.2 Exact operators and punctuation

The complete accepted single/multi-character inventory is:

```text
=  <>  <=  <  >=  >  +  -  *  /  %
(  )  {  }  [  ]  .  ,  ;  :  ?  ~  |  #  &  !
```

Longest match wins for `<>`, `<=`, `>=`. Brackets and braces must balance under
the existing nesting limit. `|` is a multiline-string continuation only in the
string state; otherwise it is the punctuation token. No other operator or
punctuation is silently trivia.

### 4.3 Closed keyword inventory

Declaration/value/operator keywords:

```text
Процедура/Procedure       КонецПроцедуры/EndProcedure
Функция/Function          КонецФункции/EndFunction
Экспорт/Export            Знач/Val
Асинх/Async               Ждать/Await
Истина/True               Ложь/False
Неопределено/Undefined    Null
И/And                     Или/Or                    Не/Not
```

Control/binder keywords, recognized as keywords and never as possible callee
identifiers:

```text
Если/If                   Тогда/Then
ИначеЕсли/ElsIf           Иначе/Else                КонецЕсли/EndIf
Для/For                   Каждого/Each              Из/In
По/To                     Пока/While                Цикл/Do
КонецЦикла/EndDo          Возврат/Return
Продолжить/Continue       Прервать/Break            Перейти/Goto
Попытка/Try               Исключение/Except         КонецПопытки/EndTry
ВызватьИсключение/Raise   Перем/Var                 Новый/New
Выполнить/Execute         ДобавитьОбработчик/AddHandler
УдалитьОбработчик/RemoveHandler
```

The compiler directives and preprocessor delimiters remain exactly the paired
rows in base sections 6.4/6.5. In particular, no arbitrary `#word` or `&word`
is skipped. A balanced unknown annotation becomes the typed scoped
`unsupported_bsl_annotation`; an unknown/mismatched preprocessor delimiter is
`unsupported_bsl_preprocessor` or `malformed_bsl_syntax` before negative proof.

The four interceptor pairs in section 4.5 are context-sensitive annotation
names, not additions to the default-state keyword registry. They are recognized
case-insensitively only as the immediate no-trivia token after `&`:

```text
&Перед | &Before                       -> Before
&После | &After                        -> After
&Вместо | &Instead                     -> Around
&ИзменениеИКонтроль | &ChangeAndValidate
                                         -> ModificationAndControl
```

Thus default-state `Перед`, `Before`, `После`, `After`, `Вместо`, `Instead`,
`ИзменениеИКонтроль` and `ChangeAndValidate` remain ordinary identifiers when
they otherwise satisfy section 4.1. `&Around` is not an alias, `& Before` is not
the immediate annotation token, and both follow the unsupported/malformed
annotation rules rather than being silently accepted. The same after-`&` state
also retains only the five inherited execution-context directive pairs; it does
not turn their names into default-state keywords.

Anything outside sections 4.1-4.3, the inherited exact directive/preprocessor
states and the context-sensitive annotation rows above is either a bounded exact
`unsupported_bsl_token` for every affected capability, or
`malformed_bsl_syntax` when token/delimiter extent cannot be proven. There is no
catch-all Hidden/Error token that is dropped and no fallback substring parser.

#### 4.3.1 Complete significant-token semantics

This addendum supersedes base-v2 section 6.7's seven-value `BslTokenClass` by
appending the lossless string and date tokens required by declaration replay.
The sole authoritative numeric mapping is the complete cache-`u16` registry in
section 4.3.2; this section defines the token semantics, not a second tag table.

The one full contextual lexer emits every complete BSL string literal as one
`StringLiteral` member and every complete BSL date literal as one `DateLiteral`
member of `BslFileAnalysis.significant_tokens`; therefore the
canonical significant-token stream is the complete nontrivia token stream used
by both parser replay and cache v3. A `StringLiteral` or `DateLiteral` token's
exact authority is only its verified source span and bytes. Its internal legacy
`comparison_text` is the exact empty string and is never used for equality,
query matching or identifier construction. Definition replay decodes either
class through its sole matching literal decoder and accepts the exact complete
token as the section-4.1 `ConstLiteral`; call/search replay treats both classes
as decoys. Comment/trivia bytes remain outside the stream; every other
unsupported literal remains the exact `Unsupported` token/gap required by the
base contract unless a later encoder version adds a closed class.

Query-mode string and date literals remain rejected exactly as in base section
8; adding parser token classes does not make either a CodeSearch term.
Interceptor replay requires class tag 8, decodes that one verified complete
token through the sole BSL string decoder and never invokes a second lexer.
Cache wire `class_tag` follows section 4.3.2. REDs cover empty/
nonempty/multiline/escaped-quote strings, empty/nonempty date literals, both
classes beside call/search decoys, a string beside an interceptor target, a
forged span inside either literal, every 7/8/9 class substitution, query-mode
rejection, Definition-default acceptance, and parse -> egress -> ingress ->
egress equality. A compile/static RED restores the base seven-class or prior
eight-class enum, or introduces a second lossless token stream, and must fail.

#### 4.3.2 Complete closed cache `u16` registry

This is the one authoritative registry for **every** `u16` enum tag serialized
inside `BslCacheEntryV3`. All eight mappings are closed and append-only;
declaration order and Rust discriminants are not wire authority:

```text
BslSignificantTokenCacheWireV3.class_tag / BslTokenClass:
  Identifier=1 | Keyword=2 | Number=3 | Boolean=4 |
  UndefinedOrNull=5 | Punctuation=6 | Unsupported=7 |
  StringLiteral=8 | DateLiteral=9
  accepted range: 1..=9; reject 0 and 10

BslSyntaxCallCacheWireV3.syntax_tag / BslCallSyntax:
  Direct=1 | Qualified=2 | Execute=3 | AccessChain=4 | Unsupported=5
  accepted range: 1..=5; reject 0 and 6

BslSyntaxGapCacheWireV3.capability_tag / BslCapability:
  Search=1 | Definition=2 | Call=3 | All=4
  accepted range: 1..=4; reject 0 and 5

BslSyntaxGapCacheWireV3.reason_tag / BslSyntaxGapReason:
  UnsupportedAnnotation=1 | UnsupportedPreprocessor=2 |
  ConditionalCompilation=3 | ConditionalSymbolEffect=4 |
  UnsupportedToken=5 | UnsupportedShadowBinder=6 |
  UnsupportedCallSyntax=7 | ModuleLevelCall=8 |
  UnsupportedDefinitionLayout=9
  accepted range: 1..=9; reject 0 and 10

BslSyntaxDefinitionCacheWireV3.declaration_line_ending_tag / BslLineEnding:
  Lf=1 | Crlf=2 | Cr=3
  accepted range: 1..=3; reject 0 and 4

DefinitionShapeCacheWireV3.context_tag / BslExecutionContext:
  ModuleDefault=1 | AtServer=2 | AtClient=3 | AtServerNoContext=4 |
  AtClientAtServer=5 | AtClientAtServerNoContext=6
  accepted range: 1..=6; reject 0 and 7

BslObservedCfeInterceptorCacheWireV3.kind_tag / ObservedCfeInterceptorKind:
  Before=1 | After=2 | Around=3 | ModificationAndControl=4
  accepted range: 1..=4; reject 0 and 5

BslObservedCfeInterceptorCacheWireV3.presence_tag / BslObservedSyntaxPresenceV1:
  Active=1 | Conditional=2 | Deleted=3
  accepted range: 1..=3; reject 0 and 4
```

`UnsupportedDefinitionLayout=9` is the sole append to the base-v2 gap-reason
registry. Complete dates likewise append only `DateLiteral=9`; neither append
renumbers an inherited row.

The permanent numeric-registry RED matrix starts from a valid full cache entry
that contains at least one row for every registry above and proves all valid
tags roundtrip parse -> cache egress -> ingress -> egress byte-for-byte. For
each registry independently it then mutates only that field to zero and to its
exact `N+1` value (`10`, `6`, `5`, `10`, `4`, `7`, `5`, `4` respectively).
Every one of these sixteen mutations rejects the whole entry before internal
graph construction and selects local parsing. Separate within-range mutation
REDs prove a semantically wrong but numerically valid tag is also rejected by
full replay rather than trusted. Renumbering, accepting an unknown value,
assigning one integer to two rows, omitting any of the eight registries from the
matrix, or publishing a second numeric table is a contract break.

### 4.4 One standalone BSL identifier authority

Task 6 owns the one standalone identifier constructor later reused by Task 8.
It is a lexer view, not a second identifier grammar and not a fabricated
artifact:

```rust
// Private fields; no serde/raw/unchecked constructor.
pub(crate) struct BslIdentifierV1 {
    exact_spelling: Box<str>,
    comparison_text: Box<str>,
}

pub(crate) fn parse_complete_bsl_identifier_v1(
    input: &str,
) -> Result<BslIdentifierV1, BslIdentifierErrorV1>;

impl BslIdentifierV1 {
    pub(crate) fn exact_spelling(&self) -> &str;
}

pub(crate) enum BslIdentifierErrorV1 {
    Utf8ByteLimitExceeded,       // stable tag 1
    UnicodeScalarLimitExceeded, // stable tag 2
    Empty,                      // stable tag 3
    TriviaNotAllowed,           // stable tag 4
    KeywordNotIdentifier,       // stable tag 5
    InvalidFirstScalar,         // stable tag 6
    InvalidContinuationScalar,  // stable tag 7
    AdditionalToken,            // stable tag 8
}
```

`parse_complete_bsl_identifier_v1` invokes the exact same default-state Task 6
lexer/token classifier as a BSL file. It accepts exactly one non-keyword
`Identifier` token followed immediately by EOF. It does not call `trim`, remove
a BOM, normalize Unicode, rewrite case or accept comments/whitespace as harmless
trivia. The retained spelling is the exact UTF-8 input. Its private
`comparison_text` is the already-produced lexer identifier comparison text;
the constructor does not run a second lowercase implementation. Equality,
ordering and hashing of `BslIdentifierV1` use that comparison text, while the
exact accessor remains available for source-spelling checks and rendering.

The closed grammar is exactly:

```text
first scalar        = underscore | Unicode General_Category Letter (L*)
continuation scalar = underscore | Unicode General_Category Letter (L*)
                    | ASCII digit 0..9
UTF-8 bytes         = 1..=512
Unicode scalars     = 1..=128
```

`char::is_alphabetic`, XID_Start/XID_Continue, Unicode normalization, a Unicode
digit other than ASCII `0..9`, and an unversioned regex approximation are not
substitutes for the lexer's exact classifier. Error precedence is byte limit,
scalar limit, empty, trivia, keyword, invalid first scalar, invalid
continuation scalar, then additional token. Errors carry no raw attacker text.
Every default-state RU/EN keyword in section 4.3 is rejected in arbitrary case;
an identifier merely resembling a keyword remains accepted only when the same
lexer classifies it as Identifier.

Required boundary REDs use deterministic builders and first assert their
preconditions:

- 128 copies of U+10400 DESERET CAPITAL LETTER LONG I are exactly 512 UTF-8
  bytes and 128 `General_Category=Lu` scalars and pass;
- 129 copies return `Utf8ByteLimitExceeded` under the fixed precedence;
- 127 copies of U+10400 plus five ASCII `A` values are exactly 513 bytes and
  return `Utf8ByteLimitExceeded`;
- 128 ASCII `A` scalars pass, while 129 return
  `UnicodeScalarLimitExceeded`;
- empty returns `Empty`; leading/interior/trailing SPACE, TAB, U+00A0, CRLF,
  bare CR, bare LF or comment returns `TriviaNotAllowed`; leading U+000C returns
  `InvalidFirstScalar` and trailing/interior U+000C returns
  `InvalidContinuationScalar`, while the file lexer reports that same scalar as
  an unsupported token rather than trivia; every RU/EN keyword and case variant returns
  `KeywordNotIdentifier`; `1Name` and a leading BOM return
  `InvalidFirstScalar`; a combining-mark/emoji/non-ASCII-digit continuation
  returns `InvalidContinuationScalar`; `Name-Other`, `Name.Other` and `Name()`
  return `AdditionalToken`; `_`, `A9`, `Имя`, and `İ` pass;
- the file lexer token and standalone constructor return byte-identical exact
  spelling/comparison identity for every accepted fixture.

Task 8's `CfeIdentifier` is only a type alias to this type and adds no
constructor, serde view, bound, keyword list or equality implementation.

### 4.5 Parser-owned exact source slices and interceptor observations

The base `BslSyntaxDefinition` is extended, never replaced. `name`, `shape`,
`local_shadow_names` and `maybe_local_shadow_names` retain their accepted v2
meaning; all new fields are produced by the same parse pass:

```rust
pub(crate) struct BslSyntaxDefinition {
    pub(crate) name: String,
    pub(crate) name_identity: BslIdentifierV1,
    pub(crate) definition_span: BslSpan,
    pub(crate) declaration_span: BslSpan,
    pub(crate) name_span: BslSpan,
    pub(crate) parameter_list_span: BslSpan,
    pub(crate) body_span: BslSpan,
    pub(crate) terminator_span: BslSpan,
    pub(crate) declaration_line_ending: BslLineEnding,
    pub(crate) shape: DefinitionShape,
    pub(crate) local_shadow_names: Vec<String>,
    pub(crate) maybe_local_shadow_names: Vec<String>,
    pub(crate) local_shadow_observations: Vec<BslShadowIdentifierObservationV1>,
    pub(crate) maybe_local_shadow_observations: Vec<BslShadowIdentifierObservationV1>,
}

pub(crate) struct BslShadowIdentifierObservationV1 {
    pub(crate) identifier: BslIdentifierV1,
    pub(crate) source_span: BslSpan,
}

pub(crate) struct BslSpan {
    pub(crate) start_byte: u32,
    pub(crate) end_byte_exclusive: u32,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

pub(crate) enum BslLineEnding {
    Lf,   // exact bytes 0a; wire tag is owned by section 4.3.2
    Crlf, // exact bytes 0d0a; wire tag is owned by section 4.3.2
    Cr,   // exact bytes 0d; wire tag is owned by section 4.3.2
}

pub(crate) enum ObservedCfeInterceptorKind {
    Before,                 // &Перед | &Before
    After,                  // &После | &After
    Around,                 // &Вместо | &Instead
    ModificationAndControl,
                            // &ИзменениеИКонтроль | &ChangeAndValidate
}

pub(crate) enum BslObservedSyntaxPresenceV1 {
    Active,
    Conditional,
    Deleted,
}

pub(crate) struct BslObservedDefinitionAnchorV1 {
    pub(crate) name: String,
    pub(crate) name_identity: BslIdentifierV1,
    pub(crate) name_span: BslSpan,
    pub(crate) definition_span: BslSpan,
    pub(crate) shape: DefinitionShape,
    pub(crate) active_definition_index: Option<u32>,
}

pub(crate) struct BslObservedCfeInterceptorV1 {
    pub(crate) kind: ObservedCfeInterceptorKind,
    pub(crate) target_name: BslIdentifierV1,
    pub(crate) annotation_span: BslSpan,
    pub(crate) target_argument_span: BslSpan,
    pub(crate) attached_definition: BslObservedDefinitionAnchorV1,
    pub(crate) presence: BslObservedSyntaxPresenceV1,
}
```

The three enum-to-cache mappings above are owned exclusively by the complete
section-4.3.2 registry; these semantic declarations do not derive wire tags
from variant order.

`BslFileAnalysis` adds a canonically span-sorted
`observed_cfe_interceptors: Vec<BslObservedCfeInterceptorV1>`. The annotation
names are case-insensitive under the same lexer; `&Around` is not an English
alias and remains an unsupported custom annotation. The eight RU/EN annotation
spellings are tokens only in the immediate after-`&` state defined in section
4.3; each same spelling in default state remains an Identifier. The annotation parser
accepts exactly one complete BSL string argument whose decoded content
passes `parse_complete_bsl_identifier_v1`, no second argument and no executable
token before the attached Procedure/Function declaration. It preserves the
bounded exact target spelling and raw argument span. An Active observation
points to exactly one equal positive definition through
`active_definition_index`; Conditional/Deleted observations have `None` and a
syntax anchor but never enter the positive definition vector. A conditional
target blocks complete negative duplicate proof. A Deleted observation is an
explicit decoy and never blocks. An orphan, malformed, unsupported or
ambiguously attached active/conditional annotation yields the existing exact
scoped annotation/preprocessor gap before negative proof; malformed deleted
text remains non-executable and cannot create a positive or a gap outside the
deleted region.

The spans obey all of these byte-level invariants against the exact verified
file bytes:

```text
definition.start == declaration.start
definition contains declaration, body and terminator
declaration contains name and parameter_list
name.end <= parameter_list.start
parameter_list includes both opening and closing parentheses
declaration.end + encoded(declaration_line_ending).len == body.start
body.end == terminator.start
terminator.end == definition.end
```

`definition_span` starts at optional `Async` or the Procedure/Function token,
excluding preceding context/interceptor annotations, and ends immediately after
the matching terminator token. `declaration_span` is the entire exact header
line from that start through accepted trailing spaces/comment, ending
immediately before its LF/CRLF/CR. This definition is intentional: stopping at
`)`/`Export` would contradict the required declaration-to-body equality for a
valid trailing comment. `name_span` is token-tight;
`parameter_list_span` includes both parentheses; `body_span` begins after the
exact header line ending and ends at the terminator start; `terminator_span` is
token-tight. No span includes a leading BOM or trailing terminator-line trivia.
Line and column remain the 1-based Unicode-scalar start coordinate from base
v2; every byte offset is on a UTF-8 scalar boundary.

The successor extraction subset requires one of those three physical line
endings after the declaration header. A balanced one-line spelling such as
`Procedure P() EndProcedure` is not squeezed into a fake enum value: it yields
the append-only syntax gap `UnsupportedDefinitionLayout`, mapped to exact reason
`unsupported_bsl_definition_layout`, for Definition/Call/extraction capability,
emits no positive `BslSyntaxDefinition` for that declaration and cannot support
negative absence. It need not invalidate unrelated completely parsed methods.
The N/N+1 and RU/EN suites include this row. Any future inline-layout support
requires a versioned line-ending/renderer contract, not an implicit `None`.

`name == name_identity.exact_spelling()` is mandatory. A pure file parser cannot
construct `ArtifactIdentityBytesV1`: that type requires a fully validated,
module-qualified `ArtifactRef`, while the parser intentionally has no path or
catalog. The provider/material projection constructs the full method
`ArtifactRef` only after exact module mapping and then calls the sole domain
`ArtifactIdentityBytesV1::try_from_artifact`. A fake Method artifact made from
the bare name is forbidden.

The semantic shadow vectors retain the base accepted closed binder set:
parameters, `Var`, bare/access/index assignment first identifiers, `For`, and
`For Each`. Unconditional binders populate Unicode-lowercase
`local_shadow_names`; conditional/preprocessor-uncertain binders populate the
conservative Unicode-lowercase `maybe_local_shadow_names`. These semantic
vectors are sorted unique by the identifier's lexer-produced comparison-text
UTF-8 bytes, not by exact source spelling. They are scope-wide rather than
source-order dependent and are disjoint only when syntax proves disjointness;
when a comparison name is both definite and maybe it stays in both because
deleting uncertainty would be unsound.

The parser additionally retains every physical shadow observation losslessly.
`BslFileAnalysis` appends separate `module_shadow_observations` and
`maybe_module_shadow_observations` vectors of
`BslShadowIdentifierObservationV1`; each `BslSyntaxDefinition` has the two
local vectors shown above. Module observations never enter a definition-local
vector, local observations belong to exactly one definition, and definite and
maybe observations remain in separate vectors. Within each vector every
recognized binder occurrence is retained, including repeated exact spellings
and comparison-equal spellings at different spans. Rows are strictly ordered
by validated source-span key
`(start_byte, end_byte_exclusive, line, column)`. Equal keys, overlapping token
spans, a row outside its owning module/definition scope, or the same physical
token appearing in both the definite and maybe vector of one scope reject the
analysis; therefore no exact-spelling tie-break is permitted. Semantic names
may still occur in both sets when they came from different definite and
uncertain observations. Each span must be token-tight for exactly one local
Identifier token, its verified UTF-8 slice must equal
`identifier.exact_spelling()`, and that slice must pass the sole standalone
identifier constructor. Only after all observations pass these checks does the
parser derive the four Unicode-lowercase sorted-unique semantic vectors from
their `BslIdentifierV1` comparison text. An accepted binder shape that cannot
be classified creates `unsupported_bsl_shadow_analysis` and blocks qualified
negative proof.

Exact extraction REDs cover RU/EN/mixed declarations, Procedure/Function,
Async, Export, zero/N parameters, Val/default, BOM, Unicode names, each context,
LF/CRLF/bare-CR, header trailing spaces/comment, empty/nonempty bodies and every
span slice. Forged cache DTOs independently mutate each start/end, coordinate,
containment, token boundary, line ending, name/identity, active-definition
index, annotation target and content digest and must be rejected. Duplicate
identical and conflicting active definitions retain two distinct validated
span observations; an active plus conditional same-name declaration is not a
unique extraction; conditional/deleted/malformed annotation fixtures exercise
all four observed interceptor kinds and all three presence tags.
They additionally prove every one of the eight interceptor spellings is an
ordinary callable/declaration Identifier in default state, the corresponding
immediate `&Name` spelling selects exactly one interceptor kind, trivia between
`&` and the name is rejected, `&Around` is unsupported, U+00A0 is the only
non-ASCII horizontal trivia, and U+000C/other Unicode whitespace cannot be
silently hidden.
Shadow REDs retain two distinct observations for `I` and `i` while deriving the
single semantic name `i`; retain repeated `I` occurrences at distinct spans;
and retain `İ` exactly while deriving the expanding lowercase comparison text
`i\u{307}` separately from `i`. Source-order permutations yield the canonical
span order. Duplicate/equal spans, overlapping spans, wrong module/local
ownership, cross-definite/maybe reuse, forged spelling or coordinates, and a
spelling/span slice mismatch all reject before semantic-set derivation.

## 5. Definition tag 8 contract

Task 6 constructs one `DefinitionObservationCluster` per exact analysis-source
queried Method. The secondary payload is the exact zero-byte `empty` payload,
not `vec([])`. Query association and conclusion scopes do not enter its key.

After complete physical-record construction and exact byte-identical physical
deduplication, the cluster selects exactly one polarity:

```text
Present -> one or more SourceFreeDefinitionShapePayloadV2 values, each with
           declaration_observation_count in 1..=u32::MAX
Absent  -> explicit empty shape vector, only under a complete exact query with
           zero physical declaration observations
```

Two location-distinct identical definitions are two observations and therefore
count 2. Two distinct shapes are both retained. Neither is a provider contract
failure. Present+Absent, Absent plus a declaration, overflow, zero count,
unvalidated query association or a non-Method subject is a contract violation
before any ceiling. EvidenceGraph later derives duplicate/conflicting status
from the retained whole group.

The implementation imports and recomputes all v7 tag-8 goldens:

```text
Present shape A x1 digest = cc6c6bd22f3621d4bb84286f9abfeb78ff206d6dbc56944b76ca7c2f673c6d30
Present shape A x2 digest = 916612e064dfc60e20f3139f9989323460096833bb8daa1ce86bcb5e735f45ba
Present shape A x1 + shape B x1 digest = 6e07e455da95ae876da3a32221b4a824d9057391ae9ba0ecd55afa265cc2e5ac
Absent empty digest = 26842aeb66c8194bb8e4bd9446c3342bbe7768d960b693f9068738cfd7a4aea5
```

Required REDs include exact duplicate physical bytes collapsing to x1,
location-distinct identical declarations producing x2, two shapes, Present plus
Absent rejection, absent under bounded/unavailable query rejection, physical
permutation invariance, and `max_records=N/N+1` retaining/dropping tag 8 whole.

## 6. Call target and constructor closure

Task 6 uses v7 `SourceFreeCallTargetPayloadV2` tags:

```text
Artifact = 1
Named(one or two exact Identifier segments, Unicode-lowercase) = 2
Dynamic = 3
```

The constructor accepts the following **total** subset for each of the six
closed `BslExecutionContext` values:

```text
Resolved   + Artifact + (Direct | Method) + object=Some(equal Artifact)
Ambiguous  + Artifact + (Direct | Method) + object=Some(equal Artifact)
Unresolved + Named    + (Direct | Method) + object=None
Dynamic    + Dynamic  + Dynamic           + object=None
```

Every other Cartesian tuple, including Callback, Resolved+Named,
Unresolved+Artifact, Dynamic+Named, non-Dynamic call type with Dynamic target,
wrong/missing object and Named with zero/three segments, is a constructor error.
Task 6 must generate the full Cartesian RED, not infer safety from four positive
examples. It also imports the exact v7 Named case-folding and `İ` goldens:

```text
Named("Missing") digest = ead5260baa60573ac62283dafc24f87f4e8110f17d4895feeab726fa92e14d67
Named("Other") digest = 622074f83867e30d1d8333b34b8789e54fd0a9e307f64ff54d979903d4c59e09
Named("İ", "MISSING") digest = 43d170188a4ba960a05f2d2b5beee7f2802552c7058cea4e22bdd351fed495
Dynamic digest = ee4df215dfb6ecdb1cd68ddc5c1dfb56dd1383efa18ff509e0fcfc632f993207
```

Only Resolved+Artifact may create `FlowKind::Calls`; Ambiguous's artifact is a
candidate identity for testimony, never a flow edge.

## 7. Query identity and association-root correction

`DefinitionQuery.methods` contains methods only. Request, Proposal, Mechanism,
`ConclusionScope`, Task 7 material association and proposal IDs are absent from
all three Task 6 query payloads, every provider-local group key, semantic
cluster, physical record and provider outcome.

The base v2 BSL query is nevertheless **superseded**, not retained. Its sole
configuration-catalog digest cannot bind FormModule mapping after Task 5B v7
moves nested Form registration and the opaque module manifest key into the
separate registered-Form sidecar. All three queries use one version together:

```text
BSL_PROVIDER_QUERY_ENCODER = "snapshot-bsl-provider-query/v3"

payload = u16be(port tag)
  || encode(analysis.source_identity())
  || fingerprint32(analysis.source_fingerprint())
  || digest32(analysis.configuration_catalog_digest())
  || u16be(analysis.registered_form_catalog_contract_version())
  || digest32(analysis.registered_form_catalog_digest())
  || vec(port-specific terms | methods | callers)
  || u16be(max_records)

query_digest = H("unica.snapshot-bsl-provider-query/v3", payload)
```

Every query additionally owns one private
`execution_binding: PlatformCatalogExecutionBindingV1`. That field is
authority only: it is excluded from this payload, query digest, cache key,
provider raw outcome, semantic/physical group identity, association-map bytes
and receipt identity. Query `PartialEq`/`Eq`/`Hash` and cache lookup continue to
use only the pre-existing v3 canonical identity fields; they must not derive
over the binding field. The six published v3 payloads and digests therefore
remain byte-for-byte unchanged; adding a binding field or frame to the
standalone generator is a contract failure.

The owned typed `AtomicSourceIdentityV2` returned by
`analysis.source_identity()` is appended directly. It is already
`u16be(role) || bytes(ResolvedSourceSetIdentityBytesV1)` and therefore must not
receive a second outer `bytes(...)` frame. V3 retains the base-v2 source-field
framing and adds only the numeric registered-Form contract version plus its
catalog digest. A double-framed source identity is a mandatory negative golden.

Each smart constructor accepts the whole exact accepted v7 composite-bound
catalog context as `&PlatformCatalogContextV1` and no lesser catalog argument.
Its first steps are exactly:

```text
let execution_binding: PlatformCatalogExecutionBindingV1 =
  context.execution_binding_v1();
let analysis = context.analysis_platform_catalog();
let source_identity: AtomicSourceIdentityV2 = analysis.source_identity();
```

The projection is called exactly once per smart construction, after entry
through the whole smart context and before any query member is canonicalized or
provider I/O is possible. A second projection call, a caller-supplied binding,
or reconstruction from the query header is forbidden. Context construction has
already validated the composite and both catalog sets; the query retains this
owned binding as the provenance seal for those borrowed header authorities.
At Task7 registration the owner authority's fourth operation below compares it
with the one registry-owned execution binding. Thus a query built from another
context/composite or either catalog-set authority rejects before invocation
allocation or I/O even when all port-specific members are equal.

For `DefinitionQuery` and `CallGraphQuery`, the constructor first performs one
constant-time checked raw-vector length test: `0..=2_000` is admitted and
`2_001+` rejects atomically before any per-element work, even if every element
is byte-identical. Within that admitted raw cardinality it creates one shared
`ExactArtifactSpellingRegistryV1::empty_v1()` and passes every raw input
Method/caller occurrence with the exact Analysis source identity to
`validate_occurrence` **before** any identity sort, set insertion,
deduplication or encoding. Raw cardinality failure has fixed precedence;
exact-spelling collision has precedence over semantic ordering/deduplication
only inside an admitted raw vector. This clause supersedes base Task 6 v2
section 5's direct identity-dedup rule. Byte-identical duplicate admitted
inputs may then collapse by `ArtifactIdentityBytesV1`; semantic-equal but
exact-different admitted inputs reject atomically before I/O in either order.
CodeSearch terms remain exact strings under their separate existing contract.
The later
invocation-local registry still begins with the accepted query members and then
validates every raw response occurrence, so a response cannot change the
query's spelling.

The two artifact-bearing Task 6 smart queries additionally expose the sealed
zero-I/O owner check used by Task7 registration:

```rust
impl DefinitionQuery {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}

impl CallGraphQuery {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

Each method visits every private canonical Method/caller with the query's exact
Analysis `AtomicSourceIdentityV2` and calls only
`registry.require_occurrence`; it returns no member/iterator/delta and cannot
add a missing spelling. `CodeSearchQuery` has no artifact member and therefore
no such projection. Omission/mutation REDs prove the visit is exhaustive and
query bytes/digests remain unchanged.

All three Task 6 smart queries additionally mint one sealed owned query-
association authority. This is an application-validation capability, not a
second query encoder and not a Task7 dependency:

```rust
impl CodeSearchQuery {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> CodeSearchQueryAssociationAuthorityV1;
}
impl DefinitionQuery {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> DefinitionQueryAssociationAuthorityV1;
}
impl CallGraphQuery {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> CallGraphQueryAssociationAuthorityV1;
}
```

Each `association_authority_v1` call copies only the already stored opaque
binding into the newly minted non-Clone authority; it never calls
`execution_binding_v1` again and has no context argument. Exact Task7 typed
registration remains the sole production minting call site per query owner and
may invoke it only once for one registration.

The three capability types are `pub(crate)` solely for the Task7 application
consumer; every field stays in its Task6 query-owner module. Each concrete type
has exactly the following borrowed API (shown once with `A` standing for that
type):

```rust
impl A {
    pub(crate) fn query_digest(&self) -> Digest32;
    pub(crate) fn validate_execution_binding_v1(
        &self,
        execution_binding: &PlatformCatalogExecutionBindingV1,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
    pub(crate) fn validate_source_group_v1(
        &self,
        source: &AtomicSourceIdentityV2,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
    pub(crate) fn validate_material_v1(
        &self,
        material: &ProviderGroupMaterialIdentityV2,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
}
```

No capability field/constructor is `pub(crate)`, no type is defined in
infrastructure, and no Task7 type is imported.

Each returned owner type has private fields and no raw constructor, Clone,
serde or member iterator. It owns the exact typed query digest, an equal clone
of the query's one opaque `PlatformCatalogExecutionBindingV1`, the exact
Analysis source group and the complete canonical query-time
`ProviderGroupMaterialIdentityV2` membership as sorted unique shared typed
values, never `digest+count`, paths, cache locators or the BSL scan-plan material
cohort. Snapshot freshness and raw-outcome completeness remain the separate
context/reader/owner-response contracts. It exposes only read-only
`query_digest()`,
`validate_execution_binding_v1(&PlatformCatalogExecutionBindingV1)`,
`validate_source_group_v1(&AtomicSourceIdentityV2)` and
`validate_material_v1(&ProviderGroupMaterialIdentityV2)`; all three validators use
the shared accepted Task5B-v7 application/query-boundary closed, non-serde
`ProviderQueryAssociationViolationV1`, whose exhaustive internal variants are
`SourceGroupNotMember=1`, `MaterialNotMember=2` and
`PlatformCatalogExecutionMismatch=3` and never serialize.
CodeSearch admits the exact Analysis
source group and no pre-I/O material root because its terms are not artifacts.
Definition admits only the exact Analysis source-scoped Method material for
each canonical queried method. CallGraph admits only the exact Analysis source-
scoped Method material for each canonical queried caller. Unknown, omitted,
foreign-source, foreign-kind and semantic-equal/exact-authority-different
values reject. `validate_execution_binding_v1` compares the complete opaque
typed binding by its Task5B-owned `Eq` implementation and maps inequality only
to `PlatformCatalogExecutionMismatch`; it returns no binding, component,
digest, byte, context member or comparator choice. It cannot be replaced by
query-digest, port or pointer equality.

Task7 may only wrap these owner values in its own closed enum and move the same
non-cloneable value from plan to finished registry entry. Provider-returned
material remains subject to the recorded typed outcome's post-I/O membership
check; an authority cannot bless an unreturned value. The authority and its
private membership never enter query payload/digest, provider/cache/group/
receipt identity or serialization. Permanent mutation/static REDs independently
omit every method/caller, substitute a foreign source/material, swap any two
owner authorities, mutate digest/port, substitute a binding from another
context/composite or either catalog-set authority, expose Clone/serde/raw
construction or make Task7 trust digest equality as membership. None changes a
published query golden. The three owner validators are the only Task6
production callers of typed binding equality for this purpose; Task7's closed
six-authority dispatch is the only production caller of each fourth operation,
once before invocation allocation and again through the registry-owned finish
recheck. Mechanical owner tests are the only additional callers.

This downstream whitelist is phased. At Task6 implementation acceptance the
three authority minting methods and fourth operations have zero downstream
production callers; their exact Task7 typed-registration and closed-dispatch
names are reserved by the co-frozen owner contracts, not compiled prerequisites.
Task7 acceptance activates only those reserved paths and reruns the same static
call-site checks. Task6 never imports Task7 to manufacture an early caller.

Permanent constructor REDs cover Definition and CallGraph in both orders,
ASCII case aliases, the expanding-Unicode `İ` identity case, byte-identical
duplicates, raw length 2,000/2,001, and each isolated variant. The 2,001-row
case rejects before inspecting a later collision or collapsing duplicates;
within 2,000 rows collision rejects before semantic dedup. Either isolated
variant produces the same existing semantic query bytes/digest; no published
v3 golden changes.

It appends that owned value directly and borrows the source fingerprint, both
catalog digests and numeric registered-Form contract version through the same
`AnalysisPlatformCatalogViewV1`. The context already validates equal
composite/source identity, fingerprint, source coverage/order and sidecar-to-
configuration digest binding. There is no impossible borrow of a computed
source-identity temporary and no constructor accepting caller-provided source,
digest strings, numeric version, detached catalog or raw authority.
All three queries bind the sidecar even when a particular requested Method is
not a Form method: CodeSearch scans the complete registered BSL surface, and a
single uniform identity prevents a cached provider outcome from being
reinterpreted under a different mapping authority.

The old v2 220-byte Definition fixture and digests are historical negative
fixtures. Product tests reject their use after this addendum. The exact v3
empty and nonempty CodeSearch/Definition/CallGraph payload bytes, lengths,
SHA-256 values and domain-separated query digests below are normative and were
generated mechanically from the exact Task 5B v7 context/catalog grammar.
External generator/review evidence binds that run to the exact four-document
tuple; this owner file contains no tuple or acceptance hash.

### 7.1 Mechanical query-v3 golden generator specification

The design-stage golden publication is a deterministic standalone generated
artifact, not a manual calculator exercise and not a Task 6 production test.
Production intentionally cannot exist before the design co-freeze and its
Task5A/Task4/Task5B implementation prerequisites. Requiring production smart
queries here would create a design-to-production cycle.

At content seal, run one stdlib-only checked standalone generator outside every
production crate/test module. Its exact executable bytes and command/output are
recorded as package-review evidence. It implements two independently coded
paths and requires byte equality before printing a hash:

1. an imperative encoder appends each field from the v3 grammar directly;
2. a declarative schema walker encodes the same typed fixture from a closed
   field/tag description, without calling path 1's field helpers.

Neither path imports future Task 5B/Task 6 production code. Both implement the
published shared framing primitives exactly and fail on unknown fields/tags,
overflow, noncanonical digest/fingerprint spelling or unsorted/duplicate final
vectors.

The design authority fixture uses only exact already-published Task 5B/Task 5A
golden authorities, not an arbitrary detached digest:

```text
Analysis ResolvedSourceSet:
  name = "analysis"
  kind = Configuration
  source_format = PlatformXml
  relative_root = "."
  mapping_digest = "sha256:" + ("a" * 64)
Analysis role tag = 1
analysis source fingerprint = "sha256:" + ("b" * 64)
analysis configuration catalog digest =
  279d317b18203fa02829d9dbfa19359913e310bddf3beee5bfd82fc5240046b9
analysis empty registered-Form catalog digest =
  cc7b8add787c08ad7678218574e5a9a55395c7959440208f9a635ed5ab222cd2
registered-Form catalog contract version =
  REGISTERED_FORM_CATALOG_CONTRACT_VERSION: u16 = 1
EvidencePort stable tags: CodeSearch=2, Definition=3, CallGraph=4
ArtifactKind::Method stable tag = 6
```

Both paths append the 148-byte Analysis `AtomicSourceIdentityV2` encoding
directly after the port tag. They must independently reject the variant with an
extra outer four-byte length frame.

The numeric contract version is read from the content-sealed Task 5B contract
by the design generator author and written visibly into the generator fixture;
it is not inferred from `/v1`. This explicit design fixture is not a production
query constructor. Later production constructors must derive the same value and
both digests from their one borrowed `PlatformCatalogContextV1`.

The six frozen rows are:

| Query | Canonical port-specific vector | `max_records` |
| --- | --- | ---: |
| CodeSearch empty | `[]` | 7 |
| CodeSearch one | exact term `Needle` | 7 |
| Definition empty | `[]` | 7 |
| Definition one | validated Method `CommonModule.Flow.Run` | 7 |
| CallGraph empty | `[]` | 7 |
| CallGraph one | validated Method `CommonModule.Flow.Run` | 7 |

The one Method vector encodes
`ArtifactIdentityBytesV1 = u16be(6) || string("commonmodule.flow.run")`; the
exact term vector preserves `Needle` byte-for-byte. For every row both design
paths emit and compare all four outputs:

```text
complete payload bytes as lowercase hex
payload byte length
SHA-256(payload bytes)
H("unica.snapshot-bsl-provider-query/v3", payload bytes)
```

The design-stage two-path generator reproduced the published 142-byte
`ResolvedSourceSetIdentityBytesV1` and 148-byte `AtomicSourceIdentityV2`
goldens before constructing these rows. Both independent encoders then agreed
byte-for-byte, and a separate blind implementation independently matched all
six lengths and both hashes. Line breaks in `payload hex` are presentation only;
concatenate the lowercase hex lines exactly:

```text
CodeSearch empty:
  payload length = 254
  payload hex =
    000200010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd2000000000007
  SHA-256(payload) =
    96671f23b236f560865fe808ad2e12e9c99c19d7f0e0980a9939e8cc93f81112
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    f0c11bd41c207547a9eb7bc8f5230edc04e6ae0bef8340039b66979c4db90683

CodeSearch one:
  payload length = 264
  payload hex =
    000200010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd20000000100000006
    4e6565646c650007
  SHA-256(payload) =
    5be4fa92d5f5347458695e7127fe40eed5e5f286a8f2fcefa86642922fe12964
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    b14163b7ec4244043e4c98919c1ab2f3393fa39d8696c67a3bc96838ca2fda1a

Definition empty:
  payload length = 254
  payload hex =
    000300010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd2000000000007
  SHA-256(payload) =
    785e54a281b9fd6359a14d54320f552ed5b9a8869ecb0b905418520203effe0a
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    2ca861ede84017e0bf6e8e110bb079bf784329a15ef325c24fcd9c962af6a9ae

Definition one:
  payload length = 281
  payload hex =
    000300010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd20000000100060000
    0015636f6d6d6f6e6d6f64756c652e666c6f772e72756e0007
  SHA-256(payload) =
    55b7c16f7fb1c9b2b44ca5dc331de40db66498c5a9c213bbb3874e34d44381e4
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    61d0dc8d91a05346311fcf5b8a087b19f6b7eed3e0bf5f0118765e24c12a2049

CallGraph empty:
  payload length = 254
  payload hex =
    000400010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd2000000000007
  SHA-256(payload) =
    99b8abf1eb160ef255e78ed8033ed7199364defb6b877530104e36bf14e572f3
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    ea5f488e008df9ac016bf78b6c60d06193164d3d6f3e2fc45a02185772510060

CallGraph one:
  payload length = 281
  payload hex =
    000400010000008e000000000000001c756e6963612e736f757263652d736574
    2d6964656e746974792e76310000000000000008616e616c7973697301010000
    0000000000012e00000000000000477368613235363a61616161616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    61616161616161616161616161616161616161616161bbbbbbbbbbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b18203fa02829
    d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add787c08ad
    7678218574e5a9a55395c7959440208f9a635ed5ab222cd20000000100060000
    0015636f6d6d6f6e6d6f64756c652e666c6f772e72756e0007
  SHA-256(payload) =
    7eadca7ec37c752cbe05fe9fdf6bc924c05f17227433239932d0ee327dc2908e
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    97f2faf6e7d9901b2b70d3d972492dfd19cfcc522c419bd68107034846008372
```

The mandatory redundant-source-frame negative fixture is:

```text
Definition empty with forbidden extra bytes(AtomicSourceIdentityV2):
  payload length = 258
  payload hex =
    00030000009400010000008e000000000000001c756e6963612e736f75726365
    2d7365742d6964656e746974792e76310000000000000008616e616c79736973
    010100000000000000012e00000000000000477368613235363a616161616161
    6161616161616161616161616161616161616161616161616161616161616161
    6161616161616161616161616161616161616161616161616161bbbbbbbbbbbb
    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb279d317b1820
    3fa02829d9dbfa19359913e310bddf3beee5bfd82fc5240046b90001cc7b8add
    787c08ad7678218574e5a9a55395c7959440208f9a635ed5ab222cd200000000
    0007
  SHA-256(payload) =
    42c3d46a2b338e08628e9e6404829d43db176a8f25897a2c92b5edb91af1b1b9
  H("unica.snapshot-bsl-provider-query/v3", payload) =
    1ea160f0b9bfacbb134047a6d1be1d23dc45961ca0a8311505420e6d25520c18
```

`H` is the shared double-length-framed primitive, not raw domain concatenation.
The generator's sole external registry input is the content-sealed
machine-readable projection
`.superpowers/sdd/task-6-v3-registry-manifest.json`, exact SHA-256
`13e5368de7f84af9ef649c5a82b7126345197dd0e29a79e252f163a604aa66b6`.
Before defining any fixture, it loads those exact bytes beside the script,
rejects duplicate JSON keys/non-finite constants/unknown or missing schema
fields/wrong scalar types/out-of-range or duplicate tags, and derives every
port/source-kind/source-format/artifact-kind tag plus the registered-Form
contract version from that manifest. `registries=PASS` means this hash-pinned
external input was loaded and validated; comparing local constants with a
second set of literals or falling back when the file is missing is forbidden.
The package evidence separately pins the executable generator bytes and exact
four-owner tuple, so changing either generator or manifest invalidates every
derived run. Every check is an explicit executable branch, never a Python
`assert` statement whose execution disappears under optimization. The generator
also rejects `python -O` before printing any output; the normal and optimized-
mode-negative commands are both recorded in external evidence.

The **design-only detached fixture** in this standalone generator then performs
single-field mutations for port, source logical identity, source fingerprint,
configuration-catalog digest, registered-Form contract version,
registered-Form digest, vector member and `max_records`, and requires a distinct
payload/domain hash for each. This is sensitivity proof for the published byte
grammar, not a production construction API. Reverse input order for a two-member
term fixture must canonicalize to byte equality. The two-Method fixture must use
unequal encoded canonical-ref lengths whose lexical tuple order conflicts with
the length-framed byte order (`CommonModule.AA.Run` versus
`CommonModule.Z.Run`): both input orders must produce exact
`ArtifactIdentityBytesV1` order, `commonmodule.z.run` before
`commonmodule.aa.run`, and byte-identical queries. Sorting by `(kind,
canonical_ref)`, display spelling or unframed UTF-8 therefore fails this
fixture. A duplicate direct final member is rejected before encoding rather
than silently dropped. The old v2 220-byte fixture must compare unequal to
every v3 row.

The external generator/review evidence records the exact four-document tuple;
the owner contract records only the normative grammar, fixture and values. A
later candidate edit does not make an unchanged file's SHA-256 mathematically
false, but it invalidates that hash's use as standalone package evidence and
invalidates all derived golden/audit/review/ledger evidence. The generator must
be rerun after every affected edit before an external ledger may accept a new
tuple.

After design acceptance and prerequisite implementations, permanent TDD REDs
construct these same six queries through the real one-build
`PlatformCatalogContextV1` smart constructors, independently reconstruct their
payload bytes, and require exact equality with the frozen design values. Thus
the design co-freeze needs no production code, while production cannot satisfy
the gate by copying prose hashes or accepting a detached catalog.

Production REDs must not reproduce those detached single-field catalog
mutations. `PlatformCatalogContextV1` deliberately makes configuration digest,
registered-Form digest and contract version private/coupled authority. The real
suite therefore changes vector member and `max_records` independently through
the public smart constructors; obtains source/logical/fingerprint and catalog
changes only through valid captured/context recaptures; requires each resulting
coupled authority change to alter the query bytes/digest; and proves an equal
independently reconstructed valid context yields byte equality. Exact imported
contract-version equality plus compile-fail/privacy tests reject caller-supplied
versions, digests, detached half-contexts and field mutation. The standalone
generator remains the sole eight-way single-field encoder-sensitivity proof;
production does not forge an invalid context merely to imitate it.

Adding a second proposal that requests the same Method must preserve the exact
query bytes/digest, provider invocation count, raw outcome bytes, tag-8 group
key, retained prefix and provider gap bytes. Only Task 7's external
`MaterialAssociationMapV2` may gain an association. A Task 6 test may use a fake
consumer to prove this invariance, but production Task 6 imports no Task 7 type.

`material_subjects` emitted by Task 6 are provider material only: exact
source-qualified queried Method/caller and, where v7 permits it, the exact
resolved provider Artifact target. They never contain conclusion scopes.

### 7.2 One opaque Analysis BSL material scan plan

The base-v2 manifest enumeration and executable Form suffix
`<RegisteredDir>/<N>/Forms/<F>/Ext/Form/Module.bsl` are superseded in full.
Task 6 never scans a manifest, tests `.bsl`, formats or reverse-maps a suffix,
or partitions Ordinary and FormModule material itself. Every provider starts
with only the whole context, exact Analysis snapshot and injected reader:

```text
context: &PlatformCatalogContextV1
snapshot: &SourceSetSnapshotV2
source_reader: &dyn SourceSnapshotPort
```

It obtains exactly one zero-I/O
`context.analysis_bsl_material_scan_plan(snapshot)?`. The plan is the complete
Task 4-captured Analysis BSL surface in one canonical order. A claimed Present
FormModule replaces the ordinary candidate at that same slot exactly once;
unsupported captured ordinary BSL remains one visible item with `module() ==
None`; and a Form-shaped decoy outside the accepted Task 4 capture produces no
item and no gap. The non-reading `items()` views expose only kind, optional
typed module and opaque diagnostic location. They expose no path, suffix, key,
length/fingerprint tuple, Task 4 handle, reader argument or private index.

Scope and admission use only this plan-owned capability:

```text
let plan = context.analysis_bsl_material_scan_plan(snapshot)?;
let available = canonical_module_membership(plan.items()); // one zero-I/O view pass
let (selection, authoritative_zero_file_modules) = match port {
  CodeSearch | CallGraph => (plan.select_all(), []),
  Definition => {
    let supported_methods =
      exact_supported_module_ownership_and_identity_validation(query.methods)?;
    let supported_modules =
      exact_unique_module_projection(supported_methods)?;
    let (in_plan, zero_file) =
      canonical_partition(supported_modules, &available);
    (plan.select_modules(&in_plan)?, zero_file)
  },
};
let mut cursor = plan.admit(selection)?;
let terminal_gap = cursor.terminal_limit().map(exact_terminal_gap_from);
```

The helper spellings above describe the mandatory algorithm, not new public
APIs. The supported-module validator is the exact base shared-catalog
ArtifactRef ownership/identity validator and consumes the constructor-owned
canonical exact Method vector without re-sorting it semantically. Projection is
closed and deterministic:

1. project every supported Method to its module `ArtifactRef` within the
   already-bound Analysis source, retaining both exact validated bytes
   `(u16be(kind.stable_tag()), canonical_ref UTF-8 spelling)` and its sole
   `ArtifactIdentityBytesV1` semantic identity;
2. if two projected modules have equal semantic identity but unequal exact
   validated bytes, reject the ambiguous alias before selection, I/O or
   absence proof with the existing typed `unsupported_bsl_module_identity`
   bounded scope; never choose one spelling and never collapse the pair;
3. collapse projections only when their complete exact validated bytes
   are identical, so two different Methods in one module select that module
   once; and
4. sort the unique modules by the exact `ArtifactIdentityBytesV1` byte
   comparator. Because aliases were rejected in step 2, this ordering has no
   exact-spelling tie and is permutation invariant.

A malformed query is rejected by its constructor; an unsupported/unregistered
owner or module identity produces the same existing typed scoped gap and is
never Absent. `available` contains every `items()` module, including registered
Missing/NotApplicable obligations, and no `module() == None` unsupported
ordinary item. It is membership lookup only: it neither reorders/deduplicates
plan items nor supplies admission indices. The canonical partition preserves
the exact-sorted unique projected-module order and handles all targets at once,
so a mixed present+absent multi-Method query still performs exactly one
`select_modules(&in_plan)` and one `admit`. Two Methods in one byte-identical
module remain two Definition query targets but produce one module selection;
method multiplicity is not lost or inferred from that selection vector.

A valid supported target absent from `available` is authoritative zero-file
scope because the same context-bound plan covers the complete captured
manifest. It may contribute to `DefinitionAbsent` only if the entire selected
scope and query-wide authority are complete; any selected terminal/gap that
leaves that authority incomplete conservatively blocks the zero-file negative.
Selection happens before resource limits, so unrelated material cannot
suppress an exact Definition target. The closed selection errors remain exact:

```text
WrongPlan=1 | NonCanonicalModules=2 | DuplicateModule=3 |
ModuleNotInPlan=4 | InvalidModuleKind=5
```

`WrongPlan`, `NonCanonicalModules`, `DuplicateModule`, `InvalidModuleKind`, and
the impossible `ModuleNotInPlan` after the zero-I/O intersection map before I/O
to exact `registered_material_handle_mismatch` ContractViolation with zero
prefix. `ModuleNotInPlan` is not used to turn malformed/unsupported identity
into Absent. CodeSearch uses the complete surface.

CallGraph is the deliberate conservative exception because matching static
target CommonModules are known only after callers are parsed. It calls
`select_all()` once, admits one merged canonical cursor, and stores the admitted
owned items. It reads/parses queried callers first, then reads only referenced
targets among those stored items. An unrelated admitted `Process` item consumes
the same global budget but need not be read or parsed. There is no second plan,
counter, cursor, order, admission pass or reread. A terminal limit before a
queried caller or referenced target emits the exact deterministic caller-scoped
`Bounded` gap and never a false `Complete` or edge. Thus an earlier unrelated
material may conservatively suppress a later CallGraph caller/target; Definition
with the same exact target remains unaffected because it scopes before limits.
A two-phase cursor is a future P2 optimization, not v7 behavior.

The same zero-I/O full available-module set distinguishes true absence from
admission truncation without another scan/pass. A valid queried caller module
absent from the complete set yields caller-scoped `missing_caller_definition`/
`Bounded`. After a caller is parsed, a valid referenced CommonModule absent
from the complete set yields Named `Unresolved`, caller-scoped
`unresolved_bsl_call`/`Bounded`, and no edge. If either module exists in the
complete set but its item is absent from the stored admitted prefix because of
the terminal, the terminal gap wins; it is never reclassified as semantic
absence/unresolved proof.

The consuming, non-Clone, `ExactSizeIterator` cursor is the sole owner of the
merged limits. Its immutable precomputed `terminal_limit()` is identical before,
during and after iteration. It freezes these rules:

1. only selected Present materials consume `MAX_BSL_FILES = 20_000`; a claimed
   Present FormModule counts once, while Missing/NotApplicable count zero;
2. the 20,001st Present creates terminal `FileCount` before that item and omits
   the remaining selected suffix;
3. a Present above `MAX_BSL_FILE_BYTES = 16 MiB` consumes one file, is yielded
   once as `FileBytesLimit`, consumes zero total admitted bytes, performs no
   read, and does not prevent later selected items;
4. every other Present participates in one checked merged byte sum; exceeding
   `MAX_BSL_TOTAL_BYTES = 512 MiB` or integer overflow creates terminal
   `TotalBytes` before that item and omits the remaining suffix; and
5. Task 6 owns no second counter or per-kind limit pass.

Terminal `FileCount`/`TotalBytes` map only to `bsl_file_limit`/
`bsl_total_bytes_limit` at the plan's opaque first-omitted location. Per-item
`FileBytesLimit` maps only to `bsl_file_bytes_limit` at that item's opaque
location and is never passed to a reader. Admission classification always
precedes Task 6's module dispatcher. In particular, a captured Present Ordinary
item with `module() == None` and exact captured length above
`MAX_BSL_FILE_BYTES` is yielded as `FileBytesLimit`, emits only
`bsl_file_bytes_limit`, and never reaches the unsupported-module branch. Only a
within-limit `Process` item is inspected for `module() == None` and may emit
`unsupported_bsl_module_identity`. `FileBytesLimit` is per-item and nonterminal:
the next selected item remains eligible under the same cursor unless the
independently precomputed FileCount/TotalBytes terminal omits it.

For each admitted `Process` item with a typed module, Task 6 consumes the item
exactly once through the sole read operation:

```text
context.read_analysis_bsl_material_verified(source_reader, snapshot, item)?
```

Task 5B privately dispatches Ordinary to Task 4's captured-BSL reader and
RegisteredFormModule to its relationship-bound reader. Task 6 imports neither
reader, handle, Form/material view nor resolver. `Present` exposes only typed
module, verified bytes, `location_for_range(u32, u32)` and opaque
`cache_locator()`; Missing/NotApplicable expose only typed module plus opaque
diagnostic location. No variant exposes a path, key, suffix, raw state or
reader/root capability.

The provider semantics are singular across CodeSearch, Definition and
CallGraph:

| Plan item / verification | CodeSearch | Definition exact Method | CallGraph exact caller |
| --- | --- | --- | --- |
| Ordinary or Registered Managed Present | parse the one verified buffer once; a claimed FormModule is never also read as Ordinary | same | parse queried callers first, then only referenced targets among the same stored admitted items |
| Registered Managed Missing | no byte read/parse and no occurrence | `DefinitionAbsent` only if every other target authority/scope is complete | caller-scoped `missing_caller_definition`/`Bounded`; never empty-complete |
| Registered NotApplicable | typed `unsupported_bsl_module_identity`; no read/parse | same, never Absent | same, never an empty-complete caller |
| valid supported module absent from the complete available-module set | not applicable: CodeSearch selects the captured surface | eligible for `DefinitionAbsent` only when selected/query-wide authority is complete | queried caller: `missing_caller_definition`; referenced CommonModule: Named Unresolved + `unresolved_bsl_call`; both caller-scoped Bounded, no edge |
| within-limit `Process` Ordinary with `module() == None` | typed location-backed `unsupported_bsl_module_identity`; no read/parse | cannot enter a valid module selection | caller/query-scoped gap, never empty-complete |
| item `FileBytesLimit`, including `module() == None` | exact scoped `bsl_file_bytes_limit`; no read/parse/dispatcher inspection | same | same; later selected items remain eligible |
| terminal before required material | exact location-backed terminal gap | exact target-scoped `Bounded` | exact caller-scoped `Bounded`, never false Complete/edge |
| semantic context/plan/selection/item/handle/projection/state/key/ordinary-entry mismatch | exact ContractViolation before I/O; no batch/records/gaps/prefix | same | same |
| semantically valid item followed by external filesystem appearance/disappearance/content/identity/ancestor-topology drift | retryable `source_fingerprint_mismatch`, zero prefix | same | same |

Before any provider-local semantic sort, exact/semantic deduplication, atomic
group insertion or `max_records` ceiling, each invocation creates one imported
`ExactArtifactSpellingRegistryV1` and uses the context-owned Analysis
`AtomicSourceIdentityV2` for every Task 6 artifact occurrence. The complete
pre-classification walk covers query targets and every raw parsed record, raw
gap scope, provisionally classified semantic/atomic group and recursively
nested artifact (subjects, callers, callees, definition targets and grouped
material). Classification may compute a provisional group kind solely to find
all nested artifacts, but no row/group may enter a canonical collection before
the registry accepts the entire raw candidate. The registry value is exact
`(u16be(kind.stable_tag()), canonical-ref UTF-8 bytes)`; it never rewrites an
`ArtifactRef`.

Two occurrences in the same complete source with equal
`ArtifactIdentityBytesV1` and unequal exact value bytes invalidate the provider
outcome atomically through the imported registry collision mapping to
`ProviderOutcome::ContractViolation("exact_artifact_spelling_collision")`;
this exact nonretryable stable reason is shared with Task5B and Task7 and is not
a display/debug string. No prefix, records, groups or gaps are
returned. Reversing either occurrence must produce the same rejection. Equal
semantic identities in different complete `AtomicSourceIdentityV2` values are
independent. An isolated case variant remains valid and retains its original
spelling; because the registry is validation-only and adds no field, isolated
`I` and isolated `i` fixtures preserve the already frozen semantic query bytes,
digest and v3 goldens. Task 7 later repeats this rule application-wide across
ports/caches; that does not weaken this Task 6 provider-local boundary.

The mandatory per-item recording matrix is:

| Plan item/result | registered verifier calls | ordinary/delegated byte reads | Task 6 parses | Task 6 direct filesystem calls |
| --- | ---: | ---: | ---: | ---: |
| Ordinary Present | 0 | 1 | 1 | 0 |
| Registered Managed Present | 1 | 1 | 1 | 0 |
| Registered Managed Missing | 1 | 0 | 0 | 0 |
| Registered NotApplicable | 0 | 0 | 0 | 0 |
| unsupported Ordinary or `FileBytesLimit` | 0 | 0 | 0 | 0 |

A claimed Present FormModule can never be read both as ordinary and registered.
Repeated terms/Methods/callers reuse the one consumed item result inside the
provider invocation. The exact injected `&dyn SourceSnapshotPort` owns all
verifier/read counters; no context, plan, item or cache locator contains a
reader, callback or root capability.

Every evidence/gap location comes directly from
`VerifiedBslSourceLocationV1`. Parsed token/range locations use only Present's
`location_for_range(u32, u32)` against the exact verified bytes. Cache access
receives only Present's `VerifiedBslCacheLocatorV1`; Missing, NotApplicable,
unsupported and limited items produce no cache request. Neither opaque wrapper
can be converted by Task 6 to a raw path/key/String or authorize I/O.

Any semantic context/plan/selection/item/handle/projection/state/key/
fingerprint/manifest/ordinary-entry mismatch maps before I/O to exactly
`ProviderOutcome::ContractViolation("registered_material_handle_mismatch".to_owned())`
for the invoked port and then only to
`DiscoveryError::ProviderContractViolation { provider, reason:
"registered_material_handle_mismatch" }`. It is never `Complete`, `Bounded`,
`Unavailable`, `Failed`, a `ProviderGap`, or retryable, and carries no batch,
records, gaps or staged prefix. Only post-validation external filesystem drift
maps to `ProviderOutcome::Unavailable { reason:
"source_fingerprint_mismatch", retryable: true, prefix: None }`.

Top-level registered-module rules remain only where they do not conflict with
this unified scan/read contract. A future generalization or two-phase CallGraph
admission requires a separately reviewed versioned addendum.

### 7.3 Parser/cache version separation

The source-slice and interceptor DTO changes are semantic cache-schema changes:

```text
BSL_PARSER_CONTRACT = "unica.snapshot-bsl.v2"
BSL_DISCOVERY_CACHE_SCHEMA_VERSION = 3
BslDiscoveryCacheRequestV3 / BslDiscoveryCacheResponseV3 / BslCacheEntryV3
```

The v3 response never serializes the internal parser graph. In particular,
`BslIdentifierV1`, `BslFileAnalysis` and every internal struct containing a
`BslIdentifierV1` implement neither `Serialize` nor `Deserialize`; adding either
trait is a compile-fail contract violation. The untrusted wire graph is separate
and has no internal identifier/comparison object:

```text
BslCacheEntryV3 {
  source_relative_path,
  byte_length,
  observed_content_digest,
  analysis: BslFileAnalysisCacheWireV3,
}

BslFileAnalysisCacheWireV3 {
  significant_tokens: Vec<BslSignificantTokenCacheWireV3>,
  definitions: Vec<BslSyntaxDefinitionCacheWireV3>,
  maybe_definitions: Vec<BslMaybeDefinitionCacheWireV3>,
  calls: Vec<BslSyntaxCallCacheWireV3>,
  module_shadow_observations: Vec<BslShadowIdentifierObservationCacheWireV3>,
  maybe_module_shadow_observations: Vec<BslShadowIdentifierObservationCacheWireV3>,
  gaps: Vec<BslSyntaxGapCacheWireV3>,
  observed_cfe_interceptors: Vec<BslObservedCfeInterceptorCacheWireV3>,
}

BslSignificantTokenCacheWireV3 {
  class_tag: u16,
  span: BslSpanCacheWireV3,
  inside_conditional: bool,
  // no comparison text or raw token bytes; literal bytes come only from span
}

#[serde(deny_unknown_fields)]
DefinitionShapeCacheWireV3 {
  is_function: bool,
  is_async: bool,
  exported: bool,
  parameters: Vec<DefinitionParameterCacheWireV3>,
  context_tag: u16,
}

#[serde(deny_unknown_fields)]
DefinitionParameterCacheWireV3 {
  name_spelling: String,
  by_value: bool,
  has_default: bool,
}

BslMaybeDefinitionCacheWireV3 {
  name_spelling: String,
  may_be_exported: bool,
  name_span: BslSpanCacheWireV3,
}

BslSyntaxDefinitionCacheWireV3 {
  name_spelling: String,
  definition_span: BslSpanCacheWireV3,
  declaration_span: BslSpanCacheWireV3,
  name_span: BslSpanCacheWireV3,
  parameter_list_span: BslSpanCacheWireV3,
  body_span: BslSpanCacheWireV3,
  terminator_span: BslSpanCacheWireV3,
  declaration_line_ending_tag: u16,
  shape: DefinitionShapeCacheWireV3,
  local_shadow_observations: Vec<BslShadowIdentifierObservationCacheWireV3>,
  maybe_local_shadow_observations: Vec<BslShadowIdentifierObservationCacheWireV3>,
}

#[serde(deny_unknown_fields)]
BslShadowIdentifierObservationCacheWireV3 {
  exact_spelling: String,
  source_span: BslSpanCacheWireV3,
}

BslSyntaxCallCacheWireV3 {
  caller_definition_index: u32,
  receiver_spelling: Option<String>,
  callee_spelling: Option<String>,
  syntax_tag: u16,
  callee_span: BslSpanCacheWireV3,
}

BslSyntaxGapCacheWireV3 {
  reason_tag: u16,
  capability_tag: u16,
  method_name_spelling: Option<String>,
  span: BslSpanCacheWireV3,
}

BslObservedDefinitionAnchorCacheWireV3 {
  name_spelling: String,
  name_span: BslSpanCacheWireV3,
  definition_span: BslSpanCacheWireV3,
  shape: DefinitionShapeCacheWireV3,
  active_definition_index: Option<u32>,
}

BslObservedCfeInterceptorCacheWireV3 {
  kind_tag: u16,
  target_name_spelling: String,
  annotation_span: BslSpanCacheWireV3,
  target_argument_span: BslSpanCacheWireV3,
  attached_definition: BslObservedDefinitionAnchorCacheWireV3,
  presence_tag: u16,
}
```

`BslSpanCacheWireV3` is likewise a distinct `deny_unknown_fields` wire DTO
containing only its bounded primitive fields and exact closed numeric tags;
none of these DTOs aliases or embeds an internal parser type.
Every serialized enum field -- significant-token `class_tag`, call
`syntax_tag`, gap `capability_tag`/`reason_tag`, definition
`declaration_line_ending_tag`, shape `context_tag`, and interceptor
`kind_tag`/`presence_tag` -- accepts exactly its one mapping and range from the
complete section-4.3.2 registry. A seven-class base-v2 or string-only
eight-class token decoder is not a cache-v3 implementation, and none of the
other seven mappings may be left to serde/Rust declaration order.
`DefinitionShapeCacheWireV3` is exactly the schema above, with no flattened,
optional or additional field. Its `parameters` length is
`0..=MAX_SIGNATURE_PARAMETERS` (`MAX_SIGNATURE_PARAMETERS = 256`); every
`name_spelling` and shadow `exact_spelling` are bounded by the one semantic
identifier limit (1..=512 UTF-8 bytes and 1..=128 Unicode scalars) and must be
a complete non-keyword BSL identifier. `context_tag` is exactly the imported
`BslExecutionContext` mapping in section 4.3.2.

Any other tag, oversized vector/string, unknown field or missing field rejects
the cache entry before internal graph construction. Rows that correspond to an
internal `BslIdentifierV1` field carry exact spelling plus its authoritative
source span. Shadow semantic-name sets have no wire field: only the four
module/local definite/maybe observation vectors cross the cache boundary, with
physical multiplicity intact. Legacy call/gap String fields are renamed as
spellings and are untrusted comparison hints only; they can construct no
internal field until the exact deterministic call/gap pass has been replayed
from verified tokens. Shadow rows likewise construct no internal field until
their complete observation pass has been replayed and every resulting source
slice has passed the sole identifier parser. No wire row contains
`comparison_text`, a serialized
`BslIdentifierV1`, a caller-provided canonical name, or an unchecked identity
constructor.

An old v2 cache envelope or `unica.snapshot-bsl.v1` entry is always a miss. The
workspace transport's unrelated global schema changes only if live code proves
that envelope embeds the discovery DTO version; it is not guessed here. V3
cache validation receives the exact already verified Present byte slice from
the section-7.2 dispatcher, not only `byte_len`, and validates every span
relation, UTF-8 boundary, recomputed line/column, name slice/identity,
line-ending bytes, terminator token,
annotation attachment/presence and canonical vector order. The same-binary
trust restriction from base v2 remains; structural validation is not a claim
that an untrusted remote parser is semantically complete.

Ingress order is exact. First validate envelope/order/bounds/digest and all
primitive tags/spans against the already verified bytes. Then run the exact
full contextual v2 lexer once over the **entire** verified Present byte slice,
from byte zero through EOF, including BOM/default/after-`&`/string/date/comment/
preprocessor state. That pass produces the complete canonical local significant-
token stream. Cache ingress requires one-to-one equality between that entire
local stream and wire `significant_tokens`: equal length, order, multiplicity,
closed `class_tag`, complete `span` (byte range plus recomputed line/column),
and `inside_conditional` for every position. Because each equal span identifies
the exact verified source slice, this is byte-exact token authority without a
wire spelling field. An omitted local token, extra wire token, reordering,
class/conditional mutation, or a forged significant-token span wholly or
partly inside a string/date/comment has no equal local row and rejects the
entry.

Only that complete locally re-lexed stream is used for all subsequent semantic
replay and internal token construction. Re-lexing individual caller-supplied
token/call/definition spans is forbidden as a completeness authority: it cannot
prove that an omitted token between supplied spans did not exist. For every
Identifier-class local token, slice its locally produced exact span and call
`parse_complete_bsl_identifier_v1`; only its returned spelling/comparison
identity may populate the internal significant token.

Run the deterministic declaration/header discovery pass across the complete
local token stream and reconstruct the complete ordered local vectors of
positive definitions, maybe-definitions and active/conditional/deleted attached
definition anchors. Require one-to-one equality with the corresponding wire
vectors before internal construction; an omitted, extra, reordered or duplicate
wire definition/anchor rejects the entry. For each matched definition, maybe-
definition or attached-definition row, slice its locally reconstructed name
span, require byte equality with `name_spelling`, and call the sole
`parse_complete_bsl_identifier_v1(name_spelling)`; its returned value supplies
the internal `name_identity` where present and its exact accessor supplies the
internal spelling. For every positive definition and every attached active,
conditional or deleted definition anchor, the complete declaration/header
parser uses only the verified full local token stream. A positive row must agree
with all its locally reconstructed definition/declaration/name/parameter/body/
terminator spans; an attached anchor must agree with its reconstructed
definition and name spans while the local parser retains every header/body
subspan rather than accepting an omitted wire authority. Replay
locally reconstructs `is_function`, `is_async`, `exported`, the complete ordered
parameter vector (`name_spelling`, `by_value`, `has_default`) and the exact
`BslExecutionContext` tag. The locally reconstructed value must equal the wire
`DefinitionShapeCacheWireV3` field-for-field and element-for-element. Wire shape
fields are comparison claims only and never populate `DefinitionShape` or an
attached anchor. Mutating either boolean, the context tag, parameter count/
order/name, `by_value` or `has_default` independently rejects the entry; the
same mutation matrix applies to an attached-definition shape.

Declaration/header replay accepts `DateLiteral=9` only as the complete verified
parameter-default `ConstLiteral`. It neither turns tag 9 into an identifier,
call or gap fact nor accepts it at an interceptor target span. Call, shadow and
gap replay treat the token as the same source decoy as local parsing; query-mode
date input remains rejected before cache lookup.

Next replay the exact deterministic call extraction, module/method shadow pass
and gap-to-method association from those verified tokens and definition spans.
Every receiver, callee and optional gap-method identifier comes from an exact
source token slice, passes `parse_complete_bsl_identifier_v1`, and only then is
compared byte-for-byte (including order, multiplicity, caller index,
syntax/capability/reason tag and span) with the corresponding wire spelling.
For shadows, replay produces the four separate module/local definite/maybe
observation vectors. Each row must match the corresponding wire
`{ exact_spelling, source_span }` field-for-field and element-for-element in
strict validated source-span order; omitted/extra/reordered rows, duplicate or
overlapping spans, wrong module/definition ownership, definite/maybe token
reuse, forged coordinates or a spelling unequal to the exact verified slice
reject the entry. Only after that equality check does ingress derive the
Unicode-lowercase sorted-unique internal shadow-name sets from the locally
constructed identifiers. Wire strings are never assigned directly to the
internal graph. For every
interceptor, require `target_argument_span` to select exactly one complete BSL
string token already present in the full local stream, decode that verified
token with the same string-literal decoder, require equality with
`target_name_spelling`, and call the same sole identifier constructor on that
decoded spelling. No second span lexer participates. Then validate active-
definition indexes, attachment, canonical vectors and every section-4.5
relation before constructing the
non-serde internal `BslFileAnalysis`. Any failed step rejects that entry and
falls back to local parsing. Cache egress performs only the reverse checked
projection using exact spelling accessors/spans. It serializes every shadow
observation in canonical span order and never attempts to reconstruct them
from the lossy semantic sets, so parse -> egress -> ingress -> egress is
byte-identical even for `I`/`i`, expanding `İ`, repeated spellings and distinct
physical occurrences. It never serializes private comparison text. No
`unsafe`, raw-field constructor, serde callback or service-trusted shortcut may
bypass this reconstruction.

The syntax cache key remains exactly `(parser_contract, content_digest)` and
does not include either catalog digest: syntax output contains no path,
ArtifactRef, provider group or Form authority. Sidecar mapping occurs only
after syntax is returned and is covered by the v3 provider query/outcome. A
cache entry containing a manifest key, module ArtifactRef or provider admission
is a contract violation.

The base-v2 instruction for Task 6 itself to strip a raw manifest path is
superseded. For every verified Present scan item, Task 6 passes only its opaque
`VerifiedBslCacheLocatorV1` to the typed infrastructure cache adapter. That
adapter alone serializes/compares the request/response `source_relative_path`
inside the cache boundary; it cannot return a String/path/key or reusable read
argument to the provider. Missing/NotApplicable have no cache locator and make
no cache request. Cache hit/miss therefore cannot reopen the FormModule path
escape hatch or create a second module-mapping algorithm.

## 8. Exact boundary and decoy fixtures

The parser RED suite must define deterministic builders and first assert the
builder's token count; otherwise an N/N+1 test is not evidence of the intended
bound.

```text
token_run(N)   = "X " repeated N; exactly N significant Identifier tokens
token_run(N+1) = "X " repeated N+1
N = MAX_BSL_TOKENS_PER_FILE = 1_000_000
```

At N the lexer completes. At N+1 the whole file has `bsl_token_limit`, no
partial Definition/Call facts, and no negative absence. Provider-level fixtures
also cover 4,096/4,097 definitions, 65,536/65,537 calls, 256/257 nesting and
every file/byte/result/gap bound from the base design, with the builder asserting
the exact precondition count.

For each control keyword in section 4.3, a fixture places
`<keyword>(Missing())` inside a valid method and proves the keyword token itself
is never extracted as a call. Paired positive control uses `KeywordLike(` where
`KeywordLike` is a valid non-keyword Identifier and expects one Unresolved Named
call. The suite separately covers `If(`, `New(`, `Execute(`, Add/RemoveHandler,
operators before parentheses, comments, one/multiline strings, dates, deleted
blocks and conditional branches. A misleading call fixture is accepted only
when every nontrivia byte has a classified token and the balanced extent is
known; otherwise the affected caller is gapped before any negative proof.

Literal fixtures freeze `1.`, `1.0`, `1`, signed numbers, every RU/EN Boolean
case, RU/EN Undefined case, English-only Null, `Нуль` rejection, exact strings,
multiline strings, and empty/nonempty/boundary complete dates as exact tag-9
parameter defaults and call/source-search decoys. The same complete date in
query mode is `unsupported_bsl_search_term`; every unterminated or structurally
invalid date is atomic `malformed_bsl_syntax`, never tag 7 or tag 9.

## 9. Implementation slices

### 9.1 Shared contract conformance

The base-v2 file map is narrowed for this bridge. Only
`crates/unica-coder/src/application/discovery/ports.rs` owns the three private
query binding fields, their whole-context smart-constructor calls, the three
owner-minted association-authority binding fields and the fourth sealed
validator. `crates/unica-coder/src/application/discovery/determinism.rs` must
not encode the binding in a Task6 query/cache/group/outcome writer.
`crates/unica-coder/src/infrastructure/discovery/bsl/` receives the accepted
query but cannot name, read or compare its binding. Task7 later imports only the
three opaque authority types through the application boundary; Task6 imports
no Task7 enum/registry. No other Task6 production file may call
`PlatformCatalogContextV1::execution_binding_v1` or compare a platform binding.

- [ ] Add REDs that forbid a local source-fingerprint parser, atomic-source
  encoder, catalog parser, fact-family mapping, detached half-context or
  conclusion-scope field.
- [ ] Add the full ProviderFact tag table, tag-8 grouping and CallTarget
  Cartesian REDs.
- [ ] Import the accepted Task 5A/Task4-v7/Task5B-v7 types, the one whole
  composite-bound catalog context, exact `AnalysisBslMaterialScanPlanV1`
  selection/admission types, opaque location/cache wrappers and context-owned
  dispatcher, opaque `PlatformCatalogExecutionBindingV1` and shared three-
  variant `ProviderQueryAssociationViolationV1`. Positive
  compile fixtures start with only `&PlatformCatalogContextV1`,
  `&SourceSetSnapshotV2`, `&dyn SourceSnapshotPort` and, for Definition, a
  canonical sorted-unique `&[ArtifactRef]`; they execute `select_all` or
  `select_modules`, one `admit` and only
  `context.read_analysis_bsl_material_verified(...)`. Compile-fail/static REDs
  reject raw construction, clone/serde, config-only/sidecar-only, private item
  indices, raw/path readers, cross-run plan/selection/item combinations and any
  Task 6 reference to a Task 4 handle/reader/projection or Task 5B Form/material
  view/resolver/state/key/path accessor. Static REDs freeze exactly one
  `execution_binding_v1` call in each of the three Task6 smart constructors and
  none in infrastructure/provider/determinism code; no Task6 raw binding
  component, local constructor, serde path or encoder exists.
- [ ] Run the shared v7 identity/golden suite and commit this independently
  reviewable slice.

### 9.2 Closed lexer/parser

- [ ] Add the exact token/literal/keyword/operator fixtures from sections 4 and
  8 plus every standalone-identifier/error-precedence fixture from section 4.4,
  including explicit unsupported-token outcomes. Freeze SPACE/TAB/U+00A0,
  CRLF/CR/LF, `//` termination, BOM coordinates, U+000C/other Unicode whitespace
  rejection, all eight default-state interceptor-name Identifier controls and
  their immediate after-`&` annotation counterparts.
- [ ] Pin `unicode-general-category = "=1.1.0"`, assert the separate
  `UNICODE_GENERAL_CATEGORY_VERSION == (16, 0, 0)` and exact five L* variants,
  import Task 5B's component-wise
  `ARTIFACT_IDENTITY_UNICODE_VERSION == (17, 0, 0)` build gate without using it
  for membership, add the three differential category REDs from section 3.1,
  and create/update exact
  `docs/third-party/project-discovery-dependencies.md` plus
  `tests/fixtures/project_discovery/bsl/PROVENANCE.md` as section 3 specifies.
- [ ] Verify the N fixtures pass and each N+1 fixture fails atomically with the
  exact typed reason.
- [ ] Add exact definition/declaration/name/parameter/body/terminator and
  LF/CRLF/CR extraction REDs, interceptor observations, shadow/maybe-shadow
  unions, lossless module/local shadow observations (`I`/`i`, expanding `İ`,
  repeated physical binders, permutations and every forged span/spelling),
  duplicate definitions and every forged-cache mutation from section 4.5.
  Record RED under parser contract v1/cache v2.
- [ ] Implement only the minimal closed lexer/parser v2 and cache DTO v3 needed
  to make these tests GREEN; no pinned-source copy, fake artifact, second
  identifier classifier or catch-all skip. Compile-fail tests prove
  `BslIdentifierV1`, `BslFileAnalysis` and all identifier-containing internal
  structs are non-serde; wire-forgery tests prove every identifier is rebuilt
  only from verified bytes through `parse_complete_bsl_identifier_v1`. REDs
  independently mutate significant-token `class_tag` across every 7/8/9
  substitution, `span`,
  `inside_conditional`, order and multiplicity, and forge definition/maybe/
  interceptor names, call receiver/callee, module/local shadow observation,
  gap-method association, caller/definition index, span, order and
  multiplicity. Build the one complete section-4.3.2 numeric-registry matrix:
  every valid significant-token/call-syntax/capability/gap-reason/definition-
  line-ending/execution-context/interceptor-kind/observed-presence tag
  roundtrips, while changing only each field to zero and its exact N+1 value
  (`10/6/5/10/4/7/5/4`) rejects the whole entry and selects local parsing;
  within-range semantic substitutions reject under full replay too.
  The cache-completeness RED starts from a valid full token stream containing
  Identifier `Needle` and removes only that wire token; separate REDs insert a
  forged token span inside a string, a date and a comment, add an extra token,
  reorder two tokens and flip `inside_conditional`. Complete empty/nonempty/
  boundary dates roundtrip as tag 9, work as Definition defaults, remain call/
  source-search decoys and reject in query mode; unterminated/invalid dates are
  atomic malformed syntax. Each forged-cache case rejects even when every
  caller/definition span still re-lexes in isolation. Shape REDs
  independently mutate `is_function`, `is_async`, `exported`, each closed
  context tag, parameter count/order/name, `by_value` and `has_default` for both
  a positive definition and an attached conditional/deleted anchor; no wire
  shape field may appear in the reconstructed internal value.
- [ ] Prove parse -> cache egress -> ingress -> egress byte equality for
  comparison-equal exact-different and repeated shadow observations while the
  four derived semantic sets remain Unicode-lowercase sorted unique, and prove
  the same byte equality for exact tag-9 date spans.
- [ ] Add Definition multiplicity and misleading-call fixtures, then commit the
  independently reviewable parser/cache-schema slice.

### 9.3 Snapshot providers and cache

- [ ] Add provider-local `ExactArtifactSpellingRegistryV1` REDs over raw
  records/gaps/groups and every nested artifact before classification/order/
  dedup/group/ceiling: case-equal exact-different occurrences reject atomically
  in both orders and across record/gap/nested positions, while either isolated
  variant preserves the existing v3 query bytes/digest/goldens.
- [ ] Add smart-constructor ingress REDs for Definition/CallGraph exact
  duplicate collapse and same-source semantic-equal/exact-different rejection
  in both orders after the O(1) raw `0..=2,000` gate and before semantic
  sort/dedup/encoding/I/O, including raw 2,000/2,001 precedence, expanding
  Unicode and isolated-variant digest invariance.
- [ ] Add sealed owner-recheck REDs proving Definition/CallGraph visit every
  private canonical Method/caller through `registry.require_occurrence`, reject
  an absent/substituted spelling, expose no iterator/member/delta, and preserve
  every query byte/digest; CodeSearch has no artifact recheck.
- [ ] Add the execution-binding RED matrix for all three smart queries and all
  three owner authorities. Each constructor obtains exactly one binding from
  its exact whole context and stores it privately; the minted authority owns an
  equal binding and its fourth operation accepts the registry binding from an
  equal independently rebuilt context. Substituting only composite,
  configuration-catalog-set digest or registered-Form-catalog-set digest
  rejects with `PlatformCatalogExecutionMismatch` before invocation allocation
  and before provider I/O while equal v3 members retain equal query Eq/Hash/
  cache identity. Compile/static REDs reject a caller-supplied binding,
  second context projection, raw component/getter/serde/local encoder, a fourth
  Task6 projection caller and any fourth-operation caller outside Task7's exact
  closed typed dispatch plus owner mechanical tests. Reconstruct all six v3
  query payloads and require byte/digest equality with the frozen goldens.
- [ ] Add REDs for verified snapshot reads, one borrowed whole catalog context,
  the complete unified Analysis BSL scan partition/order, exact selection
  errors, immutable terminal limit, merged N/N+1 file/byte admission and the
  section-7.2 per-item counter matrix. Fixtures include a claimed Present
  FormModule that replaces its ordinary slot exactly once, a captured
  unsupported ordinary item that emits one gap, and an uncaptured Form-shaped
  decoy that emits none. Static/recording spies prove the only Task 6 material
  read is `context.read_analysis_bsl_material_verified(...)` and a claimed
  FormModule never receives both ordinary and registered reads.
- [ ] Add port-specific scope REDs: CodeSearch and CallGraph call `select_all`;
  Definition validates supported module ownership/identity, partitions all
  targets from one zero-I/O `items()` available set, then passes only canonical
  `in_plan` once to `select_modules` before limits. Projection REDs cover two
  different Methods in the same byte-identical module (one selected module,
  both Method targets retained), semantic-equal byte-different module aliases
  in both orders (typed rejection before selection), and every Method/module
  input permutation (byte-identical exact-sorted unique selection). REDs cover
  valid absent CommonModule -> eligible Absent, malformed constructor rejection,
  unsupported/unregistered identity -> scoped gap never Absent, registered
  Missing/NotApplicable remaining in-plan, mixed present+absent multi-Method
  input and input permutations. Any post-intersection `ModuleNotInPlan` and all
  other selection errors map to zero-prefix handle-mismatch ContractViolation.
  CallGraph stores one admitted cursor, parses queried callers first and
  referenced targets second without another plan/counter/cursor/pass/reread.
  An earlier unrelated file may conservatively bound a later caller or target;
  the exact caller is `Bounded`, never falsely `Complete`, while the equivalent
  Definition target remains unaffected. REDs distinguish a true zero-file
  queried caller (`missing_caller_definition`) and true zero-file referenced
  target (Named Unresolved + `unresolved_bsl_call`) from a module present in the
  full plan but omitted by the terminal; the latter is only the terminal gap.
  All construction-order permutations preserve those outcomes.
- [ ] Add the admission/dispatcher intersection RED: an unsupported Ordinary
  Present item above 16 MiB yields only `bsl_file_bytes_limit`, performs no
  dispatcher/read/parse work and does not also yield
  `unsupported_bsl_module_identity`; a following within-limit selected item is
  still processed. The paired within-limit unsupported row yields only
  `unsupported_bsl_module_identity`. This proves `FileBytesLimit` is
  nonterminal and has precedence over module dispatch.
- [ ] Through the implemented one-build Task 5B context and real smart query
  constructors, independently reconstruct every frozen section-7.1 payload,
  length, SHA-256 and domain-separated digest. A copied literal without exact
  production-byte equality remains RED; production never regenerates a new
  expected contract from its own output. Each constructor accepts only
  `&PlatformCatalogContextV1`, calls `analysis_platform_catalog()`, appends the
  owned `analysis.source_identity()` directly, and borrows the remaining four
  header values from the same view.
- [ ] Keep the detached eight-way single-field mutation fixture in the
  standalone design generator only. Production tests vary query-owned vectors/
  ceilings directly, use valid recaptured contexts for coupled source/catalog
  changes, and compile-fail on caller-supplied digest/version/half-context
  forgery.
- [ ] Implement CodeSearch/Definition/CallGraph over the same pure parse result;
  obtain every ordinary/registered BSL material only from the one scan plan and
  context dispatcher, attach only opaque plan/verified locations, and apply v7
  classification before every provider-local ceiling.
- [ ] Add cache-v3 hit/miss/stale/down/old-v1/old-v2 equivalence tests; cache
  stores only `BslFileAnalysisCacheWireV3`, reconstructs the internal non-serde
  graph from exact verified bytes, and never stores catalog keys, provider
  admission/group/query association or private identifier comparison text.
- [ ] Run all provider, determinism, product-contract and static-import tests,
  then commit the independently reviewable provider/cache slice.

## 10. Acceptance matrix additions

In addition to every non-conflicting base-v2 case, acceptance requires:

1. the pinned commit/tag and all seven file hashes plus the exact Unicode-16 L*
   table archive/version/license and the separately imported Task 5B
   `ARTIFACT_IDENTITY_UNICODE_VERSION == (17, 0, 0)` standard-library gate are
   mechanically reproduced without conflation; the two exact
   provenance/license inventory targets exist and pass review, and moving
   `develop` links contributed no implementation/fixture bytes;
2. production has no `bsl-analyzer`, Task 5C or Task 7 runtime dependency;
3. all accepted literal, keyword, operator, delimiter and trivia rows are exact;
   interceptor names are annotations only immediately after `&` and the same
   eight spellings remain default-state Identifiers; the sole full token stream
   uses the exact nine-class registry with `StringLiteral=8` and
   `DateLiteral=9`, complete dates are Definition-default `ConstLiteral` values
   only, source Call/CodeSearch treats them as decoys, query mode rejects them,
   and invalid/unterminated dates are malformed;
4. every other nontrivia token becomes a typed gap/malformed result before any
   absence or runtime edge;
5. token and every parser/provider bound pass exact N/N+1 builders;
6. ProviderFact tag 8 `DefinitionObservationCluster` retains duplicate identical
   and conflicting observations whole; this is independent of token-class tag 8;
7. full CallTarget Cartesian validity and all v7 goldens pass;
8. all three v3 query payload/golden families begin from only the whole context,
   directly encode its owned Analysis source identity, bind the borrowed
   fingerprint/configuration digest/registered-Form version+digest and reject
   both the old v2 golden and redundant source-identity frame. Each query also
   obtains exactly one opaque `PlatformCatalogExecutionBindingV1` from that
   context and owns it privately, but the binding contributes zero query/cache/
   group/raw-outcome bytes, query Eq/Hash does not derive over it, and all six
   frozen payloads/digests remain exact;
9. a same-Method second proposal changes no provider byte or invocation;
10. Unicode-lowercase artifact/name goldens, including `İ`, pass through the
    shared encoder under Task 5B's Unicode-17 gate, while BSL lexical membership
    independently remains on the pinned Unicode-16 L* table;
11. provider/file/cache/input permutations produce identical outcomes;
12. standalone identifiers pass exact byte/scalar/keyword/trivia/error-precedence
    tests and Task 8 has only the alias;
13. definition/declaration/name/parameter/body/terminator spans, all three line
    endings, every shadow/maybe-shadow binder and all four observed CFE
    interceptor kinds pass exact slice/forgery/conditional/deleted tests;
    all physical shadow observations survive with source spans and multiplicity
    while `I`/`i` and expanding `İ` derive only the exact sorted-unique semantic
    comparison sets;
    balanced inline declarations yield only exact
    `unsupported_bsl_definition_layout` and no negative proof;
14. parser v1/cache v2 entries cannot satisfy parser v2/cache v3; internal
    identifier/parser graphs are non-serde; the full contextual lexer over the
    complete verified byte slice produces the sole complete local significant-
    token stream and matches the wire stream one-to-one under exact class tags
    1..=9; all eight significant-token/call-syntax/capability/gap-reason/
    definition-line-ending/execution-context/interceptor-kind/observed-presence
    registries are exactly the single section-4.3.2 table; every valid tag
    roundtrips and the complete zero/N+1 matrix (`10/6/5/10/4/7/5/4`) rejects;
    tag-9 dates roundtrip losslessly and every 7/8/9 substitution rejects;
    every wire identifier
    and every definition/attached-definition shape is rebuilt by the sole
    identifier and declaration/header parsers against those exact verified
    bytes, with every shape field compared rather than trusted; cache location
    independence and byte-identical lossless shadow-observation roundtrip are
    proven;
15. all Analysis BSL material follows section 7.2 with no manifest scan,
    suffix/path formatting or direct provider filesystem probe: the merged
    canonical surface contains each claimed Present FormModule exactly once,
    preserves unsupported captured ordinary items and omits uncaptured decoys;
    selection/admission/counters/locations and the full 0/1/1 matrix match the
    context-owned plan/dispatcher contract;
16. active `spec/architecture/extension-point-discovery.md` duplicates this
    exact lexical/trivia, extraction, identity, grouping, scan-plan/location,
    complete eight-part cache-`u16` registry, cache-wire/full-token/shape-
    replay/lossless-shadow-observation,
    provider-local exact-spelling, dual Unicode-pin, provenance and dependency
    contract before implementation
    acceptance;
17. CodeSearch and CallGraph select all while Definition projects its canonical
    exact Method vector to exact-sorted unique modules, collapses only byte-
    identical repeated module projections, rejects semantic-equal byte-different
    aliases, and selects its exact canonical in-plan modules before limits after
    one supported-identity/complete-plan partition; valid zero-file targets are
    eligible for Absent only under complete authority, mixed present+absent/
    permutation REDs pass,
    and unsupported identities never become Absent. CallGraph uses one stored
    admitted cursor, and a terminal before a caller/referenced target is
    caller-scoped `Bounded` with no false Complete/edge, while unrelated files
    cannot suppress the same Definition target; true zero-file caller/target
    map exactly to missing-caller/Named-Unresolved gaps, while terminal omission
    never becomes absence proof. Admission precedes module dispatch: an
    oversized unsupported Ordinary item is only `bsl_file_bytes_limit`, and its
    nonterminal result does not suppress a later eligible selected item.
18. every Task 6 raw record/gap/provisional group/nested artifact passes one
    provider-local source-qualified `ExactArtifactSpellingRegistryV1` before
    semantic order/dedup/group/ceiling; collisions reject in both orders with no
    prefix, while one isolated case variant changes no frozen query/golden byte.
19. Definition and CallGraph smart constructors validate every raw Method/
    caller occurrence admitted by the constant-time raw `0..=2,000` gate with
    the Analysis source through that shared registry before semantic
    sort/dedup/encoding; raw 2,001 rejects before inspection even when all rows
    are duplicates, while admitted byte-identical duplicates may collapse,
    exact-different aliases reject in both orders, and isolated ASCII/
    expanding-Unicode variants preserve all frozen query bytes/digests.
20. Definition and CallGraph's sealed
    `validate_committed_artifact_spellings_v1` owner methods exhaustively
    require every private canonical Method/caller against the execution
    registry without exposing members or changing query bytes; CodeSearch has
    no artifact member.
21. CodeSearch/Definition/CallGraph each mint the exact owner-specific opaque
    non-Clone/non-serde association authority; all three validate the exact
    Analysis source group, Definition/CallGraph validate every canonical query
    Method/caller material, CodeSearch validates no pre-I/O artifact material,
    and each authority owns the equal query binding and exposes exactly the
    fourth sealed
    `validate_execution_binding_v1(&PlatformCatalogExecutionBindingV1)`
    operation. Equal independently rebuilt execution bindings pass; a changed
    composite or either catalog-set digest rejects pre-I/O with
    `PlatformCatalogExecutionMismatch`. Omission/foreign/swap/digest/port/
    binding/constructor/serde REDs pass without changing query, cache, group or
    receipt bytes.
22. the standalone generator loads the exact section-7.1 external registry
    manifest by pinned SHA before defining fixtures, rejects every schema/hash/
    missing-file/optimized-mode mutation without PASS, and has no duplicated-
    literal or fallback path capable of printing `registries=PASS`; its
    unequal-length reverse-order Method fixture proves sorting by exact encoded
    `ArtifactIdentityBytesV1`, not by an unframed `(kind, canonical_ref)` tuple.
23. static call-site tests find exactly three Task6 production calls to
    `PlatformCatalogContextV1::execution_binding_v1`, one in each smart query
    constructor, and none in Task6 infrastructure/determinism/provider code.
    Each owner authority's fourth operation is callable in production only by
    its exact Task7 closed six-authority dispatch; no query exposes the binding
    or calls the context projection while minting an authority. These three
    additions and Task5B's owner whitelist co-freeze as one package tuple.

## 11. Hard STOP conditions

Stop and show the owner if any of the following is true:

- any of `TASK5A_ACCEPTED_SHA`, `TASK4_V7_ACCEPTED_GIT_OID` or
  `TASK5B_V7_ACCEPTED_GIT_OID` is absent when production work is about to start;
- Task 6 is made to wait for Task 5C, or imports Task 7/application conclusion
  association;
- the catalog is reparsed, only one half of the composite-bound context is accepted,
  catalog ScriptVariant/NamePrefix is reread, or a provider chains another
  provider;
- a query constructor accepts anything less authoritative than
  `&PlatformCatalogContextV1`, borrows a computed source-identity temporary,
  reconstructs `AtomicSourceIdentityV2`, or wraps the owned
  `analysis.source_identity()` in a redundant `bytes(...)` frame;
- any Task6 smart query fails to obtain exactly one
  `PlatformCatalogExecutionBindingV1` from that same whole context, accepts a
  caller binding, obtains it a second time, exposes/serializes/encodes a binding
  component, or lets the binding change query/cache/group/raw-outcome bytes or
  any of the six v3 goldens;
- a Definition/CallGraph smart constructor performs per-element work before its
  constant-time raw `0..=2,000` length gate, admits 2,001 because duplicates
  could collapse, sorts/deduplicates/encodes an admitted vector before the
  shared exact-spelling registry, or resolves semantic-equal/exact-different
  input by retaining one spelling, or maps the collision to any reason other
  than exact nonretryable `exact_artifact_spelling_collision`;
- Definition/CallGraph lacks the sealed exhaustive read-only owner recheck,
  exposes a query member/iterator/delta, can add a missing spelling, or
  CodeSearch fabricates an artifact projection;
- any Task 6 smart query lacks its exact owner-minted non-Clone/non-serde
  association authority, accepts a raw/member-list constructor, exposes an
  iterator, admits a foreign source/material, omits a canonical Method/caller,
  lacks the equal owned execution binding or exact fourth sealed
  `validate_execution_binding_v1` operation, accepts a binding from a different
  context/composite/catalog-set authority, exposes a binding/component/getter,
  lets Task7 infer membership from digest equality, or permits a production
  caller outside the exact matching Task7 typed registration to mint it (or
  permits one registration to mint it more than once);
- any Task6 production caller outside the three whole-context smart query
  constructors invokes `execution_binding_v1`, any constructor invokes it more
  than once, any infrastructure/provider/determinism module compares a binding,
  or any production caller outside Task7's exact closed six-authority dispatch
  invokes an authority's fourth operation;
- the standalone generator does not load the exact section-7.1 manifest bytes
  by their pinned SHA before fixture construction, accepts missing/unknown/
  duplicate/mistyped/out-of-range registry data, falls back to local constants,
  can print `registries=PASS` from comparisons against duplicated literals, or
  sorts Method fixtures by anything other than exact encoded
  `ArtifactIdentityBytesV1` bytes (including the invalid unframed
  `(kind, canonical_ref)` shortcut exposed by the unequal-length fixture);
- Task 6 directly looks up `RegisteredFormCatalogV1`, names a Task 5B
  Form/material view/resolver or Task 4 capture handle/reader/projection/state/
  key authority, or accepts a detached catalog/plan/selection/item/material DTO;
- a BSL manifest is scanned, `.bsl`/FormModule suffix or path is tested/formatted,
  sidecar digest/version is absent from a v3 query, or Missing/NotApplicable is
  treated as the same authority;
- any provider bypasses the one
  `context.analysis_bsl_material_scan_plan(snapshot)`, partitions Ordinary and
  RegisteredFormModule itself, or reads material other than through
  `context.read_analysis_bsl_material_verified(source_reader, snapshot, item)`;
- a claimed Present FormModule remains as both Ordinary and Registered items,
  an unsupported captured ordinary item disappears, or an uncaptured
  Form-shaped decoy creates an item/gap;
- CodeSearch or CallGraph selects less than all, Definition applies limits before
  its exact one-shot canonical `select_modules(&in_plan)`, skips the supported-
  module ownership/identity validation and zero-I/O target partition, collapses
  semantic-equal byte-different projected module aliases, fails to collapse a
  byte-identical module repeated by distinct Methods, passes a non-exact-sorted
  module vector, or any
  provider owns a second file/byte counter, per-kind order, private index
  selection or recomputed terminal;
- a valid zero-file supported Definition target is not eligible for Absent under
  complete authority, becomes Absent while selected/query-wide authority is
  incomplete, a Missing/NotApplicable registered Form is misclassified as
  zero-file, or malformed/unsupported identity becomes Absent;
- `ModuleNotInPlan` after Definition's available-set intersection or any other
  selection capability error is not exact zero-prefix
  `registered_material_handle_mismatch` ContractViolation;
- CallGraph creates a second plan/counter/cursor/admission pass or rereads an
  item, reads/parses unrelated admitted items merely because they consumed the
  budget, or reports Complete/an edge when the terminal precedes its caller or
  referenced target;
- a true zero-file CallGraph caller is not exact `missing_caller_definition`/
  Bounded, a true zero-file referenced CommonModule is not Named Unresolved +
  `unresolved_bsl_call`/Bounded/no-edge, or a terminal-omitted module is treated
  as zero-file absence/unresolved proof;
- a `FileBytesLimit` item reaches the module dispatcher, an oversized
  `module() == None` Ordinary item emits `unsupported_bsl_module_identity`
  instead of only `bsl_file_bytes_limit`, a per-item `FileBytesLimit` suppresses
  an otherwise eligible later item, a within-limit unsupported item reaches the
  byte reader/parser, Missing performs a byte read/parse, a Present FormModule
  also uses the ordinary reader, or
  verifier/read counters do not belong to the injected `&dyn SourceSnapshotPort`;
- an internal context/handle/projection/state/key/fingerprint/manifest/ordinary-
  entry mismatch is made retryable, or anything except post-validation external
  filesystem drift becomes `source_fingerprint_mismatch`;
- `StandaloneFact` uses a private fact-family table instead of exact
  `ProviderFact::stable_tag()`;
- a Definition observation uses standalone grouping, deduplicates distinct
  declarations, mixes polarity, or is split by a limit;
- an invalid CallTarget Cartesian tuple can be constructed or produce an edge;
- pinned source is copied/vendored or added as a runtime dependency without a
  separately reviewed license/architecture decision;
- a moving `bsl-parser/develop` link is treated as authority, fixture bytes lack
  the exact provenance manifest, or the dependency/license inventory target is
  absent/stale;
- any unknown token is hidden/skipped, or a broad parser fallback emits a
  positive/negative fact;
- any of the eight cache-`u16` mappings differs from the sole section-4.3.2
  registry, accepts zero or its exact N+1 (`10/6/5/10/4/7/5/4`), derives an
  integer from Rust declaration layout, lacks a valid-tag roundtrip or
  zero/N+1 mutation RED, renumbers/removes `StringLiteral=8` or `DateLiteral=9`,
  assigns `UnsupportedDefinitionLayout` any value except append-only reason tag
  9, or assigns the same integer to two variants;
- a complete date is classified as `Unsupported=7`, omitted from the sole full
  token stream, allowed to satisfy a call/query/interceptor target, rejected as
  a Definition default, or an unterminated/invalid date becomes a bounded
  unsupported token instead of malformed syntax;
- BSL identifier L* classification uses std alphabetic/XID/toolchain tables, a
  crate version other than exact `unicode-general-category=1.1.0`, or an L*
  table version other than `UNICODE_GENERAL_CATEGORY_VERSION == (16, 0, 0)`;
  or artifact comparison lowering bypasses Task 5B's component-wise
  `ARTIFACT_IDENTITY_UNICODE_VERSION == (17, 0, 0)` build gate, substitutes the
  Unicode-16 category table, or treats the two version pins as one authority;
- `Нуль` is accepted as Null, Float/Decimal grammar changes, or punctuation/
  keyword inventory grows without a versioned addendum;
- Task 8 or Task 6 constructs `ArtifactIdentityBytesV1` from a bare identifier,
  declares a second standalone identifier parser/alias constructor, trims or
  normalizes an identifier, or accepts a keyword;
- an extraction span is found by substring search, accepted from bytes other
  than the exact verified module, omits a header line ending, or an old parser/
  cache schema is accepted as containing v2 spans;
- an inline declaration is assigned a guessed LF/CRLF/CR, emitted as a positive
  definition, or silently skipped without `unsupported_bsl_definition_layout`;
- a conditional interceptor/definition permits negative duplicate proof, a
  deleted observation blocks it, or Around is omitted from the observed set;
- an interceptor spelling becomes a default-state keyword, trivia is allowed
  between `&` and its annotation name, U+00A0 is rejected as horizontal trivia,
  or U+000C/another Unicode whitespace scalar is silently hidden;
- `BslIdentifierV1`, `BslFileAnalysis` or an identifier-containing internal
  parser struct gains serde, a cache wire carries comparison text/internal
  identity, cache ingress validates only caller-supplied spans rather than
  running the full contextual lexer over the entire verified byte slice, wire
  `significant_tokens` is not one-to-one equal to that complete local stream,
  a complete BSL string/date is absent from that stream or lacks exact class tag
  8/9 respectively, any 7/8/9 class substitution survives, tag 9 is accepted at
  an interceptor target span, a second lossless/string/date-only token stream or
  span lexer is used for replay, a
  legacy call/gap wire spelling is assigned without exact verified-token
  semantic replay, shadow cache data is reduced to a spelling set or omits its
  authoritative span/multiplicity, semantic shadow sets are derived before the
  exact observation replay, cache egress tries to reconstruct observations from
  a lossy set, any wire shape field populates an internal definition or
  attached anchor without full local declaration/header reconstruction and
  exact equality, or cache ingress bypasses the sole identifier constructor;
- production forges/detaches a catalog field to imitate the generator's
  single-field mutations;
- `registered_material_handle_mismatch` becomes any provider outcome other than
  exact `ProviderOutcome::ContractViolation`, carries a batch/gap/prefix, or is
  mapped to any operation error other than `ProviderContractViolation`;
- Request/Proposal/Mechanism identity enters a query, group, provider outcome
  or cache entry;
- Task 6 canonicalizes, deduplicates, groups or applies a local ceiling before
  one provider-local `ExactArtifactSpellingRegistryV1` has visited every raw
  record/gap/provisional-group/nested artifact with its complete
  `AtomicSourceIdentityV2`, or resolves a collision by order/lexical choice;
- a local ceiling is applied before complete v7 group classification;
- Task 6 changes the public MCP/package/skill surface or implements Task 7.

## 12. Verification gate

Before claiming implementation acceptance, run the base-v2 verification suite
plus focused v7 tests, license/dependency inspection, full `unica-coder` tests,
format, clippy, product-contract tests and `git diff --check`. Recompute the
pinned hashes from a checkout at the exact commit. The implementation review
must record command outputs and all three exact accepted production OIDs from
section 1; a self-audit statement or a green subset is not a substitute.

The standalone generator gate runs both exact commands: normal
`python3.12 .superpowers/sdd/task-6-v3-golden-generator.py` must exit 0 with the
frozen stdout hash, while `python3.12 -O` on the same path must exit nonzero,
emit no stdout/`PASS`, and report the explicit optimized-mode rejection. A
static scan must find no Python `assert` statement in the generator and must
recompute the exact registry-manifest SHA before accepting `registries=PASS`.
The normal run must also exercise the unequal-length reverse-input Method
fixture and prove its final vector is sorted by the exact length-framed
`ArtifactIdentityBytesV1` byte slice (`z` before `aa`), not by a local tuple.
Review also
checks the two exact section-3 provenance/license targets and proves no fixture
maps to an unrecorded external source.

The execution binding is deliberately outside the generator schema. A fresh
normal run after this addendum change must reproduce the exact prior stdout
hash and all six payload lengths/bytes/digests; the `-O` negative remains
stdout-empty and nonzero. Generator and registry-manifest file hashes must also
remain unchanged. Product/static tests separately prove exactly three Task6
`execution_binding_v1` call sites, complete typed equality for composite plus
both catalog-set digests, equal rebuilt-context acceptance, wrong-binding
pre-I/O rejection and the exact fourth-operation downstream caller whitelist.

Static checks additionally reject the historical Form suffix algorithm or any
Task 6 manifest/`.bsl` scan, direct Task4/Form material reader, second admission
counter/order/cursor, CallGraph second pass/reread, raw location/cache path,
`snapshot-bsl-provider-query/v2` in live Task 6 query construction,
`unica.snapshot-bsl.v1`/cache-v2 acceptance, any Task 8 identifier constructor,
any cache-ingress per-span lexer used as completeness authority, and any Task 6
import of Task 5C/Task 7. Compile/product guards prove both the imported
`ARTIFACT_IDENTITY_UNICODE_VERSION == (17, 0, 0)` standard-library assertion and
the separate `UNICODE_GENERAL_CATEGORY_VERSION == (16, 0, 0)` dependency table.
Focused tests must reconstruct each v3 query and source-slice digest from typed
values and verified bytes rather than asserting copied expected strings. Cache
tests prove full-slice token-stream equality under exact class tags 1..=9,
lossless tag-9 date replay, all eight exact section-4.3.2 registries, local shape
replay and lossless span-ordered shadow-observation roundtrip. The recorded
numeric matrix must show valid-tag coverage plus zero and exact N+1 rejection
for class/syntax/capability/reason/line-ending/context/interceptor-kind/presence
(`10/6/5/10/4/7/5/4`), in addition to every 7/8/9 class substitution.
Recording tests must also prove provider-local exact-spelling rejection in both
orders before any semantic collection/ceiling with exact nonretryable reason
`exact_artifact_spelling_collision` and unchanged isolated-variant
query/golden bytes, plus the exact scan
partition, alias-rejecting/exact-deduplicating Definition module projection,
mixed present/zero-file Definition-before-limits selection and absence
blocking, nonterminal oversized-unsupported precedence, and conservative one-
cursor CallGraph terminal semantics from section 7.2.

## 13. External package protocol and downstream Task 8 obligations

The Task 6 owner contract has no owner-local open P0/P1. It nevertheless never
declares its own candidate/accepted state. Design-package status is determined
only by this external protocol:

1. compute the exact SHA-256 of the immutable Task4-v7, Task5B-v7, Task6-v7 and
   Task7-v7 owner documents and record the four-value tuple outside them;
2. record the exact standalone generator bytes/command/output plus the exact
   section-7.1 registry-manifest path/hash/bytes, prove `registries=PASS` came
   from that loaded input, and prove that the six positive and one extra-frame
   negative goldens reproduce against that same tuple;
3. obtain owner self-audits and separate independent reviews that name the same
   exact tuple and report no P0/P1;
4. make one atomic transition in
   `.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`, recording all
   four owner hashes plus exact generator/manifest/audit/review hashes; and
5. if any owner byte changes, treat the package tuple and all derived generator,
   audit, review and ledger evidence as stale and repeat the protocol. An
   unchanged individual file hash remains mathematically correct but is not
   standalone package-acceptance authority.

Missing external evidence is an external acceptance-gate failure, not a latent
Task 6 semantic P1 and not permission to edit frozen owner bytes with status or
hashes. Design acceptance requires no Task5A/Task4/Task5B implementation OID.
Task 6 production separately requires all three exact section-1 OIDs before its
first RED.

Task 8 remains downstream and may not gate or reopen the Task4/Task5B/Task6/
Task7 design package. Every Task 8 design/review/implementation must satisfy
these exact consumer obligations:

1. pure-parser `BslSyntaxDefinition.name_identity` is `BslIdentifierV1`, never
   `ArtifactIdentityBytesV1`; only provider material mapping constructs a full
   module-qualified ArtifactRef and its artifact identity;
2. `declaration_span` includes accepted trailing header trivia/comment up to,
   but not including, the exact line ending, preserving
   `declaration.end + lineEnding.len == body.start`; and
3. semantic enum `Around` imports exact lexer spellings `&Вместо|&Instead` and
   rejects `&Around` as unsupported.

These obligations are downstream acceptance checks, not reverse dependencies.
