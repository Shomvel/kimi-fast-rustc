# RS1 整数 intrinsic：保留到目标指令选择

对应 issue #5 方向 1。事实账本条目 RF1–RF3（[facts.md](facts.md)）。

## 目标

`ctpop`、`ctlz`/`cttz`（含 `_nonzero`）、`bswap`、`bitreverse`、`rotate_*`、
`unchecked_funnel_*`、`saturating_*` 以目标无关节点穿过 R/S/L，携带各自的输入契约，
在 x86 指令选择时按函数的有效 CPU 特性选机器指令或紧凑回退；常量折叠和代数规则在 SoN 上做。

## 基线现状（`37cea66`）

| 项 | 证据 | 后果 |
|---|---|---|
| 位计数、字节/位反转逐位展开 | `convert/intrinsics.rs:350-354` → `convert/intrinsics/integer.rs:65-132` | u64 `ctpop` ≈ 325 个 Body 语句与常量，`ctlz`/`cttz` ≈ 453，`bitreverse` ≈ 388，`bswap` ≈ 52（按代码静态计数） |
| `*_nonzero` 与普通版同一展开 | `integer.rs:91-124` | 零输入 UB 契约在转换器丢失 |
| rotate/funnel 为移位组合 | `integer.rs:6-63` | ≈ 15 个节点；量的掩码吸收了 funnel 的越界 UB |
| saturating 为无分支算术 | `convert/intrinsics/saturating.rs:6-79` | u64/i64 ≈ 14/22 个节点 |
| x86 无 `popcnt/lzcnt/tzcnt/bsf/bsr/bswap/rol/ror/shld/shrd/cmov` | `kfrc-x86/src/machine/x86.rs:55-171,350-500` | 展开后的 u64 `trailing_zeros` 估计 400+ 条虚拟指令；LLVM 为 `bsf`+`cmov` |
| 目标特性固定为 `fxsr/sse/sse2/x87` | `rustc_codegen_kfrc/src/lib.rs:92-106` | 不读 `-Ctarget-feature`、`-Ctarget-cpu` 与 `#[target_feature]` |
| `instcombine` 只折叠全常量输入 | `transforms/scalar/instcombine.rs:111-173` | 部分常量和恒等式（`bswap∘bswap`、`rotate(x,0)`）在展开后无法识别 |

rustc 1.94.1 的 `LowerIntrinsics` 在任何 mir-opt-level 下都把 `unchecked_*`、`wrapping_*`、
`*_with_overflow`、`three_way_compare`、`assume` 改写成 MIR 运算
（`rustc_mir_transform/src/lower_intrinsics.rs:77-136`），因此 kfrc 的同名分支
（`convert/intrinsics.rs:329-345`）只在 RS3.1 的 `exact_div`、`disjoint_bitor` 与
`ptr_offset_from` 中仍有作用。`carrying_mul_add`、`ptr_guaranteed_cmp` 和 128 位位运算
在 `intrinsics.rs:381` 显式报 unsupported，本方向保持该行为，单列为后续项。

这些 intrinsic 位于真实热路径：hashbrown 组掩码的 `cttz_nonzero`、SipHash 的
`rotate_left`、memchr SWAR 回退，以及 semver `Identifier` 的 `rotate_right` 与
`leading_zeros`（[semver-frontier](../semver-frontier.md)）。

## IR 设计

按 `MulOverflow` 的先例新增纯值 opcode，各自声明元数与类型规则：

| 节点 | 输入 | payload | 契约 |
|---|---|---|---|
| `BitCount` | x | kind ∈ {Ones, LeadingZeros, TrailingZeros}，位 `ZERO_POISON` | 结果 u32，范围 `[0, bits]`；带 `ZERO_POISON` 时 x=0 得 poison |
| `ByteSwap` / `BitReverse` | x | — | 全函数 |
| `FunnelShift` | hi, lo, amount | dir ∈ {Left, Right}，位 `AMOUNT_IN_RANGE` | 带该位时 amount ≥ bits 得 poison；不带时量取模（即 rotate） |
| `SaturatingAdd` / `SaturatingSub` | a, b | — | 全函数，符号取自 Rust 类型 |

`rotate_*` 生成 `FunnelShift(x, x, amount)`，不带 `AMOUNT_IN_RANGE`；`unchecked_funnel_*`
带该位；`*_nonzero` 带 `ZERO_POISON`。poison 语义与设计文档的标志语义一致
（[values-and-types](../../../design/ir/values-and-types.md)）。

**触点清单**（以 `MulOverflow` 的出现位置为准，逐一覆盖）：`graph.rs` opcode 与纯值判定、
`verify.rs` 元数与类型、`interpret.rs` 与 `interpret/rust.rs` 求值、`gvn.rs` 键、
`transforms/ipo/mod.rs` 内联白名单、`transforms/ipo/types.rs` 类型重映射、
`division.rs::known_defined`、`lower_layout/values.rs` 透传、x86 `lower.rs` 选择。

## 有效 CPU 特性

新增“有效特性集合”，作为 `lower_function` 的输入：会话级取 `-Ctarget-cpu`/`-Ctarget-feature`，
函数级取 `codegen_fn_attrs(def).target_features`。`target_config` 向 `cfg` 报告的集合与
会话级集合保持一致。PoC 默认目标为 x86-64 v1，只启用基线指令。

指令与特性：`bsf`、`bsr`、`bswap`、`rol`、`ror`、`shld`、`shrd`、`cmovcc` 属于基线；
`popcnt` 需 POPCNT；`lzcnt` 需 LZCNT；`tzcnt` 需 BMI1。`tzcnt` 的编码是 `rep bsf`，
在无 BMI1 的 CPU 上按 `bsf` 执行，仅对非零输入与 `tzcnt` 等价，因此只用于 `ZERO_POISON` 版本。

