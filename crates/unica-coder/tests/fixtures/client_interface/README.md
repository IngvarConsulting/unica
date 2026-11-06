# Панели и начальная страница корня 8.3.27

Фикстура доказывает, что `panels.set` и `homePage.set` пишут те же байты,
что платформа 1С 8.3.27.2074 (формат 2.20) выгружает после загрузки
отредактированной конфигурации. Тест
`panels_and_home_page_publish_the_platform_bytes_and_read_back` в
`crates/unica-coder/src/infrastructure/daemon/server/client_interface_tests.rs`
применяет операции к копии `tests/fixtures/acceptance/workspace-borrowed`
и сравнивает `src/cf/Ext/ClientApplicationInterface.xml` и
`src/cf/Ext/HomePageWorkArea.xml` с файлами отсюда побайтно.

- `two/` — `top: [["tools","open"]]`, `left: ["sections"]`,
  `bottom: ["favorites"]`; шаблон `TwoColumnsEqualWidth`, слева общая форма
  высотой 15, скрытая для всех, кроме роли `SalesReader`, справа форма
  обработки.
- `one/` — `left: [["sections","history"]]`, `right: ["functions"]`;
  шаблон `OneColumn` с формой обработки высотой 5.

Как получены (временная файловая база, лицензия не нужна):

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
"$ibcmd" infobase create "${connection[@]}" --import=./edited-cf --apply --force
"$ibcmd" config export "${connection[@]}" ./reexport
```

`edited-cf` — `workspace-borrowed/src/cf` после публикации Unica. Повторная
выгрузка совпала с записью Unica байт в байт.

Те же опыты задали правила операций:

- платформа сохраняет переданные идентификаторы панелей и групп, поэтому
  Unica оставляет имеющиеся и берёт для новых наименьшие свободные
  `00000000-0000-0000-0000-…`, а не случайные;
- «Панель инструментов» (`8e10648b-…`, `tools`) выживает только в `top` и
  только при своём `panelDef`; в других областях и без объявления
  платформа молча её выбрасывает;
- шаблон `OneColumn` пишется элементом `Column`: на `LeftColumn` загрузка
  падает с ошибкой формата;
- форма или роль, которой нет в конфигурации, роняет загрузку
  (`Несоответствие свойства и элемента данных XDTO`, `Неизвестный объект
  метаданных`), поэтому план проверяет их наличие.
