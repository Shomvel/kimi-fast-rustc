# 先例调研：IR 与优化器

状态：2026-11-08 调研，供 [Rust 语义 SoN](../../design/README.md) draft-0 修订用。每个文件四部分：是什么（来源与年份）、关键证据、对 kfrc 的结论、具体改法。数字只取原始来源；找不到的写“未找到原始来源”。`ct-*.md` 是编译时间、缓存、体积、后端和测试方面的实践。

## 文件

| 文件 | 主题 |
|---|---|
| [sea-of-nodes.md](sea-of-nodes.md) | V8 离开 SoN、HotSpot C2、Graal（分层、PEA）、Click 的 Simple |
| [egraph-rewrite.md](egraph-rewrite.md) | Cranelift aegraph、ISLE 与其验证、egg/egglog/eggcc |
| [ownership-ssa.md](ownership-ssa.md) | Swift OSSA、词法生命周期、Mojo、Hylo |
| [progressive-lowering.md](progressive-lowering.md) | MLIR、Flang HLFIR、Lean 4 LCNF、Koka Perceus |
| [rust-aliasing.md](rust-aliasing.md) | Stacked/Tree Borrows、MiniRust、rustc 实际发出的属性和 MIR 事实 |
| [rustc-mir-mono.md](rustc-mir-mono.md) | MIR GVN/DestProp、post-mono MIR、polymorphization 移除、`-Zshare-generics`、cg_clif |
| [rust-specific-irs.md](rust-specific-irs.md) | Rust 专用 IR（rustc 内部、学术、其他 Rust 编译器）与相近语言的语义级 IR |
| [generic-sharing.md](generic-sharing.md) | 泛型代码共享：Carbon、Go GC-shape、.NET、Swift、MLton |
| [ct-fast-compile.md](ct-fast-compile.md) | 快速编译：Zig 自写后端与增量、Go、Carbon/Jai 目标、cg_clif、TPDE、copy-and-patch |
| [ct-incremental-cache.md](ct-incremental-cache.md) | 增量与缓存：rustc red-green、Salsa、Bazel/Buck2/sccache、Zig/Swift/Unison、增量链接 |
| [ct-parallel-pipeline.md](ct-parallel-pipeline.md) | 并行与流水线：cargo rmeta 流水线、rustc 并行前端、headstart、Swift/Clang 模块、Go export data |
| [ct-size.md](ct-size.md) | 体积：ICF、MergeFunctions、Go GC-shape 与答案键、outlining、冷热分离 |
| [ct-backend.md](ct-backend.md) | 后端质量：寄存器分配、指令选择、展开/向量化/显式 SIMD |
| [ct-testing.md](ct-testing.md) | 测试：Csmith/YARPGen/Rustlantis、Miri oracle、Alive/Crocus/Arrival、分配 checker、opt-bisect |

## 总表

