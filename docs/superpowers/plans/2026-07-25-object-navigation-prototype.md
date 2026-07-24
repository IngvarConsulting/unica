# Object navigation prototype — implementation plan

> **Scope note:** this is a bounded vertical slice of the semantic object model.
> It makes the model inspectable through the existing `unica.meta.info` result,
> but it does not introduce a new public tool or claim that the modeled mutations
> are executable yet.

## Goal

Prove the public navigation shape needed for typed 1C configuration editing:

```
NodeKind + RelationKind + state -> semantic actions
```

The prototype must derive a graph from a real Platform XML metadata object,
preserve the existing human-readable `meta.info` output, and provide the graph
as structured result data. Identity is logical (configured source set or a
deterministic opaque ad-hoc scope, owner chain, kind, name), never an XML path.

## Non-goals

- No `unica.project.graph` tool and no change to the one-server `unica.*` MCP
  boundary.
- No public `compile`, `decompile`, or standalone `validate` replacement in
  this change.
- No executable mutations: modeled semantic actions are a contract prototype,
  not promises of currently callable tools.
- Existing `compile`/`decompile`/`validate` tools remain legacy transport in
  this slice. A later per-action migration requires an atomic native operation,
  internal validation, compatible corpus evidence, an explicit client
  migration, and a deprecation period before removal.
- No EDT parser. A graph produced from Platform XML explicitly records that
  representation; EDT sources remain rejected by the native XML boundary.
- No duplicate list of the 45 canonical metadata kinds. The existing
  infrastructure registry remains the source of truth for parseable top-level
  kinds.

## 1. Add the pure domain ontology

**Files:**

- Modify `crates/unica-coder/src/domain/mod.rs`
- Create `crates/unica-coder/src/domain/navigation.rs`

Define serializable, path-free types:

- `NavigationGraph`, `NavigationNode`, `NavigationEdge`, `ObjectRef`;
- `NodeKind` for an aggregate metadata object, metadata child, form, form
  member, and typed/generic template;
- `RelationKind`, at least `contains` and `references`;
- `SemanticAction` and separate node/relation action catalogues;
- `CapabilityState` that distinguishes source resolution from authorability;
- node/edge state and a `PlatformXml`/`Edt` representation marker.

The action catalogue must distinguish semantic actions by target type and
relation. For example, a document can model adding MXL/tabular sections, while
`move` and `bind` belong to a specific containment/reference relation rather
than a form-element node. `clone` is discoverable from a source node but commits
the owning containment relation atomically, because it creates a sibling and a
registration. It must not expose generic actions for unresolved, support-locked,
globally read-only, or unmodeled node classes.

Add focused unit tests that prove containment/reference are distinct and that
action sets are node-kind/state dependent.

## 2. Project a Platform XML object into the graph

**Files:**

- Modify `crates/unica-coder/src/infrastructure/native_operations/meta.rs`
- Modify `crates/unica-coder/src/infrastructure/native_operations/typed_result.rs`

Reuse the existing `meta.info` XML parsing path. Build a graph whose root is
the metadata object and whose child nodes preserve source order for:

- attributes;
- tabular sections and their attributes;
- forms;
- templates, retaining descriptor type evidence rather than assuming MXL;
- commands when present.

Derive source-set context from the configured project map where available, but
do not use the resolved filesystem path as an object identifier. Normalize
adapter-private paths before source-set lookup; give an unconfigured ad-hoc
object a deterministic opaque scope rather than a collision-prone `workspace`
fallback. A registered form/template with no resolvable backing content is
marked unresolved; a root, Form, or Template must structurally prove its typed
descriptor/name (and MXL its canonical direct content) before it becomes
resolved. Registration values must be validated as 1C identifiers before
filesystem probing. Support state must become authorability state for
each descriptor and be combined conservatively with its owner, so locked,
unreadable, or read-only objects do not advertise mutations.

Keep the current `AdapterOutcome` and its stdout unchanged. Extend the typed
native-result plumbing so a non-mutating `meta-info` operation returns
`OperationResult.data = { "navigation": ... }` alongside that stdout.

## 3. Lock the external contract with tests

**Files:**

- Modify/add focused tests next to `meta.rs`
- Modify `crates/unica-coder/src/application/mod.rs` boundary tests

Write tests before each implementation slice and observe them fail first.
Cover:

1. Typed containment references and source order for a document-like object.
2. A tabular-section attribute’s owner chain.
3. Template classification only when descriptor type evidence exists.
4. An unresolved registered form not offering semantic form-content mutation.
5. `unica.meta.info` retaining existing human stdout while exposing the
   navigation graph in `OperationResult.data`.
6. Relation-specific move/bind capabilities, MXL specialization, child support
   lock, unreadable support state, opaque ad-hoc identity, traversal rejection,
   typed descriptor proof, invalid-root fail-close, and `dryRun`
   safe-placeholder preservation.

Run, at minimum:

```bash
cargo test -p unica-coder navigation
cargo test -p unica-coder meta_info
cargo test -p unica-coder
cargo build -p unica-coder
```

## 4. Review the prototype boundary

Inspect the final diff against the goal and non-goals. Confirm specifically:

- no new MCP server/tool or package contract drift;
- no raw script fallback or XML paths in public identity;
- modeled actions are visibly prototype semantics, not incorrectly advertised
  as executable operations (in particular, `remove` remains deferred until the
  model has an explicit off-support eligibility state);
- Platform XML and EDT are not conflated;
- legacy `meta.info` text output remains compatible.

Commit only the prototype’s isolated worktree changes after the full test suite
passes. Do not merge or publish without a separate user instruction.
