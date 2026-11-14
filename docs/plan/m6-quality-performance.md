# M6：机器码质量与编译性能

**目标：** 在同等语义和配置下，相对固定版本 rustc+LLVM：运行时间和产物体积不退化（比值 ≤ 1.0），完整编译时间改善。

**依赖：** M5 的正确性出口。测量基础从 M0 起就存在。

**出口：**

- 按 [性能比较协议](README.md#验收) 分别报告 cold、no-op、局部修改、接口修改、缓存 hit/miss。
- 常规 portable release 和同约束下各自最佳配置分别有数据。
- 每个代表 workload 的 runtime/体积比值 ≤ 1.0；置信上界跨过 1 时记“证据不足”。
- 编译收益可重复，确实来自减少工作或缩短关键路径。

**约束：** 需要 LTO 的配置使用自有图/摘要机制，不加载 LLVM bitcode。跨 crate 调度引入内存峰值、循环等待或失效错误时，回退到上一已验证粒度。

---

<a id="t38"></a>
## T38 SoN 优化与机器码质量

`M6 · C · 依赖：T37`

- **做什么：** 依据 profile 推进。迁移顺序：纯值规则 → 控制与死代码 → 检查消除 → 受限内存/循环优化 → 内联/标量替换 → 目标相关选择/向量化。逐项迁移 `son_optimize.rs`、`value_numbering.rs`、`loop_analysis.rs`、`optimizer_pipeline.rs`，保存新旧规则差分。每个 pass 写明输入前提、保持的不变量、预算、终止条件和单独关闭开关。基础纯值规则可以在 M1 起随所属任务实现。
- **Rust 语义 SoN：** 按级别推进：R 级规则 → S 级种子化简与内联 → 分区（`Local` → `Frozen`/`Unique`）→ L 级机器优化 → 循环/向量化（[optimization-pipeline.md](../design/ir/optimization-pipeline.md)、[runtime.md](../design/goals/runtime.md)）。
- **规则验证：** 规则 DSL 增加 SMT 导出；整数规则用 SMT 证明或 ≤8 位穷举，CI 读缓存的证明结果。
- **完成标志：** 每个 pass 启用和禁用都能通过真实执行；浮点不默认 reassociate；整数规则按宽度与 overflow 模式验证；可能 trap 的除法不随意消除；关键 workload 的 runtime/体积数据。

<a id="t39"></a>
## T39 完整依赖缓存与增量

`M6 · E · 依赖：T29, T30, T37`

- **做什么：** 从完整对象缓存开始，再逐层缓存 Body / 实例 / 图。namespace 与 schema、命中解释、失效矩阵、原子发布、损坏恢复。键设计见 [cache](../design/build/cache.md)，可提前做不发布的实验。
- **Rust 语义 SoN：** 缓存项与键按 [design/build/cache.md](../design/build/cache.md)：S 图不缓存，答案键代替类型参数，源位置隔离；失效矩阵加注释、行号移动、布局相同的新类型参数。
- **完成标志：** 禁用/cold/warm 结果一致；泛型体、trait impl、hygiene、target、panic、overflow、pass 配置变化都会失效；build script 输入不明时不缓存。

<a id="t40"></a>
## T40 跨 crate 提前发布与关键路径调度

`M6 · E · 依赖：T11, T29, T37`

- **做什么：** “接口就绪 / 对象就绪”通知、构建世代与依赖校验、jobserver token、布局/泛型体等真实消费条件；必要的 Cargo 变更单独列出，可整体关闭。
- **Rust 语义 SoN：** 就绪信号逐体（R 图）、逐实例（摘要、机器码）发出；库 crate 用空闲 token 预生成实例（[compile-time.md](../design/goals/compile-time.md#关键路径)）。
- **crate 内并行：** 在 T11 的基础上扩展到全部前端阶段，并测 1/4/8 worker 曲线。
- **完成标志：** 取消、崩溃、旧产物、循环等待、内存压力都验证过。若没有稳定收益，保留 crate 内调度并记录可重复的“不采用”结论，任务同样可完成；T41 只测实际采用的配置。

<a id="t41"></a>
## T41 公平产品比较

`M6 · E · 依赖：T38, T39, T40`

- **做什么：** 按 [性能比较协议](README.md#验收) 测所有构建场景和代表 workload：多轮原始采样、置信区间、等同 strip 的文件/可加载节大小、CPU/内存/阶段成本、host/target 指纹。
- **完成标志：** 达到本阶段出口的比值目标；每个 workload 单列，不用平均数掩盖退化。

<a id="t42"></a>
## T42 产品出口与下一阶段决策

`M6 · I · 依赖：T37, T41`

- **做什么：** 最终功能、质量、速度和范围核对；可重放的发布候选证据；未达标项与后续计划。
- **完成标志：** 所有原始目标逐项满足；否则保持 `blocked`/`in_progress`，保留成果和失败证据。本任务不等于合并或发布授权。
