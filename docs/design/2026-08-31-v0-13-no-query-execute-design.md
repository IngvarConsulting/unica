- Date: `2026-08-31`
- Status: `approved`
- Decision: `DEC.2026-08-31.V0-13-NO-QUERY-EXECUTE`

# Unica v0.13 does not publish query execution

## Decision

`query.execute` is absent from the closed `unica.run` operation dictionary in
v0.13. The v0.13 catalog contains twelve runtime intentions, of which only
`syntax.check` is currently implemented. A direct request using
`op="query.execute"` is handled as an unknown canonical operation and returns
typed `unsupported_operation`; no alias, hidden successor, or capability entry
is supplied.

The v0.12 query-execution capability has no successor in v0.13. Historical
plans remain historical; active coverage, parity oracles and generated surface
material must not claim that the operation is part of the v0.13 contract.

## Boundaries

- The eight subject tools and three compatibility Task tools do not change.
- `unica.run` remains the runtime entry point and still exposes its other twelve
  closed intentions.
- No query parser, executor, provider capability, skill or migration shim is
  added in this iteration.
- `plugins/unica/skills/**` remains outside scope.

## Executable evidence

1. The Rust catalog test requires exactly twelve names and rejects
   `query.execute` as a dictionary member.
2. The production daemon test requires dictionary discovery to omit the name
   and a direct call to return `unsupported_operation`.
3. Coverage and parity tests use the same exact twelve-operation oracle.
4. Package verification continues to prove the unchanged 8/11 MCP tool set.
