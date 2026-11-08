# ThinLTO post-link：第二遍化简

release 默认 `Lto::ThinLocal`：优化开启、CGU>1、没有显式 `-Clto` 时启用，只在本 crate 的 CGU 之间导入（[session.rs#L582](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/session.rs#L582)）。`-Ccodegen-units=1` 时直接回到单遍流水线（#L634）。

## 发生了什么（源码核实）

| 步 | 位置 | 串行/并行 |
|---|---|---|
| 1 每个 CGU pre-link 后序列化成 ThinLTO buffer（带 summary），交回协调线程 | [write.rs#L870](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/write.rs#L870) | 每 CGU 一个 worker，并行 |
| 2 **屏障**：所有 CGU 的 pre-link 都结束后才进入 LTO | [write.rs#L1745](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/write.rs#L1745) | — |
| 3 全局分析：死符号、跨模块导入列表、prevailing、internalize/promote | `thin_lto` → `LLVMRustCreateThinLTOData`（[lto.rs#L377](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/lto.rs#L377)、[PassWrapper.cpp#L1186](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_llvm/llvm-wrapper/PassWrapper.cpp#L1186)） | 协调线程，串行 |
| 4 每个模块：**从 bitcode 重新解析**到新 Context，rename / resolve weak / internalize / import | `optimize_thin_module`（[lto.rs#L722](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/lto.rs#L722)；源码注释承认本地模块本可复用 Context） | 并行 |
| 5 `buildThinLTODefaultPipeline(O3, nullptr)` | [lto.rs#L596](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/lto.rs#L596)、[PassBuilderPipelines.cpp#L1784](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp#L1784) | 并行 |
| 6 codegen（[codegen-backend.md](codegen-backend.md)） | 同一 worker 接着做 | 并行 |

`-Clto=thin` 走同一条路，只是把上游 rlib 的 bitcode 也纳入：样例程序 CGU=16 时 post-link 模块从 16 个变成 83 个（实测 `matrix.tsv`）。`-Clto=fat` 则把所有模块链成**一个**模块，跑一次 `buildLTODefaultPipeline`，再单线程 codegen。

## 重跑了什么（实测）

post-link 流水线 = 模块化简半段（`buildModuleSimplificationPipeline(ThinLTOPostLink)`）+ 模块优化半段（`buildModuleOptimizationPipeline`）。与 pre-link 的叶子 pass 集合相比：

- **只在 pre-link：** `annotation2metadata` `callsite-splitting` `coro-early` `ee-instrument` `forceattrs` `inferattrs` `lower-expect`。即开头的属性标注和“前端输出清理”函数段。
- **只在 post-link：** `alignment-from-assumptions` `cg-profile` `chr` `constmerge` `coro-annotation-elide` `coro-cleanup` `coro-split` `div-rem-pairs` `elim-avail-extern` `float2int` `infer-alignment` `inject-tli-mappings` `instsimplify` `loop-distribute` `loop-load-elim` `loop-sink` `loop-unroll` `loop-vectorize` `lower-constant-intrinsics` `lowertypetests` `mergefunc` `pgo-icall-prom` `recompute-globalsaa` `rel-lookup-table-converter` `rpo-function-attrs` `simplify-type-tests` `slp-vectorizer` `transform-warning`。基本都是优化半段（循环向量化、展开、SLP、MergeFunctions 等）和 ThinLTO 专用 pass。
- 其余全部重跑：IPSCCP、GlobalOpt、整个 CGSCC 内联管线和其中的函数化简管线（InstCombine 在 post-link 出现 8 次，pre-link 5 次，见 [pass-inventory-o3.md](pass-inventory-o3.md)）。

“post-link 把整条化简流水线再跑一遍”**成立**。优化半段从 `elim-avail-extern` 开始，它删除导入的 `available_externally` 函数体，所以导入体只活在化简半段：**每个导入方把别人已化简过的体再化简一遍，用于内联，然后丢弃**。

## 代价（前期材料原文）

ripgrep 全部 crate 的 IR 规模（前期 thinlto-postlink 笔记 §2）：

| 阶段 | 函数定义 | IR 指令 | 跨 CGU 重复 |
|---|---:|---:|---:|
| prelink 输入 | 26,813 | 1,175,161 | 9.8% |
| prelink 输出 | 9,713 | 540,833 | 1.5% |
| 导入之后（post-link 输入） | 16,572 | 698,869 | 23.5%（导入的副本） |
| post-link 输出 | 8,639 | 645,793 | 0.8% |

- 离线 `opt`（rg 最大 6 个 crate、96 个模块）：pre-link 13.81 s + post-link 13.17 s，而不用 ThinLTO 的单遍 `default<O3>` 只要 15.65 s；**ThinLocal 让 LLVM 优化 CPU 多出约 75%**。同一 post-link 流水线处理导入前的模块只要 7.87 s：**导入占 post-link 的 40%**。平均每模块导入 59 个函数，post-link 后仍被调用的占 23%。
- E2B 第四轮，臂都建在 B（`-Zmono-dedup -Zshare-drop-glue=all`，相对基线 A 的 rg 指令 −6.06%）之上：Q 臂只重新化简“调用了导入函数的函数”，相对 A 的 rg 指令 −11.33%、墙钟 −6.12%，运行时各负载 −0.91%…+0.87%；O 臂改用 `-Clto=off`，rg 指令 −31.61%、墙钟 −14.53%，运行时 +1.50%…+27.83%。**ThinLocal 的价值在跨 CGU 内联，浪费在重复化简。**
- 墙钟收益小于指令收益：post-link 是 16 个 CGU 并行做的，省下的多是并行部分的 CPU。

三阶段 CPU（前期 E2B 逐 pass 重放，`census-passes.sh` 汇总；Z 臂含上面的 Q 跳过，所以 post-link 下降最多）：

| stage | rg A 进程 CPU s | rg A 计时占比 % | rg Z 进程 CPU s | rg Z 计时占比 % | cargo Z 进程 CPU s | cargo Z 计时占比 % |
|---|---:|---:|---:|---:|---:|---:|
| codegen | 28.12 | 29 | 26.41 | 35.3 | 91.73 | 30 |
| postlink | 26.27 | 31.9 | 16.97 | 25.3 | 83.06 | 31.9 |
| prelink | 31.04 | 39.1 | 24.84 | 39.4 | 99.1 | 38.1 |
| total | 85.43 | 100 | 68.22 | 100 | 273.89 | 100 |

## 其他结构性问题

- **屏障 + 长尾**：最大的 CGU 决定 pre-link 何时结束；前期 cargo trace 中 post-LTO `cgu.04` 约 14.68 s，`cgu.00` 约 5.74 s（前期 thinlto-report）。
- **成本模型**：`ThinModule::cost()` 用 bitcode 字节数估算，不看导入后的膨胀（前期 thinlto-report，源码 rustc_codegen_ssa `back/lto.rs`）。
- **bitcode 往返**：本地模块先序列化再重新解析，未单独量化。
- **导入阈值**：`-import-instr-limit=50`（T 臂）rg 指令再 −1.7 个百分点，cargo 运行时 +0.9 个百分点（前期 next-directions §4.1）。

## 对 kfrc 的含义

导入方重复化简的根源是“优化结果不能在模块之间共享”。kfrc 让内联源是 build 级缓存的已优化 S 图，内联只拼接、以接缝为 worklist 种子（optimization-pipeline.md“内联策略”，compile-time.md C2），所以不存在“导入体再化简一遍”。要保住的是 ThinLocal 带来的那部分运行时（O 臂 +1.50%…+27.83%）：跨 CGU、跨 crate 内联必须保留，见 [research/llvm-mir/mapping-to-design.md](mapping-to-design.md)。
