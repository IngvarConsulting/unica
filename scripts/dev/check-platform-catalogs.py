#!/usr/bin/env python3
"""Сверка закреплённых каталогов справки с установленными сборками платформы.

Оракулы справки живут в `crates/unica-documentation-sources` и помечены
`#[ignore]`: справки на раннерах CI нет, поэтому в ворота они не входят.
Их никто не звал, и расхождение каталога с новой сборкой платформы оставалось
незамеченным. Этот скрипт их зовёт.

Каталог привязан к линии платформы (`platform-event-catalog-<линия>.json`),
а снят с одной её сборки: она записана внутри как `source.installationVersion`.
Скрипт находит установленные сборки, сопоставляет их линиям каталогов и
печатает по каждой линии один из трёх исходов:

- `прошло` — оракулы отработали на установленной сборке;
- `расхождение` — оракул упал либо установленная сборка линии не та, на которой
  каталог снят: такую сборку надо переснять или объяснить, почему не надо;
- `не проверено` — у линии есть закреплённый каталог, но установки этой линии
  на хосте нет, и сверять не с чем.

Третий исход не успех: закреплённый каталог остался без проверки. Коды выхода
различают все три: 0 — всё проверено и прошло, 1 — есть расхождение,
2 — расхождений нет, но какой-то каталог проверить было нечем.

Отдельно печатаются установленные линии, у которых закреплённого каталога нет.
Это не исход проверки и на код выхода не влияет: каталог им никто не обещал,
а молчать о них нельзя — иначе не видно, что поверхность уже шире проверенного.

    python scripts/dev/check-platform-catalogs.py
    python scripts/dev/check-platform-catalogs.py --platform-root <каталог>
"""

from __future__ import annotations

import argparse
import json
import os
import platform
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Sequence, Tuple


REPO_ROOT = Path(__file__).resolve().parents[2]
CATALOG_DIR = Path("crates/unica-metadata/src")
CATALOG_NAME = re.compile(r"^platform-event-catalog-(\d+\.\d+\.\d+)\.json$")
BUILD_NAME = re.compile(r"^(\d+\.\d+\.\d+)\.\d+$")
ORACLE_CRATE = "unica-documentation-sources"
# Те же корни, что у `host-preflight.py` и `default_platform_roots`.
PLATFORM_ROOTS = {
    "Windows": (r"C:\Program Files\1cv8", r"C:\Program Files (x86)\1cv8"),
    "Linux": ("/opt/1cv8/x86_64", "/opt/1cv8/i386", "/usr/local/1cv8"),
    "Darwin": ("/opt/1cv8",),
}
PASSED, DIVERGED, UNCHECKED = "прошло", "расхождение", "не проверено"


@dataclass
class Catalog:
    line: str
    pinned_build: str
    path: Path


@dataclass
class Outcome:
    line: str
    state: str
    detail: str


def catalogs(root: Path) -> List[Catalog]:
    found = []
    for path in sorted((root / CATALOG_DIR).glob("platform-event-catalog-*.json")):
        match = CATALOG_NAME.match(path.name)
        if not match:
            continue
        try:
            source = json.loads(path.read_text(encoding="utf-8"))["source"]
        except (OSError, ValueError, KeyError) as error:
            raise SystemExit(f"check-platform-catalogs: {path.name}: {error}")
        found.append(Catalog(match.group(1), source["installationVersion"], path))
    return found


def line_of(build: str) -> Optional[str]:
    match = BUILD_NAME.match(build)
    return match.group(1) if match else None


def installations(roots: Sequence[str]) -> Dict[str, Path]:
    """Сборка -> каталог установки; у сборки без справки каталога нет."""
    found: Dict[str, Path] = {}
    for raw in roots:
        root = Path(raw)
        if not root.is_dir():
            continue
        for child in sorted(root.iterdir()):
            if child.is_dir() and BUILD_NAME.match(child.name):
                found.setdefault(child.name, child)
    return found


