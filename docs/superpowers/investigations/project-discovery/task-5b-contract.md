# Task 5B v5 — conditional authoritative contract for Platform XML evidence

Status: **design-ready, implementation blocked**, 2026-07-18.

This document supersedes the rejected Task 5B v4 contract whose immutable
SHA-256 is
`5c25c74d18b87799e0eea383e9a684d8674b4eefe98cfc2382f5f74fdb2df8bb`.
The v4 self-audit (`de13c4509d6333eceb63fde7f70fcd3818ad970fb878fbc6b9e278f5987fdf6d`)
was false: it claimed complete EventSubscription signature coverage while the
normative table omitted four register-record-set families and constants. v5
also closes the result-limit total-order, EventSubscription element-vs-QName
namespace layering, literal CommonModule owner segment, explicit-Direct, Form
event-owner traversal, Form main-type QName, identifier-scope ambiguities, and
the previously guessed FormCommand/HTTP Definition policies found during the
required re-audit. All v3/v4 briefs and reviews are historical;
do not implement from them.

The implementation gate is intentionally still closed. There is no accepted
Task 5A SHA at the time of this design. Before the first Task 5B RED, an owner
must accept and commit Task 5A with every back-propagation in section 3, and the
delivery worker must record the exact 40-hex commit as:

```text
TASK5A_ACCEPTED_SHA = <not yet available>
```

A dirty diff, a branch name, current `HEAD`, a topology label, or this document's
hash is not a substitute. Until `TASK5A_ACCEPTED_SHA` exists, v5 is only a
conditionally complete design.

## 1. Outcome and closed scope

Task 5B supplies two snapshot-bound providers and one shared parser family:

