- Date: `2026-08-10`
- Status: `approved`
- Decision: `none` — no architectural contract changed

# Обновление v8-runner на снимок master

## Контекст

Unica `v0.11.0` и текущий `main` закрепляют один снимок `v8-runner`:
`72d346c0a8fcf8373d9388257d11e6bef0ad70b2`. Актуальный на момент решения
upstream `master` — `7ce1b062843d86644fe55741dbe0ee79f7ca767d`; он содержит 36
последующих коммитов, но не содержит исправления private per-IB
`ConfigDumpInfo.xml` из `alkoleft/v8-runner-rust#30`.

Готового артефакта `IngvarConsulting/unica-toolchain` для нового коммита нет.
Существующий `v8-runner-nightly-master-build.1` собран из старого снимка, поэтому
изменить только `sourceCommit` в Unica нельзя: package lock должен ссылаться на
реальные неизменяемые байты и их SHA-256.

## Решение

Обновление выполняется двумя последовательными доставками.

1. В `IngvarConsulting/unica-toolchain` снимок `v8-runner` закрепляется на
   `7ce1b062843d86644fe55741dbe0ee79f7ca767d`, `buildRevision` повышается с `1`
   до `2`, после проверки исходного PR публикуется новый неизменяемый release
   `v8-runner-nightly-master-build.2` для `darwin-arm64`, `linux-x64` и
   `win-x64` вместе с лицензией и provenance metadata.
2. Только после успешной публикации всех трёх бинарников Unica переключает
   `plugins/unica/third-party/tools.lock.json` на тот же `sourceCommit`, новый
   `assetTag` и SHA-256 фактически скачанных assets. Поле `version` остаётся
   `0.5.1`, потому что именно эту package version объявляет upstream
   `Cargo.toml`; идентичность nightly-снимка задают commit и asset tag.

Порядок не обращается: Unica не должна ссылаться на ещё не существующие или
непроверенные байты. Опубликованный release/tag не перемещается и не
перезаписывается; при ошибке выпуска используется следующий build revision.

## Граница изменения

В этот срез входят:

- сборка точного upstream commit на трёх целевых платформах;
- публикация нового immutable toolchain release;
- синхронизация commit, asset tag и SHA-256 в lock-файле Unica;
- актуализация справки о поддержанном `tools.platform.strict`, поскольку поле
  уже доступно через `v8project.yaml` без изменения MCP-схемы;
- обновление provenance review для 36 рассмотренных upstream-коммитов с явным
  разделением применимых возможностей и отложенных контрактов;
- package/tool-contract smoke для каждого целевого бинарника.

В этот срез не входят:

- удаление `source_sync_dump_guard` или разрешение applied
  `mode=incremental|partial`;
- private per-IB CDFI, exact sync receipts и divergence-safe shadow publication
  из открытого `v8-runner-rust#30`/PR `#39`;
- `source-set[].dependsOn` из открытого `v8-runner-rust#32`/PR `#50`;
- новый публичный аргумент `noBuild` в `unica.runtime.execute`;
- изменение идентичности MCP-сервера, набора инструментов `unica.*`, их
  аргументов или результата.

Поэтому новый ADR не нужен: package source of truth и неизменяемая доставка уже
нормированы ADR-0006/ADR-0008 и `INV-PRODUCT-TOOL-VERSION-SOURCE`; публичный
архитектурный контракт не меняется.

## Проверка toolchain

До публикации build revision 2 проверяются:

- manifest и source provenance указывают ровно на commit `7ce1b062...`;
- сборка использует закреплённый Rust toolchain и `cargo --locked`;
- каждый asset запускается на своей целевой ОС и проходит `--version` и
  `build --help`;
- release содержит три ожидаемых бинарника и license asset;
- локально вычисленный SHA-256 каждого скачанного release asset совпадает с
  опубликованными metadata.

Если любой target не собран или smoke не прошёл, release не считается готовым,
а lock Unica не меняется.

## Проверка Unica

После переключения lock выполняются:

- тесты schema/source/license контракта `tools.lock.json`;
- загрузка каждого release asset с проверкой SHA-256;
- `scripts/ci/check-tool-contracts.py` для трёх целевых платформ, включая
  существующие behavioral smoke partial-load list и bounded external EPF;
- тесты packaging/provenance/skills;
- генерация пакета и проверка, что новый `v8-runner` попадает в runtime для
  каждой целевой платформы;
- проверка, что applied incremental/partial dump по-прежнему fail-closed и
  ссылается на незавершённый контракт `v8-runner-rust#30`.

Изменение lock и документации является data/config-only обновлением. Новый тест,
жёстко закрепляющий конкретный commit или build revision в коде теста, не
добавляется: он дублировал бы source of truth и создал бы датированный snapshot.
Доказательство обновления дают существующие общие contract tests, реальные
release assets и behavioral smoke.

## Критерии готовности

Обновление готово, когда новый toolchain release публично доступен и проверен,
Unica закрепляет его точный commit и три подтверждённых SHA-256, все связанные
contract/package tests проходят, а source-sync guard и публичная MCP-поверхность
остаются без изменений.
