# Writer Text Snapshot and EOL Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add one internal byte-exact UTF-8 source snapshot and EOL policy model, then use it from `unica.code.patch` without changing the public MCP contract.

**Architecture:** A focused `text_snapshot` module owns source observation, BOM/EOL classification, and typed EOL policy resolution. `code.patch` remains responsible for selector-local context, postimage construction, validation, exact-preimage publication, and result data; it delegates only source decoding and final inserted-text EOL selection.

**Tech Stack:** Rust 2021, existing `unica-coder` crate, built-in unit tests, `cargo fmt`, `cargo clippy`, repository platform-boundary checks.

## Global Constraints

- Base all work on `upstream/main` at or after `e06a56f7`.
- Keep the public MCP server and tool names unchanged; do not add a public EOL argument.
- Preserve every untouched source byte, including BOM and terminal newline.
- Implement `preserve`, `lf`, and `crlf`; model `repository` as typed fail-closed `RepositoryPolicyUnresolved`.
- Do not parse `.gitattributes`, `.editorconfig`, or Git configuration in this slice.
- Do not migrate `meta`, `form`, or any other XML writer.
- Do not introduce the shared structured mutation-result contract.
- Use exhaustive enums and borrowed arguments; do not use boolean policy parameters or lossy UTF-8 conversion.
- Follow RED → GREEN → REFACTOR for every production change.
- Reference #74 without closing it.

---

### Task 1: Exact UTF-8 Source Snapshot

**Files:**
- Create: `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations.rs`

**Interfaces:**
- Consumes: exact `&[u8]` read by a writer.
- Produces:

```rust
pub(crate) struct SourceTextSnapshot;

impl SourceTextSnapshot {
    pub(crate) fn from_bytes(raw: &[u8]) -> Result<Self, SnapshotError>;
    pub(crate) fn raw(&self) -> &[u8];
    pub(crate) fn text(&self) -> &str;
    pub(crate) fn decoded_text(&self) -> &str;
    pub(crate) fn bom(&self) -> Utf8Bom;
    pub(crate) fn line_endings(&self) -> LineEndingProfile;
    pub(crate) fn terminal_line_ending(&self) -> Option<LineEnding>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Utf8Bom {
    Present,
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineEndingProfile {
    None,
    Uniform(LineEnding),
    Mixed { lf: usize, crlf: usize, cr: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SnapshotError {
    InvalidUtf8,
    DuplicateUtf8Bom,
}
```

- [x] **Step 1: Register the new module**

Add this declaration beside the other family-owned native-operation modules:

```rust
pub(crate) mod text_snapshot;
```

- [x] **Step 2: Write failing source-observation tests**

Create `text_snapshot.rs` with test-only wished-for API and tests that assert:

```rust
#[test]
fn snapshot_preserves_raw_bytes_and_excludes_one_bom_from_text() {
    let raw = b"\xef\xbb\xbfProcedure Run()\r\nEndProcedure\r\n";
    let snapshot = SourceTextSnapshot::from_bytes(raw).unwrap();
    assert_eq!(snapshot.raw(), raw);
    assert_eq!(snapshot.text(), "Procedure Run()\r\nEndProcedure\r\n");
    assert_eq!(
        snapshot.decoded_text(),
        "\u{feff}Procedure Run()\r\nEndProcedure\r\n"
    );
    assert_eq!(snapshot.bom(), Utf8Bom::Present);
}

#[test]
fn snapshot_rejects_duplicate_bom() {
    let raw = b"\xef\xbb\xbf\xef\xbb\xbfProcedure Run()\nEndProcedure\n";
    assert_eq!(
        SourceTextSnapshot::from_bytes(raw),
        Err(SnapshotError::DuplicateUtf8Bom)
    );
}

#[test]
fn snapshot_rejects_invalid_utf8() {
    assert_eq!(
        SourceTextSnapshot::from_bytes(&[0xff, 0xfe]),
        Err(SnapshotError::InvalidUtf8)
    );
}
```

Add separate tests for:

```rust
("", LineEndingProfile::None, None)
("A\nB\n", LineEndingProfile::Uniform(LineEnding::Lf), Some(LineEnding::Lf))
("A\r\nB\r\n", LineEndingProfile::Uniform(LineEnding::CrLf), Some(LineEnding::CrLf))
("A\rB\r", LineEndingProfile::Uniform(LineEnding::Cr), Some(LineEnding::Cr))
(
    "A\r\nB\nC\r",
    LineEndingProfile::Mixed { lf: 1, crlf: 1, cr: 1 },
    Some(LineEnding::Cr),
)
("A\nB", LineEndingProfile::Uniform(LineEnding::Lf), None)
```

