# SoN 契约（draft-0）

## 从现状出发

现在的 [son.rs](../../../../src/son.rs)：一个程序一个 `Graph`，`Node { kind, control: Vec, data: Vec, memory: Vec }`，节点无类型，`Phi` 的 `control` 直接列出前驱 Jump（与 Region 的 control 相同），`Call(usize)` 用声明序号，13 种 `NodeKind`。保留它的 SoN 风格和已验证的优化，改动集中在以下几点。

## 函数图

```rust
pub struct FunctionGraph {
    instance: InstanceId,
    nodes: Vec<Option<Node>>,      // 保留 tombstone，NodeId 稳定
    start: NodeId,
    state: GraphState,
}
pub enum GraphState { Building, Sealed, Verified }
```

- 一个实例一个图，任务独占修改；不再用 `FunctionMetadata` 从全程序图里切分。
- `Building`：允许未封闭的循环 header（回边未全部加入）。`Sealed`：所有 Region 的前驱确定。只有 `Verified` 的图能交给优化器、lowering 和缓存。

## 节点

```rust
pub struct Node {
    kind: NodeKind,
    ty: ValueTy,                   // 结果类型；多结果节点为 Tuple，由 Projection 取
    control: Option<NodeId>,       // 所属控制（纯值节点为 None）
    inputs: SmallVec<[NodeId; 3]>, // 值输入
    effect: Option<NodeId>,        // 效果输入（内存 token）
}

pub enum ValueTy {
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
| M1 | `Unreachable` | 发散分支（`loop {}` 无 break、`!` 调用） |
| M2 | `Load { align }`、`Store { align }` | 效果输入 → 新效果输出 |
| M2 | `Addr`（基址 + 偏移）、`StaticAddr(DataId)` | 地址计算、静态数据 |
| M2 | `Trunc`/`ZExt`/`SExt`、浮点运算与转换 | 宽度变化 |
| M2 | `Check { kind: Overflow \| Bounds \| DivZero }` → `Tuple(Control ok, Control fail)` | 失败分支连到 panic 调用 |
| M3 | `CallUnwind`：在 `Call` 基础上多一个 unwind 控制结果 | 接异常 cleanup |
| M3 | `Atomic*`、`Fence`、`VolatileLoad/Store` | 都在效果链上 |

## Region 与 Phi

- `Region { preds: Vec<NodeId> }`：前驱是 `Jump`/`Projection`。
- `Phi { region: NodeId, values: Vec<NodeId> }`：`values[i]` 对应 `region.preds[i]`，结构上保证一一对应。
- 效果合并也是 Phi，`ty = Effect`。每个 Region 后至多一个效果 Phi。
- 循环 header 是 Region；`Building` 阶段可以先放入口前驱，封闭时补回边（参考 Braun et al. 的 sealed block 构造，T09 使用）。

## 效果链

`Start` 产出 `Tuple(Control, Effect, params…)`。所有读写内存、调用、可能 trap/unwind 的节点取一个效果输入、产出一个新效果。首版只有这一条链（保守），见 [representations.md](../representations.md#效果模型)。

## 过渡

旧 `son::Graph` 在调用者迁完前保留，由 `legacy_adapter` 转成 `FunctionGraph`（所有值 `Int{bits:64}`，bool 结果 `Int{bits:1}`），这样现有优化和后端可以先接到新图上验证。
