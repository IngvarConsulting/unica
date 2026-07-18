# Task 5A Report: Application Discovery Contract And Orchestration

Date: 2026-07-18

Base and current HEAD: `20f6afa7a09430614babebc0cdeebeb94c8a0189`.

Status: implementation and tracked documentation synchronized; no commit has
been created. Task 5A remains an internal application contract. It does not
register the public MCP tool, implement concrete Platform XML/support/BSL
providers, validate mutation-tool argument compatibility, or persist a
discovery receipt.

## Accepted Slice Log And RED -> GREEN Evidence

The closure also rechecked all six immutable controller review notes without
editing them:

- `.superpowers/sdd/task-5a-root-active-review.md` —
  `93348356ee63fd419c3b54a0fbeb6fc8b7c554db8d6148ece961f93bf72e0ea6`;
- `.superpowers/sdd/task-5a-root-active-review-2.md` —
  `4881d71af04d2e7997702119d7b0822955693229a6787458dd524917f8d5e2e1`;
- `.superpowers/sdd/task-5a-root-active-review-3.md` —
  `a431c7ddd4c64b02faa3ed07232d1c5befc76208860f5dfd70e0e7ac032c41ec`;
- `.superpowers/sdd/task-5a-root-active-review-4.md` —
  `77ccebebf7130ec4cc0e5cc97627e780b6e895df4af87e2e65b767dc4f225ee4`;
- `.superpowers/sdd/task-5a-root-active-review-5.md` —
  `99ec37a7c805f6569a67c96a337b9204f6f9482644f7be54dff295f09b452dc6`;
- `.superpowers/sdd/task-5a-root-active-review-6.md` —
  `65db34f614edcd99d5a13fe6728602502b31feeadab9a8a891fb7e25ee159fd2`.

The controller accepted the implementation in bounded slices. The RED entries
below name the failing contract/reproducer that preceded each implementation;
the GREEN entries name the focused regression that now owns it.

### 1. Binding, ownership, callback, support, and orchestration foundation

RED:

- the original binding model still admitted `ExchangePlan/handles` and had no
  exact `SubscriptionSource/uses` fact;
- connected ExchangePlan/callback behavior, typed callback compatibility,
  raw source support facts, staged support subjects, and exact ownership were
  absent;
- the intermediate review recorded seven live failures:
  `bounded_material_records_keep_supported_verdict_but_status_insufficient`,
  both `metadata_callback_makes_*_form_inspection_optional` rows,
  `query_wide_gap_is_material_to_every_port_consumer`,
  `callback_definition_absence_is_no_only_with_complete_definition_coverage`,
  `structural_contains_edge_is_observed_but_never_runtime_reachable`, and
  `every_runtime_port_that_contributes_a_connection_is_material`.

GREEN:

- smart-constructed `ValidatedBinding`, closed direct-parent ownership,
  `ArtifactOwnershipChain`, domain-owned `BslExecutionContext`,
  `FormCallType`, and `ScriptVariant`;
- exact `SubscriptionSource/uses`, complete ExchangePlan chain promotion,
  callback+Definition join/rejections, raw `SupportFactState`, and one staged
  support query;
- focused ownership, binding, determinism, callback, exchange, support, and
  use-case tests are part of the discovery suite.

### 2. Destination-membership domain facts and stable identity

RED:

- the application had no canonical non-nil `PlatformUuid`,
  `MetadataIdentity`, or typed `CfeObjectMembership` payload and therefore
  could not bind configuration identity to destination membership;
- stable tags/digests had no rows for those facts.

GREEN:

- `platform_uuid_accepts_only_canonical_non_nil_uuid_shape`;
- `metadata_identity_and_cfe_membership_are_closed_typed_facts`;
- exhaustive provider-fact/stable-tag and determinism tests.

### 3. Exact membership query and provider boundary

RED:

- metadata queries could not express exact configuration/destination owner
  pairs and a complete destination silence could be misread without a typed
  companion contract;
- wrong-source, orphaned, conflicting, post-limit, and out-of-plan companions
  were not rejected at one boundary.

