# Release runbook

How to publish a Unica version to the public marketplace, and why each step
exists. Follow it top to bottom; every step states what to verify before moving
on.

Two repositories are involved:

- `IngvarConsulting/unica` — source, runtime assets, release automation.
- `IngvarConsulting/unica-marketplace` — the public catalog consumers install
  from.

## Why publication has two phases

The catalog must never point at bytes that are not final. If it moved in the
same step that published them, a partial or unverified upload would be served to
every consumer immediately.

The release workflow and its contract tests therefore split publication:

1. **Stage** — put the plugin bytes on the branch of the release's channel:
   `main` for a stable release, `next` for a candidate. The catalogs still name
   the previous tag, so no consumer is affected yet.
2. **Promote** — move the catalogs of the channel to the new tag. This is the
   moment the release goes live.

Between the two sits an immutable tag the catalog pins `git-subdir` to, which
`scripts/verify_marketplace.py` in the marketplace repo enforces.

## One human action, one linear pipeline

The workflow runs the whole publication as one pass of **Publish Unica
Marketplace**, started automatically when the tag-triggered build succeeds:

| You | The pipeline |
| --- | --- |
| Step 0 — set the version | |
| Step 1 — tag the source release | build → assets → BSP assessment |
| | stage the payload (catalog untouched) |
| | create the anchor tag on the staging commit |
| | consumer install checks: fresh + upgrade, three hosts |
| | green → move the channel's catalogs → **live** |
| Step 3 — merge the line back into `main` | |

Your signed source tag is the human approval and the cryptographic anchor of
the release. Be honest about what enforces it: the pipeline proves the tag
exists and that the payload came from its successful push build, but it does
not verify the signature itself — GitHub reports these signatures as
unverified today. What keeps the tag trustworthy is write access and the
repository's tag protection rules; keep those protections on. The marketplace
tag is created by the pipeline: it is the ref the catalog resolves, and
nothing verifies its signature — the runbook used to ask for a second signed
tag. The linear pipeline removed that wait.

There is no scheduler and no waiting window: a failed stage is a red run
attached to the release tag, and the catalog stays where it was. Rerunning the
failed workflow resumes the publication — every stage is idempotent.

## Preconditions

- Write access to both repositories, and `gh` authenticated. The tag step
  pushes over HTTPS, so run `gh auth setup-git` once in a fresh checkout.
- A GPG key able to sign the source tag. If signing fails with `Operation
  cancelled` in a non-interactive shell, run
  `gpg-connect-agent updatestartuptty /bye` first, then
  `echo test | gpg --clearsign > /dev/null` to unlock the agent.
- The branch the release is cut from is green, and the version bump is ready
  to verify and merge.

## Where a release is cut from

A minor is cut from `main`. Patches for a minor that already shipped are cut
from its line branch, `release-vX.Y`, and that branch is where the version bump
lives: `main` keeps the minor's version and never takes the patch bumps. When
0.12.3 shipped, `release-v0.12` declared 0.12.3 while `main` still declared
0.12.0.

Steps 0 and 1 therefore happen on the branch the release is cut from, and the
tag names a commit on it. Step 3 brings the line back to `main`.

## Step 0 — prepare the version

One command writes the version everywhere the package contract declares it, then
runs the contract check:

```bash
python3.12 scripts/dev/bump-version.py X.Y.Z
cargo update --workspace --offline
```

The version lives in several files because each is read by a different consumer:
Cargo compiles it into the binaries, the two host manifests ship it to Codex and
Claude Code, and the tools lock pins it beside the third-party tools. They are
separate artifacts, so it cannot live in one file — but it is written by one
command and enforced by one check,
`scripts/ci/check-version-contract.py`, which fails the build if any of them
drift apart.

Tests that assert the current version still need updating by hand; they fail with
an explicit diff, so run the suite before opening the pull request:

```bash
python3.12 -m pip install -r tests/ci/requirements.txt
python3.12 -m unittest discover -s tests/ci
```

Then merge through a pull request into the branch the release is cut from.
Keep the bump its own pull request: step 1 tags its merge commit, so that commit
has to carry both the bump and everything else the version ships.

