# Task 5B v3 brief: production Project Discovery Platform XML evidence

> **SUPERSEDED BY TASK 5B v5 — DO NOT IMPLEMENT FROM THIS BRIEF.** The current
> conditional authority is `task-5b-contract.md` v5 plus its v5 self-audit. This
> file is retained only as the immutable v3 handoff whose pre-supersession
> SHA-256 was
> `03fa268474f5c2937208fce6553837d2d8ffb759de158b94246db05023146d86`.

Status: **conditional implementation brief, blocked on a future accepted Task 5A
commit**, 2026-07-18.

This brief intentionally claims no current HEAD/base SHA. Task 5B starts only
after Task 5A is accepted, committed, clean, and spec-synchronized. Before the
first RED, record its exact 40-hex SHA as `TASK5A_ACCEPTED_SHA` in the Task 5B
report. Do not build on a moving/uncommitted candidate.

Read `.superpowers/sdd/task-5b-contract.md` completely. That v3 contract is the
authoritative design for this slice. Also read the accepted Task 4 snapshot
contract/report, Task 5A report, active architecture spec, current code, and
package-contract tests. Code/tests/package metadata remain stronger than this
ignored handoff.

Task 5B implements one shared Platform XML parser family and two production
snapshot-bound adapters:

```text
PlatformXmlMetadataCatalogProvider = unica.platform_xml_catalog / 1
PlatformXmlFormInspectionProvider  = unica.platform_xml_forms / 1
```

It implements exactly seven flows: EventSubscription, managed Form
Command/Action, CommonCommand callback requirement, ScheduledJob, HTTPService
route, ExchangePlan through SubscriptionSource, and Report/DataProcessor Form
ownership. It does not implement BSL parsing, call graphs,
ParentConfigurations, receipt storage/issuance, the public MCP tool, the guard,
or unsupported BSP/direct-exchange mechanism families.

## Gate 0: configuration-only receipt-grade CFE join

General Explore remains valid for a Platform XML configuration or extension.
For a `unica.cfe.patch_method` proposal, however, the destination UUID
membership join is valid only when the captured analysis source kind is exactly
`configuration`; `cfe_preflight_blockers` blocks when
`SourceSetKind != Configuration`.

Immediately after source resolution/capture and before membership-query
construction or provider I/O, apply a per-proposal preflight. For each affected
proposal:

```text
blocker = cfe_analysis_configuration_required
verdict = Unknown
receipt eligible = false
receipt issuer calls = 0
DestinationMembershipPair count = 0
```

Emit exactly this application-owned Check:

```text
provider=DiscoveryPreflight
code=mutation_preflight
state=Skipped
outcome=Inconclusive
coverage=Unknown
severity=Blocking
affects=[nonempty sorted unique proposal:<id> values for affected proposals only]
reasonCode=cfe_analysis_configuration_required
retryable=false
details=[]
evidenceIds=[]
```

Extend `Check::validate` with that closed tuple. Do not use
`ProjectSourceResolverPort/source_readiness`: extension analysis is generally
supported.

Preserve accepted Task 5A planning exactly:

- `preflight_blockers = cfe_preflight_blockers(original_request, snapshot)`;
- all proposals blocked -> `cfe_preflight_report` early-returns Insufficient before
  every evidence-provider/issuer call, with empty evidence/flows/candidates/
  related/coverage gaps, every fact Unknown, and only the preflight blocker;
- mixed -> clone to `provider_request`, remove every blocked proposal, normalize
  `provider_plan`, and derive all Metadata/Form/support keys and membership pairs
  only from that plan. Blocked proposals keep an empty-evidence Unknown verdict;
  unrelated proposals continue collection and never inherit the blocker. The
  aggregate request remains receipt-ineligible and the issuer is not called;
- join the original proposals and blocker map only during final application
  validation/report assembly.

Do not add `BaseMetadataIdentity` in v1. An adopted analysis-extension object's
local UUID is a wrapper UUID; an Own analysis-extension object has no base UUID.

## Mandatory architecture boundary

- Extend the neutral `infrastructure/platform_xml.rs`; do not create a second
  registration parser under discovery.
- Task 4 and Task 5B call one two-phase parser family. Before snapshot authority,
  Task 4 streaming-validates UTF-8/XML QName/namespace/no-DTD/depth/node ceilings,
  capture envelope/cardinality, registration, direct Name/UUID identity, and every
  registered root/nested Form/Template/Command. No DOM is allocated before that
  streaming gate.
