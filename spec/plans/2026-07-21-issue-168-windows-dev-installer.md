# Windows Git Bash Local Installer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `scripts/dev/install-local-unica.sh` support native Windows x64 contributors running Git Bash, with regression tests, explicit prerequisites, and a real Windows executable smoke in CI.

**Architecture:** Extract a pure `target_for_host(system, machine)` shell boundary and make the installer safe to source for unit tests while preserving its direct CLI flow. Reuse the existing Windows `build-tools` matrix output to exercise package/install/verify without a duplicate Rust build; only the external Codex CLI boundary is faked, while the packaged `.exe` tools remain real.

**Tech Stack:** Bash, Python 3.12 `unittest`, GitHub Actions YAML, Rust/MSVC Windows artifacts, Markdown.

## Global Constraints

- Support only Git Bash from 64-bit Git for Windows for native Windows development.
- Map `MINGW*_NT-*` with `x86_64` or `amd64` to `win-x64`.
- Keep `MSYS_NT-*` and `CYGWIN_NT-*` unsupported.
- Keep WSL on `linux-x64` through its existing `Linux` identity.
- Preserve unsupported-target exit status 78 and diagnostic wording.
- Do not change package metadata, release targets, public MCP tools, or prompt-visible skills.
- Do not add a PowerShell installer, cross-compilation, or fallback downloads.
- Use the real Windows tool bundle in CI; fake only the external Codex CLI command boundary.

---

### Task 1: Make Host Mapping Testable And Add MINGW x64

**Files:**
- Create: `tests/ci/test_local_dev_installer.py`
- Modify: `scripts/dev/install-local-unica.sh`

**Interfaces:**
- Consumes: `uname -s` and `uname -m` strings.
- Produces: `target_for_host <system> <machine>` printing one package target or returning status 78; `detect_target` remains the current-host entry point.

- [ ] **Step 1: Write the failing host-mapping tests**

Create `tests/ci/test_local_dev_installer.py` with:

```python
from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
INSTALLER = REPO_ROOT / "scripts/dev/install-local-unica.sh"


class LocalDevInstallerTests(unittest.TestCase):
    def target_for_host(self, system: str, machine: str) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory() as tmp:
            return subprocess.run(
                [
                    "bash",
                    "-c",
                    (
                        'installer="$1"; system="$2"; machine="$3"; tmp="$4"; '
                        'set -- --build-dir "$tmp" --skip-build --skip-install '
                        '--skip-verify; source "$installer"; '
                        'target_for_host "$system" "$machine"'
                    ),
                    "bash",
                    str(INSTALLER),
                    system,
                    machine,
                    tmp,
                ],
                cwd=REPO_ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

    def test_supported_hosts_map_to_package_targets(self) -> None:
        cases = (
            ("Darwin", "arm64", "darwin-arm64"),
            ("Linux", "x86_64", "linux-x64"),
            ("Linux", "amd64", "linux-x64"),
            ("MINGW64_NT-10.0-19045", "x86_64", "win-x64"),
            ("MINGW64_NT-10.0-22631", "amd64", "win-x64"),
        )
        for system, machine, expected in cases:
            with self.subTest(system=system, machine=machine):
                completed = self.target_for_host(system, machine)
                self.assertEqual(completed.returncode, 0, completed.stderr)
                self.assertEqual(completed.stdout, expected + "\n")
                self.assertEqual(completed.stderr, "")

    def test_wsl_keeps_linux_semantics(self) -> None:
        completed = self.target_for_host("Linux", "x86_64")
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, "linux-x64\n")

    def test_unsupported_shells_and_hosts_keep_status_78(self) -> None:
        cases = (
            ("MSYS_NT-10.0-19045", "x86_64"),
            ("CYGWIN_NT-10.0-19045", "x86_64"),
            ("MINGW64_NT-10.0-19045", "aarch64"),
            ("FreeBSD", "x86_64"),
        )
        for system, machine in cases:
            with self.subTest(system=system, machine=machine):
                completed = self.target_for_host(system, machine)
                self.assertEqual(completed.returncode, 78)
                self.assertEqual(completed.stdout, "")
                self.assertEqual(
                    completed.stderr,
                    f"Unsupported local Unica tool target: {system}-{machine}\n",
                )


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
python3.12 -m unittest tests.ci.test_local_dev_installer -v
```

Expected: FAIL safely with status 66 because sourcing the current installer
reaches the missing temporary `--skip-build` bundle before `target_for_host` can
run. The RED path must not build tools, install a marketplace, or modify the
real Codex home.

- [ ] **Step 3: Add the pure mapping functions**

Immediately after `usage()` in `scripts/dev/install-local-unica.sh`, add:

