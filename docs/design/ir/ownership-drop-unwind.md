# 所有权、drop 与 unwind

存储寿命、初始化状态、借用作用域和真实析构是不同契约；边界定义见
[constraints-and-effects.md](constraints-and-effects.md)。以下为目标设计，不以标记代替源操作。

## move

- SSA 值的 move 就是把值节点交给新的使用者。源变成“已移出”只是 Body 层的初始化事实，图里没有节点。
- StackSlot 中的值被 move：按源初始化规则转移该值，源位置变为已移出；不自动 SlotDead，不清零。该槽仍可在同一存储寿命内被重新赋值，部分 move 也不影响其余字段的存储寿命。
- 部分 move（`let a = s.x;` 之后 `s.y` 仍有效）：值形式下是 `Field`，剩余字段照常可用；drop 显式化已在 Body 上决定剩余字段怎么析构。

## drop

drop 显式化在 Body 上完成（[design/frontend/body.md](../frontend/body.md)），SoN 只执行结果。

| Body | R/S 图 |
|---|---|
| `Drop(place)`，place 是 SSA 局部 | `DropValue(v)` |
| `Drop(place)`，place 在 `StackSlot` 或经指针 | `DropInPlace[P](addr)` |
| drop flag（普通 `bool` 局部） | 普通 SSA 值和 `If`；常量折叠后大多消失 |

