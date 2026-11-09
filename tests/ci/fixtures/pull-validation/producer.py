"""Controlled runner publication protocol; does not execute or impersonate 1C."""
import json
import shutil
import sys
import uuid
from pathlib import Path

if sys.argv[1:] == ["--version"]:
    print("v8-runner 0.15.2 (controlled protocol fixture)")
    raise SystemExit(0)

root = Path.cwd().resolve()
settings = json.loads((root / "pull-fault.json").read_text())
assert settings["case"] in {"unsupported-root", "wrong-version", "invalid-xml", "missing-owner", "empty-dump"}
assert "pull" in sys.argv and "--publication-stdio" in sys.argv
stage = Path(sys.argv[sys.argv.index("--publication-stdio") + 1]).resolve()
relative = stage.relative_to(root)
assert relative.parts[0].startswith(".unica-dump-guard-") and stage.is_dir()
nonce = str(uuid.uuid4())


def emit(phase, **fields):
    print(json.dumps({"version": 1, "nonce": nonce, "phase": phase, **fields}), flush=True)


def decision(phase):
    reply = json.loads(sys.stdin.readline())
    assert reply["version"] == 1 and reply["nonce"] == nonce and reply["phase"] == phase
    return reply["decision"]


emit("selected", provider={"selected": "designer"}, executor={"kind": "local", "executable": settings["platform"], "endpoint": None})
assert decision("selected") == "continue"
preview = "--dry-run" in sys.argv
if not preview:
    # Only the owned stage supplied by the standard publisher is writable.
    shutil.copyfile(Path(__file__).with_name("Configuration.xml"), stage / "Configuration.xml")
    (stage / "keep.bsl").write_text("controlled staged source\n")
    case = settings["case"]
    if case == "unsupported-root":
        (stage / "Unknown.xml").write_text('<UnregisteredDumpRoot xmlns="http://v8.1c.ru/8.3/xcf/extrnprops" version="2.20"/>')
    elif case == "wrong-version":
        owner = stage / "Configuration.xml"
        owner.write_text(owner.read_text(encoding="utf-8-sig").replace('version="2.20"', 'version="2.19"'))
    elif case == "invalid-xml":
        (stage / "Unknown.xml").write_text('<broken password=private-runtime-secret /private/runtime-absolute-marker>')
    elif case == "missing-owner":
        (stage / "Configuration.xml").unlink()
    elif case == "empty-dump":
        for path in stage.iterdir():
            path.unlink()
    emit("ready", command="pull", requested_mode="FULL", mode="FULL", mode_reason=None, extension=None,
         provider_dispatched=True, source_set="main", target_path=str(root / "src"), stage_path=str(stage),
         snapshot="prepared", snapshot_warning=None)
    assert decision("ready") == "rejected"
    print("password=private-runtime-secret /private/runtime-absolute-marker raw-runtime-stderr", file=sys.stderr)
emit("terminal", publication="not_published", memory="unchanged", result={
    "ok": preview, "command": "pull", "duration_ms": 1,
    "data": {"ok": preview, "provider_dispatched": not preview, "source_set": "main", "requested_mode": "FULL", "mode": "FULL",
             "target_path": str(root / "src"), "duration_ms": 1, "message": "controlled runtime fault"},
    "warnings": [], "errors": []})
