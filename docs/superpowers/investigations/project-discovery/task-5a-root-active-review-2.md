# Task 5A root active review addendum

Date: 2026-07-18

This addendum does not mutate the immutable first review checklist. It records
new findings discovered while the implementation was being repaired.

## Additional blocking findings

1. `EventSubscriptionDescriptorV1::new` must reject a repeated exact selector
   before sorting or deduplication. The complete original input set is checked
   by canonical typed-artifact identity; exact, case-equivalent, and accepted
   Unicode-lowercase duplicates are all errors.
2. `ScheduledJob` cannot retain only `enabled` and a runtime context. Its typed
   whole descriptor must retain `Use`, `Predefined`, `Global`, and `Server`.
   Only exact `true, true, false, true` may participate in the exported
   zero-arity Procedure-or-Function Definition join. `ServerCall` is not a
   material field.
3. FormCommand and HTTP Definition compatibility must not be invented in Task
   5A. Until Task 5B accepts primary-backed closed rows, both remain Unknown
   even when a Definition exists. EventSubscription and ScheduledJob keep their
   already accepted exact policies.
4. `SemanticAtomicGroupIdV1` is a closed cross-fact grouping registry. A CFE
   group is one source-local pair half containing its polarity and role-specific
   whole companion; it is not necessarily a cross-source megagroup. An
   EventSubscription group contains its descriptor plus every derived
   ExchangePlan uses projection. Fact tag is an inner-record ordering field,
   never a group discriminator.

## Required REDs

- exact duplicate Event selector;
- wrong ScheduledJob `Predefined`, `Global`, or `Server` with a compatible
  Definition stays non-runtime;
- present Form/HTTP Definition stays Unknown while its policy registry is not
  accepted;
- a provider and global limit that falls between distinct fact tags in each
  CFE/Event atomic cluster retains all or none in forward and reverse order.
