from __future__ import annotations

import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "unica-plugin-release.yml"


class UnicaWorkflowGuardrailTests(unittest.TestCase):
    def workflow_text(self) -> str:
        return WORKFLOW.read_text(encoding="utf-8")

    def test_pull_request_paths_cover_all_plugin_sources(self) -> None:
        text = self.workflow_text()
        required_paths = [
            ".agents/plugins/marketplace.json",
            ".github/workflows/unica-plugin-release.yml",
            "Cargo.toml",
            "Cargo.lock",
            "crates/unica-coder/**",
            "plugins/unica/**",
            "scripts/ci/**",
            "scripts/install-unica.sh",
            "tests/ci/**",
            "tests/fixtures/**",
            "spec/**",
        ]

        for path in required_paths:
            with self.subTest(path=path):
                self.assertIn(f'- "{path}"', text)

    def test_verify_source_job_runs_full_guardrail_suite(self) -> None:
        text = self.workflow_text()
        required_tokens = [
            "verify-source:",
            "uses: actions/checkout@v4",
            "uses: actions/setup-python@v5",
            'python-version: "3.12"',
            "python -m pip install -r tests/ci/requirements.txt",
            "uses: dtolnay/rust-toolchain@stable",
            "python -m unittest discover -s tests/ci",
            "python -m py_compile scripts/ci/*.py tests/ci/*.py",
            "python -m json.tool plugins/unica/.codex-plugin/plugin.json >/dev/null",
            "python -m json.tool plugins/unica/.mcp.json >/dev/null",
            "python -m json.tool plugins/unica/third-party/tools.lock.json >/dev/null",
            "python -m json.tool plugins/unica/third-party/manifest.json >/dev/null",
            "bash -n plugins/unica/scripts/*.sh",
            "cargo fmt --all -- --check",
            "cargo clippy --package unica-coder --all-targets --all-features -- -D warnings",
            "cargo test --package unica-coder",
        ]

        for token in required_tokens:
            with self.subTest(token=token):
                self.assertIn(token, text)

    def test_build_tools_waits_for_source_verification_and_sets_up_rust(self) -> None:
        text = self.workflow_text()
        self.assertIn("build-tools:", text)
        self.assertIn("needs: verify-source", text)
        self.assertIn("uses: dtolnay/rust-toolchain@stable", text)
        self.assertIn("python scripts/ci/build-unica-tools.py", text)


if __name__ == "__main__":
    unittest.main()
