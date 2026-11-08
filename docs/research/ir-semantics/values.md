# 值域：R/S/L 各级的值是什么

本文件给出推荐的值域。标“推断”的是本调研的设计判断，其余结论有来源（见 [prior-art-rust.md](prior-art-rust.md)、[prior-art-llvm.md](prior-art-llvm.md)）。

## 总表

| 值 | R/S 级（抽象值，仿 MiniRust `Value`） | L 级（`MachineTy`） | S→L 的关系 |
|---|---|---|---|
| 整数、`bool`、`char` | `Int(n)`，宽度和符号来自类型；`bool`/`char` 带有效范围 | `Int{bits}` 位串 | 补码编码；范围外的位串没有对应的 S 值 |
| 浮点 | IEEE 位串（不是实数） | `Float{bits}` | 恒等 |
| 瘦指针 | `Ptr{addr, prov}`，`prov = Option<(AllocId, 分区标签)>` | `Ptr`（同样带 prov） | 恒等，provenance 不丢 |
| 宽指针 | `WidePtr(Ptr, Meta)`，`Meta = Len(usize) \| VTable(VTableKey)` | 两个标量（ScalarPair） | 按 PassMode 拆 |
| struct/tuple/数组/闭包环境 | `Tuple(List<Value>)` | 拆成标量、≤8 字节整数，或 `ValueSlot` 字节 | `encode(layout)`：字段落到偏移，padding 为 `Uninit` |
| enum | `Variant{idx, fields}` | tag/niche 位串 + 字段 | `encode` 依 `VariantEncoding` 答案 |
| union、`MaybeUninit<T>` | `Bytes(List<AbstractByte>)` | 同样的字节 | 恒等 |
| ZST | `Unit` | 无 | 无 |
| poison | 标量叶子上的 `Poison` | 同样的 `Poison` | `Poison` ↦ `Poison` |
| 内存字节 | `AbstractByte = Uninit \| Init(u8, Option<Prov>)` | 相同 | 内存三级共用 |

`Exception`（`Landing` 产出）和 `Effect`/`Control` token 不是 Rust 值，见 [execution.md](execution.md)。

## 三个关键选择

**1. 内存三级都是字节级，SSA 值才分级。** unsafe 代码可以 transmute、按字节拷贝、经裸指针改写任意类型，所以内存不能是“按类型的格子”。MiniRust 和 Miri 都用“抽象字节 + provenance”。由此得出：R 级的 `Store{ty}` 要把值 `encode` 成字节，**语义上需要布局**。R 图“不出现布局常量”是语法限制；它的语义 ⟦R⟧ 以答案表为参数（[refinement.md](refinement.md)）。

**2. 不要 undef，要 poison（推断，依据 LLVM 的教训）。** LLVM 的 undef 每次使用可取不同值，导致 `x+x ≠ 2*x` 一类的已知不健全优化（Lee 等 PLDI '17）。kfrc 从零开始，没有兼容包袱：

| 概念 | kfrc 的表示 | 来自 |
|---|---|---|
| 未初始化内存 | 字节 `Uninit` | MiniRust |
| union/`MaybeUninit` 的未初始化部分 | `Bytes` 里的 `Uninit` | MiniRust `Value::Union` |
| 投机产生的“本不该算出”的值 | 标量 `Poison` | LLVM poison |
| undef | **不设** | — |
| freeze | **首版不设**；只有 Select→分支、循环外提条件这类变换需要它，出现时再加 | LLVM `freeze` |

**3. `noundef` 不是“全部类型”的事实。** [values-and-types.md](../../design/ir/values-and-types.md#值事实) 写“全部类型的值 `noundef`（padding 除外）”。rustc 只在标量 `!is_uninit_valid()` 时给 load 发 `!noundef`（[builder.rs#L699](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L699)、[rustc_abi lib.rs#L1594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L1594)）：union 标量（含 `MaybeUninit`）没有。Rust Reference 也只要求整数、浮点、裸指针“已初始化”，读未初始化内存只在 union 和 padding 中允许。改法见 [recommendations-soundness.md](recommendations-soundness.md) S2。

## poison 从哪里来、在哪里变成 UB

| 产生 poison | 说明 |
|---|---|
| `Add.nsw`/`Add.nuw`/`Mul.nsw`、`Shl.unchecked` 溢出 | MIR 的 `AddUnchecked` 是立即 UB（CTFE 直接报错，[operator.rs#L101](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_const_eval/src/interpret/operator.rs#L101)），rustc 降成 LLVM `add nsw`（poison，[builder.rs#L480](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/builder.rs#L480)）。立即 UB → poison 是合法的 refinement |
| `Load.spec` 解码失败 | 投机读到的字节不是该类型的合法值（`bool` 为 2、`&T` 为空、整数含 `Uninit`） |
| `Transmute.spec` 解码失败 | 同上 |
| poison 参与的纯运算 | 逐叶传播；`Select` 只传播被选中的一臂和条件 |

| UB 点（poison 到这里即 UB） | 理由 |
|---|---|
| `If`/`Switch`/`VariantSwitch` 的条件 | 与 LLVM 相同：按 poison 分支是 UB |
| 内存访问的地址、`Div`/`Rem` 的除数 | 地址/除数无法“延迟” |
| `Call` 实参、`Return` 值 | Rust Reference：值“传给函数/从函数返回”即被产生，产生无效值是立即 UB |
| 固定的 `Load` 解码失败（不是 poison，直接 UB） | Rust：从 place 读出无效值是立即 UB |
| `Refine` 断言的事实不成立 | 见 [execution.md](execution.md#refine) |

`Store` 一个 poison 标量**不是** UB，写入 `Uninit` 字节（推断）：这样 L 级把聚合放进 `ValueSlot`、溢出到栈时不会引入 UB；原程序里能流到 `Store` 的 poison 只可能来自源程序已有的 UB。

## 浮点

Rust 的 NaN 规则（`f32` 文档 “NaN bit patterns”）：算术运算返回 NaN 时，符号非确定，quiet 位与 payload 从“preferred / 传播 / 目标特定”中**非确定选择**；选择在运算执行时发生，结果是稳定位串；x86_64 没有目标特定 payload。对 kfrc 的含义：

| 事项 | 规则 |
|---|---|
| 值域 | 浮点值是位串；运算语义返回“允许的位串集合” |
| 解释器 | 取一个确定选择（建议与 x86_64 硬件一致），对拍按集合比较 |
| GVN 合并两个相同浮点运算 | 合法（两个非确定选择合成一个是 refinement） |
| 复制一个运算（再物化、尾复制）后一份被常量折叠 | 只在常量折叠与目标硬件选同一 NaN 时合法；否则 `x.to_bits() == x.to_bits()` 可能为假（推断） |
| `x + 0.0 → x`、重结合 | 不合法（`-0.0`、舍入） |
| `neg`/`abs`/`copysign`/`to_bits` | 保留位串，只改符号位；可当位运算处理 |
