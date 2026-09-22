---
id: INV.PKG.RUNTIME-TOOL-CLOSURE
check:
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_packager_rejects_missing_extra_and_metadata_drift
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_packager_rejects_mode_drift
  - tests/ci/test_package_unica_runtime.py::PackageUnicaRuntimeTests.test_runtime_packager_rejects_duplicate_and_out_of_closure_tool_paths
---

# Упаковщик сверяет файлы сборки с их описанием

Упаковщик принимает локальный набор `bin/<target>`, только если он совпадает
с описанием в `tools.json`: нет пропущенных и лишних файлов, повторяющихся
путей и локальных путей за пределами `bin/<target>`. Размер и SHA-256 каждого
файла совпадают с описанием; на Linux и macOS совпадает и признак исполняемости.

Путь запуска каждого инструмента входит в этот набор и не повторяет путь
другого инструмента. Нарушение любого из этих условий останавливает упаковку.

Сверка с диском относится к записям с локальным `path`. Файлы внешнего архива,
описанные только через `deliveredPath`, здесь повторно не проверяются.
