# Приёмка логической адресации и ресурсного доступа

Этот план связывает ADR-0021 и ADR-0022 с воспроизводимыми проверками. Нормой
владеют записи решений и правила `INV-SOURCE-LOGICAL-IDENTITY`,
`INV-SOURCE-HANDLE-REAUTH`, `INV-SOURCE-SNAPSHOT-BINDING`,
`INV-SOURCE-ROLE-ALLOWLIST`, `INV-SOURCE-PLAN-EVENT-PARITY`,
`INV-MCP-SOURCE-SURFACE`, `INV-SKILL-SOURCE-FALLBACK` и
`REQ-PERF-SOURCE-BOUNDS`; этот документ только показывает, где наблюдается их
выполнение.

## Матрица адресов и навигации

| Случай | Ожидаемое доказательство | Проверка |
| --- | --- | --- |
| Английский и русский вид метаданных | Английский и русский псевдонимы дают один канонический английский `metadataPath`, прикладное имя сохраняется | `cargo test -p unica-coder source_target -- --test-threads=1` |
| Configuration и Extension | Одинаковый логический адрес модуля разрешается в каждом явно выбранном `sourceSet`, физический путь не входит в результат | `cargo test -p unica-coder platform_xml_source_targets -- --test-threads=1` |
| ExternalProcessor и ExternalReport | В одном внешнем наборе обнаруживаются как минимум два независимых корневых артефакта без эвристического выбора одного | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Точное и префиксное разрешение | `exact` не выбирает неоднозначного кандидата, `prefix` возвращает ограниченную каноническую выдачу | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Непосредственные дети | `children` принимает цель, обходит один уровень, а коллекции возвращаются как узлы без вымышленного адреса | `cargo test -p unica-coder source_navigation -- --test-threads=1` |
| Снятые селекторы `unica.code.patch` | `path` и `sourceDir` отклоняются кодом `legacy_target_removed` с подсказкой `sourceSet + metadataPath` | `cargo test -p unica-coder code_patch -- --test-threads=1` |
| Частные поля поставщика | `providerRevision`, закрытая ручка, абсолютный путь и строка соединения отсутствуют в MCP-ответе | `cargo test -p unica-coder interfaces::mcp::tests -- --test-threads=1` |

## Матрица снимка, чтения и записи

| Случай | Ожидаемое доказательство | Проверка |
| --- | --- | --- |
| Поддельная пара идентификаторов | `resourceId` из другого снимка отклоняется, а непрозрачные значения не раскрывают внутреннее состояние | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Истёкший снимок | После пяти минут чтение и запись получают `snapshot_expired` | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Неполный манифест | `partial` и `unavailable` остаются читаемыми для выданного ресурса, но не дают возможности `replace` | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Дескриптор | Роль `metadataDescriptor` с текстовым содержимым остаётся read-only и при записи получает `resource_not_replaceable` | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Точный диапазон | `unica.source.read` возвращает ровно запрошенный диапазон байтов до 64 КиБ как UTF-8 либо base64, сохраняя общий хеш снимка | `cargo test -p unica-coder source_resources -- --test-threads=1` |
| Профиль текста | `bomPrefixBytes`, единый LF/CRLF и побайтовый preimage относятся к тем же байтам, что читаются и заменяются; смешанный EOL запрещает замену | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Подмена символической ссылки | Смена каталога на symlink между снимком, планом и публикацией получает `containment_denied` без изменения соседних файлов | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Смена owner или карты исходников | Повторная авторизация закрытой ручки отклоняет изменившегося логического owner и не публикует план | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Предпросмотр | `dryRun: true` возвращает diff, хеш и проектируемое влияние на кеш, но не сохраняет событие и не меняет байты | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Применение | Успешная непустая замена публикует байты предпросмотра и ровно одно событие `SourceResourcesReplaced`; no-op, отказ и отмена не публикуют событие | `cargo test -p unica-coder source_apply -- --test-threads=1` |
| Влияние на кеш | Одно событие инвалидирует только `bsl_index` и `bsl_diagnostics` | `cargo test -p unica-coder domain::cache::tests -- --test-threads=1` |

## Публичный MCP и скилл

- `crates/unica-coder/src/application/tool_contracts.rs` проверяет схемы пяти
  `unica.source.*`, пределы и отсутствие файловых селекторов.
- `crates/unica-coder/src/interfaces/mcp.rs` проверяет полный цикл в одном
  экземпляре приложения и вымарывание частных полей.
- `tests/ci/test_unica_skills.py` проверяет порядок маршрута: предметный writer,
  затем `resources/read`, затем обоснованный fallback через preview
  `source.apply`.
- `tests/ci/test_skill_provenance.py` проверяет, что `source-access` помечен как
  собственный скилл Unica, а не приписан донорскому проекту.

## Команды приёмки

```sh
cargo test -p unica-coder source_target -- --test-threads=1
cargo test -p unica-coder source_navigation -- --test-threads=1
cargo test -p unica-coder source_resources -- --test-threads=1
cargo test -p unica-coder source_apply -- --test-threads=1
cargo test -p unica-coder domain::cache::tests -- --test-threads=1
python3.12 -m unittest tests/ci/test_architecture_registry.py tests/ci/test_design_documents.py tests/ci/test_unica_skills.py tests/ci/test_skill_provenance.py
```
