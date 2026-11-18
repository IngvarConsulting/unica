# Object Event Handlers

Use this reference when deciding which handler a piece of logic belongs in, or
when reviewing one that already exists.

The normative rules here are development standards, not platform evidence:
`#std396` `ОбработкаЗаполнения`, `#std463` `ОбработкаПроверкиЗаполнения`,
`#std464` `ПередЗаписью`, `#std465` `ПриЗаписи`, `#std466` `ПриКопировании`,
`#std752` `ПередУдалением`, `#std686` the `Отказ` parameter, `#std773` the
`ОбменДанными.Загрузка` flag, `#std455` module structure, and diagnostics
`АПК:75`, `АПК:144`, `АПК:1340`, `АПК:1341`,
`BSLLS:DataExchangeLoading`, `BSLLS:UsingCancelParameter`,
`BSLLS:EventHandlerOutsideEventRegion`,
`BSLLS:MissingEventSubscriptionHandler`. Confirm the current wording with
`unica.docs` with `source: "development-standard"` before citing one.

## Which Handler Owns What

- **`ОбработкаЗаполнения(ДанныеЗаполнения, ТекстЗаполнения, СтандартнаяОбработка)`**
  fills the new object from `ДанныеЗаполнения`, branching on its type, and is
  also where entry by "create based on" is refused — a group used as the basis
  when the command is offered for both groups and items, or an unposted document
  used as the basis. Refuse with `ВызватьИсключение` so the user learns the
  reason, and keep the refusal here rather than in separate based-on commands
  and their handlers (`#std396`).
- **`ОбработкаПроверкиЗаполнения(Отказ, ПроверяемыеРеквизиты)`** checks that
  header attributes, dimensions and resources, and tabular rows are filled
  correctly. Use it where plain required-field checking is not enough: a value
  that depends on other attributes, or a requiredness that depends on conditions
  or functional options (`#std463`).
- **`ПередЗаписью(Отказ, РежимЗаписи, РежимПроведения)`** fills attributes, runs
  correctness checks, checks the object state against external data, and works
  with the *old* values already stored in the database. It is the handler that
  matters most when an already-written object is edited (`#std464`).
- **`ПриЗаписи(Отказ)`** does what belongs to an object that is already written:
  writing related data into other objects, and other after-the-change work. Do
  not modify the object being written — by this point it is in the database
  (`#std465`).
- **`ПриКопировании(ОбъектКопирования)`** clears the attributes whose values must
  not survive into the copy (`#std466`).
- **`ПередУдалением(Отказ)`** does what must happen before the object goes away,
  for example clearing references to it in an owner. Some related deletion is
  automatic and needs no handler — when the reference to the deleted object is a
  `Master` dimension of a register, for instance (`#std752`).
- **`ОбработкаПроведения` and `ОбработкаУдаленияПроведения`** are covered by
  `document-posting.md`, not here.

## Diagnosing A Document Query Mismatch

Start with the exact object class and handler, its caller, and subscriptions on
the same event. A form handler, an object handler and a direct procedure call
are different entry points. Establish the platform version, write mode and
transaction context before assigning a cause.

Read the class-qualified event pages through `unica.docs` with
`source: "platform-help"`. For a document, use `ДокументОбъект.ПередЗаписью`,
`ДокументОбъект.ПриЗаписи`, `ДокументОбъект.ОбработкаПроведения` and
`РежимЗаписиДокумента.Проведение`. Form callbacks need their own pages:
`ПередЗаписьюНаСервере` is outside the platform write transaction, whereas
`ПриЗаписиНаСервере` follows object `ПриЗаписи` before that transaction ends.
Check the returned version and read each full page. A search hit, another
platform version, or a development standard does not establish this timing.

For a synthetic `Document.VisibilityProbe` with a `Marker` attribute, compare
an ordinary query by the same `Ссылка` at three points:

- In object `ПередЗаписью`, compare the pending object value with the state
  before this write. Earlier writes in an outer transaction may already be
  visible, so “before this write” does not mean “last committed version”.
- At entry to `ОбработкаПроведения`, the document has already been written in
  the same transaction. A query of that document is not inherently one write
  behind. Check the actual reference, parameter values, selected field,
  filters, helper calls and subscriptions before diagnosing a reported old value.
- After `Marker` is changed in memory inside the posting handler, the object
  may differ from its last saved state. Identify the last actual write; the
  handler name alone does not prove equality between memory and the query.

