# Task 5B v5 / Task 7 independent adversarial review

Review date: 2026-07-18

Immutable inputs rechecked before review:

- `.superpowers/sdd/task-5b-contract.md`:
  `13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab`
- `.superpowers/sdd/task-7-design.md`:
  `6792d70c58a57a35871a91f5dd9059371ee13599a96e0c00e97e27a974f6ca2a`

Verdict: **NEEDS-FIX**. No confirmed P0 was found, but the P1 findings below
block an unambiguous implementation and deterministic receipt-grade execution
identity.

## Findings

### [P1] Atomic grouping is specified after a lossy provider record ceiling

`task-7-design.md:925-933` sends `request.limits.maxEvidence` into every
one-hop CallGraph query, while `task-7-design.md:1093-1125` classifies and
atomically limits only records that have already returned from providers. Task 7
never makes whole-group limiting a prerequisite of the Task 6 query boundary.
It therefore cannot reconstruct a group member that the provider omitted at its
own `max_records` ceiling.

Concrete trace: one caller has the same resolved `caller -> callee` semantic
fact at two locations. Both records classify as one `StandaloneFact` semantic
group. A query ceiling of one may return one record plus a caller-scoped
`Bounded` gap. Task 7 sees a one-record group and cannot know that the second
witness belonged to it, so the public evidence can contain a split group despite
the hard all-or-none promises at `task-7-design.md:55-60`,
`task-7-design.md:1103-1125`, and `task-7-design.md:1647-1649`.

With the required exact `Bounded` caller/method gap this trace should block a
final Supported/No conclusion, so it is not a demonstrated P0 false conclusion.
It is still a P1 contract/implementability failure: the accepted output violates
the closed atomic-retention invariant and can expose a partial edge/evidence
cluster. Back-propagate the classifier and identical prefix-stop order into every
record-producing Task 6/support query before its local ceiling, or remove the
lossy provider ceiling and let the scoped accumulator be the first limiter. Add
a RED with two locations of one semantic Call/Definition fact split exactly at
the provider boundary.

### [P1] The two documents disagree on the exact MetadataComposite query identity

Task 5B says every frozen `FormMaterialScopeV1`, including exact proposal runtime
owner/method subjects, enters the query digest
(`task-5b-contract.md:371-375`), but its purported exact Task 7 scope omits that
digest (`task-5b-contract.md:381-405`). Task 7 correctly adds
`form_material_scope_digest` (`task-7-design.md:597-606`) and again says it is
part of the exact query identity (`task-7-design.md:643-648`).

Two requests can keep the same composite snapshot, sources, pair set and
presence keys while changing the exact runtime method subject inside one Form
material scope. Under the Task 5B shape they have the same query identity, so a
cached/reused invocation can carry the previous method's missing-material scope;
under Task 7 they are different. This also makes the execution snapshot contract
non-uniform. Add `form_material_scope_digest` to the Task 5B exact shape and its
mandatory RED, then publish one canonical encoder definition in both documents.

### [P1] Enabled non-predefined ScheduledJob has contradictory material precedence

Task 5B first says that once `Use=true`, Predefined, MethodName, module profile
and Definition all become material (`task-5b-contract.md:933-935`). It then says
`Use=true, Predefined=false` is the complete specific
`non_predefined_scheduled_job_instance_unproven` Unknown
(`task-5b-contract.md:945-951`), and the metadata-first table selects that row
without MethodName/profile/Definition (`task-5b-contract.md:962-976`). But the
next rule says missing/malformed MethodName for any `Use=true` job is instead a
descriptor-local malformed gap (`task-5b-contract.md:952-954`).

Thus `Use=true + Predefined=false + missing MethodName` has two different exact
outcomes, reason/material scopes and digests. The closed atomic registry also
calls every Use=true cluster `EnabledDescriptor`
(`task-5b-contract.md:572-600`) and assigns enabled-descriptor material an owner
and handler (`task-5b-contract.md:653-668`), although this branch has no positive
descriptor endpoint. Freeze one precedence. The consistent metadata-first
choice is `Use -> Predefined`; when Predefined is false, emit one non-predefined
enabled-activation cluster independent of MethodName/profile/Definition. If the
opposite is intended, change the table and Task 7 Stage 2 contract. In either
case give this branch an exact atomic group identity and closed material-subject
function.

### [P1] Global evidence-limit rewriting has no closed reason/sentinel contract

Task 7 requires the global six-port limiter to rewrite every affected scoped
provider invocation from Complete to Bounded and add a whole-group gap
(`task-7-design.md:1103-1125`). A `ProviderGap`/check and its outcome digest need
an exact reason code, but the design never says whether a global drop uses a
port-local result-limit reason, `evidence_limit`, `global_evidence_limit`, or a
new traversal reason. The closed `DiscoveryTraversal` reasons do not include
such a code (`task-7-design.md:741-752`, `task-7-design.md:1401-1417`), while the
analysis ID binds the complete outcome snapshot
(`task-7-design.md:673-713`). Implementations can therefore produce different
checks and analysis IDs for the same retained prefix.

