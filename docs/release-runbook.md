# Release runbook

How a Unica version reaches the public marketplace, and why each step exists.
Every release is built from the previous checked stage and started by a button
or the schedule; this runbook says what each stage checks and what to do when
one fails.

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
   the previous tag, so no consumer is affected yet. The staging commit gets
   the **candidate anchor** `candidate-vX`, an immutable tag the install checks
   resolve.
2. **Promote** — commit the channel's catalogs naming `vX` and push that
   commit together with the release tag `vX` in one atomic push. This is the
   moment the release goes live.

The release tag `vX` in the marketplace is a snapshot of the published
catalog: the catalogs inside it name `vX`, so
`codex plugin marketplace add IngvarConsulting/unica-marketplace --ref vX`
installs exactly X. A stable release's tag names the promotion commit of
`main`, a candidate's the promotion commit of `next`. The catalogs pin
`git-subdir` to that tag, which `scripts/verify_marketplace.py` in the
marketplace repo enforces. The
[tag rule](../arch/rules/distribution/marketplace-release-tag.md) states the
guarantee. Tags `v0.9.1` through `v0.13.0-rc.5` were created on the staging
commit and name the previous release's catalog; they stay as they are. Earlier
tags, `v0.7.2` through `v0.8.1`, name their own version.

## Four stages, each built from the one before

Every release is built only from the previous checked stage, and nobody sets a
`v*` tag by hand: the nightly run and three buttons set them with a deploy key.

| Release | Source | Started by | Result |
| --- | --- | --- | --- |
| alpha `vX.Y.0-alpha.N` | a green night on `main` | the schedule | a prerelease on GitHub |
| RC `vX.Y.0-rc.N` | the alpha's tag | the RC button, an administrator | the tag set by the key, published on `next` |
| final `vX.Y.0` | the RC's tag, with a successful release | the final button, an administrator | the tag set by the key, published in the main catalog, the branch `release-vX.Y` on the final commit |
| patch `vX.Y.Z` | the head of `release-vX.Y`, with green CI | the patch button, an administrator | the patch number raised, the tag set by the key, published in the main catalog at once, a pull request back to `main` |

```bash
gh workflow run rc-from-alpha.yml --repo IngvarConsulting/unica --ref main -f alpha=v0.15.0-alpha.4
gh workflow run release-from-rc.yml --repo IngvarConsulting/unica --ref main -f rc=v0.15.0-rc.2
gh workflow run release-from-line.yml --repo IngvarConsulting/unica --ref main -f line=release-v0.15
```

Each tag push starts `.github/workflows/release.yml`, which runs the build and
the whole publication as one run, with no approval on the way:

| Step | The pipeline |
| --- | --- |
| tag pushed | build → assets → BSP assessment |
| | stage the payload (catalog untouched) |
| | create the candidate anchor `candidate-vX` on the staging commit |
| | consumer install checks from `candidate-vX`: fresh + upgrade, three hosts |
| | green → the channel's catalogs and the tag `vX`, one atomic push → **live** |

### Who can press a button

Only an administrator gets past the first job of any button, `admit`: it
refuses a run not started from `main`, reads the role of the person who started
or re-ran the run (`collaborators/<actor>/permission`) and refuses anything but
`admin`. No name is written in the workflows. A writer can press a button, but
the run stops there, before the job that holds the tag key starts; a re-run of
the failed job checks the role again before its key step.

The key job runs in the environment `alpha`, which admits only `main`, so a
copy of a button kept in an old tag gets no key. The buttons share one queue.
The tag ruleset lets only **Deploy keys** create `v*` tags: an administrator's
own push is refused too, so the buttons are the only way a release starts. The
tags are set by a workflow, not signed by a person: the administrator's button
and the checks below stand in for the signature.

### What each button checks

`scripts/ci/release-rc.py` refuses a candidate unless:

- the alpha is published and its slice — the parent of its version commit — is
  in `main` and equals the `main@<sha>` of its release description;
- the night that set this alpha is green and checked that very commit, so the
  slice's tree is the tree it tested;