Record the observation point, object reference, object value, query parameters
and returned value together. Trace the form-to-object conversion separately;
form data may be absent, cached, or different from the server object. If those
observations are missing, name the explanation as a hypothesis and state the
comparison that would confirm or refute it. Do not patch code from the symptom
alone or claim a runtime reproduction that has not run.

Keep the document row separate from register movements: a record set built in
memory does not establish what a register query can see. Verify its write flags
and actual write point. Other sessions may wait on locks or observe a different
committed state; visibility there is not proved by a query in the writer's own
transaction. [Transactions and responsible reads](transactions-locks.md) own
transaction control and locking; this example adds no new transaction shape.

## Conditional Fill Checking Has One Correct Shape

`#std463` is specific about how conditional requiredness is expressed. Build a
`НепроверяемыеРеквизиты` array, add the names of attributes and tabular sections
that should not be checked this time, and remove them from `ПроверяемыеРеквизиты`
at the end.

Do not do the inverse — adding names to `ПроверяемыеРеквизиты` — and do not
invent another scheme. Both hide the conditional check from analysis of the
`Проверка заполнения` property, so the metadata stops describing what the object
actually requires.

## The Data Exchange Flag Comes First

In `ПередЗаписью`, `ПриЗаписи`, and `ПередУдалением`, check
`ОбменДанными.Загрузка` and return before anything else (`#std773`). During
exchange the object must load as it is: no repeated business logic, no extra
checks, no changes that could distort the data or block the load.

- The same applies to any load mechanism that sets
  `Объект.ОбменДанными.Загрузка = Истина` before writing. A mechanism that does
  not account for a given configuration must be able to write the object as if
  the handler were not there.
- **The obligation extends to event subscription handlers for the same events.**
  A subscription that skips the check reintroduces exactly what the object module
  was careful to avoid.
- Exception: exchange that registers changes during load for forwarding to other
  nodes. With the БСП `Обмен данными` subsystem, disable registration by adding
  `ДополнительныеСвойства.Вставить("ОтключитьМеханизмРегистрацииОбъектов")`.
- Any other exception in a configuration must carry a comment stating the reason.
- Code that sets `ОбменДанными.Загрузка = Истина` takes responsibility for the
  object's integrity. In a distributed infobase that responsibility sits with the
  node where the object was created or changed; elsewhere the caller must fill
  the object correctly itself.

`АПК:75`, `BSLLS:DataExchangeLoading`, and `v8cs:data-exchange-load` all flag a
missing check.

## The Cancel Parameter Is Write-True-Only

- Never assign `Ложь` to `Отказ`, and never assign the result of a boolean
  function to it. Either can silently clear a `Истина` set earlier in the same
  handler or by another subscriber. Write
  `Если ЕстьОшибки() Тогда Отказ = Истина; КонецЕсли;` or
  `Отказ = Отказ Или ЕстьОшибки();` (`#std686`).
- The rule covers every returned boolean parameter of the same shape, including
  `СтандартнаяОбработка` and `Выполнение`.
- Setting `Отказ = Истина` without telling the user why leaves them with the
  platform's own message, which names the object and nothing else. Report the
  reason as a message, or drop `Отказ` and raise an exception instead.

`АПК:144`, `BSLLS:UsingCancelParameter`, and `v8cs:event-handler-boolean-param`
cover this.

## Where Handlers Live

- Event handlers belong in the module's standard event-handler region, and
  procedures that are not event handlers do not (`#std455`, `АПК:1340`,
  `АПК:1341`, `BSLLS:EventHandlerOutsideEventRegion`).
- A subscription declared with no handler procedure is
  `BSLLS:MissingEventSubscriptionHandler`.
- A subscription is invisible from the object module it affects. Any change to an
  object's write or delete path has to look for subscriptions on the same events
  before concluding what runs.

## Stop Rules

- Do not modify the object being written inside `ПриЗаписи`.
- Do not express conditional requiredness by adding to `ПроверяемыеРеквизиты`.
- Do not add logic to a write or delete handler without the
  `ОбменДанными.Загрузка` guard, and do not add the guard to a subscription's
  handler only after someone reports a broken exchange.
- Do not set `Отказ` to anything but `Истина`.
- If public MCP `unica` lacks the operation needed to inspect the object's
  modules, subscriptions, or diagnostics, report a Unica MCP contract gap.
