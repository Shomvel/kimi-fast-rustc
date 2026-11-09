# RS2 聚合／枚举需求传播与大值表示

对应 issue #5 方向 2。事实账本条目 RF10–RF14（[facts.md](facts.md)）。
本方向以 LP3 的实现为基线：RS2.0 负责验收 LP3，RS2.1 起在其上扩展。

## 目标

1. 只为执行消费者真正需要的字段建 Phi、做 load；固定下标数组、活跃变体与跨调用边界同样适用。
2. 每个值等价类（Phi 网、Return、调用结果、参数、move 链）在一处选定表示：
   标量叶子、私有槽，或借用 ABI 提供的存储。
3. 整值跨 ABI 时直接构造到目标位置，按真实冲突决定拷贝；只用部分字段时只读这些字段。

## 基线现状（`37cea66`）

| 项 | 证据 |
|---|---|
| 聚合 Phi 为 tag 与**全部**变体的每个叶子各建一个 L Phi，非活跃变体填 `Undefined` | `lower_layout/values.rs:157-170,264-329,447-500` |
| 数组走 product 分支，`[T; N]` 的 Phi 建 N 份子树（全 Repeat 入边除外） | `kfrc-body/src/oracle.rs:197-199`；`lower_layout/repeat.rs:48-76` |
| 复合参数在入口整树快照；复合调用结果整树解码 | `lower_layout.rs:227-237,292-305`；`lower_layout/memory.rs:143-179` |
| 复合实参、Return、Opaque 输入各自新建槽并整树写入 | `lower_layout/memory.rs:244-279` |
| sret 指针只存在于 x86；Indirect 返回先写临时槽，再 `copy_bytes` 进 sret | `kfrc-x86/src/lower/abi.rs:135-143`；`kfrc-x86/src/lower.rs:896-905` |
| Pair 与 Direct 聚合（含 `&str`、`&[T]`）两个方向都经栈槽往返 | `kfrc-x86/src/lower/abi.rs:182-235`；`lower.rs:720-765,907-933` |
| L 的 `raw_copy` 一律调用 `memmove`；`CopyNonOverlapping` 一律调用 `memcpy` | `lower_layout/maybe_uninit.rs:34-74`；`lower_layout/semantic_memory.rs:213-265` |
| 移出内存局部写入 `Undefined`，机器码中为写 0 | `build/semantic.rs:160-171`；`build/semantic/places.rs:135-155`；x86 `lower.rs:362` |
| PassMode 来自 rustc `FnAbi` | `rustc_codegen_kfrc/src/abi.rs:34-58,159-201` |

kfrc 的输入 MIR 一律为 `-Zmir-opt-level=0`（`scripts/poc-validation/*`），
rustc 的 `DestinationPropagation`、`CopyProp`、`ScalarReplacementOfAggregates`、`GVN`
都不运行，NRVO 与 move 链消除由 kfrc 完成。

## LP3 的实现与剩余项

LP3（未合入 main 的 LLVM-parity 工作，candidate23/24）已实现：

- 字段需求 worklist：`Demand{None, All, Fields}`，键为 Field、VariantField、Tag；内部 `Parts::Omitted`
  与 IR `Undefined` 分开，Omitted 进入物化即报错；
- 按需叶子 Phi、按需 load、参数与调用结果按需读取；循环 Phi 先发布占位再填充；
- 连续同地址 `LoadValue` 复用 owned 快照；
- `owned_transport`：无回边、入边为构造器、只流向 Return/StoreValue 的 enum Phi 闭包使用私有槽，
  两阶段 staging；
- LP1 的直接 tag 标量。

candidate23 的 `from_str`：L Phi 695 → 571，Undefined 473 → 363，Load/Store 180/80 不变。

LP3 文档列出的剩余项即本方向 RS2.1–RS2.7 的起点：数组、Repeat、MaybeUninit、调用实参、
完整 ABI 返回按 `All` 处理；私有槽只覆盖 enum Phi 到 Return/StoreValue；消费者处读取私有槽
尚未实现；拷贝消除尚未开始。

## 表示决策

在需求冻结之后、`expand` 之前，新增 `lower_layout/representation_plan.rs`，一次性为每个值等价类
产出 `Repr`，`Field`、Phi、load 与 ABI 物化都查询这张表：

| `Repr` | 含义 |
|---|---|
| `Leaves` | 按需标量叶子（LP3 现有表示） |
| `Owned{class}` | 类内所有值共用一个私有槽 |
| `Borrowed{ptr}` | 直接使用 ABI 提供、本函数独占且只读的存储：callee 侧 Indirect 参数，调用者提供的 sret 或结果槽 |

**等价类**用并查集合并：Phi 与值入边；Return 值与 sret；调用结果与其目标；末次使用的
`InsertField` 与 base；构造器与唯一整体使用者；编码恒等的重包装
`MakeVariant(v', VariantField(v, i))`（W01）。合并前做活跃区间干涉检查，冲突时拆类并在边上插拷贝。

