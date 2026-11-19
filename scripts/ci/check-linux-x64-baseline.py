#!/usr/bin/env python3
"""Refuse linux-x64 binaries whose ordinary code needs more than baseline x86-64.

Rust builds `x86_64-unknown-linux-gnu` for baseline x86-64, but the C parts of
dependencies take the instruction set from the C compiler's default. GCC 15 of
the ubuntu-26.04 image compiled ring's curve25519 with AVX2, and the published
bootstrap died with SIGILL on a CPU with AVX but without AVX2 (#3190).

The check disassembles each given x86-64 ELF file and looks for instructions
beyond baseline x86-64, which ends at SSE2: SSE3 to SSE4.2, POPCNT, AES-NI,
PCLMULQDQ, SHA, VEX/EVEX vector code (AVX, AVX2, AVX-512, FMA) and the BMI,
BMI2, ADX and MOVBE scalar extensions. Such code is allowed only in functions
that a CPU check selects at run time. Their names carry the extension
(`..._avx2`, `SlimAVX2`, `x86_vaes256`, `..._adx`); in Rust it is the module
that holds the `#[target_feature]` functions (`httparse::simd::sse42`). The
hand-written assembly of ring whose names do not is listed explicitly.

Function names come from the symbol table, so the check covers the binaries
Unica builds itself and refuses a file without one rather than passing it.
"""

from __future__ import annotations

import argparse
import re
import struct
import subprocess
import sys
from pathlib import Path

ELF_MAGIC = b"\x7fELF"
ELFCLASS64 = 2
EM_X86_64 = 62
SHT_SYMTAB = 2

FUNCTION = re.compile(r"^[0-9a-f]+ <(?P<name>[^>]+)>:$")
INSTRUCTION = re.compile(r"^\s+[0-9a-f]+:\s+(?P<mnemonic>[a-z][a-z0-9.]*)\s*(?P<operands>.*)$")
VECTOR_REGISTER = re.compile(r"%[xyz]mm\d+")
WIDE_REGISTER = re.compile(r"%[yz]mm\d+|%k[0-7]\b")
BEYOND_BASELINE = {
    # SSE3
    "addsubpd", "addsubps", "haddpd", "haddps", "hsubpd", "hsubps",
    "lddqu", "movddup", "movshdup", "movsldup",
    # SSSE3
    "pabsb", "pabsd", "pabsw", "palignr", "phaddd", "phaddsw", "phaddw",
    "phsubd", "phsubsw", "phsubw", "pmaddubsw", "pmulhrsw", "pshufb",
    "psignb", "psignd", "psignw",
    # SSE4.1
    "blendpd", "blendps", "blendvpd", "blendvps", "dppd", "dpps", "extractps",
    "insertps", "movntdqa", "mpsadbw", "packusdw", "pblendvb", "pblendw",
    "pcmpeqq", "pextrb", "pextrd", "pextrq", "phminposuw", "pinsrb", "pinsrd",
    "pinsrq", "pmaxsb", "pmaxsd", "pmaxud", "pmaxuw", "pminsb", "pminsd",
    "pminud", "pminuw", "pmovsxbd", "pmovsxbq", "pmovsxbw", "pmovsxdq",
    "pmovsxwd", "pmovsxwq", "pmovzxbd", "pmovzxbq", "pmovzxbw", "pmovzxdq",
    "pmovzxwd", "pmovzxwq", "pmuldq", "pmulld", "ptest", "roundpd", "roundps",
    "roundsd", "roundss",
    # SSE4.2, POPCNT
    "pcmpestri", "pcmpestrm", "pcmpistri", "pcmpistrm", "pcmpgtq", "crc32",
    "crc32b", "crc32w", "crc32l", "crc32q", "popcnt",
    # AES-NI, PCLMULQDQ, SHA
    "aesdec", "aesdeclast", "aesenc", "aesenclast", "aesimc", "aeskeygenassist",
    "pclmulqdq", "pclmullqlqdq", "pclmulhqlqdq", "pclmullqhqdq", "pclmulhqhqdq",
    "sha1msg1", "sha1msg2", "sha1nexte", "sha1rnds4",
    "sha256msg1", "sha256msg2", "sha256rnds2",
    # BMI1, BMI2, ADX, MOVBE, LZCNT (TZCNT is `rep bsf` and runs everywhere)
    "andn", "bextr", "blsi", "blsmsk", "blsr", "bzhi",
    "mulx", "pdep", "pext", "rorx", "sarx", "shlx", "shrx",
    "adcx", "adox", "movbe", "lzcnt",
}

