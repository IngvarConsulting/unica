from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


def load_attribution_module():
    module_path = Path(__file__).resolve().parents[2] / "scripts" / "ci" / "check-attributions.py"
    spec = importlib.util.spec_from_file_location("check_attributions", module_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {module_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class AttributionTests(unittest.TestCase):
    def repo_root(self) -> Path:
        return Path(__file__).resolve().parents[2]

    def make_fixture_repo(self, root: Path) -> None:
        plugin = root / "plugins" / "unica"
        (plugin / ".codex-plugin").mkdir(parents=True)
        (plugin / "third-party").mkdir()
        (plugin / "provenance").mkdir()
        (plugin / ".codex-plugin" / "plugin.json").write_text(
            json.dumps(
                {
                    "name": "unica",
                    "repository": "https://example.invalid/unica",
                    "author": {"name": "Unica Author", "url": "https://example.invalid/author"},
                    "license": "LGPL-3.0-or-later",
                }
            ),
            encoding="utf-8",
        )
        (plugin / "third-party" / "tools.lock.json").write_text(
            json.dumps(
                {
                    "tools": [
                        {
                            "name": "unica",
                            "repository": "https://example.invalid/unica",
                            "license": "LGPL-3.0-or-later",
                        },
                        {
                            "name": "demo",
                            "repository": "https://example.invalid/demo",
                            "license": "MIT",
                        },
                        {
                            "name": "other",
                            "repository": "https://example.invalid/other",
                            "license": "MIT",
                        },
                    ]
                }
            ),
            encoding="utf-8",
        )
        (plugin / "third-party" / "manifest.json").write_text(
            json.dumps({"internalAdapters": []}), encoding="utf-8"
        )
        (plugin / "provenance" / "skill-upstreams.json").write_text(
            json.dumps({"upstreams": []}), encoding="utf-8"
        )
        (plugin / "LICENSE").write_text("license", encoding="utf-8")

    def test_expected_markers_follow_package_inventories(self) -> None:
        module = load_attribution_module()

        self.assertEqual(
            module.expected_markers(self.repo_root()),
            {
                ("project", "unica"),
                ("tool", "bsl-analyzer"),
                ("tool", "v8-runner"),
                ("tool", "rlm-tools-bsl"),
                ("tool", "rlm-bsl-index"),
                ("adapter", "v8std"),
                ("upstream", "cc-1c-skills"),
                ("upstream", "ai-rules-1c"),
                ("upstream", "v8-runner-rust"),
            },
        )

    def test_parse_sections_maps_grouped_markers_to_one_section(self) -> None:
        module = load_attribution_module()

        sections = module.parse_sections(
            "## RLM\n"
            "<!-- unica-attribution: tool rlm-tools-bsl -->\n"
            "<!-- unica-attribution: tool rlm-bsl-index -->\n"
            "Общий текст.\n"
        )

        self.assertEqual(sections[("tool", "rlm-tools-bsl")], sections[("tool", "rlm-bsl-index")])
        self.assertIn("Общий текст", sections[("tool", "rlm-tools-bsl")])

    def test_parse_sections_rejects_duplicate_markers(self) -> None:
        module = load_attribution_module()

        with self.assertRaisesRegex(ValueError, "duplicate attribution marker: tool demo"):
            module.parse_sections(
                "## One\n<!-- unica-attribution: tool demo -->\nA\n"
                "## Two\n<!-- unica-attribution: tool demo -->\nB\n"
            )

    def test_validation_reports_missing_unknown_and_invalid_repository_links(self) -> None:
        module = load_attribution_module()

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.make_fixture_repo(root)
            attribution = root / "plugins" / "unica" / "ATTRIBUTIONS.md"
            attribution.write_text(
                "## Unica\n"
                "<!-- unica-attribution: project unica -->\n"
                "- Репозиторий: [Unica](https://example.invalid/unica)\n"
                "- Автор: [Author](https://example.invalid/author)\n"
                "- Лицензия: [LGPL](LICENSE)\n\n"
                "## Other\n"
                "<!-- unica-attribution: tool other -->\n"
                "- Репозиторий: [Other](http://example.invalid/other)\n"
                "- Автор: [Author](https://example.invalid/author)\n"
                "- Лицензия: [Apache](https://example.invalid/license)\n\n"
                "## Ghost\n"
                "<!-- unica-attribution: tool ghost -->\n",
                encoding="utf-8",
            )

            errors = module.validate_attributions(root, attribution)

        self.assertIn("missing attribution: tool demo", errors)
        self.assertIn("unknown attribution: tool ghost", errors)
        self.assertIn("tool other: repository link must match https://example.invalid/other", errors)
        self.assertIn("tool other: declared license MIT must appear in the section", errors)


if __name__ == "__main__":
    unittest.main()
