# Task 5B preimplementation audit

> **SUPERSEDED HISTORICAL AUDIT — DO NOT IMPLEMENT FROM THIS FILE.**
> Its B0-B13 rows were inputs to the later contracts. Where this audit differs
> from `task-5b-contract.md` v5, v5 wins. In particular, source mismatch is non-retryable
> `ProviderContractViolation(platform_xml_source_mismatch)`; missing expected
> registered Form.xml is local Bounded and is detected from catalog-vs-manifest
> state before I/O; capture-authoritative Configuration/root/nested XML/resource/
> identity failure is snapshot-fatal; provider gap overflow uses
> `platform_xml_gap_limit`; one composite Metadata invocation with deterministic
> internal source groups supersedes later one-call-per-source text. This file is
> evidence of review history, not an acceptance checklist.

| Historical topic | Current v3 rule |
| --- | --- |
| query/source mismatch | non-retryable `ProviderContractViolation(platform_xml_source_mismatch)` before I/O |
| missing catalog-derived registered Form.xml | compare catalog key to the immutable manifest; local Bounded `registered_form_material_missing`, zero reader calls |
| invalid/over-limit Configuration or root/nested descriptor | capture-fatal; the provider is not invoked |
| provider gap overflow | one permutation-independent QueryWide `platform_xml_gap_limit` sentinel |
| Metadata invocation boundary | one composite call with deterministic source-local groups and `SourceSetWide` isolation |

Date: 2026-07-17

Audited base: `20f6afa7a09430614babebc0cdeebeb94c8a0189`

Audited state: the shared worktree contains an uncommitted Task 5A candidate.
This audit is intentionally ignored and makes no tracked code/spec change.

Read completely:

- `.superpowers/sdd/task-5b-contract.md`;
- `.superpowers/sdd/task-5b-brief.md`;
- `.superpowers/sdd/task-5a-brief.md` and the current Task 5A model/port diff;
- the accepted Task 4 brief/report and current shared Platform XML/snapshot code;
- ADR-0008, the active architecture spec, and the relevant Task 5C/Task 6
  design boundaries.

## Verdict

**STOP before Task 5B implementation.** The XML shapes and snapshot reader are
sufficient for the supported v1 subset, but the currently documented Task 5B
boundary cannot yet implement truthful CFE destination membership, the
FormInspection-owned command flow, or callback alias semantics. It also gives
contradictory whole-port/local failure rules.

Task 5B may start only after the accepted Task 5A commit (not the current dirty
base) closes rows B1-B7 below and its exact SHA is recorded in the 5B report.
Rows B8-B13 are mandatory implementation constraints for 5B itself.

The live Task 5A candidate has already added `ProviderGapScope::SourceSetWide`
and exact source-set-aware destination projection. Those are the right
direction, but they are not yet an accepted commit and the 5B contract/brief
still omit or contradict them.

## Mandatory change matrix

