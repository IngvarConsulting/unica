# Testing

Test operations build first, so an applied run carries the build's
non-interruptible phase: cancelling mid-run leaves the infobase in an unproved
state. Preview the argument selections below with `dryRun=true`, run them with
`dryRun=false`, and choose `unica.runtime.job.start` when the call must not wait
for the result.

Use `operation=test`, `testRunner=yaxunit`, `testScope=all` for full YaXUnit.
Add `fullOutput=true` when you need the runner `--full` output verbosity.
This is not a source build full rebuild.

Use `operation=test`, `testRunner=yaxunit`, `testScope=module`, and `module=<name>` for narrow module-level tests.

Use `operation=test`, `testRunner=va` for the configured Vanessa Automation profile. Optional VA narrowing arguments are `features`, `filterTags`, `ignoreTags`, and `scenarioFilters`. Do not invent feature paths without inspecting project test configuration.

Preview `operation=launch`, `clientMode=mcp-va`, `dryRun=true` for interactive Vanessa Automation scenario authoring and debugging through client MCP; this detached launch is not currently admitted.

Preview syntax validation with `operation=syntax`, `dryRun=true`, and
`mode=designer-modules`, `mode=designer-config`, or `mode=edt`. Designer modes
accept client/server flags such as `server`, `thinClient`, `webClient`,
`mobileClient`, `extension`, and `allExtensions`; EDT accepts `projects`. Every
mode carries unproved cleanup ownership on every runner failure path, so a
crashed run can leave a platform process behind.

Preserve failed test artifacts and report their path when the runner prints one.
