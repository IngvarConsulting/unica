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


FIXTURE_HTML = """
<html><body>
<nav>Sign in</nav>
<div id="xwikicontent">
  <h1>Chapter 1</h1>
  <p>Use <strong>safe</strong> mode and <a href="Child/">continue</a>.</p>
  <table><tr><th>Name</th><th>Value</th></tr><tr><td>A</td><td>B</td></tr></table>
  <pre>Message("hello");</pre>
  <img src="/bin/download/Space/Page/example.png" alt="Example"/>
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


class PublicationTests(unittest.TestCase):
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
