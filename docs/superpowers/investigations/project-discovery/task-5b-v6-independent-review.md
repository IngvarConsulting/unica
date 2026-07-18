# Task 5B v6 / Task 6 v2 / Task 7 v6 independent production review

Review date: 2026-07-18

Verdict: **NEEDS-FIX**. No confirmed P0 was found. The P1 findings below block
an unambiguous production implementation or make the declared acceptance order
cyclic. The three normative files must not be treated as accepted/frozen for
implementation until every P1 is corrected and independently re-reviewed.

## Immutable review basis

The three requested inputs were read in full (5,934 lines) and their hashes were
rechecked before and after the review:

- `.superpowers/sdd/task-5b-v6-contract.md` (2,432 lines):
  `665de15a59749bf935dd03b8e15558347db1f93d10dc3cbc2a248b61015c8712`.
- `.superpowers/sdd/task-6-v2-design.md` (1,408 lines):
  `5f2d859f77878b43e627930b46a99063972f0fe1a00b3bc692213beea76db4cc`.
- `.superpowers/sdd/task-7-v6-design.md` (2,094 lines):
  `b307703e2f825d3218e8acc73d480372114e215593734e78bdf822e0588ddd9e`.

The v6 self-audit is a different file and was not substituted for the normative
contract: `.superpowers/sdd/task-5b-v6-self-audit.md` =
`988a732da0efd47ed992b8ded84ee10b2c11169490c3f7a5fc02b4a7fbe4081a`.

The immutable v5 independent review, v5 self-audit and integrity incident were
also read and hash-checked:

- `task-5b-v5-independent-review.md`:
  `c39c3893c80552e23a7769bb3601a78f2182e54590234376b6898814809bee9d`;
- `task-5b-v5-self-audit.md`:
  `d9d866094e4d5587751dd853688ec85db3c14c6db40564f9f372bedebcc23f30`;
- `task-5b-v5-integrity-incident.md`:
  `2f1bff2442995e2dab5e6de6982d3b50c04abdefb734f3f3ffe6d0987064565c`.

The live Task 5A diff, active spec/product-contract diff, current code, tracked
fixtures, root active-review notes and the current Task 5C v2 dependency ledger
were used only as cross-check evidence. They were not substituted for the three
normative inputs. No normative, frozen or tracked file was edited by this
review.

## Findings

| Priority | Finding | Decisive evidence | Required successor test |
| --- | --- | --- | --- |
| P1 | Form main-attribute namespace contradicts the Form root and real fixtures | Task 5B 1629-1634 vs 1711-1732; tracked logform fixtures | real persistent logform Form + arbitrary-prefix/wrong-URI/nested-decoy matrix |
| P1 | Present CFE half is both valid-with-gap and unconditionally invalid | Task 5B 1233-1240; live `ports.rs:1003-1009` | Complete/Bounded x exact/sibling/no-gap companion matrix |
| P1 | source-free typed payload encoder is absent | Task 5B 133-168 vs 1074-1091; live `determinism.rs:670-715` | equal non-zero CFE semantics in two destinations have equal source-free digest |
| P1 | provider query identity is not closed | Task 5B 466-497; Task 7 683-704 | inner-vector permutation/+1 goldens, upstream==invocation digest, Support freshness golden |
| P1 | conflicting Definitions cannot form one declared atomic group | Task 6 704-715 vs 931-948 | identical/conflicting duplicates at whole-group N/N+1 in both orders |
| P1 | exact BSL lexer lacks exact token productions/inventory | Task 6 424-430, 455-480, 616-653, 789-797 | positive/outside-subset boundary per keyword/operator/punctuation/literal family |
| P1 | Stage 2 truncates roots before runtime origin is knowable | Task 7 1087-1114 vs 1246-1270 | 4,097 mixed context/pending targets with a late compatible handler, both input orders |
| P1 | Task 5B/6/7 acceptance graph contains Task 8 and whole-Task5C cycles | Task 5B 1883-1885, 2260, 2376-2378, 2411-2414; Task 6 80-87; Task 7 47-51 | machine-checked DAG naming `TASK5C_EVIDENCE_ACCEPTED_GIT_OID` |
| P2 | frontier/node/mechanism bounds lack exact admission algorithms | Task 7 1122-1126, 1205-1215 | shared-handler/multi-origin and cumulative-node N/N+1 |
| P2 | unresolved Named call target lacks case-canonical identity | Task 6 478-480, 651-665, 799-817 | `Missing`/`missing` plus Unicode-expanding case metamorphic |

