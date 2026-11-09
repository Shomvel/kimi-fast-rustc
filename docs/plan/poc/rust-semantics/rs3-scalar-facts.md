# RS3 标量有效值与带作用域的算术／控制事实

对应 issue #5 方向 3。事实账本条目 RF4–RF9（[facts.md](facts.md)）。

## 目标

1. unchecked 算术、精确除法、不相交按位或、合法移位范围以节点标志保留到 x86 指令选择。
2. rustc 可信产生的标量有效范围（bool、char、NonZero、NonNull、enum tag 等）登记到类型表，
   附着在类型化操作产生的值上，与“已初始化”“非空”“可解引用”分开。
3. 检查成功分支、`Assume`、归纳变量和长度关系成为有作用域的谓词事实，供冗余检查消除、
   分支裁剪与循环优化查询。

## 基线现状（`37cea66`）

| 项 | 证据 |
|---|---|
| `AddUnchecked`/`SubUnchecked`/`MulUnchecked` 与普通运算合并为同一 opcode | `kfrc-son/src/build/values.rs:28-30` |
| `ShlUnchecked`/`ShrUnchecked` 合并为取模移位；x86 因此保留 `and` 掩码 | `build/values.rs:38-42`；`graph.rs:156`；`kfrc-x86/src/lower.rs:461-475` |
| `exact_div`、`ptr_offset_from(_unsigned)` 降为普通 `Div`；非 2 的幂元素仍发 `div` | `convert/intrinsics.rs:178-196,334`；`transforms/scalar/division.rs:79-97` |
| `disjoint_bitor` 降为 `BitOr` | `convert/intrinsics.rs:318-324` |
| 算术节点 `payload` 恒为 0；GVN 键已包含 payload | `graph.rs:397-403`；`transforms/scalar/gvn.rs:20-48` |
| 类型表只记 size/align，无 `valid_range` | `kfrc-body/src/types.rs:42-97`；`convert/types.rs:68-78` |
| 解释器只检查 bool/char 的有效性 | `interpret/semantic_memory.rs:477-480`；`interpret/value.rs:289-295` |
| `Assume` 唯一的分析消费者是变体凭据；S→L 擦除时不导出事实 | `analysis/refinement.rs:68`；`lower_layout/observation.rs:7-30` |
| `PredicateFacts::predicate_at` 已实现支配与动态实例检查（`same_instance`），只被 verifier 用于 `VariantWitness` | `analysis/refinement.rs:141-145,254-286,288-356`；测试 `refinement_tests.rs:186` |
| `simplify::run` 只折叠字面常量条件；无循环分析 | `transforms/control/simplify.rs:46-67`；`analysis/` 目录 |
| Assert 转为 `SwitchInt` 加冷块 panic 调用 | `convert/terminators.rs:105-172`；`build.rs:475-500` |

rustc 对应的产出（opt-level ≠ 0）：unchecked 运算的 `nsw`/`nuw`
（`rustc_codegen_ssa/src/mir/rvalue.rs:763-797`），`exactsdiv`/`exactudiv`
（`rustc_codegen_ssa/src/mir/intrinsic.rs:271-290`），`or disjoint`（同文件 266-270），
load 的 `!range`/`!noundef`/`!nonnull`/`!align`（`rustc_codegen_llvm/src/builder.rs:699-719`），
参数与返回的 `range`/`noundef`/`nonnull`（`rustc_ty_utils/src/abi.rs:284-320`，
`rustc_codegen_llvm/src/abi.rs:443-458`），transmute 前后的 `assume_scalar_range`
（`rustc_codegen_ssa/src/mir/rvalue.rs:944-1056`；`Scalar::Union` 直接跳过，1031 行），
niche 编码中不可能 tag 的 `assume`（`mir/operand.rs:540-557`）。kfrc 以这些产出为事实来源的上界。

## RS3.1 算术标志

**载体。** 算术节点 `payload` 位集：`NUW`、`NSW`（Add/Sub/Mul/ShiftLeft）、`EXACT`（Div*/Shr*）、
`DISJOINT`（BitOr）、`AMOUNT_IN_RANGE`（Shift*）。违例得 poison。verifier 按 opcode 检查允许的位；
解释器在 `apply` 中计算违例。改写产生新节点时默认不复制标志；GVN 按标志区分节点（现状即如此）。

