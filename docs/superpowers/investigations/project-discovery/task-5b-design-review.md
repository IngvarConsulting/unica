# Task 5B adversarial design review — production Platform XML providers

> **SUPERSEDED HISTORICAL REVIEW — DO NOT IMPLEMENT FROM THIS FILE.**
> Its findings were inputs to the later contracts. Where this review differs
> from `task-5b-contract.md` v5, v5 wins. In particular, source mismatch is non-retryable
> `ProviderContractViolation(platform_xml_source_mismatch)`; missing expected
> registered Form.xml is a local Bounded `registered_form_material_missing` decided
> from catalog-vs-immutable-manifest state before I/O; capture-authoritative
> Configuration/root/nested invalid XML/resource/identity is snapshot-fatal; gap
> overflow is `platform_xml_gap_limit`. This file is evidence of review history,
> not an acceptance checklist.

| Historical topic | Current v3 rule |
| --- | --- |
| query/source mismatch | non-retryable `ProviderContractViolation(platform_xml_source_mismatch)` before I/O |
| missing catalog-derived registered Form.xml | compare catalog key to the immutable manifest; local Bounded `registered_form_material_missing`, zero reader calls |
| invalid/over-limit Configuration or root/nested descriptor | capture-fatal; the provider is not invoked |
| provider gap overflow | one permutation-independent QueryWide `platform_xml_gap_limit` sentinel |
| Metadata invocation boundary | one composite call with deterministic source-local groups and `SourceSetWide` isolation |

Date: 2026-07-17

Review status: **changes required before implementation**.

Scope: independent read-only review of:

- `.superpowers/sdd/task-5b-contract.md`;
- `.superpowers/sdd/task-5a-destination-membership-design.md`;
- `.superpowers/sdd/task-5a-runtime-port-audit.md`;
- `.superpowers/sdd/task-5a-final-fix-design.md`;
- `spec/architecture/extension-point-discovery.md`;
- the live discovery ports/model and the current Platform XML, source snapshot,
  source resolver, and contained-filesystem infrastructure.

No tracked source was edited and no test was run while Task 5A was moving.

## Verdict

The seven supported XML flows are implementable, and the snapshot reader is a
strong foundation: manifest membership precedes I/O, reads are content verified,
Unix uses component-wise `openat/O_NOFOLLOW`, and Windows retains and revalidates
the component handle chain. The Task 5B contract is nevertheless **not yet an
implementable authoritative contract**. Six P0 boundaries are either wrong,
contradictory, or underspecified enough to permit a false `Contradicted`, a false
`ExtensionOwned`, or evidence that cannot satisfy the promised provenance.

Do not start the production adapters by translating the current handoff literally.
First apply the contract corrections below and make them RED at the application
port boundary. The implementation should then be a pure parser plus a provider
orchestrator over `SourceSnapshotPort`; it must contain no direct filesystem I/O.

## P0 findings

### P0-1 — the UUID XML path in the accepted membership handoff is wrong

The handoff says to read the descriptor UUID from `MetaDataObject/@uuid`. Platform
XML does not put an object UUID there. The UUID is on the direct object element:

```text
MetaDataObject/{RootKind}/@uuid
MetaDataObject/Form/@uuid
```

Tracked fixtures and native writers consistently use, for example,
`<MetaDataObject><Report uuid="...">` and
`<MetaDataObject><Form uuid="...">`. Reading `MetaDataObject/@uuid` would make
every valid identity absent or encourage an unsafe descendant/attribute fallback.

Required correction:

1. State the two exact direct paths above in Task 5B, the active spec, and the
   membership design.
2. Require exactly one unqualified attribute whose local name is exactly `uuid`
   on the already identity-validated direct object element. No descendant,
   `InternalInfo`, nested child UUID, namespaced lookalike, or case-folded attribute
   is accepted.
3. Parse it through domain-owned `PlatformUuid`; never through the permissive UUID
   parser used by another operation.
4. Locate `MetadataIdentity` at that exact attribute byte range.

This is a release blocker because the UUID join is the authority for
`ExtensionOwned` and receipt eligibility.

### P0-2 — `analysis=extension` makes the proposed UUID join semantically false

