---
name: code-diagnostics
description: "Диагностика BSL и объяснение отключений правил. Используй, когда нужно запустить или разобрать диагностики BSL, АПК, EDT, BSL LS, inline/range disable markers, suppression-комментарии или стандарт v8std за диагностикой."
---

# Code Diagnostics

## MCP routing

- Диагностику узла даёт `unica.check {at}`. Валидаторы следуют из вида узла:
  у модуля это BSL-анализатор, у объекта метаданных — его собственные проверки.
  Выбирать провайдера или действие не нужно и нечем: `check` принимает только
  адрес.
- Ответ несёт `status` (`passed` или `failed`), `validators` — какие проверки
  отработали, и `diagnostics` — находки с кодом, важностью, местом и стандартом.
  Провалом считается `error`, а не всякая пометка: подсказка по стилю модуль не
  ломает.
- **Пустой `diagnostics` при `status: passed` — доказательство.** Если анализ
  не завершился, `check` отказывает `provider_unavailable` и прямо говорит, что
  модуль не проверен. Отдельного вопроса о готовности провайдера больше нет:
  чистый ответ и неотработавший провайдер теперь различимы по самому ответу.
- Адрес узла даёт `unica.view {at}` по дереву или `unica.search {corpus: "names"}`
  по имени. Если отправная точка — путь из диффа или лога сборки, переведи его
  в адрес аварийным `unica.resolve`; в обычном ходе работы он не нужен.
- Стандарт v8std спрашивается только через `unica.docs` с
  `source: "development-standard"`. Не зови анализатор, стандарты или пакетные
  адаптеры напрямую.
- Код ищется `unica.search`: `role` выбирает, чем искать — `lexical` буквально,
  `symbol` по индексу символов, `semantic` по смыслу. Без `role` Unica ищет
  литерал сама.

### Чего на канонической поверхности пока нет

Называй это пробелом контракта, а не обходи стороной:

- **сплошной прогон по всему набору** — `check` проверяет один адрес; обхода
  всех модулей одним вызовом нет;
- **отбор по важности и кодам, ограничение выдачи** — `check` принимает только
  `at`;
- **каталог правил провайдера** — коды берутся из находок и из `unica.docs`;
- **граф вызовов для оценки влияния** — ветвей `Caller`/`Callee` у узла метода
  ещё нет.

## Workflow

1. Получи адрес узла: `unica.view {}` называет наборы исходников,
   `unica.search {corpus: "names"}` находит объект по имени, `unica.view {at}`
   спускается по дереву. У владельца есть ветвь `Module`, у модуля — `Method`.
2. Вызови `unica.check {at}` на модуле или объекте. Прочти `status`,
   `validators` и `diagnostics`.
3. Сгруппируй находки по месту, коду и первопричине. Иди за точным диапазоном
   в самой находке.
4. Разгляди предмет: `unica.view {at}` на узле метода даёт подпись, контекст
   компиляции и собственные строки; `unica.search` находит вхождения.
5. Спроси стандарт: `unica.docs` с `source: "development-standard"` по коду,
   имени диагностики, токену АПК/EDT/BSL LS или соседнему фрагменту.
6. Отчитайся: причина в исходнике, затронутые диагностики, логический адрес и
   место, доказательство из стандарта, результат перепроверки.

## Verification gate

This verification gate is mandatory:

- Проверяй `unica.check {at}` после правок, чувствительных к синтаксису, и
  считай новые находки важности `error` блокирующими.
- Оценивай влияние при изменении экспортного метода, обработчика метаданных,
  публичного API, пути запроса или контракта общего модуля. Пока графа вызовов
  на поверхности нет, доказательством служит `unica.search` по имени символа —
  и скажи, что доказательство неполное.
- Если публичный MCP `unica` не может показать нужное доказательство, сообщи о
  пробеле контракта вместо заявления о полной проверке.

## Suppression and range-disable comments

Когда inline/range disable markers или suppression-комментарии отключают
диагностику строки или диапазона, считайте точный маркер частью доказательства.

- Extract literal rule codes from АПК, EDT, BSL LS, analyzer, or suppression comments.
- Explain an отключение only when the code, surrounding range, and standard support the reason.
- Prefer fixing the cause or narrowing the disabled range. Keep suppression only with standards, platform-help, or runtime evidence of an intentional false positive.

## MCP examples

Проверка одного модуля:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": { "at": "main:CommonModule.Продажи.Module.Manager" }
  }
}
```

Стандарт за диагностикой:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.docs",
    "arguments": {
      "query": "АПК:142 LineLength",
      "source": "development-standard"
    }
  }
}
```

Вхождения символа перед правкой общего кода:

```jsonc
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.search",
    "arguments": {
      "query": "ПередЗаписью",
      "role": "symbol",
      "scope": "main:Configuration"
    }
  }
}
```
