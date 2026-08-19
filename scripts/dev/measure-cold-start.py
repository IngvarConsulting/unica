#!/usr/bin/env python3
"""Замерить холодный старт Unica на управляемо-медленном канале.

Гейт фазы 1 назначен по худшему замеренному у пользователя каналу — полмегабайта
в секунду, — и принять его нечем, пока нет воспроизводимого замера. Настоящая
сеть для этого не годится: она разная на каждой машине и в каждый час, а число
должно значить одно и то же везде.

Поэтому архив отдаёт локальный сервер, отмеряющий байты по заданной скорости.
Сценарий становится свойством стенда, а не канала: скачивание длится ровно
`размер / скорость`, и два прогона совпадают.

**Почему стенд оркестрирует шаги сам, а не гоняет bootstrap насквозь.** Манифест
прибивает адрес архива к релизам `IngvarConsulting/unica` и отвергает любой
другой источник как «outside the approved release origin». Свойство верное, и
ослаблять его ради замера — цена выше пользы. Поэтому стенд выполняет ту же
работу теми же шагами: тянет архив с отмеренного канала, сверяет сумму,
распаковывает, запускает распакованный бинарь и говорит с ним по проводу.
Оркестровка чужая, работа настоящая.

Отсюда единственное, чего замер не покрывает: накладные расходы самого bootstrap
между шагами. По замеру из #585 это единицы миллисекунд против секунд скачивания.

Полезная нагрузка — настоящий `unica` плюс добивка до нужного размера. Бинарь
настоящий, потому что рукопожатие и `tools/list` должны быть настоящими; добивка
нужна, потому что скачивание меряется объёмом, а не содержимым.

Usage:
    measure-cold-start.py --payload 103M --rate 512K --repeat 2 --out baseline.jsonl
    measure-cold-start.py --payload 6M --rate 512K        # ядро отдельно
"""

from __future__ import annotations

import argparse
import contextlib
import hashlib
import io
import json
import os
import socket
import subprocess
import sys
import tarfile
import tempfile
import threading
import time
import urllib.request
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
# Отдаём мелкими кусками: чем крупнее шаг, тем грубее выдерживается скорость на
# коротких нагрузках, а тест сравнивает именно длительность.
CHUNK = 16 * 1024


def parse_size(value: str) -> int:
    """`512K`, `6M`, `103M` или голое число байтов."""
    text = value.strip().upper()
    for suffix, factor in (("K", 1024), ("M", 1024**2), ("G", 1024**3)):
        if text.endswith(suffix):
            return int(float(text[:-1]) * factor)
    return int(text)


@dataclass(frozen=True)
class Server:
    url: str
    served_at: "list[float]"


class _Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args) -> None:  # тишина: стенд пишет своё
        pass

    def _range(self) -> "tuple[int, int] | None":
        header = self.headers.get("Range")
        if not header or not header.startswith("bytes="):
            return None
        start, _, end = header[len("bytes="):].partition("-")
        payload = self.server.payload
        first = int(start) if start else 0
        last = int(end) if end else len(payload) - 1
        return first, min(last, len(payload) - 1)

    def do_GET(self) -> None:  # noqa: N802 — имя задано базовым классом
        payload = self.server.payload
        window = self._range()
        if window is None:
            first, last = 0, len(payload) - 1
            self.send_response(200)
        else:
            first, last = window
            self.send_response(206)
            self.send_header("Content-Range", f"bytes {first}-{last}/{len(payload)}")
        body = payload[first : last + 1]
        self.send_header("Content-Type", "application/gzip")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Accept-Ranges", "bytes")
        self.end_headers()

        rate = self.server.rate
        started = time.monotonic()
        sent = 0
        while sent < len(body):
            chunk = body[sent : sent + CHUNK]
            try:
                self.wfile.write(chunk)
            except (BrokenPipeError, ConnectionResetError):
                return
            sent += len(chunk)
            if rate:
                # Спим до момента, когда отданный объём соответствует скорости.
                behind = sent / rate - (time.monotonic() - started)
                if behind > 0:
                    time.sleep(behind)
        self.server.served_at.append(time.monotonic())


@contextlib.contextmanager
def serve_throttled(path: Path, rate: int):
    """Локальный сервер, отдающий файл не быстрее `rate` байт в секунду."""
    payload = path.read_bytes()
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]

    httpd = HTTPServer(("127.0.0.1", port), _Handler)
    httpd.payload = payload
    httpd.rate = rate
    httpd.served_at = []
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield Server(url=f"http://127.0.0.1:{port}/runtime.tar.gz", served_at=httpd.served_at)
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)


def fetch_all(url: str) -> "tuple[float, int]":
    started = time.monotonic()
    with urllib.request.urlopen(url) as response:
        body = response.read()
    return time.monotonic() - started, len(body)


