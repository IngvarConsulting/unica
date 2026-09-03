#!/usr/bin/env python3
"""Check that every address the runtime manifest names actually resolves.

Ядро выпуск публикует сам, и его байты сверяет `verify-release-assets.py`.
Поставки приезжают из тулчейна — их байты сверены на сборке, когда CI их качал
и считал замыкание, но **адрес** в манифесте после этого не трогает никто.
Опечатка в теге или имени ассета дожила бы до первого вызова движка у
пользователя.

Проверяются все цели, а не только текущая: манифест объявляет три, а прогреть
на месте можно лишь одну.
"""
from __future__ import annotations

import argparse
import json
import urllib.error
import urllib.request
from pathlib import Path

USER_AGENT = "unica-release-verifier"


def asset_urls(manifest: dict) -> list[tuple[str, str, str]]:
    """Артефакт, цель и адрес — по одной записи на каждый объявленный ассет."""
    if manifest.get("schemaVersion") != 2:
        raise SystemExit(
            f"unsupported runtime manifest schemaVersion: {manifest.get('schemaVersion')}"
        )
    if manifest.get("development"):
        raise SystemExit("a development manifest publishes no assets to reach")
    found: list[tuple[str, str, str]] = []
    for artifact, entry in sorted(manifest.get("artifacts", {}).items()):
        for target, runtime in sorted(entry.get("targets", {}).items()):
            url = runtime.get("asset", {}).get("url")
            if not isinstance(url, str) or not url.startswith("https://"):
                raise SystemExit(f"{artifact} {target} names no https asset URL")
            found.append((artifact, target, url))
    if not found:
        raise SystemExit("runtime manifest names no assets")
    return found


def reachable(url: str, *, timeout: float) -> int:
    """Код ответа на HEAD с переходом по редиректам."""
    request = urllib.request.Request(url, method="HEAD", headers={"User-Agent": USER_AGENT})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.status


def verify(manifest_path: Path, *, timeout: float) -> int:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    entries = asset_urls(manifest)
    missing: list[str] = []
    for artifact, target, url in entries:
        try:
            status = reachable(url, timeout=timeout)
        except urllib.error.HTTPError as error:
            missing.append(f"{artifact} {target}: HTTP {error.code} {url}")
            continue
        except (urllib.error.URLError, TimeoutError, OSError) as error:
            missing.append(f"{artifact} {target}: {type(error).__name__} {url}")
            continue
        if status != 200:
            missing.append(f"{artifact} {target}: HTTP {status} {url}")
            continue
        print(f"ok {artifact} {target} {url}", flush=True)
    if missing:
        raise SystemExit(
            "runtime manifest names assets that do not resolve:\n  " + "\n  ".join(missing)
        )
    return len(entries)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--plugin-root", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=float, default=30)
    args = parser.parse_args()
    count = verify(
        args.plugin_root.resolve() / "runtime-manifest.json",
        timeout=args.timeout_seconds,
    )
    print(f"verified {count} runtime asset addresses")


if __name__ == "__main__":
    main()
