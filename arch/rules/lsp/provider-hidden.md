---
id: INV.LSP.PROVIDER-HIDDEN
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::symbol_port_answers_in_unica_places_with_a_substituted_provider
  - crates/unica-code-intelligence/src/symbols/tests.rs::an_answer_names_no_provider_and_a_failed_provider_is_a_unica_reason
  - crates/unica-code-intelligence/src/symbols/tests.rs::a_saved_module_asks_by_position_and_an_edited_one_by_saved_declaration_or_name
---

# Ответ LSP не раскрывает поставщика анализа

Ответы и уведомления LSP строятся из модели и кодов Unica. Имена, коды,
сообщения, идентификаторы и форматы конкретного поставщика анализа кода
в них не передаются.

Замена поставщика не меняет протокол LSP, объявленные возможности и форму
ответов. Отказ или недоступность поставщика выдаётся кодом Unica.
