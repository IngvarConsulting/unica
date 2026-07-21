# Windows Git Bash Local Installer Implementation Plan

**Issue:** [#168](https://github.com/IngvarConsulting/unica/issues/168)

**Goal:** Support the complete Unica local-development path on native Windows
x64 from 64-bit Git for Windows Git Bash.

**Architecture:** Keep host mapping in a pure sourceable shell function. On the
Windows CI runner, use the installer itself as the only bundle builder, then
continue the existing release pipeline with that bundle. Exercise installation
and prompt visibility through an exact real Codex CLI version, not a stub.

## Constraints

- `MINGW*_NT-*` with `x86_64` or `amd64` maps to `win-x64`.
- WSL remains `linux-x64` through its `Linux` identity.
- MSYS2 and Cygwin remain unsupported development shells.
- Unsupported combinations preserve status 78 and the existing diagnostic.
- Package metadata, release targets, skills, and the public `unica.*` MCP
  boundary do not change.
- Windows CI compiles the Rust workspace once.
- Installer deletion targets remain below the selected build root.
- Shell entrypoints remain LF in Git for Windows checkouts.

## Task 1: Host Mapping And Python Discovery

**Files:**

- `scripts/dev/install-local-unica.sh`
- `tests/ci/test_local_dev_installer.py`

1. Add regression cases for Darwin arm64, Linux x64, Git Bash MINGW x64,
   unsupported shells, and non-x64 MINGW.
2. Extract `target_for_host` and keep `detect_target` as current-host discovery.
3. Guard direct execution so mapping tests can source the script safely.
4. Accept the standard Windows `python` executable after `python3.*` candidates.

## Task 2: Windows Checkout And Contributor Contract

**Files:**

- `.gitattributes`
- `README.md`
- `plugins/unica/README.md`
- `tests/ci/test_local_dev_installer.py`

1. Require `text eol=lf` for shell scripts and assert the attribute through
   `git check-attr`.
2. Document Git Bash from 64-bit Git for Windows.
3. Document Python 3.10+, stable Rust with native MSVC, Microsoft C++ Build
   Tools, Windows SDK, and Codex CLI.
4. State explicitly that WSL uses Linux and MSYS2/Cygwin are unsupported.

## Task 3: Safe Bundle Ownership

**Files:**

- `scripts/dev/install-local-unica.sh`
- `tests/ci/test_local_dev_installer.py`

1. Keep the tool bundle derived exclusively from `BUILD_ROOT`, `TOOLS_ROOT`,
   and the detected target.
2. Do not expose an environment override that can redirect the bundle into the
   existing `rm -rf` build cleanup.
3. Pass the exact derived bundle to local-debug packaging.

## Task 4: Complete Native Windows CI Path

**Files:**

- `.github/workflows/unica-plugin-release.yml`
- `tests/ci/test_local_dev_installer.py`

1. Install Node through `actions/setup-node@v7` on `win-x64` only.
2. Install and verify exact Codex CLI `0.145.0-alpha.18` from
   `@openai/codex`.
3. Convert isolated `CODEX_HOME` with `cygpath -m` so both Git Bash utilities
   and native Windows Codex use the same directory.
4. Run `scripts/dev/install-local-unica.sh` without skip flags from Git Bash.
5. Let the installer build, package, launch `v8-runner.exe` and `unica.exe`,
   register the marketplace, install the plugin, and validate real prompt input.
6. Stage the installer-produced bundle under `.build/tool-bundles/win-x64` and
   continue existing contract, MCP, runtime, and bootstrap checks.
7. Keep the direct Python bundle builder for non-Windows matrix targets only.

## Task 5: Verification And Publication

Run locally:

```bash
bash -n scripts/dev/install-local-unica.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3.12 -m py_compile scripts/ci/*.py tests/ci/*.py
actionlint .github/workflows/unica-plugin-release.yml
GIT_CONFIG_COUNT=1 \
GIT_CONFIG_KEY_0=commit.gpgSign \
GIT_CONFIG_VALUE_0=false \
python3.12 -m unittest discover -s tests/ci --durations 20
cargo test --workspace -- --test-threads=1
git diff --check origin/main...HEAD
```

After pushing the review fixes, wait for all PR checks. The acceptance evidence
must include the `Build tools (win-x64)` log showing:

- exact Codex CLI installation and version verification;
- Git Bash host selection of `win-x64`;
- installer build/package/install/verify without skip flags;
- real `.exe` probes and real prompt visibility;
- successful downstream runtime and bootstrap checks.

Reply to and resolve review threads only after the replacement Windows CI run
passes on the updated head commit.
