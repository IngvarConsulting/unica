# Adversarial review v2: Task 8 source-bound CFE resolver

Reviewed artifact: `.superpowers/sdd/task-8-design.md`, SHA-256
`9d62dea77765c517e95485f63d3e95bf6d3f84f54f129da70cd33a7bd8f11228`.

Scope: the previous Task 8 review, Task 5A/5B/6/7 designs, live source
snapshot/native/application boundaries, active spec/ADR, and the local
authoritative 1C extension/form specifications. No tracked file was edited.

## Findings

### [P1] The allegedly cross-process artifact lease can split into different lock universes

The design puts the persistent artifact inode below `<cache_root>`
(`task-8-design.md:1563-1581`) and then claims that it serializes every applied
CFE call, including receipt-free modes (`:1640-1656`, `:2725-2727`). In live
code, however, `WorkspaceContext.cache_root` is explicitly selected by the
per-process `UNICA_CACHE_DIR` environment variable
(`crates/unica-coder/src/domain/workspace.rs:15-33`). Two fully cooperating
Unica processes can therefore use the same canonical workspace and exact
artifact but different legitimate cache roots. They acquire different inodes,
both pass the final precondition, and the Present path can lose one update.
Hashing the canonical workspace into `<workspaceKey>` does not help when the
directory above that key is already different.

Required correction: define one workspace-invariant lock root that cannot vary
per process, or make the canonical control-plane root an explicit shared
workspace contract and reject a mismatch before mutation. Receipt-free
`off/observe/warn` must not depend on discovering the same receipt store by
accident. Add two-process Present and Absent races with different
`UNICA_CACHE_DIR` values; exactly one lock universe may exist.

### [P1] The Windows primitive contradicts the promised descriptor-relative containment

The writer promises to reopen and walk the destination descriptor-relatively
with no-follow/no-reparse semantics (`task-8-design.md:2196-2208`) and to create
the temp in the already-open exact parent (`:2222-2235`). But the selected
Windows install primitive is path-based `MoveFileExW` (`:2238-2244`). The live
primitive proposed for extraction also converts both source and target paths to
UTF-16 and calls `MoveFileExW` without a parent handle
(`crates/unica-coder/src/infrastructure/runtime_jobs.rs:1507-1534`). A junction
or directory rename/swap after the checked walk can therefore redirect the
target pathname. The artifact lease does not serialize a non-Unica filesystem
actor, and `MoveFileExW` has no descriptor-relative overload. The same gap
applies to Present replacement and parent/temp creation unless each operation
is anchored to a retained parent handle.

Required correction: specify and implement an actual Windows handle-relative
contract (for example, opened-parent-handle creation plus a proven
`SetFileInformationByHandle`/rename-info mode with no-replace and replace
variants), including reparse-point and identity checks; otherwise fail Windows
closed. Add a native race that swaps every parent component between the final
walk and install. A path-based `MoveFileExW` test is not proof of the stated
containment guarantee.

### [P1] Committed/uncertain effects and staging-file residue have no typed return path

The proposed native API returns only `AdapterOutcome`
(`task-8-design.md:2145-2164`), matching the current `HandlerOutcome`, which has
only `adapter` and `job` (`crates/unica-coder/src/application/ports.rs:16-24`).
Later the design separately invents `CfeAppliedEffects` and says Task 10 will
consume it (`task-8-design.md:2266-2289`), but no API connects those values.
This is fatal on the exact uncertain paths the design says must revoke a
receipt: post-commit digest/sync failure, rollback failure, or an Absent Unix
install where `linkat(temp, target)` commits and unlinking `temp` then fails
(`:2238-2240`, `:2257-2264`). In the latter case both the authorized target and
an unplanned staging name exist, while `CfeAppliedEffects` can represent neither
the staging residue nor a committed failure. The ordinary manifest may ignore
that random temp name, so a post-snapshot diff is not a substitute.

Required correction: make the handler boundary return a typed mutation outcome
on both success and error, with commit state, exact expected effects, bounded
unexpected/uncertain effects, and cleanup status. Represent or directly prove
absence of every staging artifact before releasing the artifact lease. Task 10
must revoke/deny baseline advance from this typed value even when
`AdapterOutcome.ok == false`. Add failure injection after every mkdir/write/
fsync/link/unlink/rename/parent-sync/read-back step and assert no effect is
silently lost.

