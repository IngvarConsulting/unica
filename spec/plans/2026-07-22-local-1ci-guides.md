# Local 1Ci Guides Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and run a safe hierarchical downloader that creates an ignored English Markdown corpus of the three requested 1Ci 8.3.27 guides.

**Architecture:** A self-contained Python 3.12 module discovers pages from the sitemap, the paginated XWiki spaces catalog, and in-page links; extracts XWiki content with `html.parser`; downloads page assets; rewrites links; and atomically publishes a complete staged corpus. Unit tests exercise pure URL, path, conversion, robots, catalog discovery, and publication functions; a bounded live sample precedes the full run.

**Tech Stack:** Python 3.12 standard library, `unittest`, 1Ci XWiki sitemap/HTML endpoints.

## Global Constraints

- Output is English-only under `docs-local/1ci/8.3.27/en/`.
- Output is ignored, local-only, and never packaged or published.
- Discovery cannot escape the three exact configured URL roots.
- All page images and attachments are downloaded and linked locally.
- Failed refreshes preserve the last complete corpus.
- Requests respect `robots.txt` except for the explicitly approved
  `https://kb.1ci.com/bin/download/*` attachment path, and use timeouts,
  throttling, and retry/backoff.

---

### Task 1: URL and path contract

**Files:**
- Create: `scripts/dev/download-1ci-guides.py`
- Create: `tests/dev/test_download_1ci_guides.py`

**Interfaces:**
- Produces: `Guide`, `normalize_page_url(url)`, `guide_for_url(url)`, `page_output_path(guide, url)`.

- [x] **Step 1: Write failing normalization and boundary tests**

```python
def test_normalize_page_url_forces_english_and_drops_fragment(self):
    self.assertEqual(normalize_page_url(ROOT + "Chapter_1/#x", ROOT), ROOT + "Chapter_1/?language=en")

def test_lookalike_root_is_rejected(self):
    self.assertIsNone(guide_for_url(DEVELOPER_ROOT + "_Other/"))
```

- [x] **Step 2: Run the focused tests**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides -v`
Expected: FAIL because the module does not exist.

- [x] **Step 3: Implement exact-root URL normalization and deterministic safe paths**

```python
@dataclass(frozen=True)
class Guide:
    name: str
    root: str

def guide_for_url(url: str) -> Guide | None:
    path = urlsplit(url).path
    return next((g for g in GUIDES if path == urlsplit(g.root).path or path.startswith(urlsplit(g.root).path + "/")), None)
```

- [x] **Step 4: Run tests and commit**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides -v`
Expected: PASS.

### Task 2: XWiki extraction and Markdown conversion

**Files:**
- Modify: `scripts/dev/download-1ci-guides.py`
- Modify: `tests/dev/test_download_1ci_guides.py`

**Interfaces:**
- Produces: `extract_page(html, page_url) -> ExtractedPage` containing title, Markdown, page links, and asset URLs.

- [x] **Step 1: Add failing fixture-based tests**

```python
def test_extracts_only_xwiki_content_and_rewrites_links(self):
    page = extract_page(FIXTURE_HTML, DEVELOPER_ROOT)
    self.assertIn("# Chapter 1", page.markdown)
    self.assertIn("| Name | Value |", page.markdown)
    self.assertNotIn("Sign in", page.markdown)
    self.assertEqual(page.assets, (urljoin(DEVELOPER_ROOT, "download/example.png"),))
```

- [x] **Step 2: Verify RED**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides.ExtractionTests -v`
Expected: FAIL because extraction is missing.

- [x] **Step 3: Implement an `HTMLParser` content tree and Markdown renderer**

```python
class ContentParser(HTMLParser):
    """Capture only the element with id=xwikicontent and its descendants."""

def extract_page(html: str, page_url: str) -> ExtractedPage:
    parser = ContentParser(page_url)
    parser.feed(html)
    return parser.result()
```

Renderer rules explicitly cover headings, paragraphs, emphasis, lists, tables,
`pre`/`code`, notes, links, images, and whitespace normalization.

- [x] **Step 4: Run tests and commit**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides -v`
Expected: PASS.