```text
PlatformXmlMetadataCatalogProvider
  provider = unica.platform_xml_catalog / 1

PlatformXmlFormInspectionProvider
  provider = unica.platform_xml_forms / 1
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
all of this section.

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

Task 5A/domain must separate activation from the positive binding requirement:

```text
ScheduledJobActivationV1 {
  job,
  state = Disabled | Enabled,
  exact_use_witness,
}
```

This smart-constructor fact needs only registered job identity plus one exact
direct boolean Use singleton. `Disabled` is complete runtime-activation No and
is independent of Predefined, MethodName, module profile, and Definition.
Missing/duplicate/mixed/nonboolean Use yields no activation fact and exact
Unknown; it is never defaulted.

Only `Enabled` can accompany `ScheduledJobBindingV1` as a pending whole fact
including the exact CommonModule capability profile and `Predefined` witnesses.
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
validation, Task 5B, and Task 8 must import that one constructor; no consumer may
open-code the present-attribute token list.

This correction is part of Task 5A acceptance because the current FormCallType
is a domain contract consumed by later providers. It must land with REDs proving
absent-to-Direct and present-literal-Direct rejection before Task 5B starts.

### 3.5 Atomic grouping identity

Task 5A/domain must leave every ProviderFact classifiable by the closed
`SemanticAtomicGroupIdV1` registry in section 6.1. In particular, existence
polarity and its role-specific CFE whole companion cannot be made unrelated by
different fact tags, and an EventSubscription descriptor cannot be separated
from its complete derived ExchangePlan uses projection. The grouping identity,
stable tags, canonical encoder, and classification invariants are internal—not a
new public wire/MCP field—but must have domain/application REDs before Task 5B
applies record limits.

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

The Task 5B delivery worker records `TASK5A_ACCEPTED_SHA` and verifies sections
3.1-3.7 above against code/tests/spec. Any difference is a STOP and Task 5A
is re-reviewed; infrastructure must not compensate locally.

## 4. Provider I/O and composite query architecture

Providers receive only typed query plans, the captured composite snapshot, a
verified snapshot reader, and an injected monotonic budget/clock. Every present
byte is read through `read_verified`. Provider production modules contain no
direct filesystem walk, existence probe, canonicalize, SQLite, CLI, or display
parsing.

One `MetadataCatalogQueryPlanV1` and one MetadataCatalog invocation cover:

1. the complete analysis registered catalog;
2. all analysis presence-query keys;
3. every exact requested destination membership pair.

The provider partitions that single invocation into deterministic internal
groups: analysis first, then destination source sets by canonical source-set
identity. `SourceSetWide` is local to one group. `QueryWide` is legal only when a
limitation invalidates the whole composite invocation. An unpaired destination
sibling is never read.

FormInspection is called exactly once for the analysis source. Destination
Form.xml is read by MetadataCatalog only when an exact destination Form pair
requires its mandatory material; it never creates a second FormInspection call.

The query plan contains sorted unique, source-qualified values:

```text
analysis_presence_query_keys       <= 64
command_presence_query_keys        <= 32
destination_membership_pairs       <= 64
form_material_scopes               <= 32 forms
form_material_subjects_per_form    <= 256
all exact gap material subjects    <= 2,000
```

Each `FormMaterialScopeV1` contains the exact Form, every requested FormCommand
under that Form, every exact proposal runtime owner/method subject derived from
the normalized request, and the applicable pair key. It is frozen before I/O and
enters the query digest. It is not reconstructed from a missing document and is
not a hierarchy wildcard.

Constructors reject +1 and any source/artifact mismatch before reader call 1.
Task/search text, filesystem order, `knownArtifacts`, and `maxCandidates` never
narrow these authoritative scans.

### 4.1 Task 7 exact back-propagation

Task 7 must describe the same single composite Metadata invocation, not one call
per source. Its scoped query variant is:

```text
ProviderQueryScope::MetadataComposite {
  composite_snapshot_id,
  analysis_source_set,
  destination_source_sets,
  pair_digest,
  presence_key_digest,
}
```

Its query digest includes every sorted internal group and pair. A source-local
gap remains material only to conclusions that depend on that group. The mandatory
RED is named:

```text
metadata_runs_once_for_composite_snapshot_and_form_runs_analysis_only
```

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
- a foreign same-local/binding-shaped direct child in a closed capture context is
  capture-fatal `foreign_metadata_namespace`;
- the same defect in a mechanism-only descriptor view makes that descriptor view
  atomically Bounded with `foreign_metadata_namespace`; sibling descriptors
  survive;
- an arbitrary foreign child that is not binding-shaped cannot repair or shadow a
  required field and is ignored only in an explicitly extensible view;
- `uuid`, Form item `name`/`id`, event `name`/`callType` attributes are exact
  unqualified attributes. Namespaced lookalikes never count.

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
descriptor kind/name, UUID, and nested Form/Template/Command identity. Capture
failure yields no manifest prefix and no provider invocation.

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

## 6. Completeness, missing Form material, and result limits

MetadataCatalog full-scans the analysis registration set and emits positive
registered facts independently of the requested negative-proof keys. Every
requested analysis key has exactly one Present/Absent polarity before limits.
FormInspection full-scans every registered analysis managed Form and owns
FormCommand Present/Absent plus `Form contains FormCommand`.

For every registered Form, expected material is exactly:

```text
<OwnerDirectory>/<OwnerName>/Forms/<FormName>/Ext/Form.xml
```

The expected key is derived from the capture catalog and checked against the
immutable manifest before I/O:

- absent key: no read call; exact Bounded `registered_form_material_missing`;
- present key: exactly one `read_verified`, then shared Form parser;
- forged/nonregistered spelling or later NotInManifest:
  `platform_xml_snapshot_catalog_mismatch`.

The missing-material gap's `Artifacts` scope is the exact frozen
`FormMaterialScopeV1`: Form, requested FormCommands, proposal runtime owner/method
subjects, and pair key. It never emits command absence, never means “all
descendants”, and never omits the exact runtime subject whose proof was lost.

### 6.1 Atomic semantic evidence groups

All semantic validation and conflict checks finish before `maxEvidence` is
applied. Retention is by canonical **semantic evidence group**, not individual
location record or individual ProviderFact. “One typed semantic value” is too
narrow because a single usable conclusion may require several fact variants.
Every emitted record is assigned exactly once through this closed internal
registry:

```text
SEMANTIC_ATOMIC_GROUP_REGISTRY = "semantic-evidence-groups/v1"

SemanticAtomicGroupIdV1 (stable tags in declaration order)
  1 StandaloneFact {
      source_set, fact_family, subject, relation?, object?, semantic_digest
    }
  2 CfePairHalf {
      role = Analysis | Destination, source_scoped_artifact
    }
  3 EventSubscriptionDescriptor { source_set, subscription }
  4 CompleteFormCatalog { source_set, form }
  5 ScheduledJobCluster {
      source_set, job, state = DisabledActivation | EnabledDescriptor
    }
  6 HttpServiceDescriptor { source_set, service }
  7 PlatformCallbackRequirement { source_set, owner, callback_slot }
