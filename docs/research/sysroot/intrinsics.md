# intrinsic：定义、引用与对后端的要求

脚本：[intrinsics.sh](scripts/intrinsics.sh)（`list` 参数输出逐项明细）。统计 `core/src/intrinsics/{mod,simd,gpu}.rs` 中带 `#[rustc_intrinsic]` 的函数。“被引用”的判定是近似、偏宽的：在 core（含 `#[path]` 引入的 core_arch x86/x86_64 与 core_simd）、alloc、std 的非定义文件里，出现 `intrinsics::NAME`/`simd::NAME`，或 NAME 出现在一条涉及 `intrinsics` 的 `use` 里，或文件 glob 导入了 `intrinsics::simd::*` 且出现 `NAME(`。

## 总量

```
# 定义：按文件
gpu.rs 1
mod.rs 224
simd.rs 73
# 按类别：category defined const_fn with_fallback_body referenced
float 103 57 8 92
simd-generic 73 60 2 57
int 30 30 6 30
memory-ptr 30 21 5 23
atomic 17 0 0 17
type-layout 17 15 0 16
control-hint 9 6 3 9
const-eval 8 8 7 8
check-config 4 4 4 2
other 4 2 0 0
varargs 3 0 0 2
TOTAL 298 203 35 256
# float 类中 f16/f128 版本（名称以 f16/f128 结尾）：defined referenced
46 42
# 未被引用的定义（42 个）
```

共 298 个定义，其中 256 个被引用，203 个是 `const fn`（CTFE 可能要执行它们），35 个带回退体。

**回退体可以直接用：** 带函数体的 `#[rustc_intrinsic]` 表示“后端可以替换，不替换就调用这个体”。这 35 个中，`carrying_mul_add`、`disjoint_bitor`、`rotate_left/right`、`unchecked_funnel_shl/shr`、`minimum*/maximum*`（8 个）、`typed_swap_nonoverlapping`、四个 `prefetch_*`、`assume`、`cold_path`、`select_unpredictable`、`ub_checks`、`overflow_checks`、`contract_check_*` 首版都可以走回退体。`const_allocate`/`const_deallocate`/`const_make_global`/`ptr_guaranteed_cmp`/`is_val_statically_known`/`type_id_eq`/`type_of` 的回退体给的是运行时语义，CTFE 仍要自己实现。

## 按类别看要求

| 类别 | 定义/引用 | 对 kfrc 的要求 | 首次需要的档位 |
|---|---|---|---|
| int | 30/30 | 溢出检测（`add/sub/mul_with_overflow`）、`unchecked_*` 的 UB 前提、`exact_div`、`ctpop/ctlz/cttz(_nonzero)`、`bswap/bitreverse`、`three_way_compare`、`saturating_*`；i128/u128 版本要调用 compiler-builtins | hello world |
| float | 103/92 | f32/f64 的 `sqrt/fma/floor/ceil/trunc/round*/copysign/fabs/min/max` 映射到 SSE 指令或 libm；`sin/cos/exp*/log*/pow*` 是 libcall。f16/f128 版本定义 46、被引用 42：要实现 f16/f128 的布局、SysV ABI（XMM 传递）和软浮点 libcall，名字与 LLVM 发出的一致（`__addtf3`、`sinf128` 等） | hello world（core 与 compiler-builtins 都有非泛型的 f16/f128 代码） |
| atomic | 17/17 | `atomic_*` 的 ordering 是 const 泛型参数（`const ORD: AtomicOrdering`）；`cxchg(weak)`、`xadd/xsub/and/nand/or/xor/max/min/umax/umin`、`fence`、`singlethreadfence`；128 位 cmpxchg 只在 portable-atomic 等依赖里出现 | hello world |
| memory-ptr | 30/23 | `copy(_nonoverlapping)`、`write_bytes` 走 memcpy/memmove/memset；`volatile_*`；`ptr_offset_from(_unsigned)`、`ptr_metadata`、`aggregate_raw_ptr`（宽指针构造）、`transmute(_unchecked)`、`read_via_copy`/`write_via_move`、`raw_eq`、`compare_bytes`（memcmp）、`slice_get_unchecked` | hello world |
| type-layout | 17/16 | 编译期常量：`size_of/align_of/needs_drop/variant_count/type_id/type_name`、`vtable_size/align/for`、`assert_*_valid`、`caller_location`（`#[track_caller]` 隐式参数）、`discriminant_value` | hello world |
| control-hint | 9/9 | `abort`（ud2/trap）、`unreachable`、`black_box`、`breakpoint`、**`catch_unwind`**（需要 landing pad，见 [runtime-link](runtime-link.md)） | hello world |
| const-eval | 8/8 | `const_eval_select`（15 个文件引用）在 CTFE 选 const 分支、在运行时选另一分支；`const_allocate` 等是 const heap | hello world（CTFE） |
| simd-generic | 73/57 | `simd_shuffle`（索引是 const 数组）、`simd_bitmask`、`simd_select(_bitmask)`、比较、算术、移位、`simd_cast/as`、`simd_reduce_*`、`simd_gather/scatter`、`simd_masked_load/store` 等。引用主要来自 core_arch 与 core_simd 的 `#[inline]` 函数，用到才需要生成代码 | ripgrep（memchr、aho-corasick、hashbrown 的 SSE2 组） |
| varargs | 3/2 | `va_arg`、`va_copy`：std 的 `VaList`；ripgrep/Cargo 不定义 C 可变参数函数（推断，未逐个核实） | 可推迟，需显式拒绝 |
| check-config / other | 4/2、4/0 | `ub_checks`、`overflow_checks` 在**生成代码的那个 crate** 按它的配置求值（与 `#[rustc_inherit_overflow_checks]` 配合）；`autodiff`、`offload`、`amdgpu_dispatch_ptr`、`rustc_peek` 不需要 | hello world / 不需要 |

未被引用的 42 个（脚本列出名单）包括 `fadd_fast` 一族、`unaligned_volatile_*`、`volatile_set_memory`、大部分 `simd_f*` 数学函数和 `simd_*_dyn`，可以先报“不支持”。

## 与 LLVM 的关系

`core::intrinsics` 是编译器契约，和 LLVM 无关：每一个都要由 kfrc 后端（或 CTFE）实现。凡是 rustc 把 intrinsic 降成 LLVM intrinsic 或 libcall 的地方（例如 `ctpop` → `llvm.ctpop`，`sinf64` → `sin`，`fmaf128` → `fmaf128`），kfrc 都要自己选指令或 libcall 名，**libcall 名必须和 LLVM 的选择一致**，否则链接 compiler-builtins/libm 时会缺符号。直接写 `#[link_name = "llvm.*"]` 的是另一类问题，见 [llvm-asm-arch](llvm-asm-arch.md)。
