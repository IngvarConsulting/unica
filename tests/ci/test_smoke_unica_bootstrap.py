from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-bootstrap.py"


def load_module():
    spec = importlib.util.spec_from_file_location("smoke_unica_bootstrap", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class SmokeUnicaBootstrapTests(unittest.TestCase):
    PUBLISHED_SHA = "a" * 64

    def plugin(self, root: Path) -> Path:
        plugin = root / "plugin"
        bootstrap = plugin / "bootstrap" / "bin" / "linux-x64" / "unica-bootstrap"
        bootstrap.parent.mkdir(parents=True)
        bootstrap.write_bytes(b"bootstrap")
        # Форма та же, что пишет упаковщик: артефакты по отдельности, у
        # каждого свои цели. Замороженная здесь схема 1 делала зонд
        # холостым — манифест такой формы он просто не находил.
        (plugin / "runtime-manifest.json").write_text(
            json.dumps(
                {
                    "schemaVersion": 2,
                    "pluginVersion": "0.9.1",
                    "development": False,
                    "release": {"tag": "v0.9.1"},
                    "artifacts": {
                        "unica": {
                            "version": "0.9.1",
                            "role": "core",
                            "targets": {
                                "linux-x64": {
                                    "asset": {
                                        "name": "unica-runtime-linux-x64.tar.gz",
                                        "sha256": self.PUBLISHED_SHA,
                                    }
                                }
                            },
                        },
                        "rlm-tools-bsl": {
                            "version": "1.33.0",
                            "role": "engine",
                            "targets": {
                                "linux-x64": {
                                    "asset": {
                                        "name": "rlm-tools-bsl-linux-x64.tar.gz",
                                        "sha256": self.PUBLISHED_SHA,
                                    }
                                }
                            },
                        },
                    },
                }
            ),
            encoding="utf-8",
        )
        return plugin

    def manifest_shas(self, plugin: Path) -> "list[str]":
        manifest = json.loads(
            (plugin / "runtime-manifest.json").read_text(encoding="utf-8")
        )
        return [
            target["asset"]["sha256"]
            for artifact in manifest["artifacts"].values()
            for target in artifact["targets"].values()
        ]

    def manifest_sha(self, plugin: Path) -> str:
        return self.manifest_shas(plugin)[0]

    def test_probe_accepts_controlled_download_failure(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr="unica-bootstrap: failed to download runtime: HTTP 404",
            )

            with patch.object(module.subprocess, "run", return_value=result):
                module.smoke(
                    plugin,
                    "linux-x64",
                    2,
                    expect_download_failure=True,
                )

    def test_probe_accepts_checksum_mismatch_for_an_already_published_tag(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr=(
                    "unica-bootstrap: runtime archive sha256 actual "
                    "!= expected candidate"
                ),
            )

            with patch.object(module.subprocess, "run", return_value=result):
                module.smoke(
                    plugin,
                    "linux-x64",
                    2,
                    expect_download_failure=True,
                )

    def test_probe_neutralises_the_checksum_before_running_and_restores_it(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            observed = {}

            def fake_run(*args, **kwargs):
                observed["sha"] = self.manifest_sha(plugin)
                observed["all"] = self.manifest_shas(plugin)
                return subprocess.CompletedProcess(
                    args=[],
                    returncode=1,
                    stdout="",
                    stderr="unica-bootstrap: runtime archive sha256 actual != expected",
                )

            with patch.object(module.subprocess, "run", side_effect=fake_run):
                module.smoke(plugin, "linux-x64", 2, expect_download_failure=True)

            # The bootstrap must never see a checksum that published bytes can
            # match, otherwise the probe's outcome depends on whether this version
            # is already released and whether the host builds reproducibly.
            self.assertEqual(observed["sha"], module.UNMATCHABLE_SHA256)
            # Все артефакты, а не только ядро: манифест обязан описывать байты,
            # которых не существует нигде.
            self.assertEqual(
                observed["all"], [module.UNMATCHABLE_SHA256] * 2, observed["all"]
            )
            # Later jobs consume the same artifact and need the manifest back.
            self.assertEqual(self.manifest_sha(plugin), self.PUBLISHED_SHA)

    def test_a_manifest_the_probe_cannot_neutralise_is_refused(self) -> None:
        """Форма манифеста менялась, зонд — нет, и он молча стал холостым.

        Тогда он проверял не «управляем ли отказ», а «доехали ли уже
        опубликованные байты»: на неопубликованной версии проходил из-за 404,
        на опубликованной — из-за расхождения сумм. Отказ вслух дешевле.
        """
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            manifest_path = plugin / "runtime-manifest.json"
            manifest_path.write_text(
                json.dumps({"schemaVersion": 3, "pluginVersion": "0.9.1"}),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(SystemExit, "declares no artifact assets"):
                module.neutralise_published_checksums(manifest_path)

    def test_probe_rejects_a_runtime_accepted_despite_the_neutralised_checksum(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(args=[], returncode=0, stdout="", stderr="")

            with patch.object(module.subprocess, "run", return_value=result):
                with self.assertRaisesRegex(SystemExit, "neutralised"):
                    module.smoke(plugin, "linux-x64", 2, expect_download_failure=True)

            self.assertEqual(self.manifest_sha(plugin), self.PUBLISHED_SHA)

    def test_the_manifest_is_restored_even_when_the_probe_never_starts(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))

            # The consumer-PATH guard raises before the probe runs, and it must
            # not leave the packaged artifact holding a neutralised checksum.
            with patch.object(module.shutil, "which", return_value="/usr/bin/node"):
                with self.assertRaisesRegex(SystemExit, "Node.js leaked"):
                    module.smoke(plugin, "linux-x64", 2, expect_download_failure=True)

            self.assertEqual(self.manifest_sha(plugin), self.PUBLISHED_SHA)

    def test_probe_requires_a_packaged_runtime_manifest(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            (plugin / "runtime-manifest.json").unlink()

            with self.assertRaisesRegex(SystemExit, "runtime manifest is missing"):
                module.smoke(plugin, "linux-x64", 2, expect_download_failure=True)

    def test_release_smoke_leaves_the_packaged_checksum_untouched(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            observed = {}

            def fake_run(*args, **kwargs):
                observed["sha"] = self.manifest_sha(plugin)
                return subprocess.CompletedProcess(
                    args=[],
                    returncode=0,
                    stdout="",
                    stderr=(
                        "verified Unica 0.9.1 package, runtime, and MCP tools at /cache"
                    ),
                )

            with patch.object(module.subprocess, "run", side_effect=fake_run):
                module.smoke(plugin, "linux-x64", 2, expect_download_failure=False)

        # The tag build verifies the real published bytes end to end.
        self.assertEqual(observed["sha"], self.PUBLISHED_SHA)

    def test_probe_rejects_stack_overflow_before_download_error(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=1,
                stdout="",
                stderr="thread 'main' has overflowed its stack",
            )

            with patch.object(module.subprocess, "run", return_value=result):
                with self.assertRaisesRegex(SystemExit, "overflowed its stack"):
                    module.smoke(
                        plugin,
                        "linux-x64",
                        2,
                        expect_download_failure=True,
                    )

    def test_release_smoke_requires_success_marker(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            result = subprocess.CompletedProcess(
                args=[],
                returncode=0,
                stdout="",
                stderr=(
                    "verified Unica 0.7.8 package, runtime, and MCP tools at "
                    "/tmp/unica/runtimes/0.7.8/linux-x64"
                ),
            )

            with patch.object(module.subprocess, "run", return_value=result):
                module.smoke(
                    plugin,
                    "linux-x64",
                    2,
                    expect_download_failure=False,
                )


if __name__ == "__main__":
    unittest.main()
