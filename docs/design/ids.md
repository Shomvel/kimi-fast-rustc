# ID、阶段接口与错误分类（draft-0）

名称是设计占位，由 [T02](../plan/m0-baseline.md#t02) 冻结。

## 共享 ID 一览

| 类型/产物 | 身份范围 | 必须携带 |
|---|---|---|
| `CrateKey` / `DefKey` | 构建图与定义 | 源版本、依赖身份、规范化定义路径与消歧编号（与 rustc 一致不含 hygiene，见 [design/ids.md](#规则)）；可确定性命名 |
| `TypeId` / `LayoutId` | 一个固定的语义快照 | 驻留的类型或目标布局；数字本身不能作为持久内容键 |
| `BodyId` / `LocalId` / `BlockId` | 定义与单个函数体 | 变量遮蔽、Place、CFG 前驱/后继 |
| `InstanceKey` | 单态实例 | 定义、规范化泛型参数、所选 impl 及关联依赖 |
| `NodeId` / `ValueRef` | 一个函数图 | 图内身份、结果类型、结果投影；不能跨图裸引用 |
| `TargetSpec` / `FnAbi` | 目标与具体调用 | 宽度、对齐、传参/返回、栈约束、特性、panic/unwind |
| `SpecializationKey` | 共享的实例代码 | R 图内容哈希 + 孔位答案（[generics-specialization.md](ir/generics-specialization.md#答案键)）；多个 `InstanceKey` 可映射到同一个键 |
| `ArtifactKey` | 产物与缓存 | 输入语义指纹、依赖指纹、目标、配置、实现/schema 版本 |

图内部可以用稠密 ID 和局部 arena，但所有跨任务引用都要能追溯到所属快照。`InstanceKey` 同时用于递归前向声明、符号和缓存，三处不再各维护一套函数编号。

## 新类型

## 新类型

```rust
// 构建图层：跨进程、可持久化
pub struct CrateKey(Fingerprint);          // crate 名 + 版本 + 源指纹 + features + target
pub struct DefKey { krate: CrateKey, path: DefPathHash }   // 规范化定义路径的哈希

// 语义快照层：一次编译会话内有效
pub struct DefId(u32);                     // 会话内稠密编号，映射到 DefKey
pub struct TyId(u32);                      // 驻留在 TypeInterner
pub struct LayoutId(u32);                  // 驻留在 LayoutInterner，键为 (TyId, TargetSpecId)
pub struct BodyId(u32);                    // 一个函数/常量体

// 函数体内
pub struct LocalId(u32);                   // Body 内局部变量，遮蔽产生新的 LocalId
pub struct BlockId(u32);                   // Body 的基本块

// 实例层
pub struct InstanceKey { kind: InstanceKind, def: DefKey, args: GenericArgsHash }  // 持久；kind 区分 Item/ReifyShim/ClosureOnceShim/DropGlue/VTableShim 等，const 参数按 valtree 哈希
pub struct InstanceId(u32);                // 会话内编号

// 图内
pub struct NodeId(u32);                    // 只在一个 FunctionGraph 内有效
```

## 规则

- **持久 vs 会话。** `CrateKey`、`DefKey`、`InstanceKey`、`Fingerprint` 可以写进缓存和 metadata；`DefId`、`TyId`、`LayoutId`、`InstanceId`、`NodeId` 只在会话内有效，持久化时必须转换成持久键。
- **驻留表只追加。** 会话内 `TypeInterner` 只追加；并行任务读快照，新建类型走同步驻留（初版一把锁即可）。
- **DefPathHash** 由规范化路径计算：`crate::module::item` + 消歧器（同名 impl、闭包、宏生成的同名项按创建顺序编号）。与 rustc 一致，DefKey 不含 hygiene；“完成顺序不影响 DefKey”依赖展开顺序确定，并行展开时按展开树中的位置定序（见 [T05](../plan/m1/t05-resolve.md#defkey-与-hygiene)）。
- **符号名** 由 `InstanceKey` 确定性生成（v0 mangling 风格或自有格式），用于对象文件和链接。
- **跨图引用** 一律用 `InstanceId`/`DefId`，不用另一个图的 `NodeId`。

## 与现状的映射

| 现在 | 替换为 |
|---|---|
| `NodeKind::Call(usize)`（声明序号） | `Call { callee: InstanceId }`，符号名来自 `InstanceKey` |
| `FunctionMetadata.name: String` | `InstanceKey` + 确定性符号名 |
| `HashMap<NodeId, ScalarType>`（frontend 内） | 节点自带 `ValueTy`（见 [son.md](ir/graph.md)），源类型在 Body 中用 `TyId` |
| `Binding` 按名字查找 | Body 中的 `LocalId` |

## 阶段接口

```text
analyze_body(def, semantic_snapshot)          -> CheckedBody   | Diagnostics
build_son(checked_body)                       -> RustGraph     | LoweringError   // R 级，含孔位表
optimize(rust_graph, optimization_policy)     -> RustGraph                       // R 级，每定义一次
answer_holes(rust_graph, instance)            -> Answers       | Diagnostics     // 得到 SpecializationKey
specialize(rust_graph, answers)               -> SpecializedGraph                // S 级 + 调用摘要
lower_layout(specialized_graph, layouts, target_contract) -> LayoutGraph         // L 级
lower_machine(verified_graph, fn_abi)         -> MachineFunction | UnsupportedFeature
emit_object(functions, data, symbols, target_contract) -> ObjectArtifact
```

这是行为契约：输入快照不可变；函数图由单个任务独占修改；结果验证后发布。不能要求调用者记住“先调用某个隐藏初始化步骤”。

## 错误分类

| 类别 | 例子 |
|---|---|
| 源程序错误 | 类型不匹配、借用冲突 |
| 明确未支持 | 尚未实现的 ABI、intrinsic、语言特性 |
| 内部不变量失败 | verifier 拒绝 |
| 外部工具失败 | 链接器出错 |

未知语义不能降级成空操作、零值或跳过检查。`panic=abort`、overflow-checks 等只来自已确认的配置。
