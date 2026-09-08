---
id: INV.WIRE.ROOT-FACTS-AND-VERDICT
status: active
governs: product
decision: DEC.2026-09-08.ROOT-VERDICT-IN-CHECK
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_without_at_bootstraps_an_empty_workspace
  - crates/unica-coder/src/infrastructure/daemon/server.rs::canonical_view_bootstrap_does_not_equate_git_presence_with_repository_readiness
scope: [wire, product]
---

# Корень не смешивает факты с вердиктом

Ответ `unica.view {}` не несёт ни `ready`, ни `discoveredReady`, ни
`repositoryReady`, ни `readinessState`, ни `checks`, ни `diagnostics`: это
вердикт, и живёт он в `unica.check {}`. Ответ `unica.check {}` не несёт перечня наборов:
это факт, и живёт он в `unica.view {}`.

Оба отвечают до допуска наборов. Вердикт обязан быть достижим на
ненастроенном пространстве — там он единственный, кто может сказать, чего не
хватает, — поэтому `unica.view {}` указывает на него в `next` всегда, а не только
когда всё готово.
