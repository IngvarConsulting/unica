# cc-1c-skills: анализ изменений и кандидатов на заимствование

## Срез и метод

Этот отчёт фиксирует рассмотренный интервал до принятого corpus commit
`e01688e764a3cf1c1b4a0ad5069ea885837cfb2e`:

- general provenance baseline: `f3466e19fdc37954c030e48daabcc192f0098fe7`;
- accepted donor corpus baseline: `e01688e764a3cf1c1b4a0ad5069ea885837cfb2e`;
- `f346…` является предком `e016…`; это не сравнение расходящихся веток.

Проверены `git diff -M f346… e016…`, donor scripts, `tests/skills/**`,
`tests/web-test/**`, текущий публичный MCP-контракт Unica и provenance. Полный
corpus сохранён тестовым fixture, но это не означает автоматическое принятие
семантики или запуск всех donor suites в CI.

| Показатель | Результат |
|---|---:|
| Commits в полном интервале | 783 |
| Commits по first-parent | 493 |
| Изменённых файлов | 1 748 |
| Добавлено / удалено строк | +200 693 / -97 360 |
| Добавлено / изменено / удалено / переименовано файлов | 1 230 / 502 / 6 / 10 |
| Изменений в `tests/skills` | 1 354 файлов |
| Изменений в `.claude/skills` | 294 файлов |
| Новых файлов `tests/web-test` | 41 |

## Что изменилось у Николая

Число donor skill directories выросло с 66 до 72. Добавлены:
`db-dump-dt`, `db-load-dt`, `form-decompile`, `meta-decompile`,
`skd-decompile`, `support-edit`. Donor skills не удалялись; удалены пять
устаревших reference-файлов. Добавлено 71 script-файл: 12 PowerShell/Python
обёрток новых навыков и 59 JavaScript-модулей `web-test`.

На принятом commit corpus содержит 72 skills, 65 skills со скриптами, 193
скрипта, 564 прямых JSON scenario definitions в 61 case scope, а также 21
вложенный JSON snapshot/fixture. Отдельно есть 10 многошаговых
integration-сценариев и 32 web E2E `.test.mjs` в 41 файле `tests/web-test`.

### Горячие области

| Donor область | Изменённых файлов | Характер изменения | Вывод для Unica |
|---|---:|---|---|
| `meta-compile` | 147 | Расширены DSL, типы и fixtures; добавлен `meta-decompile` | Нужен отдельный выбор контракта для обратного преобразования; текущий `meta.info` только related capability. |
| `skd-edit` | 85 | Структурные правки, параметры, query patching, preservation | Изучить как отдельное расширение `unica.dcs.edit`; не копировать script workflow. |
| `form-compile` | 76 | Большой DSL/round-trip пласт; добавлен `form-decompile` | Существующий executable parity остаётся частичным: 2 compatible, 19 intentional divergence, 24 donor ahead. |
| `cfe-patch-method` | 74 | Source-aware v2: сигнатуры, `Instead`, resync, конфликты | Текущий public tool намеренно уже: только проверенная граница `Before/After`; нужен отдельный дизайн v2. |
| `web-test` | 68 | Новый Playwright/1C E2E продукт, license/context pool, артефакты | Не переносить как «ещё один skill»: это отдельная runtime-подсистема. |
| `skd-decompile` | 57 | Новый draft-декомпилятор с warnings/sentinel-узлами | Решить, нужен ли read-only public `unica.dcs.decompile`; сейчас его нет. |
| `support-edit` | 33 | Поддержка capability и object rules | Уже есть нативный `unica.support.edit`; нового продуктового gap нет. |
| `form-decompile` | 9 | Новый draft-декомпилятор форм | Рассматривать вместе с тем, что active spec уже упоминает несуществующий public tool. |
| `meta-decompile` | 9 | Новый draft-декомпилятор метаданных | Отложить после определения form/DCS decompile contract. |
| `form-add`, `template-add` | 9 / 8 | Идемпотентная регистрация ChildObjects после compile | Приоритетная проверка и, при подтверждении, нативное исправление. |

