---
id: CTR.FORMAT.PLATFORM-XML-8-3-27
status: active
governs: product
version: 1
decision: null
producer: crates/unica-coder/src/infrastructure/native_operations/
consumers: [platform]
check: crates/unica-coder/tests/format_8_3_27_xml_corpus.rs::source_resource_reads_preserve_every_corpus_byte
---

# Чтение ресурсов сохраняет байты корпуса Platform XML 8.3.27

Ресурсное чтение настоящих дампов и фикстур профиля 8.3.27 / формат 2.20
возвращает каждый файл корпуса побайтно. При расхождении спецификации с
доказанным поведением платформы правится спецификация.
