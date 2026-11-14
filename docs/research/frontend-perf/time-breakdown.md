# rustc 前端的时间花在哪：四种场景

## 口径

| 项 | 内容 |
|---|---|
| 平台 | E2B 8 vCPU / 16 GiB，Ubuntu 24.04，官方 rustc/cargo 1.94.1（`e408947b`），`-j8` |
| 输入 | ripgrep 15.2.0、Cargo 0.95.0，固定 `Cargo.lock`（指纹与 2026-10-10 release 运行相同） |
| 场景 | check：`cargo check`；dev：`cargo build` 默认 dev profile，`CARGO_INCREMENTAL=0`；增量：先增量冷构建，再在叶子 crate 的一个函数体里加 `let _ = 1;` 后重建（rg 只重编 rg bin，Cargo 重编 cargo lib+bin）；release：沿用 2026-10-10 同模板另一 sandbox 的官方工具链运行 |
| 采集 | `-Zself-profile`，事件 `generic-activity,query-provider`，每个进程用 measureme `summarize` 归约；按 label 的 self 时间累加（近似 CPU）；协调线程的等待单列，不计入“工作” |
| 分类 | `scripts/phase-lib.jq`；前端 = `fe_*` + `md_*`（metadata 读写）；MIR 拆成“构建”（借用检查与 CTFE 需要）与“优化” |
| 样本 | 每场景 1 次；self-profile 本身使构建慢 4–6%（前期 next-directions §5），只看比例 |

已知偏差：外部 `mod` 文件在展开中解析，`fe_parse_expand` 含大部分解析（[why-slow-passes.md](why-slow-passes.md)）；借用检查里 MIR 类型检查发起的 `type_op_*` 计入 `fe_trait`；check 构建仍完整编译构建脚本和过程宏，所以有少量 `be_*`。

## 1. 场景总量（`scripts/scenario-table.sh`）

| 项目 | 场景 | 墙钟 s | user+sys s | instructions:u G | rustc 进程 |
|---|---|---:|---:|---:|---:|
| rg | check | 3.63 | 11.27 | 41.99 | 45 |
| rg | dev | 6.05 | 24.07 | 96.82 | 45 |
| rg | dev-incr-cold | 7.13 | 27.79 | 105.86 | 45 |
| rg | incr | 0.86 | 1.43 | 3.05 | 1 |
| cargo | check | 28.46 | 127.43 | 517 | 365 |
| cargo | dev | 55.93 | 232.22 | 952.65 | 361 |
| cargo | dev-incr-cold | 61.82 | 237.3 | 993.92 | 361 |
| cargo | incr | 8.92 | 12.25 | 29.81 | 2 |

## 2. 前端内部占比（`scripts/phase-matrix.sh`）

