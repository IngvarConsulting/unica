---
id: INV.PKG.RUNTIME-ARCHIVE-REPRODUCIBLE
check:
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_archive_is_deterministic_and_target_only
---

# Один и тот же комплект файлов даёт одинаковый архив

Если дважды упаковать один и тот же готовый комплект файлов для запуска
Unica, архив ядра и JSON-файлы с описанием поставки должны совпасть байт в байт.
Время изменения каждого элемента tar-архива записывается как `0`.

Связанный тест проверяет повторную упаковку комплекта для `linux-x64`.
Другие платформы этой проверкой не охвачены.
