# Writer Text Snapshot and EOL Policy Design

Status: accepted
Date: 2026-07-26
Parent epic: [#74](https://github.com/IngvarConsulting/unica/issues/74)

## Goal

Introduce one internal, byte-exact model for UTF-8 writer input and line-ending
selection, then adopt it in `unica.code.patch` without changing the public MCP
contract or migrating unrelated XML writers.

This is the first remaining independently reviewable slice of #74 after the
canonical registrar, `CompileTransaction`, safe single-file publisher, and safe
`unica.code.patch` v1 foundations reached `main`.

## Scope

The slice:

- reads a regular UTF-8 file once into an exact immutable snapshot;
- records whether one UTF-8 BOM is present;
- classifies all source line endings as none, uniform, or mixed;
- records the exact terminal line ending independently;
- models `preserve`, `lf`, `crlf`, and `repository` EOL policies;
- resolves inserted-text EOL without rewriting untouched source bytes;
- migrates `unica.code.patch` from its private EOL detector to the shared model;
- adds focused unit tests and retains all existing `code.patch` behavior.

The slice does not:

- expose a new MCP argument or change the `unica.code.patch` schema;
- parse `.gitattributes`, `.editorconfig`, or Git configuration;
- migrate `meta`, `form`, `role`, `skd`, `subsystem`, `support`, `cfe`, or other
  writers;
- introduce the shared structured mutation-result contract;
- close #74.

The pull request must use `Refs #74`, not `Closes #74`.

## Alternatives

### Adopted: internal snapshot core with one existing consumer

Create a focused internal module and use `unica.code.patch` as the first
consumer. This removes one real duplicate immediately, proves the API against a
public writer, and keeps the review boundary small.

### Rejected: migrate all writers in one pull request

This would mix format detection, result-schema changes, serializer behavior,
and many platform-specific regressions. It would also collide with open changes
to `meta.rs` and `form.rs`.

### Rejected: keep per-writer EOL helpers

Adding more local fixes would preserve the current contradiction: `code.patch`
models only LF/CRLF while `meta.edit` separately models LF/CRLF/CR. It would not
establish the shared contract required by #74.

## Components

### `text_snapshot` module

Create
`crates/unica-coder/src/infrastructure/native_operations/text_snapshot.rs`.
The module owns only exact UTF-8 source observation and EOL policy resolution.
It does not publish files and does not know about MCP requests.

The central types are:

```rust
pub(crate) struct SourceTextSnapshot {
    raw: Vec<u8>,
    decoded_text: String,
    content_start: usize,
    bom: Utf8Bom,
    line_endings: LineEndingProfile,
    terminal_line_ending: Option<LineEnding>,
}

pub(crate) enum Utf8Bom {
    Present,
    Absent,
}

pub(crate) enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

pub(crate) enum LineEndingProfile {
    None,
    Uniform(LineEnding),
    Mixed {
        lf: usize,
        crlf: usize,
        cr: usize,
    },
}

pub(crate) enum EolPolicy {
    Preserve,
    Lf,
    CrLf,
    Repository,
}
```

`SourceTextSnapshot::from_bytes` accepts exactly zero or one leading UTF-8 BOM.
The BOM is retained unchanged in `raw` and represented as U+FEFF in
`decoded_text`. The `text()` accessor returns the content after that one
preamble, while `decoded_text()` returns the byte-aligned decoded source for
consumers whose UTF-8 offsets must still index `raw`. A second leading BOM is
rejected as an ambiguous text preamble. Invalid UTF-8 is rejected without lossy
conversion.

The module exposes borrowed accessors instead of public fields. Callers cannot
mutate the snapshot after construction.

### Line-ending classification

Classification scans the decoded text once and counts:

- `\r\n` as one CRLF;
- a remaining `\n` as LF;
- a remaining `\r` as CR.

The profile is:

- `None` when no line ending occurs;
- `Uniform(kind)` when exactly one kind occurs;
- `Mixed { ... }` when at least two kinds occur.

`terminal_line_ending` reports the exact final sequence or `None`. It is not
derived from the dominant profile.

No dominant-EOL heuristic exists in this slice.

### Policy resolution

Policy resolution returns a `LineEnding` for newly inserted text and never
normalizes the source snapshot.

```rust
pub(crate) fn resolve_line_ending(
    policy: EolPolicy,
    snapshot: &SourceTextSnapshot,
    local: Option<LineEnding>,
) -> Result<LineEnding, EolPolicyError>;
```

Rules:

- `Lf` resolves to LF.
- `CrLf` resolves to CRLF.
- `Repository` returns `RepositoryPolicyUnresolved`.
- `Preserve` uses an explicit local line ending when supplied.
- Without a local line ending, `Preserve` uses a uniform source profile.
- `Preserve` returns `AmbiguousPreservePolicy` for a mixed profile.
- `Preserve` returns `MissingPreserveContext` when the source has no line
  endings and no local line ending.

Errors are a typed internal enum with stable `Display` text. No fallback chooses
LF silently.

### `unica.code.patch` adoption

`code.patch` constructs one `SourceTextSnapshot` from the bytes it already
reads and uses `decoded_text()` so existing offsets continue to address the
exact raw preimage even when it starts with a BOM. Its selector, parser
validation, exact-preimage publication, hashes, ranges, and MCP data remain
unchanged.

The insertion-site scanner continues to determine the local line ending from
the selected method or anchor. That observation becomes `Option<LineEnding>`
and is passed to `resolve_line_ending(EolPolicy::Preserve, ...)`.

Existing behavior remains:

- LF input receives LF inserted text;
- CRLF input receives CRLF inserted text;
- mixed input uses the selected local context rather than a global majority;
- untouched bytes, BOM, and terminal newline remain byte-identical;
- dry-run and apply derive output from the same snapshot;
- repeat apply is a no-op.

CR-only source is classified by the shared core, but `code.patch` does not gain
CR insertion support in this slice because its parser/range path has not yet
established CR-only acceptance. Such a target remains rejected by existing
validation rather than being normalized.

## Error Handling

Snapshot construction fails before planning when:

- bytes are not valid UTF-8 after the optional BOM;
- more than one leading UTF-8 BOM is present.

Policy resolution fails before postimage construction when:

- `repository` has no implemented repository resolver;
- `preserve` lacks unambiguous local or uniform source evidence.

All failures occur before publication. They do not create workspace events or
cache changes.

## Testing

Focused `text_snapshot` unit tests cover:

- BOM present and absent;
- duplicate BOM rejection;
- invalid UTF-8 rejection;
- no line ending;
- uniform LF, CRLF, and CR;
- mixed LF/CRLF/CR counts;
- exact terminal LF, CRLF, CR, and absent state;
- explicit LF and CRLF policy;
- local preserve on mixed input;
- uniform preserve without local context;
- ambiguous mixed preserve;
- missing preserve context;
- unresolved repository policy.

Existing `code.patch` tests remain green. Additional integration-focused unit
tests prove:

- LF, CRLF, and mixed-local insertion still choose the same bytes;
- BOM and untouched suffix remain exact;
- repeat apply remains a no-op;
- dry-run and apply report identical planned postimage hashes.

Verification includes:

```text
cargo fmt --all -- --check
cargo test -p unica-coder text_snapshot -- --test-threads=1
cargo test -p unica-coder code_patch -- --test-threads=1
cargo test -p unica-coder -- --test-threads=1
cargo clippy -p unica-coder --all-targets --all-features -- -D warnings
python3 scripts/ci/check-rust-platform-boundary.py
git diff --check
```

## Follow-up Slices

1. Resolve `repository` from an explicit, tested repository policy source
   without inventing a dominant fallback.
2. Introduce the shared structured mutation result with effect, affected
   targets, hashes, changed ranges, selected policy, and warnings.
3. Migrate XML writers in separate reviewable pull requests after their current
   feature PRs merge.
4. Build the cross-writer byte-regression matrix and only then consider closing
   #74.