### [P1] The Form main-attribute grammar names a namespace that real Forms do not use

Task 5B first fixes the Form root and structural tree to
`http://v8.1c.ru/8.3/xcf/logform` (`task-5b-v6-contract.md:1629-1634`). It then
defines the persistent main-attribute path as
`{http://v8.1c.ru/8.3/managed-application/forms}Form/Attributes/Attribute/...`
(`task-5b-v6-contract.md:1711-1732`). Those statements cannot both describe the
same direct-child tree.

This is contradicted by current tracked product material, not merely by a
synthetic assumption:

- `tests/fixtures/unica_mcp_script_parity/bsp/forms/BusinessProcesses__Задание__ДействиеВыполнить/Form.xml:2,502-507`
  has a logform root and inherited unprefixed logform
  `Attributes/Attribute/Type/MainAttribute` children;
- `tests/fixtures/unica_mcp_script_parity/form-validate/ValidBindings.xml:2,14-18`
  has the same shape;
- live `form_event_registry.rs:632-665` correctly selects logform direct children
  and only the inner `v8:Type` from the data-core namespace.

An exact implementation of the v6 prose therefore finds no main attribute on a
normal exported Form. Context-sensitive object/record Form events become
Unknown even when the document contains complete persistent context.

Required closure: replace `FORM_MANAGED_NS` in the root/wrapper path with the
already fixed logform namespace. Keep only the innermost `Type` in data-core and
its QName value in current-config. If a real BaseForm serialization uses another
namespace, define that as a separate fixture-backed grammar rather than applying
a nonexistent namespace to both roots. Add a RED that runs the exact tracked
persistent Form shape plus arbitrary prefix, wrong-wrapper-URI and nested decoy
variants through the shared neutral registry.

### [P1] A gapped Present CFE half is simultaneously valid and a contract violation

Task 5B requires a destination Form pair with missing mandatory `Form.xml` to
retain its Present polarity while leaving the companion gapped
(`task-5b-v6-contract.md:1233-1235`). The immediately following completeness
rule unconditionally calls `Present-without-companion` a provider contract
violation (`task-5b-v6-contract.md:1237-1240`).

The live Task 5A boundary already expresses the necessary distinction:
`ports.rs:1003-1009` accepts a present destination key without its companion only
when an exact provider gap covers that key, while Complete or uncovered absence
is a violation. The analysis-half validator has the same rule at
`ports.rs:911-920`.

Concrete trace: a destination Form is registered and therefore Present, its
manifest lacks the exact mandatory Form material, and the provider emits
`registered_form_material_missing` for the pair-scoped subjects. Section 7 says
this is Bounded/Unknown; the next paragraph terminates the operation as a
contract failure. Sibling destination groups cannot have the promised isolation
until one result is chosen.

Required closure: freeze `Present + no companion` as legal **iff** the outcome is
Bounded and an exact matching Artifacts/SourceSetWide/QueryWide gap covers that
half. It is a violation for Complete or uncovered output. `Absent + companion`,
source-wrong companion and conflicting companions remain violations. Add REDs
for analysis and destination halves, exact versus sibling-only gaps, and
Complete versus Bounded coverage.

### [P1] The source-free semantic-cluster encoder still has no closed typed-payload projection

The CFE whole facts require every field, including pair key and source set, in
their semantic payload/digest (`task-5b-v6-contract.md:133-168`). The v2 atomic
record then inserts an opaque `digest32(typed payload digest)` and claims that the
result is source-free (`task-5b-v6-contract.md:1074-1091`). No section defines a
separate source-free typed-payload encoder, its domain, variant field order, or
which source-bound fields are removed.

This is not theoretical. The live implementation has to distinguish
`provider_fact_digest()` from `source_free_provider_fact_digest()` and contains
special projections for both CFE variants (`determinism.rs:670-715`). Root active
review addendum 6 required exactly that separation because the normal typed facts
embed `SourceScopedArtifact.source_set`.

