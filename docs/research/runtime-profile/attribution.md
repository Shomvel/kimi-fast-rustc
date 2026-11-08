# 归因实验：设计与 flag 核验

问题：rustc+LLVM O3 中，每一项优化对 ripgrep/Cargo 的 runtime 贡献多少？做法是一次只去掉一项，量 runtime 上界。结果表见 [attribution-results.md](attribution-results.md)，复现步骤见 [reproduce.md](reproduce.md)。

## 环境

| 项 | 值 |
|---|---|
| 机器 | HK E2B（0820 服务）模板 `flasheda-r5-cpu8-t16-20261030`，8 vCPU AMD EPYC，16 GiB，内核 6.1.158；CPU 有 SSE2/SSSE3/SSE4.2/AVX2/BMI2/AVX-512F |
| 工具链 | rustc 1.94.1（commit `e408947b`），LLVM 21.1.8，cargo 1.94.1；perf 6.8.12，`perf_event_paranoid=2`（只能采用户态） |
| ripgrep | 15.2.0，commit `e89fff89`，`Cargo.lock` SHA-256 `7a7d39cd…a64`，release profile（`debug=1`） |
| Cargo | **0.95.0**（crates.io 包，SHA-256 `0af45a75…c4d`），默认 release profile |
| 隔离 | rg 与 Cargo 各一个沙箱，臂在同一沙箱内串行构建、交错测量 |

Cargo 没用 0.99.0：它的 `cargo` 包 `rust-version = "1.96"`，依赖 libsqlite3-sys 0.38.1 的 build script 用了 1.94 还不稳定的 `cfg_select!`（E0658），rustc 1.94.1 编不过。0.95.0 是 1.94 发行版自带的 Cargo，也是 2026-10 各轮用的版本。Cargo 的 workload 输入仍是 0.99.0 源码树（resolver workload 除外，见 [workloads.md](workloads.md)）。

## 臂

全部用 `cargo build --release --offline --target x86_64-unknown-linux-gnu`，RUSTFLAGS 只作用于目标产物（build script 与 proc macro 在 O0 下编，`-regalloc=basic` 在 O0 会被 LLVM 拒绝）。定义在 [arm-flags.sh](scripts/arm-flags.sh)。

| 臂 | 去掉什么 | flag |
|---|---|---|
| A | 基线（stock release） | — |
| V | 自动向量化（循环 + SLP） | `-Cno-vectorize-loops -Cno-vectorize-slp` |
| U | 循环展开与 peeling | `-unroll-count=1 -unroll-allow-peeling=false` |
| R | greedy 寄存器分配 → basic（Olesen 称接近线性扫描） | `-regalloc=basic -optimize-regalloc` |
| F | → fast（局部单遍） | `-regalloc=fast` |
| J | 跳转表 | `-min-jump-table-entries=100000` |
| I | LLVM 内联只剩“免费”调用（MIR 内联不变） | `-inline-threshold=0` |
| G | GVN 的 load 消除与 PRE（LLVM 21.1.8 没有 `-opt-disable`） | `-enable-gvn-memdep=false -enable-load-pre=false -enable-pre=false` |
| H | LICM 的标量提升（外提本身保留） | `-disable-licm-promotion` |
| L | crate 内跨 CGU 内联（ThinLocal） | `-Clto=off` |
| O1 | 整条流水线换成 O1 | `-Copt-level=1` |
| C | V+U+R+J 叠加：首版自写后端预计缺的几项 | 上面四组合并 |
| S2 | 显式 SIMD 的 AVX2 分支（运行时检测改答“无”） | 补丁 memchr、aho-corasick |
| S0 | 全部显式 SIMD（AVX2/SSSE3/SSE2 都答“无”，走 SWAR / Two-Way / 非 Teddy） | 同上 |

