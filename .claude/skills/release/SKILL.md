---
name: release
description: Publish a Unica version to the public marketplace, or resume a release that stalled part-way. Use when asked to cut, ship, promote, or finish a release, or when consumers still see an older version than the latest tag.
argument-hint: [version, for example v0.9.2]
---

# Release Unica

Follow [docs/release-runbook.md](../../../docs/release-runbook.md). It is the
authoritative procedure; do not improvise an order.

## Most of it is automated

Release Warden merges the staging pull request, requests the promotion, and
merges the promotion, each once its checks are green. The user's part is two
tags: the source release tag and the marketplace tag. Before doing any of those
steps by hand, check whether the warden is simply about to do it:

```bash
gh workflow run release-warden.yml --repo IngvarConsulting/unica -f dry_run=true
gh run list --workflow "Release Warden" --repo IngvarConsulting/unica --limit 3
```

## Before anything

Establish where the release already is, because most requests are to *resume*
one rather than start one. Check in this order and enter the runbook at the
first unfinished step:

```bash
version=vX.Y.Z   # the version the user named, if they named one
gh release view "$version" --repo IngvarConsulting/unica --json tagName,isPrerelease
gh api "repos/IngvarConsulting/unica-marketplace/git/ref/tags/$version" || echo "no marketplace tag yet"
gh pr list --repo IngvarConsulting/unica-marketplace --state open
gh api repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json \
  --jq '.content' | base64 -d | grep '"ref"'
```

If the user named a version, act on that one and stop if the open pull requests
belong to a different version — resuming the wrong release is worse than asking.
Without a named version, take the newest release that is not a prerelease.

A source release whose tag is newer than the catalog `ref` means the release is
live for nobody. That is the common stall, and it is silent.

## When something goes wrong

Only the promotion merge is visible to consumers, so nothing before it needs
undoing on their behalf. What cleanup is required depends on how far the release
got, and the runbook's abort table is per step: closing a pull request is enough
only while one is open, whereas a merged staging commit may need reverting and a
published tag is left alone.

Once assets are published the version is burnt. Never re-cut it with different
bytes; take the next patch, and mark the abandoned release a prerelease so the
warden stops reporting it as stalled. Rolling back a live release is a revert of
the promotion commit, which is safe because published bytes never move.

## Rules

- Never move or delete a published tag, and never force-push the marketplace
  default branch. A catalog naming a missing tag breaks every install, and it is
  the only genuinely corrupt state this process can reach.
- Tag the staging merge commit, not the promotion commit. The promotion pull
  request cannot go green before that tag exists.
- Stop and ask before merging anything in `unica-marketplace` or creating a tag.
  Those are the publishing steps and they are the user's call.
- Signing is the user's: the key is theirs and the passphrase must not be handled
  for them. If `gpg` fails with `Operation cancelled`, tell them to unlock it and
  give them the exact tag command to run.
- Report each step's verification output rather than asserting success.
