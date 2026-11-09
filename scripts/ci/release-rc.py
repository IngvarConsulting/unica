#!/usr/bin/env python3
"""Кандидат `vX.Y.Z-rc.N` из проверенной alpha: служебный коммит и тег ставит workflow.

Alpha — срез `main`, который ночь проверила полным набором на всех ОС. Кандидат
берёт тот же срез, родитель служебного коммита alpha, и ставит поверх свой
служебный коммит, где меняется только версия. Перед этим проверяется:

- alpha опубликована, её срез лежит в `main` и совпадает с `main@…` описания;
- ночь, поставившая эту alpha, зелёная, и проверила она коммит среза;
- дерево среза равно дереву, которое проверила ночь;
- выпуска `vX.Y.Z` ещё нет: кандидат не может быть младше стабильной версии.

Номер `N` — следующий после существующих тегов `vX.Y.Z-rc.*`. Тег ставит ключ
развёртывания из окружения `marketplace` (`tag_push`), и его push запускает
обычный выпуск по тегу: тесты, сборку, проверки установки и канал `next`.

Кнопку нажимает администратор репозитория: `admit` сверяет роль запустившего
до задания с ключом, а не его имя.

    python scripts/ci/release-rc.py admit --actor "$GITHUB_TRIGGERING_ACTOR"
    python scripts/ci/release-rc.py publish --alpha v0.15.0-alpha.4
"""

from __future__ import annotations

import argparse
import importlib.util
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tag_push  # noqa: E402

_SPEC = importlib.util.spec_from_file_location("release_alpha", Path(__file__).with_name("release-alpha.py"))
alpha = importlib.util.module_from_spec(_SPEC)
sys.modules.setdefault("release_alpha", alpha)
_SPEC.loader.exec_module(alpha)

RC = re.compile(r"\Av(?P<core>\d+\.\d+\.\d+)-rc\.(?P<number>0|[1-9]\d*)\Z")
NIGHTLY = "nightly.yml"
ALPHA_JOB = "Alpha — служебный коммит и тег"


class RcError(RuntimeError):
    pass


@dataclass(frozen=True)
class Proof:
    alpha: str
    slice: str
    run_url: str


def next_rc(core: str, tags: list[str]) -> str:
    """Следующий номер кандидата ядра; отказ, если ядро уже выпущено."""
    if f"v{core}" in tags:
        raise RcError(f"выпуск v{core} уже вышел: кандидат был бы младше него")
    numbers = [int(match["number"]) for tag in tags if (match := RC.match(tag)) and match["core"] == core]
    return f"v{core}-rc.{max(numbers, default=0) + 1}"


def prove(repo: str, alpha_tag: str, gh, git) -> Proof:
    """Доказательство, что срез alpha проверен зелёной ночью; иначе отказ."""
    match = alpha.ALPHA.match(alpha_tag)
    if match is None:
        raise RcError(f"не тег alpha: {alpha_tag}")
    release = gh(f"repos/{repo}/releases/tags/{alpha_tag}") or {}
    if not release or release.get("draft"):
        raise RcError(f"alpha {alpha_tag} не опубликована")
    slice_sha = git(["rev-parse", f"{alpha_tag}^{{commit}}^"]).strip()
    named = re.search(r"main@([0-9a-f]{40})", str(release.get("body") or ""))
    if named is None or named[1] != slice_sha:
        raise RcError(f"срез {alpha_tag} — {slice_sha[:7]}, а описание релиза называет другой")
    try:
        git(["merge-base", "--is-ancestor", slice_sha, "origin/main"])
    except alpha.AlphaError as error:
        raise RcError(f"срез {slice_sha[:7]} не лежит в main") from error
    runs = (gh(f"repos/{repo}/actions/workflows/{NIGHTLY}/runs?head_sha={slice_sha}&per_page=100") or {})
    runs = runs.get("workflow_runs", []) if isinstance(runs, dict) else [
        run for page in runs for run in page.get("workflow_runs", [])]
    for run in sorted(runs, key=lambda item: item.get("created_at", ""), reverse=True):
        if run.get("conclusion") != "success" or run.get("head_sha") != slice_sha:
            continue
        jobs = gh(f"repos/{repo}/actions/runs/{run['id']}/jobs?per_page=100") or {}
        jobs = jobs.get("jobs", []) if isinstance(jobs, dict) else [job for page in jobs for job in page.get("jobs", [])]
        if any(job.get("name", "").endswith(ALPHA_JOB) and job.get("conclusion") == "success" for job in jobs):
            checked = git(["rev-parse", f"{run['head_sha']}^{{tree}}"]).strip()
            if checked != git(["rev-parse", f"{slice_sha}^{{tree}}"]).strip():
                raise RcError("дерево среза не равно дереву, которое проверила ночь")
            return Proof(alpha_tag, slice_sha, run.get("html_url", ""))
    raise RcError(f"нет зелёной ночи, выпустившей {alpha_tag} на срезе {slice_sha[:7]}")


