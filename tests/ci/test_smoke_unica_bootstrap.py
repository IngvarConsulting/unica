from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "smoke-unica-bootstrap.py"
# Сколько живёт внук, держащий stdout, пока его не убьёт уборка теста.
# Срок bootstrap в проверке — половина этой величины: дождись смоук EOF
# трубы, он упёрся бы в этот срок раньше, чем внук умрёт сам.
HOLDER_LIFETIME_SECONDS = 60.0
BOOTSTRAP_FUSE_SECONDS = HOLDER_LIFETIME_SECONDS / 2
# Предохранитель уборки фикстуры от зависания, не проверяемый срок.
CLEANUP_FUSE_SECONDS = 30.0


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

    def test_smoke_gives_daemon_a_physical_private_state_root(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            observed = {}

            def fake_run(*args, **kwargs):
                observed.update(kwargs["env"])
                self.assertNotEqual(kwargs["stdout"], subprocess.PIPE)
                self.assertNotEqual(kwargs["stderr"], subprocess.PIPE)
                return subprocess.CompletedProcess(args, 0, "", "verified Unica 0.9.1 package, runtime, and MCP tools at /cache")

            with patch.object(module.subprocess, "run", side_effect=fake_run):
                module.smoke(plugin, "linux-x64", 2, expect_download_failure=False)

            provider = Path(observed["UNICA_PROVIDER_STATE_DIR"])
            self.assertEqual(provider.parent, provider.parent.resolve())
            self.assertEqual(provider.parent, Path(observed["CODEX_HOME"]).parent)

    def test_smoke_keeps_the_verdict_when_the_daemon_holds_its_state(self) -> None:
        # Демон, которого поднимает verify, переживает зонд: выдерживает
        # период простоя и держит открытыми файлы в каталоге дыма. Windows их
        # не удаляет (WinError 32 — это PermissionError), и ошибка уборки
        # подменяла вердикт verify в обе стороны.
        module = load_module()
        real_unlink = os.unlink
        locks: "list[Path]" = []

        def held_unlink(path, *args, **kwargs):
            if Path(path).name == ".receipt-authority.lock":
                raise PermissionError(13, "held by the daemon", str(path))
            return real_unlink(path, *args, **kwargs)

        def verify(returncode: int, stderr: str):
            def fake_run(args, **kwargs):
                state = Path(kwargs["env"]["UNICA_PROVIDER_STATE_DIR"])
                self.addCleanup(shutil.rmtree, state.parent, True)
                lock = state / "daemon-p5-smoke" / ".receipt-authority" / ".receipt-authority.lock"
                lock.parent.mkdir(parents=True)
                lock.write_text("held", encoding="utf-8")
                locks.append(lock)
                return subprocess.CompletedProcess(args, returncode, "", stderr)

            return fake_run

        with tempfile.TemporaryDirectory() as directory:
            plugin = self.plugin(Path(directory))
            with patch.object(os, "unlink", held_unlink):
                passed = verify(0, "verified Unica 0.9.1 package, runtime, and MCP tools at /cache")
                with patch.object(module.subprocess, "run", side_effect=passed):
                    module.smoke(plugin, "linux-x64", 2, expect_download_failure=False)
                # Уборка действительно наткнулась на удерживаемый файл.
                self.assertTrue(locks[0].exists())

                failed = verify(3, "runtime refused the handshake")
                with patch.object(module.subprocess, "run", side_effect=failed):
                    with self.assertRaisesRegex(SystemExit, "exited with 3: runtime refused the handshake"):
                        module.smoke(plugin, "linux-x64", 2, expect_download_failure=False)
                self.assertTrue(locks[1].exists())

    def test_smoke_does_not_wait_for_a_descendant_holding_standard_output(self) -> None:
        module = load_module()
        wire_path = REPO_ROOT / "scripts" / "ci" / "probe-unica-wire.py"
        wire_spec = importlib.util.spec_from_file_location("bootstrap_fixture_ownership", wire_path)
        assert wire_spec is not None and wire_spec.loader is not None
        wire = importlib.util.module_from_spec(wire_spec)
        wire_spec.loader.exec_module(wire)
        with tempfile.TemporaryDirectory() as directory:
            probe = Path(directory) / "inherited-output.py"
            spawned = Path(directory) / "descendant-started"
            holder_pid_path = Path(directory) / "descendant.pid"
            probe.write_text(
                (
                    "import subprocess, sys, time\n"
                    # Only real subprocess.run.communicate closes this pipe,
                    # after the Popen spy has retained ownership of the parent.
                    "sys.stdin.buffer.read()\n"
                    "child = subprocess.Popen([sys.executable, '-c', "
                    f"'import time; time.sleep({HOLDER_LIFETIME_SECONDS!r})'])\n"
                    f"open({str(holder_pid_path)!r}, 'w').write(str(child.pid))\n"
                    f"open({str(spawned)!r}, 'w').write('started')\n"
                    "print('bootstrap started', file=sys.stderr, flush=True)\n"
                ),
                encoding="utf-8",
            )
            command = [sys.executable, str(probe)]
            real_popen = subprocess.Popen
            owned = []
            holder_identities = set()

            def owned_popen(argv, *args, **kwargs):
                if argv != command:
                    # Ownership capture itself runs ps: do not intercept it.
                    return real_popen(argv, *args, **kwargs)
                self.assertNotIn("stdin", kwargs)
                kwargs["stdin"] = subprocess.PIPE
                if os.name == "posix":
                    kwargs["start_new_session"] = True
                elif os.name == "nt":
                    kwargs["creationflags"] = (
                        subprocess.CREATE_NEW_PROCESS_GROUP | wire._WINDOWS_CREATE_SUSPENDED
                    )
                else:
                    raise AssertionError("fixture process ownership is unavailable")
                process = real_popen(argv, *args, **kwargs)
                record = [process, None]
                # Retain the still-gated process even if capture raises.
                owned.append(record)
                owner = wire.ProcessOwnership.capture(
                    process, cleanup_deadline=time.monotonic() + 0.5
                )
                record[1] = owner
                self.assertIsNotNone(owner.public_identity, "parent identity was not captured")
                if os.name == "posix":
                    self.assertEqual(owner.session_id, process.pid)
                return process

            try:
                with patch.object(module.subprocess, "Popen", side_effect=owned_popen):
                    code, _, stderr = module.run_bootstrap_command(
                        command, os.environ.copy(), BOOTSTRAP_FUSE_SECONDS
                    )
                self.assertEqual(code, 0)
                self.assertEqual(spawned.read_text(), "started")
                self.assertIn("bootstrap started", stderr)
                self.assertEqual(len(owned), 1)
                owner = owned[0][1]
                holder_pid = int(holder_pid_path.read_text())
                holder_identities = owner.capture_identities({holder_pid})
                self.assertEqual({identity.pid for identity in holder_identities}, {holder_pid})
                # Событие вместо замера: смоук вернулся, пока внук с его
                # stdout ещё жив, значит, EOF этой трубы он не ждал.
                self.assertEqual(
                    owner.survivors(holder_identities),
                    holder_identities,
                    "the bootstrap smoke returned only after the stdout holder exited",
                )
                if os.name == "posix":
                    self.assertEqual(
                        {identity.session_id for identity in holder_identities}, {owner.session_id}
                    )
                else:
                    active, error = owner.windows_job.active_process_count()
                    self.assertIsNone(error)
                    self.assertGreaterEqual(active, 1)
            finally:
                # Cleanup is outside the property, but has one bounded fuse,
                # including the independent postcondition.
                cleanup_deadline = time.monotonic() + CLEANUP_FUSE_SECONDS
                cleanup_errors = []
                for process, owner in owned:
                    try:
                        remaining = cleanup_deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("fixture cleanup deadline expired")
                        if owner is not None:
                            result = owner.terminate(holder_identities, timeout_seconds=remaining)
                            self.assertTrue(result.complete, (result.survivors, result.incomplete))
                            if os.name == "nt":
                                self.assertEqual(result.active_processes, 0)
                        elif os.name != "nt" or getattr(process, "_handle", None) is not None:
                            # Capture failed before communicate opened the gate:
                            # no descendant can have been launched. Kill/reap the
                            # direct parent before closing its private stdin.
                            if process.poll() is None:
                                process.kill()
                            process.wait(timeout=max(0.0, cleanup_deadline - time.monotonic()))
                        else:
                            # Windows attach rollback already closed its handle;
                            # never signal/wait through that closed handle again.
                            self.assertIsNotNone(process.returncode)
                        self.assertIsNotNone(process.returncode, "fixture parent was not reaped")
                    except BaseException as error:
                        cleanup_errors.append(repr(error))
                    finally:
                        if process.returncode is not None and process.stdin is not None:
                            try:
                                process.stdin.close()
                            except OSError as error:
                                cleanup_errors.append(repr(error))
                if os.name == "nt":
                    try:
                        for identity in holder_identities:
                            if time.monotonic() >= cleanup_deadline:
                                raise TimeoutError("no time remains to verify fixture cleanup")
                            current, running, error = wire._windows_process_state(identity.pid)
                            self.assertIsNone(error)
                            self.assertFalse(
                                running and identity.identifies(current),
                                "fixture pipe holder is still running",
                            )
                    except BaseException as error:
                        cleanup_errors.append(repr(error))
                if os.name == "posix" and owned:
                    try:
                        remaining = cleanup_deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("no time remains to verify fixture cleanup")
                        # An independent postcondition: read the table directly
                        # instead of trusting the helper under test, and
                        # require a successful read before claiming that our
                        # newly isolated sessions have no live members.
                        snapshot = subprocess.run(
                            ["ps", "-axo", "pid=,uid=,stat=,lstart="],
                            capture_output=True, text=True, encoding="utf-8",
                            timeout=remaining, check=True,
                        )
                        self.assertFalse(snapshot.stderr, snapshot.stderr)
                        rows = [line.split(None, 3) for line in snapshot.stdout.splitlines()]
                        self.assertTrue(all(len(row) == 4 for row in rows))
                        self.assertIn(os.getpid(), {int(row[0]) for row in rows})
                        sessions = {process.pid for process, _ in owned}
                        survivors = []
                        for pid_text, uid_text, state, _ in rows:
                            if time.monotonic() >= cleanup_deadline:
                                raise TimeoutError("fixture cleanup verification deadline expired")
                            if int(uid_text) != os.getuid() or state.startswith("Z"):
                                continue
                            pid = int(pid_text)
                            try:
                                session = os.getsid(pid)
                            except ProcessLookupError:
                                continue
                            if session in sessions:
                                survivors.append(pid)
                        self.assertFalse(survivors, f"fixture session still has live processes: {survivors}")
                    except BaseException as error:
                        cleanup_errors.append(repr(error))
                self.assertFalse(cleanup_errors, f"fixture cleanup incomplete: {cleanup_errors}")

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
