---
id: INV.WIRE.ARTIFACT-BUILD-PUBLISHES-INSIDE-THE-WORKSPACE
status: active
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/infrastructure/daemon/server.rs::v5_artifact_build_prepares_before_source_admission_and_keeps_the_revision_gate
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::arguments_are_closed_and_each_refusal_names_the_fix
  - crates/unica-coder/src/infrastructure/daemon/v13_artifact_build.rs::apply_repeats_the_preview_and_returns_an_independent_file_receipt
scope: [wire, product]
---

# Сборка артефакта пишет только внутрь рабочего пространства и сама снимает квитанцию

`make` принимает `output` только как относительный путь `.cf`
или `.cfe` внутри рабочего пространства; `.epf` и `.erf` не публикуются.
Превью ничего не публикует и называет тот же вид артефакта, выход и набор,
что запрошены; применение принимается по `ifRev` этого превью и признаётся
по размеру и дайджесту собранного файла, снятым Unica.
