#!/usr/bin/env python3
"""Собрать документ статуса сайта из релизов, веток и отчёта прогона.

Ни одно значение здесь не пишется руками: версия приходит из релиза, состав
открытых линий — из правила закрытия, счётчики тестов — из сводки собранного
отчёта. Страница не может разойтись с тем, что опубликовано.
"""

from __future__ import annotations

import argparse
import base64
import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path


LINE_BRANCH = re.compile(r"\Arelease-v(\d+)\.(\d+)\Z")
# Линия патчей живёт ещё месяц после того, как вышла следующая minor.
LINE_GRACE = timedelta(days=30)
CHANNEL_RULES = Path(__file__).with_name("release-channel.py")


def gh(repo: str, path: str) -> list:
    """Все страницы ответа одним списком.

    `--slurp` возвращает массив страниц, поэтому его достаточно развернуть —
    склейка потока JSON регулярным выражением ломается на первой же кавычке
    внутри строки.
    """
    endpoint = f"repos/{repo}/{path}" if path else f"repos/{repo}"
    result = subprocess.run(
        ["gh", "api", endpoint, "--paginate", "--slurp"],
        capture_output=True,
        text=True,
        check=True,
    )
    pages = json.loads(result.stdout)
    merged: list = []
    for page in pages:
        merged.extend(page if isinstance(page, list) else [page])
    return merged


