---
id: INV.APP.DAEMON-STORE-FAIL-STOP
status: active
governs: product
decision: DEC.2026-08-24.DAEMON-INVOCATION-ROUTING-SLICE
check: crates/unica-coder/src/infrastructure/daemon/mod.rs::daemon_store_is_bounded_and_fail_stop_is_process_owned
scope: [app, cache]
---

# Daemon store bounded, а fail-stop завершается только смертью процесса

Executor обращается к sole-writer store только через один serial store actor с
bounded channel и общим absolute monotonic deadline/cancellation. Зависший
adapter или syscall не удерживает caller: daemon закрывает admission и просит
процесс завершиться, не выдавая staged result и не запуская domain execution
повторно.

File store ограничивает writer acquisition, размер record, recovery enumeration
и число retained records. Create использует preallocated TaskId и атомарную
публикацию без замены; collision типизирован. При capacity удаляются только
истёкшие terminal records; active и неистёкшие records не вытесняются.

`RestartRequested` не означает, что in-process resources уже освобождены.
Listener закрывается, PID-bound endpoint остаётся до смерти процесса, и только
после неё successor получает sole-writer ownership, заменяет stale endpoint и
закрывает оставшийся `Working` через recovery без второго execution.
