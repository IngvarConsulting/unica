---
id: INV.SURFACE.DOCS-SOURCE-SELECTION
check:
  - crates/unica-coder/src/infrastructure/application_ports.rs::canonical_v13_docs_search_passes_the_scalar_source_kind_to_the_shared_registry
  - crates/unica-coder/src/infrastructure/application_ports.rs::canonical_v13_docs_search_rejects_unknown_source_as_typed_unsupported_source
  - crates/unica-coder/src/infrastructure/application_ports.rs::canonical_v13_docs_search_rejects_configuration_help_until_it_is_actor_safe
  - crates/unica-coder/src/infrastructure/application_ports.rs::canonical_v13_docs_search_omits_unsafe_configuration_help_by_default
---

# Поиск справки ограничивается выбранными источниками

При поиске через `unica.docs` без `source` запрашиваются справка платформы
и стандарты разработки. Явный `source` выбирает один источник.
Неизвестное имя даёт `unsupported_source`.

`configuration-documentation` не включается по умолчанию, а явный запрос
этого источника отклоняется до обращения к поставщику: чтение ещё
не использует защищённый доступ актора к исходникам.

Проверки наблюдают запрос, переданный управляемому поставщику. Они
не подтверждают наличие справки или сетевую доступность всех источников.
