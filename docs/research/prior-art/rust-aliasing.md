# Rust 别名模型与 rustc 的实际事实

## 1. 是什么

| 名称 | 来源 | 年份 | 状态 |
|---|---|---|---|
| Stacked Borrows | Jung, Dang, Kang, Dreyer, [Stacked Borrows](https://plv.mpi-sws.org/rustbelt/stacked-borrows/paper.pdf)（POPL '20） | 2019-12 | Miri 默认模型（截至 2026-10） |
| Tree Borrows | Villani, Hostert, Dreyer, Jung, [Tree Borrows](https://iris-project.org/pdfs/2025-pldi-treeborrows.pdf)（PLDI '25，[ACM](https://dl.acm.org/doi/10.1145/3735592)） | 2025-06 | Miri 可选（`-Zmiri-tree-borrows`） |
| MiniRust | [minirust/minirust](https://github.com/minirust/minirust)：以解释器形式给出的“Rust lite / MIR plus”操作语义，内存接口可替换 | 进行中 | 非规范；见 [Miri README](https://github.com/rust-lang/miri) |
| rustc 发给 LLVM 的属性 | [`rustc_ty_utils/src/abi.rs`](https://codebrowser.dev/rust/rust/compiler/rustc_ty_utils/src/abi.rs.html)（`adjust_for_rust_scalar`）；[PR #106180](https://github.com/rust-lang/rust/pull/106180)；[RFC 3467 UnsafePinned](https://rust-lang.github.io/rfcs/3467-unsafe-pinned.html) | — | 见下表 |
| rustc MIR 已用的借用事实 | [GVN #109597](https://github.com/rust-lang/rust/pull/109597)（2023-09）；[#123602](https://github.com/rust-lang/rust/pull/123602)（2024-05） | — | 默认开启 |

Rust 至今没有官方确定的别名模型。Miri README 提醒：最终模型很可能比 Tree Borrows 更严格（UB 更多）。

## 2. 关键证据

**rustc 参数属性（abi.rs，1.87 快照；master 在向 LLVM 23 的语义调整，见 [PR #158863](https://github.com/rust-lang/rust/pull/158863)）：**

| 指针 | `noalias` | `dereferenceable` | 备注 |
|---|---|---|---|
| `&T`，`T: Freeze` | 是（+ `readonly`） | `size_of(T)` | |
| `&T`，`T: !Freeze` | 否 | 0（不发） | |
| `&mut T`，`T: Unpin` | 是（`-Zmutable-noalias`，默认开） | `size_of(T)` | 1.54 起稳定开启（[#82834](https://github.com/rust-lang/rust/pull/82834)；此前因 LLVM 误编译多次关闭，[#54878](https://github.com/rust-lang/rust/issues/54878)） |
| `&mut T`，`T: !Unpin` | 否 | 0 | RFC 3467 计划改为按 `UnsafeUnpin` 判断 |
| `Box<T>` | `T: Unpin` 且全局分配器且 `-Zbox-noalias`（默认开）时是 | 0 | |
| 返回值 | 从不 | — | 源码注释：返回位置不能加 `noalias` |

内联后由 LLVM 内联器把参数 `noalias` 转成 `!alias.scope`/`!noalias`。

**rustc MIR 已经用的、比参数属性更强的事实：** GVN 把“对 `Freeze` 类型不可变引用的所有解引用视为同一值”；#123602 把“只被不可变借用的 `Freeze` 局部”仍当作 SSA，理由是在借用与解引用之间重新赋值是 UB。这是**函数体内的局部借用事实**，不是参数级的。

**Tree Borrows 证明了什么（§3、§5、§6）：**

| 变换 | SB | TB |
|---|---|---|
| 受保护（函数参数）引用上的读写插入、删除、重排 | 可 | 可，三条限制：中间未知代码不能拿到该 `&mut`；**激活写（Reserved → Unique）不能移动，新写只能插在它之后**；保护结束或引用交给未知代码后两边的值必须重新一致 |
| 在激活写之前插入新写 | 可 | **不可**（两阶段借用的代价） |
| 未受保护的局部 `&T`：读 → 把 `x` 交给调用 → 再读，复用第一次读的值（例 17） | — | **已证明** |
| 相邻读重排（含未受保护的 `&mut`，例 19） | 不可 | 可 |
| 未受保护引用的“把最后一次使用下移” | 仅顺序语义下证明 | 一般不允许（引用的寿命在最后使用处结束） |
| rustc 现有别名优化 | — | 作者称“据我们所知都被 TB 支持” |

评估：29,990 个 crate 的 674,748 个测试，TB 比 SB 少拒绝 54% 的测试用例。Rocq 证明约 3.2 万行。

## 3. 对 kfrc 的结论

| 问题 | 结论 | 理由 |
|---|---|---|
| 红线 1 “不强于 rustc 今天发给 LLVM 的事实” | **改编** | 太窄：rustc 的 MIR GVN 已使用函数体内 `&Freeze` 的事实。建议改为“不强于 rustc 今天使用的事实（LLVM 属性 + 默认开启的 MIR 优化），新增事实必须是 Tree Borrows 已证明的变换，并单独开关” |
| `Frozen` 只建在参数上 | **改编** | TB 例 17 + rustc GVN 都支持局部 `&T`（`T: Freeze`）上的**读复用**；可增加局部 `Frozen` 分区，只用于删除/合并读，不用于延长引用寿命 |
| 局部 `&mut` 重借用作用域 | **不采纳（维持“明确不做”）** | TB 对未受保护 `&mut` 能证明的变换很少 |
| `Unique` 分区对写的自由度 | **改编（收紧）** | TB 不允许在激活写之前插入或上移写；循环里的 store 外提、条件 store 合并都要在“原程序已写过该分区”的位置之后 |
| 引用的 `dereferenceable` 事实 | **改编（收紧，属正确性问题）** | values-and-types.md 写的是“引用都有 `dereferenceable(size_of(T))`”；rustc 只给 `&Freeze` 和 `&mut Unpin`。`&Cell<T>`、`&mut !Unpin` 只有非空和对齐 |
| `Box` 归 `Unknown` | **采纳（维持）** | 比 rustc 保守；以后按 rustc 条件（`Unpin` + 全局分配器）开 `Unique` |
| 返回值、TBAA 不做 | **采纳（维持）** | 与 rustc 一致 |
| 对拍工具 | **采纳** | 每个分区变换的测试在 Miri 的 SB 和 TB 下都要无 UB；只在一个模型下合法的用例不能作为启用依据 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [README.md](../../design/README.md) | 设计红线 1 | 改为：“别名事实不强于 rustc 今天使用的事实（LLVM 参数属性、内联后的作用域、默认开启的 MIR GVN 对 `&Freeze` 的推理）。更强的事实只采用 Tree Borrows 论文已证明的变换，单独开关，并在 Miri SB/TB 下对拍。” |
| [values-and-types.md](../../design/ir/values-and-types.md) | 值事实 | `dereferenceable(size_of(T))` 只给 `&T`（`T: Freeze`）和 `&mut T`（`T: Unpin`，将来 `UnsafeUnpin`）；其他引用和 `Box` 只有非空、对齐 |
| [memory-partitions.md](../../design/ir/memory-partitions.md) | 规则 2 之后 | 新增“激活写”规则：`Unique(p)` 上新增或上移的写，必须被原程序中同一分区的某个写支配；不能在分区第一次写之前推测性地写 |
| [memory-partitions.md](../../design/ir/memory-partitions.md) | 明确不做 | 拆成两条：局部 `&mut` 重借用作用域仍不做；局部 `&T`（`T: Freeze`）的读复用移到“分阶段”，作为 M6 的第一个开关，依据 TB 例 17 和 rustc GVN |
| [memory-partitions.md](../../design/ir/memory-partitions.md) | 首版与分阶段 | M6 顺序改为：参数 `Frozen` → 局部 `Frozen`（只读复用）→ 参数 `Unique`（带激活写规则）→ 内联 `Reborrow` |