| ID | Area | Current contradiction or missing contract | Mandatory correction | Acceptance RED before 5B GREEN |
| --- | --- | --- | --- | --- |
| B0 | Accepted base | `task-5b-brief.md` requires an accepted Task 5A SHA, but HEAD is still the Task 4 base with a dirty Task 5A candidate. The active architecture spec still contains the old `ExchangePlan/handles` matrix. | Finish, review, commit, and synchronize Task 5A first. Record that SHA as the only 5B base. The active spec must reject the old edge and contain `SourceSetWide`, scoped gaps, callback gating, staged support, destination membership semantics, and the Task 7 depth defer. | A product-contract test reads the active spec and rejects `ExchangePlan/handles`; clean checkout of the recorded Task 5A SHA passes the full Task 5A suite. |
| B1 | Source-qualified gaps | The 5B brief only names `Artifacts|QueryWide`. A single Metadata/Form port invocation spans analysis plus zero or more mutation source sets, so a destination-A catalog/resource gap cannot safely be represented by unqualified `QueryWide`. Task 6 already requires `SourceSetWide`. | Keep stable tags `Artifacts=1`, `QueryWide=2`, `SourceSetWide=3`. Use `Artifacts` for exact source+artifact subjects; `SourceSetWide` when the source is exact but no safe ArtifactRef exists; reserve `QueryWide` for a truly cross-source/global invocation gap. Validate every named source against the captured composite snapshot. | Analysis + destination A + destination B: a destination-A catalog limit affects only A; analysis and B retain complete conclusions. Forged source names are provider contract violations. |
| B2 | Exact CFE destination membership | Neither 5B document says that MetadataCatalog reads mutation snapshots. Yet `ProposalValidator` now distinguishes `extension_owned` from `extension_required` through exact destination `MetadataPresent/Absent`. `MetadataCatalogPort` still receives only broad `DiscoveryQueryPlan`; a Complete batch with an accidentally omitted destination record is interpreted as `No`, yielding a false `ExtensionRequired`. | Add an application-built, canonical metadata query (name may vary) containing exact destination membership subjects. Subjects are the registered metadata owners from each CFE target ownership chain, source-qualified by the exact destination. The XML provider scans seven flows only in the analysis snapshot; mutation snapshots are read only for requested catalog membership. Validate the response: every Complete destination key has exactly one Present or Absent fact; a Bounded response has an exact artifact/source-set gap for every missing key; destination bindings/callbacks and out-of-plan destination records are contract violations. | Same target present in extension A and absent in extension B yields Owned for A and Required for B. Reversing source/request order is identical. A fake Complete provider omitting one destination key is rejected rather than interpreted as absence. A destination record outside the query is rejected. |
| B3 | Negative catalog proof | The model permits `MetadataAbsent`, but 5B specifies only positive catalog outputs and no provenance rule for absence. Silence plus global Complete is too easy to misuse as destination absence. | Emit explicit `MetadataAbsent` for every requested missing destination owner/form. Define the negative witness: the exact complete direct `ChildObjects` collection (Configuration for root membership; registered owner descriptor for nested Form membership), with its path and 1-based position. If the collection cannot be proven complete, emit a scoped gap, never absence. | Root present/absent and nested Form present/absent fixtures assert exact source-set, exact Present/Absent cardinality, and witness location. A resource-limited or malformed membership catalog never emits Absent. |
| B4 | FormInspection and FormCommand existence | Only FormInspection reads `Ext/Form.xml`, but the current port contract allows it only `Binding`; Structural bindings always expect MetadataCatalog, and ProposalValidator asks MetadataCatalog for FormCommand existence. MetadataCatalog cannot know `Command[@name]` without also parsing Form.xml, which would violate the required failure isolation (`malformed Form.xml` must not fail catalog evidence). | FormInspection owns the facts derived from Form.xml: `MetadataPresent/Absent(FormCommand)`, `Form --contains--> FormCommand`, and `FormCommand --handles--> FormModule.Method`. Permit the form-to-command structural edge only from FormInspection (other structural facts remain MetadataCatalog). ProposalValidator uses FormInspection for exact FormCommand existence and binding materiality, while root/form owner membership remains MetadataCatalog. | Positive Report and DataProcessor form-command proposals become Supported with fake compatible Definition/support facts. Missing command is conclusive No only after a complete FormInspection read. Malformed one Form.xml leaves owner/form MetadataCatalog facts intact and gives no command absence. A FormInspection structural edge of any non-FormCommand shape is rejected. |
| B5 | Unconditional CommonCommand binding | `ValidatedBinding::common_command` is still accepted from MetadataCatalog and EvidenceGraph immediately promotes every non-disabled Binding to a runtime edge. This directly contradicts the 5B rule that a CommonCommand is pending until an exact Definition join. | Remove or make unconstructible the unconditional CommonCommand binding. Reserve its stable tag if compatibility requires it, but provider validation must reject it. A CommonCommand XML provider emits MetadataPresent plus only a validated PlatformCallback requirement. | A CommonCommand Binding fact is a provider contract violation. Metadata + callback alone is not reachable. Callback + compatible exact Definition is reachable. Definition absent/mismatch/unknown variant follows the exact matrix. |
| B6 | Callback requirement integrity | The 5B contract requires `registry_version` in the callback payload, but current `PlatformCallbackShape` has no such field. Current validation checks the object method spelling, but does not prove subject/owner, metadata kind, module kind, and object chain equality. | Add a closed callback registry version (`platform-callbacks/v1`) to the typed requirement and stable digest. Add a smart constructor for the whole callback fact/slot that proves subject == specialized root owner, object == exact module/method under that owner, callback metadata/module/method == the object chain, and the row is one of the four exact registry rows. Provider name/version does not replace registry version. | Every forged cross-owner, wrong module, wrong method, non-registry row, or wrong registry version is rejected before graph promotion. Changing registry version changes the evidence digest. Exactly four rows pass. |
| B7 | Callback join and provenance | The graph stores one `PendingCallback` per evidence record. The required multi-location duplicate callback facts therefore produce multiple pending items; a negative/unknown `RuntimeRejection` is selected with `.find`, losing all but one callback provenance. | Aggregate pending callback facts by exact semantic `(source, subject, object, registry requirement)` before the Definition join and union all XML evidence IDs. Produce one canonical runtime result/rejection per slot. The positive edge and negative/unknown result both retain every callback-field and Definition evidence ID. | The same callback fact at ScriptVariant and descriptor Name plus one mismatching Definition produces one rejection containing all three IDs, independent of record order. The positive form retains all IDs too. |
| B8 | Cross-language callback alias | `cross_script_alias()` currently guesses alias status from ASCII-vs-non-ASCII, and the lookup only checks Definitions keyed by canonical `pending.object`; a real opposite-language method is a different ArtifactRef, so the branch is normally unreachable. The heuristic can also call an arbitrary Cyrillic wrong name an alias. | Pair rows only through a closed callback slot identity in the four-row registry (`DocumentBeforeWrite`, `CommonCommandProcessing` or equivalent), never character classes. The exact query/join must be able to inspect the canonical method and the single opposite-language official row under the same owner. If the opposite row is actually queried/observed, return exact source-scoped `unsupported_callback_alias_variant` Unknown and prevent the complete-port No path. Do not emit an unconditional positive callback row for the alias and do not globally degrade every document when no alias artifact is in scope. Task 5B proves this with fake Definition facts; Task 6 later supplies snapshot BSL facts. | English configuration + Russian method, and Russian configuration + English method, are Unknown with the exact alias reason; arbitrary other-language wrong names are exact mismatch/No; canonical rows are positive. All permutations are stable. |
| B9 | Shared catalog and Task 4 split | Current neutral parser returns registrations/nested names but has no ScriptVariant/provenance. The rich catalog requirement spans Configuration plus registered descriptors. If mechanism semantics are simply added to Task 4's parse Result, a bad mechanism field would incorrectly become capture-fatal. | Extend `infrastructure/platform_xml.rs`; do not add a second registration parser. Use a two-phase pure API: (1) capture-authoritative envelope/registration/identity catalog used by Task 4; (2) semantic views used by 5B after verified reads. The shared result retains `Missing|Known|Unknown(exact)`, root/nested registration provenance, validated nested descriptor identity, central path construction, and central registration-to-ArtifactRef mapping. Missing/unknown ScriptVariant and bad mechanism fields must not fail Task 4 capture. | Existing Task 4 platform_xml/source_snapshot/project_sources tests stay green. New tests prove that Task 4 and Task 5 see byte-for-byte identical registration identities/provenance, while invalid mechanism fields pass capture and degrade only their owning 5B port. |
| B10 | Snapshot-bound I/O | The required reader exists and is sufficient, but 5B has no common adapter rule that prevents a provider from constructing paths or freshness independently. The contract table also contradicts the brief: it says source mismatch is a non-retryable contract violation `platform_xml_source_mismatch`; the brief/Task 5A correction says retryable Unavailable `source_set_mismatch` before I/O. | The brief rule wins and the contract table must be corrected. Every provider validates the exact query/source before any read; builds paths only from snapshot relative root + shared catalog; preflights manifest membership/length; reads through `read_verified`; rechecks bytes/length; creates Freshness only from the linked SourceSetSnapshot. `SourceFingerprintMismatch` or live read uncertainty returns retryable Unavailable with zero prefix. A manufactured record freshness mismatch remains fatal. | Reader spy: wrong requested analysis set performs zero reads and returns `source_set_mismatch`. Mutation before any required read returns retryable Unavailable and zero records. NotInManifest is mapped according to the exact required material, never silently to No. No 5B provider contains `std::fs`, `Path::exists`, SQLite, renderer, or workspace reopen calls. |
| B11 | Local malformed material vs whole-port Failed | The contract table says a malformed direct field/registered material is whole-port Failed, while the brief says no gap may erase complete facts from unrelated documents (except incomplete Configuration catalog). `ProviderOutcome::Failed` carries no records, so both cannot be true. The same issue exists for missing/malformed one Form.xml. | Deterministic document-local semantic failure is `Bounded` plus the narrowest safe source-qualified gap, retaining fully validated unrelated documents. Use exact Artifacts when all affected subjects are known; otherwise SourceSetWide. Whole-port Failed is reserved for a shared/root/parser/global material failure for which no reliable sub-batch exists. Task 4 capture remains fatal for malformed Configuration registration/root descriptor envelope/identity. Unavailable/deadline/read race discards the whole invocation prefix. | Two registered documents: one valid and one semantically malformed. The valid facts survive; no negative proof dependent on the bad document is emitted. Reversing documents is identical. A malformed Form A does not remove Form B facts or MetadataCatalog facts. A global catalog failure produces no prefix. |
| B12 | Bounds and deterministic truncation | Application collection now caps returned records, but an XML adapter could still build an unbounded Vec before collection. The 256-gap/2000-affected limits also lack a deterministic overflow algorithm. A 64 MiB byte cap alone does not prove the 128-depth/1,000,000-element contract. | Add a shared bounded XML parse/traversal helper with manifest preflight, post-read length check, checked depth/element counters, and injected deadline. Providers traverse canonical catalog order and keep only canonical smallest `maxEvidence` records using a bounded collector; they do not rely on application truncation for memory safety. Exact duplicates are removed before budget; location-distinct provenance records count separately. Canonically retain gaps within 256/2000 and replace unrepresentable remainder with one `platform_xml_result_limit` SourceSetWide or QueryWide sentinel. Deadline is retryable Unavailable with zero prefix. Global application `maxEvidence` still runs after all ports. | Boundary and +1 tests for bytes/depth/elements/records/gaps; forward/reverse file and provider order return identical kept facts/gaps; overflow never returns a filesystem/timing prefix; deadline returns zero records. |
| B13 | Exact support-independent scope | 5B providers must not derive gaps from SupportQueryPlan or raw support state. Several gaps occur before a handler ArtifactRef is safely parseable, so fabricating a guessed method would be unsafe. | Scope only from the exact metadata query, linked source snapshot, and shared catalog. Use the table below. Destination catalog processing emits membership facts/gaps only; it never emits runtime bindings/callbacks or support facts. | Identical XML with different support outcomes produces identical Metadata/Form records and gaps. Every 5B gap names only linked source sets and catalog/query artifacts. |

