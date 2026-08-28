from __future__ import annotations

import hashlib
import unittest
import xml.etree.ElementTree as ElementTree
import zipfile
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
VISUAL_KIT_ROOT = REPO_ROOT / "docs/visual-kit"
LOGO_NAME = "unica-logo-letter-transparent-paper-mark.svg"
LOGO_PATH = VISUAL_KIT_ROOT / "logos" / LOGO_NAME
DOWNLOAD_PATH = VISUAL_KIT_ROOT / "unica-visual-assets.zip"
SVG_NAMESPACE = "http://www.w3.org/2000/svg"


class VisualKitAssetTests(unittest.TestCase):
    def test_transparent_paper_mark_keeps_the_old_white_tile_without_a_canvas(self) -> None:
        self.assertTrue(LOGO_PATH.is_file(), f"missing {LOGO_PATH.relative_to(REPO_ROOT)}")

        root = ElementTree.parse(LOGO_PATH).getroot()
        full_canvas = [
            rect
            for rect in root.findall(f"{{{SVG_NAMESPACE}}}rect")
            if rect.get("width") == "192.096" and rect.get("height") == "53.36"
        ]
        self.assertEqual(full_canvas, [])

        tile = root.find(f"./{{{SVG_NAMESPACE}}}g/{{{SVG_NAMESPACE}}}rect")
        self.assertIsNotNone(tile)
        self.assertEqual(
            {attribute: tile.get(attribute) for attribute in ("width", "height", "rx", "fill")},
            {"width": "64", "height": "64", "rx": "15.304", "fill": "#FFFFFF"},
        )

        mark_path = root.find(f"./{{{SVG_NAMESPACE}}}g/{{{SVG_NAMESPACE}}}path")
        self.assertIsNotNone(mark_path)
        self.assertEqual(mark_path.get("stroke"), "#2563EB")

    def test_transparent_paper_mark_is_in_the_checksum_inventory_and_download(self) -> None:
        self.assertTrue(LOGO_PATH.is_file(), f"missing {LOGO_PATH.relative_to(REPO_ROOT)}")
        logo_bytes = LOGO_PATH.read_bytes()
        digest = hashlib.sha256(logo_bytes).hexdigest()
        checksum_line = f"{digest}  ./logos/{LOGO_NAME}"
        checksum_lines = (VISUAL_KIT_ROOT / "SHA256SUMS").read_text(encoding="utf-8").splitlines()
        self.assertIn(checksum_line, checksum_lines)

        archive_member = f"unica-visual-assets/logos/{LOGO_NAME}"
        with zipfile.ZipFile(DOWNLOAD_PATH) as archive:
            self.assertIn(archive_member, archive.namelist())
            self.assertEqual(archive.read(archive_member), logo_bytes)

        archive_digest = hashlib.sha256(DOWNLOAD_PATH.read_bytes()).hexdigest()
        self.assertIn(
            f"{archive_digest}  ./unica-visual-assets.zip",
            checksum_lines,
        )


if __name__ == "__main__":
    unittest.main()
