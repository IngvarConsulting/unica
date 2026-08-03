# Source screening for issue #186

- Snapshot date: `2026-08-03`
- Scope: screening only; no external materials transferred
- Governing design: `docs/design/2026-08-03-issue-186-research-slicing-design.md`

## Method

This review screens exactly the 19 repositories listed below using immutable
default-branch snapshots recorded in the ignored evidence workspace
`.build/issue-186/snapshots.jsonl`. `gpt-5.6-luna` may collect and paraphrase
pinned evidence into draft cards when available. Draft findings and provisional
dispositions are not final recommendations. A stronger reasoning model must
reopen all 19 cards, compare them cross-source, verify Unica mappings, and own
final decisions and the shortlist. If Luna is unavailable, the stronger model
performs both passes; the review gate is never skipped.

The exact inventory is:

```text
SteelMorgan/1c-agent-based-dev-framework
comol/ai_rules_1c
AndreevED/1c-ai-feature-dev-workflow
rmartynenko/workflow-dev-1c-claude-code
Pradushkoai/1c-ai-dev-env
Arman-Kudaibergenov/1c-ai-development-kit
Menestre1/reasoning-bank-poc
vgtitov/bsl-ai-toolkit
Regsorm/code-index-mcp
Arman-Kudaibergenov/bsl-atlas
feenlace/mcp-1c
DitriXNew/EDT-MCP
Desko77/1c-formsserver
alexiosus/mxl-merge-tool
rzateev/onec-help-mcp
mussolene/1c_hbk_bsl
genlab-1c/prism
comol/1CLLMBenchTasks
alonehobo/1c-trusted-gateway
```

All source cards use this unchanged shape:

```markdown
### Card schema: `<owner>/<repository>`

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

## Decision vocabulary

- `deep-dive`: evidence shows a potentially useful mechanism and a concrete Unica gap or comparison question that screening cannot answer.
- `defer`: the mechanism may be relevant, but current evidence, maturity, licensing, or cost does not justify a deep dive now.
- `reject`: the mechanism is duplicated, unsupported by source evidence, incompatible with non-negotiable Unica contracts, or disproportionately costly.

## Summary registry

| Source | SHA | License | Proven mechanism | Unica overlap/gap | Decision |
| --- | --- | --- | --- | --- | --- |
| [SteelMorgan/1c-agent-based-dev-framework](#steelmorgan1c-agent-based-dev-framework) | [`19f67bfed6d1`](https://github.com/SteelMorgan/1c-agent-based-dev-framework/commit/19f67bfed6d15f051f9568678bab1701b7735f95) | No repository license | Rule/skill bundle, installer selection and context-budget tests | Workflow guidance duplicates local process; no implemented compaction protocol | `reject` |
| [comol/ai_rules_1c](#comolai_rules_1c) | [`410951e74fd3`](https://github.com/comol/ai_rules_1c/commit/410951e74fd3e6b7a763cf49757935b9a34d3f31) | None published | Host adapters and structural validation | Already tracked as inspiration-only provenance | `reject` |
| [AndreevED/1c-ai-feature-dev-workflow](#andreeved1c-ai-feature-dev-workflow) | [`c67108acb534`](https://github.com/AndreevED/1c-ai-feature-dev-workflow/commit/c67108acb534e18e6e539f27b7991f7497dcc539) | MIT | Artifact-based feature phases and role prompts | Prose workflow duplicates local planning/review gates | `reject` |
| [rmartynenko/workflow-dev-1c-claude-code](#rmartynenkoworkflow-dev-1c-claude-code) | [`afde2fd1f7cc`](https://github.com/rmartynenko/workflow-dev-1c-claude-code/commit/afde2fd1f7cc419906a10ea53ee556332535a72b) | MIT | Manual session and memory-bank protocol | Human-readable continuity remains plausible but untested | `defer` |
| [Pradushkoai/1c-ai-dev-env](#pradushkoai1c-ai-dev-env) | [`32a3adeaffc1`](https://github.com/Pradushkoai/1c-ai-dev-env/commit/32a3adeaffc168301fd608a6c6984df633c9b8ad) | MIT | Tested BM25/vector fallback and typed tool catalogue | Complements but duplicates current engines and stronger index candidates | `defer` |
| [Arman-Kudaibergenov/1c-ai-development-kit](#arman-kudaibergenov1c-ai-development-kit) | [`92d389edfb7a`](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/commit/92d389edfb7a13c0799065e3865b9488ce019f2d) | AGPL-3.0 | Large host-specific skill and live-base command catalogue | Untested external-MCP workflow conflicts with the public boundary and duplicates local skills | `reject` |
| [Menestre1/reasoning-bank-poc](#menestre1reasoning-bank-poc) | [`30f6b52dbc7c`](https://github.com/Menestre1/reasoning-bank-poc/commit/30f6b52dbc7c2c54049421a0ce696b1b124f2f78) | ISC in package only | Tested SQLite experience retrieval, feedback promotion and isolation | Durable reasoning policy is absent; transfer remains blocked | `deep-dive` |
| [vgtitov/bsl-ai-toolkit](#vgtitovbsl-ai-toolkit) | [`79bfde552f7e`](https://github.com/vgtitov/bsl-ai-toolkit/commit/79bfde552f7e5acf96c80ca831817f24a3f9b9ce) | MIT plus notices | Tested layer filtering and lexical XML-style preservation | Full platform round trip and safety breadth remain unproved | `defer` |
| [Regsorm/code-index-mcp](#regsormcode-index-mcp) | [`9614acd9048f`](https://github.com/Regsorm/code-index-mcp/commit/9614acd9048f73fbc4379d5fd240dc457c8d957a) | MIT | Incremental SQLite index, daemon state and bounded result metadata | Strongest candidate for a common provider harness | `deep-dive` |
| [Arman-Kudaibergenov/bsl-atlas](#arman-kudaibergenovbsl-atlas) | [`b605768692ea`](https://github.com/Arman-Kudaibergenov/bsl-atlas/commit/b605768692ea2e51c3dfb199b788f6f4d2fb6325) | AGPL-3.0 / commercial alternative | Structural graph plus optional vector index | Complementary graph semantics, but legal and service cost dominate now | `defer` |
| [feenlace/mcp-1c](#feenlacemcp-1c) | [`926af4af57eb`](https://github.com/feenlace/mcp-1c/commit/926af4af57eb4c6c2a95a1b13ac269b0f7debe78) | MIT | Read-only query gate and generation-aware searchable cache | Strong internal cache/safety comparison; live-base behavior unproved | `deep-dive` |
| [DitriXNew/EDT-MCP](#ditrixnewedt-mcp) | [`d2f29efc520c`](https://github.com/DitriXNew/EDT-MCP/commit/d2f29efc520ce373e637e61ec708073de0540bba) | AGPL-3.0 | Tool grouping, cancellation state and headless EDT CI wiring | EDT/AGPL cost and duplicate diagnostics postpone comparison | `defer` |
| [Desko77/1c-formsserver](#desko771c-formsserver) | [`cd3f56e3508a`](https://github.com/Desko77/1c-formsserver/commit/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a) | MIT | Form schema, conversion, validation and fixture round trips | Common fixtures can test fidelity against native form operations | `deep-dive` |
| [alexiosus/mxl-merge-tool](#alexiosusmxl-merge-tool) | [`83839e91685f`](https://github.com/alexiosus/mxl-merge-tool/commit/83839e91685f743be203458194e1c11bc1ddd1fa) | MIT | Semantic three-way MXL merge and tested Git driver | Unica has MXL writers but no evidenced semantic merge contract | `deep-dive` |
| [rzateev/onec-help-mcp](#rzateevonec-help-mcp) | [`f66860b45eca`](https://github.com/rzateev/onec-help-mcp/commit/f66860b45ecad40e071ac4abe3f7ef432b30ac24) | MIT file; contradictory badge | HBK parser and hybrid-search implementation | No representative HBK/search test and unresolved label conflict | `defer` |
| [mussolene/1c_hbk_bsl](#mussolene1c_hbk_bsl) | [`cee853014d57`](https://github.com/mussolene/1c_hbk_bsl/commit/cee853014d57e34950c86ba24957af5ecc3e6d49) | MIT; LGPL diagnostic prose | Tested diagnostics, SARIF, formatter, LSP/MCP and indexing components | Mostly complements bundled analyzer; full protocol parity is unproved | `defer` |
| [genlab-1c/prism](#genlab-1cprism) | [`6adda50c572a`](https://github.com/genlab-1c/prism/commit/6adda50c572a28ca2f915b64bc89c667abf93ea3) | MIT | Tested prompt separation, runner classification and L1 scoring | Executable evaluation shape merits a bounded Unica-fixture comparison | `deep-dive` |
| [comol/1CLLMBenchTasks](#comol1cllmbenchtasks) | [`39732c770965`](https://github.com/comol/1CLLMBenchTasks/commit/39732c7709651bf6628360393cf8fe0e30d96c8c) | None published | Seventeen manual task/answer cards | Breadth is a hypothesis without license, runner or oracle | `defer` |
| [alonehobo/1c-trusted-gateway](#alonehobo1c-trusted-gateway) | [`a5cc656e3f37`](https://github.com/alonehobo/1c-trusted-gateway/commit/a5cc656e3f3763800706ec752fd33fb2e18318e4) | None published | Tested masking/type policy plus observable approval bypasses | Adversarial safety comparison remains; transfer is blocked | `deep-dive` |

## Workflow, skills, agents, and context management

### `SteelMorgan/1c-agent-based-dev-framework`

- **Snapshot:** `main`, `19f67bfed6d15f051f9568678bab1701b7735f95`, `2026-08-03`.
- **License:** no repository-level LICENSE was present and GitHub metadata reports no SPDX license; embedded notices exist at `framework/skills/tool-usage/browser-ui/playwright/LICENSE.txt` and `NOTICE.txt`. This blocks transfer, not observation.
- **Evidence:** [CLAUDE.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/.claude/CLAUDE.md), [context.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/context.md), [sdd.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/sdd.md), [tdd.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/tdd.md), sandbox skill [SKILL.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/.claude/skills/sandbox-framework/SKILL.md), and [tools/test_install_always_apply.py](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/tools/test_install_always_apply.py), which tests installation/rule application and context estimation.
- **Mechanism:** Claude rules and skills describe SDD/TDD phases, role-separated agents, sandbox permissions, session-log mapping, and context-budget estimation; the pinned installer test proves rule selection, `alwaysApply` handling, session-log mapping, and context estimation. `docs/info/context.md` is guidance, not an implemented compaction engine.
- **Unica mapping:** overlaps `plugins/unica/skills/code-review/SKILL.md`, `plugins/unica/skills/code-search/SKILL.md`, `crates/unica-coder/src/infrastructure/workspace.rs`, and `tests/ci/test_unica_skills.py`; no tested reusable compaction or persistence protocol remains as a concrete Unica gap.
- **Limits:** prose-heavy, host-specific Claude layout, no license, and no pinned tests proving the orchestration end to end.
- **Provisional decision:** `defer` — useful workflow patterns are largely represented locally; only bounded context/session experiments remain.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `reject` — duplicated workflow prose, absent end-to-end orchestration evidence, and no repository license do not justify further work.

### `comol/ai_rules_1c`

- **Snapshot:** `main`, `410951e74fd3e6b7a763cf49757935b9a34d3f31`, `2026-08-03`.
- **License:** no LICENSE file and GitHub metadata has no SPDX license; `ATTRIBUTIONS.md` records the same uncertainty and permits only inspiration-only use.
- **Evidence:** [AGENTS.md](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/AGENTS.md), planner [agent](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/content/agents/planner.md), tester [agent](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/content/agents/tester.md), and validation workflow [validate-rules.yml](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/.github/workflows/validate-rules.yml); no runtime test suite was found.
- **Mechanism:** adapter-specific rules, agents, commands, and an OpenSpec bundle provide planning, review, testing, and synchronization across IDE hosts.
- **Unica mapping:** `spec/provenance/skill-upstreams.json`, `plugins/unica/ATTRIBUTIONS.md`, `docs/provenance/reviews/2026-07-22-ai-rules-idea-provenance-correction.json`, `tests/ci/test_skill_provenance.py`, and `tests/ci/test_unica_skills.py` explicitly classify this repository as inspiration-only; it overlaps Unica-owned `code-review`, `code-search`, `test-authoring`, and planning guidance, with no new donor gap established.
- **Limits:** licensing is unresolved; behavior is mostly markdown/adapters, not executable enforcement; adapter synchronization is host-dependent.
- **Provisional decision:** `reject` — retain as existing inspiration-only provenance, not a new donor or transfer candidate.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `reject` — this is already recorded as inspiration-only provenance and supplies no independently tested runtime mechanism.

### `AndreevED/1c-ai-feature-dev-workflow`

- **Snapshot:** `main`, `c67108acb534e18e6e539f27b7991f7497dcc539`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent.
- **Evidence:** [1c-feature-dev/SKILL.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/skills/1c-feature-dev/SKILL.md), [1c-code-reviewer.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/agents/1c-code-reviewer.md), [1c-code-writer.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/agents/1c-code-writer.md), and [LICENSE](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/LICENSE); no automated tests are present.
- **Mechanism:** the feature skill prescribes complexity assessment, requirement clarification, atomic phases, plan review, implementation, and acceptance checks, with dedicated explorer/writer/reviewer agents.
- **Unica mapping:** overlaps `plugins/unica/skills/code-search/SKILL.md`, `plugins/unica/skills/code-review/SKILL.md`, `plugins/unica/skills/test-authoring/SKILL.md`, and `tests/ci/test_unica_skills.py`; gap is a compact feature-lifecycle checklist, not a missing MCP capability.
- **Limits:** prose-only orchestration, no executable acceptance harness, and no context persistence beyond task artifacts.
- **Provisional decision:** `defer` — compare its phase gates during later workflow normalization.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `reject` — the prose-only phase checklist duplicates established planning, search, review, and test gates without executable evidence.

### `rmartynenko/workflow-dev-1c-claude-code`

- **Snapshot:** `main`, `afde2fd1f7cc419906a10ea53ee556332535a72b`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent.
- **Evidence:** [feature-development.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/workflows/feature-development.md), [activeContext.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/memory-bank/activeContext.md), [start-session.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/commands/start-session.md), and [update-knowledge.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/commands/update-knowledge.md); no automated tests are present.
- **Mechanism:** seven-phase discover/explore/clarify/design/implement/review/summary workflow plus a memory-bank directory and explicit session/knowledge-update commands.
- **Unica mapping:** overlaps workspace discovery in `crates/unica-coder/src/infrastructure/workspace.rs`, state services in `crates/unica-coder/src/infrastructure/workspace_services.rs`, and skill checks in `tests/ci/test_unica_skills.py`; gap is durable human-readable session context, not runtime workspace identity.
- **Limits:** Claude-specific paths and manual memory files; no concurrency, invalidation, or automated persistence tests.
- **Provisional decision:** `deep-dive` — test whether a minimal, provider-neutral session ledger improves continuity without changing public contracts.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — human-readable continuity is plausible, but manual files have no invalidation, concurrency, or persistence tests.

### `Pradushkoai/1c-ai-dev-env`

- **Snapshot:** `main`, `32a3adeaffc168301fd608a6c6984df633c9b8ad`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent. README labels the project Beta, which tempers maturity claims.
- **Evidence:** [README.md](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/README.md), [ADR-0003](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/adr/0003-hybrid-search-bm25-vector.md), [hybrid search implementation](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/src/services/search_hybrid.py), [fallback/hybrid tests](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/tests/test_search_hybrid.py), [tool snapshot test](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/tests/test_mcp_tools_snapshot.py), [62-tool snapshot](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/tests/snapshots/test_mcp_tools_snapshot/test_tool_count_snapshot/tool_count.txt), and [LICENSE](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/LICENSE).
- **Mechanism:** agent-first four-stage workflow, 62 MCP tools (the linked snapshot and README agree), BSL analyzers, XML/DSL compilers, and optional hybrid BM25/vector search with graceful BM25 fallback; the linked snapshot test covers the tool count.
- **Unica mapping:** overlaps `crates/unica-coder/src/application/mod.rs`, `crates/unica-coder/src/infrastructure/native_operations/form.rs`, `plugins/unica/skills/code-search/SKILL.md`, and `tests/ci/test_unica_skills.py`; Unica already has typed native operations, while optional hybrid indexing is a comparison gap.
- **Limits:** search tests use a mocked vector service and do not compare retrieval quality; README breadth, Beta status, Python/Java/Docker portability, and optional vector dependencies increase comparison cost.
- **Provisional decision:** `deep-dive` — compare fallback and indexing semantics against Unica workspace/index contracts without importing code.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — tested fallback is credible, but current engines and stronger common-harness candidates cover the same decision first.

### `Arman-Kudaibergenov/1c-ai-development-kit`

- **Snapshot:** `master`, `92d389edfb7a13c0799065e3865b9488ce019f2d`, `2026-08-03`.
- **License:** `LICENSE` is AGPL-3.0; GitHub metadata is AGPL-3.0 and consistent. `COPYRIGHT` repeats the AGPL terms and advertises a separate commercial arrangement; it does not clear transfer.
- **Evidence:** [1c-test-runner/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/1c-test-runner/SKILL.md), [1c-project-init/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/1c-project-init/SKILL.md), [brainstorm/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/brainstorm/SKILL.md), and [LICENSE](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/LICENSE); no automated skill tests were found in the pinned tree.
- **Mechanism:** large Claude skill catalog for project init, XML/DB operations, forms, metadata, web sessions, brainstorming, and MCP-backed 1C unit tests; scripts support installation and synchronization.
- **Unica mapping:** overlaps `plugins/unica/skills/test-authoring/SKILL.md`, `plugins/unica/skills/code-search/SKILL.md`, `crates/unica-coder/src/application/tool_contracts.rs`, and `tests/ci/test_unica_skills.py`; Unica’s native typed boundary is the key divergence from direct MCP skill commands.
- **Limits:** AGPL-3.0 limits material reuse; many skills require a live 1C base and external MCP extension; absence of tests leaves runtime claims unverified.
- **Provisional decision:** `defer` — inspect only selected test-runner and synchronization semantics under legal review.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `reject` — the untested live-base/external-MCP catalogue is host-specific, legally constrained, and duplicates Unica skills and tools.

### `Menestre1/reasoning-bank-poc`

- **Snapshot:** `main`, `30f6b52dbc7c2c54049421a0ce696b1b124f2f78`, `2026-08-03`.
- **License:** `package.json` declares ISC, but no LICENSE file is present; GitHub metadata reports no SPDX license. The issue’s preliminary ISC claim is therefore package metadata only, not a complete license file finding.
- **Evidence:** [ReasoningBankSemantic.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/src/ReasoningBankSemantic.ts), [MemoryCore.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/src/MemoryCore.ts), [ReasoningBankSemantic.test.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/tests/ReasoningBankSemantic.test.ts), [PatientKnowledgeBase.test.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/tests/PatientKnowledgeBase.test.ts), and [package.json](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/package.json).
- **Mechanism:** SQLite-backed experience records, confidence/usage promotion, and hash retrieval are exercised with HNSW disabled; tests prove feedback promotion, domain isolation, cleanup, patient-memory deduplication, and isolation. HNSW exists in implementation but is not exercised by the pinned tests.
- **Unica mapping:** overlaps `crates/unica-coder/src/domain/cache.rs`, `crates/unica-coder/src/infrastructure/workspace_state.rs`, and workspace fingerprint tests in `crates/unica-coder/src/infrastructure/workspace.rs`; gap is a durable reasoning-memory policy, not a missing public tool.
- **Limits:** PoC status, Node/better-sqlite3/native dependency, no Unica-specific evaluations, and incomplete license evidence; ISC must not be treated as cleared transfer license.
- **Provisional decision:** `deep-dive` — bounded experiment can test retrieval/promotion and invalidation against workspace-scoped state.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — retrieval/promotion and isolation expose an independent workspace-state policy question, while transfer remains uncleared.

### `vgtitov/bsl-ai-toolkit`

- **Snapshot:** `main`, `79bfde552f7e5acf96c80ca831817f24a3f9b9ce`, `2026-08-03`.
- **License:** `LICENSE` is MIT and GitHub metadata is MIT; `NOTICE.md` and `THIRD_PARTY.md` are present and require separate review.
- **Evidence:** [core/skills/1c-dev/SKILL.md](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/core/skills/1c-dev/SKILL.md), [layer tests](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/tests/test_layers_config.py), [XML configurator](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/onec_metadata/formats/configurator.py), [XML I/O tests](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/tests/onec_metadata/test_platform_xml_io.py), and [LICENSE](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/LICENSE).
- **Mechanism:** layered Claude skills plus tested layer filtering; three small XML fixtures prove exact-byte preservation of BOM/newline lexical style through the configurator path.
- **Unica mapping:** overlaps `plugins/unica/skills/code-diagnostics/SKILL.md`, `plugins/unica/skills/test-authoring/SKILL.md`, `crates/unica-coder/src/application/tool_contracts.rs`, and `tests/ci/test_format_profile_contract.py`; the narrow lexical-style tests are relevant, but do not replace Unica's official-dump format profile.
- **Limits:** full platform dumps, semantic round trips, RLS, masking, and paid capabilities are not proved by the pinned tests; portability depends on Claude adapters and 1C tooling.
- **Provisional decision:** `defer` — require a common fixture harness before comparing artifact fidelity or safety semantics.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — narrow XML-style evidence is useful, but complete platform fidelity and safety breadth remain unproved.

## Code intelligence and alternative indexers

### `Regsorm/code-index-mcp`

- **Snapshot:** `main`, `9614acd9048f73fbc4379d5fd240dc457c8d957a`, `2026-08-03`.
- **License:** `LICENSE` at pinned SHA is MIT; GitHub metadata is MIT and consistent.
- **Evidence:** [`indexer/mod.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/indexer/mod.rs) proves hash-based incremental reindex with new/changed/skipped/deleted counts, rayon parsing and sequential SQLite writes; daemon ownership is shown in [`daemon_core/state.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/daemon_core/state.rs), [`worker.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/daemon_core/worker.rs), and [`server.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/daemon_core/server.rs); federation dispatch is in [`dispatcher.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/federation/dispatcher.rs) and [`repos.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/federation/repos.rs); MCP caps/statuses are in [`tools.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/code-index-core/src/mcp/tools.rs). [`tools_integration.rs`](https://github.com/Regsorm/code-index-mcp/blob/9614acd9048f73fbc4379d5fd240dc457c8d957a/crates/bsl-extension/tests/tools_integration.rs) is pinned test evidence; no crash-repair test was found.
- **Mechanism:** SQLite structural/FTS index, AST/content hashes, incremental deletion/update, daemon writer, optional federation, bounded query envelopes; reads are gated unavailable during `ReindexingBatch` and data tools require `Ready`, so concurrent-read behavior remains a harness question. Enrichment is optional and not assumed.
- **Unica mapping:** Adaptable as an internal `CodeIntelligenceProvider` behind `unica.code.*`; exact mappings are `crates/unica-coder/src/domain/code_intelligence.rs`, `crates/unica-coder/src/application/code_intelligence.rs`, `crates/unica-coder/src/infrastructure/code_intelligence.rs`, and `crates/unica-coder/src/infrastructure/workspace_index.rs`. It overlaps bundled `bsl-analyzer`, `rlm-tools-bsl`, and `rlm-bsl-index`, adding explicit state/truncation and federation questions; no second MCP server is adoptable.
- **Limits:** pinned code/tests prove several caps, totals, incremental add/skip/delete, readiness, and federation/error paths; uniform exact-versus-ambiguous completeness, lower bounds across every endpoint, cancellation during indexing, and partial/stale/corrupt recovery still require one common harness. Existing bundled baselines are `bsl-analyzer` version `0.2.62`, SHA `9a6cb15d60c0381dce6a3b5e536434adb12da89b`, and `rlm-tools-bsl`/`rlm-bsl-index` version `1.29.1`, SHA `8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed` from `plugins/unica/third-party/tools.lock.json`.
- **Provisional decision:** `deep-dive` — primary code proves a mature incremental/index-server mechanism with concrete completeness and state-envelope questions.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — it is the strongest structural provider candidate for the shared completeness, state, and incremental-update harness.

### `Arman-Kudaibergenov/bsl-atlas`

- **Snapshot:** `master`, `b605768692ea2e51c3dfb199b788f6f4d2fb6325`, `2026-08-03`.
- **License:** pinned [`LICENSE`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/LICENSE) and GitHub metadata are AGPL-3.0. The pinned [`COPYRIGHT`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/COPYRIGHT) advertises a separate commercial arrangement; it is not an alternative grant for this screening.
- **Evidence:** [`main.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/src/main.py) exposes ambiguity and truncation metadata; [`sqlite_store.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/src/storage/sqlite_store.py) defines SQLite/WAL structural storage; [`file_tracker.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/src/indexer/file_tracker.py) tracks hashes/status; [`vector_indexer.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/src/indexer/vector_indexer.py) uses persistent Chroma. Tests [`test_sqlite_store.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/tests/test_sqlite_store.py), [`test_wave0_edges_incremental.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/tests/test_wave0_edges_incremental.py), [`test_vector_pipeline.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/tests/test_vector_pipeline.py), and [`test_integration.py`](https://github.com/Arman-Kudaibergenov/bsl-atlas/blob/b605768692ea2e51c3dfb199b788f6f4d2fb6325/tests/test_integration.py) cover graph resolution, add/delete vector updates, incremental behavior, and fake embeddings.
- **Mechanism:** dual SQLite structural plus optional Chroma vector index, BSL/metadata/help parsers, hash-based add/edit/delete tracking, call/data edges and reverse-call queries; collection names do not prove federated multi-root querying.
- **Unica mapping:** Could sit behind `CodeIntelligenceProvider` with orchestration in `crates/unica-coder/src/application/code_intelligence.rs` and workspace identity in `crates/unica-coder/src/infrastructure/workspace_index.rs` and `crates/unica-coder/src/infrastructure/code_intelligence.rs`. It duplicates bundled `rlm-bsl-index` structural search and adds graph/vector comparison, but cannot add a public server/tool.
- **Limits:** ambiguity/truncation fields exist, but uniform completeness/lower-bound semantics are not proved; Chroma/embedding services are external, and federation, cancellation, and corrupt/partial outcomes remain unproved. No comparative performance claim is made.
- **Provisional decision:** `defer` — useful mechanisms are evidenced, but legal, service, and completeness questions require a bounded harness.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — graph semantics are complementary, but AGPL/commercial licensing and external embedding cost dominate before a common harness exists.

## Specialized implementations

### `feenlace/mcp-1c`

- **Snapshot:** `main`, `926af4af57eb4c6c2a95a1b13ac269b0f7debe78`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata reports MIT and is consistent.
- **Evidence:** [query gate](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/tools/query.go), [query test](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/tools/query_test.go#L108), [Bleve index](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/index.go), [BM25/index test](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/index_test.go#L125), [generation reload](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/reload.go), [reload tests](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/reload_test.go#L85), [stale-cache test](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/stale_cache_test.go), and [content revalidation](https://github.com/feenlace/mcp-1c/blob/926af4af57eb4c6c2a95a1b13ac269b0f7debe78/dump/content_revalidate_test.go).
- **Mechanism:** the query gate and test enforce a read-only prefix policy; Bleve supplies BM25 scoring, and tests prove positive smart-search ranking. Generation-aware reload tests cover stale/corrupt recovery and concurrent search during updates.
- **Unica mapping:** adapt these semantics through `crates/unica-coder/src/application/operation_descriptors.rs`, `plugins/unica/skills/integration-implement/SKILL.md`, and `tests/ci/test_unica_skills.py`; expose existing runtime/code-search contracts, never a second MCP server (`INV-MCP-NO-ENGINE-SERVERS`). Preserve preview/support guards, atomic publication, stable envelopes, and redaction. Event-log behavior is relevant to `plugins/unica/skills/log-analysis/SKILL.md`; state remains workspace-scoped.
- **Limits:** no real infobase was contacted; full production cache policy, open/paid entitlement enforcement, and platform-semantic equivalence remain unverified beyond the cited unit tests.
- **Provisional decision:** `deep-dive` — compare a read-only/preview adapter and cache semantics in a bounded fixture.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — its read-only gate and generation-aware cache provide a strong internal safety/state comparison, while live-base claims remain hypotheses.

### `DitriXNew/EDT-MCP`

- **Snapshot:** `master`, `d2f29efc520ce373e637e61ec708073de0540bba`, `2026-08-03`.
- **License:** `LICENSE` is AGPL-3.0; GitHub metadata reports AGPL-3.0 and is consistent, so transfer is legally constrained.
- **Evidence:** [MCP server](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/bundles/com.ditrix.edt.mcp.server/src/com/ditrix/edt/mcp/server/McpServer.java), [toolsets](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/bundles/com.ditrix.edt.mcp.server/src/com/ditrix/edt/mcp/server/tools/Toolsets.java), [toolsets test](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/tests/com.ditrix.edt.mcp.server.tests/src/com/ditrix/edt/mcp/server/tools/ToolsetsTest.java), [registry visibility test](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/tests/com.ditrix.edt.mcp.server.tests/src/com/ditrix/edt/mcp/server/tools/McpToolRegistryVisibilityTest.java), [active-call cancellation](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/bundles/com.ditrix.edt.mcp.server/src/com/ditrix/edt/mcp/server/ActiveToolCall.java), [history log](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/mcp/bundles/com.ditrix.edt.mcp.server/src/com/ditrix/edt/mcp/server/history/McpCallHistoryFileLog.java), and [headless E2E CI](https://github.com/DitriXNew/EDT-MCP/blob/d2f29efc520ce373e637e61ec708073de0540bba/.github/workflows/e2e-tests.yml).
- **Mechanism:** `Toolsets.java` and its tests exercise grouped tool state, but progressive disclosure is not promoted as proven beyond those visibility cases; `ActiveToolCall.java` proves cancellation state and the E2E workflow proves headless CI wiring. The pinned tree exposes live workspace, diagnostics, completion, query validation, and refactoring handlers, but no cited executable test proves every handler’s semantic result or EDT fixture round trip.
- **Unica mapping:** adapt workspace/diagnostic/completion semantics behind `unica.code.diagnostics`, `unica.code.search`, and `unica.runtime.execute`; preserve the single `unica.*` boundary (`INV-MCP-NO-ENGINE-SERVERS`), stable envelope (`REQ-OBS-STABLE-ENVELOPE`), preview/no-partial-write rules for refactors, and workspace-scoped provider state. Do not adopt its external server identity.
- **Limits:** AGPL-3.0, EDT runtime dependency, and incomplete per-feature test evidence; cancellation and CI are observable, while query/refactoring/progressive-disclosure guarantees need fixture confirmation.
- **Provisional decision:** `defer` — useful lifecycle ideas, but legal/runtime cost and duplicate diagnostics make a focused experiment premature.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — cancellation and headless wiring are observable, but EDT/AGPL cost and incomplete handler semantics make it secondary.

### `Desko77/1c-formsserver`

- **Snapshot:** `master`, `cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata reports MIT and is consistent.
- **Evidence:** [schema models](https://github.com/Desko77/1c-formsserver/blob/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a/src/mcp_forms/schema/model.py), [schema validator](https://github.com/Desko77/1c-formsserver/blob/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a/src/mcp_forms/schema/validator.py), [generator](https://github.com/Desko77/1c-formsserver/blob/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a/src/mcp_forms/forms/generator.py), [converter tests](https://github.com/Desko77/1c-formsserver/blob/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a/tests/test_converter.py), and [validator tests](https://github.com/Desko77/1c-formsserver/blob/cd3f56e3508aaf34f33cdaf2c9bf1c0db9ff585a/tests/test_validator.py).
- **Mechanism:** Pydantic schema/parser, form generation/conversion, and validation are implemented with fixtures covering managed/logform conversion and validator failures. The tests do not establish complete round-trip fidelity for every supported form model.
- **Unica mapping:** compare schema/generation with `crates/unica-coder/src/infrastructure/native_operations/form.rs`, `crates/unica-coder/src/application/operation_descriptors.rs`, `plugins/unica/skills/form-compile/SKILL.md`, `plugins/unica/skills/form-edit/SKILL.md`, and `tests/ci/test_format_profile_contract.py`; retain format/support guards, preview by default, and transaction publication (`REQ-SAFETY-PREVIEW-BY-DEFAULT`, `REQ-SAFETY-NO-PARTIAL-WRITE`).
- **Limits:** actual round-trip coverage is partial; EDT client/search dependencies are outside the form fixtures, and no platform runtime validation is shown.
- **Provisional decision:** `deep-dive` — a common managed/logform fixture can test fidelity and validation parity without copying code.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — shared managed/logform fixtures can answer native-operation fidelity and validation questions without transfer.

### `alexiosus/mxl-merge-tool`

- **Snapshot:** `main`, `83839e91685f743be203458194e1c11bc1ddd1fa`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata reports MIT and is consistent.
- **Evidence:** [semantic merge implementation](https://github.com/alexiosus/mxl-merge-tool/blob/83839e91685f743be203458194e1c11bc1ddd1fa/mxl_tool.py), [installer/report tests](https://github.com/alexiosus/mxl-merge-tool/blob/83839e91685f743be203458194e1c11bc1ddd1fa/tests/test_mxl_tool.py#L218), [end-to-end merge tests](https://github.com/alexiosus/mxl-merge-tool/blob/83839e91685f743be203458194e1c11bc1ddd1fa/tests/test_mxl_tool.py#L273), [conflict tests](https://github.com/alexiosus/mxl-merge-tool/blob/83839e91685f743be203458194e1c11bc1ddd1fa/tests/test_mxl_tool.py#L384), and [atomic/parseable output tests](https://github.com/alexiosus/mxl-merge-tool/blob/83839e91685f743be203458194e1c11bc1ddd1fa/tests/test_mxl_tool.py#L920).
- **Mechanism:** Python tooling and fixtures exercise semantic diff/three-way merge, conflict-oriented output, installer/report behavior, and an end-to-end Git merge-driver conflict report outside the worktree; atomic and parseable output are also tested.
- **Unica mapping:** compare with `crates/unica-coder/src/infrastructure/native_operations/mxl.rs`, `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs`, and `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs`. Any merge application remains preview-first and support-guarded, with stable envelopes and no partial writes.
- **Limits:** pinned tests prove installer/report behavior, end-to-end merge/conflict handling, atomic output, and parseable output; 1C platform-semantic equivalence and full merge-driver integration remain unproven.
- **Provisional decision:** `deep-dive` — test semantic merge conflict encoding against Unica’s JSON DSL and atomic writer.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — tested semantic merge and Git-driver integration expose a real gap beyond Unica's current MXL writers.

### `rzateev/onec-help-mcp`

- **Snapshot:** `main`, `f66860b45ecad40e071ac4abe3f7ef432b30ac24`, `2026-08-03`.
- **License:** `LICENSE` is MIT and GitHub metadata reports MIT; README’s license badge says “Non-Commercial”, contradicting the pinned MIT license file and metadata.
- **Evidence:** [HBK reader](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/src/parsers/hbk_reader.py), [search](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/src/core/search.py), [indexing tools](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/src/tools/indexing_tools.py), [settings tests](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/tests/test_settings.py), [version tests](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/tests/test_version.py), [README badge/text](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/README.md), and [LICENSE](https://github.com/rzateev/onec-help-mcp/blob/f66860b45ecad40e071ac4abe3f7ef432b30ac24/LICENSE).
- **Mechanism:** HBK/RES parsing, indexing, and hybrid BM25/dense search are implemented; tests cover settings/version behavior, but no pinned test proves extraction against representative HBK binaries or search quality.
- **Unica mapping:** extraction/search can inform `crates/unica-coder/src/infrastructure/native_operations/help.rs`, `crates/unica-coder/src/application/operation_descriptors.rs`, `plugins/unica/skills/help-add/SKILL.md`, and `plugins/unica/skills/platform-help/SKILL.md`; the registered public help mutation is `unica.help.add`. Preserve one public boundary, stable envelopes, redaction, support/format guards, and atomic writers.
- **Limits:** source-help extraction legality and binary corpus coverage are unverified; README/license contradiction blocks transfer pending clarification.
- **Provisional decision:** `defer` — observe search design, but legal contradiction and missing HBK fixture tests preclude immediate deep dive.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — no representative HBK/search test exists, and the contradictory non-commercial label must be reconciled before transfer.

### `mussolene/1c_hbk_bsl`

- **Snapshot:** `main`, `cee853014d57e34950c86ba24957af5ecc3e6d49`, `2026-08-03`.
- **License:** `LICENSE` and GitHub metadata are MIT. [`docs/THIRD_PARTY_NOTICES.md`](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/docs/THIRD_PARTY_NOTICES.md) identifies adapted diagnostic prose from BSL Language Server under LGPL-3.0; runtime code remains MIT, while copied prose needs separate treatment.
- **Evidence:** [diagnostic engine](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/src/onec_hbk_bsl/analysis/diagnostic/engine.py), [SARIF CLI tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_cli_check.py#L265), [main tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_main.py#L102), [formatter tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_formatter.py), [LSP tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_lsp_server.py), [MCP tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_mcp_server.py), and [indexing tests](https://github.com/mussolene/1c_hbk_bsl/blob/cee853014d57e34950c86ba24957af5ecc3e6d49/tests/test_indexer.py).
- **Mechanism:** the project documents diagnostics, formatter, CLI/LSP/MCP surfaces, SARIF-oriented tooling, indexing/benchmark scripts, tests, and bilingual documentation; the pinned tests prove selected diagnostics/baseline behavior, not all advertised modes or protocol compatibility.
- **Unica mapping:** compare diagnostics with `crates/unica-coder/src/infrastructure/native_operations/code.rs`, `crates/unica-coder/src/application/operation_descriptors.rs`, `plugins/unica/skills/code-diagnostics/SKILL.md`, and `tests/ci/test_unica_skills.py`; keep stable envelopes, cancellation/deadlines, workspace scoping, and secret redaction. Do not expose its CLI/LSP/MCP servers as additional public endpoints.
- **Limits:** cited tests prove SARIF CLI behavior and main entry behavior; all indexing modes, LSP/MCP interoperability, and complete RU/EN parity remain unverified. LGPL documentation content needs separate licensing treatment.
- **Provisional decision:** `deep-dive` — bounded diagnostics/SARIF comparison against Unica’s existing analyzer and envelope.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — it mostly complements the bundled analyzer, while full SARIF/protocol/index parity and third-party prose handling remain open.

## Evaluation and safe access

### `genlab-1c/prism`

- **Snapshot:** `main`, `6adda50c572a28ca2f915b64bc89c667abf93ea3`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata reports MIT and is consistent.
- **Evidence:** [prompt contract](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/generation/prompts.yaml), [OneC executor](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/harness/execute/onec/runner.py), [scoring axes](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/harness/score/platform.py), [generation tests](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/tests/test_generate_run.py), [loader tests](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/tests/test_loaders.py), [runner tests](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/tests/test_onec_runner.py), [platform scoring tests](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/tests/test_platform.py), and [syntax tests](https://github.com/genlab-1c/prism/blob/6adda50c572a28ca2f915b64bc89c667abf93ea3/tests/test_syntax.py).
- **Mechanism:** PRISM keeps generation inputs separate from canonical/hidden fixtures, and tests cover category workflows, loader kits, syntax/platform scoring, and runner result classification. A Docker OneC runner is implemented; the pinned CI/tests do not demonstrate an actual platform execution, so executable platform validation remains a deep-dive hypothesis rather than a screening result.
- **Unica mapping:** compare the executable oracle and stable result classification with `scripts/ci/release-assessment.py`, `tests/ci/test_release_assessment.py`, and exact fixtures `tests/fixtures/unica_mcp_script_parity/bsp/manifest.json` and `tests/fixtures/unica_mcp_script_parity/bsp/forms/BusinessProcesses__Задание__ФормаСписка/Form.xml`; candidate integration must remain behind `crates/unica-coder/src/application/operation_descriptors.rs` and preserve `REQ-OBS-STABLE-ENVELOPE` in `spec/architecture/quality-requirements.md`.
- **Limits:** Docker/1C and hidden fixtures constrain portability; expert L2 judging is expressly planned rather than executable; answer leakage, full-artifact coverage, and model-independent repeatability remain screening questions.
- **Provisional decision:** `deep-dive` — executable comparison is promising, but a common Unica fixture harness must prove leakage resistance and artifact coverage.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — prompt separation and tested classification justify a bounded evaluation-harness comparison; real-platform execution and artifact breadth remain unanswered.

### `comol/1CLLMBenchTasks`

- **Snapshot:** `main`, `39732c7709651bf6628360393cf8fe0e30d96c8c`, `2026-08-03`.
- **License:** no `LICENSE` file is present in the pinned tree and GitHub metadata reports no SPDX license; public visibility and README claims do not grant transfer permission.
- **Evidence:** [README task/evaluation rules](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/README.md), [query task](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/01.md), [form task](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/09.md), and [artifact/report task](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/17.md).
- **Mechanism:** seventeen Markdown cases specify prompts, expected answers or criteria, and requested deliverables spanning queries, BSL, managed forms, external print forms, reports, and 1C:ЗУП. The README instructs evaluators to send only the постановка and manually compare to the правильный ответ; no pinned executor, deterministic oracle, model adapter, scoring implementation, or CI is present. Expected answers therefore can leak if the full task file is supplied, and correctness is subjective for alternate solutions. Domain evidence includes managed-form/XML requirements in [Task 09](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/09.md), DCS/report requirements in [Task 17](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/17.md), e-mail integration and a modified executable database in [Task 15](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/15.md), and an external print form in [Task 16](https://github.com/comol/1CLLMBenchTasks/blob/39732c7709651bf6628360393cf8fe0e30d96c8c/Tasks/16.md). These are requested domains, not checked-in executable fixtures or oracles; no role-specific case is evidenced.
- **Unica mapping:** this is a task corpus only; compare its case schema with `scripts/ci/release-assessment.py`, `tests/ci/test_release_assessment.py`, and exact fixtures `tests/fixtures/unica_mcp_script_parity/bsp/forms/BusinessProcesses__Задание__ФормаСписка/Form.xml` and `tests/fixtures/unica_mcp_script_parity/bsp/dcs/Catalogs__ПравилаОбработкиЭлектроннойПочты__СхемаПравилаОбработкиЭлектроннойПочты/Template.xml`. It provides requested breadth hypotheses for forms/XML, DCS, e-mail integration, and external print forms, not executable evidence; no checked-in MXL/role/integration/full-artifact oracle or new `unica.*` tool is present.
- **Limits:** missing license is a transfer blocker; no executable oracle, platform run, model-independent scoring, or artifact fixture proves the README’s coverage claims.
- **Provisional decision:** `defer` — retain as inspiration-only cases until licensing and an independent executable harness exist.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `defer` — seventeen manual cards provide breadth hypotheses, but no license, runner, fixture, or deterministic oracle.

### `alonehobo/1c-trusted-gateway`

- **Snapshot:** `main`, `a5cc656e3f3763800706ec752fd33fb2e18318e4`, `2026-08-03`.
- **License:** no `LICENSE` file is present in the pinned tree and GitHub metadata reports no SPDX license; observation is permitted for screening, transfer is not cleared.
- **Evidence:** [privacy masking](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/privacy.go), [type policy](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/type_policy.go), [masking tests](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/privacy_test.go), [policy tests](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/type_policy_test.go), [MCP proxy](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/mcp_server.go#L474), [bridge execution](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/web.go#L1021), [raw in-memory logs](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/logs.go), and [approval UI actions](https://github.com/alonehobo/1c-trusted-gateway/blob/a5cc656e3f3763800706ec752fd33fb2e18318e4/ui_actions.js).
- **Mechanism:** `toolSuggestFields` waits for either user confirmation or the 120-second timeout, then returns the same message claiming the user approved; it does not emit a distinct approval-request/confirmation signal. `bridgeExecuteCode` receives `fromBridge=true` and executes agent code immediately in auto mode, while approval gates only the manual UI path. `logs.go` stores raw MCP response text in a clearable in-memory ring, so it is operational logging rather than tamper-evident audit. Masking/type policies and recursive handling are covered by unit tests, but no pinned fail-open/fail-closed matrix or complete approval/proxy integration test exists.
- **Unica mapping:** compare these risks with `REQ-SAFETY-SECRET-REDACTION`, `REQ-SAFETY-PREVIEW-BY-DEFAULT`, and `REQ-SAFETY-NO-PARTIAL-WRITE` in `spec/architecture/quality-requirements.md`. Current redaction evidence is `crates/unica-coder/src/infrastructure/redaction.rs` (`stream_redactor_redacts_secret_key_split_across_chunks`, runtime-secret forms) plus `crates/unica-coder/src/infrastructure/runtime_jobs.rs` (`worker_stream_tail_redacts_output_before_retaining_it`, `terminal_snapshot_and_persistence_are_redacted_and_keep_log_artifacts`). Preview enforcement is in `crates/unica-coder/src/application/mod.rs`: mutating tools default to `dryRun`, with `mutating_tool_defaults_to_dry_run_and_reports_cache` and `meta_compile_dry_run_reports_exact_registration_diff_without_writes`; `tests/ci/test_unica_mcp_smoke.py` (`test_mutating_dry_run_reports_cache_impact`) separately checks the explicit MCP dry-run cache envelope. Failure-atomic publication is implemented and tested in `crates/unica-coder/src/infrastructure/native_operations/single_file_publisher.rs` (`target_changed_after_staging_is_rejected`, `precommit_failpoints_preserve_target_and_remove_stage`) and `crates/unica-coder/src/infrastructure/native_operations/compile_transaction.rs` (`compile_transaction_rejects_readonly_registration_without_partial_creates` and concurrent-recovery rollback tests). Any adaptation belongs behind existing `unica.*` operations, not as a second MCP server; gateway auto-mode execution therefore remains a preview-by-default and no-partial-write risk requiring adversarial tests.
- **Limits:** missing license blocks transfer; README security claims exceed the cited unit-test scope; fail-open behavior, auditability, whitelist bypasses, and partial-write safety need adversarial tests.
- **Provisional decision:** `deep-dive` — safety semantics merit a bounded red-team experiment focused on failure modes and approval enforcement.
- **Review:** `strong-model-reviewed` — `gpt-5.6-sol`; evidence reopened and cross-source checked.
- **Decision:** `deep-dive` — observable approval-bypass and raw-log behavior warrant bounded adversarial comparison; missing license blocks transfer.

## Existing Unica donors and bundled engines

- `Nikolay-Shirokov/cc-1c-skills` is an existing tracked donor in
  `spec/provenance/skill-upstreams.json` and `plugins/unica/ATTRIBUTIONS.md`;
  it is not one of the 19 new discoveries and is not rescored here.
- `bsl-analyzer` version `0.2.62`, commit
  `9a6cb15d60c0381dce6a3b5e536434adb12da89b`, and `rlm-tools-bsl` plus
  `rlm-bsl-index` version `1.29.1`, commit
  `8bc6e9fc83b522f9a79eab3193eb13fc2cecb8ed`, are existing bundled baselines
  recorded in `plugins/unica/third-party/tools.lock.json`, not discoveries.
- The review compares externally observed semantics with those baselines; it
  transfers no external material and recommends no additional public engine
  server.

## Cross-source normalization

All 19 pinned snapshots were reopened under the same threshold: a mechanism is
"proven" only when primary implementation plus a pinned test or equally direct
artifact supports the exact claim. README breadth, an implemented-but-untested
path, and a requested benchmark domain are hypotheses. No comparative quality,
latency, or size numbers were produced during screening.

The workflow repositories largely duplicate Unica planning, search, review, and
test guidance; only Menestre's tested retrieval/promotion policy remains an
independent state question. Code-intelligence candidates overlap the existing
`bsl-analyzer` and RLM baselines, so only Regsorm advances as the primary shared
harness candidate; feenlace advances for its independent cache and read-only
gate semantics. Form and MXL candidates advance because their fixtures expose
artifact questions not answered by skill prose. PRISM advances for harness
shape and classification, not because real 1C execution was demonstrated.

External MCP names, live-base commands, or EDT servers cannot be adopted as a
second public boundary: any useful semantics must remain behind existing
`unica.*` operations under `INV-MCP-NO-ENGINE-SERVERS`. Missing or contradictory
licenses block material transfer. Platform round trips, live-infobase behavior,
full protocol parity, benchmark breadth, and trusted-gateway safety claims stay
hypotheses until the primary evidence named below exists.

## Thematic shortlist

This is a research shortlist, not a list of final product experiments or
follow-up issues.

### Workflow, skills, and context management

- **Candidates:** Menestre (`deep-dive`); rmartynenko (`defer`).
- **Unanswered question:** can durable experience/session state improve
  continuity without leaking across workspace identities or becoming stale?
- **Required primary evidence:** deterministic isolation, promotion,
  invalidation, cleanup, and concurrent-access tests against Unica workspace
  fingerprints.
- **Minimal boundary:** compare policy and envelopes only; no copied storage
  code, new public tool, or product experiment.

### Code intelligence

- **Candidates:** Regsorm and feenlace (`deep-dive`); Pradushko, bsl-atlas, and
  mussolene (`defer`).
- **Unanswered question:** which provider semantics preserve complete,
  explicitly bounded, workspace-scoped results under incremental change and
  failure?
- **Required primary evidence:** the common harness in the next section,
  executed against existing bundled engines and candidates on identical
  fixtures.
- **Minimal boundary:** provider adapters behind existing `unica.code.*`
  operations; no second MCP server and no screening performance claims.

### Live environments, data, and safety

- **Candidates:** gateway and feenlace (`deep-dive`); EDT (`defer`).
- **Unanswered question:** do approval, read-only, redaction, cancellation, and
  partial-failure paths fail closed at the actual execution boundary?
- **Required primary evidence:** adversarial proxy/bridge tests, live-base or
  controlled fake-base traces, explicit approval events, and atomicity checks.
- **Minimal boundary:** red-team semantics only; no external code transfer or
  live production data.

### Artifacts and documentation

- **Candidates:** formsserver and mxl-merge-tool (`deep-dive`); bsl-ai-toolkit
  and onec-help-mcp (`defer`).
- **Unanswered question:** which lexical and semantic properties survive
  parse/edit/merge/round-trip on representative platform artifacts?
- **Required primary evidence:** official or repository-owned fixtures with
  byte comparisons, platform load validation, conflict oracles, and failure
  atomicity.
- **Minimal boundary:** common fixtures against existing native operations;
  no format-policy change and no copied implementation.

### Benchmark and evaluation

- **Candidates:** PRISM (`deep-dive`); 1CLLMBenchTasks (`defer`).
- **Unanswered question:** can a leakage-resistant, model-independent oracle
  cover Unica's BSL and artifact contracts on an actual 1C run?
- **Required primary evidence:** isolated prompt/hidden fixtures, reproducible
  runner artifacts, deterministic classification, and representative
  XML/forms/MXL/DCS/role/integration cases.
- **Minimal boundary:** evaluate harness evidence only; no final benchmark,
  product experiment, or follow-up issue is selected here.

## Deferred deep-dive protocol

No provider is ranked from README metrics, and no comparative numbers are
claimed. A later authorized deep dive must use one fixture and report all eight
dimensions together:

- **Exact-symbol and ambiguous-symbol completeness:** identical exact hits and explicit ambiguity rather than silent selection.
- **Reported truncation and lower bounds:** caps plus totals/lower bounds distinguish complete from partial results.
- **Incremental add/change/delete behavior:** structural and auxiliary indexes update without stale hits.
- **Cache identity and invalidation after rename:** old entries are invalidated while unaffected workspace data remains valid.
- **Multi-root and extension topology:** roots and 1C extensions remain isolated, queryable, and provenance-labelled.
- **Cold/warm latency and index size:** cold build, warm query, and on-disk size use the same fixture and measurement method.
- **Cancellation and concurrent readers:** cancellation bounds work and readers remain consistent while one writer updates.
- **Partial, stale, unavailable, and corrupted-index outcomes:** explicit diagnostics and safe fallback/rebuild replace fabricated completeness.