The active source-readiness matrix permits a Platform XML configuration **or
extension** as the analysis source. The membership design, however, assumes that
the analysis descriptor's local `@uuid` is the base object's UUID. That is true for
a configuration, but false for an adopted object in an extension:

- the extension descriptor's local `@uuid` is the extension wrapper UUID;
- `Properties/ExtendedConfigurationObject` is the base configuration UUID.

Comparing a second extension's `ExtendedConfigurationObject` with the first
extension's local `@uuid` therefore rejects the same base object. Worse, an own
same-name object in an analysis extension has no base identity at all.

Required v1 correction: a validate proposal carrying
`unica.cfe.patch_method` may create `DestinationMembershipPair` only when the
captured analysis source kind is `configuration`. The application use case checks
this immediately after source resolution/capture and before constructing the
Metadata query or calling any evidence provider. The exact stable blocker is:

```text
cfe_analysis_configuration_required
```

This is a proposal-level fail-closed precondition, not a global source-readiness
error: general discovery against a Platform XML extension remains valid. For the
affected proposal the use case creates no membership pair, forces the proposal
verdict to `Unknown`, includes the blocker in both the proposal and receipt
eligibility, and never calls the receipt issuer. It may continue advisory evidence
collection for unrelated proposals, but no provider may be asked to perform the
invalid UUID join. It must never be converted to
`destination_extended_object_mismatch`.

The more general alternative is a new typed `BaseMetadataIdentity` lattice. For an
adopted object in an analysis extension it would use that descriptor's direct
`ExtendedConfigurationObject`, not local `@uuid`. For an **Own** analysis object
there is no base-configuration identity at all, so the lattice would still be
inconclusive; root and Form could also have mixed Own/Adopted states. Supporting
that safely requires analysis-extension membership facts, new companion rules,
new provenance, and a versioned join. It is explicitly rejected for v1 rather than
being approximated from the local wrapper UUID.

General explore against an extension remains supported. The restriction is on the
receipt-grade CFE membership join, not on all discovery.

### P0-3 — the shared catalog contract does not actually cover the material that
Task 4 and Task 5 must agree on

`PlatformConfigurationCatalogV1` currently contains only `ScriptVariant` and root
registrations. That is insufficient to make Task 4 manifest selection and Task 5
evidence parsing share one schema contract. The live snapshot path demonstrates
the drift already:

- it parses root descriptor kind/name to discover nested registrations;
- it merely requires registered nested `Forms/*.xml`, `Templates/*.xml`, and
  `Commands/*.xml` to be regular files;
- it does not parse those nested descriptors' direct kind/name identity before
  admitting their `Ext` subtrees;
- no current shared catalog parses root/Form UUID or extension membership.

A swapped or wrong-name registered Form descriptor can therefore enter the
authoritative manifest even though Task 5 must later reject it. That contradicts
the active snapshot rule that malformed or identity-mismatched registered XML is
`malformed_source_material`.

Required correction: define one pure, schema-aware parser family, not one tiny
configuration parser plus provider-local DOM walks:

```text
PlatformConfigurationCatalogV1
  script_variant
  root_registrations[]

RegisteredRootDescriptorV1
  registration
  object_uuid
  exact Properties/Name
  nested Form/Template/Command registrations[]
  optional typed provider fields for its supported kind

RegisteredNestedDescriptorV1
  owner registration
  nested kind
  exact Properties/Name
  object_uuid
  optional CFE membership
```

The snapshot capture and providers may parse the same verified bytes at different
times; they need not cache an infrastructure type inside the domain snapshot. They
must call the same pure parser and the same direct-child/singleton/identifier
helpers. Snapshot capture must identity-validate every registered nested
Form/Template/Command descriptor before admitting its subtree. Task 5 may then
consume the richer result from a verified reread.

Canonical duplicate detection must reuse the same artifact-identity function as
`ArtifactRef`, not a second ad-hoc lowercase/path key.

### P0-4 — complete negative authority has no exact query/scan contract

The application is allowed to turn complete silence into `No` only for the exact
typed query sent to a port. Task 5B never closes the Metadata/Form query semantics:

- it does not say whether `concepts` or `searchTerms` narrow an authoritative scan;
- it does not enumerate the analysis existence subjects for which silence is
  negative proof;
- it does not state whether FormInspection scans every registered Form or only the
  explicit `command_subjects`;
