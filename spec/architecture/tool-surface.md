# Ведомость публичной поверхности инструментов

Порождается `scripts/ci/generate-tool-surface.py` из `tools/list` собранного бинаря. Руками правится только [`tool-surface-review.json`](tool-surface-review.json): контракт результата и сценарии. Имена, описания и аргументы принадлежат реестру в `crates/unica-coder/src/application/mod.rs` и `tool_contracts.rs`; здесь они лишь показаны рядом (`INV-DOC-SINGLE-RULE-OWNER`).

Колонка «Результат сейчас» — наблюдение ревью, а не машинный факт: страж проверяет полноту охвата и совпадение аргументов с реестром, но не читает поведение обработчика.

## Итог

- Инструментов: **71**
- Отвечают типизированным `data`: **43**
- Типизированы частично: часть результата всё ещё текст: **1**
- Отвечают снимком задания в `job`: **6**
- Отвечают прозой в `stdout`: **21**

- В границах типизации: **43**
- Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`): **16**
- Вне границ: семейство runtime и build изучается отдельно: **12**
- Осталось перевести на типизированный `data` в границах работы: **0**
- Публикуют больше 20 аргументов из общего списка: **35**

## build — сборка и запуск платформы

### `unica.build.dump`

Dump source set through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить выгрузка набора исходников через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.load`

Load/build XML source set through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить загрузка исходников в базу через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.make`

Create CF/CFE artifact through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить сборка CF/CFE через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.run`

