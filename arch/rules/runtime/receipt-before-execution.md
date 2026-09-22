---
id: INV.APP.RECEIPT-BEFORE-EXECUTION
check:
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::reserve_precedes_validation_admission_prepare
  - crates/unica-coder/tests/daemon_receipt_ledger.rs::malformed_outer_envelope_creates_no_receipt_and_runs_no_domain_code
---

# Квитанция сохраняется до проверки и подготовки операции

После разбора сообщения демон сохраняет квитанцию с идентичностью вызова
и заранее выделенным `taskId`. Только затем начинаются предметная проверка
аргументов, допуск к рабочему пространству и подготовка операции. Их отказ
не теряет квитанцию: она получает соответствующий результат.

Некорректная внешняя форма сообщения отклоняется раньше: она не создаёт
квитанцию или задание и не запускает предметный код.
