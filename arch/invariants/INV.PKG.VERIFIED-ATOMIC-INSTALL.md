---
id: INV.PKG.VERIFIED-ATOMIC-INSTALL
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-bootstrap/tests/runtime_install.rs
scope: [pkg]
---

# Runtime проверяется контрольной суммой и ставится атомарно

Загруженный runtime сверяется по SHA-256 и публикуется в кеш хоста атомарно; неполная
загрузка не получает маркер готовности. Многофайловый инструмент входит в поставку целиком.
