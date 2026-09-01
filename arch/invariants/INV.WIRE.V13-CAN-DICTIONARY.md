---
id: INV.WIRE.V13-CAN-DICTIONARY
status: active
governs: product
decision: DEC.2026-09-01.OPERATION-DICTIONARY-LIVES-IN-CAN
check: crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_refusals_answer_one_diagnostics_channel_from_the_closed_code_set
scope: [app, wire]
---

# Словарь операций достижим из can и не раздваивается

Запрошенная секция `can` узла metadata-семейства или корня вычисляется из
одного закрытого реестра операций × применимости к виду узла — того же,
которым сервер валидирует `apply`; элемент несёт `op`, скелет `args` и
честный `implemented`. Без явного запроса секций словарь в проекцию не
попадает. Вид узла с невычисляемым словарём и невычисляемая секция отвечают
typed `unsupported_section`, а не валидным пустым узлом. Отказ по неверному
ключу аргументов называет ожидаемый скелет операции.
