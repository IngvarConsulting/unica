"""Run S-978 repeatedly through the real acceptance harness; print full failure notes."""
import json, sys, time, unittest
from contextlib import ExitStack
from pathlib import Path
sys.path.insert(0, ".")
import tests.ci.test_acceptance_scenarios as t
from tests.ci.acceptance_diagnostics import prepare_diagnostic_case

plugin = Path(sys.argv[1])
runs = int(sys.argv[2])
only = sys.argv[3] if len(sys.argv) > 3 else "S-978-helper-invalidation"
notes = []
orig = t.classify
def classify(response, context):
    actual, note = orig(response, context)
    if actual != "ok":
        notes.append(json.dumps(response, ensure_ascii=False)[:6000])
    return actual, note
t.classify = classify
t.ensure_binary(Path("."))
corpus = t.select_profile(t.load_acceptance_corpus(), "delivery")
corpus = dict(corpus, scenarios=[s for s in corpus["scenarios"] if s["id"] == only])
assert corpus["scenarios"], only
fails = 0
for run in range(runs):
    notes.clear()
    case = unittest.TestCase()
    started = time.monotonic()
    with ExitStack() as fixtures:
        def prepare(server, scenario):
            return prepare_diagnostic_case(server, scenario, plugin=plugin, fixtures=fixtures)
        try:
            t.run_corpus(case, corpus, plugin, scenario_driver=prepare, fixture_cleanup=fixtures.close)
            print(f"run {run}: ok {time.monotonic()-started:.1f}s", flush=True)
        except AssertionError as error:
            fails += 1
            print(f"run {run}: FAIL {time.monotonic()-started:.1f}s {str(error)[-600:]}", flush=True)
            for note in notes:
                print("RESPONSE", note, flush=True)
print(f"total {runs} fails {fails}")
