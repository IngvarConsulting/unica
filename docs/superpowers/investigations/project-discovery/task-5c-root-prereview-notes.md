# Task 5C root pre-review notes

Status: blocking notes for a future fresh independent review. This file is
ignored and makes no implementation claim.

## P0: live mutation identity is not specified

The design says `unica.support.edit` consumes one parsed read and refuses a
stale write, but it does not define the filesystem protocol that makes that
claim true. The current implementation reads and later calls `fs::write(path)`.
An attacker or concurrent tool can replace the leaf or an ancestor between the
read and write, so a strict parser alone does not bind the mutation to the
parsed object.

Before implementation the contract must specify and test:

- contained, no-follow root/ancestor/leaf resolution on Unix and Windows;
- an artifact-scoped mutation lease before the authoritative read;
- captured physical file identity and byte digest under that lease;
- a same-directory staged file with explicit permissions and durable flush;
- compare-before-commit against the captured identity/digest;
- atomic replacement using destination-root-relative primitives, followed by
  verification of the committed bytes and parser semantics;
- cleanup/outcome algebra for pre-commit failure, uncertain commit and residue;
- no `Path::canonicalize`, `exists`, `is_file`, path reopen, or `fs::write`
  authority in the mutation window.

If this protocol is intentionally owned by Task 8/9/10, Task 5C may implement
only the pure parser/read/render/guard assessment and must leave support-edit
mutation migration blocked until that primitive is accepted. It may not claim
the stale-write test is implementable through a second path-based stat.

## P1: the proven-subset rule contradicts accepted byte variants

The design says only provenance-backed layouts are accepted, but proposes all
of the following without a real Designer-exported fixture establishing their
semantics:

- BOM-less input;
- global flag `1` as a parsed semantic state (the only current example is a
  synthetic text replacement);
- repeated identical object-rule records as safe semantic deduplication.

The fresh review must either add primary/provenance evidence or narrow these
rows. A conservative alternative is:

- retain the BOM framing of the one real fixture as the only receipt-grade
  parser input; unsupported framing is `Unknown`;
- treat flag `1` as a closed fail-safe read-only observation only if current
  code/tests are explicitly adopted as a compatibility contract, not labelled
  fixture-proven;
- reject every duplicate rule as
  `unsupported_parent_configurations_variant` until vendor/duplicate
  composition is proven.

## P1: extension ownership must not come from support bytes

`ExtensionWithoutParentConfigurations` describes only the absence of a support
policy file. It does not prove that the queried artifact exists in, or is owned
by, that extension. Therefore it cannot independently project to
`extension_owned` or authorize a direct mutation. The projection must join the
accepted Task 5A/5B exact configuration-flavor and CFE-membership authority:

- analysis extension flavor is proven from exact MDClasses XML;
- target membership/absence is exact for that same source set;
- `extension_owned` requires exact owned membership;
- an absent target remains absent/unknown for direct mutation, even when the
  extension has no ParentConfigurations file;
- `extension_required` for a base target requires a distinct, proven extension
  destination and never follows from support state alone.

Add RED cases for identical canonical refs in base and extension source sets,
missing destination objects, forged source-set kind, and missing flavor facts.

## P1: parser/identity reuse must not become adapter chaining

SupportStatePort needs exact UUIDs but infrastructure adapters must not call one
another. The final design must name one neutral snapshot-bound MDClasses parser
and catalog result shared by MetadataCatalogPort and SupportStatePort, or pass
an application-owned typed authority input. Re-parsing through a shared pure
function is acceptable; reading display output or calling the metadata adapter
is not. Namespace, configuration flavor, ownership and source fingerprint must
remain identical in both paths.

## Required review outcome

Do not implement Task 5C until a fresh reviewer resolves every item above and
the accepted Task 5A/5B SHA gates are recorded. The review must produce exact
RED tests, ownership by task, and an immutable document hash.
