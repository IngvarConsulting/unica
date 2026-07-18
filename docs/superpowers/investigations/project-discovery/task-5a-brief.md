# Task 5A: application discovery contract and orchestration

Base: `20f6afa7a09430614babebc0cdeebeb94c8a0189`.

This is the first accepted slice of Task 5. Read, and treat as mandatory,
`.superpowers/sdd/task-5-brief.md` plus
`.superpowers/sdd/task-5-preflight.md`. Fix the application model before any
Task 5 XML adapter is implemented. Do not add public MCP registration and do
not implement the ParentConfigurations parser in this slice.

## Non-negotiable decisions

1. Replace the old `BindingDetails::ExchangePlan`/`handles` shortcut, do not
   retain it alongside the replacement. The only declarative exchange binding
   is `SubscriptionSource`, relation `uses`, from `ExchangePlan` to an exact
   `EventSubscription`. Preserve the old stable-tag position for the
   replacement so deterministic ordering stays closed; change the canonical
   semantic spelling/digest input so old and new facts cannot collide.
2. An ExchangePlan with only a `uses` edge is observed but not runtime
   connected. It becomes connected only through the complete exact chain
   `ExchangePlan --uses--> EventSubscription --subscribes--> CommonModule
   method`. Direct exchange callbacks are unsupported v1.
3. Binding facts must be smart-constructed and endpoint-validated. A public
   enum payload that callers can combine with arbitrary subject/object/relation
   is not acceptable. Validate owner identity, artifact/module kinds, relation,
   and bounded semantic fields. Blank, control-containing, and oversized fields
   are rejected. Form call type is the shared typed
   `Absent|Before|After|Override` value and participates in deterministic
   encoding. Do not create a second spelling registry.
4. Introduce `ArtifactOwnershipChain` and preserve all exact identities:
   specialized root owner, registered form when present, module, method. Cover
   CommonModule, ordinary ObjectModule, CommonCommand CommandModule,
   ExchangePlan ObjectModule, Report Form/FormModule, and DataProcessor
   Form/FormModule.
5. Existence/materiality is artifact-kind-specific:
   - methods require exact Definition plus all registered owners in their chain;
   - declarative targets require exact MetadataPresent and never DefinitionPort;
   - a form command additionally keeps FormInspection material for its exact
     runtime binding.
   Relevant conflicts/evidence include the whole ownership chain.
6. Replace raw `ProviderFact::Support { state: SupportState }` with a closed raw
   `SupportFactState`. Raw evidence may contain only source facts such as
   Editable, Locked, ConfigurationReadOnly, Removed,
   BaseWithoutParentConfigurations, and
   ExtensionWithoutParentConfigurations. `ExtensionRequired` and
   `ExtensionOwned` are proposal-policy projections and must never be raw facts.
   Candidate projection uses direct-mutation semantics and never reports
   `extension_required`; proposal projection includes the exact mutation intent
   and source/destination identity. Two proposals over one raw support fact must
   not conflict merely because their intents differ.
7. Support collection is staged. Collect the five non-support ports first;
   derive the exact canonical preliminary connected/explicit-proposal ownership
   subjects; call SupportStatePort once with that sorted/deduplicated bounded
   subject set; then rebuild/finalize. The application use case orchestrates
   this. Do not have adapters call each other.
8. `PlatformCallback` is not an unconditional runtime edge. Join it with the
   exact `DefinitionPresent` by specialized owner/module/method. Compatibility
   checks procedure/function, required export, arity, per-parameter by-value and
   default flags; parameter names are deliberately ignored. Module/owner kinds
   must match. Add a typed runtime rejection so an incompatible complete
   definition produces exact `callback_signature_mismatch`, runtime `No`, and a
   contradicted exact proposal. It is not conflicting evidence and must not be
   collapsed to generic no-match. With incomplete definition coverage it stays
   unknown.
9. `ScriptVariant` is typed `Russian|English|Unknown`, not a platform version.
   This slice provides the type and callback compatibility model; concrete
   registry rows come in Task 5B. `Unknown` must remain representable and cannot
   create a guessed runtime edge.
10. Freshness split is exact:
    - query source-set != captured analysis source-set: retryable unavailable
      `source_set_mismatch`, before any provider read;
    - live verified-read fingerprint/identity change: retryable unavailable
      `source_fingerprint_mismatch`, promote zero prefix records;
    - a provider manufactures record freshness different from its supplied
      snapshot: fatal provider contract violation;
    - registered semantic material absent from the manifest: deterministic
      failed `registered_material_missing`, except typed optional tombstones.

## RED -> GREEN sequence

1. Add failing product-contract and Rust tests that reject the old exchange
   binding and require SubscriptionSource, typed FormCallType/ScriptVariant,
   validated bindings, raw support facts, and ownership chains. Record actual
   RED output before implementation.
2. Implement application primitives, stable encodings, constructor validation,
   and determinism. Re-run focused application tests.
3. Add failing graph/validator/use-case tests for full exchange reachability,
   callback gating/mismatch, kind-specific materiality, exact ownership
   evidence, intent-specific support projection, staged support subjects, and
   zero-I/O source mismatch.
4. Implement graph, validator, ports, and staged use-case behavior. Remove every
   legacy shortcut; do not leave dead variants or permissive fallback branches.
5. Update the active spec, historical plan, and product contract in the same
   commit so they reject `ExchangePlan/handles` and describe the accepted
   application semantics. Do not add unconfirmed concrete callback rows.

## Mandatory tests

- old `ExchangePlan/handles` cannot be constructed/deserialized; only
  `SubscriptionSource/uses` is accepted and deterministic;
- every binding kind rejects wrong endpoint kinds/relations/cross-owner shapes
  and invalid bounded payloads;
