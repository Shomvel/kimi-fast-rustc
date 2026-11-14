# 节点目录

级别列：R = Rust 多态级，S = Rust 单态级，L = 布局级。walkthrough 用本文件的 dump 格式。

本目录描述目标设计。2026-11-09 起，`ObserveValue` / 通用 `Refine` 的职责按
[语义要求与事实](requirements-and-facts.md) 拆分；代码迁移和历史 walkthrough 不据此视为已完成。

## dump 格式

```text
n12 = Field[1](n7) : usize
n13 = Check.Bounds ctl=n10 (n11, n12) : (Control, Control)
n14 = Load[P2] ctl=n13 (n9) mem=n5 : u32  // 读分区 P2，固定在控制 n13 下，效果输入 n5
n20 = Call(Vec<u8>::push) ctl=n18 mem=[P0:n3, P1:n19] (n16, n17) : (Control, Control?, Effect[P0], Effect[P1], ())
```

`ctl=` 是控制输入，`mem=` 是效果输入（按分区），括号里是值输入，冒号后是结果类型。多结果用 `n20.0`、`n20.1` 投影。
`proof=w1` 引用编译期凭据，不是机器值操作数；`w1` 可存在事实旁表中。

## 控制

| 节点 | 级别 | 说明 |
|---|---|---|
| `Start` | RSL | 产出控制、每个初始分区的效果、参数 |
| `Region`、`Loop` | RSL | `Loop` 是带回边标记的 Region |
| `If`、`Switch{cases}` | RSL | 整数开关 |
| `VariantSwitch` | RS | 按 enum 值的变体分支。L 级按 **tag 值**直接分支，不先把 niche 解码成变体号（解码再比较正是 rustc 留给 InstCombine 的活）；多 case 交给 `Switch` 降低，用区间比较或位测试（W03：`bt $35`） |
| `Return` | RSL | R/S 保留源码返回值和契约，即使 ABI 为 Ignore；L 再决定运输哪些位。Never 不可伪装成普通 ZST 返回 |
| `Unreachable` | RSL | 到达此处即源 UB，无后继；不是无限循环、panic 或 abort 的通用表示。真正不正常返回的调用仍保留效果和适用的 unwind 出口 |
| `Landing` | RSL | unwind 入口：Region 型合并节点，可有多个 unwind 前驱，产出 `Exception` 值（多前驱时经 Phi 合并） |
| `Resume(exc)`、`Terminate` | RSL | 消费 `Exception` 继续展开；展开中 panic 终止 |

## 纯值

