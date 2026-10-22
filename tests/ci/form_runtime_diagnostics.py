"""Pinned-analyzer contrast for runtime form attributes; no verdict rewriting."""
from pathlib import Path

WORKSPACE = "tests/fixtures/acceptance/workspace-runtime-form-diagnostics"
DRIVER = "bsl-analyzer-form-diagnostics"
CASES = {"default-bare", "enabled-bare", "enabled-qualified"}
AT = "main:CommonForm.RuntimeAttributes.Module.Form"
MODULE = "CommonForms/RuntimeAttributes/Ext/Form/Module.bsl"
CONFIG = '{"diagnostics":{"parameters":{"UnresolvedName":true}}}\n'
BARE = '"Код,Флаг", RuntimeCode, RuntimeFlag'
QUALIFIED = '"Код,Флаг", ЭтотОбъект.RuntimeCode, ЭтотОбъект.RuntimeFlag'


def valid_scenario(scenario):
    return (scenario.get("workspace") == WORKSPACE
            and scenario.get("evaluation") in CASES
            and all(step["tool"] == "unica.check" and step["args"] == {"at": AT, "limit": 50}
                    for step in scenario["wire"]))


def prepare_case(server, scenario):
    if not valid_scenario(scenario):
        raise ValueError("invalid runtime form diagnostic case")
    source = server.workspace / "src"
    original = Path(__file__).resolve().parents[2] / WORKSPACE / "src" / MODULE
    text = original.read_text(encoding="utf-8")
    if text.count(BARE) != 1:
        raise ValueError("runtime form fixture must contain exactly one pair of bare reads")
    if scenario["evaluation"] == "enabled-qualified":
        text = text.replace(BARE, QUALIFIED)
    (source / MODULE).write_text(text, encoding="utf-8")
    config = source / ".bsl-analyzer.json"
    if scenario["evaluation"] == "default-bare":
        config.unlink(missing_ok=True)
    else:
        config.write_text(CONFIG, encoding="utf-8")
