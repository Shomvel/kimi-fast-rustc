# RS5 借用权限与跨函数语义传播

对应 issue #5 方向 5。事实账本条目 RF20–RF25（[facts.md](facts.md)）。
本方向消费 [RS4](rs4-memory-and-summaries.md) 的位置模型、`may_write` 查询、流敏感逃逸与调用摘要，
并向其提供参数权限、调用点只读实参与返回别名三类事实。设计目标见
[memory-partitions](../../../design/ir/memory-partitions.md)；本文给出它在单条 effect 链上的最小可交付子集。

## 目标

1. 参数访问权限 `Frozen`/`Unique` 的资格与 rustc 1.94.1 对直接参数发出的
   `noalias`/`readonly`/`dereferenceable` 一致，作用域为该实例函数体的一次执行（参数 protector）。
2. 内联把被调者的参数权限转成有界作用域；`UnsafeCell`、裸指针派生、调用捕获与逃逸按明确规则使权限失效。
3. 返回摘要覆盖返回实参、实参字段、实参地址（返回别名）、常量与小纯表达式；内联按语义收益排序。
4. 已知 vtable 的虚调用改写为直接调用并进入内联。

## 基线现状（`37cea66`）

| 项 | 证据 |
|---|---|
| `&T`/`&mut T` 为 `TypeShape::Reference`；`Box` 为普通 Adt | `kfrc-body/src/types.rs:69-86` |
| 参数指针不是 `MemoryFacts` 的根，经参数地址的访问一律 Unknown；文件头声明引用类型本身不产生事实 | `analysis/memory.rs:5,38-53` |
| 全仓无 `is_freeze`/`is_unpin` 使用 | grep |
| converter 调用 `fn_abi_of_instance`，`record_abi`/`pass_mode` 丢弃 `ArgAttributes` | `rustc_codegen_kfrc/src/abi.rs:34-58,141-201`；`kfrc-body/src/body.rs:159-164` |
| 虚调用：拆宽指针，数据指针按 `*const ()` 传，`IndexAddr` + `LoadValue` 读槽，`CallIndirect` | `convert/calls.rs:131-144`；`convert/trait_object.rs:30-162` |
| vtable 为函数私有、只读、带 relocation 的 `DataBinding::Local` 对象；槽读被标 `Const`，无 pass 折叠 | `rustc_codegen_kfrc/src/constants.rs:174-247`；`transforms/memory/partition.rs:56-64` |
| 内联：FIFO 工作表加硬预算（深度 4、2000 节点、32 调用），单轮；`plan` 无收益项；持有 Local data object 的被调者一律拒绝 | `transforms/ipo/mod.rs:33-47,100-104`；`ipo/plan.rs:21-27,69-87`；`rustc_codegen_kfrc/src/lib.rs:484-540` |
| 返回摘要只有“单路径、无其他节点、返回某个参数”；摘要缓存为每个调用者一次 | `ipo/identity.rs:38,94-174` |
| 可得的被调者图：本 crate 的 mono item；`#[inline(never)]` 与引用其他 CGU internal 符号的图被屏蔽 | `lib.rs:119-150,404-422` |

## rustc 1.94.1 的参数事实

`arg_attrs_for_rust_scalar`（`rustc_ty_utils/src/abi.rs:273-371`）配合
`ty_and_layout_pointee_info_at`（`rustc_middle/src/ty/layout.rs:1012-1150`）：

| 直接参数类型 | NoAlias | ReadOnly | dereferenceable | captures |
|---|---|---|---|---|
| `&T`，`T: Freeze` | 是 | 是 | `size_of::<T>()`；unsized 为 0 | address, read_provenance |
| `&T`，`T: !Freeze`（`Cell`、`RefCell`、含 `UnsafeCell` 字段的类型、`dyn`） | 否 | 否 | 0 | — |
| `&mut T`，`T: Unpin` | 是（`-Zmutable-noalias`，默认开） | 否 | `size_of::<T>()` | — |
| `&mut T`，`T: !Unpin` | 否 | 否 | 0 | — |
| `Box<T>`，`T: Unpin`，全局分配器 | 是（`-Zbox-noalias`，默认开） | 否 | 0 | — |
| 裸指针、`NonNull`、任何返回值 | 否 | 否 | 0 | — |

