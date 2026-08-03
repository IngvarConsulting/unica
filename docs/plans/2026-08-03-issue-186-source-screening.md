# Issue #186 Source Screening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce a source-backed screening of all 19 repositories named in issue #186, with immutable snapshots, license findings, proven mechanisms, live Unica mappings, and a justified shortlist for later deep dives.

**Architecture:** Keep the deliverable as one historical Markdown review under `docs/provenance/reviews/`. Build it cohort by cohort using one fixed source-card schema; gather temporary repository trees and command output only under ignored `.build/issue-186/`, then normalize decisions across all cards after every source has been inspected. This PR records evidence and recommendations only: it does not add runtime code, public tools, machine-readable contracts, follow-up issues, or copied upstream materials.

**Tech Stack:** Markdown, Git/GitHub repository metadata, PowerShell, `rg`, existing Rust/Python source and tests as Unica evidence.

## Global Constraints

- The governing design is `docs/design/2026-08-03-issue-186-research-slicing-design.md`.
- Screen exactly the 19 repositories named in issue #186; do not silently drop an unavailable, archived, renamed, or contradictory source.
- Every source card records canonical repository URL, default branch, immutable commit SHA or an explicit acquisition failure, snapshot date, license evidence, implementation/test/config evidence, mechanism, Unica mapping, limits, and one decision: `deep-dive`, `defer`, or `reject`.
- README and Telegram posts may explain claimed intent, but `deep-dive` requires evidence from code, tests, package metadata, or a reproducible probe.
- `gpt-5.6-luna` may draft source cards only when every claim links to concrete primary evidence; it does not own final decisions.
- A stronger reasoning model must perform cross-source review of all 19 cards before any decision enters the summary registry or thematic shortlist.
- If `gpt-5.6-luna` is unavailable, use the stronger model for primary screening as well; never remove or sample the stronger-model quality gate.
- No external code, prose, rules, skills, fixtures, or generated artifacts are copied into tracked Unica files.
- Missing or contradictory licensing blocks material transfer but does not block behavioral observation.
- `Nikolay-Shirokov/cc-1c-skills`, `Dach-Coin/rlm-tools-bsl`, and `itrous/bsl-analyzer` remain existing donors or bundled tools, not new discoveries.
- Keep one public MCP server named `unica`; do not add, rename, or expose tools in this screening PR.
- Temporary clones and evidence logs live only under `.build/issue-186/` and remain untracked.
- Do not create follow-up GitHub issues in this PR; the final synthesis slice owns that action.
- Use short paraphrases and links to exact upstream files; do not quote or reproduce substantial upstream text.

---

## File Structure

