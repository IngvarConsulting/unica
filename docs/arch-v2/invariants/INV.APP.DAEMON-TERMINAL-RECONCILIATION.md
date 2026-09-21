---
id: INV.APP.DAEMON-TERMINAL-RECONCILIATION
---

# Срок подтверждения конечного результата

Единый absolute terminal deadline захватывается до result preparation/clone,
Arc allocation, thread scheduling и store-channel send. Counting serialization,
store actor wait, file serialization и retry используют его без reset; worker,
начавший после deadline, не вызывает store и переводит daemon в fail-stop.