def moment(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def human(value: datetime) -> str:
    return value.astimezone(timezone.utc).strftime("%d.%m.%Y")


def open_lines(repo: str, now: datetime) -> list[str]:
    """Какие линии патчей ещё открыты.

    Линия закрывается через месяц после первого релиза следующей minor: до
    этого момента патч может выйти и там, и там.
    """
    releases = [r for r in gh(repo, "releases?per_page=100") if not r["draft"]]
    first_of_minor: dict[tuple[int, int], datetime] = {}
    for release in releases:
        found = re.match(r"\Av(\d+)\.(\d+)\.(\d+)", release["tag_name"])
        if not found:
            continue
        key = (int(found.group(1)), int(found.group(2)))
        published = moment(release["published_at"])
        first_of_minor[key] = min(first_of_minor.get(key, published), published)

    lines = []
    for branch in gh(repo, "branches?per_page=100"):
        found = LINE_BRANCH.match(branch["name"])
        if not found:
            continue
        key = (int(found.group(1)), int(found.group(2)))
        successor = min(
            (published for minor, published in first_of_minor.items() if minor > key),
            default=None,
        )
        if successor is not None and now - successor > LINE_GRACE:
            continue
        lines.append(branch["name"])
    return sorted(lines)


def site_lines(branch: str, repo: str, now: datetime) -> list[str]:
    """`main`, линия прогона и открытые релизные линии, каждая по одному разу.

    `main` стоит всегда: прогон релизной линии не отменяет его карточку. Ветка
    прогона попадает в список, только если она линия: с прогона по тегу
    приходит имя тега `vX.Y.Z`, его результаты сайт кладёт в линию тега, и
    карточка «vX.Y.Z — нет прогонов» была бы ложью. Без свёртки отчёт линии
    собирался бы дважды, а на странице появлялась бы вторая такая же строка.
    """
    own = [branch] if LINE_BRANCH.match(branch) else []
    return list(dict.fromkeys(["main", *own, *open_lines(repo, now)]))


def github_stars(repo: str) -> str:
    """Звёзд у репозитория на момент сборки."""
    try:
        return str(gh(repo, "")[0]["stargazers_count"])
    except Exception:
        return "—"


def telegram_members(chat: str) -> str:
    """Сколько человек в группе.

    Из браузера это число не прочитать: `t.me` не отдаёт CORS-заголовков, а у
    Bot API нет входа без токена, и класть токен в страницу нельзя. Поэтому
    число печатается при сборке и живёт до следующей.
    """
    try:
        with urllib.request.urlopen(f"https://t.me/{chat}", timeout=20) as response:
            page = response.read().decode("utf-8", "replace")
    except Exception:
        return "—"
    found = re.search(r'tgme_page_extra">([\d\s \xa0]+)\s+(?:members|subscribers)', page)
    return re.sub(r"[\s \xa0]", "", found.group(1)) if found else "—"


def channel_rules():
    spec = importlib.util.spec_from_file_location("release_channel", CHANNEL_RULES)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def served_release(marketplace: str, branch: str) -> str:
    """Выпуск, который раздают оба каталога ветки маркетплейса.

    Сверяет их то же правило, что и конвейер публикации: каталоги, которые
    называют разные выпуски, — отказ, а не выбор одного из них.
    """
    rules = channel_rules()
    with tempfile.TemporaryDirectory(prefix="catalogs-") as tmp:
        paths = []
        for host, catalog in (("codex", rules.CODEX_CATALOG), ("claude", rules.CLAUDE_CATALOG)):
            document = gh(marketplace, f"contents/{catalog.as_posix()}?ref={branch}")[0]
            path = Path(tmp) / f"{host}.json"
            path.write_bytes(base64.b64decode(document["content"]))
            paths.append(path)
        return rules.catalog_ref(*paths)


def channels(repo: str, marketplace: str) -> dict[str, str]:
    """Что раздают канал кандидатов и стабильный каталог на момент сборки.

    Версии называют каталоги веток маркетплейса, а не список релизов. Релиз
    выходит до проверок установки: кандидат, который их не прошёл, остаётся
    релизом, которого канал не раздавал, а стабильный выпуск попадает в
    каталог только после них. Если каталог не прочитался или каталоги
    разошлись, остаётся прочерк: называть версию, которую ветка раздаёт не
    целиком, нельзя, а сайт из-за каталога не должен переставать собираться.
    Адрес маркетплейса едет в страницу, чтобы скрипт освежал версии из того
    же места.
    """
    releases = f"https://github.com/{repo}/releases"
    status = {"marketplace": marketplace, "releases_url": releases}
    for key, branch in (("next", "next"), ("stable", "main")):
        try:
            tag = served_release(marketplace, branch)
        except Exception as error:
            print(f"выпуск ветки {branch} маркетплейса не определён: {error}", file=sys.stderr)
            tag = None
        status[f"{key}_tag"] = tag or "—"
        status[f"{key}_url"] = f"{releases}/tag/{tag}" if tag else releases
    # Версию без `v` печатает `plugin list`; страница сверяет установку по ней.
    status["next_version"] = status["next_tag"].removeprefix("v")
    return status


def release_date(releases: list, tag: str) -> str:
    """Когда вышел релиз этого тега; прочерк, если такого релиза нет."""
    for release in releases:
        if release["tag_name"] == tag:
            return human(moment(release["published_at"]))
    return "—"


def candidates(status: dict[str, object], releases: list) -> list[dict[str, str]]:
    """Кандидат для карточки главной: то, что раздаёт канал, пока он впереди.

    Карточка зовёт поставить кандидата, поэтому показывает выпуск из каталога
    канала, а не свежий пререлиз GitHub: кандидат, не прошедший проверки
    установки, остаётся пререлизом, которого канал не раздавал. Выпуск
    показывается, только если он кандидат `-rc.N` и новее и стабильного
    каталога, и последнего стабильного релиза. Полная версия сначала попадает
    в `next`, потом в `main`, и её нельзя выдать за кандидата; а стабильная
    карточка берёт версию из релиза, который выходит раньше, чем конвейер
    двигает каталоги. Список из нуля или одного элемента: страница не
    показывает «кандидата нет» вместо пустого места.
    """
    rules = channel_rules()
    tag = str(status["next_tag"])
    stable = [str(status["version"]), *([str(status["stable_tag"])] if status["stable_tag"] != "—" else [])]
    try:
        if rules.channel(tag) != "next":
            return []
        ahead = all(rules.precedence(tag) > rules.precedence(other) for other in stable)
    except rules.TagError:
        return []
    if not ahead:
        return []
    return [{"candidate_tag": tag, "candidate_url": str(status["next_url"]), "candidate_date": release_date(releases, tag)}]


def latest_alpha(releases: list, repo: str) -> dict[str, str]:
    """Последний опубликованный alpha для его страницы; прочерки, если их нет."""
    rules = channel_rules()
    found = []
    for release in releases:
        tag = str(release["tag_name"])
        try:
            if rules.channel(tag) == "none" and "-alpha." in tag:
                found.append((rules.precedence(tag), release))
        except rules.TagError:
            continue
    if not found:
        return {
            "alpha_tag": "—",
            "alpha_url": f"https://github.com/{repo}/releases",
            "alpha_date": "—",
            "alpha_commit": "—",
            "alpha_commit_url": f"https://github.com/{repo}/commits/main",
            "alpha_download": f"https://github.com/{repo}/releases",
        }
    release = max(found, key=lambda item: item[0])[1]
    tag = str(release["tag_name"])
    # Срез записан в описании релиза: родитель служебного коммита вне main.
    slice_ = re.search(r"main@([0-9a-f]{40})", str(release.get("body") or ""))
    return {
        "alpha_tag": tag,
        "alpha_url": str(release["html_url"]),
        "alpha_date": human(moment(release["published_at"])),
        "alpha_commit": slice_[1][:7] if slice_ else "—",
        "alpha_commit_url": f"https://github.com/{repo}/commit/{slice_[1]}" if slice_ else str(release["html_url"]),
        "alpha_download": f"https://github.com/{repo}/releases/download/{tag}",
    }


ALPHA_RULES = Path(__file__).with_name("release-alpha.py")
HOST_NAMES = (("ubuntu", "Linux"), ("macos", "macOS"), ("windows", "Windows"))


def alphas_kept() -> int:
    """Сколько alpha держит `release-alpha.py prune`: столько и карточек."""
    spec = importlib.util.spec_from_file_location("release_alpha", ALPHA_RULES)
    module = importlib.util.module_from_spec(spec)
    sys.modules.setdefault("release_alpha", module)
    spec.loader.exec_module(module)
    return int(module.KEEP)


def site_versions(status: dict[str, object], releases: list) -> list[tuple[str, dict]]:
    """Версии карточек сайта: стабильная, кандидат канала next и последние alpha.

    Тесты показываются по версии, а не по ветке: каждая карточка — прогон,
    давший этот тег. Alpha столько, сколько их держит `prune`; удалённая alpha
    уходит со страницы вместе с отчётом.
    """
    by_tag = {str(release["tag_name"]): release for release in releases}
    rules = channel_rules()
    found: list[tuple[str, dict]] = []
    stable = by_tag.get(str(status["version"]))
    if stable is not None:
        found.append(("Стабильная", stable))
    for candidate in candidates(status, releases):
        if candidate["candidate_tag"] in by_tag:
            found.append(("Кандидат", by_tag[candidate["candidate_tag"]]))
    alphas = []
    for release in releases:
        tag = str(release["tag_name"])
        try:
            if rules.channel(tag) == "none" and "-alpha." in tag:
                alphas.append((rules.precedence(tag), release))
        except rules.TagError:
            continue
    for _, release in sorted(alphas, key=lambda item: item[0], reverse=True)[:alphas_kept()]:
        found.append(("Alpha", release))
    return found


def host_name(host: str) -> str:
    return next((name for prefix, name in HOST_NAMES if host.startswith(prefix)), host)


def skipped_by_host(by_host: dict[str, dict[str, int]]) -> str:
    """Пропуски по ОС одной строкой: `Linux 12 · macOS 7356 · Windows 7321`."""
    order = [prefix for prefix, _ in HOST_NAMES]

    def rank(host: str) -> tuple[int, str]:
        return next((index for index, prefix in enumerate(order) if host.startswith(prefix)), len(order)), host

    parts = [f"{host_name(host)} {by_host[host].get('skipped', 0)}" for host in sorted(by_host, key=rank)]
    return " · ".join(parts) if parts else "—"


def version_card(kind: str, release: dict, data: Path | None, repo: str) -> dict[str, str]:
    """Карточка версии: тег, дата, проверенный коммит и тесты прогона на нём.

    Отчёт Allure есть, когда сайт хранит результаты этого тега. Без них
    счётчики берутся из журнала прогона, если он был жив при первой сборке,
    а ссылка ведёт на прогон и честно говорит, что отчёта нет.
    """
    tag = str(release["tag_name"])
    folder = data / "versions" / tag if data is not None else None

    def read(name: str) -> dict:
        path = folder / name if folder is not None else None
        return json.loads(path.read_text(encoding="utf-8")) if path is not None and path.is_file() else {}

    run, legacy, split = read("run.json"), read("legacy.json"), read("breakdown.json")
    counts = summary_counts(folder / "summary.json") if folder is not None else None
    # Alpha проверена ночью на срезе main: её служебный коммит меняет только версию.
    slice_ = re.search(r"main@([0-9a-f]{40})", str(release.get("body") or ""))
    sha = run.get("sha") or legacy.get("sha") or (slice_[1] if slice_ else "")
    card = {
        "version_kind": kind,
        "version_tag": tag,
        "version_notes_url": str(release["html_url"]),
        "version_published": human(moment(release["published_at"])),
        "version_commit": sha[:7] or "—",
        "version_commit_url": f"https://github.com/{repo}/commit/{sha}" if sha else str(release["html_url"]),
    }
    if counts is not None:
        return {**card, **counts, "tests_skipped_hosts": skipped_by_host(split.get("by_host", {})),
                "report_url": f"allure/versions/{tag}/", "report_label": "Отчёт Allure"}
    by_host = legacy.get("by_host", {})
    statistic = {key: sum(item.get(key, 0) for item in by_host.values()) for key in ("total", "passed", "failed", "skipped")}
    shown = statistic_counts(statistic) if by_host else None
    empty = {key: "—" for key in ("tests_total", "tests_passed", "tests_failed", "tests_skipped")}
    words = {"tests_total_word": "тестов", "tests_passed_word": "прошли", "tests_failed_word": "упали",
             "tests_skipped_word": "пропущены"}
    return {**card, **(shown or {**empty, **words}),
            "tests_skipped_hosts": skipped_by_host(by_host) if by_host else "—",
            "report_url": legacy.get("url") or str(release["html_url"]),
            "report_label": "Отчёта нет — счётчики из журнала прогона" if by_host else "Отчёта нет"}


def plural(count: int, one: str, few: str, many: str) -> str:
    """Форма слова при числе: 1 тест, 2 теста, 5 тестов, 11 тестов, 21 тест."""
    if count % 10 == 1 and count % 100 != 11:
        return one
    if 2 <= count % 10 <= 4 and not 12 <= count % 100 <= 14:
        return few
    return many


def summary_counts(path: Path | None) -> dict[str, str] | None:
    """Счётчики берутся из сводки собранного отчёта, а не из воздуха.

    Слово при числе считается здесь же: зашитое в страницу «теста» верно
    только для чисел на 2–4, а «785 теста» и «1 прошли» читаются как ошибка.
    """
    if path is None or not path.is_file():
        return None
    return statistic_counts(json.loads(path.read_text(encoding="utf-8")).get("statistic", {}))


def statistic_counts(statistic: dict) -> dict[str, str]:
    """Счётчики со словами из статистики вида Allure: `total`, `passed`, `failed`, `broken`, `skipped`."""
    total = statistic.get("total", 0)
    passed = statistic.get("passed", 0)
    failed = statistic.get("failed", 0) + statistic.get("broken", 0)
    skipped = statistic.get("skipped", 0)
    return {
        "tests_total": f"{total}",
        "tests_total_word": plural(total, "тест", "теста", "тестов"),
        "tests_passed": f"{passed}",
        "tests_passed_word": plural(passed, "прошёл", "прошли", "прошли"),
        "tests_failed": f"{failed}",
        "tests_failed_word": plural(failed, "упал", "упали", "упали"),
        "tests_skipped": f"{skipped}",
        "tests_skipped_word": plural(skipped, "пропущен", "пропущены", "пропущены"),
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=os.environ.get("GITHUB_REPOSITORY", ""))
    parser.add_argument("--branch", default="main")
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA", ""))
    parser.add_argument("--run-url", default="")
    parser.add_argument("--summaries", type=Path, help="каталог data/ собранного сайта: данные версий")
    parser.add_argument("--print-lines", action="store_true", help="напечатать открытые линии и выйти")
    parser.add_argument("--print-versions", action="store_true", help="напечатать теги карточек версий и выйти")
    parser.add_argument("--telegram", default="unica_ai", help="публичная группа Telegram")
    parser.add_argument(
        "--marketplace",
        default="IngvarConsulting/unica-marketplace",
        help="репозиторий маркетплейса: его ветки next и main называют выпуски каналов",
    )
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()

    if not args.repo:
        raise SystemExit("нужен --repo или GITHUB_REPOSITORY")
    now = datetime.now(timezone.utc)

    if args.print_lines:
        print("\n".join(site_lines(args.branch, args.repo, now)))
        return 0

    releases = [r for r in gh(args.repo, "releases?per_page=100") if not r["draft"]]
    published = [r for r in releases if not r["prerelease"]]
    if not published:
        raise SystemExit("у репозитория нет опубликованных релизов")
    latest = max(published, key=lambda r: moment(r["published_at"]))

    status: dict[str, object] = {
        "version": latest["tag_name"],
        "version_date": human(moment(latest["published_at"])),
        "version_url": latest["html_url"],
        "generated_at": human(now) + now.astimezone(timezone.utc).strftime(" %H:%M"),
        # Страницу собрал либо прогон, либо человек у себя. Во втором случае
        # коммита нет, и подписывать её прочерком — значит промолчать: пусть
        # прямо говорит `local` и ведёт в репозиторий, а не на прогон, которого
        # не было.
        "generated_sha": (args.sha or "")[:7] or "local",
        "generated_url": args.run_url
        or (
            f"https://github.com/{args.repo}/commit/{args.sha}"
            if args.sha
            else f"https://github.com/{args.repo}"
        ),
        "github_stars": github_stars(args.repo),
        "telegram_members": telegram_members(args.telegram),
        **channels(args.repo, args.marketplace),
    }

    status["candidates"] = candidates(status, releases)
    status.update(latest_alpha(releases, args.repo))
    if args.print_versions:
        print("\n".join(str(release["tag_name"]) for _, release in site_versions(status, releases)))
        return 0

    status["versions"] = [
        version_card(kind, release, args.summaries, args.repo) for kind, release in site_versions(status, releases)
    ]

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(status, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"written: {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
