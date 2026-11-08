# W02 迭代器链与闭包

## 1. 程序

```rust
#[inline(never)]
pub fn sum_scaled(xs: &[u32], k: u64) -> u64 {
    xs.iter()
        .filter(|x| **x % 3 != 0)
        .map(|&x| x as u64 * k)      // 捕获 k
        .sum::<u64>()
}
```

完整源码 [scripts/w02.rs](scripts/w02.rs)，测量脚本 [scripts/w02.sh](scripts/w02.sh)。

## 2. rustc 现在怎么做（实测）

MIR 内联已把 `iter`/`filter`/`map`/`sum`/`Map::fold`/`Filter::fold` 这些薄包装压进 `sum_scaled`，但停在 `<slice::Iter<u32> as Iterator>::fold::<u64, 合成闭包>`：它和三层闭包（`filter_fold`、`map_fold`、`Sum` 的闭包）作为独立函数进入 LLVM。

```text
== overflow-checks=off  unoptimized IR (after MIR opts), sum_scaled call tree
TOTAL functions=9 inst=331 alloca=28 memcpy=2 lifetime=26 assume=10 call=4 invoke=13 landingpad=8 load=39 store=49
== overflow-checks=off  O3 IR
TOTAL functions=1 inst=100 alloca=0 memcpy=0 lifetime=0 assume=0 call=0 invoke=0 landingpad=0 load=6 store=0
== overflow-checks=off  x86 asm (rustc 1.99.1)
   43 asm-inst _RNvCshvxMuXbm9NJ_3w0210sum_scaled:
== overflow-checks=on  unoptimized IR (after MIR opts), sum_scaled call tree
TOTAL functions=9 inst=339 alloca=28 memcpy=2 lifetime=26 assume=10 call=6 invoke=13 landingpad=8 load=39 store=49
== overflow-checks=on  O3 IR
TOTAL functions=1 inst=28 alloca=0 memcpy=0 lifetime=0 assume=0 call=2 invoke=0 landingpad=0 load=1 store=0
== overflow-checks=on  x86 asm (rustc 1.99.1)
   26 asm-inst _RNvCshvxMuXbm9NJ_3w0210sum_scaled:
== IR passes that changed sum_scaled (overflow-checks=off, host target)
5 SimplifyCFGPass; 5 LCSSAPass; 4 LoopSimplifyPass; 4 InstCombinePass; 1 TailCallElimPass; 1 SROAPass; 1 ReassociatePass; 1 PostOrderFunctionAttrsPass; 1 LoopVectorizePass; 1 LoopUnrollPass; 1 LoopLoadEliminationPass; 1 JumpThreadingPass; 1 InstSimplifyPass; 1 InlinerPass; 1 IndVarSimplifyPass; 1 GlobalOptPass; 1 GVNPass; 1 EarlyCSEPass;
IR pass changes total: 32
```

- 13 个 `invoke`、8 个 `landingpad`（其中 2 个是 `panic_in_cleanup` 终止垫）：泛型 fold 不知道闭包会不会 unwind，按会处理；cleanup 里 drop 的是 `NeedsDrop=false` 的闭包和迭代器。O3 靠推断 nounwind 全部删掉。
- 最终机器码：`x % 3 != 0` 变成乘法逆元比较（`imul $-1431655765` + `cmp $1431655766`）。overflow-checks 关时循环展开 2 次、用 `cmov` 做 filter（推断：是 LoopVectorize 以 VF=1、interleave=2 完成的，未向量化——64 位乘法）；开时是标量循环 + `jo`/`jae` 跳到两个 panic 尾块。

## 3. kfrc 编译计划

**R 级（每个泛型定义一次）。** 适配器的多态体几乎全是对 `F: FnMut` 的 `TraitCall`，R 级只能做与闭包无关的部分。例如 `<slice::Iter<T> as Iterator>::fold` 的 R 图里，`T::IS_ZST` 是 `SizeOf(T)` 孔位控制的分支，两条都留着：

```text
n3  = SizeOf(T) : usize
n4  = Compare.eq (n3, c0) : bool
n5  = If ctl=n2 (n4)                       // ZST 分支，S 级折叠
n12 = TraitCall(FnMut::call_mut, [F, (B, &T)]) ctl=n10 mem=[Unknown:n9] (n7, n11) : (Control, Control, Effect, B)
```

`TraitCall` 的 unwind 出口接 `UnwindAction::Continue`。cleanup 只在有东西要 drop 时存在：这里 drop 的是 `F` 和 `self`，R 级是 `DropValue` 孔位（`NeedsDrop(F)`）。

**S 级（自底向上特化）。** 实例链（闭包类型唯一，答案键不会与别处共享）：

