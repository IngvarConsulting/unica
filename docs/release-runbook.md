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

Between the two sits an immutable tag. The catalog pins `git-subdir` to a semver
tag, which `scripts/verify_marketplace.py` in the marketplace repo enforces, and
the promotion checks install exactly as a consumer would. Those checks cannot
pass until the tag exists, which makes the tag the natural approval gate: it is
the one step that needs a human signing key.

## Preconditions

- Write access to both repositories, and `gh` authenticated.
- A GPG key able to sign tags. If signing fails with `Operation cancelled` in a
  non-interactive shell, run `gpg-connect-agent updatestartuptty /bye` first, then
  `echo test | gpg --clearsign > /dev/null` to unlock the agent.
- The version to release is already merged to `main` and green.

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
python3.12 -m unittest discover -s tests/ci
```

Then merge through a pull request as usual.

## Step 1 — tag the source release

Tag the merged release commit on `main` in `unica` and push. The tag is what
triggers the release build.

```bash
git tag -s vX.Y.Z <release-commit-sha> -m "Unica vX.Y.Z"
git push origin vX.Y.Z
```

The version must be fixed before artifacts are built: the runtime manifest
embeds `release.tag` and derives every asset URL from it, and the bootstrap
rejects a manifest whose URL disagrees with its declared version.

## Step 2 — wait for the release build

The tag push runs **Build Unica Codex Plugin**. It builds the runtime for the
three targets, publishes `unica-runtime-<target>.tar.gz` with SHA-256 metadata to
the GitHub release, and emits the thin marketplace payload as the
`unica-thin-marketplace` artifact.

On success it automatically opens a staging pull request in the marketplace
repository. No manual trigger is needed.

```bash
gh run list --workflow "Build Unica Codex Plugin" --limit 3 \
  --json databaseId,headBranch,conclusion
gh pr list --repo IngvarConsulting/unica-marketplace --state open
```

Note the run id — it is also encoded in the staging branch name
(`codex/stage-vX.Y.Z-<run-id>`), and the promote step needs it.

## Step 3 — merge the staging pull request

Review that it changes `plugins/unica` only and that the catalog is untouched,
then merge.

```bash
gh pr merge <staging-pr> --repo IngvarConsulting/unica-marketplace --merge
```

Merging publishes nothing: the catalog still names the previous tag. Record the
merge commit, the next step tags it.

```bash
gh pr view <staging-pr> --repo IngvarConsulting/unica-marketplace \
  --json mergeCommit --jq .mergeCommit.oid
```

## Step 4 — tag the marketplace

Tag the **staging merge commit**, before running the promotion.

```bash
git clone https://github.com/IngvarConsulting/unica-marketplace.git /tmp/unica-marketplace
cd /tmp/unica-marketplace
git tag -s vX.Y.Z <staging-merge-sha> -m "Unica vX.Y.Z"
git push origin vX.Y.Z
```

Consumers read the catalog from `main` and fetch only `./plugins/unica` at the
tag, so the tag only has to carry the plugin bytes — which the staging merge
already does. Tagging here rather than after the promotion pull request is what
keeps that pull request green from the start.

Verify the tag resolves to the payload:

```bash
gh api "repos/IngvarConsulting/unica-marketplace/contents/plugins/unica/.codex-plugin/plugin.json?ref=vX.Y.Z" \
  --jq '.content' | base64 -d | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])'
```

## Step 5 — promote

```bash
gh workflow run publish-unica-marketplace.yml --repo IngvarConsulting/unica \
  -f mode=promote \
  -f source_run_id=<run-id-from-step-2> \
  -f release_tag=vX.Y.Z \
  -f staging_merge_sha=<staging-merge-sha>
```

This opens a pull request that changes only the catalog `ref`. Its checks
install the plugin the way a consumer does — fresh install and upgrade from the
previous stable, on macOS, Linux, and Windows. With the tag already pushed they
pass on the first run.

## Step 6 — merge and verify

```bash
gh pr merge <promotion-pr> --repo IngvarConsulting/unica-marketplace --merge
gh api repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json \
  --jq '.content' | base64 -d | grep '"ref"'