## Exact gap and outcome table for Task 5B

This table resolves the current `Artifacts`/`SourceSetWide`/`QueryWide` and
Bounded/Failed ambiguity.

| Observation | Outcome | Scope | Preserve unrelated records? |
| --- | --- | --- | ---: |
| Missing/unknown ScriptVariant for an exact requested/registered callback slot | Bounded | Artifacts: exact canonical/alias callback method subjects under the exact source | yes |
| Unsupported callback signature/alias after an exact callback+Definition query | Bounded/typed Unknown | Artifacts: exact method | yes |
| Unsupported direct exchange/BSP family with known owner/target | Bounded | Artifacts: exact known owner/target | yes |
| Unsupported QName source type with a valid subscription subject | Bounded | Artifacts: exact EventSubscription (and exact ExchangePlan too when resolved) | yes |
| Unsupported HTTP method/shape after exact route identity exists | Bounded | Artifacts: exact HttpRoute | yes |
| Malformed handler/QName/action before a safe target ArtifactRef exists | Bounded | SourceSetWide for the exact supplying source set | yes; discard dependent document facts |
| XML depth/node limit in one registered descriptor/Form.xml | Bounded | Artifacts if every dependent subject is known, else SourceSetWide | yes; discard dependent document facts |
| XML depth/node limit in Configuration.xml | Bounded | SourceSetWide for that source-set catalog | yes for sibling source sets; no facts from that catalog |
| Per-provider/result/gap overflow confined to one source set | Bounded | SourceSetWide | yes, canonical retained set only |
| Cross-source global result overflow with no exact bounded set | Bounded | QueryWide | yes, canonical retained set only |
| Task 4 capture-authoritative malformed registration/root descriptor identity | no provider invocation | snapshot error | no snapshot prefix |
| Shared/global provider parser invariant failure | Failed | whole port issue | no |
| Fingerprint/I/O race or provider deadline | Unavailable, retryable | whole invocation issue | no prefix |
| Provider manufactures wrong freshness, source, fact kind, or endpoint | ContractViolation | fatal | no |

