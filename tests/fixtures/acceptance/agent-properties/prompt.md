В существующей конфигурации измени три объекта:

- У общего модуля GoogleПереводчик установи Comment в «Agent1721 module»
  и отключи ClientOrdinaryApplication значением JSON false.
- У роли SalesReader установи синоним «Agent1721 role» и Comment
  «Agent1721 role comment». В одном patch используй lowercase `synonym`
  и PascalCase `Comment`: это разные свойства.
- У подсистемы Администрирование установи Synonym в «Agent1721 subsystem»,
  comment в «Agent1721 subsystem comment», очисти Explanation пустой строкой
  и отключи IncludeInCommandInterface значением JSON false. В одном patch
  используй PascalCase `Synonym` и lowercase `comment` и `explanation`.

Остальные свойства, имена, идентификаторы, права роли, состав подсистемы,
командный интерфейс, модули и соседние исходники должны сохраниться.
Проверь результат через Unica. Если check общего модуля требует отсутствующий
анализатор, точно сообщи этот результат проверки.

Объясни по прочитанным контрактам, допустимы ли пустой values, строка "false"
в boolean-поле подсистемы и обе формы Synonym/synonym одновременно.
Также выясни через Unica, можно ли этой же операцией менять Comment корня
Configuration. Дополнительных изменений для этих вопросов не требуется.
