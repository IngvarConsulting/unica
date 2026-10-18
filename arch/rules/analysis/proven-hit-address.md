---
id: INV.APP.CODE-SEARCH-HIT-ADDRESS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::a_text_search_hit_opens_the_narrowest_body_line_address
  - crates/unica-coder/src/infrastructure/daemon/v13_service.rs::body_line_provider_search_uses_current_spans_and_preserves_evidence
  - crates/unica-coder/tests/v13_search_integration.rs::public_body_line_ranges_open_search_hits_and_refuse_line_edits
---

# Адрес текстового совпадения подтверждается текущими исходниками

Адрес текстового совпадения открывает абсолютный диапазон строк `Body`.
Если текущая структура модуля подтверждает содержащий метод, адрес ведёт
в его `Body`; иначе модульный адрес сохраняет полезное совпадение, а предел
установления границ метода обозначается явно. Строка, колонка, фрагмент
и ранг поставщика сохраняются. Адрес для провайдерного фрагмента выдаётся,
только если текущий текст подтверждает этот фрагмент в указанном диапазоне;
имя символа поставщика само по себе границу метода не доказывает.
