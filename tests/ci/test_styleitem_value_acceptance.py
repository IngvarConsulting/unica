"""Check the MCP scenario's scalar expectations against independent XML facts."""
from __future__ import annotations

import json
from pathlib import Path
import unittest
import xml.etree.ElementTree as ET


ROOT = Path(__file__).resolve().parents[2]
BASE = ROOT / "tests/fixtures/acceptance"
MD = "{http://v8.1c.ru/8.3/MDClasses}"
UI = "{http://v8.1c.ru/8.1/data/ui}"


class StyleItemValueAcceptanceTests(unittest.TestCase):
    def test_detail_expectations_preserve_the_independent_xml_values(self):
        scenario = json.loads((BASE / "scenarios/S-1617-styleitem-font-border.json").read_text(encoding="utf-8"))
        workspace = ROOT / scenario["workspace"]
        for step in scenario["wire"][1:]:
            name = step["args"]["at"].removeprefix("$capture.")
            with self.subTest(name=name):
                props = ET.parse(workspace / f"src/StyleItems/{name}.xml").getroot().find(f"{MD}StyleItem/{MD}Properties")
                self.assertIsNotNone(props)
                value = props.find(f"{MD}Value")
                expected = {"Type": props.findtext(f"{MD}Type"), "kind": "StyleItem"}
                if expected["Type"] == "Font":
                    for attribute, raw in value.attrib.items():
                        if attribute in {"bold", "italic", "underline", "strikeout"}:
                            self.assertIn(raw, {"true", "false"})
                            observed = raw == "true"
                        elif attribute in {"height", "scale"}:
                            observed = int(raw)
                        elif attribute in {"ref", "kind", "faceName"}:
                            observed = raw
                        else:
                            continue
                        expected["font" + attribute[0].upper() + attribute[1:]] = observed
                else:
                    self.assertEqual(expected["Type"], "Border")
                    expected["borderWidth"] = int(value.attrib["width"])
                    expected["borderStyle"] = value.findtext(f"{UI}style")
                assertions = {item["pointer"].removeprefix("/data/props/"): item["eq"]
                              for item in step["assertions"] if "eq" in item}
                self.assertEqual(assertions, expected)
                for item in step["assertions"]:
                    if item.get("absent"):
                        self.assertNotIn(item["pointer"].removeprefix("/data/props/"), expected)
                self.assertEqual(
                    (workspace / f"src/StyleItems/{name}.xml").read_bytes(),
                    (ROOT / f"tests/fixtures/styleitem_values/{name}.xml").read_bytes(),
                )


if __name__ == "__main__":
    unittest.main()