- Create `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
  - Owns methodology, 19 detailed cards, summary registry, cross-source normalization, and thematic shortlist.
- Modify `docs/plans/2026-08-03-issue-186-source-screening.md`
  - Check off completed steps during execution.
- Read, do not modify:
  - `plugins/unica/third-party/tools.lock.json`
  - `spec/provenance/skill-upstreams.json`
  - `plugins/unica/ATTRIBUTIONS.md`
  - `spec/architecture/invariants.md`
  - `spec/architecture/quality-requirements.md`
  - `spec/architecture/runtime.md`
  - `spec/decisions/0017-provider-neutral-code-intelligence.md`
  - `spec/decisions/0018-worktree-scoped-provider-state.md`
  - `crates/unica-coder/src/application/code_intelligence.rs`
  - `crates/unica-coder/src/domain/code_intelligence.rs`
  - `crates/unica-coder/src/infrastructure/code_intelligence.rs`
  - `crates/unica-coder/src/infrastructure/workspace_index.rs`
  - `crates/unica-coder/src/infrastructure/workspace_services.rs`
  - `crates/unica-coder/src/infrastructure/native_operations/form.rs`
  - `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`
  - `crates/unica-coder/src/infrastructure/native_operations/help.rs`
  - `scripts/ci/release-assessment.py`
  - `tests/ci/test_release_assessment.py`

No new test, schema, helper script, provenance manifest, ADR, or architecture-registry entry is created. The review is a dated research artifact, not a stable machine-readable contract.

---

### Task 1: Create the Evidence Workspace and Review Skeleton

**Files:**
- Create: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Temporary, untracked: `.build/issue-186/`

**Interfaces:**
- Produces: one fixed Markdown card schema used unchanged by Tasks 2–5.
- Produces: an inventory of exactly 19 canonical repository identifiers.
- Produces: `.build/issue-186/snapshots.jsonl` containing acquisition metadata for research use only.
- Produces: explicit `draft` and `strong-model-reviewed` review states.

- [x] **Step 1: Verify the branch and clean tracked state**

Run:

```powershell
git branch --show-current
git status --short
```

Expected: branch is `codex/issue-186-screening`; only deliberate plan-tracking changes may be present.

- [x] **Step 2: Create the ignored evidence workspace**

Run:

```powershell
New-Item -ItemType Directory -Force '.build\issue-186\sources' | Out-Null
git check-ignore '.build/issue-186/sources'
```

Expected: `git check-ignore` prints `.build/issue-186/sources`; stop if it is not ignored.

- [x] **Step 3: Resolve immutable snapshots for all sources**

Use this exact inventory:

```powershell
$issue186Repositories = @(
  'SteelMorgan/1c-agent-based-dev-framework',
  'comol/ai_rules_1c',
  'AndreevED/1c-ai-feature-dev-workflow',
  'rmartynenko/workflow-dev-1c-claude-code',
  'Pradushkoai/1c-ai-dev-env',
  'Arman-Kudaibergenov/1c-ai-development-kit',
  'Menestre1/reasoning-bank-poc',
  'vgtitov/bsl-ai-toolkit',
  'Regsorm/code-index-mcp',
  'Arman-Kudaibergenov/bsl-atlas',
  'feenlace/mcp-1c',
  'DitriXNew/EDT-MCP',
  'Desko77/1c-formsserver',
  'alexiosus/mxl-merge-tool',
  'rzateev/onec-help-mcp',
  'mussolene/1c_hbk_bsl',
  'genlab-1c/prism',
  'comol/1CLLMBenchTasks',
  'alonehobo/1c-trusted-gateway'
)

$snapshotRows = foreach ($issue186Repository in $issue186Repositories) {
  $repositoryMetadata = gh api "repos/$issue186Repository" | ConvertFrom-Json
  $defaultBranch = $repositoryMetadata.default_branch
  $commitMetadata = gh api "repos/$issue186Repository/commits/$defaultBranch" | ConvertFrom-Json
  [ordered]@{
    repository = $issue186Repository
    canonicalUrl = $repositoryMetadata.html_url
    defaultBranch = $defaultBranch
    commit = $commitMetadata.sha
    snapshotDate = '2026-08-03'
    archived = $repositoryMetadata.archived
    githubLicense = $repositoryMetadata.license.spdx_id
  } | ConvertTo-Json -Compress
}
$snapshotRows | Set-Content -Encoding UTF8 '.build\issue-186\snapshots.jsonl'
Get-Content -Encoding UTF8 '.build\issue-186\snapshots.jsonl'
```

Run GitHub network commands outside the sandbox as required by AGENTS.md. If a repository cannot be resolved, append a JSON line with its identifier, `snapshotDate`, and exact error instead of removing it from the inventory.

Expected: 19 JSON lines, each with either a 40-character commit SHA or an explicit acquisition error.

- [x] **Step 4: Materialize exact source trees for file-level inspection**

For each successful snapshot, clone without checkout and detach at the recorded SHA:

```powershell
$snapshotObjects = Get-Content -Encoding UTF8 '.build\issue-186\snapshots.jsonl' | ForEach-Object { $_ | ConvertFrom-Json }
foreach ($snapshotObject in $snapshotObjects | Where-Object { $_.commit }) {
  $sourceName = $snapshotObject.repository.Replace('/', '--')
  $sourcePath = Join-Path '.build\issue-186\sources' $sourceName
  git clone --filter=blob:none --no-checkout $snapshotObject.canonicalUrl $sourcePath
  git -C $sourcePath fetch origin $snapshotObject.commit --depth 1
  git -C $sourcePath checkout --detach $snapshotObject.commit
  git -C $sourcePath rev-parse HEAD
}
```

Expected: every printed HEAD equals the corresponding recorded SHA. Do not add any clone to Git.

- [x] **Step 5: Create the review skeleton**

Create `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md` with these sections in order:

```markdown
# Source screening for issue #186

- Snapshot date: `2026-08-03`
- Scope: screening only; no external materials transferred
- Governing design: `docs/design/2026-08-03-issue-186-research-slicing-design.md`

