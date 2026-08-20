#!/usr/bin/env python3
"""Подготовить проверку доставки движка на живом Claude Code.

Гейт фазы 3 требует, чтобы вызов к недоставленному движку возвращал результат,
а не отказ, и чтобы ход доставки был виден в хосте. Проверить это можно только
руками и только на опубликованной поставке: доставка ходит за архивом по адресу
из манифеста, а манифест прибит к релизам `IngvarConsulting/unica`.

Скрипт делает то, что можно сделать заранее: сверяет, есть ли в релизе архивы
движков, готовит пустой кеш артефактов и печатает сценарий проверки. Сам вызов
и наблюдение остаются за человеком — хост не автоматизируется.

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

REPO = "IngvarConsulting/unica"
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


def release_assets(tag: str | None) -> "tuple[str, list[str]]":
    command = ["gh", "release", "view", "--repo", REPO, "--json", "tagName,assets"]
    if tag:
        command.insert(3, tag)
    finished = subprocess.run(command, capture_output=True, text=True, check=False)
    if finished.returncode != 0:
        raise SystemExit(f"не удалось прочитать релиз: {finished.stderr.strip()}")
    published = json.loads(finished.stdout)
    return published["tagName"], [asset["name"] for asset in published["assets"]]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--tag", help="релиз; по умолчанию последний")
    parser.add_argument("--engine", default="bsl-analyzer", help="артефакт движка")
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
    tag, assets = release_assets(arguments.tag)
    engine_asset = f"{arguments.engine}-runtime-{target}.tar.gz"
    core_asset = f"unica-runtime-{target}.tar.gz"

    print(f"релиз {tag}, цель {target}")
    print(f"  ядро:  {core_asset}: {'есть' if core_asset in assets else 'НЕТ'}")
    print(f"  движок {engine_asset}: {'есть' if engine_asset in assets else 'НЕТ'}")

    if engine_asset not in assets:
        print()
        print("Проверку провести нечем: в релизе нет отдельного архива движка.")
        print("Опубликованные архивы:")
        for name in sorted(assets):
            print(f"  {name}")
        print()
        print("Разрез поставки на артефакты сделан в этой ветке, но ни один релиз")
        print("им ещё не собран. Нужен выпуск новым упаковщиком: он кладёт по")
        print(f"архиву на артефакт, среди них {engine_asset}.")
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
    print(f"  установка: {arguments.cache}/{arguments.engine}/<версия>/{target}/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
