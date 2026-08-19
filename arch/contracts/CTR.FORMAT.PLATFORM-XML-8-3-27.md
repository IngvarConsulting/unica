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

# Профиль Platform XML 8.3.27 и сохранение байтов

Порождаемый Platform XML соответствует профилю выгрузки 8.3.27 / формат 2.20 и
сохраняет то, что платформа уже написала: порядок узлов, присутствующие
объявления префиксов пространств имён, BOM и наблюдённый стиль перевода строки.

Свидетельство — настоящие дампы платформы и фикстуры, а не проза: при
расхождении спецификации с доказанным поведением эмиттера правится
спецификация.
