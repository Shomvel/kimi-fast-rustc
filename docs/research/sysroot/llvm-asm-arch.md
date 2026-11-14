# LLVM intrinsic、`asm!` 与 `core::arch::x86_64`

脚本：[llvm-asm-arch.sh](scripts/llvm-asm-arch.sh)（`list` 参数输出逐函数表 `x86-fns.txt`）、[asm-sites.sh](scripts/asm-sites.sh)。kfrc 没有 LLVM，所以有两类东西要自己实现：一是 `#[link_name = "llvm.*"]` 声明的外部函数，二是 `asm!` 模板，它在 rustc 中交给 LLVM 的集成汇编器处理。

## `core::arch::x86_64` 的规模与实现方式

x86/x86_64 目录（排除 `test.rs`，每个文件截到 `mod tests`）中的公开函数，按函数体分类：调用了本文件 `llvm.*` 声明的记为 llvm，否则看有没有 `asm!`，再看是否只用通用 `simd_*` intrinsic，其余记为 other。

```
# core::arch x86/x86_64 公开 intrinsic 函数总数: 6269（不同名称 6267）
simd 3028
llvm 1864
other 1320
asm 57
# baseline x86_64（sse/sse2，含 x86_64/ 下同名特性）: 334
```

按 `#[target_feature(enable = ...)]` 的第一个特性分组（节选）：

```
# feature total llvm asm simd other
avx512f 2633 976 10 1326 321      avx512fp16 935 220 19 246 450     avx512bw 828 39 0 628 161
avx512dq 399 80 0 160 159         sse2 227 59 4 104 60              avx2 194 58 1 132 3
avx 189 36 3 92 58                avx512vbmi2 150 37 0 89 24        sse 108 53 1 28 26
sse4.1 61 16 0 42 3               fma 32 0 0 32 0                   gfni 27 27 0 0 0
sse4.2 19 19 0 0 0                ssse3 16 8 0 8 0                  - 27 7 13 1 6
```

`link_name = "llvm.*"` 声明：

```
core_arch/x86 980 976
core_arch/x86_64 66 66
core_arch/全部架构 5219 -
core_arch 之外: 1 个文件 unwind/src/wasm.rs
```

按族统计，x86+x86_64 的 LLVM 名称前几位是 `x86.avx512` 537、`x86.avx512fp16` 146、`x86.avx2` 65、`x86.sse2` 45、`x86.avx` 31、`x86.sse` 29、`x86.sse42` 18、`x86.aesni` 14。

**含义：**

1. **只要实现被用到的那部分。** 这些函数都是 `#[inline]`，只在被调用时生成代码。下表是三档实际引用到的 x86 函数，来自 [deps-scan.sh](scripts/deps-scan.sh) 的 `x86` 模式（对每个 crate 的源码做文本匹配，不考虑 cfg，是上界）：

   | 范围 | 不同函数 | llvm | asm | simd | other | 需要自实现的 LLVM 名称来源 |
   |---|---:|---:|---:|---:|---:|---|
   | sysroot（core、std、std_detect、hashbrown、memchr…） | 25 | 6 | 1 | 9 | 9 | `_mm_pause`、`_xgetbv`；`_rdrand*`、`_rdtsc` 只出现在 SGX 路径 |
   | ripgrep 依赖闭包 | 53 | 3 | 0 | 33 | 17 | `_mm_shuffle_epi8`、`_mm256_shuffle_epi8`、`_mm_packus_epi16` |
   | Cargo 0.95.0 依赖闭包 | 171 | 27 | 1 | 101 | 42 | SHA-NI 7 个、PCLMUL、AVX2 的 `madd`/`maddubs`/`sad`/`permutevar8x32`、`_mm256_zeroupper`、`_mm_prefetch`、`_xgetbv`；另有 `_mm512_*` 6 个与 `_rdrand*` 2 个按 cfg 不编译 |

