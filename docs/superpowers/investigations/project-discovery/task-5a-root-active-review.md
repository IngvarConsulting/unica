# Task 5A root active review

Date: 2026-07-18

Status: **implementation STOP** until every item below has a RED, a production
fix, and an independent review against one immutable diff.

## Blocking findings

1. Event source identity uniqueness cannot use an adjacent-window check after a
   differently ordered sort. Use a complete canonical typed-artifact set and
   cover `A, Z, a` plus Unicode/case permutations.
2. `EventSubscriptionDescriptorV1.selected_sources` is authoritative. Any
   `SubscriptionSource` projection must be derived from it or validated by exact
   ExchangePlan-set equality. Reject orphan/missing/extra/conflicting uses and
   multiple semantic descriptors per source-qualified subscription.
3. CFE authority cannot be assembled from independent flavor, UUID, membership,
   presence, or freshness facts. Use role-specific, source-qualified whole
   observations. Wrong-but-well-formed flavor/membership remains a valid source
   observation so the distinct proposal blockers stay reachable; malformed or
   incomplete material is a gap.
4. Source-qualified whole observations must match `EvidenceRecord.freshness` in
   universal record validation, before any query-plan-specific validation.
5. `FormCallType::Direct` is an internal semantic state for an absent XML
   `callType`. Literal `callType="Direct"` must remain invalid in native and
   discovery paths; explicit XML accepts only Before/After/Override.
6. Both per-provider and global evidence limiters currently retain individual
   records after semantic validation. They must retain/drop atomic semantic
   groups, including CFE pair-half polarity+companion, EventSubscription
   descriptor+derived uses, and all other non-separable witnesses. Dropped gaps
   cover the whole group and retention is permutation invariant.
7. Declarative bindings are not runtime edges by themselves. EventSubscription,
   ScheduledJob, FormCommand, and HTTP binding-subject proposals must stay
   Unknown when their exact handler Definition is missing or incompatible.
   EventSubscription joins an exported Procedure with exact registry arity;
   active predefined ScheduledJob joins an exported zero-arity Procedure or
   Function and exact module/job profile. Other mechanisms fail closed pending
   their accepted compatibility policies.

## Naming invariant

Types named `Base*` or `Extension*` must not store the opposite observed flavor.
When mismatch observations are intentionally representable, name the type by
query role (`Analysis*`, `Destination*`) and observation semantics.

## Acceptance gate

- focused RED/GREEN tests for every row above;
- full locked crate tests;
- product-contract tests;
- fmt, all-target clippy with warnings denied, and `git diff --check`;
- fresh code and spec reviews with no P0/P1;
- report updated with exact immutable diff hash.
