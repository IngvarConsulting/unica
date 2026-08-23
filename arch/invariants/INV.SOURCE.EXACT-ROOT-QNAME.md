---
id: INV.SOURCE.EXACT-ROOT-QNAME
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_owner.rs::exact_declared_target_rejects_a_source_set_owner_with_the_wrong_root
scope: [source]
---

# Цель записи опознаётся по точному QName корня

Существующая объявленная цель проверяется до записи по точному QName корневого
элемента. Документ с корнем другого зарегистрированного вида не заимствует
совместимость окружающего набора исходников и отклоняется.