GREEN:

- `destination_membership_pair_binds_exact_analysis_and_mutation_subjects`;
- `metadata_query_plan_preserves_exact_membership_pairs`;
- the `metadata_membership_*`, bounded-companion, exact-scope, and
  pre-limit-completeness test group.

### 4. UUID join, support projection, provenance, and CFE preflight

RED:

- the earlier name/presence-only projection could report
  `ExtensionOwned` without proving which configuration UUID was adopted;
- absent, `Own`, mismatched `Adopted`, source gaps, root/form precedence,
  provenance, and extension-analysis preflight were not fail-closed.

GREEN:

- `cfe_root_membership_is_joined_by_exact_uuid_and_never_borrows_implicitly`;
- `cfe_root_and_form_membership_uses_indeterminate_precedence`;
- all `cfe_membership_*` gap/limit/provenance/determinism rows;
- `cfe_mutation_from_extension_analysis_fails_closed_before_provider_io`,
  deduplicated preflight, and mixed-proposal preflight tests with stable
  `cfe_analysis_configuration_required`.

### 5. Runtime mechanism profile and exact partial dependencies

RED:

- positive port capability was incorrectly being used as negative authority;
- the active spec's blanket Metadata+CallGraph+FormInspection negative clause
  contradicted unsupported-variant `unknown` behavior;
- a partial `ExchangePlan --uses--> EventSubscription` gap could yield false
  `No/Contradicted`, and callback-owner negatives omitted exact canonical/alias
  Definition dependencies.

GREEN:

- `runtime_mechanism_profile_v1_is_closed_and_registry_driven`;
- `runtime_profile_ordinary_method_ignores_non_mechanism_metadata_gap`;
- `unsupported_implicit_runtime_variants_never_become_contradicted`;
- `callback_owner_runtime_negative_depends_on_exact_definitions`;
- `partial_exchange_runtime_negative_uses_exact_observed_subscription` and
  exact `potential_runtime_endpoints` graph tests.

### 6. Closed candidate capability

RED:

- the adversarial reproducer proved that generic
  `positive_existence && connected` promoted Document context and ExchangePlan
  context to actionable candidates and let either consume `maxCandidates=1`.

GREEN:

- `candidate_role_matrix_is_exact_and_record_order_independent`;
- `candidate_kind_allowlist_covers_all_fifteen_tags`;
- forged connected Document/ExchangePlan roots cannot cross the allowlist;
- Document callback and complete ExchangePlan chain limit regressions keep the
  contextual roots out of candidate slots.

### 7. Lossless gap diagnostics with bounded public summaries

RED:

- a legal provider batch with 129 distinct material gaps reached
  `DiscoveryReport::validate()` with a list over the public 128 limit and
  became a fatal `DiscoveryError::Operation`;
- provider gaps permit 256 rows, so lowering that limit would have hidden the
  root contradiction.

GREEN:

- `material_gap_boundaries_are_nonfatal_lossless_and_order_independent` covers
  128, 129, and 256 gaps plus reverse order;
- semantic reasons remain before the largest canonical gap prefix;
- per-gap Checks stay exact, duplicate reason scopes remain separate, and a
  provider cannot forge `material_gap_reason_overflow`.

### 8. Application-owned mutation-intent receipt gate

RED:

- exact focused failure for R1:
  `left: []`, `right: ["mutation_intent_required"]`; the old path called the
  issuer and returned eligible for a fully supported proposal with no intent;
- the same defect allowed mixed selections to delegate a subset invariant to
  the future issuer.

GREEN:

- `mutation_intent_receipt_gate_is_all_or_nothing_and_independent` covers
  R1-R6 and canonical independent blocker union with zero issuer calls;
- `missing_mutation_intent_only_blocks_receipt_not_proposal_validation` proves
  verdict, coverage gaps, and proposal blockers are unchanged;
- `optional_code_search_degradation_keeps_supported_mutation_receipt_eligible`
  is R7 and proves exactly one issuer call when all gates pass.

### 9. Tracked source-of-truth synchronization

