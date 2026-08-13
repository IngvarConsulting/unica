# Logical Provider-Neutral Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the physical-path `unica.code.diagnostics` contract with one provider-neutral logical contract that returns navigable code and metadata findings, preserves provider provenance, and can add BSL LS or metadata validators without changing the public DTO.

**Architecture:** Add a diagnostics domain with a stable provider registry and typed observations, an application coordinator that selects and isolates providers, and one infrastructure mapper that translates provider resources into proven logical targets. Keep `bsl-analyzer` as the only live provider: its one-shot JSONL and resident MCP replies become typed provider outcomes, while the coordinator owns filtering, ordering, limits, completeness, and public serialization.

**Tech Stack:** Rust 2021, `serde`/`serde_json`, existing cancellation/deadline primitives, Platform XML source-target resolver, `bsl-analyzer` 0.2.62, Python contract tests, GitHub Actions Windows/Linux/macOS matrix.

## Global Constraints

- Implement the approved contract in `docs/design/2026-08-12-logical-provider-neutral-diagnostics-design.md` and proposed ADR-0055 through ADR-0058; do not re-open the design while executing the plan.
- Work on one branch based directly on current `main`; do not base the PR on another open PR.
- Follow strict TDD for every task: add the named failing test, run it and confirm the expected failure, then write production code and rerun it.
- Keep one public MCP server named `unica` and one public tool named `unica.code.diagnostics`; BSL LS and metadata-validator implementations are test doubles only in this PR.
- Public target selectors and diagnostic results never contain an absolute path, URI, `sourceDir`, or provider-specific payload. The generic optional `cwd` remains the sole absolute-path exception: it selects the workspace context, never identifies the diagnostic target, and is not echoed in result `data`. The only public physical diagnostic observation is a safe `/`-normalized `observedPath` relative to the selected source set.
- Do not deduplicate findings across providers. Rule identity is the exact `(provider, code)` pair.
- Resolve a named `sourceSet` exactly. Do not fall back to `main`, a sole source set, `cwd`, or a caller-supplied directory.
- Cancellation wins over all partial outcomes and publishes no collected items.
- Keep the current closed JSONL grammar, 8 MiB line bound, UTF-8 validation, secret redaction, and terminal-counter checks.
- The existing CI already runs `cargo test --workspace -- --test-threads=1` on `windows-latest`; add portable unit coverage and Windows-only drive/URI cases without adding a workflow.
- Before accepting ADR-0055 through ADR-0058, update from the actual target branch and renumber the proposed ADR files if another ADR number landed first.

---

## Task 1: Publish the strict action-based MCP request contract

**Files:**

- Create: `crates/unica-coder/src/domain/diagnostics.rs`
- Modify: `crates/unica-coder/src/domain/mod.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/application/tool_contracts.rs`

- [ ] **Step 1: Replace the old schema test with a failing action-union contract test**

In `application/tool_contracts.rs`, replace `bsl_diagnostics_contract_exposes_modes_and_configured_analyze_fallback` with focused tests named `diagnostics_contract_*`. Assert:

```rust
#[test]
fn diagnostics_contract_is_a_strict_discriminated_action_union() {
    let tool = tools().into_iter()
        .find(|tool| tool.name == "unica.code.diagnostics")
        .unwrap();
    let schema = input_schema_for_tool(&tool);
    let text = serde_json::to_string(&schema).unwrap();

    assert!(text.contains("\"action\""));
    assert!(text.contains("\"sourceSet\""));
    assert!(text.contains("\"analyze\""));
    assert!(text.contains("\"findings\""));
    assert!(text.contains("\"status\""));
    assert!(text.contains("\"catalog\""));
    let branches = schema["oneOf"].as_array().unwrap();
    assert_eq!(branches.len(), 4);
    for branch in branches {
        let properties = branch["properties"].as_object().unwrap();
        for removed in ["mode", "sourceDir", "path", "config", "format", "detail", "maxFiles", "rangeStart", "rangeEnd"] {
            assert!(!properties.contains_key(removed), "{removed}");
        }
    }
}
```

Add JSON Schema validation cases for the four actions. Cover required `action + sourceSet`, `metadataPath` only and required for `findings`, `range` only for `findings`, `timeoutSeconds` only for `analyze`, strict `filter`, strict `range`, strict `{provider, code}` entries, `providers.minItems=1`, and unique providers/codes.

- [ ] **Step 2: Add a failing stable legacy-removal test**

```rust
#[test]
fn diagnostics_legacy_target_removed_precedes_unknown_argument_validation() {
    let tool = diagnostic_tool();
    for legacy in ["mode", "sourceDir", "path"] {
        let args = Map::from_iter([
            ("action".into(), json!("analyze")),
            ("sourceSet".into(), json!("main")),
            (legacy.into(), json!("legacy")),
        ]);
        let error = validate_tool_argument_shape(tool, &args).unwrap_err();
        assert!(error.starts_with("legacy_target_removed:"), "{error}");
        assert!(error.contains("action + sourceSet"), "{error}");
    }
}
```

Run:

```powershell
cargo test -p unica-coder diagnostics_contract --lib
cargo test -p unica-coder diagnostics_legacy_target_removed --lib
```

Expected: failures because the live schema still exposes `mode/sourceDir/path` and accepts the old default mode.

- [ ] **Step 3: Introduce action and live-provider descriptors**

