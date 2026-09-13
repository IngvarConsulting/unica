- Date: `2026-09-01`
- Status: `approved`
- Decision: `DEC.2026-09-01.VIEW-WORKSPACE-BOOTSTRAP`

# Bootstrap рабочего пространства через `unica.view {}`

## Проблема

Канонический `unica.view` требует `at`, но модель не получает ни списка source
sets, ни формата логического адреса, ни состояния `v8project.yaml`. В пустом
окружении вызов останавливается на schema validation с сообщением о пропущенном
поле, а затем actor admission всё равно требует уже обнаруженный Platform XML
source set. Это замкнутый круг: адрес можно узнать только после успешного
дискавери, а дискавери недоступен без адреса.

## Решение

`unica.view` получает два read-only режима:

- `{}` наблюдает рабочее пространство до source admission;
- `{"at":"<sourceSet>:<Kind>[.<Name>...]"}` читает логический узел как сейчас.

Bootstrap-ответ объединяет компактную часть прежних `project.map` и
`project.status`: `workspaceRoot`, путь и состояние `v8project.yaml`, найденные
source sets с kind/path/format, выбранный source set, `ready`, отдельный
`repositoryReady`, checks и diagnostics старого health evaluator.

Состояния конфигурации закрыты:

- `configured` — `v8project.yaml` прочитан;
- `autodetected` — файла нет, но source sets найдены;
- `missing` — файла и source sets нет;
- `invalid` — файл есть, но не читается или не разбирается.

`configured`, `autodetected` и `missing` являются успешным наблюдением. `invalid`
возвращает типизированную ошибку и не маскируется автодискавери.

В `next` публикуются только исполнимые следующие действия. Для найденного source
set это вызов `unica.view` с готовым корневым адресом, но только когда health
подтвердил source readiness, а формат допускается canonical actor admission.
EDT, unknown, invalid и отсутствующий root описываются, но не получают
неисполнимого `next`. Имя effective source set дополнительно обязано кодироваться
как XML NCName; иначе `ready=false`, diagnostics объясняет переименование, а
невалидный логический адрес не публикуется. Когда конфигурации нет и найденные
source sets имеют один известный формат, ответ дополнительно содержит `setup` с
точным относительным путём и полным содержимым `v8project.yaml`; модель может
создать файл доступным ей файловым инструментом. Не реализованные
`unica.run source.create/source.attach` не предлагаются.

`setup.content` порождается YAML-сериализатором, поэтому имена и пути с
YAML-значимыми символами остаются точными. Для полностью EDT-дискавери рецепт
сохраняет `format: EDT`; для пустого workspace безопасный пример остаётся
Designer export в `src/`. Смешанный autodetect выбирает global default по
effective source set, а сильные format evidence остальных наборов сохраняют их
собственный формат; если effective известного формата нет, `content=null` вместо
ложного рецепта. Если валидный существующий `v8project.yaml` не объявляет source
sets, `content=null`, а `sourceSetExample` задаёт структурированный пример для
точечного изменения поля; тем самым неизвестные поля и комментарии
существующего файла не выдаются за заменяемые.

Если `at` отсутствует, `filter`, `limit` и `cursor` запрещены: bootstrap не
имеет постраничного логического результата. Некорректный `at` сообщает формат и
предлагает `unica.view {}` вместо повторного угадывания.

## Discoverability на проводе

`initialize.instructions` всегда сообщает начальный вызов и формат адреса;
startup notice добавляется отдельным абзацем. Все восемь канонических tools и
каждое опубликованное поле input schema получают короткое предметное описание.
Описание одного tool ограничено 2 KiB, а сериализованный compatibility
`tools/list` — 16 KiB, чтобы исправление не вернуло мегабайтную поверхность.
Поставляемые skills и references не направляют модель к снятым
`unica.project.map/status`.

## Границы

Публичные имена, число tools и result envelope не меняются. Bootstrap ничего не
пишет, не создаёт source set и не подменяет `unica.check`. Реализация
`source.create/source.attach` остаётся отдельным вертикальным срезом с
транзакцией публикации; включать её в read-only исправление означало бы смешать
дискавери и новый mutation-контракт.

Bootstrap отвечает прямо до actor admission и не создаёт durable task. Его
discovery и health inspection получают только остаток handoff budget; даже при
нулевом frontend budget 125 ms response margin не расходуются на probes и
остаются сериализации ответа. Deadline-ограниченная неполная health-сводка
публикуется с `readinessState=incomplete`, а не маскируется как complete.
Межпроцессная доставка frontend cancellation отсутствует у всего canonical
daemon-router и этой работой не расширяется.

## Проверка

- schema принимает `{}` и по-прежнему принимает адресный режим;
- empty/configured/autodetected/invalid проверяются через daemon boundary;
- directory и broken-link `v8project.yaml` классифицируются как `invalid`;
- EDT и невалидное имя source set не получают неисполняемый actor `next`;
- YAML-рецепт round-trip безопасен, mixed-format не получает ложный рецепт, а
  существующий config без source sets не предлагается перезаписать;
- каждый аргумент и tool описан, лимиты поверхности закреплены тестом;
- initialize-инструкция присутствует с notice и без него;
- интеграционный stdio MCP отвечает на `unica.view {}` и его `tools/list` измеряется
  тем же сериализованным payload, который получает клиент; ожидание stdout
  ограничено реальным channel timeout, а не проверкой перед blocking read.