| 顺序 | 实例 | 特化时做的事 |
|---|---|---|
| 1 | 用户闭包 `{closure#0}`（filter）、`{closure#1}`（map，捕获 `&k`） | 无孔位，就是 R 图 |
| 2 | `filter_fold` 闭包、`map_fold` 闭包、`Sum` 闭包 | 解析 `TraitCall` → 内联 1 的图 |
| 3 | `slice::Iter<u32>::fold<u64, 合成闭包>` | `SizeOf(u32)=4` 折叠 ZST 分支；内联 2；`NeedsDrop=false` 删 `DropValue`；摘要 `may_unwind=false` 删 unwind 边 |
| 4 | `Filter::fold`、`Map::fold`、`<u64 as Sum>::sum` | 同样是单用实例：解析 `TraitCall`，内联 3 |
| 5 | `sum_scaled` | 非泛型函数没有孔位，R 图就是 S 图；内联 4，接缝种子化简 |

结果的循环（S 级，overflow-checks 关）：

```text
n20 = Loop ctl=[n6, n31]
n21 = Phi(n20) (c0, n28) : usize                 // 下标
n22 = Phi(n20) (c0, n30) : u64                   // 累加
n23 = IndexAddr(n1.0, n21) : &u32                // xs 是 Frozen(xs)，load 是纯的
n24 = Load[Frozen(xs)](n23) : u32
n25 = Rem.u32 (n24, c3) ; n26 = Compare.ne (n25, c0)
n27 = Mul.wrap (ZExt(n24), n2) ; n29 = Select (n26, n27, c0) ; n30 = Add.wrap (n22, n29)
n28 = Add.nuw (n21, c1) ; n31 = If (Compare.ult (n28, n1.1))
```

`Refine` 不需要：下标循环来自 std 的实现，`IndexAddr` 由 `n21 < len` 支配，没有 `Check.Bounds`。

**L 级。** `Rem.u32 x, 3` 与 0 比较 → 乘法逆元比较；`Select` → `cmov`；`IndexAddr` → `base + idx*4` 地址模式。overflow-checks 开时 `Mul`/`Add` 换成 `MulWithOverflow`/`AddWithOverflow` + `Check.Overflow`，两个 fail 出口进冷区域，各自调用不同的 panic 函数（不能合并）。

**期望机器码：** 与 rustc 的 overflow-on 版本同形（约 13 条循环指令）；overflow-off 版本如果不做 interleave，每元素约 8 条指令，rustc 展开 2 次后约 7.5 条——差距来自展开，不来自语义。

## 4. 代价核算

| 项 | rustc | kfrc（推断） |
|---|---|---|
| 进入后端的函数 | 9 个，331 条未优化 IR | 1 个对外函数；8 个单用实例只在 S 级存在 |
| 消除 invoke/landingpad | 靠 FunctionAttrs 推断 + SimplifyCFG | 摘要 `may_unwind=false` 直接删边 |
| 消除 alloca/lifetime（28/26） | SROA 等 | 本来没有：闭包环境和迭代器是值 |
| 改动 `sum_scaled` 的 IR pass | 32 次 | 构建期窥孔 + 每级一次 worklist |
| 缓存 | — | 见细化 2 |

## 5. 设计细化

1. **单用实例不应独立存在。** 8 个实例的参数里含调用者私有的闭包类型，只有一个调用点，最终全部内联。按现设计它们各有答案键、各自特化、各写摘要和机器码缓存，是纯浪费。建议：实参含“某个 typeck root 私有的闭包/fn item 类型”且只被该 root 使用的实例标为**嵌入实例**，在调用者的 S 级特化里直接展开，不单独登记答案键、不进缓存；被取地址或多处调用时退回普通实例。改 [generics-specialization.md](../../design/ir/generics-specialization.md)、[cache.md](../../design/build/cache.md)。
2. **R 级内联的范围要写清。** 非泛型函数（本例的 `sum_scaled`）调用的全是具体实例，它的 R 级与 S 级重合，内联的是被调者的 S 图。泛型函数在 R 级能内联的，只有实参全由调用者 `Param` 原样转发和具体类型组成的被调者：拼接时做 `Param` 换名，被调者的孔位表并入调用者的。现文“依赖但实参就是本函数的 `Param`”需要按这两种情况改写。改 [optimization-pipeline.md](../../design/ir/optimization-pipeline.md#r-级)。
3. **孔位可以控制分支。** `SizeOf(T)==0` 这类 ZST 分支在 std 中很常见；R 级优化必须保留两边，S 级折叠后要把被删分支的孔位从答案向量中去掉，否则 `u32` 和 ZST 实例会因为问了用不到的问题而无法共享。建议：答案键只含**特化后仍可达**的孔位，计算顺序是“先折叠控制孔位，再收集剩余孔位”。改 [generics-specialization.md](../../design/ir/generics-specialization.md#答案键)。
4. **L 级必须有常量除法/取模的强度削减**（乘法逆元、`x % c == 0` 判定）和 if-conversion（`Select`/`cmov`）。改 [runtime.md](../../design/goals/runtime.md#必须自己做的部分)。
5. **interleave/展开影响本例约 6% 的每元素指令数**，属于 T38 的循环项；记入 runtime 风险表，不需改 IR。
