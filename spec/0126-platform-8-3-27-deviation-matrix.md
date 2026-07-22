# Issue 126: platform 8.3.27 deviation matrix

## Authority and scope

The official source is
`docs-local/1ci/8.3.27/en/developer/Chapter_2._Managing_configurations/2.17._Dumping_configurations_to_files_Restoring_configurations_from_files/2.17.2._Export_format_versions/index.md`.
Its export table maps platform `8.3.27` to format `2.20`, treats a missing root
version as `1.0`, and permits 8.3.27 to import formats less than or equal to
`2.20`.

The runtime XDTO export from issue #126 is exact-build evidence for
`8.3.27.2074` (archive SHA-256
`e7539a02520cf7bd73585d80b038c2c95078aac281d3700842a5f3a1f3c0c204`).
It is not a universal replacement for EDT schemas or platform roundtrips.

Status vocabulary:

- **deviation**: current code contradicts confirmed 8.3.27 evidence;
- **guard gap**: writer can modify an older/newer dump without a common preflight;
- **aligned**: current family output already follows confirmed evidence;
- **experiment**: code and prose conflict; capture an 8.3.27.2074 roundtrip before editing.

## Tool matrix

| Tool | Operation/family | Current behavior | 8.3.27 contract | Status and action | Evidence / regression |
| --- | --- | --- | --- | --- | --- |
| `unica.cf.edit` | `cf-edit`, Configuration/HomePage | HomePage hard-codes `2.17`; other edits preserve input | existing dump must be `2.20`; emitted HomePage uses `2.20` | deviation + guard gap; central preflight and active profile | official table; `cf_edit_home_page_uses_active_format` |
| `unica.cf.info` | `cf-info` | accepts `2.17`, `2.20`, `2.21` | diagnose relative to `2.20`, continue read-only | deviation; replace allowlist with compatibility diagnostic | `cf_info_reports_format_compatibility` |
| `unica.cf.init` | `cf-init` | fixtures/default output use `2.17` | new dump is `2.20` | deviation; active profile | `cf_init_uses_active_format` |
| `unica.cf.validate` | `cf-validate` | accepts `2.17`, `2.20`, `2.21` | read-only validation plus compatibility diagnostic | deviation | `cf_validate_reports_format_compatibility` |
| `unica.support.edit` | support sidecars | no export-format preflight | containing dump must be `2.20` before mutation | guard gap | `support_edit_blocks_older_dump` |
| `unica.cfe.borrow` | extension metadata/forms | inherits/falls back to `2.17` | source and destination roots must be `2.20` | deviation + guard gap | `cfe_borrow_blocks_mismatched_roots` |
| `unica.cfe.diff` | extension inspection | accepts several versions | diagnose both roots relative to `2.20` | guard gap; read-only warning | `cfe_diff_reports_both_formats` |
| `unica.cfe.init` | extension scaffold | defaults to `2.17` without base | new extension is `2.20`; base must be `2.20` when supplied | deviation | `cfe_init_uses_active_format` |
| `unica.epf.init` | external processor scaffold | constant `FORMAT_VERSION = "2.17"` | new Designer XML is `2.20` | deviation | `epf_init_uses_active_format` |
| `unica.erf.init` | external report scaffold | constant `FORMAT_VERSION = "2.17"` | new Designer XML is `2.20` | deviation | `erf_init_uses_active_format` |
| `unica.cfe.patch_method` | BSL interceptor plus extension context | BSL write lacks dump-format preflight | destination extension root must be `2.20` | guard gap | `cfe_patch_blocks_older_extension` |
| `unica.cfe.validate` | extension validator | accepts `2.17`, `2.20`, `2.21` | diagnose relative to `2.20` | deviation | `cfe_validate_reports_format_compatibility` |
| `unica.meta.compile` | metadata descriptors | uses detected version with `2.17` fallback | existing dump is exactly `2.20`; new standalone output uses `2.20` | deviation + guard gap | `meta_compile_uses_active_format` |
| `unica.meta.edit` | metadata descriptor | preserves input without central root check | containing dump must be `2.20` | guard gap | `meta_edit_blocks_older_dump` |
| `unica.meta.info` | metadata inspection | family validator accepts `2.17`/`2.20` | continue read-only with compatibility warning | deviation | `meta_info_reports_format_compatibility` |
| `unica.meta.profile` | indexed read-only metadata | no filesystem mutation | no writer constraint | aligned; no format guard | existing application tests |
| `unica.meta.remove` | metadata tree mutation | no common format preflight | containing dump must be `2.20` | guard gap | `meta_remove_blocks_older_dump` |
| `unica.meta.validate` | metadata validator | accepts `2.17`/`2.20` | continue read-only with compatibility warning | deviation | `meta_validate_reports_format_compatibility` |
| `unica.help.add` | Help descriptor/content | inherits detector's `2.17` fallback | containing/new descriptor uses `2.20` | deviation + guard gap | `help_add_uses_active_format` |
| `unica.form.add` | form descriptor/Form.xml | inherits detector fallback | containing dump and emitted roots use `2.20` | deviation + guard gap | `form_add_uses_active_format` |
| `unica.form.compile` | Form.xml | accepts caller-detected version; validator allows `2.17`/`2.20` | emitted Form root is `2.20` | deviation + guard gap | `form_compile_uses_active_format` |
| `unica.form.edit` | Form.xml | preserves current root | form/containing dump must be `2.20` | guard gap | `form_edit_blocks_older_form` |
| `unica.form.info` | form inspection | accepts older root | continue with compatibility warning | guard gap | `form_info_reports_format_compatibility` |
| `unica.form.remove` | form tree mutation | no common format preflight | containing dump must be `2.20` | guard gap | `form_remove_blocks_older_dump` |
| `unica.form.validate` | form validator | explicitly accepts `2.17`/`2.20` | `2.20` supported; others diagnosed | deviation | `form_validate_reports_format_compatibility` |
| `unica.interface.edit` | CommandInterface | test emitter contains `2.17` | containing dump and emitted root use `2.20` | deviation + guard gap | `interface_edit_uses_active_format` |
| `unica.interface.validate` | CommandInterface validator | no shared compatibility diagnostic | continue read-only with warning | guard gap | `interface_validate_reports_format_compatibility` |
| `unica.subsystem.compile` | subsystem descriptor | detector falls back to `2.17` | output uses `2.20` | deviation + guard gap | `subsystem_compile_uses_active_format` |
| `unica.subsystem.edit` | subsystem descriptor | model defaults missing version to `2.17` | missing root means old `1.0`; do not rewrite | deviation + guard gap | `subsystem_edit_blocks_missing_version` |
| `unica.subsystem.info` | subsystem inspection | no central classification | continue read-only with warning | guard gap | `subsystem_info_reports_format_compatibility` |
| `unica.subsystem.validate` | subsystem validator | no central classification | continue read-only with warning | guard gap | `subsystem_validate_reports_format_compatibility` |
| `unica.template.add` | template descriptor/content | descriptor inherits fallback; MXL emits root `SpreadsheetDocument` in `http://v8.1c.ru/spreadsheet/document` | descriptor `2.20`; MXL runtime namespace is `http://v8.1c.ru/8.2/data/spreadsheet`, root `document` | confirmed deviation; capture minimal platform fixture then reuse MXL contract | runtime XSD `0023.xsd`; `template_add_spreadsheet_matches_mxl_contract` |
| `unica.template.remove` | template tree mutation | no common preflight | containing dump must be `2.20` | guard gap | `template_remove_blocks_older_dump` |
| `unica.dcs.compile` | SKD content | content has no export-version attribute | containing descriptor/dump must be `2.20`; preserve SKD namespace | guard gap; XSD advisory | `dcs_compile_blocks_older_dump` |
| `unica.dcs.edit` | SKD content | content edit has no common preflight | containing dump must be `2.20` | guard gap | `dcs_edit_blocks_older_dump` |
| `unica.dcs.info` | SKD inspection | read-only | continue; warn from containing dump when resolvable | guard gap | `dcs_info_reports_dump_format` |
| `unica.dcs.validate` | SKD validator | semantic-first | retain semantic validation; add containing-format warning | aligned schema policy | issue #126 SKD incompatibility; `dcs_validate_reports_dump_format` |
| `unica.mxl.compile` | MXL `document` | uses runtime namespace/root; no export version | content aligned; containing dump must be `2.20` | aligned family + guard gap | XSD `0023.xsd`; `mxl_compile_blocks_older_dump` |
| `unica.mxl.decompile` | MXL read | read-only | continue; warn from containing dump | guard gap | `mxl_decompile_reports_dump_format` |
| `unica.mxl.info` | MXL inspection | read-only | continue; warn from containing dump | guard gap | `mxl_info_reports_dump_format` |
| `unica.mxl.validate` | MXL validator | semantic-first | retain semantic validation; warn from containing dump | aligned schema policy | issue #126 MXL incompatibility; `mxl_validate_reports_dump_format` |
| `unica.role.compile` | Rights.xml + role descriptor | descriptor uses detector; Rights carries caller version | emitted versioned roots use `2.20` | deviation + guard gap | `role_compile_uses_active_format` |
| `unica.role.info` | role inspection | read-only | continue; warn relative to `2.20` | guard gap | `role_info_reports_format_compatibility` |
| `unica.role.validate` | role validator | type-library semantics | retain semantics; add format warning | guard gap; XSD advisory | issue #126 type-only roles schema |

