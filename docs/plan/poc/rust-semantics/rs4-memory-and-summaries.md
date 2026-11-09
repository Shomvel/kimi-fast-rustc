# RS4 字段敏感内存分析与调用效果摘要

对应 issue #5 方向 4。事实账本条目 RF15–RF19（[facts.md](facts.md)）。
引用类型产生的别名事实（`&T`/`&mut T` 的 protector）归 [RS5](rs5-borrow-permissions.md)，
本方向在查询接口上为其预留扩展点。

## 目标

1. 以“分配根 + 字段路径／字节区间”描述内存位置，支持有界 Phi 根集合与按根的流敏感逃逸。
2. 计算调用摘要：读、写、地址捕获、返回别名，正常返回与发散路径分开；未知调用取顶。
3. 让 load/store 转发跨过可证明无关的访问与调用，并支持 DSE、拷贝前递与部分提升。
4. `CopyNonOverlapping` 在访问模型中分为源读取与目标写入，并提供“源与目标不重叠”的操作契约事实。

## 基线现状（`37cea66`）

| 项 | 证据 |
|---|---|
| `MemoryFacts` 为整函数逃逸；根只有 `LocalSlot`、可映射回 local 的 `StackSlot` 与 `SymbolAddress`，单根、无路径 | `analysis/memory.rs:34-95,120-128` |
| Phi、Aggregate、Call 实参、存储地址值、ptr→int、Opaque 均使整个 local 在全函数逃逸 | `analysis/memory.rs:91-93`；测试 `promotion_tests.rs:485,658` |
| `forward` 只处理 control 与 effect 都紧邻同一 `StoreValue`、地址节点相同的整值 store→load | `transforms/memory/forward.rs:58-87` |
| `promotion` 整个 slot 全提升或全不提升；拒绝原因无统计 | `transforms/memory/promotion.rs:162-268,19-29` |
| Local/Const 分区标签只被 verifier 检查，无优化消费者；x86 与解释器不读 | `verify.rs:1402-1462`；`transforms/memory/partition.rs:18-77` |
| 每函数单条 effect 变量，内存节点同时固定在 control 上；解释器与 x86 按 control 执行 | `build.rs:100,137,157`；`graph.rs:237-266`；`interpret.rs:464-466`；`kfrc-x86/src/lower/schedule.rs:24-80` |
| `CopyNonOverlapping` 为单节点、分区固定 Unknown、丢弃元素大小；L 一律调用 `memcpy` | `build/memory.rs:419-441`；`verify.rs:1409-1416`；`lower_layout/semantic_memory.rs:213-265` |
| `typed_swap_nonoverlapping` 生成 4 次字节 copy 加两个 `[u8; N]` 中转 | `convert/intrinsics/swap.rs:50-67` |
| 无调用图、无 SCC 顺序；阶段 1 构建并优化全部 S 图，阶段 2 自顶向下按需编译 | `rustc_codegen_kfrc/src/lib.rs:122-147,166,167-290` |
| 阶段 2 的 seams 不运行 forward/GVN，内联后暴露的内存机会无全函数 pass 处理 | `pipeline.rs:166-233`；`lib.rs:542-551` |
| 唯一的被调者摘要是 identity（返回某个实参） | `transforms/ipo/identity.rs:93-171` |
| `function_callee` 只记录符号与 ABI | `convert/calls.rs:35-42`；`kfrc-body/src/body.rs:151-156` |

kfrc 输入为 `-Zmir-opt-level=0` 的 MIR，rustc 的 MIR `DeadStoreElimination`、`GVN`、`SROA`、
`DestinationPropagation` 都不运行。未取地址的 local 已由 kfrc 的 Braun SSA 构造处理；本方向的
增量集中在取过地址的 local、内联后暴露的形状与跨调用的情形。

## 设计决定：查询式依赖，保持单条 effect 链

| 方案 | 结论 |
|---|---|
| 按分区拆 effect 链（MemMerge、分区 Effect Phi） | 内存节点仍固定在 control 上，拆链本身不带来重排；需要改 promotion、forward、splice、identity、thread_variants、prepare_control、`bypass_fixed` 与 CFG 编辑；Effect Phi 按分区数放大（`from_str` 的 S Phi 中 Effect 已占 43/66） |
| **查询式（采用）** | 保持 IR 契约、解释器、x86 与 verifier 不变；分析给出 Location、根集合、逃逸位与摘要；transform 沿 effect 链有界回溯，跳过可证明不相交的访问 |
| 浮动 load（memory slice） | 需要改解释器执行模型、x86 调度与 verifier 的支配规则；以 LICM/调度的实测收益为前提另行设计（RS4.9） |

