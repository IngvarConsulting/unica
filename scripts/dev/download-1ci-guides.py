#!/usr/bin/env python3
"""Download selected English 1Ci guides into a private local Markdown corpus."""

from __future__ import annotations

import argparse
from collections import deque
from datetime import datetime, timezone
import hashlib
from html import unescape
from html.parser import HTMLParser
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import tempfile
import time
from typing import NamedTuple
from urllib.error import HTTPError, URLError
from urllib.parse import unquote, urljoin, urlsplit, urlunsplit
from urllib.request import HTTPCookieProcessor, Request, build_opener
from urllib.robotparser import RobotFileParser


BASE_URL = "https://kb.1ci.com"
SITEMAP_URL = BASE_URL + "/sitemap.xml"
ROBOTS_URL = BASE_URL + "/robots.txt"
SPACES_URL = BASE_URL + "/rest/wikis/xwiki/spaces"
USER_AGENT = "UnicaLocalDocs/1.0 (+private development corpus)"
XWIKI_VIEW_PREFIX = "/bin/view/OnecInt/KB"


class Guide(NamedTuple):
    name: str
    root: str


CLIENT_SERVER = Guide(
    "administrator-client-server",
    BASE_URL
    + "/1C_Enterprise_Platform/Guides/Administrator_Guides/"
    + "1C_Enterprise_8.3.27_Administrator_Guide._Client_Server_Mode/",
)
FILE_MODE = Guide(
    "administrator-file-mode",
    BASE_URL
    + "/1C_Enterprise_Platform/Guides/Administrator_Guides/"
    + "1C_Enterprise_8.3.27_Administrator_Guide/",
)
DEVELOPER = Guide(
    "developer",
    BASE_URL
    + "/1C_Enterprise_Platform/Guides/Developer_Guides/"
    + "1C_Enterprise_8.3.27_Developer_Guide/",
)
GUIDES = (CLIENT_SERVER, FILE_MODE, DEVELOPER)


class DownloadError(RuntimeError):
    pass


class Node:
    def __init__(self, tag: str, attrs: dict[str, str] | None = None):
        self.tag = tag
        self.attrs = attrs or {}
        self.children: list[Node | str] = []


class ExtractedPage(NamedTuple):
    title: str
    markdown: str
    page_links: tuple[str, ...]
    assets: tuple[str, ...]


def _root_path(guide: Guide) -> str:
    return urlsplit(guide.root).path.rstrip("/") + "/"


def _public_path(path: str) -> str:
    if path == XWIKI_VIEW_PREFIX:
        return "/"
    if path.startswith(XWIKI_VIEW_PREFIX + "/"):
        return path[len(XWIKI_VIEW_PREFIX) :]
    return path


def guide_for_url(url: str) -> Guide | None:
    parsed = urlsplit(urljoin(BASE_URL, url))
    if parsed.scheme not in {"http", "https"} or parsed.netloc.lower() != "kb.1ci.com":
        return None
    path = _public_path(parsed.path).rstrip("/") + "/"
    for guide in GUIDES:
        root = _root_path(guide)
        if path == root or path.startswith(root):
            return guide
    return None


def normalize_page_url(url: str) -> str:
    absolute = urljoin(BASE_URL, unescape(url))
    guide = guide_for_url(absolute)
    if guide is None:
        raise ValueError(f"URL is outside configured guide roots: {url}")
    parsed = urlsplit(absolute)
    path = re.sub(r"/+", "/", _public_path(parsed.path))
    if not path.endswith("/"):
        path += "/"
    return urlunsplit(("https", "kb.1ci.com", path, "language=en", ""))


def _safe_segment(value: str) -> str:
    value = unquote(value).strip()
    value = re.sub(r"[\x00-\x1f/:]", "_", value)
    if value in {"", ".", ".."}:
        return "_"
    return value


def page_relative_path(guide: Guide, url: str) -> Path:
    normalized = normalize_page_url(url)
    relative = urlsplit(normalized).path[len(_root_path(guide)) :].strip("/")
    parts = [_safe_segment(part) for part in PurePosixPath(relative).parts]
    return Path(guide.name, *parts, "index.md")