### Task 3: Network discovery, robots, assets, and atomic publication

**Files:**
- Modify: `scripts/dev/download-1ci-guides.py`
- Modify: `tests/dev/test_download_1ci_guides.py`

**Interfaces:**
- Produces: `Downloader.run(max_pages: int | None = None) -> Manifest`, CLI flags `--output`, `--max-pages`, `--delay`, and `--check-links`.

- [x] **Step 1: Add failing mocked-network and transaction tests**

```python
def test_failed_refresh_preserves_complete_destination(self):
    destination.joinpath("manifest.json").write_text('{"complete": true}')
    with self.assertRaises(DownloadError):
        Downloader(fetcher=failing_fetcher, output=destination).run()
    self.assertEqual(json.loads(destination.joinpath("manifest.json").read_text())["complete"], True)
```

- [x] **Step 2: Verify RED**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides.DownloaderTests -v`
Expected: FAIL because `Downloader` is missing.

- [x] **Step 3: Implement sitemap-first discovery, page-link supplementation, asset download, retries, and staged replacement**

```python
with tempfile.TemporaryDirectory(dir=output.parent) as staging_name:
    manifest = crawl_into(Path(staging_name), max_pages=max_pages)
    if manifest.failures:
        raise DownloadError(manifest.failure_summary())
    publish_complete_staging(Path(staging_name), output)
```

Use `urllib.robotparser`, `urllib.request`, SHA-256 hashes, JSON with sorted keys,
three retries with exponential backoff, a 30-second timeout, and a default
0.25-second delay between requests.

- [x] **Step 4: Run tests and commit**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides -v`
Expected: PASS.

### Task 4: Repository and agent workflow

**Files:**
- Modify: `.gitignore`
- Modify: `AGENTS.md`
- Modify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Consumes: CLI created in Task 3.
- Produces: local-first agent rule and packaging/ignore regression contract.

- [x] **Step 1: Add failing contract assertions**

```python
def test_local_1ci_corpus_is_ignored_and_not_packaged(self):
    self.assertIn("docs-local/", (ROOT / ".gitignore").read_text())
    self.assertNotIn("docs-local", package_script_text)
```

- [x] **Step 2: Verify RED**

Run: `python3.12 -m unittest tests.ci.test_product_contracts -v`
Expected: FAIL because the ignore and workflow rule are absent.

- [x] **Step 3: Add `docs-local/` ignore and an AGENTS section requiring local search then downloader fallback**

```markdown
## Local 1Ci Platform Documentation

For official 1C platform behavior, search `docs-local/1ci/8.3.27/en/` first.
If the required guide or complete manifest is absent, run
`python3.12 scripts/dev/download-1ci-guides.py` and retry the local search.
```

- [x] **Step 4: Run contract and downloader tests, then commit**

Run: `python3.12 -m unittest tests.dev.test_download_1ci_guides tests.ci.test_product_contracts -v`
Expected: PASS.

### Task 5: Live corpus and completion verification

**Files:**
- Create locally ignored: `docs-local/1ci/8.3.27/en/**`

**Interfaces:**
- Consumes: completed downloader CLI.
- Produces: complete local corpus and manifest.

- [x] **Step 1: Run a bounded sample**

Run: `python3.12 scripts/dev/download-1ci-guides.py --max-pages 3 --output docs-local/1ci/8.3.27/en-sample`
Expected: three Markdown page records, downloaded assets, zero failures.

- [x] **Step 2: Inspect sample Markdown and links, then remove the sample through the script's staging replacement behavior**

Run: `python3.12 scripts/dev/download-1ci-guides.py --output docs-local/1ci/8.3.27/en --check-links`
Expected: all three guides complete, zero failed records, zero broken local links.

- [x] **Step 3: Verify repository and test state**

Run: `git status --short --ignored | rg 'docs-local|download-1ci|AGENTS|gitignore|test_product'`
Expected: corpus lines start with `!!`; only source, tests, rules, spec, and plan are tracked changes.

Run: `python3.12 -m unittest discover -s tests -v`
Expected: PASS.

Run: `git diff --check`
Expected: no output.
