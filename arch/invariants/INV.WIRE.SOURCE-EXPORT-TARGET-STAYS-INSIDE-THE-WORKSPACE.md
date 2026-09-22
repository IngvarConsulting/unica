---
id: INV.WIRE.SOURCE-EXPORT-TARGET-STAYS-INSIDE-THE-WORKSPACE
status: superseded
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_source_export_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::preview_refuses_a_target_outside_the_workspace_an_undeclared_set_or_a_write
  - crates/unica-coder/src/infrastructure/daemon/v13_source_export.rs::apply_repeats_the_preview_and_counts_the_exported_files_itself
scope: [wire, product]
---

# Выгрузка в исходники не выходит за рабочее пространство, а квитанцию считает Unica

Превью `source.export` называет набор из объявленного состава и цель
относительным путём внутри рабочего пространства; цель снаружи, набор вне
проектного файла или уже выполненная запись отклоняются как негодный
результат провайдера. Применение принимается только по `ifRev` этого
превью и признаётся по пересчёту файлов в цели, не по отчёту раннера.
