# Adversarial review: Task 8 source-bound CFE resolver design

Reviewed artifact: `.superpowers/sdd/task-8-design.md`, SHA-256
`ec4bddba6923d1dd80305d0470e4f4e8989c3d9bf75ae5eff2a13fc1cb4a9c3a`.

Scope: live code, accepted Task 5A destination-membership contract, Task
5B/5C/6/7 designs, active architecture/spec, local extension format evidence,
and official 1C module/form-extension mechanics. No tracked files were edited.

## Findings

### [P1] The final precondition and atomic rename are not one conditional write

Task 8 checks the current module and parents before the write
(`task-8-design.md:1594-1605`) and then performs a same-directory replace/create
(`:1610-1625`). That leaves a TOCTOU window. The primitive selected for
extraction is currently an unconditional `fs::rename` on Unix and
`MoveFileExW(..., MOVEFILE_REPLACE_EXISTING, ...)` on Windows
(`runtime_jobs.rs:1476-1534`). If another call changes a Present module, or
creates an Absent module, after step 6 and before rename, Task 8 overwrites the
new bytes. Reading the file after commit only proves that Unica's bytes won; it
cannot detect the lost concurrent edit.

The future receipt lease does not close this. It is keyed by receipt, while two
different receipts may target the same module, and active spec explicitly gives
`observe`/`warn` calls no receipt lease
(`extension-point-discovery.md:1445-1452`). The design's own rolling scenario
allows multiple grants for one destination module, so artifact-level
serialization is part of the resolver/write contract, not an optional receipt
detail.

Required correction: define one persistent process + OS mutation lease keyed
by canonical workspace/destination-source/allowed-artifact (or a deliberately
coarser workspace mutation lease), acquire it before the final recapture, and
hold it through install, post-snapshot, and receipt transition. Specify lock
order with the receipt lease to prevent deadlocks. Absent installation must use
an atomic **no-replace** operation. Present replacement needs an explicit
platform contract for conditional replacement or an honestly bounded
cooperative-lock guarantee; ordinary rename is not compare-and-swap. Add
cross-process tests for same receipt, different receipts, and no-receipt
observe/warn calls, including an Absent-create race.

### [P1] `no parent creation` makes the first real patch after borrow impossible

The destination artifact is beneath
`<OwnerDir>/<Name>/Ext/<Module>.bsl` (or the analogous Common/Form path), and an
Absent target is explicitly supported (`task-8-design.md:847-864`). But the
write contract says that adopted registration proves every parent and forbids
parent creation (`:1612-1622`, `:1824-1829`, `:1932-1935`). That premise is
false for the live borrower: `cfe_borrow_object_shell` creates only
`<OwnerDir>/<Name>.xml` and the containing `<OwnerDir>`
(`cfe.rs:506-522`); it does not create `<OwnerDir>/<Name>/Ext`. A normal
already-adopted Designer export can likewise omit an empty module directory.
The temporary file therefore cannot even be created for the most important
"first interceptor" case.

`SnapshotWatch` watches only the final artifact plus root/form descriptor
prerequisites; it neither captures nor authorizes missing directory entries.

Required correction: make parent-chain state a typed, captured precondition.
Either (a) add exact bounded directory creation to the mutation plan/grant and
typed effects, using component-by-component no-follow creation with rollback
of only directories created by this call, or (b) introduce a separately proven
preparation contract that creates those directories before receipt issuance.
Do not replace this with broad `create_dir_all`. Add fixtures for borrowed
Common/Owner/Form targets with the module and one or more platform-owned parent
directories absent.

### [P1] The design collapses an unsafe own/mismatched object into “run borrow”

The accepted Task 5A contract distinguishes three materially different rows:
destination descriptor absent means `RequiresBorrow`, a present same-name Own
object is `Indeterminate/destination_object_not_adopted`, and an Adopted object
with another UUID is
`Indeterminate/destination_extended_object_mismatch`
(`task-5a-destination-membership-design.md:187-221`). Task 8 instead represents
missing `ObjectBelonging` as `ObjectBelongingObservation::Missing`
(`task-8-design.md:192-210`), maps it to
`cfe_destination_borrow_required`, and collapses every non-success row to the
public blocker `destination_borrow_required` (`:1211-1223`, `:1656-1658`).

For a **present** descriptor, missing both fields is exactly the Own encoding,
not descriptor absence. Advising borrow is unsafe: the live non-form borrower
unconditionally rewrites the target descriptor (`cfe.rs:250-264`, `:506-520`).
The same advice is wrong for a cross-UUID adopted object.

Required correction: keep existence polarity separate from membership. Only a
destination `MetadataAbsent` pair may become `ExtensionRequired` /
`destination_borrow_required`. Present Own, UUID mismatch, malformed, and
inconclusive membership stay `Unknown` with their accepted exact blockers and
must not produce “run cfe.borrow” guidance. Align direct reasons, discovery
checks, skill text, support projection, and RED cases with the Task 5A lattice.

### [P1] Complete form-handler negative proof is not specified for form events