- it specifies output limits, but not the pre-limit semantic scope whose
  completeness those limits degrade.

An adapter that filters XML by search terms and returns `Complete` can make a real
proposal `Contradicted`. An adapter that scans only explicit commands can make an
explore form flow disappear while still claiming complete coverage.

Choose and encode one of these designs. The simplest safe v1 design is:

```text
Metadata analysis scope = FullRegisteredCatalogV1
Form analysis scope     = Every registered managed Form/Commands in that catalog
Destination scope       = exactly DestinationMembershipPair keys only
```

`concepts`, `searchTerms`, and filesystem order never narrow those authoritative
scopes. `maxEvidence` limits emitted records only after the full semantic scan and
adds exact/source-wide/query-wide gaps for every omitted conclusion. A `Complete`
outcome therefore means all registered material in that closed v1 scope was
validated. Alternatively, add explicit `analysis_existence_subjects` and
`analysis_mechanism_subjects` to the query plans and validate one polarity/result
per key. Do not leave the meaning implicit in `DiscoveryQueryPlan`.

At the application boundary, forged Complete output must be rejected for every
explicit destination pair and every explicit analysis subject if the explicit
query design is selected. Search-term filtering may be a separate lexical provider;
it cannot change Metadata/Form negative authority.

### P0-5 — the contract assigns BSL/runtime conclusions to the XML provider

Task 5B classifies cross-language aliases and signature variants as scoped provider
gaps, but a Platform XML adapter does not read or own BSL definitions. It can emit
only a callback requirement selected from `ScriptVariant`. Whether a canonical or
opposite-alias definition exists, and whether its procedure/function, context,
arity, `Val`, defaults, or optional tail is compatible, is known only after joining
Definition evidence in the application graph.

Likewise, an implicit unsupported lifecycle/BSP/direct-exchange proposal can have
no corresponding XML node from which a provider could emit a gap. Requiring the
provider to do so would reintroduce name switches and policy in infrastructure;
omitting the gap while retaining blanket negative coverage would create false
`Contradicted` verdicts.

Required ownership split:

| Observation | Owner | Result |
| --- | --- | --- |
| missing/unknown direct Configuration `ScriptVariant` | Metadata provider | scoped `unsupported_platform_script_variant` gap |
| supported XML field has future/unsupported token or QName/URL shape | XML provider | exact scoped provider gap |
| canonical callback requirement | Metadata provider | `PlatformCallback` fact only |
| canonical definition compatible | EvidenceGraph | runtime edge |
| exact signature mismatch with complete Definition coverage | EvidenceGraph/validator | typed `callback_signature_mismatch`, `No` |
| alias or unproven signature variant | EvidenceGraph/validator | exact unsupported reason, `Unknown` |
| implicit lifecycle/command/direct-exchange/BSP family outside v1 | `RuntimeMechanismProfileV1` | `Unsupported(reason)`, `Unknown` even with complete providers |
| Definition unavailable/bounded | DefinitionPort | Definition gap; never synthesized by XML |

The XML provider remains raw fact extraction. The runtime profile remains the sole
negative-policy authority. Positive `connection_ports` are not negative authority,
and an empty positive-port set never authorizes `No`.

### P0-6 — active spec, accepted handoffs, and live model disagree on the public
binding/runtime contract

The active spec still lists `CommonCommand + handles` and `ExchangePlan + handles`
as `BindingDetails` rows. The accepted Task 5A/5B model instead uses:

- `PlatformCallback` for CommonCommand;
- `SubscriptionSource + uses` for ExchangePlan to EventSubscription;
- only a complete `uses + subscribes` chain for ExchangePlan runtime promotion.

The spec also still requires blanket Metadata + CallGraph + FormInspection
coverage for every connectionless target, while the accepted runtime audit requires
the split `RuntimeMechanismProfileV1` and explicit unsupported-negative policy.

Task 5B cannot be accepted against both contracts. Update the active spec before or
in the same commit as the providers. Product-contract tests must reject the old
rows and blanket rule. Historical plan prose that calls ExchangePlan a candidate
must remain corrected: it is contextual/connected after a full chain, never a v1
candidate.

## P1 findings and exact corrections

### P1-1 — absence and multi-field provenance are not defined

The provenance rule says every record points to a direct material field, but several
accepted facts are proofs of absence and have no such field:

- `CfeObjectMembership::Own` means both direct membership fields are absent;
- `MetadataAbsent` means no direct registration exists;
- missing `ScriptVariant` means no direct property exists;
- a missing registered `Ext/Form.xml` has no XML node.

Conversely, `Adopted` depends on two fields, but the membership handoff does not
require both locations.

Close the witness rules exactly:

| Fact/gap | Required location witness |
| --- | --- |
| root/Form `MetadataIdentity` | exact direct object `@uuid` |
| root `MetadataPresent` | identical facts at Configuration registration and descriptor `Properties/Name` |
| registered Form `MetadataPresent` | identical facts at owner `ChildObjects/Form` and Form descriptor `Properties/Name` |
| root `MetadataAbsent` | direct Configuration `ChildObjects` container |
| registered Form `MetadataAbsent` | direct owner `ChildObjects` container |
| destination `Own` | direct descriptor `Properties` container whose complete direct children were inspected |
| destination `Adopted` | identical membership facts at `ObjectBelonging` and `ExtendedConfigurationObject` |
| missing ScriptVariant | direct Configuration `Properties` container |
| unknown ScriptVariant | exact `ScriptVariant` element |
| missing `Ext/Form.xml` | registered Form descriptor `Properties/Name` or its registration, plus the missing workspace-relative path in bounded details |

The semantic payload remains identical across multiple locations, so evidence IDs
differ while the graph collapses only the fact/edge. The source fingerprint is what
makes a container witness authoritative for absence.

### P1-2 — exact QName lexical grammar is missing

The contract says “invalid QName lexical form” without defining the accepted
grammar. Text content is not validated as a QName by the XML parser, so splitting on
one colon and looking up a namespace is not sufficient. Different implementations
can disagree on leading digits, combining characters, dots, hyphens, or empty local
parts.

Specify one versioned rule. Recommended:

1. trim XML surrounding whitespace exactly once;
2. enforce the byte/control limits;
3. parse exactly one colon and validate the prefix as XML 1.0 Fifth Edition
   `NCName` (or explicitly declare a narrower v1 ASCII grammar);
4. resolve that prefix through the Type element's in-scope namespaces;
5. parse the local part through a closed platform type parser:
   `ExchangePlanObject`.`CanonicalIdentifier` is supported; other syntactically
   valid platform type identities are unsupported; malformed identifiers are
   malformed material;
6. compare namespace URI and canonical artifact identity, never prefix spelling.

Also state the exact cardinality of direct `Properties/Source` and direct `Type`
children. The tracked writer emits exactly one Source container and zero or more
direct Type children. Nested Type decoys must not count.

### P1-3 — cross-document references need a tri-state identity join

A current-config `ExchangePlanObject.X` source is not enough to emit `uses`; the
plan must be registered and its descriptor identity must be validated. The contract
handles “not registered” as malformed, but does not distinguish a registered plan
whose descriptor was discarded by a resource gap.

Use `Validated | Absent | Inconclusive` catalog identity:

- validated exact plan => emit `uses`;
- truly unregistered exact current-config plan => `malformed_registered_material`;
- registered but resource-limited/unavailable plan descriptor => no edge plus the
  exact `platform_xml_resource_limit`/read gap affecting both plan and subscription;
- never turn the inconclusive row into dangling-malformed or select another plan.

Apply the same rule to handler owner references where relevant. A syntactically
valid reference to an unregistered CommonModule must have one documented outcome;
it must not accidentally become positive merely because the binding smart
constructor accepts its shape.

### P1-4 — XML node/depth bounds are outcome bounds, not yet parser-resource
bounds

`roxmltree::Document::parse` constructs the DOM before a later descendant count can
notice one million elements or depth 129. The 64 MiB byte ceiling is a hard upper
bound, but the advertised element/depth limits do not prevent the DOM allocation.

Before calling Task 5B resource-bounded, either:

- perform a streaming/token preflight that counts element starts and depth with
  checked arithmetic, rejects DTD, and only then builds the DOM; or
- use a parser with enforced node/depth allocation limits.

`Document::parse` currently rejects DTD by default; preserve that explicitly and
test it. Parse one document at a time and release its DOM before the next. Run all
structural/singleton/resource validation for a document before emitting any fact
derived from it. Configuration overflow discards the whole catalog batch; another
descriptor overflow retains only conclusions from fully validated documents and
adds a scoped gap.

