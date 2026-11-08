# O3 pre-link 流水线：pass 清单

release 默认（opt-level=3、16 个 CGU、ThinLocal）下，每个 CGU 先跑 LLVM 的 `buildThinLTOPreLinkDefaultPipeline(O3)`（[PassBuilderPipelines.cpp#L1723](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp#L1723)）。它是“只化简、不展开不向量化”的半条流水线；后半条和第二遍化简在 post-link（[thinlto-postlink.md](thinlto-postlink.md)）。

## rustc 自己加了什么（源码核实）

`LLVMRustOptimize`（[PassWrapper.cpp#L560](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_llvm/llvm-wrapper/PassWrapper.cpp#L560)）**默认不插入任何优化 pass**，只做配置：

- `PipelineTuningOptions`：展开/交错、循环向量化、SLP、MergeFunctions 按 opt-level 开关（#L581–L586，取值见 [opt-levels.md](opt-levels.md)）。
- AA 用 `buildDefaultAAPipeline`；`#![no_builtins]` 时关闭全部库函数识别（#L662–L668）。
- 回调只在开启时注册：sanitizer、CFI、coverage、`-Zverify-llvm-ir`/`-Zlint-llvm-ir`（PipelineStart / OptimizerLast，#L685–L803）；`-Zself-profile` 的逐 pass 计时回调（#L591）；`-Zllvm-plugins`；`-Cpasses` 追加在流水线末尾（#L880）；Enzyme。
- 按阶段选 LLVM 的现成流水线（#L831–L869）；ThinLTO 需要时追加 `CanonicalizeAliases`、`NameAnonGlobal` 和 bitcode writer（#L889–L901）。

所以下表就是 LLVM 21 的默认流水线，rustc 的影响只在 PTO 和输入 IR 上。

## 结构（实测，`tree-ThinPre-O3.txt`）

| 段 | 内容 | 作用 |
|---|---|---|
| 1 前导 | `annotation2metadata` `forceattrs` `inferattrs` `coro-early` | 标注与属性推断 |
| 2 前端输出清理 | `function(ee-instrument, lower-expect, simplifycfg, sroa, early-cse, callsite-splitting)` | 把每个局部的 alloca 先提升掉，便宜地清理 rustc 的直译 IR |
| 3 模块 IPO | `ipsccp` `called-value-propagation` `globalopt` `function(mem2reg, instcombine, simplifycfg)` `always-inline` | 过程间常量、全局变量、强制内联 |
| 4 CGSCC 内联管线 | `cgscc(devirt<4>(inline, function-attrs, argpromotion, function(…), function-attrs))` | 自底向上按调用图 SCC：内联 → 推断属性 → 对 SCC 内每个函数跑**函数化简管线**；间接调用变直接时重跑，最多 4 轮 |
| 5 收尾 | `deadargelim` `globalopt` `globaldce` + ThinLTO 必需 pass | 删死参数和死全局 |

段 4 的函数化简管线（O3，[#L567](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp#L567)）按顺序：`sroa` `early-cse<memssa>` `speculative-execution` `jump-threading` `correlated-propagation` `simplifycfg` `instcombine` `aggressive-instcombine` `libcalls-shrinkwrap` `tailcallelim` `simplifycfg` `reassociate` `constraint-elimination` `loop-mssa(loop-instsimplify, loop-simplifycfg, licm, loop-rotate, licm, simple-loop-unswitch)` `simplifycfg` `instcombine` `loop(loop-idiom, indvars, loop-deletion, loop-unroll-full)` `sroa` `vector-combine` `mldst-motion` `gvn` `sccp` `bdce` `instcombine` `jump-threading` `correlated-propagation` `adce` `memcpyopt` `dse` `move-auto-init` `loop-mssa(licm)` `simplifycfg` `instcombine`。

这一段是成本主体：每个函数在内联进新代码后都会从头再走一遍；InstCombine 在这一段里出现 4 次，整条 pre-link 5 次。

## 每条流水线里各 pass 的出现次数（实测，静态计数）

`PerModule-O3` 是 `-Clto=off` 或 CGU=1 时的单遍流水线；`FatPre`/`Fat` 是 `-Clto=fat`。叶子 pass 计数，不含 pass manager 与适配器。

| pass | PerModule-O3 | ThinPre-O3 | ThinPost-O3 | FatPre-O3 | Fat-O3 |
|---|---:|---:|---:|---:|---:|
| leaf-total | 115 | 77 | 108 | 112 | 78 |
| sroa | 4 | 3 | 3 | 4 | 3 |
| mem2reg | 1 | 1 | 1 | 1 | 1 |
| instcombine | 8 | 5 | 8 | 8 | 5 |
| aggressive-instcombine | 1 | 1 | 1 | 1 | 1 |
| instsimplify | 1 | 0 | 1 | 1 | 0 |
| simplifycfg | 8 | 6 | 7 | 8 | 2 |
| early-cse | 2 | 2 | 1 | 2 | 0 |
| gvn | 1 | 1 | 1 | 1 | 1 |
| newgvn | 0 | 0 | 0 | 0 | 0 |
| sccp | 1 | 1 | 1 | 1 | 1 |
| ipsccp | 1 | 1 | 1 | 1 | 1 |
| inline | 1 | 1 | 1 | 1 | 2 |
| always-inline | 1 | 1 | 1 | 1 | 0 |
| function-attrs | 2 | 2 | 2 | 2 | 2 |
| argpromotion | 1 | 1 | 1 | 1 | 2 |
| deadargelim | 1 | 1 | 1 | 1 | 1 |
| globalopt | 2 | 2 | 2 | 2 | 2 |
| globaldce | 2 | 1 | 2 | 2 | 3 |
| elim-avail-extern | 1 | 0 | 1 | 0 | 1 |
| mergefunc | 1 | 0 | 1 | 1 | 1 |
| constmerge | 1 | 0 | 1 | 1 | 1 |
| jump-threading | 2 | 2 | 2 | 2 | 2 |
| correlated-propagation | 2 | 2 | 2 | 2 | 0 |
| constraint-elimination | 1 | 1 | 1 | 1 | 1 |
| dse | 1 | 1 | 1 | 1 | 1 |
| memcpyopt | 1 | 1 | 1 | 1 | 1 |
| mldst-motion | 1 | 1 | 1 | 1 | 1 |
| licm | 4 | 3 | 4 | 4 | 2 |
| loop-rotate | 2 | 1 | 2 | 2 | 0 |
| indvars | 1 | 1 | 1 | 1 | 1 |
| loop-idiom | 1 | 1 | 1 | 1 | 0 |
| loop-deletion | 2 | 1 | 2 | 2 | 1 |
| loop-unroll | 1 | 0 | 1 | 1 | 1 |
| loop-unroll-full | 1 | 1 | 1 | 1 | 1 |
| simple-loop-unswitch | 1 | 1 | 1 | 1 | 0 |
| loop-vectorize | 1 | 0 | 1 | 1 | 1 |
| slp-vectorizer | 1 | 0 | 1 | 1 | 1 |
| vector-combine | 2 | 1 | 2 | 2 | 1 |
| loop-distribute | 1 | 0 | 1 | 1 | 1 |
| loop-load-elim | 1 | 0 | 1 | 1 | 0 |
| reassociate | 1 | 1 | 1 | 1 | 0 |
| adce | 1 | 1 | 1 | 1 | 0 |
| bdce | 1 | 1 | 1 | 1 | 1 |
| tailcallelim | 2 | 1 | 2 | 2 | 1 |
| div-rem-pairs | 1 | 0 | 1 | 1 | 1 |
| float2int | 1 | 0 | 1 | 1 | 0 |
| lower-expect | 1 | 1 | 0 | 1 | 0 |
| called-value-propagation | 1 | 1 | 1 | 1 | 1 |

读法：

- **静态次数 ≠ 实际次数。** 段 4 对每个 SCC 跑一次，内联改动后重跑，LLVM 21 的 `LastRunTrackingAnalysis` 会跳过上次之后没变的函数。前期 cargo 主库 trace 的动态计数是 InstCombinePass 892,440 次、SimplifyCFGPass 961,358 次、InlinerPass 158,493 次、GVNPass 158,578 次（前期 thinlto-report），post-link 中约 405,000 次 InstCombine 调用里 337,000 次没有改动（前期 work-reduction 结论）。
- **PerModule-O3 的叶子集合 = ThinPre-O3 ∪ ThinPost-O3**，只差 ThinLTO 专用的 `lowertypetests`、`simplify-type-tests`、`pgo-icall-prom`（实测 `diff-permodule-vs-thin-O3.txt`）。ThinLocal 不是“加了一步”，而是把单遍流水线拆成两半，再把化简半段做两次。
- **Fat-O3** 是另一条流水线（`buildLTODefaultPipeline`，[#L1852](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp#L1852)）：内联 2 次、SimplifyCFG 只 2 次、没有 CVP/EarlyCSE，但在**单个合并模块**上串行跑。

## 各组在重建什么

分类沿用 evidence.md：A 值在内存，B 薄包装，C enum/niche 与比较，D 事实，M 机器级，G 全局/链接期。时间占比见 [README.md](README.md) 总表。

| 组 | pass | 类 | 在 Rust 代码上主要做的事 |
|---|---|---|---|
| 提升 | `sroa` `mem2reg` | A | 每个 MIR local 一个 alloca → SSA；拆开聚合 |
| 内存清理 | `memcpyopt` `dse` `mldst-motion` `move-auto-init` `early-cse<memssa>` `gvn`（load 部分） | A | move 的 memcpy 链、写后即死的 store、重复 load |
| 内联与属性 | `inline` `always-inline` `function-attrs` `argpromotion` `deadargelim` `called-value-propagation` | B | 压平闭包、drop glue、trait glue、Fn shim；推断 nounwind/readonly/nocapture 后删 landingpad |
| 控制流与值 | `instcombine` `simplifycfg` `jump-threading` `correlated-propagation` `constraint-elimination` `sccp` `bdce` `adce` | C（兼通用化简） | niche 解码折叠、重复判别值 switch、derive 比较链、`?` 的 ControlFlow 往返、边界检查 |
| 事实 | assume 处理（散在 `instcombine`）、`infer-alignment` `alignment-from-assumptions` | D | rustc 已发的属性/元数据基本够用（evidence.md §1） |
| 循环与标量 | `licm` `loop-rotate` `indvars` `loop-idiom` `simple-loop-unswitch` `loop-deletion` `loop-unroll-full` `reassociate` `tailcallelim` `speculative-execution` `libcalls-shrinkwrap` | M | 迭代器内联后的指针循环 |
| 全局 | `ipsccp` `globalopt` `globaldce` | G | 过程间常量、未写全局折叠、删死函数 |
