# FnAbi、PassMode 与 Rust ABI 调整

x86_64 SysV 分类见 [sysv-x86_64.md](sysv-x86_64.md)。链接前缀固定到 `e408947b`。

## 数据结构（`rustc_target::callconv`）

- **`PassMode`**（[mod.rs#L39](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L39)）
  - `Ignore`：ZST。Rust ABI 下 ZST 必须是 `Ignore`（闭包→fn 指针转换依赖它，[abi.rs#L388](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L388)）。
  - `Direct(ArgAttributes)`：`Scalar`/`SimdVector`。sanity check 禁止 `ScalarPair` 用 `Direct`，`Memory` 只有 `"unadjusted"` ABI 允许（[abi.rs#L375](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L375)）。
  - `Pair(a, b)`：`ScalarPair` 拆成两个独立参数；b 的属性偏移 = `a.size` 按 b 对齐取整。
  - `Cast{pad_i32, cast: CastTarget}`：把内存表示“transmute”成若干寄存器块再传。
  - `Indirect{attrs, meta_attrs, on_stack}`：传指针；`meta_attrs` 非空表示 unsized 参数（指针+元数据）；`on_stack=true` 即 LLVM `byval`，值被拷到调用栈参数区。Rust ABI 断言 `on_stack == false`。
- **`ArgAttributes`**（[#L142](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L142)）：`ArgExtension{None,Zext,Sext}`、`NoAlias/NonNull/ReadOnly/NoUndef/...`、`pointee_size`、`pointee_align`。除扩展属性和 `Indirect` 的所有权约定（见下文）外，都是给 LLVM 的优化提示，不影响调用约定。
- **`CastTarget`**（[#L265](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L265)）：`prefix: [Option<Reg>; 8]` + `rest: Uniform{unit: Reg, total, is_consecutive}` + `rest_offset`；`Reg{kind: Integer|Float|Vector, size}`。x86_64 只用到 `CastTarget::from(reg)` 和 `CastTarget::pair(lo, hi)`。
- **`ArgAbi{layout, mode}`**（[#L366](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L366)），**`FnAbi{args, ret, c_variadic, fixed_count, conv: CanonAbi, can_unwind}`**（[#L587](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L587)）。`args` 可能比源码签名多（`#[track_caller]` 的位置参数）。

## 默认模式 `ArgAbi::new`（[#L381](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L381)）

ZST → `Ignore`；`Scalar` → `Direct(scalar_attrs)`；`ScalarPair` → `Pair`；向量 → `Direct`；`Memory` → `Indirect`（`indirect_pass_mode`：NoAlias、NonNull、NoUndef、CapturesAddress，[#L400](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L400)）。之后再按 ABI 调整。**FnAbi 完全由 `BackendRepr` + 少量类型信息决定**，所以布局的 `backend_repr` 必须和 rustc 一致，Rust ABI 才一致。

## 构造流程 `fn_abi_new_uncached`（[abi.rs#L483](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L483)）

1. **签名** `fn_sig_for_fn_abi`（[#L33](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L33)）：`FnDef` 取实例化后的签名；`VTableShim` 把 `self` 改成 `*mut Self`；闭包在前面插入环境参数（`&Self`/`&mut Self`/`Self`，取决于闭包 kind）；协程等另有规则。
2. **`"rust-call"`**（闭包 `call`/`call_mut`/`call_once`）：最后一个 tuple 参数被**展开**成多个参数（[#L508](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L508)）。
3. **variadic 额外参数**：调用点把超出 `fixed_count` 的实参类型作为 `extra_args` 传给 `fn_abi_of_instance`/`fn_abi_of_fn_ptr`（[block.rs#L1097](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/block.rs#L1097)–[#L1105](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/block.rs#L1105)），每个调用点一份 FnAbi。
4. **`#[track_caller]`**：`instance.def.requires_caller_location()` 为真时，在所有参数（含 extra）之后追加一个 `&'static core::panic::Location<'static>`（[#L563](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L563)、[context.rs#L2394](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/context.rs#L2394)）。只对 `InstanceKind::Item`/`Virtual`（看属性）和 `ClosureOnceShim{track_caller}` 成立（[instance.rs#L320](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/instance.rs#L320)）。
5. **虚调用**：`InstanceKind::Virtual` 的 `self` 改成瘦指针布局（`*mut ()` 的布局，类型保留），`Rc<Self>` 之类一路剥 newtype 到裸指针（`make_thin_self_ptr`，[#L675](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L675)）。
6. **标量属性** `arg_attrs_for_rust_scalar`（[#L273](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L273)）：`bool` 一律 `Zext` + NoUndef；指针按 valid_range 加 NonNull、按 pointee 加 dereferenceable/noalias/readonly。**Rust ABI 下其他窄整数没有扩展属性**。
7. **按 ABI 调整** `fn_abi_adjust_for_abi`（[#L594](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/abi.rs#L594)）：`Unadjusted` → 全 `Direct`；`is_rustic_abi()`（`Rust`/`RustCall`/`RustCold`）→ `adjust_for_rust_abi`；其他 → `adjust_for_foreign_abi`（x86_64 非 Windows 的 `C`/`sysv64` 都走 `x86_64::compute_abi_info`，[mod.rs#L656](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L656)）。
8. `can_unwind = fn_can_unwind(...)`（[ty/layout.rs#L1233](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_middle/src/ty/layout.rs#L1233)），决定调用是否需要展开边。

## Rust ABI 调整 `adjust_for_rust_abi`（[mod.rs#L711](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L711)）

x86_64 没有架构特定的 Rust 规则（只有 X86/RISC-V/LoongArch/AArch64/BPF 有）。对 `ret` 和每个参数，跳过 `Ignore`/`Cast`：

1. **返回值** size > 2×指针（16 字节）且不是 SIMD → `make_indirect`（sret）（[#L739](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L739)）。这条对 `ScalarPair` 也生效，例如两个 u128。
2. `Memory` 且 sized 且 size ≤ 8 → `Cast` 成一个 `Reg{Integer, size}`（[#L799](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L799)），可以是 3、5、6、7 字节的整数。
3. `Memory` 且 size > 8 → 保持 `Indirect{on_stack:false}`，即**传指针、不是 byval**。
4. `SimdVector` → `simd_types_indirect`（默认 true）时 `Indirect`（[#L828](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L828)）。

| 类型（Rust ABI） | backend_repr | 参数 | 返回 |
|---|---|---|---|
| `u8`/`i32`/`f64`/`&T`/`bool` | Scalar | Direct（bool zext） | Direct |
| `u128` | Scalar(I128) | Direct（LLVM 拆两个 GPR，推断） | Direct（rax:rdx，推断） |
| `&[T]`/`&str`/`&dyn Tr`/`(u64,u64)`/`(f64,f64)`/`Option<u64>` | ScalarPair | Pair | Pair |
| `(u128,u128)` | ScalarPair | Pair | Indirect（>16） |
| `[u8;3]`/`(u16,u8,u8)`/`[u32;2]` | Memory ≤8 | Cast i24/i32/i64 | Cast |
| `[u64;2]`/`(u32,u32,u32)` | Memory >8 | Indirect 指针 | Indirect sret |
| `#[repr(C)] struct F(f64)` | Memory 8 | Cast **i64（GPR）** | Cast i64 |
| `struct F(f64)`（Rust repr）| Scalar(f64) | Direct（XMM） | Direct |
| `()`、`PhantomData`、无捕获闭包 | ZST | Ignore | Ignore |

## tricky 边界

- **Indirect 参数的所有权：** 属性是 NoAlias + “callee 拿到自己的副本”（[#L400](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/callconv/mod.rs#L400) 注释）。callee 可以就地修改这块内存；caller 只有在实参是被 move 的临时值时才能直接传原地址，否则要先拷贝。
- `Pair` 返回是两个独立 SSA 值；ScalarPair 的第二个元素可能是 `Scalar::Union`（可能未初始化，例如 `Result<u32,()>` 的 Err 负载）。
- `"rust-cold"` 在非 Windows 上映射到 `CanonAbi::RustCold`，LLVM 后端用 `PreserveMost` 调用约定（[abi_map.rs#L90](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_target/src/spec/abi_map.rs#L90)、[codegen_llvm abi.rs#L696](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_llvm/src/abi.rs#L696)），callee-save 集合不同。是否出现在冻结 sysroot 里需核对（推断：std 可能用到）。
- 函数指针调用的 ABI 兼容（transparent newtype、`Option<&T>` vs `*const T` 等）靠两边 `PassMode` + `Layout::eq_abi` 一致（[lib.rs#L2202](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_abi/src/lib.rs#L2202)），用例见 [tests/ui/abi/compatibility.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/compatibility.rs)。
- `#[track_caller]` 函数取 fn 指针或进 vtable 时用 `ReifyShim`，shim 以函数定义处为位置调用原函数；带 `track_caller` 的尾调用（`become`）在收集时按 fn 指针解析（[collector.rs#L845](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_monomorphize/src/collector.rs#L845)）。
- 所有 `Assert` 失败（越界、溢出、除零）调用的 panic lang item 都带位置参数：`panic_bounds_check(index, len, location)`、溢出类 `(location)`（[block.rs#L734](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_codegen_ssa/src/mir/block.rs#L734)）。

## 对 kfrc FnAbi 设计的含义

- rustc 的 `FnAbi` 停在“IR 层参数列表”：每个 `Direct`/`Pair` 元素、每个 `Cast` 块、每个 `Indirect` 指针变成一个 LLVM 参数，**寄存器/栈槽分配由 LLVM 完成**。kfrc 没有 LLVM，需要第二层 `assign_locations(FnAbi, CallConv) -> [PartLocation]`，把每个“部件”落到 GPR/XMM/栈偏移；Rust ABI 在 x86_64 上等于“扁平部件列表按 SysV 标量规则分配”（推断，需对拍）。
- 首轮标量 FnAbi 就按 `ArgAbi{layout, mode}` + `PassMode` 枚举定义，只实现 `Ignore` 和 `Direct(Int/Pointer ≤ 64 位)`，其余模式明确报 `UnsupportedFeature`；见 [README.md](README.md#kfrc-最小闭包与移植建议)。