Launch 1C runtime or Designer through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить запуск платформы или конфигуратора через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.update`

Apply built configuration changes through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить применение изменений конфигурации через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

## cf — корень конфигурации

### `unica.cf.edit`

Edit root Configuration.xml properties, ChildObjects, panels, and home page.

Публикует **159** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: каждая операция с признаком применения и причиной пропуска, счётчики, факт перезаписи и валидации (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Зарегистрировать новый объект в составе конфигурации
- Переключить роли по умолчанию или стартовую страницу

### `unica.cf.info`

Inspect root Configuration.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ConfigPath` | string | да | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: идентичность, поддержка, свойства корня, состав и начальная страница (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Оценить размер и состав конфигурации перед началом работы
- Проверить режим совместимости и версию платформы

### `unica.cf.init`

Create empty 1C configuration XML scaffold.

Публикует **162** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: имя конфигурации, корень и созданные файлы заготовки (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Создать пустую конфигурацию для эксперимента или теста

### `unica.cf.validate`

Validate root configuration XML structure.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ConfigPath` | string | да | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить корень после ручной правки Configuration.xml

## cfe — расширения конфигурации

### `unica.cfe.borrow`

Borrow configuration objects/forms into an extension.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ExtensionPath` | string | да | Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead |
| `ConfigPath` | string | да | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `Object` | string | да | On unica.runtime.execute this is one metadata object name for operation dump with mode partial, written in colon form such as Catalog:Номенклатура (use objects for several); on the native XML tools Object is instead the dotted metadata reference the tool acts on, such as Catalog.Контрагенты.Form.ФормаЭлемента for unica.cfe.borrow. |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: перенесённые объекты и формы, что подтянулось автоматически, что оставлено без изменений и почему (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Заимствовать форму для доработки без снятия с поддержки
- Перехватить объект конфигурации в расширении

### `unica.cfe.diff`

Inspect extension contents and transferred insertion blocks.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ConfigPath` | string | да | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `ExtensionPath` | string | да | Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: состав расширения со статусом каждого объекта, перехватчики и проверка переноса вставок (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Понять, что уже содержит расширение
- Проверить, перенесены ли вставки в основную конфигурацию перед снятием расширения

### `unica.cfe.init`

Create extension XML scaffold.

Публикует **157** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: свойства расширения, источник каждого выведенного свойства (база или умолчание) и созданные файлы (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Создать расширение для доработки поставляемой конфигурации

### `unica.cfe.patch_method`

Generate a CFE Before/After interceptor for a caller-verified existing parameterless procedure on a registered adopted object.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ExtensionPath` | string | да | Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead |
| `ModulePath` | string | да | `unica.cfe.patch_method` only: dotted module reference such as `Catalog.X.ObjectModule`, `CommonModule.X` or `Document.X.Form.Y` — a metadata path, not a filesystem path |
| `MethodName` | string | да | `unica.cfe.patch_method` only: name of the existing parameterless procedure to intercept; must match a 1C identifier (Latin or Cyrillic letter or underscore, then letters, digits, underscores) |
| `InterceptorType` | string | да | `unica.cfe.patch_method` only: `"Before"` to generate a `&Перед` interceptor or `"After"` for `&После` |

Публикует **161** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: модуль и признак его создания, декоратор, метод, процедура, директива компиляции и переключённый дескриптор (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Сгенерировать Before-перехватчик для существующей процедуры

### `unica.cfe.validate`

Validate extension XML structure.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ExtensionPath` | string | да | Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить расширение перед сборкой CFE

## code — код BSL

### `unica.code.definition`

Find BSL method definitions through the typed Unica code index boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `moduleHint` | string | нет | Substring of a module path or object name that narrows unica.code.definition when the same method name exists in several modules; matched case-insensitively |
| `name` | string | да | Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `meta.info`, `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section` |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: определения с файлом, строкой, видом, параметрами и признаком экспорта (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Найти, где объявлен экспортный метод, вызванный из формы
- Отличить одноимённые методы в разных общих модулях

### `unica.code.diagnostics`

Run BSL diagnostics through the internal code analysis adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `codes` | array | нет | Array of diagnostic codes such as "АПК:142" or "LineLength"; on standards.explain it selects diagnostics mode and outranks snippet/id/query, on code.diagnostics it filters the catalog, and standards.search ignores it. |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `detail` | string | нет | How much detail to return, with a per-tool enum: names, signatures or bodies for unica.code.graph; concise or detailed for unica.code.diagnostics |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `maxFiles` | integer | нет | Integer cap on how many files one unica.code.diagnostics read covers, forwarded to the analyzer as max_files |
| `minSeverity` | string | нет | Lowest diagnostic severity unica.code.diagnostics should report: error, warning, info, or hint |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `rangeEnd` | integer | нет | Integer end of the source line range for unica.code.diagnostics, forwarded as range_end; pair it with rangeStart to scope a mode=file read |
| `rangeStart` | integer | нет | Integer start of the source line range for unica.code.diagnostics, forwarded as range_start; pair it with rangeEnd to scope a mode=file read |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `timeoutSeconds` | integer | нет | Only supported for mode analyze. Defaults to 120 seconds. |

**Результат сейчас:** `data`: ответ MCP анализатора как есть, тем же путём, что `code.graph`. `analyze` — имя инструмента анализатора, а не внешний процесс 1С, поэтому исключение ADR-0023 §4 на него не распространяется (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Прогнать диагностики по изменённому модулю перед коммитом
- Объяснить, почему BSL LS ругается на конструкцию

### `unica.code.graph`

Inspect BSL call graph through the typed Unica code analysis boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `detail` | string | нет | How much detail to return, with a per-tool enum: names, signatures or bodies for unica.code.graph; concise or detailed for unica.code.diagnostics |
| `dir` | string | нет | Edge direction to follow on unica.code.graph - in, out, or both; applies to the traversal modes such as neighbors, callers, and callees |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `edgeKinds` | array | нет | Array of graph edge-kind names, forwarded to the analyzer as edge_kinds; unica.code.graph only, and the Unica contract does not enumerate the accepted values |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `ids` | array | нет | Array of code-graph node ids for unica.code.graph, forwarded as ids alongside the single-node id argument; use it when one request targets several nodes |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `maxOutputTokens` | integer | нет | Integer output budget for unica.code.graph, forwarded as max_output_tokens; use it to keep a large graph answer within context |
| `mode` | string | да | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `provenance` | array | нет | Array of provenance filter values forwarded to the analyzer as provenance; unica.code.graph only, and the Unica contract does not enumerate the accepted values |
| `query` | string | нет | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: ответ анализатора с узлами и рёбрами графа как есть (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Проследить, кто вызывает метод, который планируется удалить
- Найти цикл вызовов между общими модулями

### `unica.code.outline`

Read compact BSL module outline from the current source file.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `includeMethods` | boolean | нет | Boolean for unica.code.outline controlling whether method entries appear in the outline; defaults to true |
| `path` | string | да | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений (эталон ADR-0020)

**Сценарии:**

- Получить экспортный интерфейс общего модуля перед написанием вызова
- Проверить сигнатуру процедуры до генерации перехватчика CFE

### `unica.code.patch`

Insert content into one logically addressed existing Platform XML Configuration or Extension BSL module.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `content` | string | да | BSL text for unica.code.patch: inserted at the selector for operation insert, or written over the selected method or anchor for operation replace |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `metadataPath` | string | да | Canonical logical module address inside sourceSet, for example CommonModule.Service.Module or Catalog.Items.ObjectModule. |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; the metadata edit verbs for unica.meta.edit — read the enum published in the tool's own schema. |
| `position` | string | нет | Where unica.code.patch places the content relative to the selector: before or after |
| `selector` | object | да | Object naming the unica.code.patch insertion point: exactly one of {"method": "Name"} for a whole procedure or function, or {"anchor": "text"} for a fragment that occurs once inside one method |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Вставить обработчик после существующего метода в модуле конфигурации
- Заменить тело метода целиком, сохранив соседей побайтово

### `unica.code.search`

Search code concurrently through provider-local RLM, bsl-analyzer, and literal git-grep sections. Migration: use sourceDir instead of the former path/config fields and a per-provider limit from 1 to 50.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `query` | string | да | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: три секции поставщиков с попаданиями, диагностикой и статусом (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Найти вызовы метода по всей конфигурации перед его изменением
- Оценить масштаб правки: сколько мест затронет переименование

## dcs — схемы компоновки данных

### `unica.dcs.compile`

Compile Data Composition Schema XML from JSON DSL.

Публикует **161** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Собрать СКД из JSON-описания

### `unica.dcs.edit`

Edit Data Composition Schema Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: операция, набор данных и вариант, по каждому значению признак применения с причиной, факт перезаписи и валидации (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить поле и итог в существующую СКД

### `unica.dcs.info`

Inspect Data Composition Schema Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: наборы данных с полями и точным текстом запроса, связи, вычисляемые поля, ресурсы, параметры, варианты настроек и макеты — все секции сразу (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Разобрать источник данных отчёта перед его правкой
- Достать текст запроса набора данных

### `unica.dcs.validate`

Validate Data Composition Schema Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить СКД после правки текста запроса

## epf — внешние обработки

### `unica.epf.init`

Create a make-ready external data processor scaffold in a Designer/platform-XML external source-set, optionally with a managed form.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormName` | string | нет | Name of the managed form as a 1C identifier: the form to create in `unica.form.add`, `epf.init` and `erf.init`, or the form to delete in `unica.form.remove` |
| `Name` | string | да | Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `meta.info`, `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section` |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile` |
| `Synonym` | string | нет | Human-readable synonym written into the generated XML; it defaults to the matching `name`, `formName` or `templateName` when omitted |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: созданные файлы заготовки внешней обработки (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Создать заготовку внешней обработки с формой

## erf — внешние отчёты

### `unica.erf.init`

Create a make-ready external report scaffold in a Designer/platform-XML external source-set, optionally with a managed form.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormName` | string | нет | Name of the managed form as a 1C identifier: the form to create in `unica.form.add`, `epf.init` and `erf.init`, or the form to delete in `unica.form.remove` |
| `Name` | string | да | Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `meta.info`, `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section` |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile` |
| `Synonym` | string | нет | Human-readable synonym written into the generated XML; it defaults to the matching `name`, `formName` or `templateName` when omitted |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: созданные файлы заготовки внешнего отчёта (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Создать заготовку внешнего отчёта

## form — управляемые формы

### `unica.form.add`

Add managed form metadata and files.

Публикует **160** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: объект, имя формы, дескриптор регистрации, свойство формы по умолчанию и созданные файлы (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить объекту пустую форму списка

### `unica.form.compile`

Compile managed Form.xml from JSON DSL or metadata.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `OutputPath` | string | да | Path of the single file to generate: the `Form.xml` for `unica.form.compile` or the `Template.xml` for `unica.dcs.compile` and `unica.mxl.compile` |

Публикует **158** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Сгенерировать форму по описанию или по пресету объекта

### `unica.form.edit`

Edit managed Form.xml elements, attributes, commands, and validated events.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormPath` | string | да | Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: удалённые узлы с причиной, добавленные элементы, реквизиты, команды и обработчики событий, факт изменения и валидации (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить поле на существующую форму
- Подписать обработчик события к элементу

### `unica.form.info`

Inspect managed Form.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormPath` | string | да | Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd` |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: свойства, события, полное дерево элементов без сворачивания, реквизиты с колонками, параметры и команды (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Изучить форму перед написанием её модуля
- Найти имя элемента для программного обращения

### `unica.form.remove`

Remove a managed form and registration.

Публикует **162** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: удалённые пути формы и обновлённый дескриптор объекта (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Удалить неиспользуемую форму вместе с регистрацией

### `unica.form.validate`

Validate managed Form.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormPath` | string | да | Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить форму после генерации или ручной правки

## help — встроенная справка

### `unica.help.add`

Add built-in help metadata and page to a 1C object.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ObjectName` | string | да | Name of the owning object for `unica.form.remove` and `unica.template.add`/`remove`; for `unica.help.add` it is instead the object's path under `srcDir`, e.g. `Catalogs/МойСправочник` |

Публикует **162** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: созданные файлы справки и обновлённые дескрипторы (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить объекту встроенную справку на русском

## interface — командный интерфейс

### `unica.interface.edit`

Edit subsystem CommandInterface.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `CIPath` | string | да | The `CIPath` spelling of the command-interface path: a subsystem's `Ext/CommandInterface.xml` or its directory, relative to `cwd`, for `unica.interface.edit`/`validate` |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: `added`, `removed`, `modified` и `mutation` с обновлённым файлом (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Скрыть команду из интерфейса подсистемы

### `unica.interface.validate`

Validate CommandInterface.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `CIPath` | string | да | The `CIPath` spelling of the command-interface path: a subsystem's `Ext/CommandInterface.xml` or its directory, relative to `cwd`, for `unica.interface.edit`/`validate` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить интерфейс после настройки видимости

## meta — объекты метаданных

### `unica.meta.compile`

Compile metadata object XML from JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `JsonPath` | string | да | Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.meta.compile`, `unica.mxl.compile` and `unica.role.compile` |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile` |

Публикует **161** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Создать справочник из JSON-описания
- Сгенерировать регистр сведений с измерениями и ресурсами

### `unica.meta.edit`

Edit metadata object XML.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ObjectPath` | string | да | Path to an object's metadata XML — a directory resolves to `<name>/<name>.xml` — for `unica.meta.edit`/`validate` and `unica.form.add`, relative to `cwd`; `meta.validate` accepts several joined by `\|`. `unica.meta.info` takes `sourceSet` + `metadataPath` instead |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: объект, признак изменения, счётчики добавленного/удалённого/изменённого и проектируемый diff (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить реквизит существующему документу
- Назначить владельцев подчинённому справочнику

### `unica.meta.info`

Inspect metadata object XML.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `metadataPath` | string | да | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** `data`: адрес, вид, имя, синоним, пообъектная поддержка, свойства именами платформы, владельцы, реквизиты/измерения/ресурсы, ТЧ, значения перечисления, формы, макеты, команды (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Изучить структуру справочника перед написанием запроса
- Сравнить два объекта по подчинению и составу реквизитов
- Уточнить длину кода и основное представление перед генерацией формы

### `unica.meta.profile`

Read compact metadata object profile from the internal RLM index.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `name` | string | да | Name of the object being created (`cf.init`, `cfe.init`, `epf.init`, `erf.init`), or the drill-down target for `meta.info`, `subsystem.info` and `dcs.info`; on `cf.info` it is an alias of `section` |
| `sections` | array | нет | Array of profile sections unica.meta.profile returns, from structure, modules, roles, subscriptions, functionalOptions, predefinedItems; omit it for all sections except predefinedItems, which must be listed explicitly |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on the path-based unica.code.* tools and unica.meta.profile it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: секции профиля со статусом, счётчиками и элементами в их собственной форме (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Быстро получить сводку объекта из индекса RLM без чтения XML
- Узнать, какие подписки и функциональные опции связаны с объектом

### `unica.meta.remove`

Remove metadata object XML and registration.

Публикует **162** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: вид и имя объекта, признак пробного прогона, вычищенные подсистемы и удалённые пути (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Удалить устаревший объект вместе с регистрацией в Configuration.xml
- Проверить предпросмотром, что ещё ссылается на объект

### `unica.meta.validate`

Validate metadata object XML.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ObjectPath` | string | да | Path to an object's metadata XML — a directory resolves to `<name>/<name>.xml` — for `unica.meta.edit`/`validate` and `unica.form.add`, relative to `cwd`; `meta.validate` accepts several joined by `\|`. `unica.meta.info` takes `sourceSet` + `metadataPath` instead |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить объект после ручной правки XML
- Прогнать пакетную проверку набора объектов перед сборкой

## mxl — табличные макеты

### `unica.mxl.compile`

Compile spreadsheet Template.xml from JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `JsonPath` | string | да | Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.meta.compile`, `unica.mxl.compile` and `unica.role.compile` |
| `OutputPath` | string | да | Path of the single file to generate: the `Form.xml` for `unica.form.compile` or the `Template.xml` for `unica.dcs.compile` and `unica.mxl.compile` |

Публикует **161** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Собрать печатную форму из JSON-описания

### `unica.mxl.decompile`

Decompile spreadsheet Template.xml to JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |

Публикует **157** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Получить редактируемое описание готового макета

### `unica.mxl.info`

Inspect spreadsheet Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SrcDir` | string | нет | Directory holding `<objectName>.xml`, default `src`; for `unica.form.remove` and `unica.template.add`/`remove` point it at the type folder such as `src/Reports`, and `unica.mxl.info`/`help.add` use it too |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `WithText` | boolean | нет | `unica.mxl.info` only: boolean including static cell text and template strings with `[Parameter]` substitutions in the report |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `withText` | boolean | нет | `unica.mxl.info` only: boolean including static cell text and template strings with `[Parameter]` substitutions in the report |

**Результат сейчас:** `data`: области с границами и параметрами, наборы колонок, содержимое вне областей и счётчики (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Узнать заполняемые параметры печатной формы перед написанием печати
- Построить пересечения строчных и колоночных областей для `ПолучитьОбласть`
- Достать текст ячеек макета вместе с параметрами через `WithText`

### `unica.mxl.validate`

Validate spreadsheet Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | да | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить макет после сборки

## project — рабочее пространство

### `unica.project.map`

Inspect configured source sets and effective source format per source set.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: карта наборов исходников (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Узнать имена наборов исходников перед любым логическим вызовом (`sourceSet`)
- Проверить, в каком формате лежит набор — Platform XML или EDT — до попытки правки
- Разобраться, почему инструмент выбрал не тот корень исходников

### `unica.project.status`

Inspect current Unica workspace, source set, and cache state.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: корни рабочего пространства и наборы исходников (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Понять, готов ли workspace к работе после клонирования
- Выяснить, устарел ли BSL-индекс перед серией поисковых вызовов

## role — роли и права

### `unica.role.compile`

Compile role metadata and Rights.xml from JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `JsonPath` | string | да | Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.meta.compile`, `unica.mxl.compile` and `unica.role.compile` |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile` |

Публикует **161** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Создать роль из описания прав

### `unica.role.info`

Inspect role Rights.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `RightsPath` | string | да | Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd` |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: разрешённые и запрещённые права по видам объектов, RLS, шаблоны и поддержка (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Проверить, какие права даёт роль перед её выдачей
- Найти объекты с ограничением на уровне записей

### `unica.role.validate`

Validate role Rights.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `RightsPath` | string | да | Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить роль после правки Rights.xml

## runtime — выполнение и задания

### `unica.runtime.execute`

Execute typed v8-runner runtime workflows through the single Unica MCP boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; the metadata edit verbs for unica.meta.edit — read the enum published in the tool's own schema. |

Публикует **64** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data` по операции (типизированы частично: часть результата всё ещё текст)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Прогнать синтаксическую проверку конфигурации
- Запустить модульные тесты YAxUnit в базе

### `unica.runtime.job.cancel`

Request safe cancellation for a durable runtime job.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Отменить зависшее задание

### `unica.runtime.job.list`

List durable runtime job snapshots in the current workspace.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Перечислить задания рабочего пространства

### `unica.runtime.job.logs`

Read bounded redacted stdout and stderr tails for a durable runtime job.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |
| `tailChars` | integer | нет | Integer 1..32768 bounding how many trailing characters of stdout and stderr unica.runtime.job.logs returns, defaulting to 4096 |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Прочитать хвост логов задания после падения

### `unica.runtime.job.start`

Start a durable typed v8-runner runtime job without changing runtime.execute.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; the metadata edit verbs for unica.meta.edit — read the enum published in the tool's own schema. |

Публикует **61** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Запустить длительную операцию, не блокируя сессию

### `unica.runtime.job.status`

Read a durable runtime job snapshot by jobId.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Узнать состояние запущенного задания

### `unica.runtime.job.wait`

Wait for a durable runtime job with a caller-side bounded timeout.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |
| `timeoutSeconds` | integer | нет | Integer seconds bounding a blocking call: 1..60 (default 30) for unica.runtime.job.wait, and 30..3600 (default 120) for unica.code.diagnostics, which accepts it only with mode analyze. |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Дождаться завершения задания с ограниченным таймаутом

## source — логическая адресация и ресурсы

### `unica.source.children`

List exactly one level below a logical source-set root or metadata address.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `metadataPath` | string | нет | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Обойти дерево метаданных на один уровень вниз от корня набора
- Перечислить формы объекта, не читая каталог `Forms/`

### `unica.source.locate`

Recover the logical metadata address that owns one source path inside a named source set.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `path` | string | да | Source file to look up, given either workspace-relative or relative to the named source set; the answer names the metadata address that owns it |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Перевести путь из вывода grep или git diff в логический адрес
- Узнать, какому объекту принадлежит найденный файл модуля

### `unica.source.read`

Read one bounded byte range from a resource in an issued immutable snapshot.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `offset` | integer | нет | Zero-based byte offset inside the immutable resource snapshot |
| `resourceId` | string | да | Opaque resource identifier returned inside one source.resources snapshot; valid only together with the snapshotId that issued it |
| `snapshotId` | string | да | Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Прочитать байты модуля кусками по 64 КиБ с сохранением BOM и профиля EOL
- Достать фрагмент двоичного макета в base64

### `unica.source.resolve`

Resolve an exact or prefix logical metadata query inside one named source set.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `query` | string | да | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `targetKind` | string | нет | Optional `unica.source.resolve` filter: `metadataObject` or `module`; it narrows exact or prefix matches without changing their canonical metadataPath |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Найти объект по русскому имени и получить канонический адрес для следующих вызовов
- Проверить, существует ли объект, не зная раскладки выгрузки
- Разрешить префикс `Справочник.` в ограниченный список кандидатов

### `unica.source.resources`

Open or page an immutable bounded manifest for one logical source target.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `metadataPath` | string | нет | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `scope` | string | нет | Bounded source.resources manifest scope: self, aggregate, or registrations |
| `snapshotId` | string | нет | Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Получить манифест ресурсов объекта: дескриптор и его модули
- Открыть неизменяемый снимок перед серией ограниченных чтений

## standards — стандарты 1С

### `unica.standards.explain`

Explain 1C diagnostics or standards through the internal standards adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `bodyLimit` | string | нет | Max page-body size for `unica.standards.explain` when it fetches a standard by `id`/`idOrAliasOrUrl`; the XML/DSL tools accept the key but never read it |
| `body_limit` | string | нет | Maximum size of the standard page body returned by unica.standards.explain in page mode (snake_case alias of bodyLimit); honoured only alongside id/idOrAliasOrUrl, and ignored by standards.search. |
| `codes` | array | нет | Array of diagnostic codes such as "АПК:142" or "LineLength"; on standards.explain it selects diagnostics mode and outranks snippet/id/query, on code.diagnostics it filters the catalog, and standards.search ignores it. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `idOrAliasOrUrl` | string | нет | Standard number, alias or full URL (e.g. "644") that puts standards.explain in page-fetch mode; prefer it over id, which it overrides when both are passed, and standards.search ignores it. |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `query` | string | нет | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `snippet` | string | нет | Literal BSL source text for standards.explain to explain against standards, sent with language and limit; codes outranks it when both are passed, and standards.search ignores it. |
| `types` | array | нет | Array of strings forwarded unchanged as the types parameter of the standards search; honoured only by standards.search and by standards.explain given query alone, with no allowed values declared. |

**Результат сейчас:** `data`: стандарт или диагностика из удалённого MCP как есть (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Раскрыть смысл кода диагностики из отчёта проверки
- Прочитать стандарт целиком по его идентификатору

### `unica.standards.search`

Search 1C standards through the internal standards adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `bodyLimit` | string | нет | Max page-body size for `unica.standards.explain` when it fetches a standard by `id`/`idOrAliasOrUrl`; the XML/DSL tools accept the key but never read it |
| `body_limit` | string | нет | Maximum size of the standard page body returned by unica.standards.explain in page mode (snake_case alias of bodyLimit); honoured only alongside id/idOrAliasOrUrl, and ignored by standards.search. |
| `codes` | array | нет | Array of diagnostic codes such as "АПК:142" or "LineLength"; on standards.explain it selects diagnostics mode and outranks snippet/id/query, on code.diagnostics it filters the catalog, and standards.search ignores it. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `idOrAliasOrUrl` | string | нет | Standard number, alias or full URL (e.g. "644") that puts standards.explain in page-fetch mode; prefer it over id, which it overrides when both are passed, and standards.search ignores it. |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained |
| `limit` | integer | нет | Output cap for the tool being called: maximum printed lines, default 150, for the paginating XML readers (cf.info, meta.info, form.info, dcs.info, subsystem.info, role.info, mxl.info); elsewhere it caps returned results with per-tool defaults (code.search 20 per provider, code.definition 50, meta.profile 20, code.graph nodes, code.diagnostics findings, standards results). |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `query` | string | да | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `snippet` | string | нет | Literal BSL source text for standards.explain to explain against standards, sent with language and limit; codes outranks it when both are passed, and standards.search ignores it. |
| `types` | array | нет | Array of strings forwarded unchanged as the types parameter of the standards search; honoured only by standards.search and by standards.explain given query alone, with no allowed values declared. |

**Результат сейчас:** `data`: результат удалённого MCP стандартов как есть, без JSON-RPC конверта (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Найти стандарт 1С по теме перед проектированием API
- Проверить, есть ли норматив на спорное решение

## subsystem — подсистемы

### `unica.subsystem.compile`

Compile subsystem XML from JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `meta.compile`/`role.compile`/`subsystem.compile` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Добавить новый раздел в конфигурацию

### `unica.subsystem.edit`

Edit subsystem XML content and hierarchy.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | да | Path to a subsystem's XML, its directory, or the whole `Subsystems/` folder for `Mode=tree`, used by `unica.subsystem.info`/`edit`/`validate`, relative to `cwd` |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: каждая операция с признаком применения, причиной пропуска и нормализованной ссылкой, счётчики, созданные заготовки и факт валидации (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Включить объект в подсистему

### `unica.subsystem.info`

Inspect subsystem XML and command interface.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | да | Path to a subsystem's XML, its directory, or the whole `Subsystems/` folder for `Mode=tree`, used by `unica.subsystem.info`/`edit`/`validate`, relative to `cwd` |
| `confirm` | boolean | нет | Boolean acknowledgement accepted by every tool and stripped before the runner is called; it does not enable execution on its own, dryRun false does |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch present on every tool; when omitted it defaults to true for mutating tools, which then only report the command they would run, and to false for read-only tools, so send false explicitly only on a mutating tool and only when the user asked for execution. |

**Результат сейчас:** `data`: состав, группы, дочерние подсистемы и командный интерфейс, либо дерево иерархии для каталога (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Понять границы подсистемы перед добавлением объекта
- Прочитать видимость и размещение команд подсистемы
- Построить дерево подсистем конфигурации по каталогу `Subsystems`

### `unica.subsystem.validate`

Validate subsystem XML.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | да | Path to a subsystem's XML, its directory, or the whole `Subsystems/` folder for `Mode=tree`, used by `unica.subsystem.info`/`edit`/`validate`, relative to `cwd` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить подсистему после правки состава

## support — поддержка поставщика

### `unica.support.edit`

Toggle 1C vendor support editing capability or per-object support rule.

Публикует **160** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: вид переключения, применённость с причиной, состояние правки, объект и правило, счётчики записей (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Снять объект с замка поставщика перед доработкой
- Вернуть объект на поддержку после отката правки

## template — макеты объектов

### `unica.template.add`

Add a template to an object and register it.

Публикует **162** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: созданные и обновлённые файлы макета (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить объекту пустой макет печатной формы

### `unica.template.remove`

Remove a template from an object.

Публикует **162** аргументов: обязательные — не объявлено ни одного, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: удалённые пути и обновлённый дескриптор объекта (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Удалить неиспользуемый макет
