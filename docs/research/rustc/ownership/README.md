# 所有权：move、初始化、drop 与 MIR 构建中的作用域

基于 rustc 1.94.1（`e408947bfd200af42db322daf0fadfe7e26d3bd1`）源码。范围：move path、初始化数据流、move 错误判定、drop 显式化与 drop glue、MIR 构建中的作用域/cleanup/临时值/求值顺序/match 降低、panic 路径。借用冲突、loan、region 推导见 [../borrowck/](../borrowck/README.md)。

## 文件

| 文件 | 内容 |
|---|---|
| [move-paths.md](move-paths.md) | `MoveData`/`MovePath` 前缀树、`gather_moves`、哪些 place 不能 move out |
| [init-dataflow.md](init-dataflow.md) | MaybeInit/MaybeUninit/EverInit、`drop_flag_effects`、borrowck 的 use-after-move、部分初始化、E0384 判定 |
| [drop-elaboration.md](drop-elaboration.md) | ElaborateDrops pass、`DropStyle`、drop flag、open drop（struct/enum/数组/Box）、drop glue 与 shim、析构顺序 |
| [scopes-and-cleanup.md](scopes-and-cleanup.md) | 作用域栈、`DropTree`、break/continue/return/else 退出、unwind 树与缓存、cleanup 不变量、panic=abort |
| [temporaries.md](temporaries.md) | region scope tree、terminating scope、临时值生命周期与延长、`super let`、edition 2024 变化 |
| [eval-order.md](eval-order.md) | `as_place`/`as_operand`/`as_temp`/`expr_into_dest`、Copy/Move 决定、各表达式求值顺序、Assert |
| [match-lowering.md](match-lowering.md) | 决策树（回溯自动机）、测试种类、or-pattern、false edge、guard 绑定、fake borrow |
| [test-cases.md](test-cases.md) | tests/ui 中的代表用例及其考察的规则 |

## rustc 中这条链怎么走

```
HIR ──region_scope_tree (语法 + span edition)──┐
 │                                              │ temporary_scope / var_scope
 ▼                                              ▼
THIR（每个表达式包 Scope{Node}，记 temp_scope_id）
 │  mir_build：作用域栈 + DropTree；每个作用域出口无条件插 Drop(local)/StorageDead；
 │            可 unwind 终结符经 diverge_from 挂 unwind 树；Copy/Move 按类型决定；
 │            match 生成 FalseEdge / FakeRead / Fake 借用；panic 策略不参与
 ▼
MIR(Analysis) ── borrowck：gather_moves(|_| true) + MaybeUninit + EverInit + Borrows (+NLL)
 │                → use after move / 部分初始化 / move out of borrow / 二次赋值
 ▼  CleanupPostBorrowck：删 FakeRead、Fake 借用、FalseEdge
 ▼  ElaborateDrops：gather_moves(needs_drop) + MaybeInit + MaybeUninit
 │                → Dead / Static / Conditional(drop flag) / Open(drop ladder)
 ▼  AbortUnwindingCalls：按 panic 策略把不会 unwind 的边改 Unreachable，删死块
MIR(Runtime) ── codegen：Drop(place) = 调 drop_in_place::<T>（shim 用同一展开引擎生成）
```

三条贯穿全链的设计：(1) **先粗后细**：建 MIR 时不判断值是否已 move，所有出口都插 Drop，由数据流事后决定；(2) **检查用的 CFG 比执行用的 CFG 更宽**：false edge、`FalseUnwind`、cleanup 边在检查时都在，检查后再删；(3) **生命周期由语法决定**：临时值和绑定的 drop 点只取决于 region scope tree，与类型推导结果无关（只用 `needs_drop` 过滤）。

## kfrc 最小闭包与移植建议

### Typed Body 必须能表达的东西

对应 [representations.md](../../../design/ir/levels.md) 和 [frontend.md](../../../design/frontend/overview.md)：

| 需要 | 原因 | rustc 对应 |
|---|---|---|
| `Place = LocalId + [投影]`，投影含 Field/Deref/Downcast/ConstantIndex{offset,min_length,from_end}/Subslice/Index(LocalId) | move path 前缀树、非法 move 判定、数组模式 | `Place`、`ProjectionElem` |
| `Operand = Copy(place) \| Move(place) \| Const` | move 只从 `Move` 收集 | `Operand` |
| `Drop{place, target, unwind, replace}` 终结符 | 作用域出口与赋值的析构点；可 unwind | `TerminatorKind::Drop` |
| Call/Assert/Drop/InlineAsm 的 `unwind: Continue \| Cleanup(bb) \| Terminate \| Unreachable`；块级 `is_cleanup` | unwind 路径析构、Call 结果只在正常边初始化、cleanup 不变量 | `UnwindAction`、`BasicBlockData::is_cleanup` |
| `StorageDead(local)`（可选 `StorageLive`） | move data 把它当 move；EverInit 在此清零（循环里 `let x; x = ..`）；NLL 借用活不够久 | `StatementKind::StorageDead` |
| 仅检查用：`FalseEdge`、`FalseUnwind`、`FakeRead`、`Fake` 借用、guard 内的 ref-binding | match/loop 的 borrowck 精度 | 见 [match-lowering.md](match-lowering.md) |
| `UnwindResume`、`UnwindTerminate`、`Unreachable` | cleanup 树根、cleanup 内再 panic | 同名终结符 |

回答计划中的四个问题：

