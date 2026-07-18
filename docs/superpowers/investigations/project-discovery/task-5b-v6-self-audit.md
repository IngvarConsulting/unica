# Task 5B v6 / Task 6 v2 / Task 7 v6 self-audit

Audit date: 2026-07-18.

Verdict: **PASS for conditional design completeness; implementation remains
blocked**. No known P0, P1 or P2 design defect remains in the three versioned
normative files audited below. This self-audit is not an independent acceptance
review and does not clear the recorded Task 5A/5B/5C implementation SHA gates.

## Frozen normative inputs

The new files were frozen for this audit at:

| File | SHA-256 | Lines |
| --- | --- | ---: |
| `task-5b-v6-contract.md` | `665de15a59749bf935dd03b8e15558347db1f93d10dc3cbc2a248b61015c8712` | 2432 |
| `task-6-v2-design.md` | `5f2d859f77878b43e627930b46a99063972f0fe1a00b3bc692213beea76db4cc` | 1408 |
| `task-7-v6-design.md` | `b307703e2f825d3218e8acc73d480372114e215593734e78bdf822e0588ddd9e` | 2094 |

The immutable historical inputs remained byte-identical:

| File | SHA-256 |
| --- | --- |
| `task-5b-contract.md` | `13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab` |
| `task-7-design.md` | `6792d70c58a57a35871a91f5dd9059371ee13599a96e0c00e97e27a974f6ca2a` |
| `task-6-design.md` | `0462f2b97a4cb04aa9503af00df8d64c74a197257471e4d4fe0459bbf1995743` |

Review evidence used:

| File | SHA-256 |
| --- | --- |
| `task-5b-v5-independent-review.md` | `c39c3893c80552e23a7769bb3601a78f2182e54590234376b6898814809bee9d` |
| `task-6-root-prereview-notes.md` | `42ed0882872fe10be480779e093fcb342086c24f1f82dc98a72ec7c55ba84b5b` |
| `task-5a-root-active-review-3.md` | `a431c7ddd4c64b02faa3ed07232d1c5befc76208860f5dfd70e0e7ac032c41ec` |
| `task-5a-root-active-review-6.md` | `65db34f614edcd99d5a13fe6728602502b31feeadab9a8a891fb7e25ee159fd2` |
| `task-5c-root-prereview-notes.md` | `2531ace5f7ea59e088dece447b042b77637d9ad86682ac0c320b9a7d9f0839b2` |

## Closure of the six independent-review P1 findings

| Finding | v6 closure |
| --- | --- |
| Provider ceiling could split a group before Task 7 saw it | Platform XML and all Task 6 providers classify/deduplicate complete `SemanticAtomicGroupIdV2` groups before their local `max_records`; Support has no lossy local ceiling; Task 7 rejects a partial-group provider contract and applies only later whole-group prefix-stop admission. Provider versions and query encoders are bumped to v2. |
| MetadataComposite omitted Form material from query identity | Metadata and Form scopes both include exact `form_material_scope_digest` and `max_records`; Metadata also binds the once-built catalog-set digest, while Form separately binds source fingerprint and its borrowed catalog digest. Exact payload grammar, RED names and normative query goldens are frozen. |
| ScheduledJob material precedence contradicted itself | The state machine is exact `Use -> Predefined`. Disabled is metadata-only No. Exact Predefined=false is dedicated `ScheduledJobNonPredefinedV1`/Unknown with no Binding or handler material. Missing Predefined or incomplete predefined descriptor emits its exact gap only, with no partial fact, candidate or zero-record group. Only the complete supported predefined descriptor opens Definition. |
| Global admission had no closed reason/sentinel owner | `DiscoveryEvidenceAdmission` owns exact per-port/global/gap-limit code/reason tuples. Raw provider testimony is immutable. Provider and admission gaps are projected together, then the complete vector is kept at 256/2,000 or wholly replaced at 257/2,001 by one QueryWide application sentinel. |
| Persistent Form main attribute admitted descendant decoys | The exact direct-child managed-namespace grammar fixes Attributes/Attribute/MainAttribute/Type/data-core Type cardinality, QName namespace resolution, duplicate/mixed/nested failures and BaseForm fallback precedence. |
| Atomic encoder/order was under-specified | Fixed-width/framed primitives, all seven group variants, state/role tags, secondary payloads, source-free and physical projections, material subjects, prefix-stop and domains are closed. Normative source, artifact, group, physical and global-order goldens are published. |

