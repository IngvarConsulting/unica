# Broken BSL body

Synthetic XML 2.20 source derived from workspace-code, reduced to two registered catalogs. Catalog.Пример has a deliberately missing EndIf at line 3; Catalog.Здоровый has a valid BeforeWrite event and helper method. This tests recovery text, parser diagnostics, unavailable structure, honest code capabilities and refusal without writes.

The source is intentionally invalid BSL and is not claimed to import or execute in 1C. The healthy neighboring module controls that the refusal remains local to the damaged source.
