# Issue 126: platform 8.3.27 / export 2.20 XML contract

## Decision and scope

Unica's only writable platform-export profile is:

- platform: `8.3.27`;
- export format: `2.20`.

This document records the compatibility boundary and the audit of every public
native mutator in that boundary, including operations whose platform payload is
non-XML. The corpus has a complete public-mutator/case inventory and covers the
mandatory representative branches listed below. It does not claim exhaustive
coverage of every possible combination of public arguments.

This is the prerequisite writer/profile phase of issue #126. It does not yet
add the proposed optional capability-driven XDTO/XSD service or multi-profile
dispatch. That design is the next phase.

## Contract authority

The sources answer different questions and are used in this order:

| Source | What it establishes |
| --- | --- |
| Official 8.3.27 export-version documentation | Platform `8.3.27` maps to export format `2.20`; a missing version on a version-owning root means `1.0`; the platform may import older formats. |
| Runtime XSD export from 8.3.27.2074 | XDTO element/type structure where the exported schema is complete enough to compile and exposes a document root. |
| EDT 8.3.27 XDTO declarations | Declaration evidence for metadata, managed-form, and external-property families that the runtime XSD export does not expose as complete dump-document schemas. |
| Exact `ibcmd 8.3.27.2074` roundtrip | For each selected corpus checkpoint, decisive evidence that that exact emitted source tree or external artifact is accepted and semantically stable across two cycles. It does not prove unselected argument combinations or intent that a serializer omitted before import. |
| Unica code and tests | The repository contract that applies the profile, rejects unsafe writes, and keeps the evidence inventory reproducible. |

Official documentation is local at
`docs-local/1ci/8.3.27/en/developer/Chapter_2._Managing_configurations/2.17._Dumping_configurations_to_files_Restoring_configurations_from_files/2.17.2._Export_format_versions/index.md`.

Pinned evidence identities:

| Evidence | SHA-256 |
| --- | --- |
| Runtime XSD archive, 56 packages, `platformVersion=8.3.27.2074` | `e7539a02520cf7bd73585d80b038c2c95078aac281d3700842a5f3a1f3c0c204` |
| EDT bundle `com._1c.g5.v8.dt.platform_v8.3.27`, version `1.0.300.v202603181342` | `a0c13bbff0527503c23cde14fb10f07742223c6e7d85bf9f06a753cfcc3707b8` |
| `/opt/1cv8/8.3.27.2074/ibcmd` | `e00f3c945fb6f60bb2802151df1b4e7ee4f3caaf7c9e24a981020af575fda6e5` |
| Complete `/opt/1cv8/8.3.27.2074` install tree: 4337 files and 96 directories, including path, entry type, mode, size, and file content | `5eb8897c4f7e95876572f2f36943439b0d57e47688314b622f5771e5a22df0ef` |

The fact that 8.3.27 can import a format below `2.20` does not make that format
writable by Unica. Import compatibility and the Unica writer contract are
separate decisions.

The fixed-profile contract follows exact 8.3.27 behavior when it is narrower
than a permissive XSD declaration. XSD establishes allowed structure where it
is usable; it does not override a normalization or rejection reproduced by the
exact platform.

## Compatibility boundary

The format is resolved from the version-owning XML root for the selected source
set. Versionless subordinate XML inherits that owner. CF, CFE, EPF, ERF,
recognized standalone documents, path aliases, defaults, and multi-input
operations use the same bounded owner resolver.

| Resolved input | Read-only operation | Mutation |
| --- | --- | --- |
| `< 2.20`, including an existing version-owning XML root whose `version` attribute is absent and therefore interpreted as `1.0` | Warn and continue only where the existing parser can safely do so. | Refuse before the first write and propose an explicit user-driven re-export with 1C:Enterprise 8.3.27. |
| exact raw literal `2.20` | Work normally. | Work normally. |
| `> 2.20` | Warn that the newer profile is unsupported. | Refuse before the first write; state that platform 8.5 support is not available yet but is planned; never offer a downgrade. |
| Malformed, numerically equal but not exact (`2.20.0`, `02.20`, or `2.020`), entity-spelled (`2.&#50;0`, `&#x32;.20`, or `2.2&#48;`), unreadable, or ambiguous version evidence | Report invalid/ambiguous source evidence. | Refuse before the first write. |
| Required version-owning XML owner is missing or cannot be resolved | Report missing/unresolvable owner evidence; do not reinterpret it as version `1.0`. | Refuse before the first write. |
| Existing recognized versionless DCS/MXL root | Follow the fixed operation profile without inventing a version. | Follow the fixed operation profile and preserve the versionless root contract. |
| Genuinely new output with no containing source set | Not applicable. | Follow the fixed operation profile: write `2.20` only when that document root owns an export version; do not invent `version` on versionless DCS/MXL roots. |

