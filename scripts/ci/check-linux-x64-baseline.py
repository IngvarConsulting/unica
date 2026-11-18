#!/usr/bin/env python3
"""Refuse linux-x64 binaries whose ordinary code needs more than baseline x86-64.

Rust builds `x86_64-unknown-linux-gnu` for baseline x86-64, but the C parts of
dependencies take the instruction set from the C compiler's default. GCC 15 of
the ubuntu-26.04 image compiled ring's curve25519 with AVX2, and the published
bootstrap died with SIGILL on a CPU with AVX but without AVX2 (#3190).

The check disassembles every x86-64 ELF file under the given paths and looks
for instructions beyond baseline: VEX/EVEX vector code (AVX, AVX2, AVX-512,
FMA) and the BMI, BMI2, ADX and MOVBE scalar extensions. Such code is allowed
only in functions that a CPU check selects at run time. Their names carry the
extension (`..._avx2`, `SlimAVX2`, `x86_vaes256`, `..._adx`), and the
hand-written assembly of ring whose names do not is listed explicitly.

The check needs the symbol table, so it covers the binaries Unica builds
itself. Stripped files are refused rather than passed.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ELF_MAGIC = b"\x7fELF"
EM_X86_64 = 62

FUNCTION = re.compile(r"^[0-9a-f]+ <(?P<name>[^>]+)>:$")
INSTRUCTION = re.compile(r"^\s+[0-9a-f]+:\s+(?P<mnemonic>[a-z][a-z0-9.]*)\s*(?P<operands>.*)$")
VECTOR_REGISTER = re.compile(r"%[xyz]mm\d+")
WIDE_REGISTER = re.compile(r"%[yz]mm\d+|%k[0-7]")
SCALAR_EXTENSIONS = {
    # BMI1, BMI2
    "andn", "bextr", "blsi", "blsmsk", "blsr", "bzhi",
    "mulx", "pdep", "pext", "rorx", "sarx", "shlx", "shrx",
    # ADX, MOVBE, LZCNT (TZCNT is `rep bsf` and runs everywhere)
    "adcx", "adox", "movbe", "lzcnt",
}

# Функции, которые выбирает проверка CPU во время выполнения.
DISPATCHED_NAME = re.compile(
    r"avx|vaes|vpclmulqdq|_adx|adx_|bmi|_mulx|mulx4x|sqrx|shaext|sha_ni",
    re.IGNORECASE,
)
# Ассемблер AES-GCM ring на AVX и MOVBE: ring вызывает его только после
# проверки CPU, а имя расширения не называет.
DISPATCHED_ASSEMBLY = re.compile(
    r"^ring_core_\d+_\d+_\d+__aesni_gcm_(?:en|de)crypt$|^_aesni_ctr32_(?:ghash_)?6x$"
)


def is_x86_64_elf(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            header = handle.read(20)
    except OSError:
        return False
    return (
        len(header) == 20
        and header[:4] == ELF_MAGIC
        and int.from_bytes(header[18:20], "little") == EM_X86_64
    )


def elf_files(paths: list[Path]) -> list[Path]:
    found = []
    for path in paths:
        candidates = sorted(path.rglob("*")) if path.is_dir() else [path]
        found.extend(p for p in candidates if p.is_file() and not p.is_symlink() and is_x86_64_elf(p))
    return found


def beyond_baseline(mnemonic: str, operands: str) -> bool:
    if mnemonic in SCALAR_EXTENSIONS:
        return True
    if WIDE_REGISTER.search(operands):
        return True
    # VEX-форма SSE: тот же регистр xmm, но инструкция AVX.
    return mnemonic.startswith("v") and (
        bool(VECTOR_REGISTER.search(operands)) or mnemonic in {"vzeroupper", "vzeroall"}
    )


def dispatched(function: str) -> bool:
    return bool(DISPATCHED_NAME.search(function) or DISPATCHED_ASSEMBLY.match(function))


def violations(disassembly, limit_per_function: int = 1) -> dict[str, list[str]]:
    """Map each non-dispatched function to its first instructions beyond baseline."""
    found: dict[str, list[str]] = {}
    function = None
    allowed = True
    named = False
    for line in disassembly:
        line = line.rstrip("\n")
        header = FUNCTION.match(line)
        if header:
            function = header["name"]
            # Без таблицы символов objdump называет код именем секции, и
            # отличить функцию за проверкой CPU от обычной нечем.
            named = named or not function.startswith(".")
            allowed = dispatched(function)
            continue
        if allowed or function is None:
            continue
        instruction = INSTRUCTION.match(line)
        if not instruction:
            continue
        mnemonic, operands = instruction["mnemonic"], instruction["operands"]
        if beyond_baseline(mnemonic, operands):
            seen = found.setdefault(function, [])
            if len(seen) < limit_per_function:
                seen.append(f"{mnemonic} {operands}".strip())
    if function is not None and not named:
        raise StrippedBinary
    return found


class StrippedBinary(Exception):
    """The file has no function symbols, so dispatched code cannot be told apart."""


def check_file(path: Path, objdump: str) -> dict[str, list[str]]:
    command = [objdump, "-d", "--no-show-raw-insn", str(path)]
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True, errors="replace") as process:
        assert process.stdout is not None
        try:
            found = violations(process.stdout)
        except StrippedBinary:
            process.kill()
            raise SystemExit(f"{path}: no symbol table, the check needs function names")
    if process.returncode != 0:
        raise SystemExit(f"{objdump} failed on {path} with code {process.returncode}")
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="+", type=Path, help="файлы или каталоги поставки linux-x64")
    parser.add_argument("--objdump", default="objdump")
    parser.add_argument("--show", type=int, default=20, help="сколько функций показать на файл")
    args = parser.parse_args(argv)

    files = elf_files(args.paths)
    if not files:
        print("no x86-64 ELF files found", file=sys.stderr)
        return 2
    failed = False
    for path in files:
        found = check_file(path, args.objdump)
        if not found:
            print(f"ok: {path}")
            continue
        failed = True
        print(f"{path}: {len(found)} functions need more than baseline x86-64 without a CPU check")
        for function, instructions in sorted(found.items())[: args.show]:
            print(f"  {function}: {instructions[0]}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
