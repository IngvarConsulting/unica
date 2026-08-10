# Приёмка логической адресации и ресурсного доступа

Этот план связывает ADR-0021 и ADR-0022 с воспроизводимыми проверками. Нормой
владеют записи решений и правила `INV-SOURCE-LOGICAL-IDENTITY`,
`INV-SOURCE-WRITE-TARGET-KIND`, `INV-SOURCE-SNAPSHOT-BINDING`,
`INV-SOURCE-ROLE-ALLOWLIST`, `INV-MCP-SOURCE-SURFACE`,
`INV-SKILL-SOURCE-FALLBACK` и `REQ-PERF-SOURCE-BOUNDS`; этот документ только
показывает, где наблюдается их выполнение.

## Матрица адресов и навигации

| Случай | Ожидаемое доказательство | Проверка |
| --- | --- | --- |
| Английский и русский вид метаданных | Английский и русский псевдонимы дают один канонический английский `metadataPath`, прикладное имя сохраняется | `cargo test -p unica-coder source_target -- --test-threads=1` |
| Configuration и Extension | Одинаковый логический адрес модуля разрешается в каждом явно выбранном `sourceSet`, физический путь не входит в результат | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |
| ExternalProcessor и ExternalReport | В одном внешнем наборе обнаруживаются как минимум два независимых корневых артефакта без эвристического выбора одного | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Точное и префиксное разрешение | `exact` не выбирает неоднозначного кандидата, `prefix` возвращает ограниченную каноническую выдачу | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Вид цели по числу сегментов | Вид определяется арностью адреса, поэтому объект с прикладным именем `Module` остаётся `metadataObject`, а не читается как роль модуля | `cargo test -p unica-coder source_target -- --test-threads=1` |
| Частичный корень префикса | Неполный сегмент сопоставляется с каноническим английским токеном, а неполный псевдоним отклоняется отдельной ошибкой вместо непредсказуемого частичного совпадения | `cargo test -p unica-coder source_target -- --test-threads=1` |
| Непосредственные дети | `children` принимает цель, обходит один уровень, а коллекции возвращаются как узлы без вымышленного адреса | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Ответ не зависит от размера набора | `exact` и `children` отвечают, отрисовывая кандидата по адресу, а не перечисляя набор исходников; префикс сканирует только коллекции, закреплённые ведущими сегментами, и фильтрует по имени файла до чтения дескриптора | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |
| Путь → адрес | `unica.source.locate` восстанавливает логический адрес и владельца по пути в workspace- или source-set-относительной форме; отказ типизирован `outsideSourceSet`, `notAddressable` или `ownerUnproven` | `cargo test -p unica-coder locate_recovers -- --test-threads=1` |
| Дескриптор вложенной формы | Дескриптор `Form`/`Command` читается как `<Forms>/<Имя>.xml`, а не как содержимое `<Имя>/Ext/Form.xml`, поэтому модули форм адресуемы в реальной выгрузке | `cargo test -p unica-coder nested_child_descriptor -- --test-threads=1` |
| Усечение отличимо от отсутствия | Недоказуемый кандидат даёт `partial` либо отдельную ошибку, а отсутствующий — `complete` либо `was not found`; перечисление, до которого не дошёл обход, не выдаётся за отсутствие | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |
| Замена в `unica.code.patch` | `operation: replace` перезаписывает выбранный метод целиком либо ровно вхождение якоря, сохраняя EOL источника и не трогая соседние методы; `position` для замены не принимается | `cargo test -p unica-coder code_patch_replace -- --test-threads=1` |
| Повтор замены | Повтор идентичной замены либо пуст, либо отказывает без записи, если селектор поглощён самой заменой | `cargo test -p unica-coder code_patch_replace -- --test-threads=1` |
| Снятые селекторы `unica.code.patch` | `path` и `sourceDir` отклоняются кодом `legacy_target_removed` с подсказкой `sourceSet + metadataPath` | `cargo test -p unica-coder code_patch -- --test-threads=1` |
| Объект метаданных как цель | Двухсегментный адрес и вложенный `Form`/`Command` разрешаются в свой дескриптор с `targetKind: metadataObject`; дескриптор с чужим именем, отсутствующий дескриптор и связанный файл отклоняются раздельно | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |
| Расширение резолвера не расширяет запись | `unica.code.patch` отклоняет адрес объекта метаданных и ничего не пишет, потому что писатель разрешает цель под политикой «только модуль» | `cargo test -p unica-coder code_patch -- --test-threads=1` |
| Снятые селекторы `unica.meta.info` | `ObjectPath` и `Path` отклоняются кодом `legacy_target_removed`; `Detailed`, который инструмент никогда не читал, отклоняется как незнакомый аргумент | `cargo test -p unica-coder meta_info -- --test-threads=1` |
| Логический адрес в предметном читателе | `unica.meta.info` читает дескриптор по `sourceSet + metadataPath`, принимает русский псевдоним вида, отклоняет терминал модуля по имени и возвращает разрешённую цель типизированными данными | `cargo test -p unica-coder meta_info -- --test-threads=1` |
| Полный локальный read-профиль объекта | Ответ связывает каждый из 23 `kind` с обязательным вариантом `details`, не выводит свойства чтения из writer allowlist, сохраняет наблюдаемый UUID как `{"kind":"uuid"}` с `mutationCapability: editable`, а вложенные HTTP/WebService-коллекции различают `[]` и недоказанное `null` с диагностикой | `cargo test -p unica-coder meta_info -- --test-threads=1` |
| Частные поля поставщика | `providerRevision`, закрытая ручка, абсолютный путь и строка соединения отсутствуют в MCP-ответе | `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1` |

