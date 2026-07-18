# Task 5C: strict ParentConfigurations and fail-closed support consumers

Base: accepted Task 5B commit (record exact SHA before implementation).

Read `.superpowers/sdd/task-5c-support-design.md` completely. It is the
authoritative evidence boundary. This slice implements one proven strict parser,
snapshot-bound support provider, application projection integration, legacy
renderers, support guard, and `unica.support.edit`. It must not invent a full
ParentConfigurations grammar from historical donor code or PR #83.

## Non-negotiable format boundary

- Add one neutral pure byte parser in
  `infrastructure/parent_configurations.rs`. Discovery and every legacy
  consumer use it. Delete duplicate/lossy/pattern-scan semantic parsers only
  after all consumers migrate.
- Typed read result is Missing, Parsed(UnderSupport document), Malformed(problem),
  or IoFailure(problem,retryable). Missing is proven absence, not metadata/read
  failure. Object rules are typed Locked/Editable/Removed, never public numeric
  flags.
- Accept only the fixture-proven v1 subset: strict UTF-8, optional retained BOM,
  exact marker 6, global flag 0|1, structurally tokenized and fully consumed
  supported one-vendor layout, canonical UUIDs, validated edit spans. Reject
  quoted lexical decoys, unbalanced/trailing/truncated data, invalid encoding,
  NUL, unknown flags, and unproven variants.
- Identical duplicate rules dedupe deterministically only when the structural
  count remains valid. Distinct rules for one UUID are
  `conflicting_parent_configuration_rules`; never select numeric minimum.
- There is no constructible present-file ExplicitNotUnderSupport in v1. Empty,
  `len<=32`, zero-vendor, multi-vendor, or other unproven present layout is
  non-retryable `unsupported_parent_configurations_variant`, not removed/not-
  under-support. Clearing this stop requires a real Designer-exported fixture.

## Snapshot provider and raw facts

Read exactly manifest `Ext/ParentConfigurations.bin` through
`read_optional_verified`.

- verified missing base -> BaseWithoutParentConfigurations;
- verified missing extension -> ExtensionWithoutParentConfigurations;
- parsed global disabled -> ConfigurationReadOnly;
- parsed global enabled + exact resolved UUID -> Editable/Locked/Removed;
- parsed complete map + exact child UUID absent -> ObjectNotListed;
- unresolved UUID -> failed `support_subject_unresolved`, no fact;
- malformed/unsupported -> Failed, non-retryable, no fact;
- fingerprint/identity read change -> Unavailable
  `source_fingerprint_mismatch`, retryable, zero records;
- optional path missing from manifest -> fatal provider contract violation.

Raw facts are exactly the seven Task 5A variants. Never emit raw Unknown,
ExtensionRequired, ExtensionOwned, or guessed explicit-not-under-support.
Facts retain exact source-set freshness and deterministic order.

## Projection contract

Candidate/direct mapping:

- Editable -> editable;
- Locked -> locked/block direct mutation;
- ConfigurationReadOnly -> configuration_read_only/block;
- Removed -> removed;
- ObjectNotListed and base missing -> not_under_support;
- extension missing -> extension_owned;
- failed/unavailable/no fact -> unknown/ineligible.

For current CFE proposal, require known analysis and exact Extension destination
support. Safe destination states Extension missing/ObjectNotListed/Editable/
Removed project extension_owned only when the target already belongs to that
same extension, otherwise extension_required. Destination Locked/ConfigRO
wins. Unknown/ambiguous/malformed/unavailable/source mismatch stays unknown.
Different source-set facts for one ArtifactRef are not conflicts.

## Legacy consumers and guard

Migrate:

- configuration support lines used by cf.info;
- shared object support status used by form/meta/skd/mxl/role/subsystem info;
- all application support-guard descriptors and detailed preview path;
- native `unica.support.edit`.

Renderers may remain successful but must say malformed/unsupported or
unavailable; they must never render those cases as not under support/free to
edit/removed.

Guard assessment is Safe, Violation, or Indeterminate. Resolve configured mode
before mapping:

- off: intentional bypass, no false claim of safe evidence;
- warn: handler allowed with exact malformed/unavailable/violation warning;
- deny, missing mode, or invalid mode: violation or indeterminate blocks before
  handler; retryability only changes diagnostics.

Existing state requirements remain: ConfigRO blocks all; Editable requirement
blocks Locked; Removed requirement blocks Locked and Editable, allows Removed.
Do not mechanically change form/template removal policy: real nested rule
composition is an explicit stop condition.

`unica.support.edit` consumes the same single parsed document and validated
spans, preserves BOM/no-BOM framing, validates output by reparsing, and refuses
stale/conflicting/malformed/unsupported/I/O input without writes. It never
rereads and reinterprets through a second parser.

## RED -> GREEN

1. Copy the three identical tracked files into one canonical provenance-backed
   Task 5 support fixture; prove they are one semantic example.
2. RED/GREEN pure parser: tracked positive, synthetic current positive helper,
   strict encoding, empty/short garbage, marker/global flag, truncation,
   lexical decoy, trailing data, duplicate/conflict/count mismatch, and
   unsupported zero/multi-vendor.
3. RED/GREEN snapshot support provider and exact raw facts/retry split.
4. RED/GREEN application projection, including one raw fact projected
   differently for direct candidate and CFE intent without conflict.
5. RED/GREEN renderers, starting cf.info then the shared object renderer and all
   consumers.
6. RED/GREEN off|warn|deny guard matrix over violation, malformed, unsupported,
   I/O, unresolved UUID, missing base, ObjectNotListed, Removed requirements;
   assert handler calls and zero writes precisely.
7. RED/GREEN support.edit single-parse/span/framing/stale behavior.
8. Add regression documenting current nested resolver UUID choices without
   broadening destructive policy.
9. Synchronize active spec/plan/product contract; do not document guessed
   zero-vendor grammar.

## Verification and stop conditions

Run focused parser/provider/projection/renderer/guard/edit tests, full
unica-coder, fmt, clippy `-D warnings`, product contracts, and diff check. Write
`.superpowers/sdd/task-5c-report.md` with actual RED/GREEN evidence, commit SHA,
and every remaining fixture/policy stop.

Commit as:

`fix: сделать support evidence fail-closed`

Stop instead of guessing if success requires present explicit-not-under-support,
multiple parent/vendor composition, per-vendor conflict aggregation, nested
form/template removal authorization, or an encoding/layout without a real
provenance-backed fixture.