- Providers receive typed query plans, captured snapshots, the verified
  `SourceSnapshotPort` reader, and an injected monotonic budget. Present XML is
  read only through `read_verified`; mandatory Form.xml absence is catalog-vs-
  manifest state and never uses `read_optional_verified`.
- Provider code contains no direct filesystem access, path existence check,
  glob, canonicalize, directory walk, SQLite, CLI/renderer, or workspace reopen.
- Build every read key with one shared source-root/typed-relative-path helper.
  Root `.` maps to `Configuration.xml`, never `./Configuration.xml`.
- Validate query roles, source sets, kinds/formats, pair identities, and limits
  before reader call 1. A mismatch is non-retryable
  `ProviderContractViolation(platform_xml_source_mismatch)`, not Unavailable.
- Construct freshness only from the exact linked `SourceSetSnapshot`; callers
  cannot provide a fingerprint.

The XML providers own observed XML facts and gaps only. Definition compatibility,
callback alias/signature policy, runtime negative permission, proposal support,
and receipt eligibility remain application concerns.

## Closed query and completeness

Use the versioned analysis scope `FullRegisteredCatalogV1`.

Both typed provider query plans carry that closed scope. Metadata query also
carries application-derived, sorted source-qualified
`analysis_presence_query_keys`; Form query carries exact
`command_presence_query_keys`. Each key is a valid potential registration identity
that may legitimately resolve Present or Absent; it is not proof the object was
already observed. Validate key shape/source before I/O without consulting the
observed catalog. Full-scan registered positives are a separate output set.
Providers never reinterpret raw task/search text to construct negative scope.

MetadataCatalog always scans the full direct Configuration registration set,
every registered root descriptor, every shared registered nested descriptor
identity, all supported kind fields and references, and only the exact requested
destination membership pairs. It also emits explicit analysis MetadataAbsent
facts for exact typed presence-query keys missing after the complete scan.

FormInspection accounts for every registered analysis managed Form's mandatory
`Ext/Form.xml`, not only explicit command subjects. It first compares the
catalog-derived key with the immutable manifest, reads every present material, and emits all observed
FormCommands plus explicit MetadataAbsent for exact requested command presence keys
missing after the complete scan. It alone owns:

- `MetadataPresent|MetadataAbsent(FormCommand)`;
- `Form -> contains -> FormCommand`;
- `FormCommand -> handles -> same FormModule.Method`.

MetadataCatalog owns every other structural fact. `task`, concepts, search
terms, known artifacts, filesystem order, `maxCandidates`, and support state
never narrow either authoritative scan. `maxEvidence` truncates canonical output
only after semantic consistency/completeness checks.

Complete silence is negative proof only inside the exact typed scope. Destination
XML is read only for canonical `DestinationMembershipPair` keys; a mutation
sibling outside those keys is never scanned.

One composite `MetadataCatalogQueryPlan`/provider invocation is the authoritative
Task 5A boundary. It contains the analysis full scan plus all exact destination
pairs. Internally process deterministic groups: analysis first, then canonical
paired destination sources. SourceSetWide is local to one group; QueryWide means
the whole composite call. Back-propagate this into Task 7, replacing its older one-
Metadata-call-per-source wording and corpus RED.

Before limits, every requested analysis metadata/command key has exactly one
semantic Present or Absent polarity. In Bounded, every missing key is covered by
an exact/matching-source/global gap. Complete omission, out-of-scope Absent, or
wrong-source polarity is a provider contract violation; full-scan positives are
allowed outside the requested negative set.

## Shared parser and exact identity

The pure parser family returns a typed Configuration catalog, registered root
descriptors, and registered nested Form/Template/Command descriptors with exact
provenance. It uses direct-child local-name semantics, exact singleton
cardinalities, canonical ArtifactRef identity comparison, UTF-8 with optional
BOM, and rejects DTD/entity declarations.

The two phases are exact:

1. `capture_authority`: streaming resource/UTF-8/XML/QName/namespace/no-DTD plus
   capture envelope/registration/direct identity/cardinality; any Configuration/
   root/nested failure or overflow is snapshot-fatal before authority/provider;
2. `semantic_views`: only after a capture-valid envelope and verified reread;
   malformed mechanism semantics may be local Bounded. Form.xml is selected by
   Task 4 but semantically parsed only here, so its malformed/overflow/incomplete
   binding catalog is exact Form Bounded.

