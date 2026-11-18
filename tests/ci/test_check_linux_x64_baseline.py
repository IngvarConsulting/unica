from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "ci" / "check-linux-x64-baseline.py"


def load_module():
    spec = importlib.util.spec_from_file_location("check_linux_x64_baseline", SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load {SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def disassembly(*functions: tuple[str, list[str]]) -> list[str]:
    lines = ["", "unica:\tfile format elf64-x86-64", "", "Disassembly of section .text:", ""]
    address = 0x1000
    for name, instructions in functions:
        lines.append(f"{address:016x} <{name}>:")
        for instruction in instructions:
            lines.append(f"    {address:x}:      \t{instruction}")
            address += 4
        lines.append("")
    return lines


class LinuxX64BaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_the_avx2_broadcast_of_3190_in_ring_c_code_is_refused(self) -> None:
        found = self.module.violations(disassembly(
            ("ring_core_0_17_14__x25519_ge_scalarmult_base", [
                "pushq\t%rbp",
                "vmovd\t%eax, %xmm2",
                "vpbroadcastd\t%xmm2, %ymm2",
            ]),
        ))

        self.assertEqual(
            found, {"ring_core_0_17_14__x25519_ge_scalarmult_base": ["vmovd %eax, %xmm2"]}
        )

    def test_baseline_sse_and_tzcnt_pass(self) -> None:
        found = self.module.violations(disassembly(
            ("_RNvCs1_4core3str7compare", [
                "movdqu\t(%rdi), %xmm0",
                "pcmpeqb\t%xmm1, %xmm0",
                "pmovmskb\t%xmm0, %eax",
                "tzcnt\t%eax, %eax",
            ]),
        ))

        self.assertEqual(found, {})

    def test_scalar_extensions_outside_a_cpu_check_are_refused(self) -> None:
        found = self.module.violations(disassembly(
            ("table_select", ["shlx\t%rax, %rbx, %rcx"]),
            ("slide", ["movbe\t(%rdi), %eax"]),
            ("fe4_cswap", ["andn\t%rax, %rbx, %rcx"]),
        ))

        self.assertEqual(set(found), {"table_select", "slide", "fe4_cswap"})

    def test_code_behind_a_cpu_check_is_allowed(self) -> None:
        wide = ["vpbroadcastd\t%xmm2, %ymm2", "mulx\t%rdx, %rax, %rbx", "vzeroupper"]
        found = self.module.violations(disassembly(
            ("ring_core_0_17_14__ChaCha20_ctr32_avx2", wide),
            ("ring_core_0_17_14__x25519_scalar_mult_adx", wide),
            ("_RNvNvNtNtNtCs4_6memchr4arch6x86_646memchr9count_raw9find_avx2", wide),
            ("_RNvMs_NtNtNtNtCs8_12aho_corasick6packed5teddy7builder6x86_64INtB5_8SlimAVX2Kj4_E4find", wide),
            ("_RINvMNtNtCs9_3aes8backends11x86_vaes256INtB3_3AesKjf_E7encrypt", wide),
            ("ring_core_0_17_14__aesni_gcm_encrypt", wide),
            ("_aesni_ctr32_ghash_6x", wide),
        ))

        self.assertEqual(found, {})

    def test_avx512_mask_registers_are_refused(self) -> None:
        found = self.module.violations(disassembly(("slide", ["kmovw\t%k1, %eax"])))

        self.assertEqual(set(found), {"slide"})

    def test_a_stripped_binary_is_refused_rather_than_passed(self) -> None:
        with self.assertRaises(self.module.StrippedBinary):
            self.module.violations(disassembly((".text", ["movl\t$1, %eax"])))

    def test_only_x86_64_elf_files_are_selected(self) -> None:
        def elf(machine: int) -> bytes:
            return b"\x7fELF" + bytes(14) + machine.to_bytes(2, "little")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "bin").mkdir()
            (root / "bin" / "unica").write_bytes(elf(62))
            (root / "bin" / "arm").write_bytes(elf(183))
            (root / "notes.txt").write_text("not a binary", encoding="utf-8")

            self.assertEqual(self.module.elf_files([root]), [root / "bin" / "unica"])


if __name__ == "__main__":
    unittest.main()
