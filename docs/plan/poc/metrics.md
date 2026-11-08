# 指标与测量协议

## 主张与门槛

| # | 主张 | 指标 | 门槛 | 对照 |
|---|---|---|---|---|
| 1 | 正确 | crate 测试通过数；W01–W10 输出 | 全部通过 | rustc 同配置构建 |
| 2 | 编译工作少 | 中后端 `instructions:u`：kfrc 的 Body 转换之后全部阶段 vs rustc 的 codegen 部分（MIR → LLVM IR、LLVM 优化、LLVM 后端） | ≥ 3× 更少 | rustc O3、16 CGU、ThinLocal |
| 3 | 运行不差 | 解析负载 runtime 比值 | 单侧 95% 置信上界 ≤ 1.10 | 同上 |
| 4 | 体积不差 | strip 后 `.text` 字节（只计 kfrc 编译的 crate 的片段） | ≤ 1.0 | 同上；另报 `opt-level=s` 与 lld `--icf=all` 两种基线 |
| 5 | 泛型只优化一次 | 每定义 R 级优化次数；实例数 / 特化身份键数 | 恰好 1；去重比 ≥ `-Zmono-dedup` | rustc + `-Zmono-dedup`（前期补丁） |
| 6 | 增量 | 编辑场景下机器码片段复用率；重编的定义数 | 行号移动 100% 命中；注释 0 重编；改一个非泛型函数体只重编它和内联了它的调用者 | rustc `-Cincremental` 同场景 |

主张 3 只覆盖 PoC 的负载（解析器类、无 SIMD），不外推到 ripgrep 整体：ripgrep 能否 ≤ 1.0 取决于显式 SIMD 与 intrinsic 内联（[runtime-profile](../../research/runtime-profile/README.md)），不在 PoC 范围。PoC 必须做到的是跨 crate 内联（经 R 图缓存拿到 std 和上游的泛型、`#[inline]` 函数体），这是 Cargo 解析类负载的主要因素。

门槛在 P0 开始前冻结。看到数据后不放宽；达不到的项如实报告并给出归因。

## 测量环境与方法

- 正式测量在 E2B 上做（按仓库 AGENTS.md 的约定），本机只做功能验证。每个配置一个独立沙箱，基线与候选在同一沙箱内串行对照。
- 编译工作用 `instructions:u` 计数（前期的 perfcount 方法，噪声约 0.004%），不只看 wall。wall 另报，标明核数。
- 中后端的边界：kfrc 从 Body 转换器的出口开始计；rustc 从 `codegen_crate` 开始计（`-Zself-profile` 的 codegen 相关事件）。前端部分两边相同，单独报告，不计入主张 2。
- 运行时间：先验证输出一致；多轮交错、随机臂顺序；保留全部原始样本。
- 两边使用同一份 `-Zbuild-std -Zmir-opt-level=0` 的 std（[architecture](architecture.md#接缝-3std-的未优化-mir)）；另报一组基线用默认 MIR 优化的数字作参考。

## 结构性指标（解释用，不设门槛）

| 指标 | 来源 |
|---|---|
| 各级节点数、每 pass 访问数与改动比例 | [optimization-pipeline](../../design/ir/optimization-pipeline.md#共同机制) |
| 字节/节点、每节点分配次数、存活率 | [levels](../../design/ir/levels.md) 风险表 |
| 中后端时间按级（R/S/L/后端）和按 LLVM pass 族分项 | [llvm-mir](../../research/llvm-mir/README.md) 的总表 |
| 被嵌入实例数、特化次数、内联次数 | [generics-specialization](../../design/ir/generics-specialization.md) |
| 拷贝与 alloca 的对应物：value slot 数、`CopyMem` 数 | [W01](../../research/walkthroughs/w01-try-error-path.md)、[W09](../../research/walkthroughs/w09-large-values.md) |

## 报告

每项主张单列：结论（达到 / 未达到 / 证据不足）、原始数据位置、归因。未达到的项给出它属于设计问题、实现未完成，还是 PoC 范围之外（例如没有向量化）。
