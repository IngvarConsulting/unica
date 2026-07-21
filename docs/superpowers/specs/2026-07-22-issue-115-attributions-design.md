# Issue 115: Manually Maintained Attribution Page

## Goal

Give plugin users one packaged, readable page that identifies the authors and
repositories behind Unica's shipped tools, external service adapters, and
adapted skill behavior; thanks those authors; and explains the applicable
license chain.

The page is a manually written editorial document. Automated checks protect
its inventory coverage, while human review remains responsible for the
accuracy and quality of its prose.

## Scope

The attribution page covers the public Unica plugin artifact:

- Unica itself;
- every bundled executable declared in `third-party/tools.lock.json`, excluding
  duplicate presentation of the Unica executable itself;
- every external service adapter declared in the packaged source manifest;
- every distinct upstream repository represented in
  `provenance/skill-upstreams.json` whose behavior, guidance, or ideas informed
  packaged skills.

Development-only dependencies, CI actions, test fixtures, build tools, and
transitive Rust or Python dependencies are outside this issue. Their license
obligations remain governed by their own dependency and distribution
mechanisms.

## Source and ownership model

Existing package metadata remains authoritative:

- `third-party/tools.lock.json` owns bundled tool identity, version, repository,
  pinned source revision, and license;
- `provenance/skill-upstreams.json` owns adapted-skill donor repositories and
  adaptation coverage;
- the packaged source manifest owns external service adapter identity and URL;
- `.codex-plugin/plugin.json` owns Unica author, homepage, and license identity.

`plugins/unica/ATTRIBUTIONS.md` owns the human-readable author names,
acknowledgements, explanations, and license-chain narrative. The design does
not introduce a new unified manifest or duplicate machine-readable version and
commit data in another JSON file.

Authorship and license values must be verified against the upstream repository
and then recorded explicitly. A repository owner inferred from its GitHub URL
is not sufficient evidence of authorship.

`ai_rules_1c` is inspiration only: Unica used ideas from that repository, not
copied or adapted material. Its baseline publishes no license, so the page must
not imply that it grants redistribution rights or forms part of Unica's
license chain. Provenance metadata must state this boundary explicitly.

The pinned `v8-runner` source is AGPL-3.0. The incorrect MIT declaration in
`third-party/tools.lock.json` must be corrected and the corresponding license
text must be packaged.

## Attribution page

Maintainers will write `plugins/unica/ATTRIBUTIONS.md` manually. It will
contain:

1. Unica copyright, homepage, and LGPL-3.0-or-later license;
2. bundled tools grouped by upstream project, with authors, repository, pinned
   version/revision, shipped role, upstream license, and included license or
   notice path;
3. external service adapters, with provider/project, endpoint or homepage,
   integration role, and a clear statement that the remote service is not
   redistributed as part of Unica;
4. adapted skill sources grouped by donor repository, with author links,
   affected packaged skills, upstream license, and an explanation that the
   behavior was adapted behind Unica's typed `unica.*` MCP boundary;
   inspiration-only sources are labelled separately and make no redistribution
   or adaptation claim;
5. a concise license-chain explanation and acknowledgements.

Repeated tools from one repository and repeated skill entries from one donor
will be grouped. The page will point maintainers to the authoritative package
metadata that must be consulted when it is edited.

The plugin README and the repository root README will link to the page. The
existing packaging process must copy it into the marketplace artifact without
special release-time network access.

## License-chain semantics

The page will describe relationships rather than claim that one license
automatically replaces another:

- original third-party code remains subject to its upstream license;
- Unica's own code and adaptations are distributed under
  LGPL-3.0-or-later;
- copied or modified material retains required upstream notices and license
  terms;
- separately invoked or aggregated binaries retain their declared licenses;
- remote services are referenced integrations, not redistributed components.
- inspiration-only sources contribute ideas rather than copied expression and
  do not enter the redistribution license chain.

Full upstream license texts continue to live under `third-party/licenses/`
when redistribution requires or warrants inclusion. The attribution page links
to those files; it does not duplicate complete license texts.

## Validation and failure behavior

An offline CI checker will compare scoped component identifiers from the
authoritative metadata with explicit machine-readable markers embedded in the
manually written Markdown. It will fail when:

- a scoped tool, adapter, or donor has no matching attribution entry;
- a required repository, author, or license URL is absent or is not an
  absolute HTTPS URL;
- a declared bundled-license path does not exist in the packaged tree;
- an attribution marker is duplicated or refers to a component outside the
  authoritative inventories.

Checks are offline and deterministic. They validate recorded URLs and paths,
not live network availability. Upstream facts are verified during metadata
changes and reviewed in source control.

CI cannot prove that free-form prose remains factually current. This is an
accepted consequence of choosing a manually authored page. Reviewers must
compare author, acknowledgement, and license wording with upstream sources
when components or their metadata change.

## Testing strategy

Implementation follows test-first development:

1. add failing unit/contract tests for complete source coverage, unique
   markers, URL validation, and license-path validation;
2. write the attribution page and add the smallest checker needed to pass the
   tests;
3. add README links;
4. run the focused attribution tests, provenance validation, packaging tests,
   and the relevant repository test suite.

The packaging test must inspect a produced plugin archive or staging directory
and prove that `ATTRIBUTIONS.md` and referenced packaged license files are
present.

## Non-goals

- replacing the existing provenance or tool-lock contracts;
- producing an SBOM for every transitive dependency;
- making CI query GitHub or other upstream hosts;
- generating or rewriting the attribution page from metadata;
- adding attribution prose to prompt-visible skills;
- changing the public MCP server or `unica.*` tool boundary.
