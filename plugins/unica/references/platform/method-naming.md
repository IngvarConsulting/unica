# Method Naming Under 1C Standard 647

Use this reference before choosing a name for every new or renamed BSL
procedure or function. The normative source is
[1C development standard 647](https://its.1c.ru/db/content/v8std/src/400/100/i8100647.htm);
fetch the current text with `unica.standards.explain` and
`idOrAliasOrUrl: "647"` before quoting a rule.

## Decision Table

| Method intent | Naming shape | Example |
| --- | --- | --- |
| Procedure that performs an action | Infinitive verb | `ЗагрузитьКонтрагента` |
| Function that returns a value | Name of the returned result | `ПолноеИмя`, not `ПолучитьПолноеИмя` |
| Function that constructs a value | `Новый`, `Новая`, `Новое`, or `Новые` plus the result | `НовыеПараметры` |
| Predicate | `Это...` or a short participle describing the result | `ЭтоДокумент`, `ДокументПроведен` |
| Function whose contract is itself an action or check | Infinitive is allowed only for the cases covered by clauses 6.4 and 6.5 | `ВыбратьДанныеПоПравилу` |

Do not add a result type to a method name when the business meaning already
identifies the value. Prefer `ПараметрыПечати` to
`СтруктураПараметровПечати` unless the distinction between result types is part
of the public contract.

## Compatibility Boundary

- Keep platform event-handler names exactly as the platform defines them.
- Do not automatically rename an existing exported or otherwise public method.
  Inspect callers with `unica.code.search` and `unica.code.graph`, then treat a
  compatible migration as a separate API change.
- Keep a project-local public name when compatibility outweighs the naming
  improvement; document the exception instead of silently changing callers.
- Apply the standard to a newly introduced method and to an explicit rename.
  An edit that changes only an existing method body does not reopen its public
  name.

## Patch Workflow

1. Classify the method as a procedure, value function, constructor, predicate,
   action/check function, platform handler, or existing public API.
2. Choose the name from the decision table before producing BSL.
3. Call `unica.code.patch` with `dryRun: true` and inspect
   `data.validation.methodNaming` together with the parser validation.
4. Treat a `warning` as a reason to revise the proposed name before applying.
   `automatedChecks` names each deterministic check and its related standard
   rule without claiming that the whole rule is automated. When
   `semanticReview.required` is true, apply every listed `check` to every listed
   method before applying the patch. The deterministic preview currently
   catches a new or renamed function whose name starts with `Получить` or
   `Get`; the other semantic distinctions remain a required review step because
   the AST alone does not reveal business intent. A `passed` status therefore
   means that the automated checks passed, not that semantic review can be
   skipped.
5. Run `unica.code.diagnostics` for the changed logical module with
   `filter.minSeverity: "hint"` so `FunctionNameStartsWithGet` is not hidden by
   the normal `warning` threshold.

## Review Checklist

- Every new procedure is named as an action in the infinitive.
- Every value function is named after its result rather than after obtaining it.
- Constructors and predicates use their dedicated forms.
- An infinitive function is justified by clauses 6.4 or 6.5.
- Every method listed by `semanticReview.methods` was checked against every
  entry in `semanticReview.checks`.
- Platform handlers and existing public APIs were not renamed implicitly.
- The `code.patch` preview and hint-level diagnostics contain no unresolved
  naming warning for the changed method.
