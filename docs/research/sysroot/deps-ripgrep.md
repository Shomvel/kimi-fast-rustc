# ripgrep 15.2.0 依赖闭包的额外需求

脚本：[deps-closure.sh](scripts/deps-closure.sh) → [deps-scan.sh](scripts/deps-scan.sh)、[asm-sites.sh](scripts/asm-sites.sh)。目标是 `x86_64-unknown-linux-gnu`、默认 features，只读 `Cargo.lock` 与清单（`cargo tree --locked --offline -e normal,build --no-dedupe`）。依赖源码在 `$CARGO_HOME/registry/src/<index>/` 下，只做文本扫描。

## 默认构建实际编译的包

```
target crates=34 rs_lines=498498 unsafe=1923 proc_macro=0 build_rs=10 links=0 sys=0 core_arch=5
       target_feature=2 asm=1 repr_simd=0 feature_always=0 feature_cfg=12 rustc_probe=5 c_src=1 custom_cfg=11
```

34 个包都在 target 侧，**host 侧为 0**（没有过程宏，也没有 build-dependency）：

- 工作区 9 个：`ripgrep`、`grep`、`grep-cli`、`grep-matcher`、`grep-printer`、`grep-regex`、`grep-searcher`、`globset`、`ignore`；
- registry 25 个：`aho-corasick`、`anyhow`、`bstr`、`cfg-if`、`crossbeam-{deque,epoch,utils}`、`encoding_rs`、`encoding_rs_io`、`itoa`、`lexopt`、`libc`、`log`、`memchr`、`memmap2`、`regex-automata`、`regex-syntax`、`same-file`、`serde`、`serde_core`、`serde_json`、`termcolor`、`textwrap`、`walkdir`、`zmij`。

**与 T00/T30 的出入：** 10 个构建脚本的数目对得上（`anyhow`、`crossbeam-deque`、`crossbeam-epoch`、`crossbeam-utils`、`libc`、`ripgrep`、`serde`、`serde_core`、`serde_json`、`zmij`），但 gnu 目标的闭包里**没有 `cc` 和 `pkg-config`**。它们只出现在 musl 目标（`tikv-jemallocator`）或 `pcre2` feature 下。所以 ripgrep 默认构建既没有 host 侧依赖 crate，也没有 C 依赖。`walkdir` 的 `c_src=1` 是仓库里的非构建文件。

## 逐 crate 的特殊需求（只列非零项）

| crate | 行数 | unsafe | `core::arch` 行 | x86 函数 l/a/s/o | `#[target_feature]` | 特性检测 | `asm!` | 其他 |
|---|---:|---:|---:|---|---:|---:|---:|---|
| memchr 2.8.3 | 9146 | 333 | 12 | 0/0/8/6 | 61 | 4 | 0 | SSE2 + AVX2 两套实现，运行时分派 |
| aho-corasick 1.1.4 | 13032 | 227 | 8 | 2/0/19/5 | 60 | 10 | 0 | Teddy 用 `pshufb`（`_mm_shuffle_epi8`/`_mm256_shuffle_epi8`，LLVM 名） |
| encoding_rs 0.8.35 | 132600 | 264 | 8 | 1/0/1/0 | 0 | 0 | 0 | `_mm_packus_epi16`（LLVM 名），SSE2 基线 |
| bstr 1.13.0 | 6757 | 39 | 1 | 0/0/2/2 | 0 | 0 | 0 | SSE2 基线 |
| zmij 1.0.23 | 1855 | 70 | 3 | 1/0/17/11 | 0 | 0 | 7 | build.rs 探测 rustc；`asm!("/*{0}*/", inout(reg) x)` 优化屏障 |
| ripgrep | 37817 | 5 | 0 | — | 0 | 3 | 0 | build.rs |
| libc 0.2.186 | 123563 | 424 | 0 | — | 0 | 0 | 0 | build.rs 探测 rustc，设置 `gnu_time_bits64`、`freebsd1x` 等自定义 cfg |
| serde / serde_core 1.0.228 | 13306 / 8447 | 2 / 2 | 0 | — | 0 | 0 | 0 | build.rs 探测 rustc，设置 `no_core_cstr`、`no_std_atomic64` 等；`include!(concat!(env!("OUT_DIR"), …))` |
| anyhow 1.0.103 | 2897 | 106 | 0 | — | 0 | 0 | 0 | build.rs **编译探针程序**（`compile_probe`）检测 `error_generic_member_access` |
| crossbeam-{deque,epoch,utils} | 1395/3004/2187 | 41/129/76 | 0 | — | 0 | 0 | 0 | build.rs 设置 `crossbeam_no_atomic` 等 cfg |
| regex-automata / regex-syntax | 33397 / 52876 | 55 / 0 | 0 | — | 0 | 0 | 0 | 体量大，纯安全代码为主（regex-syntax 的 Unicode 表） |

x86 函数一列是“该 crate 引用的不同 `core::arch` 函数，按 llvm/asm/simd/other 分”。整个闭包取并集：

```
# 合计：不同 x86 intrinsic 名称（全部 crate 取并集）：total llvm asm simd other
53 3 0 33 17
# 需要自实现的 LLVM intrinsic 名称
_mm256_shuffle_epi8 _mm_packus_epi16 _mm_shuffle_epi8
```

没有 crate 用 `#[repr(simd)]`、`global_asm!` 或不加条件的 `#![feature]`（12 个 crate 有 `cfg_attr(…, feature(…))`，只在 nightly feature 下生效）。

## 对 kfrc 的需求清单

1. **SIMD 代码生成**：`__m128i`/`__m256i` 的布局与传参。Rust ABI 下向量按间接方式传递，要与 rustc 一致；33 个通用 `simd_*` 路径（比较、按位、`movemask`、移位、`alignr`/`unpack`/`permute` 类 shuffle、extract）；17 个 other 类（`loadu`/`set1`/`setzero`/`storeu` 等，降成普通内存访问与 splat）；3 个 LLVM x86 intrinsic（`pshufb` 128/256、`packuswb`）。
2. **按函数的 `target_feature`**：`#[target_feature(enable = "avx2")]` 函数内允许 AVX2 编码，函数间内联要遵守特性子集规则；`is_x86_feature_detected!` 走 std_detect 的缓存（atomics + cpuid）。
3. **构建脚本**：10 个 host 程序要用 kfrc（或标记的 bootstrap 工具链）编译并运行，产出 `cargo:rustc-cfg`/`rustc-check-cfg`，涉及 11 个 crate 的自定义 cfg。其中 5 个探测编译器：读 `RUSTC` 运行 `-vV` 或编译探针。**探针的接受/拒绝必须与稳定版 rustc 一致**，例如 anyhow 的探针在 kfrc 误接受 nightly 特性时会切到另一条代码路径。
4. **`asm!`**：zmij 的 7 处中，x86_64 分支编译 4 处（L508、L1114、L1418、L1528，按 cfg 人工核对），都是空模板 + `inout(reg)` 的优化屏障，其中 L1418 用了 `{0:e}` 修饰符；`inout(vreg)` 与 `{0:w}` 在 aarch64 分支里。
5. 没有过程宏和 C 依赖，T30.b 与 C 工具链都不在 ripgrep 默认构建的关键路径上。
