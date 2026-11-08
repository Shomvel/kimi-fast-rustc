# 执行语义：控制骨架、按需求值、效果与 UB

## 图怎么执行

推荐的语义沿用 Graal IR 的 Isabelle 形式化（Webb 等 ATVA '21：数据节点按需大步求值、控制小步、phi 同时更新，可执行并对拍过 Graal 测试），细节见 [prior-art-graph-irs.md](prior-art-graph-irs.md)；效果 token 与检查模式是本调研的补充（推断）：

| 部分 | 规则 |
|---|---|
| 控制骨架 | 从 `Start` 沿控制边执行**固定节点**；`Region` 记下从第几个前驱进入；`Phi` 取对应输入，同一 `Region` 的全部 `Phi` 同时取值 |
| 浮动节点 | **按需求值**：固定节点或 `Phi` 需要它时才算；结果在“它依赖的 `Phi` 都没变”期间复用（循环每轮重新算） |
| 效果 token | 值，不是隐式状态：`Effect(P)` 表示“分区 `P` 在该点的内存”；固定的内存节点按控制顺序执行并产出新 token |
| 浮动 load | 在求值时读内存，**义务**是读到的等于其 token 时刻的内容（[memory.md](memory.md#检查模式把静态分区规则变成动态断言) 的第三条断言） |
| 可观察行为 | 外部调用（FFI、系统调用）及其实参、进程终止方式（正常返回、panic 消息与退出码、abort）；**不含**分配器调用、地址值、允许集合内的 NaN 位串 |
| refinement | 目标的可观察行为集合 ⊆ 源的；源在某点 UB 时，目标在该点之后任意（CompCert 式，见 [prior-art-verified.md](prior-art-verified.md)） |

按需求值成立的前提：**浮动节点必须是全函数**——求值时机不同，结果不能从“正常”变成“UB”。TurboTV 把同一要求表述为“SoN 有效 = 所有合法调度等价”，V8 默认假设而不检查。这一条决定了下面的分类。

## 节点的四类语义

| 类 | 节点 | 求值时机 | 出错时 | 可投机（上提出条件） |
|---|---|---|---|---|
| 纯、全函数 | `Add.wrap`、`Compare`、`Field`、`Aggregate`、`MakeVariant`、`WidePtr`、`Select`、`Cast`（非 transmute）、`FieldAddr`、`AddrAdd` | 按需 | 不出错；poison 逐叶传播 | 是 |
| 纯、产生 poison | `Add.nsw/nuw`、`Shl.unchecked`、`Load.spec`、`Transmute.spec`、`VariantField`（变体不符时，推荐） | 按需 | 返回 `Poison` | 是 |
| 立即 UB，固定 | `Div`/`Rem`（除数可能为 0 或 `MIN/-1`）、固定的 `Load`/`Store`、`Refine`、`Transmute`（固定版） | 控制顺序 | UB | 否；verifier 的 `TrapHoisted`/`UnpinnedAccess` |
| 效果 | `Call`、`Store`、原子、`Fence`、`Alloc`、`DropValue`/`DropInPlace`、`SlotLive`/`SlotDead` | 控制顺序 + token | 依节点 | 否 |

现设计把 `Add.nsw` 定为“溢出时 UB”（[values-and-types.md](../../design/ir/values-and-types.md#整数运算模式)），同时 [levels.md](../../design/ir/levels.md#固定与浮动) 让纯值节点浮动。两者合起来，程序的 UB 集合取决于 placement 把它放在哪里，语义不确定。修法见 [recommendations-soundness.md](recommendations-soundness.md) S1。

`Div` 不能改成 poison：x86_64 的 `div` 对 0 会陷入，硬件层面就是立即效果。

## Refine

`Refine{fact}(v) ctl=k` 的语义：控制到达 `k` 时，若 `v` 是 poison 或 `fact(v)` 不成立 → 见下表；否则结果等于 `v`。使用者只能在 `k` 之后求值（`RefineEscapes` 是它的静态近似）。

| 来源 | 例 | 事实不成立时 | 解释器检查模式的处理 |
|---|---|---|---|
| `Proven`：支配它的分支或检查推出 | `Check.Bounds` ok 出口后 `i < len`；`VariantSwitch` 出口后 `variant = v`；`x != null` 分支后 `nonnull` | 不可能发生 | 失败即**编译器 bug** |
| `Assumed`：语言 UB 规则推出 | `hint::assert_unchecked`、`unreachable_unchecked` 的反面、固定 load 的有效范围、`&T` 参数的对齐 | 程序 UB | 失败即**程序 UB**，需 Miri 同样报 UB 才算合法用例 |

两种来源对优化的权利不同：`Proven` 事实可以被任何变换保留；`Assumed` 事实被投机移动时必须丢弃（LLVM 把 load 上提时丢掉 `!noundef`/`!range` 是同一道理）。

## Select

`Select(c, a, b)`：`a`、`b` 都已按需求值（纯，不会 UB）；`c` 是 poison → 结果 poison；否则取一臂，另一臂即使是 poison 也不影响结果。

| 变换 | 是否合法 |
|---|---|
| `If c {a} else {b}` + Phi → `Select(c, a, b)`（L 级 if-conversion，W02） | 合法：源按 poison 分支是 UB，目标只是 poison |
| `Select` → 分支 | 只在 `c` 不可能是 poison 时合法（`c` 来自固定 load、参数或 `Refine`）；否则需要 `freeze` |
| `Select(c, x, x)` → `x` | 合法 |
| `Select(c, a, b)` 的臂含 `Div` 等立即 UB 节点 | 不可能：立即 UB 节点是固定的，臂只能引用它的结果 |

## 投机 load

`Load.spec[P]{ty}(addr) mem=t` 只在下列条件**静态**成立时可以存在（否则就是固定 `Load`）：

| 条件 | 依据 |
|---|---|
| `P ∈ {Frozen, Unique, Const}`，访问区间落在 sized pointee 的静态大小内 | protector 期间可解引用；F13、F48 |
| `P = Local(s)`：位置在 `SlotLive(s)` 之后、`SlotDead(s)` 之前 | 槽在活跃区间外不可解引用；现设计未写（S4） |
| 没有数据竞争 | `Frozen`/`Const` 无写者；`Unique` 期间其他线程不能访问 |

解码失败得 poison，不是 UB；`range`、`nonnull` 这类事实留在原位置，以 `Refine{Assumed}` 表达。

## fence 与原子

顺序解释器观察不到 F11（Release fence 前的写对另一线程不可见）。这类约束写成 placement 规则：Release/AcqRel/SeqCst 之前（按 Body 顺序）的所有非 `Frozen`/`Const` 写，在放置后的块中仍在它之前；由 placement checker（[verifier-mapping.md](verifier-mapping.md)）检查，并用多线程 litmus 程序真实执行。Miri 的弱内存模拟可以作为源程序一侧的参考（推断，未核对其覆盖面）。