### P1-5 — provider gap overflow needs a fixed reserved sentinel

There can be more than 256 distinct gaps or more than 2,000 total exact affected-
artifact entries across gaps. The provider cannot return 257 `ProviderGap`
values, and selecting the reason of “the gap that happened to cross the limit”
makes the result input-order dependent.

Define a provider-owned reserved reason such as
`platform_xml_gap_limit`. Canonicalize all semantic gaps first; if the 256-gap or
2,000-total-scoped-artifact bound would overflow, replace the whole provider gap
set with one deterministic QueryWide sentinel using that fixed code. Do not reuse
the last observed reason. The application-level
`material_gap_reason_overflow` remains a different summary sentinel and must
never be emitted by a provider.

For record truncation, retain the canonical smallest semantic records with a
bounded heap or equivalent; do not retain all unbounded records merely to sort
afterward. Validate semantic consistency and UUID companions before truncation.

### P1-6 — line/column semantics are not precise enough for deterministic tests

“Calculate one-based line and column from the byte range” does not say whether a
column counts UTF-8 bytes, Unicode scalars, UTF-16 code units, or XML-normalized
characters. CRLF and bare CR also need an exact rule.

Define v1 coordinates over original verified UTF-8 bytes:

- line follows XML 1.0 line endings (`CRLF` is one break; bare `CR` and bare `LF`
  are one break);
- column is one plus the Unicode-scalar count from the original line start to the
  node/attribute byte start;
- a leading UTF-8 BOM is not a visible column;
- location path is the exact manifest key, not a reconstructed case variant.

If another repository-wide SourceLocation convention already exists, use it
instead, but name it and test the same BOM/CRLF/non-ASCII cases in both providers.

### P1-7 — provider deadline behavior needs an injectable monotonic contract

The 120-second statement does not specify when timing starts, where it is checked,
or how a DOM parse that returns after the deadline is handled.

Add a provider budget/clock to infrastructure construction. Start before the first
guard/read; check before and after every verified read, preflight, DOM parse, and
document loop. If the deadline is observed at any point, discard the entire
affected port batch and return retryable `platform_xml_deadline`; never return the
prefix already accumulated. Use a fake clock for exact tests, not wall time.

### P1-8 — exact source/read failure mapping must be implemented as a closed table

The generic `source_set_guard()` currently returns an unavailable outcome, while
Task 5B requires a source/query mismatch detected before I/O to be a non-retryable
provider contract violation. Do not reuse the generic behavior blindly.

Required mapping:

| Condition | Outcome |
| --- | --- |
| analysis/query or pair/source mismatch before I/O | ContractViolation `platform_xml_source_mismatch` |
| catalog-derived registered descriptor path missing from manifest | ContractViolation `platform_xml_snapshot_catalog_mismatch` |
| explicitly mandatory registered `Ext/Form.xml` missing from manifest | Failed `registered_form_material_missing` |
| `SourceFingerprintMismatch` | Unavailable retryable `source_fingerprint_mismatch`, discard batch |
| `SnapshotUnavailable`/I/O | Unavailable retryable `source_snapshot_unavailable`, discard batch |
| `NotInManifest` for a caller-forged/noncatalog path | ContractViolation, never absence |

For a requested destination Form, missing mandatory Form material must prevent an
otherwise matching UUID pair from becoming `ExtensionOwned`. State which supplying
port owns that failure; because FormInspection is analysis-only, destination
material failure must be visible through MetadataCatalog or a later resolver gate
before receipt eligibility.

### P1-9 — source-relative path construction must be shared and Windows-safe

Providers should never hand-build `"{root}/..."`; source root `.` must produce
`Configuration.xml`, not `./Configuration.xml`. Introduce one contained
`manifest_path(source.relative_root, typed_relative_path)` helper and return the
exact manifest key used for the read and location.

Production providers must have no `std::fs`, glob, canonicalize, directory walk, or
path-existence fallback. They receive copied verified bytes only. Add a code/product
contract check for that boundary and a spy reader that records every requested
manifest key.