| 节点 | 级别 | 说明 |
|---|---|---|
| `Const` | RSL | 标量、ZST、`ConstAlloc(AllocKey)`；R/S 级可以是聚合常量 |
| `Param`、`Phi`、`Select` | RSL | |
| 整数/浮点运算 | RSL | 模式在 opcode 上（[values-and-types.md](values-and-types.md#整数运算模式)） |
| `Compare{mode, pred}` | RSL | `mode ∈ {int_signed, int_unsigned, float_ordered, float_unordered}`；`bool`/`char`/指针按无符号 |
| `Cast{kind}` | RSL | 数值转换；`Cast.Unsize{meta}` 见下 |
| `Aggregate{ty}`、`Field[i]`、`InsertField[i]` | RS | struct/tuple/闭包环境/数组的值形式 |
| `MakeVariant{ty, v}`、`VariantOf`、`VariantField[v, i]` | RS | enum 构造、逻辑变体编号、字段投影。纯 VariantField 在变体不符时得 poison；匹配的 VariantWitness 可证明当前访问。降低成物理访问还需访问安全和控制／内存依赖，不得把纯投影的可投机性转给无保护的 load |
| `WidePtr(ptr, meta)` | RS | `&[T]`、`&dyn Tr`；`Field[0]`/`Field[1]` 取部件 |
| `Discriminant` | RS | 用户可见的判别值，规范化为 `Const(判别值表)[VariantOf(v)]`。判别值互不相同，所以 `Discriminant(a) == Discriminant(b)` 改写为变体相等，derive 的 `eq` 由此折叠 |
| `Cast.Unsize{meta}` | RS | `&T → &dyn Tr`、`Box<[T; N]> → Box<[T]>` 等；`meta` 是 vtable 常量或长度。作用于引用/裸指针时直接产出 `WidePtr`；作用于 `Box` 等智能指针时穿透到其指针字段 |
| `Repeat{count}` | RS | `[x; N]` 的值形式；L 级按 `TargetSpec` 阈值展开或调用 memset/循环 |

## 地址与内存

| 节点 | 级别 | 说明 |
|---|---|---|
| `StackSlot{ty}` | RS | 地址被取走的局部；L 级变成栈帧偏移 |
| `FieldAddr[i]`、`IndexAddr`、`VariantFieldAddr[v, i]` | RS | 结构化投影；L 级变成 `AddrAdd` |
| `AddrAdd`、`StaticAddr` | L / RSL | |
| `Load[P]{ty}`、`Store[P]` | RSL | 指定分区，固定在产生它的控制下；读到非法值是 UB。R/S 级的 `ty` 可以是聚合 |
| `Load.spec[P]{ty}` | RSL | 投机 load：可浮动、可上提，读到非法值得 poison。何时允许见 [semantics.md](semantics.md#load固定与投机) |
| `Transmute`、`Transmute.spec` | RS | 按字节重解释；固定版非法值 UB，投机版得 poison；需要两边的布局孔位 |
| `SlotLive`、`SlotDead` | RSL | 开始／结束动态存储寿命；Live 初始未初始化，无清零、drop；move 不等于 Dead。依赖交接后不产生机器指令 |
| `VariantOfAt[P]`、`FieldLoad[P, i]` | RS | 直接从内存读 enum 变体或单个字段，避免整值 load 后只用投影 |
| `CopyMem[Psrc→Pdst]` | RSL | 类型化拷贝（`copy_nonoverlapping` 等），读一个分区、写另一个 |
| `Reborrow{kind}`、`EndBorrow` | RS(L) | 建立／结束有源契约依据的借用 scope 和凭据，可选拆子分区；不分配、写回或 drop，复用父链仍保留作用域 |
| `MemMerge` | RSL | 同一控制点的稀疏分区版本打包；无指令，不等于 Phi、跨线程 join 或 fence |
| `Escape` | RSL 的分析转换 | 精确分区知识保守化，重建到 Unknown 的依赖；实际逃逸操作另有节点，不独立产生效果或 UB |
| `Atomic*`、`Volatile*` | RSL | 访问用地址所属分区；排序规则见 [memory-partitions.md](memory-partitions.md#规则) 规则 7 |
| `Fence{ordering}` | RSL | Release/AcqRel/SeqCst 以 `Unknown` 和全部 `Unique`、`Local`、`Heap` token 为输入 |
| `Alloc{size, align, zeroed}`、`Dealloc` | RSL | 来自全局分配器 lang item；`Alloc` 的结果是新的 `Heap` 分区。配对且结果未逃逸时整体删除（W07 中 O3 删掉了 `Box` 分配） |

## 调用与孔位

| 节点 | 级别 | 说明 |
|---|---|---|
| `Call(InstanceKey)` | SL（R 级用于不依赖参数的被调者） | 结果：正常控制、可选 unwind 控制、各分区新效果、返回值 |
| `CallIndirect`、`CallVirtual{slot}` | RSL | fn 指针；`dyn` 方法表槽位 |
| `TraitCall{item, args}` | R | 依赖 `Param` 的 trait 方法调用；特化时解析成 `Call` |
| `Const(VTable K)` | RS(L) | 整张 vtable 是常量（`Hole::VTable` 的答案）；`CallVirtual` 的 vtable 输入是它时，S 级查答案表直接去虚化 |
| `SizeOf(T)`、`AlignOf(T)`、`NeedsDrop(T)`、`TypeName(T)`、`TypeIdOf(T)` | R | 特化时变常量 |
| `Intrinsic{id}` | RSL | 纯 intrinsic 是值节点；有副作用的挂效果链 |
| `Opaque` | RSL | black_box 等的值恒等与优化策略边界；不提供 definedness，不是通用内存屏障。输入求值、操作要求和后端屏障范围须单独声明 |

## 检查、drop、unwind

| 节点 | 级别 | 说明 |
|---|---|---|
| `Check{kind}(cond)` | RS | `Bounds`/`Overflow`/`DivZero`/`Misc`；产出 ok 与 fail 两个控制，fail 进冷区域调用 panic；L 级变成 `If` + 冷标记 |
| `DropValue(v)` | RS | 消费一个 SSA 值；`NeedsDrop=false` 时删除；L 级变成 drop glue 调用（值先落到槽）或内联 |
| `DropInPlace[P](ptr)` | RSL | 内存中的值 |

两种 drop 的结果都是 `(Control, Control?, Effect…)`：正常出口、可选 unwind 出口（drop glue 摘要 `may_unwind=false` 时没有）和效果。cleanup 区域里的 drop 再 unwind 进 `Terminate`。

## 语义要求与编译期凭据

完整契约见 [requirements-and-facts.md](requirements-and-facts.md)。以下要求固定在执行位置，
不返回新业务值，不读写程序内存；迁移期可复用效果 token 排序。它们在声明的
S→L 擦除边界按 refinement 处理，不应降低成逐字段检查分支。

| 操作／记录 | 级别 | 说明 |
|---|---|---|
| `RequireDefined(v)` | RS | 当前语义组成不得含 poison；活动 payload、MaybeUninit、空数组按各自契约解释 |
| `RequireValid<T>(v)` | RS | 满足 producer 指定且已建模的类型有效性要求；不能由 contains_poison 代替 |
| `Assume(p)` | RS | p 必须 defined 且为真，否则 UB；没有运行时 fail 出口 |
| `VariantWitness(value, variant, witness)` | RS，L 保留对应证明依赖 | 编译期变体凭据，不产生值、指令或整树 Refine |
| 标量谓词／访问凭据 | RSL 的分析信息 | 范围、非空等值事实与内存状态相关访问条件分开；消费者引用其作用域和来源 |

`Proven` / `Assumed` 只分类事实来源，两者均受控制范围和动态值实例约束。
旧 `Refine(ControlOnly)` 的必要约束迁移到实际操作；旧 `Refine(Assumed)` 的 UB
职责迁移到独立要求。凭据不是可执行 opcode，最终存储不要求每项占一个节点。

## 不存在的节点

- 不再新增通用的 `ObserveValue` / `Refine` 值包装节点；现有实现按上述职责迁移。
- 不另增含义不明的 lifetime 提示：地址存储按 SlotLive／SlotDead 明确建模，值的活跃区间另由 SSA 推导（[ownership-drop-unwind.md](ownership-drop-unwind.md#栈槽活跃区间)）。
- 没有 `memcpy` 形式的 move：move 是值转发。

Effect、Escape、借用 scope、Unreachable、Opaque 和 cold 等的完整分类与错误边界见
[constraints-and-effects.md](constraints-and-effects.md)。没有机器值结果不等于可以无条件删除。
