# Agent Entry Points

## Source Of Truth

When changing Unica, resolve conflicts in this order:

1. code and tests
2. `plugins/unica/.mcp.json`, `plugins/unica/.codex-plugin/plugin.json`, `plugins/unica/.claude-plugin/plugin.json`, and `plugins/unica/third-party/tools.lock.json` are package-contract sources, not background notes.
3. `plugins/unica/references/specs/` is the contract source for 1C XML and JSON DSL formats. When the question is what an emitted `.xml`, `.mxl`, or DSL payload must look like, these specs outrank `spec/`, skills, and prose. They do not outrank emitter behavior proven by a fixture or by an official platform dump; when they disagree with one, fix the spec.
4. `spec/` is the active architecture layer unless it contradicts live code, tests, or package metadata. Its normative rules live in `spec/architecture/invariants.md` and the records under `spec/decisions/`.
5. `README.md` and skill prose

## Where To Look, Where To Change

Paths in the middle column are relative to `spec/` unless prefixed otherwise.

| Task | Read first | Change in code |
| --- | --- | --- |
| New or changed public `unica.*` tool | `architecture/invariants.md` (`INV-MCP-04`, `INV-MCP-08`), `architecture/change-checklist.md` | `crates/unica-coder/src/application/mod.rs` (`tools()`), `application/tool_contracts.rs`, `application/operation_descriptors.rs`, `plugins/unica/skills/<name>/SKILL.md` |
| 1C XML or DSL format change | `0126-platform-8-3-27-deviation-matrix.md`, plus `plugins/unica/references/specs/` | `crates/unica-coder/src/infrastructure/native_operations/` |
| Cache, workspace state, domain events | ADR-0003, `INV-CACHE-01`…`INV-CACHE-07` | `crates/unica-coder/src/domain/events.rs`, `domain/cache.rs`, `infrastructure/workspace_state.rs`, `infrastructure/workspace.rs` |
| Hidden workspace service or runtime job | ADR-0006, `architecture/arc42/06-runtime-view.md`, `INV-APP-07` | `crates/unica-coder/src/infrastructure/workspace_services.rs`, `infrastructure/runtime_jobs.rs` |
| Packaging or release | ADR-0008, ADR-0012, `INV-PKG-01`…`INV-PKG-08`, plus `docs/release-runbook.md` | `scripts/ci/package-unica-plugin.py`, `crates/unica-bootstrap/src/`, `.github/workflows/unica-plugin-release.yml` |
| OS-specific behavior | ADR-0009, `INV-PLATFORM-01`…`INV-PLATFORM-04` | `crates/unica-coder/src/infrastructure/platform/`, `crates/unica-bootstrap/src/platform/`, guard `scripts/ci/check-rust-platform-boundary.py` |
| The architecture rule itself | `architecture/invariants.md` and a record in `spec/decisions/` | the check named in that entry's `Check` field |

## Releasing

Publishing a version to the public marketplace follows
`docs/release-runbook.md`. Read it before acting on any request to cut, ship,
promote, or finish a release; the step order carries the ADR-0008 guarantee that
the catalog never points at bytes that are not final, and improvising it exposes
consumers to an unverified package.

## Search Hygiene

Do not scan local ignored corpora as part of normal repo understanding:

- `target`
- `.build`
- `dist`
- `docs-local` (except when the task needs official 1C platform documentation)

Dated plan and design trees are tracked, but they record how a past change was
made, not how the system behaves now. Do not scan them either:

- `docs/design/**`
- `docs/plans/**`

Open them only to reconstruct the history of one specific change, and never
derive a current rule from them. The exception is an archived document that a
CI test pins by path — `tests/ci/test_format_profile_contract.py` reads
`docs/design/2026-07-23-platform-8-3-27-format-2-20-design.md`, so
that file is a live contract despite its location. Before treating an archived
document as dead, check with `rg <path> tests/ scripts/`.

Use `rg`/`git ls-files` first. For packaging questions, prefer tracked files plus generated package artifacts over raw filesystem walks.

## Design Documents And Decisions

These rules bind the brainstorming and planning skills. They take precedence
over a skill's own defaults.

