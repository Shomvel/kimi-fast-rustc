# candidate13 实测差距

所有数字来自 `evidence/poc-semantic-only-20261109/`：`candidate13-acceptance.json`、
`candidate13-attribution.tar.gz`（`semver-performance/`、`graph-attribution/`、`total-profile/`）。
semver 1.0.28，rustc 1.94.1，O3、16 CGU，共享 `-Zmir-opt-level=0` std，E2B 8 vCPU。

## 六项门槛中的三项

| 指标 | kfrc | LLVM | 比值 | 门槛 |
|---|---:|---:|---:|---|
| 目标 crate `.text` | 983,122 B | 18,515 B | 53× | ≤ 1.0 |
| parse runtime 中位数 | 144.1 ms | 2.84 ms | 51× | ≤ 1.10 |
| req | 354.6 ms | 2.22 ms | 160× | ≤ 1.10 |
| identifier | 138.5 ms | 9.53 ms | 15× | ≤ 1.10 |
| 编译指令（总窗口） | 5.56 G | 1.67 G | 3.34× | ≤ 0.33× |
| 编译 wall 中位数 | 0.67 s | 0.26 s | 2.6× | 仅参考 |

编译指令的窗口从 `codegen_crate` 到 `join_codegen`，**含 MIR 转换**，比 metrics.md 的边界宽（见 LP0）。
c12→c13 编译指令 +10.8%，尚未归因。

## `.text` 的去向

- kfrc 有 250 个 text 符号，LLVM 有 81 个。
- 23 个 alloc/Layout/Alignment 相关符号共 447,475 B，占 kfrc `.text` 的 45.6%。
  统计来源是 `kfrc-symbol-sizes.txt`（`nm -S` 的大小列是十进制，250 个符号合计 981,217 B，与 `text-attribution.json` 一致）。

| 符号 | kfrc | LLVM |
|---|---:|---:|
| `RawVecInner::finish_grow` | 70,317 B | 337 B |
| `Global::grow_impl_runtime` | 60,236 B | 内联 |
| `Layout::pad_to_align` | 44,181 B | 内联 |
| `raw_vec::layout_array` | 41,947 B | 内联 |
| `RawVec<Comparator>::drop` | 41,262 B | 40 B |
| `Alignment::as_usize` | 14,714 B（3,879 条指令） | 内联 |

`Alignment` 是 64 个变体、判别值为 `1<<k` 的 fieldless enum。反汇编显示
`as_usize` 对每个变体做一次 `cmp`／`sete`／掩码运算，见 LP1。

## `from_str` 的结构

| 阶段 | 节点 | 关键计数 |
|---|---:|---|
| S 输入 | 532 | Call 38 |
| S 优化后 | 845 | Phi 66、Call 31、VariantWitness 35 |
| L | 4,859 | Phi 695、Undefined 473、Projection 1,138、Const 728、PtrOffset 475、If 150、Load 180、Store 80 |

| 机器码 | kfrc | LLVM |
|---|---:|---:|
| 指令 | 8,810 | 232 |
| mov（不含 movzx/movsx） | 7,241 | 62 |
| 以 rbp 为基址的 mov | 3,751 | 0 |
| 显式读／写内存指令 | 1,911 / 2,180 | 29 / 30 |
| 条件／无条件跳转 | 150 / 142 | 25 / 20 |
| call | 31 | 9 |
| 体积 | 45,111 B | 917 B |

L 层只有 180 个 Load、80 个 Store，机器码却有约 4,100 条访存指令，九成以上是分配器和
Phi 搬运产生的（LP2、LP3）。另外 call 是 LLVM 的 3.4 倍，说明内联也有差距（LP8 记录）。

## 代码位置（main `37cea66`）

