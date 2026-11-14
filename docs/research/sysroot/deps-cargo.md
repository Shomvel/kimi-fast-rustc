# Cargo 0.95.0 依赖闭包的额外需求

方法同 [deps-ripgrep](deps-ripgrep.md)，包选 `cargo`（`-p cargo`），目标 `x86_64-unknown-linux-gnu`，默认 features。源码是 crates.io 包 `cargo-0.95.0.crate`（SHA-256 `0af45a75a5265a587a8d9e4098cd2b12a07b34bcbd9eb7952ae6a62ea6d65c4d`，git `85eff7c8`）的解包目录，依赖用 `cargo fetch --locked` 取得，版本与 [T00](../../plan/m0/t00-freeze-inputs.md) 冻结的一致。不用 0.99.0 的原因见 T00：它要求 Rust 1.96，参考 rustc 1.94.1 编不了。

## 规模

```
# closure-cargo.txt 的 role 计数
  12 both   22 host  273 target
# 合计（role=target 与 both 计入 target 侧；host 单列）
host crates=22 rs_lines=98917 unsafe=223 proc_macro=9 build_rs=2 links=0 sys=0 core_arch=1 target_feature=0 asm=1 repr_simd=0 feature_always=0 feature_cfg=5 rustc_probe=2 c_src=1 custom_cfg=3
target crates=285 rs_lines=1985221 unsafe=18430 proc_macro=0 build_rs=33 links=7 sys=9 core_arch=26 target_feature=12 asm=9 repr_simd=0 feature_always=0 feature_cfg=126 rustc_probe=16 c_src=10 custom_cfg=42
```

`both` 指同一版本同时被 target 和 host 使用（`cfg-if`、`crc32fast`、`filetime`、`flate2`、`jobserver`、`libc`、`libz-rs-sys`、`memchr`、`shlex`、`tar`、`unicode-ident`、`zlib-rs`）。它们要按 host 和 target 各编一次；目标相同的情况下也可以共享，但来源清单要分开记账。

## 过程宏与 host 图

9 个过程宏 crate：`color-print-proc-macro`、`displaydoc`、`maybe-async`、`serde_derive`、`thiserror-impl`、`tracing-attributes`、`yoke-derive`、`zerofrom-derive`、`zerovec-derive`。它们依赖 `proc-macro2`、`quote`、`syn`、`synstructure`、`nom`、`minimal-lexical`，以及构建期工具 `cc`、`pkg-config`、`vcpkg`、`autocfg`、`version_check`、`cfg_aliases`、`find-msvc-tools`。

要求：

- sysroot 里的 `proc_macro` crate（3606 行，打开 `negative_impls`、`decl_macro` 等 9 个 feature）要原生编译，作为 host 侧库；
- kfrc 实现过程宏 bridge 的服务端：TokenStream/Span/Literal 的句柄表，以及 `quote!` 内建过程宏；
- 用自己的协议把宏 dylib 或进程接起来（T30.b）；
- host 侧只有 `proc-macro2`、`quote` 两个 build script，都会调用 `RUSTC` 读版本；
- `maybe-async` 以 `is_sync` 打开（gix-protocol 的 `blocking-client` 带入），把 gix 中标了 `#[maybe_async]` 的 `async fn` 改写成同步函数。展开结果里没有 async，但展开前的源码里有 `async fn`，kfrc 的解析器要接受它并把记号原样交给宏。

## async/await

```
# async-cargo.txt（async-sites.sh）：合计：crate 数 文件数 使用行数
7 26 163
# 按 crate：crate ver 文件数 使用行数
anyhow 1.0.100 1 2
gix 0.77.0 5 20
gix-packetline 0.20.0 3 45
gix-protocol 0.55.0 8 38
gix-transport 0.52.1 6 51
prodash 30.0.1 2 5
tracing 0.1.44 1 2
```

逐文件按 cfg 人工核对（x86_64 Linux，默认 features）：