Concrete trace: destination A and destination B contain semantically equal
Adopted membership (same artifact, flavor, role and UUID values) under different
pair/source identities. Hashing the normal whole-fact payload makes their
supposedly source-free semantic digests different. That changes the group-key
suffix and can change which group is retained at an unchanged prefix limit,
contradicting `task-5b-v6-contract.md:986-991,1158-1160`.

Required closure: publish one versioned source-free typed-payload encoder for
every `ProviderFact` variant used by v2 grouping. It must preserve semantic role,
polarity, flavor, membership, UUIDs, artifact identities and binding/definition
shape while excluding outer source, pair, snapshot/fingerprint, provider and
location identity. State whether the fact stable tag is inside that digest and
do not reuse the normal source-bound whole-fact digest. Add a non-zero
cross-destination CFE golden; the current all-zero StandaloneFact golden cannot
detect this defect.

### [P1] Provider query identity is not closed end to end

There are three independent holes in the query identity contract.

First, Task 5B says `pair_digest`, `presence_key_digest` and
`form_material_scope_digest` are domain-separated hashes of canonical typed
vectors (`task-5b-v6-contract.md:466-469,493-497`), but nowhere publishes the
three domains or any vector field/tag/framing grammar. The normative Metadata
golden merely injects opaque `d*64/e*64/f*64` values
(`task-5b-v6-contract.md:532-545`), so it cannot prove that two implementations
derive those values identically.

Second, Task 5B/Task 6 each define the final upstream query digest directly as
`H` over their exact versioned payload (`task-5b-v6-contract.md:474-499,515-520`;
`task-6-v2-design.md:275-291`). Task 7 instead says every scoped digest is a
domain-separated SHA-256 over “port stable tag and exact typed scope” without
naming a domain or saying that the field equals the already computed upstream
digest (`task-7-v6-design.md:683-704`). Reusing the upstream digest, wrapping it,
or rebuilding and prepending another port tag are all defensible readings and
produce different invocation keys and analysis IDs.

Third, Support has no upstream encoder at all. Its Task 7 scope is only a subject
vector (`task-7-v6-design.md:633-655`), even though the adapter is snapshot-bound
and borrows the exact catalog set. No domain, composite/catalog freshness input,
source-identity lookup rule or golden is defined. Stable catalog object lookup
does not close invocation/cache identity.

Required closure:

1. define exact domains and complete vector encoders for pair, presence and Form
   material scope, or inline those typed vectors in the Metadata payload;
2. state that `ScopedProviderInvocation.query_digest` is byte-for-byte the exact
   upstream Metadata/Form/BSL query digest, with no second wrapper;
3. define `support-state-query/v2` over the captured composite identity, borrowed
   catalog-set digest and canonical full `SourceScopedArtifactIdentityBytesV2`
   vector (and any other actual freshness input), then publish a nontrivial
   golden;
4. add permutation/empty/+1-member REDs and a test proving that changing only
   Form runtime subjects, one pair/presence member, catalog set or snapshot
   freshness changes exactly the intended query identity.

### [P1] Conflicting Definition observations cannot be atomic under the declared group registry

Task 6 correctly requires duplicate declarations to survive and requires
different shapes to create `conflicting_definition_shapes`
(`task-6-v2-design.md:704-715`). But section 11 assigns BSL definitions to
`StandaloneFact`, whose typed semantic digest is part of the group identity
(`task-6-v2-design.md:931-948`). Two shapes for the same method therefore form
two different groups. The sentence that conflict/companion observations must
“stay in the same semantic group or make classification a contract violation”
does not define a group that can contain both and would make an expected input
conflict fatal rather than observable.

Concrete boundary: method M has two definitions with different shapes and the
local record ceiling ends between their groups. One shape may survive while the
other shape/conflict observation is dropped. An exact result-limit gap should
keep the conclusion Unknown, so this is not a demonstrated P0 false Supported
result, but the provider has violated the promised all-or-none conflict cluster
and Task 7 cannot reconstruct it.

