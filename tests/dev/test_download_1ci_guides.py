import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "dev" / "download-1ci-guides.py"
SPEC = importlib.util.spec_from_file_location("download_1ci_guides", SCRIPT)
guides = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(guides)


class UrlContractTests(unittest.TestCase):
    def test_rest_request_can_require_json_without_changing_page_default(self):
        self.assertEqual(
            guides.request_headers("application/json")["Accept"], "application/json"
        )
        self.assertNotIn("Accept", guides.request_headers())

    def test_normalize_forces_english_and_drops_fragment(self):
        source = guides.DEVELOPER.root + "Chapter_1/#section"
        self.assertEqual(
            guides.normalize_page_url(source),
            guides.DEVELOPER.root + "Chapter_1/?language=en",
        )

    def test_lookalike_root_is_rejected(self):
        lookalike = guides.DEVELOPER.root.rstrip("/") + "_Other/"
        self.assertIsNone(guides.guide_for_url(lookalike))

    def test_page_path_mirrors_hierarchy(self):
        url = guides.DEVELOPER.root + "Chapter_1/1.2._Terms/?language=en"
        self.assertEqual(
            guides.page_relative_path(guides.DEVELOPER, url),
            Path("developer/Chapter_1/1.2._Terms/index.md"),
        )

    def test_only_bin_download_has_explicit_robots_exception(self):
        self.assertTrue(
            guides.is_allowed_by_policy(
                "https://kb.1ci.com/bin/download/Space/Page/example.zip", False
            )
        )
        self.assertFalse(
            guides.is_allowed_by_policy(
                "https://kb.1ci.com/bin/edit/Space/Page", False
            )
        )

    def test_space_catalog_discovers_pages_missing_from_sitemap(self):
        rows = sorted(
            [
                {"id": "xwiki:Help", "xwikiAbsoluteUrl": "https://kb.1ci.com/bin/view/Help/"},
                {
                    "id": guides.guide_space_id(guides.DEVELOPER),
                    "xwikiAbsoluteUrl": guides.DEVELOPER.root,
                },
                {
                    "id": guides.guide_space_id(guides.DEVELOPER) + ".Chapter_1\\._General_concepts",
                    "xwikiAbsoluteUrl": guides.DEVELOPER.root + "Chapter_1._General_concepts/",
                },
                {"id": "xwiki:Zzz", "xwikiAbsoluteUrl": "https://kb.1ci.com/bin/view/Zzz/"},
            ],
            key=lambda row: row["id"],
        )

        def fetch_batch(start, number):
            return rows[start : start + number]

        pages = guides.discover_space_pages(fetch_batch, (guides.DEVELOPER,), batch_size=2)

        self.assertEqual(
            pages,
            {
                guides.DEVELOPER.root + "?language=en",
                guides.DEVELOPER.root + "Chapter_1._General_concepts/?language=en",
            },
        )


FIXTURE_HTML = """
<html><body>
<nav>Sign in</nav>
<div id="xwikicontent">
  <h1>Chapter 1</h1>
  <p>Use <strong>safe</strong> mode and <a href="Child/">continue</a>.</p>
  <table><tr><th>Name</th><th>Value</th></tr><tr><td>A</td><td>B</td></tr></table>
  <pre>Message("hello");</pre>
  <img src="/bin/download/Space/Page/example.png" alt="Example"/>
  <a href="%20http://localhost/httpservice/hs/example">local HTTP example</a>
</div>
<footer>Copyright</footer>
</body></html>
"""


class ExtractionTests(unittest.TestCase):
    def test_extracts_xwiki_content_as_markdown(self):
        page = guides.extract_page(FIXTURE_HTML, guides.DEVELOPER.root)
        self.assertIn("# Chapter 1", page.markdown)
        self.assertIn("**safe**", page.markdown)
        self.assertIn("| Name | Value |", page.markdown)
        self.assertIn('Message("hello");', page.markdown)
        self.assertNotIn("Sign in", page.markdown)
        self.assertNotIn("Copyright", page.markdown)

    def test_collects_child_page_and_asset_urls(self):
        page = guides.extract_page(FIXTURE_HTML, guides.DEVELOPER.root)
        self.assertEqual(
            page.page_links,
            (guides.DEVELOPER.root + "Child/?language=en",),
        )
        self.assertEqual(
            page.assets,
            ("https://kb.1ci.com/bin/download/Space/Page/example.png",),
        )

    def test_whitespace_prefixed_absolute_example_is_not_a_guide_page(self):
        page = guides.extract_page(FIXTURE_HTML, guides.DEVELOPER.root)
        self.assertNotIn("localhost", "\n".join(page.page_links))
        self.assertIn("(http://localhost/httpservice/hs/example)", page.markdown)


class PublicationTests(unittest.TestCase):
    def test_link_check_ignores_syntax_placeholders(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "index.md").write_text(
                "[HIERARCHY](<Query description>)\n"
                "```\n[User](SessionNumber)\n```\n",
                encoding="utf-8",
            )

            self.assertEqual(guides._check_links(root), [])

    def test_link_check_still_reports_missing_links_outside_code(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "index.md").write_text("[Missing](missing.md)\n", encoding="utf-8")

            self.assertEqual(guides._check_links(root), ["index.md -> missing.md"])

    def test_limited_manifest_is_not_complete(self):
        manifest = guides.build_manifest([], [], max_pages=3)
        self.assertFalse(manifest["complete"])
        self.assertTrue(manifest["limited"])

    def test_failed_refresh_preserves_complete_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            destination = parent / "corpus"
            destination.mkdir()
            old = {"complete": True, "pages": ["old"]}
            (destination / "manifest.json").write_text(json.dumps(old))
            staging = parent / "staging"
            staging.mkdir()
            (staging / "manifest.json").write_text(json.dumps({"complete": False}))

            with self.assertRaises(guides.DownloadError):
                guides.publish_staging(staging, destination)

            self.assertEqual(
                json.loads((destination / "manifest.json").read_text()), old
            )


if __name__ == "__main__":
    unittest.main()
