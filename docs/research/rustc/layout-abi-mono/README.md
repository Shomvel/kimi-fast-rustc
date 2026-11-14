# 布局、调用约定、单态化与常量求值

基于官方 rustc 1.94.1（`e408947bfd200af42db322daf0fadfe7e26d3bd1`）。链接都固定到该 commit，行号已核对；标“推断”的内容没有在源码中直接验证。

## 索引

| 文件 | 内容 |
|---|---|
| [layout.md](layout.md) | `LayoutData`/`FieldsShape`/`BackendRepr`/`Scalar`、基本类型与宽指针、`univariant` 字段重排、repr 标志、union |
| [enum-niche.md](enum-niche.md) | enum 分流、niche-filling 与 tagged 两种布局及取舍、tag 宽度、判别值读写 |
| [fn-abi.md](fn-abi.md) | `PassMode`/`ArgAbi`/`FnAbi`、构造流程、Rust ABI 调整规则与阈值、`#[track_caller]`、`rust-call`、variadic |
| [sysv-x86_64.md](sysv-x86_64.md) | `extern "C"` 的 INTEGER/SSE/MEMORY 分类、寄存器计数、sret/byval、扩展；LLVM 承担的部分 |
| [instances.md](instances.md) | `InstanceKind` 全表、`Instance::resolve` 及 fn 指针/vtable/drop 变体、vtable 布局 |
| [mono-collector.md](mono-collector.md) | 根、used/mentioned 两种模式、各类边、分配引用、`should_codegen_locally`、分区与实例化模式 |
| [const-eval.md](const-eval.md) | 查询入口、`InterpCx`/`Machine`/内存与 provenance、CTFE 限制、驻留与有效性、promoted、const fn 检查、值树、常量进入 codegen |
| [test-cases.md](test-cases.md) | rustc oracle 属性、代表性 `tests/ui` 用例、x86_64 SysV 双向对拍形状表 |

## 概览：四件事怎么串起来

1. **布局**是一切的底座。`layout_of(Ty)` 产出 `LayoutData`，其中 `backend_repr`（Scalar/ScalarPair/Memory）同时决定 CTFE 的立即数形状、codegen 的值形状和 `FnAbi` 的默认传参模式。算法层 `rustc_abi::LayoutCalculator` 与 `TyCtxt` 解耦。
2. **调用约定**分两段：`fn_abi_new_uncached` 按 `backend_repr` 给出默认 `PassMode`，再按 ABI 调整——Rust ABI 用三条尺寸规则，`extern "C"` 走 SysV 分类。rustc 的结果停在“IR 参数列表”，寄存器/栈槽由 LLVM 分配。
3. **单态化**在 MIR 上从根做 DFS：调用、取 fn 指针、unsize 到 `dyn`、drop、常量里的指针、static 初始化都会引出新实例；常量求值失败在这里以“与优化级别无关”的方式报告（mentioned items）。
4. **常量求值**是一个在优化前 MIR 上运行的解释器，使用与 codegen 相同的布局；值是符号化的 `AllocId + offset`，结果经驻留与有效性检查后成为 `ConstValue`/`Allocation`，由 codegen 发射成只读数据和重定位。

依赖环：数组长度、显式判别值要常量求值；常量求值要布局；enum 布局要判别值。rustc 靠 query 系统按需求值并检测环。

## kfrc 最小闭包与移植建议

### 布局一致性范围

| 类别 | 例子 | 是否必须与 rustc 位级一致 |
|---|---|---|
| 语言/FFI 保证 | 基本类型大小对齐（i128 对齐 16）、`repr(C)` struct/union/enum、`repr(u8…)`、`repr(C,u8)`、`repr(transparent)`、数组 stride、`bool`/`char` 有效值、判别值（`as` 转换）、`Option<&T>`/`Option<NonNull>`/`Option<Box>`/`Option<NonZero*>`/`Option<fn>` 为可空指针 | 必须 |
| 未指定但可观察 | `repr(Rust)` 字段顺序、`size_of`/`align_of`/`offset_of!` 结果、niche 选择、宽指针 (ptr, meta) 顺序、vtable 前三项、Rust ABI 的 `PassMode` | 语言不保证，建议一致 |
| 只影响性能 | `-Zrandomize-layout`、`max_repr_align`、`unadjusted_abi_align`（x86_64 不用） | 不需要 |

