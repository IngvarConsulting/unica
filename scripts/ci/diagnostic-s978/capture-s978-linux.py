#!/usr/bin/env python3
"""Ignored S978-only diagnostic capture; fixed ten fresh observations, no retries."""
import argparse
from contextlib import ExitStack
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import time
import traceback
import unittest
from unittest.mock import patch

NAMES = ("UNICA_CACHE_DIR", "UNICA_PROVIDER_STATE_DIR", "UNICA_ARTIFACT_CACHE",
         "UNICA_PLUGIN_ROOT", "XDG_RUNTIME_DIR", "TMPDIR", "BSL_MCP_BROKER",
         "BSL_MCP_IDLE_TTL_SECS", "BSL_MCP_ORPHAN_GRACE_SECS",
         "HOME", "PATH", "XDG_CACHE_HOME", "XDG_STATE_HOME")

def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

CAPTURE_STATE = {"output":None,"observationsPlanned":10,"results":[],"captureErrors":[],"stage":"arguments"}

def progress(status="inProgress"):
    report = {"captureStatus":status,"stage":CAPTURE_STATE["stage"],
              "sourceRef":CAPTURE_STATE.get("sourceRef"),
              "observationsPlanned":CAPTURE_STATE["observationsPlanned"],
              "observationsActual":len(CAPTURE_STATE["results"]),
              "currentObservation":CAPTURE_STATE.get("currentObservation"),
              "results":CAPTURE_STATE["results"],"captureErrors":CAPTURE_STATE["captureErrors"],
              "scope":"S978-only fixed-count diagnostic; all observed failures retained"}
    output=CAPTURE_STATE["output"]
    if output is not None:
        try:
            (output/"summary.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
        except Exception as error:
            issue={"operation":"summary-write","error":repr(error)}
            CAPTURE_STATE["captureErrors"].append(issue)
            report["captureStatus"]="captureIncomplete"
            report["summaryWriteError"]=issue
    print(json.dumps({"captureProgress":report},ensure_ascii=False),flush=True)
    return report

def safe_main():
    try:
        return main()
    except KeyboardInterrupt as error:
        CAPTURE_STATE["captureErrors"].append({"operation":"interrupted","error":repr(error)})
        progress("captureIncomplete")
        return 130
    except SystemExit as error:
        # argparse --help remains successful; explicit/argument gates are incomplete captures.
        if error.code==0:return 0
        CAPTURE_STATE["captureErrors"].append({"operation":"preflight-exit","error":str(error.code)})
        progress("captureIncomplete")
        return 2
    except Exception as error:
        CAPTURE_STATE["captureErrors"].append({"operation":"unhandled-capture","error":repr(error),"traceback":traceback.format_exc()})
        progress("captureIncomplete")
        return 2

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", type=Path, required=True)
    ap.add_argument("--source-ref", choices=("80ad95b3fe8e46bd8d9964387c10af19566d68ea",
                                             "47cdae67e8248432ff539c554bc7023065ebed79"), required=True)
    ap.add_argument("--binary", type=Path, required=True)
    ap.add_argument("--binary-sha256", required=True)
    ap.add_argument("--plugin", type=Path, required=True)
    ap.add_argument("--proposal", type=Path, required=True)
    ap.add_argument("--output", type=Path, required=True)
    ap.add_argument("--operational-manifest", type=Path)
    args = ap.parse_args()
    if platform.system() != "Linux":
        raise SystemExit("This is the symmetric Linux diagnostic, not a cross-OS result")
    repo, binary, plugin, proposal = (x.resolve() for x in (args.repo,args.binary,args.plugin,args.proposal))
    out = args.output.resolve()
    CAPTURE_STATE.update(sourceRef=args.source_ref,requestedOutput=str(out),stage="preflight")
    out.mkdir(parents=True, exist_ok=False)
    CAPTURE_STATE["output"]=out
    progress()
    meta = json.loads((proposal / "proposal.json").read_text())
    if sha(proposal / "trace-only.patch") != meta["patchSha256"]:
        raise SystemExit("proposal patch changed")
    if sha(__file__) != meta["controllerSha256"]:
        raise SystemExit("controller differs from reviewed proposal")
    operational = {"files":{}}
    if args.operational_manifest:
        operational = json.loads(args.operational_manifest.read_text())
        if set(operational) != {"files"} or set(operational["files"]) - {".github/workflows/unica-plugin-release.yml"}:
            raise SystemExit("only the reviewed dispatch-only workflow may be operational source delta")
    def validate_source():
        changed = subprocess.check_output(["git","diff","--name-only",args.source_ref,"--"],cwd=repo).decode().splitlines()
        expected_changed = set(meta["sourceAfterSha256"])
        rows = {}
        for name, expected in meta["sourceAfterSha256"].items():
            if sha(repo/name) != expected:
                raise ValueError("applied trace source mismatch: " + name)
            before = subprocess.check_output(["git","show",args.source_ref+":"+name],cwd=repo)
            if hashlib.sha256(before).hexdigest() != meta["sourceBeforeSha256"][name]:
                raise ValueError("source base mismatch: " + name)
            rows[name] = {"baseSha256":meta["sourceBeforeSha256"][name],"currentSha256":expected,"kind":"reviewed trace-only source"}
        for name, entry in operational["files"].items():
            if set(entry) != {"sha256","purpose"} or not entry["purpose"]:
                raise ValueError("operational manifest needs exact SHA and purpose")
            if sha(repo/name) != entry["sha256"]:
                raise ValueError("dispatch workflow differs from reviewed operational manifest")
            before = subprocess.check_output(["git","show",args.source_ref+":"+name],cwd=repo)
            base_sha = hashlib.sha256(before).hexdigest()
            rows[name] = {"baseSha256":base_sha,"currentSha256":entry["sha256"],"kind":entry["purpose"]}
            if base_sha != entry["sha256"]:expected_changed.add(name)
        if set(changed) != expected_changed:
            raise ValueError("unexpected tracked source delta: " + repr(changed))
        return rows
    source_before = validate_source()
    source_delta = subprocess.check_output(["git","diff","--binary",args.source_ref,"--"],cwd=repo)
    if sha(binary) != args.binary_sha256:
        raise SystemExit("provided executable hash mismatch")
    lock = json.loads((repo / "plugins/unica/third-party/tools.lock.json").read_text())
    tool = next(t for t in lock["tools"] if t["name"] == "bsl-analyzer")
    analyzer = plugin / "bin/linux-x64/bsl-analyzer"
    if sha(analyzer) != tool["assets"]["linux-x64"]["sha256"]:
        raise SystemExit("staged analyzer differs from published pin")
    if (plugin / "third-party/tools.lock.json").read_bytes() != (repo / "plugins/unica/third-party/tools.lock.json").read_bytes():
        raise SystemExit("staged lock differs from source")
    (out / "source.patch").write_bytes(source_delta)
    (out / "provenance.json").write_text(json.dumps({
        "sourceRef":args.source_ref,"head":subprocess.check_output(["git","rev-parse","HEAD"],cwd=repo).decode().strip(),
        "binary":str(binary),"binarySha256":sha(binary),"analyzer":str(analyzer),"analyzerSha256":sha(analyzer),
        "controllerSha256":sha(__file__),"proposalPatchSha256":meta["patchSha256"],
        "sourceBefore":source_before,"operationalManifest":operational,
        "operationalManifestSha256":sha(args.operational_manifest) if args.operational_manifest else None,
        "environment":{k:os.environ.get(k) for k in NAMES},"observationsPlanned":10,
        "scope":"S978 only; not the original ordered full delivery testcase; sourceRef plus explicit trace-only patch",
    },ensure_ascii=False,indent=2)+"\n")
    sys.path.insert(0,str(repo))
    from tests.ci import test_acceptance_scenarios as ac
    from tests.ci import acceptance_diagnostics as ad
    ac.BINARY = binary
    original_call, original_close, original_popen = ac.AcceptanceServer.call, ac.DeliveredAcceptanceServer.close, subprocess.Popen
    corpus = ac.load_acceptance_corpus()
    scenario = next(s for s in corpus["scenarios"] if s["id"] == "S-978-helper-invalidation")
    corpus = {**corpus,"scenarios":[scenario]}
    summary = []
    CAPTURE_STATE["results"]=summary
    for observation in range(1,11):
        CAPTURE_STATE.update(currentObservation=observation,stage="observation-setup")
        progress()
        run = out / f"observation-{observation:02d}"
        run.mkdir()
        rust = run / "producer"
        rust.mkdir()
        errors, known_logs, known_caches = [], set(), set()
        sequence = [0]
        snapshots = [0]
        def append(name, value):
            try:
                with (run/name).open("a",encoding="utf-8") as stream:
                    stream.write(json.dumps(value,ensure_ascii=False)+"\n")
            except Exception as error:
                errors.append({"operation":"append","error":repr(error)})
        def snapshot(server, reason):
            try:
                snapshots[0] += 1
                dest = run / "snapshots" / f"{snapshots[0]:04d}"
                dest.mkdir(parents=True)
                state = server.stderr_path.parent
                known_logs.add(server.stderr_path)
                paths = set(known_logs)
                for cache in known_caches:
                    paths.add(cache/"writer.lease")
                for root in (state, server.workspace/".build/unica"):
                    if root.is_dir():
                        for name in ("service.json","service.stderr.log","writer.lease"):
                            paths.update(root.rglob(name))
                records = []
                pids = {server.process.pid}
                for path in sorted(paths):
                    row = {"path":str(path)}
                    try:
                        data = path.read_bytes()
                        target = dest / (hashlib.sha256(os.fsencode(path)).hexdigest()[:16] + "-" + path.name)
                        target.write_bytes(data)
                        row.update(saved=str(target.relative_to(run)),sha256=hashlib.sha256(data).hexdigest())
                        if path.name in ("service.json","writer.lease"):
                            try:
                                pid = json.loads(data).get("pid")
                                if isinstance(pid,int): pids.add(pid)
                            except (ValueError,UnicodeDecodeError): pass
                    except OSError as error: row["missing"] = repr(error)
                    records.append(row)
                for file in rust.glob("rust-*.jsonl"):
                    for line in file.read_text().splitlines():
                        try:
                            event = json.loads(line).get("event",{})
                            pid = event.get("helperPid")
                            if isinstance(pid,int): pids.add(pid)
                        except ValueError: pass
                processes = []
                for pid in sorted(pids):
                    row = {"pid":pid}
                    try:
                        proc = Path("/proc")/str(pid)
                        row["argv"] = [x.decode("utf-8",errors="replace") for x in (proc/"cmdline").read_bytes().split(b"\0") if x]
                        row["cwd"] = os.readlink(proc/"cwd")
                        environ = dict(x.split(b"=",1) for x in (proc/"environ").read_bytes().split(b"\0") if b"=" in x)
                        row["environment"] = {k:environ.get(k.encode(),b"").decode("utf-8",errors="replace") if k.encode() in environ else None for k in NAMES}
                    except OSError as error: row["missing"] = repr(error)
                    processes.append(row)
                (dest/"index.json").write_text(json.dumps({"reason":reason,"workspace":str(server.workspace),"state":str(state),"files":records,"processes":processes},ensure_ascii=False,indent=2)+"\n")
            except Exception as error: errors.append({"operation":"snapshot","error":repr(error)})
        def observed_call(server,*pos,**kw):
            sequence[0]+=1
            record={"sequence":sequence[0],"label":pos[2] if len(pos)>2 else kw.get("label"),"tool":pos[0] if pos else None,"args":pos[1] if len(pos)>1 else None,"timeout":kw.get("timeout"),"monotonicBefore":time.monotonic(),"frontendPid":server.process.pid,"workspace":str(server.workspace)}
            try:
                response=original_call(server,*pos,**kw)
            except BaseException as error:
                record.update(monotonicAfter=time.monotonic(),exception={"type":type(error).__name__,"message":str(error)})
                append("public-calls.jsonl",record);snapshot(server,"call-exception")
                raise
            record.update(monotonicAfter=time.monotonic(),response=response)
            append("public-calls.jsonl",record);snapshot(server,"after-call")
            return response
        def observed_close(server,*pos,**kw):
            if not getattr(server,"_closed",False): snapshot(server,"before-close")
            return original_close(server,*pos,**kw)
        def observed_popen(*pos,**kw):
            try: inherited_cwd={"path":os.getcwd()}
            except OSError as error: inherited_cwd={"error":repr(error)}
            process=original_popen(*pos,**kw)
            try:
                argv=pos[0] if pos else kw.get("args")
                if isinstance(argv,(list,tuple)) and argv:
                    argv=[os.fsdecode(x) for x in argv]
                    if Path(argv[0]).resolve()==binary or Path(argv[0]).name=="bsl-analyzer":
                        environment=kw.get("env") if kw.get("env") is not None else os.environ
                        stderr=kw.get("stderr")
                        name=getattr(stderr,"name",None)
                        if isinstance(name,(str,os.PathLike)):known_logs.add(Path(name))
                        if "--cache-dir" in argv:known_caches.add(Path(argv[argv.index("--cache-dir")+1]))
                        append("python-spawns.jsonl",{"pid":process.pid,"argv":argv,"cwd":str(kw.get("cwd")) if kw.get("cwd") is not None else None,"inheritedCwd":inherited_cwd,"effectiveCwd":{"path":str(kw["cwd"])} if kw.get("cwd") is not None else inherited_cwd,"environment":{k:environment.get(k) for k in NAMES},"stderrPath":os.fsdecode(name) if isinstance(name,(str,os.PathLike)) else None})
            except Exception as error: errors.append({"operation":"spawn-observer","error":repr(error)})
            return process
        started=time.monotonic()
        outcome={"observation":observation,"binarySha256Before":sha(binary)}
        if outcome["binarySha256Before"]!=args.binary_sha256:raise SystemExit("binary changed before observation")
        CAPTURE_STATE["stage"]="scenario"
        try:
            with patch.dict(os.environ,{"UNICA_S978_TRACE_DIR":str(rust)}), patch.object(ac.AcceptanceServer,"call",observed_call), patch.object(ac.DeliveredAcceptanceServer,"close",observed_close), patch.object(subprocess,"Popen",observed_popen):
                with ExitStack() as fixtures:
                    def prepare(server, selected):
                        return ad.prepare_diagnostic_case(server,selected,plugin=plugin,fixtures=fixtures)
                    ac.run_corpus(unittest.TestCase(),corpus,plugin,scenario_driver=prepare,fixture_cleanup=fixtures.close)
            outcome["status"]="PASS"
        except Exception as error:
            outcome.update(status="FAIL",exception={"type":type(error).__name__,"message":str(error),"traceback":traceback.format_exc()})
        outcome.update(elapsedSeconds=time.monotonic()-started,binarySha256After=sha(binary),captureErrors=errors)
        if outcome["binarySha256After"]!=args.binary_sha256:outcome["captureErrors"].append({"operation":"binary-origin","error":"binary changed during observation"})
        summary.append(outcome)
        CAPTURE_STATE["stage"]="observation-collector"
        progress()
        try:
            producer=[]
            producer_rows=[]
            for file in rust.glob("rust-*.jsonl"):
                for line in file.read_text().splitlines():
                    try:
                        row=json.loads(line)
                        producer_rows.append(row)
                        producer.append(row["event"])
                    except (ValueError,KeyError) as error:outcome["captureErrors"].append({"operation":"producer-json","error":repr(error)})
            outcome["producerKinds"]={kind:sum(e.get("kind")==kind for e in producer) for kind in {e.get("kind") for e in producer}}
            pairs={}
            for row in producer_rows:
                event=row["event"]
                if event.get("kind") in {"bslToolStart","bslToolEnd"}:
                    key=(row.get("processPid"),event.get("callId"))
                    if None in key:
                        outcome["captureErrors"].append({"operation":"producer-pairing","error":"missing processPid/callId"})
                        continue
                    slot=pairs.setdefault(key,{"start":0,"end":0})
                    slot["start" if event["kind"]=="bslToolStart" else "end"]+=1
            outcome["producerPairing"]=[{"processPid":key[0],"callId":key[1],**value} for key,value in sorted(pairs.items())]
            if not pairs or any(value!={"start":1,"end":1} for value in pairs.values()):
                outcome["captureErrors"].append({"operation":"producer-pairing","error":"private calls do not have exactly one start and end each"})
            if pairs and not any(e.get("kind") in {"persistentMcpSpawned","persistentMcpSpawnFailed"} for e in producer):
                outcome["captureErrors"].append({"operation":"spawn-coverage","error":"no final-command spawn outcome recorded"})
            for file in (run/"snapshots").rglob("*") if (run/"snapshots").exists() else []:
                if file.is_file() and b"UNICA_S978_TRACE_WRITE_FAILED" in file.read_bytes():
                    outcome["captureErrors"].append({"operation":"trace-write","error":"Rust trace-write failure marker in "+str(file.relative_to(run))})
        except Exception as error:
            outcome["captureErrors"].append({"operation":"final-collector","error":repr(error),"traceback":traceback.format_exc()})
        try:
            (run/"outcome.json").write_text(json.dumps(outcome,ensure_ascii=False,indent=2)+"\n")
        except Exception as error:
            outcome["captureErrors"].append({"operation":"outcome-write","error":repr(error)})
        progress("captureIncomplete" if outcome["captureErrors"] else "inProgress")
    CAPTURE_STATE["stage"]="source-after"
    source_after_error=None
    try:
        source_after=validate_source()
        (out/"source-after.json").write_text(json.dumps({"sourceAfter":source_after,"error":None},ensure_ascii=False,indent=2)+"\n")
    except Exception as error:
        source_after=None
        source_after_error=repr(error)
        CAPTURE_STATE["captureErrors"].append({"operation":"source-after","error":source_after_error})
    if source_after is not None and source_after!=source_before:
        CAPTURE_STATE["captureErrors"].append({"operation":"source-after","error":"source state changed during observations"})
    incomplete = bool(CAPTURE_STATE["captureErrors"] or any(o["captureErrors"] for o in summary) or len(summary)!=10)
    CAPTURE_STATE["stage"]="terminal"
    progress("captureIncomplete" if incomplete else "complete")
    if incomplete or CAPTURE_STATE["captureErrors"]:return 2
    return int(any(o["status"]!="PASS" for o in summary))

if __name__=="__main__":
    sys.exit(safe_main())
