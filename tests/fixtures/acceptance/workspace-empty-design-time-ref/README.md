# Платформенные значения метаданных

Синтетическая Designer2.20 конфигурация для issue #1845: пустой типизированный `FillValue xsi:type="xr:DesignTimeRef"` у `Catalog.Values.Attribute.EmptyItem` должен читаться как незаданное заполнение и сохраняться при добавлении Boolean-реквизита. Соседние Enum.Flags.EnumValue.Yes и TypeSet cfg:Characteristic.Properties позволяют проверить сохранность других значений. Это source fixture, не поставочная конфигурация и не свидетельство загрузки в 1С.
