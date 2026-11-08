# 对 M3/M4 任务拆分的缺口与修改建议

对照 [m3-semantics](../../plan/m3-semantics.md)、[m4-cargo-sysroot](../../plan/m4-cargo-sysroot.md)、[m5-real-projects](../../plan/m5-real-projects.md) 与 [cargo-sysroot](../../design/build/cargo-sysroot.md)。本文不改这些文件，只列建议。

## 计划里没有任何任务承接的需求

| 缺口 | 依据 | 建议落点 |
|---|---|---|
| **LLVM intrinsic 的替代实现。** core_arch 用 `#[link_name = "llvm.*"]` 声明了 1046 个 x86/x86_64 外部函数；kfrc 没有 LLVM，每个被用到的都要映射到自有指令选择 | 三档实际需要 2 / 3 / 29 个（[llvm-asm-arch](llvm-asm-arch.md)） | 在 T27 增设子项“LLVM 名 → kfrc 降低表”，表中每项记录档位与测试；第三档的 AVX-512（EVEX）单列为 M5 风险 |
| **汇编器。** `asm!` 模板要汇编成机器码，naked 函数里还有 `.cfi_*` 指令；rustc 由 LLVM MC 完成 | hello world 档就要用：compiler-builtins 的 `div`/`rep movsb`/`__rust_probestack`，std_detect 的 `cpuid` | T27 增设“asm 子系统”：操作数约束 + 寄存器分配接口 + 汇编器（自写 x86-64 子集，或调用外部汇编器并在来源清单中单列）。T31 的“空模板 + 寄存器约束”改为引用这个子项 |
| **分配器 shim** | alloc 只声明 `__rust_alloc` 等，由编译器生成转发到 `__rdl_*`/`__rg_*` 的 shim（[runtime-link](runtime-link.md)） | T32 |
| **C `main` 入口 shim** | `#[lang = "start"]`，编译器生成 `main(argc, argv)` | T33（或 T19 的符号/链接部分） |
| **async/await 与协程变换** | sysroot 本体几乎不用；Cargo 的 `async-trait`、`futures-macro` 生成 async 代码 | M3 只在 T26 写了“缺失的 async 如实列出”。建议新增 M5 前置任务（T36 之前），包括协程布局、drop、unwind |
| **staged API / 稳定性检查** | `#[stable]` 12363、`#[unstable]` 5718（core） | T29 的 metadata 携带稳定性；T28 或 T34 至少实现“`#[unstable]` 项 + 未开 feature → 报错”。const 稳定性列入未覆盖清单 |
| **按 crate 求值的配置 intrinsic** | `ub_checks`、`overflow_checks` 与 `#[rustc_inherit_overflow_checks]`（53 处）按**生成代码的 crate** 的配置求值 | T25：InstanceKey 或代码生成上下文带上这两个配置位 |

## 现有任务的补充

- **T21：**
  - 除了已列的 const trait、`min_specialization`、负 impl、auto trait，还要加入：`with_negative_coherence`、`marker_trait_attr`、`fundamental`、`rustc_reservation_impl`、`rustc_coinductive`、`rustc_deny_explicit_impl`、`rustc_do_not_implement_via_object`；
  - 跨 crate 的 incoherent 固有 impl：`rustc_allow_incoherent_impl` 198 处，alloc/std 在 `str`、`[T]`、`f64` 上追加的方法；
  - `rustc_skip_during_method_dispatch`；
  - **`rustc_legacy_const_generics` 的调用改写**：x86 有 1544 处，ripgrep 的依赖直接调用。
- **T22：** `rustc_deprecated_safe_2024`，即按调用处 edition 决定 `env::set_var` 等是否 unsafe；`rustc_nounwind` 的边界（310 处）。
- **T23：**
  - 内建宏清单补上 `cfg_select!`（std 79 处）、`pattern_type!`、`const_format_args!`、`concat_bytes!`；
  - library 声明的 58 个 `#[rustc_builtin_macro]` 都要能绑定，未实现的在调用处报错；
  - `macro` 2.0 的 def-site hygiene 与 `rustc_macro_transparency`；
  - `macro_metavar_expr(_concat)`。
