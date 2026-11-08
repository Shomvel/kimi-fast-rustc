# 架构：外壳与三条接缝

## 外壳

```text
cargo（profile 按包选 codegen-backend）
  └─ rustc 1.94.1 前端 ── 多态 MIR、tcx 查询
       └─ rustc_codegen_kfrc（外壳，依赖 rustc_private）
            ├─ MIR → kfrc Body 转换器          ← 接缝 2
            ├─ TypeOracle 的 tcx 实现           ← 接缝 1
            └─ kfrc-son、kfrc-x86（不依赖任何 rustc_* crate）
                 └─ .o ──→ 系统链接器 ←── rustc 编译的 std/alloc（接缝 3）
```

- 外壳实现 `CodegenBackend`：在 `codegen_crate` 里自己从根做可达性（不用 rustc 的 CGU 划分），对每个可达定义取多态 MIR，建 R 图，回答孔位，按特化身份键去重，特化、降低、编码，输出目标文件。
- 工具链固定为 rustc 1.94.1 + `rustc-dev` 组件，用 `RUSTC_BOOTSTRAP=1` 打开 `-Z` 选项；P0 先核实 cargo 的 `codegen-backend` profile 选项（不稳定）能按包选择后端。
- 只有外壳依赖 `rustc_*`。`kfrc-son` 和 `kfrc-x86` 是产品代码，CI 检查它们的依赖树里没有 `rustc_*`。

## 接缝 1：TypeOracle

SoN 不持有 `Ty<'tcx>`，只用自有的 `TyId`，对类型的所有提问都经过：

```rust
pub trait TypeOracle {
    fn kind(&self, t: TyId) -> TyShape;          // kfrc 自己的结构枚举：Adt/Ref/Tuple/Param…
    fn answer(&self, hole: &Hole) -> Answer;     // SizeAlign、ScalarShape/ValidRange、FieldOffsets、VariantEncoding、
                                                 // PassMode、NeedsDrop、IsFreeze/IsUnpin、Resolve、VTable…
    fn stable_hash(&self, t: TyId) -> Fingerprint;
}
```

- **PoC 实现**：`layout_of`、`fn_abi_of_instance`、`Instance::resolve`、`needs_drop`、`is_freeze`；`Ty<'tcx>` 驻留成 `TyId`。答案自然与 rustc 位级一致，所以能和 rustc 编译的 std 混链。
- **产品实现**：自有前端 + `KfrcInterner` + next-solver + `rustc_abi`。
- 为什么不直接在 SoN 里用 `rustc_type_ir`：外壳必须链接 rustc 内部那一份（rustc_private），产品依赖 crates.io 上的 `ra-ap-rustc_type_ir`，两者源码相同但对编译器来说是两个 crate，trait 和类型互不兼容；`Ty<'tcx>` 的生命周期也会传染整个 IR。
- 这同时强制执行设计里“S/L 级只经答案表读类型事实”（F01）：SoN 没有别的途径拿到类型信息。

## 接缝 2：MIR → kfrc Body

R 级 builder 只认 [Body 契约](../../design/frontend/body.md)。PoC 写一个转换器（估计 1–2 千行，一次性代码）：

- 1:1 映射：基本块、`Place` 与投影、`Rvalue`、`Call`/`Assert`/`Drop`、cleanup 边、被取地址局部的 `StorageLive/Dead`。
- 需要改写：`CopyForDeref`、`ShallowInitBox`、`NullaryOp`、以调用形式出现的 intrinsic、`Repeat`。
- 转不过去的构造就是 Body 契约的缺口，记入 [relation.md](relation.md) 的缺口清单，不在转换器里绕过。

## 接缝 3：std 的未优化 MIR

rlib 的 metadata 只存泛型函数优化后的 MIR（已经跑过 rustc 的 MIR 优化和 MIR 内联）。直接用预编译 std 时，kfrc 拿到的 `Vec::push`、迭代器适配器是半优化状态，对比不干净。做法：

- 用 `-Zbuild-std` 重编 std，加 `-Zmir-opt-level=0`，metadata 里就是未优化的 MIR；std 的非泛型代码仍由 LLVM（O3）编译。
- 基线 rustc+LLVM 用**同一份** std 构建；两边的用户 crate 都从 `-Zmir-opt-level=0` 的 MIR 出发，另报一组基线用默认 MIR 优化的数字作参考。

## 跨 crate 的 R 图复用

PoC 里每个 crate 由独立的 rustc 进程编译，没有 kfrc 自己的 metadata。用 build 级缓存模拟 [C1/C2](../../design/goals/compile-time.md)：R 图按 `(DefPathHash, Body 哈希, 配置)` 存，下游 crate 的特化命中即复用；机器码按机器码缓存键存。缓存目录由环境变量指定，测量时分别报告冷、热两种状态。
