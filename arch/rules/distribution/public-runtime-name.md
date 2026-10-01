---
id: INV.PKG.PUBLIC-BINARY-NAME
check:
  - tests/ci/test_version_contract.py::VersionContractTests.test_public_runtime_binary_name_is_unica
---

# Исполняемый файл ядра называется unica

Пакет Cargo `unica-coder` объявляет исполняемый файл `unica`.
Запись `unica` в `tools.lock.json` указывает то же имя файла и тот же
Cargo-пакет, чтобы сборщик находил ядро по его публичному имени.
