### Task 3: Typed evidence ports, graph, validation, and materiality

**Files:**
- Create: `crates/unica-coder/src/application/discovery/ports.rs`
- Create: `crates/unica-coder/src/application/discovery/evidence_graph.rs`
- Create: `crates/unica-coder/src/application/discovery/proposal_validator.rs`
- Create: `crates/unica-coder/src/application/discovery/use_case.rs`
- Modify: `crates/unica-coder/src/application/discovery/mod.rs`
- Create: `crates/unica-coder/src/domain/source_snapshot.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: these modules with fake ports

- [ ] **Step 1: Write failing fake-port behavior tests**

Test all `ProviderOutcome` states and prove these distinctions:

- empty complete batch can contradict an exact proposal;
- bounded/unavailable batch produces `unknown`, never `contradicted`;
- lexical evidence alone never yields an actionable candidate;
- a connected target plus known support is actionable;
- a material blocker survives unrelated provider success;
- non-material optional degradation yields `partial` and may keep receipt
  eligibility;
- conflicting facts block a receipt;
- no actionable result is `insufficient`, not an operation error.

```rust
#[test]
fn unavailable_definition_is_unknown_not_contradicted() {
    let report = fixture()
        .definitions(ProviderOutcome::unavailable("index_building", true))
        .validate(method_proposal())
        .unwrap();
    assert_eq!(report.proposal_verdicts[0].verdict, Verdict::Unknown);
    assert!(!report.receipt_eligibility.eligible);
}
```

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery::use_case -- --nocapture`

- [ ] **Step 3: Define exactly six named evidence ports**

Add `MetadataCatalogPort`, `CodeSearchPort`, `DefinitionPort`, `CallGraphPort`,
`FormInspectionPort`, and `SupportStatePort`. Each returns typed
`ProviderBatch<T>` or an explicit unavailable/failed/contract-violation state.
Every fact carries canonical identity, `SourceLocation`, provider name/version,
coverage, source fingerprint, and workspace epoch. Only contract violations are
fatal `Err`.

Keep non-evidence orchestration dependencies explicit and separate:
`ProjectSourceResolverPort`, `SourceSnapshotPort`, and `ReceiptIssuerPort`.
Define typed `ResolvedSourceSet`, `SourceSnapshot`, and
`DiscoveryExecutionContext` in the domain/application boundary now so this
slice compiles before concrete filesystem adapters exist. Fake implementations
prove that application code imports no infrastructure module.

- [ ] **Step 4: Implement evidence graph and proposal validator**

Retain conflicting facts. Build only typed `contains`, `defines`, `calls`,
`handles`, `subscribes`, and `uses` edges. Compute evidence levels from facts,
not ordering. Attach checks per affected candidate/proposal and apply the
specified status precedence.

- [ ] **Step 5: Implement use-case orchestration with injected providers**

`DiscoverExtensionPointsUseCase::execute` selects the analysis source-set,
normalizes the query plan, obtains one source snapshot, invokes ports, builds
the graph, validates proposals, and returns a typed report. It does not issue a
persisted receipt yet; inject a `NoopReceiptIssuer` returning an explicit
eligibility blocker until the receipt-store task.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery -- --nocapture`

```bash
git add crates/unica-coder/src/application/discovery
git -c commit.gpgsign=false commit -m "feat: реализовать evidence graph и validation"
```

#### Controller-resolved decisions

- Synchronize the accepted spec and this plan in the same task where the
  implementation exposes a previously ambiguous contract.
- `Check.affects` uses `proposal:<proposalId>` and
  `candidate:<canonicalRef>`; both must resolve to an item in the report.
- In validate mode, any selected proposal with a material `unknown` makes the
  report `insufficient`. `partial` is reserved for conclusive results with
  only non-material optional degradation.
- A complete empty batch is negative proof only for its exact typed query.
  Bounded, unavailable, and failed outcomes are inconclusive and can never be
  converted to contradiction.
- Preserve mechanism semantics in typed facts. Add explicit binding details
  and call-resolution/context data now when required so later Platform XML,
  BSL, and CFE validators never infer event, action/call type, HTTP
  verb/template, scheduled-job enabled state, or execution context from names.
- `SourceSnapshot` must be content/fingerprint based, reuse the single domain
  `SourceFormat`, and model one analysis snapshot plus sorted/deduplicated
  mutation snapshots for future multi-grant receipts. Filesystem capture is a
  later task and remains behind fakes here.
- `ReceiptIssuerPort` is non-persistent in this task. The default no-op issuer
  adds a stable explicit blocker; tests may inject a deterministic fake to
  prove otherwise-eligible conclusions.
- No discovery application module may import infrastructure, parse display
  output, open SQLite, scan the filesystem, register the public MCP tool, or
  persist a receipt.
- Fix the Task 3 commit command to include domain/spec/plan files actually
  changed.

#### Required report

Write `.superpowers/sdd/task-3-report.md` with: RED command and expected
failure; files changed; design decisions; focused/full/fmt/clippy/diff-check
commands and results; commit hash; concerns. Return only DONE,
DONE_WITH_CONCERNS, NEEDS_CONTEXT, or BLOCKED plus a one-line summary.
