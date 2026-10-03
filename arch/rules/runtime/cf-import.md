---
id: INV.WIRE.CF-IMPORT-KEEPS-ITS-SOURCE-INTACT
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::upload_refuses_a_receipt_that_implicitly_applied_the_database
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::preview_names_the_source_and_the_target_without_touching_the_infobase
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::preview_refuses_a_plan_for_another_artifact_or_one_that_applied
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::apply_without_prior_preview_attributes_the_infobase_state_to_the_provider
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::input_changed_during_preflight_stops_before_mutating_dispatch
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::apply_refuses_a_provider_that_touched_the_source_or_reported_nothing_applied
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Импорт конфигурации сохраняет входной файл

`upload` адаптера 0.11.2 загружает основную конфигурацию или расширение
без применения к конфигурации БД. Отдельное применение выполняет `apply`.

`upload` принимает непустой CF или CFE внутри рабочего пространства;
для CFE требуется имя расширения, для CF оно запрещено. Режим загрузки
фиксирован: `load`; аргумент `mode` не принимается. Preview ничего
не применяет и подтверждает запрошенный файл и цель. План явно называет
совместимость до применения неизвестной: `compatibilityKnownBeforeApply: false`.
Операция доступна без допуска исходников и требует явный boolean `dryRun`.
`dryRun: false` загружает текущий входной файл без предварительного preview.
`run` не принимает `ifRev` и не выдаёт `rev`; отдельный preview не закрепляет
файл, проект или состояние базы между вызовами.
Внутри вызова проектный файл, его локальное дополнение и входной CF/CFE
сверяются до и после внутреннего preview раннера. Их изменение даёт
`concurrent_change` до загрузки.

Успех допускается, только если раннер подтвердил загрузку без применения к БД, а входной
файл остался байт в байт прежним, в том числе при подмене без изменения
размера. Unica не проверяет состояние базы после загрузки: ответ называет
его засвидетельствованным провайдером. Пути платформы и журналов не
переносятся в результат.

Проверки исполняют адаптер с управляемыми ответами раннера и настоящими
входными файлами. Они не запускают платформу 1С и не обещают откат её
эффектов при отказе.
