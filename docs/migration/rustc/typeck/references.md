# 引用与借用类型：类型层面的规则

本文件只讲 typeck 层面：`&T`/`&mut T` 的子类型、隐式借用与重借用如何记成 adjustment、`&mut` 的特殊限制。region 求解和冲突检查属于 borrowck（[../borrowck/](../borrowck/README.md)）。

## 1. 子类型与 variance

- `structurally_relate_tys` 的 `Ref` 分支（[relate.rs#L439](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate.rs#L439)）：
  - 可变性必须相同，否则 `TypeError::Mutability`。子类型关系**不会**把 `&mut T` 变成 `&T`，那是 coercion（见 [coercion.md](coercion.md)）。
  - region 按外层 variance 关联：`&'a T <: &'b T` 记为 `'a: 'b`（`make_subregion(b, a)`，[type_relating.rs#L224](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_infer/src/infer/relate/type_relating.rs#L224)）。
  - pointee：`&T` 协变，`&mut T` 不变。
- `RawPtr` 分支（[relate.rs#L422](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_type_ir/src/relate.rs#L422)）同理：`*const T` 协变、`*mut T` 不变，可变性必须相同。
- 由于 typeck 忽略 region（[infer-ctxt.md §6](infer-ctxt.md)），在 typeck 内部 `&'a T` 与 `&'b T` 总能关联；能观察到的差别只剩 pointee 的 variance 和可变性。例如 `&mut ?X` 与 `&mut u8` 关联时 `?X` 被**等同**为 `u8`，而 `&?X <: &u8` 只让 `?X <: u8`（无子类型的类型上二者等价）。

## 2. 显式借用表达式

- `check_expr_addr_of`（[expr.rs#L476](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/expr.rs#L476)）：
  - 期望类型是 `&U`/`*U` 时，把 `U` 作为操作数的提示：place 表达式用 `ExpectHasType(U)`，值表达式用 `rvalue_hint`（`U` 是 `[T]`/`str`/`dyn` 时变成 `ExpectRvalueLikeUnsized`，避免把右值本身推成 unsized）。
  - `&mut e` 用 `Needs::MutPlace` 检查操作数，事后 `convert_place_derefs_to_mutable` 把 place 路径上的重载 `Deref`/`Index` 改成 `DerefMut`/`IndexMut`（[place_op.rs#L265](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/place_op.rs#L265)）。
  - 结果类型 `&'?r T`，`'?r` 是新 region 变量（`BorrowRegion`）；`&raw` 产生 `*T` 并要求操作数是具名 place。
- 字面量 `"..."` 是 `&'static str`、`b"..."` 是 `&'static [u8; N]`（[checks.rs#L744](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L744)）。

## 3. 隐式借用、解引用、重借用都是 adjustment

`Adjust`（[adjustment.rs#L95](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/adjustment.rs#L95)）与引用相关的变体：

| 变体 | 含义 | 产生位置 |
|---|---|---|
| `Deref(None)` | 内建解引用（`&T`/`&mut T`/`Box` 内建 deref），产生 place | autoderef、reborrow |
| `Deref(Some(OverloadedDeref{mutbl}))` | 调 `Deref::deref`/`DerefMut::deref_mut` | autoderef 经过用户 `Deref` |
| `Borrow(AutoBorrow::Ref(AutoBorrowMutability))` | 对 place 取 `&`/`&mut`；`Mut` 带 `allow_two_phase_borrow` | autoref、reborrow、deref coercion |
| `Borrow(AutoBorrow::RawPtr(m))` | 取 `*const`/`*mut` | `&T → *const T` coercion |
| `Pointer(MutToConstPointer)` | `*mut T → *const T` | coercion、方法 `ToConstPtr` |

- **重借用 = `[Deref(None), Borrow(Ref(m))]`**。`coerce_to_ref` 在期望类型是引用、实际类型也是引用时总是插入它（`&mut` 也插），唯一例外是 `&T → &T` 恰好一步的无操作（[coercion.rs#L461](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L461)）。对 `&mut` 不省略的原因写在注释里：否则 `foo(self.x)` 会成为 move 而不是重借用。
- adjustment **不带 region**：`AutoBorrow::Ref` 只有可变性（[adjustment.rs#L187](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/adjustment.rs#L187)），region 只出现在 `target` 类型里，而 writeback 会把它擦掉。
- **两阶段借用**只能由 typeck 标记：`AllowTwoPhase::Yes` 出现在方法接收者 autoref（[confirm.rs#L195](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/method/confirm.rs#L195)）、函数实参 coercion（[checks.rs#L391](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L391)）、复合赋值/按引用运算符的 autoref（[op.rs#L266](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/op.rs#L266)）。unsize 里的 reborrow、`CoerceMany` 的 coercion、`let` 初始化都是 `AllowTwoPhase::No`（[adjustment.rs#L140-L154](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/adjustment.rs#L140)）。
- THIR 构建按顺序把 adjustment 变成显式节点：`Deref(None)` → `ExprKind::Deref`，`Borrow(Ref(m))` → `ExprKind::Borrow`，重载 deref → 先借用再调 `deref` 再解引用（[thir/cx/expr.rs#L89](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/expr.rs#L89)）。

## 4. `&mut` 相关的类型层限制

- `coerce_mutbls(from, to)`：`from >= to` 才行（`Mut > Not`），所以 `&mut T → &T` 可以、反之 `TypeError::Mutability`（[coercion.rs#L100](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/coercion.rs#L100)）。
- 泛型参数推导顺序会暴露这个限制：`fn test<T>(a: T, b: T)`，`test(&7, &mut 7)` 通过（`T=&i32`，第二个实参 `&mut → &`），`test(&mut 7, &7)` 失败（`T=&mut i32`）。见 coercion.rs 顶部注释和 `tests/ui/coercion/coerce-reborrow-multi-arg-fail.rs`。
- 带 `ref`/`ref mut` 绑定的 `let` 初始化不做 coercion，而用 `demand_eqtype`（[checks.rs#L861](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/fn_ctxt/checks.rs#L861)），注释说明 `ref mut` 情况下这是 soundness 要求（#23116）。
- 赋值左侧 `check_expr_with_needs(lhs, Needs::MutPlace)`；“place 是否可变”大部分由 borrowck 检查，typeck 只负责把重载 deref 换成可变版本。
- match ergonomics 的默认绑定模式在经过 `&` 后不会再回到 `ref mut`（见 [patterns.md](patterns.md)）。

## 对 kfrc 的含义

- 在类型表示里，引用 = (可变性, pointee, region 槽)。typeck 阶段 region 槽可以统一填“未定”，不参与相等判断；borrowck 再按 Body 位置重新编号。
- 重借用、autoref、autoderef、`&mut→&`、`&T→*const T` 全部要作为 Typed Body 的显式操作（或在 Body 构建时由 adjustment 展开），并带两阶段标记；SoN builder 不能再推断这些。这正是 [frontend.md](../../architecture/frontend.md) “类型检查阶段记录隐式借用、解引用……降低阶段只执行这些决定”的具体内容。
- 不变性只在 `&mut T`、`*mut T`、带不变参数的 ADT 和推导变量合一时有可观察效果；M2 引入引用时，`relate` 需要带 variance 参数，而不是只实现“相等”。