S 臂由 [make-simd-patches.sh](scripts/make-simd-patches.sh) 只改 `is_x86_feature_detected!` 与 SSE2 `is_available()`，算法不动；用 `--config patch.crates-io.*` 打进副本。rg 用 memchr 2.8.3，Cargo 用 2.7.6；aho-corasick 都是 1.1.4。`rg --version` 的 `simd(runtime)` 在三臂都报 `+SSE2,+SSSE3,+AVX2`，因为它报告 CPU 能力而不是 memchr 实际走的分支；实际分支以 profile 中的符号为准（[hotspots-ripgrep.md](hotspots-ripgrep.md)）。

## flag 是否真的生效

[check-llvm-args.sh](scripts/check-llvm-args.sh) 在探针 crate 上逐臂编译。未知的 `-Cllvm-args` 会让 rustc 退出 1，所以 exit 0 证明选项存在；各列是 `-Cremark=all` 的成功 remark 计数、跳转表标签数、`-debug-pass=Structure` 报的分配器、`call` 条数：

```
arm	exit	loop-vectorize	slp-vectorizer	loop-unroll	inline	gvn	licm	jt	ra	calls
A	0	3	0	3	10	0	4	1	Greedy	1
V	0	0	0	5	10	0	4	1	Greedy	1
U	0	3	0	0	10	0	4	1	Greedy	1
R	0	3	0	3	10	0	4	1	Basic	1
F	0	3	0	3	10	0	4	1	Fast	1
J	0	3	0	3	10	0	4	0	Greedy	1
I	0	3	0	3	10	0	4	1	Greedy	1
G	0	3	0	3	10	0	4	1	Greedy	1
H	0	3	0	3	10	1	1	1	Greedy	1
L	0	3	0	3	10	0	4	1	Greedy	1
O1	0	0	0	6	45	0	3	1	Greedy	1
C	0	0	0	0	10	0	4	0	Basic	1
```

- V、U、R、F、J、H、C 在探针上可见效果；U 下 remark 变成 “unrolled loop by a factor of 1”（不计入）。
- I、G、L 在探针上看不出：探针里只有负代价内联、没有 GVN 能消的 load、只有一个 crate。它们在真实构建中生效的证据是：每个臂的 rg/Cargo 二进制 SHA-256 都与 A 不同，体积也变了（[reproduce.md](reproduce.md)），且 runtime 指令数变化远超噪声。
- `-opt-disable=gvn|licm` 在 LLVM 21.1.8 下报 “Unknown command line argument”，所以 G/H 只能关子功能。
- **F 臂不可用**：`-regalloc=fast` 配 O3 产出的 rg 与 Cargo 在所有 workload 上都 SIGSEGV（exit 139），从结果中排除。

## 指标

| 指标 | 来源 | 噪声（A 臂 spread = (max−min)/median） |
|---|---|---|
| `instructions:u` | `perf stat`，整棵进程树 | rg ≤0.5%，Cargo ≤0.1% |
| `cycles:u` | 同上 | rg 百 ms 级 workload 0.4–2.3%，Cargo 2.6–10.4%；几十 ms 以下的 6.6–69.5% |
| 墙钟 | bash `EPOCHREALTIME` | 同 cycles，另含内核时间 |
| user/sys | `/usr/bin/time`，每个 workload 连跑 20 次取平均 | 10 ms 粒度 |

每个 workload 先逐臂比对输出（多线程输出排序后比、`--json` 去掉耗时字段），再 1 轮预热 + 5 轮（短 workload 10–30 轮，见 [workloads.sh](scripts/workloads.sh)）交错测量，轮间正反序交替。除 F 外 13 个臂在 22 个 workload 上输出全部与 A 一致。

## 局限

- 预编译的 std 在所有臂都是 LLVM O3。泛型的 std 代码（HashMap、迭代器、`core::arch` 包装）在用户 crate 里单态化，受 flag 影响；非泛型的 std 函数（`core::slice::memchr`、`fmt`）和 glibc 不受影响。
- 每个臂是“LLVM 去掉一项”，不是“kfrc 缺这一项”：kfrc 的中层优化可能补回一部分，也可能缺得更多。数字是上界性质的归因，不是 kfrc 的预测。
- 单台 E2B 宿主；绝对时间不与其他机器混用。
