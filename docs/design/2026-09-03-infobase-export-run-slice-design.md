- Date: `2026-09-03`
- Status: `approved`
- Decision: `DEC.2026-09-03.INFOBASE-EXPORT-RUN-SLICE`

# Выгрузка CF/CFE и DT через unica.run

## Цель

Дать модели две первые полезные runtime-операции без нового MCP tool и без
знания CLI платформы: выгрузить working/database конфигурацию или extension в
CF/CFE и выгрузить полную ИБ в DT. Существующая ИБ может быть единственным
входом workspace; XML-исходники для этих операций не требуются.

## Публичный контракт

- `infobase.configuration.export`: обязательные `state=working|database` и
  workspace-relative `output`; необязательное имя `extension`. Без extension
  output заканчивается `.cf`, с extension — `.cfe`.
- `infobase.dump`: только workspace-relative `.dt` `output`.
- Оба args object закрыты. Аргументов engine/provider, raw CLI, timeout и
  credentials нет.
- `dryRun=true` обязателен первым. Он возвращает provider plan, кандидатов,
  artifact kind, revision и точный следующий apply.
- `dryRun=false` требует `ifRev`; операция исполняется как Task и возвращает
  path, size, SHA-256 и target state.

## Граница Unica и runner

Unica проверяет публичные аргументы, workspace containment, suffix, bundled
runner identity, preview/apply fence и финальный файл. v8-runner загружает
`v8project.yaml` с локальным overlay, валидирует подключение к ИБ, выбирает
ibcmd или Designer, применяет fallback до внешнего эффекта, владеет platform
timeout и безопасно публикует output.

Preview — реальный resolver runner с `--dry-run`: provider не запускается,
workPath и output не создаются. Apply повторяет preview непосредственно перед
запуском; изменение config, local config, output, версии runner или provider
делает revision устаревшей.

## Начальное состояние

`unica.view {}` различает пустой workspace, source workspace и infobase-only
workspace. Для последнего отсутствие source roots не является проблемой:
ответ показывает настроенную ИБ и предлагает только два точных preview-вызова.
Если config не содержит ни source set, ни infobase connection, основной
диагноз один и setup предлагает добавить только выбранный вход.

## Проверки

- unit: закрытые args, CF/CFE/DT suffix, exact runner argv, preview без записи,
  stale revision, повторный preflight, несовпавшая apply-квитанция;
- daemon: обе операции допускаются как Task без Platform XML source set;
- bootstrap: infobase-only workspace не получает source-root diagnostic;
- runner: Linux, Windows и macOS CI для v0.7.1 и опубликованные native assets;
- live platform: отдельный release gate на host с полной платформой; thin-only
  или отсутствующая платформа должны дать один `provider_unavailable`, а не
  конфликтующие рекомендации.