# Функции, которые выбирает проверка CPU во время выполнения: имя называет
# расширение отдельным словом, а не случайными буквами хеша или слова.
DISPATCHED_NAME = re.compile(
    r"(?:^|[_\d])(?:avx(?:2|512)?|sse4[12])(?:[_\d.]|$)"  # ring *_avx2, memchr 4avx2, crc32fast calculate_avx512, httparse 5sse42
    r"|(?:Slim|Fat)(?:AVX2|SSSE3)"  # aho-corasick teddy
    r"|\dx86_v?aes(?:256|512)?\w"  # aes backends x86_aes, x86_vaes256, x86_vaes512
    r"|\dpclmulqdq\d"  # crc32fast specialized::pclmulqdq
    r"|4sha2\d+sha\d+3x86"  # sha2 sha256::x86 (SHA-NI)
    r"|_adx(?:[_.]|$)"  # ring x25519 *_adx
)
# Ассемблер ring, который ring вызывает только после проверки CPU, а имя
# расширения не называет: AES-NI, SSSE3, CLMUL, SHA и AVX-вариант AES-GCM,
# вместе с его внутренними метками.
#
# MULX/ADX-варианты Монтгомери (суффикс `x` по соглашению OpenSSL) ring 0.17
# выбирает так: `bn_mulx4x_mont`, `bn_mulx4x_mont_gather5` и `bn_powerx5`
# вызывает Rust только при `Some((Adx, Bmi2..))` (`limbs/x86_64/mont.rs`);
# `bn_sqrx8x_internal` зовёт `bn_sqr8x_mont` по флагу `mulx_adx_capable`
# из той же проверки; `mulx4x_internal`, `__bn_postx4x_internal` и
# `__bn_sqrx8x_reduction` — их внутренние метки. `__ecp_nistz256_*_montx`
# вызываются только из `ecp_nistz256_*_adx`, а их C-код P-256 выбирает
# по `adx_bmi2_available`, который выставляет та же проверка CPUID.
DISPATCHED_ASSEMBLY = re.compile(
    r"^ring_core_\d+_\d+_\d+__(?:"
    r"aes_hw_\w+|aesni_gcm_(?:en|de)crypt|vpaes_\w+"
    r"|gcm_(?:init|gmult|ghash)_clmul|ChaCha20_ctr32_ssse3\w*"
    r"|sha(?:256|512)_block_data_order_(?:hw|ssse3)"
    r"|bn_(?:mulx4x_mont(?:_gather5)?|powerx5|sqrx8x_internal)"
    r")$"
    r"|^_aesni_(?:ctr32_(?:ghash_)?6x|encrypt\d)$|^_vpaes_\w+$|^process_extra_in_trailer$"
    r"|^(?:mulx4x_internal|__bn_postx4x_internal|__bn_sqrx8x_reduction"
    r"|__ecp_nistz256_(?:mul|sqr)_montx)$"
)


def is_x86_64_elf(header: bytes) -> bool:
    return (
        len(header) >= 20
        and header[:4] == ELF_MAGIC
        and header[4] == ELFCLASS64
        and int.from_bytes(header[18:20], "little") == EM_X86_64
    )


def has_symbol_table(data: bytes) -> bool:
    shoff = struct.unpack_from("<Q", data, 0x28)[0]
    shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)
    if shoff == 0 or shentsize < 8 or shoff + shentsize * shnum > len(data):
        return False
    return any(
        struct.unpack_from("<I", data, shoff + index * shentsize + 4)[0] == SHT_SYMTAB
        for index in range(shnum)
    )


def require_checkable(path: Path) -> None:
    """Refuse a given path that is not an x86-64 ELF file with a symbol table."""
    if not path.is_file():
        raise SystemExit(f"{path}: not a file")
    data = path.read_bytes()
    if not is_x86_64_elf(data[:20]):
        raise SystemExit(f"{path}: not an x86-64 ELF file")
    if not has_symbol_table(data):
        raise SystemExit(f"{path}: no symbol table, the check needs function names")


def beyond_baseline(mnemonic: str, operands: str) -> bool:
    if mnemonic in BEYOND_BASELINE or WIDE_REGISTER.search(operands):
        return True
    # VEX-форма SSE: тот же регистр xmm, но инструкция AVX.
    return mnemonic.startswith("v") and (
        bool(VECTOR_REGISTER.search(operands)) or mnemonic in {"vzeroupper", "vzeroall"}
    )


def dispatched(function: str) -> bool:
    return bool(DISPATCHED_NAME.search(function) or DISPATCHED_ASSEMBLY.match(function))


def violations(disassembly) -> dict[str, str]:
    """Map each non-dispatched function to its first instruction beyond baseline."""
    found: dict[str, str] = {}
    function = None
    allowed = True
    for line in disassembly:
        header = FUNCTION.match(line.rstrip("\n"))
        if header:
            function = header["name"]
            allowed = dispatched(function)
            continue
        if allowed or function is None or function in found:
            continue
        instruction = INSTRUCTION.match(line.rstrip("\n"))
        if instruction and beyond_baseline(instruction["mnemonic"], instruction["operands"]):
            found[function] = f"{instruction['mnemonic']} {instruction['operands']}".strip()
    return found


def check_file(path: Path, objdump: str) -> dict[str, str]:
    require_checkable(path)
    command = [objdump, "-d", "--no-show-raw-insn", str(path)]
    with subprocess.Popen(command, stdout=subprocess.PIPE, text=True, errors="replace") as process:
        assert process.stdout is not None
        found = violations(process.stdout)
    if process.returncode != 0:
        raise SystemExit(f"{objdump} failed on {path} with code {process.returncode}")
    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("binaries", nargs="+", type=Path, help="бинари linux-x64 с таблицей символов")
    parser.add_argument("--objdump", default="objdump")
    parser.add_argument("--show", type=int, default=20, help="сколько функций показать на файл")
    args = parser.parse_args(argv)

    for path in args.binaries:
        require_checkable(path)
    failed = False
    for path in args.binaries:
        found = check_file(path, args.objdump)
        if not found:
            print(f"ok: {path}")
            continue
        failed = True
        print(f"{path}: {len(found)} functions need more than baseline x86-64 without a CPU check")
        for function, instruction in sorted(found.items())[: args.show]:
            print(f"  {function}: {instruction}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