| 现象 | 位置 |
|---|---|
| tag → 变体序号 select 链 | `crates/kfrc-son/src/lower_layout/variants.rs` `decode_tag_index` |
| 变体序号 → 判别值 select 链 | 同文件，`select_scalar` 后的循环 |
| 编码时的 select 链 | `lower_layout/variant_encoding.rs` `encode_variant` 末尾 |
| select 用掩码算术实现 | `lower_layout/variants.rs` `select_scalar`（IntCast/Negate/BitNot/And/Or） |
| 内存路径只用 5 个寄存器 | `crates/kfrc-x86/src/regalloc/memory.rs` `REGISTERS` |
| 6 个参数寄存器无条件存栈 | `crates/kfrc-x86/src/lower/memory.rs` `compile_memory_function` |
| 有访存即走内存路径 | `crates/kfrc-x86/src/lower.rs` `lower_function_inner` |
| Phi 每条边搬两次 | `lower.rs` `emit_terminator` 的 Jump 分支 |
| `Undefined` 物化为 `Imm(0)` | `lower.rs` `value` |
| If 先 setcc 再 `test`+`jne`+`jmp` | `lower.rs` `emit_terminator` 的 If 分支 |
| 每个结果都额外经过 `canonicalize` 再 Mov 一次 | `lower.rs` `canonicalize` |
| 无 LEA、cmov、`[base+idx*s+d]` 操作数 | `machine/x86.rs` 的 `Operand` 与 `MachineInstr` |
| 无循环分析 | `kfrc-son/src/analysis/` 只有 cfg/dominators/uses/memory/refinement |

## 测量缺口

- c10–c13 没有 runtime profile，“分配路径主导 runtime”目前只是推断：req 分配最多、也最慢。
- c10–c13 只测了编译指令总数，没有按阶段分项。


## candidate14：LP1 直接 tag 数据流

本轮以 main `37cea66` 为原版，candidate14 源码归档 SHA-256 为
`d05c88c7bd330c2860e61d47c71c856a2a8c8d04909823529df410c80576132f`。
同一独立 E2B（8 vCPU／16 GiB）、rustc 1.94.1／LLVM 21.1.8、共享 MIR0 std；
LLVM／原版／候选三臂交错串行，编译 5 轮、runtime 7 轮，profile 单独运行。
下表原样摘自脚本生成的 [comparison.md](../../../../evidence/poc-llvm-parity-lp1-20261109/comparison.md)。
完整回执、源文件哈希和验收范围见
[candidate14-acceptance.json](../../../../evidence/poc-llvm-parity-lp1-20261109/candidate14-acceptance.json)。

| 指标 | baseline | candidate14 | LLVM | 候选/基线 | 候选/LLVM |
|---|---:|---:|---:|---:|---:|
| 目标 crate .text (B) | 983122 | 461920 | 18515 | 0.4699 | 24.9484 |
| 编译 wall (s) | 0.67 | 0.62 | 0.26 | 0.9254 | 2.3846 |
| parse (ms) | 149.5075 | 135.9682 | 2.8043 | 0.9094 | 48.4849 |
| req (ms) | 354.4799 | 140.8180 | 3.4736 | 0.3973 | 40.5396 |
| identifier (ms) | 154.4934 | 139.5587 | 11.6006 | 0.9033 | 12.0303 |

| runtime | 候选/基线单侧 95% 上界 | 候选/LLVM 单侧 95% 上界 |
|---|---:|---:|
| parse | 0.9361 | 49.8729 |
| req | 0.4168 | 64.5485 |
| identifier | 1.0125 | 14.5880 |

上界采用按轮配对的 median ratio percentile bootstrap（20,000 次，固定 seed）；7 轮的小样本结果按诊断性估计解读。

`identifier` 中位数改善，但候选／基线的单侧上界超过 1；本轮不足以认定其稳定提速。
三个 runtime 与 `.text` 的 LLVM 门槛均未达到。

编译旧口径总窗口为 5,561,405,476 → 4,926,712,259 条指令（下降 11.41%），
候选／LLVM 为 2.9530。主要下降在 x86（28.39%）、verify_l（35.36%）、
layout（21.64%）和 object（43.33%），S 阶段基本不变；严格 Body 转换后口径仍证据不足。

机器形状与图规模的完整表见同一报告。关键结论：

- `Alignment::as_usize`：14,714 B／3,876 条 → 102 B／31 条；
  L 节点 1,168 → 15。旧 candidate13 表中的 3,879 条不是本轮重新统计结果。