For a local malformed document, a narrow artifact gap is allowed only when it
also covers every proposal/candidate whose negative runtime conclusion depended
on the missing binding. If an invalid lexical handler prevents naming that
method, SourceSetWide is the safe result; a gap on the registration owner alone
would let an unrelated explicit method proposal become falsely contradicted.

## Required query/response boundaries

The current single `DiscoveryQueryPlan` is not sufficient to validate provider
completeness. Before concrete adapters, add exact typed query layers (names may
vary):

```text
MetadataCatalogQuery {
    discovery,
    destination_membership: sorted unique SourceScopedArtifact[],
}

FormInspectionQuery {
    discovery,
    exact_form_commands: sorted unique analysis SourceScopedArtifact[],
}
```

Rules:

1. The use case, not infrastructure, derives destination membership from every
   CFE proposal's `ArtifactOwnershipChain`.
2. Destination membership permits only registered root owners and registered
   forms. Methods remain Definition evidence; FormCommands remain
   FormInspection evidence.
3. MetadataCatalog scans all registered supported v1 mechanisms in analysis,
   but scans mutations only for the exact membership list.
4. FormInspection reads only analysis Form.xml files and owns command
   presence/absence, structure, and action bindings.
5. Complete empty is negative proof only for the exact typed scope. Response
   validation must make an accidentally omitted requested key a provider
   contract violation, not a false absence.

