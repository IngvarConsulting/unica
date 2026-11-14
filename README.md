<p align="center">
  <a href="https://ingvar.pro/products/unica">
    <img src="docs/visual-kit/logos/unica-logo-letter-transparent-blue.svg" alt="Unica" width="420">
  </a>
</p>

# Unica

Unica (Ю&#x301;ника) — плагин для [Codex](https://openai.com/codex/),
[Claude Code](https://code.claude.com/docs/en/overview) и ZCode для разработки на
1С:Предприятии. Он добавляет навыки и один MCP-сервер `unica`, через который
агент создаёт и проверяет метаданные, формы, роли, СКД, внешние обработки и
отчёты, запускает 1С и ищет BSL-код.

## Требования

- один из агентов:
  - [Codex CLI](https://learn.chatgpt.com/docs/codex/cli);
  - [Claude Code](https://code.claude.com/docs/en/overview);
  - ZCode Desktop с поддержкой `.zcode-plugin` (контракт проверяется для 3.14.4);
- стандартный Git, включая Git for Windows на Windows;
- платформа 1С только для операций, которым нужен запуск 1С.

### Поддерживаемые версии платформы 1С

| Версия платформы | Статус | Что это означает |
| --- | --- | --- |
| `8.5.4.x` | Поддерживается | Исходники в формате выгрузки 2.22 читаются и правятся в своём формате. Эталоны выгрузки сняты на 8.5.4.2028. |
| `8.5.1.x` | Поддерживается | Исходники в формате выгрузки 2.21 читаются и правятся в своём формате. Эталоны выгрузки сняты на 8.5.1.1529. |
| `8.3.27.x` | Поддерживается | Unica поддерживает все актуальные релизы ветки 8.3.27. |
| `8.3.26.x` и ниже | Не планируется | Помогаем мигрировать на 8.3.27. Если вам действительно нужна более старая версия, [создайте issue](https://github.com/IngvarConsulting/unica/issues/new) и опишите причину — нам важно понимать такой сценарий. |

Каждый набор исходников правится в своём формате выгрузки: Unica не переводит набор в другой формат без вашего выбора. Основа и расширение могут быть выгружены разными версиями одной линии — платформа загружает их вместе.

## Установка, обновление и удаление

После каждого из этих действий начните новую сессию агента: уже открытая
сессия не видит изменённые навыки и MCP-сервер. В Claude Code вместо этого
можно выполнить `/reload-plugins`.

### Codex

```sh
codex plugin marketplace add IngvarConsulting/unica-marketplace --ref main
codex plugin add unica@unica
```

Обновление: каталог обновляется, плагин переустанавливается.

```sh
codex plugin marketplace upgrade unica
codex plugin remove unica@unica
codex plugin add unica@unica
```

Удаление: `codex plugin remove unica@unica`, затем
`codex plugin marketplace remove unica`.

### Claude Code

```sh
claude plugin marketplace add IngvarConsulting/unica-marketplace
claude plugin install unica@unica
```

Обновление:

```sh
claude plugin marketplace update unica
claude plugin update unica@unica
```

Удаление: `claude plugin uninstall unica@unica`, затем
`claude plugin marketplace remove unica`. Навыки доступны с префиксом
плагина, например `/unica:meta-info`.

Скачанные ядро и движки можно оставить в кэше для повторной установки:
при обычном удалении плагина удалять их вручную не нужно.

### ZCode

1. В **Plugin Marketplace → Add → Add Plugin Marketplace** добавьте каталог
   выпуска с манифестом ZCode.
2. В **Personal** установите `unica`.

Чтобы обновить плагин, обновите каталог в источниках marketplace и нажмите **Update**
у плагина в **Personal**. Установленными плагинами управляет
**Settings → Plugins**. Оставьте включённой одну установку Unica.

Канал кандидатов `next`, установка конкретной версии по тегу, доставка ядра
и движков, каталоги кэша, офлайн-образ, сертификаты и прокси описаны в
[README плагина](plugins/unica/README.md).

## Ссылки

- [Справочник инструментов](docs/tool-surface.md) — инструменты `unica.*`
  и их аргументы.
- [Сайт проекта](https://ingvarconsulting.github.io/unica/) и
  [канал кандидатов `next`](https://ingvarconsulting.github.io/unica/next.html).
- [Разработка Unica](CONTRIBUTING.md) — локальная сборка, окружение
  и проверки.
- [Авторы, источники и лицензии](plugins/unica/ATTRIBUTIONS.md).
  Лицензия Unica: LGPL-3.0-or-later.