| crate | 使用点 | 是否编译 | 依据 |
|---|---|---|---|
| gix 0.77.0 | `clone/fetch`、`remote/connect` 等 5 个文件 | 否（改写为同步） | 都标 `#[maybe_async]`，`maybe-async` 开了 `is_sync` |
| gix-protocol 0.55.0 | `util.rs`、`handshake/function.rs`、`fetch/function.rs` | 否（改写为同步） | 同上 |
| gix-protocol 0.55.0 | `*/async_io.rs`、`ls_refs.rs`、`handshake/mod.rs` | 否 | `cfg(feature = "async-client")`，实际 features 为 `blocking-client,fetch,handshake` |
| gix-transport 0.52.1 | `client/async_io/`、`client/git/async_io.rs`、`client/capabilities.rs` L257 起 | 否 | `cfg(feature = "async-client")` |
| gix-packetline 0.20.0 | `async_io/` | 否 | `cfg(feature = "async-io")`，实际只开 `blocking-io` |
| prodash 30.0.1 | `render/tui/` | 否 | `cfg(feature = "render-tui")` 未开 |
| anyhow 1.0.100 | `ensure.rs` | 不是 async 代码 | `macro_rules!` 匹配 `async {…}` 记号 |
| tracing 0.1.44 | `instrument.rs` | 不是 async 代码 | `#[must_use = "… .await …"]` 字符串 |

**结论：Cargo 0.95.0 默认构建不编译任何 async 代码，不需要协程变换。** `Instrumented<T>` 这类手写 `Future`/`poll` 实现只用到 `Future`、`Pin`、`Context` 等 lang item，不需要协程。kfrc 仍要能解析 async 语法（被 cfg 关掉的项也要先解析，再按 cfg 剔除）。

## 构建脚本、链接库与 C 依赖

- **构建脚本**：target 侧 33 个（两侧都用的 `crc32fast`、`libc`，以及 `anyhow`、`blake3`、`cargo`、`crossbeam-utils`、`curl`、`erased-serde`、`generic-array`、`getrandom`、`heapless`、`icu_normalizer_data`、`icu_properties_data`、`im-rc`、带 `links` 的 7 个 `-sys`、`nix`、`num-traits`、`parking_lot_core`、`rustix`、`serde`、`serde_core`、`serde_json`、`thiserror`、`typeid`、`typenum`、`zerocopy`、`zmij`），host 侧 2 个（`proc-macro2`、`quote`）。探测编译器的有 target 侧 16 个、host 侧 2 个。`heapless` 的 build.rs 用 ARM 汇编探针决定 `arm_llsc`，kfrc 在 x86_64 上对这个探针必须编译失败，才能和 rustc 得到相同结论。
- **`links` 7 个**：`curl-sys(curl)`、`libgit2-sys(git2)`、`libnghttp2-sys(nghttp2)`、`libsqlite3-sys(sqlite3)`、`libssh2-sys(ssh2)`、`libz-sys(z)`、`openssl-sys(openssl)`。
- **`-sys` 9 个**：上面 7 个，加 `linux-raw-sys`（纯 Rust 常量与类型）和 `libz-rs-sys`（zlib-rs 的 C ABI 外壳，纯 Rust，`flate2` 默认走它）。后两个都不链接 C。
- **含 C/汇编源码的 crate**（`.c/.cc/.cpp/.S/.s` 文件数）：`curl-sys` 232、`libgit2-sys` 252、`libnghttp2-sys` 251、`libssh2-sys` 92、`libz-sys` 88、`blake3` 20（SIMD 的 `.S`/`.c`，默认通过 `cc` 编译）、`libsqlite3-sys` 3（`bundled` amalgamation）、`openssl-sys` 1（实际链接系统 OpenSSL，靠 pkg-config 找）。`cc`、`nix`、`walkdir` 各 1 个是测试或探测文件。

C/C++ 部分用系统 C 工具链编译即可，但按 [cargo-sysroot](../../design/build/cargo-sysroot.md) 的要求在来源清单中单列。kfrc 侧的契约是：`cargo:rustc-link-lib`/`rustc-link-search` 正确传给链接器，`extern "C"` 声明的 ABI 与 C 侧一致（含 `#[repr(C)]` 结构体和可变参数函数**调用**）。

## SIMD、`target_feature` 与 `asm!`

