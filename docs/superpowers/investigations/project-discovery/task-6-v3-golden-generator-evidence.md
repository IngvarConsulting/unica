# Task 6 query-v3 standalone golden-generator evidence

Status: **frozen-owner-tuple reproducibility evidence; not design-package
acceptance**, 2026-07-18.

This evidence is external to every owner contract. It records a deterministic
design-stage run bound to the exact four owner hashes in section 2. Binding
identifies audited bytes; it does not accept them or edit an owner. Only
`.superpowers/sdd/task-4-7-v7-design-package-acceptance.md` may make the atomic
four-document acceptance transition after final audits and independent reviews
bind the same final tuple.

## 1. Executable identity and command

```text
generator = .superpowers/sdd/task-6-v3-golden-generator.py
generator SHA-256 = 5fc0280a1b6eea92e644bfd6907abbcff75af3df0a06a964fe88850af0218821
working directory = repository root
command = python3.12 .superpowers/sdd/task-6-v3-golden-generator.py
exit status = 0
stdout bytes = 6282
complete stdout SHA-256 = 7e1ac5cf305367fd83bf1c25b1c866553a90bbbd7692444a256a969c20536897
stderr bytes = 0
```

The generator is stdlib-only and imports no Unica production crate, test helper
or future Task 5B/Task 6 smart constructor. Path A is an imperative field
appender. Path B is a closed declarative schema walker with independent
validation and framing code. Every positive, mutation and negative payload is
required to match across both paths before publication.

The executable contains no Python `assert` statement; every invariant uses an
explicit failure branch. Optimized mode is itself a negative gate:

```text
command = python3.12 -O .superpowers/sdd/task-6-v3-golden-generator.py
exit status = 1
stdout bytes = 0
stderr = Task 6 query-v3 generator rejects Python optimized mode; run without -O
```

Therefore `python -O` cannot erase the checks and still publish a `PASS` line.
The normal stdout hash stayed byte-identical after removing `assert` dependence.

The script's explicit source/digest constants are design-authority fixtures,
not a production constructor surface. Production starts only from
the composite-bound `&PlatformCatalogContextV1`, obtains its restricted
context-bound Analysis header view, consumes the owned `source_identity()`
directly, and borrows only the fingerprint, both digests and numeric version
from that same view. BSL material remains outside the header view and is
available only through the unified plan/item/dispatcher. These API constraints
preserve the exact frozen bytes below; they do not legitimize detached
production input.

## 2. Exact frozen owner binding

Fresh SHA-256 recomputation before the recorded normal/optimized/independent
runs produced exactly:

```text
Task4 = 1581d0b737a9e4e856526d67987a292edd39404ec5dda1cb3299c6041409cde2
Task5B = 30430abeb69aeb83bd665a08b41fa1837675a651b3be736936c6e4e96e14f3ad
Task6 = 9f488f78ba20f188e1c28e5393eb9d5d16889cde8f8ca5363bb2ea476631fca0
Task7 = 708022ff0b179092d5f23609449dfa8a7415adaa2e404179b9a24b43d95c1b7d
```

The same frozen Task6/Task7 text contains all six positive query digests and the
redundant-frame negative reproduced below; a mechanical seven-value cross-check
against this generator passed. Any owner or generator byte change makes this
binding and all derived audit/review/ledger evidence stale. This file records
reproducibility only and performs no acceptance transition.

## 3. Exact output summary

The complete stdout includes lowercase payload hex for the two imported source
identities, all six positive v3 rows and the redundant-frame negative. The
reproduced identities were:

| Value | Length | SHA-256(bytes) |
| --- | ---: | --- |
| `ResolvedSourceSetIdentityBytesV1` | 142 | `e1d804d1e18f2d02679dce05b4e2a822c9a776cfd749a67754c9328fc48d9396` |
| `AtomicSourceIdentityV2` | 148 | `8543b710e36b6393bd362435b76774cf62e59a24bc5b61ee3926a473a2234710` |

The exact positive query-v3 results were:

