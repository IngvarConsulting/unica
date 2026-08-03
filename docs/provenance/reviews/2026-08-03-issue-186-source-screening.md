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

## Decision vocabulary

- `deep-dive`: evidence shows a potentially useful mechanism and a concrete Unica gap or comparison question that screening cannot answer.
- `defer`: the mechanism may be relevant, but current evidence, maturity, licensing, or cost does not justify a deep dive now.
- `reject`: the mechanism is duplicated, unsupported by source evidence, incompatible with non-negotiable Unica contracts, or disproportionately costly.

## Summary registry

Populated after all 19 cards pass the stronger-model review gate.

## Workflow, skills, agents, and context management

### `SteelMorgan/1c-agent-based-dev-framework`

- **Snapshot:** `main`, `19f67bfed6d15f051f9568678bab1701b7735f95`, `2026-08-03`.
- **License:** no repository-level LICENSE was present and GitHub metadata reports no SPDX license; embedded notices exist at `framework/skills/tool-usage/browser-ui/playwright/LICENSE.txt` and `NOTICE.txt`. This blocks transfer, not observation.
- **Evidence:** [CLAUDE.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/.claude/CLAUDE.md), [context.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/context.md), [sdd.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/sdd.md), [tdd.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/docs/info/tdd.md), sandbox skill [SKILL.md](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/.claude/skills/sandbox-framework/SKILL.md), and [tools/test_install_always_apply.py](https://github.com/SteelMorgan/1c-agent-based-dev-framework/blob/19f67bfed6d15f051f9568678bab1701b7735f95/tools/test_install_always_apply.py), which tests installation/rule application and context estimation.
- **Mechanism:** Claude rules and skills define discovery/context compaction, SDD/TDD phases, role-separated agents, and a sandbox permission model; installation is represented by `skills-manifest.json` and bootstrap scripts.
- **Unica mapping:** overlaps `plugins/unica/skills/code-review/SKILL.md`, `plugins/unica/skills/code-search/SKILL.md`, `crates/unica-coder/src/infrastructure/workspace.rs` (workspace discovery/fingerprint), and `tests/ci/test_unica_skills.py`; gap is a source-backed, reusable context-compaction/session protocol rather than another public tool.
- **Limits:** prose-heavy, host-specific Claude layout, no license, and no pinned tests proving the orchestration end to end.
- **Provisional decision:** `defer` — useful workflow patterns are largely represented locally; only bounded context/session experiments remain.
- **Review:** `draft`.

### `comol/ai_rules_1c`

- **Snapshot:** `main`, `410951e74fd3e6b7a763cf49757935b9a34d3f31`, `2026-08-03`.
- **License:** no LICENSE file and GitHub metadata has no SPDX license; `ATTRIBUTIONS.md` records the same uncertainty and permits only inspiration-only use.
- **Evidence:** [AGENTS.md](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/AGENTS.md), planner [agent](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/content/agents/planner.md), tester [agent](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/content/agents/tester.md), and validation workflow [validate-rules.yml](https://github.com/comol/ai_rules_1c/blob/410951e74fd3e6b7a763cf49757935b9a34d3f31/.github/workflows/validate-rules.yml); no runtime test suite was found.
- **Mechanism:** adapter-specific rules, agents, commands, and an OpenSpec bundle provide planning, review, testing, and synchronization across IDE hosts.
- **Unica mapping:** `spec/provenance/skill-upstreams.json`, `plugins/unica/ATTRIBUTIONS.md`, `docs/provenance/reviews/2026-07-22-ai-rules-idea-provenance-correction.json`, `tests/ci/test_skill_provenance.py`, and `tests/ci/test_unica_skills.py` explicitly classify this repository as inspiration-only; it overlaps Unica-owned `code-review`, `code-search`, `test-authoring`, and planning guidance, with no new donor gap established.
- **Limits:** licensing is unresolved; behavior is mostly markdown/adapters, not executable enforcement; adapter synchronization is host-dependent.
- **Provisional decision:** `reject` — retain as existing inspiration-only provenance, not a new donor or transfer candidate.
- **Review:** `draft`.

### `AndreevED/1c-ai-feature-dev-workflow`

- **Snapshot:** `main`, `c67108acb534e18e6e539f27b7991f7497dcc539`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent.
- **Evidence:** [1c-feature-dev/SKILL.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/skills/1c-feature-dev/SKILL.md), [1c-code-reviewer.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/agents/1c-code-reviewer.md), [1c-code-writer.md](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/agents/1c-code-writer.md), and [LICENSE](https://github.com/AndreevED/1c-ai-feature-dev-workflow/blob/c67108acb534e18e6e539f27b7991f7497dcc539/LICENSE); no automated tests are present.
- **Mechanism:** the feature skill prescribes complexity assessment, requirement clarification, atomic phases, plan review, implementation, and acceptance checks, with dedicated explorer/writer/reviewer agents.
- **Unica mapping:** overlaps `plugins/unica/skills/brainstorm/SKILL.md`, `plugins/unica/skills/code-review/SKILL.md`, `plugins/unica/skills/test-authoring/SKILL.md`, and `tests/ci/test_unica_skills.py`; gap is a compact feature-lifecycle checklist, not a missing MCP capability.
- **Limits:** prose-only orchestration, no executable acceptance harness, and no context persistence beyond task artifacts.
- **Provisional decision:** `defer` — compare its phase gates during later workflow normalization.
- **Review:** `draft`.

### `rmartynenko/workflow-dev-1c-claude-code`

- **Snapshot:** `main`, `afde2fd1f7cc419906a10ea53ee556332535a72b`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent.
- **Evidence:** [feature-development.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/workflows/feature-development.md), [activeContext.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/memory-bank/activeContext.md), [start-session.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/commands/start-session.md), and [update-knowledge.md](https://github.com/rmartynenko/workflow-dev-1c-claude-code/blob/afde2fd1f7cc419906a10ea53ee556332535a72b/.claude/commands/update-knowledge.md); no automated tests are present.
- **Mechanism:** seven-phase discover/explore/clarify/design/implement/review/summary workflow plus a memory-bank directory and explicit session/knowledge-update commands.
- **Unica mapping:** overlaps workspace discovery in `crates/unica-coder/src/infrastructure/workspace.rs`, state services in `crates/unica-coder/src/infrastructure/workspace_services.rs`, and skill checks in `tests/ci/test_unica_skills.py`; gap is durable human-readable session context, not runtime workspace identity.
- **Limits:** Claude-specific paths and manual memory files; no concurrency, invalidation, or automated persistence tests.
- **Provisional decision:** `deep-dive` — test whether a minimal, provider-neutral session ledger improves continuity without changing public contracts.
- **Review:** `draft`.

### `Pradushkoai/1c-ai-dev-env`

- **Snapshot:** `main`, `32a3adeaffc168301fd608a6c6984df633c9b8ad`, `2026-08-03`.
- **License:** `LICENSE` is MIT; GitHub metadata is MIT and consistent. README labels the project Beta, which tempers maturity claims.
- **Evidence:** [README.md](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/README.md), [AGENTS.md](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/AGENTS.md), [ADR-0003](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/adr/0003-hybrid-search-bm25-vector.md), and [LICENSE](https://github.com/Pradushkoai/1c-ai-dev-env/blob/32a3adeaffc168301fd608a6c6984df633c9b8ad/LICENSE); repository test directories and CI configuration are present, but no bounded test command was run because dependencies are not installed.
- **Mechanism:** agent-first four-stage workflow, 62 MCP tools (the pinned `tests/snapshots/test_mcp_tools_snapshot/test_tool_count_snapshot/tool_count.txt` and `README.en.md` agree), BSL analyzers, XML/DSL compilers, and optional hybrid BM25/vector search with graceful BM25 fallback; implementation is in `src/mcpserver/tools/tool_definitions.py` and the count is covered by `tests/test_mcp_tools_snapshot.py`.
- **Unica mapping:** overlaps `crates/unica-coder/src/application/mod.rs`, `crates/unica-coder/src/infrastructure/native_operations/form.rs`, `plugins/unica/skills/code-search/SKILL.md`, and `tests/ci/test_unica_skills.py`; Unica already has typed native operations, while optional hybrid indexing is a comparison gap.
- **Limits:** README metrics and feature breadth are not independently verified here; Beta status, Python/Java/Docker portability, and optional vector dependencies increase experiment cost.
- **Provisional decision:** `deep-dive` — compare fallback and indexing semantics against Unica workspace/index contracts without importing code.
- **Review:** `draft`.

### `Arman-Kudaibergenov/1c-ai-development-kit`

- **Snapshot:** `master`, `92d389edfb7a13c0799065e3865b9488ce019f2d`, `2026-08-03`.
- **License:** `LICENSE` is AGPL-3.0; GitHub metadata is AGPL-3.0 and consistent, creating copyleft constraints for transfer.
- **Evidence:** [1c-test-runner/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/1c-test-runner/SKILL.md), [1c-project-init/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/1c-project-init/SKILL.md), [brainstorm/SKILL.md](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/.claude/skills/brainstorm/SKILL.md), and [LICENSE](https://github.com/Arman-Kudaibergenov/1c-ai-development-kit/blob/92d389edfb7a13c0799065e3865b9488ce019f2d/LICENSE); no automated skill tests were found in the pinned tree.
- **Mechanism:** large Claude skill catalog for project init, XML/DB operations, forms, metadata, web sessions, brainstorming, and MCP-backed 1C unit tests; scripts support installation and synchronization.
- **Unica mapping:** overlaps `plugins/unica/skills/test-authoring/SKILL.md`, `plugins/unica/skills/code-search/SKILL.md`, `crates/unica-coder/src/application/tool_contracts.rs`, and `tests/ci/test_unica_skills.py`; Unica’s native typed boundary is the key divergence from direct MCP skill commands.
- **Limits:** AGPL-3.0 limits material reuse; many skills require a live 1C base and external MCP extension; absence of tests leaves runtime claims unverified.
- **Provisional decision:** `defer` — inspect only selected test-runner and synchronization semantics under legal review.
- **Review:** `draft`.

### `Menestre1/reasoning-bank-poc`

- **Snapshot:** `main`, `30f6b52dbc7c2c54049421a0ce696b1b124f2f78`, `2026-08-03`.
- **License:** `package.json` declares ISC, but no LICENSE file is present; GitHub metadata reports no SPDX license. The issue’s preliminary ISC claim is therefore package metadata only, not a complete license file finding.
- **Evidence:** [ReasoningBankSemantic.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/src/ReasoningBankSemantic.ts), [MemoryCore.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/src/MemoryCore.ts), [ReasoningBankSemantic.test.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/tests/ReasoningBankSemantic.test.ts), [PatientKnowledgeBase.test.ts](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/tests/PatientKnowledgeBase.test.ts), and [package.json](https://github.com/Menestre1/reasoning-bank-poc/blob/30f6b52dbc7c2c54049421a0ce696b1b124f2f78/package.json).
- **Mechanism:** SQLite-backed experience records, confidence/usage promotion, hash or HNSW similarity retrieval, patient/code memory, and tests for recording, retrieval, and deduplication.
- **Unica mapping:** overlaps `crates/unica-coder/src/domain/cache.rs`, `crates/unica-coder/src/infrastructure/workspace_state.rs`, and workspace fingerprint tests in `crates/unica-coder/src/infrastructure/workspace.rs`; gap is a durable reasoning-memory policy, not a missing public tool.
- **Limits:** PoC status, Node/better-sqlite3/native dependency, no Unica-specific evaluations, and incomplete license evidence; ISC must not be treated as cleared transfer license.
- **Provisional decision:** `deep-dive` — bounded experiment can test retrieval/promotion and invalidation against workspace-scoped state.
- **Review:** `draft`.

### `vgtitov/bsl-ai-toolkit`

- **Snapshot:** `main`, `79bfde552f7e5acf96c80ca831817f24a3f9b9ce`, `2026-08-03`.
- **License:** `LICENSE` is MIT and GitHub metadata is MIT; `NOTICE.md` and `THIRD_PARTY.md` are present and require separate review.
- **Evidence:** [core/skills/1c-dev/SKILL.md](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/core/skills/1c-dev/SKILL.md), [core/skills/1c-tester/SKILL.md](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/core/skills/1c-tester/SKILL.md), [tests/test_layers_config.py](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/tests/test_layers_config.py), [tests/onec_metadata/test_platform_xml_io.py](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/tests/onec_metadata/test_platform_xml_io.py), and [LICENSE](https://github.com/vgtitov/bsl-ai-toolkit/blob/79bfde552f7e5acf96c80ca831817f24a3f9b9ce/LICENSE); repository CI/test configuration exists, but byte-level claims were not probed.
- **Mechanism:** layered Claude skills with development, analysis, testing, metadata, and operations guidance, plus adapters and core MCP configuration; proven behavior is the checked-in rules and tests only.
- **Unica mapping:** overlaps `plugins/unica/skills/code-diagnostics/SKILL.md`, `plugins/unica/skills/test-authoring/SKILL.md`, `crates/unica-coder/src/application/tool_contracts.rs`, and `tests/ci/test_unica_skills.py`; no evidence here proves byte-perfect XML, round-trip, RLS, masking, or paid-capability claims against Unica contracts.
- **Limits:** separate proven code/tests from README or commercial claims; byte-perfect, round-trip, RLS, masking, and paid features remain unverified; portability depends on Claude adapters and 1C tooling.
- **Provisional decision:** `defer` — require a common fixture harness before comparing artifact fidelity or safety semantics.
- **Review:** `draft`.

## Code intelligence and alternative indexers

Reserved for the code-intelligence cohort cards.

## Specialized implementations

Reserved for the specialized implementation cohort cards.

## Evaluation and safe access

Reserved for benchmark and trusted-access cards.

## Existing Unica donors and bundled engines

Existing donors and bundled engines will be classified separately from new
discoveries; no external material is transferred by this screening.

## Cross-source normalization

Cross-source comparison and final decisions are written only after every card
has been reopened by the stronger reasoning model.

## Thematic shortlist

Later deep-dive themes will be listed only after normalization; this artifact
does not select final product experiments.

## Deferred deep-dive protocol

Deferred experiments must use a common harness and measure completeness,
truncation reporting, incremental updates, cache invalidation, multi-root
topology, cold/warm latency, cancellation, concurrency, and partial or corrupt
index outcomes.