```
# 合计：不同 x86 intrinsic 名称（全部 crate 取并集）：total llvm asm simd other
171 27 1 101 42
# 需要自实现的 LLVM intrinsic 名称（实现方式为 llvm 的 x86 函数）
_mm256_madd_epi16 _mm256_maddubs_epi16 _mm256_packus_epi16 _mm256_permutevar8x32_epi32 _mm256_sad_epu8 _mm256_shuffle_epi8 _mm256_zeroupper _mm512_clmulepi64_epi128 _mm512_madd_epi16 _mm512_maddubs_epi16 _mm512_permutexvar_epi32 _mm512_sad_epu8 _mm512_ternarylogic_epi32 _mm_clmulepi64_si128 _mm_packus_epi16 _mm_prefetch _mm_sha1msg1_epu32 _mm_sha1msg2_epu32 _mm_sha1nexte_epu32 _mm_sha1rnds4_epu32 _mm_sha256msg1_epu32 _mm_sha256msg2_epu32 _mm_sha256rnds2_epu32 _mm_shuffle_epi8 _rdrand32_step _rdrand64_step _xgetbv
```

用到 `core::arch` 的 crate 有 26 个，带 `#[target_feature]` 的 12 个：`memchr` 61、`aho-corasick` 60、`zlib-rs` 37、`blake3` 21、`ppv-lite86` 12、`bitmaps` 9、`getrandom`（0.3）9 等。27 个 LLVM 名按 cfg 人工核对后分三类：

- **x86_64 Linux 不编译的 8 个**：
  - `_mm512_*` 6 个，全部来自 zlib-rs 0.5.5。它的 AVX-512 代码在非默认 feature `avx512`/`vpclmulqdq` 之下，而且分派用的是编译期 `cfg!(target_feature = "avx512f")`，不是运行时检测。rand 0.9.2 引用的 `_mm512_mulhi_epu16` 等（simd 类）也在 `cfg(target_feature = "avx512bw")` 下。所以**默认目标（基线 x86_64）不生成任何 AVX-512 代码**；只有用 `-C target-cpu`/`target-feature` 打开 AVX-512 时才需要；
  - `_rdrand32/64_step` 2 个：getrandom 0.2/0.3 只在 SGX、`rdrand` feature 或 `getrandom_backend = "rdrand"` 时使用。
- **其余 19 个要自实现**（全是 legacy SSE/VEX 编码）：
  - SHA-NI 7 个（`_mm_sha1*`、`_mm_sha256*`，sha1/sha2 经 `cpufeatures` 运行时检测）；
  - PCLMUL（`_mm_clmulepi64_si128`，`crc32fast`、`zlib-rs`）；
  - AVX2 的 `madd`/`maddubs`/`sad`/`permutevar8x32`（zlib-rs 的 adler32）、`packus`、`shuffle_epi8`，以及 `_mm256_zeroupper`（ppv-lite86）；
  - SSE 的 `_mm_shuffle_epi8`、`_mm_packus_epi16`、`_mm_prefetch`，以及 `_xgetbv`。

`asm!` 逐文件核对（x86_64 Linux）：

| crate | 文件 | 次数 | 是否编译 | 形态 |
|---|---|---:|---|---|
| rustix 1.1.3 | `backend/linux_raw/arch/x86_64.rs` | 14 | 是（Linux 默认用 linux_raw 后端；features `alloc,default,fs,std,termios`） | `syscall` 指令，`rax`/`rdi`/`rsi`/`rdx`/`r10`/`r8`/`r9` 显式寄存器，`lateout("rcx")`/`lateout("r11")` 破坏，`options(nostack, preserves_flags)`（只读调用另加 `readonly`） |
| constant_time_eq 0.3.1 | `lib.rs` | 2 | 1 处（L10，x86/x86_64 分支） | 注释模板 `/* {0} */`，`inout(reg_byte)`，`options(pure, nomem, nostack, preserves_flags)` |
| zmij 1.0.8 | `lib.rs` | 4 | 1 处（L645，aarch64/x86_64 分支；其余 3 处在 aarch64 NEON 块内） | 注释模板 `/*{0}*/`，`inout(reg)` |
| getrandom 0.3.4 | `backends/linux_raw.rs` | 7 | 否（只在 `cfg(getrandom_backend = "linux_raw")` 时） | — |
| parking_lot 0.12.5 | `elision.rs` | 4 | 否（`hardware-lock-elision` feature 未开） | — |
| heapless 0.8.0 | `pool/treiber/llsc.rs` | 3 | 否（`arm_llsc` 只在 ARM 探针成功时设置） | — |
| rustix | `arch/x86.rs`、`vdso_wrappers.rs` | 22+1 | 否（32 位 x86） | — |

## 小结

