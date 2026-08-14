---
name: platform-help
description: "Справка платформы 1С и объектной модели BSL. Используй когда нужно уточнить метод, свойство, конструктор, поведение API, версию платформы, совместимость или стандартное решение задачи."
---

# Platform Help

## MCP routing

- For platform API and mechanics, use MCP `unica` tool `unica.documentation.search` with `"sourceKinds": ["platform-help"]`: фильтр по смыслу источника оставляет справку платформы и не тратит сетевой вызов стандартов на вопрос, который им не является.
- For domain questions about the workspace configuration itself — назначение справочника, роль документа в учёте, — используйте `"sourceKinds": ["configuration-documentation"]`: отвечает встроенная справка конфигурации (корпус `configuration-help`, локатор `configuration-help:<набор-исходников>:<путь>`), а не справка платформы. `applicableVersion` такого попадания — версия конфигурации, не платформы.
- Секции приходят от четырёх поставщиков: встроенная справка конфигурации рабочего пространства, установка платформы (Синтакс-помощник и справка конфигуратора), руководства площадки вендора (`kb-developer-guide`, `kb-administrator-guide` — описательный слой: механизмы целиком, форматы адресов, администрирование) и сервер стандартов. Установка старше в интерфейсе программирования, руководства — в описательном; расхождение их версий называйте в ответе.
- Каждая секция несёт `sourceKind` и `authority`, каждое попадание в ней — `applicableVersion` и `documentId`. Ответ обязан называть источник, версию установки и `documentId` страницы: без него читатель не может вернуться к той же странице.
- `language` секции — локаль, которой источник ответил на самом деле, а не запрошенная. Если они расходятся, назовите подстановку локали в ответе: справка поставляется не во всех локалях, и запрос `en` на русскоязычной установке молча отвечал бы русскими страницами.
- Секция со смыслом источника `development-standard` не закрывает вопрос о сигнатуре или механике платформы, каким бы уместным ни выглядел её текст. Это правило чтения, а не правило вызова. Симметрично: секция `configuration-documentation` описывает прикладную конфигурацию и не доказывает поведение самой платформы.
- For project context, use `unica.code.search`, `unica.project.map`, and `unica.runtime.execute`.
- Текущий runtime-контракт: `unica.runtime.execute` — preview-only и вызывается только с `dryRun: true`; любой applied-режим возвращает fail-closed до workspace discovery и process spawn. Preview не является runtime verification. Не обходи этот отказ прямым runner-ом, через `unica.build.*` или fallback через `unica.runtime.job.*`.
- Use object-specific `unica.*.info` tools when the API question depends on metadata structure.
- Do not call internal standards, runtime, or package adapters directly.

## Workflow

1. State the exact platform/API question: object, method/property, platform version, infobase mode, client/server context.
2. Call `unica.documentation.search` with the object or member name — или с естественной формулировкой вопроса: поиск пословный, морфологический и нечёткий (ADR-0037), точная подстрока и порядок слов не требуются, опечатка в имени не прячет страницу.
3. Read `applicableVersion` in the hit. Если она расходится с версией проекта, назовите расхождение в ответе.
4. Подтвердите ответ текстом открытой страницы: передайте `documentId` попадания в `unica.documentation.get` дословно и опирайтесь на поле `text`. Заголовок и фрагмент выдачи доказательством не является — доказательство только текст документа.
5. Validate against local project context with `unica.project.map` and targeted `unica.code.search` if the answer depends on project conventions.
6. For code examples, use `unica.runtime.execute` only to preview `operation=syntax`; report actual syntax and runtime behavior as unverified.

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
    "name": "unica.documentation.search",
    "arguments": {
      "cwd": "<workspace>",
      "query": "СтрНайти",
      "sourceKinds": ["platform-help"],
      "limit": 10
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.documentation.search",
    "arguments": {
      "cwd": "<workspace>",
      "query": "ТаблицаЗначений.Свернуть",
      "platformVersion": "8.3.27.2074",
      "limit": 10
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.documentation.search",
    "arguments": {
      "cwd": "<workspace>",
      "query": "как удалить элемент массива",
      "sourceKinds": ["platform-help"],
      "limit": 10
    }
  }
}
```

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.documentation.get",
    "arguments": {
      "cwd": "<workspace>",
      "documentId": "platform-syntax-help:syntax-context:objects/catalog238/ValueTable/methods/GroupBy1290.html"
    }
  }
}
```
