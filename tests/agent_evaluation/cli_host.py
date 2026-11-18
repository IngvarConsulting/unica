"""Native CLI hosting; the evidence and mutation audits stay host independent."""
import functools
import importlib.util
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import time

IS_WINDOWS = sys.platform == 'win32'
_completed_jobs = set()


@functools.cache
def process_support():
    path = Path(__file__).resolve().parents[2] / 'scripts/ci/probe-unica-wire.py'
    spec = importlib.util.spec_from_file_location('agent_owned_processes', path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class WindowsRun:
    def __init__(self, command, environment, stdout, stderr):
        self.state = environment.get('UNICA_PROVIDER_STATE_DIR')
        if self.state:
            _completed_jobs.discard(Path(self.state).resolve())
        self.process = subprocess.Popen(
            command, stdin=subprocess.PIPE, stdout=stdout, stderr=stderr, env=environment,
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.CREATE_NO_WINDOW | 0x00000004,
        )
        # The leader cannot spawn descendants until the retained Job Object
        # is assigned and the existing ownership helper resumes its thread.
        try:
            support = process_support()
            job = support._attach_windows_process_job(self.process, deadline=time.monotonic() + 10)
            self.ownership = support.ProcessOwnership(self.process, None, None, windows_job=job)
        except BaseException:
            if getattr(self.process, '_handle', None) is not None:
                self.process.kill()
                self.process.wait(timeout=10)
            raise

    def close(self):
        result = self.ownership.terminate(timeout_seconds=10)
        self.process.wait(timeout=10)
        if not result.complete or result.active_processes != 0:
            raise RuntimeError(f'Windows CLI cleanup incomplete: {result.incomplete}; active={result.active_processes}')
        if self.state:
            _completed_jobs.add(Path(self.state).resolve())


def consume_cleanup(state):
    state = Path(state).resolve()
    if state not in _completed_jobs:
        raise RuntimeError('missing complete retained Windows CLI cleanup proof')
    _completed_jobs.remove(state)


def powershell_path():
    return shutil.which('pwsh.exe') or shutil.which('powershell.exe')


def help_command(path):
    if IS_WINDOWS:
        return "Get-Content -LiteralPath '" + str(path).replace("'", "''") + "'"
    return 'cat ' + shlex.quote(str(path))


def windows_help_allowed(command, path):
    if not IS_WINDOWS:
        return False
    literal = "Get-Content -LiteralPath '" + str(path).replace("'", "''") + "'"
    if command == literal:
        return True
    try:
        parts = shlex.split(command, posix=False)
    except ValueError:
        return False
    shell = powershell_path()
    if not shell or not parts:
        return False
    if parts[1:-1] not in [['-Command'], ['-NoProfile', '-Command'], ['-NoLogo', '-NoProfile', '-Command']]:
        return False
    script = parts[-1]
    if len(script) >= 2 and script[0] == script[-1] == '"':
        script = script[1:-1]
    # CLI command events may quote Windows separators twice for display.
    # Compare two exact renderings; never decode arbitrary shell escapes.
    for logged_shell, logged_literal in [(shell, literal),
                                         (shell.replace('\\', '\\\\'), literal.replace('\\', '\\\\'))]:
        if parts[0].strip('"').casefold() == logged_shell.casefold() and script == logged_literal:
            return True
    return False
