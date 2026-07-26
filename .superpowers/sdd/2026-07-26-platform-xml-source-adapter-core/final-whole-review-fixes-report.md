# Final Whole-Branch Review Fixes

Date: 2026-07-26

## Verification

- P2 XML boundary confirmed: decoder had the only bounded parser; probe and `Configuration.xml` identity extraction built a DOM directly and materialized wrapper children in a `Vec`.
- P2 support boundary confirmed: `ParentConfigurations.bin` materialized recursive `AstValue` lists before any nesting limit.

## Changed contracts

- `platform_xml::xml` owns BOM-aware UTF-8 validation, streaming XML depth preflight, and DOM construction. It returns only classified errors; decoder, probe, and provider retain their established caller-specific typed errors without physical paths.
- Probe and provider inspect at most two direct element children when enforcing exact-one metadata cardinality.
- ParentConfigurations AST lists share the native depth 64 bound. Excessive nesting becomes `SupportSourceState::Unreadable` with `NestingLimitExceeded` before recursive allocation.

## Production parse audit

- `decoder`: root descriptor and all captured companions use `platform_xml::xml`.
- `probe`: descriptor parsing uses `platform_xml::xml`.
- `provider`: `Configuration.xml` UUID parsing uses `platform_xml::xml`.
- Remaining `Document::parse` sites are test-only helpers in decoder, projector, and schema.
- Other recursive entries: probe structural traversal is bounded by XML preflight; `parse_vendor_blocks` is bounded by `MAX_VENDOR_COUNT = 8`; only `AstParser` needed a new budget.

## Focused tests

- `cargo fmt`
- `cargo test -p unica-coder platform_xml::probe::tests`
- `cargo test -p unica-coder platform_xml::provider::tests`
- `cargo test -p unica-coder platform_xml::support::tests`
- `cargo test -p unica-coder platform_xml::decoder::tests`

## Commit

- `fix: bound platform XML review parse paths`