The production API is intentionally missing, so compilation must fail because
`SourceTextSnapshot`, its enums, and its methods do not exist.

- [x] **Step 3: Run the focused test and verify RED**

Run:

```bash
cargo test -p unica-coder text_snapshot -- --test-threads=1
```

Expected: compilation fails on missing `SourceTextSnapshot`/enum definitions.
Confirm the failure is caused by the missing feature, not module wiring or a
test typo.

- [x] **Step 4: Implement minimal snapshot types and classification**

Implement `SourceTextSnapshot` with owned `Vec<u8>` and one decoded `String`.
Record the byte length of exactly one leading BOM:

```rust
const UTF8_BOM: &[u8] = b"\xef\xbb\xbf";

pub(crate) fn from_bytes(raw: &[u8]) -> Result<Self, SnapshotError> {
    let (bom, content_start) = if let Some(without_bom) = raw.strip_prefix(UTF8_BOM) {
        if without_bom.starts_with(UTF8_BOM) {
            return Err(SnapshotError::DuplicateUtf8Bom);
        }
        (Utf8Bom::Present, UTF8_BOM.len())
    } else {
        (Utf8Bom::Absent, 0)
    };
    let decoded_text = std::str::from_utf8(raw)
        .map_err(|_| SnapshotError::InvalidUtf8)?
        .to_owned();
    let text = decoded_text
        .get(content_start..)
        .ok_or(SnapshotError::InvalidUtf8)?;
    let (line_endings, terminal_line_ending) = classify_line_endings(text);
    Ok(Self {
        raw: raw.to_vec(),
        decoded_text,
        content_start,
        bom,
        line_endings,
        terminal_line_ending,
    })
}
```

Classify in one byte scan. When a `\r` is immediately followed by `\n`, count
one CRLF and advance by two bytes; otherwise count CR. Count remaining `\n` as
LF. Build `None`, `Uniform`, or exact `Mixed` counts without a dominant-style
fallback.

Implement `Display` for `SnapshotError` with:

```text
source contains more than one UTF-8 BOM
source is not valid UTF-8
```

- [x] **Step 5: Run focused tests and verify GREEN**

Run:

```bash
cargo test -p unica-coder text_snapshot -- --test-threads=1
```

Expected: all snapshot tests pass with no warnings.

- [x] **Step 6: Run formatting and crate compilation**

Run:

```bash
cargo fmt --all -- --check
cargo check -p unica-coder --all-features
```

Expected: both commands exit 0.

- [x] **Step 7: Commit Task 1**

```bash
git add crates/unica-coder/src/infrastructure/native_operations.rs \
  crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs
git commit -m "feat(writer): add exact text source snapshot"
```

---

### Task 2: Typed EOL Policy Resolution

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`

**Interfaces:**
- Consumes: `EolPolicy`, `&SourceTextSnapshot`, and optional selector-local `LineEnding`.
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EolPolicy {
    Preserve,
    Lf,
    CrLf,
    Repository,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EolPolicyError {
    AmbiguousPreservePolicy,
    MissingPreserveContext,
    RepositoryPolicyUnresolved,
}

pub(crate) fn resolve_line_ending(
    policy: EolPolicy,
    snapshot: &SourceTextSnapshot,
    local: Option<LineEnding>,
) -> Result<LineEnding, EolPolicyError>;

impl LineEnding {
    pub(crate) fn as_str(self) -> &'static str;
}
```

- [x] **Step 1: Write failing policy tests**

Add tests with these exact expectations:

```rust
#[test]
fn explicit_policies_ignore_source_profile() {
    let snapshot = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();
    assert_eq!(
        resolve_line_ending(EolPolicy::Lf, &snapshot, None),
        Ok(LineEnding::Lf)
    );
    assert_eq!(
        resolve_line_ending(EolPolicy::CrLf, &snapshot, None),
        Ok(LineEnding::CrLf)
    );
}

#[test]
fn preserve_prefers_local_context_for_mixed_source() {
    let snapshot = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();
    assert_eq!(
        resolve_line_ending(
            EolPolicy::Preserve,
            &snapshot,
            Some(LineEnding::CrLf)
        ),
        Ok(LineEnding::CrLf)
    );
}

#[test]
fn preserve_uses_uniform_source_without_local_context() {
    let snapshot = SourceTextSnapshot::from_bytes(b"A\nB\n").unwrap();
    assert_eq!(
        resolve_line_ending(EolPolicy::Preserve, &snapshot, None),
        Ok(LineEnding::Lf)
    );
}

#[test]
fn preserve_rejects_ambiguous_or_missing_context() {
    let mixed = SourceTextSnapshot::from_bytes(b"A\r\nB\n").unwrap();
    assert_eq!(
        resolve_line_ending(EolPolicy::Preserve, &mixed, None),
        Err(EolPolicyError::AmbiguousPreservePolicy)
    );
    let empty = SourceTextSnapshot::from_bytes(b"A").unwrap();
    assert_eq!(
        resolve_line_ending(EolPolicy::Preserve, &empty, None),
        Err(EolPolicyError::MissingPreserveContext)
    );
}

#[test]
fn repository_policy_is_fail_closed() {
    let snapshot = SourceTextSnapshot::from_bytes(b"A\n").unwrap();
    assert_eq!(
        resolve_line_ending(EolPolicy::Repository, &snapshot, None),
        Err(EolPolicyError::RepositoryPolicyUnresolved)
    );
}
```

- [x] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p unica-coder text_snapshot -- --test-threads=1
```

Expected: compilation fails because `EolPolicy`, `EolPolicyError`, and
`resolve_line_ending` are missing.

- [x] **Step 3: Implement minimal exhaustive policy resolution**

Implement without wildcard arms:

```rust
pub(crate) fn resolve_line_ending(
    policy: EolPolicy,
    snapshot: &SourceTextSnapshot,
    local: Option<LineEnding>,
) -> Result<LineEnding, EolPolicyError> {
    match policy {
        EolPolicy::Lf => Ok(LineEnding::Lf),
        EolPolicy::CrLf => Ok(LineEnding::CrLf),
        EolPolicy::Repository => Err(EolPolicyError::RepositoryPolicyUnresolved),
        EolPolicy::Preserve => match (local, snapshot.line_endings()) {
            (Some(line_ending), _) => Ok(line_ending),
            (None, LineEndingProfile::Uniform(line_ending)) => Ok(line_ending),
            (None, LineEndingProfile::Mixed { .. }) => {
                Err(EolPolicyError::AmbiguousPreservePolicy)
            }
            (None, LineEndingProfile::None) => {
                Err(EolPolicyError::MissingPreserveContext)
            }
        },
    }
}
```

Return `"\n"`, `"\r\n"`, or `"\r"` from `LineEnding::as_str`. Implement stable
`Display` text:

```text
preserve EOL policy is ambiguous for mixed line endings without local context
preserve EOL policy requires local or uniform source context
repository EOL policy is unresolved
```

- [x] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
cargo test -p unica-coder text_snapshot -- --test-threads=1
```

Expected: all snapshot and policy tests pass.

- [x] **Step 5: Run formatting and clippy for the module change**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --lib --all-features -- -D warnings
```

Expected: both commands exit 0 without warnings.

- [x] **Step 6: Commit Task 2**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs
git commit -m "feat(writer): resolve typed EOL policies"
```

---