## Матрица снимка и чтения

| Случай | Ожидаемое доказательство | Проверка |
| --- | --- | --- |
| Поддельная пара идентификаторов | `resourceId` из другого снимка отклоняется, а непрозрачные значения не раскрывают внутреннее состояние | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Истёкший снимок | После пяти минут чтение и запись получают `snapshot_expired` | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Точный диапазон | `unica.source.read` возвращает диапазон до 64 КиБ, усекая фрагмент текстового ресурса до ближайшей границы UTF-8; base64 остаётся везде, где фрагмент не является корректным UTF-8, в том числе для двоичных данных, лимита уже одного символа и `offset` внутри символа; чтение всегда продвигается, а общий хеш снимка не меняется | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Манифест объекта метаданных | Область `self` даёт один `metadataDescriptor` и полноту `complete`, `aggregate` добавляет только доказанные модули объекта и остаётся `partial`, доступ у обоих — только `read` | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Отказ отличим от недоступности | Недоказуемый адрес и неизвестный набор дают `target_not_found`, пустое имя набора — `invalid_request`, а `source_unavailable` остаётся за настоящей недоступностью источника; сообщение не содержит физического пути | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Влияние на кеш | Одно событие инвалидирует только `bsl_index` и `bsl_diagnostics` | `cargo test -p unica-coder domain::cache::tests -- --test-threads=1` |

## Публичный MCP и скилл

- `crates/unica-coder/src/application/tool_contracts.rs` проверяет схемы
  читающих `unica.source.*`, пределы и отсутствие файловых селекторов.
- `crates/unica-coder/src/interfaces/mcp.rs` проверяет полный цикл в одном
  экземпляре приложения и вымарывание частных полей.
- `tests/ci/test_unica_skills.py` проверяет порядок маршрута: предметный writer,
  затем `resources/read` для исследования, затем правка через
  `unica.code.patch` с предпросмотром.
- `tests/ci/test_skill_provenance.py` проверяет, что `source-access` помечен как
  собственный скилл Unica, а не приписан донорскому проекту.

## Команды приёмки

```sh
cargo test -p unica-coder source_target -- --test-threads=1
cargo test -p unica-coder source_navigation -- --test-threads=1
cargo test -p unica-coder source_resources -- --test-threads=1
cargo test -p unica-coder code_patch_replace -- --test-threads=1
cargo test -p unica-coder meta_info -- --test-threads=1
cargo test -p unica-coder domain::cache::tests -- --test-threads=1
python3.12 -m unittest tests/ci/test_architecture_registry.py tests/ci/test_design_documents.py tests/ci/test_unica_skills.py tests/ci/test_skill_provenance.py
```
