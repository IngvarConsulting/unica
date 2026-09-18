---
id: CTR.SOURCE.MODULE-PROJECTION-SHAPE
---

# Неперенесённые предметные поля проекций модуля

Сводка не содержит методы или исходный текст. Она объявляет шесть ветвей
в порядке `Method`, `Region`, `Interface`, `Event`, `Compilation`, `Body`;
их счётчики соответствуют содержимому. Допустимый модуль без файла имеет
нулевые ветви, производные от исходника, но сохраняет применимые события.

`Method` передаёт точную декларацию, documentation, kind, export,
compilation facts, handles и extension target; `Region` — полный вложенный
адрес; `Interface` — одну из трёх подсистем; `Event` — состояние, двуязычный
handler, точную сигнатуру, контексты и binding; `Compilation` — диапазон
и effective contexts; `Body` — исходные строки с номерами и pagination.

Восемь нормализованных свойств общего модуля появляются один раз в его
`props`; `serverCall`, `privileged` и `returnValuesReuse` не становятся
контекстами.