### [P1] Form safety proves only source-handler role, not destination generated-name ownership

The correction adds a complete analysis Form binding catalog, but the prepared
seed contains only `analysis_form_binding_artifact`
(`task-8-design.md:830-847`), and `CfeResolutionMaterial` contains only
`analysis_form_bindings` (`:1664-1682`). Destination duplicate preflight then
consults only BSL annotation/definition facts (`:1917-1948`). That misses a
valid destination `Form.xml` event or command Action already naming the newly
generated handler while the BSL definition is absent. Applying the patch would
create that definition and silently make one procedure serve both the XML
event/action binding and the new module interceptor. Local format evidence is
explicit that extension Form.xml binds these handler names and that such
handlers are ordinary unannotated procedures
(`plugins/unica/references/specs/1c-extension-spec.md:502-564`, `:600-624`).

Required correction: for Form targets capture and completely parse both
analysis and destination Form.xml with the same Task 5 parser. Source negative
proof must classify `MethodName` as Ordinary; destination negative proof must
prove the case-canonical `generated_method_name` is absent from every complete
event/action binding. Bind that destination semantic proof into grant scope and
its exact material identity into execution/baseline. Add orphan destination
binding, duplicate binding, incomplete destination Form.xml, and batch cases.

### [P1] “Configuration-only” authority trusts the topology label but does not prove base XML flavor

The shared `PlatformConfigurationCatalogV1` carries only ScriptVariant,
NamePrefix, and registrations (`task-8-design.md:264-273`). CFE eligibility is
then gated by declared `SourceSetKind::Configuration + PlatformXml`
(`:401-409`, `:1153-1163`), while the adopted join merely consumes the analysis
descriptor `@uuid` and never requires the analysis descriptor membership to be
`Own` (`:1751-1764`). A source configured with kind `Configuration` but pointing
at an extension export can therefore pass the kind gate; its wrapper/local
object UUID can become the alleged base identity if the destination is crafted
to match it. This is exactly the wrapper-UUID substitution the design claims to
forbid. The local format spec says configuration and extension roots have the
same outer XML shape, while direct root `ObjectBelonging=Adopted` and
extension-only properties distinguish the extension
(`plugins/unica/references/specs/1c-extension-spec.md:33-44`, `:48-87`); adopted
object descriptors likewise carry ObjectBelonging and ExtendedConfigurationObject
(`:206-245`).

Required correction: extend the shared configuration catalog with a closed,
direct-field `ConfigurationFlavor` proof and require declared source kind to
match captured XML flavor. For a CFE analysis plan require base Configuration
flavor plus `Own` root/form analysis descriptors before their UUIDs can enter
`DescriptorIdentityDigest`; destination requires Extension flavor plus the
existing Adopted chain. This does not require a second `BaseMetadataIdentity`
fact. Add a misdeclared-extension-as-Configuration fixture whose wrapper/local
UUID intentionally matches the destination; it must fail before issuance and
direct rendering.

### [P2] The final “normalized” plan still retains raw assertion presence

`CfeMethodPatchRequestCore` retains optional Context/IsFunction assertions
(`task-8-design.md:805-828`), the seed retains that request (`:830-856`), and
the final immutable plan retains it again (`:1062-1087`). Yet the canonical
digest deliberately erases whether those assertions were omitted or explicitly
matching (`:1987-2002`). Thus two semantically identical calls can have equal
argument/grant/execution digests but unequal plan values, which is an avoidable
trap for future equality, audit, or expected-binding code.

Required correction: keep assertion provenance only in the prepared seed or a
non-authoritative diagnostic object. After validation, construct a closed
`ResolvedCfeMethodPatchCore` containing only derived context/kind/async and use
that in the final plan/grant tuple. Test omitted and explicit-matching calls for
full semantic-plan equality, not digest equality alone.

## Decision

**Verdict: needs-fix / not implementation-ready.**

The previous seven findings are substantially corrected, including the honest
Present CAS boundary, parent-suffix creation, exact UUID lattice, complete
source Form scan, Around observation, retained BSL spans, and zero-material-read
prepare phase. The five P1 findings above still leave real lost-update,
cross-platform containment, effect-accounting, Form binding, and base-source
identity holes. Resolve them in Task 8 and back-propagate the affected
Task 5B/9/10/spec/ADR/skill contracts before production implementation.