RED command:

```text
python3.12 -m unittest tests.ci.test_product_contracts.ProductContractTests.test_project_discovery_architecture_is_synchronized -v
```

The focused failure was:

```text
AssertionError: 'Historical execution context only' not found
```

GREEN: the focused test now parses exact endpoint-aware binding rows, runtime
profiles, candidate roles, the CFE lattice, material bounds, and the receipt
gate from the active spec/ADR. The historical plan is checked only for its
banner and forbidden contradictions; it is no longer an equal contract source.

### 10. Closed EventSubscription source and signature registry

RED:

- selector deduplication happened after a differently ordered sort and could
  miss non-adjacent case-equivalent identities;
- descriptor sources and ExchangePlan `uses` projections could disagree;
- handler arity was inferred through a guarded wildcard rather than one closed
  21-cell table.

GREEN:

- the exact 13-family/21-cell registry owns compatibility and three signature
  classes;
- descriptor construction rejects exact, case-equivalent, and accepted
  Unicode-lowercase duplicates before canonical sorting;
- the provider boundary enforces exact selected ExchangePlan set equality;
- fixed registry cardinality/class/arity tests and forward/reverse descriptor
  projection tests are green.

### 11. Whole CFE observations and universal source binding

RED:

- independent flavor, UUID, membership, presence, or freshness facts could be
  assembled across sources into authority that no one source had observed.

GREEN:

- `AnalysisMetadataAuthorityObservationV1` and
  `DestinationMetadataMembershipObservationV1` carry role-specific whole
  source-qualified payloads;
- universal provider validation checks embedded source identity against record
  freshness before query-specific validation;
- equal UUID decoys from another source cannot satisfy the join.

### 12. Declarative Definition join and ScheduledJob activation

RED:

- declarative bindings were promoted to runtime edges without their exact
  handler Definition;
- Definition identity omitted the async bit;
- ScheduledJob collapsed `Use`, `Predefined`, `Global`, and `Server`, and a
  disabled job could not be represented without a handler binding;
- Form/HTTP compatibility was invented without an accepted primary-backed
  policy.

GREEN:

- Definition identity retains sync/async and exact BSL context;
- EventSubscription and active predefined ScheduledJob use closed compatibility
  rows, with hard mismatches as `No` and unsupported async/explicit-context
  variants as `Unknown`;
- missing/conflicting Definition remains `Unknown`, and Form/HTTP remain
  policy-unavailable;
- metadata-only ScheduledJob activation branches do not require or materialize
  a handler Definition; positive binding retains the complete typed descriptor;
- runtime rejection aggregation uses the complete Yes/Unknown/all-No lattice.

### 13. Semantic atomic evidence ceilings

RED:

- record-by-record truncation could split CFE halves and EventSubscription
  descriptor/projection clusters;
- generic union-by-material-subject merged independent descriptors through one
  shared handler or Definition;
- skip-and-continue retained a smaller later group after an earlier group did
  not fit;
- SHA sorting substituted for the frozen per-port, global, and inner typed
  tuples.

GREEN:

- the seven-variant `SemanticAtomicGroupIdV1` has explicit stable tags and
  role-bearing CFE halves; groups never span ports;
- two subscriptions sharing one handler remain independent and Definition is
  standalone;
- per-port and global ceilings retain the largest whole-group canonical prefix
  and gap the complete dropped suffix;
- source-group rank is Analysis=0 and every Destination=1, followed by the
  domain-owned source identity without fingerprint;
- source-free CFE cluster digests exclude embedded source names but preserve the
  complete role-specific typed payload;
- fixed/permutation tests cover group tags, multiple sources, distinct
  per-port/global order, inner options/location/relation/object/fact order, and
  CFE/Event all-or-none behavior.

## Accepted Decisions

Primary-source checks used for the platform-specific contract:

- 1C EventSubscription developer guide:
  https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_5._Configuration_objects/5.3.__Common__configuration_branch/5.3.8._Event_subscriptions/?language=en
