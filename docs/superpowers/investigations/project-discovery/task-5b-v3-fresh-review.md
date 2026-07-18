# Task 5B v3 — fresh adversarial implementation-readiness review

> **RESOLVED AS A DESIGN INPUT BY TASK 5B v5.** The P0/P1/P2 findings below are
> closed normatively by `task-5b-contract.md` v5 and verified by the separate v5
> self-audit. The operational G0 remains open until an accepted immutable Task 5A
> commit exists. Do not implement from this v3 review. Its pre-resolution
> SHA-256 was
> `cd52d99e2b46c3328443148ec5d7b9be01b92bf22c8216eb37e732379f45e76a`.

Date: 2026-07-18

## Review anchors

- `task-5b-contract.md`: `3062b1d34ba0e93185aee55ca2a3ac05b10b68d6732be039e0db36709535f2cf`
- `task-5b-brief.md`: `03fa268474f5c2937208fce6553837d2d8ffb759de158b94246db05023146d86`
- `task-5b-design-review.md`: `00cdf889aee69cbc87cec70f06536cbad910825089ff004f4c4b651952c6d381`
- `task-5b-preimplementation-audit.md`: `9d78df86012923a57281fbaab4df559c00b0343714fb35b78e75ee9593239a62`
- `task-5b-v2-review.md`: `55db8d0fa2df289e6654b5c2faf0779fcc7930473428c9e98485a83af3dcaaed`
- current Task 7 design observed during review: `8e719dde23e61400c965f97c784325f54dfec2320c4c3d0cc8ca71cb37a428fe`
- current Task 8 design was changing concurrently; the reviewed clauses are cited by exact lines/content below, not treated as an immutable base
- checkout HEAD: `20f6afa7a09430614babebc0cdeebeb94c8a0189`, with the Task 5A candidate still uncommitted

No tracked files were edited and no production tests were run.

## Verdict

**NEEDS-FIX — v3 is not implementation-ready.**

The v3 text closes the old byte-permutation, capture/provider ownership, all-invalid/mixed preflight, manifest-before-I/O, presence-key, composite-call choice, historical-banner, scalar, and closed-ChildObjects ambiguities. The old accepted-Task-5A P0 remains an intentionally documented but still unsatisfied operational gate. In addition, the three P0 contract holes and three P1 holes below must be corrected before the first Task 5B RED.

## P0

### P0-G0 — the accepted Task 5A base still does not exist

Evidence:

- The contract explicitly blocks work until an accepted, committed, clean, spec-synchronized Task 5A SHA exists (`task-5b-contract.md:1-18,65-67,1460-1468`).
- HEAD remains `20f6afa7a09430614babebc0cdeebeb94c8a0189`, while Task 5A application/domain/spec/product-contract files are modified and uncommitted.

Impact: this is not a new v3 wording defect, but prior P0-1 is not operationally closed. There is no reproducible application boundary on which Task 5B can be implemented or reviewed.

Required correction: finish Task 5A review/GREEN/spec synchronization, commit it, start from a clean worktree, and record the exact commit as `TASK5A_ACCEPTED_SHA`. This is a Task 5A gate; no Task 5B infrastructure workaround is valid.

### P0-1 — CFE receipt authority trusts topology labels and wrapper UUIDs without captured flavor/Own proof

Evidence:

- The preflight checks only declared `SourceSetKind::Configuration` (`task-5b-contract.md:109-130,153-190`).
- The parser computes captured `BaseConfiguration|ExtensionConfiguration`, but the contract expressly says the public seven-flow evidence does not use flavor for topology (`task-5b-contract.md:520-551`).
- For every requested analysis half, section 8 emits `MetadataIdentity` directly from the descriptor `@uuid`; destination membership is classified independently, with no requirement that analysis flavor is Base, the analysis descriptor is Own, or destination flavor is Extension (`task-5b-contract.md:721-740`). The exact pair contract then requires that companion and permits the UUID join (`task-5b-contract.md:329-344`).
- Live Task 5A projection likewise compares one `MetadataIdentity` UUID with one destination `Adopted` UUID and has no flavor/analysis-Own fact (`proposal_validator.rs:637-715`).
- Current Task 8 correctly requires Base + analysis Own + destination Extension before UUID comparison and assigns `cfe_configuration_flavor_mismatch` / `analysis_descriptor_not_base_owned` (`task-8-design.md:66-74,464-470,2190-2194`). That stronger rule currently arrives after discovery receipt eligibility.

Minimal REDs:

1. Map the tracked adopted extension-shaped `Configuration.xml` as a declared Configuration analysis source; choose its local wrapper/object `@uuid` equal to a destination `ExtendedConfigurationObject`. The result must be Unknown/ineligible, `cfe_configuration_flavor_mismatch`, and zero issuer calls. It must never become `ExtensionOwned`.
2. Use a Base-flavor analysis catalog whose requested root or Form descriptor is `Adopted`. It must stop before UUID comparison with `analysis_descriptor_not_base_owned`.
3. Map a Base-flavor catalog as a declared Extension destination. It must be Unknown/ineligible before membership promotion even if its bytes can otherwise construct an Own/Adopted-looking row.

