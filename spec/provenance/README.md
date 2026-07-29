# Происхождение skills

Нормативный источник по апстримам Unica. `skill-upstreams.json` связывает
packaged skills и runtime-контракты с донорскими репозиториями: какие пути
донора отслеживаются, какой коммит принят как baseline последней адаптации,
какое решение принято по каждой записи (`ported`, `ignored-with-reason`,
`blocked-by-product-contract`, `needs-tool-update`) и какая запись
`third-party/tools.lock.json` держит версию бинарника (`toolLockRef`).

Индекс — сопровожденческие метаданные исходного дерева. В поставку он не
входит: у потребителя пакета нет ни донорских репозиториев, ни процедур
сверки, а лицензионная обязанность закрывается уведомлением
`plugins/unica/ATTRIBUTIONS.md`.

## Отношение к `ATTRIBUTIONS.md`

- `plugins/unica/ATTRIBUTIONS.md` — самодостаточное уведомление в поставке:
  каждый апстрим назван поимённо, со ссылками на репозиторий, автора,
  проверенный baseline и лицензию. Читать его можно без этого каталога.
- `spec/provenance/skill-upstreams.json` — инвентарь для проверки полноты
  уведомления. `scripts/ci/check-attributions.py` требует раздел
  `<!-- unica-attribution: upstream <id> -->` на каждую запись `upstreams[]`
  и падает как на пропущенном, так и на лишнем разделе.
- Проверка выполняется по исходному дереву (`--repo-root`), в рабочем
  окружении потребителя не запускается никогда.
- Версии, теги, коммиты и лицензии поставляемых бинарников остаются за
  `plugins/unica/third-party/tools.lock.json`; здесь они не дублируются.

Каждый packaged skill из `plugins/unica/skills/` обязан иметь хотя бы одну
запись `entries[].skill`. Skill может встречаться несколько раз, когда разные
доноры дали разные части поведения: например, один отслеживает паритет
операций, другой — руководство.

Запись может нести собственный `baselineCommit`, чтобы закрыть дрейф по
отдельному skill, не двигая baseline всего донорского репозитория.

## Сборочные процедуры

1. Офлайн-валидация: форма JSON, покрытие путей, покрытие skills и
   согласованность `toolLockRef`.

   ```sh
   python3.12 scripts/ci/check-skill-upstreams.py --validate-only
   ```

2. Дрейф донора относительно принятого baseline адаптации.

   ```sh
   python3.12 scripts/ci/check-skill-upstreams.py --check
   ```

   `baselineCommit` указывает на коммит донора, отвечающий последней локальной
   адаптации, а не на текущий head. Для поставляемых инструментов baseline
   берётся из `plugins/unica/third-party/tools.lock.json`.

3. Пакет ревью, когда нужен JSON-артефакт.

   ```sh
   python3.12 scripts/ci/check-skill-upstreams.py --prepare-upstream-review --format json
   ```

   `--format json` даёт поskill-овый отчёт `entries[]` с флагами
   `upstreamDrift`. Хеши файлов отчёт сознательно не хранит: полезны диапазон
   коммитов донора, изменённые отслеживаемые пути, затронутые skills и решение
   сопровождающего.

4. Проверить диффы донора и решить, что переносить.
5. Адаптировать принятое к публичному MCP-контракту Unica (`unica.*`).
6. Обновить `baselineCommit` записи для проверенного skill; `baselineCommit`
   апстрима — только когда донор догнан целиком; для донора-инструмента —
   `plugins/unica/third-party/tools.lock.json`.

После обновления поставляемых инструментов прогнать контрактный smoke по
собранным или локально установленным нативным бинарникам:

```sh
python3.12 scripts/ci/check-tool-contracts.py --target darwin-arm64 --tools-dir plugins/unica/bin/darwin-arm64
```

Unica проверяет исполняемый жизненный цикл `rlm-bsl-index` (`build`, `update`,
`info`), а чтения анализа кода выполняет только через опубликованный MCP API
`rlm-tools-bsl`. Проверка не принимает путь к базе и не зависит от частной
SQLite-схемы поставщика (INV-CACHE-ORCHESTRATOR-OWNED).

Для записей `runtime-tool-contract` индекс отслеживает skill и MCP-контракт,
выведенные из репозитория runtime-инструмента. Версия бинарника здесь не
дублируется — она читается из `tools.lock.json` через `toolLockRef`.

Датированные записи ревью — исторические свидетельства, а не источник истины;
они лежат в [`docs/provenance/reviews/`](../../docs/provenance/reviews/).
