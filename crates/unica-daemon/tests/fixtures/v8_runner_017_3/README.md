# Runner 0.17.3 envelopes: the layer of an environment refusal and the warning code

Captured 2026-11-14 on Windows from the published release binary of v8-runner
`v0.17.3` (IngvarConsulting/v8-runner-rust `c5164137`):
`v8-runner-windows-x86_64.zip`, archive SHA-256
`a330420ac945d6037bb244b2b93a98090b437b58abf0ecfcc7d1853a08a5e731`, `v8-runner.exe`
SHA-256 `cba8cf57e248396e173c8bd6cff3f78c085f591cde5904979e39e2bf74b2cb04`. The stand
directory is replaced by `C:\workspace` and the host name by `workstation`;
every other byte is as captured.

Working copies have `format: DESIGNER` and one `CONFIGURATION` set `main` in
`src`. The infobase `origin` is declared in `v8project.local.yaml`.

- `attach-unreachable.json`: `pull main --force` with `providers: {pull:
  agent}` and `tools.designer_agent.attach` on a port nothing listens on —
  `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `attach-foreign-ssh-server.json`: the same with another server on the port:
  it sends an SSH banner and then bytes no SSH client accepts —
  `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `pull-takes-unowned-infobase.json`: `pull main --force` in working copy `C`
  over a file infobase without an owner (created empty by `ibcmd` 8.3.27.2342):
  the copy takes it. The warning has no code, and there is no `warning_details`.
- `push-other-copy-force-preview.json`: `push --force --dry-run` in working copy
  `D`, which declares the same infobase that `C` holds: the warning and its
  `warning_details` entry `infobase_of_another_copy`, exit 0.
