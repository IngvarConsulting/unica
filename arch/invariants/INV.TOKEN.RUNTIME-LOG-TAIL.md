---
id: INV.TOKEN.RUNTIME-LOG-TAIL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::truncated_worker_stdout_cannot_authorize_full_fallback
scope: [app, product]
---

# В памяти удерживается ограниченный хвост вывода задания

Рабочий поток обрезает длинный stdout до точного предела удерживаемого хвоста;
усечённый вывод не используется как доказательство полного результата.