建议 kfrc 对第二类也复刻 rustc：算法本身自包含、移植成本低；程序里 `size_of` 的输出、和 rustc 编译对象混链（过渡期 sysroot、对照 driver）、fn 指针 ABI 兼容规则（[tests/ui/abi/compatibility.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/tests/ui/abi/compatibility.rs)）都依赖它；Rust ABI 由 `backend_repr` 直接决定（例如 `#[repr(C)] struct F(f64)` 在 Rust ABI 下走 GPR，Rust repr 的同名 struct 走 XMM）。这与 [差分协议](../../../plan/README.md#验收) 中“语义测试不依赖未指定布局”不冲突：另设一类带标记的“布局/ABI 对齐测试”，以 `#[rustc_layout(debug)]`/`#[rustc_abi(debug)]` 输出为期望值（见 [test-cases.md](test-cases.md)）。

### T08 首轮标量 FnAbi（[m1](../../../plan/m1-vertical-chain.md#t08)）

- 数据结构直接按 rustc 形状定义，只实现子集：`FnAbi{args, ret, conv: Rust|C, c_variadic, fixed_count, can_unwind}`、`ArgAbi{layout: LayoutId, mode}`、`PassMode::{Ignore, Direct(ext), Pair, Cast, Indirect{on_stack, meta}}`。首轮只接受 `Ignore` 和 `Direct(Int/Pointer ≤ 64 位)`，其余返回 `UnsupportedFeature`。
- 增加 rustc 没有的第二层：`assign_locations(&FnAbi) -> CallLayout`，把每个部件（Direct 一个、Pair 两个、Cast 每个 `Reg` 一个、Indirect 一个指针）分到 GPR/XMM/栈偏移，并给出 sret 寄存器、栈参数区大小与对齐。机器层只消费 `CallLayout`；现有 `StoreArg/LoadArg` 就是“部件落在栈偏移”的特例。之后加 Pair/Cast/Indirect/XMM 只扩充两层的枚举，不改调用者。
- 扩展语义写进 `Direct(ext)`：现在所有值是 i64，窄整数进入 T17 后由 ABI 边界显式截断/扩展（C ABI 参数扩到 32 位；返回值和 Rust ABI 参数不保证高位，callee 自行规范化）。
- `#[track_caller]` 的尾随 `&Location` 参数从第一版就在 FnAbi 构造里处理：所有 `Assert` 失败调用的 panic lang item 都带它（[fn-abi.md](fn-abi.md#tricky-边界)）。

### T15 类型与布局（[m2](../../../plan/m2-types-memory-abi.md#t15)）

- 移植 `rustc_abi/src/layout.rs` + `layout/simple.rs` 的算法到自有 `LayoutData`（驻留为 `LayoutId`）；重写 `layout_of_uncached` 的类型分派。顺序：标量/指针/宽指针 → struct/tuple（含重排与 Scalar/ScalarPair 提升）→ 数组 → union → tagged enum → niche enum → `repr(packed/align)`。
- 同时移植 `layout_sanity_check` 的不变量作为 verifier（[invariant.rs](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_ty_utils/src/layout/invariant.rs#L8)）。
- 布局查询的依赖：规范化、`is_sized`、struct tail 与 `Pointee::Metadata`、ADT 定义/repr/判别值、`layout_scalar_valid_range` 属性、`UnsafeCell`/`UnsafePinned`/pattern type、数组长度常量。判别值求值必须不依赖该 enum 自身布局。

### T17 SoN 类型与内存（[m2](../../../plan/m2-types-memory-abi.md#t17)）

- SoN 值类型 = `Primitive`（宽度、符号、浮点、指针）；`ScalarPair` 拆成两个值，`Memory` 类型只以地址出现。enum 的 `Discriminant`/`SetDiscriminant` 在构图时展开成整数运算（[enum-niche.md](enum-niche.md#判别值读写)）。`valid_range` 首版只用于 verifier/断言，不做依赖它的优化。

### T18 完整 ABI（[m2](../../../plan/m2-types-memory-abi.md#t18)）

- `extern "C"`：移植 `x86_64.rs` 分类（265 行，几乎无耦合）；分配层自写，覆盖 [sysv-x86_64.md](sysv-x86_64.md) 中“LLVM 承担、kfrc 必须自己做的部分”6 条。
- Rust ABI：移植 `adjust_for_rust_abi` 的三条规则（返回 >16 字节 sret；`Memory` ≤8 字节转整数；`Memory` >8 字节传指针），分配层与 C 共用。i128、Pair 的寄存器行为按 LLVM 推断，必须与 rustc 编译的对象对拍。
- `"rust-cold"`（LLVM `PreserveMost`）、SIMD 按值、`unadjusted` 先明确拒绝。

### T19 数据对象（[m2](../../../plan/m2-types-memory-abi.md#t19)）

- 每个常量 `Allocation` → 一个数据片段：字节 + 按 provenance 表生成的重定位（指向其他分配、函数实例、vtable、static）；不可变的进 `.rodata`，按内容哈希去重命名（rustc 的 `alloc_<hash>`）；零长度分配不发射，用 `align + offset` 作整数地址。未初始化字节写 0。
- panic 的 `Location` 常量含 `&str` 文件名指针，所以第一个能 panic 的程序就需要“分配 + 重定位”。

### T24 常量求值（[m3](../../../plan/m3-semantics.md#t24)）

1. 值与内存模型从第一步就照 rustc：`Scalar = Int(bits,size) | Ptr(AllocId, offset)`、`Allocation{bytes, provenance, init_mask, align, mutability}`、`ConstValue = Scalar | ZeroSized | Slice | Indirect`。“纯整数”阶段只是限制可执行的操作，不换数据模型。
2. 解释 ConcreteBody（对应 `mir_for_ctfe`，优化前），不解释 SoN；布局用同一个 `LayoutId`；局部变量默认立即数。
3. 限制照 CTFE machine：只调 const fn、ptr→int 报错、int→ptr 无 provenance、指针比较不支持、溢出恒检查、terminator 计数 lint、递归深度。
4. 驻留 + 有效性检查（[const-eval.md](const-eval.md#驻留与有效性)），再做 static 初始化（含递归 static 检测）、promoted、值树（const 泛型）、按 sysroot 需要逐个加 intrinsic（`intrinsics.rs` 1218 行）。
5. 跨 crate：rustc 为 const fn、const/关联 const 编码 `mir_for_ctfe`，为需要单态化或可跨 crate 内联的 fn 编码优化后 MIR（`should_encode_mir`，[encoder.rs#L1099](https://github.com/rust-lang/rust/blob/e408947bfd200af42db322daf0fadfe7e26d3bd1/compiler/rustc_metadata/src/rmeta/encoder.rs#L1099)）。kfrc 的 [M4](../../../plan/m4-cargo-sysroot.md) metadata 同样要携带这两类 Body，否则跨 crate 的常量求值和单态化都做不了。

### T25 单态化（[m3](../../../plan/m3-semantics.md#t25)）

- 最小闭包：根（`main` + `lang_start::<R>`、导出的非泛型 fn、static）；边（直接调用、fn 指针、闭包转 fn 指针、unsize 到 dyn 的 vtable 方法与 drop glue、`Drop`、常量/静态分配引用、Assert 的 panic lang item）；`DropGlue` 体生成；vtable 分配。
- `InstanceKey` 必须含 `InstanceKind`（同一 DefId 可有 `Item`/`ReifyShim`/`ClosureOnceShim` 等多个实例），const 实参用值树；`DropGlue(None)` 只在取地址时生成。
- 首轮遍历全部基本块（不做 `mono_reachable` 剪枝），得到 rustc 的超集（推断：报错集合不变）；最终集合排序，保证与并行度无关。

### T27 runtime 能力（[m3](../../../plan/m3-semantics.md#t27)）

- panic 路径 = 带位置参数的 lang item 调用；`can_unwind` 来自 `fn_can_unwind`；`Virtual`/`ReifyShim`/`track_caller` 组合按 [instances.md](instances.md) 逐项开放。

### 与 TyCtxt 的耦合点

| 机制 | 需要的查询 | kfrc 替代 |
|---|---|---|
| 布局 | `normalize_erasing_regions`、`is_sized`、`struct_tail`、`adt_def`/`repr`/`discriminants`、`layout_scalar_valid_range`、lang item、常量求值 | 语义快照上的类型查询 + 布局缓存 |
| FnAbi | `fn_sig`、闭包签名与 kind、`codegen_fn_attrs`（track_caller、nounwind）、`fn_can_unwind`、`deduced_param_attrs`（仅优化，可跳过） | 定义属性表 + 目标配置 |
| 解析/单态化 | `codegen_select_candidate`、specialization graph、`vtable_entries`、`needs_drop`、`instance_mir`、`mentioned_items`/`required_consts`、`reachable_non_generic` | trait 求解（T21）+ ConcreteBody + 导出表 |
| CTFE | `mir_for_ctfe`、`is_const_fn`、`Instance::resolve`、`layout_of`、全局分配表、`eval_static_initializer` | ConcreteBody、自有 `AllocId` 表（快照内唯一） |

### 工作量与风险（推断）

| 项 | 估计 | 主要风险 |
|---|---|---|
| 布局移植（含 enum/niche、verifier、对齐测试） | 2–3 人周 | 字段重排/niche 的细节差一处就导致 `size_of` 与混链不一致 |
| FnAbi + SysV 分类 | 1–1.5 人周 | 低；源码小且独立 |
| 分配层与机器接口（sret、byval、XMM、AL、栈对齐、扩展） | 2–3 人周 | LLVM 隐含行为（i128、Pair、扩展）只能靠对拍发现 |
| 单态化收集 | 1.5–2 人周 | 依赖 T21 解析与 drop glue 体生成；递归限制与确定性 |
| CTFE（模型 + 解释器 + 驻留/有效性 + static/promoted + 值树） | 5–7 人周，intrinsic 另计 | core 的 const fn 用到大量 intrinsic；跨 crate Body 依赖 M4 |
