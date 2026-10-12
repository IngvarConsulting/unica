# Incomplete borrowed metadata without a parent

S389 reads five borrowed objects from a single EXTENSION source set. No parent
configuration is registered, so local observations must remain available without
resolving inherited values.

The five descriptors are byte-exact copies of `extension-*.xml` in
`../../platform_8_3_27/incomplete_borrowed/`. That directory documents two
independent successful import/check/apply/export cycles using 1C 8.3.27.2130,
XML 2.20, and records descriptor and executable hashes. `Configuration.xml` and
`Languages/Русский.xml` come from the first cycle's unmodified extension export.
This is supplemental reader evidence, not the pinned 8.3.27.2074 platform gate.

`v8project.yaml` and `CommonModules/CorpusModule/Ext/Module.bsl` are synthetic
reader inputs. The BSL declares one exported procedure under a server guard.
It verifies that unavailable execution flags leave contexts unknown while method
names, export status, guards, ranges and source lines remain readable. The mixed
workspace, including this added BSL file, was not validated as an importable
platform extension; it makes no claim about module override or inheritance rules.

The scenario checks descriptor omissions separately from explicit false and
empty Comment values, rejects context filtering when availability is unknown,
and verifies the SHA-256 of every source file and project configuration before
and after its sequence of read calls.
