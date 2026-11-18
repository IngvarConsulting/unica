# Native bytes of new template files

These files are unmodified exports from 1C 8.3.27.2074, format 2.20.
`manifest.json` binds the retained-route fixtures to their original source,
both native exports, the complete corpus manifest and the completed platform
report. All eight retained-route XML files are identical between the two
independent infobase cycles. The canonical DCS fixture has its own provenance
record, linked from the same manifest.

The source composition is `4bf85f5f32c7fdea3728a8a5b50b9aa2b77cb202`.
Its five `template-add-*` cases execute retained `unica.meta.edit` in the
research test artifact. They are not canonical `unica.apply` scenarios.
The platform report SHA-256 is
`db331df7f10c4272a625613138c2471a273a8647f3cc6878dafbe60696e86372`.

The expected standalone descriptors cover SpreadsheetDocument,
DataCompositionSchema, HTMLDocument, TextDocument and BinaryData. The three
expected XML payloads are the empty spreadsheet, retained empty DCS and HTML
page descriptor. Tests compare complete bytes. The sole permitted substitution
is the validated UUID on the one `Template` element; names, property values,
indentation, namespace declarations, BOM, line endings and final bytes stay fixed.

The canonical DCS initializer retains a StructureItemGroup and its
SelectedItemAuto. The retained DCS initializer has empty settings. Its payload
fixture must not be used to replace or accept the different canonical payload.
The canonical descriptor and spreadsheet tests use the same field values as
the native fixtures, through the actual preview/token/publication route.

`native-canonical-dcs-content.xml` has a separate provenance chain in
`canonical-dcs-provenance.json`: canonical `unica.apply` preview and token-only
publication by the ordinary core, followed by a native import/check/export.
That first experiment remains FAILED because its strict semantic guard rejected
the platform's removal of two empty elements: top-level settings selection and
group order. Both Group and Auto survived. A separately reviewed continuation
imported the exact first export into another fresh infobase. Its exported payload
is byte-identical to the first, and only ConfigDumpInfo bookkeeping changes in
the full tree. This fixture is the unmodified final payload, including BOM,
CRLF and no terminal newline. It is an oracle for that particular scaffold,
not proof that a future producer or arbitrary DCS content has passed native
acceptance.

`seed/` is the complete second export of the separate `meta-compile-report`
checkpoint: a Report without templates, its Configuration and Language,
auxiliary XML and both modules. It is copied into a temporary workspace without
rewriting any source file. ConfigDumpInfo is retained exactly from export2;
its generated bookkeeping differs from export1, as recorded in the manifest.

The already-correct HTML page reuses `../html-template/native-ru.html`.
Text and Binary payloads are exactly empty in source and both exports; tests
keep those controls empty. No text, binary, HTML or BSL normalization is implied.

The corresponding `.gitattributes` entries disable checkout EOL conversion for
these raw files. No platform command is needed to run the Rust tests. These
fixtures describe the retained native observations, not acceptance of a future
producer change or release.