- 1C ScheduledJob developer guide:
  https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.22_Developer_Guide/Chapter_19._Job_feature/19.3._Scheduled_jobs/?language=en

- Binding tag 4 remains reserved. CommonCommand has no declarative binding.
- Form-to-FormCommand containment belongs to `FormInspectionPort`; other
  accepted direct-parent structural rows belong to `MetadataCatalogPort`.
- ExchangePlan is connected context only after the full exact
  uses/subscribes chain and is never a candidate.
- Positive runtime evidence and negative authority are separate contracts.
- CFE support is an exact configuration-UUID to extension-membership join;
  patch assessment never borrows implicitly.
- ProviderGap/Check diagnostics remain lossless within their own bounds;
  bounded public reason arrays use an application-owned sentinel.
- Every selected proposal needs typed mutation intent before issuer assessment,
  while absence remains irrelevant to the proposal verdict itself.
- `maxGraphDepth` is strict input and analysis-ID material now; traversal
  semantics are deferred to Task 7.
- Current `ReceiptIssuerPort::assess` returns eligibility only. Task 9 owns
  persistent issuance, lease, and receipt state.

## Remaining Boundaries

### Task 5B

Implement the Platform XML metadata/binding/form provider from primary source:
registered objects only, exact callback rows, direct UUID locations, exact
destination membership (`Own` or `Adopted`), document-local bounded parse
failure, and no partial membership companion after failure. Task 5B must
consume, not redefine, the Task 5A application contract.

### Task 5C

Implement support evidence and `ParentConfigurations.bin` parsing with exact
missing/malformed/I/O/support distinctions. Keep borrowing as explicit
`unica.cfe.borrow`; CFE patch tooling must never borrow as a side effect.

### Task 6

Implement bounded typed BSL definitions, lexical search, and call edges behind
the existing ports. Comments/strings remain lexical decoys, dynamic calls stay
bounded/unknown, and adapters must not expose or parse display-oriented SQLite
state through the application boundary.

### Later work

- Task 7: concrete multi-mechanism orchestration, traversal roots/depth, and
  end-to-end mechanism families.
- Task 8: shared mutation target/argument resolver and compatibility.
- Task 9: persistent receipt model, atomic store, issuance, and lease.
- Task 12: public MCP registration and packaged delivery.

## Known Risks And Explicit Contradictions

- Historical plan receipt/guard prose is a future target, not current
  behavior. The banner and active spec/ADR now say so explicitly.
- The input accepts and hashes `maxGraphDepth`, but Task 5A does not traverse by
  that value. Claiming otherwise would be a false implementation claim.
- Task 5A fake providers prove the application boundary; they do not prove the
  future Platform XML/support/BSL adapters.
- EDT with a recognized marker produces the bounded diagnostic
  `unsupported_source_format` report and never receipt eligibility. Other
  unsupported layouts fail source readiness before providers.
- An eligible assessment from a fake issuer is not a persisted receipt.
- Strict Clippy is part of closure. Mechanical diagnostics found during the
  repair were corrected in the tracked implementation rather than recorded as
  an accepted exception.

## Tracked Diff Hash And Reproduction

Final tracked diff SHA-256: `<fill-after-final-verification>`.

Recompute from the frozen base without including ignored SDD evidence:

```text
BASE=20f6afa7a09430614babebc0cdeebeb94c8a0189
git diff --binary "$BASE" -- . | shasum -a 256
git diff --check "$BASE"
git status --short
```

## Final Verification Evidence

- `python3.12 -m unittest tests.ci.test_product_contracts -v`: GREEN, 14
  passed.
- `cargo test --locked -p unica-coder --lib discovery`: GREEN, 168 passed,
  441 filtered out.
- `cargo test --locked -p unica-coder`: GREEN, 609 library tests passed; main
  and doc-test targets had zero tests and passed.
- `cargo fmt --all -- --check`: GREEN.
- `git diff --check 20f6afa7a09430614babebc0cdeebeb94c8a0189`: GREEN.
- `cargo clippy --locked -p unica-coder --all-targets -- -D warnings`: GREEN
  after the final repair pass.
