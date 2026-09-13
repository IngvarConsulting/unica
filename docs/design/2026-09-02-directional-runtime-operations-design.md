- Date: `2026-09-02`
- Status: `approved`
- Decision: `DEC.2026-09-02.DIRECTIONAL-RUNTIME-OPERATIONS`

# Направленные runtime-операции v0.13

## Проблема

Словарь из PR #657 смешал разные источники правды. `artifact.make` означал
сборку из исходников, но не экспорт текущей ИБ; `artifact.load` одновременно
маршрутизировал CF и DT, хотя первое меняет конфигурацию, а второе заменяет всю
ИБ. `syntax.check`, `test.run` и `extension.sync` дополнительно создавали
неясную границу с `unica.check`, будущим test tool и source-set workflows.

## Выбранная поверхность

| Намерение | Направление |
| --- | --- |
| `workspace.initialize` | discovery/declarations → `v8project.yaml` |
| `source.create` | запрос → новый source set |
| `infobase.create` | target → пустая ИБ |
| `infobase.build` | source sets → ИБ |
| `source.dump` | ИБ → source set |
| `source.convert` | Designer ↔ EDT |
| `artifact.build` | source sets → CF/CFE/EPF/ERF |
| `infobase.configuration.export` | working/database configuration или extension → CF/CFE |
| `infobase.configuration.load` | CF/CFE → ИБ |
| `infobase.dump` | вся ИБ → DT |
| `infobase.restore` | DT → новая или заменяемая ИБ |
| `client.run` | ИБ → интерактивная client session |

`workspace.initialize` сначала сохраняет уже реализованную source-only ветвь
`source.attach`; infobase-only и combined declarations добавляются отдельными
вертикалями без второго владельца `v8project.yaml`.

## Discovery и исполнение

`run {}` показывает реализованные и запланированные намерения. Планируемое
намерение никогда не становится `view.next`. Preview не запускает platform и
не пишет файлы; apply повторяет discovery, сверяет `ifRev` и только затем
публикует результат.

Обычный вызов не принимает выбор engine. Runner получает semantic intent и сам
возвращает выбранный provider и причины отклонения кандидатов. Fallback после
первого внешнего эффекта запрещён.

## Инициализационные маршруты

- CF в пустом workspace: `workspace.initialize` → `infobase.create` →
  `infobase.configuration.load`; `source.dump` только если нужны исходники.
- DT в пустом workspace: `workspace.initialize` → `infobase.restore` с create
  target semantics.
- Существующая ИБ: `workspace.initialize` с infobase declaration →
  `source.dump`, `infobase.configuration.export` или `infobase.dump` по цели.
- Исходники: `workspace.initialize` → `infobase.create` → `infobase.build`.

Source format и состав source sets не умножают тесты CF/DT: эти операции читают
состояние ИБ. Extension влияет на CFE subject, а EPF/ERF остаются только в
source artifact workflows.
