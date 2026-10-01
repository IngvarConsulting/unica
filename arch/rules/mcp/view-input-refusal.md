---
id: INV.WIRE.VIEW-INPUT-REFUSAL
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::review_invalid_logical_address_reaches_typed_bad_value_result
  - crates/unica-coder/src/infrastructure/daemon/server.rs::valid_unknown_source_reaches_typed_provider_unavailable_without_scanning
  - crates/unica-coder/src/infrastructure/daemon/server.rs::zero_fence_view_rejection_accepts_only_the_exact_canonical_envelope
---

# Ошибочный адрес view отклоняется до чтения исходников

Некорректный адрес `view` возвращает `bad_value`, а правильный по форме
адрес неизвестного набора — `provider_unavailable`. Для этих отказов Unica
не сканирует исходники и не захватывает их ревизию.

Без допуска исходников можно выдать только ожидаемый ответ об этой ошибке.
Успешный результат, дополнительные данные, ревизия, курсор или другая
диагностика через этот путь не публикуются.