## Callback registry/join lock

The four-row registry must be one pure table and include:

- a stable registry version in each requirement digest;
- a stable semantic callback slot pairing the two language rows;
- exact ScriptVariant, metadata kind, module kind, method, callable kind,
  export policy, ordered parameter flags, and execution-context requirement;
- no aliases/synonyms outside the exact opposite row of the same slot;
- Document object-module positive fixture using the explicitly documented
  module-default/server rule; CommonCommand requires explicit AtClient;
- a whole-fact smart constructor and exact endpoint validation.

The graph joins by exact source set + specialized owner + slot + module/method,
not by character class or lexical similarity. Task 5B does not implement BSL,
so its positive/negative/alias join tests use fake Definition facts. The real
snapshot-backed Definition facts remain Task 6.

## Shared catalog lock

The neutral parser remains `infrastructure/platform_xml.rs`. The minimum rich
pure model is:

```text
PlatformConfigurationCatalogV1 {
    script_variant: Missing | Known(Russian|English) | Unknown(exact bounded),
    root_registrations: sorted unique RegistrationWithProvenance[],
}

RegisteredDescriptorCatalogV1 {
    exact_root_identity,
    forms/templates/commands: sorted unique RegistrationWithProvenance[],
}
```

It must expose central safe path and ArtifactRef mapping helpers. Task 4 may
consume compatibility projections, but no caller may independently interpret
Configuration/ChildObjects. Task 5B additionally validates registered nested
Form descriptor identity before using its Form.xml. Unknown ScriptVariant is a
semantic observation, never a Task 4 capture error.

## Mandatory fixtures missing before 5B

`git ls-files tests/fixtures/project_discovery` is currently empty. There is
also no tracked CommonCommand descriptor/module fixture. Task 5B therefore
needs a new minimal synthetic fixture family, not borrowed business data.

Minimum fixture groups:

1. shared catalog: BOM, namespace prefixes, CRLF/mixed endings, direct vs
   descendant decoys, duplicate singleton, ScriptVariant Missing/Known/Unknown,
   root and nested registration provenance;
2. destination membership: analysis + two extensions, root present/absent,
   nested form present/absent, destination-A-only resource gap;
