- Date: `2026-08-10`
- Status: `approved`
- Decision: `ADR-0041`

# Завершённость типизированных readers для diagnostics и RLM

## Результат исследования

Issue [#291](https://github.com/IngvarConsulting/unica/issues/291) полностью
воспроизводится на `origin/main` (`b0205624`). Default-ветка
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
нельзя; новое решение ADR-0041 выбирает исполняемую границу и конкретные
протоколы двух readers.

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

### Guard плюс два предметных протокола

Выбран этот вариант. Live-реестр и application-finalizer защищают общее
обязательство, JSONL parser переводит one-shot analyze в устойчивую модель
Unica, а типизированная RLM-ошибка сохраняет readiness до публичной границы.
Изменение остаётся одним связным вертикальным срезом: оба issue относятся к
ложному успеху typed-reader, но данные и отказ каждого поставщика не
смешиваются.

## Live-контракт результата

`ToolSpec` получает обязательный `result_contract`. Минимальный закрытый набор
значений:

- `Typed` — предметный результат принадлежит `OperationResult.data`;
- `ExternalProcessStream` — текст является потоком внешнего процесса, который
  ADR-0023 сознательно не типизирует;
- `Legacy` — инструмент находится вне принятой typed-границы.

CI сопоставляет `ToolSpec.result_contract` с `scope` и `result.contract` из
`tool-surface-review.json`. Новая декларация не становится вторым ручным
ledger: Rust владеет исполняемым видом результата, JSON — ревью-состоянием и
сценариями, а тест доказывает их совпадение.

После handler и до публикации cache/events application-finalizer проверяет
каждый немутирующий `Typed` tool:

- `ok=true` требует `data.is_some()`;
- `ok=true` требует отсутствующий `stdout`;
- нарушение превращается в `ok=false` с кодом
  `typed_result_violation:`, очищает `stdout` и не создаёт сфабрикованный
  `data`;
- `ok=false` может нести типизированное `data` о состоянии отказа, но не
  обязано его синтезировать для process spawn, timeout или cancellation.

`dryRun` не имеет смысла для немутирующего typed-reader. Такой аргумент должен
отклоняться validation-слоем, а не запускать успешный placeholder без данных.
Preview-контракт typed mutations этим решением не меняется и остаётся в #290.

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

### Изолированный parser

JSONL-разбор выносится из уже крупного `internal_adapters.rs` в отдельный
инфраструктурный модуль. Он принимает полную stdout-строку завершившегося
процесса и строит доменный `DiagnosticsAnalyzeData`; CLI runner по-прежнему
владеет командой, timeout, cancellation, redaction и stderr.

Допустимый автомат событий закрыт:

1. ровно один `start` первым;
2. ноль или больше `file` с уникальным `path`;
3. ровно один `done` последним;
4. после `done` нет событий;
5. число `file` равно `start.total_files` и `done.total_files`;
6. сумма diagnostics и число file-errors равны totals из `done`;
7. числовые поля неотрицательны, `elapsed_secs` конечен.

Неизвестный event, невалидная строка, повторный path, лишний terminal event и
противоречивые totals не отбрасываются как warning: без доказанного terminal
состояния они не могут означать чистый код.

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
  `mode=file`: четырёхуровневую severity, 0-based range и tags; detailed-mode
  дополнительно может нести `internalSeverity`;
- `kind=fileFailure` несёт path и redacted message файла, анализ которого
  upstream не завершил.

`codes` и `minSeverity` применяются к diagnostics после разбора полного
потока. File failures фильтром не скрываются. `limit` затем ограничивает
объединённые предметные `items` в детерминированном порядке потока, а не строки
JSONL. `itemsTotal`, `itemsReturned` и `truncated` делают срез наблюдаемым;
`diagnostics.reported` сохраняет полный terminal total, а `matched` показывает
результат фильтров до `limit`.

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
| `Missing` | `ok=false`, `index_unavailable: index is missing`, recovery hint |
| `Stale` | `ok=false`, `index_unavailable:` с нормализованным status |
| `Failed` | `ok=false`, `index_unavailable:` с redacted причиной |
| `Unavailable` | `ok=false`, `index_unavailable:` с redacted причиной |
| cancellation | отдельный cancelled outcome |

`index_pending:` сам является документированным retryable-кодом.
`index_unavailable:` не обещает, что повтор поможет; recovery hint добавляется
только когда service доказал, что build/update действительно запрошен.

Готовый индекс с `definitions=[]` остаётся успешным typed-result. Это
единственный пустой результат: неготовность индекса больше нельзя прочитать как
«определений нет».

## Изменения по слоям

- `application/mod.rs`: live result contract и общий typed-reader finalizer;
- `application/tool_contracts.rs`: mode-scoped `format`, отказ `dryRun` у
  typed readers и точная diagnostics validation;
- новый изолированный infrastructure parser JSONL;
- `internal_adapters.rs`: принудительный jsonl route и преобразование parser
  result в `BslAnalyzerOutcome`;
- `workspace_services.rs`: сохранение structured readiness на execution
  boundary;
- `rlm_navigation.rs`: единый mapper readiness и typed definition success;
- `code-diagnostics` и `code-search` skills, acceptance и tool-surface ledger:
  актуальные данные и коды вместо старого prose;
- при реализации ADR-0041 переводится в `accepted`, а существующее правило
  `INV-MCP-TYPED-RESULT` получает ссылку на новый исполняемый guard.

`meta.profile` нигде не восстанавливается. При последующей реализации результат
по #292 фиксируется с пояснением, что его metadata-половина устранена снятием
инструмента в #309, а новая работа исправляет оставшийся `code.definition`.

## Красно-зелёная проверка реализации

До изменения production-кода должны упасть:

1. application-test, передающий `ok=true` без `data` для typed-reader;
2. command-test default и explicit analyze, требующий `--format jsonl`;
3. parser fixtures для clean, findings, file failure, zero files, only-start,
   missing-done, malformed line, duplicate event/path и inconsistent totals;
4. limit-test по предметным items, доказывающий totals и truncation;
5. табличная матрица всех `IndexReadiness` для `code.definition`;
6. post-execution stale-generation test, доказывающий тот же mapper;
7. MCP smoke default analyze и unready definition, проверяющий публичный
   `OperationResult`, а не только adapter outcome;
8. ledger parity test для live `result_contract`.

После реализации выполняются целевые Rust-тесты, полный
`cargo test -p unica-coder -- --test-threads=1`, MCP smoke, architecture guards,
skill tests, `cargo fmt --check`, `cargo clippy` и `git diff --check`.

## Граница доставки

Этот PR фиксирует одобренный проект и proposed ADR-0041. Он не меняет runtime,
поэтому использует `Relates to #291` и `Relates to #292`, не закрывает issue и
не называется исправлением. Реализация начинается только после ревью этой
записки; тогда ADR, инвариант, код, fixtures, skills и acceptance должны войти
в один самостоятельно проверяемый implementation PR.