- `vX.Y.Z` is not released yet.

It commits `bump-version.py X.Y.Z-rc.N` on the slice, with `N` the next free
number, tags the commit and pushes the tag.

`scripts/ci/release-final.py` refuses a final version unless:

- the candidate's tag exists and its GitHub release is published;
- the release run of that tag's push (`release.yml`) is green;
- the marketplace has the candidate's tag, which appears only together with
  the `next` catalog that names it;
- no later candidate of the same core exists;
- neither `vX.Y.Z` nor the branch `release-vX.Y` exists yet.

It commits `bump-version.py X.Y.Z` on the candidate's commit — the tree that
was tested and served on `next`, with only the version changed — and one
atomic push sets the tag and creates `release-vX.Y` on that commit.

`scripts/ci/release-patch.py` refuses a patch unless:

- the source is a branch `release-vX.Y` whose final version exists;
- its head is not released yet: it is neither the commit of a `vX.Y.*` tag nor
  the slice under one's version commit;
- the head has a green CI run: the line's merge queue (`ci.yml`) or a check of
  the line by the `release.yml` button.

It commits `bump-version.py X.Y.Z` on the head, with `Z` the next free patch,
and pushes only the tag: the version commit lives under the tag, and the line's
head stays a slice. The release publishes the patch to the main catalog at once.
The same run then opens the pull request back to `main`, described below.

### Which revision runs the pipeline

The tag push runs `release.yml` from the tagged commit, marketplace jobs
included. A change to the publication order therefore reaches the first release
tagged after it; a rerun of an old release repeats the old order. What does not
come from the tag is the channel: every marketplace job checks out only
`scripts/ci/release-channel.py`, sparsely, from `main`, and nothing in those
jobs checks out the tag's code. `UNICA_MARKETPLACE_TOKEN` lives in the
`marketplace` environment, which admits only `main` and `v*` tags, so another
branch cannot read it. The environment holds no tag key and has no reviewer:
the publication from a tag runs to the end without waiting for a person. The
jobs holding the token still come from the tagged commit's `release.yml`, so
whoever can create a `v*` tag defines those steps. Old tags also run against
the current `release-channel.py` from `main`, so keep its commands compatible
with them.

A tag's line — where the site files its report — is the `release-vX.Y` that
holds its commit; a final version holds its own line from the start. A button's
version commit lies on no branch, so the commit beneath it names the line, and
only the line the tag's name allows: `main` for an alpha or a candidate, its own
`release-vX.Y` for a patch. Anything else is refused
(`scripts/ci/resolve-line.py`).

The site job deploys through the `github-pages` environment, which must allow
`main`, `release-v*` and `v*`: a ref it refuses ends the run red after the release.

There is no scheduler and no waiting window: a failed stage is a red run
attached to the release tag, and the catalog stays where it was. Rerunning the
failed jobs resumes the publication — every stage is idempotent.

## Where a release is cut from

A minor is cut from `main`: the night makes an alpha of a `main` slice, the RC
button a candidate of that slice, and the final button the release of that
candidate. The final button also opens the line `release-vX.Y` on the release
commit.

Patches for a minor that already shipped are cut from its line. A fix reaches
`release-vX.Y` through a pull request and the merge queue, as it reaches `main`;
the line's queue run is the green CI the patch button asks for. `main` keeps its
own version and never takes the patch numbers.

## Raising the version of `main`

After a final release the night publishes no alpha of that version and says so
("raise the version of main"), because an alpha of `X.Y.0` would be older than
the release. Raise `main` to the next minor through a pull request:

```bash
python3.12 scripts/dev/bump-version.py X.Y.Z
cargo update --workspace --offline
```

The version lives in several files because each is read by a different consumer:
Cargo compiles it into the binaries, the three host manifests ship it to Codex,
Claude Code and ZCode, and the tools lock pins it beside the third-party tools.
They are separate artifacts, so it cannot live in one file — but it is written
by one command and enforced by one check, `scripts/ci/check-version-contract.py`,
which fails the build if any of them drift apart.