Windows acceptance must include case-only registration collisions, Unicode names,
drive/UNC-looking text, reserved/separator/control attempts, long components,
directory/leaf reparse swaps, and a file replacement between manifest lookup and
open. Unix acceptance must include ancestor and leaf symlink swaps, FIFO/device
leaves, and hard-link content mutation. Every topology race is detected by the
existing reader; the provider must translate it, not retry through a raw path.

### P1-10 — singleton and unsupported-token outcomes need one exhaustive schema
table

The prose gives paths but leaves several cardinalities implicit. For every supported
descriptor, define direct required/optional/repeated fields and the exact outcome
for missing, duplicate, empty, malformed, and future-valid values. In particular:

- EventSubscription: one Source container, zero or more direct Type, exactly one
  Event and Handler;
- ScheduledJob: exactly one MethodName and Use;
- HTTPService: exactly one RootURL, unique direct URLTemplate names, unique direct
  Method names per template, and one Template/HTTPMethod/Handler per row;
- managed Form: exactly one direct Commands collection; command name uniqueness is
  canonical; explicitly document whether multiple direct Action children are a
  supported ordered set or a malformed singleton;
- ScriptVariant: exactly zero or one direct field; duplicate is malformed;
- membership: zero/one direct ObjectBelonging and ExtendedConfigurationObject;
  nested/sibling values never count.

The existing statement that multiple Action elements are retained is a semantic
choice, not merely parser behavior. Keep it only with a platform fixture proving
the ordering/identity semantics; otherwise fail closed as an unsupported form
action variant.

## Corrected provider boundary

### Construction

Two production adapters are sufficient:

```text
PlatformXmlMetadataCatalogProvider
  provider = unica.platform_xml_catalog / 1
  reads     = full analysis catalog + exact destination membership pairs

PlatformXmlFormInspectionProvider
  provider = unica.platform_xml_forms / 1
  reads     = registered analysis forms only
```

Both depend only on:

- the closed registry/parser family;
- `SourceSnapshotPort` verified reads;
- an injected monotonic provider clock/budget;
- evidence constructors that take a linked `SourceSetSnapshot`, never a caller
  fingerprint.

The metadata adapter must not call the form adapter. Shared pure parsing helpers are
allowed; adapter-to-adapter calls are not.

### Processing order

1. Validate query/source roles, kinds, pair identities, and limits before I/O.
2. Bind the exact analysis snapshot and exact requested destination snapshots.
3. Check manifest membership and declared XML length before every read/parse.
4. Read through `SourceSnapshotPort` only.
5. Run XML token resource preflight, DOM parse, direct schema validation, and
   semantic cross-reference validation.
6. Stage per-document conclusions; publish them only after that document is wholly
   valid.
7. Validate all UUID polarity/companion and cross-document semantic consistency on
   the pre-limit set.
8. Canonicalize facts/gaps, enforce fixed provider bounds, and mark Bounded with
   exact conservative gaps.
9. Construct one ProviderOutcome; on fingerprint/I/O/deadline discard the entire
   port batch.

### Runtime negative authority

The provider emits only observed XML facts and observed-field gaps. The application
combines them with Definition/CallGraph evidence and the closed runtime profile.
No provider Complete state, no missing edge, and no empty positive-port set is by
itself permission to answer runtime `No`.

## Mandatory RED matrix

Every deterministic row runs forward and with reversed registration/document/
record/gap order. Structured outcomes, record digests, gap order, evidence IDs, and
analysis IDs must be identical where applicable.

### A. Guard, isolation, and snapshot reads

| ID | Case | Expected |
| --- | --- | --- |
| A1 | plan analysis differs from snapshot | ContractViolation before reader call 1 |
| A2 | membership pair names an uncaptured destination | ContractViolation before I/O |
| A3 | destination pair source equals analysis | constructor/contract rejection |
| A4 | provider attempts mutation sibling not in pairs | contract rejection; no read |
| A5 | registered descriptor path absent from manifest | snapshot/catalog ContractViolation |
| A6 | unregistered decoy file present in manifest-like filesystem path | never read, no fact |
| A7 | registered Form `Ext/Form.xml` absent | exact Failed/material blocker, never MetadataAbsent |
| A8 | fingerprint changes before read | Unavailable retryable; zero returned records |
| A9 | fingerprint changes after one staged document | whole affected provider batch discarded |
| A10 | snapshot reader unavailable after partial staging | whole batch discarded |
| A11 | fake deadline before first read / between docs / after DOM parse | same retryable deadline outcome; zero records |
| A12 | root `.` | reads exact `Configuration.xml`, never `./Configuration.xml` |
| A13 | analysis and destination have same artifact name and different bytes | facts remain source-qualified and independent |
| A14 | analysis is Extension with CFE intent | stable unsupported-analysis-kind before provider I/O |

