# Task 5B v7 — owner contract for Platform XML evidence

Contract date: 2026-07-18.

This owner file defines contract bytes and acceptance criteria; it neither
declares nor denies its current design-acceptance state. The sole design-status
authority is the current external four-document ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`. Absence, presence,
or later transition of that ledger never requires editing this owner file.
Production implementation authorization is separate and exists only when the
successor implementation reports/ledger satisfy the OID gates below.

This document supersedes, rather than edits, rejected immutable Task 5B v6:

```text
task-5b-v6-contract.md
SHA-256 = 665de15a59749bf935dd03b8e15558347db1f93d10dc3cbc2a248b61015c8712
```

The decisive independent review is immutable
`.superpowers/sdd/task-5b-v6-independent-review.md`, SHA-256
`cef8ee1e8e12af88d10805c6824a38a2182c29b3a507becd5058a0a864d8de71`.
It found zero P0, eight P1 and two cross-task P2 defects. v7 keeps every accepted
v6 rule except the exact clauses explicitly changed here: Form main-attribute
namespace, CFE companion-gap consistency, source-free ProviderFact payloads,
closed query-vector identities, the shared Definition-observation group, the
neutral future-consumer Form seam, and the acyclic acceptance ledger. The Task 6
and Task 7 corrections live in owner-specific successor addenda; the dynamic
capture seam lives in the Task4-v7 owner addendum. Their immutable historical
Task4/Task6-v2/Task7-v6 inputs are not edited.

For history, v6 itself superseded v5
`13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab`.
All earlier briefs, rejected contracts and self-audits remain historical evidence
only. When the external package ledger selects this exact owner tuple, Task 5B
production implements this v7 contract; Task4-v7 implements the upstream
capture seam and downstream Task 6/Task 7 implement the peer addenda selected
by that same ledger. A
branch, mtime or current `HEAD` cannot substitute for ledger-recorded document
and independent-review hashes.

Design acceptance and implementation acceptance are deliberately separate.
These exact four owner documents form one atomic conditional design package:

```text
.superpowers/sdd/task-4-v7-dynamic-material-addendum.md
.superpowers/sdd/task-5b-v7-contract.md
.superpowers/sdd/task-6-v2-v7-addendum.md
.superpowers/sdd/task-7-v7-addendum.md
```

They require owner self-audits and independent reviews. The external ledger may
transition the tuple atomically only after their exact names/tags/encoders/bounds/DAG imports are
cross-checked with no P0/P1, including the Task4 raw-composite writer -> Task5B
`PlatformCatalogExecutionBindingV1` -> Task7 execution-header chain and the
Task5B `ProviderMaterialArtifactSetV2` -> Task7 admission-scope writer plus
sealed union-cardinality -> Task7 finality-threshold chains. The binding
cross-check also includes the exact three Task6 whole-context query constructors
and their owner authorities.
Transitioning one while any of the other three remains
moving is STOP: the capture provider contract could otherwise drift after a
consumer design was accepted. After tuple sealing, editing one document
invalidates the four-document package tuple, its status labels,
derived cross-document goldens, audits/reviews and ledger transition. The
content hashes of three unchanged peer byte streams remain mathematically true,
but they cannot be reused as acceptance evidence until the complete tuple is
cross-checked, re-reviewed and resealed.

This design-package transition requires no Task5A or production implementation
OID. Independently, after the four-document design co-freeze and before the
first Task 5B production RED, an owner must implement, review, accept and commit
Task 5A with every
Task5A/domain-owned back-propagation in sections 3.1 through 3.10.1 (not the
Task5B-owned 3.10.2-.4 or Task4-owned 3.11), and the Task 5B delivery worker must record the
exact 40-hex commit as:

```text
required external field name: TASK5A_ACCEPTED_SHA
value contract: exact 40-lowercase-hex accepted Task5A commit
```

The same rule applies to the Task 4 successor required by section 3.11. After
Task 5A is accepted, but still before the first Task 5B production RED, its
owner must implement, review, accept and commit the neutral dynamic registered-
material capture seam and record its exact 40-lowercase-hex Git object ID as:

```text
required external field name: TASK4_V7_ACCEPTED_GIT_OID
value contract: exact 40-lowercase-hex accepted Task4-v7 commit
```

These are symbolic required field names, not placeholders to replace inside
the frozen design. The later exact values are recorded only in successor
implementation reports/ledger, including
`.superpowers/sdd/task-5b-report.md`, after the prerequisite commits exist;
owner bytes remain unchanged. The separate design package ledger
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` is created only
after independent reviews and records the exact four-owner/audit/review SHA
tuple plus atomic accepted status. It contains no later production OIDs. None
of those hashes is self-embedded into an owner document whose hash it attests.

A dirty diff, a branch name, current `HEAD`, a topology label, or this document's
hash is not a substitute for the implementation prerequisite. A Task 5B
production RED/implementation/report may start only when successor
implementation authority contains both `TASK5A_ACCEPTED_SHA` and
`TASK4_V7_ACCEPTED_GIT_OID`; this gate does not alter or determine design status.

The implementation dependency is acyclic and machine-checked:

```text
co-frozen {Task4-v7, Task5B-v7, Task6-v7-addendum, Task7-v7-addendum} designs
  -> accepted Task5A implementation of the frozen back-propagated seams
  -> accepted Task4-v7 dynamic registered-material capture implementation
  -> accepted Task5B-v7 production implementation
  -> {accepted Task6-v2+addendum implementation,
      accepted TASK5C_EVIDENCE_ACCEPTED_GIT_OID}
  -> independently accepted Task7PrerequisiteSliceV1 implementation
  -> Task8, including Task7Task8IntegrationV1
  -> Task9 -> Task10 -> Task5C-Mutation
```

After the shared Task 5A seams and the Task4-v7 capture seam are accepted,
Task5B may start. Its two production successors are independent and the
machine-readable ledger encodes the real partial order as
`Task5A -> Task4-v7 -> Task5B -> {Task6, Task5C-Evidence} ->
Task7PrerequisiteSliceV1 -> Task8/Task7Task8IntegrationV1`. The whole Task 7
addendum is not an accepted production prerequisite before Task8: Task8 owns
and evidences the integration slice. Task 6 has no Task 5C type/import and
therefore must not wait for any Task 5C commit. Only Task 7 records and imports the exact 40-hex
`TASK5C_EVIDENCE_ACCEPTED_GIT_OID`. Design freeze is outside this production
DAG and therefore cannot form a design-to-implementation cycle.

Task 5B implements and compile-tests neutral future-consumer contracts. It does
not import Task 6 or Task 8, and no actual Task 8 import, test, commit or behavior
may gate Task 5B acceptance. `Task5C-Evidence` is the read-only predecessor
slice of Task 7 only; whole Task 5C or `Task5C-Mutation` is downstream and is
never an upstream gate for Task 6/7.

## 1. Outcome and closed scope

Task 5B supplies two snapshot-bound providers and one shared parser family:

```text
PlatformXmlMetadataCatalogProvider
  provider = unica.platform_xml_catalog / 2

PlatformXmlFormInspectionProvider
  provider = unica.platform_xml_forms / 2
```

They support exactly these eight public declarative flows:

1. a Document to its versioned canonical ObjectModule lifecycle callback;
2. a validated EventSubscription to its exact CommonModule handler;
3. a registered managed Form command/action to its exact FormModule handler;
4. a CommonCommand to a versioned platform callback requirement;
5. an enabled predefined ScheduledJob to an exact server-capable CommonModule method;
6. an HTTPService route to its exact service Module handler;
7. an ExchangePlan source through a validated EventSubscription and handler;
8. Report/DataProcessor ownership through registered Form, Command, and Action.

Task 5B does not parse BSL definitions or calls, issue/store receipts, implement
the public MCP tool, infer source topology from labels, or support arbitrary
event/source combinations. Unsupported but well-formed platform variants yield
typed `Unknown` plus an exact material gap. They never become positive runtime
edges and never become `No` merely because v1 is narrow.

The ownership boundary is strict:

| Conclusion | Sole owner |
| --- | --- |
| XML envelope, registration, capability properties, selected sources, and declarative references | Task 5B provider/shared parser |
| immutable source identity, manifest, and fingerprint | Task 4 snapshot capture |
| callable kind, Export, parameters, annotations, and BSL context | Task 6 DefinitionPort |
| cross-source companion join and callback/binding compatibility | application EvidenceGraph |
| positive runtime connection and supported negative authority | application mechanism registry |
| proposal actionability and receipt eligibility | application use case |

The XML provider emits typed observations or pending whole-fact requirements. It
does not manufacture a BSL definition, normalize an unsupported alias, or infer
reachability from a handler-looking string.

## 2. Authority and primary evidence

Repository code/tests/package metadata remain stronger than this ignored design;
the active spec must be synchronized in the implementation commit. Historical
plans are context, not authority.

The bounded v1 choices below are grounded in these primary 1C sources, accessed
2026-07-18:

- 1C:Enterprise 8.3.23 Developer Guide, `5.3.8 Event subscriptions`:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_5._Configuration_objects/5.3.__Common__configuration_branch/5.3.8._Event_subscriptions/?language=en>
- 1C:Enterprise 8.3.23 Developer Guide, `5.3.2 Common modules`:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_5._Configuration_objects/5.3.__Common__configuration_branch/5.3.2._Common_modules/?language=en>
- 1C:Enterprise 8.3.22 Developer Guide, `19.3 Scheduled jobs`:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.22_Developer_Guide/Chapter_19._Job_feature/19.3._Scheduled_jobs/?language=en>
- official 1Ci scheduled-job FAQ:
  <https://kb.1ci.com/1C_Enterprise_Platform/FAQ/Development/Misc/How_to_run_a_procedure_automatically_by_using_scheduled_jobs/>
- 1C:Enterprise 8.3.23 Developer Guide, `36.4.3 Forms`:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_36._Configuration_extension/36.4._Extension_objects/36.4.3._Forms/>
- 1C:Enterprise 8.3.23 Developer Guide, `7.6 Form module`, which limits
  developer-created Form command handlers to client methods:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_7._Forms/7.6._Form_module/>
- official 1C practical guide Form-command examples, including the generated
  synchronous and asynchronous `&AtClient ... Procedure ...(Command)` shapes:
  <https://kb.1ci.com/1C_Enterprise_Platform/Tutorials/Practical_developer_guide_8.3/Lesson_26._Picking_list_items__avoiding_modal_windows__and_generating_data_based_on_other_data/Avoiding_modal_windows/Requesting_user_input_in_a_form_command/>
- official 1C:DN HTTP-service example, where the platform-generated handler is
  a non-exported, unannotated `Function ...(Request)` in the service Module:
  <https://1c-dn.com/blog/work-with-http-services-in-1c-part-1-get-method/>
- official 1C ITS HTTP-service example, which states that the handler is a
  Function accepting the single `HTTPServiceRequest` parameter:
  <https://its.1c.ru/db/content/metod8dev/src/developers/platform/metod/web-services/i8105756.htm>
- official 1C Standard Subsystems Library guide, including the BeforeWrite and
  BeforeDelete source/signature tables:
  <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_Standard_Subsystems_Library_Developer_Guide/Chapter_3._Setting_and_using_subsystems_upon_configuration_development/Chapter_3._Setting_and_using_subsystems_upon_configuration_development/>

Tracked fixtures are evidence of serialization only, not a platform-wide rule.
In particular,
`tests/fixtures/unica_mcp_script_parity/bsp/meta/CommonModules/GoogleПереводчик.xml`
proves the exact CommonModule property spellings and explicit boolean values; it
does not independently prove that that module is an EventSubscription handler.

Where official sources describe a wider platform capability than this contract,
v1 deliberately supports a smaller proven subset and classifies the rest
`Unknown`. No default value is guessed when an XML field is absent.

## 3. Mandatory Task 5A/domain back-propagation

Task 5B implementation is forbidden until the accepted Task 5A commit contains
all Task5A/domain-owned clauses in sections 3.1 through 3.10.1. Sections
3.10.2, 3.10.3 and 3.10.4 are deliberately adjacent Task5B owner bridges built
on those accepted primitives; they are not Task5A implementation/OID fields.
Section 3.11 is the separate Task4 prerequisite. The implementation order in
section 15 enforces those exact ownership cuts.

### 3.1 Typed CFE companion facts

Plain `MetadataIdentity`, plain `CfeObjectMembership`, equal UUID strings, and
declared `SourceSetKind` are insufficient to authorize a CFE ownership join.
The application/domain boundary must have two source-bound typed companions:

```text
BaseOwnedMetadataIdentityV1 {
  pair_key,
  analysis_source_set,
  registered_artifact,
  configuration_flavor = BaseConfiguration,
  object_membership = Own,
  object_uuid: PlatformUuid,
}

ExtensionMetadataMembershipV1 {
  pair_key,
  destination_source_set,
  registered_artifact,
  configuration_flavor = ExtensionConfiguration,
  membership:
    Own { wrapper_uuid }
    | Adopted { wrapper_uuid, extended_configuration_object_uuid },
}
```

Both are whole-fact smart-constructor types with stable version tags. Every
field is in the semantic payload and digest. Their constructors reject:

- an analysis semantic flavor other than `BaseConfiguration`;
- an analysis object's membership other than exact Own;
- a destination semantic flavor other than `ExtensionConfiguration`;
- a pair/source/artifact not present in the typed query;
- a missing, nil, invalid, or source-wrong UUID;
- a value assembled from records belonging to different source snapshots.

Task 5A replaces the two generic authority ProviderFact payloads with these
strict types without renumbering the outer fact registry:

```text
ProviderFact tag 10 = BaseOwnedMetadataIdentity(BaseOwnedMetadataIdentityV1)
ProviderFact tag 11 = ExtensionMetadataMembership(ExtensionMetadataMembershipV1)

Extension membership subtags: Own=1, Adopted=2
```

Tag 10 cannot carry Extension flavor or Adopted membership. Tag 11 cannot carry
Base flavor or an Absent state. Destination absence exists exactly once as
generic `ProviderFact::MetadataAbsent` tag 2 with no membership companion.
There is no `DestinationMetadataMembershipStateV1::Absent`, reserved ghost
subtag, generic constructor or decoder branch. Attempted tag-10 Adopted, tag-11
Absent, wrong-flavor or raw generic construction fails before grouping.

The application pair join accepts only these companions. Generic identity or
membership observations may remain useful elsewhere but cannot be promoted into
`DestinationMembershipPair` conclusions.

The join is exact:

```text
BaseOwned(uuid=A) + Extension.Adopted(extended_uuid=A) -> adopted/equal
BaseOwned(uuid=A) + Extension.Adopted(extended_uuid!=A) -> Unknown/mismatch
BaseOwned(uuid=A) + Extension.Own(...)                  -> Unknown/not adopted
missing or gapped companion                             -> Unknown/exact gap
```

The destination wrapper UUID is retained for provenance and never compared with
the base UUID. `configuration`/`extension` labels do not repair a contradictory
XML semantic flavor.

### 3.2 Dedicated EventSubscription requirement

The old Task 5A shape that represents an EventSubscription handler as an
`AtServer` callback is invalid. Task 5A/domain must split declarative binding
runtime context from BSL declaration context:

```text
BindingRuntimeContextV1::SameAsSourceEvent
!= BslExecutionContext
```

It must also carry the exact CommonModule capability profile, selected sources,
event/signature class, expected parameter count, handler owner/method, and
registry version from section 8. It cannot be represented by a generic callback
row with a server annotation. Its `selected_sources` is authoritative; any
separate `SubscriptionSource` projection must be derived from it and prove exact-
set equality before mechanism promotion.

The domain registry is not merely thirteen enum spellings. It contains exactly
the 13 family-to-root mappings, 21 compatible `(event, family)` rows, and three
signature classes in section 8. Signature lookup is partial and returns None for
every incompatible cell; no catch-all branch is permitted. Descriptor
construction must validate canonical set uniqueness before sorting and prove one
common signature class across the complete selected set.

Only EvidenceGraph may join that pending whole fact with a compatible Task 6
Definition. The binding does not require or synthesize an `&AtServer` directive:
the 1C platform invokes the subscription in the same context as its source
action. Until the join succeeds there is no `subscribes` runtime edge. A
missing/gapped Definition is `Unknown`; a complete exact arity/kind/export
mismatch is `No`; an otherwise matching explicit BSL compilation-context or
async variant is `Unknown`; an unsupported event/source/profile variant is
`Unknown`.

### 3.3 Dedicated ScheduledJob requirement

Task 5A/domain must separate activation from the positive binding requirement
and preserve the v6 metadata-first decision order:

```text
ScheduledJobActivationV1 {
  job,
  state = Disabled,
  exact_use_witness,
}

ScheduledJobNonPredefinedV1 {
  job,
  use_enabled = true,
  predefined = false,
  exact_use_witness,
  exact_predefined_witness,
}
```

This smart-constructor fact needs only registered job identity plus one exact
direct boolean Use singleton. `Disabled` is complete runtime-activation No and
is independent of Predefined, MethodName, module profile, and Definition.
Missing/duplicate/mixed/nonboolean Use yields no activation fact and exact
Unknown; it is never defaulted. Exact `Use=true` is a control decision, not an
independently useful activation fact or runtime-positive observation: the
provider must next resolve exact Predefined before it may request
MethodName/profile material.

The decision order is normative and short-circuiting:

```text
capture-valid registered job
  -> exact Use
     false -> Disabled; STOP positive view
     true  -> exact Predefined
              false -> NonPredefinedActivation; STOP positive view
              true  -> MethodName + CommonModule profile -> pending binding
```

`NonPredefinedActivation` is the metadata-only atomic semantic state from
section 6.1. It is
the complete v1 source conclusion
`non_predefined_scheduled_job_instance_unproven`, Unknown. MethodName, module
profile and Definition are not material, are not queried for this job, and
cannot replace that reason with a malformed/profile gap. Missing, duplicate,
mixed or nonboolean Predefined after Use=true is an exact Predefined-scoped
Unknown; MethodName/profile/Definition are likewise not opened until Predefined
is exact true. This branch is not a `ValidatedBinding`, handler candidate, or
runtime root.
The only record for that state is the dedicated
`ProviderFact::ScheduledJobNonPredefined(ScheduledJobNonPredefinedV1)`; its
constructor rejects any MethodName/profile/Definition payload. EvidenceGraph
projects it to the exact Unknown reason and never to `handles`, a Definition
requirement, or a discovery candidate. This new ProviderFact uses stable tag 13;
the existing 1..=12 tags are not renumbered.

Only a complete `Use=true + Predefined=true` descriptor can construct
`ScheduledJobBindingV1` as a pending whole fact including the exact CommonModule
capability profile. A missing/malformed MethodName or incomplete/unsupported
profile emits only its exact scoped gap: there is no partial activation
`ProviderFact`, no zero-record semantic group, and no candidate.
Its declarative runtime context is
`BindingRuntimeContextV1::Server`; that is not a requirement for a BSL
`&AtServer` annotation. A raw `MethodName` string is not a runtime edge.
EvidenceGraph joins it with a compatible exported server-callable Definition as
specified in section 9. “Server-callable” comes from the validated CommonModule
profile plus its unannotated ModuleDefault method; it does not authorize Task 5A
to accept or reject explicit per-method directives by guess.

The same split applies to the other Task 5B declarative bindings:

```text
EventSubscription -> BindingRuntimeContextV1::SameAsSourceEvent
ScheduledJob       -> BindingRuntimeContextV1::Server
HTTPService        -> BindingRuntimeContextV1::Server
FormCommand        -> BindingRuntimeContextV1::Client
```

`BslExecutionContext` remains a Task 6 observation about a declaration. It is
never populated from these declarative runtime contexts.

### 3.4 One Form call-type boundary

The semantic domain may retain `FormCallTypeV1::Direct`, but the shared lexical
constructor must take `Option<&str>` and implement section 10.4 exactly. An
existing `from_xml("Direct") -> Direct` branch is rejected. Native Form
validation and Task 5B must import that one constructor; the neutral seam makes
the same constructor available to future Task 8. No consumer may open-code the
present-attribute token list. Actual Task 8 import is downstream and non-gating
for Task 5B.

This correction is part of Task 5A acceptance because the current FormCallType
is a domain contract consumed by later providers. It must land with REDs proving
absent-to-Direct and present-literal-Direct rejection before Task 5B starts.

### 3.5 Atomic grouping identity

Task 5A/domain must leave every ProviderFact classifiable by the closed
`SemanticAtomicGroupIdV2` registry in section 6.1. In particular, existence
polarity and its role-specific CFE whole companion cannot be made unrelated by
different fact tags, and an EventSubscription descriptor cannot be separated
from its complete derived ExchangePlan uses projection. The grouping identity,
stable tags, canonical encoder, and classification invariants are internal—not a
new public wire/MCP field—but must have domain/application REDs before Task 5B
applies record limits. The shared v2 classifier/encoder is invoked inside every
Platform XML and BSL provider before its local `max_records` ceiling. Support
has no independent lossy record ceiling and passes complete facts to Task 7,
which reuses the same classifier for per-port/global admission. Classification
after any provider has already dropped individual records is forbidden.

Task 5A also changes the internal Support fact to
`ProviderFact::Support { subject, state,
authority: SupportFactAuthorityBindingV2 { semantic_authority_digest,
snapshot_authority_digest, query_digest } }`. All three digests are private
constructor results from one borrowed smart-constructed section-4.2 query entry,
not arbitrary strings. The semantic digest excludes source,
freshness, catalog/query and evidence provenance; the snapshot digest binds
source/subject/catalog/composite/freshness authority except diagnostics-only
evidence IDs; the query digest binds the exact enclosing Support invocation.
This split is required
for shared tag-9 `SupportStateObservation`; Existing and planned-destination
testimony with the same state must not collapse or substitute. It is an internal
evidence-domain change, not a public MCP wire field.

### 3.6 Closed Form-command and HTTP Definition policies

Task 5A/domain must define two pending requirements and two pure compatibility
functions rather than letting Task 5B or Task 7 treat any existing method as a
handler:

```text
FORM_COMMAND_HANDLER_POLICY = "form-command-handlers/v1"
HTTP_SERVICE_HANDLER_POLICY = "http-service-handlers/v1"

FormCommandHandlerRequirementV1 {
  policy_version,
  form,
  command,
  action_call_type,
  handler: exact own FormModule method,
}

HttpServiceHandlerRequirementV1 {
  policy_version,
  service,
  route,
  handler: exact same-service Module method,
}
```

The shared `DefinitionShape` accepted at the Task 5A boundary must already
contain `is_async: bool`; its canonical encoding, equality, digest, fixtures,
and conflict merge include that field. Task 6 later parses and supplies the
observation, but Task 5A cannot be accepted with a shape that discards it.

Their complete decision tables are sections 10.5 and 11.3. `Export` is
explicitly nonmaterial for both policies, but callable kind, exact arity,
parameter transfer/default shape, compilation context, and the policy's
supported async state remain typed Task 6 observations. The application
EvidenceGraph is the sole join owner. A route/action fact is pending declarative
evidence, not a `handles` runtime edge, until that join returns compatible.

These policy versions and every material decision input participate in the
whole-fact digest and Task 7 analysis ID. A guessed `Procedure` of arbitrary
arity/context for Form commands, or a guessed one-argument HTTP `Function`
without checking the closed table, is a Task 5A acceptance failure.

### 3.7 Preflight and STOP gate

For `unica.cfe.patch_method`, the captured analysis source kind must still be
exactly `configuration`. An extension analysis is proposal-level
`cfe_analysis_configuration_required`, `Unknown`, receipt-ineligible, and absent
from every provider plan/presence key/pair. All-invalid requests return before
provider and receipt calls; mixed requests collect only unblocked proposals.

After provider parsing, an XML flavor/membership contradiction is not the same
preflight error. It is a source-bound exact provider gap such as
`analysis_not_base_owned` or `destination_not_extension_flavor`, and the join is
`Unknown`.

### 3.8 One exact source-fingerprint type

Task 5A/domain must replace discovery-internal source-fingerprint `String`
authority with one shared smart newtype:

```rust
pub(crate) struct SourceFingerprintV1([u8; 32]);
pub(crate) struct SnapshotLeafFingerprintV1([u8; 32]);
pub(crate) struct PlatformConfigurationCaptureCatalogDigestV1([u8; 32]);

impl SourceFingerprintV1 {
    pub(crate) fn parse_exact(value: &str) -> Result<Self, SourceFingerprintError>;
    pub(crate) fn fingerprint32(&self) -> &[u8; 32];
    pub(crate) fn as_transport(&self) -> String; // exact lowercase sha256:<64 hex>
}
```

`parse_exact` accepts exactly 71 ASCII bytes: lowercase `sha256:` followed by
64 lowercase ASCII hexadecimal characters. It rejects uppercase prefix/hex,
leading/trailing whitespace, Unicode lookalikes, missing/extra characters and
all other algorithms. The constructor is private; there is no unchecked
`From<String>`, public field, permissive `Deserialize`, or caller-supplied raw
digest constructor. `fingerprint32` is the sole byte authority used by every
`fingerprint32(...)` encoder below; `as_transport` renders the unique canonical
transport spelling from those bytes. It is not `Digest32`, because its type
records that the transport prefix was validated at the snapshot boundary.

Task 4 snapshot ingress converts its current transport string once. Catalog,
Metadata/Form/BSL/Support query types, execution context, physical evidence and
planned-destination authority borrow or own this exact newtype rather than
reparse arbitrary strings. Composite snapshot IDs remain their separately
versioned identity type. Mandatory Task 5A REDs cover 64 lowercase hex,
uppercase prefix/hex, 63/65 hex, whitespace, non-ASCII, a forged serde/field
construction compile failure, and byte-for-byte agreement between
`fingerprint32` and `as_transport`. A repository static test rejects any new
discovery field named `source_fingerprint: String`.

`SnapshotLeafFingerprintV1` has the same exact lowercase `sha256:<64 hex>`
transport/byte grammar and private parse/access/render shape, but is a distinct
type owned at Task 4 manifest-leaf ingress. It denotes one captured material's
content bytes, not the aggregate source/manifest fingerprint. No
`From<SourceFingerprintV1>`/cross-type equality/raw digest constructor exists.
Registered Form sidecars and parser handles use this leaf type consistently;
compile-fail and mutation REDs prove source and leaf fingerprints cannot be
swapped even when their 32 bytes happen to match.

`PlatformConfigurationCaptureCatalogDigestV1` has that same exact private
parse/borrow/render grammar but denotes the section-5 domain hash over the
capture-admitted configuration-material subset. It is neither aggregate source
freshness nor one leaf. `PlatformConfigurationCatalogV1.capture_catalog_digest`
and its witness carrier use this exact type. There are no cross-type
conversions/equality/serde/raw constructors among the three types; compile-fail
tests reject source/leaf/capture swaps even when all 32 bytes are equal, while
encoder goldens prove each typed value contributes the same canonical raw bytes
at its designated field.

### 3.9 Source-qualified evidence and gap identity

Task 5A's current `SourceScopedArtifact { source_set: String }` and
`ProviderGapScope::SourceSetWide(String)` are not acceptable authorities: equal
display names can denote different roots, formats, kinds or mapping digests.
The internal evidence/query/gap boundary must instead be:

```rust
pub(crate) struct SourceScopedArtifact {
    source: AtomicSourceIdentityV2,
    artifact: ArtifactRef,
}

pub(crate) enum ProviderGapScope {
    Artifacts(Vec<SourceScopedArtifact>), // nonempty, canonical unique
    QueryWide,
    SourceSetWide(AtomicSourceIdentityV2),
}
```

Fields and constructors are private and validate that the source is one exact
captured logical source. Public wire rendering may project the display name as
a separate diagnostic field; it never reconstructs internal identity from that
name. The complete `AtomicSourceIdentityV2` is encoded and compared at every
materiality boundary. The reachable end-to-end RED uses analysis and
destination with an equal display name but different role/kind/root/mapping and
proves distinct gap/query/group identity. A pure encoder test may construct two
destination identities to prove total identity behavior, but current
`ResolvedSourceSelection` rejects conflicting mutation names and this contract
does not broaden that capture rule. Case-equivalent artifacts within one source
remain equal.

`Artifacts` contains only actual `SourceScopedArtifact` values. A
`DestinationMembershipPair` material key expands to its exact analysis and
destination `SourceScopedArtifact` halves; the pair key itself is not inserted
into `Artifacts`. A callback slot is represented by its exact owner and method
artifacts plus the typed reason/group identity, never by a fake artifact.
Request/Proposal/Mechanism associations exist only in Task 7's application map
and never enter a provider gap. The closed `gap_artifact_projection_v2` sorts and
deduplicates expanded halves, rejects an empty result, and is used before the
2,000-artifact bound. A static/exhaustive RED rejects any non-artifact value in
`ProviderGapScope::Artifacts` and any remaining discovery-internal
`source_set: String`/`SourceSetWide(String)`.

When Task7 later converts an exact Artifacts gap into an admission/effective
scope, it calls only `ProviderGapScope::project_material_artifact_set_v2` from
section 3.10.4. That owner method revalidates the complete artifact-only
identity and returns an opaque set; Task7 does not copy/walk this vector or
re-expand a pair. QueryWide/SourceSetWide do not return an artifact set.

### 3.10 One opaque artifact-identity byte type

`ArtifactIdentityBytesV1` is a Task 5A/domain smart type, not merely notation
for bytes that each consumer may rebuild:

```rust
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ArtifactIdentityBytesV1 {
    bytes: Box<[u8]>, // private
}

pub(crate) const ARTIFACT_IDENTITY_UNICODE_VERSION: (u8, u8, u8) =
    (17, 0, 0);

const _: () = {
    assert!(std::char::UNICODE_VERSION.0 == ARTIFACT_IDENTITY_UNICODE_VERSION.0);
    assert!(std::char::UNICODE_VERSION.1 == ARTIFACT_IDENTITY_UNICODE_VERSION.1);
    assert!(std::char::UNICODE_VERSION.2 == ARTIFACT_IDENTITY_UNICODE_VERSION.2);
};

impl ArtifactIdentityBytesV1 {
    pub(crate) fn try_from_artifact(
        artifact: &ArtifactRef,
    ) -> Result<Self, ArtifactIdentityError>;

    pub(crate) fn as_bytes(&self) -> &[u8];
}
```

`try_from_artifact` is the sole constructor. It first calls the full
`ArtifactRef::validate()` even though its argument is typed: live
`ArtifactRef` fields are crate-visible and an in-crate struct literal must not
bypass canonical-ref validation. Only after successful validation does it emit
exactly `u16be(kind.stable_tag()) || string(UnicodeLowercase(canonical_ref))`
from section 6.2. Unknown kind tags, a forged invalid ref, length overflow, or
any validation failure returns the closed typed error and emits no partial
identity. There is no second post-lowercase 1,024-byte restriction: the
validated input boundary is authoritative and the encoder's u32 byte length
describes the exact possibly expanding lowercase output.

`char::to_lowercase` is permitted in this identity constructor only behind the
compile-time component gate above. A Rust toolchain whose
`std::char::UNICODE_VERSION` is not exactly `(17, 0, 0)` cannot build the
product; changing the constant requires an encoder-version review and new
identity/query/group/record goldens. This identity-lowercase version pin is
separate from Task6's frozen Unicode 16.0 grammar-category tables: neither
version silently upgrades, derives, or substitutes for the other, and the pin
changes no encoder bytes while the asserted standard-library version matches.

The type has no public field, unchecked `From<Vec<u8>>`/`From<String>`, raw-byte
constructor, `Serialize`/`Deserialize`, display parser or caller-selected
normalization. `as_bytes` is read-only. Its `Eq`, `Ord` and `Hash` operate on
the complete canonical byte slice, so stable sorting and set membership cannot
fall back to `ArtifactRef::identity_key()`, exact spelling, Debug text, or a
consumer-local tuple. Every query/catalog/group/record/material/Form-lookup
consumer constructs this type at its validated ingress and then borrows it;
Task 5B and future consumers may not declare a local duplicate.

Mandatory Task 5A REDs cover case-equivalent refs, the expanding `İ` golden,
different kind tags, canonical sort/hash/set behavior, the 128/129 identifier-
scalar boundary, and an in-crate forged invalid `ArtifactRef` rejected by
`try_from_artifact`. A product compile fixture asserts all three
`std::char::UNICODE_VERSION == (17,0,0)` components; a fixture with any one
expected component mutated must fail at compile time before an identity is
constructed. Compile-fail/static tests prove private bytes cannot be
forged/deserialized and reject a second identity encoder or raw-byte
constructor. The exact section-6.2 bytes and SHA goldens are recomputed through
this API, not a test-only helper.

The Task 5B delivery worker records `TASK5A_ACCEPTED_SHA` only in the successor
implementation report/ledger and verifies sections 3.1 through the base
section 3.10 plus the shared registry type/API named in section 3.10.1 against
code/tests/spec; sections 3.10.2-.4 remain its own later slice. Any
difference is a STOP; infrastructure must not compensate locally or edit the
frozen design.

### 3.10.1 Catalog/provider-local fail-closed exact-spelling validation

Before `PlatformCatalogPort` canonicalizes either catalog set, and before either
Task5B evidence provider classifies, sorts, deduplicates, groups or applies any
record/gap/subject ceiling, the owning builder imports and uses the one
Task5A/domain-owned validation primitive required by Task7 section 0.1
verbatim:

```rust
impl ExactArtifactSpellingRegistryV1 {
    pub(crate) fn empty_v1() -> Self;

    pub(crate) fn validate_occurrence(
        &mut self,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn require_occurrence(
        &self,
        source: &AtomicSourceIdentityV2,
        artifact: &ArtifactRef,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

The registry has private fields and no first-wins, lexical-minimum, raw-key or
caller-selected normalization API. `validate_occurrence` accepts one typed
`ArtifactRef` and internally derives both `ArtifactIdentityBytesV1` and the
exact `u16` kind-tag/canonical-ref value; callers cannot supply those fields
separately. Its key is the complete exact
`AtomicSourceIdentityV2` plus `ArtifactIdentityBytesV1`; its value is the exact
`(kind stable tag, canonical-ref UTF-8 bytes)` spelling. `PlatformCatalogPort`
creates one fresh registry for the complete composite build and visits every
raw typed artifact occurrence from Configuration registration, both catalog
sets, registered Form rows and captured Analysis BSL module projections before
their first semantic-key sort, set insertion or deduplication. Existing exact
and canonical duplicate capture failures remain fatal; the registry cannot
turn either into a retained spelling.

A catalog-build-local `ExactArtifactSpellingViolationV1` maps exactly to the
existing nonretryable `PlatformXmlParserInvariant` /
`platform_xml_parser_invariant` build error with no partial context. Task 4
capture and the shared registration grammar should already have rejected every
valid-input alias collision; reaching the shared registry therefore proves a
capture/catalog projection disagreement, not external drift or a resource
limit. No fifth build-error variant is introduced. The later Task7
request-versus-valid-context collision is an application error and never
re-enters `PlatformCatalogBuildErrorV1`.

Each Task5B evidence-provider invocation independently creates one fresh local
registry through `empty_v1` and exhaustively walks the staged, still-unsorted
typed decode: every query/catalog-derived artifact, every record artifact,
every raw gap artifact,
every source-qualified pair half, every binding/descriptor/observation field,
and every nested material subject. When a raw semantic group is assembled, all
query-derived or auxiliary artifacts that did not occur in a decoded record or
gap are validated through that same registry before the first group-key
comparison, set insertion, canonical sort or deduplication. Each call supplies
the occurrence's own typed source identity; an outer provider name, display
source name or artifact text may never infer it.

Two occurrences with the same source and semantic identity but different exact
spellings reject the entire staged provider result with nonretryable stable
reason `exact_artifact_spelling_collision`; no partial group, retained prefix,
raw outcome or cache value survives. Reversing record, gap, group or nested-
material input order produces the same rejection. A single isolated spelling,
including either case/Unicode variant by itself, remains valid; substituting
the other isolated variant preserves the existing semantic identity, query,
group and raw-outcome bytes. Task7 later applies the same primitive to its one
execution-wide registry to catch request/catalog, cache and cross-provider
collisions. This provider-local use adds no spelling to a query or provider
identity, changes no valid query/catalog/group golden, and creates no new
Task5A implementation-OID field or production-DAG edge beyond the already
required `TASK5A_ACCEPTED_SHA`.

After a valid catalog build, the context retains one closed zero-I/O projection
over every accepted artifact occurrence that it or any of its catalog/material
views can later expose. It includes every Configuration-catalog artifact,
registered owner/Form/nested semantic artifact and every captured Analysis BSL
module projection, with each occurrence's exact `AtomicSourceIdentityV2`; it
contains no manifest key, path, witness, reader, material handle or display
source name. The context alone may stage this private projection against an
application baseline:

```rust
impl PlatformCatalogContextV1 {
    pub(crate) fn stage_complete_catalog_spellings_v1(
        &self,
        baseline: &ExactArtifactSpellingRegistryV1,
    ) -> Result<StagedExactArtifactSpellingDeltaV1,
                ExactArtifactSpellingViolationV1>;
}
```

The method calls only
`baseline.stage_occurrences_v1(complete_private_occurrences)` and performs no
filesystem/read/parse operation. It returns one opaque delta and no iterator,
artifact vector or callback. Its occurrence set is complete by construction:
deleting any retained catalog/material artifact, adding a foreign occurrence,
substituting a source, or exposing a path/handle is a permanent RED. Task7 uses
this sole gate once to compare the request baseline with the complete context;
it never walks private catalog fields.

### 3.10.2 Sealed typed-query spelling recheck

The three Task5B-owned artifact-bearing smart queries expose one identical
crate-private, zero-I/O validation method:

```rust
impl MetadataCompositeQueryV2<'_> {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}

impl FormSourceSetQueryV2<'_> {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}

impl SupportStateQueryV2<'_> {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

Each owner method exhaustively visits its private canonical members with their
already bound `AtomicSourceIdentityV2` and calls only
`registry.require_occurrence(source, artifact)`. Metadata includes both halves
of every destination pair, every presence key and every artifact nested in a
Form material scope; Form includes every nested material-scope artifact;
Support includes every grouped subject and nested source-qualified artifact.
The method returns no iterator, member, callback, delta or raw field and cannot
add a missing spelling. Task7 calls it before allocating an invocation slot;
failure invalidates that execution with no I/O. Permanent omission/mutation
REDs prove every nested member is covered and that no query digest/golden byte
changes.

The same three smart queries also mint their own sealed, owned, non-Clone,
non-serde association authority; Task7 never derives membership from a digest.
The authority API uses the shared Task5A/domain-owned closed
`ProviderQueryAssociationViolationV1`; that error is non-serde and carries no
query member, display name or caller-supplied reason. Its exhaustive internal
variants are `SourceGroupNotMember=1`, `MaterialNotMember=2` and
`PlatformCatalogExecutionMismatch=3`; the tags are for closed matching only and
never serialize or enter identity:

```rust
impl MetadataCompositeQueryV2<'_> {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> MetadataQueryAssociationAuthorityV1;
}
impl FormSourceSetQueryV2<'_> {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> FormQueryAssociationAuthorityV1;
}
impl SupportStateQueryV2<'_> {
    pub(crate) fn association_authority_v1(
        &self,
    ) -> SupportQueryAssociationAuthorityV1;
}
```

The three capability types are `pub(crate)` solely so the Task7 application
boundary can own them; every field remains module-private. Each has exactly the
same crate-private borrowed API (shown once with `A` standing for that concrete
owner type):

```rust
impl A {
    pub(crate) fn query_digest(&self) -> Digest32;
    pub(crate) fn validate_platform_catalog_execution_v1(
        &self,
        context: &PlatformCatalogContextV1,
        snapshot: &SourceSnapshotV2,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
    pub(crate) fn validate_source_group_v1(
        &self,
        source: &AtomicSourceIdentityV2,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
    pub(crate) fn validate_material_v1(
        &self,
        material: &ProviderGroupMaterialIdentityV2,
    ) -> Result<(), ProviderQueryAssociationViolationV1>;
}
```

There is no `pub(crate)` field, constructor, raw membership projection or
Task5B infrastructure definition of these application/query-owner types.
Their private layout invariant nevertheless includes the same typed field in
all three owners:

```text
MetadataQueryAssociationAuthorityV1::execution_binding:
  PlatformCatalogExecutionBindingV1
FormQueryAssociationAuthorityV1::execution_binding:
  PlatformCatalogExecutionBindingV1
SupportQueryAssociationAuthorityV1::execution_binding:
  PlatformCatalogExecutionBindingV1
```

Each returned owner type has private fields and no raw constructor, Clone,
serde or member iterator. It owns the exact query digest, one exact
`PlatformCatalogExecutionBindingV1`, plus the complete
canonical source-group and `ProviderGroupMaterialIdentityV2` membership needed
as sorted unique typed values, not `digest+count`, paths, witnesses or a
provider-read material cohort. Source freshness/read completeness remain the
separate snapshot and owner response-validation contracts. Membership checks
use the shared typed values' Eq/Ord boundary without Task7 walking a private
variant. For association validation it exposes only read-only `query_digest()`,
`validate_platform_catalog_execution_v1(&PlatformCatalogContextV1,
&SourceSnapshotV2)`,
`validate_source_group_v1(&AtomicSourceIdentityV2)` and
`validate_material_v1(&ProviderGroupMaterialIdentityV2)`, with both validators
and the execution validator returning
`Result<(), ProviderQueryAssociationViolationV1>`. The execution validator
delegates only to its owned binding's sealed context+snapshot validator; it
returns no binding, bytes, digest or context member. Metadata covers
every composite source group and every exact query-time pair-half, presence-key
or requested Form material root; Form covers the exact Analysis group and every
exact requested/effective Form material root; Support covers every canonical
source group and grouped query-subject material root. These are exactly the
pre-I/O legal roots. A provider-returned gap/group may add a post-I/O material
root only when Task7's finished handle validates it against the recorded typed
outcome as well; the query authority alone never blesses a value merely because
it shares a digest or display spelling.

Unknown, omitted or foreign values reject through the shared closed violation.
Task7 can only wrap the exact typed owner value in its own closed enum, compare
its digest and enum-implied port to the accepted typed query, require the owned
execution binding against the one `EvidenceExecutionContext` context+snapshot,
and move the same
non-cloneable value from registered plan to finished registry entry. The
authority object, private membership, validation result and any second binding
frame never enter query/raw provider identity, cache keys, group bytes, receipts
or serialization. The binding's three semantic fields occur once only in the
pre-existing analysis-execution header through its sealed writer. Adding a
private query member without extending both spelling and association-authority
projections is a compile/static RED; all query bytes/digests stay unchanged.
Mutation REDs reject a missing member, a foreign source/material, swapped owner
authority, context/snapshot/catalog binding mismatch, digest/port mismatch,
Clone/serde/raw-constructor exposure and any
Task7 path that decides membership from digest equality alone.

### 3.10.3 Sealed association/response-material spelling recheck

The two Task5B-owned material owner types imported by Task7's association layer expose
the same crate-private, zero-I/O read-only check:

```rust
impl ProviderGroupMaterialIdentityV2 {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn validate_staged_artifact_spellings_v1(
        &self,
        baseline: &ExactArtifactSpellingRegistryV1,
        delta: &StagedExactArtifactSpellingDeltaV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}

impl SemanticAtomicEvidenceGroupV2 {
    pub(crate) fn validate_committed_artifact_spellings_v1(
        &self,
        registry: &ExactArtifactSpellingRegistryV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;

    pub(crate) fn validate_staged_artifact_spellings_v1(
        &self,
        baseline: &ExactArtifactSpellingRegistryV1,
        delta: &StagedExactArtifactSpellingDeltaV1,
    ) -> Result<(), ExactArtifactSpellingViolationV1>;
}
```

Each method exhaustively destructures its closed private variant without `..`
and calls only `registry.require_occurrence` in the committed form or
`baseline.require_staged_occurrence_v1(delta, ...)` in the pre-commit response
form for every artifact-bearing member with that member's already bound exact
`AtomicSourceIdentityV2`.
`ProviderGroupMaterialIdentityV2` covers the source-scoped artifact or both
analysis/destination halves of a membership pair.
`SemanticAtomicEvidenceGroupV2` is the lossless private owner emitted by the
shared section-6.1 classifier before any record ceiling. It owns one closed
`SemanticAtomicEvidenceGroupKindV2` nine-variant payload containing the
canonical `SemanticAtomicGroupIdV2`, all
complete physical records and every frozen typed material authority needed by
`material_subjects(group)`, including material that is intentionally absent
from the group ID or an emitted record. The ID is only its canonical identity
projection; an ID, secondary digest, record slice or caller-built artifact
cohort cannot construct or stand in for this owner. The owner walk covers the
primary source-qualified subject and every additional typed artifact occurrence
retained by its StandaloneFact, CFE pair, descriptor, effective Form scope,
ScheduledJob, callback, Definition or Support variant. Relations,
callback-slot tags, semantic digests and evidence IDs contain no artifact and
are excluded deliberately.

The methods expose no iterator, callback, raw member, occurrence list, returned
delta or mutation capability. The staged overload only borrows the opaque
delta and exact baseline; it cannot consume, alter or commit either. Missing,
exact-different or foreign-baseline membership rejects. A compile-fail/static
RED adds one artifact-bearing field or variant
member without extending the corresponding exhaustive destructure; mutation
REDs replace every direct and nested artifact independently and require
rejection. Isolated valid spellings preserve all existing identity/group/query
bytes and goldens. Task7's closed association contribution calls only these
owner methods; it never walks Task5B-private material/group fields or trusts a
caller-supplied artifact list.

### 3.10.4 Sealed material-set and catalog-execution bridges

Task 7 must encode complete admission material and bind every typed provider
association authority to the exact catalog execution without opening Task5B's
private groups, catalog sets or composite-ID representation. Two immutable
owner types close those boundaries. They are material/execution **identity**,
not query membership authority.

The required downstream Task7 storage shape is exact:

```text
EvidenceAdmissionScopeV3
  Artifacts(ProviderMaterialArtifactSetV2) tag 1
  SourceSetWide(AtomicSourceIdentityV2)    tag 2
  QueryWide                               tag 3
```

The raw upstream `ProviderGapScope::Artifacts(Vec<SourceScopedArtifact>)`
remains provider-owned input/history. It is not reused as the application
admission DTO and Task7 may not recreate the opaque set from a caller vector.

```rust
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProviderMaterialArtifactSetV2 {
    // private nonempty, strictly increasing by complete
    // SourceScopedArtifactIdentityBytesV2
    artifacts: Box<[SourceScopedArtifact]>,
}

pub(crate) enum ProviderMaterialArtifactSetErrorV2 {
    Empty                 = 1,
    InvalidArtifact      = 2,
    InvalidPairExpansion = 3,
    CardinalityOverflow  = 4,
}

impl SemanticAtomicEvidenceGroupV2 {
    pub(crate) fn project_material_artifact_set_for_groups_v2(
        groups: &[&SemanticAtomicEvidenceGroupV2],
    ) -> Result<ProviderMaterialArtifactSetV2,
                ProviderMaterialArtifactSetErrorV2>;
}

impl ProviderGapScope {
    pub(crate) fn project_material_artifact_set_v2(
        &self,
    ) -> Result<Option<ProviderMaterialArtifactSetV2>,
                ProviderMaterialArtifactSetErrorV2>;
}

impl ProviderMaterialArtifactSetV2 {
    // Counts the distinct complete artifact identities in the union. Empty
    // input is zero; no member, bytes, token or iterator is returned.
    pub(crate) fn canonical_union_cardinality_v2(
        sets: &[&ProviderMaterialArtifactSetV2],
    ) -> Result<u32, ProviderMaterialArtifactSetErrorV2>;

    // Appends exactly vec(SourceScopedArtifactIdentityBytesV2), including its
    // u32 count, and returns no bytes/member/callback.
    pub(crate) fn write_identity_v2(
        &self,
        out: &mut Vec<u8>,
    );
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PlatformCatalogExecutionBindingV1 {
    composite_snapshot_id: CompositeSnapshotIdV2,
    configuration_catalog_set_digest: Digest32,
    registered_form_catalog_set_digest: Digest32,
}

pub(crate) struct PlatformCatalogExecutionBindingMismatchV1 {
    _private: (),
}

impl PlatformCatalogContextV1 {
    pub(crate) fn execution_binding_v1(
        &self,
    ) -> PlatformCatalogExecutionBindingV1;
}

impl PlatformCatalogExecutionBindingV1 {
    pub(crate) fn validate_context_and_snapshot_v1(
        &self,
        context: &PlatformCatalogContextV1,
        snapshot: &SourceSnapshotV2,
    ) -> Result<(), PlatformCatalogExecutionBindingMismatchV1>;

    // Exact 96 bytes: raw composite digest, configuration-set digest,
    // registered-Form-set digest. No tag, length, domain or returned buffer.
    pub(crate) fn write_identity_v1(&self, out: &mut Vec<u8>);
}
```

`ProviderMaterialArtifactSetV2` has private fields and no raw/vector/slice
constructor, `From`, serde, iterator, instance length/member getter, callback, `contains`,
source-group validator, material validator, query digest, association-authority
conversion or mutable union operation. It is an immutable `Clone + Eq` value
only so one validated admission scope can be copied into each exact owning
invocation and revalidated at finish; cloning cannot reveal or add a member.
Both owner projections validate each `SourceScopedArtifact` through
`ArtifactIdentityBytesV1`, collect the complete typed occurrences, sort by
`SourceScopedArtifactIdentityBytesV2`, reject an empty result and collapse only
byte-equal identity duplicates. Same-source semantic-equal/exact-different
spellings have already failed the shared spelling registry and cannot select a
winner here. `ProviderMaterialArtifactSetErrorV2` is private/non-serde; its
numeric tags are closed matching aids only and enter no identity/output. Task7
maps any reachable projection failure to its existing nonretryable provider-
outcome/association contract mismatch before admission mutation, never to a
provider gap or a partial set. `CardinalityOverflow` is reserved only for the
checked aggregate below and maps to a nonretryable registry-finish contract
mismatch before sentinel selection or finished state; no partial count escapes.

`project_material_artifact_set_for_groups_v2` rejects an empty group slice,
then exhaustively destructures every one of the nine closed private
`SemanticAtomicEvidenceGroupKindV2` payload variants without `..`. The full
owner is the only constructive input: `SemanticAtomicGroupIdV2` alone cannot reveal the
nested records, effective Form scope or dependent pairs and is therefore never
accepted by this projection. Its artifact walk is exactly the complete owner
walk used by section 3.10.3 and the section-6.1
`material_subjects(group)` algorithm; it collects those same retained real
artifact occurrences instead of returning only spelling-validation state:

| Group tag | Required real source-qualified artifacts |
| ---: | --- |
| 1 StandaloneFact | primary subject plus every other explicitly artifact-bearing retained member; relation/value tags themselves are not artifacts |
| 2 CfePairHalf | primary half plus both real halves of every dependent `DestinationMembershipPair` |
| 3 EventSubscriptionDescriptor | subscription plus every retained selected-source, handler and ExchangePlan-use artifact |
| 4 FormCommandEvidenceCluster | Form plus every command/runtime artifact and both halves of every effective-scope pair |
| 5 ScheduledJobCluster | job plus every retained enabled-descriptor module/handler artifact; disabled/nonpredefined states retain only their real job material |
| 6 HttpServiceDescriptor | service plus every retained route/handler artifact |
| 7 PlatformCallbackRequirement | owner plus every retained concrete callback target/method artifact; the callback-slot tag itself is not an artifact |
| 8 DefinitionObservationCluster | the exact queried Method |
| 9 SupportStateObservation | exact source-qualified subject only; semantic/snapshot/query authority and any planned pair are validation authority, not this group's material |

Every `DestinationMembershipPair` is expanded before sorting to its exact
Analysis and Destination `SourceScopedArtifact` halves. The pair value/tag is
never a set member, and pair halves cannot be inferred from a display name.
`ProviderGapScope::Artifacts` uses its complete already validated real-artifact
vector and returns `Some`; `QueryWide` and `SourceSetWide` return `None` and
cannot fabricate an artifact set. The provider-artifact path rejects empty,
fake proposal/mechanism/callback-slot values and any non-artifact pair token.

`canonical_union_cardinality_v2` accepts only borrowed opaque sets, treats an
empty input slice as cardinality zero, and counts each distinct complete
`SourceScopedArtifactIdentityBytesV2` once across the canonical union. Repeated
set references, overlapping sets and input permutations produce the same
count. The implementation performs checked `u32` accumulation and returns only
that scalar; it has no threshold/limit argument and returns no member, bytes,
identity token, iterator, callback, temporary union or per-set view. In
particular it does not decide whether 2,000 or any other bound is exceeded.
Task7 alone compares the returned count with its application finality threshold.
This sealed aggregate is not source/query/outcome membership authority and
cannot be used to add, remove or reconstruct an artifact set.

The projection call-site whitelist is exact. Both the group-slice and provider-
gap projections may be called in production only by Task 7's private pure
`ports.rs::derive_application_admission_v3` and the private recheck inside
`ports.rs::RegistryFinishValidationV4` / `finish_execution_v4`.
`derive_application_admission_v3` is the one shared derivation reused unchanged
by preview and prepare; neither orchestration nor a recorded handle calls either
projection or supplies a member vector. The finish recheck reruns that same
derivation against registry-owned current state and compares the opaque set by
typed equality. `write_identity_v2` may be called only by Task 7's private
`determinism.rs::CanonicalIdentitySinkV4::append_provider_material_artifact_set_v2`
adapter. That adapter owns Task7's private `Vec<u8>` and is callable only by the
single sealed `EvidenceAdmissionScopeV3` identity encoder after it has written
the Artifacts tag. It passes its hidden buffer directly to the upstream writer;
no Task7 sibling receives `&mut Vec<u8>`, bytes, length or a slice. The writer
adds the canonical vector with no extra `bytes(...)` frame. Provider raw-gap
encoding continues to use the upstream `ProviderGapScope` owner encoder and
cannot call this admission writer or adapter. Tests inside the Task5B owner
module may call the projections/writer only for exhaustive/mutation/mechanical
goldens. Any other production caller, re-export, alias or function-pointer
capture fails the static whitelist. Task5B imports no Task7 sink type; this is
an external call-site contract, not a reverse dependency.

The aggregate caller whitelist is separate and exact. The only production
caller of `ProviderMaterialArtifactSetV2::canonical_union_cardinality_v2` is
Task7's private
`ports.rs::RegistryFinishValidationV4::count_effective_gap_material_subjects_v4`.
That Task7 method owns the selected registry-finality set cohort and the later
comparison with 2,000; it receives no members from Task5B. Owner-module tests
are the only additional callers and cover empty=0, singleton, disjoint,
overlapping, duplicate-reference, permutation and checked-u32-overflow cases.
Any alias, re-export, callback/function-pointer capture, second production
caller, threshold parameter or bool-over-limit convenience method fails the
static API/call-site guard.

The set deliberately cannot prove that a group belongs to the current Task7
execution: `SemanticAtomicEvidenceGroupV2` contains provider semantic evidence,
not a Task7 registry brand. Before either whitelisted projection path, the
registry-owned pure derivation/recheck validates every group/root or provider gap and passes only
that current cohort. Treating this material set as execution, query or
outcome membership authority is forbidden; its inability to do so is why no
`contains`/validator exists. After that Task7 gate, equal semantic-group inputs
from distinct valid invocation roots are legal material contributions and
collapse only through the final sorted-unique artifact identity; this set does
not decide whether either root is duplicate or required.

`PlatformCatalogExecutionBindingV1` is constructed only from one valid whole
`PlatformCatalogContextV1`; there is no three-field/raw-digest/string/transport
constructor, serde, field accessor, digest getter or conversion from an
analysis-only catalog view. Construction clones the exact typed composite ID
and the two derived set digests from that same context. Its derived
`PartialEq`/`Eq` compares all three typed fields and nothing else; there is no
caller-selected comparator. The context+snapshot validator
compares each owned typed field directly with the supplied whole context, then
requires that context and snapshot have the same exact
`CompositeSnapshotIdV2`, Analysis source and canonical Destination set. A wrong
context, equal catalogs under a different composite, either set-digest swap,
snapshot replay or half-context returns the opaque non-serde
`PlatformCatalogExecutionBindingMismatchV1`; it never returns a mismatching
field or byte. The three association-authority owner methods map that opaque
mismatch only to
`ProviderQueryAssociationViolationV1::PlatformCatalogExecutionMismatch`; no
new public/provider stable reason or retryable path is introduced.

`write_identity_v1` appends exactly 96 bytes in this order:

```text
CompositeSnapshotIdV2 raw digest                    32 bytes
configuration_catalog_set_digest raw digest         32 bytes
registered_form_catalog_set_digest raw digest        32 bytes
```

The first field delegates only to Task 4's sealed
`CompositeSnapshotIdV2::write_platform_catalog_execution_digest_v1`; the latter
two delegate to the one shared checked `digest32` writer. There is no schema
tag, vector/bytes frame, transport prefix, secondary hash or diagnostics epoch.
The only production caller of this 96-byte writer is Task 7's private
`determinism.rs::CanonicalIdentitySinkV4::append_platform_binding_v1` adapter.
That adapter owns Task7's private `Vec<u8>`, passes it directly to this upstream
writer and exposes no buffer/bytes/length/slice to a sibling. The sealed
`AnalysisIdentityPrefixV4` identity encoder is the adapter's only production
caller; it writes the binding once at the exact former three-field position,
byte-for-byte, so no v4 execution bytes or digest change. Task5B mechanical
tests are the only additional direct writer callers. Task 7 receives no raw
component accessor and cannot invoke Task 4's writer directly. Task5B imports
no Task7 sink type; the fully qualified adapter is only a static call-site
whitelist entry.

Accordingly Task7's successor `AnalysisIdentityPrefixV4` owns the opaque
`platform_catalog_execution: PlatformCatalogExecutionBindingV1` instead of
three independently supplied composite/configuration-set/registered-Form-set
fields. `FinishedAnalysisExecutionProjectionV4` carries that sealed prefix and
`AnalysisExecutionSnapshotV4` accepts only the complete finished projection.
The registry-construction path obtains the binding once from the same whole
context+snapshot, validates it, stores it in `AnalysisIdentityPrefixV4`, and the
finalization path carries that registry-owned prefix after every association
authority has passed current context+snapshot validation; no snapshot/prefix
constructor has an independent binding or three-field argument and no second
caller-selected binding exists. Public/
report projections may render their existing fields only through their already
typed owners; they never open this binding.

The `execution_binding_v1` construction whitelist contains exactly the three
Metadata/Form/Support smart-query constructors, the three Task6
CodeSearch/Definition/CallGraph whole-context smart-query constructors and Task
7's one `with_provider_invocation_registry_v2` private registry-construction
path. The three Task6 additions each call the context projection exactly once,
store the opaque binding outside every query/cache/group/raw-outcome identity,
and mint an owner authority that can only compare full typed binding equality;
they receive no component accessor and Task5B imports no Task6 type. This owner
whitelist addition and the Task6-v7 authority contract must co-freeze in the
same four-document package tuple. The Task7 path obtains and validates that
opaque binding once, places it in the sealed
`AnalysisIdentityPrefixV4`, and the finished-execution projection carries it; no
orchestration DTO reconstructs the three fields at finality or snapshot time.
Each
`MetadataQueryAssociationAuthorityV1`, `FormQueryAssociationAuthorityV1` and
`SupportQueryAssociationAuthorityV1` owns one binding minted from its query's
private context borrow. The query smart constructor stores that binding;
`association_authority_v1` only clones the already validated opaque value while
moving the otherwise non-Clone authority, and never calls the context
constructor again. Their
`validate_platform_catalog_execution_v1(context, snapshot)` methods are the
only production callers of `validate_context_and_snapshot_v1`; Task 7 invokes
that owner method before allocating an invocation slot and again during final
registry validation. No binding object or extra frame participates in a query,
provider-outcome, atomic-group, cache or association-map identity. Its three
components are written once in their unchanged analysis-execution-header
position and nowhere else. The
binding closes authority provenance; it does not authorize a source group or
material member and cannot replace either owner membership validator.

Permanent REDs freeze the 96-byte zero/one-bit mutation matrix, equality under
an allocation move, inequality for each of the three fields, context/snapshot
cross-product rejection, exact caller whitelists, no identity drift, all nine
group rows above, every direct/nested artifact mutation, CFE/Form material-pair
half expansion, Support subject-only material, canonical permutation/duplicate
behavior, empty rejection and compile failure for a tenth artifact-bearing
group until both the spelling and material-set exhaustive matches are extended.
Aggregate REDs freeze empty-slice zero, singleton/disjoint/overlap/duplicate-set
and permutation cardinalities, checked u32 overflow, scalar-only return, no
threshold parameter/decision and the sole Task7 finish-validation caller.
Binding REDs additionally freeze the exact three Task6 whole-context projection
callers, wrong composite/either-catalog binding rejection in their owner
authorities, and unchanged six Task6 v3 query bytes/digests.

The downstream whitelist is phased without weakening it. At Task5B production
acceptance, `write_identity_v1`, `write_identity_v2` and
`canonical_union_cardinality_v2` have zero downstream production callers and
only owner mechanical tests; `execution_binding_v1` has the three Task5B smart-
query callers. Task6 acceptance activates exactly the three reserved
CodeSearch/Definition/CallGraph whole-context constructor calls and reruns the
same static whitelist without changing any Task6 query bytes. Task7 acceptance
then activates exactly the two sink adapters, one prefix/admission encoder
caller per adapter, the one registry-construction binding projection, the
shared `derive_application_admission_v3` plus finish recheck, the sole
`count_effective_gap_material_subjects_v4` aggregate caller and the closed-enum
authority-validator dispatches. These downstream names are reservations, not
Task5B compile prerequisites; Task5B imports neither Task6 nor Task7. Any
different/additional caller or earlier phase activation fails, so the final
product cannot silently broaden an upstream capability.

### 3.11 Mandatory Task 4 back-propagation: dynamic registered material

The accepted Task 4 manifest can represent Present files and five fixed
`AbsentOptional` paths only. It cannot construct authoritative Missing
Form.xml/FormModule keys for an arbitrary registered Form. Task 5B must not
paper over that gap with a provider suffix table. Before the first Task5B
provider RED, land and re-review this exact Task 4 successor seam:

```text
REGISTERED_MATERIAL_EXPECTATION_CATALOG =
  "registered-material-expectations/v1"
REGISTERED_MATERIAL_PATH_POLICY =
  "registered-form-material-paths/v1"
MAX_REGISTERED_MATERIAL_EXPECTATIONS = 400_000
SINGLE_SOURCE_EXPECTATION_FIXTURE_AT_FILE_LIMIT =
  2 * (MAX_SNAPSHOT_FILES - 2) = 399_996
MAX_SNAPSHOT_MANIFEST_KEY_BYTES = 4_096
SOURCE_SNAPSHOT_FINGERPRINT_ENCODER = "source-set-snapshot/v2"
SOURCE_FINGERPRINT_DOMAIN = "unica.source-set-snapshot.v2"
COMPOSITE_SNAPSHOT_FINGERPRINT_ENCODER = "source-composite/v2"
COMPOSITE_FINGERPRINT_DOMAIN = "unica.source-composite.v2"
MAX_CAPTURED_ANALYSIS_BSL_SCAN_ITEMS = 400_000

SourceSetSnapshotV2       // exact one-source atomic snapshot
SourceSnapshotV2          // exact analysis-plus-destinations composite
CompositeSnapshotIdV2     // exact composite sha256 authority

impl CompositeSnapshotIdV2 {
  // Task4-sealed raw projection; exact sole production caller is
  // PlatformCatalogExecutionBindingV1::write_identity_v1.
  pub(crate) fn write_platform_catalog_execution_digest_v1(
    &self,
    out: &mut Vec<u8>,
  );
}

// Owned by neutral infrastructure::platform_xml, imported without aliases.
PlatformRegisteredFormTypeAuthorityV1
  Known(Managed)                                  tag 1, value tag 1
  Known(Ordinary)                                 tag 1, value tag 2
  Inconclusive(PlatformRegisteredFormTypeProblemV1) tag 2

PlatformRegisteredFormTypeProblemV1
  Missing                tag 1
  Duplicate              tag 2
  WrongNamespace         tag 3
  MixedContent           tag 4
  UnsupportedOrOverLimit tag 5

impl PlatformXmlSourceSpanV1 {
  fn start_byte(&self) -> u32;
  fn end_byte_exclusive(&self) -> u32;
}

impl PlatformRegisteredFormTypeAuthorityV1 {
  fn stable_tag(&self) -> u16;
  fn known_value_tag(&self) -> Option<u16>;
  fn problem(&self) -> Option<&PlatformRegisteredFormTypeProblemV1>;
}

impl PlatformRegisteredFormTypeProblemV1 {
  fn stable_tag(&self) -> u16;
}

impl PlatformRegisteredFormTypeCaptureV1 {
  fn authority(&self) -> &PlatformRegisteredFormTypeAuthorityV1;
  fn form_properties_span(&self) -> &PlatformXmlSourceSpanV1;
  fn form_type_spans(&self) -> &[PlatformXmlSourceSpanV1];
}

// Exact neutral retention traits; construction and serde remain private/absent.
PlatformXmlSourceSpanV1: Copy + Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeAuthorityV1: Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeProblemV1: Clone + Eq + Ord + Hash
PlatformRegisteredFormTypeCaptureV1: Clone + Eq

// Task4-owned opaque imports. Task5B repeats no private fields or constructors.
SnapshotManifestKeyRefV1<'snapshot>
SnapshotManifestKeyProjectionV1
RegisteredMaterialKindV1       // FormXml tag 1, FormModule tag 2
RegisteredMaterialRelationshipRefV1<'snapshot>
RegisteredMaterialRelationshipProjectionV1
RegisteredFormCaptureHandleV1<'snapshot>
CapturedRegisteredFormViewV1<'snapshot>
RegisteredMaterialExpectationHandleV1<'snapshot>
VerifiedRegisteredFormDescriptorBytesV1<'snapshot>
RegisteredMaterialReadV1<'snapshot>
VerifiedRegisteredMaterialNotApplicableV1<'snapshot>
VerifiedRegisteredMaterialAbsenceV1<'snapshot>
VerifiedRegisteredMaterialBytesV1<'snapshot>
CapturedAnalysisBslMaterialHandleV1<'snapshot> // Task5B builder only
CapturedAnalysisBslMaterialKindV1              // Task5B builder only
CapturedBslLocationRefV1<'snapshot>             // Task5B builder only
VerifiedCapturedBslMaterialBytesV1<'snapshot>
VerifiedBslSourceLocationV1
VerifiedBslCacheLocatorV1

impl SnapshotManifestKeyRefV1<'snapshot> {
  fn to_projection(self) -> SnapshotManifestKeyProjectionV1;
}

impl SnapshotManifestKeyProjectionV1 {
  fn encode_identity_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;

  fn encode_catalog_string_u32_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;
}

impl RegisteredMaterialRelationshipRefV1<'snapshot> {
  fn to_projection(self)
    -> RegisteredMaterialRelationshipProjectionV1;
}

impl RegisteredMaterialRelationshipProjectionV1 {
  fn encode_source_local_identity_v1(
    &self,
    enclosing_source: &ResolvedSourceSet,
    enclosing_source_fingerprint: &SourceFingerprintV1,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1>;
}

impl RegisteredFormCaptureHandleV1<'snapshot> {
  fn view(&self) -> CapturedRegisteredFormViewV1<'snapshot>;
  fn material(
    &self,
    kind: RegisteredMaterialKindV1,
  ) -> RegisteredMaterialExpectationHandleV1<'snapshot>;
}

impl CapturedRegisteredFormViewV1<'snapshot> {
  fn owner_descriptor_manifest_key(&self)
    -> SnapshotManifestKeyRefV1<'snapshot>;
  fn form_descriptor_manifest_key(&self)
    -> SnapshotManifestKeyRefV1<'snapshot>;
  fn form_descriptor_byte_length(&self) -> u64;
  fn form_descriptor_content_fingerprint(&self)
    -> &'snapshot SnapshotLeafFingerprintV1;
  fn form_root_span(&self) -> &'snapshot PlatformXmlSourceSpanV1;
  fn form_type_capture(&self)
    -> &'snapshot PlatformRegisteredFormTypeCaptureV1;
  fn form_type_authority(&self)
    -> &'snapshot PlatformRegisteredFormTypeAuthorityV1;
  fn form_properties_span(&self) -> &'snapshot PlatformXmlSourceSpanV1;
  fn form_type_spans(&self) -> &'snapshot [PlatformXmlSourceSpanV1];
}

impl RegisteredMaterialExpectationHandleV1<'snapshot> {
  fn kind(&self) -> RegisteredMaterialKindV1;
  fn relationship(&self)
    -> RegisteredMaterialRelationshipRefV1<'snapshot>;
  fn state(&self) -> RegisteredMaterialExpectationStateViewV1<'snapshot>;
}

impl CapturedAnalysisBslMaterialHandleV1<'snapshot> {
  fn kind(&self) -> CapturedAnalysisBslMaterialKindV1;
  // Builder-only typed capture projection; None only for unsupported ordinary.
  fn module(&self) -> Option<&'snapshot ArtifactRef>;
  // Builder-only admission authority: Some for either Present kind; None for
  // registered Missing/NotApplicable. No state/key/path/fingerprint accessor.
  fn admission_byte_length(&self) -> Option<u64>;
  fn registered_form(&self)
    -> Option<CapturedRegisteredFormViewV1<'snapshot>>;
  fn diagnostic_location(&self) -> CapturedBslLocationRefV1<'snapshot>;
}

impl CapturedBslLocationRefV1<'snapshot> {
  // Builder-only, zero-I/O conversion; no key/path/string access.
  fn to_verified_location(self) -> VerifiedBslSourceLocationV1;
}

RegisteredMaterialExpectationStateViewV1<'snapshot>
  NotApplicable
  | Missing { expected_manifest_key: SnapshotManifestKeyRefV1<'snapshot> }
  | Present {
      expected_manifest_key: SnapshotManifestKeyRefV1<'snapshot>,
      byte_length: u64,
      content_fingerprint: &'snapshot SnapshotLeafFingerprintV1,
    }

impl SourceSetSnapshotV2 {
  fn registered_forms<'snapshot>(&'snapshot self)
    -> impl ExactSizeIterator<
         Item=RegisteredFormCaptureHandleV1<'snapshot>> + 'snapshot;

  fn resolve_registered_material_projection<'snapshot>(
    &'snapshot self,
    projection: &RegisteredMaterialRelationshipProjectionV1,
  ) -> Result<RegisteredMaterialExpectationHandleV1<'snapshot>,
              SourceReadError>;

  fn captured_analysis_bsl_materials<'snapshot>(&'snapshot self)
    -> impl ExactSizeIterator<
         Item=CapturedAnalysisBslMaterialHandleV1<'snapshot>> + 'snapshot;
}

impl SourceSnapshotV2 {
  fn analysis_snapshot(&self) -> &SourceSetSnapshotV2;
  fn destination_snapshots(&self) -> &[SourceSetSnapshotV2];
  fn composite_snapshot_id(&self) -> &CompositeSnapshotIdV2;
  fn diagnostic_workspace_epoch(&self) -> u64;
}

trait SourceSnapshotPort {
  fn read_registered_form_descriptor_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: &RegisteredFormCaptureHandleV1<'snapshot>,
  ) -> Result<VerifiedRegisteredFormDescriptorBytesV1<'snapshot>,
              SourceReadError>;

  fn read_registered_material_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: RegisteredMaterialExpectationHandleV1<'snapshot>,
  ) -> Result<RegisteredMaterialReadV1<'snapshot>, SourceReadError>;

  fn read_captured_bsl_material_verified<'snapshot>(
    &self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    handle: CapturedAnalysisBslMaterialHandleV1<'snapshot>,
  ) -> Result<VerifiedCapturedBslMaterialBytesV1<'snapshot>,
              SourceReadError>;
}

impl VerifiedRegisteredFormDescriptorBytesV1<'_> {
  fn bytes(&self) -> &[u8];
}

RegisteredMaterialReadV1<'snapshot>
  NotApplicable(VerifiedRegisteredMaterialNotApplicableV1<'snapshot>) tag 1
  Missing(VerifiedRegisteredMaterialAbsenceV1<'snapshot>)             tag 2
  Present(VerifiedRegisteredMaterialBytesV1<'snapshot>)               tag 3

impl VerifiedRegisteredMaterialBytesV1<'_> {
  fn bytes(&self) -> &[u8];
  fn location_for_range(
    &self,
    start_byte: u32,
    end_byte_exclusive: u32,
  ) -> Result<VerifiedBslSourceLocationV1, SourceReadError>;
  fn cache_locator(
    &self,
  ) -> Result<VerifiedBslCacheLocatorV1, SourceReadError>;
}

impl VerifiedCapturedBslMaterialBytesV1<'_> {
  fn bytes(&self) -> &[u8];
  fn location_for_range(
    &self,
    start_byte: u32,
    end_byte_exclusive: u32,
  ) -> Result<VerifiedBslSourceLocationV1, SourceReadError>;
  fn cache_locator(&self) -> VerifiedBslCacheLocatorV1;
}
```

The imported composite writer is not a general Task4 digest API. Task5B may
name it only inside
`PlatformCatalogExecutionBindingV1::write_identity_v1`, where it appends the
first exact 32 bytes of the binding. No Task5B query, catalog, provider or
adapter may retain those bytes, call the method through an alias/callback, or
render/reparse the composite transport spelling. Task7 imports only the opaque
Task5B binding and never this Task4 method. The Task4 and Task5B static
whitelists must agree on that one fully qualified production call site.

No public/crate consumer API accepts a raw
`&RegisteredMaterialExpectationV1`, manifest path, relationship tuple or
caller-assembled key. The opaque handle binds its expectation to the enclosing
source manifest/source fingerprint, but contains no reader, filesystem root,
global service or hidden I/O capability. Both registered readers and the
captured-ordinary-BSL reader are methods on the injected `SourceSnapshotPort`;
their only generic parameter is a lifetime, so all remain object-safe and
callable through `&dyn SourceSnapshotPort`. There is no free registered/BSL
reader or global/root fallback. The specialized
reader compares complete
semantic snapshot/source authority before I/O; an equal independently
reconstructed validated snapshot is allowed, while a different snapshot/source/
manifest handle returns nonretryable
`SourceReadError::RegisteredMaterialHandleMismatch` /
`registered_material_handle_mismatch` before any probe or byte read.
The Present wrapper retains the same complete authority after I/O; general
consumers borrow only `bytes()`, while the whitelisted analysis-BSL dispatcher
may additionally retain its opaque verified location/cache capabilities. No
consumer can replay generic verified bytes across another Form, kind, key or
snapshot. Missing and NotApplicable are likewise specialized
opaque authorities, not bare state tags.

The error boundary is exact. A handle/projection/source/fingerprint/manifest/
Form/kind/relationship/state/key/ordinary-entry disagreement is semantic and
returns nonretryable `RegisteredMaterialHandleMismatch` before any I/O (or is
rejected earlier as construction-impossible). Only after all semantic checks
pass may external filesystem appearance/disappearance, identity/content change,
link/reparse swap or containment drift return retryable
`SourceFingerprintMismatch`. Internal state/key/entry disagreement is never
laundered into a retryable drift error.

The expectation map is part of the immutable snapshot manifest. For applicable
Managed material, its exact expected path remains there even when it has no
ordinary `ManifestEntry`; NotApplicable carries no path at all. Relationship/
state/key fields are private capture-owned smart values with no raw path, serde
or consumer constructor. Missing returns an opaque absence authority bound to
source identity/fingerprint, relationship and expected key. Present delegates
to the ordinary verified reader and requires byte length/fingerprint equality
with both the expectation and Present manifest entry, then wraps the bytes as
`VerifiedRegisteredMaterialBytesV1<'snapshot>`. NotApplicable returns
opaque `VerifiedRegisteredMaterialNotApplicableV1<'snapshot>` bound to source fingerprint/
relationship/state and performs no filesystem lookup. No caller treats an enum
tag alone as verified authority. PlatformCatalogPort receives
only the borrow-only registered-Form iterator/resolver. Exactly one pair with
kind tags 1/2 exists for every iterator handle; missing/duplicate/reversed-kind
pair is snapshot construction failure. `RegisteredFormCaptureHandleV1` is an
opaque lifetime-bound borrow created only by its enclosing manifest iterator;
`material(kind)` consults that same borrowed manifest, so no caller can pass a
Form from snapshot A to a resolver on snapshot B. No address/generation token,
`ptr::eq`, or allocation identity is created: a separately reconstructed but
byte-identical validated snapshot is equal authority, while any semantic
snapshot/source-fingerprint mismatch fails later context resolution. The port never
constructs a tuple from descriptor strings or changes a frozen state. For
Missing/Present only it converts the captured expected key to
`RegisteredFormManifestKeyV1`; for NotApplicable it copies no key.

Task4's projection lookup is the sole later reverse bridge. It validates the
projection's complete private source identity/fingerprint/manifest/
owner/Form/kind binding against the supplied snapshot, then uses Task4's private
ordered relationship index to return the exact by-value expectation handle in
`O(log N)`. Any source/fingerprint/manifest/relationship mismatch returns
nonretryable `registered_material_handle_mismatch` before I/O. It exposes no key,
tuple, index or projection fields. Task5B may not recover a relationship by
scanning `registered_forms()`; that iterator is for the one catalog build only.

The conversion is the sole lossless Task5B key projection and uses Task4's
opaque token and identity encoder verbatim:

```text
// Task5B-private consumer newtype; Task4 never imports it.
RegisteredFormManifestKeyV1 {
  capture_key: SnapshotManifestKeyProjectionV1,
}

impl RegisteredFormManifestKeyV1 {
  // private; caller must supply a ref returned by capture view/state
  fn from_capture_key(key: SnapshotManifestKeyRefV1<'_>) -> Self {
    Self { capture_key: key.to_projection() }
  }

  fn encode_identity_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_key.encode_identity_v1(encoder)
  }

  fn encode_registered_form_catalog_string_v1(
    &self,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_key.encode_catalog_string_u32_v1(encoder)
  }
}

impl RegisteredFormMaterialAuthorityV1 {
  // private; the ref must come from the matching Task4 expectation handle
  fn retain_capture_relationship(
    relationship: RegisteredMaterialRelationshipRefV1<'_>,
  ) -> RegisteredMaterialRelationshipProjectionV1 {
    relationship.to_projection()
  }

  fn encode_capture_relationship_identity_v1(
    &self,
    enclosing_source: &ResolvedSourceSet,
    enclosing_source_fingerprint: &SourceFingerprintV1,
    encoder: &mut CanonicalIdentityEncoderV1,
  ) -> Result<(), CanonicalEncodingErrorV1> {
    self.capture_relationship.encode_source_local_identity_v1(
      enclosing_source,
      enclosing_source_fingerprint,
      encoder,
    )
  }
}
```

There is no constructor from String, bytes, `Path`/`PathBuf`, `ArtifactRef`,
suffix, parsed components, generic normalized-key type, serde or display text.
Clone/Eq/Ord/Hash and both permitted encodings delegate losslessly to the
Task4-owned `SnapshotManifestKeyProjectionV1`; Task5B owns no normalization,
key identity encoder or string framing. The existing `encode_identity_v1`
remains the u64-framed manifest/source-identity projection. Only the private
`encode_registered_form_catalog_string_v1` wrapper may encode one of the three
registered-Form sidecar manifest-key `string` fields, and it delegates directly
to Task4's checked u32-framed `encode_catalog_string_u32_v1`. It has no raw or
length accessor and cannot prepend a prefix locally. The private constructor is called only by `PlatformCatalogPort` with
`CapturedRegisteredFormViewV1::{owner_descriptor_manifest_key,
form_descriptor_manifest_key}` or a Missing/Present state's
`expected_manifest_key`. Task4 never imports this wrapper, so the dependency
remains one-way and acyclic.

The complete material relationship follows the same one-way rule. The private
`RegisteredFormMaterialAuthorityV1::capture_relationship` field is exactly one
Task4-owned `RegisteredMaterialRelationshipProjectionV1`, obtained immediately
from the matching expectation handle's opaque relationship ref. It is
`Clone + Eq + Ord + Hash` only through that Task4 projection and has no raw
tuple/key/component accessor, serde form, local duplicate or Task5B constructor.
Its sealed encoder is used only by the private capture-resolution comparison.
It first validates the enclosing `ResolvedSourceSet` and `SourceFingerprintV1`
against the projection's private complete binding, then appends only the
unchanged source-local owner-key/Form-key/kind bytes. Task5B must neither prepend
source/fingerprint again nor emulate the nested relationship bytes. This private
comparison transcript is not a field of
`RegisteredFormCatalogIdentityBytesV1`, is not persisted or exposed, and cannot
change its published nonempty goldens. A mismatch is a resolver/encoding error,
never a reason to regenerate catalog goldens.

The exact public construction boundary is object-safe and complete:

```text
PlatformCatalogBuildErrorV1
  SourceRead(SourceReadError)                 tag 1
  SnapshotCatalogMismatch                    tag 2
  PlatformXmlParserInvariant                 tag 3
  CatalogResourceLimit                       tag 4

trait PlatformCatalogPort {
  fn build_context(
    &self,
    snapshot: &SourceSnapshotV2,
    source_reader: &dyn SourceSnapshotPort,
  ) -> Result<PlatformCatalogContextV1, PlatformCatalogBuildErrorV1>;
}
```

`SourceRead` preserves the exact nested reason/retryability. The other three
cases are nonretryable and expose respectively
`platform_xml_snapshot_catalog_mismatch`, `platform_xml_parser_invariant`, and
`platform_catalog_resource_limit`; none may carry parser text, a path or a
partial context. The trait has no generic type/associated constructor/`Self`
result and is callable through `&dyn PlatformCatalogPort`.

The mapping is total and noninterchangeable:

| Failure origin | Exact build error |
| --- | --- |
| injected descriptor/read failure | `SourceRead(exact SourceReadError)` |
| composite/atomic identity, order, catalog or witness-bijection mismatch | `SnapshotCatalogMismatch` |
| catalog-local exact-artifact-spelling collision after accepted capture | `PlatformXmlParserInvariant` |
| equal verified descriptor bytes disagree with the captured guard/envelope, or an impossible neutral parser state | `PlatformXmlParserInvariant` |
| checked catalog/witness allocation or count bound | `CatalogResourceLimit` |

No provider gap, generic invariant or alternate retryability mapping is allowed.

This one public method owns all preparation; there is no separately callable
preparation phase and no overload omits or replaces the injected reader. It
visits `snapshot.analysis_snapshot()` first and
`snapshot.destination_snapshots()` in their Task4 canonical unique order. For
each atomic source it iterates `atomic_snapshot.registered_forms()` in canonical
order and, for each of the composite's N source-qualified capture handles,
calls exactly once
`source_reader.read_registered_form_descriptor_verified(atomic_snapshot,
&handle)`.
That Task4 port seam
performs the one ordinary Present descriptor read and returns
`VerifiedRegisteredFormDescriptorBytesV1`; Task5B never supplies a path to
generic `read_verified`. It then performs exactly one shared guard/semantic
pass over `bytes()`, requires its root/properties/FormType authority and spans
to equal `handle.view()`, and obtains wrapper UUID/membership semantics. Task5B
cannot select, normalize or reinterpret FormType; it copies the neutral
authority/spans exposed by the view. Any guard/envelope disagreement on equal
verified bytes is `platform_xml_parser_invariant`, discards the whole catalog
context as unavailable, and yields no prefix. This is one shared descriptor
guard/semantic pass, not an independent Task5B FormType parser.

The descriptor method validates each handle through Task4's private ordered
captured-Form index in `O(log N)` before its one ordinary read; it does not compare
or walk a complete manifest/Form catalog per handle. A synthetic accepted
multi-source 200,000-captured-Form context-build RED records exactly N indexed descriptor-
handle validations, N ordinary descriptor reads and N shared parses, with zero
full-manifest, captured-Form or expectation-map rescans. Thus the full public
build is `O(N log N)`, never `O(N^2)`.

`build_context` is deterministic, not globally one-shot. Two calls with
semantically equal `SourceSnapshotV2` values and equal verified reader bytes
are allowed and return semantically equal contexts/digests; a different
diagnostic epoch alone changes nothing. Non-Clone/private context fields prevent
detached lookalikes, but do not falsely claim that borrowed inputs can be
consumed once. Task7 is the sole production call site and enforces exactly one
call per execution through its orchestration/static call-site check and injected
recording spy. A second production call is a Task7 contract violation, not a
`PlatformCatalogBuildErrorV1` branch.

After preparation, a private non-port
`PlatformCatalogContextV1::from_prepared(...)` validates the manifest handle/
expectation bijection and snapshot/catalog identities exactly once in memory.
It is not externally callable and performs zero I/O. The single public port
call performs **zero** dynamic `registered_material_verifier_calls` and zero
Form.xml/FormModule probes/byte reads; recording spies distinguish its expected
ordinary descriptor reads from dynamic-material counters. Every query
constructor performs zero reader/filesystem calls.

Only after query construction may a semantic MetadataCatalog/FormInspection
provider request FormXml verification by passing its context-owned Form view,
exact atomic snapshot and injected reader to
`context.read_registered_platform_form_verified(...)`, once per deduplicated
applicable relationship demanded by that provider invocation. The provider
never resolves a capture handle or calls the raw Task4 registered reader. Inside
the private context method, Managed+Present performs one verifier and one
verified byte read; only Managed+Missing performs one verifier plus the
contained component-wise absence proof with zero file-byte reads and zero XML
parses. Ordinary/Inconclusive NotApplicable performs zero verifier calls, zero
byte reads and zero XML parses. Later
appearance/disappearance/mutation returns exact `source_fingerprint_mismatch`
and discards the whole staged provider batch. Thus freshness proof is demand-
scoped without turning catalog/query construction into dynamic-material I/O.

The manifest validator treats the handle catalog and expectation map as one
bijective relation, not two optional side tables. Every handle key occurs once,
has exactly two relationship rows with byte-equal owner/Form descriptor keys
and distinct kind tags 1/2, and every relationship row points back to exactly
one handle. An orphan/duplicate handle, orphan relationship, unequal descriptor
key, missing/duplicate kind, or FormType/state matrix mismatch rejects the
whole snapshot before fingerprint construction. Each handle constructor/
decoder also requires its Form descriptor key to resolve to the ordinary
Present manifest entry with exact equal byte length and leaf fingerprint; every
root/properties/FormType span must satisfy
`start < end <= form_descriptor_byte_length` and exact UTF-8/node boundaries
in the admitted descriptor. Missing, AbsentOptional,
non-regular, length/fingerprint mismatch or out-of-bounds span rejects the
whole manifest. The source fingerprint binds the complete handle catalog,
exact descriptor length/leaf, neutral FormType authority and exact properties/
field witness spans as well as the two expectation rows; therefore equal
expectation rows cannot replay a different descriptor, FormType capture or
witness.

`PlatformRegisteredFormTypeAuthorityV1`, its closed problem type,
`PlatformXmlSourceSpanV1`, `PlatformRegisteredFormTypeCaptureV1`, and the sole
FormType capture parser are owned by neutral `infrastructure::platform_xml`.
That module imports neither Task 4 snapshot types nor Task 5B catalog/provider
types. Task 4 and Task 5B both import the neutral contract; neither may define
an alias, local duplicate, conversion copy, or callback into the other task. A
compile-time dependency test rejects `platform_xml -> task4`,
`platform_xml -> task5b`, and `task4 -> task5b` module edges. Task 5B's existing
one-way import of the Task 4 snapshot/read port remains legal, but it does not
own, wrap, alias, or reconstruct the neutral FormType capture authority.

During capture, Task 4 calls that parser first and retains the exact returned
authority, properties span and canonical contributing/defect span vector behind
its private capture value. `PlatformCatalogPort` later obtains only
`CapturedRegisteredFormViewV1` from the opaque handle and losslessly clones the
same neutral `Clone`/`Eq` authority and span values into its semantic catalog/
witness projection. It does not own an independent FormType parser, constructor,
conversion or choice. Its one shared guard pass
over verified descriptor bytes must reproduce the stored envelope byte-for-byte
and cannot reinterpret the authority. Capture creates exactly two relationship slots for
**every** admitted nested Form. Known Ordinary and Inconclusive freeze both
slots as NotApplicable with no key derivation or path probe. For exact Known
Managed only, start from the already-validated normalized
`form_descriptor_manifest_key`, require an exact case-sensitive terminal
`.xml`, remove only those four ASCII bytes to obtain `form_stem`, and derive:

```text
FormXml    = form_stem || "/Ext/Form.xml"
FormModule = form_stem || "/Ext/Form/Module.bsl"
```

This is the sole versioned serializer relationship algorithm. It runs inside
Task 4 capture, never in Metadata/Form/BSL/Task8 consumers and never from an
ArtifactRef/display name. The owner and Form descriptor keys remain in the
relationship identity so equal expected path spelling cannot be rebound to
another registration. Each Managed-derived key reruns the same normalized
contained-relative-file grammar including the v2-published exact
`MAX_SNAPSHOT_MANIFEST_KEY_BYTES = 4,096` UTF-8 byte bound:
nonempty components, `/` separators, no absolute/drive/
UNC/traversal/control/empty component, platform alias or case-insensitive
collision. A relationship-key duplicate, two relationships claiming one
expected key, collision with another admitted material relationship, 4,097-byte
key, or descriptor without exact `.xml` is snapshot-fatal.

Live Task4 code already enforces `contained_relative_file(path.len() <= 4096)`.
The successor promotes that existing code-authoritative invariant into the
published `SnapshotManifestKeyV1` contract; it does not narrow the accepted key
set. Exact 4,096-byte keys retain spelling/semantics and 4,097-byte keys remain
rejected. The new snapshot/composite v2 domains version the added handle/
relationship authority, not a path-compatibility change, and prevent silent
v1/v2 mixing.

The capture plan is two-ended and race-safe:

1. the initial bounded registration-aware enumeration captures each Form
   descriptor, runs the neutral FormType parser, fixes every opaque registered-
   Form handle/witness and freezes its two NotApplicable or applicable
   relationship states;
2. for each applicable expected key present in that enumeration, capture
   no-follow file identity, bounded bytes, length and SHA-256; otherwise record Missing after
   a component-wise contained no-follow lookup proves NotFound at the first
   absent component. A safely missing intermediate directory is valid absence;
   permission/I/O error, symlink/reparse/special object, containment ambiguity
   or case alias is not Missing;
   A NotApplicable slot performs no lookup;
3. after all reads and the injected mutation hook, independently re-enumerate
   registrations, reread/reparse verified FormType and rederive the complete
   expectation map; require exact initial/final FormType authority, witness,
   relationship, key and presence-state equality;
4. revalidate every Present identity/content under the existing Task 4 rules
   and repeat a contained no-follow NotFound check for every Missing expected
   key. Appearance, disappearance, kind change, key/registration change or
   identity/content drift discards the whole capture as retryable
   `SnapshotCaptureReason::SourceChangedDuringCapture` /
   `source_changed_during_capture`.

After an accepted snapshot,
`source_reader.read_registered_material_verified` repeats the
same Present identity/content or Missing component-walk check. Later appearance,
disappearance or byte/identity drift is `SourceReadError::SourceFingerprintMismatch`
with reason `source_fingerprint_mismatch`; it is never rewritten as the earlier
capture-time reason or as complete absence.

The ordinary `MAX_SNAPSHOT_FILES=200_000`, `MAX_SNAPSHOT_BYTES=4 GiB` and node/
deadline limits still count real captured material once across the complete
analysis-plus-destinations capture. Independently, the relationship
collection/decoder uses checked addition and the hard allocation ceiling
`MAX_REGISTERED_MATERIAL_EXPECTATIONS=400,000`; its isolated bounded helper
accepts exactly 400,000 slots and rejects 400,001 as the snapshot resource
limit before hashing or provider I/O. This is a defense-in-depth Task4 memory
cap, not the structurally reachable production maximum and not a provider scan/
admission prefix.

A nonempty **single-source** registered-Form capture needs at least one
Configuration.xml, one distinct owner descriptor and one distinct descriptor
per Form. Its 200,000-file fixture therefore reaches at most 199,998 Forms and
exactly `SINGLE_SOURCE_EXPECTATION_FIXTURE_AT_FILE_LIMIT=399,996` slots when
managed material is Missing/NotApplicable. The fixture derives this value by
checked `2 * (MAX_SNAPSHOT_FILES - 2)`; 199,998 Forms pass and 199,999 Forms
fail the ordinary file boundary.

That test-only single-source fixture constant is **not** a production/resource
`MAX_` and is not the global relationship maximum.
Live capture counts unique Present workspace paths globally, while accepted
source roots may be nested/overlapping; one physical descriptor may therefore
legitimately contribute a distinct source-qualified handle in more than one
manifest. Without adding a new path-disjoint invariant, capture must sum all
source-qualified slots with checked arithmetic and enforce the independent
global 400,000 memory cap. An overlapping-source synthetic fixture reaches
exactly 400,000 and passes; adding one further Form pair yields 400,002 and
rejects before hashing/provider I/O. The isolated collection/decoder helper
also tests raw 400,000 pass/400,001 reject even though a valid handle matrix has
an even slot count. No per-source cap or disjoint-root shortcut substitutes for
the global sum.

Source snapshot v2 retains the existing v1 entry encoding, then appends the
complete captured-Form handle catalog followed by the relationship catalog.
Captured handles are sorted unique by complete handle-key bytes and encode as:

```text
u64be(captured_form_count)
|| each captured form:
     string(owner_descriptor_manifest_key)
  || string(form_descriptor_manifest_key)
  || u64be(form_descriptor_byte_length)
  || fingerprint32(form_descriptor_content_fingerprint)
  || u32be(form_root_span.start_byte)
  || u32be(form_root_span.end_byte_exclusive)
  || [Known: u16be(1) || u16be(Managed=1 | Ordinary=2)
      | Inconclusive: u16be(2) || u16be(problem tag)]
  || u32be(form_properties_span.start_byte)
  || u32be(form_properties_span.end_byte_exclusive)
  || u64be(form_type_span_count)
  || each canonical sorted-unique FormType span:
       u32be(start_byte) || u32be(end_byte_exclusive)
```

The encoder then appends `u64be(expectation_count)` and each expectation sorted
unique by complete relationship bytes:

```text
string(owner_descriptor_manifest_key)
|| string(form_descriptor_manifest_key)
|| u16be(material kind tag)
|| u16be(NotApplicable=1 | Missing=2 | Present=3)
|| [Missing: string(expected_manifest_key)
    | Present: string(expected_manifest_key)
      || u64be(byte_length)
      || fingerprint32(content_fingerprint)
    | NotApplicable: empty]
```

Here `string` retains Task 4's existing `u64be(byte_length) || UTF-8` framing;
it is deliberately not the section-6.1 u32-framed semantic encoder.

The following three single-row payload goldens are normative and are rebuilt
by the Task 4 v2 encoder, never copied from prose. Their common relationship is
owner key `Catalogs/Σ.xml`, Form descriptor key
`Catalogs/Σ/Forms/Main.xml`, and `FormXml` kind tag 1. Missing/Present use
expected key `Catalogs/Σ/Forms/Main/Ext/Form.xml`; Present additionally uses
`byte_length=7` and raw leaf-fingerprint bytes `0xff` repeated 32 times:

```text
NotApplicable row length = 61
SHA-256(row bytes) =
  919a2297228863374bc95db5c2202b207a44800963d2f908832cd7d8974900f8

Missing row length = 104
SHA-256(row bytes) =
  e1b7e076c32b256446871f8057a4b2302399d7cc4c111d0ec131ddaa01987bfe

Present row length = 144
SHA-256(row bytes) =
  a5b89c72bf780da36a0a8f84115e3ef4314843ef578fd48cab35de1032fdb2f3
```

Changing owner/Form key, kind, state, expected key, length, or any fingerprint
byte changes the designated row hash. Expectation input permutation preserves
the sorted unique manifest suffix bytes and complete source fingerprint.

The source hash uses the new source-snapshot v2 domain; the composite hash uses
the new composite v2 domain because it embeds those source fingerprints. Old v1
snapshots cannot be mixed with v2 or accepted by a v7 query/context. The
transport remains the distinct exact `SourceFingerprintV1` smart type from
section 3.8; its `V1` denotes transport grammar, not the snapshot hash domain.

Mandatory REDs cover zero/one/many Forms; NotApplicable, Present and Missing
for each kind; no lookup/path derivation for NotApplicable; all three exact row
goldens plus every listed mutation and expectation permutation;
safe absence at the Form directory, `Ext`, `Form` and final-leaf component;
checked two-slot, single-source 199,998/199,999-Form file boundaries,
overlapping-source global 400,000/400,002 valid-matrix boundaries and raw
400,000/400,001 allocation-helper arithmetic; 4,096/4,097 key bytes; `.xml`
case/terminal/short-name boundaries; traversal,
absolute, separator, case/alias, relationship and expected-key collisions;
initial Missing then final Present, initial Present then final Missing, content/
identity change and registration rename with exact capture-time reason; the
same post-acceptance races with exact read-time reason; cross-kind/cross-Form/cross-source
absence replay; Present expectation without equal ManifestEntry; and a static
ban on the two suffix literals outside the one Task 4 relationship module and
diagnostic/spec fixtures. FormType mutation between the two capture passes
selects the exact capture-time race reason. Handle add/remove/duplicate,
FormType authority/problem mutation, properties/field-span mutation, an orphan
relationship, descriptor length/leaf mismatch, root/properties/field span past
descriptor length or off UTF-8/node boundaries,
and a relationship/handle key mismatch are rejected or change the source
fingerprint exactly as specified; handle/row input permutation does not.
The exact Missing key survives the
immutable manifest/fingerprint/context boundary despite having no ordinary
ManifestEntry; NotApplicable survives without ever acquiring a key.

Static/API-token REDs additionally freeze the ownership boundary itself:

```text
RegisteredMaterialExpectationHandleV1::relationship()
  -> RegisteredMaterialRelationshipRefV1<'snapshot>
RegisteredMaterialRelationshipRefV1::to_projection()
  -> RegisteredMaterialRelationshipProjectionV1
RegisteredFormMaterialAuthorityV1::capture_relationship
  : RegisteredMaterialRelationshipProjectionV1
RegisteredMaterialRelationshipProjectionV1::encode_source_local_identity_v1(
  &self,
  &ResolvedSourceSet,
  &SourceFingerprintV1,
  &mut CanonicalIdentityEncoderV1,
)
SourceSetSnapshotV2::resolve_registered_material_projection(
  &self,
  &RegisteredMaterialRelationshipProjectionV1,
) -> Result<RegisteredMaterialExpectationHandleV1<'_>, SourceReadError>
```

The same checks reject a Task5B field of private
`RegisteredMaterialRelationshipKeyV1`, a local tuple/projection/encoder, raw key
or relationship accessors, and any source/fingerprint bytes emitted again inside
the nested relationship payload. Mechanical nonempty registered-Form catalog
goldens must remain the exact section-5 values after this ownership repair; a
mismatch is STOP and cannot be repaired by silently regenerating goldens or
changing a contract version/domain.

## 4. Provider I/O and composite query architecture

Providers receive only typed query plans, the captured composite snapshot, the
borrowed `PlatformCatalogContextV1`, a verified snapshot reader, and an injected
monotonic budget/clock. Every present byte outside the already-built catalog
authorities is read through the correct injected Task4
`&dyn SourceSnapshotPort`: fixed manifest material uses
`source_reader.read_verified` only at its owner-whitelisted fixed-material
boundary. Registered material never exposes a Task4 expectation handle or raw
registered-reader method to a provider: MetadataCatalog/FormInspection call the
context-owned FormXml verification boundary, Task 6 calls only
`context.read_analysis_bsl_material_verified(...)` with one admitted item, and
the private Task 5B context implementation alone resolves the expectation and
invokes `source_reader.read_registered_material_verified`. Provider production
modules contain no direct filesystem walk, existence probe, canonicalize,
SQLite, CLI, or display parsing.

One `MetadataCompositeQueryV2` and one MetadataCatalog invocation cover:

1. the complete analysis registered catalog;
2. all analysis presence-query keys;
3. every exact requested destination membership pair.

The provider partitions that single invocation into deterministic internal
groups: analysis first, then destination source sets by complete
`ResolvedSourceSetIdentityBytesV1`. `SourceSetWide` is local to one group. `QueryWide` is legal only when a
limitation invalidates the whole composite invocation. An unpaired destination
sibling is never read.

FormInspection is called exactly once for the analysis source. For an exact Form
membership pair, MetadataCatalog independently selects the typed analysis and
destination catalog views, invokes the composite-context verification method for
each Form.xml, and passes each Present wrapper to the same neutral parser before
it may emit either role-specific companion. It never receives raw material
bytes, a Task4 handle or a detached digest, consumes no FormInspection output,
and creates no second FormInspection call. This intentional provider-local read
keeps each provider result, atomic limit, and gap authority self-contained.

The query plan contains sorted unique, source-qualified values:

```text
analysis_presence_query_keys       <= 64
requested_command_presence_keys    <= 1,024 (derived from <=32 x <=32;
                                             never caller supplied)
destination_membership_pairs       <= 64
requested_form_material_scopes     <= 32 forms
form_commands_per_form             <= 32
form_runtime_subjects_per_form     <= 256
destination_pairs_per_form         <= 64
provider-local exact-gap retention <= 2,000 distinct material subjects
                                     (post-scan normalization threshold)
```

The 1,024 command value bounds only request-derived presence keys. Commands
discovered while scanning all Managed Forms are not silently capped by it;
their total is bounded by the accepted snapshot/parser node and byte limits,
then admitted only as complete semantic groups under `maxEvidence`. Any dropped
tail is normalized through the exact provider gap rules below.

The request association builder first produces at most 32 requested
`FormMaterialScopeV1` values and their digest. The query separately borrows the
exact analysis `RegisteredFormCatalogV1` and binds its digest. Before provider
I/O, FormInspection iterates **every** sidecar entry in canonical Form identity
order. An unrequested exact-Managed entry receives a deterministic Form-only
effective scope with explicit empty command/runtime/pair vectors; a requested
exact-Managed entry unions the already-frozen request contribution. Exact
Ordinary and FormType-inconclusive registrations remain in the exhaustive
authority but produce their exact Form-scoped typed gap with zero Form.xml
reads. Thus every registered Form is classified and every registered managed
Form is scanned, but the query need not encode up to 200,000 expanded empty
scopes. No scope is reconstructed from a missing Form.xml or inferred from a
path.

The application `FormMaterialAssociationBuilderV1` accepts bounded raw request
contributions, groups them by complete
`SourceScopedArtifactIdentityBytesV2(form)`, validates the same analysis source/
owner, and unions equal command, runtime and pair members before applying final
form/member bounds. Equal duplicate proposal contributions disappear only at
that application association boundary. It then calls the private
`FormMaterialScopeV1::new` with already canonical unique vectors; that final
typed constructor rejects duplicate direct members, conflicting source/
ownership or a pair whose analysis half/artifact is not this Form. The requested
vector has one scope per requested Form, never one per proposal or destination;
the sidecar supplies the complete registered scan authority. Therefore one Form
is read/grouped once even when it supports destinations A and B, and a missing
requested Form gap names both pairs. A missing unrequested Form uses the exact
Form-only scope.

Each sidecar Form consumes one unique capture-admitted descriptor manifest key,
so Task 4 `MAX_SNAPSHOT_FILES=200_000` and `MAX_SNAPSHOT_BYTES=4 GiB` bound the
full scan authority. There is no arbitrary smaller Form-count cap or prefix.
Snapshot file/byte N+1 fails capture before a provider query. A provider
deadline/unavailable result has zero staged records and never returns a timing-
dependent partial Form scan.

The 2,000-subject value is not a request, sidecar, scan or parser-construction
bound. A full scan may classify more than 2,000 unrequested Forms. Only after
all exact provider gaps are known does section 6 retain an exact union at
<=2,000 or replace the complete gap vector at 2,001 with the one QueryWide
`platform_xml_gap_limit` sentinel. It never rejects, prefixes or truncates the
registered scan.

Typed query-member constructors reject their listed +1 and any source/artifact
mismatch before reader call 1; the post-scan gap threshold is excluded from
that sentence.
Task/search text, filesystem order, `knownArtifacts`, and `maxCandidates` never
narrow these authoritative scans.

### 4.1 Task 7 exact back-propagation

Task 7 must describe the same single composite Metadata invocation, not one call
per source. Its scoped query variant and cache identity are versioned together:

```text
METADATA_COMPOSITE_QUERY_ENCODER = "metadata-composite-query/v2"
FORM_INSPECTION_QUERY_ENCODER = "form-inspection-query/v2"

MetadataCompositeQueryV2<'context> {
  // Private semantic authority borrow; excluded from every identity byte.
  catalog_context: &'context PlatformCatalogContextV1,
  execution_binding: PlatformCatalogExecutionBindingV1,
  composite_snapshot_id,
  configuration_catalog_set_digest,
  registered_form_catalog_set_digest,
  analysis_source_set,
  destination_source_sets,
  destination_membership_pairs: Vec<DestinationMembershipPair>,
  analysis_presence_query_keys: Vec<SourceScopedArtifact>,
  requested_form_material_scopes: Vec<FormMaterialScopeV1>,
  max_records,
}

FormSourceSetQueryV2<'context> {
  // Private semantic authority borrow; excluded from every identity byte.
  catalog_context: &'context PlatformCatalogContextV1,
  execution_binding: PlatformCatalogExecutionBindingV1,
  analysis_source_set,
  analysis_source_fingerprint,
  analysis_configuration_catalog_digest,
  analysis_registered_form_catalog_digest,
  requested_form_material_scopes: Vec<FormMaterialScopeV1>,
  max_records,
}
```

The public smart constructors accept one explicit
`&'context PlatformCatalogContextV1` plus those typed vectors, not caller-selected
inner hashes. They canonicalize and validate every member, recompute the three
private digests below, and expose read-only digest accessors for invocation
snapshots. A constructor receiving a serialized/forged inner digest does not
exist. The configuration-catalog-set/catalog and registered-Form-sidecar-set/
catalog digests are recomputed from and compared with the same-lifetime
once-built context authorities; a mismatch, missing sidecar source or cross-set
source/fingerprint/configuration-catalog mismatch fails before provider I/O.
Each constructor also mints and retains the equal private
`PlatformCatalogExecutionBindingV1`. The query retains that binding and private
context borrow for its full lifetime; it cannot outlive or detach from the
authority that validated the digests. The borrow, pointer/address and binding do not enter any query
payload or digest, so this lifetime repair changes none of the frozen bytes or
goldens below.

The three inner contracts are exact:

```text
DESTINATION_MEMBERSHIP_PAIR_SET_ENCODER =
  "destination-membership-pair-set/v1"
METADATA_PRESENCE_KEY_SET_ENCODER = "metadata-presence-key-set/v1"
FORM_MATERIAL_SCOPE_SET_ENCODER = "form-material-scope-set/v1"

pair_digest = H("unica.destination-membership-pair-set/v1",
  vec(DestinationMembershipPairIdentityBytesV2 sorted unique))

presence_key_digest = H("unica.metadata-presence-key-set/v1",
  vec(SourceScopedArtifactIdentityBytesV2 sorted unique))

FormMaterialScopeIdentityBytesV1 =
  SourceScopedArtifactIdentityBytesV2(form)
  || vec(SourceScopedArtifactIdentityBytesV2(commands) sorted unique)
  || vec(SourceScopedArtifactIdentityBytesV2(runtime subjects) sorted unique)
  || vec(DestinationMembershipPairIdentityBytesV2(applicable pairs)
         sorted unique)

form_material_scope_digest = H("unica.form-material-scope-set/v1",
  vec(FormMaterialScopeIdentityBytesV1 sorted unique))
```

`SourceScopedArtifactIdentityBytesV2` always contains the complete resolved
`AtomicSourceIdentityV2`, never a source display name. Each command must be the
exact FormCommand child of its scope Form; each runtime subject must be an exact
owner/Method material subject of that Form/proposal; every pair must contain
that same Form identity and the scope's exact analysis source half. The pair
vector is a sorted-unique subset of the outer destination-membership-pair vector;
missing/extra pairs are rejected. The command-presence
worklist is the derived union of the scope command vectors; there is no fourth
caller-controlled vector/digest that can disagree. An empty nested vector and an
empty outer vector encode their explicit count and are not omitted; the former
singular `option(applicable pair)` is superseded.

The empty-vector domain goldens are:

```text
pair_digest(empty) =
  16f939c6863dcef2b8a428df37ca1bb916d7cd2fbcb9e5eb55079bfc1c6d157e
presence_key_digest(empty) =
  0309b6b4725ffa77e02293f24978d1d179ce99ad30fc447fcb33bb8f43c3a992
form_material_scope_digest(empty) =
  2401443d3ea67d99941f92b72562614ec57a24049c64b023fd03b3db736c6e07
```

The one-member inner golden adds Destination `patch` (Extension, PlatformXml,
root `ext/patch`, mapping digest `sha256:` + `b`*64), pair Form
`Document.Order.Form.Main`, analysis presence `MetadataObject "Document.Σ"`,
command `Document.Order.Form.Main.Command.Run`, and runtime Method
`Document.Order.Form.Main.FormModule.Run`:

```text
pair identity bytes length = 339
pair_digest(one) =
  0dd462387baf8fd7f7a55b8eb83a47dfbe22b2f3bac28b932410571ab22eaed4
presence identity bytes length = 169
presence_key_digest(one) =
  e1edb517a8e4e4188543c23e3c5f6048cb8719c02048f8dc4795f50df257497b
FormMaterialScopeIdentityBytesV1 length = 924
form_material_scope_digest(one) =
  7f2411f23ec2f4bddcd5fefef5f3ef77badeeb53722dc28272d2417cc99f58d1

Add destination `patch-b` (Extension, PlatformXml, root `ext/patch-b`, mapping
digest `sha256:` + `c`*64) for the same Form. Its pair identity length is 343.
The one merged two-pair scope has length 1267, SHA-256(scope bytes)
`22a5cae59ef4aad757c406aad8b631433128f8d378c55bed872d85d56c1e74f7`,
and `form_material_scope_digest` =
`744a56b607a33ebfd06184b7597fb55dcc64ebe261104682aa19fda75be4fa8f`.
Forward/reverse destination and duplicate-proposal contribution order produces
these exact same bytes.
```

Every outer/nested vector permutation is byte-identical after construction.
Duplicate raw proposal contributions union in
`FormMaterialAssociationBuilderV1`; duplicate direct final members passed to
`FormMaterialScopeV1::new`, +1 member beyond its bound, source/pair mismatch, or
a caller-supplied digest are rejected before reader call 1.

The exact query payload uses the section-6.1 length-delimited encoder and is:

```text
u16be(scope-tag=1)
bytes(composite_snapshot_id exact UTF-8)
digest32(configuration_catalog_set_digest)
digest32(registered_form_catalog_set_digest)
atomic_source_identity(analysis role=Analysis)
vec(atomic_source_identity(destination role=Destination), canonical unique)
digest32(pair_digest)
digest32(presence_key_digest)
digest32(form_material_scope_digest)
u16be(max_records)
```

`digest32` accepts exactly 64 lowercase ASCII hex characters and encodes the
decoded 32 bytes. Destination identities sort by their complete encoded bytes;
every destination has role/rank Destination, never an order-dependent ordinal.
The internal group vector is derived exactly as analysis followed by that
destination vector, so there is no second hidden group list to disagree with
the scope. The final query digest is
`H("unica.metadata-composite-query/v2", payload)` from section 6.1. Pair,
presence and Form-material digests are themselves domain-separated hashes over
their canonical sorted typed vectors; empty vectors encode an explicit zero
count rather than an absent field. `max_records` is part of the query/cache
identity: two otherwise equal queries with different local ceilings cannot
reuse one invocation outcome.

A source-local gap remains material only to conclusions that depend on that
group. The mandatory REDs are named:

```text
metadata_runs_once_for_composite_snapshot_and_form_runs_analysis_only
metadata_query_digest_changes_with_only_form_runtime_subject
metadata_query_digest_changes_with_only_pair_member
metadata_query_digest_changes_with_only_presence_member
metadata_query_digest_changes_with_only_max_records
metadata_query_digest_changes_with_only_catalog_set_digest
metadata_query_digest_changes_with_only_registered_form_catalog_set_digest
metadata_query_digest_golden_bytes_are_stable
inner_query_vectors_permute_but_duplicate_and_n_plus_one_reject
same_form_two_destinations_merge_once_before_bounds_and_read_once
same_form_second_proposal_preserves_query_and_provider_outcome
form_query_digest_changes_with_material_scope_or_max_records
form_query_digest_changes_with_only_source_fingerprint
form_query_digest_changes_with_only_catalog_digest
form_query_digest_changes_with_only_registered_form_catalog_digest
```

FormSourceSet uses
`H("unica.form-inspection-query/v2", u16be(scope-tag=2) ||
atomic_source_identity(Analysis) ||
fingerprint32(analysis_source_fingerprint) ||
digest32(analysis_configuration_catalog_digest) ||
digest32(analysis_registered_form_catalog_digest) ||
digest32(form_material_scope_digest) || u16be(max_records))`.
`analysis_source_fingerprint` is the exact captured snapshot fingerprint and
`analysis_configuration_catalog_digest` is the matching borrowed catalog digest.
`analysis_registered_form_catalog_digest` is the exact matching analysis
sidecar-catalog digest from the same composite context. All three are separate from
logical `AtomicSourceIdentityV2`; they bind query/cache
freshness without changing source/group ordering. The query therefore cannot reuse a
missing-material scope from another snapshot or proposal/runtime subject even
when the registered Form set is equal.

The normative nonempty query goldens use the section-6.1 Analysis source fixture
(`name="analysis"`, Configuration, PlatformXml, root `.`, mapping digest
`sha256:` + `a`*64) and the one-member fixture above:

```text
MetadataComposite:
  composite_snapshot_id = "sha256:" + ("e" * 64)
  configuration_catalog_set_digest = "c" * 64
  registered_form_catalog_set_digest = "d" * 64
  destinations = [patch]
  pair_digest = 0dd462387baf8fd7f7a55b8eb83a47dfbe22b2f3bac28b932410571ab22eaed4
  presence_key_digest = e1edb517a8e4e4188543c23e3c5f6048cb8719c02048f8dc4795f50df257497b
  form_material_scope_digest = 7f2411f23ec2f4bddcd5fefef5f3ef77badeeb53722dc28272d2417cc99f58d1
  max_records = 7
  payload length = 544
  SHA-256(payload) =
    65e6fed0a472f469256546211a9742c302285bacc95173c7f0078a012e770678
  H("unica.metadata-composite-query/v2", payload) =
    264efe11644aaea7528f33b05269b1912b5fe358258303ec848d030ceec0c361

FormSourceSet:
  analysis_source_fingerprint = "sha256:" + ("b" * 64)
  analysis_configuration_catalog_digest = "c" * 64
  analysis_registered_form_catalog_digest = "d" * 64
  form_material_scope_digest = 7f2411f23ec2f4bddcd5fefef5f3ef77badeeb53722dc28272d2417cc99f58d1
  max_records = 7
  payload length = 280
  SHA-256(payload) =
    2233ce6e902f793f8c20402261b1f4b9eda5f0b80b21aa73d725c87b73ad931e
  H("unica.form-inspection-query/v2", payload) =
    77855673179fe925318f5873863daa2ba34020c849a57ff9f44b46c13c39abcf
```

These fixed values are changed only with their encoder version.

### 4.2 Exact future Support query seam

Task 5B exports the shared identity/key primitives needed by the downstream
`Task5C-Evidence` Support provider. The provider owns one exact query identity:

```text
SUPPORT_STATE_QUERY_ENCODER = "support-state-query/v2"
PLANNED_DESTINATION_ABSENT_ENCODER = "planned-destination-absent/v1"
SUPPORT_SUBJECT_SEMANTIC_AUTHORITY_ENCODER =
  "source-free-support-subject-authority/v2"
SUPPORT_SUBJECT_SNAPSHOT_AUTHORITY_ENCODER =
  "support-subject-snapshot-authority/v2"
SUPPORT_LOOKUP_UUID_POLICY = "support-lookup-uuid/v2"
MAX_SUPPORT_QUERY_SUBJECTS = 4096

DestinationMetadataAbsenceV1 {
  pair_key,
  destination_source_set,
  registered_artifact,
  configuration_flavor = ExtensionConfiguration,
  catalog_digest,
}

PlannedDestinationAbsentV1 {
  pair: DestinationMembershipPair,
  analysis_authority: BaseOwnedMetadataIdentityV1,
  destination_absence: DestinationMetadataAbsenceV1,
  analysis_presence_fact_digest: Digest32,
  analysis_authority_fact_digest: Digest32,
  destination_absence_fact_digest: Digest32,
  analysis_source_fingerprint: SourceFingerprintV1,
  destination_source_fingerprint: SourceFingerprintV1,
  catalog_set_digest: Digest32,
  provenance_evidence_ids: Vec<EvidenceId>, // canonical union of all witnesses
}

SupportSubjectAuthorityV2
  Existing {
    object_key: PlatformConfigurationObjectKeyV1,
    lookup_uuid: SupportLookupUuidAuthorityV2,
  }                                                       tag 1
  PlannedDestinationAbsent {
    authority: PlannedDestinationAbsentV1,
    lookup_uuid: SupportLookupUuidAuthorityV2,
  }                                                       tag 2

SupportStateQueryV2<'context> {
  composite_snapshot_id,
  catalog_context: &'context PlatformCatalogContextV1,
  execution_binding: PlatformCatalogExecutionBindingV1,
  groups: Vec<SupportSourceGroupV2>,
}

SupportSourceGroupV2 {
  source: AtomicSourceIdentityV2,
  catalog_digest,
  subjects: Vec<SupportQuerySubjectV2>,
}

SupportQuerySubjectV2 {
  source_scoped_artifact,
  authority: SupportSubjectAuthorityV2,
  semantic_authority_digest: SupportSubjectSemanticAuthorityDigestV2,
  snapshot_authority_digest: SupportSubjectSnapshotAuthorityDigestV2,
}

SupportLookupUuidAuthorityV2
  Known { uuid: PlatformUuid, basis: SupportLookupUuidBasisV2 } tag 1
  Inconclusive(SupportLookupUuidProblemV2)                     tag 2

SupportLookupUuidBasisV2
  BaseOwnCatalogWrapper    tag 1
  PlannedBaseOwnCatalog    tag 2

SupportLookupUuidProblemV2
  UnsupportedRootMetadataKind       tag 1
  NestedArtifactMappingUnproven     tag 2
  ExtensionOwnMappingUnproven       tag 3
  ExtensionAdoptedMappingUnproven   tag 4
  CatalogAuthorityInconclusive      tag 5
```

`DestinationMetadataAbsenceV1` is not a CFE companion fact. Its private
constructor consumes one exact requested pair, the borrowed known
ExtensionConfiguration catalog and one complete exact MetadataAbsent record for
the destination half. It rejects a covering same-half gap, any membership
companion, source/artifact/catalog mismatch or unqueried pair. This preserves
the section-7 rule that `Absent + companion` is invalid.

`PlannedDestinationAbsentV1` is a neutral Task 5B/domain authority exported for
Task5C-Evidence; Task 5B never imports Task 5C. Its private constructor consumes
exactly two complete `CfePairHalf` semantic atomic groups from one validated
composite invocation: the analysis half must contain both MetadataPresent and
its whole BaseOwnedMetadataIdentity companion, while the destination half must
contain MetadataAbsent and no membership companion. These two groups carry the
three required fact variants. It requires
BaseConfiguration+Own, ExtensionConfiguration+Absent, equal pair/artifact,
freshness equal to each borrowed catalog, membership in the exact catalog set,
and no material pair gap. It recomputes all three ordinary source-bound
ProviderFact digests and validates every physical record in both groups.
The diagnostics-only provenance vector is the canonical sorted union of every
retained witness evidence ID from both complete groups, not one arbitrarily
selected ID per semantic fact. It is nonempty per group, duplicate-free, and
bounded by the checked sum of the two already-bounded group record counts
(never above `2 * u16::MAX`); caller-supplied additions/omissions are rejected.

This type is **query-construction authority only**. It authorizes asking the
destination support policy about the exact analysis Base/Own wrapper UUID while
also asserting that the destination metadata object is absent. It never asserts
destination existence/Present, Own/Adopted membership, `ExtensionOwned`,
patchability/actionability, mutation eligibility or receipt eligibility, and it
has no conversion into any of those domain types. The planned Support branch may
only produce missing-policy, configuration-read-only or object-not-listed safe/
advisory rows; it can never emit Editable, Locked or Removed. A later
`ExtensionRequired` advice is Task5C projection and requires all retained
Metadata+Support groups plus separate destination policy/safety checks; neither
this token nor its authority index can create that conclusion alone. Compile-
fail/misuse REDs reject conversion to destination ownership/mutation/receipt
types and a runtime RED drops any one required group and obtains Unknown.

Its exact bytes are:

```text
PlannedDestinationAbsentIdentityBytesV1 =
  u16be(schema=1)
  || bytes(DestinationMembershipPairIdentityBytesV2)
  || u16be(analysis flavor BaseConfiguration=1)
  || u16be(analysis membership Own=1)
  || uuid(analysis object UUID)
  || u16be(destination flavor ExtensionConfiguration=2)
  || u16be(destination polarity Absent=1)
  || digest32(catalog_set_digest)
  || fingerprint32(analysis_source_fingerprint)
  || fingerprint32(destination_source_fingerprint)
  || digest32(analysis_presence_fact_digest)
  || digest32(analysis_authority_fact_digest)
  || digest32(destination_absence_fact_digest)

planned_destination_absence_digest =
  H("unica.planned-destination-absent/v1",
    PlannedDestinationAbsentIdentityBytesV1)
```

The planned golden uses the Analysis/`patch` source fixtures, pair artifact
`MetadataObject "Catalog.Σ"`, Base/Own UUID
`11111111-1111-4111-8111-111111111111`, catalog-set digest `d`*64,
analysis/destination fingerprints `sha256:` + `b`*64 / `sha256:` + `f`*64,
and the three fact digests `1`*64, `2`*64, `3`*64:

```text
DestinationMembershipPairIdentityBytesV2 length = 325
SHA-256(pair bytes) =
  b14ea2dbf586a01690fe873897e6cf45c5d0b097fa239bee16a8c91e924878ff
PlannedDestinationAbsentIdentityBytesV1 length = 571
SHA-256(payload) =
  5d3f950a9b60e1893e5d07369202e7d56fc29ee4fdf2147fbeab829b9c80711a
planned_destination_absence_digest =
  af0951bbfabf2d6a9409d9eac0ad14f8004a094f55372cc25fb6b80bb1a9aae8
source-free planned semantic authority digest (Known basis tag 2) =
  fae54a98946ac31d80614c863e380093f57f9834ab0f92d6191dbebba9aa9e2e
```

An evidence ID is exact `ev_` followed by 64 lowercase ASCII hex characters.
The three fact digests are exact raw 32-byte values after their validated
`sha256:` transport prefix is removed. `SourceFingerprintV1` is instead the
validated exact `sha256:<64 lowercase hex>` transport value consumed by
`fingerprint32`; it is not a raw `Digest32`. The canonical union of provenance
IDs validates and audits every physical witness in both complete half-groups,
but is diagnostics-only and deliberately excluded from
both semantic and snapshot authority digests. No preliminary authority survives as a
final ownership fact after Task 7 admission drops either required half-group;
downstream must rebuild the final authority from retained evidence.

The authority has two deliberately different projections. The semantic
projection preserves policy meaning but excludes outer source/subject, pair,
catalog/composite/fingerprint/fact/evidence identity. The snapshot projection
binds the exact query authority and freshness:

```text
SupportLookupUuidAuthorityBytesV2 =
    u16be(Known=1) || uuid(uuid) || u16be(basis tag)
  | u16be(Inconclusive=2) || u16be(problem tag)

SourceFreeSupportSubjectSemanticAuthorityBytesV2 =
  u16be(authority tag)
  || [Existing:
        flavor_authority(
          u16be(Known=1) || u16be(Base=1 | Extension=2)
          | u16be(Inconclusive=2) || u16be(flavor problem tag))
        || u16be(metadata kind tag)
        || uuid(catalog entry wrapper UUID)
        || membership_authority(
             u16be(Own=1)
             | u16be(Adopted=2) || uuid(extended UUID)
             | u16be(Inconclusive=3) || u16be(membership problem tag))
      | PlannedDestinationAbsent:
        u16be(BaseConfiguration=1) || u16be(Own=1)
        || uuid(analysis object UUID)
        || u16be(ExtensionConfiguration=2) || u16be(Absent=1)]
  || SupportLookupUuidAuthorityBytesV2

support_subject_semantic_authority_digest =
  H("unica.source-free-support-subject-authority/v2",
    SourceFreeSupportSubjectSemanticAuthorityBytesV2)

SupportSubjectSnapshotAuthorityIdentityBytesV2 =
  u16be(authority tag)
  || SourceScopedArtifactIdentityBytesV2(subject)
  || bytes(composite_snapshot_id)
  || digest32(catalog_set_digest)
  || [bytes(PlatformConfigurationObjectKeyBytesV1(existing key))
      | digest32(planned_destination_absence_digest)]
  || SupportLookupUuidAuthorityBytesV2

support_subject_snapshot_authority_digest =
  H("unica.support-subject-snapshot-authority/v2",
    SupportSubjectSnapshotAuthorityIdentityBytesV2)
```

For Existing, the semantic flavor, metadata kind, wrapper UUID and membership
values are resolved from the exact keyed catalog entry, not accepted separately
from a caller. The wrapper UUID is object identity; the lookup UUID is a
separate policy result and cannot substitute for it, especially when lookup is
Inconclusive. For
Planned, they come from the validated pair authority. The digest fields on
`PlannedDestinationAbsentV1` and `SupportQuerySubjectV2` are read-only
constructor results. `ProviderFact::Support` is back-propagated to carry one
private `SupportFactAuthorityBindingV2 { semantic_authority_digest,
snapshot_authority_digest, query_digest }` beside subject/state. Its source-free
tag-9 payload and semantic group use only the semantic digest. Its normal
whole-fact digest, response-authority validation and Support-specific physical-
record suffix use both the snapshot digest and the exact enclosing
`SupportStateQueryV2.query_digest`. The constructor receives a borrowed
smart-constructed query entry; callers cannot attach any of these three digests
independently. A local Task5C custom group encoder is forbidden.

Closed Support-query construction failures are:

| Tag | Exact reason |
| ---: | --- |
| 1 | `support_query_empty` |
| 2 | `support_query_subject_limit` |
| 3 | `conflicting_support_subject_authority` |
| 4 | `support_source_group_mismatch` |
| 5 | `support_catalog_resolution_mismatch` |
| 6 | `support_composite_snapshot_mismatch` |
| 7 | `support_lookup_uuid_authority_mismatch` |
| 8 | `planned_destination_absence_authority_mismatch` |
| 9 | `exact_artifact_spelling_collision` |

An empty semantic worklist skips the provider and does not attempt to construct
the query. If a constructor is called with an empty group/query it returns tag
1. After that empty/basic typed-input check and before any identity encoding,
grouping, set insertion, sorting, deduplication, authority comparison or the
4,096 semantic-subject count, the constructor creates one shared
`ExactArtifactSpellingRegistryV1::empty_v1()` and calls
`validate_occurrence` for every raw source-qualified subject artifact with its
exact `AtomicSourceIdentityV2`. A same-source semantic-equal/exact-different
pair rejects the whole constructor in either order as tag 9; a byte-identical
repeat remains legal, and an equal semantic identity under a different source
is independent. This validation introduces no raw-cardinality cap: 4,097
byte-identical raw contributions are all visited and may still collapse to one
semantic subject.

Only after the complete raw spelling walk, and before the 4,096 count, group raw requests by complete
`SourceScopedArtifactIdentityBytesV2`: byte-identical semantic+snapshot
authority duplicates canonicalize to one subject; any differing semantic or
snapshot authority for the same subject returns tag 3. Thus two proposals may
safely request one object without doubling the provider subject, while a stale/
conflicting authority cannot substitute. Proposal/Request/Mechanism associations
are not fields of this provider constructor or query. Task 7 owns a separate
material-association map and unions its scopes when reusing the one immutable
invocation.

The smart constructor borrows the whole context, mints its one private
`PlatformCatalogExecutionBindingV1`, resolves every existing-object
key against exactly one matching catalog/artifact, recomputes all catalog and
planned-absence digests, and rejects a source/group/catalog/composite mismatch.
The group authority is the exact `(AtomicSourceIdentityV2, catalog_digest)` pair
resolved through that borrowed catalog set. There is deliberately no synthetic
"Configuration" `ArtifactRef` or configuration-root object key: the catalog is
per registered object, while physical `ParentConfigurations`/Configuration-root
material remains capture-manifest authority rather than a fake catalog entry.
Groups sort by complete `AtomicSourceIdentityV2`; subjects sort by complete
`SourceScopedArtifactIdentityBytesV2`. Existing and planned authority tags are
1 and 2. Every group and subject vector is nonempty; an empty Support plan makes
no provider invocation. The union is bounded at 4,096 subjects before I/O and
is independent of public `maxEvidence`: there is no local lossy Support ceiling.

Permanent constructor REDs cover ASCII case aliases and expanding `İ` aliases
in both orders, byte-identical duplicates, equal identities under different
sources, collision plus conflicting authority (spelling reason wins), and
4,097 byte-identical raw contributions collapsing to one subject before the
4,096 semantic bound. A static RED rejects any Support grouping/deduplication
or identity-byte construction before the complete raw spelling walk. Each
isolated spelling variant preserves the published Support query bytes/digest.

`SUPPORT_LOOKUP_UUID_POLICY = "support-lookup-uuid/v2"` is deliberately
smaller than the catalog's registration grammar. The tracked 337-byte
`aaaa/111/222/333` `ТестВендор` material is synthetic parity
compatibility data introduced by a test-corpus sync; it is not a Designer
export, donor manifest or primary platform proof. V2 therefore labels its two
Known Catalog rows as an explicit current-product compatibility policy, not a
platform-wide inference: exact BaseConfiguration + Own Catalog wrapper, or the
same exact BaseConfiguration + Own Catalog UUID carried by a validated planned-
destination pair. Configuration-root UUID is only a group invariant from the
catalog header and is not a query subject. Every other root kind, nested
Form/Template/Command subject, extension
Own/Adopted mapping, or inconclusive flavor/membership yields the typed
Inconclusive reason above and a scoped Unknown; no wrapper UUID is guessed.
Adding a row requires verified primary/donor provenance or an explicitly
approved compatibility row, a policy-version bump and new goldens. This
restriction exposes a real current coverage limit rather than
silently treating every metadata wrapper UUID as a ParentConfigurations key.

The exact payload is:

```text
u16be(schema=2)
|| bytes(composite_snapshot_id exact UTF-8)
|| digest32(catalog_set_digest)
|| vec(groups sorted unique:
     bytes(AtomicSourceIdentityV2)
     || digest32(catalog_digest)
     || vec(subjects sorted unique:
          SourceScopedArtifactIdentityBytesV2(subject)
          || digest32(support_subject_snapshot_authority_digest)))

query_digest = H("unica.support-state-query/v2", payload)
```

There is no source-set display string, arbitrary caller digest, provider order,
workspace epoch or `maxEvidence`. `ScopedProviderInvocation.query_digest` in
Task 7 is this exact value byte-for-byte; Task 7 does not wrap or rebuild it.

The normative Known Existing-authority golden reuses the Analysis fixture and
uses `MetadataObject "Catalog.Σ"`, exact BaseConfiguration + Own catalog
authority, catalog-entry wrapper UUID and lookup UUID both
`11111111-1111-4111-8111-111111111111`, basis
BaseOwnCatalogWrapper, composite snapshot `sha256:` + `e`*64, catalog-set digest
`d`*64 and catalog digest `c`*64. The object is the requested subject; there is
no configuration-root object key:

```text
SourceFreeSupportSubjectSemanticAuthorityBytesV2 length = 94
support_subject_semantic_authority_digest =
  eae37f61ece01911f5745282d56188aeec1b9ae1975e930de2c067400feac13c
SupportSubjectSnapshotAuthorityIdentityBytesV2 length = 373
support_subject_snapshot_authority_digest =
  996106669201f3efac3be0dce633dd118c3668156030e3dd313df755658ccfbd

SupportStateQueryV2 payload length = 501
SHA-256(payload) =
  3307d5c1fc7b7999d1227ba2e2e86e4abe21cdeaa44376945a424bb91234e843
H("unica.support-state-query/v2", payload) =
  4b19743c0dabab91c42daeddf6ae2eb734738263f94ea6279412d28bbf79068b

ProviderFact::Support(Editable=1) source-free payload length = 36
source_free_typed_payload_digest =
  30549c54bfe950984605a47e31724b94710c6b06221105b0bc79e3e45c5633b6
SupportStateObservation secondary_digest =
  1199d03d35a08c335a7fb3165213e17c2188edffc039c3356e5930e8c3f91970
source_free_semantic_cluster_digest =
  4eb0395901517d21c33a938002ae1b7a89aefb509074438817ae55e240467826
complete group-key length = 387
SHA-256(complete group-key bytes) =
  d7af274937dd106d65d4bac2daffe21e2baae496df47ea8d786ac6e817ad47fa

AtomicPhysicalRecordV2 with location=None, port=SupportState(6),
provider="unica.support_state", version="2", Complete and source fingerprint
`sha256:` + `b`*64, snapshot authority digest above and exact enclosing Support
query digest `4b19743c...79068b` has length 329 and digest =
  d296ae3f5be4695fe6f4988ba523cf59ff6dc2a4040dbe3b1623ee24fbef3570
```

An Existing authority with flavor Inconclusive(problem tag 1), Catalog tag 28,
wrapper UUID `11111111-1111-4111-8111-111111111111`, membership
Inconclusive(problem tag 4), and lookup Inconclusive(problem tag 5) has
semantic-authority payload length 56 and digest
`867f2e8b1d3944f8b4aad7ac9d465e2863b9e7887160201f8453dbf48fbf3c22`.
It yields a scoped Unknown/gap and no Support fact, but its exact typed authority
and query identity remain constructible and deterministic.

Changing only the wrapper UUID changes semantic/snapshot/query authority and
cannot be masked by an equal Known lookup UUID. Changing only the enclosing
Support query digest preserves semantic/group order but changes the normal fact,
physical-record/evidence and analysis identities; replaying a record under a
different Support round/query is rejected.

Changing only composite snapshot identity, catalog-set digest, group catalog,
full source identity, subject artifact or authority payload changes the query
digest. Input permutation does not. A stale catalog/fingerprint authority fails
construction rather than receiving a reusable query identity.
The constructor RED `support_query_rejects_synthetic_configuration_artifact`
proves that no unsupported Configuration-root `ArtifactRef` can be invented to
satisfy group authority.

Any remaining Task 7 statement or corpus expectation requiring separate
analysis/destination Metadata calls is superseded and blocks implementation.

## 5. Shared Platform XML parser and exact namespace

Task 4 capture and Task 5B semantic views use one parser family in neutral
`infrastructure/platform_xml.rs` or an adjacent neutral module. There is no
discovery-only registration parser.

### 5.1 Exact MDClasses namespace

The default namespace URI for Platform metadata structure is:

```text
MDCLASSES_NS = "http://v8.1c.ru/8.3/MDClasses"
```

Prefix spelling is arbitrary and nonsemantic. Namespace URI is semantic. Every
MDClasses structural element consumed by capture or a metadata semantic view
must have the exact expanded name `{MDCLASSES_NS}LocalName`, including
`MetaDataObject`, the object-kind element, `Properties`, `ChildObjects`,
registrations, capability properties, membership fields, and MDClasses
mechanism fields. A cross-namespace child is legal only when a closed field
registry gives its exact expanded name; v1's material example is the direct
data-core Type child of EventSubscription Source in section 8. A local-name-only
test can never turn such an exception into a wildcard.

The following are mandatory rules:

- local-name-only comparison is forbidden;
- `<m:MetaDataObject xmlns:m="urn:1c">` is invalid, not accepted;
- a same-local direct child in another namespace cannot satisfy cardinality;
- a foreign same-local/binding-shaped direct child in the capture-authoritative
  envelope/registration grammar (`MetaDataObject`, `Configuration`,
  `ChildObjects`, registered kind/name/UUID structure) is capture-fatal
  `foreign_metadata_namespace`; in particular a direct foreign same-local
  `Configuration` never reaches catalog authority;
- inside one already capture-valid exact-namespace `Configuration` or registered
  object, foreign same-local catalog-semantic discriminator fields do not satisfy
  an exact field and select the closed flavor/membership `WrongNamespace`
  problem; a namespaced same-local `uuid` attribute on that exact
  `Configuration` similarly selects root-UUID `WrongNamespace` under section
  5.4;
- the same defect in a mechanism-only descriptor view makes that descriptor view
  atomically Bounded with `foreign_metadata_namespace`; sibling descriptors
  survive;
- an arbitrary foreign child that is not binding-shaped cannot repair or shadow a
  required field and is ignored only in an explicitly extensible view;
- `uuid`, Form item `name`/`id`, event `name`/`callType` attributes are exact
  unqualified attributes. Namespaced lookalikes never count as values; the
  closed catalog authority still records the reachable typed namespace problem
  specified above.

The current live parser test that accepts `urn:1c` must become a RED and then be
replaced by an arbitrary-prefix exact-URI positive test.

### 5.2 Two phases and safety

```text
capture_authority(bytes, role, CaptureXmlLimits)
  -> CaptureValidatedEnvelopeV1

semantic_views(capture_envelope, requested_views, ProviderSemanticLimits)
  -> typed views | scoped semantic failures
```

Before any DOM allocation, capture streaming-validates UTF-8/BOM, XML/QName and
namespace well-formedness, rejects DTD/entity declarations, enforces checked
depth/node limits, and validates every capture-authoritative registration,
descriptor kind/name, **registered-object identity UUID**, and nested Form/
Template/Command identity. The direct Configuration element's cardinality/QName
is capture-authoritative, but its root `uuid` attribute cardinality, namespace
and value are the reachable semantic catalog authority in section 5.4. Thus a
missing/invalid/namespaced root UUID may produce typed Inconclusive after
capture, while a missing/duplicate/foreign direct Configuration produces no
snapshot. Capture failure yields no manifest prefix and no provider invocation.

Task 5B rereads verified bytes, reruns the same capture guard, proves the result
matches Task 4's admitted catalog, and only then requests semantic views. A guard
disagreement on identical bytes is `platform_xml_parser_invariant`, not Bounded.

Exact limits:

```text
one present XML material     <= 64 MiB
XML nesting depth            <= 128
XML nodes per material       <= 1,000,000
identifier                   <= 512 UTF-8 bytes and 128 Unicode scalars
qualified handler/QName      <= 1024 UTF-8 bytes total
opaque Form item/command id  <= 256 UTF-8 bytes and 128 Unicode scalars
closed/unknown XML token     <= 256 UTF-8 bytes
```

A canonical identifier is nonempty and contains only Unicode alphanumeric
characters or underscore; exact registered spelling is retained. An opaque ID
is nonempty and contains no Unicode whitespace/control but otherwise stays an
exact token. A qualified handler/QName must also satisfy each segment's
identifier ceiling. Raw values beyond these bounds fail before they enter a
stable reason, set key, or digest.

Task 4 overflow on Configuration/root/nested capture material is snapshot-fatal.
Overflow in a present, capture-registered Form.xml semantic view is an exact Form
Bounded gap; other Forms and Metadata survive.

Scalars concatenate direct Text/CDATA chunks in document order; comments and PIs
contribute nothing. A direct child element makes the scalar mixed-content
malformed. Descendant text, attributes, wrappers, and same-local foreign nodes
never repair it.

### 5.3 Capture catalog and configuration flavor

`Configuration/ChildObjects` is closed by the versioned domain MetadataKind
registry. Every direct element must use `MDCLASSES_NS`, be a known kind, and have
a valid direct registration name. Known kinds outside the eight flows remain
registered and identity-validated. Unknown kinds and canonical duplicates are
capture-fatal.

Object UUID is exactly the one unqualified `@uuid` on the direct registered
object-kind element, never on `MetaDataObject` or a descendant. `PlatformUuid`
accepts exactly the non-nil 36-byte hyphenated ASCII-hex UUID shape, accepts hex
letter case, and canonicalizes it to lowercase before identity/digest. Braced,
compact, URN, padded, non-ASCII, or nil forms are rejected.

Configuration flavor is derived from direct exact-namespace properties:

| ObjectBelonging | ConfigurationExtensionPurpose | Flavor |
| --- | --- | --- |
| absent | absent | BaseConfiguration |
| exact Adopted | exact Patch, Customization, or AddOn | ExtensionConfiguration |
| any other/missing/duplicate combination | inconclusive flavor view |

`ConfigurationExtensionCompatibilityMode` and
`KeepMappingToExtendedConfigurationObjectsByIDs` are optional validated
non-discriminators. `NamePrefix` is independent `Missing|Empty|Value`. Declared
source kind never overwrites semantic flavor.

### 5.4 Neutral snapshot-bound configuration catalog authority

MetadataCatalog and the later SupportState adapter require the same exact UUID,
flavor and ownership authority, but infrastructure adapters must never call one
another. Freeze one pure shared result in neutral Platform XML infrastructure:

```text
PLATFORM_CONFIGURATION_CATALOG = "platform-configuration-catalog/v1"
PLATFORM_CONFIGURATION_CATALOG_ENCODER =
  "platform-configuration-catalog-encoder/v1"
PLATFORM_CONFIGURATION_CATALOG_SET = "platform-configuration-catalog-set/v1"
PLATFORM_CONFIGURATION_OBJECT_KEY_ENCODER =
  "platform-configuration-object-key/v1"
REGISTERED_FORM_CATALOG = "registered-form-catalog/v1"
REGISTERED_FORM_CATALOG_SET = "registered-form-catalog-set/v1"
PLATFORM_CONFIGURATION_CATALOG_CONTRACT_VERSION: u16 = 1
PLATFORM_CONFIGURATION_CATALOG_SET_CONTRACT_VERSION: u16 = 1
REGISTERED_FORM_CATALOG_CONTRACT_VERSION: u16 = 1
REGISTERED_FORM_CATALOG_SET_CONTRACT_VERSION: u16 = 1

PlatformConfigurationCatalogV1 {
  contract_version: u16 = PLATFORM_CONFIGURATION_CATALOG_CONTRACT_VERSION,
  source_role: Analysis | Destination,
  resolved_source_set: ResolvedSourceSet,
  source_fingerprint,
  capture_catalog_digest: PlatformConfigurationCaptureCatalogDigestV1,
  configuration_root_uuid: ConfigurationRootUuidAuthorityV1,
  configuration_flavor: CatalogConfigurationFlavorAuthorityV1,
  script_variant_authority: CatalogScriptVariantAuthorityV1,
  name_prefix_authority: CatalogNamePrefixAuthorityV1,
  entries: Vec<PlatformConfigurationObjectAuthorityV1>,
}

PlatformConfigurationObjectAuthorityV1 {
  artifact: ArtifactRef,
  metadata_kind: exact MetadataKind registry value,
  wrapper_uuid: PlatformUuid,
  membership: CatalogObjectMembershipAuthorityV1,
}

PlatformConfigurationCatalogSetV1 {
  contract_version: u16 = PLATFORM_CONFIGURATION_CATALOG_SET_CONTRACT_VERSION,
  composite_snapshot_id,
  catalogs: Vec<PlatformConfigurationCatalogV1>,
  catalog_set_digest: Digest32, // derived; not re-encoded into its payload
}

RegisteredFormCatalogV1 {
  contract_version: u16 = REGISTERED_FORM_CATALOG_CONTRACT_VERSION,
  source_role: Analysis | Destination,
  resolved_source_set: ResolvedSourceSet,
  source_fingerprint: SourceFingerprintV1,
  configuration_catalog_digest: Digest32,
  registered_form_catalog_digest: Digest32,
  entries: Vec<RegisteredFormAuthorityV1>,
}

RegisteredFormAuthorityV1 {
  owner: ArtifactRef,                    // exact registered top-level owner
  form: ArtifactRef(Form),               // exact nested registered Form
  wrapper_uuid: PlatformUuid,
  membership: CatalogObjectMembershipAuthorityV1,
  form_type: PlatformRegisteredFormTypeAuthorityV1,
  form_descriptor_manifest_key: RegisteredFormManifestKeyV1,
  managed_form_xml_material: RegisteredFormMaterialAuthorityV1,
  form_module_material: RegisteredFormMaterialAuthorityV1,
}

RegisteredFormMaterialAuthorityV1 {
  // private on every state; exact Task4 relationship identity, never only key
  capture_relationship: RegisteredMaterialRelationshipProjectionV1,
  state:
    NotApplicable                                         tag 1
    | Missing { manifest_key: RegisteredFormManifestKeyV1 } tag 2
    | Present {
        manifest_key: RegisteredFormManifestKeyV1,
        content_fingerprint: SnapshotLeafFingerprintV1,
      }                                                   tag 3
}

RegisteredFormCatalogSetV1 {
  contract_version: u16 = REGISTERED_FORM_CATALOG_SET_CONTRACT_VERSION,
  composite_snapshot_id,
  catalogs: Vec<RegisteredFormCatalogV1>,
  catalog_set_digest: Digest32, // derived; not re-encoded into its payload
}

PlatformCatalogContextV1 {
  composite_snapshot_id,
  configuration: PlatformConfigurationCatalogSetV1,
  registered_forms: RegisteredFormCatalogSetV1,
  configuration_witnesses: PlatformConfigurationCatalogWitnessSetV1,
  registered_form_witnesses: RegisteredFormCatalogWitnessSetV1,
  analysis_bsl_material_witnesses: AnalysisBslMaterialWitnessSetV1,
}

impl PlatformCatalogContextV1 {
  fn composite_snapshot_id(&self) -> &CompositeSnapshotIdV2;
  fn configuration_catalog_set_digest(&self) -> &Digest32;
  fn registered_form_catalog_set_digest(&self) -> &Digest32;
  fn execution_binding_v1(&self) -> PlatformCatalogExecutionBindingV1;

  fn stage_complete_catalog_spellings_v1(
    &self,
    baseline: &ExactArtifactSpellingRegistryV1,
  ) -> Result<StagedExactArtifactSpellingDeltaV1,
              ExactArtifactSpellingViolationV1>;

  // Infallible: a valid context contains exactly one analysis catalog.
  fn analysis_platform_catalog<'context>(
    &'context self,
  ) -> AnalysisPlatformCatalogViewV1<'context>;

  fn platform_catalog<'context>(
    &'context self,
    source: &AtomicSourceIdentityV2,
  ) -> Option<PlatformCatalogViewV1<'context>>;

  fn resolve_capture_material<'context, 'snapshot>(
    &'context self,
    snapshot: &'snapshot SourceSetSnapshotV2,
    form_view: &RegisteredFormAuthorityViewV1<'context>,
  ) -> Result<RegisteredFormMaterialResolverV1<'context, 'snapshot>,
              SourceReadError>;

  fn read_registered_platform_form_verified<'context, 'snapshot>(
    &'context self,
    source_reader: &dyn SourceSnapshotPort,
    snapshot: &'snapshot SourceSetSnapshotV2,
    form_view: &RegisteredFormAuthorityViewV1<'context>,
  ) -> Result<RegisteredPlatformFormVerificationV1<'context>,
              SourceReadError>;

  fn analysis_bsl_material_scan_plan<'context, 'snapshot>(
    &'context self,
    snapshot: &'snapshot SourceSetSnapshotV2,
  ) -> Result<AnalysisBslMaterialScanPlanV1<'context, 'snapshot>,
              SourceReadError>;

  fn read_analysis_bsl_material_verified<'context, 'snapshot>(
    &'context self,
    source_reader: &dyn SourceSnapshotPort,
    snapshot: &'snapshot SourceSetSnapshotV2,
    item: AnalysisBslMaterialScanItemV1<'context, 'snapshot>,
  ) -> Result<AnalysisBslMaterialVerificationV1<'context, 'snapshot>,
              SourceReadError>;
}

PlatformCatalogViewV1<'context> {
  // private context/catalog borrows for one exact Analysis or Destination source

  fn source_identity(&self) -> AtomicSourceIdentityV2;
  fn source_fingerprint(&self) -> &'context SourceFingerprintV1;
  fn configuration_catalog_digest(&self) -> &'context Digest32;
  fn configuration_root_uuid_authority(&self)
    -> &'context ConfigurationRootUuidAuthorityV1;
  fn configuration_flavor_authority(&self)
    -> &'context CatalogConfigurationFlavorAuthorityV1;
  fn script_variant_authority(&self)
    -> &'context CatalogScriptVariantAuthorityV1;
  fn name_prefix_authority(&self)
    -> &'context CatalogNamePrefixAuthorityV1;
  fn configuration_object(
    &self,
    artifact: &ArtifactRef,
  ) -> Option<PlatformConfigurationObjectAuthorityViewV1<'context>>;
  fn registered_form_catalog_contract_version(&self) -> u16;
  fn registered_form_catalog_digest(&self) -> &'context Digest32;
  fn registered_forms(
    &self,
  ) -> impl ExactSizeIterator<
         Item=RegisteredFormAuthorityViewV1<'context>> + '_;
  fn registered_form(
    &self,
    form: &ArtifactRef,
  ) -> Option<RegisteredFormAuthorityViewV1<'context>>;
}

PlatformConfigurationObjectAuthorityViewV1<'context> {
  // private context/catalog/entry borrows; no public constructor/fields

  fn artifact(&self) -> &'context ArtifactRef;
  fn metadata_kind(&self) -> MetadataKind;
  fn wrapper_uuid(&self) -> &'context PlatformUuid;
  fn membership(&self)
    -> &'context CatalogObjectMembershipAuthorityV1;
}

AnalysisPlatformCatalogViewV1<'context> {
  // private context/catalog borrows; no public constructor/fields

  fn source_identity(&self) -> AtomicSourceIdentityV2;
  fn source_fingerprint(&self) -> &'context SourceFingerprintV1;
  fn configuration_catalog_digest(&self) -> &'context Digest32;
  fn registered_form_catalog_contract_version(&self) -> u16;
  fn registered_form_catalog_digest(&self) -> &'context Digest32;
}

RegisteredFormAuthorityViewV1<'context> {
  // private context/catalog/entry borrows; no public constructor/fields

  fn owner(&self) -> &'context ArtifactRef;
  fn form(&self) -> &'context ArtifactRef;
  fn wrapper_uuid(&self) -> &'context PlatformUuid;
  fn membership(&self)
    -> &'context CatalogObjectMembershipAuthorityV1;
  fn form_type(&self)
    -> &'context PlatformRegisteredFormTypeAuthorityV1;
  fn managed_form_xml_material(&self)
    -> RegisteredFormMaterialAuthorityRefV1<'context>;
  fn form_module_material(&self)
    -> RegisteredFormMaterialAuthorityRefV1<'context>;
}

RegisteredFormMaterialAuthorityRefV1<'context> {
  // opaque private context/catalog/form/material borrow capability;
  // no public fields, constructors, state/key/relationship accessors or serde
}

RegisteredFormMaterialResolverV1<'context, 'snapshot> {
  // private borrow-only context/form plus Task4 handle pair;
  // no Clone/serde/raw constructor
  form_xml: RegisteredMaterialExpectationHandleV1<'snapshot>,
  form_module: RegisteredMaterialExpectationHandleV1<'snapshot>,

  fn expectation_for(
    self,
    material_ref: RegisteredFormMaterialAuthorityRefV1<'context>,
  ) -> Result<RegisteredMaterialExpectationHandleV1<'snapshot>,
              SourceReadError>;
}

// Private Task5B scan-plan/admission constants. Task6 imports no copy and owns
// no parallel counter.
MAX_BSL_FILES: usize = 20_000
MAX_BSL_TOTAL_BYTES: u64 = 512 * 1024 * 1024
MAX_BSL_FILE_BYTES: u64 = 16 * 1024 * 1024

AnalysisBslMaterialScanPlanV1<'context, 'snapshot> {
  // Exact private backing; none of these fields has a public accessor.
  context: &'context PlatformCatalogContextV1,
  snapshot: &'snapshot SourceSetSnapshotV2,
  entries: Vec<AnalysisBslMaterialScanEntryV1<'context, 'snapshot>>,
  plan_identity: AnalysisBslMaterialPlanIdentityV1,

  // No Clone/serde/raw constructor or reader/callback field.

  fn items(&self) -> impl ExactSizeIterator<
    Item=AnalysisBslMaterialScanItemViewV1<'_, 'context, 'snapshot>> + '_;

  fn select_all(&self) -> AnalysisBslMaterialSelectionV1;

  fn select_modules(
    &self,
    modules: &[ArtifactRef],
  ) -> Result<AnalysisBslMaterialSelectionV1,
              AnalysisBslMaterialSelectionErrorV1>;

  fn admit(
    self,
    selection: AnalysisBslMaterialSelectionV1,
  ) -> Result<AnalysisBslMaterialAdmissionCursorV1<'context, 'snapshot>,
              AnalysisBslMaterialSelectionErrorV1>;
}

AnalysisBslMaterialScanEntryV1<'context, 'snapshot> {
  // A borrow of the canonical owned context witness zipped with one fresh
  // Task4 handle that borrows the actual snapshot-stored index entry.
  witness: &'context AnalysisBslMaterialWitnessV1,
  capture_handle: CapturedAnalysisBslMaterialHandleV1<'snapshot>,
}

AnalysisBslMaterialPlanIdentityV1 {
  // Private checked context/snapshot/entry-vector identity. It has no byte,
  // digest, serde or caller-selected constructor and is not an encoder field.
}

AnalysisBslMaterialScanItemViewV1<'plan, 'context, 'snapshot> {
  // private borrow of one plan-owned item; no read capability
  fn kind(&self) -> AnalysisBslMaterialScanItemKindV1;
  fn module(&self) -> Option<&'context ArtifactRef>;
  fn diagnostic_location(&self) -> VerifiedBslSourceLocationV1;
}

AnalysisBslMaterialSelectionV1 {
  // private plan identity plus canonical sorted-unique item indices;
  // no public fields/constructor/Clone/serde
}

AnalysisBslMaterialSelectionErrorV1
  WrongPlan              tag 1
  NonCanonicalModules    tag 2
  DuplicateModule        tag 3
  ModuleNotInPlan        tag 4
  InvalidModuleKind      tag 5

AnalysisBslMaterialAdmissionCursorV1<'context, 'snapshot> {
  // consuming, non-Clone ExactSizeIterator; yields each retained selected item
  // once. The immutable terminal value is computed before iteration.
  fn terminal_limit(&self)
    -> Option<&AnalysisBslMaterialScanLimitV1>;
}

impl Iterator for AnalysisBslMaterialAdmissionCursorV1<'context, 'snapshot> {
  type Item = AnalysisBslMaterialScanItemV1<'context, 'snapshot>;
}

impl ExactSizeIterator
  for AnalysisBslMaterialAdmissionCursorV1<'context, 'snapshot>;

AnalysisBslMaterialScanItemKindV1
  Ordinary             tag 1
  RegisteredFormModule tag 2

AnalysisBslMaterialAdmissionV1
  Process        tag 1
  FileBytesLimit tag 2

AnalysisBslMaterialScanLimitV1
  FileCount {
    first_omitted_location: VerifiedBslSourceLocationV1,
  } tag 1, public reason `bsl_file_limit`
  | TotalBytes {
    first_omitted_location: VerifiedBslSourceLocationV1,
  } tag 2, public reason `bsl_total_bytes_limit`

AnalysisBslMaterialScanItemV1<'context, 'snapshot> {
  // private context/snapshot/witness plus one Task4 capture capability

  fn kind(&self) -> AnalysisBslMaterialScanItemKindV1;
  fn module(&self) -> Option<&'context ArtifactRef>;
  fn admission(&self) -> AnalysisBslMaterialAdmissionV1;
  fn diagnostic_location(&self) -> VerifiedBslSourceLocationV1;
}

AnalysisBslMaterialVerificationV1<'context, 'snapshot>
  Present(VerifiedAnalysisBslMaterialV1<'context, 'snapshot>) tag 1
  | Missing(VerifiedAnalysisBslMaterialMissingV1<'context>)   tag 2
  | NotApplicable(
      VerifiedAnalysisBslMaterialNotApplicableV1<'context>)   tag 3

VerifiedAnalysisBslMaterialV1<'context, 'snapshot> {
  // private module/context authority and already validated final backing
  module: &'context ArtifactRef,
  backing: VerifiedAnalysisBslMaterialBackingV1<'snapshot>,

  fn module(&self) -> &'context ArtifactRef;
  fn bytes(&self) -> &[u8];
  fn location_for_range(
    &self,
    start_byte: u32,
    end_byte_exclusive: u32,
  ) -> Result<VerifiedBslSourceLocationV1, SourceReadError>;
  fn cache_locator(&self) -> VerifiedBslCacheLocatorV1;
}

VerifiedAnalysisBslMaterialBackingV1<'snapshot>
  Ordinary(VerifiedCapturedBslMaterialBytesV1<'snapshot>) tag 1
  | RegisteredFormModule {
      material: VerifiedRegisteredMaterialBytesV1<'snapshot>,
      cache_locator: VerifiedBslCacheLocatorV1,
    } tag 2

VerifiedAnalysisBslMaterialMissingV1<'context> {
  fn module(&self) -> &'context ArtifactRef;
  fn diagnostic_location(&self) -> VerifiedBslSourceLocationV1;
}

VerifiedAnalysisBslMaterialNotApplicableV1<'context> {
  fn module(&self) -> &'context ArtifactRef;
  fn diagnostic_location(&self) -> VerifiedBslSourceLocationV1;
}

SourceReadError::RegisteredMaterialHandleMismatch
  public stable reason = "registered_material_handle_mismatch"
  retryable = false

PlatformXmlSourceSpanV1
  imported unchanged from the neutral section-3.11 capture contract

PlatformConfigurationManifestKeyV1 // opaque capture-owned normalized key

PlatformConfigurationCatalogWitnessSetV1 {
  composite_snapshot_id,
  catalogs: Vec<PlatformConfigurationCatalogWitnessV1>,
}

PlatformConfigurationCatalogWitnessV1 {
  source: AtomicSourceIdentityV2,
  source_fingerprint: SourceFingerprintV1,
  capture_catalog_digest: PlatformConfigurationCaptureCatalogDigestV1,
  configuration_catalog_digest: Digest32,
  configuration_manifest_key: PlatformConfigurationManifestKeyV1,
  configuration_content_fingerprint: SnapshotLeafFingerprintV1,
  configuration_root_span: PlatformXmlSourceSpanV1,
  configuration_properties_span: PlatformXmlSourceSpanV1,
  configuration_child_objects_span: PlatformXmlSourceSpanV1,
  root_uuid_spans: Vec<PlatformXmlSourceSpanV1>,
  flavor_spans: Vec<PlatformXmlSourceSpanV1>,
  script_variant_spans: Vec<PlatformXmlSourceSpanV1>,
  name_prefix_spans: Vec<PlatformXmlSourceSpanV1>,
  objects: Vec<PlatformConfigurationObjectWitnessV1>,
}

PlatformConfigurationObjectWitnessV1 {
  artifact_identity: ArtifactIdentityBytesV1,
  descriptor_manifest_key: PlatformConfigurationManifestKeyV1,
  descriptor_content_fingerprint: SnapshotLeafFingerprintV1,
  configuration_registration_span: PlatformXmlSourceSpanV1,
  object_root_span: PlatformXmlSourceSpanV1,
  object_properties_span: PlatformXmlSourceSpanV1,
  object_child_objects_span: PlatformXmlSourceSpanV1,
  name_spans: Vec<PlatformXmlSourceSpanV1>,
  wrapper_uuid_spans: Vec<PlatformXmlSourceSpanV1>,
  membership_spans: Vec<PlatformXmlSourceSpanV1>,
}

RegisteredFormCatalogWitnessSetV1 {
  composite_snapshot_id,
  entries: Vec<RegisteredFormDescriptorWitnessV1>,
}

// Private owned context sidecar for the Analysis atomic source. It is neither
// semantic catalog data nor a fingerprint/query payload.
AnalysisBslMaterialWitnessSetV1 {
  composite_snapshot_id: CompositeSnapshotIdV2,
  source: AtomicSourceIdentityV2,
  source_fingerprint: SourceFingerprintV1,
  configuration_catalog_digest: Digest32,
  registered_form_catalog_digest: Digest32,
  entries: Vec<AnalysisBslMaterialWitnessV1>,
}

AnalysisBslMaterialWitnessV1 {
  kind: AnalysisBslMaterialScanItemKindV1,
  module: Option<ArtifactRef>,
  admission_byte_length: Option<u64>,
  diagnostic_location: VerifiedBslSourceLocationV1,
  // None iff kind=Ordinary; Some iff kind=RegisteredFormModule.
  registered_form_module: Option<AnalysisRegisteredFormModuleWitnessV1>,
}

AnalysisRegisteredFormModuleWitnessV1 {
  form_identity: ArtifactIdentityBytesV1,
  // Complete owned Task5B-private FormModule state/key/fingerprint plus the
  // exact Task4 relationship projection; never exposed outside this module.
  material_authority: RegisteredFormMaterialAuthorityV1,
}

RegisteredFormDescriptorWitnessV1 {
  source: AtomicSourceIdentityV2,
  source_fingerprint: SourceFingerprintV1,
  configuration_catalog_digest: Digest32,
  registered_form_catalog_digest: Digest32,
  form_identity: ArtifactIdentityBytesV1,
  owner_object_key: PlatformConfigurationObjectKeyV1,
  owner_child_objects_span: PlatformXmlSourceSpanV1,
  owner_form_registration_span: PlatformXmlSourceSpanV1,
  form_descriptor_manifest_key: RegisteredFormManifestKeyV1,
  form_descriptor_content_fingerprint: SnapshotLeafFingerprintV1,
  form_root_span: PlatformXmlSourceSpanV1,
  form_properties_span: PlatformXmlSourceSpanV1,
  name_spans: Vec<PlatformXmlSourceSpanV1>,
  wrapper_uuid_spans: Vec<PlatformXmlSourceSpanV1>,
  membership_spans: Vec<PlatformXmlSourceSpanV1>,
  form_type_spans: Vec<PlatformXmlSourceSpanV1>,
}

PlatformConfigurationObjectKeyV1 {
  catalog_digest,
  artifact: ArtifactRef,
}

ConfigurationRootUuidAuthorityV1
  Known(PlatformUuid)                         tag 1
  Inconclusive(ConfigurationRootUuidProblemV1) tag 2

ConfigurationRootUuidProblemV1
  Missing                    tag 1
  Duplicate                  tag 2
  WrongNamespace             tag 3
  InvalidOrNil               tag 4

CatalogConfigurationFlavorAuthorityV1
  Known(ConfigurationFlavorV1)                    tag 1
  Inconclusive(CatalogConfigurationFlavorProblemV1) tag 2

CatalogConfigurationFlavorProblemV1
  MissingExtensionPurposeForAdopted      tag 1
  UnexpectedExtensionPurposeWithoutAdopted tag 2
  DuplicateDiscriminator                 tag 3
  UnsupportedDiscriminatorValue          tag 4
  WrongNamespace                         tag 5
  InvalidOptionalConfigurationProperty   tag 6

CatalogScriptVariantAuthorityV1
  Missing                                      tag 1
  Known(KnownScriptVariant::Russian|English)   tag 2
  Unknown(BoundedScriptVariantTokenV1)         tag 3
  Inconclusive(CatalogScriptVariantProblemV1)  tag 4

CatalogScriptVariantProblemV1
  DuplicateExactField            tag 1
  WrongNamespace                 tag 2
  MixedContent                   tag 3
  InvalidOrOverLimit             tag 4

CatalogNamePrefixAuthorityV1
  Missing                                   tag 1
  Empty                                     tag 2
  Value(BoundedNamePrefixV1)                tag 3
  Inconclusive(CatalogNamePrefixProblemV1)  tag 4

CatalogNamePrefixProblemV1
  DuplicateExactField            tag 1
  WrongNamespace                 tag 2
  MixedContent                   tag 3
  InvalidOrOverLimit             tag 4

CatalogObjectMembershipAuthorityV1
  Own                                                        tag 1
  Adopted { extended_configuration_object_uuid: PlatformUuid } tag 2
  Inconclusive(CatalogObjectMembershipProblemV1)              tag 3

CatalogObjectMembershipProblemV1
  DuplicateObjectBelonging          tag 1
  DuplicateExtendedObjectUuid       tag 2
  UnsupportedObjectBelonging        tag 3
  MissingExtendedUuidForAdopted     tag 4
  UnexpectedExtendedUuidForOwn      tag 5
  InvalidOrNilExtendedUuid          tag 6
  WrongNamespace                    tag 7

PlatformRegisteredFormTypeAuthorityV1 and
PlatformRegisteredFormTypeProblemV1
  imported unchanged from the neutral section-3.11 capture contract;
  no Task5B-local alias, duplicate tag table, or constructor exists
```

The registered-Form sidecar is produced by the **same single** neutral catalog
port and the same capture envelopes as the configuration catalogs. It is not a
second filesystem scan/parser. Capture supplies exact nested Form registration,
its unqualified wrapper UUID/membership fields, the direct exact-namespace
`Properties/FormType` authority, the already-validated top-level owner entry,
and capture-owned opaque manifest keys. `RegisteredFormManifestKeyV1` has a
single private Task5B constructor that accepts only Task4's validated
`SnapshotManifestKeyRefV1` and stores only its lossless
`SnapshotManifestKeyProjectionV1`. It has no constructor from raw spelling,
bytes, `ArtifactRef`, a suffix or a formatted directory name; Task5B performs no
normalization and cannot inspect the captured key. Thus neither
`<owner>/Forms/<form>` nor `Ext/Form.xml` is inferred downstream from canonical
ref text.

Every sidecar entry requires its owner to resolve uniquely in the exact bound
`PlatformConfigurationCatalogV1`, the Form ref to be the exact registered child
of that owner, one non-nil nested wrapper UUID, and the same full membership
defect-set authority used for top-level objects. Duplicate owner/Form identity,
UUID, manifest key or source/capture mismatch is capture/catalog failure. The
`form_descriptor_manifest_key` is always the unique nested Form descriptor
`.../Forms/<Form>.xml`; it is never the owner's descriptor key. Every registered
Form, including Ordinary and FormType-inconclusive descriptors, remains in the
catalog so absence cannot be misreported as “not registered”. Only exact
`Known(Managed)` may carry `Missing` or `Present` managed Form.xml material.
`Missing` retains the exact capture-owned expected key; `Present` additionally
stores its exact captured leaf fingerprint. The same closed material grammar
independently captures the exact managed FormModule leaf. Exact
`Known(Ordinary)` and `Inconclusive` must carry `NotApplicable` for **both**
materials, with no Form.xml/FormModule key or fingerprint: downstream code may
not invent managed `Ext/Form.xml` or `Ext/Form/Module.bsl` semantics for either.
Exact `Known(Managed)` requires Missing/Present independently for both leaves.
Any other authority/material combination is a catalog-construction error.
A Form query/handle resolves only through this sidecar. Canonical-ref/path
inference, top-level MetadataKind pretence, silent omission of Ordinary Forms,
or “registered because the file exists” is forbidden.

`RegisteredFormMaterialAuthorityV1` is one closed Task5B-private smart enum
reused internally by both positions. It is not exported: Task 6 and Task 8 never
import, name, borrow, match, downcast, serialize or observe this enum, its tags,
state, key, fingerprint or retained projection. Its private `Clone + Eq`
implementation exists only so the one catalog builder can store an exact-equal
owned FormModule authority in the non-self-referential Analysis-BSL witness
sidecar; it exposes no field or constructor. Its sole constructor belongs to Task5B's
`PlatformCatalogPort` and accepts only the borrowed Task4 capture handle/state;
Task4 never imports or constructs this Task5B type. Missing/Present require an exact opaque admitted key, Present additionally
requires the exact manifest leaf fingerprint, and NotApplicable carries no key.
There is no serde/raw-key/path-suffix constructor. The enclosing
`RegisteredFormAuthorityV1` constructor enforces the FormType/material matrix
and which capture relationship (Form.xml versus FormModule) supplied each key.

Every material authority also privately retains the exact Task4
`RegisteredMaterialRelationshipProjectionV1` created from the opaque relationship
ref that supplied its state; a manifest key alone is never the bridge. Task5B
cannot copy Task4's private `RegisteredMaterialRelationshipKeyV1` or reconstruct
an owner/Form/kind tuple. The projection is intentionally excluded from the
published `RegisteredFormCatalogIdentityBytesV1`; that payload already binds
semantic owner/Form, descriptor/material states and the source-bound catalog
header, and its existing goldens must not drift. During
`resolve_capture_material`, Task5B compares the retained projection to the fresh
matching handle projection and invokes Task4's sealed
`encode_source_local_identity_v1` with the enclosing source/fingerprint. The
method validates that complete private binding and emits only the private
source-local owner-key/Form-key/kind comparison transcript. Task5B neither
duplicates a source header nor owns a relationship encoder. The projection
remains mandatory snapshot authority for context validation; stripping or
replacing it makes the value unresolvable.

The only Task6-facing catalog surface is the restricted context-bound header
view in the preceding block. `analysis_platform_catalog()` is infallible because
context construction already proved exactly one analysis catalog. Its source is
available only as an owned typed `AtomicSourceIdentityV2` validated projection
from the retained source role plus `ResolvedSourceSet`; no impossible
`'context` borrow of a computed temporary is exposed. The restricted analysis
header accessors expose only borrowed source fingerprint, configuration-catalog
digest, registered-Form-catalog digest and the numeric exact
`REGISTERED_FORM_CATALOG_CONTRACT_VERSION: u16 = 1`. It exposes no registered-
Form iterator/lookup, `RegisteredFormAuthorityViewV1`, internal catalog, entry
vector, raw digest input, identity bytes, manifest key, relationship or detached
DTO. Task 6 obtains the complete BSL surface only through the separate unified
scan-plan API.

The separate `platform_catalog(source)` lookup is the Task8/future-consumer
any-source entry point. It compares the complete typed
`AtomicSourceIdentityV2` against the canonical context source set and returns one
`PlatformCatalogViewV1<'context>` for the exact Analysis or Destination source,
or `None`; labels, rank alone and detached catalog digests cannot select a
catalog. In addition to the common fingerprint/digest/Form operations it exposes
the exact typed configuration root-UUID, configuration-flavor, ScriptVariant
and NamePrefix authorities plus exact object lookup. Object lookup returns a
lifetime-bound `PlatformConfigurationObjectAuthorityViewV1` containing only the
typed Artifact, MetadataKind, wrapper UUID and full Own/Adopted/Inconclusive
membership authority. Wrong-kind/absent objects return `None`; no display-name,
UUID-only or membership-only lookup exists. `analysis_platform_catalog()`
remains unchanged for Task6 and is an infallible restricted projection of that
same exact analysis catalog.

`PlatformCatalogViewV1<'context>`, `AnalysisPlatformCatalogViewV1<'context>`,
`PlatformConfigurationObjectAuthorityViewV1<'context>`,
`RegisteredFormAuthorityViewV1<'context>` and
`RegisteredFormMaterialAuthorityRefV1<'context>` have private fields, no public
constructor, `Serialize`/`Deserialize`, raw representation, `Clone`/`Copy` escape
or lifetime erasure. They can exist only while borrowing the originating
`PlatformCatalogContextV1`. A Form view exposes exact read-only owner, Form,
nested wrapper UUID, full typed membership, neutral FormType authority and the
two opaque material capabilities. UUID/membership access is semantic catalog
authority, not a material escape; it returns no key/path/relationship.
The material capability exposes no state, key, relationship, fingerprint or
catalog digest; it is useful only as the input to `expectation_for` on a resolver
created from the same semantic context and Form. Task8 passes its any-source Form
view only to `read_registered_platform_form_verified`; that context method owns
the opaque material-ref/resolver chain internally. Task8 never receives the
internal catalog entry, material enum/ref, relationship projection or Task4
expectation handle.

The context's composite-ID and two set-digest accessors are borrowed from the
same composite context and let Task8 bind a mutation preflight to the whole
Analysis+Destination authority. The derived set-digest fields are never encoded
back into their own payloads. No accessor exposes a catalog vector or permits a
caller to combine a set digest from one context with a view from another; every
Task8 smart constructor takes the whole context plus its lifetime-bound views
and checks all three values before material I/O.

`PlatformCatalogContextV1::resolve_capture_material` first verifies the exact
composite snapshot, source identity/fingerprint, configuration- and registered-
Form-catalog digests, the view's originating context, exact owner/Form identity,
both private relationship projections, their kind positions and their complete
NotApplicable/Missing/Present states against Task4's borrow-only manifest view.
It obtains the two handles through exactly two Task4
`resolve_registered_material_projection` calls, each `O(log N)` against the
private ordered relationship index. It never rescans the up-to-200,000 Form
iterator, so resolving all Forms cannot become `O(N^2)`. It compares immutable
semantic values, never an address or allocation identity, and performs this
validation before any I/O. Only then does it return the non-Clone lifetime-bound
pair resolver. `expectation_for` repeats exact semantic
context/catalog/Form/material-authority/kind equality before returning the
corresponding opaque Task4
`RegisteredMaterialExpectationHandleV1`; no raw expectation reference escapes.
Semantically different cross-snapshot, cross-source, cross-catalog,
cross-Form, cross-kind, relationship or state replay fails with the closed error
`SourceReadError::RegisteredMaterialHandleMismatch`, public reason
`registered_material_handle_mismatch`, retryable=false, before
`registered_material_verifier_calls` or any filesystem/byte read. An
independently reconstructed snapshot with exactly equal validated identity/
fingerprint/manifest bytes is accepted without pointer comparison. Task4
imports only the neutral `infrastructure::platform_xml` capture parser; this
Task5B-owned resolver imports Task4's port one-way, so no Task4 -> Task5B module
edge or alias is introduced.

### 5.4.1 One opaque Analysis BSL material scan plan

The context build also creates one private
`AnalysisBslMaterialWitnessSetV1` from the Analysis atomic snapshot's exact
Task4 `captured_analysis_bsl_materials()` projection. It is a bijectively
complete, nonsemantic witness set: every Task4 scan candidate maps to exactly
one context entry and every context entry maps back to exactly one candidate.
It adds no catalog/query/fingerprint encoder field and therefore changes none of
the eight Task4 expectation/source/composite fingerprint goldens or any
published Task5B catalog/query/group golden.

The set is an ordinary owned context field, not a lifetime-erased Task4 handle
store. Its header owns the exact composite ID, Analysis source identity/source
fingerprint and both matching catalog digests. Its canonical vector owns, for
each Task4-derived position, the exact kind, typed optional module, admission
length and opaque verified diagnostic location. Only a
`RegisteredFormModule` entry additionally owns
`AnalysisRegisteredFormModuleWitnessV1`: the exact Form identity and the
complete Task5B-private `RegisteredFormMaterialAuthorityV1`, including its
Task4 relationship projection. An Ordinary entry must carry `None`; a
RegisteredFormModule entry must carry `Some`. Construction rejects any other
matrix, duplicate module, non-strict order or cardinality mismatch. None of
these fields participates in semantic/catalog/query/fingerprint encoding, and
no Task6-facing accessor returns the set, an entry, its location backing,
material authority or relationship projection.

The builder creates the registered witness authority as an owned exact-equal
private clone of the FormModule authority during the same catalog transaction;
it never borrows another field of the context, so the returned context is not
self-referential. Opaque `VerifiedBslSourceLocationV1` cloning/comparison uses
only Task4's typed `Clone`/`Eq` implementation and yields no path/key bytes.

Every call to `analysis_bsl_material_scan_plan` obtains a fresh Task4
`captured_analysis_bsl_materials()` iterator from the supplied snapshot and
zips it position-for-position with the stored witness vector. It first checks
the complete header and exact length, then compares each fresh handle's kind,
module, admission length, opaque diagnostic authority and, for a registered
entry, exact retained FormModule relationship/state authority. The resulting
private `AnalysisBslMaterialScanEntryV1` borrows the canonical context witness
and owns that fresh handle, which itself borrows the actual Task4
snapshot-stored derived-index entry. The plan therefore has no self-reference,
temporary handle, raw manifest key/path, detached relationship DTO or Task6
escape. Any disagreement aborts the whole plan with nonretryable
`registered_material_handle_mismatch` before selection, admission or I/O.

For `RegisteredFormModule`, the builder requires the exact context-owned Form
entry, wrapper/Form membership, `FormType`, source fingerprint and retained
FormModule relationship projection. For every item it consumes Task4's sole
builder-whitelisted typed `module()` projection: `Some` must look up the equal
context-owned `ArtifactRef`, while `None` is valid only for an `Ordinary` item
and itself records the typed unsupported classification. Registered FormModule
with `None`, wrong kind/source or unequal context
lookup is nonretryable `registered_material_handle_mismatch` before a plan is
returned. The builder also revalidates Task4's unique `Some(ArtifactRef)`
projection; a duplicate/case-alias-equivalent module is the same mismatch, not
two selectable items. It never receives or parses a path string.
For pre-I/O admission it calls only Task4's builder-whitelisted
`admission_byte_length()`: `Some(exact captured length)` is required for either
Present kind and `None` for registered Missing/NotApplicable. The retained
registered-material witness distinguishes those two non-Present states without
exposing them to Task6. A claimed Present FormModule length must equal the
ordinary manifest entry it replaced; disagreement is nonretryable
`registered_material_handle_mismatch` before a plan is returned.
The builder converts each Task4 diagnostic anchor only through
`CapturedBslLocationRefV1::to_verified_location()`, a zero-I/O transition to the
opaque owned receipt authority; it never decodes or copies a key/path.
An ordinary item that merely resembles a registered FormModule but is not the
Task4 claim cannot be promoted; a captured unsupported ordinary BSL item remains
one visible `module() == None` item so Task6 emits
`unsupported_bsl_module_identity`. A Form-shaped decoy outside the accepted
capture produces no Task4 candidate, no witness, no item and no gap.

`analysis_bsl_material_scan_plan(context, snapshot)` is zero-I/O. Before
returning it validates that `snapshot` is exactly the context's Analysis atomic
source by complete source identity, fingerprint, configuration and
registered-Form catalog bindings, then validates complete witness/candidate
bijection and the Task4 canonical order. A Destination snapshot, different
context, omitted/duplicate/reordered candidate, changed Present claim or any
semantic projection/state mismatch returns nonretryable
`registered_material_handle_mismatch` before reader/verifier/filesystem calls.
Equal independently reconstructed semantic authority is accepted; pointer
identity is forbidden.

`items()` exposes only non-reading views in the one Task4-derived total order.
It never exposes a key, path, suffix, length/fingerprint tuple, Task4 handle or
reader argument. Task6 chooses scope through exactly one plan-owned selection:

- `select_all()` selects the complete ordered surface for CodeSearch and for
  CallGraph;
- `select_modules(modules)` accepts only a canonical sorted-unique vector of
  validated module `ArtifactRef`s and resolves it to an ordered unique
  subsequence of this plan. Wrong kind, absent module, duplicate, noncanonical
  order or a selection from another semantic plan returns the closed selection
  error before admission/I/O.

For Definition, and only Definition, this preserves the inherited Task6 order
`enumerate -> intersect final typed module scope -> apply resource limits ->
read`: unrelated ordinary or registered files cannot suppress a one-file
Definition selection. Task6 may not build a selection from item indices, raw
paths, separate Ordinary/Registered vectors or its own sort.

CallGraph is the explicit conservative exception to the base Task6 query-
subsequence optimization because its static target CommonModules are data-
dependent on parsing the selected callers. It calls `select_all()` exactly
once, admits exactly one merged canonical cursor and stores the admitted owned
items. It may read/parse queried callers first and then read only matching
stored static-target items; an unrelated `Process` item consumes the one global
admission budget but need not be read or parsed. There is no second plan,
counter, cursor, order, admission pass or reread for the target phase. A
terminal limit before a queried caller or before a referenced target yields the
exact deterministic caller-scoped Bounded gap and never a false Complete or
edge. Consequently an earlier unrelated material may conservatively suppress a
later CallGraph caller/target at a global terminal limit. A future two-phase
cursor may optimize this only if it preserves these semantics under a separately
reviewed contract; it is not part of v7.

`admit(self, selection)` consumes both capabilities, verifies their semantic
context/snapshot/plan identity, and computes one
`AnalysisBslMaterialAdmissionCursorV1`. Its
`terminal_limit()` is an immutable value computed before iteration; observing
it before, during or after exhaustion returns the same result. The cursor is
the sole authority for the inherited constants and exact semantics:

1. only selected Present materials consume `MAX_BSL_FILES`; a claimed Present
   FormModule is one file, never an ordinary file plus a registered file;
2. selected Missing/NotApplicable obligations consume zero files/bytes but keep
   their canonical relative position;
3. the first Present that would make file count 20,001 creates terminal
   `FileCount` before that item and omits the remaining selected suffix;
4. a Present above 16 MiB still consumes one file, is yielded once with
   `FileBytesLimit`, consumes zero total admitted bytes, performs zero read, and
   later selected items remain eligible;
5. for every other Present, checked addition to admitted bytes occurs in this
   one merged order; the first value above 512 MiB or arithmetic overflow creates
   terminal `TotalBytes` before that item and omits the remaining selected
   suffix; and
6. no Task6 loop owns a second counter or reapplies a bound per item kind.

Exact N/N+1 REDs cover each bound with Ordinary/Registered items on both sides
of the boundary, reversed input construction and a claimed FormModule at the
boundary. The resulting retained cursor, item admissions and terminal location
are byte-identical for every input permutation. A file-count/total terminal maps
to exact `bsl_file_limit`/`bsl_total_bytes_limit`; an item's
`FileBytesLimit` maps to exact `bsl_file_bytes_limit`. Task6 derives no reason
from a path or private state.

`read_analysis_bsl_material_verified` consumes one admitted item by value and
is the sole Task6 read operation. It rejects `FileBytesLimit`, unsupported
ordinary, cross-context/snapshot/selection replay and a repeated/forged item as
nonretryable `registered_material_handle_mismatch` before I/O. For an Ordinary
item it dispatches once to Task4's
`read_captured_bsl_material_verified`; for a RegisteredFormModule it privately
resolves the retained projection and dispatches once to
`read_registered_material_verified`. The exact counter matrix is:

| Plan item/result | registered verifier calls | ordinary byte reads | Task6 parses |
| --- | ---: | ---: | ---: |
| Ordinary Present | 0 | 1 | 1 |
| Registered Managed Present | 1 | 1 delegated inside registered read | 1 |
| Registered Managed Missing | 1 | 0 | 0 |
| Registered NotApplicable | 0 | 0 | 0 |
| unsupported Ordinary or `FileBytesLimit` | 0 | 0 | 0 |

The dispatcher maps the Task4 result without copying or exposing a state/key:
Present returns one `VerifiedAnalysisBslMaterialV1`; Missing and NotApplicable
return their opaque context/module/location tokens. A claimed Present
FormModule can never invoke the ordinary scan reader in addition to the
registered reader. Repeated query terms/methods/callers reuse the one consumed
item result inside the provider invocation.

Construction of a Present final wrapper is exact and contains no latent
fallible kind cast. The Ordinary branch accepts only
`VerifiedCapturedBslMaterialBytesV1`. The registered branch first proves that
the plan item, retained witness/relationship and Task4 result are all
`RegisteredFormModule`, then calls the common registered wrapper's fallible
`cache_locator()` with normal `?` propagation and stores the resulting opaque
`VerifiedBslCacheLocatorV1` in the private final backing before
`VerifiedAnalysisBslMaterialV1` exists. A FormXml wrapper, cross-kind replay or
failed capability projection returns the nonretryable
`RegisteredMaterialHandleMismatch`; there is no `unwrap`, panic, unchecked
cast, fabricated locator or partially constructed final wrapper. Consequently
the final wrapper's own `cache_locator()` may remain infallible: both of its
constructible private variants already contain a verified BSL cache capability.

Every item, including unsupported Ordinary, Missing and NotApplicable, provides
one opaque `diagnostic_location()` for a receipt-grade gap. Present additionally
provides `location_for_range(start_byte, end_byte_exclusive)`, which delegates
to Task4's exact verified-byte range validation and computes reproducible
1-based coordinates, plus an opaque `cache_locator()` accepted only by the
typed cache adapter. Neither capability exposes a reusable path/key/String or
can authorize a reader call. Task6 attaches `VerifiedBslSourceLocationV1`
directly to evidence/gaps and passes `VerifiedBslCacheLocatorV1` to the cache
adapter; only the whitelisted receipt/cache infrastructure projection may
serialize the contained location. A direct ordinary `read_verified`, suffix
test, manifest scan, FormModule resolver chain or raw cache path in Task6 fails
static/product tests.

`FileBytesLimit` is orthogonal to the optional typed module projection. Thus an
oversized unsupported Ordinary entry is still yielded with both
`module() == None` and `admission() == FileBytesLimit`; Task5B neither discards
one fact nor calls the dispatcher. Task6 owns the provider-gap precedence and,
under its frozen per-item table, handles the admission first as exact
`bsl_file_bytes_limit`. This API decision requires no Task4/Task5B reason
ranking and prevents an oversized unsupported item from becoming readable.

A mandatory compile-consumability fixture models Task6 from only the whole
context, exact Analysis atomic snapshot, injected reader and a canonical typed
module selection; it does not begin with a private catalog, Form authority,
manifest or Task4 handle:

```text
fn fake_task6_consumer<'context, 'snapshot>(
  context: &'context PlatformCatalogContextV1,
  snapshot: &'snapshot SourceSetSnapshotV2,
  source_reader: &dyn SourceSnapshotPort,
  scope: FakeTask6Scope<'_>,
) -> Result<FakeTask6ScanResult, FakeConsumerError> {
  let plan = context.analysis_bsl_material_scan_plan(snapshot)?;
  let selection = match scope {
    FakeTask6Scope::CodeSearch | FakeTask6Scope::CallGraph =>
      plan.select_all(),
    FakeTask6Scope::Definition { canonical_modules } =>
      plan.select_modules(canonical_modules)?,
  };
  let mut cursor = plan.admit(selection)?;
  let terminal = cursor.terminal_limit().map(FakeGap::from);
  let mut outcomes = Vec::with_capacity(cursor.len());
  for item in cursor {
    if item.module().is_none() ||
       item.admission() == AnalysisBslMaterialAdmissionV1::FileBytesLimit {
      outcomes.push(FakeTask6Outcome::Gap(item.diagnostic_location()));
      continue;
    }
    outcomes.push(FakeTask6Outcome::Verified(
      context.read_analysis_bsl_material_verified(
        source_reader, snapshot, item,
      )?
    ));
  }
  Ok(FakeTask6ScanResult { outcomes, terminal })
}
```

The fixture compiles using only the listed API and proves that selection,
admission and each read item remain context/snapshot-bound. A separate recording
CallGraph fixture stores this one `select_all()` cursor, reads queried callers
first and then only matching stored static-target items, without a second
selection/admission/counter/read pass. Compile-fail tests
reject direct internal-catalog/entry/plan/selection/item construction,
raw/detached digest inputs, a view/item/location outliving its authority, and
access to a Task4 handle/projection/key/path. Same-crate architectural denial of
a direct reader, second counter/order, suffix test, raw cache locator or
`RegisteredFormMaterialAuthorityV1` name is enforced by exact static/product
scans, not falsely described as Rust module privacy. Runtime tests supply
semantically different context/snapshot/selection/item capabilities and prove
every mismatch returns `registered_material_handle_mismatch` before
verifier/probe/read calls. A 200,000-Form plus ordinary-BSL recording fixture
proves one Task4 canonical iterator, indexed witness resolution and zero
Task6-side manifest/Form scans.

A separate compile-consumability fixture models the actual Task8
Analysis+Destination mutation preflight without private catalog/material
authority:

```text
fn fake_task8_pair_consumer<'context>(
  context: &'context PlatformCatalogContextV1,
  analysis_source: &AtomicSourceIdentityV2,
  destination_source: &AtomicSourceIdentityV2,
  analysis_snapshot: &SourceSetSnapshotV2,
  destination_snapshot: &SourceSetSnapshotV2,
  source_reader: &dyn SourceSnapshotPort,
  owner_ref: &ArtifactRef,
  form_ref: &ArtifactRef,
  method_ref: &ArtifactRef,
) -> Result<FakeTask8MutationAuthorityV1, FakeConsumerError> {
  let analysis = context.platform_catalog(analysis_source)
    .ok_or(SourceNotInContext)?;
  let destination = context.platform_catalog(destination_source)
    .ok_or(SourceNotInContext)?;

  require_base_configuration(analysis.configuration_flavor_authority())?;
  require_extension_configuration(destination.configuration_flavor_authority())?;
  require_known_script_variant(analysis.script_variant_authority())?;
  require_known_script_variant(destination.script_variant_authority())?;
  require_usable_destination_name_prefix(destination.name_prefix_authority())?;
  let roots = (
    require_known_configuration_root_uuid(
      analysis.configuration_root_uuid_authority(),
    )?,
    require_known_configuration_root_uuid(
      destination.configuration_root_uuid_authority(),
    )?,
  );

  let analysis_object = analysis.configuration_object(owner_ref)
    .ok_or(ObjectNotRegistered)?;
  let destination_object = destination.configuration_object(owner_ref)
    .ok_or(ObjectNotRegistered)?;
  require_analysis_own(analysis_object.wrapper_uuid(),
                       analysis_object.membership())?;
  require_destination_adopted_from(destination_object.wrapper_uuid(),
                                   destination_object.membership(),
                                   analysis_object.wrapper_uuid())?;

  let analysis_form = analysis.registered_form(form_ref)
    .ok_or(FormNotRegistered)?;
  let destination_form = destination.registered_form(form_ref)
    .ok_or(FormNotRegistered)?;
  require_known_managed(analysis_form.form_type())?;
  require_known_managed(destination_form.form_type())?;
  require_analysis_own(analysis_form.wrapper_uuid(),
                       analysis_form.membership())?;
  require_destination_adopted_from(destination_form.wrapper_uuid(),
                                   destination_form.membership(),
                                   analysis_form.wrapper_uuid())?;

  let analysis_verified = require_present(
    context.read_registered_platform_form_verified(
      source_reader, analysis_snapshot, &analysis_form,
    )?
  )?;
  let destination_verified = require_present(
    context.read_registered_platform_form_verified(
      source_reader, destination_snapshot, &destination_form,
    )?
  )?;
  let analysis_parse = parse_platform_form_v2(&analysis_verified)?;
  let destination_parse = parse_platform_form_v2(&destination_verified)?;
  require_plain(analysis_parse.document_flavor_authority())?;
  require_borrowed(destination_parse.document_flavor_authority())?;
  let analysis_binding = analysis_parse.method_bindings()?
    .lookup_method(&analysis_verified, method_ref)?;
  let destination_binding = destination_parse.method_bindings()?
    .lookup_method(&destination_verified, method_ref)?;

  FakeTask8MutationAuthorityV1::new(
    context.composite_snapshot_id(),
    context.configuration_catalog_set_digest(),
    context.registered_form_catalog_set_digest(),
    roots,
    analysis_object,
    destination_object,
    analysis_form,
    destination_form,
    analysis_parse.registered_form_authority(),
    destination_parse.registered_form_authority(),
    analysis_binding,
    destination_binding,
  )
}
```

The fixture cannot reach its smart constructor unless both source roles,
configuration flavors, typed ScriptVariant/NamePrefix authorities, object and
nested-Form wrapper UUID/membership joins, Known Managed FormType, Analysis
Plain/Destination Borrowed flavor authorities and both complete binding lookups
come from the same context-bound pair. A catalog digest alone cannot stand in
for any field. It performs one context-owned FormXml resolution/read per side
and one neutral parse per Present wrapper, with no second read/copy.

Compile-fail tests reject private field/token/result construction, raw bytes,
private material/projection access and a view/result outliving its context.
Because the neutral Present factory is honestly `pub(crate)`, same-crate calls
to that factory are rejected by the exact static/product call-site whitelist,
not claimed to be a Rust compile failure. Runtime spies reject role/snapshot,
foreign-context, cross-object, cross-Form, wrapper/extended-UUID, membership,
catalog, flavor and material swaps with
`registered_material_handle_mismatch` before reader/filesystem I/O. Task6 uses
only the analysis-BSL plan/dispatcher above; it imports neither this FormXml
result/parser nor the private Form material resolver chain.

Location witnesses are not discarded or smuggled into semantic digests. The
same port invocation creates opaque `PlatformConfigurationCatalogWitnessSetV1`
and `RegisteredFormCatalogWitnessSetV1` values in the composite context.
`PlatformConfigurationManifestKeyV1` is the same style of private capture-owned
normalized workspace-relative key as the Form key, with no raw/path-derived/
serde constructor. A configuration witness binds source fingerprint, capture
catalog digest, configuration-catalog digest, exact Configuration.xml key+leaf
fingerprint and all root/flavor/ScriptVariant/NamePrefix same-local spans. Each
object witness binds the exact catalog artifact identity, object descriptor key+
leaf fingerprint and all Name/wrapper-UUID/membership same-local spans. Its
`configuration_registration_span` is validated instead against the parent
configuration witness's Configuration.xml key/fingerprint and ChildObjects
container; one span is never attributed to the wrong file. It is the sole
location authority for BaseOwned/Extension,
Support, callback and configuration-property observations; no consumer reparses
Configuration/object XML.

Each private registered-Form witness is keyed by the complete source+
Form identity and binds the exact source fingerprint, both catalog digests,
the exact `PlatformConfigurationObjectKeyV1` of its owner plus the owner's
ChildObjects container and exact Form registration spans, and the nested Form descriptor key+leaf
fingerprint+root span. The context resolver first resolves that owner key to the
one `PlatformConfigurationObjectWitnessV1` and validates the child span against
its descriptor key/fingerprint; it does not duplicate or coerce a Form key into
a configuration key. Separate
Name, wrapper-UUID, membership and FormType vectors contain every same-local
contributing/defect observation and may be empty only for a complete absence
rule. Spans are 0-based half-open byte ranges validated against their named
exact bytes; `start < end <= verified_len`, UTF-8/node boundaries and sorted-
unique ordering are constructor invariants. Line/column rendering is derived
later and is not authority. Witness spans and their descriptor leaf
fingerprints are excluded from the exact semantic catalog payload digest field
`registered_form_catalog_digest`. That digest is **not** source-free: its
published encoder already binds resolved source identity, source fingerprint,
configuration-catalog digest and material leaf states. The separate witness
authority cannot be replayed because lookup additionally requires exact context/
source/fingerprint/catalog/key equality. The context exposes only the
borrow-only resolver:

```text
PlatformCatalogContextV1::registered_form_witness(
  catalog: &RegisteredFormCatalogV1,
  form: &RegisteredFormAuthorityV1,
) -> Result<&RegisteredFormDescriptorWitnessV1,
            PlatformCatalogWitnessLookupErrorV1>

PlatformCatalogWitnessLookupErrorV1
  ContextAuthorityMismatch tag 1
```

All four numeric catalog/set contract-version fields are private fixed `u16`
constants above, not caller input. Constructors always write exact 1 and any
decoder/import receiving 0, 2 or an unknown value rejects before catalog/query
construction. Task6 imports
`REGISTERED_FORM_CATALOG_CONTRACT_VERSION` and encodes exact `u16be(1)`; it may
not infer the number from the `/v1` string. A mechanical 1-to-2 mutation changes
the designated catalog/query bytes and is rejected by the production smart
constructor before I/O.

Both arguments must carry complete byte-equal validated context/source/
fingerprint/catalog authority. A different catalog/context fails before witness
lookup, while an independently reconstructed but completely equal validated
authority is not rejected solely for address/allocation identity. A Form
identity alone is never a witness key.

Adapters cannot construct/deserialize a witness or reparse the descriptor to
recover locations. Equivalent borrow-only configuration-catalog and object-key
resolvers require the current context authority before returning their opaque
witnesses. All witness sets have identical source coverage/capture run to both
semantic sets; a missing/extra/foreign witness is a context construction failure,
not an optional no-location record. Within each source the relationship is
bijective: exactly one configuration witness per configuration semantic catalog,
exactly one object witness per `PlatformConfigurationObjectAuthorityV1`, and
exactly one Form witness per `RegisteredFormAuthorityV1`, with no duplicate or
unreferenced entry. Missing/extra/duplicate, cross-source, cross-object,
cross-Form, wrong-key/fingerprint or catalog-digest swaps fail the one context
build before any provider/query constructor; cross-key compile-fail and
recording REDs exercise every row.

Empty contributing-field vectors never mean “no location”. Their closed
fallback matrix is:

| Absence-derived authority | Exact fallback witness |
| --- | --- |
| Configuration root UUID Missing | `configuration_root_span` |
| Base flavor from absent discriminators | `configuration_properties_span` |
| ScriptVariant or NamePrefix Missing | `configuration_properties_span` |
| complete top-level registration absence/MetadataAbsent | `configuration_child_objects_span` plus exact queried artifact identity |
| nested Form/Command/etc. absence under a present owner | owner's `object_child_objects_span` plus exact owner key and queried artifact identity |
| top-level object Own from absent membership fields | `object_properties_span` |
| nested Form Own from absent membership fields | `form_properties_span` |
| FormType Missing | `form_properties_span` |

Every container span is required, nonempty and validated against the same named
leaf as its field spans. A nonempty exact field vector uses its canonical field
spans; an empty valid/typed-absence vector uses only the row above. No root-wide
fallback is allowed when a narrower required container exists. REDs cover every
absence row, container/key/fingerprint replay, and prove an empty vector still
emits the one canonical physical location without reparsing.

These required spans are reachable because capture structure is closed: the one
exact Configuration must contain exactly one direct exact-namespace
`Properties` and one direct exact-namespace `ChildObjects`; every captured
top-level registered object descriptor must contain exactly one direct
exact-namespace object root, its one direct `Properties` and its one direct
`ChildObjects` container; every captured
nested Form descriptor must contain exactly one direct exact-namespace Form
root and its one direct `Properties`. For each required container, count 0 or 2,
or a direct foreign-namespace same-local lookalike, is capture-fatal and creates
no semantic catalog/witness context. Count 1 is the sole admitted row and its
span is mandatory. This structural rule does not make optional fields inside
Properties present. N=0/1/2 and foreign-lookalike capture REDs prove the
container fallback cannot be unreachable or ambiguous.

Task 6 receives FormModule material only as an owned
`AnalysisBslMaterialScanItemV1` from the context-created unified
`AnalysisBslMaterialScanPlanV1`, consumes the one plan-owned admission cursor,
and passes an admitted item by value only to
`read_analysis_bsl_material_verified`. Task 6 never imports or calls
`RegisteredFormAuthorityViewV1`, `form_module_material()`,
`resolve_capture_material` or `expectation_for`; the Task 5B context privately
uses that relationship/resolver chain when its sole dispatcher handles a
`RegisteredFormModule` item. Task8 instead selects either typed source view and
passes its Form view to the one atomic
`read_registered_platform_form_verified` method; that method owns the FormXml
material-ref/resolver/Task4-handle chain and Task8 can observe only the closed
verification result. Neither consumer borrows the internal entry or material
enum: Task 6 observes only the closed Analysis-BSL verification result, and
Task8 never receives a material ref, Task4 handle or raw bytes. There is no
suffix table, formatted registered directory, or `ArtifactRef`-to-path
conversion in either consumer. Only successful exact relation/state resolution
inside Task 5B may reach Task4's specialized verified reader. Managed+Present performs one
verifier/verified byte read; only Managed+Missing becomes complete exact absence
after one deduplicated contained-absence verifier call with zero file-byte reads
and zero XML parses. Ordinary/Inconclusive NotApplicable performs zero verifier
operations, zero byte reads and zero XML parses and is a typed unsupported/
inconclusive Form kind. A captured Missing that later appears is
`source_fingerprint_mismatch`, never stale `DefinitionAbsent`. Any BSL query/
cache identity that consumes this mapping binds
`registered_form_catalog_digest` and the accepted catalog contract version; it
may not silently reuse a pre-sidecar query encoder.

FormType scans all direct children whose local name is `FormType` under the one
capture-valid exact-namespace Form `Properties`. Zero same-local children is
`Missing`; more than one across exact/foreign namespaces adds `Duplicate`; a
foreign-only singleton is `WrongNamespace`; direct child element content is
`MixedContent`; and an exact scalar other than case-sensitive `Managed` or
`Ordinary`, an empty/control-bearing scalar, or a scalar beyond the common token
bound is `UnsupportedOrOverLimit`. The builder computes the full defect set and
chooses its lowest stable tag, so XML permutation cannot select another result.

The sidecar also owns CFE Form membership authority. For an exact Managed Form pair,
tag-10 BaseOwned/tag-11 Extension Own/Adopted facts use the nested Form wrapper
UUID and membership from this entry, never the top-level owner UUID. Positive
companion construction additionally requires exact `Known(Managed)` and a Known
`PlatformFormDocumentFlavorAuthorityV2` from the neutral single-pass result that
matches this exact flavor join. `method_bindings` completeness is independent
and is not a prerequisite:

| Source/configuration membership | Parser-derived Form flavor | Result |
| --- | --- | --- |
| analysis BaseConfiguration + Own | Plain | BaseOwned Form companion |
| destination ExtensionConfiguration + Own | Plain | Extension Own companion (application Unknown/not adopted) |
| destination ExtensionConfiguration + Adopted | Borrowed | Extension Adopted companion |
| analysis Base+Own | Borrowed | scoped `form_flavor_membership_mismatch`, no companion |
| destination Extension+Own | Borrowed | scoped `form_flavor_membership_mismatch`, no companion |
| destination Extension+Adopted | Plain | scoped `form_flavor_membership_mismatch`, no companion |

Ordinary/FormType-inconclusive, Missing Form.xml, flavor Inconclusive or a flavor
mismatch leaves the independent MetadataPresent polarity with its exact matching
Form-scoped Bounded gap and no companion. A binding-only typed failure does not.
Task 8 must compare the same typed FormType, Known flavor authority, complete
binding projection and sidecar membership before treating a lookup as mutation
authority; `Unbound` alone never authorizes a patch.

The sidecar payload is exact:

```text
RegisteredFormCatalogIdentityBytesV1 =
  u16be(schema=1)
  || u16be(source role tag)
  || bytes(ResolvedSourceSetIdentityBytesV1)
  || fingerprint32(source_fingerprint)
  || digest32(configuration_catalog_digest)
  || vec(entries sorted unique by ArtifactIdentityBytesV1(form))

entry = ArtifactIdentityBytesV1(owner)
  || ArtifactIdentityBytesV1(form)
  || uuid(wrapper_uuid)
  || membership_authority(the exact configuration-catalog grammar)
  || form_type_authority(
       u16be(Known=1) || u16be(Managed=1 | Ordinary=2)
     | u16be(Inconclusive=2) || u16be(problem tag))
  || string(form_descriptor_manifest_key exact UTF-8)
  || managed_form_xml_material(
       u16be(NotApplicable=1)
     | u16be(Missing=2) || string(form_xml_manifest_key exact UTF-8)
     | u16be(Present=3) || string(form_xml_manifest_key exact UTF-8)
         || fingerprint32(content_fingerprint))
  || form_module_material(
       u16be(NotApplicable=1)
     | u16be(Missing=2) || string(form_module_manifest_key exact UTF-8)
     | u16be(Present=3) || string(form_module_manifest_key exact UTF-8)
         || fingerprint32(content_fingerprint))

registered_form_catalog_digest =
  H("unica.registered-form-catalog/v1",
    RegisteredFormCatalogIdentityBytesV1)

registered_form_catalog_set_digest =
  H("unica.registered-form-catalog-set/v1",
    bytes(composite_snapshot_id)
    || vec(catalog digest32 in canonical source order))
```

Each of the three manifest-key `string(...)` positions above is emitted only by
`RegisteredFormManifestKeyV1::encode_registered_form_catalog_string_v1`, which
delegates to Task4's sealed
`SnapshotManifestKeyProjectionV1::encode_catalog_string_u32_v1`. Task5B never
calls the u64-framed `encode_identity_v1` for these fields and never obtains a
raw spelling/length to write its own prefix. For an exact valid key of `N`
UTF-8 bytes the field is `u32be(N) || N bytes`; a valid `N + 1` fixture is
`u32be(N + 1) || N + 1 bytes`, including multibyte fixtures where scalar count
differs. The 4,096-byte boundary begins `00 00 10 00`; 4,097 bytes is rejected
before a projection exists. This ownership repair reproduces, rather than
regenerates, every published sidecar payload: in particular the Analysis
Managed+Own+both-Present entry remains exactly 271 bytes with the unchanged
payload/digest values below.

The sidecar catalog set has exactly the same source coverage/order as
`PlatformConfigurationCatalogSetV1`; each sidecar catalog binds the exact
matching configuration-catalog digest. Empty entry vectors are explicit. The
old configuration-catalog payload and its goldens remain byte-identical because
the sidecar is a separate typed authority; its digest is stored beside, not
inside, the old digest.

The sidecar goldens reuse the Analysis/`patch` source fixtures and matching
configuration-catalog digests `279d317b...0046b9` / `0494c51f...88d20` above.
The one Form fixture has owner `MetadataObject "Catalog.Σ"`, Form
`"Catalog.Σ.Form.Main"`, Analysis wrapper UUID
`55555555-5555-4555-8555-555555555555`, descriptor key
`Catalogs/Σ/Forms/Main.xml`, Form.xml key
`Catalogs/Σ/Forms/Main/Ext/Form.xml`, FormModule key
`Catalogs/Σ/Forms/Main/Ext/Form/Module.bsl`, Form.xml leaf fingerprint
`sha256:` + `f`*64 and FormModule leaf fingerprint `sha256:` + `1`*64.
The destination prefixes all three keys with `ext/patch/`, uses wrapper UUID
`66666666-6666-4666-8666-666666666666`, Adopted extended UUID equal to the
Analysis Form wrapper UUID, Form.xml fingerprint `sha256:` + `a`*64 and module
fingerprint `sha256:` + `2`*64. Exact mechanically rebuilt values are:

```text
Analysis empty catalog:
  payload length = 218
  SHA-256(payload) =
    2b727f11bf1c2b8613894b1fe48dd4d8a9cd72d501cd4dc4b2b7d862d718cddc
  registered_form_catalog_digest =
    cc7b8add787c08ad7678218574e5a9a55395c7959440208f9a635ed5ab222cd2

Analysis Managed + Own + both materials Present:
  entry length = 271; payload length = 489
  SHA-256(payload) =
    ebc35ca9308325074400841ab96bf1cdc1826ee3067a74e6795addf2cea10040
  registered_form_catalog_digest =
    56704cb084d99f5ffb4b3c037b1f0d1c2c9e40a13a1c03c68a5023ca8cc7a30f

Same Analysis entry with Form.xml Missing and module Present:
  entry length = 239; payload length = 457
  SHA-256(payload) =
    7de4594026a67acea07813746748971c514193f76d76657a744cd0e9a87306f3
  registered_form_catalog_digest =
    bd701d64ac9614f99cf4eed66777bc270c9877742c752fbaa391e07ea7e909d6

Same Analysis entry with Form.xml Present and module Missing:
  entry length = 239; payload length = 457
  SHA-256(payload) =
    1dcd2a09ceeb57afd728394e0ac2573a4d6c799ec6d3b26bd51b21dd13c56786
  registered_form_catalog_digest =
    8f45120f8f3a95e033abc0b737bd53076083fbacf2fec9f793e26c9c5f3b14dd

Same Analysis registration as Known(Ordinary), both materials NotApplicable:
  entry length = 122; payload length = 340
  SHA-256(payload) =
    46d263b24e3967a51d1bde24989ecc0c02b378fdbb2b1c8915f094fd56a3d02e
  registered_form_catalog_digest =
    843a63f4e58f19439e6fa30dd4028050b90026455b423b5026df10e04e090924

Destination Managed + Adopted + both materials Present:
  entry length = 341; payload length = 564
  SHA-256(payload) =
    c14e4545a08ec192a7f3a87e419a09337aee1185b648f634f488e84a99196594
  registered_form_catalog_digest =
    0cde8a5d1b8e0d340a1cd60ad7357924a37b95dcc547cce038ae661a07782504

Two-source set, composite_snapshot_id="sha256:" + "e"*64, canonical
[Analysis Present, Destination Adopted] order:
  payload length = 143
  SHA-256(payload) =
    adb2e964d9456744db13470a3cace7f74e7e2ad65be2a10284971abba9628537
  registered_form_catalog_set_digest =
    1a9a9cc8c204bce7e293bd3e7dd8a333caf13d83d774665dc746e15b3523a2fc
```

Changing only FormType to an Inconclusive problem retains entry/payload lengths
122/340 and yields catalog digests:

```text
Missing = c11fa5cffe7def72e8b5636f814ed9c158b473b7e002672e11f2a13ac0c34bea
Duplicate = cbea29d556f92aa2531fa62008100391eede9f5d09c104cf7402b2672a38ba0a
WrongNamespace = 04957f5cfc6102bdf490883903043bc8b726b9e27429a8ff7588e4338dab56a7
MixedContent = fc10e238dc6811e3b13d1b71f61815e8f893ecc0b5a6884340a732852ef93bf6
UnsupportedOrOverLimit =
  88b9023734fcd151812715fc9f854c20e1f30dfd6a12da59ab655a810e33254d
```

Tests rebuild the full payload bytes and values. They also mutate only
descriptor key, Form.xml key/state/fingerprint,
FormModule key/state/fingerprint, membership, FormType, wrapper UUID, source,
configuration-catalog digest and entry order; every semantic mutation changes
the appropriate catalog/set digest, while input permutation preserves it.

Each single-problem authority above is the deterministic projection of a fully
validated **set** of applicable defects, never the first XML defect visited.
The builder scans the complete closed direct field scope, computes every
applicable stable problem tag, and chooses the numerically lowest tag. Known is
constructible only when that set is empty and the positive row is exact. XML
document/attribute order, parser callback order and hash-map order therefore
cannot select a different authority or digest.

The four closed defect-set rules are exact:

- root UUID considers only attributes whose local name is `uuid` on the one
  capture-valid direct exact-namespace `Configuration`. Zero such attributes is
  `Missing`; more than one across unqualified/foreign namespaces adds
  `Duplicate`; any namespaced same-local attribute adds `WrongNamespace`; any
  unqualified value outside non-nil `PlatformUuid` adds `InvalidOrNil`. Thus one
  exact valid unqualified UUID plus a foreign lookalike is Inconclusive
  `Duplicate`, while one foreign lookalike alone is `WrongNamespace`. A second
  direct `Configuration` or foreign same-local `Configuration` is capture-fatal
  and produces no catalog, so `Duplicate` never means duplicate Configuration;
- flavor scans all direct `ObjectBelonging` and
  `ConfigurationExtensionPurpose` discriminator children. It independently
  adds missing-purpose-for-exact-Adopted, unexpected-purpose-without-exact-
  Adopted, duplicate exact-namespace discriminator, unsupported exact value,
  invalid/duplicate direct optional `ConfigurationExtensionCompatibilityMode`
  or `KeepMappingToExtendedConfigurationObjectsByIDs`, and foreign same-local
  namespace defects. The lowest applicable tag 1..=6
  wins. Only absent+absent is Known Base, and only singleton exact
  Adopted+(Patch|Customization|AddOn) is Known Extension;
- membership scans all direct object `ObjectBelonging` and
  `ExtendedConfigurationObject` fields. It independently adds duplicate exact
  fields, unsupported belonging, missing UUID for exact Adopted, unexpected UUID
  for Own/absent belonging, invalid/nil exact UUID, and foreign same-local
  namespace defects. The lowest applicable tag 1..=7 wins. Only absent/exact
  Own with no extended UUID is Own, and only exact Adopted with one valid
  extended UUID is Adopted;
- FormType applies the complete direct-scope rule stated above. Only one exact
  scalar `Managed` or `Ordinary` with no same-local foreign/duplicate/mixed/
  invalid material is Known; every representable multi-defect permutation
  selects the lowest stable `PlatformRegisteredFormTypeProblemV1` tag.

`ScriptVariant` and `NamePrefix` are catalog-owned direct exact-namespace
`Configuration/Properties` views so no downstream consumer reparses
Configuration.xml. For each field, full direct-scope validation first adds every
applicable duplicate, foreign same-local, mixed-content and invalid/over-limit
problem, then chooses the lowest tag 1..=4. Only an empty problem set may produce
a semantic value:

- absent ScriptVariant is Missing; exact scalar `Russian`/`English` reuses the
  domain `KnownScriptVariant` tags Russian=1/English=2; any other nonempty
  control-free token of at most 256 UTF-8 bytes and 128 Unicode scalars is
  Unknown with exact UTF-8 spelling. Empty, control-bearing or over-limit is
  Inconclusive, never Unknown;
- absent NamePrefix is Missing; an exact empty scalar is Empty; a nonempty
  control-free value within the same 256-byte/128-scalar bound is Value with
  exact UTF-8 spelling. It is never trimmed, case-folded or defaulted.

Known/Unknown/Value token bytes are private bounded smart values, not arbitrary
Strings or reason text. The same pairwise/higher-order/permutation RED strategy
applies to both new authorities. Task 8 may consume only these typed catalog
fields and cannot re-open raw Configuration.xml.

The two flavor non-discriminators also have an exact closed grammar.
`ConfigurationExtensionCompatibilityMode` is absent or one direct exact-
namespace, non-mixed, nonempty control-free scalar of at most 256 UTF-8 bytes
and 128 Unicode scalars. `KeepMappingToExtendedConfigurationObjectsByIDs` is
absent or one direct exact-namespace, non-mixed scalar exactly lowercase `true`
or `false`. Duplicate, mixed, foreign same-local, empty/invalid or over-limit
optional material adds
`InvalidOptionalConfigurationProperty=6`; a foreign same-local
ObjectBelonging/ConfigurationExtensionPurpose discriminator independently adds
`WrongNamespace=5`. Full validation still selects the lowest applicable flavor
problem tag. Exact N/N+1, every invalid boolean spelling and permutation RED is
mandatory.

Pairwise and higher-order REDs enumerate every two-problem combination that is
syntactically representable, representative three-problem combinations, and
forward/reverse permutations of every child/attribute sequence. They assert the
same lowest tag, payload and digest. A hidden "last error wins" or early-return
implementation is forbidden.

`PLATFORM_METADATA_KIND_REGISTRY = "platform-metadata-kinds/v1"`. Its stable
tag is the one-based position in this exact append-only registry:

```text
 1 Language; 2 Subsystem; 3 StyleItem; 4 Style; 5 CommonPicture;
 6 SessionParameter; 7 Role; 8 CommonTemplate; 9 FilterCriterion;
10 CommonModule; 11 Bot; 12 CommonAttribute; 13 ExchangePlan;
14 XDTOPackage; 15 WebService; 16 HTTPService; 17 WSReference;
18 EventSubscription; 19 ScheduledJob; 20 SettingsStorage;
21 FunctionalOption; 22 FunctionalOptionsParameter; 23 DefinedType;
24 CommonCommand; 25 CommandGroup; 26 Constant; 27 CommonForm;
28 Catalog; 29 Document; 30 DocumentNumerator; 31 Sequence;
32 DocumentJournal; 33 Enum; 34 Report; 35 DataProcessor;
36 InformationRegister; 37 AccumulationRegister;
38 ChartOfCharacteristicTypes; 39 ChartOfAccounts;
40 AccountingRegister; 41 ChartOfCalculationTypes;
42 CalculationRegister; 43 BusinessProcess; 44 Task;
45 IntegrationService.
```

The implementation imports this one registry from Task 5A/domain; a local
catalog-only copy, lexical sort, filesystem-directory order or Debug enum tag is
forbidden. A future kind appends a new tag and bumps every affected catalog
golden/version if its presence changes accepted semantics.

The root-UUID authority is a catalog header, never a registered-object entry.
It is parsed only after capture has admitted exactly one direct
`{http://v8.1c.ru/8.3/MDClasses}MetaDataObject/{http://v8.1c.ru/8.3/MDClasses}Configuration`
and then classifies its same-local `uuid` attributes by the defect-set rule
above. A namespaced same-local UUID lookalike is the reachable semantic
`WrongNamespace` case; multiple same-local attributes are `Duplicate` by
precedence. A missing or invalid/nil unqualified UUID selects its exact typed
problem. A nested decoy cannot repair the header. A missing/duplicate direct
Configuration, a direct foreign-namespace same-local Configuration, or any
invalid envelope/root QName is capture-fatal and never constructs a catalog.
The known UUID uses shared canonical lowercase `PlatformUuid`.
Task5C-Evidence uses this exact header UUID only to validate
the ParentConfigurations root rule; it must not invent a Configuration
`ArtifactRef` or object key.

`capture_catalog_digest` is exact `sha256:<64 lowercase hex>` whose suffix is
the lowercase hex of `H("unica.platform-configuration-capture-catalog/v1",
vec(string(workspace-relative path) || fingerprint32(manifest content digest)))`
over every capture-admitted material used to assemble this result, sorted by
exact path UTF-8 and unique. The catalog semantic payload below separately binds
the parsed projection.
All entries from one catalog must share the exact logical source identity,
source fingerprint and capture run; mixing envelopes is a constructor error.
Entry order is `ArtifactIdentityBytesV1`; absence is authoritative only after
the complete capture catalog, never from a filtered semantic query.

The exact catalog payload is:

```text
u16be(schema=1)
|| u16be(source role tag)
|| bytes(ResolvedSourceSetIdentityBytesV1)
|| fingerprint32(source_fingerprint)
|| fingerprint32(capture_catalog_digest)
|| root_uuid_authority(
     u16be(Known=1) || string(canonical UUID)
     | u16be(Inconclusive=2) || u16be(problem tag))
|| flavor_authority(
     u16be(Known=1) || u16be(BaseConfiguration=1 | ExtensionConfiguration=2)
     | u16be(Inconclusive=2) || u16be(problem tag))
|| script_variant_authority(
     u16be(Missing=1)
     | u16be(Known=2) || u16be(Russian=1 | English=2)
     | u16be(Unknown=3) || string(exact bounded token)
     | u16be(Inconclusive=4) || u16be(problem tag))
|| name_prefix_authority(
     u16be(Missing=1)
     | u16be(Empty=2)
     | u16be(Value=3) || string(exact bounded value)
     | u16be(Inconclusive=4) || u16be(problem tag))
|| vec(entries sorted unique by ArtifactIdentityBytesV1)

entry = ArtifactIdentityBytesV1(artifact)
     || u16be(MetadataKind stable tag)
     || uuid(wrapper UUID)
     || membership_authority(
          u16be(Own=1)
          | u16be(Adopted=2) || uuid(extended UUID)
          | u16be(Inconclusive=3) || u16be(problem tag))
```

There are no arbitrary reason strings in this payload. Every Inconclusive value
uses only the closed problem tag above; bounded location evidence stays outside
the semantic catalog digest.

The catalog digest is
`H("unica.platform-configuration-catalog/v1", payload)`. Exact Known/Unknown
tags, flavor tags, membership tags and MetadataKind tags are frozen by the
shared domain registries; changing a tag requires a catalog encoder version.
After that digest is known, an entry's non-circular stable lookup key is:

```text
encode(PlatformConfigurationObjectKeyV1) =
  digest32(catalog_digest) || ArtifactIdentityBytesV1(artifact)
```

The key bytes are not included back into the catalog payload. A catalog set
resolver first requires exactly one matching catalog digest, then exactly one
entry matching the Artifact identity; missing, duplicate or digest-collision
resolution is a contract violation/typed absence as appropriate, never a
display-name fallback.
The set sorts Analysis first and every Destination by complete
`AtomicSourceIdentityV2`, requires exactly one catalog for every captured source,
and computes
`H("unica.platform-configuration-catalog-set/v1",
bytes(composite_snapshot_id) || vec(catalog digest32))`.

The catalog goldens reuse the Analysis/`patch` logical source fixtures and
`MetadataObject "Catalog.Σ"` (MetadataKind Catalog tag 28). Analysis uses
source/capture fingerprints `sha256:` + `b`*64 / `sha256:` + `c`*64,
Known root UUID `11111111-1111-4111-8111-111111111111`, Known Base flavor,
Known English ScriptVariant, Missing NamePrefix, one entry with wrapper UUID
`22222222-2222-4222-8222-222222222222` and Own membership:

```text
Known-root nonempty Analysis catalog payload length = 330
SHA-256(payload) =
  e3c0c0218cdc7a855749d68c111644820e959431dae2680bf0fa996288348e88
catalog digest =
  279d317b18203fa02829d9dbfa19359913e310bddf3beee5bfd82fc5240046b9

Same Analysis header with root Inconclusive(Missing=1), Known Base flavor and
the same ScriptVariant/NamePrefix authorities plus explicit empty entry vector:
payload length = 232
SHA-256(payload) =
  e093cf75b2f53f26b333724b43c01c15144d67927b18b9b37b77d95319b505bc
catalog digest =
  152fa913844d5a83fe94bdcd147c30faa60e1fe1738276cad6ee4c9381f72e67

Same empty-entry header with an invalid exact UUID plus a foreign same-local
UUID lookalike computes `{Duplicate=2, WrongNamespace=3, InvalidOrNil=4}` and
selects `Duplicate=2` regardless of attribute order:
payload length = 232
SHA-256(payload) =
  1eb02d3092a7861e3ffa387067300a4a7cc5698030dd2894a4612bfec0037552
catalog digest =
  85c4f7ac1dbedf6c2b6638fc8f9e2547e13f6e2bc650c27e6f71e49c804723ee

Same Analysis source/capture and Known root with two exact flavor
discriminators, unsupported values and a foreign lookalike computes
`{DuplicateDiscriminator=3, UnsupportedDiscriminatorValue=4,
WrongNamespace=5}` and selects tag 3. With an explicit empty entry vector:
payload length = 270
SHA-256(payload) =
  09dfab14416ec3df1e24f9a62e778fd5601ace4d19aec7025e6f1fdc5ae653a4
catalog digest =
  80d08e28c3f46f2aee2524b5c1e8f75e46ef54274dea78ab4e5aa710b3b34ae6

Same nonempty Analysis catalog as the Known golden, but with duplicate exact
ObjectBelonging plus invalid/foreign extended-UUID material, computes at least
`{DuplicateObjectBelonging=1, InvalidOrNilExtendedUuid=6,
WrongNamespace=7}` and selects membership tag 1:
payload length = 332
SHA-256(payload) =
  feaee355891253324b5e829a2d6f4ffbe64c59c8678bde768fb5719bd9ca89aa
catalog digest =
  2e86a23bb81ef0bdf0d08ca22aa3fb27a9363ad00d9f2bd85370ae42df82c770

Same Known root/flavor with ScriptVariant defects selecting
Inconclusive(DuplicateExactField=1), NamePrefix defects selecting
Inconclusive(WrongNamespace=2), and an empty entry vector:
payload length = 272
SHA-256(payload) =
  ac0bf12e401df4bec3c9f7222dde705bae4bd053d896eedbb17190ebf40b2232
catalog digest =
  bfe66006ad9ac18520c9da4285be442bafe42f6ad3a5db5c4c7ce446a0d847b9

Same Known root/flavor with exact Unknown ScriptVariant token `Future`, Empty
NamePrefix and an empty entry vector:
payload length = 278
SHA-256(payload) =
  52edf6a149e3bae4c0a1e4032dcc1682269b6645ec8e4b52c0256c4e673d94ee
catalog digest =
  adf383e0b91d7cf2172bc1e34c719a3b38636d9bf18a76a6c223b68c99b3cc11
```

The Destination catalog uses source/capture fingerprints
`sha256:` + `f`*64 / `sha256:` + `e`*64, Known root UUID
`33333333-3333-4333-8333-333333333333`, Known Extension flavor, Known Russian
ScriptVariant, Value NamePrefix `Ext_`, wrapper UUID
`44444444-4444-4444-8444-444444444444`, and Adopted extended UUID equal to the
Analysis wrapper UUID:

```text
Destination catalog payload length = 383
SHA-256(payload) =
  17efce32d6666db4fd78ee6b7cbc9be2f06e10b6155349bf7b7c6479624cbb58
catalog digest =
  0494c51f76227524b5d05d97ac716efdb7bbb76077fa65f54554babaf9288d20

Catalog-set payload for composite_snapshot_id="sha256:" + "e"*64 and canonical
[Analysis, Destination] catalog order has length 143:
SHA-256(payload) =
  3786293ab8286ed65085220ad87a5b7719085814ac4ecd13a7e797870b63bcc4
catalog-set digest =
  712fe375345adb7d81d8d0a7f17141b4c521878df2c9df0c353bb418cb12900e

Analysis Catalog object-key length = 48
bytes =
  279d317b18203fa02829d9dbfa19359913e310bddf3beee5bfd82fc5240046b9
  00010000000a636174616c6f672ecf83
SHA-256(key bytes) =
  e61456f30f098912c61a98008f36730ee3c7f9953bf8ce601cda09ebed1ea4a2
```

Tests mechanically rebuild these payloads from typed values and compare both
the bytes and digests; asserting copied digest literals without checking field
framing/order is insufficient. Reversing input catalogs still yields the same
Analysis-first bytes. Changing only root authority Known/Inconclusive, problem
tag, metadata-kind tag, membership, ScriptVariant authority/token or NamePrefix
authority/value changes the affected catalog, set digest and derived object key.
Permuting unchanged typed catalog/entry inputs changes none of those bytes.

The one public object-safe
`PlatformCatalogPort::build_context(&SourceSnapshotV2,
&dyn SourceSnapshotPort)` owns namespace, registration, flavor,
root/object/nested-Form UUID, membership, FormType, manifest-key,
ScriptVariant and NamePrefix extraction. It visits
`snapshot.analysis_snapshot()` once, followed by
`snapshot.destination_snapshots()` in their canonical unique order. For every
source-qualified captured Form handle in each atomic `SourceSetSnapshotV2`, it
calls exactly once
`source_reader.read_registered_form_descriptor_verified(atomic_snapshot,
&handle)`, borrows only
`VerifiedRegisteredFormDescriptorBytesV1::bytes()`, and performs exactly one
shared guard/semantic pass. It performs zero dynamic Form.xml/FormModule
verifier/probe calls and then invokes only private
`PlatformCatalogContextV1::from_prepared`. One call produces one composite
`PlatformCatalogContextV1` containing both exact catalog sets and all three
opaque, bijectively complete configuration, registered-Form and Analysis-BSL
witness sets.

Construction requires identical `CompositeSnapshotIdV2`, source count,
Analysis-first/canonical-Destination order, complete atomic source identity,
fingerprint and per-source catalog binding across `configuration`,
`registered_forms` and all witness sets; no half-context, config-only or
witness-optional public constructor exists. Equal immutable semantic inputs
and equal verified reader bytes may be built repeatedly and produce equal
contexts/digests; a changed diagnostic workspace epoch alone changes nothing.
Only after all of those invariants pass can `execution_binding_v1()` clone the
typed composite ID and two matching set digests into the sealed
`PlatformCatalogExecutionBindingV1`; no partially built context or catalog view
can mint it.
Exactly-once construction is deliberately not claimed by this borrowed-input
API. Task7 is the sole production orchestration owner: it invokes the port
exactly once per execution before MetadataCatalog, FormInspection or
SupportState, records that count with its injected spy/static call-site check,
and stores the whole composite context in `EvidenceExecutionContext`.

All adapters and all three Metadata/Form/Support query smart constructors
borrow this same context-owned value through an opaque lifetime-bound
capability. `MetadataCompositeQueryV2<'context>`,
`FormSourceSetQueryV2<'context>` and `SupportStateQueryV2<'context>` each retain
one private `&'context PlatformCatalogContextV1`; the first two do not merely
copy digests and pretend to have a lifetime. Context/catalog/set fields
and constructors are module-private, and none is `Clone`, `Deserialize` or
caller-constructible; only a port result can create the capability. A query
carries that explicit borrow and cannot outlive the context. This is semantic
snapshot/catalog authority, not object-address identity: `ptr::eq`, pointer
values and allocation/move addresses are forbidden in validation and digests.
There is no external API that can rebuild a detached lookalike set and present
it as authority. Neither adapter may
reread/reparse MDClasses, invoke the other adapter, decode evidence/display
output, or keep a second parser. Source identity/fingerprint/capture digest
equality is checked before either adapter projects facts. A missing source,
duplicate catalog, mixed capture run or catalog-set/composite mismatch is a
pre-provider contract violation with zero evidence prefix; a semantic
flavor/membership Inconclusive remains typed and becomes the exact scoped gap.

Mandatory REDs prove every consumer receives the exact matching configuration
and registered-Form catalog digests. Each catalog-port invocation performs
exactly one injected
`SourceSnapshotPort::read_registered_form_descriptor_verified` call, one
ordinary Present descriptor read inside that Task4 seam and one shared guard/
semantic pass per source-qualified captured Form handle, and performs zero
dynamic registered-material verifier/probe calls. Calling the port twice with
semantically equal composite inputs and equal reader bytes succeeds and returns
byte-identical catalog/set digests, while Task7's recording integration RED
fails if production orchestration invokes it twice. All adapters/query
constructors accept only the whole context borrow and query construction has
zero I/O. Config-only, sidecar-only, mixed-source, source-permuted and missing-
source detached-authority combinations fail before provider reader I/O;
compile-fail tests reject direct construction, clone/deserialization and a
Metadata/Form/Support query outliving the context; moving the context
does not change any digest and no address comparison occurs;
equal refs in base and extension remain source-distinct; forged source kind,
missing object, flavor/membership-inconclusive, fingerprint mismatch and mixed
capture envelopes never authorize Support ownership; Missing/Unknown/
Inconclusive ScriptVariant and Missing/Empty/Inconclusive NamePrefix remain
typed and cannot be defaulted; and a static dependency test rejects Metadata-
adapter calls, display parsing from Support, or any second Configuration/
Properties parser in Task5C/Task8 consumers.

The accepted multi-source 200,000-Form stress fixture additionally asserts N Task4 indexed
descriptor-handle validations, N ordinary Present descriptor reads, N shared
semantic parses and zero full-manifest/Form/expectation rescans; a recording
implementation that linearly searches the catalog for any handle fails the RED.

## 6. Completeness, missing Form material, and result limits

MetadataCatalog full-scans the analysis registration set and emits positive
registered facts independently of the requested negative-proof keys. Every
requested analysis key has exactly one Present/Absent polarity before limits.
FormInspection classifies every canonical entry in the exact borrowed analysis
`RegisteredFormCatalogV1`. It whole-document scans Form.xml only for exact
Known(Managed)+Present entries and owns their FormCommand Present/Absent plus
`Form contains FormCommand`. Known(Ordinary) and FormType Inconclusive are
NotApplicable: each emits its exact typed Form-scoped gap with zero registered-
material verifier calls, zero material byte reads and zero material XML parses.
Only Known(Managed)+Missing performs one deduplicated
`context.read_registered_platform_form_verified(...)` call before its exact
gap; the private context method's Missing branch performs the one contained
absence verification, reads zero file bytes and performs zero XML parses. The
query's at-most-32 request scopes only enrich material for matching
Managed entries; they never narrow the registered classification/scan. With
zero requested scopes, an empty registered-Form sidecar returns Complete(empty)
with zero reads, while a nonempty sidecar still classifies every Form and reads
every Managed+Present Form. Task 8 can reach the neutral Form parser only with a
Present wrapper returned by the any-source composite-context method; it cannot call
the parser with captured material authority, a Task4 handle or raw bytes.

For every exact Managed sidecar Form, expected material is the exact capture-
owned `form_xml_manifest_key` carried by Missing/Present; the familiar serializer
shape is diagnostic only. Ordinary/Inconclusive entries carry no such key:

```text
<OwnerDirectory>/<OwnerName>/Forms/<FormName>/Ext/Form.xml
```

The key is never re-derived from this displayed path. Inside the private
`PlatformCatalogContextV1::read_registered_platform_form_verified` method, the
context first resolves the exact capture relationship before any I/O; the
following Task4 calls and handles never cross into the provider:

- Missing state: exactly one deduplicated
  `source_reader.read_registered_material_verified` call and contained absence proof; on
  verified Missing, zero file-byte reads/zero XML parses and exact Bounded
  `registered_form_material_missing`;
- Present state: exactly one registered-material verifier call/verified byte
  read, then the shared Form parser;
- NotApplicable: zero verifier/byte-read/parse calls and the exact typed
  unsupported/inconclusive classification;
- forged/nonregistered handle/projection/state/key/entry disagreement:
  nonretryable `registered_material_handle_mismatch` before I/O;
- a semantically valid handle whose external file later appears, disappears,
  changes identity/content or becomes a reparse/link alias: retryable
  `source_fingerprint_mismatch`.

If a Missing path appears, a Present path disappears, or identity/content
changes after capture, the specialized reader returns
`source_fingerprint_mismatch`; the provider discards its whole staged batch and
does not emit a stale negative or partial sibling result. Within one provider
invocation every demanded relationship is canonicalized before reads, so two
conclusions that need the same relation still produce verifier call count 1.

The missing-material gap's `Artifacts` scope is the exact
`gap_artifact_projection_v2(FormMaterialScopeV1)`: Form, requested FormCommands,
runtime owner/Method subjects, and the analysis+destination
`SourceScopedArtifact` halves of every applicable pair. Neither the
`FormMaterialScopeV1` value nor a raw pair key inhabits `Artifacts`. It never
emits command absence, never means “all descendants”, and never omits the exact
runtime/pair-half artifact whose proof was lost.

### 6.1 Atomic semantic evidence groups

All semantic validation and conflict checks finish before `maxEvidence` is
applied. Retention is by canonical **semantic evidence group**, not individual
location record or individual ProviderFact. “One typed semantic value” is too
narrow because a single usable conclusion may require several fact variants.
Every emitted record is assigned exactly once through this closed internal
registry:

```text
SEMANTIC_ATOMIC_GROUP_REGISTRY = "semantic-evidence-groups/v2"
SEMANTIC_ATOMIC_ENCODER = "semantic-evidence-group-encoder/v2"

SemanticAtomicGroupIdV2 (stable u16 tags in declaration order)
  1 StandaloneFact {
      source: AtomicSourceIdentityV2,
      provider_fact_stable_tag, subject, relation?, object?, semantic_digest
    }
  2 CfePairHalf {
      source: AtomicSourceIdentityV2,
      role = Analysis | Destination, source_scoped_artifact
    }
  3 EventSubscriptionDescriptor { source: AtomicSourceIdentityV2, subscription }
  4 FormCommandEvidenceCluster { source: AtomicSourceIdentityV2, form }
  5 ScheduledJobCluster {
      source: AtomicSourceIdentityV2, job,
      state = DisabledActivation
            | NonPredefinedActivation
            | EnabledDescriptor
    }
  6 HttpServiceDescriptor { source: AtomicSourceIdentityV2, service }
  7 PlatformCallbackRequirement {
      source: AtomicSourceIdentityV2, owner, callback_slot
    }
  8 DefinitionObservationCluster {
      source: AtomicSourceIdentityV2, queried_method: ArtifactRef(Method)
    }
  9 SupportStateObservation {
      source: AtomicSourceIdentityV2, subject: ArtifactRef,
      semantic_authority_digest: SupportSubjectSemanticAuthorityDigestV2
    }
```

`SemanticAtomicGroupIdV2` is the canonical identity projection, not the
lossless group value. The same classifier constructs one private
`SemanticAtomicEvidenceGroupV2` with exactly one module-private
`kind: SemanticAtomicEvidenceGroupKindV2`. That kind registry has the same nine
names and stable tags as the ID registry above; each variant owns that ID's
typed inputs, the complete canonical physical records and its exact
variant-specific frozen typed inputs to `material_subjects(group)`, never a
caller-supplied or pre-encoded artifact vector. There is no
parallel caller-selected variant tag: the ID, secondary payload and material
walk are derived exhaustively from that one payload. This distinction is
constructively required for Event, Form, ScheduledJob, HTTP and callback
groups, whose nested real artifacts are deliberately absent from the compact
ID, and for Form material that can be frozen even when no corresponding record
is emitted. No raw constructor accepts an ID plus records/material, and no
artifact cohort, iterator or material-member callback is exposed. The owner is
carried intact through provider limiting/outcome state; retaining or dropping a
group moves that whole owner and does not add independent identity bytes.

The grouping registry is semantic, not a fact-tag switch:

- one `CfePairHalf` contains that source/artifact's exact Present/Absent polarity
  and its whole role-specific companion when Present. Analysis and destination
  halves stay separate source-local groups, so dropping one half yields exact
  Unknown rather than forcing a cross-source megagroup. If several normalized
  pairs reuse one half, they reuse this one physical group and all dependent pair
  keys enter its material subjects;
- one `EventSubscriptionDescriptor` contains the whole binding/descriptor, any
  complete SelectedEventSource companion projection, and **all** derived
  ExchangePlan `SubscriptionSource`/`uses` bindings. Its ordinary registration
  Present may remain standalone because it proves existence only and cannot seed
  a runtime mechanism;
- one `FormCommandEvidenceCluster` contains every actually emitted FormCommand
  polarity, the exact Form `contains` FormCommand structural binding, and each
  CommandAction pending binding derived from that Form's V2 catalog. The independent
  Form registration Present may remain standalone existence evidence. The
  neutral complete Form/Element event lookup catalog is auxiliary authority,
  not a ProviderFact member of this evidence cluster;
- one `ScheduledJobCluster(DisabledActivation)` contains only the exact
  registered-job/Use=false activation-negative facts;
- one `ScheduledJobCluster(NonPredefinedActivation)` contains exact Use=true and
  Predefined=false metadata-only evidence. It is not a positive Binding,
  handler candidate, or runtime root. MethodName, module profile and Definition
  are neither read nor material in this branch;
- one `ScheduledJobCluster(EnabledDescriptor)` contains exact Use=true,
  Predefined=true and every distinct ProviderFact whose complete combination
  authorizes the pending positive binding descriptor. The three states are
  mutually exclusive for one job/snapshot. Non-activation fields never enter or
  gap Disabled or NonPredefinedActivation;
- one `HttpServiceDescriptor` or `PlatformCallbackRequirement` contains every
  distinct ProviderFact whose combination authorizes that route/callback
  requirement;
- one `DefinitionObservationCluster` contains the complete polarity and every
  complete physical `DefinitionPresent` observation for one exact queried
  Method. Identical duplicate declarations and conflicting shapes are valid
  source observations, not provider contract violations. `DefinitionAbsent` is
  legal only for a complete exact query with zero present declaration;
- one `SupportStateObservation` contains the exact Support fact for one
  source-qualified subject and one `SupportSubjectAuthorityV2`. Existing and
  PlannedDestinationAbsent authority can never share a group even when their
  source/artifact/state spellings are equal;
- a fact that participates in none of those clusters is `StandaloneFact`.

This ScheduledJob branch does not weaken whole-descriptor atomicity. Use=false
selects DisabledActivation and stops. Use=true followed by exact
Predefined=false selects NonPredefinedActivation and stops. Use=true with a
missing/malformed Predefined, or Predefined=true with incomplete positive
descriptor material, emits only the exact scoped gap and no ProviderFact,
candidate, zero-record group, or partial activation state. Only a complete
positive metadata descriptor selects EnabledDescriptor. No partial
MethodName/profile record is emitted outside EnabledDescriptor, and every
physical record belongs to exactly one mutually exclusive cluster.

An unclassified record, one record assigned to two groups, a cluster assembled
from different source snapshots, or two conflicting semantic cluster values is a
provider contract violation before limits. The last clause does not apply to
valid multiple DefinitionPresent shapes inside tag 8: only mixed
Present+Absent polarity, Absent with a physical declaration, or malformed query
association is a contract violation. A group is retained whole or dropped whole;
neither a location witness nor a different fact tag from the same cluster can
survive as an apparently complete prefix.

The v2 encoder is shared by Platform XML, Task 6 BSL, Support and Task 7. It is
not Rust `Debug`, JSON, display text, native-endian memory, or a caller-selected
serializer. Its primitives are exact:

```text
SOURCE_SET_IDENTITY_ENCODER = "unica.source-set-identity.v1"
ARTIFACT_IDENTITY_ENCODER = "artifact-identity/v1"
```

```text
u8/u16/u32/u64       = unsigned big-endian fixed width
bool(false/true)      = u8(0) / u8(1)
bytes(x)             = u32be(byte_length(x)) || x
string(s)            = bytes(exact UTF-8 bytes; no normalization)
uuid(x)              = string(PlatformUuid canonical lowercase 36-byte ASCII)
option(None)         = u8(0)
option(Some(x))      = u8(1) || encode(x)
vec(xs)              = u32be(count) || encode(each canonical item)
digest32(hex)        = 32 decoded bytes from exact 64-char lowercase ASCII hex
fingerprint32(s)     = require exact "sha256:" prefix, then digest32(suffix)
H(domain, payload)   = SHA-256(bytes(ASCII domain) || bytes(payload))
```

Lengths/counts that do not fit u32 and noncanonical digest spelling are
constructor errors before hashing. Closed enum tags are u16 and never depend on
declaration memory layout. Every typed artifact position uses exactly one
identity projection matching live `ArtifactRef` equality/hash semantics:

```text
ArtifactIdentityBytesV1 =
  u16be(ArtifactKind stable tag)
  || string(UnicodeLowercase(canonical_ref))
```

`ARTIFACT_IDENTITY_ENCODER` versions this byte grammar even though the bytes are
not independently hashed with that string; its consumers bind the version
through their enclosing atomic/query/analysis contract.

`UnicodeLowercase` is Rust `value.chars().flat_map(char::to_lowercase)` only in
a build that passed the exact `(17,0,0)` component assertion in section 3.10,
with no locale, normalization or exact-spelling tie-break. The original canonical-ref
spelling remains display/location payload only. Secondary payloads, primary
subjects, source-free records, physical records, pair keys, material subjects,
query vectors and every golden below use `ArtifactIdentityBytesV1`; none uses
`Display` output or exact `canonical_ref` bytes as identity.

The source projection is closed and reuses the domain encoder rather than
inventing a limiter-local spelling:

```text
AtomicSourceIdentityV2 {
  role: Analysis(tag=1) | Destination(tag=2),
  resolved_source_set: exact ResolvedSourceSet {
    name,
    kind,
    source_format,
    relative_root,
    mapping_digest,
  },
}

ResolvedSourceSetIdentityBytesV1 =
  u64be(byte_length("unica.source-set-identity.v1"))
  || ASCII "unica.source-set-identity.v1"
  || u64be(byte_length(name UTF-8)) || name UTF-8
  || u8(kind: Configuration=1 | Extension=2 | ExternalProcessor=3 | ExternalReport=4)
  || u8(source_format: PlatformXml=1 | Edt=2 | Unknown=3 | Invalid=4)
  || u64be(byte_length(relative_root UTF-8)) || relative_root UTF-8
  || u64be(byte_length(mapping_digest UTF-8)) || mapping_digest UTF-8

encode(AtomicSourceIdentityV2) =
  u16be(role tag) || bytes(ResolvedSourceSetIdentityBytesV1)
```

The sort rank is `u8(0)` for Analysis and `u8(1)` for **every** Destination;
destinations do not receive input/ordinal ranks 1..N. Equal ranks sort by the
complete encoded `AtomicSourceIdentityV2`. A source/manifest fingerprint is
snapshot freshness, not logical source-set identity: changing only it does not
change this encoding, source ordering, or any semantic group key. Different
roots, mappings, kinds, formats, or names cannot collide merely because a
display source-set name is equal. Equal source-free semantics in two logically
different destinations remain distinct.

The normative identity goldens are:

```text
ResolvedSourceSet {
  name="analysis", kind=Configuration, source_format=PlatformXml,
  relative_root=".", mapping_digest="sha256:" + ("a" * 64)
}
ResolvedSourceSetIdentityBytesV1 length = 142
SHA-256(ResolvedSourceSetIdentityBytesV1) =
  e1d804d1e18f2d02679dce05b4e2a822c9a776cfd749a67754c9328fc48d9396
encode(AtomicSourceIdentityV2 { role=Analysis, ... }) length = 148
SHA-256(encode(AtomicSourceIdentityV2)) =
  8543b710e36b6393bd362435b76774cf62e59a24bc5b61ee3926a473a2234710

ArtifactIdentityBytesV1(MetadataObject, "Document.Σ") =
ArtifactIdentityBytesV1(MetadataObject, "Document.σ") =
  00010000000b646f63756d656e742ecf83
SHA-256 = c8b634908a27b4b0e863456caebaff34ecb1dcee84527054421395db23db8599
ArtifactIdentityBytesV1(MetadataObject, "Document.İ") =
  00010000000c646f63756d656e742e69cc87
```

These values are assertions, not examples. An encoder version cannot retain
the same name while changing any field, tag, length width or Unicode-lowercase
behavior.

Each group has one primary source-qualified subject and one fixed 32-byte
secondary digest. The secondary payload is variant-specific and closed:

```text
StandaloneFact              = u16(ProviderFact stable tag) || option(relation) || option(object)
CfePairHalf                  = u16(role) || vec(sorted unique dependent pair keys)
EventSubscriptionDescriptor = empty
FormCommandEvidenceCluster  = bytes(EffectiveFormMaterialScopeIdentityBytesV1)
ScheduledJobCluster         = u16(state tag)
HttpServiceDescriptor       = empty
PlatformCallbackRequirement = u16be(PlatformCallbackSlot stable tag)
DefinitionObservationCluster = empty
SupportStateObservation = digest32(support_subject_semantic_authority_digest)
```

The first `StandaloneFact` field is **exactly** `record.fact.stable_tag()` from
the closed `ProviderFact` registry, including append-only
`ScheduledJobNonPredefined` tag 13. There is no separate fact-family registry and
no many-to-one Present/Absent collapse: `MetadataPresent` emits 1,
`MetadataAbsent` emits 2, and the two values therefore order and partition as
distinct standalone groups. `CfePairHalf` role tags are Analysis=1 and
Destination=2. ScheduledJob state tags are DisabledActivation=1,
NonPredefinedActivation=2 and
EnabledDescriptor=3. No fourth partial-activation tag is reserved or accepted
in v2. Every other referenced ProviderFact/relation/callback-slot tag is the
accepted closed domain-registry tag; missing or unknown tags are constructor
errors, never enum memory layout.

`ProviderGroupMaterialIdentityV2` is closed to
`SourceScopedArtifactIdentityBytesV2` (tag 1) and
`DestinationMembershipPairIdentityBytesV2` (tag 2). It is used only by the CFE
pair-key vector and the closed material/gap projection below. EventSubscription's
selected sources/handler are already inside
its source-free descriptor fact, so its secondary is the exact empty byte
string. ScheduledJob state alone partitions its three mutually exclusive
clusters; HTTP routes are already source-free facts inside the primary service
cluster. Definition's queried Method is the primary subject, so its secondary
is also empty. Callback slots use the existing closed numeric tag (canonical
lifecycle tag 1, presentation tag 2), never Debug/display strings; unknown tag
is rejected. Support needs only its semantic authority digest beside its primary
subject.

`EffectiveFormMaterialScopeIdentityBytesV1` is derived deterministically before
I/O from exactly one exhaustive `RegisteredFormCatalogV1` entry plus zero or one
canonical requested `FormMaterialScopeV1` for the same source/Form. For an
unrequested exact-Managed entry it is the canonical Form-only scope with
explicitly empty command/runtime/pair vectors; for a requested entry it is the
validated union with that one request scope. Missing/duplicate sidecar entry,
requested Form absent from the sidecar, foreign source/Form/owner or duplicate
request enrichment is a contract error before reader call 1. There is no
post-I/O synthesis, prefix-derived scope or implicit path scan. Query equality
binds both `registered_form_catalog[_set]_digest` and the requested scope-set
digest, so it implies equal effective-scope input for every entry. Every
record-derived command/handler difference remains in the source-free semantic
cluster rather than a hidden group key.

Callback slot tag 1 has
`secondary_digest =
d197df97f3d37820cd2f0ce62c69de7ca5182ea7860a6904cb3d37a3cd045690`;
slot tag 2 has
`6559fce918e7134b87b729f1ebeee75dbad173702d23360f649442f3f1ffa5d4`.
The tests rebuild both payloads, mutate only the slot, and reject tag 0/3.

Request, Proposal, Mechanism, `ConclusionScope`, query-association records and
upstream query digests are deliberately absent from every provider-local group
key/order. Adding a second proposal for an already-requested artifact therefore
preserves query digest, provider outcome, secondary digest and retained prefix.
Task 7 owns a separate application `MaterialAssociationMapV2` from provider
material identities/group IDs to sorted conclusion scopes; equal invocation
reuse unions that map without re-invoking or mutating provider testimony.

The table token `empty` means a **zero-byte payload**. It is distinct from an
actual `vec([])` field, which is the explicit four-byte zero count `00000000`.
`secondary_digest` is
`H("unica.atomic.secondary/v2", u16be(group tag) || secondary payload)`.
Pair keys and dependent subjects use this closed projection:

```text
SourceScopedArtifactIdentityBytesV2 =
  bytes(AtomicSourceIdentityV2) || ArtifactIdentityBytesV1(artifact)

DestinationMembershipPairIdentityBytesV2 =
  bytes(AtomicSourceIdentityV2(role=Analysis))
  || bytes(AtomicSourceIdentityV2(role=Destination))
  || ArtifactIdentityBytesV1(the equal typed artifact identity)

ProviderGroupMaterialIdentityV2 =
    u16be(1) || SourceScopedArtifactIdentityBytesV2
  | u16be(2) || DestinationMembershipPairIdentityBytesV2
```

Every provider-material vector sorts by the complete bytes above. Duplicates are
a final typed-constructor error before sorting/hashing, not silently erased
afterward. Application association builders may union duplicate normalized
request contributions before calling that constructor. A provider query never
invents `Request` or any other application conclusion scope.

The later admission artifact identity is the sealed
`ProviderMaterialArtifactSetV2` from section 3.10.4, not this mixed
`ProviderGroupMaterialIdentityV2` vector and not a Task7-built member list. Its
owner projection exhaustively visits the nine group variants, expands every
tag-2 destination pair to two real `SourceScopedArtifact` halves, includes all
direct and nested real artifacts, then canonicalizes that artifact-only set.
Task7 may encode the resulting set only through its owner writer; it cannot use
the set to answer whether a query or outcome authorized a root.

The zero-byte payload goldens are EventSubscription tag 3
`6d5279b6c1359a472d5fa7b948218fac1158c4801d90b1ffac6332f585e1cd48`,
HTTP tag 6
`c2672c86fa24b502cf70635ede9aab5de73e6d6ee0eccffe46fd0ec982fef877`,
and Definition tag 8
`5b480e02a937c8535b08692fd4ba5072d6ab2eae10b63a268cb823947b409566`.
By contrast, CFE tag 2 + Analysis role tag 1 + an actual empty pair vector has
`secondary_digest =
e3bb6e1195781e2173bad67a105dd063342858e2833bf3db843d6210439a22d5`.
Tests rebuild both forms and reject their conflation.

#### 6.1.1 Closed source-free ProviderFact payloads

The ordinary whole-fact digest is intentionally source-bound and remains valid
for `EvidenceRecord`/evidence-ID identity. It is forbidden in semantic-group or
physical-group classification. Every v2 classifier instead computes:

```text
SOURCE_FREE_PROVIDER_FACT_PAYLOAD_ENCODER =
  "source-free-provider-fact-payload/v2"

SourceFreeProviderFactPayloadV2 =
  u16be(ProviderFact stable tag) || exact variant payload below

source_free_typed_payload_digest =
  H("unica.source-free-provider-fact-payload/v2",
    SourceFreeProviderFactPayloadV2)
```

The fact tag is deliberately inside this digest even though
`AtomicSemanticRecordV2` also carries it. The duplicated discriminator prevents
a payload from being reinterpreted across fact variants. There is no fallback
to `provider_fact_digest`, Debug/JSON, or an empty/all-zero digest. A future fact
variant is a registry error until this encoder and its golden matrix are bumped.

The shared Definition payload is:

```text
SourceFreeDefinitionShapePayloadV2 =
  bool(is_function)
  || bool(is_async)
  || bool(exported)
  || u16be(BslExecutionContext stable tag)
  || vec(parameters in declaration order:
       string(UnicodeLowercase(name))
       || bool(by_value)
       || bool(has_default))
```

Parameter order is semantic; path, source spelling case and declaration
location are not. The closed per-variant payload after the leading fact tag is:

| ProviderFact stable tag | Exact source-free variant payload |
| --- | --- |
| 1 MetadataPresent | empty |
| 2 MetadataAbsent | empty |
| 3 CodeOccurrence | `string(exact validated search_term)` |
| 4 DefinitionPresent | `SourceFreeDefinitionShapePayloadV2` |
| 5 DefinitionAbsent | empty |
| 6 Binding | `SourceFreeValidatedBindingPayloadV2` below |
| 7 Call | `SourceFreeCallTargetPayloadV2 || u16(resolution) || u16(call_type) || u16(BslExecutionContext)` |
| 8 PlatformCallback | full `SourceFreePlatformCallbackPayloadV2` below |
| 9 Support | `u16(SupportFactState) || digest32(SupportSubjectSemanticAuthorityDigestV2)` |
| 10 BaseOwnedMetadataIdentity | `u16(BaseConfiguration=1) || u16(Own=1) || uuid(object_uuid)` |
| 11 ExtensionMetadataMembership | `u16(ExtensionConfiguration=2) || u16(Own=1 \| Adopted=2) || uuid(wrapper_uuid) || [for Adopted only: uuid(extended_configuration_object_uuid)]` |
| 12 ScheduledJobActivation | `u16(activation)` |
| 13 ScheduledJobNonPredefined | `string("scheduled-job-non-predefined/v1") || u16(state=1)` |

Tag 10 is total only over strict BaseConfiguration+Own; it has no optional
extended UUID and no Adopted/Extension arm. Tag 11 is total only over strict
ExtensionConfiguration Own/Adopted. Own carries only `uuid(wrapper_uuid)`;
Adopted additionally carries
`uuid(extended_configuration_object_uuid)`. It has no Absent arm. Pair keys,
source-set identities and the outer registered artifact are excluded. Their
authoritative positions are the atomic source/group key, primary artifact and
secondary dependent-pair bytes. UUID/flavor/membership semantics are retained.

The Call target projection is closed:

```text
SourceFreeCallTargetPayloadV2 =
    u16be(Artifact=1)
  | u16be(Named=2)
      || vec(1..=2 identifier segments in source order:
           string(UnicodeLowercase(segment)))
  | u16be(Dynamic=3)
```

For Artifact, `AtomicSemanticRecordV2.object` must be Some and equal to the
target by `ArtifactIdentityBytesV1`; duplicating it in the payload is forbidden.
Named and Dynamic require object=None. Each Named segment satisfies the exact
Task 6 identifier grammar; dots are framing between one/two segments, not part
of a segment. Exact source spelling remains display/location only. The Task 6
v2 constructor accepts only this total Cartesian subset:

| Resolution | Target | CallType | BSL context |
| --- | --- | --- | --- |
| Resolved | Artifact | Direct or Method | any one of the six closed values |
| Ambiguous | Artifact candidate | Direct or Method | any one of the six closed values |
| Unresolved | Named | Direct or Method | any one of the six closed values |
| Dynamic | Dynamic | Dynamic | any one of the six closed values |

`CallType::Callback` is not emitted by the Task 6 snapshot BSL provider. Every
other `(resolution,target,call_type,object)` combination is a constructor error
before grouping. The optional Evidence object is Some and equal only for the
two Artifact rows; it is None for Named/Dynamic. An exhaustive Cartesian RED,
not four hand-picked positives, freezes this table.

Tag-7 goldens use Unresolved=4, Direct=1 and ModuleDefault=1:

```text
Named("Missing") payload length = 25
source_free_typed_payload_digest =
  ead5260baa60573ac62283dafc24f87f4e8110f17d4895feeab726fa92e14d67
Named("missing") is byte-identical.

Named("Other") payload length = 23
source_free_typed_payload_digest =
  622074f83867e30d1d8333b34b8789e54fd0a9e307f64ff54d979903d4c59e09

Named("İ", "MISSING") payload bytes =
  00070002000000020000000369cc87000000076d697373696e67000400010001
source_free_typed_payload_digest =
  43d170188a4ba960a05f2d2b5beeeed7f2802552c7058cea4e22bdd351fed495

Dynamic target + Dynamic resolution(2) + Dynamic call type(4) payload length=10
source_free_typed_payload_digest =
  ee4df215dfb6ecdb1cd68ddc5c1dfb56dd1383efa18ff509e0fcfc632f993207
```

Thus two unresolved names cannot collapse and `İ` uses the same expanding
Unicode-lowercase rule as Artifact identity.

`SourceFreeValidatedBindingPayloadV2` excludes only the outer subject,
relation and object already encoded by `AtomicSemanticRecordV2`; it retains the
binding kind and every descriptor/profile/policy field:

```text
u16be(BindingKind stable tag) ||
  Structural:
    empty
  EventSubscription:
    string("event-subscriptions/v1") || u16(event)
    || vec(sorted canonical sources:
         u16(family) || string(UnicodeLowercase(object_name)))
    || bool(global) || bool(client_ordinary_application)
    || bool(client_managed_application) || bool(server)
    || bool(external_connection)
    || u16(handler_signature_class) || u16(binding_runtime_context)
  FormCommand:
    string("form-command-handlers/v1")
    || string(UnicodeLowercase(action))
    || u16(form_call_type) || u16(binding_runtime_context)
  ScheduledJob:
    string("scheduled-jobs/v2")
    || bool(use_enabled) || bool(predefined)
    || bool(common_module_global) || bool(common_module_server)
    || u16(binding_runtime_context)
  HttpRoute:
    string("http-service-handlers/v1") || u16(http_verb)
    || string(exact normalized url_template)
    || u16(binding_runtime_context)
  SubscriptionSource:
    string("subscription-source-resolved-artifact/v1")
    || u16(source namespace) || u16(platform source type)
    || string(UnicodeLowercase(source object_name))
```

`SourceFreePlatformCallbackPayloadV2` is exactly:

```text
string(registry_version) || u16(callback_slot) || u16(script_variant)
|| string(metadata_kind) || string(module_kind)
|| string(UnicodeLowercase(method_name))
|| u16(callable_kind) || u16(export_requirement)
|| u16(context_requirement)
|| vec(parameters: bool(by_value) || bool(has_default))
```

No provider/source/pair/snapshot/fingerprint/coverage/location/evidence ID is in
these payloads. Conversely, no semantic descriptor/profile/policy/shape field
may be dropped merely because a current compatibility table does not inspect
it. Both `AtomicSemanticRecordV2` and `AtomicPhysicalRecordV2` below carry this
`source_free_typed_payload_digest`; the normal source-bound whole-fact digest is
never a limiter/classifier input.

The non-zero CFE golden freezes the distinction. For
ExtensionMetadataMembership(tag 11), ExtensionConfiguration(tag 2),
Adopted(tag 2), wrapper UUID
`22222222-2222-4222-8222-222222222222`, and extended UUID
`11111111-1111-4111-8111-111111111111`:

```text
SourceFreeProviderFactPayloadV2 length = 86
bytes =
  000b000200020000002432323232323232322d323232322d343232322d38323232
  2d3232323232323232323232320000002431313131313131312d313131312d3431
  31312d383131312d313131313131313131313131
source_free_typed_payload_digest =
  59b794153fbebec4902be3de0d7af2fdf25363d0007d71e7a5ca04c53251f50f
```

Constructing this same semantic membership for destination A and destination B
with different pair keys, logical source identities and source fingerprints
must produce that identical payload/digest. Their group source and dependent
pair secondary bytes remain different. Changing only flavor, state, either UUID
or fact tag changes the digest; changing only pair/source/snapshot/fingerprint
does not.

The complete cluster semantic digest is source-free by construction. Project
every record to:

```text
AtomicSemanticRecordV2 =
  u16be(fact stable tag)
  || ArtifactIdentityBytesV1(source-free subject)
  || option(u16be(relation tag))
  || option(ArtifactIdentityBytesV1(source-free object))
  || digest32(source_free_typed_payload_digest)
```

Sort and require uniqueness by these complete bytes, then encode the vector and
compute `H("unica.atomic.semantic-cluster/v2", vector)`. This projection
explicitly excludes source-set identity, composite/snapshot IDs, source
fingerprint, provider/port/version, coverage, freshness, evidence ID and source
location. Source binding is supplied only by the preceding source identity in
the group key.

Tag 8 is the one closed exception to sorted-unique semantic records because
duplicate Definition declarations are themselves semantic evidence. First
encode every physical record, sort it by complete `AtomicPhysicalRecordV2`, and
remove only byte-identical duplicates. Validate one polarity:

```text
DefinitionObservationPolarityV2
  Present = 1
  Absent  = 2

DefinitionShapeMultiplicityV2 =
  bytes(SourceFreeDefinitionShapePayloadV2)
  || u32be(declaration_observation_count)

DefinitionObservationSemanticPayloadV2 =
  u16be(polarity)
  || vec(DefinitionShapeMultiplicityV2 sorted unique by shape bytes)

definition_source_free_semantic_cluster_digest =
  H("unica.definition-observation-semantic-cluster/v2",
    u16be(DefinitionObservationCluster tag=8)
    || DefinitionObservationSemanticPayloadV2)
```

`declaration_observation_count` is the number of distinct physical
DefinitionPresent observations with that shape after exact physical dedup. A
Present payload must contain at least one shape/count and every count is
`1..=u32::MAX`; Absent must contain the explicit empty shape vector and is legal
only when the complete exact query observed zero declaration. Present+Absent is
a provider contract violation. Multiple Present rows with the same or different
shapes are valid; EvidenceGraph derives `duplicate_definition` and
`conflicting_definition_shapes` only after the full retained cluster is
available. Paths/locations are excluded from the multiset bytes, so renaming two
paths while preserving two distinct physical declarations preserves count and
digest. One versus two identical declarations changes the digest. All non-tag-8
groups retain the sorted-unique `AtomicSemanticRecordV2` algorithm above.

The tag-8 goldens use shape A = synchronous exported zero-parameter Procedure
in ModuleDefault and shape B = the same except Function:

```text
shape A bytes = 000001000100000000
shape B bytes = 010001000100000000

Present, shape A count=1:
  semantic payload length = 23
  digest = cc6c6bd22f3621d4bb84286f9abfeb78ff206d6dbc56944b76ca7c2f673c6d30

Present, shape A count=2:
  semantic payload length = 23
  digest = 916612e064dfc60e20f3139f9989323460096833bb8daa1ce86bcb5e735f45ba

Present, shape A count=1 + shape B count=1:
  semantic payload length = 40
  digest = 6e07e455da95ae876da3a32221b4a824d9057391ae9ba0ecd55afa265cc2e5ac

Absent, empty shape vector:
  semantic payload bytes = 000200000000
  digest = 26842aeb66c8194bb8e4bd9446c3342bbe7768d960b693f9068738cfd7a4aea5
```

Exact duplicate physical bytes collapse before the count and preserve the first
digest. Two location-distinct declarations produce count=2 and the second
digest. Renaming both paths without changing their distinct cardinality
preserves the second digest. Forward/reverse path and record order must produce
the same result.

The strict total-order key is the byte tuple:

```text
(
  u8(source rank),
  bytes(AtomicSourceIdentityV2 encoding),
  u16be(SemanticAtomicGroupIdV2 tag),
  bytes(AtomicSourceIdentityV2 encoding || ArtifactIdentityBytesV1(primary subject)),
  secondary_digest[32],
  source_free_semantic_cluster_digest[32],
)
```

Fact tag is deliberately not a group-identity discriminator. Physical records
inside the selected group are encoded, sorted and deduplicated by:

```text
AtomicPhysicalRecordV2 =
  AtomicSourceIdentityV2
  || u16be(fact stable tag)
  || ArtifactIdentityBytesV1(subject)
  || option(u16be(relation tag))
  || option(ArtifactIdentityBytesV1(object))
  || digest32(source_free_typed_payload_digest)
  || [for ProviderFact::Support only:
        digest32(support_subject_snapshot_authority_digest)
        || digest32(support_state_query_digest)
      | for every other fact: empty]
  || option(string(path) || option(u32be(line)) || option(u32be(column)))
  || u16be(port tag) || string(provider name) || string(provider version)
  || u16be(coverage tag) || fingerprint32(freshness/source fingerprint)
```

`None` sorts before `Some` through the option tag. Evidence ID is derived later
and is excluded to avoid a hash cycle. Location-distinct witnesses therefore
count as distinct physical records while exact byte-identical duplicates count
once. `physical_record_digest` is
`H("unica.atomic.physical-record/v2", AtomicPhysicalRecordV2 bytes)`. No
input-order/stable-sort fallback exists at record or group level.

The Support-only suffix is unambiguous because the preceding closed fact tag is
9; no option byte is emitted for other variants, so their published physical
goldens remain unchanged. It binds response/admission provenance to the exact
snapshot authority **and** exact upstream `SupportStateQueryV2.query_digest`
without contaminating the source-free semantic payload or group order. The
normal full Support ProviderFact digest includes all three binding digests and
must agree with the borrowed query entry and this suffix before a record is
accepted. Invocation-level validation alone is insufficient: a physical Support
record cannot be replayed from another delta round/query.

The normative StandaloneFact group golden reuses the Analysis source and
`MetadataObject "Document.Σ"` identity fixtures above and fixes:

```text
group tag = StandaloneFact(1)
source rank = 0
ProviderFact stable tag = MetadataPresent(1)
relation = None; object = None
semantic record fact tag = MetadataPresent(1)
SourceFreeProviderFactPayloadV2 bytes = 0001
source_free_typed_payload_digest =
  71c5b84f4b062dc65b0f602de5075adf08a4b5bcf048889e73ea883c8132e792
semantic record vector count = 1

secondary_digest =
  d676f3489f6c9b6794c72c0cbd47f8a139e8fe96574dd53e99a440f16eae405c
source_free_semantic_cluster_digest =
  1e493a672a5fe9dab86c687d050f58652d72f11359848dc261544c9456a41dd2
complete group-key length = 388
SHA-256(complete group-key bytes) =
  b3c237eaccb7eb8732611804c1e972ec329c2f6a3d303ffd017863d4c16eed01

AtomicPhysicalRecordV2 adds:
  location=None, port=MetadataCatalog(1),
  provider="unica.platform_xml_catalog", provider_version="2",
  coverage=Complete(1), source_fingerprint="sha256:" + ("b" * 64)
physical-record length = 273
H("unica.atomic.physical-record/v2", physical bytes) =
  ccf7e795f73c9721ff5de924220a7d346416b2d5f349dfea5d5e0686894159f7
```

Changing only that fingerprint to `sha256:` + `c`*64 must preserve every
source/artifact/secondary/semantic/group-key byte above and change only the
physical digest and identities derived from physical/snapshot freshness.

Retention is an atomic canonical prefix. Iterate groups in that total order and
retain whole groups while `retained_record_count + group.record_count <=
maxEvidence`. On the first group that does not fit, stop retaining: that group
and every later group form the dropped tail. The provider does **not** skip it
and continue with a smaller later group. Every dropped-tail group's exact
material subjects contribute to `platform_xml_result_limit`; if the first group
alone is too large, zero records are retained and its plus all later material is
gapped. A bounded heap/equivalent may compute the same prefix, but must produce
this exact behavior.

This classification and prefix-stop execute inside both Platform XML providers
before their local `max_records` ceiling returns an outcome. No individual
record may be discarded first. Task 6 v2 applies the identical encoder before
each BSL query's local ceiling; Support has no independent lossy record ceiling,
so Task 7 is its first admission step. Task 7 may apply later per-port/global
ceilings only to already-complete v2 groups.

`material_subjects(group)` is closed:

- registration/presence and standalone fact: exact source-qualified subject and
  optional object artifacts from the fact;
- CFE pair half: that exact source-qualified half plus both exact source-
  qualified artifact halves of every dependent pair key;
- EventSubscription descriptor: subscription, owner/handler Method, every
  selected source artifact, and every derived ExchangePlan `uses` artifact;
- FormCommand evidence cluster uses the closed artifact-only projection below:
  exact Form; every requested or emitted FormCommand; each emitted
  CommandAction's exact handler Method; plus both artifact halves of every pair
  in its effective Form-material scope;
- disabled ScheduledJob activation and nonpredefined metadata: exact job only;
  neither branch has owner/handler material;
- enabled ScheduledJob descriptor: exact job, owner and handler Method;
- HTTP declarative descriptor: exact service, every emitted route, owner and
  handler Method;
- platform callback requirement: exact owner and Method. Callback slot stays in
  the typed secondary/reason and never becomes a fake artifact;
- Definition observation cluster: exact queried Method; declaration paths are
  locations, not material artifacts;
- Support state observation: exact source-qualified subject. Semantic/snapshot/
  query authority digests are validation keys, not fabricated artifacts.

Every item above is a real `SourceScopedArtifact`; every pair first passes the
section-3.9 two-half projection. Request/Proposal/Mechanism scopes and
query-association objects are forbidden here and are attached only by Task 7's
`MaterialAssociationMapV2`. A metamorphic RED adds a second proposal for an
already-requested artifact and proves byte-identical provider query, material
vector, dropped-tail gap, retained prefix and raw outcome while only the Task 7
association map changes.

The tag-4 evidence projection is exact and total over its actually emitted
FormCommand/CommandAction ProviderFacts plus the frozen effective scope:

```text
form_command_group_gap_artifact_projection_v2(records, effective_scope) =
  { source-qualified catalog Form }
  union every requested FormCommand in effective_scope
  union every exact source-qualified runtime owner/Method subject in
    effective_scope.runtime_subjects
  union for each CommandAction:
    { source-qualified catalog Form, source-qualified FormCommand,
      source-qualified handler Method }
  union both source-qualified artifact halves of every effective-scope pair
```

The runtime-subject union is unconditional: every frozen
`effective_scope.runtime_subjects` member enters the projection even when its
requested FormCommand is absent, the parsed catalog has no Action, or no
CommandAction ProviderFact was emitted. Record presence may add material but
can never delete request-frozen material.

`FormEvent` and `ElementEvent` rows are not ProviderFacts and never enter this
projection, record count, semantic cluster, prefix or result-limit gap. Their
structural owner kind/name/id/path remains solely inside the auxiliary complete
catalog semantic identity. The closed public `ArtifactKind` has no FormElement,
so that path is never cast to an `ArtifactRef`, inserted into
`ProviderGapScope::Artifacts`, replaced by a sentinel, or reconstructed from a
display name. The function canonicalizes through `ArtifactIdentityBytesV1`,
unions duplicates, rejects an empty/foreign-source/foreign-Form result and runs
before the 2,000-subject bound.

After all provider-local semantic and result-limit gaps are known, canonicalize
and deduplicate the complete gap vector, then count gaps and the union of exact
`Artifacts` subjects. At `<=256` gaps and `<=2,000` subjects retain that exact
vector. At 257 gaps or 2,001 subjects, replace the **entire** provider gap vector
with exactly one QueryWide `platform_xml_gap_limit`; it is not appended to a
truncated prefix. Before that bound, `platform_xml_result_limit` must not use an
empty, approximate, hierarchy-only, or sentinel scope. Task 7 owns a separate
post-collection overflow pass after per-port/global admission. Its v7 finish
validator obtains the exact distinct opaque-set union count only through
section-3.10.4's sealed `canonical_union_cardinality_v2`, then owns the
comparison with 2,000; it never receives a set member. It never appends an
application limit gap behind this provider-local sentinel without applying that
second closed normalization.

Complete silence is negative proof only inside the exact complete closed scan.
Bounded/unavailable/failed silence is always `Unknown`.

## 7. CFE destination membership extraction

The provider emits `BaseOwnedMetadataIdentityV1` only when all are true:

1. the query half is analysis;
2. the parsed configuration flavor is exact BaseConfiguration;
3. the exact keyed shared-catalog entry has
   `CatalogObjectMembershipAuthorityV1::Own`, whether the direct
   ObjectBelonging field is absent or exact `Own`, with no extended UUID;
4. its direct object UUID is valid;
5. the source/artifact/pair are exact query members.

It emits `ExtensionMetadataMembershipV1` only when the query half is destination
and the borrowed catalog flavor is exact ExtensionConfiguration. Neither
projection reparses membership fields; both consume the same shared catalog
authority used by Support and future Task8 code. Direct object membership is:

| ObjectBelonging | ExtendedConfigurationObject | Typed membership |
| --- | --- | --- |
| absent | absent | Own `{ wrapper_uuid }` |
| exact Own | absent | Own `{ wrapper_uuid }` |
| exact Adopted | one valid non-nil UUID | Adopted `{ wrapper_uuid, extended_uuid }` |
| catalog Inconclusive(problem) | exact-pair Bounded typed membership gap |

Explicit Own is therefore not malformed. Under BaseConfiguration it can produce
`BaseOwnedMetadataIdentityV1`; under ExtensionConfiguration it produces the
typed destination Own companion that the application conservatively maps to
Unknown/`destination_object_not_adopted`, never ExtensionOwned. Any local
fallback table that accepts only absence or reparses ObjectBelonging is
forbidden.

All contributing fields have descriptor witnesses. **Both** the analysis and
destination halves of a Form pair require their own exact Known(Managed)
sidecar entry and mandatory Form.xml material to be Present, produce a Known
document-flavor authority in the neutral pass, and be flavor-compatible
before membership can be promoted. Complete command/event binding success is
not required. MetadataCatalog reads
and parses both halves independently through the neutral parser; it neither
borrows FormInspection records nor treats one half's complete catalog as proof
for the other. Missing/unsupported FormType, flavor-inconclusive or flavor-
mismatched material leaves that
half's independent MetadataPresent polarity and zero companion under the exact
descriptor-local mapping below.

| Form half condition | Exact Bounded reason | Canonical witnesses | Gap scope / companion |
| --- | --- | --- | --- |
| `Known(Ordinary)` | `unsupported_registered_form_type` | exact sidecar Form identity + direct FormType scalar | `gap_artifact_projection_v2(effective_scope)`; zero companion; zero registered-material verifier calls/byte reads/XML parses |
| `PlatformRegisteredFormTypeAuthorityV1::Inconclusive(problem)` | `registered_form_type_inconclusive` | exact Form identity + canonical lowest-tag FormType defect span set | same exact projection; zero companion; zero registered-material verifier calls/byte reads/XML parses |
| `Known(Managed)` + Form.xml `Missing` | `registered_form_material_missing` | registration/FormType + exact capture-owned missing manifest key + verified contained-absence authority | same exact projection; zero companion; one deduplicated registered-material verifier call, zero file-byte reads/XML parses |
| `PlatformFormDocumentFlavorAuthorityV2::Inconclusive(problem)` | `form_document_flavor_inconclusive` | FormType/membership + exact opaque flavor witness set | same exact projection; zero companion |
| Analysis Base+Own+Managed + Known `Borrowed` | `form_flavor_membership_mismatch` | FormType + Own authority + exact opaque flavor witness set | same exact projection; zero companion |
| Destination Extension+Own+Managed + Known `Borrowed` | `form_flavor_membership_mismatch` | FormType + Own authority + exact opaque flavor witness set | same exact projection; zero companion |
| Destination Extension+Adopted+Managed + Known `Plain` | `form_flavor_membership_mismatch` | FormType + Adopted/extended-UUID authority + exact opaque flavor witness set | same exact projection; zero companion |

The witness set is the sorted unique union returned by the context's opaque
`RegisteredFormDescriptorWitnessV1` resolver and the neutral parser's opaque
`PlatformFormDocumentFlavorWitnessSetV2`; exact catalog/handle equality is
required before either borrow. No adapter reparses a descriptor/Form.xml or
substitutes a raw parser message to recover locations. The projection always includes
both source-qualified pair halves plus every command/runtime subject already in
the effective scope, even if this failed half emitted no Action. These mappings
apply symmetrically to the relevant analysis/destination half and survive the
other half succeeding.

`TypedFormCatalogFailureV2` by itself is **not** an ownership-companion blocker.
An unknown Event, unsupported main-attribute context, malformed Action or other
binding-only defect leaves a Known flavor and the BaseOwned/Extension companion
byte-identical; it only gaps FormInspection/Task8 binding conclusions. When a
BaseForm defect exists, the independent flavor authority is Inconclusive and
the row above blocks the companion regardless of the binding projection's
derived `MalformedRegisteredMaterial` result.

Provider completeness validates pair polarity, companion and gap consistency
before limits. `gap_covers(half)` is true only for a Bounded outcome and one of:

- `Artifacts` whose exact source-qualified material contains that half and all
  pair/Form subjects required for the missing companion;
- `SourceSetWide` whose complete `AtomicSourceIdentityV2` equals that half's
  source; or
- `QueryWide` for this exact composite invocation.

A sibling-only artifact gap, equal display source name with different logical
identity, another pair's Form material, or another invocation never covers the
half. The full state matrix is normative:

| Polarity | Companion | Outcome/gap | Result |
| --- | --- | --- | --- |
| Present | exactly one canonical matching companion | Complete, no same-half contradictory gap | valid |
| Present | exactly one canonical matching companion | Bounded only by unrelated/sibling gaps | valid |
| Present | exactly one canonical matching companion | a gap semantically withdraws this same half/companion | contract violation |
| Present | none | Bounded + matching Artifacts | valid Unknown |
| Present | none | Bounded + exact matching SourceSetWide | valid Unknown |
| Present | none | Bounded + QueryWide | valid Unknown |
| Present | none | Complete, no gap, sibling-only/wrong-source/uncovered gap, Unavailable or Failed record prefix | contract violation |
| Present | duplicate/conflicting/source-wrong companion | any | contract violation |
| Absent | none | Complete, or Bounded only by unrelated material | valid absence polarity |
| Absent | none | a gap withdraws this same half's absence proof | contract violation |
| Absent | any companion | any | contract violation |

An out-of-plan half and multiple semantic polarities are always contract
violations. A provider-wide Unavailable/Failed outcome has zero records, so it
cannot carry a Present/Absent prefix. After this validation, an atomic local
result-limit may drop the whole `CfePairHalf` and emit exact Bounded material;
that later all-or-none drop is not reclassified as a malformed provider input.

## 8. Closed EventSubscription registry and whole-fact binding

Registry version is:

```text
EVENT_SUBSCRIPTION_REGISTRY = "event-subscriptions/v1"
```

### 8.1 Exact selected sources

`Properties/Source` is exactly one direct `{MDCLASSES_NS}Source` and must contain
at least one direct
`{http://v8.1c.ru/8.1/data/core}Type`. There is no intermediate MDClasses
`<Type>` wrapper. Empty Source is malformed; v3's “subscribes only” behavior is
forbidden.

Element namespace and scalar QName namespace are independent:

```text
EVENT_SOURCE_TYPE_ELEMENT_NS = "http://v8.1c.ru/8.1/data/core"
EVENT_SOURCE_QNAME_NS =
  "http://v8.1c.ru/8.1/data/enterprise/current-config"
```

Each data-core Type scalar is one QName-valued source reference. Its prefix may
be any valid XML NCName but must resolve in scope to exact
`EVENT_SOURCE_QNAME_NS`; literal prefix spelling such as `cfg` is nonsemantic.
Its local value must be exactly `<Family>.<RegisteredName>` with one dot and two
valid canonical identifiers. A same-local MDClasses/foreign Type element cannot
count. Unprefixed, undeclared, multi-colon, wrong element URI, wrong QName URI,
empty, control-bearing, or unresolved values cannot participate in a positive
whole fact.

`Properties/Event` and `Properties/Handler` are each exactly one direct
MDClasses singleton with a nonempty scalar. Missing, duplicate, nested, mixed-
content, or foreign same-local nodes make this descriptor view atomically
Bounded; neither field is defaulted from the subscription name or source set.

The closed registry has exactly 13 named source families, three signature
classes, and an explicit event-compatibility cell for every supported pair:

| Type family | Registered root kind | BeforeWrite | BeforeDelete |
| --- | --- | --- | --- |
| DocumentObject | Document | `SourceCancelWriteModePostingMode`, 4 | `SourceAndCancel`, 2 |
| CatalogObject | Catalog | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| ChartOfCharacteristicTypesObject | ChartOfCharacteristicTypes | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| ChartOfAccountsObject | ChartOfAccounts | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| ChartOfCalculationTypesObject | ChartOfCalculationTypes | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| BusinessProcessObject | BusinessProcess | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| TaskObject | Task | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| ExchangePlanObject | ExchangePlan | `SourceAndCancel`, 2 | `SourceAndCancel`, 2 |
| InformationRegisterRecordSet | InformationRegister | `SourceCancelReplacement`, 3 | unsupported |
| AccumulationRegisterRecordSet | AccumulationRegister | `SourceCancelReplacement`, 3 | unsupported |
| AccountingRegisterRecordSet | AccountingRegister | `SourceCancelReplacement`, 3 | unsupported |
| CalculationRegisterRecordSet | CalculationRegister | `SourceCancelReplacement`, 3 | unsupported |
| ConstantValueManager | Constant | `SourceAndCancel`, 2 | unsupported |

The three class payloads are exact:

```text
SourceAndCancel                       -> Source, Cancel                     -> 2
SourceCancelWriteModePostingMode      -> Source, Cancel, WriteMode,
                                         PostingMode                       -> 4
SourceCancelReplacement               -> Source, Cancel, Replacement       -> 3
```

`BeforeWrite` is compatible with all 13 families. `BeforeDelete` is compatible
only with the first eight deletable object families; all four record-set
families and ConstantValueManager are explicit unsupported cells. The registry
is represented as 21 exact `(event, family)` rows, not a default branch.

Signature lookup is partial and occurs only after compatibility:

```text
event_signature_class(event, family) -> Option<EventHandlerSignatureClassV1>
```

An incompatible pair returns None. A catch-all mapping from an incompatible
BeforeDelete register/constant to `SourceAndCancel` is forbidden even if the
caller currently checks compatibility first.

Every selected source must resolve to an exact registered, identity-validated
current-configuration object. The selected set is nonempty, contains at most 256
entries, is canonical unique and sorted, retains exact registered spelling, and
is included in the whole-fact semantic digest. Uniqueness is checked by insertion
into a set keyed by the complete canonical typed artifact identity before output
sorting; an adjacent-window check over a differently ordered tuple is forbidden.
Duplicate, unsupported, unregistered, resource-gapped, or mixed-signature-class
selection makes the whole EventSubscription binding `Unknown`; no positive
`subscribes` edge is emitted.

`EventSubscriptionBindingV1.selected_sources` is the sole authoritative source
set. The provider may emit location witnesses for every member, but it must not
independently parse a second authority. If the wire model requires per-source
`SelectedEventSourceV1` companion facts, those facts are constructed from the
descriptor and their complete `(source-scoped artifact, family)` set must equal
the descriptor set exactly before promotion.

The public `SubscriptionSource`/`uses` projection is not an alternate complete
source set. It is derived exactly as:

```text
descriptor.selected_sources
  .filter(family == ExchangePlanObject)
  -> ExchangePlan --uses--> EventSubscription
```

The exact ExchangePlan uses set must equal that filtered descriptor subset. A
missing, extra, differently canonicalized, or independently parsed companion/
uses member makes the affected mechanism `Unknown`. Other selected families
remain in the authoritative descriptor and signature compatibility, but do not
invent public `uses` edges. Diagnostic observations may survive; no partial set
can create a runtime mechanism.

A platform-valid all-objects-of-family selector is outside named-source v1 until
its exact XML serialization is backed by primary/fixture evidence. When
recognized, it yields `unsupported_event_subscription_all_objects_selector`,
Unknown, not a dangling named object or a malformed guess.

### 8.2 Exact event/signature classes

One subscription may select multiple named sources only when every selected
`(event, family)` cell is compatible and every member maps to the same exact
signature class:

- BeforeWrite DocumentObject cannot mix with any other family;
- the seven non-Document object families may mix with each other and with
  ConstantValueManager because all are `SourceAndCancel`;
- the four BeforeWrite record-set families may mix with each other because all
  are `SourceCancelReplacement`;
- a record-set family cannot mix with an object/constant family;
- all eight BeforeDelete object families may mix because all are
  `SourceAndCancel`;
- BeforeDelete with a register or constant is unsupported regardless of other
  selected sources.

Other event tokens are well-formed but
`unsupported_event_subscription_variant`, `Unknown`. The Definition join checks
callable kind Procedure, Export required, exact parameter **count** from the
single selected class, `BslExecutionContext::ModuleDefault`, and
`is_async=false`. A complete wrong kind/export/arity is
`event_subscription_signature_mismatch`, `No`; `is_async=true` or any explicit
BSL context is the unproven
`unsupported_event_subscription_signature_variant`, `Unknown`, and never a
root. Parameter names, `Val`, and default flags are retained by Task 6
but are not guessed as compatibility requirements because the primary guide
specifies arity, not those flags. Runtime context for every supported row is
`BindingRuntimeContextV1::SameAsSourceEvent`.

### 8.3 Exact CommonModule capability profile

Handler scalar is exactly
`CommonModule.<RegisteredCommonModuleName>.<MethodName>`; the literal leading
segment is required and is not the module name. The owner CommonModule must be
registered and its descriptor must contain exactly one direct explicit
lowercase boolean for each of the five platform-required capability fields:

```text
EVENT_SUBSCRIPTION_COMMON_MODULE_PROFILE = "event-subscription-modules/v1"

Global                     = false
ClientOrdinaryApplication  = true
ClientManagedApplication   = false
Server                     = true
ExternalConnection         = true
```

`ServerCall` and `Privileged` are not part of the EventSubscription validity
predicate because the official EventSubscription guide does not require them and
official 1C material contains a ServerCall-enabled handler-module pattern. If
present, their raw exact booleans may be retained as separate diagnostic
observations, but neither value changes the whole-fact compatibility digest and
absence is never filled with a guessed default. Missing, duplicate, mixed,
non-boolean, contradictory, unregistered, or resource-gapped material in one of
the five required fields makes the whole binding `Unknown` with
`unsupported_event_subscription_module_profile` or the precise malformed gap.

The typed whole fact is:

```text
EventSubscriptionBindingV1 {
  registry_version,
  subscription,
  event,
  handler_signature_class,
  selected_sources,
  handler_module,
  handler_method,
  common_module_profile: EventHandlerModuleProfileV1 {
    global, client_ordinary, client_managed, server, external_connection
  },
  expected_parameter_count,
  runtime_context = BindingRuntimeContextV1::SameAsSourceEvent,
}
```

It is witnessed at every Type, Event, Handler, CommonModule registration/name,
and every one of the five material capability fields. EvidenceGraph emits
`subscribes` only after compatible Definition coverage. The previous `AtServer`
callback shape is a P0 contract violation, not a tolerated alias.

## 9. ScheduledJob capability and binding

Registry versions are:

```text
SCHEDULED_JOB_REGISTRY = "scheduled-jobs/v2"
SCHEDULED_JOB_COMMON_MODULE_PROFILE = "scheduled-job-modules/v1"
```

The platform-valid binding predicate comes from the broad Developer Guide: the
method is an exported procedure/function of a non-global CommonModule callable
on the server. Therefore only these capability fields are material:

```text
Global                     = false
Server                     = true
```

Both are exact direct explicit lowercase booleans. `ServerCall`, the two client
flags, `ExternalConnection`, and `Privileged` may be parsed as raw diagnostic
properties but are not platform-validity gates and no missing value is guessed.
The FAQ's ServerCall-enabled module is an example, not the general rule. A
registered module with Global=false and Server=true is not rejected merely
because ServerCall=false.

Activation is parsed before the positive binding subview. The order is a closed
short-circuit state machine, not a list of fields that become material together:

```text
registered capture-valid ScheduledJob
  -> exact direct Use singleton
     false -> DisabledActivation; STOP
     true  -> exact direct Predefined singleton
              false -> NonPredefinedActivation; STOP
              true  -> exact MethodName
                         -> exact registered CommonModule profile
                            -> pending binding/Definition endpoint
```

- `Use=false` emits exact `scheduled_job_disabled`, No, and creates no runtime
  root. Predefined, MethodName, module profile and Definition are not read,
  queried, witnessed, or gapped for this job; an independently requested method
  remains unrelated evidence;
- missing/duplicate/mixed/nonboolean Use emits no activation fact and one exact
  activation-scoped Unknown. No later field is opened;
- `Use=true` emits no activation fact and opens only Predefined;
- missing/duplicate/mixed/nonboolean Predefined emits an exact
  Predefined-scoped Unknown. MethodName, profile and Definition remain unopened;
- exact `Use=true, Predefined=false` emits only the atomic
  NonPredefinedActivation conclusion
  `non_predefined_scheduled_job_instance_unproven`, Unknown. Runtime instance
  parameters are absent from source XML; MethodName/profile/Definition are
  nonmaterial and cannot replace this reason with another gap. This is a
  metadata-only observation, never a positive Binding or handler candidate;
- exact `Use=true, Predefined=true` opens MethodName and then its exact owner
  profile. Missing/malformed MethodName is descriptor-local Unknown; a valid
  MethodName opens only that registered module's Global/Server fields. An
  incomplete/unsupported descriptor emits only the exact scoped gap and no
  partial activation fact/candidate;
- only exact MethodName + Global=false + Server=true creates the pending
  `ScheduledJobBindingV1`/Definition endpoint. The platform guide states that a
  predefined job has no parameters, so compatibility requires exact arity zero.

This independence applies after a capture-valid registered ScheduledJob XML
envelope exists. A capture-authoritative fatal XML/envelope/namespace failure
still yields no provider prefix at all; it is not evidence for Use=false. A
well-formed mechanism-local defect in any non-activation sibling, however,
cannot erase the exact Disabled fact.

The Definition endpoint decision is closed and metadata-first:

| Complete metadata state | Runtime activation result | Definition endpoint |
| --- | --- | --- |
| exact Use=false | No, `scheduled_job_disabled` | none |
| missing/malformed/conflicted Use | Unknown activation gap | none |
| Use=true with missing/malformed/conflicted Predefined | Unknown exact Predefined gap | none |
| Use=true, Predefined=false | Unknown, `non_predefined_scheduled_job_instance_unproven` | none |
| Use=true, Predefined=true with missing/malformed MethodName | Unknown exact MethodName gap | none |
| Use=true, Predefined=true, but Global!=false or Server!=true | Unknown, `unsupported_scheduled_job_module_profile` | none |
| exact Use=true, Predefined=true, Global=false, Server=true, valid registered MethodName/owner | pending active supported binding | exact handler Method |

Therefore only the last row may enter a Definition query for the job's sake.
The Definition result then applies the kind/export/arity/context/async table
below. A method independently required by another conclusion may still be
queried, but its existence never changes this ScheduledJob metadata decision.

MethodName scalar is exactly
`CommonModule.<RegisteredCommonModuleName>.<MethodName>`, with a registered
profile-validated CommonModule. Definition compatibility accepts an exported
Procedure or Function with zero formal parameters and `is_async=false` for the
supported predefined row. Its BSL context is exact `ModuleDefault`, the
unannotated CommonModule shape in the primary examples; a function return is
ignored by the platform. A complete wrong export/arity/kind is
`scheduled_job_signature_mismatch`, `No`. An otherwise matching
`is_async=true` or explicit BSL context Definition is not primary-backed in v1:
it is `unsupported_scheduled_job_signature_variant`, `Unknown`, and never a
runtime root.

```text
ScheduledJobBindingV1 {
  registry_version,
  job,
  enabled = true,
  predefined = true,
  handler_module,
  handler_method,
  common_module_profile: ScheduledJobCommonModuleProfileV1 {
    global = false, server = true
  },
  callable_policy = ExportedSynchronousModuleDefaultZeroArityProcedureOrFunction,
  runtime_context = BindingRuntimeContextV1::Server,
}
```

`ScheduledJobActivationV1` is witnessed only at the registered job and exact Use
singleton. `NonPredefinedActivation` is witnessed only at the job, Use and
Predefined=false. The enabled binding whole fact is witnessed at MethodName,
Use, Predefined=true, module registration/name, Global, and Server. No runtime
edge exists before Definition compatibility, and no binding/handler observation
is constructed for the nonpredefined branch.

## 10. One neutral complete managed-Form registry

There must be one neutral registry, not a discovery copy. Extract the current
live `native_operations/form_event_registry.rs` matrices/context validation into
an infrastructure-neutral module used now by native form
edit/compile/validate and Task 5B. The registry has one stable version:

```text
PLATFORM_FORM_BINDING_REGISTRY = "platform-form-bindings/v2"
COMPLETE_FORM_METHOD_BINDINGS_ENCODER = "complete-form-method-bindings/v2"
FORM_MATCHING_BINDING_SET_ENCODER = "platform-form-matching-binding-set/v2"

PlatformFormBindingRegistryVersionV2 // opaque exact platform-form-bindings/v2

PlatformFormDocumentFlavorV2 (stable u16 tags)
  Plain    tag 1
  Borrowed tag 2

PlatformFormDocumentFlavorProblemV2 (stable u16 tags)
  DuplicateBaseForm      tag 1
  MisplacedBaseForm      tag 2
  WrongNamespaceBaseForm tag 3

PlatformFormDocumentFlavorAuthorityV2
  Known {
    flavor: PlatformFormDocumentFlavorV2,
    witnesses: PlatformFormDocumentFlavorWitnessSetV2,
  }                                                tag 1
  Inconclusive {
    problem: PlatformFormDocumentFlavorProblemV2,
    witnesses: PlatformFormDocumentFlavorWitnessSetV2,
  }                                                tag 2

PlatformFormDocumentFlavorWitnessSetV2 {
  registered_form_authority_digest:
    RegisteredPlatformFormAuthorityDigestV1,
  verified_form_content_fingerprint: SnapshotLeafFingerprintV1,
  root_span: PlatformXmlSourceSpanV1,
  base_form_spans: Vec<PlatformXmlSourceSpanV1>,
}

// Owned by the Task5B application/context module.
RegisteredPlatformFormVerificationV1<'context>
  NotApplicable(
    VerifiedRegisteredPlatformFormNotApplicableV1<'context>) tag 1
  Missing(
    VerifiedRegisteredPlatformFormMissingV1<'context>)       tag 2
  Present(
    VerifiedRegisteredPlatformFormV1<'context>)              tag 3

// Owned by the neutral platform-XML module.
parse_platform_form_v2(
  verified_form: &VerifiedRegisteredPlatformFormV1<'_>,
) -> Result<PlatformFormParseV2, PlatformFormParseErrorV2>

PlatformFormParseErrorV2 (stable u16 tags)
  VerifiedContentFingerprintMismatch tag 1
  ParserInvariant                   tag 2

PlatformFormParseV2 {
  registered_form_authority: RegisteredPlatformFormAuthorityBindingV1,
  document_flavor: PlatformFormDocumentFlavorAuthorityV2,
  method_bindings:
    Result<CompleteFormMethodBindingsV2, TypedFormCatalogFailureV2>,
}

parse_form_call_type(attribute: Option<&str>) -> Result<FormCallType, ...>
```

`VerifiedRegisteredPlatformFormV1` is one non-forgeable bound authority+bytes
object, not an `ArtifactRef` alias or a separately swappable handle/slice pair:

```text
VerifiedRegisteredPlatformFormV1<'context> {
  // all fields private; actual borrows of context-owned authority are retained
  source: AtomicSourceIdentityV2,
  source_fingerprint: &'context SourceFingerprintV1,
  configuration_catalog_digest: &'context Digest32,
  registered_form_catalog_digest: &'context Digest32,
  form: &'context ArtifactRef, // validated Form
  form_identity: ArtifactIdentityBytesV1,
  verified_form_content_fingerprint: SnapshotLeafFingerprintV1,
  verified_bytes: Box<[u8]>,
}

// Both records are owned by the Task5B application/context module. All fields
// and constructors are private to that module.
VerifiedRegisteredPlatformFormNotApplicableV1<'context> {
  source: AtomicSourceIdentityV2,
  source_fingerprint: &'context SourceFingerprintV1,
  configuration_catalog_digest: &'context Digest32,
  registered_form_catalog_digest: &'context Digest32,
  form: &'context ArtifactRef, // validated Form
  form_identity: ArtifactIdentityBytesV1,
}

VerifiedRegisteredPlatformFormMissingV1<'context> {
  source: AtomicSourceIdentityV2,
  source_fingerprint: &'context SourceFingerprintV1,
  configuration_catalog_digest: &'context Digest32,
  registered_form_catalog_digest: &'context Digest32,
  form: &'context ArtifactRef, // validated Form
  form_identity: ArtifactIdentityBytesV1,
}

pub(crate) fn assemble_verified_registered_platform_form_v1<'context>(
  source: AtomicSourceIdentityV2,
  source_fingerprint: &'context SourceFingerprintV1,
  configuration_catalog_digest: &'context Digest32,
  registered_form_catalog_digest: &'context Digest32,
  form: &'context ArtifactRef,
  form_identity: ArtifactIdentityBytesV1,
  verified_form_content_fingerprint: SnapshotLeafFingerprintV1,
  verified_bytes: Box<[u8]>,
) -> VerifiedRegisteredPlatformFormV1<'context>;
```

All fields are private/non-serde. The verification enum and its opaque
NotApplicable/Missing token records live in the Task5B application/context
module. Their constructors are private to
`PlatformCatalogContextV1::read_registered_platform_form_verified`; they expose
no state, key, path, fingerprint, relationship or Task4-wrapper accessor. The
Task5B context module imports the neutral Present wrapper, while the neutral
module imports neither the result enum nor either token. The crate-visible
Present factory is governed by the exact call-site whitelist below, so the sole
production ingress is
`PlatformCatalogContextV1::read_registered_platform_form_verified`. It first
validates the Form view's originating context, exact source, both catalog
digests, Form identity and the matching `SourceSetSnapshotV2`. Internally it
obtains `managed_form_xml_material()`, resolves that opaque ref through
`resolve_capture_material(snapshot, form_view)`, obtains the exact Task4 handle
through `expectation_for`, and invokes exactly once
`source_reader.read_registered_material_verified(snapshot, expectation)`.

The method maps Task4's exact result atomically: NotApplicable and verified
Missing become their typed opaque outcomes without exposing a state tag payload,
key, path, fingerprint, relationship or Task4 wrapper; Present immediately
copies the returned wrapper bytes through its private `bytes()` accessor into
the same newly constructed `VerifiedRegisteredPlatformFormV1` that retains the
context/source/catalog/Form/leaf authority. No caller supplies bytes or can swap
a wrapper after validation. This is exactly one bounded in-memory copy from the
Task4 wrapper and no second filesystem read, parser pass, path lookup,
raw `ArtifactRef`/digest constructor, hidden reader or public prepared phase.
Cross-context/source/catalog/Form/material/result replay fails before semantic
use; only external filesystem drift from the injected reader remains retryable.

This verification result, both no-material tokens, the Present wrapper and its
parser are capability/API state only. None is encoded by
`RegisteredFormCatalogIdentityBytesV1`, either query encoder, any semantic-group
encoder or an evidence record. Their tags therefore do not change a catalog/
query/group contract version, payload, digest or published golden; every
existing manifest/catalog/query/group golden remains byte-identical.

The dependency layout is acyclic and compile-checked:

```text
Task8 -> Task5B context/view API -> {Task4 SourceSnapshotPort, neutral Form API}
Task6 -> Task5B AnalysisBslMaterialScanPlanV1/read_analysis_bsl_material_verified -> Task4
native Form operations -> neutral Form parser core
neutral Form API -> neutral/domain types only
```

Only `VerifiedRegisteredPlatformFormV1`, the parser facade and the exact
`pub(crate) assemble_verified_registered_platform_form_v1(...)` factory live in
the neutral platform-XML module. Rust visibility is honestly crate-wide; no
unrelated-module privacy is claimed. Architecture seals the factory with a
product/static call-site whitelist permitting exactly one production caller:
the Task5B application context method above. Any second caller, re-export,
function pointer, alias or test-bypass outside the neutral module's own invariant
tests fails CI.
The application/context module alone owns
`RegisteredPlatformFormVerificationV1` and both opaque no-material token types;
private-constructor compile tests exercise all three branches from the context
method and reject token/result construction from neutral, native, Task6, Task8
or any other consumer module. Thus Missing/NotApplicable are constructible by
their owner without adding a reverse neutral-to-Task5B dependency.
The wrapper owns exact source/binding/leaf/bytes authority and constructively
borrows the context catalog's source fingerprint, both digests and Form; it has
no imaginary lifetime or Task4/Task5B catalog field. The accepted snapshot is
bound semantically by the exact verified source/leaf fingerprints before the
owned bytes are copied. Therefore the neutral
parser imports no discovery orchestration, Task4 handle/result, Task6 or Task8,
while Task8 is forbidden by the call-site whitelist from using the factory.
Native operations enter the
same private parser core through their existing native verified-input facade and
cannot construct the discovery wrapper or call its factory. The dependency is
only `Task5B application -> {Task4 port, neutral Form API}`; the neutral module
never imports a Task5B application child.

`document_flavor` is deliberately absent from the input handle: Plain/Borrowed
is a result of parsing the same Form.xml (`BaseForm`/action rules) and requiring
it beforehand would be circular.

`parse_platform_form_v2` accepts only the single verified wrapper and accesses
its private bytes internally. It never accepts a slice, second handle, catalog,
digest or path. A defensive internal hash check must equal the wrapper's retained
leaf before one bounded neutral pass returns the document-flavor authority and
complete method-binding result. No production caller can create the old
handle/bytes mismatch. Both projections copy the same private opaque snapshot
binding from the wrapper:

```text
RegisteredPlatformFormAuthorityBindingV1 {
  source: AtomicSourceIdentityV2,
  source_fingerprint: SourceFingerprintV1,
  configuration_catalog_digest: Digest32,
  registered_form_catalog_digest: Digest32,
  form_identity: ArtifactIdentityBytesV1,
  verified_form_content_fingerprint: SnapshotLeafFingerprintV1,
}

RegisteredPlatformFormAuthorityDigestV1(binding) =
  H("unica.registered-platform-form-authority/v1",
    bytes(AtomicSourceIdentityV2)
    || fingerprint32(source_fingerprint)
    || digest32(configuration_catalog_digest)
    || digest32(registered_form_catalog_digest)
    || ArtifactIdentityBytesV1(form)
    || fingerprint32(verified_form_content_fingerprint))
```

The error boundary is closed. Tag 1 is construction-impossible in production and
exists only for a test-injected corruption of the private wrapper's bytes versus
its retained leaf; tag 2 is any other impossible internal parser state. Both map
to nonretryable `Failed(platform_xml_parser_invariant)` with zero records.
Retryable external drift is returned earlier by the injected Task4 reader and no
verified wrapper/parser call occurs. Raw parser strings, slices, handles and
resource variants do not cross this API. Invalid UTF-8/XML/envelope and
byte/depth/node N+1 are snapshot-fatal capture failures before a handle/parser
call; semantic well-formed binding defects are the typed binding result, not an
outer error. Compile-fail REDs reject cross-wrapper/content construction and raw
slice calls; a private test-only corruption exercises tag-1 zero semantic parse
callbacks, an injected parser invariant exercises tag 2, and capture limits
cannot be downcast to either tag.

Here `configuration_catalog_digest` is the exact source/snapshot-bound shared
configuration-catalog digest from section 5; it is deliberately not called
semantic. The binding and digest smart types have private fields and no raw
constructor or serde; the parser copies the exact binding from the validated
handle. Both are excluded from the complete Form
`catalog_semantic_digest`, because that latter digest remains source-free Form
semantics, but the binding is stored privately in
`CompleteFormMethodBindingsV2` and exposed read-only by its view together with
its derived digest. Every Task5B/future-consumer use must compare exact binding
equality with the current `VerifiedRegisteredPlatformFormV1` authority before
lookup. Replay of a complete result after changing the source, either catalog
digest, Form identity or leaf returns the closed
`InvalidFormMethodLookupV2::RegisteredAuthorityMismatch` before the Method is
inspected, never Unbound. A semantic context/view/snapshot mismatch detected
before wrapper construction is instead nonretryable
`registered_material_handle_mismatch` before reader/filesystem I/O. Native Form
operations keep their existing native verified-input authority, but both
wrappers call the same extracted neutral registry/parser core and the same
call-type/context functions. Native code does not import the discovery catalog
set and may not keep a second event matrix or semantic parser. Compile-fail and
recording-spy REDs reject field forging, a nonregistered Form, Ordinary/
Inconclusive FormType, wrong source, wrong configuration or registered-Form
catalog, wrong bytes and replay of one complete result under another Form.

For the Analysis Managed+Present fixture in section 5, the exact authority
payload length is 306, `SHA-256(payload)` is
`a3c227cf50383c310abc68c3dc959c6a5783e2890f3c39843cb41ecc0264b3ca`,
and `RegisteredPlatformFormAuthorityDigestV1` is
`1ca64ff3e092c6571adf6929f38223b76c0cfc8311b2e0ed6bbdf446d79b1bf1`.
Tests rebuild the payload, change only either catalog digest or the leaf
fingerprint, and reject replay under each changed authority.

The flavor projection considers only the exact logform root and every same-local
`BaseForm` observation. With no defects, zero direct exact-namespace BaseForm is
Known Plain and exactly one is Known Borrowed. More than one direct exact node
adds `DuplicateBaseForm`; an exact same-local descendant outside the direct
position adds `MisplacedBaseForm`; a foreign-namespace same-local node adds
`WrongNamespaceBaseForm`. The parser computes the complete defect set and
selects the lowest tag under every node permutation. The opaque witness set
always carries the verified root span and all same-local BaseForm spans, sorted
unique and bound to the same handle digest/fingerprint; its spans follow the
section-5 `PlatformXmlSourceSpanV1` validation. Witness bytes are excluded from
the source-free Form semantic digest.

The binding projection consumes the Known flavor but is otherwise independent.
An unsupported Event, Action, main-attribute context or binding-owner defect may
return `TypedFormCatalogFailureV2` while the Known Plain/Borrowed authority stays
byte-identical and remains usable for the CFE ownership companion. A flavor
Inconclusive forces the binding projection to
`MalformedRegisteredMaterial` at the selected flavor span, but a binding-only
failure never rewrites flavor. MetadataCatalog consumes only Known flavor;
FormInspection and Task 8 method lookup require the complete binding projection.
Task 8 must compare both projections to the same registered authority before
mutation. No consumer invokes a second flavor/binding parse or reconstructs
witness spans.

`TypedFormCatalogFailureV2` is a closed semantic/resource failure, not an
opaque parser string and not a catch-all invariant:

```text
TypedFormCatalogFailureV2 (stable u16 tags)
  1 UnsupportedMainAttributeContext {
      location: optional validated PlatformXmlSourceSpanV1
    }
  2 MalformedRegisteredMaterial {
      location: optional validated PlatformXmlSourceSpanV1
    }
```

The whole-document audit collects the complete set of reachable closed defects
before choosing a failure. It selects the lowest stable variant tag; among
several observations of that variant it selects the smallest canonical source
span `(start_byte, end_byte_exclusive)` after validating the span against the
same verified bytes. XML traversal/insertion order, namespace prefix and
attribute order therefore cannot select a different reason. Unknown defect
variants, count overflow, invalid spans and an impossible parser state do
**not** construct this enum.

The provider adapter mapping is exact:

| Typed failure | Provider outcome/reason | Gap scope |
| --- | --- | --- |
| `UnsupportedMainAttributeContext` | descriptor-local `Bounded`, `unsupported_form_main_attribute_context` | exact `gap_artifact_projection_v2(FormMaterialScopeV1)` |
| `MalformedRegisteredMaterial` | descriptor-local `Bounded`, `malformed_registered_material` | exact `gap_artifact_projection_v2(FormMaterialScopeV1)` |

That projection contains only real source-qualified Form, requested
FormCommands, runtime Method/owner artifacts and both halves of every applicable
pair. No partial binding catalog, structural element path or fake FormElement is
projected. Other complete Forms in the same invocation survive.

There is deliberately no second Form-binding or matching-binding resource cap.
Every binding consumes at least one node already counted by the outer exact
`MAX_XML_NODES=1_000_000` capture boundary; total and per-method matching counts
are therefore `<=1_000_000`, fit checked u32, and cannot reach u32 overflow.
Adding a nominal 1,000,000/1,000,001 Form-binding RED would be unreachable
because the containing Form/owner nodes consume the same budget. Capture-invalid
UTF-8/XML/envelope/namespace and document byte/depth/node N+1 fail snapshot
capture before this parser. A semantic context/source/catalog/Form/material/
state mismatch is nonretryable `registered_material_handle_mismatch` before the
injected reader or filesystem I/O and constructs no wrapper. Later external
filesystem identity/content drift is retryable `source_fingerprint_mismatch`,
discards the whole staged provider batch and likewise never calls the parser.
An impossible neutral-parser invariant is an outer nonretryable provider failure
`platform_xml_parser_invariant` with zero records, not a third typed catalog
failure and not a descriptor-local Bounded claim. No adapter may translate
either outer failure into one of the two artifact-scoped variants.

The neutral module exports only the Present wrapper/parser DTOs and imports no
discovery orchestration, Task4, Task 6 or Task 8. The Task5B context module owns
the any-source view, verification result enum and opaque NotApplicable/Missing
tokens, imports Task4 plus the neutral Present factory, and is their only
constructor. A compile-only fake future consumer starts from
context+typed-source+snapshot+injected-reader+Form ref, receives the closed
result and can only pass Present to the wrapper-only parser; it constructs no
catalog, verification token, wrapper or raw byte input. A static product test
rejects any dependency edge from neutral Form code to Task5B/Task4/Task6/Task8
and any Task5B dependency on Task6/Task8. This is the Task5B-owned
future-consumer seam.
Actual Task 8 integration is a downstream Task 8 acceptance obligation and is
not a Task 5B RED, commit or test gate.

The complete V2 catalog is auxiliary snapshot-bound lookup authority. Task 5B
does not serialize it, embed it in `ProviderOutcome<EvidenceRecord>`, convert
FormEvent/ElementEvent rows to ProviderFacts, or make its row count consume
`maxEvidence`. Task 5B's FormCommand evidence projection borrows the catalog
only to validate the CommandAction rows it actually emits. Later Task 8 must
select its exact Analysis or Destination `PlatformCatalogViewV1` by typed source
identity, obtain the Form view, call the composite-context verification method, and
pass only a Present `VerifiedRegisteredPlatformFormV1` to this **same** neutral
parser. Typed NotApplicable/Missing outcomes carry no bytes/key/path and never
reach the parser;
it does not receive a catalog through provider-record transport. A second Form
parser, unchecked byte slice, display/path reconstruction, serialized catalog
handoff or reuse under another source/catalog handle is forbidden.

Any consumer-specific duplicate event list is a STOP. The extraction must first
pass a characterization run of the existing native Form tests before semantic
edits; after the v2 corrections in sections 10.2-10.4, the updated native REDs,
all unaffected native regressions, Task 5B tests and the fake-consumer compile
test must pass.

### 10.1 Closed event targets

Form events are exactly:

```text
OnCreateAtServer, OnOpen, BeforeClose, OnClose, NotificationProcessing,
ChoiceProcessing, ExternalEvent, OnReopen, OnMainServerAvailabilityChange,
OnReadAtServer, BeforeWrite, NewWriteProcessing, FillCheckProcessingAtServer,
BeforeWriteAtServer, OnWriteAtServer, AfterWriteAtServer, AfterWrite,
BeforeLoadDataFromSettingsAtServer, OnLoadDataFromSettingsAtServer,
OnSaveDataInSettingsAtServer, BeforeLoadUserSettingsAtServer,
OnLoadUserSettingsAtServer, OnSaveUserSettingsAtServer,
OnUpdateUserSettingSetAtServer, BeforeLoadVariantAtServer,
OnLoadVariantAtServer, OnSaveVariantAtServer, OnChangeDisplaySettings,
URLProcessing, URLListGetProcessing, URLGetProcessing, NavigationProcessing
```

Element registry and exact events:

| XML kind | Exact allowed events |
| --- | --- |
| InputField | OnChange, StartChoice, Clearing, ChoiceProcessing, AutoComplete, TextEditEnd, Opening, Creating, EditTextChange, Tuning, StartListChoice, MultipleValuesDelete |
| CheckBoxField | OnChange |
| RadioButtonField | OnChange |
| TrackBarField | OnChange |
| LabelDecoration | Click, URLProcessing |
| LabelField | URLProcessing, Click, OnChange |
| Table | Selection, OnActivateRow, BeforeAddRow, BeforeDeleteRow, OnStartEdit, OnChange, BeforeRowChange, AfterDeleteRow, OnEditEnd, OnActivateCell, OnGetDataAtServer, Drag, DragCheck, ValueChoice, ChoiceProcessing, DragStart, BeforeEditEnd, BeforeExpand, DragEnd, OnUpdateUserSettingSetAtServer, BeforeCollapse, BeforeLoadUserSettingsAtServer, OnActivateField, RefreshRequestProcessing, NewWriteProcessing, OnLoadUserSettingsAtServer, OnCurrentParentChange, OnSaveUserSettingsAtServer, URLGetProcessing |
| Pages | OnCurrentPageChange |
| PictureDecoration | Click, Drag, DragCheck |
| PictureField | Click |
| CalendarField | Selection, OnChange, OnPeriodOutput |
| ExtendedTooltip | URLProcessing, Click |
| FormattedDocumentField, TextDocumentField | OnChange |
| GraphicalSchemaField | Selection, OnActivate |
| HTMLDocumentField | OnClick, DocumentComplete |
| SpreadSheetDocumentField | DetailProcessing, Selection, OnActivate, AdditionalDetailProcessing, OnChange, Drag, URLProcessing, BeforePrint, BeforeWrite, DragCheck, OnChangeAreaContent |
| Page, Button, CommandBar, AutoCommandBar, UsualGroup, ButtonGroup, Popup | no events |

`Button` is explicitly non-event-capable in this registry. An Events node on a
Button makes the complete catalog unavailable. This corrects v3.

Object/record Form events `OnReadAtServer`, `BeforeWrite`,
`BeforeWriteAtServer`, `OnWriteAtServer`, `AfterWriteAtServer`, and `AfterWrite`
require a supported persistent main attribute exactly as the v2 registry below
defines it.
A Table event additionally requires a nonempty direct DataPath. Unknown inherited
context is incomplete, not positive evidence.

### 10.2 Closed structure, event-owner enumeration, and BaseForm semantics

The Form root namespace remains exact
`http://v8.1c.ru/8.3/xcf/logform`. Structure and binding discovery are two
bounded passes over the same exact-namespace tree; neither pass substitutes for
the other.

The structural pass is a binding-owner grammar, not a claim to validate the
entire managed-Form XDTO schema. It starts at these registered roots:

```text
/Form/ChildItems/<closed item and companion edges>
/Form/AutoCommandBar/<closed item and companion edges>
/Form/Commands/Command/Action
```

The exact grammar is:

```text
Form -> direct AutoCommandBar
Form | Container -> direct ChildItems -> RegistryItem*
RegistryItem -> direct ExtendedTooltip?
Table -> direct AutoCommandBar?

Container = UsualGroup | Pages | Page | Table | CommandBar | AutoCommandBar |
            ButtonGroup | Popup
RegistryItem = every XML kind in the section-10.1 element registry
```

`ExtendedTooltip` is therefore reachable as an event-capable companion even
though it is not under ChildItems. Other exact nonbinding Form/XDTO siblings are
outside this binding-owner grammar and are not rejected merely for existing.
However, an Events/Event or Commands/Command/Action below such an unregistered
wrapper is found by the independent binding-shaped-node audit and makes the
catalog incomplete; it cannot smuggle a nested recognized owner into the
grammar. A registry item at an illegal edge is likewise not consumed.

The independent event-owner pass enumerates the Form root plus every exact
logform descendant outside BaseForm that has a **direct exact-namespace**
`Events` child. Each such node must be either the Form root or a member of the
neutral event-target registry, and every non-root owner must also have been
consumed by the structural pass. This bounded descendant enumeration is required
so companion owners such as `ExtendedTooltip` cannot disappear merely because
they are not under `ChildItems`. Unknown, misplaced, or multiply consumed event
owners make the whole catalog unavailable. Commands and Actions remain only the
exact direct `/Form/Commands/Command/Action` material; a descendant Commands
lookalike is never promoted.

There is exactly one direct exact-namespace `/Form/Commands` collection; it may
be empty. Missing/duplicate/foreign same-local Commands makes the V2 catalog
incomplete rather than proving every requested command absent. Root Events and
each registered owner's Events are optional single direct collections; duplicate
or foreign binding-shaped lookalikes are incomplete.

There are zero or one direct BaseForm nodes. Its presence classifies a borrowed
extension form; absence classifies a plain base/extension-owned form. The single
recognized BaseForm subtree is saved base material: it may supply the exact
main-attribute fallback used by the neutral context validator, but its Events,
Commands, and Actions are excluded from the extension-local binding projection
and from the unconsumed-node audit. Duplicate/misplaced BaseForm is incomplete.
No arbitrary descendant is treated as BaseForm.

### 10.3 Exact identifiers and persistent main-attribute context

Every non-root node consumed by the binding-owner grammar outside BaseForm has exactly
one unqualified canonical `name` and opaque `id`; the Form root takes identity
from its capture-validated registered Form descriptor. Item names are globally unique
across the complete local binding-owner structure, not merely within one direct
collection. Non-sentinel IDs are likewise globally unique. Exact `id="-1"` is a
platform serialization sentinel, not an identity: inside the binding-owner
grammar it is accepted only on AutoCommandBar and ExtendedTooltip and is excluded
from ID uniqueness. This deliberately does **not** claim that `-1` belongs only
to command bars: tracked live fixtures also use it on ExtendedTooltip and on
nonbinding companions outside this projection. Names remain identity-bearing and
unique even when the ID is `-1`.

Commands have exactly one unqualified canonical `name` and opaque non-sentinel
`id`; command names and IDs are each unique within the one direct
`/Form/Commands` collection. Item and Command identity domains do not collapse
into each other. Every identity retains exact source spelling/ranges; IDs are
never parsed numerically or case-folded.

Persistent object/record Form events use an exact QName-aware main-attribute
view. The two material namespace URIs are:

```text
FORM_LOGFORM_NS = "http://v8.1c.ru/8.3/xcf/logform"
FORM_V8_NS = "http://v8.1c.ru/8.1/data/core"
FORM_CURRENT_CONFIG_NS =
  "http://v8.1c.ru/8.1/data/enterprise/current-config"
```

The main-attribute selector is one exact direct-child grammar. All unqualified
names below mean expanded names in `FORM_LOGFORM_NS`; only the innermost type is
in `FORM_V8_NS`:

```text
{FORM_LOGFORM_NS}Form
  / {FORM_LOGFORM_NS}Attributes
  / {FORM_LOGFORM_NS}Attribute
      / {FORM_LOGFORM_NS}MainAttribute = exact lowercase true
      / {FORM_LOGFORM_NS}Type
          / {FORM_V8_NS}Type = QName scalar
```

There are zero or one direct `Attributes` under the recognized Form root.
Every candidate is a direct `Attribute` child; nested Attributes/Attribute
lookalikes never count. Each Attribute has zero or one direct MainAttribute.
Absent means non-main; present is one direct non-mixed scalar and must be exact
lowercase `true` or `false`. Duplicate, nested, mixed-content or other boolean
spelling makes the effective main-attribute view malformed. Across the direct
Attribute vector there are zero or one exact `true`; two true attributes are
malformed rather than “first wins”.

The selected main Attribute contains exactly one direct logform-namespace Type
wrapper. That wrapper contains exactly one direct `{FORM_V8_NS}Type` and no
second semantic Type child. A data-core Type below another wrapper/descendant,
under another Attribute, or elsewhere in Form is a decoy and cannot repair a
missing direct child. Duplicate logform Type wrappers, duplicate direct
data-core Type children, a foreign same-local wrapper/child, or mixed-content
scalar makes only the effective context view incomplete.

The data-core Type scalar is an XML QName. Its in-scope prefix spelling is
arbitrary, but it must resolve to exact `FORM_CURRENT_CONFIG_NS`. Stripping the
literal text `cfg:` without namespace resolution is forbidden. An unprefixed
value, a wrong-bound `cfg`, a wrong-namespace Type lookalike, multiple semantic
types, or an unresolved QName cannot prove persistent context.

The v2 live-supported persistent types are exact:

```text
CatalogObject.<Name>
DocumentObject.<Name>
BusinessProcessObject.<Name>
TaskObject.<Name>
ExchangePlanObject.<Name>
ChartOfCharacteristicTypesObject.<Name>
InformationRegisterRecordManager.<Name>
InformationRegisterRecordSet.<Name>
ConstantsSet
```

`DynamicList` is explicitly nonpersistent. Other well-formed current-
configuration families are Unknown for the v2 context predicate. Apply the same
relative direct-child grammar to the already-recognized single BaseForm body
from section 10.2. An exact local main Attribute wins. BaseForm is consulted only
when the complete local main-attribute view is well-formed and contains zero
true attributes; a malformed/duplicate local view cannot fall back. A missing
BaseForm view, unavailable inherited body, or malformed effective type makes
only a context-sensitive event incomplete, while unrelated valid bindings
remain parseable until whole-catalog validation applies its exact material gap.

### 10.4 Exact actions and one lexical call-type parser

There is one shared parser API for native Form validation and Task 5B, exported
through the neutral future-consumer seam:

```text
parse_form_call_type(attribute: Option<&str>)
  None             -> Direct
  Some("Before")   -> Before
  Some("After")    -> After
  Some("Override") -> Override
  Some("Direct" | "" | anything else) -> error
```

`Direct` is a semantic enum value produced only by an absent attribute; it is
not a legal XML token. `FormCallType::from_xml("Direct")` or any equivalent
consumer-specific parser is a Task 5A implementation defect. Native Form
validation must call the same API, so extraction cannot make an XML shape pass
discovery while native validation rejects it or vice versa.

Every Command must contain at least one valid direct Action. Zero Action is
`malformed_registered_material` and makes the entire Form catalog unavailable;
it is not command presence without a runtime binding.

For a plain form, a Command has exactly one Action and an absent `callType`. For a
borrowed extension form, v1 accepts:

- exactly one Action with absent callType (semantic Direct) or exact Before, After, or
  Override; or
- exactly two Actions consisting of one Before and one After.

More than two, duplicate call types, absent/Direct mixed with another action,
Override mixed with another action, literal `callType="Direct"`, empty/unknown/
case-variant callType, or empty handler makes the complete catalog unavailable.
`Override` is the current serializer/domain token corresponding to the
replacement/Around concept in the guide; consumers must not invent an `Around`
XML alias.

Event callType uses that same parser: absence means semantic Direct, literal
`Direct` is invalid, and any explicit callType is forbidden on a plain form.
Borrowed-extension behavior must pass the shared neutral registry. Duplicate
canonical event names under one owner remain invalid. No consumer relaxes the
shared rule independently.

### 10.5 Closed FormCommand Definition compatibility

`FORM_COMMAND_HANDLER_POLICY = "form-command-handlers/v1"` supports exactly the
platform-generated Form command shape proven by the primary sources in section
2. For each retained Action, the complete compatibility row is:

| Definition dimension | Compatible v1 value |
| --- | --- |
| owner | the exact registered Form's own `FormModule` |
| method | the Action's exact nonempty handler |
| callable | `Procedure` |
| formal parameters | exactly one, by-reference, without a default; name is nonmaterial |
| BSL context | explicit `AtClient` |
| `is_async` | `false` or `true` |
| Export | nonmaterial; exported and non-exported both pass |

The asynchronous row is intentional: the official guide binds an
`Async Procedure ...(Command)` to a Form command. It is not a reason to accept
an asynchronous Function, a different arity, or a non-client method.

Compatibility is tri-state and evaluated in this order:

1. wrong owner/method, `Function`, arity other than one, or
   `ModuleDefault | AtServer | AtServerNoContext` is complete exact
   `form_command_handler_signature_mismatch`, `No`;
2. after those hard dimensions match, `Val`/defaulted parameter or an observed
   `AtClientAtServer | AtClientAtServerNoContext` hybrid context not proven by
   the v1 primary rows is
   `unsupported_form_command_handler_signature_variant`, `Unknown`;
3. the exact row above is compatible;
4. missing, duplicate, conflicted, or gapped Definition material is `Unknown`
   under its exact Definition scope.

No action binding creates `handles` before this join. A complete Form catalog
may still prove command/action registration while its pending handler remains
Unknown. Task 7 consumes the joined result and never substitutes
`Procedure + any arity + ModuleDefault/AtClient` or re-evaluates this table.

### 10.6 Whole-document completeness

The parser emits:

```text
CompleteFormMethodBindingsV2 {
  registry_version: PlatformFormBindingRegistryVersionV2,
  registered_form_authority: RegisteredPlatformFormAuthorityBindingV1,
  form_document_flavor: PlatformFormDocumentFlavorV2,
  form,
  bindings: sorted FormEvent | ElementEvent | CommandAction,
  semantic_digest: Digest32,
}

CompleteFormMethodLookupV2
  Unbound
  Bound {
    matching_binding_set_digest: Digest32,
    matching_binding_count: NonZeroU32,
  }

trait PlatformFormParseV2View {
  fn registered_form_authority(
    &self,
  ) -> &RegisteredPlatformFormAuthorityBindingV1;
  fn document_flavor_authority(
    &self,
  ) -> &PlatformFormDocumentFlavorAuthorityV2;
  fn method_bindings(
    &self,
  ) -> Result<&CompleteFormMethodBindingsV2, &TypedFormCatalogFailureV2>;
}

trait CompleteFormMethodBindingsV2View {
  fn registry_version(&self) -> &PlatformFormBindingRegistryVersionV2;
  fn registered_form_authority(
    &self,
  ) -> &RegisteredPlatformFormAuthorityBindingV1;
  fn registered_form_authority_digest(&self)
    -> RegisteredPlatformFormAuthorityDigestV1;
  fn form_document_flavor(&self) -> PlatformFormDocumentFlavorV2;
  fn catalog_semantic_digest(&self) -> Digest32;
  fn lookup_method(
    &self,
    authority: &VerifiedRegisteredPlatformFormV1<'_>,
    method: &ArtifactRef,
  ) -> Result<CompleteFormMethodLookupV2, InvalidFormMethodLookupV2>;
}

InvalidFormMethodLookupV2 (stable u16 tags)
  1 RegisteredAuthorityMismatch
  2 InvalidArtifactRef
  3 WrongKind
  4 ForeignFormModule
```

A bounded audit counts every binding-shaped Events, Event, Commands, Command, and
Action outside the recognized BaseForm subtree, including wrong-namespace
lookalikes. Every counted node must be consumed exactly once. Unknown/unconsumed
nodes, unsupported target/event, invalid context/callType/action cardinality,
duplicate identity, or semantic limit makes the whole catalog unavailable. No
prefix can prove an unbound/non-event-handler Method.

The registry-version constructor is private and has exactly one value whose
identity bytes are `string("platform-form-bindings/v2")`; no arbitrary string,
Debug tag or downstream alias exists. `Unbound` is constructible only after the
whole-document audit above produced a complete catalog. `lookup_method` first
derives the private binding from `authority` and requires exact equality with
the result's stored binding; a mismatch returns
`RegisteredAuthorityMismatch` before inspecting the Method. It then reruns
`method.validate()` so an in-crate forged `ArtifactRef` returns
`InvalidArtifactRef`; it then requires exact kind Method or returns `WrongKind`.
Using the same centralized typed canonical-ref grammar as `ArtifactRef`
validation, it requires the exact parent `<this catalog Form>.FormModule` or
returns `ForeignFormModule`. It never parses opaque identity bytes or display
text. Only after those checks does it construct the sole
`ArtifactIdentityBytesV1`. `Bound` collects **all** bindings whose exact Method
identity equals that value, sorts and requires unique complete binding
identities, then encodes:

```text
FormBindingOwnerIdentityBytesV2 =
    u16be(FormRoot=1) || ArtifactIdentityBytesV1(form)
  | u16be(Element=2) || ArtifactIdentityBytesV1(form)
      || vec(ancestor/owner segments in structural order:
           string(exact registry item-kind token)
           || string(UnicodeLowercase(name)) || string(exact opaque id))
  | u16be(Command=3) || ArtifactIdentityBytesV1(form)
      || ArtifactIdentityBytesV1(FormCommand)

CompleteFormBindingIdentityBytesV2 =
    u16be(FormEvent=1) || bytes(owner identity)
      || string(exact registry event token) || u16be(FormCallType tag)
      || ArtifactIdentityBytesV1(method)
  | u16be(ElementEvent=2) || bytes(owner identity)
      || string(exact registry event token) || u16be(FormCallType tag)
      || ArtifactIdentityBytesV1(method)
  | u16be(CommandAction=3) || bytes(owner identity)
      || u16be(FormCallType tag) || ArtifactIdentityBytesV1(method)

matching_binding_set_digest =
  H("unica.platform-form-matching-binding-set/v2",
    vec(CompleteFormBindingIdentityBytesV2 sorted unique))

CompleteFormMethodBindingsIdentityBytesV2 =
  string("platform-form-bindings/v2")
  || u16be(form_document_flavor Plain=1 | Borrowed=2)
  || ArtifactIdentityBytesV1(form)
  || vec(CompleteFormBindingIdentityBytesV2 sorted unique)

catalog_semantic_digest =
  H("unica.complete-form-method-bindings/v2",
    CompleteFormMethodBindingsIdentityBytesV2)
```

The owner path is the complete parser-owned structural identity, not only a
display name; repeated `-1` sentinel IDs remain distinguishable through kind/
name/path. The four lookup errors contain no raw attacker text and have private
constructors. A foreign-form/wrong-kind/invalid method is an error, never
Unbound. Total and matching counts are checked u32 values and the outer
one-million-node capture bound proves they cannot overflow. Several legal
bindings naming the same method therefore return one opaque set digest plus the
complete nonzero count, never a first row.

The normative command golden uses plain Form
`Document.Order.Form.Main`, command
`Document.Order.Form.Main.Command.Run`, absent callType/semantic Direct and
Method `Document.Order.Form.Main.FormModule.Run`:

```text
Command owner identity length = 74
SHA-256(owner bytes) =
  d44aa3d4853efe07eb7f0a455553567ce81c47651796ef639ebbf401595ac788
CommandAction binding identity length = 127
SHA-256(binding bytes) =
  95b622b03266c890bdad1119c3f617f36b15f5af1401b32086e09efe6febe47b
one-binding set payload length = 131
SHA-256(set payload) =
  1c3821aa0fd02ca7d6bf5ccbab6e53f67ce4afda856c88255a81d2cd427bca6d
matching_binding_set_digest =
  319ac84d786426615c729da220356115dc1c242086d97cff2efc17cb8658dd37
complete catalog payload length = 192
SHA-256(catalog payload) =
  28f36fcc75e8ff582ba29cef432c6475f22d858a63879b62d04d6401e94cf76c
catalog_semantic_digest =
  fbc6cdf8316626cfe23b351884af77fbd16aeabe411199a54b238b97819a7ab9
```

Mandatory REDs cover zero/one/multiple matching bindings, reverse binding
order, two legal bindings for one method, case-equivalent Method identity,
foreign Form Method, forged registry version/digest, checked count agreement
with the outer XML-node ceiling, outer node N/N+1, and an incomplete catalog
that can never return Unbound. The fake future consumer sees
only registry version, catalog semantic digest and this opaque lookup; it cannot
inspect or copy the registry table.

Downstream obligation, not a Task 5B gate: Task 8 must later consume this same
V2 catalog independently for exact analysis and destination Forms; it may not
reuse one side, a BSL-only absence, or a command-only projection. Task 5B proves
only that this behavior is expressible through its typed any-source context API
and the neutral wrapper-only parser used by the fake consumer.

## 11. Remaining supported flows

### 11.1 Self-contained callback registry

The registry version is `platform-callbacks/v1` and has exactly four rows:

| ScriptVariant | Owner/module | Method | Callable/export | Parameters | BSL context | Async |
| --- | --- | --- | --- | --- | --- | --- |
| English | Document/ObjectModule | BeforeWrite | Procedure, Export NotRequired | 3, by-ref, no defaults | object-module server default | false |
| Russian | Document/ObjectModule | ПередЗаписью | Procedure, Export NotRequired | 3, by-ref, no defaults | object-module server default | false |
| English | CommonCommand/CommandModule | CommandProcessing | Procedure, Export NotRequired | 2, by-ref, no defaults | explicit AtClient | false |
| Russian | CommonCommand/CommandModule | ОбработкаКоманды | Procedure, Export NotRequired | 2, by-ref, no defaults | explicit AtClient | false |

Parameter names are not compatibility material. `Export NotRequired` accepts
either actual export spelling. The whole pending requirement binds registry
version, exact registered owner, exact module/method chain, callback slot,
callable shape, and context; a cross-owner or non-registry row is rejected before
EvidenceGraph.

The callback provider never rereads or reparses
`Configuration/Properties/ScriptVariant`. It borrows the once-built
catalog's exact `script_variant_authority`. Missing or Unknown never guesses a
callback row: noncallback facts survive and the callback gets exact scoped
`unsupported_platform_script_variant`. Inconclusive(problem) becomes the exact
callback-scoped malformed/Bounded catalog-authority gap, still without deleting
unrelated metadata facts. Known reuses only the domain Russian/English tag.
EvidenceGraph joins only the canonical selected row with Task 6 Definition. A
reader spy and static dependency test reject a second Configuration Properties
parser in callback or future Task8 code. A compatible definition creates the
runtime edge; a complete exact
kind/arity/context mismatch is No. An otherwise matching `is_async=true`,
unproven `Val`,
default/extra-optional, or the one official opposite-language row under the same
owner remains Unknown with `unsupported_callback_signature_variant` or
`unsupported_callback_alias_variant`; character-class alias inference is
forbidden.

### 11.2 Self-contained HTTP route shape

HTTPService route binding is witnessed at exact direct RootURL, URLTemplate
Template, HTTPMethod, Handler, service/template/method registration and names.
RootURL and Template are each at most 2048 UTF-8 bytes and their canonical
combined route at most 4096. RootURL is nonempty with no leading or trailing
slash. Template starts with exactly one slash; exact `/` and one meaningful
terminal slash are supported. The route is exactly `"/" + RootURL + Template`.

RootURL/Template reject a repeated or empty internal segment, `.`/`..` segment,
backslash, query marker, fragment marker, or control. The parser performs no
decoding, case folding, slash collapse, dot removal, or Unicode/percent
normalization; percent spelling/case, Unicode, braces, and a valid terminal slash
are preserved byte-for-byte in the semantic value. The closed verb registry is
exact uppercase `GET | POST | PUT | PATCH | DELETE | HEAD | OPTIONS`. A wider
syntactically present route shape—including repeated slash, dot segment,
backslash, query, fragment, or control—or wider verb is scoped
`unsupported_http_route_variant`/`unsupported_http_method`, Unknown, and creates
no runtime edge; a missing/duplicate/mixed-content/over-limit singleton is
descriptor-local Bounded. Handler is
exactly a method in that same registered HTTPService's `Module`.

The descriptor cardinality is closed: one direct RootURL; direct URLTemplate
children with unique canonical registration names; exactly one direct Template
per URLTemplate; direct Method children with names unique within that template;
and exactly one direct HTTPMethod and Handler per Method. Nested/sibling decoys
never satisfy these fields. An empty templates/methods collection is complete
but emits no route; duplicate or malformed registration/singleton material is
descriptor-local Bounded with no partial route from that descriptor.

### 11.3 Closed HTTPService Definition compatibility

`HTTP_SERVICE_HANDLER_POLICY = "http-service-handlers/v1"` deliberately
supports only the platform-generated service handler shape demonstrated by the
official source in section 2:

| Definition dimension | Compatible v1 value |
| --- | --- |
| owner | the exact registered HTTPService's own `Module` |
| method | the route Handler's exact nonempty method |
| callable | `Function` |
| formal parameters | exactly one, by-reference, without a default; name is nonmaterial |
| BSL context | `ModuleDefault` (the unannotated HTTPService Module method) |
| `is_async` | `false` |
| Export | nonmaterial; exported and non-exported both pass |

Compatibility is tri-state and evaluated in this order:

1. wrong owner/method, `Procedure`, or arity other than one is complete exact
   `http_service_handler_signature_mismatch`, `No`;
2. after those hard dimensions match, `Val`/defaulted Request,
   `is_async=true`, or a context other than `ModuleDefault` is
   `unsupported_http_service_handler_signature_variant`, `Unknown`, because the
   v1 primary row does not prove that wider shape or that every explicit
   context is platform-invalid;
3. the exact row above is compatible;
4. missing, duplicate, conflicted, or gapped Definition material is `Unknown`
   under its exact Definition scope.

The provider does not infer a return value type from BSL syntax, but it does
require `Function`: a Procedure cannot return the platform response. The route
descriptor remains useful declarative evidence when the Definition is Unknown;
it creates no `handles` runtime edge. Task 7 imports the joined result and never
uses the previous guessed shortcut `Function + arity one + ModuleDefault` while
ignoring parameter/async material.

Report/DataProcessor ownership requires the complete specialized chain:

```text
owner contains Form
Form contains FormCommand
FormCommand handles FormModule.method
```

ExchangePlan never has a direct `handles` row. Its v1 runtime path is:

```text
ExchangePlan --uses--> EventSubscription
EventSubscription --subscribes--> validated CommonModule.method
```

The second edge exists only after the entire section-8 whole fact and Definition
join; one valid Type from an otherwise unsupported selected-source set cannot
create the mechanism.

## 12. Evidence witnesses and coordinates

Every record carries exact source freshness and the verified manifest path.
Multi-field whole facts are emitted at every contributing field; EvidenceGraph
collapses semantic values while retaining all evidence IDs.

Required witnesses include:

| Fact/requirement/gap | Exact witnesses |
| --- | --- |
| registration Present | registration entry and descriptor Properties/Name |
| requested Absent | exact completely scanned direct collection |
| BaseOwnedMetadataIdentityV1 | configuration flavor fields, object membership container/fields, object @uuid |
| ExtensionMetadataMembershipV1 | configuration flavor fields, wrapper @uuid, ObjectBelonging and ExtendedConfigurationObject or inspected Properties container for Own |
| EventSubscriptionBindingV1 | Source, every data-core Type plus its in-scope element/QName namespace declarations, Event, Handler, CommonModule registration/name, the five material capability fields |
| ScheduledJobActivationV1 | ScheduledJob registration/name and exact Use singleton |
| ScheduledJobNonPredefinedV1 | ScheduledJob registration/name, exact Use=true and exact Predefined=false singletons |
| ScheduledJobBindingV1 | enabled Use, MethodName, Predefined, CommonModule registration/name, Global and Server |
| Form event/context | exact event owner name/id/path, Event name/callType/handler, effective MainAttribute flag, data-core Type, and its QName namespace declaration when context-sensitive |
| Form command/binding | Command name/id and every Action including callType/handler |
| callback requirement | ScriptVariant plus exact registered owner/module/method-slot material |
| HTTP route binding | RootURL, URLTemplate Template/name, Method name, HTTPMethod, Handler, and all containing registrations |
| missing Form material | registered Form name/registration plus failed deterministic manifest key; no invented nonexistent-file location |
| result-limit gap | retained provider outcome plus exact dropped semantic-group subjects |

Coordinates map original verified UTF-8 bytes: one-based line/column, BOM not a
visible column, CRLF/bare CR/LF each one break, column by Unicode scalar from line
start. A normal mapping failure is a test failure; only an unexpected mapper
failure may omit coordinates while retaining exact path.

## 13. Failure classes, isolation, and stable reasons

Capture-authoritative invalid UTF-8/XML/namespace/envelope/identity/resource
material fails the snapshot with no manifest prefix. A well-formed capture-valid
descriptor with a malformed **material** mechanism view produces an atomic
descriptor-local Bounded gap; siblings survive. Materiality is conclusion-
specific: after exact ScheduledJob Use=false, Predefined/MethodName/profile/
Definition fields are not a view of the Disabled activation conclusion and do
not create its gap. Verified reader fingerprint/path/identity mutation discards
the whole staged provider batch as retryable unavailable.

A semantic snapshot/source/Form/kind/relationship/state/key/ordinary-entry mismatch at either
opaque material resolver or Task4 reader boundary is nonretryable
`SourceReadError::RegisteredMaterialHandleMismatch` with stable public reason
`registered_material_handle_mismatch`, zero filesystem/probe calls and zero
provider prefix. It is a caller/context contract defect, not source mutation;
an exactly equal reconstructed validated snapshot never fails by address.

The closed `PLATFORM_XML_STABLE_REASON_REGISTRY_V7` values used by this contract
are:

```text
foreign_metadata_namespace
analysis_not_base_owned
destination_not_extension_flavor
malformed_cfe_membership
unsupported_event_subscription_variant
unsupported_event_subscription_source_family
unsupported_event_subscription_all_objects_selector
unsupported_event_subscription_module_profile
unsupported_event_subscription_signature_variant
event_subscription_signature_mismatch
unsupported_scheduled_job_module_profile
unsupported_scheduled_job_signature_variant
scheduled_job_signature_mismatch
scheduled_job_disabled
non_predefined_scheduled_job_instance_unproven
unsupported_platform_script_variant
unsupported_callback_signature_variant
unsupported_callback_alias_variant
callback_signature_mismatch
unsupported_http_route_variant
unsupported_http_method
unsupported_http_service_handler_signature_variant
http_service_handler_signature_mismatch
unsupported_form_main_attribute_context
unsupported_form_command_handler_signature_variant
form_command_handler_signature_mismatch
registered_form_material_missing
unsupported_registered_form_type
registered_form_type_inconclusive
form_document_flavor_inconclusive
form_flavor_membership_mismatch
malformed_registered_material
platform_xml_result_limit
platform_xml_gap_limit
registered_material_handle_mismatch
platform_xml_parser_invariant
exact_artifact_spelling_collision
```

Reason strings never contain attacker-controlled raw token text. Exact bytes stay
bound by fingerprint and source locations.

Processing permutations preserve immutable bytes/fingerprints and must produce
byte-identical provider outcomes/report IDs. XML byte mutation creates a new
fingerprint and source-bound evidence/analysis IDs even if the source-free
semantic value is equal.

## 14. Mandatory minimal REDs

All REDs run before implementation and remain permanent regression tests.

### A. Task 5A/domain and CFE

1. analysis descriptor is an adopted extension wrapper whose wrapper UUID equals
   the destination extended UUID: no BaseOwned companion and no positive join;
2. analysis declared configuration but parsed ExtensionConfiguration: exact
   `analysis_not_base_owned`, Unknown;
3. destination declared extension but parsed BaseConfiguration: exact
   `destination_not_extension_flavor`, Unknown;
4. base Own UUID A + destination Adopted extended UUID A: only valid positive;
5. wrapper UUID equality without typed companions never promotes;
6. missing/post-limit companion yields exact pair gap, not partial positive;
7. accepted Task 5A rejects an EventSubscription encoded as generic AtServer
   callback and accepts only `BindingRuntimeContextV1::SameAsSourceEvent` pending
   requirement;
8. `DefinitionShape` equality/digest/conflict merge distinguish only the
   `is_async` bit, and the Task 5A constructor cannot discard it;
9. Task 5A owns versioned FormCommand/HTTP pending requirements but creates no
   `handles` edge before a compatible Definition join.
10. absent and exact `ObjectBelonging=Own` with no extended UUID produce the
    same shared-catalog Own authority: Base analysis emits BaseOwned; Extension
    destination emits typed Own and the application remains Unknown/not-adopted.
    Duplicate/unsupported/foreign/extended-UUID combinations are Inconclusive
    and never fall back to absent/Own; forward/reverse field order is identical.
11. ProviderFact tag 10 accepts only strict BaseConfiguration+Own and tag 11
    only strict ExtensionConfiguration Own=1/Adopted=2. In-crate constructor,
    compile-fail and exhaustive encoder tests reject tag-10 Adopted/Extension,
    tag-11 Absent/Base and any generic/raw state. A destination absent half
    emits only MetadataAbsent tag 2 and no companion; only Present may carry one
    tag-11 Own/Adopted companion. The updated Adopted payload/digest golden is
    reproduced and all insertion permutations preserve it.
12. an exact Managed Form pair makes MetadataCatalog independently read/parse
    both analysis and destination material; a spy proves no FormInspection
    outcome is consumed. Analysis or destination Form.xml Missing or flavor
    Inconclusive preserves that half's MetadataPresent, emits zero companion and
    the exact full effective-scope Bounded projection. A binding-only typed
    parser failure with Known matching flavor preserves the byte-identical
    companion while FormInspection/binding conclusions receive their exact gap.
13. Base+Own/Borrowed, Extension+Own/Borrowed and
    Extension+Adopted/Plain each emit exact
    `form_flavor_membership_mismatch`, the canonical sidecar/parser witnesses,
    zero companion and both pair halves plus all frozen command/runtime
    material; the other half may remain complete.
14. Known Ordinary and every FormType Inconclusive problem remain registered,
    retain both NotApplicable rows, perform zero registered-material verifier
    calls, zero managed Form.xml byte reads and zero XML parses, and map respectively to
    `unsupported_registered_form_type` / `registered_form_type_inconclusive`.
    Missing/duplicate/wrong-namespace/mixed/unsupported multi-defect and XML
    permutations select the lowest tag and identical gap/catalog bytes.

### B. Exact namespace and parser

1. arbitrary prefix bound to exact MDClasses URI passes with equal source-free
   semantic digest;
2. same bytes with prefix bound to `urn:1c` fail closed;
3. exact root plus foreign same-local direct Properties/ChildObjects/Name/uuid
   lookalike cannot satisfy or shadow cardinality;
4. foreign binding-shaped capture child is snapshot-fatal; mechanism child is
   descriptor-local Bounded;
5. exact namespace nested/wrapper/attribute decoys never repair direct fields;
6. DTD/entity, 64 MiB +1, depth 129, node 1,000,001 boundaries fail at the
   correct capture/provider phase before unbounded DOM allocation;
7. direct object UUID accepts lower/uppercase ASCII hex with one canonical
   lowercase digest; UUID on MetaDataObject/descendant/namespaced attribute and
   nil/braced/compact/URN/padded/non-ASCII forms fail with no fallback;
8. 512-byte/128-scalar identifier, 1024-byte qualified value, 256-byte ID/token,
   and every +1 boundary fail before set/digest/reason construction.
9. tracked exact-namespace `Configuration.xml` with one valid unqualified root
   UUID constructs Known authority; missing UUID, one namespaced lookalike,
   multiple same-local attributes, invalid and nil values select the exact
   reachable root problem. Missing/duplicate/foreign direct Configuration and a
   wrong root QName are capture-fatal and construct no catalog;
10. root/flavor/membership pairwise and representative three-defect fixtures
    compute the full problem set, select the lowest stable tag and reproduce the
    multi-defect goldens under every child/attribute permutation; catalog-entry
    and catalog-set input permutations recompute identical bytes/digests.
11. ScriptVariant covers Missing, both Known tags, exact Unknown token and every
    Inconclusive problem; NamePrefix covers Missing/Empty/Value and every
    Inconclusive problem. Duplicate/foreign/mixed/invalid N/N+1 permutations
    select lowest tags and reproduce catalog goldens; callback/fake Task8 reader
    spies prove zero second Configuration Properties reads/parses.
12. both optional flavor properties pass exact singleton boundaries;
    compatibility empty/control/257-byte/129-scalar and KeepMapping case/empty/
    unknown/duplicate/foreign/mixed values add exact tag 6 without changing XML
    order semantics.
13. empty, Managed Present, Form.xml Missing, FormModule Missing, Ordinary,
    every FormType problem, Adopted destination and two-source registered-Form
    catalog goldens mechanically rebuild. Key/state/fingerprint mutation changes
    the digest. Every sidecar manifest-key field reaches only Task4's sealed
    u32 catalog-string encoder: valid UTF-8 byte-length `N`/`N+1` fixtures have
    exact four-byte prefixes, 4,096 frames as `00 00 10 00`, 4,097 cannot be
    constructed, and a static/API test rejects a Task5B raw/length accessor,
    local prefix writer, u64 identity-encoder call or alternate framing. The
    exact Managed+Own+both-Present entry remains 271 bytes with its published
    payload and digest. A recording Task6 fake can consume FormModule only through the
    complete context-owned `AnalysisBslMaterialScanPlanV1` and dispatcher; it
    cannot name or resolve a Form material ref, expectation handle or Task4
    capture projection. The separate
    Task8 fake selects either typed source and gives only its Form view to the
    composite-context FormXml verification method; it cannot resolve or observe
    a material ref/handle. Path/suffix formatting, raw expectation/path/tuple reader input,
    a second material enum, a config-only context and catalog-digest replay are
    rejected. Semantically different snapshot/source, another Form, FormXml/
    FormModule kind swap, relationship/state replay and stale catalog fail before
    verifier/byte I/O; a byte-identical independently reconstructed validated
    snapshot passes without pointer identity. Captured Missing that appears
    before either consumer call returns `source_fingerprint_mismatch` and never
    a stale negative. All catalog/set numeric contract versions are exact private
    u16 constants 1; Task6 encodes imported registered-Form version 1, while
    mutation to 2 changes bytes and production construction rejects.
14. the exact object-safe port signature accepts
    `(&SourceSnapshotV2, &dyn SourceSnapshotPort)`, visits the Analysis atomic
    snapshot before canonical unique Destinations and maps reader, semantic
    snapshot/catalog, parser-invariant and catalog-bound failures to the exact
    four `PlatformCatalogBuildErrorV1` variants/reasons. Two equal invocations
    with equal verified bytes return equal context/set digests; changing only
    diagnostic epoch also preserves them. A Task7 integration spy, not the port
    API, rejects a second production orchestration call.
15. the Task8 compile/runtime fake uses a real Analysis+Destination pair and
    cannot construct its mutation authority until it has checked both typed
    configuration flavors, ScriptVariants, destination NamePrefix, both root
    UUID authorities, object wrapper UUID/membership, nested-Form wrapper UUID/
    membership, Known Managed FormType, Analysis Plain/Destination Borrowed and
    both complete binding lookups. Dropping or cross-swapping any one authority
    fails compilation or returns the exact pre-I/O mismatch; catalog/set/
    composite digests alone cannot replace the typed checks.
16. the Analysis-BSL plan has one canonical merged Ordinary/
    RegisteredFormModule order. A claimed Present FormModule occupies exactly
    one registered item and causes one registered verifier plus one delegated
    ordinary byte read, never a second ordinary item/read/parse. Managed Missing
    and Ordinary/Inconclusive NotApplicable retain their canonical slots with
    zero byte reads; a captured unsupported ordinary `.bsl` remains one
    diagnostic item, while an unregistered/outside-capture Form-shaped decoy is
    absent. Builder-only `module()` returns the exact typed context-resolvable
    `Some(ArtifactRef)` for supported Ordinary and every registered FormModule,
    and `None` only for unsupported Ordinary; no consumer parses a path. The
    builder-only `admission_byte_length()` returns the exact equal
    `Some(length)` for ordinary/registered Present and `None` for Missing/
    NotApplicable; Task6 cannot call it. Forward/reverse construction gives the
    same item/admission order. The context-owned witness vector is constructible
    without borrowed Task4 handles, while each fresh plan position zips it with
    exactly one handle borrowing an actual snapshot-stored Task4 derived-index
    entry; a temporary/rebuilt handle, cardinality/order/state mismatch or raw
    key/path escape is rejected before admission/I/O.
17. Definition's final canonical module selection occurs before the one plan-
    owned file/byte admission budget, so unrelated material cannot suppress it.
    CodeSearch and CallGraph select all. CallGraph consumes one merged canonical
    cursor and may defer reads of stored target candidates, but an earlier
    unrelated Process item still consumes the conservative global budget; a
    terminal before a queried caller or referenced target yields the exact
    deterministic caller-scoped Bounded gap and no edge/Complete. Static tests
    reject a second plan, cursor, counter, order, admission pass or reread.
18. exact N/N+1 mixed-kind REDs freeze 20,000 files, 512 MiB total and 16 MiB
    per file, including a claimed FormModule on each boundary. Every state and
    unsupported item has a reproducible opaque diagnostic location; only
    Present supports checked range locations and an opaque typed cache locator.
    Cross-plan/context/snapshot/item replay maps exactly to nonretryable
    `registered_material_handle_mismatch` before I/O. Static scans reject raw
    keys/paths, suffix tests, direct ordinary/registered readers and raw cache
    paths in Task6. An oversized unsupported Ordinary item retains
    `module()==None` plus `FileBytesLimit` and Task6 selects exact
    `bsl_file_bytes_limit` without invoking the dispatcher. A common registered
    FormXml Present wrapper's location/cache projections return nonretryable
    handle mismatch; it cannot construct `VerifiedAnalysisBslMaterialV1`, and
    tests prove zero panic/unwrap and zero additional I/O.
19. Task4's sealed composite writer emits exactly the accepted raw 32-byte
    `CompositeSnapshotIdV2` with no frame/transport and has exactly one Task5B
    production caller. `PlatformCatalogExecutionBindingV1::write_identity_v1`
    delegates that first field, appends the two exact raw catalog-set digests,
    and produces a 96-byte golden. A bit mutation in each field changes binding
    equality/bytes; diagnostic epoch, allocation move and equal rebuild do not.
    Raw slice/array/string/serde construction, Task7 direct Task4 access,
    aliases/re-exports/function pointers and a second production caller fail
    compile/static tests.

### C. EventSubscription

1. empty Source is malformed and creates no subscribes edge;
2. the real serialized shape `{MDCLASSES_NS}Source` with direct
   `{data-core}Type` and a scalar QName whose arbitrary prefix resolves to
   current-config passes; independently changing either element URI or QName URI
   fails and a same-local MDClasses Type never repairs it;
3. unprefixed, undeclared, multi-colon, empty, and wrong-bound QName values fail
   without a string-prefix fallback; changing both arbitrary element/QName prefix
   spellings with the same two URIs preserves the source-free digest;
4. table-driven RED iterates all 13 BeforeWrite families and asserts exact root
   mapping, compatibility, signature class, and arity (4/2/3 as specified);
5. table-driven RED iterates all eight compatible BeforeDelete object families
   as SourceAndCancel/2 and all five incompatible register/constant families as
   unsupported/None;
6. compatible mixed sets: Catalog+Constant -> SourceAndCancel/2, all four
   record sets -> SourceCancelReplacement/3, all eight BeforeDelete objects ->
   SourceAndCancel/2;
7. incompatible mixed sets: Document+Catalog BeforeWrite, register+constant
   BeforeWrite, and any BeforeDelete set containing register/constant -> Unknown;
8. signature lookup for an incompatible event/family returns None; no catch-all
   SourceAndCancel fallback exists;
9. supported + unsupported/unregistered/wrong-URI Type -> no partial binding;
10. selected source order/prefix spelling changes source bytes but not source-free
   selected-set digest; exact source list remains in whole-fact digest;
11. duplicate canonical source identities separated by other tuple sort keys are
   still rejected; 256/257 source boundary passes/rejects before positive output;
12. complete SelectedEventSource companion set must exactly equal the descriptor;
    ExchangePlan `uses` set must exactly equal only the descriptor's
    ExchangePlanObject-filtered subset; missing/extra members prevent promotion;
13. recognized all-objects selector is scoped unsupported, never guessed as a
    named source;
14. Handler accepts only exact `CommonModule.<registered module>.<method>`;
    omitted/duplicated/wrong literal segment, wrong registered owner, or empty
    method creates no binding;
15. every one of the five material CommonModule capability fields missing/
   duplicate/wrong independently -> no binding and exact witness gap;
   ServerCall true/false alone does not change validity;
16. old AtServer-only Definition is not compatibility evidence; an exact same-
    action synchronous arity/export Definition is required;
17. complete wrong kind/export/arity -> No signature mismatch; missing/gapped
    Definition -> Unknown;
18. an otherwise exact `is_async=true` or explicit-context Definition is
    `unsupported_event_subscription_signature_variant`, Unknown, and creates no
    root; changing only that bit changes evidence/analysis identity.

### D. ScheduledJob

1. Use=true + Predefined=true + Global=false + Server=true + exported zero-arity
   Procedure joins; complete Use=false emits exact `scheduled_job_disabled`, No,
   and creates no root even when the Definition otherwise matches;
2. zero-arity Function is accepted and return ignored under the broad platform
   rule;
3. with Use=true, Predefined=true, Global=true, Server=false, or a missing/
   duplicate/nonboolean profile field -> Unknown exact profile gap; ServerCall
   true/false does not change validity;
4. Use=true + Predefined=false emits only NonPredefinedActivation with
   `non_predefined_scheduled_job_instance_unproven`, no handler/binding and no
   runtime root. Removing, duplicating or corrupting MethodName/profile material
   leaves the same source conclusion/digest and the reader/Definition spies show
   those fields/endpoints were never opened;
5. with Use=true, Predefined=true plus a one-parameter Definition is an exact
   mismatch; absent/malformed Predefined is an exact Predefined gap and opens no
   MethodName/profile/Definition material;
6. only with Use=true and Predefined=true, MethodName accepts exact
   `CommonModule.<registered module>.<method>`; omitted/duplicated/wrong literal
   segment is malformed;
7. with Use=true and Predefined=true, raw MethodName with missing module profile
   or Definition never creates a runtime edge;
8. exact exported synchronous zero-arity Procedure and Function pass; an
   otherwise exact async or explicit-context variant is
   `unsupported_scheduled_job_signature_variant`, Unknown, while exact wrong
   kind/export/arity is `scheduled_job_signature_mismatch`, No; missing,
   malformed, or conflicted Use remains scoped Unknown and is never defaulted to
   disabled;
9. Use=false plus absent/gapped Definition, absent/wrong MethodName, absent/
   malformed Predefined, or missing/wrong module profile remains exact
   `scheduled_job_disabled`, No, with zero Definition requirement for that job;
10. a capture-fatal ScheduledJob envelope produces no activation fact, while a
    capture-valid non-activation sibling defect cannot erase Disabled;
11. exact Use=true with missing/malformed Predefined, or Predefined=true with
    incomplete MethodName/profile, emits only the exact scoped gap and zero
    ProviderFacts/candidates/groups; exact Predefined=false classifies as the
    metadata-only NonPredefinedActivation group; forward/reverse record order
    gives identical v2 group bytes and material scopes. No incomplete descriptor
    ever looks like an observed runtime-positive activation.

### E. Form registry/catalog

1. neutral registry is imported by native form validation and Task 5B; a
   separate fake future consumer proves the exported API without importing Task
   6 or Task 8, and a static/product test rejects a second event matrix or a
   Task5B -> Task6/Task8 dependency;
2. every live Form/element event row passes once; unknown event fails once;
3. Button Events makes catalog incomplete; Pages OnCurrentPageChange passes;
4. RadioButtonField, TrackBarField, all document fields, CommandBar/
   AutoCommandBar, ButtonGroup, and Popup are not lost; a direct companion
   ExtendedTooltip event is enumerated and consumed exactly once even though it
   is not under ChildItems;
5. unknown/misplaced descendant event owner and a companion omitted by the
   structural pass make the whole catalog incomplete rather than disappear;
6. Table event without DataPath and persistent Form event with unknown main
   attribute are incomplete;
7. persistent main type passes with arbitrary prefixes bound to exact data-core
   Type, current-config QName and exact
   `http://v8.1c.ru/8.3/xcf/logform` wrapper URIs; arbitrary prefixes on all
   three exact URIs preserve the result. A foreign same-local Form/Attributes/
   Attribute/MainAttribute/logform-Type wrapper, wrong-bound literal `cfg`,
   wrong data-core Type URI, unprefixed/multiple types, and DynamicList cannot
   prove persistent context. Nested Attributes/Attribute/Type/data-core-Type
   decoys, duplicate direct Attributes, duplicate MainAttribute children, two
   exact true attributes, duplicate Type wrappers and duplicate direct data-core
   Type children are table-tested and never select a “first” value;
8. every exact supported persistent family, ConstantsSet, and both information-
   register record families are table-tested; an unlisted family is Unknown;
9. direct BaseForm fallback context is used, while BaseForm binding descendants
   do not appear as extension-local bindings. Zero direct BaseForm produces
   Known Plain, one produces Known Borrowed; duplicate, misplaced and foreign-
   namespace same-local observations produce the exact lowest-tag flavor
   Inconclusive authority and canonical opaque witness set;
10. shared call-type parser maps absent to Direct and accepts only present Before/
    After/Override; literal `callType="Direct"` and empty/case variants fail for
    both Event and Action, and native validation uses this same API;
11. zero Action is incomplete; regular one absent-callType Direct Action passes;
    every explicit regular callType fails;
12. borrowed extension one absent-callType Direct/Before/After/Override and exact
    Before+After pair pass; invalid mixtures/>2 fail;
13. duplicate names/IDs across different ChildItems branches fail globally;
    numeric-looking distinct IDs stay distinct; repeated `-1` is accepted only
    for AutoCommandBar/ExtendedTooltip inside the binding-owner projection while
    their names remain unique;
14. missing identity, wrong namespace,
   unknown item/illegal edge, and unconsumed binding-shaped node exercise whole-
   catalog completeness;
15. one Task5B-owned fake future consumer selects either exact analysis or
    destination source through `platform_catalog`, obtains the Form view, calls
    the atomic verification method with its injected reader and passes only a
    Present wrapper to the neutral parser; actual Task 8 adoption is a
    downstream non-gating acceptance obligation;
16. synchronous and asynchronous `&AtClient Procedure Handler(Command)` pass
    with either Export spelling; parameter name is nonmaterial;
17. Function, arity != 1, ModuleDefault/AtServer/AtServerNoContext are exact
    `form_command_handler_signature_mismatch`, No; Val/default or either hybrid
    context is `unsupported_form_command_handler_signature_variant`, Unknown;
    every non-compatible result leaves the command registered but creates no
    `handles` edge.
18. complete lookup returns Unbound only after the whole-document audit; one or
    several legal bindings for the same Method return Bound with the canonical
    full-set digest and NonZeroU32 count under forward/reverse order. A foreign-
    Form Method, forged registry/digest, incomplete catalog and first-row
    selection all fail closed; the outer node boundary proves the checked count
    cannot exceed 1,000,000;
19. registry version, complete catalog semantic digest, command golden and
    matching-set golden mechanically rebuild; the same fake future consumer can
    query both analysis and destination catalogs but cannot inspect/copy the
    event registry, construct any verification outcome/wrapper, supply parser
    bytes or construct Unbound/Bound directly.
20. each `TypedFormCatalogFailureV2` variant maps to its exact bounded reason and
    the same effective Form-scope artifact projection; paired and three-defect
    XML permutations always select the lowest stable tag/canonical span.
    Capture byte/depth/node N+1, pre-I/O semantic handle mismatch, later external
    fingerprint drift and an injected parser invariant follow their separate
    outer paths and can never be downcast to descriptor-local Bounded. A
    mechanical invariant proves every complete/matching binding count is
    bounded by the accepted outer node count; no unreachable separate binding-
    limit fixture exists.
21. the verification enum and opaque NotApplicable/Missing tokens are
    privately constructible only inside the Task5B context method and external
    field/token construction is compile-fail. The neutral
    `VerifiedRegisteredPlatformFormV1` is constructible only through its exact
    `pub(crate)` whitelisted Present factory; any same-crate reference, call,
    alias, re-export or function pointer outside the one production caller is a
    static/product-test failure, not a compile-fail claim. Exact context+typed
    source+matching snapshot+
    injected reader+Form ref passes for either role. Nonregistered/wrong-kind,
    wrong context/source/catalog/fingerprint/Form/material and cross-role replay
    fail before reader/filesystem I/O; raw bytes, a second handle/read and
    cross-Form wrapper/result replay fail before semantic lookup. Lookup
    separately returns the exact closed RegisteredAuthorityMismatch,
    InvalidArtifactRef, WrongKind and ForeignFormModule tags; a case-equivalent
    valid Method of the exact Form reaches the same Bound/Unbound lookup without
    consumer-side identity-byte or display parsing.
22. starting from a Known flavor+complete catalog, inject independently an
    unknown Event, malformed Action and unsupported main-attribute context: the
    binding projection becomes its exact typed failure while flavor authority,
    BaseOwned/Extension companion bytes and flavor witnesses remain identical.
    Inject duplicate/misplaced/foreign BaseForm instead: flavor becomes exact
    Inconclusive and the companion is gapped.
23. descriptor and Form.xml location records are obtained only from the two
    opaque context/parser witness resolvers. Missing/extra/duplicate/cross-source/
    cross-Form/key/fingerprint witness swaps fail context/lookup before evidence;
    spies prove exactly one
    `source_reader.read_registered_form_descriptor_verified(atomic_snapshot,
    &handle)`
    call, exactly
    one ordinary Present descriptor read inside that Task4 seam, and one shared
    guard/semantic pass over
    `VerifiedRegisteredFormDescriptorBytesV1::bytes()` per source-qualified
    captured Form handle during the public port's internal preparation,
    zero adapter-owned/second descriptor parse, zero witness-recovery parse, and
    exact span replay rejection. Form.xml verifier/read/parse counts remain the
    demand-specific Present/Missing/NotApplicable matrix in section 6.

### F. Missing material and record limits

1. missing exact-Managed catalog-registered Form.xml produces exactly one
   deduplicated registered-material verifier call, zero file-byte reads and
   zero XML parses, then exact material
   `Artifacts` projection containing Form + requested FormCommand + exact
   runtime Method + both source-qualified halves of every applicable pair;
2. it emits no FormCommand Absent and does not contaminate an unrelated Form;
   an unrequested exact-Managed+Present Form is still read by the full scan and
   uses an exact Form-only scope; Ordinary/Inconclusive is classified/gapped
   with zero verifier calls, zero Form.xml byte reads and zero XML parses. More
   than 32 Managed siblings do not consume the
   request-enrichment 32-scope bound. Zero requested scopes with a nonempty
   sidecar still scans all entries; an empty sidecar performs zero verifier/
   byte-read/parse calls. Appearance after captured Missing returns exact
   `source_fingerprint_mismatch`, emits no stale gap/fact and discards the whole
   staged provider batch;
3. `maxEvidence=1` where a semantic group has two location witnesses retains or
   drops the whole group, never one witness;
4. one CFE pair half has MetadataPresent under one fact tag and its whole
   role-specific companion under another; a limit fitting only one record drops
   both in both insertion orders and gaps every pair depending on that half;
5. EventSubscription descriptor/binding and its derived ExchangePlan uses rows
   have different fact tags; a boundary through that cluster retains all or none,
   never a subscribes-without-uses or uses-without-descriptor prefix;
6. a FormCommandEvidenceCluster whose FormCommand polarity/contains/CommandAction
   binding crosses different fact tags is retained/dropped whole under
   forward/reverse record insertion; its gap contains Form + requested/emitted
   FormCommands + handler Methods + pair halves;
7. an ElementEvent-only auxiliary complete catalog changes its catalog digest
   and exact lookup but emits zero provider records, consumes zero maxEvidence
   and changes no evidence prefix/result-limit gap. Adding the same ElementEvent
   to a catalog with CommandAction evidence changes the auxiliary digest/lookup
   but not tag-4 record count/order/material. No FormElement kind, fake ref,
   hierarchy sentinel, display-path artifact or provider-record catalog
   transport is emitted;
8. a requested FormCommand is Absent and its tag-4 group is later dropped; the
   limit gap still contains the exact Form, absent command, every frozen runtime
   owner/Method subject and pair half even though no CommandAction handler record
   exists;
9. identical source-free semantic clusters in destination B and A, inserted in both
   orders with a limit fitting only one group -> canonical destination A/B source-
   set order chooses the same retained group and gaps the other;
10. an early two-record group that does not fit followed by a one-record group ->
   prefix-stop retains neither; both groups' exact material appears in the limit
   gap, never skip-and-continue;
11. records inside one cluster with fact-tag and location permutations serialize
   in the exact canonical inner order with no stable-sort fallback;
12. 256/257 gaps and 2,000/2,001 exact subjects produce stable exact/sentinel
    outcomes independent of order;
13. golden-byte tests freeze encoder primitives, all nine group variants,
    exact empty secondary payloads for EventSubscription/Definition/HTTP,
    analysis rank 0, every destination rank 1,
    two destination identity order, the complete five-field
    `ResolvedSourceSetIdentityBytesV1`, Unicode-lowercase
    `ArtifactIdentityBytesV1`, and fixed group/record SHA-256 values. A product
    compile fixture accepts exact `std::char::UNICODE_VERSION == (17,0,0)` and
    three one-component mutations each fail at compile time; this does not
    mutate or replace Task6's separate Unicode 16.0 grammar-category fixture;
14. changing only a dependent pair/subject changes the secondary digest;
    changing only source fingerprint preserves the source-free semantic digest
    and `AtomicSourceIdentityV2`/group-key bytes while changing the physical
    record digest and evidence/analysis identity; provider/coverage/location
    changes affect physical-record order but never the source-free digest;
15. source sets with equal display names but different kind, source format,
    relative root, or mapping digest have different source-identity/group-key
    bytes; destination input permutations produce one canonical logical order;
16. either case/Unicode-equivalent `ArtifactRef` in isolation (including an
    expanding lowercase mapping) is valid and produces the same identity,
    query, secondary, semantic, physical and group-key bytes as the other
    isolated spelling. Putting both exact spellings under one
    `AtomicSourceIdentityV2` in record/record, record/gap, group-derived material,
    binding or nested-material positions rejects the whole staged provider
    result with `exact_artifact_spelling_collision` before classification,
    sort, dedup or any ceiling, in both input orders. A spy observes no retained
    prefix/raw outcome/cache write. Equal semantic artifacts under distinct
    exact source identities remain separate registry keys.
17. planned-destination construction consumes the two complete CFE half-groups
    containing three fact variants; one/many physical witnesses per variant,
    reverse record/group order and duplicate exact bytes produce one canonical
    union of every retained evidence ID, while omission/addition, a split group,
    pair mismatch and checked `2 * u16::MAX`/overflow boundaries reject;
18. adding a second proposal for the same typed provider material preserves
    query/group/material/gap/prefix/raw-outcome bytes; only the downstream
    application association map may gain that proposal scope.
19. table-driven material-set REDs cover all nine private
    `SemanticAtomicEvidenceGroupV2` payload variants and mutate every
    direct/nested retained artifact independently. CFE and Form effective-scope
    pairs expand to both exact real halves; Support contributes only its exact
    subject, while planned-pair values remain query/validation authority.
    Pair/order permutations and byte-equal
    group/artifact duplicates produce one sorted-unique set, while an empty group list, empty
    provider Artifacts, fake proposal/mechanism/callback-slot material,
    and a pair token reject. A foreign-execution
    group is rejected by Task7's registry-owned shared derivation before this
    owner projection can become accepted state. Preview/prepare and finish
    recheck produce equal opaque sets. The set exposes no
    member/bytes/token/iterator/contains/query validator; only Task7's private canonical-
    sink material adapter can call its writer, and only the admission-scope
    encoder can call that adapter. Adding a tenth artifact-bearing group
    fails compilation until both exhaustive owner walks are updated.
20. sealed union-cardinality REDs require empty input = 0 and exact distinct
    complete-identity counts for singleton, disjoint, overlapping, duplicate-
    reference and permuted set cohorts. Checked u32 overflow rejects with no
    partial count. The API returns no member/byte/token/iterator/temporary union,
    accepts no threshold and has only Task7's exact private finish-validation
    counter as a production caller; Task7 alone tests 2,000 versus 2,001.

### G. Composite invocation and remaining flows

1. one Metadata invocation contains analysis plus canonical destination groups;
2. destination-A SourceSetWide gap leaves analysis/destination-B complete;
3. FormInspection runs analysis once and never destination;
4. the Task5B-owned MetadataComposite query constructor/golden bytes and digest
   change when any source identity, pair, presence key, Form runtime owner/
   method subject, or `max_records` changes; unchanged typed vectors under input
   permutations are byte-identical;
5. ExchangePlan requires complete uses+validated subscribes chain;
6. Report/DataProcessor requires full owner/Form/Command/Action chain;
7. all four exact callback rows pass; wrong owner/module/method/context/arity,
   missing/unknown ScriptVariant, opposite-language alias, and unproven Val/
   default variants produce the exact No/Unknown outcomes in section 11.1;
8. HTTP accepts `/` and one meaningful terminal slash, preserves percent
   spelling/case/Unicode/braces, and rejects repeated slash, dot segment,
   backslash, query, fragment, control, and 2048/4096 +1 boundaries without
   normalization;
9. all seven uppercase HTTP verbs pass; lowercase/custom verb is scoped
   unsupported; Handler must resolve to the same HTTPService Module;
10. a synchronous unannotated Function with exactly one by-reference,
    nondefaulted parameter passes with either Export spelling; Procedure or
    wrong arity is `http_service_handler_signature_mismatch`, No;
11. otherwise exact async/Val/default/non-ModuleDefault HTTP handlers are
    `unsupported_http_service_handler_signature_variant`, Unknown; missing or
    gapped Definition is Unknown, and none creates a runtime edge.
12. `MetadataCompositeQueryV2<'context>` and
    `FormSourceSetQueryV2<'context>` retain the same private whole-context
    borrow as `SupportStateQueryV2<'context>`; compile-fail fixtures reject each
    query outliving/moving away from that context. The borrow and binding add no
    query bytes. Each smart query mints an association authority owning the
    equal `PlatformCatalogExecutionBindingV1`; wrong context, composite,
    snapshot, either catalog-set digest, swapped port/authority or finish-time
    replay rejects before invocation allocation/output, while an equal rebuilt
    context+snapshot passes without pointer identity. The binding is never used
    as source/material membership authority. Task7's identity prefix accepts
    only the one binding moved from that sealed execution context; the complete
    finished projection carries it into the snapshot. Only the private
    canonical-sink binding adapter calls its writer once at the former three-
    field position; neither prefix nor snapshot accepts detached header fields
    or a caller-supplied second binding.
13. Task6 CodeSearch/Definition/CallGraph smart-query constructors each obtain
    exactly one binding from the same whole context and store it outside every
    query/cache/group/raw-outcome identity. Their owner authorities accept only
    full typed equality with Task7's registry binding; changing composite or
    either catalog-set digest rejects pre-I/O. Static tests find exactly these
    three additional projection calls, no fourth Task6 caller and no change to
    any of the six Task6 v3 query bytes/digests.

### H. Determinism and containment

1. reverse internal group/document/registration/record/gap insertion order on the
   same snapshot -> byte-identical outcomes, checks, analysis ID;
2. XML prefix/whitespace/line-ending byte mutation -> new fingerprint/evidence/
   analysis IDs even if semantic digest is equal;
3. Unix symlink/FIFO/device/content swap and Windows reparse/case/identity swap
   are rejected by verified reader;
4. production provider direct-filesystem import/static scan fails product tests.
5. static call-site tests freeze the Task4 composite writer, Task5B 96-byte
   binding writer, context-binding constructor, three authority validators,
   atomic-group/provider-gap material projections, sealed union-cardinality
   aggregate and admission-scope writer whitelists exactly. The context-binding
   whitelist includes exactly three Task5B query constructors, three Task6
   whole-context query constructors and Task7's registry constructor under the
   phased DAG. The Task4 composite writer is called only by
   `PlatformCatalogExecutionBindingV1::write_identity_v1`. The two Task5B
   binding/material-set writers are called only by their named private
   `CanonicalIdentitySinkV4` adapters; only the named prefix/admission-scope
   encoders can call those adapters, and no sibling obtains the sink's private
   `Vec`. The union-cardinality method has only Task7's named private finish-
   validation counter caller and returns no member/token/threshold decision. No
   callback, alias, re-export, function-pointer capture, raw DTO, second encoder
   or Task7 private-member walk is accepted.

## 15. Implementation order and STOP gates

0. Atomically co-freeze the exact Task4-v7 addendum, this Task5B-v7 contract,
   Task6-v2-v7 addendum and Task7-v7 addendum only after all four owner self-
   audits, cross-document encoder/API/DAG checks and no-P0/P1 independent
   reviews. Record all four exact design hashes plus exact Task6 generator/
   registry-manifest/audit/review evidence hashes only in
   `.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`;
   no individual document may freeze against a moving provider/consumer peer.
   This design-only gate does not require Task5A or production implementation
   OIDs.
1. As a prerequisite implementation slice, land every Task5A-owned
   section-3.1-through-3.10.1 RED/GREEN plus the shared registry type/API
   required by section 3.10.1,
   and
   GREEN for typed CFE companions, full source/gap/fingerprint identity, the
   opaque validated `ArtifactIdentityBytesV1` authority,
   dedicated same-action EventSubscription and ScheduledJob requirements,
   shared Form call-type parsing, FormCommand/HTTP policies,
   `DefinitionShape.is_async`, Support's three-digest binding and all nine
   atomic groups. Synchronize spec, review, accept and commit that slice; only
   then record `TASK5A_ACCEPTED_SHA` in the successor implementation report/
   ledger. No Task 5B production RED exists before
   this point. Any later mutation of that boundary invalidates the SHA and
   requires a new Task5A review/acceptance.
2. In a separate Task 4 successor slice, first land the section-3.11 REDs and
   GREEN for the neutral FormType authority/problem/span parser, dynamic
   registered-material expectation pairs, source/composite v2 fingerprints,
   the sealed exact raw composite-digest writer and its one Task5B caller
   whitelist,
   publication of live code's existing
   `MAX_SNAPSHOT_MANIFEST_KEY_BYTES=4,096` invariant and
   exact MDClasses URI. Flip the live wrong-URI test; synchronize Task 4 spec,
   run its complete snapshot/read suite, independently review, accept and
   commit the slice, then record exact `TASK4_V7_ACCEPTED_GIT_OID` in the
   successor implementation report/ledger. No Task 5B
   production RED exists before this point. Any later mutation invalidates the
   OID and requires a new Task 4 review/acceptance.
3. Only after both prerequisite IDs are recorded, land Task5B-owned provider/
   infrastructure/integration REDs, including the section-3.10.1 exhaustive
   catalog/provider-local walker/order tests and the sealed complete-context
   delta projection, sections 3.10.2-.3's sealed owner rechecks/authorities and
   section-3.10.4's opaque material-set, sealed union-cardinality and catalog-
   execution bridges/caller whitelists, that consume the frozen Task5A and
   Task4-v7 boundaries;
   do not patch either prerequisite behind its recorded ID.
4. Characterize and extract the live form event/context registry into one
   neutral module, then apply the v2 QName/identity/event-owner/call-type
   corrections through that shared boundary; native Form REDs and prior
   regressions must be green before discovery work.
5. Add the deterministic object-safe `PlatformCatalogPort` with the exact
   `build_context(&SourceSnapshotV2, &dyn SourceSnapshotPort)` boundary; the
   composite `PlatformCatalogContextV1` carrying both exact sets plus
   configuration, registered-Form and Analysis-BSL witness sets; the injected
   object-safe `SourceSnapshotPort` registered/captured readers; query
   constructors; composite source groups; the opaque Analysis-BSL scan plan,
   merged Ordinary/Registered admission cursor and receipt/cache locations;
   the three Task5B query-owner opaque association authorities in the
   application/query boundary (never infrastructure or Task7), each with its
   owned `PlatformCatalogExecutionBindingV1`; explicit private context
   lifetimes on Metadata/Form/Support queries; the exact 96-byte binding writer;
   exact Form material scopes; adapter-no-chaining static checks; exact one-
   descriptor-read/shared-pass counters per port invocation; repeat-build
   determinism; the 200,000-Form `O(N log N)`/zero-rescan RED; zero dynamic-
   material calls during port build; zero query-constructor I/O; and the
   demand-scoped provider verifier/read/parse matrix. Task7, not this port API,
   owns the production exactly-once call invariant. The context's sealed
   `stage_complete_catalog_spellings_v1` method is zero-I/O and its complete
   occurrence-projection REDs reject every omitted/foreign/source-substituted
   catalog or captured BSL artifact.
6. Implement analysis catalog projections and typed BaseOwned/Extension
   companions from the borrowed catalog set.
7. Implement EventSubscription registry/profile/whole-fact binding and its
   application compatibility join against recording fake Definition facts.
8. Implement ScheduledJob profile/whole-fact binding and its compatibility join
   against recording fake Definition facts.
9. Implement complete Form V2 catalog and public command projection.
10. Add HTTP/callback/ownership projections, witnesses, coordinates, atomic
    semantic-group limiting, and deterministic gap sentinel.
11. Compile-test the neutral query/catalog/Form APIs with separate recording
    future-consumer fakes. The Task8 fake must use the same typed any-source
    lookup and context-owned Form verification path for Analysis and
    Destination, including configuration flavor, ScriptVariant, NamePrefix,
    root UUID, object wrapper UUID/membership, nested-Form wrapper UUID/
    membership, document flavor and complete binding checks. The Task6 fake
    uses only `AnalysisBslMaterialScanPlanV1` plus the context-owned dispatcher:
    CodeSearch and CallGraph select all, Definition selects its final canonical
    modules before admission, and CallGraph uses one conservative merged cursor.
    Its three fake whole-context smart constructors each call
    `execution_binding_v1` once, retain the opaque value outside query identity
    and prove equal/full-typed binding acceptance plus composite/either-catalog
    mismatch rejection without opening a component. The Task7 fake proves only
    the reserved sealed union-cardinality caller and never receives a set member
    or threshold decision.
    Verify/report exact conformance to the already frozen owner-specific Task6/
    Task7 addenda. Do not import, implement, edit or test actual Task 6/7/8
    behavior in this Task5B slice.
12. Synchronize active spec/product contracts and run focused tests, full locked
    suite, fmt, clippy `-D warnings`, product contracts, Windows compile, and
    `git diff --check`.

STOP rather than broaden v1 when a platform shape lacks primary evidence or a
tracked fixture. STOP on any stale local-name-only MD parser, generic AtServer
subscription callback, plain UUID CFE join, per-source Task 7 Metadata call, or
second Form event registry. STOP if MetadataCatalog, FormInspection and
SupportState do not borrow the same once-built `PlatformCatalogContextV1`, or
if `MetadataCompositeQueryV2` or `FormSourceSetQueryV2` copies digests without
retaining its explicit private context lifetime, or any adapter calls/reparses
another adapter/raw catalog material. STOP if a
Task5B sidecar extracts a manifest-key spelling/length, writes its own u32
prefix or uses Task4's u64 identity projection for a catalog `string`. STOP if
catalog construction or a provider sorts, deduplicates, groups or limits before
its exact-spelling walk, chooses first/last/lexically-minimal spelling for an
alias-equal collision, or the valid context lacks the complete sealed zero-I/O
catalog/material delta gate and exposes a partial iterator/path/witness/handle
instead. STOP if Metadata/Form/Support query registration would require Task7
to inspect private members because the exact sealed exhaustive
`validate_committed_artifact_spellings_v1` owner method is absent, incomplete,
or exposes an iterator/member/delta. STOP if Task7 must inspect private
`ProviderGroupMaterialIdentityV2`/`SemanticAtomicEvidenceGroupV2` members or
trust a caller artifact list because either exact sealed association-material
owner recheck is absent, incomplete or non-exhaustive. Hard STOP if Task7 builds an
admission Artifacts vector by walking a group/provider member instead of
receiving `ProviderMaterialArtifactSetV2`, if that set is empty/noncanonical,
omits any of the nine variant's direct/nested artifacts or either half of a
material-bearing CFE/Form pair, or exposes a member/bytes/token/iterator/
contains/query
validator/general writer. STOP if its sealed union-cardinality aggregate returns
anything except checked `u32`, exposes a member/byte/token/iterator/temporary
union, accepts a threshold/limit, decides the 2,000 boundary, treats an empty
set slice as nonzero, or has a production caller other than Task7's exact
`RegistryFinishValidationV4::count_effective_gap_material_subjects_v4`.
STOP if the response-stage
overload cannot check baseline plus an opaque staged delta, mutates/commits the
delta, or lets Task7 inspect a private material/group member. STOP if
`SupportStateQueryV2` groups, encodes, compares authority, deduplicates or
applies its semantic bound before validating every raw source-qualified
artifact, adds a raw cap, erases a same-source exact alias, or maps it to any
reason other than `exact_artifact_spelling_collision`. Hard STOP if
Metadata/Form/Support lacks its exact owner-minted non-Clone/non-serde query-
association authority, exposes a raw/member-list constructor or iterator,
omits/accepts a foreign source/material, does not own and validate the exact
`PlatformCatalogExecutionBindingV1`, places the capability in infrastructure,
or requires Task7 to reconstruct membership from a digest. STOP if a production
caller other than the exact matching Task7 typed registration invokes
`association_authority_v1` or one registration mints it more than once. Hard
STOP if the catalog-execution binding can be built from three caller fields/an
analysis-only view, exposes a component, is serialized, is used as query
membership authority, differs from exact 96-byte order, or if its writer/context
constructor/validator or Task4 raw writer gains any caller outside the
section-3.10.4 whitelist. In particular STOP if the context projection omits
any of the exact three Task6 whole-context smart constructors, if one of those
constructors calls it twice or lets the binding enter a Task6 query/cache/group/
raw-outcome identity, or if any fourth Task6 production caller appears. STOP if
either Task7 sink adapter leaks its private
Vec/bytes/length/slice or gains another caller. STOP if Task7's identity prefix
or execution snapshot accepts the three detached header fields or any binding
not moved through the same sealed registry-owned `AnalysisIdentityPrefixV4` ->
finished-projection chain. Hard STOP if the build's
`std::char::UNICODE_VERSION` is not exactly `(17,0,0)`, if identity
lowercasing bypasses that const gate, or if the separate Task6 Unicode 16.0
grammar-category table is silently replaced by the identity version.

## 16. Acceptance and v7 self-audit gate

Implementation is accepted only when:

- every RED group A-H is green;
- the accepted Task 5A SHA is recorded and immutable;
- the exact accepted `TASK4_V7_ACCEPTED_GIT_OID` is recorded and immutable, and
  Task5B consumes its dynamic registered-material/context seam without a local
  FormType parser, suffix relationship, or path reconstruction;
- no CFE join can use topology labels/plain UUID facts;
- no EventSubscription runtime edge exists without all selected sources, exact
  event class, exact module capability profile, same-action context, and
  compatible synchronous Definition arity/kind/export;
- EventSubscription Source uses exact MDClasses Source, data-core Type elements,
  and current-config QName values; Handler/MethodName retain the literal
  `CommonModule` owner segment;
- no ScheduledJob edge exists without exact enabled/profile/Definition evidence;
- FormCommand and HTTP route facts remain pending until their exact versioned
  Definition policy joins; sync/async, kind, arity, parameter, context, and
  Export material follow sections 10.5/11.3 without guessed defaults;
- exact MDClasses namespace is enforced by capture and semantic views;
- `ArtifactIdentityBytesV1` exists once in Task 5A/domain with private bytes,
  sole validating construction from `ArtifactRef`, byte-based Eq/Ord/Hash and
  read-only borrowing; forged refs/raw bytes/deserialization and Task5B-local
  encoders are rejected; its exact `(17,0,0)` standard-library Unicode gate is
  a compile-time product assertion and remains distinct from Task6 grammar
  category tables;
- Platform catalog construction and every Task5B provider use the shared
  `ExactArtifactSpellingRegistryV1` primitive on every exact source-qualified
  raw catalog/record/gap/group/nested-material occurrence before semantic
  classification, sorting, deduplication or limits; the valid context exposes
  only the sealed zero-I/O `stage_complete_catalog_spellings_v1` delta gate,
  never a path/manifest/witness/handle or incomplete public iterator;
  same-source alias-equal different spellings reject in both orders while either
  isolated spelling remains valid and preserves all published query/group bytes;
- `MetadataCompositeQueryV2<'context>`, `FormSourceSetQueryV2<'context>` and
  `SupportStateQueryV2<'context>` each retain the same private whole-context
  borrow and
  expose only the sealed zero-I/O
  `validate_committed_artifact_spellings_v1(&registry)` recheck, exhaustively
  require every private nested artifact occurrence already committed, return no
  iterator/member/delta, and leave every query/golden byte unchanged;
- those same three query owners each mint one exact private-field, non-Clone,
  non-serde association authority from the accepted smart query; its only
  read-only projections are query digest, sealed context+snapshot execution-
  binding validation and closed source-group/material
  validation, every canonical query-time root is covered, foreign/omitted/
  swapped/digest/port/context/snapshot/catalog/raw-constructor mutations reject,
  each owns one equal opaque `PlatformCatalogExecutionBindingV1`, provider-returned roots
  still require finished-outcome proof, and no capability state enters query,
  cache, group, receipt or serialized bytes;
- `SupportStateQueryV2` additionally performs its own complete raw spelling
  walk before identity/grouping/deduplication/authority comparison and the
  4,096 semantic bound; both-order aliases reject with the exact closed reason,
  while 4,097 byte-identical contributions remain legal and collapse to one;
- for association/response-material artifact access,
  `ProviderGroupMaterialIdentityV2` and `SemanticAtomicEvidenceGroupV2` expose
  only the sealed zero-I/O committed and staged rechecks, exhaustively require
  every direct and nested source-qualified
  artifact already committed or admitted by the exact opaque delta, return no
  iterator/member/callback/delta, mutate neither authority, and force a compile failure when a new
  artifact-bearing private member lacks an explicit visit;
- `ProviderMaterialArtifactSetV2` is nonempty, immutable, sorted-unique and
  constructible only through exhaustive owner projections from all nine atomic
  group variants or exact provider Artifacts; every material-bearing CFE/Form
  pair becomes both real source-qualified halves, no fake/member/query
  authority or member/bytes/token/iterator/contains projection exists, and its only production writer
  path is the named Task7 canonical-sink adapter called solely by the sealed
  admission-scope identity encoder. Its only aggregate is
  `canonical_union_cardinality_v2`: empty input is zero, overlaps/duplicates/
  permutations count distinct complete identities once, overflow is checked,
  only `u32` is returned, no threshold is accepted/decided, and Task7's exact
  private finish-validation counter is its sole production caller;
- `PlatformCatalogExecutionBindingV1` is constructible only from the whole
  context, compares and validates the exact composite plus two catalog-set
  digests against context+snapshot, and writes exactly 96 raw bytes in that
  order. Its context-projection whitelist is exactly three Task5B smart queries,
  three Task6 whole-context smart queries and Task7's registry constructor under
  the phased co-freeze. Its first 32 bytes delegate to Task4's sole sealed composite writer;
  the Task4 writer, binding writer/context constructor/validators and all
  material-set projections/writer pass their exact static call-site whitelists
  under the phased section-3.10.4 rule: Task5B has zero downstream production
  writer/aggregate/validator callers; Task6 activates exactly three reserved
  whole-context query-constructor binding calls with unchanged six v3 query
  bytes/digests; final Task7 activates only the two private canonical-sink
  adapters, closed dispatches and sole union-cardinality counter without
  exposing their Vec, a set member or a binding component. Task7's
  identity prefix owns the binding obtained once from the same whole catalog
  context during private registry construction,
  and the complete finished projection alone carries it into the snapshot, with
  no three detached header fields;
- all registered-Form sidecar manifest-key `string` fields delegate through the
  Task5B private wrapper to Task4's sealed checked u32 catalog-string encoder;
  Task4's u64 identity encoder remains distinct, N/N+1 and 4,096/4,097 REDs pass,
  and the published 271-byte entry golden is unchanged;
- the exact object-safe catalog port accepts the composite `SourceSnapshotV2`
  plus injected reader, visits Analysis then canonical Destinations, and an
  equal repeated invocation returns equal catalog/set digests; Task7 alone
  proves exactly one production invocation per execution. Its composite context
  contains matching configuration/registered-Form sets plus configuration,
  registered-Form and Analysis-BSL snapshot-bound witness sets; MetadataCatalog,
  FormInspection, SupportState and query constructors borrow it with no config-
  only rebuild, adapter chaining, second MDClasses parser or location reparse;
- registered descriptor/material I/O is reachable only through the injected
  object-safe `&dyn SourceSnapshotPort`; no free/global/root reader or reader-
  carrying handle exists, the 200,000-Form build is `O(N log N)` with zero full
  rescans, semantic mismatches are nonretryable before I/O, and only external
  filesystem drift maps to retryable `source_fingerprint_mismatch`;
- Task6 can consume only the context-owned `AnalysisBslMaterialScanPlanV1` and
  dispatcher: claimed Present FormModule is yielded/read once, ordinary and
  registered obligations share one canonical admission budget, unsupported
  captured ordinary BSL remains a gap-capable item, material outside Task4
  capture is absent, and all receipt/cache locations stay opaque. Definition
  selects final modules before limits; CodeSearch and conservative CallGraph
  select all, with one CallGraph cursor and caller-scoped terminal gaps;
- the Analysis-BSL witness set is an owned, context-bound, nonsemantic sidecar;
  every plan zips its canonical entries with fresh handles borrowing Task4's
  snapshot-stored derived index, and no raw key/path/relationship or temporary
  handle escapes. A common registered FormXml wrapper cannot produce a BSL
  cache locator or final analysis-BSL wrapper: it returns nonretryable
  `registered_material_handle_mismatch` without panic/unwrap/I/O;
- one neutral Form registry is shared by all consumers, Button has no events,
  companion event owners cannot disappear, QName-aware main context and the one
  absent-to-Direct lexical parser are enforced, zero Action is incomplete, and
  one Task5B-owned fake future consumer selects typed Analysis/Destination views,
  checks configuration flavor, ScriptVariant, NamePrefix, root UUID, object and
  nested-Form wrapper UUID/membership, obtains each wrapper only through the
  context-owned verification method, checks Analysis Plain/Destination Borrowed,
  invokes the wrapper-only neutral parser and obtains both exact complete Bound/
  Unbound lookups without raw bytes, a second read or any actual Task8
  dependency;
- missing Form and result-limit gaps name every exact material subject;
- every Platform XML provider applies the shared v2 group classifier before any
  local record ceiling; the exported classifier is compile/golden-tested with
  fake Definition tag 8 and Support tag 9 records, and `maxEvidence` operates on
  complete cross-fact groups, so
  CFE half companions, EventSubscription descriptor/uses, and FormCommand
  evidence clusters cannot split by ProviderFact tag; auxiliary Form/Element
  event lookup rows never consume evidence admission;
- Task5B's typed query/recording fake proves one composite Metadata invocation
  with source-local groups; actual Task6/Task7/Task8 imports are explicitly
  downstream and non-gating;
- active spec/product contracts contain no superseded v3 wording;
- Task 4 regression, focused discovery tests, full suite, formatting, clippy,
  product contracts, Windows compile, and diff checks pass.

The delivery report `.superpowers/sdd/task-5b-report.md` records fixture/source
provenance, `TASK5A_ACCEPTED_SHA`, `TASK4_V7_ACCEPTED_GIT_OID`, the Task5B
implementation commit SHA, exact commands and results, provider/parser/registry
versions, accepted v7 design/review hashes and the published non-gating
successor-addendum references. It must not claim completed Task 6/7/8
implementation/back-propagation.

Current design-package status is never inferred from this closing clause.
Consumers consult only
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md`; implementation
authorization additionally consults the successor production reports/OID
ledger. The owner self-audit and independent review provide evidence to those
external authorities without self-embedding status or hashes here.
