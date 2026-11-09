# SoN 契约（draft-0）

> 级别（R/S/L）、Rust 级节点、内存分区和孔位见 [levels.md](levels.md)、[nodes.md](nodes.md)。本文件是图的数据结构契约；与它们冲突时以它们为准，并回改本文件。

## 历史起点与当前边界

最初原型使用无类型的全程序 Graph；该旧链已经移除，不再以 legacy_adapter 过渡。
当前实现见 [graph.rs](../../../crates/kfrc-son/src/graph.rs)，下列结构仍是目标契约示意，
不代表字段和多分区机制已经全部实现。语义要求／凭据见
[requirements-and-facts.md](requirements-and-facts.md)，其他约束见
[constraints-and-effects.md](constraints-and-effects.md)。

## 函数图

```rust
pub struct FunctionGraph {
    instance: InstanceId,
    recs: Vec<NodeRec>,            // 16 字节定长记录，按创建顺序
    ctl: Vec<NodeId>,              // 列：控制输入（纯值节点为 NONE）
    ty: Vec<ValueTyId>,            // 列：结果类型
    extra: Vec<NodeId>,            // 多输入节点（Phi、Call、Aggregate）的输入池
    forward: Vec<NodeId>,          // 替换转发表（4 字节/节点），不维护 use-list
    start: NodeId,
    state: GraphState,
    level: GraphLevel,             // Rust | Specialized | Layout，只降不升
    holes: Vec<Hole>,              // 仅 R 级非空
}
pub enum GraphState { Building, Sealed, Verified }
```

- 一个实例一个图，任务独占修改；不再用 `FunctionMetadata` 从全程序图里切分。
- `Building`：允许未封闭的循环 header（回边未全部加入）。`Sealed`：所有 Region 的前驱确定。只有 `Verified` 的图能交给优化器、lowering 和缓存。

## 节点

```rust
#[repr(C)]
pub struct NodeRec {               // 16 字节；字段私有，经访问器读写
    op: u16, flags: u16,           // opcode、DEAD 等标志
    payload: u32,                  // 常量池下标、字段号、孔位号…
    in0: NodeId, in1: NodeId,      // 前两个输入；更多输入放 extra 池，in1 存池区间
}
```

- **布局依据**：现状每节点约 96 字节、1.33 次堆分配；16 字节记录加 `ctl`/`ty` 两列实测 19–22 字节，几乎不分配，克隆从 36–64 ns/节点降到 0.3–1.0 ns/节点，1M 节点 DFS 快约 2.6 倍（本机微基准，只作相对比较，[node-storage](../../research/data-structures/node-storage.md)）。Carbon、Cranelift 也是 16 字节。
- **效果输入就是普通 `NodeId`**：分区写在效果 token 的类型 `Effect(P)` 里，不在每条边上带 `PartitionId`；调用和 `Return` 只收一个 `MemMerge`。
- **不维护 use-list**：维护它每节点多约 30 字节，比节点本身还大（V8 SoN 节点大正是因为同时指向输入和使用）。替换用转发表；需要使用者的 pass 临时建反向索引，或用饱和计数（Turboshaft 只留 1 字节）。
- **压实**：删除只打 `DEAD` 标志；在 R 图发布、`specialize`、placement 之前三处保序压实（随机顺序存放时遍历慢 2.6–4.0 倍；Graal 在存活率低于 70% 时压缩）。
- **证明引用与执行输入分开**：事实可用紧凑旁表表示，保留 predicate、witness、scope 及必要状态依赖；重写和压实同时重映射。它们参与验证和优化依赖，但不自动产生机器数据需求或寄存器活跃区间。
- **Verified 有版本范围**：相关图、类型或凭据变更使验证状态及依赖分析失效；Verified 不允许跳过对新图的检查。

```rust
pub enum ValueTy {
    Rust(TyId),                       // R/S 级：聚合、enum、引用、宽指针都是值
    Int { bits: u8, signed: bool },   // 8/16/32/64/128；bool 用 Int{bits:1}
    Float { bits: u8 },               // 32/64
    Ptr,
    Control, Effect,
    Tuple(TupleTyId),
    Unit,
}
```

整数的 signed 只影响需要它的运算（除法、比较、扩展）。也可以像现在一样把有/无符号放进 opcode（`Div`/`UDiv`），T02 二选一，推荐保留现在 opcode 区分的做法，`ValueTy::Int` 只记宽度。

## 新增的节点种类（按阶段）

| 阶段 | 节点 | 说明 |
|---|---|---|
| M1 | `Call { callee: InstanceId }` 产出 `Tuple(Control, Effect, Value)` | 取代 `Call(usize)`；即使首版无内存，也带效果输入输出 |
| M1 | `Unreachable` | 执行到达即源 UB；实际无限循环保留 CFG，非返回调用保留效果／unwind，不用此节点替代它们 |
| M2 | `Load { align }`、`Store { align }` | 效果输入 → 新效果输出 |
| M2 | `Addr`（基址 + 偏移）、`StaticAddr(DataId)` | 地址计算、静态数据 |
| M2 | `Trunc`/`ZExt`/`SExt`、浮点运算与转换 | 宽度变化 |
| M2 | `Check { kind: Overflow \| Bounds \| DivZero }` → `Tuple(Control ok, Control fail)` | 失败分支连到 panic 调用 |
| M3 | `CallUnwind`：在 `Call` 基础上多一个 unwind 控制结果 | 接异常 cleanup |
| M3 | `Atomic*`、`Fence`、`VolatileLoad/Store` | 都在效果链上 |

## Region 与 Phi

- `Region { preds: Vec<NodeId> }`：前驱是 `Jump`/`Projection`。
- `Phi { region: NodeId, values: Vec<NodeId> }`：`values[i]` 对应 `region.preds[i]`，结构上保证一一对应。
- 效果合流也是 Phi；单链时每个 Region 至多一个效果 Phi，多分区时每个 P 各一个 Phi<Effect(P)>。MemMerge 只打包同点视图，不替代 Phi。
- 循环 header 是 Region；`Building` 阶段可以先放入口前驱，封闭时补回边（参考 Braun et al. 的 sealed block 构造，T09 使用）。

## 效果链

`Start` 产出 `Tuple(Control, Effect, params…)`。所有读写内存、调用、可能 trap/unwind 的节点取一个效果输入、产出一个新效果。首版只有这一条链（保守），见 [representations.md](memory-partitions.md)。

链上的顺序依赖不等同“写所有内存”。Require 的迁移可以暂借效果链排序，但没有
内存读写；Opaque 的优化策略也不自动变成全局内存屏障。Slot／借用标记改变抽象
状态或凭据范围，消去它们的表示前仍须落实实际操作的依赖。

## 实现边界

生产路径只使用 semantic FunctionGraph。目标设计的多分区链、要求节点与事实旁表
逐项迁移和验收，不恢复旧 IR。现有实现覆盖见 [PoC 状态](../../plan/poc/semantic-ir-status.md)。
