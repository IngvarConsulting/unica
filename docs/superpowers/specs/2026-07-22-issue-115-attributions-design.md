# Issue 115: Packaged Attribution Page

## Goal

Give plugin users one packaged, readable page that identifies the authors and
repositories behind Unica's shipped tools, external service adapters, and
adapted skill behavior; thanks those authors; and explains the applicable
license chain.

The page must remain consistent with package-contract metadata. It must not
become a second manually maintained inventory.

## Scope

The attribution page covers the public Unica plugin artifact:

- Unica itself;
- every bundled executable declared in `third-party/tools.lock.json`, excluding
  duplicate presentation of the Unica executable itself;
- every external service adapter declared in the packaged source manifest;
- every distinct upstream repository represented in
  `provenance/skill-upstreams.json` whose behavior or guidance was adapted into
  packaged skills.

Development-only dependencies, CI actions, test fixtures, build tools, and
transitive Rust or Python dependencies are outside this issue. Their license
obligations remain governed by their own dependency and distribution
mechanisms.

## Source-of-truth model

Existing package metadata remains authoritative:

- `third-party/tools.lock.json` owns bundled tool identity, version, repository,
  pinned source revision, and license;
- `provenance/skill-upstreams.json` owns adapted-skill donor repositories and
  adaptation coverage;
- the packaged source manifest owns external service adapter identity and URL;
- `.codex-plugin/plugin.json` owns Unica author, homepage, and license identity.

Those records will gain only the attribution fields that cannot be derived
reliably, such as a human-readable project/author name, author URL,
acknowledgement text, and an upstream license reference. The design does not
introduce a new unified manifest or duplicate version and commit data.

Authorship and license values must be verified against the upstream repository
and then recorded explicitly. A repository owner inferred from its GitHub URL
is not sufficient evidence of authorship.

## Generated page

A deterministic repository script will render
`plugins/unica/ATTRIBUTIONS.md`. The generated page will contain:

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
5. a concise license-chain explanation and acknowledgements.

Repeated tools from one repository and repeated skill entries from one donor
will be grouped. Lists and groups will use stable sorting so regeneration is
reproducible. The page will label itself as generated and point maintainers to
the authoritative metadata and generator.

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

Full upstream license texts continue to live under `third-party/licenses/`
when redistribution requires or warrants inclusion. The generated page links
to those files; it does not duplicate complete license texts.

## Validation and failure behavior

The generator has a check mode that renders in memory and fails when the
checked-in page differs. CI tests will also fail when:

- a scoped tool, adapter, or donor lacks required attribution metadata;
- an attribution URL is absent or is not an absolute HTTPS URL;
- a declared bundled-license path does not exist in the packaged tree;
- a tool or donor disappears from the generated page;
- output ordering is nondeterministic.

Checks are offline and deterministic. They validate recorded URLs and paths,
not live network availability. Upstream facts are verified during metadata
changes and reviewed in source control.

## Testing strategy

Implementation follows test-first development:

1. add failing unit/contract tests for complete source coverage, grouping,
   deterministic rendering, URL/path validation, and stale-output detection;
2. add the smallest metadata extensions and generator needed to pass them;
3. generate the page and add README links;
4. run the focused attribution tests, provenance validation, packaging tests,
   and the relevant repository test suite.

The packaging test must inspect a produced plugin archive or staging directory
and prove that `ATTRIBUTIONS.md` and referenced packaged license files are
present.

## Non-goals

- replacing the existing provenance or tool-lock contracts;
- producing an SBOM for every transitive dependency;
- making CI query GitHub or other upstream hosts;
- adding attribution prose to prompt-visible skills;
- changing the public MCP server or `unica.*` tool boundary.

