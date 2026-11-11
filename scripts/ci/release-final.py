#!/usr/bin/env python3
"""Финальная версия `vX.Y.Z` из проверенного кандидата: служебный коммит и тег ставит workflow.

Кандидат `vX.Y.Z-rc.N` уже прошёл полный выпуск по тегу и лежит в канале `next`.
Финальная версия берёт коммит его тега и ставит поверх служебный коммит, где
меняется только версия. Перед этим проверяется:

- тег кандидата существует, и его выпуск опубликован;
- прогон выпуска по push этого тега (`release.yml`) зелёный;
- в маркетплейсе есть тег кандидата: он появляется только вместе с каталогом
  канала `next`, который его называет;
- кандидат — последний для своего ядра: финальная версия не обходит более
  позднего кандидата;
- выпуска `vX.Y.Z` ещё нет;
- ветки линии `release-vX.Y` ещё нет.

Ключ развёртывания из окружения `alpha` (`tag_push`) одним атомарным push
ставит тег и создаёт на том же коммите ветку линии `release-vX.Y`: из её головы
потом выходят исправительные версии. Push тега запускает обычный выпуск по тегу:
тесты, сборку, проверки установки и основной каталог маркетплейса. Кнопку нажимает администратор репозитория: роль сверяет
`release-rc.py admit`.

    python scripts/ci/release-final.py publish --rc v0.15.0-rc.2
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tag_push  # noqa: E402

_SPEC = importlib.util.spec_from_file_location("release_rc", Path(__file__).with_name("release-rc.py"))
rc = importlib.util.module_from_spec(_SPEC)
sys.modules.setdefault("release_rc", rc)
_SPEC.loader.exec_module(rc)
alpha = rc.alpha

RELEASE = "release.yml"
MARKETPLACE = "IngvarConsulting/unica-marketplace"


class FinalError(RuntimeError):
    pass


@dataclass(frozen=True)
class Proof:
    rc: str
    commit: str
    run_url: str


def line_branch(core: str) -> str:
    """Ветка линии выпуска: `release-vX.Y` для ядра `X.Y.Z`."""
    major, minor, _ = core.split(".")
    return f"release-v{major}.{minor}"


def pages(answer, key: str) -> list[dict]:
    """Записи ответа списка: одна страница или склеенные `--paginate`."""
    if isinstance(answer, dict):
        return answer.get(key, [])
    return [item for page in answer or [] for item in page.get(key, [])]


def prove(repo: str, rc_tag: str, gh, git) -> Proof:
    """Доказательство, что кандидат выпущен и опубликован в `next`; иначе отказ."""
    match = rc.RC.match(rc_tag)
    if match is None:
        raise FinalError(f"не тег кандидата: {rc_tag}")
    core = match["core"]
    tags = alpha.version_tags(repo, gh)
    if rc_tag not in tags:
        raise FinalError(f"тега {rc_tag} нет")
    if f"v{core}" in tags:
        raise FinalError(f"выпуск v{core} уже вышел")
    later = [tag for tag in tags if (other := rc.RC.match(tag)) and other["core"] == core
             and int(other["number"]) > int(match["number"])]
    if later:
        raise FinalError(f"у v{core} есть более поздний кандидат: {', '.join(sorted(later))}")
    release = gh(f"repos/{repo}/releases/tags/{rc_tag}") or {}
    if not release or release.get("draft"):
        raise FinalError(f"выпуск {rc_tag} не опубликован")
    commit = git(["rev-parse", f"{rc_tag}^{{commit}}"]).strip()
    runs = pages(gh(f"repos/{repo}/actions/workflows/{RELEASE}/runs?event=push&head_sha={commit}&per_page=100"),
                 "workflow_runs")
    green = [run for run in runs if run.get("head_branch") == rc_tag and run.get("head_sha") == commit
             and run.get("conclusion") == "success"]
    if not green:
        raise FinalError(f"зелёного выпуска по тегу {rc_tag} нет")
    # Зелёный итог прогона говорит о его последней попытке. Публикацию
    # подтверждают отдельно: тег кандидата в маркетплейсе появляется только
    # атомарным push вместе с каталогом `next`, который его называет.
    try:
        gh(f"repos/{MARKETPLACE}/git/ref/tags/{rc_tag}")
    except alpha.AlphaError as error:
        if "Not Found" in str(error) or "404" in str(error):
            raise FinalError(f"{rc_tag} не опубликован в канале next маркетплейса") from error
        raise FinalError(f"тег {rc_tag} в маркетплейсе не прочитан: {error}") from error
    line = line_branch(match["core"])
    try:
        gh(f"repos/{repo}/branches/{line}")
    except alpha.AlphaError as error:
        if "Not Found" not in str(error) and "404" not in str(error):
            raise FinalError(f"ветка {line} не прочитана: {error}") from error
    else:
        raise FinalError(f"ветка {line} уже есть: финальная версия этой линии уже выходила")
    newest = max(green, key=lambda run: run.get("created_at", ""))
    return Proof(rc_tag, commit, newest.get("html_url", ""))


def publish(root: Path, repo: str, rc_tag: str, *, dry_run: bool, gh=alpha.gh_api, runner=alpha.run) -> str:
    def git(arguments: list[str]) -> str:
        return runner(["git", *arguments], cwd=root)

    proof = prove(repo, rc_tag, gh, git)
    version = rc.RC.match(rc_tag)["core"]
    tag = f"v{version}"
    print(f"{tag}: из {rc_tag}, коммит {proof.commit[:7]}, выпуск {proof.run_url}")
    if dry_run:
        print("проба: коммит и тег не выполнялись")
        return tag
    python = sys.executable
    git(["checkout", "--detach", proof.commit])
    runner([python, "scripts/dev/bump-version.py", version], cwd=root)
    runner(["cargo", "update", "--workspace"], cwd=root)
    runner([python, "scripts/ci/check-version-contract.py", "--expected", version], cwd=root)
    identity = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com"]
    git([*identity, "commit", "--all", "--message", f"{version}: кандидат {rc_tag} ({proof.commit})"])
    git([*identity, "tag", "--annotate", tag, "--message",
         f"Unica {tag} из кандидата {rc_tag}: коммит {proof.commit}, выпуск {proof.run_url}"])
    try:
        with tag_push.ssh_remote(repo) as (remote, env):
            # Тег и ветка линии уходят вместе: тега нет без линии, из которой
            # выйдут исправительные, и линии нет без выпуска, который её открыл.
            runner(["git", "push", "--atomic", remote, f"refs/tags/{tag}",
                    f"HEAD:refs/heads/{line_branch(version)}"], cwd=root, env=env)
    except tag_push.TagKeyMissing as error:
        raise FinalError(str(error)) from error
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"tag={tag}\n")
    return tag


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "IngvarConsulting/unica"))
    commands = parser.add_subparsers(dest="command", required=True)
    publish_command = commands.add_parser("publish", help="поставить финальную версию из кандидата")
    publish_command.add_argument("--rc", required=True, help="тег кандидата, например v0.15.0-rc.2")
    publish_command.add_argument("--root", type=Path, default=Path("."))
    publish_command.add_argument("--dry-run", action="store_true", help="проверить и назвать версию, ничего не меняя")
    args = parser.parse_args(argv)
    try:
        publish(args.root, args.repo, args.rc, dry_run=args.dry_run)
    except (FinalError, alpha.AlphaError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