## Cross-family deviations to resolve

| Family | Conflict | Required evidence | Rule |
| --- | --- | --- | --- |
| Export version | shared detector, external scaffolds, HomePage and several tests use `2.17`; CF/CFE validators also admit `2.21` | official 8.3.27 export table | one active profile, no local allowlists |
| MXL created by `template.add` | current namespace/root differs from `unica.mxl.compile` and runtime XSD | XSD `0023.xsd` plus `tests/fixtures/platform_8_3_27/mxl/Template.xml` | change after roundtrip fixture is captured |
| ExchangePlanContent | Rust emits `xcf/extrnprops`; prose claims `MDClasses` | EDT XSD plus `tests/fixtures/platform_8_3_27/exchange_plan/Content.xml` from 8.3.27.2074 | correct the contradicted side only after capture |
| Raw XSD strictness | SKD, MXL, client interface, and roles have known raw-schema incompatibilities | issue #126 corpus results and platform reload | XSD remains advisory until family profile is proven |

## Migration boundary

Older roots are not rewritten by ordinary tools. They receive
`formatMigrationAvailable` and an explicit recommendation for
`unica.cf.migrate_format` or `unica.cfe.migrate_format`. Newer roots receive
`platformVersionUnsupported`, the agreed 8.5 roadmap message, and no downgrade
offer. The migration child process must use a verified `8.3.27.*` installation
through a scoped `V8_PATH`; ambient 8.5 installations are not accepted.
