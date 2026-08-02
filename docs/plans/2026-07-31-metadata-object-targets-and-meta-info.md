# Metadata Object Targets and `meta.info` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:subagent-driven-development` to implement this plan task by task.
> Every production change starts with a failing test, records the observed RED
> reason, and ends with the narrow GREEN command before broader verification.

**Goal:** Make a metadata object a first-class resolvable target of the Platform
XML provider, and spend that capability on the first read tool: `unica.meta.info`
accepts `sourceSet + metadataPath`, reports owners, and stops publishing an
argument it never reads. Closes
[#273](https://github.com/IngvarConsulting/unica/issues/273) and
[#274](https://github.com/IngvarConsulting/unica/issues/274), and delivers the
first slice of [#272](https://github.com/IngvarConsulting/unica/issues/272).

**Architecture:** One provider-side resolver gains an object branch and an
explicit target-kind policy; the write surface stays module-only by declaration
instead of by accident. `unica.source.resources` reports the resolver's real
refusal instead of collapsing every one of them into `source_unavailable`.
`unica.meta.info` resolves its descriptor through the same closed handle and
keeps its physical location only as an observed artifact, never as a selector.

**Tech Stack:** Rust 2021, `roxmltree`, existing
`domain/source_target.rs` + `infrastructure/platform_xml_source_targets.rs` +
`infrastructure/platform_xml_resources.rs` seams, Python CI contract tests,
Markdown ADR/invariant/acceptance corpus.

---

## Why these three issues are one merge request

They are one capability gap seen from three sides.

- #273 fails because `resolve_platform_xml_target`
  ([platform_xml_source_targets.rs:1957](../../crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs))
  accepts only a module terminal, and because `open_snapshot`
  ([platform_xml_resources.rs:234](../../crates/unica-coder/src/infrastructure/platform_xml_resources.rs))
  maps every `SourceTargetError` to `source_unavailable`.
- #272 fails for `meta.info` because there is no way to turn `Catalog.X` into
  the descriptor the reader needs — the same missing object branch.
- #274 is the payload: once `meta.info` is reachable by a logical address, its
  answer must actually contain what a subordinate catalog is, and must not
  publish `Detailed`, which no `meta.info` code path reads.

Fixing #273 without #272 leaves the capability with one consumer that already
had a workaround. Fixing #272 without #274 migrates a tool whose answer is still
missing the property that sent the reporter to the raw XML.

## Scope boundary

**In:** the object branch of the resolver, object resource evidence,
`source.resources` error fidelity and object scopes, `unica.meta.info` selector
migration, `meta.info` owners/presence output, contract and skill sync.

**Out, with reason** — the four remaining read tools from #272 each need a
*separate* address or role decision, and ADR-0021 §13 requires one
self-verifiable tool or coherent group per merge request:

| Tool | What it still needs | Why not here |
| --- | --- | --- |
| `form.info` | resource role `form`: `<Kind>/<Obj>/Forms/<F>/Ext/Form.xml`, not the `Forms/<F>.xml` descriptor | address exists (`Catalog.X.Form.Y`), the *resource selection* does not |
| `role.info` | `Roles/<R>/Ext/Rights.xml` under address `Role.R` | needs a rights resource role; `Role.R` alone resolves to the descriptor |
| `subsystem.info` | nested `Subsystem.A.Subsystem.B` | address grammar nests only `Form` and `Command` ([source_target.rs:181](../../crates/unica-coder/src/domain/source_target.rs)) |
| `dcs.info` | `Report.X.Template.Y` → `Templates/Y/Ext/Template.xml` | needs a new nested kind `Template`, which ADR-0021 §6 requires proving by fixture |

Each becomes its own slice on top of the evidence seam this plan introduces.
Predefined items for `meta.info` are also deferred: they live in
`<Kind>/<Obj>/Ext/Predefined.xml`, a second resource, and belong with the
resource-role work above.

## Global constraints

- One merge request migrates one tool. After the switch the old selector is not
  a second public contract: `ObjectPath`/`objectPath`/`Path`/`path` on
  `unica.meta.info` fail with `legacy_target_removed` naming
  `sourceSet + metadataPath` (ADR-0021 §13).
- Resolving an object must not widen write authority. `unica.code.patch` and
  every guard path keep refusing a non-module address, and that refusal becomes
  an explicit declaration rather than a side effect of the module-only check.
- Public `source.*` results keep disclosing no physical path, no closed handle,
  no provider revision (`INV-MCP-SOURCE-SURFACE`).
- `meta.info` may keep reporting the resolved descriptor as an *observed*
  artifact, per ADR-0021 §9, but its identity in the answer is
  `sourceSet` + canonical `metadataPath`.
- Error codes added to `SourceResourceErrorCode` are stable public strings and
  reveal only what `unica.source.resolve` already reveals to the same caller.
- Do not weaken the existing snapshot bounds: 100 resources, 50 per page,
  64 KiB per read, five-minute lifetime, per-snapshot and live byte ceilings.

---

## Task 1: Resolve a metadata object, and declare the module-only write policy

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/platform_xml_source_targets.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/code.rs`

**Contract:**

- Add `TargetKindPolicy { ModuleOnly, Any }` as a required argument of
  `resolve_platform_xml_target`. `ModuleOnly` reproduces today's
  `TargetKindMismatch` verbatim for `sourceRoot`-relative object addresses.
- `code-patch` resolution (`code_patch_source_target` consumers,
  `resolve_code_patch_guard_path`, `guard_code_patch_resolved_target`) passes
  `ModuleOnly`. No caller of the write path may pass `Any`.
- Under `Any`, a two-segment address resolves to
  `<kind directory>/<Name>.xml` and a four-segment `Form`/`Command` address to
  `<kind directory>/<Owner>/Forms|Commands/<Name>.xml`. Both reuse the existing
  descriptor evidence (`descriptor_outcome`, `metadata_descriptor`,
  `metadata_kind`) — one mapping, not a second copy alongside
  `exact_object_outcome`.
- The object branch proves descriptor kind and name, refuses links and
  non-regular files, re-checks containment against the selected source root, and
  returns `ResolvedTarget { target_kind: MetadataObject }`.
- `platform_xml_resource_evidence` returns, for an object handle: the descriptor
  as `target_path`, the object's proven module and nested descriptor paths as
  `descriptor_paths`, and the unchanged `Configuration.xml` registration path.
- `revalidate_platform_xml_target` re-runs with the same policy the handle was
  created under; a handle created as `MetadataObject` can never revalidate into
  a module write target.

**TDD sequence:**

1. Add resolver tests: `Catalog.Items` resolves under `Any` with
   `targetKind: metadataObject`; the same address under `ModuleOnly` still
   returns `TargetKindMismatch`; a `Catalog.Items.Form.ListForm` address
   resolves; a descriptor whose `<Name>` disagrees with the address is refused;
   a symlinked descriptor is refused; a missing descriptor returns
   `MetadataAddressNotFound`.
2. Add a `code.patch` test proving `metadataPath: "Catalog.Items"` is refused
   with the target-kind error and writes nothing.
3. Run and observe RED:
   `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1`.
4. Implement the policy argument and the object branch.
5. Run:
   `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1`,
   `cargo test -p unica-coder code_patch -- --test-threads=1`.
6. Commit: `feat(source): resolve metadata object targets`.

## Task 2: Report the real refusal from `unica.source.resources`

**Files:**

- Modify: `crates/unica-coder/src/domain/source_resources.rs`
- Modify: `crates/unica-coder/src/infrastructure/platform_xml_resources.rs`

**Contract:**

- Add `SourceResourceErrorCode::TargetNotFound` (`target_not_found`) and
  `SourceResourceErrorCode::TargetKindUnsupported`
  (`target_kind_unsupported`).
- `open_snapshot` maps `SourceTargetError` instead of discarding it:
  `SourceSetRequired`/`MetadataAddressInvalid`/`AddressProfileUnsupported` →
  `invalid_request`;
  `SourceSetNotFound`/`SourceRootNotAddressable`/`MetadataAddressNotFound` →
  `target_not_found`; `TargetKindMismatch` → `target_kind_unsupported`;
  `ContainmentDenied` → `containment_denied`. `source_unavailable` remains only
  for genuine unavailability (evidence read failure, workspace identity, store
  lock).
- The message carries the logical address and source set, never a physical path.
- Object targets get scopes: `self` → one `metadataDescriptor`, completeness
  `complete`; `aggregate` → descriptor plus proven modules and nested
  descriptors, completeness `partial`; `registrations` → the configuration
  registration, completeness `partial`. Every resource stays `read`-only.
- The `(_, _) => unavailable` fallback arm keeps a test proving which
  combinations still land there.

**TDD sequence:**

1. Add provider tests: `Catalog.Items` + `self` yields exactly one
   `metadataDescriptor`; `aggregate` also yields its object module; an unknown
   address yields `target_not_found`; a module address with an unsupported scope
   yields `target_kind_unsupported`; a denied containment yields
   `containment_denied`; no error message contains a path separator.
2. Add a domain test pinning the two new code strings.
3. Run and observe RED:
   `cargo test -p unica-coder source_resources -- --test-threads=1`.
4. Implement the mapping and the object scopes.
5. Run:
   `cargo test -p unica-coder source_resources -- --test-threads=1`,
   `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1`.
6. Commit: `fix(source): report metadata object targets and real refusals`.

## Task 3: Migrate the `unica.meta.info` selector

**Files:**

- Modify: `crates/unica-coder/src/application/tool_contracts.rs`
- Modify: `crates/unica-coder/src/application/operation_descriptors.rs`
- Modify: `crates/unica-coder/src/infrastructure/format_guard.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Contract:**

- Publish a narrow `META_INFO_ARGS` next to `CODE_PATCH_ARGS` instead of the
  shared `NATIVE_XML_DSL_ARGS`: `sourceSet`, `metadataPath`, `Mode`/`mode`,
  `Name`/`name`, `Limit`/`limit`, `Offset`/`offset`, plus `COMMON_ARGS`. Nothing
  else — this is also the fix for the `Detailed` half of #274, since the
  argument's own description already scopes it to the `*.validate` tools.
- Extend `validate_removed_target_arguments` so `unica.meta.info` with
  `ObjectPath`, `objectPath`, `Path` or `path` fails with
  `legacy_target_removed` naming `sourceSet + metadataPath`.
- `meta-info` required args become `sourceSet` + `metadataPath`; its format
  dependency stays `HandlerResolved` and resolves through the Task 1 resolver in
  `format_guard.rs`, alongside the existing `code-patch` arm.
- `analyze_meta_info` resolves the descriptor from the logical target, keeps
  `Поддержка:` and pagination unchanged, and prefixes the answer with the
  resolved `sourceSet` and canonical `metadataPath`. The descriptor path stays
  in `artifacts` as an observed location.
- Drill-down by `Name` is unchanged; it is a projection of the resolved object,
  not a second selector.

**TDD sequence:**

1. Change contract tests first: `meta.info` accepts `sourceSet + metadataPath`,
   rejects each legacy field with `legacy_target_removed`, rejects `Detailed`
   and `OutFile` with the unknown-argument error, and `ARG_DESCRIPTIONS` has no
   stale entry. Update the `read_only_native_tools_reject_out_file_arguments`
   table, which currently keys `meta.info` on `ObjectPath`.
2. Rewrite the `meta.info` handler tests onto workspaces that carry a
   `v8project.yaml` source set, mirroring the `fixture(...)` helper in
   `platform_xml_source_targets.rs` tests. Assert byte-identical output to the
   pre-migration answer for the same object.
3. Run and observe RED:
   `cargo test -p unica-coder meta_info -- --test-threads=1`.
4. Implement schema, descriptor, guard and handler migration.
5. Run:
   `cargo test -p unica-coder meta_info -- --test-threads=1`,
   `cargo test -p unica-coder application::tests -- --test-threads=1`,
   `cargo test -p unica-coder format_guard -- --test-threads=1`.
6. Commit: `feat(meta): address meta.info targets logically`.

## Task 4: Answer what a subordinate object is

**Files:**

- Modify: `crates/unica-coder/src/infrastructure/native_operations/meta.rs`

**Contract:**

- Print owners for every descriptor carrying `<Owners>`: `Владельцы: Catalog.A,
  Catalog.B` when present, and an explicit `Владельцы: нет` when the element is
  empty. Owners are a first-class property of the object, so they appear in the
  default `overview` mode, not only in `full`.
- Make the catalog header unambiguous instead of silent. Today
  `meta_info_append_catalog_header` prints `Иерархический` only when true and
  the code/description lengths only when non-zero, so a reader cannot tell "not
  hierarchical" from "not reported" — which is what sent #274 to the raw XML.
  Report the negative cases explicitly.
- Add `Основное представление` from `<DefaultPresentation>`
  (`AsDescription`/`AsCode`) for the reference kinds that publish it.
- List forms in `overview` for every object kind that has them, not only for
  `Report`/`DataProcessor`.
- No new argument is introduced for any of this; `Mode` remains the single
  detail lever.

**TDD sequence:**

1. Add handler tests: a catalog subordinate to two owners lists both; a catalog
   with an empty `<Owners/>` says so; a non-hierarchical catalog reports it
   explicitly; `Основное представление` appears for a catalog that sets it;
   forms appear for a catalog in `overview`.
2. Run and observe RED:
   `cargo test -p unica-coder meta_info -- --test-threads=1`.
3. Implement the output changes.
4. Run:
   `cargo test -p unica-coder meta_info -- --test-threads=1`,
   `cargo test -p unica-coder infrastructure::native_operations::meta -- --test-threads=1`.
5. Commit: `feat(meta): report owners and explicit object properties`.

## Task 5: Synchronize the public surface

**Files:**

- Modify: `plugins/unica/skills/meta-info/SKILL.md`
- Modify: the fourteen other skills that call `unica.meta.info`
  (`meta-edit`, `api-design`, `autonomous-server`, `background-jobs`,
  `bsp-patterns`, `data-exchange`, `data-separation`, `db-performance`,
  `integration-implement`, `log-analysis`, `query-optimize`,
  `release-support`, `security-auth-crypto`, `support-edit`)
- Modify: `plugins/unica/README.md`
- Modify: `spec/decisions/0021-logical-source-addressing.md`,
  `spec/decisions/0022-bounded-source-resource-access.md`
- Modify: `spec/architecture/invariants.md`
- Modify: `spec/acceptance/logical-source-addressing-and-resource-access.md`
- Modify: `tests/ci/test_unica_skills.py`,
  `tests/ci/test_unica_mcp_script_parity.py`,
  `tests/ci/test_architecture_registry.py`

**Contract:**

- Every `tools/call` example for `unica.meta.info` uses
  `sourceSet + metadataPath` and still executes as an MCP dry run
  (`INV-SKILL-EXECUTABLE-EXAMPLES`). The `meta-info` skill states how to obtain a
  source-set name (`unica.project.map`) and an address
  (`unica.source.resolve`), and that `unica.source.locate` converts a path found
  by other means.
- The README migration table gains a `unica.meta.info` row:
  `ObjectPath` → `sourceSet` + `metadataPath`.
- ADR-0021 records the second migrated tool; the ADRs are amended by their own
  rules, not rewritten. ADR-0022 records the two new error codes and object
  scopes.
- Acceptance gains: object target resolution, module-only write policy,
  refusal-code fidelity, and the migrated `meta.info` contract.

**TDD sequence:**

1. Adjust the CI assertions first and observe failures:
   `python3.12 -m unittest tests/ci/test_unica_skills.py tests/ci/test_unica_mcp_script_parity.py tests/ci/test_architecture_registry.py`.
2. Update skills, README, ADRs, invariants and acceptance.
3. Run the same command until green, then:
   `python3.12 scripts/ci/check-architecture-sync.py --base origin/main`.
4. Commit: `docs(architecture): record metadata object addressing`.

## Task 6: Prove it end to end and run the regression suite

**Files:**

- Modify: `tests/ci/test_unica_mcp_smoke.py`
- Modify: `scripts/ci/smoke-unica-mcp.py`

**Contract:**

- Through JSON-RPC `tools/call`: `source.resolve` an object, feed the returned
  address to `source.resources` with `self` and to `meta.info`, and assert both
  succeed on the same address without any physical path in the request.
- Assert the legacy `ObjectPath` call fails with `legacy_target_removed`.
- Assert an unknown address returns `target_not_found`, not
  `source_unavailable`.

**TDD sequence:**

1. Add the smoke assertions, run and record the failure:
   `python3.12 -m unittest tests/ci/test_unica_mcp_smoke.py`.
2. Complete any missing wiring without weakening lower-level guards.
3. Run: `python3.12 -m unittest tests/ci/test_unica_mcp_smoke.py`,
   `cargo test -p unica-coder -- --test-threads=1`, and the repository CI entry
   point named by `.github/workflows/`.
4. Verify `git diff --check`, a clean worktree, and no leaked physical selector
   in any published schema.
5. Commit: `test(meta): prove logical meta.info end to end`.

---

## Risks

- **Write-path widening.** The resolver currently protects `code.patch` by
  refusing every non-module address. Task 1 removes that accident; the
  `TargetKindPolicy` and its test are what replace it. Do not merge Task 1
  without the `code.patch` refusal test.
- **Fixture churn.** Every `meta.info` test moves to a workspace with a project
  map. This is mechanical but touches many fixtures; keep one shared helper.
- **Breaking change surface.** `meta.info` loses `ObjectPath` and `Detailed` in
  the same release. Both are named in the README table and in the release notes;
  the failure is typed, not a silent behaviour change.
- **Error-code disclosure.** `target_not_found` distinguishes absent from
  unaddressable. This tells the caller nothing that `unica.source.resolve`
  does not already tell them, and the acceptance record must say so.