### Task 3: Adopt the Shared Snapshot in `code.patch`

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs` only if a borrowed accessor required by the consumer is missing.

**Interfaces:**
- Consumes:

```rust
SourceTextSnapshot::from_bytes(&before)
resolve_line_ending(EolPolicy::Preserve, &snapshot, local)
LineEnding::as_str()
```

- Produces: unchanged public `CodePatchExecution` and serialized `CodePatchData`.

- [x] **Step 1: Write failing integration-focused unit tests**

Add a helper-level test that requires the shared type:

```rust
#[test]
fn local_eol_observation_is_resolved_by_shared_preserve_policy() {
    let snapshot = SourceTextSnapshot::from_bytes(
        b"Procedure First()\r\nEndProcedure\r\nProcedure Second()\nEndProcedure\n"
    )
    .unwrap();
    let offset = snapshot.decoded_text().len();
    let local = local_line_ending_at(snapshot.decoded_text(), offset, Position::After);
    assert_eq!(
        resolve_line_ending(EolPolicy::Preserve, &snapshot, local),
        Ok(LineEnding::Lf)
    );
}
```

Extend the existing BOM integration test so its assertions include the exact
original BOM and untouched suffix after preview/apply/no-op. Retain the existing
mixed-EOL test as a behavioral regression.

The production imports and `local_line_ending_at` API are not present yet, so
the focused test must fail to compile.

- [x] **Step 2: Run code-patch tests and verify RED**

Run:

```bash
cargo test -p unica-coder code_patch -- --test-threads=1
```

Expected: compilation fails because `code.rs` has not adopted the shared
snapshot/policy API.

- [x] **Step 3: Replace private source decoding and EOL enum**

Import:

```rust
use super::text_snapshot::{
    resolve_line_ending, EolPolicy, LineEnding, SourceTextSnapshot,
};
```

In `build_patch`, replace direct `std::str::from_utf8(&before)` with:

```rust
let snapshot = SourceTextSnapshot::from_bytes(&before)
    .map_err(|error| format!("BSL module snapshot: {error}"))?;
let text = snapshot.decoded_text();
```

Remove the private `Eol` enum and its `as_str` implementation. Change
`InsertionSite.eol` to `LineEnding`.

Change `locate_selector` to consume `&SourceTextSnapshot` and obtain
`snapshot.decoded_text()` internally. Rename the local observation helper to:

```rust
fn local_line_ending_at(
    text: &str,
    offset: usize,
    position: Position,
) -> Option<LineEnding>;
```

It returns `Some(LineEnding::CrLf)` or `Some(LineEnding::Lf)` when a neighboring
newline exists and `None` when neither side supplies local context. It does not
invent LF.

Within `locate_selector`, resolve after calculating the selector offset:

```rust
let local = local_line_ending_at(text, offset, position);
let eol = resolve_line_ending(EolPolicy::Preserve, snapshot, local)
    .map_err(|error| format!("resolve code.patch EOL: {error}"))?;
```

Before calculating the selector offset, reject `Uniform(LineEnding::Cr)` and
any `Mixed` profile whose lone-CR count is nonzero. This keeps the first
consumer fail-closed because its parser accepts CR-only input even though this
slice does not establish CR mutation support.

Store the resolved `LineEnding` in `InsertionSite` before calling
`normalized_content`. Update the test-only `locate_insertion` helper to create a
snapshot and pass it to `locate_selector`. In `prove_repeat_is_noop`, create one
snapshot from `postimage.as_bytes()` and use it for repeat location and policy
resolution. Do not alter selector offsets, diff generation, hash generation,
validation, or publication.

- [x] **Step 4: Run focused code-patch tests and verify GREEN**

Run:

```bash
cargo test -p unica-coder code_patch -- --test-threads=1
```

Expected: all code-patch tests pass, including LF, CRLF, mixed-local, BOM,
missing-terminal-newline, dry-run/apply equality, and repeat no-op coverage.

- [x] **Step 5: Run snapshot and code-patch tests together**

Run:

```bash
cargo test -p unica-coder text_snapshot -- --test-threads=1
cargo test -p unica-coder code_patch -- --test-threads=1
```

Expected: both commands exit 0.

- [x] **Step 6: Refactor while green**

Remove the old `Eol`, `local_eol_at`, and `eol_at_newline` definitions. Ensure
the shared `LineEnding` is used only for format semantics and `Position` remains
owned by `code.patch`.

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --lib --all-features -- -D warnings
```

Expected: no dead code, unused imports, wildcard matches, or warnings.

- [x] **Step 7: Commit Task 3**

```bash
git add crates/unica-coder/src/infrastructure/native_operations/code.rs \
  crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs
git commit -m "refactor(code): use shared text EOL snapshot"
```

---

### Task 4: Contract Documentation and Full Verification

**Files:**
- Modify: `docs/superpowers/specs/2026-07-26-writer-text-snapshot-eol-policy-design.md` only if implementation evidence reveals a factual mismatch.
- Modify: `docs/superpowers/plans/2026-07-26-writer-text-snapshot-eol-policy.md` by checking completed steps.

**Interfaces:**
- Consumes: completed implementation from Tasks 1–3.
- Produces: a verified, reviewable branch that references #74 without claiming the epic is complete.

