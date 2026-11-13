# Borrowed objects: platform evidence (2.22)

These descriptors are what 1C **8.5.4.2028** exports for an extension (format
**2.22**, the `empty_extension` corpus) that borrows objects of the `all_kinds`
corpus of the same format. The extension was loaded with
`DESIGNER /LoadConfigFromFiles ... -Extension` into a disposable file infobase
and exported with `/DumpConfigToFiles ... -Extension`.

- `extension-enum-value.xml`: `Enum.СпособыУстановкиКурсаВалюты` with its
  borrowed value `ЗагрузкаИзИнтернета`.
- `extension-form-borrowed.xml`, `extension-form-borrowed-Form.xml`:
  `Catalog.Валюты.Form.ФормаЭлемента`, loaded from the files Unica wrote for
  `object.borrow`.
- `extension-document.xml`: `Document.Приход`.
- `extension-catalog-members.xml`: `Catalog.Валюты` with the borrowed attribute
  `Наценка` and the borrowed tabular section `Представления` with its attribute
  `КодЯзыка`.
- `extension-common-module.xml`: `CommonModule.EventHandlers`.

Every borrowed object and member, the enumeration value, attributes and the
tabular section included, has
`<ActionOnConfigurationObjectMissing>Auto</ActionOnConfigurationObjectMissing>`
right after `ExtendedConfigurationObject`; the export writes it even when the
loaded file has none. The borrowed form descriptor then also has
`<ActionOnConfigurationExtensionChangeError>Warning</ActionOnConfigurationExtensionChangeError>`
before `FormType`. Format 2.21 (8.5.1) writes neither
(`../../2.21/cfe_borrow/`). The borrowed `Form.xml` is exported unchanged.

The borrowed document has an empty `<RegisterRecords/>`, as 8.3.27 exports it
(`../../2.20/incomplete_borrowed/`).
