# Issue 126: 1C 8.3.27 / export 2.20 research baseline

## Status

This memo records the evidence, the implemented compatibility boundary, and the
completed acceptance checks for the fixed 8.3.27 / 2.20 profile.

## Fixed profile decision

The only writable profile in this phase is platform `8.3.27` with the exact
raw XML export literal `2.20`.

The official local 1C guide is
`docs-local/1ci/8.3.27/en/developer/Chapter_2._Managing_configurations/2.17._Dumping_configurations_to_files_Restoring_configurations_from_files/2.17.2._Export_format_versions/index.md`.
It maps the 8.3.27 platform line to export format `2.20` and treats a missing
version on a version-owning root as `1.0`.

| Source format evidence | Required behavior |
| --- | --- |
| Below `2.20`, including a missing version-owning root version | Read-only operations may warn and continue where safe. Mutation must stop before its first write and recommend explicit user-driven load/re-export with 1C 8.3.27. |
| Exact raw `2.20` | The supported profile. |
| Above `2.20` | Read-only operations warn; mutation stops before writing. The product message says that 1C 8.5 is not supported yet but is planned. No downgrade is offered. |
| `2.20.0`, `02.20`, `2.020`, entity-spelled equivalents, malformed, unreadable, or ambiguous owner | Invalid for mutation. Equality is lexical before entity decoding, not numeric normalization. |

Automatic migration is out of scope. Future multi-format support must add a
profile resolver rather than weakening this boundary.

## Evidence hierarchy

1. Official 1C export-version documentation establishes the profile mapping.
2. Runtime XSD/XDTO from 8.3.27.2074 establishes structural constraints where a
   complete document schema is actually exported.
3. EDT declarations cover families that the runtime archive exposes only as
   types or omits as complete dump documents.
4. A pinned 1C 8.3.27.2074 import/check/export roundtrip is decisive evidence
   of platform acceptance and semantic stability.

Raw XSD is therefore evidence, not a universal source-tree validator. A
platform-valid document must not be rewritten merely to satisfy an incomplete
or incompatible raw schema.

## Static XSD/XDTO result

The current independent-corpus static run covered 649 XML pre/post documents:

| Result | Count | Interpretation |
| --- | ---: | --- |
| Strict pass | 11 | The exported schema had a controlled document-root/type binding and accepted the XML. |
| Inconclusive | 638 | The runtime schema is incomplete, type-only, or has a known incompatibility for that family. |
| Failure | 0 | No applicable strict-schema, root, QName-prefix, lexical owner-version, or well-formedness violation was found. |

The `638` inconclusive results are not promoted to passes. They are the reason
the exact-platform gate remains a required acceptance condition.

## Corpus coverage boundary

Two independently generated `schemaVersion: 2` corpora normalized to the same
case-contract digest:

`e1f9b8b73288699b5202df1c0814110b255fa80eec908f1b7ea921f55acb82f8`

Each has 63 selected native-mutator checkpoints, 1039 regular files, and 90
empty directories. The manifest binds public calls, owner links, XML and
non-XML bytes, expected deltas, and empty-directory topology.

The 63 cases cover the complete **native-mutator** inventory with selected
representative branches. They do not prove every public argument combination
and do not by themselves cover non-native `BuildRuntime` or runtime-adapter
routes. Any non-native route capable of publishing platform XML must have a
separate verified staged-publication boundary and end-to-end proof, or fail
before it writes source XML.

## Completed acceptance evidence

The following reproducible checks completed against the implementation in this
change set:

- two independently generated corpus manifests matched the expected
  case-contract digest and retained 1039 regular files plus 90 empty
  directories each;
- both static XSD/XDTO verifier runs reported 11 strict passes, 638 explicit
  inconclusive results, and zero failures;
- the full 63-checkpoint `ibcmd` 8.3.27.2074 gate passed: 63 passed, zero
  rejected, zero normalized, zero source errors, and 432 platform commands;
- the gate verified unchanged corpus bytes and topology, and unchanged pinned
  platform-install bytes and topology (4337 files and 96 directories);
- every public XML-writing route is covered by the native profile guard and
  corpus inventory, or uses the separately verified synchronous staged full
  dump boundary; and
- the implementation and documentation review completed without unresolved
  findings.

This baseline is the prerequisite writer/profile phase of issue #126. The
optional capability-driven XDTO/XSD service proposed by the issue is a later
design phase.
