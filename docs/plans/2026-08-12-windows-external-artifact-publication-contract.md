# Windows External Artifact Publication Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a deterministic Windows consumer contract that reproduces the Unica 0.11.0 `v8-runner make` directory-fsync failure and proves the currently locked runner publishes and replaces external EPF output directories cleanly.

**Architecture:** Extend the existing packaged-tool contract script with one Windows-only end-to-end check. It compiles a tiny Rust Designer stub in a temporary directory, runs the real packaged `v8-runner` twice against a one-object `EXTERNAL_DATA_PROCESSORS` source set, and validates the JSON envelope, published bytes, replacement semantics, and cleanup. Python unit tests protect result validation and ensure the Windows check is wired into targeted tool contracts.

**Tech Stack:** Python 3.12 standard library, `unittest`, Rust `rustc` platform stub, packaged `v8-runner` 0.5.1, Windows filesystem semantics.

## Global Constraints

- Work only on branch `codex/issue-310-windows-publication-contract` in the isolated issue worktree.
- Preserve the public `unica.*` MCP surface, tool lock, package manifests, runtime budgets, and staged-publication policy.
- Direct `v8-runner` execution is limited to this maintainer/packaged-tool contract; user-facing guidance remains MCP-first.
- Do not require a real 1C installation, license, credentials, network, or persistent information base.
- The check is mandatory only when `target == "win-x64"`; non-Windows targeted runs do not execute it.
- The old runtime is diagnostic input only and is never copied into the repository.
- Every defect-facing production helper is introduced through a witnessed RED test.

---

### Task 1: Define the external publication result contract

**Files:**
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `scripts/ci/check-tool-contracts.py`

**Interfaces:**
- Consumes: a parsed runner JSON envelope, resolved output directory, expected EPF path and bytes, and the fixture root.
- Produces: `validate_v8_runner_windows_external_publication_result(envelope: object, output_dir: Path, expected_epf: Path, expected_bytes: bytes, fixture_root: Path) -> list[str]`.

- [ ] **Step 1: Write the failing validator tests**

Add two tests to `ProductContractTests`. The positive fixture uses hand-derived literals and real files:

```python
def test_v8_runner_windows_external_publication_result_accepts_clean_epf(self) -> None:
    module = load_contract_module()
    validator = getattr(
        module,
        "validate_v8_runner_windows_external_publication_result",
        None,
    )
    self.assertIsNotNone(validator)

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        output = root / "Deploy"
        epf = output / "Alpha.epf"
        output.mkdir()
        epf.write_bytes(b"issue-310-current")
        envelope = {
            "ok": True,
            "command": "make",
            "data": {
                "ok": True,
                "mode": "external_data_processor_epf",
                "source_set": "external-processors",
                "output_path": str(output),
                "artifacts": {
                    "root_dir": str(output),
                    "items": [
                        {
                            "kind": "package",
                            "path": str(epf),
                            "role": "package_file",
                        }
                    ],
                },
                "execution": {
                    "status": "succeeded",
                    "payload": {
                        "artifact_type": "external_data_processor_epf",
                        "output_path": str(output),
                        "file_names": ["Alpha.epf"],
                        "published": True,
                    },
                },
            },
        }

        self.assertEqual(
            validator(envelope, output, epf, b"issue-310-current", root),
            [],
        )
```

The negative fixture is explicit and catches three independent mutations:

```python
def test_v8_runner_windows_external_publication_result_rejects_failed_or_dirty_publish(
    self,
) -> None:
    module = load_contract_module()
    validator = module.validate_v8_runner_windows_external_publication_result

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        output = root / "Deploy"
        epf = output / "Alpha.epf"
        output.mkdir()
        epf.write_bytes(b"issue-310-stale")
        (root / ".artifacts-stage-leftover").mkdir()
        envelope = {
            "ok": False,
            "data": {
                "ok": False,
                "mode": "external_data_processor_epf",
                "source_set": "external-processors",
                "output_path": str(output),
                "execution": {"status": "failed"},
            },
        }

        errors = validator(envelope, output, epf, b"issue-310-current", root)

    self.assertTrue(any("envelope" in error for error in errors), errors)
    self.assertTrue(any("unexpected bytes" in error for error in errors), errors)
    self.assertTrue(any("temporary state" in error for error in errors), errors)
```

- [ ] **Step 2: Run the validator tests and witness RED**

Run:

```powershell
python tests/ci/test_product_contracts.py ProductContractTests.test_v8_runner_windows_external_publication_result_accepts_clean_epf ProductContractTests.test_v8_runner_windows_external_publication_result_rejects_failed_or_dirty_publish
```

Expected: FAIL because `validate_v8_runner_windows_external_publication_result` does not exist. The failure must not be an import, fixture, or syntax error.

