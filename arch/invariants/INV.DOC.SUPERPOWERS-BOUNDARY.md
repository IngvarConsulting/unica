---
id: INV.DOC.SUPERPOWERS-BOUNDARY
status: active
governs: process
decision: DEC.2026-08-18.REGISTRY-SHAPE
check: tests/arch/test_registry.py::test_superpowers_shapes_never_enter_arch
scope: [docs]
---

# Формы superpowers не входят в реестр
Формы документов, которые навязывают скиллы superpowers, в `arch/` не
появляются: у реестра свой формат. Скиллы пишут в `docs/plans/` и
`docs/design/`, откуда ссылаются на символы реестра.