收益形态是**删除** load、store 与 copy；x86 按程序序发射，重排本身没有直接收益。
查询所需的 Location、逃逸与摘要也是将来拆链的输入。

## 位置模型

```text
Root     = Local(local) | Argument(param, path) | Global{symbol, mutable, tls}
         | Heap(call)   | Unknown
Path     = [Field(i) | VariantField(v, i) | ConstIndex(k) | DynIndex | Cast(ty)]
Location = { roots: SmallSet<(Root, Path)>（≤ 4，超出为 Unknown）, access: 类型或字节区间 }
```

- S 层路径即结构化投影；L 层或改变 pointee 类型的 `PointerCast` 之后退化为字节区间。
- Phi 取根集合并集，循环 Phi 在 SCC 内迭代；Aggregate/WidePtr 携带“字段 i 的根集合”，`Field` 取回；
  Phi 与 Aggregate 只派生根，捕获发生在其结果流入的固定 sink。
- `Heap` 仅来自 allocator 契约（`ALLOCATOR`/`ALLOCATOR_ZEROED`/`REALLOCATOR` 标志）或摘要声明的新分配。

**alias(a, b)** 取 `NoAlias | MustAlias | Partial | MayAlias`，只用 provenance 与地址算术：

1. 两个不同的已识别对象（Local、Global、allocator Heap）为 NoAlias；
2. `Local(x)` 与 Argument、Global、Unknown：在查询点 x 尚未逃逸时为 NoAlias；
3. 同根比较路径：struct 不同字段、不同常量下标为 NoAlias；同一 enum 的不同变体、
   `SetVariant` 与其 payload、union 的任意字段为 MayAlias；前缀关系为 Partial；
4. 只读 Global 不会被写入 clobber（verifier 已有 Const 证明，`verify.rs:1419-1445`）；
5. 查询签名携带查询点（control 节点）与 `AliasEvidence` 扩展点，RS5 的权限证据由此接入。

**流敏感逃逸。** 捕获点为 control 上的固定 sink：无 `captures=None` 摘要的调用实参、
存储地址值的 store、`Opaque`、`TransmuteValue`。在 CFG 上做前向 may-escaped 位向量，
只为被查询且存在捕获点的根分配位。ptr→int（浮动的 `IntCast`）保持函数入口即逃逸。
`SlotLive` 不重置逃逸位。流敏感逃逸服务 forward、DSE 与 clobber 查询；promotion 与 verifier 的
Local 标签继续使用整函数逃逸。

**Clobber walker。** 沿 effect 链回溯，每次查询有步数预算（初值 64，可配置），
在 Effect Phi 处按前驱共享预算，按（effect 节点, Location）缓存，超预算回答 MayAlias。

## 调用摘要

```text
CallSummary {
  args: Vec<ArgEffect { read, write, capture: None | Address | Full, returned }>,
  globals: { read, write: SymbolSet | Any },
  unknown: { read, write },
  ret: None | Arg(i, Path) | Fresh | Unknown,
  normal_return: Effects, diverging: Effects,
  may_diverge, may_unwind, synchronizes,
}
```

- **计算位置：** `collect_references`（`lib.rs:166`）之后、阶段 2 之前。Tarjan SCC，逆拓扑序，
  SCC 内从 ⊥（不读、不写、不捕获、不发散）迭代到最小不动点，迭代顺序固定。
- **消费位置：** 阶段 2 的 seams 之后新增全函数 `optimize_memory(graph, summaries)`，
  依次运行 forward v2、DSE、拷贝前递与部分提升，然后 verify。
- **正常返回与发散分开：** 跨调用转发只需要正常返回路径上的写；删除调用前的 store 需要全部路径上的读。
  可能 panic 的被调者，其发散部分对已逃逸根与全局的读为 Any（panic hook 运行任意代码）。
- **synchronizes：** 函数体含原子、fence、无摘要调用或 `CallIndirect` 时置位，此时对已逃逸根与全局按 Unknown。
- **适用范围：** 只对本 crate 发射且链接时不可替换的定义使用摘要：internal 与 hidden 符号；
  dylib/cdylib 的 default-visibility 导出跳过。`CallIndirect`、LLVM 编译的 std 非泛型函数取顶，
  仅使用 rustc 的声明级属性。
