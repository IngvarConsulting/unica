# Task 5A runtime-port audit

Date: 2026-07-17

Scope: the P1 finding `negative runtime potential ports remain too broad` in
the uncommitted Task 5A slice. No tracked file was edited.

## Verdict

**The finding is real, but the proposed fix is not safe as written.**

`runtime_ports_for()` currently conflates two different contracts:

1. the closed set of ports which can contribute a positive runtime edge to
   `connection_ports`; and
2. the evidence/unsupported-variant conditions which authorize a conclusive
   negative.

The first set can be narrowed exactly from the live smart constructors and the
four-row callback registry. The second cannot be obtained by merely removing
ports. In particular, the accepted Task 5B contract says unregistered
lifecycle/command callbacks, direct ExchangePlan callbacks, BSP variants, and
cross-language aliases stay `unknown`, not negative proof. If MetadataCatalog
is removed and the remaining complete CallGraph set is fed to `.all(...)`, the
current validator returns false `No`/`Contradicted` for precisely those
unsupported variants.

Do not implement the review bullet list as a narrower `BTreeSet` switch alone.
Introduce one closed runtime-mechanism profile which separates positive ports
from negative policy.

## Evidence from the live contract

- `ValidatedBinding::event_subscription` and `scheduled_job` target exact
  CommonModule methods and are owned by MetadataCatalog
  (`model.rs:601-621`, `667-684`, `760-770`).
- `ValidatedBinding::form_command` targets an exact registered FormModule
  method and is owned by FormInspection (`model.rs:624-665`, `760-770`).
- `ValidatedBinding::http_route` targets the same HTTPService `Module` method
  and is owned by MetadataCatalog (`model.rs:687-709`, `876-903`).
- `SubscriptionSource/uses` is not itself runtime. Only the exact
  ExchangePlan -> EventSubscription -> CommonModule chain promotes the
  ExchangePlan through MetadataCatalog
  (`evidence_graph.rs:632-680`).
- resolved `Call` records are the only CallGraph runtime source and both
  endpoints are Methods (`evidence_graph.rs:235-250`; `model.rs:1982-1991`).
- the only callback rows are the Cartesian product of two script variants and
  two slots: Document/ObjectModule/BeforeWrite and
  CommonCommand/CommandModule/CommandProcessing in English/Russian
  (`model.rs:1589-1617`, `1689-1728`). A compatible Definition is mandatory
  before the Metadata callback becomes a runtime edge
  (`evidence_graph.rs:299-397`).
- `add_edge` marks only `calls`, `handles`, and `subscribes` as runtime;
  `contains`, `defines`, and `uses` do not populate `connection_ports`
  (`evidence_graph.rs:590-637`).

## Exact positive connection-port matrix (v1)

Here `M` means `MetadataCatalogPort`, `C` means `CallGraphPort`, and `F` means
`FormInspectionPort`. `DefinitionPort` is a compatibility/existence
prerequisite, not a `connection_ports` contributor.

| Exact target/ownership shape | Positive runtime ports | Closed source |
| --- | --- | --- |
| any `Method` | `C` | resolved Call may use any Method as either endpoint |
| `CommonModule.<name>.<method>` | `C + M` | EventSubscription and ScheduledJob objects |
| `<owner>.Form.<form>.FormModule.<method>` | `C + F` | exact FormCommand/action binding |
| `HTTPService.<service>.Module.<method>` | `C + M` | exact HttpRoute binding |
| `Document.<name>.ObjectModule.<registered-row-name>` | `C + M` | Document callback row |
| `CommonCommand.<name>.CommandModule.<registered-row-name>` | `C + M` | CommonCommand callback row |
| every other Method shape | `C` only | no other live binding/callback constructor can add a positive edge |
| `FormCommand` | `F` | FormCommand subject endpoint |
| `EventSubscription` | `M` | EventSubscription subject endpoint |
| `ScheduledJob` | `M` | ScheduledJob subject endpoint; disabled is still a potential negative mechanism |
| `HttpRoute` | `M` | HttpRoute subject endpoint |
| `ExchangePlan` root | `M` | only exact promoted subscription chain |
| `Document` MetadataObject root | `M` | callback subject endpoint |
| `CommonCommand` root | `M` | callback subject endpoint |
| Module, Form, MetadataAttribute, TabularSection, TabularSectionAttribute, non-Document MetadataObject, Report root, DataProcessor root | none | only structural edges or no v1 runtime endpoint |

