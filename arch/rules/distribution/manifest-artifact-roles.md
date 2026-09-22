---
id: INV.PKG.MANIFEST-ARTIFACT-ROLES
check:
  - crates/unica-bootstrap/tests/manifest_contract.rs::loading_a_manifest_rejects_an_unknown_artifact_role
---

# Неизвестная роль артефакта делает манифест недопустимым

В манифесте поставки поле `role` принимает `core` для ядра и `engine`
для внешнего движка. При чтении манифеста bootstrap отклоняет неизвестную
роль как ошибку конфигурации. Он не приписывает ей смысл одной из известных
ролей по умолчанию.
