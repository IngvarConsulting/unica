---
id: INV.RUNTIME.RUNNER-BASE-REFUSALS
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::every_runner_wire_code_maps_away_from_the_fallback
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::runner_outcomes_follow_the_codes_rather_than_the_fallback
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::runner_refusals_keep_their_outcome
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_base_contention_refusals_keep_their_outcomes
  - crates/unica-coder/src/infrastructure/daemon/v13_source_import.rs::captured_exchange_refusals_name_both_ways_out_and_the_generations
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::captured_pull_into_the_infobase_of_another_copy_names_it
  - crates/unica-coder/src/infrastructure/daemon/v13_extensions.rs::captured_extension_change_in_the_infobase_of_another_copy_names_it
  - crates/unica-coder/src/infrastructure/daemon/v13_infobase_exports.rs::captured_restore_into_the_infobase_of_another_copy_names_it
---

# Отказ раннера о базе называет, кто действует дальше

Раннер отказывает, когда базой занят кто-то другой или когда память рабочей
копии о базе разошлась с базой. Unica отображает эти отказы в свой словарь
так, чтобы исход называл следующего действующего:

- `infobase_busy` — файловую базу держит другая команда — отвечает
  `concurrent_change`: тот же вызов можно повторить, когда она закончит.
- `non_fast_forward` — база ушла вперёд записанного поколения — и
  `no_memory` — памяти о базе нет — отвечают `invalid_state`. Отказ несёт
  в `data` код раннера, набор и поколения базы и записи, если раннер их
  назвал, а в `next` — два превью для того же набора: выгрузку (`pull`) и
  перезапись базы (`push` с `force:true`). Текст отказа — свой: команды
  командной строки раннера и пути проекта наружу не идут. Выбор между ними за тем, кто знает, чья правка
  верна; Unica `force` сама не добавляет.

Запись в файловую базу другой рабочей копии раннер 0.14 отказом не считает:
команда идёт, а метка владельца остаётся прежней. Загрузка набора
копией-владельцем после этого видит, что база ушла вперёд её записи, и
получает `non_fast_forward`; набор, пропущенный по памяти, базу не сверяет.
О такой записи раннер предупреждает прозой без кода. Unica опознаёт это
предупреждение по его устойчивой части и отвечает своим предупреждением
`infobase_of_another_copy` с выходом к превью `infobase.create`: текст свой,
без путей чужой копии, метки владельца и команд раннера. Так отвечают
`push`, `pull`, изменение расширений и `infobase.restore` — в превью, после
исполнения и на отказе `push`; у превью первым выходом остаётся исполнение.
Запуск клиента (`launch`) и отказы остальных операций этого предупреждения не
называют. Прочие предупреждения раннера наружу не идут.

Код раннера, которого словарь не знает, идёт запасным `provider_failed`.
