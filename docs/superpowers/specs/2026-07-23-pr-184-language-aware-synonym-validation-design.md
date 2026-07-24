# PR 184: Owner-Aware Metadata Presentation Validation

## Goal

Correct the metadata convention validator introduced by PR 184 so that it
checks the text actually used by list commands, applies the rule only to
metadata types that support `ListPresentation`, and declares the exact XML
read-set required by the format guard introduced in PR 188.

The Rust implementation and the Python parity oracle must expose the same
behavior. Hidden boolean arguments must not create a second, weaker public
validation mode.

## Decisions

- The conventions adapted from `TemplatesNewObject1C` are treated as general
  project conventions, not as rules limited to 1C:Accounting.
- An absent translation is valid while localization is in progress.
- An empty synonym may be valid when the platform generates the presentation.
  Therefore `meta-validate` does not emit `Synonym is empty`.
- The 38-character command-interface limit is checked only for metadata types
  that support `ListPresentation` in the 1C:Enterprise 8.3.27 platform model.
- For each registered configuration language, non-empty `ListPresentation` is
  the command text; non-empty `Synonym` is used only when that language has no
  list presentation.
- Missing or empty localized text is skipped without a warning.
- Every non-external metadata object is resolved through its owning
  `Configuration.xml`. Observed `v8:lang` and language-neutral values are not
  fallback language profiles.
- A non-external object fails validation when its owner or registration cannot
  be resolved. A type that supports `ListPresentation` also fails when the
  owner's complete registered language profile cannot be resolved.
- `ExternalReport` and `ExternalDataProcessor` are independent XML artifacts.
  They do not resolve or inherit a neighboring configuration language profile,
  and the list-command rule does not apply to them.

## Validation Modes

The string argument `InternalLocalOwnerOnly` is removed. It currently conflates
two different operations:

1. public semantic validation of an object in its owner context;
2. private structural validation used to verify a file just written by a
   mutating operation.

`unica.meta.validate` always performs owner-aware semantic validation and
follows the references required by its checks. It has no reduced public mode.

Mutating operations use a private typed owner-shape validator for transactional
post-write checks. They do not call the public validator through a hidden
boolean argument. The same cleanup applies to configuration-owner checks:
`unica.cf.validate` remains the full public validator, while compile/edit
transactions call an explicit private configuration-owner validator.

This removes `InternalLocalOwnerOnly` from argument maps throughout the native
implementation without forcing every local post-write check to become a full
workspace validation.

## Owner And Language Resolution

The validator first parses the requested metadata descriptor and determines
whether its type participates in the list-command rule.

For every non-external object in a configuration or extension source-set:

1. resolve the containing `Configuration.xml`;
2. verify that the object is registered in
   `Configuration/ChildObjects`.

Owner resolution reuses the source-set and platform-owner model introduced by
PR 188. When no source-set is configured, ancestry lookup is accepted only
when the candidate `Configuration.xml` actually registers the object; directory
proximity alone is not ownership.

For a type participating in the list-command rule, resolution continues:

3. read registered `Language` names in declaration order;
4. resolve every name to `Languages/<Name>.xml`;
5. read a non-empty `Properties/LanguageCode` from every registered language.

Missing `Configuration.xml`, missing object registration, an empty language
set for a participating type, a missing registered language file, malformed
owner/language XML, or an empty language code is a validation error. The
validator does not silently switch to languages observed in the object.

Duplicate language codes are deduplicated in registration order. Missing or
empty presentation text for an otherwise valid registered language remains
allowed and is skipped.

Types outside the list-command rule do not read language descriptors for this
check, but they still require the owner root and registration. This keeps the
owner contract uniform without inflating the content read-set with unused
language files.

## External Reports And Data Processors

Platform XML exports of external reports and data processors are independent
root artifacts:

- `<Name>.xml` contains `ExternalReport` or `ExternalDataProcessor`;
- a single external source-set may contain several independent descriptors;
- a nearby `Configuration.xml` does not own those descriptors;
- their platform properties do not include `ListPresentation`.

Therefore `meta-validate` treats each external descriptor as its own owner and
does not read configuration or language files for the 38-character rule. If a
future validation rule needs the runtime configuration language profile of an
external artifact, that relationship must be supplied explicitly; it must not
be inferred from directory proximity.

## Applicable Metadata Types

The platform capability is represented by a dedicated predicate rather than
being coupled to standard-attribute validation. The 8.3.27 set is:

- `ExchangePlan`
- `FilterCriterion`
- `Catalog`
- `Document`
- `DocumentJournal`
- `Enum`
- `ChartOfCharacteristicTypes`
- `ChartOfAccounts`
- `ChartOfCalculationTypes`
- `InformationRegister`
- `AccumulationRegister`
- `AccountingRegister`
- `CalculationRegister`
- `BusinessProcess`
- `Task`