| Query | Length | SHA-256(payload) | `H("unica.snapshot-bsl-provider-query/v3", payload)` |
| --- | ---: | --- | --- |
| CodeSearch empty | 254 | `96671f23b236f560865fe808ad2e12e9c99c19d7f0e0980a9939e8cc93f81112` | `f0c11bd41c207547a9eb7bc8f5230edc04e6ae0bef8340039b66979c4db90683` |
| CodeSearch one `Needle` | 264 | `5be4fa92d5f5347458695e7127fe40eed5e5f286a8f2fcefa86642922fe12964` | `b14163b7ec4244043e4c98919c1ab2f3393fa39d8696c67a3bc96838ca2fda1a` |
| Definition empty | 254 | `785e54a281b9fd6359a14d54320f552ed5b9a8869ecb0b905418520203effe0a` | `2ca861ede84017e0bf6e8e110bb079bf784329a15ef325c24fcd9c962af6a9ae` |
| Definition one `CommonModule.Flow.Run` | 281 | `55b7c16f7fb1c9b2b44ca5dc331de40db66498c5a9c213bbb3874e34d44381e4` | `61d0dc8d91a05346311fcf5b8a087b19f6b7eed3e0bf5f0118765e24c12a2049` |
| CallGraph empty | 254 | `99b8abf1eb160ef255e78ed8033ed7199364defb6b877530104e36bf14e572f3` | `ea5f488e008df9ac016bf78b6c60d06193164d3d6f3e2fc45a02185772510060` |
| CallGraph one `CommonModule.Flow.Run` | 281 | `7eadca7ec37c752cbe05fe9fdf6bc924c05f17227433239932d0ee327dc2908e` | `97f2faf6e7d9901b2b70d3d972492dfd19cfcc522c419bd68107034846008372` |

The forbidden redundant `bytes(AtomicSourceIdentityV2)` frame was generated
only as a negative and rejected by the direct-frame invariant:

```text
length = 258
SHA-256(payload) = 42c3d46a2b338e08628e9e6404829d43db176a8f25897a2c92b5edb91af1b1b9
H(v3, payload) = 1ea160f0b9bfacbb134047a6d1be1d23dc45961ca0a8311505420e6d25520c18
```

The historical v2 negative was also reproduced and proved unequal to every v3
row:

```text
length = 220
SHA-256(payload) = 3d363a007dacb05ffaeabf40ce645e793979b8b5f5391e86224e9fd79582b709
H(v2, payload) = 78cc2f7fa751f7e5c52c669e668c2031abcbc6919ce137fe6fc4f1d41329a0cc
```

The exact terminal assertion summary was:

```text
assertions|two_paths=PASS|owned_atomic_direct=PASS|registries=PASS|canonical_order=PASS|duplicate_rejection=PASS|single_field_mutations=8/8|redundant_frame_rejection=PASS|historical_v2_inequality=PASS
```

`assertions|` is a frozen human-readable stdout label, not evidence of Python
`assert` statements; the source scan for executable `assert` statements is
empty.

The eight single-field mutations were port, source logical identity, source
fingerprint, configuration-catalog digest, registered-Form contract version,
registered-Form catalog digest, vector member and `max_records`. Each changed
both payload bytes and the domain hash while the two paths remained equal.
Reverse two-member term and Method input order canonicalized to byte equality;
duplicate direct final members were rejected rather than silently dropped.
Those eight detached fields are design-generator sensitivity fixtures only.
Production starts from one non-forgeable `PlatformCatalogContextV1`, uses valid
coupled recaptures for catalog changes and compile-fail tests for private field
forgery; it does not reproduce a detached single-field catalog constructor.

## 4. Blind cross-check

A separate stdlib-only calculation, performed without importing or calling this
generator and encoding each Definition-one field directly from the published
grammar, reproduced:

```text
Definition one length = 281
SHA-256(payload) = 55b7c16f7fb1c9b2b44ca5dc331de40db66498c5a9c213bbb3874e34d44381e4
H(v3, payload) = 61d0dc8d91a05346311fcf5b8a087b19f6b7eed3e0bf5f0118765e24c12a2049
complete cross-check stdout SHA-256 = bc7564517b489390d0d8082eab08a9f956bc4223ffc739eb2caa6301dfbb1ff0
```

This records an independent Definition-one check only; it does not turn either
calculation into owner or package acceptance authority.

## 5. Remaining external acceptance step

The coordinator reran both normal and optimized-mode-negative commands plus the
independent Definition-one cross-check after the section-2 tuple stopped
changing. The external acceptance ledger must still record this evidence hash,
the owner self-audits and separate independent reviews that name the same tuple
before changing package status. Owner contracts remain hash/status-free; this
evidence is not a substitute for that atomic ledger transition.
