# M2：类型、内存、布局和 ABI

**目标：** 支持整数各宽度、浮点、指针/引用、数组/slice、struct/enum，以及它们的布局、内存读写和调用约定。开放的安全子集同时具备初始化、move 和借用检查——新接受的引用程序必须同时被检查。

**依赖：** M1。接口稳定后可提前做测试向量和 ABI 调研。

**出口：**

- 类型布局与 ABI 对拍、跨函数/独立编译对象的值与内存行为、双向 C FFI 通过。
- 对齐、截断/扩展、别名和边界行为正确；效果链 verifier 拒绝非法变换。
- 最小跨编译单元 fixture 通过（不需要 M4 的 metadata）。

**通过后仍不代表：** 借用和泛型正确性已自动获得。

**约束：** 保守内存模型正确之前，不做激进别名假设、load 消除或跨调用移动。浮点和目标指令能力单独记录。

---

<a id="t15"></a>
## T15 类型、Place 与目标布局

`M2 · B · 依赖：T14`

- **做什么：** 整数宽度、浮点、tuple/struct/enum、array/slice、指针/引用、`repr`/layout，每类一个子项。产出自有类型/布局、Place 投影和 ABI 分类所需输入；区分语言保证的布局与内部约定。
- **补充：** 布局算法按 `rustc_abi::layout` 移植，`repr(Rust)` 也与 rustc 位级一致（x86_64 Rust ABI 由布局决定）；含生命周期参数的 ADT 需要方差推导（`variances_of`）与推断 outlives（`inferred_outlives_of`）。
- **完成标志：** 大小、对齐、投影、有效值的正反例；布局与 ABI 还要通过[一致性测试](../validation/layout-abi-parity.md)。

<a id="t16"></a>
## T16 安全子集的初始化、move 与借用检查

`M2 · B · 依赖：T13, T15`

- **做什么：** 对 M2 接受的引用/聚合语义做受限借用、move 和初始化分析，产出 CheckedBody。复杂生命周期暂不支持时明确拒绝。drop-live 先用保守近似（字段递归 + 有 `Drop` impl 的类型参数全部 live），真 dropck 等 T21。
- **完成标志：** 可变别名借用、move 后使用、分支未初始化等拒绝用例与 oracle 一致。

<a id="t17"></a>
## T17 SoN 类型与内存效果

`M2 · C · 依赖：T15, T16`

- **做什么：** 整数截断/扩展、指针和聚合值；通用 Load/Store、地址计算、静态对象、间接调用、数据初始化；效果节点与合并（规划 `src/son/effects.rs`、`lower_place.rs`）。首版用一条保守效果链，详见 [SoN 效果模型](../architecture/representations.md#son)。越界/溢出检查按配置保留；地址逃逸和未知调用保守处理。
- **完成标志：** 用固定的带类型 Body/图 fixture 验证：别名写后读、调用改写内存、循环中的效果、检查失败路径；访问不会跨检查、volatile 或未知调用移动。真实机器差分在 T20 完成。

<a id="t18"></a>
## T18 完整 ABI 与寄存器类别

`M2 · D · 依赖：T15, T17`

- **做什么：** System V 参数/返回分类、间接聚合传递、栈对齐、caller/callee-save；寄存器类别（含浮点/SIMD）、固定寄存器、flags liveness、Phi 并行复制、spill。职责拆为 `src/machine/{ir,abi,lower,regalloc}/`，仍用线性扫描。
- **完成标志：** 双向 C/Rust FFI、不同宽度/聚合/浮点传参与返回、栈参数、递归、跨调用活跃值、高寄存器压力、循环中的并行复制，真实执行一致。未支持的约定明确失败。

<a id="t19"></a>
## T19 数据对象、符号与跨编译单元链接

`M2 · D · 依赖：T17, T18`

- **做什么：** `src/object/`：`.text`/`.data`/`.rodata`、可见性、所需重定位、稳定符号；ObjectFragment 确定性合并。此时用显式声明 + C ABI 的独立对象 fixture，不涉及 Rust metadata。
- **完成标志：** 系统链接器接受并真实执行；线程完成顺序不改变对象字节；重复/缺失符号、未知 relocation 正确报错。

<a id="t20"></a>
## T20 M2 出口

`M2 · I · 依赖：T16, T17, T18, T19`

- **做什么：** 类型 × 内存 × ABI × 跨函数 × 跨编译单元的组合差分；支持/拒绝/缺口矩阵；拆分 M3 子项。
- **完成标志：** 满足本阶段出口；spill 用的 Load/Store 未被当作通用内存支持。
