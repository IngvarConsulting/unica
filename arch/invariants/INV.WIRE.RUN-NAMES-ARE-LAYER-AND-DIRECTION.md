---
id: INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION
status: superseded
governs: product
decision: DEC.2026-09-22.RUNNER-ONE-TARGET-VOCABULARY
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_names_read_as_layer_and_direction
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
scope: [wire, product]
---

# Имя операции `run` — слой и направление, и ничего больше

Каждое имя словаря `unica.run` имеет вид `<слой>.<глагол>` из закрытых
списков: слои `infobase`, `cf`, `source`, `artifact`, `client`, `extension`; глаголы
`create`, `export`, `import`, `build`, `run`, `list`, `info`, `delete`, `activate`. `export` всегда значит «из базы
наружу», `import` — «снаружи в базу», и у каждого `export` на слое есть парный
`import`. Глаголы `dump`, `restore`, `load`, `convert` в словаре не
появляются: они называли бы направление, у которого уже есть слово.