## Method
## Decision vocabulary
## Summary registry
## Workflow, skills, agents, and context management
## Code intelligence and alternative indexers
## Specialized implementations
## Evaluation and safe access
## Existing Unica donors and bundled engines
## Cross-source normalization
## Thematic shortlist
## Deferred deep-dive protocol
```

Under `## Decision vocabulary`, define exactly:

- `deep-dive`: evidence shows a potentially useful mechanism and a concrete Unica gap or a comparison question that cannot be answered by screening;
- `defer`: the mechanism may be relevant, but current evidence, maturity, licensing, or cost does not justify a deep dive now;
- `reject`: the mechanism is duplicated, unsupported by source evidence, incompatible with non-negotiable Unica contracts, or disproportionately costly.

Under `## Method`, record the model boundary:

- `gpt-5.6-luna` may collect and paraphrase pinned evidence into draft cards;
- draft findings and provisional dispositions are not final recommendations;
- a stronger reasoning model reopens the evidence for all 19 cards, compares them across sources, verifies Unica mappings, and owns final decisions and shortlist;
- unavailable Luna means the stronger model performs both passes, not that the review pass is skipped.

Use this exact card shape for all 19 sources:

```markdown
### `owner/repository`

- **Snapshot:** default branch, full commit SHA, `2026-08-03`.
- **License:** file paths, detected license, GitHub metadata, contradictions.
- **Evidence:** links to exact files at the pinned commit and any bounded probe.
- **Mechanism:** concise source-backed behavior.
- **Unica mapping:** exact local code/test/contract paths and the observed gap or overlap.
- **Limits:** unsupported claims, missing evidence, maturity, portability, or legal constraints.
- **Provisional decision:** `deep-dive`, `defer`, or `reject` — draft reason.
- **Review:** `draft` until cross-source review; then `strong-model-reviewed` with the reviewing model and reviewed evidence.
- **Decision:** absent while `draft`; after review, `deep-dive`, `defer`, or `reject` — final reason.
```

- [x] **Step 6: Verify skeleton and inventory invariants**

Run:

```powershell
(Get-Content -Encoding UTF8 '.build\issue-186\snapshots.jsonl').Count
rg -n '^## |^### ' 'docs/provenance/reviews/2026-08-03-issue-186-source-screening.md'
git status --short
```

Expected: snapshot count is 19; all top-level sections exist; `.build/issue-186/` is absent from `git status`.

