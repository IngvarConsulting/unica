- Date: `2026-08-27`
- Status: `approved`
- Decision: `DEC.2026-08-27.ACTOR-REVISION-ARTIFACT-POLICY-SLICE`

# Actor-owned revision artifact policy

## Problem

Retained apply projected every staged file into its candidate manifest, while
ambient, retained and incremental revision capture indexed only a global
extension list. A staged Platform XML resource such as
`XDTOPackages/<name>/Ext/Package.bin` therefore produced a candidate which the
post-publication retained scan could never reproduce. The equality gate and
rollback were correct; the revision corpus had two owners.

This is not a `.bin`-suffix exception. Later v0.13 slices also own support,
template, help and form resources which do not belong in a global extension
allow-list. Widening the legacy list would change v0.12 revision identity and
hash unrelated or potentially multi-gigabyte artifacts.

## Approved design

One typed `RevisionArtifactPolicy` belongs to the authenticated actor source
binding. `WorkspaceActor` derives it from `SourceSetKind`, `SourceFormat` and
`SourceProfile`; callers cannot construct it from wire input. The corresponding
`SourceRevisionService` retains the policy and uses it for ambient capture,
retained capture, incremental watcher reconciliation and retained-apply
projection. The legacy constructor retains the exact v0.12 policy.

The policy has three dispositions:

- `Content`: relative path, entry kind and SHA-256 of bounded bytes contribute;
- `Presence`: relative path, entry kind and retained identity contribute, but
  payload bytes are not read;
- `Ignored`: no file manifest entry and no byte budget, while directory
  enumeration still consumes the existing entry budget.

Directory entries remain revision-bearing exactly as before. Manifest entry
kinds are typed internally with stable encoded values `Directory = 1`,
`Content = 2`, and new `Presence = 3`. The digest domain
`unica-source-sha256-v1`, record schema and serialized revision shape do not
change, so legacy manifests remain byte-identical.

## Closed Platform XML 8.3.27 / format 2.20 profile

The existing extension set remains `Content`. Configuration and extension
source sets additionally classify:

- `Ext/ParentConfigurations.bin` as `Content`;
- direct `Ext/ParentConfigurations/<name>.cf` as `Presence`;
- `XDTOPackages/<name>/Ext/Package.bin` as `Content`.

All Platform XML source kinds classify `Template.bin`, `Template.txt` and files
inside the `Template/` resource subtree beneath `Templates/<name>/Ext` as
`Content`; files beneath an `Ext/Help/` resource subtree and beneath
`Forms/<name>/Ext/Form/Items/` are also `Content`. Dynamic components are opaque
native path components; fixed components are exact. Unrelated binaries remain
ignored.

Vendor `.cf` bytes are not semantic input to support planning. Adding,
removing or renaming a direct member rotates the revision, while an in-place
payload rewrite is guarded by the support transaction for that invocation and
does not impose an unbounded content hash.

## Projection and publication

Only a `Content` path may be a staged file change. `Presence` and `Ignored`
staged paths fail with a typed internal invariant during preparation, before
source, cache, record or state publication. Content projection uses the same
entry kind/digest and parent-directory closure as live capture.

Post-publication validation still requires two stable retained captures, equal
retained identities, equality to the projected manifest and equality of the
candidate digest. Late failures still roll back every participant and return no
receipt. This slice introduces no directory topology mutation; the later
support slice must project explicit topology through this policy.

## Bounds, restart and migration

Content retains chunked hashing, cancellation/deadline checkpoints, no-follow
identity evidence, per-file and aggregate byte limits. Presence proves a
regular no-follow file and retains identity without reading bytes. Ignored
files consume only enumeration capacity.

Actor state scope already includes source kind, format and profile. A rebuilt
service therefore derives the same policy and record path. Its first live
observation compares the new closed corpus with an old scoped record: a present
newly classified resource rotates once, then unchanged bytes remain stable
across subsequent admissions and restart. External drift of classified content
or presence membership rotates the next admitted revision.

## Rejected alternatives

- Global extension widening changes v0.12 identity and hashes unrelated data.
- A process-global path helper lacks actor/profile ownership and applies the
  Platform XML grammar to unsupported source formats.
- A transaction-only supplemental manifest cannot reproduce the returned
  revision on the next invocation, external drift check or restart.

## Compatibility boundary

This slice changes no MCP tool, argument schema, result envelope, route,
revision algorithm string or revision-record schema. It does not implement the
XDTO planner. The public v0.12 service remains selected until the atomic v0.13
cutover; tool ledger and schema checks remain before/after characterization
gates rather than fabricated RED evidence.
