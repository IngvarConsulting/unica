# Права роли на объекты внешнего источника: эталон платформы

Оба `Rights.xml` выгружены платформой **8.3.27.2342** (формат XML **2.20**) из
временной файловой базы. Конфигурация базы — выгрузка
`../all_kinds/src` со внешним источником `AllKindsExternalSource`: таблица
`Items` с полем `Code`, куб `Sales` с измерением `Product`, ресурсом `Amount`
и таблицей измерения `Products` (поле `Code`), функция `Total`.

В роль `SalesReader` перед загрузкой записывались все имена прав, известные
`unica-roles`, и права изменения аутентификации — на каждый объект источника.
Платформа молча отбрасывает право, неприменимое к объекту, поэтому выгрузка
оставляет ровно применимые.

- `objects/Rights.xml` — все права со значением `true` на источник, таблицу,
  куб и таблицу измерения. Права членов со значением `true` совпадают
  с умолчанием (`setForAttributesByDefault`) и не пишутся.
- `members/Rights.xml` — все права со значением `false` на поле таблицы,
  поле таблицы измерения, измерение и ресурс куба и функцию.

Команды (PowerShell, пути относительно стенда):

```powershell
$exe = "C:\Program Files\1cv8\8.3.27.2342\bin\1cv8.exe"
Start-Process $exe -ArgumentList 'CREATEINFOBASE','File="ib";Locale="ru_RU"' -Wait
Start-Process $exe -ArgumentList 'DESIGNER','/F','ib','/DisableStartupDialogs','/LoadConfigFromFiles','src' -Wait
Start-Process $exe -ArgumentList 'DESIGNER','/F','ib','/DisableStartupDialogs','/DumpConfigToFiles','dump' -Wait
```

Выгрузка делалась в пустой каталог после каждой загрузки.