| 做法 | 结论 | 主要依据 | 改哪里 |
|---|---|---|---|
| 保留 SoN（AOT、强类型、有借用信息） | 采纳（不变） | V8 的问题来自 JS 动态别名；Cranelift 2026 认为“SoN + CFG 骨架”效果好 | [levels.md](../../design/ir/levels.md) |
| 效果节点固定在控制骨架，只有纯节点和可证不变的 load 浮动 | 采纳 | Graal FixedNode、Cranelift skeleton；V8：内存操作本来很少浮动 | levels.md |
| 懒拆分内存视图（Simple 的 `MemMerge`） | 改编 | Simple 第 11 章 | [nodes.md](../../design/ir/nodes.md) |
| 按类型/字段的别名类（ECA、TBAA） | 不采纳 | Rust 不保证类型别名规则 | — |
| V8 指标：每次改动的访问数、L1d miss | 采纳 | V8：20 次访问才改 1 次，miss 约 3× | levels.md |
| 部分逃逸分析 | 改编（M6 后） | Graal PEA | [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) |
| equality saturation | 不采纳（维持） | eggcc 约为 LLVM -O3 编译时间的 11 倍 | [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) |
| aegraph 多版本 e-class（union） | 不采纳 | Cranelift：收益约 0.1%，平均 e-class 1.13 | optimization-pipeline.md |
| 创建时立即重写、规则单向化简、递归有上限、`subsume` | 采纳 | Cranelift：一遍重写在约 400 万节点中只错过 2 次 | optimization-pipeline.md |
| elaboration 式 placement（作用域 GVN + LICM + 再物化） | 采纳 | Cranelift aegraph | optimization-pipeline.md |
| ISLE 式声明式规则表 + overlap 检查 | 改编 | ISLE 2.7 万行规则；检查器发现永不触发的规则 | optimization-pipeline.md |
| 规则 SMT 验证 | 改编（M6 后） | Crocus/Arrival | [design/README.md](../../design/README.md) |
| 需 drop 的值线性消费，verifier 检查 | 采纳 | Swift OSSA | ownership-drop-unwind.md、son-verifier |
| `EndBorrow` 联合后支配、借用后地址不可用 | 采纳 | OSSA borrow scope | [memory-partitions.md](../../design/ir/memory-partitions.md)、son-verifier |
| copy propagation / ASAP 析构 | 不采纳 | Rust 的 drop 时机是语义；Swift 6.2 因此出现 4.6 s → 2629 s 的退化 | ownership-drop-unwind.md |
| verifier 只在级别边界全量运行 | 采纳 | Lean 4.22：逐 pass 检查花时间且很少发现问题 | levels.md |
| 多态阶段图发布进模块产物 | 采纳（已一致） | Lean `saveBase`、Mojo KGEN | — |
| MLIR 式 dialect 混合 | 不采纳 | 单向三级已足够；混合态只在 `lower_layout` 内部 | levels.md |
| 红线 1 放宽到“rustc 今天使用的事实 + TB 已证明的变换” | 改编 | rustc MIR GVN 已用 `&Freeze` 局部事实；TB 例 17 | [README.md](../../design/README.md) |
| 引用的 `dereferenceable` 只给 `&Freeze`/`&mut Unpin` | 改编（正确性） | rustc `abi.rs` | [values-and-types.md](../../design/ir/values-and-types.md) |
| `Unique` 分区的激活写规则 | 改编（收紧） | Tree Borrows §5 | memory-partitions.md |
| 局部 `Frozen`（只读复用） | 改编（M6 第一个开关） | TB 例 17、rustc #123602 | memory-partitions.md |
| 局部 `&mut` 重借用作用域 | 不采纳（维持） | TB 对未受保护 `&mut` 的可证变换很少 | — |
| 分区变换在 Miri SB 与 TB 下都对拍 | 采纳 | 最终模型未定 | memory-partitions.md |
| 单一 `specialize` 入口、S 图无 `Param` | 采纳 | polymorphization 移除的主因 | [generics-specialization.md](../../design/ir/generics-specialization.md) |
| S 图临时、先查重后特化、内存预算 | 采纳 | post-mono MIR：max RSS +7%（最多 +31%），“无法修复” | generics-specialization.md |
| 机器码共享位置与可内联性分开 | 采纳 | `-Zshare-generics` 的内联损失 | generics-specialization.md |
| 答案含所选 impl，不按纯布局共享 | 采纳（已一致） | Go GC-shape 字典分派变慢 | generics-specialization.md |
| 前端按类型化答案去重，先于内联 | 采纳（已一致） | Carbon：函数数 −44%～−98%，MergeFunctions 0～4% | generics-specialization.md |
| 未特化共享体 + 运行时字典 | 不采纳（冷区域除外） | Swift 未特化路径慢约两个数量级 | generics-specialization.md |
| 以 Swift SIL 的约 1/3 编译时间作为 R+S 级参照 | 采纳 | Swift WMO 博客 | [compile-time.md](../../design/goals/compile-time.md) |

## 有没有人做过 R/S/L

没有找到“多态 Rust 语义 SoN + 按答案特化 + 去重 + 自有后端”的公开实现。最接近的部分先例：rustc MIR（多态体优化一次，非 SSA，无单态后优化）、Swift SIL（语义 IR 上特化，后端仍是 LLVM）、Mojo KGEN（实例化前表示参数化代码，无公开数据）、Carbon（前端按指纹合并实例，压力测试数据好）。Rust 的其他编译器和后端都把优化交给 GCC/LLVM/C 编译器。详见 [rust-specific-irs.md](rust-specific-irs.md)、[generic-sharing.md](generic-sharing.md)。

## 编译时间、缓存、体积、后端、测试（ct-*）

| 做法 | 结论 | 主要依据 |
|---|---|---|
| debug 档用 TPDE 式单遍分配和编码 | 改编 | TPDE 比 LLVM -O0 后端快 8–26×；Zig 0.15 debug 约快 5× |
| copy-and-patch | 不采纳 | 代码慢 2.32×、体积 4.27× |
| 接口不带函数体；跨 crate 两道屏障；逐体等待限同进程 | 采纳 | rustc #64112；headstart 原型 −42% |
| 接口体规则（impl Trait、async、const、数组长度、static） | 采纳 | headstart 设计文档 |
| early cutoff 只看中间结果哈希、负依赖进键 | 采纳 | Build Systems à la Carte；Rust coherence |
| `line!()` 等值是语义输入；hygiene 不分离 | 采纳（划边界） | Zig、rustc #47389 |
| 键里不得有调度参数、绝对路径 | 采纳 | swift-driver #2249、sccache |
| 答案键之后再做 ICF 式片段折叠 | 采纳 | gold/lld ICF、mold |
| outliner | 不采纳 | `-Oz` −1.29%，全局 outliner 构建时间 1.41× |
| 冷区域留在函数内，可放 `.text.unlikely` | 采纳 | MFS RFC：hot/cold splitting 体积 +9.6% |
| SSA 线性扫描 + regalloc2 式约束接口与 checker | 采纳 | Wimmer & Franz 2010；regalloc2 |
| 规则表生成 SMT 查询，CI 检查证明 | 采纳 | Alive、Crocus、Arrival |
| Rustlantis 式 MIR fuzz，先过 Miri | 采纳 | Rustlantis OOPSLA '24 |
| pass bisect，设置进缓存键 | 采纳 | LLVM opt-bisect；rustc 删 `-Zfuel` |
| M0 做 LLVM 归因实验，量出运行时间缺口上界 | 采纳 | Cranelift 比 LLVM 慢约 14% |
| 原地增量链接 | 推迟（T39 后） | MSVC `/INCREMENTAL` 的退回条件 |
