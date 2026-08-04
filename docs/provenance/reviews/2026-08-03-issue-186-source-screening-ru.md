# Компактный русский screening источников по issue #186

- Дата среза: `2026-08-03`
- Область: `screening only`; внешние материалы не переносились
- Статус проверки: решения сверены с полным обзором; перед использованием выводов обязательна отдельная перекрёстная проверка сильной моделью

Полный корпус доказательств — английский [evidence ledger](2026-08-03-issue-186-source-screening.md):
там зафиксированы SHA-срезы, лицензии, первичные ссылки, ограничения и
сопоставление с кодом Unica. Этот документ служит только русской навигацией и
синтезом. При любом расхождении действует английский evidence ledger.

## Сводка 19 источников

`deep-dive` означает кандидат на ограниченное исследование, а не рекомендацию
к переносу или продуктовому изменению. Утверждения ниже не выходят за границы
доказанного в английских карточках.

| Источник | Доказанный механизм | Значение для Unica | Решение |
| --- | --- | --- | --- |
| `SteelMorgan/1c-agent-based-dev-framework` | Набор правил и скиллов, выборочная установка и тесты оценки бюджета контекста | Процесс в основном дублирует локальные правила; реализованный протокол сжатия не доказан | `reject` |
| `comol/ai_rules_1c` | Адаптеры для разных хостов и структурная валидация правил | Уже учтён как источник идей без права переноса; нового runtime-механизма нет | `reject` |
| `AndreevED/1c-ai-feature-dev-workflow` | Фазы разработки по артефактам и ролевые prompts | Проза дублирует локальные gates планирования, ревью и тестирования | `reject` |
| `rmartynenko/workflow-dev-1c-claude-code` | Ручной протокол сессий и memory bank | Читаемая человеком непрерывность контекста правдоподобна, но не проверена на инвалидацию и конкуренцию | `defer` |
| `Pradushkoai/1c-ai-dev-env` | Тестированный fallback BM25/vector и типизированный каталог инструментов | Дополняет, но дублирует текущие движки и уступает более сильным кандидатам общего harness | `defer` |
| `Arman-Kudaibergenov/1c-ai-development-kit` | Большой каталог хост-специфичных скиллов и команд к живой базе | Внешний MCP-процесс конфликтует с публичной границей и дублирует локальные скиллы | `reject` |
| `Menestre1/reasoning-bank-poc` | Тестированное SQLite-хранилище опыта, продвижение по feedback и изоляция | В Unica нет доказанной политики долговременной reasoning-memory; перенос не разрешён лицензией | `deep-dive` |
| `vgtitov/bsl-ai-toolkit` | Тесты фильтрации слоёв и сохранения лексического XML-стиля | Узкая проверка полезна, но полный platform round trip и широта safety не доказаны | `defer` |
| `Regsorm/code-index-mcp` | Инкрементальный SQLite-индекс, состояние daemon и ограниченные result envelopes | Самый сильный кандидат для общего harness полноты, состояния и обновлений провайдера | `deep-dive` |
| `Arman-Kudaibergenov/bsl-atlas` | Структурный граф и необязательный vector index | Графовые семантики дополняют Unica, но AGPL/коммерческая модель и стоимость сервисов пока доминируют | `defer` |
| `feenlace/mcp-1c` | Read-only gate и generation-aware поисковый cache | Сильное сравнение внутренних cache/safety-семантик; поведение на живой базе не доказано | `deep-dive` |
| `DitriXNew/EDT-MCP` | Группировка tools, состояние отмены и headless EDT CI | EDT/AGPL и дублирование диагностики откладывают сравнение | `defer` |
| `Desko77/1c-formsserver` | Схема форм, конвертация, валидация и fixture round trips | Общие fixtures могут проверить fidelity нативных операций с формами | `deep-dive` |
| `alexiosus/mxl-merge-tool` | Семантический трёхсторонний merge MXL и тестированный Git driver | У Unica есть MXL writers, но нет доказанного контракта семантического merge | `deep-dive` |
| `rzateev/onec-help-mcp` | Реализованные HBK parser и hybrid search | Нет репрезентативного HBK/search-теста; маркировка лицензии противоречива | `defer` |
| `mussolene/1c_hbk_bsl` | Тестированные компоненты diagnostics, SARIF, formatter, LSP/MCP и indexing | В основном дополняет встроенный analyzer; полная совместимость протоколов не доказана | `defer` |
| `genlab-1c/prism` | Тестированное разделение prompts, классификация runner и L1 scoring | Исполняемая форма оценки заслуживает bounded-сравнения на fixtures Unica | `deep-dive` |
| `comol/1CLLMBenchTasks` | Семнадцать ручных карточек задач и ответов | Даёт гипотезу широты, но без лицензии, runner и детерминированного oracle | `defer` |
| `alonehobo/1c-trusted-gateway` | Тестированные masking/type policy и наблюдаемые обходы approval | Нужен adversarial safety-тест; отсутствие лицензии запрещает перенос | `deep-dive` |

