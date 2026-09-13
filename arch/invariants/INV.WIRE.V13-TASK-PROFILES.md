---
id: INV.WIRE.V13-TASK-PROFILES
status: active
governs: product
decision: DEC.2026-08-24.COMPATIBILITY-TASK-TOOLS-SLICE
check: crates/unica-coder/src/interfaces/mcp.rs::v13_compatibility_task_tools_are_profile_gated_durable_and_replay_free
scope: [app, wire]
---

# Hidden V13 выбирает ровно один Tasks-профиль без replay

Запрос с действительной native Tasks authority видит ровно восемь предметных
инструментов; compatibility-запрос видит их же плюс ровно get/result/cancel.
Профиль не переносится между клиентами, legacy session не повышается metadata,
а initial receipt, polling, wait, cancel, reconnect и restart обращаются к одной
durable Invocation и не повторяют её execution. Compatibility projection
принимает только согласованную status/result/failure-presence форму без failure
prose; один absolute monotonic cutoff без duration rebase охватывает connect и
полный exchange, включая post-parse checkpoint и закрытие поздней session.