A scalar concatenates direct Text/CDATA chunks in document order; comments/PIs
contribute nothing; a direct element makes it malformed. Configuration
`ChildObjects` is closed over the versioned domain MetadataKind registry: unknown
direct element kinds are capture-fatal, while registry-known kinds outside the
seven flows are still registered/identity-validated without a guessed mechanism.

The shared catalog also exposes fixture-backed `ConfigurationFlavor` and
NamePrefix. Base is absent direct ObjectBelonging + absent Purpose. Extension is
exact Adopted + one exact Patch/Customization/AddOn Purpose.
ConfigurationExtensionCompatibilityMode and
KeepMappingToExtendedConfigurationObjectsByIDs are optional validated fields on
either flavor and never discriminators: tracked base fixtures contain compatibility
mode, while the valid mode-b extension omits both. Preserve NamePrefix as Missing/
Empty/Value. Back-propagate this correction and its mixed/duplicate/N/N+1 REDs into
Task 8/spec.

Object UUID is exactly on the direct object element:

```text
MetaDataObject/{RootKind}/@uuid
MetaDataObject/Form/@uuid
MetaDataObject/Template/@uuid
MetaDataObject/Command/@uuid
```

Require one unqualified exact `uuid`. Never fall back to
`MetaDataObject/@uuid`, descendants, `InternalInfo`, namespaced/case lookalikes,
or another parser. Use domain-owned `PlatformUuid`: exact 36-byte hyphenated
ASCII hex, canonical lowercase, non-nil.

Registered descriptor kind and direct `Properties/Name` must exactly match the
registration. Task 4 rejects a swapped or wrong-name nested descriptor before
its subtree becomes authoritative.

The shared parser also returns complete `CompleteFormMethodBindingsV1`, covering
exact logform `/Form/Events/Event`, recursively supported registered item Events,
and all `/Form/Commands/Command/Action`. It owns exact item name/id paths, handler,
event/command name, zero-based Action ordinal and absent=Direct or exact
Before/After/Override callType. Unknown/illegal item/container, duplicate identity,
wrong-namespace or unconsumed binding-shaped node, unsupported callType, malformed
handler, or semantic limit makes the whole Form catalog incomplete; no prefix may
prove a method Ordinary/unbound. Task 8 consumes independent analysis and
destination catalogs/digests; the public Task 5B projection still emits only the
seven v1 flows. Task 6 and the Form parser share one domain-owned canonical Unicode
method identity. Item/Event/Command/handler identifiers are 1..=512 UTF-8 bytes
and <=128 Unicode scalars. Item `id` is an opaque exact token, 1..=256 UTF-8 bytes
and <=128 scalars with no Unicode whitespace/control; it is never numeric- or
case-normalized. Back-propagate these constants and N/N+1 REDs into Task 8 §2.2/
§15.0 before its implementation.

ScriptVariant comes only from direct
`Configuration/Properties/ScriptVariant`: Missing, Known Russian/English, or
Unknown exact control-free, non-whitespace-only <=256-byte token. Missing/Unknown and even an invalid
mechanism field do not make Task 4 capture fail. The parser preserves Unknown
verbatim; the provider emits non-callback facts plus a scoped gap and does not
copy the token into a stable reason or guess a callback row.

## Destination membership

For every exact pair, emit analysis `MetadataIdentity` from the direct object
UUID and destination `CfeObjectMembership` from direct Properties only:

- neither ObjectBelonging nor ExtendedConfigurationObject => Own;
- exact Adopted plus one valid ExtendedConfigurationObject => Adopted(UUID);
- every other/duplicate/invalid shape => exact-pair Bounded gap, no partial fact.

Never compare the destination wrapper UUID. A requested destination registered
Form without mandatory `Ext/Form.xml` yields a Bounded exact Form
`registered_form_material_missing` gap; unrelated pairs survive and the Form can
never become ExtensionOwned.

Derive the expected Form.xml key only from the capture-validated registration
catalog and compare it to the immutable manifest before I/O. Missing key emits the
gap with zero reads and no tombstone. Present key uses exactly `read_verified`;
NotInManifest after a manifest-present decision or any forged/noncatalog key is
`platform_xml_snapshot_catalog_mismatch`. Apply the same rule to registered
analysis Forms in FormInspection.

Preserve Task 5A's pre-limit companion rules. Present requires exactly one
identity/membership semantic value, Absent forbids it, impossible multi-values
are contract violations, and a Bounded missing companion must be covered by an
exact Artifacts, matching SourceSetWide, or QueryWide gap.

## Exact flows and ownership