```

The release is live once the catalog names the new tag. Claude Code consumers
read `.claude-plugin/marketplace.json`; check that entry too once it exists.

## What consumers see, and when

Only one step changes anything for consumers. Everything before it is invisible
to them, which is what makes aborting cheap.

| After step | Visible to consumers |
| --- | --- |
| 0 version prepared | nothing |
| 1 source tag pushed | nothing |
| 2 assets published | nothing — no catalog names them |
| 3 staging merged | nothing — the catalog still names the previous tag |
| 4 marketplace tag pushed | nothing |
| 5 promotion pull request open | nothing |
| **6 promotion merged** | **the release is live** |

So this is not a distributed transaction that needs compensating steps. There is
a single commit point, and before it "abort" means "stop and clean up".

## One-way doors

Two things can never be taken back once published, because other artifacts
reference them by identity:

- **Release assets** in `unica`. Runtime manifests pin them by SHA-256.
- **Tags** in either repository. Consumers resolve `git-subdir` against them.

This gives the rule that replaces rollback: **never reuse a version number**. If
anything is wrong after step 1, abandon that version and release the next patch
instead. Re-cutting `vX.Y.Z` with different bytes breaks every consumer that
already resolved it.

## Aborting

| Abort after | What to do | Cost |
| --- | --- | --- |
| 0 | Close the version pull request | none |
| 1–2 | Leave the tag and assets in place, abandon the version, bump to the next patch | a burnt version number |
| 3 staging pull request open | Close it | none |
| 4 staging merged | Nothing is served. Either continue, or revert the staging commit on the marketplace default branch and abandon the version | none |
| 5 marketplace tag pushed | Leave the tag, abandon the version. An unused tag is harmless | a burnt version number |
| 6 promotion pull request open | Close it. The catalog is untouched | none |

Never delete a tag to "clean up" an abandoned version. An unused tag costs
nothing; a deleted one that something already resolved costs every consumer.

## Rolling back a live release

Reverting is a one-file change, and it works because published bytes never move,
so the previous tag still resolves to exactly what it always did.

```bash
git clone https://github.com/IngvarConsulting/unica-marketplace.git /tmp/unica-marketplace
cd /tmp/unica-marketplace
git revert --no-edit <promotion-merge-sha>
git push origin main   # or open a pull request if the branch is protected
```

Confirm the catalog names the previous tag again, then treat the bad version as
burnt and fix forward in the next patch. Consumers move back on their next
update; those who already installed the bad version keep it until then, so
prefer fixing forward when the fault is not severe.

## The one state to avoid

A catalog that names a tag which does not exist. Every install then fails with
`pathspec 'vX.Y.Z' did not match any file(s)`, including for consumers who had
been working fine.

It has only two causes, both preventable:

- deleting or moving a published tag;
- merging a promotion pull request whose checks are red, since the consumer
  install checks are exactly what proves the ref resolves.

Protecting tags in the marketplace repository removes the first cause outright.

## Failure modes

| Symptom | Cause | Action |
| --- | --- | --- |
| Promotion checks fail with `pathspec 'vX.Y.Z' did not match any file(s)` | The marketplace tag is missing | Do step 4, then `gh run rerun <run-id> --failed` |
| `regression-policy` reports `consumer-fresh-install is required but concluded failure` | Aggregate gate reflecting the row above | Same as above |
| Packaging fails with `release tag ... != ...` | Step 0 missed the workflow `RELEASE_TAG` fallback | Bump it and re-run |
| `probe-thin-bootstrap` fails with `unexpectedly downloaded` | Pre-2026-07 behaviour, fixed by neutralising the checksum in the probe | Rebase onto a branch that contains the fix |
| Consumers still report the old version | Promotion was never merged | Check for an open promotion pull request |

## Never

- Move or delete a published tag, or force-push the marketplace default branch.
  Consumers resolve `git-subdir` against those refs; changed bytes require a new
  version.
- Merge the promotion pull request before its checks are green. Red here means
  the consumer install path is broken, not that the checks are wrong.
