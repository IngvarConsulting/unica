- Date: `2026-08-31`
- Status: `approved`
- Decision: `DEC.2026-08-31.V0-13-SURFACE-FIRST-CUTOVER`

# v0.13 first-surface cutover: complete v0.12.3 migration design

## Scope and decision boundary

This is the first-cutover design for exactly the 74 names published by
`tests/fixtures/migration/v0.12.3-baseline.json`.  It maps each legacy entry to
one of the eight canonical v0.13 calls -- `unica.view`, `unica.apply`,
`unica.find`, `unica.search`, `unica.check`, `unica.diff`, `unica.run`, and
`unica.docs` -- or to the three compatibility Task calls (`unica.task.get`,
`unica.task.result`, `unica.task.cancel`) when the client has no native Tasks
transport.  Native Tasks uses `tasks/get` and `tasks/cancel`; it has no
`tasks/result` operation.

This document is intentionally a migration map, not an alias layer: its target
is one atomic breaking cutover without legacy names.  `plugins/unica/skills/**`
is excluded from scope by the user's decision.  It therefore changes neither
skills nor skill routing.

The user-owned follow-up gate is manual review of the migration impact on
`plugins/unica/skills/**`; those files remain outside this document's scope.
The corresponding decision and public-surface contract/invariant records will
be added with the broader cutover work, not substituted by this design note.
**Merging to `main` is not a release**; package/release evidence remains a
separate gate.

## Reading the matrix

`?` means an optional legacy field; no default is implied.  The legacy fields
shown are the semantically consumed published parameters; broad native XML/DSL
compatibility fields are abbreviated as `nativeArgs?` where they are not
operation-specific.  Known explicit defaults are retained: legacy source-tree
mutation `dryRun=false`, `runtime.execute` preview `dryRun=true` when omitted,
`code.outline.includeMethods=true`, `runtime.job.wait.timeoutSeconds=1..60`,
and `runtime.job.logs.tailChars=1..32768`.  `cwd?` and `confirm?` are legacy
transport/context fields, never v0.13 logical addresses.

`A(S,p)` below is a required, not-yet-implemented conversion of legacy
`sourceSet`/physical path into a qualified logical address such as
`S:Catalog.Product.Form.Item`; it is not a field accepted by a v0.13 tool.
`Cfg(S)` means `S:Configuration`.  `R(...)` marks the selected entry in
`unica.run.args`; every displayed `filter`/`args` member is a planned typed
union, not permission to pass arbitrary old payloads through the shallow object
schema.  `implemented` means executable and publicly selected, not merely
catalogued or hidden.

Disposition: **mapped** has a faithful target shape; **absorbed** is a narrower
projection in a canonical read/search tool; **transport-replaced** moves durable
job protocol to Tasks; **removed** has no successor; **deferred** needs a new
typed contract, resolver, or explicitly accepted loss.  A row may name more
than one disposition where legacy variants differ.

## Complete 74-row migration matrix

