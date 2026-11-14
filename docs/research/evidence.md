# 依据：LLVM 在重建什么

数据来自 2026-10 的 rustc 构建剖析（rustc 1.94.1 / LLVM 21.1.8，cargo 0.95.0 库与 ripgrep 15.2.0，release）。除特别说明外是 E2B Linux x86_64 上的 `instructions:u` 或未优化 IR 计数。原始报告不在本仓库，这里只摘结论和数字。

## 1. 结构重建占主体，事实通道基本喂满

cargo 库 `--emit=llvm-ir -Cno-prepopulate-passes`（已过 MIR 优化和 MIR 内联，未过 LLVM；`--emit=llvm-ir` 隐式使用单 CGU，真实 16 CGU 构建的函数数多约 23%）：82,130 个函数，431 万条指令。

| 成分 | 占指令 | LLVM 用来消除它的 pass |
|---|---:|---|
| `llvm.lifetime.*` | 12.8% | SROA、栈着色 |
| `alloca` | 7.1% | SROA / mem2reg |
| `llvm.memcpy` | 3.4% | MemCpyOpt、DSE、GVN（MemorySSA） |
| `llvm.assume` | 2.0% | 多数与已有元数据重复 |

- 函数规模：≤20 条指令的占全部函数 48%；drop glue 10,311 个函数，承接 21.3% 的调用点；闭包/Fn shim 占 29.2% 指令。
- 事实覆盖：指针参数 `noundef` 99.8%、`noalias` 89%、`dereferenceable` 80%；load 的 `!noundef` 92%。**再加属性的空间有限**，瓶颈在结构。
- 单个 `Target::doctestable`（3 个函数、172 条指令）在 `opt -O3` 下被 31 次 pass 改动，最后剩约 20 条指令：指针循环 + `switch tag`。
- `?` 的错误路径：每次 move 是独立 alloca 间的 40 字节 memcpy，一个字段形成 6 次拷贝的链；`visit_map` 一个函数 508 个 alloca、428 个 memcpy。

## 2. MIR 层没有把结构做干净

- 逐个关闭 MIR 的 GVN、DestProp、SROA、CopyProp、DSE 等，未优化 IR 规模变化都在 ±1.6% 内；只有 MIR 内联有明显作用。
- 原因四类：phase ordering（`?` 的“构造后投影”在 GVN 之后才出现）、表示能力（CopyProp/DestProp 只处理裸 local）、泛型边界（闭包和 `F` 的调用在单态化后才确定，而 MIR 只优化多态体）、codegen 重复发事实。
- 116 个 dump body 中 DestProp 只对 11 个起作用。

**对 kfrc 的含义：** “单态化前优化一次”是对的方向，但必须有单态化后的轻量特化步骤来处理闭包和 trait 调用（[generics-specialization.md](../design/ir/generics-specialization.md)）。

## 3. 重复工作

| 现象 | 数字 |
|---|---|
| 完全重复的函数（cargo IR） | 13.9% |
| 同一类型 × 反序列化器的实例数 | 3–8 份 |
| ThinLTO post-link 中导入体的重新简化 | ripgrep post-link CPU 的 40% |
| InstCombine 无变化调用 | 约 405k 次中 337k；每函数在 pipeline 中约跑 8 次 |
| `-Zmono-dedup`（布局键合并实例） | rg/cargo 编译指令 −2.35% / −2.74%；IR 去重 5.1% / 6.8% |
| `-Zshare-drop-glue=all` | −3.97% / −3.95%；放置位置比共享多少更重要 |
| 二者 + 跳过导入体 post-link + mono-inline | −12.35% / −12.20%（cargo wall −10.8%） |
| 全程序每个泛型定义只保留一份体的上限 | codegen 输入 −56.7%（≤2.3×） |

## 4. 死代码和冷代码

- 本构建编译出的 `__text` 有 22% 在链接时被丢弃，库 crate 31%（encoding_rs 81%、serde_json 94%）。
- 训练中 53% 的链接后代码从未执行；但对 PGO 冷函数 optnone 只省 14.1% 编译指令，held-out 负载慢 2.04×。**按 profile 分层不可取；按结构识别冷路径（panic、unwind、`#[cold]`）可取。**

## 5. 位置比数量重要

- MIR pass 的 IR 减少 14% 而 wall 不降：代价发生在 rmeta 之前（串行、关键路径上），收益是 rmeta 之后的并行 LLVM 工作。
- 去掉的 IR 不等于省下的时间：旧的 codegen 补丁去掉 4.4% IR 只省约 0.1% 编译时间。
- `-Clto=off`（建在 `-Zmono-dedup` 等补丁之上）省编译指令 rg 31.6%、cargo 28.7%，但运行时 rg 慢 1.5%–27.8%、cargo 慢 5.8%–8.7%：跨 crate 内联不能砍。
- 签名优先的流水线：模型上限是 ripgrep wall 比 bin pipelining 再降 28.5%（CPU 不设上限）；8 vCPU 上的原型实测只降 3.3%，cargo 项目无变化。接口应先于任何函数体优化发布，但收益取决于空闲核数。

## 6. 小结：哪些杠杆对应哪项设计

| 杠杆 | 上限/证据 | 设计 |
|---|---|---|
| 值形式的聚合，不经内存 | 23.3% 的未优化 IR 是 alloca/lifetime/memcpy | [values-and-types.md](../design/ir/values-and-types.md) |
| 变体操作 + 延后编码 | doctestable 31 次 pass 改动中大半是 InstCombine/SimplifyCFG/JumpThreading/CVP | [nodes.md](../design/ir/nodes.md) |
| 泛型优化一次 + 答案键去重 | ≤2.3×；dedup 5–7% IR | [generics-specialization.md](../design/ir/generics-specialization.md) |
| build 级共享实例，不按导入方重复 | post-link 40% | [compile-time.md](../design/goals/compile-time.md) |
| 只发射可达实例 | 22–31% 死代码 | [size.md](../design/goals/size.md) |
| 变化驱动的 pass | 83% 的 InstCombine 调用无变化 | [optimization-pipeline.md](../design/ir/optimization-pipeline.md) |
| 接口先发布 | 模型上限 −28.5% wall；8 vCPU 实测 −3.3% | [compile-time.md](../design/goals/compile-time.md) |