The overflow interaction is also unspecified. Task 5B requires more than 256
gaps or 2,000 exact affected subjects to collapse to the one
`platform_xml_gap_limit` QueryWide sentinel
(`task-5b-contract.md:670-673`), but Task 7 can append a global-limit gap after
the provider has already reached that boundary and merely says “exact
source-scoped evidence-limit gaps” (`task-7-design.md:1110-1113`). Freeze an
exact per-port/global reason registry, the owner of the check tuple, and the
post-rewrite gap-count/subject overflow algorithm. Add REDs for an existing
provider sentinel plus one global drop and for 2,000/2,001 affected subjects.

### [P1] Persistent Form main-attribute XML grammar is not closed enough to prevent a decoy root

The Form contract lists the accepted QName families and namespace URIs, but it
does not define the exact XML path/cardinalities that select the main attribute
(`task-5b-contract.md:1140-1195`). In particular it does not freeze cardinality
of direct `Attributes`, `Attribute`, `MainAttribute=true`, and `Type`, whether
the data-core `Type` must be a direct child rather than an arbitrary descendant,
or the result of duplicate true flags/Type wrappers. The RED list checks wrong
URI, unprefixed and multiple semantic types, but not these path/cardinality
cases (`task-5b-contract.md:1638-1646`).

This is material, not cosmetic: an implementation that searches descendants can
accept a nested data-core Type decoy as persistent context, validate a
context-sensitive Form event, and seed a runtime mechanism. Specify one exact
`Form[/BaseForm]/Attributes/Attribute[MainAttribute=true]/Type/{data-core}Type`
grammar with directness, singleton/boolean/mixed-content rules and duplicate
handling, then add nested-wrapper and duplicate-main-attribute REDs.

### [P1] The normative atomic key/digest encoder is not actually closed

Task 5B calls the tuple a strict total order, but several normative components
remain placeholders rather than canonical encodings
(`task-5b-contract.md:620-641`):

- `analysis=0; destinations=1 in canonical order` does not say whether every
  destination has rank 1 and is then ordered by identity, or receives ordinal
  ranks 1..N;
- “exact canonical source_set identity bytes” does not identify the typed field
  set (logical source-set name, role/name tuple, snapshot identity, fingerprint,
  or physical-source identity) or its normalization;
- `group secondary key / dependent-pair-set digest` has no variant-by-variant
  value, empty encoding, sorting rule, hash domain, or byte width;
- “complete cluster source-free semantic digest” has no exact projection for
  source-bound facts, so it is unclear whether source_set, snapshot fingerprint,
  provider freshness and location are excluded. The following inner-record tuple
  explicitly includes provider/coverage/freshness, which makes accidental reuse
  especially plausible.

Task 7 then requires this “exact canonical group key” for per-port retention and
uses canonical SemanticAtomicGroupId bytes plus the cluster digest in the global
order (`task-7-design.md:1093-1113`). Different defensible choices can retain a
different group at N/N+1 and therefore change gaps, graph, checks and analysis ID.
This is P1, not cosmetic P2. Publish one domain-separated, length-delimited
variant encoder with fixed integer widths; state all-destinations-rank-1 versus
ordinal explicitly; define the exact source identity projection; and define
source-free/dependent digest input records. Add golden-byte/digest REDs for two
destinations, equal source-free clusters under different fingerprints, and a
changed dependent pair set.

## Cross-document consistency checks that passed

- EventSubscription is internally consistent at 13 family mappings, 21 exact
  compatible event/family rows (13 BeforeWrite + 8 BeforeDelete), and three
  signature classes; Task 7 consumes rather than reconstructs that descriptor.
- FormCommand compatibility agrees across both documents: exact own FormModule,
  Procedure, one by-reference nondefaulted parameter, explicit AtClient,
  synchronous or asynchronous, Export nonmaterial.
- HTTP compatibility agrees across both documents: exact same-service Module,
  synchronous ModuleDefault Function, one by-reference nondefaulted parameter,
  Export nonmaterial; wider parameter/context/async variants remain Unknown.
- MDClasses, Event Source Type and Form main-type namespace/QName URIs agree, and
  arbitrary prefix spelling is consistently nonsemantic.
- Disabled ScheduledJob is consistently metadata-only No and schedules no
  Definition; the defect above is limited to the enabled/non-predefined branch.
- Once complete groups are available to the accumulator, both documents agree
  on whole-group prefix-stop (no skip-and-continue), source-scoped materiality,
  one composite Metadata invocation, and analysis-only FormInspection.
