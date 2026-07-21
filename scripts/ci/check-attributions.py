#!/usr/bin/env python3
"""Validate coverage and structural links in Unica's manual attribution page."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from urllib.parse import urlparse


MARKER_RE = re.compile(
    r"^<!-- unica-attribution: (project|tool|adapter|upstream) "
    r"([a-z0-9][a-z0-9._-]*) -->$"
)
LABELS = {
    "repository": re.compile(r"(?:Repository|Репозиторий):\s*\[[^]]+\]\(([^)]+)\)"),
    "author": re.compile(r"(?:Author|Автор):\s*\[[^]]+\]\(([^)]+)\)"),
    "license": re.compile(r"(?:License|Лицензия):\s*\[[^]]+\]\(([^)]+)\)"),
    "provider": re.compile(r"(?:Provider|Поставщик):\s*\[[^]]+\]\(([^)]+)\)"),
    "service": re.compile(r"(?:Service|Сервис):\s*\[[^]]+\]\(([^)]+)\)"),
}


def load_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def plugin_root(repo_root: Path) -> Path:
    return repo_root / "plugins" / "unica"


def inventory(repo_root: Path) -> dict[tuple[str, str], dict]:
    root = plugin_root(repo_root)
    plugin = load_json(root / ".codex-plugin" / "plugin.json")
    tools = load_json(root / "third-party" / "tools.lock.json").get("tools", [])
    adapters = load_json(root / "third-party" / "manifest.json").get("internalAdapters", [])
    upstreams = load_json(root / "provenance" / "skill-upstreams.json").get("upstreams", [])

    result: dict[tuple[str, str], dict] = {
        ("project", plugin["name"]): {
            "repository": plugin["repository"],
            "author": plugin["author"]["url"],
            "license": plugin["license"],
        }
    }
    result.update(
        {
            ("tool", tool["name"]): tool
            for tool in tools
            if tool["name"] != plugin["name"]
        }
    )
    result.update({("adapter", adapter["name"]): adapter for adapter in adapters})
    result.update({("upstream", upstream["id"]): upstream for upstream in upstreams})
    return result


def expected_markers(repo_root: Path) -> set[tuple[str, str]]:
    return set(inventory(repo_root))


def parse_sections(markdown: str) -> dict[tuple[str, str], str]:
    sections: dict[tuple[str, str], str] = {}
    pending: list[tuple[str, str]] = []
    body: list[str] = []

    def flush() -> None:
        nonlocal pending, body
        if not pending:
            body = []
            return
        text = "\n".join(body).strip()
        for marker in pending:
            sections[marker] = text
        pending = []
        body = []

    for line in markdown.splitlines():
        match = MARKER_RE.fullmatch(line.strip())
        if match:
            marker = (match.group(1), match.group(2))
            if marker in sections or marker in pending:
                raise ValueError(f"duplicate attribution marker: {marker[0]} {marker[1]}")
            if pending and any(item.strip() for item in body):
                flush()
            pending.append(marker)
            continue
        if line.startswith("## ") and pending:
            flush()
        if pending:
            body.append(line)
    flush()
    return sections


def labelled_link(section: str, label: str) -> str | None:
    match = LABELS[label].search(section)
    return match.group(1) if match else None


def is_https(url: str | None) -> bool:
    if not url:
        return False
    parsed = urlparse(url)
    return parsed.scheme == "https" and bool(parsed.netloc)


def validate_local_license(
    root: Path,
    marker: tuple[str, str],
    link: str | None,
    *,
    allow_external: bool,
) -> str | None:
    label = f"{marker[0]} {marker[1]}"
    if not link:
        return f"{label}: license link is required"
    if is_https(link):
        if allow_external:
            return None
        return f"{label}: license link must point to a packaged file"
    relative = Path(link.split("#", 1)[0])
    if relative.is_absolute() or ".." in relative.parts:
        return f"{label}: license link must stay inside plugins/unica"
    target = root / relative
    if not target.is_file():
        return f"{label}: packaged license path does not exist: {link}"
    return None


def validate_attributions(
    repo_root: Path, attribution_file: Path | None = None
) -> list[str]:
    root = plugin_root(repo_root)
    attribution_file = attribution_file or root / "ATTRIBUTIONS.md"
    if not attribution_file.is_file():
        return [f"ATTRIBUTIONS.md not found: {attribution_file}"]

    try:
        sections = parse_sections(attribution_file.read_text(encoding="utf-8"))
    except ValueError as exc:
        return [str(exc)]

    records = inventory(repo_root)
    actual = set(sections)
    expected = set(records)
    errors = [f"missing attribution: {kind} {name}" for kind, name in sorted(expected - actual)]
    errors.extend(f"unknown attribution: {kind} {name}" for kind, name in sorted(actual - expected))

    for marker in sorted(expected & actual):
        kind, name = marker
        section = sections[marker]
        record = records[marker]
        label = f"{kind} {name}"

        if kind == "adapter":
            provider = labelled_link(section, "provider")
            service = labelled_link(section, "service")
            if not is_https(provider):
                errors.append(f"{label}: provider link must be absolute HTTPS")
            if service != record["url"] or not is_https(service):
                errors.append(f"{label}: service link must match {record['url']}")
            lowered = section.lower()
            if "not redistributed" not in lowered and "не распространяется" not in lowered:
                errors.append(f"{label}: section must state that the service is not redistributed")
            continue

        repository = labelled_link(section, "repository")
        if repository != record["repository"] or not is_https(repository):
            errors.append(f"{label}: repository link must match {record['repository']}")
        author = labelled_link(section, "author")
        if not is_https(author):
            errors.append(f"{label}: author link must be absolute HTTPS")

        needs_license = kind in {"project", "tool"} or (
            kind == "upstream" and record.get("usage") != "inspiration-only"
        )
        if needs_license:
            if kind in {"project", "tool"} and record["license"] not in section:
                errors.append(f"{label}: declared license {record['license']} must appear in the section")
            license_error = validate_local_license(
                root,
                marker,
                labelled_link(section, "license"),
                allow_external=kind == "upstream",
            )
            if license_error:
                errors.append(license_error)

    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo-root", type=Path, default=Path("."))
    parser.add_argument("--attribution-file", type=Path)
    args = parser.parse_args()

    repo_root = args.repo_root.resolve()
    attribution_file = args.attribution_file.resolve() if args.attribution_file else None
    errors = validate_attributions(repo_root, attribution_file)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("attributions: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