## Кандидаты `deep-dive`

### `Menestre1/reasoning-bank-poc`

- **Что доказано:** SQLite-записи опыта, hash retrieval, confidence/usage promotion, очистка, дедупликация и доменная изоляция проверены тестами при отключённом HNSW.
- **Что проверить в Unica:** улучшает ли долговременный опыт повторные задачи, не протекая между workspace identity и не сохраняя устаревшие выводы.
- **Минимальный bounded experiment:** на фиксированном наборе повторных сценариев измерить качество retrieval до и после feedback; отдельно проверить fingerprint isolation, promotion, invalidation, cleanup и concurrent access. Сравниваются только policy и envelopes, без копирования storage-кода и без нового публичного tool.

### `Regsorm/code-index-mcp`

- **Что доказано:** структурный/FTS SQLite-индекс использует hashes для add/change/skip/delete, отдельного daemon writer, readiness states, federation paths и ограниченные query envelopes.
- **Что проверить в Unica:** даёт ли провайдер полные, явно ограниченные и workspace-scoped результаты при неоднозначных символах, инкрементальных изменениях и отказах по сравнению с bundled engines.
- **Минимальный bounded experiment:** прогнать один общий fixture через текущие `bsl-analyzer`/RLM и адаптер кандидата; собрать exact/ambiguous completeness, truncation и lower bounds, add/change/delete, rename invalidation, multi-root/extensions, cold/warm latency и размер, cancellation/concurrent readers, stale/corrupt/unavailable outcomes. Только адаптер за существующими `unica.code.*`.

### `feenlace/mcp-1c`

- **Что доказано:** тесты подтверждают read-only prefix gate, BM25 ranking, generation-aware reload, восстановление stale/corrupt cache и concurrent search во время обновления.
- **Что проверить в Unica:** закрываются ли фактические execution boundaries при read-only/preview, а cache остаётся корректным при смене generation и отказе; live-base семантика пока не считается доказанной.
- **Минимальный bounded experiment:** на контролируемой fake-base/fixture проверить разрешённые и запрещённые запросы, явные preview outcomes, generation reload, content revalidation, stale/corrupt recovery и одновременные чтения. Семантики остаются за существующими `unica.*`, без второго MCP server.

### `Desko77/1c-formsserver`

- **Что доказано:** реализованы Pydantic schema/parser, generation/conversion и validation; fixtures покрывают managed/logform conversion и ошибки validator, но не полную fidelity всех моделей.
- **Что проверить в Unica:** какие структурные и лексические свойства общих managed/logform fixtures сохраняют нативные form operations и где расходятся validation results.
- **Минимальный bounded experiment:** выполнить parse/edit/generate/round-trip на одном общем наборе repository-owned или официальных fixtures, сравнить bytes и структуру, проверить platform load, ожидаемые validation failures и atomic failure. Политику форматов не менять.

### `alexiosus/mxl-merge-tool`

- **Что доказано:** fixtures и тесты покрывают semantic diff/three-way merge, conflict output, Git merge-driver report, atomic и parseable output.
- **Что проверить в Unica:** совместимы ли conflict semantics с JSON DSL и writer Unica и сохраняется ли платформенная семантика MXL.
- **Минимальный bounded experiment:** на bounded-наборе base/ours/theirs fixtures сравнить clean merge и конфликты, parseability, platform load и atomic publication; применение результата остаётся preview-first и support-guarded.

### `genlab-1c/prism`

