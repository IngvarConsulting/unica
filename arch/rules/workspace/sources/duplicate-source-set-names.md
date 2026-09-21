---
id: INV.SOURCE.AMBIGUOUS-NAME-HEALTH
check:
  - crates/unica-coder/src/infrastructure/project_health/layout.rs::duplicate_names_have_one_group_fact_and_no_ambiguous_root
  - crates/unica-coder/src/infrastructure/project_health.rs::duplicate_source_set_names_do_not_invent_an_addressable_repository_row
---

# Одноимённые наборы исходников получают общую диагностику

Если в построенной карте имя набора исходников повторяется, проверка
готовности сообщает одну причину на уровне проекта с полным количеством
одноимённых наборов.
Она не выбирает один из них по этому имени и не создаёт проверки репозитория,
которые невозможно однозначно отнести к набору.

Тесты проверяют разбор карты и итоговую диагностику проверки готовности.

Пропущенные при автообнаружении наборы `main` описаны отдельным
[правилом диагностики конфликта](autodetected-main-conflict.md).