- **rustc 声明级属性：** `NEVER_UNWIND`、`FFI_PURE`/`FFI_CONST`、allocator 标志、`COLD` 经 kfrc-body
  新增的 `CalleeFacts` 进入 Body（与 RS6/RS7 共用，见 [facts.md](facts.md#公共载体)）。
  `deduced_param_attrs`（跨 crate 可读，只覆盖 Indirect 参数）供 RS2.3 省去实参拷贝。
- **libcall：** `memcpy`/`memmove`/`memset`/`memcmp` 在转换器改为带契约的 Body 语句；
  摘要只来自操作契约或函数体分析。
- **健全性：** 发射代码是同一 S 图经内联与降低得到的 refinement，行为包含于摘要。
  摘要进入读取它的调用者的机器码缓存键。

## CopyNonOverlapping

copy 是表示拷贝：保留 padding、未初始化字节与 provenance，元素可以是非法值。

1. **访问模型：** `Read(src, count × size)` 与 `Write(dst, count × size)`；操作契约给出该点源与目标不相交。
2. **精确前递：** count 为常量且 src 的到达定义是覆盖整个 src 的 typed `StoreValue(v)`、
   dst 的 pointee 类型与 v 相同时，dst 的全覆盖 typed load 可替换为 v，copy 可改写为 `StoreValue(dst, v)`。
3. **源 slot 提升：** slot 仅作 copy 源且全部字节来自 typed 写时，copy 改写为 `StoreValue(dst, v)`，slot 被提升。
4. **死 copy 与 copy 链：** dst 在被覆盖或 `SlotDead` 之前无读者时删除；`A→B; B→C` 且其间 A、B 无写时改为 `A→C`。
5. **L 层：** `CopyBytes{effect, src, dst, bytes}` 节点，契约为只读 src、只写 dst、不捕获；
   x86 对小常量长度内联整寄存器 mov，长度大时调用 `memcpy`。`raw_copy` 在已知不重叠时同样替换。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS4.0 度量 | promotion 与 forward 拒绝原因枚举与计数；L copy 调用计数；`memory_s` profile 窗口 | `promotion.rs`、`forward.rs`、`pipeline.rs`、`lower_layout/semantic_memory.rs`、`lib.rs` | `from_str` 与语料基线由脚本生成 | LP3 合入后改 `forward.rs` |
| RS4.1 Location | `analysis/memory/location.rs`；`MemoryFacts` 改为基于它并保持现有逃逸语义 | `analysis/memory.rs` 拆为目录 | 全部测试通过；语料上 Local/Const 标签与旧实现逐函数一致 | — |
| RS4.2 alias 查询 | 上节规则 1–5；动态 alias 断言模式（解释器记录每次访问的真实分配，检查其属于静态根集合） | `analysis/memory/alias.rs`、`interpret/semantic_memory.rs` | 每条规则单测；负例 N4.5–N4.7、N4.10–N4.11 | RS4.1 |
| RS4.3 流敏感逃逸 | 捕获点；位向量；`escaped_before` | `analysis/memory/escape.rs` | N4.1–N4.3、N4.13 解释器差分通过；标签不变 | RS4.1 |
| RS4.4 walker 与 forward v2 | 有界回溯与缓存；Effect Phi 处各前驱都给出值时建值 Phi，回边处停止；包含 LP3 的 load→load 规则 | `transforms/memory/walker.rs`、`forward.rs` | 正例 P4.1/P4.2/P4.5/P4.6 生效；LP3 回归通过；`optimize_s` 增量 ≤ 5% | RS4.2、RS4.3；LP3 合入 |
| RS4.5 DSE | 向后查找完全覆盖的写、`SlotDead` 或未逃逸 local 的 Return；中间任何可能读取即停止 | `transforms/memory/dse.rs` | P4.7、N4.8 | RS4.4 |
| RS4.6 调用摘要 | `CalleeFacts` 进入 Body；`analysis/summary.rs`；SCC 不动点；阶段 2 新增 `optimize_memory` | `kfrc-body/src/body.rs`、`convert/calls.rs`、`lib.rs`、`transforms/ipo/mod.rs` | P4.3、P4.11、N4.9、N4.14、N4.15；输出 `summary_hits / calls` | RS4.4、RS4.5 |
| RS4.7 copy | 上节 1–4（S 层），5（L 层）；libcall 改为带契约语句 | `analysis/memory/*`、`promotion.rs`、`lower_layout/semantic_memory.rs`、`maybe_uninit.rs`、`verify.rs`、x86 `lower.rs`、`convert/intrinsics.rs:231-275` | P4.8、P4.9、N4.12；memcpy/memmove 调用数下降；padding 与 provenance 专项断言 | 1–4 依赖 RS4.4；5 依赖 LP3 |
| RS4.8 部分提升 | 整体提升失败时按顶层 struct 字段子树分别判定（非 enum、非 union） | `promotion.rs` | P4.10；按原因的拒绝数下降 | RS4.0、RS4.2 |
| RS4.9 多链或浮动 load | 仅当 LICM/调度数据表明删除访存之外仍有显著收益时另立设计 | — | 独立设计 PR | RS4.4–RS4.8 的数据 |

## Fixture 与负例

正例 P4.1–P4.11 依次对应下面的 `p1`–`p11`。

```rust
pub unsafe fn p1(p: *mut (u64, u64)) -> u64 { (*p).0 = 1; (*p).1 = 2; (*p).0 }       // → 1
pub unsafe fn p2(p: *mut (u64, u64)) -> u64 { let a = (*p).0; (*p).1 = 3; a + (*p).0 }  // 第二次 load 消除
#[inline(never)] pub fn get(p: *const u64) -> u64 { unsafe { *p } }
pub unsafe fn p3(a: *mut u64, b: *const u64) -> u64 { *a = 1; let r = get(b); *a + r }  // 摘要：get 不写
pub fn p4() -> u64 { let mut x = 1u64; x += 2; let y = x; stash(&mut x); y + x }        // y 前递；x 在 stash 后重读
pub fn p5() -> u64 { let mut x = 0u64; let p = &mut x as *mut u64; unsafe { *p = 5 }; x } // → 5
pub fn p6(c: bool) -> u64 { let mut s = (0u64, 0u64, 7u64);
    let r: *mut u64 = if c { &mut s.0 } else { &mut s.1 }; unsafe { *r = 1 }; s.2 }       // → 7
pub unsafe fn p7(p: *mut u64) { *p = 1; *p = 2 }                                         // 删除第一条 store
pub fn p8(v: [u64; 4]) -> u64 { let src = v; let mut dst = [0u64; 4];
    unsafe { core::ptr::copy_nonoverlapping(&src, &mut dst, 1) }; dst[1] }                // → v[1]
pub fn p9(a: &mut [u64; 2], b: &mut [u64; 2]) { core::mem::swap(a, b) }                 // copy 数下降
pub fn p10(i: usize, x: u64) -> u64 { let mut s = (x, [0u8; 16]); s.1[i & 15] = 1; s.0 + 1 } // s.0 提升
pub fn p11() -> u64 { let b = Box::new(3u64); opaque_noinline(); *b }                  // Heap 根
```

| 编号 | 形状 | 期望 |
|---|---|---|
| N4.1 | `stash(&mut x); unknown(); x` | unknown 之后重读 |
| N4.2 | `&mut x` 存入 `static mut` 后调用 | 存储地址即捕获 |
| N4.3 | `&mut x as *mut u64 as usize` 传出 | 入口即逃逸 |
| N4.4 | `Cell` 经 `&` 传给被调者后 `c.get()` | 重读（本方向不使用 `&T` 只读规则） |
| N4.5 | `UnsafeCell::get` 的指针写后读 | 同根同区间不跨写转发 |
| N4.6 | 写 A 变体 payload，`SetVariant(B)`，读 A payload | MayAlias，SetVariant 为 clobber |
| N4.7 | union 不同字段写后读 | MayAlias |
| N4.8 | `*g = 1; might_panic(); *g = 2`，g 为全局 | 保留第一条 store |
| N4.9 | 中间夹 `CallIndirect` | 取顶 |
| N4.10 | 两个 `*mut u64` 参数：`*a = 1; *b = 2; *a` | 重读 |
| N4.11 | Phi 根集合超过 4 | Unknown |
| N4.12 | copy 源曾经由 `[u8; N]` 视图写入 padding | 拒绝精确前递与源提升 |
| N4.13 | 循环中 `SlotDead`/`SlotLive`，上一轮已捕获地址 | 逃逸位保持 |
| N4.14 | 互递归 SCC，其中一个写全局 | 不动点收敛，写效果传遍 SCC |
| N4.15 | dylib 的 default-visibility 导出 | 不用其摘要 |

解释器不建 provenance 模型（`interpret/semantic_memory.rs:11-26`），因此“指针字节经整数往返”
的形状另写断言，动态 alias 断言模式（RS4.2）用于在错编译之前验证分析的健全性。

## 指标

S：`loads_forwarded`、`stores_removed`、`copies_removed`、`slots_promoted`（含部分提升）与按原因的拒绝；
L：字段 load/store、memcpy/memmove 调用；机器层：内存操作数 mov、`call memcpy`、`.text`；
分析成本：`MemoryFacts` 访问数（已有）、`location_updates`、`escape_bits`、`queries`、`walker_steps`、
`cache_hits`、`budget_bailouts`、`summary_sccs`、`summary_iterations`、`summary_hits / calls`；
`optimize_s`、`post_inline_optimize_s`、`memory_s`、`layout` 窗口的编译指令。
