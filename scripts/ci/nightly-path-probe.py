#!/usr/bin/env python3
"""Пройти ночной путь Rust в записи очереди, если она меняет CI.

Правка `scripts/ci/`, `.github/workflows/` или `.config/nextest.toml` меняет то, что
исполняет только ночь: план полос, сборку архива, `nextest list` по нему и учёт тестов
против инвентаря прошлой ночи. Тесты очереди этот путь не проходят, и поломка в нём
видна только утром. Здесь он идёт в лёгком виде на Linux:

1. план полос профиля ночи;
2. сборка одной маленькой полосы теми же `run-tests.py --phase build`, что ночью:
   архив, `nextest list` по нему с ночными аргументами, сверка разбиения, манифест;
3. состав полосы `ordinary` тем же отбором и профилем nextest, что ночью, но без
   архива: её бинари уже собрал шаг `Тесты Rust` очереди;
4. учёт: обе конфигурации Linux против базы ночи (`rust_inventory_baseline.py`)
   по `compare_inventory`.

Тесты не исполняются: их гоняют шаг `Тесты Rust` очереди и сама ночь. Поломки,
которые видны только на macOS и Windows, остаются ночи.
"""

from __future__ import annotations

import argparse
import copy
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ci_rust_lanes

ROOT = Path(__file__).resolve().parents[2]
# Пути, правка которых меняет ночной путь, а тестами очереди не исполняется.
PROBED = re.compile(r"^(scripts/ci/|\.github/workflows/|\.config/nextest\.toml$)")
PROFILE = "large"
HOST = ci_rust_lanes.LINUX
# Самая маленькая полоса ночи: одна библиотека `ureq`. Архив и `nextest list`
# у всех полос идут одними функциями, поэтому их поломку она показывает.
LANE = "vendor"
# Основная полоса: её учёт ломался чаще всего, а бинари очередь уже собрала.
LISTED = "ordinary"


def touched(paths: list[str]) -> list[str]:
    return sorted(path for path in paths if PROBED.match(path))


def changed_paths(base: str, head: str, root: Path = ROOT) -> list[str] | None:
    """Пути diff; `None`, если diff не прочитан: тогда путь проверяется."""
    completed = subprocess.run(["git", "diff", "--name-only", "--no-renames", base, head, "--"], cwd=root,
                               capture_output=True, text=True, encoding="utf-8", errors="replace")
    if completed.returncode != 0:
        print(f"diff {base}..{head} не прочитан: {completed.stderr.strip()}", flush=True)
        return None
    return [path for path in completed.stdout.split("\n") if path]


def plan(profile: str = PROFILE, host: str = HOST, lane: str = LANE) -> dict[str, tuple[str, ...]]:
    """Полосы каждого раннера ночи; проверяемая полоса обязана в нём быть."""
    planned = {runner: ci_rust_lanes.lanes(profile, runner) for runner in ci_rust_lanes.profile_hosts(profile)}
    if lane not in planned.get(host, ()):
        raise ValueError(f"в плане {profile} нет полосы {host}/{lane}: {planned}")
    return planned


def narrowed(baseline: dict, configurations: set[tuple[str, str]]) -> dict:
    """База только с проверяемыми конфигурациями: остальные здесь не строятся."""
    document = copy.deepcopy(baseline)
    document.pop("previousPlans", None)
    inventory = document.get("inventory")
    if isinstance(inventory, dict) and isinstance(inventory.get("configurations"), list):
        inventory["configurations"] = [item for item in inventory["configurations"]
                                       if (item.get("runner"), item.get("lane")) in configurations]
        inventory["jobs"] = None
    return document


def listing_command(profile: str = PROFILE, host: str = HOST, lane: str = LISTED) -> list[str]:
    """`nextest list` полосы с отбором и профилем ночи, по собранным бинарям."""
    return ["cargo", "nextest", "list", *ci_rust_lanes.selection(lane), "--locked",
            "--profile", ci_rust_lanes.nextest_profile(profile, lane, host),
            "--run-ignored", "all", "--message-format", "json"]


