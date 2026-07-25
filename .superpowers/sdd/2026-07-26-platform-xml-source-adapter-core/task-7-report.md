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
## Fix Round 3

### RED

- `xsi:type` compared a literal `xs:*` prefix instead of resolving the QName value against its in-scope namespace URI.
- Unknown, conflicting and unsupported scalar annotations could fail the whole decoder instead of producing a local unresolved property.
- `FillValue` used `f64` only as a lexical gate and accepted Boolean, Integer and UUID annotations despite its bounded 2.20 contract.
- Qualifier validation inferred group presence from child keys, so empty alien groups and duplicate empty groups could evade compatibility checks.
- Initial RED test integration exposed fixture-only defects: an attribute-bearing `Properties` tag made the helper insert a second block, and a duplicated test child produced an artificial UUID collision. Namespace declarations were moved to the scalar element and the existing fixture child retained; product assertions were not weakened.

### GREEN

- Decoder resolves `xsi:type` values as prefixed QNames through `lookup_namespace_uri`; every prefix bound to the XML Schema URI is accepted and normalized to a bounded enum. Alien, unbound, malformed and raw/unqualified annotations expose no raw QName.
- `Missing`, `Unknown`, `Conflicting` and `Unqualified` are explicit `UnresolvedScalar` states. They retain no scalar lexical value or raw annotation, preserve sibling decode/projection, and project as `valueState=unresolved`, no canonical value, unknown provenance and read-only property capability.
- `FillValue` accepts only validated XML Schema `decimal` and `string`. Decimal uses a lossless XML Schema lexical parser and canonical normalized string representation; exponent, NaN, INF and invalid lexical values fail closed locally. Boolean, Integer and UUID annotations are unresolved.
- Qualifier parser records `String`, `Number` and `Date` group identity before children, rejects duplicate groups including empty groups, validates each child, and enforces exact primitive/group compatibility.

### Tests

- `cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture` -- 26 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture` -- 18 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::schema -- --nocapture` -- 4 passed.
- `cargo test -p unica-coder source_adapters::registry::tests -- --nocapture` -- 9 passed.
- `cargo test -p unica-coder domain::navigation::tests -- --nocapture` -- 17 passed.

### Concerns

- The scalar and qualifier grammar remains intentionally bounded to certified 2.20 forms. Future XML Schema local names or qualifier groups require an explicit shared-schema extension; they must not be inferred from text or promoted to canonical values.
## Fix Round 4

### RED

- Legacy `meta.info` rendering had the safe unresolved marker branch but no focused regression proof against exposing rejected scalar lexical content.
- Declared compatible qualifier groups with no children passed because parser tracked group identity but did not require a validated child.
- Decoder classified empty scalar text as `Absent` before examining `xsi:type`, losing valid empty strings and hiding invalid/unsupported annotations.

### GREEN

- Legacy native rendering covers `UnresolvedScalar` with the fixed `<unresolved>` marker and no canonical/raw scalar data. No Task 8 public rendering switch was made.
- Every declared `StringQualifiers`, `NumberQualifiers`, or `DateQualifiers` group must contribute at least one validated child; duplicate group detection remains prior to child parsing and errors expose no input XML.
- Decoder resolves scalar annotation before empty-body handling: valid XML Schema string remains explicit empty String; valid decimal empty body is `InvalidLexical`; alien, unbound, conflicting and unsupported annotations remain local unresolved states; unannotated empty scalar remains Absent.
- Empty annotation failures preserve sibling projection; projector serializes unresolved decimal without a canonical value.

### Tests

- `cargo test -p unica-coder source_adapters::platform_xml::decoder::tests -- --nocapture` -- 27 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::projector::tests -- --nocapture` -- 19 passed.
- `cargo test -p unica-coder source_adapters::platform_xml::schema -- --nocapture` -- 5 passed.
- `cargo test -p unica-coder native_operations::meta::native_legacy_rendering_tests -- --nocapture` -- 1 passed.

### Concerns

- The bounded grammar intentionally rejects declared qualifier groups that carry no validated information. Future platform grammar variants must be added as explicit certified groups rather than treated as empty/defaulted state.
