---
id: INV.WIRE.CF-IMPORT-KEEPS-ITS-SOURCE-INTACT
check:
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::preview_names_the_source_and_the_target_without_touching_the_infobase
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::preview_refuses_a_plan_for_another_artifact_or_one_that_applied
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::apply_repeats_the_preview_and_attributes_the_infobase_state_to_the_provider
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::stale_apply_stops_after_the_non_executing_preflight
  - crates/unica-coder/src/infrastructure/daemon/v13_cf_import.rs::apply_refuses_a_provider_that_touched_the_source_or_reported_nothing_applied
gap: https://github.com/IngvarConsulting/unica/issues/950
---

# Импорт конфигурации сохраняет входной файл

Правило относится к сохранённому внутреннему обработчику `cf.import`.
Это имя снято с публичной поверхности. `upload` пока недоступен: старый
раннер после загрузки также применяет конфигурацию к БД, а целевой upload
должен останавливаться после загрузки основной конфигурации.

`cf.import` принимает непустой CF или CFE внутри рабочего пространства;
для CFE требуется имя расширения, для CF оно запрещено. Режим загрузки
фиксирован: `load`; аргумент `mode` не принимается. Preview ничего
не применяет и подтверждает запрошенный файл и цель. План явно называет
совместимость до применения неизвестной: `compatibilityKnownBeforeApply: false`.
Применение доступно без допуска исходников, но требует `ifRev` своего preview; несовпадение
останавливает вызов до исполняющего запуска раннера.

Ревизия preview связывает проектный файл, SHA-256 входного файла, версию
раннера и цель импорта. Изменение этих входов требует нового preview;
прежний `ifRev` не разрешает исполняющий запуск. Состояние самой базы
этой ревизией не фиксируется.

Успех допускается, только если раннер подтвердил применение, а входной
файл остался байт в байт прежним, в том числе при подмене без изменения
размера. Unica не проверяет состояние базы после загрузки: ответ называет
его засвидетельствованным провайдером. Пути платформы и журналов не
переносятся в результат.

Проверки исполняют адаптер с управляемыми ответами раннера и настоящими
входными файлами. Они не запускают платформу 1С и не обещают откат её
эффектов при отказе.
