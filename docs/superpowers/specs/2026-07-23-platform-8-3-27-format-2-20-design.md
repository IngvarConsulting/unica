# Platform 8.3.27 / export format 2.20 design

## Status

The fixed-profile design and implementation scope were approved in conversation
on 2026-07-23. The current implementation supports one writable profile:
platform 8.3.27 and configuration export format `2.20`.

Native migration and multiple format profiles are not implemented by this
design. They are explicitly deferred future work and require a separate public
contract and approval.

## Authority

The 1C:Enterprise 8.3.27 Developer Guide maps platform line 8.3.27 to export
format `2.20`. It also states that:

- all configuration objects exported to files use the same export format;
- the version is present on object root XML files and on some subordinate root
  files;
- absence of a version on a version-owning root means format `1.0`;
- platform 8.3.27 can import export formats less than or equal to `2.20`.

The platform's ability to import an older format does not make that format a
writable Unica profile. Unica writes only `2.20` in the current implementation.

Runtime XDTO/XSD from exact build 8.3.27.2074 is useful evidence, but is not a
universal contract for every configuration XML family. Some schemas exist only
in the EDT corpus, some runtime schemas are type libraries without global
roots, and some raw schemas contradict files accepted and emitted by the real
platform. A platform 8.3.27 load and roundtrip remains the final compatibility
proof for generated XML. Where a permissive XSD and reproduced exact-platform
behavior differ, the fixed 8.3.27 profile follows the platform behavior.

## Goals

1. Keep `8.3.27 / 2.20` as the only writable platform-XML profile.
2. Detect unsupported owner formats before a modifying operation writes.
3. Let read-only inspection and validation continue where safe, with a distinct
   compatibility warning.
4. For older sources, refuse mutation and recommend an explicit user-driven
   load and re-export using 1C:Enterprise 8.3.27 tooling.
5. For newer sources, refuse mutation, never downgrade, and report that
   platform 8.5 is not supported yet but is planned.
6. Correct document-family deviations only when backed by code, schemas, EDT
   contracts, or real-platform evidence.

## Non-goals

- Supporting more than one platform or export-format profile now.
- Automatically migrating source as a side effect of any operation.
- Providing a native migration endpoint in the current public MCP contract.
- Rewriting only a root `version` attribute as a migration technique.
- Downgrading a format newer than `2.20`.
- Replacing semantic validators with raw XSD validation.
- Treating the runtime 8.3.27.2074 schema archive as universal.
- Implementing the complete XDTO/XSD service proposed by issue #126.

## Supported profile

| Property | Current value |
| --- | --- |
| Platform line | `8.3.27` |
| Configuration export format | `2.20` |
| Exact runtime build used by current platform proofs | `8.3.27.2074` |
| Complete runtime-install tree identity | `5eb8897c4f7e95876572f2f36943439b0d57e47688314b622f5771e5a22df0ef` |

`ACTIVE_FORMAT_PROFILE` owns the platform line and export format. Writers and
validators do not maintain local version allowlists.

The supported XML attribute is the exact raw lexical slice `2.20`, compared
before XML entity decoding. Numerically equivalent spellings such as `2.20.0`,
`02.20`, and `2.020`, as well as entity spellings such as `2.&#50;0`,
`&#x32;.20`, and `2.2&#48;`, are invalid rather than supported. Numeric
component comparison is used only to classify values that are genuinely below
or above that exact profile.

## Compatibility classification

The effective version comes from the version-owning XML file for the selected
source set or standalone artifact. A legitimate subordinate XML document may
be versionless and inherit the source-set owner's format.

| Source format | Classification | Modifying tools | Read-only tools | User remediation |
| --- | --- | --- | --- | --- |
| missing on a version-owning root, or `< 2.20` | `older` | stop before write | continue where parsing is safe, with warning | explicitly load and re-export with 1C 8.3.27 tooling |
| exact literal `2.20` | `supported` | continue | continue | none |
| `> 2.20` | `newer` | stop before write | continue where parsing is safe, with warning | no downgrade; wait for a supported newer profile |
| malformed, numerically equal but not exact, unreadable, or ambiguous | `invalid` | stop before write | continue only as far as safe parsing permits | correct the source/owner selection |

## Architecture

### Central profile and owner resolution

The domain classifier compares numeric version components, requires the exact
raw literal for the supported equality case, and treats a missing owner version
as `1.0`. One source-set-aware owner resolver is shared by the application
guard and native writers:

