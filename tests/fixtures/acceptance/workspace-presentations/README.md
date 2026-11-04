# Native presentation metadata fixture

`src/` contains the complete, unmodified second XML export from 1C
8.3.27.2074 on Windows (XML 2.20). It supplies five kinds for issue #1693
and a Catalog regression control. The source set is `main`.

| Kind | Name | Initial representation |
| --- | --- | --- |
| ChartOfAccounts | Accounts | AsDescription |
| ChartOfCalculationTypes | CalculationTypes | AsDescription |
| ChartOfCharacteristicTypes | Properties | AsDescription |
| ExchangePlan | Exchange | AsDescription |
| Task | Tasks | AsDescription |
| Catalog | Items | AsDescription |

Code and description lengths are nonzero for all coded kinds; Task has
nonzero number and description lengths. Properties is hierarchical and
starts with FoldersOnTop=true.

The preparation used copies of the tracked native Catalog, ChartOfAccounts,
ChartOfCalculationTypes and ExchangePlan descriptors, the small existing
characteristic-types descriptor, and the native template-new-xml configuration
and language. Identifiers and self references were renamed. Task began as an
authored minimal descriptor. Task and characteristic-types GeneratedType
identities were authored for import. The platform supplied the complete native
properties and other export files. No working UT configuration or real infobase
was used. `manifest.json` records source templates and actual file hashes.

Two fresh service infobases imported/applied, checked and exported the seed.
The second complete export is retained byte-for-byte. Only ConfigDumpInfo.xml
bookkeeping differed between exports; all other bytes were stable. No modules
were removed from the export and no XML/EOL normalization was applied.

A separate copy changed DefaultPresentation to AsCode for all coded kinds
(including Catalog), to AsNumber for Task, and FoldersOnTop to false for
Properties. Two further fresh service infobases imported/applied, checked and
exported those changes. All changed values survived both cycles; all exported
files apart from ConfigDumpInfo.xml were byte-stable between the two cycles.
The logs, argv arrays, per-command exit codes, durations and report are local
evidence in `.session-temp/platform-1693/`; the manifest binds the report hash.

These observations establish native XML acceptance and retention for this
fixture. They do not establish runtime UI presentation or business behavior,
the Unica producer route, or a full platform-profile corpus gate.

Two further isolated observations returned all six DefaultPresentation values
to AsDescription and hierarchical Properties to FoldersOnTop=true, then made
Properties nonhierarchical with FoldersOnTop=false. Native import/apply, check
and export succeeded for both copies; the exported values matched the inputs.
Thus the platform accepts the retained FoldersOnTop field even when Hierarchical
is false. This is a metadata retention observation, not a runtime list-order test.
The report also distinguishes two unsuccessful seed-preparation imports from
the successful six complete cycles; neither failed import produced the fixture.
