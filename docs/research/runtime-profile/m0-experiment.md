# M0 归因实验设计

目的：在 M0 用 rustc+LLVM 自身量出“每缺一项后端优化，ripgrep/Cargo 慢多少”，作为 [T38](../../plan/m6-quality-performance.md#t38) 的排序依据，并冻结 runtime 验收用的 workload 与指标。本轮（2026-11-08）已经按此设计跑过一次，脚本可直接复用（[reproduce.md](reproduce.md)）；M0 要做的是在正式参考主机上重跑、补齐下面标“新增”的臂，并按冻结的判定标准出结论。归入 [M0 计划](../../plan/m0-baseline.md) 的 T00（固定输入）之后。

## 输入

| 项 | 规定 |
|---|---|
| 参考工具链 | rustc 1.94.1 / LLVM 21.1.8（与 T00 一致）；`rustc -Vv`、`perf --version` 入指纹 |
| 项目 | ripgrep 15.2.0（`e89fff89`）；Cargo 0.95.0（crates.io SHA-256 `0af45a75…c4d`，T00 已定版）；workload 输入也统一用 0.95.0 源码树 |
| profile | 项目自己的 release profile，`--target x86_64-unknown-linux-gnu`（RUSTFLAGS 不进 build script / proc macro） |
| 主机 | 每个项目一个独立 E2B 沙箱（8 vCPU、有 AVX2）；臂在同一沙箱内串行构建、交错测量；不同主机的绝对值不混用 |
| 语料 | 由 [setup-sandbox.sh](scripts/setup-sandbox.sh) 确定性生成，SHA-256 入指纹 |

## workload

用 [workloads.md](workloads.md) 的 22 个（rg 15、Cargo 7），覆盖：两个验收 workload；字面量（memmem）、Teddy、lazy DFA、稠密 DFA、反向后缀、Unicode；行号/上下文/JSON 输出；目录遍历与多线程；启动；Cargo 的 manifest 解析、带锁解析、从零解析、指纹、打包列表。新增建议：

- **held-out 组**：E1 中没进训练、冷函数降级后最差的 `-F -f` 多字面量与 `--json`，固定保留，用于以后验证分层/冷热决策。
- **Cargo 在线组**（可选）：git 依赖与 registry 更新，量 libgit2/curl 路径；本轮未覆盖。

## 臂

| 臂 | 状态 | 量什么 |
|---|---|---|
| A、V、U、R、J、I、G、H、L、O1、C、S2、S0 | 本轮已有 | 见 [attribution.md](attribution.md) |
| F（fast 分配器） | 本轮 SIGSEGV，**删除** | — |
| **Vm**（新增） | 只对 memchr 的 `count_byte_by_byte` 关向量化（`#[optimize]` 或补丁加 `-Cno-vectorize-loops` 仅作用于 memchr） | 确认 V 的代价只来自这一处 |
| **Ix**（新增） | I 臂，但 `core::arch` 包装强制内联（补丁 core_arch 加 `#[inline(always)]` 或用 `-Zbuild-std` 重编） | 把 #2“intrinsic 成为调用”从 #3“普通内联”里分出来 |
| **BS-A / BS-C**（新增） | A、C 加 `-Zbuild-std`（`RUSTC_BOOTSTRAP=1`） | 把 std 内非泛型代码也纳入 flag 范围，看上界是否变大 |
| **P-skip**（新增） | 按 pass 名跳过 `ipsccp`、`globalopt` 的插件 | 过程间常量的运行时价值（[llvm-mir/recommendations.md](../llvm-mir/recommendations.md) R1） |

每个臂必须先过 [check-llvm-args.sh](scripts/check-llvm-args.sh)：rustc exit 0（选项存在），探针或真实二进制上可见变化（remark、汇编特征或二进制哈希与 A 不同）；不满足的臂不进入测量。

## 指标

| 指标 | 用途 | 采集 |
|---|---|---|
| `instructions:u` 中位数比 | 主判定（本轮噪声 ≤0.5%） | `perf stat`，1 轮预热 + 5–30 轮，正反序交错 |
| `cycles:u` 中位数比 | 主判定的旁证；只在 A 臂 spread ≤5% 的 workload 上使用 | 同上 |
| 墙钟、sys/(user+sys) | 说明稀释程度，不单独判定 | `EPOCHREALTIME`、[kernel-share.sh](scripts/kernel-share.sh) |
| 热点归属 | 解释差距落在哪个函数 | [profile-hotspots.sh](scripts/profile-hotspots.sh) A 臂与 ≥1.02 的臂各采一次，[compare-profiles.sh](scripts/compare-profiles.sh) 对比 |
| 输出等价 | 前置条件 | 每臂每 workload 比对规范化输出；不一致的臂整组作废 |

## 判定标准（出数据前冻结）

| 结论 | 条件 |
|---|---|
| **必须做（进 T38 前段）** | 任一代表 workload 上 `instructions:u` ≥1.02，或 `cycles:u` ≥1.03 且该 workload A 臂 spread ≤2% |
| **可后做** | 所有 workload 上 `instructions:u` ≤1.01，且 `cycles:u` 都在 A 臂 spread 之内 |
| **待定** | 其余；加轮数或补 profile 再判，不放宽阈值 |
| 验收 workload 合格 | 一个 workload 只有在 S0 臂（无显式 SIMD）的墙钟比 ≥1.10 时才能单独承担 ≤1.0 判定；否则只能做冒烟 |

按本轮数据预判：S2/S0、I、L、V（仅输出类）、R 属于“必须做”；U 在 Cargo 上 1.023–1.024，也落在“必须做”的边缘；J、H 属于“可后做”；G 在 dir-files 上 1.012，按规则为“待定”。rg official 的 S0 墙钟比 1.013，不合格；Cargo metadata-nodeps 的 S0 墙钟比 0.991，不合格。

## 交付

- `results/<host>/<date>/`：原始 TSV、check 表、profile 文本、指纹；表格由脚本生成后原样粘贴。
- 一页结论：每臂“必须做 / 可后做 / 待定”，附代表 workload 的比值；更新 [design/goals/runtime.md](../../design/goals/runtime.md) 的风险列与 T38 顺序。
