---
id: INV.PKG.RUNTIME-CACHE-SERVICE-DATA
check:
  - crates/unica-bootstrap/tests/runtime_install.rs::collecting_does_not_touch_the_lock_and_transaction_areas
---

# Очистка версий сохраняет служебные данные кеша

Очистка версий не удаляет содержимое `.locks`, `.transactions`, `.partial`
и `.attempts`: блокировки, незавершённые установки, недокачанные архивы
и записи о попытках установки. Лимит числа версий к этим каталогам
не применяется.
