"""Transparent stdio recorder for the exact evaluated MCP binary."""
import hashlib
import json
import subprocess
import sys
import threading
from pathlib import Path


def main():
    binary, log_path, source = map(Path, sys.argv[1:])
    lock = threading.Lock()
    child = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    def record(direction, line):
        payload = json.loads(line)
        with lock, log_path.open("a", encoding="utf-8") as log:
            digest = hashlib.sha256(source.read_bytes()).hexdigest()
            log.write(json.dumps({"direction": direction, "payload": payload,
                                  "sourceSha256": digest}, ensure_ascii=False) + "\n")
    def receive():
        for line in child.stdout:
            record("response", line)
            sys.stdout.buffer.write(line)
            sys.stdout.buffer.flush()
    reader = threading.Thread(target=receive)
    reader.start()
    try:
        for line in sys.stdin.buffer:
            record("request", line)
            child.stdin.write(line)
            child.stdin.flush()
    finally:
        child.stdin.close()
        try:
            child.wait(timeout=10)
        except subprocess.TimeoutExpired:
            child.terminate()
            try:
                child.wait(timeout=10)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=10)
        reader.join(timeout=10)
        if reader.is_alive():
            raise TimeoutError("MCP recorder reader did not stop")


if __name__ == "__main__":
    main()
