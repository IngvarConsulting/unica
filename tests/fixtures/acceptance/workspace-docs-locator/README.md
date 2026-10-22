Synthetic platform help for issue #1537. All page text is authored for this fixture; no vendor materials are included.

`platform/8.3.27.2074/shcntx_ru.hbk` contains a single `FileStorage` entry in a 1C container, holding a ZIP of the HTML pages in `pages.json`. ZIP timestamps are fixed at 2026-01-01. The container uses the layout of `crates/unica-documentation-sources/src/platform_help/container.rs::tests_support::container_with`. The relative platform pin isolates the scenario from installed platforms.

`unica.toml` disables network providers so searches use only the fixture.
