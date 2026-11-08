# 测量方法与口径

本目录的数字有三种来源，正文逐处标注：

| 标注 | 含义 |
|---|---|
| **实测** | 本目录 [scripts/](scripts) 在本机跑出的输出，表格由 `tsv-to-md.sh` 直接生成 |
| **前期 E2B** | 2026-10 rustc 构建剖析的 E2B 数据（Linux x86_64，8 vCPU），本目录脚本重新汇总，或引原报告原文 |
| **源码核实** / **推断** | 读 rustc `e408947b` 或 LLVM `llvmorg-21.1.8` 源码得出；推断表示未直接验证 |

## 环境（实测）

- rustc 1.94.1（Homebrew，commit `e408947bf`，LLVM 21.1.8），aarch64-apple-darwin。
- llc 21.1.8（Homebrew `llvm@21`），用于 x86-64 后端结构。cargo 1.94.1。
- nightly（1.101.0，2026-10-24）自带 LLVM 23.1.4，与 1.94.1 不同，**未使用**；需要的 `-Z` 开关用 `RUSTC_BOOTSTRAP=1` 在 1.94.1 上打开。
- 本机负载较高（load average 7–25），所以本机只取结构（pass 列表、次数），不取时间。时间一律用前期 E2B 数据。

## 哪些开关能导出流水线（实测 + 源码核实）