2. **simd 类的 3028 个函数依赖通用 SIMD 语义**：kfrc 后端要有向量类型（`#[repr(simd)]`）、`simd_shuffle` 等通用 intrinsic 和按 `target_feature` 选择指令编码的能力，见 [intrinsics](intrinsics.md)。
3. **前端负担与是否使用无关。** 6269 个函数共 138785 行（[sources](sources.md)），带 6265 个 `#[target_feature]`、1544 个 `#[rustc_legacy_const_generics]`、3316 个 `#[rustc_const_unstable]`，编 core 时都要解析和类型检查。
4. **默认构建不需要 AVX-512。** Cargo 0.95.0 闭包里引用 `_mm512_*` 的 zlib-rs 0.5.5 把 AVX-512 代码放在非默认 feature `avx512`/`vpclmulqdq` 下，并用编译期 `cfg!(target_feature = …)` 分派；rand 的 AVX-512 分支也在 `cfg(target_feature = "avx512bw")` 下。基线 x86_64 目标不生成 EVEX 代码，只有用 `-C target-cpu`/`target-feature` 打开 AVX-512 时才需要（[deps-cargo](deps-cargo.md)）。

## `asm!` / `naked_asm!` / `global_asm!`

sysroot 中与 x86_64 Linux 相关的使用点（脚本按路径粗分；“generic”需要按 cfg 人工核对，核对结果写在右列）：

| 文件 | 次数 | x86_64 Linux 是否编译 | 形态 |
|---|---:|---|---|
| `compiler-builtins/src/int/specialized_div_rem/mod.rs` L189–L209 | 2 | 是（除非开 `no-asm`） | `div {0}` + `inlateout("rax")`/`inlateout("rdx")` + `options(pure, nomem, nostack)` |
| `compiler-builtins/src/mem/x86_64.rs` | 12 | 是（`mem/mod.rs` L7–L10：`not(feature = "no-asm")` 且 x86_64） | `rep movsb/stosb` 等字符串指令，显式寄存器 |
| `compiler-builtins/src/probestack.rs` L55–L59 | 3 | 是（x86_64 分支） | `#[unsafe(naked)]` + `extern "custom"` + `naked_asm!`，模板里有 `.cfi_*` 指令 |
| `compiler-builtins/src/{x86,x86_64}.rs` | 2+1 | 否（Windows/UEFI 的 `___chkstk`） | — |
| `compiler-builtins/src/int/udiv.rs` | 2 | 否（AVR/SPARC） | — |
| `core/src/num/dec2flt/fpu.rs` | 2 | 否（32 位 x86 无 SSE2） | — |
| `core_arch/src/x86{,_64}/` 的 57 个 asm 类函数 | 45+5 | 用到才生成 | `cpuid`（保存 `rbx`）、`bt*`、`pushf/popf` 等 |
| std（27 处 `asm!` + 2 处 `global_asm!`） | 29 | 否（windows、sgx、xous、vexos、teeos、fortanix） | — |

std_detect 在运行时检测 CPU 特性，要调用 `__cpuid_count`（asm）和 `_xgetbv`（LLVM intrinsic）。`detect_features` 是非泛型函数，会编进 std_detect 本身，所以 **hello world 档就需要 asm**。

**结论：** kfrc 至少要具备以下 asm 能力：

1. 操作数约束：`reg`、显式寄存器（`"rax"`）、`in`/`out`/`inout`/`lateout`/`inlateout`、`options(pure, nomem, nostack, readonly, preserves_flags)`，以及模板修饰符（`{0:e}`、`{0:r}`）；
2. naked 函数与 `extern "custom"` ABI；
3. 把 Intel 语法模板汇编成机器码，包括 `.cfi_*` 等汇编指令。

第 3 点在 rustc 里由 LLVM MC 完成，kfrc 要么自写一个 x86-64 汇编子集，要么调用外部汇编器（GNU as 的 `.intel_syntax noprefix`），后者属于外部工具链依赖，要在来源清单中单列。现有 T31 只写了“空模板 + 寄存器约束”（来自 zmij），低估了需求。
