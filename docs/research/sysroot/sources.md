# 源码来源、规模与 std 依赖闭包

脚本：[sources-check.sh](scripts/sources-check.sh)、[deps-closure.sh](scripts/deps-closure.sh)。下文数字是脚本原样输出（2026-11-08，[run-all.sh](scripts/run-all.sh) 一次跑完）。

## 用的是哪份 `library/`

官方 1.94.1 源码 checkout 只有 `compiler/`，没有 `library/`。rustup 的 stable 与 nightly 分别是 1.99.1 和 1.101，都不能用。实际用了下面两份，并做了交叉核对：

| 来源 | 版本依据 | 用途 |
|---|---|---|
| **Homebrew rust 1.94.1 附带的 rust-src 组件**（主数据源） | 同一安装的 `rustc -Vv`：`release: 1.94.1`、`commit-hash: e408947bfd200af42db322daf0fadfe7e26d3bd1` | 所有统计 |
| 本地派生树（rustc-pipeline，HEAD `acea19e8`） | `src/version` 为 `1.94.1`；`git rev-parse e408947b:library` 与 `HEAD:library` 都是 `f06f3d11ca8ff7d42cf59a10549a9c10dc6b470a` | 证明派生改动没有碰 `library/` |

派生树核对结果（脚本输出）：HEAD 相对 `e408947b` 的 `library/` 改动文件数 0，未提交改动 0，未跟踪文件 0；`library/` 下唯一的子模块是 `library/backtrace`，派生树里这个子模块没有检出。

两份 `library/` 的 `diff -rq` 汇总：

```
only-first backtrace 12        # rust-src 有 backtrace 子模块内容，派生树为空目录
only-first library 1           # Homebrew 包里多一份重复的 library/library（2274 个文件，打包产物）
only-second compiler-builtins 1
only-second portable-simd 1
only-second stdarch 5          # 派生树多出 .gitignore、stdarch-verify 等开发文件
```

没有“内容不同”的文件。std 用 `#[path = "../../backtrace/src/lib.rs"]` 把 backtrace 源码编进自身（`std/src/lib.rs` L691），所以必须用带子模块的 rust-src。脚本对 Homebrew 包里重复的 `library/library` 做了排除。

## 规模

非空、非 `//` 注释行（含 `#[cfg(test)]` 内联测试，排除 `tests/`、`benches/`）：

| crate | .rs 文件 | 代码行 |
|---|---:|---:|
| core（`core/src`） | 262 | 68557 |
| core_arch x86 + x86_64（`#[path]` 编入 core） | 70 | 138785 |
| core_arch 全部架构（含 x86） | 136 | 302440 |
| core_simd（`#[path]` 编入 core） | 37 | 4327 |
| alloc | 67 | 23833 |
| std（含全部平台的 `sys/`） | 601 | 80821 |
| backtrace（编入 std） | 36 | 5599 |
| std_detect | 40 | 2693 |
| panic_unwind / panic_abort / unwind | 7 / 3 / 4 | 433 / 100 / 567 |
| compiler_builtins / 其中的 libm | 45 / 145 | 5633 / 15362 |
| proc_macro / test | 16 / 25 | 3606 / 4480 |

**x86 的 core_arch 是 core 本体的两倍。** core 在 `lib.rs` L368 用 `#[path]` 引入整个 `core_arch`，按 `cfg(target_arch)` 只保留 x86/x86_64 两个目录。这 6269 个函数（见 [llvm-asm-arch](llvm-asm-arch.md)）都是 `#[inline]`，不用就不生成代码，但前端仍要解析、解析名称、类型检查并产出可内联体。

## std 的依赖闭包（x86_64-unknown-linux-gnu）

bootstrap 默认的 std features 是 `panic-unwind`，`config.backtrace` 默认为真时再加 `backtrace`（`src/bootstrap/src/lib.rs` 的 `std_features`）；发行版在有预编译 compiler-rt 时还加 `compiler-builtins-c`。下面按 `sysroot` 包、`backtrace,panic-unwind` 求闭包（`RUSTC_BOOTSTRAP=1 cargo tree --locked --offline`，不构建）：

```
addr2line 0.25.1  adler2 2.0.1  alloc  cfg-if 1.0.4  compiler_builtins 0.1.160  core
getopts 0.2.24  gimli 0.32.3  hashbrown 0.16.1  libc 0.2.178  memchr 2.7.6  miniz_oxide 0.8.9
object 0.37.3  panic_abort  panic_unwind  proc_macro  rustc-demangle 0.1.26
rustc-literal-escaper 0.0.7  rustc-std-workspace-{alloc,core,std}  std  std_detect  sysroot  test  unwind
```

共 26 个包，全部在 target 侧。分工：

- **hello world 链接进来的：** core、compiler_builtins、alloc、std、libc、cfg-if、hashbrown、std_detect、unwind、panic_unwind（或 panic_abort），以及 backtrace 符号化用的 addr2line、gimli、object（及其依赖 memchr）、miniz_oxide、adler2、rustc-demangle。
- **只给 test harness 用：** test、getopts。
- **只给过程宏用（host 侧）：** proc_macro、rustc-literal-escaper。
- `rustc-std-workspace-*` 是转发壳（各 2–7 行）。

registry 里的这几个包靠 `rustc-dep-of-std` feature 改成依赖 `core`/`alloc`，并打开各自的 nightly 路径（例如 `hashbrown` 的 `nightly`）。它们算 sysroot 的一部分，统计见 [runtime-link](runtime-link.md) 和 [tiers](tiers.md)。
