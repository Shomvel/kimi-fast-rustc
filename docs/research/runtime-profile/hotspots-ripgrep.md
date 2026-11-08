# ripgrep 热点画像

A 臂（stock release）用户态 cycles 采样，`perf record -e cycles:u -F 4999`，每个 workload 重复 10 次（短 workload 40 次），每份 942–34,437 个样本。[classify-profile.sh](scripts/classify-profile.sh) 按符号归类，内联进调用者的代码记在调用者上。只采用户态；内核占比见 [workloads.md](workloads.md)。

## 各 workload 的类别占比（% 用户态 cycles，`matrix 10` 原样输出）

| workload | memchr/memmem SIMD (core::arch) | Teddy / packed (core::arch) | regex DFA (dense/onepass) | regex lazy DFA | allocator | std/core other | regex meta + prefilter glue | searcher (lines, buffers) | regex compile/other | dir walk + ignore/glob | CLI parsing/help | printer | rest |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| ctx | 84.3 | 0.0 | 0.0 | 0.0 | 0.4 | 0.2 | 0.1 | 3.6 | 0.0 | 0.0 | 0.0 | 2.1 | 9.2 |
| dfa | 0.0 | 0.0 | 99.0 | 0.0 | 0.0 | 0.0 | 0.0 | 0.1 | 0.0 | 0.0 | 0.0 | 0.0 | 0.9 |
| dir-files | 0.3 | 0.0 | 2.9 | 0.0 | 22.7 | 24.0 | 0.0 | 0.0 | 12.0 | 14.5 | 0.0 | 0.0 | 23.6 |
| dir-mt | 10.0 | 0.0 | 0.9 | 10.6 | 15.4 | 7.2 | 7.7 | 3.7 | 4.6 | 5.8 | 0.0 | 4.5 | 29.6 |
| dir-regex | 17.4 | 0.0 | 1.5 | 3.9 | 13.7 | 10.9 | 4.1 | 3.4 | 7.3 | 8.0 | 0.0 | 2.0 | 27.7 |
| help | 0.0 | 0.0 | 0.0 | 0.0 | 5.7 | 22.3 | 0.0 | 0.0 | 0.0 | 0.0 | 28.7 | 0.0 | 43.3 |
| json | 81.8 | 0.0 | 0.0 | 0.0 | 0.4 | 1.9 | 0.2 | 0.5 | 0.0 | 0.0 | 0.0 | 0.1 | 15.1 |
| lines | 44.3 | 0.0 | 0.0 | 0.0 | 0.0 | 2.9 | 4.5 | 22.5 | 0.0 | 0.0 | 0.0 | 12.6 | 13.2 |
| lit-i | 0.0 | 91.5 | 0.8 | 0.0 | 0.5 | 0.0 | 0.3 | 0.0 | 0.2 | 0.0 | 0.0 | 0.0 | 6.6 |
| lit | 82.3 | 0.0 | 0.0 | 0.0 | 0.7 | 0.0 | 0.9 | 1.2 | 0.0 | 0.2 | 0.0 | 0.0 | 14.6 |
| lit40 | 1.3 | 72.9 | 15.7 | 0.0 | 0.0 | 0.1 | 4.8 | 1.0 | 2.1 | 0.0 | 0.0 | 0.0 | 2.0 |
| official | 8.4 | 0.0 | 2.8 | 0.1 | 17.9 | 7.1 | 2.2 | 1.3 | 7.1 | 8.1 | 0.0 | 1.2 | 43.7 |
| suffix | 60.0 | 0.0 | 9.8 | 0.0 | 0.4 | 0.0 | 16.3 | 1.5 | 0.0 | 0.1 | 0.0 | 0.0 | 11.9 |
| unicode | 57.1 | 0.0 | 5.6 | 0.0 | 1.0 | 0.0 | 10.7 | 4.2 | 2.9 | 0.0 | 0.0 | 0.0 | 18.4 |
| word500 | 10.5 | 0.0 | 0.0 | 68.6 | 0.0 | 0.0 | 8.5 | 10.2 | 1.3 | 0.0 | 0.0 | 0.0 | 0.8 |

`rest` 含内核入口的采样偏移（official 6.8%）、libc mem*（official 6.5%）和长尾。“allocator” 是 glibc malloc/free。

## 按搜索模式的热路径

