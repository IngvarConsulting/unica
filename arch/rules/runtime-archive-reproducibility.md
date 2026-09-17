---
id: INV.PKG.RUNTIME-ARCHIVE-REPRODUCIBLE
check:
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_archive_is_deterministic_and_target_only
---

# Повторная упаковка даёт одинаковые байты

Повторная упаковка одного неизменного runtime bundle даёт побайтово одинаковые
архив ядра и JSON-манифесты. Связанная проверка подтверждает эту гарантию
на bundle `linux-x64`; остальные targets ею не проверяются.