def admit(repo: str, actor: str, gh=alpha.gh_api) -> None:
    """Кандидата ставит только администратор репозитория; роль, а не имя."""
    if not actor:
        raise RcError("не назван запустивший: кандидата ставит только администратор репозитория")
    try:
        answer = gh(f"repos/{repo}/collaborators/{actor}/permission")
    except alpha.AlphaError as error:
        raise RcError(f"{actor}: роль в репозитории не прочитана, кандидата ставит только администратор: {error}") from error
    permission = answer.get("permission") if isinstance(answer, dict) else None
    if permission != "admin":
        raise RcError(
            f"{actor}: роль в репозитории {permission or 'неизвестна'}, а кандидата из alpha ставит "
            "только администратор. Попросите администратора нажать кнопку"
        )
    print(f"{actor}: администратор, кандидата ставить можно")


def publish(root: Path, repo: str, alpha_tag: str, *, dry_run: bool, gh=alpha.gh_api, runner=alpha.run) -> str:
    def git(arguments: list[str]) -> str:
        return runner(["git", *arguments], cwd=root)

    proof = prove(repo, alpha_tag, gh, git)
    core = alpha.ALPHA.match(alpha_tag)["core"]
    tag = next_rc(core, alpha.version_tags(repo, gh))
    print(f"{tag}: из {alpha_tag}, срез {proof.slice[:7]}, ночь {proof.run_url}")
    if dry_run:
        print("проба: коммит и тег не выполнялись")
        return tag
    version = tag.removeprefix("v")
    python = sys.executable
    git(["checkout", "--detach", proof.slice])
    runner([python, "scripts/dev/bump-version.py", version], cwd=root)
    runner(["cargo", "update", "--workspace"], cwd=root)
    runner([python, "scripts/ci/check-version-contract.py", "--expected", version], cwd=root)
    identity = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com"]
    git([*identity, "commit", "--all", "--message", f"rc {version}: срез main {proof.slice} из {alpha_tag}"])
    git([*identity, "tag", "--annotate", tag, "--message",
         f"Кандидат из {alpha_tag}: срез main {proof.slice}, проверен ночью {proof.run_url}"])
    try:
        with tag_push.ssh_remote(repo) as (remote, env):
            runner(["git", "push", remote, f"refs/tags/{tag}"], cwd=root, env=env)
    except tag_push.TagKeyMissing as error:
        raise RcError(str(error)) from error
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"tag={tag}\n")
    return tag


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "IngvarConsulting/unica"))
    commands = parser.add_subparsers(dest="command", required=True)
    admit_command = commands.add_parser("admit", help="пустить к кнопке только администратора репозитория")
    admit_command.add_argument("--actor", required=True, help="запустивший прогон")
    publish_command = commands.add_parser("publish", help="поставить кандидата из проверенной alpha")
    publish_command.add_argument("--alpha", required=True, help="тег alpha, например v0.15.0-alpha.4")
    publish_command.add_argument("--root", type=Path, default=Path("."))
    publish_command.add_argument("--dry-run", action="store_true", help="проверить и назвать номер, ничего не меняя")
    args = parser.parse_args(argv)
    try:
        if args.command == "admit":
            admit(args.repo, args.actor)
        else:
            publish(args.root, args.repo, args.alpha, dry_run=args.dry_run)
    except (RcError, alpha.AlphaError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