**生产者。** `AddUnchecked` 等：有符号置 `NSW`，无符号置 `NUW`，与 rustc 一致；
`ShlUnchecked/ShrUnchecked` 置 `AMOUNT_IN_RANGE`；kfrc-body 增加 `DivExact` 与 `BitOrDisjoint`
（或给 `Rvalue::BinaryOp` 带标志），供 `exact_div`、`disjoint_bitor` 与 `ptr_offset_from` 使用
（无符号版为 `sub nuw` + `udiv exact`，有符号版为 `sub` + `sdiv exact`，
`rustc_codegen_ssa/src/mir/intrinsic.rs:556-578`）。R 路径
（`convert/definition/emit.rs:175-229`）同步接受这些运算。

**消费者。**

1. `division.rs`：`EXACT` 的有符号 2 的幂除法变为单条算术右移；非 2 的幂常量变为
   `sar k` 加乘模逆。slice 迭代器 `len()` 与 `offset_from_unsigned` 的非 2 的幂元素直接受益。
2. `instcombine`：`(a +nuw b) <u a → false` 等少量规则。
3. x86：`AMOUNT_IN_RANGE` 省去移位掩码；窄类型 `NUW`/`NSW` 运算省去规范化，与 LP4 宽度规则一并实现。

**擦除点：** x86 指令选择。

## RS3.2 类型有效值范围

**前置：解释器检查。** `TransmuteValue` 与 `LoadValue` 解码时检查已登记的标量范围。
差分测试从此能暴露“非法值被当作合法”的错误。此项先于任何范围消费者合入。

**载体。** `RecordedTypes` 增加 `scalar_valid_range(TyId)` 与 ScalarPair 各车道范围。
生产者 `convert/types.rs` 读取 `layout.backend_repr`，仅登记
`BackendRepr::Scalar(Scalar::Initialized{..})` 且非全范围的条目。`MaybeUninit<T>` 与 union
的 layout 为 `Scalar::Union`，自然不登记，规则与 rustc 相同。pattern type 当前进入
`TypeShape::Other`（`convert/types.rs:64-66`），先确认 1.94.1 的 NonZero 实际 layout 再接入。

**不变量。** S 中由类型化操作产生的非 poison 值满足其类型的标量范围。类型化操作指构造器、
`LoadValue`、`TransmuteValue`、S 入口参数与调用返回、`MakeVariant`/`VariantOf`。
`Undefined`、`MaybeUninit` 包装、L 级原始 `Load` 与 `ConstRepr` 不携带该事实。

**分析。** 新增只读 `analysis/ranges.rs`：`range_of(node)` 按 opcode 推导（Const、Field、
`TransmuteValue` 目标、`IntCast`、常量掩码 `BitAnd`、常量 `ShiftRightLogical`、
`BitCount → [0,bits]`、`VariantOf → [0,变体数)`、Phi 取并集），以类型范围为底。

**消费者。** `Compare` 折叠（`nz.get() != 0 → true`、`c as u32 <= 0x10FFFF → true`）；
NonZero 上的 `BitCount` 置 `ZERO_POISON`；bool → 整数 `IntCast` 省去掩码；LP5 的 switch 降低
用 `VariantOf` 范围省略默认范围检查。非空只服务比较折叠；可解引用、对齐与权限由 RS4/RS5 负责。

**擦除点：** 类型范围随 Rust 类型留在 L 标量节点上；分析在指令选择前结束。

## RS3.3 作用域条件折叠

新增 `transforms/control/conditions.rs`，接在 `optimize_specialized_graph` 的 GVN 之后、
`simplify::run` 之前（`pipeline.rs:132-139`），内联后的 seam 流程同样接入：

1. 建索引“谓词节点 → 证明边”，来源为 `If` 两个出口与 `Assume` 出口；
2. 对每个 `If(c)`，沿支配树查找证明；`c` 与谓词相同或为其 `BitNot`（覆盖 `build.rs:486-487`
   的形状），且 `PredicateFacts::predicate_at(proof, if_control)` 成立时，把条件替换为常量，交给
   `simplify` 删除失败分支；
3. 加入蕴含：同操作数比较族（`a <u b ⇒ a ≤u b, a ≠ b, b ≠ 0`）、常量放宽（`i <u 10 ⇒ i <u 16`）、
   RS3.2 的范围；