- [x] **Step 1: Run the complete crate test suite**

Run:

```bash
cargo test -p unica-coder -- --test-threads=1
```

Expected: all `unica-coder` unit, integration, and doc tests pass; existing
ignored tests remain ignored and no new ignore is added.

- [x] **Step 2: Run formatting and strict Rust linting**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets --all-features -- -D warnings
```

Expected: both commands exit 0 with no warnings.

- [x] **Step 3: Run repository guardrails**

Run:

```bash
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check upstream/main...HEAD
```

Expected: platform-boundary check passes and Git reports no whitespace errors.

- [x] **Step 4: Review the Rust diff against the accepted design**

Verify explicitly:

- every new function is exercised by a test that was observed RED first;
- enums are exhaustive and type policy choices instead of booleans;
- source bytes are read once and no lossy conversion exists;
- mixed EOL has no dominant fallback;
- `repository` cannot silently resolve;
- `code.patch` public schema and serialized result fields are unchanged;
- no `meta.rs` or `form.rs` changes exist;
- no direct publication path was added.

Run:

```bash
git diff --stat upstream/main...HEAD
git diff upstream/main...HEAD -- \
  crates/unica-coder/src/infrastructure/native_operations.rs \
  crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs \
  crates/unica-coder/src/infrastructure/native_operations/code.rs
```

- [x] **Step 5: Commit any evidence-only doc correction**

If the accepted design needed a factual correction discovered by verification,
edit only that fact and commit:

```bash
git add docs/superpowers/specs/2026-07-26-writer-text-snapshot-eol-policy-design.md
git commit -m "docs: align writer snapshot design evidence"
```

If no correction is required, do not create an empty commit.

- [x] **Step 6: Verify final branch state**

Run:

```bash
git status --short --branch
git log --oneline --decorate upstream/main..HEAD
```

Expected: clean worktree and a small sequence containing the design, snapshot,
policy, and `code.patch` adoption commits.

---

### Task 5: Review Follow-up for Modules Without EOL

**Files:**
- Modify: `crates/unica-coder/src/infrastructure/native_operations/code.rs`
- Modify: `spec/architecture/code-patch-v1.md`
- Modify: `docs/superpowers/plans/2026-07-26-writer-text-snapshot-eol-policy.md`

**Interfaces:**
- Consumes: strict shared `resolve_line_ending(EolPolicy::Preserve, ...)`.
- Produces: a `code.patch`-local LF fallback for `LineEndingProfile::None`;
  the shared preserve policy remains fail-closed.

- [x] **Step 1: Add a failing end-to-end regression test**

Add a test using the valid one-line module
`Процедура Тест() КонецПроцедуры` that exercises preview, apply, and repeated
apply. Assert that preview does not write, apply inserts an LF separator and
LF-terminated content, and repeated apply is a semantic no-op.

- [x] **Step 2: Run the regression test and verify RED**

Run:

```bash
cargo test -p unica-coder infrastructure::native_operations::code::tests::code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop -- --exact
```

Expected: FAIL because `Preserve + LineEndingProfile::None` returns
`MissingPreserveContext`.

- [x] **Step 3: Implement the consumer-local policy choice**

In `locate_selector`, choose `EolPolicy::Lf` only when
`snapshot.line_endings() == LineEndingProfile::None`; use
`EolPolicy::Preserve` for every other supported profile. Do not weaken
`resolve_line_ending` and do not add a public EOL argument.

- [x] **Step 4: Run the regression and focused suites and verify GREEN**

Run:

```bash
cargo test -p unica-coder infrastructure::native_operations::code::tests::code_patch_without_any_source_eol_uses_lf_for_preview_apply_and_repeat_noop -- --exact
cargo test -p unica-coder native_operations::code::tests -- --test-threads=1
cargo test -p unica-coder text_snapshot -- --test-threads=1
```

Expected: all commands exit 0.

- [x] **Step 5: Update the active v1 contract**

Document in `spec/architecture/code-patch-v1.md` that a source without any EOL
uses LF for inserted separators/content for v1 compatibility, while any lone
CR source remains rejected before mutation.

- [x] **Step 6: Run full verification**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p unica-coder --all-targets --all-features -- -D warnings
cargo test -p unica-coder -- --test-threads=1
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check upstream/main...HEAD
```

Expected: every command exits 0; existing ignored tests remain ignored.
