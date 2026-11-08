# 前端在关键路径上的位置

问题：前端占 CPU 不到三成，为什么墙钟上它常常是主角？答案在“rmeta 之前串行、之后并行”。rustc 各步骤在时间轴上的位置见 [llvm-mir/timeline-rustc.md](../llvm-mir/timeline-rustc.md)，这里只算前端那一段。

## 1. 结构（源码核实）

| 段 | 内容 | 并行性 | 下游何时能开工 |
|---|---|---|---|
| 库 crate：rmeta 之前 | 解析、展开、解析名字、HIR、typeck、borrowck、MIR 构建与**优化**、单态化收集（导出泛型）、写 rmeta | 默认 `-Zthreads=1`，全段单线程 | 写完 rmeta（Cargo 流水线） |
| 库 crate：rmeta 之后 | IR 生成、LLVM（按 CGU 并行） | 多线程 | 二进制 crate 等 rlib |
| 二进制 crate | 前端 + IR 生成串行，LLVM 并行 | 同上 | — |

metadata 先于 codegen 编码（[passes.rs#L1253](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_interface/src/passes.rs#L1253)），且要求所有可导出泛型/可内联项先算出 `optimized_mir`（[encoder.rs#L1099](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L1099)）。所以 rustc 的“前端”在关键路径意义上包含 MIR 优化。

## 2. 实测：release 关键路径（E2B 8 vCPU，官方 1.94.1，带 self-profile）

`scripts/cargo-critical-path.sh` 从 Cargo `--timings` 的单元数据回溯关键路径（前期 2026-10-10 运行的原始数据）。二进制单元在 1.94 没有 frontend/codegen 分段，记为“未拆分”。

ripgrep（构建总长 11.33 s）：

| 单元 | 前端 | codegen | 未拆分 |
|---|---:|---:|---:|
| memchr | 0.85 | 0 | 0 |
| aho-corasick | 0.8 | 0 | 0 |
| regex-automata | 2.26 | 0 | 0 |
| bstr | 0.39 | 0 | 0 |
| globset | 0.28 | 0 | 0 |
| ignore | 0.53 | 1.78 | 0 |
| ripgrep bin "rg" | 0 | 0 | 3.42 |
| **合计** | 5.11 | 1.78 | 3.42 |

等待间隙 0.82 s。Cargo（构建总长 105.31 s）：关键路径上前端 18.29 s（cargo 库 17.36 s）、codegen 27.64 s、构建脚本运行 35.22 s（libsqlite3-sys）、未拆分 4.33 s、等待间隙 14.33 s。

| 观察 | 数字 | 来源 |
|---|---|---|
| rg：库前端链占关键路径 | 5.11 / 11.33 s = 45%，另加 rg 自身的串行前端 | 上表（实测） |
| Cargo：cargo 库 rmeta 前单线程 | 前端墙钟 17.36 s；cargo crate（lib+bin）前端 CPU 15.7 s | 上表 + `phase-breakdown.sh … cargo`（实测） |
| stage1 下 cargo 库前端期间只有 1.46 个 CPU 忙 | 前端段 36.2 s | 前期 next-directions §4.2 |
| `-Zthreads=4` 填空窗 | cargo 关键路径前端段 36.2 → 21.1 s；墙钟 rg −6.2%、cargo −9.8%，产物逐字节相同 | 前期 next-directions §4.1、§4.2 |

check 构建的关键路径全部是前端；dev 与增量见 [time-breakdown.md](time-breakdown.md)。

## 3. 前期流水线实验：接口提前能换来多少

| 实验 | 结果 | 来源 |
|---|---|---|
| P1：bin 在上游 rmeta 齐备后启动 | rg 墙钟 −8.4% / −9.8% / −9.6%（三轮）；cargo −3.1% | 前期 pipelined-p1 |
| P2a：rmeta 分两段 | 库元数据阶段的 90–98% 是可后移的 MIR 选择、optimized MIR、导出符号；链上元数据阶段约 2.1 s → 约 0.14 s（模型） | 前期 p2a §2 |
| P2b M0：签名就绪时刻 | regex_automata 0.556 s（rmeta 4.210 s，13%）；cargo 1.561 s（15.186 s，10%）；各库 10–17% | p2b-m0 |
| P2b 模型（CPU 不限） | rg 15.43 → 11.04 s（−28.5%）；cargo −8.8% | p2b-m0 |
| P2b 实测（8 vCPU） | rg −3.3%，cargo 无变化；8 vCPU 下 rg CPU 地板约 13 s，P1 后只剩约 3 s 调度空间 | p2b-m2-m3 |
| headstart（外部，16 核） | `cargo build` 最多 −42%，`cargo check` 最多 −54%；4 核时收益明显变小 | [prior-art/ct-parallel-pipeline.md](../prior-art/ct-parallel-pipeline.md) |

## 4. 结论

1. **rustc 的前端是关键路径的串行段**：库的 rmeta 链（rg 45%）加 bin 自身前端；Cargo 的 cargo 库前端约 17 s 墙钟几乎全部单线程。
2. **签名比 rmeta 早得多**（10–17%）。把接口与函数体分开发布，链长可以缩到“各库签名就绪 + 最长单库函数体”，但只在 CPU 有富余时兑现（8 vCPU 实测 −3.3%，模型 −28.5%）。
3. **因此 kfrc 前端要同时降两样东西**：串行段长度（接口先发、库内按 typeck root 并行）和前端总 CPU（8 核时 CPU 地板就是上限）。只做其一，P2b 的教训会重演。
4. **MIR 优化不该在接口之前。** kfrc 的 R 级优化在接口发布之后（[goals/compile-time.md](../../design/goals/compile-time.md) C4），对应 rustc 前端 CPU 的 15–22%（[time-breakdown.md](time-breakdown.md)）直接移出串行段。

指标建议：关键路径构成里单列“接口就绪时刻 / 分析结束 / R 图就绪”，与 p2b-m0 的签名就绪表同口径对比（[recommendations.md](recommendations.md)）。
