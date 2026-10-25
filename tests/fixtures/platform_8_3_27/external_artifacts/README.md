# Выгрузка внешней обработки и внешнего отчёта 8.3.27

`epf/` и `erf/` — нетронутые иерархические выгрузки Конфигуратора 1С
8.3.27.2074, формат 2.20: внешняя обработка `SyntheticProcessor` и внешний
отчёт `SyntheticReport`. Это эталон байтов для писателей Unica во внешние
наборы (#2033). `manifest.json` фиксирует SHA-256 и размер каждого файла и
хеши бинарей платформы; `tests/ci/test_external_artifacts_fixture.py` сверяет
его с файлами и проверяет, что Git не нормализует их байты.

Данные синтетические, без частных сведений. UUID получены детерминированно
(`uuid5`) при сборке входа и сохранены платформой как есть.

## Состав

Оба набора:

- реквизиты `TextValue` (Строка 50), `Amount` (Число 15.2), `DocDate`
  (Дата, только дата), `Flag` (Булево);
- табличная часть `Lines` с реквизитами `Item` (Строка 100) и `Quantity`
  (Число 10.3, неотрицательное);
- `DefaultForm` = форма `Main`, `AuxiliaryForm` = форма `Auxiliary`;
- форма `Main`: событие формы `OnCreateAtServer`, поле `TextValue` с событием
  `OnChange`, таблица `Lines` по табличной части с двумя колонками, поле
  реквизита формы `Counter`, кнопка `RefreshButton` команды формы `Refresh`;
  модуль формы `Module.bsl`;
- `Ext/ObjectModule.bsl`;
- макеты `Layout` (SpreadsheetDocument; тело взято без изменений из
  `../mxl_edit/Template.xml` и выгружено байт в байт), `Notes`
  (TextDocument), `Blob` (BinaryData, 16 байт `00..0F`).

Только отчёт:

- `MainDataCompositionSchema` = макет `MainSchema` (DataCompositionSchema):
  локальный источник, набор-запрос `ВЫБРАТЬ &Label КАК Label, &Amount КАК
  Amount` без таблиц конфигурации, два параметра, итог `Сумма(Amount)`,
  вариант `Main`;
- `DefaultSettingsForm` = форма `Settings` (`ReportFormType` Settings),
  `DefaultVariantForm` = форма `Variant` (`ReportFormType` Variant),
  `AuxiliarySettingsForm`, `VariantsStorage` и `SettingsStorage` пусты;
- форма `Main` — основная форма отчёта: `ReportResult`, `DetailsData`,
  `ReportFormType` Main, поле табличного документа `Result`, событие
  `OnLoadVariantAtServer` с обработчиком в модуле формы.

## Как получено

Все базы — новые пустые файловые базы, созданные для опыта и затем удалённые.
`<1cv8>` — `1cv8` из каталога 8.3.27.2074. Запуск без пользователей и без
сервера.

```sh
<1cv8> CREATEINFOBASE "File=<ib1>;Locale=ru_RU" /DisableStartupDialogs /Out <log>
<1cv8> DESIGNER /F <ib1> /DisableStartupDialogs /DisableStartupMessages /Out <log> \
  /LoadExternalDataProcessorOrReportFromFiles <input>/epf/SyntheticProcessor.xml <bin1>/SyntheticProcessor.epf
<1cv8> DESIGNER /F <ib1> /DisableStartupDialogs /DisableStartupMessages /Out <log> \
  /DumpExternalDataProcessorOrReportToFiles <round1>/epf/SyntheticProcessor.xml <bin1>/SyntheticProcessor.epf \
  -Format Hierarchical
```

То же для `erf/SyntheticReport.xml` и `SyntheticReport.erf`. Круг 2: новая
база `<ib2>`, загрузка из `<round1>`, выгрузка в `<round2>`. Все команды
вернули 0; журнал загрузки — «Загрузка завершена», без предупреждений.

`<round1>` и `<round2>` совпали побайтно во всех 32 файлах. Сюда положен
`<round1>`. Повтор тех же двух кругов из файлов этого каталога в двух новых
базах снова дал те же байты: каталог воспроизводит сам себя.

Двоичные `.epf` и `.erf` от круга к кругу различаются (SHA-256 разные при
одинаковом содержимом выгрузки), поэтому не хранятся и эталоном не служат.

## Что доказано и что нет

Доказано: эти байты — неподвижная точка загрузки и выгрузки 8.3.27.2074 для
этого состава; платформа приняла каждую часть входа. Выгрузка внешнего набора
не содержит `ConfigDumpInfo.xml`.

Не доказано: работа обработки и отчёта в режиме Предприятия (формы не
открывались, отчёт не формировался, модули не исполнялись); поведение
с конфигурацией в базе; другие версии платформы; что любые другие XML
выгружаются так же. Синтаксис модулей не проверялся.

## Что платформа не приняла

Пробы вне эталона, на отдельной пустой базе:

- Вход без `InternalInfo` у объекта и табличной части отвергнут (код 1):
  «узел - ExternalDataProcessor: Отсутствует внутренняя информация (узел
  InternalInfo) для объекта ExternalDataProcessor.», то же для
  `TabularSection` и `ExternalReport`, затем «Ошибка загрузки документа.».
- Ссылочный тип без конфигурации отвергнут (код 1): «Неизвестное имя типа -
  CatalogRef.Missing», «Ошибка загрузки документа.». Поэтому в эталоне только
  примитивные типы.
- Имя `ExternalDataProcessorTabularSection.<Имя>.<ТЧ>` в `GeneratedType`
  табличной части принято, но выгружено как `DataProcessorTabularSection…`:
  имя типа платформа выводит сама, UUID `TypeId` и `ValueId` берёт из входа.

## Что меняет платформа относительно входа

Вход и журналы проб в репозиторий не положены: это запись наблюдения при
сравнении входа с выгрузкой круга 1, а не проверяемое здесь доказательство.
Проверяемы только байты выгрузки и их неподвижность.

- Все XML и `.bsl` выгружаются с BOM и CRLF, без перевода строки в конце XML;
  `.bsl` оканчиваются CRLF. Вход был без BOM и с LF, кроме тела `Layout`,
  которое уже было выгрузкой платформы.
- Идентификаторы `id` элементов, реквизитов и команд формы и порядок
  элементов сохранены как во входе, включая пропуски номеров (реквизиты
  основной формы отчёта 1, 2, 4, 5) и поле `Result` после кнопки.
- Объявления пространств имён (`xmlns:dcssch` в `Form.xml`) и
  `UsePurposes` с `MobilePlatformApplication` пришли из входа и сохранены;
  убирает ли их платформа при отсутствии, не проверялось.
- Тела `Template.txt` и `Template.bin` выгружены теми же байтами, что во
  входе: без BOM, LF в тексте.
- В тексте запроса СКД переводы строк остаются LF внутри файла с CRLF.
- Табличной части добавлен блок `StandardAttributes` со стандартным
  реквизитом `LineNumber` (25 свойств `xr:`, включая `FullTextSearch` Use и
  `DataHistory` Use).
- Таблице формы добавлен `<RowFilter xsi:nil="true"/>` после `DataPath`.
- Каждой форме отчёта добавлены `AutoShowState`, `ReportResultViewMode` и
  `ViewModeApplicationOnSetReportResult` со значением `Auto` после
  `ReportFormType`. Форме `Auxiliary`, где `ReportFormType` не был задан,
  платформа записала `ReportFormType` Main.
- События формы отчёта переупорядочены: `OnLoadVariantAtServer` выгружен
  перед `OnCreateAtServer`, хотя во входе шёл после.
- СКД переформатирована отступами; состав элементов не изменился.

## Расхождения со спецификациями

Спецификации в этом изменении не правятся.

- `1c-epf-spec.md`, раздел 9, подраздел «BOM в файлах метаданных»: `Form.xml` и
  `.bsl`-модули названы файлами без BOM. В выгрузке 8.3.27.2074 BOM есть
  у обоих.
- `1c-erf-spec.md`, раздел 4: в примере табличной части нет
  `StandardAttributes`. Платформа выгружает `LineNumber` и у табличной части
  отчёта, как и у обработки.
- `1c-erf-spec.md`, разделы 6.2–6.5: нет `AutoShowState`,
  `ReportResultViewMode` и `ViewModeApplicationOnSetReportResult`, которые
  платформа пишет в каждую форму отчёта, и не сказано, что форма без
  `ReportFormType` получает Main. В `1c-form-spec.md` эти три свойства есть.
- `1c-form-spec.md`, «Свойства отчётов»: значения `ReportFormType` названы
  `Main`, `Settings`, `Choice`; `1c-erf-spec.md` называет `Variant`.
  Платформа приняла и выгрузила `Variant`.
- `1c-epf-spec.md`, раздел 6.3: в примерах `Table` нет `RowFilter`.
- `1c-form-spec.md`, раздел 6: среди событий формы и в наборе событий отчёта
  нет `OnLoadVariantAtServer`. Ни одна спецификация не задаёт порядок событий;
  выгрузка ставит `OnLoadVariantAtServer` перед `OnCreateAtServer`.
- `1c-epf-spec.md`, раздел 2.2: в объявлениях `Form.xml` нет `xmlns:dcssch`;
  выгрузка его сохраняет (в `1c-form-spec.md` он есть).
- `1c-epf-spec.md`, раздел 3: примечание об отсутствии `FullTextSearch` и
  `DataHistory` относится к пользовательским реквизитам; у стандартного
  `LineNumber` табличной части оба есть со значением Use.
- `GeneratedType` табличных частей совпадает со спецификациями:
  `DataProcessorTabularSection[Row]` и `ReportTabularSection[Row]`.

## Бинари

`designerBinarySha256` — `1cv8`, которым выполнены все команды.
`ibcmdSha256` — `ibcmd` того же каталога; он совпадает с
`platformBinarySha256` в `../cfe_borrow/manifest.json`, то есть это та же
поставка 8.3.27.2074, что у прежних фикстур.
