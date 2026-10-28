"""Closed CF→XML test seam over a separate immutable native fixture."""
import hashlib
import json
from pathlib import Path
import shutil
import sys


def convert(arguments):
    if (len(arguments) != 9 or arguments[0] != "--config"
            or arguments[2:4] != ["--json-message", "convert"]
            or arguments[5:8] != ["--to", "xml", "--output"]):
        raise ValueError("unsupported controlled converter invocation")
    config, source, output = map(Path, (arguments[1], arguments[4], arguments[8]))
    config = config.resolve(strict=True)
    source = source.resolve(strict=True)
    root = config.parent.parent
    if (not root.name.startswith("unica-supplier-") or config.parent.name != "project"
            or config.name != "v8project.yaml" or source != root / "supplier.cf"
            or output != root / "export" or output.exists()):
        raise ValueError("controlled conversion requires an isolated fresh output")
    baseline = Path(__file__).resolve().parent / "baseline"
    expected = baseline / "Ext/ParentConfigurations/РеактивныйHTML.cf"
    if hashlib.sha256(source.read_bytes()).digest() != hashlib.sha256(expected.read_bytes()).digest():
        raise ValueError("CF is not the immutable native supplier fixture")
    shutil.copytree(baseline, output)
    return {"ok": True, "command": "convert", "data": {
        "ok": True, "direction": "PACKAGE_TO_DESIGNER", "provider_dispatched": True,
        "provider": {"selected": "ibcmd"},
        "outputs": [{"source_path": str(source), "target_path": str(output)}],
    }}


if __name__ == "__main__":
    try:
        result = convert(sys.argv[1:])
    except (ValueError, OSError) as error:
        print(json.dumps({"ok": False, "command": "convert", "error": {
            "code": "controlled_fixture_refusal", "message": str(error)}}))
        sys.exit(4)
    print(json.dumps(result))
