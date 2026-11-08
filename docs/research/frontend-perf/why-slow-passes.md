# rustc 前端为什么慢（二）：展开、多次建 IR、typeck/trait、借用检查

约定同 [why-slow-infra.md](why-slow-infra.md)。本机两项补测只看比例和计数：trait 重复用 Homebrew rustc 1.94.1（同 commit），借用检查拆分用 nightly 1.101.0（`12433c307` 2026-10-24，它的借用检查有更细的事件），输入都是 ripgrep 15.2.0 的 `cargo check`。

## 1. 宏展开与名称解析：串行、每次全量、解析器反复进出

| 事实 | 位置 | 性质 |
|---|---|---|
| 展开是与导入解析交错的不动点；每个片段重新收集调用；derive 解析后再入队重解析（FIXME） | [expand.rs#L493-L637](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L493-L637)、[#L571-L573](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L571-L573) | 源码 |
| `macro_rules` 按臂顺序尝试；`$e:expr` 等片段调用完整解析器；转录时把已解析的 AST **转回 token** 再交给解析器 | [macro_rules.rs#L509-L583](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_rules.rs#L509-L583)、[macro_parser.rs#L667-L693](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/macro_parser.rs#L667-L693)、[transcribe.rs#L440-L478](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/mbe/transcribe.rs#L440-L478) | 源码；失败臂重复解析为推断 |
| derive 过程宏：item → token → bridge → token → `parse_item`；缓存需要增量 + `-Zcache-proc-macros`（默认关） | [proc_macro.rs#L134-L198](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/proc_macro.rs#L134-L198) | 源码 |
| 展开 + 解析在 `eval_always` 的 `resolver_for_lowering_raw` 里，增量每次全跑 | [query/mod.rs#L212-L216](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/query/mod.rs#L212-L216) | 源码；rg 单行编辑宏展开 0.14 s/次（前期，实测） |
| 外部 `mod x;` 文件在展开中解析，`parse_crate` 只解析根文件 | [expand.rs#L1388](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_expand/src/expand.rs#L1388) | 源码；所以 `expand_crate` 的 self 时间含大部分解析 |
| 导入不动点每轮重解析全部未定导入；`finalize_imports` 再全量解析一遍；晚期解析两次整 AST 遍历；parse/expand/resolve/lowering 无任何 `par_*` | [imports.rs#L576-L660](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/imports.rs#L576-L660)、[late.rs#L5505-L5509](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_resolve/src/late.rs#L5505-L5509) | 源码 |

实测分量：cargo release 中 `expand_crate` 5.44 s、`hir_crate` 3.03 s、`late_resolve_crate` 2.18 s（364 个进程累计，`top-labels.sh`）。

## 2. 每个函数体建四种 IR，MIR 至少两份

| 形态 | 怎么来 | 位置 |
|---|---|---|
| AST（整 crate） | 解析 + 展开 | — |
| HIR | 偷走 AST，串行逐 owner 降低；输出 metadata 时逐 owner 稳定哈希 | [ast_lowering/lib.rs#L502-L540](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ast_lowering/src/lib.rs#L502-L540) |
| TypeckResults（旁表） | `typeck` 每个 body owner 一次 | [hir_typeck/lib.rs#L89-L91](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_hir_typeck/src/lib.rs#L89-L91) |
| THIR | HIR + TypeckResults，执行 adjustment | [thir/cx/mod.rs#L17-L51](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_build/src/thir/cx/mod.rs#L17-L51) |
| MIR | built → promoted → drops_elaborated → optimized 靠 `Steal` 原地传递；**借用检查深拷贝一份**；`const fn` 的 CTFE 再拷一份 | [borrowck/lib.rs#L327-L329](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L327-L329)、[mir_transform/lib.rs#L502-L503](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_mir_transform/src/lib.rs#L502-L503) |

cargo release 实测：`mir_built` 2.83 s、`thir_body` 1.17 s、`hir_crate` 3.03 s；三者合计约占前端的 6%（同上脚本，推断：去掉 HIR/THIR 两层能省的上限量级）。

## 3. typeck 与 trait：缓存只在会话内，跨 crate 重复其实不多

| 事实 | 位置 | 性质 |
|---|---|---|
| 1.94.1 typeck 用旧求解器（新求解器只用于 coherence）；`ObligationForest` 每次 `select_where_possible` 重扫挂起项；`structurally_resolve_type` 类入口在 rustc_hir_typeck 有 74 个调用点 | [config.rs#L993-L999](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_session/src/config.rs#L993-L999)、[obligation_forest/mod.rs#L433-L475](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_data_structures/src/obligation_forest/mod.rs#L433-L475) | 源码 |
| 全局 selection/evaluation 缓存在 tcx 上；含推导变量、coherence 模式或定义域内 opaque 的 goal 只进 InferCtxt 局部缓存，随它丢弃 | [ty/context.rs#L1595-L1605](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L1595-L1605)、[select/mod.rs#L1488-L1532](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_trait_selection/src/traits/select/mod.rs#L1488-L1532) | 源码；“丢弃”为推断 |
| 任何 trait 结果都不跨会话、不跨 crate | [query/mod.rs#L2515-L2519](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/query/mod.rs#L2515-L2519) | 源码 |

**跨 crate 共享 canonical 缓存的上限**（`scripts/trait-goal-dup.sh`，按打印出的 canonical key 比较 17 类 trait/type-op 查询）：

| 量 | 值 |
|---|---:|
| 查询执行次数（46 个进程） | 192,251 |
| 在 >1 个 crate 里重复计算的部分 | 约 18,877 次（9.8%） |
| trait 查询 self 时间中可省的比例 | 12.5% |
| 其中 `evaluate_obligation` | 50,805 次，可省 21.2% |

trait 类只占前端 6–10%（[time-breakdown.md](time-breakdown.md)），所以跨 crate 共享冷构建的上限约为前端的 1%。原因（推断）：每个 crate 内已有会话级全局缓存，跨 crate 剩下的 goal 大多含本 crate 的类型或泛型参数。泛型参数按名字打印，会把不同函数的 `T: Clone` 判为相同，所以这个上限偏宽。

## 4. 借用检查：贵的不是数据流

nightly 1.101 的子事件（`scripts/local-check-profile.sh` → `borrowck-split.jq`，self 时间占借用检查合计）：

| 事件 | 占比 |
|---|---:|
| `mir_borrowck` 自身（MIR 类型检查、region 推导、逐点访问检查、克隆 body 等未拆分部分） | 66.6% |
| region liveness（`borrowck_liveness`、`_trace`、`_compute`） | 12.2% |
| `check_liveness`（未用变量分析，分类脚本计入借用检查） | 4.9% |
| `mir_borrowck_implied_outlives_bounds` | 6.3% |
| 四个数据流（borrows、maybe_uninits、ever_inits、maybe_inits） | 4.5% |
| `MIR_borrow_checking`、polonius loan liveness | 5.6% |

源码：每个 body 先克隆 MIR、重编号 region、`gather_moves`、`BorrowSet::build`、MIR 类型检查（含 liveness），再求 region、跑三个数据流（[borrowck/lib.rs#L309-L384](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L309-L384)、[#L606-L620](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_borrowck/src/lib.rs#L606-L620)）。MIR 类型检查里的 `type_op_*` 查询计入 trait 类。

## 5. 小结

| 原因 | 冷构建证据 | 对 kfrc 的含义 |
|---|---|---|
| 展开/解析串行且不增量 | 前端 6–10%（parse+expand），rg 增量每次 0.14 s | 按文件并行解析；展开结果在增量里可缓存（[recommendations.md](recommendations.md)） |
| 四种 IR + MIR 克隆 | HIR+THIR+MIR 构建约 6% | 设计已跳过 THIR；HIR 级 AST 只建一次，借用检查原地加 region 旁表 |
| trait 缓存不跨 crate | 跨 crate 重复只占 trait 时间 12.5% | 冷构建不值得优先做；优先单 crate 内的求解效率 |
| 借用检查 | 前端 9–13%，其中数据流 <5% | 优化重点是 MIR 类型检查与 region 求解，不是 bitset 数据流 |
