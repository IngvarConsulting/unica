# Borrowed properties: platform evidence

These are raw descriptor exports from 1C **8.3.27.2074**, XML format **2.20**.
`manifest.json` records the platform binary and fixture SHA-256 hashes.
Each experiment used disposable local infobases; no existing user base was used.

The parent configuration and empty extension were generated from the repository's
`tests/fixtures/unica_mcp_script_parity/unica_reference_models/cf-init/scripts/cf-init.py`
and corresponding `cfe-init/scripts/cfe-init.py` scaffolds. The parent contained
`Catalog.CorpusCatalog` and `CommonModule.CorpusModule`; both were registered in
`Configuration.xml`. The four files here are exported object descriptors, not
complete importable configurations. Keep them in their `Catalogs/` and
`CommonModules/` locations inside complete scaffolds when repeating the experiment.

The following is the executed command sequence with temporary paths replaced by
relative placeholders. `parent-src` and `extension-src` are complete XML exports.
The same binary was used for every command; all commands exited with status zero.

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
round=./round1
connection=(--db-path="$round/ib/db" --data="$round/ib/data"
  --temp="$round/ib/temp" --users-data="$round/ib/users"
  --session-data="$round/ib/session" --log-data="$round/ib/log")
"$ibcmd" infobase create "${connection[@]}" --import=./parent-src --apply --force
"$ibcmd" extension "${connection[@]}" create --name=CorpusExtension --name-prefix=CorpusExtension_ --purpose=customization
"$ibcmd" config "${connection[@]}" import --extension=CorpusExtension ./extension-src
"$ibcmd" config "${connection[@]}" check --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" apply --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" export --extension=CorpusExtension "$round/export"
```

Repeat all six commands with `round=./round2`, the same parent sources, and
`./round1/export` as the extension import. Both extension descriptors were
byte-identical after the second export from this independently created infobase.

The Catalog fixture proves preservation of `DescriptionLength=50` with
`xr:PropertyState` `DescriptionLength/Extended`, alongside `CodeLength=3` and
`Hierarchical=false` without state markers. The CommonModule fixture proves
preservation of `Server=false` with `Server/Notify`. A separate `Server/Extended`
probe lost its marker on export; it is not evidence for that override format.
These observations establish XML import/check/apply/export behavior, not a
separate runtime `CheckCanApply` result.

A further execution used the current Unica canonical `object.borrow` output as
the extension input to the same two-cycle sequence. Before import, a parent
refresh changed `CodeLength` from 3 to 5 while preserving local
`DescriptionLength=50`, Comment, object UUID and generated type identities.
Both cycles preserved those values, `DescriptionLength/Extended` and
`Server/Notify`; both final descriptors were byte-identical between cycles.
The canonical protocol probe also checked write-free preview, rejection of a
revision made stale by a parent edit, and byte- and file-identity-preserving
repeat apply with the committed combined revision. This execution supplements
the fixture evidence; its refreshed descriptors are not the files in this folder.

## Adopted CommonModule without properties (#1304)

A separate run with the same binary and the same six commands imported a
parent with `CommonModule.КешЗначений` (`Privileged=true`,
`ReturnValuesReuse=DuringRequest`) and `CommonModule.СерверныеФункции`, and an
extension that adopts both with BSL in their modules. The first adopted
descriptor carried only `ObjectBelonging`, `Name`, `Comment` and
`ExtendedConfigurationObject`; the second also carried the six environment
flags, but neither `Privileged` nor `ReturnValuesReuse`. Import, `check` and
`apply` succeeded; the export kept exactly those property sets and added only
`PropertyState Module/Extended`. A second cycle through an independently created
infobase reproduced both descriptors byte for byte.

Both extension input descriptors were written by hand, not produced by the
Configurator's "add to extension" command. The experiment therefore proves that
the platform accepts these shapes and preserves them unchanged through
import/export; it does not show what the Configurator writes when it adopts a
module itself.

`extension-common-module-inherited.xml` and
`extension-common-module-environment-only.xml` are those exports;
`parent-common-module-cache.xml` is the parent of the first one. They prove that
any of the eight CommonModule properties may be absent from an adopted
descriptor, and that absence is not the parent's value copied or a default:
the parent's `DuringRequest` was not written. The complete trees are the
`workspace-borrowed` acceptance fixture.

## Adopted objects without their own content files (#1381)

A third run with the same binary and the same six commands extended the
#1304 parent with `Role.SalesReader` (with `Ext/Rights.xml`),
`CommonForm.ОбщаяФорма` (with `Ext/Form.xml`), `XDTOPackage.ПакетОбмена`
(with `Ext/Package.bin`), `CommonTemplate.ОбщийМакет` and
`DataProcessor.Обработка` with the form `Форма` and the templates `Макет`
(spreadsheet) and `СхемаКомпоновки` (data composition schema), each with its
`Ext/Form.xml` or `Ext/Template.xml`. The extension adopted every one of them
with a descriptor of `ObjectBelonging`, `Name`, `Comment` and
`ExtendedConfigurationObject` only, and with none of the companion files.
Import, `check` and `apply` succeeded. The export contains no `Rights.xml`,
`Form.xml`, `Template.xml` or `Package.bin` for these objects. The platform
added only `PropertyState Rights/Extended` to the role and `Form/Extended` to
both forms; the XDTO package keeps no `Namespace`.

A template descriptor imported without `TemplateType` is exported with
`TemplateType` `SpreadsheetDocument` even when the parent template is a data
composition schema: the platform writes its default, not the parent's value.
The schema template was therefore imported once more with `TemplateType`
`DataCompositionSchema`; that value survived. The files here come from that
round. A second cycle through an independently created infobase reproduced
every extension file byte for byte.

A further probe added `Ext/Form/Module.bsl` to the adopted `CommonForm`
without `Form.xml`. Import succeeded, but the export dropped the module: the
platform keeps no form module without the form's own content.

As in the #1304 run, the extension descriptors on input were written by hand,
not produced by the Configurator's "add to extension" command. The parent
objects were written by hand too; the data composition schema and the
spreadsheet body are minimal own documents. The experiment proves that the
platform accepts and preserves these shapes, not what the Configurator writes
when it adopts an object.

`extension-role-inherited.xml`, `extension-common-form-inherited.xml`,
`extension-object-form-inherited.xml`, `extension-template-spreadsheet-inherited.xml`,
`extension-template-schema-inherited.xml`, `extension-common-template-inherited.xml`,
`extension-xdto-package-inherited.xml` and their owner
`extension-data-processor.xml` are those exports. The complete trees are part
of the `workspace-borrowed` acceptance fixture.

## Заимствованное значение перечисления (#1396)

`extension-enum-value.xml` — исходный экспорт расширения после живого цикла
импорта, проверки, применения и выгрузки через `ibcmd` 8.3.27.2074. Файл
сохраняется побайтно с BOM и CRLF. Он фиксирует порядок `EnumValue`,
`InternalInfo`, `Properties`, а внутри свойств — `ObjectBelonging`, `Name`,
`Synonym`, `Comment`, `ExtendedConfigurationObject`; у вложенных элементов нет
повторных объявлений пространств имён.

## Заимствованная форма объекта со своим `Form.xml` (#1982)

Опыт с тем же бинарём 8.3.27.2074 и теми же шестью командами. Родитель —
конфигурация из `cf-init.py` с `Catalog.CorpusCatalog` (описание как в
`parent-catalog.xml`, плюс `DefaultObjectForm` и `ChildObjects/Form`) и
управляемой формой элемента `CorpusForm`: поле `Объект.Description`, кнопка
командной панели с командой формы, событие `OnCreateAtServer` и модуль с двумя
обработчиками. Родитель написан вручную, один раз импортирован в базу
(`infobase create --import --apply`, затем `config check` и
`config export`) и дальше всегда брался из этой выгрузки. Байты этой выгрузки
лежат в `parent-catalog-with-form.xml`, `parent-catalog-form.xml`,
`parent-catalog-form-Form.xml` и `parent-catalog-form-Module.bsl`.

Расширение — пустое расширение из `cfe-init.py` (`Customization`,
префикс `CorpusExtension_`), в котором заимствованы справочник и форма.
Описание владельца несёт только `ObjectBelonging`, `Name`, `Comment`,
`ExtendedConfigurationObject` и сгенерированные типы. Порядок команд:

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
round=./round1
connection=(--db-path="$round/ib/db" --data="$round/ib/data"
  --temp="$round/ib/temp" --users-data="$round/ib/users"
  --session-data="$round/ib/session" --log-data="$round/ib/log")
"$ibcmd" infobase create "${connection[@]}" --import=./parent-src --apply --force
"$ibcmd" extension "${connection[@]}" create --name=CorpusExtension --name-prefix=CorpusExtension_ --purpose=customization
"$ibcmd" config "${connection[@]}" import --extension=CorpusExtension ./extension-src
"$ibcmd" config "${connection[@]}" check --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" apply --extension=CorpusExtension --force
"$ibcmd" config "${connection[@]}" export --extension=CorpusExtension "$round/export"
```

