---
id: INV.PERF.SERVICE-READINESS-WITHOUT-SOURCE-SCAN
check:
  - crates/unica-coder/tests/workspace_service_support/readiness_before_preparation.rs::production_service_readiness_does_not_wait_for_provider_source_preparation
---

# Сервис отвечает на проверку готовности до обхода исходников

Готовность транспорта сервиса рабочего пространства не требует полного
обхода исходников. Большой проект не должен мешать сервису начать отвечать
на ping только из-за вычисления состояния исходного дерева.

Подготовка данных для анализа относится к рабочей операции и расходует
её срок. Готовый транспорт сам по себе не означает готовый индекс или
проверенную свежесть результатов.
