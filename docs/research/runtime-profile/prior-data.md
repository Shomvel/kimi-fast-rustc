# 前期 runtime 数据汇总

本页只整理 2026-10/11 已有的 runtime 测量，不含本轮新测（新测见 [attribution.md](attribution.md)）。“指令比”是 `instructions:u`（或含内核的 instructions retired）的中位数之比，“时间比”是墙钟中位数之比，都以各轮自己的基线为 1。

## 机器与 workload

| 代号 | 机器 | 工具链 | 计数 | 用在 |
|---|---|---|---|---|
| M-mac | 本机 macOS aarch64，M 系列 12 核，重负载（load 140–490） | Homebrew rustc 1.94.1 / LLVM 21.1.8 | `time -l` instructions retired（含内核） | E1 冷函数实验 |
| M-e2b | HK E2B，Linux x86_64，8 vCPU / 16 GiB | stage1 rustc 1.94.1 / LLVM 21 | perf `instructions:u`、`cycles:u` | mirx 各轮 |
| M-e2b-n | 同上 | nightly-2026-03-25（1.96 前端）/ LLVM 22 | 墙钟中位数 | Cranelift 对照 |
| M-yi | yicloud-dev，Linux x86_64，4 CPU | Rust 1.99.1 | 墙钟中位数，15–30 次交替 | kfrc VALIDATION（LLVM candidate） |

| workload 组 | 内容 | 出处 |
|---|---|---|
| E1-F | rg 单文件 312 MB（rustc 源码拼 4 份，mmap 路径）：F1–F4 与训练同模式，F5–F10 新模式（F6 `-F -f` 39 个字面量、F8 `--json`、F9 `-n -C2`） | rustc 10× 重写分析（2026-10-20）§5 |
| T/H | rg 目录搜索 87 MB rustc 源码；T1–T5 是 PGO 训练集 | 同上 |
| mirx-rg | 目录语料 registry + `library/`：regex、literal-ignorecase、word-class、printer-sorted、files、json、many-patterns、multithread、help、version | mirx `bench_runtime.py` 与各轮 workloads |
| mirx-cargo | cargo 0.95.0 源码：metadata、tree、noop-build、version、build-help | 同上 |
| clif | rg：source-search、64 MiB 的 literal/regex/unicode/many-patterns、help；cargo：metadata、tree、package-list、help；库基准 json/regex/gzip/sha256 | Cranelift 对照报告（2026-10-09） |
| kfrc | rg `-n --no-messages 'fn\s+main'` 搜 ripgrep 源码树；cargo `metadata --no-deps` | kfrc `docs/VALIDATION.md`（commit 138723c） |

## 各实验对 runtime 的影响

| 实验（相对谁） | 机器 | 影响 | 来源 |
|---|---|---|---|
| PGO（`-Cprofile-use`，B1/B0） | M-mac | E1-F 指令比 0.944–1.011 | 重写分析 §5、coldexp `summary.txt` |
| PGO 冷函数加 `optnone`+`noinline`（B2/B1） | M-mac | 同模式 F1–F4 1.018–1.036；新正则 1.032–1.047；`--json` 1.174；`-n -C2` 1.182；**`-F -f` 39 字面量 2.037**（user 0.93→1.33 s） | 同上 |
| 冷代码规模 | M-mac | 链接进 B1 的机器码 53% 在训练中从未执行；构建出的机器码 22.2% 被 dead-strip | 同上 |
| `-Clto=off`（O 臂，建在 B 臂上，相对 A） | M-e2b | rg 指令 +1.50%…+27.83%，cycles 0.972…1.448；cargo 指令 +5.79%…+8.71%，cycles 1.036…1.190 | thinlto-postlink 笔记 §4；[prior-runtime-table.sh](scripts/prior-runtime-table.sh) |
| ThinLocal 相对 lto=off（clif 报告的补充组） | M-e2b-n | rg literal 0.74×、regex 0.83×、unicode 0.82×、many-patterns 0.87×；cargo metadata-full 0.90×、tree 0.94× | Cranelift 对照报告 |
| Cranelift 替换 LLVM（release，双方 lto=off） | M-e2b-n | rg 搜索 4.06–14.63×，help 3.14×；cargo metadata-full 4.07×、tree 3.31×、metadata-no-deps 1.49×；sha256 57.82×（force-soft 4.39×）、gzip 15.23×、regex-scan 13.99×、json-parse 3.50× | 同上 |
| post-link 只化简调用导入函数者（Q） | M-e2b | rg 搜索 +0.2…+0.5 个百分点（相对 B）；cargo 持平 | thinlto-postlink 笔记 §4–6 |
| `-Zmono-inline=30`（K） | M-e2b | rg help +3.87%、many-patterns −2.69%；cargo −0.6% 量级 | 同上 §6 |
| 导入上限 50（T） | M-e2b | cargo +0.37%…+0.56%，rg help +1.95% | 同上 §4 |
| `-Zshare-generics=y`（S） | M-e2b | rg 搜索 +3.1–7.5%，help +10.67%；cargo metadata/tree/no-op +4.7–6.2% | monoexp 第二轮笔记 §3 |
| coldopt（rg `flags::` 降优化） | M-e2b | help +5.2%（coldsize）/ +8.3%（none）；version +1.6% / +11.3% | 同上 §4 |
| drop glue 共享（L） | M-e2b | cargo 5 个负载 ±0.13% 以内 | 同上 §2 |
| LLVM candidate（ThinLTO、CGU1）对 stock | M-yi | rg 0.931（4.322 vs 4.640 ms），cargo 0.988（47.634 vs 48.198 ms）；早一轮 rg 0.985；30 次的较大搜索 1.00415 | kfrc VALIDATION |
| 同一二进制的“controlled”臂 | M-yi | rg 1.007、cargo 0.997（SHA-256 相同，纯噪声） | 同上 |

## 读法

- **跨 CGU 内联是 LLVM 侧最大的单项。** 关掉它（lto=off）rg 慢到 1.28×（指令），cycles 到 1.45×；所有“少内联一点”的手段（K、T、share-generics、coldopt）都先在 help/启动和小函数密集的路径上显出代价。
- **profile 覆盖不到的路径是最大风险。** E1 中 `-F -f` 没进训练集，冷函数降级后慢 2.04×，而同模式负载只慢 2–4%。
- **完整替换后端的代价是数量级的。** Cranelift 在 rg 字面量/多模式上 13–15×，主要可确认的机制是 SIMD intrinsic 包装调用（SHA 消融 57.82× → 4.39×），其余负载的原因前期未查明。
- **kfrc 验收 workload 太短。** rg 约 4.5 ms、cargo 约 48 ms，同一二进制的比值就有 0.7% 的抖动；VALIDATION 的较大搜索在 1.00415，不满足严格的 ≤1.0。见 [workloads.md](workloads.md)。
- 前期报告 `hotspot-optimization/`、`trace-hotspots-report.md`、`thinlto-report.md` 只有编译侧数据；thinlto-report 明确写了没做 Cargo 运行性能基准。
