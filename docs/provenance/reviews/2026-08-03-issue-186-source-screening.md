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

Reserved for the workflow cohort cards.

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