- Form callType Absent/Before/After/Override changes evidence digest;
- ownership chains preserve all six required specialized shapes;
- declarative existence does not consult DefinitionPort; method existence does;
- form-command materiality includes FormInspection;
- ExchangePlan uses-only is not connected; exact two-edge chain is connected;
- callback requires a compatible definition; each shape mismatch is the exact
  runtime rejection; parameter rename remains compatible;
- raw support fact shared by two mutation intents creates no graph conflict and
  projects intent-specific public policy;
- staged support port sees exactly sorted/deduplicated ownership subjects and is
  invoked once; non-support failure does not leak partial support evidence;
- source-set mismatch performs zero reads and returns the typed retryable
  unavailable outcome;
- determinism/collision/fail-closed tests remain exhaustive for all new closed
  enums and tags.

## Verification and delivery

Run at minimum:

```text
cargo test --locked -p unica-coder application::discovery -- --nocapture
cargo test --locked -p unica-coder
cargo fmt --all -- --check
cargo clippy --locked -p unica-coder --all-targets -- -D warnings
python3 tests/ci/test_product_contracts.py
git diff --check
```

Write `.superpowers/sdd/task-5a-report.md` with the exact RED failures, GREEN
commands/results, decisions, remaining Task 5B/5C boundaries, and risks. Commit
all Task 5A tracked changes as one focused commit:

`feat: исправить application contract discovery`

Do not stage unrelated files. Stop and report instead of weakening a test or
retaining a contradictory compatibility path.

## Controller corrections after primary-source Task 5B/5C design

The later authoritative design reports
`.superpowers/sdd/task-5b-contract.md` and
`.superpowers/sdd/task-5c-support-design.md` refine this brief as follows. These
rules supersede any earlier conflicting shorthand.

- Semantic form call type is `Direct|Before|After|Override`. A missing XML
  attribute maps to `Direct`; it is not stable/public `Absent`.
- Split script-variant observation from the known registry:
  `Missing|Known(Russian|English)|Unknown(exact bounded token)`. Only a known
  variant can select a callback row. Do not erase the exact unknown token.
- Do not reuse the public CFE mutation `ExecutionContext` for discovery facts.
  Add an internal stable `BslExecutionContext` with ModuleDefault, AtServer,
  AtClient, AtServerNoContext, AtClientAtServer, and
  AtClientAtServerNoContext. DefinitionShape, Call facts, callback requirements,
  and declarative binding contexts use it. The public MutationIntent enum stays
  unchanged; no BSL directive is coerced to Server.
- Subscription source is closed and prefix-independent:
  `CurrentConfiguration + ExchangePlanObject + canonical object name`. Its
  constructor proves equality to the exact ExchangePlan subject and an
  EventSubscription object. No lexical prefix or generic URI/string pair is a
  semantic identity.
- Event/action identifiers are `1..=512` UTF-8 bytes, at most 128 Unicode
  scalars, letters/digits/underscore only. Qualified handler and lexical QName
  are at most 1024 bytes total and each segment has the same 512-byte/128-scalar
  ceiling. Closed XML tokens are pre-capped at 256 bytes.
- RootURL and Template are each at most 2048 bytes and the combined route at
  most 4096. RootURL has no leading/trailing slash; Template begins with exactly
  one slash. Reject backslash, query/fragment markers, controls, dot segments,
  and repeated/empty internal segments. Exact `/` and one meaningful terminal
  Template slash are supported. Never collapse repeated slash.
- Official v1 callback evidence proves exact method/callable kind/owner/module/
  required arity/context mismatches. Those are
  `callback_signature_mismatch`. It does not prove `Val`, default, extra-
  optional, or cross-language alias variations invalid; those remain scoped
  `unsupported_callback_signature_variant` or
  `unsupported_callback_alias_variant`, hence runtime Unknown. Export policy is
  typed `Required|NotRequired`; all four v1 callback rows are NotRequired and
  accept either actual export spelling.
- A query source-set mismatch is retryable unavailable `source_set_mismatch`
  before I/O. The Task 5B report's one contrary table row is rejected; only a
  provider-manufactured record freshness mismatch is a fatal contract error.
- Replace bounded partial-success `reason_code` with canonically sorted typed
  scoped `ProviderGap[]` containing a reason,
  `ProviderGapScope::Artifacts(nonempty exact set)|QueryWide`, and optional
  location. Artifact scope is used when exact subjects are known; QueryWide is
  required for root/module/resource/result gaps before a bounded ArtifactRef is
  representable and degrades every material consumer of that port. Multiple
  independent gaps must survive. Enforce
  per-port and global `maxEvidence` canonically; truncation produces a scoped
  material gap and can never select a filesystem/provider-order prefix.
- Raw support facts are exactly seven variants: Editable, Locked,
  ConfigurationReadOnly, Removed, ObjectNotListed,
  BaseWithoutParentConfigurations, ExtensionWithoutParentConfigurations.
  Facts are keyed by exact source-set plus subject; different source sets do not
  conflict. Candidate direct projection is respectively editable, locked,
  configuration_read_only, removed, not_under_support, not_under_support, and
  extension_owned. Candidate never reports extension_required.
- A CFE proposal requires a known analysis fact and an exact Extension
  destination fact. A safely writable destination (Extension missing,
  ObjectNotListed, Editable, or Removed) projects to extension_owned only when
  the target already belongs to that same extension; otherwise it projects to
  extension_required. Destination Locked/ConfigurationReadOnly blocks. Any
  malformed, unavailable, ambiguous, or missing analysis/destination state is
  public unknown and ineligible.
- No present-file explicit-not-under-support raw state exists in v1. The
  repository has no real fixture proving that byte layout. `len <= 32` and
  zero-vendor heuristics are forbidden and remain Task 5C failed/unknown.
