# Borrowed enumeration value: platform evidence (2.21)

`extension-enum-value.xml` is the descriptor 1C **8.5.1.1529** exports for an
extension (format **2.21**, the `empty_extension` corpus) that borrows
`Enum.СпособыУстановкиКурсаВалюты.EnumValue.ЗагрузкаИзИнтернета` from the
`all_kinds` corpus of the same format.

The extension was loaded with `DESIGNER /LoadConfigFromFiles ... -Extension` and
exported with `/DumpConfigToFiles ... -Extension` into a disposable file
infobase. A borrowed value is exported without `Color`, even when the loaded
file carried `<Color>auto</Color>`; a new value of format 2.21 has it.

`extension-form-borrowed.xml` and `extension-form-borrowed-Form.xml` are what 8.5.1.1529
exports for `Catalog.Валюты.Form.ФормаЭлемента` borrowed into the same extension:
the extension was loaded from the files Unica wrote for `object.borrow` and
exported again. The export is byte for byte the same; a borrowed form has no
`UseInInterfaceCompatibilityMode`, which a new form of format 2.21 has.
