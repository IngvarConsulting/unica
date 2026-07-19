# Unica v0.7.7 Migration Bridge Amendment

## Status

Accepted correction to the `v0.7.6` bridge design.

## Barrier finding

The published `v0.7.6` manual full-history regression failed for `v0.3.3` and
the issue #90 duplicated layouts on all operating systems. Those states contain
a legacy package below `plugins/cache/unica/unica`, which is also the parent of
the newly installed canonical package. The original transaction verified the
new package and then removed the captured parent path, deleting its own result.

Because the signed `v0.7.6` tag and release assets are immutable, this defect is
corrected only in patch release `v0.7.7`. `v0.7.6` is not a supported legacy
bridge.

## Corrected transaction boundary

After legacy registrations are removed, any captured legacy path that contains
the canonical plugin destination is removed before Codex installs the new
package. Non-overlapping legacy paths remain until the installed package,
runtime, MCP tool list, and prompt-visible skills have been verified. The
existing transaction backup owns rollback for both groups.

The canonical migration entry points are the immutable `v0.7.7` assets:

- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.7/install-unica.sh`
- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.7/install-unica.ps1`

Issue #90 may close only after the marketplace dispatch-only `bridge` profile
succeeds against published and promoted `v0.7.7` bytes on macOS, Linux, and
Windows, including the injected rollback jobs. `v0.8.0` may remove legacy code
only after that proof exists.