```bash
target_for_host() {
  local host_os="$1"
  local host_arch="$2"
  case "${host_os}-${host_arch}" in
    Darwin-arm64) printf '%s\n' "darwin-arm64" ;;
    Linux-x86_64|Linux-amd64) printf '%s\n' "linux-x64" ;;
    MINGW*_NT-*-x86_64|MINGW*_NT-*-amd64) printf '%s\n' "win-x64" ;;
    *)
      echo "Unsupported local Unica tool target: ${host_os}-${host_arch}" >&2
      return 78
      ;;
  esac
}

detect_target() {
  target_for_host "$(uname -s)" "$(uname -m)"
}
```

Delete the old `detect_target()` definition. Insert `main() {` immediately
before the existing `REPO_ROOT=...` assignment. Leave every existing statement
from that assignment through the final output block in its current order, then
close `main` and add the direct-execution guard after the final existing `fi`:

```bash
main() {
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
```

The end of the file becomes:

```bash
echo "==> Local Unica marketplace ready: $MARKETPLACE_DIR"
if [ "$DO_INSTALL" -eq 1 ]; then
  echo "==> Installed in Codex as marketplace '$MARKETPLACE_NAME'"
  if [ "$DO_VERIFY" -eq 1 ]; then
    echo "==> Fresh prompt proof: $PROMPT_PROOF"
  fi
fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  main "$@"
fi
```

This is a wrapper-only move: do not rewrite the argument parser, Python
selection, package commands, Codex cache logic, or final output while placing
them inside `main`.

- [ ] **Step 4: Run shell syntax and focused tests to verify GREEN**

Run:

```bash
bash -n scripts/dev/install-local-unica.sh
python3.12 -m unittest tests.ci.test_local_dev_installer -v
```

Expected: shell syntax exits 0; all three test methods pass, including MINGW x64 and explicit MSYS2/Cygwin rejection.

- [ ] **Step 5: Verify unchanged Darwin direct execution reaches the existing bundle check**

Run:

```bash
tmp_dir="$(mktemp -d)"
set +e
scripts/dev/install-local-unica.sh \
  --build-dir "$tmp_dir" \
  --skip-build \
  --skip-install \
  --skip-verify >"$tmp_dir/stdout" 2>"$tmp_dir/stderr"
exit_code=$?
set -e
test "$exit_code" -eq 66
grep -F "==> Unica local target: darwin-arm64" "$tmp_dir/stdout"
grep -F -- "--skip-build requested, but bundle is missing:" "$tmp_dir/stderr"
```

Expected: every assertion exits 0, proving direct execution still selects the current macOS target and reaches the pre-existing missing-bundle failure.

- [ ] **Step 6: Commit the mapping and regression contract**

```bash
git add scripts/dev/install-local-unica.sh tests/ci/test_local_dev_installer.py
git commit --no-gpg-sign -m "fix: detect Windows Git Bash local target"
```

### Task 2: Document The Native Windows Development Contract

**Files:**
- Modify: `tests/ci/test_local_dev_installer.py`
- Modify: `README.md`
- Modify: `plugins/unica/README.md`

**Interfaces:**
- Consumes: the Git Bash-only support boundary from Task 1.
- Produces: contributor-facing prerequisites and explicit WSL/MSYS2/Cygwin guidance.

- [ ] **Step 1: Add a failing documentation contract test**

Add this method to `LocalDevInstallerTests`:

```python
    def test_windows_local_development_docs_name_shell_and_prerequisites(self) -> None:
        required = (
            "Git Bash",
            "Python 3.10",
            "MSVC",
            "Microsoft C++ Build Tools",
            "Windows SDK",
            "scripts/dev/install-local-unica.sh",
            "WSL",
            "MSYS2",
            "Cygwin",
        )
        for relative_path in ("README.md", "plugins/unica/README.md"):
            text = (REPO_ROOT / relative_path).read_text(encoding="utf-8")
            for value in required:
                with self.subTest(path=relative_path, value=value):
                    self.assertIn(value, text)
```

- [ ] **Step 2: Run the documentation contract and verify RED**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_local_dev_installer.LocalDevInstallerTests.test_windows_local_development_docs_name_shell_and_prerequisites \
  -v
```

Expected: FAIL because the current local-development sections do not contain the complete Windows prerequisite and shell boundary.

- [ ] **Step 3: Add the Russian root README guidance**

After the local installer command in `README.md`, add:

```markdown
На Windows x64 запускайте этот скрипт из **Git Bash**, входящего в 64-битный
Git for Windows. Для локальной сборки нужны Python 3.10 или новее, стабильный
Rust с нативным toolchain MSVC, а также Microsoft C++ Build Tools и Windows SDK.
Для установки и проверки видимости плагина нужен актуальный Codex CLI.

