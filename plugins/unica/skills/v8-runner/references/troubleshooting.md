# Troubleshooting

Classify failures before retrying:

- License text means hard stop. Do not repair licensing automatically.
- Authentication failure without credentials allows only `Администратор` with empty password, then `Admin` with empty password, then ask the user.
- Missing platform, runner, tool, VA, or MCP extension is an environment/setup issue; report the exact missing component.
- Stale generated state after branch switch or rebase should use `build` with `fullRebuild=true`.
- A default `build` runs the normal strategy first. One full retry is automatic only when external exit code `4` accompanies a valid structured failure for the completed partial load step. Treat that as stage evidence, not a diagnosis of vendor support or any other cause.
- Do not retry arbitrary or malformed errors, process spawn failures, cancellation, timeout, truncated output, or failures from another build step. Explicit `fullRebuild=true` and a failed full fallback also do not start another attempt.
- The support-independent fallback is temporary; the runtime/runner redesign for v14 is a separate change.
- Unexpected source changes after dump should be reviewed as a Git diff before continuing.

Do not bypass typed MCP arguments with raw shell flags. If the needed v8-runner flag is missing from `unica.runtime.execute`, treat that as a Unica MCP contract gap.
