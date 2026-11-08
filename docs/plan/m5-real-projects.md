# M5：真实项目正确性

**目标：** ripgrep 和 Cargo 整仓原生构建，功能与 rustc 构建版本等价。

**依赖：** M4。沿固定 Cargo.lock 和 features 的依赖闭包推进：小库 → ripgrep → Cargo。

**做法：** 根据真实阻塞项补齐 SIMD/target-feature、atomics、TLS、panic/unwind、过程宏、intrinsic/asm；处理 C/C++ 库的构建与链接。不修改业务源码、不减少 features。

**出口：**

- 正式功能测试，以及 CLI 的 stdout/stderr、退出码、输出文件和必要副作用对拍通过。
- 每个目标对象来源可追溯；语义和配置没有被削减。

**通过后仍不代表：** 运行性能、体积和编译速度达标。

---

<a id="t35"></a>
## T35 ripgrep 依赖闭包与功能验收

`M5 · I · 依赖：T34`

- **做什么：** 冻结 lockfile/features 下逐个依赖 crate 推进，按实际阻塞项补能力；维护子任务级的依赖与特性销项清单。
- **完成标志：** 原生 `rg` 通过功能测试；不同搜索/匹配/输入规模下 stdout、stderr、退出码和文件行为一致。

<a id="t36"></a>
## T36 Cargo 及其 native 依赖

`M5 · I · 依赖：T35`

- **做什么：** 冻结 Cargo 及其 C/C++ 库、host 工具、系统接口；完整来源账本，C/C++ 工具链成本单列。
- **完成标志：** 构建、解析、依赖操作和正式功能测试通过；`cargo metadata --no-deps` 只能证明该子命令。

<a id="t37"></a>
## T37 正确性冻结与质量差距分类

`M5 · I · 依赖：T35, T36`

- **做什么：** 功能矩阵、无 fallback 证明、可重放的产品输入和性能 workload；按机器码 / 优化 / 前端 / 调度 / 链接分类性能差距。
- **完成标志：** 满足本阶段出口；所有对象来源明确，语义和配置等价。
