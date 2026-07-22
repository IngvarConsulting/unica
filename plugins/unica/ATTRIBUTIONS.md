# Авторы, источники и лицензии

Эта страница описывает компоненты публичного пакета Unica, источники идей и
адаптированного поведения, а также границы применимых лицензий. Версии,
репозитории и закреплённые коммиты инструментов задаются в
[`third-party/tools.lock.json`](third-party/tools.lock.json), а происхождение
skills — в [`provenance/skill-upstreams.json`](provenance/skill-upstreams.json).

## Unica

<!-- unica-attribution: project unica -->

- Репозиторий: [IngvarConsulting/unica](https://github.com/IngvarConsulting/unica)
- Автор: [Ingvar Consulting, LLC](https://ingvar.pro)
- Лицензия: [LGPL-3.0-or-later](LICENSE)

Команда Unica благодарит всех авторов перечисленных ниже проектов. Unica
объединяет их через один типизированный MCP-сервер `unica`; это объединение не
заменяет и не отменяет лицензии отдельных компонентов.

## Встроенные инструменты

### BSL Analyzer

<!-- unica-attribution: tool bsl-analyzer -->

- Репозиторий: [itrous/bsl-analyzer](https://github.com/itrous/bsl-analyzer)
- Автор: [BSL Analyzer Contributors](https://github.com/itrous/bsl-analyzer/graphs/contributors)
- Закреплённая версия: `0.2.55`, commit `5a02bb44dedaf29e0e29af1f740279d279199854`
- Лицензия: [LGPL-3.0-or-later](third-party/licenses/bsl-analyzer/LICENSE-LGPL)
- Дополнительные условия и происхождение: [NOTICE](third-party/licenses/bsl-analyzer/NOTICE)

Unica поставляет LSP-бинарник `bsl-analyzer`. Его лицензионная заметка
объясняет смешанную модель workspace: итоговый бинарник статически связывает
компоненты уровня LGPL и поэтому распространяется как LGPL-3.0-or-later; там же
перечислены архитектурные источники, тестовые данные и материалы платформы 1С
с отдельными условиями.

### v8-runner

<!-- unica-attribution: tool v8-runner -->

- Репозиторий: [alkoleft/v8-runner-rust](https://github.com/alkoleft/v8-runner-rust)
- Автор: [v8-runner contributors](https://github.com/alkoleft/v8-runner-rust/graphs/contributors)
- Закреплённая версия: `0.5.1`, commit `ad72f64222ab0a7e6dfd391adb437a956c0a2428`
- Лицензия: [AGPL-3.0-only](third-party/licenses/v8-runner/LICENSE)

`v8-runner` запускается Unica как отдельный внутренний процесс. На его
распространяемый бинарник и исходный код действует AGPL-3.0-only; лицензия
LGPL-3.0-or-later проекта Unica не заменяет эти условия.

### rlm-tools-bsl и rlm-bsl-index

<!-- unica-attribution: tool rlm-tools-bsl -->
<!-- unica-attribution: tool rlm-bsl-index -->

- Репозиторий: [Dach-Coin/rlm-tools-bsl](https://github.com/Dach-Coin/rlm-tools-bsl)
- Автор: [Roman Starchenko](https://github.com/Dach-Coin); исходный проект
  `rlm-tools` — [Stefan O'Shea](https://github.com/stefanoshea)
- Закреплённая версия: `1.26.0`, commit `dcfff95ce678f49971b14d8acd82b042a6855470`
- Лицензия: [MIT](third-party/licenses/rlm-tools-bsl/LICENSE)

Оба бинарника собираются из одного репозитория. MIT notice сохраняет
благодарность Stefan O'Shea за исходный `rlm-tools` и Roman Starchenko за
адаптацию `rlm-tools-bsl`.

## Внешние сервисы

### v8std

<!-- unica-attribution: adapter v8std -->

- Поставщик: [проект v8std и его участники](https://github.com/zeegin/v8std)
- Сервис: [ai.v8std.ru/mcp](https://ai.v8std.ru/mcp)

Unica обращается к этому MCP-сервису как к удалённому адаптеру стандартов 1С.
Сам сервис, его серверный код и содержимое сайта **не распространяется** в
пакете Unica и не включается в цепочку лицензирования поставляемых бинарников.

## Источники поведения и идей

### cc-1c-skills

<!-- unica-attribution: upstream cc-1c-skills -->

- Репозиторий: [Nikolay-Shirokov/cc-1c-skills](https://github.com/Nikolay-Shirokov/cc-1c-skills)
- Автор: [Nick Shirokov](https://github.com/Nikolay-Shirokov)
- Проверенный baseline: `f3466e19fdc37954c030e48daabcc192f0098fe7`
- Лицензия: [MIT](https://github.com/Nikolay-Shirokov/cc-1c-skills/blob/f3466e19fdc37954c030e48daabcc192f0098fe7/LICENSE)

Unica благодарит Nick Shirokov за практические операции и описание форматов
1С. Принятое поведение переработано в собственную реализацию Unica и доступно
только через типизированные инструменты `unica.*`; исходные script-wrapper'ы
донорского проекта в публичный workflow не входят.

### ai_rules_1c

<!-- unica-attribution: upstream ai-rules-1c -->

- Репозиторий: [comol/ai_rules_1c](https://github.com/comol/ai_rules_1c)
- Автор: [Oleg Philippov (comol)](https://github.com/comol)
- Проверенный baseline: `484e550043a4cb749d59d0671329f3112e3ae668`

Из `ai_rules_1c` использованы только общие идеи. Текст, код и иные формы
выражения из репозитория не копировались и не адаптировались; соответствующие
skills принадлежат Unica. На указанном baseline лицензия не опубликована,
поэтому Unica не заявляет право на распространение материалов этого проекта и
не включает его в цепочку лицензий поставки.

### v8-runner-rust как источник runtime-контракта

<!-- unica-attribution: upstream v8-runner-rust -->

- Репозиторий: [alkoleft/v8-runner-rust](https://github.com/alkoleft/v8-runner-rust)
- Автор: [v8-runner contributors](https://github.com/alkoleft/v8-runner-rust/graphs/contributors)
- Проверенный baseline: версия и commit берутся из `third-party/tools.lock.json`
- Лицензия: [AGPL-3.0-only](third-party/licenses/v8-runner/LICENSE)

Контракт runtime-навыков Unica согласован с возможностями закреплённого
`v8-runner`. Сам бинарник остаётся отдельным AGPL-компонентом; адаптер и
публичная MCP-поверхность Unica распространяются по лицензии Unica.

## Как читается цепочка лицензий

- собственный код и документация Unica — LGPL-3.0-or-later;
- адаптированное по MIT поведение `cc-1c-skills` сохраняет ссылку на автора и
  исходную лицензию, а реализация Unica публикуется под LGPL-3.0-or-later;
- отдельные встроенные бинарники сохраняют собственные лицензии:
  LGPL-3.0-or-later, AGPL-3.0-only или MIT согласно разделам выше;
- `ai_rules_1c` является источником идей, а не распространяемого или
  адаптированного материала;
- удалённый сервис v8std не поставляется вместе с Unica.

Полные тексты и обязательные notices для поставляемых компонентов находятся в
каталоге [`third-party/licenses/`](third-party/licenses/). При расхождении этой
страницы с package metadata источником истины являются закреплённые manifests и
тексты лицензий; страницу необходимо исправить вручную.

## Благодарности

Спасибо авторам и участникам BSL Analyzer, v8-runner, rlm-tools,
rlm-tools-bsl, cc-1c-skills, ai_rules_1c и v8std, а также сообществу разработки
1С за открытые инструменты, исследования форматов и практические знания, на
которых строится Unica.
