# ADR-0008: Public marketplace with a thin verified runtime

- Status: accepted
- Date: 2026-07-19

## Context

A full three-platform plugin is too large for a stable Git marketplace, and
consumer machines cannot be assumed to have Node.js, Python, download clients,
or archive tools. Codex package metadata currently exposes one command shape,
not an operating-system command matrix. Git remains a supported installation
prerequisite and Git for Windows provides the same shell entry mechanism as
POSIX Git.

Publishing plugin files and immediately moving the stable catalog would expose
an unverified or not-yet-tagged package.

## Decision

The public marketplace stores a thin plugin with three small native bootstrap
binaries. `.mcp.json` invokes a command-scoped Git shell alias and the tracked
portable selector. Bootstrap downloads the exact host runtime from a pinned
source release, verifies archive and file SHA-256 values, and publishes it
atomically in the Codex home cache.

Marketplace publication is two-phase. A staging PR changes plugin files only.
After merge and creation of an immutable signed tag, a promotion PR changes only
the stable `git-subdir` catalog entry. Existing tags and release bytes are never
moved; changed bytes require a new version.

Legacy migration uses one native transaction engine on every platform. Shell
and PowerShell files are acquisition shims, not separate mutation
implementations.

## Consequences

- Git and Codex CLI are consumer prerequisites; Node.js is not.
- First MCP startup may download one target runtime, while later startups reuse
  the verified ready cache.
- Runtime stdout remains dedicated to MCP JSON-RPC.
- Source, runtime, marketplace tag, and catalog promotion have separate proof
  points and cannot be published as one unsafe mutable step.