Cargo 档在 ripgrep 档之上新增：过程宏（9 个宏 crate + `proc_macro` 原生编译 + bridge）、target 33 + host 2 个构建脚本（其中 16 + 2 个探测编译器）、7 个 C 库的链接与来源记账、系统调用级 `asm!`（显式寄存器与破坏列表）和 `reg_byte` 寄存器类，以及 19 个实际编译的 LLVM x86 intrinsic（SHA-NI、PCLMUL、AVX2，没有 EVEX）。默认构建不需要 async/协程，也不需要 AVX-512。

## 与 0.99.0 的差异

0.99.0 的数字来自同一套脚本，源码是 0.99.0 的检出；两份输出都在 walkthrough 草稿目录 `sysroot/out/` 下（0.95.0 在 `out/cargo-0.95.0/`）。

```
# 0.99.0：role 计数 / deps-scan.sh 合计 / x86 合计 / async-sites.sh 合计
   9 both   26 host  301 target
host crates=26 rs_lines=102769 unsafe=234 proc_macro=12 build_rs=3 links=0 sys=0 core_arch=1 target_feature=0 asm=1 repr_simd=0 feature_always=0 feature_cfg=5 rustc_probe=2 c_src=1 custom_cfg=4
target crates=310 rs_lines=2140217 unsafe=20550 proc_macro=0 build_rs=32 links=7 sys=8 core_arch=30 target_feature=16 asm=15 repr_simd=0 feature_always=0 feature_cfg=142 rustc_probe=15 c_src=10 custom_cfg=47
178 29 1 97 51
10 120 399
```

| 项目 | 0.99.0 | 0.95.0 | 原因 |
|---|---|---|---|
| 包数（target / host / 两侧） | 336（301 / 26 / 9） | 307（273 / 22 / 12） | 按包名比较，0.99.0 独有 26 个（`futures` 与 `futures-*` 10 个、`async-trait`、`rustversion`、`portable-atomic`、`cmov`、`chacha20`、`hybrid-array`、`ctutils`、`http`、`slab` 等），0.95.0 独有 5 个（`imara-diff`、`libz-rs-sys`、`ppv-lite86`、`rand_chacha`、`zerocopy`） |
| 过程宏 crate | 12 | 9 | 少了 `async-trait`、`futures-macro`、`rustversion` |
| 构建脚本（target / host） | 32 / 3 | 33 / 2 | target 侧少 `portable-atomic`，多 `typenum`、`zerocopy`；host 侧少 `rustversion` |
| 探测编译器（target / host） | 15 / 2 | 16 / 2 | — |
| `links` C 库 | 7 | 7 | 相同 |
| x86 函数（llvm） | 178（29） | 171（27） | 少 `_mm256_permutex2var_epi64`、`_mm512_dpbusd_epi32`、`_mm512_permutex2var_epi64`，多 `_mm256_zeroupper` |
| AVX-512 | 原文：zlib-rs 0.6.4 运行时检测分派，一定生成代码 | zlib-rs 0.5.5 在非默认 feature 下，不编译 | 见表后更正 |
| 含 `asm!` 的 target 侧 crate | 15 | 9 | 没有 `portable-atomic`（`cmpxchg16b`、`lock` 前缀、`cpuid`）、`cmov`、`zeroize` 的 `#` 模板、`constant_time_eq` 0.4 的 `xmm_reg` |
| async 使用行（crate / 文件） | 399（10 / 120） | 163（7 / 26） | 0.99.0 的 `cargo` 本身 18 个文件 151 行 async（`Source` trait 的 `query`/`download` 等改为 `async fn`，依赖 `futures`）；0.95.0 的全部被 cfg 关闭或由 `maybe-async` 改写 |

**对原 0.99.0 结论的更正：** 本次复核 zlib-rs 0.6.4 源码，它的 `adler32/avx512*`、`crc32/vpclmulqdq`、`compare256` 的 AVX-512 模块同样在非默认 feature `avx512`/`vpclmulqdq` 之下，而且分派用的是编译期 `cfg!`；0.99.0 闭包中 chacha20 0.10.1 的 AVX-512 后端要 `chacha20_avx512` cfg，它没有 build.rs，只能由 RUSTFLAGS 设置。所以原文“zlib-rs 的 AVX-512 一定会生成代码”不成立，0.99.0 默认构建也不需要 EVEX。两版的真实差别是 async：0.99.0 需要协程变换，0.95.0 不需要。
