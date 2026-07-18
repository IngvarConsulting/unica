# Task 5B v5 immutable-artifact integrity incident

Date: 2026-07-18

Status: closed; no tracked file or Task 8 design was changed.

## Incident

The coordinator authorized a v6 design-fix agent to edit the already frozen
`.superpowers/sdd/task-5b-contract.md` path in place while an independent Task 8
review was still verifying prerequisite hashes. This was a coordination error:
the v5 content was immutable evidence and v6 required a new versioned path.

The Task 8 reviewer detected the mismatch and stopped using the changed file.
The writer was interrupted. The partial v6 bytes were preserved separately,
the writer's patches were reversed on the v5 path, and every anchor was
reverified.

## Restored immutable anchors

- Task 5B v5 contract:
  `13ca8e3599ce3e4843ae82773a8911194f2786ce741b9040c14563b60dbedbab`
- Task 7 v5 design:
  `6792d70c58a57a35871a91f5dd9059371ee13599a96e0c00e97e27a974f6ca2a`
- Task 5B v5 self-audit:
  `d9d866094e4d5587751dd853688ec85db3c14c6db40564f9f372bedebcc23f30`
- Task 5B v5 independent review:
  `c39c3893c80552e23a7769bb3601a78f2182e54590234376b6898814809bee9d`

## Preserved v6 draft

The partial draft was copied byte-for-byte to the new versioned path
`.superpowers/sdd/task-5b-v6-contract.md` with SHA-256
`e5b709832c253ff826fa11faa4aa7b76fb45efb18322e3000f4ef227b0773a8e`.
That hash is not an acceptance hash and the draft is not a complete v6.

## Process correction

All later design versions use new versioned paths. A frozen artifact is never
edited in place. A superseding version receives its own self-audit, independent
review and published hashes before implementation may consume it.