- [ ] **Step 3: Implement the minimal validator**

Add the function beside the existing v8-runner validators. It must validate literal consumer behavior:

```python
def validate_v8_runner_windows_external_publication_result(
    envelope: object,
    output_dir: Path,
    expected_epf: Path,
    expected_bytes: bytes,
    fixture_root: Path,
) -> list[str]:
    errors: list[str] = []
    data = envelope.get("data") if isinstance(envelope, dict) else None
    if not isinstance(envelope, dict) or envelope.get("ok") is not True:
        errors.append("runner JSON envelope is not successful")
    if not isinstance(data, dict) or data.get("ok") is not True:
        errors.append("runner JSON data is not successful")
        return errors
    if data.get("mode") != "external_data_processor_epf":
        errors.append(f"runner JSON mode is not external_data_processor_epf: {data.get('mode')!r}")
    if data.get("source_set") != "external-processors":
        errors.append(f"runner JSON source_set is not external-processors: {data.get('source_set')!r}")
    actual_output = data.get("output_path")
    if not isinstance(actual_output, str) or Path(actual_output).resolve() != output_dir.resolve():
        errors.append(f"runner JSON output_path does not resolve to {output_dir.resolve()}")
    execution = data.get("execution")
    if not isinstance(execution, dict) or execution.get("status") != "succeeded":
        errors.append("runner execution status is not succeeded")
    payload = execution.get("payload") if isinstance(execution, dict) else None
    if not isinstance(payload, dict) or payload.get("published") is not True:
        errors.append("runner execution payload is not published")
    if not expected_epf.is_file():
        errors.append(f"published EPF was not created: {expected_epf}")
    elif expected_epf.read_bytes() != expected_bytes:
        errors.append(f"published EPF has unexpected bytes: {expected_epf}")
    retained = sorted(
        path.name
        for path in fixture_root.iterdir()
        if path.name.startswith((".artifacts-stage-", ".artifacts-backup-"))
        or (
            path.name.startswith(".artifacts-")
            and path.name.endswith(".meta.json")
        )
    )
    if retained:
        errors.append(f"publication temporary state was retained: {retained}")
    return errors
```

Also verify `data.artifacts.items` contains a `package_file` whose resolved path equals `expected_epf`, and `payload.file_names` equals `['Alpha.epf']`. Keep filesystem read errors as returned diagnostic strings rather than uncaught exceptions.

- [ ] **Step 4: Run the validator tests and witness GREEN**

Run the command from Step 2.

Expected: both tests PASS.

- [ ] **Step 5: Commit the validator cycle**

```powershell
git add tests/ci/test_product_contracts.py scripts/ci/check-tool-contracts.py
git commit -m "test: define Windows external publication result contract"
```

---

### Task 2: Execute packaged `v8-runner make` on Windows

**Files:**
- Modify: `tests/ci/test_product_contracts.py`
- Modify: `scripts/ci/check-tool-contracts.py`

**Interfaces:**
- Consumes: the packaged runner path and target name passed by `check_tool_contracts`.
- Produces: `check_v8_runner_windows_external_publication_contract(runner: Path, target: str) -> list[str]`; returns no errors for non-`win-x64` targets.

- [ ] **Step 1: Write the failing routing test**

Extend the targeted tool-contract test so the real dispatcher is exercised while only external processes are replaced:

```python
with (
    patch.object(module, "TOOL_HELP_CHECKS", []),
    patch.object(module, "check_v8_runner_partial_load_contract", return_value=[]),
    patch.object(module, "check_v8_runner_bounded_external_epf_contract", return_value=[]),
    patch.object(
        module,
        "check_v8_runner_windows_external_publication_contract",
        return_value=["windows publication failure"],
    ) as publication_check,
):
    errors = module.check_tool_contracts(tools_dir, "win-x64")

self.assertEqual(errors, ["windows publication failure"])
publication_check.assert_called_once_with(runner.resolve(), "win-x64")
```

Use `v8-runner.exe` as the fixture filename so the production resolver chooses the same path as Windows packaging.

- [ ] **Step 2: Run the routing test and witness RED**

Run:

```powershell
python tests/ci/test_product_contracts.py ProductContractTests.test_targeted_tool_contracts_run_windows_external_publication_smoke
```

Expected: FAIL because the Windows external publication check is absent from the dispatcher.

- [ ] **Step 3: Add the Windows contract implementation**

Add `check_v8_runner_windows_external_publication_contract` after the bounded EPF check with this exact boundary and early validation:

```python
def check_v8_runner_windows_external_publication_contract(
    runner: Path,
    target: str,
) -> list[str]:
    label = "v8-runner Windows external publication contract"
    if target != "win-x64":
        return []
    if not runner.is_file():
        return [f"{label}: binary not found: {runner}"]
```

