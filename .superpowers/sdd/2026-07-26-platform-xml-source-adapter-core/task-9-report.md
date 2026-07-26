# Task 9 report: Platform XML read-adapter certification

## Certified boundary

`certify_read_adapter` exercises `BuiltInSourceAdapterRegistry`, not decoder or
projector internals. It certifies Platform XML 2.20 as `ready` with exact
adapter ID `platform-xml-2.20`, and certifies 2.19 as the exact seven-key typed
unavailable envelope with diagnostic code `format_unsupported`.

The supported fixture is a bounded temporary Platform XML source set containing
a Document, Attribute, TabularSection, Command, Form, and SpreadsheetDocument
MXL template. The Form has `Partial` coverage. All actions are non-executable;
this certification claims object-level read navigation only, not Form internals,
cross-family semantic parity, or a writer.

The closed failure cases are duplicate child identity, duplicate Form identity,
noncanonical MXL filename, malformed root XML, unreadable support evidence, and
invalid source map. They either return the expected typed error or remain
read-only without an executable action.

## Public typed gateway capture

Captured by
`public_typed_gateway_platform_xml_2_20_serializes_navigation`, which invokes
`native_operations::meta::inspect_meta_navigation` on the 2.20 fixture and
prints the result from `serde_json::to_string` under the public
`data.navigation` envelope. This is an actual serialized test invocation, not
handwritten example data. The compact capture is path-free and includes the
full navigation envelope shape plus one canonical typed property/capability:

```json
{"data":{"navigation":{"schemaVersion":"1","status":"ready","snapshot":{"sourceId":"workspace:main","revision":"sha256:6639379e4ec1d89d128a206c994ad093e27284717ef4b23f290e7ef67a40c936","consistency":"consistent","adapterId":"platform-xml-2.20"},"root":{"sourceId":"workspace:main","objectKey":"uuid:11111111-1111-1111-1111-111111111111","identityStrength":"persistent","kind":"document","displayName":"Shipment"},"nodes":[{"objectRef":{"sourceId":"workspace:main","objectKey":"uuid:11111111-1111-1111-1111-111111111111","identityStrength":"persistent","kind":"document","displayName":"Shipment"},"reference":{"sourceId":"workspace:main","objectKey":"uuid:11111111-1111-1111-1111-111111111111","identityStrength":"persistent","kind":"document","displayName":"Shipment"},"properties":{"name":{"type":"string","valueState":"explicit","value":"Shipment","provenance":"descriptor","capability":"readOnly"}},"capabilityState":{"resolutionState":"resolved","authorability":"unknown_read_only"},"actionProfile":"document_metadata_object"}],"relations":[],"diagnostics":[]}}}
```

## Completion-criteria audit

| # | Result | Evidence |
| --- | --- | --- |
| 1 | Pass | Registry selects 2.20 by probe evidence and snapshot adapter ID. |
| 2 | Pass | 2.19 produces `unavailable` / `format_unsupported`; no 2.20 reader result exists. |
| 3 | Pass | Unavailable is the typed seven-key navigation envelope; no legacy analyzer path is invoked. |
| 4 | Pass | Gateway capture asserts no fixture root path is serialized. |
| 5 | Pass | Duplicate child identity returns `IdentityCollision`. |
| 6 | Pass | Certified Document has an owning contains relation. |
| 7 | Pass | 2.20 nodes carry `Compatible`; 2.19 is excluded before read. |
| 8 | Pass | Every certified action is non-executable; adapter source access remains read-only. |
| 9 | Pass | Unreadable support remains non-authorable/non-executable; focused support-guard suite covers mutation blocking. |
| 10 | Pass | Invalid `v8project.yaml` returns `SourceUnavailable`; no synthetic identity is emitted. |
| 11 | Pass | Focused meta suite verifies typed-only output and removed legacy inputs/stdout. |
| 12 | Pass | Focused meta suite verifies packaged `meta-info` skill is typed-navigation-only. |
| 13 | Pass | Gateway capture asserts `type`, `valueState`, and `capability`; focused meta suite covers canonical property states. |
| 14 | Pass | Focused meta suite covers structured 1C type values and type sets. |
| 15 | Pass | Focused meta suite covers HMAC snapshot cursors, cache scope/eviction, and rejects `pageSize > 100`. |
| 16 | Pass | Certification and focused suites pass; full crate test/build passed after this report edit. |

## Maturity note

The manifest was already `ReadCompatible` in the checked-out baseline before
Task 9 edits. Certification now supplies the missing evidence, but strict
temporal ordering of the original promotion cannot be reconstructed from this
task alone. The claim remains limited to object-level navigation; Form coverage
is explicitly partial and no writer is present.