def fetch_range(url: str, start: int) -> bytes:
    request = urllib.request.Request(url, headers={"Range": f"bytes={start}-"})
    with urllib.request.urlopen(request) as response:
        return response.read()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def build_archive(runtime: Path, payload_bytes: int, out: Path) -> "tuple[list[dict], str]":
    """Архив с настоящим `unica` и добивкой до заданного объёма."""
    binary = runtime.read_bytes()
    members = [("unica", binary, True)]
    padding = max(0, payload_bytes - len(binary))
    if padding:
        # Несжимаемая добивка: объём в архиве должен совпадать с заказанным,
        # иначе скачивание померяет не тот сценарий.
        members.append(("padding.bin", os.urandom(padding), False))

    with out.open("wb") as raw:
        with tarfile.open(fileobj=raw, mode="w:gz", format=tarfile.PAX_FORMAT) as archive:
            for name, data, executable in members:
                info = tarfile.TarInfo(name)
                info.size = len(data)
                info.mode = 0o755 if executable else 0o644
                info.mtime = 0
                archive.addfile(info, io.BytesIO(data))

    files = [
        {"path": name, "sha256": sha256_bytes(data), "executable": executable}
        for name, data, executable in members
    ]
    return files, sha256_bytes(out.read_bytes())


def _rpc(process: subprocess.Popen, message: dict) -> dict:
    process.stdin.write(json.dumps(message) + "\n")
    process.stdin.flush()
    while True:
        line = process.stdout.readline()
        if not line:
            raise RuntimeError("рантайм закрыл провод, не ответив")
        answer = json.loads(line)
        if answer.get("id") == message["id"]:
            return answer


def measure(runtime: Path, payload_bytes: int, rate: int) -> dict:
    """Один холодный прогон: отмеренный канал, свежая распаковка, живой провод."""
    with tempfile.TemporaryDirectory() as raw:
        workspace = Path(raw)
        archive = workspace / "runtime.tar.gz"
        unpacked = workspace / "runtime"
        unpacked.mkdir()

        files, archive_sha = build_archive(runtime, payload_bytes, archive)
        transferred = archive.stat().st_size

        with serve_throttled(archive, rate) as server:
            started = time.monotonic()
            _, received = fetch_all(server.url)
            downloaded_at = time.monotonic()

        blob = archive.read_bytes()
        if sha256_bytes(blob) != archive_sha:
            raise RuntimeError("контрольная сумма архива не сошлась")
        verified_at = time.monotonic()

        with tarfile.open(archive, "r:gz") as bundle:
            bundle.extractall(unpacked, filter="data")
        entrypoint = unpacked / "unica"
        entrypoint.chmod(0o755)
        extracted_at = time.monotonic()

        process = subprocess.Popen(
            [str(entrypoint)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            text=True,
        )
        try:
            handshake = _rpc(process, {
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "measure-cold-start", "version": "0"},
                },
            })
            initialized_at = time.monotonic()
            listing = _rpc(process, {
                "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {},
            })
            listed_at = time.monotonic()
        finally:
            with contextlib.suppress(Exception):
                process.stdin.close()
            process.terminate()
            with contextlib.suppress(subprocess.TimeoutExpired):
                process.wait(timeout=5)

    return {
        "payload": payload_bytes,
        "transferred": transferred,
        "received": received,
        "rate": rate,
        "download": round(downloaded_at - started, 3),
        "verify": round(verified_at - downloaded_at, 3),
        "extract": round(extracted_at - verified_at, 3),
        "spawn_initialize": round(initialized_at - extracted_at, 3),
        "tools_list": round(listed_at - initialized_at, 3),
        "total": round(listed_at - started, 3),
        "server": handshake["result"]["serverInfo"]["name"],
        "tools": len(listing["result"]["tools"]),
    }


def main(argv: "list[str] | None" = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--payload", type=parse_size, default="6M",
                        help="объём архива: 6M — ядро, 103M — сегодняшняя поставка")
    parser.add_argument("--rate", type=parse_size, default="512K",
                        help="скорость канала, байт в секунду")
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--runtime", type=Path,
                        default=REPO_ROOT / "target" / "release" / "unica")
    arguments = parser.parse_args(argv)

    if not arguments.runtime.is_file():
        print(f"нет бинаря {arguments.runtime}; соберите cargo build --release", file=sys.stderr)
        return 2

    lines = []
    for run in range(arguments.repeat):
        result = measure(
            runtime=arguments.runtime,
            payload_bytes=arguments.payload, rate=arguments.rate,
        )
        result["run"] = run
        lines.append(result)
        print(
            f"прогон {run}: всего {result['total']:.2f} с "
            f"= скачивание {result['download']:.2f}"
            f" + сумма {result['verify']:.2f}"
            f" + распаковка {result['extract']:.2f}"
            f" + запуск и рукопожатие {result['spawn_initialize']:.2f}"
            f" + перечисление {result['tools_list']:.2f}"
            f"  ({result['tools']} инструментов)"
        )

    if arguments.out:
        arguments.out.write_text(
            "".join(json.dumps(line, ensure_ascii=False) + "\n" for line in lines),
            encoding="utf-8",
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