- `finish_grow`：70,317 B → 4,069 B；`pad_to_align`：44,181 B → 396 B；
  `RawVec<Comparator>::drop`：41,262 B → 1,111 B。
- `from_str`：45,111 B → 44,655 B，mov 7,241 → 7,129，rbp mov 3,751 → 3,733；
  L Phi 695、Undefined 473、Load／Store 180／80、条件／无条件分支 150／142 均未变。
- 2／16／64 变体的同形按值 fixture：原版 155／995／3,875 条，候选均为 30 条／99 B；
  LLVM 均为 2 条／4 B。O(n) 成本已消除，但 ≤ 4 条的结构门槛未达到，
  剩余固定帧与搬运由 LP2 处理。

正确性：本机 582 项通过，Linux 611 项通过，各 1 项按原配置忽略；
三臂语义语料每臂 466 行一致，semver 每臂 34 项原版测试，7 套 legacy 语料，
优化关／开各 200 个生成函数 × 6 组输入通过。
W01–W10、regex-syntax、旧 p1 的不支持范围和主张 5／6 未在本轮重新验收。

### LP0 结论与剩余缺口

已取得 baseline／candidate14 的阶段计数与三项负载的 `instructions:u` 采样、
符号表和前 20 个热点 annotate。以下占比是采样的用户态指令占比，不能当作 wall 时间占比；
两次 profile 的迭代次数不同，不比较原始 period 总量。

| req 指令采样热点 | baseline | candidate14 |
|---|---:|---:|
| `finish_grow` | 13.19% | 1.79% |
| `pad_to_align` | 9.80% | 0.26% |
| `layout_array` | 9.72% | 0.40% |

分配路径确实是原版 req 的主要热点，LP1 后显著下降；不能把这一结论外推到全部负载。
原版 parse 的前两项为 `numeric_identifier`（18.80%）和 `from_str`（18.14%），
identifier 的 83.52% 在 `parse::identifier`；候选对应负载仍主要消耗在解析与字符串操作。

阶段和覆盖总窗口 96.83%（baseline）／96.44%（candidate14）。尚未隔离严格 Body 出口，
也未完成 LLVM pass 族分项、spill／Phi mov 来源标签及全部热点到 LP 项的精确映射，
因此 LP0 只能记为部分完成。下一步按 LP2 → LP3 推进；保留 parser 热循环给 LP4／LP6，
以及 req 的字符串 helper 调用与内联成本给 LP8。

## LP2 迭代证据（candidate15–20）

本轮固定同一 E2B 沙箱、LP1 candidate14 基线与共享 MIR0 std。
原始样本与冻结源码在 `evidence/poc-llvm-parity-lp2-20261109/`，每次候选独立保留。

- [candidate15 报告](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate15-comparison.md)：
  统一路径正确性通过，但 identifier 与编译指令严重回退，未验收。
- [candidate18 报告](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate18-comparison.md)：
  修复临时寄存器选择、逐指令重排队和真实死亡判断；parse／req 改善，identifier 仍退。
- [candidate19 报告](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate19-comparison.md)：
  精确生命周期空洞后，三项 runtime 均优于 LP1，但较 candidate18 的 parse／req／text 有回退。
- [candidate20 回执](../../../../evidence/poc-llvm-parity-lp2-20261109/candidate20-acceptance.json)：
  仅优化活跃分析与相邻边搬运计算，debug／release 各五套对象与 candidate19 逐字节相同；
  因此引用 candidate19 runtime，没有给它捏造新计时。编译总指令降低 10.31%，
  x86 阶段降低 23.64%，相对 LP1 总指令仍偏高。

candidate17 被生成式 checker 拒绝，保留原 seed5 输入与失败日志，未做性能计时；
新增回归在独立 candidate17 副本上重现同类错误，再由 candidate18 修复。
完整进展与尚未完成的循环／冷路径策略见 [LP2](lp2-register-allocation.md)。
LLVM 总体门槛、严格 Body 后计数、LP3–LP8 与主张 5／6 未因这轮局部改进而自动达标。