def default_roots() -> Sequence[str]:
    return PLATFORM_ROOTS.get(platform.system(), PLATFORM_ROOTS["Linux"])


def run_oracles(root: Path, installation: Path, env: dict, runner) -> Tuple[bool, str]:
    """Прогнать ignored-оракулы крейта справки на одной установке."""
    command = [
        "cargo",
        "test",
        "-p",
        ORACLE_CRATE,
        "--lib",
        "--",
        "--ignored",
        "platform_help",
    ]
    done = runner(command, root, {**env, "UNICA_PLATFORM_HELP_DIR": str(installation)})
    return done.returncode == 0, (done.stdout or "") + (done.stderr or "")


def failing_lines(output: str) -> List[str]:
    keep = [line.strip() for line in output.splitlines() if line.strip()]
    named = [line for line in keep if line.startswith("test ") and "FAILED" in line]
    panics = [line for line in keep if "panicked at" in line or "differs" in line]
    return named + panics[:3]


def plan(found: Sequence[Catalog], installed: Dict[str, Path]) -> List[Tuple[Catalog, Optional[Path], str]]:
    """По каталогу: на какой установке его сверять и почему именно на ней."""
    work = []
    for catalog in found:
        same_line = {build: path for build, path in installed.items() if line_of(build) == catalog.line}
        if not same_line:
            work.append((catalog, None, f"установки линии {catalog.line} нет"))
        elif catalog.pinned_build in same_line:
            work.append((catalog, same_line[catalog.pinned_build], catalog.pinned_build))
        else:
            newest = sorted(same_line)[-1]
            work.append((catalog, same_line[newest], f"закреплена {catalog.pinned_build}, установлена {newest}"))
    return work


def report(outcomes: Sequence[Outcome], orphan_lines: Sequence[str], echo) -> int:
    for outcome in outcomes:
        echo(f"[{outcome.state}] {outcome.line}: {outcome.detail}")
    if orphan_lines:
        echo("")
        echo(f"Линии без закреплённого каталога: {', '.join(orphan_lines)}")
    diverged = [outcome for outcome in outcomes if outcome.state == DIVERGED]
    unchecked = [outcome for outcome in outcomes if outcome.state == UNCHECKED]
    echo("")
    echo(f"Расхождений: {len(diverged) if diverged else 'нет'}")
    echo(f"Каталогов без проверки: {len(unchecked) if unchecked else 'нет'}")
    if diverged:
        return 1
    return 2 if unchecked else 0


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--platform-root", action="append", default=[], help="где искать установки платформы")
    args = parser.parse_args(argv)

    found = catalogs(REPO_ROOT)
    if not found:
        print("check-platform-catalogs: закреплённых каталогов нет", file=sys.stderr)
        return 2
    installed = installations(list(args.platform_root) + list(default_roots()))
    env = {**os.environ, "PYTHONUTF8": "1"}

    def runner(command, cwd, environment):
        return subprocess.run(
            command, cwd=str(cwd), env=environment, capture_output=True, text=True,
            encoding="utf-8", errors="replace",
        )

    outcomes = []
    for catalog, installation, note in plan(found, installed):
        if installation is None:
            outcomes.append(Outcome(catalog.line, UNCHECKED, note))
            continue
        if not note.startswith(catalog.pinned_build):
            outcomes.append(Outcome(catalog.line, DIVERGED, f"{note}: каталог на ней не проверен"))
            continue
        print(f"… оракулы справки на {installation}", file=sys.stderr)
        passed, output = run_oracles(REPO_ROOT, installation, env, runner)
        if passed:
            outcomes.append(Outcome(catalog.line, PASSED, f"сборка {note}"))
        else:
            detail = "; ".join(failing_lines(output)) or "оракул упал без разбираемого вывода"
            outcomes.append(Outcome(catalog.line, DIVERGED, detail))

    known = {catalog.line for catalog in found}
    orphans = sorted({line for build in installed if (line := line_of(build)) and line not in known})
    return report(outcomes, orphans, print)


if __name__ == "__main__":
    raise SystemExit(main())