def guide_space_id(guide: Guide) -> str:
    segments = ["OnecInt", "KB", *[part for part in urlsplit(guide.root).path.split("/") if part]]
    escaped = [segment.replace("\\", "\\\\").replace(".", "\\.") for segment in segments]
    return "xwiki:" + ".".join(escaped)


def discover_space_pages(fetch_batch, guides=GUIDES, *, batch_size: int = 1000) -> set[str]:
    pages: set[str] = set()
    prefixes = {guide_space_id(guide): guide for guide in guides}
    offset = 0
    seen_batches: set[tuple[tuple[str, str], ...]] = set()
    while True:
        rows = fetch_batch(offset, batch_size)
        if not rows:
            break
        signature = tuple(
            (str(row.get("id", "")), str(row.get("xwikiAbsoluteUrl", "")))
            for row in rows
        )
        if signature in seen_batches:
            raise DownloadError(f"XWiki spaces endpoint repeated a batch at offset {offset}")
        seen_batches.add(signature)
        for row in rows:
            identifier = row.get("id", "")
            url = row.get("xwikiAbsoluteUrl")
            if not url:
                continue
            for prefix, guide in prefixes.items():
                if identifier != prefix and not identifier.startswith(prefix + "."):
                    continue
                if guide_for_url(url) == guide:
                    pages.add(normalize_page_url(url))
                break
        offset += len(rows)
    return pages


def is_allowed_by_policy(url: str, robots_allowed: bool) -> bool:
    parsed = urlsplit(url)
    explicit_attachment = (
        parsed.scheme == "https"
        and parsed.netloc.lower() == "kb.1ci.com"
        and parsed.path.startswith("/bin/download/")
    )
    return robots_allowed or explicit_attachment


