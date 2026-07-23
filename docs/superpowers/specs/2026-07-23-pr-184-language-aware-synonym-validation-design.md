# PR 184: Language-Aware Metadata Presentation Validation

## Goal

Correct the metadata convention validator introduced by PR 184 so that it
checks the text actually used in the command interface, respects the languages
declared by the configuration, and does not report an empty synonym when that
state may be intentional or temporary.

The Rust implementation and the Python parity oracle must continue to expose
the same behavior.

## Decisions

- The conventions adapted from `TemplatesNewObject1C` are treated as general
  project conventions, not as rules limited to 1C:Accounting.
- An absent translation is valid while localization is in progress.
- An empty synonym may be valid when the platform generates the presentation.
  Therefore `meta-validate` must not emit `Synonym is empty`.
- The 38-character command-interface limit is checked independently for every
  applicable language.
- For each language, `ListPresentation` is the command text when it is
  non-empty; otherwise `Synonym` is the fallback.
- Missing or empty localized text is skipped without a warning.

## Language Resolution

When the validated object belongs to a complete configuration dump,
`meta-validate` finds the nearest ancestor containing `Configuration.xml`.
It reads the registered `Language` names from
`Configuration/ChildObjects`, then resolves each name through
`Languages/<Name>.xml` and its `Properties/LanguageCode`.

Only successfully resolved language codes constrain the command-text check.
Missing or malformed language files do not turn metadata convention validation
into configuration validation and do not produce new warnings here.

For a standalone metadata XML without an accessible configuration,
`meta-validate` collects every `v8:lang` found in localized properties of that
XML. It does not scan neighboring metadata files because doing so would make
results depend on unrelated objects and could mix different language profiles.

If localized items contain no usable language code, all non-empty localized
items are still checked as language-neutral values.

## Command Text Selection

The validator parses `Synonym` and `ListPresentation` as collections of
localized values rather than selecting the first `v8:item`.

For every resolved or observed language:

1. use the non-empty `ListPresentation` value when present;
2. otherwise use the non-empty `Synonym` value;
3. skip the language when neither property supplies text;
4. emit a warning when the selected text exceeds 38 Unicode characters.

The warning identifies the language when known and states whether the measured
text came from `ListPresentation` or `Synonym`. This makes the remediation
unambiguous without treating missing translations as defects.

Duplicate items for one language are malformed input. The validator checks
each non-empty duplicate rather than silently discarding later values; broader
structural validation remains outside this change.

## Documentation And Provenance

`metadata-conventions.md` continues to state that synonyms should normally be
meaningful and filled, but classifies that as a manual semantic review item.
It no longer claims that `meta-validate` diagnoses an empty synonym.

The provenance entry and `ATTRIBUTIONS.md` must enumerate the complete adapted
scope: naming, synonyms, presentations, fill checks, catalog code conventions,
and information-register command-interface conventions. No
1C:Accounting-specific limitation is added because the project deliberately
adopts these as general conventions.

## Tests

Tests are added before production changes and must fail for the expected
behavioral reason on the PR head.

Rust and Python parity coverage includes:

- a configuration with Russian and English language files;
- a long second-language synonym that the current first-item parser misses;
- a short `ListPresentation` overriding a long `Synonym` for the same language;
- per-language fallback when one language has no `ListPresentation`;
- an empty translation alongside completed translations;
- a completely empty synonym producing no warning;
- a standalone object using its observed `v8:lang` values;
- a language-neutral localized item with no usable language code.

Existing attribution, provenance, package, Rust, and MCP script-parity suites
remain green.

## Non-Goals

- Requiring every configured language to have a completed translation.
- Diagnosing missing or malformed configuration language files.
- Validating synonyms of attributes, tabular sections, dimensions, or
  resources in this PR.
- Scanning unrelated neighboring metadata objects to infer a language.
- Changing warning severity or the public `unica.meta.validate` tool contract.