| 模式 | 热点符号（占用户态 cycles） | 机制 |
|---|---|---|
| 单个字面量（lit、suffix、unicode 的前过滤） | `memchr::arch::x86_64::avx2::packedpair::Finder::find_impl` 82.01% / 58.28% / 52.84% | memmem 的 AVX2 字节对前过滤，`core::arch` 运行时分派 |
| 少量字面量（lit-i、lit40） | `aho_corasick::packed::teddy::…::SlimAVX2<4>` 91.52%；`FatAVX2<4>` 72.89% | Teddy，AVX2 `pshufb` 系列 |
| 多字面量（word500） | `regex_automata::hybrid::search::find_fwd` 68.50% | 500 个字面量时选了 lazy DFA，不是 Aho-Corasick |
| 无字面量正则（dfa） | `regex_automata::dfa::search::find_fwd` 98.97% | 稠密 DFA 查表循环 |
| 后缀字面量（suffix） | memmem 58.28% + `meta::limited::dfa_try_search_half_rev` 9.27% | 前过滤 + 反向 DFA |
| 行号/上下文输出（lines、ctx、json） | `memchr::arch::x86_64::memchr::count_raw::find_avx2` 20.86% / 26.67% / 27.18% | 数换行，见下 |
| 目录遍历（dir-files） | `_int_malloc` 8.48%、`Path::Components::next_back` 7.13%、`determinize::next` 5.54%、`Ignore::matched_dir_entry` 5.00% | 路径拼接与分配、gitignore 的 glob→DFA 构建与匹配 |
| 启动（help） | `textwrap::core::break_words` 16.35%、`str::replace` 7.58% + 6.44% | 文本排版、字符串处理 |

用户态之外：目录 workload 的 sys 占 63–78%（目录读取、stat、open/read 等系统调用）。

## 运行时分派的实际路径

| 证据 | 结论 |
|---|---|
| A 臂符号是 `…::avx2::…`、`SlimAVX2`/`FatAVX2` | 本机型（有 AVX2）走 AVX2 分支 |
| S2 臂 lit-i 热点变成 `SlimSSSE3<4>`（94.80%，cycles 1.749×） | 分支可以只改检测结果而切换，比较时双方必须在同一分支 |
| S0 臂 lit-i 热点变成 `aho_corasick::automaton::try_find_fwd`（69.44%）+ `memchr2_raw::find_fallback`；lit 变成 `memchr_raw::find_fallback`（85.09%） | 没有显式 SIMD 时的退路是非 Teddy 自动机和 SWAR |

热路径用到的 intrinsic（[intrinsic-census.sh](scripts/intrinsic-census.sh)，不含测试与注释）：memchr 2.8.3 共 14 个（AVX2：`_mm256_{and,or}_si256`、`cmpeq_epi8`、`load(u)_si256`、`movemask_epi8`、`set1_epi8`；SSE2 对应 7 个）；aho-corasick 1.1.4 共 26 个（AVX2 16 个、SSSE3/SSE2 10 个，比 memchr 多 `alignr`、`shuffle_epi8`、`permute2x128`、`permute4x64`、`broadcastsi128`、`srli_epi16`、`unpack{hi,lo}`、`extract_epi8/64`）；hashbrown 的 SSE2 组 9 个（编译期 cfg，不是运行时分派）。全量清单见 [sysroot/llvm-asm-arch.md](../sysroot/llvm-asm-arch.md)。

## 两个意外

**1. 自动向量化只影响 memchr 的首尾循环。** V 臂中 ctx 的 `count_raw::find_avx2` 从 26.67% 涨到 38.15%（按 cycles 比 1.181 折算成 A 的 45.06%，+18.39 个百分点；[compare-profiles.sh](scripts/compare-profiles.sh)）。memchr 的计数主循环是手写 SIMD，但对齐前后不足 32 字节的部分走 `count_byte_by_byte`（逐字节 `if *p == b { n += 1 }`），这个循环靠 LLVM 在 `#[target_feature(enable = "avx2")]` 函数里自动向量化。行号输出时每次匹配都要从上次位置数到当前位置，片段短，首尾循环占比就大。

**2. I/L 臂的 rg 代价主要是 intrinsic 没被内联。** dir-regex 在 I 臂（cycles 1.392）中出现独立函数 `core::core_arch::x86::avx2::_mm256_movemask_epi8`，占 21.90%（折算 +30.48 个百分点）；L 臂（1.381）同样 21.66%（+29.91）。各 workload 的这类“intrinsic 被当成函数调用”的占比：

| 臂 | dir-regex | dir-mt | lines | official | unicode | word500 | 其余 rg workload |
|---|---:|---:|---:|---:|---:|---:|---:|
| I | 21.9% | 14.2% | 8.8% | 7.1% | 4.7% | 3.3% | ≤1.0% |
| L | 21.7% | 15.0% | 9.1% | 7.3% | 4.8% | 3.9% | ≤1.0% |

这与前期 Cranelift 的 SHA 诊断是同一机制（intrinsic 包装调用 57.82× → force-soft 4.39×）：`core::arch` 函数是带 `#[target_feature]` 的普通 `#[inline]` 函数，只要内联失败（阈值不够、跨 CGU、调用者没有同样的 target feature），每条向量指令都会变成一次调用。
