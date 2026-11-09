"""Push тега выпуска ключом из секрета окружения, а не токеном прогона.

Теги `v*` защищены ruleset: создавать их могут только владелец и ключ развёртывания
из секрета `RELEASE_TAG_KEY`. Секрет лежит в окружении `alpha`, которое пускает
только `main`: ключ получают ночь и кнопки выпуска (`rc-from-alpha.yml`,
`release-from-rc.yml`), а writer, запустивший workflow со своей ветки или с тега,
ключа не получит. Кнопки выпуска дальше проверки роли пропускают только
администратора репозитория.

Ключ хоста GitHub закреплён здесь (`gh api meta`, поле `ssh_keys`): первый
встречный ключ на соединении не принимается.
"""

from __future__ import annotations

import os
import tempfile
from collections.abc import Iterator
from contextlib import contextmanager
from pathlib import Path

SECRET = "RELEASE_TAG_KEY"
GITHUB_HOST_KEY = "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl"


class TagKeyMissing(RuntimeError):
    pass


@contextmanager
def ssh_remote(repo: str, environ: dict[str, str] | None = None) -> Iterator[tuple[str, dict[str, str]]]:
    """Адрес репозитория по SSH и окружение git с ключом из секрета; файлы ключа живут только внутри."""
    environ = dict(os.environ if environ is None else environ)
    key = environ.get(SECRET, "")
    if not key.strip():
        raise TagKeyMissing(
            f"для push тега нужен секрет {SECRET} окружения задания: ключ развёртывания с правом записи"
        )
    with tempfile.TemporaryDirectory(prefix="tag-push-") as directory:
        folder = Path(directory)
        identity = folder / "key"
        identity.write_text(key.strip() + "\n", encoding="utf-8")
        identity.chmod(0o600)
        known = folder / "known_hosts"
        known.write_text(GITHUB_HOST_KEY + "\n", encoding="utf-8")
        command = (f"ssh -i {identity.as_posix()} -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes "
                   f"-o UserKnownHostsFile={known.as_posix()} -o HostKeyAlgorithms=ssh-ed25519")
        environ.pop(SECRET, None)
        yield f"git@github.com:{repo}.git", {**environ, "GIT_SSH_COMMAND": command}
