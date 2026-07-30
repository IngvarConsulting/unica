---
name: source-access
description: Найти логическую цель 1С, безопасно исследовать её снимок ресурсов и как fallback полностью заменить один существующий BSL-модуль через MCP Unica
argument-hint: <sourceSet> [metadataPath] <inspect|replace-bsl>
allowed-tools:
  - Read
  - Glob
---

# /source-access — логическая навигация и ограниченный fallback

## MCP routing

- Preferred path: сначала выбери предметный writer MCP `unica`. Для точечной
  вставки BSL используй `unica.code.patch`; для формы, DCS, MXL, роли,
  подсистемы или метаданных — соответствующий `unica.*.edit` либо
  `unica.*.compile`.
- Для поиска цели используй `unica.source.resolve` или
  `unica.source.children`. Для исследования уже выбранной цели используй
  `unica.source.resources`, затем `unica.source.read`.
- Когда на руках физический путь — из `unica.code.search`, из диффа, из лога
  сборки — переведи его в логический адрес через `unica.source.locate`, а не
  выводи адрес из раскладки каталогов вручную. `locate` отвечает и владельцем
  файла: для модуля это его объект метаданных, для содержимого формы — сама
  форма. Отказ типизирован: `outsideSourceSet`, `notAddressable` или
  `ownerUnproven`.
- `unica.source.apply` — только fallback, когда ни один предметный writer не
  выражает нужную полную замену существующего BSL-модуля. До вызова явно назови
  причину fallback и почему `unica.code.patch` либо другой предметный
  инструмент не подходит.
- Не вызывай внутренние MCP/CLI-адаптеры и не подменяй логическую цель
  физическим путём. Все операции идут через один MCP `unica`.

## Порядок работы

1. Выбери точный `sourceSet`. Разреши английский или русский запрос через
   `unica.source.resolve`; при исследовании дерева обойди один уровень через
   `unica.source.children`.
   - Вид цели задаёт число сегментов адреса: `Document.ЗаказКлиента` — объект,
     `Document.ЗаказКлиента.ObjectModule` — модуль. Прикладное имя, совпадающее
     с названием роли, объект в модуль не превращает.
   - Корень набора исходников адресуется отсутствием `metadataPath`, а корневые
     модули приложения — голым терминалом, например `ManagedApplicationModule`.
   - В `mode: "prefix"` неполным может быть только канонический английский
     токен: `Doc`, `Su` и `Man` работают, `Документ` работает целиком, а
     неполный псевдоним вроде `Док` отклоняется — у него нет единственной
     канонической формы.
2. Открой `unica.source.resources` с точным `sourceSet` и `metadataPath`, когда
   выбранная цель находится ниже корня набора исходников. Для записи требуй
   `scope: "self"`, `completeness: "complete"`, ровно один ресурс
   `role: "bslModule"` и `access`, содержащий `replace`.
3. Читай ресурс через `unica.source.read` фрагментами до объявленного
   `limits.maxReadBytes`. Сохрани `snapshotId`, `resourceId`, полный `hash`,
   `bomPrefixBytes` и профиль EOL. Продвигайся на возвращённый `length`, а не
   на запрошенный `limit`: фрагмент текстового ресурса усекается до ближайшей
   границы UTF-8, поэтому он бывает короче лимита. `contentEncoding: "base64"`
   означает точные байты, а не текст, который можно молча перекодировать. У
   текстового ресурса он остаётся как минимум там, где лимит уже одного символа
   и где `offset` задан внутри многобайтового символа: `offset` адресует байты,
   а не символы. Всегда смотри на возвращённый `contentEncoding`, а не
   предполагай его.
4. Если предметного writer-а действительно нет, сформулируй причину fallback.
   Не пытайся заменить `metadataDescriptor`, регистрации, формы, DCS, MXL,
   права, двоичный или `unknown` ресурс: ожидаемый отказ —
   `resource_not_replaceable`.
5. Всегда сначала вызови `unica.source.apply` с теми же `snapshotId`,
   `resourceId`, `expectedHash`, полным UTF-8 содержимым и `dryRun: true`.
   Проверь `preHash`, `postHash`, diff, диапазоны, BSL-валидацию и
   проектируемое влияние на кеш.
6. Вызывай тот же план с `dryRun: false` только когда пользователь попросил
   применить именно эту полную замену и предпросмотр всё ещё актуален. При
   `snapshot_expired`, `stale_revision` или `hash_mismatch` открой новый снимок
   и повтори чтение и preview; не обходи отказ.

## Preview fallback

```json
{
  "jsonrpc": "2.0",
  "method": "tools/call",
  "params": {
    "name": "unica.source.apply",
    "arguments": {
      "cwd": "<workspace>",
      "snapshotId": "<snapshotId from unica.source.resources>",
      "resourceId": "<bsl resourceId>",
      "expectedHash": "<sha256 from the same snapshot>",
      "content": "<full replacement BSL>",
      "contentEncoding": "utf-8",
      "dryRun": true
    }
  }
}
```

После подтверждения используйте те же аргументы с `dryRun: false`; изменение
содержимого, снимка или хеша требует нового preview.
