# 类型表示契约（draft-0）

依据：[rustc/typeck](../../research/rustc/typeck/README.md#kfrc-最小闭包与移植建议)、[rustc/traits](../../research/rustc/traits/README.md#kfrc-最小闭包与移植建议)。

## TyKind

种类划分与 `rustc_type_ir` 同构，这样以后可以选择实现 rustc 的 `Interner` trait 去复用新 trait 求解器（是否复用在 T14 决定，见 [T12](../../plan/m1/t12-trait-experiment.md)）。

```rust
pub enum TyKind {
    Bool, Char,
    Int(IntTy), Uint(UintTy), Float(FloatTy),     // IntTy::{I8,I16,I32,I64,I128,Isize}
    Adt(AdtId, ArgsId), Str, Array(TyId, ConstId), Slice(TyId),
    Ref(Region, TyId, Mutability), RawPtr(TyId, Mutability),
    Tuple(TyListId), Never,
    FnDef(DefId, ArgsId), FnPtr(FnSigId), Closure(DefId, ArgsId),
    Dynamic(PredListId, Region),
    Param(ParamIdx), Placeholder(Universe, BoundVar), Bound(DebruijnIndex, BoundVar),
    Alias(AliasKind, AliasId, ArgsId),             // Projection | Opaque | Free
    Infer(InferVar),                               // 只在推导期出现
    Error,
}
pub enum InferVar { Ty(TyVid), Int(IntVid), Float(FloatVid) }
```

- **M1 必需的种类：** `Bool`、`Int/Uint` 全部宽度（**含 `i32`**，因为无约束整数字面量 fallback 到 `i32`）、`Tuple([])`、`Never`、`FnDef`、`Infer`、`Error`。其他种类在类型上存在，lowering 遇到即报未支持。
- **region 槽：** `Ref` 和 `Dynamic` 带 `Region`。typeck 一律填 `Erased`；借用检查在 Body 上把每个槽替换为自己的 `RegionVid`。
- **驻留：** `TyId` 全局驻留（见 [ids.md](../ids.md)）。`Infer` 只能出现在一个 body 的推导表里，发布 TypedBody 前全部解析掉（对应 rustc writeback 的断言）；它不能出现在 Body、缓存键或跨任务产物中。

## 推导表

- 三张 union-find（类型变量、整数变量、浮点变量）+ 一条 undo log；快照就是 log 长度，严格嵌套。
- 提供 `commit_if_ok`、`probe`；M1 就实现，因为 coercion 尝试和将来的方法探测都建立在它上面。
- 关系接口从一开始带方差：`relate(a, b, Variance) -> Result<Goals, TypeError>`；M1 只用 `Invariant`。
- 挂起约束队列：M1 就需要（例如 `-x` 在 `x` 稍后被定为 `u64` 时报 E0600）。

## 谓词（trait 求解接口）

```rust
pub enum Predicate {
    Trait(TraitRef, Polarity), Projection(AliasTerm, Term), NormalizesTo(AliasTerm, Term),
    AliasRelate(Term, Term, Direction), WellFormed(Term), Ambiguous,
    // 之后：Outlives、HostEffect（const trait）、Coerce、Subtype …
}
```

typeck 需要求解器提供三种接口：obligation 队列（注册 + 尽量选择）、局部选择循环（如 unsize 时当场证明 `CoerceUnsized`）、探测式评估（`predicate_may_hold`）。见 [typeck/expectation-and-fallback.md](../../research/rustc/typeck/expectation-and-fallback.md)。
