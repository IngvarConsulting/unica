---
name: cfe-borrow
description: Заимствование объектов из конфигурации 1С в расширение (CFE). Используй когда нужно перехватить метод, изменить форму или добавить реквизит к существующему объекту конфигурации
argument-hint: -ExtensionPath <path> -ConfigPath <path> -Object "Catalog.Контрагенты.Form.ФормаЭлемента" -BorrowMainAttribute
allowed-tools:
  - Bash
  - Read
  - Glob
---

# /cfe-borrow — Заимствование объектов из конфигурации

## MCP routing

- **Канонической операции заимствования на поверхности нет.** Разбор трёх
  ролей (`own`, `borrowed`, корень расширения), проекция в `props` и форма
  будущей операции `object.borrow` держит отдельная
  [задача #935](https://github.com/IngvarConsulting/unica/issues/935).
  Согласованы проверенный адрес родителя и обязательность заимствования
  объектов верхнего уровня для v0.13. Повтор обновляет свойства из родителя,
  сохраняя локальные перекрытия и идентификаторы; реализация отслеживается
  в той же задаче. Заимствование форм отложено в
  [#937](https://github.com/IngvarConsulting/unica/issues/937) и v0.13 не блокирует.
  Остальной контракт уточняется; операция ещё не реализована.
- Не зови внутренние адаптеры напрямую: они спрятаны за MCP `unica`.
- Готовность проверяет `unica.check`, результат читает `unica.view` по адресу.

Что делать сейчас: сформировать дескриптор заимствованного объекта по формату
ниже (`ObjectBelonging>Adopted` плюс `ExtendedConfigurationObject` с UUID
объекта родителя), зарегистрировать его в `ChildObjects` корня расширения,
затем проверить `unica.check` и прочитать `unica.view`. Признак заимствования —
именно `ExtendedConfigurationObject`: корень расширения тоже `Adopted`, и
читать его как заимствованный объект — дефект.

Если нужна операция — сообщи о пробеле контракта Unica MCP и сошлись на
[задачу #935](https://github.com/IngvarConsulting/unica/issues/935).

## Примеры дескриптора

Ниже — то, что должен нести записанный дескриптор заимствованного объекта в
наборе расширения. UUID берётся из дескриптора того же объекта у родителя.

### Заимствованный объект метаданных

```xml
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses" version="2.20">
  <Catalog uuid="<uuid объекта в расширении>">
    <InternalInfo/>
    <Properties>
      <ObjectBelonging>Adopted</ObjectBelonging>
      <Name>Контрагенты</Name>
      <Comment/>
      <ExtendedConfigurationObject><uuid объекта у родителя></ExtendedConfigurationObject>
    </Properties>
    <ChildObjects/>
  </Catalog>
</MetaDataObject>
```

Имя обязано совпадать с именем у родителя, а сам объект — быть зарегистрирован
в `ChildObjects` корня расширения.

### Заимствованный объект с перекрытым свойством

Перекрытия несёт `InternalInfo` списком, а не флагом. Префикс `xr` объявляется
на корне дескриптора — фрагмент ниже без этого объявления не разбирается:

```xml
<MetaDataObject xmlns="http://v8.1c.ru/8.3/MDClasses"
                xmlns:xr="http://v8.1c.ru/8.3/xcf/readable" version="2.20">
  <Catalog uuid="<uuid объекта в расширении>">
    <InternalInfo>
      <xr:PropertyState>
        <xr:Property>Synonym</xr:Property>
        <xr:State>Extended</xr:State>
      </xr:PropertyState>
    </InternalInfo>
    <Properties>
      <ObjectBelonging>Adopted</ObjectBelonging>
      <Name>Контрагенты</Name>
      <ExtendedConfigurationObject><uuid объекта у родителя></ExtendedConfigurationObject>
    </Properties>
  </Catalog>
</MetaDataObject>
```

Платформа объявляет на корне весь свой набор пространств имён; здесь показаны
только те два, без которых пример не читается.

Платформенной улики на эту форму у объекта метаданных пока нет: её снимает
круговой путь из [задачи #935](https://github.com/IngvarConsulting/unica/issues/935).
Пока улики нет, считай
перекрытие непроверенным фактом и говори об этом в ответе.

### Заимствованная форма

Дескриптор формы устроен так же: `Adopted` плюс `ExtendedConfigurationObject`
формы родителя. Реквизиты и элементы формы заимствуются вместе с ней; выборочно
перенести часть реквизитов поверхность не умеет — это тот же пробел контракта.

## Верификация

Проверка расширения — `unica.check` на корне набора-расширения (`ext` — имя набора типа `EXTENSION` в `v8project.yaml`); валидатор `cfe` выбирается по виду набора, вердикт в `data.status`.

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.check",
    "arguments": {
      "at": "ext:Configuration"
    }
  }
}
```