- CF and CFE use their source-set `Configuration.xml`;
- EPF and ERF use the top-level sibling artifact descriptor;
- recognized standalone version-owning roots own their version;
- versionless DCS, MXL, and ClientApplicationInterface documents inherit a
  resolved same-case source-set owner where applicable;
- owner lookup is normalized and stops at the configured source-set/workspace
  boundary rather than scanning to filesystem root.

Missing, malformed, unreadable, wrong-QName, or ambiguous owners are structured
`formatVersionInvalid` failures.

Existing operation-declared Form, DCS, and MXL targets are checked by exact
QName before any write, including `.XML` and suffixless paths. Correct
versionless DCS/MXL roots and genuinely absent outputs remain allowed.

### Preflight guard

Every public modifying platform-XML workflow declares an effective path policy.
The guard resolves handler aliases, default target paths, and multi-input paths
before the native handler or support guard runs. It blocks an incompatible
owner before directory creation, temporary output, platform mutation, or file
write. New CF, EPF, and ERF scaffolds use the active profile; CFE initialization
also guards an optional base configuration.

Read-only `info`, `validate`, `diff`, and decompile workflows receive the same
classification as a warning and continue where their own parser can proceed.
The incompatibility is returned separately from XML syntax or semantic errors.

### Transaction and concurrency boundary

The application preflight is an early user-facing decision, not the only
authorization check. Mutating handlers re-resolve their effective format owners
and bind every XML/source preimage used to derive output to the publication
transaction. Cooperating Unica writers share advisory locks, and a changed
preimage aborts before the planned mutation is published.

Atomicity here covers reported validation, I/O, post-validation, and detected
concurrent-change failures. It does not promise crash-atomic multi-file storage:
advisory locks cannot stop non-Unica writers, path/no-follow checks do not hold
permanent open handles against every external rename, and process, OS, or power
failure may interrupt multiple filesystem renames. Rollback is attempted on a
reported publication failure; cleanup failures are surfaced as warnings.

### Verified synchronous full-dump boundary

Applied synchronous `mode=full` dump is supported only for DESIGNER
`CONFIGURATION` and `EXTENSION` source-sets through `unica.build.dump` and
`unica.runtime.execute`. Unica selects an exact 8.3.27.x installation, redirects
the selected source-set into an owner-only private sibling stage, validates the
required owner and exact raw `2.20` version-bearing XML, rechecks config, target,
and stage preimages under the shared exclusive lock, then publishes the whole
tree with rollback.

The final directory rename is not claimed to be a source-identity CAS: portable
Linux and macOS rename APIs bind a parent descriptor and name, while their
no-replace flags protect only destination absence. The moved tree is therefore
tentative until Unica recaptures it under the still-held lock and proves an
exact match with the sealed snapshot. On a mismatch or validation error, Unica
atomically moves that target into owner-only recovery before returning, then
restores the original with no-clobber semantics. If restoration cannot prove an
unoccupied target, Unica fails closed and retains recovery instead of
overwriting the current entry. The rollback rename is tentative too: its
post-rename snapshot must equal the captured backup, or the replacement is
returned to private quarantine and is not accepted as the source tree. Within
this trust boundary, the method does not return or release its lock with an
unvalidated tree installed by the invocation at the selected source path.

This postcondition does not defend against a continuously adversarial process
running as the same OS user and replacing pathnames during quarantine or
rollback. Portable filesystem primitives cannot establish that stronger
boundary; it would require isolation such as a separate privileged identity or
an immutable parent directory.

Async full and external source-set dump remain fail-closed. Applied
incremental/partial dump, `convert`, and Designer `rawKeys` containing
`DumpConfigToFiles` or `LoadConfigFromFiles` are also blocked; dry-run previews
remain available.

### Manual migration boundary

Unica currently has no public native format-migration operation and never
starts migration automatically. For an older source, the warning recommends an
explicit operator action using 1C:Enterprise 8.3.27 tooling: load the source and
re-export it, then retry the Unica mutation against the resulting `2.20` tree.

The machine-readable code `formatMigrationAvailable` means that an explicit
manual remediation exists. It does not identify or imply a callable Unica tool.

A source newer than `2.20` is never downgraded. The current user-facing text is:

> Формат выгрузки `{actual}` новее поддерживаемого `2.20` для платформы 1С 8.3.27. Unica пока не поддерживает работу с этой выгрузкой. Поддержка платформы 1С 8.5 планируется в ближайших версиях.

This text does not infer that the source was created specifically by 8.5; only
the export format is known to be newer than the 8.3.27 contract.

