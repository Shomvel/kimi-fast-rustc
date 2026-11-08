# x86_64 System V 分类（extern "C" / "sysv64"）

源码：[x86_64.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs)（注释说明算法取自 clay 语言）。Rust ABI 不走这里，见 [fn-abi.md](fn-abi.md)。

## 分类 `classify_arg`（[#L29](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L29)）

- 类别只有三种：`Int`、`Sse`、`SseUp`（[#L16](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L16)），外加 `Err(Memory)`。**没有 X87/X87UP/COMPLEX_X87**——Rust 没有 `long double`/`_Complex`，这类 C 签名无法表达。枚举顺序即合并优先级：`unify(a,b) = min(a,b)`，`Int` 压过 `Sse`。
- 每个 eightbyte 一个槽，最多 `MAX_EIGHTBYTES = 8`（AVX-512，64 字节）；超过直接 `Memory`（[#L100](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L100)）。
- 递归 `classify(layout, off)`（[#L37](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L37)）：
  1. `off` 未按**该字段自身类型**对齐且非 ZST → `Memory`。所以 `repr(C, packed)` 里错位的 u32 让整个参数走内存。
  2. `Scalar` Int/Pointer → `Int`，Float → `Sse`；`SimdVector` → `Sse`。
  3. `ScalarPair`/`Memory` → 逐字段递归（偏移 = off + 字段 offset）；`Variants::Multiple` 再把**每个变体**当 union 成员在同一 off 递归（[#L70](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L70)）。
  4. 标量/向量覆盖的 eightbyte 范围内 `cls = min(old, c)`；向量第一个 eightbyte 之后的槽记 `SseUp`。
- 后处理（[#L107](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L107)）：超过 2 个 eightbyte 时，只有“`Sse` + 全 `SseUp`”（单个大向量）能留在寄存器，否则 `Memory`；≤ 2 个时孤立的 `SseUp` 改回 `Sse`。
- 未被任何字段覆盖的槽保持 `None`（全 padding 的 eightbyte），`reg_component` 遇 `None` 停止。

## 结果转换（[#L133](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L133)、[#L161](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L161)）

- `Int` → 剩余 size < 8 时 `Reg{Integer, size}`，否则 `i64`；`Sse` 单槽 → 剩余 4 字节为 `f32`，否则 `f64`；`Sse`+`SseUp`… → `Vector` 寄存器。
- `cast_target(cls, size)`：一个或两个 `Reg`（`CastTarget::pair(lo, hi)`），例如 `{f32,f32}` → 一个 `f64` 槽（两 float 打包在一个 XMM 的低 8 字节），`{f64,u64}` → `(f64, i64)`。

## 寄存器计数 `compute_abi_info`（[#L178](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L178)）

- 预算 `int_regs = 6`（RDI,RSI,RDX,RCX,R8,R9）、`sse_regs = 8`（XMM0–7）（[#L175](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L175)）。**先处理返回值**，再按顺序处理参数，`Ignore` 跳过（ZST，含空 `repr(C)` struct，见 [tests/ui/abi/c-zst.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/c-zst.rs)）。
- unsized 参数不动（保持默认 `Indirect`）。带 `#[rustc_pass_indirectly_in_non_rustic_abis]` 的类型直接 `make_indirect` 并占一个 int 寄存器（[#L192](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L192)；x86_64 Linux 上的使用者需核对 sysroot，推断与 `VaList` 类有关）。
- 参数：统计需要的 `Int`/`Sse` 个数；够用就扣减；**不够时，聚合（含 ScalarPair）整体改为 `Memory`，寄存器预算不扣**，后面更小的参数仍可用剩余寄存器（[#L210](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L210)）。标量不够时保持 `Direct`，由 LLVM 放栈。
- `Memory` 参数 → `pass_by_stack_offset(None)`，即 `Indirect{on_stack:true}`（byval，按类型对齐）；`Memory` 返回值 → `make_indirect`（sret），并断言此前没用寄存器、扣一个 int 寄存器（sret 指针占 RDI）（[#L229](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L229)）。
- 寄存器内的聚合 → `cast_to(cast_target)`；标量参数 → `extend_integer_width_to(32)`：小于 32 位的整数加 `Sext`/`Zext`（[mod.rs#L498](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L498)）。**返回值不扩展**（只有 Darwin 扩展）。
- `c_variadic` 与 `fixed_count` 不参与分类：variadic 实参与固定参数用同一套规则（只是每个调用点单独算 FnAbi）。

## LLVM 承担、kfrc 必须自己做的部分（推断，均需对拍）

rustc 在这里只给出 PassMode；以下行为来自 SysV psABI/LLVM，不在 rustc 源码里：

1. 按参数顺序把 `Int` 部件分到 RDI…R9、`Sse` 部件分到 XMM0–7；`Cast` pair 的两个部件各自独立占寄存器（`{f64,u64}` → XMM0 + RDI）。
2. 栈参数从调用前 RSP 起向上按 8 字节槽排列，`byval` 按类型对齐（≥8），调用点 RSP 16 字节对齐；i128/u128 标量对齐 16（数据布局 `i128:128`）。
3. 返回：`Int` 部件用 RAX、RDX，`Sse` 部件用 XMM0、XMM1，混合 pair 用 RAX+XMM0；sret 时 callee 在 RAX 返回 sret 指针。
4. variadic 调用：AL = 使用的 XMM 寄存器个数上界。
5. `Sext`/`Zext`：caller 把 i8/i16/bool 扩展到 32 位；callee 是否依赖高位由编译器决定。kfrc 作为 caller 总是扩展，作为 callee 总是自己重新截断/扩展，两边都安全。
6. i128 在寄存器里占一对 GPR；只剩一个 GPR 时整体上栈（LLVM 行为）。

## tricky 边界

- `{f32, u32}` 同一 eightbyte → `Int`（Int 压过 Sse）；`{f32,f32,f32}` → `(f64, f32)` 两个 XMM；`{u8,f32}` → 一个 i64。
- 10 字节 `{u16×5}` → 两个 `Int`，第二个 `Reg{Integer, 2}`。
- 24 字节 `{u64,u64,u64}` → `Memory`：参数 byval 上栈，返回 sret。
- 已用 5 个 GPR 后再传需要两个 `Int` 的 struct：struct 整体 byval 上栈且不扣预算，紧随其后的 i64 仍进 R9（由 [#L210](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/x86_64.rs#L210) 推出）。
- 7 个指针参数耗尽 GPR（第 7 个上栈）后，`{f32×4}` 仍走 XMM0/XMM1（issue 62350，[用例](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/abi-sysv64-arg-passing.rs#L305)）。
- 判别值多变体的 `repr(C)` enum 按“所有变体 union”分类，tag 是 `Int`。
- union `{f64, u64}` → `Int`。

## 对 kfrc 的含义

- kfrc 的 `CallConv::SysV` 需要“分类（复用本文件算法）+ 分配（自写，上面 6 条）”两层；Rust ABI 只复用分配层。
- 现有 `StoreArg/LoadArg` 只覆盖“标量上栈”；byval 聚合、sret、XMM、AL、16 字节对齐槽都是新机器接口，按 T18 分项开放，未支持的组合明确报错。
