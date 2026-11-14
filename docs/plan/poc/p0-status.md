# P0 进度

状态日期 2026-11-09。阶段定义见 [phases.md](phases.md)，代码在 `crates/`。
模块边界和实现限制见 [implementation.md](implementation.md)。

## 已有

| 部分 | 位置 | 状态 |
|---|---|---|
| Body 契约 | `crates/kfrc-body` | MIR 形状的 Typed Body（语句、投影、常量入池）、`TypeOracle` 与记录的 rustc 布局、Body verifier |
| 当前 SoN（单态、提前布局） | `crates/kfrc-son` | 16 字节节点记录 + 列存 + 转发表；标量 SSA；聚合主要按地址表示，enum/niche 在 builder 展开；独立整数优化 pass；结构/类型/支配 verifier；以标量及内存为主的解释器 |
| 新语义切片（显式选择） | `build_specialized_graph`、`lower_layout` | 真实聚合/enum/宽指针值、字段级解释与阶段检查；product/宽指针 S→L、聚合折叠/GVN/常量控制裁剪已落盘；范围与真实验收见 [semantic-ir-status.md](semantic-ir-status.md) |
| x86 | `crates/kfrc-x86` | 标量寄存器分配与保守内存路径；Direct/Pair/Indirect、函数指针、栈槽、数据重定位；`object` crate 写 ELF |
| 外壳 | `crates/rustc_codegen_kfrc` | rustc 1.94.1 的 `CodegenBackend`；按职责拆分的 MIR → Body 转换器；panic Location、drop glue、unit-main 与 allocator shim；不支持的构造直接报错，不回退 LLVM |

旧图默认标签现已改为 `LegacyLayout`；新链有 Specialized/Layout 的真实检查，
`GraphLevel::Rust` 在定义级实现完成前被明确拒绝。定义级 R、孔位答案与 S 特化
尚待按 [迁移方案](semantic-ir-migration.md)交付。`Unknown`/`Local`/`Const`
分区、完整范围内优化及 R 图/机器码缓存也未因现有测试通过而完成。

## 支持的输入

整数（≤64 位）、`bool`、`char`、单元类型；算术、位运算、移位、比较、整数转换；控制流、直接与间接调用、递归、>6 个参数。已通过实测的新增范围包括共享/可变引用、match guard、tuple/array、Pair 参数/返回、Direct 与 niche enum、slice、只读字符串、checked arithmetic，以及除零、越界和溢出的 panic 路径。

目标为 x86_64-unknown-linux-gnu，要求 `-Cpanic=abort`。新增内存测试开启 overflow checks；普通可执行文件入口目前只接受 `fn main() -> ()`。这不代表 P1–P4 的所有语义和性能要求已经完成。

## 验证

2026-11-09 使用 HK 控制的独立 E2B 沙箱、rustc 1.94.1（含 rustc-dev）。
实际链路为 **rustc 前端 + kfrc 中后端 + rustc std**。输入快照、哈希、命令和原始日志在
`evidence/poc-20261109/`，一个命令入口见 `scripts/poc-validation/README.md`。