## Step 1 — tag the source release

Tag the merge commit of the version pull request in `unica` and push. On a
patch that commit is on `release-vX.Y`, not on `main`. The tag triggers the
release build, and the successful build starts the publication pipeline on its
own.

```bash
git tag -s vX.Y.Z <release-commit-sha> -m "Unica vX.Y.Z"
git push origin vX.Y.Z
```

The version must be fixed before artifacts are built: the runtime manifest
embeds `release.tag` and derives every asset URL from it, and the bootstrap
rejects a manifest whose URL disagrees with its declared version.

## Step 2 — watch it land

The tag push runs **Build Unica Codex Plugin**, and its success triggers
**Publish Unica Marketplace**: stage → tag → verify → promote.

### What the release carries

Only the core is built here, so only the core is published here:

| On the GitHub release | Count | What it is |
| --- | --- | --- |
| `unica-runtime-<target>.tar.gz` | 3 | the core, one archive per target |
| `unica-runtime-<target>.json` | 3 | what the core pins: version, asset, SHA-256, file closure |

Six assets, and both halves have a reader: `verify-published-assets` re-downloads
each pair and rehashes every member. Descriptions of the *engine* artifacts stay
inside the build — the packager reads them from the workflow artifact, and on the
release they would be a third copy of facts already in `tools.lock.json` at the
source tag and in the published plugin's `runtime-manifest.json`, naming an asset
this release does not carry.

The plugin release references engine assets by URL and SHA-256 without
republishing them. `v8-runner` comes from releases of
`IngvarConsulting/v8-runner-rust`; other engines come from
`IngvarConsulting/unica-toolchain`, as specified by the
[engine source rule](../arch/rules/distribution/engine-release-origins.md).

That splits verification three ways, and each part is a job in the build:

| What | Checked by | How |
| --- | --- | --- |
| core bytes | `verify-published-assets` | re-downloads the release assets and rehashes every member |
| every asset address | `verify-published-assets` | HEAD on all twelve URLs the manifest names, across all three targets |
| the whole delivery | `smoke-thin-plugin` (linux-x64) | `unica-bootstrap prefetch` — address, checksum and layout, end to end |

The address check exists because the toolchain bytes are verified when the build
downloads them, and nothing touches the address after that: a typo in a tag would
otherwise surface at a user's first engine call.

```bash
gh run list --workflow "Build Unica Codex Plugin" --limit 3 \
  --json databaseId,headBranch,conclusion
gh run list --workflow "Publish Unica Marketplace" --limit 3 \
  --json databaseId,conclusion
```

The release is live when the catalog names the new tag — both host catalogs
move in the same commit:

```bash
gh api repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json \
  --jq '.content' | base64 -d | grep '"ref"'
```

The marketplace repository runs its own **Verify marketplace** on every push it
receives, so `stage`, `tag` and `promote` each leave a run there, after the
fact. Those runs do not gate anything: the pipeline's `verify-upgrade` and
`verify-fresh-install` jobs already ran on three hosts before `promote` moved
the catalog.

## Step 3 — merge the line back into `main`

A patch leaves `main` behind, because the fix and the bump landed on the line
branch only. Merge the line back through a `merge/release-vX.Y.Z-into-main`
pull request once the release is live.

The version contract conflicts every time. Keep `main`'s side in all five files:
the line changed nothing there but the number. `Cargo.lock` conflicts for the
same reason and also keeps `main`'s side, which carries the real dependency
state while the line moved only the two workspace crate versions.

## If a stage fails

The pipeline stops before the catalog moves, so consumers are unaffected.
Rerun the whole workflow after fixing the cause — completed stages detect
themselves and pass through:

```bash
gh run rerun <publish-run-id> --failed
```

To run the pipeline for a build that already succeeded (for example after the
`workflow_run` trigger was missed), dispatch it with the build's run id:

```bash
gh workflow run publish-unica-marketplace.yml --repo IngvarConsulting/unica \
  -f source_run_id=<build-run-id>
```

## What consumers see, and when

Only one step changes anything for consumers. Everything before it is invisible
to them, which is what makes aborting cheap.

