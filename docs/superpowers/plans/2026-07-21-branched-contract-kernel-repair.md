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
- Modify: the MCP JSON-RPC line parser selected by code search
- Test: the existing MCP protocol test module

- [ ] Add failing protocol tests for duplicate top-level, `params`, `arguments`,
  and nested request members. Assert JSON-RPC `-32700` and zero handler calls.
- [ ] Implement a recursive serde visitor that builds `serde_json::Value` while
  rejecting a duplicate key in every object scope.
- [ ] Replace only the protocol parse boundary; preserve ordinary request/error
  behavior and existing I-JSON number/string rejection.
- [ ] Add a regression proving identical keys in different object scopes remain
  valid.
- [ ] Run MCP protocol tests and the full interface test group.
- [ ] Commit and obtain independent review.

### Task 4: Bind replay to durable policy and computed input

**Files:**
- Modify: `crates/unica-coder/src/domain/branched_development/operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/operation.rs`

- [ ] Add failing compile/runtime coverage showing a replay view cannot contain
  `readOnly` and all replay decisions compare the centrally computed digest.
- [ ] Change `OperationReplayView` to `DurableExecutionPolicy` and expose no
  constructor that accepts the broad `ExecutionPolicy`.
- [ ] Replace arbitrary test digests for canonical input with Task 2's producer;
  retain distinct validated digests only for terminal/recovery envelopes.
- [ ] Preserve the four-state exhaustive classifier and all illegal-field tests.
- [ ] Run the full domain suite, formatting, boundary check, and diff checks.
- [ ] Commit and obtain independent review.

### Task 5: Fail closed on corrupt durable policy

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/durable_operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/durable_operation.rs`

- [ ] Define the versioned raw stored-operation projection required to validate
  schema digest, tool, durable policy, canonical input, four-state fields, owner,
  lease/heartbeat references, terminal envelope, and recovery digest.
- [ ] Add failing tests for a byte-for-byte `policy:"readOnly"` record and each
  illegal state/presence combination. Assert deterministic `stateCorrupt` data
  with expected/observed schema digest and retained source bytes.
- [ ] Introduce a pure loader result that cannot yield a replay view until schema,
  policy, identifiers, digests, and state invariants all validate.
- [ ] Use spy ports in the next storage task to prove corrupt input causes zero
  CAS/worker/receipt/effect calls; this pure task must itself have no such ports.
- [ ] Commit and obtain independent review.

### Task 6: Complete active-operation status projection

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/status.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/status.rs`

- [ ] Add failing tests for the exact active-operation fields, durable policy,
  four operation states, owner/live-or-orphaned rules, nullable terminal data,
  recovery requirement, and absence of an `observed` state.
- [ ] Project only from the validated durable-operation type; do not deserialize a
  second weaker status model from disk.
- [ ] Make illegal field combinations unrepresentable with closed variants, then
  serialize to the public tagged/optional-field contract.
- [ ] Run all branched domain tests, full workspace tests, formatting, platform
  boundary, and diff checks.
- [ ] Commit and obtain independent review.

## Completion evidence

- Every task has a recorded RED command/output, GREEN command/output, commit, and
  independent review in `.superpowers/sdd/progress.md`.
- `cargo test -p unica-coder domain::branched_development` passes.
- MCP duplicate-key protocol tests pass.
- `cargo fmt --all -- --check`, platform boundary check, `git diff --check`, and
  `cargo test --workspace -- --test-threads=1` pass on the final kernel commit.
- No public branched handler is registered by this plan.
