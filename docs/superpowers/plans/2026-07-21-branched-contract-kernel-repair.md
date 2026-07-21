# Branched Contract Kernel Repair Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` and execute one task at a time. Every
> behavior starts with an observed failing test, followed by the smallest
> implementation, focused verification, commit, and independent review.

**Goal:** Close the post-approval semantic gaps in the pure operation kernel
before durable storage or any public branched handler is implemented.

**Architecture:** Keep policy and canonical-input invariants in
`domain::branched_development`; reject duplicate JSON names at the MCP parsing
boundary; make durable records deserialize through strict closed types; project
status only from validated records. No filesystem, worker, lease, receipt, or
platform effect is allowed in this plan.

**Tech Stack:** Rust 2021, `serde`, `serde_json`, `sha2`, RFC 8785 canonical JSON
via `serde_json_canonicalizer`, existing MCP protocol tests.

## Global constraints

- `ExecutionPolicy` remains the complete six-value public policy vocabulary.
- `DurableExecutionPolicy` is a separate five-value type and cannot deserialize
  `readOnly`; conversion from `ExecutionPolicy::ReadOnly` is a typed error.
- Canonical input is the domain-separated record
  `{digestKind:"branchedOperationInputV1",toolName,executionPolicy,request}`.
- Only the top-level `operationId` is removed from the request. Nested fields and
  every approval/guard digest remain bound.
- JSON duplicate member names fail as JSON-RPC parse errors before schema,
  hashing, task lookup, CAS, lease, worker, receipt, or effect.
- Stored/legacy durable `policy:"readOnly"` fails closed as `stateCorrupt`; its
  bytes are retained and no migration coerces or deletes it.
- Operation states stay exactly four. `observed` is not added.

### Task 1: Separate durable execution policy

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`

- [x] Add failing tests proving the exact five-value serialized list, rejection
  of `"readOnly"`, successful conversion of the five durable policies, and a
  typed rejection for `ExecutionPolicy::ReadOnly`.
- [x] Add `DurableExecutionPolicy` and `NonDurableExecutionPolicyError` with
  `Display`/`Error`, `ALL`, `as_str`, and explicit `TryFrom<ExecutionPolicy>`.
- [x] Export the new types and run focused tests, formatting, and diff checks.
- [x] Commit and obtain independent review.

### Task 2: RFC 8785 operation-input digest

**Files:**
- Modify: `Cargo.toml`
- Modify: `crates/unica-coder/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `crates/unica-coder/src/domain/branched_development/canonical_json.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/canonical_json.rs`

- [x] Add failing RFC vectors for `[]`, reordered `{"b":2,"a":1}`, and
  `{"a":"€","z":null}`, including exact canonical bytes and contract hashes.
- [x] Add failing operation-input tests proving key-order independence,
  tool/policy domain separation, removal of only top-level `operationId`, and
  retention of nested `operationId` plus all other fields.
- [x] Add pinned workspace dependency `serde_json_canonicalizer = "0.3.2"`.
- [x] Implement a private-literal digest record and a typed canonicalization/hash
  error; accept only a JSON object request and return validated `Sha256Digest`.
- [x] Run focused tests, dependency/license audit, formatting, and diff checks.
- [x] Commit and obtain independent review.

### Task 3: Reject duplicate JSON names before hashing

**Files:**
- Create: `crates/unica-coder/src/interfaces/strict_json.rs`
- Modify: `crates/unica-coder/src/interfaces/mod.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/canonical_json.rs`
  or a lower shared number-policy module if code search proves that cleaner
- Modify: the MCP JSON-RPC line parser selected by code search
- Test: the existing MCP protocol test module

- [x] Add failing protocol tests for duplicate top-level, `params`, `arguments`,
  nested request members, and escape-equivalent names. Assert JSON-RPC `-32700`
  and zero handler calls.
- [x] Implement a recursive serde visitor that builds `serde_json::Value` while
  rejecting a duplicate key in every object scope.
- [x] Add raw integer/decimal/exponent tests at and beyond `2^53-1`; reject the
  non-interoperable forms as `-32700` before dispatch and keep safe boundaries,
  finite fractions, and ordinary request/error behavior unchanged. Ordinary
  `serde_json::Value` parsing did not provide this guard, so share or exactly
  align the parser rule with Task 2 instead of claiming it already existed.
- [x] Replace only the public MCP protocol parse boundary. Preserve serde's
  existing invalid UTF-8, lone-surrogate, non-finite, depth, and trailing-data
  rejection and the existing 8 MiB line bound.
- [x] Add a regression proving identical keys in different object scopes remain
  valid.
- [x] Run MCP protocol tests and the full interface test group.
- [x] Commit and obtain independent review.