1. EventSubscription emits presence, exact subscribes CommonModule handler, and
   zero or more URI-resolved SubscriptionSource uses facts.
2. FormInspection emits specialized owner/Form/Command existence, contains, and
   every direct Action binding with typed Direct/Before/After/Override. The shared
   catalog additionally retains complete direct Form Events and recursively
   supported item Events for Task 8 collision/negative proof, without exposing a
   new public v1 Event flow. Multiple Actions are retained; absent callType is
   Direct; zero Actions means command presence without handles.
3. CommonCommand emits presence plus a pending callback requirement only. It
   never emits an unconditional CommonCommand binding and never reads BSL.
4. ScheduledJob emits its exact binding and enabled state. Only lowercase true/
   false is valid; disabled produces no runtime connection/candidate.
5. HTTPService emits canonical route presence and exact service Module handler.
6. ExchangePlan is connected only by
   `ExchangePlan --uses--> EventSubscription --subscribes--> Method`; it is not
   a v1 candidate and has no direct handles row.
7. Report/DataProcessor ownership preserves specialized identities through Form
   and FormCommand; never flatten or infer BSP print conventions.

The active binding matrix must contain `SubscriptionSource + uses` from
MetadataCatalog and the FormInspection structural exception. Remove/reject old
CommonCommand binding, ExchangePlan handles, ExchangePlan-candidate, and blanket
Metadata+CallGraph+FormInspection negative wording.

## Callback registry and application join

Use registry version `platform-callbacks/v1` and exactly four rows:

- English/Russian Document ObjectModule BeforeWrite/ПередЗаписью: procedure,
  3 by-ref parameters, no defaults, Export not required, object-module server
  default context;
- English/Russian CommonCommand CommandModule
  CommandProcessing/ОбработкаКоманды: procedure, 2 by-ref parameters, no
  defaults, Export not required, explicit AtClient.

Parameter names do not matter. XML emits one pending requirement for each
registered owner only with Known ScriptVariant. EvidenceGraph aggregates
location-distinct equivalent requirements and joins exact Definition evidence.
The whole-fact constructor binds registry version, owner, module/method, slot,
shape, and context into the stable digest and rejects every cross-owner or
non-registry row. The Definition query includes the selected method and only the
official opposite-language row for the same owner/slot; no character-class alias
heuristic is allowed.
Compatible shape creates a runtime edge; exact complete mismatch is No;
opposite official language row and unproven Val/default/extra-optional variants
remain Unknown with exact application-owned reasons. Unsupported lifecycle,
direct exchange, and BSP mechanisms remain Unknown. XML never owns these
verdicts.

## QName, HTTP, witnesses, and coordinates

Resolve each direct EventSubscription Source/Type QName using XML 1.0 Fifth
Edition NCName for the prefix and its in-scope namespace. The only supported URI
is exactly `http://v8.1c.ru/8.1/data/enterprise/current-config`, and the local
part is exactly `ExchangePlanObject.<CanonicalIdentifier>`. Prefix spelling is
not semantic. Validate a referenced plan as Validated, Absent, or Inconclusive;
never turn a resource gap into dangling malformed material.

HTTP preserves case, Unicode, percent spelling, braces, and valid terminal slash.
It does no decoding/casefolding/slash collapse/dot removal. RootURL, Template,
route-length, segment, and seven uppercase verb rules are exactly those in the
v3 contract. Future-valid noncanonical shapes are scoped unsupported; malformed
singletons are document-local Bounded.

Emit multifield semantic facts at every contributing field so graph dedup keeps
all evidence IDs. Emit explicit absence at the completely scanned collection.
Use the witness table in the v3 contract, including FormCommand present/absent
and destination Own/Adopted.

Locations use the exact manifest key and original verified UTF-8 bytes: one-based
line/column, CRLF or bare CR/LF as one line break, Unicode-scalar columns, and BOM
invisible. Omit coordinates only for a genuinely unexpected mapper failure.

## Failure isolation and limits

Task 4 capture rejects capture-authoritative Configuration/root/nested invalid
UTF-8/XML/QName/namespace/DTD, depth/node overflow, malformed envelopes/
registration/direct identity/UUID/cardinality, and required descriptor absence.
The provider is never called for that snapshot, so no RED may expect provider-
Bounded for those failures.

