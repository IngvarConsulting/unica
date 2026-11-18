#!/usr/bin/env python3
"""Выпуск alpha: срез main под номером `vX.Y.Z-alpha.N` вне истории main.

`publish` берёт ядро `X.Y.Z` из версии main, поднимает номер, делает служебный
коммит с этой версией поверх вершины main и ставит на него тег. В main ничего не
попадает. Сборку на этом коммите делает тот же прогон `release.yml`: тег и коммит
уходят ему выводами шага. Родитель служебного коммита и есть срез main, поэтому
прошлый срез читается из самого тега. Срез, у которого уже есть alpha, второй раз
не выпускается. Ядро, вышедшее полной версией, тоже: такой alpha был бы младше
своего выпуска, и выпускать нечего, пока main не поднимет версию.

`prune` оставляет последние опубликованные alpha и удаляет релизы остальных.
Теги остаются: номер следующей alpha и выпущенный срез читаются из тегов, поэтому
номер только растёт, а ruleset может запретить всем удаление и переписывание
тегов `v*`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tag_push  # noqa: E402

ALPHA = re.compile(r"\Av(?P<core>\d+\.\d+\.\d+)-alpha\.(?P<number>0|[1-9]\d*)\Z")
CORE = re.compile(r'(?m)^\[workspace\.package\](?:\n(?!\[).*)*?\nversion = "(\d+\.\d+\.\d+)')
KEEP = 2


class AlphaError(RuntimeError):
    pass


class RuleRefused(AlphaError):
    pass


Gh = Callable[..., object]


def run(command: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> str:
    completed = subprocess.run(
        command, cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8", check=False
    )
    if completed.returncode != 0:
        raise AlphaError(f"{command[0]} {command[1]} завершился с кодом {completed.returncode}: {completed.stderr.strip()}")
    return completed.stdout


def gh_api(path: str, *, method: str = "GET") -> object:
    """Один запрос к GitHub; ответ со списком собирается со всех страниц."""

    command = ["gh", "api", "--method", method, path]
    if method == "GET":
        command.append("--paginate")
    completed = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", check=False)
    if completed.returncode != 0:
        detail = f"{completed.stdout} {completed.stderr}".strip()
        if "Repository rule violations" in detail:
            raise RuleRefused(detail)
        raise AlphaError(f"gh api {method} {path}: {detail}")
    if not completed.stdout.strip():
        return None
    # `--paginate` склеивает страницы списка подряд: `[...][...]`.
    decoder = json.JSONDecoder()
    text, position, pages = completed.stdout, 0, []
    while position < len(text):
        value, position = decoder.raw_decode(text, position)
        pages.append(value)
        while position < len(text) and text[position].isspace():
            position += 1
    if len(pages) == 1:
        return pages[0]
    return [item for page in pages for item in page]


def alpha_key(tag: str) -> tuple[tuple[int, ...], int]:
    match = ALPHA.match(tag)
    if match is None:
        raise AlphaError(f"не тег alpha: {tag}")
    return tuple(int(part) for part in match["core"].split(".")), int(match["number"])


def core_version(cargo_toml: str) -> str:
    match = CORE.search(cargo_toml)
    if match is None:
        raise AlphaError("в Cargo.toml нет версии workspace.package")
    return match[1]


def version_tags(repo: str, gh: Gh) -> list[str]:
    refs = gh(f"repos/{repo}/git/matching-refs/tags/v") or []
    return [ref["ref"].removeprefix("refs/tags/") for ref in refs]


def alpha_tags(tags: list[str]) -> list[str]:
    return sorted((tag for tag in tags if ALPHA.match(tag)), key=alpha_key)


def tag_parent(repo: str, tag: str, gh: Gh) -> str | None:
    commit = gh(f"repos/{repo}/commits/{tag}")
    parents = commit.get("parents", [])
    return parents[0]["sha"] if parents else None


@dataclass(frozen=True)
class Plan:
    tag: str | None
    reason: str


def plan_next(core: str, head: str, tags: list[str], parent_of: Callable[[str], str | None], *, force: bool = False) -> Plan:
    if f"v{core}" in tags:
        return Plan(None, f"вышел выпуск v{core}: alpha этого ядра был бы младше него, поднимите версию main")
    alphas = alpha_tags(tags)
    if alphas and not force and parent_of(alphas[-1]) == head:
        return Plan(None, f"срез {head[:7]} уже выпущен как {alphas[-1]}")
    numbers = [alpha_key(tag)[1] for tag in alphas if ALPHA.match(tag)["core"] == core]
    return Plan(f"v{core}-alpha.{max(numbers, default=0) + 1}", f"срез {head[:7]}")


def publish(root: Path, repo: str, *, force: bool, dry_run: bool, gh: Gh = gh_api, runner=run) -> str | None:
    head = runner(["git", "rev-parse", "HEAD"], cwd=root).strip()
    core = core_version((root / "Cargo.toml").read_text(encoding="utf-8"))
    plan = plan_next(core, head, version_tags(repo, gh), lambda tag: tag_parent(repo, tag, gh), force=force)
    if plan.tag is None:
        print(f"alpha не нужен: {plan.reason}")
        return None
    print(f"{plan.tag}: {plan.reason}")
    if dry_run:
        print("проба: коммит и тег не выполнялись")
        return plan.tag
    version = plan.tag.removeprefix("v")
    python = sys.executable
    runner([python, "scripts/dev/bump-version.py", version], cwd=root)
    # Сеть есть, а кэша реестра на чистом раннере может не быть, поэтому без --offline.
    runner(["cargo", "update", "--workspace"], cwd=root)
    runner([python, "scripts/ci/check-version-contract.py", "--expected", version], cwd=root)
    identity = ["-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com"]
    runner(["git", *identity, "commit", "--all", "--message", f"alpha {version}: срез main {head}"], cwd=root)
    runner(["git", *identity, "tag", "--annotate", plan.tag, "--message", f"Срез main {head}"], cwd=root)
    # Тег ставит ключ из секрета окружения `alpha`, а не токен прогона: теги `v*`
    # защищены ruleset. Push ключом будит `on: push`, поэтому `release.yml` не
    # слушает теги alpha: собирает этот же прогон.
    try:
        with tag_push.ssh_remote(repo) as (remote, env):
            runner(["git", "push", remote, f"refs/tags/{plan.tag}"], cwd=root, env=env)
    except tag_push.TagKeyMissing as error:
        raise AlphaError(str(error)) from error
    outputs = os.environ.get("GITHUB_OUTPUT")
    if outputs:
        sha = runner(["git", "rev-parse", "HEAD"], cwd=root).strip()
        with open(outputs, "a", encoding="utf-8") as stream:
            stream.write(f"tag={plan.tag}\nsha={sha}\n")
    return plan.tag


def stale_alphas(tags: list[str], published: set[str], keep: int) -> list[str]:
    """Чьи релизы удалить: alpha старше последних `keep` опубликованных.

    Alpha новее них, возможно, ещё собирается. Удаляется только релиз: тег
    остаётся, поэтому номер следующей alpha растёт дальше и не повторяется.
    """

    alphas = alpha_tags(tags)
    kept = [tag for tag in alphas if tag in published][-keep:]
    if len(kept) < keep:
        return []
    return [tag for tag in alphas if alpha_key(tag) < alpha_key(kept[0])]


def prune(repo: str, *, keep: int, dry_run: bool, gh: Gh = gh_api) -> list[str]:
    """Удалить релизы старых alpha; их теги не трогаются.

    Номер следующей alpha и «срез уже выпущен» читаются из тегов, поэтому
    после удаления релиза номер не повторяется. Тегу ruleset ничего не
    разрешает: удаление и переписывание тегов `v*` можно запретить всем.
    """

    releases = {release["tag_name"]: release for release in gh(f"repos/{repo}/releases") or []}
    published = {tag for tag, release in releases.items() if not release.get("draft")}
    stale = [tag for tag in stale_alphas(version_tags(repo, gh), published, keep) if tag in releases]
    for tag in stale:
        if dry_run:
            print(f"проба: удалился бы релиз {tag}, тег остаётся")
            continue
        gh(f"repos/{repo}/releases/{releases[tag]['id']}", method="DELETE")
        print(f"удалён релиз {tag}, тег остаётся")
    if not stale:
        print("удалять нечего")
    return [] if dry_run else stale


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", "IngvarConsulting/unica"))
    commands = parser.add_subparsers(dest="command", required=True)
    publish_command = commands.add_parser("publish", help="выпустить alpha с вершины текущего checkout")
    publish_command.add_argument("--root", type=Path, default=Path("."))
    publish_command.add_argument("--force", action="store_true", help="выпустить, даже если срез уже выпущен")
    publish_command.add_argument("--dry-run", action="store_true", help="назвать номер и ничего не менять")
    prune_command = commands.add_parser("prune", help="удалить релизы старых alpha, теги оставить")
    prune_command.add_argument("--keep", type=int, default=KEEP)
    prune_command.add_argument("--dry-run", action="store_true", help="назвать, что удалилось бы")
    args = parser.parse_args(argv)
    try:
        if args.command == "publish":
            publish(args.root, args.repo, force=args.force, dry_run=args.dry_run)
        else:
            prune(args.repo, keep=args.keep, dry_run=args.dry_run)
    except AlphaError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