## Выявленные противоречия

### Full corpus не равен исполняемому parity

Сохранённый corpus содержит сценарии для `form-add`, `template-add` и других
навыков, но relation обязательны только для пяти `executableCaseScopes`.
Поэтому наличие donor case в fixture или `parityBaselineCommit` у dependency
skill не доказывает, что этот case прогоняется против Unica. Публичная матрица
показывает такие строки как `dependency_only` или `not_selected`, а не как
ложный `donor_ahead`.

### `form.add` и `template.add` требуют native regression

Последний donor commit исправляет повторную регистрацию `<Form>` и `<Template>`
в `ChildObjects` после compile. В текущем Unica путь `form.add` вызывает
регистрацию формы без видимой проверки существующего тега, а путь
`template.add` аналогично добавляет child entry. Это source-level evidence, но
ещё не исполнительное доказательство: перед изменением нужно добавить native
регрессию `compile -> add` и проверить, что в `ChildObjects` ровно одна запись.

Это важнее поверхностного «поднятия baseline»: если regression подтвердится,
исправлять нужно регистрацию в общем native helper, а не скрывать дубль в
конкретном skill или snapshot.

### Active specs обещают отсутствующие decompile tools

Current form/DCS specifications упоминают `form-decompile` и `dcs-decompile`,
тогда как живой публичный MCP-контракт содержит `form.info` и `dcs.info`, но
не содержит `unica.form.decompile`, `unica.meta.decompile` или
`unica.dcs.decompile`. Это архитектурное противоречие: либо ссылки должны быть
явно отнесены к future work, либо должен быть спроектирован read-only контракт;
донорские PowerShell/Python scripts не являются допустимым публичным path.

## Решения по заимствованию

| Приоритет | Решение | Обоснование и граница |
|---|---|---|
| P0 | **Подтвердить и исправить** идемпотентность `form.add` и `template.add` | Перенять семантику проверки регистрации `ChildObjects`, добавить native regression. Не заявлять executable parity до результата. |
| P1 | **Принять продуктовое решение** по decompile | Для form/DCS сначала определить output, неполноту, fail-closed и warnings. Для DCS полезны donor `__unsupported__` и warning artifact. `meta.decompile` — следующий кандидат, не предпосылка. |
| P1 | **Изучить отдельно** `cfe.patch_method` v2 | Нужны BSL signature parsing, source resolver, atomic resync/conflict contract и отдельные native tests. Не расширять текущий tool неявно. |
| P2 | **Усилить parity corpus** уже заимствованных навыков | Выбирать сценарии по одному scope после contract review; приоритет — существующие mutators (`form-add`, `meta-edit`, `skd-edit`) и проверки. |
| P3 | **Отложить** DT dump/load | Full `.dt` backup/restore несёт destructive lifecycle, secrets/redaction, retention, confirmation и isolated integration requirements; `unica.build.*` не эквивалентен ему. |
| P3 | **Отложить как отдельный продукт** `web-test` | Это 1C + Playwright E2E runtime с инфраструктурой и лицензированием, не MCP command alias. |
| P3 | **Не добавлять автоматически** DB/EPF/ERF overlaps | `unica.runtime.execute` — related/supporting capability, а не доказательство заимствования donor skill. |

## Что зафиксировано этим refresh

1. Все donor scripts и тестовые материалы на `e016…` сохранены и
   хэшированы в test-only fixture.
2. 152 уже reviewed executable relations перенесены без изменения digest.
3. Новые или невыбранные donor cases сохранены для будущей оценки, но не
   получают synthetic relation.
4. Семантическая карта и генерируемая публичная матрица отделяют факт
   заимствования от related capability и от наличия test corpus.

Следующий refresh должен повторить этот отчёт для нового exact commit, а не
молча менять итоговые выводы.