- **T24：**
  - 浮点常量求值用软浮点（移植 `rustc_apfloat`），覆盖 f16/f128；
  - const heap：`const_allocate`/`const_make_global`；
  - `const_eval_select`；
  - `type_name` 的字符串要与 rustc 的类型打印一致（可观察）；
  - niche 类型的有效性检查（`rustc_layout_scalar_valid_range_*`）；
  - 规模目标：core 8446 个 `const fn`，std 3169 个 `const` 项。
- **T27：**
  - LSDA 必须能被 std 自带的 `rust_eh_personality`（`sys/personality/gcc.rs`）解析，这和“生成异常表”不是同一个验收；
  - panic runtime 选择规则；
  - ELF TLS 模型与重定位；
  - 上面两个新子项（LLVM 替代表、asm 子系统）。
- **T28：** 支持 `--check-cfg`、build script 产出的 `--cfg`，以及 `#[link(…, cfg(…))]`；`-vV` 与探针编译的行为要让 5 个（ripgrep）/17 个（Cargo）探测型 build script 得到和稳定版 rustc 相同的结论，`RUSTC_BOOTSTRAP` 的处理也要与 rustc 相同。
- **T30：**
  - T00 写的“ripgrep 闭包有 10 个构建脚本（含 `cc`、`pkg-config`）”在 gnu 目标上不成立。数目是 10，但 `cc`/`pkg-config` 只在 musl（jemalloc）或 `pcre2` 下出现，ripgrep 默认构建的 host 侧依赖为 0（[deps-ripgrep](deps-ripgrep.md)）；
  - T30.b 的前置是原生编译 sysroot 的 `proc_macro` crate。
- **T31：**
  - 依赖关系加上 T30.a：compiler_builtins 的 build.rs 设置 `f16_enabled`/`f128_enabled` 等 cfg，sysroot 共有 4 个 build script（compiler_builtins、libc、object、std）；
  - 写明 `#![compiler_builtins]`/`#![no_builtins]` 语义（禁止把本 crate 的运算降成对自身的 libcall），以及 libcall 名表（导出 169 个）；
  - 写明 f16/f128 的布局与 SysV ABI；
  - 规模：core 加 x86 core_arch 共 207342 行（68557 + 138785），要给前端定时间和内存预算。可选优化是对未使用的 `#[inline]` core_arch 函数推迟体检查，但这会偏离 rustc“全部检查”的行为，需要标出。
- **T32：** 分配器 shim，以及 `#[global_allocator]`、`#[alloc_error_handler]` 两个内建属性宏的展开。
- **T33：**
  - C `main` shim；
  - `.init_array.00099` + `#[used]`（glibc 下捕获 argv）、`extern_weak`（3 处）、`weak!`/`syscall!`（23 处）、系统库链接（`gcc_s`、`dl`、`pthread` 等）；
  - std_detect 的 `cpuid`/`xgetbv`；
  - backtrace 符号化链（addr2line/gimli/object）原生编译。

## 阶段归属调整

- **SIMD 与 `target_feature` 从 M5 前移到 M4。** M5 的“根据真实阻塞项补齐 SIMD/target-feature”太晚：std 的 HashMap（hashbrown 的 SSE2 组）在实例化时就要通用 SIMD，ripgrep 的 memchr/aho-corasick 也在关键路径上。建议在 T27 或 T33 增设“通用 SIMD 与 x86 SSE2–AVX2 子集”，T35 只验收。
- **AVX-512 留在 M5，单列为 T36 的风险项**，原因是 EVEX 编码器工作量大，且 zlib-rs 的函数一定会生成代码。
- **混合 bootstrap 只允许用于 C ABI 的部件**（compiler-builtins、libgcc_s、系统 C 库），见 [mixed-link](mixed-link.md)。