Второй круг — те же шесть команд с `round=./round2` в независимо созданной
базе и с `./round1/export` в качестве `extension-src`. Каждый вариант ниже
прошёл оба круга с кодом выхода 0 у всех двенадцати команд, без `[WARN]` и
`[ERROR]`. Выгрузки двух кругов совпали побайтно во всех файлах, включая
`ConfigDumpInfo.xml`.

Проверенные формы `Ext/Form.xml` при одном и том же описании формы
с `FormType` `Managed`:

| Вход | Что выгрузила платформа |
|---|---|
| вывод v0.12 `unica.cfe.borrow` (копия свойств, командной панели и элементов, `CommandName` 0, без `DataPath`, `Events`, `Commands`, пустой `Attributes`, та же копия в `BaseForm`) | то же без пустых строк |
| своя часть пуста, `BaseForm` — тело формы родителя | своя часть стала `<AutoCommandBar name="" id="-1"/>` и `<Attributes/>`; `BaseForm` сохранён, но без `UseForFoldersAndItems`, а `Event` и `Action` в нём получили `callType="Before"` |
| своя часть и `BaseForm` — тело формы родителя | обе части сохранены; `UseForFoldersAndItems` выпал; `Event` и `Action` получили `callType="Before"`, то есть стали обработчиками расширения |
| свойства формы, `AutoCommandBar` с `Autofill` `false`, `<Attributes/>` в обеих частях | без изменений |
| своя часть пуста, `<BaseForm version="2.20"/>` | обе части стали `<AutoCommandBar name="" id="-1"/>` и `<Attributes/>` |
| только корень `Form` без `BaseForm` | `<AutoCommandBar name="" id="-1"/>`, `<Attributes/>`, `BaseForm` не выгружен |

