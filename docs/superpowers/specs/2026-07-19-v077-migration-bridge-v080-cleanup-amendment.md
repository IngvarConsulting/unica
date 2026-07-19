# Unica v0.7.7 Technical Migration Amendment

## Status

Superseded by the stable `v0.7.8` bridge. This document records why `v0.7.7`
was created, but `v0.7.7` is now classified as a technical release.

## Barrier finding

The published `v0.7.6` manual full-history regression failed for `v0.3.3` and
the issue #90 duplicated layouts on all operating systems. Those states contain
a legacy package below `plugins/cache/unica/unica`, which is also the parent of
the newly installed canonical package. The original transaction verified the
new package and then removed the captured parent path, deleting its own result.

Because the signed `v0.7.6` tag and release assets are immutable, the overlap
defect was corrected in patch release `v0.7.7`. The later tag/catalog and ref
propagation defects mean that neither version is a supported legacy bridge.

## Corrected transaction boundary

After legacy registrations are removed, any captured legacy path that contains
the canonical plugin destination is removed before Codex installs the new
package. Non-overlapping legacy paths remain until the installed package,
runtime, MCP tool list, and prompt-visible skills have been verified. The
existing transaction backup owns rollback for both groups.

The supported migration entry points are the immutable `v0.7.8` assets:

- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.sh`
- `https://github.com/IngvarConsulting/unica/releases/download/v0.7.8/install-unica.ps1`

Issue #90 may close only after the marketplace dispatch-only `bridge` profile
succeeds against published and promoted `v0.7.8` bytes on macOS, Linux, and
Windows, including the injected rollback jobs. `v0.8.0` may remove legacy code
only after that proof exists.