Required closure: add a closed BSL `DefinitionObservation`/`DefinitionConflict`
group keyed by source plus queried Method and containing every present shape,
location and duplicate/conflict observation, or emit one whole typed conflict
fact before grouping. Define its secondary/source-free bytes and material
subjects. Add N/N+1 REDs for identical duplicates and different shapes in both
input orders, proving all witnesses survive or the whole method group is
dropped with one exact method gap.

### [P1] The “exact” BSL lexical subset omits the token grammar needed to implement it

Task 6 says the handwritten lexer is a closed exact subset and that every
nontrivia source byte must become a known or explicit unsupported token
(`task-6-v2-design.md:424-430,527-554`). However:

- the “closed keywords” table is only a required partial list and omits the
  control/operator set later used to decide whether a parenthesized construct is
  ignored or becomes `unsupported_bsl_call_syntax`
  (`task-6-v2-design.md:455-480,789-797`);
- `Number`, `Boolean`, `UndefinedOrNull` and `Punctuation` token classes have no
  exact lexical productions or punctuation inventory
  (`task-6-v2-design.md:616-653`);
- `ConstLiteral` delegates optional sign and literal acceptance to “where the
  language grammar permits it” (`task-6-v2-design.md:675-690`), but the immutable
  design pins neither a vendored grammar nor a grammar commit/file hash;
- multiline-string continuation is described semantically but not as an exact
  start-of-line/whitespace production.

This leaves two conforming implementations free to classify the same operator,
numeric spelling, Boolean/null spelling, parenthesized control construct or
continuation differently. Their facts, gaps, coverage and analysis IDs then
differ even before Task 7. Conservative failure avoids a demonstrated false
positive, but it does not make the production contract implementable.

Required closure: either vendor and hash the exact grammar authority plus freeze
the deliberately accepted subset, or enumerate all accepted significant tokens,
RU/EN keyword/control/operator pairs, punctuation, numeric/constant productions
and string/date continuation rules in this contract. Add one positive and one
outside-subset RED per token family, including mixed-language/case forms and an
unsupported token immediately before a possible Definition absence.

### [P1] Stage 2 applies the root limit before it can know which roots are runtime roots

Stage 2 combines pending typed handler requirements with proposal/known/search
method anchors, then sorts and applies the application root bound **before**
Definition and compatibility joins (`task-7-v6-design.md:1087-1114`). Section
7.3 promises the opposite semantic order: merge origins, sort runtime-origin
first, and then keep the first 4,096 roots
(`task-7-v6-design.md:1246-1270`). A pending handler is not known to be a runtime
root until the Definition/policy join, so the earlier step cannot implement the
later key.

Concrete trace: the exact bounded inputs produce 4,097 mixed initial Method
targets (for example, the canonical context prefix can include up to 2,000
CodeSearch occurrences, 128 known Methods and 32 proposal Methods, followed by
pending handler Methods from the independently bounded Metadata/Form outcomes).
A compatible pending ScheduledJob/Form/Event handler sorts just after the first
4,096 mixed targets. The pre-Definition prefix removes it and no Definition is
queried for it. It can never become the runtime-first root that section 7.3 says
must displace a context root. The root-limit gap prevents a safe negative proof,
so this is a P1 false-negative/coverage defect rather than a confirmed P0.

Required closure: do not apply `MAX_TRAVERSAL_ROOTS` to the mixed initial
Definition target set. Query the bounded pending-mechanism targets and context
anchors in explicitly defined stable chunks, build compatibility/mechanism
origins, merge equal methods, then apply the one runtime-first root selection.
If a separate pre-Definition safety bound is needed, give it its own constant,
priority (pending mechanisms first), gap identity and analysis-version input.
Add an exact 4,097 mixed-target RED whose final canonical target is a compatible
pending handler, plus its reversed input permutation.

### [P1] The frozen documents recreate two dependency cycles

Task 5B makes a real Task 8 consumer part of its own implementation and
acceptance: section 10.6 requires Task 8 consumption
(`task-5b-v6-contract.md:1883-1885`), RED E15 requires Task 8 binding
(`:2260`), implementation step 11 requires importing its accepted result
(`:2376-2378`), and acceptance requires Task 8 to use both V2 catalogs
(`:2411-2414`). Task 7, however, requires accepted Task 5B before implementation
and Task 8 is downstream of Task 7. This creates
`Task5B -> Task7 -> Task8 -> Task5B`.