The callback method-name row is compared with `ArtifactRef`'s Unicode-lowercase
identity. Across all known script variants the registered-name union already
contains both English and Russian spellings. For one selected source variant,
one spelling is canonical and the opposite spelling is an unsupported alias;
that semantic distinction cannot be derived from `ArtifactOwnershipChain`
alone.

## Required closed helper

Do not leave a private name/suffix switch in `proposal_validator.rs`. Put an
application-owned helper beside `ArtifactOwnershipChain`/the callback registry,
with a shape equivalent to:

```text
RuntimeMechanismProfileV1 {
    connection_ports: BTreeSet<EvidencePort>,
    negative_policy: Exact | Unsupported(reason) | CallbackDependent,
}
```

Recommended construction:

1. Add `PlatformCallbackSlot::ALL` and a single
   `PlatformCallbackShape::registered_rows()` iterator over
   `KnownScriptVariant::ALL x PlatformCallbackSlot::ALL`.
2. Add a registry-owned match method which compares a target chain's owner,
   module, and terminal method identity against those rows. Do not spell
   `BeforeWrite`, `ПередЗаписью`, `CommandProcessing`, or `ОбработкаКоманды`
   again in the validator.
3. Centralize `ArtifactOwnershipChain` owner-kind/module-kind projection and
   reuse it from `ProviderFact::validate`, callback compatibility, and this
   profile; those projections are currently duplicated.
4. Reuse the same endpoint predicates as the smart constructors for
   CommonModule, FormModule, and HTTPService Module. A constructor and the
   potential profile must not be able to disagree.
5. Replace every `runtime_ports_for()` call with the profile. Positive
   materiality uses actual `connection_ports`; negative evaluation uses
   `negative_policy` plus exact coverage requirements. Never let an empty
   `connection_ports` set authorize `No` by vacuous `.all(...)`.

`CallbackDependent` needs the exact Definition subjects selected by the
callback row/rejection. This matters for a proposal targeting the callback
owner (`Document.<name>` or `CommonCommand.<name>`): Metadata can observe the
callback while Definition is unavailable. The current method-only insertion of
Definition does not cover that root target.

## Exact RED matrix

### A. Closed profile table

One table-driven model test must assert the profile for all of these exact
targets (including case-equivalent spellings):

| Case | Target | Expected positive ports |
| --- | --- | --- |
| common module | `CommonModule.Sync.Run` | `C + M` |
| form module | `Report.Sales.Form.Main.FormModule.Print` | `C + F` |
| HTTP route handler | `HTTPService.Api.Module.Get` | `C + M` |
| Document callback EN | `Document.Sale.ObjectModule.BeforeWrite` | `C + M` |
| Document callback RU | `Document.Sale.ObjectModule.ПередЗаписью` | `C + M` |
| CommonCommand callback EN | `CommonCommand.Sync.CommandModule.CommandProcessing` | `C + M` |
| CommonCommand callback RU | `CommonCommand.Sync.CommandModule.ОбработкаКоманды` | `C + M` |
| ordinary ObjectModule | `Catalog.Items.ObjectModule.Helper` | `C` |
| unsupported ExchangePlan callback shape | `ExchangePlan.Sync.ObjectModule.BeforeWrite` | `C` |
| noncanonical CommonCommand method | `CommonCommand.Sync.CommandModule.Helper` | `C` |
| unrelated CommandModule | `Document.Sale.CommandModule.CommandProcessing` | `C` |
| FormCommand | `Report.Sales.Form.Main.Command.Print` | `F` |
| EventSubscription | `EventSubscription.OnWrite` | `M` |
| ScheduledJob | `ScheduledJob.Nightly` | `M` |
| HttpRoute | `HTTPService.Api.URLTemplate.Items.Method.Get` | `M` |
| ExchangePlan root | `ExchangePlan.Sync` | `M` |
| Document root | `Document.Sale` | `M` |
| CommonCommand root | `CommonCommand.Sync` | `M` |
| Report/DataProcessor roots and structural-only artifacts | representative valid target of each kind | empty |

Also generate all four callback rows from `registered_rows()` and assert that
each exact object and owner includes Metadata. This makes a future row addition
fail RED unless the profile follows the registry automatically.

### B. Negative behavioral matrix

1. **Every Method keeps CallGraph material while negative.** A target-scoped
   CallGraph gap changes `No` to `Unknown` for common, form, HTTP, callback, and
   ordinary method shapes.
