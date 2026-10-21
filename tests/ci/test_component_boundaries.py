from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("component_boundary", ROOT / "scripts/ci/check-component-boundaries.py")
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)


class ComponentBoundaryTests(unittest.TestCase):
    def test_real_workspace_retains_component_direction(self) -> None:
        self.assertEqual(checker.check(ROOT), [])

    def test_owner_edges_reject_hidden_concrete_dependencies(self) -> None:
        invalid = (
            ("unica-operations", "unica-forms"),
            ("unica-analysis-services", "unica-adapter-rlm"),
            ("unica-analysis-services", "unica-adapter-bsl-analyzer"),
            ("unica-code-intelligence", "unica-adapter-lexical"),
            ("unica-diagnostics", "unica-adapter-bsl-analyzer"),
            ("unica-navigation", "unica-metadata-designer"),
            ("unica-bindings", "unica-forms-designer"),
            ("unica-metadata", "unica-metadata-designer"),
            ("unica-forms", "unica-workspace"),
            ("unica-forms-designer", "unica-workspace"),
            ("unica-roles-designer", "unica-forms-designer"),
            ("unica-xdto-designer", "unica-forms-designer"),
            ("unica-source-designer", "unica-dcs"),
            ("unica-workspace", "unica-analysis-services"),
            ("unica-workspace", "unica-forms-designer"),
            ("unica-project-health", "unica-execution"),
            ("unica-project-health", "unica-metadata-designer"),
            ("unica-project-health", "unica-adapter-rlm"),
            ("unica-execution", "unica-adapter-v8-runner"),
            ("unica-execution", "unica-daemon"),
            ("unica-execution", "unica-bootstrap"),
            ("unica-execution", "unica-metadata-designer"),
            ("unica-daemon", "unica-forms"),
            ("unica-daemon", "unica-adapter-rlm"),
            ("unica-daemon", "unica-project-health"),
            ("unica-bsl-parser", "unica-bsl"),
            ("unica-roles", "unica-coder"),
            ("unica-bootstrap", "rmcp"),
            ("unica-coder", "bsl-syntax"),
            ("unica-code-intelligence", "syntax"),
        )
        for kind in ("dependencies", "build-dependencies", "dev-dependencies"):
            for owner, target in invalid:
                with self.subTest(kind=kind, owner=owner, target=target):
                    self.assertTrue(checker.edge_errors(owner, target, kind))

    def test_real_semantic_and_backend_edges_are_allowed(self) -> None:
        for owner, target in (
            ("unica-forms", "unica-metadata"),
            ("unica-bindings", "unica-forms"),
            ("unica-bindings", "unica-metadata"),
            ("unica-bindings", "unica-bsl"),
            ("unica-analysis-services", "unica-workspace"),
            ("unica-adapter-rlm", "unica-analysis-services"),
            ("unica-adapter-bsl-analyzer", "unica-diagnostics"),
            ("unica-adapter-lexical", "unica-code-intelligence"),
            ("unica-dcs", "unica-templates"),
            ("unica-roles-designer", "unica-source-designer"),
            ("unica-dcs-designer", "unica-operations"),
            ("unica-forms-designer", "unica-bsl-parser"),
            ("unica-metadata-designer", "unica-bsl-parser"),
            ("unica-xdto-designer", "unica-metadata-designer"),
            ("unica-workspace", "unica-source-contract"),
            ("unica-project-health", "unica-workspace"),
            ("unica-project-health", "unica-process"),
            ("unica-project-health", "unica-source-designer"),
            ("unica-execution", "unica-runner"),
            ("unica-execution", "unica-workspace"),
            ("unica-execution", "unica-execution-contract"),
            ("unica-daemon", "unica-execution"),
            ("unica-daemon", "unica-workspace"),
            ("unica-daemon", "unica-bootstrap"),
            ("unica-daemon", "unica-artifact-delivery"),
            ("unica-daemon", "unica-process"),
            ("unica-adapter-v8-runner", "unica-execution"),
            ("unica-coder", "rmcp"),
            ("unica-bsl-parser", "bsl-syntax"),
        ):
            with self.subTest(owner=owner, target=target):
                self.assertEqual(checker.edge_errors(owner, target, "dependencies"), [])

    def test_target_specific_alias_cannot_hide_a_composition_backedge(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace]\nmembers=["family", "coder"]\n')
            for directory, name in (("family", "unica-roles"), ("coder", "unica-coder")):
                (root / directory).mkdir()
                (root / directory / "Cargo.toml").write_text('[package]\nname="'+name+'"\n')
            with (root / "family/Cargo.toml").open("a") as output:
                output.write('\n[target.\'cfg(windows)\'.dev-dependencies]\nhelper={package="unica-coder",path="../coder"}\n')
            self.assertIn("dev-dependencies -> unica-coder", "\n".join(checker.check(root)))

    def test_unknown_local_member_and_wrong_alias_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace]\nmembers=["owner"]\n')
            (root / "owner").mkdir()
            manifest = root / "owner/Cargo.toml"
            manifest.write_text('[package]\nname="unica-operations"\n[dependencies]\nhelper={path="../unlisted"}\n')
            with self.assertRaisesRegex(ValueError, "not a workspace member"):
                checker.check(root)
            manifest.write_text('[package]\nname="unica-operations"\n[dependencies]\nhelper={package="unica-metadata",path="../owner"}\n')
            with self.assertRaisesRegex(ValueError, "path provides unica-operations"):
                checker.check(root)

    def test_workspace_inherited_parser_alias_keeps_the_upstream_boundary(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace]\nmembers=["coder"]\n[workspace.dependencies]\nraw={package="syntax",git="https://example.invalid/analyzer"}\n')
            (root / "coder").mkdir()
            (root / "coder/Cargo.toml").write_text('[package]\nname="unica-coder"\n[dependencies]\nraw={workspace=true}\n')
            self.assertIn("upstream parser types belong", "\n".join(checker.check(root)))
