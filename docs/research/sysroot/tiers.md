# 三档需求与工作量

三档按“要原生编出并运行什么”划分。下一档包含上一档的全部需求。数字来自本目录各文件引用的脚本输出；工作量是推断，单位为人周，指**在 M3 现有 T21–T27 交付物之上**新增的部分。

## 第一档：hello world

**范围：** 原生编译 core、compiler_builtins、alloc、std，以及 std 的 registry 依赖（libc、hashbrown、std_detect、unwind、panic_unwind、addr2line、gimli、object、miniz_oxide、adler2、rustc-demangle、memchr、cfg-if），再链接并运行 `println!("hello")`。

关键事实：编 sysroot 时，**前端要处理全部源码**，不只是 hello world 用到的部分。每个 item 都要解析、名称解析、类型检查，泛型体和 `#[inline]` 体要写进 metadata；后端只为非泛型、非 `#[inline]` 的项生成代码。所以第一档的前端需求等于“整个 sysroot 的前端需求”，三档的差别主要在后端与 host 工具。

| 编号 | 需求 | 量化依据 | 工作量（推断） |
|---|---|---|---:|
| F1 | 类型系统类语言 feature：const trait（HostEffect）、`min_specialization`、负 impl + `with_negative_coherence`、auto trait、`marker_trait_attr`、`fundamental` | `impl const` 407、`[const]` 282、`const trait` 69；`default fn` 90/34/10；负 impl 24/9/27（[features](features.md)） | 6–9 |
| F2 | 其余语言 feature：`never_type` 回退、`adt_const_params`、`unsized_fn_params`、`pattern_types`、TAIT（std 1 处）、`decl_macro` 的 def-site hygiene、`macro_metavar_expr(_concat)`、`try_blocks`、`if_let_guard`、f16/f128 类型 | unstable 71 + internal 21 + incomplete 1 个语言 feature | 4–6 |
| F3 | `#[rustc_*]` A 组 31 种的语义：`legacy_const_generics` 改写调用、`inherit_overflow_checks`、跨 crate 的 incoherent 固有 impl、`promotable`、`coinductive`、`reservation_impl` 等 | [rustc-attrs](rustc-attrs.md) | 2–3 |
| F4 | 203 个 lang item 全部接上；`panic_const_*` 断言降低；`Box` 内建语义 | [lang-items](lang-items.md) | 2–3 |
| F5 | sysroot 的 CTFE 覆盖面：const heap、`const_eval_select`、`type_id`/`type_name`、niche 有效性、**f16/f32/f64/f128 的软浮点常量求值**（rustc 用 `rustc_apfloat`，与宿主无关） | `const` 项 1195/27/3169、`static` 37/1/140；203 个 const intrinsic | 4–6 |
| F6 | 规模：core 68557 行 + x86 core_arch 138785 行 + alloc 23833 + std 80821 + backtrace 5599 | [sources](sources.md) | 性能风险，不单列 |
| B1 | 非 SIMD intrinsic 的后端降低与 libcall 名表（含 i128、f16/f128） | 256 个被引用；f16/f128 版本被引用 42；compiler-builtins 导出 169 个（[intrinsics](intrinsics.md)、[runtime-link](runtime-link.md)） | 3–4 |
| B2 | `asm!` 基础设施：操作数约束、显式寄存器、修饰符、naked + `extern "custom"`、Intel 语法汇编（含 `.cfi_*`） | compiler-builtins 的 `div`、`rep movsb`、`__rust_probestack`；std_detect 的 `cpuid`（[llvm-asm-arch](llvm-asm-arch.md)） | 4–6（外部汇编器 2–3） |
| B3 | unwind：全部函数的 `.eh_frame`、与 std 的 `rust_eh_personality` 兼容的 LSDA、landing pad、`catch_unwind` | `_Unwind_*` 引用 152 处 | 4–6 |
| B4 | ELF TLS 模型与重定位、`extern_weak`、`link_section`/`#[used]`、`#[link(…, cfg)]` | `#[thread_local]` 10、`linkage` 3、`.init_array.00099` | 2–3 |
| B5 | 编译器生成的 shim：分配器 shim、C `main`、`#[rustc_std_internal_symbol]` 命名；`#![no_builtins]`/`#![compiler_builtins]` 语义 | [runtime-link](runtime-link.md) | 1–2 |
| B6 | LLVM intrinsic 替代：`_mm_pause`（`hint::spin_loop`）、`_xgetbv`（std_detect） | 2 个 | 0.5 |
| H1 | sysroot 的 4 个 build script（compiler_builtins、libc、object、std）要运行，生成自定义 cfg | [deps-scan](scripts/deps-scan.sh) `scan-sysroot` | 计入 T30.a |

