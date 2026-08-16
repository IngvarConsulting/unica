---
name: release
description: Publish a Unica version to the public marketplace, or resume a release that stalled part-way. Use when asked to cut, ship, promote, or finish a release, or when consumers still see an older version than the latest tag.
argument-hint: [version, for example v0.9.2]
---

# Release Unica

Follow [docs/release-runbook.md](../../../docs/release-runbook.md). It is the
authoritative procedure; do not improvise an order.

## One human action, one pipeline

The user's part is a single signed source tag. Its push builds the release, and
the successful build starts **Publish Unica Marketplace** on its own:
stage → tag → verify → promote, with the catalog moving only behind green
consumer install checks (ADR-0068). There are no pull requests to merge and no
scheduler to wait for.

## Before anything

Establish where the release already is, because most requests are to *resume*
one rather than start one. Check in this order and enter the runbook at the
first unfinished step:

```bash
version=vX.Y.Z   # the version the user named, if they named one
gh release view "$version" --repo IngvarConsulting/unica --json tagName,isPrerelease
gh run list --workflow "Publish Unica Marketplace" --repo IngvarConsulting/unica --limit 3 \
  --json databaseId,status,conclusion
gh api "repos/IngvarConsulting/unica-marketplace/git/ref/tags/$version" || echo "no marketplace tag yet"
gh api repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json \
  --jq '.content' | base64 -d | grep '"ref"'
```

If the user named a version, act on that one. Without a named version, take the
newest release that is not a prerelease.

A source release newer than the catalog `ref` with no publish run in flight
means the pipeline did not finish: find its failed run and rerun it — every
stage is idempotent, a rerun resumes the publication:

```bash
gh run rerun <publish-run-id> --failed
```

If the `workflow_run` trigger was missed entirely, dispatch the pipeline with
the build's run id:

```bash
gh workflow run publish-unica-marketplace.yml --repo IngvarConsulting/unica \
  -f source_run_id=<build-run-id>
```

## When something goes wrong

Only the catalog commit is visible to consumers, so nothing before it needs
undoing on their behalf. A pipeline that failed before promote left the catalog
untouched; fix the cause and rerun, or abandon the version.

Once assets are published the version is burnt. Never re-cut it with different
bytes; take the next patch, and mark the abandoned release a prerelease.
Rolling back a live release is a revert of the promotion commit, which is safe
because published bytes never move.

## Rules

- Never move or delete a published tag, and never force-push the marketplace
  default branch. A catalog naming a missing tag breaks every install, and it is
  the only genuinely corrupt state this process can reach.
- Never point the catalog at a tag by hand: the promote job is the catalog's
  only writer, and it runs only behind green install checks.
- Stop and ask before creating the source tag or dispatching the publish
  pipeline by hand. Those are the publishing steps and they are the user's call.
- Signing is the user's: the key is theirs and the passphrase must not be handled
  for them. If `gpg` fails with `Operation cancelled`, tell them to unlock it and
  give them the exact tag command to run.
- Report each step's verification output rather than asserting success.
