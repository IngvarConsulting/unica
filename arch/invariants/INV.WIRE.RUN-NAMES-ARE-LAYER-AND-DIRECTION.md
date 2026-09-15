---
id: INV.WIRE.RUN-NAMES-ARE-LAYER-AND-DIRECTION
status: active
governs: product
decision: DEC.2026-09-15.RUN-NAMES-READ-AS-LAYER-AND-DIRECTION
check:
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_names_read_as_layer_and_direction
  - crates/unica-coder/src/application/v13/tool_catalog.rs::v13_run_dictionary_has_twelve_directional_runtime_intents
scope: [wire, product]
---

# Имя операции `run` — слой и направление, и ничего больше

Каждое имя словаря `unica.run` имеет вид `<слой>.<глагол>` из закрытых
списков: слои `infobase`, `cf`, `source`, `artifact`, `client`; глаголы
`create`, `export`, `import`, `build`, `run`. `export` всегда значит «из базы
наружу», `import` — «снаружи в базу», и у каждого `export` на слое есть парный
`import`. Глаголы `dump`, `restore`, `load`, `convert` в словаре не
появляются: они называли бы направление, у которого уже есть слово.