| After | Visible to consumers |
| --- | --- |
| source tag pushed | nothing |
| assets published | nothing — no catalog names them |
| payload staged | nothing — the catalog still names the previous tag |
| anchor tag pushed | nothing |
| install checks green | nothing |
| **catalog moved** | **the release is live** — for a candidate, only on the `next` channel |

## A release candidate: served on `next` only

Some things can only be measured against a real release — a runtime manifest
pins its assets to `github.com/IngvarConsulting/unica/releases/download/<tag>/`,
so nothing but a published tag will do — and a candidate needs testers before
the release it becomes. A release candidate is published to the `next`
channel of the marketplace and never to the stable catalog.

Give the version an `-rc.N` suffix and tag it as usual:

```bash
python3.12 scripts/dev/bump-version.py 0.13.0-rc.1
cargo update --workspace --offline
# merge, then tag as in step 1
```

The suffix is part of the version, not a label beside it, because the runtime
manifest requires the tag to equal `v` + the plugin version literally.

What the pipeline does, by the source tag:

| Tag | Assets | Stage, anchor tag, install checks | Catalogs moved |
| --- | --- | --- | --- |
| `vX.Y.Z` | published | on `main` | `next` unless it serves a newer candidate, then `main` |
| `vX.Y.Z-rc.N` | published, marked as a prerelease | on `next` | `next` only |
| any other suffix | published, marked as a prerelease | skipped | none — a measurement build |

The `next` branch of `unica-marketplace` carries catalogs named `unica-next`:
Claude Code identifies a marketplace by its name, so a channel cannot reuse
`unica`. When the branch is missing, the first publication starts it as a copy
of `main`. Every publication to `next` also brings over from `main`
everything but the served plugin and the channel's catalogs, so the
marketplace checks `next` with the same scripts and workflows; do not edit
them on `next`. `scripts/ci/release-channel.py` owns the channel rules and the
version order, which is SemVer: `v0.13.0` is newer than `v0.13.0-rc.3`, while
`sort -V` ranks them the other way round. A candidate that is older than what
either channel serves is refused at stage.

