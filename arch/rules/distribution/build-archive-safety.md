---
id: INV.PKG.BUILD-ARCHIVE-SAFETY
check:
  - tests/ci/test_build_unica_tools.py::BuildUnicaToolsTests.test_verified_archive_rejects_unsafe_and_drifted_members
  - tests/ci/test_build_unica_tools.py::BuildUnicaToolsTests.test_upstream_archive_rejects_unsafe_members_before_materialization
---

# Сборщик принимает только архив с проверенными файлами

Перед созданием готового набора инструментов сборщик проверяет runtime-архив
`tar.gz`: описание в `manifest.json` и файлы в `payload/`. Разрешены только
обычные файлы с безопасными относительными путями. Ссылки, повторы
и непереносимые имена отклоняются.

Состав файлов должен точно совпадать с манифестом. Для каждого файла сверяются
размер, SHA-256 и признак исполняемого файла. Любое несоответствие останавливает
обработку архива.

Архив издателя без `manifest.json` (`tar.gz` или `zip`) проходит те же
проверки состава: только обычные файлы с безопасными относительными путями,
без ссылок, шифрования и повторов. Сумма и размер сверяются с замком до
чтения, а объявленный в замке бинарь обязан быть в архиве.
