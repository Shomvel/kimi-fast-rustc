# LP6 循环优化

## 现状

- SoN 没有循环分析（`kfrc-son/src/analysis/` 只有 cfg/dominators/uses/memory/refinement）。
- x86 调度把纯节点放在最早位置（`lower/schedule.rs`）。这相当于把所有不变量都外提，但不会把只在冷路径用到的值下沉，也没有控制寄存器压力。
- [optimization-pipeline.md](../../../design/ir/optimization-pipeline.md) 的 L 级已列循环项（rotate、deletion、idiom、unswitch、归纳变量、穿过分区 Phi 的 load 转发与 PRE），并写明 placement 里做 LICM，均未实现。
- 研究建议 R3 指出这些项需要逐项写“做／由 GCM 承接／不做”。

## 范围（PoC 负载是解析器：字节循环、迭代器、`Vec` 增长）

| 项 | 做法 | 优先级 |
|---|---|---|
| 循环分析 | 基于支配树求自然循环、header、latch、exit、嵌套深度；作为 `analysis/loops.rs` 只读分析 | 必须，最先 |
| placement 中的 LICM | 按 [optimization-pipeline.md](../../../design/ir/optimization-pipeline.md) 做 elaboration 式 placement：纯值放在支配所有使用的最浅循环深度（schedule late within loop depth），不再一律最早放置 | 必须 |
| loop-rotate | 把 `while` 形转成 guard + do-while，为外提提供前置块，并让循环体只有一个条件跳转 | 必须 |
| 归纳变量化简与强度削减 | 识别基本 IV，派生 IV 的 `base + i*stride` 改为指针递增；循环条件改为与终值比较；与 LP4 的地址模式配合 | 必须 |
| 边界检查消除 | `i < len` 已由循环条件或范围事实证明时删除 `get`／下标检查。事实来自 [requirements-and-facts](../../../design/ir/requirements-and-facts.md) 的作用域谓词，不跨迭代复用失效事实 | 必须（Rust 解析器的主要开销之一） |
| 循环内 load 转发／PRE | 穿过分区 Phi 转发 `Vec` 的 `ptr`／`len`／`cap`（W05）；循环内有调用时按调用摘要判断 | 必须 |
| loop-idiom | 识别 memset/memcpy 形的循环 | 按 LP0 结果 |
| loop-deletion | 需要终止性证明；Rust 没有 `mustprogress`，无副作用的无限循环是合法程序 | 低 |
| unswitch | 循环不变条件外提并复制循环；受体积预算限制 | 低 |
| 展开、向量化 | 维持在 T38 之后，PoC 不做 | 不做 |

## 正确性边界

- 外提可能 trap 的操作（除法、可能越界的 load）只能放到守卫之后，或证明循环至少执行一次。
- 不把分支内的事实带到分支外；循环的下一次迭代是新的动态值实例，旧凭据不能复用。
- store 外提或下沉遵守 [memory-partitions](../../../design/ir/memory-partitions.md) 的激活写规则。

## LLVM 参考

- 分析：[LoopInfo.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/LoopInfo.cpp)、[ScalarEvolution.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Analysis/ScalarEvolution.cpp)（只借用 add-recurrence 的思路，不实现完整 SCEV）；
- 变换：[LoopRotation.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/LoopRotation.cpp)、[LICM.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/LICM.cpp)、[IndVarSimplify.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/IndVarSimplify.cpp)、[LoopStrengthReduce.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/LoopStrengthReduce.cpp)、[ConstraintElimination.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/ConstraintElimination.cpp)、[LoopIdiomRecognize.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/LoopIdiomRecognize.cpp)、[LoopDeletion.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/LoopDeletion.cpp)、[SimpleLoopUnswitch.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Transforms/Scalar/SimpleLoopUnswitch.cpp)；
- 机器级：[MachineLICM.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineLICM.cpp)、[MachineSink.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/CodeGen/MachineSink.cpp)，对应 kfrc 的 placement（GCM），不单独实现；
- 族占比：[pass-inventory-o3.md](../../../research/llvm-mir/pass-inventory-o3.md)。循环族约占 LLVM 计时的 8.7%（rg），可用来估计编译预算。

## 验收

- 解释器差分：每个变换的前后图在解释器下输出相同，覆盖 0 次、1 次、多次迭代以及 break/continue。
- 形状：字节扫描循环（`bytes().position`、`split`）、`for i in 0..v.len()` 求和、`Vec::push` 循环，kfrc 的循环体指令数与 LLVM 对照，并断言循环体内无边界检查。
- W05 walkthrough 的 load 转发结论复现。
- semver 的 parse／req runtime 与 LP0 的热点表对照。
