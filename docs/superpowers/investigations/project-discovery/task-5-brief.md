### Task 5: Platform XML catalog, bindings, forms, and support providers

**Files:**
- Create: `crates/unica-coder/src/infrastructure/discovery/mod.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/platform_xml.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/platform_callbacks.rs`
- Create: `crates/unica-coder/src/infrastructure/discovery/support.rs`
- Modify: `crates/unica-coder/src/infrastructure/mod.rs`
- Modify: `crates/unica-coder/src/infrastructure/native_operations/common.rs`
- Test: new modules and `tests/fixtures/project_discovery/platform_xml/`

- [ ] **Step 1: Add failing provider fixtures/tests for seven declarative flows**

Fixtures must cover event subscription, form command/action, common command,
scheduled job, HTTP route, exchange-plan subscription, and report/data-
processor form ownership. Include wrong binding, malformed XML, lexical decoy,
registered hard decoy, and source-set mismatch cases.

- [ ] **Step 2: Run and verify RED**

Run: `cargo test --locked -p unica-coder discovery::platform_xml -- --nocapture`

- [ ] **Step 3: Implement typed XML readers**

Read `Configuration.xml` child registrations first, then only registered object
files. Parse local-name namespace-insensitively but retain path and best-effort
line provenance. Return typed metadata and binding facts; never render and
reparse human text. Report malformed registered files as failed material
checks, not absent objects.

- [ ] **Step 4: Add versioned platform callback catalog**

Model platform lifecycle and command callbacks by platform script variant,
metadata kind, module kind, method, export requirement, and signature. This is
a platform API registry, not a business-term dictionary. Unknown callback
variants remain `unknown`.

- [ ] **Step 5: Make support parsing error-aware**

Refactor existing `ParentConfigurations.bin` parsing so missing, malformed,
I/O failure, and explicit not-under-support are distinct typed outcomes. The
legacy display renderer consumes the typed result; discovery receives
`SupportFactState` directly.

- [ ] **Step 6: Re-run and commit**

Run: `cargo test --locked -p unica-coder discovery::platform_xml -- --nocapture`

```bash
git add crates/unica-coder/src/infrastructure tests/fixtures/project_discovery
git -c commit.gpgsign=false commit -m "feat: добавить typed platform xml evidence"
```

## Controller decisions after adversarial audit

The original file list is incomplete. Task 5 must also update the application
discovery model, determinism, graph/validator/use case, active spec, and product
contract tests where required. Infrastructure-only GREEN is not acceptable.

### Snapshot-bound execution

- Every provider reads only through Task 4's captured analysis snapshot and
  `read_verified`/`read_optional_verified`; providers do not reopen arbitrary
  workspace paths or manufacture freshness values.
- Provider collection verifies that every record's source-set and fingerprint
  exactly match a linked snapshot. A hash/path mismatch is retryable
  unavailable, never complete/no-match. Source-set mismatch is rejected before
  I/O.
- Snapshot manifest selection and Task 5 catalog parsing share one typed
  `Configuration.xml` registration parser.

### Required application-model corrections

- Replace the direct `ExchangePlan + handles` shortcut with a typed
  `SubscriptionSource` binding: `ExchangePlan.P --uses-->
  EventSubscription.S --subscribes--> CommonModule.M.Handler`. Add a stable
  determinism tag and allow this detail only as `uses` from
  `MetadataCatalogPort`. Direct exchange callbacks remain unsupported v1.
- Proposal existence/materiality is artifact-kind specific. `method` uses
  Definition plus its registered owner; declarative artifacts use exact
  `MetadataPresent`; a form command additionally requires FormInspection.
  `DefinitionPort` is not material for declarative target existence.
- Replace the method-centric owner helper with an `ArtifactOwnershipChain`
  that preserves specialized Report, DataProcessor, CommonCommand,
  ExchangePlan, form, module, and method identities.
