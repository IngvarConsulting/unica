Synthetic platform help for issue #2864. All page text is authored for this fixture; no vendor materials are included. The markup shape — `V8SH_*` classes, the BOM, the trailing space of a chapter title, the unclosed `<div>` of a parameter rubric, `&nbsp;` and `<font>` inside the example — is the shape measured on an installed 8.5.4.1878 help, because that shape is what the parser answers for.

`platform/8.3.27.2074/shcntx_ru.hbk` contains a single `FileStorage` entry in a 1C container, holding a ZIP of the HTML pages in `pages.json`. ZIP timestamps are fixed at 2026-01-01. The container uses the layout of `crates/unica-documentation-sources/src/platform_help/container.rs::tests_support::container_with`. The relative platform pin isolates the scenario from installed platforms.

The method page carries every typed section; the property page carries a «Параметры» chapter with no parameter rubric, so the scenario can tell an unparsed section from an empty one.

`unica.toml` disables network providers so searches use only the fixture.
