# Third-Party Notices

Pinned third-party tool versions, repositories, commits, licenses, and target
asset names are declared once in `third-party/tools.lock.json`.

Official marketplace archives generate `third-party/manifest.json` from that
lock file and CI-built binary bundles. The repository source tree does not
commit generated tool binaries.

## bsl-analyzer

- Included notices: `third-party/licenses/bsl-analyzer/`

## v8-runner

- Included license: `third-party/licenses/v8-runner/LICENSE`
- See `third-party/tools.lock.json` for the pinned repository, version, tag,
  commit, AGPL-3.0-only license field, and target assets.

## rlm-tools-bsl

- Included notices: `third-party/licenses/rlm-tools-bsl/`

When updating tool versions, update `third-party/tools.lock.json`, bump the
plugin version, and let the release workflow regenerate the manifest and
archives.
