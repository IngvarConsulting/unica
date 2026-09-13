- Date: `2026-08-31`
- Status: `approved`
- Decision: `DEC.2026-08-31.V0-13-FIRST-IMPLEMENTATION-VERTICALS`

# v0.13 first implementation verticals

## Objective

Move the already-selected 8/11 v0.13 surface from one useful mode per tool to
the first connected set of production workflows. The implementation is grouped
by workflows, not by legacy tool names: one metadata mutation pipeline, one
logical read/projection pipeline, and one bounded runtime operation.

`plugins/unica/skills/**` remains outside this iteration. `query.execute` is
absent from the v0.13 dictionary by explicit user decision; a direct call is an
unknown canonical operation and returns typed `unsupported_operation`.

## Implementation truth

The result-envelope field `contract:"typed"` describes the result shape. It
does not mean that every operation or projection behind a tool is implemented.
The repository therefore gains a separate machine-readable implementation
coverage record. Each closed mode is one of `supported`, `partial`,
`unsupported`, or `removed` and names executable test evidence. CI rejects a
`supported` entry without evidence and rejects a public mode missing from the
coverage record.

## Shared apply pipeline

All implemented metadata operations use one pipeline:

```text
parse -> resolve -> validate -> plan -> stage -> prepare -> publish -> effects
```

`dryRun:true` and publication run the same parser, resolver, validators and
family planner. Dry-run stops after preparation and returns the exact planned
changes/effects without publishing source, cache or revision state. Real apply
commits the existing actor-owned retained Source + WorkspaceCache transaction.
There is no second direct filesystem writer and no legacy MCP dispatch.

The first candidate metadata operations were:

- `props.set`;
- `relation.add`, `relation.remove`, `relation.replace`;
- `attribute.add`, `attribute.set`, `attribute.remove`;
- `object.create`, `object.remove`.

Their arguments are closed typed unions derived from the existing metadata
model. Operations retain request order and the whole request is atomic. Any
other registered operation continues to return typed `unsupported_operation`
at the exact `ops[i]` path.

Planning evidence narrowed this candidate set, and integrated actor tests now
prove retained publication plus identical dry-run/real plan hashes for
`props.set` and `attribute.add/set/remove`. They do not prove `object.create/remove`: the
template/removal publishers do not expose a retained planner. They also do not
prove `relation.*`: the public skeleton does not identify the relation while
the existing writer requires relation-specific dependency evidence. Those five
operations therefore remain exact typed unsupported in this iteration rather
than acquiring an invented schema. This is a discovered contract gap, not an
implementation shortcut.

## First read vertical

- `view.filter.sections` exposes selected typed sections of a resolved logical
  node and preserves the base-view revision.
- `search.scope` accepts the configuration root or one metadata-object subtree.
  Descendant scopes remain `unsupported_scope` until each match can carry an
  exact logical owner; a physical path is never accepted or returned.
- `check.filter.validation.profile` parses a closed reserved union but returns
  `unsupported_operation` until a real canonical validator is wired. Base
  admission/readability check remains useful without claiming validity.
- `diff.filter` supports closed path/section selection before structural
  comparison; unsupported filter members remain typed errors.

These projections share actor-owned read authorities and cancellation. They do
not start hidden analysis work from a persisted-read operation.

## First run vertical

Only `unica.run({"op":"syntax.check",...})` becomes executable. It delegates
to the existing runtime adapter with an explicit five-minute process timeout
and bounded capture, accepts a closed typed argument union, returns a sanitized
terminal canonical result, and always hands valid work to the existing durable
Invocation/Task path. The dictionary marks only `syntax.check` as implemented.
Raw stdout/stderr, command lines, artifact paths, credentials, unbounded client
sessions, tool downloads, and query execution are not published.

## Parallel ownership

The integrator owns `v13_service.rs`, daemon module wiring, actor publication,
architecture records, and aggregate tests. Independent workers own:

1. metadata-family parsing/planning and its focused tests;
2. logical read projections and their focused tests;
3. the bounded `syntax.check` adapter and its focused tests.

Workers do not edit shared integration files or skill files. Integration occurs
only after each worker demonstrates the intended failing test and then the
focused green test.

## Acceptance

The iteration is complete when:

1. coverage JSON and its CI guard agree with the selected 8/11 catalog;
2. `props.set` and `attribute.add/set/remove` have identical dry-run/real plans
   and atomic retained publication evidence; the five underdefined operations
   remain exact typed unsupported with named contract gaps;
3. sections, bounded object-subtree search, base readability check and filtered
   diff have positive tests; descendant search and validation profiles have
   typed-rejection tests;
4. `syntax.check` is the only implemented run operation and has bounded Task,
   cancellation and sanitized terminal/provider evidence;
5. `query.execute` is absent from the v0.13 Run dictionary;
6. exact 8/11 surface, architecture registry, package verification and all
   affected Rust/Python suites pass.
