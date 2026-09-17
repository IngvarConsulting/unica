---
id: INV.APP.RUNTIME-RESOURCE-TREE
---

# Механизм владения runtime и общий бюджет очистки

Windows Job Object либо Unix bundled-runner capability из retained unreaped
leader и установленного Unica child-only inherited lifetime sentinel dynamic FD
определяют owned tree. Sentinel сохраняется текущим pinned runner без отдельного
handshake/acknowledgement. Cancellation и drop завершают tree, reap и оба output reader в
одном абсолютном monotonic bounded окне; Drop не создаёт второе окно.

Один исходный job-directory capability принимается до initial spawn либо до
принятия attach process ownership и переносится через normal и fallback
lifecycle без повторного разрешения `jobs/<id>`.
