#!/usr/bin/env python3
"""Подготовить проверку доставки движка на живом Claude Code.

Гейт фазы 3 требует, чтобы вызов к недоставленному движку возвращал результат,
а не отказ, и чтобы ход доставки был виден в хосте. Проверить это можно только
руками и только на опубликованной поставке: доставка ходит за архивом по адресу
из манифеста, а манифест прибит к релизам `IngvarConsulting/unica`.

Скрипт делает то, что можно сделать заранее: сверяет архив ядра в релизе Unica,
читает неизменяемый адрес движка из манифеста поставки, проверяет этот ассет в
релизе unica-toolchain, готовит пустой кеш и печатает сценарий проверки. Сам
вызов и наблюдение остаются за человеком — хост не автоматизируется.

Usage:
    prepare-delivery-probe.py                     # последний релиз, движок bsl-analyzer
    prepare-delivery-probe.py --tag v0.13.0 --engine rlm-tools-bsl
"""

from __future__ import annotations

import argparse
import json
import platform
import shutil
import subprocess
import sys
from pathlib import Path
from urllib.parse import urlparse

CORE_REPO = "IngvarConsulting/unica"
TOOLCHAIN_REPO = "IngvarConsulting/unica-toolchain"
REPO_ROOT = Path(__file__).resolve().parents[2]


def host_target() -> str:
    system, machine = platform.system(), platform.machine()
    if system == "Darwin" and machine in ("arm64", "aarch64"):
        return "darwin-arm64"
    if system == "Linux" and machine in ("x86_64", "amd64"):
        return "linux-x64"
    if system == "Windows" and machine in ("AMD64", "x86_64"):
        return "win-x64"
    raise SystemExit(f"цель не обслуживается: {system}-{machine}")


def release_assets(repository: str, tag: str | None) -> "tuple[str, list[str]]":
    command = [
        "gh",
        "release",
        "view",
        "--repo",
        repository,
        "--json",
        "tagName,assets",
    ]
    if tag:
        command.insert(3, tag)
    finished = subprocess.run(command, capture_output=True, text=True, check=False)
    if finished.returncode != 0:
        raise SystemExit(f"не удалось прочитать релиз: {finished.stderr.strip()}")
    published = json.loads(finished.stdout)
    return published["tagName"], [asset["name"] for asset in published["assets"]]


def engine_delivery(
    manifest_path: Path, engine: str, target: str
) -> "tuple[str, str, str, str]":
    """Return version, checksum, toolchain tag and asset from a pinned manifest."""
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    artifact = (manifest.get("artifacts") or {}).get(engine)
    if not isinstance(artifact, dict) or artifact.get("role") != "engine":
        raise SystemExit(f"манифест не объявляет движок {engine}")
    version = artifact.get("version")
    target_entry = (artifact.get("targets") or {}).get(target)
    asset = target_entry.get("asset") if isinstance(target_entry, dict) else None
    if not isinstance(version, str) or not version or not isinstance(asset, dict):
        raise SystemExit(f"манифест не описывает поставку {engine} для {target}")

    name = asset.get("name")
    url = asset.get("url")
    sha256 = asset.get("sha256")
    if (
        not isinstance(name, str)
        or not name
        or "/" in name
        or not isinstance(url, str)
        or not isinstance(sha256, str)
        or len(sha256) != 64
        or any(character not in "0123456789abcdef" for character in sha256)
    ):
        raise SystemExit(f"манифест содержит некорректный ассет {engine} для {target}")

    parsed = urlparse(url)
    prefix = "/IngvarConsulting/unica-toolchain/releases/download/"
    if (
        parsed.scheme != "https"
        or parsed.netloc != "github.com"
        or not parsed.path.startswith(prefix)
        or parsed.query
        or parsed.fragment
    ):
        raise SystemExit(
            f"движок {engine} должен происходить из релиза unica-toolchain"
        )
    remainder = parsed.path.removeprefix(prefix)
    parts = remainder.split("/")
    if len(parts) != 2 or not parts[0] or parts[1] != name:
        raise SystemExit(f"адрес ассета {engine} не совпадает с его именем")
    return version, sha256, parts[0], name


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", help="релиз; по умолчанию последний")
    parser.add_argument("--engine", default="bsl-analyzer", help="артефакт движка")
    parser.add_argument(
        "--manifest",
        type=Path,
        default=REPO_ROOT / "plugins" / "unica" / "runtime-manifest.json",
        help="runtime-manifest.json из проверяемого пакета",
    )
    parser.add_argument(
        "--cache",
        type=Path,
        default=Path.home() / ".claude" / "unica" / "runtimes",
        help="кеш артефактов Claude Code",
    )
    parser.add_argument(
        "--wipe",
        action="store_true",
        help="удалить кеш, чтобы движка заведомо не было",
    )
    arguments = parser.parse_args()

    target = host_target()
    version, engine_sha, toolchain_tag, engine_asset = engine_delivery(
        arguments.manifest, arguments.engine, target
    )
    tag, core_assets = release_assets(CORE_REPO, arguments.tag)
    _, toolchain_assets = release_assets(TOOLCHAIN_REPO, toolchain_tag)
    core_asset = f"unica-runtime-{target}.tar.gz"

    print(f"релиз {tag}, цель {target}")
    core_state = "есть" if core_asset in core_assets else "НЕТ"
    print(f"  ядро ({CORE_REPO}): {core_asset}: {core_state}")
    print(
        f"  движок ({TOOLCHAIN_REPO}@{toolchain_tag}): {engine_asset}: "
        f"{'есть' if engine_asset in toolchain_assets else 'НЕТ'}"
    )

    if core_asset not in core_assets or engine_asset not in toolchain_assets:
        print()
        print("Проверку провести нечем: один из закреплённых ассетов не опубликован.")
        print(f"Ассеты {CORE_REPO}@{tag}:")
        for name in sorted(core_assets):
            print(f"  {name}")
        print(f"Ассеты {TOOLCHAIN_REPO}@{toolchain_tag}:")
        for name in sorted(toolchain_assets):
            print(f"  {name}")
        return 2

    if arguments.wipe and arguments.cache.exists():
        shutil.rmtree(arguments.cache)
        print(f"кеш удалён: {arguments.cache}")
    arguments.cache.mkdir(parents=True, exist_ok=True)

    print()
    print("Что сделать в Claude Code:")
    print(f"  1. Поставить плагин Unica {tag} из маркетплейса.")
    print(f"  2. Убедиться, что кеш пуст: {arguments.cache}")
    print("  3. Открыть сессию — поднимется ядро, движка на диске ещё нет.")
    print("  4. Вызвать инструмент, которому движок нужен:")
    print('       unica.code.graph {"mode": "status"}')
    print()
    print("Что смотреть:")
    print("  · вызов не отказывает, а идёт: приходят уведомления о прогрессе")
    print("    с ключом io.unica/deliveryProgress и растущим числом байтов;")
    print("  · по окончании доставки вызов возвращает результат, а не отказ;")
    print("  · повторный вызов отвечает сразу — движок уже на диске;")
    print("  · отмена вызова посреди доставки не отменяет саму доставку:")
    print("    следующий вызов подхватывает то, что доехало.")
    print()
    print(f"Кеш для наблюдения: {arguments.cache}")
    print(f"  недокачка: {arguments.cache}/.partial/{arguments.engine}/")
    print(
        "  установка: "
        f"{arguments.cache}/{arguments.engine}/{version}--{engine_sha}/{target}/"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