### Task 4: Bind replay to durable policy and computed input

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/canonical_json.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/operation.rs`

- [x] Add failing compile/runtime coverage showing a replay view cannot contain
  `readOnly`, no classifier accepts an arbitrary observed SHA, and incoming
  request/tool/policy are all bound before any state-specific decision.
- [x] Change `OperationReplayView` to concrete `BranchedLifecycleToolName` plus
  `DurableExecutionPolicy` for this milestone. Do not retain the unconstrained
  `TTool` generic: Phase 1 replaces the concrete type with the generated closed
  `TaskOperationToolName` union before handlers, never a `String` or open trait.
- [x] Replace the public raw-parts constructor with a sibling-only
  `from_validated_record_parts` that accepts the stored canonical digest for
  Task 5's validated loader but cannot be called as a public request shortcut.
- [x] Make replay classification accept exact incoming tool, durable policy, and
  request; compute the observed digest internally. Explicitly compare incoming
  tool/policy to stored fields as well as the digest. Return the computed digest
  in `DispatchNew` so registration cannot recompute it differently.
- [x] Replace arbitrary test digests for canonical input with Task 2's producer;
  retain distinct validated digests only for terminal/recovery envelopes. Add
  invalid-I-JSON and different-tool/different-policy precedence regressions.
- [x] Preserve the four-state exhaustive classifier and all illegal-field tests.
- [x] Run the full domain suite, formatting, boundary check, and diff checks.
- [x] Commit and obtain independent review.

### Task 5A: Exact-byte durable-policy poison preflight

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/operation_preflight.rs`
- Modify: `crates/unica-coder/src/domain/i_json.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Modify: `crates/unica-coder/src/interfaces/{mod.rs,mcp.rs}`
- Remove: `crates/unica-coder/src/interfaces/strict_json.rs` after moving its
  neutral parser into `domain::i_json`
- Test: `crates/unica-coder/src/domain/branched_development/operation_preflight.rs`

The original Task 5 was premature: the complete record depends on Phase 1's
generated `TaskOperationToolName`, `UnicaId`, normalized UTC, typed result/error
envelope, lease, and storage schemas. The normative record has no embedded
schema-version field, so this plan must not invent one or fabricate an expected
schema digest. Full validation is Task 5B in Phase 2.

- [ ] Add failing tests proving exact source bytes are retained and hashed before
  parsing; whitespace/key-order changes produce different observed digests.
- [ ] Move/share the duplicate-aware strict I-JSON parser under neutral
  `domain::i_json`; keep MCP behavior byte-for-byte compatible.
- [ ] Introduce a closed exact-byte preflight result: strict-JSON failure,
  non-object failure, forbidden top-level literal `policy:"readOnly"`, or an
  opaque candidate. Every result owns the exact bytes and observed byte digest.
- [ ] Prove escaped top-level `readOnly` is forbidden, while nested `readOnly` is
  not mistaken for the discriminator; duplicate/escape-equivalent policy names,
  invalid UTF-8/trailing data/noncharacters/non-I-JSON numbers fail strictly.
- [ ] Prove a five-value durable policy yields only the opaque candidate and
  cannot construct or classify an `OperationReplayView`.
- [ ] Expose no expected schema digest, migration coercion, deletion, replay,
  status, CAS, lease, worker, receipt, or effect API in this task.
- [ ] Commit and obtain independent review.

### Task 5B and Task 6: Deferred typed loader and active-operation status

Execute only after Phase 1 defines and snapshots the complete tool union,
operation record, lease, result envelope, stable errors, and status schemas.
Task 5B then maps strict/preflight/schema failures to `stateCorrupt` with
`expectedDigest` from the committed current schema and `observedDigest` from
the exact bytes, retains those bytes, validates the full presence/digest matrix,
and proves zero CAS/lease/worker/receipt/effect calls. Task 6 projects only from
that validated record plus validated lease/liveness evidence. Per the normative
contract, `ActiveOperationStatus.state` is only `registered`, `intentWritten`,
or `effectUnknown`; terminal records belong to recent/terminal result evidence.

## Completion evidence

- Every executed Phase 0 task has a recorded RED command/output, GREEN
  command/output, commit, and
  independent review in `.superpowers/sdd/progress.md`.
- `cargo test -p unica-coder domain::branched_development` passes.
- MCP duplicate-key protocol tests pass.
- `cargo fmt --all -- --check`, platform boundary check, `git diff --check`, and
  `cargo test --workspace -- --test-threads=1` pass on the final kernel commit.
- No public branched handler is registered by this plan. Task 5B/6 completion
  evidence is recorded in their later Phase 2/3 plans, not claimed here.
