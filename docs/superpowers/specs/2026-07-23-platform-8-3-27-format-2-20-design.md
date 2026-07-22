# Platform 8.3.27 / export format 2.20 design

## Status

Design sections were approved in conversation on 2026-07-23. This written specification awaits user review. Implementation remains gated on that review.

## Context

Issue [#126](https://github.com/IngvarConsulting/unica/issues/126) proposes an optional, provenance-aware XDTO/XSD validation layer. Before designing that service further, the existing Unica XML tools need one coherent platform and export-format contract.

The 1C:Enterprise 8.3.27 Developer Guide maps platform line 8.3.27 to configuration export format `2.20`. It also states that:

- all configuration objects exported to files use the same export format version;
- the version is written to root XML files of configuration objects and to some subordinate root XML files;
- absence of an explicit version means format `1.0`;
- 8.3.27 can import export formats less than or equal to its own format.

The XDTO schemas exported from runtime 8.3.27.2074 are useful evidence, but they are not a universal schema for every configuration XML family. Some required schemas exist only in the EDT corpus, some runtime schemas are type libraries without global roots, and several real Unica fixtures do not validate strictly without compatibility work. Platform load and roundtrip therefore remain the authoritative compatibility check.

## Goals

1. Make platform 8.3.27 and export format `2.20` the only writable profile currently supported by Unica.
2. Detect unsupported source formats before any modifying operation writes files.
3. Preserve read-only inspection and validation for unsupported formats while reporting the incompatibility.
4. Offer an explicit, user-authorized migration path for older exports.
5. Record and correct deviations between current writers, validators, documentation, the 8.3.27 schemas, and real platform output.
6. Keep the internal boundary ready for multiple format profiles later without exposing unsupported profiles now.

## Non-goals

- Supporting multiple platform or export-format profiles in this iteration.
- Automatically migrating a source tree as a side effect of `add`, `edit`, or `compile`.
- Downgrading formats newer than `2.20`.
- Replacing existing semantic validators with XSD validation.
- Treating the runtime 8.3.27.2074 schema set as a universal contract for all patches, configurations, and document families.
- Implementing the full XDTO/XSD service proposed by issue #126.

## Supported profile

Unica has one active `FormatProfile`:

| Property | Value |
| --- | --- |
| Platform line | `8.3.27` |
| Configuration export format | `2.20` |
| Runtime schema provenance used by the current research | `8.3.27.2074` |

The exact patch build is recorded in runtime and schema provenance. It is not used to contradict the official line-wide mapping `8.3.27 -> 2.20`.

## Compatibility classification

The format is determined from the root file of the configuration or extension dump. Legitimate subordinate XML files are not required to carry a `version` attribute.

| Source format | Classification | Modifying tools | Read-only tools | Migration offer |
| --- | --- | --- | --- | --- |
| missing on a root where absence denotes `1.0`, or `< 2.20` | `older` | warn and stop before the first write | continue with warning | yes |
| `2.20` | `supported` | continue | continue | no |
| `> 2.20` | `newer` | warn and stop before the first write | continue with warning | no downgrade |
| malformed or not classifiable | `invalid` | error and stop | continue only as far as safe parsing permits | no |

The central guard, not individual tools, owns this classification.

## Architecture

### Format profile

A single application-level format-profile component owns the supported platform line, export format, comparison rules, diagnostic codes, and user-facing messages. Writers and validators must not maintain independent allowlists such as `2.17 | 2.20 | 2.21`.

The component is intentionally shaped so a future implementation can resolve a profile by platform and document family, but this iteration exposes only the fixed `8.3.27 / 2.20` profile.

### Preflight guard

Every public modifying tool calls the same preflight guard before constructing a write plan. The guard:

1. resolves the configuration or extension root;
2. reads and classifies its export format;
3. returns structured compatibility information;
4. either authorizes planning or returns a no-write diagnostic.

The guard runs before temporary output, support locks, platform calls that mutate an infobase, or filesystem writes. Batch operations perform preflight for their full input set before modifying any item.

### Read-only behavior

`info` and `validate` operations retain their current structural and semantic analysis when it is safe. Their result includes the format mismatch as a distinct compatibility diagnostic; a format mismatch is not misreported as an XML syntax error.

### Explicit migration

Migration is exposed as two separately invoked public operations: `unica.cf.migrate_format` for configuration dumps and `unica.cfe.migrate_format` for extension dumps. A warning from another tool may recommend the matching operation, but must not invoke it automatically. Migration starts only after an explicit user tool call.

For a source format older than `2.20`, migration uses the 8.3.27 platform as the transformer:

1. complete a read-only preflight;
2. create a disposable infobase and isolated staging directory;
3. load the source dump with platform 8.3.27;
4. dump the configuration or extension from the same platform into staging, producing `2.20`;
5. run format, structural, semantic, and applicable XSD checks;
6. load the staged result once more with platform 8.3.27;
7. present or record the migration result and replace the source tree only within the explicitly requested migration operation.

Plain replacement of the XML `version` attribute is forbidden because format revisions can change document structure and values.

The migration must not attempt a downgrade from a format newer than `2.20`.

### Transaction boundary

No failed compatibility check or migration leaves a partially rewritten dump. Migration builds and verifies a complete staged tree first. Replacement uses a recoverable backup/rename protocol with rollback on failure. The implementation plan must specify the exact cross-platform filesystem protocol and its recovery tests.

## Diagnostics

Compatibility results contain at least:

- `actualFormat`;
- `targetFormat: "2.20"`;
- `targetPlatform: "8.3.27"`;
- `compatibility`: `supported`, `older`, `newer`, or `invalid`;
- the dump root path;
- one machine-readable diagnostic code.

Required codes:

- `formatVersionMismatch` for the common incompatibility envelope;
- `formatMigrationAvailable` for an older source;
- `platformVersionUnsupported` for a newer source;
- `formatVersionInvalid` when the root cannot be classified.

For a format newer than `2.20`, the user-facing text is:

> Формат выгрузки `{actual}` новее поддерживаемого `2.20` для платформы 1С 8.3.27. Unica пока не поддерживает работу с этой выгрузкой. Поддержка платформы 1С 8.5 планируется в ближайших версиях.

This message does not claim that the source was produced specifically by 8.5; the detected fact is only that its export format is newer than the 8.3.27 contract.

Diagnostics and migration provenance must not expose credentials, connection strings, infobase identifiers, or unrelated local paths.

## Deviation inventory

Before changing writers, implementation records a reviewed matrix with these columns:

- public tool and native operation;
- document family and root;
- current emitted or accepted namespace/version;
- 8.3.27 contract;
- evidence source: official guide, exact-build XSD, EDT schema, real platform export, or platform roundtrip;
- required correction;
- regression test.

The initial confirmed candidates are:

1. the shared format detector falls back to `2.17`;
2. the HomePage writer hard-codes `2.17`;
3. CF/CFE validators accept `2.21`, while meta/form validators have different allowlists;
4. `template.add` emits a spreadsheet namespace different from real MXL output;
5. `ExchangePlanContent` differs between current code and specification.

These are candidates, not permission to fix by textual substitution. Each correction requires document-family evidence. If code, tests, prose, and schema disagree, code and platform roundtrip evidence take precedence according to repository policy; the contradiction must be recorded rather than hidden.

## Validation authority

Validation is layered:

1. XML well-formedness;
2. central export-format compatibility;
3. existing Unica structural and semantic validation;
4. XSD validation only for a compatibility-tested schema profile and document family;
5. platform 8.3.27 load/roundtrip as the final compatibility proof for generated or migrated artifacts.

XSD failure remains separately attributed. The known runtime schema limitations from issue #126 prevent raw XSD results from silently replacing semantic validation or platform verification.

## Test strategy

### Guard tests

Use roots representing:

- missing version interpreted as `1.0`;
- `2.17`;
- `2.19`;
- `2.20`;
- `2.21`;
- malformed version;
- a legitimate subordinate XML without a version attribute.

For every modifying tool family, prove that unsupported input returns the shared diagnostic and leaves all files byte-identical. Prove that `2.20` reaches the existing operation.

### Writer tests

Every generated versioned root must use `2.20`. Namespace and structure fixtures must be taken from verified 8.3.27 output or a compatibility-tested schema. Tests that currently encode an older or newer default are migrated deliberately and reviewed by family.

### Migration tests

Cover:

- explicit invocation requirement;
- migration from no explicit version and representative older versions;
- refusal of `2.21` and newer;
- platform absence, wrong platform line, license failure, timeout, load failure, dump failure, and validation failure;
- no source mutation before staged verification;
- rollback after replacement failure;
- successful staged and final loads on 8.3.27;
- provenance containing the exact platform build without secrets.

### Corpus and regression tests

Run current real fixtures through the deviation matrix. XSD-incompatible families stay advisory until their schema profile is proven on the corpus. The repository test suite and package-contract tests remain required.

## Documentation changes

Public skill and reference prose must state:

- Unica currently writes only platform 8.3.27 export format `2.20`;
- older sources are read-only until the user explicitly requests migration;
- newer sources are unsupported and are not downgraded;
- read-only diagnostics remain available where safe;
- support for multiple profiles is future work.

Prompt-visible skills continue to use only public `unica.*` MCP tools.

## Acceptance criteria

- One central profile defines `8.3.27 / 2.20`; no modifying tool has an independent version allowlist.
- Every modifying operation preflights before its first write.
- Older formats produce a structured warning, no mutation, and an explicit migration recommendation.
- Newer formats produce `platformVersionUnsupported`, the agreed 8.5 roadmap text, no mutation, and no downgrade recommendation.
- Read-only tools report incompatibility separately and continue where safe.
- Migration is never automatic and can start only through an explicit public operation.
- Configuration and extension migration use `unica.cf.migrate_format` and `unica.cfe.migrate_format`, respectively.
- Migration uses platform 8.3.27 and verified staging rather than attribute replacement.
- Writers produce `2.20` and corrected document-family namespaces/structures backed by evidence.
- The deviation matrix covers every current XML writer and validator in scope.
- Platform roundtrip verifies generated and migrated configuration artifacts.
- No credentials or connection details appear in diagnostics or provenance.

## Future extension

Adding 1C:Enterprise 8.5 or another platform line will add a new `FormatProfile` and document-family compatibility evidence. It must not weaken the `8.3.27 / 2.20` behavior or infer platform versions solely from XML format numbers.