- R 级：`DropValue` 的类型含 `Param` 时是孔位，答案是 `NeedsDrop(T)` 和 drop glue 实例。
- S 级：`NeedsDrop = false` → 删除节点（及只为它存在的 unwind 边）。类型有 `Drop` impl → `Call(<T as Drop>::drop)` 接上字段的 `DropValue`；小的 drop glue 直接展开成字段 drop 序列。
- L 级：剩下的 `DropValue` 先把值放进 value slot（drop 需要 `&mut`），再调用共享 drop glue 实例。drop glue 实例按[答案键](generics-specialization.md#答案键)共享：`Vec<u32>` 和 `Vec<i32>` 共用一个。
- `Option<String>` 这类：`DropValue` 遇到 enum，S 级展开成 `VariantSwitch` + 每变体字段 drop；已知变体（刚 `MakeVariant` 过）直接折叠。
- `dyn Trait`：drop 读 vtable 槽 0，非空则间接调用；`Box<dyn>` 再按 vtable 的 size/align 调用 `Dealloc`。vtable 是已知常量 `Const(VTable K)` 时折叠成具体类型的 drop（rustc O3 拿到常量 vtable 后仍在运行时读槽，W07）。
- drop 节点有 unwind 出口（[nodes.md](nodes.md#检查dropunwind)），按 drop glue 摘要裁剪。

## 线性消费检查

验证 Body 指定的所有权／drop 义务是否正确转移、执行或解除，不能机械要求所有
资源恰好析构一次。mem::forget、ManuallyDrop 及合法泄漏允许不析构，Copy 值的多个
使用者也不违反契约；不得据此制造 UB 或补 Drop。正常、unwind、终止及不返回路径
分别检查。EndBorrow 要覆盖 Reborrow 的有限正常和 unwind 出口，之后不能再用该
scope 的凭据；底层地址是否仍可用取决于其他有效权限，不由 EndBorrow 一刀切决定。

Rust 的 drop 时机是语义：`DropValue` 不能上移，也不能越过 `Unknown` 效果。不做 Swift 那样的 copy propagation/ASAP 析构（Swift 6.2 因此出现过 4.6 s → 2629 s 的编译时间退化）。

## 栈槽活跃区间

只有 `StackSlot` 需要活跃区间（给栈帧复用）。Body 清理阶段**保留被取地址局部的 `StorageLive/StorageDead`**，在图里变成 `SlotLive`/`SlotDead` 效果节点（挂在该槽的 `Local` 分区链上）。SSA 局部的 Storage 语句照旧删除。和 LLVM 相比，lifetime 标记只出现在少数真正有地址的槽上。

SlotLive 开始一次动态存储寿命，初始内容未初始化；SlotDead 结束寿命，均不执行
析构或清零。循环中重新 SlotLive 不复活旧寿命凭据。未初始化状态、已移出状态和
存储死亡必须分别处理；标记只在访问安全及槽复用依赖已落实后擦除。槽地址逃逸后
相应效果依赖也可能属于 Unknown，不能永久强制使用创建时的 Local 标签。

## 大值与原地构造

L 级把 `Memory` 表示的值放进 value slot。槽位分配在 SSA 上做，规则：

0. **哪些值进槽按 PassMode 决定**，不按“是不是 `Memory`”：≤8 字节的 `Memory` 聚合在 Rust ABI 下按整数传（W09：`[u8; 4]` 走 `i32`），是一个 `Int` 值；>8 字节进槽；>16 字节的 ScalarPair 只在返回边界走 sret。
1. 一个值一个槽；move（值转发）不产生新槽。`InsertField` 在操作数最后一次使用处复用它的槽（`b.n += 1` 不拷贝 136 字节，W09）。
2. Phi 的输入和结果尽量合并到同一槽（对应寄存器分配的合并）。
3. **原地构造：** 值的使用是 `Store` 到某地址、作为 sret 返回、作为 `Indirect` 实参，或是某个父聚合/变体的字段（递归：目标是父值的槽 + 字段偏移）时，在目标位置直接构造。前提是**目标地址在值产生之前已存在**：`Box::new(call())` 的堆地址在调用之后才有，首版保留一次拷贝；把分配移到调用之前需要语言层确认，并要求被调者纯、不 unwind、不发散（rustc 也留下这次拷贝，W09）。
4. **拷贝亲和合并：** 两槽之间只有整槽同偏移的拷贝、且编码重写是恒等时合并。`?` 的 `Err(Field(e))` 与 `Err(e)` 编码逐字节相同，调用结果槽和 sret 槽可以合并，错误路径零拷贝（rustc O3 仍每字段拷贝一次 40 字节，W01）。
5. `Indirect` 实参：被调者摘要 `writes_indirect_args` 为假时直接传原槽，否则先拷贝。
6. 活跃区间不重叠的槽共用栈空间。rustc 在没有 lifetime 标记时做不到（W09 `pipe<256>` 的栈帧是两槽之和），kfrc 的区间来自 SSA，不依赖标记。

这是 SROA + MemCpyOpt + DSE + 栈着色合起来的工作，但只做一次，而且在 SSA 上做，不需要从 memcpy 反推 move。

## unwind

- `panic=abort`：CheckedBody 和 R 图一律保留 unwind 边，只在 S 级按 `PanicStrategy` 答案裁剪，本 crate 与下游相同（W04：rustc abort 构建里上游泛型体留下 `resume=8` 的死 cleanup）。
- `panic=unwind`：`Call` 带 unwind 控制出口，接 cleanup 区域，区域以 `Landing` 开始、`Resume` 结束。
- **nounwind 推断：** 每个实例的调用摘要带 `may_unwind`（[计算方式](optimization-pipeline.md#调用摘要)：未优化 S 图上的廉价遍历，SCC 从 ⊥ 迭代）。调用点的被调者摘要为 `false` 时，unwind 出口是死边，cleanup 区域随 DCE 消失。这替代 LLVM 的 FunctionAttrs 推断 + 删除空 landingpad。
- 展开中再 panic：`UnwindTerminate` 变成 `Terminate` 控制。
- cleanup 区域的合并：同一 drop 序列的 cleanup 在 L 级尾部合并（相同节点哈希的控制链共享）。

## 冷区域

Check 的 fail 出口、unwind 出口和 #[cold] 调用点可按策略标为冷；这是优化启发，
不证明不可达或 UB。不能仅凭 -> ! 或最终接 Unreachable 把整个前缀视为冷：不返回
函数可能是热事件循环，前缀也可能含真实调用。#[inline(never)] 也不算冷。
被策略选中的冷区域：

- 不作为内联目标的调用点（除非被调者是 `#[inline(always)]`）。
- 只跑构建期窥孔和 DCE，不跑循环与强度削减。
- L 级放到函数末尾；同一函数里相同 panic 调用（同一 panic 函数、实参只差常量）合并成一个尾块，实参用 Phi 传入。

冷策略可以使用结构信息而不依赖 profile，但仍可能误估真实频率；不得以冷标记
放松正确性检查，收益需真实负载验证。
