# Final Source Adapter Stabilization Report

Date: 2026-07-26

## Scope

This stabilization resolves final-review findings 1, 2, 4, and 5 together:

- common source-adapter contracts now use a source-neutral captured session;
- Platform XML captures the requested aggregate and authorized source-root
  support/configuration evidence in one immutable revision;
- `ParentConfigurations.bin` v6 uses the real counted vendor/rule grammar;
- type descriptions are normalized while namespace scope is still available.

Cursor/cache findings 3, 6, and 7 were not changed.

## Decisions

- `BuiltInSourceAdapterRegistry` owns capture-provider registration and always
  runs `capture -> registered probes -> selected reader`.  `SourceReadAdapter`,
  `SourceProbe`, and registry APIs contain no `PlatformXmlProvider` method or
  family-specific captured-inspect bypass.
- `CapturedSourceSession` is source-neutral and supports internal safe typed
  downcast.  Platform XML performs that downcast only in its own probe/reader.
  A fake CF session verifies that a second family uses the same registry path.
- `meta.info` stores only a generic captured session plus authorization binding
  evidence.  It calls generic `inspect_captured`; it does not reopen a path or
  serialize a capture path publicly.
- Platform XML captures the descriptor aggregate, source-root
  `Configuration.xml`, and `Ext/ParentConfigurations.bin` twice before
  accepting a revision.  Source-root evidence is revision-covered and only
  represented by internal virtual keys.
- v6 is `{6,globalFlag,vendorCount,vendorBlock*}`.  Each vendor validates
  `providerUuid,vendorFlag,vendorConfigurationUuid,version,vendor,name,ruleCount`
  and consumes exactly the declared configuration/object records.  Configuration
  UUID evidence is checked against captured `Configuration.xml`.
- Multi-vendor facts remain retained.  Because an effective merge rule is not
  proven, they yield `UnknownReadOnly`, never authorability.  Configuration lock
  remains monotonic over object `Removed` or editable rules.
- Type-description roots require the Platform metadata URI; members and
  qualifiers require the Platform data-core URI; QName values resolve to the
  official XML Schema or current-configuration URI.  Prefix spelling is not
  semantic and detached raw XML is not retained.

## RED

- The first compile pass exposed direct tests and helper code that still called
  the old path/provider and raw-XML APIs.  These were converted to captured
  probe helpers and normalized `TypeSet` fixtures.
- The first support pass exposed two synthetic assumptions: a root-level
  support file and an invented `schema/scope/two-rule` profile.  Tests now use
  `Ext/ParentConfigurations.bin`, declared rule counts, and the tracked real
  fixture.
- Exact consumption initially classified a valid counted prefix plus surplus as
  an alternate malformed rule.  The decoder now reports `TrailingData` at the
  first surplus field.

## GREEN

`cargo fmt` completed.

Focused tests completed successfully: 121 tests total.

| Filter | Count |
| --- | ---: |
| `source_adapters::platform_xml::provider::tests` | 8 |
| `source_adapters::platform_xml::probe::tests` | 17 |
| `source_adapters::platform_xml::decoder::tests` | 27 |
| `source_adapters::platform_xml::schema::tests` | 6 |
| `source_adapters::platform_xml::projector::tests` | 19 |
| `source_adapters::registry::tests` | 11 |
| `source_adapters::platform_xml::support::tests` | 8 |
| `support_guard::tests` | 3 |
| `native_operations::meta::tests` | 18 |
| `source_adapters::certification` | 4 |

The certification suite includes the tracked
`tests/fixtures/.../on-support/Ext/ParentConfigurations.bin` layout and proves
that its locked object projects as `SupportLocked`.

## Accepted limits

- Only Platform XML 2.20 is certified.
- Capture remains bounded to one target aggregate plus the two required
  source-root evidence files; it does not claim hostile-filesystem transactional
  guarantees beyond the existing eager two-capture consistency check.
- Support input is bounded to 1 MiB, at most 8 vendor blocks, and at most 4096
  rules per vendor.  Unsupported or ambiguous layouts fail closed.
- Multi-vendor merge semantics are deliberately deferred rather than guessed.

## Fix Round 1

- RED: foreign capture/probe processing could abort inspection, registry trusted unbound descriptors and Ready snapshots, direct QName type properties could fall through to scalar decoding, and support alternatives discarded later UUID errors.
- GREEN: declared family gates capture/probe routing; Platform XML returns `NoMatch` for foreign sessions; descriptors and Ready snapshots are bound to immutable sessions and selected readers; all type properties use namespace-aware decoding; support alternatives retain the furthest typed error.
- Accepted limit: configured source sets declare a source family but no XML revision. The binding carries this absence explicitly; a configured revision is checked whenever present.