After a capture-valid verified reread, a malformed mechanism field is local
Bounded for its semantic view. Form.xml is not semantically parsed by Task 4:
missing expected manifest key, malformed Form XML, Form depth/node overflow, or
incomplete event/action catalog is exact Form Bounded with no Form prefix.
Discard the affected semantic-view sub-batch, preserve independent capture
identity and unrelated documents/source groups, and use the narrowest safe gap.
ScriptVariant is independent. If invalid handler/action/QName prevents naming all
affected endpoints, use SourceSetWide. A repeated capture guard disagreement over
identical verified bytes is Failed `platform_xml_parser_invariant`, not Bounded.

Whole-port Failed is reserved for a shared/global parser invariant with no
reliable sub-batch. Fingerprint/read uncertainty and deadline are retryable
Unavailable and discard the whole invocation prefix. Provider contract/freshness
forgeries are fatal contract violations.

Preflight each capture/provider XML byte stream before DOM allocation: <=64 MiB,
depth <=128, element starts <=1,000,000, checked arithmetic, UTF-8/XML QName/
namespace well-formed, no DTD/entities. Parse one DOM at a time. Use one injected
120-second monotonic deadline per port call;
check before/after every read, preflight, DOM parse, and document loop. A late
deadline still discards all staged records.

Keep at most canonical `maxEvidence` records with bounded memory after semantic
checks. Canonicalize/deduplicate gaps first. Limits are:

- at most 256 distinct provider gaps;
- at most 2,000 total exact affected-artifact entries across all gaps.

At 257 gaps or 2,001 affected-artifact entries, replace the entire provider gap
set with one permutation-independent QueryWide `platform_xml_gap_limit` gap.
Never use application sentinel `material_gap_reason_overflow` in a provider.

## Required RED -> GREEN order

1. Gate on and record a clean accepted Task 5A SHA. Add exact all-invalid early
   zero-provider/issuer-I/O and mixed blocked-proposal-removal REDs.
2. Add shared capture/semantic parser REDs: no DOM before ceilings; capture-fatal
   Configuration/root/nested invalid UTF-8/XML/DTD/resource/identity; direct UUID;
   nested identity; direct Text/CDATA through comments; closed Configuration
   registry; fixture-backed Base/Extension flavor with optional compatibility/
   KeepMapping; NamePrefix and ScriptVariant semantic isolation. Rerun every Task
   4 test.
3. Add query/source guards, Present-or-Absent key constructors, one composite
   Metadata invocation/source-group isolation, and spy-reader zero-I/O tests.
4. Add Metadata full scan, explicit analysis absence, exact destination pairs,
   membership companions, and expected-Form-key manifest absence/present-read/
   forged-path tests with zero `read_optional_verified` calls.
5. Add complete Form catalog REDs for form Events, every supported recursively
   visited item Event, every Command Action, exact call types/ownership/ordinals,
   unknown/unconsumed/duplicate fail-closed cases, independent analysis/
   destination catalogs, and malformed Form A + valid Form B isolation. Then add
   the public FormInspection command projection.
6. Add EventSubscription/QName tri-state, callback registry/application join,
   ScheduledJob, HTTP, ExchangePlan uses chain, and specialized Report/
   DataProcessor ownership.
7. Add exact witnesses/coordinates, maxEvidence, 256/257 gaps, 2,000/2,001 affected
   artifacts, fake deadline, OS races, same-snapshot processing permutations, and
   separate XML-byte mutation/fingerprint tests.
8. Back-propagate one composite Metadata call into Task 7; complete shared Form
   catalogs into Task 8; and one canonical Unicode method identity into Task 6.
   Synchronize Task 5A terminology, active spec, report/plan and product contracts.

Byte identity applies only when reversing internal registration-vector/document-
worklist/source-group/provider/record/gap processing over the **same immutable
snapshot bytes, manifest, fingerprint and locations**. Reordering XML bytes or
changing prefix/whitespace/line endings creates a new fingerprint and changes
source-bound evidence/analysis IDs as applicable; only source-free semantic
payload/digest equality may be asserted separately.

## Delivery

Write `.superpowers/sdd/task-5b-report.md` with actual RED/GREEN evidence, fixture
provenance, `TASK5A_ACCEPTED_SHA`, exact implementation SHA, commands/results, and
the completed Task 6/7/8 back-propagation evidence. Run focused
provider/application/snapshot tests, the full
locked unica-coder suite, formatting, clippy with warnings denied, product
contracts, Windows compile, and `git diff --check`.

Tracked Task 5B changes commit only as:

```text
feat: добавить typed platform xml evidence
```

Stop rather than broaden v1 when an XML shape or platform behavior is not backed
by the closed contract and an authoritative fixture/source.