```

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
- one `CompleteFormCatalog` contains every FormCommand polarity, structural
  contains row, Event/Action binding, and ownership projection derived from that
  Form's V2 catalog. The independent Form registration Present may remain
  standalone existence evidence;
- one `ScheduledJobCluster(DisabledActivation)` contains only the exact
  registered-job/Use=false activation-negative facts. One
  `ScheduledJobCluster(EnabledDescriptor)` contains Use=true and every distinct
  ProviderFact whose combination authorizes the positive binding descriptor.
  Disabled and Enabled are mutually exclusive for one complete exact Use
  singleton; non-activation fields never enter or gap the Disabled cluster;
- one `HttpServiceDescriptor` or `PlatformCallbackRequirement` contains every
  distinct ProviderFact whose combination authorizes that route/callback
  requirement;
- a fact that participates in none of those clusters is `StandaloneFact`.

This ScheduledJob branch does not weaken whole-descriptor atomicity. When Use is
false, no positive `ScheduledJobBindingV1` is constructed, so the complete
semantic conclusion is the Disabled activation cluster itself and there is no
descriptor half to split. When Use is true, the activation and all positive
descriptor facts are one EnabledDescriptor cluster. Every physical record still
belongs to exactly one cluster in either branch.

An unclassified record, one record assigned to two groups, a cluster assembled
from different source snapshots, or two conflicting semantic cluster values is a
provider contract violation before limits. A group is retained whole or dropped
whole; neither a location witness nor a different fact tag from the same cluster
can survive as an apparently complete prefix.

The group identity is source-scoped. Its strict total-order key is:

```text
(
  source_group_rank,              // analysis=0; destinations=1 in canonical order
  exact canonical source_set identity bytes,
  SemanticAtomicGroupIdV1 stable tag,
  group primary source-qualified subject canonical key,
  group secondary key / dependent-pair-set digest,
  complete cluster source-free semantic digest,
)
```

Fact stable tag is deliberately **not** a group-key discriminator. Within a
group, records are sorted and deduplicated by the exact canonical tuple
`(fact stable tag, source-qualified subject, relation tag, optional object,
typed payload digest, location presence/path UTF-8 bytes/line/column,
provider/coverage/freshness digest)`. `None` location sorts before `Some`; line
and column Options use absent-before-present and then numeric order. Equal
source-free clusters in two destinations are distinct because source-set
identity precedes the cluster digest. There is no stable-sort fallback to input
order at either level.

Retention is an atomic canonical prefix. Iterate groups in that total order and
retain whole groups while `retained_record_count + group.record_count <=
maxEvidence`. On the first group that does not fit, stop retaining: that group
and every later group form the dropped tail. The provider does **not** skip it
and continue with a smaller later group. Every dropped-tail group's exact
material subjects contribute to `platform_xml_result_limit`; if the first group
alone is too large, zero records are retained and its plus all later material is
gapped. A bounded heap/equivalent may compute the same prefix, but must produce
this exact behavior.

`material_subjects(group)` is closed:

- registration/presence: exact source-qualified artifact;
- CFE pair half: every dependent pair key plus that exact source-qualified half;
- EventSubscription descriptor: subscription, owner/handler method, every
  selected source, every derived ExchangePlan uses subject, and all dependent
  proposal/mechanism subjects;
- complete Form catalog: exact Form, every emitted FormCommand/event owner and
  handler method, plus dependent owner/pair/proposal subjects;
- disabled ScheduledJob activation: exact job and dependent runtime-activation
  proposal subjects only;
- enabled ScheduledJob/HTTP declarative descriptor: exact entry/route, owner,
  handler, and dependent proposal/mechanism subjects;
- platform callback requirement: exact owner, callback slot, and method subject;
- standalone fact: its exact subject and optional object plus dependent proposal
  subjects from the frozen query.

More than 256 gaps or more than 2,000 distinct exact affected subjects collapses
deterministically to one QueryWide `platform_xml_gap_limit`. Before that bound,
`platform_xml_result_limit` must not use an empty, approximate, or hierarchy-only
scope.

Complete silence is negative proof only inside the exact complete closed scan.
Bounded/unavailable/failed silence is always `Unknown`.

## 7. CFE destination membership extraction

The provider emits `BaseOwnedMetadataIdentityV1` only when all are true:

1. the query half is analysis;
2. the parsed configuration flavor is exact BaseConfiguration;
3. the exact registered object has Own shape: no direct ObjectBelonging and no
   direct ExtendedConfigurationObject;
4. its direct object UUID is valid;
5. the source/artifact/pair are exact query members.

It emits `ExtensionMetadataMembershipV1` only when the query half is destination
and parsed flavor is exact ExtensionConfiguration. Direct object membership is:

| ObjectBelonging | ExtendedConfigurationObject | Typed membership |
| --- | --- | --- |
| absent | absent | Own `{ wrapper_uuid }` |
| exact Adopted | one valid non-nil UUID | Adopted `{ wrapper_uuid, extended_uuid }` |
| anything else/duplicate | exact-pair Bounded malformed membership gap |

All contributing fields have descriptor witnesses. A destination Form pair also
requires its mandatory Form.xml material to be present and semantically readable
before membership can be promoted. Missing material leaves the companion gapped.

Provider completeness validates pair polarity and companion consistency before
limits. Present-without-companion, Absent-with-companion, out-of-plan half,
multiple semantic UUIDs/memberships, or source-wrong companion is a provider
contract violation. A later atomic result-limit drop is instead exact Bounded.

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
SCHEDULED_JOB_REGISTRY = "scheduled-jobs/v1"
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

Activation is parsed before the positive binding subview. `Use` is an exact
direct singleton and produces `ScheduledJobActivationV1`; only when it is true
do `Predefined`, `MethodName`, module profile, and Definition become material:

- `MethodName` + the two-field module profile + compatible Definition proves the
  declarative method binding;
- `Use=false` is complete supported negative authority for runtime activation:
  emit exact `scheduled_job_disabled`, `No`, and create no runtime root. The
  registered job/MethodName observation may survive, but Definition presence
  cannot override this negative. Missing/duplicate/wrong MethodName,
  Predefined, module-profile fields, or Definition do not downgrade this No and
  do not create a binding gap because they are nonmaterial after Disabled;
- `Use=true, Predefined=true` proves the v1 statically configured job instance
  shape. The platform guide states that predefined jobs have no parameters, so
  Definition compatibility requires exact arity zero;
- `Use=true, Predefined=false` proves only an enabled metadata template. A
  runtime instance and its parameter values are not present in source XML, so v1
  emits `non_predefined_scheduled_job_instance_unproven`, `Unknown`, and no
  runtime root;
- with Use=true, missing/malformed Predefined or MethodName is a descriptor-local
  malformed gap, never a default. Missing/duplicate/mixed/nonboolean Use itself
  is activation-scoped Unknown and cannot borrow Disabled.

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
| Use=true, Predefined=false | Unknown, `non_predefined_scheduled_job_instance_unproven` | none |
| Use=true with missing/malformed Predefined or MethodName | Unknown exact material gap | none |
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
singleton. The enabled binding whole fact is witnessed at MethodName, Use,
Predefined, module registration/name, Global, and Server. No runtime edge exists
before Definition compatibility. A declarative binding observation may survive
for a non-predefined job, but it is never an active Task 7 root without runtime-
instance evidence.

## 10. One neutral complete managed-Form registry

There must be one neutral registry, not a discovery copy. Extract the current
live `native_operations/form_event_registry.rs` matrices/context validation into
an infrastructure-neutral module used by form edit/compile/validate, Task 5B,
and Task 8. The registry has one stable version:

```text
PLATFORM_FORM_BINDING_REGISTRY = "platform-form-bindings/v2"
```

Any consumer-specific duplicate event list is a STOP. The extraction must first
pass a characterization run of the existing native Form tests before semantic
edits; after the v2 corrections in sections 10.2-10.4, the updated native REDs,
all unaffected native regressions, and then discovery/Task 8 tests must pass.

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
FORM_V8_NS = "http://v8.1c.ru/8.1/data/core"
FORM_CURRENT_CONFIG_NS =
  "http://v8.1c.ru/8.1/data/enterprise/current-config"
```

