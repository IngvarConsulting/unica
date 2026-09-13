- Date: `2026-08-22`
- Status: `approved`
- Decision: `DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT`

# Свойство обычного приложения клиентского модуля

## Проблема

В актуальном `upstream/main` реестр записываемых свойств не содержит
`ClientOrdinaryApplication`. Вызов `unica.apply` с `props.set` для
`main:CommonModule.OrdinaryClient` отклоняет платформенное boolean-свойство,
хотя спецификация XML и независимый профиль чтения его уже описывают.

## Решение

`DEC.2026-08-22.COMMON-MODULE-ORDINARY-CLIENT` добавляет свойство в реестр
writer только для `CommonModule`. Канонический planner использует внутреннюю
типизированную проверку метаданных. `unica.view` наблюдает результат в
`props.commonModule.clientOrdinaryApplication` согласно `CTR.SOURCE.MODULE-PROJECTION-SHAPE`.

Диагностика неизвестного свойства перечисляет альтернативы из того же
реестра, отфильтрованные по виду владельца.

## Адаптация PR к upstream/main, 2026-09-13

Ветка обновлена слиянием. Публичный сценарий заменён на `unica.apply` и
`unica.view` по `CTR.WIRE.TOOL-SURFACE`; прежние публичные инструменты
метаданных не возвращаются. Решение этого PR ещё не попадало в `main`,
поэтому его текст и свидетельство обновлены в том же изменении.

## Проверка

Канонический invocation-сценарий проверяет запись `true` и `false`, XML с
единственным элементом свойства, чтение через `view`, preview без изменения
файла, результата чтения и ревизии, а также отказы в preview и apply для
строки вместо boolean и для `Document`. Применение использует fence из preview.
Доменная матрица закрепляет допустимый вид для всех поддержанных метаданных.
Контрактные проверки сохраняют boolean-тип и каноническую поверхность.

До восстановления поддержки выполнен тот же invocation-тест с файлами
реестра и парсера из `upstream/main` (`16c9f605`): `props.set` вернул
`bad_value`, адрес `ops[0].args.values.ClientOrdinaryApplication`, сообщение
«unknown metadata property `ClientOrdinaryApplication`». Начальное чтение
при этом успешно вернуло `false`.