| 开关 | 结果 |
|---|---|
| `-Cllvm-args=-print-pipeline-passes` | 接受但无输出：该选项由 `opt` 驱动处理，rustc 不读 |
| `-Cllvm-args=-debug-pass-manager` | 报未知参数（只在 `opt` 驱动中定义） |
| `-Zautodiff=PrintPasses` | **可用**。`LLVMRustOptimize` 在 `PrintPasses` 为真时打印 `MPM.printPipeline`（[PassWrapper.cpp#L927](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_llvm/llvm-wrapper/PassWrapper.cpp#L927)），每个模块每个阶段一行；只打印、不启用 autodiff（[write.rs#L575](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/back/write.rs#L575)） |
| `-Zno-parallel-backend` | 必须同时加：多个 LLVM worker 并发写 stdout 会把行打碎（不加时，CGU=16 的各组都出现大量无法识别的半行）。该开关串行跑 LLVM，保留 CGU 与 ThinLTO（[options.rs#L2534](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/options.rs#L2534)） |
| `-Zprint-llvm-passes` | 等于 `-Cllvm-args=-debug-pass=Structure`（[llvm_util.rs#L89](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/llvm_util.rs#L89)），只覆盖 legacy PM，也就是后端 |
| `-Ztime-llvm-passes` | 等于 `-Cllvm-args=-time-passes`（同文件 #L86） |

## 脚本

| 脚本 | 做什么 | 输出 |
|---|---|---|
| `sample-crate.sh WORKDIR` | 写出 5 个模块的样例程序（泛型、闭包、迭代器、enum、`?`、`dyn`、循环） | `src/main.rs` |
| `pipelines.sh WORKDIR [RUSTC]` | 单模块参考运行给每条流水线定标签（CGU=1 时先 pre-link 后 LTO 阶段，顺序确定）；再跑 opt-level 0/1/2/3/s/z × CGU 1/16 × LTO default/off/thin/fat 共 48 组，外加两组 `--emit=llvm-ir` | `matrix.tsv`、`pass-table.tsv`、`tree-*.txt`、`diff-*.txt` |
| `codegen-passes.sh WORKDIR [RUSTC] [LLC]` | rustc `-Zprint-llvm-passes`（aarch64）与 `llc -mtriple=x86_64-unknown-linux-gnu -debug-pass=Structure`，O0–O3 | `summary.tsv`、`*.passes.txt` |
| `census-passes.sh CENSUS_JSON [TOP]` + `census-passes.jq` | 汇总前期 E2B 逐 pass 重放：阶段、pass 族、单 pass 占比 | TSV |
| `cargo-flags.sh WORKDIR [CARGO]` | 三 crate 工作区，记录 cargo 给 rustc 的 `--emit`、`-C` 与 `--extern` 类型 | `cargo-flags.tsv` |
| `make-tables.sh WORKDIR CENSUS_DIR` | 由以上输出生成本目录全部数字表；README 总表的定性列（族 → pass 名、类、位置、kfrc 承接）来自 `families.tsv`，次数与占比由脚本填入 | `tables/*.md` |

复现（WORKDIR 任选空目录；CENSUS_DIR 是前期材料的 `mirx/e2b/runs/20261010-rs-prof-diag/results`）：

```sh
scripts/pipelines.sh "$W/pipelines" rustc
scripts/codegen-passes.sh "$W/cg" rustc llc
scripts/cargo-flags.sh "$W/cargo" cargo
scripts/make-tables.sh "$W" "$CENSUS_DIR"
```

## 前期 E2B 逐 pass 数据的口径

- 来源：第六轮诊断 sandbox（2026-10-10，stage1 rustc `e408947b` + 实验补丁，开关全关为 A 臂，Z 臂为 `-Zmono-dedup -Zshare-drop-glue=all -Zthinlto-postlink-skip=imported,clean -Zmono-inline=30`）。
- 方法（原文）：`-Csave-temps` 保存的 bitcode 用 apt 的 LLVM 21 重放，pre-link `opt -passes=thinlto-pre-link<O3>`、post-link `opt -passes=thinlto<O3>`、codegen `llc -O3`，每个 CGU 单进程、`-time-passes`。rg 取最大的 8 个 crate（Z 下占 LLVM 时间的 86%）。
- 本目录的汇总规则（`census-passes.jq` 注释）：剔除 opt/llc 默认运行而 rustc 不跑的 IR verifier；pre/post-link 的分析计时与 pass 计时互斥，相加；llc 的指令选择/寄存器分配子分组不再相加。占比的分母是三阶段计时之和，**不是**进程 CPU，也不含 bitcode 解析。原报告“InstCombine 两阶段合计 10%”用的分母不同，所以与下文 13.3% 不矛盾。
- 重放用 `opt` 的默认 `PipelineTuningOptions`，与 rustc 不完全一致（例如 rustc 显式设置向量化与 MergeFunctions，见 [opt-levels.md](opt-levels.md)）；原报告已用 `-enable-merge-functions` 对齐 MergeFunctions。

## 已知结论的核对结果

| 结论 | 核对 | 依据 |
|---|---|---|
| post-link 把整条 simplification 流水线再跑一遍 | **成立**，只少开头的“前端输出清理”和几个只做一次的 module pass | 实测 `diff-thinpre-only-O3.txt`，见 [thinlto-postlink.md](thinlto-postlink.md) |
| rg 上导入体重新简化占 post-link CPU 40% | **成立**（离线 `opt`：13.17 s → 去掉导入体 7.87 s） | 前期 thinlto-postlink 笔记 §3 |
| `-Clto=off` 省 27–29% 编译指令、运行慢 6–28% | **需改写**。原表 O 臂（= B 臂 + `-Clto=off`，B 自身相对基线 rg −6.06%、cargo −6.41%）相对基线：rg −31.61%、cargo −28.74% 指令；运行时 rg +1.50%…+27.83%、cargo +5.79%…+8.71% | 前期 thinlto-postlink 笔记 §4 |
| `--emit=llvm-ir` 隐式把 CGU 设为 1 | **成立**，并关闭本地 ThinLTO；显式 `-Ccodegen-units=16` 且带 `-o` 时警告后同样重置为 1 | 实测 `matrix.tsv` 末两行；[config.rs#L2171](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L2171) |
| MIR pass 代价落在 rmeta 之前，串行 | **成立**，且库 crate 的单态化收集与 CGU 划分也在 rmeta 之前 | [timeline-rustc.md](timeline-rustc.md) |
| 签名优先比 P1 再降 28.5% | **只是模型上限**（CPU 不设上限）；M3 在 8 vCPU 实测 rg −3.3%、cargo 项目无变化 | 前期 p2b-m0、p2b-m2-m3 |
