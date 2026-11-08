# Typed Body 契约（draft-0）

依据：[rustc/ownership](../../research/rustc/ownership/README.md#typed-body-必须能表达的东西)、[rustc/borrowck](../../research/rustc/borrowck/README.md#kfrc-最小闭包与移植建议)、[rustc/typeck](../../research/rustc/typeck/writeback.md)。形状接近 rustc 的 MIR，因为 move、初始化、drop 显式化和 NLL 算法都直接建立在这种形状上，偏离它就要重新设计这些算法。

## 两个产物

1. **TypeckResults**（side table，typeck 产出）：每个表达式/模式/局部的最终类型、per-expr adjustment 列表、运算符是否内建、数值 cast 种类、调用目标、方法 callee、模式绑定模式、`tainted` 标志。
2. **Body**（Body builder 用 HIR 级 AST + TypeckResults 构建）。

两者必须分开：if/match 的 LUB coercion 会回头修改前面分支的 adjustment，fallback 和 cast 检查在函数末尾，运算符是否内建在 writeback 才确定——所以不能边检查边建 Body（[T06](../../plan/m1/t06-typeck-and-body.md)）。

## Body 结构

```rust
pub struct Body {
    pub locals: IndexVec<LocalId, LocalDecl>,      // _0 = 返回值，随后参数，再是用户变量与临时值
    pub blocks: IndexVec<BlockId, BlockData>,
    pub arg_count: usize,
    // 源位置不在 Body 里：BodySourceMap 旁表（参考 RA 的 Body/BodySourceMap 分离），只移动位置不改变 CheckedBody 的哈希
}
pub struct BlockData { pub stmts: Range<StmtId>, pub term: Terminator, pub is_cleanup: bool }  // 语句全函数连续存放，块是区间

pub struct Place { pub local: LocalId, pub proj: ProjListId }
pub enum ProjElem {
    Field(FieldIdx, TyId), Deref, Index(LocalId),
    ConstantIndex { offset: u64, min_length: u64, from_end: bool },
    Subslice { from: u64, to: u64, from_end: bool },
    Downcast(VariantIdx),
}
pub enum Operand { Copy(Place), Move(Place), Const(ConstId) }

pub enum StmtKind {
    Assign(Place, Rvalue),
    StorageLive(LocalId), StorageDead(LocalId),
    FakeRead(FakeReadCause, Place),               // 只供检查
    SetDiscriminant(Place, VariantIdx),
    Nop,
}
pub enum Rvalue {
    Use(Operand), Ref(Region, BorrowKind, Place), RawPtr(Mutability, Place),
    BinaryOp(BinOp, Operand, Operand), UnaryOp(UnOp, Operand), Cast(CastKind, Operand, TyId),
    Aggregate(AggregateKind, Vec<Operand>), Discriminant(Place), Len(Place),
}
pub enum BorrowKind { Shared, Fake(FakeKind), Mut { kind: MutBorrowKind } }  // MutBorrowKind::{Default, TwoPhase, ClosureCapture}

pub enum TerminatorKind {
    Goto(BlockId), SwitchInt { discr: Operand, targets: SwitchTargets },
    Return, Unreachable, UnwindResume, UnwindTerminate,
    Call { func: Operand, args: Vec<Operand>, dest: Place, target: Option<BlockId>, unwind: UnwindAction },
    Assert { cond: Operand, expected: bool, msg: AssertKind, target: BlockId, unwind: UnwindAction },
    Drop { place: Place, target: BlockId, unwind: UnwindAction, replace: bool },
    FalseEdge { real: BlockId, imaginary: BlockId },     // 只供检查
    FalseUnwind { real: BlockId, unwind: UnwindAction }, // 只供检查
}
pub enum UnwindAction { Continue, Cleanup(BlockId), Terminate, Unreachable }
```

紧凑表示：`Place` 是 8 字节（局部 + 投影列表 id），变长实参放进池，span 放旁表（rustc 的 `Statement` 56 字节且 `Assign` 要装箱）。

## 关键规则

| 规则 | 原因 |
|---|---|
| 所有局部读写走 Place，不在 Body 里做 SSA | SSA 只在 SoN builder 里构造一次（[T09](../../plan/m1/t09-body-to-son.md)），根治 T01 那类问题 |
| 每个作用域出口无条件插 `Drop` 和 `StorageDead` | drop flag 由事后的数据流决定（Dead/Static/Conditional/Open），flag 是普通 `bool` 局部 |
| 溢出、越界、除零是可 unwind 的 `Assert` | 纯标量子集也有 cleanup 边 |
| **即使 `panic=abort`，Body 也建出全部 cleanup 边** | rustc 的 MIR 构建和借用检查不看 panic 策略；去掉 cleanup 边会改变接受/拒绝结果。CheckedBody 和 R 图始终保留完整控制流，按策略裁剪只在 S 级进行（`PanicStrategy` 是孔位答案），这样每个定义只建一份 R 图 |
| `StorageDead` 必须有 | move data 把它当 move；“曾经初始化”分析在此清零（循环里的 `let x; x = …`）；NLL 用它判断“活得不够久” |
| Copy/Move 在构建时按类型是否 `Copy` 决定 | Body builder 依赖一个最小 trait 查询（M1：基本类型硬编码；M3 起走 T21） |
| 两阶段借用标记只出现在三类位置 | 方法接收者的 autoref、复合赋值运算符、实参重借用 |
| 临时值和绑定的析构点由 region scope tree 决定 | 语法级、按 span 的 edition；Body builder 查询它（[temporaries.md](../../research/rustc/ownership/temporaries.md)） |

## 阶段与清理

```text
Body (built) ─→ 借用检查/移动检查 ─→ drop 显式化 ─→ 清理 ─→ SoN builder（R 级）
```

“清理”把 `FalseEdge` 变 `Goto`，删除 `FakeRead`、Fake 借用，以及**未被取地址局部**的 `StorageLive/StorageDead`；被取地址局部的保留，在图中变成 `SlotLive/SlotDead`（[ownership-drop-unwind.md](../ir/ownership-drop-unwind.md#栈槽活跃区间)）。SoN 只见到清理后的 Body；`Call`/`Assert`/`Drop` 在 SoN 里是带异常控制出口的效果节点。

## 首轮可不实现

`Downcast`/`Subslice` 投影、`SetDiscriminant`、`Aggregate` 中的 enum、match guard 的 fake 借用、`super let`（`format_args!` 需要，M3/M4 前补齐）。类型上存在，构建时遇到即报未支持。
