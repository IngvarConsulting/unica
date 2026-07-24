# Issue #185: RLM stale-content recovery

## Context

Unica runs RLM index maintenance in the background:

1. `rlm-bsl-index index info`
2. `index update` when the index is stale
3. another `index info` after the update

With bundled `rlm-tools-bsl` 1.26.0, the Git fast path can make `update`
perform no file work when the indexed Git HEAD matches the current HEAD.
If file modification times drifted while contents and sizes stayed unchanged,
the following strict `info` check can still report `stale (content)`.

The current worker records that terminal state as `failed`, but the next tool
request immediately starts another update. The failed marker is overwritten
with `building`, and the adapter maps every stale readiness state to
`rlm index building`. This creates an endless
`update -> stale (content) -> update` loop and hides the actual failure.

This design addresses [issue #185](https://github.com/IngvarConsulting/unica/issues/185)
inside Unica without weakening RLM freshness checks or changing the bundled
RLM command-line contract.

## Goals

- Recover automatically when a successful incremental update leaves the index
  in `stale (content)`.
- Return `rlm index building` only while an index worker actually owns an
  active lock.
- Preserve the original stale-content reason in recovery diagnostics.
- Surface a terminal failed marker instead of starting an endless retry loop.
- Keep cancellation, timeout, tool-execution, and other operational failures
  retryable after their cause is removed.
- Keep `RLM_INDEX_SAMPLE_SIZE` and strict freshness detection enabled.
- Preserve the existing behavior for missing, fresh, and actively building
  indexes.

## Non-goals

- Changing `rlm-tools-bsl` or adding a new `update --full-scan` option.
- Treating a stale-content index as ready for normal reads.
- Implementing content hashes or a metadata-only mtime refresh in Unica.
- Automatically retrying a terminal recovery failure.
- Refactoring unrelated workspace-service or adapter behavior.

## Considered approaches

### 1. Update followed by a full-build fallback in Unica

After a successful `update`, inspect the exact RLM status. If it is
`stale (content)`, run `build` in the same background job while retaining the
same Unica lock, then inspect the index again.

This is the selected approach. It is compatible with the bundled CLI,
guarantees that stored file metadata is refreshed, and confines the change to
Unica.

### 2. Read a stale-content index

RLM can technically read an existing database after a stale-content result.
This is cheaper, but it leaves the index in a dirty state, repeats the same
maintenance attempt on later calls, and may expose genuinely outdated content.
It is rejected for this issue.

### 3. Add full-scan or metadata-refresh support to RLM

The most efficient long-term solution is an RLM operation that bypasses the
Git fast path or refreshes metadata after confirming identical content. The
bundled 1.26.0 CLI exposes no such operation. This requires a separate RLM
change, release, toolchain build, and package lock update, so it is outside the
scope of issue #185.

## Design

### Exact stale status

The index-info parser must retain the normalized RLM status value instead of
collapsing all values beginning with `stale` into one undifferentiated state.
The worker must be able to distinguish at least:

- `stale (content)`
- `stale (age)`
- `stale (structure changed)`

Only `stale (content)` after a successful update activates the full-build
fallback. Other stale statuses continue through the existing incremental
update policy.

The exact status text is internal diagnostic data. The public MCP tool names
and the single public `unica` server contract do not change.

### Recovery job

An update background job carries its normal update command, the existing info
command, and a recovery build command. A build job has no recovery command.

The update state machine is:

```text
update
  |-- command failed ------------------------------> failed (retryable)
  |
  `-- command succeeded
        |
        `-- info
              |-- fresh ---------------------------> ready
              |-- stale (content)
              |      |
              |      `-- build
              |            |-- command failed -----> failed (retryable)
              |            |
              |            `-- command succeeded
              |                  |
              |                  `-- info
              |                        |-- fresh ---> ready
              |                        |-- stale ---> failed (terminal)
              |                        `-----------> failed (retryable)
              |
              `-- any other non-ready state -------> failed (retryable)
```

The Unica index lock remains owned for the complete update, fallback build,
and final info sequence. Therefore concurrent tool calls see a real active
operation and may correctly return `rlm index building`.

There is exactly one fallback build per update job. A non-fresh result after
the fallback is terminal and cannot recurse into another build.

### Marker and diagnostics

While the fallback is running, the marker remains `building`; its action or
message identifies that the worker is rebuilding after
`stale (content)`.

On successful recovery, the ready marker retains last-run diagnostics that
show:

- the maintenance path was `update -> build`;
- the fallback reason was `stale (content)`;
- timing or command metrics remain available under the existing last-run
  diagnostics boundary.

On failure, the marker is `failed` with a persisted `failure_class` of either
`retryable` or `terminal`. Its message contains both:

- the original post-update `stale (content)` reason;
- the rebuild failure or final non-fresh status.

This preserves the causal chain instead of reporting only the last command.

### Terminal failed state

An active lock has priority over marker state: while a worker owns the lock,
readiness is `Building`.

Both synchronous entry points recheck the active lock after the potentially
blocking `index info` call and before interpreting or persisting that result.
This prevents an older info snapshot from overwriting `building` or exposing
the index while a competing request has started maintenance.

Without an active lock, only a matching `failed` marker whose persisted
`failure_class` is `terminal` blocks maintenance for the resolved source root.
Terminal is written only after a successful update and recovery build still
produce a stale final `info` result. Both maintenance startup and readiness
checks honor that marker:

- startup does not overwrite it by launching another automatic update;
- readiness returns `Failed(marker.message)`;
- adapters render the actual unavailable/failed reason rather than
  `rlm index building`.

If `index info` reports a fresh index, freshness wins and the marker is replaced
with `ready`. This allows a manual or external rebuild to recover the workspace
without deleting marker files.

Cancellation, timeout, process-launch, tool-execution, disk-full, and malformed
or unavailable `info` failures use `failure_class: retryable`. Legacy failed
markers without a class are also treated as retryable. Their diagnostic remains
available on disk, but the next request may start maintenance again. This avoids
requiring a cache reset after a transient failure.

The terminal marker comparison uses normalized source-root identity so a marker
from another source set cannot block the current one.

### Workspace service and adapters

The workspace-service serialization already supports `failed` readiness with
an error message. The implementation must preserve that message across
`ServiceResponse::from_readiness` and `index_readiness`.

Adapter warning mapping follows these rules:

- active lock / `Building` -> `rlm index building`;
- terminal `Failed(message)` -> an RLM index unavailable warning containing
  `message`;
- `Stale` without an active maintenance job must not be presented as active
  building merely because the index status is stale.

No MCP request or response schema changes are required.

## Error handling

- Update command failure: record the existing update failure, with no fallback.
- Post-update info execution failure: record the info failure, with no fallback.
- Post-update `stale (content)`: run exactly one fallback build.
- Fallback build failure, cancellation, or timeout: record a retryable failure
  containing the stale-content recovery context.
- Final info cancellation, execution failure, or unavailable status: record a
  retryable failure containing both the recovery context and final status.
- Final stale status after a successful recovery build: record a terminal
  failure because the one-shot recovery policy was exhausted.
- Cancellation continues to use the stable `cancelled:` prefix.
- Update, post-update info, recovery build, and final info all run through the
  same lock-loss cancellation wrapper. In-memory ownership is checked on each
  child poll; disk-backed ownership is checked on heartbeat refresh. Losing
  either cancels the active child, prevents later recovery commands, and stops
  the displaced worker from overwriting replacement state.

## Tests

### Index worker unit tests

- Parse `stale (content)` distinctly from other stale statuses.
- A stale index initially schedules `update`, not `build`.
- Successful update followed by fresh info writes `ready` and does not build.
- Successful update followed by `stale (content)` runs one build under the same
  job, then writes `ready` after fresh info.
- The successful ready marker retains the recovery reason and recovery action.
- Rebuild failure writes `failed` with the original stale-content reason.
- Retryable failures do not block the next automatic maintenance attempt.
- Cancellation after the primary update preserves the stable `cancelled:`
  prefix and remains retryable.
- A non-fresh final info result writes `failed` and does not recurse.
- Cancellation during fallback preserves the cancellation prefix and releases
  the lock.

### Marker/readiness regression tests

- A real active lock still returns `rlm index building`.
- A lock acquired while `index info` is running wins before either entry point
  writes or returns the older info result.
- A terminal failed marker without an active lock returns `Failed(message)`.
- A terminal failed marker is not overwritten by another automatic update.
- A fresh info result replaces a previous terminal failed marker with `ready`.
- A terminal failed marker for another normalized source root does not block
  the current source root.

### Workspace-service and adapter tests

- `Failed(message)` survives workspace-service serialization.
- RLM-backed tools show the failed reason rather than `rlm index building`.
- Stale readiness without an active lock is not described as active building.

The worker tests use scripted `IndexRunner` outputs to model the state machine.
The build-tools contract gate additionally runs the exact packaged
`rlm-bsl-index` against a clean temporary Git fixture and proves:

1. initial build is fresh;
2. changing only tracked-file mtimes produces `stale (content)`;
3. Git HEAD remains unchanged across update;
4. update reports `Changed: 0` and `Fast path: True`;
5. the following info remains `stale (content)`;
6. a full build restores `fresh`.

The fixture uses explicit sampling thresholds for a young, small index; it does
not disable freshness sampling.

## Follow-up

A separate upstream RLM issue may introduce a supported full-scan incremental
update or safe metadata refresh. If bundled RLM later exposes such an operation,
Unica can insert it before the full-build fallback without changing the marker,
lock, or external-status rules defined here.