Two guarantees follow: the
[stable catalog names only releases](../arch/rules/distribution/stable-catalog-releases.md),
and the
[`next` channel is never older than it](../arch/rules/distribution/next-channel-order.md).
Testers therefore update from a candidate to the release it became the same
way they received the candidate; how they join and leave the channel is in the
[plugin README](../plugins/unica/README.md#release-candidates).

A candidate burns its own version number, never the stable one: measure
against `0.13.0-rc.1`, then release `0.13.0` from the same code. "The same
code" still means a second bump pull request — the version is part of the
package contract, so `0.13.0` is a commit, not a relabelling of `0.13.0-rc.1`.

Two things follow from engines being named rather than republished. The
measurement is cheaper than it looks: engine artifacts keep their own versions,
so `0.13.0-rc.1` and `0.13.0` name the same engine bytes, and a machine that
warmed its cache on the prerelease downloads nothing but the core for the
stable. And the prerelease's own assets are only the core — the toolchain
releases it points at are already published and are not touched.

Keep a candidate marked as a prerelease — `gh release view` without a tag
must keep naming the last stable one.

## Explicit runtime verification

`unica-bootstrap verify` installs the runtime if needed, checks the skill
package and probes the MCP protocol and tool list. The ordinary bootstrap
launch does not repeat this probe. The command currently passes a 20-second
waiting budget; this is not a deadline for the whole installation and all
protocol exchanges. See the [verification contract](../arch/rules/distribution/bootstrap-protocol-verification.md)
for the checks and their limits.

## One-way doors

Two things can never be taken back once published, because other artifacts
reference them by identity:

- **Release assets** in `unica`. Runtime manifests pin them by SHA-256.
- **Release assets in `unica-toolchain`.** Published Unica versions name them by
  address and SHA-256, so deleting a toolchain release, moving its tag, or
  re-uploading an asset under the same name breaks every Unica version that
  pinned it — including versions released long ago. Toolchain releases are as
  immutable as this repository's own.
- **Tags** in either repository. Consumers resolve `git-subdir` against them.

This gives the rule that replaces rollback: **never reuse a version number**. If
anything is wrong after step 1, abandon that version and release the next patch
instead. Re-cutting `vX.Y.Z` with different bytes breaks every consumer that
already resolved it.

Whenever you abandon a version whose assets are already published, mark that
release so it stops looking like a release waiting to be served:

```bash
gh release edit vX.Y.Z --repo IngvarConsulting/unica --prerelease
```

Never delete a tag to "clean up" an abandoned version. An unused tag costs
nothing; a deleted one that something already resolved costs every consumer.

## Rolling back a live release

Reverting touches only promotion commits, and it works because published bytes
never move, so the previous tag still resolves to exactly what it always did.
Revert the promotion commit on every branch the release moved: `main` for a
stable release, and `next` too when the release moved it rather than leaving a
newer candidate there; `next` alone for a candidate. Revert `main` first, so
`next` never falls behind it:

```bash
git clone https://github.com/IngvarConsulting/unica-marketplace.git /tmp/unica-marketplace
cd /tmp/unica-marketplace
git revert --no-edit <main-promotion-commit-sha>
git push origin main
git switch next
git revert --no-edit <next-promotion-commit-sha>
git push origin next
```

Confirm every reverted catalog names the previous tag again, then treat the bad
version as burnt and fix forward in the next patch. Consumers move back on their next
update; those who already installed the bad version keep it until then, so
prefer fixing forward when the fault is not severe.

## The one state to avoid

A catalog that names a tag which does not exist. Every install then fails with
`pathspec 'vX.Y.Z' did not match any file(s)`, including for consumers who had
been working fine.

The pipeline cannot reach it — the promote job requires the tag job — so it has
one remaining cause, which is preventable outright by protecting tags in the
marketplace repository: deleting or moving a published tag by hand.

## Failure modes

| Symptom | Cause | Action |
| --- | --- | --- |
| Publish run failed at `stage` or `tag` | Transient push failure or a moved branch | `gh run rerun <run-id> --failed`; stages are idempotent |
| Publish run failed at the install checks | The candidate does not install as a consumer | Fix forward; the version is burnt, the catalog never moved |
| `stage` fails with `vX.Y.Z-rc.N is older than vA.B.C` | A newer release or candidate is already served | The candidate is stale; tag the next `-rc.N` of a newer version |
| `tag` fails on an existing tag | The version was already published with different bytes | Never move the tag; release the next patch |
| Packaging fails with `release tag vX.Y.Z != vA.B.C` | The tagged commit does not declare X.Y.Z: step 0 was not merged, or the wrong commit was tagged | Tag the merge commit of the version pull request. If the bad tag was pushed, that version is burnt — take the next patch |
| `Verify marketplace` red after the catalog moved, at `previous-stable-upgrade` with `git clone marketplace source timed out after 30s` | Codex re-clones the whole marketplace on `plugin marketplace upgrade`; a clone that misses its own 30s budget leaves the stale local catalog, which then installs the previous version | Transient. `gh run rerun <run-id> --failed`. Consumers are unaffected: the catalog is already correct and the pipeline's own upgrade checks passed before promote |
| Consumers still report the old version | The publish run did not finish | Check its failed stage and rerun |
| `verify-delivery-reachable` fails with HTTP 404 | The toolchain asset the lock names is gone or was renamed | Never re-tag a toolchain release; point the lock at a published asset and cut the next patch |
| The prefetch step fails on a checksum | Toolchain bytes were replaced under a published name | Treat the toolchain release as burnt, publish a new toolchain build, bump the lock |
| `build-tools` fails while downloading a tool asset | The lock names a toolchain tag that does not exist yet | Publish the toolchain release first; the lock may only pin what is already public |

## Never

- Move or delete a published tag, or force-push `main` or `next` of the
  marketplace. Consumers resolve `git-subdir` against those refs; changed bytes
  require a new version.
- Point a catalog at a tag by hand, or point `main` at a candidate. The promote
  job is the only writer of both channels' catalog files, and it runs only
  behind green install checks.
