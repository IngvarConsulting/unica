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

Use the installer itself as the Windows `build-tools` matrix builder instead of
building a bundle first and then calling it with `--skip-build`. The
Windows-only path will:

1. install an exact Codex CLI version from the official npm package;
2. convert the isolated `CODEX_HOME` to a native forward-slash Windows path;
3. run `install-local-unica.sh` from Git Bash without skip flags;
4. build, package, launch, install, and verify through the real Codex CLI;
5. stage the bundle produced by the installer under the existing
   `.build/tool-bundles/win-x64` downstream pipeline path.

The installer packages the plugin, runs the bundled `v8-runner.exe` and
`unica.exe` help probes, registers the local marketplace, installs the cache,
enables the plugin, and validates a real `codex debug prompt-input` result. The
same bundle then continues through the existing tool-contract, MCP, runtime,
and bootstrap checks, so the Rust workspace is compiled only once.

No external bundle override is added to the installer. Its build output remains
contained below the selected build root. Shell scripts are explicitly marked
`text eol=lf` so a Git for Windows checkout with `core.autocrlf=true` cannot
corrupt the executable shebang before host detection starts.

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
- the Windows installer completes build/package/install/verify using a pinned
  real Codex CLI and isolated `CODEX_HOME`;
- a checkout with `core.autocrlf=true` retains an LF shell entrypoint;
- documentation names the supported shell and prerequisites;
- the final diff contains no MSYS2/Cygwin support claim and no change to the
  public `unica.*` MCP boundary.