| Legacy v0.12.3 tool | Exact legacy selector, important parameters, and defaults | v0.13 call and parameter transformation | Disposition; implementation now; loss/deferred boundary |
| --- | --- | --- | --- |
| `unica.build.dump` | generic build args `{config?,database?,dbUser?,dbPassword?,format?,infobase?,mode?,password?,path?,sourceDir?,sourceSet?,target?,user?,cwd?,confirm?,dryRun?}` | first resolve/admit `sourceSet` or legacy `sourceDir` as source identity `S`, then `unica.run({"op":"source.dump","args":{"sourceSet":S,"mode?":mode,"format?":format}})`; credentials go to protected runtime config. | mapped; the public run dictionary names this operation, but execution is typed unsupported. `sourceDir`/path must not become public args and dump result/revision is unmodelled. |
| `unica.build.load` | same generic build args | first resolve/admit the legacy source input as `S`, then `unica.run({"op":"infobase.build","args":{"sourceSet":S,"target?":target}})`. | mapped; the public run dictionary names this operation, but execution is typed unsupported. This is build-from-source, not `artifact.load`; credentials and target identity need typed runtime configuration. |
| `unica.build.make` | generic build args plus `output` required for execution and `extension?` | first resolve/admit the legacy source input as `S`, then `unica.run({"op":"artifact.make","args":{"output":output,"extension?":extension,"sourceSet":S,"format?":format}})`. | mapped; the public run dictionary names this operation, but execution is typed unsupported. Output destination/CFE export must be an artifact contract, not an unchecked path. |
| `unica.build.run` | generic build args; client-oriented `mode?`, `target?` may be selected | nominal `unica.run({"op":"client.run","args":{"mode?":mode,"target?":target}})`. | deferred; the public run dictionary names this operation, but execution is typed unsupported. V13 rejects a client session that outlives the response; raw launch/session fields have no faithful successor. |
| `unica.build.update` | same generic build args | first resolve/admit the legacy source input as `S`, then `unica.run({"op":"infobase.build","args":{"sourceSet":S,"target?":target,"update":true}})`. | mapped; the public run dictionary names this operation, but execution is typed unsupported. `update:true` must be a closed typed distinction; folding it into ordinary build changes semantics. |
| `unica.cf.edit` | `ConfigPath|configPath|Path|path`, `DefinitionFile|definitionFile?` or `Value|value?`, `Operation?`, `NoValidate?`, `dryRun=false`, `cwd?`, `confirm?` | resolve configuration, normalize each legacy change to one of `props.set`, `relation.add`, `relation.remove`, or `relation.replace`, then call `unica.apply({"at":Cfg(S),"ops":mappedOperations,"dryRun":dryRun})`. | mapped/partial; `props.set` now uses the shared dry-run/real retained publisher. `relation.*`, panels/home page and opaque generic DSL remain typed unsupported; never tunnel `Operation` unchanged. |
| `unica.cf.info` | exclusive `sourceSet` or `ConfigPath`; `cwd?`, `confirm?` | `unica.view({"at":Cfg(sourceSet),"filter":{"sections?":sections},"limit?":limit,"cursor?":cursor})`; path branch first uses `A`. | absorbed; public base view exists. Direct path and rich configuration report projections need resolver/filter contracts. |
| `unica.cf.init` | `OutputDir|outputDir`, `Name?`, `Synonym?`, `Vendor?`, `Version?`, `CompatibilityMode?`, `dryRun=false` | no legal existing `at`; proposed future `run({"op":"source.create","args":{...}})` creates/adopts a source set. | deferred; no public run implementation. Root bootstrap is not `apply`; output path and source-set admission need a typed transaction. |
| `unica.cf.validate` | exclusive `sourceSet` or `ConfigPath`, `nativeArgs?`, `cwd?`, `confirm?` | candidate `unica.check({"at":Cfg(sourceSet),"filter":{"validation":{"profile":"cf"}}})`; direct path first uses `A`. | deferred; public admission/readability check exists and the closed profile is parsed, but no real validator execution is proven. Physical-path resolution and canonical diagnostics remain missing. |
| `unica.cfe.borrow` | `ExtensionPath`, `ConfigPath`, `Object`, `BorrowMainAttribute|borrowMainAttribute?`, `dryRun=false` | proposed `unica.apply({"at":"Sext:<borrowed-node>","ops":[{"op":"extension.borrow","args":{"baseSourceSet":Sbase,"object":Object,"borrowMainAttribute?":...}}]})`. | deferred; no registry/planner. Cross-source input violates current one-source `apply` address rule; preserve adopted object/form checks. |
| `unica.cfe.diff` | `ConfigPath`, `ExtensionPath`, `cwd?`, `confirm?` | candidate `unica.diff({"left":A(Sbase,ConfigPath),"right":A(Sext,ExtensionPath)})`. | deferred; public structural same-kind diff exists, but these roots are not proven comparable and neither path has safe source binding. |
| `unica.cfe.init` | `ConfigPath?`, `OutputDir|ExtensionPath`, `Name?`, `Synonym?`, `Vendor?`, `Version?`, `CompatibilityMode?`, `Purpose?`, `NamePrefix?`, `NoRole?`, `dryRun=false` | proposed `unica.run({"op":"source.create","args":{"kind":"extension",...}})`, then source-set admission. | deferred; no run handler. Extension root creation and optional base configuration need one atomic bootstrap contract. |
| `unica.cfe.patch_method` | `ExtensionPath`, `ModulePath`, `MethodName|methodName`, `InterceptorType|interceptorType`, `Context?`, `IsFunction|isFunction=false`, `dryRun=false` | normalize `InterceptorType` to `timing="before"` or `timing="after"`, then call `unica.apply({"at":A(Sext,ModulePath),"ops":[{"op":"interceptor.add","args":{"method":MethodName,"timing":timing,"context?":Context}}],"dryRun":dryRun})`. | deferred; code/apply family stub. An interceptor is not ordinary `code.insert`; retain adopted-object and base-signature constraints. |
| `unica.cfe.validate` | `ExtensionPath`, `nativeArgs?`, `cwd?`, `confirm?` | candidate `unica.check({"at":A(Sext,ExtensionPath),"filter":{"validation":{"profile":"cfe"}}})`. | deferred; the closed profile is parsed, but no extension source-set/address route or real validator execution is proven. |
| `unica.code.definition` | `name` required; `moduleHint?`, `limit?`, `sourceDir?`, `cwd?`, `confirm?` | `unica.find({"query":name,"kind":"definition","limit?":limit})`; map `moduleHint` only after an address/module filter is designed. | absorbed; public base find exists. `sourceDir` and module ambiguity have no current canonical filter. |
| `unica.code.diagnostics` | `action`, `sourceSet`; `analyze:{filter?,limit?,timeoutSeconds?}`; `findings:{metadataPath required,filter?,range?,limit?}`; `status`; `catalog:{filter?,limit?}`; `cwd?` | candidate `unica.check({"at":A(sourceSet,metadataPath)?,"filter":{"diagnostics":{"action":action,"filter?":filter,"range?":range,"limit?":limit}}})`. | deferred; public admission/readability check exists, but diagnostic filters are typed unsupported. `analyze` launches/awaits providers, unlike persisted check; timeout/provider/filter union must be explicit. |
| `unica.code.graph` | `mode` required; `detail?`, `dir?`, `edgeKinds?`, `id?`, `ids?`, `limit?`, `maxOutputTokens?`, `provenance?`, `query?`, `sourceDir?`, `cwd?`, `confirm?` | candidate `unica.view({"at":A(S,node),"filter":{"graph":{"mode":mode,"detail?":detail,"dir?":dir,"edgeKinds?":edgeKinds,"id?":id,"ids?":ids,"provenance?":provenance,"query?":query}},"limit?":limit})`. | deferred; no graph view/filter/result projection. `sourceDir`, traversal identities and max-output-token semantics are not represented. |
| `unica.code.outline` | `path` required; `includeMethods=true`, `sourceDir?`, `cwd?`, `confirm?` | `unica.view({"at":A(S,path),"filter":{"outline":{"includeMethods":includeMethods}}})`. | absorbed; public base view exists. Resolver and outline projection are unimplemented. |
| `unica.code.patch` | `sourceSet`, `metadataPath`, `operation:"insert"|"replace"`, `content`; `selector?`, `position?`, `dryRun=false`, `cwd?`, `confirm?` | `unica.apply({"at":"sourceSet:metadataPath","ops":[{"op":"code.insert","args":{"text":content,"selector?":selector,"position?":position}}],"dryRun":dryRun})` or `code.replace` with `{text,selector}`. | mapped; registry name exists but hidden code planner rejects. Requires module resolver, byte-preserving writer and typed selector validation. |
| `unica.code.search` | `query`; exactly one `sourceSet|sourceDir`; `metadataPath?` only with sourceSet; `limit?`, `cwd?`, `confirm?` | `unica.search({"query":query,"scope":"sourceSet:metadataPath","limit?":limit})`; absent `metadataPath` becomes `"sourceSet:Configuration"`; sourceDir branch first uses `A`. | mapped/partial; public literal BSL search supports Configuration and resolved metadata-object subtrees. Physical path conversion, regex and symbol search remain missing. |
| `unica.dcs.compile` | `OutputPath|outputPath`, `Value?` or `DefinitionFile|definitionFile?`, `NoValidate?`, `dryRun=false` | candidate `unica.apply({"at":A(S,OutputPath),"ops":[{"op":"dcs.set","args":{"definition":normalizedDsl}}],"dryRun":dryRun})`. | deferred; Dcs/Mxl apply seam stub. Output path resolver, full definition schema, and snapshotted query-file values are required. |
| `unica.dcs.edit` | `TemplatePath|templatePath|Path|path`, `Operation`, `DataSet?`, `Variant?`, `NoSelection?`, `Value?`, `NoValidate?`, `dryRun=false` | normalize each legacy operation to its one closed DCS op name (`dcs.set`, the corresponding field/total/parameter/filter/query/selection/order/appearance/link/data-set/variant/drilldown/output/structure op), then call `unica.apply({"at":A(S,TemplatePath),"ops":mappedOperations,"dryRun":dryRun})`. | mapped; names closed but planner stub. Each old DSL variant requires its own schema; no opaque `Operation` passthrough. |
| `unica.dcs.info` | exclusive `sourceSet+metadataPath` or `TemplatePath`; `delivery?`, `filter?`, `page?`, `resultRef?`, `section?`, `cwd?`, `confirm?` | `unica.view({"at":"sourceSet:metadataPath","filter":{"sections?":section,"legacyDelivery?":delivery},"cursor?":page})`; path branch uses `A`. | absorbed; public base view exists. Legacy delivery/resultRef and page tokens do not map losslessly to cursor semantics. |
| `unica.dcs.validate` | exclusive `sourceSet+metadataPath` or `TemplatePath`, `nativeArgs?`, `cwd?`, `confirm?` | `unica.check({"at":A(sourceSet,metadataPath),"filter":{"validation":{"profile":"dcs"}}})`. | deferred; the profile belongs to the closed filter union, but its resolver and real validator execution are absent. |
| `unica.documentation.get` | `documentId`, `language?`, `platformVersion?`, `cwd?`, `confirm?` | no faithful single target; the only schema-valid search-shaped candidate is `unica.docs({"query":documentId})`. | deferred; public docs is search-shaped. Stable document identity/full-body retrieval, locale and platform-version pinning need a new contract. |
| `unica.documentation.search` | `query`, `sourceKinds?`, `language?`, `platformVersion?`, `limit?`, `cwd?`, `confirm?` | omitted `sourceKinds` → `unica.docs({"query":query})`, which currently searches only the safe platform-help and development-standard sources; `["platform-help"]` → `unica.docs({"query":query,"source":"platform-help"})`; `["development-standard"]` → `unica.docs({"query":query,"source":"development-standard"})`; `["configuration-documentation"]` maps to the same named source but currently returns typed `unsupported_source`. | mapped/deferred; `source` is one kind string, multiple legacy source kinds require separate calls, and configuration help is deferred until its reader uses the actor-owned nofollow/cancellation boundary. Language, platformVersion and limit have no v13 docs fields. |
| `unica.epf.init` | `Name`, `Synonym?`, `OutputDir`, `FormName?`, `dryRun=false` | proposed `unica.run({"op":"source.create","args":{"kind":"externalProcessor","name":Name,"synonym?":Synonym,"formName?":FormName}})`. | deferred; no run handler. Generated source set plus optional form must be one atomic bootstrap. |
| `unica.erf.init` | `Name`, `Synonym?`, `OutputDir`, `FormName?`, `dryRun=false` | proposed `unica.run({"op":"source.create","args":{"kind":"externalReport","name":Name,"synonym?":Synonym,"formName?":FormName}})`. | deferred; same atomic root-creation boundary. |
| `unica.form.add` | `ObjectPath|objectPath|Path|path`, `FormName|formName`, `Purpose?`, `SetDefault|setDefault?`, `dryRun=false` | `unica.apply({"at":A(S,ObjectPath),"ops":[{"op":"form.add","args":{"name":FormName,"purpose?":Purpose,"setDefault?":SetDefault}}],"dryRun":dryRun})`. | mapped; registry present, Form planner stub. Target is owning logical object/Forms collection, never `Form.xml`. |
| `unica.form.compile` | `OutputPath`, `JsonPath|jsonPath?` or `FromObject|fromObject?`, `ObjectPath?`, `Purpose?`, `FormName?`, `dryRun=false` | choose `form.create` for a missing admitted target and `form.set` for an existing target, then call `unica.apply({"at":A(S,OutputPath),"ops":[{"op":selectedOp,"args":{"definition":normalizedForm}}],"dryRun":dryRun})`. | deferred; Form planner stub. Compile can create/infer multiple files; preserve atomic compiler semantics rather than relabel as `form.set`. |
| `unica.form.edit` | `FormPath|formPath|Path|path`, `JsonPath|jsonPath?` or inline definition, `dryRun=false` | normalize every legacy change to one closed form op (`form.set`, `element.add`, `element.remove`, `formAttribute.add`, `formCommand.add`, or `event.bind`), then call `unica.apply({"at":A(S,FormPath),"ops":mappedOperations,"dryRun":dryRun})`. | mapped/deferred; planner stub. `formAttribute.set/remove`, `formCommand.set/remove`, whole-definition replacement require new typed ops. |
| `unica.form.info` | exclusive `sourceSet+metadataPath` or `FormPath`; `delivery?`, `filter?`, `page?`, `resultRef?`, `section?`, `cwd?`, `confirm?` | `unica.view({"at":A(sourceSet,metadataPath),"filter":{"sections?":section},"cursor?":page})`. | absorbed; public base view exists. Old delivery/resultRef/page behavior is not preserved. |
| `unica.form.remove` | `SrcDir|srcDir?`, `ObjectName`, `FormName`, `dryRun=false` | `unica.apply({"at":"S:<ObjectName>.Form.<FormName>","ops":[{"op":"form.remove","args":{}}],"dryRun":dryRun})` after resolving `SrcDir`. | mapped; registry present, planner stub. Needs resolver and reference/removal guard port. |
| `unica.form.validate` | exclusive `sourceSet+metadataPath` or `FormPath`, `nativeArgs?`, `cwd?`, `confirm?` | `unica.check({"at":A(sourceSet,metadataPath),"filter":{"validation":{"profile":"form"}}})`. | deferred; the profile belongs to the closed filter union, but its resolver, real validator execution and canonical diagnostics are missing. |
| `unica.help.add` | `SrcDir|srcDir?`, `ObjectName|objectName`, `Lang|lang?` or `Language|language?`, `dryRun=false` | normalize the accepted alias to `language`, then call `unica.apply({"at":"S:<ObjectName>","ops":[{"op":"help.create","args":{"language":language}}],"dryRun":dryRun})`. | mapped; baseline-only handler and registry seam, planner stub. Port Help facet; do not re-publish an old public tool only for migration. |
| `unica.interface.edit` | `CIPath|ciPath|Path|path`, `DefinitionFile|definitionFile?` or `Value|value?`, `CreateIfMissing?`, `NoValidate?`, `dryRun=false` | proposed `unica.apply({"at":"S:Subsystem.<SS>","ops":[{"op":"commandInterface.set","args":{"definition":...,"createIfMissing?":...}}],"dryRun":dryRun})`. | deferred; no stable descriptor. Command interface is a companion resource, not `command.add`. |
| `unica.interface.validate` | `CIPath`, `nativeArgs?`, `cwd?`, `confirm?` | candidate `unica.check({"at":A(S,CIPath),"filter":{"validation":{"profile":"interface"}}})`. | deferred; the profile is parsed, but CIPath has no source identity and no real interface validator execution is proven. |
| `unica.meta.add` | `sourceSet`, `kind`, `name`, `operations?`, `dryRun=false` | `unica.apply({"at":Cfg(sourceSet),"ops":[{"op":"object.create","args":{"values":{"kind":kind,"name":name}}},...typed(operations)],"dryRun":dryRun})`. | mapped; Metadata seam exists but hidden planner stub. `{kind,name}` schema, exact legacy union and atomic creation image remain required. |
| `unica.meta.edit` | `sourceSet`, `metadataPath`, `operations`, `dryRun=false` | `unica.apply({"at":"sourceSet:metadataPath","ops":typed(operations),"dryRun":dryRun})`; variants map to `props.set`, `relation.*`, `attribute.*`, `tabularSection.*`, `dimension.*`, `resource.*`, `enumValue.*`, `column.*`, `form.*`, `template.*`, `command.*`, `predefinedItem.*`. | mapped/partial; atomic retained publication and identical dry-run/real postimage/effect plan hashes are proven for `props.set` and `attribute.add/set/remove`. Relation and remaining collections stay exact typed unsupported. |
| `unica.meta.info` | `sourceSet`, `metadataPath`; `sections?`, `limit?`, `cwd?`, `confirm?` | `unica.view({"at":"sourceSet:metadataPath","filter":{"sections?":sections},"limit?":limit})`. | absorbed/partial; closed `props`, `branches`, `can`, `limits`, `items` selection is implemented. Legacy report-only section names remain deferred. |
| `unica.meta.remove` | `sourceSet`, `metadataPath`, `force?=false`, `confirm?=false`, `dryRun=false` | `unica.apply({"at":"sourceSet:metadataPath","ops":[{"op":"object.remove","args":{"force":force,"confirm":confirm}}],"dryRun":dryRun})`. | mapped; registry skeleton/planner stub. Destructive schema, usage scan and reference report must be ported. |
| `unica.mxl.compile` | `JsonPath`, `OutputPath`, `dryRun=false` | candidate `unica.apply({"at":A(S,OutputPath),"ops":[{"op":"mxl.set","args":{"definition":parsedDsl}}],"dryRun":dryRun})`. | deferred; Dcs/Mxl seam stub. Template resolver and exact spreadsheet writer/schema are missing. |
| `unica.mxl.decompile` | exclusive `sourceSet+metadataPath` or `TemplatePath`, `nativeArgs?`, `cwd?`, `confirm?` | no current call; future export/artifact operation required. | removed/deferred; it yields a JSON DSL/text artifact, neither admitted `view` projection nor `run` op. Do not misclassify as view. |
| `unica.mxl.info` | exclusive `sourceSet+metadataPath` or `TemplatePath`; `WithText|withText?`, `delivery?`, `filter?`, `page?`, `resultRef?`, `section?`, `cwd?`, `confirm?` | normalize the accepted alias to `withText`, resolve the selected target, then call `unica.view({"at":A(S,target),"filter":{"sections?":section,"withText?":withText},"cursor?":page})`. | absorbed; public base view exists. Legacy delivery/resultRef/page and full-text size behavior need a new projection/cursor contract. |
| `unica.mxl.validate` | exclusive `sourceSet+metadataPath` or `TemplatePath`, `nativeArgs?`, `cwd?`, `confirm?` | `unica.check({"at":A(sourceSet,metadataPath),"filter":{"validation":{"profile":"mxl"}}})`. | deferred; the profile belongs to the closed filter union, but its resolver, real validator execution and diagnostic mapping are absent. |
| `unica.project.map` | `confirm?`, `cwd?` | no lossless call; tentative `unica.view({"at":Cfg(defaultSourceSet),"filter":{"projectMap":true}})` only after workspace/source-set selection is defined. | deferred; public base view has no workspace inventory/map projection. |
| `unica.project.status` | `confirm?`, `cwd?` | `unica.view({"at":Cfg(defaultSourceSet),"filter":{"status":true}})` only when a default admitted source set exists. | absorbed/deferred; no default source-set/status filter or public service. |
| `unica.role.compile` | `JsonPath`, `OutputDir`, `dryRun=false` | proposed atomic `unica.apply({"at":Cfg(S),"ops":[{"op":"object.create","args":{"values":{"kind":"Role","name":...}}},{"op":"role.create","args":{"values":rightsDsl}}],"dryRun":dryRun})`. | deferred; role create seam/planner stub. Separate ops can publish Role without Rights.xml; compiler transaction must remain atomic. |
| `unica.role.edit` | `sourceSet`, `metadataPath`, `operations`, `dryRun=false` | `unica.apply({"at":"sourceSet:metadataPath","ops":[{"op":"right.set","args":typed(operations)}],"dryRun":dryRun})`. | mapped; `right.set` registry name but resource planner stub. Preserve closed rights/RLS union, no arbitrary map. |
| `unica.role.info` | exclusive `sourceSet+metadataPath` or `RightsPath`; `delivery?`, `filter?`, `page?`, `resultRef?`, `section?`, `cwd?`, `confirm?` | `unica.view({"at":A(sourceSet,metadataPath),"filter":{"sections?":section},"cursor?":page})`. | absorbed; public base view exists. Legacy report delivery/paging does not map to cursor without a projection contract. |
| `unica.role.validate` | exclusive `sourceSet+metadataPath` or `RightsPath`, `nativeArgs?`, `cwd?`, `confirm?` | `unica.check({"at":A(sourceSet,metadataPath),"filter":{"validation":{"profile":"role"}}})`. | deferred; the profile belongs to the closed filter union, but its resolver, real validator execution and diagnostics contract are absent. |
| `unica.runtime.execute` | `operation`; common `config`, `workdir`, `cwd?`, `confirm?`, `dryRun=true`; variants: `config-init(config,workdir,sourceSet?,connection?,format?,builder?,force?)`; `init`; `build(sourceSet?,fullRebuild?)`; `dump(mode=full|incremental|partial,object?/objects?,sourceSet?,extension?)`; `convert(sourceSet?,output?)`; `make(output required,sourceSet?,extension?)`; `load(path required,mode=load|merge,settings? merge-only,extension?)`; `syntax(mode=designer-config|designer-modules|edt,...flags)`; `test(testRunner=yaxunit|va,...runner fields)`; `launch(clientMode/direct fields or mcpConfig,mcpPort)`; `extensions(sourceSet?|sourceSets?)`; `tools-download(tool=yaxunit|vanessa|client-mcp,sources?,force?)`. | map operation to `unica.run({"op":R(operation),"args":typed(selected fields)})`: `config-init→source.create` or external attach→`source.attach`; `init→infobase.create`; `build→infobase.build`; `dump→source.dump`; `convert→source.convert`; `make→artifact.make`; `load→artifact.load`; `syntax→syntax.check`; `test→test.run`; `launch→client.run`; `extensions→extension.sync`; `tools-download→none`. | mapped/partial/removed; `syntax.check` is supported as bounded durable Task with closed args, five-minute timeout, bounded capture and sanitized terminal/provider result. The other eleven dictionary operations remain typed unsupported; `query.execute` is absent from v0.13 and `tools-download` remains removed. |
| `unica.runtime.job.cancel` | `jobId`, `cwd?`, `confirm?` | native `tasks/cancel(taskId)`; non-Tasks `unica.task.cancel({"taskId":jobId})`. | transport-replaced; compatibility task call is now public. A legacy jobId is not a Task ID; cancellation is cooperative/idempotent. |
| `unica.runtime.job.list` | `cwd?`, `confirm?` | no call. | removed; V13 deliberately has no task enumeration; task IDs are opaque per invocation. |
| `unica.runtime.job.logs` | `jobId`, `tailChars=1..32768?`, `cwd?`, `confirm?` | no call. | removed; V13 has no raw stdout/stderr tail/logs API; bounded diagnostics/terminal outcome belong in Task state. |
| `unica.runtime.job.start` | `operation` plus the selected `runtime.execute` variant except `waitForExit`, `waitTimeoutMs`, `stderrOutput`; `cwd?`, `confirm?`, `dryRun?` | invoke the corresponding `unica.run({"op":R(operation),"args":typed(...)})`; server returns direct result or Task handle. | transport-replaced/partial; `syntax.check` returns a durable Task, while other run handlers are absent. There is no caller-selected async job record; download/client-session variants stay removed/deferred. |
| `unica.runtime.job.status` | `jobId`, `cwd?`, `confirm?` | native `tasks/get(taskId)`; non-Tasks `unica.task.get({"taskId":jobId})`. | transport-replaced; native and compatibility Task projections are public. Legacy job ID bridge is deliberately absent. |
| `unica.runtime.job.wait` | `jobId`, `timeoutSeconds=1..60?`, `cwd?`, `confirm?` | native `tasks/get(taskId)` polling/updates; non-Tasks `unica.task.result({"taskId":jobId,"waitMs":min(timeoutSeconds*1000,7000)})`. | transport-replaced; native and compatibility Task projections are public. Seven-second cap and polling change old blocking semantics; never re-run invocation. |
| `unica.source.children` | `sourceSet`; `metadataPath?`, `limit?`, `cursor?`, `cwd?`, `confirm?` | with `metadataPath`, resolve it to `A(sourceSet,metadataPath)` and call `unica.view({"at":A(sourceSet,metadataPath),"filter":{"children":true},"limit?":limit,"cursor?":cursor})`; without it call the same shape with `"at":"sourceSet:Configuration"`. | absorbed; public base view exists. Children projection/cursor token implementation incomplete. |
| `unica.source.locate` | `sourceSet`, `path`, `cwd?`, `confirm?` | `unica.find({"query":path,"kind":"address"})` only after a path→logical address query contract exists. | deferred; public find is not a physical locator. Do not leak/accept filesystem paths in v0.13. |
| `unica.source.read` | `snapshotId`, `resourceId`; `offset?`, `limit?`, `cwd?`, `confirm?` | no faithful call; candidate future `unica.view({"at":"<resource address>","filter":{"content":{"offset?":offset,"limit?":limit}}})`. | deferred; snapshots/resource IDs and byte paging have no qualified-address/revision/content contract. |
| `unica.source.resolve` | `sourceSet`, `query`; `mode?`, `targetKind?`, `limit?`, `cursor?`, `cwd?`, `confirm?` | `unica.find({"query":query,"kind?":targetKind,"limit?":limit})`; resolve mode/paging need a typed result contract. | mapped/absorbed; public base find exists. `sourceSet` scope, `mode`, cursor and ambiguous-resolution semantics missing. |
| `unica.source.resources` | branch A: `sourceSet`, `metadataPath?`, `scope?`, `limit?`; branch B: `snapshotId`, `cursor`, `limit?`; `cwd?`, `confirm?` | branch A with `metadataPath` uses `unica.view({"at":A(sourceSet,metadataPath),"filter":{"resources":{"scope?":scope}},"limit?":limit})`; without it the call uses `"at":"sourceSet:Configuration"`. Branch B has no target. | absorbed/deferred; public base view exists. Snapshot enumeration/cursor cannot be silently converted to logical-address pagination. |
| `unica.standards.explain` | `codes?`, `id?`, `idOrAliasOrUrl?`, `language?`, `limit?`, `mode?`, `snippet?`, `types?`, `bodyLimit?` or `body_limit?`, `cwd?`, `confirm?`; precedence: codes diagnostic, idOrAliasOrUrl overrides id | choose `query` before the call: `idOrAliasOrUrl` when present, otherwise `id`, otherwise the normalized `codes`; then call `unica.docs({"query":query,"source":"development-standard"})`. | mapped/deferred; public docs search supports this source kind. Locale, types, mode, snippet, codes expansion and body limits need a new typed contract. |
| `unica.standards.search` | `query`, plus `codes?`, `id?`, `idOrAliasOrUrl?`, `language?`, `limit?`, `mode?`, `snippet?`, `types?`, `bodyLimit?` or `body_limit?`, `cwd?`, `confirm?` | `unica.docs({"query":query,"source":"development-standard"})`. | mapped/deferred; public docs search supports the core query. Legacy standards filters have no v0.13 docs fields and are explicitly deferred/lost until a typed contract exists. |
| `unica.subsystem.compile` | `OutputDir`, `Parent?`, `DefinitionFile|definitionFile?` or `Value|value?`, `NoValidate?`, `dryRun=false` | choose `at="S:Configuration"` when `Parent` is absent and `at="S:Subsystem.<Parent>"` when present, then call `unica.apply({"at":at,"ops":[{"op":"subsystem.create","args":{"definition":normalizedDsl}}],"dryRun":dryRun})`. | deferred; registry name but resource planner stub. Preserve compile parent registration atomically. |
| `unica.subsystem.edit` | `SubsystemPath|subsystemPath|Path|path`, `DefinitionFile|definitionFile?` or `Value|value?`, `NoValidate?`, `dryRun=false` | normalize each legacy change to one of `props.set`, `content.add`, `content.remove`, `childSubsystem.add`, or `childSubsystem.remove`, then call `unica.apply({"at":A(S,SubsystemPath),"ops":mappedOperations,"dryRun":dryRun})`. | mapped; registry names/planner stub. Whole-definition variants need typed expansion. |
| `unica.subsystem.info` | exclusive `sourceSet` or `SubsystemPath`; `metadataPath?` only with sourceSet; `delivery?`, `filter?`, `page?`, `resultRef?`, `section?`, `cwd?`, `confirm?` | resolve the sourceSet branch from `metadataPath` and the path branch from `SubsystemPath` to one `at`, then call `unica.view({"at":at,"filter":{"sections?":section},"cursor?":page})`. | absorbed; public base view exists. Old delivery/report-page semantics unmodelled. |
| `unica.subsystem.validate` | exclusive `sourceSet+metadataPath` or `SubsystemPath`, `nativeArgs?`, `cwd?`, `confirm?` | `unica.check({"at":A(sourceSet,metadataPath),"filter":{"validation":{"profile":"subsystem"}}})`. | deferred; the profile belongs to the closed filter union, but its resolver, real validator execution and diagnostics are absent. |
| `unica.support.edit` | `Path|path|TargetPath|targetPath`; exclusive `Capability|capability:"on"|"off"` or `Set|set:"editable"|"off-support"|"locked"`; `dryRun=false` | normalize the path alias to `target`; the capability branch maps to `{"op":"supportCapability.set","args":{"values":{"capability":capability}}}` and the set branch to `{"op":"supportRule.set","args":{"values":{"rule":set}}}`; call `unica.apply({"at":A(S,target),"ops":[mappedOperation],"dryRun":dryRun})`. | mapped; registry names/planner stub. Resolver and typed exclusivity must be retained. |
| `unica.template.add` | `ObjectPath|objectPath|Path|path`, `TemplateName|templateName`, `TemplateType|templateType?`, `dryRun=false` | `unica.apply({"at":A(S,ObjectPath),"ops":[{"op":"template.add","args":{"name":TemplateName,"type?":TemplateType}}],"dryRun":dryRun})`. | mapped; registry/planner stub. Template collection addressing/type schema needed. |
| `unica.template.remove` | `SrcDir|srcDir?`, `ObjectName`, `TemplateName`, `dryRun=false` | `unica.apply({"at":"S:<ObjectName>.Template.<TemplateName>","ops":[{"op":"template.remove","args":{}}],"dryRun":dryRun})` after source resolution. | mapped; registry/planner stub. Must port reference/removal guard. |
| `unica.xdto.edit` | `sourceSet`, `metadataPath`, `operations`, `dryRun=false` | map each legacy operation to a separate canonical op object: `valueType.add`, `objectType.add`, `property.add`, `type.remove`, or `property.remove`; then call `unica.apply({"at":"sourceSet:metadataPath","ops":mappedOperations,"dryRun":dryRun})`. XDTO operation `args.at` carries the same/descendant target where the registry requires it. | mapped; Xdto registry names exist, planner stub. Exact legacy closed union and XDTO invariants must be ported; invented `xdto.*` op names are invalid. |
| `unica.xdto.info` | `sourceSet`, `metadataPath`; `typeName?`, `limit?`, `cursor?`, `cwd?`, `confirm?` | `unica.view({"at":"sourceSet:metadataPath","filter":{"xdto":{"typeName?":typeName}},"limit?":limit,"cursor?":cursor})`. | absorbed; public base view exists. XDTO filter/projection/cursor not yet implemented. |

