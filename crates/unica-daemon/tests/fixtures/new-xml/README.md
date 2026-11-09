# Native newly created XML

The initial three XML files and companion BSL module are unmodified 1C 8.3.27.2074 exports. Each checkpoint
completed import/apply, check and export in two isolated databases; the second
imported the first export. The two exports of each retained file are byte-equal.
`manifest.json` records raw paths, hashes, source revision, build provenance,
platform binary and installation identity. No fixture bytes were normalized.

The form checkpoint used canonical `unica.apply` through the research corpus
binary. The Catalog checkpoint used retained typed `unica.meta.add`; it was a
minimal Catalog, with no attribute operations. These native observations do not
claim normal-runtime, canonical object.create or full-release acceptance.
The new tests exercise canonical object.create separately against the same
minimal Catalog result.

All retained files contain one UTF-8 BOM, CRLF line endings and no terminal newline.
New managed form content also contains the native dcssch namespace declaration.
The tests bind only generated identity values in memory: one Form UUID, or the
Catalog UUID plus TypeId/ValueId of exactly five named GeneratedType entries.
Everything else, including namespaces, formatting and properties, remains part
of the full byte comparison. Existing authored XML and BSL are outside this
new-file formatting boundary. Native TextDocument template bytes remain separate
research evidence; canonical template support is not asserted here.

`native-form-module.bsl` is a 492-byte guard for the unchanged generated form
module: one BOM and five region blocks with CRLF, not a BOM-only empty module.
Its original source and both native exports are exact. The Catalog ObjectModule
remains BOM-only; the tests distinguish these existing behaviors.

The minimal ChartOfAccounts descriptor comes from the ongoing full66 replay on
composition 4bf85f5f. Its two already retained exports are byte-identical; the
input differs only by LF/CRLF and one final LF. This is a raw native observation,
not a full66 pass claim. The per-file capture in the manifest overrides the
earlier baseline metadata. The separate completed three-chart oracle also
observes BOM/CRLF/no terminal newline; its attributes are not removed to invent
a minimal golden file. ChartOfAccounts binds exactly 15 generated UUID slots:
the owner plus seven named GeneratedType TypeId/ValueId pairs.