`is_freeze`/`is_unpin` 是整类型、浅层的 auto-trait 查询。Miri 1.94.1 的 SB 与 TB 在 FnEntry
protector 下对 `&T: Freeze` 与 `&mut T: Unpin` 给出一致结论，kfrc 只使用两者都成立的子集
（[memory-partitions](../../../design/ir/memory-partitions.md#分区的两类地位) 的验证要求）。
MIR 中的 `Retag` 只在 `-Zmir-emit-retag` 下出现，kfrc 按类型谓词重算权限。

**事实来源（决定 D1）。** rustc 只在 `optimize != No` 时计算 `frozen`/`unpin`
（`layout.rs:1039-1046`）。kfrc 的 semver 验收与多数脚本使用 `-Copt-level=0`，该配置下
`ArgAttributes` 不含这些属性；性能脚本另有 `-Copt-level=3` 一臂。本方向采用
**rustc-O 契约**：kfrc 使用 rustc 1.94.1 在 `optimize != No` 时为同一实例、同一 `-Z` 开关发出的事实，
由 converter 调用同一组 rustc 谓词计算；一个 `-Copt-level=1` 守卫测试逐参数断言 kfrc 的权限被
`fn_abi_of_instance` 的 `ArgAttributes` 蕴含（N5.14）。rustc 源注释说明 O0 关闭这些属性是为了节省
查询与 LLVM 时间（`layout.rs:1036-1038`）。

## 权限模型

```rust
pub enum ArgPermission {
    None,
    Frozen { dereferenceable: Option<u64> },  // &T, T: Freeze
    Unique { dereferenceable: Option<u64> },  // &mut T, T: Unpin, -Zmutable-noalias
}
// FunctionGraph.arg_permissions: Vec<ArgPermission>，与 FunctionAbi 分开存放，
// 使函数指针调用与实例 ABI 的相等性比较保持不变。
// callee_permissions：与 callees 平行，记录直接调用目标的参数权限。
```

- **生产者：** converter 对 `DefinitionArgs.parts[i].ty` 本身为 `ty::Ref` 且 PassMode 为
  Direct/Pair 的参数计算权限；其余为 `None`。R 图不带权限，特化出 S 实例图后由 converter 填入；
  `IsFreeze`/`IsUnpin` 答案与两个 `-Z` 开关进入 RS6 的身份键。开关 `KFRC_ARG_PERMISSIONS=off|rustc-o` 用于 A/B。
- **作用域：** 未内联的实例图中，每个内存节点都处在本次调用的执行期间内；PoC 为 `-Cpanic=abort`，
  出口只有 Return 与发散，发散路径无需关闭作用域。

**消费（查询式，接入 RS4 的 `may_write(effect, access)`）：**

| 编号 | 规则 | 依据 |
|---|---|---|
| Q1 | 地址根为 Frozen 参数 p 的访问：函数体内任何调用、store、`SetVariant`、copy 对它回答“不写” | `noalias + readonly` 覆盖整个调用，含被调者再调用的函数；与 p 是否逃逸无关 |
| Q2 | 两次同地址、同类型 `LoadValue`，前者支配后者、其间效果节点对该访问均不写：后者复用前者的值 | 作为 LP3 `available_snapshot` 的“跨越不写本访问的效果节点”扩展 |
| Q3 | 直接调用的第 i 个实参在 `callee_permissions` 中为 `Frozen{Some(n)}`：本次调用不写该实参前 n 字节内以常量结构化路径到达的位置 | 调用点属性；逃逸状态保持不变（被调者可留下只读 provenance） |
| Q4 | Unique 参数 p 的访问与效果节点 E 不相交：E 的根是另一已识别根；或 E 的根为 Unknown 且 p 在 E 之前未逃逸；或 E 为调用、实参不含基于 p 的指针且 p 未逃逸 | `&mut T: Unpin` 的 protector；新增或上移写须被原程序同一分区的写支配（激活写规则） |

**内联（决定 D2）。** v1：`splice` 不复制被调者的 `arg_permissions`；被调者在作为 provider 之前
已完成依赖权限的改写。v2：侧表作用域
`PermissionScope{kind, root, parent}` 与 `scope_of: NodeId → ScopeId`，`splice` 为有权限的形参建立作用域
并登记被复制的内存节点与调用；同一作用域内、支配关系成立的两次访问属于同一次激活，Q1/Q2 适用。
任何 pass 克隆或替换内存节点时复制或删除登记；verifier 检查作用域树与 `root` 的支配关系；
S→L 丢弃全部作用域。设计中的 Reborrow/EndBorrow 节点在 RS4.9 引入多效果链时由侧表物化。

**失效与排除：**

| 情况 | 处理 |
|---|---|
| `&Cell`/`&RefCell`/`&Mutex`/含 `UnsafeCell` 字段的 struct | `None`（整类型、浅层） |
| `&&Cell` | 外层 `Frozen{8}`；经它读出的内层指针为 Unknown 根 |
| `&mut T: !Unpin`、`&dyn`、`&mut dyn` | `None` |
| `Box<T>`、聚合实参中的引用、返回值 | 首版 `None`（延后表） |
| 局部重借用、两阶段借用 | 权限从调用进入点开始；局部重借用不建作用域 |
| `&mut` 派生的裸指针（`PointerCast`） | 同根，属于同一权限；`IntCast` 到整数即逃逸 |
| Unique 的 p 逃逸 | 首版整函数失效；RS4.3 后按捕获点 |
| `-Zmutable-noalias=no`、`-Zbox-noalias=no` | 对应权限为 `None` |

## 返回摘要与收益驱动内联

```rust
pub enum ReturnSummary {
    Argument(usize),
    ArgumentField { arg: usize, path: Vec<u32> },
    ArgumentAddress { arg: usize, path: Vec<Step> },   // 返回别名：&self.x、as_ptr
    Constant(ConstId),
    PureExpr { nodes: Vec<TemplateNode> },             // ≤ K 个纯值节点
}
```

资格沿用 identity 的单路径检查，并要求无内存操作、无调用、无可陷入节点、无 data object。
`ArgumentAddress` 让调用者看到“基于实参根的 `FieldAddr`”，实参地址不再流入未知调用，
RS4 的提升与跨调用转发由此获得输入。只含一次基于实参地址的 `LoadValue` 的访问器作为
“平凡被调者”走普通 `splice`，免除深度与调用数预算。摘要缓存提升为 crate 级，按符号计算一次，
RS6 身份键落地后按键缓存；机器码缓存键包含读取的摘要哈希。“返回由实参决定的变体”
（`checked_add`、`Ordering`）需要分支，由收益驱动内联处理。

**收益驱动内联。** 成本取 `plan` 已算的 `cloned + extra`。收益在已优化的被调者图上做一次
以调用者实参已知形态为种子的前向抽象求值：常量、已知变体、`Aggregate`、带 vtable 的 `WidePtr`
使被调者中的 `Compare`/`If`/`VariantSwitch` 折叠；调用者只用结果的部分字段（RS2 需求）；
`CallIndirect` 解析到 `SymbolAddress`；实参为只流向本调用的 `LocalSlot` 地址；被调者为 internal 且只有一个调用点。
工作表按（收益 − 成本）降序、`NodeId` 升序排列，`InlineBudget` 保留为硬上限；冷调用点（RS7.5）
只做摘要折叠，`#[inline(always)]` 例外。所有常数经 A/D/B 标定后写入实验记录。

## 去虚化

| 编号 | 缺口 | 做法 |
|---|---|---|
| E1 | Const 分区 load 不折叠 | 新 pass：根为不可写、非 TLS 的 `DataObject`，地址为常量结构化偏移；指针类型且恰有一条 relocation 起于该偏移 → `SymbolAddress(target, addend)`；整数类型、无 relocation 重叠且字节已初始化 → `Const`。`DataObject` 增加初始化掩码（`constants.rs:216-218` 现把未初始化字节读成 0）；v1 只对 vtable 放开 |
| E2 | `CallIndirect(SymbolAddress f)` 保持间接 | 改写为 `Call(CallTarget{f, abi})`；dyn drop 的判空（`trait_object.rs:210-217`）配 instcombine：已定义符号地址非 0 |
| E3 | 去虚化后数据指针 `*const ()` 与形参 `&Concrete` 类型不等 | 改写时对第 0 实参插 `PointerCast`；ABI 不等（按值 self 的 `VTableShim`）时只保留直接调用 |
| E4 | 单轮流水线 | {内联新调用 → seam 优化（含 E1/E2）} 至多 N 轮的有界不动点，共享一份 `InlineBudget` |
| E5 | 持有 vtable/匿名常量的被调者不可内联（决定 D3） | v1：splice 克隆不可写、非 TLS、非 static 的 Local 对象并改名，同一目标文件内按内容去重；长期：按 `(Ty, trait_ref)` 内容寻址的全局 vtable 符号（需要 ELF COMDAT/隐藏弱定义，与 RS6 共用） |

vtable 来自内存（`&[&dyn Shape]`）时保持 `CallIndirect`；闭世界同槽摘要并集与堆分配消除在延后表。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS5.0 基线 | `InlineStats`/`IdentityFoldStats` 的跳过原因直方图（Missing/Unsupported/DataConflict/TypeConflict/AbiMismatch/Budget）、`CallIndirect` 数、`const_loads`；semver/regex 增加 `-Zshare-generics=no` 一臂（决定 D4） | `lib.rs`、`tests/p3-semver/` | 直方图与 O0 可得性损失由脚本生成 | — |
| RS5.1 权限事实 | `ArgPermission`、`arg_permissions`、`callee_permissions`；开关；dump；O1 守卫 | `kfrc-son/src/graph.rs`、`rustc_codegen_kfrc/src/abi.rs`、`convert/calls.rs`、`verify.rs` | 守卫在 fixture 与 regex 上通过；开关 on/off 产物逐字节相同（尚无消费者） | — |
| RS5.2 Frozen 消费 | 参数根；Q1、Q3；Q2 并入 `available_snapshot`；内联 v1 | `analysis/memory.rs`、`transforms/memory/forward.rs`、`pipeline.rs` | P5.1–P5.3；N5.1–N5.3、N5.5、N5.7、N5.10、N5.12；Miri SB/TB 对用例无报错；`frozen_loads_reused` 在 regex 上为正 | RS5.1；RS4 的 `may_write` 接口；LP3 合入 |
| RS5.3 返回摘要 | 五种 `ReturnSummary`；crate 级缓存；平凡访问器 | `ipo/identity.rs`、`ipo/types.rs`、`lib.rs` | P5.6、N5.8；`InlineBudget` 消耗不增 | — |
| RS5.4 去虚化 | E1–E5（v1） | `transforms/memory/constant.rs`、`transforms/ipo/devirt.rs`、`ipo/plan.rs`、`ipo/splice.rs`、`instcombine.rs`、`kfrc-body/src/body.rs`、`constants.rs`、`lib.rs` | P5.7、N5.13；W07 `local_devirt` 形状去虚化并内联；`devirtualized / virtual_calls` 有数 | — |
| RS5.5 收益内联 | 调用点抽象求值、优先队列、冷调用点规则、决策日志 | `ipo/mod.rs`、`ipo/plan.rs`、`ipo/benefit.rs` | P5.8；`nodes_added` 不增前提下折叠与去虚化增加；决策日志跨运行哈希一致 | RS2 需求分析；RS7.5 |
| RS5.6 Unique | Q4；调用实参值字段可达指针 | `analysis/memory.rs`、`forward.rs`、RS4.5 的 DSE | P5.4、P5.5；N5.4–N5.7；Miri SB/TB | RS4.3、RS4.6 |
| RS5.7 内联作用域 v2 | `PermissionScope` 侧表；splice 登记与嵌套；verifier；S→L 丢弃 | `ipo/splice.rs`、`graph.rs`、`verify.rs`、`lower_layout.rs` | N5.6–N5.8 内联后仍正确；越界成员关系被 verifier 拒绝；内联后新暴露的 Frozen 复用有计数 | RS5.2、RS5.6 |

## Fixture 与负例

| 编号 | 程序 | 期望 |
|---|---|---|
| P5.1 | `fn frozen(x: &u32, f: fn()) -> u32 { let a = *x; f(); a + *x }` | S `LoadValue` 2 → 1 |
| P5.2 | `fn s2(s: &[u64], f: fn()) -> u64 { let a = s[0]; f(); a + s[0] }` | 第二次读复用 |
| P5.3 | `fn d(x: &mut u32) -> u32 { *x = 3; observe(&*x); *x }`，`observe` 不内联 | Q3 跨调用转发 3 |
| P5.4 | `fn u(x: &mut u32, y: &mut u32) -> u32 { *x = 1; *y = 2; *x }` | 返回常量 1 |
| P5.5 | `fn uc(x: &mut u32, f: fn()) -> u32 { *x = 1; f(); *x }` | 跨未知调用转发 |
| P5.6 | `fn first(p: (u64, u64)) -> u64 { p.0 }`；`fn fref(s: &S) -> &u64 { &s.b }`；`fn seven(_: &S) -> usize { 7 }` | 摘要折叠，不消耗预算 |
| P5.7 | `call(&dyn Value)` 被 unsize `&Concrete` 的调用者调用；`destroy(Box<dyn Value>)` 已知 vtable | L 无 `CallIndirect`；drop 槽判空折叠 |
| P5.8 | `fn pick(flag: bool, a: u64, b: u64) -> u64 { if flag { heavy(a) } else { b } }` 以 `false` 调用 | 收益排序优先内联 |

| 编号 | 程序 / 检查 | 期望 |
|---|---|---|
| N5.1 | `fn cell(c: &Cell<u32>, f: &dyn Fn()) -> u32`，闭包写同一 `Cell` | 重读；权限 `None` |
| N5.2 | `&Mixed { n, c: Cell }` | 权限计数为 0，与 rustc 属性一致 |
| N5.3 | `&RefCell<u32>`，回调 `borrow_mut()` 写入 | 观察到写 |
| N5.4 | `&mut Pinned`（`PhantomPinned`，自引用裸指针被写） | 无 Unique；重读 |
| N5.5 | `let p = x as *mut u32; *x = 1; unsafe { *p = 2 }; *x` | 返回 2（TB 下有定义） |
| N5.6 | `v.push(v.len())` 等两阶段借用，`set` 内联 | 与 LLVM 一致 |
| N5.7 | `cap(&a)` 把地址存入 `static mut`，随后 `reader()`；`cap_mut(&mut a); writer(7); a` | 5；7 |
| N5.8 | `fn get(x: &mut S) -> &mut u32 { &mut x.a }` 内联后经返回引用写、经别的路径读 | 作用域外无事实 |
| N5.9 | 被内联者含 panic 发散路径；`-Cpanic=unwind` 仍被拒绝 | 作用域无需伪造出口 |
| N5.10 | `fn boxed(b: Box<u32>, f: fn()) -> u32` | 无事实 |
| N5.11 | `-Zmutable-noalias=no` | Unique 全部为 `None` |
| N5.12 | 同一 `fn g<T>(x: &T, f: fn())`，`T = u32` 与 `T = Cell<u32>` | 前者 Frozen，后者 None；R 图无权限 |
| N5.13 | vtable 来自内存；`static mut TABLE: [fn(); 2]`；`dyn FnOnce` 按值 self | 保持 `CallIndirect` 或只直接化不内联 |
| N5.14 | `-Copt-level=1` 守卫 | kfrc 权限 ⊆ rustc `ArgAttributes` |

所有用例源码另跑 Miri SB 与 TB，只使用两种模式都有定义的输入。

## 指标

有权限的参数数及其占 rustc-O `NoAlias` 参数的比例；`frozen_loads_reused`、`calls_crossed_by_forwarding`、
`unique_loads_forwarded`、`dead_stores_removed`；按 `ReturnSummary` 种类的折叠数；内联决策数、收益分布、
跳过原因直方图；`virtual_calls`、`devirtualized`、`devirt_inlined`、`const_loads_folded`；
`identity_s`/`inline_s`/`post_inline_optimize_s` 窗口指令；R/S/L 节点、L load/store、机器指令、spill、`.text`；
semver/regex runtime，含 `-Zshare-generics=no` 一臂。
