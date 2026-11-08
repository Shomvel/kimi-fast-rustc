# sysroot 需求清单：kfrc 原生编译 core/alloc/std 与 ripgrep/Cargo 依赖

依据 rustc 1.94.1（commit `e408947bfd200af42db322daf0fadfe7e26d3bd1`）的 `library/`，目标 `x86_64-unknown-linux-gnu`，2026-11-08 统计。源码用 Homebrew rust 1.94.1 附带的 rust-src，并与本地派生树在 `e408947b` 的 `library/` tree 哈希交叉核对（[sources](sources.md)）。Cargo 用 0.95.0（[T00](../../plan/m0/t00-freeze-inputs.md) 冻结的 crates.io 包）。所有数字都是 [scripts/](scripts/) 下 bash 脚本的原样输出；工作量与分组判断标为推断。对应计划：[M3](../../plan/m3-semantics.md)、[M4](../../plan/m4-cargo-sysroot.md)、[cargo-sysroot](../../design/build/cargo-sysroot.md)。

## 结论

1. **编 sysroot 需要的前端能力就是“全部 sysroot 源码”的能力，与 hello world 用到多少无关。** 共 93 个语言 feature（unstable 71、internal 21、incomplete 1）、54 种 `#[rustc_*]` 属性（其中 31 种改变语义）、203 个 lang item（core 194 个）、298 个 intrinsic（256 个被引用、203 个是 `const fn`）。规模上，x86 core_arch 有 138785 行，是 core 本体（68557 行）的两倍。
2. **const trait 是 core 的主体写法，不是边角。** `impl const` 407 处、`[const]` 约束 282 处。T21 不覆盖 HostEffect，core 就过不了类型检查。
3. **hello world 档就要用到 asm、unwind 和 LLVM intrinsic 替代。** 要用 asm 的地方：compiler-builtins 的 `div`/`rep movsb`、naked 的 `__rust_probestack`（`extern "custom"`，带 `.cfi_*`），以及 std_detect 的 `cpuid`。unwind 方面，LSDA 要能被 std 自带的 personality 解析。LLVM intrinsic 要替代 `_mm_pause`、`_xgetbv`。另外，编译器还要生成分配器 shim 和 C `main`。现有 T31 只写了“空模板 + 寄存器约束”，计划中也没有承接分配器 shim 和入口 shim 的任务。
4. **ripgrep 默认构建没有过程宏，也没有 C 依赖。** 34 个包都在 target 侧；10 个构建脚本，其中 5 个探测编译器；需要 SIMD（53 个 x86 函数，3 个 LLVM 名）和按函数的 `target_feature`。T00 所说的 `cc`/`pkg-config` 只在 musl 或 `pcre2` 下出现。
5. **Cargo 0.95.0 档新增：** 过程宏（9 个宏 crate）、7 个 C 库、系统调用级 `asm!`、19 个实际编译的 LLVM x86 intrinsic（SHA-NI、PCLMUL、AVX2）。默认构建不编译 async 代码，也不生成 AVX-512 代码。0.99.0 的 `cargo` 本身用 `async fn`，需要协程变换；AVX-512 在 0.99.0 默认构建下复核后同样不编译，原结论有误（[deps-cargo](deps-cargo.md) 末节）。
6. **混链官方 sysroot 省不下前端工作**，因为 T29 不读 `.rmeta`，泛型体只能从源码得到。它还会新增 mangling、`TypeId`、实例归属三项不可复用的工作，并带来静默错误风险。建议全原生；只允许 C ABI 部件（compiler-builtins/libgcc/glibc）在过渡期标注为混合使用（[mixed-link](mixed-link.md)）。

## 三档总表

| 档位 | 原生编译范围 | 新增需求（摘要） | 关键数字 | 增量工作量（推断，人周） |
|---|---|---|---|---:|
| **hello world** | sysroot 26 包中的 20 个：core、compiler_builtins、alloc、std、libc、hashbrown、std_detect、unwind、panic_unwind/abort、backtrace 依赖链等 | sysroot 全部前端（const trait、特化、负 impl、`decl_macro`、f16/f128、`legacy_const_generics`…）；CTFE（软浮点、const heap、`type_name`）；非 SIMD intrinsic 与 libcall 名表；asm 子系统 + 汇编器；`.eh_frame`/LSDA/landing pad；ELF TLS、`extern_weak`、`.init_array`；分配器 shim、`main` shim；`_mm_pause`/`_xgetbv`；4 个 sysroot build script | 语言 feature 71+21+1；A 组属性 31 种；lang item 203；intrinsic 被引用 256；compiler-builtins 导出 169；`_Unwind_*` 引用 152；`#[thread_local]` 10 | 32–48 |
| **ripgrep** | + 34 包 | 向量类型布局/ABI、通用 `simd_*`、SSE2–AVX2 指令子集、`is_x86_feature_detected!`；10 个 build script（5 个探测）；zmij 的空模板 `asm!` | x86 函数 53（llvm 3 / simd 33 / other 17）；`#[target_feature]` memchr 61、aho-corasick 60 | 9–13 |
| **Cargo 0.95.0** | + 307 包（273 target、22 host、12 两侧） | 原生 `proc_macro` + bridge；target 33 + host 2 个 build script；7 个 `links` C 库；rustix 的 `syscall` asm、`reg_byte`；19 个 LLVM x86 intrinsic（无 AVX-512） | 过程宏 crate 9；含 C 源码的 crate 中 curl-sys 232、libgit2-sys 252、libnghttp2-sys 251 个 C 文件；x86 函数 171（llvm 27，其中 8 个按 cfg 不编译） | 9–14 |

