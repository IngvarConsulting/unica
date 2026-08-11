# Ведомость публичной поверхности инструментов

Порождается `scripts/ci/generate-tool-surface.py` из `tools/list` собранного бинаря. Руками правится только [`tool-surface-review.json`](tool-surface-review.json): контракт результата и сценарии. Имена, описания и аргументы принадлежат реестру в `crates/unica-coder/src/application/mod.rs` и `tool_contracts.rs`; здесь они лишь показаны рядом (`INV-DOC-SINGLE-RULE-OWNER`).

Колонка «Результат сейчас» — наблюдение ревью, а не машинный факт: страж проверяет полноту охвата и совпадение аргументов с реестром, но не читает поведение обработчика.

## Итог

- Инструментов: **74**
- Отвечают типизированным `data`: **48**
- Типизированы частично: часть результата всё ещё текст: **1**
- Отвечают снимком задания в `job`: **6**
- Отвечают прозой в `stdout`: **19**

- В границах типизации: **48**
- Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`): **14**
- Вне границ: семейство runtime и build изучается отдельно: **12**
- Осталось перевести на типизированный `data` в границах работы: **0**
- Публикуют больше 20 аргументов из общего списка: **31**

## build — сборка и запуск платформы

### `unica.build.dump`

Dump source set through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить выгрузку набора исходников через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.load`

Load/build XML source set through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить загрузку исходников в базу через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.make`

Create CF/CFE artifact through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `sourceSet` | string | нет | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `target` | string | нет | String forwarded to unica.build.* as --target; the skills document no behaviour for it beyond the flag name |
| `user` | string | нет | String forwarded to unica.build.* as --user; the skills document no behaviour for it beyond the flag name |

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Выполнить сборку CF/CFE через единый MCP без ручной командной строки
- Проверить предпросмотром, что будет запущено, до фактического запуска

### `unica.build.run`

Launch 1C runtime or Designer through the internal build/runtime adapter.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `config` | string | нет | Workspace-relative path to v8project.yaml on unica.runtime.execute, unica.runtime.job.start and unica.build.* — the file to create for operation config-init and the existing project config for every other operation, never v8project.local.yaml; on unica.code.diagnostics `config` is a separate passthrough to the bsl-analyzer run and is not the project config. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `database` | string | нет | String forwarded to unica.build.* as --database with no behaviour documented in the skills; prefer connection on operation config-init when working through unica.runtime.execute |
| `dbPassword` | string | нет | String forwarded to unica.build.* as --db-password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `dbUser` | string | нет | String forwarded to unica.build.* as --db-user; the skills document no behaviour for it beyond the flag name |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `infobase` | string | нет | String forwarded to unica.build.* as --infobase with no behaviour documented in the skills; unica.runtime.execute has no such argument and reaches a database through connection at config-init |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `password` | string | нет | String forwarded to unica.build.* as --password and redacted in reported commands; undocumented in the skills, and credentials belong in v8project.local.yaml, not tool arguments |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
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
| `ConfigPath` | string | по ветви | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` **либо** `ConfigPath`. Ни одной или обе сразу отклоняются.

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
| `ConfigPath` | string | по ветви | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **158** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` **либо** `ConfigPath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить корень после ручной правки Configuration.xml

## cfe — расширения конфигурации

### `unica.cfe.borrow`

Borrow configuration objects/forms into an extension.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `ConfigPath` | string | да | Path to `Configuration.xml` or the dump directory for `unica.cf.edit`, `unica.cf.info` and `unica.cf.validate`, and the path of the base configuration for `unica.cfe.init`/`borrow`/`diff`; relative to `cwd`. `unica.cf.init` ignores it and writes to `outputDir`. |
| `ExtensionPath` | string | да | Path to the extension — its directory or its `Configuration.xml` — for every `unica.cfe.*` tool, relative to `cwd`; the base configuration goes in `configPath` instead |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |

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
| `InterceptorType` | string | да | `unica.cfe.patch_method` only: `"Before"` to generate a `&Перед` interceptor or `"After"` for `&После` |
| `MethodName` | string | да | `unica.cfe.patch_method` only: name of the existing parameterless procedure to intercept; must match a 1C identifier (Latin or Cyrillic letter or underscore, then letters, digits, underscores) |
| `ModulePath` | string | да | `unica.cfe.patch_method` only: dotted module reference such as `Catalog.X.ObjectModule`, `CommonModule.X` or `Document.X.Form.Y` — a metadata path, not a filesystem path |

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

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить расширение перед сборкой CFE

## code — код BSL

