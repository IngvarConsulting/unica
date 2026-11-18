# Маркеры поддержки 8.3.27

`Ext/ParentConfigurations.bin` из трёх выгрузок платформы 8.3.27.2074
(`DESIGNER /DumpConfigToFiles`, формат 2.20) одной конфигурации на поддержке.
Конфигурация — `tests/fixtures/acceptance/workspace-exchange-content/src` с
заполненными «Поставщик» и «Версия»: из неё сделаны файлы поставки
(`/CreateDistributionFiles -cffile`), и `.cf` загружен в пустую базу
(`/LoadCfg`), поэтому все UUID и имена — тестовые.

- `off.bin` — сразу после загрузки поставки: возможность изменения выключена.
- `editable.bin` — после `/ConfigurationRepositoryCreate -AllowConfigurationChanges
  -ChangesAllowedRule ObjectIsEditableSupportEnabled -ChangesNotRecommendedRule
  ObjectIsEditableSupportEnabled -NoBind` на временном хранилище.
- `locked.bin` — то же с правилами `ObjectNotEditable`.
- `removed.bin` — конфигурация, полностью снятая с поддержки в Конфигураторе:
  16 байт `﻿{6,0,0,0,1,0}`. Одинаковые байты дали выгрузки 8.3.27 и 8.5.1
  раннера (`v8-runner`, стенды `bench/p-new` и `ssl/project`).

Второе поле заголовка `{6,<поле>,…}` — `1`, пока изменения выключены, и `0`
после их включения. Запись объекта — `<правило>,<отметка>,<uuid>,<uuid>`:
правило `0` на замке, `1` редактируется с сохранением поддержки, `2` снят с
поддержки. Отметка поставщика — `0` у объектов, изменения которых он разрешил,
и `-1` у объектов, где не рекомендует; в этой конфигурации поставщик второго
не задавал, а поставка типовой конфигурации 1С пишет обе отметки (#1976).
