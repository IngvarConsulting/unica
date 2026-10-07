#!/usr/bin/env python3
"""Канал выпуска и порядок версий для публикации в маркетплейс.

Тег источника решает, куда идёт выпуск. `vX.Y.Z` — стабильный: его получают
основной каталог (ветка `main` маркетплейса) и канал `next`. `vX.Y.Z-rc.N` —
кандидат: его получает только канал `next`. Другие предвыпуски существуют
для замеров и не публикуются вовсе.

Порядок версий — по SemVer: предвыпуск младше своего выпуска, поэтому
`v0.13.0` новее `v0.13.0-rc.3`. `sort -V` упорядочивает их наоборот, и выход
полной версии после кандидата выглядел бы откатом.

`forward` отвечает кодом выхода: 0 — кандидат не старше текущего ref каталога,
1 — кандидат старше (опоздавший прогон), 2 — ref или тег не являются тегом
выпуска. Пустой ref — тоже 2: каталог всегда что-то раздаёт, и пустое значение
означает несчитанный каталог, а не первую публикацию.

Тег `vX` маркетплейса — снимок опубликованного каталога: каталоги внутри него
называют `vX`. До продвижения такого коммита нет, поэтому проверки установки
идут по якорю `candidate-vX` на коммите stage: `anchor` печатает его имя,
а `write-catalogs --ref` пишет каталоги кандидата, называющие якорь.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

TAG = re.compile(
    r"\Av(?P<major>0|[1-9]\d*)\.(?P<minor>0|[1-9]\d*)\.(?P<patch>0|[1-9]\d*)"
    r"(?:-(?P<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?\Z"
)
CANDIDATE = re.compile(r"\Arc\.(?:0|[1-9]\d*)\Z")
# Якорь проверок установки. Не начинается с `v`, поэтому не совпадает
# ни с тегом выпуска, ни с триггером `v*` проверок маркетплейса.
ANCHOR_PREFIX = "candidate-"
CODEX_CATALOG = Path(".agents/plugins/marketplace.json")
CLAUDE_CATALOG = Path(".claude-plugin/marketplace.json")
# Claude Code опознаёт маркетплейс по имени и не держит два одноимённых сразу,
# поэтому у канала своё имя: тестировщик добавляет его рядом с основным.
MARKETPLACE_NAMES = {"stable": "unica", "next": "unica-next"}
DISPLAY_NAMES = {"stable": "Unica", "next": "Unica (next)"}


class TagError(ValueError):
    pass


def parse(tag: str) -> tuple[tuple[int, int, int], tuple[str, ...]]:
    match = TAG.match(tag)
    if match is None:
        raise TagError(f"not a release tag: {tag!r}")
    prerelease = match["prerelease"]
    identifiers = tuple(prerelease.split(".")) if prerelease else ()
    for identifier in identifiers:
        if identifier.isdigit() and len(identifier) > 1 and identifier.startswith("0"):
            raise TagError(f"numeric prerelease identifier has a leading zero: {tag!r}")
    return (int(match["major"]), int(match["minor"]), int(match["patch"])), identifiers


def channel(tag: str) -> str:
    _, prerelease = parse(tag)
    if not prerelease:
        return "stable"
    return "next" if CANDIDATE.match(".".join(prerelease)) else "none"


def anchor(tag: str) -> str:
    """The tag the install checks resolve before the catalog names `tag`."""
    parse(tag)
    return ANCHOR_PREFIX + tag


def _identifier_key(identifier: str) -> tuple[int, int, str]:
    # Числовые идентификаторы сравниваются как числа и младше буквенных.
    if identifier.isdigit():
        return (0, int(identifier), "")
    return (1, 0, identifier)


def precedence(tag: str) -> tuple:
    core, prerelease = parse(tag)
    if not prerelease:
        return (core, 1, ())
    return (core, 0, tuple(_identifier_key(identifier) for identifier in prerelease))


def is_forward(current: str, candidate: str) -> bool:
    return precedence(candidate) >= precedence(current)


def _load(path: Path) -> dict:
    try:
        value = json.loads(Path(path).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise TagError(f"cannot read catalog {path}: {error}") from error
    if not isinstance(value, dict):
        raise TagError(f"catalog {path} is not a JSON object")
    return value


def _single_ref(catalog: dict, path: Path) -> str:
    plugins = catalog.get("plugins")
    if not isinstance(plugins, list) or len(plugins) != 1:
        raise TagError(f"catalog {path} must expose exactly one plugin")
    source = plugins[0].get("source")
    ref = source.get("ref") if isinstance(source, dict) else None
    if not isinstance(ref, str):
        raise TagError(f"catalog {path} does not pin a release ref")
    parse(ref)
    return ref


def catalog_ref(codex_path: Path, claude_path: Path) -> str:
    """The release both host catalogs serve; they must agree, or neither is trusted."""
    codex_ref = _single_ref(_load(codex_path), codex_path)
    claude_ref = _single_ref(_load(claude_path), claude_path)
    if codex_ref != claude_ref:
        raise TagError(f"host catalogs disagree: codex {codex_ref}, claude {claude_ref}")
    return codex_ref


def write_catalogs(target: str, tag: str, source: Path, destination: Path, ref: str | None = None) -> None:
    """Write the catalogs of a channel from the payload built for `tag`.

    `ref` names the tag the catalogs resolve. It defaults to `tag` itself, the
    published catalog; the only other value is the candidate anchor of `tag`,
    which the install checks resolve before the release tag exists.
    """
    if target not in MARKETPLACE_NAMES:
        raise TagError(f"unknown channel: {target!r}")
    if channel(tag) not in ("stable", target):
        raise TagError(f"{tag} does not belong to the {target} channel")
    codex_source, claude_source = source / CODEX_CATALOG, source / CLAUDE_CATALOG
    if catalog_ref(codex_source, claude_source) != tag:
        raise TagError(f"payload catalogs do not pin {tag}")
    resolved = tag if ref is None else ref
    if resolved not in (tag, anchor(tag)):
        raise TagError(f"{resolved!r} is neither {tag} nor its candidate anchor")
    codex, claude = _load(codex_source), _load(claude_source)
    for catalog in (codex, claude):
        catalog["plugins"][0]["source"]["ref"] = resolved
    codex["name"] = MARKETPLACE_NAMES[target]
    codex.setdefault("interface", {})["displayName"] = DISPLAY_NAMES[target]
    claude["name"] = MARKETPLACE_NAMES[target]
    for relative, catalog in ((CODEX_CATALOG, codex), (CLAUDE_CATALOG, claude)):
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(catalog, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Release channel and SemVer order for marketplace publication.")
    commands = parser.add_subparsers(dest="command", required=True)
    channel_command = commands.add_parser("channel", help="print stable, next or none for a source tag")
    channel_command.add_argument("tag")
    forward_command = commands.add_parser("forward", help="exit 0 when the candidate is not older than current")
    forward_command.add_argument("current")
    forward_command.add_argument("candidate")
    ref_command = commands.add_parser("catalog-ref", help="print the release both host catalogs pin")
    ref_command.add_argument("codex", type=Path)
    ref_command.add_argument("claude", type=Path)
    anchor_command = commands.add_parser("anchor", help="print the candidate anchor tag of a release tag")
    anchor_command.add_argument("tag")
    write_command = commands.add_parser("write-catalogs", help="write a channel's catalogs from the payload")
    write_command.add_argument("--ref", help="tag the catalogs resolve: the release tag (default) or its anchor")
    write_command.add_argument("channel", choices=sorted(MARKETPLACE_NAMES))
    write_command.add_argument("tag")
    write_command.add_argument("source", type=Path)
    write_command.add_argument("destination", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "channel":
            print(channel(args.tag))
        elif args.command == "catalog-ref":
            print(catalog_ref(args.codex, args.claude))
        elif args.command == "anchor":
            print(anchor(args.tag))
        elif args.command == "write-catalogs":
            write_catalogs(args.channel, args.tag, args.source, args.destination, args.ref)
        elif not is_forward(args.current, args.candidate):
            print(f"{args.candidate} is older than {args.current}", file=sys.stderr)
            return 1
    except TagError as error:
        print(error, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