细节见 [tiers](tiers.md)，对任务拆分的修改建议见 [plan-gaps](plan-gaps.md)。

## 文件

| 文件 | 内容 |
|---|---|
| [sources.md](sources.md) | 源码来源与版本核对、各 crate 规模、std 依赖闭包 |
| [intrinsics.md](intrinsics.md) | 298 个 intrinsic 按类别的定义/引用/const/回退体，对后端与 CTFE 的要求 |
| [lang-items.md](lang-items.md) | 203 个 lang item 的种类与语义类别，入口类属性 |
| [rustc-attrs.md](rustc-attrs.md) | 54 种 `#[rustc_*]` 按“必须实现 / 可推迟 / 忽略”分组 |
| [features.md](features.md) | 246 个 feature gate 的语言/库分类、重点 feature、语言构造计数 |
| [builtin-macros.md](builtin-macros.md) | 58 个内建宏声明与 core/alloc/std 的调用量 |
| [llvm-asm-arch.md](llvm-asm-arch.md) | `core::arch::x86_64` 6269 个函数的实现方式、`llvm.*` 声明、`asm!` 使用点 |
| [runtime-link.md](runtime-link.md) | panic runtime、unwind、分配器 shim、入口、TLS、弱符号、compiler-builtins |
| [deps-ripgrep.md](deps-ripgrep.md) | ripgrep 依赖闭包逐 crate 的 unsafe/SIMD/构建脚本/探针 |
| [deps-cargo.md](deps-cargo.md) | Cargo 0.95.0 依赖闭包的过程宏、async、构建脚本、C 库、SIMD 与 `asm!`，以及与 0.99.0 的差异 |
| [tiers.md](tiers.md) | 三档需求明细与工作量 |
| [plan-gaps.md](plan-gaps.md) | M3/M4 任务（T21–T34）的缺口与修改建议 |
| [mixed-link.md](mixed-link.md) | 混链官方 sysroot / 只原生 core / 全原生的利弊与风险 |

## 复现

脚本都是独立 bash 脚本，路径用参数传入；依赖源码只做文本读取，不构建、不运行其中的脚本。`cargo tree` 只读 `Cargo.lock` 与清单（`--locked --offline`）。

```bash
cargo fetch --locked --manifest-path <bench>/ripgrep/Cargo.toml     # library 的依赖要加 RUSTC_BOOTSTRAP=1
cargo fetch --locked --manifest-path <bench>/cargo/Cargo.toml       # cargo/ 为 cargo-0.95.0.crate 的解包目录
scripts/run-all.sh <rust-src>/library <rustc-1.94.1>/compiler \
  "$CARGO_HOME/registry/src/<index>" <bench> <out> [rustc] [派生树 git 仓库 e408947b…]
```

`run-all.sh` 依次调用 `sources-check.sh`、`intrinsics.sh`、`lang-items.sh`、`rustc-attrs.sh`、`features.sh`、`builtin-macros.sh`、`llvm-asm-arch.sh`、`runtime.sh`、`deps-closure.sh`、`deps-scan.sh`、`asm-sites.sh`、`async-sites.sh`，每个脚本输出一个 `.txt`。Cargo 源码不在 `<bench>/cargo` 时用环境变量 `CARGO_DIR` 指定。本次草稿与原始输出保存在仓库外的 walkthrough 草稿目录 `sysroot/out/` 下，Cargo 0.95.0 的在 `sysroot/out/cargo-0.95.0/`。

**已知局限：**

- 文本匹配不区分 cfg，“被引用”“使用点”都是上界。x86_64 Linux 是否真的编译，在各文件的表格里按 cfg 人工核对并注明。
- 宏模板内的出现也计入。
- `cargo tree` 的 host 侧依赖按执行机（aarch64-apple-darwin）过滤，没有逐项核对 Linux 主机上的差异（`cc`、`jobserver` 等在两种 unix 上相同，推断）；target 侧由 `--target` 决定，不受影响。
