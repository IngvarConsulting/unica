---
id: INV.APP.RUNTIME-RESOURCE-TREE
status: active
governs: product
decision: DEC.2026-08-24.LONG-WORK-OWNERSHIP-SLICE
check: crates/unica-coder/src/infrastructure/runtime_jobs.rs::runtime_resource_tree_lease_contract
scope: [app, platform]
---

# Runtime resource освобождается только по поддержанной tree capability

Windows Job Object либо Unix bundled-runner capability из retained unreaped
leader и установленного Unica child-only inherited lifetime sentinel FD
определяют owned tree. Sentinel сохраняется текущим pinned runner без отдельного
handshake/acknowledgement. Завершение
leader при живом подтверждённом descendant сохраняет phase Running и
`active.lock`. Cancellation и drop завершают tree, reap и оба output reader в
одном абсолютном monotonic bounded окне; Drop не создаёт второе окно. Ошибка
startup, fallback, poll, output или persistence передаёт retained process и
readers canonical worker supervision; только поздний terminal+EOF proof может
освободить lock. Потеря/отсутствие capability или не доказанный cleanup оставляют
`active.lock` quarantined и запрещают публикацию доказанной tree terminality,
resource release и signal по освобождённой numeric identity.
Durable compatibility phase `Lost` только классифицирует эту неопределённость и
не разрешает replacement. Runtime SharedWork join выполняется
под lifecycle authority точного физического `active.lock` и UUIDv4 lease; ключ
не выдаётся вызывающему и не выводится из аргументов. Quarantine release удаляет
lease descriptor-relative только из сохранённого jobs-root; replacement root
не является authority даже с совпадающим текстом job id.