Task 8 correctly rejects any Form.xml event/action handler and requires a
complete negative proof before classifying a FormModule method as Ordinary
(`task-8-design.md:118-125`, `:1181-1193`). However, the prerequisite Task 5B
contract currently parses only the direct command path
`/Form/Commands/Command/Action` (`task-5b-contract.md:244-277`). Task 8 adds an
`Event` enum value and the sentence “exposes exact event/action identities”
(`task-8-design.md:181-233`), but never defines the event-bearing XML paths,
recursive element rules, completeness boundary, duplicate rules, or callType
handling needed to prove absence.

This is not a theoretical edge: the local extension format shows both
form-level `Events/Event` and element-level nested `Events/Event` handlers, in
addition to command actions
(`1c-extension-spec.md:502-564`). Official 1C documentation likewise says form
handlers are assigned on the relative form item and are generated without
module annotations.

Required correction: extend the one shared Form parser with an explicit
schema-aware catalog of all supported form-level and nested item event carriers
plus command actions, or fail the whole Form-method classification closed when
an unsupported event-bearing structure is present. Bind exact handler method,
event/action owner identity, Direct/Before/After/Override call type, and parser
completeness. Add top-level, nested element, command, duplicate/conflict,
malformed, lexical-decoy, and unsupported-node tests; none may yield Ordinary
without complete negative proof.

### [P1] Duplicate preflight ignores the platform's Around/Вместо interceptor

The requested write enum is intentionally limited to Before, After, and
ModificationAndControl (`task-8-design.md:445-470`), but the destination scan
must understand more than the set the tool can generate. The duplicate matrix
recognizes only those three (`:1355-1371`, `:1780-1787`). Existing
`&Around`/`&Вместо` is a real platform interceptor and is mutually exclusive
with Before/After for the same method; local format evidence lists it alongside
Before, After, and ChangeAndValidate (`1c-extension-spec.md:701-710`). Official
1C module-extension documentation also lists `&Around` and allows only
Before+After as a paired combination.

As written, an existing Around on the same target is either treated as an
unrelated unknown annotation or is not represented in the closed conflict
matrix, so Task 8 can issue a plan that is invalid or changes interception
semantics.

Required correction: separate `RequestedCfeInterceptorType` from a closed
`ObservedCfeInterceptorKind` that includes RU/EN
Before/After/Around/ChangeAndValidate spellings. Define and test the complete
target conflict matrix even though the public tool still cannot request
Around. Conditional/malformed Around must block negative proof; deleted/comment
and string decoys must not. Batch compatibility must use the same matrix.

### [P1] The immutable source plan drops the exact spans it promises to bind

The Task 6 back-propagation requires definition, declaration, name,
parameter-list, body, and terminator spans plus declaration line ending
(`task-8-design.md:235-274`). The renderer relies on those exact offsets for
signature copy and name splice (`:1284-1335`), and the execution-plan digest is
said to include “definition spans” (`:1458-1472`). Yet
`CfeSourceMethodPlan` retains only `definition_span` and three opaque digests
(`:594-620`). It drops declaration/name/parameter/body/terminator ranges and
the line-ending value.

That makes the final plan incapable of self-validating the claimed splice
authority or unambiguously encoding the promised execution material. It also
leaves fixed digest fixtures under-specified: two implementations can hash
different subsets while both claim conformance.

Required correction: carry every validated byte range and the declaration line
ending in `CfeSourceMethodPlan` (or one closed `VerifiedDefinitionSlicesV1`),
define the exact bytes/semantic fields of definition/signature/body digests,
and list every range in the execution-plan encoder order. Constructors must
check ordering, containment/non-overlap, module digest, and round-trip slices.
Add fixed-vector tests that mutate each range/line-ending independently and
reject forged-but-in-bounds offsets.

### [P1] The prepare/capture boundary still contains a circular instruction

The high-level API is correctly two-stage and says `prepare` derives topology
without reading source material, while adoption is resolved from the captured
snapshot (`task-8-design.md:127-141`, `:1031-1069`). But §6.3 explicitly says
“`prepare` validates borrowed references” (`:866-887`). Borrowed status and
UUID equality are descriptor-material facts; they cannot be validated before
the watched capture without reopening the live tree or depending on stale Task
5 output.

Required correction: state one hard phase ownership table. `prepare` may
validate raw aliases, source selection, ExtensionPath mapping, canonical target,
derived candidate paths, and expected identity only. Registration presence,
descriptor UUIDs, ObjectBelonging, ExtendedConfigurationObject, ScriptVariant,
NamePrefix, Form bindings, source definition, and destination duplicates all
belong exclusively to `resolve(captured)`. Add a recording fake proving prepare
performs zero snapshot/file/material reads and a stale pre-capture descriptor
cannot influence the seed or any digest.

## Decision

The design is not implementation-ready at the reviewed SHA. The seven P1
findings above affect data-loss resistance, the primary first-patch workflow,
adoption safety, form-handler correctness, interceptor conflict detection,
receipt/execution digest authority, and the core prepare/capture boundary.
Resolve them in the design and the back-propagated Task 5A/5B/6/9/10/spec/skill
contracts before Task 8 production code starts.

Official primary references used for the mechanism checks:

- <https://1c-dn.com/blog/module-extensions/>
- <https://kb.1ci.com/1C_Enterprise_Platform/Guides/Developer_Guides/1C_Enterprise_8.3.23_Developer_Guide/Chapter_36._Configuration_extension/36.4._Extension_objects/36.4.3._Forms/>