A second cycle is introduced by Task 6 requiring “Task 5C v2 SHAs” and Task 7
requiring “GREEN Task 5C” (`task-6-v2-design.md:80-87,1344-1348`;
`task-7-v6-design.md:47-51`). The current Task 5C v2 ledger correctly splits
read-only Evidence from the later Mutation slice:
`5C-Evidence -> Task6/7 -> Task8/9/10 -> 5C-Mutation`. Waiting for whole Task 5C
therefore waits for work that itself waits for Task 6/7.

Required closure: Task 5B acceptance may export and statically test the neutral
V2 catalog seam/future-consumer contract, but an actual Task 8 import/test/commit
must not gate Task 5B. Task 6 and Task 7 must name the exact accepted
`TASK5C_EVIDENCE_ACCEPTED_GIT_OID`, explicitly not final Task 5C/Mutation. Update
all STOP/acceptance/report wording and add one machine-checked acyclic dependency
ledger before any implementation worker starts.

### [P2] Frontier, node and mechanism limits have constants but no exact admission algorithm

Task 7 defines `MAX_TRAVERSAL_FRONTIER_METHODS`, `MAX_TRAVERSAL_NODES` and
`MAX_MECHANISM_INSTANCES` (`task-7-v6-design.md:1205-1215`) and promises N/N+1
acceptance, but the algorithm says only “apply frontier/node bounds”
(`:1122-1126`). It never fixes:

- whether node count means distinct Method identities or `(Method, origin)` work
  items, and whether it is checked before or after target Definition;
- the exact retained prefix and conclusion-scope union when one Method has
  several origins;
- whether mechanism-instance limiting happens before root-origin merge, after
  it, or only as a report projection.

Example: 4,097 mechanism instances share one handler. Root count is one, but
the mechanism limit still must omit one instance without leaving its Mechanism
origin attached to the handler. The current text does not determine the gap,
query sequence or execution snapshot.

Required closure: publish a numbered algorithm for each bound, counted identity,
canonical prefix, application point, omitted scope and rebuild behavior. Add
shared-handler/multi-origin and cumulative-node N/N+1 REDs.

### [P2] `CallTarget::Named` has no canonical case-insensitive encoding rule

BSL identifiers are case-insensitive and syntax tokens carry one Unicode-lowercase
comparison form (`task-6-v2-design.md:478-480,651-665`). `CallTarget::Named`,
however, is only a `String` described as syntactically static spelling and as
participating in the evidence digest (`task-6-v2-design.md:799-817`). The design
does not say whether it stores source spelling or the comparison form.

This cannot create a runtime edge because Named is unresolved, so it is P2, but
`Missing()` and `missing()` can receive different source-free group identities
under one reasonable implementation. Freeze Named as one/two Unicode-lowercase
identifier segments for identity, retain exact spelling only in location/display,
and add a case/Unicode-expansion metamorphic RED.

## Independent mechanical verification

I independently rebuilt the published encoders from the prose. Rust-compatible
Unicode lowercase was reproduced character-by-character (`chars().flat_map`),
not with a whole-string lowercase operation; the distinction matters for Greek
final sigma. All published outer values passed:

| Fixture | Length | Verified SHA-256 / domain hash |
| --- | ---: | --- |
| ResolvedSourceSetIdentityBytesV1 | 142 | `e1d804d1e18f2d02679dce05b4e2a822c9a776cfd749a67754c9328fc48d9396` |
| AtomicSourceIdentityV2 | 148 | `8543b710e36b6393bd362435b76774cf62e59a24bc5b61ee3926a473a2234710` |
| Metadata query payload | 359 | `56700dfff7680dcd522f11ebe5ced807a06a8d14e97883e10f810ea98d94d4f9` |
| Metadata query domain hash | n/a | `a979e44cb1a1f6a3a6b923b91dd61b38e5e73975aaeff001e03d9de7259371c6` |
| Form query payload | 248 | `b451fd55fbf92ac2d3dfce93e497ad7af9a33e7ad4616ab20e4a216197aa0e51` |
| Form query domain hash | n/a | `d9819ec00b4efbc7c2a03dc0681047230b642118d8f608a578b5efac64c2acc5` |
| BSL Definition query payload | 220 | `3d363a007dacb05ffaeabf40ce645e793979b8b5f5391e86224e9fd79582b709` |
| BSL query domain hash | n/a | `78cc2f7fa751f7e5c52c669e668c2031abcbc6919ce137fe6fc4f1d41329a0cc` |
| Standalone secondary digest | 32 | `d676f3489f6c9b6794c72c0cbd47f8a139e8fe96574dd53e99a440f16eae405c` |
| Source-free semantic cluster | 32 | `2398dac1eea977cb341f08a3fc4f5293a7209e8009520490eb7dc94a4877e788` |
| Complete group key | 388 | `a430566280d0cb70bb731d3d349ee852cb13cafec461ddd1772941fc123a126e` |
| Atomic physical record | 273 | `74f3339fcb2f2165a2196b8b0190c994c56286c0a8ffb3e557d7ae1c42780e77` |
| Global group order | 394 | `57b305568677e427f0ff95fa39e319e2c8284955fd5ccfe543ea6428beeeb406` |

The published `Document.Σ`/`Document.σ` equal identity bytes and expanding
`Document.İ -> Document.i + combining dot` bytes also pass. These checks prove
the published outer fixtures; they do not close the missing inner vector/source-
free encoders identified above.

## Cross-document checks that passed

- ScheduledJob now consistently short-circuits `Use -> Predefined`: Disabled is
  metadata-only No, NonPredefined is metadata-only Unknown, and only the exact
  supported predefined/profile row opens Definition.
- EventSubscription remains exactly 13 source-family mappings, 21 supported
  event/family rows and three signature classes. Task 7 consumes the complete
  descriptor rather than reconstructing it.
- FormCommand compatibility is synchronized: own FormModule, Procedure, one
  by-reference nondefaulted parameter, explicit AtClient, sync or async, Export
  nonmaterial. HTTP is synchronized as the exact same-service synchronous
  ModuleDefault one-parameter Function row.
- Exact MDClasses, Event Source data-core element and current-config QName URI
  handling are consistent. Arbitrary XML prefix spelling remains nonsemantic.
- Platform XML and BSL local ceilings classify complete v2 groups first; Task 7
  per-port/global limiters use prefix-stop rather than skip-and-continue. The
  effective-gap 256/257 and 2,000/2,001 replacement sentinel is closed.
- Task 6 closes the prior conditional-definition/shadow leaks, recognizes the
  named parameter/Var/assignment/For/For Each/access/index binders, uses one
  identifier/keyword/Boolean/null lowercase comparison rule, and fails closed
  for async terminator and unsupported-token ambiguity. The remaining BSL issue
  is the absent exact token inventory, not those already corrected policies.
- Task 7 keeps structural/call observation separate from directed runtime
  reachability, scopes repeated invocations, preserves explicit proposals outside
  `maxCandidates`, and makes application admission/traversal gaps distinct from
  provider testimony.
- No normative design authorizes a new public MCP server/tool/package/skill
  surface in Tasks 5B-7.

## Live implementation gate

The current tracked Task 5A diff is intentionally not v6-complete. Among other
visible blockers it still has `SemanticAtomicGroupIdV1`, name-only
`SourceSetWide(String)`/`SourceScopedArtifact`, a Support query bound of 656, and
no landed v2 composite query/shared-catalog implementation. `DefinitionShape`
does retain `is_async`, and the live Form registry/fixtures provide useful
correct namespace evidence. These are implementation STOP facts, not substitutes
for correcting the design findings above.

## Acceptance decision

**FAIL / NEEDS-FIX.** The v6 self-audit PASS is not independently sustained.
There is no demonstrated P0 false Supported/receipt path, but all eight P1
findings must be resolved in new immutable inputs and a fresh independent review
must report no P0/P1 before Task 5B, Task 6 or Task 7 implementation acceptance.

The SHA-256 of this review is intentionally reported out of band after the file
is finalized; embedding a file's own digest in its contents would be
self-referential.