**第一档合计约 32–48 人周（推断）。** 关键路径是 F1→F5→“core 类型检查通过”，因为 core 只要有一个 item 过不了，就产出不了任何 metadata。B2/B3 可以并行，但都必须在 T31 之前就绪。

## 第二档：ripgrep 默认构建

在第一档之上（[deps-ripgrep](deps-ripgrep.md)）：

| 需求 | 量化依据 | 工作量（推断） |
|---|---|---:|
| SIMD 类型布局与 ABI、通用 `simd_*` 降低（shuffle 用 const 数组索引） | 33 个 simd 类 + 17 个 other 类 x86 函数；hashbrown 的 SSE2 组在实例化 `HashMap` 时也需要 | 4–6 |
| 按函数 `target_feature`，SSE2–AVX2 中实际用到的指令编码；`is_x86_feature_detected!` | memchr 61、aho-corasick 60 个 `#[target_feature]` | 3–4 |
| 3 个 LLVM x86 intrinsic（`pshufb` ×2、`packuswb`） | — | 0.5 |
| 10 个构建脚本作为 host 程序运行，其中 5 个探测编译器；自定义 cfg | anyhow 的 `compile_probe` | 1–2 |
| `asm!` 增量：zmij 4 处空模板（含 `{0:e}`） | — | 计入 B2 |

**第二档增量约 9–13 人周（推断）。** 没有过程宏，也没有 C 依赖。

## 第三档：Cargo

在第二档之上（[deps-cargo](deps-cargo.md)），目标为 Cargo 0.95.0（[T00](../../plan/m0/t00-freeze-inputs.md)）：

| 需求 | 量化依据 | 工作量（推断） |
|---|---|---:|
| 过程宏：原生编译 `proc_macro` + bridge 服务端 + runner | 9 个宏 crate、22 个 host-only 包 | 5–8 |
| 19 个 LLVM x86 intrinsic：SHA-NI 7 个、PCLMUL、AVX2 的 `madd`/`maddubs`/`sad`/`permutevar8x32`、`_mm256_zeroupper` 等，都是 legacy SSE/VEX 编码 | 脚本列出 27 个，其中 `_mm512_*` 6 个与 `_rdrand*` 2 个按 cfg 核对不编译 | 2–3 |
| `asm!` 增量：`syscall` 与显式寄存器破坏列表、`reg_byte` 寄存器类、注释模板 | rustix 14、constant_time_eq 1、zmij 1 | 1 |
| target 33 + host 2 个构建脚本（16 + 2 个探测编译器）；7 个 `links` C 库的链接与来源记账 | — | 1–2 |

**第三档增量约 9–14 人周（推断）。** 默认构建不编译 async 代码，也不生成 AVX-512 代码，所以不需要协程变换和 EVEX 编码。如果目标改回 Cargo 0.99.0，要加回 async/协程（4–6），并补上 `cmpxchg16b`、`xmm_reg`；原先 0.99.0 估计的 15–24 人周里还算了 AVX-512，复核后它在 0.99.0 默认构建下也不编译（见 [deps-cargo](deps-cargo.md) 末节）。

## 汇总

| 档位 | 原生编译的 Rust 包 | 新增需求的主体 | 增量工作量（推断） |
|---|---|---|---:|
| hello world | sysroot 26 包中的 20 个（含转发壳 rustc-std-workspace-core/alloc；panic_unwind 与 panic_abort 二选一链接） | 全部 sysroot 前端 + intrinsic/asm/unwind/TLS/shim | 32–48 |
| ripgrep | + 34 包 | SIMD、`target_feature`、构建脚本 | 9–13 |
| Cargo 0.95.0 | + 307 包（273 target、22 host、12 两侧） | 过程宏、syscall asm、SHA-NI/PCLMUL/AVX2 的 LLVM 名、C 库 | 9–14 |