def listed_configuration(listed: dict, identity: dict, *, profile: str = PROFILE, host: str = HOST,
                         lane: str = LISTED) -> dict:
    """Конфигурация из `nextest list` в той форме, которую сверка ночи читает из манифеста."""
    cases = ci_rust_lanes.inventory(listed)
    return {"runner": host, "lane": lane, "identity": identity, "inventory": cases, "shards": [cases],
            "selection": list(ci_rust_lanes.selection(lane)),
            "nextestProfile": ci_rust_lanes.nextest_profile(profile, lane, host),
            "binaries": ci_rust_lanes.archive_binaries(listed)}


def account(manifest: dict, baseline: dict, listed: dict | None = None, *, profile: str = PROFILE,
            host: str = HOST, lane: str = LANE) -> list[str]:
    """Отказы учёта для проверенных полос, как их назвала бы сверка ночи."""
    sha = manifest.get("identity", {}).get("sha")
    ci_rust_lanes.validate_manifest(manifest, profile=profile, host=host, lane=lane, sha=sha)
    configurations = [manifest] + ([] if listed is None else [listed])
    current = ci_rust_lanes.inventory_summary(configurations, profile=profile, sha=sha, jobs=None)
    checked = {(item["runner"], item["lane"]) for item in configurations}
    problems, _ = ci_rust_lanes.compare_inventory(narrowed(baseline, checked), current)
    return problems


def python(script: str, *arguments: str) -> list[str]:
    return [sys.executable, str(ROOT / "scripts" / "ci" / script), *arguments]


def build_command(work: Path) -> list[str]:
    return python("run-tests.py", "--profile", PROFILE, "--ecosystem", "rust", "--phase", "build",
                  "--prepared", str(work / "lanes"), "--results", str(work / "report"),
                  "--runner", HOST, "--lane", LANE)


def baseline_command(work: Path) -> list[str]:
    return python("rust_inventory_baseline.py", "--profile", PROFILE, "--runners", json.dumps([HOST]),
                  "--out", str(work / "baseline.json"))


def run(command: list[str]) -> int:
    print("+ " + " ".join(command), flush=True)
    return subprocess.run(command, cwd=ROOT, check=False).returncode


def capture(command: list[str]) -> tuple[int, str]:
    print("+ " + " ".join(command), flush=True)
    completed = subprocess.run(command, cwd=ROOT, check=False, stdout=subprocess.PIPE, text=True, encoding="utf-8")
    return completed.returncode, completed.stdout


def probe(work: Path, execute=run, read=capture) -> list[str]:
    """Пройти путь; вернуть отказы. Первый отказ сборки останавливает учёт."""
    planned = plan()
    print("план ночи: " + "; ".join(f"{runner}: {', '.join(lanes)}" for runner, lanes in planned.items()), flush=True)
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)
    if execute(build_command(work)) != 0:
        return [f"{HOST}/{LANE}: архив ночи не собран или не перечислен"]
    manifest_path = work / "lanes" / LANE / "lane.json"
    if not manifest_path.is_file():
        return [f"{HOST}/{LANE}: сборка не оставила манифест полосы"]
    if execute(baseline_command(work)) != 0 or not (work / "baseline.json").is_file():
        return ["инвентарь прошлой ночи не найден: учёт не сверен"]
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    baseline = json.loads((work / "baseline.json").read_text(encoding="utf-8"))
    code, output = read(listing_command())
    if code != 0:
        return [f"{HOST}/{LISTED}: состав полосы не перечислен"]
    try:
        listed = listed_configuration(json.loads(output), manifest["identity"])
        return account(manifest, baseline, listed)
    except (KeyError, TypeError, ValueError) as error:
        return [f"учёт не сверен: {error}"]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--base", required=True)
    parser.add_argument("--head", required=True)
    parser.add_argument("--work", type=Path, default=ROOT / ".build" / "nightly-probe")
    args = parser.parse_args(argv)
    paths = changed_paths(args.base, args.head)
    hits = touched(paths) if paths is not None else ["diff не прочитан"]
    if not hits:
        print("CI не меняется: ночной путь не проверяется", flush=True)
        return 0
    print("ночной путь проверяется: " + ", ".join(hits[:10]) + (" …" if len(hits) > 10 else ""), flush=True)
    problems = probe(args.work)
    for problem in problems:
        print(f"::error::{problem}", flush=True)
    if os.environ.get("GITHUB_STEP_SUMMARY") and problems:
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("### Ночной путь\n\n" + "\n".join(f"- {problem}" for problem in problems) + "\n")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
