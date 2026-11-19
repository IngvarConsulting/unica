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


def elf(*, machine: int, section_types: list[int]) -> bytes:
    """A minimal ELF64 header followed by section headers of the given types."""
    header = bytearray(64)
    header[:5] = b"\x7fELF\x02"
    header[18:20] = machine.to_bytes(2, "little")
    header[0x28:0x30] = (64).to_bytes(8, "little")
    header[0x3A:0x3C] = (64).to_bytes(2, "little")
    header[0x3C:0x3E] = len(section_types).to_bytes(2, "little")
    sections = b"".join(bytes(4) + kind.to_bytes(4, "little") + bytes(56) for kind in section_types)
    return bytes(header) + sections


class LinuxX64BaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_the_avx2_broadcast_of_3190_in_ring_c_code_is_refused(self) -> None:
        found = self.module.violations(disassembly(
            ("ring_core_0_17_14__x25519_ge_scalarmult_base", [
                "pushq\t%rbp",
                "vpbroadcastd\t%xmm2, %ymm2",
            ]),
        ))

        self.assertEqual(
            found, {"ring_core_0_17_14__x25519_ge_scalarmult_base": "vpbroadcastd %xmm2, %ymm2"}
        )

    def test_baseline_sse2_and_tzcnt_pass(self) -> None:
        found = self.module.violations(disassembly(
            ("_RNvCs1_4core3str7compare", [
                "movdqu\t(%rdi), %xmm0",
                "pcmpeqb\t%xmm1, %xmm0",
                "pmovmskb\t%xmm0, %eax",
                "tzcnt\t%eax, %eax",
            ]),
        ))

        self.assertEqual(found, {})

    def test_extensions_beyond_sse2_outside_a_cpu_check_are_refused(self) -> None:
        found = self.module.violations(disassembly(
            ("table_select", ["shlx\t%rax, %rbx, %rcx"]),
            ("slide", ["movbe\t(%rdi), %eax"]),
            ("fe4_cswap", ["vpxor\t%xmm0, %xmm0, %xmm0"]),
            ("ssse3_shuffle", ["pshufb\t%xmm0, %xmm1"]),
            ("vector_test", ["ptest\t%xmm0, %xmm1"]),
            ("count_bits", ["popcnt\t%rax, %rax"]),
            ("mask", ["kmovw\t%k1, %eax"]),
        ))

        self.assertEqual(
            set(found),
            {"table_select", "slide", "fe4_cswap", "ssse3_shuffle", "vector_test", "count_bits", "mask"},
        )

    def test_names_that_merely_contain_an_extension_are_not_exempt(self) -> None:
        wide = ["vpbroadcastd\t%xmm2, %ymm2"]
        found = self.module.violations(disassembly(
            ("_RNvMs_NtCs1_21unica_daemon_protocol9client_v517submit_invocation", wide),
            ("_RNvCsAVXq1_5crate8handlers", wide),
            ("travxa_bmi2_davx", wide),
        ))

        self.assertEqual(len(found), 3)

    def test_code_behind_a_cpu_check_is_allowed(self) -> None:
        wide = ["vpbroadcastd\t%xmm2, %ymm2", "mulx\t%rdx, %rax, %rbx", "pshufb\t%xmm0, %xmm1"]
        found = self.module.violations(disassembly(
            ("ring_core_0_17_14__ChaCha20_ctr32_avx2", wide),
            ("ring_core_0_17_14__x25519_scalar_mult_adx", wide),
            ("ring_core_0_17_14__chacha20_poly1305_open_sse41", wide),
            ("ring_core_0_17_14__aesni_gcm_encrypt", wide),
            ("ring_core_0_17_14__vpaes_ctr32_encrypt_blocks", wide),
            ("_aesni_ctr32_ghash_6x", wide),
            ("_RNvNvNtNtNtCs4_6memchr4arch6x86_646memchr9count_raw9find_avx2", wide),
            ("_RNvMs_NtNtNtNtCs8_12aho_corasick6packed5teddy7builder6x86_64INtB5_8SlimAVX2Kj4_E4find", wide),
            ("_RNvXs1_NtNtNtNtCs8_12aho_corasick6packed5teddy7builder6x86_64INtB5_9SlimSSSE3Kj1_E4find", wide),
            ("_RINvMNtNtCs9_3aes8backends11x86_vaes256INtB3_3AesKjf_E7encrypt", wide),
            ("_RINvMNtNtCs9_3aes8backends7x86_aesINtB3_3AesKjf_E7encrypt", wide),
            ("_RNvNtNtCs5_4sha26sha2563x8613digest_blocks", wide),
            ("_RNvNtNtCsj_9crc32fast11specialized9pclmulqdq9calculate", wide),
        ))

        self.assertEqual(found, {})

    def test_the_nightly_functions_of_httparse_and_ring_mulx_pass(self) -> None:
        # Ночь 44423152914: httparse выбирает модуль sse42 через
        # is_x86_feature_detected!, ring — MULX/ADX-код по проверке CPUID.
        found = self.module.violations(disassembly(
            ("_RNvNtNtCs2Yr9gS91ZMf_8httparse4simd5sse4227match_header_value_vectored",
             ["lddqu	(%rcx), %xmm3", "pcmpestri	$0x14, %xmm3, %xmm0"]),
            ("__bn_postx4x_internal", ["andn	%rax, %r12, %r12"]),
            ("__bn_sqrx8x_reduction", ["mulx	0x0(%rbp), %rax, %r8"]),
            ("__ecp_nistz256_mul_montx", ["mulx	%r9, %r8, %r9"]),
            ("__ecp_nistz256_sqr_montx", ["mulx	%r14, %r9, %r10"]),
            ("mulx4x_internal", ["mulx	(%rsi), %r8, %rax"]),
            ("ring_core_0_17_14__bn_mulx4x_mont", ["mulx	(%rsi), %r8, %rax"]),
            ("ring_core_0_17_14__bn_sqrx8x_internal", ["mulx	0x8(%rsi), %r8, %rax"]),
        ))

        self.assertEqual(found, {})

    def test_a_rust_module_named_after_a_feature_must_be_a_whole_word(self) -> None:
        lddqu = ["lddqu	(%rcx), %xmm3"]
        found = self.module.violations(disassembly(
            ("_RNvNtCs1_5crate8sse42abc5parse", lddqu),
            ("_RNvNtCs1_5crate6xsse425parse", lddqu),
            ("bn_mulx4x_mont_unprefixed_copy", ["mulx	(%rsi), %r8, %rax"]),
        ))

        self.assertEqual(len(found), 3)

    def test_each_given_binary_must_be_checkable(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            cases = {
                "missing": None,
                "notes.txt": b"not a binary",
                "arm": elf(machine=183, section_types=[1, 2]),
                # Без .symtab objdump называет код метками PLT или секцией.
                "stripped": elf(machine=62, section_types=[1, 11, 3]),
            }
            for name, content in cases.items():
                if content is not None:
                    (root / name).write_bytes(content)
            (root / "unica").write_bytes(elf(machine=62, section_types=[1, 2, 3]))

            self.module.require_checkable(root / "unica")
            for name in cases:
                with self.subTest(name=name), self.assertRaises(SystemExit):
                    self.module.main([str(root / "unica"), str(root / name), "--objdump", "false"])


if __name__ == "__main__":
    unittest.main()