In `domain/diagnostics.rs`, start with contract-owned identifiers:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticAction { Analyze, Findings, Status, Catalog }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub struct DiagnosticProviderId(&'static str);

pub const BSL_ANALYZER_PROVIDER: DiagnosticProviderId =
    DiagnosticProviderId::new_const("bsl-analyzer");
```

In `application/operation_descriptors.rs`, add `DiagnosticActionDescriptor` values defining required/allowed fields per action. Make this table the source for both semantic validation and the four schema branches; do not duplicate the action matrix in separate matches.

- [ ] **Step 4: Replace `CodeAdapter { command: ["analyze"] }` with a diagnostics handler**

Add `ToolHandler::Diagnostics` and change only `unica.code.diagnostics` to use it. Keep `unica.code.graph` on `CodeAdapter`.

Update the description to state that the tool returns provider-neutral logical diagnostics. Require `action` and `sourceSet`; retain the generic optional `cwd` context field.

- [ ] **Step 5: Build a complete four-branch schema**

Add `diagnostics_input_schema()` in `tool_contracts.rs`. Each `oneOf` branch must be an independent `type: object`, `additionalProperties: false` object with `action.const`, `sourceSet`, `cwd`, and only its permitted fields. Define two filter schemas so catalog cannot accept `minSeverity`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "minSeverity": {"enum": ["error", "warning", "info", "hint"]},
    "codes": {
      "type": "array",
      "uniqueItems": true,
      "items": {
        "type": "object",
        "additionalProperties": false,
        "required": ["provider", "code"]
      }
    }
  }
}
```

The provider enum must be derived from the live descriptor list and initially equal exactly `["bsl-analyzer"]`. Do not publish `bsl-language-server` or `metadata-validator`.

- [ ] **Step 6: Implement pre-unknown-argument removal and action semantics**

Extend `validate_removed_target_arguments` for only `mode`, `sourceDir`, and `path`. Delete old diagnostics constants and validation for `config`, `format`, `detail`, `maxFiles`, `rangeStart`, and `rangeEnd`; those fields should receive the normal unknown-argument error.

Validate non-empty strings, ordered non-empty half-open ranges, `limit=1..=200`, `timeoutSeconds=30..=3600`, and exact `(provider, code)` filter structure. Leave target-kind applicability to Task 3 after logical resolution.

- [ ] **Step 7: Run the focused contract tests**

```powershell
cargo test -p unica-coder diagnostics_contract --lib
cargo test -p unica-coder diagnostics_legacy_target_removed --lib
cargo test -p unica-coder application::tool_contracts::tests --lib
```

Expected: all pass; `tools/list` has no old diagnostics fields.

- [ ] **Step 8: Commit the public request contract**

```powershell
git add crates/unica-coder/src/domain/diagnostics.rs crates/unica-coder/src/domain/mod.rs crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/operation_descriptors.rs crates/unica-coder/src/application/tool_contracts.rs
git commit -m "feat(diagnostics): объявить логический контракт запроса"
```

---

## Task 2: Add provider-neutral observations, public DTOs, and registry

**Files:**

- Modify: `crates/unica-coder/src/domain/diagnostics.rs`

- [ ] **Step 1: Write failing serialization and registry tests**

Cover:

- provider descriptors remain in registration order;
- duplicate provider IDs are rejected;
- test IDs `bsl-language-server` and `metadata-validator` can be registered without changing request/result types;
- public addressed locations have the same wire shape as `SourceLocation::Addressed`;
- public DTOs contain no `path`, `uri`, transport payload, command, stdout, or stderr;
- ranges serialize as zero-based, end-exclusive coordinate objects;
- metadata focus serializes `elementPath`, optional `property`, and optional `language`.

Run:

```powershell
cargo test -p unica-coder domain::diagnostics::tests --lib
```

Expected: compilation failure because the types and registry do not exist.

- [ ] **Step 2: Define parsed request and canonical selection types**

Use typed values instead of carrying `serde_json::Value` beyond parsing:

```rust
pub struct DiagnosticRequest {
    pub action: DiagnosticAction,
    pub source_set: String,
    pub metadata_path: Option<MetadataAddress>,
    pub requested_providers: Option<Vec<String>>,
    pub filter: DiagnosticFilter,
    pub range: Option<DiagnosticRange>,
    pub limit: usize,
    pub timeout: Option<Duration>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticSelection {
    pub source_set: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_path: Option<MetadataAddress>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<TargetKind>,
    pub providers: Vec<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<DiagnosticFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}
```

Materialize defaults (`minSeverity=warning`, empty `codes`, `limit=200`) in `selection` for `analyze/findings`; status and catalog expose only fields applicable to those actions.

- [ ] **Step 3: Define internal observations and public locations/focus**

Keep provider resource handles internal:

```rust
pub enum DiagnosticObservationLocation {
    Logical { metadata_path: Option<MetadataAddress> },
    Resource { handle: String }, // relative path, absolute path, or URI; never serialized
}

pub enum DiagnosticObservationFocus {
    Target,
    SourceRange(DiagnosticRange),
    Metadata(MetadataFocus),
}

pub enum DiagnosticLocation {
    Addressed { source_set: String, metadata_path: Option<MetadataAddress>, target_kind: TargetKind },
    Unaddressable { source_set: String, owner_metadata_path: Option<MetadataAddress>, observed_path: String, reason: UnaddressableReason },
}
```

Use closed public enums for severity, tags, unaddressable reason, provider status, readiness, result state, item kind, and focus kind.

- [ ] **Step 4: Define provider outcome and public result types**

The provider returns one typed structure for all actions:

```rust
pub struct DiagnosticProviderOutcome {
    pub status: DiagnosticProviderStatus,
    pub complete: bool,
    pub version: Option<String>,
    pub observations: Vec<DiagnosticObservation>,
    pub rules: Vec<DiagnosticRuleObservation>,
    pub readiness: Option<DiagnosticReadiness>,
    pub error: Option<DiagnosticError>,
}
```

The public result must have `action`, `selection`, `state`, `complete`, provider sections, global totals/truncation, and a flat `items` union. Provider sections always expose `resourceFailures` as the total before the global limit.

- [ ] **Step 5: Define the provider trait and stable registry**

```rust
pub trait DiagnosticProvider: Send + Sync {
    fn descriptor(&self) -> &'static DiagnosticProviderDescriptor;
    fn execute(
        &self,
        request: &DiagnosticProviderRequest,
        context: &DiagnosticContext,
        deadline: ProviderDeadline,
        cancellation: &CancellationToken,
    ) -> DiagnosticProviderOutcome;
}
```

`DiagnosticProviderRegistry::new(Vec<Arc<dyn DiagnosticProvider>>)` rejects duplicate IDs and preserves vector order. It supplies descriptors/IDs without sorting alphabetically.

- [ ] **Step 6: Run tests and commit**

```powershell
cargo test -p unica-coder domain::diagnostics::tests --lib
cargo fmt --all -- --check
git add crates/unica-coder/src/domain/diagnostics.rs
git commit -m "feat(diagnostics): добавить типизированную модель поставщиков"
```

---

## Task 3: Resolve exact logical targets and map provider locations safely

**Files:**

- Create: `crates/unica-coder/src/infrastructure/diagnostics.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/domain/metadata/operations.rs`

- [ ] **Step 1: Add failing `diagnostic_location_*` mapper tests**

Create Platform XML fixtures for:

- source root with no `metadataPath` -> addressed `sourceRoot`;
- `Catalog.Номенклатура` -> addressed `metadataObject`;
- `CommonModule.Доки_Обработка.Module` and nested form module -> addressed `module`;
- unknown file under a proven owner -> unaddressable with `resourceNotAddressable`;
- matching layout without owner proof -> `ownerUnproven`;
- unsupported source format -> `sourceFormatUnsupported`;
- path outside the selected root -> `location_outside_source_set` with no raw handle in the error;
- exact metadata property, attribute, and tabular-section attribute focus;
- unknown metadata element -> `focus.target`.

Name the test functions with `diagnostic_location` so the ADR command selects them.

Run:

```powershell
cargo test -p unica-coder diagnostic_location --lib
```

Expected: compilation failure because the mapper does not exist.

- [ ] **Step 2: Add failing Windows and URI normalization tests**

Test backslashes, Cyrillic/spaces, `file:///C:/...`, drive-letter case, another drive, `.` segments, and `..` escape. Keep drive-specific assertions under `#[cfg(windows)]`, but keep separator/Unicode/relative containment tests portable.

Assert every accepted observation uses `/` and every serialized failure omits the original absolute handle.

Run:

```powershell
cargo test -p unica-coder diagnostics_windows --lib
```

Expected: compilation failure.

- [ ] **Step 3: Add exact diagnostic context resolution to the application port**

Add:

```rust
fn resolve_diagnostic_context(
    &self,
    request: &DiagnosticRequest,
    workspace: &WorkspaceContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticContext, DiagnosticRequestError>;

fn map_diagnostic_observation(
    &self,
    observation: DiagnosticObservation,
    context: &DiagnosticContext,
    cancellation: &CancellationToken,
) -> Result<DiagnosticItem, DiagnosticMapError>;
```

The production implementation must call `resolve_named_source_set` and the Platform XML target resolver. It must never call the old `resolve_source_root` fallback. For findings, prove the exact target before provider selection; for analyze/status/catalog, prove the named source root.

- [ ] **Step 4: Reuse the existing Platform XML locator**

Extract a crate-private helper from `locate_platform_xml_source_path` that accepts the already-resolved named source set. Both `unica.source.locate` and diagnostics must use this helper. Do not duplicate module-layout or owner-evidence tables.

Expose only the minimum closed evidence needed by the BSL provider (`platform_xml_resource_evidence` after `resolve_platform_xml_target`); never add a physical path getter to a public/domain target.

- [ ] **Step 5: Implement `DiagnosticLocationMapper`**

The mapper must:

1. normalize `file:` URI or path identity;
2. reject other URI schemes;
3. prove containment in the selected root, including Windows drive identity;
4. ask the shared Platform XML locator for address/owner evidence;
5. compute `TargetKind` from the proven address;
6. emit addressed or safe unaddressable location;
7. validate or weaken focus.

Map `LocateRejection::NotAddressable` to `resourceNotAddressable` and `OwnerUnproven` to `ownerUnproven`. Treat `OutsideSourceSet` as a provider-contract failure, not as an unaddressable item.

- [ ] **Step 6: Reuse the canonical metadata vocabulary**

Make the existing `MetaCollection` parser/as-string mapping available crate-wide and add a closed canonical property validator beside it. The diagnostics mapper accepts only these canonical collection/property names; provider-specific XPath or XML field names must be translated inside that provider adapter or weakened to `focus.target`.

- [ ] **Step 7: Run mapper tests and commit**

```powershell
cargo test -p unica-coder diagnostic_location --lib
cargo test -p unica-coder diagnostics_windows --lib
cargo test -p unica-coder source_navigation --lib
git add crates/unica-coder/src/infrastructure/diagnostics.rs crates/unica-coder/src/infrastructure/mod.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/infrastructure/application_ports.rs crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs crates/unica-coder/src/domain/metadata/operations.rs
git commit -m "feat(diagnostics): сопоставлять наблюдения с логическими целями"
```

---

## Task 4: Select providers and assemble canonical results

**Files:**

- Create: `crates/unica-coder/src/application/diagnostics.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`

- [ ] **Step 1: Add failing fake-provider selection tests**

Use three fake providers registered in the order `bsl-analyzer`, `bsl-language-server`, `metadata-validator`. Assert:

- omitted `providers` selects all applicable providers in registry order;
- explicit order is canonicalized to registry order;
- explicit unsupported provider remains as `unsupported`;
- auto-excluded unsupported provider does not make the result partial;
- no auto-applicable provider returns `no_applicable_provider`;
- `filter.codes` without explicit providers narrows effective providers;
- explicit providers missing a code-filter provider returns `filter_provider_not_selected` before execution;
- unknown rule code is allowed and returns empty data.

Run:

```powershell
cargo test -p unica-coder diagnostics_provider_selection --lib
```

Expected: compilation failure because the coordinator does not exist.

- [ ] **Step 2: Add failing filter/order/limit tests**

Create observations whose physical input order differs from logical order. Assert:

- severity and exact `(provider, code)` filtering happens after mapping;
- range filtering uses half-open intersection;
- `focus.target` and `resourceFailure` for the selected module survive a range filter;
- duplicate-looking observations from two providers both remain;
- sort key is location, focus, provider registry order, item kind, code, message;
- metadata focus compares full `elementPath`, then property, then language;
- `itemsTotal` is after filters and before global limit;
- provider `itemsReturned` reflects the globally retained slice;
- `resourceFailures` remains the pre-limit total;
- `truncated` does not change `complete`.

Run:

```powershell
cargo test -p unica-coder diagnostics_result_assembly --lib
```

Expected: compilation failure.

- [ ] **Step 3: Parse validated JSON into `DiagnosticRequest`**

Implement one parser in `application/diagnostics.rs`; do not let individual providers read the MCP `Map<String, Value>`. Return stable request errors with code, optional field, and sanitized message. Parse `metadataPath` with `PLATFORM_XML_8_3_27_FORMAT_2_20` and reject `range` on a non-module as `target_kind_mismatch` after target resolution.

- [ ] **Step 4: Implement applicability and provider selection**

Selection uses the descriptor's supported actions and `findingsTargetKinds`. For omitted providers, silently exclude inapplicable providers. For explicit providers, create visible `unsupported` sections instead of dropping them.

Add `diagnostic_provider_registry()` to `ApplicationPorts`; production composition initially returns only `BslAnalyzerDiagnosticProvider`. Tests can return arbitrary registries.

- [ ] **Step 5: Implement normalization, filters, sort, totals, and public DTO**

Map every observation through the shared application port before filtering. If any observation produces `location_outside_source_set`, discard every item from that provider and replace its section with `failed` and sanitized error code.

Do not construct public JSON manually. Serialize `DiagnosticResult` once after the coordinator has finalized sections and items.

- [ ] **Step 6: Run tests and commit**

```powershell
cargo test -p unica-coder diagnostics_provider_selection --lib
cargo test -p unica-coder diagnostics_result_assembly --lib
git add crates/unica-coder/src/application/diagnostics.rs crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/ports.rs crates/unica-coder/src/infrastructure/application_ports.rs
git commit -m "feat(diagnostics): собрать результаты поставщиков"
```

---

## Task 5: Isolate provider failure, timeout, panic, and cancellation

**Files:**

- Modify: `crates/unica-coder/src/application/diagnostics.rs`
- Modify: `crates/unica-coder/src/domain/diagnostics.rs`

- [ ] **Step 1: Add failing outcome-matrix tests**

Test all provider statuses and result combinations:

| Provider outcomes | Expected result |
| --- | --- |
| all `completed/empty`, complete | `ok=true`, `state=completed`, `complete=true` |
| useful plus failed/unavailable/unsupported/incomplete | `ok=true`, `state=partial`, `complete=false` |
| no useful provider | `ok=false`, `state=failed`, `complete=false` |
| status returning readiness `building` | provider `completed`, call completed |
| findings while building | `unavailable`, `provider_not_ready`, retryable |

Run:

```powershell
cargo test -p unica-coder diagnostics_outcome_matrix --lib
```

Expected: failures in result state and completeness.

- [ ] **Step 2: Add failing concurrency safety tests**

Use blocking, panicking, and slow fake providers. Assert:

- providers start independently;
- a panic becomes only that provider's `failed` section;
- per-provider timeout cancels its linked child token;
- an all-call timeout finalizes unfinished providers without hanging;
- parent cancellation cancels every child and returns cancellation before public result serialization;
- no completed sibling items are published on cancellation.

Name the cancellation test `diagnostics_cancellation_discards_partial_items`.

- [ ] **Step 3: Implement bounded workers using the existing coordinator pattern**

Reuse the `CodeSearchCoordinator` pattern: registry-order result slots, `catch_unwind`, linked cancellation, `recv_timeout`, and tracked join handles. Extract a small shared worker-admission helper only if both coordinators can consume it without changing code-search behavior; otherwise keep diagnostics-specific bounded admission in `application/diagnostics.rs`.

The `timeoutSeconds` value is the total analyze budget. Every provider receives `ProviderDeadline::from_started_at(started_at, remaining_budget)`; do not forward the public number as a provider-specific CLI option.

- [ ] **Step 4: Normalize malformed provider outcomes**

Reject or normalize mismatched provider identity, payload for the wrong action, readiness on findings, rules on analyze, and an error-less failed status. Do not panic on a provider contract violation.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo test -p unica-coder diagnostics_outcome_matrix --lib
cargo test -p unica-coder diagnostics_cancellation --lib
cargo test -p unica-coder application::code_intelligence::tests --lib
git add crates/unica-coder/src/application/diagnostics.rs crates/unica-coder/src/domain/diagnostics.rs
git commit -m "feat(diagnostics): изолировать исполнение поставщиков"
```

---

## Task 6: Make the JSONL parser return typed analyzer observations

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

- [ ] **Step 1: Rewrite parser assertions first and confirm failure**

Change parser tests to expect `DiagnosticProviderOutcome`/typed observations instead of indexing `DiagnosticsProjection.data`. Preserve tests for start/file/done ordering, unknown fields, duplicate normalized paths, invalid UTF-8, line size, redaction, counter disagreement, incomplete stream, Cyrillic paths, and streams larger than the legacy 1 MiB stdout tail.

Add explicit assertions for the typed fields:

```rust
assert_eq!(outcome.observations[0].location, resource(".../Module.bsl"));
assert_eq!(outcome.observations[0].focus, source_range(0, 10, 0, 18));
```

Keep `DiagnosticProviderOutcome` and `DiagnosticObservation` free of `Serialize`;
only the separate public result types derive it.

Run:

```powershell
cargo test -p unica-coder infrastructure::diagnostics_jsonl::tests --lib
```

Expected: compilation failures because the parser still returns `Value`.

- [ ] **Step 2: Remove public filtering and JSON assembly from the parser**

Delete `codes`, `min_severity`, `detailed`, public `limit`, `StoredItem::into_json`, and `DiagnosticsProjection.data`. The parser should return a typed batch with analyzer version, file totals, elapsed time, observations, and a typed protocol failure.

Map a file error to `DiagnosticObservation::ResourceFailure` with a redacted message. Map analyzer severities to the common four-level enum and known tags to the common tag enum.

- [ ] **Step 3: Keep protocol validation exact**

Do not relax `#[serde(deny_unknown_fields)]` for JSONL events. Invalid or contradictory streams return a failed provider outcome with zero publishable observations; they never return an empty completed analysis.

- [ ] **Step 4: Adjust the streaming process adapter**

Change `invoke_diagnostics_analyze` to return the typed analyzer batch to the provider adapter. Keep managed-process cancellation/tree termination and stderr handling internal. Remove construction of public `action/state/items` JSON from `internal_adapters.rs`.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo test -p unica-coder infrastructure::diagnostics_jsonl::tests --lib
cargo test -p unica-coder diagnostics_analyze --lib
git add crates/unica-coder/src/infrastructure/diagnostics_jsonl.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "refactor(diagnostics): типизировать поток bsl-analyzer"
```

---

## Task 7: Implement the live `bsl-analyzer` provider for all four actions

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/diagnostics.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`
- Modify: `crates/unica-coder/src/infrastructure/workspace_services.rs`

- [ ] **Step 1: Add failing adapter request tests**

Assert exact private requests:

- analyze runs `bsl-analyzer analyze --format jsonl <resolved source root>`;
- public filters and `limit` are not forwarded as authoritative filtering;
- findings resolves `metadataPath` to the proven module resource before calling upstream `diagnostics` action `file`;
- status calls upstream action `status` for the selected source-set session;
- catalog calls upstream action `catalog`;
- no public request field is treated as an upstream raw argument.

Run:

```powershell
cargo test -p unica-coder bsl_diagnostics_provider_request --lib
```

Expected: failures because the old adapter still reads `mode/sourceDir/path`.

- [ ] **Step 2: Add failing typed reply tests**

Fixture resident replies for:

- complete findings with range/code/message/severity/tags;
- `status=loading` -> readiness `building` for status;
- `status=loading` -> `provider_not_ready` for findings;
- ready/stale/not-started mappings;
- catalog capabilities and rules;
- malformed JSON -> failed provider, raw reply not public;
- file/resource failure -> common `resourceFailure`;
- analyzer version `0.2.62` preserved in the provider section.

- [ ] **Step 3: Implement `BslAnalyzerDiagnosticProvider`**

Descriptor:

```rust
DiagnosticProviderDescriptor {
    id: BSL_ANALYZER_PROVIDER,
    actions: &[Analyze, Findings, Status, Catalog],
    findings_target_kinds: &[TargetKind::Module],
    emits_focus_kinds: &[DiagnosticFocusKind::SourceRange],
}
```

For findings, call `resolve_platform_xml_target(..., ModuleOnly)` and `platform_xml_resource_evidence` inside infrastructure, then pass only the proven module path to the private upstream request.

- [ ] **Step 4: Key resident diagnostics state correctly**

Ensure the workspace-service identity includes workspace root/worktree, `workspaceEpoch`, resolved source-set identity, and provider ID. Keep graph and diagnostics requests on compatible shared `bsl-analyzer` service machinery, but do not let state for one source set satisfy another.

Status is a successful read even when readiness is `building`. Findings never publishes stale findings.

- [ ] **Step 5: Parse upstream replies into common outcomes**

Define private serde DTOs beside the provider. Unknown upstream fields may be ignored internally for forward compatibility, but only explicitly mapped common fields reach observations/rules/readiness. Never return upstream `Value`.

- [ ] **Step 6: Run provider tests and commit**

```powershell
cargo test -p unica-coder bsl_diagnostics_provider --lib
cargo test -p unica-coder infrastructure::workspace_services::tests --lib
git add crates/unica-coder/src/infrastructure/diagnostics.rs crates/unica-coder/src/infrastructure/internal_adapters.rs crates/unica-coder/src/infrastructure/workspace_services.rs
git commit -m "feat(diagnostics): подключить поставщик bsl-analyzer"
```

---

## Task 8: Wire the coordinator through the public tool and operational budget

**Files:**

- Modify: `crates/unica-coder/src/application/mod.rs`
- Modify: `crates/unica-coder/src/application/diagnostics.rs`
- Modify: `crates/unica-coder/src/application/operational_config.rs`
- Modify: `crates/unica-coder/src/infrastructure/application_ports.rs`
- Modify: `crates/unica-coder/src/infrastructure/internal_adapters.rs`

- [ ] **Step 1: Add failing application-level contract tests**

Using fake ports, assert:

- `ToolHandler::Diagnostics` dispatches only through `DiagnosticCoordinator`;
- a completed diagnostic result appears only in `OperationResult.data`;
- no success uses `stdout`, `stderr`, `command`, or physical artifacts;
- `OperationResult.ok` follows coordinator useful-result semantics;
- cancellation returns no typed partial data;
- direct `ports.invoke_handler` rejects diagnostics as misrouted;
- graph still uses `BslAnalyzerMcpAdapter` unchanged.

Run:

```powershell
cargo test -p unica-coder diagnostics_application_boundary --lib
```

Expected: failures because the handler is not dispatched.

- [ ] **Step 2: Dispatch `ToolHandler::Diagnostics` in application**

Add a branch beside code intelligence/source navigation:

```rust
ToolHandler::Diagnostics => diagnostics::invoke(
    ports,
    args,
    &context,
    operational_config.as_ref(),
    cancellation,
)?,
```

Convert `DiagnosticExecution` to `HandlerOutcome` once; do not merge provider diagnostics into the unrelated top-level `OperationResult.diagnostics` field.

- [ ] **Step 3: Change operational-config selection from mode to action**

`requires_snapshot` is true for diagnostics only when `action == "analyze"`. The public `timeoutSeconds` override remains `30..=3600` and replaces `operational.code_diagnostics.analyze_timeout_seconds`; status/findings/catalog do not read operational config.

Update tests:

```rust
for action in ["findings", "status", "catalog"] {
    assert!(!requires_snapshot(spec, &diagnostic_args(action)));
}
assert!(requires_snapshot(spec, &diagnostic_args("analyze")));
```

- [ ] **Step 4: Delete the legacy diagnostics dispatch**

Remove diagnostics handling from `BslAnalyzerMcpAdapter::invoke_cancellable_with_operational_config`, `diagnostics_mode`, `validate_diagnostics_path`, and the old `bsl_mcp_tool_request` public-argument bridge. Retain graph paths and private helpers used by `BslAnalyzerDiagnosticProvider`.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo test -p unica-coder diagnostics_application_boundary --lib
cargo test -p unica-coder application::operational_config::tests --lib
cargo test -p unica-coder typed_result --lib
git add crates/unica-coder/src/application/mod.rs crates/unica-coder/src/application/diagnostics.rs crates/unica-coder/src/application/operational_config.rs crates/unica-coder/src/infrastructure/application_ports.rs crates/unica-coder/src/infrastructure/internal_adapters.rs
git commit -m "feat(diagnostics): включить нейтральный координатор"
```

---

## Task 9: Prove the AI navigation contract and future-provider compatibility

**Files:**

- Modify: `crates/unica-coder/src/application/diagnostics.rs`
- Modify: `crates/unica-coder/src/infrastructure/diagnostics.rs`
- Modify: `crates/unica-coder/src/application/mod.rs`

- [ ] **Step 1: Add a failing Platform XML end-to-end fixture**

Build a source map and Platform XML module with Cyrillic/spaces. Invoke:

```json
{
  "action": "findings",
  "sourceSet": "main",
  "metadataPath": "CommonModule.Доки Обмена.Module"
}
```

Return one fake analyzer observation addressed by its resource path. Assert the public result contains:

```json
{
  "location": {
    "kind": "addressed",
    "sourceSet": "main",
    "metadataPath": "CommonModule.Доки Обмена.Module",
    "targetKind": "module"
  },
  "focus": {"kind": "sourceRange"}
}
```

Recursively assert no key named `path`, `uri`, `sourceDir`, `stdout`, `stderr`, or `command` exists in `data`, and no string contains the absolute fixture root.

Run:

```powershell
cargo test -p unica-coder diagnostics_logical_end_to_end --lib
```

Expected: failure until every application/mapper/provider seam is connected.

- [ ] **Step 2: Add a fake BSL LS conformance test**

Register only a test `bsl-language-server` provider that emits `file:///.../Module.bsl`. Run the same request and assert the same `addressed + sourceRange` wire shape with provider provenance changed. Also assert the production schema still lists only `bsl-analyzer`.

- [ ] **Step 3: Add a fake metadata provider conformance test**

Register only `metadata-validator`, select `Catalog.Номенклатура`, and emit:

```rust
MetadataFocus {
    element_path: vec![
        element("tabularSections", "Товары"),
        element("attributes", "Цена"),
    ],
    property: Some("Type".into()),
    language: None,
}
```

Assert the request DTO, public selection, provider section, and flat item union are unchanged; only `targetKind=metadataObject` and `focus.kind=metadata` differ.

- [ ] **Step 4: Add a metadata-object scope boundary test**

Prove that findings for an object may include its property/non-addressable inner-element observations but excludes a separately addressable child module/form/command/template. Those observations belong to their own `metadataPath` request.

- [ ] **Step 5: Run tests and commit**

```powershell
cargo test -p unica-coder diagnostics_logical_end_to_end --lib
cargo test -p unica-coder diagnostics_future_provider_contract --lib
git add crates/unica-coder/src/application/diagnostics.rs crates/unica-coder/src/infrastructure/diagnostics.rs crates/unica-coder/src/application/mod.rs
git commit -m "test(diagnostics): доказать логическую навигацию и расширяемость"
```

---

## Task 10: Update the model-facing skill, migration guide, and release probes

**Files:**

- Modify: `plugins/unica/skills/code-diagnostics/SKILL.md`
- Modify: `plugins/unica/skills/code-review/SKILL.md`
- Modify: `plugins/unica/skills/query-optimize/SKILL.md`
- Modify: `tests/ci/test_unica_skills.py`
- Create: `docs/migrations/0.13.0-logical-diagnostics.md`
- Modify: `docs/migrations/README.md`
- Modify: `README.md`
- Modify: `scripts/ci/release-assessment.py`
- Modify: `tests/ci/test_release_assessment.py`
- Modify: `spec/architecture/tool-surface-review.json`
- Regenerate: `spec/architecture/tool-surface.md`
- Modify: `tests/ci/test_tool_surface_ledger.py`

- [ ] **Step 1: Add failing skill examples and migration assertions**

Update `test_unica_skills.py` to require `action`, `sourceSet`, logical `metadataPath`, provider-qualified codes, and no old diagnostics fields in every executable diagnostics example. Add direct assertions for one analyze and one findings example.

Run:

```powershell
python -m unittest tests.ci.test_unica_skills
```

Expected: failures on the old `mode=file/workspace/analyze` examples.

- [ ] **Step 2: Rewrite `code-diagnostics` for the new workflow**

Teach the model:

1. choose exact `sourceSet`;
2. call `status` when resident readiness matters;
3. call `findings` with a logical `metadataPath` for one target;
4. use `analyze` for a complete one-shot source-set scan;
5. use `catalog` to discover provider-qualified rule codes;
6. follow `location` to a logical target and `focus` inside it;
7. treat `partial`, `resourceFailure`, and `unaddressable` as distinct signals.

Replace parameterized old-mode guidance in `code-review` and `query-optimize`. Scan tracked skills for any remaining old diagnostics examples and update only those hits:

```powershell
rg -n "mode=(analyze|file|workspace)|\"mode\": \"(analyze|file|workspace)\"" plugins/unica/skills
```

Expected after edits: no diagnostics example uses an old mode.

- [ ] **Step 3: Write the clean-break migration guide**

Create `docs/migrations/0.13.0-logical-diagnostics.md` with the exact approved table:

```text
mode=analyze + sourceDir -> action=analyze + sourceSet
mode=file + path         -> action=findings + sourceSet + metadataPath
mode=status              -> action=status + sourceSet
mode=catalog             -> action=catalog + sourceSet
mode=workspace           -> analyze, or status + logical findings
codes[]                  -> filter.codes[{provider, code}]
minSeverity              -> filter.minSeverity
rangeStart/rangeEnd      -> range
config/format/detail/maxFiles -> removed
```

Link it from `docs/migrations/README.md` and update the root README transition section. Do not bump manifests or Cargo version in this feature PR; the release owner chooses the actual version. If `main` is not targeting 0.13.0 at implementation time, rename this guide to the release version selected on `main` before commit and update both links atomically.

- [ ] **Step 4: Update packaged release assessment calls**

Change the diagnostics probe in `scripts/ci/release-assessment.py` from `mode=workspace` to an executable logical action/sourceSet call. Update `test_release_assessment.py` to reject old modes and verify the selected action.

- [ ] **Step 5: Update and regenerate the tool-surface ledger**

Change the reviewed result note to the provider-neutral logical DTO and keep `scope: in`, `contract: typed`. Add focused ledger assertions for action/provider/filter/range and absence of old arguments.

Regenerate instead of hand-editing:

```powershell
python scripts/ci/generate-tool-surface.py
```

- [ ] **Step 6: Run documentation/packaging tests and commit**

```powershell
python -m unittest tests.ci.test_unica_skills tests.ci.test_release_assessment tests.ci.test_tool_surface_ledger
git add plugins/unica/skills/code-diagnostics/SKILL.md plugins/unica/skills/code-review/SKILL.md plugins/unica/skills/query-optimize/SKILL.md tests/ci/test_unica_skills.py docs/migrations docs/migrations/README.md README.md scripts/ci/release-assessment.py tests/ci/test_release_assessment.py spec/architecture/tool-surface-review.json spec/architecture/tool-surface.md tests/ci/test_tool_surface_ledger.py
git commit -m "docs(diagnostics): описать логический публичный контракт"
```

---

## Task 11: Accept the atomic ADRs and bind their derived invariants

**Files:**

- Modify: `spec/decisions/0055-tipizirovannaya-gotovnost-rlm.md`
- Modify: `spec/decisions/0056-logicheskie-nablyudeniya-diagnostiki.md`
- Modify: `spec/decisions/0057-neytralnaya-kompoziciya-diagnostik.md`
- Modify: `spec/decisions/0058-yavnyy-rezhim-migracii-chitatelya.md`
- Modify: `spec/decisions/0045-typed-reader-completion-contract.md`
- Modify: `spec/decisions/0049-most-logicheskoy-adresacii-predmetnyh-chitateley.md`
- Modify: `spec/decisions/README.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/architecture/runtime.md`
- Modify: `spec/architecture/change-checklist.md`
- Modify: `tests/ci/test_architecture_registry.py`
- Modify: `tests/ci/test_architecture_sync_guard.py`

- [ ] **Step 1: Add failing registry ownership tests**

Require three new atomic registry rules:

- `INV-MCP-DIAGNOSTIC-TARGET` owned only by ADR-0056 for logical location/focus and absence of public absolute paths;
- `INV-APP-DIAGNOSTIC-PROVIDERS` owned only by ADR-0057 for registry order, provenance, independent failure, and no cross-provider deduplication;
- `INV-SOURCE-READER-MIGRATION` owned only by ADR-0058 for explicit `bridge|directSwitch`, with diagnostics named as the sole direct switch.

Keep RLM readiness in the existing typed-result/application rule but replace ADR-0045 ownership with ADR-0055. Do not merge these concerns into one new invariant.

Run:

```powershell
python -m unittest tests.ci.test_architecture_registry tests.ci.test_architecture_sync_guard
```

Expected: failures because the new rules and accepted owners are absent.

- [ ] **Step 2: Rebase the ADR numbers against target `main`**

```powershell
git fetch origin main
git log --oneline -- spec/decisions
```

If `0054..0057` conflict with decisions now present on `origin/main`, renumber the four unmerged proposed files monotonically, update every citation/design Decision field/index entry/test, and only then continue. Never reuse a number that reached `main`.

- [ ] **Step 3: Accept the four decisions and supersede historical owners**

Set ADR-0055 through ADR-0058 to `accepted`. On historical ADR-0045 change only the lifecycle metadata and add a resolvable replacement note naming ADR-0055, ADR-0056, and ADR-0057; do not rewrite its accepted decision text. Do the same minimal lifecycle update on ADR-0049 with ADR-0058. Update the decision index sections.

- [ ] **Step 4: Update derived architecture without broadening an ADR**

Add the three rules above with direct checks:

- diagnostics contract/mapper/coordinator Rust tests;
- `tests/ci/test_unica_skills.py` for model-facing calls;
- `tests/ci/test_architecture_registry.py` for migration mode ownership.

Update `INV-MCP-TYPED-RESULT`, `INV-APP-CONFIG-SNAPSHOT`, and relevant runtime prose from diagnostics `mode=analyze` to `action=analyze`. Keep ADR-0055 references limited to RLM readiness.

Add the diagnostics direct-switch checklist to `change-checklist.md`: schema, handler, skill, examples, ledger, migration guide, release probe, and executable tests must land together.

- [ ] **Step 5: Run architecture checks and commit**

```powershell
python -m unittest tests.ci.test_architecture_registry tests.ci.test_architecture_sync_guard tests.ci.test_design_documents
git add spec/decisions spec/architecture tests/ci/test_architecture_registry.py tests/ci/test_architecture_sync_guard.py
git commit -m "docs(architecture): принять контракт логических диагностик"
```

---

## Task 12: Full verification, self-review, and pull-request handoff

**Files:**

- Review: all files changed by Tasks 1-11

- [ ] **Step 1: Run formatter and focused Rust suites**

```powershell
cargo fmt --all -- --check
cargo test -p unica-coder diagnostics --lib -- --test-threads=1
cargo test -p unica-coder source_navigation --lib -- --test-threads=1
cargo test -p unica-coder code_definition --lib -- --test-threads=1
cargo test -p unica-coder infrastructure::rlm_navigation::tests:: --lib -- --test-threads=1
```

Expected: all pass.

- [ ] **Step 2: Run the complete crate and CI contract suites**

```powershell
cargo test -p unica-coder -- --test-threads=1
python -m unittest discover -s tests/ci -p "test_*.py"
```

Expected: all pass. Do not claim Windows portability only from path-string unit tests; record the Windows job result after CI runs.

- [ ] **Step 3: Run static repository guards**

```powershell
python scripts/ci/check-architecture-sync.py
python scripts/ci/check-rust-platform-boundary.py
git diff --check origin/main...HEAD
```

Expected: all commands exit 0.

- [ ] **Step 4: Audit the public wire contract recursively**

Inspect schema fixtures and representative analyze/findings/status/catalog results. Verify:

- no old request fields are published;
- only `bsl-analyzer` is in the live provider enum;
- no addressed item contains a physical path;
- unaddressable items contain only safe relative `observedPath`;
- no provider payload or unknown upstream field crosses the boundary;
- public counts remain internally consistent after filters and limits;
- metadata observations use canonical element/property vocabulary;
- cancellation tests serialize no partial result.

- [ ] **Step 5: Review scope and history**

```powershell
git status --short
git log --oneline origin/main..HEAD
git diff --stat origin/main...HEAD
git diff origin/main...HEAD
```

Confirm there is no BSL LS production registration, no metadata-validator production registration, no version bump, no unrelated cleanup, and no child-PR base.

- [ ] **Step 6: Commit any review-only corrections after reproducing them**

If review finds a defect, first add and run a failing regression test, then fix it and commit the focused correction. Do not fold an untested review fix silently into an earlier commit.

- [ ] **Step 7: Push and open one PR against `main`**

Use the repository's publishing workflow after all checks are green. The PR description must include:

- old/new request migration table;
- representative logical result without paths;
- provider partial-success/cancellation semantics;
- Windows path/URI cases tested;
- exact local commands and results;
- ADR-0055 through ADR-0058 atomic ownership;
- explicit statement that BSL LS and metadata diagnostics are future providers, not part of this PR.

Wait for the existing Rust matrix, especially `windows-latest`, before merge. Address introduced review/CI defects on the same head branch with a failing regression test first.
