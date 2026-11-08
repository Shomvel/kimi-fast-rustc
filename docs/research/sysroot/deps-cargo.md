# Cargo 0.99.0 依赖闭包的额外需求

方法同 [deps-ripgrep](deps-ripgrep.md)，包选 `cargo`（`-p cargo`），目标 `x86_64-unknown-linux-gnu`，默认 features。

## 规模

```
# closure-cargo.txt 的 role 计数
   9 both     26 host    301 target
# 合计（target 与 both 计入 target 侧；host 单列）
host   crates=26  rs_lines=102769  unsafe=234   proc_macro=12 build_rs=3  links=0 sys=0 core_arch=1  target_feature=0  asm=1  repr_simd=0 feature_cfg=5   rustc_probe=2  c_src=1  custom_cfg=4
target crates=310 rs_lines=2140217 unsafe=20550 proc_macro=0  build_rs=32 links=7 sys=8 core_arch=30 target_feature=16 asm=15 repr_simd=0 feature_cfg=142 rustc_probe=15 c_src=10 custom_cfg=47
```

`both` 指同一版本同时被 target 和 host 使用（`cfg-if`、`filetime`、`flate2`、`jobserver`、`libc`、`memchr`、`tar`、`unicode-ident`、`zlib-rs`）。它们要按 host 和 target 各编一次；目标相同的情况下也可以共享，但来源清单要分开记账。

## 过程宏与 host 图

12 个过程宏 crate：`async-trait`、`color-print-proc-macro`、`displaydoc`、`futures-macro`、`maybe-async`、`rustversion`、`serde_derive`、`thiserror-impl`、`tracing-attributes`、`yoke-derive`、`zerofrom-derive`、`zerovec-derive`。它们依赖 `proc-macro2`、`quote`、`syn`、`synstructure`、`nom`、`minimal-lexical`，以及构建期工具 `cc`、`pkg-config`、`vcpkg`、`autocfg`、`version_check`、`cfg_aliases`、`find-msvc-tools`、`shlex`。

要求：

- sysroot 里的 `proc_macro` crate（3606 行，打开 `negative_impls`、`decl_macro` 等 9 个 feature）要原生编译，作为 host 侧库；
- kfrc 实现过程宏 bridge 的服务端：TokenStream/Span/Literal 的句柄表，以及 `quote!` 内建过程宏；
- 用自己的协议把宏 dylib 或进程接起来（T30.b）；
- `rustversion` 自己有 build.rs，会调用 `RUSTC` 读版本；
- `async-trait`、`futures-macro` 生成 async 代码，所以 **Cargo 档需要 async/await 与协程变换**。M3 没有列这项（[plan-gaps](plan-gaps.md)）。

## 构建脚本、链接库与 C 依赖

- **构建脚本**：target 侧 32 个（`libc`、`anyhow`、`blake3`、`cargo`、`crc32fast`、`crossbeam-utils`、`curl`、`curl-sys`、`erased-serde`、`generic-array`、`getrandom`、`heapless`、`icu_*_data`、`im-rc`、6 个 `-sys`、`nix`、`num-traits`、`openssl-sys`、`parking_lot_core`、`portable-atomic`、`rustix`、`serde*`、`thiserror`、`typeid`、`zmij` 等），host 侧 3 个（`proc-macro2`、`quote`、`rustversion`）。探测编译器的有 target 侧 15 个、host 侧 2 个。
- **`links` 7 个**：`curl-sys(curl)`、`libgit2-sys(git2)`、`libnghttp2-sys(nghttp2)`、`libsqlite3-sys(sqlite3)`、`libssh2-sys(ssh2)`、`libz-sys(z)`、`openssl-sys(openssl)`。
- **`-sys` 8 个**：上面 7 个加 `linux-raw-sys`（纯 Rust 常量与类型，不链接 C）。
- **含 C/汇编源码的 crate**（`.c/.cc/.cpp/.S/.s` 文件数）：`curl-sys` 244、`libgit2-sys` 252、`libnghttp2-sys` 267、`libssh2-sys` 92、`libz-sys` 104、`blake3` 20（SIMD 的 `.S`/`.c`，默认通过 `cc` 编译）、`libsqlite3-sys` 3（`bundled` amalgamation）、`openssl-sys` 1（实际链接系统 OpenSSL，靠 pkg-config 找）。`cc`、`nix`、`walkdir` 各 1 个是测试或探测文件。

