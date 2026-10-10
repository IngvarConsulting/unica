"""Transparent stdio recorder for the exact evaluated MCP binary."""
import hashlib
import json
import subprocess
import sys
import threading
from pathlib import Path


def source_file_hashes(source):
    """One physical snapshot; relative paths also detect creation/removal/rename."""
    if source.is_symlink():
        raise ValueError("recorded source must not be a symlink")
    if not source.is_dir():
        return {source.name: hashlib.sha256(source.read_bytes()).hexdigest()}
    entries = {}
    for path in sorted(source.rglob("*")):
        if path.is_symlink():
            raise ValueError("recorded source tree must not contain symlinks")
        if path.is_file():
            entries[path.relative_to(source).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return entries


def tree_digest(entries):
    payload = json.dumps(entries, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def source_digest(source):
    entries = source_file_hashes(source)
    return tree_digest(entries) if source.is_dir() else next(iter(entries.values()))


def source_observation(source, workspace, source_set_roots):
    entries = source_file_hashes(source)
    prefix = source.relative_to(workspace).as_posix()
    files = {f"{prefix}/{name}": digest for name, digest in entries.items()}
    project = workspace / "v8project.yaml"
    if project.is_symlink():
        raise ValueError("recorded project map must not be a symlink")
    project_bytes = project.read_bytes()
    files["v8project.yaml"] = hashlib.sha256(project_bytes).hexdigest()
    roots = {}
    # The corpus supplies its fixed topology; this recorder is not a YAML
    # resolver. Any source-map byte change remains a protected input change.
    for name, declared in source_set_roots.items():
        path = Path(declared)
        if path.is_absolute() or ".." in path.parts or not path.parts:
            raise ValueError("source map path must be relative to the recorded workspace")
        root = path.as_posix()
        if root != prefix and not root.startswith(prefix + "/"):
            raise ValueError("source map root is outside the recorded source tree")
        roots[name] = root
    return {"sourceSha256": tree_digest(entries), "sourceFilesSha256": files,
            "sourceSetRoots": roots}


def main():
    binary, log_path, source, workspace = map(Path, sys.argv[1:5])
    roots = json.loads(sys.argv[5])
    lock = threading.Lock()
    child = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    def record(direction, line):
        payload = json.loads(line)
        with lock, log_path.open("a", encoding="utf-8") as log:
            observation = source_observation(source, workspace, roots)
            log.write(json.dumps({"direction": direction, "payload": payload,
                                  **observation}, ensure_ascii=False) + "\n")
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
