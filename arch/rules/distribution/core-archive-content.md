---
id: INV.CI.RUNTIME-ARCHIVE-DETERMINISTIC
check:
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_archive_is_deterministic_and_target_only
---

# Архив ядра содержит ядро выбранной платформы и манифест инструментов

В архив ядра входят только исполняемый файл ядра из `bin/<target>/`
и `third-party/manifest.json`. Исполняемые файлы других платформ и внешние
движки в него не попадают.

Связанная проверка создаёт и открывает архив для `linux-x64`.
Она не проверяет состав архивов остальных платформ.
