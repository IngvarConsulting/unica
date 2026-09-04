---
name: platform-help
description: "Справка платформы 1С и объектной модели BSL. Используй когда нужно уточнить метод, свойство, конструктор, поведение API, версию платформы, совместимость или стандартное решение задачи."
---

# Platform Help

## MCP routing

- Документацию спрашивает MCP `unica` инструментом `unica.docs`. `source` —
  один вид источника, а не имя поставщика. Список закрыт: `platform-help`,
  `development-standard`, `configuration-documentation`. Неизвестное имя
  отклоняется кодом `unsupported_source`, который перечисляет допустимые.
- `source: "platform-help"` — интерфейс программирования и механика платформы
  (Синтакс-помощник, справка конфигуратора, руководства площадки вендора).
  `source: "development-standard"` — сервер стандартов разработки.
- Без `source` спрашиваются оба сразу: справка платформы и стандарты. Это
  единственный смысл умолчания — встроенная справка конфигурации в него не
  входит.
- `source: "configuration-documentation"` пока отвечает `unsupported_source` с
  названной причиной: её читатель ещё не переведён на границу отмены рабочего
  пространства. Называйте это границей поверхности, а не отсутствием ответа.
- `unica.docs` может ответить задачей (`status: "working"`). Тогда возьмите
  `taskId` из ответа и дождитесь результата через `unica.task.result`. Ответ
  «задача принята» не является ответом на вопрос.
- Полного текста страницы канонический провод не отдаёт: инструмента,
  открывающего страницу по `documentId`, он не публикует. Доказательство —
  это `documentId`, `applicableVersion` и фрагмент попадания вместе; называйте
  их в ответе, чтобы читатель мог открыть ту же страницу сам, и не выдавайте
  фрагмент за прочитанную страницу.
- For project context, use `unica.search`, `unica.view {}`, and
  `unica.runtime.execute`.
- По INV-MCP-RUNTIME-RECEIPT и ADR-0074: `unica.runtime.execute` с `dryRun: true`
показывает запланированную команду без побочных эффектов, а с `dryRun: false`
исполняет классифицированную операцию и отвечает её терминальным результатом в
том же вызове, приложив названную причину риска (`runtime_risk_*`)
предупреждением; неклассифицированная операция по-прежнему отказывает
`runtime_operation_unbounded` до обнаружения рабочего пространства. Preview
исполнением не является. Работу, которую вызов ждать не должен, запускай через
`unica.runtime.job.start`. Не обходи контракт прямым runner-ом или через
`unica.build.*`.
- Когда вопрос об API зависит от структуры метаданных, читай её `unica.view` по
  логическому адресу объекта.
- Do not call internal standards, runtime, or package adapters directly.

## Чтение ответа

Ответ приходит секциями, по секции на поставщика, в порядке реестра.

- Каждая секция несёт `sourceKind` и `authority`, каждое попадание в ней —
  `applicableVersion` и `documentId`. Ответ обязан называть источник, версию и
  `documentId` страницы: без него читатель не может вернуться к той же странице.
- `language` секции — локаль, которой источник ответил на самом деле, а не
  запрошенная. Если они расходятся, назовите подстановку локали в ответе:
  справка поставляется не во всех локалях, и запрос на одной локали молча
  отвечал бы страницами другой.
- Секция со смыслом источника `development-standard` не закрывает вопрос о
  сигнатуре или механике платформы, каким бы уместным ни выглядел её текст. Это
  правило чтения, а не правило вызова. Симметрично: секция
  `configuration-documentation` описывает прикладную конфигурацию и не
  доказывает поведение самой платформы.
- Установка платформы старше в интерфейсе программирования, руководства
  площадки — в описательном слое: механизмы целиком, форматы адресов,
  администрирование. Расхождение их версий называйте в ответе.
- Отказ `provider_unavailable` означает, что ни один поставщик не дал
  результата: справка не установлена либо сетевой выход закрыт политикой.
  Назовите условие среды, а не отвечайте по памяти.

## Workflow

1. State the exact platform/API question: object, method/property, platform
   version, infobase mode, client/server context.
2. Вызовите `unica.docs` с именем объекта или члена — или с естественной
   формулировкой вопроса: поиск пословный, морфологический и нечёткий
   (ADR-0037), точная подстрока и порядок слов не требуются, опечатка в имени
   не прячет страницу.
3. Если ответ пришёл задачей, дождитесь его через `unica.task.result`.
4. Read `applicableVersion` in the hit. Если она расходится с версией проекта,
   назовите расхождение в ответе.
5. Назовите `documentId` попадания дословно вместе с ответом. Если фрагмента
   не хватает, чтобы утверждение стояло, скажите это прямо и не достраивайте
   страницу по памяти.
6. Validate against local project context with `unica.view {}` and targeted
   `unica.search` if the answer depends on project conventions.
7. For code examples, use `unica.runtime.execute` to preview `operation=syntax`
   and, with `dryRun: false`, to run it; report actual syntax and runtime
   behavior as unverified.

## Platform context

- Read `../../references/platform/compatibility-modes.md` for every question about a
  compatibility mode or version-sensitive behavior. Resolve the runtime
  platform, literal mode, effective compatibility version, and
  feature-specific boundary separately.
- Read `../../references/platform/platform-mechanics.md` when the answer depends on runtime context, auth, temporary storage, data separation, background jobs, or client/server boundaries.
- Read `../../references/platform/runtime-diagnostics.md` when a platform question is really about a startup/runtime failure and needs evidence before an answer.
- Do not give a platform answer from memory when version, mode, or context can change the behavior. Resolve that first, then answer.

## Stop rules

- Do not present a `development-standard` section as proof of platform API behavior or exact method signatures.
- Справка отвечает, что и с какими типами вызывать. Целостное описание механизма — за пределами источника: сообщите границу источника вместо ответа по памяти.
- Если секция вернула `unavailable` с причиной `version-missing`, назовите, какой установки или версии документа не хватает (отказ перечисляет доступные). Не подставляйте справку соседней версии.
- Если секция вернула `unavailable` с причиной `policy-denied` — сетевой выход запрещён политикой `unica.toml` самим пользователем. Назовите это решением проекта, а не сбоем, и отвечайте из оставшихся секций.
- Если ни один поставщик не дал подтверждения, сообщите `platform-help contract gap` и назовите требуемую версию и контекст.

## MCP examples

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.docs",
    "arguments": {
      "cwd": "<workspace>",
      "query": "СтрНайти",
      "source": "platform-help"
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.docs",
    "arguments": {
      "cwd": "<workspace>",
      "query": "как удалить элемент массива"
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.task.result",
    "arguments": { "taskId": "<taskId из ответа unica.docs>", "waitMs": 7000 }
  }
}
```