### `unica.code.definition`

Find BSL method definitions through the typed Unica code index boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `moduleHint` | string | нет | Substring of a module path or object name that narrows unica.code.definition when the same method name exists in several modules; matched case-insensitively |
| `name` | string | да | Subject name whose meaning is tool-scoped: the object being created by `cf.init`, `cfe.init`, `epf.init` and `erf.init`, the required BSL method to locate on `unica.code.definition`, and the XDTO type or property named by the chosen `unica.xdto.edit` operation. The eight narrowed native XML readers no longer take it: they answer with every section at once, so there is nothing left for it to drill into (ADR-0048). |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: определения с обязательными файлом и строкой; вид, параметры и признак экспорта равны `null`, когда индекс их не сообщил (ADR-0023) (отвечают типизированным `data`)

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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `detail` | string | нет | How much detail to return, with a per-tool enum: names, signatures or bodies for unica.code.graph; concise or detailed for unica.code.diagnostics |
| `format` | string | нет | On unica.runtime.execute this is the source format (designer or edt) recorded by config-init and no other runtime operation accepts it; on unica.code.* and the native XML tools `format` selects the report/output format instead (for example text, json or jsonl), and on unica.build.* it is an undocumented --format passthrough. |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `maxFiles` | integer | нет | Integer cap on how many files one unica.code.diagnostics read covers, forwarded to the analyzer as max_files |
| `minSeverity` | string | нет | Lowest diagnostic severity unica.code.diagnostics should report: error, warning, info, or hint |
| `mode` | string | нет | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `path` | string | нет | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `rangeEnd` | integer | нет | Integer end of the source line range for unica.code.diagnostics, forwarded as range_end; pair it with rangeStart to scope a mode=file read |
| `rangeStart` | integer | нет | Integer start of the source line range for unica.code.diagnostics, forwarded as range_start; pair it with rangeEnd to scope a mode=file read |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |
| `timeoutSeconds` | integer | нет | Only supported for mode analyze. Overrides operational.code_diagnostics.analyze_timeout_seconds from workspace config, whose compiled fallback is 120 seconds. |

