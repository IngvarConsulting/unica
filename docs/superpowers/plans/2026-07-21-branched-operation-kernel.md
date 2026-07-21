# Branched Operation Kernel Implementation Plan

> **Historical plan:** Tasks 1-3 were completed and rebased onto specification
> revision `1531d36`. The post-approval contract audit found additional durable
> policy, canonical-input, and corrupt-record requirements. Further work follows
> `2026-07-21-branched-contract-kernel-repair.md` and
> `2026-07-21-branched-development-roadmap.md`; this document is retained as the
> reviewed history of the first kernel increment.

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the pure, transport-independent identifier, lifecycle-vocabulary, and operation-replay kernel required by ADR-0012 before any branched-development MCP handler is registered.

**Architecture:** Add a focused `domain::branched_development` module with no filesystem, process, MCP, cache, or Designer dependency. The module validates public identifiers at construction/deserialization, exposes the exact closed lifecycle vocabulary, and classifies operation replay from an invariant-checked projection of the durable record. Persistence, schema generation, and handlers are separate later plans; this plan deliberately creates no public tool.

**Tech Stack:** Rust 2021, existing `serde`, `serde_json`, and `uuid` dependencies, in-module unit tests.

## Global Constraints

- Do not register any of the 21 branched-development handlers until their Rust request/result types, generated strict JSON Schemas, committed snapshots, and contract tests exist.
- `TaskId` is 1-64 ASCII characters matching `[A-Za-z0-9][A-Za-z0-9._-]{0,63}`.
- `OperationId` is a canonical lowercase hyphenated UUID string.
- `Sha256Digest` is exactly 64 lowercase hexadecimal ASCII characters.
- Operation states are exactly `registered`, `intentWritten`, `effectUnknown`, and `terminal`; do not invent an `observed` state.
- Replay compares canonical-input digests before phase or recovery gates: a mismatch is always rejected, a terminal match replays, a live owner reports in-progress, an orphaned no-intent registration may resume, and unknown effect requires recovery.
- This kernel must not use `WorkspaceStateRepository`, `.build/unica`, `UNICA_CACHE_DIR`, or `RuntimeJobOperation`; branched operational state is a separate later bounded-context repository.
- Do not expose raw command material, executable paths, credentials, or task filesystem paths.

---

### Task 1: Validated identifiers and digests

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Create: `crates/unica-coder/src/domain/branched_development/identifiers.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/identifiers.rs`

**Interfaces:**
- Consumes: `serde::{Serialize, Deserialize}` and `uuid::Uuid`.
- Produces: `TaskId`, `OperationId`, `Sha256Digest`, and `IdentifierError`, each with `parse`, `Display`, `FromStr`, transparent JSON serialization, and validating deserialization.

- [ ] **Step 1: Add module exports and failing identifier tests**

Add to `domain/mod.rs`:

```rust
pub mod branched_development;
```

Create `domain/branched_development/mod.rs`:

```rust
mod identifiers;

pub use identifiers::{IdentifierError, OperationId, Sha256Digest, TaskId};
```