Required correction: back-propagate the Task 8 rule into the accepted Task 5A application/domain response contract and receipt gate, then make Task 5B emit/validate pair companions only under Base+Own analysis and Extension destination authority. Reuse the shared typed flavor/membership parser; do not add a second parser or compare an extension wrapper UUID. This is a Task 5A + shared-domain + Task 5B correction, not Task 5B-only adapter logic.

### P0-2 — metadata authority is local-name-only and accepts a foreign namespace as platform metadata

Evidence:

- Metadata structural matching is defined only by exact local name; namespace prefix spelling is the only stated namespace rule (`task-5b-contract.md:460-476,664`). No exact MDClasses URI is required.
- The live Task 4 parser uses `tag_name().name()` in every authoritative helper (`platform_xml.rs:19-38,156-193`). Its test explicitly accepts a complete metadata envelope in `xmlns:md="urn:1c"` (`platform_xml.rs:222-233`).
- Tracked platform fixtures use `http://v8.1c.ru/8.3/MDClasses`; the contract already treats exact namespace as authority for Form XML (`task-5b-contract.md:599-602,693`).

Impact: a same-local-name foreign-namespace document can be captured as authoritative, populate registrations/identity/runtime evidence, and reach receipt issuance even though it is not 1C metadata XML. This is not merely a decoy-field issue: the complete root can be foreign.

Minimal REDs:

1. The current `urn:1c` prefixed envelope must fail Task 4 capture before manifest authority and before any provider call.
2. The same prefixed document bound to exact `http://v8.1c.ru/8.3/MDClasses` must pass.
3. A correct MDClasses parent with a same-local-name direct child in another namespace must neither satisfy nor duplicate the authoritative field; the exact required MDClasses node/cardinality decides the outcome.

Required correction: change the shared capture/parser helpers to consume authoritative metadata elements only in the exact MDClasses URI, with arbitrary prefix spelling. A foreign root/envelope or same-local substitute cannot become authority; if no exact required node remains, capture fails closed. Closed authoritative collections must also reject unconsumed foreign binding-shaped children rather than infer a registration from local name. Migrate Task 4 and rerun its full snapshot corpus before provider work. This is a shared parser + Task 4 correction; a Task 5B-only semantic check is too late.

### P0-3 — EventSubscription emits a positive runtime edge for any identifier-shaped Event

Evidence:

- The contract defines `Properties/Event` only as an `event identifier` and treats any structurally valid identifier as sufficient for `subscribes` (`task-5b-contract.md:870-892`). Its RED matrix checks a wrong event *shape*, but no unknown event token or event/source compatibility (`task-5b-contract.md:1355-1374`).
- `ValidatedBindingDetails::EventSubscription` stores an untyped `String`, and its smart constructor applies only the generic identifier validator (`model.rs:512-523,576-596,762-775`).
- Event is a platform enum whose applicability depends on source type. A typo such as `TypoBeforeWrite`, or a real token used with an incompatible source kind, does not prove that the platform invokes the handler.

Impact: malformed or semantically inapplicable metadata can emit `EventSubscription --subscribes--> Method`, promote a false runtime path, and make a receipt eligible.

Minimal REDs:

1. Exact valid handler plus `Event=TypoBeforeWrite` must emit no `subscribes` fact and must produce a material scoped unsupported/malformed gap.
2. A known event paired only with an incompatible resolved source kind must emit no positive edge.
3. Every supported event/source row and N/N+1 registry boundary must be exhaustive and versioned; an unrecognized future token fails closed rather than becoming a generic identifier.

Required correction: introduce one closed, versioned domain event/event-source compatibility registry and make the whole-fact smart constructor accept its typed validated result, not `&str`. Task 5B resolves all source types before positive promotion; unsupported or inconclusive combinations remain Unknown with exact scope. This back-propagates through the Task 5A-owned application/domain binding contract and active spec, then is consumed by Task 5B; it is not a parser-local string check.

## P1

### P1-1 — `CompleteFormMethodBindingsV1` duplicates and contradicts the live audited Form event authority

Evidence:

- v3 creates a second closed item table, marks `Button` event-capable, accepts any canonical Event name, accepts explicit callType without classifying regular versus borrowed extension Form, and declares zero Action a valid command-without-runtime case (`task-5b-contract.md:613-650,693-714,1376-1403`).
- The existing audited registry includes additional known event-bearing kinds such as RadioButtonField, TrackBarField, ExtendedTooltip, document fields, GraphicalSchemaField, HTMLDocumentField, and SpreadSheetDocumentField (`form_event_registry.rs:262-286`); it marks Button as `NO_EVENTS` (`form_event_registry.rs:370-389`).
- That registry validates exact event names and rejects callType on a regular Form (`form_event_registry.rs:529-586`). The live Form validator reports a missing/empty command Action as an error (`form.rs:849-878`). Repository extension guidance also limits Before/After/Override callType to borrowed forms with direct `BaseForm`.

