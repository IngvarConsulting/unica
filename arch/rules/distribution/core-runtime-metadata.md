---
id: INV.CI.RUNTIME-METADATA-HASHES
check:
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_metadata_hashes_archive_and_each_runtime_file
---

# Описание ядра соответствует сформированному архиву

Метаданные ядра содержат SHA-256 его runtime-архива и полный список файлов
внутри. Для каждого файла указаны SHA-256 его байтов и признак исполняемости,
совпадающий с правами в архиве. В описание не попадают файлы внешних движков:
их поставки описываются отдельно.
