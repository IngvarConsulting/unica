# Adversarial review v3: Task 8 source-bound CFE resolver

Reviewed artifact: `.superpowers/sdd/task-8-design.md`, all 3331 lines,
SHA-256 `f4bcac74f30612deb36830e8f195c22d4b32c797f8addc77e0c8ec14be61e7c9`.

Comparison baseline: `.superpowers/sdd/task-8-v2-review.md`, SHA-256
`e7ca40be6848eb149033a1d515ee68002a966cd2128fe29a1a7b27b6c4c69078`.
The review also checked the current tracked source-snapshot identity, project
source mapping, application/native boundaries, active spec/ADR/skill, package
contracts, and tracked Configuration/CFE fixtures. No tracked file was edited.

## Findings

### [P1] The same physical artifact can still acquire two different artifact locks

The new control root correctly removes `WorkspaceContext.cache_root` and
`UNICA_CACHE_DIR` from the equation (`task-8-design.md:1756-1789`), but the key
inside that root is not a stable physical-artifact identity. It hashes an
otherwise undefined `destinationIdentityDigest` plus `artifact`
(`:1717-1721`, `:1773-1783`). The plan calls the corresponding field
`destination_source_identity_digest` (`:1250-1265`). The only concrete current
source identity contains `name`, kind, format, relative root, and
`mapping_digest` (`crates/unica-coder/src/domain/source_snapshot.rs:356-365`),
and that mapping digest hashes every configured source set, including unrelated
ones (`crates/unica-coder/src/infrastructure/project_sources.rs:494-513`).

Consequently process A can hold a lock derived from mapping M, an unrelated
source-map edit can produce M+1 without changing the destination target, and
process B can acquire a different inode for the same file. If the edit occurs
after A's final mapping check (`task-8-design.md:2496-2508`), both cooperating
writers can reach the same target. The additional `<workspaceKey>` directory
also hashes canonical path bytes while already living below the workspace
(`:1761-1763`, `:1773-1783`): two aliases/bind mounts of one physical workspace
can therefore select different subdirectories of the same `.build` tree. The
required two-process test only varies `UNICA_CACHE_DIR` (`:2680-2681`,
`:3037-3039`) and cannot detect either split.

Required correction: define the artifact lease key from one stable physical
target locus (retained workspace identity plus canonical workspace-relative
target, or verified directory/file identities), excluding source-set name and
map-wide mapping digest. Since the control root is already workspace-relative,
remove the path-derived workspace subnamespace or derive it from the retained
root object's stable identity. The process-local registry must use the same
identity. Add same-process and two-process tests for an unrelated mapping edit,
two configured aliases of one destination, and two path aliases/bind mounts of
one workspace; exactly one contender may pass.

### [P1] Configuration flavor rejects tracked valid base and extension layouts

The classifier says a base configuration requires
`ConfigurationExtensionCompatibilityMode` to be absent and an extension
requires both it and `KeepMappingToExtendedConfigurationObjectsByIDs`
(`task-8-design.md:405-413`). That is contradicted by tracked authoritative
fixtures in both directions: valid base configurations contain
`ConfigurationExtensionCompatibilityMode`
(`tests/fixtures/unica_mcp_script_parity/cf-info/Configuration.xml:13-15` and
`tests/fixtures/unica_mcp_script_parity/bsp/cf/Configuration.xml:42-45`), while
the accepted extension fixture has exact `ObjectBelonging=Adopted` and
`ConfigurationExtensionPurpose=Customization` but neither optional property
(`tests/fixtures/unica_mcp_script_parity/cfe-diff/mode-b/src-cfe/Configuration.xml:3-16`).
The repository's Configuration reference also lists compatibility mode among
ordinary Configuration properties
(`plugins/unica/references/specs/1c-configuration-spec.md:92-100`). Thus the
fifth v2 P1 was not actually closed: normal base analysis fails as "mixed", and
a normal extension fails as "partial", before the Own/adopted UUID proof can run.

Required correction: classify flavor by exact direct singleton
`ObjectBelonging`/`ConfigurationExtensionPurpose`: absent/absent is base;
`Adopted` plus one supported purpose is extension; partial/other/duplicate is
inconclusive. Treat compatibility mode and KeepMapping as optional on either
flavor and validate them only when present. Back-propagate one exact table and
cardinalities into Task 5B/8/spec, then add the two tracked fixtures plus
missing/duplicate/invalid N/N+1 cases. The repaired Task 5B contract already
states this boundary (`task-5b-contract.md:519-550`, `:1308-1320`); Task 8 must
import it rather than retain its contradictory table.

### [P1] `Committed` cannot represent the staging residue the writer requires it to return

The declared handler type permits `Committed` only with
`MutationCleanupProof`, whose invariant is that every owned staging path is
proven absent (`task-8-design.md:2383-2418`). The atomic-write contract then
requires `linkat` success plus temp-unlink failure to return `Committed` with an
unexpected `OwnedStagingFile` (`:2543-2549`), and repeats that any known committed
target with staging residue remains `Committed` (`:2615-2623`). No value of the
declared `Committed` variant can truthfully satisfy both contracts. An
implementation must either forge a clean proof or violate the required commit
classification, which recreates the v2 effect-accounting hole.

Required correction: give `Committed` a cleanup state capable of `Verified`,
`Residue`, and, if commit is known but cleanup observation fails, `Unknown` (or
split it into explicit clean/residual committed variants). Constructors must
make `NoChange` require verified-clean and empty vectors, while Task 10 may
advance only verified-clean exact `Committed`; residue/unknown always revokes.
Add type-level and failure-injection tests for committed-target/temp-unlink
failure so no clean proof can be constructed for that path.

### [P1] Windows still opens the destination root by path, contradicting its handle-relative guarantee