C/C++ 部分用系统 C 工具链编译即可，但按 [cargo-sysroot](../../design/build/cargo-sysroot.md) 的要求在来源清单中单列。kfrc 侧的契约是：`cargo:rustc-link-lib`/`rustc-link-search` 正确传给链接器，`extern "C"` 声明的 ABI 与 C 侧一致（含 `#[repr(C)]` 结构体和可变参数函数**调用**）。

## SIMD、`target_feature` 与 `asm!`

```
# 合计：不同 x86 intrinsic 名称（全部 crate 取并集）：total llvm asm simd other
178 29 1 97 51
```

用到 `core::arch` 的 crate 有 30 个，带 `#[target_feature]` 的 16 个：`memchr` 61、`aho-corasick` 60、`zlib-rs` 42、`chacha20` 30、`sha2` 25（0.11）、`blake3` 21 等。需要自实现的 29 个 LLVM 名包括：

- SHA-NI 7 个（`_mm_sha1*`、`_mm_sha256*`）；
- PCLMUL（`_mm_clmulepi64_si128`，`crc32fast`、`zlib-rs`）；
- AVX2 的 `madd`/`maddubs`/`sad`/`permutevar8x32`；
- AVX-512 的 `_mm512_ternarylogic_epi32`、`_mm512_permutex2var_epi64`、`_mm512_dpbusd_epi32`、`_mm512_clmulepi64_epi128`、`_mm512_sad_epu8` 等（zlib-rs 按运行时检测分派，函数非泛型，**一定会生成代码**）；
- `_mm_prefetch`、`_rdrand32/64_step`、`_xgetbv`。

`asm!` 逐文件核对（x86_64 Linux）：

| crate | 文件 | 次数 | 是否编译 | 形态 |
|---|---|---:|---|---|
| rustix 1.1.4 | `backend/linux_raw/arch/x86_64.rs` | 14 | 是（Linux 默认用 linux_raw 后端；features `fs,std,termios`） | `syscall` 指令，`rax`/`rdi`/`rsi`/`rdx`/`r10`/`r8`/`r9` 显式寄存器，`lateout("rcx")`/`lateout("r11")` 破坏，`options(nostack, preserves_flags)`（只读调用另加 `readonly`） |
| portable-atomic 1.13.1 | `imp/x86.rs`、`imp/atomic128/x86_64.rs`、`imp/detect/x86_64.rs` | 7+8+1 | 是 | `lock` 前缀指令、`cmpxchg16b`（运行时检测）、`cpuid` |
| constant_time_eq 0.4.2 | `sse2.rs` | 6 | 是（x86_64 + SSE2） | `pcmpeqb`/`vpcmpeqb`，`xmm_reg` 寄存器类 |
| cmov 0.5.4 | `backends/x86.rs` | 2 | 是 | 宏模板 `test {0}, {0}` + `cmovnz {1:e}, {2:e}`，`reg_byte` 寄存器类 |
| zeroize 1.9.0 | `barrier.rs` | 1 | 是 | `asm!("# {}", in(reg) ptr, options(readonly, preserves_flags, nostack))` |
| zmij 1.0.21 | `lib.rs` | 6 | 是（同 ripgrep） | 空模板屏障 |
| getrandom 0.4.3 | `backends/linux_raw.rs` | 7 | 否（只在 `cfg(getrandom_backend = "linux_raw")` 时） | — |
| parking_lot 0.12.5 | `elision.rs` | 4 | 否（`hardware-lock-elision` feature 未开） | — |
| rustix | `arch/x86.rs` | 22 | 否（32 位 x86） | — |

## 小结

Cargo 档在 ripgrep 档之上新增：过程宏（12 个宏 crate + `proc_macro` 原生编译 + bridge）、async/协程、target 32 + host 3 个构建脚本（其中 15 + 2 个探测编译器）、7 个 C 库的链接与来源记账、系统调用级 `asm!`（显式寄存器与破坏列表）、`cmpxchg16b`，以及 29 个 LLVM x86 intrinsic，其中 AVX-512 需要 EVEX 编码。
