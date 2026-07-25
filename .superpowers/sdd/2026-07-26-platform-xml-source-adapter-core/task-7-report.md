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
## Fix Round 2

### RED

- `read_support_facts_bytes` принимал snapshot bytes без ограничения размера до whitespace/parse; oversized whitespace мог пройти в неправильное состояние поддержки.
- Native scalar model не сохранял ограниченную type annotation, поэтому projector не мог различить polymorphic `FillValue` без эвристики по тексту.
- `FillValue` ошибочно был зафиксирован как Boolean; qualifier groups не проверялись на совместимость с primitive kind.
- Hashed contains relation наследовал identity capability target object, хотя его собственная identity derived.
- Предыдущий TOCTOU test не открывал provider до изменения реального `ParentConfigurations.bin`, поэтому не доказывал snapshot isolation.
- Во время RED-прохода обнаружены две реализации причины: InputTooLarge сначала попадал в generic `UnknownSupportState`, а первый XML fixture для annotation содержал два `Properties` блока. Исправлены contract mapping и fixture, а не assertions.

### GREEN

- `read_support_facts_bytes` применяет предел 1 MiB к исходным snapshot bytes до trim/parse, возвращает typed `InputTooLarge`, а authorability для этого состояния -- `UnknownReadOnly`.
- Decoder сохраняет только validated `xsi:type` как `NativeScalarType`; raw XML attributes не становятся semantic data.
- Versioned 2.20 property schema моделирует `FillValue` как polymorphic: `xs:decimal` сохраняется Decimal, `xs:string` -- String; отсутствующие, unknown, conflicting или invalid annotations остаются unresolved без text inference.
- Type parser отклоняет alien qualifier groups для primitive variants; malformed, path-like и incompatible descriptions fail closed.
- Derived contains relation теперь задает и `identityStrength`, и `capability.identity` как `Derived`, независимо от UUID target.
- TOCTOU regression открывает реальный provider с locked `ParentConfigurations.bin`, затем изменяет тот же файл в editable и projects via `inspect_provider`; результат остается `SupportLocked` из revision snapshot.

### Tests

- `cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture` -- 24 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture` -- 16 passed.
- `cargo test -p unica-coder source_adapters::registry::tests -- --nocapture` -- 9 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::provider::tests -- --nocapture` -- 7 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::support::tests -- --nocapture` -- 21 passed.

### Concerns

- The 2.20 scalar/type grammar is deliberately bounded. New platform variants or property IDs must be added to the shared schema explicitly; unknown annotations and incompatible qualifiers stay unresolved or rejected rather than becoming canonical data.
