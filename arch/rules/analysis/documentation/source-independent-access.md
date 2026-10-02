---
id: INV.SURFACE.DOCS-BEFORE-ADMISSION
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_documentation_answers_before_source_admission_without_an_actor_lease
---

# Справка доступна без набора исходников

`unica.docs` готовится и исполняется без допущенного набора PlatformXml
и без аренды его ревизии. Каталог без `v8project.yaml` и корней 1С не вызывает
общего отказа допуска: инструмент отвечает результатом поставщика справки
или собственным отказом. Свод `configuration-documentation` отвечает
`unsupported_source` и в таком каталоге.

Класс исполнения остаётся `InlineCandidate`: локальный результат может
вернуться сразу, а работа после общего срока прямого ответа передаётся
в Task. Вызов не занимает актора исходников.