## Closure of post-review encoder and integration regressions

1. `AtomicSourceIdentityV2` is role plus the complete live
   `canonical_source_identity_bytes_v1` projection: name, kind, source format,
   relative root and mapping digest. Fingerprint is excluded from logical
   identity and all group ordering, but remains in query freshness, physical
   record/evidence digest and analysis identity.
2. Task 7 global order is exactly `u16 port tag || bytes(group key)`. It no
   longer begins with a physical-record digest, so fingerprint/provider/location
   cannot reorder semantic groups.
3. Every artifact identity byte uses `ArtifactIdentityBytesV1`: kind stable tag
   plus Rust Unicode-lowercase canonical ref. Exact spelling is only a display
   tie-break. Greek case-equivalence and the expanding `İ -> i + combining dot`
   byte vectors are frozen.
4. The former fourth partial ScheduledJob activation state is absent. The only
   group states are DisabledActivation=1, NonPredefinedActivation=2 and
   EnabledDescriptor=3. Dedicated nonpredefined ProviderFact tag 13 does not
   renumber the existing 1..=12 tags.
5. One neutral `PlatformConfigurationCatalogPort` builds exactly one
   `PlatformConfigurationCatalogSetV1` per composite snapshot. Metadata,
   Support and Task 6 borrow the same typed authority from
   `EvidenceExecutionContext`; adapter chaining, display parsing and a second
   MDClasses parser are prohibited.
6. Catalog/set digests enter Metadata/Form/BSL query and Task 7 execution
   identity. Stable Support lookup uses
   `PlatformConfigurationObjectKeyV1 = catalog digest +
   ArtifactIdentityBytesV1`, outside the catalog payload to avoid a hash cycle.

## Task 6 pre-review closure

| Blocking note | v2 closure |
| --- | --- |
| Conditional symbols affect calls outside the branch | Canonical maybe-definition, maybe-module-shadow and maybe-local-shadow sets propagate to the containing scope, including nested alternatives. Intersections remain Ambiguous/Dynamic/Unknown and cannot create an edge or negative absence. |
| Binders were incomplete | The closed pass handles parameters, Var, bare and module assignments, For, For Each, and access/index lvalues. Any accepted but unclassified binder yields `unsupported_bsl_shadow_analysis` and keeps qualified calls Dynamic. |
| Token comparison contradicted case-insensitive grammar | Identifier, keyword, boolean, undefined and null all use one Unicode-lowercase comparison rule. Required TRUE/FALSE/UNDEFINED/NULL case metamorphics are named. |
| Declarative runtime and BSL contexts were conflated | Exact Task 5B joins are imported: Event `SameAsSourceEvent` to synchronous ModuleDefault Procedure with descriptor arity; Scheduled/HTTP declarative Server does not mean BSL AtServer; Form requires AtClient. |
| Async/unsupported-token handling was open | Explicit async Procedure/Function modes require the matching semantic terminator and fail closed across malformed/nested/preprocessor ambiguity. Every unsupported nontrivia token creates an exact capability gap. |
| Shared-catalog and acceptance gates were absent | Task 6 requires accepted immutable Task 5A, Task 5B v6 and Task 5C v2 SHAs and borrows the once-built catalog set. Cache remains syntax-only, optional and non-authoritative. |

## Independent mechanical golden verification