Impact: v3 can declare semantically invalid Form material Complete and use it for Task 8 negative Ordinary/unbound proof; it also makes ordinary valid forms containing already-supported item kinds needlessly inconclusive. The duplicate tables will continue to drift.

Minimal REDs:

1. Regular Form + `OnOpen callType=After` is incomplete; the same proven event/callType in a borrowed extension Form follows the exact extension rule.
2. Button/Click is not accepted by the current platform matrix; RadioButtonField/OnChange and each other already-audited kind are consumed exactly once.
3. An unknown Event name never leaves a Complete catalog.
4. Zero-Action Command is incomplete unless a real authoritative platform fixture proves an exact Form-definition case where it is valid; do not bless zero globally.

Required correction: extract one neutral versioned Form definition/item/event/callType registry from the existing audited implementation and reuse it in Form edit/validate, Task 5B, and Task 8. Parse direct BaseForm presence only to classify Form semantics; do not traverse BaseForm bindings. Reconcile command Action cardinality from authoritative fixtures before claiming Complete. This is shared parser/registry + Task 5B + Task 8 work, not Task 5A membership work.

### P1-2 — the mandated missing-Form gap cannot cover requested FormCommand polarity

Evidence:

- A missing registered analysis `Ext/Form.xml` is specified as an exact Form-scoped gap with no command absence (`task-5b-contract.md:272-285`).
- The same contract requires every omitted requested FormCommand presence key in Bounded coverage to be covered by exact Artifacts, matching SourceSetWide, or QueryWide (`task-5b-contract.md:297-303`).
- `ProviderGapScope::Artifacts` is exact, not hierarchical, and both presence validation and runtime negative coverage use exact membership (`ports.rs:965-979`; `proposal_validator.rs:395-414,494-506`). A gap naming the Form does not cover `FormCommand.C` or its FormModule method.

Minimal RED: request FormCommand C under a registered Form whose mandatory Form.xml key is absent. Returning only the contract-mandated Form gap must not produce a contract violation and must not let C/the potential handler receive Complete negative coverage.

Required correction: define the exact affected-set projection for this case. It must include the Form, every requested FormCommand key under it, and every material potential runtime subject/object, or conservatively use the matching analysis SourceSetWide scope. Add response-validation and proposal-verdict REDs. The preferred fix is Task 5B query/provider scoping plus application contract tests; do not silently introduce hierarchical gap semantics globally.

### P1-3 — `maxEvidence` truncation names a reason but not the material gap scope

Evidence:

- v3 requires a complete semantic scan, retains canonical records, and returns `platform_xml_result_limit`, but never defines the affected artifacts for each dropped record (`task-5b-contract.md:342-362,1098-1107,1197,1441`).
- Exact artifact gaps affect only exact runtime-negative subjects (`proposal_validator.rs:470-506`). Dropping a binding while scoping the limit only to its owner can leave the target Method Complete and permit a false negative conclusion. The companion sentence at `task-5b-contract.md:342-344` promises Unknown after a dropped companion without defining how the gap achieves it.

Minimal RED: produce two material binding records with `maxEvidence=1`, make the dropped record the only `handles` edge to an explicit proposal target, and assert that the proposal is Unknown under both forward and reversed same-snapshot processing. Repeat for a dropped identity/membership companion. Merely asserting Bounded or record count is insufficient.

Required correction: define one canonical `material_gap_subjects(record)` projection that includes every source-scoped subject, object, presence/identity companion key, and potential runtime endpoint whose conclusion changes when that record is dropped. Accumulate those exact subjects under `platform_xml_result_limit`; if the existing affected-artifact cap is exceeded, use the already-defined QueryWide `platform_xml_gap_limit` sentinel. This needs Task 5B provider + shared application materiality tests and Task 7 corpus back-propagation; the existing exact gap model need not change.

## P2

### P2-1 — Task 7 still contains the superseded per-source invocation contract

Evidence:

- v3 correctly announces the replacement (`task-5b-contract.md:346-351,1480-1494,1518-1527`).
- The current Task 7 design still requires one MetadataCatalog invocation for every captured source and retains `metadata_runs_once_per_captured_source_and_form_runs_analysis_only` (`task-7-design.md:763-786,1343-1354`).

Impact: there is no remaining v3 ambiguity for a reader who starts with Task 5B, but the standalone Task 7 handoff still directs a later worker to implement the incompatible boundary the v3 acceptance gate forbids.

Required correction: update Task 7 Stage 1, scoped-fault/query-digest text, RED name/expectation, and corpus before Task 5B acceptance. This is documentation/corpus back-propagation; it is not a provider implementation detail.
