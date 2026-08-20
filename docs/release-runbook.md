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

So publication is split, per
[ADR-0008](../spec/decisions/0008-public-marketplace-thin-runtime.md):

1. **Stage** — put the plugin bytes on the marketplace default branch. The
   catalog still names the previous tag, so no consumer is affected yet.
2. **Promote** — move the catalog to the new tag. This is the moment the release
   goes live.

Between the two sits an immutable tag the catalog pins `git-subdir` to, which
`scripts/verify_marketplace.py` in the marketplace repo enforces.

## One human action, one linear pipeline

Per [ADR-0068](../spec/decisions/0068-lineynyy-konveyer-postavki.md) the whole
publication runs as one pass of **Publish Unica Marketplace**, started
automatically when the tag-triggered build succeeds:

| You | The pipeline |
| --- | --- |
| Step 0 — set the version | |
| Step 1 — tag the source release | build → assets → BSP assessment |
| | stage the payload (catalog untouched) |
| | create the anchor tag on the staging commit |
| | consumer install checks: fresh + upgrade, three hosts |
| | green → move the catalog → **live** |

Your signed source tag is the human approval and the cryptographic anchor of
the release. Be honest about what enforces it: the pipeline proves the tag
exists and that the payload came from its successful push build, but it does
not verify the signature itself — GitHub reports these signatures as
unverified today. What keeps the tag trustworthy is write access and the
repository's tag protection rules; keep those protections on. The marketplace
tag is created by the pipeline: it is the ref the catalog resolves, and
nothing verifies its signature — the runbook used to ask for a second signed
tag, and ADR-0068 retired it.

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
- `main` is green, and the version bump is ready to verify and merge.

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

Then merge through a pull request as usual.

## Step 1 — tag the source release

Tag the merged release commit on `main` in `unica` and push. The tag triggers
the release build, and the successful build starts the publication pipeline on
its own.

```bash
git tag -s vX.Y.Z <release-commit-sha> -m "Unica vX.Y.Z"
git push origin vX.Y.Z
```

The version must be fixed before artifacts are built: the runtime manifest
embeds `release.tag` and derives every asset URL from it, and the bootstrap
rejects a manifest whose URL disagrees with its declared version.

## Step 2 — watch it land

The tag push runs **Build Unica Codex Plugin** (runtime for three targets,
`unica-runtime-<target>.tar.gz` with SHA-256 metadata on the GitHub release,
the thin payload as the `unica-thin-marketplace` artifact), and its success
triggers **Publish Unica Marketplace**: stage → tag → verify → promote.

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
| **catalog moved** | **the release is live** |

## A prerelease: built, published, never served

Some things can only be measured against a real release — a runtime manifest
pins its assets to `github.com/IngvarConsulting/unica/releases/download/<tag>/`,
so nothing but a published tag will do. A prerelease is the release that exists
for us and not for consumers.

Give the version a SemVer prerelease suffix and tag it as usual:

```bash
python3.12 scripts/dev/bump-version.py 0.13.0-rc.1
cargo update --workspace --offline
# merge, then tag as in step 1
```

The suffix is part of the version, not a label beside it, because the runtime
manifest requires the tag to equal `v` + the plugin version literally.

What the pipeline does with it:

| Stage | Prerelease |
| --- | --- |
| build, assets on the GitHub release | runs — and marks the release as a prerelease |
| stage, anchor tag, install checks, promote | **skipped** |

The publish workflow asks first: its `gate` job reads the source tag and stops
the whole publication when the tag carries a suffix. Nothing is disabled by
hand, so a colleague tagging a real release meanwhile is unaffected.

A prerelease burns its own version number, never the stable one: measure
against `0.13.0-rc.1`, then release `0.13.0` from the same code. Keep it marked
as a prerelease — it is not a release waiting to be served, and `gh release
view` without a tag must keep naming the last stable one.

## One-way doors

Two things can never be taken back once published, because other artifacts
reference them by identity:

- **Release assets** in `unica`. Runtime manifests pin them by SHA-256.
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

Reverting is a one-file change, and it works because published bytes never move,
so the previous tag still resolves to exactly what it always did.

```bash
git clone https://github.com/IngvarConsulting/unica-marketplace.git /tmp/unica-marketplace
cd /tmp/unica-marketplace
git revert --no-edit <promotion-commit-sha>
git push origin main
```

Confirm the catalog names the previous tag again, then treat the bad version as
burnt and fix forward in the next patch. Consumers move back on their next
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
| `tag` fails on an existing tag | The version was already published with different bytes | Never move the tag; release the next patch |
| Packaging fails with `release tag ... != ...` | Step 0 missed the workflow `RELEASE_TAG` fallback | Bump it and re-run |
| Consumers still report the old version | The publish run did not finish | Check its failed stage and rerun |

## Never

- Move or delete a published tag, or force-push the marketplace default branch.
  Consumers resolve `git-subdir` against those refs; changed bytes require a new
  version.
- Point the catalog at a tag by hand. The promote job is the only writer of the
  catalog files, and it runs only behind green install checks.