### B. Shared catalog and descriptor identity

| ID | Case | Expected |
| --- | --- | --- |
| B1 | valid prefixed MetaDataObject and exact direct children | accepted |
| B2 | root-kind/name/ChildObjects lexical descendants | ignored |
| B3 | duplicate direct singleton | malformed |
| B4 | registered root descriptor kind or Name mismatch | capture/provider reject identically |
| B5 | registered nested Form/Template/Command kind or Name mismatch | capture rejects before subtree authority |
| B6 | swapped two registered Form descriptors | malformed, never cross-bound |
| B7 | duplicate case-equivalent registration | deterministic duplicate rejection |
| B8 | root UUID on direct object element | exact canonical identity fact |
| B9 | UUID only on MetaDataObject/nested child/namespaced lookalike | invalid identity; no fallback |
| B10 | uppercase UUID | canonical lowercase; identical digest |
| B11 | nil/braced/compact/URN/padded/non-ASCII UUID | exact invalid UUID outcome |
| B12 | DTD/entity declaration | malformed; no entity expansion |

### C. Destination UUID membership

| ID | Analysis | Destination | Expected |
| --- | --- | --- | --- |
| C1 | config root UUID X | Adopted -> X | AlreadyBorrowed |
| C2 | config root+Form X/Y | both Adopted -> X/Y | ExtensionOwned if other policy passes |
| C3 | valid X | no registration | RequiresBorrow + ineligible |
| C4 | root equal, Form absent | RequiresBorrow + ineligible |
| C5 | valid X | Own same name | Unknown `destination_object_not_adopted` |
| C6 | valid X | Adopted -> Z | Unknown mismatch |
| C7 | extension wrapper W adopted from X | destination Adopted -> X | rejected as unsupported analysis kind, never compare W |
| C8 | valid X | duplicate same Adopted fields/locations | semantic one, provenance all |
| C9 | valid X | Own+Adopted or two UUID values | provider failure/contract violation, no winner |
| C10 | Present without identity/membership companion | pre-limit contract violation |
| C11 | resource gap removes companion | Unknown through exact gap, not second contract violation |
| C12 | registered adopted Form lacks mandatory Form material | cannot become ExtensionOwned/eligible |

### D. Event subscription and QName

| ID | Case | Expected |
| --- | --- | --- |
| D1 | empty direct Source | subscribes fact, no uses, Complete |
| D2 | arbitrary NCName prefix -> exact current-config URI | same resolved source/digest |
| D3 | same prefix spelling -> other URI | unsupported source type, no uses |
| D4 | undeclared prefix / empty side / multiple colon / invalid NCName | malformed |
| D5 | unprefixed syntactically valid source | scoped unsupported, not string fallback |
| D6 | current-config DocumentObject | scoped unsupported source type |
| D7 | current-config ExchangePlanObject registered+validated | exact uses edge |
| D8 | current-config ExchangePlanObject unregistered | dangling malformed material |
| D9 | registered plan descriptor resource-limited | bounded inconclusive, not dangling failure |
| D10 | nested Type decoy | ignored |
| D11 | two supported Type entries | two exact uses facts with both locations |
| D12 | supported + unsupported Type | positive known edge plus material scoped gap |

### E. Form provider

| ID | Case | Expected |
| --- | --- | --- |
| E1 | registered owner/Form/Command/Action | full owner->Form->Command->Method chain |
| E2 | Direct/Before/After/Override | four distinct stable call-type tags |
| E3 | missing callType | Direct |
| E4 | future callType | exact malformed/unsupported outcome selected by schema table |
| E5 | duplicate canonical command name | malformed |
| E6 | duplicate Action | exact documented supported-set or fail-closed outcome; never silently choose first |
| E7 | BaseForm/nested Commands decoy | ignored |
| E8 | malformed Form.xml | FormInspection failed while valid Metadata facts remain |
| E9 | report and data-processor ownership | specialized identities preserved |
| E10 | BSP/common print decoy | no inferred binding; runtime profile Unknown |