- Platform callbacks are runtime edges only when a matching definition proves
  procedure/function kind, export, arity, by-value/default signature, and
  module-kind compatibility. Parameter names are irrelevant. Add `is_function`
  and typed signature requirements; mismatch is
  `callback_signature_mismatch`, not generic no-match.
- Binding smart constructors validate endpoints and bounded payload fields:
  subscription/job/route/form-command owners and handler kinds must agree;
  event/action/template are non-blank and control-free; form action
  `callType` is typed and included in evidence digests.
- Support providers return raw source state only. Proposal policy derives
  `ExtensionRequired`/`ExtensionOwned` per mutation intent so two proposals do
  not create conflicting raw support facts.

### Authoritative Platform XML flows

- Event subscription: exact direct registration and descriptor produce
  metadata presence plus `subscribes` to an exact CommonModule method/event.
- Form command: registered specialized owner -> registered form -> command ->
  FormModule action, with exact action and typed callType/client context.
- Common command: registered descriptor/module plus a versioned platform
  callback, gated by definition signature.
- Scheduled job: exact handler and enabled state; disabled is observed but not
  runtime-connected.
- HTTP service route: registered service/route -> exact module handler with
  typed verb and normalized RootURL+URLTemplate.
- Exchange subscription: exact QName-resolved source type produces the `uses`
  edge to a registered EventSubscription, then its handler edge.
- Report/DataProcessor form: preserve specialized owner identity through the
  registered form/command/action chain.

Read only direct `Configuration/ChildObjects` and direct nested owner
registrations. Build paths through `discovery_registry`, require descriptor
kind and `<Properties><Name>` to match, resolve QName namespace URIs rather than
prefix spelling, ignore comments/strings/unregistered decoys, and return 1-based
workspace-relative provenance at the material XML field. Malformed registered
metadata fails its material port; malformed registered Form.xml fails
FormInspection without inventing absence.

### Callback registry boundary

Use a pure versioned registry
`ScriptVariant x MetadataKind x ModuleKind x Method -> function/export/signature`.
`ScriptVariant` is Russian/English script naming from the configuration, not a
platform release number. Include only platform-owned lifecycle/command
callbacks verified by fixtures. Unknown variants/signatures produce scoped
`unsupported_platform_script_variant`/`unsupported_mechanism_variant` unknown
outcomes; do not guess business synonyms.

### Typed support-state refactor

Model `ParentConfigurationsRead` as Missing, Parsed(under support or explicit
not-under-support), Malformed(reason), or IoFailure(reason,retryable). Model
object rules as Locked/Editable/Removed rather than numeric values. Consume
Task 4's optional tombstone: verified missing base state is NotUnderSupport;
verified missing extension state is ExtensionOwned; malformed is Failed;
I/O/fingerprint mismatch is retryable Unavailable.

Legacy renderers consume the typed outcome. Malformed/I/O must not render as
"not under support"; `support_guard_violation` becomes error-aware and cannot
fail open; support-edit shares the parser; invalid UTF-8 and short garbage are
malformed rather than automatically removed from support.

### Required test matrix and delivery

- For each of seven flows: positive, valid alternative, wrong exact binding,
  disabled state where applicable, malformed registered material, lexical
  decoy, registered hard decoy, valid/malformed unregistered decoy, source-set
  mismatch, snapshot mutation before read, stable ordering and exact location.
- Add Russian/English/unknown script variants; module/signature/function/export
  mismatches; parameter rename compatibility; QName prefix variation; duplicate
  registration and descriptor mismatch; full raw support-state/error matrix;
  malformed support blocks legacy guard; direct exchange callback remains
  unknown.
- Follow strict RED -> GREEN -> REFACTOR. Record the RED output, focused/full
  tests, fmt, clippy, commit SHA, and risks in
  `.superpowers/sdd/task-5-report.md`.
- Commit Task 5 with `feat: добавить typed platform xml evidence`; include every
  required application/spec/test file, not only the original infrastructure
  list.
