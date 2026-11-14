# verifier 是语义的静态近似

原则：每条 verifier 检查都应能说出“它近似的是哪条语义规则”；语义里有、verifier 里没有对应检查的规则，要么是优化的证明义务（不可静态判定），要么是缺口。现有检查见 [verifier.md](../../design/ir/verifier.md)。

## 现有检查 → 语义规则

| 检查 | 近似的语义规则 | 检查模式的动态版本 |
|---|---|---|
| `DanglingInput`、`WrongArity`、`PhiArity`、`BadProjection`、`TypeMismatch` | 求值规则的前提（输入存在、类型对得上） | 解释器遇到即内部错误 |
| `IllegalAtLevel` | 每级的语义只定义本级节点 | 同上 |
| `DominanceViolation` | 按需求值：值只在其控制点之后可用；phi 输入在对应前驱末尾可用 | 求值时输入尚未产生 |
| `DataCycle` | 大步求值终止 | 求值递归深度 |
| `BadLoop` | phi 同时更新只对单入口 header 有定义 | — |
| `EffectFork` | token 是线性的内存状态 | token 新鲜度断言 |
| `TrapHoisted` | 立即 UB 节点不能按需求值，必须固定 | 浮动求值遇到立即 UB 节点即内部错误 |
| `UnpinnedAccess` | 只有 `Load.spec` 可以浮动，且要满足可解引用条件 | `Load.spec` 访问越出可解引用区间即编译器 bug |
| `FenceMissingPartition` | fence 的跨线程可见性（顺序解释器观察不到） | 无；靠 placement checker 与 litmus 测试 |
| `PartitionMismatch` | 访问的分区列表包含指针的动态分区标签 | 动态标签断言 |
| `EscapedPartitionUse` | 逃逸事件后字节归属 `Unknown` | 字节归属断言 |
| `UnprovenVariant` | `VariantField` 在变体不符时的结果（UB 或 poison） | 变体不符即断言失败 |
| `RefineEscapes` | `Refine` 的结果只在其控制点之后有定义 | 同 `DominanceViolation` |
| `OwnershipLinear` | drop 是可观察的调用序列；线性消费保证 drop 序列不变 | 对拍 drop 调用序列 |
| `BorrowScopeClosed`、`UseAfterEndBorrow` | 子分区的 UB 条件只在 `[Reborrow, EndBorrow]` 内有效 | 子分区的区间断言 |

## 语义推出、现在没有的检查

| 新检查 | 语义来源 | 级别 | 说明 |
|---|---|---|---|
| `SpeculativeLoadFootprint` | `Load.spec` 的可解引用条件 | full | 静态偏移 + 访问大小 ≤ sized pointee 大小；`&[T]`/`&str`/`&dyn` 内容永不满足（F48 的机械化） |
| `LocalLoadOutsideSlotLife` | `Local` 槽只在 `SlotLive`–`SlotDead` 间可解引用 | full / placed | 浮动的 `Local` load 被 `SlotLive` 支配、被 `SlotDead` 后支配（见 S4） |
| `AntiDependence` | token 新鲜度：浮动 load 不能放到同分区后续写之后 | placed | Click GCM 的反依赖；[ct-testing.md](../prior-art/ct-testing.md) 只列了“load 提到 store 之前”这个方向 |
| `RefineOrigin` | `Refine{Assumed}` 不能被移动或复制到新控制点 | full | `Refine{Proven}` 不受限 |
| `PoisonSourceAtUBPoint` | 新位置的 UB 点（分支条件、地址、除数、实参）若由变换引入，输入不能是 poison 产生节点的未细化结果 | 不可完全静态判定 | 写成规则表的证明义务（`introduces_ub_point`），verifier 只做保守检查：`If` 条件直接来自 `Add.nsw`/`Load.spec` 且该 `If` 是变换新建的（需要 pass 标注新建节点） |
| `ReleaseOrder` | Release/AcqRel/SeqCst 之前（按 Body 顺序）的非 `Frozen`/`Const` 写在放置后仍在其前 | placed | F11 的放置层检查 |
| `PointerEqSubstitution` | provenance：地址相等不蕴含可互换 | 规则表审查 | 禁止“`p == q` 分支内把 `q` 换成 `p`”这类规则（OOPSLA '18 的 Rust 误编译例） |
| `NoundefFact` | `noundef` 只对允许未初始化以外的标量叶子成立 | typed | 对 union/`MaybeUninit` 类型的值使用 `noundef` 事实即报错（S2） |

## 不应由 verifier 承担的

| 语义规则 | 原因 | 由谁保证 |
|---|---|---|
| 优化保持行为（refinement） | 不可判定 | 逐 pass 差分 + 规则表证明 |
| `Frozen`/`Unique` 分区对源程序成立 | 依赖源程序是否违反别名规则 | Miri（SB 与 TB）+ 检查模式 |
| NaN 选择一致 | 依赖常量折叠与硬件 | 折叠器按 x86_64 硬件规则实现，对拍 |

## 级别边界的全量检查

[levels.md](../../design/ir/levels.md#verifier-何时全量运行) 规定全量检查只在 R 发布前、S 结束、L 结束运行。检查模式解释器建议放在同样的位置，但只在测试与 `--verify-semantics` 下开，开销按“每函数一次解释”计。