**Where the artifacts go.** Design documents go to
`docs/design/YYYY-MM-DD-<topic>-design.md`; implementation plans go to
`docs/plans/YYYY-MM-DD-<feature-name>.md`. This overrides the default location
in the `brainstorming` and `writing-plans` skills. Session scratch stays under
the ignored `.superpowers/`; never `git add -f` a path the repository ignores.

**Everything in `spec/` is normative and nothing outside it is.** A design
document records how a choice was reached, including options that were
rejected. It is not a source of truth on the day it is written and does not
become one later, however recent it is.

**Read the rules before proposing a design.** When exploring project context,
read `spec/architecture/invariants.md` and `spec/decisions/README.md` first. An
approach that conflicts with an accepted decision or a registry entry is either
dropped or proposed together with the record that supersedes it — say which,
explicitly, rather than leaving the conflict for review to find.

**Distil a decision when the contract moves.** After writing a design document
and before asking the user to review it, decide whether the work changes an
architectural contract. It does when it touches any of:

- the set of public `unica.*` tools, their arguments, or their result payloads;
- the MCP server identity or the single-public-server boundary;
- ownership of cache or workspace state, or the domain-event contract;
- the packaging, host, or release contract;
- a layer boundary or any rule carried by the invariant registry.

If it does, write the decision record in `spec/decisions/` in the same commit.
The record is short and normative; the design document stays as its provenance
and is linked from the record's Context. If it does not, say so in the header.

**Every design document opens with three fields:**

```markdown
- Date: `YYYY-MM-DD`
- Status: `draft` | `approved` | `superseded`
- Decision: `ADR-NNNN` | `none — no architectural contract changed`
```

`Decision: none` is a claim that review can reject, not a default. The format is
checked by `tests/ci/test_design_documents.py`.

## Local 1Ci Platform Documentation

For questions about official 1C platform behavior, search the private local
corpus at `docs-local/1ci/8.3.27/en/` before using the network. If the required
guide is absent or `manifest.json` is missing or not marked `"complete": true`,
run `python3.12 scripts/dev/download-1ci-guides.py` from the repository root and
retry the local search.

The corpus is local research material only. Do not commit it, copy it into
`plugins/unica/`, include it in packages, or publish it. The downloader may
fetch `https://kb.1ci.com/bin/download/*` attachments despite that path being
disallowed by `robots.txt`; this is a narrow, explicitly approved exception and
must not be generalized to other disallowed paths.

## Development Rules

- Fix root causes, not symptoms.
- Surface contradictions in assumptions, docs, tests, and runtime behavior.
- Keep the public MCP boundary as one server named `unica` with `unica.*` tools unless an ADR changes that contract.
- Prompt-visible skills stay MCP-first. Direct packaged-script execution paths must not return once a native `unica.*` tool exists, except for documented utility exceptions.
- One plugin directory serves Codex and Claude Code. Keep both manifests at the same version, keep `.mcp.json` host-neutral, and do not add optional manifest or catalog keys without checking that the oldest supported client accepts them; an unrecognized key is a load error there, not a warning.
- A change to the public surface — `unica.*` tools, MCP server identity, skill routing, packaging, or layer boundaries — updates `spec/architecture/invariants.md` and the owning record under `spec/decisions/` in the same change set. Shipping the code and deferring the documentation leaves the registry asserting something the build disproves.
- Reference an invariant or a decision by ID rather than restating its text. Two copies of one rule eventually disagree, and then neither is authoritative.

## Pull-request Topology

- Default to one independently reviewable PR per coherent change, targeting `main` or an explicitly named release branch.
- Do not open a PR whose base is the head branch of another open PR. Do not use child PRs as a queue for review fixes: commit and push those fixes to the existing PR's head branch.
- Before opening a PR, inspect the intended base on GitHub. If it belongs to an open PR, stop and either use that PR's head branch or ask the user for direction; branch names alone are not evidence of an independent base.
- A stacked PR is allowed only when the user explicitly requests a named stack and its merge/rebase order. Each member must explain its parent, standalone review boundary, and closure plan in its PR body.
- If an agent cannot push to the existing PR head, it must provide a patch or ask for access; it must not create a child PR as a workaround.
- A distinct bug discovered during review belongs in an independent `main`-targeted PR or an issue, never in an implicit PR stack.
