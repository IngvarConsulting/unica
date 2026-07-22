from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[2]
MATRIX = ROOT / "spec/0126-platform-8-3-27-deviation-matrix.md"


class FormatProfileContractTests(unittest.TestCase):
    def test_format_matrix_covers_native_xml_operations(self):
        text = MATRIX.read_text(encoding="utf-8")
        required = {
            "unica.cf.edit",
            "unica.cf.init",
            "unica.cfe.borrow",
            "unica.cfe.init",
            "unica.meta.compile",
            "unica.meta.edit",
            "unica.form.add",
            "unica.form.compile",
            "unica.form.edit",
            "unica.template.add",
            "unica.mxl.compile",
            "unica.role.compile",
            "unica.subsystem.compile",
        }
        missing = sorted(name for name in required if f"`{name}`" not in text)
        self.assertFalse(missing, missing)
        self.assertIn("2.17", text)

    def test_matrix_cites_official_8_3_27_mapping(self):
        text = MATRIX.read_text(encoding="utf-8")
        self.assertIn("8.3.27", text)
        self.assertIn("2.20", text)
        self.assertIn("Export_format_versions/index.md", text)


if __name__ == "__main__":
    unittest.main()
