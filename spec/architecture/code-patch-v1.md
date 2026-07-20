# `unica.code.patch` v1: границы и контракт

Статус: proposed.  
Связанная задача: #73.

## Цель

Добавить одну безопасную точечную мутацию BSL-модуля через публичный MCP
`unica.code.patch`. Инструмент не является редактором произвольного текста и не
заменяет общий writer contract из #74.

## Границы v1

- Только platform XML `CONFIGURATION` source set, выбранный обычной политикой
  source-set Unica.
- Только один существующий файл `Module.bsl` внутри выбранного source set.
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

Общие поля остаются единообразными для public tools: `cwd`, `sourceDir`,
`dryRun` и `supportPolicy`.

| Поле | Тип | Правило v1 |
| --- | --- | --- |
| `path` | string | Обязательный workspace-relative путь к существующему `Module.bsl`. |
| `operation` | string | Обязательное значение `insert`. |
| `selector` | object | Ровно один selector `method` или `anchor`. |
| `content` | string | Непустой BSL-фрагмент для вставки. |
| `position` | string | `before` или `after`; для `method` — граница метода, для `anchor` — граница совпавшего anchor. |

`selector.method` — точное имя процедуры или функции. Scanner распознаёт
`Процедура`/`Функция` и `КонецПроцедуры`/`КонецФункции` только как структурные
токены в начале строки, а не как свойства, строки или комментарии.

`selector.anchor` — точный многострочный текст. Он должен дать ровно одно
совпадение в допустимой method-scoped области. Неоднозначный или отсутствующий
selector — ошибка, а не no-op.

## Результат и идемпотентность

`OperationResult.data` должен включать:

- canonical `path` и выбранный `sourceSet`;
- `preHash` и `postHash` SHA-256 по исходным байтам;
- `changedRanges` в байтовых и line/column координатах postimage;
- unified diff, построенный из той же postimage, что и `postHash`;
- нормализованный affected target: source set, owner, module role и raw-byte
  hash postimage.

Повторный идентичный вызов распознаётся до записи как semantic no-op: `preHash`
равен `postHash`, diff пустой, ranges пусты, cache/event не публикуется.

## Byte-stable writer requirements

- Неизменённые байты, включая BOM, XML-adjacent файловую структуру, terminal
  newline и локальный стиль EOL, сохраняются.
- Вставленный фрагмент получает EOL своего непосредственного контекста; mixed
  EOL не нормализуется глобально.
- Unified diff, ranges и `postHash` вычисляются из одного byte-exact postimage.
- Applied write выполняется через staging file, validation и atomic replace;
  ошибка validation не меняет оригинал.

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
5. `path` вне workspace, symlink escape, не-`Module.bsl`, неизвестный source
   set, external/EDT/extension target и `operation != insert` отклоняются.
6. `dryRun` возвращает те же pre/post hashes, diff и ranges, но не меняет файл
   и state; успешный apply публикует ровно один affected target.
7. Post-write validation failure и occupied staging path сохраняют исходные
   байты.

## Последующие задачи

- Отдельный ADR и MCP schema/tests добавляются вместе с реализацией public
  инструмента.
- `replace`, `delete`, method documentation и atomic batch требуют отдельных
  контрактных решений.
- #74 расширяет writer guarantees на XML и остальные mutation tools; он не
  должен раздувать v1 `code.patch`.