## Current implementation truth and cutover gates

The selected package surface is now v0.13: clients with native Tasks receive
exactly eight subject tools, and compatibility clients receive those eight plus
the three `unica.task.*` calls.  The production stdio frontend connects to the
user daemon and cannot fall back to legacy execution.  Every subject tool has a
useful closed mode: base logical `view`/`find`, literal BSL `search`,
admission/readability `check`, filtered same-kind structural `diff`, the closed
`run` dictionary, safe platform/standard `docs` search, and retained
dry-run/real metadata apply for `props.set`. Validation profiles, the
`syntax.check` is counted as supported through its bounded durable Task;
four metadata operations have proven publication. Configuration documentation is explicitly
`unsupported_source` until its workspace traversal moves behind the actor-owned
nofollow/cancellation reader.

This remains a surface-first partial implementation, not full 74-call semantic
parity. Object/relation and other apply families, every run operation,
specialized validators, regex, twelve remaining run operations and several legacy projections return the typed
`unsupported_*` result named in the matrix. Full v0.12 parity
and the manual skills review gate do not block the
merge.  They do block calling the respective modes supported; release remains a
separate gate.

## Registry consistency exposed by the cutover

The cutover changes existing mutable rules and supersedes an accepted but
unbuilt planned decision.  The registry previously required every historical
`decision.establishes` entry to point back from the rule, while immutability
forbade deleting that historical entry; it also required realized evidence
from every superseded decision, including one never built.  Those conditions
made a valid successor impossible to represent.  Process decision
`DEC.2026-08-31.HISTORICAL-RULE-OWNERSHIP` keeps historical establishes links,
requires only the current owner to be reciprocal, and permits an unbuilt
superseded direction to retain `realized: null`.

