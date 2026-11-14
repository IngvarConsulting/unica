"""Host adaptation must preserve the single-file audit and process ownership."""
import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch
from unittest.mock import Mock
from types import SimpleNamespace

from tests.agent_evaluation import dcs_driver as driver

# Размер тестов модуля (scripts/ci/python_sizes.py).
# Родной Job Object: запуск CLI-подобного процесса и проверка потомков.
MEDIUM = ("NativeAgentCliCleanupTests",)


class AgentCliHostTests(unittest.TestCase):
    def test_windows_read_only_command_selects_a_native_sandbox(self):
        with patch('tests.agent_evaluation.cli_host.IS_WINDOWS', True):
            command = driver.codex_command(Path('/workspace'), Path('/schema'), Path('/final'), [])
        self.assertIn('windows.sandbox="unelevated"', command)
        self.assertEqual(command[command.index('--sandbox') + 1], 'read-only')
        self.assertIn('--ignore-rules', command)

    def test_windows_help_is_literal_and_confined_to_the_prepared_file(self):
        shell = r'C:\Program Files\PowerShell\7\pwsh.exe'
        path = Path(r'C:\prepared packet\skill $value`name.md')
        script = "Get-Content -LiteralPath '" + str(path) + "'"
        with patch('tests.agent_evaluation.cli_host.IS_WINDOWS', True), \
             patch('tests.agent_evaluation.cli_host.powershell_path', return_value=shell):
            for command in [script, f'"{shell}" -Command "{script}"',
                            f'"{shell}" -NoProfile -Command "{script}"',
                            f'"{shell}" -Command "{script}"'.replace('\\', '\\\\')]:
                self.assertTrue(driver.allowed_help_command(command, path), command)
            for command in [script + '; Get-Content C:\\other', script + ' > C:\\output',
                            script.replace('LiteralPath', 'Path'), script.replace('skill ', 'other '),
                            f'"C:\\untrusted\\pwsh.exe" -Command "{script}"',
                            "cat '" + str(path) + "'",
                            '/bin/bash -lc "cat ' + "'" + str(path) + "'" + '"',
                            '/bin/sh -lc \"cat '+str(path)+'\"',
                            'Get-Content -LiteralPath "$env:SECRET"', 'cat $(Get-Content C:\\other)']:
                self.assertFalse(driver.allowed_help_command(command, path), command)

    def test_windows_daemon_cleanup_requires_complete_retained_job_evidence(self):
        from tests.agent_evaluation import cli_host
        with tempfile.TemporaryDirectory() as raw, patch.object(cli_host, 'IS_WINDOWS', True):
            state = Path(raw) / 'state'
            with self.assertRaisesRegex(RuntimeError, 'cleanup'):
                driver.stop_owned_daemons(state, Path(raw) / 'unica.exe')

    def test_unknown_or_nonzero_job_count_never_becomes_successful_cleanup(self):
        from tests.agent_evaluation import cli_host
        with tempfile.TemporaryDirectory() as raw:
            for complete, active in [(False, 0), (True, 1), (True, None)]:
                with self.subTest(complete=complete, active=active):
                    run = cli_host.WindowsRun.__new__(cli_host.WindowsRun)
                    run.process = Mock()
                    run.state = str(Path(raw) / 'state')
                    run.ownership = Mock()
                    run.ownership.terminate.return_value = SimpleNamespace(
                        complete=complete, active_processes=active, incomplete=())
                    with self.assertRaisesRegex(RuntimeError, 'cleanup'):
                        run.close()
                    with self.assertRaisesRegex(RuntimeError, 'cleanup'):
                        cli_host.consume_cleanup(Path(run.state))


@unittest.skipUnless(os.name == 'nt', 'requires native Windows Job Objects')
class NativeAgentCliCleanupTests(unittest.TestCase):
    # Завершение Job Object асинхронное: `TerminateJobObject` возвращается
    # раньше, чем процесс уходит из таблицы, и на занятом хосте это занимает
    # десятые доли секунды. Ждём смерти до дедлайна, а не нулевое время;
    # потомок, который дедлайн переждал, — по-прежнему падение.
    CLEANUP_DEADLINE_SECONDS = 1.0

    def _identity_after_cleanup(self, cli_host, pid):
        support = cli_host.process_support()
        deadline = time.monotonic() + self.CLEANUP_DEADLINE_SECONDS
        identity = support._current_process_identity(pid)
        while identity is not None and time.monotonic() < deadline:
            time.sleep(0.01)
            identity = support._current_process_identity(pid)
        return identity

    def test_success_and_timeout_clean_descendants_without_touching_an_unrelated_process(self):
        from tests.agent_evaluation import cli_host
        sentinel = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'])
        try:
            for timeout in [False, True]:
                with self.subTest(timeout=timeout), tempfile.TemporaryDirectory() as raw:
                    root = Path(raw)
                    pidfile = root / 'child.pid'
                    code = ('import subprocess,sys,time,json; from pathlib import Path; '
                            'child=subprocess.Popen([sys.executable,"-c","import time; time.sleep(60)"],'
                            'stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); '
                            f'Path({str(pidfile)!r}).write_text(str(child.pid)); '
                            'print(json.dumps({"type":"thread.started"}),flush=True); '
                            + ('time.sleep(60)' if timeout else ''))
                    env = {**os.environ, 'UNICA_PROVIDER_STATE_DIR': str(root / 'state')}
                    if timeout:
                        with self.assertRaises(subprocess.TimeoutExpired):
                            driver.run_cli([sys.executable, '-c', code], '', env, root / 'agent', timeout=3)
                    else:
                        events, _, _ = driver.run_cli([sys.executable, '-c', code], '', env, root / 'agent')
                        self.assertEqual(events, [{'type': 'thread.started'}])
                    self.assertTrue(pidfile.exists(), 'the descendant must actually have started')
                    pid = int(pidfile.read_text())
                    self.assertIsNone(
                        self._identity_after_cleanup(cli_host, pid),
                        'the descendant must not outlive the run',
                    )
                    self.assertIsNone(sentinel.poll(), 'cleanup terminated an unrelated process')
                    driver.stop_owned_daemons(root / 'state', root / 'unica.exe')
        finally:
            sentinel.terminate()
            sentinel.wait(timeout=10)
