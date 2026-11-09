#!/usr/bin/env python3
"""Исправительная версия `vX.Y.Z` из головы линии `release-vX.Y`: тег ставит workflow, PR обратно в `main` — тоже.

Линию открывает финальная версия `vX.Y.0`: её кнопка создаёт ветку на коммите
выпуска. Исправления приходят в линию pull request через очередь. Кнопка берёт
голову линии и ставит поверх служебный коммит, где меняется только версия.
Перед этим проверяется:

- источник — ветка `release-vX.Y`, и у линии есть финальная версия;
- голова линии ещё не выпущена: она не коммит тега `vX.Y.*` и не срез под его
  служебным коммитом;
- на голове есть зелёный прогон CI: очередь линии (`ci.yml`) или проверка линии
  кнопкой `release.yml`.

Номер `Z` — следующий после существующих `vX.Y.*`. Ключ развёртывания из
окружения `alpha` (`tag_push`) ставит тег, и его push запускает обычный выпуск
по тегу: тесты, сборку, проверки установки и сразу основной каталог
маркетплейса. Служебный коммит живёт только под тегом: голова линии остаётся
срезом.

Затем `back-merge` открывает pull request линии обратно в `main` (шаг 3
runbook). Номер версии в линии и `main` расходится всегда: в пяти файлах
контракта версии и в `Cargo.lock` остаётся сторона `main`. Конфликт в другом
файле — отказ с перечнем: его решает человек.

    python scripts/ci/release-patch.py publish --line release-v0.15
    python scripts/ci/release-patch.py back-merge --tag v0.15.1
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import subprocess
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

LINE = re.compile(r"\Arelease-v(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\Z")
STABLE = re.compile(r"\Av(?P<major>\d+)\.(?P<minor>\d+)\.(?P<patch>\d+)\Z")
# Прогоны, чей зелёный итог на голове линии считается её CI.
CI_WORKFLOWS = (".github/workflows/ci.yml", ".github/workflows/release.yml")
# Контракт версии (`scripts/dev/bump-version.py`) и `Cargo.lock`: в линии меняется
# только номер, и при слиянии в `main` остаётся сторона `main`.
VERSION_FILES = (
    "Cargo.toml",
    "plugins/unica/.codex-plugin/plugin.json",
    "plugins/unica/.claude-plugin/plugin.json",
    "plugins/unica/.zcode-plugin/plugin.json",
    "plugins/unica/third-party/tools.lock.json",
    "Cargo.lock",
)
IDENTITY = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com"]


class PatchError(RuntimeError):
    pass


@dataclass(frozen=True)
class Proof:
    line: str
    head: str
    tag: str
    run_url: str


def line_tags(line: str, tags: list[str]) -> list[tuple[int, str]]:
    match = LINE.match(line)
    return sorted(
        (int(stable["patch"]), tag)
        for tag in tags
        if (stable := STABLE.match(tag)) and (stable["major"], stable["minor"]) == (match["major"], match["minor"])
    )


def prove(repo: str, line: str, gh, git) -> Proof:
    """Доказательство, что голову линии можно выпустить; иначе отказ."""
    match = LINE.match(line)
    if match is None:
        raise PatchError(f"исправительную ставят только из ветки release-vX.Y, а не из {line}")
    released = line_tags(line, alpha.version_tags(repo, gh))
    if not released:
        raise PatchError(f"у линии {line} нет финальной версии v{match['major']}.{match['minor']}.0")
    try:
        head = git(["rev-parse", "--verify", f"refs/remotes/origin/{line}^{{commit}}"]).strip()
    except alpha.AlphaError as error:
        raise PatchError(f"ветки {line} нет") from error
    for _, tag in released:
        commit = git(["rev-parse", f"{tag}^{{commit}}"]).strip()
        parent = git(["rev-parse", f"{commit}^"]).strip()
        if head in (commit, parent):
            raise PatchError(f"голова {line} ({head[:7]}) уже выпущена как {tag}: в линии нет новых исправлений")
    runs = rc_pages(gh(f"repos/{repo}/actions/runs?head_sha={head}&per_page=100"))
    green = [run for run in runs if run.get("head_sha") == head and run.get("conclusion") == "success"
             and str(run.get("path", "")).split("@", 1)[0] in CI_WORKFLOWS]
    if not green:
        raise PatchError(f"на голове {line} ({head[:7]}) нет зелёного CI: очереди линии или проверки кнопкой release.yml")
    newest = max(green, key=lambda run: run.get("created_at", ""))
    tag = f"v{match['major']}.{match['minor']}.{released[-1][0] + 1}"
    return Proof(line, head, tag, newest.get("html_url", ""))


def rc_pages(answer) -> list[dict]:
    if isinstance(answer, dict):
        return answer.get("workflow_runs", [])
    return [item for page in answer or [] for item in page.get("workflow_runs", [])]


def publish(root: Path, repo: str, line: str, *, dry_run: bool, gh=alpha.gh_api, runner=alpha.run) -> str:
    def git(arguments: list[str]) -> str:
        return runner(["git", *arguments], cwd=root)

    proof = prove(repo, line, gh, git)
    tag = proof.tag
    version = tag.removeprefix("v")
    print(f"{tag}: голова {line} {proof.head[:7]}, CI {proof.run_url}")
    if dry_run:
        print("проба: коммит и тег не выполнялись")
        return tag
    python = sys.executable
    git(["checkout", "--detach", proof.head])
    runner([python, "scripts/dev/bump-version.py", version], cwd=root)
    runner(["cargo", "update", "--workspace"], cwd=root)
    runner([python, "scripts/ci/check-version-contract.py", "--expected", version], cwd=root)
    git([*IDENTITY, "commit", "--all", "--message", f"{version}: голова {line} {proof.head}"])
    git([*IDENTITY, "tag", "--annotate", tag, "--message",
         f"Unica {tag} из {line}: голова {proof.head}, CI {proof.run_url}"])
    try:
        with tag_push.ssh_remote(repo) as (remote, env):
            runner(["git", "push", remote, f"refs/tags/{tag}"], cwd=root, env=env)
    except tag_push.TagKeyMissing as error:
        raise PatchError(str(error)) from error
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"tag={tag}\n")
    return tag


def merge_branch(tag: str) -> str:
    return f"merge/release-{tag}-into-main"


def unchecked(command: list[str], *, cwd: Path) -> str:
    """Команда, чей отказ — ответ, а не сбой: слияние с конфликтом, пустой diff."""
    return subprocess.run(command, cwd=cwd, capture_output=True, text=True, encoding="utf-8", check=False).stdout


def back_merge(root: Path, repo: str, tag: str, *, runner=alpha.run, probe=unchecked) -> str | None:
    """Ветка слияния линии в `main` и pull request; `None`, если вливать нечего."""
    stable = STABLE.match(tag)
    if stable is None:
        raise PatchError(f"не тег исправительной версии: {tag}")
    line = f"release-v{stable['major']}.{stable['minor']}"
    branch = merge_branch(tag)

    def git(arguments: list[str]) -> str:
        return runner(["git", *arguments], cwd=root)

    def look(arguments: list[str]) -> str:
        return probe(["git", *arguments], cwd=root)

    git(["fetch", "origin", "main", line])
    git(["checkout", "-B", branch, "origin/main"])
    look([*IDENTITY, "merge", "--no-ff", "--no-commit", f"origin/{line}"])
    conflicted = look(["diff", "--name-only", "--diff-filter=U"]).split()
    foreign = sorted(set(conflicted) - set(VERSION_FILES))
    if foreign:
        look(["merge", "--abort"])
        raise PatchError(
            f"{line} не вливается в main без человека: конфликт в {', '.join(foreign)}. "
            f"Влейте линию вручную веткой {branch}, оставив сторону main в файлах версии"
        )
    # Номер линии в main не идёт: во всех файлах версии остаётся сторона main.
    git(["checkout", "origin/main", "--", *VERSION_FILES])
    if not look(["diff", "--cached", "--name-only", "origin/main"]).strip():
        look(["merge", "--abort"])
        print(f"{line}: всё уже в main, pull request не нужен")
        return None
    title = f"Линия {line} после {tag} — обратно в main"
    git([*IDENTITY, "commit", "--no-edit", "--message", title])
    # Checkout не оставляет токен в дереве: push идёт через помощник `gh`.
    runner(["gh", "auth", "setup-git"], cwd=root)
    git(["push", "--force-with-lease", "origin", f"HEAD:refs/heads/{branch}"])
    body = "\n\n".join((
        f"Шаг 3 runbook: исправления линии `{line}`, выпущенные как `{tag}`, возвращаются в `main`.",
        "В файлах контракта версии и `Cargo.lock` оставлена сторона `main`: линия меняла там только номер.",
        "Ветку и pull request создал прогон кнопки исправительной версии токеном прогона, поэтому CI "
        "на нём сам не запустился: координатор закрывает и открывает pull request своим токеном "
        "или кладёт пустой коммит.",
    ))
    url = runner(["gh", "pr", "create", "--repo", repo, "--base", "main", "--head", branch,
                  "--title", title, "--body", body], cwd=root)
    print(url.strip())
    return branch


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "IngvarConsulting/unica"))
    commands = parser.add_subparsers(dest="command", required=True)
    publish_command = commands.add_parser("publish", help="поставить исправительную версию из головы линии")
    publish_command.add_argument("--line", required=True, help="ветка линии, например release-v0.15")
    publish_command.add_argument("--root", type=Path, default=Path("."))
    publish_command.add_argument("--dry-run", action="store_true", help="проверить и назвать версию, ничего не меняя")
    merge_command = commands.add_parser("back-merge", help="открыть pull request линии обратно в main")
    merge_command.add_argument("--tag", required=True, help="выпущенная исправительная версия, например v0.15.1")
    merge_command.add_argument("--root", type=Path, default=Path("."))
    args = parser.parse_args(argv)
    try:
        if args.command == "publish":
            publish(args.root, args.repo, args.line, dry_run=args.dry_run)
        else:
            back_merge(args.root, args.repo, args.tag)
    except (PatchError, alpha.AlphaError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
