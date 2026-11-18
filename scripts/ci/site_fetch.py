#!/usr/bin/env python3
"""Чтение файла с опубликованного сайта: «файла нет» и «сайт не отвечает» — разные ответы.

Сайт — единственная долговременная память результатов. Прочитать сбой сети
как «файла нет» значит молча переиздать сайт без этого файла: архив профиля,
история, память ночного прогона пропали бы насовсем. Поэтому отсутствие —
ответ 404; сбой сети и любой другой код останавливают сборку, и прежний сайт
остаётся на месте до следующего прогона.

Одно исключение — 503. GitHub Pages отвечает 503 (страница «Unicorn»)
на заметную долю запросов к отсутствующим путям проектного сайта, тогда как
существующие файлы отдаёт с 200 (#3099). Поэтому 503 повторяется с паузой,
а устойчивый 503 читается как «файла нет» только тогда, когда сайт тут же
отдаёт заведомо существующую страницу `index.html`. Не отдаёт — сайт
недоступен, сборка останавливается, как прежде.
"""

from __future__ import annotations

import time
import urllib.error
import urllib.request
from pathlib import Path

ATTEMPTS = 3
PAUSE = 2.0
ALIVE_PAGE = "index.html"


def fetch(site: str, path: str, target: Path, timeout: int = 60) -> bool:
    """Скачать `site/path` в `target`; `False` — только когда файла на сайте нет."""
    url = f"{site}/{path}"
    for attempt in range(ATTEMPTS):
        if attempt:
            time.sleep(PAUSE)
        status, payload = _get(url, timeout)
        if status != 503:
            break
    if status == 200:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)
        return True
    if status == 404:
        return False
    if status == 503 and _get(f"{site}/{ALIVE_PAGE}", timeout)[0] == 200:
        return False
    raise SystemExit(f"сайт не отвечает: {url}: HTTP {status}")


def _get(url: str, timeout: int) -> tuple[int, bytes]:
    """Код ответа и тело; сбой сети останавливает сборку сразу."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return 200, response.read()
    except urllib.error.HTTPError as error:
        return error.code, b""
    except (urllib.error.URLError, OSError) as error:
        raise SystemExit(f"сайт не отвечает: {url}: {error}") from error
