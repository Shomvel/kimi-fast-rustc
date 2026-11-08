# 共享身份、阶段接口与错误分类

以下名称是设计占位，由 [T02](../roadmap/m0-baseline.md#t02) 冻结；不要求马上成为公开库接口。

## 共享 ID

| 类型/产物 | 身份范围 | 必须携带 |
|---|---|---|
| `CrateKey` / `DefKey` | 构建图与定义 | 源版本、依赖身份、规范化定义路径与消歧编号（与 rustc 一致不含 hygiene，见 [contracts/ids.md](contracts/ids.md)）；可确定性命名 |
| `TypeId` / `LayoutId` | 一个固定的语义快照 | 驻留的类型或目标布局；数字本身不能作为持久内容键 |
| `BodyId` / `LocalId` / `BlockId` | 定义与单个函数体 | 变量遮蔽、Place、CFG 前驱/后继 |
| `InstanceKey` | 单态实例 | 定义、规范化泛型参数、所选 impl 及关联依赖 |
| `NodeId` / `ValueRef` | 一个函数图 | 图内身份、结果类型、结果投影；不能跨图裸引用 |
| `TargetSpec` / `FnAbi` | 目标与具体调用 | 宽度、对齐、传参/返回、栈约束、特性、panic/unwind |
| `ArtifactKey` | 产物与缓存 | 输入语义指纹、依赖指纹、目标、配置、实现/schema 版本 |

图内部可以用稠密 ID 和局部 arena，但所有跨任务引用都要能追溯到所属快照。`InstanceKey` 同时用于递归前向声明、符号和缓存，三处不再各维护一套函数编号。

## 阶段接口

```text
analyze_body(def, semantic_snapshot)          -> CheckedBody   | Diagnostics
instantiate(checked_body, instance, layouts)  -> ConcreteBody  | Diagnostics
build_son(concrete_body, target_contract)     -> VerifiedGraph | LoweringError
optimize(verified_graph, optimization_policy) -> VerifiedGraph
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