Unica never migrates or downgrades as a side effect and exposes no native
format-migration operation. For an older source, the user must load and
re-export it explicitly with the target platform, then retry the Unica
operation.

Only the exact raw lexical slice of the `version` attribute, before XML entity
decoding, may equal `2.20`. Numeric component comparison classifies genuinely
older or newer values; it does not canonicalize an alternative spelling of the
supported value. Existing operation-declared Form, DCS, and MXL targets are
checked by exact QName before writing, including `.XML` and suffixless target
paths. Correct versionless DCS/MXL documents and genuinely absent outputs remain
valid because those roots do not own an export version.

The roadmap wording for `>2.20` is product copy, not provenance detection: an
export version alone does not prove that the source was produced by platform
8.5.

### Transaction boundary

For mutating calls, the public preflight is repeated inside the handler against
the effective XML dependencies. Bytes used to derive a mutation are bound to
the compile transaction as exact preimages, and cooperating Unica writers use
the same advisory publication locks. The atomicity regressions prove that
reported validation, I/O, and detected concurrent-change failures do not leave
the planned source-tree mutation partially applied.

This is not a claim of crash-atomic multi-file storage. Advisory locks do not
stop Designer, scripts, or other non-Unica writers; path normalization and
no-follow checks do not retain permanent open-handle identity across every
external rename; and a process, operating-system, or power failure can interrupt
several filesystem renames. Rollback is attempted for reported publication
failures. Failure to restore or remove a published source path is a hard
transaction error: the result includes `rollback encountered:` integrity
diagnostics, identifies the affected or preserved recovery paths, and requires
the caller to treat source-tree integrity as unverified. Only failure to remove
temporary staging, quarantine, or already-restored recovery residue is
warning-level cleanup.

## Resolved deviations and known boundaries

