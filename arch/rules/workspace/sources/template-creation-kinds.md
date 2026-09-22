---
id: INV.SOURCE.TEMPLATE-CREATION-KINDS
check: []
gap: https://github.com/IngvarConsulting/unica/issues/992
---

# Создание макета поддерживает пять видов

Через канонический `apply` можно создать макет `HTMLDocument`,
`TextDocument`, `SpreadsheetDocument`, `BinaryData` или
`DataCompositionSchema`. Новый макет зарегистрирован у владельца и имеет
корректное начальное содержимое своего вида.

Заготовка пригодна для загрузки платформой. Допустимость пустого содержимого
зависит от вида макета; отсутствие пользовательских данных не разрешает
создавать файл произвольного формата.
