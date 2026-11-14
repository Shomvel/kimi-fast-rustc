# 阶段

估计为单人工作量（推断）。每个阶段结束都有可运行的产物；P3 是第一个“真实 crate 通过”的里程碑，也是止损点。

2026-11-09：以下是目标阶段，不是当前完成状态。按
[semantic-ir-migration.md](semantic-ir-migration.md) 先修正值语义和布局接缝，
再完成定义级 R/S；这是实施顺序调整，不降低任何阶段的内容与出口。
旧链 semver 34/34 仅作正确性对照，P3 仍需前置语义与优化交付、新链测试及实测。

| 阶段 | 人周 | 内容 | 出口 |
|---|---:|---|---|
| P0 外壳与标量链 | 3 | 外壳、`TypeOracle` 的 tcx 实现、MIR → Body 转换器（标量、控制流）、R 级 builder、verifier、规则 DSL 首版、小解释器；接现有 x86 后端（标量） | W01–W10 中纯标量部分经 cargo 编译、链接 rustc std、运行结果一致；cargo 按包选后端已核实 |
| P1 聚合、enum、drop | 3 | 值形式聚合、变体操作、niche 推迟到 L 级、`DropValue`/`DropInPlace`、`Local`/`Const` 分区、R 级规则（聚合折叠、按前驱拆分、槽位提升、load 转发、检查消除） | W01、W03、W04、W09 通过；R 图 dump 中 `?` 错误路径无拷贝链 |
| P2 泛型 | 2 | 孔位表、特化身份键、递归去重、嵌入实例、S 级种子化简与内联、调用摘要（含 SCC）、mentioned items；`-Zbuild-std -Zmir-opt-level=0` 的 std 构建 | W02、W06 通过；去重比与 `-Zmono-dedup` 对比；每个定义 R 级优化恰好 1 次 |
| P3 L 级与真实 crate | 3–4 | L 级降低（PassMode、value slot、原地构造、switch 降低、常量除法、elaboration placement）、Rust ABI 全部 PassMode、只读数据与重定位、线性扫描扩展 | **semver 的测试全部通过**；在 E2B 上首次测量主张 2–4 |
| P4 主负载与缓存 | 2–3 | regex-syntax 通过测试；build 级 R 图与机器码缓存、源位置隔离；按 [metrics](metrics.md) 做全部测量；报告 | 六项主张逐项给出结论 |
| 合计 | 13–15 | | |

## 止损与调整

- **P3 出口**：若 semver 上的中后端指令数比不到 2×，或 runtime 比值上界 > 1.3，先做归因（哪一级、哪类节点、哪个 pass），给出“继续 / 调整设计 / 停止”的建议，再进入 P4。
- **范围调整建议顺序**：先放弃扩展目标（`dyn`、`Frozen`/`Unique`、toml）→ 再把 regex-syntax 中不支持的模块所在 crate 交给 LLVM 并标注覆盖率 → 最后才提出缩小主张。当前仍执行 [scope.md](scope.md) 的完整中后端范围；部分覆盖不计为该 crate 全量通过，主张和门槛不随测量结果自动放宽。
- **同类错误**：首次暴露一类错误后停止重复跑，静态排查同类实现，批量补回归再继续（总览红线 6）。

## 每阶段都要有的

- 一个命令从干净目录重放：构建 std、构建负载、跑测试、出指标表。
- R/S/L 三级 dump 与 rustc 的 MIR/IR/汇编对照，按 [walkthrough](../../research/walkthroughs/README.md) 的格式。
- 数字由脚本生成，原样粘贴。
