#!/usr/bin/env python3
"""Чтение файла с опубликованного сайта: «файла нет» и «сайт не отвечает» — разные ответы.

Сайт — единственная долговременная память результатов. Прочитать сбой сети
как «файла нет» значит молча переиздать сайт без этого файла: архив профиля,
история, память ночного прогона пропали бы насовсем. Поэтому отсутствие —
только ответ 404; всё остальное останавливает сборку, и прежний сайт остаётся
на месте до следующего прогона.
"""

from __future__ import annotations

import urllib.error
import urllib.request
from pathlib import Path


def fetch(url: str, target: Path, timeout: int = 60) -> bool:
    """Скачать файл в `target`; `False` — только когда сайт ответил 404."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            payload = response.read()
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise SystemExit(f"сайт не отвечает: {url}: HTTP {error.code}") from error
    except (urllib.error.URLError, OSError) as error:
        raise SystemExit(f"сайт не отвечает: {url}: {error}") from error
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_bytes(payload)
    return True
