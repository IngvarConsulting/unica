# Windows Git Bash Local Installer Design

**Issue:** [#168](https://github.com/IngvarConsulting/unica/issues/168)

**Goal:** Let contributors on native Windows x64 run the existing local Unica
development installer from Git Bash, while keeping WSL on the Linux target and
making the supported shell boundary explicit and regression-tested.

## Context

Windows is already part of the package contract: `tools.lock.json`, package
scripts, runtime assets, and executable naming all support `win-x64`. The local
installer also appends `.exe` after a target has been selected. The missing
piece is the installer's host mapping: `uname -s` from Git for Windows reports a
`MINGW*_NT-*` value, but `detect_target()` accepts only Darwin and Linux.

This creates a contract contradiction. User-facing documentation describes Git
for Windows and a Windows runtime, while the documented local-development entry
point exits with status 78 before it reaches the already-supported Windows
build and packaging path.

## Scope

The supported native Windows development shell is Git Bash from 64-bit Git for
Windows. The mapping accepts `MINGW*_NT-*` with `x86_64` or `amd64` and returns
`win-x64`.

The change does not claim support for MSYS2 or Cygwin. Their `MSYS_NT-*` and
`CYGWIN_NT-*` identities remain unsupported until their complete path,
subprocess, and toolchain behavior is validated separately. WSL continues to
report `Linux` and therefore maps to `linux-x64`.

No package metadata, public MCP tool, skill, runtime target, or release asset
contract changes.

## Installer Structure

Split host mapping from host discovery inside
`scripts/dev/install-local-unica.sh`:

- `target_for_host <system> <machine>` is a pure shell function that owns the
  case mapping and status-78 unsupported diagnostic;
- `detect_target` reads `uname -s` and `uname -m`, then delegates to
  `target_for_host`;
- the executable flow moves into `main`, called only when the script is run
  directly.

The direct-execution guard makes the pure mapping sourceable by regression
tests without starting a build, packaging a plugin, or modifying Codex state.
Normal command-line behavior and existing options remain unchanged.

The mapping contract is:

| `uname -s` | `uname -m` | Target |
| --- | --- | --- |
| `Darwin` | `arm64` | `darwin-arm64` |
| `Linux` | `x86_64` or `amd64` | `linux-x64` |
| `MINGW*_NT-*` | `x86_64` or `amd64` | `win-x64` |
| anything else | any value | status 78 with the existing diagnostic |

## Automated Verification

Add a focused Python `unittest` module under `tests/ci/`. Each case starts Bash,
sources the installer, and calls the pure mapping function. It verifies Darwin
arm64, Linux x64, representative `MINGW64_NT-*` Windows x64, WSL's Linux
identity, and an unsupported host. A separate negative assertion keeps
`MSYS_NT-*` and `CYGWIN_NT-*` outside the declared contract.

Extend the existing `build-tools` Windows matrix path rather than introducing a
second expensive Windows build. That job already builds the real `win-x64`
bundle and launches the bundled `unica.exe` through the MCP smoke. A
Windows-only installer step will:

1. pass that real bundle directly through the internal
   `UNICA_LOCAL_TOOL_BUNDLE` test seam used with `--skip-build`;
2. use an isolated temporary `CODEX_HOME`;
3. provide a deterministic fake only for the external `codex plugin` and
   `codex debug prompt-input` boundary;
4. run `install-local-unica.sh --skip-build` from Git Bash without skipping
   install or verification.

The installer itself still packages the plugin, runs the real bundled
`v8-runner.exe` and `unica.exe` help probes, installs the package into the
isolated cache, enables it in the isolated config, and validates the generated
prompt proof. The fake Codex boundary avoids coupling Unica CI to the release
timing or authentication state of an external CLI package; it must not replace
the real tool executables under test.

Together, the pre-existing Windows build step and the new installer smoke cover
build, package, executable launch, install, and verify in one Windows job
without compiling the Rust workspace twice or reintroducing the removed
`unica-tools-*` workflow artifact convention.

## Contributor Documentation

The root development section and `plugins/unica/README.md` will state that
native Windows development uses Git Bash from 64-bit Git for Windows. They will
list the prerequisites needed by the current installer path:

- Git for Windows with Git Bash;
- Python 3.10 or newer;
- stable Rust with the native MSVC toolchain;
- Microsoft C++ Build Tools and Windows SDK required by that toolchain;
- a current Codex CLI for real local installation and prompt verification.

The documented command remains `scripts/dev/install-local-unica.sh`. WSL users
are told to follow the Linux path. MSYS2 and Cygwin are explicitly not included
in the supported shell list.

## Error Handling And Compatibility

Unsupported combinations retain exit status 78 and the existing diagnostic
shape. A MINGW host with a non-x64 machine value is rejected rather than
silently selecting an incompatible bundle. Existing Darwin and Linux mappings
must remain byte-for-byte equivalent at the output boundary.

The installer continues to fail at its existing prerequisites and package
checks. This change does not add fallback downloads, cross-compilation, shell
translation layers, or a parallel PowerShell installer.

## Acceptance Evidence

The pull request is ready for review only after:

- focused host-mapping tests pass;
- the full Python source test suite passes;
- the Rust workspace tests remain green;
- the Windows `build-tools` job builds and launches the real `.exe` bundle;
- the Windows installer smoke completes package/install/verify using the
  isolated Codex boundary;
- documentation names the supported shell and prerequisites;
- the final diff contains no MSYS2/Cygwin support claim and no change to the
  public `unica.*` MCP boundary.