| Area | Contract enforced for 8.3.27 / 2.20 |
| --- | --- |
| Shared format boundary | All public native platform-XML mutations declare their effective input/output paths and pass the shared owner guard before planning filesystem changes. New version-owning roots use the active profile. Read aliases reach the same compatibility diagnostic. |
| `cf.init` and `cf.edit` | A fresh non-nil Configuration UUID, 8.3.27 compatibility defaults, `TextToSpeech=false`, and owner-only versions are emitted. Home page `OneColumn` contains the required `Column` node. ClientApplicationInterface remains versionless and inherits the Configuration owner. |
| `cfe.init` and `cfe.borrow` | Optional base owners are checked before output creation. A borrowed source descriptor must have the exact MD namespace/root, exactly one expected object tag, an exact `Properties/Name`, and a valid non-nil UUID. Borrowed managed forms are emitted as `2.20` roots and include the platform `InternalInfo/PropertyState` structure instead of copying an incompatible subordinate version. |
| `epf.init` and `erf.init` | External descriptors and managed-form roots are created with the active profile and are validated as complete external artifacts. |
| Managed forms | The exact `{http://v8.1c.ru/8.3/xcf/logform}Form` root is required. QName bindings, attributes, parameters, columns, paths, and element layout follow the 8.3.27 declaration model. `HeaderDataPath` is rejected on `UsualGroup` because it belongs to the unsupported `ColumnGroup`; collection paths must address declared collection columns; `RowPictureDataPath` must be a subpath of the table `DataPath`; and an absent table filter is emitted as canonical `RowFilter xsi:nil="true"`. The platform child order is fixed for groups, input fields, check boxes, buttons, and tables. Type input is fully parsed and validated before any XML is emitted or written. |
| DCS emitted-XML contract | The exact `{http://v8.1c.ru/8.1/data-composition-system/schema}DataCompositionSchema` root is required. The mapped `dcs.compile` subset and the selected operation-specific `dcs.edit` branches use the 8.3.27 child order. Exact-platform canonical behavior narrows the permissive XSD where necessary: `dataSetLink/required=true` is omitted while `false` is retained; `DefinedType.*` in `valueType` is rejected because 8.3.27 removes it on roundtrip and callers must provide expanded constituent types; `StandardPeriod` start/end dates are emitted only for `Custom`; and an empty details-group `groupItems` container is omitted. Type input on mapped branches is parsed and validated before emission/write. This is not a blanket guarantee for the full JSON DSL. |
| DCS semantic mapping gaps | `dcs.compile` does not serialize field `role`, field `orderExpression`/`order`, field/calculated-field `appearance`, field `availableValues`, root templates/bindings, settings `userFields`/`additionalProperties`, or parameter `inputParameters`/`nilValue`; a parameter `value` array is not mapped as repeated values. Field shorthand `@role`/`#restrict` is also lossy. Parameter `availableValues`, settings/group `order`, settings `conditionalAppearance`, settings `dataParameters[].nilValue`, and object-form restrictions are separate mapped branches. These omissions are outside XSD/platform acceptance because the omitted nodes never reach `ibcmd`; no coverage is claimed for their JSON-to-XML fidelity. |
| Type descriptions | `Type*`, then `TypeSet*`, then `TypeId*`, then Number/String/Date/Binary qualifiers. Multiplicity, qualifier values, and XSD group order are preserved. String length is `0..1024`; number digits are `0..38`; fraction digits cannot exceed total digits; fixed-length zero and invalid sign/namespace/type spellings are rejected atomically. |
| MXL | The spreadsheet document root, empty-document sentinel, gap indices, row/cell/style order, and generated template content match accepted 8.3.27 documents. |
| `unica.template.*` metadata templates | Binary, text, HTML, spreadsheet, and DCS Template-object branches use their selected descriptors/content; empty DCS Template-object settings use the platform shape. This is distinct from embedded DCS schema `templates`, which `dcs.compile` does not map. Removal updates the containing owner atomically. |
| Metadata | All 23 exposed `meta.compile` kinds have their selected property/child-order branches represented in the corpus. Register and chart layouts were corrected. Business-process Flowchart is a `GraphicalSchema`; ExchangePlan Content uses the platform QName; DefinedType, `ValueStorage`, task addressing, attributes, resources, and tabular sections use the bounded type-description serializer. The EventSubscription checkpoint deliberately supplies DSL `String(10)` and proves canonical source identity as `xs:string` with `StringQualifiers/Length=0` and `AllowedLength=Variable`. |
| Other metadata-associated writers | Help, command interface, subsystem compile/edit, and Rights descriptors use their 8.3.27 roots/order and the active owner profile. |
| Edit/remove operations | The immutable corpus records the expected pre/post XML delta and directory topology for its selected `form.remove`, `meta.edit/remove`, `template.remove`, and create-or-modify cases. Removing the last form, metadata object of a type, or template also removes the resulting empty `Forms`, metadata-type, or `Templates` directory. The removal is bound to one full transaction snapshot: an existing sibling is preserved and a late sibling aborts with rollback. Atomic validation tests cover invalid type/root/owner paths. Neither statement extends platform proof to unselected operation arguments. |
| `unica.cfe.patch_method` | The exact EDT 8.3.27 type/role intersection used by the v1 grammar contains 51 `ModulePath` layouts. Unit contracts bind that matrix; six corpus checkpoints represent its six physical BSL layouts (CommonModule, ObjectModule, ManagerModule, RecordSetModule, Form, and ValueManagerModule), not all 51 type/path combinations. The target must be a registered adopted `2.20` object; forms additionally require an adopted wrapper and direct `BaseForm`. The writer atomically publishes the BSL interceptor and the descriptor `xr:PropertyState` with `State=Extended`; the form wrapper already contains the same state after `cfe.borrow`, so that XML update is idempotent. `meta.compile` and `cfe.borrow` share one total 8.3.27 registry profile covering all 45 metadata types: 22 non-empty `GeneratedType` profiles and 23 explicitly empty profiles. Unknown types fail closed, and ExchangePlan additionally receives exactly one `xr:ThisNode`. V1 generates only `Before`/`After` for a caller-verified existing procedure without parameters. It does not resolve the base method or prove its signature. Functions, parameters, `ModificationAndControl`, `Around`, and special form-handler semantics are outside the proven contract and are rejected or deferred rather than emitted as platform-compatible output. |
| Non-XML-only mutations | `unica.code.patch` and `unica.support.edit` still pass the owner guard where applicable. Their corpus contract requires an unchanged XML map and records the exact pre/post logical paths and bytes of BSL or binary platform payloads. `support.edit` derives its decision from one read of `ParentConfigurations.bin`, changes only its global header capability, locks vendor/object slots, preserves every existing `.cf` byte-for-byte, and binds exact `.cf` preimages plus case-insensitive directory membership to the same transaction. A concurrent `ConcurrentVendor.CF` addition aborts the `.bin` mutation; post-validation parses the exact bytes just written. `unica.cfe.patch_method` is not in this class: five selected layouts change BSL and the owning descriptor atomically, while the already-extended form layout changes only BSL. |