- [x] **Step 7: Commit the methodology and skeleton**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): define issue 186 screening method"
```

---

### Task 2: Screen Workflow, Skills, Agents, and Context Management Sources

**Files:**
- Modify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Read: `spec/provenance/skill-upstreams.json`
- Read: `plugins/unica/ATTRIBUTIONS.md`
- Read: `plugins/unica/skills/*/SKILL.md`
- Read: `tests/ci/test_unica_skills.py`

**Interfaces:**
- Consumes: snapshot metadata and card schema from Task 1.
- Produces: eight draft source cards; Task 6 owns their final decisions and summary rows.
- Produces: an explicit reconciliation of `comol/ai_rules_1c` with its existing inspiration-only provenance.

- [x] **Step 1: Inspect the eight pinned trees**

Inspect exactly:

```text
SteelMorgan/1c-agent-based-dev-framework
comol/ai_rules_1c
AndreevED/1c-ai-feature-dev-workflow
rmartynenko/workflow-dev-1c-claude-code
Pradushkoai/1c-ai-dev-env
Arman-Kudaibergenov/1c-ai-development-kit
Menestre1/reasoning-bank-poc
vgtitov/bsl-ai-toolkit
```

For each pinned tree, run:

```powershell
git -C <exact-source-path> ls-tree -r --name-only HEAD | rg '(^|/)(LICENSE|NOTICE|COPYRIGHT|README|package\.json|pyproject\.toml|Cargo\.toml|\.mcp\.json|plugin\.json|SKILL\.md|agents?|commands?|rules?|workflows?|tests?|core|context|memory|session)'
```

Replace `<exact-source-path>` with the concrete path under `.build/issue-186/sources/` for that repository. Read the matched implementation, test, configuration, and licensing files at the detached HEAD; do not infer behavior from filenames alone.

- [x] **Step 2: Map workflow mechanisms to live Unica evidence**

For every claimed mechanism, search Unica before declaring a gap:

```powershell
rg -n --glob '!target/**' --glob '!.build/**' --glob '!dist/**' --glob '!docs-local/**' --glob '!docs/design/**' --glob '!docs/plans/**' 'discovery|context|session|memory|retrospective|verification|subagent|workflow|skill|provenance|upstream' plugins/unica spec tests/ci crates
```

At minimum reconcile `comol/ai_rules_1c` against:

```text
spec/provenance/skill-upstreams.json
plugins/unica/ATTRIBUTIONS.md
docs/provenance/reviews/2026-07-22-ai-rules-idea-provenance-correction.json
tests/ci/test_skill_provenance.py
```

Record whether each source adds a mechanism, duplicates a current process, or only offers prose without enforceable evidence.

- [x] **Step 3: Write all eight draft cards**

`gpt-5.6-luna` may execute this evidence-extraction step when available. Each
draft card must cite at least:

- one pinned implementation/configuration path;
- one pinned test or an explicit statement that no relevant test exists;
- license-file evidence and GitHub metadata;
- one exact Unica code, test, package-contract, or spec path;
- a provisional decision with a falsifiable reason;
- `Review: draft`.

For `Menestre1/reasoning-bank-poc`, explicitly resolve the issue's preliminary ISC claim against the pinned tree. For `vgtitov/bsl-ai-toolkit`, separate proven open-source behavior from unverified byte-perfect, round-trip, RLS, masking, or paid capability claims.

- [x] **Step 4: Verify cohort completeness**

Run:

```powershell
$workflowSources = @('SteelMorgan/1c-agent-based-dev-framework','comol/ai_rules_1c','AndreevED/1c-ai-feature-dev-workflow','rmartynenko/workflow-dev-1c-claude-code','Pradushkoai/1c-ai-dev-env','Arman-Kudaibergenov/1c-ai-development-kit','Menestre1/reasoning-bank-poc','vgtitov/bsl-ai-toolkit')
$reviewText = Get-Content -Raw -Encoding UTF8 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
$workflowSources | ForEach-Object { if (($reviewText.Split("### `$_`").Count - 1) -ne 1) { throw "missing or duplicate workflow card: $_" } }
```

Expected: no exception; every workflow source appears as exactly one detailed card.

- [x] **Step 5: Commit the workflow cohort**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): screen issue 186 workflow sources"
```

---

### Task 3: Screen Alternative Code-Intelligence Sources

**Files:**
- Modify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Read: `plugins/unica/third-party/tools.lock.json`
- Read: `spec/decisions/0017-provider-neutral-code-intelligence.md`
- Read: `spec/decisions/0018-worktree-scoped-provider-state.md`
- Read: `crates/unica-coder/src/domain/code_intelligence.rs`
- Read: `crates/unica-coder/src/application/code_intelligence.rs`
- Read: `crates/unica-coder/src/infrastructure/code_intelligence.rs`
- Read: `crates/unica-coder/src/infrastructure/workspace_index.rs`

**Interfaces:**
- Consumes: snapshot metadata and card schema from Task 1.
- Produces: draft cards for `Regsorm/code-index-mcp` and `Arman-Kudaibergenov/bsl-atlas`.
- Produces: a bounded-experiment hypothesis list for the later code-intelligence slice, without running the experiment here.

- [x] **Step 1: Inspect both pinned implementations and tests**

For each source, locate and read evidence for index schema, writer ownership, update/invalidation, query semantics, multi-root behavior, result completeness, cancellation, concurrency, and corruption handling:

```powershell
git -C '.build\issue-186\sources\Regsorm--code-index-mcp' ls-tree -r --name-only HEAD | rg 'Cargo\.toml|src/|tests?/|migrations?/|schema|sqlite|fts|daemon|writer|federat|LICENSE|NOTICE|README'
git -C '.build\issue-186\sources\Arman-Kudaibergenov--bsl-atlas' ls-tree -r --name-only HEAD | rg 'Cargo\.toml|package\.json|pyproject\.toml|src/|tests?/|index|vector|semantic|graph|LICENSE|NOTICE|README'
```

Do not treat an advertised semantic/vector mode as implemented unless the pinned code or tests exercise it.

`gpt-5.6-luna` may perform this pinned-evidence extraction when available, but
its Unica-gap statement and disposition remain provisional until Task 6.

- [x] **Step 2: Map each mechanism to the provider-neutral Unica boundary**

Read the exact Unica sources listed in this task and answer in each card:

- Can the mechanism live behind `CodeIntelligenceProvider` without exposing another MCP server?
- Does it duplicate current `rlm`, `bsl-analyzer`, or `git-grep` behavior?
- Does it improve current completeness epistemics, invalidation, federation, or resource behavior?
- Which questions require a common fixture experiment rather than screening evidence?

Record the currently pinned versions and SHAs of `bsl-analyzer`, `rlm-tools-bsl`, and `rlm-bsl-index` from `plugins/unica/third-party/tools.lock.json`; label them existing baselines, not new candidates.

- [x] **Step 3: Write both draft cards and experiment hypotheses**

Under `## Deferred deep-dive protocol`, add only hypotheses that can later be measured on a common harness, including:

```text
exact-symbol and ambiguous-symbol completeness
reported truncation and lower bounds
incremental add/change/delete behavior
cache identity and invalidation after rename
multi-root and extension topology
cold/warm latency and index size
cancellation and concurrent readers
partial, stale, unavailable, and corrupted-index outcomes
```

Do not record performance numbers in this PR unless they arise from a tiny screening probe clearly marked non-comparative.

- [x] **Step 4: Verify both cards and existing-engine classification**

Run:

```powershell
$reviewText = Get-Content -Raw -Encoding UTF8 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
@('Regsorm/code-index-mcp','Arman-Kudaibergenov/bsl-atlas') | ForEach-Object { $n = ([regex]::Matches($reviewText, [regex]::Escape(('### `' + $_ + '`')))).Count; if ($n -ne 1) { throw "missing or duplicate code-intelligence card: $_" } }
@('bsl-analyzer','rlm-tools-bsl','rlm-bsl-index') | ForEach-Object { if ($reviewText -notmatch [regex]::Escape($_)) { throw "missing existing-engine classification: $_" } }
```

Expected: no exception.

- [x] **Step 5: Commit the code-intelligence cohort**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): screen issue 186 code intelligence sources"
```

---

### Task 4: Screen Specialized Implementations

**Files:**
- Modify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Read: `crates/unica-coder/src/infrastructure/native_operations/form.rs`
- Read: `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`
- Read: `crates/unica-coder/src/infrastructure/native_operations/help.rs`
- Read: `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`
- Read: `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`
- Read: `spec/architecture/invariants.md`
- Read: `spec/architecture/quality-requirements.md`

**Interfaces:**
- Consumes: snapshot metadata and card schema from Task 1.
- Produces: six draft specialized-source cards; Task 6 owns final decisions and summary rows.
- Produces: a separation between live-data/EDT/safety mechanisms and artifact/help mechanisms.

- [x] **Step 1: Inspect all six pinned trees**

Inspect exactly:

```text
feenlace/mcp-1c
DitriXNew/EDT-MCP
Desko77/1c-formsserver
alexiosus/mxl-merge-tool
rzateev/onec-help-mcp
mussolene/1c_hbk_bsl
```

For each source, inspect implementation and tests for the issue-specific claims:

- `mcp-1c`: live-base transport, read-only enforcement, event log, offline BM25, installation, cache invalidation, and open/paid boundary;
- `EDT-MCP`: live workspace, diagnostics, completion, query validation, refactoring, cancellation, progressive disclosure, and headless CI;
- `1c-formsserver`: supported form models, schema, generation, validation, and actual round-trip tests;
- `mxl-merge-tool`: semantic diff, three-way merge, merge driver, conflict representation, validation, and reports;
- `onec-help-mcp`: HBK extraction/search and the README badge versus LICENSE contradiction;
- `1c_hbk_bsl`: diagnostics, formatter, CLI/LSP/MCP, SARIF, indexing modes, tests, and RU/EN documentation.

`gpt-5.6-luna` may fill these evidence-backed draft cards when available. Mark
every card `Review: draft`; do not promote architectural compatibility claims
to final findings in this task.

Use `git ls-tree`, `git show HEAD:<path>`, and bounded local commands declared by the pinned project. Do not install globally or connect to a real 1C infobase during screening.

- [x] **Step 2: Map specialized semantics to Unica guarantees**

Compare source evidence against exact Unica paths and rules for:

```text
one public unica.* boundary
preview by default
support and format guards
atomic/no-partial publication
stable result envelope
secret redaction
workspace-scoped provider state
Form, MXL, help, diagnostics, integration, and log-analysis operations
```

Use `rg` to locate every referenced invariant or operation before writing the mapping:

```powershell
rg -n 'INV-MCP-NO-ENGINE-SERVERS|REQ-SAFETY-PREVIEW-BY-DEFAULT|REQ-SAFETY-NO-PARTIAL-WRITE|REQ-SAFETY-SECRET-REDACTION|REQ-OBS-STABLE-ENVELOPE' spec/architecture
rg -n 'unica\.(form|mxl|help|code|integration|log)' crates/unica-coder/src/application plugins/unica/skills tests/ci
```

- [x] **Step 3: Write six draft cards and provisional theme notes**

For live-data and EDT sources, explicitly distinguish useful internal semantics from incompatible public server/tool names. For forms and MXL, state whether round-trip or merge guarantees are proven by fixtures/tests or merely claimed. For help sources, record extraction legality and license conflicts separately from search quality.

- [x] **Step 4: Verify cohort completeness**

Run:

```powershell
$specializedSources = @('feenlace/mcp-1c','DitriXNew/EDT-MCP','Desko77/1c-formsserver','alexiosus/mxl-merge-tool','rzateev/onec-help-mcp','mussolene/1c_hbk_bsl')
$reviewText = Get-Content -Raw -Encoding UTF8 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
$specializedSources | ForEach-Object { if (($reviewText.Split("### `$_`").Count - 1) -ne 1) { throw "missing or duplicate specialized card: $_" } }
```

Expected: no exception.

- [x] **Step 5: Commit the specialized cohort**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): screen issue 186 specialized sources"
```

---

### Task 5: Screen Evaluation and Safe-Access Sources

**Files:**
- Modify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Read: `scripts/ci/release-assessment.py`
- Read: `tests/ci/test_release_assessment.py`
- Read: `tests/fixtures/unica_mcp_script_parity/`
- Read: `spec/architecture/quality-requirements.md`

**Interfaces:**
- Consumes: snapshot metadata and card schema from Task 1.
- Produces: three draft source cards for benchmark and trusted-access candidates.
- Produces: explicit separation between executable oracle evidence and subjective model judging.

- [x] **Step 1: Inspect the three pinned trees**

Inspect exactly:

```text
genlab-1c/prism
comol/1CLLMBenchTasks
alonehobo/1c-trusted-gateway
```

For `prism` and `1CLLMBenchTasks`, inspect task definitions, prompt construction, expected answers, executors, fixtures, scoring, model adapters, and CI. For `1c-trusted-gateway`, inspect enforcement code and tests for masking, whitelist/policy behavior, approvals, MCP proxying, auditability, and fail-open/fail-closed outcomes.

`gpt-5.6-luna` may perform the structured evidence pass when available. Leakage,
oracle quality, safety classification, and disposition remain provisional until
the stronger-model cross-source review in Task 6.

- [x] **Step 2: Check leakage and oracle quality**

For each benchmark source, answer with pinned file evidence:

- Can expected answers enter the model prompt or context?
- Is the oracle executable, deterministic, and independent of the evaluated model?
- Does evaluation run BSL or platform behavior, or only compare text/LLM scores?
- Are XML, forms, MXL, DCS, roles, integrations, or complete artifacts covered?
- Can the same case compare skills, schemas, providers, and models without changing the oracle?

For missing licenses in `1CLLMBenchTasks` and `1c-trusted-gateway`, state the exact pinned-tree result and GitHub metadata; do not infer permission from public visibility.

- [x] **Step 3: Map to current Unica evaluation and safety evidence**

Compare benchmark mechanisms with `scripts/ci/release-assessment.py`, `tests/ci/test_release_assessment.py`, and existing parity fixtures. Compare gateway mechanisms with `REQ-SAFETY-SECRET-REDACTION`, preview-by-default, support locks, and no-partial-write requirements.

Write each gap narrowly: screening may recommend a later experiment, but it must not claim that a benchmark improves Unica without an executable comparative result.

- [x] **Step 4: Write three draft cards and verify completeness**

Run after writing:

```powershell
$evaluationSources = @('genlab-1c/prism','comol/1CLLMBenchTasks','alonehobo/1c-trusted-gateway')
$reviewText = Get-Content -Raw -Encoding UTF8 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
$evaluationSources | ForEach-Object { if (($reviewText.Split("### `$_`").Count - 1) -ne 1) { throw "missing or duplicate evaluation/safety card: $_" } }
```

Expected: no exception.

- [x] **Step 5: Commit the evaluation and safety cohort**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): screen issue 186 evaluation sources"
```

---

### Task 6: Normalize Decisions and Produce the Thematic Shortlist

**Files:**
- Modify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`

**Interfaces:**
- Consumes: all 19 source cards from Tasks 2–5.
- Produces: a complete summary registry with one row per source.
- Produces: cross-source duplicate/conflict findings and a thematic shortlist for later deep dives.
- Produces: no GitHub issues and no implementation recommendation stronger than screening evidence supports.
- Requires: `gpt-5.6-sol` or the strongest available reasoning successor for the entire task; `gpt-5.6-luna` is not sufficient for this gate.

- [ ] **Step 1: Reopen and cross-check all 19 draft cards with a stronger reasoning model**

For every card, the reviewing model must reopen at least:

```text
the pinned implementation/config/package evidence
the pinned test or explicit absence evidence
the license file and GitHub license metadata
the exact Unica code/test/contract path used for mapping
```

Compare each mechanism with all other cards in its cohort before accepting the
gap or disposition. Correct unsupported claims inline. Set
`Review: strong-model-reviewed` and name the reviewing model, for example
`gpt-5.6-sol`, only after those checks pass. A sampling review is not
sufficient.

- [ ] **Step 2: Build the summary registry from the reviewed cards**

Use these columns:

```markdown
| Source | SHA | License | Proven mechanism | Unica overlap/gap | Decision |
| --- | --- | --- | --- | --- | --- |
```

Every row must agree with a `strong-model-reviewed` detailed card. Use full SHAs in detailed cards; the summary may use a linked 12-character prefix if the link resolves to the exact commit. A draft card must not receive a summary row.

- [ ] **Step 3: Normalize decisions across cohorts**

Re-read every `Decision` and apply the same threshold:

- keep `deep-dive` only when primary evidence proves a mechanism and a concrete comparison or Unica gap remains;
- use `defer` when the idea is plausible but evidence, maturity, legal clarity, or evaluation cost is insufficient;
- use `reject` for duplicates, unsupported claims, incompatible assumptions, or disproportionate cost.

When two sources offer the same mechanism, select the stronger evidence candidate for deep dive and explicitly record the other as duplicate, complementary, or deferred. Do not let the order of research determine priority.

- [ ] **Step 4: Record mandatory duplicate and boundary findings**

The review must explicitly state:

- why `cc-1c-skills` is an existing tracked donor rather than a new source;
- the exact locked versions and SHAs proving `bsl-analyzer` and `rlm-tools-bsl` are current bundled baselines;
- why an external MCP server name is not itself adoptable, while its internal semantics may be adapted behind `unica.*`;
- which claims remain unverified until a common bounded experiment;
- which license conflicts block transfer even if behavioral observation is useful.

- [ ] **Step 5: Produce a thematic shortlist, not 19 follow-up proposals**

Group surviving questions under the later slices approved by the design:

```text
workflow, skills, and context management
code intelligence
live environments, data, and safety
artifacts and documentation
benchmark and evaluation
```

For each theme, list candidate sources, the exact unanswered question, required evidence, and a minimal deep-dive boundary. Do not select the final 3–5 product experiments; issue #186's final synthesis owns that choice.

- [ ] **Step 6: Check review-gate and summary/detail consistency**

Run:

```powershell
$reviewPath = 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
$reviewText = Get-Content -Raw -Encoding UTF8 $reviewPath
$cardCount = ([regex]::Matches($reviewText, '(?m)^### `[^`]+/[^`]+`$')).Count
$reviewedCount = ([regex]::Matches($reviewText, '(?m)^- \*\*Review:\*\* `strong-model-reviewed`')).Count
$decisionCount = ([regex]::Matches($reviewText, '(?m)^- \*\*Decision:\*\* `(deep-dive|defer|reject)`')).Count
if ($cardCount -ne 19) { throw "expected 19 cards, found $cardCount" }
if ($reviewedCount -ne 19) { throw "expected 19 stronger-model reviews, found $reviewedCount" }
if ($decisionCount -ne 19) { throw "expected 19 card decisions, found $decisionCount" }
```

Expected: 19 cards, 19 stronger-model reviews, and 19 final decisions.

- [ ] **Step 7: Commit normalized decisions and shortlist**

```powershell
git add docs/provenance/reviews/2026-08-03-issue-186-source-screening.md docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): shortlist issue 186 deep dives"
```

---

### Task 7: Complete Screening Verification

**Files:**
- Verify: `docs/provenance/reviews/2026-08-03-issue-186-source-screening.md`
- Modify: `docs/plans/2026-08-03-issue-186-source-screening.md`
- Verify unchanged: `plugins/unica/third-party/tools.lock.json`
- Verify unchanged: `spec/provenance/skill-upstreams.json`
- Verify unchanged: `spec/architecture/`
- Verify untracked/ignored: `.build/issue-186/`

**Interfaces:**
- Consumes: the completed review from Tasks 1–6.
- Produces: a verified screening PR that is independently reviewable and ready for publication.

- [ ] **Step 1: Run the exact source-coverage check**

```powershell
$expectedSources = @(
  'SteelMorgan/1c-agent-based-dev-framework','comol/ai_rules_1c','AndreevED/1c-ai-feature-dev-workflow','rmartynenko/workflow-dev-1c-claude-code','Pradushkoai/1c-ai-dev-env','Arman-Kudaibergenov/1c-ai-development-kit','Menestre1/reasoning-bank-poc','vgtitov/bsl-ai-toolkit','Regsorm/code-index-mcp','Arman-Kudaibergenov/bsl-atlas','feenlace/mcp-1c','DitriXNew/EDT-MCP','Desko77/1c-formsserver','alexiosus/mxl-merge-tool','rzateev/onec-help-mcp','mussolene/1c_hbk_bsl','genlab-1c/prism','comol/1CLLMBenchTasks','alonehobo/1c-trusted-gateway'
)
$reviewText = Get-Content -Raw -Encoding UTF8 'docs\provenance\reviews\2026-08-03-issue-186-source-screening.md'
foreach ($expectedSource in $expectedSources) {
  $occurrences = $reviewText.Split("### `$expectedSource`").Count - 1
  if ($occurrences -ne 1) { throw "$expectedSource card count is $occurrences" }
}
```

Expected: no exception.

- [ ] **Step 2: Check immutable snapshots and primary evidence**

Manually verify each card has:

```text
full 40-character SHA or explicit acquisition failure
default branch and snapshot date
license file result plus GitHub metadata
implementation/config evidence
test evidence or an explicit absence statement
at least one exact live Unica path
one limits statement
`strong-model-reviewed` quality-gate marker
one normalized decision
```

Open every link to a pinned GitHub `blob/<full-sha>/...` or `tree/<full-sha>/...` target. Replace branch-based links before completion.

- [ ] **Step 3: Check that no external material entered tracked files**

Run:

```powershell
git status --short
git diff --name-only upstream/main...HEAD
git check-ignore '.build/issue-186/snapshots.jsonl'
git diff --exit-code upstream/main...HEAD -- plugins/unica/third-party/tools.lock.json spec/provenance/skill-upstreams.json spec/architecture crates tests scripts
```

Expected:

- tracked changes are limited to the approved design, this plan, and the screening review;
- `.build/issue-186/snapshots.jsonl` is ignored;
- code, tests, package contracts, provenance manifest, and architecture are unchanged.

- [ ] **Step 4: Run repository documentation checks**

```powershell
python -m pytest tests/ci/test_design_documents.py -q
git diff --check
```

Expected: all design-document tests pass and no whitespace errors are reported.

- [ ] **Step 5: Review for placeholders, unsupported certainty, and copied text**

Run:

```powershell
rg -n 'TBD|TODO|FIXME|\?\?\?|probably|apparently|seems to' docs/provenance/reviews/2026-08-03-issue-186-source-screening.md
```

Expected: no placeholders. For any uncertainty, replace vague wording with the exact missing evidence or bounded observation. Confirm that upstream descriptions are paraphrased and linked rather than reproduced.

- [ ] **Step 6: Inspect the complete branch diff**

```powershell
git diff --stat upstream/main...HEAD
git diff --check upstream/main...HEAD
git log --oneline upstream/main..HEAD
```

Expected: a sequence of focused research commits, no runtime or package-contract changes, and a self-contained review covering all 19 sources.

- [ ] **Step 7: Commit completed plan tracking if needed**

```powershell
git add docs/plans/2026-08-03-issue-186-source-screening.md
git commit -m "docs(research): complete issue 186 screening plan"
```
