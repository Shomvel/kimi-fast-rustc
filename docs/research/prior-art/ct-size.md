# 体积优化：ICF、函数合并、泛型去重、outlining、冷热分离

问题：[size.md](../../design/goals/size.md) 的对策（可达性、答案键共享、冷区域）和业界做法比，缺什么、哪些不该学？泛型共享的总体比较见 [generic-sharing.md](generic-sharing.md)，本文件只展开体积相关的部分和 Go GC-shape 与答案键的逐项对比。

## 1. 是什么

| 项目 | 年份 | 来源 | 要点 |
|---|---|---|---|
| gold 安全 ICF | 2010 | Tallam 等, [Safe ICF](https://research.google/pubs/safe-icf-pointer-safe-and-unwinding-aware-identical-code-folding-in-gold/)（GCC Summit 2010）；[binutils 补丁邮件](https://sourceware.org/legacy-ml/binutils/2010-01/msg00468.html) 2010-01 | 链接时合并字节和重定位相同的函数节；安全模式按**重定位类型**猜哪些函数被取地址（直接重定位 = 取地址，PC 相对 = 调用），只折叠没被取地址的；PIE 下只折叠构造/析构函数 |
| lld `--icf=all\|safe` | 2018 | Collingbourne, [RFC](https://lists.llvm.org/pipermail/llvm-dev/2018-May/123514.html) 2018-05；[D48146](https://reviews.llvm.org/D48146)；[ICF.cpp](https://github.com/llvm/llvm-project/blob/main/lld/ELF/ICF.cpp) | 编译器把“不是 `(local_)unnamed_addr`”的符号写进 `.llvm_addrsig`（ULEB128 符号下标），`safe` 只折叠不在表里的；等价类算法：先按含重定位目标的哈希分桶，再迭代细化直到不再分裂，各步并行 |
| mold ICF | 2022 / 2026 | [mold #484](https://github.com/rui314/mold/issues/484)（2022-05 加 `safe`，同样读 `.llvm_addrsig`）；Ueyama, [mold 论文](https://arxiv.org/abs/2608.23228) §4.6，2026-08 | 把 ICF 当作有向图上的互模拟：每轮哈希 = 自身旧哈希 + 重定位目标旧哈希，不同哈希数不再增加即停；每轮无同步、完全并行 |
| LLVM MergeFunctions | — | [MergeFunctions.html](https://llvm.org/docs/MergeFunctions.html) | IR 级：全序比较器 + 二叉树查找（文档称比哈希快，最多 15%）；地址不重要时 G 改为 F 的别名，否则 G 变成尾调用 F 的 thunk；自/互递归函数不处理 |
| rustc 与 mergefunc | 2018 / 2019 / 2022 | [#49479](https://github.com/rust-lang/rust/pull/49479)（2018-05 重新启用）；[#57268](https://github.com/rust-lang/rust/pull/57268) 加 `-Zmerge-functions=disabled\|trampolines\|aliases`；[#100035](https://github.com/rust-lang/rust/pull/100035)（1.65，扩展到 `opt-level=s/z`） | 当前源码：目标默认 `aliases`，优化目标为速度或体积时运行（[write.rs](https://github.com/rust-lang/rust/blob/master/compiler/rustc_codegen_ssa/src/back/write.rs)） |
| Rust 函数指针相等 | 当前 | [declare.rs](https://github.com/rust-lang/rust/blob/master/compiler/rustc_codegen_llvm/src/declare.rs)；[`ptr::fn_addr_eq`](https://doc.rust-lang.org/std/ptr/fn.fn_addr_eq.html)（1.85）；[#162140](https://github.com/rust-lang/rust/pull/162140) | rustc 给所有 Rust 函数发 `unnamed_addr`，注释为“Rust 函数地址从不重要，允许合并”；文档：相等比较可能意外为真，也可能任何情况下为假 |
| Go GC-shape stenciling | 2020 设计 / 2022 实现 | [gcshape 设计](https://go.googlesource.com/proposal/+/refs/heads/master/design/generics-implementation-gcshape.md)（2020-09）；[Go 1.18 实现文档](https://go.googlesource.com/proposal/+/master/design/generics-implementation-dictionaries-go1.18.md)（2022-02）；[stenciling 设计](https://go.googlesource.com/proposal/+/refs/heads/master/design/generics-implementation-stenciling.md) | 每个 shape 一份代码 + 编译期静态字典；字典作为首个隐藏参数（甚至在接收者前）；1.18 只用字典、从不纯 stencil |
| Swift 泛型 | 2017 / 2018 | Pestov & McCall, [Implementing Swift Generics](https://llvm.org/devmtg/2017-10/slides/Pestov-McCall-ImplementingGenerics.pdf)（LLVM DevMtg 2017-10）；[SE-0193](https://github.com/swiftlang/swift-evolution/blob/main/proposals/0193-cross-module-inlining-and-specialization.md)（Swift 4.2）；[`@_specialize`](https://github.com/swiftlang/swift/blob/main/docs/ReferenceGuides/UnderscoredAttributes.md) | 未特化体接收类型元数据隐藏参数，经 value witness table（size/align/copy/move/destroy 函数指针）操作值；特化“只是优化”；跨模块特化要求 `@inlinable` 导出函数体；`@_specialize` 强制生成指定特化 |
| .NET 共享泛型 | 2019 | [shared-generics.md](https://github.com/dotnet/runtime/blob/main/docs/design/coreclr/botr/shared-generics.md)（BOTR） | 引用类型实例共享 `__Canon` 代码，值类型各自一份；字典槽位初始为 NULL，首次使用经运行时 helper 懒填充；读槽要 context → 字典指针 → 槽位多级 load |
| MachineOutliner | 2016 | Paquette, [RFC](https://lists.llvm.org/pipermail/llvm-dev/2016-August/104170.html) 2016-08；[DevMtg 2016-11 幻灯片](https://llvm.org/devmtg/2016-11/Slides/Paquette-Outliner.pdf) | 寄存器分配后在机器指令串上用后缀树找重复序列，抽成函数 + 调用 |
| 全局 outliner / 函数合并 | 2020 / 2022 / 2023 | Lee & Tillmann, [DevMtg 2020-02](https://llvm.org/devmtg/2020-02-23/)；Lee, Hoag, Tillmann, [CC 2022](https://dl.acm.org/doi/10.1145/3497776.3517764)；Lee, Ren, Hoag, [arXiv 2312.03214](https://arxiv.org/abs/2312.03214) | ThinLTO 下跨模块 outlining：用上一次构建的全局信息，各模块独立、乐观地生成相同 outlined 函数并去重 |
| IR 级 hot/cold splitting | 2018 / 2019 | Aditya Kumar 与 Sebastian Pop 2018-09 提交 r341669（据 [V. Kumar 2019-01 状态更新](https://lists.llvm.org/pipermail/llvm-dev/2019-January/129606.html)）；A. Kumar, [DevMtg 2019-10](https://llvm.org/devmtg/2019-10/talk-abstracts.html)；[D57082](https://reviews.llvm.org/D57082) | 冷块依据 profile、trap、`cold` 属性、EH；用 CodeExtractor 抽成独立函数 |
| MachineFunctionSplitter | 2020 | S. Kumar, [RFC](https://lists.llvm.org/pipermail/llvm-dev/2020-August/144012.html) 2020-08；[D85368](https://reviews.llvm.org/D85368) | codegen 末尾按 profile 把冷块移到独立节（基于 basic block sections），函数不拆成新函数 |
| BOLT | 2019 | Panchenko 等, [CGO 2019](https://arxiv.org/abs/1807.06735) | 链接后按采样 profile 重排基本块、拆冷热（`-split-functions`、`-split-all-cold`、`-split-eh`），另含一遍 ICF |

## 2. 关键证据

| 量 | 数字 | 来源 |
|---|---|---|
| gold ICF（Google 二进制，平均 text 64 MB） | text 约 −6%；安全模式“几乎同样好”，x86 运行时间不变；理论上可去掉多达 10% | Tallam 2010 摘要 |
| gold 安全 ICF（x86-64） | 效果为完全 ICF 的 98% | binutils 补丁邮件 2010-01 |
| lld 链接 clang（release+asserts） | none 109,407,184 B；safe −0.8%；all −2%；地址表使对象文件大 0.08% | Collingbourne RFC 2018 |
| lld ICF 源码注释 | “通常减少输出几个百分点” | ICF.cpp |
| mold ICF | 未找到原始来源的体积数字（论文与 #484 都没给） | — |
| rustc 重新启用 mergefunc | Servo 可执行文件 −10%；某项目 2.3 MB → 1.5 MB | #49479（2018） |
| `opt-level=s` 下手动开 mergefunc | 约 −1% | [#98215](https://github.com/rust-lang/rust/issues/98215)（用户报告） |
| MachineOutliner 原型（LLVM test suite，≥4 KB，x86-64） | 对默认 clang 平均 −12.94%（最多 −51.4%）；对 `-Oz` 平均 −1.29%（最多 −37%） | Paquette RFC 2016 |
| 全局 outliner（构建 clang） | ThinLTO + 默认 outliner −5.5%，构建 1.05×；全局 outliner −11.5%，1.41×；Full LTO −12.0%，2.84× | CC 2022 表 4（据搜索摘录，二手，未读到全文） |
| 全局函数合并（iOS app） | 全局 outlining 平均 −17.3%，再加合并最多再 −3.5% | arXiv 2312.03214 摘要 |
| IR 级 HCS 放在 pipeline 末尾 vs 内联前 | iOS shared cache 体积增长 0.69% → 0.34%；理由：冷代码先抽出就不再被内联 | D57082（2019） |
| IR 级 HCS 实际拆出的量 | LLVM test suite 0.1%；iOS shared cache 0.9%（libdispatch 7%）；arm64 性能在噪声内 | V. Kumar 2019-01 |
| HCS vs MFS（clang） | 性能 +1.10% vs +2.65%；体积 +9.6% vs +1.7% | MFS RFC 2020 |
| MFS 动机 | 约一半最热函数中，超过 50% 的代码字节从未执行；clang bootstrap +2.33%，Google 负载 +0.8–1.2% | MFS RFC 2020 |
| A. Kumar 2019 演讲的数字 | 摘要无数字；未找到原始来源 | — |

### Go GC-shape 与 kfrc 答案键

| 维度 | Go（1.18 起） | kfrc 答案键（[generics-specialization.md](../../design/ir/generics-specialization.md)） |
|---|---|---|
| 分组依据 | 类型实参整体：底层类型相同，或都是指针；接口与非接口永不同组；`int16`/`int32` 因移位不同而分开（1.18 文档）。2020 设计原本按大小/对齐/指针位置 | 孔位答案：只问图真正用到的事；同一对类型实参对不同函数可能同键也可能不同键 |
| 指针 | 所有指针同一 shape（`go.shape.*uint8_N`），方法信息全在字典 | 只在解引用/偏移/按值拷贝时问 pointee；对指针调 trait 方法时 `Resolve` 答案不同，键就不同 |
| 差异放哪 | 运行时字典：类型实参的 `_type`、派生类型、子字典、itab；闭包共用外层字典 | 没有运行时数据；不同答案就是不同键，各自静态特化 |
| 方法调用 | 经字典 itab 间接调用，不能内联；逃逸分析保守（设计文档 Risks；[#75056](https://github.com/golang/go/issues/75056) 2025-08，Go 团队确认）；2026-09 [#81377](https://github.com/golang/go/issues/81377) 封闭约束下仍未去虚化 | `Resolve` 答案是被调实例的答案键，S 级变直接 `Call`，按内联策略内联 |
| 递归与传递 | 子字典编译期全部算好；非单态化递归（#48018）不支持 | 键递归；调用环用类细化不动点 |
| 静态特化 | 1.18 从不纯 stencil；[#81955](https://github.com/golang/go/issues/81955)（2026-10，未处理）提议对小/热/stdlib 函数特化；PGO 只做接口调用去虚化（[1.21](https://go.dev/doc/go1.21)、[1.22](https://go.dev/doc/go1.22)），发布说明未提字典调用 | 全部静态特化 |
| 代价落在 | 运行时：手工特化 5.06 µs → 泛型指针 7.18 µs → 传接口 9.68–17.6 µs（[PlanetScale 2022-03](https://planetscale.com/blog/generics-can-make-your-go-code-slower)，二手但有可复现基准）；体积：每 itab 字典项 8 字节、无字典方法的包装器（#81955） | 编译期：回答孔位、哈希；体积：答案不同的实例各一份 |
| 体积收益数据 | gcshape 设计文档把“比全 stencil 省多少”列为开放问题；未找到原始来源的测量 | `-Zmono-dedup` IR 去重 5.1% / 6.8%（[evidence.md](../evidence.md) 第 3 节） |

## 3. 对 kfrc 的结论

| 做法 | 结论 | 理由 |
|---|---|---|
| 函数合并合法性 | **采纳（已一致）** | rustc 本来就给所有函数发 `unnamed_addr` 并默认开 mergefunc；kfrc 的“全模式”折叠不比 rustc 更激进。`#[inline(never)]` 导出泛型的例外维持 |
| 答案键之后再做机器码片段折叠 | **采纳（补充层）** | 答案键在 codegen 前按类型化答案合并（主力，见 generic-sharing.md）；但键含泛型定义的图哈希，不同定义优化后相同的代码（derive 生成的 impl、newtype 转发、被调者折叠后变得相同的调用者）不会同键。gold/lld 数据：残余 2–6%；做法用 lld/mold 的哈希 + 细化，片段已按内容寻址，代价很小 |
| `.llvm_addrsig` 式安全模式 | **不采纳** | kfrc 自有全程序实例图，知道哪些函数被取地址；Rust 不承诺函数地址唯一。只需保证 `static` 不参与折叠 |
| Go/.NET/Swift 运行时字典 | **不采纳（维持）** | 方法经字典不能内联，Go 1.18 发布四年后仍在讨论补回特化（#81955）；kfrc 的体积靠“只问用到的事”拿，不拿运行时换 |
| “只依赖 `SizeAlign(T)` 的冷区域”参数化 | **改编（维持扩展，加约束）** | 相当于 Swift VWT 的 size/align 部分；只允许数据类孔位，`Resolve` 不得进运行时参数，否则就是 Go 的问题 |
| MachineOutliner / 全局 outliner | **不采纳（首版）** | 对 `-Oz` 平均只 −1.29%；全局版本要上次构建的全局信息，和按片段缓存、确定性冲突；构建 1.41× |
| 同函数 panic 尾块合并 | **改编：扩到跨函数** | outliner 的有效子集：`Location` 进数据表后，相同 panic 序列跨函数字节相同，可作共享冷桩 |
| IR 级抽冷区域为新函数（HCS） | **不采纳** | clang 上体积 +9.6%；kfrc 的冷区域留在原函数 |
| 冷块放独立节（MFS/BOLT 方式） | **改编（结构冷，无 profile）** | MFS 体积仅 +1.7% 且有性能收益；kfrc 冷是结构事实，不受 profile 失真影响 |
| 冷区域不内联 | **采纳（已一致）** | D57082：冷代码在内联前识别，体积增长减半 |

## 4. 具体建议

| 目标文件 | 节 | 改法 |
|---|---|---|
| [size.md](../../design/goals/size.md) | 来源和对策 | 加一行“答案不同但机器码相同”：L 级片段按（字节、重定位目标所在类）做 lld/mold 式哈希细化，同类出别名；只限函数和常量数据，`static` 不折叠；依据 lld −2%、gold −6% |
| [size.md](../../design/goals/size.md) | 指标 | 加“片段折叠数及节省字节”“答案键与片段折叠各自贡献”，对照 rustc mergefunc |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 答案键 | 环的类细化写明算法：按 lld ICF.cpp / mold §4.6 的哈希细化，每轮只读上轮结果，可并行 |
| [generics-specialization.md](../../design/ir/generics-specialization.md) | 可能的扩展 | 加约束：运行时参数只承载 `SizeAlign`/`NeedsDrop` 等数据类答案，禁止 `Resolve`；只用于冷区域；引用 Go #75056 |
| [ownership-drop-unwind.md](../../design/ir/ownership-drop-unwind.md) | 冷区域 | 写明冷区域不抽成独立函数（HCS +9.6%）；L 级可把冷块放入 `.text.unlikely` 节；相同 panic 序列跨函数共享冷桩 |
| [optimization-pipeline.md](../../design/ir/optimization-pipeline.md) | L 级·冷区域布局 / 不做 | “不做”加：MachineOutliner、全局 outliner（理由同上） |
| [cache.md](../../design/build/cache.md) | 存储形式 | 注明内容寻址的片段存储同时就是片段折叠的索引，折叠结果不另存 |
| [plan/README.md](../../plan/README.md#验收) | 产物体积 | rustc“最佳 release”基线写明含 `opt-level=s/z`、mergefunc 默认、lld `--icf=all`；kfrc 报告关/开片段折叠两组 |