**判据**按类计算，先给出两种表示的代价表达式，再用 LP0 归因数据校准：

| 输入 | 偏向 |
|---|---|
| 需求叶数 D、逻辑叶数 L、物理字数 W | D ≪ W 选 Leaves；D 接近 L 且 L > W（变体重叠）选 Owned |
| 值入边数 E、回边数 B | Leaves 搬运约 D×E 次；Owned 约 E 次块拷贝加消费者 load |
| 消费者种类 | 整体消费者均为 Indirect ABI 时选 Owned；只有 Field/Tag 时选 Leaves |
| 源种类 | 构造器可构造到目标；Indirect 参数为 Borrowed；`LoadValue` 必须在原读取点快照 |
| 数组长度、Repeat | 长数组只有字段需求时用稀疏 Leaves，其余用 Owned |
| PassMode 为 Pair/Direct/Cast | Leaves，按车道搬运 |

固定字节阈值只作为参考数据（LLVM SROA 的 1024、`max_unrolled_memory_bytes = 128`）。

**私有槽规则。** 构造器入边只写活跃字段与 tag；Owned/Borrowed 入边整块拷贝；
仅当同一边上某个拷贝的源是另一个拷贝的目标时才 staging。私有槽在拷入完成后到类死亡前
只读且不逃逸，因此 `Field`/`VariantField`/`Discriminant` 可降为 `Load(slot + off)`，
effect 取类定义点之后的 token，分区为 Local；L 末尾重算 `MemoryFacts` 时须识别这类槽。

**ABI 构造到目标。**

1. Return：L 暴露 sret（`ReturnSlot`），Return 直接编码进 sret；Return 是 Owned 类唯一整体汇时，
   类槽就是 sret。x86 在源等于 sret 时省去 `copy_bytes`。
2. Indirect 调用结果：结果槽改由 L 分配，可作为类槽。与调用者 sret 或某个 Phi 类合并时，
   要求目标槽在调用期间（正常与 unwind 出口）不持有其他活值、实参不可达、地址未逃逸。
3. callee 侧 Indirect 参数：`Borrowed{ptr}`，删除入口整树快照；参数内存由 callee 独占
   （`rustc_middle/src/mir/syntax.rs:814-819`，`rustc_target/src/callconv/mod.rs:400-417`）。
4. caller 侧 Indirect 实参：值为该槽的末次使用（正常与 unwind 出口）且槽未逃逸时直接传类槽，
   等同 rustc 对 `Operand::Move` 的处理（`rustc_codegen_ssa/src/mir/block.rs:1206-1218,1632-1652`）；
   callee 摘要声明不写该参数（RS4/RS5）时也直接传；其余情况拷贝。判据使用 SSA 活跃性。