Импорт не восстанавливает `BaseForm` по родителю: пустой `BaseForm`
остаётся пустым, а тело родителя в нём хранится таким, каким его записали.
Значит, при импорте платформа не сверяет `BaseForm` с родителем.

Описание формы, импортированное без `FormType`, принимается, но выгружается
без `FormType` и без `Form.xml`: собственное содержимое молча теряется, и
форма переходит в состояние `extension-object-form-inherited.xml` (#1381).
Файл `extension-object-form-borrowed-without-form-type.xml` — эта выгрузка.
С `FormType` `Managed` описание формы, описание владельца и
`Configuration.xml` расширения выгружаются одинаково при любом из
перечисленных `Form.xml`. Платформа добавляет к описанию формы только
`PropertyState Form/Extended`.

Модуль `Ext/Form/Module.bsl` с одним перехватчиком `&После` сохраняется
побайтно рядом с любым принятым `Form.xml`: с выводом v0.12, с пустой своей
частью и с минимальной формой ниже.

Фиксированная точка — вход, который платформа выгружает теми же байтами
уже в первом круге:

```xml
<Form xmlns="http://v8.1c.ru/8.3/xcf/logform" ... version="2.20">
	<AutoCommandBar name="" id="-1"/>
	<Attributes/>
	<BaseForm version="2.20">
		<AutoCommandBar name="" id="-1"/>
		<Attributes/>
	</BaseForm>
</Form>
```

С модулем `&После` и без него эти байты прошли оба круга без изменений.
Их выгрузка лежит здесь: `extension-object-form-borrowed.xml` (описание
формы), `extension-object-form-borrowed-Form.xml`,
`extension-object-form-borrowed-Module.bsl`,
`extension-catalog-with-borrowed-form.xml` (владелец) и
`extension-configuration-with-borrowed-form.xml`.

Чего опыт не доказывает. Входы расширения написаны вручную или получены
из v0.12 `unica.cfe.borrow`: это не вывод команды Конфигуратора
«Добавить в расширение». Опыт проверяет только импорт, `check`, `apply` и
выгрузку через `ibcmd`. Он не проверяет, какую форму клиент собирает
во время работы: не проверено, что пустой `BaseForm` и такая же
собственная часть оставляют форму родителя без изменений, что пустая
собственная часть при полном `BaseForm` удаляет элементы родителя и что
перехватчик `&После` вызывается. Не проверено и поведение при обновлении
родителя.