## Diagnostics

Compatibility diagnostics include:

- `actualFormat` when it can be parsed;
- `targetFormat: "2.20"`;
- `targetPlatform: "8.3.27"`;
- `compatibility`: `supported`, `older`, `newer`, or `invalid`;
- the resolved owner path and owner kind;
- one machine-readable code.

Current codes are:

- `formatMigrationAvailable` for an older source and manual platform re-export;
- `platformVersionUnsupported` for a newer source;
- `formatVersionInvalid` for an owner that cannot be classified.

Diagnostics must not expose credentials, connection strings, infobase
identifiers, or unrelated local paths.

## Writer and document-family contract

All newly generated version-bearing XML roots use the active `2.20` profile.
This is an implementation invariant, not a claim that every emitted document
family has completed a real-platform canonical roundtrip.

Current real-platform proofs are narrower and explicit:

- empty MXL uses the platform-produced direct sentinel sequence
  `languageSettings, columns, rowsItem, templateMode, vgRows`;
- ExchangePlan `Content.xml` uses the proven `xcf/extrnprops`
  `ExchangePlanContent` QName with `xr`, `xs`, and `xsi` declarations;
- `unica.cf.init` emits a fresh non-nil Configuration UUID, defaults the 8.3.27
  compatibility properties, includes `TextToSpeech=false`, and is semantically
  stable across two 8.3.27.2074 import/check/export cycles;
- ClientApplicationInterface remains versionless and inherits `2.20` from the
  same-case Configuration owner;
- DCS follows exact 8.3.27 canonical behavior even where XSD permits a broader
  representation: `dataSetLink/required=true` is omitted while `false` is
  retained; `DefinedType.*` in `valueType` is rejected because the platform
  removes it on roundtrip and callers must supply expanded constituent types;
  `StandardPeriod` start/end dates are emitted only for the `Custom` variant;
  and an empty details-group `groupItems` container is omitted.

These focused real-platform probes were the precursor to the complete
63-checkpoint gate recorded below and in the deviation matrix.

Managed Form, DCS, and MXL consumers now enforce their exact proven root QNames.
Form QName text must resolve through in-scope namespace bindings. These
structural fixes do not by themselves prove every writer branch to be
platform-canonical; the remaining corpus and roundtrip work is recorded in the
deviation matrix.

## Validation authority

Validation is layered:

1. XML well-formedness;
2. central owner and export-format compatibility;
3. existing Unica structural and semantic validation;
4. XSD only for a compatibility-tested schema profile and document family;
5. platform 8.3.27 load/roundtrip as final proof for generated artifacts.

XSD failures retain their provenance. Known raw-schema limitations must not be
used to rewrite platform-valid XML.

## Verification strategy

### Guard and writer tests

- classify missing, older, supported, newer, and malformed owner versions;
- prove mutation is blocked before handler/write and bytes remain unchanged;
- prove read-only path aliases retain warnings and continue;
- prove every mutating platform-XML descriptor has an effective owner path;
- prove every generated version-owning root uses `2.20`.

### Document-family tests

- compare proven MXL and ExchangePlan output with normalized platform fixtures;
- reject wrong MXL, Form, and DCS root QNames before output or mutation;
- validate Form QName prefix bindings and atomically repair emitted bindings;
- retain existing semantic regression suites.

### Platform checkpoints

- retain the exact platform build, complete 4337-file/96-directory install-tree
  identity, commands, exit codes, and hashes for evidence;
- distinguish a code/test contract from a completed platform roundtrip;
- use a corpus with a complete public-mutator/case inventory and selected
  representative branches to close the recorded writer-family evidence;
- do not interpret that complete operation inventory as exhaustive coverage of
  every public-argument combination;
- bind exact empty-directory topology in source/base checkpoints, both
  roundtrip comparisons, retained evidence, and corpus immutability checks;
- for EPF/ERF, materialize each descriptor/content pair under a private source
  directory and pass only that directory to `ibcmd config import`, including
  the second cycle.

### Current corpus inventory

Two independent corpus generations use manifest `schemaVersion: 2` and have the
same normalized case-contract SHA-256
`e1f9b8b73288699b5202df1c0814110b255fa80eec908f1b7ea921f55acb82f8`.
The digest includes the complete `emptyDirectoryPaths` inventory. Their
complete public inventory contains 63 platform checkpoints: 52 configuration,
nine extension, one EPF, and one ERF.

