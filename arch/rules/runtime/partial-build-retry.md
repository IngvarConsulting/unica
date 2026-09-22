---
id: INV.APP.PARTIAL-FALLBACK
check:
  - crates/unica-coder/src/infrastructure/runtime_build_fallback.rs::completed_designer_partial_load_failure_is_retryable
  - crates/unica-coder/src/infrastructure/runtime_build_fallback.rs::unrelated_platform_failures_and_interruption_are_not_retryable
  - crates/unica-coder/src/infrastructure/runtime_build_fallback.rs::malformed_or_mismatched_receipts_fail_closed
---

# Повтор частичной загрузки требует подтверждённого завершения

Unica разрешает полный повтор сборки после частичной загрузки Designer,
только если процесс завершился с кодом `4`, а его структурированный ответ
подтверждает отказ именно этого шага. Из ответа извлекаются набор исходников,
число файлов и внутренний код выхода платформы.

Отмена или превышение срока, зафиксированные Unica, другая ошибка,
обрезанный или некорректный ответ и ответ для другого набора исходников
не дают такого разрешения. Одного текста
«exit code 4» недостаточно.

Связанные проверки относятся к классификатору квитанции. Они не обещают
автоматический полный повтор при отправке исходников через `push`: режим импорта
выбирает раннер, а Unica проверяет его по согласованному плану.