Tests that assert the current version still need updating by hand; they fail with
an explicit diff, so run the suite before opening the pull request:

```bash
python3.12 -m pip install -r tests/ci/requirements.txt
python3.12 -m unittest discover -s tests/ci
```

The buttons need no such pull request: each writes its version on its own
commit under the tag.

### Check a line before a patch

Some contracts run only on the tag build and in the `large` profile: the
Windows tool contracts in `scripts/ci/check-tool-contracts.py` and the
Windows test lane. A pull request does not run them, and the nightly checks
`main` only. A check that first fails on the tag build burns the version
number, and the fix ships under the next one — that is how `v0.13.0-rc.6` was
lost to a Windows-only `make` contract. Before a patch, run `large` on the line
and wait for it to finish green; the patch button accepts that run as the
line's CI:

```bash
gh workflow run release.yml --ref release-vX.Y -f profile=large
```

A release line carries its own `release.yml`. A line without it has no workflow
that publishes: bring the workflow onto the line before a patch.

## Watching a release

The tag push runs **Unica Release**: build, then stage → anchor → verify →
promote, then the site. The site updates only from these runs and the nightly.

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
gh run list --workflow release.yml --limit 3 \
  --json databaseId,headBranch,conclusion
```

The release is live when the catalog names the new tag — both host catalogs
move in the same commit — and the tag carries that catalog. Read the catalog
of the channel branch (`main`, or `?ref=next` for a candidate) and of the tag:

```bash
gh api 'repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json?ref=main' \
  --jq '.content' | base64 -d | grep '"ref"'
gh api 'repos/IngvarConsulting/unica-marketplace/contents/.agents/plugins/marketplace.json?ref=vX.Y.Z' \
  --jq '.content' | base64 -d | grep '"ref"'