| 占前端 | rg check | rg dev | rg release | rg 增量 | Cargo check | Cargo dev | Cargo release | Cargo 增量 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| fe_parse_expand | 9.5% | 7.9% | 9.6% | 8.5% | 8.5% | 6.8% | 5.9% | 14% |
| fe_resolve_lower | 8.9% | 7.4% | 11.4% | 7.7% | 8.6% | 6.9% | 6% | 15.3% |
| fe_typeck | 27.9% | 22.8% | 15% | 3.5% | 26.7% | 22.6% | 19.5% | 2.6% |
| fe_trait | 8.6% | 9.5% | 7% | 2.9% | 10.3% | 11.1% | 10.4% | 1.5% |
| fe_borrowck | 16.2% | 13.6% | 8.7% | 2% | 15.4% | 13.1% | 11.1% | 1.6% |
| fe_mir_build | 7.6% | 6.2% | 8.9% | 0.2% | 6.7% | 5.6% | 4.8% | 0% |
| fe_mir_opt | 2.2% | 11.3% | 14.9% | 0.5% | 3.2% | 9.8% | 21.7% | 0.2% |
| fe_const_eval | 1.7% | 1.9% | 1.4% | 0% | 3.8% | 5.7% | 3.4% | 0% |
| fe_layout | 0.2% | 1% | 0.7% | 1% | 0.2% | 1% | 1% | 0.2% |
| fe_lint | 2.9% | 2.4% | 1.5% | 5.3% | 2.6% | 2.2% | 2% | 8.1% |
| fe_incr_overhead | 0% | 0% | 0% | 33.3% | 0% | 0% | 0% | 25.9% |
| fe_other | 5.1% | 5.1% | 10.8% | 28.9% | 5% | 4.9% | 4.4% | 18.9% |
| md_decode | 7.2% | 8.7% | 8.3% | 6.1% | 7.1% | 8.3% | 7.8% | 4.5% |
| md_encode | 2.1% | 2.3% | 1.8% | 0% | 2% | 2.1% | 2.1% | 7.1% |
| **前端秒（累计）** | 9.1 | 11 | 17.9 | 0.38 | 78.9 | 94.7 | 110.5 | 4.9 |
| **前端占全部工作** | 91.9% | 49.7% | 25.4% | 47% | 88.8% | 49.7% | 23.1% | 58.6% |

rg release 的 `fe_other` 偏高主要是 `registered_tools` 1.01 s（45 个进程累计，`top-labels.sh`），疑为早期 crate 加载计入该事件（推断）。

## 3. 增量编辑

增量重建里前端只有 0.38 s（rg）和 4.9 s（Cargo），但几乎全是与 crate 大小成正比的固定成本（`top-labels.sh`，Cargo 两个进程累计）：

| 项 | Cargo 增量 self 秒 | 性质 |
|---|---:|---|
| `incr_comp_encode_dep_graph`（3,314,863 个节点）+ load + promotion + persist | 0.43 + 0.32 + 0.26 + 0.18 | 依赖图 O(图大小) |
| `expand_crate` + `expand_proc_macro` | 0.42 + 0.22 | 展开不增量 |
| `hir_crate` + `late_resolve_crate` | 0.4 + 0.24 | 解析与降低不增量 |
| `typeck` + `mir_borrowck` 类（大多是 green 验证） | 0.13 + 0.08 | 与改动相关的部分很小 |
| 后端：`run_linker` + `link_rlib`，单态化收集 | 1.54 + 0.68，0.55 | 整体重链接、整 crate 遍历 |

`dev-incr-cold` 比非增量 dev 多 9.3%（rg）和 4.3%（Cargo）的指令；这既含查询结果哈希（self-profile 不单列），也含 CGU 数 16 → 256 的后端差异，不能全部算作指纹成本。

## 4. 结论

1. **check 构建几乎全是前端**（89–92%），其中 typeck + trait + 借用检查占前端 52–53%，解析/展开/名字解析/降低约 17–18%，metadata 读写约 9%。
2. **dev 构建前端约占一半**；release 降到 23–25%，但 release 的前端里 MIR 优化占 15–22%，这部分 kfrc 会移到接口发布之后。
3. **增量编辑的前端成本是 O(crate) 固定项**：依赖图 26–33%，展开 + 解析 + 降低 16–29%，真正重查的 typeck/借用检查 ≤6%。rustc 的增量“正比于改动”只覆盖了 HIR 之后。
4. **关键路径上前端的比重高于 CPU 比重**：rg check 关键路径 3.58 s 中前端 3.37 s；Cargo check 28.4 s 中前端 12.23 s、构建脚本运行 12.95 s（`cargo-critical-path.sh`；release 见 [critical-path.md](critical-path.md)）。
5. 与前期 stage1 数据一致：cargo release 前端内部 MIR 构建与优化 24%、typeck 24%、借用检查 13%、trait 12%、元数据解码 6%、宏展开 6%、名字解析与降低 5%（前期 next-directions §5，分类粒度不同）。