5. Pair、Direct 聚合与 Cast：L 按车道产出标量，去掉寄存器与槽之间的往返。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS2.0 LP3 验收 | 以冻结 candidate24 为准：需求格单调性与 Omitted 外泄单测；`owned_transport` 的真实代价（Phi、Undefined、Load、Store、memmove 调用、帧）对照 candidate23；补齐 `LayoutStats` | `lower_layout.rs`、`demand.rs`、`owned_transport.rs` | 三臂差分通过；`from_str` L 计数表由脚本生成；runtime 回退项记录原因并转入 RS2.3 | LP3 合入 |
| RS2.1 按车道传 Pair/Direct；小拷贝展开 | Pair、Direct 聚合与 Cast 不经栈槽；L `raw_copy` 与 copy 小尺寸改为 `CopyBytes` 节点交给 x86 展开 | `lower_layout/memory.rs`、`maybe_uninit.rs`、x86 `lower.rs`、`lower/abi.rs` | `len2(&str, &str)` 参数与返回不经栈；memmove 调用数下降 | LP2 合入 |
| RS2.2 表示决策与私有槽 | `representation_plan.rs`；`owned_transport` 推广到 struct、数组、带回边的类；staging 只在冲突时；构造器直接写类槽；末次使用的 `InsertField` 原地写 | 新文件，`owned_transport.rs`、`values.rs`、`memory.rs` | `from_str` 的 `Result<Version, Error>` 链在 L 无叶子 Phi；循环交换负例通过；决策直方图进入统计 | RS2.0 |
| RS2.3 ABI 构造到目标 | 上节 1–4 | `memory.rs`（`materialize_before`/`decode_boundary`）、`lower_layout.rs::run`、x86 `lower.rs`（Call/Return）、`lower/abi.rs` | W09 `ret_big` 结果槽即 sret；`pass`/`fwd` 至多一次块拷贝；W01 错误路径零拷贝；semver 34/34 | RS2.2；RS4.6 的写摘要（第 4 条第二种情形） |
| RS2.4 私有槽字段读取 | Owned/Borrowed 值的字段读取在消费者处；switch 分支内 payload 读取下沉 | `values.rs`、`semantic_memory.rs`、`analysis/memory.rs`、`transforms/memory/partition.rs` | 大 enum 在 switch 内只读需要的 payload，无额外 tag 守卫 | RS2.2 |
| RS2.5 数组与 Repeat 需求 | 数组不再强制 All；稀疏数组表示与 `AllExcept` 需求；Repeat 元素需求取各下标需求的并集；带 override 的 Repeat Phi | `demand.rs`、`values.rs`、`repeat.rs`、`memory.rs` | `[x; 256]` 加 `a[7] = y` 的 L 节点数与 256 无关；`[T; 0]`、ZST 元素、MaybeUninit 元素负例通过 | RS2.0 |
| RS2.6 变体需求细化 | 按边已知变体填 Phi（复用 `analysis/refinement.rs` 的作用域查询）；已证明 case 内的 `VariantField` 不再附加 Tag 需求；不可居留变体（`Result<T, Infallible>`）删除需求，前提是值 defined | `demand.rs`、`values.rs`、`analysis/refinement.rs`（只读） | 非活跃 payload 无 Phi/load；循环证明刷新与 sibling 跨合流负例通过 | RS2.0 |
| RS2.7 跨调用需求 | callee 的 `ParamDemand[i]` 与 callee 自身 L 共用同一需求函数，并断言 L 读取不超出摘要；caller 的 Indirect 实参只写需求字段；摘要进入机器码缓存键；返回需求特化只用于所有调用者已知的 local 函数 | `transforms/ipo/summary.rs`（RS4.6 新增）、`demand.rs`、`rustc_codegen_kfrc/src/lib.rs` | 未内联的 `fn g(b: Big) -> u64 { b.n }` 的调用者只写 `n`；LLVM 编译的 callee 保持 All | RS4.6 |
| RS2.8 拷贝消除收尾 | 编码恒等重包装合并；删除移出时的 `StoreValue(Undefined)`（从 poison 到旧字节的 refinement）；局部 Const/PtrOffset 去重 | `representation_plan.rs`、`semantic_memory.rs` | 移出不再生成写 0；W01 错误路径零拷贝保持 | RS2.3 |

建议顺序：RS2.0 → RS2.1 → RS2.2 + RS2.3（一起落地，避免“有槽却仍拆开再组装”）→ RS2.4 →
RS2.5 → RS2.6 → RS2.7 → RS2.8。栈槽按区间共享归 RS7.1。

## 证明义务与负例

| 编号 | 义务 | 负例 |
|---|---|---|
| N2.1 | 快照时点：`LoadValue` 的值取自原读取点；延后读取私有槽仅在拷入完成且槽只读时成立 | `unsafe fn f(p: *mut Big, q: *mut Big) { let a = *p; (*q).x = 1; a.x }`，`p == q`，含循环版本 |
| N2.2 | Move 后源可被重新初始化；Owned 类使用独立私有槽 | `let a = s.big; s.big = new(); use(a)`，`s` 曾被借用；`x = f(x)`，`x` 为 Indirect |
| N2.3 | 部分初始化：构造到目标只写已定义字段 | `?` 早退发生在部分字段赋值之后 |
| N2.4 | padding 与非活跃 payload 只以表示拷贝搬运 | `Result<u8, [u64; 4]>` 的 Phi 经 Return 给 LLVM 编译的调用者；`(u8, u64)` |
| N2.5 | kfrc 自己生成的槽间拷贝在可能重叠时使用 memmove 或证明不相交 | 大 enum 的循环交换；`Some(make())` 与 `make()` 合并到同一槽 |
| N2.6 | 取地址后类退回保守内存，L 重算 `MemoryFacts` | `let v = big(); opaque(&v as *const _); v.x` |
| N2.7 | 结果槽合并在 unwind 出口上保持 cleanup 所需的值 | `x = f()`，`x: Drop`，`f` 写 sret 一半后 panic（随 RS7.6 启用） |
| N2.8 | MaybeUninit 只做原始运输 | `[MaybeUninit<bool>; 4]` 带部分 override 的 Phi |
| N2.9 | caller 直接传槽时为末次使用或 callee 只读 | W09 `take(a) ^ take(a)`，`take` 修改参数，并与 LLVM 编译的 callee 混链 |
| N2.10 | 跨调用需求一致 | 修改 callee 后缓存键变化，重编调用者 |

## 指标

`LayoutStats` 新增：按类型的值叶 Phi 数、进入 Phi 的 Undefined 数、Omitted 叶数、L Load/Store、
`raw_copy`/copy 调用数与静态字节数、StackSlot 数与帧字节数、`materialize_before` 次数与字节数、
sret 直接构造次数、Indirect 实参直接传槽与拷贝次数、表示决策直方图。机器层：mov 数、
memmove/memcpy 调用数、帧大小、`.text`。runtime：semver parse/req/identifier。