The type item is an exact `{FORM_V8_NS}Type` element. Its scalar is an XML QName;
the in-scope prefix spelling is arbitrary, but it must resolve to exact
`FORM_CURRENT_CONFIG_NS`. Stripping the literal text `cfg:` without namespace
resolution is forbidden. An unprefixed value, a wrong-bound `cfg`, a wrong-
namespace Type lookalike, multiple semantic types, or an unresolved QName cannot
prove persistent context.

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
configuration families are Unknown for the v2 context predicate. The exact
direct Form MainAttribute wins; a direct recognized BaseForm MainAttribute is
the fallback. A missing/inherited-unavailable or malformed effective type makes
only a context-sensitive event incomplete, while unrelated valid bindings
remain parseable until whole-catalog validation applies its exact material gap.

### 10.4 Exact actions and one lexical call-type parser

There is one shared parser API for native Form validation, Task 5B, and Task 8:

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
  registry_version,
  form_document_flavor,
  form,
  bindings: sorted FormEvent | ElementEvent | CommandAction,
  semantic_digest,
}
```

A bounded audit counts every binding-shaped Events, Event, Commands, Command, and
Action outside the recognized BaseForm subtree, including wrong-namespace
lookalikes. Every counted node must be consumed exactly once. Unknown/unconsumed
nodes, unsupported target/event, invalid context/callType/action cardinality,
duplicate identity, or semantic limit makes the whole catalog unavailable. No
prefix can prove Ordinary or unbound.

Task 8 consumes the same V2 catalog independently for exact analysis and
destination Forms. It must not reuse one side, a BSL-only absence, or a command-
only projection.

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

ScriptVariant comes only from the exact direct
`Configuration/Properties/ScriptVariant` singleton and is
`Missing | Known(Russian|English) | Unknown(exact bounded token)`. Missing or
Unknown never guesses a callback row: noncallback facts survive and the callback
gets exact scoped `unsupported_platform_script_variant`. Duplicate, mixed-
content, foreign same-local, or over-limit ScriptVariant is instead an exact
callback-scoped malformed/Bounded view gap, still without deleting unrelated
metadata facts. EvidenceGraph joins only the canonical selected row with Task 6
Definition. A compatible definition creates the runtime edge; a complete exact
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

Required stable v5 reasons include:

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
malformed_registered_material
platform_xml_result_limit
platform_xml_gap_limit
platform_xml_snapshot_catalog_mismatch
platform_xml_parser_invariant
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
3. with Use=true, Global=true, Server=false, or a missing/duplicate/nonboolean
   material field -> Unknown exact profile gap; ServerCall true/false does not
   change validity;
4. Use=true + Predefined=false retains declarative binding but yields
   `non_predefined_scheduled_job_instance_unproven`, no runtime root;
5. with Use=true, Predefined=true plus a one-parameter Definition is an exact
   mismatch; absent Predefined is malformed rather than defaulted;
6. with Use=true, MethodName accepts only exact
   `CommonModule.<registered module>.<method>`; omitted/duplicated/wrong literal
   segment is malformed;
7. with Use=true, raw MethodName with missing module profile or Definition never
   creates a runtime edge;
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
    capture-valid non-activation sibling defect cannot erase Disabled.

### E. Form registry/catalog

1. neutral registry is imported by native form validation, Task 5B, and Task 8;
   static/product test rejects a second event matrix;
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
   Type and current-config QName URIs; wrong-bound literal `cfg`, wrong Type URI,
   unprefixed/multiple types, and DynamicList cannot prove persistent context;
8. every exact supported persistent family, ConstantsSet, and both information-
   register record families are table-tested; an unlisted family is Unknown;
9. direct BaseForm fallback context is used, while BaseForm binding descendants
   do not appear as extension-local bindings;
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
15. Task 8 independently binds complete analysis and destination V2 catalogs.
16. synchronous and asynchronous `&AtClient Procedure Handler(Command)` pass
    with either Export spelling; parameter name is nonmaterial;
17. Function, arity != 1, ModuleDefault/AtServer/AtServerNoContext are exact
    `form_command_handler_signature_mismatch`, No; Val/default or either hybrid
    context is `unsupported_form_command_handler_signature_variant`, Unknown;
    every non-compatible result leaves the command registered but creates no
    `handles` edge.

### F. Missing material and record limits

1. missing registered Form.xml produces zero reader calls and exact material
   scope containing Form + requested FormCommand + exact runtime method + pair;
2. it emits no FormCommand Absent and does not contaminate an unrelated Form;
3. `maxEvidence=1` where a semantic group has two location witnesses retains or
   drops the whole group, never one witness;
4. one CFE pair half has MetadataPresent under one fact tag and its whole
   role-specific companion under another; a limit fitting only one record drops
   both in both insertion orders and gaps every pair depending on that half;
5. EventSubscription descriptor/binding and its derived ExchangePlan uses rows
   have different fact tags; a boundary through that cluster retains all or none,
   never a subscribes-without-uses or uses-without-descriptor prefix;
6. a CompleteFormCatalog whose FormCommand polarity/contains/Action binding cross
   different fact tags is retained/dropped whole under forward/reverse record
   insertion;
7. identical source-free semantic clusters in destination B and A, inserted in both
   orders with a limit fitting only one group -> canonical destination A/B source-
   set order chooses the same retained group and gaps the other;
8. an early two-record group that does not fit followed by a one-record group ->
   prefix-stop retains neither; both groups' exact material appears in the limit
   gap, never skip-and-continue;
9. records inside one cluster with fact-tag and location permutations serialize
   in the exact canonical inner order with no stable-sort fallback;
10. 256/257 gaps and 2,000/2,001 exact subjects produce stable exact/sentinel
    outcomes independent of order.

### G. Composite invocation and remaining flows

1. one Metadata invocation contains analysis plus canonical destination groups;
2. destination-A SourceSetWide gap leaves analysis/destination-B complete;
3. FormInspection runs analysis once and never destination;
4. Task 7 query digest changes when any pair/group/presence/material scope changes;
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

### H. Determinism and containment

1. reverse internal group/document/registration/record/gap insertion order on the
   same snapshot -> byte-identical outcomes, checks, analysis ID;
2. XML prefix/whitespace/line-ending byte mutation -> new fingerprint/evidence/
   analysis IDs even if semantic digest is equal;
3. Unix symlink/FIFO/device/content swap and Windows reparse/case/identity swap
   are rejected by verified reader;
4. production provider direct-filesystem import/static scan fails product tests.

## 15. Implementation order and STOP gates

1. **STOP** until Task 5A is clean, accepted, committed, and section 3 is present;
   record `TASK5A_ACCEPTED_SHA`.
2. Land domain/application REDs for typed CFE companions, dedicated same-action
   EventSubscription requirement, ScheduledJob profile requirement, shared Form
   call-type parsing, FormCommand/HTTP pending requirements and closed
   Definition policies, `DefinitionShape.is_async`, and cross-fact
   `SemanticAtomicGroupIdV1` classification.
3. Back-propagate exact MDClasses URI into the shared Task 4 capture parser and
   flip the live wrong-URI test; rerun all Task 4 snapshot tests.
4. Characterize and extract the live form event/context registry into one
   neutral module, then apply the v2 QName/identity/event-owner/call-type
   corrections through that shared boundary; native Form REDs and prior
   regressions must be green before discovery work.
5. Add query constructors, composite source groups, exact Form material scopes,
   and zero-I/O spies.
6. Implement analysis catalog and typed BaseOwned/Extension companions.
7. Implement EventSubscription registry/profile/whole-fact binding and Task 6
   Definition join.
8. Implement ScheduledJob profile/whole-fact binding and Definition join.
9. Implement complete Form V2 catalog and public command projection.
10. Add HTTP/callback/ownership projections, witnesses, coordinates, atomic
    semantic-group limiting, and deterministic gap sentinel.
11. Apply the exact Task 7 composite-invocation back-propagation and Task 8 V2
    catalog import. Do not edit Task 8 from a concurrent worktree; merge/re-audit
    its accepted result instead.
12. Synchronize active spec/product contracts and run focused tests, full locked
    suite, fmt, clippy `-D warnings`, product contracts, Windows compile, and
    `git diff --check`.

STOP rather than broaden v1 when a platform shape lacks primary evidence or a
tracked fixture. STOP on any stale local-name-only MD parser, generic AtServer
subscription callback, plain UUID CFE join, per-source Task 7 Metadata call, or
second Form event registry.

## 16. Acceptance and v5 self-audit gate

Implementation is accepted only when:

- every RED group A-H is green;
- the accepted Task 5A SHA is recorded and immutable;
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
- one neutral Form registry is shared by all consumers, Button has no events,
  companion event owners cannot disappear, QName-aware main context and the one
  absent-to-Direct lexical parser are enforced, zero Action is incomplete, and
  Task 8 uses complete V2 catalogs on both sides;
- missing Form and result-limit gaps name every exact material subject;
- `maxEvidence` operates on the closed cross-fact atomic grouping registry, so
  CFE half companions, EventSubscription descriptor/uses, and Form catalog
  clusters cannot split by ProviderFact tag;
- Task 7 performs one composite Metadata invocation with source-local groups;
- active spec/product contracts contain no superseded v3 wording;
- Task 4 regression, focused discovery tests, full suite, formatting, clippy,
  product contracts, Windows compile, and diff checks pass.

The delivery report `.superpowers/sdd/task-5b-report.md` records fixture/source
provenance, `TASK5A_ACCEPTED_SHA`, implementation commit SHA, exact commands and
results, provider/parser/registry versions, and completed Task 6/7/8 back-prop.

This design itself is not declared implementation-ready until the separate v5
self-audit has verified every P0/P1/P2 closure and published its final immutable
SHA-256 values.
