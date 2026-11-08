# W04：drop flag、`?` 早退与 unwind

检验 [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) 的 `DropValue`/`DropInPlace`、drop flag 折叠、nounwind 摘要和冷区域，以及 [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#调用摘要) 的自底向上摘要。命令：[scripts/w04.sh](scripts/w04.sh)（MIR/IR 用 rustc 1.94.1，x86 汇编用 rustc 1.99.1 `x86_64-unknown-linux-musl`），代码块中的数字是脚本输出原样粘贴。标“推断”的是设计模拟，其余是实测。

## 1. 程序

[scripts/w04.rs](scripts/w04.rs)：`may_panic` 有越界检查；`no_panic` 是叶子；`even`/`odd` 互递归（一个 SCC）；`consume` 拿走 `String`。

```rust
pub fn work(t: &str, flag: bool) -> Result<usize, ParseIntError> {
    let name = String::from(t);
    let mut buf: Vec<u8> = Vec::with_capacity(8);
    buf.extend_from_slice(name.as_bytes());
    let n: usize = t.trim().parse()?;           // 早退：drop buf、name
    let mut acc = no_panic(n) + even(n) as usize;
    if flag { acc += consume(name); }           // 条件 move → drop flag
    acc += may_panic(&buf, n) as usize;         // 此处 unwind 要看 flag
    Ok(acc)
}
```

## 2. rustc 现在怎么做

**MIR（实测）。** `name` 得到 drop flag `_20`：入口置 `true`，`consume` 前置 `false`；正常出口 `switchInt(_20)` 后才 `drop(_3)`，cleanup 块 bb17 也 `switchInt(_20)`。cleanup 里的 `drop(buf)` 是 `unwind terminate(cleanup)`。abort 下 MIR 构建不变，`AbortUnwindingCalls` 把 unwind 改成 `unreachable` 并删掉 cleanup 块，**drop flag 仍在**（正常路径要用）：

```text
[unwind] MIR work: blocks=30 cleanup_blocks=4 drop=6 calls=10 unwind_to_bb=10 unwind_continue=4 unwind_unreachable=1 unwind_terminate=2 resume=1 drop_flag_writes=2
[abort] MIR work: blocks=26 cleanup_blocks=0 drop=4 calls=10 unwind_to_bb=0 unwind_continue=0 unwind_unreachable=16 unwind_terminate=0 resume=0 drop_flag_writes=2
```

**LLVM IR（实测）。** `count-ir.sh` 的 `call`/`invoke` 列把 rustc 的 `; call X`/`; invoke X` 注释行也计入（work 的 26 对 13），下面以 `instructions only` 行为准：

```text
[unwind pre] count-ir work: functions=1 inst=229 alloca=12 memcpy=4 lifetime=25 assume=12 call=10 invoke=26 landingpad=3 load=31 store=26
[unwind pre] work, instructions only: functions=1 call=5 invoke=13 landingpad=3 resume=1 phi_i1=0 select_i1=0 phi_exc=0
[unwind o3] work, instructions only: functions=1 call=13 invoke=3 landingpad=2 resume=1 phi_i1=1 select_i1=1 phi_exc=2
[unwind o3] call sites in work:
   ...
   1   invoke alloc::raw_vec::RawVecInner<A>::reserve::do_reserve_and_handle
   1   invoke alloc::raw_vec::handle_error
   1   invoke w04::may_panic
[unwind o3] nounwind attribute: may_panic=no no_panic=yes even=yes odd=yes consume=yes
[unwind x86] work: 386 asm-inst _Unwind_Resume_refs=2
[abort pre] work, instructions only: functions=1 call=15 invoke=0 landingpad=0 resume=0 phi_i1=0 select_i1=0 phi_exc=0
[abort pre] whole module: functions=95 call=170 invoke=0 landingpad=0 resume=8 phi_i1=0 select_i1=21 phi_exc=0
[abort o3] work, instructions only: functions=1 call=17 invoke=0 landingpad=0 resume=0 phi_i1=0 select_i1=1 phi_exc=0
[abort o3] call sites in work:
   ...
   2   call   w04::may_panic
   1   call   w04::no_panic
[abort o3] nounwind attribute: may_panic=yes no_panic=yes even=yes odd=yes consume=yes
[abort x86] work: 374 asm-inst _Unwind_Resume_refs=0
```

- **rustc 不知道 nounwind。** 未优化 IR 里叶子函数 `no_panic` 也是 `invoke`；LLVM 的 FunctionAttrs 推断后，O3 只剩 `may_panic`、`reserve` 冷路径、`handle_error` 三个 `invoke`。`even`/`odd` 这个 SCC 也被证明 nounwind。
- **drop flag 被折成用户条件。** O3 unwind 中 flag 变成 `%_20.sroa.0.3.off0 = xor i1 %flag, true`，以 `phi i1` 进 landingpad；正常路径是 `or i1 %flag, len==0` 决定是否释放。abort 的 O3 把 `may_panic` 调用点复制成两份来消掉 flag 分支。
- **cleanup 合并要合并异常值。** O3 work 有 `phi { ptr, i32 }`（`phi_exc`），即两个 landingpad 共享后缀时把异常对象 phi 起来再 `resume`。
- **abort 下上游 MIR 仍带 cleanup。** abort 未优化 IR 整模块 `landingpad=0` 但 `resume=8`：这些 `resume` 在 std 泛型函数的 cleanup 链里（std 的优化 MIR 以 unwind 构建）。codegen 发现 `fn_abi.can_unwind` 为假就把 unwind 改为 `Unreachable`（`rustc_codegen_ssa/src/mir/block.rs` L209），cleanup 链变成死代码；`fn_can_unwind` 读的是**当前会话**的 panic 策略（`rustc_middle/src/ty/layout.rs` L1243–1246）。

## 3. kfrc 编译计划

**Body（unwind，清理后）** 与 MIR 同形：C1 = `Drop(buf) → C2`，C2 = `if _20 { Drop(name) } → resume`，`with_capacity` 的 unwind 直接进 C2。**R 图：** `work` 非泛型：没有孔位，R 图即 S 图。被调的 std 函数（`Vec<u8>` 实例）在这一轮按答案内联。构建后的关键节点（P0 = `Unknown`，L7 = buf 的 `Local` 分区）：

```text
n1  = Start : (Control, Effect[P0], &str, bool)
n3  = Call(<String as From<&str>>::from) ctl=n1.0 mem=[P0:n1.1] (n1.2) : (Control, Control?, Effect[P0], String)
n5  = Call(Vec<u8>::with_capacity) ctl=n3.0 mem=[P0:n3.2] (8) : (Control, Control?, Effect[P0], Vec<u8>)
n7  = StackSlot{Vec<u8>} : &mut Vec<u8>                     // buf 被 autoref 借用
n9  = Call(Vec<u8>::extend_from_slice) ctl=n5.0 mem=[P0:n5.2, L7:n8] (n7, n10) : (Control, Control?, Effect[P0], Effect[L7], ())   // n8 = Store[L7](n7, n5.3)
n12 = Call(<str>::parse::<usize>) ctl=n9.0 mem=[P0:n9.2] (n11) : (Control, Control?, Effect[P0], Result<usize, ParseIntError>)
n13 = VariantSwitch ctl=n12.0 (n12.3) : (Control, Control)   // Err：DropInPlace(buf)、DropValue(name)、Return
n15 = Call(no_panic) ctl=n13.0 mem=[P0:n12.2] (n14) : (Control, Control?, Effect[P0], usize)
n16 = Call(even) ctl=n15.0 mem=[P0:n15.2] (n14) : (Control, Control?, Effect[P0], bool)
n18 = If ctl=n16.0 (n1.3) : (Control, Control)
n19 = Call(consume) ctl=n18.0 mem=[P0:n16.2] (n3.3) : (Control, Control?, Effect[P0], usize)
n21 = Region(n19.0, n18.1)
n22 = Phi(n21; false, true) : bool                          // drop flag _20
n23 = Phi(n21; n19.2, n16.2) : Effect[P0]
n24 = Call(may_panic) ctl=n21 mem=[P0:n23] (n25, n14) : (Control, Control?, Effect[P0], u8)   // n25 = buf 的 &[u8]
n26 = DropInPlace[L7](n7) ctl=n24.0 mem=[L7:..] : (Control, Control?, Effect[L7])   // 细化 1
n30 = Landing(n9.1, n12.1, n15.1, n16.1, n19.1, n24.1) : (Control, Exception)      // C1，细化 2
n31 = Phi(n30; true, true, true, true, false, n22) : bool
n32 = DropInPlace[L7](n7) ctl=n30.0 : (Control, Terminate)
n34 = Landing(n5.1) : (Control, Exception)                                         // 直接进 C2
n35 = Region(n32.0, n34.0)   n36 = Phi(n35; n31, true) : bool   n37 = Phi(n35; n30.1, n34.1) : Exception
n38 = If ctl=n35 (n36) → DropValue(n3.3) → Resume(n37)
```

**优化（S 级，与 R 级同一套规则）：**

| 步 | 变换 | 规则 / Rust 事实 |
|---|---|---|
| 1 | 内联 `String::from`、`with_capacity`、`extend_from_slice`、`Deref` | 直接调用、答案已知；`#[inline]` |
| 2 | buf 的 `StackSlot` 退回 SSA，`DropInPlace[L7]` → `DropValue(buf)` | 槽位提升：引用全部消失 |
| 3 | `no_panic`、`even`、`consume` 的 unwind 出口删掉；`trim`/`from_ascii_radix` 用上游摘要 | 摘要 `may_unwind=false`；Landing 只剩 `may_panic`、扩容冷路径、`handle_error` |
| 4 | `n22 = Phi(If(flag); false, true)` → `Not(flag)` | 常量 Phi 折叠；正常路径保留 `If(Not flag)`，与 O3 相同 |
| 5 | `DropValue(Vec<u8>/String)` 展开成 `if cap != 0 { __rust_dealloc }`；cleanup 里的 `Terminate` 出口删掉 | 小 drop glue 展开；dealloc 摘要 nounwind |
| 6 | Landing 之后、`handle_error`（`-> !`）之前的控制标冷 | 结构性冷区域：只做窥孔/DCE，L 级放末尾 |

**L 级与期望机器码（推断）：** unwind 下与 O3 同形，3 个可 unwind 调用、2 个 landing 入口，flag 是 `flag` 参数本身。abort 下没有 Landing，`may_panic` 只有一个调用点加一次 `test flag`（kfrc 不做尾复制），比 rustc 少一个调用点。

**检验：摘要的自底向上依赖。** 设计规定“调用者的 S 级优化等被调者的摘要”，摘要在 S 级**结束时**产出。按 rustc 未优化 IR（已过 MIR 内联）的调用图算高度（叶子为 0）：

```text
defined=95 local_edges=113 back_edges=1 max_height=16
functions per height: 0:26 1:19 2:10 3:7 4:6 5:6 6:7 7:3 8:3 9:1 10:1 11:1 12:1 13:1 14:1 15:1 16:1
height of work: 8
```

- （推断）`work` 前面有 8 层串行的“被调者 S 级跑完 → 调用者 S 级开始”；kfrc 在内联前建调用图，层数只会更多。本 crate 的 95 个函数里高度 ≥9 的每层只有 1 个，这段是纯串行。
- `size_estimate` 定义为“L 级机器码字节估计”，调用者的 S 级还要等被调者的 **L 级**。上游摘要随 metadata 发布，跨 crate 不等；串行只在本 crate 的实例图内部。

**检验：SCC。** 设计写“从保守值开始迭代到不动点”。保守初值 `may_unwind=true` 本身就是不动点，`even`/`odd` 永远证不出 nounwind；LLVM 实测证出来了（上面的 `even=yes odd=yes`）。正确做法是从乐观初值（⊥：不 unwind、不写内存）迭代到最小不动点。

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| unwind 边 | 先给每个调用生成 `invoke` 和 cleanup，再由 FunctionAttrs + SimplifyCFG 删 | 摘要已知时 S 级第一轮就删掉，cleanup 区域不进 L 级 |
| abort | 本地 MIR 删 cleanup；上游泛型 MIR 的 cleanup 在 IR 里成死块 | 同一裁剪对本地和上游 R 图都做（细化 3） |
| 编译时间 | LLVM 的 CGSCC 在每个 CGU 内也是自底向上串行，CGU 之间并行 | 摘要链可能成为关键路径（细化 5） |
| 缓存 | — | `work` 的机器码随 `no_panic` 的 `may_unwind` 变化而变（invoke↔call、Landing 有无），即使没内联（细化 6） |

## 5. 设计细化

| # | 问题 | 证据 | 建议改法 | 文件 |
|---|---|---|---|---|
| W04-1 | `DropValue`/`DropInPlace` 没写控制和效果输出，看不出可 unwind，cleanup 里也没有 `Terminate` 出口 | MIR `drop(_4) -> [return, unwind: bb17]`、cleanup 中 `unwind terminate(cleanup)`；未优化 IR 有 `panic_in_cleanup` | 两者结果改为 `(Control, Control?, Effect…)`；unwind 出口按 drop glue 答案键的摘要裁剪；在 cleanup 区域内出口接 `Terminate` | nodes.md、ownership-drop-unwind.md |
| W04-2 | `Landing`/`Resume` 语义不全：不能合并多条 unwind 边，没有异常值 | MIR 中多个调用共用 bb13（`unwind_to_bb=10`）；O3 有 `phi { ptr, i32 }`；drop flag 要按入边取值 | `Landing` 是 Region 型合并节点，产出 `Exception` 值；`Resume` 消费它；cleanup 合并用普通 `Phi` 合并异常值 | nodes.md |
| W04-3 | “`panic=abort` 在 Body 交给 SoN 前裁剪”只覆盖本地函数体；上游 R 图在 metadata 里，策略由上游定 | abort 未优化 IR `landingpad=0`、`resume=8`；`fn_can_unwind` 读当前会话策略 | 发布的 R 图一律保留 unwind 边；裁剪移到 S 级，panic 策略作为特化输入并进答案键（或 L 级配置键） | ownership-drop-unwind.md、generics-specialization.md、cache.md |
| W04-4 | SCC 摘要从保守值迭代，证不出互递归函数 nounwind | `even`/`odd` 被 LLVM 推断 nounwind | SCC 内从 ⊥ 开始迭代到最小不动点；SCC 内部不互相内联，或按固定顺序只内联一层 | optimization-pipeline.md |
| W04-5 | 摘要在 S 级结束时产出，`size_estimate` 又来自 L 级，调用图深度直接变成串行链 | `height of work: 8`，高度 ≥9 的每层只有 1 个函数 | 摘要改为独立的廉价自底向上遍历（在未优化 S 图上求传递函数），先于 S 优化完成；`size_estimate` 改用 S 图节点数估计；精度损失用 A/D/B 测 | optimization-pipeline.md、compile-time.md |
| W04-6 | 缓存键只含被内联者，但直接被调者的摘要也改变调用者代码 | `no_panic` 的 nounwind 决定 `work` 里是 `call` 还是 `invoke` | 机器码键加入全部直接被调者的摘要哈希；摘要哈希不变即 early cutoff | cache.md |
| W04-7 | “drop flag 折叠”只有名字 | O3 结果就是 `xor i1 %flag, true` | 写明它就是常量 Phi → 条件（`Phi(If c; false, true) = Not(c)`），不需要专用节点，也不做尾复制 | optimization-pipeline.md |