## Explicit native mutator/case inventory

Every public native mutator in the fixed-profile boundary is listed explicitly
below, including the two selected operations whose platform payload is
non-XML-only. The selected-case count records the complete operation/case inventory
and representative branch coverage only; it is not a claim that the pending
exact-platform gate has passed or that all argument combinations are covered.

| Public operation | Selected cases |
| --- | ---: |
| `unica.cf.edit` | 3 |
| `unica.cf.init` | 1 |
| `unica.cfe.borrow` | 2 |
| `unica.cfe.init` | 1 |
| `unica.cfe.patch_method` | 6 |
| `unica.code.patch` | 1 |
| `unica.epf.init` | 1 |
| `unica.erf.init` | 1 |
| `unica.meta.compile` | 23 |
| `unica.meta.edit` | 1 |
| `unica.meta.remove` | 1 |
| `unica.help.add` | 1 |
| `unica.form.add` | 1 |
| `unica.form.compile` | 1 |
| `unica.form.edit` | 1 |
| `unica.form.remove` | 1 |
| `unica.interface.edit` | 1 |
| `unica.subsystem.compile` | 1 |
| `unica.subsystem.edit` | 1 |
| `unica.template.add` | 5 |
| `unica.template.remove` | 1 |
| `unica.dcs.compile` | 1 |
| `unica.dcs.edit` | 4 |
| `unica.mxl.compile` | 1 |
| `unica.role.compile` | 1 |
| `unica.support.edit` | 1 |
| **Total exact-platform checkpoints selected** | **63** |

## Public-operation evidence inventory

The generated corpus is produced through public application calls. Its manifest
schema is exactly `schemaVersion: 2`. It captures immutable XML and non-XML bytes
and empty-directory topology before and after each call, verifies every declared
hash and directory again, rejects undeclared files or directories, classifies
the exact create/modify/remove delta, and keeps versionless XML tied to one
same-source-set owner.

| Inventory | Count / identity |
| --- | --- |
| Mandatory public cases | 63 |
| Platform checkpoints | 63: 52 configuration, 9 extension, 1 EPF, 1 ERF |
| Pre-call XML documents | 297 |
| Post-call XML documents | 352 |
| Static XML inputs, pre plus post | 649 |
| Pre-call platform non-XML payloads | 75 |
| Post-call platform non-XML payloads | 110 |
| Stable auxiliary payloads outside platform checkpoint roots | 141 |
| Total regular corpus files | 1039, including 63 case reports and one manifest |
| Empty corpus directories | 90 |
| Pinned normalized case-contract SHA-256 | `52d9889946c1e44ebf542721eee8a83c4ba525526f97fb1fe2f4f74074a7a161` |

Representative branch counts include 23 `meta.compile` kinds, five
`template.add` kinds, three `cf.edit` branches, two `cfe.borrow` branches, and
four order-sensitive `dcs.edit` branches. Six `cfe.patch_method` checkpoints
cover CommonModule, ObjectModule, ManagerModule, RecordSetModule, Form, and
ValueManagerModule layouts with registered adopted extension objects,
platform-canonical UTF-8 BOM/CRLF BSL, and the descriptor `Extended` state
required by 8.3.27. These six cases prove physical layout
families, while the exact 51-path type/role matrix is a separate EDT-backed
unit contract. They do not prove arbitrary base-method signatures. Every other
registered native mutator in scope has at least one mandatory case.

The current corpus was generated independently twice. Fresh UUID-dependent
content and hashes make the raw manifests byte-different, so raw directory
equality would be a false reproducibility requirement. Both runs nevertheless
produced the same normalized case-contract digest above, with the same
63-case/63-checkpoint inventory, 1039 files, and 90 empty directories. The
digest binds the empty-directory inventory in addition to public calls and
expected pre/post transitions. Each raw corpus remains independently
hash-checked and topology-checked during its verifier run.

