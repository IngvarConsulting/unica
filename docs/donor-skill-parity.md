# Матрица паритета навыков Николая и Unica

Этот файл генерируется из принятого donor corpus, provenance, reviewed parity relations и явного семантического реестра. Не редактируйте его вручную.

- Донор: `https://github.com/Nikolay-Shirokov/cc-1c-skills` @ `e01688e764a3cf1c1b4a0ad5069ea885837cfb2e` (`main`).
- Перегенерация: `python3.12 scripts/ci/generate-donor-skill-matrix.py --repo-root . --write`.
- Принято: 72 donor skills, 193 scripts, 564 JSON-кейса в сохранённом donor corpus и 21 JSON snapshots/fixtures; 46 skills с явным `ported-to-unica` provenance.
- Исполняемый паритет: 152 cases; exact: 4, compatible: 8, intentional_divergence: 88, donor_ahead: 52.

## Как читать статусы

`Да` в колонке заимствования означает только явный `ported-to-unica` в provenance. `related`-инструмент показывает пересечение возможностей и не является доказательством заимствования. `not_selected` означает, что тестовый корпус сохранён, но его сценарии ещё не выбраны для исполняемого паритета; `unmapped` — что нет утверждённого семантического соответствия Unica.

## Матрица

| Какие скиллы есть у Николая | Заимствовали ли мы этот скилл в Unica | Какие наши тулзы есть для этого скилла | Какие скрипты Николая используются у него в скиле | Какое состояние парити для этого скилла | Какое состояние тестового корпуса для этого скилла |
|---|---|---|---|---|---|
| cf-edit | Да — `cf-edit` | `unica.cf.edit` (direct) | `skills/cf-edit/scripts/cf-edit.ps1`<br>`skills/cf-edit/scripts/cf-edit.py` | not_selected | 12/12 JSON |
| cf-info | Да — `cf-info` | `unica.cf.info` (direct) | `skills/cf-info/scripts/cf-info.ps1`<br>`skills/cf-info/scripts/cf-info.py` | not_selected | 8/8 JSON |
| cf-init | Да — `cf-init` | `unica.cf.init` (direct) | `skills/cf-init/scripts/cf-init.ps1`<br>`skills/cf-init/scripts/cf-init.py` | dependency_only | 6/6 JSON |
| cf-validate | Да — `cf-validate` | `unica.cf.validate` (direct) | `skills/cf-validate/scripts/cf-validate.ps1`<br>`skills/cf-validate/scripts/cf-validate.py` | not_selected | 6/6 JSON |
| cfe-borrow | Да — `cfe-borrow` | `unica.cfe.borrow` (direct) | `skills/cfe-borrow/scripts/cfe-borrow.ps1`<br>`skills/cfe-borrow/scripts/cfe-borrow.py` | intentional_divergence: 6 | 6/6 JSON |
| cfe-diff | Да — `cfe-diff` | `unica.cfe.diff` (direct) | `skills/cfe-diff/scripts/cfe-diff.ps1`<br>`skills/cfe-diff/scripts/cfe-diff.py` | not_selected | 3/3 JSON |
| cfe-init | Да — `cfe-init` | `unica.cfe.init` (direct) | `skills/cfe-init/scripts/cfe-init.ps1`<br>`skills/cfe-init/scripts/cfe-init.py` | dependency_only | 6/6 JSON |
| cfe-patch-method | Да — `cfe-patch-method` | `unica.cfe.patch_method` (direct) | `skills/cfe-patch-method/scripts/cfe-patch-method.ps1`<br>`skills/cfe-patch-method/scripts/cfe-patch-method.py` | not_selected | 22/22 JSON |
| cfe-validate | Да — `cfe-validate` | `unica.cfe.validate` (direct) | `skills/cfe-validate/scripts/cfe-validate.ps1`<br>`skills/cfe-validate/scripts/cfe-validate.py` | dependency_only | 4/4 JSON |
| db-create | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-create/scripts/db-create.ps1`<br>`skills/db-create/scripts/db-create.py` | not_selected | 3/3 JSON |
| db-dump-cf | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-dump-cf/scripts/db-dump-cf.ps1`<br>`skills/db-dump-cf/scripts/db-dump-cf.py` | not_selected | 2/2 JSON |
| db-dump-dt | Нет — в provenance нет `ported-to-unica` | — | `skills/db-dump-dt/scripts/db-dump-dt.ps1`<br>`skills/db-dump-dt/scripts/db-dump-dt.py` | unmapped | 1/1 JSON |
| db-dump-xml | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-dump-xml/scripts/db-dump-xml.ps1`<br>`skills/db-dump-xml/scripts/db-dump-xml.py` | not_selected | 2/2 JSON |
| db-list | Нет — в provenance нет `ported-to-unica` | `unica.project.status` (related) | — | not_selected | 0/0 JSON |
| db-load-cf | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-load-cf/scripts/db-load-cf.ps1`<br>`skills/db-load-cf/scripts/db-load-cf.py` | not_selected | 2/2 JSON |
| db-load-dt | Нет — в provenance нет `ported-to-unica` | — | `skills/db-load-dt/scripts/db-load-dt.ps1`<br>`skills/db-load-dt/scripts/db-load-dt.py` | unmapped | 1/1 JSON |
| db-load-git | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-load-git/scripts/db-load-git.ps1`<br>`skills/db-load-git/scripts/db-load-git.py` | not_selected | 2/2 JSON |
| db-load-xml | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-load-xml/scripts/db-load-xml.ps1`<br>`skills/db-load-xml/scripts/db-load-xml.py` | not_selected | 3/3 JSON |
| db-run | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-run/scripts/db-run.ps1`<br>`skills/db-run/scripts/db-run.py` | not_selected | 2/2 JSON |
| db-update | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/db-update/scripts/db-update.ps1`<br>`skills/db-update/scripts/db-update.py` | not_selected | 2/2 JSON |
| epf-bsp-add-command | Да — `epf-bsp-add-command` | — | — | not_selected | 0/0 JSON |
| epf-bsp-init | Да — `epf-bsp-init` | `unica.runtime.execute` (supporting) | — | not_selected | 0/0 JSON |
| epf-build | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/epf-build/scripts/epf-build.ps1`<br>`skills/epf-build/scripts/epf-build.py`<br>`skills/epf-build/scripts/stub-db-create.ps1`<br>`skills/epf-build/scripts/stub-db-create.py` | not_selected | 2/2 JSON |
| epf-dump | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/epf-dump/scripts/epf-dump.ps1`<br>`skills/epf-dump/scripts/epf-dump.py` | not_selected | 2/2 JSON |
| epf-init | Да — `epf-init` | `unica.epf.init` (direct) | `skills/epf-init/scripts/init.ps1`<br>`skills/epf-init/scripts/init.py` | not_selected | 4/4 JSON |
| epf-validate | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | `skills/epf-validate/scripts/epf-validate.ps1`<br>`skills/epf-validate/scripts/epf-validate.py` | not_selected | 5/5 JSON |
| erf-build | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | — | not_selected | 0/0 JSON |
| erf-dump | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | — | not_selected | 0/0 JSON |
| erf-init | Да — `erf-init` | `unica.erf.init` (direct) | `skills/erf-init/scripts/init.ps1`<br>`skills/erf-init/scripts/init.py` | not_selected | 4/4 JSON |
| erf-validate | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (related) | — | not_selected | 1/1 JSON |
| form-add | Да — `form-add` | `unica.form.add` (direct) | `skills/form-add/scripts/form-add.ps1`<br>`skills/form-add/scripts/form-add.py` | dependency_only | 11/11 JSON |
| form-compile | Да — `form-compile` | `unica.form.compile` (direct) | `skills/form-compile/scripts/form-compile.ps1`<br>`skills/form-compile/scripts/form-compile.py` | compatible: 2, intentional_divergence: 19, donor_ahead: 24 | 45/45 JSON |
| form-decompile | Нет — в provenance нет `ported-to-unica` | `unica.form.info` (related) | `skills/form-decompile/scripts/form-decompile.ps1`<br>`skills/form-decompile/scripts/form-decompile.py` | not_selected | 2/2 JSON |
| form-edit | Да — `form-edit` | `unica.form.edit` (direct) | `skills/form-edit/scripts/form-edit.ps1`<br>`skills/form-edit/scripts/form-edit.py` | not_selected | 6/6 JSON |
| form-info | Да — `form-info` | `unica.form.info` (direct) | `skills/form-info/scripts/form-info.ps1`<br>`skills/form-info/scripts/form-info.py` | not_selected | 6/6 JSON |
| form-patterns | Да — `form-patterns` | `unica.form.compile` (supporting) | — | not_selected | 0/0 JSON |
| form-remove | Да — `form-remove` | `unica.form.remove` (direct) | `skills/form-remove/scripts/remove-form.ps1`<br>`skills/form-remove/scripts/remove-form.py` | not_selected | 2/2 JSON |
| form-validate | Да — `form-validate` | `unica.form.validate` (direct) | `skills/form-validate/scripts/form-validate.ps1`<br>`skills/form-validate/scripts/form-validate.py` | dependency_only | 11/11 JSON |
| help-add | Да — `help-add` | `unica.help.add` (direct) | `skills/help-add/scripts/add-help.ps1`<br>`skills/help-add/scripts/add-help.py` | not_selected | 5/5 JSON |
| img-grid | Нет — в provenance нет `ported-to-unica` | — | `skills/img-grid/scripts/overlay-grid.py` | unmapped | 0/0 JSON |
| interface-edit | Да — `interface-edit` | `unica.interface.edit` (direct) | `skills/interface-edit/scripts/interface-edit.ps1`<br>`skills/interface-edit/scripts/interface-edit.py` | not_selected | 5/5 JSON |
| interface-validate | Да — `interface-validate` | `unica.interface.validate` (direct) | `skills/interface-validate/scripts/interface-validate.ps1`<br>`skills/interface-validate/scripts/interface-validate.py` | not_selected | 3/3 JSON |
| meta-compile | Да — `meta-compile` | `unica.meta.compile` (direct) | `skills/meta-compile/scripts/meta-compile.ps1`<br>`skills/meta-compile/scripts/meta-compile.py` | exact: 4, compatible: 2, intentional_divergence: 41, donor_ahead: 26 | 73/73 JSON |
| meta-decompile | Нет — в provenance нет `ported-to-unica` | `unica.meta.info` (related) | `skills/meta-decompile/scripts/meta-decompile.ps1`<br>`skills/meta-decompile/scripts/meta-decompile.py` | not_selected | 3/3 JSON |
| meta-edit | Да — `meta-edit` | `unica.meta.edit` (direct) | `skills/meta-edit/scripts/meta-edit.ps1`<br>`skills/meta-edit/scripts/meta-edit.py` | not_selected | 19/19 JSON |
| meta-info | Да — `meta-info` | `unica.meta.info` (direct) | `skills/meta-info/scripts/meta-info.ps1`<br>`skills/meta-info/scripts/meta-info.py` | not_selected | 17/17 JSON |
| meta-remove | Да — `meta-remove` | `unica.meta.remove` (direct) | `skills/meta-remove/scripts/meta-remove.ps1`<br>`skills/meta-remove/scripts/meta-remove.py` | not_selected | 7/7 JSON |
| meta-validate | Да — `meta-validate` | `unica.meta.validate` (direct) | `skills/meta-validate/scripts/meta-validate.ps1`<br>`skills/meta-validate/scripts/meta-validate.py` | dependency_only | 22/22 JSON |
| mxl-compile | Да — `mxl-compile` | `unica.mxl.compile` (direct) | `skills/mxl-compile/scripts/mxl-compile.ps1`<br>`skills/mxl-compile/scripts/mxl-compile.py` | not_selected | 13/13 JSON |
| mxl-decompile | Да — `mxl-decompile` | `unica.mxl.decompile` (direct) | `skills/mxl-decompile/scripts/mxl-decompile.ps1`<br>`skills/mxl-decompile/scripts/mxl-decompile.py` | not_selected | 5/5 JSON |
| mxl-info | Да — `mxl-info` | `unica.mxl.info` (direct) | `skills/mxl-info/scripts/mxl-info.ps1`<br>`skills/mxl-info/scripts/mxl-info.py` | not_selected | 7/7 JSON |
| mxl-validate | Да — `mxl-validate` | `unica.mxl.validate` (direct) | `skills/mxl-validate/scripts/mxl-validate.ps1`<br>`skills/mxl-validate/scripts/mxl-validate.py` | not_selected | 8/8 JSON |
| role-compile | Да — `role-compile` | `unica.role.compile` (direct) | `skills/role-compile/scripts/role-compile.ps1`<br>`skills/role-compile/scripts/role-compile.py` | not_selected | 9/9 JSON |
| role-info | Да — `role-info` | `unica.role.info` (direct) | `skills/role-info/scripts/role-info.ps1`<br>`skills/role-info/scripts/role-info.py` | not_selected | 6/6 JSON |
| role-validate | Да — `role-validate` | `unica.role.validate` (direct) | `skills/role-validate/scripts/role-validate.ps1`<br>`skills/role-validate/scripts/role-validate.py` | not_selected | 6/6 JSON |
| skd-compile | Да — `dcs-compile` | `unica.dcs.compile` (direct) | `skills/skd-compile/scripts/skd-compile.ps1`<br>`skills/skd-compile/scripts/skd-compile.py` | compatible: 4, intentional_divergence: 22, donor_ahead: 2 | 28/28 JSON |
| skd-decompile | Нет — в provenance нет `ported-to-unica` | `unica.dcs.info` (related) | `skills/skd-decompile/scripts/skd-decompile.ps1`<br>`skills/skd-decompile/scripts/skd-decompile.py` | not_selected | 17/17 JSON |
| skd-edit | Да — `dcs-edit` | `unica.dcs.edit` (direct) | `skills/skd-edit/scripts/skd-edit.ps1`<br>`skills/skd-edit/scripts/skd-edit.py` | not_selected | 49/49 JSON |
| skd-info | Да — `dcs-info` | `unica.dcs.info` (direct) | `skills/skd-info/scripts/skd-info.ps1`<br>`skills/skd-info/scripts/skd-info.py` | not_selected | 10/10 JSON |
| skd-validate | Да — `dcs-validate` | `unica.dcs.validate` (direct) | `skills/skd-validate/scripts/skd-validate.ps1`<br>`skills/skd-validate/scripts/skd-validate.py` | dependency_only | 15/15 JSON |
| subsystem-compile | Да — `subsystem-compile` | `unica.subsystem.compile` (direct) | `skills/subsystem-compile/scripts/subsystem-compile.ps1`<br>`skills/subsystem-compile/scripts/subsystem-compile.py` | not_selected | 9/9 JSON |
| subsystem-edit | Да — `subsystem-edit` | `unica.subsystem.edit` (direct) | `skills/subsystem-edit/scripts/subsystem-edit.ps1`<br>`skills/subsystem-edit/scripts/subsystem-edit.py` | not_selected | 6/6 JSON |
| subsystem-info | Да — `subsystem-info` | `unica.subsystem.info` (direct) | `skills/subsystem-info/scripts/subsystem-info.ps1`<br>`skills/subsystem-info/scripts/subsystem-info.py` | not_selected | 7/7 JSON |
| subsystem-validate | Да — `subsystem-validate` | `unica.subsystem.validate` (direct) | `skills/subsystem-validate/scripts/subsystem-validate.ps1`<br>`skills/subsystem-validate/scripts/subsystem-validate.py` | not_selected | 6/6 JSON |
| support-edit | Да — `support-edit` | `unica.support.edit` (direct) | `skills/support-edit/scripts/support-edit.ps1`<br>`skills/support-edit/scripts/support-edit.py` | not_selected | 5/5 JSON |
| template-add | Да — `template-add` | `unica.template.add` (direct) | `skills/template-add/scripts/add-template.ps1`<br>`skills/template-add/scripts/add-template.py` | not_selected | 7/7 JSON |
| template-remove | Да — `template-remove` | `unica.template.remove` (direct) | `skills/template-remove/scripts/remove-template.ps1`<br>`skills/template-remove/scripts/remove-template.py` | not_selected | 3/3 JSON |
| web-info | Нет — в provenance нет `ported-to-unica` | — | `skills/web-info/scripts/web-info.ps1`<br>`skills/web-info/scripts/web-info.py` | unmapped | 0/0 JSON |
| web-publish | Нет — в provenance нет `ported-to-unica` | — | `skills/web-publish/scripts/web-publish.ps1`<br>`skills/web-publish/scripts/web-publish.py` | unmapped | 0/0 JSON |
| web-stop | Нет — в provenance нет `ported-to-unica` | — | `skills/web-stop/scripts/web-stop.ps1`<br>`skills/web-stop/scripts/web-stop.py` | unmapped | 0/0 JSON |
| web-test | Нет — в provenance нет `ported-to-unica` | `unica.runtime.execute` (supporting) | `skills/web-test/scripts/browser.mjs`<br>`skills/web-test/scripts/cli/commands/exec.mjs`<br>`skills/web-test/scripts/cli/commands/run.mjs`<br>`skills/web-test/scripts/cli/commands/shot.mjs`<br>`skills/web-test/scripts/cli/commands/start.mjs`<br>`skills/web-test/scripts/cli/commands/status.mjs`<br>`skills/web-test/scripts/cli/commands/stop.mjs`<br>`skills/web-test/scripts/cli/commands/test.mjs`<br>`skills/web-test/scripts/cli/exec-context.mjs`<br>`skills/web-test/scripts/cli/server.mjs`<br>`skills/web-test/scripts/cli/session.mjs`<br>`skills/web-test/scripts/cli/test-runner/assertions.mjs`<br>`skills/web-test/scripts/cli/test-runner/context-pool.mjs`<br>`skills/web-test/scripts/cli/test-runner/discover.mjs`<br>`skills/web-test/scripts/cli/test-runner/reporters.mjs`<br>`skills/web-test/scripts/cli/test-runner/severity.mjs`<br>`skills/web-test/scripts/cli/test-runner/suite-root.mjs`<br>`skills/web-test/scripts/cli/util.mjs`<br>`skills/web-test/scripts/dom.mjs`<br>`skills/web-test/scripts/dom/_shared.mjs`<br>`skills/web-test/scripts/dom/edd.mjs`<br>`skills/web-test/scripts/dom/edit-state.mjs`<br>`skills/web-test/scripts/dom/errors-stack.mjs`<br>`skills/web-test/scripts/dom/errors.mjs`<br>`skills/web-test/scripts/dom/filter.mjs`<br>`skills/web-test/scripts/dom/form-state.mjs`<br>`skills/web-test/scripts/dom/forms.mjs`<br>`skills/web-test/scripts/dom/grid-edit.mjs`<br>`skills/web-test/scripts/dom/grid.mjs`<br>`skills/web-test/scripts/dom/nav.mjs`<br>`skills/web-test/scripts/dom/row-state.mjs`<br>`skills/web-test/scripts/dom/submenu.mjs`<br>`skills/web-test/scripts/engine/core/click.mjs`<br>`skills/web-test/scripts/engine/core/clipboard.mjs`<br>`skills/web-test/scripts/engine/core/deadline.mjs`<br>`skills/web-test/scripts/engine/core/errors.mjs`<br>`skills/web-test/scripts/engine/core/helpers.mjs`<br>`skills/web-test/scripts/engine/core/scroll-horiz.mjs`<br>`skills/web-test/scripts/engine/core/session.mjs`<br>`skills/web-test/scripts/engine/core/state.mjs`<br>`skills/web-test/scripts/engine/core/wait.mjs`<br>`skills/web-test/scripts/engine/forms/click-form.mjs`<br>`skills/web-test/scripts/engine/forms/click-group.mjs`<br>`skills/web-test/scripts/engine/forms/click-popup.mjs`<br>`skills/web-test/scripts/engine/forms/close.mjs`<br>`skills/web-test/scripts/engine/forms/fill.mjs`<br>`skills/web-test/scripts/engine/forms/select-value.mjs`<br>`skills/web-test/scripts/engine/forms/state.mjs`<br>`skills/web-test/scripts/engine/nav/navigation.mjs`<br>`skills/web-test/scripts/engine/recording/captions.mjs`<br>`skills/web-test/scripts/engine/recording/capture.mjs`<br>`skills/web-test/scripts/engine/recording/highlight.mjs`<br>`skills/web-test/scripts/engine/recording/narration.mjs`<br>`skills/web-test/scripts/engine/recording/tts.mjs`<br>`skills/web-test/scripts/engine/spreadsheet/spreadsheet.mjs`<br>`skills/web-test/scripts/engine/table/click-cell.mjs`<br>`skills/web-test/scripts/engine/table/click-row.mjs`<br>`skills/web-test/scripts/engine/table/filter.mjs`<br>`skills/web-test/scripts/engine/table/grid-toggle.mjs`<br>`skills/web-test/scripts/engine/table/grid.mjs`<br>`skills/web-test/scripts/engine/table/row-fill.mjs`<br>`skills/web-test/scripts/package-lock.json`<br>`skills/web-test/scripts/package.json`<br>`skills/web-test/scripts/run.mjs` | not_selected | 0/0 JSON; web-test: 41 files |
| web-unpublish | Нет — в provenance нет `ported-to-unica` | — | `skills/web-unpublish/scripts/web-unpublish.ps1`<br>`skills/web-unpublish/scripts/web-unpublish.py` | unmapped | 0/0 JSON |

## Инварианты

- Полный corpus — это сохранённые и хэшированные donor bytes; он не означает, что Unica заявляет совместимость.
- Relations обязательны только для `executableCaseScopes`; неподдержанные сохранённые кейсы не маскируются как `donor_ahead`.
- Donor scripts и corpus тестовые: они не входят в marketplace plugin.
