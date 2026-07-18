# Task 5A root active review addendum 5

Immutable handoff checklist for source roles and record ordering. This note is
review evidence only and is outside the tracked diff.

## P0: source-role identity is not a source-name heuristic

- `CfePairHalf` includes an explicit closed Analysis/Destination role tag in
  addition to the source-scoped artifact. Omitting the role changes canonical
  identity bytes and permits adversarial role aliasing.
- Per-port ordering needs the captured source-group rank and canonical
  destination order. Pass the immutable snapshot/rank map through both the
  per-provider and global limit seams; never infer role from source-set text.
- Add a role-separation test with equal artifact identities across analysis and
  destination projections and prove the halves remain distinct.

## P0: exact inner-record order

- Sorting records by SHA-256 digest is deterministic but is not the frozen
  semantic tuple order.
- Implement the exact typed inner key: fact stable tag, source-qualified
  subject, relation tag, optional object, typed payload digest, location
  presence/path/line/column, and provider/coverage/freshness digest.
- Add fixed order vectors covering absent/present location, option ordering,
  relation/object differences, fact families, and input permutations.

## P0: closed event signature lookup

- Replace the guarded wildcard in `event_handler_signature_class_v1` with a
  literal closed 21-row partial lookup (preferably one row table shared with
  compatibility).
- Prove all 21 supported cells occur exactly once, all five unsupported cells
  return None, and the three signature classes/parameter counts are exact.

## Required report evidence

- Task 5A report must name this note and SHA-256 in addition to the first four
  immutable root review notes.