- **Что доказано:** inputs генерации отделены от canonical/hidden fixtures; тесты покрывают загрузку, категории, syntax/platform scoring и классификацию runner. Реальный запуск платформы в pinned CI не доказан.
- **Что проверить в Unica:** можно ли получить leakage-resistant, model-independent oracle с устойчивой классификацией для BSL и артефактов на фактическом запуске 1С.
- **Минимальный bounded experiment:** отделить prompt от hidden fixtures, запустить одинаковые BSL и representative XML/form/MXL/DCS/role/integration cases, сохранить reproducible runner artifacts и сравнить deterministic classifications. Не объявлять итоговый benchmark.

### `alonehobo/1c-trusted-gateway`

- **Что доказано:** unit tests покрывают masking/type policies, но timeout выдаёт тот же результат, что approval, bridge auto-mode обходит ручной gate, а raw MCP responses сохраняются в очищаемом in-memory log.
- **Что проверить в Unica:** fail-closed ли approval, redaction, cancellation и partial-failure на реальной границе исполнения, включая whitelist/bridge bypass и log handling.
- **Минимальный bounded experiment:** adversarial proxy/bridge matrix с явными approval events, timeout/deny/cancel, секретами на границах chunks, обходами auto-mode и injected partial failures; проверить отсутствие записи и утечки при каждом отказе. Внешний код и production data не переносить.

## Пять направлений исследования

Порядок ниже задаёт только последовательность проверки гипотез. Он не выбирает
продуктовые эксперименты, новые инструменты или follow-up issues.

### 1. Общая экспериментальная рамка: workflow/context

- **Кандидаты:** Menestre (`deep-dive`), rmartynenko (`defer`).
- **Главный вопрос:** улучшает ли долговременное состояние непрерывность в измеримом повторном сценарии без утечки между workspace identities и без stale context?
- **Граница:** фиксированные задачи и метрики retrieval/повторного успеха плюс deterministic tests isolation, promotion, invalidation, cleanup и concurrency; только policy/envelopes, без нового публичного tool и без переноса storage-кода.

### 2. Code intelligence

- **Кандидаты:** Regsorm и feenlace (`deep-dive`); Pradushko, bsl-atlas и mussolene (`defer`).
- **Главный вопрос:** какие provider semantics сохраняют полные, явно ограниченные, workspace-scoped результаты при incremental change и failure?
- **Граница:** один общий harness против bundled engines и кандидатов на одинаковых fixtures; адаптеры только за `unica.code.*`, без второго MCP server и без performance-утверждений из screening.

### 3. Live data/safety

- **Кандидаты:** alonehobo gateway и feenlace (`deep-dive`), EDT (`defer`).
- **Главный вопрос:** fail-closed ли approval, read-only, redaction, cancellation и partial-failure на фактической execution boundary?
- **Граница:** adversarial proxy/bridge tests, controlled fake-base или live-base traces без production data, явные approval events и atomicity checks; только red-team semantics, без переноса внешнего кода.

### 4. Artifacts/documentation

- **Кандидаты:** formsserver и mxl-merge-tool (`deep-dive`); bsl-ai-toolkit и onec-help-mcp (`defer`).
- **Главный вопрос:** какие lexical и semantic properties сохраняются при parse/edit/merge/round-trip репрезентативных platform artifacts?
- **Граница:** общие официальные или repository-owned fixtures, byte comparison, platform load, conflict oracles и failure atomicity против существующих native operations; без изменения format policy и без копирования реализаций.

### 5. Benchmark/evaluation

- **Кандидаты:** PRISM (`deep-dive`), 1CLLMBenchTasks (`defer`).
- **Главный вопрос:** может ли leakage-resistant и model-independent oracle охватить BSL и artifact contracts Unica на реальном запуске 1С?
- **Граница:** изолированные prompt/hidden fixtures, воспроизводимые runner artifacts, deterministic classification и representative XML/forms/MXL/DCS/role/integration cases; оценивается только evidence harness, итоговый benchmark не выбирается.

## Граница следующего шага

Screening не ранжирует кандидатов по README-метрикам и не утверждает
сравнительных чисел качества, задержки или размера. До любого решения о переносе
нужны первичные результаты ограниченных экспериментов выше и отдельная проверка
синхронности с английским evidence ledger моделью `gpt-5.6-sol`.
