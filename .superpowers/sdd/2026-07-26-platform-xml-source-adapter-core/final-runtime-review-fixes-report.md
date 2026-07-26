# Final Runtime Review Fixes

Date: 2026-07-26

## Review verification

- P1 confirmed: cursor issuance serialized a typed `NavigationSelection`, while authentication serialized the raw `serde_json::Value`; `preserve_order` could therefore reject a semantically unchanged selection with reordered object keys.
- P2 confirmed: the root descriptor alone preflighted XML depth. Registered Form and Template descriptors, managed Form content, and MXL content parsed without the shared depth bound. `single_metadata_class` also collected every wrapper child before deciding that more than one class is ambiguous.

## Changed contracts

- Cursor authentication now signs one named canonical claim set whose selection component is the normalized `selectionHash`. The raw selection is structurally preflighted before authentication, normalized only afterwards, and checked against that authenticated hash.
- All captured XML documents pass the same bounded parse helper before semantic decoding. Descriptor cardinality reads at most two metadata-class children and retains `projection_ambiguous` for the existing multiple-class schema contract.

## Focused tests

- `cargo fmt`
- `cargo test -p unica-coder domain::navigation::tests`
- `cargo test -p unica-coder platform_xml::decoder::tests`

## Commit

- `fix: address navigation runtime review findings`