Before a partial cutover, implement only the closed schemas and fixtures for
modes advertised as supported, including their qualified-address resolution,
revision/cursor and canonical-diagnostics behavior, and one-invocation/one-Task
ownership where applicable.  The deliberate removals are caller-controlled tool
download and durable job logs/list; terminal client/session and artifact/export
cases remain typed unsupported until their contracts exist.

## Mechanical completeness check

The following command must print `74 74 [] []`; it verifies one table row per
baseline name, no duplicated row name, and no omitted or extra name:

```zsh
python3 - <<'PY'
import json, re
from pathlib import Path
baseline = set(json.loads(Path('tests/fixtures/migration/v0.12.3-baseline.json').read_text())['wire']['toolNames'])
text = Path('docs/design/2026-08-31-v0-13-surface-first-cutover-design.md').read_text()
rows = re.findall(r'^\\| `([^`]+)` \\|', text, re.M)
names = set(rows)
print(len(rows), len(names), sorted(baseline - names), sorted(names - baseline))
assert len(rows) == len(names) == len(baseline) == 74
assert names == baseline
PY
```

## Evidence

The exhaustive legacy inventory is `tests/fixtures/migration/v0.12.3-baseline.json`.
Legacy selectors and implementation seams were cross-read from
`crates/unica-coder/src/application/mod.rs`,
`crates/unica-coder/src/application/tool_contracts.rs`, and
`arch/tool-surface.md`; target envelopes and current status from
`crates/unica-coder/src/application/v13/tool_catalog.rs` and
`docs/design/2026-08-23-v0-13-execution-surface-design.md`.  The detailed
working matrices under `.superpowers/v13-surface-matrix-audit/` are the audit
inputs for this tracked record.
