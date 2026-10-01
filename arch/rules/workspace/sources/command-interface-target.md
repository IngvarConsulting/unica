---
id: INV.SOURCE.COMMAND-INTERFACE-TARGET
check: []
gap: https://github.com/IngvarConsulting/unica/issues/934
---

# Настройка командного интерфейса выбирает его владельца

`commandVisibility.set`, `commandPlacement.set`, `commandOrder.set`
и `groupOrder.set` выбирают интерфейс подсистемы по адресу
`…Subsystem.<Имя>.Interface`. Команда указывается в аргументах.
`subsystemOrder.set` выбирает корень `Configuration`.

Вложенная подсистема сохраняет полную цепочку владельцев;
одноимённая подсистема в другом месте не становится целью.
Отдельного корневого маршрута `main:Interface` нет.
