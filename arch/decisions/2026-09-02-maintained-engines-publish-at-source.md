---
id: DEC.2026-09-02.MAINTAINED-ENGINES-PUBLISH-AT-SOURCE
status: active
governs: product
realized: tests/ci/test_product_contracts.py::test_both_sides_of_the_wire_approve_the_same_release_origins
supersedes: [DEC.2026-08-20.ENGINES-COME-FROM-THE-TOOLCHAIN]
superseded-by: null
establishes: [INV.PKG.ENGINE-RELEASE-SOURCES]
---

# Сопровождаемый движок публикуется из своего репозитория

**Решение.** Движок, который Ingvar Consulting сопровождает и выпускает сам,
приезжает из immutable-релиза собственного защищённого репозитория. Внешние
движки по-прежнему приезжают из `unica-toolchain`; старые выпуски
`v8-runner` там остаются историческими артефактами и источником отката, но новые
версии тулчейн не пересобирает.

Первый сопровождаемый движок — `v8-runner`. Его запись в `tools.lock.json`
связывает форк, source tag, commit, release tag, имена нативных ассетов и их
SHA256. Bootstrap разрешает этот источник только артефакту `v8-runner`, а не
любому движку из организации. Новый сопровождаемый движок требует явного
расширения закрытого перечня и проверки своего release-контракта.

Целостность поставки по-прежнему определяется lock-файлом и SHA256. Доверие к
происхождению дополняется защищённой веткой, неизменяемыми тегом и релизом,
нативным аудитом и build-attestation в репозитории сопровождаемого движка.
