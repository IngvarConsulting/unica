# Task 7: Platform XML semantic navigation report

## RED

- Before the projector module existed, the required filtered command completed
  with zero matching tests rather than failing compilation. Cargo accepts a
  non-existent filter, so this differs from the brief's expected failure.
- After adding the projection invariants, compilation correctly failed first
  on projector-local implementation errors (property conversion shadowing and
  a fixture path type), then on the navigation node's private descriptor cache.

## GREEN

- `cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture`
  passed: 6 tests.
- `cargo test -p unica-coder source_adapters::registry::tests -- --nocapture`
  passed: 9 tests.

## Decisions

- Semantic object keys use native UUIDs as `uuid:<uuid>` when present. The
  fallback is only `sha256(sourceId NUL ownerKey NUL canonicalKind NUL name)`;
  no display path or physical location participates in identity.
- The projection creates a derived `SourceRoot`, creates the root metadata
  object as a normal semantic node, and connects every native child recursively
  with independently hashed `contains` relations. Duplicate object and
  relation keys fail with `identity_collision`.
- Properties are emitted as typed `SemanticProperty` values. XML type
  descriptions become structured `TypeSetValue` variants, never raw XML or a
  canonical string expression.
- Capabilities carry resolution, key strength, captured snapshot consistency,
  coverage, exact 2.20 compatibility, strict support state, and the read-only
  adapter access state. With no writer binding all advertised actions are
  `modeled`, never executable.
- `BuiltInSourceAdapterRegistry::new()` installs exactly `PlatformXmlProbe` and
  `PlatformXmlReadAdapter`. No public `unica.meta.info` surface was changed.

## Concerns

- Form internals remain unprojected and partial form coverage reduces modeled
  actions to inspection. No Task 8 form-element, binding, move, or handler work
  was started.

## Fix Round 1

### RED

- New tests initially failed to compile because the snapshot support reader,
  provider support-bytes accessor, and derived relation identity field did not
  exist. The first implementation also exposed two parser-local compile errors
  (borrowed XML text lifetime and predicate signature).

### GREEN

- Projector tests: 12 passed.
- Registry tests: 9 passed.
- Provider tests: 7 passed.
- Support tests: 20 passed.

### Decisions

- `ParentConfigurations.bin` is read only from immutable
  `PlatformXmlProvider` snapshot bytes; snapshot absence maps to `Removed`.
  A post-`open` filesystem change cannot change projected support capability.
- The shared exact-2.20 schema now owns scalar property IDs and the bounded
  type-description grammar. It accepts only declared primitive/reference/enum
  variants and direct declared qualifiers; malformed or path-like values fail
  closed without becoming canonical output.
- Forms always advertise partial coverage and inspection only until form
  internals are projected. Scalar coercion is selected by canonical property
  ID, never by the text value. Generated relation keys always have derived
  identity strength.
