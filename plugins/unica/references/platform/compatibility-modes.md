# Platform Compatibility Modes

Use this reference when platform behavior depends on a configuration or
extension compatibility mode.

## Terms

- **runtime platform line** — the platform line that actually runs or will run
  the infobase;
- **configured compatibility mode** — the literal stored in configuration or
  extension metadata;
- **effective compatibility version** — the behavior version used for a
  feature-specific compatibility decision.

Do not substitute an XML dump format or a serializer profile for the runtime
platform line unless the question is specifically about that serializer's
mutation contract.

## Normalization

`DontUse` -> runtime platform line

`VersionX` -> `X`

`DontUse` does not mean that compatibility semantics are absent. It selects
current-platform behavior. An explicit `VersionX` selects the documented
compatibility-controlled behavior of `X` while the code still runs on the
actual runtime platform.

## Mode Families

| Metadata property | Scope | Interpretation |
|---|---|---|
| `CompatibilityMode` | Configuration behavior | Normalize `DontUse` or explicit `VersionX` |
| `ConfigurationExtensionCompatibilityMode` | Extension capabilities and behavior | Normalize its own literal; when initializing it from the base configuration, report the copied value explicitly |
| `InterfaceCompatibilityMode` | Client interface behavior | Interpret the exact enum value from the target-platform documentation; do not apply the generic `DontUse`/`VersionX` formula |

The documented feature contract selects the applicable property; code location does not select the mode family.
For example, code located in an extension does not by itself make
`ConfigurationExtensionCompatibilityMode` decisive for general configuration
behavior.

Compatibility mode can affect platform APIs, metadata rules, persisted
behavior, and other version-gated mechanics. It is not limited to making old
methods behave as they did in an older release.

## Evidence And Decision Workflow

1. Resolve the exact runtime platform version for every source and target
   environment.
2. Read the configured mode literal from the owning configuration or extension
   metadata. Keep the three mode families separate.
3. Verify through exact-target-platform guidance that the literal is supported.
   A platform version such as 8.5.4 does not prove that a literal named
   `Version8_5_4` exists.
4. Normalize `CompatibilityMode` or
   `ConfigurationExtensionCompatibilityMode` to the effective compatibility
   version.
5. Find the documented boundary for the specific feature or behavior. Compare
   the effective version with that boundary; do not infer a global answer from
   one feature.
6. When environments or candidate modes differ, present the result as a
   matrix.

Use `unica.standards.search` and `unica.standards.explain` for the platform
contract. Inspect project metadata and code through the relevant public
`unica.*` tools.

BSP code is corroborating implementation evidence: it can show how a carefully
maintained library resolves platform and compatibility versions in practice.
It is not the platform specification, so explain any inference and reconcile
it with the exact platform contract.

## Equivalence Boundary

For a feature controlled by compatibility version, these rows can produce the
same feature result:

| runtime platform line | configured compatibility mode | effective compatibility version |
|---|---|---|
| 8.3.24 | `DontUse` | 8.3.24 |
| 8.3.26 | `Version8_3_24` | 8.3.24 |

This is feature-scoped compatibility-controlled equivalence, not complete old-platform equivalence.
The newer runtime can still contain unrelated fixes, performance changes,
supported enum differences, and behavior outside the compatibility contract.

For a future-platform question, keep the same separation:

| runtime platform line | configured compatibility mode | effective compatibility version |
|---|---|---|
| 8.5.4 | `DontUse` | 8.5.4 |
| 8.5.4 | `Version8_3_24` | 8.3.24 |

The second row is valid only if the exact 8.5.4 platform contract confirms that
`Version8_3_24` remains a supported literal.

## Required Answer Shape

Report these columns for every relevant case:

| runtime platform | literal mode | effective version | feature result | evidence |
|---|---|---|---|---|

Name the feature-specific boundary and distinguish confirmed platform facts
from inferences based on BSP or project code.

## Common Reasoning Errors

- Treating `DontUse` as an unconditional newest-mode flag instead of resolving
  the runtime platform line.
- Guessing a same-numbered explicit mode from the runtime version.
- Saying that the infobase runs as the older platform. Only documented
  compatibility-controlled behavior is selected.
- Reducing compatibility mode to old method behavior.
- Claiming complete runtime equivalence from equal effective versions.
- Mixing configuration, extension, and interface compatibility properties.

## Stop Rules

- Do not give a hard version-sensitive answer without the runtime platform,
  exact mode literal, affected metadata owner, and feature boundary.
- Do not compare mode strings lexically.
- Do not invent future enum literals or support ranges.
- If public MCP `unica` cannot confirm the required platform source or project
  state, report a Unica MCP contract gap.
