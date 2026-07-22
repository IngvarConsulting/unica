# Local 1Ci 8.3.27 Guides Corpus

## Goal

Provide a reproducible, private local Markdown corpus for development work from
the English 1C:Enterprise 8.3.27 client/server administrator, file-mode
administrator, and developer guides on `kb.1ci.com`.

The downloaded material is for local analysis only. It must not be committed,
packaged, or published with Unica.

## Sources

The downloader owns exactly these source subtrees:

1. `1C_Enterprise_8.3.27_Administrator_Guide._Client_Server_Mode`
2. `1C_Enterprise_8.3.27_Administrator_Guide`
3. `1C_Enterprise_8.3.27_Developer_Guide`

Only English pages are selected. URLs with and without `language=en` are the
same logical page and must be deduplicated.

## Repository Boundary

Tracked files:

- the downloader and its tests;
- the source-root configuration embedded in or adjacent to the downloader;
- `.gitignore` and `AGENTS.md` rules for the corpus.

Ignored local output:

```text
docs-local/1ci/8.3.27/en/
├── administrator-client-server/
├── administrator-file-mode/
└── developer/
```

This path is deliberately separate from the removed repository `docs/` tree
and from `plugins/unica/references/`. Packaging scripts must not consume it.

## Discovery

Use a hybrid discovery model:

1. Fetch and respect `robots.txt`.
2. Read `sitemap.xml` and select English pages below the three configured roots.
3. Parse every selected page for child-page links and attachments.
4. Add a discovered page only when its normalized URL remains below the same
   configured root. Never follow XWiki edit, login, REST, skin, resource, or
   other service URLs.
5. Download all page attachments and content images, including PDFs, archives,
   examples, and other file types. External web links remain external.

The sitemap provides breadth; in-page discovery detects sitemap omissions and
collects files that the sitemap does not enumerate.

## Conversion and Layout

Each normalized page URL maps deterministically to a directory that mirrors its
path below the guide root. Page content is stored as `index.md`. Binary content
is stored in that page's `_assets/` directory using a collision-safe normalized
name.

Conversion retains headings, paragraphs, lists, tables, code/preformatted
blocks, notes, links, and meaningful image alternative text. Site chrome,
navigation, account controls, history controls, and footer content are removed.
Internal page and asset links are rewritten to relative local paths. A source
URL and retrieval timestamp are included as compact front matter so every page
remains traceable.

## Manifest and Refresh

The corpus root contains:

- `README.md`, listing the three guides and refresh command;
- `manifest.json`, recording source URL, normalized URL, local path, content
  hash, retrieval time, relevant HTTP metadata, and status for every page and
  attachment.

Downloads use bounded concurrency or a conservative request interval, a clear
user agent, timeouts, and retry/backoff for transient failures. Files are
written atomically. An interrupted or partially failed run must leave the last
complete corpus usable.

A successful refresh replaces the manifest and may remove files no longer
referenced by the newly complete manifest. A failed refresh reports errors and
does not perform stale-file cleanup. Unchanged content is not rewritten.

## Agent Workflow

`AGENTS.md` instructs agents working on 1C platform behavior to search the
local corpus first. If the corpus, required guide, or complete manifest is
missing, they run the downloader and retry the local search. Ordinary repository
exploration continues to exclude this ignored corpus; it is searched only when
the task needs official platform documentation.

## Verification

Automated tests cover:

- root-boundary checks and rejection of lookalike prefixes;
- English URL normalization and deduplication;
- deterministic page and asset paths;
- HTML content extraction and Markdown conversion fixtures;
- relative link rewriting;
- manifest replacement and failed-refresh preservation;
- robots exclusions.

End-to-end verification runs a bounded sample download before the complete
download. Completion requires all three roots in the manifest, no failed
records, no broken local links, no tracked corpus files, and confirmation that
the package inputs do not include `docs-local/`.

## Non-goals

- Publishing or redistributing 1Ci documentation.
- Downloading other product versions or languages.
- General-purpose website mirroring.
- Adding the corpus to prompt-visible plugin skills or the public MCP package.
