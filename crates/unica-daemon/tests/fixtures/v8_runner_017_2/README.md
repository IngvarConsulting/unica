# Runner 0.17.2 envelopes: the layer of an environment refusal and the warning code

Captured 2026-11-14 on Windows from v8-runner built from tag `v0.17.2`
(IngvarConsulting/v8-runner-rust `610a8618`, envelope 6), binary SHA-256
`470393ee3591374558f35a76ef4dc2be961f1bebd07bc24a3c25b1cfc47e4b7f`. The stand
directory is replaced by `C:\workspace` and the host name by `workstation`;
every other byte is as captured.

Working copies have `format: DESIGNER`, one `CONFIGURATION` set `main` in
`src`, and the file infobase `origin` declared in `v8project.local.yaml`.

- `attach-unreachable.json`: `pull main --force` with `providers: {pull:
  agent}` and `tools.designer_agent.attach` on a port nothing listens on —
  `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `attach-foreign-ssh-server.json`: the same with another SSH server on the
  port: it sends its banner and then bytes no SSH client accepts —
  `environment_unavailable`, `layer: agent_endpoint`, exit 2.
- `pull-takes-unowned-infobase.json`: `pull main --force` in working copy `C`
  over an infobase without an owner (created empty by 8.3.27.2342): the copy
  takes it. The warning has no code, and there is no `warning_details`.
- `push-other-copy-force-preview.json`: `push --force --dry-run` in working copy
  `D`, which declares the same infobase that `C` holds: the warning and its
  `warning_details` entry `infobase_of_another_copy`, exit 0.