The Windows contract opens both the workspace and destination roots using
path-based `CreateFileW` and only walks children with a retained `RootDirectory`
(`task-8-design.md:2552-2560`). `CreateFileW` has no parent-handle parameter, so
opening the destination root this way leaves the complete intermediate path
outside the retained workspace-handle chain. This directly contradicts the
same section's prohibition on an absolute-path reopen (`:2572-2575`) and the
acceptance claim that *every* parent operation is relative to retained handles
(`:3162-3166`). A junction or rename swap in a destination-root component can
therefore redirect the initial root open before the later child walk becomes
safe.

Required correction: open the workspace root once, then open every destination-
root component from that retained handle with the reviewed root-relative native
primitive, no-follow/reparse rejection, and `FileIdInfo` verification. Never
reopen the destination by an absolute path. Add native races that swap each
destination-root component during root acquisition as well as after the final
walk; unsupported root-relative semantics must fail before the first mutation.

### [P1] A renamed-away retained parent has no truthful typed effect representation

The design explicitly admits that a non-cooperating actor may rename the
already-open directory object and says post-path/identity verification can then
return `Uncertain` (`task-8-design.md:2576-2580`). But all typed effects carry
only a workspace-relative `artifact` string; `CreatedFile` has no object identity
at all (`:2397-2408`), and the authority section requires those paths to stay
inside the stable allowed scope (`:2605-2624`, `:3170-3177`). If install commits
through a retained handle after its parent has been moved, the created/updated
file belongs to the moved directory object, not necessarily the declared
workspace-relative path. Reporting the intended path is false; omitting the
effect loses a known mutation; classifying only cleanup as unknown does not
identify the committed object.

Required correction: either prove and enforce that a retained mutation parent
cannot be renamed for the supported platform/backend, or add an explicit typed
detached/relocated-object effect carrying the intended artifact plus stable
volume/file identity and keep it through revocation/cleanup reporting. State the
Unix and Windows guarantee separately if they differ. Add a test that moves the
original retained parent itself (not merely replaces its pathname), lets install
reach its classification seam, and asserts an honest non-advancing result with
the actual object identity preserved.

### [P2] Task 8 omits exact lexical bounds for Form item/event/command identities and opaque IDs

The Form section gives exact handler bounds but describes item names/IDs and
Event/Command names only as missing/duplicate/invalid bounded material
(`task-8-design.md:432-466`). It never defines their byte/scalar bounds, whether
IDs are opaque or numeric-normalized, or the duplicate equality rule, while the
acceptance criteria require N/N+1 proof for every limit (`:3131`). Two compliant-
looking implementations can therefore disagree about `01` versus `1`, Unicode
case identity, whitespace, or the 256/257 boundary and reach opposite negative-
proof results.

Required correction: import the shared parser's exact constants and equality
rules into Task 8 before freezing its digest/semantic-proof fixtures. The
repaired Task 5B contract defines identifiers as 1..512 UTF-8 bytes/1..128
scalars, opaque IDs as 1..256 bytes/1..128 scalars with exact decoded-byte
equality/no numeric or case normalization, and duplicate rules
(`task-5b-contract.md:621-649`, `:1384-1395`). Add those N/N+1 and `01`/`1`
cross-task REDs to §15.0 rather than leaving them implicit.

### [P2] The final Task 8 gate conflates Task 8 seams with later receipt implementation

Task 8.7 requires tests in which a receipt advances revision/baseline and lock
order includes a current receipt lease, then says those discovery/receipt model
tests must turn GREEN (`task-8-design.md:2919-2929`). Final verification runs the
`discovery_receipts` suite (`:3003-3019`). Yet the same design explicitly forbids
Task 8 from adding receipt persistence, receipt lease, or guard policy
(`:3185-3187`), while the historical delivery order assigns the store/lease and
guard pipeline to Tasks 9 and 10. Without a precise fake/algebra boundary, the
implementer must either expand Task 8 into later production scope or leave its
own final gate red.

Required correction: make Task 8 acceptance name only pure grant/effect algebra
and recording fake seams that can turn green without a receipt store or guard.
Move persistent revision advancement, receipt-lease ordering, and the production
`discovery_receipts` integration gate to Tasks 9/10; alternatively reorder and
complete those tasks before claiming Task 8 final verification. Record the new
ownership explicitly in the plan so a green fake cannot be mistaken for a green
production guard.

## Decision

**Verdict: needs-fix / not implementation-ready.**

The v3 design substantially closes the previous destination Form dual-proof and
raw-assertion-erasure findings (`task-8-design.md:1234-1246`, `:3077-3081`,
`:3111-3118`). It also correctly rejects `MoveFileExW` and adds a generic typed
handler boundary. However, the lock identity can still split for the same
artifact, the flavor table rejects live valid inputs, the committed-residue type
is uninhabitable, and the Windows/detached-parent contracts still overclaim
containment/effect precision.

Independently, the mandatory back-propagation gate is currently red: the active
spec and ADR still acquire only the receipt lease before the handler
(`spec/architecture/extension-point-discovery.md:903-927`, `:1445-1467`;
`spec/decisions/0008-project-discovery-and-discovery-receipts.md:71-95`), the
skill still advertises synthetic Context/IsFunction defaults
(`plugins/unica/skills/cfe-patch-method/SKILL.md:20-35`), and the historical
Task 9 plan still prescribes `MoveFileExW` plus `${cache_root}`
(`docs/superpowers/plans/2026-07-17-project-discovery-receipts.md:717-725`). This
is a declared hard stop under §15.0/§18, not documentation cleanup that can be
postponed. Correct the seven findings, back-propagate one coherent contract, and
make every named RED gate green before production Task 8 implementation.