The effective validation scope is the intersection of this capability set and
the metadata types currently accepted by `meta-validate`.

The parity fixture uses `Enum`, not `CommonModule`. It contains valid
`GeneratedType` entries, is registered in `Configuration.xml`, and uses export
format `2.20`.

## Exact Platform-XML Read-Set

A shared read-plan resolver is used by both the format guard and the validator.
This section describes the platform-XML content whose export format PR 188
must classify; non-XML source reads and existence/directory-membership probes
used by semantic checks are outside that versioned set. For each requested
non-external object, the deterministic platform-XML read-set is:

1. the requested metadata descriptor;
2. its owning `Configuration.xml`;
3. for a type participating in the list-command rule, each registered
   `Languages/<Name>.xml` in declaration order;
4. for registrar-sensitive registers, the sorted prefix of document
   descriptors inspected until the matching registrar is found.

The plan excludes unregistered language files, documents after a registrar
match, directory-membership probes, and external-artifact neighbors. Batch
plans use stable deduplication and preserve handler read order.

The owner path remains in the plan even when it is malformed. A registered
language path remains in the plan even when the file is missing, while format
classification only inspects content that exists. This lets the validator
produce the semantic missing-file error without hiding the attempted read from
the plan.

## Command Text Selection

The validator parses `Synonym` and `ListPresentation` as collections of
localized values rather than selecting the first `v8:item`.

For every resolved configuration language:

1. collect all non-empty `ListPresentation` values for the language;
2. if any exist, validate all of them and ignore `Synonym` for that language;
3. otherwise validate every non-empty `Synonym` value for the language;
4. skip the language when neither property supplies text;
5. emit a warning when a selected text exceeds 38 Unicode characters.

The warning identifies the language and whether the measured text came from
`ListPresentation` or `Synonym`. Duplicate values for one language are checked
instead of silently discarding later values.

There is no language-neutral or observed-language selection branch. The old
neutral regression test is replaced by owner/profile failure tests.

## Error Handling And Parity

Configuration and language parse failures are explicit validation errors.
The Python oracle catches targeted `OSError` and `lxml.etree.XMLSyntaxError`
exceptions and reports the affected path. It does not use `except Exception`
to silently enter a different language-resolution mode.

Rust and Python use the same type capability set, owner/profile requirements,
selection order, warning text, and error conditions.

## Documentation And Provenance

`metadata-conventions.md` documents the type restriction, owner-derived
language profile, and `ListPresentation` to `Synonym` precedence. It does not
claim support for standalone observed or neutral language fallback.

The link in `plugins/unica/references/README.md` is relative to that file:
`platform/metadata-conventions.md`.

The provenance entry and `ATTRIBUTIONS.md` enumerate the complete adapted
scope: naming, synonyms, presentations, fill checks, catalog code conventions,
and information-register command-interface conventions. No
1C:Accounting-specific limitation is added because the project deliberately
adopts these as general conventions.

## Tests

Tests are added before production changes and must fail for the expected
behavioral reason on the PR head or on the PR 184 plus PR 188 merge tree.

Rust and Python parity coverage includes:

- a registered `Enum` with Russian and English language files;
- a long second-language synonym;
- a short `ListPresentation` overriding a long `Synonym` for the same language;
- per-language fallback when one language has no `ListPresentation`;
- an empty translation alongside completed translations;
- a completely empty synonym producing no warning;
- a registered `CommonModule` with a long synonym producing no command-text
  warning and reading no language descriptors;
- a standalone non-external metadata object producing an owner-resolution
  error;
- missing `Configuration.xml` for an applicable type producing an error;
- missing registration, missing language XML, malformed language XML, empty
  language code, and empty registered language set producing errors;
- external report/processor descriptors not acquiring a neighboring
  configuration profile;
- exact read-set inclusion of owner and registered languages;
- exclusion of unregistered languages and post-match registrar documents;
- format warning when a read language descriptor uses a newer export format;
- absence of every `InternalLocalOwnerOnly` argument-map use.

Existing attribution, provenance, package, Rust, format-guard, and MCP
script-parity suites remain green.

## Non-Goals

- Requiring every configured language to have a completed object translation.
- Validating synonyms of attributes, tabular sections, dimensions, or
  resources in this PR.
- Inferring a runtime configuration for an external report or processor.
- Adding `FilterCriterion` support to `meta-validate`.
- Turning private transactional owner-shape checks into full workspace
  validation.
- Changing warning severity or the public `unica.meta.validate` tool name.
