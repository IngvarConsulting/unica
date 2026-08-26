from __future__ import annotations

import hashlib
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
        (root / "docs" / "provenance").mkdir(parents=True)
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
        (root / "docs" / "provenance" / "skill-upstreams.json").write_text(
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
                ("tool", "rlm-bsl-mcp"),
                ("tool", "rlm-bsl-index"),
                ("adapter", "v8std"),
                ("upstream", "cc-1c-skills"),
                ("upstream", "ai-rules-1c"),
                ("upstream", "1c-design-guide"),
                ("upstream", "templates-new-object-1c"),
                ("upstream", "v8-runner-rust"),
            },
        )

    def test_design_guide_attribution_references_packaged_license(self) -> None:
        module = load_attribution_module()
        attribution = self.repo_root() / "plugins" / "unica" / "ATTRIBUTIONS.md"

        sections = module.parse_sections(attribution.read_text(encoding="utf-8"))

        self.assertIn(
            "[MIT](third-party/licenses/1c-design-guide/LICENSE)",
            sections.get(("upstream", "1c-design-guide"), ""),
        )

    def test_bsl_analyzer_packages_the_complete_upstream_license_set(self) -> None:
        root = self.repo_root()
        license_dir = root / "plugins/unica/third-party/licenses/bsl-analyzer"
        expected_hashes = {
            "LICENSE-APACHE": "62c7a1e35f56406896d7aa7ca52d0cc0d272ac022b5d2796e7d6905db8a3636a",
            "LICENSE-GPL": "fb981668c18a279e285fc4d83fba1e836cc84dd4daa73c9697d3cfd2d8aca6e0",
            "LICENSE-LGPL": "996af0513df21f7496288951c41428a03c174e9e4a9d63665c57d670f845ccb1",
            "LICENSE-MIT": "eabf424905be03c7e86b9ba3905ee0935936f85ad98afce697716f3d046ac838",
        }

        self.assertEqual(
            {path.name for path in license_dir.iterdir() if path.name.startswith("LICENSE-")},
            set(expected_hashes),
            "the package must carry the four license texts published with bsl-analyzer",
        )
        for name, expected_hash in expected_hashes.items():
            actual_hash = hashlib.sha256((license_dir / name).read_bytes()).hexdigest()
            self.assertEqual(actual_hash, expected_hash, name)

        section = load_attribution_module().parse_sections(
            (root / "plugins/unica/ATTRIBUTIONS.md").read_text(encoding="utf-8")
        )[("tool", "bsl-analyzer")]
        for name in expected_hashes:
            self.assertIn(f"third-party/licenses/bsl-analyzer/{name}", section)

    def test_bsl_analyzer_attribution_and_notice_match_current_contract(self) -> None:
        root = self.repo_root()
        lock = json.loads(
            (root / "plugins/unica/third-party/tools.lock.json").read_text(encoding="utf-8")
        )
        analyzer = next(tool for tool in lock["tools"] if tool["name"] == "bsl-analyzer")
        attribution = (root / "plugins/unica/ATTRIBUTIONS.md").read_text(encoding="utf-8")
        section = load_attribution_module().parse_sections(attribution)[("tool", "bsl-analyzer")]
        notice = (
            root / "plugins/unica/third-party/licenses/bsl-analyzer/NOTICE"
        ).read_text(encoding="utf-8")

        self.assertIn(f"`{analyzer['version']}`", section)
        self.assertIn(analyzer["sourceCommit"], section)
        for disclosure in (
            "Derivation notice for the SDBL and BSL grammar layers",
            "Notice for crates written with a copyleft reference open",
            "crates/bsl-metadata",
            "mdclasses",
        ):
            self.assertIn(disclosure, notice)

    def test_parse_sections_maps_grouped_markers_to_one_section(self) -> None:
        module = load_attribution_module()

        sections = module.parse_sections(
            "## RLM\n"
            "<!-- unica-attribution: tool rlm-bsl-mcp -->\n"
            "<!-- unica-attribution: tool rlm-bsl-index -->\n"
            "Общий текст.\n"
        )

        self.assertEqual(sections[("tool", "rlm-bsl-mcp")], sections[("tool", "rlm-bsl-index")])
        self.assertIn("Общий текст", sections[("tool", "rlm-bsl-mcp")])

    def test_rlm_attribution_identifies_current_immutable_build(self) -> None:
        root = self.repo_root()
        section = load_attribution_module().parse_sections(
            (root / "plugins/unica/ATTRIBUTIONS.md").read_text(encoding="utf-8")
        )[("tool", "rlm-bsl-mcp")]

        self.assertIn("`1.33.0`", section)
        self.assertIn("`3e6920cd015a61af4ba7aa1a5f1fedd8bc935549`", section)
        self.assertIn("`rlm-tools-bsl-v1.33.0-build.3`", section)
        self.assertIn("Nuitka", section)

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
        self.assertIn("tool other: license link must point to a packaged file", errors)

    def test_repository_attribution_page_is_complete_and_linked(self) -> None:
        module = load_attribution_module()
        repo_root = self.repo_root()

        self.assertEqual(module.validate_attributions(repo_root), [])
        self.assertIn(
            "[Авторы, источники и лицензии](plugins/unica/ATTRIBUTIONS.md)",
            (repo_root / "README.md").read_text(encoding="utf-8"),
        )
        self.assertIn(
            "[Авторы, источники и лицензии](ATTRIBUTIONS.md)",
            (repo_root / "plugins/unica/README.md").read_text(encoding="utf-8"),
        )


if __name__ == "__main__":
    unittest.main()