After the guards, create one `TemporaryDirectory(prefix="unica-v8-runner-310-")`, then create `src/external-processors`, `work`, `ib`, and `platform/bin`. The descriptor is exactly:

```xml
<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>
```

The stub parses its own arguments and performs these real side effects:

```rust
if argument.eq_ignore_ascii_case("/LoadExternalDataProcessorOrReportFromFiles") {
    fs::write(&arguments[index + 2], b"issue-310-current")?;
}
if argument.eq_ignore_ascii_case("/DumpExternalDataProcessorOrReportToFiles") {
    fs::write(
        &arguments[index + 1],
        b"<ExternalDataProcessor><Properties><Name>Alpha</Name></Properties></ExternalDataProcessor>",
    )?;
}
if argument.eq_ignore_ascii_case("/Out") {
    fs::write(&arguments[index + 1], b"issue-310-platform-ok\n")?;
}
```

Compile the same executable to `platform/bin/1cv8c.exe` and copy it to `platform/bin/1cv8.exe`. The config contains:

```yaml
workPath: '<absolute work path>'
execution_timeout: 30000
format: DESIGNER
builder: DESIGNER
infobase:
  connection: 'File=<absolute ib path>'
source-set:
  - name: external-processors
    type: EXTERNAL_DATA_PROCESSORS
    path: '<absolute source path>'
tools:
  platform:
    path: '<absolute platform root>'
```

Invoke:

```python
command = [
    str(runner),
    "--config", str(config),
    "--json-message",
    "make",
    "--source-set", "external-processors",
    "--output", "Deploy",
]
```

Run from the temporary fixture root with a 60-second process timeout. Parse stdout as JSON. Validate the first result, then write `Deploy/stale.epf` and overwrite `Deploy/Alpha.epf` with `b"issue-310-stale"`; run the same command again and validate that only `Alpha.epf` remains with `b"issue-310-current"`. The process branch is exact:

```python
def run_make() -> tuple[object | None, list[str]]:
    try:
        result = subprocess.run(
            command,
            cwd=root,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=60,
            check=False,
        )
    except subprocess.TimeoutExpired:
        return None, [f"{label}: runner did not exit within 60 seconds"]
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip()
        return None, [
            f"{label}: runner OS process exited with {result.returncode}: {detail}"
        ]
    try:
        return json.loads(result.stdout), []
    except json.JSONDecodeError as error:
        return None, [f"{label}: runner returned invalid JSON: {error}"]
```

Prefix compile and result errors with `label`. After the second validation, require `sorted(path.name for path in output.iterdir()) == ["Alpha.epf"]` so replacement cannot leave `stale.epf` behind.

Finally append the check in `check_tool_contracts` after the two existing v8-runner behavioral checks.

- [ ] **Step 4: Run the routing test and witness GREEN**

Run the command from Step 2.

Expected: PASS.

- [ ] **Step 5: Run all product-contract unit tests**

Run:

```powershell
python tests/ci/test_product_contracts.py
```

Expected: all tests related to the changed helpers pass. If the known Windows-only `/missing/v8-runner` path spelling assertion fails, confirm it also fails on unmodified `origin/main` and record it separately; do not weaken it in this PR.

- [ ] **Step 6: Commit the executable Windows contract**

```powershell
git add tests/ci/test_product_contracts.py scripts/ci/check-tool-contracts.py
git commit -m "test: run Windows external publication contract"
```

---

### Task 3: Prove RED on 0.11.0 and GREEN on the current lock

**Files:**
- Verify only: `scripts/ci/check-tool-contracts.py`
- Verify only: `plugins/unica/third-party/tools.lock.json`

**Interfaces:**
- Consumes: installed Unica 0.11.0 runner from source commit `72d346c0a8fcf8373d9388257d11e6bef0ad70b2` and immutable current asset `v8-runner-nightly-master-build.2` from source commit `7ce1b062843d86644fe55741dbe0ee79f7ca767d`.
- Produces: reproducible PR evidence containing both binary provenance and the opposite contract outcomes.

- [ ] **Step 1: Verify old binary provenance**

Read `C:\Users\IApresov\.codex\unica\runtimes\0.11.0\win-x64\third-party\manifest.json` and require:

```text
name = v8-runner
version = 0.5.1
sourceCommit = 72d346c0a8fcf8373d9388257d11e6bef0ad70b2
```

- [ ] **Step 2: Run the contract against the old binary and witness issue #310**

Import `scripts/ci/check-tool-contracts.py` with `importlib.util` and invoke only:

```python
check_v8_runner_windows_external_publication_contract(
    Path(r"C:\Users\IApresov\.codex\unica\runtimes\0.11.0\win-x64\bin\win-x64\v8-runner.exe"),
    "win-x64",
)
```

Expected: a non-empty error list whose process diagnostic contains exit code 3 and the directory publication/fsync failure (`os error 3` or `os error 5`). Confirm the stub wrote its platform marker before publication failed.

- [ ] **Step 3: Download and verify the current immutable Windows asset**

Download `v8-runner-win-x64.exe` and its checksum from GitHub release `IngvarConsulting/unica-toolchain@v8-runner-nightly-master-build.2` into an ignored temporary directory under `.build/issue-310-current-runner`. Verify SHA-256 equals the lock value:

```text
191a3d7c930007377238dda0543d1e42cc1a1bd4b209736d54fd41c0ffaac32e
```

Resolve and validate the absolute temporary path before any cleanup.

- [ ] **Step 4: Run the same contract against the current asset**

Invoke the same helper with the downloaded binary and `win-x64`.

Expected: `[]`; both new-target and replacement publication pass, no stage/backup/metadata residue remains.

- [ ] **Step 5: Retain only the verified ignored download through final verification**

Keep `.build/issue-310-current-runner` only until Task 4 repeats the GREEN contract. It is ignored and must not be staged. Do not copy the binary elsewhere or modify any runtime installation.

---

### Task 4: Final verification and pull request

**Files:**
- Verify: `docs/design/2026-08-12-windows-external-artifact-publication-contract-design.md`
- Verify: `docs/plans/2026-08-12-windows-external-artifact-publication-contract.md`
- Verify: `scripts/ci/check-tool-contracts.py`
- Verify: `tests/ci/test_product_contracts.py`

**Interfaces:**
- Consumes: completed commits and RED/GREEN evidence.
- Produces: one independently reviewable PR to `IngvarConsulting/unica:main` linked to #310 and #264.

- [ ] **Step 1: Run formatting and static checks**

```powershell
python -m py_compile scripts/ci/check-tool-contracts.py tests/ci/test_product_contracts.py
git diff --check origin/main...HEAD
python scripts/ci/check-rust-platform-boundary.py
python scripts/ci/check-architecture-sync.py --base origin/main
```

- [ ] **Step 2: Run focused and repository-level tests**

```powershell
python tests/ci/test_product_contracts.py
python tests/ci/test_design_documents.py
python tests/ci/test_architecture_registry.py
```

For any baseline failure, run the exact test on `origin/main` in a clean detached worktree and preserve the comparison in the PR evidence.

- [ ] **Step 3: Run the current packaged Windows contract directly**

Run `check_v8_runner_windows_external_publication_contract` against the SHA-verified current asset one final time.

Expected: `[]`. Then resolve `.build/issue-310-current-runner`, require that it is a child of this worktree's `.build`, and remove only that verified directory. Do not delete any runtime installation or user worktree.

- [ ] **Step 4: Review scope and commits**

```powershell
git status --short
git diff --check origin/main...HEAD
git diff --stat origin/main...HEAD
git log --oneline origin/main..HEAD
```

Expected tracked scope: one design, one plan, `check-tool-contracts.py`, and `test_product_contracts.py`. No binaries, generated fixture, logs, credentials, runtime caches, or live-IB files are tracked.

- [ ] **Step 5: Commit any final test-only adjustments**

```powershell
git add scripts/ci/check-tool-contracts.py tests/ci/test_product_contracts.py docs/design/2026-08-12-windows-external-artifact-publication-contract-design.md docs/plans/2026-08-12-windows-external-artifact-publication-contract.md
git commit -m "test: protect Windows artifact publication"
```

Skip this commit when the index is empty; do not create an empty commit.

- [ ] **Step 6: Push and open the pull request**

Push `codex/issue-310-windows-publication-contract` and open a non-draft PR against `main` with:

```markdown
Closes #310.
Related: #264.

The production fix already arrived through upstream v8-runner-rust#48 and the
locked build.2 refresh. This PR adds the missing Windows consumer regression.

RED: Unica 0.11.0 runner (`72d346c0a8fcf8373d9388257d11e6bef0ad70b2`) exits 3 after successful stub build
when directory publication reaches fsync.

GREEN: current locked runner (`7ce1b062843d86644fe55741dbe0ee79f7ca767d`, SHA-256 `191a3d7c930007377238dda0543d1e42cc1a1bd4b209736d54fd41c0ffaac32e`) publishes a
new output, replaces an existing output, and leaves no stage/backup metadata.
```

- [ ] **Step 7: Verify remote PR state**

Require that the PR head SHA equals local `HEAD`, base is `main`, the PR is open and non-draft, and initial GitHub checks are present. Report the PR URL and any pending checks without claiming CI success before completion.