Create `identifiers.rs` with only this test module first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn task_id_accepts_only_the_bounded_ascii_contract() {
        assert_eq!(TaskId::from_str("TASK-142").unwrap().as_str(), "TASK-142");
        assert!(TaskId::from_str("a").is_ok());
        assert!(TaskId::from_str(&format!("A{}", "_".repeat(63))).is_ok());

        for invalid in ["", ".task", "task/name", "задача"] {
            assert!(TaskId::from_str(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(TaskId::from_str(&format!("A{}", "_".repeat(64))).is_err());
    }

    #[test]
    fn operation_id_requires_canonical_lowercase_hyphenated_uuid() {
        let canonical = "123e4567-e89b-12d3-a456-426614174000";
        assert_eq!(OperationId::from_str(canonical).unwrap().as_str(), canonical);

        for invalid in [
            "123E4567-E89B-12D3-A456-426614174000",
            "123e4567e89b12d3a456426614174000",
            "{123e4567-e89b-12d3-a456-426614174000}",
            "not-a-uuid",
        ] {
            assert!(OperationId::from_str(invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn sha256_digest_requires_exact_lowercase_hex() {
        let canonical = "0123456789abcdef".repeat(4);
        assert_eq!(Sha256Digest::from_str(&canonical).unwrap().as_str(), canonical);

        for invalid in [
            "0".repeat(63),
            "0".repeat(65),
            "G".repeat(64),
            "A".repeat(64),
        ] {
            assert!(Sha256Digest::from_str(&invalid).is_err(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn identifier_json_is_transparent_and_deserialization_revalidates() {
        let task = TaskId::from_str("TASK-142").unwrap();
        assert_eq!(serde_json::to_string(&task).unwrap(), "\"TASK-142\"");
        assert_eq!(serde_json::from_str::<TaskId>("\"TASK-142\"").unwrap(), task);
        assert!(serde_json::from_str::<TaskId>("\"../task\"").is_err());
        assert!(serde_json::from_str::<OperationId>("\"NOT-A-UUID\"").is_err());
    }
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder domain::branched_development::identifiers::tests
```

Expected: compile errors because the identifier types do not exist.

- [ ] **Step 3: Implement the validated transparent value objects**

Above the tests, implement one private validated-string helper and the three public wrappers. Use this exact public surface:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentifierError {
    kind: &'static str,
    reason: &'static str,
}

impl std::fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for IdentifierError {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct TaskId(String);

impl TaskId {
    pub fn parse(value: &str) -> Result<Self, IdentifierError>;
    pub fn as_str(&self) -> &str;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct OperationId(String);

impl OperationId {
    pub fn parse(value: &str) -> Result<Self, IdentifierError>;
    pub fn as_str(&self) -> &str;
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    pub fn parse(value: &str) -> Result<Self, IdentifierError>;
    pub fn as_str(&self) -> &str;
}
```

For all three wrappers:

- implement `Display` as the inner canonical string;
- implement `FromStr<Err = IdentifierError>` by calling `parse`;
- implement `Deserialize` with a string visitor that calls `parse` so JSON cannot bypass validation;
- never normalize invalid input silently.

Validation is exact:

```rust
fn valid_task_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b'-')
        })
}

fn valid_operation_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value)
        .map(|parsed| parsed.hyphenated().to_string() == value)
        .unwrap_or(false)
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
```

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test -p unica-coder domain::branched_development::identifiers::tests
cargo fmt --all -- --check
```

Expected: 4 identifier tests pass and formatting is clean.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/domain/branched_development/mod.rs crates/unica-coder/src/domain/branched_development/identifiers.rs
git commit -m "feat: add branched workflow identifiers"
```

### Task 2: Exact lifecycle vocabulary

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/vocabulary.rs`

**Interfaces:**
- Consumes: no Task 1 types.
- Produces: `TaskPhase`, `ExecutionPolicy`, and `BranchedLifecycleToolName`, each serde-serializable/deserializable with an `ALL` constant and `as_str()`.

- [ ] **Step 1: Write failing exact-enum tests**

Register `mod vocabulary;` and re-export its three types. Add tests that serialize `Type::ALL` and compare to these exact arrays:

```rust
const EXPECTED_PHASES: &[&str] = &[
    "created", "preflightPassed", "baselineReady", "developing", "localVerified",
    "synchronizationPrepared", "synchronizationConflicts", "synchronized",
    "integrationPlanned", "acquiringLocks", "locked", "mainMerged", "mainValidated",
    "committing", "committedAndUnlocked", "archivedSuccess", "cleanedSuccess",
    "blockedByForeignLock", "staleRelevantBaseline", "lockPlanExpansionRequired",
    "staleSupportPreflight", "unexpectedDelta", "validationFailed", "commitBlocked",
    "recoveryRequired", "committedUnverified", "abandonmentReady",
    "archivedAbandoned", "cleanedAbandoned",
];

const EXPECTED_POLICIES: &[&str] = &[
    "readOnly", "localJournaled", "contained", "preparedJournaledEffect",
    "journaledEffect", "previewedJournaledEffect",
];

const EXPECTED_TOOLS: &[&str] = &[
    "unica.branched.start", "unica.branched.status", "unica.branched.archive",
    "unica.branched.cleanup", "unica.delivery.inspect", "unica.delivery.create",
    "unica.delivery.verify", "unica.delivery.deploy", "unica.merge.compare",
    "unica.merge.prepare", "unica.merge.conflicts", "unica.merge.resolve",
    "unica.merge.apply", "unica.merge.verify", "unica.repository.status",
    "unica.repository.update", "unica.repository.planLocks", "unica.repository.lock",
    "unica.repository.unlock", "unica.repository.commit", "unica.repository.recover",
];
```

For each enum, assert:

```rust
let actual = Type::ALL.iter().map(Type::as_str).collect::<Vec<_>>();
assert_eq!(actual, EXPECTED_VALUES);
for value in EXPECTED_VALUES {
    let encoded = format!("\"{value}\"");
    let parsed: Type = serde_json::from_str(&encoded).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), encoded);
}
assert!(serde_json::from_str::<Type>("\"unknown\"").is_err());
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder domain::branched_development::vocabulary::tests
```

Expected: compile errors because the vocabulary types do not exist.

- [ ] **Step 3: Implement the closed enums**

Implement all variants represented by the arrays above. Derive:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
```

Use `#[serde(rename_all = "camelCase")]` for `TaskPhase` and `ExecutionPolicy`. Give every `BranchedLifecycleToolName` variant an explicit `#[serde(rename = "unica....")]`. `ALL` must contain every variant in normative contract order; `as_str()` must return the same literal as serde.

Do not add `notCreated`: it is a read/preflight status, not a persisted `TaskPhase`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test -p unica-coder domain::branched_development::vocabulary::tests
cargo fmt --all -- --check
```

Expected: 3 exact-vocabulary tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/branched_development/mod.rs crates/unica-coder/src/domain/branched_development/vocabulary.rs
git commit -m "feat: add branched lifecycle vocabulary"
```

### Task 3: Operation replay classifier

**Files:**
- Create: `crates/unica-coder/src/domain/branched_development/operation.rs`
- Modify: `crates/unica-coder/src/domain/branched_development/mod.rs`
- Test: `crates/unica-coder/src/domain/branched_development/operation.rs`

**Interfaces:**
- Consumes: `OperationId`, `Sha256Digest`, and `ExecutionPolicy`; tests instantiate the generic tool-name parameter with `BranchedLifecycleToolName`.
- Produces: `OperationState`, `OperationOwnerState`, `OperationReplayView`, `OperationInvariantError`, `ReplayDisposition`, and `classify_replay`.

- [ ] **Step 1: Write failing replay and invariant tests**

Add tests for these exact cases using fixed digests and `unica.branched.start`:

1. `None` record returns `DispatchNew`.
2. Any recorded state with a different input digest returns `ReplayMismatch { expected, observed }` before state-specific classification.
3. Matching `terminal` returns `ReplayTerminal { terminal_envelope_digest }`.
4. Matching live `registered` or `intentWritten` returns `InProgress`.
5. Matching orphaned `registered` returns `ResumeRegistered`; matching orphaned `intentWritten` returns `ObserveIntentWritten`.
6. Matching `effectUnknown` returns `RecoveryRequired { recovery_digest }` and never a dispatch/resume disposition.
7. The constructor rejects every illegal field combination: missing owner on registered/intent-written, owner on effect-unknown/terminal, missing recovery digest on effect-unknown, recovery digest elsewhere, missing terminal digest on terminal, and terminal digest elsewhere.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p unica-coder domain::branched_development::operation::tests
```

Expected: compile errors because the replay types do not exist.

- [ ] **Step 3: Implement invariant-checked replay projection**

Use this exact shape:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OperationState {
    Registered,
    IntentWritten,
    EffectUnknown,
    Terminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationOwnerState {
    Live,
    Orphaned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationReplayView<TTool> {
    operation_id: OperationId,
    tool_name: TTool,
    policy: ExecutionPolicy,
    canonical_input_digest: Sha256Digest,
    state: OperationState,
    owner_state: Option<OperationOwnerState>,
    terminal_envelope_digest: Option<Sha256Digest>,
    recovery_digest: Option<Sha256Digest>,
}

impl<TTool> OperationReplayView<TTool> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: OperationId,
        tool_name: TTool,
        policy: ExecutionPolicy,
        canonical_input_digest: Sha256Digest,
        state: OperationState,
        owner_state: Option<OperationOwnerState>,
        terminal_envelope_digest: Option<Sha256Digest>,
        recovery_digest: Option<Sha256Digest>,
    ) -> Result<Self, OperationInvariantError>;

    pub fn operation_id(&self) -> &OperationId;
    pub fn tool_name(&self) -> &TTool;
    pub fn policy(&self) -> ExecutionPolicy;
    pub fn state(&self) -> OperationState;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayDisposition {
    DispatchNew,
    ReplayMismatch {
        expected: Sha256Digest,
        observed: Sha256Digest,
    },
    ReplayTerminal {
        terminal_envelope_digest: Sha256Digest,
    },
    InProgress,
    ResumeRegistered,
    ObserveIntentWritten,
    RecoveryRequired {
        recovery_digest: Sha256Digest,
    },
}

pub fn classify_replay<TTool>(
    record: Option<&OperationReplayView<TTool>>,
    observed_input_digest: &Sha256Digest,
) -> ReplayDisposition;
```

`OperationInvariantError` is a closed enum with one variant per illegal presence rule and a useful `Display`; it must not contain filesystem or process information. Keep all record fields private so callers cannot bypass `new`. The generic `TTool` lets the later generated `TaskOperationToolName` union include both lifecycle and compatible general tools without weakening this kernel. `classify_replay` first handles `None`, then digest mismatch, then matches the invariant-checked state/owner pair. It must contain no fallback arm.

- [ ] **Step 4: Verify GREEN and the module boundary**

Run:

```bash
cargo test -p unica-coder domain::branched_development
cargo fmt --all -- --check
cargo clippy -p unica-coder --lib --all-features -- -D warnings
python3.12 scripts/ci/check-rust-platform-boundary.py
```

Expected: all branched-domain tests pass, clippy and the platform-boundary check exit 0.

- [ ] **Step 5: Commit**

```bash
git add crates/unica-coder/src/domain/branched_development/mod.rs crates/unica-coder/src/domain/branched_development/operation.rs
git commit -m "feat: classify branched operation replay"
```

## Plan Completion Verification

Run:

```bash
cargo test --workspace -- --test-threads=1
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3.12 -m unittest discover -s tests/ci
git diff --check
```

Expected: the complete existing suite remains green, the new pure kernel has no public MCP surface, and `git diff origin/codex/issue-137-branched-development...HEAD` contains only this plan plus the new domain module and tests.
