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
