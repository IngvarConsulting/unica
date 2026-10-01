---
id: INV.PKG.SHARED-BUILD-ARCHIVE-DOWNLOAD
check:
  - tests/ci/test_build_unica_tools.py::BuildUnicaToolsTests.test_bundle_builder_downloads_shared_archive_once_and_declares_runtime_closure
---

# Сборщик загружает общий архив один раз

Если несколько инструментов используют один архив, сборщик загружает его
один раз за сборку для выбранной платформы. Например, `rlm-bsl-index`
и `rlm-bsl-mcp` получают программы и общую библиотеку из одной загрузки.
