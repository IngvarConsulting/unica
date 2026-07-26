from pathlib import Path
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]


class MetadataConventionReferenceTests(unittest.TestCase):
    def test_reference_describes_owner_aware_list_presentation_contract(self) -> None:
        text = (
            REPO_ROOT
            / "plugins"
            / "unica"
            / "references"
            / "platform"
            / "metadata-conventions.md"
        ).read_text(encoding="utf-8")
        for marker in (
            "ListPresentation",
            "Configuration.xml",
            "Languages/<Name>.xml",
            "ExternalReport",
            "ExternalDataProcessor",
        ):
            self.assertIn(marker, text)
        self.assertNotIn("наблюдаемым значениям `v8:lang`", text)
        self.assertNotIn("языконезависим", text)

    def test_reference_index_uses_path_relative_to_itself(self) -> None:
        text = (
            REPO_ROOT / "plugins" / "unica" / "references" / "README.md"
        ).read_text(encoding="utf-8")
        self.assertIn("`platform/metadata-conventions.md`", text)
        self.assertNotIn("`references/platform/metadata-conventions.md`", text)


if __name__ == "__main__":
    unittest.main()