```

Both must print `vX.Y.Z`. The marketplace repository runs its own **Verify
marketplace** on every push of a branch or a `v*` tag it receives, so `stage`
and `promote` leave runs there, after the fact; the `candidate-` anchor does
not match `v*` and starts none. The tag of a stable release now names a commit
whose catalog matches its plugin, so the run on that tag also performs a fresh
install from it. Those runs do not gate anything: the
pipeline's `verify-upgrade` and `verify-fresh-install` jobs already ran on
three hosts before `promote` moved the catalog.

## Bringing the line back into `main`

A patch leaves `main` behind, because the fix landed on the line only. After the
patch tag is pushed, the job `back-merge` of the patch button merges the line
into a branch `merge/release-vX.Y.Z-into-main` cut from `main` and opens a pull
request into `main`.

The version contract differs every time. In all five of its files and in
`Cargo.lock`, `main`'s side stays: the line changed nothing there but the
number, and `main`'s `Cargo.lock` carries the real dependency state. A conflict
in any other file stops the job with the list of files; merge the line by hand
on that branch, keeping `main`'s side in those six files. A line already in
`main`, or one that differs only in the version, opens nothing. The run's token
cannot push a change to `.github/workflows`, so a line that carries one is also
merged by hand. A rerun finds the branch of an earlier run and leaves it, with
its pull request, as it is.

The run's token creates that pull request, so its CI does not start on its own:
the coordinator closes and reopens it with their own token, or pushes an empty
commit.

## When a stage fails

| Stage | What failed | What to do |
| --- | --- | --- |
| alpha | the night is red | Fix `main`; the next green night releases the alpha. `gh workflow run nightly.yml --ref main -f force=true` repeats a night on the same tip |
| alpha | the night is green but releases nothing | Either `main` did not move, or its version is already released: raise the version of `main` |
| RC | the button refuses | Read the refusal: an unpublished alpha, a slice outside `main`, a night that did not check it, or a released version. Take another alpha, or raise `main` |
| RC | the release of `vX.Y.0-rc.N` is red | Rerun the failed jobs (below). A defect in the candidate's code burns its number: fix `main`, wait for the next alpha, press the RC button again |
| final | the button refuses | The candidate's release is not green, not on `next`, or not the newest candidate of its version, or the line already exists. Finish or replace the candidate first |
| final | the release of `vX.Y.0` is red | Rerun the failed jobs. A defect in the code burns the number; `release-vX.Y` already exists, so the fix ships as a patch from the line |
| patch | the button refuses | The source is not a `release-*` branch with a final version, the head is already released, or it has no green CI: merge the fix through the line's queue, or run the `large` check of the line |
| patch | the release of `vX.Y.Z` is red | Rerun the failed jobs. A defect in the code burns the number: fix the line and press the patch button again |
| patch | `back-merge` fails | A conflict outside the version files, or a line that changes `.github/workflows`: merge the line by hand on `merge/release-vX.Y.Z-into-main`. The patch itself is already released |

A tag push that started nothing at all — a red `admit` or a refused key job —
changed nothing: press the button again once the cause is gone.

### Rerunning a release run

The pipeline stops before the catalog moves, so consumers are unaffected.
Rerun the failed jobs after fixing the cause — completed stages detect
themselves and pass through:

- `stage` finds the payload already on the branch and reports that commit.
- `anchor` finds `candidate-vX` and only checks that its `plugins/unica` tree
  is the staged one.
- `promote` pushes the branches and the tag `vX` atomically, so a refused push
  moved nothing and a rerun starts over. A branch that already serves `vX` is
  left alone. An existing tag `vX` is never moved: its `plugins/unica` tree
  must equal the staged one, and then only the branches that do not yet serve
  `vX` are pushed. A different tree means a burnt version.
- A rerun of a release the old pipeline published (its `vX` on the staging
  commit, its catalog already moved) finds no anchor and may place
  `candidate-vX` on the promotion commit: `stage` reports the branch head. The
  `plugins/unica` tree is the same, so the anchor still names the verified
  bytes.

```bash
gh run rerun <release-run-id> --failed
```

A rerun uses the tag's own workflow and the payload that run built. A defect in
the publication steps themselves needs a new tag, as does a run GitHub no longer
reruns.

## What consumers see, and when

Only one step changes anything for consumers. Everything before it is invisible
to them, which is what makes aborting cheap.

| After | Visible to consumers |
| --- | --- |
| source tag pushed | nothing |
| assets published | nothing — no catalog names them |
| payload staged | nothing — the catalog still names the previous tag |
| candidate anchor pushed | nothing — no catalog names `candidate-vX` |
| install checks green | nothing |
| **catalog and tag `vX` pushed** | **the release is live** — for a candidate, only on the `next` channel |

Consumers choose what they follow when they add the marketplace:

- **A branch** — `--ref main` (Claude Code: no suffix) for stable releases,
  `--ref next` (`#next`) for candidates. The catalog follows the channel, and
  an update brings its latest release.
- **A tag** — `--ref vX.Y.Z` (Claude Code: `#vX.Y.Z`) installs exactly that
  version and stays on it. The catalog inside a candidate's tag is the `next`
  one, named `unica-next`, so its plugin is `unica@unica-next`. Tags
  `v0.9.1` through `v0.13.0-rc.5` carry the previous version's catalog and
  install it, not their own: install those versions by branch. A marketplace
  of the same name already added by branch is removed first
  (`marketplace remove`), then added by tag.

## A release candidate: served on `next` only

Some things can only be measured against a real release — a runtime manifest
pins its assets to `github.com/IngvarConsulting/unica/releases/download/<tag>/`,
so nothing but a published tag will do — and a candidate needs testers before
the release it becomes. A release candidate is published to the `next`
channel of the marketplace and never to the stable catalog.

