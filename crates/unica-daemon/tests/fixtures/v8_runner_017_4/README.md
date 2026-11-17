# Runner 0.17.4 envelopes: the layer of an environment refusal and the warning code

Captured 2026-11-15 on Windows from the published release binary of v8-runner
`v0.17.4` (IngvarConsulting/v8-runner-rust `0781c9b1`):
`v8-runner-windows-x86_64.zip`, archive SHA-256
`b0fa55084664b86c96589f85e1aa9a57396c12b207d36c796ee4afd277ade38f`, `v8-runner.exe`
SHA-256 `f263f665349dc4ff50be9b840a147a78cb5221786ae48871fddc9a01dc71da90`. The stand
directory is replaced by `C:\workspace` and the host name by `workstation`;
every other byte is as captured.

Working copies have `format: DESIGNER` and one `CONFIGURATION` set `main` in
`src`. The infobase `origin` is declared in `v8project.local.yaml`.

- `attach-unreachable.json`: `pull main --force` in working copy `endpoint`,
  which declares an infobase of its own, with `providers: {pull: agent}` and
  `tools.designer_agent.attach` (and its `base-dir`) on a port nothing listens
  on — `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `attach-foreign-ssh-server.json`: the same with another server on the port:
  it sends an SSH banner and then bytes no SSH client accepts —
  `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `pull-takes-unowned-infobase.json`: `pull main --force` in working copy `C`
  over a file infobase without an owner (created empty by `ibcmd` 8.3.27.2342):
  the copy takes it. The warning has no code, and there is no `warning_details`.
- `push-other-copy-force-preview.json`: `push --force --dry-run` in working copy
  `D`, which declares the same infobase that `C` holds: the warning and its
  `warning_details` entry `infobase_of_another_copy`, exit 0.
