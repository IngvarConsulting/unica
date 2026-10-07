---
id: INV.RUNTIME.RUNNER-BASE-REFUSALS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::every_runner_wire_code_maps_away_from_the_fallback
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::runner_outcomes_follow_the_codes_rather_than_the_fallback
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::runner_refusals_keep_their_outcome
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_base_contention_refusals_keep_their_outcomes
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_exchange_refusals_name_both_ways_out_and_the_generations
---

# Отказ раннера о базе называет, кто действует дальше

Раннер отказывает, когда базой занят кто-то другой или когда память рабочей
копии о базе разошлась с базой. Unica отображает эти отказы в свой словарь
так, чтобы исход называл следующего действующего:

- `infobase_busy` — файловую базу держит другая команда — отвечает
  `concurrent_change`: тот же вызов можно повторить, когда она закончит.
- `infobase_held` — файловую базу держит другая рабочая копия — отвечает
  `invalid_state`: повтор этого не меняет. Текст раннера называет копию и
  способ освободить базу, `next` — превью создания своей чистой базы.
  Согласие делить базу (`shared: true`) Unica аргументом не выставляет:
  раннер читает его из местного слоя.
- `non_fast_forward` — база ушла вперёд записанного поколения — и
  `no_memory` — памяти о базе нет — отвечают `invalid_state`. Отказ несёт
  в `data` код раннера, набор и поколения базы и записи, если раннер их
  назвал, а в `next` — два превью для того же набора: выгрузку (`pull`) и
  перезапись базы (`push` с `force:true`). Текст отказа — свой: команды
  командной строки раннера и пути проекта наружу не идут. Выбор между ними за тем, кто знает, чья правка
  верна; Unica `force` сама не добавляет.

Код раннера, которого словарь не знает, идёт запасным `provider_failed`.