### F. Common command, callbacks, and negative policy

| ID | Case | Expected |
| --- | --- | --- |
| F1-F4 | all four registry rows | exact PlatformCallback fact and provenance |
| F5 | missing/unknown ScriptVariant | XML facts retained, callback-scoped gap, no callback edge |
| F6 | canonical compatible Definition | runtime edge |
| F7 | canonical exact mismatch with complete Definition | callback mismatch No |
| F8 | opposite language alias only | application Unknown alias reason; XML provider did not inspect BSL |
| F9 | Val/default/extra optional unproven variant | application Unknown signature reason |
| F10 | ordinary lifecycle/direct exchange/BSP proposal | runtime profile Unknown unsupported reason |
| F11 | same cases with complete CallGraph and no positive edge | still Unknown, never vacuous No |
| F12 | callback owner target + exact Definition gap | owner Unknown with exact dependency |

### G. Scheduled job and HTTP route

| ID | Case | Expected |
| --- | --- | --- |
| G1 | enabled exact job | handles runtime edge |
| G2 | disabled exact job | binding observed, no runtime connection/candidate |
| G3 | invalid/missing/duplicate Use or MethodName | exact schema outcome, no partial fact |
| G4 | valid RootURL/Template boundaries including `/` and trailing slash | exact canonical route |
| G5 | repeated slash, dot segment, backslash, query, fragment, control | scoped unsupported route, never normalize |
| G6 | percent spelling/case/Unicode/braces | preserved byte-for-byte |
| G7 | seven uppercase verbs | accepted closed tags |
| G8 | custom/lowercase verb | scoped unsupported method |
| G9 | duplicate template/method names | malformed |
| G10 | descendant field decoys | ignored |

### H. Bounds, gaps, coordinates, and OS containment

| ID | Case | Expected |
| --- | --- | --- |
| H1 | XML 64 MiB boundary and +1 | accept boundary / preparse resource outcome |
| H2 | depth 128 and 129 | accept / deterministic resource gap without DOM over-allocation |
| H3 | 1,000,000 and +1 elements | accept / deterministic resource gap |
| H4 | resource limit in Configuration | zero catalog facts + QueryWide/source gap |
| H5 | resource limit in one descriptor | other fully validated docs retained; affected doc atomic |
| H6 | maxEvidence N/N+1 with duplicate locations | semantic consistency before deterministic truncation |
| H7 | 256/257 provider gaps under permutations | 256 exact / fixed `platform_xml_gap_limit` sentinel |
| H8 | 2,000 total scoped artifacts and 2,001 | exact / fixed conservative sentinel |
| H9 | UTF-8 BOM, LF, CRLF, bare CR, mixed endings, Cyrillic before node | exact defined 1-based coordinates |
| H10 | line mapping fallback | exact manifest path retained, line/column omitted only on genuine mapper failure |
| H11 | Unix ancestor/leaf symlink swap and FIFO | reader rejects; provider maps closed outcome |
| H12 | Windows ancestor/leaf reparse swap and handle-path change | reader rejects; provider maps closed outcome |
| H13 | case-only/Unicode path aliases on Windows | no duplicate semantic identity or alternate manifest key |
| H14 | provider source contains direct filesystem call (contract/static check) | test/product contract fails |

## Acceptance verdict

Task 5B is ready to implement only after the following are represented in RED
tests or compile-time types:

1. corrected direct object UUID path;
2. configuration-only analysis for receipt-grade CFE UUID joins;
3. one shared parser family that identity-validates nested descriptors;
4. an explicit full-scan or exact-subject query completeness contract;
5. strict ownership split between XML observations and application runtime
   negative policy;
6. synchronized active spec binding/runtime matrices;
7. exact absence/multi-field witness locations;
8. fixed QName, gap-overflow, deadline, coordinate, and source-read mappings;
9. provider-only verified reads with OS-race tests;
10. the full RED matrix above.

With those corrections, the provider slice can be implemented without changing
the accepted domain facts or weakening Task 5A safety. Without them, a passing
happy-path parser suite would still leave receipt-grade discovery unsound or
unverifiable.
