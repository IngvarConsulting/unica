# ADR-0009: OS-specific code lives behind infrastructure platform facades

- Status: `accepted`
- Date: `2026-07-20`

## Context

Rust platform branches are spread across domain models, application code,
infrastructure adapters, binary entrypoints, and `unica-bootstrap`. This makes
the source path an unreliable signal for deciding whether a change needs the
full macOS/Linux/Windows test matrix.

The current application layer also imports concrete infrastructure adapters,
while domain modules perform filesystem discovery and canonicalization. Moving
only `cfg(windows)` blocks would preserve those inverted dependencies and would
not establish the intended DDD boundary from ADR-0002.

## Decision

Unica uses dependency inversion and two explicit platform infrastructure
facades.

1. `domain` owns models and pure rules; `application` owns use cases and
   platform-neutral ports; `infrastructure` implements those ports. Production
   wiring happens outside the application layer.
2. OS-specific production code lives only under
   `crates/unica-coder/src/infrastructure/platform/**` and
   `crates/unica-bootstrap/src/platform/**`.
3. Platform modules expose only platform-neutral types. Filesystem/path and
   process/entrypoint behavior enters the rest of the code through these
   facades.
4. A tracked-source architecture guard enforces both allowed platform roots and
   dependency direction. Its implementation and allowlist are part of the
   platform contract and may not contain path-by-path legacy exemptions.
5. Platform-specific tests live beside their adapters or under
   `crates/<crate>/tests/platform/**`.

## Consequences

- Source paths become a stable input for cross-platform CI classification.
- Domain filesystem discovery moves to infrastructure while domain models and
  pure selection rules remain in domain.
- Public MCP contracts, package behavior, path safety, and process-lifecycle
  guarantees do not change.

GitHub Actions routing and CI optimization are outside this decision.
