# 程序体积：怎么保证

比较口径见 [性能比较协议](../../plan/README.md#验收)：等同 strip 后的可加载节大小，比值 ≤ 1.0。

## 来源和对策

| 体积来源 | 对策 | 依据 |
|---|---|---|
| 不可达代码 | 只发射从根（`main`、导出符号、static 初始化、vtable）可达的实例 | rustc 构建 22% 代码被链接器丢弃，库 31% |
| 同构实例 | 答案键共享一份代码，其余是别名符号 | 完全重复函数 13.9%；`-Zmono-dedup` 去重 5–7% |
| 答案不同但机器码相同 | 答案键之后再做一层片段折叠（ICF 式）：L 级片段按“字节 + 重定位目标所在类”细化，同类出别名；`static` 不折叠；环的细化每轮只读上一轮结果，可并行 | gold ICF text −6%；lld `--icf=all` −2%；rustc 默认 mergefunc |
| drop glue 与 shim 重复 | drop glue、vtable/reify/FnOnce shim 生成后按图内容进答案键表；小的直接展开 | drop glue 10,311 个函数；W07 三个 `drop_in_place<Box<dyn _>>` 屏蔽符号后相同 |
| 只差值域事实的实例 | `ValidRange` 只在值流入比较/算术时登记 | W06：`largest<char>` 与 `largest<u32>` 的 x86 逐字节相同 |
| 单用的闭包/适配器实例 | 嵌入调用者，不单独生成 | W02：9 个函数进入 LLVM，最后 1 个 |
| 内联后残留的外部副本 | 被调者所有调用点都内联、未取地址、不导出 → 不发射 | LocalCopy |
| panic 路径 | 冷区域留在函数内、放末尾或 `.text.unlikely`，不抽成独立函数（hot/cold splitting 体积 +9.6%，MFS +1.7%）；同函数相同 panic 合并尾块；`Location` 进数据表并按内容去重 | |
| 过度内联 | 内联按被调者 `size_estimate` 决策，冷区域不内联 | |
| 栈帧 | value slot 合并与活跃区间复用 | |
| vtable、常量 | 按内容哈希去重（[T25](../../plan/m3-semantics.md#t25)） | |

## 可达性在哪里决定

- **二进制/动态库 crate：** 链接前在全部 crate 的实例图上从根做可达性。库 crate 的非泛型函数不再默认生成并放进 rlib，而是提供 R 图；需要时在 build 级存储里生成（[compile-time.md](compile-time.md#关键路径) 的预生成）。
- **rlib 作为最终产物**（用户直接分发 rlib 的场景不在 M5 范围）：按 rustc 的规则生成全部导出实例。

## 和 LLVM 相比可能更大的地方

| 风险 | 原因 | 对策 |
|---|---|---|
| 没有 LLVM 的指令选择精度 | 自写的 x86 编码 | 地址模式、`lea`、条件传送、短跳转编码；按体积 profile 补 |
| 线性扫描的 spill 代码 | 无全局分配 | 活跃区间拆分；冷区域的 spill 放冷区域 |
| 循环展开/向量化缺失 | 反而更小 | 体积不受影响，运行时间见 [runtime.md](runtime.md) |

## 不做

- Machine outliner：`-Oz` 平均只 −1.29%；全局 outliner 构建时间 1.41×，且依赖上一次构建，与片段缓存和确定性冲突。

## 指标

公平基线：rustc 体积对照包含 opt-level s/z 和 lld `--icf=all` 两种配置。

- `.text`、`.rodata`、`.eh_frame`、`.data` 各自大小，与 rustc 同配置对比。
- 每个答案键的机器码字节；别名符号数。
- 被可达性剪掉的实例数（应对应 rustc 被链接器丢弃的那部分）。
- 冷区域字节占比。
