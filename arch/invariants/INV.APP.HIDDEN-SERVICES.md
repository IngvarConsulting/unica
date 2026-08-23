---
id: INV.APP.HIDDEN-SERVICES
status: active
governs: product
decision: DEC.2026-08-23.USER-CORE-DAEMON-SLICE
check: crates/unica-coder/src/infrastructure/workspace_services.rs::hidden_service_identity_distinguishes_user_core_daemon_from_workspace_helpers
scope: [app]
---

# Служебная топология отделяет пользовательский executor от workspace helpers

Executor ключуется пользователем, версией протокола и совместимой идентичностью
ядра, но не workspace. Существующие служебные helpers до удаления compatibility
слоя остаются ключеваны каноническими корнями workspace и набора исходников.
