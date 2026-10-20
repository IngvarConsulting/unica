import hashlib,json,os,sys
from pathlib import Path
log,expected,repo=map(Path,sys.argv[1:])
expected,repo=expected.resolve(),repo.resolve()
rows=[json.loads(line) for line in log.read_text().splitlines() if line.strip()]
found=[r for r in rows if r.get("reason")=="compiler-artifact" and r.get("target",{}).get("name")=="unica" and r.get("executable")]
assert len(found)==1,found
r=found[0]
assert Path(r["executable"]).resolve()==expected,r
assert r["fresh"] is False,r
assert Path(r["target"]["src_path"]).resolve()==repo/"crates/unica-coder/src/main.rs",r
assert str(repo/"crates/unica-coder") in r["package_id"],r
print(json.dumps({"compilerArtifact":r,"binarySha256":hashlib.sha256(expected.read_bytes()).hexdigest(),"buildLogSha256":hashlib.sha256(log.read_bytes()).hexdigest(),"environment":{k:os.environ.get(k) for k in ("CARGO_TARGET_DIR","CARGO_PROFILE_DEV_DEBUG","RUSTUP_TOOLCHAIN","RUSTFLAGS")}},indent=2))