Only the volatile internal runtime cache at `.build/unica` is removed before
the pre-call capture and again after the public call. It is neither platform
source nor immutable operation evidence. This exclusion does not cover the
whole `.build` tree: every other regular workspace payload outside the selected
platform checkpoint roots is declared as auxiliary evidence and must remain
byte-identical.

## Static XSD/XDTO result and limitations

The runtime archive contains 56 schemas. Fifty-four compile. The two known
source incompatibilities are an `xs:restriction` without `base` in the ECS
schema and an external W3C `xml.xsd` dependency. These are properties of the
exported evidence, not failures in generated Unica XML.

The current 649-document static run has zero failures:

| Static classification | Documents | Meaning |
| --- | ---: | --- |
| `pass` / strict | 11 | Ten DCS documents and one Rights document validate through the controlled document-root/type binding. |
| `inconclusive` / known schema incompatibility | 122 | The relevant exported runtime schema has a known source incompatibility or cannot accept the platform document as a complete dump root. |
| `inconclusive` / not covered | 516 | 515 documents have EDT declaration evidence but no compatibility-tested complete runtime dump-document schema; one `GraphicalSchema`/`Flowchart.xml` document has structural-profile evidence only. |
| `fail` | 0 | No well-formedness, root, QName-prefix, owner-version, or applicable strict-schema violation was found. |

`Inconclusive` is deliberately not promoted to `pass`. Raw XSD alone cannot
prove complete configuration/source-tree validity; the exact-platform gate is
therefore authoritative.

## Exact-platform gate

For every selected checkpoint, the verifier proves that the complete pinned
8.3.27.2074 install tree is unchanged, uses its pinned `ibcmd`, and runs two
isolated import/export cycles, with the second cycle consuming the first export.
Configuration checkpoints import and apply the source into a new
infobase, run platform `check`, and export it. Extension checkpoints additionally
import and apply a base configuration, then create, import, check, apply, and
export the extension. EPF/ERF checkpoints use the platform's external-artifact
path: each round materializes the descriptor/content pair under a private source
directory, passes that directory to `ibcmd config import` rather than passing
the descriptor file, and exports the resulting artifact back to XML. This
checkpoint path invokes no artifact apply/check stages. Source/base checkpoint
identity and both roundtrip comparisons include the exact empty-directory set.
The input corpus is rehashed and topology-checked after the run and must remain
unchanged.

The result is scoped to the checkpoint IDs and immutable bytes named by that
corpus manifest. It does not establish platform behavior for unselected public
arguments, and it cannot prove the fidelity of JSON fields that a writer omitted
before producing the imported XML.

Final result: `PASS`. The complete gate processed all 63 selected checkpoints:
63 passed, 0 rejected, 0 source errors, and 0 unstable roundtrips. It executed
432 platform commands over two cycles per checkpoint. The corpus stayed
unchanged (1039 files and 90 empty directories), as did the pinned platform
installation (4337 files, 96 directories, and the recorded installation hash).

The gate accepts only `pass` for all 63 checkpoints. Any rejected import,
platform-normalized delta that remains non-equivalent after the documented
semantic-multiset normalization, unstable second roundtrip, source error, or
corpus mutation fails the gate.

## Semantic versus byte canonicality

The platform orders configuration-specific `Type` values using the surrounding
configuration's `GeneratedType/xr:TypeId` index. A standalone form, DCS, or
metadata serializer does not possess that global index, so it cannot promise
the platform's byte order for those values.

The verifier therefore treats only contiguous repetitions inside each XSD
group (`Type`, `TypeSet`, or `TypeId`) as a semantic multiset. It remains strict
about group order, multiplicity, interleaving, every qualifier, all other XML
structure, and second-roundtrip stability. The proven contract is platform
acceptance plus semantic stability, not universal byte-for-byte canonical
output.

## Next design phase

After the selected-corpus platform gate passes, issue #126 can use the bounded
8.3.27 / 2.20 emitted-XML baseline to design an optional capability-driven
schema/XDTO service. The known DCS semantic mapping gaps remain separate work
and must not be reclassified as platform-proven merely because omitted XML is
accepted. The design must make profile selection explicit, report the
quality/coverage of its schema source, keep the native `unica.*` boundary, and
preserve the no-implicit-migration rule. Multi-format support must add profiles
rather than weaken this baseline.