4. checked 算术：在支配事实与范围下证明某次 `AddWithOverflow` 的溢出位为 false，只折叠这一次检查。

每次折叠记录（条件、证明节点），verifier 在调试构建中用 `PredicateFacts` 复查，
使错误的 `Proven` 事实以编译器错误暴露。

## RS3.4 归纳变量与循环边界检查

在 LP6 的 `analysis/loops.rs` 与归纳变量识别之上：`i ∈ [start, end)` 区间事实，`i + 1` 的 `NUW`
推导（`i < end ≤ usize::MAX`），以及 slice 长度上界 `len * size_of::<T>() ≤ isize::MAX`
作为 `IndexAddr` 乘法的 `NUW` 依据。`for i in 0..s.len()` 的形状可能已由 RS3.3 覆盖，以 fixture 实测为准。

## 分步计划

| 步骤 | 退出条件 | 依赖 |
|---|---|---|
| RS3.1 算术标志 | 下列 fixture 中掩码与 `div` 消失；负例保留标志为空；三臂差分通过 | — |
| RS3.2a 解释器范围检查 | 非法 NonZero/NonNull/char/bool 解码在解释器中报 UB | — |
| RS3.2b 范围登记与分析 | 比较折叠 fixture 生效；MaybeUninit/union 负例不折叠 | RS3.2a |
| RS3.3 条件折叠 | 重复越界检查消失；verifier 复查开启；循环动态实例负例通过 | — |
| RS3.4 循环 BCE | `for i in 0..len`、`chunks`、`enumerate` 的体内检查消失 | LP6 循环分析 |

## Fixture 与负例

```rust
pub fn ua(a: u32, b: u32) -> u64 { unsafe { a.unchecked_add(b) as u64 } }      // 无 and 0xffffffff
pub fn ushl(x: u32, s: u32) -> u32 { unsafe { x.unchecked_shl(s) } }            // 无移位掩码
pub fn ofs(a: *const [u8; 24], b: *const [u8; 24]) -> usize { unsafe { a.offset_from_unsigned(b) } } // 无 div
pub fn ilen(s: &[[u8; 24]]) -> usize { s.iter().len() }
pub fn nz(x: core::num::NonZeroU32) -> bool { x.get() != 0 }                     // → true
pub fn ch(c: char) -> bool { (c as u32) <= 0x10FFFF }                            // → true
pub fn bl(b: bool) -> usize { b as usize }                                        // 无 and 1
pub fn bc(s: &[u8], i: usize) -> u8 { if i < s.len() { s[i] } else { 0 } }       // 检查消失
pub fn bc2(s: &[u8], i: usize) -> u8 { s[i].wrapping_add(s[i]) }                 // 第二次检查消失
pub fn arr(a: &[u32; 8], i: usize) -> u32 { if i < 4 { a[i] } else { 0 } }       // 需要蕴含
pub fn asm(x: usize, s: &[u8; 16]) -> u8 { unsafe { core::hint::assert_unchecked(x < 16) }; s[x] }
```

负例：

- N3.1 `wrapping_*`、overflow-checks 关闭的 `a + b`、`checked_add` 的结果节点不带标志；
- N3.2 有符号 `offset_from` 不带 `NSW`（`rustc_codegen_ssa/src/mir/intrinsic.rs:567-568`）；
- N3.3 `MaybeUninit<NonZeroU32>` 经原始指针读 `u32 != 0` 不折叠；`union { nz: NonZeroU32, raw: u32 }` 读 `raw` 不折叠；
- N3.4 条件位于不支配的分支；`Assume` 之前的使用；
- N3.5 循环中上一轮的 `i < len` 不证明本轮；
- N3.6 `Vec` 在两次检查之间被 push，`len` 重新加载后不复用旧事实（需要 RS4 的内存依赖）。

`i32::MIN.unchecked_sub(1)`、`unchecked_shl(32)`、非法 transmute 属于源 UB 输入，只用于确认解释器报 UB。

## 指标

`OptimizeStats` 新增 `conditions_folded`、`checks_removed`（按 panic lang item 分 bounds/overflow/div）、
`fact_queries`、`fact_node_visits`；目标文件中 `panic_bounds_check` 调用点数；`div` 指令数；
x86 掩码指令数；分阶段编译指令。
