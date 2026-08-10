- Date: `2026-08-10`
- Status: `approved`
- Decision: `ADR-0045`

# Завершённость типизированных readers для diagnostics и RLM

## Результат исследования

Issue [#291](https://github.com/IngvarConsulting/unica/issues/291) полностью
воспроизводится на `origin/main` (`4c3e4733`). Default-ветка
`unica.code.diagnostics` выбирает `mode=analyze`, запускает отдельный
`bsl-analyzer analyze` и возвращает `BslAnalyzerOutcome::plain`. Без аргумента
`format` upstream печатает console report; с `format=json` Unica только
нормализует имя в `jsonl`, но не разбирает события. В обоих случаях успешный
ответ остаётся в `stdout`, а `data` отсутствует.

Issue [#292](https://github.com/IngvarConsulting/unica/issues/292) на текущей
ветке воспроизводится только для `unica.code.definition`.
`RlmNavigationAdapter::invoke_resolved_cancellable` по-прежнему превращает
любой неготовый `IndexReadiness` в `AdapterOutcome::ok` с warning и без
`data`. Тест
`definition_keeps_the_warning_only_contract_for_an_unready_index` закрепляет
это поведение как ожидаемое.

Вторая половина #292 устарела после PR #309: `unica.meta.profile` снят с
публичной поверхности, его данные перенесены в новую typed Meta surface, а
`unica.meta.info` больше не зависит от RLM. Возвращать `meta.profile` или
добавлять к `meta.info` индексную readiness-семантику ради буквального текста
issue было бы новой регрессией.

Ссылка #292 на #247 тоже больше не задаёт рабочий эталон. После ADR-0020
`unica.code.outline` строится из текущего BSL-файла и не читает RLM-индекс.
Коды readiness из #247 полезны как историческая лексика, но не как контракт
действующего outline.

## Причина

Оба оставшихся дефекта возникли не из-за сериализации JSON как таковой, а из-за
разрыва между объявленным и исполняемым контрактом:

1. `spec/architecture/tool-surface-review.json` объявляет оба инструмента
   типизированными, но live-реестр `ToolSpec` не несёт вид результата;
2. общий `call_tool` переносит любой `HandlerOutcome` в `OperationResult` и не
   проверяет импликацию «успешный typed-reader ⇒ есть `data`, нет `stdout`»;
3. поэтому отдельная CLI-ветка diagnostics и ранний выход RLM могут вернуть
   `AdapterOutcome::ok` с `data=None`, не нарушив ни тип Rust, ни общий тест;
4. существующие проверки доказывают декларацию ledger и отдельные happy paths,
   но не default-маршрут analyze и не матрицу readiness.

ADR-0023 уже запрещает такой успех. Переписывать принятую ADR задним числом
нельзя; новое решение ADR-0045 выбирает конкретные протоколы двух readers и
усиливает общий финализатор запретом текстового дубля.

Пока шло исследование, реализация #297 была отдельно влита PR #428 и принята
как ADR-0044. Она уже владеет `ToolExecution::Read | Mutation`, режимами `Read |
Preview | Apply`, удалением `dryRun` у readers, классификацией результата
`ResultContract::Typed | ExternalStream` и постусловием присутствия `data` у
успешного typed-read. ADR-0045 опирается на этот live-контракт, не заменяет его
четырёхзначной моделью и владеет только дополнительным transport-guard и
JSONL/RLM-протоколами #291/#292.

## Рассмотренные подходы

### Точечные ветвления в двух адаптерах

Можно разобрать JSONL в `internal_adapters.rs` и заменить `AdapterOutcome::ok`
на ошибку в `rlm_navigation.rs`. Это минимальный diff, но live-реестр всё ещё
не знает, что обещал typed-result, а следующий ранний выход снова сможет
нарушить ADR-0023. Подход исправляет наблюдаемые места, но сохраняет причину.

### Только общий runtime-guard

Можно пометить typed readers в `ToolSpec` и отвергать успех без `data`. Такой
guard сразу перестанет сообщать ложный успех, но превратит #291 в стабильный
отказ вместо полезного анализа и не определит retry-семантику #292. Это хорошая
последняя линия защиты, но не полный пользовательский контракт.

### Общий guard и два предметных протокола одним изменением

Выбран этот вариант с единым владельцем предметных протоколов. PR #425 опирается
на live-реестр и missing-data guard ADR-0044, усиливает финализатор, переводит
one-shot analyze через JSONL parser в устойчивую модель Unica и сохраняет RLM
readiness до публичной границы. Оба issue относятся к ложному успеху
typed-reader, но данные и отказ каждого поставщика не смешиваются.

## Общий live-контракт ADR-0044

Влитый PR #428 предоставляет в `ToolSpec` исполняемые
`ToolExecution::{Read, Mutation}` и `ResultContract::{Typed, ExternalStream}`.
CI сверяет их с `tool-surface-review.json`: значение ledger `typed` становится
`Typed`, а остальные способы выдачи остаются внешним потоком. ADR-0045 не
меняет эту классификацию.

После handler и до построения публичного `OperationResult`
общий application-finalizer проверяет каждый вызов
`ToolExecution::Read + ResultContract::Typed`:

- `ok=true` требует `data.is_some()`;
- `ok=true` требует отсутствующий `stdout`;
- отсутствие данных завершает `call_tool` транспортным `Err` с префиксом
  `typed_result_missing:`, а текстовый дубль при существующем `data` —
  транспортным `Err` с префиксом `typed_result_textual:`; если нарушены оба
  постусловия, первым сообщается `typed_result_missing:`; публичный
  `OperationResult` не строится и сфабрикованный `data` не создаётся;
- `ok=false` может нести типизированное `data` о состоянии отказа, но не
  обязано его синтезировать для ошибки запуска процесса, timeout или
  cancellation.

`dryRun` у readers уже удалён реализацией ADR-0044. Reader не имеет режима
preview, ручной `dryRun` отклоняется до workspace/provider/backend, а каждый
допустимый `Read + Typed` проходит единственный transport-guard, который эта
работа усиливает проверкой отсутствия `stdout`.

## Контракт `code.diagnostics mode=analyze`

### Команда и совместимость аргумента `format`

Вызов без `mode` и явный `mode=analyze` всегда добавляют внутренний
`--format jsonl`. Console report больше не является возможным успешным
ответом Unica.

Публичный `format` временно сохраняется как migration alias только для
`mode=analyze`:

- отсутствие, `json` и `jsonl` ведут к одному и тому же typed-result;
- `console` и неизвестные значения отклоняются до запуска;
- аргумент не выбирает публичную форму ответа и не передаёт наружу raw JSONL;
- `format` в остальных diagnostics modes отклоняется как неприменимый.

Так существующий вызов с `format=json` продолжает работать, но старый prose
нельзя вернуть случайным selector-ом.

### Потоковый runner и изолированный parser

Обычный `ManagedChild::wait_for_output` хранит только хвост stdout размером
`1 MiB` и помечает остальное усечённым. Он не может быть транспортом JSONL:
корректный большой workspace потеряет начало потока прежде, чем parser увидит
`start`, а `limit` результата этого не предотвращает.

Для analyze вводится `DiagnosticsJsonlRunner`. Он использует тот же
`ManagedChild`, process-tree cleanup, deadline и cancellation, но конкурентно
дренирует stdout построчно в автомат протокола и stderr — в существующий
ограниченный очищаемый хвост. Raw stdout целиком не накапливается и не попадает
в `ManagedOutput`. Одна физическая JSONL-строка ограничена `8 MiB`, тем же
пределом, что строка ответа workspace service; превышение даёт
`diagnostics_invalid:` после полного дренирования pipe. Parser хранит
счётчики, множество нормализованных путей и не более `limit` лучших элементов
в bounded top-K, а не все findings. Поэтому общий размер stdout может превышать
`1 MiB`, не меняя объём публичного ответа.

После первой ошибки протокола runner продолжает дренировать оба pipe, чтобы не
заблокировать child, но больше не публикует элементы. Приоритет исходов:
cancellation, timeout и ненулевой exit сохраняют собственную process-семантику;
только успешный exit классифицируется автоматом JSONL.

Разбор выносится из уже крупного `internal_adapters.rs` в отдельный
инфраструктурный модуль. Локальные serde-структуры используют
`deny_unknown_fields`, потому что бинарник и его JSONL-протокол зафиксированы
одним `tools.lock`; добавление upstream-поля требует явной миграции адаптера.
Закрытая форма событий:

1. `start` содержит ровно `type`, неотрицательный `total_files` и непустой
   `version`; он встречается один раз первым;
2. каждый `file` содержит ровно `type`, непустой `path`, массив `diagnostics`,
   опциональные `metrics` и `error`; абсолютный path допустим только внутри
   выбранного source root, после чего публикуется нормализованно относительно
   него с `/`; escape, пустой и повторный нормализованный path запрещены;
3. `metrics`, когда присутствует, содержит только неотрицательные `functions`,
   `complexity` и `cognitive_complexity`;
4. каждый diagnostic содержит непустые `code` и `message`, одну из семи
   upstream severity (`Blocker`, `Critical`, `Major`, `Error`, `Warning`,
   `Information`, `Hint`), координаты `usize` и опциональные уникальные tags из
   `Unnecessary|Deprecated`; конец range не предшествует началу;
5. непустой `error` взаимоисключается с diagnostics и metrics; его текст
   проходит redaction до сохранения как `fileFailure`;
6. `done` содержит ровно `type`, конечный неотрицательный `elapsed_secs` и
   неотрицательные `total_files`, `total_diagnostics`, `failed_files`; он
   встречается один раз последним, после него событий нет;
7. число `file` совпадает с обоими `total_files`, сумма diagnostics — с
   `total_diagnostics`, а число `file.error` — с `failed_files`.

Неизвестный event/field, неверный scalar, невалидная строка, слишком длинная
строка, повтор, отсутствующий terminal event и противоречивый total не
отбрасываются как warning: без доказанного terminal состояния они не могут
означать чистый код.

### Публичное `data`

Успешный результат имеет стабильную форму Unica, а не копию upstream JSONL:

```json
{
  "action": "analyze",
  "state": "completed",
  "complete": true,
  "retryable": false,
  "analyzerVersion": "0.2.62",
  "files": {
    "discovered": 2,
    "processed": 2,
    "failed": 0
  },
  "diagnostics": {
    "reported": 3,
    "matched": 2
  },
  "itemsTotal": 2,
  "itemsReturned": 1,
  "truncated": true,
  "items": [
    {
      "kind": "diagnostic",
      "path": "CommonModules/Sales/Ext/Module.bsl",
      "code": "LineLength",
      "severity": "warning",
      "message": "Line too long",
      "range": {
        "startLine": 10,
        "startColumn": 0,
        "endLine": 10,
        "endColumn": 150
      },
      "tags": []
    }
  ],
  "elapsedSeconds": 0.4
}
```

`items` — закрытый union:

- `kind=diagnostic` несёт path и finding в форме, совместимой по смыслу с
  `mode=file`: четырёхуровневую severity, 0-based range и tags; при
  `detail=detailed` дополнительно публикуется исходная семиуровневая
  `internalSeverity`;
- `kind=fileFailure` несёт path и redacted message файла, анализ которого
  upstream не завершил.

Семиуровневая severity отображается ровно так же, как в upstream MCP:
`Blocker|Critical|Major|Error → error`, `Warning → warning`,
`Information → info`, `Hint → hint`. Tags публично нормализуются в
`unnecessary|deprecated`.

Фильтры и ограничение имеют закрытую семантику:

- отсутствующий или пустой `codes` означает все коды; элементы массива —
  непустые уникальные строки и сравниваются с `diagnostic.code` точно, с учётом
  регистра;
- `minSeverity` по умолчанию `warning` и задаёт включающий нижний порог по
  четырём публичным уровням;
- `detail` по умолчанию `concise`;
- `limit` — целое `1..=200`, по умолчанию `200`, как действующий default
  upstream MCP; он ограничивает объединённые предметные сущности, не строки;
- file failures не скрываются `codes` и `minSeverity`.

До `limit` элементы сортируются по стабильному ключу: нормализованный `path`,
затем `fileFailure` перед `diagnostic`, затем start/end range, `code` и
`message`. Потоковый runner может поддерживать первые `limit` элементов этим
же ключом через bounded top-K; порядок discovery upstream публичным контрактом
не является.

Счётчики определены формулами: `diagnostics.reported` равен
`done.total_diagnostics`; `diagnostics.matched` — число диагностик после
`codes` и `minSeverity`, но до `limit`; `itemsTotal = matched + failed`;
`itemsReturned = items.length`; `truncated = itemsReturned < itemsTotal`.
`files.discovered` равен `start.total_files`, `files.failed` —
`done.failed_files`, `files.processed = discovered - failed`. Поэтому «полный
результат до среза» означает отфильтрованное множество diagnostics вместе со
всеми file failures, а не исходный upstream total.

### Незавершённые состояния

После успешного запуска процесса Unica различает четыре состояния одной формы
`data`:

| Состояние | `ok` | Код | `complete` | `retryable` |
| --- | --- | --- | --- | --- |
| полный `start…done` | `true` | — | `true` | `false` |
| только валидный `start` | `false` | `diagnostics_pending:` | `false` | `true` |
| есть `file`, но нет `done` | `false` | `diagnostics_incomplete:` | `false` | `false` |
| невалидная грамматика или totals | `false` | `diagnostics_invalid:` | `false` | `false` |

Неизвестные counts и `elapsedSeconds` представлены `null`, `items` остаётся
пустым: частичный поток не публикует неполные findings. Raw stdout не
возвращается даже при parser failure; ошибка называет номер физической строки
и класс нарушения, но не копирует потенциально чувствительное содержимое.

Spawn failure, ненулевой exit, timeout и cancellation сохраняют существующие
process outcomes. Cancellation не превращается в protocol failure, а stderr
остаётся только redacted диагностическим потоком процесса.

## Контракт RLM readiness для `code.definition`

Неготовность должна сохраняться как типизированная provider-error до
`RlmNavigationAdapter`. Сегодня `WorkspaceServiceManager::call_rlm_cancellable`
схлопывает post-admission `index_status` в строковый `Err`, поэтому одного
исправления раннего `match readiness` недостаточно: stale generation после
запуска запроса обошёл бы общий mapper.

Новый внутренний результат RLM различает:

- предметный helper output;
- `IndexReadiness` до вызова и после freshness recheck;
- cancellation;
- transport/helper failure.

Оба readiness-выхода проходят один mapper:

| Readiness | Публичный результат |
| --- | --- |
| `Ready` и helper ответил | `ok=true`, `CodeDefinitionResult` в `data` |
| `Building` | `ok=false`, `index_pending:`, retry hint, без `data`/`stdout` |
| `Missing` | `ok=false`, `index_unavailable: index is missing`, без retry hint |
| `Stale` | `ok=false`, `index_unavailable:` с нормализованным status |
| `Failed` | `ok=false`, `index_unavailable:` с redacted причиной |
| `Unavailable` | `ok=false`, `index_unavailable:` с redacted причиной |
| cancellation | отдельный cancelled outcome |

`index_pending:` сам является документированным retryable-кодом.
`index_unavailable:` имеет `retryable=false` и не обещает, что повтор поможет.
Текущий service умеет поставить в очередь поток, который ещё только попробует
запустить maintenance, но не возвращает доказательство фактического build или
update. Поэтому ADR-0045 не публикует recovery hint для `Missing`, `Stale`,
`Failed` или `Unavailable`; отдельный типизированный maintenance-disposition
понадобится прежде, чем такой hint станет честным.

Готовый индекс с `definitions=[]` остаётся успешным typed-result. Это
единственный пустой результат: неготовность индекса больше нельзя прочитать как
«определений нет».

## Изменения по слоям

Implementation PR ADR-0045 меняет:

- `application/mod.rs` и `application/tool_contracts.rs`: поверх контракта
  ADR-0044 добавляет запрет текстового дубля typed-reader и точную схему
  аргумента diagnostics `codes`;
- `application/tool_contracts.rs`: mode-scoped `format`, закрытые defaults и
  bounds для filters/limit diagnostics;
- `infrastructure/platform/process.rs` и новый изолированный JSONL-модуль:
  общий lifecycle child с отдельным потоковым drain/parser без stdout-tail;
  платформенная реализация остаётся за границей ADR-0009 и стражем
  `check-rust-platform-boundary.py`;
- `internal_adapters.rs`: принудительный jsonl route и преобразование результата
  потокового runner в `BslAnalyzerOutcome`;
- `workspace_services.rs`: сохранение structured readiness на execution
  boundary;
- `rlm_navigation.rs`: единый mapper readiness и typed definition success;
- `code-diagnostics` skill и `INV-MCP-TYPED-RESULT`: актуальные данные, коды и
  исполняемые проверки вместо старого prose.

`INV-MCP-TYPED-RESULT` ссылается на фактическое решение-владельца общего guard
и предметных протоколов в том же changeset.

`meta.profile` нигде не восстанавливается. Результат по #292 фиксируется с
пояснением, что его metadata-половина устранена снятием инструмента в #309, а
эта работа исправляет оставшийся `code.definition`.

## Красно-зелёная проверка реализации

Каждая строка сначала добавляется как падающий test на текущем коде и только
потом получает production-изменение:

Общий красный срез поверх ADR-0044 доказывает отклонение текстового дубля
успешного `Read + Typed` с префиксом `typed_result_textual:` и точное совпадение
схемы `codes` с runtime-валидацией. Предметные тесты diagnostics и RLM
доказывают полезные данные и честные отказы.

| Правило | Красная проверка |
| --- | --- |
| command | default и explicit analyze добавляют `--format jsonl`; отсутствие/`json`/`jsonl` эквивалентны; `console`, неизвестный `format` и `format` в другом mode отвергаются |
| transport | валидный JSONL суммарно больше `1 MiB` завершается typed success; одна строка больше `8 MiB` fail-closed; stdout нигде не удерживается и не публикуется |
| process priority | cancellation, timeout и ненулевой exit побеждают накопленное состояние parser и сохраняют существующие redacted outcomes |
| platform boundary | потоковый child lifecycle проходит одинаковые fake-process tests на macOS, Linux и Windows; новые `cfg` не выходят из `infrastructure/platform` |
| event grammar | fixtures clean/findings/file failure/zero files/only-start/missing-done, unknown event/field, malformed scalar/line, duplicate start/done/path и inconsistent totals |
| diagnostic semantics | пустые path/code/message, escape path, семь допустимых и неизвестная severity, неверный range, неизвестный/повторный tag, error вместе с diagnostics/metrics |
| projection | точная 7→4 severity и tags mapping; defaults `warning`/`concise`/`200`; case-sensitive codes; file failures обходят filters |
| limit | стабильный результат при разном upstream discovery order; `itemsTotal = matched + failed`, `itemsReturned`, `truncated` и file counters на границах 1 и 200 |
| readiness | табличная матрица всех `IndexReadiness`, `retryable=true` только для `Building`, без recovery hint у остальных; готовый empty definitions — success |
| freshness | post-execution stale generation проходит тот же mapper, не публикует helper output и не маскируется transport-строкой |
| public envelope | MCP smoke default analyze и unready definition проверяет полный `OperationResult`, отсутствие stdout/partial data и стабильные error prefixes |

После реализации выполняются целевые Rust-тесты, полный
`cargo test -p unica-coder -- --test-threads=1`, MCP smoke, architecture guards,
skill tests, `cargo fmt --check`, `cargo clippy` и `git diff --check`.

## Граница доставки

Этот PR доставляет одобренный проект и accepted ADR-0045 вместе с реализацией
задачи #291 и текущей части #292. После отдельного слияния #297 через PR #428 ветка
обновлена от `main` и использует ADR-0044 как уже принятый общий контракт. Здесь
остаются совместимое усиление guard, предметные адаптеры, падавшие до исправления
тесты, skill и синхронизация инварианта; зависимого или последующего
implementation PR нет.