WSL сохраняет Linux-семантику и собирает `linux-x64`. MSYS2 и Cygwin не входят
в поддерживаемые shell для этого installer; используйте Git Bash.
```

- [ ] **Step 4: Add the English plugin README guidance**

After the local installer command in `plugins/unica/README.md`, add:

```markdown
On native Windows x64, run the script from **Git Bash** included with 64-bit Git
for Windows. The local build requires Python 3.10 or newer, stable Rust with the
native MSVC toolchain, Microsoft C++ Build Tools, and the Windows SDK. A current
Codex CLI is required for the install and fresh-prompt verification steps.

WSL keeps Linux semantics and builds `linux-x64`. MSYS2 and Cygwin are not
supported shells for this installer; use Git Bash.
```

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run:

```bash
python3.12 -m unittest tests.ci.test_local_dev_installer -v
```

Expected: all four test methods pass.

- [ ] **Step 6: Commit the contributor documentation**

```bash
git add README.md plugins/unica/README.md tests/ci/test_local_dev_installer.py
git commit --no-gpg-sign -m "docs: describe Windows Git Bash development"
```

### Task 3: Exercise The Installer In The Existing Windows Build Job

**Files:**
- Modify: `tests/ci/test_local_dev_installer.py`
- Modify: `.github/workflows/unica-plugin-release.yml`

**Interfaces:**
- Consumes: `.build/tool-bundles/win-x64` produced and already MCP-smoked by the `build-tools` matrix job.
- Produces: an isolated Windows Git Bash package/install/verify smoke using real `v8-runner.exe` and `unica.exe` binaries.

- [ ] **Step 1: Add a failing workflow contract test**

Add this method to `LocalDevInstallerTests`:

```python
    def test_windows_ci_runs_local_installer_package_install_verify_smoke(self) -> None:
        workflow = (
            REPO_ROOT / ".github/workflows/unica-plugin-release.yml"
        ).read_text(encoding="utf-8")
        required = (
            "Smoke local development installer on Windows",
            "if: matrix.target == 'win-x64'",
            'bundle_root="$build_root/tool-artifacts/unica-tools-win-x64"',
            'cp -R ".build/tool-bundles/win-x64" "$bundle_root"',
            'CODEX_HOME="$build_root/codex-home"',
            'PATH="$fake_bin:$PATH"',
            "scripts/dev/install-local-unica.sh",
            "--skip-build",
        )
        for value in required:
            with self.subTest(value=value):
                self.assertIn(value, workflow)
```

- [ ] **Step 2: Run the workflow contract and verify RED**

Run:

```bash
python3.12 -m unittest \
  tests.ci.test_local_dev_installer.LocalDevInstallerTests.test_windows_ci_runs_local_installer_package_install_verify_smoke \
  -v
```

Expected: FAIL because the Windows build job does not invoke the local installer.

- [ ] **Step 3: Add the Windows-only installer smoke step**

In `.github/workflows/unica-plugin-release.yml`, immediately after `Smoke packaged Unica MCP`, add:

```yaml
      - name: Smoke local development installer on Windows
        if: matrix.target == 'win-x64'
        shell: bash
        run: |
          set -euo pipefail
          build_root="$PWD/.build/local-installer-smoke"
          bundle_root="$build_root/tool-artifacts/unica-tools-win-x64"
          fake_bin="$build_root/fake-bin"
          CODEX_HOME="$build_root/codex-home"
          rm -rf "$build_root"
          mkdir -p "$(dirname "$bundle_root")" "$fake_bin"
          cp -R ".build/tool-bundles/win-x64" "$bundle_root"
          cat > "$fake_bin/codex" <<'EOF'
          #!/usr/bin/env bash
          set -euo pipefail
          case "$*" in
            plugin\ marketplace\ remove\ *) exit 0 ;;
            plugin\ marketplace\ add\ *) exit 0 ;;
            debug\ prompt-input\ test)
              printf '%s\n' '{"skills":["Unica","v8-runner","db-auth-check"]}'
              ;;
            *)
              printf 'unexpected fake codex invocation: %s\n' "$*" >&2
              exit 64
              ;;
          esac
          EOF
          chmod +x "$fake_bin/codex"
          CODEX_HOME="$CODEX_HOME" PATH="$fake_bin:$PATH" \
            scripts/dev/install-local-unica.sh \
              --build-dir "$build_root" \
              --skip-build
```

Keep the heredoc content aligned exactly as YAML block content so the shell receives `#!/usr/bin/env bash` at column 1 after YAML removes common indentation.

- [ ] **Step 4: Run source-level workflow checks and verify GREEN**

Run:

```bash
python3.12 -m unittest tests.ci.test_local_dev_installer -v
python3.12 -c 'import yaml; yaml.safe_load(open(".github/workflows/unica-plugin-release.yml", encoding="utf-8"))'
```

Expected: all five installer test methods pass and PyYAML parses the workflow.

- [ ] **Step 5: Commit the Windows integration smoke**

```bash
git add .github/workflows/unica-plugin-release.yml tests/ci/test_local_dev_installer.py
git commit --no-gpg-sign -m "ci: smoke local installer on Windows"
```

### Task 4: Verify The Complete Source Change

**Files:**
- No planned changes; failures return to the task that owns the affected file.

**Interfaces:**
- Consumes: the mapping, documentation, and CI contracts from Tasks 1-3.
- Produces: fresh evidence that issue #168 is isolated and all existing source contracts remain intact.

- [ ] **Step 1: Run formatting and static validation**

```bash
bash -n scripts/dev/install-local-unica.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
python3.12 -c 'import yaml; yaml.safe_load(open(".github/workflows/unica-plugin-release.yml", encoding="utf-8"))'
git diff --check origin/main...HEAD
```

Expected: every command exits 0 without warnings promoted to errors.

- [ ] **Step 2: Run all Python source tests with non-interactive Git commits**

```bash
GIT_CONFIG_COUNT=1 \
GIT_CONFIG_KEY_0=commit.gpgSign \
GIT_CONFIG_VALUE_0=false \
python3.12 -m unittest discover -s tests/ci --durations 20
```

Expected: all tests pass; platform-specific skips remain skips.

- [ ] **Step 3: Run the Rust workspace tests**

```bash
cargo test --workspace -- --test-threads=1
```

Expected: all non-ignored tests pass.

- [ ] **Step 4: Review scope and package-boundary invariants**

```bash
git status -sb
git diff --stat origin/main...HEAD
git diff origin/main...HEAD -- \
  plugins/unica/.mcp.json \
  plugins/unica/.codex-plugin/plugin.json \
  plugins/unica/third-party/tools.lock.json
git grep -n -E 'MSYS_NT|CYGWIN_NT' -- README.md plugins/unica/README.md scripts/dev/install-local-unica.sh
```

Expected: package-contract files have no diff; MSYS/Cygwin appear only as unsupported documentation/tests, never as accepted target patterns.

### Task 5: Publish The Draft Pull Request And Follow Windows CI

**Files:**
- No source files planned; PR metadata summarizes the committed diff and verification.

**Interfaces:**
- Consumes: a clean branch with all Task 4 checks passing.
- Produces: a draft PR targeting `IngvarConsulting/unica:main`, linked to issue #168, with Windows CI evidence.

- [ ] **Step 1: Confirm the publish scope**

```bash
git status -sb
git log --oneline origin/main..HEAD
git diff --stat origin/main...HEAD
```

Expected: the worktree is clean and every commit/file belongs to issue #168.

- [ ] **Step 2: Push the isolated branch**

```bash
git push -u origin codex/issue-168-windows-dev-installer
```

Expected: the branch is created on `origin` and tracks it.

- [ ] **Step 3: Open a draft PR**

Create a draft PR with title `fix(dev-installer): support Windows Git Bash x64`, base `main`, head `codex/issue-168-windows-dev-installer`, and a body containing:

```markdown
Closes #168.

## What changed

- maps Git for Windows `MINGW*_NT-*` x64 hosts to `win-x64`;
- keeps WSL on Linux and leaves MSYS2/Cygwin unsupported;
- documents the native Windows development prerequisites;
- exercises package/install/verify in the existing Windows build job with real `.exe` tools.

## Verification

- `python3.12 -m unittest discover -s tests/ci --durations 20`
- `cargo test --workspace -- --test-threads=1`
- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Windows `build-tools` and local installer smoke in GitHub Actions

The CI smoke isolates only the external Codex CLI boundary; the built and launched Unica tools are the real Windows executables.
```

Prefer the connected GitHub PR creation action after the push; use `gh pr create --draft` only if the connector cannot create the PR cleanly.

- [ ] **Step 4: Wait for required checks and inspect failures**

Run:

```bash
gh pr checks --watch --fail-fast=false
```

Expected: `Unica CI` and its Windows `build-tools (win-x64)` dependency pass. If a check fails, inspect the exact Actions log, fix the root cause in the owning task, rerun the complete local verification affected by that fix, commit, and push.

- [ ] **Step 5: Report the evidence and remaining platform limitation**

Report the PR URL, branch, commits, fresh local verification, and GitHub Actions result. State explicitly that GitHub Actions verified Git Bash/MINGW x64 with real Unica `.exe` tools while the Codex CLI command boundary was deterministic and isolated; do not claim a separate manual Windows desktop smoke unless one was actually performed.
