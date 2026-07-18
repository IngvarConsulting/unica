# Task 5A root active review addendum 4

Immutable handoff checklist for exact atomic ordering and ScheduledJob negative
authority. This note is review evidence only and is outside the tracked diff.

## P0: ScheduledJob disabled must be independently representable

- A disabled result cannot be derived from `ValidatedBinding::ScheduledJob`,
  because constructing that positive binding already requires handler,
  MethodName and module-profile material that is explicitly nonmaterial when
  `Use=false`.
- Add a dedicated typed whole activation observation/fact equivalent to the
  frozen `ScheduledJobActivationV1::Disabled`, constructed from registered job
  plus exact singleton `Use=false` only.
- Disabled activation emits metadata-only `scheduled_job_disabled`/No and the
  DisabledActivation atomic group. It has no handler object and plans no
  Definition.
- Positive ScheduledJob binding exists only for exact `Use=true` enabled
  descriptor material. Missing/malformed/conflicted Use is Unknown, not false.
- Prove Use=false remains No with missing/wrong MethodName, profile and
  Definition; prove no physical fact can enter both disabled and enabled groups.

## P0: exact per-port and global total orders

- Do not use derived enum/struct `Ord` as the contract encoder.
- The per-port group key must implement the frozen tuple in this order:
  source-group rank, exact canonical source-set bytes, semantic group stable tag,
  primary source-qualified subject key, secondary/dependent-set digest, complete
  source-free cluster digest.
- `StandaloneFact` identity must retain its frozen fact-family, subject,
  optional relation/object and semantic digest fields. A convenient full record
  digest is not a schema-equivalent replacement.
- The global ceiling uses its different frozen tuple: minimum full record digest
  in group, port stable tag, canonical semantic-group bytes, complete cluster
  digest. It must not reuse per-port ordering accidentally.
- Add fixed byte/order vectors and permutation tests that distinguish the two
  orders, multiple source sets and multiple semantic group tags.

## Required report evidence

- Task 5A report must name this note and SHA-256 in addition to the first three
  immutable root review notes.
