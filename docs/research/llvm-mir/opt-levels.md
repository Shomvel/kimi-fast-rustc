# opt-level × CGU × LTO：rustc 实际选哪条流水线

## 选择规则（源码核实）

| 输入 | 决定 | 位置 |
|---|---|---|
| `-Clto` 未给、opt-level>0、CGU>1 | `Lto::ThinLocal` | [session.rs#L582–L643](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/session.rs#L582) |
| CGU=1（且未给 `-Zthinlto`） | `Lto::No`：单遍 `buildPerModuleDefaultPipeline` | 同上 #L634 |
| CGU 默认值 | 16；增量时 256 | [session.rs#L770](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/session.rs#L770) |
| `--emit=llvm-ir/llvm-bc/asm/obj` 且未显式给 CGU，或显式 N>1 但带 `-o` | CGU 强制为 1，关闭本地 ThinLTO | [config.rs#L2171](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L2171) |
| pre-link 阶段 | Fat→`PreLinkFatLTO`，Thin/ThinLocal→`PreLinkThinLTO`，否则 `PreLinkNoLTO` | [write.rs#L875](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L875) |
| opt-level → PassBuilder 级别 | 0/1/2/3/s/z → O0/O1/O2/O3/Os/Oz | [write.rs#L154](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L154) |
| 循环向量化 | O2、O3 | [rustc_codegen_ssa write.rs#L226](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/write.rs#L226) |
| SLP 向量化 | 只 O3 | 同上 #L229 |
| MergeFunctions | O2、O3、s、z（目标可关） | 同上 #L241 |
| 循环展开与交错 | 除 s、z 以外 | [write.rs#L596](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L596) |
| lifetime 标记 | opt-level>0 或开 ASan/MSan/HWASan | [session.rs#L511](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/session.rs#L511) |

## 矩阵（实测，样例程序，`pipelines.sh`）

“组成”是该次编译打印的流水线按标签计数；标签来自 CGU=1 的参考运行。`A=B` 表示两条流水线文本完全相同。全部 48 组见脚本输出 `matrix.tsv`，这里只列 O3 和各级默认配置。

| config | pipelines | composition |
|---|---:|---|
| O0 cgu=16 lto=default | 16 | 16×O0=ThinPre-O0 |
| O1 cgu=16 lto=default | 32 | 16×ThinPost-O1 + 16×ThinPre-O1 |
| O2 cgu=16 lto=default | 32 | 16×ThinPost-O2 + 16×ThinPre-O2 |
| O3 cgu=1 lto=default | 1 | 1×PerModule-O3 |
| O3 cgu=1 lto=off | 1 | 1×PerModule-O3 |
| O3 cgu=1 lto=thin | 69 | 68×ThinPost-O3 + 1×ThinPre-O3 |
| O3 cgu=1 lto=fat | 2 | 1×Fat-O3 + 1×FatPre-O3 |
| O3 cgu=16 lto=default | 32 | 16×ThinPost-O3 + 16×ThinPre-O3 |
| O3 cgu=16 lto=off | 16 | 16×PerModule-O3 |
| O3 cgu=16 lto=thin | 99 | 83×ThinPost-O3 + 16×ThinPre-O3 |
| O3 cgu=16 lto=fat | 17 | 1×Fat-O3 + 16×FatPre-O3 |
| Os cgu=16 lto=default | 32 | 16×ThinPost-Os + 16×ThinPre-Os |
| Oz cgu=16 lto=default | 32 | 16×ThinPost-Oz + 16×ThinPre-Oz |
| O3 --emit=llvm-ir,link | 1 | 1×PerModule-O3 |
| O3 cgu=16 --emit=llvm-ir,link -o | 1 | 1×PerModule-O3 |

- **O0 的 ThinLTO pre-link 与普通 O0 完全相同**：只有 `always-inline`、`globaldce` 等 10 个叶子 pass。
- **CGU 数只改变模块个数，不改变流水线文本**；但 CGU=1 会让 ThinLocal 消失，换成单遍 `PerModule`。所以“CGU=1 对照 CGU=16”同时改变了划分和流水线，不能只当作“并行度”的对照。
- `-Clto=thin` 的 post-link 模块数 = 本地 CGU + 上游 rlib 中参与 LTO 的模块（样例：CGU=1 时 68 个，CGU=16 时 83 个）。`-Clto=fat` 只有一条 LTO 流水线，在整个程序合并成的一个模块上串行跑。
- `--emit=llvm-ir` 两种写法都退化成 1×PerModule-O3：前期“未优化 IR 矩阵”因此测的是单 CGU 的结构（前期 pass-cost-retrospective §4），比真实 16 CGU 构建少约 19% 的函数定义。

## 各级的 pass 计数（实测，选行）

| pass | O0 | PerModule-O1 | ThinPre-O1 | ThinPost-O1 | PerModule-O3 | ThinPre-O3 | ThinPost-O3 | ThinPre-Os | ThinPost-Oz |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| leaf-total | 10 | 94 | 59 | 88 | 115 | 77 | 108 | 73 | 103 |
| sroa | 0 | 4 | 3 | 3 | 4 | 3 | 3 | 3 | 3 |
| instcombine | 0 | 8 | 5 | 8 | 8 | 5 | 8 | 5 | 8 |
| simplifycfg | 0 | 8 | 6 | 7 | 8 | 6 | 7 | 6 | 7 |
| early-cse | 0 | 2 | 2 | 1 | 2 | 2 | 1 | 2 | 1 |
| gvn | 0 | 0 | 0 | 0 | 1 | 1 | 1 | 1 | 1 |
| inline | 0 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| argpromotion | 0 | 0 | 0 | 0 | 1 | 1 | 1 | 0 | 0 |
| mergefunc | 0 | 0 | 0 | 0 | 1 | 0 | 1 | 0 | 1 |
| jump-threading | 0 | 0 | 0 | 0 | 2 | 2 | 2 | 2 | 2 |
| correlated-propagation | 0 | 0 | 0 | 0 | 2 | 2 | 2 | 2 | 2 |
| dse | 0 | 0 | 0 | 0 | 1 | 1 | 1 | 1 | 1 |
| memcpyopt | 0 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 |
| licm | 0 | 3 | 2 | 3 | 4 | 3 | 4 | 3 | 4 |
| loop-unroll | 0 | 1 | 0 | 1 | 1 | 0 | 1 | 0 | 1 |
| loop-vectorize | 0 | 1 | 0 | 1 | 1 | 0 | 1 | 0 | 1 |
| slp-vectorizer | 0 | 0 | 0 | 0 | 1 | 0 | 1 | 0 | 0 |

- O1 用专门的 `buildO1FunctionSimplificationPipeline`（[#L430](https://github.com/llvm/llvm-project/blob/llvmorg-21.1.8/llvm/lib/Passes/PassBuilderPipelines.cpp#L430)）：没有 GVN、JumpThreading、CVP、DSE；InstCombine 次数与 O3 相同。
- O1、Os、Oz 的 `loop-vectorize` 参数是 `vectorize-forced-only`，只处理带 pragma 的循环；SLP 只在 O3。
- Os/Oz 与 O3 的 post-link 只差 `argpromotion` `chr` `libcalls-shrinkwrap` `openmp-opt-cgscc` `slp-vectorizer`（`diff-thinpost-O3-vs-Oz.txt`）；体积级别主要靠 pass 内部阈值（内联、展开），不靠换 pass。
- O2 与 O3：pre-link 只差 `argpromotion` `callsite-splitting`，post-link 只差 `argpromotion` `chr` `slp-vectorizer`。

## cargo 传了什么（实测，`cargo-flags.sh`）

| profile | 库 crate | 二进制 crate | `--extern` |
|---|---|---|---|
| release 默认 | `--emit=dep-info,metadata,link -C opt-level=3 -C embed-bitcode=no` | `--emit=dep-info,link`，同样的 `-C` | 库依赖库用 `.rmeta`，二进制用 `.rlib` |
| release + `lto = "thin"`/`"fat"` | `-C linker-plugin-lto`（**库只出 bitcode**） | `-C lto=thin`/`fat` | 同上 |
| release + `lto = "off"` | 加 `-C lto=off` | 同 | 同上 |

cargo 不传 `-C codegen-units`，所以真实 release 构建是 16 CGU + ThinLocal。开启跨 crate LTO 时库 crate 只做 pre-link（linker-plugin-lto 把 `emit_obj` 设为 bitcode，[rustc_codegen_ssa write.rs#L140](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/back/write.rs#L142)），**全部 post-link 与后端推迟到二进制 crate**。这正是“后端集中到末尾”的现成形态，见 [timeline-rustc.md](timeline-rustc.md)。
