---
id: INV.SOURCE.EXACT-ROOT-VERSIONLESS
status: active
governs: product
decision: DEC.2026-08-18.CARRIED-RULES
check: crates/unica-coder/src/infrastructure/platform_xml_owner.rs::exact_declared_versionless_roots_and_absent_outputs_have_no_owner
scope: [source]
---

# Версионно независимые цели не получают владельца формата

Точная цель DCS или MXL с правильным версионно независимым QName, а также
действительно отсутствующий выходной файл не создают владельца версии и
остаются допустимыми для своей объявленной операции.
