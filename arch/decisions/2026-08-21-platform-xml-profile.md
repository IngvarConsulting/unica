---
id: DEC.2026-08-21.PLATFORM-XML-PROFILE
status: active
governs: product
realized: crates/unica-coder/tests/format_8_3_27_xml_corpus.rs::source_resource_reads_preserve_every_corpus_byte
establishes: [CTR.FORMAT.PLATFORM-XML-8-3-27]
---

# Resource reads preserve the Platform XML 8.3.27 corpus

**Решение.** Ресурсное чтение дампов и фикстур профиля Platform XML 8.3.27 /
формат 2.20 сохраняет каждый файл корпуса побайтно.