3. each seven flow: positive, second valid alternative, wrong exact binding,
   semantic malformed, lexical decoy, registered hard decoy, valid/malformed
   unregistered decoy, permutation, source mismatch, verified-read mutation,
   exact locations;
4. forms: Document/Report/DataProcessor owners, missing command, duplicate
   action, four call types, malformed Form A plus valid Form B;
5. callbacks: all four rows, registry-version digest, cross-owner rejection,
   compatible join, complete absence, exact mismatch dimensions,
   Val/default/extra optional Unknown, both cross-language directions, and
   arbitrary Cyrillic/ASCII wrong-name non-aliases;
6. QName and HTTP boundary matrices from the 5B contract;
7. bounds: bytes/depth/elements/result/gap exact boundary and +1, plus
   deterministic deadline and permutation tests;
8. scoped local failure: two valid documents around one bad document, and
   three source sets proving no sibling contamination.

Large boundary XML should be generated deterministically inside tests; do not
commit 64 MiB/1,000,001-node fixtures.

## Task 5C cross-task appendix: batch support receipts

The current Task 5C design row `UUID resolution failed -> Failed, no fact`
contradicts the now-exact batch `SupportQueryPlan`: one invocation can contain
many independent `(source_set, artifact)` keys. Failing the whole port for one
unresolved UUID discards valid facts for all other subjects.

Required correction before Task 5C implementation:

- deterministic subject-local UUID resolution failure returns a Bounded batch,
  no fact for that key, and
  `ProviderGap::Artifacts("support_subject_unresolved", [exact key])`;
- all unaffected requested keys still return exactly one raw fact;
- aggregate multiple unresolved keys deterministically without weakening them
  to QueryWide/SourceSetWide;
- malformed/unsupported shared ParentConfigurations material, a root/parser
  invariant, or another truly whole-port material failure remains Failed with
  zero facts;
- fingerprint/I/O race remains retryable Unavailable with zero prefix;
- a Complete response still requires exactly one raw fact for every key;
- `validate_support_response` must reject `support_subject_unresolved` carried
  as QueryWide/SourceSetWide;
- align `MAX_SUPPORT_QUERY_SUBJECTS` and the total exact affected-artifact cap,
  or chunk before the one-call invariant is frozen. The current 4096 query-key
  cap versus 2000 total exact affected-artifact cap cannot represent the
  worst-case exact unresolved batch.

Required RED: request three exact keys, make the middle UUID unresolved, and
assert a Bounded result with facts for keys 1 and 3 plus one exact gap for key
2, stable under request order. A malformed shared file must instead return
Failed/zero, and a read race Unavailable/zero.

## Suggested execution order

1. Accept Task 5A and synchronize active spec/product contract (B0-B7
   application-contract portions). B8 is completed together with the concrete
   four-row registry in Task 5B.
2. RED/GREEN the shared rich catalog while keeping every Task 4 regression
   green (B9).
3. Add exact Metadata/Form queries and response validation; prove destination
   membership and FormInspection ownership before seven-flow adapter code.
4. Add the closed callback registry, fact constructor, aggregated join, and
   alias tests with fake Definitions.
5. Implement the seven flows one vertical slice at a time.
6. Add local Bounded failure handling and deterministic bounded collectors.
7. Run focused catalog/provider/application/snapshot tests, then full tests,
   fmt, clippy `-D warnings`, product contracts, and `git diff --check`.

## Final gate

Task 5B is ready to implement only when all of the following are true:

- accepted clean Task 5A SHA exists;
- active spec and 5B brief/contract agree on source mismatch and local failure;
- SourceSetWide is accepted and stable-encoded;
- exact destination membership query/response semantics exist;
- FormInspection can prove FormCommand existence and structure;
- unconditional CommonCommand Binding is rejected;
- callback facts bind registry version and exact endpoints;
- callback join aggregates provenance and uses a closed alias slot;
- the first RED fixtures above fail for the intended missing behavior.

Until then, starting XML adapter code would force policy guesses into
infrastructure or make Complete-empty batches look like proof they do not have.

## Token usage

The execution environment does not expose an exact per-subtask token counter;
an exact number cannot be reported without inventing one.