## 分步计划

| 步骤 | 内容 | 主要文件 | 退出条件 | 依赖 |
|---|---|---|---|---|
| RS1.0 计数 | 转换器按名字统计 intrinsic 调用点；记录每个调用点的 Body 语句、S/L 节点与机器指令 | `convert/intrinsics.rs`、`profile.rs` | semver 与 W02 fixture 的基线表由脚本生成 | — |
| RS1.1 节点与回退 | 上表节点；解释器与 verifier；全常量折叠；x86 用现有 `VirtualInstr` 生成紧凑回退（SWAR popcnt、对数步 bswap/bitreverse、二分 ctlz/cttz、移位 rotate） | kfrc-body `Rvalue`、`build/values.rs`、`graph.rs`、`verify.rs`、`interpret*.rs`、`instcombine.rs`、x86 `lower.rs` | 每个 u64 调用点 S 节点 ≤ 3、机器指令 ≤ 30；三臂差分与生成式对拍通过 | — |
| RS1.2 代数规则 | `bswap∘bswap`、`bitreverse∘bitreverse`、`rotate(x,0)`、同向 rotate 合并、`ctpop(x)==0 ⇔ x==0`；结果范围 `[0,bits]` 交给 RS3.2 的范围分析 | `instcombine.rs`（规则 DSL 就绪后迁入） | 每条规则有 ≤ 8 位穷举测试 | RS1.1 |
| RS1.3 基线原生指令 | 64 位 `bswap`、`rol/ror`、`shld/shrd`、`bsf/bsr`；`cmov` 随 LP5 | `machine/x86.rs`、`x86_encode.rs`、`machine/eval.rs`、分配器约束（计数固定在 CL） | 机器求值器与编码器单测；semver 34/34 | LP2 分配器接口合入；LP5（cmov） |
| RS1.4 特性门控 | 有效特性集合；`popcnt`/`lzcnt`/`tzcnt`；函数级 `#[target_feature]` | `rustc_codegen_kfrc/src/lib.rs`、x86 lowering 入参 | 默认配置的目标文件中 objdump 找不到这三条指令；带特性的函数使用它们 | RS1.3 |
| RS1.5 窄宽度 | `rol r32`、`bswap r32`、`rol r16, 8` | x86 lowering | i8/i16/i32 fixture 无额外规范化 | LP4 宽度规则 |

`carrying_mul_add`（`mul` 的 rdx:rax）、128 位位运算、`BinOp::Cmp` 的 `setcc` 形式
（`build/semantic/compare.rs:18-99` 现为控制菱形）记为后续，进入 [README](README.md#延后项) 的延后表。

## Fixture 与负例

```rust
#[inline(never)] pub fn pop(x: u64) -> u32 { x.count_ones() }
#[inline(never)] pub fn tz(x: u64) -> u32 { x.trailing_zeros() }          // 输入含 0
#[inline(never)] pub fn tz_nz(x: core::num::NonZeroU64) -> u32 { x.trailing_zeros() }
#[inline(never)] pub fn lz8(x: u8) -> u32 { x.leading_zeros() }
#[inline(never)] pub fn bs(x: u64) -> u64 { x.swap_bytes() }
#[inline(never)] pub fn rb(x: u32) -> u32 { x.reverse_bits() }
#[inline(never)] pub fn rot(x: u64, n: u32) -> u64 { x.rotate_left(n) }   // n ∈ {0, 63, 64, 1000}
#[inline(never)] pub fn sat(a: i32, b: i32) -> i32 { a.saturating_add(b) }
pub fn kconst() -> u32 { 0xF0F0u64.count_ones() }                          // 折叠为常量
```

复用 `rustc_codegen_kfrc/tests/p1-memory/bitops.rs`、`saturation.rs` 与
`scripts/poc-validation/integer-intrinsics.sh`。真实载体：semver `Identifier`、hashbrown 查找、SipHash。

负例（编号沿用 [acceptance.md](acceptance.md#负例编号)）：

- N1.1 `ctlz/cttz(0)` 等于位宽；
- N1.2 rotate 量为 0 与 ≥ 位宽；`fshl(a,b,0)=a`，`fshr(a,b,0)=b`（`integer.rs:45-57` 的注释）；
- N1.3 i8/i16/isize 的符号扩展与规范化；
- N1.4 默认配置下 objdump 中无 `popcnt/lzcnt/tzcnt`；
- N1.5 128 位位运算保持显式 unsupported。

`cttz_nonzero(0)` 属于源 UB 输入，只用于确认解释器报 UB（红线 R3）。

## 指标

每调用点的 Body 语句、S/L 节点、机器指令、spill；`.text`；`KFRC_PROFILE` 的
`build`/`optimize_s`/`layout`/`x86` 窗口；SipHash `HashMap` 插入、hashbrown 查找与 memchr 回退微基准的 runtime。

## 风险

| 风险 | 处理 |
|---|---|
| 测试主机具备 POPCNT/BMI1，误发指令也能运行通过 | 以 objdump 与特性断言（N1.4）为准，运行通过只作补充 |
| `tzcnt` 与 `bsf` 零输入行为不同 | `tzcnt` 仅用于 `ZERO_POISON` |
| 窄类型旋转在 64 位寄存器上错位 | RS1.5 之前先规范化输入 |
| 新 opcode 漏改某个 match | 按触点清单逐项提交，并加编译期穷尽匹配 |
| 与 LP2 改写的 x86 文件冲突 | RS1.3 起在 LP2 合入后的文件上实现 |