| 测试 | 结果 |
|---|---|
| 标量回归 | 565 行固定输出与 LLVM 一致；5 个生成程序 × 40 函数一致 |
| 优化前后 | 开启/关闭独立优化 pass 的标量对照均通过 |
| 内存与聚合 | 候选 19 的完整 P1 1155 行与 LLVM 一致；含指针类型保持、真实线程 TLS/Drop/HashMap、SIMD/XMM 跨后端调用、select 和既有范围；3 类 panic 状态/消息/精确 Location 一致 |
| panic | 除零、越界、溢出与 LLVM 的 abort 状态一致，消息与 file:line:column 一致 |
| 可执行入口 | 候选 15 patch1：普通 Rust main 退出 0、no_main C main 退出 37，Vec 退出 0、OOM 134，输出和状态与 LLVM 一致；对象来源为 kfrc |
| 分配与释放 | 候选 07：Vec 从容量 1 增长到 65 个元素，内容、和 35555、drop/dealloc 与 LLVM 一致；固定 Layout 的 handle_alloc_error 路径 abort 134、stderr 一致 |
| Cargo 按包后端 | 待测库选择 kfrc、driver 选择 LLVM；输出与对象来源核验通过 |
| 候选 19 workspace | 89 个 suite，781 passed，5 ignored，0 failed；含跨对象数据绑定、pthread TLS、真实 C 向量 ABI、编码对象/GNU 汇编双路径、内存、union、可写数据执行 |
| TLS ELF 证据 | 候选 17：4 个自有符号与其 kfrc 对象的 STT_TLS、SHF_TLS、TPOFF32 及最终 executable 的同名 TLS 符号/PT_TLS 全部通过 |
| Cast ABI | 候选 06：与 C 的双向参数/返回对照通过，覆盖部分尾字节、padding、Sign/Zero 扩展及寄存器耗尽 |
| Debug/闭包实链路 | 候选 11：manual/派生 Debug、虚调用、捕获闭包的 12 行与 LLVM 一致 |
| funnel/rotate 与饱和算术 | 候选 14 独立整数 E2E：500 组位宽/移位边界差分、131872 对饱和输入通过 LLVM 差分及 LLVM driver 的 i128 数学 clamp reference；u8/i8 输入对穷举 |
| rustc 原版测试第六轮 | 候选 19 官方 compiletest 双后端执行：144/144 源文件、145 revisions passed，0 failed/deferred；完整候选池另有 90 blocked、193 excluded，来源见 [upstream-tests.md](upstream-tests.md) |
| semver 1.0.28 | 候选 19：原版 34 项集成测试在两组全部通过，工作负载输出一致；25 份原源码前后哈希一致，Version::Debug 定义与链接实现来自 kfrc 对象 |
| regex-syntax 0.8.8 | 候选 19 原版库公共 API 差分通过；16 CGU 的 8584 个私有 allocation 全部 LOCAL，无重复 GLOBAL。原版 147 内部测试 LLVM 基线通过，kfrc 编译在 uninhabited enum 构造阻塞，未计通过 |

候选 05 修复了与 rustc mono collector 的可达性差异，避免为编译期已裁掉的 UB-check 分支写出未定义符号重定位。候选 09 已通过零大小闭包和 trait-object 的虚调用、动态 size/align、有析构及空析构槽的动态 drop 差分。候选 10–11 的 wrapper unsize 和 Debug/闭包差分通过；候选 15 patch2 首次通过 semver 原版集成测试，候选 19 再次 34/34 通过。测试 harness、std 和跨 crate 的部分泛型实例依然来自 LLVM，不能宣称整个程序都由 kfrc 编译。regex-syntax 原版内部测试仍有上述编译边界。

此前 2026-11-08 在 HK 直接运行的历史结果：

| 测试 | 命令 | 结果 |
|---|---|---|
| 固定输入表：T01 各形状、控制流、调用、各位宽 | `tests/p0-scalar/run.sh` | 565 行与 LLVM 构建逐字节一致；目标文件由 kfrc 写出 |
| 生成式差分：随机标量程序 | `tests/p0-scalar/generated/run.sh` | 种子 1–150（150 个程序）× 每程序 40 函数 × 6 组输入一致 |
| 单元测试 | `cargo test -p kfrc-son`、根 crate `cargo test` | 通过 |

差分发现并修复的错误（都在复用的旧后端里，不在新代码的语义层）：调用前先装参数寄存器再保存调用者保存寄存器；可变移位用 `rcx` 却没有固定区间；驱逐溢出的区间复用了在其起点之前仍被占用的溢出槽；调用区栈槽与溢出槽重叠。

## 离 P0 出口还差

- W01–W10 的纯标量部分（本轮用自写的标量负载代替）。
- 规则 DSL 生成器与构建期窥孔/GVN；目前是独立整数优化 pass，不能计为完整 DSL 交付。
- W01–W10 标量链所需的 Body/type oracle、builder 和小解释器契约仍须按设计验收，不能把现有默认 R 枚举当作分层实现。

## 后续阶段未完成项

- P1：值形式聚合/enum/drop、`Local`/`Const` 分区及规定优化；当前提前布局链的正确性结果仅作迁移对照。
- P2：定义级 R、孔位/答案、身份键与递归去重、嵌入实例、S 变化种子和内联、调用摘要、mentioned items；同一份 `-Zbuild-std -Zmir-opt-level=0` 的 std。
- P3：显式 L 降低、placement、ABI 与线性扫描全部出口；新链 semver 测试及主张 2–4 的 E2B 实测。旧链 34/34 不代表 P3 完成。
- P4：regex-syntax 完整测试、内容寻址缓存与源位置隔离、六项指标逐项结论。

TLS、整数 SIMD/XMM、union 等历史结果属于范围外实现记录，不扩大
[scope.md](scope.md)，也不替代以上必交付项。