The inventory includes the two non-XML-only mutators whose selected cases
intentionally leave the XML map unchanged: `unica.code.patch` and
`unica.support.edit`. Their BSL or binary platform payloads are bound by exact
pre/post logical paths and bytes. `support.edit` bases its decision on one read
of `ParentConfigurations.bin`, changes only the global header capability, locks
vendor/object slots, preserves every existing `.cf`, and binds exact `.cf`
preimages plus case-insensitive directory membership to the transaction. A
concurrent `.CF` addition aborts the binary mutation. The six
`cfe.patch_method` checkpoints cover the CommonModule, ObjectModule,
ManagerModule, RecordSetModule, Form, and ValueManagerModule layouts with
registered adopted extension objects and platform-canonical UTF-8 BOM/CRLF
BSL. Five layouts atomically add the required descriptor `xr:PropertyState`
with `State=Extended`; the form wrapper already contains that state after
`cfe.borrow`, so its XML write is idempotent.

The `cfe.patch_method` proof is deliberately narrower than every platform
interception capability. Exact EDT 8.3.27 declarations produce 51 type/role
paths inside the current v1 grammar; unit contracts bind that matrix, while
the six platform checkpoints represent its six physical file layouts rather
than all 51 paths. V1 emits only `Before`/`After` for a caller-verified existing
procedure without parameters. It does not read the base method or prove its
signature. Functions, parameters, `ModificationAndControl`, `Around`, and
special form-handler semantics are outside this proof and are rejected or
deferred, not described as platform-unsupported.

The shared `meta.compile`/`cfe.borrow` metadata registry profile is total for all
45 known types: 22 have non-empty `GeneratedType` profiles and 23 have
explicitly empty profiles. Unknown types fail closed. ExchangePlan additionally
receives exactly one `xr:ThisNode`.

Each corpus contains:

- 297 pre-call and 352 post-call XML documents, for 649 static XML inputs;
- 75 pre-call and 110 post-call platform non-XML payloads;
- 141 stable auxiliary payloads outside the platform checkpoint roots;
- 1039 regular files in total, including the 63 case reports and corpus
  manifest;
- 90 empty directories whose exact paths are part of the corpus contract.

The current static XSD/XDTO result for all 649 XML inputs is 11 strict passes,
638 inconclusive results, and zero failures. The inconclusive result is split
into 122 documents affected by a known schema-source incompatibility and 516
documents not covered by a compatibility-tested complete dump-document schema.
Of those 516, 515 have EDT declaration evidence; the single
`GraphicalSchema`/`Flowchart.xml` input has structural-profile evidence only.
The result is not promoted to a pass.

The generator excludes only the volatile internal runtime cache
`.build/unica`: it removes that cache before the pre-call capture and after the
public call. Other regular files under `.build`, and every regular workspace
payload outside the selected platform roots, remain in scope as auxiliary
evidence and must be byte-identical.

Final exact-platform result: `PASS`. The gate recorded 63 passed, with zero
rejections, normalized semantic deltas, source errors, or unstable roundtrips.
The run executed 432 platform commands and reverified the unchanged corpus and
pinned platform installation. Static validation and a pinned corpus did not
substitute for that run; the detailed evidence remains in the deviation matrix.

There are no native migration orchestration, migration receipt, or migration
provenance tests in the current scope because that feature does not exist. The
verified full-dump stage is a guarded publication path for a requested dump,
not a format migration.

## Acceptance criteria

- One central profile defines `8.3.27 / 2.20`.
- Every modifying platform-XML operation resolves its effective owner before
  the first write, or is an explicit new-dump operation using `2.20`.
- Older formats produce a warning and no mutation, with a manual 1C 8.3.27
  load/re-export recommendation and no invented public tool name.
- Newer formats produce `platformVersionUnsupported`, the agreed 8.5 roadmap
  text, no mutation, and no downgrade recommendation.
- Read-only operations report incompatibility separately and continue where
  safe.
- Writers use `2.20`; platform-canonical claims are made only for families with
  recorded real-platform evidence.
- MXL, ExchangePlanContent, Form, DCS, and `cf.init` corrections are locked by
  their focused regressions and platform evidence.
- No automatic or native migration is exposed.

## Deferred future work

Multiple profiles, including a future 1C 8.5 profile, require new per-family
evidence and must not weaken the `8.3.27 / 2.20` behavior. A native migration
workflow, if later approved, needs its own public API design, platform-selection
policy, transaction/recovery model, security review, and tests. It is not an
unfinished part of this implementation.
