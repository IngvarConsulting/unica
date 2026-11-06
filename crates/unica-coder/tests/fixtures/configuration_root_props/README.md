# Свойства корня конфигурации и расширения 8.3.27

Фикстура доказывает, что `props.set` на `<набор>:Configuration` пишет те же
байты, что платформа 1С 8.3.27.2074 (формат 2.20) выгружает после загрузки
отредактированного корня. Тесты
`configuration_root_properties_match_the_platform_reexport` и
`extension_root_properties_match_the_platform_reexport` в
`crates/unica-coder/src/infrastructure/native_operations/apply_families/metadata.rs`
применяют правку и сравнивают результат с файлом отсюда побайтно.

- `main/Configuration.xml` — корень из `../empty_configuration/src` после
  правки всех свойств профиля конфигурации: синонима, комментария,
  поставщика, версии, адресов, сведений, флагов справки и форм и восьми перечислений — всего 21 свойство.
- `extension/source-Configuration.xml` — корень расширения `Корректировки`,
  созданного `ibcmd extension create` над той же пустой конфигурацией и
  выгруженного `ibcmd config export --extension`. Ключ `--name-prefix`
  ibcmd в корень не записал: `NamePrefix` в выгрузке пуст, языка у
  расширения нет.
- `extension/Configuration.xml` — тот же корень после правки комментария,
  поставщика, версии и режима совместимости расширения.

Как получен `main/Configuration.xml` (файловая база во временном каталоге):

```bash
v8=/opt/1cv8/8.3.27.2074/1cv8
"$v8" CREATEINFOBASE File=./ib
"$v8" DESIGNER /F ./ib /LoadConfigFromFiles ./edited-src
"$v8" DESIGNER /F ./ib /DumpConfigToFiles ./reexport
```

`edited-src` — `empty_configuration/src` с корнем, записанным Unica. Как
получены файлы расширения:

```bash
ibcmd=/opt/1cv8/8.3.27.2074/ibcmd
"$ibcmd" infobase create "${connection[@]}" --import=./empty_configuration/src --apply --force
"$ibcmd" extension "${connection[@]}" create --name=Корректировки --name-prefix=Корр_ --purpose=customization
"$ibcmd" config "${connection[@]}" export --extension=Корректировки ./export
"$ibcmd" config "${connection[@]}" import --extension=Корректировки ./edited
"$ibcmd" config "${connection[@]}" check --extension=Корректировки --force
"$ibcmd" config "${connection[@]}" apply --extension=Корректировки --force
"$ibcmd" config "${connection[@]}" export --extension=Корректировки ./reexport
```

Повторная выгрузка совпала с записью Unica байт в байт; `ConfigDumpInfo.xml`
в сравнение не входит.

Тот же приём определил границы профиля. Платформа читает режим основного
окна как `Normal`, `Workplace`, `EmbeddedWorkplace`, `FullscreenWorkplace`
и `Kiosk`, а на `Fullscreen` загрузка падает с ошибкой XDTO. `CompatibilityMode` со
значением `DontUse` возвращается как `Version8_3_8`, а
`ConfigurationExtensionCompatibilityMode` основной конфигурации
возвращается как `Version8_3_27` при любом записанном значении. Поэтому
`DontUse` и режим совместимости расширений основной конфигурации в профиль
`props.set` не входят.
