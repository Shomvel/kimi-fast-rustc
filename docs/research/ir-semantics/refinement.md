# 级间关系：Body → R → S → L

## 总表

| 边 | 关系 | 性质 | 可执行检查 |
|---|---|---|---|
| Body → R（`build_son`） | ⟦R⟧_A ⊑ ⟦Body⟧_A | refinement：Body 的立即 UB 可以变成 R 的 poison | Body 解释器与 R 解释器在同一答案表 `A` 下对拍 |
| R → R′（R 级 pass） | ⟦R′⟧_A ⊑ ⟦R⟧_A，对**所有**答案表 `A` | refinement，量词覆盖所有实例 | 用若干具体实例的答案表抽样对拍 |
| R → S（`specialize`） | ⟦specialize(R, A)⟧ = ⟦R⟧_A | **相等**：“代入答案”，没有 refinement 余量 | 同一输入下可观察行为与最终内存逐字节相同 |
| S → S′（S 级 pass） | ⊑ | refinement | 差分 |
| S → L（`lower_layout`） | 内存相同；L 的 SSA 值 = `encode(S 值, 布局)` 按 PassMode 拆分 | refinement，值关系是 `encode` | 两边锁步运行到每个外部调用和 `Return`，比较内存与编码后的值 |
| L → Machine IR | 现有 `machine_eval` 的寄存器/栈语义 | refinement，需要栈帧的块映射 | 现有对拍 |

⊑ 读作“行为是其子集，源 UB 处任意”（Alive2/CompCert 的定义，见 [prior-art-llvm.md](prior-art-llvm.md)、[prior-art-verified.md](prior-art-verified.md)）。

## R 的语义以答案表为参数

R 图没有布局常量，但 R 的 `Store{ty: T}` 要把值编码成字节，`TraitCall` 要知道调哪个实例，`DropValue` 要知道 `NeedsDrop`。所以：

```text
⟦R⟧ : Answers → (Inputs → Behaviors)
```

| 推论 | 说明 |
|---|---|
| [levels.md](../../design/ir/levels.md#r-级rust-语义多态) 的“R 级不能出现布局常量”是**语法**限制 | 语义上 R 依赖布局，只是经孔位间接依赖 |
| R→S 是代入定理 | `specialize` 只做“孔位 ↦ 答案”和代入后的折叠；折叠若用了 R 级规则之外的推理，相等就退化成 refinement，要单独测 |
| 答案键的健全性可执行检查 | R 解释器在答案表上求值时记录**读过的孔位**；两个实例共享答案键 ⇒ 读过的孔位答案全部相同。读到未登记的类型事实即 ICE，与 [generics-specialization.md](../../design/ir/generics-specialization.md#孔位) 的 debug 断言是同一件事的动态版（F01、F14） |
| 发布的 R 图与会话无关 | `PanicStrategy` 是孔位（F24）：⟦R⟧ 不能读会话状态 |
| R 级 pass 的义务对所有答案成立 | 一个 R 级变换若只在 `size_of::<T>() > 0` 时正确，就必须先把该条件变成控制孔位；解释器抽样时应包含 ZST、`!Freeze`、`NeedsDrop=true` 等边界实例 |

## S → L：“布局降低”就是 `encode`

MiniRust 的 `encode`/`decode`（[prior-art-rust.md](prior-art-rust.md)）正好是 S→L 的值关系：

| S 值 | L 表示 | 降低时要守住的性质 |
|---|---|---|
| `Tuple` | 按 `backend_repr`/PassMode：标量、标量对、≤8 字节整数、`ValueSlot` | padding 编码成 `Uninit`；L 级不能读 padding 当数据 |
| `Variant{v, …}` | tag/niche 位串 | `VariantSwitch` 按 tag 值分支等价于 `decode` 后比较变体（F19） |
| `WidePtr` | 两个标量 | 宽指针元数据不丢 provenance |
| `Bytes` | 字节原样 | union 与 `MaybeUninit` 的 `Uninit` 保留 |

[ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md#大值与原地构造) 的“编码重写是恒等”（F39）在这里有精确定义：`encode(Err(Field(e))) == encode(Err(e))` 逐字节成立（包括 `Uninit` 位置相同）。

## Body 的参考语义与 MiniRust 对齐

| 方案 | 做法 | 评估 |
|---|---|---|
| A. 直接运行 MiniRust | kfrc Body → MiniRust 程序 → `specr-transpile` 生成的解释器 | 要写 Body→MiniRust 翻译；MiniRust 无浮点、unwind 不全；规范在变（推断：维护成本高） |
| B. 照 MiniRust 结构自写 Body 解释器 | `Value`、`AbstractByte`、`encode/decode`、`Memory` 接口照 MiniRust；Rust 实现 | **推荐**。与 CTFE 共享核心；MiniRust 作为文字规范和 UB 用例来源（`tests/ub` 47 个） |
| C. 不要 Body 语义，只用 Miri 作源程序 oracle | 源码 → Miri；kfrc 产物 → 真实执行 | 已在计划中，但定位不到 Body 构建和 `build_son` 的错误 |

推荐 B + C：Miri 判定源程序（de-facto UB、期望输出），kfrc 的 Body 解释器对 Miri 对拍，R/S/L 解释器对 Body 解释器对拍。R 级与 MiniRust 的对齐**经由 S 级**：MiniRust 是单态的，⟦R⟧_A = ⟦S⟧ 把 R 的语义归约到可以对齐 MiniRust 的 S。

## CTFE 能不能解释 R 图

| 问题 | 答案 |
|---|---|
| 能否用优化后的 R 图做常量求值 | **不能**。优化是 refinement，会删掉 UB（溢出、越界）；常量求值必须报出这些错误，否则 kfrc 会接受 rustc 拒绝的程序。rustc 也因此解释 `mir_for_ctfe`（优化前的 MIR，见 [const-eval.md](../rustc/layout-abi-mono/const-eval.md)） |
| 能否解释**未优化**的 R 图 | 可以但收益小：R 图丢了 Body 的 place 结构，CTFE 的驻留、有效性检查、promoted 都按 place 工作 |
| 建议 | 维持现设计（CTFE 解释 Body），**共享核心**：CTFE 与 IR 解释器共用 `Value`/`Memory`/`encode`/`decode`，对应 rustc 中 CTFE 与 Miri 共用 `InterpCx` + `Machine` |
