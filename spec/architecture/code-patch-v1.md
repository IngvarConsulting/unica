# `unica.code.patch` v1: границы и контракт

Статус: accepted for safe narrow v1.
Связанная задача: #73.

## Цель

Добавить одну безопасную точечную мутацию BSL-модуля через публичный MCP
`unica.code.patch`. Инструмент не является редактором произвольного текста и не
заменяет общий writer contract из #74.

## Границы v1

- Только platform XML `CONFIGURATION` source set, выбранный обычной политикой
  source-set Unica.
- Только один существующий regular-файл `*Module.bsl` в поддерживаемом
  canonical platform-XML layout внутри автоматически выбранного source set;
  metadata owner и обязательные companion descriptors должны существовать.
- Только одна операция за вызов; batch, несколько файлов и каскадные изменения
  запрещены.
- Только операция `insert` с одним из selector-ов `method` или `anchor`.
- `ExternalDataProcessor`, `ExternalReport`, EDT, configuration extension,
  создание модуля, `delete`, rename и `allowMissing` не входят в v1.
- `dryRun` остаётся значением по умолчанию. Applied вызов требует
  `dryRun: false` и проходит обычную политику workspace path/support guard.

Такой scope покрывает исходный сценарий: добавить BSL-фрагмент в известный
существующий модуль без широкого переписывания источника.

## Публичный MCP-контракт

Имя: `unica.code.patch`.

Общие поля остаются единообразными для public tools: `cwd`, `sourceDir` и
`dryRun`. `sourceDir` выбирает один настроенный Configuration source set и
обязателен при неоднозначном выборе.

| Поле | Тип | Правило v1 |
| --- | --- | --- |
| `path` | string | Обязательный workspace-relative путь к существующему `*Module.bsl`. |
| `operation` | string | Обязательное значение `insert`. |
| `selector` | object | Ровно один selector `method` или `anchor`. |
| `content` | string | Непустой BSL-фрагмент для вставки. |
| `position` | string | `before` или `after`; вставка всегда происходит по границе строки метода или anchor. |
| `sourceDir` | string | Необязательный путь к настроенному source set; разрешает неоднозначность. |

`selector.method` — точное имя процедуры или функции из AST. Индекс и
валидация используют `parser`/`syntax` из того же source commit
`bsl-analyzer`, который закреплён в `third-party/tools.lock.json`; contract-test
не позволяет этим версиям разойтись. Диапазон метода включает связанные
аннотации, а слова в свойствах, строках и комментариях не образуют методы.

`selector.anchor` — точный многострочный текст. Он должен дать ровно одно
совпадение в допустимой method-scoped области. Неоднозначный или отсутствующий
selector — ошибка, а не no-op. EOL в payload канонизируется, поэтому LF-anchor
сопоставляется с LF/CRLF и mixed-EOL source, но найденные диапазоны остаются
реальными байтовыми диапазонами исходника. Пересекающиеся совпадения считаются
отдельно.

Для `before` выбирается начало строки declaration/anchor. Для `after` — байт
после окончания его строки. Если последняя строка не имеет EOL, writer сначала
добавляет EOL непосредственного контекста, поэтому closing token и новый текст
не склеиваются.

## Результат и идемпотентность

`OperationResult.data` должен включать:

- canonical `path`, выбранный `sourceSet` и `affectedTarget` с module role;
- `preHash` и `postHash` SHA-256 по исходным байтам;
- `changedRanges` в байтовых и line/column координатах postimage;
- unified diff, построенный из той же postimage, что и `postHash`;
- нормализованный affected target: source set, owner, module role и raw-byte
  hash postimage.
- terminal `validation`: `kind=bsl-analyzer-parser`, `status=passed|failed`,
  `validatedPostHash` и структурированные diagnostics с byte/line/column
  диапазонами.

Это именно JSON-object в `OperationResult.data`; `stdout` не используется как
внутренний контейнер для сериализованного JSON.

Повторный идентичный вызов распознаётся до записи как semantic no-op: `preHash`
равен `postHash`, diff пустой, ranges пусты, cache/event не публикуется.
Первый вызов отклоняется без записи, если его postimage не позволяет доказать
такой no-op при следующем идентичном вызове (например, вставка создаёт второй
selected method/anchor).

## Byte-stable writer requirements

- Неизменённые байты, включая BOM, XML-adjacent файловую структуру, terminal
  newline и локальный стиль EOL, сохраняются.
- Вставленный фрагмент получает EOL своего непосредственного контекста; mixed
  EOL не нормализуется глобально.
- Unified diff, ranges и `postHash` вычисляются из одного byte-exact postimage;
  перед возвратом diff повторно разбирается и применяется in-memory, а результат
  обязан побайтно совпасть с postimage.
- Exact in-memory postimage валидируется до staging/publication; applied write
  затем выполняется через staging file и atomic replace. Ошибка validation не
  меняет оригинал.

## Связь с source-sync

v1 возвращает `affectedTarget`, но не объявляет, что обычный runtime build
загрузил модуль в ИБ. До delivery contract из #76 результат является
доказательством source mutation, а не receipt для runtime/dump round-trip.
Подключение mutation event к durable dirty state выполняется отдельным
изменением после принятия #76 contract.

## RED-набор до реализации

1. BOM+CRLF: preview показывает только вставку; suffix не становится ложной
   заменой; applied результат byte-identical вне вставки.
2. Mixed EOL: method и multiline anchor используют локальный EOL; второй apply
   — no-op без записи.
3. Property/comment/string с `Обработчик.Процедура` или
   `Обработчик.КонецПроцедуры` не меняет границы метода.
4. Отсутствующий, двусмысленный или пересекающий closing token selector
   отклоняется до записи.
5. `path` вне workspace, symlink escape, не-`*Module.bsl`, неизвестный source
   set, external/EDT/extension target и `operation != insert` отклоняются.
6. `dryRun` возвращает те же pre/post hashes, diff и ranges, но не меняет файл
   и state; успешный apply публикует ровно один affected target.
7. Postimage validation failure до staging и occupied staging path сохраняют
   исходные байты.

## Последующие задачи

- Отдельный ADR и MCP schema/tests добавляются вместе с реализацией public
  инструмента.
- `replace`, `delete`, method documentation и atomic batch требуют отдельных
  контрактных решений.
- #74 расширяет writer guarantees на XML и остальные mutation tools; он не
  должен раздувать v1 `code.patch`.
