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

One commit bumps every place the version appears. `check-version-contract.py`
covers most of them, but not all, so bump all of these together:

| File | What changes |
| --- | --- |
| `Cargo.toml` | `workspace.package.version` (then refresh `Cargo.lock`) |
| `plugins/unica/.codex-plugin/plugin.json` | `version` |
| `plugins/unica/.claude-plugin/plugin.json` | `version` |
| `plugins/unica/third-party/tools.lock.json` | the `unica` tool entry `version` |
| `.github/workflows/unica-plugin-release.yml` | the `RELEASE_TAG` fallback used by non-tag builds |

The workflow fallback is **not** covered by the version contract. If it is left
behind, packaging fails on every later pull request with
`release tag vX.Y.Z != vA.B.C`, because the runtime manifest requires
`release.tag == v{pluginVersion}`.

Verify, then merge through a pull request as usual:

```bash
python3.12 scripts/ci/check-version-contract.py
```

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