An independent Python 3.12 script implemented only the published byte grammar
with `struct.pack` and `hashlib`, asserted every expected length/value, then ran
the fingerprint/source/artifact metamorphic checks. It returned:

```text
GOLDEN_OK source len=142 value=e1d804d1e18f2d02679dce05b4e2a822c9a776cfd749a67754c9328fc48d9396
GOLDEN_OK atomic len=148 value=8543b710e36b6393bd362435b76774cf62e59a24bc5b61ee3926a473a2234710
GOLDEN_OK artifact_sigma len=17 value=00010000000b646f63756d656e742ecf83
GOLDEN_OK artifact_expand len=18 value=00010000000c646f63756d656e742e69cc87
GOLDEN_OK metadata_payload len=359 value=56700dfff7680dcd522f11ebe5ced807a06a8d14e97883e10f810ea98d94d4f9
GOLDEN_OK metadata_digest len=32 value=a979e44cb1a1f6a3a6b923b91dd61b38e5e73975aaeff001e03d9de7259371c6
GOLDEN_OK form_payload len=248 value=b451fd55fbf92ac2d3dfce93e497ad7af9a33e7ad4616ab20e4a216197aa0e51
GOLDEN_OK form_digest len=32 value=d9819ec00b4efbc7c2a03dc0681047230b642118d8f608a578b5efac64c2acc5
GOLDEN_OK bsl_payload len=220 value=3d363a007dacb05ffaeabf40ce645e793979b8b5f5391e86224e9fd79582b709
GOLDEN_OK bsl_digest len=32 value=78cc2f7fa751f7e5c52c669e668c2031abcbc6919ce137fe6fc4f1d41329a0cc
GOLDEN_OK secondary len=32 value=d676f3489f6c9b6794c72c0cbd47f8a139e8fe96574dd53e99a440f16eae405c
GOLDEN_OK cluster len=32 value=2398dac1eea977cb341f08a3fc4f5293a7209e8009520490eb7dc94a4877e788
GOLDEN_OK group len=388 value=a430566280d0cb70bb731d3d349ee852cb13cafec461ddd1772941fc123a126e
GOLDEN_OK physical len=273 value=74f3339fcb2f2165a2196b8b0190c994c56286c0a8ffb3e557d7ae1c42780e77
GOLDEN_OK global len=394 value=57b305568677e427f0ff95fa39e319e2c8284955fd5ccfe543ea6428beeeb406
METAMORPHIC_OK fingerprint_preserves_source_artifact_group_global_and_changes_physical
METAMORPHIC_OK root_or_mapping_changes_logical_source_identity
ALL_GOLDENS_OK
```

A separate exact-value presence pass found all 15 published values in the three
normative documents and returned `ALL_DOC_GOLDENS_PRESENT`.

## Static audit

- Markdown fence counts were even: Task 5B 112, Task 6 36, Task 7 52.
- No trailing whitespace or common unfinished-work marker was found.
  The explicit unavailable `TASK5A_ACCEPTED_SHA` value is an intentional hard
  implementation gate, not unfinished design text.
- No stale v1 atomic/query/global encoder, partial Scheduled state, string-only
  SourceSetWide, fingerprint-bearing logical identity or physical-digest-first
  global order pattern was found.
- The first shell aggregation used a zsh scalar where an array was required and
  was invalid; the corrected array-based sweep returned
  `STALE_PATTERNS_OK`. No result from the invalid invocation was accepted.
- All four new audit artifacts are ignored by the existing
  `.superpowers/sdd/.gitignore`; no immutable historical design/review file was
  edited.

## Remaining gates and stop decision

The design is conditionally complete, not implementation-ready. Stop before the
first implementation RED until the required accepted Task 5A/Task 5B v6/Task
5C v2 SHAs exist, active spec/product contracts are synchronized, and a fresh
independent reviewer accepts these exact frozen hashes. A different hash, dirty
branch, current HEAD or this self-audit cannot substitute for those gates.