class ContentParser(HTMLParser):
    VOID = {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "wbr"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.root: Node | None = None
        self.stack: list[Node] = []
        self.depth = 0

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        attributes = {key: value or "" for key, value in attrs}
        if self.root is None:
            if attributes.get("id") != "xwikicontent":
                return
            self.root = Node(tag, attributes)
            self.stack = [self.root]
            self.depth = 1
            return
        if not self.stack:
            return
        node = Node(tag, attributes)
        self.stack[-1].children.append(node)
        if tag not in self.VOID:
            self.stack.append(node)
            self.depth += 1

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.handle_starttag(tag, attrs)

    def handle_endtag(self, tag: str) -> None:
        if not self.stack:
            return
        for index in range(len(self.stack) - 1, -1, -1):
            if self.stack[index].tag == tag:
                del self.stack[index:]
                self.depth = len(self.stack)
                return

    def handle_data(self, data: str) -> None:
        if self.stack:
            self.stack[-1].children.append(data)


def _plain(node: Node | str) -> str:
    if isinstance(node, str):
        return node
    return "".join(_plain(child) for child in node.children)


def _clean_link_target(value: str) -> str:
    raw = unescape(value).strip()
    decoded = unquote(raw).strip()
    return decoded if urlsplit(decoded).scheme else raw


def _inline(node: Node | str, page_url: str) -> str:
    if isinstance(node, str):
        return re.sub(r"\s+", " ", node)
    content = "".join(_inline(child, page_url) for child in node.children).strip()
    if node.tag in {"strong", "b"}:
        return f"**{content}**"
    if node.tag in {"em", "i"}:
        return f"*{content}*"
    if node.tag == "code":
        return f"`{content}`"
    if node.tag == "br":
        return "  \n"
    if node.tag == "a":
        href = _clean_link_target(node.attrs.get("href", ""))
        if not href:
            return content
        absolute = urljoin(page_url, href)
        try:
            target = normalize_page_url(absolute)
        except ValueError:
            target = absolute
        fragment = urlsplit(absolute).fragment
        if fragment and guide_for_url(absolute):
            target += "#" + fragment
        return f"[{content or href}]({target})"
    if node.tag == "img":
        src = urljoin(page_url, _clean_link_target(node.attrs.get("src", "")))
        alt = node.attrs.get("alt", "")
        return f"![{alt}]({src})" if src else ""
    return content


def _render_table(node: Node, page_url: str) -> str:
    rows: list[list[str]] = []

    def visit(candidate: Node) -> None:
        if candidate.tag == "tr":
            cells = [
                _inline(child, page_url).replace("|", "\\|").strip()
                for child in candidate.children
                if isinstance(child, Node) and child.tag in {"th", "td"}
            ]
            if cells:
                rows.append(cells)
            return
        for child in candidate.children:
            if isinstance(child, Node):
                visit(child)

    visit(node)
    if not rows:
        return ""
    width = max(len(row) for row in rows)
    rows = [row + [""] * (width - len(row)) for row in rows]
    header, *body = rows
    lines = ["| " + " | ".join(header) + " |", "| " + " | ".join(["---"] * width) + " |"]
    lines.extend("| " + " | ".join(row) + " |" for row in body)
    return "\n".join(lines) + "\n\n"


def _render(node: Node | str, page_url: str, list_depth: int = 0) -> str:
    if isinstance(node, str):
        return re.sub(r"\s+", " ", node)
    tag = node.tag
    if tag in {"script", "style", "nav", "footer", "form", "button"}:
        return ""
    if tag in {f"h{i}" for i in range(1, 7)}:
        return f"{'#' * int(tag[1])} {_inline(node, page_url)}\n\n"
    if tag == "p":
        text = _inline(node, page_url).strip()
        return text + "\n\n" if text else ""
    if tag == "pre":
        return "```\n" + _plain(node).strip("\n") + "\n```\n\n"
    if tag == "table":
        return _render_table(node, page_url)
    if tag in {"ul", "ol"}:
        ordered = tag == "ol"
        lines: list[str] = []
        number = 1
        for child in node.children:
            if isinstance(child, Node) and child.tag == "li":
                prefix = f"{number}." if ordered else "-"
                lines.append("  " * list_depth + prefix + " " + _inline(child, page_url).strip())
                number += 1
        return "\n".join(lines) + "\n\n" if lines else ""
    if tag == "blockquote" or "box" in node.attrs.get("class", "").split():
        text = "".join(_render(child, page_url, list_depth) for child in node.children).strip()
        return "\n".join("> " + line for line in text.splitlines()) + "\n\n"
    if tag in {"a", "img", "strong", "b", "em", "i", "code", "span"}:
        return _inline(node, page_url)
    return "".join(_render(child, page_url, list_depth) for child in node.children)


def _all_nodes(node: Node | str):
    if isinstance(node, str):
        return
    yield node
    for child in node.children:
        yield from _all_nodes(child)


def _looks_like_asset(url: str) -> bool:
    parsed = urlsplit(url)
    if parsed.scheme != "https" or parsed.netloc.lower() != "kb.1ci.com":
        return False
    path = parsed.path.lower()
    return path.startswith("/bin/download/") or "/download/" in path or bool(
        re.search(r"\.(?:png|jpe?g|gif|svg|webp|pdf|zip|rar|7z|tar|gz|epf|erf|xml|json|txt|csv|xlsx?|docx?|pptx?)$", path)
    )


def extract_page(html: str, page_url: str) -> ExtractedPage:
    parser = ContentParser()
    parser.feed(html)
    if parser.root is None:
        raise DownloadError(f"page has no #xwikicontent: {page_url}")
    markdown = _render(parser.root, page_url)
    markdown = re.sub(r"[ \t]+\n", "\n", markdown)
    markdown = re.sub(r"\n{3,}", "\n\n", markdown).strip() + "\n"
    title = next(
        (_plain(node).strip() for node in _all_nodes(parser.root) if node.tag == "h1" and _plain(node).strip()),
        unquote(urlsplit(page_url).path.rstrip("/").split("/")[-1]),
    )
    pages: set[str] = set()
    assets: set[str] = set()
    for node in _all_nodes(parser.root):
        attribute = "src" if node.tag == "img" else "href" if node.tag == "a" else None
        if not attribute or not node.attrs.get(attribute):
            continue
        absolute = urljoin(page_url, _clean_link_target(node.attrs[attribute]))
        if _looks_like_asset(absolute):
            assets.add(absolute)
        elif guide_for_url(absolute):
            try:
                pages.add(normalize_page_url(absolute))
            except ValueError:
                pass
    return ExtractedPage(title, markdown, tuple(sorted(pages)), tuple(sorted(assets)))


class Response(NamedTuple):
    body: bytes
    url: str
    content_type: str
    etag: str
    last_modified: str


class Fetcher:
    def __init__(self, delay: float = 0.25, timeout: float = 30.0):
        self.delay = delay
        self.timeout = timeout
        self.opener = build_opener(HTTPCookieProcessor())
        self.last_request = 0.0

    def fetch(self, url: str, *, accept: str | None = None) -> Response:
        error: Exception | None = None
        for attempt in range(3):
            wait = self.delay - (time.monotonic() - self.last_request)
            if wait > 0:
                time.sleep(wait)
            request = Request(url, headers=request_headers(accept))
            try:
                with self.opener.open(request, timeout=self.timeout) as response:
                    self.last_request = time.monotonic()
                    return Response(
                        response.read(),
                        response.geturl(),
                        response.headers.get_content_type(),
                        response.headers.get("ETag", ""),
                        response.headers.get("Last-Modified", ""),
                    )
            except (HTTPError, URLError, TimeoutError) as caught:
                error = caught
                if isinstance(caught, HTTPError) and caught.code < 500 and caught.code != 429:
                    break
                time.sleep(2**attempt)
        raise DownloadError(f"failed to fetch {url}: {error}")


def request_headers(accept: str | None = None) -> dict[str, str]:
    headers = {"User-Agent": USER_AGENT, "Accept-Language": "en"}
    if accept:
        headers["Accept"] = accept
    return headers


def _parse_robots(data: bytes) -> RobotFileParser:
    parser = RobotFileParser()
    parser.set_url(ROBOTS_URL)
    parser.parse(data.decode("utf-8", "replace").splitlines())
    return parser


class SitemapParser(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.locations: list[str] = []
        self._location: list[str] | None = None

    def handle_starttag(self, tag: str, attrs) -> None:
        if tag.rsplit(":", 1)[-1] == "loc":
            self._location = []

    def handle_data(self, data: str) -> None:
        if self._location is not None:
            self._location.append(data)

    def handle_entityref(self, name: str) -> None:
        if self._location is not None:
            self._location.append(f"&{name};")

    def handle_endtag(self, tag: str) -> None:
        if tag.rsplit(":", 1)[-1] == "loc" and self._location is not None:
            self.locations.append("".join(self._location).strip())
            self._location = None


def _sitemap_pages(data: bytes) -> set[str]:
    parser = SitemapParser()
    parser.feed(data.decode("utf-8", "replace"))
    pages: set[str] = set()
    for location in parser.locations:
        if not location:
            continue
        try:
            normalized = normalize_page_url(location)
        except ValueError:
            continue
        pages.add(normalized)
    return pages


def _asset_relative_path(page_path: Path, url: str) -> Path:
    name = _safe_segment(Path(unquote(urlsplit(url).path)).name or "attachment")
    stem, suffix = os.path.splitext(name)
    digest = hashlib.sha256(url.encode()).hexdigest()[:10]
    return page_path.parent / "_assets" / f"{stem}-{digest}{suffix}"


def _relative_link(from_page: Path, target: Path) -> str:
    return Path(os.path.relpath(target, from_page.parent)).as_posix()


def _atomic_write(path: Path, data: bytes) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, path)


def publish_staging(staging: Path, destination: Path, *, allow_limited: bool = False) -> None:
    manifest_path = staging / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise DownloadError(f"invalid staging manifest: {error}") from error
    if manifest.get("complete") is not True and not (
        allow_limited and manifest.get("limited") is True
    ):
        raise DownloadError("refusing to publish an incomplete corpus")
    backup = destination.with_name(destination.name + ".previous")
    if backup.exists():
        shutil.rmtree(backup)
    if destination.exists():
        os.replace(destination, backup)
    try:
        os.replace(staging, destination)
    except Exception:
        if backup.exists() and not destination.exists():
            os.replace(backup, destination)
        raise
    if backup.exists():
        shutil.rmtree(backup)


class Downloader:
    def __init__(self, output: Path, delay: float = 0.25, fetcher: Fetcher | None = None):
        self.output = output
        self.fetcher = fetcher or Fetcher(delay=delay)

    def run(self, max_pages: int | None = None) -> dict:
        parent = self.output.parent
        parent.mkdir(parents=True, exist_ok=True)
        staging = Path(tempfile.mkdtemp(prefix=self.output.name + ".staging-", dir=parent))
        try:
            manifest = self._crawl(staging, max_pages)
            _atomic_write(
                staging / "manifest.json",
                (json.dumps(manifest, ensure_ascii=False, indent=2, sort_keys=True) + "\n").encode(),
            )
            _atomic_write(staging / "README.md", self._readme(manifest).encode())
            publish_staging(staging, self.output, allow_limited=max_pages is not None)
            return manifest
        except Exception:
            shutil.rmtree(staging, ignore_errors=True)
            raise

    def _crawl(self, staging: Path, max_pages: int | None) -> dict:
        robots_response = self.fetcher.fetch(ROBOTS_URL)
        robots = _parse_robots(robots_response.body)
        sitemap_response = self.fetcher.fetch(SITEMAP_URL)
        def fetch_space_batch(start: int, number: int) -> list[dict]:
            separator = "&" if "?" in SPACES_URL else "?"
            response = self.fetcher.fetch(
                f"{SPACES_URL}{separator}start={start}&number={number}",
                accept="application/json",
            )
            try:
                return json.loads(response.body)["spaces"]
            except (json.JSONDecodeError, KeyError, TypeError) as error:
                raise DownloadError(f"invalid XWiki spaces response at offset {start}: {error}") from error

        discovered = _sitemap_pages(sitemap_response.body)
        discovered.update(discover_space_pages(fetch_space_batch))
        discovered.update(normalize_page_url(guide.root) for guide in GUIDES)
        print(f"discovered {len(discovered)} pages", flush=True)
        pending = deque(sorted(discovered))
        seen: set[str] = set()
        records: list[dict] = []
        failures: list[dict] = []
        while pending and (max_pages is None or len(seen) < max_pages):
            url = pending.popleft()
            if url in seen:
                continue
            seen.add(url)
            guide = guide_for_url(url)
            assert guide is not None
            allowed = robots.can_fetch(USER_AGENT, url)
            if not is_allowed_by_policy(url, allowed):
                failures.append({"url": url, "error": "blocked by robots.txt"})
                continue
            try:
                response = self.fetcher.fetch(url)
                page = extract_page(response.body.decode("utf-8", "replace"), url)
                page_path = page_relative_path(guide, url)
                markdown = page.markdown
                for link in page.page_links:
                    target_guide = guide_for_url(link)
                    if target_guide == guide:
                        pending.append(link)
                        markdown = markdown.replace(link, _relative_link(page_path, page_relative_path(guide, link)))
                assets: list[dict] = []
                for asset_url in page.assets:
                    asset_allowed = robots.can_fetch(USER_AGENT, asset_url)
                    if not is_allowed_by_policy(asset_url, asset_allowed):
                        failures.append({"url": asset_url, "page": url, "error": "blocked by robots.txt"})
                        continue
                    asset_response = self.fetcher.fetch(asset_url)
                    asset_retrieved = datetime.now(timezone.utc).isoformat()
                    asset_path = _asset_relative_path(page_path, asset_url)
                    _atomic_write(staging / asset_path, asset_response.body)
                    markdown = markdown.replace(asset_url, _relative_link(page_path, asset_path))
                    assets.append(
                        {
                            "url": asset_url,
                            "path": asset_path.as_posix(),
                            "sha256": hashlib.sha256(asset_response.body).hexdigest(),
                            "contentType": asset_response.content_type,
                            "etag": asset_response.etag,
                            "lastModified": asset_response.last_modified,
                            "retrieved": asset_retrieved,
                        }
                    )
                retrieved = datetime.now(timezone.utc).isoformat()
                front_matter = (
                    "---\n"
                    f"source: {json.dumps(url)}\n"
                    f"retrieved: {json.dumps(retrieved)}\n"
                    "---\n\n"
                )
                _atomic_write(staging / page_path, (front_matter + markdown).encode("utf-8"))
                records.append(
                    {
                        "guide": guide.name,
                        "url": url,
                        "path": page_path.as_posix(),
                        "title": page.title,
                        "sha256": hashlib.sha256(response.body).hexdigest(),
                        "etag": response.etag,
                        "lastModified": response.last_modified,
                        "retrieved": retrieved,
                        "assets": assets,
                    }
                )
                print(f"downloaded {len(records)}: {url}", flush=True)
            except Exception as error:
                failures.append({"url": url, "error": str(error)})
        manifest = build_manifest(records, failures, max_pages=max_pages, remaining=len(pending))
        if failures:
            raise DownloadError(f"download failed for {len(failures)} item(s): {failures[0]}")
        return manifest

    @staticmethod
    def _readme(manifest: dict) -> str:
        counts = {guide.name: 0 for guide in GUIDES}
        for page in manifest["pages"]:
            counts[page["guide"]] += 1
        lines = ["# Local 1Ci 8.3.27 guides", "", "Private development corpus. Do not publish.", ""]
        lines.extend(f"- `{guide.name}`: {counts[guide.name]} pages" for guide in GUIDES)
        lines.extend(["", "Refresh from the repository root:", "", "```sh", "python3.12 scripts/dev/download-1ci-guides.py", "```", ""])
        return "\n".join(lines)


def build_manifest(
    records: list[dict],
    failures: list[dict],
    *,
    max_pages: int | None,
    remaining: int = 0,
) -> dict:
    return {
        "schemaVersion": 1,
        "complete": not failures and remaining == 0 and max_pages is None,
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "sourceRoots": [guide.root for guide in GUIDES],
        "pages": sorted(records, key=lambda record: record["url"]),
        "failures": failures,
        "limited": max_pages is not None,
    }


def _check_links(root: Path) -> list[str]:
    broken: list[str] = []
    pattern = re.compile(r"!?\[[^]]*]\(([^)]+)\)")
    fenced_code = re.compile(r"^\s*```.*?^\s*```\s*$", re.MULTILINE | re.DOTALL)
    for markdown in root.rglob("*.md"):
        content = fenced_code.sub("", markdown.read_text(encoding="utf-8"))
        for target in pattern.findall(content):
            if (
                urlsplit(target).scheme
                or target.startswith("#")
                or (target.startswith("<") and target.endswith(">"))
            ):
                continue
            path = (markdown.parent / unquote(target.split("#", 1)[0])).resolve()
            if not path.exists():
                broken.append(f"{markdown.relative_to(root)} -> {target}")
    return broken


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=Path("docs-local/1ci/8.3.27/en"))
    parser.add_argument("--max-pages", type=int)
    parser.add_argument("--delay", type=float, default=0.25)
    parser.add_argument("--check-links", action="store_true")
    args = parser.parse_args()
    manifest = Downloader(args.output, delay=args.delay).run(max_pages=args.max_pages)
    if args.check_links:
        broken = _check_links(args.output)
        if broken:
            print("broken local links:\n" + "\n".join(broken[:50]))
            return 1
    assets = sum(len(page["assets"]) for page in manifest["pages"])
    print(f"complete: {len(manifest['pages'])} pages, {assets} assets, 0 failures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
