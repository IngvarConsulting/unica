---
name: security-auth-crypto
description: "Безопасная аутентификация и криптография 1С. Используй когда нужно спроектировать или диагностировать OpenID, сертификаты, CryptoPro, TLS, роли, секреты и auth для интеграций."
---

# Security Auth Crypto

## MCP routing

- Preferred path: use MCP `unica` tools `unica.project.map`, `unica.code.search`, `unica.meta.info`, `unica.role.info`, `unica.code.diagnostics`, `unica.standards.search`, `unica.standards.explain`, and `unica.runtime.execute`.
- Use integration and autonomous-server skills when auth behavior must be verified through HTTP service or web-client runtime.
- Do not call internal analyzer, standards, runtime, or package adapters directly. They are hidden behind MCP `unica`.

## References

- Read `../../references/platform/integration-contracts.md` for auth contract, secret handling, retry behavior, and error semantics.
- Read `../../references/platform/platform-mechanics.md` for client/server crypto boundary, rights, OpenID, certificates, and temporary secret handling.
- Read `../../references/platform/runtime-diagnostics.md` for startup/auth/runtime evidence.

## Workflow

1. Identify the trust boundary: user login, service account, external API, OpenID provider, certificate store, CryptoPro provider, TLS endpoint, or file/key storage.
2. Inspect existing auth and role paths with `unica.code.search`, `unica.meta.info`, and `unica.role.info`.
3. Define secret lifecycle: source, storage, rotation, masking, runtime process user, test fixture policy, and log redaction.
4. Define auth error semantics: missing credentials, expired token, invalid certificate, provider unavailable, denied rights, tenant mismatch, and remote auth failure.
5. Verify code with `unica.code.diagnostics` and syntax/tests through `unica.runtime.execute` when feasible.

## Review checklist

- Secrets and private keys are not committed, logged, or echoed in final output.
- Certificate/OpenID/CryptoPro behavior states platform version, OS/process user, store location, and client/server boundary.
- Rights checks are explicit and audited with `unica.role.info` when metadata rights matter.
- Integration auth failures are distinguishable from validation and business failures.
- Temporary files with sensitive data have clear cleanup and access boundaries.

## Contract gaps

If public MCP `unica` cannot inspect the needed role, runtime, certificate, or auth artifact, report a Unica MCP contract gap instead of bypassing the public boundary.