**Результат сейчас:** `data`: ответ MCP анализатора как есть, тем же путём, что `code.graph`. `analyze` — имя инструмента анализатора, а не внешний процесс 1С, поэтому исключение ADR-0023 §4 на него не распространяется (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Прогнать диагностики по изменённому модулю перед коммитом
- Объяснить, почему BSL LS ругается на конструкцию

### `unica.code.graph`

Inspect BSL call graph through the typed Unica code analysis boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `detail` | string | нет | How much detail to return, with a per-tool enum: names, signatures or bodies for unica.code.graph; concise or detailed for unica.code.diagnostics |
| `dir` | string | нет | Edge direction to follow on unica.code.graph - in, out, or both; applies to the traversal modes such as neighbors, callers, and callees |
| `edgeKinds` | array | нет | Array of graph edge-kind names, forwarded to the analyzer as edge_kinds; unica.code.graph only, and the Unica contract does not enumerate the accepted values |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `ids` | array | нет | Array of code-graph node ids for unica.code.graph, forwarded as ids alongside the single-node id argument; use it when one request targets several nodes |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `maxOutputTokens` | integer | нет | Integer output budget for unica.code.graph, forwarded as max_output_tokens; use it to keep a large graph answer within context |
| `mode` | string | да | Tool-scoped mode selector: on unica.runtime.execute and unica.runtime.job.start it is full\|incremental\|partial for dump, load\|merge for load, designer-config\|designer-modules\|edt for syntax, and the client kind for an mcp or mcp-va launch, while every other tool defines its own values (for example analyze\|status\|catalog\|file\|workspace on unica.code.diagnostics) — always use the enum published in that tool's own schema. |
| `provenance` | array | нет | Array of provenance filter values forwarded to the analyzer as provenance; unica.code.graph only, and the Unica contract does not enumerate the accepted values |
| `query` | string | нет | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** `data`: ответ анализатора с узлами и рёбрами графа как есть (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Проследить, кто вызывает метод, который планируется удалить
- Найти цикл вызовов между общими модулями

### `unica.code.outline`

Read compact BSL module outline from the current source file.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `includeMethods` | boolean | нет | Boolean for unica.code.outline controlling whether method entries appear in the outline; defaults to true |
| `path` | string | да | Workspace-relative file path whose meaning is tool-scoped: the required .cf or .cfe artifact for unica.runtime.execute operation load (.epf and .erf are rejected there), a module-relative file for the path-based unica.code.* tools — on unica.code.diagnostics only mode `file` reads one file, so every other mode rejects `path` instead of ignoring it — the canonical alias of the object/config path argument on the native XML tools, and a plain --path passthrough on unica.build.*. |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений (эталон ADR-0020)

**Сценарии:**

- Получить экспортный интерфейс общего модуля перед написанием вызова
- Проверить сигнатуру процедуры до генерации перехватчика CFE

### `unica.code.patch`

Insert or replace BSL in one logically addressed Platform XML Configuration or Extension module.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `content` | string | да | BSL text for unica.code.patch: inserted at the selector for operation insert, appended to the end of the module when insert names no selector, or written over the selected method or anchor for operation replace |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `metadataPath` | string | да | Canonical logical module address inside sourceSet, for example CommonModule.Service.Module or Catalog.Items.ObjectModule. |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema. |
| `position` | string | нет | Where unica.code.patch places the content relative to the selector: before or after. Accepted only when insert names a selector |
| `selector` | object | нет | Optional object naming the unica.code.patch edit point: exactly one of {"method": "Name"} for a whole procedure or function, or {"anchor": "text"} for a fragment that occurs once inside one method. Required by replace; when insert omits it the content goes to the end of the module |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Результат сейчас:** типизированный `data` (отвечают типизированным `data`)

**Целевой контракт:** без изменений

**Сценарии:**

- Вставить обработчик после существующего метода в модуле конфигурации
- Заменить тело метода целиком, сохранив соседей побайтово
- Дописать метод в конец модуля, не называя соседа

### `unica.code.search`

Search code concurrently through provider-local RLM, bsl-analyzer, and literal git-grep sections. Migration: use sourceDir instead of the former path/config fields and a per-provider limit from 1 to 50.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `query` | string | да | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceDir` | string | нет | Workspace-relative source root to work in: on path-based unica.code.* tools it selects the configured Configuration source set and is required when the workspace has more than one, and on unica.build.* it is forwarded as --source-dir; unica.code.patch and unica.runtime.execute select sources by configured sourceSet name instead. |

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
| `TemplatePath` | string | по ветви | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `TemplatePath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** `data`: наборы данных с полями и точным текстом запроса, связи, вычисляемые поля, ресурсы, параметры, варианты настроек и макеты — все секции сразу (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Разобрать источник данных отчёта перед его правкой
- Достать текст запроса набора данных

### `unica.dcs.validate`

Validate Data Composition Schema Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | по ветви | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `TemplatePath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить СКД после правки текста запроса

## documentation — справка платформы и стандарты разработки

### `unica.documentation.get`

Fetch the full text of a documentation search hit by its documentId locator.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `documentId` | string | да | Stable locator of a unica.documentation.search hit, passed verbatim to unica.documentation.get to fetch the full document text: configuration-help:<source-set>:<path> for the workspace configuration's embedded help, platform-syntax-help:<corpus>:<path> for the installed platform's help, an absolute https://kb.1ci.com/... page address for the vendor knowledge base, and an https://v8std.ru/... address for a development standard; the provider that minted the locator is the only one that resolves it. |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` and `unica.documentation.get` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section and document reports the locale that actually answered |
| `platformVersion` | string | нет | Requested platform installation version for unica.documentation.search and unica.documentation.get, matched against an installation directory name exactly, for example 8.3.27.2074; when omitted the project's own tools.platform.version constrains the choice, and without that the numerically newest installation found under a configured platform root wins; a tools.platform.path pin names the installation directly instead of walking the roots, with the same version constraints applied to it. |

**Результат сейчас:** `data`: документ целиком с происхождением, локалью ответа, версией и полным текстом (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Подтвердить ответ текстом открытой страницы, а не фрагментом выдачи
- Прочитать стандарт разработки или главу руководства целиком по локатору попадания

### `unica.documentation.search`

Search the workspace configuration's embedded help, platform help, and development standards across documentation providers.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` and `unica.documentation.get` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section and document reports the locale that actually answered |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `platformVersion` | string | нет | Requested platform installation version for unica.documentation.search and unica.documentation.get, matched against an installation directory name exactly, for example 8.3.27.2074; when omitted the project's own tools.platform.version constrains the choice, and without that the numerically newest installation found under a configured platform root wins; a tools.platform.path pin names the installation directly instead of walking the roots, with the same version constraints applied to it. |
| `query` | string | да | Search text: provider-neutral query for unica.code.search, node-lookup text for unica.code.graph mode=resolve, the required unica.standards.search string, and explain's last-resort fallback |
| `sourceKinds` | array | нет | Optional filter of unica.documentation.search by source kind, not by provider id: an array of configuration-documentation, platform-help and/or development-standard; providers without a matching corpus are not polled and their sections are not published, an empty or omitted array means every kind, and an unknown value is refused rather than silently ignored. |

**Результат сейчас:** `data`: секции поставщиков документации с происхождением, локалью ответа и версией (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Уточнить сигнатуру и доступность метода платформы до написания кода
- Проверить поведение механизма платформы для конкретной версии установки
- Отличить справку платформы от стандарта разработки в одном ответе
- Найти главу руководства площадки вендора для закреплённой версии
- Найти встроенную справку объекта конфигурации рабочего пространства

## epf — внешние обработки

### `unica.epf.init`

Create a make-ready external data processor scaffold in a Designer/platform-XML external source-set, optionally with a managed form.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `FormName` | string | нет | Name of the managed form as a 1C identifier: the form to create in `unica.form.add`, `epf.init` and `erf.init`, or the form to delete in `unica.form.remove` |
| `Name` | string | да | Subject name whose meaning is tool-scoped: the object being created by `cf.init`, `cfe.init`, `epf.init` and `erf.init`, the required BSL method to locate on `unica.code.definition`, and the XDTO type or property named by the chosen `unica.xdto.edit` operation. The eight narrowed native XML readers no longer take it: they answer with every section at once, so there is nothing left for it to drill into (ADR-0048). |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `role.compile`/`subsystem.compile` |
| `Synonym` | string | нет | Human-readable synonym written into the generated XML; it defaults to the matching `name`, `formName` or `templateName` when omitted |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |

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
| `Name` | string | да | Subject name whose meaning is tool-scoped: the object being created by `cf.init`, `cfe.init`, `epf.init` and `erf.init`, the required BSL method to locate on `unica.code.definition`, and the XDTO type or property named by the chosen `unica.xdto.edit` operation. The eight narrowed native XML readers no longer take it: they answer with every section at once, so there is nothing left for it to drill into (ADR-0048). |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `role.compile`/`subsystem.compile` |
| `Synonym` | string | нет | Human-readable synonym written into the generated XML; it defaults to the matching `name`, `formName` or `templateName` when omitted |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |

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
| `FormPath` | string | по ветви | Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd` |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `FormPath`. Ни одной или обе сразу отклоняются.

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
| `FormPath` | string | по ветви | Path to an existing `Form.xml`, or the form directory that resolves to it, for `unica.form.info`, `unica.form.edit` and `unica.form.validate`, relative to `cwd` |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `FormPath`. Ни одной или обе сразу отклоняются.

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

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить интерфейс после настройки видимости

## meta — объекты метаданных

### `unica.meta.add`

Create one metadata object from a typed internal template and optionally configure it atomically with ordered operations.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `dryRun` | boolean | нет | Preview the mutation without writing workspace files. |
| `kind` | string | да | Supported metadata object kind for the minimal template. |
| `name` | string | да | Metadata object name using a valid 1C identifier. |
| `operations` | array | нет | Optional ordered typed operations applied to the private creation image before one atomic publication. |
| `sourceSet` | string | да | Exact Configuration source-set name from v8project.yaml. |

**Результат сейчас:** `structuredContent.data`: логический адрес, валидация, семантические `effects` и план атомарной публикации одного объекта из типизированного шаблона с необязательными ordered operations (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Создать минимальный справочник по логическому sourceSet
- Одним вызовом создать и настроить объект через общий типизированный `operations` union
- Создать подписку и атомарно задать совместимые `Source`, `Event` и `Handler`
- Создать поддерживаемый объект вместе с корневым предопределённым элементом
- Предварительно проверить план создания объекта без записи файлов

### `unica.meta.edit`

Apply ordered typed metadata edit operations atomically.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `dryRun` | boolean | нет | Preview the mutation without writing workspace files. |
| `metadataPath` | string | да | Logical metadata path of the object to edit. |
| `operations` | array | да | Ordered typed edit operations applied as one atomic change. |
| `sourceSet` | string | да | Exact Configuration source-set name from v8project.yaml. |

**Результат сейчас:** `structuredContent.data`: логический адрес, валидация, семантические `effects` по `operationIndex` и план атомарной публикации пяти вариантов typed-операций, включая проверку единой связки `EventSubscription.Source`/`Event`/`Handler` через `editRelations.source` и `setProperties` и коллекцию `predefinedItems` в `add`, `update`, `remove` (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Добавить реквизит существующему документу
- Назначить владельцев подчинённому справочнику
- Атомарно заменить источник, событие и обработчик существующей подписки
- Добавить, изменить или удалить предопределённый элемент по UUID

### `unica.meta.info`

Inspect one metadata object with validation, proven subsystem memberships, and source-tree usage.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `limit` | integer | нет | Maximum `predefinedItems` returned (1 through 50). Usage lists are read from the source tree, so they are exact and complete and the limit does not apply to them. |
| `metadataPath` | string | да | Logical metadata path of the object to inspect. |
| `sections` | array | нет | Extra sections to compute, all read from the source tree: `roles`, `subscriptions` and `functionalOptions` land in `usage`, `predefinedItems` in its own field. Omit or pass [] to inspect the object alone. |
| `sourceSet` | string | да | Exact Configuration source-set name from v8project.yaml. |

**Результат сейчас:** `structuredContent.data`: локальная структура и валидация объекта с обязательной связанной парой `kind + details` для 23 видов; `details` возвращает наблюдаемые типы Constant/DefinedType, метод ScheduledJob, schedule CalculationRegister, HTTP templates/methods и WebService packages/operations/parameters с expanded XDTO QName (ADR-0047, INV-MCP-META-INFO-COVERAGE). Формы и макеты наблюдаются по ссылке владельца и отдельному XML-дескриптору, встроенные команды — по дескриптору; HTML-страницы зарегистрированных макетов удерживаются как UTF-8 без XML-разбора. Тип содержит `mutationCapability: editable | readOnly`; UUID представлен вариантом `uuid` и доказанно редактируем, а неизвестный корректный платформенный QName оставляет только свой элемент `incomplete` с warning (ADR-0042, INV-MCP-META-OBSERVATION). `ChartOfCharacteristicTypes.details.type`, `ChartOfCalculationTypes.details.baseCalculationTypes` и `DocumentJournal.details.registeredDocuments` сохраняют kind-specific факты; `standardAttributes`, `characteristics`, `standardTabularSections`, `relations.dataLockFields` и коллекции `recalculations`/`accountingFlags`/`extDimensionAccountingFlags`/`addressingAttributes` имеют typed-владельцев и all-or-none tri-state. Общие read-properties не зависят от writer allowlist; неизвестный составной узел даёт `provider_unavailable`, а не пропускается. `relations.source`, `Event`, `Handler` сохраняют контракт подписки; `functionalSubsystems` и `interfaceSubsystems` содержат только членства текущего объекта в зарегистрированной топологии как плоские `SubsystemAddress`, сопоставляя `Content` по адресу метаданных или UUID корневого дескриптора; доказанное отсутствие членств сериализуется как `[]`, а при недоступном или повреждённом доказательстве поля отсутствуют и диагностика содержит `provider_unavailable` (ADR-0036, INV-SOURCE-SUBSYSTEM-TOPOLOGY). Явно выбранные секции читаются из дерева исходников в `usage` и `predefinedItems`, а `predefinedItems.items` возвращает плоский документный порядок, UUID, `parentId` и typed-поля владельца; обращения к RLM нет ни при каких аргументах (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Изучить структуру справочника перед написанием запроса
- Сравнить два объекта по подчинению и составу реквизитов
- Уточнить длину кода и основное представление перед генерацией формы
- Прочитать фактическую логическую связку источников, события и обработчика подписки
- Увидеть функциональные и интерфейсные подсистемы, в которые входит объект
- Прочитать вложенные маршруты HTTP-сервиса и типизированный контракт WebService
- Прочитать предопределённые элементы в документном порядке вместе с их parentId

### `unica.meta.remove`

Remove one metadata object through a logical guarded target.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Explicitly confirm a forced metadata object removal. |
| `dryRun` | boolean | нет | Preview the mutation without writing workspace files. |
| `force` | boolean | нет | Allow removal despite discovered references when confirmed. |
| `metadataPath` | string | да | Logical metadata path of the object to remove. |
| `sourceSet` | string | да | Exact Configuration source-set name from v8project.yaml. |

**Результат сейчас:** `structuredContent.data`: логический адрес, ссылки, валидация, семантический `removeObject` effect и план атомарного удаления (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Удалить устаревший объект вместе с регистрацией в Configuration.xml
- Проверить предпросмотром, что ещё ссылается на объект

## mxl — табличные макеты

### `unica.mxl.compile`

Compile spreadsheet Template.xml from JSON DSL.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `JsonPath` | string | да | Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.mxl.compile` and `unica.role.compile` |
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
| `TemplatePath` | string | по ветви | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **156** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `TemplatePath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Получить редактируемое описание готового макета

### `unica.mxl.info`

Inspect spreadsheet Template.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `TemplatePath` | string | по ветви | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `WithText` | boolean | нет | `unica.mxl.info` only: boolean including static cell text and template strings with `[Parameter]` substitutions in the report |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `withText` | boolean | нет | `unica.mxl.info` only: boolean including static cell text and template strings with `[Parameter]` substitutions in the report |

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `TemplatePath`. Ни одной или обе сразу отклоняются.

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
| `TemplatePath` | string | по ветви | Path to a `Template.xml`, or its directory which auto-resolves to `Ext/Template.xml`, for `unica.dcs.edit`/`info`/`validate` and `unica.mxl.info`/`validate`/`decompile`, relative to `cwd`; `unica.dcs.compile` writes through `outputPath` and ignores this argument. |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **158** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `TemplatePath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить макет после сборки

## project — рабочее пространство

### `unica.project.map`

Inspect configured source sets and effective source format per source set.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |

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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |

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
| `JsonPath` | string | да | Path to the JSON DSL file, relative to `cwd`, for `unica.form.compile`, `unica.form.edit`, `unica.mxl.compile` and `unica.role.compile` |
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `role.compile`/`subsystem.compile` |

Публикует **161** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Создать роль из описания прав

### `unica.role.edit`

Edit role rights through a closed logical typed contract.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `dryRun` | boolean | нет | Preview the typed role edit without writing workspace files; when omitted it defaults to true. Send false only when the user explicitly requests application. |
| `metadataPath` | string | да | Canonical logical role address in the form Role.<name>. |
| `operations` | array | да | Ordered closed setRight operations; each effect is reported by operationIndex. |
| `sourceSet` | string | да | Exact configured source-set name; physical source paths are not accepted. |

**Результат сейчас:** `structuredContent.data`: канонический `metadataPath`, `changed`, семантические `effects` по `operationIndex`, `validation` и `diagnostics` без stdout, diff и физических путей (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Запретить удаление для одного справочника, сохранив остальные права, RLS и шаблоны роли
- Проверить последовательность прав в предпросмотре до атомарного применения

### `unica.role.info`

Inspect role Rights.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `RightsPath` | string | по ветви | Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd` |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `RightsPath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** `data`: разрешённые и запрещённые права по видам объектов, RLS, шаблоны и поддержка (ADR-0023) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Проверить, какие права даёт роль перед её выдачей
- Найти объекты с ограничением на уровне записей

### `unica.role.validate`

Validate role Rights.xml.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `RightsPath` | string | по ветви | Path to a role's `Rights.xml`, or the role directory that resolves to it, for `unica.role.info` and `unica.role.validate`, relative to `cwd` |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `RightsPath`. Ни одной или обе сразу отклоняются.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Проверить роль после правки Rights.xml

## runtime — выполнение и задания

### `unica.runtime.execute`

Execute typed v8-runner runtime workflows through the single Unica MCP boundary.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema. |

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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Отменить зависшее задание

### `unica.runtime.job.list`

List durable runtime job snapshots in the current workspace.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Перечислить задания рабочего пространства

### `unica.runtime.job.logs`

Read bounded redacted stdout and stderr tails for a durable runtime job.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
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
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema. |

Публикует **61** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Запустить длительную операцию, не блокируя сессию

### `unica.runtime.job.status`

Read a durable runtime job snapshot by jobId.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Узнать состояние запущенного задания

### `unica.runtime.job.wait`

Wait for a durable runtime job with a caller-side bounded timeout.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `jobId` | string | да | UUID of a durable runtime job as returned by unica.runtime.job.start; required by the job status, wait, logs and cancel tools |
| `timeoutSeconds` | integer | нет | Integer seconds bounding a blocking call: 1..60 (default 30) for unica.runtime.job.wait, and 30..3600 for unica.code.diagnostics mode analyze; diagnostics falls back to operational.code_diagnostics.analyze_timeout_seconds from workspace config, then to 120. |

**Результат сейчас:** снимок задания в `job` (отвечают снимком задания в `job`)

**Вне границ: семейство runtime и build изучается отдельно.**

**Сценарии:**

- Дождаться завершения задания с ограниченным таймаутом

## source — логическая адресация и ресурсы

### `unica.source.children`

List exactly one level below a logical source-set root or metadata address.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cursor` | string | по ветви | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `metadataPath` | string | только в ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `scope` | string | только в ветви | Bounded source.resources manifest scope: self, aggregate, or registrations |
| `snapshotId` | string | по ветви | Opaque application-instance and workspace-bound identifier returned by source.resources; expires after five minutes |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` **либо** `snapshotId` + `cursor`. Ни одной или обе сразу отклоняются.
`metadataPath` принимается только вместе с `sourceSet`.
`scope` принимается только вместе с `sourceSet`.

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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `idOrAliasOrUrl` | string | нет | Standard number, alias or full URL (e.g. "644") that puts standards.explain in page-fetch mode; prefer it over id, which it overrides when both are passed, and standards.search ignores it. |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` and `unica.documentation.get` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section and document reports the locale that actually answered |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
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
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `id` | string | нет | Standard id, alias or URL for standards.explain (lower-precedence alias of idOrAliasOrUrl), but a graph node id such as method:CommonModule.Sales.OnPost for code.graph; standards.search ignores it. |
| `idOrAliasOrUrl` | string | нет | Standard number, alias or full URL (e.g. "644") that puts standards.explain in page-fetch mode; prefer it over id, which it overrides when both are passed, and standards.search ignores it. |
| `language` | string | нет | Alias of `lang` for `unica.help.add`; on `unica.standards.explain` the same key instead names the language of the `snippet` being explained; on `unica.documentation.search` and `unica.documentation.get` it picks the locale of the platform help containers to read and of the signature returned with each hit, defaulting to ru, and each corpus falls back on its own to the installed locale (the English `root` container first) when the installation ships no containers in the requested one, so every section and document reports the locale that actually answered |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
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
| `OutputDir` | string | да | Destination root directory relative to `cwd`: the new dump for `cf.init`/`cfe.init`/`epf.init`/`erf.init`, or the existing dump root holding `Configuration.xml` for `role.compile`/`subsystem.compile` |

Публикует **160** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** текст в `stdout` (отвечают прозой в `stdout`)

**Вне границ: снимается отдельной фичей (`*.validate`, `*.compile`, `*.decompile`).**

**Сценарии:**

- Добавить новый раздел в конфигурацию

### `unica.subsystem.edit`

Edit subsystem XML content and hierarchy.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | да | Path to a subsystem XML or `Subsystems` directory, relative to `cwd`; `unica.subsystem.info` returns the registered tree for a directory, the ancestor chain plus descendants for a registered XML, and local data without `tree` for an unregistered XML |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Результат сейчас:** `data`: каждая операция с признаком применения, причиной пропуска и нормализованной ссылкой, счётчики, созданные заготовки и факт валидации (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Включить объект в подсистему

### `unica.subsystem.info`

Inspect a registered subsystem tree from a directory, a focused registered tree from XML, or an unregistered XML locally.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | по ветви | Path to a subsystem XML or `Subsystems` directory, relative to `cwd`; `unica.subsystem.info` returns the registered tree for a directory, the ancestor chain plus descendants for a registered XML, and local data without `tree` for an unregistered XML |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `metadataPath` | string | только в ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

**Селектор:** ровно одна ветвь — `sourceSet` **либо** `SubsystemPath`. Ни одной или обе сразу отклоняются.
`metadataPath` принимается только вместе с `sourceSet`.

**Результат сейчас:** `data`: состав, группы, дочерние подсистемы и командный интерфейс; каталог `Subsystems` возвращает зарегистрированное `tree`, зарегистрированный XML — сфокусированное `tree` с цепочкой от корня до выбранного узла и всеми его потомками, а самостоятельный незарегистрированный XML — только локальные данные без `tree`; повреждение доказательства даёт `provider_unavailable`, а отмена и истечение срока сохраняют собственную типизированную семантику сбоя; ни один из этих случаев не публикует частичное дерево (ADR-0023, ADR-0036, INV-SOURCE-SUBSYSTEM-TOPOLOGY) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Понять границы подсистемы перед добавлением объекта
- Прочитать видимость и размещение команд подсистемы
- Построить полное дерево зарегистрированной топологии каталога `Subsystems`
- Увидеть цепочку от корня до выбранной подсистемы и полное дерево её потомков
- Прочитать самостоятельный XML локально, не принимая отсутствие регистрации за пустое дерево

### `unica.subsystem.validate`

Validate subsystem XML.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `SubsystemPath` | string | по ветви | Path to a subsystem XML or `Subsystems` directory, relative to `cwd`; `unica.subsystem.info` returns the registered tree for a directory, the ancestor chain plus descendants for a registered XML, and local data without `tree` for an unregistered XML |
| `metadataPath` | string | по ветви | Tool-scoped metadata address; consult the selected tool contract for its accepted shape and semantics. |
| `sourceSet` | string | по ветви | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |

Публикует **159** аргументов: обязательные — показаны выше, остальные приходят из общего списка `NATIVE_XML_DSL_ARGS`, и обработчик читает из них единицы.

**Селектор:** ровно одна ветвь — `sourceSet` + `metadataPath` **либо** `SubsystemPath`. Ни одной или обе сразу отклоняются.

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

## xdto — пакеты XDTO

### `unica.xdto.edit`

Preview or apply a safe targeted mutation to one logically addressed 1C XDTO package schema.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `base` | string | нет | Prefixed lexical QName naming the base type of a new XDTO valueType in `unica.xdto.edit`, for example `xs:string`; an unprefixed name or surrounding whitespace is rejected. |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `dryRun` | boolean | нет | Boolean preview switch for mutation tools; when omitted or true the tool only reports the change it would make, and false applies the mutation when the user requested execution. |
| `metadataPath` | string | да | Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected. |
| `name` | string | нет | Subject name whose meaning is tool-scoped: the object being created by `cf.init`, `cfe.init`, `epf.init` and `erf.init`, the required BSL method to locate on `unica.code.definition`, and the XDTO type or property named by the chosen `unica.xdto.edit` operation. The eight narrowed native XML readers no longer take it: they answer with every section at once, so there is nothing left for it to drill into (ADR-0048). |
| `operation` | string | да | Required selector whose accepted values are tool-scoped: config-init, init, build, dump, convert, make, load, syntax, test, launch, extensions or tools-download for unica.runtime.execute and unica.runtime.job.start; `insert` or `replace` for unica.code.patch; `add-value-type`, `add-object-type`, `add-property`, `remove-type` or `remove-property` for `unica.xdto.edit` — read the enum published in the tool's own schema. |
| `property` | object | нет | New XDTO property object for `unica.xdto.edit`: `name` must be an XML NCName and `type` a prefixed lexical QName; `minOccurs` is optional and must be 0 or 1. |
| `propertyPath` | string | нет | Property path to a nested XDTO `typeDef`: an unescaped dot separates segments and `\.` denotes a literal dot inside one NCName, for example `A\.B.Child`. |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `typeName` | string | нет | Name of the XDTO valueType or objectType, or of the target objectType for a property operation. |

**Результат сейчас:** `data`: операция, no-op, byte-local план изменения и стабильные findings (ADR-0024) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Предпросмотром добавить тип или свойство и проверить точный план до записи
- Применить подтверждённый неизменный план с guard-проверками цели и снимка
- Без записи распознать точный повтор операции как no-op

### `unica.xdto.info`

Inspect one logically addressed 1C XDTO package schema.

| Аргумент | Тип | Обяз. | Описание |
| --- | --- | --- | --- |
| `confirm` | boolean | нет | Boolean acknowledgement accepted where published and stripped before the runner is called; it does not select or enable an invocation mode on its own. |
| `cursor` | string | нет | Opaque continuation token returned by the same source navigation request or source.resources snapshot page; do not inspect or reuse it with another request or snapshot |
| `cwd` | string | нет | Absolute path to the workspace root holding v8project.yaml; it becomes the runner's working directory, so every other path argument is read relative to it |
| `limit` | integer | нет | Cap on how much one call returns, counted in the entities that tool answers with and never in printed lines: meta.info section items (default 20), xdto.info package types, code.search hits (20 per provider), code.definition definitions (50), code.graph nodes, code.diagnostics findings, standards and documentation results. On `unica.source.read` alone the unit is bytes, because that tool returns one bounded byte range. The eight narrowed native XML readers answer with every section at once and publish no `limit` (ADR-0048). |
| `metadataPath` | string | да | Logical address of an XDTO package in the form `XDTOPackage.<name>`; the physical `Package.bin` path is rejected. |
| `sourceSet` | string | да | Exact name of one source-set declared in v8project.yaml, such as main or addOn; unica.code.patch requires a Platform XML Configuration or Extension source set |
| `typeName` | string | нет | Name of the XDTO valueType or objectType, or of the target objectType for a property operation. |

**Результат сейчас:** `data`: сводка, импорты, типы, свойства и логические позиции пакета XDTO (ADR-0024) (отвечают типизированным `data`)

**Целевой контракт:** достигнут

**Сценарии:**

- Прочитать сводку и импорты XDTO-пакета по логическому адресу
- Перелистать именованные типы ограниченными страницами
- Получить рекурсивную деталь одного типа без раскрытия физического Package.bin