2. **CommonModule and HTTP handlers keep Metadata material.** With exact owner
   existence still complete, a target-scoped Metadata gap changes runtime to
   `Unknown`; an exact FormInspection gap does not.
3. **FormModule keeps FormInspection material.** A target-scoped FormInspection
   gap changes runtime to `Unknown`; a target-scoped Metadata runtime gap does
   not (registered owner/Form existence remains independently material).
4. **Canonical callback method keeps Metadata + Definition semantics.** Missing
   or bounded Metadata/Definition is `Unknown`; exact complete mismatch is the
   typed rejection, not generic `No`.
5. **Official cross-language alias remains Unknown.** Reversing provider and
   definition record order must retain
   `unsupported_callback_alias_variant` and identical evidence IDs.
6. **Callback owner root needs Definition gating.** Metadata callback present +
   unavailable/bounded Definition for its exact method makes both
   `Document.<name>` and `CommonCommand.<name>` target verdicts `Unknown`, never
   `No`.
7. **Unsupported lifecycle/command variants remain Unknown.** With complete
   CallGraph but a scoped Metadata gap `unsupported_mechanism_variant`, each of
   the following stays Unknown and carries that exact reason:
   `Catalog.Items.ObjectModule.BeforeWrite`,
   `ExchangePlan.Sync.ObjectModule.BeforeWrite`,
   `CommonCommand.Sync.CommandModule.Helper`, and a Report/DataProcessor
   non-form-command print path. None may become Contradicted merely because
   Metadata was removed from the positive-port set.
8. **Exact declarative subjects use only their port.** FormCommand requires F;
   EventSubscription/ScheduledJob/HttpRoute/ExchangePlan require M. An
   unrelated provider gap/state does not change runtime, while the exact port's
   scoped gap does.
9. **No vacuous negative.** Every shape with an empty positive-port set must
   have an explicit negative policy. Unsupported shapes return Unknown; only a
   deliberately closed `NoRuntimeMechanism` profile may return No without a
   provider.
10. **Determinism.** Reverse providers, records, and gaps for every behavioral
    row; report bytes and evidence IDs stay identical. Case-equivalent callback
    targets select the same profile.

## Contradictions in the reviewer's proposed matrix

1. **Positive capability is not negative authority.** “No Metadata runtime
   mechanism” is correct for positive edges of ordinary ObjectModule,
   ExchangePlan ObjectModule, and noncanonical CommandModule methods. It does
   not imply that complete CallGraph proves runtime `No`.
2. **It conflicts with the accepted Task 5B contract.** That contract says no
   callback outside the four rows is authoritative and explicitly requires
   direct ExchangePlan callbacks, other lifecycle/command callbacks, and BSP
   conventions to remain scoped `unknown` (`task-5b-contract.md:19-22`,
   `199-202`, `577-580`). The proposed narrow set would drop the provider gap
   and return false `Contradicted`.
3. **Alias status is not an ownership-chain property.** English versus Russian
   canonical/alias status depends on the observed ScriptVariant. A chain-only
   switch can include the union of registered names, but must not classify an
   alias as a supported callback.
4. **Callback-owner targets are omitted.** Document and CommonCommand roots are
   positive callback edge endpoints, but a conclusive negative also depends on
   the exact callback Definition. The current `callback_observed` branch only
   checks `object == proposal.target`, so it misses root targets.
5. **“FormCommand/declarative targets keep existing mechanisms” is ambiguous.**
   Only the explicitly enumerated EventSubscription, ScheduledJob, HttpRoute,
   ExchangePlan, CommonCommand, Document callback owner, and FormCommand shapes
   are runtime endpoints. Metadata existence for Form, Module, Report,
   DataProcessor, or another root is not a runtime edge.
6. **The active spec is also contradictory.** It still requires blanket
   Metadata + CallGraph + FormInspection coverage for every connectionless
   target (`extension-point-discovery.md:1112-1116`), while its v1 proof
   boundary requires unsupported exchange/report variants to stay Unknown
   (`1235-1247`). The tracked spec must state the split profile, not either
   blanket rule.

## Acceptance boundary for this P1

This P1 is closed only when:

- the table-driven positive matrix and registry-generated callback rows pass;
- negative evaluation uses an explicit profile/policy, not only a port set;
- the four unsupported-variant REDs remain Unknown with exact reasons;
- callback-owner Definition REDs pass;
- actual positive connection-port materiality remains unchanged;
- the active spec is synchronized with the same distinction.