- **cleanup 边**：需要，而且 abort-only 阶段也要在 Body 上建出来供检查（rustc 的 MIR 构建和 borrowck 都不看 panic 策略），在交给 SoN 之前按策略裁掉。见 [scopes-and-cleanup.md](scopes-and-cleanup.md#cfg-不变量与-panic-策略)。
- **drop flag**：TypedBody 不含；drop 显式化后以普通 `bool` 局部和 `SwitchInt` 出现在 CheckedBody 上，SoN 不需要专门节点。
- **StorageDead**：需要（三个用途见上表）；SoN 构造时忽略。unwind 路径上不发（非协程）。
- **临时值作用域**：Body 不存 scope tree，但 Body builder 必须查询它来放置 `Drop`/`StorageDead`；表达式降低接口带 `TempLifetime`。region scope tree 只依赖语法和 span edition，可以先于类型检查移植。

### 阶段建议

- **[T06](../../../plan/m1-vertical-chain.md#t06)**：在 Body builder 里直接实现作用域栈 + break/continue/else/unwind 四类 `DropTree`（标量阶段 Value drop 为空，但 `StorageDead`、break/return 出口、溢出/越界 `Assert` 的 unwind 边都已存在）。所有控制流经 CFG 汇合，正是消除 if 错编译那类问题的结构。同时移植 region scope tree（~1000 行，无类型依赖）。
- **[T09](../../../plan/m1-vertical-chain.md#t09)**：SoN builder 只吃“检查后、清理后”的 Body：`FalseEdge`→Goto，删 `FakeRead`/Fake 借用/`StorageDead`；Call/Assert/Drop 是带异常控制出口的效果节点；abort 配置下 cleanup 块已被裁掉。
- **[T13](../../../plan/m1-vertical-chain.md#t13) 实验闭包**：move path、前向数据流框架、MaybeUninit/EverInit 判定 E0381/E0382/E0384、`check_movable_place` 判定 E0507/E0508/E0509（E0505 等借用冲突归 [../borrowck/](../borrowck/README.md)）、MaybeInit+MaybeUninit 驱动的 drop 显式化（Dead/Static/Conditional + struct/元组的 Open）、`let`/`if`/`loop`/`return` 的作用域退出、一个带副作用 `Drop` 的类型。可推迟：精确 enum drop、数组/slice/Box/union 的 open drop、match guard/or-pattern/fake borrow、`super let`、2024 rescope、协程/async drop、迁移 lint（`ForLint`/`BackwardIncompatibleDropHint`）。
- **[T22](../../../plan/m3-semantics.md#t22)**：补齐上面推迟的项，以及 drop-and-replace、部分初始化规则、enum 精确 drop 与 `&mut` 规则（#90752）。
- **[T25](../../../plan/m3-semantics.md#t25)**：drop glue 复用同一个 `elaborate_drop` 引擎（无 flag 的 elaborator），每个需要 drop 的类型一个实例；`needs_drop` 为假时不生成。
- **[T27](../../../plan/m3-semantics.md#t27)**：实现 `AbortUnwindingCalls` 的等价裁剪（abort 下非 foreign 调用一律不 unwind）、`Terminate(InCleanup)` → 中止、landing pad 与展开表；验收用 [test-cases.md](test-cases.md) 中的 dynamic-drop 系列。

### 耦合与工作量（行数为 rustc 源码规模；kfrc 估算为推断）

| 组件 | rustc 规模 | 需要的外部信息 | 移植方式 |
|---|---|---|---|
| move path + lookup | ~1050 行 | Drop impl 有无、Box/union、数组长度、`needs_drop` | 可借算法 |
| 初始化数据流 + drop_flag_effects | ~900 行 + 框架 | 无（只用 MoveData 与 CFG） | 可借算法；框架自写前向位集版 |
| borrowck 的 move/init 检查 | ~600 行（lib.rs 相关函数） | 前缀遍历、mutability | 可借；诊断文本自写 |
| ElaborateDrops + 展开引擎 | ~2000 行 | ADT 字段/variant/判别值、`Drop::drop` 与 `drop_in_place` lang item | 可借 |
| scope.rs（作用域、DropTree） | ~2200 行（协程部分可删） | region scope tree、`needs_drop` | 重写适配到自有 Body builder |
| region scope tree | ~1250 行 | HIR 形状、span edition | 可借，需 kfrc 的 HIR 级 ID |
| 表达式降低 expr/ | ~3350 行 | THIR、类型、Copy 判定、方法/运算符解析结果 | 重写适配 |
| match 降低 matches/ | ~4650 行 | THIR 模式、判别值、常量值树 | 重写适配，需先冻结 kfrc 模式 IR |

真正的耦合点是 Copy 判定（trait 求解）、ADT 元数据和 THIR 形状，而不是 `TyCtxt`/query 本身；move/init/drop 三块算法只需要约六个类型查询，适合作为 T13 的首批移植对象。

## 关键结论

1. rustc 的 MIR 构建与 borrowck 不读 panic 策略；abort 只在 drop 显式化之后由 `AbortUnwindingCalls` 裁边。kfrc 的 abort-only 首轮也必须在 Body 上生成 cleanup 边，否则 accept/reject 与 rustc 不一致。
2. Drop 在建 Body 时无条件插在所有出口，drop flag 是事后由数据流生成的普通 `bool`；SoN 层不需要 drop 概念，只需要“可能 unwind 的调用”。
3. 溢出、越界、除零都是可 unwind 的 `Assert` 终结符；纯标量子集也有 cleanup 边。
4. 临时值与绑定的析构点完全由语法级 region scope tree 和 span edition 决定，且依赖 `for`/`while`/`format_args!`（`super let`）等脱糖的精确形状。
5. Copy/Move 在建 Body 时按 `Copy` impl 决定，Typed Body 依赖最小 trait 查询。
6. 需复刻而非“修正”的 rustc 行为：#47949 泄漏、or-pattern 按第一分支顺序 drop（#142163）、参数晚于体内局部、2021/2024 块尾与 `if let` 差异。