A candidate is cut from a checked alpha by the RC button, and the final
version from a checked candidate by the final button; the
[four stages](#four-stages-each-built-from-the-one-before) say what each checks.
The candidate's version carries the suffix `-rc.N`.

The suffix is part of the version, not a label beside it, because the runtime
manifest requires the tag to equal `v` + the plugin version literally.

What the pipeline does, by the source tag:

| Tag | Assets | Stage, candidate anchor, install checks | Catalogs moved, tag `vX` |
| --- | --- | --- | --- |
| `vX.Y.Z` | published | on `main` | `next` unless it serves a newer candidate, then `main` |
| `vX.Y.Z-rc.N` | published, marked as a prerelease | on `next` | `next` only |
| `vX.Y.Z-alpha.N` | published with the full set of every OS, marked as a prerelease | skipped | none — [a slice of `main`](#an-alpha-a-slice-of-main) |
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
code" still means a second version commit, which the final button makes on the
candidate's commit — the version is part of the package contract, so `0.13.0`
is a commit, not a relabelling of `0.13.0-rc.1`.

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
- **Tags** in either repository, the marketplace's `candidate-vX` anchors
  included. Consumers resolve `git-subdir` against `vX`, and a rerun proves an
  existing tag or anchor carries the staged bytes instead of moving it.

This gives the rule that replaces rollback: **never reuse a version number**. If
anything is wrong after a tag is set, abandon that version and release the next
one instead. Re-cutting `vX.Y.Z` with different bytes breaks every consumer that
already resolved it.

Whenever you abandon a version whose assets are already published, mark that
release so it stops looking like a release waiting to be served:

```bash
gh release edit vX.Y.Z --repo IngvarConsulting/unica --prerelease
```

Never delete a tag to "clean up" an abandoned version; only alpha tags are
removed, by their own retention. An unused tag costs
nothing; a deleted one that something already resolved costs every consumer.

## An alpha: a slice of `main`

An alpha is a prerelease of the current `main` for checks and for consumers that
need a fresh build before a candidate. It never reaches the marketplace.

`scripts/ci/release-alpha.py publish` takes `X.Y.Z` from the version in `main`,
raises `N`, commits `bump-version.py` on top of the `main` tip and tags that
commit `vX.Y.Z-alpha.N`. The commit stays outside `main`, so the number in the
code equals the tag, the version contract holds, and `main` gets no release
commits. The commit's parent is the slice; the release description names it as
`main@<sha>`.

- **When.** **Unica Nightly** calls `release.yml` with profile `large`. After the
  tests pass on every OS, the job `alpha-tag` makes the commit and the tag, and
  the rest of that run builds and publishes them. The tests do not run again: the
  commit changes only the version. A `main` that did not move starts nothing.
- **The tag key.** The job pushes the tag over SSH with the deploy key from the
  secret `RELEASE_TAG_KEY` of the environment `alpha`, not with the run's token:
  `v*` tags are protected, and the environment admits only `main`. A push by the
  key wakes `on: push`, so `release.yml` does not listen to `v*-alpha.*` tags:
  the night that set the tag builds it.
- **By hand.** The same path, with no commits in `main`:

  ```bash
  gh workflow run nightly.yml --repo IngvarConsulting/unica --ref main -f force=true
  ```

  `force` runs the night on a tip that was already checked and releases a slice
  that already has an alpha, for example after a failed build.
- **The assets.** Besides the core pair, the release carries
  `unica-full-<target>.tar.gz` for every OS: the core and all engines laid out
  like the plugin package, so it needs no marketplace or download on first use.
- **The order.** By SemVer an alpha is older than a `-rc` of the same `X.Y.Z`, even
  when its content is newer; compare the slices, not the numbers.
- **After a full release.** Once `vX.Y.Z` exists, an alpha of that `X.Y.Z` would
  be older than the release, so the night stays green and releases no alpha until
  `main` raises the version.
- **Retention.** After a publication the run calls
  `release-alpha.py prune`: the two newest published alphas stay, and the GitHub
  releases of older ones are deleted. Their tags stay. The next number and the
  released slice are read from the tags, so `N` only grows, and a kept tag holds
  no assets. Nothing deletes or rewrites a `v*` tag, so the tag ruleset needs no
  exclusion for alphas.
- **Tag ruleset.** One rule covers every `v*` tag, alphas included: deletion and
  non-fast-forward updates are blocked for everyone, and creation is restricted
  with only **Deploy keys** in the bypass list — no role, the administrator's
  included. The deploy key from `RELEASE_TAG_KEY` lives only in the environment
  `alpha`, which admits only `main`, and creates every `v*` tag there: the alpha,
  candidate, final and patch ones, and the branch `release-vX.Y` with a final.
  The app GitHub Actions gets no bypass, so a workflow run from any branch cannot
  create a `v*` tag with the run's token.
- **Immutable releases.** Assets can be uploaded only before publication, so every
  prerelease goes up as a draft and a separate step publishes it. A failed run
  before that step can be rerun: it continues the draft. After publication a
  failure means a new number.

The page `alpha.html` on the site shows the latest alpha. It is given out as a
direct link: nothing on the site links to it and it is closed to search robots.

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
prefer fixing forward when the fault is not severe. The tag `vX` stays where
it is: a consumer who added the marketplace with `--ref vX` asked for exactly
that version and keeps resolving it.

## The one state to avoid

A catalog that names a tag which does not exist. Every install then fails with
`pathspec 'vX.Y.Z' did not match any file(s)`, including for consumers who had
been working fine.

The pipeline cannot reach it — the catalog and the tag it names leave in one
atomic push — so it has one remaining cause, which is preventable outright by
protecting tags in the marketplace repository: deleting or moving a published
tag by hand.

## Failure modes

| Symptom | Cause | Action |
| --- | --- | --- |
| Publish run failed at `stage`, `anchor` or `promote` on a push | Transient push failure or a moved branch; a refused atomic push in `promote` moved nothing | `gh run rerun <run-id> --failed`; stages are idempotent |
| Publish run failed at the install checks | The candidate does not install as a consumer | Fix forward; the version is burnt, the catalog never moved |
| `stage` fails with `vX.Y.Z-rc.N is older than vA.B.C` | A newer release or candidate is already served | The candidate is stale; tag the next `-rc.N` of a newer version |
| `anchor` or `promote` fails comparing an existing `vX` or `candidate-vX` | The version was already published with different bytes | Never move the tag; release the next patch |
| `--ref vX` installs the previous version | `vX` is one of `v0.9.1` through `v0.13.0-rc.5`: those tags name the staging commit, whose catalog still named the previous release | Expected for those tags; add the marketplace by branch |
| Packaging fails with `release tag vX.Y.Z != vA.B.C` | The tagged commit does not declare X.Y.Z: the button's version commit and its tag disagree | A defect in the button script; that version is burnt — fix the script and take the next number |
| `Verify marketplace` red after the catalog moved, at `previous-stable-upgrade` with `git clone marketplace source timed out after 30s` | Codex re-clones the whole marketplace on `plugin marketplace upgrade`; a clone that misses its own 30s budget leaves the stale local catalog, which then installs the previous version | Transient. `gh run rerun <run-id> --failed`. Consumers are unaffected: the catalog is already correct and the pipeline's own upgrade checks passed before promote |
| Consumers still report the old version | The publish run did not finish | Check its failed stage and rerun |
| `verify-delivery-reachable` fails with HTTP 404 | The toolchain asset the lock names is gone or was renamed | Never re-tag a toolchain release; point the lock at a published asset and cut the next patch |
| The prefetch step fails on a checksum | Toolchain bytes were replaced under a published name | Treat the toolchain release as burnt, publish a new toolchain build, bump the lock |
| `build-tools` fails while downloading a tool asset | The lock names a toolchain tag that does not exist yet | Publish the toolchain release first; the lock may only pin what is already public |

## Never

- Move or delete a published tag other than an expired alpha, or force-push `main` or `next` of the
  marketplace. Consumers resolve `git-subdir` against those refs; changed bytes
  require a new version.
- Set a `v*` tag by hand; the tag ruleset refuses it, and a release starts only
  from a button or the night.
- Point a catalog at a tag by hand, or point `main` at a candidate. The promote
  job is the only writer of both channels' catalog files, and it runs only
  behind green install checks.
